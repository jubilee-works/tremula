//! Language-neutral manifest validation. Language-level checks (does the
//! replacement parse?) belong to the pack; everything here is bytes, hashes,
//! and paths.

use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path},
};

use sha2::{Digest, Sha256};
use tremula_contracts::manifest::{Manifest, Mutant, Span};

/// How large a replacement can get before we warn about serialization bulk.
const LARGE_REPLACEMENT_BYTES: usize = 10 * 1024;

/// The byte-order mark a UTF-8 encoder may prepend to a file.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// How many leading lines may carry an encoding declaration.
const COOKIE_LINES: usize = 2;

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

/// A non-fatal observation about the manifest.
#[derive(Debug, PartialEq, Eq)]
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

/// Compute the canonical content-derived identifier for a mutation, exactly as
/// the manifest contract's `Mutant.id` documentation specifies.
#[must_use]
pub fn canonical_mutant_id(
    file: &str,
    span: &Span,
    base_file_sha256: &str,
    replacement: &str,
) -> String {
    let mut hasher = Sha256::new();
    for (index, part) in [
        "tremula/mutant/v1",
        file,
        &span.start_byte.to_string(),
        &span.end_byte.to_string(),
        base_file_sha256,
        replacement,
    ]
    .iter()
    .enumerate()
    {
        if index > 0 {
            hasher.update([0u8]);
        }
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Validate a manifest against the project's actual files.
///
/// Returns the non-fatal observations worth telling the user about, or the
/// first defect that makes the manifest unusable. Checks run in a fixed order
/// so that the reported defect is always the most fundamental one: a stale file
/// is reported as stale rather than as a span mismatch it happens to cause.
///
/// # Errors
///
/// Returns [`ValidationError`] for any mutant that does not agree with the
/// bytes on disk, or whose identifier is not its canonical derivation.
pub fn validate_manifest(
    manifest: &Manifest,
    project_root: &Path,
) -> Result<Vec<ValidationWarning>, ValidationError> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut warnings = Vec::new();
    for mutant in &manifest.mutants {
        let relative = project_relative_path(mutant)?;
        if !seen.insert(mutant.id.as_str()) {
            return Err(ValidationError::DuplicateId {
                id: mutant.id.clone(),
            });
        }
        let bytes = read_target(mutant, &project_root.join(relative))?;
        check_encoding(mutant, &bytes)?;
        check_hash(mutant, &bytes)?;
        check_span(mutant, &bytes)?;
        check_replacement(mutant)?;
        check_id(mutant)?;
        if mutant.replacement.len() > LARGE_REPLACEMENT_BYTES {
            warnings.push(ValidationWarning::LargeReplacement {
                mutant_id: mutant.id.clone(),
                bytes: mutant.replacement.len(),
            });
        }
    }
    Ok(warnings)
}

/// Accept only paths built entirely from ordinary names. This is stricter than
/// rejecting paths that escape after normalization: `./x.py` and `a/./b.py` are
/// refused too, because the identifier derivation hashes the `file` string
/// verbatim and two spellings of one file must not yield two identifiers.
///
/// The spelling is checked on the raw string before the path is taken apart,
/// because a host's own path syntax is not the contract's. On a POSIX host
/// `..\escape.py` and `C:\pkg\mod.py` are each a single ordinary name, and
/// `a//b.py` quietly collapses to `a/b.py` — all three would slip past a
/// component walk while denoting something other than what they say.
fn project_relative_path(mutant: &Mutant) -> Result<&Path, ValidationError> {
    let path = Path::new(&mutant.file);
    let spelled_as_posix = is_plain_posix_spelling(&mutant.file);
    let all_plain_names = path
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if !spelled_as_posix || !all_plain_names {
        return Err(ValidationError::PathEscapesProject {
            mutant_id: mutant.id.clone(),
            file: mutant.file.clone(),
        });
    }
    Ok(path)
}

/// Whether the string is a non-empty relative POSIX path with no backslash and
/// no empty segment.
fn is_plain_posix_spelling(file: &str) -> bool {
    !file.is_empty()
        && !file.contains('\\')
        && !file.starts_with('/')
        && !file.ends_with('/')
        && !file.contains("//")
}

/// Read the target, keeping "it is not there" and "it is there but I cannot
/// read it" apart. Reporting the second as the first sends the reader looking
/// for a missing file that exists.
fn read_target(mutant: &Mutant, path: &Path) -> Result<Vec<u8>, ValidationError> {
    fs::read(path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            ValidationError::FileMissing {
                mutant_id: mutant.id.clone(),
                file: mutant.file.clone(),
            }
        } else {
            ValidationError::FileUnreadable {
                mutant_id: mutant.id.clone(),
                file: mutant.file.clone(),
                reason: err.to_string(),
            }
        }
    })
}

/// Reject anything that is not plain UTF-8 with LF endings. A byte-order mark
/// and a raw non-UTF-8 byte are both fatal here rather than later, where a
/// language pack would meet them as an unexplained decoding failure.
fn check_encoding(mutant: &Mutant, bytes: &[u8]) -> Result<(), ValidationError> {
    let unsupported = || ValidationError::UnsupportedEncoding {
        mutant_id: mutant.id.clone(),
        file: mutant.file.clone(),
    };
    if bytes.starts_with(&UTF8_BOM) {
        return Err(unsupported());
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(ValidationError::NotUtf8 {
            mutant_id: mutant.id.clone(),
            file: mutant.file.clone(),
        });
    };
    if declares_other_encoding(text) {
        return Err(unsupported());
    }
    if bytes.contains(&b'\r') {
        return Err(ValidationError::UnsupportedLineEndings {
            mutant_id: mutant.id.clone(),
            file: mutant.file.clone(),
        });
    }
    Ok(())
}

/// Whether one of the leading comment lines declares an encoding that is not
/// UTF-8. Only comments count, so a `coding:` inside a string literal further
/// down the file cannot trip this.
fn declares_other_encoding(text: &str) -> bool {
    text.lines()
        .take(COOKIE_LINES)
        .filter(|line| line.trim_start().starts_with('#'))
        .filter_map(declared_encoding)
        .any(|name| name != "utf8")
}

/// Extract a declared encoding name, folded so that `UTF-8`, `utf_8`, and
/// `utf8` compare equal.
fn declared_encoding(line: &str) -> Option<String> {
    let after_keyword = line.find("coding").map(|at| &line[at + "coding".len()..])?;
    let value = after_keyword
        .strip_prefix(':')
        .or_else(|| after_keyword.strip_prefix('='))?;
    let name: String = value
        .trim_start()
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .collect();
    if name.is_empty() {
        return None;
    }
    Some(name.to_ascii_lowercase().replace(['-', '_'], ""))
}

fn check_hash(mutant: &Mutant, bytes: &[u8]) -> Result<(), ValidationError> {
    if format!("{:x}", Sha256::digest(bytes)) == mutant.base_file_sha256 {
        return Ok(());
    }
    Err(ValidationError::StaleFile {
        mutant_id: mutant.id.clone(),
        file: mutant.file.clone(),
    })
}

fn check_span(mutant: &Mutant, bytes: &[u8]) -> Result<(), ValidationError> {
    let start = usize::try_from(mutant.span.start_byte).unwrap_or(usize::MAX);
    let end = usize::try_from(mutant.span.end_byte).unwrap_or(usize::MAX);
    if start > bytes.len() || end > bytes.len() {
        return Err(ValidationError::SpanOutOfBounds {
            mutant_id: mutant.id.clone(),
            start: mutant.span.start_byte,
            end: mutant.span.end_byte,
            file: mutant.file.clone(),
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        });
    }
    if start >= end {
        return Err(ValidationError::EmptySpan {
            mutant_id: mutant.id.clone(),
            start: mutant.span.start_byte,
            end: mutant.span.end_byte,
        });
    }
    if bytes.get(start..end) != Some(mutant.original.as_bytes()) {
        return Err(ValidationError::OriginalMismatch {
            mutant_id: mutant.id.clone(),
        });
    }
    Ok(())
}

fn check_replacement(mutant: &Mutant) -> Result<(), ValidationError> {
    if mutant.replacement == mutant.original {
        return Err(ValidationError::IdenticalReplacement {
            mutant_id: mutant.id.clone(),
        });
    }
    Ok(())
}

fn check_id(mutant: &Mutant) -> Result<(), ValidationError> {
    let expected = canonical_mutant_id(
        &mutant.file,
        &mutant.span,
        &mutant.base_file_sha256,
        &mutant.replacement,
    );
    if expected == mutant.id {
        return Ok(());
    }
    Err(ValidationError::IdMismatch {
        mutant_id: mutant.id.clone(),
        expected,
    })
}
