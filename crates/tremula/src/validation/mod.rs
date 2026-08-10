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

pub use defects::{TargetFileError, ValidationError, ValidationWarning};

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
    for mutant in &manifest.mutants {
        if !seen.insert(mutant.id.as_str()) {
            return Err(ValidationError::DuplicateId {
                id: mutant.id.clone(),
            });
        }
        let bytes = match read.entry(mutant.file.as_str()) {
            Entry::Occupied(already) => already.into_mut(),
            Entry::Vacant(first) => {
                let bytes = read_project_file(project_root, &mutant.file)
                    .map_err(|problem| problem.about(mutant))?;
                first_mentioned.push(mutant.file.as_str());
                first.insert(bytes)
            }
        };
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

/// Read one file of the project and hold it to every rule a target file keeps.
///
/// The same rules [`validate_manifest`] applies to a mutant's target, asked of a
/// path with no mutant behind it — which is what a caller that has no manifest yet
/// needs. It is the same code rather than the same intent: a caller that read a
/// file this refuses would go on to build a manifest the neutral validation throws
/// out, and with a generator involved it would pay a model first.
///
/// # Errors
///
/// Returns [`TargetFileError`] for a path that is not the project's own, a file
/// that is not there or cannot be read, or bytes tremula cannot address by offset:
/// anything but plain UTF-8 with line feeds.
pub fn read_target_file(project_root: &Path, file: &str) -> Result<Vec<u8>, TargetFileError> {
    read_project_file(project_root, file).map_err(|problem| problem.on_its_own(file))
}

/// Read one file of the project, or say what is wrong with it in the terms both
/// callers translate from.
fn read_project_file(project_root: &Path, file: &str) -> Result<Vec<u8>, FileProblem> {
    let relative = project_relative_path(file)?;
    let root = fs::canonicalize(project_root).map_err(|err| FileProblem::RootUnresolvable {
        root: project_root.to_path_buf(),
        reason: err.to_string(),
    })?;
    let path = project_root.join(relative);
    check_containment(&root, &path)?;
    let bytes = fs::read(&path).map_err(|err| which_read_failure(&err))?;
    check_encoding(&bytes)?;
    Ok(bytes)
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
fn project_relative_path(file: &str) -> Result<&Path, FileProblem> {
    let path = Path::new(file);
    let all_plain_names = path
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if !is_plain_posix_spelling(file) || !all_plain_names {
        return Err(FileProblem::NotProjectRelative);
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

/// Confirm the target really is a file of the project's own.
///
/// A relative path with no `..` in it still reaches out of the project when the
/// filesystem cooperates: the file itself can be a symbolic link, or a directory
/// on the way to it can be. Either would have tremula mutate a file the project
/// does not own — in place, and with a snapshot that does not cover it — so the
/// spelling is checked against where the path actually leads, not only against
/// how it reads.
fn check_containment(root: &Path, path: &Path) -> Result<(), FileProblem> {
    let link = fs::symlink_metadata(path).map_err(|err| which_read_failure(&err))?;
    if link.file_type().is_symlink() {
        return Err(FileProblem::Symlink);
    }
    let resolved = fs::canonicalize(path).map_err(|err| which_read_failure(&err))?;
    if !resolved.starts_with(root) {
        return Err(FileProblem::OutsideProject);
    }
    Ok(())
}

/// Which of the two "cannot use this file" failures an I/O error is. Reporting
/// the second as the first sends the reader looking for a missing file that
/// exists.
fn which_read_failure(err: &io::Error) -> FileProblem {
    if err.kind() == io::ErrorKind::NotFound {
        return FileProblem::Missing;
    }
    FileProblem::Unreadable(err.to_string())
}

/// Reject anything that is not plain UTF-8 with LF endings. A byte-order mark
/// and a raw non-UTF-8 byte are both fatal here rather than later, where a
/// language pack would meet them as an unexplained decoding failure.
fn check_encoding(bytes: &[u8]) -> Result<(), FileProblem> {
    if bytes.starts_with(&UTF8_BOM) {
        return Err(FileProblem::UnsupportedEncoding);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(FileProblem::NotUtf8);
    };
    if declares_other_encoding(text) {
        return Err(FileProblem::UnsupportedEncoding);
    }
    if bytes.contains(&b'\r') {
        return Err(FileProblem::UnsupportedLineEndings);
    }
    Ok(())
}

/// What can be wrong with a file before any mutant is involved.
///
/// The one vocabulary both readings of these rules translate out of: a manifest's,
/// which names the mutant that carries the path, and a caller's that has only the
/// path. Keeping the checks in one place is what stops the two from drifting into
/// different opinions of the same file.
enum FileProblem {
    /// The path is not a plain project-relative POSIX path.
    NotProjectRelative,
    /// The project root does not resolve. The root travels with the reason because
    /// it is not a fact about the file being read: it is the value the caller
    /// passed, and the only thing they can correct.
    RootUnresolvable {
        /// The root as it was given.
        root: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The path leads to a link rather than a file of the project's own.
    Symlink,
    /// The path resolves to somewhere outside the project.
    OutsideProject,
    /// There is no such file.
    Missing,
    /// There is, and it could not be read.
    Unreadable(String),
    /// Its bytes are not UTF-8.
    NotUtf8,
    /// It carries a byte-order mark or announces another encoding.
    UnsupportedEncoding,
    /// It uses carriage returns.
    UnsupportedLineEndings,
}

impl FileProblem {
    /// This problem as a defect of the mutant whose target the file is.
    fn about(self, mutant: &Mutant) -> ValidationError {
        let id = mutant.id.clone();
        let file = mutant.file.clone();
        match self {
            Self::NotProjectRelative => ValidationError::PathEscapesProject {
                mutant_id: id,
                file,
            },
            Self::RootUnresolvable { root, reason } => {
                ValidationError::ProjectRootUnresolvable { root, reason }
            }
            Self::Symlink => ValidationError::SymlinkTarget {
                mutant_id: id,
                file,
            },
            Self::OutsideProject => ValidationError::TargetOutsideProject {
                mutant_id: id,
                file,
            },
            Self::Missing => ValidationError::FileMissing {
                mutant_id: id,
                file,
            },
            Self::Unreadable(reason) => ValidationError::FileUnreadable {
                mutant_id: id,
                file,
                reason,
            },
            Self::NotUtf8 => ValidationError::NotUtf8 {
                mutant_id: id,
                file,
            },
            Self::UnsupportedEncoding => ValidationError::UnsupportedEncoding {
                mutant_id: id,
                file,
            },
            Self::UnsupportedLineEndings => ValidationError::UnsupportedLineEndings {
                mutant_id: id,
                file,
            },
        }
    }

    /// This problem as a defect of the file itself, with no mutant to name.
    fn on_its_own(self, file: &str) -> TargetFileError {
        let file = file.to_owned();
        match self {
            Self::NotProjectRelative => TargetFileError::NotProjectRelative { file },
            // The root is left out of this one, and only this one: its message is
            // read by a caller who named the root a line ago and has no mutant to
            // be told about, so echoing the path back says nothing new.
            Self::RootUnresolvable { reason, .. } => TargetFileError::RootUnresolvable { reason },
            Self::Symlink => TargetFileError::Symlink { file },
            Self::OutsideProject => TargetFileError::OutsideProject { file },
            Self::Missing => TargetFileError::Missing { file },
            Self::Unreadable(reason) => TargetFileError::Unreadable { file, reason },
            Self::NotUtf8 => TargetFileError::NotUtf8 { file },
            Self::UnsupportedEncoding => TargetFileError::UnsupportedEncoding { file },
            Self::UnsupportedLineEndings => TargetFileError::UnsupportedLineEndings { file },
        }
    }
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
