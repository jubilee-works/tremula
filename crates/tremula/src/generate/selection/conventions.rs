//! Finding the tests of a function by the convention every Python project shares.
//!
//! Showing a model the tests of the function it is asked about was measured to improve
//! what it proposes, and a selection that chose the function for itself has nobody to
//! name those tests. So it looks — at one path, and only one.
//!
//! # The one path, and why the boundary is where it is
//!
//! The convention is `tests/test_<stem>.py`, and the question is *whose* `tests`. Two
//! answers were measured and both are wrong. Looking only in the target file's own
//! directory finds nothing at all in the layout most projects have, where a module sits
//! under `src/` and the tests sit beside the packaging; walking up to the project root
//! finds the *next package over*'s tests of a module of the same name — `pkgb/util.py`
//! answered with `tests/test_util.py`, which is `pkga`'s.
//!
//! So the boundary is the package: the nearest directory above the file, inside the
//! project, that declares one — a `pyproject.toml`, a `setup.py`, or a `setup.cfg`. Its
//! `tests/test_<stem>.py` is looked at, and nothing else is. A package that has no such
//! file answers nothing rather than borrowing its neighbour's, and a project that declares
//! no package anywhere is one package whose root is the project root.
//!
//! What this deliberately does not do is search. The consequence of a wrong answer here is
//! not a wrong report: **the file that is found is read and sent to a model provider.** So
//! the answer is either the path the convention names or nothing at all, and nothing at all
//! is recorded as such.

use std::path::Path;

/// The directory a package's tests live in.
const TESTS: &str = "tests";

/// The extension of the files this looks for.
const PYTHON: &str = ".py";

/// The files that say a directory is the root of a package.
const DECLARES_A_PACKAGE: [&str; 3] = ["pyproject.toml", "setup.py", "setup.cfg"];

/// The test file the convention names for `file`, when the project really has one.
///
/// At most one, and empty when the convention names nothing that exists. `file` is
/// POSIX-style and relative to `project`, and so is the answer.
#[must_use]
pub fn inferred_tests(project: &Path, file: &str) -> Vec<String> {
    let Some(stem) = file.strip_suffix(PYTHON) else {
        return Vec::new();
    };
    let mut directories: Vec<&str> = stem.split('/').collect();
    let Some(module) = directories.pop() else {
        return Vec::new();
    };
    let named = format!("test_{module}{PYTHON}");
    let candidate = under(package_root(project, &directories), &[TESTS, &named]);
    if project.join(&candidate).is_file() {
        return vec![candidate];
    }
    Vec::new()
}

/// The package the target file belongs to, as the directories leading to it.
///
/// The nearest one that declares a package, looked for from the file's own directory
/// outwards. The project root is the last candidate and the answer when nothing declares
/// anything: a repository of scripts is one package, and stopping short of its root would
/// make this find nothing in any project at all.
fn package_root<'a>(project: &Path, directories: &'a [&'a str]) -> &'a [&'a str] {
    let mut at = directories;
    loop {
        if DECLARES_A_PACKAGE
            .iter()
            .any(|declares| project.join(under(at, &[declares])).is_file())
        {
            return at;
        }
        match at.split_last() {
            Some((_, outer)) => at = outer,
            None => return at,
        }
    }
}

/// One path, built from a directory and what is under it.
fn under(directory: &[&str], rest: &[&str]) -> String {
    directory
        .iter()
        .chain(rest.iter())
        .copied()
        .collect::<Vec<&str>>()
        .join("/")
}
