//! The whole hand-off, on a purpose-built project: a real run, a real bundle, and the
//! bug in it reproduced somewhere the run never touched.
//!
//! What this proves that the unit tests cannot is the one promise a bundle makes to
//! somebody who was not there — "apply this patch to this revision and you have the
//! bug". Keeping that promise honest means the repository has to exist *before* the
//! run: a run of a project that is not a checkout records no revision, and a
//! repository created afterwards would have a commit the run never measured, so
//! applying the patch to it would prove nothing about what the bundle claims.
//!
//! The patch is applied in a clone rather than in the project. Applying it in place
//! would succeed against the very sources the diff was made from, which is what the
//! bundle already checked when it was built; a clone at the named revision is the
//! situation the promise is actually about.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as Process, Output},
};

use assert_cmd::Command;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula::validation::canonical_mutant_id;
use tremula_contracts::{
    bundle::BundleIndex,
    manifest::{Manifest, Span},
    report::{Report, Verdict},
};

/// The project both tests measure, and the mutation the suite fails to catch.
const PROJECT: &str = "sample_project";
const FILE: &str = "schedule.py";
const ORIGINAL: &str = "minutes >= 60";
const REPLACEMENT: &str = "minutes > 60";

/// The one test file the run is told to collect, so `suite.tests` has something in it.
const TESTS: &str = "test_schedule.py";

/// A fixture tree copied somewhere it can be mutated, optionally as a checkout.
struct Fixture {
    workspace: TempDir,
}

impl Fixture {
    /// Copy the project out of the repository's fixtures.
    fn copy() -> Self {
        let workspace = TempDir::new().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests")
            .join("fixtures")
            .join(PROJECT);
        copy_tree(&source, &workspace.path().join(PROJECT));
        Self { workspace }
    }

    fn project(&self) -> PathBuf {
        self.workspace.path().join(PROJECT)
    }

    fn manifest(&self) -> PathBuf {
        self.workspace.path().join("manifest.json")
    }

    /// Make the project a checkout with everything in it committed, and say what the
    /// commit is. Done before the run, which is the whole point of the test.
    fn commit_everything(&self) -> String {
        for arguments in [
            vec!["init", "--initial-branch=main"],
            vec!["config", "user.email", "tests@example.invalid"],
            vec!["config", "user.name", "tremula tests"],
            vec!["add", "."],
            vec!["commit", "--message", "the project as it was measured"],
        ] {
            let done = git(&self.project(), &arguments);
            assert!(
                done.status.success(),
                "git {arguments:?}: {}",
                String::from_utf8_lossy(&done.stderr)
            );
        }
        let head = git(&self.project(), &["rev-parse", "HEAD"]);
        String::from_utf8_lossy(&head.stdout).trim().to_owned()
    }

    /// Write a one-mutant manifest, derived from the copy's own bytes.
    ///
    /// Outside the project, so that making the project a checkout and then running does
    /// not report a modified working tree.
    fn with_manifest(&self) -> Span {
        let bytes = fs::read(self.project().join(FILE)).unwrap();
        let at = find(&bytes, ORIGINAL.as_bytes());
        let span = Span {
            start_byte: u64::try_from(at).unwrap(),
            end_byte: u64::try_from(at + ORIGINAL.len()).unwrap(),
        };
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let document = serde_json::json!({
            "schema_version": "0.1",
            "language": "python",
            "base": { "revision": null },
            "mutants": [{
                "id": canonical_mutant_id(FILE, &span, &digest, REPLACEMENT),
                "file": FILE,
                "base_file_sha256": digest,
                "span": { "start_byte": span.start_byte, "end_byte": span.end_byte },
                "original": ORIGINAL,
                "replacement": REPLACEMENT,
            }],
        });
        fs::write(self.manifest(), document.to_string()).unwrap();
        span
    }

    /// Run the mutant against the project's own suite.
    fn run(&self) -> Output {
        Command::cargo_bin("tremula")
            .unwrap()
            .current_dir(self.workspace.path())
            .args(["run", "--manifest", "manifest.json", "--project", PROJECT])
            .arg("--python")
            .arg(interpreter())
            .args(["--tests", TESTS])
            .output()
            .unwrap()
    }

    /// Package what the run left.
    fn bundle(&self, extra: &[&str]) -> Output {
        Command::cargo_bin("tremula")
            .unwrap()
            .current_dir(self.workspace.path())
            .args(["bundle", "--project", PROJECT, "--out", "bundle"])
            .args(extra)
            .output()
            .unwrap()
    }

    fn bundled(&self) -> PathBuf {
        self.workspace.path().join("bundle")
    }

    fn run_dir(&self) -> PathBuf {
        fs::canonicalize(self.project().join(".tremula").join("runs").join("latest")).unwrap()
    }

    fn report(&self) -> Report {
        read(&self.run_dir().join("report.json"))
    }

    fn manifest_document(&self) -> Manifest {
        read(&self.manifest())
    }

    fn index(&self) -> BundleIndex {
        read(&self.bundled().join("bundle.json"))
    }
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let document =
        fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&document).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn git(directory: &Path, arguments: &[&str]) -> Output {
    Process::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap()
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The promise, kept: the revision the run recorded is the commit the project was at,
/// and the patch the bundle carries turns a clone of that commit into what the manifest
/// says the mutation is.
#[test]
fn a_bundle_reproduces_its_bug_in_a_checkout_of_the_revision_it_names() {
    let fixture = Fixture::copy();
    let span = fixture.with_manifest();
    // Before the run, so that what the run records is a commit that really holds the
    // bytes it measured.
    let head = fixture.commit_everything();
    assert_eq!(head.len(), 40, "{head}");

    let run = fixture.run();
    assert_eq!(run.status.code(), Some(1), "{}", stderr(&run));
    let report = fixture.report();
    assert_eq!(
        report.run.observed_revision.as_deref(),
        Some(head.as_str()),
        "the run recorded a revision that is not the one it measured"
    );
    assert!(!report.run.dirty, "the run's own directory made it dirty");
    assert_eq!(report.run.tests, vec![TESTS]);
    assert_eq!(report.verdicts.len(), 1);
    assert_eq!(report.verdicts[0].verdict, Verdict::Survived);

    let bundled = fixture.bundle(&[]);

    assert_eq!(bundled.status.code(), Some(0), "{}", stderr(&bundled));
    let index = fixture.index();
    assert_eq!(index.run_id, report.run.run_id);
    assert_eq!(index.project, PROJECT);
    assert_eq!(index.base.revision.as_deref(), Some(head.as_str()));
    assert!(index.base.reproducible_from_revision);
    assert_eq!(index.suite.tests, vec![TESTS]);
    assert_eq!(index.attachments.len(), 1);
    let patch = index.attachments[0]
        .patch
        .as_ref()
        .expect("a survivor's patch is what the bundle exists for");

    // A clone of the project is a checkout the run never touched, which is the
    // situation the bundle's promise is about.
    let elsewhere = fixture.workspace.path().join("elsewhere");
    let cloned = Process::new("git")
        .arg("clone")
        .arg("--quiet")
        .arg(fixture.project())
        .arg(&elsewhere)
        .output()
        .unwrap();
    assert!(cloned.status.success(), "{}", stderr(&cloned));
    let out = git(&elsewhere, &["checkout", "--quiet", &head]);
    assert!(out.status.success(), "{}", stderr(&out));
    let before = fs::read(elsewhere.join(FILE)).unwrap();

    let applied = git(
        &elsewhere,
        &[
            "apply",
            &fixture.bundled().join(&patch.path).display().to_string(),
        ],
    );

    assert!(applied.status.success(), "{}", stderr(&applied));
    // What the manifest says the mutation is, done to the bytes of that revision.
    let mutant = &fixture.manifest_document().mutants[0];
    let mut expected = before.clone();
    let start = usize::try_from(span.start_byte).unwrap();
    let end = usize::try_from(span.end_byte).unwrap();
    expected.splice(start..end, mutant.replacement.bytes());
    assert_eq!(
        fs::read(elsewhere.join(FILE)).unwrap(),
        expected,
        "the patch did something other than what the manifest describes"
    );
}

/// The log of a real suite, which is where a machine's own paths actually come from:
/// nothing here is written by a fixture, so this is the only place the cleaning is held
/// to output a test framework really produced.
#[test]
fn a_bundle_from_a_real_run_carries_no_path_from_the_machine_it_ran_on() {
    let fixture = Fixture::copy();
    fixture.with_manifest();
    let run = fixture.run();
    assert_eq!(run.status.code(), Some(1), "{}", stderr(&run));

    let bundled = fixture.bundle(&[]);

    assert_eq!(bundled.status.code(), Some(0), "{}", stderr(&bundled));
    let index = fixture.index();
    assert!(index.exposure.standalone_logs);
    let project = fixture.project().display().to_string();
    let resolved = fs::canonicalize(fixture.project())
        .unwrap()
        .display()
        .to_string();
    let logs = fixture.bundled().join("logs");
    let mut read_any = false;
    for entry in fs::read_dir(&logs).unwrap() {
        let path = entry.unwrap().path();
        let said = fs::read_to_string(&path).unwrap();
        assert!(!said.contains(&project), "{}: {said}", path.display());
        assert!(!said.contains(&resolved), "{}: {said}", path.display());
        assert!(
            !said.contains("TREMULA-RESULT"),
            "{}: {said}",
            path.display()
        );
        read_any = true;
    }
    assert!(read_any, "no log travelled at all");
    assert!(logs.join("baseline.txt").is_file());
}

/// A project that is not a checkout. The run records no revision, so the bundle says
/// there is nothing to check out and its starting document says the same — which is the
/// case anybody trying this on a scratch copy meets first.
#[test]
fn a_run_outside_a_checkout_produces_a_bundle_that_says_so() {
    let fixture = Fixture::copy();
    fixture.with_manifest();
    let run = fixture.run();
    assert_eq!(run.status.code(), Some(1), "{}", stderr(&run));
    assert_eq!(fixture.report().run.observed_revision, None);

    let bundled = fixture.bundle(&[]);

    assert_eq!(bundled.status.code(), Some(0), "{}", stderr(&bundled));
    let index = fixture.index();
    assert_eq!(index.base.revision, None);
    assert!(!index.base.reproducible_from_revision);
    assert!(!index.base.dirty);
    let said = stdout(&bundled);
    assert!(said.contains("no revision"), "{said}");
    let starting = fs::read_to_string(fixture.bundled().join("START_HERE.md")).unwrap();
    assert!(starting.contains("no revision to return to"), "{starting}");
}

/// A run of a manifest with nothing in it leaves a report and no other document. It is a
/// successful run, and a bundle of it would be a bundle of nothing.
#[test]
fn a_run_that_tested_nothing_is_not_packaged_and_is_not_a_failure() {
    let fixture = Fixture::copy();
    fs::write(
        fixture.manifest(),
        r#"{"schema_version":"0.1","language":"python","base":{},"mutants":[]}"#,
    )
    .unwrap();
    let run = fixture.run();
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));

    let bundled = fixture.bundle(&[]);

    assert_eq!(bundled.status.code(), Some(0), "{}", stderr(&bundled));
    assert!(
        stdout(&bundled).contains("nothing to package"),
        "{}",
        stdout(&bundled)
    );
    assert!(!fixture.bundled().exists());
}
