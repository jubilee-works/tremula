//! Neutral test-runner signals. Backend vocabulary is translated by the
//! language pack before it reaches this contract, so the core never matches on
//! a backend's enum values or magic strings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What one test-suite execution reported, in language-neutral terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunnerResult {
    /// Neutral classification of how the runner exited.
    pub exit_class: ExitClass,
    /// Tests that passed.
    pub passed: u32,
    /// Tests that failed an assertion or raised inside the test body.
    pub failed: u32,
    /// Tests that errored during setup or teardown.
    pub errors: u32,
    /// Tests that were skipped.
    pub skipped: u32,
    /// Tests collected before execution.
    pub collected: u32,
    /// SHA-256 of the sorted collected test identifiers. Detects a changed test
    /// set even when `collected` is unchanged.
    pub collected_ids_hash: String,
    /// Whether collection itself failed.
    pub collect_error: bool,
    /// Whether the runner stopped the suite on its own time limit.
    pub timed_out: bool,
    /// Wall-clock duration of the run.
    pub duration_ms: u64,
}

/// Neutral runner exit classification. The Python pack maps pytest exit codes
/// onto these: 0 to `ok`, 1 to `test_failures`, 5 to `no_tests`, 2 to
/// `interrupted`, 3 and 4 to `infra_error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExitClass {
    /// The suite ran and every test passed.
    Ok,
    /// The suite ran and reported failures or errors.
    TestFailures,
    /// The suite ran but collected no tests.
    NoTests,
    /// The run was interrupted before finishing.
    Interrupted,
    /// The runner itself failed, through bad usage or an internal error.
    InfraError,
}
