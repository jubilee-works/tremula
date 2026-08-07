//! Raw per-mutant signals produced by a language pack. Contains no verdicts:
//! judgement belongs to the core (see [`crate::report`]).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runner::RunnerResult;

/// Everything one pack execution observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Results {
    /// Contract version of this document.
    pub schema_version: String,
    /// Identifier of the run these results belong to.
    pub run_id: String,
    /// Which pack produced them.
    pub pack: PackInfo,
    /// One entry per manifest mutant, including mutants that never ran.
    pub entries: Vec<ResultEntry>,
}

/// Provenance of the pack that produced a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PackInfo {
    /// Distribution name, for example `tremula-python`.
    pub name: String,
    /// Pack version.
    pub version: String,
    /// Contract version the pack implements, the same value documents carry as
    /// `schema_version`.
    pub contract_version: String,
}

/// What happened to one mutant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResultEntry {
    /// The manifest mutant this entry reports on.
    pub mutant_id: String,
    /// Neutral outcome of the attempt.
    pub execution_status: ExecutionStatus,
    /// Where the mutant landed, for humans and code review anchors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// Runner signals. Absent when no suite ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<RunnerResult>,
    /// Unified diff of the applied mutation, with `a/` and `b/` prefixes and
    /// project-relative paths, normalized by the pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// Whether captured output was truncated.
    #[serde(default)]
    pub truncated: bool,
    /// When the attempt finished, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Backend-specific values kept for cross-checking only, never for
    /// judgement.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub backend_raw: serde_json::Map<String, serde_json::Value>,
}

/// Neutral outcome of one mutation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// The mutation was applied and the suite ran to completion.
    Completed,
    /// The suite exceeded its time limit.
    Timeout,
    /// Deliberately not executed, for example filtered out.
    Skipped,
    /// The mutation could not be applied to the source.
    NotApplied,
    /// Scheduled but never executed, for example in an interrupted run.
    NotRun,
    /// The execution backend itself failed.
    BackendError,
}

/// A position in the target file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Location {
    /// Line number, 1-indexed.
    pub line: u32,
    /// Character offset within the line, 0-indexed, matching the parser the
    /// Python pack uses.
    pub column: u32,
}
