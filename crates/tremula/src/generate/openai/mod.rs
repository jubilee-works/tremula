//! The reference adapter: one `OpenAI` chat completion, held to a schema.
//!
//! Two things are asked of a provider here and they are separate modules: this one
//! asks for mutations, and [`judge`] asks whether one mutation can change what a
//! function does. Everything they share — the key, the sending, the statuses, the
//! wait, the schema closing, the reading of an envelope — is [`transport`] and
//! [`answer`], so a third ask would inherit all of it.
//!
//! One call, one answer, and at most two retries that are not the same retry: one
//! wait when the provider asks for one, and one corrected ask when the answer is
//! not the contract. The key is read from the environment at the moment of the
//! call, is put on a header and never on the payload, and is taken back out of
//! anything the provider says before that reaches an error message.

use std::time::Duration;

use serde_json::Value;

use crate::generate::{
    Attempt, GenerateError, GenerateFailure, GeneratedMutantsResponse, GenerationOutcome,
    GenerationRequest, MutantGenerator, prompt,
};

mod answer;
pub mod judge;
mod reply;
mod transport;

use answer::Attempted;
use reply::{Answered, mutations};
use transport::{Declined, Strictness, Unsent, strict_schema};
pub(crate) use transport::{quote, redact};

pub use transport::{
    ENDPOINT, KEY_VARIABLE, MAX_COMPLETION_TOKENS, RETRY_AFTER_CAP, TIMEOUT, retry_delay,
};

/// What the schema is called in the request. The provider reports it back in
/// its own errors, so it names this tool rather than the endpoint.
pub const SCHEMA_NAME: &str = "tremula_mutants";

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
        let (key, client) = match transport::ready(self.timeout) {
            Ok(ready) => ready,
            Err(Unsent::MissingCredential) => {
                return Err(GenerateError::unsent(GenerateFailure::MissingCredential {
                    variable: KEY_VARIABLE.to_owned(),
                }));
            }
            Err(Unsent::NoClient(reason)) => {
                return Err(GenerateError::unsent(GenerateFailure::Unreachable {
                    reason,
                }));
            }
        };
        let assembled = prompt::assemble(request);
        let mut correction = assembled.correction.clone();
        let mut attempts: Vec<Attempt> = Vec::new();
        let mut rate_limit_retried = false;
        let mut correction_retried = false;
        loop {
            let payload = transport::payload(
                &self.model,
                &assembled,
                correction.as_deref(),
                SCHEMA_NAME,
                &response_schema(),
            );
            let (attempt, model, spoke) = match answer::attempt::<GeneratedMutantsResponse>(
                &client,
                &self.endpoint,
                &key,
                &payload,
            ) {
                Attempted::Spoke(attempt, model, spoke) => (attempt, model, spoke),
                Attempted::RateLimited(wait) => {
                    attempts.push(Attempt::default());
                    if let (Some(wait), false) = (wait, rate_limit_retried) {
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
                other => {
                    attempts.push(Attempt::default());
                    return Err(GenerateError {
                        kind: refusal(other),
                        attempts,
                    });
                }
            };
            attempts.push(attempt);
            let defect = match mutations(spoke, request.mutant_count) {
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

/// What an attempt that never reached the model was, in a generation's own words.
///
/// Rate limiting is not among these: it is the one of them a call may answer by
/// waiting, so it stays where the wait is spent.
fn refusal<T>(attempted: Attempted<T>) -> GenerateFailure {
    match attempted {
        Attempted::Unreachable(reason) => GenerateFailure::Unreachable { reason },
        Attempted::UnreadableEnvelope(reason) => GenerateFailure::UnreadableEnvelope { reason },
        Attempted::Declined(Declined::CredentialRejected) => GenerateFailure::CredentialRejected {
            variable: KEY_VARIABLE.to_owned(),
        },
        Attempted::Declined(Declined::Rejected { status, detail }) => {
            GenerateFailure::Rejected { status, detail }
        }
        // Both of the remaining shapes carry an answer, and an answer is read
        // rather than refused. Reaching here would mean this function had been
        // handed the one thing it exists to be the alternative to.
        Attempted::Spoke(..) | Attempted::RateLimited(_) => GenerateFailure::NoAnswer,
    }
}

/// The schema a model answering with mutations is held to.
///
/// Every member of this one is required by the type itself, so it is sent exactly
/// as it was measured.
#[must_use]
pub fn response_schema() -> Value {
    strict_schema::<GeneratedMutantsResponse>(Strictness::AsMeasured)
}
