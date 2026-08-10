//! Turning what a model said into a mutant a manifest can carry.
//!
//! A model is asked for the *text* to replace and never for a byte offset,
//! because a measurement of 144 proposals found the offsets right in under one
//! per cent of them and the text findable in four out of five. So the address is
//! settled here instead, against the file's own bytes — and only when the text
//! occurs exactly once in the part of the function a mutation is allowed to touch.
//! Anything else is a defect the model is told about rather than a guess made on
//! its behalf: the measured sweep had a search pick the first of two matches, and
//! it landed in a parameter list and produced a function with two parameters of
//! the same name.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula::{
    generate::{
        GeneratedMutant,
        enrich::{
            CARRIAGE_RETURN_IN_REPLACEMENT, NUL_IN_REPLACEMENT, ORIGINAL_AMBIGUOUS,
            ORIGINAL_NOT_FOUND, REPLACEMENT_UNCHANGED, SourceFile, Stamp, Target, WRONG_FILE,
            enrich, with_the_closing_newline,
        },
        prompt::PROMPT_VERSION,
    },
    validation::canonical_mutant_id,
};
use tremula_contracts::{
    manifest::Span,
    spans::{ExcludedKind, ExcludedSpan, FunctionSpan},
};

/// The file every case below is about.
///
/// It has what a search has to cope with: a docstring, a function nested inside
/// another, one text that occurs twice in the outer body, and one that occurs
/// once.
const SOURCE: &str = r#""""Half-open ranges."""


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    """Whether two half-open ranges share a minute."""

    def touching() -> bool:
        return start == other_end

    if touching():
        return False
    if start > end:
        return False
    return start < other_end and other_start < end
"#;

const FILE: &str = "src/ranges.py";

fn source() -> SourceFile {
    SourceFile::new(FILE, SOURCE.as_bytes().to_vec())
}

/// Where `text` sits in the fixture, which must spell it exactly once.
fn span_of(text: &str) -> Span {
    let at = SOURCE
        .find(text)
        .unwrap_or_else(|| panic!("the fixture has no `{text}`"));
    assert_eq!(
        SOURCE.rfind(text),
        Some(at),
        "the fixture spells `{text}` more than once"
    );
    Span {
        start_byte: u64::try_from(at).unwrap(),
        end_byte: u64::try_from(at + text.len()).unwrap(),
    }
}

/// A span as a range this host can slice with.
fn text_of(span: Span) -> std::ops::Range<usize> {
    usize::try_from(span.start_byte).unwrap()..usize::try_from(span.end_byte).unwrap()
}

/// The whole of `overlaps`, as the language pack would report it.
fn overlaps() -> FunctionSpan {
    let last_statement = span_of("return start < other_end and other_start < end");
    FunctionSpan {
        qualified_name: "overlaps".to_owned(),
        span: Span {
            start_byte: span_of("def overlaps").start_byte,
            end_byte: last_statement.end_byte,
        },
        body_span: Span {
            start_byte: span_of(r#""""Whether two half-open ranges"#).start_byte,
            end_byte: last_statement.end_byte,
        },
        excluded: vec![ExcludedSpan {
            kind: ExcludedKind::Docstring,
            span: span_of(r#""""Whether two half-open ranges share a minute.""""#),
        }],
    }
}

/// The nested function, reported against its own entry.
fn touching() -> FunctionSpan {
    FunctionSpan {
        qualified_name: "overlaps.<locals>.touching".to_owned(),
        span: Span {
            start_byte: span_of("def touching").start_byte,
            end_byte: span_of("return start == other_end").end_byte,
        },
        body_span: span_of("return start == other_end"),
        excluded: Vec::new(),
    }
}

/// Both functions of the file, which is what the report carries.
fn functions() -> Vec<FunctionSpan> {
    vec![overlaps(), touching()]
}

fn outer() -> Target {
    Target::new(&overlaps(), &functions())
}

fn stamp() -> Stamp {
    Stamp {
        model: "gpt-5.2-2025-12-11".to_owned(),
        generated_at: "2026-08-10T09:12:44Z".to_owned(),
    }
}

fn proposal(original: &str, replacement: &str) -> GeneratedMutant {
    GeneratedMutant {
        file: FILE.to_owned(),
        original: original.to_owned(),
        replacement: replacement.to_owned(),
        description: "an off-by-one at a boundary a test might not pin".to_owned(),
    }
}

fn refusal(original: &str, replacement: &str) -> String {
    enrich(
        &proposal(original, replacement),
        &source(),
        &outer(),
        &stamp(),
    )
    .expect_err("this proposal cannot become a mutant")
    .defect
}

#[test]
fn a_text_that_occurs_once_settles_the_span_and_the_identity() {
    let mutant = enrich(
        &proposal(
            "start < other_end and other_start < end",
            "start <= other_end and other_start < end",
        ),
        &source(),
        &outer(),
        &stamp(),
    )
    .unwrap();

    let expected = span_of("start < other_end and other_start < end");
    assert_eq!(mutant.span, expected);
    assert_eq!(mutant.file, FILE);
    assert_eq!(&SOURCE[text_of(expected)], mutant.original);
    assert_eq!(mutant.base_file_sha256, source().sha256);
    assert_eq!(
        mutant.id,
        canonical_mutant_id(FILE, &expected, &source().sha256, &mutant.replacement)
    );
    assert_eq!(
        mutant.description.as_deref(),
        Some("an off-by-one at a boundary a test might not pin")
    );
}

#[test]
fn a_text_the_function_does_not_hold_is_a_defect_rather_than_a_guess() {
    assert_eq!(
        refusal("start < other_start", "start <= other_start"),
        ORIGINAL_NOT_FOUND
    );
}

#[test]
fn a_text_that_occurs_twice_is_refused_and_neither_occurrence_is_chosen() {
    // The measured sweep's own accident: a search that took the first of two
    // matches put `start` into a parameter list and made a function with two
    // parameters of the same name, which the interpreter refuses outright.
    assert_eq!(SOURCE.matches("return False").count(), 2);

    assert_eq!(refusal("return False", "return True"), ORIGINAL_AMBIGUOUS);
}

#[test]
fn a_text_that_only_a_nested_function_holds_belongs_to_that_function() {
    // The pack protocol's ownership rule: a function's targets are its body less
    // the whole of every function inside it. `touching` owns its own line, and a
    // mutation of it is a mutation of `touching`.
    assert_eq!(
        refusal("start == other_end", "start != other_end"),
        ORIGINAL_NOT_FOUND
    );

    let inner = Target::new(&touching(), &functions());
    let mutant = enrich(
        &proposal("start == other_end", "start != other_end"),
        &source(),
        &inner,
        &stamp(),
    )
    .unwrap();
    assert_eq!(mutant.span, span_of("start == other_end"));
}

#[test]
fn a_text_inside_an_excluded_stretch_is_not_somewhere_a_mutation_may_land() {
    // Nothing evaluates a docstring, so a mutation there is one no test suite
    // could be blamed for missing.
    assert_eq!(
        refusal("share a minute", "share an hour"),
        ORIGINAL_NOT_FOUND
    );
}

#[test]
fn a_proposal_about_another_file_is_not_a_proposal_about_this_one() {
    let mut elsewhere = proposal("start < other_end and other_start < end", "False");
    elsewhere.file = "src/somewhere_else.py".to_owned();

    let defect = enrich(&elsewhere, &source(), &outer(), &stamp())
        .expect_err("a mutation of another file cannot be enriched against this one");

    assert_eq!(defect.defect, WRONG_FILE);
    assert!(
        defect.detail.contains("src/somewhere_else.py"),
        "{}",
        defect.detail
    );
}

#[test]
fn a_replacement_no_manifest_could_carry_never_becomes_a_mutant() {
    // Both of these would be refused later and worse: a carriage return by the
    // neutral validation of the whole manifest, and a NUL by nothing at all —
    // the identifier is derived over parts joined with NUL bytes, so a NUL inside
    // one of them makes two different mutations derive the same identifier.
    assert_eq!(
        refusal(
            "start < other_end and other_start < end",
            "start < other_end\r\n"
        ),
        CARRIAGE_RETURN_IN_REPLACEMENT
    );
    assert_eq!(
        refusal(
            "start < other_end and other_start < end",
            "start\0< other_end"
        ),
        NUL_IN_REPLACEMENT
    );
}

#[test]
fn a_replacement_that_is_the_original_changes_nothing_and_is_refused() {
    let text = "start < other_end and other_start < end";
    assert_eq!(refusal(text, text), REPLACEMENT_UNCHANGED);
}

#[test]
fn a_proposal_that_names_no_text_at_all_addresses_nothing() {
    assert_eq!(refusal("", "False"), ORIGINAL_NOT_FOUND);
}

#[test]
fn what_produced_a_mutant_is_recorded_under_the_published_keys() {
    let mutant = enrich(
        &proposal(
            "start < other_end and other_start < end",
            "start <= other_end and other_start < end",
        ),
        &source(),
        &outer(),
        &stamp(),
    )
    .unwrap();

    let generator = mutant.provenance["generator"].as_object().unwrap();
    assert_eq!(generator["name"], serde_json::json!("tremula-generate"));
    assert_eq!(
        generator["version"],
        serde_json::json!(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(generator["model"], serde_json::json!("gpt-5.2-2025-12-11"));
    assert_eq!(
        generator["prompt_version"],
        serde_json::json!(PROMPT_VERSION)
    );
    assert_eq!(
        generator["generated_at"],
        serde_json::json!("2026-08-10T09:12:44Z")
    );
    // Provenance is excluded from the identifier, so recording more of it cannot
    // change what the mutation is.
    assert_eq!(
        mutant.id,
        canonical_mutant_id(FILE, &mutant.span, &source().sha256, &mutant.replacement)
    );
}

#[test]
fn a_span_can_take_in_the_newline_that_closes_it_and_stays_its_own_original() {
    // A compound statement's node ends after the newline that closes it, so a
    // search that stopped before that newline found the right text at an extent
    // the backend cannot match. Taking the newline into the span *and* into
    // `original` is what keeps the two the same bytes, which every reader relies on.
    let mutant = enrich(
        &proposal(
            "start < other_end and other_start < end",
            "start <= other_end and other_start < end",
        ),
        &source(),
        &outer(),
        &stamp(),
    )
    .unwrap();

    let snapped = with_the_closing_newline(&mutant, &source()).expect("a newline closes that span");

    assert_eq!(snapped.span.end_byte, mutant.span.end_byte + 1);
    assert_eq!(snapped.original, format!("{}\n", mutant.original));
    assert_eq!(&SOURCE[text_of(snapped.span)], snapped.original);
    assert_ne!(
        snapped.id, mutant.id,
        "a different span is a different mutation"
    );
    assert_eq!(
        snapped.id,
        canonical_mutant_id(FILE, &snapped.span, &source().sha256, &snapped.replacement)
    );
}

#[test]
fn a_span_with_no_newline_after_it_has_nothing_to_take_in() {
    let mutant = enrich(
        &proposal("start < other_end", "start <= other_end"),
        &source(),
        &outer(),
        &stamp(),
    )
    .unwrap();

    assert_eq!(with_the_closing_newline(&mutant, &source()), None);
}
