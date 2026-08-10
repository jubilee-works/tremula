//! Which files a manifest may name, and where the filesystem actually sends
//! them. Both halves matter: a path can be spelled entirely inside the project
//! and still lead out of it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::Path};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tremula_contracts::{
    SCHEMA_VERSION,
    manifest::{Base, Language, Manifest, Mutant, Span},
};

use tremula::validation::{ValidationError, canonical_mutant_id, validate_manifest};

/// The file every manifest in these tests targets.
const TARGET: &str = "src/scheduling/overlap.py";
const SOURCE: &str = "def overlaps(start, other):\n    return start < other.end\n";
const ORIGINAL: &str = "start < other.end";
const REPLACEMENT: &str = "start <= other.end";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A project directory whose only source file is the manifest's target.
fn project() -> TempDir {
    let root = TempDir::new().unwrap();
    let path = root.path().join(TARGET);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, SOURCE).unwrap();
    root
}

fn manifest(mutants: Vec<Mutant>) -> Manifest {
    Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        language: Language::Python,
        base: Base { revision: None },
        mutants,
    }
}

/// A mutant that agrees with the project's file on every field. Each test below
/// changes only which file it names.
fn valid_mutant() -> Mutant {
    let start = u64::try_from(SOURCE.find(ORIGINAL).unwrap()).unwrap();
    let span = Span {
        start_byte: start,
        end_byte: start + u64::try_from(ORIGINAL.len()).unwrap(),
    };
    let base_file_sha256 = sha256_hex(SOURCE.as_bytes());
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

/// Re-derive the identifier after a test changed the file it names, so that the
/// identifier check is not the rule under test.
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
fn a_path_that_leaves_the_project_is_rejected() {
    let root = project();
    let mutant = Mutant {
        file: "../escape.py".to_owned(),
        ..valid_mutant()
    };
    assert!(matches!(
        reject(root.path(), vec![mutant]),
        ValidationError::PathEscapesProject { .. }
    ));
}

#[test]
fn an_absolute_path_is_rejected() {
    let root = project();
    let mutant = Mutant {
        file: "/etc/hosts".to_owned(),
        ..valid_mutant()
    };
    assert!(matches!(
        reject(root.path(), vec![mutant]),
        ValidationError::PathEscapesProject { .. }
    ));
}

/// A host's own path syntax is not the contract's. Each spelling here either
/// means something other than it says on a POSIX host, or is a second spelling
/// of a file that already has one — and a second spelling would derive a second
/// identifier for the same mutation.
#[test]
fn a_path_that_is_not_written_as_plain_posix_is_rejected() {
    for spelling in [
        r"..\escape.py",
        r"C:\pkg\mod.py",
        "src//overlap.py",
        "/src/overlap.py",
        "src/overlap.py/",
        "",
    ] {
        let root = project();
        let mutant = Mutant {
            file: spelling.to_owned(),
            ..valid_mutant()
        };

        let error = reject(root.path(), vec![mutant]);

        assert!(
            matches!(error, ValidationError::PathEscapesProject { .. }),
            "`{spelling}` was not rejected: {error}"
        );
    }
}

/// The path spelling is inside the project; where the filesystem sends it is
/// not. Mutating through the link would edit a file the project does not own,
/// and put it back from a snapshot that never covered it.
#[test]
fn a_target_that_is_a_symbolic_link_is_rejected() {
    let root = project();
    let outside = TempDir::new().unwrap();
    let real = outside.path().join("elsewhere.py");
    fs::write(&real, SOURCE).unwrap();
    let link = root.path().join("src/scheduling/linked.py");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mutant = with_canonical_id(Mutant {
        file: "src/scheduling/linked.py".to_owned(),
        ..valid_mutant()
    });

    let error = reject(root.path(), vec![mutant]);

    assert!(
        matches!(error, ValidationError::SymlinkTarget { .. }),
        "{error}"
    );
}

/// The root every path in a manifest is relative to. A root that does not resolve
/// is reported about the root rather than about a mutant, and it names the root it
/// could not resolve, because that is the value the reader passed and the only one
/// they can correct.
#[test]
fn a_project_root_that_does_not_resolve_is_named_in_the_message() {
    let root = project();
    let missing = root.path().join("no-such-directory");

    let error = reject(&missing, vec![valid_mutant()]);

    assert!(
        matches!(error, ValidationError::ProjectRootUnresolvable { .. }),
        "{error}"
    );
    let said = error.to_string();
    assert!(said.contains("cannot resolve the project root"), "{said}");
    assert!(
        said.contains(&missing.display().to_string()),
        "`{said}` does not say which root could not be resolved"
    );
    assert!(said.contains("exists and is readable"), "{said}");
}

/// The same escape one level up: the file itself is ordinary, but a directory on
/// the way to it is a link out of the project. Only resolving the whole path
/// catches this one.
#[test]
fn a_target_reached_through_a_linked_directory_is_rejected() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::create_dir_all(outside.path().join("scheduling")).unwrap();
    fs::write(outside.path().join("scheduling/overlap.py"), SOURCE).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("src")).unwrap();

    let error = reject(root.path(), vec![valid_mutant()]);

    assert!(
        matches!(error, ValidationError::TargetOutsideProject { .. }),
        "{error}"
    );
}
