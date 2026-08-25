//! A run's own directory, and the two claims that go with it: the name no other
//! run shares, and the lock no other run may hold.
//!
//! Run directories cannot collide, but the source tree can: mutants are applied
//! in place, so two runs of one project would corrupt each other's sources. The
//! project lock is what prevents that, and it records the process holding it so
//! that a run killed outright does not lock the project for good.
//!
//! The snapshot is the other half of the same problem. A run that is killed
//! between applying a mutant and putting it back leaves the mutant in the
//! sources; the snapshot is what `restore` puts back, and it holds the bytes the
//! manifest was checked against rather than a later reading of a file that may
//! have been mutated by then.

mod lock;

use std::{
    fs, io,
    path::{Path, PathBuf},
    process,
};

use sha2::{Digest, Sha256};
use time::{OffsetDateTime, UtcOffset, macros::format_description};

pub use lock::{Lock, LockError};

use crate::validation::VerifiedTarget;

/// Where a project keeps everything tremula puts there.
pub const TREMULA_DIR: &str = ".tremula";

/// Name of the symlink that points at the newest run.
const LATEST: &str = "latest";

/// Where a run keeps the sources as they were before it started.
const SNAPSHOT_DIR: &str = "snapshot";

/// How much of the manifest's hash goes into a run's name.
const SHA_CHARS: usize = 6;

/// The last suffix a run will accept when its own name is taken. Nine names for
/// one second of one manifest is past the point where the tenth run is a mistake
/// rather than a coincidence.
const LAST_SUFFIX: u32 = 9;

/// A run's identifier: when it started, and which manifest it ran.
///
/// It exists before the run's directory does, because the lock has to record
/// which run holds it and the lock is taken before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunId(String);

impl RunId {
    /// Name a run started at `now` from a manifest hashing to `manifest_sha`.
    #[must_use]
    pub fn new(now: OffsetDateTime, manifest_sha: &str) -> Self {
        let utc = now.to_offset(UtcOffset::UTC);
        // The description below cannot fail to render an `OffsetDateTime`, since
        // every component it names is one such a value always has. The fallback
        // keeps that impossibility from being a panic all the same, and stays
        // recognisable as a moment in time.
        let stamp = utc
            .format(format_description!(
                "[year][month][day]T[hour][minute][second]Z"
            ))
            .unwrap_or_else(|_| format!("{}Z", utc.unix_timestamp()));
        let short: String = manifest_sha.chars().take(SHA_CHARS).collect();
        Self(format!("{stamp}-{short}"))
    }

    /// Name the work `restore` does while it holds the project.
    ///
    /// Restoring is not a run and produces no directory of its own, but it writes
    /// the same files a run mutates and so takes the same lock — and a lock has to
    /// say who holds it.
    #[must_use]
    pub fn for_restore() -> Self {
        Self(format!("restore-{}", process::id()))
    }

    /// The identifier as it is written in paths and documents.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The manifest fingerprint a run's name carries, when the name is one this tool derived.
///
/// This is the only link there is between a run and a manifest that is not inside the run's
/// own directory: a manifest holding the same mutants under the same identifiers names no run
/// of its own, so nothing but the name says whether the two belong together. A reader that
/// paired the wrong two would quote spans, originals and replacements that the verdicts were
/// never reached about.
///
/// A name that carries no fingerprint — a run copied to a name of somebody's own — answers
/// nothing rather than answering wrongly, and a caller with nothing to compare against has to
/// decide for itself what that means.
#[must_use]
pub fn fingerprint(run_id: &str) -> Option<&str> {
    let mut segments = run_id.split('-');
    segments.next()?;
    let fingerprint = segments.next()?;
    (fingerprint.len() == SHA_CHARS && hexadecimal(fingerprint)).then_some(fingerprint)
}

/// The fingerprint a run of these manifest bytes would have been named after.
#[must_use]
pub fn fingerprint_of(manifest: &[u8]) -> String {
    format!("{:x}", Sha256::digest(manifest))
        .chars()
        .take(SHA_CHARS)
        .collect()
}

/// Whether every character is one a lowercase hexadecimal digest is spelled with.
fn hexadecimal(said: &str) -> bool {
    said.bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A directory reserved for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDir {
    path: PathBuf,
}

impl RunDir {
    /// Where the run writes everything it produces.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The run's identifier, which is this directory's name.
    ///
    /// The directory is the identifier's home: the pack reads it back off the
    /// path it is handed rather than being told twice.
    #[must_use]
    pub fn run_id(&self) -> &str {
        self.path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
    }
}

/// Reserve a directory named `id` under `out_parent`.
///
/// The directory is created, not merely named: a run that finds its own name
/// taken has to get a different one, because the pack is willing to write into a
/// directory that already exists and two runs sharing one would interleave their
/// documents. Runs of the same manifest within one second are the case that makes
/// this real, and each after the first gets a suffixed name.
///
/// # Errors
///
/// Returns the underlying I/O error when a name cannot be created, or one saying
/// they are all taken when every suffix is.
pub fn create(out_parent: &Path, id: &RunId) -> io::Result<RunDir> {
    fs::create_dir_all(out_parent)?;
    for suffix in 1..=LAST_SUFFIX {
        let name = match suffix {
            1 => id.as_str().to_owned(),
            taken => format!("{}-{taken}", id.as_str()),
        };
        let path = out_parent.join(name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(RunDir { path }),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "`{}` and every name up to `{}-{LAST_SUFFIX}` already exist",
            id.as_str(),
            id.as_str()
        ),
    ))
}

/// Point `latest` at `run_dir`, replacing whatever it pointed at before.
///
/// The link is written under another name and renamed over the old one, so a
/// reader never finds `latest` missing. Its target is the run's name alone, so
/// that moving or copying the whole tree keeps the link working.
///
/// # Errors
///
/// Returns the underlying I/O error when the link cannot be written or moved
/// into place.
pub fn publish_latest(out_parent: &Path, run_dir: &Path) -> io::Result<()> {
    let Some(name) = run_dir.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a run directory has to have a name",
        ));
    };
    let staging = out_parent.join(".latest.new");
    if let Err(err) = fs::remove_file(&staging)
        && err.kind() != io::ErrorKind::NotFound
    {
        return Err(err);
    }
    std::os::unix::fs::symlink(name, &staging)?;
    fs::rename(&staging, out_parent.join(LATEST))
}

/// Keep every target file as it was, under `snapshot/` inside the run.
///
/// The bytes come from validation rather than from the files: they are the bytes
/// the manifest's hashes were checked against, which is what makes the snapshot a
/// faithful "before" even if something changes a file a moment later.
///
/// # Errors
///
/// Returns the underlying I/O error when the snapshot cannot be written.
pub fn snapshot(run_dir: &Path, targets: &[VerifiedTarget]) -> io::Result<()> {
    let root = run_dir.join(SNAPSHOT_DIR);
    fs::create_dir_all(&root)?;
    for target in targets {
        let path = root.join(&target.file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &target.bytes)?;
    }
    Ok(())
}

/// Why a run's sources could not be put back.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// There is no such run.
    #[error(
        "no run directory at `{}`; pass --run with the run to restore from, or check `{TREMULA_DIR}/runs`",
        path.display()
    )]
    NoRunDirectory {
        /// The path that was looked for.
        path: PathBuf,
    },
    /// The run kept no snapshot, so there is nothing to put back.
    #[error(
        "the run in `{}` kept no snapshot of the project's files, so there is nothing to restore; a run that stopped before its sources were read keeps none",
        run_dir.display()
    )]
    NoSnapshot {
        /// The run with no snapshot.
        run_dir: PathBuf,
    },
    /// A run is working on the project right now.
    #[error(
        "another tremula run ({run_id}, process {pid}) is in progress on this project; wait for it to finish before restoring, or its sources would be replaced while it is measuring them"
    )]
    InProgress {
        /// The run that holds the project.
        run_id: String,
        /// The process that holds it.
        pid: u32,
    },
    /// The project could not be claimed for the restore.
    #[error(transparent)]
    Unclaimable(#[from] LockError),
    /// A file could not be copied back.
    #[error(
        "cannot restore `{}`: {reason}; check that the project directory is writable",
        path.display()
    )]
    Unusable {
        /// The path that could not be written or read.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
}

/// Put every file the run snapshotted back into the project, and clear the lock
/// the run left behind.
///
/// Returns the project-relative paths that were restored.
///
/// # Errors
///
/// Returns [`RestoreError`] when there is no such run, when it kept no snapshot,
/// when a run is still in progress, or when a file cannot be written.
pub fn restore(run_dir: &Path, project_root: &Path) -> Result<Vec<String>, RestoreError> {
    if !run_dir.is_dir() {
        return Err(RestoreError::NoRunDirectory {
            path: run_dir.to_path_buf(),
        });
    }
    let snapshot = run_dir.join(SNAPSHOT_DIR);
    if !snapshot.is_dir() {
        return Err(RestoreError::NoSnapshot {
            run_dir: run_dir.to_path_buf(),
        });
    }
    // Restoring writes the very files a run mutates, so it takes the same lock —
    // and holds it for the whole copy rather than only long enough to look. A run
    // that started half way through would have the mutant it was measuring
    // overwritten, and its own lock deleted under it.
    let claimed = Lock::acquire(project_root, &RunId::for_restore()).map_err(refusal)?;
    let mut restored = Vec::new();
    copy_back(&snapshot, project_root, Path::new(""), &mut restored)?;
    restored.sort();
    // Releasing it is also what clears the lock the crashed run left, since this
    // one took that one over. That is the other half of recovering from a crash.
    drop(claimed);
    Ok(restored)
}

/// Why restoring could not claim the project, in the words of the reader who
/// asked for it.
fn refusal(failure: LockError) -> RestoreError {
    match failure {
        LockError::Held { run_id, pid } => RestoreError::InProgress { run_id, pid },
        other => RestoreError::Unclaimable(other),
    }
}

/// Take away a link standing where a file is about to be written.
///
/// Writing to a link writes through it, so restoring a target that has since been
/// replaced by a link would truncate whatever the link points at — a file
/// somewhere else in the project, or outside it altogether. The snapshot says a
/// file belongs at this path, and putting one there means replacing the link
/// rather than following it.
fn unlink_any_link(destination: &Path) -> Result<(), RestoreError> {
    let Ok(existing) = fs::symlink_metadata(destination) else {
        return Ok(());
    };
    if !existing.file_type().is_symlink() {
        return Ok(());
    }
    fs::remove_file(destination).map_err(|err| RestoreError::Unusable {
        path: destination.to_path_buf(),
        reason: err.to_string(),
    })
}

/// Copy one snapshot directory back into the project, recording what was
/// written.
fn copy_back(
    from: &Path,
    into: &Path,
    relative: &Path,
    restored: &mut Vec<String>,
) -> Result<(), RestoreError> {
    let unusable = |path: &Path, err: &io::Error| RestoreError::Unusable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    };
    for entry in fs::read_dir(from).map_err(|err| unusable(from, &err))? {
        let entry = entry.map_err(|err| unusable(from, &err))?;
        let here = relative.join(entry.file_name());
        let destination = into.join(&here);
        let kind = entry
            .file_type()
            .map_err(|err| unusable(&entry.path(), &err))?;
        if kind.is_dir() {
            fs::create_dir_all(&destination).map_err(|err| unusable(&destination, &err))?;
            copy_back(&entry.path(), into, &here, restored)?;
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|err| unusable(parent, &err))?;
            }
            unlink_any_link(&destination)?;
            fs::copy(entry.path(), &destination).map_err(|err| unusable(&destination, &err))?;
            restored.push(here.to_string_lossy().into_owned());
        }
    }
    Ok(())
}
