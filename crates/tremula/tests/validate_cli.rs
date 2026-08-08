//! `tremula validate` from the outside: the exit codes it promises and the
//! guidance its messages carry.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Output,
};

use assert_cmd::Command;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula::validation::canonical_mutant_id;
use tremula_contracts::manifest::Span;

const TARGET: &str = "src/overlap.py";
const SOURCE: &str = "def overlaps(start, other):\n    return start < other.end\n";
const ORIGINAL: &str = "start < other.end";
const REPLACEMENT: &str = "start <= other.end";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A project whose one source file the manifests below target.
fn project() -> TempDir {
    let root = TempDir::new().unwrap();
    let path = root.path().join(TARGET);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, SOURCE).unwrap();
    root
}

fn write_manifest(root: &TempDir, contents: &str) -> PathBuf {
    let path = root.path().join("manifest.json");
    fs::write(&path, contents).unwrap();
    path
}

fn manifest_json(base_file_sha256: &str) -> String {
    manifest_json_with(base_file_sha256, REPLACEMENT)
}

/// A manifest for the project's one file. The hash and the replacement are
/// parameters so that a test can hand over a hash the file no longer has, or a
/// replacement large enough to be worth a warning.
fn manifest_json_with(base_file_sha256: &str, replacement: &str) -> String {
    let start = u64::try_from(SOURCE.find(ORIGINAL).unwrap()).unwrap();
    let span = Span {
        start_byte: start,
        end_byte: start + u64::try_from(ORIGINAL.len()).unwrap(),
    };
    serde_json::json!({
        "schema_version": "0.1",
        "language": "python",
        "base": {},
        "mutants": [{
            "id": canonical_mutant_id(TARGET, &span, base_file_sha256, replacement),
            "file": TARGET,
            "base_file_sha256": base_file_sha256,
            "span": { "start_byte": span.start_byte, "end_byte": span.end_byte },
            "original": ORIGINAL,
            "replacement": replacement,
        }],
    })
    .to_string()
}

fn run_validate(root: &Path, manifest: &Path, extra: &[&str]) -> Output {
    let mut command = Command::cargo_bin("tremula").unwrap();
    command
        .arg("validate")
        .arg("--manifest")
        .arg(manifest)
        .arg("--project")
        .arg(root)
        .args(extra);
    command.output().unwrap()
}

/// `tremula validate` as somebody standing in the project types it: paths relative
/// to where they are, and the interpreter left to be discovered.
///
/// Whatever virtual environment the test runner itself was started from is cleared,
/// so that what gets discovered is the project's own.
fn run_validate_in(directory: &Path, manifest: &str, extra: &[&str]) -> Output {
    let mut command = Command::cargo_bin("tremula").unwrap();
    command
        .current_dir(directory)
        .env_remove("VIRTUAL_ENV")
        .arg("validate")
        .arg("--manifest")
        .arg(manifest)
        .arg("--project")
        .arg(".")
        .args(extra);
    command.output().unwrap()
}

/// A virtual environment in the project whose interpreter answers the one question
/// the core asks of it — which pack is installed — and accepts anything else.
fn fake_venv(root: &Path) {
    let interpreter = root.join(".venv").join("bin").join("python");
    fs::create_dir_all(interpreter.parent().unwrap()).unwrap();
    fs::write(
        &interpreter,
        "#!/bin/sh\ncase \"$*\" in\n  *importlib.metadata*)\n    echo '0.1.0'\n    ;;\nesac\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o755)).unwrap();
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_manifest_that_matches_the_project_is_accepted() {
    let root = project();
    let manifest = write_manifest(&root, &manifest_json(&sha256_hex(SOURCE.as_bytes())));

    let output = run_validate(root.path(), &manifest, &[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn a_manifest_with_nothing_to_test_is_accepted() {
    let root = project();
    let manifest = write_manifest(
        &root,
        r#"{"schema_version": "0.1", "language": "python", "base": {}, "mutants": []}"#,
    );

    let output = run_validate(root.path(), &manifest, &[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn a_manifest_written_against_older_sources_is_rejected_with_the_next_step() {
    let root = project();
    let manifest = write_manifest(&root, &manifest_json(&sha256_hex(b"an older revision")));

    let output = run_validate(root.path(), &manifest, &[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("regenerate the manifest"), "{complaint}");
}

#[test]
fn a_manifest_that_is_not_valid_json_is_rejected_with_the_reason() {
    let root = project();
    let manifest = write_manifest(
        &root,
        r#"{"schema_version": "0.1", "language": "python", "base": {}, "mutants": ["#,
    );

    let output = run_validate(root.path(), &manifest, &[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("is not a valid manifest"), "{complaint}");
}

#[test]
fn a_manifest_path_that_cannot_be_read_is_rejected_with_the_reason() {
    let root = project();
    let nowhere = root.path().join("no-such-directory").join("manifest.json");

    let output = run_validate(root.path(), &nowhere, &[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("cannot read manifest"), "{complaint}");
    assert!(complaint.contains("check the path"), "{complaint}");
}

/// A warning is advice, not a defect: it is reported and the manifest is still
/// accepted.
#[test]
fn an_oversized_replacement_is_reported_without_failing() {
    let root = project();
    let bulky = "x".repeat(10 * 1024 + 1);
    let manifest = write_manifest(
        &root,
        &manifest_json_with(&sha256_hex(SOURCE.as_bytes()), &bulky),
    );

    let output = run_validate(root.path(), &manifest, &[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let advice = stderr(&output);
    assert!(advice.contains("warning:"), "{advice}");
    assert!(advice.contains("10241 bytes"), "{advice}");
}

/// `--deep` is the one form of validation that needs the project's own Python,
/// so it fails the way a missing interpreter fails rather than silently skipping
/// the pack's checks.
#[test]
fn deep_validation_without_an_interpreter_says_which_one_it_wanted() {
    let root = project();
    let manifest = write_manifest(&root, &manifest_json(&sha256_hex(SOURCE.as_bytes())));
    let nowhere = root.path().join("no-such-python");

    let output = run_validate(
        root.path(),
        &manifest,
        &["--deep", "--python", &nowhere.display().to_string()],
    );

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(
        complaint.contains(&nowhere.display().to_string()),
        "{complaint}"
    );
}

/// A discovered interpreter has to keep meaning the same thing after the core
/// leaves the caller's directory, which it does: the pack is probed from a
/// directory belonging to no project. An interpreter remembered as
/// `.venv/bin/python`, next to a project spelled `.`, would be looked for there
/// instead — and not found.
#[test]
fn a_project_named_relative_to_here_still_finds_the_interpreter_beside_it() {
    let root = project();
    write_manifest(&root, &manifest_json(&sha256_hex(SOURCE.as_bytes())));
    fake_venv(root.path());

    let output = run_validate_in(root.path(), "manifest.json", &["--deep"]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

/// The same thing on the repository itself, with its real virtual environment and
/// its real pack: the command a reader would type from a checkout.
#[test]
fn deep_validation_runs_from_the_repository_with_a_relative_project() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !repository
        .join(".venv")
        .join("bin")
        .join("python")
        .is_file()
    {
        eprintln!("skipped: no virtual environment in the repository");
        return;
    }

    let output = run_validate_in(
        &repository,
        "contracts/examples/manifest/minimal.json",
        &["--deep"],
    );

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

/// The real thing: the installed pack is asked about a manifest it can accept.
#[test]
fn deep_validation_asks_the_installed_pack_and_accepts_a_valid_manifest() {
    let interpreter = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".venv")
        .join("bin")
        .join("python");
    if !interpreter.is_file() {
        eprintln!(
            "skipped: no virtual environment at {}",
            interpreter.display()
        );
        return;
    }
    let root = project();
    let manifest = write_manifest(&root, &manifest_json(&sha256_hex(SOURCE.as_bytes())));

    let output = run_validate(
        root.path(),
        &manifest,
        &["--deep", "--python", &interpreter.display().to_string()],
    );

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}
