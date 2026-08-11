//! Report assembly: turn a manifest, one pack's results, and the run's
//! baseline into the document machines read.
//!
//! Two rules govern this module. The report must cover every manifest mutant
//! exactly once — a missing, surplus, or repeated result is an adapter defect,
//! not a verdict — and the exit code must never let an incomplete run look
//! successful.

use std::collections::{HashMap, HashSet};

use tremula_contracts::{
    SCHEMA_VERSION,
    baseline::Baseline,
    manifest::Manifest,
    report::{MutantVerdict, Report, RunMeta, Score, Verdict},
    results::{ResultEntry, Results},
};

use crate::decision::decide;

/// Survived means the suite did not detect the mutant, which is not the same as
/// the mutant having been exercised at all.
pub const CAVEAT_COVERAGE_UNVERIFIED: &str =
    "survived = not killed by the existing suite; execution coverage is unverified";

/// Some mutants cannot change observable behaviour, so no suite could detect
/// them. tremula does not prove which ones those are.
pub const CAVEAT_POSSIBLE_EQUIVALENTS: &str = "survived may include equivalent mutants";

/// A mismatch between the manifest and the results. Each of these means the
/// adapter, not the project under test, is broken.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// The results say nothing about a mutant the manifest asked for.
    #[error(
        "mutant {mutant_id}: the manifest asked for this mutant but the results do not mention it; the run is incomplete and its score would be misleading"
    )]
    MissingEntry {
        /// The mutant with no result of its own.
        mutant_id: String,
    },
    /// The results report on a mutant the manifest never asked for.
    #[error(
        "mutant {mutant_id}: the results report on a mutant that is not in the manifest; the results belong to a different run"
    )]
    UnknownEntry {
        /// The unplanned mutant the results mention.
        mutant_id: String,
    },
    /// One mutant has more than one result.
    #[error(
        "mutant {mutant_id}: the results report on this mutant more than once; a mutant has exactly one outcome"
    )]
    DuplicateEntry {
        /// The mutant reported on twice.
        mutant_id: String,
    },
}

/// Judge every mutant in `manifest` and assemble the run's report.
///
/// `run` carries the provenance the caller collected — the run identifier, the
/// observed revision, the timestamps — because none of it can be derived from
/// the three documents.
///
/// The manifest's identifiers are trusted to be unique, because validation runs
/// first and rejects a manifest that repeats one. Behaviour on a manifest with
/// a duplicate identifier is unspecified.
///
/// # Errors
///
/// Returns [`ReportError`] when the results do not cover the manifest exactly
/// once per mutant.
pub fn build_report(
    manifest: &Manifest,
    results: &Results,
    baseline: &Baseline,
    run: RunMeta,
) -> Result<Report, ReportError> {
    let entries = index_by_mutant(manifest, results)?;
    let mut verdicts = Vec::with_capacity(manifest.mutants.len());
    for mutant in &manifest.mutants {
        let Some(entry) = entries.get(mutant.id.as_str()) else {
            return Err(ReportError::MissingEntry {
                mutant_id: mutant.id.clone(),
            });
        };
        let judged = decide(entry, &baseline.runner);
        verdicts.push(MutantVerdict {
            mutant_id: mutant.id.clone(),
            file: mutant.file.clone(),
            span: mutant.span,
            location: entry.location,
            verdict: judged.verdict,
            label: judged.label,
            detail: judged.detail,
        });
    }
    Ok(assemble(run, verdicts))
}

/// The report of a run that had nothing to judge.
///
/// A manifest with no mutants is valid input — a change that touched no code has
/// none — and such a run still produces a report, because a consumer that reads
/// one run's report should not have to special-case the empty case. The score
/// policy and the caveats come from the same place as every other report's, so
/// the two cannot drift apart.
#[must_use]
pub fn build_empty_report(run: RunMeta) -> Report {
    assemble(run, Vec::new())
}

/// Wrap judged verdicts in the document that carries them.
fn assemble(run: RunMeta, verdicts: Vec<MutantVerdict>) -> Report {
    let score = tally(&verdicts);
    Report {
        schema_version: SCHEMA_VERSION.to_owned(),
        run,
        verdicts,
        score,
        exit_code: exit_code(score),
        caveats: vec![
            CAVEAT_COVERAGE_UNVERIFIED.to_owned(),
            CAVEAT_POSSIBLE_EQUIVALENTS.to_owned(),
        ],
    }
}

/// Index the results by mutant, refusing anything but an exact cover of the
/// manifest.
fn index_by_mutant<'results>(
    manifest: &Manifest,
    results: &'results Results,
) -> Result<HashMap<&'results str, &'results ResultEntry>, ReportError> {
    let mut indexed = HashMap::with_capacity(results.entries.len());
    for entry in &results.entries {
        if indexed.insert(entry.mutant_id.as_str(), entry).is_some() {
            return Err(ReportError::DuplicateEntry {
                mutant_id: entry.mutant_id.clone(),
            });
        }
    }
    for mutant in &manifest.mutants {
        if !indexed.contains_key(mutant.id.as_str()) {
            return Err(ReportError::MissingEntry {
                mutant_id: mutant.id.clone(),
            });
        }
    }
    let planned: HashSet<&str> = manifest
        .mutants
        .iter()
        .map(|mutant| mutant.id.as_str())
        .collect();
    for entry in &results.entries {
        if !planned.contains(entry.mutant_id.as_str()) {
            return Err(ReportError::UnknownEntry {
                mutant_id: entry.mutant_id.clone(),
            });
        }
    }
    Ok(indexed)
}

/// Count the verdicts. A timeout counts towards `killed` as well as towards
/// `timeout`, so the six categories still partition the total.
fn tally(verdicts: &[MutantVerdict]) -> Score {
    let mut score = Score {
        total: u32::try_from(verdicts.len()).unwrap_or(u32::MAX),
        killed: 0,
        timeout: 0,
        survived: 0,
        runtime_error: 0,
        not_applied: 0,
        not_run: 0,
        skipped: 0,
    };
    for judged in verdicts {
        match judged.verdict {
            Verdict::Killed => score.killed += 1,
            Verdict::Timeout => {
                score.killed += 1;
                score.timeout += 1;
            }
            Verdict::Survived => score.survived += 1,
            Verdict::RuntimeError => score.runtime_error += 1,
            Verdict::NotApplied => score.not_applied += 1,
            Verdict::NotRun => score.not_run += 1,
            Verdict::Skipped => score.skipped += 1,
        }
    }
    score
}

/// The run's exit code: 2 outranks 1 outranks 0.
///
/// 2 is reserved for a run whose score cannot be trusted — a mutation that
/// never landed, a mutant that never ran, or a manifest that produced no usable
/// verdict at all. A single mutant with an environment problem is not one of
/// those: it is reported and excluded, and the rest of the run still counts.
///
/// Run-level infrastructure failures (a failing baseline, a pack that could not
/// start) also exit 2, but they never reach this function: there is no report
/// to build.
fn exit_code(score: Score) -> u8 {
    let usable = score.killed + score.survived;
    let incomplete = score.not_applied > 0 || score.not_run > 0;
    if incomplete || (score.total >= 1 && usable == 0) {
        return 2;
    }
    u8::from(score.survived > 0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use tremula_contracts::{
        manifest::{Base, Language, Mutant, Span},
        results::{ExecutionStatus, Location, PackInfo},
        runner::{ExitClass, RunnerResult},
    };

    use super::{
        Baseline, Manifest, Report, ResultEntry, Results, RunMeta, Score, build_report, tally,
    };

    const TEST_SET: &str = "b5c7d9e1f3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7";

    /// What a test wants a mutant's outcome to be, spelled as the neutral
    /// signals a pack would report to get it.
    #[derive(Clone, Copy)]
    enum Outcome {
        Killed,
        Survived,
        TimedOut,
        Erroring,
        NotApplied,
        NotRun,
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
            duration_ms: 1180,
        }
    }

    fn signals(outcome: Outcome) -> (ExecutionStatus, Option<RunnerResult>) {
        match outcome {
            Outcome::Killed => (
                ExecutionStatus::Completed,
                Some(RunnerResult {
                    exit_class: ExitClass::TestFailures,
                    passed: 13,
                    failed: 1,
                    ..passing_runner()
                }),
            ),
            Outcome::Survived => (ExecutionStatus::Completed, Some(passing_runner())),
            Outcome::TimedOut => (ExecutionStatus::Timeout, None),
            Outcome::Erroring => (ExecutionStatus::BackendError, None),
            Outcome::NotApplied => (ExecutionStatus::NotApplied, None),
            Outcome::NotRun => (ExecutionStatus::NotRun, None),
            Outcome::Skipped => (ExecutionStatus::Skipped, None),
        }
    }

    fn run_meta() -> RunMeta {
        RunMeta {
            run_id: "20260807T041500Z-3b1f8c".to_owned(),
            tremula_version: env!("CARGO_PKG_VERSION").to_owned(),
            decision_rules_version: crate::decision::DECISION_RULES_VERSION.to_owned(),
            project: ".".to_owned(),
            tests: Vec::new(),
            observed_revision: None,
            dirty: false,
            started_at: "2026-08-07T04:15:00Z".to_owned(),
            finished_at: "2026-08-07T04:15:04Z".to_owned(),
        }
    }

    /// Judge one mutant per outcome, with the manifest and the results covering
    /// each other exactly.
    fn judge(outcomes: &[Outcome]) -> Report {
        let mut mutants = Vec::new();
        let mut entries = Vec::new();
        for (index, outcome) in outcomes.iter().enumerate() {
            let id = format!("mutant-{index}");
            let start = u64::try_from(index).unwrap_or(0) * 100;
            mutants.push(Mutant {
                id: id.clone(),
                file: "src/pkg/mod.py".to_owned(),
                base_file_sha256: TEST_SET.to_owned(),
                span: Span {
                    start_byte: start,
                    end_byte: start + 10,
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
            schema_version: super::SCHEMA_VERSION.to_owned(),
            language: Language::Python,
            base: Base { revision: None },
            mutants,
        };
        let results = Results {
            schema_version: super::SCHEMA_VERSION.to_owned(),
            run_id: "20260807T041500Z-3b1f8c".to_owned(),
            pack: PackInfo {
                name: "tremula-python".to_owned(),
                version: "0.1.0".to_owned(),
                contract_version: super::SCHEMA_VERSION.to_owned(),
            },
            entries,
        };
        let baseline = Baseline {
            schema_version: super::SCHEMA_VERSION.to_owned(),
            run_id: "20260807T041500Z-3b1f8c".to_owned(),
            runner: passing_runner(),
        };
        build_report(&manifest, &results, &baseline, run_meta()).unwrap()
    }

    /// Every exit code the judgement pipeline can produce, and why. The point
    /// of the table is the precedence: 2 for a run that cannot be trusted, 1
    /// for a trustworthy run with survivors, 0 otherwise.
    #[test]
    fn the_exit_code_follows_the_documented_precedence() {
        let cases: &[(&str, &[Outcome], u8)] = &[
            ("nothing to test", &[], 0),
            ("a survivor", &[Outcome::Survived], 1),
            (
                "survivors alongside one mutant with an environment problem",
                &[
                    Outcome::Survived,
                    Outcome::Survived,
                    Outcome::Survived,
                    Outcome::Erroring,
                ],
                1,
            ),
            (
                "survivors alongside a mutation that never landed",
                &[
                    Outcome::Survived,
                    Outcome::Survived,
                    Outcome::Survived,
                    Outcome::NotApplied,
                ],
                2,
            ),
            ("a mutant that never ran", &[Outcome::NotRun], 2),
            (
                "a manifest that produced no usable verdict",
                &[Outcome::Erroring, Outcome::Erroring],
                2,
            ),
            (
                "a manifest whose every mutant was filtered out",
                &[Outcome::Skipped, Outcome::Skipped],
                2,
            ),
            ("only kills", &[Outcome::Killed, Outcome::Killed], 0),
            (
                "a timeout alongside a kill",
                &[Outcome::TimedOut, Outcome::Killed],
                0,
            ),
        ];
        for (what, outcomes, expected) in cases {
            assert_eq!(judge(outcomes).exit_code, *expected, "{what}");
        }
    }

    #[test]
    fn an_empty_manifest_scores_nothing_and_succeeds() {
        let report = judge(&[]);
        assert_eq!(report.score, tally(&[]));
        assert_eq!(report.score.total, 0);
        assert_eq!(report.exit_code, 0);
        assert!(report.verdicts.is_empty());
    }

    #[test]
    fn the_score_partitions_the_verdicts_with_timeouts_inside_killed() {
        let report = judge(&[
            Outcome::Killed,
            Outcome::TimedOut,
            Outcome::Survived,
            Outcome::Erroring,
            Outcome::NotApplied,
            Outcome::NotRun,
            Outcome::Skipped,
        ]);
        let Score {
            total,
            killed,
            timeout,
            survived,
            runtime_error,
            not_applied,
            not_run,
            skipped,
        } = report.score;
        assert_eq!(total, 7);
        assert_eq!(killed, 2, "a timeout counts as a detection");
        assert_eq!(timeout, 1);
        assert!(timeout <= killed);
        assert_eq!(
            total,
            killed + survived + runtime_error + not_applied + not_run + skipped,
            "the six categories must partition the total"
        );
    }
}
