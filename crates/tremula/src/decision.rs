//! The verdict decision rules. The core owns policy; a language pack owns the
//! translation from its backend's vocabulary into the neutral signals judged
//! here, so nothing in this module knows what a backend is.
//!
//! The rules form a ladder: the first condition that holds decides, and the
//! order is the policy. A mutation that never landed is reported as such rather
//! than as the absence of test failures it also is; a suite that timed out is a
//! detection rather than whatever partial counts it managed to report.

use tremula_contracts::{
    report::Verdict,
    results::{ExecutionStatus, ResultEntry},
    runner::{ExitClass, RunnerResult},
};

/// Version of the rules in this module, recorded in every report so that a
/// verdict stays attributable to the policy that produced it.
pub const DECISION_RULES_VERSION: &str = "v0";

/// What the rules concluded about one mutant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgement {
    /// The verdict itself.
    pub verdict: Verdict,
    /// Refinement of the verdict, such as `killed_by_error`.
    pub label: Option<String>,
    /// One-line summary for display, such as `3 failed`.
    pub detail: String,
}

impl Judgement {
    /// A verdict that needs no refinement.
    fn plain(verdict: Verdict, detail: impl Into<String>) -> Self {
        Self {
            verdict,
            label: None,
            detail: detail.into(),
        }
    }
}

/// Judge one mutant from its neutral signals, against the run's baseline.
///
/// The baseline supplies the test set the mutant run has to match: a mutant
/// that "passed" a suite that collected different tests has not been tested.
#[must_use]
pub fn decide(entry: &ResultEntry, baseline: &RunnerResult) -> Judgement {
    match entry.execution_status {
        ExecutionStatus::Skipped => Judgement::plain(Verdict::Skipped, "skipped"),
        ExecutionStatus::NotApplied => {
            Judgement::plain(Verdict::NotApplied, "mutation was never applied")
        }
        ExecutionStatus::NotRun => {
            Judgement::plain(Verdict::NotRun, "scheduled but never executed")
        }
        ExecutionStatus::BackendError => Judgement::plain(Verdict::RuntimeError, "backend error"),
        ExecutionStatus::Timeout => Judgement::plain(Verdict::Timeout, "timed out"),
        ExecutionStatus::Completed => match entry.runner.as_ref() {
            None => Judgement::plain(Verdict::RuntimeError, "no runner signal"),
            Some(runner) if runner.timed_out => Judgement::plain(Verdict::Timeout, "timed out"),
            Some(runner) => judge_runner(runner, baseline),
        },
    }
}

/// Judge a run that reached the test suite.
///
/// Failures and errors both count as detection: the test code ran and reacted
/// to the mutant. A collection or infrastructure failure does not, because no
/// test code ran at all — counting those as detections would inflate the score
/// with kills that say nothing about the suite.
fn judge_runner(runner: &RunnerResult, baseline: &RunnerResult) -> Judgement {
    if runner.collect_error {
        return Judgement::plain(Verdict::RuntimeError, "collection failed");
    }
    match runner.exit_class {
        ExitClass::Interrupted => {
            return Judgement::plain(Verdict::RuntimeError, "run interrupted");
        }
        ExitClass::InfraError => {
            return Judgement::plain(Verdict::RuntimeError, "runner infrastructure error");
        }
        ExitClass::Ok | ExitClass::TestFailures | ExitClass::NoTests => {}
    }
    if runner.failed > 0 {
        return Judgement::plain(Verdict::Killed, format!("{} failed", runner.failed));
    }
    if runner.errors > 0 {
        return Judgement {
            verdict: Verdict::Killed,
            label: Some("killed_by_error".to_owned()),
            detail: format!("{} errored", runner.errors),
        };
    }
    let same_test_set = runner.collected == baseline.collected
        && runner.collected_ids_hash == baseline.collected_ids_hash;
    if runner.passed >= 1 && runner.exit_class == ExitClass::Ok && same_test_set {
        return Judgement::plain(Verdict::Survived, format!("{} passed", runner.passed));
    }
    Judgement::plain(
        Verdict::RuntimeError,
        inconclusive_detail(runner, same_test_set),
    )
}

/// Why a completed run still produced no verdict. Each of these looks like a
/// survival from the outside — nothing failed — while proving nothing.
fn inconclusive_detail(runner: &RunnerResult, same_test_set: bool) -> &'static str {
    if runner.exit_class == ExitClass::NoTests {
        "no tests collected"
    } else if !same_test_set {
        "test set changed"
    } else if runner.passed == 0 {
        "all collected tests were skipped"
    } else {
        "inconsistent runner signals"
    }
}
