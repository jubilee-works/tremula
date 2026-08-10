//! How tremula asks a model for mutants.
//!
//! The seam is [`MutantGenerator`]: one request about one function, one answer
//! carrying the mutations, which model produced them, and what the call cost.
//! Nothing above this seam names a provider, and nothing below it reads a file
//! or a byte offset — a generator is handed the text to mutate and answers with
//! text, which is why [`GeneratedMutant`] carries `original` and no span. Where
//! that text sits in the file is decided by the caller, against the file's own
//! bytes, long after the answer arrived.
//!
//! # Who retries what
//!
//! A generator owns exactly the failures it can see for itself, and retries each
//! of those once. Each is a budget of its own and neither is borrowed from the
//! other: waiting out a rate limit leaves the corrected ask unspent, and asking
//! again with a defect list leaves the wait unspent. Everything a generator
//! cannot see — whether `original` is really in the function, whether the
//! replacement compiles once spliced in — belongs to the caller, which is why
//! [`GenerationRequest::feedback`] exists: the caller asks again with the defects
//! it found, and the generator puts them back in front of the model.
//!
//! | failure | owner | what happens |
//! | --- | --- | --- |
//! | the answer is not the JSON this contract asks for, carries a member the contract forbids, or holds a number of mutants nobody asked for | the generator | one corrected retry, with the defect list attached |
//! | the model refused, the answer stopped at the token limit, or there was no answer at all | the generator | no retry: three distinct failures, each naming what to do next |
//! | what came back was not this provider's protocol at all | the generator | no retry: a correction is addressed to a model, and no model spoke |
//! | the provider is rate limiting | the generator | one wait when it said how long to wait, otherwise a failure that names the quota; either way the attempt is on the ledger |
//! | `original` is not in the function, or is there more than once | the caller | ask again with [`CorrectionFeedback`] |
//! | the replacement does not compile once spliced into the file | the caller | ask again with [`CorrectionFeedback`] |
//! | the mutation cannot change what the function does | the caller | discard it; asking again does not help, which was measured |
//!
//! Every failure message says only what actually happened. A message that speaks
//! of a retry was reached by a path that spent one, and the ledger has the
//! attempt to prove it.
//!
//! # What a call costs
//!
//! Every attempt is recorded, including the attempts that failed, because a
//! provider charges for those too. [`GenerationOutcome::attempts`] and
//! [`GenerateError::attempts`] carry the same ledger, so a caller that books
//! spend before it calls can settle the account whichever way the call went.
//! The totals are the caller's arithmetic: an attempt reports only its own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod choose;
pub mod command;
pub mod enrich;
pub mod failures;
pub mod openai;
pub mod prompt;
pub mod round;

/// A source of mutants for one function at a time.
pub trait MutantGenerator {
    /// Ask for mutants of the function `request` describes.
    ///
    /// # Errors
    ///
    /// Returns [`GenerateError`] when no answer this contract can read was
    /// obtained, after whatever retry the generator owns. The error carries the
    /// attempts the call was charged for.
    fn generate(&self, request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError>;
}

/// One function to mutate, with everything a generator is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationRequest {
    /// The file the function lives in, POSIX-style and relative to the project
    /// root: the same spelling a manifest uses for the same file, because the
    /// answer repeats it back.
    pub file: String,
    /// The function's own source text, exactly as the file spells it.
    pub source: String,
    /// The tests that cover the function. Sending them is what a model needs to
    /// aim past what the suite already catches; an empty list asks about the
    /// function alone, which was measured to be the weaker question.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<CoveringTest>,
    /// Stretches of the function's own source a mutation must not aim at, quoted
    /// verbatim.
    ///
    /// A docstring, the type of an annotated assignment: code that carries no
    /// behaviour a test could see, so a mutation there is one no suite could be
    /// blamed for missing. Naming them in the request is what stops a model
    /// spending a proposal on one, and it is cheaper than the alternative — the
    /// caller refuses such a mutation anyway, and a refusal costs a correction.
    /// Both happen: this is what a model is told, and the refusal stands behind it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<String>,
    /// How many mutants to ask for.
    ///
    /// Three to five is the recommended range and the range that was measured.
    /// It is not an invariant of this type: a caller under a cap of its own may
    /// ask for fewer, and a generator answers with whatever number it was asked
    /// for or fails saying it could not.
    pub mutant_count: usize,
    /// What was wrong with the previous answer about this same function.
    ///
    /// Absent on a first ask. Present when the caller has checked an answer
    /// against the file and is asking again — the defects reach the model
    /// instead of being spent on a fresh question that would repeat them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<CorrectionFeedback>,
}

/// One test file that exercises the function being mutated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveringTest {
    /// The test file, spelled the way the project spells it.
    pub file: String,
    /// Its source text.
    pub source: String,
}

/// What was wrong with the last answer, in the caller's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionFeedback {
    /// The defects to put back in front of the model. An empty list is
    /// pointless rather than invalid: it asks again and says nothing.
    pub defects: Vec<Defect>,
}

/// One thing wrong with one proposed mutation.
///
/// The vocabulary is the caller's, not this crate's: a generator forwards
/// whatever it is handed, so a checker added later needs no change here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defect {
    /// What kind of defect it is, as a short machine-readable name such as
    /// `original_not_found`.
    pub defect: String,
    /// The particulars: which text, how many matches, what the compiler said.
    pub detail: String,
}

/// One mutation, as a generator describes it.
///
/// There is no span. A model asked for byte offsets got them right in under one
/// per cent of cases, and asked for the text instead got text a caller could
/// find in four out of five — so `original` is the address, and finding it is
/// the caller's job.
///
/// This is a wire type of the provider exchange, and it refuses a member it does
/// not name. The schema sent with the request forbids one, so an answer that
/// carries a `span` anyway is a model that did not do as it was told: a receiver
/// that quietly dropped the field would be declining to enforce the contract it
/// had just sent, and would take an answer built on offsets nobody asked for as
/// if it were the one that was. Reported instead, it is correctable — the defect
/// goes back and the answer is asked for again. Nothing on disk is read through
/// this type, so the rule that a reader of a published contract ignores what it
/// does not recognise does not reach here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratedMutant {
    /// The file to change, as the request spelled it.
    pub file: String,
    /// The text to replace, copied verbatim from the function and expected to
    /// occur there exactly once.
    pub original: String,
    /// The text to put in its place.
    pub replacement: String,
    /// Which real mistake this simulates, and why a test might miss it.
    pub description: String,
}

/// The whole answer: an object with one array in it.
///
/// A root object rather than a bare array, because a provider that enforces a
/// schema server side requires one, and because a later version can add a key
/// beside `mutants` — as a change to this type, which is where the sent schema is
/// derived from, so the model is told to send the new key in the same breath as
/// this side learns to read it.
///
/// Closed for the reason [`GeneratedMutant`] is: the request forbids a member
/// nobody named, and an answer is held to what the request asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratedMutantsResponse {
    /// The mutations, in the order the model proposed them.
    pub mutants: Vec<GeneratedMutant>,
}

/// A call that produced mutants, and the account of what it took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationOutcome {
    /// The mutations, as they were answered.
    pub mutants: Vec<GeneratedMutant>,
    /// The model that answered, spelled the way the provider names that exact
    /// snapshot — read from the answer, never from what was asked for, because
    /// a family name resolves to whichever version is current and a
    /// reproduction cannot rely on that.
    pub model_resolved: String,
    /// Every attempt, in order, including the ones that failed before the last
    /// one succeeded.
    pub attempts: Vec<Attempt>,
}

/// One request-and-answer, whatever came of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    /// What the provider said this attempt consumed. Zero throughout when it
    /// said nothing, which is what a rejected call reports.
    pub usage: Usage,
    /// Why the answer ended, as the provider named it — `stop`, `length`, or
    /// whatever else it reports. Absent when there was no answer to end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

/// Tokens consumed by one attempt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens the prompt cost.
    #[serde(default)]
    pub prompt: u64,
    /// Tokens the answer cost.
    #[serde(default)]
    pub completion: u64,
    /// What the provider reported as the sum. Taken as given rather than added
    /// up here: a provider that counts something extra is reporting a fact
    /// about the bill.
    #[serde(default)]
    pub total: u64,
}

/// A generation call that produced no mutants, and what it still cost.
#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct GenerateError {
    /// What went wrong.
    pub kind: GenerateFailure,
    /// The attempts the call was charged for before it gave up. Empty only when
    /// nothing was ever sent.
    pub attempts: Vec<Attempt>,
}

impl GenerateError {
    /// A failure that happened before anything was sent, so nothing was owed.
    #[must_use]
    pub fn unsent(kind: GenerateFailure) -> Self {
        Self {
            kind,
            attempts: Vec::new(),
        }
    }
}

/// Why a generation call produced no mutants.
///
/// The distinctions are the ones a person has to act on differently: a refusal
/// is about what was asked, a truncated answer is about how much was asked for,
/// and an empty answer is about the provider. Collapsing them into one message
/// would leave every reader guessing which of the three they had.
#[derive(Debug, thiserror::Error)]
pub enum GenerateFailure {
    /// No key in the environment.
    #[error(
        "no model provider key in the environment; set `{variable}` to a key that may call the model this generator names"
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
        /// Whether the wait this generator owns had already been spent on this
        /// call, which is the only thing that makes the message say so.
        retried: bool,
    },
    /// The model declined to answer.
    #[error(
        "the model refused to answer: {reason}; ask about another function, or review whether this one's source is something the provider's policy allows"
    )]
    Refused {
        /// The refusal, as the model worded it.
        reason: String,
    },
    /// The answer ran out of room.
    #[error(
        "the answer stopped at the token limit ({limit}) before it was complete; ask for fewer mutants, or raise the limit"
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
    ///
    /// Distinct from [`Self::Unreadable`], which is about what the model said
    /// inside a well-formed envelope. This one is reached before any of the
    /// model's words exist, so there is nothing to correct and nothing is asked
    /// again — and the message may not speak of a retry, because none was spent.
    #[error(
        "what the model provider answered was not the shape this adapter reads, before any of the model's own words: {reason}; check that the endpoint named speaks this provider's chat completions protocol, and whether something in between answered instead"
    )]
    UnreadableEnvelope {
        /// Why the envelope could not be read.
        reason: String,
    },
    /// The model's answer was not what this contract reads, twice.
    ///
    /// Only reached once the corrected ask has been spent, which is what lets the
    /// message say so.
    #[error(
        "the answer was not the JSON object this contract asks for, even after one corrected retry: {reason}; check that the model named still supports a schema-constrained answer"
    )]
    Unreadable {
        /// Why the answer could not be read.
        reason: String,
    },
    /// The answer held the wrong number of mutants, twice.
    ///
    /// Reached the same way [`Self::Unreadable`] is, and says the same about it.
    #[error(
        "the model answered with {received} mutants where {requested} were asked for, even after one corrected retry; ask for a number in the range this generator recommends, or lower it"
    )]
    WrongCount {
        /// How many were asked for.
        requested: usize,
        /// How many arrived.
        received: usize,
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
