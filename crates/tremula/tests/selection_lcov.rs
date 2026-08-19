//! What a coverage document says about a project's lines, and what it refuses to
//! say.
//!
//! Every document here is written by hand, because what is being tested is the
//! reading of one: a fixture produced by a coverage tool would prove that this
//! parser agrees with that tool's current output and nothing about the forms it is
//! promised to accept.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fmt::Write as _, path::Path};

use tempfile::TempDir;
use tremula::generate::selection::lcov::Coverage;

/// A document naming one file, with the lines and hits given.
fn about(file: &str, lines: &[(u32, u64)]) -> String {
    let mut document = format!("SF:{file}\n");
    for (line, hits) in lines {
        let _ = writeln!(document, "DA:{line},{hits}");
    }
    document.push_str("end_of_record\n");
    document
}

#[test]
fn a_document_that_names_its_files_absolutely_is_read_relative_to_the_project() {
    let project = TempDir::new().unwrap();
    let absolute = project.path().join("src/scheduling/overlap.py");
    let document = about(&absolute.to_string_lossy(), &[(4, 1), (5, 3)]);

    let coverage = Coverage::read(&document, project.path()).unwrap();

    assert!(coverage.knows("src/scheduling/overlap.py"));
    assert!(coverage.covered("src/scheduling/overlap.py", 4));
    assert!(coverage.covered("src/scheduling/overlap.py", 5));
    assert_eq!(coverage.outside_the_project(), 0);
}

/// One directory, two absolute spellings. A coverage tool prints whichever form the
/// process that ran it was handed, and a run whose project came off the command line
/// as the other one is the same project.
#[test]
fn the_other_absolute_spelling_of_the_project_is_the_same_project() {
    let project = TempDir::new().unwrap();
    let resolved = std::fs::canonicalize(project.path()).unwrap();
    let document = about(
        &resolved.join("src/overlap.py").to_string_lossy(),
        &[(7, 2)],
    );

    let coverage = Coverage::read(&document, project.path()).unwrap();

    assert!(
        coverage.covered("src/overlap.py", 7),
        "the resolved spelling of the project root names the project"
    );
    assert_eq!(coverage.outside_the_project(), 0);
}

/// `relative_files = true` is a setting a project may have, and it is the one form
/// where nothing has to be worked out at all.
#[test]
fn a_document_written_with_relative_files_is_read_as_it_stands() {
    let project = TempDir::new().unwrap();
    let document = about("./src/overlap.py", &[(3, 1)]);

    let coverage = Coverage::read(&document, project.path()).unwrap();

    assert!(coverage.covered("src/overlap.py", 3));
}

#[test]
fn a_line_the_document_measured_and_nothing_reached_is_not_covered() {
    let project = TempDir::new().unwrap();
    let document = about("src/overlap.py", &[(4, 1), (5, 0)]);

    let coverage = Coverage::read(&document, project.path()).unwrap();

    assert!(coverage.measured("src/overlap.py", 5));
    assert!(
        !coverage.covered("src/overlap.py", 5),
        "a measured line no test reached is the gap this exists to report"
    );
    assert!(
        !coverage.measured("src/overlap.py", 6),
        "a line the document never mentions is not a statement at all"
    );
}

/// A coverage tool writes far more than the lines. None of it is read, and a
/// document full of it must come out the same as one without.
#[test]
fn the_records_a_coverage_tool_writes_beside_the_lines_are_ignored() {
    let project = TempDir::new().unwrap();
    let document = concat!(
        "TN:\n",
        "SF:src/overlap.py\n",
        "FN:4,overlaps\n",
        "FNDA:3,overlaps\n",
        "FNF:1\n",
        "FNH:1\n",
        "BRDA:5,0,0,2\n",
        "BRF:2\n",
        "BRH:1\n",
        "DA:4,1\n",
        "DA:5,0\n",
        "LF:2\n",
        "LH:1\n",
        "end_of_record\n",
    );

    let coverage = Coverage::read(document, project.path()).unwrap();

    assert!(coverage.covered("src/overlap.py", 4));
    assert!(!coverage.covered("src/overlap.py", 5));
    assert!(!coverage.measured("src/overlap.py", 2), "FNF is not a line");
}

/// A document may say the same file twice, and each block is a partial account of
/// it: a line reached under either of them is a line a test reached.
#[test]
fn two_blocks_about_one_file_are_read_as_one() {
    let project = TempDir::new().unwrap();
    let document = format!(
        "{}{}",
        about("src/overlap.py", &[(4, 0), (5, 1)]),
        about("src/overlap.py", &[(4, 2), (6, 0)])
    );

    let coverage = Coverage::read(&document, project.path()).unwrap();

    assert!(
        coverage.covered("src/overlap.py", 4),
        "reached under the second block is reached"
    );
    assert!(coverage.covered("src/overlap.py", 5));
    assert!(coverage.measured("src/overlap.py", 6));
    assert!(!coverage.covered("src/overlap.py", 6));
}

/// A document covering more than this project — a monorepo's, or a dependency's —
/// is not an error. What it says about somebody else's file is left out, and counted,
/// because a reader who selected nothing has to be able to tell "nothing changed"
/// from "this document is about another tree".
#[test]
fn a_file_outside_the_project_is_left_out_and_counted() {
    let project = TempDir::new().unwrap();
    let elsewhere = Path::new("/somewhere/else/lib/thing.py");
    let document = format!(
        "{}{}",
        about(&elsewhere.to_string_lossy(), &[(2, 1)]),
        about("../sibling/thing.py", &[(3, 1)])
    );

    let coverage = Coverage::read(&document, project.path()).unwrap();

    assert!(!coverage.knows("lib/thing.py"));
    assert!(!coverage.knows("../sibling/thing.py"));
    assert_eq!(coverage.outside_the_project(), 2);
}

#[test]
fn a_line_record_that_cannot_be_read_stops_the_document() {
    let project = TempDir::new().unwrap();

    for unreadable in ["DA:4\n", "DA:four,1\n", "DA:4,many\n", "DA:,\n"] {
        let document = format!("SF:src/overlap.py\n{unreadable}end_of_record\n");

        let failure = Coverage::read(&document, project.path())
            .expect_err("a line record nobody can read is not a line record");

        assert!(
            failure.to_string().contains("line 2"),
            "the message says where to look: {failure}"
        );
    }
}

/// A line belongs to the file the block named. One outside any block belongs to
/// nothing, and reading it as belonging to the previous file would attribute one
/// file's coverage to another.
#[test]
fn a_line_record_before_any_file_stops_the_document() {
    let project = TempDir::new().unwrap();

    let failure = Coverage::read("DA:4,1\n", project.path())
        .expect_err("a line outside any file is not a line of any file");

    assert!(failure.to_string().contains("no file"), "{failure}");
}

#[test]
fn a_file_the_document_never_mentions_is_one_it_knows_nothing_about() {
    let project = TempDir::new().unwrap();
    let document = about("src/overlap.py", &[(4, 1)]);

    let coverage = Coverage::read(&document, project.path()).unwrap();

    assert!(!coverage.knows("src/sync.py"));
    assert!(!coverage.covered("src/sync.py", 4));
    assert!(!coverage.measured("src/sync.py", 4));
}

/// A file whose block is empty was still measured: the document says the file was
/// instrumented and no line of it is a statement, which is not the same thing as
/// saying nothing about the file.
#[test]
fn a_file_the_document_names_and_says_nothing_else_about_is_still_known() {
    let project = TempDir::new().unwrap();

    let coverage = Coverage::read("SF:src/__init__.py\nend_of_record\n", project.path()).unwrap();

    assert!(coverage.knows("src/__init__.py"));
    assert!(!coverage.measured("src/__init__.py", 1));
}
