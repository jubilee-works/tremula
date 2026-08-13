//! A run directory to triage, and a pack that answers about it from a script.
//!
//! Triage reads three documents of one run and nothing else, so a fixture for it is
//! a directory holding those three: the manifest that says what each mutation
//! replaces, the snapshot holding the bytes the run was measured against, and the
//! report saying which mutants the suite failed to catch. Every one of them is
//! written here rather than produced by a run, because what these tests are about is
//! the reading and the joining — a real run does the same thing far more slowly and
//! is what the end-to-end test is for.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula::{python_env::PythonEnv, triage::TriageArgs, validation::canonical_mutant_id};
use tremula_contracts::{manifest::Span, triage::Triage};

/// The run every fixture names, spelled the way a run directory is.
pub const RUN_ID: &str = "20260810T090000Z-abc123";

/// The file the fixture's survivors live in.
pub const FILE: &str = "schedule.py";

/// The model a fixture's triage names.
pub const MODEL: &str = "gpt-5.2-2025-12-11";

/// The source the snapshot holds: two functions, so that the enclosing one has to
/// be worked out rather than guessed.
pub const SOURCE: &str = "\"\"\"Two boundaries worth getting right.\"\"\"


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    \"\"\"Whether two half-open minute ranges share a minute.\"\"\"
    return start < other_end and other_start < end


def needs_break(minutes: int) -> bool:
    \"\"\"Whether a meeting that long should have a break scheduled after it.\"\"\"
    return minutes >= 60
";

/// One survivor of `needs_break`, which is the fixture's loose boundary.
pub fn one_survivor() -> [Mutation; 2] {
    [
        Mutation {
            original: "other_start < end",
            replacement: "other_start <= end",
            verdict: "killed",
        },
        Mutation {
            original: "minutes >= 60",
            replacement: "minutes > 60",
            verdict: "survived",
        },
    ]
}

/// Both of the fixture's mutations, surviving, so that one file holds two survivors.
pub fn two_survivors() -> [Mutation; 2] {
    [
        Mutation {
            original: "other_start < end",
            replacement: "other_start <= end",
            verdict: "survived",
        },
        Mutation {
            original: "minutes >= 60",
            replacement: "minutes > 60",
            verdict: "survived",
        },
    ]
}

/// One mutation of the fixture's source, spelled the way a person would.
pub struct Mutation {
    /// The text it replaces, which must occur once in [`SOURCE`].
    pub original: &'static str,
    /// The text it puts there.
    pub replacement: &'static str,
    /// What the run's report says the suite did to it.
    pub verdict: &'static str,
}

/// A run directory, and the pack that answers about it.
pub struct RunFixture {
    workspace: TempDir,
    /// The mutant identifiers, in the order the fixture's mutations were given.
    pub ids: Vec<String>,
}

impl RunFixture {
    /// A run directory holding these mutations and the verdicts they were given.
    pub fn of(mutations: &[Mutation]) -> Self {
        let workspace = TempDir::new().unwrap();
        let run_dir = workspace.path().join("runs").join(RUN_ID);
        fs::create_dir_all(run_dir.join("snapshot")).unwrap();
        fs::write(run_dir.join("snapshot").join(FILE), SOURCE).unwrap();
        let digest = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
        let mut ids = Vec::new();
        let mut mutants = Vec::new();
        let mut verdicts = Vec::new();
        for mutation in mutations {
            let span = span_of(mutation.original);
            let id = canonical_mutant_id(FILE, &span, &digest, mutation.replacement);
            ids.push(id.clone());
            mutants.push(json!({
                "id": id,
                "file": FILE,
                "base_file_sha256": digest,
                "span": {"start_byte": span.start_byte, "end_byte": span.end_byte},
                "original": mutation.original,
                "replacement": mutation.replacement,
            }));
            verdicts.push(json!({
                "mutant_id": id,
                "file": FILE,
                "span": {"start_byte": span.start_byte, "end_byte": span.end_byte},
                "verdict": mutation.verdict,
                "detail": "as the fixture says",
            }));
        }
        write(&run_dir.join("manifest.json"), &manifest(&mutants));
        write(&run_dir.join("report.json"), &report(RUN_ID, &verdicts));
        Self { workspace, ids }
    }

    /// The project the run belongs to, where a decision about it is recorded.
    pub fn project(&self) -> PathBuf {
        self.workspace.path().to_path_buf()
    }

    /// The run directory itself.
    pub fn run_dir(&self) -> PathBuf {
        self.workspace.path().join("runs").join(RUN_ID)
    }

    /// The run's snapshot, which is the root a witness runs under.
    pub fn snapshot(&self) -> PathBuf {
        self.run_dir().join("snapshot")
    }

    /// What the triage wrote, read back.
    pub fn triage(&self) -> Triage {
        let document = fs::read_to_string(self.run_dir().join("triage.json")).unwrap();
        serde_json::from_str(&document).unwrap()
    }

    /// Rewrite one of the run's documents, for the tests about them disagreeing.
    pub fn rewrite(&self, name: &str, document: &Value) {
        write(&self.run_dir().join(name), document);
    }

    /// Take one of the run's documents away.
    pub fn remove(&self, name: &str) {
        fs::remove_dir_all(self.run_dir().join(name))
            .or_else(|_| fs::remove_file(self.run_dir().join(name)))
            .unwrap();
    }

    /// Where the stand-in pack lives once [`Self::pack`] has written it.
    pub fn interpreter(&self) -> PathBuf {
        self.workspace.path().join("pack").join("python")
    }

    /// A pack that answers about this fixture: real spans over the real bytes, and
    /// one scripted probe answer per call, in order.
    pub fn pack(&self, probes: &[Value]) -> PythonEnv {
        let directory = self.workspace.path().join("pack");
        fs::create_dir_all(&directory).unwrap();
        // Compact, on one line: the protocol reserves the *last* line of stdout for
        // the document, so a pretty-printed one would arrive as a closing brace.
        one_line(&directory.join("capabilities.json"), &capabilities());
        one_line(&directory.join("spans.json"), &spans());
        for (which, probe) in probes.iter().enumerate() {
            one_line(&directory.join(format!("probe-{}.json", which + 1)), probe);
        }
        let interpreter = self.interpreter();
        fs::write(&interpreter, script(&directory)).unwrap();
        fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o755)).unwrap();
        PythonEnv::at(interpreter)
    }

    /// What a person would type to triage this run.
    pub fn asking(&self) -> TriageArgs {
        TriageArgs {
            run: Some(self.run_dir()),
            project: self.workspace.path().to_path_buf(),
            python: Some(self.interpreter()),
            model: MODEL.to_owned(),
            suppressions: None,
            exclude_suspected_equivalent: false,
            out_manifest: None,
        }
    }
}

/// A stand-in pack: it answers the handshake, answers about spans from a file, and
/// hands out probe answers in order the way a scripted provider hands out replies.
fn script(directory: &Path) -> String {
    let held = directory.display();
    format!(
        r#"#!/bin/sh
held='{held}'
mode=''
for argument in "$@"; do
  case "$argument" in
    -c) echo '0.1.0'; exit 0;;
    --capabilities) cat "$held/capabilities.json"; exit 0;;
    spans) mode=spans;;
    probe) mode=probe;;
  esac
done
if [ "$mode" = spans ]; then cat "$held/spans.json"; exit 0; fi
if [ "$mode" = probe ]; then
  made=$(cat "$held/counter" 2>/dev/null || echo 0)
  made=$((made + 1))
  echo "$made" > "$held/counter"
  if [ -f "$held/probe-$made.json" ]; then cat "$held/probe-$made.json"; exit 0; fi
  echo '{{"error":{{"stage":"probe","code":"probe_not_started","message":"the fixture scripted no answer for that call"}}}}'
  exit 2
fi
exit 2
"#
    )
}

/// The capabilities of a pack that can do everything triage needs.
fn capabilities() -> Value {
    json!({
        "name": "tremula-python",
        "version": "0.1.0",
        "contract_version": "0.1",
        "subcommands": ["run", "collect", "validate", "spans", "probe"],
        "validate_checks": ["compiles_in_file"],
    })
}

/// Where the fixture's two functions are, over the fixture's own bytes.
fn spans() -> Value {
    json!({
        "schema_version": "0.1",
        "file": FILE,
        "file_sha256": format!("{:x}", Sha256::digest(SOURCE.as_bytes())),
        "functions": [
            function("overlaps", "def overlaps", "return start < other_end and other_start < end"),
            function("needs_break", "def needs_break", "return minutes >= 60"),
        ],
    })
}

/// One function of the fixture, from its `def` to the end of its last statement.
fn function(name: &str, opens: &str, closes: &str) -> Value {
    let start = SOURCE.find(opens).unwrap();
    let end = SOURCE.find(closes).unwrap() + closes.len();
    let body = SOURCE[start..end].find("\"\"\"").unwrap() + start;
    json!({
        "qualified_name": name,
        "span": {"start_byte": start, "end_byte": end},
        "body_span": {"start_byte": body, "end_byte": end},
        "excluded": [],
    })
}

/// A probe answer saying the two versions did different things.
pub fn differs(call: &str, original: &str, mutant: &str) -> Value {
    json!({
        "schema_version": "0.1",
        "file": FILE,
        "call": call,
        "outcome": "differs",
        "original": {"ended": "returned", "value": original, "type_name": "bool", "stdout": ""},
        "mutant": {"ended": "returned", "value": mutant, "type_name": "bool", "stdout": ""},
        "runs_per_side": 3,
    })
}

/// A probe answer saying one input failed to tell them apart.
pub fn indistinguishable(call: &str) -> Value {
    json!({
        "schema_version": "0.1",
        "file": FILE,
        "call": call,
        "outcome": "indistinguishable",
        "original": {"ended": "returned", "value": "True", "type_name": "bool", "stdout": ""},
        "mutant": {"ended": "returned", "value": "True", "type_name": "bool", "stdout": ""},
        "runs_per_side": 3,
    })
}

/// A probe answer saying nothing could be compared, and why.
pub fn undecided(call: &str, why: &str) -> Value {
    json!({
        "schema_version": "0.1",
        "file": FILE,
        "call": call,
        "outcome": "undecided",
        "undecided": why,
        "runs_per_side": 3,
    })
}

/// The manifest of a run, around whatever mutants it had.
pub fn manifest(mutants: &[Value]) -> Value {
    json!({
        "schema_version": "0.1",
        "language": "python",
        "base": {"revision": null},
        "mutants": mutants,
    })
}

/// The report of a run, around whatever verdicts it reached.
pub fn report(run_id: &str, verdicts: &[Value]) -> Value {
    let survived = verdicts
        .iter()
        .filter(|verdict| verdict["verdict"] == json!("survived"))
        .count();
    let killed = verdicts.len() - survived;
    json!({
        "schema_version": "0.1",
        "run": {
            "run_id": run_id,
            "tremula_version": "0.1.0",
            "decision_rules_version": "1",
            "project": "schedule",
            "dirty": false,
            "started_at": "2026-08-10T09:00:00Z",
            "finished_at": "2026-08-10T09:00:10Z",
        },
        "verdicts": verdicts,
        "score": {
            "total": survived + killed,
            "killed": killed,
            "timeout": 0,
            "survived": survived,
            "runtime_error": 0,
            "not_applied": 0,
            "not_run": 0,
            "skipped": 0,
        },
        "exit_code": u8::from(survived > 0),
        "caveats": ["survived mutants are not proven equivalent"],
    })
}

/// The span of one stretch of the fixture's source.
pub fn span_of(original: &str) -> Span {
    let at = SOURCE.find(original).unwrap();
    assert_eq!(
        SOURCE.matches(original).count(),
        1,
        "`{original}` is not unique"
    );
    Span {
        start_byte: u64::try_from(at).unwrap(),
        end_byte: u64::try_from(at + original.len()).unwrap(),
    }
}

fn write(path: &Path, document: &Value) {
    let mut text = serde_json::to_string_pretty(document).unwrap();
    text.push('\n');
    fs::write(path, text).unwrap();
}

/// One document on one line, which is what the pack protocol asks a pack for.
fn one_line(path: &Path, document: &Value) {
    let mut text = serde_json::to_string(document).unwrap();
    text.push('\n');
    fs::write(path, text).unwrap();
}
