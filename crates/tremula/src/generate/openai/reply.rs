//! Reading one answer: what it cost, and what it amounted to.
//!
//! Kept apart from the sending half because it is the half that judges. An
//! answer is either the mutations that were asked for, a failure nothing would
//! improve, or a defect the model can be told about and asked about again — and
//! whichever it is, what the attempt consumed is read out first, because a
//! provider charges for an answer nobody could use.

use serde::Deserialize;

use super::{MAX_COMPLETION_TOKENS, quote, redact};
use crate::generate::{
    Attempt, Defect, GenerateFailure, GeneratedMutant, GeneratedMutantsResponse, Usage,
};

/// What an answer amounted to, when it was not mutants.
pub(super) enum Answered {
    /// Nothing another ask would fix.
    Fatal(GenerateFailure),
    /// Something the model could be told about and asked again.
    Correctable(Correctable),
}

/// A defect worth putting back in front of the model.
pub(super) enum Correctable {
    /// The answer was not the JSON this contract reads.
    NotTheContract(String),
    /// It was, and held a number of mutations nobody asked for.
    WrongCount(usize),
}

impl Correctable {
    /// The defect as the model is told about it.
    pub(super) fn stated(&self) -> Defect {
        match self {
            Self::NotTheContract(reason) => Defect {
                defect: "answer_is_not_the_contract".to_owned(),
                detail: reason.clone(),
            },
            Self::WrongCount(received) => Defect {
                defect: "wrong_number_of_mutants".to_owned(),
                detail: format!("{received} mutations were proposed"),
            },
        }
    }

    /// The failure it becomes once the retry has been spent.
    pub(super) fn failure(self, requested: usize) -> GenerateFailure {
        match self {
            Self::NotTheContract(reason) => GenerateFailure::Unreadable { reason },
            Self::WrongCount(received) => GenerateFailure::WrongCount {
                requested,
                received,
            },
        }
    }
}

/// Read one reply: what it cost, and what it said.
pub(super) fn read(
    reply: Reply,
    wanted: usize,
    key: &str,
) -> (Attempt, Result<Vec<GeneratedMutant>, Answered>) {
    let usage = reply.usage.into();
    let Some(choice) = reply.choices.into_iter().next() else {
        return (
            Attempt {
                usage,
                finish: None,
            },
            Err(Answered::Fatal(GenerateFailure::NoAnswer)),
        );
    };
    let attempt = Attempt {
        usage,
        finish: choice.finish_reason.clone(),
    };
    if let Some(refusal) = choice.message.refusal.filter(|said| !said.is_empty()) {
        let refused = GenerateFailure::Refused {
            reason: quote(&refusal, key),
        };
        return (attempt, Err(Answered::Fatal(refused)));
    }
    if choice.finish_reason.as_deref() == Some("length") {
        let cut = GenerateFailure::Truncated {
            limit: MAX_COMPLETION_TOKENS,
        };
        return (attempt, Err(Answered::Fatal(cut)));
    }
    let said = choice.message.content.unwrap_or_default();
    let read = match serde_json::from_str::<GeneratedMutantsResponse>(&said) {
        Ok(answer) if answer.mutants.len() == wanted => Ok(answer.mutants),
        Ok(answer) => Err(Answered::Correctable(Correctable::WrongCount(
            answer.mutants.len(),
        ))),
        Err(error) => Err(Answered::Correctable(Correctable::NotTheContract(redact(
            &error.to_string(),
            key,
        )))),
    };
    (attempt, read)
}

/// What the provider answers with, as much of it as is read.
#[derive(Debug, Deserialize)]
pub(super) struct Reply {
    /// The model that answered, which is the one recorded.
    #[serde(default)]
    pub(super) model: String,
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
