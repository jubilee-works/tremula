//! The part of one provider exchange that is the same whatever is being asked.
//!
//! Two asks live above this module — one for mutations, one for a judgement about
//! a mutation — and everything they have in common is here: where the key comes
//! from, how a payload is sent, which statuses are the provider declining rather
//! than answering, how long to wait when it says to wait, and how a schema is
//! closed before it is sent. What is left to each ask is the question, the type
//! its answer is read into, and the words its failures are reported in.
//!
//! Nothing here names mutations or judgements, and nothing here decides whether
//! an answer is good enough. That is the point: an ask added later gets the
//! transport for free and still owes its reader a message about its own subject.

use std::time::Duration;

use schemars::{JsonSchema, Schema, generate::SchemaSettings, transform::RecursiveTransform};
use serde_json::{Value, json};

use crate::generate::prompt::Prompt;

/// Where the key is read from, every time a call is made.
pub const KEY_VARIABLE: &str = "OPENAI_API_KEY";

/// The endpoint this adapter speaks to unless it is pointed elsewhere.
pub const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

/// The completion-token ceiling on one answer.
///
/// Enough for five mutations of a long function with room to spare: the longest
/// measured answer used under a quarter of it. A judgement is smaller again.
pub const MAX_COMPLETION_TOKENS: u32 = 2000;

/// How long one attempt may take. The measured calls averaged seven seconds and
/// the slowest took nine, so this is generous by design and still short enough
/// that a hung call fails inside a person's patience.
pub const TIMEOUT: Duration = Duration::from_mins(1);

/// The longest wait this adapter will honour when a provider asks for one.
/// Past this, a call that is waiting has stopped being a call.
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);

/// How much of a provider's own message is quoted back in an error.
const QUOTED: usize = 400;

/// What replaces anything key-shaped on its way into a message.
const REDACTED: &str = "«redacted»";

/// Why nothing was sent at all, so nothing was owed.
pub(crate) enum Unsent {
    /// No key in the environment.
    MissingCredential,
    /// The client itself could not be built.
    NoClient(String),
}

/// The provider declining rather than answering, with the key taken out of
/// whatever it said.
pub(crate) enum Declined {
    /// It would not accept the key.
    CredentialRejected,
    /// It refused the call for some other reason.
    Rejected {
        /// The status it answered with.
        status: u16,
        /// What it said.
        detail: String,
    },
}

/// One answer as it came off the wire.
pub(crate) struct Sent {
    /// The status it carried.
    pub(crate) status: u16,
    /// How long it asked to be left alone, when it asked.
    pub(crate) wait: Option<Duration>,
    /// Its body, whatever the status.
    pub(crate) body: String,
}

/// The key and a client to send with, or the reason nothing was sent.
pub(crate) fn ready(timeout: Duration) -> Result<(String, reqwest::blocking::Client), Unsent> {
    let Ok(key) = std::env::var(KEY_VARIABLE) else {
        return Err(Unsent::MissingCredential);
    };
    match reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
    {
        Ok(client) => Ok((key, client)),
        Err(error) => Err(Unsent::NoClient(redact(&error.to_string(), &key))),
    }
}

/// Send one payload and read back what came of it, or say why it could not be
/// reached. The key goes on a header and never on the payload.
pub(crate) fn send(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    key: &str,
    payload: &Value,
) -> Result<Sent, String> {
    let unreachable = |error: &dyn std::fmt::Display| redact(&error.to_string(), key);
    let answer = client
        .post(endpoint)
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

/// Whether a status is the provider declining rather than answering.
///
/// Rate limiting is not among these: it is the one status an ask may answer by
/// waiting, so the decision belongs to the ask that owns the wait.
pub(crate) fn declined(answered: &Sent, key: &str) -> Option<Declined> {
    if answered.status == 401 {
        return Some(Declined::CredentialRejected);
    }
    if (200..300).contains(&answered.status) {
        return None;
    }
    Some(Declined::Rejected {
        status: answered.status,
        detail: quote(&answered.body, key),
    })
}

/// The payload of one attempt: what to ask, and the schema the answer is held to.
///
/// What is sent is what was measured — the token limit under the name this
/// endpoint uses, a schema the provider enforces server side, and no sampling
/// controls at all. Setting a temperature or a seed here would be a change to
/// something nobody has measured, not a refinement of it.
pub(crate) fn payload(
    model: &str,
    prompt: &Prompt,
    correction: Option<&str>,
    schema_name: &str,
    schema: &Value,
) -> Value {
    let mut messages = vec![
        json!({"role": "system", "content": prompt.system}),
        json!({"role": "user", "content": prompt.user}),
    ];
    if let Some(correction) = correction {
        messages.push(json!({"role": "user", "content": correction}));
    }
    json!({
        "model": model,
        "messages": messages,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": schema_name,
                "strict": true,
                "schema": schema,
            },
        },
        "max_completion_tokens": MAX_COMPLETION_TOKENS,
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
/// longest wait this adapter would ever spend on any value, and therefore the
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

/// How far a derived schema has to be adjusted before a strict provider takes it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Strictness {
    /// Close every object, and nothing else.
    ///
    /// What the mutation schema is. It has no optional member and no enumerated
    /// value, so nothing further applies to it — and it is the schema 48 measured
    /// calls were made against, so an adjustment that changed it would be a change
    /// to something nobody has re-measured.
    AsMeasured,
    /// Close every object, list every member as required, spell a choice of values
    /// as one `enum` with the values documented, and drop the defaults.
    ///
    /// Every one of those is something strict server-side enforcement needs and
    /// the derive does not produce: a member that may be absent has to arrive as
    /// one that may be null, and a choice of values expressed as a union of
    /// constants is a composition keyword a strict schema may not use.
    Adjusted,
}

/// The schema a model is held to, as the provider requires it.
///
/// Derived from the type that reads the answer, so the two cannot drift apart.
/// Subschemas are inlined: a schema that refers to itself elsewhere is one more
/// thing for a provider to disagree with, and these are small enough to spell out.
pub(crate) fn strict_schema<T: JsonSchema>(strictness: Strictness) -> Value {
    let settings = SchemaSettings::default().with(|settings| {
        settings.inline_subschemas = true;
        settings.meta_schema = None;
        settings.transforms = vec![Box::new(RecursiveTransform(
            close_object as fn(&mut Schema),
        ))];
        if matches!(strictness, Strictness::Adjusted) {
            for adjustment in [
                name_the_values as fn(&mut Schema),
                require_every_member as fn(&mut Schema),
                drop_the_default as fn(&mut Schema),
            ] {
                settings
                    .transforms
                    .push(Box::new(RecursiveTransform(adjustment)));
            }
        }
    });
    settings
        .into_generator()
        .into_root_schema_for::<T>()
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

/// List every member an object schema names as required, so a member that may be
/// absent arrives as one that may be null.
fn require_every_member(schema: &mut Schema) {
    let Some(named) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    let required: Vec<Value> = named
        .keys()
        .map(|name| Value::String(name.clone()))
        .collect();
    schema.insert("required".to_owned(), Value::Array(required));
}

/// Turn a union of string constants into one `enum`, keeping what each value
/// meant.
///
/// The derive spells a set of named values as a composition of constants, and a
/// strict schema may not compose. Folding the values into an `enum` says the same
/// thing in the one keyword that is allowed — and folding their descriptions into
/// the schema's own keeps the sentences that tell a model what it is choosing
/// between, which is the half of a schema that does the work.
fn name_the_values(schema: &mut Schema) {
    for keyword in ["oneOf", "anyOf"] {
        let Some(branches) = schema.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let mut values = Vec::new();
        let mut meanings = Vec::new();
        for branch in branches {
            let Some(value) = branch.get("const").and_then(Value::as_str) else {
                return;
            };
            if let Some(said) = branch.get("description").and_then(Value::as_str) {
                meanings.push(format!("`{value}`: {said}"));
            }
            values.push(Value::String(value.to_owned()));
        }
        if values.is_empty() {
            return;
        }
        schema.remove(keyword);
        schema.insert("type".to_owned(), Value::String("string".to_owned()));
        schema.insert("enum".to_owned(), Value::Array(values));
        if !meanings.is_empty() {
            let said = schema
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let meanings = meanings.join("\n");
            let description = if said.is_empty() {
                meanings
            } else {
                format!("{said}\n\n{meanings}")
            };
            schema.insert("description".to_owned(), Value::String(description));
        }
        return;
    }
}

/// Take out a default nobody applies. A strict provider validates the answer
/// against the schema it was sent and fills nothing in, so a default is at best
/// noise and at worst a keyword it refuses.
fn drop_the_default(schema: &mut Schema) {
    schema.remove("default");
}

/// What a provider said, shortened and with anything key-shaped taken out.
pub(crate) fn quote(said: &str, key: &str) -> String {
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
pub(crate) fn redact(text: &str, key: &str) -> String {
    if key.is_empty() {
        return text.to_owned();
    }
    text.replace(key, REDACTED)
}
