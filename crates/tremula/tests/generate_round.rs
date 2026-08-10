//! One function's round of generation: ask, check every answer against the file
//! and the project, ask once more with what was wrong, keep what survived.
//!
//! The generator here is scripted and the project's own check is a closure, so
//! that every path can be put in front of the loop without a model or a
//! subprocess. What each path has to do is decided by measurement: a schema
//! failure practically never happens with a schema the provider enforces, so the
//! one retry a round owns is spent on the defects only the file can reveal —
//! text that is not there, text that is there twice, a replacement the file will
//! not compile with.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
};

use tremula::{
    generate::{
        Attempt, Defect, GenerateError, GenerateFailure, GeneratedMutant, GenerationOutcome,
        GenerationRequest, MutantGenerator, Usage,
        enrich::{ORIGINAL_NOT_FOUND, SourceFile, Target},
        round::{Round, RoundFailure},
    },
    pack::PackError,
};
use tremula_contracts::{
    manifest::{Mutant, Span},
    spans::FunctionSpan,
};

const SOURCE: &str = "def overlaps(start, end, other_start, other_end):\n    return start < other_end and other_start < end\n";

const FILE: &str = "ranges.py";

const MODEL: &str = "gpt-5.2-2025-12-11";

/// What a language pack calls a span its backend cannot match.
const NO_NODE: &str = "span_matches_no_node";

fn span_of(text: &str) -> Span {
    let at = SOURCE
        .find(text)
        .unwrap_or_else(|| panic!("the fixture has no `{text}`"));
    Span {
        start_byte: u64::try_from(at).unwrap(),
        end_byte: u64::try_from(at + text.len()).unwrap(),
    }
}

fn source() -> SourceFile {
    SourceFile::new(FILE, SOURCE.as_bytes().to_vec())
}

fn function() -> FunctionSpan {
    let body = span_of("return start < other_end and other_start < end");
    FunctionSpan {
        qualified_name: "overlaps".to_owned(),
        span: Span {
            start_byte: 0,
            end_byte: body.end_byte,
        },
        body_span: body,
        excluded: Vec::new(),
    }
}

fn request(mutant_count: usize) -> GenerationRequest {
    GenerationRequest {
        file: FILE.to_owned(),
        source: SOURCE.to_owned(),
        tests: Vec::new(),
        excluded: Vec::new(),
        mutant_count,
        feedback: None,
    }
}

fn proposal(original: &str, replacement: &str) -> GeneratedMutant {
    GeneratedMutant {
        file: FILE.to_owned(),
        original: original.to_owned(),
        replacement: replacement.to_owned(),
        description: "an off-by-one at a boundary a test might not pin".to_owned(),
    }
}

/// A well-formed answer carrying `mutants`, charged for one attempt, wrapped the
/// way the script holds it — a failure is the other arm of the same type.
#[allow(clippy::unnecessary_wraps)]
fn answers(mutants: Vec<GeneratedMutant>) -> Result<GenerationOutcome, GenerateError> {
    Ok(GenerationOutcome {
        mutants,
        model_resolved: MODEL.to_owned(),
        attempts: vec![Attempt {
            usage: Usage {
                prompt: 682,
                completion: 436,
                total: 1118,
            },
            finish: Some("stop".to_owned()),
        }],
    })
}

/// A generator that answers from a script and remembers what it was asked.
struct Scripted {
    answers: Mutex<VecDeque<Result<GenerationOutcome, GenerateError>>>,
    asked: Mutex<Vec<GenerationRequest>>,
}

impl Scripted {
    fn new(answers: Vec<Result<GenerationOutcome, GenerateError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<GenerationRequest> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl MutantGenerator for Scripted {
    fn generate(&self, request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("an ask nobody scripted an answer for")
    }
}

/// What the project's own check was asked about, in order.
#[derive(Default)]
struct Inspected {
    seen: Mutex<Vec<Mutant>>,
}

impl Inspected {
    fn record(&self, mutant: &Mutant) {
        self.held().push(mutant.clone());
    }

    fn seen(&self) -> Vec<Mutant> {
        self.held().clone()
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Mutant>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn refuses(code: &str, detail: &str) -> Defect {
    Defect {
        defect: code.to_owned(),
        detail: detail.to_owned(),
    }
}

#[test]
fn every_proposal_that_lands_is_recorded_and_nothing_is_asked_twice() {
    let generator = Scripted::new(vec![answers(vec![
        proposal("start < other_end", "start <= other_end"),
        proposal("other_start < end", "other_start <= end"),
    ])]);
    let inspected = Inspected::default();

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|mutant| {
            inspected.record(mutant);
            Ok(None)
        },
    }
    .run(&request(2));

    assert_eq!(gathered.mutants.len(), 2);
    assert_eq!(gathered.proposed, 2);
    assert!(gathered.refused.is_empty(), "{:?}", gathered.refused);
    assert_eq!(gathered.duplicates, 0);
    assert!(gathered.failure.is_none());
    assert_eq!(
        generator.asked().len(),
        1,
        "nothing was wrong, so nothing was asked again"
    );
    assert_eq!(inspected.seen().len(), 2);
    assert_eq!(gathered.model_resolved.as_deref(), Some(MODEL));
    assert_eq!(gathered.attempts.len(), 1);
    assert_eq!(gathered.attempts[0].usage.total, 1118);
}

#[test]
fn a_proposal_the_file_does_not_hold_is_asked_again_for_that_slot_alone() {
    let generator = Scripted::new(vec![
        answers(vec![
            proposal("start < other_end", "start <= other_end"),
            proposal("start < other_start", "start <= other_start"),
        ]),
        answers(vec![proposal("other_start < end", "other_start <= end")]),
    ]);

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|_| Ok(None),
    }
    .run(&request(2));

    let asked = generator.asked();
    assert_eq!(asked.len(), 2);
    assert_eq!(
        asked[1].mutant_count, 1,
        "only the slot that failed is asked for again"
    );
    let defects = &asked[1]
        .feedback
        .as_ref()
        .expect("the defects ride back")
        .defects;
    assert_eq!(defects.len(), 1);
    assert_eq!(defects[0].defect, ORIGINAL_NOT_FOUND);
    assert_eq!(gathered.mutants.len(), 2);
    assert_eq!(gathered.proposed, 3);
    assert_eq!(gathered.refused.get(ORIGINAL_NOT_FOUND), Some(&1));
    assert_eq!(gathered.attempts.len(), 2, "both asks are on the ledger");
}

#[test]
fn a_second_answer_that_repeats_a_mutant_already_kept_is_dropped_and_counted() {
    // Two proposals can be the same mutation: the identifier is derived from the
    // span and the replacement, and a manifest carrying it twice is refused
    // outright. Dropping the repeat is what keeps one bad slot from spoiling the
    // whole manifest.
    let repeat = proposal("start < other_end", "start <= other_end");
    let generator = Scripted::new(vec![
        answers(vec![
            repeat.clone(),
            proposal("start < other_start", "False"),
        ]),
        answers(vec![repeat]),
    ]);

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|_| Ok(None),
    }
    .run(&request(2));

    assert_eq!(gathered.mutants.len(), 1);
    assert_eq!(gathered.duplicates, 1);
    assert_eq!(gathered.proposed, 3);
}

#[test]
fn a_span_the_backend_cannot_match_is_tried_once_with_the_newline_it_stops_before() {
    // A compound statement's node ends after the newline that closes it, so a
    // search that stopped before that newline found the right text at the wrong
    // extent. That is the caller's mistake and not the model's, so it is repaired
    // and put back to the pack rather than sent back to the model.
    let statement = "return start < other_end and other_start < end";
    let closes_at = span_of(statement).end_byte;
    let generator = Scripted::new(vec![answers(vec![proposal(statement, "return False")])]);
    let inspected = Inspected::default();

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|mutant| {
            inspected.record(mutant);
            if mutant.span.end_byte == closes_at {
                return Ok(Some(refuses(NO_NODE, "not exactly one syntax node")));
            }
            Ok(None)
        },
    }
    .run(&request(1));

    assert_eq!(generator.asked().len(), 1, "the model was not asked again");
    let seen = inspected.seen();
    assert_eq!(
        seen.len(),
        2,
        "the pack was asked twice: once, then with the newline"
    );
    assert_eq!(seen[1].span.end_byte, closes_at + 1);
    assert_eq!(gathered.mutants.len(), 1);
    assert_eq!(gathered.mutants[0].original, format!("{statement}\n"));
    assert!(gathered.refused.is_empty(), "{:?}", gathered.refused);
    assert_eq!(gathered.model_resolved.as_deref(), Some(MODEL));
}

#[test]
fn a_span_the_newline_does_not_rescue_goes_back_to_the_model() {
    let statement = "return start < other_end and other_start < end";
    let generator = Scripted::new(vec![
        answers(vec![proposal(statement, "return False")]),
        answers(vec![proposal("start < other_end", "start <= other_end")]),
    ]);

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|_| Ok(Some(refuses(NO_NODE, "not exactly one syntax node"))),
    }
    .run(&request(1));

    let asked = generator.asked();
    assert_eq!(asked.len(), 2);
    let defects = &asked[1].feedback.as_ref().unwrap().defects;
    assert_eq!(defects[0].defect, NO_NODE);
    assert_eq!(gathered.refused.get(NO_NODE), Some(&2));
    assert!(gathered.mutants.is_empty());
}

#[test]
fn what_the_project_refuses_is_put_in_front_of_the_model_the_way_the_file_is() {
    // The defects the pack finds — a replacement the file will not compile with, a
    // replacement that is two statements — are as correctable as the ones the
    // search finds, and are corrected the same way.
    let generator = Scripted::new(vec![
        answers(vec![proposal("start < other_end", "start <= ")]),
        answers(vec![proposal("other_start < end", "other_start <= end")]),
    ]);
    let inspected = Inspected::default();

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|mutant| {
            inspected.record(mutant);
            if mutant.replacement.trim_end().ends_with("<=") {
                return Ok(Some(refuses(
                    "replacement_does_not_compile",
                    "ranges.py does not compile with this replacement in it",
                )));
            }
            Ok(None)
        },
    }
    .run(&request(1));

    let asked = generator.asked();
    assert_eq!(asked.len(), 2);
    let defects = &asked[1].feedback.as_ref().unwrap().defects;
    assert_eq!(defects[0].defect, "replacement_does_not_compile");
    assert!(
        defects[0].detail.contains("does not compile"),
        "{}",
        defects[0].detail
    );
    assert_eq!(gathered.mutants.len(), 1);
    assert_eq!(
        gathered.refused.get("replacement_does_not_compile"),
        Some(&1)
    );
}

#[test]
fn a_generator_that_fails_on_the_first_ask_gathers_nothing_and_says_why() {
    let generator = Scripted::new(vec![Err(GenerateError {
        kind: GenerateFailure::Refused {
            reason: "I cannot help with that".to_owned(),
        },
        attempts: vec![Attempt::default()],
    })]);

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|_| Ok(None),
    }
    .run(&request(2));

    assert!(gathered.mutants.is_empty());
    assert_eq!(gathered.proposed, 0);
    assert_eq!(
        gathered.attempts.len(),
        1,
        "a failed call was still charged for"
    );
    let failure = gathered.failure.expect("the round records why it stopped");
    assert!(matches!(failure, RoundFailure::Generator(_)), "{failure}");
    assert!(failure.to_string().contains("refused"), "{failure}");
}

#[test]
fn a_generator_that_fails_when_asked_again_keeps_what_the_first_answer_gave() {
    // Partial output on purpose. The mutants of the first answer are real and
    // checked, and throwing them away because a second call failed would cost the
    // caller work it has already paid for — so they are kept and the failure is
    // reported alongside them.
    let generator = Scripted::new(vec![
        answers(vec![
            proposal("start < other_end", "start <= other_end"),
            proposal("start < other_start", "False"),
        ]),
        Err(GenerateError {
            kind: GenerateFailure::RateLimited { retried: true },
            attempts: vec![Attempt::default(), Attempt::default()],
        }),
    ]);

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|_| Ok(None),
    }
    .run(&request(2));

    assert_eq!(gathered.mutants.len(), 1);
    assert_eq!(gathered.refused.get(ORIGINAL_NOT_FOUND), Some(&1));
    assert_eq!(gathered.attempts.len(), 3, "every attempt of both calls");
    assert!(matches!(gathered.failure, Some(RoundFailure::Generator(_))));
}

#[test]
fn a_check_that_cannot_be_made_stops_the_round_without_blaming_the_model() {
    // A language pack that will not start is not a defect of any mutation, so
    // nothing goes back to the model and nothing is counted as refused.
    let generator = Scripted::new(vec![answers(vec![
        proposal("start < other_end", "start <= other_end"),
        proposal("other_start < end", "other_start <= end"),
    ])]);

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|_| {
            Err(PackError::Unreadable {
                reason: "lost contact".to_owned(),
            })
        },
    }
    .run(&request(2));

    assert!(gathered.mutants.is_empty());
    assert!(gathered.refused.is_empty(), "{:?}", gathered.refused);
    assert_eq!(generator.asked().len(), 1);
    assert!(matches!(gathered.failure, Some(RoundFailure::Pack(_))));
}

#[test]
fn every_mutant_records_the_model_that_answered_for_it() {
    let generator = Scripted::new(vec![answers(vec![proposal(
        "start < other_end",
        "start <= other_end",
    )])]);

    let gathered = Round {
        generator: &generator,
        file: &source(),
        target: &Target::new(&function(), &[function()]),
        inspect: &|_| Ok(None),
    }
    .run(&request(1));

    let generator_keys = gathered.mutants[0].provenance["generator"]
        .as_object()
        .unwrap();
    assert_eq!(generator_keys["model"], serde_json::json!(MODEL));
    // Read from the answer and never from what was asked for: a family name
    // resolves to whichever version is current, and a reproduction cannot rely on
    // that.
    assert_eq!(gathered.model_resolved.as_deref(), Some(MODEL));
    let stamped = generator_keys["generated_at"].as_str().unwrap();
    assert!(
        time::OffsetDateTime::parse(stamped, &time::format_description::well_known::Rfc3339)
            .is_ok(),
        "{stamped} is not the RFC 3339 the convention asks for"
    );
}
