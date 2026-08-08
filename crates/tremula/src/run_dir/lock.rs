//! The claim that keeps two runs off one source tree.
//!
//! Mutants are applied in place, so a project may be measured by one run at a
//! time. Everything about how the lock is written follows from two facts: a run
//! can be killed without releasing anything, and two runs can want the same
//! project at the same moment. So the lock records the process holding it — one
//! whose process is gone is taken over rather than obeyed — and every step that
//! decides who holds the project is a step no other run can interleave with.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use super::{RunId, TREMULA_DIR};

/// Name of the lock inside the project's own directory.
const LOCK_FILE: &str = "lock";

/// Name of the file whose advisory lock decides who may take an abandoned lock
/// over. It is created once and never removed: unlinking a file other processes
/// are waiting on would let two of them through at once.
const GATE_FILE: &str = "lock.gate";

/// What the file a lock's document is staged through is called.
const STAGED: &str = "new";

/// What an abandoned lock is called once it has been moved aside to be taken
/// over.
const MOVED_ASIDE: &str = "stale";

/// What a lock left by a run whose identity could not be read is called. A
/// corrupt lock is still a lock somebody left, and the sentence it appears in
/// reads as an account of what happened rather than as a name.
const UNKNOWN_HOLDER: &str = "an earlier run";

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

/// Which run holds a project, in which process, and — while there is one — the
/// process of the pack it has working on the sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Holder {
    run_id: String,
    pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pack_pid: Option<u32>,
}

impl Holder {
    /// Whether anything this lock names is still running.
    ///
    /// The pack counts. A run killed while the pack was working leaves the pack
    /// itself applying mutants to the sources, and the project is not free until
    /// that process is gone too.
    fn still_running(&self) -> bool {
        is_alive(self.pid) || self.pack_pid.is_some_and(is_alive)
    }

    /// Whether this is the same claim as `other`.
    ///
    /// The pack is deliberately left out: it is recorded and cleared while the
    /// lock is held, so a run comparing the file against what it claimed would
    /// stop recognising its own lock the moment its pack started.
    fn is_same_claim(&self, other: &Self) -> bool {
        self.run_id == other.run_id && self.pid == other.pid
    }
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
        let holder = Holder {
            run_id: run_id.as_str().to_owned(),
            pid: process::id(),
            pack_pid: None,
        };
        Self::take(directory.join(LOCK_FILE), holder)
    }

    /// Claim the lock file, taking an abandoned one over if that is what is
    /// there.
    fn take(path: PathBuf, holder: Holder) -> Result<Self, LockError> {
        if try_claim(&path, &holder)? {
            return Ok(Self {
                path,
                holder,
                reclaimed: None,
            });
        }
        // Something is there. Whether it is a lock somebody holds or one an
        // abandoned run left has to be decided *and acted on* without another run
        // deciding the same thing about the same file: both would find it
        // abandoned, and both would end up patching the same sources. Reading the
        // lock and replacing it are separate operations, so the gate is what makes
        // the pair of them one.
        let gate = Gate::hold(&path)?;
        // The run that held it may have finished while this one waited its turn,
        // in which case there is nothing to take over.
        if try_claim(&path, &holder)? {
            return Ok(Self {
                path,
                holder,
                reclaimed: None,
            });
        }
        let previous = read_holder(&path);
        if let Some(existing) = &previous
            && existing.still_running()
        {
            return Err(LockError::Held {
                run_id: existing.run_id.clone(),
                pid: existing.pid,
            });
        }
        let reclaimed = previous.map_or_else(|| UNKNOWN_HOLDER.to_owned(), |it| it.run_id);
        // Moved aside rather than deleted, so that nothing is destroyed until this
        // run's own claim is in place.
        let aside = beside(&path, MOVED_ASIDE);
        if let Err(err) = fs::rename(&path, &aside)
            && err.kind() != io::ErrorKind::NotFound
        {
            return Err(LockError::Unusable {
                path,
                reason: err.to_string(),
            });
        }
        let claimed = try_claim(&path, &holder);
        drop(fs::remove_file(&aside));
        drop(gate);
        if claimed? {
            return Ok(Self {
                path,
                holder,
                reclaimed: Some(reclaimed),
            });
        }
        Err(LockError::Contended)
    }

    /// The run whose abandoned lock this one took over, if it took one over.
    /// Worth telling the reader: it means an earlier run did not finish.
    #[must_use]
    pub fn reclaimed_from(&self) -> Option<&str> {
        self.reclaimed.as_deref()
    }

    /// Record the process of the pack this run has working on the sources, or
    /// clear it once the pack has been collected.
    ///
    /// The pack is what actually patches the files, and it can outlive the run
    /// that started it — a run killed outright leaves it there. Naming it in the
    /// lock is what keeps the next run from reading the project as free while a
    /// pack is still editing it.
    ///
    /// The lock is ours, so the document is simply written; a lock another run has
    /// already taken over is left alone, for the same reason releasing one is.
    /// Failing to record it is not worth stopping a run over — the worst it costs
    /// is the extra caution above.
    pub fn note_pack(&self, pack: Option<u32>) {
        if !read_holder(&self.path).is_some_and(|current| current.is_same_claim(&self.holder)) {
            return;
        }
        let mut holder = self.holder.clone();
        holder.pack_pid = pack;
        // Renamed into place rather than written over: overwriting truncates
        // first, and a reader catching the file empty would take a held lock
        // for an abandoned one — the very window `claim` is built to close.
        let Ok(document) = serde_json::to_string(&holder) else {
            return;
        };
        let staged = beside(&self.path, STAGED);
        if fs::write(&staged, document).is_err() {
            return;
        }
        if fs::rename(&staged, &self.path).is_err() {
            drop(fs::remove_file(&staged));
        }
    }
}

impl Drop for Lock {
    /// Release the project, but only if it is still ours.
    ///
    /// A run that hangs long enough for its lock to be taken over as stale must
    /// not delete the lock of the run that took it: the check is what the file
    /// says now, not what this run wrote into it.
    fn drop(&mut self) {
        if read_holder(&self.path).is_some_and(|current| current.is_same_claim(&self.holder)) {
            drop(fs::remove_file(&self.path));
        }
    }
}

/// Try the claim once, saying whether it was made. A lock that is already there
/// is not a failure — it is the case the caller goes on to look into.
///
/// # Errors
///
/// Returns [`LockError::Unusable`] when the lock could not be written for any
/// other reason.
fn try_claim(path: &Path, holder: &Holder) -> Result<bool, LockError> {
    match claim(path, holder) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(LockError::Unusable {
            path: path.to_path_buf(),
            reason: err.to_string(),
        }),
    }
}

/// Write the lock, failing if one is already there.
///
/// Two things have to hold at once. The claim has to be a single operation the
/// filesystem either performs or refuses, which is what linking to a name that
/// already exists gives us. And a lock that exists at all has to be complete,
/// because a reader who found an empty one would conclude that nobody holds the
/// project and take it over. Writing the document under a name of its own and
/// linking the lock to it gives both: the lock appears only once its contents
/// are there.
fn claim(path: &Path, holder: &Holder) -> io::Result<()> {
    let document = serde_json::to_string(holder)?;
    let staged = beside(path, STAGED);
    fs::write(&staged, document)?;
    let claimed = fs::hard_link(&staged, path);
    drop(fs::remove_file(&staged));
    claimed
}

/// A name beside the lock that belongs to one call alone. Claiming a lock and
/// handing one over both need a file of their own first, and neither may collide
/// with another run's — or another thread's.
fn beside(path: &Path, purpose: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!("{LOCK_FILE}.{purpose}.{}.{ordinal}", process::id()))
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

/// The exclusive right to decide what becomes of an abandoned lock.
///
/// The kernel keeps this one, which is what makes waiting for it safe: a run
/// killed while it holds the gate has the gate released on its behalf, so no
/// crash can leave a project impossible to take over. It is held across a
/// handful of filesystem operations and never across a run.
struct Gate(fs::File);

impl Gate {
    /// Wait for the right to take over the lock at `path`.
    fn hold(path: &Path) -> Result<Self, LockError> {
        let gate = path.with_file_name(GATE_FILE);
        let unusable = |reason: String| LockError::Unusable {
            path: gate.clone(),
            reason,
        };
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gate)
            .map_err(|err| unusable(err.to_string()))?;
        file.lock().map_err(|err| unusable(err.to_string()))?;
        Ok(Self(file))
    }
}

impl Drop for Gate {
    /// Closing the file releases the lock on its own; saying so is clearer than
    /// relying on it.
    fn drop(&mut self) {
        drop(self.0.unlock());
    }
}
