//! `tremula generate` as a person meets it: the arguments it refuses, the
//! functions it cannot pick out, and the manifest it will not write over.
//!
//! The pack here is a script standing in for an interpreter, so that everything a
//! generation decides before it asks a model can be put in front of it without a
//! model and without Python. Every case below fails before a single call is made,
//! which is the point of deciding these things first.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Output};

use assert_cmd::Command;
use sha2::Digest as _;
use tempfile::TempDir;

const SOURCE: &str = "def overlaps(start, end):\n    \"\"\"Whether they share a minute.\"\"\"\n    return start < end\n";

const PACK_VERSION: &str = "0.1.0";

/// A project, and a stand-in interpreter that answers the two calls a generation
/// makes before it asks anything of a model.
struct Workspace {
    root: TempDir,
}

impl Workspace {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let project = root.path().join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("ranges.py"), SOURCE).unwrap();
        Self { root }
    }

    fn project(&self) -> PathBuf {
        self.root.path().join("project")
    }

    fn python(&self) -> PathBuf {
        self.root.path().join("python")
    }

    fn manifest(&self) -> PathBuf {
        self.project().join("tremula-manifest.json")
    }

    /// An interpreter that reports the pack, answers the handshake with `spans`
    /// among its subcommands, and prints `report` when asked where a mutation may
    /// land.
    fn write_pack(&self, report: &str) -> &Self {
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *importlib.metadata*)\n    echo '{PACK_VERSION}'\n    exit 0\n    ;;\n  *--capabilities*)\n    echo '{}'\n    exit 0\n    ;;\n  *spans*)\n    echo '{report}'\n    exit 0\n    ;;\nesac\nexit 2\n",
            capabilities("[\"run\",\"collect\",\"validate\",\"spans\"]")
        );
        fs::write(self.python(), script).unwrap();
        fs::set_permissions(self.python(), fs::Permissions::from_mode(0o755)).unwrap();
        self
    }

    /// The same, for a pack that has never heard of `spans`.
    fn write_pack_without_spans(&self) -> &Self {
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *importlib.metadata*)\n    echo '{PACK_VERSION}'\n    exit 0\n    ;;\n  *--capabilities*)\n    echo '{}'\n    exit 0\n    ;;\nesac\nexit 2\n",
            capabilities("[\"run\",\"collect\",\"validate\"]")
        );
        fs::write(self.python(), script).unwrap();
        fs::set_permissions(self.python(), fs::Permissions::from_mode(0o755)).unwrap();
        self
    }

    /// Run the binary, with no provider key in reach.
    ///
    /// Every case here is meant to fail before a model is wanted, and taking the
    /// key out is what makes that a fact rather than a hope: a case that got
    /// further would spend money on whoever ran the suite.
    fn generate(&self, extra: &[&str]) -> Output {
        let mut command = Command::cargo_bin("tremula").unwrap();
        command
            .env_remove("OPENAI_API_KEY")
            .arg("generate")
            .arg("--file")
            .arg("ranges.py")
            .arg("--project")
            .arg(self.project())
            .arg("--python")
            .arg(self.python())
            .arg("--model")
            .arg("gpt-5.2-2025-12-11")
            .args(extra);
        command.output().unwrap()
    }
}

fn capabilities(subcommands: &str) -> String {
    format!(
        r#"{{"name":"tremula-python","version":"{PACK_VERSION}","contract_version":"0.1","subcommands":{subcommands},"validate_checks":["compiles_in_file"]}}"#
    )
}

/// A spans report over the fixture, whose hash has to be the one the core reads.
fn report(functions: &str) -> String {
    report_about("ranges.py", functions)
}

/// The same report, said to be about whichever file the caller names.
fn report_about(file: &str, functions: &str) -> String {
    let digest = format!("{:x}", sha2::Sha256::digest(SOURCE.as_bytes()));
    format!(
        r#"{{"schema_version":"0.1","file":"{file}","file_sha256":"{digest}","functions":{functions}}}"#
    )
}

/// One function entry, with the span the caller would disambiguate it by.
fn function(name: &str, start: u64, end: u64) -> String {
    format!(
        r#"{{"qualified_name":"{name}","span":{{"start_byte":{start},"end_byte":{end}}},"body_span":{{"start_byte":{start},"end_byte":{end}}}}}"#
    )
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_generation_with_no_function_named_is_refused() {
    // There is no automatic choice of what to mutate here, and picking one would
    // be inventing a policy nobody asked for.
    let workspace = Workspace::new();
    workspace.write_pack(&report("[]"));

    let output = workspace.generate(&[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--function"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_count_of_no_mutants_at_all_is_refused() {
    let workspace = Workspace::new();
    workspace.write_pack(&report("[]"));

    let output = workspace.generate(&["--function", "overlaps", "--count", "0"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("above zero"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_manifest_that_is_already_there_stops_the_generation_before_anything_is_spent() {
    let workspace = Workspace::new();
    workspace.write_pack(&report("[]"));
    fs::write(workspace.manifest(), "{}\n").unwrap();

    let output = workspace.generate(&["--function", "overlaps"]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("already exists"), "{complaint}");
    assert!(complaint.contains("--out"), "{complaint}");
}

#[test]
fn a_file_the_project_does_not_own_is_refused_before_the_environment_is_touched() {
    let workspace = Workspace::new();
    // No pack at all: reaching the interpreter would be a failure of its own, and
    // the message has to be about the file.
    let mut command = Command::cargo_bin("tremula").unwrap();
    let output = command
        .arg("generate")
        .arg("--file")
        .arg("../outside.py")
        .arg("--project")
        .arg(workspace.project())
        .arg("--model")
        .arg("gpt-5.2-2025-12-11")
        .arg("--function")
        .arg("overlaps")
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("relative path inside the project"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_pack_that_cannot_say_where_a_mutation_may_land_is_refused_by_name() {
    // A generation has no way to find that out for itself, and a pack that predates
    // the call still runs a manifest somebody else generated — so it is required
    // here and nowhere else.
    let workspace = Workspace::new();
    workspace.write_pack_without_spans();

    let output = workspace.generate(&["--function", "overlaps"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("spans"), "{}", stderr(&output));
}

#[test]
fn a_report_about_another_file_is_refused_even_when_the_hash_agrees() {
    // Every span in a report is an offset into the bytes of the file the report
    // names. One that names another file describes another file's functions, and
    // agreeing about a hash is not agreeing about that — two files with the same
    // contents hash the same.
    let workspace = Workspace::new();
    workspace.write_pack(&report_about(
        "elsewhere.py",
        &format!("[{}]", function("overlaps", 0, 80)),
    ));

    let output = workspace.generate(&["--function", "overlaps"]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("`ranges.py`"), "{complaint}");
    assert!(complaint.contains("`elsewhere.py`"), "{complaint}");
}

#[test]
fn a_function_the_file_does_not_have_is_refused_with_the_ones_it_does() {
    let workspace = Workspace::new();
    workspace.write_pack(&report(&format!("[{}]", function("overlaps", 0, 80))));

    let output = workspace.generate(&["--function", "overlapping"]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(
        complaint.contains("no function called `overlapping`"),
        "{complaint}"
    );
    assert!(complaint.contains("`overlaps@0:80`"), "{complaint}");
}

#[test]
fn a_name_the_file_spells_twice_asks_for_the_span_of_the_one_that_is_meant() {
    // A qualified name is for a person to read and is never a key: a definition
    // made twice under different conditions reports the same name both times.
    let workspace = Workspace::new();
    let twice = format!(
        "[{},{}]",
        function("overlaps", 0, 40),
        function("overlaps", 41, 80)
    );
    workspace.write_pack(&report(&twice));

    let output = workspace.generate(&["--function", "overlaps"]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("2 times"), "{complaint}");
    assert!(complaint.contains("`overlaps@0:40`"), "{complaint}");
    assert!(complaint.contains("`overlaps@41:80`"), "{complaint}");
}

#[test]
fn a_span_that_is_not_a_span_says_how_to_write_one() {
    let workspace = Workspace::new();
    workspace.write_pack(&report(&format!("[{}]", function("overlaps", 0, 80))));

    let output = workspace.generate(&["--function", "overlaps@nowhere"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("name@START:END"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_disambiguated_name_gets_past_the_choice_and_on_to_the_model() {
    // The proof that the disambiguator works is that the generation stops at the
    // next step instead of this one: with two functions of one name, naming the
    // span of one of them leaves nothing to complain about until a model is wanted
    // — and there is deliberately no key to want one with.
    let workspace = Workspace::new();
    let twice = format!(
        "[{},{}]",
        function("overlaps", 0, 40),
        function("overlaps", 41, 80)
    );
    workspace.write_pack(&report(&twice));

    let output = workspace.generate(&["--function", "overlaps@41:80"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        !stderr(&output).contains("does not say which one"),
        "{}",
        stderr(&output)
    );
    // And the summary says which function did not finish and why, which is what an
    // exit code of 2 over a written manifest would otherwise leave unsaid.
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains("overlaps: stopped —"), "{said}");
    assert!(said.contains("OPENAI_API_KEY"), "{said}");
    assert!(said.contains("partial output: overlaps"), "{said}");
    assert!(said.contains("wrote nothing"), "{said}");
}

#[test]
fn the_help_says_what_generate_means_by_a_test() {
    // `run --tests` hands whatever it is given to the project's test runner. These
    // are files to read and show a model, which is a different thing under the same
    // name, so the help says so.
    let output = Command::cargo_bin("tremula")
        .unwrap()
        .args(["generate", "--help"])
        .output()
        .unwrap();

    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("read and show"), "{help}");
    assert!(help.contains("not checked"), "{help}");
}
