//! The wording of every manifest defect a user can hit. These strings are
//! part of the tool's public surface: each one has to name the cause, and where
//! the reader can do something about it, the next action.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula::validation::ValidationError;

/// The mutant identifier every message below is built with.
const MESSAGE_ID: &str = "3b1f8c";
/// The path every message below is built with.
const MESSAGE_FILE: &str = "src/pkg/mod.py";

/// Failures about the target file itself.
fn messages_about_the_file() -> Vec<(ValidationError, Vec<&'static str>)> {
    let id = MESSAGE_ID.to_owned();
    let file = MESSAGE_FILE.to_owned();
    vec![
        (
            ValidationError::PathEscapesProject {
                mutant_id: id.clone(),
                file: file.clone(),
            },
            vec!["not a plain relative path", "relative to the project root"],
        ),
        (
            ValidationError::FileMissing {
                mutant_id: id.clone(),
                file: file.clone(),
            },
            vec!["does not exist", "project root"],
        ),
        (
            ValidationError::FileUnreadable {
                mutant_id: id.clone(),
                file: file.clone(),
                reason: "Is a directory".to_owned(),
            },
            vec![
                "cannot read target file",
                "Is a directory",
                "readable regular file",
            ],
        ),
        (
            ValidationError::StaleFile {
                mutant_id: id.clone(),
                file: file.clone(),
            },
            vec!["has changed since the manifest was generated", "regenerate"],
        ),
        (
            ValidationError::NotUtf8 {
                mutant_id: id.clone(),
                file: file.clone(),
            },
            vec!["not valid UTF-8", "only plain UTF-8 sources are supported"],
        ),
        (
            ValidationError::UnsupportedEncoding {
                mutant_id: id.clone(),
                file: file.clone(),
            },
            vec!["byte-order mark", "coding cookie", "only plain UTF-8"],
        ),
        (
            ValidationError::UnsupportedLineEndings {
                mutant_id: id.clone(),
                file: file.clone(),
            },
            vec!["CRLF line endings", "convert the file to LF"],
        ),
    ]
}

/// Failures about the mutation the manifest describes.
fn messages_about_the_mutation() -> Vec<(ValidationError, Vec<&'static str>)> {
    let id = MESSAGE_ID.to_owned();
    let file = MESSAGE_FILE.to_owned();
    vec![
        (
            ValidationError::SpanOutOfBounds {
                mutant_id: id.clone(),
                start: 0,
                end: 100,
                file: file.clone(),
                len: 40,
            },
            vec!["does not fit inside", "40 bytes"],
        ),
        (
            ValidationError::EmptySpan {
                mutant_id: id.clone(),
                start: 10,
                end: 10,
            },
            vec!["span is empty", "insertions are not supported"],
        ),
        (
            ValidationError::OriginalMismatch {
                mutant_id: id.clone(),
            },
            vec!["do not match", "regenerate the manifest"],
        ),
        (
            ValidationError::IdenticalReplacement {
                mutant_id: id.clone(),
            },
            vec!["identical to the original", "must change the code"],
        ),
        (
            ValidationError::IdMismatch {
                mutant_id: id.clone(),
                expected: "beef".to_owned(),
            },
            vec!["does not match the canonical derivation", "recompute it"],
        ),
        (
            ValidationError::DuplicateId { id: id.clone() },
            vec!["appears more than once", "must be unique"],
        ),
    ]
}

/// Every message is a public surface: it names the cause and, where the
/// reader can act, the next action.
#[test]
fn every_error_names_its_cause_and_the_next_action() {
    let cases = messages_about_the_file()
        .into_iter()
        .chain(messages_about_the_mutation());
    for (error, needles) in cases {
        let rendered = error.to_string();
        assert!(
            rendered.contains(MESSAGE_ID),
            "`{rendered}` does not say which mutant it is about"
        );
        for needle in needles {
            assert!(
                rendered.contains(needle),
                "`{rendered}` does not mention `{needle}`"
            );
        }
    }
}
