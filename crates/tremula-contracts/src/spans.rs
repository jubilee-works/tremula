//! Where a mutation may land: one source file's functions, as the language pack
//! reads them.
//!
//! A generator needs two things before it can propose a mutation — which bytes
//! belong to which function, and which bytes inside a function carry no
//! behaviour to change. This document carries both, and nothing else: it states
//! positional facts about a file and never what makes a mutation worth making.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::manifest::Span;

/// What one source file offers a generator, as the pack found it.
///
/// The spans are raw byte offsets into the same bytes `file_sha256` hashes, so a
/// consumer that reads the file itself arrives at the same text the pack saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpansReport {
    /// Contract version of this document.
    pub schema_version: String,
    /// The file these spans describe, POSIX-style and relative to the project
    /// root — the same spelling a manifest uses for the same file.
    pub file: String,
    /// SHA-256 of the file's raw bytes as the pack read them, written as
    /// lowercase hexadecimal. A manifest built from this report carries the same
    /// value as `base_file_sha256`, so a report that has gone stale can be told
    /// from one that still describes the file.
    pub file_sha256: String,
    /// Every function in the file, in the order the file spells them: a function
    /// precedes the ones nested inside it. A file with no functions reports an
    /// empty list.
    pub functions: Vec<FunctionSpan>,
}

/// One function: where it is, where its body is, and what inside that body is
/// not a mutation target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionSpan {
    /// Dotted name for a person to read: `Invoice.total` for a method,
    /// `sync_events.<locals>.merge` for a function nested in another.
    ///
    /// Display only, and never a key: overloaded definitions, a definition made
    /// twice under different conditions, and a property's getter and setter all
    /// report the same name. A consumer that has to identify a function uses
    /// `span`, which is unique within a report.
    pub qualified_name: String,
    /// The whole function, from the `def` or `async def` that opens it to the end
    /// of its last statement.
    ///
    /// Decorators are excluded, because they precede that keyword: the bytes
    /// between a decorator and `def` belong to no span in this report.
    pub span: Span,
    /// The statements of the body: from the start of the first to the end of the
    /// last.
    ///
    /// The signature and any decorator are therefore outside it by construction,
    /// and so are the comments and blank lines between the signature and the
    /// first statement, which are not statements to mutate. What remains inside
    /// it is a function's own code — and the functions nested in it, which is
    /// what the ownership rule in `contracts/pack-protocol.md` subtracts.
    pub body_span: Span,
    /// Stretches of `body_span` a mutation must not aim at. Absent means there
    /// are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<ExcludedSpan>,
}

/// A stretch of a function's body that carries no behaviour a mutation could
/// change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExcludedSpan {
    /// What is standing there.
    pub kind: ExcludedKind,
    /// The bytes to leave alone.
    pub span: Span,
}

/// Why a stretch of a body is not a mutation target.
///
/// Only kinds a generator has been observed to aim at are listed. `Unknown` is
/// the fallback that makes a new kind additive: a consumer built before the kind
/// existed still learns that those bytes are excluded, which is the part of the
/// document it has to obey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum ExcludedKind {
    /// A documentation string. Changing one changes no behaviour, so a mutation
    /// there is one no test suite can be blamed for missing.
    Docstring,
    /// The type in an annotated assignment. Annotations are erased at runtime, so
    /// the same argument holds.
    Annotation,
    /// A kind this consumer does not know, reported by a newer producer.
    #[serde(other)]
    Unknown,
}
