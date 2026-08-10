//! The reference judge: one `OpenAI` chat completion about one mutation.
//!
//! The same transport as the generator, the same two retries counted apart, and a
//! different question with a different schema. What is not shared is the
//! vocabulary of failure: the advice for an answer that ran out of room is not the
//! advice a generation gives, because nobody asked for a number of anything.

use std::time::Duration;

use serde_json::Value;

use crate::generate::{
    Attempt,
    judge::{
        EquivalenceJudge, JudgeError, JudgeFailure, Judgement, JudgementOutcome, JudgementRequest,
        prompt,
    },
    openai::{
        MAX_COMPLETION_TOKENS,
        answer::{self, Attempted, Spoke},
        transport::{self, Declined, Strictness, Unsent, strict_schema},
    },
};

pub use crate::generate::openai::{ENDPOINT, KEY_VARIABLE, TIMEOUT};

/// What the schema is called in the request, as the provider reports it back.
pub const SCHEMA_NAME: &str = "tremula_judgement";

/// A judge that asks an OpenAI-compatible endpoint about one mutation.
#[derive(Debug, Clone)]
pub struct OpenAiJudge {
    model: String,
    endpoint: String,
    timeout: Duration,
}

impl OpenAiJudge {
    /// Name the exact model snapshot to ask.
    ///
    /// A family name resolves to whichever version is current, and what the answer
    /// reports is what gets written into `triage.json` beside every classification
    /// it produced.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            endpoint: ENDPOINT.to_owned(),
            timeout: TIMEOUT,
        }
    }

    /// Point this judge at another endpoint that speaks the same protocol.
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

impl EquivalenceJudge for OpenAiJudge {
    /// Ask about one mutation, waiting out one rate limit and correcting one
    /// answer.
    ///
    /// # Errors
    ///
    /// Returns [`JudgeError`] with the attempts already paid for: no key, a key
    /// the provider rejected, a provider that could not be reached, would not
    /// answer, or answered in some other protocol, a refusal, an answer cut off at
    /// the token limit, an answer with no choices in it, or an answer that broke
    /// the contract twice.
    fn judge(&self, request: &JudgementRequest) -> Result<JudgementOutcome, JudgeError> {
        let (key, client) = match transport::ready(self.timeout) {
            Ok(ready) => ready,
            Err(Unsent::MissingCredential) => {
                return Err(JudgeError::unsent(JudgeFailure::MissingCredential {
                    variable: KEY_VARIABLE.to_owned(),
                }));
            }
            Err(Unsent::NoClient(reason)) => {
                return Err(JudgeError::unsent(JudgeFailure::Unreachable { reason }));
            }
        };
        let assembled = prompt::assemble(request);
        let mut correction: Option<String> = None;
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
            let (attempt, model, spoke) =
                match answer::attempt::<Judgement>(&client, &self.endpoint, &key, &payload) {
                    Attempted::Spoke(attempt, model, spoke) => (attempt, model, spoke),
                    Attempted::RateLimited(wait) => {
                        attempts.push(Attempt::default());
                        if let (Some(wait), false) = (wait, rate_limit_retried) {
                            rate_limit_retried = true;
                            std::thread::sleep(wait);
                            continue;
                        }
                        return Err(JudgeError {
                            kind: JudgeFailure::RateLimited {
                                retried: rate_limit_retried,
                            },
                            attempts,
                        });
                    }
                    other => {
                        attempts.push(Attempt::default());
                        return Err(JudgeError {
                            kind: refusal(other),
                            attempts,
                        });
                    }
                };
            attempts.push(attempt);
            let reason = match spoke {
                Spoke::Answer(judgement) => {
                    return Ok(JudgementOutcome {
                        judgement,
                        model_resolved: model,
                        attempts,
                    });
                }
                Spoke::NotTheContract(reason) => reason,
                Spoke::Refused(reason) => {
                    return Err(JudgeError {
                        kind: JudgeFailure::Refused { reason },
                        attempts,
                    });
                }
                Spoke::Truncated => {
                    return Err(JudgeError {
                        kind: JudgeFailure::Truncated {
                            limit: MAX_COMPLETION_TOKENS,
                        },
                        attempts,
                    });
                }
                Spoke::NoAnswer => {
                    return Err(JudgeError {
                        kind: JudgeFailure::NoAnswer,
                        attempts,
                    });
                }
            };
            if correction_retried {
                return Err(JudgeError {
                    kind: JudgeFailure::Unreadable { reason },
                    attempts,
                });
            }
            correction_retried = true;
            correction = Some(prompt::correction_turn(&reason));
        }
    }
}

/// What an attempt that never reached the model was, in a judgement's own words.
fn refusal<T>(attempted: Attempted<T>) -> JudgeFailure {
    match attempted {
        Attempted::Unreachable(reason) => JudgeFailure::Unreachable { reason },
        Attempted::UnreadableEnvelope(reason) => JudgeFailure::UnreadableEnvelope { reason },
        Attempted::Declined(Declined::CredentialRejected) => JudgeFailure::CredentialRejected {
            variable: KEY_VARIABLE.to_owned(),
        },
        Attempted::Declined(Declined::Rejected { status, detail }) => {
            JudgeFailure::Rejected { status, detail }
        }
        // Both of the remaining shapes carry an answer, and an answer is read
        // rather than refused.
        Attempted::Spoke(..) | Attempted::RateLimited(_) => JudgeFailure::NoAnswer,
    }
}

/// The schema a model answering a judgement is held to.
///
/// Every member is listed as required, including the witness, because strict
/// server-side enforcement requires it — so "no witness" is spelled as a member
/// that is null rather than one that is absent.
#[must_use]
pub fn response_schema() -> Value {
    strict_schema::<Judgement>(Strictness::Adjusted)
}
