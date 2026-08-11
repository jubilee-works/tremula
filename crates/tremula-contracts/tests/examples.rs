//! Every example under `contracts/examples/` must agree with the committed
//! schema and with the Rust type at the same time.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::PathBuf};

use schemars::{JsonSchema, schema_for};
use serde::{Serialize, de::DeserializeOwned};
use tremula_contracts::{
    baseline::Baseline,
    bundle::BundleIndex,
    capabilities::Capabilities,
    manifest::Manifest,
    pack_error::{PackError, Stage},
    probe::{ProbeOutcome, ProbeReport},
    report::Report,
    results::Results,
    spans::{ExcludedKind, SpansReport},
    suppressions::{DismissalReason, Suppressions},
    triage::{Classification, Triage},
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

/// The committed schema, ready to judge a document.
fn validator(schema_file: &str) -> jsonschema::Validator {
    let committed_schema = read_json(&format!("schemas/{schema_file}"));
    jsonschema::validator_for(&committed_schema)
        .unwrap_or_else(|err| panic!("{schema_file} is not a usable schema: {err}"))
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

    if let Err(error) = validator(schema_file).validate(&instance) {
        panic!("{example} does not satisfy {schema_file}: {error}");
    }

    let committed_schema = read_json(&format!("schemas/{schema_file}"));
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
    accepts::<Manifest>("manifest/generated.json", "manifest.schema.json");
    accepts::<Results>("results/completed.json", "results.schema.json");
    accepts::<Baseline>("baseline/passing.json", "baseline.schema.json");
    accepts::<Report>("report/survived.json", "report.schema.json");
    accepts::<Capabilities>("capabilities/python.json", "capabilities.schema.json");
    accepts::<PackError>("pack-error/baseline-failed.json", "pack-error.schema.json");
    accepts::<SpansReport>("spans/simple.json", "spans.schema.json");
    accepts::<SpansReport>("spans/decorated.json", "spans.schema.json");
    accepts::<SpansReport>("spans/async_nested.json", "spans.schema.json");
    accepts::<ProbeReport>("probe/differs.json", "probe.schema.json");
    accepts::<ProbeReport>("probe/no-difference.json", "probe.schema.json");
    accepts::<ProbeReport>("probe/undecided.json", "probe.schema.json");
    accepts::<ProbeReport>("probe/raised.json", "probe.schema.json");
    accepts::<Triage>("triage/survivors.json", "triage.schema.json");
    accepts::<Suppressions>(
        "suppressions/two-decisions.json",
        "suppressions.schema.json",
    );
    accepts::<BundleIndex>("bundle/minimal.json", "bundle.schema.json");
    accepts::<BundleIndex>("bundle/with-triage.json", "bundle.schema.json");
}

/// A bundle index is read by whoever the evidence was handed to, and that consumer
/// is the furthest from this repository of any: it has to be able to read a bundle
/// written by a later tremula than the one it knows.
#[test]
fn unknown_fields_in_a_bundle_index_are_ignored() {
    let mut instance = read_json("examples/bundle/with-triage.json");
    instance["future_field"] = serde_json::json!(42);
    instance["documents"]["report"]["future_field"] = serde_json::json!("a signature");
    instance["attachments"][0]["future_field"] = serde_json::json!(true);
    instance["exposure"]["future_field"] = serde_json::json!("a kind of content named later");
    serde_json::from_value::<BundleIndex>(instance)
        .expect("additive fields must not break existing consumers");
}

/// What a bundle leaves out and what it says is empty have to mean the same thing,
/// in both directions: a consumer that finds no `triage` key must read the same
/// bundle as one that finds an explicit `null`, and re-serializing must not invent
/// a key that says "there is no triage" where the original said nothing.
#[test]
fn an_absent_optional_in_a_bundle_index_reads_as_an_empty_one() {
    let listed: BundleIndex =
        serde_json::from_value(read_json("examples/bundle/minimal.json")).unwrap();
    assert!(listed.documents.triage.is_none());
    assert!(listed.suite.tests.is_empty());
    assert!(listed.attachments[0].patch_error.is_none());

    let mut spelled_out = read_json("examples/bundle/minimal.json");
    spelled_out["documents"]["triage"] = serde_json::Value::Null;
    spelled_out["suite"]["tests"] = serde_json::json!([]);
    spelled_out["attachments"][0]["patch_error"] = serde_json::Value::Null;
    let same: BundleIndex = serde_json::from_value(spelled_out).unwrap();
    assert_eq!(listed, same, "an explicit null must read as an absent key");

    let round_tripped = serde_json::to_value(&same).unwrap();
    assert_eq!(
        round_tripped,
        read_json("examples/bundle/minimal.json"),
        "an empty optional must be written as an absent key, not as a null"
    );
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

/// The must-ignore rule holds for the newest document as much as for the oldest,
/// and at every depth: a field added to a nested object is as additive as one
/// added to the top level.
#[test]
fn unknown_fields_in_a_spans_report_are_ignored() {
    let mut instance = read_json("examples/spans/decorated.json");
    instance["future_field"] = serde_json::json!(42);
    instance["functions"][0]["future_field"] = serde_json::json!(["a", "statement", "list"]);
    instance["functions"][1]["excluded"][0]["future_field"] = serde_json::json!(true);
    serde_json::from_value::<SpansReport>(instance)
        .expect("additive fields must not break existing consumers");
}

/// A probe report is read by the core alone, and the must-ignore rule holds for it
/// as much as for a document both sides read: a pack that learns to say more about
/// what it observed must not break the core that asked it.
#[test]
fn unknown_fields_in_a_probe_report_are_ignored() {
    let mut instance = read_json("examples/probe/differs.json");
    instance["future_field"] = serde_json::json!(42);
    instance["original"]["future_field"] = serde_json::json!("a repr of something");
    serde_json::from_value::<ProbeReport>(instance)
        .expect("additive fields must not break existing consumers");
}

/// Every enum a producer may extend carries an `Unknown` fallback, which makes a
/// new variant as additive as a new field: a consumer built before the value
/// existed reads it rather than rejecting the whole document.
#[test]
fn enum_values_this_consumer_does_not_know_are_read_as_unknown() {
    let mut report = read_json("examples/spans/decorated.json");
    report["functions"][1]["excluded"][0]["kind"] = serde_json::json!("type_parameter");
    let parsed: SpansReport = serde_json::from_value(report).unwrap();
    assert_eq!(parsed.functions[1].excluded[0].kind, ExcludedKind::Unknown);

    let mut failure = read_json("examples/pack-error/baseline-failed.json");
    failure["error"]["stage"] = serde_json::json!("a_step_invented_later");
    let parsed: PackError = serde_json::from_value(failure).unwrap();
    assert_eq!(parsed.error.stage, Stage::Unknown);

    let mut probe = read_json("examples/probe/differs.json");
    probe["outcome"] = serde_json::json!("a_verdict_invented_later");
    let parsed: ProbeReport = serde_json::from_value(probe).unwrap();
    assert_eq!(parsed.outcome, ProbeOutcome::Unknown);

    let mut judged = read_json("examples/triage/survivors.json");
    judged["entries"][0]["classification"] = serde_json::json!("a_grade_invented_later");
    let parsed: Triage = serde_json::from_value(judged).unwrap();
    assert_eq!(parsed.entries[0].classification, Classification::Unknown);

    let mut decided = read_json("examples/suppressions/two-decisions.json");
    decided["suppressions"][0]["reason"] = serde_json::json!("a_reason_invented_later");
    let parsed: Suppressions = serde_json::from_value(decided).unwrap();
    assert_eq!(parsed.suppressions[0].reason, DismissalReason::Unknown);
}

/// The fallback is worth nothing if the published schema contradicts it. A
/// consumer that validates before it parses would refuse the very document the
/// fallback exists to let it read, so the schema of an extensible enum has to
/// accept a name no version of this contract has defined yet.
#[test]
fn a_schema_accepts_an_enum_value_it_does_not_name() {
    let mut report = read_json("examples/spans/decorated.json");
    report["functions"][1]["excluded"][0]["kind"] = serde_json::json!("type_parameter");
    if let Err(error) = validator("spans.schema.json").validate(&report) {
        panic!("a kind invented later must validate: {error}");
    }

    let mut failure = read_json("examples/pack-error/baseline-failed.json");
    failure["error"]["stage"] = serde_json::json!("a_step_invented_later");
    if let Err(error) = validator("pack-error.schema.json").validate(&failure) {
        panic!("a stage invented later must validate: {error}");
    }

    let mut probe = read_json("examples/probe/differs.json");
    probe["outcome"] = serde_json::json!("a_verdict_invented_later");
    if let Err(error) = validator("probe.schema.json").validate(&probe) {
        panic!("an outcome invented later must validate: {error}");
    }
}

/// Tolerance stops at the shape of the value. Nothing but a string could ever
/// name one of these, so the schema stays as strict as the type about that —
/// which is what keeps the schema a real check rather than an empty one.
#[test]
fn a_schema_refuses_an_enum_value_that_is_not_a_name() {
    let mut report = read_json("examples/spans/decorated.json");
    report["functions"][1]["excluded"][0]["kind"] = serde_json::json!(7);
    assert!(
        !validator("spans.schema.json").is_valid(&report),
        "a kind that is not a string must not validate"
    );
    assert!(
        serde_json::from_value::<SpansReport>(report).is_err(),
        "a kind that is not a string must not deserialize"
    );

    let mut failure = read_json("examples/pack-error/baseline-failed.json");
    failure["error"]["stage"] = serde_json::json!(["baseline"]);
    assert!(
        !validator("pack-error.schema.json").is_valid(&failure),
        "a stage that is not a string must not validate"
    );
    assert!(
        serde_json::from_value::<PackError>(failure).is_err(),
        "a stage that is not a string must not deserialize"
    );
}

/// The constants stay in the document a consumer reads even though they are no
/// longer a rule it is held to: a schema that only said "a string" would leave
/// a reader with no way to learn what this version actually emits.
#[test]
fn a_schema_still_names_the_enum_values_this_version_defines() {
    for (schema_file, definition, values) in [
        (
            "pack-error.schema.json",
            "Stage",
            &[
                "preflight",
                "spans",
                "probe",
                "validate",
                "baseline",
                "plan",
                "execute",
                "collect",
                "unknown",
            ][..],
        ),
        (
            "spans.schema.json",
            "ExcludedKind",
            &["docstring", "annotation", "unknown"][..],
        ),
        (
            "probe.schema.json",
            "ProbeOutcome",
            &["differs", "indistinguishable", "undecided", "unknown"][..],
        ),
        (
            "probe.schema.json",
            "Undecided",
            &[
                "nondeterministic",
                "incomparable",
                "unsafe_witness",
                "method",
                "no_such_function",
                "timed_out",
                "unknown",
            ][..],
        ),
        (
            "probe.schema.json",
            "Ending",
            &["returned", "raised", "unknown"][..],
        ),
        (
            "suppressions.schema.json",
            "DismissalReason",
            &["equivalent", "not_useful", "unknown"][..],
        ),
        (
            "triage.schema.json",
            "Classification",
            &[
                "distinguished_at_function_level",
                "suspected_equivalent",
                "undecided",
                "unknown",
            ][..],
        ),
        (
            "triage.schema.json",
            "Undecided",
            &[
                "no_witness",
                "witness_showed_no_difference",
                "nondeterministic",
                "incomparable",
                "unsafe_witness",
                "method",
                "no_such_function",
                "timed_out",
                "no_judgement",
                "unknown",
            ][..],
        ),
    ] {
        let schema = read_json(&format!("schemas/{schema_file}"));
        let described = schema["$defs"][definition]["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{definition} has no description in {schema_file}"));
        for value in values {
            assert!(
                described.contains(&format!("`{value}`")),
                "{definition} no longer names `{value}` for a reader of {schema_file}"
            );
        }
    }
}

/// The counts in a triage partition its entries, the same way a report's do.
#[test]
fn the_triage_score_agrees_with_its_entries() {
    let judged: Triage =
        serde_json::from_value(read_json("examples/triage/survivors.json")).unwrap();
    let score = judged.score;
    assert_eq!(
        score.survivors,
        u32::try_from(judged.entries.len()).unwrap()
    );
    assert_eq!(
        score.survivors,
        score.distinguished_at_function_level + score.suspected_equivalent + score.undecided,
        "every entry is classified exactly once"
    );
    assert!(
        judged
            .entries
            .iter()
            .all(|entry| (entry.classification == Classification::Undecided)
                == entry.undecided.is_some()),
        "a reason belongs to an undecided entry and to no other"
    );
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
