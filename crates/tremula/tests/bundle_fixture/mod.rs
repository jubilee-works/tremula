//! A finished run directory to package, written rather than run.
//!
//! A bundle reads one run directory and nothing else, so a fixture for it is a
//! directory holding what a finished run leaves: the four contract documents, the
//! snapshot of the bytes the run measured, and the logs the suite printed. Every one
//! of them is written here instead of produced by a real run, because what these
//! tests are about is the reading, the checking, and the packaging — the real
//! pipeline does the same thing far more slowly, and that is what the end-to-end
//! test is for.
//!
//! The diffs are the exception: they are produced by `git diff`, so that the patches
//! the fixture offers are patches git itself will accept against the snapshot rather
//! than something hand-written that happens to look like one.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula::{bundle::BundleArgs, validation::canonical_mutant_id};
use tremula_contracts::{bundle::BundleIndex, manifest::Span};

/// The moment every fixture's run started, which is the first half of its name. The
/// second half is the fingerprint of the manifest the run ran, so a fixture derives it
/// the way a run does rather than writing one down.
pub const STAMP: &str = "20260810T090000Z";

/// How much of the manifest's hash a run's name carries.
const FINGERPRINT_CHARS: usize = 6;

/// The project every fixture builds, so the name in a bundle is worth asserting.
pub const PROJECT: &str = "sample_project";

/// The file the fixture's mutations are in.
pub const FILE: &str = "schedule.py";

/// The source the snapshot holds: two functions, so a patch has real context.
pub const SOURCE: &str = "\"\"\"Two boundaries worth getting right.\"\"\"


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    \"\"\"Whether two half-open minute ranges share a minute.\"\"\"
    return start < other_end and other_start < end


def needs_break(minutes: int) -> bool:
    \"\"\"Whether a meeting that long should have a break scheduled after it.\"\"\"
    return minutes >= 60
";

/// One mutation of the fixture's source, spelled the way a person would.
pub struct Mutation {
    /// The text it replaces, which must occur once in [`SOURCE`].
    pub original: &'static str,
    /// The text it puts there.
    pub replacement: &'static str,
    /// What the run's report says the suite did to it.
    pub verdict: &'static str,
}

/// One mutation the suite caught and one it did not.
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

/// A finished run, in a project of its own.
pub struct RunFixture {
    workspace: TempDir,
    /// The run's name, which is the moment it started and the fingerprint of its
    /// manifest — derived here the way a run derives it.
    pub run_id: String,
    /// The mutant identifiers, in the order the fixture's mutations were given.
    pub ids: Vec<String>,
}

impl RunFixture {
    /// A run directory holding these mutations, the verdicts they were given, and a
    /// diff for each that git will accept against the snapshot.
    pub fn of(mutations: &[Mutation]) -> Self {
        Self::empty().build(mutations, true)
    }

    /// The same, with no diff recorded for any mutant: what a pack that could not
    /// render one leaves behind.
    pub fn without_diffs(mutations: &[Mutation]) -> Self {
        Self::empty().build(mutations, false)
    }

    /// A workspace with no run in it yet.
    fn empty() -> Self {
        Self {
            workspace: TempDir::new().unwrap(),
            run_id: String::new(),
            ids: Vec::new(),
        }
    }

    /// A run of a manifest with nothing in it: a report, and no other document.
    pub fn nothing_to_test() -> Self {
        let mut fixture = Self::empty();
        // No manifest, so there is no fingerprint to derive one from.
        fixture.run_id = format!("{STAMP}-abc123");
        fs::create_dir_all(fixture.run_dir()).unwrap();
        write(
            &fixture.run_dir().join("report.json"),
            &report(&fixture.run_id, &[]),
        );
        fixture.link_latest();
        fixture
    }

    fn build(mut self, mutations: &[Mutation], with_diffs: bool) -> Self {
        let digest = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
        let mut mutants = Vec::new();
        let mut verdicts = Vec::new();
        let mut entries = Vec::new();
        let mut logs = Vec::new();
        for mutation in mutations {
            let span = span_of(mutation.original);
            let id = canonical_mutant_id(FILE, &span, &digest, mutation.replacement);
            self.ids.push(id.clone());
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
            let mut entry = json!({
                "mutant_id": id,
                "execution_status": "completed",
                "runner": runner(u32::from(mutation.verdict == "killed")),
            });
            if with_diffs {
                entry["diff"] = json!(self.diff_of(mutation));
            }
            entries.push(entry);
            logs.push((
                format!("{id}.txt"),
                format!("the suite said this about {}\n", mutation.original),
            ));
        }
        // The manifest first, because the run's own name is derived from its bytes and
        // everything else is written under that name.
        let document = pretty(&manifest(&mutants));
        self.run_id = format!("{STAMP}-{}", fingerprint_of(document.as_bytes()));
        let run_dir = self.run_dir();
        fs::create_dir_all(run_dir.join("snapshot")).unwrap();
        fs::create_dir_all(run_dir.join("logs")).unwrap();
        fs::write(run_dir.join("snapshot").join(FILE), SOURCE).unwrap();
        fs::write(run_dir.join("logs").join("baseline.txt"), "4 passed\n").unwrap();
        for (name, said) in logs {
            fs::write(run_dir.join("logs").join(name), said).unwrap();
        }
        fs::write(run_dir.join("manifest.json"), &document).unwrap();
        write(
            &run_dir.join("report.json"),
            &report(&self.run_id, &verdicts),
        );
        write(
            &run_dir.join("results.json"),
            &results(&self.run_id, &entries),
        );
        write(&run_dir.join("baseline.json"), &baseline(&self.run_id));
        self.link_latest();
        self
    }

    /// Point `latest` at the run, which is what the default `--run` follows.
    fn link_latest(&self) {
        let runs = self.project().join(".tremula").join("runs");
        drop(fs::remove_file(runs.join("latest")));
        std::os::unix::fs::symlink(&self.run_id, runs.join("latest")).unwrap();
    }

    /// A unified diff of one mutation, made by git so that git will accept it.
    ///
    /// Trimmed to begin at the `---` line, because that is the shape the language pack
    /// writes: whole-line context with `a/` and `b/` prefixes, and no `diff --git`
    /// header. A fixture whose patches were richer than the real ones would prove
    /// something about git rather than about the run.
    fn diff_of(&self, mutation: &Mutation) -> String {
        let root = self.workspace.path().join("diffing");
        let mutated = SOURCE.replacen(mutation.original, mutation.replacement, 1);
        for (side, text) in [("a", SOURCE), ("b", mutated.as_str())] {
            let path = root.join(side).join(FILE);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        // `git diff --no-index` reports a difference by exiting 1, so its status says
        // nothing about whether it worked. `--no-prefix` keeps it from putting its own
        // `a/` in front of the `a/` already in the path it was given.
        let done = Process::new("git")
            .current_dir(&root)
            .args([
                "diff",
                "--no-index",
                "--no-color",
                "--no-prefix",
                "--",
                &format!("a/{FILE}"),
                &format!("b/{FILE}"),
            ])
            .output()
            .unwrap();
        let rendered = String::from_utf8(done.stdout).unwrap();
        let at = rendered
            .find("--- ")
            .unwrap_or_else(|| panic!("git rendered no diff: {rendered}"));
        rendered[at..].to_owned()
    }

    /// The project the run belongs to.
    pub fn project(&self) -> PathBuf {
        self.workspace.path().join(PROJECT)
    }

    /// The run directory itself.
    pub fn run_dir(&self) -> PathBuf {
        self.project()
            .join(".tremula")
            .join("runs")
            .join(&self.run_id)
    }

    /// A path outside the project, for the tests about `--out`.
    pub fn beside(&self, name: &str) -> PathBuf {
        self.workspace.path().join(name)
    }

    /// Where a bundle goes when nobody says.
    pub fn default_out(&self) -> PathBuf {
        self.project()
            .join(format!("tremula-bundle-{}", self.run_id))
    }

    /// Rewrite one of the run's documents, for the tests about them disagreeing.
    pub fn rewrite(&self, name: &str, document: &Value) {
        write(&self.run_dir().join(name), document);
    }

    /// One of the run's documents, read back as JSON to be edited.
    pub fn document(&self, name: &str) -> Value {
        let text = fs::read_to_string(self.run_dir().join(name)).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// Take one of the run's documents away.
    pub fn remove(&self, name: &str) {
        fs::remove_dir_all(self.run_dir().join(name))
            .or_else(|_| fs::remove_file(self.run_dir().join(name)))
            .unwrap();
    }

    /// What a person would type to package this run, following `latest`.
    pub fn asking(&self) -> BundleArgs {
        BundleArgs {
            run: None,
            project: self.project(),
            out: None,
            no_logs: false,
        }
    }

    /// The index of a bundle at `out`, read back.
    #[allow(clippy::unused_self)]
    pub fn index(&self, out: &Path) -> BundleIndex {
        let text = fs::read_to_string(out.join("bundle.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }
}

/// The neutral runner signals of one suite execution.
fn runner(failed: u32) -> Value {
    json!({
        "exit_class": if failed > 0 { "test_failures" } else { "ok" },
        "passed": 4 - failed,
        "failed": failed,
        "errors": 0,
        "skipped": 0,
        "collected": 4,
        "collected_ids_hash": "b5c7d9e1f3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7",
        "collect_error": false,
        "timed_out": false,
        "duration_ms": 1180,
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
            "project": ".",
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

/// The results of a run, around whatever entries it observed.
pub fn results(run_id: &str, entries: &[Value]) -> Value {
    json!({
        "schema_version": "0.1",
        "run_id": run_id,
        "pack": {"name": "tremula-python", "version": "0.1.0", "contract_version": "0.1"},
        "entries": entries,
    })
}

/// The unmutated reference run.
pub fn baseline(run_id: &str) -> Value {
    json!({
        "schema_version": "0.1",
        "run_id": run_id,
        "runner": runner(0),
    })
}

/// A triage of the run, naming the report it is about.
pub fn triage(run_id: &str, report: &str, mutant_id: &str) -> Value {
    json!({
        "schema_version": "0.1",
        "run": {"run_id": run_id, "report": report, "judged_at": "2026-08-10T09:04:12Z"},
        "judge": {"model": "a-model-2026-01-01", "prompt_version": "1"},
        "entries": [{
            "mutant_id": mutant_id,
            "file": FILE,
            "span": {"start_byte": 268, "end_byte": 281},
            "original": "minutes >= 60",
            "replacement": "minutes > 60",
            "classification": "undecided",
            "undecided": "no_witness",
            "detail": "the model named no input a probe could run",
        }],
        "score": {
            "survivors": 1,
            "distinguished_at_function_level": 0,
            "suspected_equivalent": 0,
            "undecided": 1,
        },
        "spend": {"calls": 1, "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        "caveats": ["nothing here retires a survivor"],
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
    fs::write(path, pretty(document)).unwrap();
}

/// One document, spelled the way a run writes it.
fn pretty(document: &Value) -> String {
    let mut text = serde_json::to_string_pretty(document).unwrap();
    text.push('\n');
    text
}

/// The part of a manifest's hash a run's name carries.
pub fn fingerprint_of(document: &[u8]) -> String {
    format!("{:x}", Sha256::digest(document))
        .chars()
        .take(FINGERPRINT_CHARS)
        .collect()
}
