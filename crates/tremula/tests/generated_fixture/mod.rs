//! The purpose-built project a generation is measured against, and the answer a
//! provider really sent about it.
//!
//! The recording is why the fixture exists rather than the other way round: its
//! `ranges.py` spells `overlaps` the way the recording's prompt shows it, and its
//! suite is the one the recording was shown. Replaying the answer instead of asking
//! for one costs nothing and says the same thing every time.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
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
use tremula_contracts::report::Report;

/// The fixture the recording is about.
pub const PROJECT: &str = "generated_manifest";

/// The model the recording names, which is what its answer reports.
pub const MODEL: &str = "gpt-5.2-2025-12-11";

/// A file where a manifest would go, belonging to whoever put it there.
pub const SOMEBODY_ELSES: &str = "{ \"mine\": true }\n";

/// The one expression in `overlaps` every recorded mutation is about.
pub const OVERLAPPING_CONDITION: &str = "start < other_end and other_start < end";

/// A generator that replays an answer a provider really sent.
///
/// One transformation is applied, and it is the same one the adapter's own tests
/// apply: the recording is the measurement's, the measurement asked for byte
/// offsets, and every mutation in it carries a `span` this version's schema does
/// not name and this version's reader refuses. Taking it out makes the recording
/// what a provider sends when it is asked what this version asks.
pub struct Replay {
    outcome: GenerationOutcome,
    asked: Mutex<Vec<GenerationRequest>>,
}

impl Replay {
    /// The recorded answer about `overlaps`, without the span nobody asks for.
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
            // The offsets in the recording are also the measurement's, and the
            // fixture has gained a module docstring since: what a test relies on
            // is the span this pipeline finds for itself.
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

    /// An answer whose every mutation names text the function does not hold.
    pub fn about_nothing_that_is_there() -> Self {
        Self {
            outcome: GenerationOutcome {
                mutants: (0..4)
                    .map(|which| GeneratedMutant {
                        file: "ranges.py".to_owned(),
                        original: format!("no_such_text_{which}"),
                        replacement: "False".to_owned(),
                        description: "a mutation of code that is not there".to_owned(),
                    })
                    .collect(),
                model_resolved: MODEL.to_owned(),
                attempts: vec![Attempt::default()],
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

/// The fixture tree copied somewhere a run may mutate it.
pub struct Fixture {
    workspace: TempDir,
}

impl Fixture {
    pub fn copy() -> Self {
        let workspace = TempDir::new().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests")
            .join("fixtures")
            .join(PROJECT);
        copy_tree(&source, &workspace.path().join(PROJECT));
        Self { workspace }
    }

    pub fn project(&self) -> PathBuf {
        self.workspace.path().join(PROJECT)
    }

    /// What a person would type to generate for `overlaps` in this project.
    pub fn asking_about(&self, function: &str) -> GenerateArgs {
        GenerateArgs {
            file: "ranges.py".to_owned(),
            functions: vec![function.to_owned()],
            tests: vec!["test_ranges.py".to_owned()],
            model: MODEL.to_owned(),
            python: Some(interpreter()),
            project: self.project(),
            count: 4,
            out: None,
            suppressions: None,
        }
    }

    pub fn manifest(&self) -> PathBuf {
        self.project().join(command::DEFAULT_MANIFEST)
    }

    /// Run the manifest that was just written, the way a person would.
    pub fn run(&self) -> std::process::Output {
        Command::cargo_bin("tremula")
            .unwrap()
            .current_dir(self.workspace.path())
            .arg("run")
            .arg("--manifest")
            .arg(self.manifest())
            .arg("--project")
            .arg(PROJECT)
            .arg("--python")
            .arg(interpreter())
            .output()
            .unwrap()
    }

    pub fn report(&self) -> Report {
        let latest =
            fs::canonicalize(self.project().join(".tremula").join("runs").join("latest")).unwrap();
        serde_json::from_str(&fs::read_to_string(latest.join("report.json")).unwrap()).unwrap()
    }
}

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
