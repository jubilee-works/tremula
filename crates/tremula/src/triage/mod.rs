//! Sorting the survivors a run leaves behind.
//!
//! A run answers one question — did the suite catch this? — and a survivor is the
//! answer "no". What it does not say is whether that is a gap in the suite or a
//! mutation that changes nothing, and a reader with fifteen survivors and no order
//! to read them in is a reader who reads none of them.
//!
//! So this asks a second question about each one and writes the answers to a
//! document of its own. `report.json` is never rewritten: it is the record of what
//! the suite did, this is the record of what a second look made of it, and a reader
//! who wants to know which is which has to be able to tell the two documents apart.
//!
//! # What it does not do
//!
//! It does not discard anything. Not one classification here retires a survivor,
//! and the reason is asymmetric: a filter that wrongly drops a real gap in a suite
//! destroys the evidence this tool exists to produce, while a filter that wrongly
//! keeps a harmless mutation costs somebody a minute. So the automatic part
//! establishes what it can, in the order that is most useful to read, and the
//! deciding stays with a person.
//!
//! # Exit code
//!
//! Zero when the judging happened, whatever it decided — including a triage that
//! decided nothing about anything, which is information and not a failure. Two when
//! the judging could not happen: no run to read, documents of two different runs, no
//! key, or every single survivor failing for a reason that was about the tools. This
//! is not a gate, and a project that wants one wants a different command.

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use time::OffsetDateTime;
use tremula_contracts::{
    SCHEMA_VERSION,
    triage::{
        Classification, Dismissed, Judge, Spend, Triage, TriageEntry, TriageRun, TriageScore,
    },
};

use crate::{
    console,
    generate::{
        Attempt,
        judge::{EquivalenceJudge, prompt::PROMPT_VERSION},
        openai::judge::OpenAiJudge,
    },
    pack, provenance, python_env,
    run_dir::TREMULA_DIR,
    suppressions::{DEFAULT_SUPPRESSIONS, Dismissals, warn_about_stale},
    triage::inputs::Survivor,
};

pub mod classify;
pub mod failures;
pub mod inputs;

use failures::TriageFailure;

/// What a triage exits with when it could not be made. The same code every other
/// operational failure in this tool reports.
pub const EXIT_FAILURE: u8 = 2;

/// Where run directories live under a project.
const RUNS_DIR: &str = "runs";

/// The link that points at the newest run.
const LATEST: &str = "latest";

/// Where a triage's own document goes, beside the report it is about.
pub const TRIAGE_DOCUMENT: &str = "triage.json";

/// The statements that go into every document this writes.
///
/// In the document rather than only in a renderer, because the document is what
/// gets pasted into a pull request, and a classification quoted without these reads
/// as a verdict on a mutation rather than as a place to start looking.
const CAVEATS: [&str; 4] = [
    "distinguished_at_function_level = one input separated the two versions of the function; \
     whether the program around it can reach that input is unverified",
    "suspected_equivalent = a model said the mutation changes nothing, and nothing ran to \
     check it",
    "an input that showed no difference is not evidence that no input would",
    "nothing here retires a survivor: every one of them is still for a person to accept or \
     dismiss",
];

/// What `tremula triage` was asked to do.
#[derive(Debug, clap::Args)]
pub struct TriageArgs {
    /// Run to triage. Defaults to the project's most recent run.
    #[arg(long, value_name = "DIR")]
    pub run: Option<PathBuf>,
    /// Project whose environment holds the language pack. The survivors and the
    /// sources they are about come from the run directory, never from here.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
    /// Python interpreter of the project's environment. Discovered if omitted.
    #[arg(long, value_name = "PATH")]
    pub python: Option<PathBuf>,
    /// Model snapshot to ask, spelled the way its provider names that exact
    /// version.
    #[arg(long, value_name = "MODEL")]
    pub model: String,
    /// Where the project's dismissals are kept. Defaults to
    /// `tremula-suppressions.json` in the project.
    #[arg(long, value_name = "PATH")]
    pub suppressions: Option<PathBuf>,
}

/// Judge a run's survivors and write what came of it.
#[must_use]
pub fn triage(args: &TriageArgs) -> ExitCode {
    triage_with(args, &OpenAiJudge::new(&args.model))
}

/// The same, from a judge the caller has already built.
///
/// The seam a test replays recorded answers through, and the seam a second provider
/// would arrive at.
#[must_use]
pub fn triage_with(args: &TriageArgs, judge: &dyn EquivalenceJudge) -> ExitCode {
    match assess(args, judge) {
        Ok(judged) => {
            println!("{}", console::render_triage(&judged));
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("error: {failure}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Ask about every survivor of one run, and write the answers beside its report.
///
/// # Errors
///
/// Returns [`TriageFailure`] when the run cannot be read, when its documents are
/// about different runs, when the pack or the key cannot be used, or when every
/// survivor failed for a reason that was about the tools rather than about a
/// mutation.
pub fn assess(args: &TriageArgs, judge: &dyn EquivalenceJudge) -> Result<Triage, TriageFailure> {
    // Before anything is paid for. A survivor somebody has already dismissed is not
    // asked about again, and a record of decisions that cannot be read is a failure
    // worth having before the first call.
    let dismissals = Dismissals::load(
        &args
            .suppressions
            .clone()
            .unwrap_or_else(|| args.project.join(DEFAULT_SUPPRESSIONS)),
    )?;
    let env = python_env::discover(args.python.as_deref(), &args.project)?;
    env.verify_pack()?;
    // Before a single survivor is paid for: a pack that cannot run a witness would
    // otherwise be discovered one model call in.
    pack::handshake_for_triage(&env)?;
    let read = inputs::read(&env, &where_to_look(args))?;
    let mut entries = Vec::with_capacity(read.survivors.len());
    let mut dismissed = Vec::new();
    let mut attempts: Vec<Attempt> = Vec::new();
    let mut model_resolved = None;
    let mut infrastructure: Vec<String> = Vec::new();
    let mut asked_about = Vec::new();
    for survivor in &read.survivors {
        match already_decided(&dismissals, survivor) {
            Some(decision) => dismissed.push(decision),
            None => asked_about.push(survivor),
        }
    }
    for survivor in asked_about {
        warn_about_stale(&dismissals, &survivor.mutant.file, &survivor.function);
        let assessed = classify::assess(judge, &env, &read.snapshot, survivor)?;
        attempts.extend(assessed.attempts);
        model_resolved = model_resolved.or(assessed.model_resolved);
        if let Some(said) = assessed.infrastructure {
            infrastructure.push(said);
        }
        entries.push(assessed.entry);
    }
    if !entries.is_empty() && infrastructure.len() == entries.len() {
        return Err(TriageFailure::NothingCouldBeJudged {
            survivors: entries.len(),
            last: infrastructure.pop().unwrap_or_default(),
        });
    }
    let judged = Triage {
        schema_version: SCHEMA_VERSION.to_owned(),
        run: TriageRun {
            run_id: read.run_id.clone(),
            report: inputs::REPORT.to_owned(),
            judged_at: provenance::timestamp(OffsetDateTime::now_utc()),
        },
        judge: Judge {
            model: args.model.clone(),
            model_resolved,
            prompt_version: PROMPT_VERSION.to_owned(),
        },
        score: counted(&entries),
        spend: spent(&attempts),
        entries,
        dismissed,
        caveats: CAVEATS.map(str::to_owned).to_vec(),
    };
    write(&read.run_dir.join(TRIAGE_DOCUMENT), &judged)?;
    Ok(judged)
}

/// Which run to read: the one named, or the newest.
fn where_to_look(args: &TriageArgs) -> PathBuf {
    args.run
        .clone()
        .unwrap_or_else(|| args.project.join(TREMULA_DIR).join(RUNS_DIR).join(LATEST))
}

/// The decision covering this survivor, when a person has already made one.
fn already_decided(dismissals: &Dismissals, survivor: &Survivor) -> Option<Dismissed> {
    let mutant = &survivor.mutant;
    let decision = dismissals.covering(&mutant.file, &mutant.original, &mutant.replacement)?;
    Some(Dismissed {
        mutant_id: mutant.id.clone(),
        file: mutant.file.clone(),
        original: mutant.original.clone(),
        replacement: mutant.replacement.clone(),
        reason: decision.reason,
        dismissed_at: decision.dismissed_at.clone(),
    })
}

/// The counts, taken from the entries rather than kept alongside them.
fn counted(entries: &[TriageEntry]) -> TriageScore {
    let count = |wanted: Classification| {
        u32::try_from(
            entries
                .iter()
                .filter(|entry| entry.classification == wanted)
                .count(),
        )
        .unwrap_or(u32::MAX)
    };
    TriageScore {
        survivors: u32::try_from(entries.len()).unwrap_or(u32::MAX),
        distinguished_at_function_level: count(Classification::DistinguishedAtFunctionLevel),
        suspected_equivalent: count(Classification::SuspectedEquivalent),
        undecided: count(Classification::Undecided) + count(Classification::Unknown),
    }
}

/// What every call cost, the failed ones included.
fn spent(attempts: &[Attempt]) -> Spend {
    let mut spend = Spend {
        calls: u32::try_from(attempts.len()).unwrap_or(u32::MAX),
        ..Spend::default()
    };
    for attempt in attempts {
        spend.prompt_tokens += attempt.usage.prompt;
        spend.completion_tokens += attempt.usage.completion;
        spend.total_tokens += attempt.usage.total;
    }
    spend
}

/// Put the document where machines will look for it, by renaming a finished file
/// over it, so a reader never finds half of one.
fn write(path: &Path, judged: &Triage) -> Result<(), TriageFailure> {
    let unwritable = |reason: String| TriageFailure::Unwritable {
        path: path.to_path_buf(),
        reason,
    };
    let mut document =
        serde_json::to_string_pretty(judged).map_err(|err| unwritable(err.to_string()))?;
    document.push('\n');
    let staging = path.with_extension("json.part");
    fs::write(&staging, document).map_err(|err| unwritable(err.to_string()))?;
    fs::rename(&staging, path).map_err(|err| unwritable(err.to_string()))
}
