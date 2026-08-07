//! Every example under `contracts/examples/` must agree with the committed
//! schema and with the Rust type at the same time.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::PathBuf};

use schemars::{JsonSchema, schema_for};
use serde::{Serialize, de::DeserializeOwned};
use tremula_contracts::{
    baseline::Baseline, capabilities::Capabilities, manifest::Manifest, pack_error::PackError,
    report::Report, results::Results,
};

fn contracts_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts")
        .join(relative)
}

fn read_json(relative: &str) -> serde_json::Value {
    let path = contracts_path(relative);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not JSON: {err}", path.display()))
}

/// A valid example must satisfy three things at once: the committed schema
/// accepts it, the Rust type deserializes it, and re-serializing reproduces it.
/// Together these catch any drift between the generated schema and what serde
/// actually accepts.
fn accepts<T>(example: &str, schema_file: &str)
where
    T: DeserializeOwned + Serialize + JsonSchema,
{
    let instance = read_json(&format!("examples/{example}"));

    let committed_schema = read_json(&format!("schemas/{schema_file}"));
    let validator = jsonschema::validator_for(&committed_schema)
        .unwrap_or_else(|err| panic!("{schema_file} is not a usable schema: {err}"));
    if let Err(error) = validator.validate(&instance) {
        panic!("{example} does not satisfy {schema_file}: {error}");
    }

    let generated_schema = serde_json::to_value(schema_for!(T)).unwrap();
    assert_eq!(
        committed_schema, generated_schema,
        "{schema_file} no longer matches the type it documents"
    );

    let parsed: T = serde_json::from_value(instance.clone())
        .unwrap_or_else(|err| panic!("{example} does not deserialize: {err}"));
    assert_eq!(
        instance,
        serde_json::to_value(&parsed).unwrap(),
        "{example} does not round-trip"
    );
}

#[test]
fn valid_examples_are_accepted() {
    accepts::<Manifest>("manifest/minimal.json", "manifest.schema.json");
    accepts::<Manifest>("manifest/full.json", "manifest.schema.json");
    accepts::<Results>("results/completed.json", "results.schema.json");
    accepts::<Baseline>("baseline/passing.json", "baseline.schema.json");
    accepts::<Report>("report/survived.json", "report.schema.json");
    accepts::<Capabilities>("capabilities/python.json", "capabilities.schema.json");
    accepts::<PackError>("pack-error/baseline-failed.json", "pack-error.schema.json");
}

#[test]
fn a_mutant_without_a_span_is_rejected() {
    let instance = read_json("examples/manifest/invalid-missing-span.json");

    let schema = read_json("schemas/manifest.schema.json");
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(
        !validator.is_valid(&instance),
        "the schema must be as strict as the type: a missing span must not validate"
    );

    assert!(
        serde_json::from_value::<Manifest>(instance).is_err(),
        "a mutant without a span must not deserialize"
    );
}

#[test]
fn unknown_fields_are_ignored() {
    let mut instance = read_json("examples/manifest/minimal.json");
    instance["future_field"] = serde_json::json!(42);
    serde_json::from_value::<Manifest>(instance)
        .expect("additive fields must not break existing consumers");
}

#[test]
fn the_report_score_agrees_with_its_verdicts() {
    let report: Report =
        serde_json::from_value(read_json("examples/report/survived.json")).unwrap();
    let counted = u32::try_from(report.verdicts.len()).unwrap();
    let score = report.score;
    assert_eq!(score.total, counted);
    assert_eq!(
        score.total,
        score.killed
            + score.survived
            + score.runtime_error
            + score.not_applied
            + score.not_run
            + score.skipped,
        "score categories must partition the verdicts; timeout is a subset of killed"
    );
    assert!(
        score.timeout <= score.killed,
        "timeout counts a subset of killed and can never exceed it"
    );
}
