//! How tremula asks a model whether one mutation can change anything at all.
//!
//! The seam is [`EquivalenceJudge`], shaped like [`MutantGenerator`] and isolated
//! behind a provider the same way: one question about one mutation, one answer
//! carrying a claim, an input that would prove the claim, which model answered,
//! and what the call cost.
//!
//! # What the answer is worth
//!
//! Nothing, on its own — and this module is built around that. It was measured:
//! asked to name the input separating a mutation from the original, a model wrote
//! down concrete inputs in the form it was asked for, and four out of four of the
//! sampled ones were false when executed. A claim of `equivalent` is therefore not
//! a finding, and a claim of `distinguishable` is not one either. What has standing
//! is [`Witness::call`], because a call can be run against both versions and the
//! two results compared.
//!
//! So the division of labour is: the model's job is to *find candidate inputs*,
//! which is a search problem it is good at, and the deciding is done by running
//! them. That is why [`Witness::expect_original`] and [`Witness::expect_mutant`]
//! exist and are never read by anything that decides: asking for both results is
//! what makes a model look for an input rather than reach for a phrase, and
//! grading its prediction would be grading the wrong thing. The difference between
//! the two observed results is the evidence; the prediction of them is not.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::generate::Attempt;

pub mod prompt;

/// A source of judgements about one mutation at a time.
pub trait EquivalenceJudge {
    /// Ask whether the mutation `request` describes can change what the function
    /// does, and for the input that would show it.
    ///
    /// # Errors
    ///
    /// Returns [`JudgeError`] when no answer this contract can read was obtained,
    /// after whatever retry the judge owns. The error carries the attempts the
    /// call was charged for.
    fn judge(&self, request: &JudgementRequest) -> Result<JudgementOutcome, JudgeError>;
}

/// One mutation to judge, with everything a judge is allowed to see.
///
/// The function's source rather than the file's: what a model needs in order to
/// find a reachable input is the guards standing between the arguments and the
/// changed expression, and those are inside the function. It also keeps the ask
/// small enough to be cheap, which matters when there is one of these per
/// survivor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgementRequest {
    /// The file the function lives in, POSIX-style and relative to the project
    /// root, exactly as the manifest spells it.
    pub file: String,
    /// The function's own source text, as the run recorded it.
    pub source: String,
    /// The text the mutation replaces, quoted from that source.
    pub original: String,
    /// The text it puts in place of it.
    pub replacement: String,
}

/// What a model says about one mutation.
///
/// A wire type of the provider exchange, and it refuses a member it does not name
/// for the same reason the generation's answer does: the schema sent with the
/// request forbids one, and a receiver that quietly dropped a field would be
/// declining to enforce the contract it had just sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Judgement {
    /// Whether the model says the mutation can be told apart from the original.
    pub claim: Claim,
    /// The input that would show it, when the model could name one that a probe
    /// can evaluate. Null is a real answer, and the honest one whenever the
    /// separating input needs something a literal cannot spell.
    #[serde(default)]
    pub witness: Option<Witness>,
}

/// What a model claims about a mutation.
///
/// Two values and no third, because a judge that may answer "unsure" answers it,
/// and an unsure answer is what the execution check already produces for free when
/// there is no witness to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Claim {
    /// There is an input for which the two versions do different things.
    Distinguishable,
    /// There is no such input.
    Equivalent,
}

/// The input a claim stands on, in the only form that can be run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    /// One call of the function by its own name, with literal arguments only —
    /// what a probe is willing to evaluate.
    pub call: String,
    /// What the model expects the unmutated function to do with that call.
    ///
    /// Never read by anything that decides a classification. A model's prediction
    /// of a result is not evidence about the result; the two observed results are.
    /// It is asked for because asking for it is what makes a model look for an
    /// input it can defend, which is the whole value of the field.
    pub expect_original: String,
    /// What it expects the mutated function to do. Never read, as above.
    pub expect_mutant: String,
}

/// A judgement call that produced an answer, and the account of what it took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgementOutcome {
    /// The answer, as it was given.
    pub judgement: Judgement,
    /// The model that answered, spelled the way the provider names that exact
    /// snapshot — read from the answer, never from what was asked for.
    pub model_resolved: String,
    /// Every attempt, in order, including the ones that failed before the last
    /// one succeeded.
    pub attempts: Vec<Attempt>,
}

/// A judgement call that produced no answer, and what it still cost.
#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct JudgeError {
    /// What went wrong.
    pub kind: JudgeFailure,
    /// The attempts the call was charged for before it gave up. Empty only when
    /// nothing was ever sent.
    pub attempts: Vec<Attempt>,
}

impl JudgeError {
    /// A failure that happened before anything was sent, so nothing was owed.
    #[must_use]
    pub fn unsent(kind: JudgeFailure) -> Self {
        Self {
            kind,
            attempts: Vec::new(),
        }
    }
}

/// Why a judgement call produced no answer.
///
/// The distinctions are the ones a person has to act on differently, and the
/// advice differs from the generating side's even where the event is the same: an
/// answer cut off at the token limit is not fixed by asking for fewer mutations
/// when nobody asked for any.
#[derive(Debug, thiserror::Error)]
pub enum JudgeFailure {
    /// No key in the environment.
    #[error(
        "no model provider key in the environment; set `{variable}` to a key that may call the model this judge names"
    )]
    MissingCredential {
        /// The environment variable that was looked for.
        variable: String,
    },
    /// The provider would not accept the key.
    #[error(
        "the model provider rejected the key (HTTP 401); check that `{variable}` holds a current key for the account that owns the model"
    )]
    CredentialRejected {
        /// The environment variable the key was read from.
        variable: String,
    },
    /// The call never got an answer.
    #[error(
        "could not reach the model provider: {reason}; check the network and any proxy, then try again"
    )]
    Unreachable {
        /// What the transport reported.
        reason: String,
    },
    /// The provider is rate limiting this key.
    #[error(
        "the model provider is rate limiting this key (HTTP 429){}; wait and run again, or check the account's rate limit and quota",
        if *retried { " and did it again after the wait it asked for" } else { " and did not say how long to wait" }
    )]
    RateLimited {
        /// Whether the wait this judge owns had already been spent on this call,
        /// which is the only thing that makes the message say so.
        retried: bool,
    },
    /// The model declined to answer.
    #[error(
        "the model refused to judge this mutation: {reason}; the survivor stays undecided, which is what a person reviews it as"
    )]
    Refused {
        /// The refusal, as the model worded it.
        reason: String,
    },
    /// The answer ran out of room.
    #[error(
        "the answer stopped at the token limit ({limit}) before it was complete; the function is long enough that judging it does not fit in one answer — raise the limit, or judge it by hand"
    )]
    Truncated {
        /// The completion-token limit that was hit.
        limit: u32,
    },
    /// The provider answered with nothing in it.
    #[error(
        "the model provider returned no answer at all, only an empty list of choices; run again, and if it repeats, check the provider's status"
    )]
    NoAnswer,
    /// What came back was not this provider's own answer envelope.
    #[error(
        "what the model provider answered was not the shape this adapter reads, before any of the model's own words: {reason}; check that the endpoint named speaks this provider's chat completions protocol, and whether something in between answered instead"
    )]
    UnreadableEnvelope {
        /// Why the envelope could not be read.
        reason: String,
    },
    /// The model's answer was not what this contract reads, twice.
    #[error(
        "the answer was not the JSON object this contract asks for, even after one corrected retry: {reason}; check that the model named still supports a schema-constrained answer"
    )]
    Unreadable {
        /// Why the answer could not be read.
        reason: String,
    },
    /// The provider refused the call for some other reason.
    #[error(
        "the model provider refused the call (HTTP {status}): {detail}; the message above is the provider's own"
    )]
    Rejected {
        /// The status it answered with.
        status: u16,
        /// What it said, with anything key-shaped taken out.
        detail: String,
    },
}
