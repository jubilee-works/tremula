//! A person's decision, recorded once and honoured afterwards.
//!
//! Two halves, and the seam between them is the whole design. Dismissing writes the
//! *mutation* down — the file, the text it replaces, the text it puts there — and
//! never the mutant's identifier, because an identifier is derived from the file's
//! hash and stops naming that mutation the next time anything in the file changes.
//! Honouring it therefore keeps working across the edits that would have orphaned a
//! decision keyed the other way, and that is what these tests hold.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod triage_fixture;

use std::fs;

use serde_json::json;
use tremula::{
    dismiss::{DismissArgs, DismissFailure, Recorded, confirmation, record},
    suppressions::{DEFAULT_SUPPRESSIONS, Dismissals, SuppressionError},
};
use tremula_contracts::suppressions::DismissalReason;

use triage_fixture::{FILE, Mutation, RunFixture, SOURCE};

fn a_run() -> [Mutation; 2] {
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

fn dismissing(fixture: &RunFixture, id: &str, reason: DismissalReason) -> DismissArgs {
    DismissArgs {
        mutant_id: id.to_owned(),
        reason,
        note: None,
        run: Some(fixture.run_dir()),
        project: fixture.project(),
        suppressions: None,
    }
}

#[test]
fn a_dismissal_records_the_mutation_and_keeps_the_identifier_as_provenance() {
    let fixture = RunFixture::of(&a_run());
    let survivor = fixture.ids[1].clone();

    let recorded = record(&dismissing(
        &fixture,
        &survivor,
        DismissalReason::Equivalent,
    ))
    .unwrap();

    let where_it_went = match recorded {
        Recorded::Added(path, mutant) => {
            assert_eq!(mutant.id, survivor);
            path
        }
        Recorded::Already(..) => panic!("nothing was there to be already dismissed"),
    };
    assert_eq!(where_it_went, fixture.project().join(DEFAULT_SUPPRESSIONS));
    let dismissals = Dismissals::load(&where_it_went).unwrap();
    let decision = dismissals
        .covering(FILE, "minutes >= 60", "minutes > 60")
        .expect("the decision is found by the mutation");
    assert_eq!(decision.reason, DismissalReason::Equivalent);
    assert_eq!(decision.mutant_id.as_deref(), Some(survivor.as_str()));
    assert!(!decision.dismissed_at.is_empty());
    // Committed alongside the project, so what is written is a document and not a
    // line: pretty, and ending on a newline.
    let text = fs::read_to_string(&where_it_went).unwrap();
    assert!(text.ends_with("}\n"), "{text}");
    assert!(text.contains("\n  \"suppressions\""), "{text}");
}

#[test]
fn a_leading_part_of_an_identifier_is_enough_to_name_one() {
    let fixture = RunFixture::of(&a_run());
    let short: String = fixture.ids[1].chars().take(8).collect();

    let recorded = record(&dismissing(&fixture, &short, DismissalReason::NotUseful)).unwrap();

    assert!(matches!(recorded, Recorded::Added(..)));
}

#[test]
fn dismissing_the_same_survivor_twice_records_it_once() {
    let fixture = RunFixture::of(&a_run());
    let survivor = fixture.ids[1].clone();
    let asking = dismissing(&fixture, &survivor, DismissalReason::Equivalent);

    record(&asking).unwrap();
    let again = record(&asking).unwrap();

    assert!(matches!(again, Recorded::Already(..)));
    let dismissals = Dismissals::load(&fixture.project().join(DEFAULT_SUPPRESSIONS)).unwrap();
    assert_eq!(dismissals.all().len(), 1);
}

/// Dismissing something twice tells the person what the recorded decision says.
///
/// The second dismissal changes nothing, which means a different reason typed the
/// second time is not the reason on the record. Saying only "already dismissed" leaves
/// finding that out to whoever thinks to open the file.
#[test]
fn dismissing_again_says_the_reason_that_is_already_on_the_record() {
    let fixture = RunFixture::of(&a_run());
    let survivor = fixture.ids[1].clone();

    record(&dismissing(
        &fixture,
        &survivor,
        DismissalReason::Equivalent,
    ))
    .unwrap();
    let again = record(&dismissing(&fixture, &survivor, DismissalReason::NotUseful)).unwrap();

    match &again {
        Recorded::Already(_, _, decision) => {
            assert_eq!(decision.reason, DismissalReason::Equivalent);
            assert!(!decision.dismissed_at.is_empty());
        }
        Recorded::Added(..) => panic!("it was recorded a second time"),
    }
    let said = confirmation(&again);
    assert!(said.contains("already dismissed"), "{said}");
    assert!(said.contains("equivalent"), "{said}");
    assert!(!said.contains("not-useful"), "{said}");
    let dismissals = Dismissals::load(&fixture.project().join(DEFAULT_SUPPRESSIONS)).unwrap();
    assert_eq!(dismissals.all().len(), 1);
    assert_eq!(dismissals.all()[0].reason, DismissalReason::Equivalent);
}

#[test]
fn an_identifier_no_mutant_of_the_run_has_is_refused() {
    let fixture = RunFixture::of(&a_run());

    let failure = record(&dismissing(
        &fixture,
        "0000000000",
        DismissalReason::Equivalent,
    ))
    .unwrap_err();

    match failure {
        DismissFailure::NoSuchMutant { count, .. } => assert_eq!(count, 2),
        other => panic!("{other}"),
    }
    assert!(!fixture.project().join(DEFAULT_SUPPRESSIONS).exists());
}

#[test]
fn an_identifier_that_names_more_than_one_mutant_is_refused() {
    let fixture = RunFixture::of(&a_run());

    let failure = record(&dismissing(&fixture, "", DismissalReason::Equivalent)).unwrap_err();

    assert!(
        matches!(failure, DismissFailure::AmbiguousMutant { count: 2, .. }),
        "{failure}"
    );
}

#[test]
fn a_run_that_is_not_there_is_refused_rather_than_guessed_at() {
    let fixture = RunFixture::of(&a_run());
    let mut asking = dismissing(&fixture, &fixture.ids[1], DismissalReason::Equivalent);
    asking.run = Some(fixture.run_dir().join("nowhere"));

    let failure = record(&asking).unwrap_err();

    assert!(
        matches!(failure, DismissFailure::NoRunDirectory { .. }),
        "{failure}"
    );
}

#[test]
fn a_record_of_decisions_that_cannot_be_read_is_refused_before_anything_is_written() {
    let fixture = RunFixture::of(&a_run());
    let where_it_goes = fixture.project().join(DEFAULT_SUPPRESSIONS);
    fs::write(&where_it_goes, "{ not a document }").unwrap();

    let failure = record(&dismissing(
        &fixture,
        &fixture.ids[1],
        DismissalReason::Equivalent,
    ))
    .unwrap_err();

    assert!(
        matches!(
            failure,
            DismissFailure::Suppressions(SuppressionError::Invalid { .. })
        ),
        "{failure}"
    );
    assert_eq!(
        fs::read_to_string(&where_it_goes).unwrap(),
        "{ not a document }",
        "a file nobody could read is a file nothing was written over"
    );
}

#[test]
fn a_record_from_another_contract_version_is_refused_by_name() {
    let fixture = RunFixture::of(&a_run());
    fs::write(
        fixture.project().join(DEFAULT_SUPPRESSIONS),
        json!({"schema_version": "9.9", "suppressions": []}).to_string(),
    )
    .unwrap();

    let failure = record(&dismissing(
        &fixture,
        &fixture.ids[1],
        DismissalReason::Equivalent,
    ))
    .unwrap_err();

    assert!(failure.to_string().contains("9.9"), "{failure}");
}

/// A project that has dismissed nothing has no such file, which is not an absence
/// worth reporting.
#[test]
fn no_record_at_all_reads_as_no_decisions() {
    let nowhere = std::env::temp_dir().join("tremula-no-such-suppressions-file.json");
    let dismissals = Dismissals::load(&nowhere).unwrap();
    assert!(dismissals.is_empty());
    assert_eq!(dismissals.stale_for(FILE, SOURCE), 0);
}

/// The point of keying by the mutation: an unrelated edit to the file changes every
/// mutant identifier in it, and the decision still applies.
#[test]
fn a_decision_survives_an_edit_that_changes_every_identifier() {
    let fixture = RunFixture::of(&a_run());
    record(&dismissing(
        &fixture,
        &fixture.ids[1],
        DismissalReason::Equivalent,
    ))
    .unwrap();
    let dismissals = Dismissals::load(&fixture.project().join(DEFAULT_SUPPRESSIONS)).unwrap();

    // The same mutation, in a file whose bytes — and so whose every mutant
    // identifier — have moved.
    let edited = SOURCE.replace("Two boundaries", "Two boundaries of ours");
    assert_eq!(dismissals.stale_for(FILE, &edited), 0);
    assert!(
        dismissals
            .covering(FILE, "minutes >= 60", "minutes > 60")
            .is_some()
    );
}

/// A decision whose text is no longer in the file is counted and kept. Dropping it
/// would be deciding, on a person's behalf, that they had changed their mind.
#[test]
fn a_decision_that_no_longer_applies_is_counted_and_not_dropped() {
    let fixture = RunFixture::of(&a_run());
    record(&dismissing(
        &fixture,
        &fixture.ids[1],
        DismissalReason::Equivalent,
    ))
    .unwrap();
    let where_it_is = fixture.project().join(DEFAULT_SUPPRESSIONS);
    let dismissals = Dismissals::load(&where_it_is).unwrap();

    let rewritten = SOURCE.replace("minutes >= 60", "minutes >= BREAK_AFTER");

    assert_eq!(dismissals.stale_for(FILE, &rewritten), 1);
    assert_eq!(
        dismissals.all().len(),
        1,
        "a stale decision is still a decision somebody made"
    );
    assert!(
        fs::read_to_string(&where_it_is)
            .unwrap()
            .contains("minutes >= 60")
    );
}
