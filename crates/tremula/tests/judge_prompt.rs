//! What a judgement asks a model, and what it refuses to ask.
//!
//! The judging prompt is a second prompt, not a variant of the generating one.
//! What it must never do is ask the model to decide equivalence on its own
//! authority: the answer it wants is an *input*, because an input can be run and
//! a claim cannot. These tests pin that, and they pin the shape of the answer the
//! request holds a model to.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula::generate::judge::{
    JudgementRequest,
    prompt::{PROMPT_VERSION, SYSTEM, assemble},
};

fn a_survivor() -> JudgementRequest {
    JudgementRequest {
        file: "ranges.py".to_owned(),
        source: "def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:\n    \"\"\"Whether two half-open minute ranges share a minute.\"\"\"\n    return start < other_end and other_start < end\n".to_owned(),
        original: "start < other_end".to_owned(),
        replacement: "start <= other_end".to_owned(),
    }
}

#[test]
fn one_survivor_assembles_into_one_prompt() {
    let prompt = assemble(&a_survivor());
    insta::assert_snapshot!(format!("{}\n---\n{}", prompt.system, prompt.user));
}

#[test]
fn the_prompt_shows_the_change_as_two_texts_rather_than_a_diff() {
    let prompt = assemble(&a_survivor());
    assert!(prompt.user.contains("start < other_end"));
    assert!(prompt.user.contains("start <= other_end"));
    assert!(
        !prompt.user.contains("---") && !prompt.user.contains("+++"),
        "a patch header would make the model read line numbers nobody sent"
    );
}

#[test]
fn nothing_asks_the_model_to_be_believed() {
    // The one measured fact this prompt exists downstream of: the witnesses a
    // model writes are fabricated often enough that a claim is worth nothing on
    // its own. So the prompt has to say the witness is executed, and it must not
    // promise that an answer of `equivalent` settles anything.
    assert!(
        SYSTEM.contains("is run"),
        "the prompt has to tell the model its witness will be executed"
    );
    for absent in ["trust", "we will accept", "final"] {
        assert!(
            !SYSTEM.to_lowercase().contains(absent),
            "`{absent}` promises the answer more standing than an execution check gives it"
        );
    }
}

#[test]
fn the_witness_is_asked_for_in_the_only_form_that_can_be_run() {
    assert!(
        SYSTEM.contains("literal"),
        "an argument that is not a literal is one the probe refuses to evaluate"
    );
    assert!(
        SYSTEM.contains("null"),
        "a model with no runnable witness has to be told to leave the field empty"
    );
}

#[test]
fn the_prompt_carries_a_version_of_its_own() {
    assert_eq!(PROMPT_VERSION, "1");
}
