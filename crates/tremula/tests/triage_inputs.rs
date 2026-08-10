//! What a triage is allowed to read, and what it refuses to read as one run's.
//!
//! Triage joins three documents of one run — the manifest, the report, and the bytes
//! the run kept — and every way they can disagree is a way for a survivor's evidence
//! to be about something other than what it says. So each disagreement is a test, and
//! each fails as the mismatch it is rather than as a confusing classification later.
//!
//! The decisions a project has recorded are read here too, because what makes one of
//! them stale is a question about a file and the files are what this module reads.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod triage_fixture;

use serde_json::json;
use tremula::{
    python_env::PythonEnv,
    suppressions::Dismissals,
    triage::{
        self,
        failures::TriageFailure,
        inputs::{self, Survivor},
    },
};
use tremula_contracts::suppressions::{DismissalReason, Suppression};

use triage_fixture::{
    FILE, RunFixture, SOURCE, manifest, one_survivor, report, span_of, two_survivors,
};

/// One decision about the fixture's file, recorded the way a person's would be.
fn decided(original: &str, replacement: &str) -> Dismissals {
    let mut dismissals = Dismissals::default();
    assert!(dismissals.add(Suppression {
        file: FILE.to_owned(),
        original: original.to_owned(),
        replacement: replacement.to_owned(),
        reason: DismissalReason::NotUseful,
        dismissed_at: "2026-08-10T09:12:00Z".to_owned(),
        note: None,
        mutant_id: None,
    }));
    dismissals
}

/// What makes a decision stale is the *file* no longer holding its text.
///
/// A triage that counted the text a *function* no longer holds would warn about every
/// decision recorded against a sibling function — and warn about it again for every
/// survivor of the file, since it asked the question once per survivor. Neither the
/// count nor the number of times it is said is a small matter: a warning that is
/// wrong and repeated is one a reader learns to scroll past, and the real one goes
/// with it.
#[test]
fn a_decision_about_another_function_of_the_file_is_not_stale_and_a_vanished_one_is_said_once() {
    let fixture = RunFixture::of(&two_survivors());
    let env = fixture.pack(&[]);

    let read = inputs::read(&env, &fixture.run_dir()).unwrap();

    assert_eq!(read.survivors.len(), 2, "two survivors of the one file");
    assert_eq!(read.sources.len(), 1, "which is read once, whole");
    assert_eq!(read.sources[0].file, FILE);
    assert_eq!(
        read.sources[0].text, SOURCE,
        "the run's own bytes, not the function the mutation lands in"
    );

    // A decision about `overlaps`, which is still in the file and is not in the
    // function either survivor lands in.
    let elsewhere = decided("other_start < end", "other_start <= end");
    assert!(
        triage::stale_warnings(&elsewhere, &read.sources).is_empty(),
        "a decision about another function of the file names text the file still has"
    );
    assert_eq!(
        elsewhere.stale_for(FILE, &read.survivors[1].function),
        1,
        "and counting it against one function is what used to call it stale"
    );

    // A decision naming text no function of the file holds: stale, said once for the
    // file rather than once for each of its survivors.
    let vanished = decided("minutes >= 90", "minutes > 90");
    let warnings = triage::stale_warnings(&vanished, &read.sources);
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert!(warnings[0].contains(FILE), "{}", warnings[0]);
    assert!(
        warnings[0].contains("nothing was dropped"),
        "{}",
        warnings[0]
    );
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
