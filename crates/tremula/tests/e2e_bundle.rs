//! The whole hand-off, on a purpose-built project: a real run, a real bundle, and the
//! mutation in it reproduced somewhere the run never touched.
//!
//! What this proves that the unit tests cannot is the one promise a bundle makes to
//! somebody who was not there — "apply this patch to this revision and you have the
//! mutation". Keeping that promise honest means the repository has to exist *before* the
//! run: a run of a project that is not a checkout records no revision, and a
//! repository created afterwards would have a commit the run never measured, so
//! applying the patch to it would prove nothing about what the bundle claims.
//!
//! The patch is applied in a clone rather than in the project. Applying it in place
//! would succeed against the very sources the diff was made from, which is what the
//! bundle already checked when it was built; a clone at the named revision is the
//! situation the promise is actually about.
//!
//! # Why the commands are read out of the bundle
//!
//! Because the promise is made by `START_HERE.md` and not by this file. A test that built
//! `git -C <checkout> apply <absolute path>` itself would prove that a patch applies while
//! leaving the document free to tell a reader something that cannot be run — which is
//! exactly what it did. So the commands are parsed out of the document the bundle
//! published, the two names it asks the reader to substitute are substituted, and they are
//! run from a directory that is neither the bundle nor the checkout: anything depending on
//! a working directory the document never named fails here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    ffi::OsStr,
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

/// The test a reader of the bundle would write: it passes on the original and fails while
/// the patch is applied, which is what the starting document asks for. Appended to the file
/// the run collected, since a new file the selection does not name would not be run at all.
const KILLING_TEST: &str = "

def test_an_hour_long_meeting_needs_a_break() -> None:
    assert needs_break(60)
";

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

/// Run the mutant and package what the run left, holding both to what they claim: the
/// revision the run recorded is the commit the project was at, and the bundle is of that
/// run, that project, and that suite.
fn measured_and_packaged(fixture: &Fixture, head: &str) -> BundleIndex {
    let run = fixture.run();
    assert_eq!(run.status.code(), Some(1), "{}", stderr(&run));
    let report = fixture.report();
    assert_eq!(
        report.run.observed_revision.as_deref(),
        Some(head),
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
    assert_eq!(index.base.revision.as_deref(), Some(head));
    assert!(index.base.reproducible_from_revision);
    assert_eq!(index.suite.tests, vec![TESTS]);
    assert_eq!(index.attachments.len(), 1);
    assert!(
        index.attachments[0].patch.is_some(),
        "a survivor's patch is what the bundle exists for"
    );
    index
}

/// A clone of the project: a checkout the run never touched, which is the situation the
/// bundle's promise is about. Nothing else is done to it — the document's own commands put
/// it at the revision and apply the patch.
fn cloned(fixture: &Fixture) -> PathBuf {
    let checkout = fixture.workspace.path().join("elsewhere");
    let done = Process::new("git")
        .arg("clone")
        .arg("--quiet")
        .arg(fixture.project())
        .arg(&checkout)
        .output()
        .unwrap();
    assert!(done.status.success(), "{}", stderr(&done));
    checkout
}

/// Every command the starting document sets out in a block of its own, in order.
fn commands_in(document: &str) -> Vec<String> {
    document
        .lines()
        .filter_map(|line| line.strip_prefix("    "))
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The one command the document gives inside a sentence rather than in a block: how to
/// take a patch back out again.
fn undoing(document: &str) -> String {
    document
        .split('`')
        .find(|span| span.starts_with("git ") && span.contains("apply -R"))
        .map(str::to_owned)
        .expect("the document says to take the patch back out and does not say how")
}

/// One of the document's commands with the names it asks the reader to substitute
/// substituted, and nothing else changed.
fn substituted(command: &str, bundle: &Path, checkout: &Path, mutant_id: &str) -> String {
    command
        .replace("<BUNDLE>", &bundle.display().to_string())
        .replace("<CHECKOUT>", &checkout.display().to_string())
        .replace("<mutant-id>", mutant_id)
}

/// Run one of the document's commands, from a directory that is neither the bundle nor the
/// checkout: a command that depended on a working directory the document never named would
/// fail here, which is the whole point of running them this way.
fn as_written(command: &str, from: &Path, extra: &[&OsStr]) -> Output {
    let words = words(command);
    let (program, arguments) = words
        .split_first()
        .unwrap_or_else(|| panic!("no command in {command:?}"));
    // The document names the command the way a reader has it on their path; this test has
    // to run the one it just built.
    let program = if program == "tremula" {
        assert_cmd::cargo::cargo_bin("tremula")
    } else {
        PathBuf::from(program)
    };
    Process::new(program)
        .args(arguments)
        .args(extra)
        .current_dir(from)
        .output()
        .unwrap()
}

/// The words of a command, with a quoted run kept together, the way a shell reads it.
fn words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for character in command.chars() {
        match character {
            '\'' => quoted = !quoted,
            ' ' if !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(character),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Run the suite the way the project's own tests are run, to see one test fail.
fn pytest(suite: &Path, from: &Path) -> Output {
    Process::new(interpreter())
        .args(["-m", "pytest", "-q"])
        .arg(suite)
        .current_dir(from)
        .output()
        .unwrap()
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

/// The promise, kept by doing what the bundle says: the revision the run recorded is the
/// commit the project was at, the document's own commands turn a clone of that commit into
/// what the manifest says the mutation is, and the test the document asks for turns the
/// survivor into a kill.
#[test]
fn following_the_starting_document_reproduces_the_mutation_and_then_kills_it() {
    let fixture = Fixture::copy();
    let span = fixture.with_manifest();
    // Before the run, so that what the run records is a commit that really holds the
    // bytes it measured.
    let head = fixture.commit_everything();
    assert_eq!(head.len(), 40, "{head}");

    let index = measured_and_packaged(&fixture, &head);
    let mutant_id = index.attachments[0].id.clone();

    let checkout = cloned(&fixture);
    let document = fs::read_to_string(fixture.bundled().join("START_HERE.md")).unwrap();
    let steps = commands_in(&document);
    assert_eq!(
        steps.len(),
        3,
        "the document gives commands this test does not account for: {steps:?}"
    );
    let substituting =
        |command: &str| substituted(command, &fixture.bundled(), &checkout, &mutant_id);

    // The checkout, then the patch: the two steps that are the bundle's whole promise.
    for step in &steps[..2] {
        let done = as_written(&substituting(step), fixture.workspace.path(), &[]);
        assert!(done.status.success(), "{step}: {}", stderr(&done));
    }

    // What the manifest says the mutation is, done to the bytes of that revision.
    let at_the_revision = git(&checkout, &["show", &format!("{head}:{FILE}")]);
    assert!(
        at_the_revision.status.success(),
        "{}",
        stderr(&at_the_revision)
    );
    let mutant = &fixture.manifest_document().mutants[0];
    let mut expected = at_the_revision.stdout.clone();
    let start = usize::try_from(span.start_byte).unwrap();
    let end = usize::try_from(span.end_byte).unwrap();
    expected.splice(start..end, mutant.replacement.bytes());
    assert_eq!(
        fs::read(checkout.join(FILE)).unwrap(),
        expected,
        "the document's commands did something other than what the manifest describes"
    );

    // And the rest of what the document asks for: a test that fails while the patch is
    // applied, the patch taken back out, and the run it spells out.
    let suite = checkout.join(TESTS);
    let mut with_a_test = fs::read_to_string(&suite).unwrap();
    with_a_test.push_str(KILLING_TEST);
    fs::write(&suite, &with_a_test).unwrap();
    let failing = pytest(&suite, &checkout);
    assert!(
        !failing.status.success(),
        "the test does not fail while the patch is applied: {}",
        stdout(&failing)
    );

    let reverting = substituting(&undoing(&document));
    let undone = as_written(&reverting, fixture.workspace.path(), &[]);
    assert!(undone.status.success(), "{reverting}: {}", stderr(&undone));
    let passing = pytest(&suite, &checkout);
    assert!(
        passing.status.success(),
        "the test does not pass on the project as it is: {}",
        stdout(&passing)
    );
    let interpreter = interpreter().into_os_string();
    let killing = as_written(
        &substituting(&steps[2]),
        fixture.workspace.path(),
        // The one thing the document cannot know: which interpreter the language pack is
        // installed in. Everything else in the command is the document's own.
        &["--python".as_ref(), interpreter.as_os_str()],
    );

    assert_eq!(killing.status.code(), Some(0), "{}", stderr(&killing));
    let judged: Report = read(
        &fs::canonicalize(checkout.join(".tremula").join("runs").join("latest"))
            .unwrap()
            .join("report.json"),
    );
    assert_eq!(judged.verdicts.len(), 1);
    assert_eq!(
        judged.verdicts[0].verdict,
        Verdict::Killed,
        "the test the document asked for did not kill the mutation"
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
