//! The reference judge over the same transport the reference generator uses.
//!
//! Everything about one provider exchange that is not the question being asked is
//! shared code, so these tests are about the part that is not shared: the schema
//! this ask sends, and what this ask makes of an answer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod scripted_provider;

use serde_json::json;
use tremula::generate::judge::{Claim, EquivalenceJudge, JudgeFailure};

use scripted_provider::{
    FAKE_KEY, MODEL, Provider, Reply, envelope, judgement, judgement_request, with_key,
};

#[test]
fn a_clean_answer_becomes_a_judgement_with_a_witness() {
    let said = judgement("distinguishable", Some("overlaps(0, 30, 30, 60)"));
    let provider = Provider::scripted(vec![Reply::ok(envelope(&said, "stop"))]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.judge().judge(&judgement_request()).unwrap()
    });
    assert_eq!(outcome.judgement.claim, Claim::Distinguishable);
    assert_eq!(
        outcome.judgement.witness.unwrap().call,
        "overlaps(0, 30, 30, 60)"
    );
    assert_eq!(outcome.model_resolved, MODEL);
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.attempts[0].usage.total, 1118);
    assert_eq!(provider.calls(), 1);
}

#[test]
fn a_claim_of_equivalence_arrives_without_one() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(
        &judgement("equivalent", None),
        "stop",
    ))]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.judge().judge(&judgement_request()).unwrap()
    });
    assert_eq!(outcome.judgement.claim, Claim::Equivalent);
    assert!(outcome.judgement.witness.is_none());
}

#[test]
fn the_request_sends_the_judgement_schema_under_a_name_of_its_own() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(
        &judgement("equivalent", None),
        "stop",
    ))]);
    with_key(Some(FAKE_KEY), || {
        provider.judge().judge(&judgement_request()).unwrap()
    });
    let format = provider.payload(0)["response_format"]["json_schema"].clone();
    assert_eq!(format["strict"], json!(true));
    assert_eq!(format["name"], json!("tremula_judgement"));
    assert_eq!(format["schema"]["properties"]["claim"]["enum"].clone(), {
        json!(["distinguishable", "equivalent"])
    });
    assert!(
        format["schema"]["properties"].get("mutants").is_none(),
        "a judgement is not a generation and asks for nothing of the kind"
    );
}

#[test]
fn the_question_reaches_the_model_as_two_turns_and_no_more() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(
        &judgement("equivalent", None),
        "stop",
    ))]);
    with_key(Some(FAKE_KEY), || {
        provider.judge().judge(&judgement_request()).unwrap()
    });
    let messages = provider.payload(0)["messages"].clone();
    let turns = messages.as_array().unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["role"], json!("system"));
    assert_eq!(turns[1]["role"], json!("user"));
    assert!(
        turns[1]["content"]
            .as_str()
            .unwrap()
            .contains("start <= other_end")
    );
}

#[test]
fn an_answer_that_is_not_the_contract_is_asked_for_once_more() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope("{\"claim\": \"maybe\"}", "stop")),
        Reply::ok(envelope(&judgement("equivalent", None), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.judge().judge(&judgement_request()).unwrap()
    });
    assert_eq!(outcome.judgement.claim, Claim::Equivalent);
    assert_eq!(provider.calls(), 2, "one corrected ask, and only one");
    let corrected = provider.payload(1)["messages"].clone();
    assert_eq!(corrected.as_array().unwrap().len(), 3);
    assert!(
        corrected[2]["content"]
            .as_str()
            .unwrap()
            .contains("did not satisfy the contract")
    );
}

#[test]
fn an_answer_that_is_never_the_contract_says_so_with_the_attempts_it_cost() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope("{\"claim\": \"maybe\"}", "stop")),
        Reply::ok(envelope("{\"claim\": \"perhaps\"}", "stop")),
    ]);
    let failure = with_key(Some(FAKE_KEY), || {
        provider.judge().judge(&judgement_request()).unwrap_err()
    });
    assert!(matches!(failure.kind, JudgeFailure::Unreadable { .. }));
    assert_eq!(failure.attempts.len(), 2);
    assert_eq!(failure.attempts[1].usage.total, 1118);
}

#[test]
fn a_truncated_answer_names_the_limit_rather_than_being_corrected() {
    let provider = Provider::scripted(vec![Reply::ok(envelope("{\"claim\":", "length"))]);
    let failure = with_key(Some(FAKE_KEY), || {
        provider.judge().judge(&judgement_request()).unwrap_err()
    });
    assert!(matches!(failure.kind, JudgeFailure::Truncated { .. }));
    assert_eq!(provider.calls(), 1, "a longer answer is not a better one");
}

#[test]
fn no_key_in_the_environment_costs_nothing() {
    let provider = Provider::scripted(vec![]);
    let failure = with_key(None, || {
        provider.judge().judge(&judgement_request()).unwrap_err()
    });
    assert!(matches!(
        failure.kind,
        JudgeFailure::MissingCredential { .. }
    ));
    assert!(failure.attempts.is_empty());
    assert_eq!(provider.calls(), 0);
}
