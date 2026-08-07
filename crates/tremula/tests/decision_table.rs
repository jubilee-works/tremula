//! Table-driven tests for the verdict decision rules: one case per row of the
//! decision table, plus the precedence conflicts between rows.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula::decision::{Judgement, decide};
use tremula_contracts::{
    report::Verdict,
    results::{ExecutionStatus, ResultEntry},
    runner::{ExitClass, RunnerResult},
};

const MUTANT_ID: &str = "3b1f8c2d4e6a8b0c2d4e6f8a0b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b8c";
const BASELINE_TEST_SET: &str = "b5c7d9e1f3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7";
const ANOTHER_TEST_SET: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b";

/// A passing reference run: fourteen tests collected, fourteen passed. Rows
/// below restate only the signals their own rule reads, so each row exercises
/// exactly the condition its guard names.
fn baseline() -> RunnerResult {
    RunnerResult {
        exit_class: ExitClass::Ok,
        passed: 14,
        failed: 0,
        errors: 0,
        skipped: 0,
        collected: 14,
        collected_ids_hash: BASELINE_TEST_SET.to_owned(),
        collect_error: false,
        timed_out: false,
        duration_ms: 1180,
    }
}

fn entry(execution_status: ExecutionStatus, runner: Option<RunnerResult>) -> ResultEntry {
    ResultEntry {
        mutant_id: MUTANT_ID.to_owned(),
        execution_status,
        location: None,
        runner,
        diff: None,
        truncated: false,
        finished_at: None,
        backend_raw: serde_json::Map::new(),
    }
}

/// One row of the decision table.
struct Row {
    what: &'static str,
    entry: ResultEntry,
    verdict: Verdict,
    label: Option<&'static str>,
    detail: &'static str,
}

/// Rows the execution status alone decides, before any runner signal is read.
fn rows_decided_by_execution_status() -> Vec<Row> {
    vec![
        Row {
            what: "a filtered mutant",
            entry: entry(ExecutionStatus::Skipped, None),
            verdict: Verdict::Skipped,
            label: None,
            detail: "skipped",
        },
        Row {
            what: "a mutation the backend never applied",
            entry: entry(ExecutionStatus::NotApplied, None),
            verdict: Verdict::NotApplied,
            label: None,
            detail: "mutation was never applied",
        },
        Row {
            what: "a mutant left behind by an interrupted run",
            entry: entry(ExecutionStatus::NotRun, None),
            verdict: Verdict::NotRun,
            label: None,
            detail: "scheduled but never executed",
        },
        Row {
            what: "a backend that failed on its own",
            entry: entry(ExecutionStatus::BackendError, None),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "backend error",
        },
        Row {
            what: "a suite that exceeded its time limit",
            entry: entry(ExecutionStatus::Timeout, None),
            verdict: Verdict::Timeout,
            label: None,
            detail: "timed out",
        },
        Row {
            what: "a completed attempt whose runner said nothing",
            entry: entry(ExecutionStatus::Completed, None),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "no runner signal",
        },
    ]
}

/// Rows where the suite never got far enough to say anything about the mutant.
fn rows_decided_by_runner_health() -> Vec<Row> {
    vec![
        Row {
            what: "a suite whose collection failed",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    collect_error: true,
                    ..baseline()
                }),
            ),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "collection failed",
        },
        Row {
            what: "a suite that was interrupted",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    exit_class: ExitClass::Interrupted,
                    ..baseline()
                }),
            ),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "run interrupted",
        },
        Row {
            what: "a runner that failed for its own reasons",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    exit_class: ExitClass::InfraError,
                    ..baseline()
                }),
            ),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "runner infrastructure error",
        },
    ]
}

/// Rows the test outcomes decide: the mutant was detected, or it was not.
fn rows_decided_by_test_outcomes() -> Vec<Row> {
    vec![
        Row {
            what: "tests that failed",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    exit_class: ExitClass::TestFailures,
                    passed: 11,
                    failed: 3,
                    ..baseline()
                }),
            ),
            verdict: Verdict::Killed,
            label: None,
            detail: "3 failed",
        },
        Row {
            what: "tests that only errored",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    exit_class: ExitClass::TestFailures,
                    passed: 12,
                    errors: 2,
                    ..baseline()
                }),
            ),
            verdict: Verdict::Killed,
            label: Some("killed_by_error"),
            detail: "2 errored",
        },
        Row {
            what: "tests that both failed and errored",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    exit_class: ExitClass::TestFailures,
                    passed: 11,
                    failed: 2,
                    errors: 1,
                    ..baseline()
                }),
            ),
            verdict: Verdict::Killed,
            label: None,
            detail: "2 failed",
        },
        Row {
            what: "the same suite passing as it did without the mutant",
            entry: entry(ExecutionStatus::Completed, Some(baseline())),
            verdict: Verdict::Survived,
            label: None,
            detail: "14 passed",
        },
    ]
}

/// Rows that look like survival from the outside — nothing failed — while
/// proving nothing about the mutant.
fn rows_with_no_usable_outcome() -> Vec<Row> {
    vec![
        Row {
            what: "a suite that collected nothing",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    exit_class: ExitClass::NoTests,
                    passed: 0,
                    collected: 0,
                    collected_ids_hash: ANOTHER_TEST_SET.to_owned(),
                    ..baseline()
                }),
            ),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "no tests collected",
        },
        Row {
            what: "a suite whose every test was skipped",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    passed: 0,
                    skipped: 14,
                    ..baseline()
                }),
            ),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "all collected tests were skipped",
        },
        Row {
            what: "a suite that collected as many tests but not the same ones",
            entry: entry(
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    collected_ids_hash: ANOTHER_TEST_SET.to_owned(),
                    ..baseline()
                }),
            ),
            verdict: Verdict::RuntimeError,
            label: None,
            detail: "test set changed",
        },
    ]
}

#[test]
fn every_row_of_the_decision_table_judges_as_documented() {
    let rows = rows_decided_by_execution_status()
        .into_iter()
        .chain(rows_decided_by_runner_health())
        .chain(rows_decided_by_test_outcomes())
        .chain(rows_with_no_usable_outcome());
    for row in rows {
        let judged = decide(&row.entry, &baseline());
        assert_eq!(
            judged,
            Judgement {
                verdict: row.verdict,
                label: row.label.map(str::to_owned),
                detail: row.detail.to_owned(),
            },
            "{} was judged wrongly",
            row.what
        );
    }
}

#[test]
fn a_timeout_outranks_the_failures_reported_alongside_it() {
    let timed_out_with_failures = entry(
        ExecutionStatus::Timeout,
        Some(RunnerResult {
            exit_class: ExitClass::TestFailures,
            failed: 3,
            ..baseline()
        }),
    );
    let judged = decide(&timed_out_with_failures, &baseline());
    assert_eq!(judged.verdict, Verdict::Timeout);
    assert_eq!(judged.detail, "timed out");
}

/// A run that never reached the test bodies is excluded even when the runner
/// managed to report failures on the way down. The asymmetry is the policy: a
/// failure proves the suite ran and reacted, an interrupted run proves nothing,
/// and counting the latter as a detection would inflate the score with kills
/// that say nothing about test quality.
#[test]
fn an_interrupted_run_outranks_the_failures_it_reported() {
    let interrupted_with_failures = entry(
        ExecutionStatus::Completed,
        Some(RunnerResult {
            exit_class: ExitClass::Interrupted,
            passed: 11,
            failed: 3,
            ..baseline()
        }),
    );
    let judged = decide(&interrupted_with_failures, &baseline());
    assert_eq!(judged.verdict, Verdict::RuntimeError);
    assert_eq!(judged.detail, "run interrupted");
}

/// The same asymmetry for the other half of the rule: a suite whose collection
/// failed did not run the tests that errored.
#[test]
fn a_collection_failure_outranks_the_errors_it_reported() {
    let collection_failed_with_errors = entry(
        ExecutionStatus::Completed,
        Some(RunnerResult {
            exit_class: ExitClass::TestFailures,
            collect_error: true,
            passed: 12,
            errors: 2,
            ..baseline()
        }),
    );
    let judged = decide(&collection_failed_with_errors, &baseline());
    assert_eq!(judged.verdict, Verdict::RuntimeError);
    assert_eq!(judged.detail, "collection failed");
}

/// A runner that classifies its exit as a test failure while reporting neither
/// a failure nor an error has contradicted itself. There is nothing to judge,
/// and the alternative to saying so is reporting a survival.
#[test]
fn contradictory_runner_signals_produce_no_verdict() {
    let failed_without_failures = entry(
        ExecutionStatus::Completed,
        Some(RunnerResult {
            exit_class: ExitClass::TestFailures,
            passed: 5,
            failed: 0,
            errors: 0,
            ..baseline()
        }),
    );
    let judged = decide(&failed_without_failures, &baseline());
    assert_eq!(judged.verdict, Verdict::RuntimeError);
    assert_eq!(judged.detail, "inconsistent runner signals");
}

/// A pack translates its runner's own time limit into a `timeout` status, so
/// this combination should not occur. If one ever reports the flag on an
/// otherwise completed attempt, the flag decides: a suite that stopped itself
/// never produced a result to judge.
#[test]
fn a_runner_that_stopped_itself_is_a_timeout_whatever_the_status_says() {
    let completed_but_timed_out = entry(
        ExecutionStatus::Completed,
        Some(RunnerResult {
            timed_out: true,
            ..baseline()
        }),
    );
    let judged = decide(&completed_but_timed_out, &baseline());
    assert_eq!(judged.verdict, Verdict::Timeout);
    assert_eq!(judged.detail, "timed out");
}
