//! `tremula run` from the outside, with a language pack that only pretends.
//!
//! The pack here is a script, which is what makes these cases reachable: a real
//! pack cannot be asked to report success without writing its documents, or to
//! write documents belonging to another run. What is under test is the core's
//! share of a run — the order it does things in, what it records, and what it
//! refuses.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command as Process, Output},
};

use assert_cmd::Command;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula::validation::canonical_mutant_id;
use tremula_contracts::{manifest::Span, report::Report};

const TARGET: &str = "src/overlap.py";
const SOURCE: &str = "def overlaps(start, other):\n    return start < other.end\n";
const ORIGINAL: &str = "start < other.end";
const REPLACEMENT: &str = "start <= other.end";

/// The pack the handshake below describes, which the identity checks compare the
/// results against.
const PACK_NAME: &str = "tremula-python";
const PACK_VERSION: &str = "0.1.0";

/// The contract version the handshake below reports, which the documents a run
/// judges have to agree with.
const CONTRACT_VERSION: &str = "0.1";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A workspace holding a project, a manifest, and a stand-in interpreter.
struct Workspace {
    root: TempDir,
}

impl Workspace {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let target = root.path().join("project").join(TARGET);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, SOURCE).unwrap();
        Self { root }
    }

    fn project(&self) -> PathBuf {
        self.root.path().join("project")
    }

    fn manifest(&self) -> PathBuf {
        self.root.path().join("manifest.json")
    }

    fn python(&self) -> PathBuf {
        self.root.path().join("python")
    }

    fn write_manifest(&self, mutants: usize) -> &Self {
        fs::write(self.manifest(), manifest_json(mutants)).unwrap();
        self
    }

    /// An interpreter that answers the handshake and then does `body` when asked
    /// to run. `$out` is bound to the run directory it was given.
    fn write_pack(&self, body: &str) -> &Self {
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *importlib.metadata*)\n    echo '{PACK_VERSION}'\n    exit 0\n    ;;\n  *--capabilities*)\n    echo '{}'\n    exit 0\n    ;;\nesac\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"--out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\n{body}\n",
            capabilities()
        );
        fs::write(self.python(), script).unwrap();
        fs::set_permissions(self.python(), fs::Permissions::from_mode(0o755)).unwrap();
        self
    }

    /// A pack that leaves the two documents a finished run owes, stamped with
    /// whatever `run_id` evaluates to in the shell — the run's real name, or
    /// another run's.
    fn write_finishing_pack(&self, run_id: &str) -> &Self {
        self.write_finishing_pack_speaking(run_id, CONTRACT_VERSION)
    }

    /// The same, with the contract version the results claim the pack implements
    /// as a parameter: a pack swapped for another between the handshake and the
    /// work is what makes it differ from what the handshake said.
    fn write_finishing_pack_speaking(&self, run_id: &str, contract: &str) -> &Self {
        let ids = mutant_ids(2);
        let body = format!(
            "run_id={run_id}\ncat > \"$out/baseline.json\" <<DOCUMENT\n{}\nDOCUMENT\ncat > \"$out/results.json\" <<DOCUMENT\n{}\nDOCUMENT\nexit 0",
            baseline_json("$run_id"),
            results_json("$run_id", &ids, contract)
        );
        self.write_pack(&body)
    }

    fn run(&self, extra: &[&str]) -> Output {
        let mut command = Command::cargo_bin("tremula").unwrap();
        command
            .arg("run")
            .arg("--manifest")
            .arg(self.manifest())
            .arg("--project")
            .arg(self.project())
            .arg("--python")
            .arg(self.python())
            .args(extra);
        command.output().unwrap()
    }

    /// The one run directory the project has.
    fn run_dir(&self) -> PathBuf {
        let runs = self.project().join(".tremula").join("runs");
        fs::canonicalize(runs.join("latest")).unwrap()
    }
}

fn capabilities() -> String {
    format!(
        r#"{{"name":"{PACK_NAME}","version":"{PACK_VERSION}","contract_version":"{CONTRACT_VERSION}","subcommands":["run","collect","validate"],"validate_checks":["parses"]}}"#
    )
}

fn span_of(index: usize) -> Span {
    let start = u64::try_from(SOURCE.find(ORIGINAL).unwrap()).unwrap();
    let width = u64::try_from(ORIGINAL.len()).unwrap();
    // A second mutant needs a span of its own; only its identifier matters here,
    // and a narrower span inside the first is still a span the file has.
    Span {
        start_byte: start,
        end_byte: start + width - u64::try_from(index).unwrap(),
    }
}

fn replacement_of(index: usize) -> String {
    format!("{}{}", REPLACEMENT, " ".repeat(index))
}

fn original_of(index: usize) -> String {
    let span = span_of(index);
    let start = usize::try_from(span.start_byte).unwrap();
    let end = usize::try_from(span.end_byte).unwrap();
    SOURCE[start..end].to_owned()
}

fn mutant_ids(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            canonical_mutant_id(
                TARGET,
                &span_of(index),
                &sha256_hex(SOURCE.as_bytes()),
                &replacement_of(index),
            )
        })
        .collect()
}

fn manifest_json(mutants: usize) -> String {
    let digest = sha256_hex(SOURCE.as_bytes());
    let listed: Vec<serde_json::Value> = (0..mutants)
        .map(|index| {
            let span = span_of(index);
            serde_json::json!({
                "id": canonical_mutant_id(TARGET, &span, &digest, &replacement_of(index)),
                "file": TARGET,
                "base_file_sha256": digest,
                "span": { "start_byte": span.start_byte, "end_byte": span.end_byte },
                "original": original_of(index),
                "replacement": replacement_of(index),
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": "0.1",
        "language": "python",
        "base": {},
        "mutants": listed,
    })
    .to_string()
}

fn runner_json(failed: u32, exit_class: &str) -> serde_json::Value {
    serde_json::json!({
        "exit_class": exit_class,
        "passed": 14 - failed,
        "failed": failed,
        "errors": 0,
        "skipped": 0,
        "collected": 14,
        "collected_ids_hash": "b5c7d9e1f3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7",
        "collect_error": false,
        "timed_out": false,
        "duration_ms": 1180
    })
}

fn baseline_json(run_id: &str) -> String {
    serde_json::json!({
        "schema_version": "0.1",
        "run_id": run_id,
        "runner": runner_json(0, "ok"),
    })
    .to_string()
}

/// One mutant caught, one not — the shape every report has to be able to carry.
fn results_json(run_id: &str, ids: &[String], contract: &str) -> String {
    let entries: Vec<serde_json::Value> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let (failed, exit_class) = if index == 0 {
                (1, "test_failures")
            } else {
                (0, "ok")
            };
            serde_json::json!({
                "mutant_id": id,
                "execution_status": "completed",
                "location": { "line": 2, "column": 11 },
                "runner": runner_json(failed, exit_class),
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": "0.1",
        "run_id": run_id,
        "pack": { "name": PACK_NAME, "version": PACK_VERSION, "contract_version": contract },
        "entries": entries,
    })
    .to_string()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A manifest with nothing to test still gets a run of its own, and the pack is
/// never asked about it: this interpreter fails if it is run at all.
#[test]
fn a_manifest_with_nothing_to_test_never_reaches_the_pack() {
    let workspace = Workspace::new();
    workspace.write_manifest(0);
    fs::write(
        workspace.python(),
        "#!/bin/sh\ntouch \"$(dirname \"$0\")/was-called\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(workspace.python(), fs::Permissions::from_mode(0o755)).unwrap();

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("nothing to test"), "{said}");
    assert!(said.contains("run_dir="), "{said}");
    assert!(
        !workspace.root.path().join("was-called").exists(),
        "the pack was called for a manifest with nothing to test"
    );
}

#[test]
fn a_run_judges_the_packs_results_and_leaves_a_report_machines_can_read() {
    let workspace = Workspace::new();
    // The run's name is the run directory's, which the pack is handed and reads
    // back off the path — exactly as the real one does.
    workspace
        .write_manifest(2)
        .write_finishing_pack("$(basename \"$out\")");

    let output = workspace.run(&[]);

    let said = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(said.contains("KILLED"), "{said}");
    assert!(said.contains("SURVIVED"), "{said}");
    assert!(said.contains("score: 1/2 killed"), "{said}");
    let document = fs::read_to_string(workspace.run_dir().join("report.json")).unwrap();
    let report: Report = serde_json::from_str(&document).unwrap();
    assert_eq!(report.exit_code, 1);
    assert_eq!(report.verdicts.len(), 2);
    assert!(!report.caveats.is_empty());
}

/// A project that is not a repository has no revision to record, and saying so
/// is not the same as recording nothing.
#[test]
fn a_project_outside_a_repository_records_no_revision() {
    let workspace = Workspace::new();
    workspace.write_manifest(0);

    workspace.run(&[]);

    let document = fs::read_to_string(workspace.run_dir().join("report.json")).unwrap();
    let report: Report = serde_json::from_str(&document).unwrap();
    assert_eq!(report.run.observed_revision, None);
    assert!(!report.run.dirty);
}

/// The run directory is created inside the project, so a run that measured the
/// working tree after writing it would call every clean project modified.
#[test]
fn a_clean_repository_is_not_reported_as_modified_by_the_run_itself() {
    let workspace = Workspace::new();
    workspace.write_manifest(0);
    let project = workspace.project();
    for arguments in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.email", "tests@example.invalid"],
        vec!["config", "user.name", "tremula tests"],
        vec!["add", "."],
        vec!["commit", "--message", "the project as it was"],
    ] {
        let done = Process::new("git")
            .arg("-C")
            .arg(&project)
            .args(&arguments)
            .output()
            .unwrap();
        assert!(done.status.success(), "git {arguments:?}");
    }

    workspace.run(&[]);

    let document = fs::read_to_string(workspace.run_dir().join("report.json")).unwrap();
    let report: Report = serde_json::from_str(&document).unwrap();
    let revision = report.run.observed_revision.unwrap_or_default();
    assert_eq!(revision.len(), 40, "{revision}");
    assert!(!report.run.dirty, "the run's own directory made it dirty");
}

#[test]
fn a_pack_that_reports_a_failure_says_what_it_means_and_whether_the_sources_survived() {
    let workspace = Workspace::new();
    workspace.write_manifest(1).write_pack(
        "echo '{\"error\":{\"stage\":\"baseline\",\"code\":\"baseline_failed\",\"message\":\"3 tests failed before any mutation was applied\"}}'\nexit 2",
    );

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("your test suite failed"), "{complaint}");
    assert!(complaint.contains("baseline_failed"), "{complaint}");
    assert!(
        complaint.contains("your source files are intact"),
        "{complaint}"
    );
}

/// A pack that mutated a file and died leaves the reader with a modified tree, so
/// the run says so and names the command that undoes it — this run's command, not
/// the one whose defaults happen to match. A project or an output directory the
/// reader chose is exactly what `tremula restore` on its own would fail to find.
#[test]
fn a_failed_run_that_left_a_file_mutated_names_the_command_that_undoes_it() {
    let workspace = Workspace::new();
    workspace.write_manifest(1).write_pack(&format!(
        "printf '%s' 'mutated' > \"{}\"\necho 'no diagnosis'\nexit 1",
        workspace.project().join(TARGET).display()
    ));

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("tremula restore"), "{complaint}");
    assert!(
        complaint.contains(&format!("--project {}", workspace.project().display())),
        "{complaint}"
    );
    assert_eq!(
        fs::canonicalize(run_named_in(&complaint)).unwrap(),
        workspace.run_dir()
    );
}

/// The run directory a message tells the reader to restore from.
fn run_named_in(complaint: &str) -> &str {
    let (_, after) = complaint.split_once("--run ").expect(complaint);
    let (path, _) = after.split_once('`').expect(complaint);
    path
}

/// A pack that answers the handshake and then writes documents claiming another
/// contract version is a pack that was swapped out mid-run. Judging what it left
/// would mean reading a document by rules it was not written to.
#[test]
fn documents_from_a_pack_speaking_another_contract_version_are_refused() {
    let workspace = Workspace::new();
    workspace
        .write_manifest(2)
        .write_finishing_pack_speaking("$(basename \"$out\")", "9.9");

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("pack contract version"), "{complaint}");
    assert!(complaint.contains("9.9"), "{complaint}");
}

#[test]
fn a_pack_that_reports_success_without_its_documents_is_a_defect_in_the_pack() {
    let workspace = Workspace::new();
    workspace.write_manifest(1).write_pack("exit 0");

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("defect in the pack"), "{complaint}");
    assert!(complaint.contains("baseline.json"), "{complaint}");
}

/// Documents stamped with another run would be judged as if they were this
/// run's, and the report would describe work nobody asked for.
#[test]
fn documents_belonging_to_another_run_are_refused() {
    let workspace = Workspace::new();
    workspace
        .write_manifest(2)
        .write_finishing_pack("20260101T000000Z-abcdef");

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("different run"), "{complaint}");
}

/// One source tree, patched in place. The lock is what keeps a second run from
/// corrupting the first one's sources, and this process is certainly alive.
#[test]
fn a_run_on_a_project_that_is_already_being_run_is_refused() {
    let workspace = Workspace::new();
    workspace.write_manifest(1).write_pack("exit 0");
    let directory = workspace.project().join(".tremula");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("lock"),
        format!(
            r#"{{"run_id":"20260101T000000Z-abcdef","pid":{}}}"#,
            std::process::id()
        ),
    )
    .unwrap();

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("in progress"), "{complaint}");
}

#[test]
fn a_manifest_from_another_contract_version_is_refused_with_both_versions() {
    let workspace = Workspace::new();
    fs::write(
        workspace.manifest(),
        r#"{"schema_version": "9.9", "language": "python", "base": {}, "mutants": []}"#,
    )
    .unwrap();
    workspace.write_pack("exit 0");

    let output = workspace.run(&[]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("9.9"), "{complaint}");
    assert!(complaint.contains("0.1"), "{complaint}");
}

/// Zero would be obeyed exactly — every suite killed as it started, every mutant
/// a timeout, and a report that blamed the code for it. A limit that is not a
/// number at all is worse: `inf` parses, and a run given it would wait for a hung
/// suite forever while looking like a run with a time limit.
#[test]
fn a_time_limit_that_is_not_a_positive_number_of_seconds_is_refused() {
    let workspace = Workspace::new();
    workspace.write_manifest(0).write_pack("exit 0");

    for spelling in ["0", "inf", "nan"] {
        let output = workspace.run(&["--timeout", spelling]);

        assert_eq!(output.status.code(), Some(2), "{spelling}");
        let complaint = stderr(&output);
        assert!(
            complaint.contains("more than zero"),
            "{spelling}: {complaint}"
        );
        assert!(complaint.contains(spelling), "{spelling}: {complaint}");
    }
}
