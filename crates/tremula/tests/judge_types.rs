//! The judgement contract: what a model may answer, and what a caller reads.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use tremula::generate::{
    Attempt, Usage,
    judge::{
        Claim, EquivalenceJudge, JudgeError, Judgement, JudgementOutcome, JudgementRequest, Witness,
    },
    openai::judge::response_schema,
};

/// A judge that answers from a script, which is how everything above the seam is
/// tested without a provider.
struct Scripted(Judgement);

impl EquivalenceJudge for Scripted {
    fn judge(&self, _request: &JudgementRequest) -> Result<JudgementOutcome, JudgeError> {
        Ok(JudgementOutcome {
            judgement: self.0.clone(),
            model_resolved: "a-model-2026-01-01".to_owned(),
            attempts: vec![Attempt {
                usage: Usage {
                    prompt: 1,
                    completion: 2,
                    total: 3,
                },
                finish: Some("stop".to_owned()),
            }],
        })
    }
}

fn a_request() -> JudgementRequest {
    JudgementRequest {
        file: "ranges.py".to_owned(),
        source: "def overlaps(a, b):\n    return a < b\n".to_owned(),
        original: "a < b".to_owned(),
        replacement: "a <= b".to_owned(),
    }
}

#[test]
fn a_scripted_judge_answers_through_the_seam() {
    let judge = Scripted(Judgement {
        claim: Claim::Distinguishable,
        witness: Some(Witness {
            call: "overlaps(1, 1)".to_owned(),
            expect_original: "False".to_owned(),
            expect_mutant: "True".to_owned(),
        }),
    });
    let outcome = judge.judge(&a_request()).unwrap();
    assert_eq!(outcome.judgement.claim, Claim::Distinguishable);
    assert_eq!(
        outcome.judgement.witness.as_ref().map(|w| w.call.as_str()),
        Some("overlaps(1, 1)")
    );
    assert_eq!(outcome.attempts[0].usage.total, 3);
}

#[test]
fn a_claim_of_equivalence_carries_no_witness() {
    let judgement = Judgement {
        claim: Claim::Equivalent,
        witness: None,
    };
    assert_eq!(
        serde_json::to_value(&judgement).unwrap(),
        json!({"claim": "equivalent", "witness": null}),
        "the field stays in the document: the schema a provider enforces requires every member"
    );
}

#[test]
fn an_answer_that_carries_a_member_nobody_asked_for_is_refused() {
    let said = json!({
        "claim": "distinguishable",
        "witness": null,
        "confidence": 0.9,
    });
    assert!(
        serde_json::from_value::<Judgement>(said).is_err(),
        "the request forbids a member the schema does not name, and the reader holds the answer to it"
    );
}

#[test]
fn the_schema_a_provider_enforces_names_every_member_and_refuses_the_rest() {
    let schema = response_schema();
    assert_eq!(schema["type"], json!("object"));
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["required"], json!(["claim", "witness"]));
    let claim = &schema["properties"]["claim"];
    assert_eq!(claim["enum"], json!(["distinguishable", "equivalent"]));
    assert!(
        claim["description"]
            .as_str()
            .unwrap()
            .contains("`equivalent`: There is no such input."),
        "a value the model chooses between has to arrive with what it means"
    );
    // Strict server-side enforcement requires every property to be required, so
    // an optional member is spelled as one that may be null rather than absent.
    let witness = &schema["properties"]["witness"];
    assert_eq!(
        witness["type"],
        json!(["object", "null"]),
        "a judgement with no runnable witness has to be expressible"
    );
    assert_eq!(witness["additionalProperties"], json!(false));
    assert_eq!(
        witness["required"],
        json!(["call", "expect_mutant", "expect_original"])
    );
    assert!(
        witness.get("default").is_none(),
        "a strict provider fills nothing in, so a default is a keyword it may refuse"
    );
}
