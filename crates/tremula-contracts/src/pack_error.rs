//! The structured failure a language pack reports when it cannot finish its
//! work. A pack prints this document as the last line of its stdout and exits
//! with a non-zero status, so the core learns *where* the work stopped without
//! parsing human-readable diagnostics.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A pack's failure report. The single `error` key keeps this document
/// distinguishable from any other JSON a pack may print.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PackError {
    /// What went wrong.
    pub error: PackErrorDetail,
}

/// Why a pack stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PackErrorDetail {
    /// The step of the pack's work that failed.
    pub stage: Stage,
    /// Stable machine-readable identifier for this kind of failure, such as
    /// `baseline_failed`. Codes are `lower_snake_case` and outlive any wording
    /// change to `message`.
    pub code: String,
    /// Human-readable explanation of the failure. For display only: consumers
    /// branch on `stage` and `code`, never on this text.
    pub message: String,
}

/// The steps of a pack run, in the order a pack performs them, plus the steps of
/// the calls that are not a run. A `stage` tells the core how far the pack got,
/// which distinguishes a project problem such as a failing baseline from an
/// adapter problem such as a failed plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum Stage {
    /// Checking the environment before any work starts.
    Preflight,
    // A pack only ever reports this to whoever asked it for spans, so no core
    // that predates the value can meet it: a core that does not know the `spans`
    // subcommand never calls it, and nothing else in a run produces the stage.
    // The `Unknown` fallback below is what makes the next stage after this one
    // safe for the calls where that reasoning does not hold.
    /// Reading a source file's functions, before any manifest exists.
    Spans,
    // Reported only to whoever asked for a probe, on the same reasoning as the
    // stage above it: a core that does not know the `probe` subcommand never calls
    // it, and nothing in a run produces the stage.
    /// Running one input against both versions of a function.
    Probe,
    /// Language-level validation of the manifest.
    Validate,
    /// The unmutated reference run.
    Baseline,
    /// Turning the manifest into work items for the execution backend.
    Plan,
    /// Applying mutants and running the suite.
    Execute,
    /// Reading results back out of the backend's state.
    Collect,
    /// A step this consumer does not know, reported by a newer pack. The failure
    /// is still a failure and still carries a code and a message; only where it
    /// happened is beyond what this consumer can name.
    #[serde(other)]
    Unknown,
}
