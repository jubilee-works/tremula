//! The reference generator: one `OpenAI` chat completion, held to a schema.
//!
//! One call, one answer, and at most two retries that are not the same retry:
//! one wait when the provider asks for one, and one corrected ask when the answer
//! is not the contract. The key is read from the environment at the moment of the
//! call, is put on a header and never on the payload, and is taken back out of
//! anything the provider says before that reaches an error message.
//!
//! What is sent is what was measured: the token limit under the name this
//! endpoint uses, a schema the provider enforces server side, and no sampling
//! controls at all. The measurement stands only for what was sent, so setting a
//! temperature or a seed here would be a change to something nobody has
//! measured, not a refinement of it.

use std::time::Duration;

use schemars::{Schema, generate::SchemaSettings, transform::RecursiveTransform};
use serde_json::{Value, json};

use crate::generate::{
    Attempt, GenerateError, GenerateFailure, GeneratedMutantsResponse, GenerationOutcome,
    GenerationRequest, MutantGenerator, prompt,
};

mod reply;

use reply::{Answered, Reply, read};

/// Where the key is read from, every time a call is made.
pub const KEY_VARIABLE: &str = "OPENAI_API_KEY";

/// The endpoint this generator speaks to unless it is pointed elsewhere.
pub const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

/// The completion-token ceiling on one answer.
///
/// Enough for five mutations of a long function with room to spare: the longest
/// measured answer used under a quarter of it.
pub const MAX_COMPLETION_TOKENS: u32 = 2000;

/// How long one attempt may take. The measured calls averaged seven seconds and
/// the slowest took nine, so this is generous by design and still short enough
/// that a hung call fails inside a person's patience.
pub const TIMEOUT: Duration = Duration::from_mins(1);

/// The longest wait this generator will honour when a provider asks for one.
/// Past this, a call that is waiting has stopped being a call.
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);

/// What the schema is called in the request. The provider reports it back in
/// its own errors, so it names this tool rather than the endpoint.
pub const SCHEMA_NAME: &str = "tremula_mutants";

/// How much of a provider's own message is quoted back in an error.
const QUOTED: usize = 400;

/// What replaces anything key-shaped on its way into a message.
const REDACTED: &str = "«redacted»";

/// A generator that asks an OpenAI-compatible endpoint for mutants.
#[derive(Debug, Clone)]
pub struct OpenAiGenerator {
    model: String,
    endpoint: String,
    timeout: Duration,
}

impl OpenAiGenerator {
    /// Name the exact model snapshot to ask.
    ///
    /// A family name is accepted by the provider and is a mistake here: it
    /// resolves to whichever version is current, and what the answer reports is
    /// what gets written into a mutant's provenance.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            endpoint: ENDPOINT.to_owned(),
            timeout: TIMEOUT,
        }
    }

    /// Point this generator at another endpoint that speaks the same protocol.
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Give one attempt a different amount of time than [`TIMEOUT`].
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The payload of one attempt.
    fn payload(&self, prompt: &prompt::Prompt, correction: Option<&str>) -> Value {
        let mut messages = vec![
            json!({"role": "system", "content": prompt.system}),
            json!({"role": "user", "content": prompt.user}),
        ];
        if let Some(correction) = correction {
            messages.push(json!({"role": "user", "content": correction}));
        }
        json!({
            "model": self.model,
            "messages": messages,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": SCHEMA_NAME,
                    "strict": true,
                    "schema": response_schema(),
                },
            },
            "max_completion_tokens": MAX_COMPLETION_TOKENS,
        })
    }
}

impl MutantGenerator for OpenAiGenerator {
    /// Ask for mutants, waiting out one rate limit and correcting one answer.
    ///
    /// The two retries are counted apart. A call that spent its wait can still
    /// correct an answer, and a call that corrected an answer can still be told
    /// to wait — collapsing them into one budget would make the second failure of
    /// a call depend on which kind the first one was, and would put a claim in a
    /// message that the ledger does not support.
    ///
    /// # Errors
    ///
    /// Returns [`GenerateError`] with the attempts already paid for: no key, a
    /// key the provider rejected, a provider that could not be reached, would not
    /// answer, or answered in some other protocol, a refusal, an answer cut off
    /// at the token limit, an answer with no choices in it, or an answer that
    /// broke the contract twice.
    fn generate(&self, request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        let (key, client) = self.ready()?;
        let assembled = prompt::assemble(request);
        let mut correction = assembled.correction.clone();
        let mut attempts: Vec<Attempt> = Vec::new();
        let mut rate_limit_retried = false;
        let mut correction_retried = false;
        loop {
            let sent = self.send(
                &client,
                &key,
                &self.payload(&assembled, correction.as_deref()),
            );
            let answered = match sent {
                Ok(answered) => answered,
                Err(failure) => {
                    attempts.push(Attempt::default());
                    return Err(GenerateError {
                        kind: failure,
                        attempts,
                    });
                }
            };
            if answered.status == 429 {
                attempts.push(Attempt::default());
                if let (Some(wait), false) = (answered.wait, rate_limit_retried) {
                    rate_limit_retried = true;
                    std::thread::sleep(wait);
                    continue;
                }
                return Err(GenerateError {
                    kind: GenerateFailure::RateLimited {
                        retried: rate_limit_retried,
                    },
                    attempts,
                });
            }
            if let Some(failure) = refused(&answered, &key) {
                attempts.push(Attempt::default());
                return Err(GenerateError {
                    kind: failure,
                    attempts,
                });
            }
            // A body that is not this provider's envelope is not the model
            // failing to answer well: it is something other than the model
            // answering, so there is no defect to put in front of anyone and no
            // ask to correct. The attempt is on the ledger all the same.
            let reply: Reply = match serde_json::from_str(&answered.body) {
                Ok(reply) => reply,
                Err(error) => {
                    attempts.push(Attempt::default());
                    return Err(GenerateError {
                        kind: GenerateFailure::UnreadableEnvelope {
                            reason: redact(&error.to_string(), &key),
                        },
                        attempts,
                    });
                }
            };
            let model = reply.model.clone();
            let (attempt, read) = read(reply, request.mutant_count, &key);
            attempts.push(attempt);
            let defect = match read {
                Ok(mutants) => {
                    return Ok(GenerationOutcome {
                        mutants,
                        model_resolved: model,
                        attempts,
                    });
                }
                Err(Answered::Fatal(failure)) => {
                    return Err(GenerateError {
                        kind: failure,
                        attempts,
                    });
                }
                Err(Answered::Correctable(defect)) => defect,
            };
            if correction_retried {
                return Err(GenerateError {
                    kind: defect.failure(request.mutant_count),
                    attempts,
                });
            }
            correction_retried = true;
            let mut all = request
                .feedback
                .as_ref()
                .map(|feedback| feedback.defects.clone())
                .unwrap_or_default();
            all.push(defect.stated());
            correction = Some(prompt::correction(&all));
        }
    }
}

impl OpenAiGenerator {
    /// The key and the client, or the failure that means nothing was sent.
    fn ready(&self) -> Result<(String, reqwest::blocking::Client), GenerateError> {
        let Ok(key) = std::env::var(KEY_VARIABLE) else {
            return Err(GenerateError::unsent(GenerateFailure::MissingCredential {
                variable: KEY_VARIABLE.to_owned(),
            }));
        };
        match reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .build()
        {
            Ok(client) => Ok((key, client)),
            Err(error) => Err(GenerateError::unsent(GenerateFailure::Unreachable {
                reason: redact(&error.to_string(), &key),
            })),
        }
    }

    /// Send one payload and read back what came of it.
    fn send(
        &self,
        client: &reqwest::blocking::Client,
        key: &str,
        payload: &Value,
    ) -> Result<Sent, GenerateFailure> {
        let unreachable = |error: &dyn std::fmt::Display| GenerateFailure::Unreachable {
            reason: redact(&error.to_string(), key),
        };
        let answer = client
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {key}"))
            .body(payload.to_string())
            .send()
            .map_err(|error| unreachable(&error))?;
        let status = answer.status().as_u16();
        let wait = answer
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(retry_delay);
        let body = answer.text().map_err(|error| unreachable(&error))?;
        Ok(Sent { status, wait, body })
    }
}

/// One answer as it came off the wire.
struct Sent {
    /// The status it carried.
    status: u16,
    /// How long it asked to be left alone, when it asked.
    wait: Option<Duration>,
    /// Its body, whatever the status.
    body: String,
}

/// Whether a status is the provider declining rather than answering.
fn refused(answered: &Sent, key: &str) -> Option<GenerateFailure> {
    if answered.status == 401 {
        return Some(GenerateFailure::CredentialRejected {
            variable: KEY_VARIABLE.to_owned(),
        });
    }
    if (200..300).contains(&answered.status) {
        return None;
    }
    Some(GenerateFailure::Rejected {
        status: answered.status,
        detail: quote(&answered.body, key),
    })
}

/// How long to wait when a provider says how long to wait.
///
/// A header that is there earns the retry, whatever it says in. A number of
/// seconds is honoured as given, capped at [`RETRY_AFTER_CAP`].
///
/// The header may also carry an HTTP date, which this does not parse — a date
/// needs a clock the provider and this process do not share, and a wrong reading
/// of one asks to come back too early, which is the reading that gets the key
/// limited again. So a value this cannot read is answered with the cap: the
/// longest wait this generator would ever spend on any value, and therefore the
/// most conservative one available without a parser. A date that meant less than
/// the cap costs the call the difference; a date that meant more was going to be
/// capped anyway. A header with nothing in it asked for nothing, and gets
/// nothing.
#[must_use]
pub fn retry_delay(header: &str) -> Option<Duration> {
    let asked = header.trim();
    if asked.is_empty() {
        return None;
    }
    match asked.parse::<u64>() {
        Ok(seconds) => Some(Duration::from_secs(seconds).min(RETRY_AFTER_CAP)),
        Err(_) => Some(RETRY_AFTER_CAP),
    }
}

/// The schema a model is held to, as the provider requires it.
///
/// Derived from the type that reads the answer, so the two cannot drift apart,
/// and then closed: every object in it gets `additionalProperties: false`,
/// which the derive does not add and the provider will not accept a strict
/// schema without. Subschemas are inlined for the same reason — a schema that
/// refers to itself elsewhere is one more thing for a provider to disagree
/// with, and this one is small enough to spell out.
#[must_use]
pub fn response_schema() -> Value {
    let settings = SchemaSettings::default().with(|settings| {
        settings.inline_subschemas = true;
        settings.meta_schema = None;
        settings.transforms = vec![Box::new(RecursiveTransform(
            close_object as fn(&mut Schema),
        ))];
    });
    settings
        .into_generator()
        .into_root_schema_for::<GeneratedMutantsResponse>()
        .to_value()
}

/// Refuse any member a schema did not name.
///
/// Applied to every subschema, including the `false` this inserts, which is why
/// anything that is not an object schema is left exactly as it was.
fn close_object(schema: &mut Schema) {
    let names_members = schema.get("properties").is_some()
        || schema.get("type").and_then(Value::as_str) == Some("object");
    if names_members && schema.get("additionalProperties").is_none() {
        schema.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
}

/// What a provider said, shortened and with anything key-shaped taken out.
fn quote(said: &str, key: &str) -> String {
    let cleaned = redact(said.trim(), key);
    match cleaned.char_indices().nth(QUOTED) {
        Some((cut, _)) => format!("{}…", &cleaned[..cut]),
        None => cleaned,
    }
}

/// Take the key out of anything on its way to a person.
///
/// A provider that echoes a credential back into an error message is the case
/// this exists for: the message is still worth reading, and the credential is
/// not worth repeating into a log, a terminal, or a bug report.
fn redact(text: &str, key: &str) -> String {
    if key.is_empty() {
        return text.to_owned();
    }
    text.replace(key, REDACTED)
}
