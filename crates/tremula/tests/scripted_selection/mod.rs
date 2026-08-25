//! The repository a selection is measured against, and the answers it is given.
//!
//! One project, four pull requests. Each of them is made the way a person makes one: the base
//! is committed, a branch is taken from it, and the change is committed on the branch — so the
//! comparison a selection makes is a comparison of two real revisions rather than of two
//! strings somebody handed it.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as Process, Output},
    sync::{Mutex, PoisonError},
};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;
use tremula::generate::{
    Attempt, GenerateError, GeneratedMutant, GenerationOutcome, GenerationRequest, MutantGenerator,
    Usage,
    command::{self, GenerateArgs},
};
use tremula_contracts::{manifest::Manifest, report::Report};

/// The fixture tree, which holds the project as the branch leaves it.
pub const PROJECT: &str = "selected_targets";

/// The model the recording names, which is what its answer reports.
pub const MODEL: &str = "gpt-5.2-2025-12-11";

/// The branch the comparison is made against.
pub const BASE: &str = "base";

/// The condition of `overlaps` as the branch leaves it, which is what the recorded answer is
/// about.
pub const CONDITION: &str = "start < other_end and other_start < end";

/// The same condition as the base commit spells it: the same meaning, other words, so the
/// change is a real rewrite of that line and of no other.
const CONDITION_BEFORE: &str = "start < other_end and end > other_start";

/// What `merge` returns as the branch leaves it, and as the base spells it.
const MERGE: &str = "(min(first[0], second[0]), max(first[1], second[1]))";
const MERGE_BEFORE: &str = "(min(second[0], first[0]), max(second[1], first[1]))";

/// The name the decorator of `busy` is given, on the branch and on the base.
const UNDER: &str = r#"@cached("busy-hours")"#;
const UNDER_BEFORE: &str = r#"@cached("busy")"#;

/// The whole of what `busy` returns, which is the one expression a mutation of it can find.
pub const BUSY_BODY: &str = "overlaps(start, end, WORKING_DAY[0], WORKING_DAY[1])";

/// A coverage document in which every line of both files was reached, including the decorator
/// — which is executed when the module is imported, and is why a covered selection cannot see
/// a changed decorator without being told to look for one.
pub const COVERED: &str = concat!(
    "SF:ranges.py\n",
    "DA:1,1\n",
    "DA:4,1\n",
    "DA:6,4\n",
    "DA:9,1\n",
    "DA:11,2\n",
    "end_of_record\n",
    "SF:routes.py\n",
    "DA:9,1\n",
    "DA:11,1\n",
    "DA:13,1\n",
    "DA:16,1\n",
    "DA:19,1\n",
    "DA:20,1\n",
    "DA:28,1\n",
    "DA:31,1\n",
    "DA:32,1\n",
    "DA:34,2\n",
    "end_of_record\n",
);

/// The same document, with the one line this change rewrote reached by nothing.
pub const UNCOVERED: &str = concat!(
    "SF:ranges.py\n",
    "DA:1,1\n",
    "DA:4,1\n",
    "DA:6,0\n",
    "DA:9,1\n",
    "DA:11,2\n",
    "end_of_record\n",
);

/// Which pull request the fixture is.
#[derive(Debug, Clone, Copy)]
pub enum Change {
    /// One function's condition was rewritten. The canonical case.
    Condition,
    /// Two functions were touched, which is what a limit is for.
    TwoFunctions,
    /// Only a decorator was changed — a covered line that no function's span contains.
    Decorator,
}

/// The project, its history, and the pull request under measurement.
pub struct Fixture {
    workspace: TempDir,
}

impl Fixture {
    /// The canonical pull request: one function's condition rewritten.
    pub fn new() -> Self {
        Self::changing(Change::Condition)
    }

    /// The project with `change` committed on a branch taken from the base.
    pub fn changing(change: Change) -> Self {
        let workspace = TempDir::new().unwrap();
        let fixture = Self { workspace };
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests")
            .join("fixtures")
            .join(PROJECT);
        copy_tree(&source, &fixture.project());
        let head = fixture.sources();
        fixture.put(&before(&head, change));
        fixture.git(&["init", "--quiet", "--initial-branch", "main"]);
        fixture.git(&["add", "."]);
        fixture.commit("the base");
        fixture.git(&["branch", BASE]);
        fixture.put(&head);
        fixture.git(&["add", "."]);
        fixture.commit("the change");
        fixture
    }

    pub fn project(&self) -> PathBuf {
        self.workspace.path().join(PROJECT)
    }

    pub fn manifest(&self) -> PathBuf {
        self.project().join(command::DEFAULT_MANIFEST)
    }

    /// The two source files of the project, as they stand.
    fn sources(&self) -> Vec<(String, String)> {
        ["ranges.py", "routes.py"]
            .iter()
            .map(|file| {
                (
                    (*file).to_owned(),
                    fs::read_to_string(self.project().join(file)).unwrap(),
                )
            })
            .collect()
    }

    /// Put those files back the way `sources` describes them.
    fn put(&self, sources: &[(String, String)]) {
        for (file, text) in sources {
            fs::write(self.project().join(file), text).unwrap();
        }
    }

    fn git(&self, arguments: &[&str]) {
        let done = Process::new("git")
            .arg("-C")
            .arg(self.project())
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            done.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&done.stderr)
        );
    }

    fn commit(&self, message: &str) {
        self.git(&[
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "user.name=Fixture",
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "-m",
            message,
        ]);
    }

    /// Write a coverage document into the project and answer with its path.
    pub fn coverage(&self, document: &str) -> PathBuf {
        let path = self.project().join("lcov.info");
        fs::write(&path, document).unwrap();
        path
    }

    /// What a person would type to select from this change. No coverage: the tests that want
    /// some say so, and the one that does not is the degraded case.
    pub fn selecting(&self) -> GenerateArgs {
        GenerateArgs {
            file: None,
            functions: Vec::new(),
            diff_base: Some(BASE.to_owned()),
            coverage: None,
            max_functions: command::DEFAULT_MAX_FUNCTIONS,
            tests: Vec::new(),
            model: MODEL.to_owned(),
            python: Some(interpreter()),
            project: self.project(),
            count: 4,
            out: None,
            suppressions: None,
        }
    }

    pub fn written(&self) -> Manifest {
        Self::read(&self.manifest())
    }

    /// Run the manifest that was just written, the way a person would.
    pub fn run(&self) -> Output {
        Command::cargo_bin("tremula")
            .unwrap()
            .current_dir(self.workspace.path())
            .arg("run")
            .arg("--manifest")
            .arg(self.manifest())
            .arg("--project")
            .arg(self.project())
            .arg("--python")
            .arg(interpreter())
            .output()
            .unwrap()
    }

    /// Package what the run left.
    pub fn bundle(&self) -> Output {
        Command::cargo_bin("tremula")
            .unwrap()
            .current_dir(self.workspace.path())
            .arg("bundle")
            .arg("--project")
            .arg(self.project())
            .arg("--out")
            .arg(self.bundled())
            .output()
            .unwrap()
    }

    pub fn bundled(&self) -> PathBuf {
        self.workspace.path().join("bundle")
    }

    /// The comment body, as the command prints it.
    pub fn comment(&self, extra: &[&str]) -> String {
        let output = Command::cargo_bin("tremula")
            .unwrap()
            .env_remove("GITHUB_TOKEN")
            .env_remove("GITHUB_REPOSITORY")
            .arg("comment")
            .arg("--project")
            .arg(self.project())
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub fn run_dir(&self) -> PathBuf {
        fs::canonicalize(self.project().join(".tremula").join("runs").join("latest")).unwrap()
    }

    pub fn report(&self) -> Report {
        Self::read(&self.run_dir().join("report.json"))
    }

    pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> T {
        let document =
            fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        serde_json::from_str(&document).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }
}

/// The sources as the base commit has them, which is the branch's own change undone.
fn before(head: &[(String, String)], change: Change) -> Vec<(String, String)> {
    head.iter()
        .map(|(file, text)| {
            let undone = match (file.as_str(), change) {
                ("ranges.py", Change::Condition) => text.replace(CONDITION, CONDITION_BEFORE),
                ("ranges.py", Change::TwoFunctions) => text
                    .replace(CONDITION, CONDITION_BEFORE)
                    .replace(MERGE, MERGE_BEFORE),
                ("routes.py", Change::Decorator) => text.replace(UNDER, UNDER_BEFORE),
                _ => text.clone(),
            };
            (file.clone(), undone)
        })
        .collect()
}

/// A generator that replays an answer a provider really sent about `overlaps`.
///
/// The same recording the manual generation is measured against, and for the same reason: the
/// four mutations in it are what a real model proposed, three of them are caught by this suite
/// and one is not, so a run downstream of a selection means something.
pub struct Replay {
    outcome: GenerationOutcome,
    asked: Mutex<Vec<GenerationRequest>>,
}

impl Replay {
    pub fn of_overlaps() -> Self {
        let recorded: Value = serde_json::from_str(include_str!(
            "../fixtures/llm/recorded-response-overlaps.json"
        ))
        .unwrap();
        let said = recorded["response"]["choices"][0]["message"]["content"]
            .as_str()
            .expect("a recorded answer says something");
        let mut answer: Value = serde_json::from_str(said).unwrap();
        for mutant in answer["mutants"].as_array_mut().unwrap() {
            // The offsets in the recording are the measurement's; what this pipeline relies on
            // is the span it finds for itself.
            mutant.as_object_mut().unwrap().remove("span");
        }
        let mutants: Vec<GeneratedMutant> = serde_json::from_value(answer["mutants"].clone())
            .expect("the recorded mutations, once the span is out of them");
        Self {
            outcome: GenerationOutcome {
                mutants,
                model_resolved: recorded["model_resolved"].as_str().unwrap().to_owned(),
                attempts: vec![Attempt {
                    usage: Usage {
                        prompt: recorded["usage"]["prompt_tokens"].as_u64().unwrap(),
                        completion: recorded["usage"]["completion_tokens"].as_u64().unwrap(),
                        total: recorded["usage"]["total_tokens"].as_u64().unwrap(),
                    },
                    finish: Some("stop".to_owned()),
                }],
            },
            asked: Mutex::new(Vec::new()),
        }
    }

    pub fn asked(&self) -> Vec<GenerationRequest> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl MutantGenerator for Replay {
    fn generate(&self, request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(self.outcome.clone())
    }
}

/// A generator that proposes one mutation, for the functions no recording is about.
pub struct Proposing {
    mutants: Vec<GeneratedMutant>,
}

impl Proposing {
    pub fn about(original: &str, replacement: &str) -> Self {
        Self {
            mutants: vec![GeneratedMutant {
                file: "routes.py".to_owned(),
                original: original.to_owned(),
                replacement: replacement.to_owned(),
                description: "a range that misses the end of the working day".to_owned(),
            }],
        }
    }
}

impl MutantGenerator for Proposing {
    fn generate(&self, _request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        Ok(GenerationOutcome {
            mutants: self.mutants.clone(),
            model_resolved: MODEL.to_owned(),
            attempts: vec![Attempt {
                usage: Usage {
                    prompt: 420,
                    completion: 96,
                    total: 516,
                },
                finish: Some("stop".to_owned()),
            }],
        })
    }
}

/// The interpreter the pack is installed in. These tests are gated on a feature precisely
/// because they need one.
pub fn interpreter() -> PathBuf {
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
