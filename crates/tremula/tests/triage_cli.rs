//! What a triage makes of a run's survivors, and what it refuses to make of them.
//!
//! One test per path to a classification, because the paths are the design: there
//! is exactly one way to be distinguished at the level of the function, and every
//! other answer leaves a survivor for a person. A test that only checked the counts
//! would pass while the paths were crossed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod triage_fixture;

use std::sync::{Mutex, PoisonError};

use serde_json::json;
use tremula::{
    console,
    generate::{
        Attempt, Usage,
        judge::{
            Claim as Answered, EquivalenceJudge, JudgeError, JudgeFailure, Judgement,
            JudgementOutcome, JudgementRequest, Witness,
        },
    },
    python_env::PythonEnv,
    triage::{
        assess,
        failures::TriageFailure,
        inputs::{self, Survivor},
    },
};
use tremula_contracts::{
    suppressions::{DismissalReason, Suppression, Suppressions},
    triage::{Claim, Classification, Undecided},
};

use triage_fixture::{
    FILE, MODEL, Mutation, RunFixture, SOURCE, differs, indistinguishable, manifest, report,
    span_of, undecided,
};

/// A judge that answers from a script, one answer per survivor in order.
struct Scripted {
    answers: Mutex<Vec<Result<Judgement, JudgeFailure>>>,
    asked: Mutex<Vec<JudgementRequest>>,
}

impl Scripted {
    fn of(answers: Vec<Result<Judgement, JudgeFailure>>) -> Self {
        Self {
            answers: Mutex::new(answers),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Every judgement claims a difference and names the same call.
    fn distinguishing(call: &str, times: usize) -> Self {
        Self::of(
            (0..times)
                .map(|_| {
                    Ok(Judgement {
                        claim: Answered::Distinguishable,
                        witness: Some(Witness {
                            call: call.to_owned(),
                            expect_original: "one thing".to_owned(),
                            expect_mutant: "another".to_owned(),
                        }),
                    })
                })
                .collect(),
        )
    }

    fn asked(&self) -> Vec<JudgementRequest> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl EquivalenceJudge for Scripted {
    fn judge(&self, request: &JudgementRequest) -> Result<JudgementOutcome, JudgeError> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let mut answers = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        match answers.remove(0) {
            Ok(judgement) => Ok(JudgementOutcome {
                judgement,
                model_resolved: MODEL.to_owned(),
                attempts: vec![Attempt {
                    usage: Usage {
                        prompt: 300,
                        completion: 40,
                        total: 340,
                    },
                    finish: Some("stop".to_owned()),
                }],
            }),
            Err(kind) => Err(JudgeError {
                kind,
                attempts: vec![Attempt::default()],
            }),
        }
    }
}

/// One survivor of `needs_break`, which is the fixture's loose boundary.
fn one_survivor() -> [Mutation; 2] {
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

/// The witness that really separates the fixture's survivor.
const SEPARATING: &str = "needs_break(60)";

#[test]
fn a_witness_that_separates_the_two_distinguishes_the_survivor_at_function_level() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[differs(SEPARATING, "True", "False")]);
    let judge = Scripted::distinguishing(SEPARATING, 1);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    assert_eq!(judged.entries.len(), 1, "the killed mutant is not triaged");
    let entry = &judged.entries[0];
    assert_eq!(
        entry.classification,
        Classification::DistinguishedAtFunctionLevel
    );
    assert_eq!(entry.undecided, None);
    assert_eq!(entry.claim, Some(Claim::Distinguishable));
    assert_eq!(
        entry.witness.as_ref().map(|w| w.call.as_str()),
        Some(SEPARATING)
    );
    assert!(entry.probe.is_some(), "the evidence is kept with the entry");
    assert!(entry.detail.contains("True"), "{}", entry.detail);
    assert_eq!(judged.score.distinguished_at_function_level, 1);
    assert_eq!(judged.score.survivors, 1);
    // The question was about the function the mutation lands in, not the whole file.
    let asked = judge.asked();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].source.starts_with("def needs_break"),
        "{}",
        asked[0].source
    );
    assert!(!asked[0].source.contains("def overlaps"));
    assert_eq!(asked[0].original, "minutes >= 60");
    assert_eq!(asked[0].file, FILE);
    // Written where a machine will look for it, and it agrees with what was returned.
    assert_eq!(fixture.triage(), judged);
    assert_eq!(judged.run.report, inputs::REPORT);
    assert_eq!(judged.judge.prompt_version, "1");
    assert_eq!(judged.judge.model_resolved.as_deref(), Some(MODEL));
    assert_eq!(judged.spend.calls, 1);
    assert_eq!(judged.spend.total_tokens, 340);
}

#[test]
fn a_claim_of_equivalence_is_a_suspicion_and_nothing_is_run() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[]);
    let judge = Scripted::of(vec![Ok(Judgement {
        claim: Answered::Equivalent,
        witness: None,
    })]);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    let entry = &judged.entries[0];
    assert_eq!(entry.classification, Classification::SuspectedEquivalent);
    assert_eq!(entry.claim, Some(Claim::Equivalent));
    assert!(
        entry.probe.is_none(),
        "nothing was run, so nothing is recorded"
    );
    assert_eq!(judged.score.suspected_equivalent, 1);
}

#[test]
fn a_witness_that_shows_no_difference_is_undecided_rather_than_equivalent() {
    // The measured failure mode: the model claims a difference, writes down an
    // input, and the input does not separate anything. That is not a finding of
    // equivalence, and calling it one would be believing the claim it just failed.
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[indistinguishable("needs_break(120)")]);
    let judge = Scripted::distinguishing("needs_break(120)", 1);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    let entry = &judged.entries[0];
    assert_eq!(entry.classification, Classification::Undecided);
    assert_eq!(entry.undecided, Some(Undecided::WitnessShowedNoDifference));
    assert_eq!(entry.claim, Some(Claim::Distinguishable));
    assert!(
        entry.probe.is_some(),
        "the input that failed is the useful part"
    );
    assert_eq!(judged.score.suspected_equivalent, 0);
    assert_eq!(judged.score.undecided, 1);
}

#[test]
fn a_claim_with_no_witness_is_undecided() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[]);
    let judge = Scripted::of(vec![Ok(Judgement {
        claim: Answered::Distinguishable,
        witness: None,
    })]);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    assert_eq!(judged.entries[0].undecided, Some(Undecided::NoWitness));
}

#[test]
fn every_reason_a_probe_gives_arrives_as_a_reason_of_its_own() {
    for (reported, expected) in [
        ("nondeterministic", Undecided::Nondeterministic),
        ("incomparable", Undecided::Incomparable),
        ("unsafe_witness", Undecided::UnsafeWitness),
        ("method", Undecided::Method),
        ("no_such_function", Undecided::NoSuchFunction),
        ("timed_out", Undecided::TimedOut),
        ("a_reason_invented_later", Undecided::Unknown),
    ] {
        let fixture = RunFixture::of(&one_survivor());
        fixture.pack(&[undecided(SEPARATING, reported)]);
        let judge = Scripted::distinguishing(SEPARATING, 1);

        let judged = assess(&fixture.asking(), &judge).unwrap();

        let entry = &judged.entries[0];
        assert_eq!(entry.classification, Classification::Undecided);
        assert_eq!(entry.undecided, Some(expected), "reported as {reported}");
    }
}

#[test]
fn a_judge_that_cannot_answer_leaves_the_survivor_undecided() {
    let fixture = RunFixture::of(&[
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
    ]);
    fixture.pack(&[differs(SEPARATING, "True", "False")]);
    let judge = Scripted::of(vec![
        Err(JudgeFailure::NoAnswer),
        Ok(Judgement {
            claim: Answered::Distinguishable,
            witness: Some(Witness {
                call: SEPARATING.to_owned(),
                expect_original: "True".to_owned(),
                expect_mutant: "False".to_owned(),
            }),
        }),
    ]);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    assert_eq!(judged.entries[0].undecided, Some(Undecided::NoJudgement));
    assert_eq!(
        judged.entries[1].classification,
        Classification::DistinguishedAtFunctionLevel,
        "one survivor nobody could answer about does not stop the others"
    );
    assert_eq!(judged.spend.calls, 2, "a failed call is on the ledger too");
}

#[test]
fn a_triage_where_nothing_could_be_judged_is_an_operational_failure() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[]);
    let judge = Scripted::of(vec![Err(JudgeFailure::NoAnswer)]);

    let failure = assess(&fixture.asking(), &judge).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::NothingCouldBeJudged { .. }),
        "{failure}"
    );
}

#[test]
fn no_key_stops_the_whole_triage_rather_than_one_survivor() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[]);
    let judge = Scripted::of(vec![Err(JudgeFailure::MissingCredential {
        variable: "OPENAI_API_KEY".to_owned(),
    })]);

    let failure = assess(&fixture.asking(), &judge).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::NoCredential { .. }),
        "{failure}"
    );
    assert!(failure.to_string().contains("OPENAI_API_KEY"));
}

#[test]
fn a_run_with_no_survivors_is_a_triage_of_nothing_and_still_succeeds() {
    let fixture = RunFixture::of(&[Mutation {
        original: "minutes >= 60",
        replacement: "minutes > 60",
        verdict: "killed",
    }]);
    fixture.pack(&[]);
    let judge = Scripted::of(vec![]);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    assert!(judged.entries.is_empty());
    assert_eq!(judged.score.survivors, 0);
    assert_eq!(judged.spend.calls, 0);
}

#[test]
fn the_document_carries_what_a_reader_must_not_misunderstand() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[differs(SEPARATING, "True", "False")]);
    let judge = Scripted::distinguishing(SEPARATING, 1);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    let said = judged.caveats.join("\n");
    assert!(said.contains("unverified"), "{said}");
    assert!(said.contains("nothing ran to check it"), "{said}");
    assert!(said.contains("is not evidence"), "{said}");
    assert!(said.contains("retires"), "{said}");
    // And the console says the same things, since that is what most readers see.
    let rendered = console::render_triage(&judged);
    assert!(
        rendered.contains("distinguished at function level (1)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("minutes >= 60 → minutes > 60"),
        "{rendered}"
    );
    for caveat in &judged.caveats {
        assert!(rendered.contains(caveat), "{rendered}");
    }
}

/// A survivor somebody has already dismissed is not asked about again — which is
/// the whole point of dismissing one — and is still accounted for, so that a reader
/// comparing this document with the run's report finds every survivor in it.
#[test]
fn a_survivor_already_dismissed_is_accounted_for_and_never_asked_about() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[]);
    let where_it_is = fixture.project().join("decisions.json");
    std::fs::write(
        &where_it_is,
        serde_json::to_string(&Suppressions {
            schema_version: "0.1".to_owned(),
            suppressions: vec![Suppression {
                file: FILE.to_owned(),
                original: "minutes >= 60".to_owned(),
                replacement: "minutes > 60".to_owned(),
                reason: DismissalReason::NotUseful,
                dismissed_at: "2026-08-10T09:12:00Z".to_owned(),
                note: None,
                mutant_id: None,
            }],
        })
        .unwrap(),
    )
    .unwrap();
    let mut asking = fixture.asking();
    asking.suppressions = Some(where_it_is);
    let judge = Scripted::of(vec![]);

    let judged = assess(&asking, &judge).unwrap();

    assert!(judged.entries.is_empty(), "nothing was asked about");
    assert_eq!(judged.spend.calls, 0, "and so nothing was paid for");
    assert_eq!(judged.dismissed.len(), 1);
    assert_eq!(judged.dismissed[0].original, "minutes >= 60");
    assert_eq!(judged.dismissed[0].reason, DismissalReason::NotUseful);
    assert_eq!(judged.dismissed[0].mutant_id, fixture.ids[1]);
    assert_eq!(judged.score.survivors, 0);
    let rendered = console::render_triage(&judged);
    assert!(rendered.contains("already dismissed (1)"), "{rendered}");
}

/// A record of decisions that cannot be read is a failure before the first call and
/// never after it: the whole reason it is read this early.
#[test]
fn a_record_of_decisions_that_cannot_be_read_stops_the_triage_before_it_pays() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.pack(&[]);
    let where_it_is = fixture.project().join("decisions.json");
    std::fs::write(&where_it_is, "{ not a document }").unwrap();
    let mut asking = fixture.asking();
    asking.suppressions = Some(where_it_is);
    // A judge that would panic if it were ever asked, which is the assertion.
    let judge = Scripted::of(vec![]);

    let failure = assess(&asking, &judge).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::Suppressions(_)),
        "{failure}"
    );
    assert!(judge.asked().is_empty());
}

/// The report and the manifest are documents of one run, and a directory where
/// they are not is one nothing can be joined in.
#[test]
fn a_directory_whose_report_is_about_another_run_is_refused() {
    let fixture = RunFixture::of(&one_survivor());
    let env = fixture.pack(&[]);
    let verdicts = vec![json!({
        "mutant_id": fixture.ids[1],
        "file": FILE,
        "span": {"start_byte": span_of("minutes >= 60").start_byte, "end_byte": span_of("minutes >= 60").end_byte},
        "verdict": "survived",
        "detail": "as the fixture says",
    })];
    fixture.rewrite("report.json", &report("20260810T090000Z-999999", &verdicts));

    let failure = inputs::read(&env, &fixture.run_dir()).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::AnotherRun { .. }),
        "{failure}"
    );
}

#[test]
fn a_report_giving_a_verdict_on_a_mutant_the_manifest_lacks_is_refused() {
    let fixture = RunFixture::of(&one_survivor());
    let env = fixture.pack(&[]);
    fixture.rewrite("manifest.json", &manifest(&[]));

    let failure = inputs::read(&env, &fixture.run_dir()).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::MutantNotInManifest { .. }),
        "{failure}"
    );
}

#[test]
fn a_run_that_kept_no_snapshot_has_nothing_to_run_a_witness_against() {
    let fixture = RunFixture::of(&one_survivor());
    let env = fixture.pack(&[]);
    fixture.remove("snapshot");

    let failure = inputs::read(&env, &fixture.run_dir()).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::NoSnapshot { .. }),
        "{failure}"
    );
}

#[test]
fn a_snapshot_holding_other_bytes_than_the_run_measured_is_refused() {
    let fixture = RunFixture::of(&one_survivor());
    let env = fixture.pack(&[]);
    std::fs::write(
        fixture.snapshot().join(FILE),
        SOURCE.replace("minutes >= 60", "minutes >= 61"),
    )
    .unwrap();

    let failure = inputs::read(&env, &fixture.run_dir()).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::SnapshotIsAnotherFile { .. }),
        "{failure}"
    );
}

#[test]
fn a_directory_that_is_not_a_run_is_refused_before_anything_is_read() {
    let fixture = RunFixture::of(&one_survivor());
    let env = fixture.pack(&[]);

    let failure = inputs::read(&env, &fixture.run_dir().join("nowhere")).unwrap_err();

    assert!(
        matches!(failure, TriageFailure::NoRunDirectory { .. }),
        "{failure}"
    );
}

#[test]
fn a_report_that_is_not_a_report_says_which_document_it_was() {
    let fixture = RunFixture::of(&one_survivor());
    let env = fixture.pack(&[]);
    fixture.rewrite("report.json", &json!({"not": "a report"}));

    let failure = inputs::read(&env, &fixture.run_dir()).unwrap_err();

    match failure {
        TriageFailure::Invalid { what, .. } => assert_eq!(what, "report"),
        other => panic!("{other}"),
    }
}

/// A survivor of no function at all is not something a model can be asked about,
/// and sending it the whole file would make the answer a judgement about something
/// nobody asked about.
#[test]
fn a_mutation_inside_no_function_is_undecided_without_a_call() {
    let fixture = RunFixture::of(&[Mutation {
        original: "Two boundaries worth getting right.",
        replacement: "Two boundaries.",
        verdict: "survived",
    }]);
    fixture.pack(&[]);
    let judge = Scripted::of(vec![]);

    let judged = assess(&fixture.asking(), &judge).unwrap();

    assert_eq!(judged.entries[0].undecided, Some(Undecided::NoJudgement));
    assert_eq!(
        judged.spend.calls, 0,
        "nothing was asked, so nothing was paid"
    );
}

/// The survivors a triage reads are the ones a report calls survived, joined to the
/// manifest by identifier — and the function each one lands in is worked out from
/// the snapshot rather than from anything in the working tree.
#[test]
fn the_survivors_are_read_out_of_the_runs_own_three_documents() {
    let fixture = RunFixture::of(&one_survivor());
    let env: PythonEnv = fixture.pack(&[]);

    let read = inputs::read(&env, &fixture.run_dir()).unwrap();

    assert_eq!(read.run_id, triage_fixture::RUN_ID);
    assert_eq!(read.mutants, 2);
    let survivors: Vec<&Survivor> = read.survivors.iter().collect();
    assert_eq!(survivors.len(), 1);
    assert_eq!(survivors[0].function_name, "needs_break");
    assert_eq!(survivors[0].mutant.id, fixture.ids[1]);
    assert!(survivors[0].function.contains("return minutes >= 60"));
    assert_eq!(
        read.snapshot,
        std::fs::canonicalize(fixture.snapshot()).unwrap(),
        "the root a witness runs under is the run's own snapshot, resolved"
    );
}
