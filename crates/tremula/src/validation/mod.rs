//! Language-neutral manifest validation. Language-level checks (does the
//! replacement parse?) belong to the pack; everything here is bytes, hashes,
//! and paths.

mod defects;

use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fs, io,
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tremula_contracts::manifest::{Manifest, Mutant, Span};

pub use defects::{ValidationError, ValidationWarning};

/// How large a replacement can get before we warn about serialization bulk.
const LARGE_REPLACEMENT_BYTES: usize = 10 * 1024;

/// The byte-order mark a UTF-8 encoder may prepend to a file.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// How many leading lines may carry an encoding declaration.
const COOKIE_LINES: usize = 2;

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

/// One target file as validation read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedTarget {
    /// POSIX path relative to the project root, spelled as the manifest spells
    /// it.
    pub file: String,
    /// The bytes whose hash the manifest was checked against.
    pub bytes: Vec<u8>,
}

/// What validation concluded, and what it read to conclude it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// Non-fatal observations worth telling the reader about.
    pub warnings: Vec<ValidationWarning>,
    /// Every target file, once each, in the order the manifest first mentions
    /// them.
    pub targets: Vec<VerifiedTarget>,
}

/// Validate a manifest against the project's actual files.
///
/// Returns what was observed along with the bytes that were read, or the first
/// defect that makes the manifest unusable. Checks run in a fixed order so that
/// the reported defect is always the most fundamental one: a stale file is
/// reported as stale rather than as a span mismatch it happens to cause.
///
/// Each target is read exactly once, and those bytes are what every check is
/// measured against and what the caller gets back. Handing them over rather than
/// dropping them is deliberate: the run's snapshot has to be the bytes the
/// hashes matched, not a second read of a file that may since have changed.
///
/// # Errors
///
/// Returns [`ValidationError`] for any mutant that does not agree with the
/// bytes on disk, or whose identifier is not its canonical derivation.
pub fn validate_manifest(
    manifest: &Manifest,
    project_root: &Path,
) -> Result<Verified, ValidationError> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut warnings = Vec::new();
    let mut read: HashMap<&str, Vec<u8>> = HashMap::new();
    let mut first_mentioned: Vec<&str> = Vec::new();
    if manifest.mutants.is_empty() {
        return Ok(Verified {
            warnings,
            targets: Vec::new(),
        });
    }
    let root = canonical_root(project_root)?;
    for mutant in &manifest.mutants {
        let relative = project_relative_path(mutant)?;
        if !seen.insert(mutant.id.as_str()) {
            return Err(ValidationError::DuplicateId {
                id: mutant.id.clone(),
            });
        }
        let bytes = match read.entry(mutant.file.as_str()) {
            Entry::Occupied(already) => already.into_mut(),
            Entry::Vacant(first) => {
                let path = project_root.join(relative);
                check_containment(mutant, &root, &path)?;
                let bytes = read_target(mutant, &path)?;
                first_mentioned.push(mutant.file.as_str());
                first.insert(bytes)
            }
        };
        check_encoding(mutant, bytes)?;
        check_hash(mutant, bytes)?;
        check_replacement_line_endings(mutant)?;
        check_span(mutant, bytes)?;
        check_replacement(mutant)?;
        check_id(mutant)?;
        if mutant.replacement.len() > LARGE_REPLACEMENT_BYTES {
            warnings.push(ValidationWarning::LargeReplacement {
                mutant_id: mutant.id.clone(),
                bytes: mutant.replacement.len(),
            });
        }
    }
    let targets = first_mentioned
        .into_iter()
        .filter_map(|file| {
            read.remove(file).map(|bytes| VerifiedTarget {
                file: file.to_owned(),
                bytes,
            })
        })
        .collect();
    Ok(Verified { warnings, targets })
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

/// The project root as the filesystem sees it, which is what every target has
/// to resolve inside of.
fn canonical_root(project_root: &Path) -> Result<PathBuf, ValidationError> {
    fs::canonicalize(project_root).map_err(|err| ValidationError::ProjectRootUnresolvable {
        root: project_root.to_path_buf(),
        reason: err.to_string(),
    })
}

/// Confirm the target really is a file of the project's own.
///
/// A relative path with no `..` in it still reaches out of the project when the
/// filesystem cooperates: the file itself can be a symbolic link, or a directory
/// on the way to it can be. Either would have tremula mutate a file the project
/// does not own — in place, and with a snapshot that does not cover it — so the
/// spelling is checked against where the path actually leads, not only against
/// how it reads.
fn check_containment(mutant: &Mutant, root: &Path, path: &Path) -> Result<(), ValidationError> {
    let link = fs::symlink_metadata(path).map_err(|err| unreadable_target(mutant, &err))?;
    if link.file_type().is_symlink() {
        return Err(ValidationError::SymlinkTarget {
            mutant_id: mutant.id.clone(),
            file: mutant.file.clone(),
        });
    }
    let resolved = fs::canonicalize(path).map_err(|err| unreadable_target(mutant, &err))?;
    if !resolved.starts_with(root) {
        return Err(ValidationError::TargetOutsideProject {
            mutant_id: mutant.id.clone(),
            file: mutant.file.clone(),
        });
    }
    Ok(())
}

/// Read the target, keeping "it is not there" and "it is there but I cannot
/// read it" apart. Reporting the second as the first sends the reader looking
/// for a missing file that exists.
fn read_target(mutant: &Mutant, path: &Path) -> Result<Vec<u8>, ValidationError> {
    fs::read(path).map_err(|err| unreadable_target(mutant, &err))
}

/// Which of the two "cannot use this file" failures an I/O error is.
fn unreadable_target(mutant: &Mutant, err: &io::Error) -> ValidationError {
    if err.kind() == io::ErrorKind::NotFound {
        return ValidationError::FileMissing {
            mutant_id: mutant.id.clone(),
            file: mutant.file.clone(),
        };
    }
    ValidationError::FileUnreadable {
        mutant_id: mutant.id.clone(),
        file: mutant.file.clone(),
        reason: err.to_string(),
    }
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

/// Hold replacements to the same rule as the files they go into: LF only.
///
/// This one cannot be delegated to a language pack, because what a carriage
/// return does depends on the shape of the span it lands in. Measured against
/// the Python pack and its backend: in a replacement for an expression the
/// carriage return disappears, in a replacement for a whole statement it
/// survives into the file as a CRLF line ending — the very thing a CRLF *file*
/// is rejected for — and inside a string literal it can change how many
/// statements the replacement parses as. A pack's own checks cannot see any of
/// that, and none of it is the mutation the manifest described, so carriage
/// returns are refused outright rather than reshaped.
fn check_replacement_line_endings(mutant: &Mutant) -> Result<(), ValidationError> {
    if mutant.replacement.contains('\r') {
        return Err(ValidationError::CarriageReturnInReplacement {
            mutant_id: mutant.id.clone(),
        });
    }
    Ok(())
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
