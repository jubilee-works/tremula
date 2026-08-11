//! What a run records about where it came from.
//!
//! None of this can be derived from the documents a run judges, and none of it
//! can be observed later: the revision has to be read before the run writes its
//! first artifact, because the run directory lives inside the project and would
//! otherwise report every clean project as modified.

use std::{path::Path, process::Command};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tremula_contracts::report::RunMeta;

use crate::{
    decision::DECISION_RULES_VERSION, generate::prompt::PROMPT_VERSION, run_dir::TREMULA_DIR,
};

/// What a generated mutant names as its producer.
///
/// The pair a pack reports about itself, for the thing on this side of the
/// boundary that produces mutants rather than runs them.
pub const GENERATOR_NAME: &str = "tremula-generate";

/// What the run found when it started, including when that was.
#[derive(Debug)]
pub struct Observed {
    /// The moment the run began, before it had read anything.
    pub started: OffsetDateTime,
    /// The revision the project was at, when it has one.
    pub revision: Option<String>,
    /// Whether the working tree had changes of its own.
    pub dirty: bool,
}

/// Observe the project's revision, if it has one.
///
/// The working tree's own artifacts are excluded from the comparison: the run
/// directory is created inside the project, and counting it would report every
/// clean project as modified.
#[must_use]
pub fn observe(project_root: &Path, started: OffsetDateTime) -> Observed {
    let Some(revision) = git(project_root, &["rev-parse", "HEAD"]) else {
        return Observed {
            started,
            revision: None,
            dirty: false,
        };
    };
    let dirty = git(
        project_root,
        &[
            "status",
            "--porcelain",
            "--",
            ".",
            &format!(":(exclude){TREMULA_DIR}"),
        ],
    )
    .is_some_and(|changes| !changes.is_empty());
    Observed {
        started,
        revision: Some(revision),
        dirty,
    }
}

/// Ask git something about the project, or nothing if it is not a repository.
fn git(project_root: &Path, arguments: &[&str]) -> Option<String> {
    let answer = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(arguments)
        .output()
        .ok()?;
    if !answer.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&answer.stdout).trim().to_owned())
}

/// Everything about the run that cannot be derived from the documents it judged.
///
/// `project` is the root as the caller spelled it, which is what a reader
/// recognises and what makes the console line about it worth printing.
///
/// `tests` are the selectors the run was told to collect, and they are recorded
/// for the same reason the revision is: nothing a run leaves behind can be used
/// to work out how the suite was chosen, and a reader who cannot choose it the
/// same way cannot check a verdict.
///
/// The run is stamped as finished at the moment this is called, so it has to be
/// called once the work is: a reader who compares the two stamps is asking how
/// long the run took.
#[must_use]
pub fn run_meta(run_id: &str, project: &str, tests: &[String], observed: &Observed) -> RunMeta {
    RunMeta {
        run_id: run_id.to_owned(),
        tremula_version: env!("CARGO_PKG_VERSION").to_owned(),
        decision_rules_version: DECISION_RULES_VERSION.to_owned(),
        project: project.to_owned(),
        tests: tests.to_vec(),
        observed_revision: observed.revision.clone(),
        dirty: observed.dirty,
        started_at: timestamp(observed.started),
        finished_at: timestamp(OffsetDateTime::now_utc()),
    }
}

/// What produced one mutant, as its `provenance` records it.
///
/// The keys are the convention `contracts/pack-protocol.md` publishes, so that two
/// producers recording the same fact record it under the same name. Provenance is
/// excluded from a mutant's identifier, so writing more of it here could never
/// change what a mutation is.
///
/// `model` is the model's own account of itself rather than what was asked for: a
/// family name resolves to whichever version is current, which is the one thing a
/// reproduction cannot rely on.
#[must_use]
pub fn generator(model: &str, generated_at: &str) -> serde_json::Map<String, serde_json::Value> {
    let generator = serde_json::json!({
        "name": GENERATOR_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "model": model,
        "prompt_version": PROMPT_VERSION,
        "generated_at": generated_at,
    });
    let mut provenance = serde_json::Map::new();
    provenance.insert("generator".to_owned(), generator);
    provenance
}

/// A moment, as every contract document spells one.
#[must_use]
pub fn timestamp(at: OffsetDateTime) -> String {
    // The description is a fixed standard and every component it needs is one an
    // `OffsetDateTime` always has, so this cannot fail; falling back to the
    // epoch's own spelling keeps that from being a panic.
    at.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
