//! The unmutated reference run. Mutant runs are compared against its collected
//! test set, so it reuses the same runner shape.
//!
//! Whether a baseline qualifies as a reference is derived, not recorded: a
//! usable baseline exits `ok` with at least one passing test and no failures or
//! errors. The pack reports an unusable baseline through its error channel.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runner::RunnerResult;

/// Result of running the suite with no mutation applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Baseline {
    /// Contract version of this document.
    pub schema_version: String,
    /// Identifier of the run this baseline belongs to, so a stale baseline
    /// paired with fresh results is detectable.
    pub run_id: String,
    /// Signals from the unmutated run.
    pub runner: RunnerResult,
}
