//! Finding the tests of a function by the convention every Python project shares.
//!
//! Showing a model the tests of the function it is asked about was measured to improve
//! what it proposes, and a selection that chose the function for itself has nobody to
//! name those tests. So it looks — and looks in exactly the places the convention puts
//! them, at exact paths, one at a time, taking the first that is really a file.
//!
//! What it deliberately does not do is search. A project-wide hunt for a file whose name
//! resembles the target's would find another package's tests of another module of the
//! same name, and the consequence of a wrong answer here is not a wrong report: **the
//! file that is found is read and sent to a model provider.** So the answer is either a
//! path the convention names or nothing at all, and nothing at all is recorded as such.

use std::path::Path;

/// The directory a project's tests of one package live in.
const TESTS: &str = "tests";

/// The extension of the files this looks for.
const PYTHON: &str = ".py";

/// The test file the convention names for `file`, when the project really has one.
///
/// At most one, and empty when the convention names nothing that exists. `file` is
/// POSIX-style and relative to `project`, and so is the answer.
#[must_use]
pub fn inferred_tests(project: &Path, file: &str) -> Vec<String> {
    let Some(stem) = file.strip_suffix(PYTHON) else {
        return Vec::new();
    };
    let mut components: Vec<&str> = stem.split('/').collect();
    let Some(module) = components.pop() else {
        return Vec::new();
    };
    let named = format!("test_{module}{PYTHON}");
    for candidate in where_it_would_be(&components, &named) {
        if project.join(&candidate).is_file() {
            return vec![candidate];
        }
    }
    Vec::new()
}

/// Every path the convention would put the tests of a module at, in the order they are
/// worth trying: the file's own package first, then beside the file, then the tests
/// directory of each package the file sits inside, out to the project root.
///
/// Out to the root because that is where the convention puts them in the layout most
/// projects use — a module under `src/` and its tests under `tests/` — and each step is
/// one exact path rather than a search of anything.
fn where_it_would_be(package: &[&str], named: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut consider = |path: String| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };
    consider(under(package, &[TESTS, named]));
    consider(under(package, &[named]));
    let mut enclosing = package;
    loop {
        consider(under(enclosing, &[TESTS, named]));
        match enclosing.split_last() {
            Some((_, outer)) => enclosing = outer,
            None => break,
        }
    }
    candidates
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
