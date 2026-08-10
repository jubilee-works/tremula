//! What a second look at a run's survivors came to.
//!
//! A run's report says what the test suite did. It cannot say whether a survivor
//! is a gap in the suite or a mutation that changes nothing, and it does not try:
//! `report.json` is never rewritten, and this is a document beside it holding a
//! different question's answer.
//!
//! # What this document is for
//!
//! Putting a person's attention in the right order. It is not a filter and it
//! retires nothing: no classification here means a survivor may be discarded, and
//! a survivor leaves a project's attention only when a person dismisses it. The
//! three classifications differ in what was established, not in how much they are
//! to be trusted:
//!
//! * [`Classification::DistinguishedAtFunctionLevel`] — one input was run against
//!   both versions of the function and they did different things. Whether the
//!   program around the function can reach that input is **not** established. The
//!   name is long on purpose: two of the survivors in the measured sample were
//!   separable at the level of the function and unreachable in the program.
//! * [`Classification::SuspectedEquivalent`] — a model said the mutation cannot
//!   change what the function does, and nothing was run to check it. The weakest
//!   thing in the document, and named as a suspicion for that reason.
//! * [`Classification::Undecided`] — no comparison was made, and [`Undecided`]
//!   says which of the many reasons it was.
//!
//! # Why a model's answer is never a classification on its own
//!
//! It was measured. Asked to name the input separating a mutation from the
//! original, a model produced concrete inputs in the required form, and four out of
//! four of the sampled ones turned out to be false when executed. So the only
//! classification with anything behind it is the one an execution produced, and
//! `claim` and `witness` are kept as a record of what was asked and answered rather
//! than as evidence.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{manifest::Span, probe::ProbeReport, suppressions::DismissalReason};

/// The judged outcome of one run's survivors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Triage {
    /// Contract version of this document.
    pub schema_version: String,
    /// Which run's survivors these are.
    pub run: TriageRun,
    /// Who was asked about them, and what they were asked.
    pub judge: Judge,
    /// One entry per survivor, in the report's order.
    pub entries: Vec<TriageEntry>,
    /// The survivors a person had already dismissed, which were not asked about at
    /// all. Listed rather than left out, because a reader comparing this document
    /// with the run's report has to be able to account for every survivor in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissed: Vec<Dismissed>,
    /// Aggregate counts, consistent with `entries`.
    pub score: TriageScore,
    /// What the calls cost.
    pub spend: Spend,
    /// Statements that must accompany any presentation of these classifications.
    /// Written into the document rather than into a renderer, so that a reader who
    /// only has the file still meets them.
    pub caveats: Vec<String>,
}

/// The run this document is a second opinion on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TriageRun {
    /// Identifier shared with the run directory, the results, and the report.
    pub run_id: String,
    /// The document whose survivors were judged, relative to the run directory.
    /// Named rather than assumed: this is a claim about that document's verdicts,
    /// and a reader has to be able to tell which one.
    pub report: String,
    /// When the judging finished, RFC 3339.
    pub judged_at: String,
}

/// Who was asked, and what they were asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Judge {
    /// The model that was asked, as the caller named it.
    pub model: String,
    /// The model that answered, as it named itself. Absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_resolved: Option<String>,
    /// Which revision of the judging prompt was sent. A change in how survivors
    /// sort is traceable to a change in what was asked only if this is recorded.
    pub prompt_version: String,
}

/// What became of one survivor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TriageEntry {
    /// The manifest mutant this entry is about.
    pub mutant_id: String,
    /// Target file, POSIX-style and relative to the project root.
    pub file: String,
    /// The byte range the mutation replaces.
    pub span: Span,
    /// The text it replaces, as the manifest holds it.
    pub original: String,
    /// The text it puts there.
    pub replacement: String,
    /// What was established about it.
    pub classification: Classification,
    /// Why nothing was established. Present when, and only when, `classification`
    /// is `undecided`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undecided: Option<Undecided>,
    /// What the model claimed, when one answered. A record of the question, not
    /// evidence for the classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<Claim>,
    /// The input the model offered, when it offered one. Kept whether or not it
    /// turned out to separate anything, because a witness that did not is the most
    /// useful thing a reader of this document can see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub witness: Option<Witness>,
    /// What happened when that input was run. Absent when none was run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<ProbeReport>,
    /// One line for display, saying what happened in a person's terms. Not machine
    /// readable: everything a machine reads is in the fields above.
    pub detail: String,
}

/// What was established about a survivor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum Classification {
    /// One input was run against both versions of the function and they did
    /// different things. Whether the program can reach that input is unverified.
    DistinguishedAtFunctionLevel,
    /// A model said the mutation changes nothing, and nothing was run to check it.
    SuspectedEquivalent,
    /// No comparison was made. `undecided` says why.
    Undecided,
    /// A classification this consumer does not know, written by a newer producer.
    #[serde(other)]
    Unknown,
}

/// Why nothing was established about a survivor.
///
/// Long, and every entry earns its place: what a person does next differs for each
/// of these, and collapsing them into one "unknown" would leave every reader
/// guessing which of them they had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum Undecided {
    /// The model claimed a difference and named no input a probe could run.
    NoWitness,
    /// The input it named was run, and the two versions did the same thing. This
    /// is **not** evidence of equivalence: one input failed to separate them, and
    /// the claim it fails to support was a claim about every input.
    WitnessShowedNoDifference,
    /// A version of the function did not agree with itself across its own runs.
    Nondeterministic,
    /// A value came back that two processes cannot be said to agree about.
    Incomparable,
    /// The input was not one a probe evaluates: an argument that is not a literal.
    UnsafeWitness,
    /// The name the input used is a method, which has no receiver a witness names.
    Method,
    /// The module defines no function of that name, so there was nothing to run.
    NoSuchFunction,
    /// A version did not finish inside the time limit.
    TimedOut,
    /// No judgement could be obtained at all — the model, or the pack, could not
    /// answer about this survivor. The one reason here that is about the tools
    /// rather than about the mutation.
    NoJudgement,
    /// A reason this consumer does not know, written by a newer producer.
    #[serde(other)]
    Unknown,
}

/// What a model claimed about a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum Claim {
    /// It said there is an input for which the two versions do different things.
    Distinguishable,
    /// It said there is no such input.
    Equivalent,
    /// A claim this consumer does not know, written by a newer producer.
    #[serde(other)]
    Unknown,
}

/// The input a model offered, as it offered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Witness {
    /// One call of the function, with literal arguments.
    pub call: String,
    /// What the model expected the unmutated function to do with it.
    ///
    /// Recorded and never compared against what happened. A prediction of a result
    /// is not evidence about the result, and grading the prediction would be
    /// grading the wrong thing — what the probe compares is the two results.
    pub expect_original: String,
    /// What it expected the mutated function to do. Recorded, as above.
    pub expect_mutant: String,
}

/// One survivor that was not judged, because a person had already dismissed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Dismissed {
    /// The manifest mutant this is about.
    pub mutant_id: String,
    /// Target file, POSIX-style and relative to the project root.
    pub file: String,
    /// The text the mutation replaces.
    pub original: String,
    /// The text it puts there.
    pub replacement: String,
    /// Why the person dismissed it.
    pub reason: DismissalReason,
    /// When they did, RFC 3339, as the suppression records it.
    pub dismissed_at: String,
}

/// Aggregate counts over one triage. Every field is a plain count of `entries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TriageScore {
    /// Survivors this document is about.
    pub survivors: u32,
    /// Those one input separated at the level of the function.
    pub distinguished_at_function_level: u32,
    /// Those a model called equivalent, unchecked.
    pub suspected_equivalent: u32,
    /// Those nothing was established about.
    pub undecided: u32,
}

/// What the judging cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Spend {
    /// How many calls were made to the model, the failed ones included: a provider
    /// charges for those too.
    pub calls: u32,
    /// Tokens the prompts cost.
    pub prompt_tokens: u64,
    /// Tokens the answers cost.
    pub completion_tokens: u64,
    /// What the provider reported as the sum.
    pub total_tokens: u64,
}
