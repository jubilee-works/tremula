//! A run's own directory: how it is named, how it keeps other runs out, and how
//! it gets the project's sources back.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{self, Command},
    sync::Barrier,
    thread,
};

use tempfile::TempDir;
use time::macros::datetime;
use tremula::{
    run_dir::{self, Lock, LockError, RestoreError, RunId},
    validation::VerifiedTarget,
};

const MANIFEST_SHA: &str = "3b1f8c9d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b";

/// How many runs of one manifest can start in the same second and still each get
/// a directory: the name itself, and the suffixed ones after it.
const NAMES_PER_SECOND: usize = 9;

/// How many runs go for one abandoned lock at the same moment below.
const CONTENDERS: usize = 8;

/// How many times that race is run. Once would prove nothing about a race: the
/// window a wrong hand-over opens is a few instructions wide, and these many
/// rounds hit it every time.
const ROUNDS: usize = 50;

fn run_id() -> RunId {
    RunId::new(datetime!(2026-08-09 04:15:00 UTC), MANIFEST_SHA)
}

/// A project root with the parent directory a lock lives under.
fn project() -> TempDir {
    TempDir::new().unwrap()
}

/// The identifier of a process that has certainly finished: started, waited for,
/// and reaped. A zombie would not do — signalling one succeeds, so it would read
/// as alive.
fn finished_process() -> i32 {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    i32::try_from(pid).unwrap()
}

fn write_lock(project_root: &Path, run_id: &str, pid: i32) {
    let directory = project_root.join(".tremula");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("lock"),
        format!(r#"{{"run_id":"{run_id}","pid":{pid}}}"#),
    )
    .unwrap();
}

/// A lock as a run leaves it while the pack is working: its own process, and the
/// pack's.
fn write_lock_naming_pack(project_root: &Path, run_id: &str, pid: i32, pack_pid: u32) {
    let directory = project_root.join(".tremula");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("lock"),
        format!(r#"{{"run_id":"{run_id}","pid":{pid},"pack_pid":{pack_pid}}}"#),
    )
    .unwrap();
}

fn lock_exists(project_root: &Path) -> bool {
    project_root.join(".tremula").join("lock").exists()
}

/// Everything the project's own directory holds, sorted. A claim and a hand-over
/// both stage a file there, and neither may leave one behind.
fn entries(project_root: &Path) -> Vec<String> {
    let Ok(listing) = fs::read_dir(project_root.join(".tremula")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = listing
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_run_is_named_for_the_moment_it_started_and_the_manifest_it_ran() {
    assert_eq!(run_id().as_str(), "20260809T041500Z-3b1f8c");
}

#[test]
fn the_same_manifest_run_again_is_a_different_run() {
    let later = RunId::new(datetime!(2026-08-09 04:15:01 UTC), MANIFEST_SHA);

    assert_ne!(run_id().as_str(), later.as_str());
}

#[test]
fn a_run_directory_is_reserved_rather_than_reused() {
    let parent = TempDir::new().unwrap();

    let first = run_dir::create(parent.path(), &run_id()).unwrap();
    let second = run_dir::create(parent.path(), &run_id()).unwrap();

    assert_ne!(first.path(), second.path());
    assert_eq!(first.run_id(), "20260809T041500Z-3b1f8c");
    assert!(second.path().is_dir());
}

/// A run is named for the second it started in and the manifest it ran, so runs of
/// one manifest that start in the same second have nothing else to tell them apart.
/// Each still gets a directory of its own, until the suffixed names run out: two
/// runs sharing one directory would interleave their documents, so the alternative
/// to a new name is refusing the run.
#[test]
fn runs_of_one_manifest_in_one_second_each_get_a_directory_of_their_own() {
    let parent = TempDir::new().unwrap();

    let reserved: Vec<_> = (0..NAMES_PER_SECOND)
        .map(|_| run_dir::create(parent.path(), &run_id()).unwrap())
        .collect();

    let refused = run_dir::create(parent.path(), &run_id()).unwrap_err();
    let mut paths: Vec<_> = reserved.iter().map(|run| run.path().to_owned()).collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), NAMES_PER_SECOND);
    assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists, "{refused}");
}

#[test]
fn the_latest_symlink_follows_the_newest_run() {
    let parent = TempDir::new().unwrap();
    let first = run_dir::create(parent.path(), &run_id()).unwrap();
    let second = run_dir::create(
        parent.path(),
        &RunId::new(datetime!(2026-08-09 04:16:00 UTC), MANIFEST_SHA),
    )
    .unwrap();

    run_dir::publish_latest(parent.path(), first.path()).unwrap();
    run_dir::publish_latest(parent.path(), second.path()).unwrap();

    let latest = fs::canonicalize(parent.path().join("latest")).unwrap();
    assert_eq!(latest, fs::canonicalize(second.path()).unwrap());
}

#[test]
fn a_project_with_no_run_in_progress_can_be_locked() {
    let root = project();

    let lock = Lock::acquire(root.path(), &run_id()).unwrap();

    assert!(lock_exists(root.path()));
    assert_eq!(lock.reclaimed_from(), None);
}

/// One source tree, patched in place: a second run would corrupt the first one's
/// sources, so the lock says no and says what to do about it.
#[test]
fn a_second_run_on_the_same_project_is_refused_with_the_run_that_holds_it() {
    let root = project();
    let held = Lock::acquire(root.path(), &run_id()).unwrap();

    let failure = Lock::acquire(
        root.path(),
        &RunId::new(datetime!(2026-08-09 04:16:00 UTC), MANIFEST_SHA),
    )
    .unwrap_err();

    let complaint = failure.to_string();
    assert!(matches!(failure, LockError::Held { .. }), "{complaint}");
    assert!(complaint.contains("20260809T041500Z-3b1f8c"), "{complaint}");
    assert!(complaint.contains("in progress"), "{complaint}");
    drop(held);
}

/// A run killed outright leaves its lock behind. The next run takes it over —
/// otherwise one crash would lock the project for good.
#[test]
fn a_lock_left_by_a_run_that_is_gone_is_taken_over() {
    let root = project();
    write_lock(root.path(), "20260808T101500Z-aaaaaa", finished_process());

    let lock = Lock::acquire(root.path(), &run_id()).unwrap();

    assert_eq!(lock.reclaimed_from(), Some("20260808T101500Z-aaaaaa"));
}

/// A lock whose process cannot be signalled is held by another user, not stale.
/// Process 1 is the portable example: it is always running and never ours.
#[test]
fn a_lock_held_by_another_users_process_is_not_mistaken_for_a_stale_one() {
    let root = project();
    write_lock(root.path(), "20260808T101500Z-bbbbbb", 1);

    let failure = Lock::acquire(root.path(), &run_id()).unwrap_err();

    assert!(matches!(failure, LockError::Held { .. }), "{failure}");
}

/// A run killed while the pack was working leaves the pack running, and the pack
/// is what applies mutants to the sources. The project is not free until that
/// process is gone too, so the lock names it and a lock naming a live pack is a
/// lock somebody holds.
#[test]
fn a_lock_whose_run_is_gone_but_whose_pack_is_not_is_still_held() {
    let root = project();
    write_lock_naming_pack(
        root.path(),
        "20260808T101500Z-aaaaaa",
        finished_process(),
        process::id(),
    );

    let failure = Lock::acquire(root.path(), &run_id()).unwrap_err();

    let complaint = failure.to_string();
    assert!(matches!(failure, LockError::Held { .. }), "{complaint}");
    assert!(complaint.contains("20260808T101500Z-aaaaaa"), "{complaint}");
}

/// While the pack is running the lock names its process too, and stops naming it
/// once the pack has been reaped. Whose lock it is does not change either way: the
/// run that took it is still the one that releases it.
#[test]
fn the_lock_names_the_packs_process_only_while_the_pack_is_running() {
    let root = project();
    let path = root.path().join(".tremula").join("lock");
    let lock = Lock::acquire(root.path(), &run_id()).unwrap();

    lock.note_pack(Some(4242));
    let while_running = fs::read_to_string(&path).unwrap();
    lock.note_pack(None);
    let once_reaped = fs::read_to_string(&path).unwrap();

    assert!(while_running.contains("4242"), "{while_running}");
    assert!(!once_reaped.contains("4242"), "{once_reaped}");
    assert!(
        once_reaped.contains("20260809T041500Z-3b1f8c"),
        "{once_reaped}"
    );
}

/// A run that named its pack in the lock still recognises the lock as its own,
/// which is what releasing one depends on.
#[test]
fn a_run_releases_a_lock_that_still_names_its_pack() {
    let root = project();
    let lock = Lock::acquire(root.path(), &run_id()).unwrap();
    lock.note_pack(Some(4242));

    drop(lock);

    assert!(!lock_exists(root.path()));
}

/// The lock is claimed by linking it to a document that is already whole, so a
/// reader can never find a half-written one and take it for a lock nobody holds.
/// The file the document was staged through is not part of the claim, and does not
/// outlive it.
#[test]
fn a_lock_names_its_run_and_leaves_nothing_beside_it() {
    let root = project();

    let lock = Lock::acquire(root.path(), &run_id()).unwrap();

    let document = fs::read_to_string(root.path().join(".tremula").join("lock")).unwrap();
    assert!(document.contains("20260809T041500Z-3b1f8c"), "{document}");
    assert_eq!(entries(root.path()), ["lock"]);
    drop(lock);
    assert!(
        entries(root.path()).is_empty(),
        "{:?}",
        entries(root.path())
    );
}

/// Runs that find the same abandoned lock must not all take it over: each would
/// apply mutants to the sources the others were measuring. However the race falls
/// out, one run ends up holding the project and the rest are told to wait.
///
/// Reading a lock and replacing it are separate operations, so a run that decides
/// a lock is abandoned can have that answer go out of date before it acts on it.
/// Taking a lock over is therefore done one run at a time, and the file the turns
/// are taken through is the one extra name left in the project's directory.
#[test]
fn runs_that_find_one_abandoned_lock_leave_exactly_one_holding_the_project() {
    for round in 0..ROUNDS {
        let root = project();
        write_lock(root.path(), "20260808T101500Z-aaaaaa", finished_process());
        let start = Barrier::new(CONTENDERS);

        let outcomes: Vec<Result<Lock, LockError>> = thread::scope(|scope| {
            let racing: Vec<_> = (0..CONTENDERS)
                .map(|which| {
                    let (start, root) = (&start, root.path());
                    scope.spawn(move || {
                        let id =
                            RunId::new(datetime!(2026-08-09 04:15:00 UTC), &format!("{which:06}"));
                        start.wait();
                        Lock::acquire(root, &id)
                    })
                })
                .collect();
            racing
                .into_iter()
                .map(|contender| contender.join().unwrap())
                .collect()
        });

        let holding = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        assert_eq!(holding, 1, "round {round}: {outcomes:?}");
        for refused in outcomes.iter().filter_map(|outcome| outcome.as_ref().err()) {
            assert!(
                matches!(refused, LockError::Held { .. } | LockError::Contended),
                "round {round}: {refused}"
            );
        }
        assert_eq!(entries(root.path()), ["lock", "lock.gate"], "round {round}");
    }
}

#[test]
fn a_lock_file_that_makes_no_sense_is_treated_as_stale() {
    let root = project();
    let directory = root.path().join(".tremula");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("lock"), "not a lock").unwrap();

    let lock = Lock::acquire(root.path(), &run_id()).unwrap();

    assert_eq!(lock.reclaimed_from(), Some("an earlier run"));
}

#[test]
fn a_finished_run_releases_the_project() {
    let root = project();

    drop(Lock::acquire(root.path(), &run_id()).unwrap());

    assert!(!lock_exists(root.path()));
}

/// Once another run has taken a stale lock over, the lock belongs to that run.
/// The first run's own release must not remove it.
#[test]
fn releasing_a_lock_that_another_run_took_over_leaves_it_alone() {
    let root = project();
    let ours = Lock::acquire(root.path(), &run_id()).unwrap();
    write_lock(root.path(), "20260809T042000Z-cccccc", 1);

    drop(ours);

    assert!(lock_exists(root.path()));
}

/// The bytes the manifest was checked against are the bytes the snapshot keeps,
/// so restoring puts the file back exactly as validation saw it.
#[test]
fn a_snapshot_puts_the_projects_files_back_as_they_were() {
    let root = project();
    let source = root.path().join("src").join("schedule.py");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, "def overlaps():\n    return True\n").unwrap();
    let run = run_dir::create(&root.path().join("runs"), &run_id()).unwrap();
    run_dir::snapshot(
        run.path(),
        &[VerifiedTarget {
            file: "src/schedule.py".to_owned(),
            bytes: b"def overlaps():\n    return True\n".to_vec(),
        }],
    )
    .unwrap();
    fs::write(&source, "def overlaps():\n    return False\n").unwrap();

    let restored = run_dir::restore(run.path(), root.path()).unwrap();

    assert_eq!(restored, ["src/schedule.py"]);
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "def overlaps():\n    return True\n"
    );
}

/// A link standing where a target file belongs would have restoring write through
/// it and truncate whatever it points at, which is how a file outside the project
/// gets destroyed by putting the project's own sources back. The snapshot says a
/// file belongs at that path, so the link itself is what gets replaced.
#[test]
fn restoring_replaces_a_link_standing_where_a_target_belongs() {
    let root = project();
    let elsewhere = TempDir::new().unwrap();
    let bystander = elsewhere.path().join("notes.txt");
    fs::write(&bystander, "not this run's to touch\n").unwrap();
    let run = run_dir::create(&root.path().join("runs"), &run_id()).unwrap();
    run_dir::snapshot(
        run.path(),
        &[VerifiedTarget {
            file: "schedule.py".to_owned(),
            bytes: b"def overlaps():\n    return True\n".to_vec(),
        }],
    )
    .unwrap();
    let target = root.path().join("schedule.py");
    std::os::unix::fs::symlink(&bystander, &target).unwrap();

    let restored = run_dir::restore(run.path(), root.path()).unwrap();

    assert_eq!(restored, ["schedule.py"]);
    assert_eq!(
        fs::read_to_string(&bystander).unwrap(),
        "not this run's to touch\n"
    );
    assert!(
        !fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "def overlaps():\n    return True\n"
    );
}

/// A crash leaves a lock behind, and restoring is how the reader recovers, so
/// restoring clears it.
#[test]
fn restoring_clears_the_lock_a_crashed_run_left() {
    let root = project();
    let run = run_dir::create(&root.path().join("runs"), &run_id()).unwrap();
    run_dir::snapshot(run.path(), &[]).unwrap();
    write_lock(root.path(), "20260809T041500Z-3b1f8c", finished_process());

    run_dir::restore(run.path(), root.path()).unwrap();

    assert!(!lock_exists(root.path()));
}

/// Restoring claims the project for the whole copy, so a copy that fails part way
/// through has to release it again: leaving it claimed would refuse the reader's
/// next command on behalf of a restore that is no longer happening.
#[test]
fn a_restore_that_cannot_finish_still_releases_the_project() {
    let root = project();
    let run = run_dir::create(&root.path().join("runs"), &run_id()).unwrap();
    run_dir::snapshot(
        run.path(),
        &[VerifiedTarget {
            file: "sealed/schedule.py".to_owned(),
            bytes: b"def overlaps():\n    return True\n".to_vec(),
        }],
    )
    .unwrap();
    // A directory the copy cannot write into is the cheapest way to fail half way
    // through one.
    let sealed = root.path().join("sealed");
    fs::create_dir_all(&sealed).unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o500)).unwrap();

    let failure = run_dir::restore(run.path(), root.path()).unwrap_err();

    let complaint = failure.to_string();
    assert!(
        matches!(failure, RestoreError::Unusable { .. }),
        "{complaint}"
    );
    assert!(!lock_exists(root.path()), "{complaint}");
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o700)).unwrap();
}

/// Restoring during a live run would overwrite the mutant it was measuring.
#[test]
fn restoring_while_a_run_is_in_progress_is_refused() {
    let root = project();
    let run = run_dir::create(&root.path().join("runs"), &run_id()).unwrap();
    run_dir::snapshot(run.path(), &[]).unwrap();
    let held = Lock::acquire(root.path(), &run_id()).unwrap();

    let failure = run_dir::restore(run.path(), root.path()).unwrap_err();

    let complaint = failure.to_string();
    assert!(
        matches!(failure, RestoreError::InProgress { .. }),
        "{complaint}"
    );
    assert!(complaint.contains("in progress"), "{complaint}");
    drop(held);
}

#[test]
fn restoring_a_run_that_is_not_there_says_so() {
    let root = project();

    let failure = run_dir::restore(&root.path().join("no-such-run"), root.path()).unwrap_err();

    assert!(
        matches!(failure, RestoreError::NoRunDirectory { .. }),
        "{failure}"
    );
}

#[test]
fn restoring_a_run_that_kept_no_snapshot_says_so() {
    let root = project();
    let run = run_dir::create(&root.path().join("runs"), &run_id()).unwrap();

    let failure = run_dir::restore(run.path(), root.path()).unwrap_err();

    assert!(
        matches!(failure, RestoreError::NoSnapshot { .. }),
        "{failure}"
    );
}
