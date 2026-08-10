//! The shape of a generation call: what survives being written down, and what
//! every failure tells the person who hit it.
//!
//! A failed call is still a paid call, so the ledger of attempts is part of the
//! failure and not something a caller has to reconstruct.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula::generate::{
    Attempt, CorrectionFeedback, CoveringTest, Defect, GenerateError, GenerateFailure,
    GeneratedMutant, GeneratedMutantsResponse, GenerationOutcome, GenerationRequest, Usage,
};

/// A request with everything optional filled in, so a round trip has something
/// to lose.
fn full_request() -> GenerationRequest {
    GenerationRequest {
        file: "src/ranges.py".to_owned(),
        source: "def overlaps(a, b):\n    return a < b\n".to_owned(),
        tests: vec![CoveringTest {
            file: "tests/test_ranges.py".to_owned(),
            source: "def test_it():\n    assert overlaps(0, 1)\n".to_owned(),
        }],
        excluded: vec!["\"\"\"Whether two ranges overlap.\"\"\"".to_owned()],
        mutant_count: 4,
        feedback: Some(CorrectionFeedback {
            defects: vec![Defect {
                defect: "original_is_ambiguous".to_owned(),
                detail: "found 2 times".to_owned(),
            }],
        }),
    }
}

/// One attempt's worth of ledger.
fn one_attempt() -> Attempt {
    Attempt {
        usage: Usage {
            prompt: 682,
            completion: 436,
            total: 1118,
        },
        finish: Some("stop".to_owned()),
    }
}

#[test]
fn a_request_survives_a_round_trip() {
    let request = full_request();
    let written = serde_json::to_string(&request).unwrap();
    let read: GenerationRequest = serde_json::from_str(&written).unwrap();
    assert_eq!(read, request);
}

#[test]
fn an_outcome_survives_a_round_trip() {
    let outcome = GenerationOutcome {
        mutants: vec![GeneratedMutant {
            file: "src/ranges.py".to_owned(),
            original: "a < b".to_owned(),
            replacement: "a <= b".to_owned(),
            description: "an off-by-one at the boundary".to_owned(),
        }],
        model_resolved: "gpt-5.2-2025-12-11".to_owned(),
        attempts: vec![one_attempt()],
    };
    let written = serde_json::to_string(&outcome).unwrap();
    let read: GenerationOutcome = serde_json::from_str(&written).unwrap();
    assert_eq!(read, outcome);
}

#[test]
fn a_request_with_nothing_optional_in_it_omits_every_optional_key() {
    let request = GenerationRequest {
        file: "src/ranges.py".to_owned(),
        source: "def overlaps(a, b):\n    return a < b\n".to_owned(),
        tests: Vec::new(),
        excluded: Vec::new(),
        mutant_count: 3,
        feedback: None,
    };
    let written = serde_json::to_string(&request).unwrap();
    assert!(!written.contains("tests"), "{written}");
    assert!(!written.contains("excluded"), "{written}");
    assert!(!written.contains("feedback"), "{written}");
    let read: GenerationRequest = serde_json::from_str(&written).unwrap();
    assert_eq!(read, request);
}

#[test]
fn the_response_envelope_reads_an_answer_that_says_only_what_was_asked_for() {
    let body = r#"{"mutants":[{"file":"ranges.py","original":"a < b","replacement":"a <= b","description":"why"}]}"#;
    let read: GeneratedMutantsResponse = serde_json::from_str(body).unwrap();
    assert_eq!(read.mutants.len(), 1);
    assert_eq!(read.mutants[0].original, "a < b");
    let written = serde_json::to_string(&read).unwrap();
    assert!(
        !written.contains("span"),
        "this contract does not carry a span: {written}"
    );
}

#[test]
fn the_response_envelope_refuses_a_member_the_request_forbade() {
    // The schema that goes out closes every object, so a member nobody named is
    // something the answer was told not to send. Reading it anyway would be this
    // side declining to hold the model to the contract it was handed.
    let with_a_span = r#"{"mutants":[{"file":"ranges.py","original":"a < b","span":{"start_byte":1,"end_byte":2},"replacement":"a <= b","description":"why"}]}"#;
    let refused = serde_json::from_str::<GeneratedMutantsResponse>(with_a_span)
        .expect_err("a mutation carrying a span is not this contract")
        .to_string();
    assert!(
        refused.contains("span"),
        "the defect names the field it found: {refused}"
    );
    let beside_the_mutants = r#"{"mutants":[],"notes":"something nobody asked for"}"#;
    let refused = serde_json::from_str::<GeneratedMutantsResponse>(beside_the_mutants)
        .expect_err("the root object is closed too")
        .to_string();
    assert!(
        refused.contains("notes"),
        "the defect names the field it found: {refused}"
    );
}

/// Every failure a caller can meet, with the words its message has to contain.
fn failures() -> Vec<(GenerateFailure, Vec<&'static str>)> {
    vec![
        (
            GenerateFailure::MissingCredential {
                variable: "OPENAI_API_KEY".to_owned(),
            },
            vec!["OPENAI_API_KEY", "set"],
        ),
        (
            GenerateFailure::CredentialRejected {
                variable: "OPENAI_API_KEY".to_owned(),
            },
            vec!["401", "OPENAI_API_KEY"],
        ),
        (
            GenerateFailure::Unreachable {
                reason: "operation timed out".to_owned(),
            },
            vec!["operation timed out", "try again"],
        ),
        (
            GenerateFailure::RateLimited { retried: false },
            vec!["429", "did not say", "quota"],
        ),
        (
            GenerateFailure::RateLimited { retried: true },
            vec!["429", "after the wait it asked for", "quota"],
        ),
        (
            GenerateFailure::Refused {
                reason: "I cannot help with that".to_owned(),
            },
            vec!["refused", "I cannot help with that", "ask about another"],
        ),
        (
            GenerateFailure::Truncated { limit: 2000 },
            vec!["2000", "stopped", "fewer mutants"],
        ),
        (GenerateFailure::NoAnswer, vec!["no answer", "run again"]),
        (
            GenerateFailure::Unreadable {
                reason: "expected value at line 1 column 1".to_owned(),
            },
            vec!["expected value at line 1 column 1", "one corrected retry"],
        ),
        (
            GenerateFailure::UnreadableEnvelope {
                reason: "expected value at line 1 column 1".to_owned(),
            },
            vec!["expected value at line 1 column 1", "chat completions"],
        ),
        (
            GenerateFailure::WrongCount {
                requested: 4,
                received: 2,
            },
            vec!["2", "4", "one corrected retry"],
        ),
        (
            GenerateFailure::Rejected {
                status: 500,
                detail: "upstream is unwell".to_owned(),
            },
            vec!["500", "upstream is unwell"],
        ),
    ]
}

#[test]
fn every_failure_names_its_cause_and_the_next_action() {
    for (failure, needles) in failures() {
        let rendered = failure.to_string();
        for needle in needles {
            assert!(
                rendered.contains(needle),
                "`{rendered}` does not mention `{needle}`"
            );
        }
    }
}

#[test]
fn no_failure_says_a_retry_happened_when_none_did() {
    // A generator owns two retries, one for a wait and one for a correction, and
    // several failures are reached without spending either. A message that
    // mentions a retry on one of those paths sends a reader looking for a second
    // attempt that is not on the ledger.
    let spent_nothing = [
        GenerateFailure::UnreadableEnvelope {
            reason: "expected value at line 1 column 1".to_owned(),
        },
        GenerateFailure::RateLimited { retried: false },
        GenerateFailure::NoAnswer,
        GenerateFailure::Truncated { limit: 2000 },
        GenerateFailure::Refused {
            reason: "I cannot help with that".to_owned(),
        },
    ];
    for failure in spent_nothing {
        let rendered = failure.to_string();
        assert!(
            !rendered.contains("retr"),
            "`{rendered}` speaks of a retry that nothing spent"
        );
    }
}

#[test]
fn a_failure_carries_the_attempts_it_was_charged_for() {
    let error = GenerateError {
        kind: GenerateFailure::Truncated { limit: 2000 },
        attempts: vec![one_attempt(), one_attempt()],
    };
    assert_eq!(error.attempts.len(), 2);
    assert_eq!(error.attempts[0].usage.total, 1118);
    assert_eq!(error.to_string(), error.kind.to_string());
}
