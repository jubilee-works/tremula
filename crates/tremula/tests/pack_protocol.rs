//! Talking to a language pack: what the core requires of the handshake, and what
//! it makes of a pack that fails.
//!
//! The pack here is a script standing in for an interpreter, so that every answer
//! a pack could give — including the ones a working pack never gives — can be put
//! in front of the core.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf, sync::Mutex};

use tempfile::TempDir;
use tremula::{
    child::Reaped,
    pack::{self, PackError},
    python_env::PythonEnv,
};
use tremula_contracts::{SCHEMA_VERSION, manifest::Span, pack_error::Stage, probe::ProbeOutcome};

/// The capabilities of a pack the core is happy with.
const GOOD_CAPABILITIES: &str = r#"{"name":"tremula-python","version":"0.1.0","contract_version":"0.1","subcommands":["run","collect","validate"],"validate_checks":["parses"]}"#;

/// A stand-in interpreter. `body` is a shell script that receives the arguments
/// the core would pass a real one, and every invocation appends its arguments to
/// `arguments.txt` beside it first.
fn fake_pack(root: &Path, body: &str) -> PythonEnv {
    let interpreter = root.join("python");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"{}/arguments.txt\"\n{body}",
        root.display()
    );
    fs::write(&interpreter, script).unwrap();
    fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o755)).unwrap();
    PythonEnv::at(interpreter)
}

/// A pack that answers the handshake with `document` and nothing else.
fn pack_answering(root: &Path, document: &str) -> PythonEnv {
    fake_pack(root, &format!("cat <<'DOCUMENT'\n{document}\nDOCUMENT\n"))
}

fn arguments(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("arguments.txt"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A capabilities document with one field replaced.
fn capabilities_with(field: &str, value: &str) -> String {
    let mut document: serde_json::Value = serde_json::from_str(GOOD_CAPABILITIES).unwrap();
    document[field] = serde_json::from_str(value).unwrap();
    document.to_string()
}

#[test]
fn a_pack_that_speaks_the_contract_is_accepted() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(workspace.path(), GOOD_CAPABILITIES);

    let capabilities = pack::handshake(&env).unwrap();

    assert_eq!(capabilities.name, "tremula-python");
    assert_eq!(capabilities.contract_version, SCHEMA_VERSION);
}

/// The handshake runs the module, not a script, and isolated: a `tremula_python`
/// directory in the working directory must not be able to answer for the
/// installed pack.
#[test]
fn every_pack_call_runs_the_installed_module_in_isolation() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(workspace.path(), GOOD_CAPABILITIES);

    pack::handshake(&env).unwrap();

    let passed = arguments(workspace.path());
    assert_eq!(passed[0], "-I");
    assert_eq!(passed[1], "-m");
    assert_eq!(passed[2], "tremula_python");
    assert_eq!(passed[3], "--capabilities");
}

#[test]
fn a_pack_on_another_contract_version_is_refused_with_both_versions() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(
        workspace.path(),
        &capabilities_with("contract_version", "\"9.9\""),
    );

    let failure = pack::handshake(&env).unwrap_err();

    let complaint = failure.to_string();
    assert!(complaint.contains("9.9"), "{complaint}");
    assert!(complaint.contains(SCHEMA_VERSION), "{complaint}");
    assert!(complaint.contains("tremula-python"), "{complaint}");
}

#[test]
fn a_pack_that_cannot_do_everything_the_core_needs_is_refused_by_name() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(
        workspace.path(),
        &capabilities_with("subcommands", r#"["run"]"#),
    );

    let failure = pack::handshake(&env).unwrap_err();

    let complaint = failure.to_string();
    assert!(complaint.contains("collect"), "{complaint}");
    assert!(complaint.contains("validate"), "{complaint}");
}

/// The handshake asks for the subcommands the core needs, not for the ones the
/// pack happens to have, so a pack that grows a subcommand this core has never
/// heard of stays usable by it.
#[test]
fn a_pack_that_offers_more_subcommands_than_the_core_needs_is_accepted() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(
        workspace.path(),
        &capabilities_with(
            "subcommands",
            r#"["run","collect","validate","spans","a_subcommand_invented_later"]"#,
        ),
    );

    pack::handshake(&env).unwrap();
}

/// A pack that performs no language checks cannot tell a mutant this language
/// can express from one it cannot, which is the pack's whole share of validation.
#[test]
fn a_pack_that_performs_no_language_checks_is_refused() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(
        workspace.path(),
        &capabilities_with("validate_checks", "[]"),
    );

    let failure = pack::handshake(&env).unwrap_err();

    assert!(
        matches!(failure, PackError::NoLanguageChecks { .. }),
        "{failure}"
    );
}

#[test]
fn a_handshake_that_is_not_a_capabilities_document_is_refused() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(workspace.path(), "this is not a document");

    let failure = pack::handshake(&env).unwrap_err();

    assert!(
        matches!(failure, PackError::UnreadableHandshake { .. }),
        "{failure}"
    );
}

#[test]
fn an_interpreter_that_will_not_start_is_reported_as_such() {
    let workspace = TempDir::new().unwrap();

    let failure = pack::handshake(&PythonEnv::at(workspace.path().join("nothing"))).unwrap_err();

    assert!(matches!(failure, PackError::NotStarted { .. }), "{failure}");
}

/// A run directory and a manifest, for the `run` calls below.
struct Request {
    project: PathBuf,
    manifest: PathBuf,
    run_dir: PathBuf,
}

fn request(workspace: &Path) -> Request {
    let project = workspace.join("project");
    let run_dir = workspace.join("runs").join("20260809T041500Z-3b1f8c");
    fs::create_dir_all(run_dir.join("logs")).unwrap();
    fs::create_dir_all(&project).unwrap();
    let manifest = workspace.join("manifest.json");
    fs::write(&manifest, "{}").unwrap();
    Request {
        project,
        manifest,
        run_dir,
    }
}

fn run(env: &PythonEnv, request: &Request) -> Result<(), PackError> {
    run_watched(env, request, &|_| {})
}

/// A run that reports the pack's process to `watch`, the way one holding the
/// project's lock does.
fn run_watched(
    env: &PythonEnv,
    request: &Request,
    watch: &dyn Fn(Option<u32>),
) -> Result<(), PackError> {
    pack::run(
        env,
        &pack::RunRequest {
            manifest: &request.manifest,
            project_root: &request.project,
            run_dir: &request.run_dir,
            tests: &["tests/unit".to_owned()],
            timeout: Some(12.5),
            verbose: false,
        },
        watch,
    )
}

#[test]
fn a_run_passes_the_manifest_the_project_and_its_limits_as_absolute_paths() {
    let workspace = TempDir::new().unwrap();
    let env = fake_pack(workspace.path(), "exit 0\n");
    let asked = request(workspace.path());

    run(&env, &asked).unwrap();

    let passed = arguments(workspace.path());
    assert!(passed.contains(&"run".to_owned()), "{passed:?}");
    for value in [&asked.manifest, &asked.project, &asked.run_dir] {
        let spelling = value.display().to_string();
        assert!(
            passed.contains(&spelling),
            "{spelling} missing from {passed:?}"
        );
        assert!(value.is_absolute(), "{spelling}");
    }
    assert!(passed.contains(&"tests/unit".to_owned()), "{passed:?}");
    assert!(passed.contains(&"12.5".to_owned()), "{passed:?}");
}

/// The pack diagnoses its own failures, and the core says what the diagnosis
/// means in the reader's terms while keeping the pack's own code.
#[test]
fn a_pack_that_diagnoses_its_own_failure_is_reported_in_the_readers_terms() {
    let workspace = TempDir::new().unwrap();
    let env = fake_pack(
        workspace.path(),
        "echo 'running the suite'\necho '{\"error\":{\"stage\":\"baseline\",\"code\":\"baseline_failed\",\"message\":\"3 tests failed before any mutation was applied\"}}'\nexit 2\n",
    );
    let asked = request(workspace.path());

    let failure = run(&env, &asked).unwrap_err();

    let PackError::Reported {
        stage, ref code, ..
    } = failure
    else {
        panic!("{failure}");
    };
    assert_eq!(stage, Stage::Baseline);
    assert_eq!(code, "baseline_failed");
    let complaint = failure.to_string();
    assert!(complaint.contains("your test suite failed"), "{complaint}");
    assert!(complaint.contains("3 tests failed"), "{complaint}");
    assert!(complaint.contains("baseline_failed"), "{complaint}");
}

/// Only the last line is the document. Anything a pack printed before it is
/// diagnostics, even when it looks exactly like a diagnosis.
#[test]
fn only_the_last_line_counts_as_the_diagnosis() {
    let workspace = TempDir::new().unwrap();
    let env = fake_pack(
        workspace.path(),
        "echo '{\"error\":{\"stage\":\"preflight\",\"code\":\"decoy\",\"message\":\"not the diagnosis\"}}'\necho '{\"error\":{\"stage\":\"collect\",\"code\":\"unreadable_session\",\"message\":\"the session could not be read\"}}'\nexit 2\n",
    );
    let asked = request(workspace.path());

    let failure = run(&env, &asked).unwrap_err();

    let PackError::Reported {
        stage, ref code, ..
    } = failure
    else {
        panic!("{failure}");
    };
    assert_eq!(stage, Stage::Collect);
    assert_eq!(code, "unreadable_session");
}

#[test]
fn a_pack_that_fails_without_a_diagnosis_is_reported_with_what_it_did_say() {
    let workspace = TempDir::new().unwrap();
    let env = fake_pack(
        workspace.path(),
        "echo 'Traceback (most recent call last):'\necho 'MemoryError'\nexit 1\n",
    );
    let asked = request(workspace.path());

    let failure = run(&env, &asked).unwrap_err();

    let complaint = failure.to_string();
    assert!(matches!(failure, PackError::Crashed { .. }), "{complaint}");
    assert!(complaint.contains("MemoryError"), "{complaint}");
    assert!(
        complaint.contains(&asked.run_dir.display().to_string()),
        "{complaint}"
    );
}

/// Whatever the pack said is kept, because the run directory is what a reader is
/// pointed at when something went wrong.
#[test]
fn what_the_pack_said_is_kept_in_the_runs_log() {
    let workspace = TempDir::new().unwrap();
    let env = fake_pack(workspace.path(), "echo 'planning 2 mutants'\nexit 0\n");
    let asked = request(workspace.path());

    run(&env, &asked).unwrap();

    let log = fs::read_to_string(asked.run_dir.join("logs").join("pack.txt")).unwrap();
    assert!(log.contains("planning 2 mutants"), "{log}");
}

#[test]
fn deep_validation_asks_the_pack_to_validate_and_nothing_more() {
    let workspace = TempDir::new().unwrap();
    let env = fake_pack(workspace.path(), "exit 0\n");
    let asked = request(workspace.path());

    pack::validate_deep(&env, &asked.manifest, &asked.project).unwrap();

    let passed = arguments(workspace.path());
    assert!(passed.contains(&"validate".to_owned()), "{passed:?}");
    assert!(!passed.contains(&"run".to_owned()), "{passed:?}");
}

/// Dropping the guard is what every way out of driving a pack goes through, so a
/// process it is still responsible for must be stopped — and collected, or it would
/// linger as one nobody ever waits for.
#[test]
fn a_pack_process_the_guard_still_holds_is_stopped_when_it_is_dropped() {
    let mut sleeping = std::process::Command::new("/bin/sleep");
    let pack = Reaped::new(sleeping.arg("30").spawn().unwrap());
    let pid = pack.pid().unwrap();

    drop(pack);

    assert!(!is_alive(pid), "process {pid} outlived the guard");
}

/// A pack that ends on its own is reported while it runs and unreported once it is
/// gone, which is what a lock naming it depends on.
#[test]
fn a_run_reports_the_packs_process_while_it_lasts_and_no_longer() {
    let workspace = TempDir::new().unwrap();
    let env = fake_pack(workspace.path(), "exit 0\n");
    let asked = request(workspace.path());
    let seen = Mutex::new(Vec::new());

    run_watched(&env, &asked, &|pack| {
        seen.lock().unwrap().push(pack);
    })
    .unwrap();

    let reported = seen.lock().unwrap().clone();
    assert_eq!(reported.len(), 2, "{reported:?}");
    assert!(reported[0].is_some(), "{reported:?}");
    assert_eq!(reported[1], None, "{reported:?}");
}

/// Whether a process is still there. A collected one is gone outright rather than
/// left as a zombie, which signalling would still find.
fn is_alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    rustix::process::test_kill_process(pid).is_ok()
}

/// The interpreter of the repository's own environment, or nothing when there is
/// none to use.
fn installed_pack() -> Option<PythonEnv> {
    let interpreter = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".venv")
        .join("bin")
        .join("python");
    if interpreter.is_file() {
        return Some(PythonEnv::at(interpreter));
    }
    eprintln!(
        "skipped: no virtual environment at {}",
        interpreter.display()
    );
    None
}

/// Triage asks for two calls beyond what a run needs, and asks before it pays a
/// model for anything. The installed pack offers both.
#[test]
fn the_repositorys_pack_offers_what_triage_needs() {
    let Some(env) = installed_pack() else { return };

    let capabilities = pack::handshake_for_triage(&env).unwrap();

    assert!(capabilities.subcommands.iter().any(|had| had == "probe"));
}

/// A pack that cannot run a witness is refused before a survivor costs a call.
#[test]
fn a_pack_that_cannot_run_a_witness_is_refused_for_triage() {
    let workspace = TempDir::new().unwrap();
    let env = pack_answering(
        workspace.path(),
        &capabilities_with("subcommands", r#"["run", "collect", "validate", "spans"]"#),
    );

    let failure = pack::handshake_for_triage(&env).unwrap_err();

    assert!(
        failure.to_string().contains("probe"),
        "the refusal has to name the call the pack is missing: {failure}"
    );
}

/// The one call whose answer is an observation rather than a claim, made against
/// the real pack over a real file.
///
/// Two cases and not one, because a probe that could only report a difference
/// would be a probe that reported one whatever it saw.
#[test]
fn the_repositorys_pack_runs_a_witness_against_both_versions() {
    let Some(env) = installed_pack() else { return };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests")
        .join("fixtures")
        .join("generated_manifest");
    let source = fs::read_to_string(root.join("ranges.py")).unwrap();
    let start = u64::try_from(source.find("start < other_end").unwrap()).unwrap();
    let span = Span {
        start_byte: start,
        end_byte: start + u64::try_from("start < other_end".len()).unwrap(),
    };

    let separating = pack::probe(
        &env,
        &pack::ProbeRequest {
            root: &root,
            file: "ranges.py",
            span,
            replacement: "start <= other_end",
            call: "overlaps(30, 60, 0, 30)",
        },
    )
    .unwrap();
    assert_eq!(separating.outcome, ProbeOutcome::Differs);
    assert_eq!(
        separating
            .original
            .as_ref()
            .and_then(|side| side.value.as_deref()),
        Some("False")
    );
    assert_eq!(
        separating
            .mutant
            .as_ref()
            .and_then(|side| side.value.as_deref()),
        Some("True")
    );

    let fabricated = pack::probe(
        &env,
        &pack::ProbeRequest {
            root: &root,
            file: "ranges.py",
            span,
            replacement: "start <= other_end",
            call: "overlaps(0, 30, 40, 60)",
        },
    )
    .unwrap();
    assert_eq!(fabricated.outcome, ProbeOutcome::Indistinguishable);
}

/// The one case with a real pack: the values in the contract's own example are
/// the values the installed pack reports.
#[test]
fn the_repositorys_pack_answers_the_handshake() {
    let Some(env) = installed_pack() else { return };

    let capabilities = pack::handshake(&env).unwrap();

    assert_eq!(capabilities.name, "tremula-python");
    assert_eq!(capabilities.contract_version, SCHEMA_VERSION);
    assert_eq!(
        capabilities.subcommands,
        ["run", "collect", "validate", "spans", "probe"]
    );
    assert_eq!(
        capabilities.validate_checks,
        [
            "compiles_in_file",
            "single_statement",
            "round_trips",
            "span_matches_node",
            "ast_equal"
        ]
    );
}
