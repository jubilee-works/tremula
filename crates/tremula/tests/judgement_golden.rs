//! The committed contract examples must flow through the real pipeline: the
//! full manifest + completed results + passing baseline must produce exactly
//! the committed report example.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::PathBuf};

use serde::de::DeserializeOwned;
use tremula::{
    decision::DECISION_RULES_VERSION,
    report::{ReportError, build_report},
};
use tremula_contracts::{
    baseline::Baseline,
    manifest::Manifest,
    report::{Report, RunMeta},
    results::Results,
};

fn example<T: DeserializeOwned>(relative: &str) -> T {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples")
        .join(relative);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} does not deserialize: {err}", path.display()))
}

/// The provenance recorded in the committed report example. Only the two
/// version fields are derived rather than transcribed, so that renaming the
/// decision rules or bumping the crate without refreshing the example fails
/// here instead of drifting silently.
fn run_meta() -> RunMeta {
    RunMeta {
        run_id: "20260807T041500Z-3b1f8c".to_owned(),
        tremula_version: env!("CARGO_PKG_VERSION").to_owned(),
        decision_rules_version: DECISION_RULES_VERSION.to_owned(),
        project: ".".to_owned(),
        tests: Vec::new(),
        observed_revision: Some("9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a80".to_owned()),
        dirty: false,
        started_at: "2026-08-07T04:15:00Z".to_owned(),
        finished_at: "2026-08-07T04:15:04Z".to_owned(),
    }
}

#[test]
fn the_contract_examples_judge_into_the_committed_report() {
    let manifest: Manifest = example("manifest/full.json");
    let results: Results = example("results/completed.json");
    let baseline: Baseline = example("baseline/passing.json");
    let committed: Report = example("report/survived.json");

    let built = build_report(&manifest, &results, &baseline, run_meta()).unwrap();

    assert_eq!(built, committed);
}

#[test]
fn a_mutant_with_no_result_of_its_own_is_an_adapter_failure() {
    let manifest: Manifest = example("manifest/full.json");
    let mut results: Results = example("results/completed.json");
    let baseline: Baseline = example("baseline/passing.json");
    results.entries.clear();

    let error = build_report(&manifest, &results, &baseline, run_meta()).unwrap_err();

    assert!(matches!(error, ReportError::MissingEntry { .. }), "{error}");
}

#[test]
fn a_result_for_a_mutant_nobody_planned_is_an_adapter_failure() {
    let manifest: Manifest = example("manifest/full.json");
    let mut results: Results = example("results/completed.json");
    let baseline: Baseline = example("baseline/passing.json");
    let mut stranger = results.entries[0].clone();
    stranger.mutant_id = "0".repeat(64);
    results.entries.push(stranger);

    let error = build_report(&manifest, &results, &baseline, run_meta()).unwrap_err();

    assert!(matches!(error, ReportError::UnknownEntry { .. }), "{error}");
}

#[test]
fn two_results_for_one_mutant_are_an_adapter_failure() {
    let manifest: Manifest = example("manifest/full.json");
    let mut results: Results = example("results/completed.json");
    let baseline: Baseline = example("baseline/passing.json");
    let twin = results.entries[0].clone();
    results.entries.push(twin);

    let error = build_report(&manifest, &results, &baseline, run_meta()).unwrap_err();

    assert!(
        matches!(error, ReportError::DuplicateEntry { .. }),
        "{error}"
    );
}
