//! What happened when one input was run against both versions of a function.
//!
//! A survivor comes with a claim about it and, when the claim is worth anything,
//! with one input said to separate the mutation from the original. This document
//! is the account of running that input: what each version did with it, and
//! whether the two did the same thing. It is the only evidence in the whole of
//! triage that was not produced by asking a model.
//!
//! Two things it deliberately does not say. It does not say the mutation is
//! equivalent — [`ProbeOutcome::Indistinguishable`] means one input failed to tell
//! two functions apart, and no number of inputs that fail to do that adds up to a
//! proof that none can. And it does not say the difference matters to the program:
//! the two versions are called directly, so a difference here is a difference at
//! the level of the function, which the program around it may or may not be able
//! to reach.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The result of running one witness call against both versions of a function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProbeReport {
    /// Contract version of this document.
    pub schema_version: String,
    /// The file the function lives in, POSIX-style and relative to the root the
    /// probe was pointed at. Echoed back so a caller can tell that the answer is
    /// about the file it asked about.
    pub file: String,
    /// The witness call, exactly as the caller wrote it.
    pub call: String,
    /// Whether the two versions did the same thing.
    pub outcome: ProbeOutcome,
    /// Why no comparison could be made. Present when, and only when, `outcome` is
    /// `undecided`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undecided: Option<Undecided>,
    /// What the unmutated function did. Absent when it was never observed
    /// consistently enough to describe, which is what a nondeterministic side
    /// means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<Observation>,
    /// What the mutated function did, on the same terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutant: Option<Observation>,
    /// How many times each version was run, each time in a subprocess of its own.
    /// More than one because a side that disagrees with itself cannot be compared
    /// with anything.
    pub runs_per_side: u32,
}

/// Whether one input told the two versions of a function apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum ProbeOutcome {
    /// The two versions did observably different things on this input.
    Differs,
    /// They did the same thing on this input. Not a finding of equivalence: it is
    /// one input, and the claim it fails to support is a claim about all of them.
    Indistinguishable,
    /// No comparison could be made. `undecided` says why.
    Undecided,
    /// An outcome this consumer does not know, reported by a newer pack.
    #[serde(other)]
    Unknown,
}

/// Why a probe could not compare the two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum Undecided {
    /// One version did not agree with itself across its own runs, so there is
    /// nothing stable to compare the other version against.
    Nondeterministic,
    /// A value came back whose type compares by identity rather than by content,
    /// so two of them made in two processes cannot be told equal or unequal.
    Incomparable,
    /// The call was not the form a probe evaluates: an argument that is not a
    /// literal, or anything else the whitelist refuses.
    UnsafeWitness,
    /// The name the call uses is a method rather than a function of the module.
    /// A method needs a receiver, and nothing in a witness says what it should be.
    Method,
    /// The module defines no function of that name at all, so there was nothing to
    /// run — which is a fact about the witness, not about the mutation.
    NoSuchFunction,
    /// One version did not finish inside the time limit, so what it would have
    /// done is unknown.
    TimedOut,
    /// A reason this consumer does not know, reported by a newer pack.
    #[serde(other)]
    Unknown,
}

/// What one version of the function did with the witness call.
///
/// Every field of it is the agreed account of several runs: a side that produced
/// two different accounts is reported as nondeterministic instead, and never
/// arrives here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Observation {
    /// How the call ended.
    pub ended: Ending,
    /// The value it returned, rendered by the language's own means, or absent when
    /// the call raised instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The type of the returned value, or of the exception that was raised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// The exception's own message, with no traceback in it: a traceback carries
    /// line numbers, and the two versions have different line numbers by
    /// construction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Whatever the call wrote to standard output, captured and compared like any
    /// other observable result.
    pub stdout: String,
}

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum Ending {
    /// It returned a value.
    Returned,
    /// It raised an exception.
    Raised,
    /// An ending this consumer does not know, reported by a newer pack.
    #[serde(other)]
    Unknown,
}
