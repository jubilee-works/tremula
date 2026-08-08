//! The whole pipeline, on purpose-built projects: the real binary, the real
//! language pack, the real execution backend.
//!
//! One fixture tree per behaviour, and the tree's name is the behaviour. Each
//! test copies its tree into a directory of its own before running anything,
//! because mutants are applied in place: running in the repository would mutate
//! files under version control, and two tests sharing a tree would corrupt each
//! other's sources.
//!
//! Manifests are not committed alongside the trees. A mutant's identifier is
//! derived from the bytes of the file it targets, so a stored manifest would go
//! stale the moment a fixture was edited — with a failure that looked like a bug
//! in validation. The helper below derives them from the copy it just made.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

use assert_cmd::Command;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula::validation::canonical_mutant_id;
use tremula_contracts::{manifest::Span, report::Report, results::Results};

/// One mutation, spelled the way a person would describe it.
struct Mutation {
    file: &'static str,
    original: &'static str,
    replacement: &'static str,
}

/// A fixture tree copied somewhere it can be mutated, with a manifest derived
/// from the copy.
struct Fixture {
    workspace: TempDir,
    name: &'static str,
}

impl Fixture {
    /// Copy `name` out of the repository's fixtures.
    ///
    /// The copy keeps the tree's name, and every command below is run from the
    /// directory holding it with `--project <name>`. That is what a person would
    /// type, and it keeps the console's account of the project readable.
    fn copy(name: &'static str) -> Self {
        let workspace = TempDir::new().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests")
            .join("fixtures")
            .join(name);
        copy_tree(&source, &workspace.path().join(name));
        Self { workspace, name }
    }

    fn project(&self) -> PathBuf {
        self.workspace.path().join(self.name)
    }

    /// Write a manifest for `mutations`, derived from the files as they are now.
    fn with_manifest(&self, mutations: &[Mutation]) -> &Self {
        let mutants: Vec<serde_json::Value> = mutations
            .iter()
            .map(|mutation| self.describe(mutation))
            .collect();
        let document = serde_json::json!({
            "schema_version": "0.1",
            "language": "python",
            "base": { "revision": null },
            "mutants": mutants,
        });
        fs::write(self.manifest(), document.to_string()).unwrap();
        self
    }

    /// One mutant, with the span and hashes the copy's own bytes give it.
    fn describe(&self, mutation: &Mutation) -> serde_json::Value {
        let bytes = fs::read(self.project().join(mutation.file)).unwrap();
        let at = find(&bytes, mutation.original.as_bytes());
        let span = Span {
            start_byte: u64::try_from(at).unwrap(),
            end_byte: u64::try_from(at + mutation.original.len()).unwrap(),
        };
        let digest = format!("{:x}", Sha256::digest(&bytes));
        serde_json::json!({
            "id": canonical_mutant_id(mutation.file, &span, &digest, mutation.replacement),
            "file": mutation.file,
            "base_file_sha256": digest,
            "span": { "start_byte": span.start_byte, "end_byte": span.end_byte },
            "original": mutation.original,
            "replacement": mutation.replacement,
        })
    }

    fn manifest(&self) -> PathBuf {
        self.workspace.path().join("manifest.json")
    }

    /// Run tremula the way a person in this directory would.
    fn run(&self, extra: &[&str]) -> Output {
        let mut command = Command::cargo_bin("tremula").unwrap();
        command
            .current_dir(self.workspace.path())
            .arg("run")
            .arg("--manifest")
            .arg("manifest.json")
            .arg("--project")
            .arg(self.name)
            .arg("--python")
            .arg(interpreter())
            .args(extra);
        command.output().unwrap()
    }

    fn restore(&self) -> Output {
        let mut command = Command::cargo_bin("tremula").unwrap();
        command
            .current_dir(self.workspace.path())
            .arg("restore")
            .arg("--project")
            .arg(self.name);
        command.output().unwrap()
    }

    fn run_dir(&self) -> PathBuf {
        fs::canonicalize(self.project().join(".tremula").join("runs").join("latest")).unwrap()
    }

    fn results(&self) -> Results {
        let document = fs::read_to_string(self.run_dir().join("results.json")).unwrap();
        serde_json::from_str(&document).unwrap()
    }

    fn report(&self) -> Report {
        let document = fs::read_to_string(self.run_dir().join("report.json")).unwrap();
        serde_json::from_str(&document).unwrap()
    }
}

/// The interpreter the pack is installed in. These tests are gated on a feature
/// precisely because they need one.
fn interpreter() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".venv")
        .join("bin")
        .join("python")
}

fn copy_tree(from: &Path, into: &Path) {
    fs::create_dir_all(into).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = into.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), &destination).unwrap();
        }
    }
}

/// Where `needle` starts inside `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| {
            panic!(
                "the fixture does not contain {:?}",
                String::from_utf8_lossy(needle)
            )
        })
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Replace the two things about a console report that a second run would spell
/// differently: how long the suite took, and where this particular run's
/// directory is. Everything else — the identifiers included, since they are
/// derived from the fixture's own bytes — is the same every time.
fn without_the_incidentals(said: &str) -> String {
    said.lines()
        .map(|line| {
            if line.starts_with("run_dir=") {
                "run_dir=[path]".to_owned()
            } else if let Some((before, after)) = line.split_once(" passed in ") {
                let tail = after.split_once(' ').map_or("", |(_, rest)| rest);
                format!("{before} passed in [duration] {tail}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The case tremula exists for: a suite that catches one mutant and not the
/// other, and a report that says so without overstating either.
#[test]
fn a_suite_with_a_gap_in_it_reports_one_kill_and_one_survivor() {
    let fixture = Fixture::copy("sample_project");
    fixture.with_manifest(&[
        Mutation {
            file: "schedule.py",
            original: "other_start < end",
            replacement: "other_start <= end",
        },
        Mutation {
            file: "schedule.py",
            original: "minutes >= 60",
            replacement: "minutes > 60",
        },
    ]);

    let output = fixture.run(&[]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let said = stdout(&output);
    assert_eq!(said.matches("KILLED").count(), 1, "{said}");
    assert_eq!(said.matches("SURVIVED   ").count(), 1, "{said}");
    let report = fixture.report();
    assert_eq!(report.score.killed, 1);
    assert_eq!(report.score.survived, 1);
    assert_eq!(report.score.timeout, 0);
    assert_eq!(report.exit_code, 1);
    assert_eq!(report.caveats.len(), 2);
    insta::assert_snapshot!(without_the_incidentals(&said));
}

/// A change that touched no code produces a manifest with nothing in it, which
/// is a run like any other — except that no pack is asked anything.
#[test]
fn a_manifest_with_nothing_to_test_succeeds_without_running_a_suite() {
    let fixture = Fixture::copy("empty_manifest");
    fixture.with_manifest(&[]);

    let output = fixture.run(&[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("nothing to test"), "{said}");
    assert!(said.contains("run_dir="), "{said}");
    assert!(
        !fixture.run_dir().join("results.json").exists(),
        "a run with nothing to test called the pack anyway"
    );
}

/// A byte span cannot mean the same thing to tremula and to a backend that reads
/// the file through universal newlines, so a CRLF file is refused before the pack
/// is involved at all.
#[test]
fn a_target_file_with_carriage_returns_is_refused_before_anything_runs() {
    let fixture = Fixture::copy("crlf_rejected");
    fixture.with_manifest(&[Mutation {
        file: "windows.py",
        original: "start < end",
        replacement: "start <= end",
    }]);

    let output = fixture.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("convert the file to LF"), "{complaint}");
    assert!(
        !fixture.project().join(".tremula").join("runs").exists(),
        "a refused manifest still started a run"
    );
}

/// A manifest generated against an earlier revision describes spans the file no
/// longer has, and running it would mutate the wrong bytes.
#[test]
fn a_manifest_generated_against_older_sources_is_refused() {
    let fixture = Fixture::copy("stale_manifest");
    fixture.with_manifest(&[Mutation {
        file: "schedule.py",
        original: "minutes >= 60",
        replacement: "minutes > 60",
    }]);
    // The manifest is right about the file it was generated from; the edit is
    // what makes it stale, exactly as a commit between generating and running
    // would.
    let target = fixture.project().join("schedule.py");
    let edited = fs::read_to_string(&target).unwrap() + "\n\n# a later thought\n";
    fs::write(&target, edited).unwrap();

    let output = fixture.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("regenerate the manifest"), "{complaint}");
}

/// A mutant that makes the suite hang is a detection, not a failure of the run:
/// something about the code stopped terminating, which is what a test would have
/// caught if it had one.
#[test]
fn a_mutant_that_makes_the_suite_hang_is_counted_as_a_detection() {
    let fixture = Fixture::copy("timeout_mutant");
    fixture.with_manifest(&[Mutation {
        file: "countdown.py",
        original: "remaining > 0",
        replacement: "True",
    }]);

    let output = fixture.run(&["--timeout", "3"]);

    let said = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{}\n{said}", stderr(&output));
    assert!(said.contains("TIMEOUT"), "{said}");
    assert!(said.contains("(1 timeout)"), "{said}");
    assert!(said.contains("warning:"), "{said}");
    let report = fixture.report();
    assert_eq!(report.score.timeout, 1);
    assert_eq!(report.score.killed, 1, "a timeout counts towards killed");
}

/// A mutant that stops the module loading is *not* a kill, and the asymmetry is
/// deliberate. A test that failed would mean the suite reacted to the change; a
/// collection error means no test code ran at all, and every suite in existence
/// would "catch" it. Counting those would inflate a score that is supposed to say
/// something about the tests, so the run has no usable verdict and says so.
#[test]
fn a_mutant_that_stops_the_module_loading_produces_no_usable_verdict() {
    let fixture = Fixture::copy("import_breaker");
    fixture.with_manifest(&[Mutation {
        file: "limits.py",
        original: "60",
        replacement: "1 // 0",
    }]);

    let output = fixture.run(&[]);

    let said = stdout(&output);
    assert_eq!(output.status.code(), Some(2), "{}\n{said}", stderr(&output));
    assert!(said.contains("RUNTIME_ERROR"), "{said}");
    assert!(said.contains("no usable verdict"), "{said}");
    let report = fixture.report();
    assert_eq!(report.score.runtime_error, 1);
    assert_eq!(report.score.killed, 0);
    assert_eq!(report.score.survived, 0);
    // The asymmetry, in the signals themselves: the pack reported an error, which
    // taken alone would be a kill, *and* a failed collection. Collection is
    // decided first, so this is not counted.
    let results = fixture.results();
    let runner = results.entries[0].runner.as_ref().unwrap();
    assert!(runner.collect_error, "{runner:?}");
    assert!(runner.errors > 0, "{runner:?}");
}

/// The snapshot is what a run leaves for the reader when something goes wrong
/// with the sources, so it has to survive the run and still be right afterwards.
#[test]
fn restoring_puts_back_a_file_that_was_left_modified() {
    let fixture = Fixture::copy("sample_project");
    fixture.with_manifest(&[Mutation {
        file: "schedule.py",
        original: "minutes >= 60",
        replacement: "minutes > 60",
    }]);
    fixture.run(&[]);
    let target = fixture.project().join("schedule.py");
    let original = fs::read_to_string(&target).unwrap();
    fs::write(&target, "# whatever a killed run left behind\n").unwrap();

    let output = fixture.restore();

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("schedule.py"),
        "{}",
        stdout(&output)
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    assert!(!fixture.project().join(".tremula").join("lock").exists());
}

/// One source tree, patched in place. A second run has to be refused, and this
/// test's own process is the liveliest possible holder of the lock.
#[test]
fn a_run_started_while_another_holds_the_project_is_refused() {
    let fixture = Fixture::copy("sample_project");
    fixture.with_manifest(&[Mutation {
        file: "schedule.py",
        original: "minutes >= 60",
        replacement: "minutes > 60",
    }]);
    let directory = fixture.project().join(".tremula");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("lock"),
        format!(
            r#"{{"run_id":"20260101T000000Z-abcdef","pid":{}}}"#,
            std::process::id()
        ),
    )
    .unwrap();

    let output = fixture.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("in progress"), "{complaint}");
}
