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
