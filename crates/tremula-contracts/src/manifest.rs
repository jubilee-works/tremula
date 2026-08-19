//! The mutant manifest: what to mutate, decided outside the execution backend.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A set of mutants to apply to one project revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    /// Contract version of this document.
    pub schema_version: String,
    /// Language pack that can execute these mutants.
    pub language: Language,
    /// The revision the spans were computed against.
    pub base: Base,
    /// Mutants to apply. An empty list is valid and means "nothing to test".
    pub mutants: Vec<Mutant>,
    /// Why these targets were chosen, when the generator chose them for itself.
    ///
    /// Absent when a person named the functions. Nothing that reads a manifest to
    /// run it looks at this: it is the record of a decision, kept beside the
    /// decision's result so that whoever receives the evidence receives the reason
    /// with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
}

/// Languages with a tremula pack. Adding a variant is a breaking change for
/// older consumers, which is intended: an unknown language has no pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// Python projects, executed through the Cosmic Ray backend.
    Python,
}

/// The revision mutants were derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Base {
    /// Source revision the generator used, or absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// One mutation: replace the bytes of `span` in `file` with `replacement`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Mutant {
    /// Content-derived identifier, written as lowercase hexadecimal.
    ///
    /// It is the SHA-256 of six UTF-8 encoded parts joined by single NUL
    /// bytes, in this order: the literal tag `tremula/mutant/v1`, `file`,
    /// `span.start_byte`, `span.end_byte`, `base_file_sha256`, and
    /// `replacement`. Both offsets are rendered as unpadded, unsigned decimal
    /// ASCII digits, so byte 412 contributes the three bytes `412`.
    ///
    /// Because the parts are joined with NUL bytes, no part may contain one: a
    /// producer that would emit a NUL inside `file` or `replacement` must
    /// reject the mutant rather than derive an ambiguous identifier.
    ///
    /// `provenance` is excluded, so provenance can change without changing
    /// identity. Any producer that follows this derivation arrives at the same
    /// identifier for the same mutation.
    pub id: String,
    /// Target file, POSIX-style and relative to the project root.
    pub file: String,
    /// SHA-256 of the target file's raw bytes, used to reject stale manifests.
    pub base_file_sha256: String,
    /// Byte range to replace.
    pub span: Span,
    /// Exact source text currently occupying the span, UTF-8 decoded.
    pub original: String,
    /// Replacement source text.
    pub replacement: String,
    /// What bug this mutant simulates, for humans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Free-form generator provenance, excluded from the identifier.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub provenance: serde_json::Map<String, serde_json::Value>,
}

/// What a generator looked at, what it chose out of it, and what it left behind.
///
/// Every list here is written even when it is empty, and that is the point of the
/// document: a selection that reported only what it chose would be indistinguishable
/// from one that quietly dropped half of what it found. A reader has to be able to
/// account for every changed line — chosen, capped, uncovered, or outside any
/// function.
///
/// # What this does to a run's identity
///
/// A run derives its identifier from the bytes of the manifest it was given, and
/// these bytes are part of that manifest. So the same mutants selected against a
/// different base, or with different coverage, produce a different run identifier.
/// That is the intended behaviour and not a defect: two runs whose reasons differ
/// are two runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Selection {
    /// The revision the changes were measured against, as the caller spelled it.
    pub diff_base: String,
    /// The commit the comparison actually ran from — the merge base of `diff_base`
    /// and the working revision — or absent when it could not be observed.
    ///
    /// Recorded because it, and not `diff_base`, is what the line numbers mean: a
    /// branch that has fallen behind its base would otherwise report the base's own
    /// drift as this change's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_base: Option<String>,
    /// The coverage document that was read, as the caller spelled it. Absent when
    /// none was given, which is the degraded selection: a mutant that survives may
    /// have survived because no test ever reached it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<String>,
    /// The functions that were selected, in the order they were asked about.
    pub functions: Vec<SelectedFunction>,
    /// The functions that were selected and then cut by the limit on how many one
    /// generation asks about. Nothing was asked about these, so their `generation`
    /// counts are zero and their `inferred_tests` are empty.
    pub skipped_over_limit: Vec<SelectedFunction>,
    /// Changed lines no test reached, per file. Reported rather than discarded: a
    /// changed line nothing covers is the finding, even though it is not a target.
    pub coverage_gaps: Vec<CoverageGap>,
    /// Changed files the coverage document says nothing at all about, POSIX-style
    /// and relative to the project root.
    pub files_not_in_coverage: Vec<String>,
    /// How many changed lines fell inside no function — a class body, a module-level
    /// assignment, an import. Counted rather than listed, because what a reader does
    /// about them is the same whichever line it was.
    pub lines_outside_functions: u32,
}

/// One function a generation chose to ask about, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SelectedFunction {
    /// The file it is in, POSIX-style and relative to the project root.
    pub file: String,
    /// The function, as the language pack names it.
    pub function: String,
    /// Where the function is. This is the authority on which function was chosen:
    /// a file can spell one name more than once, and the byte range is what tells
    /// those apart.
    pub span: Span,
    /// The same extent as lines, for a person to read. Derived from `span` against
    /// the bytes of the file, and carried so that a reader does not have to count
    /// newlines to find what was selected. Where the two could disagree — a file
    /// edited since — `span` is what a consumer obeys.
    pub lines: LineRange,
    /// How many of this function's lines the change touched. What the limit sorts on.
    pub candidate_lines: u32,
    /// The test file found for this function by convention, or an empty list when
    /// none was. At most one: an exact `test_<stem>.py` beside the package the
    /// target file belongs to, and no wider search than that.
    pub inferred_tests: Vec<String>,
    /// What asking about this function came to.
    pub generation: Generation,
}

/// What one function's generation produced.
///
/// Both numbers, because their difference is the fact worth reading: a function
/// with proposals and no records is a function every proposal about which was
/// refused, and a run reporting no survivors for it is reporting nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Generation {
    /// How many mutations a model proposed for it.
    pub proposed: u32,
    /// How many of those the manifest carries.
    pub recorded: u32,
}

/// The changed lines of one file that no test reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoverageGap {
    /// The file, POSIX-style and relative to the project root.
    pub file: String,
    /// The uncovered stretches, in ascending order and merged where they touch.
    pub ranges: Vec<LineRange>,
}

/// A stretch of lines, 1-indexed and inclusive at both ends.
///
/// A single line is a range whose ends are equal. Named members rather than a pair
/// of numbers, so that a reader of the document never has to guess which end is
/// which or whether the last line is included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LineRange {
    /// The first line of the stretch.
    pub start_line: u32,
    /// The last line of the stretch, included in it.
    pub end_line: u32,
}

/// A half-open byte range over a file's raw bytes: `[start_byte, end_byte)`.
/// That `start_byte` is below `end_byte` is a validation rule, not a schema
/// constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Span {
    /// Inclusive start offset, 0-indexed.
    pub start_byte: u64,
    /// Exclusive end offset.
    pub end_byte: u64,
}
