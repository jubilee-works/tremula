//! The labelled mutations, the run directory they are triaged in, and the judge that
//! replays what a model said about them.
//!
//! `tests/fixtures/labelled-mutations/` holds six frozen modules, every mutation of
//! them a model proposed and a person then labelled one at a time, and one recorded
//! answer per mutation. Everything here is the machinery for putting that fixture
//! through the real triage; what the measurement then comes to is asserted next door,
//! in `judge_accuracy.rs`.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula::{
    generate::{
        Attempt, Usage,
        judge::{EquivalenceJudge, JudgeError, Judgement, JudgementOutcome, JudgementRequest},
        openai::judge::OpenAiJudge,
    },
    triage::TriageArgs,
    validation::canonical_mutant_id,
};
use tremula_contracts::manifest::Span;

/// The model the recordings were taken against, and the only one they stand for.
pub const MODEL: &str = "gpt-5.2-2025-12-11";

/// The run the fixture's directory is named after.
pub const RUN_ID: &str = "20260810T120000Z-labels";

/// How many attempts one judgement may cost at worst.
///
/// The first, one more after a provider says to slow down, and one more to correct an
/// answer that did not fit the schema. All three are booked before the judgement is
/// asked for, because the ceiling has to hold against what a judgement *may* cost: one
/// that booked a single call and then made three would pass the ceiling before anything
/// noticed, which is exactly the failure a ceiling exists to prevent.
pub const ATTEMPTS_PER_JUDGEMENT: usize = 3;

/// How many calls the measurement may ever make.
///
/// Forty judgements at [`ATTEMPTS_PER_JUDGEMENT`] attempts each, which is the fixture
/// with room for a mutation or two more. A ceiling that a legitimate recording of the
/// fixture could trip would be a ceiling somebody raises in a hurry, so it is set where
/// the arithmetic puts it and the fixture is held to fitting inside it.
pub const MAX_CALLS: usize = 120;

/// How many tokens it may ever consume. The other half of the same ceiling, because a
/// call is not a fixed price.
///
/// The same arithmetic: forty judgements, three attempts each, [`BOOKED_PER_CALL`]
/// tokens booked for every one of them, which comes to 960,000. What that is in money
/// is the number worth knowing before setting the recording variable: a million tokens
/// costs a dollar where output is priced at a dollar per million and ten dollars where
/// it is priced at ten, so the worst this target can spend — every attempt of every
/// judgement, all of it billed at the output rate — is single-digit dollars. The
/// measurement as it stands spends 37,546 tokens over 38 calls, under four per cent of
/// the ceiling. The ceiling is not a budget for the work; it is a stop on a mistake,
/// before the mistake becomes a bill anybody has to explain.
pub const MAX_TOTAL_TOKENS: u64 = 1_000_000;

/// What one attempt is booked as before it is made: the answer's ceiling, plus room for
/// a prompt. An attempt is charged for what it used, but a budget that only counts what
/// already happened is a budget that can be overrun exactly once.
pub const BOOKED_PER_CALL: u64 = 8_000;

/// What one whole judgement is booked as, worst case, before it is asked for.
pub const BOOKED_PER_JUDGEMENT: u64 = BOOKED_PER_CALL * ATTEMPTS_PER_JUDGEMENT as u64;

/// The variable that asks for the recordings to be taken again.
pub const RECORD_VARIABLE: &str = "TREMULA_JUDGE_RECORD";

/// One labelled mutation of one frozen module.
#[derive(Debug, Clone, Deserialize)]
pub struct Labelled {
    /// The function the mutation lands in.
    pub function: String,
    /// The frozen module it is in.
    pub file: String,
    /// Where in that module's bytes it lands.
    pub span: Span,
    /// The text the span holds.
    pub original: String,
    /// The text put there instead.
    pub replacement: String,
    /// Whether it changes nothing any input the program can reach would show.
    pub equivalent: bool,
    /// The reader's one-word account of the decision.
    pub label: String,
    /// Whether calling the function directly separates the two versions even though
    /// the program around it cannot.
    #[serde(default)]
    pub separable_at_function_level: bool,
}

/// The fixture as a whole.
#[derive(Debug, Deserialize)]
struct Labels {
    mutations: Vec<Labelled>,
}

/// Where the fixture lives.
pub fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests")
        .join("fixtures")
        .join("labelled-mutations")
}

/// Every labelled mutation, in the order the fixture lists them.
pub fn labels() -> Vec<Labelled> {
    let document = fs::read_to_string(fixture().join("labels.json")).unwrap();
    serde_json::from_str::<Labels>(&document).unwrap().mutations
}

/// Where the recorded answers are kept.
pub fn recordings() -> PathBuf {
    fixture().join("judgements")
}

/// A run directory shaped exactly as a real run leaves one, holding every labelled
/// mutation as a survivor.
///
/// Built rather than recorded, because what a run would contribute is the verdicts,
/// and the verdicts are not what is being measured: every one of these mutations is
/// treated as a survivor so that every one of them is judged.
pub struct Golden {
    workspace: TempDir,
}

impl Golden {
    /// Lay the directory out, refusing a fixture whose spans no longer describe its
    /// own sources.
    pub fn of(mutations: &[Labelled]) -> Self {
        let workspace = TempDir::new().unwrap();
        let run = workspace.path().join(RUN_ID);
        let snapshot = run.join("snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        for entry in fs::read_dir(fixture().join("sources")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), snapshot.join(entry.file_name())).unwrap();
        }
        let mut mutants = Vec::new();
        let mut verdicts = Vec::new();
        for mutation in mutations {
            let bytes = fs::read(snapshot.join(&mutation.file)).unwrap();
            let digest = format!("{:x}", Sha256::digest(&bytes));
            // The fixture's spans are offsets into these exact bytes, and a mismatch
            // here would make every judgement about the wrong text.
            let start = usize::try_from(mutation.span.start_byte).unwrap();
            let end = usize::try_from(mutation.span.end_byte).unwrap();
            assert_eq!(
                &bytes[start..end],
                mutation.original.as_bytes(),
                "the fixture's span for `{}` is not where its text is; the sources are frozen and \
                 one of them has been edited",
                mutation.original
            );
            let id = canonical_mutant_id(
                &mutation.file,
                &mutation.span,
                &digest,
                &mutation.replacement,
            );
            mutants.push(json!({
                "id": id,
                "file": mutation.file,
                "base_file_sha256": digest,
                "span": {
                    "start_byte": mutation.span.start_byte,
                    "end_byte": mutation.span.end_byte,
                },
                "original": mutation.original,
                "replacement": mutation.replacement,
            }));
            verdicts.push(json!({
                "mutant_id": id,
                "file": mutation.file,
                "span": {
                    "start_byte": mutation.span.start_byte,
                    "end_byte": mutation.span.end_byte,
                },
                "verdict": "survived",
                "detail": "a labelled mutation, treated as a survivor so that it is judged",
            }));
        }
        write(
            &run.join("manifest.json"),
            &json!({
                "schema_version": "0.1",
                "language": "python",
                "base": {"revision": null},
                "mutants": mutants,
            }),
        );
        let total = verdicts.len();
        write(
            &run.join("report.json"),
            &json!({
                "schema_version": "0.1",
                "run": {
                    "run_id": RUN_ID,
                    "tremula_version": "0.1.0",
                    "decision_rules_version": "1",
                    "project": "labelled-mutations",
                    "dirty": false,
                    "started_at": "2026-08-10T12:00:00Z",
                    "finished_at": "2026-08-10T12:00:10Z",
                },
                "verdicts": verdicts,
                "score": {
                    "total": total, "killed": 0, "timeout": 0, "survived": total,
                    "runtime_error": 0, "not_applied": 0, "not_run": 0, "skipped": 0,
                },
                "exit_code": 1,
                "caveats": ["survived mutants are not proven equivalent"],
            }),
        );
        Self { workspace }
    }

    /// What a triage of that directory is asked for.
    pub fn asking(&self) -> TriageArgs {
        TriageArgs {
            run: Some(self.workspace.path().join(RUN_ID)),
            project: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
            python: Some(interpreter()),
            model: MODEL.to_owned(),
            suppressions: Some(self.workspace.path().join("no-decisions.json")),
        }
    }
}

fn write(path: &Path, document: &serde_json::Value) {
    let mut text = serde_json::to_string_pretty(document).unwrap();
    text.push('\n');
    fs::write(path, text).unwrap();
}

fn interpreter() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".venv")
        .join("bin")
        .join("python")
}

/// One judgement as it was received, kept so it never has to be paid for twice.
#[derive(Debug, Deserialize, Serialize)]
struct Kept {
    /// The mutation it is about, so a recording can be read by a person.
    about: String,
    /// The model that answered, as it named itself.
    model_resolved: String,
    /// What the call consumed.
    usage: Usage,
    /// The answer.
    judgement: Judgement,
}

/// A judge that replays recorded answers, or records them when asked to.
pub struct Recorded {
    directory: PathBuf,
    live: Option<OpenAiJudge>,
    ledger: Mutex<Ledger>,
}

/// What the calls have cost so far, and the ceilings they are held to.
#[derive(Debug, Default)]
struct Ledger {
    calls: usize,
    tokens: u64,
}

impl Recorded {
    /// Replaying, which is what every run of the measurement does.
    pub fn replaying(directory: PathBuf) -> Self {
        Self {
            directory,
            live: None,
            ledger: Mutex::new(Ledger::default()),
        }
    }

    /// Recording, which costs money and is asked for by name.
    pub fn recording(directory: PathBuf) -> Self {
        Self {
            directory,
            live: Some(OpenAiJudge::new(MODEL)),
            ledger: Mutex::new(Ledger::default()),
        }
    }

    /// Where one mutation's answer is kept.
    ///
    /// Named from the mutation rather than from its position, so that adding a
    /// mutation to the fixture does not silently repoint every other recording.
    pub fn kept_at(&self, request: &JudgementRequest) -> PathBuf {
        let mut digest = Sha256::new();
        for part in [&request.file, &request.original, &request.replacement] {
            digest.update(part.as_bytes());
            digest.update([0]);
        }
        let name: String = format!("{:x}", digest.finalize())
            .chars()
            .take(16)
            .collect();
        self.directory.join(format!("{name}.json"))
    }

    /// What was spent, for the record.
    pub fn spent(&self) -> (usize, u64) {
        let ledger = self.ledger.lock().unwrap_or_else(PoisonError::into_inner);
        (ledger.calls, ledger.tokens)
    }

    /// Book one judgement's worst case before a byte of it is sent.
    ///
    /// The whole of what it may cost, not the one attempt it will make if nothing goes
    /// wrong: a ceiling checked against an optimistic booking is a ceiling that can be
    /// passed by the retries of the judgement that reaches it.
    fn book(&self) {
        let mut ledger = self.ledger.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(
            ledger.calls + ATTEMPTS_PER_JUDGEMENT <= MAX_CALLS,
            "the call ceiling of {MAX_CALLS} leaves no room for another judgement's \
             {ATTEMPTS_PER_JUDGEMENT} attempts; nothing further is sent"
        );
        assert!(
            ledger.tokens + BOOKED_PER_JUDGEMENT <= MAX_TOTAL_TOKENS,
            "the token ceiling of {MAX_TOTAL_TOKENS} would be passed; nothing further is sent"
        );
        ledger.calls += ATTEMPTS_PER_JUDGEMENT;
        ledger.tokens += BOOKED_PER_JUDGEMENT;
    }

    /// Put the booking right once the judgement is over, however it ended.
    ///
    /// Both ways it can end, because a judgement that failed still made the attempts it
    /// made and still cost what they cost: reconciling only the answers would leave the
    /// ledger reading whatever the booking guessed for every judgement that went wrong.
    fn settle(&self, attempts: &[Attempt]) {
        let spent: u64 = attempts.iter().map(|attempt| attempt.usage.total).sum();
        let mut ledger = self.ledger.lock().unwrap_or_else(PoisonError::into_inner);
        ledger.calls = ledger.calls.saturating_sub(ATTEMPTS_PER_JUDGEMENT) + attempts.len();
        ledger.tokens = ledger.tokens.saturating_sub(BOOKED_PER_JUDGEMENT) + spent;
    }
}

impl EquivalenceJudge for Recorded {
    fn judge(&self, request: &JudgementRequest) -> Result<JudgementOutcome, JudgeError> {
        let path = self.kept_at(request);
        let Some(live) = self.live.as_ref() else {
            let document = fs::read_to_string(&path).unwrap_or_else(|err| {
                panic!(
                    "no recorded judgement at {}: {err}. Run this target once with \
                     {RECORD_VARIABLE}=1 and a key in the environment to record one.",
                    path.display()
                )
            });
            let kept: Kept = serde_json::from_str(&document).unwrap();
            return Ok(JudgementOutcome {
                judgement: kept.judgement,
                model_resolved: kept.model_resolved,
                attempts: vec![Attempt {
                    usage: kept.usage,
                    finish: Some("stop".to_owned()),
                }],
            });
        };
        // Booked whole before the call, so that a judgement which retries cannot spend
        // its way past the ceiling, and settled afterwards either way it ended.
        self.book();
        let answered = live.judge(request);
        self.settle(match &answered {
            Ok(outcome) => &outcome.attempts,
            Err(failure) => &failure.attempts,
        });
        let outcome = answered?;
        fs::create_dir_all(&self.directory).unwrap();
        let usage = outcome
            .attempts
            .last()
            .map(|attempt| attempt.usage)
            .unwrap_or_default();
        let kept = Kept {
            about: format!(
                "{}: {} → {}",
                request.file, request.original, request.replacement
            ),
            model_resolved: outcome.model_resolved.clone(),
            usage,
            judgement: outcome.judgement.clone(),
        };
        let mut text = serde_json::to_string_pretty(&kept).unwrap();
        text.push('\n');
        fs::write(&path, text).unwrap();
        Ok(outcome)
    }
}
