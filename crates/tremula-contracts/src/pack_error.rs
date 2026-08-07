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

/// The steps of a pack run, in the order a pack performs them. A `stage` tells
/// the core how far the pack got, which distinguishes a project problem such as
/// a failing baseline from an adapter problem such as a failed plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Checking the environment before any work starts.
    Preflight,
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
}
