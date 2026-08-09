//! Language-neutral manifest validation, one test per rule: every check the
//! core performs against the bytes on disk, the order those checks run in, and
//! the canonical identifier derivation. The wording of the failures is pinned
//! separately, in `validation_messages.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::Path};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula_contracts::{
    SCHEMA_VERSION,
    manifest::{Base, Language, Manifest, Mutant, Span},
};

use tremula::validation::{
    ValidationError, ValidationWarning, canonical_mutant_id, validate_manifest,
};

/// The file every manifest in these tests targets.
const TARGET: &str = "src/scheduling/overlap.py";
const SOURCE: &str = "def overlaps(start, other):\n    return start < other.end\n";
const ORIGINAL: &str = "start < other.end";
const REPLACEMENT: &str = "start <= other.end";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A project directory whose only source file holds `contents`.
fn write_target(contents: &[u8]) -> TempDir {
    let root = TempDir::new().unwrap();
    let path = root.path().join(TARGET);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, contents).unwrap();
    root
}

fn project() -> TempDir {
    write_target(SOURCE.as_bytes())
}

fn manifest(mutants: Vec<Mutant>) -> Manifest {
    Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        language: Language::Python,
        base: Base { revision: None },
        mutants,
    }
}

/// A mutant that agrees with `source` on every field, including its
/// canonical identifier. Each test below breaks exactly one agreement.
fn mutant_for(source: &str) -> Mutant {
    let start = u64::try_from(source.find(ORIGINAL).unwrap()).unwrap();
    let span = Span {
        start_byte: start,
        end_byte: start + u64::try_from(ORIGINAL.len()).unwrap(),
    };
    let base_file_sha256 = sha256_hex(source.as_bytes());
    Mutant {
        id: canonical_mutant_id(TARGET, &span, &base_file_sha256, REPLACEMENT),
        file: TARGET.to_owned(),
        base_file_sha256,
        span,
        original: ORIGINAL.to_owned(),
        replacement: REPLACEMENT.to_owned(),
        description: None,
        provenance: serde_json::Map::new(),
    }
}

fn valid_mutant() -> Mutant {
    mutant_for(SOURCE)
}

/// Re-derive the identifier after a test changed a field that feeds it, so
/// that the identifier check is not the rule under test.
fn with_canonical_id(mut mutant: Mutant) -> Mutant {
    mutant.id = canonical_mutant_id(
        &mutant.file,
        &mutant.span,
        &mutant.base_file_sha256,
        &mutant.replacement,
    );
    mutant
}

fn reject(root: &Path, mutants: Vec<Mutant>) -> ValidationError {
    validate_manifest(&manifest(mutants), root).expect_err("this manifest must be rejected")
}

#[test]
fn an_empty_mutant_list_is_valid() {
    let root = project();
    let verified = validate_manifest(&manifest(vec![]), root.path()).unwrap();
    assert!(
        verified.warnings.is_empty(),
        "nothing to test is a valid manifest"
    );
}

#[test]
fn a_mutant_that_agrees_with_its_file_is_valid() {
    let root = project();
    let verified = validate_manifest(&manifest(vec![valid_mutant()]), root.path()).unwrap();
    assert!(verified.warnings.is_empty());
}

/// Provenance is outside the identifier's derivation, which is what lets a
/// generator record how it worked — and later record it differently — without
/// changing what the mutation is. The keys are the ones the pack protocol
/// documents, so the case that proves the exclusion is the case a generator
/// really writes.
#[test]
fn recording_how_a_mutant_was_generated_leaves_its_identifier_alone() {
    let root = project();
    let bare = valid_mutant();
    let mut recorded = valid_mutant();
    recorded.provenance = serde_json::json!({
        "generator": {
            "name": "tremula-generate",
            "version": "0.1.0",
            "model": "example-model-2026-05-01",
            "prompt_version": "3",
            "generated_at": "2026-08-10T09:12:44Z"
        },
        "response_tokens": 148
    })
    .as_object()
    .unwrap()
    .clone();

    let verified = validate_manifest(&manifest(vec![recorded.clone()]), root.path()).unwrap();

    assert!(verified.warnings.is_empty());
    assert_eq!(
        recorded.id, bare.id,
        "the identifier is derived from what the mutation is, and provenance is not part of that"
    );
}

#[test]
fn a_missing_target_file_is_rejected() {
    let root = TempDir::new().unwrap();
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::FileMissing { .. }
    ));
}

/// A directory standing where the target should be is the portable way to make
/// a read fail for a reason other than absence.
#[test]
fn a_target_that_exists_but_cannot_be_read_names_the_real_cause() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join(TARGET)).unwrap();

    let error = reject(root.path(), vec![valid_mutant()]);

    assert!(
        matches!(error, ValidationError::FileUnreadable { .. }),
        "{error}"
    );
    assert!(
        !error.to_string().contains("does not exist"),
        "a file that is present must not be reported as missing: {error}"
    );
}

#[test]
fn a_file_that_changed_since_the_manifest_was_generated_is_rejected() {
    let root = project();
    let mutant = Mutant {
        base_file_sha256: sha256_hex(b"an older revision of the file"),
        ..valid_mutant()
    };
    let error = reject(root.path(), vec![mutant]);
    assert!(matches!(error, ValidationError::StaleFile { .. }));
    assert!(error.to_string().contains("regenerate the manifest"));
}

#[test]
fn a_span_that_does_not_fit_inside_the_file_is_rejected() {
    let root = project();
    let mutant = Mutant {
        span: Span {
            start_byte: 0,
            end_byte: 100_000,
        },
        ..valid_mutant()
    };
    assert!(matches!(
        reject(root.path(), vec![mutant]),
        ValidationError::SpanOutOfBounds { .. }
    ));
}

#[test]
fn an_empty_span_is_rejected() {
    let root = project();
    let mutant = Mutant {
        span: Span {
            start_byte: 10,
            end_byte: 10,
        },
        ..valid_mutant()
    };
    assert!(matches!(
        reject(root.path(), vec![mutant]),
        ValidationError::EmptySpan { .. }
    ));
}

#[test]
fn a_span_that_does_not_hold_the_original_text_is_rejected() {
    let root = project();
    let mutant = Mutant {
        original: "start > other.end".to_owned(),
        ..valid_mutant()
    };
    assert!(matches!(
        reject(root.path(), vec![mutant]),
        ValidationError::OriginalMismatch { .. }
    ));
}

#[test]
fn a_replacement_identical_to_the_original_is_rejected() {
    let root = project();
    let mutant = Mutant {
        replacement: ORIGINAL.to_owned(),
        ..valid_mutant()
    };
    assert!(matches!(
        reject(root.path(), vec![mutant]),
        ValidationError::IdenticalReplacement { .. }
    ));
}

/// A carriage return has to be refused here, because nothing downstream can.
/// The Python pack's round-trip check cannot see one — its parser keeps a `\r`
/// as ordinary text — while the backend reads and writes source with universal
/// newline translation and would turn it into a `\n` on the way out.
#[test]
fn a_replacement_containing_a_carriage_return_is_rejected() {
    for spelling in ["x = 2\r\n", "\r", "start <= other.end\r", "a\rb"] {
        let root = project();
        let mutant = with_canonical_id(Mutant {
            replacement: spelling.to_owned(),
            ..valid_mutant()
        });

        let error = reject(root.path(), vec![mutant]);

        assert!(
            matches!(error, ValidationError::CarriageReturnInReplacement { .. }),
            "{spelling:?} was not rejected: {error}"
        );
    }
}

#[test]
fn a_replacement_with_lf_line_endings_is_accepted() {
    let root = project();
    let mutant = with_canonical_id(Mutant {
        replacement: "if start:\n        return other.end".to_owned(),
        ..valid_mutant()
    });

    let verified = validate_manifest(&manifest(vec![mutant]), root.path()).unwrap();

    assert!(verified.warnings.is_empty());
}

#[test]
fn an_identifier_that_is_not_the_canonical_derivation_is_rejected() {
    let root = project();
    let derived = valid_mutant().id;
    let mutant = Mutant {
        id: "0".repeat(64),
        ..valid_mutant()
    };
    let error = reject(root.path(), vec![mutant]);
    assert!(matches!(error, ValidationError::IdMismatch { .. }));
    assert!(
        error.to_string().contains(&derived),
        "the message must carry the identifier the mutant's fields derive"
    );
}

#[test]
fn the_same_mutant_listed_twice_is_rejected() {
    let root = project();
    assert!(matches!(
        reject(root.path(), vec![valid_mutant(), valid_mutant()]),
        ValidationError::DuplicateId { .. }
    ));
}

#[test]
fn a_target_file_that_is_not_utf8_is_rejected() {
    // Latin-1 "café": no byte-order mark and no coding cookie announces it.
    let root = write_target(b"caf\xe9");
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::NotUtf8 { .. }
    ));
}

#[test]
fn a_target_file_with_a_byte_order_mark_is_rejected() {
    let root = write_target(b"\xef\xbb\xbfvalue = 1\n");
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::UnsupportedEncoding { .. }
    ));
}

#[test]
fn a_target_file_with_crlf_line_endings_is_rejected() {
    let root = write_target(b"line1\r\nline2\n");
    let error = reject(root.path(), vec![valid_mutant()]);
    assert!(matches!(
        error,
        ValidationError::UnsupportedLineEndings { .. }
    ));
    assert!(error.to_string().contains("convert the file to LF"));
}

#[test]
fn a_target_file_declaring_a_non_utf8_encoding_is_rejected() {
    let root = write_target(b"# -*- coding: latin-1 -*-\nvalue = 1\n");
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::UnsupportedEncoding { .. }
    ));
}

/// Encoding is settled before line endings, so a file that breaks both rules
/// reports the one that has to be fixed first.
#[test]
fn a_file_with_both_a_byte_order_mark_and_crlf_reports_the_byte_order_mark() {
    let root = write_target(b"\xef\xbb\xbfline1\r\nline2\n");
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::UnsupportedEncoding { .. }
    ));
}

#[test]
fn a_file_that_is_neither_utf8_nor_lf_reports_that_it_is_not_utf8() {
    let root = write_target(b"caf\xe9\r\n");
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::NotUtf8 { .. }
    ));
}

#[test]
fn a_coding_cookie_written_with_an_equals_sign_is_rejected() {
    let root = write_target(b"# coding=latin-1\nvalue = 1\n");
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::UnsupportedEncoding { .. }
    ));
}

#[test]
fn a_coding_cookie_on_the_second_line_is_rejected() {
    let root = write_target(b"#!/usr/bin/env python\n# -*- coding: latin-1 -*-\nvalue = 1\n");
    assert!(matches!(
        reject(root.path(), vec![valid_mutant()]),
        ValidationError::UnsupportedEncoding { .. }
    ));
}

/// A declaration only counts in the first two lines; below that it is a
/// comment like any other.
#[test]
fn a_coding_cookie_below_the_second_line_is_not_a_declaration() {
    let source =
        format!("#!/usr/bin/env python\n# nothing to declare\n# -*- coding: latin-1 -*-\n{SOURCE}");
    let root = write_target(source.as_bytes());

    let verified = validate_manifest(&manifest(vec![mutant_for(&source)]), root.path()).unwrap();

    assert!(verified.warnings.is_empty());
}

/// Only comment lines can declare an encoding, so the same words inside a
/// string literal are just a string.
#[test]
fn the_word_coding_inside_a_string_literal_is_not_a_declaration() {
    let source = format!("label = \"coding: latin-1\"\n{SOURCE}");
    let root = write_target(source.as_bytes());

    let verified = validate_manifest(&manifest(vec![mutant_for(&source)]), root.path()).unwrap();

    assert!(verified.warnings.is_empty());
}

#[test]
fn a_coding_cookie_that_spells_utf8_differently_is_accepted() {
    let source = format!("# -*- coding: UTF_8 -*-\n{SOURCE}");
    let root = write_target(source.as_bytes());
    let verified = validate_manifest(&manifest(vec![mutant_for(&source)]), root.path()).unwrap();
    assert!(
        verified.warnings.is_empty(),
        "utf-8 spelled with an underscore is still utf-8"
    );
}

#[test]
fn an_oversized_replacement_warns_instead_of_failing() {
    let root = project();
    let replacement = "x".repeat(10 * 1024 + 1);
    let mutant = with_canonical_id(Mutant {
        replacement: replacement.clone(),
        ..valid_mutant()
    });
    let verified = validate_manifest(&manifest(vec![mutant]), root.path()).unwrap();
    assert!(matches!(
        verified.warnings.as_slice(),
        [ValidationWarning::LargeReplacement { bytes, .. }] if *bytes == replacement.len()
    ));
}

/// The derivation is a cross-producer contract, so this expectation is
/// computed outside this code base:
///
/// ```text
/// echo -ne 'tremula/mutant/v1\x00src/scheduling/overlap.py\x00412\x00431\x007d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b8c0d2e\x00start <= other.end' | shasum -a 256
/// ```
#[test]
fn canonical_identifiers_follow_the_documented_derivation() {
    let span = Span {
        start_byte: 412,
        end_byte: 431,
    };
    assert_eq!(
        canonical_mutant_id(
            "src/scheduling/overlap.py",
            &span,
            "7d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b8c0d2e",
            "start <= other.end",
        ),
        "7c00b65a07e206cc7a472043b4a74359ca09e404db1dc431b06bda48e80ca829"
    );
}

/// A second vector over the parts another producer is most likely to encode
/// differently: multibyte text in the path and in the replacement, and a
/// deletion, whose replacement is the empty string. Both expectations are
/// computed outside this code base:
///
/// ```text
/// echo -ne 'tremula/mutant/v1\x00src/일정/겹침.py\x007\x0031\x004a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b\x00return True  # 항상 참' | shasum -a 256
/// echo -ne 'tremula/mutant/v1\x00src/일정/겹침.py\x007\x0031\x004a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b\x00' | shasum -a 256
/// ```
#[test]
fn canonical_identifiers_cover_multibyte_text_and_deletions() {
    let span = Span {
        start_byte: 7,
        end_byte: 31,
    };
    let file = "src/일정/겹침.py";
    let base_file_sha256 = "4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b";

    assert_eq!(
        canonical_mutant_id(file, &span, base_file_sha256, "return True  # 항상 참"),
        "adea87810c4439bf31615e62d5f4886ed3fcd217cb63bfda049698eb46697972"
    );
    assert_eq!(
        canonical_mutant_id(file, &span, base_file_sha256, ""),
        "ff897c25e25ea68af3bbaa2c52d02af1e981cf3eee3b5201bf610faa688641f7"
    );
}
