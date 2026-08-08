//! Finding the Python that belongs to the project under test, and refusing an
//! interpreter that cannot host the language pack.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use tempfile::TempDir;
use tremula::python_env::{EnvError, PythonEnv, discover_from};

/// A virtual environment with an interpreter that is never actually run.
fn fake_venv(root: &Path, name: &str) -> std::path::PathBuf {
    let interpreter = root.join(name).join("bin").join("python");
    fs::create_dir_all(interpreter.parent().unwrap()).unwrap();
    write_script(&interpreter, "#!/bin/sh\nexit 0\n");
    interpreter
}

/// An executable script standing in for an interpreter.
fn write_script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn an_explicit_interpreter_wins_over_every_other_candidate() {
    let workspace = TempDir::new().unwrap();
    let chosen = fake_venv(workspace.path(), "chosen");
    let active = workspace.path().join("active");
    fake_venv(workspace.path(), "active");
    let project = workspace.path().join("project");
    fake_venv(&project, ".venv");

    let env = discover_from(Some(&chosen), Some(&active), &project).unwrap();

    assert_eq!(env.interpreter(), chosen);
}

#[test]
fn the_active_virtualenv_wins_over_the_projects_own() {
    let workspace = TempDir::new().unwrap();
    let active = workspace.path().join("active");
    let expected = fake_venv(workspace.path(), "active");
    let project = workspace.path().join("project");
    fake_venv(&project, ".venv");

    let env = discover_from(None, Some(&active), &project).unwrap();

    assert_eq!(env.interpreter(), expected);
}

#[test]
fn the_projects_own_virtualenv_is_the_last_place_looked() {
    let workspace = TempDir::new().unwrap();
    let project = workspace.path().join("project");
    let expected = fake_venv(&project, ".venv");

    let env = discover_from(None, None, &project).unwrap();

    assert_eq!(env.interpreter(), expected);
}

/// An empty `VIRTUAL_ENV` is a variable that was exported and then cleared,
/// which names no environment at all.
#[test]
fn an_active_virtualenv_without_an_interpreter_falls_through() {
    let workspace = TempDir::new().unwrap();
    let active = workspace.path().join("stale-active");
    let project = workspace.path().join("project");
    let expected = fake_venv(&project, ".venv");

    let env = discover_from(None, Some(&active), &project).unwrap();

    assert_eq!(env.interpreter(), expected);
}

#[test]
fn with_nowhere_to_look_the_message_lists_every_candidate_and_the_way_out() {
    let workspace = TempDir::new().unwrap();
    let active = workspace.path().join("active");
    let project = workspace.path().join("project");

    let failure = discover_from(None, Some(&active), &project).unwrap_err();

    let complaint = failure.to_string();
    assert!(
        matches!(failure, EnvError::NoInterpreter { .. }),
        "{complaint}"
    );
    assert!(
        complaint.contains(&active.join("bin").join("python").display().to_string()),
        "{complaint}"
    );
    assert!(
        complaint.contains(
            &project
                .join(".venv")
                .join("bin")
                .join("python")
                .display()
                .to_string()
        ),
        "{complaint}"
    );
    assert!(complaint.contains("--python"), "{complaint}");
}

#[test]
fn an_explicit_interpreter_that_is_not_there_is_named() {
    let workspace = TempDir::new().unwrap();
    let nowhere = workspace.path().join("no-such-python");

    let failure = discover_from(Some(&nowhere), None, workspace.path()).unwrap_err();

    let complaint = failure.to_string();
    assert!(
        complaint.contains(&nowhere.display().to_string()),
        "{complaint}"
    );
    assert!(complaint.contains("--python"), "{complaint}");
}

/// The pack is what the interpreter is for, so an interpreter without it is
/// refused before any work starts — with the command that fixes it.
#[test]
fn an_interpreter_without_the_pack_is_refused_with_the_command_that_installs_it() {
    let workspace = TempDir::new().unwrap();
    let interpreter = workspace.path().join("python");
    write_script(
        &interpreter,
        "#!/bin/sh\necho \"ModuleNotFoundError: No module named 'tremula_python'\" >&2\nexit 1\n",
    );

    let failure = PythonEnv::at(&interpreter).verify_pack().unwrap_err();

    let complaint = failure.to_string();
    assert!(
        matches!(failure, EnvError::PackMissing { .. }),
        "{complaint}"
    );
    assert!(complaint.contains("tremula-python"), "{complaint}");
    assert!(complaint.contains("No module named"), "{complaint}");
}

#[test]
fn an_interpreter_that_cannot_be_run_at_all_is_reported_as_such() {
    let workspace = TempDir::new().unwrap();
    let interpreter = workspace.path().join("python");
    // Present, but not executable: discovery accepts it and running it fails.
    fs::write(&interpreter, "not an interpreter\n").unwrap();
    fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o644)).unwrap();

    let failure = PythonEnv::at(&interpreter).verify_pack().unwrap_err();

    assert!(
        matches!(failure, EnvError::InterpreterUnusable { .. }),
        "{failure}"
    );
}

/// The real check, against a real editable install: the pack is accepted because
/// its distribution metadata is installed, not because its source happens to sit
/// somewhere the interpreter can see. An editable install puts the code outside
/// the interpreter's own directories, and a containment check would reject it.
#[test]
fn the_repositorys_own_virtualenv_hosts_the_pack() {
    let Some(interpreter) = repository_interpreter() else {
        return;
    };

    let pack = PythonEnv::at(&interpreter).verify_pack().unwrap();

    assert_eq!(pack.distribution, "tremula-python");
    assert!(
        pack.version.chars().next().is_some_and(char::is_numeric),
        "{}",
        pack.version
    );
}

/// The interpreter of the repository's own virtual environment, when there is
/// one. A checkout that has not been synced yet has no pack to verify, so the
/// tests that need one report nothing rather than failing.
fn repository_interpreter() -> Option<std::path::PathBuf> {
    let interpreter = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".venv")
        .join("bin")
        .join("python");
    if interpreter.is_file() {
        return Some(interpreter);
    }
    eprintln!(
        "skipped: no virtual environment at {}",
        interpreter.display()
    );
    None
}
