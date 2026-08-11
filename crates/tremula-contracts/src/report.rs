//! Verdicts decided by the core. This is what machines read; the console
//! rendering is a view over this document.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{manifest::Span, results::Location};

/// The judged outcome of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    /// Contract version of this document.
    pub schema_version: String,
    /// Run provenance.
    pub run: RunMeta,
    /// One verdict per manifest mutant, in manifest order.
    pub verdicts: Vec<MutantVerdict>,
    /// Aggregate counts, consistent with `verdicts`.
    pub score: Score,
    /// Process exit code the run reported.
    pub exit_code: u8,
    /// Statements that must accompany any presentation of these results, such
    /// as that survived mutants are not proven non-equivalent.
    pub caveats: Vec<String>,
}

/// Who produced this report, from what, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunMeta {
    /// Identifier shared with the run directory and the results document.
    pub run_id: String,
    /// Version of the tremula core that judged this run.
    pub tremula_version: String,
    /// Version of the decision rules used, so verdicts stay attributable.
    pub decision_rules_version: String,
    /// Project root, as given on the command line.
    pub project: String,
    /// Whatever `tremula run --tests` was given, in that order and spelled that
    /// way, and handed straight to the test runner. Empty means the project's own
    /// default collection, which is not the same as selecting nothing: it is the
    /// only record of how to run the same suite again.
    ///
    /// Not `tremula generate --tests`, which names test files to read and show a
    /// model. These are selectors a runner resolves, and nothing here reads them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    /// Source revision observed at run time, or absent when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_revision: Option<String>,
    /// Whether the working tree had uncommitted changes at run time.
    pub dirty: bool,
    /// When the run started, RFC 3339.
    pub started_at: String,
    /// When the run finished, RFC 3339.
    pub finished_at: String,
}

/// The verdict for one mutant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MutantVerdict {
    /// The manifest mutant this verdict is about.
    pub mutant_id: String,
    /// Target file, POSIX-style and relative to the project root.
    pub file: String,
    /// Byte range that was mutated.
    pub span: Span,
    /// Position of the mutation, when the pack reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// What the suite did to this mutant.
    pub verdict: Verdict,
    /// Refinement of the verdict, for example `killed_by_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// One-line summary for display only, such as `3 failed`. Not machine
    /// readable: counts live in the results document under the same `run_id`.
    pub detail: String,
}

/// What the existing test suite did to a mutant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The suite detected the mutant.
    Killed,
    /// The suite ran and did not detect the mutant.
    Survived,
    /// The suite exceeded its time limit under this mutant, counted as
    /// detected.
    Timeout,
    /// The attempt could not produce a judgement.
    RuntimeError,
    /// The mutation was never applied, which indicates an adapter defect.
    NotApplied,
    /// The mutant was scheduled but never executed.
    NotRun,
    /// The mutant was deliberately not executed.
    Skipped,
}

/// Aggregate counts over one run. Every field is a plain count of `verdicts`,
/// broken out per category because the exit code depends on which kind of
/// exclusion occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Score {
    /// Mutants in the manifest.
    pub total: u32,
    /// Mutants the suite detected, including those that timed out.
    pub killed: u32,
    /// Subset of `killed` that timed out rather than failing a test.
    pub timeout: u32,
    /// Mutants the suite ran but did not detect.
    pub survived: u32,
    /// Attempts that could not produce a judgement.
    pub runtime_error: u32,
    /// Mutations that were never applied.
    pub not_applied: u32,
    /// Mutants scheduled but never executed.
    pub not_run: u32,
    /// Mutants deliberately not executed.
    pub skipped: u32,
}
