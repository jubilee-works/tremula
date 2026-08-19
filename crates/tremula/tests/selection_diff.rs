//! Which lines a pull request added, read out of what `git diff --unified=0` says.
//!
//! The output here is real: each of these is a form git was observed to emit, and
//! the ones that look like edge cases — a hunk header with no count, a count of
//! zero, a rename with no hunk at all — are the forms that decide whether a file
//! ends up with candidate lines or with none.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;

use tremula::generate::selection::diff::changed;

/// The lines one file contributes, or nothing when the diff offers that file none.
fn lines_of(diff: &str, file: &str) -> Option<Vec<u32>> {
    changed(diff)
        .files
        .into_iter()
        .find(|found| found.file == file)
        .map(|found| found.lines)
}

#[test]
fn a_new_file_contributes_every_line_it_has() {
    let diff = concat!(
        "diff --git a/src/overlap.py b/src/overlap.py\n",
        "new file mode 100644\n",
        "index 0000000..8f1c2d3\n",
        "--- /dev/null\n",
        "+++ b/src/overlap.py\n",
        "@@ -0,0 +1,3 @@\n",
        "+def overlaps(a, b):\n",
        "+    return a < b\n",
        "+\n",
    );

    assert_eq!(lines_of(diff, "src/overlap.py"), Some(vec![1, 2, 3]));
}

/// A hunk that added exactly one line says so by leaving the count out. Reading the
/// absent count as zero would lose every single-line change in a pull request.
#[test]
fn a_hunk_with_no_count_added_one_line() {
    let diff = concat!(
        "diff --git a/src/overlap.py b/src/overlap.py\n",
        "--- a/src/overlap.py\n",
        "+++ b/src/overlap.py\n",
        "@@ -2 +2 @@\n",
        "-    return a < b\n",
        "+    return a <= b\n",
    );

    assert_eq!(lines_of(diff, "src/overlap.py"), Some(vec![2]));
}

/// A hunk that only removed lines has a count of zero on the new side, and the
/// number beside it is where the removal was rather than a line that now exists.
#[test]
fn a_hunk_that_only_removed_lines_contributes_none() {
    let diff = concat!(
        "diff --git a/src/overlap.py b/src/overlap.py\n",
        "--- a/src/overlap.py\n",
        "+++ b/src/overlap.py\n",
        "@@ -4,2 +3,0 @@\n",
        "-    if a is None:\n",
        "-        return False\n",
    );

    assert_eq!(
        lines_of(diff, "src/overlap.py"),
        None,
        "a file that gained no line is not a file with candidates"
    );
}

#[test]
fn every_hunk_of_a_file_contributes_its_own_lines() {
    let diff = concat!(
        "diff --git a/src/overlap.py b/src/overlap.py\n",
        "--- a/src/overlap.py\n",
        "+++ b/src/overlap.py\n",
        "@@ -2 +2 @@\n",
        "-    return a < b\n",
        "+    return a <= b\n",
        "@@ -9,0 +10,2 @@\n",
        "+def touches(a, b):\n",
        "+    return a == b\n",
    );

    assert_eq!(lines_of(diff, "src/overlap.py"), Some(vec![2, 10, 11]));
}

/// A file that only moved has no hunk in the diff at all. Nothing about it changed,
/// so there is nothing in it to mutate — and a reader who found it selected would be
/// looking at a mutation of code this pull request did not write.
#[test]
fn a_file_that_was_only_renamed_contributes_nothing() {
    let diff = concat!(
        "diff --git a/src/overlap.py b/src/ranges.py\n",
        "similarity index 100%\n",
        "rename from src/overlap.py\n",
        "rename to src/ranges.py\n",
    );

    assert!(changed(diff).files.is_empty());
}

#[test]
fn a_file_that_was_deleted_contributes_nothing() {
    let diff = concat!(
        "diff --git a/src/overlap.py b/src/overlap.py\n",
        "deleted file mode 100644\n",
        "--- a/src/overlap.py\n",
        "+++ /dev/null\n",
        "@@ -1,3 +0,0 @@\n",
        "-def overlaps(a, b):\n",
    );

    assert!(changed(diff).files.is_empty());
}

/// Every spelling a project uses for a test file, and the count that says how many
/// were left out. The count is the point: a pull request that changed only its tests
/// selects nothing, and a reader has to be able to tell that from a run that found
/// nothing to look at.
#[test]
fn the_test_files_a_change_touched_are_left_out_and_counted() {
    let excluded = [
        "tests/test_overlap.py",
        "src/tests/helpers.py",
        "test/support.py",
        "src/test_overlap.py",
        "src/overlap_test.py",
        "src/overlap_tests.py",
        "src/conftest.py",
    ];
    let mut diff = String::new();
    for file in excluded {
        let _ = write!(
            diff,
            "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -1 +1 @@\n+x = 1\n"
        );
    }
    diff.push_str(
        "diff --git a/src/overlap.py b/src/overlap.py\n--- a/src/overlap.py\n+++ b/src/overlap.py\n@@ -1 +1 @@\n+x = 1\n",
    );

    let found = changed(&diff);

    assert_eq!(
        found
            .files
            .iter()
            .map(|file| file.file.as_str())
            .collect::<Vec<&str>>(),
        vec!["src/overlap.py"]
    );
    assert_eq!(found.tests_excluded, excluded.len());
}

/// A file no language pack of this project handles is not a file it could mutate, and
/// leaving it out is not the same event as leaving a test file out: one is a change
/// somebody may want mutated later, the other is a change that is deliberately never
/// a target.
#[test]
fn a_file_this_project_cannot_mutate_is_left_out_without_being_counted_as_a_test() {
    let diff = concat!(
        "diff --git a/README.md b/README.md\n",
        "--- a/README.md\n",
        "+++ b/README.md\n",
        "@@ -1 +1 @@\n",
        "+# a heading\n",
        "diff --git a/src/overlap.pyi b/src/overlap.pyi\n",
        "--- a/src/overlap.pyi\n",
        "+++ b/src/overlap.pyi\n",
        "@@ -1 +1 @@\n",
        "+def overlaps(a: int, b: int) -> bool: ...\n",
    );

    let found = changed(diff);

    assert!(found.files.is_empty());
    assert_eq!(found.tests_excluded, 0);
}

/// The files come back in a settled order whatever order the diff put them in, so
/// that what a limit cuts is decided by the change and not by git.
#[test]
fn the_files_come_back_in_the_order_their_paths_sort_in() {
    let mut diff = String::new();
    for file in ["src/z.py", "src/a.py", "src/m.py"] {
        let _ = write!(
            diff,
            "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -1 +1 @@\n+x = 1\n"
        );
    }

    let found = changed(&diff);

    assert_eq!(
        found
            .files
            .iter()
            .map(|file| file.file.as_str())
            .collect::<Vec<&str>>(),
        vec!["src/a.py", "src/m.py", "src/z.py"]
    );
}

/// A path git had to quote is still that path. Reading the quotes as part of the name
/// would look for a file nothing has.
#[test]
fn a_path_the_diff_quoted_is_read_without_its_quotes() {
    let diff = concat!(
        "diff --git \"a/src/od\\303\\251.py\" \"b/src/od\\303\\251.py\"\n",
        "--- \"a/src/od\\303\\251.py\"\n",
        "+++ \"b/src/od\\303\\251.py\"\n",
        "@@ -1 +1 @@\n",
        "+x = 1\n",
    );

    let found = changed(diff);

    assert_eq!(
        found
            .files
            .iter()
            .map(|file| file.file.as_str())
            .collect::<Vec<&str>>(),
        vec!["src/odé.py"],
        "the quotes and the escapes are the diff's, not the path's"
    );
}

/// A line of a removed hunk can be spelled exactly like a hunk header, and a parser
/// that read every line looking for one would take that line for a header.
#[test]
fn a_removed_line_that_looks_like_a_hunk_header_is_not_one() {
    let diff = concat!(
        "diff --git a/src/overlap.py b/src/overlap.py\n",
        "--- a/src/overlap.py\n",
        "+++ b/src/overlap.py\n",
        "@@ -2 +2 @@\n",
        "-@@ -900,4 +900,4 @@\n",
        "+@@ -901,4 +901,4 @@\n",
    );

    assert_eq!(lines_of(diff, "src/overlap.py"), Some(vec![2]));
}
