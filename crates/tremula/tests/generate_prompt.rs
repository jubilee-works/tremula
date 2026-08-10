//! What tremula asks a model for, and how that differs from what was measured.
//!
//! `tests/fixtures/llm/measured-prompt-v0.txt` is the whole prompt of the run
//! that was measured against `gpt-5.2-2025-12-11`: the system prompt, a rule of
//! seventy hyphens, then the user prompt for one function shown with the tests
//! that cover it. It is the reproduction canon — the archive constants are
//! checked against it rather than against each other, and the prompt this
//! version sends is checked against the archive, delta by delta.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula::generate::{
    CorrectionFeedback, CoveringTest, Defect, GenerationRequest,
    prompt::{PROMPT_VERSION, SYSTEM, archive, assemble, correction},
};

/// What separates the two halves of the archived prompt.
const RULE: &str = "----------------------------------------------------------------------";

/// The archived prompt, as its system half and its user half.
fn archived_halves() -> (String, String) {
    let text = include_str!("fixtures/llm/measured-prompt-v0.txt");
    let separator = format!("\n{RULE}\n");
    let (system, user) = text
        .split_once(&separator)
        .expect("the archived prompt separates its two halves with a rule of hyphens");
    (system.to_owned(), user.to_owned())
}

/// A request about the same function the archive shows, with its test file.
fn overlaps_request() -> GenerationRequest {
    GenerationRequest {
        file: "ranges.py".to_owned(),
        source: "def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:\n    \"\"\"Whether two half-open minute ranges share a minute.\"\"\"\n    return start < other_end and other_start < end\n".to_owned(),
        tests: vec![CoveringTest {
            file: "test_ranges.py".to_owned(),
            source: "from ranges import overlaps\n\n\ndef test_ranges_that_only_touch_do_not_overlap() -> None:\n    assert not overlaps(0, 30, 30, 60)\n".to_owned(),
        }],
        mutant_count: 4,
        feedback: None,
    }
}

/// The literal stretches of a template, in the order it spells them.
fn literal_parts(template: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut inside = false;
    for character in template.chars() {
        match character {
            '{' => {
                inside = true;
                if !literal.is_empty() {
                    parts.push(std::mem::take(&mut literal));
                }
            }
            '}' => inside = false,
            _ if !inside => literal.push(character),
            _ => {}
        }
    }
    if !literal.is_empty() {
        parts.push(literal);
    }
    parts
}

#[test]
fn the_archived_system_prompt_is_the_one_that_was_measured() {
    let (system, _) = archived_halves();
    assert_eq!(archive::SYSTEM, system);
}

#[test]
fn the_archived_user_template_spells_the_prompt_that_was_measured() {
    let (_, user) = archived_halves();
    let mut rest = user.as_str();
    for part in literal_parts(archive::USER_TEMPLATE) {
        let found = rest.find(&part).unwrap_or_else(|| {
            panic!("the archived prompt has no `{part}` where the template does")
        });
        rest = &rest[found + part.len()..];
    }
}

#[test]
fn three_of_the_four_changes_are_lines_of_the_system_prompt() {
    let archived: Vec<&str> = archive::SYSTEM.lines().collect();
    let sent: Vec<&str> = SYSTEM.lines().collect();
    let dropped: Vec<&str> = archived
        .iter()
        .filter(|line| !sent.contains(line))
        .copied()
        .collect();
    let added: Vec<&str> = sent
        .iter()
        .filter(|line| !archived.contains(line))
        .copied()
        .collect();
    assert_eq!(
        dropped,
        vec![
            "- `span` is a half-open byte range into the whole file: 0-indexed, end",
            "  exclusive, counted in UTF-8 bytes.",
            "- `original` is exactly the text those bytes hold.",
            "- `replacement` is valid Python that parses on its own as a single node, with no",
            "  comment and no blank line around it.",
            "- The span is never empty. A pure insertion cannot be expressed.",
        ]
    );
    assert_eq!(
        added,
        vec![
            "- `original` is verbatim contiguous text that can be found exactly once inside",
            "  the function's body.",
            "- `replacement` is valid Python, with no comment and no blank line around it.",
        ]
    );
}

#[test]
fn the_fourth_change_is_the_word_the_correction_turn_uses() {
    // The three changes above are in the system prompt. The fourth is not in it
    // at all: the turn that goes back with a defect list says "the contract"
    // where the measured one said "the schema", because the list now also carries
    // defects found after the schema was satisfied. It is a change to what a
    // model is told, so it is pinned like the rest of what a model is told.
    let sentence = "That response did not satisfy the contract.";
    let asked_again = correction(&[Defect {
        defect: "original_not_found".to_owned(),
        detail: "no such text in the function".to_owned(),
    }]);
    assert!(asked_again.starts_with(sentence), "{asked_again}");
    assert!(
        !asked_again.contains("schema"),
        "the measured wording said schema, and this one deliberately does not: {asked_again}"
    );
    assert_eq!(
        correction(&[])
            .split_once(" Problems:")
            .map(|(said, _)| said),
        Some(sentence),
        "an empty list is still that sentence and still a message"
    );
}

#[test]
fn nothing_that_is_sent_asks_for_a_byte_span() {
    let prompt = assemble(&overlaps_request());
    let whole = format!("{}\n{}", prompt.system, prompt.user);
    for forbidden in ["span", "byte", "offset", "0-indexed"] {
        assert!(
            !whole.to_lowercase().contains(forbidden),
            "the prompt still says `{forbidden}`"
        );
    }
}

#[test]
fn a_function_and_its_tests_assemble_into_one_prompt() {
    let prompt = assemble(&overlaps_request());
    assert_eq!(prompt.system, SYSTEM);
    assert!(prompt.correction.is_none());
    insta::assert_snapshot!(format!("{}\n{RULE}\n{}", prompt.system, prompt.user));
}

#[test]
fn a_function_without_tests_omits_the_paragraph_about_them() {
    let mut request = overlaps_request();
    request.tests.clear();
    let prompt = assemble(&request);
    assert!(!prompt.user.contains("These are the tests"));
    assert!(
        prompt
            .user
            .ends_with("Produce 4 mutations of the function above.")
    );
}

#[test]
fn a_defect_list_rides_back_with_the_next_request() {
    let mut request = overlaps_request();
    request.feedback = Some(CorrectionFeedback {
        defects: vec![
            Defect {
                defect: "original_not_found".to_owned(),
                detail: "return start < other_end and other_start <= end".to_owned(),
            },
            Defect {
                defect: "original_is_ambiguous".to_owned(),
                detail: "found 2 times".to_owned(),
            },
        ],
    });
    let correction = assemble(&request)
        .correction
        .expect("feedback puts a correction on the prompt");
    assert!(correction.contains("original_not_found"));
    assert!(correction.contains("return start < other_end and other_start <= end"));
    assert!(correction.contains("original_is_ambiguous"));
    assert!(correction.contains("found 2 times"));
    assert!(
        correction.contains(r#"[{"defect":"original_not_found""#),
        "the defects travel as the JSON list the model was told to expect: {correction}"
    );
    assert!(
        correction.ends_with("Answer again, with the same JSON object shape and nothing else.")
    );
}

#[test]
fn the_prompt_version_is_the_one_a_mutant_records() {
    assert_eq!(PROMPT_VERSION, "1");
    assert_eq!(archive::VERSION, "0");
}
