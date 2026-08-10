//! Every committed schema under `contracts/schemas/` must match the type it
//! documents.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{env, fs, path::PathBuf};

use schemars::{JsonSchema, schema_for};
use tremula_contracts::{
    baseline::Baseline, capabilities::Capabilities, manifest::Manifest, pack_error::PackError,
    probe::ProbeReport, report::Report, results::Results, spans::SpansReport,
    suppressions::Suppressions, triage::Triage,
};

/// Compare a generated schema against the committed file under `contracts/schemas/`.
///
/// Set `TREMULA_UPDATE_CONTRACTS` to any value other than `0` to rewrite the
/// committed files instead (`just contracts`).
fn assert_schema_matches<T: JsonSchema>(file_name: &str) {
    let generated = serde_json::to_string_pretty(&schema_for!(T)).unwrap() + "\n";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/schemas")
        .join(file_name);

    if env::var_os("TREMULA_UPDATE_CONTRACTS").is_some_and(|value| value != "0") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &generated).unwrap();
        return;
    }

    let committed = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "cannot read {}: {err}. Run `just contracts` to generate it.",
            path.display()
        )
    });
    assert_eq!(
        committed,
        generated,
        "{} is out of date. Run `just contracts` to regenerate it.",
        path.display()
    );
}

#[test]
fn manifest_schema_is_up_to_date() {
    assert_schema_matches::<Manifest>("manifest.schema.json");
}

#[test]
fn results_schema_is_up_to_date() {
    assert_schema_matches::<Results>("results.schema.json");
}

#[test]
fn baseline_schema_is_up_to_date() {
    assert_schema_matches::<Baseline>("baseline.schema.json");
}

#[test]
fn report_schema_is_up_to_date() {
    assert_schema_matches::<Report>("report.schema.json");
}

#[test]
fn capabilities_schema_is_up_to_date() {
    assert_schema_matches::<Capabilities>("capabilities.schema.json");
}

#[test]
fn pack_error_schema_is_up_to_date() {
    assert_schema_matches::<PackError>("pack-error.schema.json");
}

#[test]
fn spans_schema_is_up_to_date() {
    assert_schema_matches::<SpansReport>("spans.schema.json");
}

#[test]
fn probe_schema_is_up_to_date() {
    assert_schema_matches::<ProbeReport>("probe.schema.json");
}

#[test]
fn triage_schema_is_up_to_date() {
    assert_schema_matches::<Triage>("triage.schema.json");
}

#[test]
fn suppressions_schema_is_up_to_date() {
    assert_schema_matches::<Suppressions>("suppressions.schema.json");
}
