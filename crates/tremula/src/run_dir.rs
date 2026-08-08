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

use std::{
    fs, io,
    path::{Path, PathBuf},
    process,
};

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset, macros::format_description};

use crate::validation::VerifiedTarget;

/// Where a project keeps everything tremula puts there.
pub const TREMULA_DIR: &str = ".tremula";

/// Name of the lock inside [`TREMULA_DIR`].
const LOCK_FILE: &str = "lock";

/// Name of the symlink that points at the newest run.
const LATEST: &str = "latest";

/// Where a run keeps the sources as they were before it started.
const SNAPSHOT_DIR: &str = "snapshot";

/// How much of the manifest's hash goes into a run's name.
const SHA_CHARS: usize = 6;

/// Suffix for the one retry when a run's first choice of name is taken.
const SECOND_TRY: &str = "-2";

/// What a lock left by a run whose identity could not be read is called. A
/// corrupt lock is still a lock somebody left, and the sentence it appears in
/// reads as an account of what happened rather than as a name.
const UNKNOWN_HOLDER: &str = "an earlier run";

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

    /// The identifier as it is written in paths and documents.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
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
/// documents. Two runs of the same manifest within one second are the case that
/// makes this real, and the second of them gets a suffixed name.
///
/// # Errors
///
/// Returns the underlying I/O error when neither name can be created.
pub fn create(out_parent: &Path, id: &RunId) -> io::Result<RunDir> {
    fs::create_dir_all(out_parent)?;
    let first = out_parent.join(id.as_str());
    match fs::create_dir(&first) {
        Ok(()) => return Ok(RunDir { path: first }),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    let second = out_parent.join(format!("{}{SECOND_TRY}", id.as_str()));
    fs::create_dir(&second)?;
    Ok(RunDir { path: second })
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

/// Why the project could not be claimed for a run.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Another run is working on this project right now.
    #[error(
        "another tremula run ({run_id}, process {pid}) is in progress on this project; wait for it to finish, or run `tremula restore` if it is gone and left the sources mutated"
    )]
    Held {
        /// The run that holds the project.
        run_id: String,
        /// The process that holds it.
        pid: u32,
    },
    /// Two runs claimed the project at the same moment.
    #[error(
        "another tremula run claimed this project a moment ago; wait for it to finish and try again"
    )]
    Contended,
    /// The lock itself could not be written.
    #[error(
        "cannot claim this project for a run: {reason} (`{}`); check that the project directory is writable",
        path.display()
    )]
    Unusable {
        /// The path that could not be used.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
}

/// Which run holds a project, and in which process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Holder {
    run_id: String,
    pid: u32,
}

/// One project, claimed for one run, released when this value is dropped.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    holder: Holder,
    reclaimed: Option<String>,
}

impl Lock {
    /// Claim `project_root` for the run named `run_id`.
    ///
    /// A lock whose process is gone is taken over rather than obeyed, because a
    /// run that was killed cannot release anything and would otherwise lock the
    /// project permanently. A lock whose process cannot be signalled counts as
    /// held: that is what a process belonging to another user looks like, and
    /// treating it as gone would let two runs patch one source tree.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] when another run holds the project, or when the
    /// lock cannot be written.
    pub fn acquire(project_root: &Path, run_id: &RunId) -> Result<Self, LockError> {
        let directory = project_root.join(TREMULA_DIR);
        fs::create_dir_all(&directory).map_err(|err| LockError::Unusable {
            path: directory.clone(),
            reason: err.to_string(),
        })?;
        let path = directory.join(LOCK_FILE);
        let holder = Holder {
            run_id: run_id.as_str().to_owned(),
            pid: process::id(),
        };
        match claim(&path, &holder) {
            Ok(()) => {
                return Ok(Self {
                    path,
                    holder,
                    reclaimed: None,
                });
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => {
                return Err(LockError::Unusable {
                    path,
                    reason: err.to_string(),
                });
            }
        }
        let previous = read_holder(&path);
        if let Some(existing) = &previous
            && is_alive(existing.pid)
        {
            return Err(LockError::Held {
                run_id: existing.run_id.clone(),
                pid: existing.pid,
            });
        }
        let reclaimed = previous.map_or_else(|| UNKNOWN_HOLDER.to_owned(), |it| it.run_id);
        if let Err(err) = fs::remove_file(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            return Err(LockError::Unusable {
                path,
                reason: err.to_string(),
            });
        }
        match claim(&path, &holder) {
            Ok(()) => Ok(Self {
                path,
                holder,
                reclaimed: Some(reclaimed),
            }),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Err(LockError::Contended),
            Err(err) => Err(LockError::Unusable {
                path,
                reason: err.to_string(),
            }),
        }
    }

    /// The run whose abandoned lock this one took over, if it took one over.
    /// Worth telling the reader: it means an earlier run did not finish.
    #[must_use]
    pub fn reclaimed_from(&self) -> Option<&str> {
        self.reclaimed.as_deref()
    }
}

impl Drop for Lock {
    /// Release the project, but only if it is still ours.
    ///
    /// A run that hangs long enough for its lock to be taken over as stale must
    /// not delete the lock of the run that took it: the check is what the file
    /// says now, not what this run wrote into it.
    fn drop(&mut self) {
        if read_holder(&self.path).as_ref() == Some(&self.holder) {
            drop(fs::remove_file(&self.path));
        }
    }
}

/// Write the lock, failing if one is already there. Creating the file is the
/// claim, so it has to be one operation the filesystem either does or refuses.
fn claim(path: &Path, holder: &Holder) -> io::Result<()> {
    use std::io::Write as _;

    let document = serde_json::to_string(holder)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(document.as_bytes())
}

/// Who a lock says holds the project. A lock that cannot be read says nobody:
/// there is no process to wait for, so it is treated as one a dead run left.
fn read_holder(path: &Path) -> Option<Holder> {
    let document = fs::read_to_string(path).ok()?;
    serde_json::from_str(&document).ok()
}

/// Whether the process holding a lock is still running.
///
/// Signal zero asks the question without sending anything. "No such process"
/// means the run is gone; anything else — in practice, "not permitted", which is
/// how a process belonging to another user answers — means it is not.
fn is_alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    match rustix::process::test_kill_process(pid) {
        Ok(()) => true,
        Err(errno) => errno != rustix::io::Errno::SRCH,
    }
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
    let lock = project_root.join(TREMULA_DIR).join(LOCK_FILE);
    if let Some(holder) = read_holder(&lock)
        && is_alive(holder.pid)
    {
        return Err(RestoreError::InProgress {
            run_id: holder.run_id,
            pid: holder.pid,
        });
    }
    let snapshot = run_dir.join(SNAPSHOT_DIR);
    if !snapshot.is_dir() {
        return Err(RestoreError::NoSnapshot {
            run_dir: run_dir.to_path_buf(),
        });
    }
    let mut restored = Vec::new();
    copy_back(&snapshot, project_root, Path::new(""), &mut restored)?;
    restored.sort();
    // Whatever lock is here belongs to a run that is gone, since a live one was
    // refused above. Clearing it is the other half of recovering from a crash.
    drop(fs::remove_file(&lock));
    Ok(restored)
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
            fs::copy(entry.path(), &destination).map_err(|err| unusable(&destination, &err))?;
            restored.push(here.to_string_lossy().into_owned());
        }
    }
    Ok(())
}
