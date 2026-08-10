//! What the reference adapter asks again, and what it refuses to ask again.
//!
//! Two retries belong to the adapter, one each and neither borrowed from the
//! other: a wait when the provider says how long to wait, and one corrected ask
//! when the answer is not one this contract can read. A refusal, an answer cut
//! short, an empty answer, an answer that is not this protocol at all, a rejected
//! key: none of those is a question worth repeating, and each is a failure of its
//! own so that the person reading it knows which one they have. Whatever happens,
//! every attempt that reached the provider stays on the ledger the failure
//! carries, because the provider charges for those too.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod scripted_provider;

use std::time::Duration;

use serde_json::{Value, json};
use tremula::generate::{
    Attempt, CorrectionFeedback, Defect, GenerateFailure, MutantGenerator,
    openai::{KEY_VARIABLE, MAX_COMPLETION_TOKENS, RETRY_AFTER_CAP, retry_delay},
};

use scripted_provider::{FAKE_KEY, MODEL, Provider, Reply, answer, envelope, request, with_key};

/// An otherwise well-formed answer with a member the sent schema forbids.
fn answer_carrying_a_span(count: usize) -> String {
    let mut body: Value = serde_json::from_str(&answer(count)).unwrap();
    body["mutants"][0]["span"] = json!({"start_byte": 1, "end_byte": 2});
    body.to_string()
}

/// A rate limit that says to come back immediately.
fn rate_limited() -> Reply {
    Reply::refusing(429, r#"{"error":{"message":"slow down"}}"#).and_header("Retry-After", "0")
}

#[test]
fn an_answer_that_breaks_the_contract_is_asked_again() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope(r#"{"mutants":[{"file":"ranges.py"}]}"#, "stop")),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(outcome.attempts.len(), 2, "both attempts are on the ledger");
    assert_eq!(provider.calls(), 2);
    let asked_again = provider.payload(1);
    let turns = asked_again["messages"].as_array().unwrap();
    assert_eq!(turns.len(), 3, "the correction is a turn of its own");
    let correction = turns[2]["content"].as_str().unwrap();
    assert!(correction.contains("Problems:"), "{correction}");
    assert!(correction.contains("missing field"), "{correction}");
}

#[test]
fn a_second_answer_that_breaks_the_contract_is_the_end_of_it() {
    let broken = || Reply::ok(envelope("not JSON at all", "stop"));
    let provider = Provider::scripted(vec![broken(), broken()]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    assert!(matches!(error.kind, GenerateFailure::Unreadable { .. }));
    assert_eq!(error.attempts.len(), 2, "both attempts were paid for");
    assert_eq!(provider.calls(), 2, "one retry, and only one");
    assert!(
        error.to_string().contains("one corrected retry"),
        "two calls arrived, so saying one was corrected is a true account: {error}"
    );
}

#[test]
fn a_member_the_sent_schema_forbids_is_asked_again() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope(&answer_carrying_a_span(4), "stop")),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(outcome.attempts.len(), 2, "both attempts are on the ledger");
    assert_eq!(provider.calls(), 2);
    let correction = provider.payload(1)["messages"][2]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        correction.contains("answer_is_not_the_contract"),
        "{correction}"
    );
    assert!(
        correction.contains("span"),
        "the model is told which field it sent that the schema forbade: {correction}"
    );
}

#[test]
fn a_member_the_sent_schema_forbids_twice_is_the_end_of_it() {
    let forbidden = || Reply::ok(envelope(&answer_carrying_a_span(4), "stop"));
    let provider = Provider::scripted(vec![forbidden(), forbidden()]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match &error.kind {
        GenerateFailure::Unreadable { reason } => assert!(reason.contains("span"), "{reason}"),
        other => panic!("{other}"),
    }
    assert_eq!(error.attempts.len(), 2, "both attempts were paid for");
    assert_eq!(provider.calls(), 2, "one retry, and only one");
}

#[test]
fn an_answer_that_is_not_this_protocol_at_all_is_a_failure_of_its_own() {
    let provider = Provider::scripted(vec![Reply::ok(
        "<html>a proxy said hello</html>".to_owned(),
    )]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    assert!(
        matches!(error.kind, GenerateFailure::UnreadableEnvelope { .. }),
        "{}",
        error.kind
    );
    assert_eq!(
        provider.calls(),
        1,
        "a correction is addressed to a model, and no model spoke here"
    );
    let said = error.to_string();
    assert!(
        !said.contains("retr"),
        "nothing was asked twice, so nothing says it was: {said}"
    );
    assert_eq!(
        error.attempts,
        vec![Attempt::default()],
        "the attempt is on the ledger, with nothing readable to say what it cost"
    );
}

#[test]
fn an_answer_with_the_wrong_number_of_mutants_is_asked_again() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope(&answer(2), "stop")),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(outcome.attempts.len(), 2);
    let correction = provider.payload(1)["messages"][2]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        correction.contains("wrong_number_of_mutants"),
        "{correction}"
    );
}

#[test]
fn a_number_of_mutants_that_stays_wrong_names_both_numbers() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope(&answer(2), "stop")),
        Reply::ok(envelope(&answer(3), "stop")),
    ]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match error.kind {
        GenerateFailure::WrongCount {
            requested,
            received,
        } => {
            assert_eq!(requested, 4);
            assert_eq!(received, 3);
        }
        other => panic!("{other}"),
    }
    assert_eq!(error.attempts.len(), 2);
    assert!(
        error.to_string().contains("one corrected retry"),
        "two calls arrived, so saying one was corrected is a true account: {error}"
    );
}

#[test]
fn what_the_caller_found_travels_with_the_first_call() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(&answer(4), "stop"))]);
    let mut asked = request(4);
    asked.feedback = Some(CorrectionFeedback {
        defects: vec![Defect {
            defect: "original_not_found".to_owned(),
            detail: "no such text in the function".to_owned(),
        }],
    });
    with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&asked).unwrap()
    });
    let turns = provider.payload(0)["messages"].as_array().unwrap().clone();
    assert_eq!(turns.len(), 3);
    assert!(
        turns[2]["content"]
            .as_str()
            .unwrap()
            .contains("original_not_found")
    );
}

#[test]
fn what_the_caller_found_is_still_there_when_the_answer_is_asked_again() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope("not JSON at all", "stop")),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let mut asked = request(4);
    asked.feedback = Some(CorrectionFeedback {
        defects: vec![Defect {
            defect: "original_is_ambiguous".to_owned(),
            detail: "found 2 times".to_owned(),
        }],
    });
    with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&asked).unwrap()
    });
    let correction = provider.payload(1)["messages"][2]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(correction.contains("original_is_ambiguous"), "{correction}");
    assert!(
        correction.contains("answer_is_not_the_contract"),
        "both defects are still true, so both are sent: {correction}"
    );
}

#[test]
fn a_refusal_is_a_failure_of_its_own() {
    let body = json!({
        "model": MODEL,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": null, "refusal": "I will not do that"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 600, "completion_tokens": 12, "total_tokens": 612},
    });
    let provider = Provider::scripted(vec![Reply::ok(body.to_string())]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match &error.kind {
        GenerateFailure::Refused { reason } => assert_eq!(reason, "I will not do that"),
        other => panic!("{other}"),
    }
    assert_eq!(provider.calls(), 1, "a refusal is not asked again");
    assert_eq!(error.attempts.len(), 1);
    assert_eq!(
        error.attempts[0].usage.total, 612,
        "a refusal is charged for"
    );
}

#[test]
fn an_answer_cut_off_at_the_limit_is_a_failure_of_its_own() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(
        r#"{"mutants":[{"file":"ranges.p"#,
        "length",
    ))]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match error.kind {
        GenerateFailure::Truncated { limit } => assert_eq!(limit, MAX_COMPLETION_TOKENS),
        other => panic!("{other}"),
    }
    assert_eq!(provider.calls(), 1, "a truncated answer is not asked again");
    assert_eq!(error.attempts.len(), 1);
    assert_eq!(error.attempts[0].finish.as_deref(), Some("length"));
    assert_eq!(error.attempts[0].usage.prompt, 682);
    assert_eq!(error.attempts[0].usage.completion, 436);
    assert_eq!(
        error.attempts[0].usage.total, 1118,
        "an answer that ran out of room is charged for every token it did spend"
    );
}

#[test]
fn an_answer_with_nothing_in_it_is_a_failure_of_its_own() {
    let body = json!({
        "model": MODEL,
        "choices": [],
        "usage": {"prompt_tokens": 600, "completion_tokens": 0, "total_tokens": 600},
    });
    let provider = Provider::scripted(vec![Reply::ok(body.to_string())]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    assert!(matches!(error.kind, GenerateFailure::NoAnswer));
    assert_eq!(provider.calls(), 1);
    assert_eq!(error.attempts.len(), 1);
    assert_eq!(
        error.attempts[0].usage.prompt, 600,
        "an answer with nothing in it is charged for too"
    );
}

#[test]
fn a_rate_limit_that_says_when_is_waited_out_once() {
    let provider = Provider::scripted(vec![
        rate_limited(),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(
        outcome.attempts.len(),
        2,
        "the refused attempt is on the ledger too"
    );
    assert_eq!(outcome.attempts[0].usage.total, 0);
    assert!(outcome.attempts[0].finish.is_none());
}

#[test]
fn a_rate_limit_that_says_nothing_fails_at_once() {
    let provider = Provider::scripted(vec![Reply::refusing(429, r#"{"error":{"code":"quota"}}"#)]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match error.kind {
        GenerateFailure::RateLimited { retried } => assert!(!retried),
        other => panic!("{other}"),
    }
    assert!(error.to_string().contains("quota"));
    assert_eq!(provider.calls(), 1);
    assert_eq!(error.attempts.len(), 1);
}

#[test]
fn a_rate_limit_that_repeats_says_the_retry_was_spent() {
    let provider = Provider::scripted(vec![rate_limited(), rate_limited()]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match error.kind {
        GenerateFailure::RateLimited { retried } => assert!(retried),
        other => panic!("{other}"),
    }
    assert_eq!(provider.calls(), 2);
    assert_eq!(error.attempts.len(), 2);
}

#[test]
fn a_wait_that_was_spent_still_leaves_the_corrected_ask() {
    let provider = Provider::scripted(vec![
        rate_limited(),
        Reply::ok(envelope("not JSON at all", "stop")),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(
        provider.calls(),
        3,
        "waiting out a rate limit does not spend the retry that corrects an answer"
    );
    assert_eq!(outcome.attempts.len(), 3);
    let correction = provider.payload(2)["messages"][2]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        correction.contains("answer_is_not_the_contract"),
        "{correction}"
    );
}

#[test]
fn a_corrected_ask_still_leaves_the_wait_a_rate_limit_asks_for() {
    let provider = Provider::scripted(vec![
        Reply::ok(envelope("not JSON at all", "stop")),
        rate_limited(),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(
        provider.calls(),
        3,
        "correcting an answer does not spend the wait a rate limit asks for"
    );
    assert_eq!(outcome.attempts.len(), 3);
    let asked_again = provider.payload(2)["messages"].as_array().unwrap().clone();
    assert_eq!(
        asked_again.len(),
        3,
        "the correction is still on the call the wait let through"
    );
}

#[test]
fn a_wait_a_provider_asks_for_is_capped() {
    assert_eq!(retry_delay("0"), Some(Duration::from_secs(0)));
    assert_eq!(retry_delay("3"), Some(Duration::from_secs(3)));
    assert_eq!(
        retry_delay("120"),
        Some(RETRY_AFTER_CAP),
        "a call that waits longer than this has stopped being a call"
    );
    assert_eq!(
        retry_delay("Wed, 21 Oct 2026 07:28:00 GMT"),
        Some(RETRY_AFTER_CAP),
        "a provider that asked to be left alone gets a retry, and a date this does not parse gets the longest wait there is"
    );
    assert_eq!(
        retry_delay(""),
        None,
        "a header with nothing in it asked for nothing"
    );
}

#[test]
fn a_rejected_key_says_which_variable_to_look_at() {
    let provider = Provider::scripted(vec![Reply::refusing(
        401,
        r#"{"error":{"message":"Incorrect API key"}}"#,
    )]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match &error.kind {
        GenerateFailure::CredentialRejected { variable } => assert_eq!(variable, KEY_VARIABLE),
        other => panic!("{other}"),
    }
    assert_eq!(provider.calls(), 1, "a bad key is not tried twice");
    assert_eq!(
        error.attempts,
        vec![Attempt::default()],
        "a call the provider would not accept is still on the ledger, reporting nothing consumed"
    );
}

#[test]
fn no_key_is_a_failure_before_anything_is_sent() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(&answer(4), "stop"))]);
    let error = with_key(None, || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    match &error.kind {
        GenerateFailure::MissingCredential { variable } => assert_eq!(variable, KEY_VARIABLE),
        other => panic!("{other}"),
    }
    assert!(error.attempts.is_empty(), "nothing was sent, nothing owed");
    assert_eq!(provider.calls(), 0);
}

#[test]
fn a_provider_that_never_answers_is_a_failure_and_not_a_wait() {
    let provider = Provider::scripted(vec![Reply::silence()]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    assert!(
        matches!(error.kind, GenerateFailure::Unreachable { .. }),
        "{}",
        error.kind
    );
    assert_eq!(
        error.attempts,
        vec![Attempt::default()],
        "a call that never came back is on the ledger too, reporting nothing consumed"
    );
}
