//! The defects and observations a manifest can carry.
//!
//! Every message here names the cause and the next action. They are a public
//! surface — what a reader of a failed run actually reads — and are pinned by
//! tests as one.

use std::path::PathBuf;

/// A manifest defect that stops the run. Every message names the cause and the
/// next action — these strings are a public surface, tested like one.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    /// The target path is absolute or leaves the project root.
    #[error(
        "mutant {mutant_id}: file `{file}` is not a plain relative path inside the project; use a POSIX path relative to the project root"
    )]
    PathEscapesProject {
        /// The mutant that carries the offending path.
        mutant_id: String,
        /// The path as written in the manifest.
        file: String,
    },
    /// The project root itself could not be resolved.
    #[error(
        "cannot resolve the project root `{}`: {reason}; check that the directory exists and is readable",
        root.display()
    )]
    ProjectRootUnresolvable {
        /// The root as given.
        root: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The target is a symbolic link rather than a file of the project's own.
    #[error(
        "mutant {mutant_id}: `{file}` is a symbolic link; point the manifest at the real file inside the project, because mutating a link edits a file the project does not own"
    )]
    SymlinkTarget {
        /// The mutant whose target is a link.
        mutant_id: String,
        /// The path that is a link.
        file: String,
    },
    /// The target resolves to somewhere outside the project.
    #[error(
        "mutant {mutant_id}: `{file}` resolves to a location outside the project root; a mutant may only target a file the project itself contains"
    )]
    TargetOutsideProject {
        /// The mutant whose target is elsewhere.
        mutant_id: String,
        /// The path that leads out of the project.
        file: String,
    },
    /// The target file does not exist.
    #[error("mutant {mutant_id}: target file `{file}` does not exist under the project root")]
    FileMissing {
        /// The mutant whose target is missing.
        mutant_id: String,
        /// The path that was looked up.
        file: String,
    },
    /// The target file is there but could not be read.
    #[error(
        "mutant {mutant_id}: cannot read target file `{file}`: {reason}; check that it is a readable regular file"
    )]
    FileUnreadable {
        /// The mutant whose target could not be read.
        mutant_id: String,
        /// The path that could not be read.
        file: String,
        /// What the operating system reported.
        reason: String,
    },
    /// The file changed since the manifest was generated.
    #[error(
        "mutant {mutant_id}: `{file}` has changed since the manifest was generated (its SHA-256 no longer matches base_file_sha256); regenerate the manifest against the current sources"
    )]
    StaleFile {
        /// The mutant whose target no longer hashes to the recorded value.
        mutant_id: String,
        /// The path that changed.
        file: String,
    },
    /// The file is not valid UTF-8 at all.
    #[error(
        "mutant {mutant_id}: `{file}` is not valid UTF-8; only plain UTF-8 sources are supported"
    )]
    NotUtf8 {
        /// The mutant whose target cannot be decoded.
        mutant_id: String,
        /// The path that cannot be decoded.
        file: String,
    },
    /// The file carries a BOM or declares a non-UTF-8 coding cookie.
    #[error(
        "mutant {mutant_id}: `{file}` carries a byte-order mark or declares a non-UTF-8 coding cookie; only plain UTF-8 sources are supported"
    )]
    UnsupportedEncoding {
        /// The mutant whose target announces another encoding.
        mutant_id: String,
        /// The path that announces another encoding.
        file: String,
    },
    /// The file uses CRLF line endings.
    #[error(
        "mutant {mutant_id}: `{file}` uses CRLF line endings; convert the file to LF before generating mutants for it"
    )]
    UnsupportedLineEndings {
        /// The mutant whose target uses CRLF.
        mutant_id: String,
        /// The path that uses CRLF.
        file: String,
    },
    /// The span does not fit inside the file.
    #[error("mutant {mutant_id}: span {start}..{end} does not fit inside `{file}` ({len} bytes)")]
    SpanOutOfBounds {
        /// The mutant with the out-of-range span.
        mutant_id: String,
        /// Start offset as written in the manifest.
        start: u64,
        /// End offset as written in the manifest.
        end: u64,
        /// The file the span was measured against.
        file: String,
        /// Actual size of that file in bytes.
        len: u64,
    },
    /// The span is empty or inverted.
    #[error(
        "mutant {mutant_id}: span is empty (start_byte {start} is not below end_byte {end}); insertions are not supported"
    )]
    EmptySpan {
        /// The mutant with the empty span.
        mutant_id: String,
        /// Start offset as written in the manifest.
        start: u64,
        /// End offset as written in the manifest.
        end: u64,
    },
    /// The replacement carries a carriage return.
    #[error(
        "mutant {mutant_id}: replacement contains a carriage return; use LF-only line endings in replacements"
    )]
    CarriageReturnInReplacement {
        /// The mutant whose replacement is not LF-only.
        mutant_id: String,
    },
    /// The bytes at the span differ from `original`.
    #[error(
        "mutant {mutant_id}: the bytes at the span do not match `original`; regenerate the manifest against the current sources"
    )]
    OriginalMismatch {
        /// The mutant whose recorded original text is stale.
        mutant_id: String,
    },
    /// The replacement changes nothing.
    #[error(
        "mutant {mutant_id}: replacement is identical to the original text; a mutant must change the code"
    )]
    IdenticalReplacement {
        /// The mutant that would be a no-op.
        mutant_id: String,
    },
    /// The id is not the canonical derivation of the mutant's fields.
    #[error(
        "mutant {mutant_id}: id does not match the canonical derivation (expected {expected}); recompute it as documented in the manifest schema"
    )]
    IdMismatch {
        /// The identifier as written in the manifest.
        mutant_id: String,
        /// The identifier the mutant's own fields derive.
        expected: String,
    },
    /// Two mutants share one id.
    #[error("mutant id {id} appears more than once; every mutant in a manifest must be unique")]
    DuplicateId {
        /// The repeated identifier.
        id: String,
    },
}

/// Why one file of the project cannot be a mutation target, said without a mutant
/// to name.
///
/// The same rules [`ValidationError`] reports about a manifest's target, for a
/// caller that has a path and no manifest yet. Each message says the same thing
/// its counterpart does, because a caller who fixes the file has to satisfy the
/// same validation afterwards.
#[derive(Debug, thiserror::Error)]
pub enum TargetFileError {
    /// The path is absolute or leaves the project root.
    #[error(
        "`{file}` is not a plain relative path inside the project; use a POSIX path relative to the project root"
    )]
    NotProjectRelative {
        /// The path as given.
        file: String,
    },
    /// The project root itself could not be resolved.
    #[error(
        "cannot resolve the project root: {reason}; check that the directory exists and is readable"
    )]
    RootUnresolvable {
        /// What the operating system reported.
        reason: String,
    },
    /// The file is a symbolic link rather than a file of the project's own.
    #[error(
        "`{file}` is a symbolic link; name the real file inside the project, because mutating a link edits a file the project does not own"
    )]
    Symlink {
        /// The path that is a link.
        file: String,
    },
    /// The path resolves to somewhere outside the project.
    #[error(
        "`{file}` resolves to a location outside the project root; only a file the project itself contains can be mutated"
    )]
    OutsideProject {
        /// The path that leads out of the project.
        file: String,
    },
    /// The file does not exist.
    #[error("`{file}` does not exist under the project root")]
    Missing {
        /// The path that was looked up.
        file: String,
    },
    /// The file is there but could not be read.
    #[error("cannot read `{file}`: {reason}; check that it is a readable regular file")]
    Unreadable {
        /// The path that could not be read.
        file: String,
        /// What the operating system reported.
        reason: String,
    },
    /// The file is not valid UTF-8 at all.
    #[error("`{file}` is not valid UTF-8; only plain UTF-8 sources are supported")]
    NotUtf8 {
        /// The path that cannot be decoded.
        file: String,
    },
    /// The file carries a BOM or declares a non-UTF-8 coding cookie.
    #[error(
        "`{file}` carries a byte-order mark or declares a non-UTF-8 coding cookie; only plain UTF-8 sources are supported"
    )]
    UnsupportedEncoding {
        /// The path that announces another encoding.
        file: String,
    },
    /// The file uses CRLF line endings.
    #[error(
        "`{file}` uses CRLF line endings; convert the file to LF before generating mutants for it"
    )]
    UnsupportedLineEndings {
        /// The path that uses CRLF.
        file: String,
    },
}

/// A non-fatal observation about the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationWarning {
    /// A replacement above `LARGE_REPLACEMENT_BYTES` — legal, but it is
    /// serialized twice on its way to the backend.
    LargeReplacement {
        /// The mutant with the bulky replacement.
        mutant_id: String,
        /// Size of that replacement in bytes.
        bytes: usize,
    },
}
