//! Reading one answer: what it cost, and which of five things it was.
//!
//! Kept apart from the sending half because it is the half that judges, and
//! generic over what was asked for because none of the five distinctions depends
//! on the subject. An answer is the thing that was asked for, a refusal, an answer
//! that ran out of room, no answer at all, or text that is not the JSON the
//! request's schema described — and whichever it is, what the attempt consumed is
//! read out first, because a provider charges for an answer nobody could use.
//!
//! What each of the five *means* is the caller's to say. A truncated judgement and
//! a truncated list of mutations are the same event with different advice attached.

use std::time::Duration;

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

use super::transport::{Declined, declined, quote, redact, send};
use crate::generate::{Attempt, Usage};

/// What one attempt over the wire came to, before anything about the subject.
///
/// Both asks make the same five distinctions here and act on them the same way
/// except in the words they report; the one that is not a failure carries the
/// attempt, the model that answered, and what it said.
pub(crate) enum Attempted<T> {
    /// The model answered. The attempt is on the ledger whatever it said.
    Spoke(Attempt, String, Spoke<T>),
    /// The provider is rate limiting, and said how long to wait when it said.
    RateLimited(Option<Duration>),
    /// The provider declined rather than answering.
    Declined(Declined),
    /// The call never got an answer.
    Unreachable(String),
    /// What came back was not this provider's envelope, so no model spoke.
    UnreadableEnvelope(String),
}

/// Send one payload and read whatever came back as the answer `T`.
pub(crate) fn attempt<T: DeserializeOwned>(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    key: &str,
    payload: &Value,
) -> Attempted<T> {
    let answered = match send(client, endpoint, key, payload) {
        Ok(answered) => answered,
        Err(reason) => return Attempted::Unreachable(reason),
    };
    if answered.status == 429 {
        return Attempted::RateLimited(answered.wait);
    }
    if let Some(refusal) = declined(&answered, key) {
        return Attempted::Declined(refusal);
    }
    // A body that is not this provider's envelope is not the model failing to
    // answer well: it is something other than the model answering, so there is no
    // defect to put in front of anyone and no ask to correct.
    let reply: Reply = match serde_json::from_str(&answered.body) {
        Ok(reply) => reply,
        Err(error) => return Attempted::UnreadableEnvelope(redact(&error.to_string(), key)),
    };
    let model = reply.model.clone();
    let (attempt, spoke) = read(reply, key);
    Attempted::Spoke(attempt, model, spoke)
}

/// What one answer amounted to.
pub(crate) enum Spoke<T> {
    /// The answer, read into the type the request's schema described.
    Answer(T),
    /// The model declined to answer, in its own words.
    Refused(String),
    /// The answer stopped at the token limit before it was complete.
    Truncated,
    /// The provider answered with nothing in it.
    NoAnswer,
    /// What the model said was not the JSON this contract reads.
    NotTheContract(String),
}

/// Read one reply: what it cost, and what it said.
fn read<T: DeserializeOwned>(reply: Reply, key: &str) -> (Attempt, Spoke<T>) {
    let usage = reply.usage.into();
    let Some(choice) = reply.choices.into_iter().next() else {
        return (
            Attempt {
                usage,
                finish: None,
            },
            Spoke::NoAnswer,
        );
    };
    let attempt = Attempt {
        usage,
        finish: choice.finish_reason.clone(),
    };
    if let Some(refusal) = choice.message.refusal.filter(|said| !said.is_empty()) {
        return (attempt, Spoke::Refused(quote(&refusal, key)));
    }
    if choice.finish_reason.as_deref() == Some("length") {
        return (attempt, Spoke::Truncated);
    }
    let said = choice.message.content.unwrap_or_default();
    let spoke = match serde_json::from_str::<T>(&said) {
        Ok(answer) => Spoke::Answer(answer),
        Err(error) => Spoke::NotTheContract(redact(&error.to_string(), key)),
    };
    (attempt, spoke)
}

/// What the provider answers with, as much of it as is read.
#[derive(Debug, Deserialize)]
pub(crate) struct Reply {
    /// The model that answered, which is the one recorded.
    #[serde(default)]
    pub(crate) model: String,
    /// The answers. More than one is never asked for, so the first is the one.
    #[serde(default)]
    choices: Vec<Choice>,
    /// What the attempt consumed.
    #[serde(default)]
    usage: WireUsage,
}

/// One answer among the choices.
#[derive(Debug, Deserialize)]
struct Choice {
    /// Why the answer ended.
    #[serde(default)]
    finish_reason: Option<String>,
    /// What was said.
    #[serde(default)]
    message: Message,
}

/// The content of one answer.
#[derive(Debug, Default, Deserialize)]
struct Message {
    /// The answer itself, which this contract expects to be JSON.
    #[serde(default)]
    content: Option<String>,
    /// Present instead when the model declined to answer.
    #[serde(default)]
    refusal: Option<String>,
}

/// Token counts, under the names this endpoint gives them.
#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    /// Tokens the prompt cost.
    #[serde(default, rename = "prompt_tokens")]
    prompt: u64,
    /// Tokens the answer cost.
    #[serde(default, rename = "completion_tokens")]
    completion: u64,
    /// What the provider reported as the sum.
    #[serde(default, rename = "total_tokens")]
    total: u64,
}

impl From<WireUsage> for Usage {
    fn from(usage: WireUsage) -> Self {
        Self {
            prompt: usage.prompt,
            completion: usage.completion,
            total: usage.total,
        }
    }
}
