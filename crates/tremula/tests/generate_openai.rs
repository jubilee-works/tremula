//! What the reference adapter sends, and what it makes of a real answer.
//!
//! Two of the answers here are not written by hand: they are the bodies a
//! provider returned, kept under `tests/fixtures/llm/` as they arrived, and they
//! are replayed to prove the adapter reads what a provider actually sends rather
//! than what a test author imagines it sends.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod scripted_provider;

use serde_json::{Value, json};
use tremula::generate::{
    GenerateFailure, MutantGenerator,
    openai::{KEY_VARIABLE, MAX_COMPLETION_TOKENS, OpenAiGenerator, response_schema},
};

use scripted_provider::{FAKE_KEY, MODEL, Provider, Reply, answer, envelope, request, with_key};

#[test]
fn a_clean_answer_becomes_mutants() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(&answer(4), "stop"))]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(outcome.model_resolved, MODEL);
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.attempts[0].usage.prompt, 682);
    assert_eq!(outcome.attempts[0].usage.completion, 436);
    assert_eq!(outcome.attempts[0].usage.total, 1118);
    assert_eq!(outcome.attempts[0].finish.as_deref(), Some("stop"));
    assert_eq!(provider.calls(), 1);
}

#[test]
fn the_request_names_its_token_limit_the_way_this_endpoint_does() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(&answer(4), "stop"))]);
    with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    let payload = provider.payload(0);
    assert_eq!(
        payload["max_completion_tokens"],
        json!(MAX_COMPLETION_TOKENS)
    );
    assert!(
        payload.get("max_output_tokens").is_none(),
        "that name belongs to another endpoint, which refuses this one's requests"
    );
    assert_eq!(payload["model"], json!(MODEL));
}

#[test]
fn the_request_leaves_the_sampling_controls_alone() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(&answer(4), "stop"))]);
    with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    let payload = provider.payload(0);
    for absent in ["temperature", "seed", "top_p"] {
        assert!(
            payload.get(absent).is_none(),
            "`{absent}` was absent from what was measured, and the measurement stands only for what was sent"
        );
    }
}

#[test]
fn the_schema_that_is_sent_is_strict_and_closed() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(&answer(4), "stop"))]);
    with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    let format = provider.payload(0)["response_format"].clone();
    assert_eq!(format["type"], json!("json_schema"));
    assert_eq!(format["json_schema"]["strict"], json!(true));
    assert!(format["json_schema"]["name"].is_string());
    let schema = &format["json_schema"]["schema"];
    assert_eq!(schema["type"], json!("object"));
    assert_eq!(schema["additionalProperties"], json!(false));
    let items = &schema["properties"]["mutants"]["items"];
    assert_eq!(items["additionalProperties"], json!(false));
    assert_eq!(
        items["required"],
        json!(["file", "original", "replacement", "description"])
    );
    assert!(
        items["properties"].get("span").is_none(),
        "no span is asked for"
    );
}

#[test]
fn the_schema_is_the_one_that_was_measured_without_the_span() {
    let recorded: Value =
        serde_json::from_str(include_str!("fixtures/llm/recorded-response-overlaps.json")).unwrap();
    let mut measured = recorded["request"]["response_format"]["json_schema"]["schema"].clone();
    let items = &mut measured["properties"]["mutants"]["items"];
    items["properties"].as_object_mut().unwrap().remove("span");
    items["required"]
        .as_array_mut()
        .unwrap()
        .retain(|field| field != "span");
    assert_eq!(essentials(&response_schema()), essentials(&measured));
}

/// A schema without the words that document it, so that two schemas can be
/// compared on what they constrain.
fn essentials(schema: &Value) -> Value {
    match schema {
        Value::Object(members) => Value::Object(
            members
                .iter()
                .filter(|(name, _)| !matches!(name.as_str(), "$schema" | "title" | "description"))
                .map(|(name, value)| (name.clone(), essentials(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(essentials).collect()),
        other => other.clone(),
    }
}

/// A recorded answer with the span taken out of every mutation.
///
/// The recording is the measurement's, and the measurement asked for byte
/// offsets, so every mutation in it carries a `span` that this version's schema
/// does not name and this version's reader refuses. Taking it out here is the
/// same delta the schema comparison above applies, on the answer side: what is
/// replayed is what a provider sends when it is asked what this adapter asks.
fn without_the_span(response: &Value) -> String {
    let mut response = response.clone();
    let said = response["choices"][0]["message"]["content"]
        .as_str()
        .expect("a recorded answer says something");
    let mut answer: Value = serde_json::from_str(said).unwrap();
    for mutant in answer["mutants"].as_array_mut().unwrap() {
        mutant.as_object_mut().unwrap().remove("span");
    }
    response["choices"][0]["message"]["content"] = Value::String(answer.to_string());
    response.to_string()
}

#[test]
fn an_answer_a_provider_really_sent_still_parses() {
    for fixture in [
        include_str!("fixtures/llm/recorded-response-overlaps.json"),
        include_str!("fixtures/llm/recorded-response-crosses-midnight.json"),
    ] {
        let recorded: Value = serde_json::from_str(fixture).unwrap();
        let provider = Provider::scripted(vec![Reply::ok(without_the_span(&recorded["response"]))]);
        let outcome = with_key(Some(FAKE_KEY), || {
            provider.generator().generate(&request(4)).unwrap()
        });
        assert_eq!(outcome.mutants.len(), 4);
        assert_eq!(
            outcome.model_resolved,
            recorded["model_resolved"].as_str().unwrap()
        );
        assert_eq!(
            outcome.attempts[0].usage.total,
            recorded["usage"]["total_tokens"].as_u64().unwrap()
        );
        for mutant in &outcome.mutants {
            assert!(!mutant.original.is_empty());
            assert!(!mutant.replacement.is_empty());
            assert!(!mutant.description.is_empty());
        }
    }
}

#[test]
fn the_span_a_provider_really_sent_is_a_defect_this_side_reports() {
    // The request no longer asks for a span and the schema it sends forbids one,
    // so an answer shaped like the measurement's is a model answering the wrong
    // question. This is that answer, verbatim, and what it costs is one corrected
    // ask rather than a caller handed offsets nobody asked for.
    let recorded: Value =
        serde_json::from_str(include_str!("fixtures/llm/recorded-response-overlaps.json")).unwrap();
    let provider = Provider::scripted(vec![
        Reply::ok(recorded["response"].to_string()),
        Reply::ok(envelope(&answer(4), "stop")),
    ]);
    let outcome = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    assert_eq!(provider.calls(), 2);
    let correction = provider.payload(1)["messages"][2]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        correction.contains("span"),
        "the model is told which field it sent that nobody asked for: {correction}"
    );
    assert_eq!(outcome.mutants.len(), 4);
    assert_eq!(outcome.attempts.len(), 2);
}

#[test]
fn the_key_goes_in_a_header_and_nowhere_else() {
    let provider = Provider::scripted(vec![Reply::ok(envelope(&answer(4), "stop"))]);
    with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap()
    });
    let whole = provider.request(0);
    let (head, body) = whole.split_once("\r\n\r\n").unwrap();
    assert!(
        head.contains(&format!("Bearer {FAKE_KEY}")),
        "the key travels as a header"
    );
    assert!(!body.contains(FAKE_KEY), "and never in the payload");
}

#[test]
fn nothing_a_failure_says_carries_the_key() {
    // A provider that echoes the key back is the case redaction exists for.
    let echoed = format!(r#"{{"error":{{"message":"key {FAKE_KEY} is not allowed"}}}}"#);
    let provider = Provider::scripted(vec![Reply::refusing(400, &echoed)]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    let said = error.to_string();
    let debugged = format!("{error:?}");
    assert!(!said.contains(FAKE_KEY), "{said}");
    assert!(!debugged.contains(FAKE_KEY), "{debugged}");
    assert!(
        said.contains("is not allowed"),
        "the rest of what it said is still useful: {said}"
    );
}

#[test]
fn a_provider_that_refuses_for_another_reason_quotes_it() {
    let provider = Provider::scripted(vec![Reply::refusing(
        500,
        r#"{"error":{"message":"upstream is unwell"}}"#,
    )]);
    let error = with_key(Some(FAKE_KEY), || {
        provider.generator().generate(&request(4)).unwrap_err()
    });
    assert!(matches!(error.kind, GenerateFailure::Rejected { .. }));
    let said = error.to_string();
    assert!(said.contains("500"), "{said}");
    assert!(said.contains("upstream is unwell"), "{said}");
}

/// One real call, skipped unless the environment names both a key and a model.
///
/// It is not part of any gate: nothing here can make a network call answer, so
/// the test exists to be run deliberately by someone who has a key, with
/// `TREMULA_LIVE_MODEL` set to the exact snapshot to ask.
#[test]
fn a_real_provider_answers_with_mutants() {
    let guard = scripted_provider::ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (Ok(_), Ok(model)) = (
        std::env::var(KEY_VARIABLE),
        std::env::var("TREMULA_LIVE_MODEL"),
    ) else {
        eprintln!("skipped: set {KEY_VARIABLE} and TREMULA_LIVE_MODEL to call a real provider");
        return;
    };
    let outcome = OpenAiGenerator::new(model)
        .generate(&request(4))
        .expect("a real provider answers");
    drop(guard);
    assert_eq!(outcome.mutants.len(), 4);
    assert!(!outcome.model_resolved.is_empty());
    assert!(outcome.attempts[0].usage.total > 0);
}
