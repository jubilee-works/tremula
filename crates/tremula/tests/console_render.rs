//! Snapshots of the console report. The console is a view, so its shape is
//! pinned the same way the documents are: whole-output goldens rather than
//! assertions about individual lines.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::PathBuf};

use serde::de::DeserializeOwned;
use tremula::{
    bundle::Written,
    console::{render, render_bundle},
    decision::DECISION_RULES_VERSION,
    report::build_report,
    triage::filtered_manifest::PublicationSummary,
};
use tremula_contracts::{
    SCHEMA_VERSION,
    baseline::Baseline,
    bundle::BundleIndex,
    manifest::{Base, Language, Manifest, Mutant, Span},
    report::{Report, RunMeta},
    results::{ExecutionStatus, Location, PackInfo, ResultEntry, Results},
    runner::{ExitClass, RunnerResult},
};

const RUN_ID: &str = "20260807T041500Z-3b1f8c";
const TEST_SET: &str = "b5c7d9e1f3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7";
const FILES: [&str; 3] = ["src/calc.py", "src/overlap.py", "src/pkg/mod.py"];

fn example<T: DeserializeOwned>(relative: &str) -> T {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples")
        .join(relative);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} does not deserialize: {err}", path.display()))
}

/// What a scenario wants a mutant's outcome to be.
#[derive(Clone, Copy)]
enum Outcome {
    Killed,
    KilledByError,
    Survived,
    TimedOut,
    Erroring,
    Skipped,
}

fn passing_runner() -> RunnerResult {
    RunnerResult {
        exit_class: ExitClass::Ok,
        passed: 14,
        failed: 0,
        errors: 0,
        skipped: 0,
        collected: 14,
        collected_ids_hash: TEST_SET.to_owned(),
        collect_error: false,
        timed_out: false,
        duration_ms: 1240,
    }
}

fn signals(outcome: Outcome) -> (ExecutionStatus, Option<RunnerResult>) {
    match outcome {
        Outcome::Killed => (
            ExecutionStatus::Completed,
            Some(RunnerResult {
                exit_class: ExitClass::TestFailures,
                passed: 11,
                failed: 3,
                ..passing_runner()
            }),
        ),
        Outcome::KilledByError => (
            ExecutionStatus::Completed,
            Some(RunnerResult {
                exit_class: ExitClass::TestFailures,
                passed: 12,
                errors: 2,
                ..passing_runner()
            }),
        ),
        Outcome::Survived => (ExecutionStatus::Completed, Some(passing_runner())),
        Outcome::TimedOut => (ExecutionStatus::Timeout, None),
        Outcome::Erroring => (ExecutionStatus::BackendError, None),
        Outcome::Skipped => (ExecutionStatus::Skipped, None),
    }
}

/// Judge one mutant per outcome through the real pipeline, so a snapshot can
/// never show a score its verdicts do not support.
fn judged(outcomes: &[Outcome]) -> (Report, Baseline) {
    let mut mutants = Vec::new();
    let mut entries = Vec::new();
    for (index, outcome) in outcomes.iter().enumerate() {
        // Distinct identifiers whose leading characters differ, since the
        // console shows only the first few.
        let id = format!("{:0<64}", format!("{:x}", 0x3b1f_8c2d + index));
        let start = u64::try_from(index).unwrap_or(0) * 100 + 120;
        mutants.push(Mutant {
            id: id.clone(),
            file: FILES[index % FILES.len()].to_owned(),
            base_file_sha256: TEST_SET.to_owned(),
            span: Span {
                start_byte: start,
                end_byte: start + 38,
            },
            original: "a < b".to_owned(),
            replacement: "a <= b".to_owned(),
            description: None,
            provenance: serde_json::Map::new(),
        });
        let (execution_status, runner) = signals(*outcome);
        entries.push(ResultEntry {
            mutant_id: id,
            execution_status,
            location: Some(Location {
                line: 18,
                column: 11,
            }),
            runner,
            diff: None,
            truncated: false,
            finished_at: None,
            backend_raw: serde_json::Map::new(),
        });
    }
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        language: Language::Python,
        base: Base { revision: None },
        mutants,
    };
    let results = Results {
        schema_version: SCHEMA_VERSION.to_owned(),
        run_id: RUN_ID.to_owned(),
        pack: PackInfo {
            name: "tremula-python".to_owned(),
            version: "0.1.0".to_owned(),
            contract_version: SCHEMA_VERSION.to_owned(),
        },
        entries,
    };
    let baseline = Baseline {
        schema_version: SCHEMA_VERSION.to_owned(),
        run_id: RUN_ID.to_owned(),
        runner: passing_runner(),
    };
    let run = RunMeta {
        run_id: RUN_ID.to_owned(),
        tremula_version: env!("CARGO_PKG_VERSION").to_owned(),
        decision_rules_version: DECISION_RULES_VERSION.to_owned(),
        project: "sample_project".to_owned(),
        tests: Vec::new(),
        observed_revision: None,
        dirty: false,
        started_at: "2026-08-07T04:15:00Z".to_owned(),
        finished_at: "2026-08-07T04:15:04Z".to_owned(),
    };
    let report = build_report(&manifest, &results, &baseline, run).unwrap();
    (report, baseline)
}

#[test]
fn a_survivor_renders_the_reference_report() {
    let report: Report = example("report/survived.json");
    let baseline: Baseline = example("baseline/passing.json");
    insta::assert_snapshot!(render(&report, Some(&baseline)));
}

/// The two commands that follow a run with survivors in it are not two things to choose
/// between. A bundle built before a triage is a bundle with no `triage.json` in it, and a
/// run that offered them side by side left a reader to find that out afterwards.
#[test]
fn what_to_do_after_a_run_with_survivors_says_which_command_comes_first() {
    let (report, baseline) = judged(&[Outcome::Survived, Outcome::Killed]);

    let said = render(&report, Some(&baseline));

    let next = said.lines().last().unwrap_or_default();
    assert!(next.starts_with("next:"), "{said}");
    let triage = next.find("tremula triage").unwrap_or(usize::MAX);
    let bundle = next.find("tremula bundle").unwrap_or(usize::MAX);
    assert!(triage < bundle, "{next}");
    assert!(next.contains("1."), "the order is not spelled out: {next}");
    assert!(next.contains("2."), "the order is not spelled out: {next}");
}

#[test]
fn a_timeout_adds_a_warning_above_the_exit_line() {
    let (report, baseline) = judged(&[Outcome::Killed, Outcome::TimedOut]);
    insta::assert_snapshot!(render(&report, Some(&baseline)));
}

#[test]
fn an_empty_manifest_renders_without_a_baseline() {
    let (report, _) = judged(&[]);
    insta::assert_snapshot!(render(&report, None));
}

/// A kill caused only by errors carries a `killed_by_error` label in the
/// report. The console has no column for it, so the snapshot records what a
/// reader actually sees.
#[test]
fn a_kill_caused_only_by_errors_renders_among_mixed_verdicts() {
    let (report, baseline) =
        judged(&[Outcome::KilledByError, Outcome::Survived, Outcome::Erroring]);
    insta::assert_snapshot!(render(&report, Some(&baseline)));
}

#[test]
fn excluded_mutants_render_their_own_verdicts() {
    let (report, baseline) = judged(&[Outcome::Survived, Outcome::Erroring, Outcome::Skipped]);
    insta::assert_snapshot!(render(&report, Some(&baseline)));
}

#[test]
fn a_filtered_manifest_summary_says_exactly_what_was_kept_and_where() {
    let summary = PublicationSummary {
        path: PathBuf::from("review/without-suspected.json"),
        source_count: 3,
        kept_count: 2,
        excluded_count: 1,
        source_ids: vec![
            "keep-one".to_owned(),
            "exclude".to_owned(),
            "keep-two".to_owned(),
        ],
        kept_ids: vec!["keep-one".to_owned(), "keep-two".to_owned()],
        excluded_ids: vec!["exclude".to_owned()],
    };

    assert_eq!(
        tremula::console::render_filtered_manifest(&summary),
        "filtered manifest: 2/3 kept · 1 suspected equivalent excluded\nmanifest=review/without-suspected.json"
    );
}

/// The reference bundle: everything a reader deciding whether to send the directory
/// somewhere has to see, taken from the committed example so that the console and the
/// contract cannot drift apart.
#[test]
fn a_bundle_renders_what_it_carries_and_what_it_exposes() {
    let index: BundleIndex = example("bundle/minimal.json");
    let written = Written {
        path: PathBuf::from("/somewhere/tremula-bundle-20260810T090000Z-abc123"),
        index,
        unusable: Vec::new(),
    };
    insta::assert_snapshot!(render_bundle(&written));
}

/// The unmutated run's log is one of the files somebody is about to hand over, and it
/// belongs to no mutant. A count taken from the mutants alone is short by one, and says
/// none at all of the bundle whose only log is that one.
#[test]
fn a_bundle_whose_only_log_is_the_unmutated_run_s_still_counts_it() {
    let mut index: BundleIndex = example("bundle/minimal.json");
    for attachment in &mut index.attachments {
        attachment.log = None;
    }
    let written = Written {
        path: PathBuf::from("/somewhere/tremula-bundle-20260810T090000Z-abc123"),
        index,
        unusable: Vec::new(),
    };

    let said = render_bundle(&written);

    assert!(said.contains("logs: 1 file(s)"), "{said}");
}

/// A bundle whose survivor has no patch is not a directory to hand over: the one thing
/// its reader would do with it is the thing it cannot do. Telling somebody to send it
/// anyway sends a package that fails at what it is for.
#[test]
fn a_bundle_that_cannot_reproduce_a_survivor_is_not_offered_for_handing_over() {
    let mut index: BundleIndex = example("bundle/with-triage.json");
    index.exposure.patch_context = true;
    let unusable = vec![index.attachments[1].id.clone()];
    let written = Written {
        path: PathBuf::from("/somewhere/tremula-bundle-20260810T042611Z-3e6e2e"),
        index,
        unusable,
    };

    let said = render_bundle(&written);

    assert!(!said.contains("hand this directory over"), "{said}");
    assert!(said.contains("patch_error"), "{said}");
    assert!(said.contains("results.json"), "{said}");
}

/// The same bundle with a survivor nobody can reproduce from it. The line is an error
/// rather than a note, and the exit code says the bundle is not the one that was asked
/// for even though it was written.
#[test]
fn a_bundle_whose_survivor_has_no_patch_renders_the_refusal() {
    let mut index: BundleIndex = example("bundle/with-triage.json");
    index.exposure.patch_context = true;
    let unusable = vec![index.attachments[1].id.clone()];
    let written = Written {
        path: PathBuf::from("/somewhere/tremula-bundle-20260810T042611Z-3e6e2e"),
        index,
        unusable,
    };
    insta::assert_snapshot!(render_bundle(&written));
}
