//! A run's own directory: how it is named, how it keeps other runs out, and how
//! it gets the project's sources back.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::Path, process::Command};

use tempfile::TempDir;
use time::macros::datetime;
use tremula::{
    run_dir::{self, Lock, LockError, RestoreError, RunId},
    validation::VerifiedTarget,
};

const MANIFEST_SHA: &str = "3b1f8c9d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b";

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

fn lock_exists(project_root: &Path) -> bool {
    project_root.join(".tremula").join("lock").exists()
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
