//! Letting a person dismiss a survivor once and for all.
//!
//! The one decision in this tool that is a person's and nobody else's. Nothing
//! classifies its way to a dismissal, and nothing dismisses on a model's word: a
//! survivor stops being shown because somebody looked at it and said so.
//!
//! What is recorded is the mutation, taken out of the run's own manifest so that a
//! person only has to name the identifier the console showed them. What is *not*
//! recorded as the key is that identifier — see the suppressions contract for why —
//! though it is kept beside the decision so a reader can find the run it came from.

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use time::OffsetDateTime;
use tremula_contracts::{
    manifest::{Manifest, Mutant},
    suppressions::{DismissalReason, Suppression},
};

use crate::{
    provenance,
    run_dir::TREMULA_DIR,
    suppressions::{DEFAULT_SUPPRESSIONS, Dismissals, SuppressionError},
};

/// What a dismissal exits with when it could not be recorded.
pub const EXIT_FAILURE: u8 = 2;

/// Where run directories live under a project.
const RUNS_DIR: &str = "runs";

/// The link that points at the newest run.
const LATEST: &str = "latest";

/// The copy of the manifest a run keeps, which is where a mutant is looked up.
const MANIFEST: &str = "manifest.json";

/// What `tremula dismiss` was asked to do.
#[derive(Debug, clap::Args)]
pub struct DismissArgs {
    /// The mutant to dismiss, as a run's report or triage names it. A leading part
    /// of the identifier is enough as long as it names only one.
    #[arg(value_name = "MUTANT_ID")]
    pub mutant_id: String,
    /// Why it is being dismissed.
    #[arg(long, value_name = "REASON", value_parser = reason)]
    pub reason: DismissalReason,
    /// Anything worth saying to whoever reads the decision next.
    #[arg(long, value_name = "TEXT")]
    pub note: Option<String>,
    /// The run the mutant belongs to. Defaults to the project's most recent run.
    #[arg(long, value_name = "DIR")]
    pub run: Option<PathBuf>,
    /// Project the decision belongs to.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
    /// Where the decisions are kept. Defaults to `tremula-suppressions.json` in the
    /// project.
    #[arg(long, value_name = "PATH")]
    pub suppressions: Option<PathBuf>,
}

/// A reason, spelled the way a person types it on a command line.
fn reason(spelling: &str) -> Result<DismissalReason, String> {
    match spelling {
        "equivalent" => Ok(DismissalReason::Equivalent),
        "not-useful" | "not_useful" => Ok(DismissalReason::NotUseful),
        other => Err(format!(
            "`{other}` is not a reason to dismiss a survivor; write `equivalent` when it cannot \
             change what the program does, or `not-useful` when it can and a test for that is not \
             one this project wants"
        )),
    }
}

/// Why a dismissal could not be recorded.
#[derive(Debug, thiserror::Error)]
pub enum DismissFailure {
    /// There is no such run to look the mutant up in.
    #[error(
        "no run directory at `{}`; pass --run with the run whose survivor this is, or check `{TREMULA_DIR}/runs`",
        path.display()
    )]
    NoRunDirectory {
        /// The path that was looked for.
        path: PathBuf,
    },
    /// The run's manifest could not be read.
    #[error("cannot read `{}`: {reason}", path.display())]
    Unreadable {
        /// The document that could not be read.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The run's manifest is not a manifest.
    #[error("`{}` is not a valid manifest: {reason}", path.display())]
    Invalid {
        /// The document that could not be read as one.
        path: PathBuf,
        /// Why it could not be.
        reason: String,
    },
    /// No mutant of that run has that identifier.
    #[error(
        "the run in `{}` has no mutant whose identifier starts with `{asked}`; it has {count}, and the report in that directory names every one of them",
        run_dir.display()
    )]
    NoSuchMutant {
        /// The run that was searched.
        run_dir: PathBuf,
        /// What was asked for.
        asked: String,
        /// How many mutants the run had.
        count: usize,
    },
    /// The identifier names more than one mutant of that run.
    #[error(
        "`{asked}` names {count} of the mutants in `{}`, so it does not say which one to dismiss; give more of the identifier",
        run_dir.display()
    )]
    AmbiguousMutant {
        /// The run that was searched.
        run_dir: PathBuf,
        /// What was asked for.
        asked: String,
        /// How many it matched.
        count: usize,
    },
    /// The record of decisions could not be used.
    #[error(transparent)]
    Suppressions(#[from] SuppressionError),
}

/// Record that a person has dismissed one survivor.
#[must_use]
pub fn dismiss(args: &DismissArgs) -> ExitCode {
    match record(args) {
        Ok(recorded) => {
            println!("{}", confirmation(&recorded));
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("error: {failure}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// What a person is told a dismissal came to.
///
/// A dismissal that was already recorded says what the recorded decision was, and not
/// only that there was one. The reason a person has just typed is not the reason kept
/// — the first one is, and nothing here overwrites it — so a message that left the
/// recorded reason out would leave them to work that out by opening the file.
#[must_use]
pub fn confirmation(recorded: &Recorded) -> String {
    match recorded {
        Recorded::Added(where_it_went, mutant) => format!(
            "dismissed {} in {}: {} → {}\nrecorded in {}",
            short(&mutant.id),
            mutant.file,
            one_line(&mutant.original),
            one_line(&mutant.replacement),
            where_it_went.display()
        ),
        Recorded::Already(where_it_is, mutant, decision) => format!(
            "already dismissed: {} in {} is in {}, recorded as {} at {}; nothing was changed, and \
             a decision to record differently has to be edited there",
            short(&mutant.id),
            mutant.file,
            where_it_is.display(),
            worded(decision.reason),
            decision.dismissed_at
        ),
    }
}

/// A reason as the command line spells it, so what is read back is what a person typed.
fn worded(reason: DismissalReason) -> &'static str {
    match reason {
        DismissalReason::Equivalent => "equivalent",
        DismissalReason::NotUseful => "not-useful",
        DismissalReason::Unknown => "a reason this version does not know",
    }
}

/// What recording a dismissal came to.
#[derive(Debug)]
pub enum Recorded {
    /// It was added, to the file named.
    Added(PathBuf, Box<Mutant>),
    /// It was already there, in the file named, as the decision carried here says.
    Already(PathBuf, Box<Mutant>, Box<Suppression>),
}

/// Look the mutant up in its run and write the decision down.
///
/// # Errors
///
/// Returns [`DismissFailure`] when there is no such run, when its manifest cannot be
/// read, when no mutant of it has that identifier or more than one does, or when the
/// record of decisions cannot be read or written.
pub fn record(args: &DismissArgs) -> Result<Recorded, DismissFailure> {
    let run_dir = args
        .run
        .clone()
        .unwrap_or_else(|| args.project.join(TREMULA_DIR).join(RUNS_DIR).join(LATEST));
    if !run_dir.is_dir() {
        return Err(DismissFailure::NoRunDirectory { path: run_dir });
    }
    let mutant = look_up(&run_dir, &args.mutant_id)?;
    let where_it_goes = args
        .suppressions
        .clone()
        .unwrap_or_else(|| args.project.join(DEFAULT_SUPPRESSIONS));
    let mut dismissals = Dismissals::load(&where_it_goes)?;
    // Looked up before anything is written, and the decision itself is carried back:
    // what a second dismissal of one survivor owes the person is what the first one
    // said, since that is the one that stands.
    if let Some(decision) = dismissals.covering(&mutant.file, &mutant.original, &mutant.replacement)
    {
        return Ok(Recorded::Already(
            where_it_goes,
            Box::new(mutant.clone()),
            Box::new(decision.clone()),
        ));
    }
    // Whether it was added is settled above; `add` refusing a duplicate is a guarantee
    // it keeps for its other callers, and here it can only be adding one.
    let added = dismissals.add(Suppression {
        file: mutant.file.clone(),
        original: mutant.original.clone(),
        replacement: mutant.replacement.clone(),
        reason: args.reason,
        dismissed_at: provenance::timestamp(OffsetDateTime::now_utc()),
        note: args.note.clone(),
        mutant_id: Some(mutant.id.clone()),
    });
    debug_assert!(added, "a decision nothing covers is one `add` records");
    dismissals.write(&where_it_goes)?;
    Ok(Recorded::Added(where_it_goes, Box::new(mutant)))
}

/// The one mutant of this run whose identifier starts with what was asked for.
fn look_up(run_dir: &Path, asked: &str) -> Result<Mutant, DismissFailure> {
    let path = run_dir.join(MANIFEST);
    let text = fs::read_to_string(&path).map_err(|err| DismissFailure::Unreadable {
        path: path.clone(),
        reason: err.to_string(),
    })?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|err| DismissFailure::Invalid {
            path: path.clone(),
            reason: err.to_string(),
        })?;
    let matched: Vec<&Mutant> = manifest
        .mutants
        .iter()
        .filter(|mutant| mutant.id.starts_with(asked))
        .collect();
    match matched.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(DismissFailure::NoSuchMutant {
            run_dir: run_dir.to_path_buf(),
            asked: asked.to_owned(),
            count: manifest.mutants.len(),
        }),
        many => Err(DismissFailure::AmbiguousMutant {
            run_dir: run_dir.to_path_buf(),
            asked: asked.to_owned(),
            count: many.len(),
        }),
    }
}

/// As much of an identifier as the console shows.
fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// A stretch of source as one line, so a confirmation stays one line.
fn one_line(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}
