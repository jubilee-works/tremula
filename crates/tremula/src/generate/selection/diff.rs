//! Which lines a change added, read out of `git diff --unified=0`.
//!
//! Only the added side is read. A line a change removed is not somewhere a mutation
//! could land, and a line neither added nor removed is not this change's.
//!
//! # The hunk header forms that decide everything
//!
//! `@@ -a,b +c,d @@` is the general form, and every part of it that may be left out or
//! be zero is a case worth naming:
//!
//! * a count left out is one line — `@@ -2 +2 @@` is the header of every single-line
//!   change there is, and reading the absent count as zero would lose all of them;
//! * a count of zero on the added side is a removal, and the number beside it is where
//!   the removal happened rather than a line that now exists;
//! * a file that only moved has no hunk at all, so it contributes nothing — there is
//!   nothing in it this change wrote.
//!
//! # Which files are left out, and which of those are counted
//!
//! Only files this project's language pack could mutate, which is `.py`. A test file is
//! never a target, and is counted: a change that touched only its own tests selects
//! nothing, and a reader has to be able to tell that from a change nothing was found
//! in. A file of some other language is left out without being counted, because it is
//! not a decision about testing — it is a file no pack here handles.

use std::collections::{BTreeMap, BTreeSet};

/// The header that opens what one file's part of a diff says.
const FILE: &str = "diff --git ";

/// The header naming the file as the change leaves it.
const AFTER: &str = "+++ ";

/// The header that opens one hunk.
const HUNK: &str = "@@ ";

/// What a file is called when the change deleted it.
const NOWHERE: &str = "/dev/null";

/// The prefix git puts on the path of a file as the change leaves it.
const AFTER_PREFIX: &str = "b/";

/// The extension of the files this project's language pack can mutate.
const MUTABLE: &str = ".py";

/// What a diff added, file by file.
#[derive(Debug, Default)]
pub struct Changed {
    /// The files that gained lines, in the order their paths sort in.
    pub files: Vec<ChangedFile>,
    /// How many test files the change touched and this left out.
    pub tests_excluded: usize,
}

/// One file the change added lines to.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    /// The file, POSIX-style and relative to the project root.
    pub file: String,
    /// The lines it gained, 1-indexed, ascending, without repeats.
    pub lines: Vec<u32>,
}

/// Read what one diff added.
///
/// The files come back sorted by path, so that what a limit later cuts is decided by
/// the change rather than by the order git happened to print it in.
#[must_use]
pub fn changed(diff: &str) -> Changed {
    let mut added: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut tests: BTreeSet<String> = BTreeSet::new();
    let mut file: Option<String> = None;
    let mut in_hunks = false;
    for raw in diff.lines() {
        let record = raw.trim_end_matches('\r');
        if record.starts_with(FILE) {
            file = None;
            in_hunks = false;
            continue;
        }
        // Only before the first hunk of this file's part: past that, every line
        // beginning with a `+` is a line of somebody's source, and one of them can be
        // spelled exactly like a header.
        if !in_hunks && let Some(named) = record.strip_prefix(AFTER) {
            file = target(&unquoted(named.trim()), &mut tests);
            continue;
        }
        let Some(header) = record.strip_prefix(HUNK) else {
            continue;
        };
        in_hunks = true;
        let Some(name) = file.clone() else {
            continue;
        };
        let Some((first, count)) = added_by(header) else {
            continue;
        };
        let lines = added.entry(name).or_default();
        for line in first..first.saturating_add(count) {
            lines.insert(line);
        }
    }
    Changed {
        files: added
            .into_iter()
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(file, lines)| ChangedFile {
                file,
                lines: lines.into_iter().collect(),
            })
            .collect(),
        tests_excluded: tests.len(),
    }
}

/// The file this part of the diff is about, when it is one a mutation could land in.
///
/// A test file is remembered on its way out, by name, so that a file with two hunks is
/// counted once.
fn target(path: &str, tests: &mut BTreeSet<String>) -> Option<String> {
    if path == NOWHERE {
        return None;
    }
    let path = path.strip_prefix(AFTER_PREFIX).unwrap_or(path);
    if !path.ends_with(MUTABLE) {
        return None;
    }
    if a_test(path) {
        tests.insert(path.to_owned());
        return None;
    }
    Some(path.to_owned())
}

/// Whether a path is one of the project's tests rather than one of its sources.
///
/// By convention, because there is nothing else to go on: a directory called `test` or
/// `tests`, or a file named the way every Python test runner expects to find one.
fn a_test(path: &str) -> bool {
    let mut components: Vec<&str> = path.split('/').collect();
    let name = components.pop().unwrap_or_default();
    if components
        .iter()
        .any(|component| *component == "test" || *component == "tests")
    {
        return true;
    }
    name == "conftest.py"
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_tests.py")
}

/// The first line a hunk added and how many, or nothing when it added none.
fn added_by(header: &str) -> Option<(u32, u32)> {
    let side = header
        .split_whitespace()
        .find(|token| token.starts_with('+'))?;
    let extent = side.trim_start_matches('+');
    match extent.split_once(',') {
        None => Some((extent.parse().ok()?, 1)),
        Some((first, count)) => Some((first.parse().ok()?, count.parse().ok()?)),
    }
}

/// The path a header names, with the quoting git applies to it undone.
///
/// git quotes a path that is not plain ASCII and escapes the bytes of it, so the quoted
/// form is not the name of any file: taken literally it would look for a file spelled
/// with backslashes and digits in it.
fn unquoted(spelled: &str) -> String {
    let Some(quoted) = spelled
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return spelled.to_owned();
    };
    let escaped = quoted.as_bytes();
    let mut bytes: Vec<u8> = Vec::with_capacity(escaped.len());
    let mut at = 0usize;
    while let Some(byte) = escaped.get(at).copied() {
        at += 1;
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        let Some(escape) = escaped.get(at).copied() else {
            bytes.push(byte);
            break;
        };
        at += 1;
        match escape {
            b'n' => bytes.push(b'\n'),
            b't' => bytes.push(b'\t'),
            b'r' => bytes.push(b'\r'),
            b'0'..=b'7' => {
                let mut value = u32::from(escape - b'0');
                for _ in 0..2 {
                    let Some(digit @ b'0'..=b'7') = escaped.get(at).copied() else {
                        break;
                    };
                    value = value * 8 + u32::from(digit - b'0');
                    at += 1;
                }
                bytes.push(u8::try_from(value).unwrap_or(b'?'));
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
