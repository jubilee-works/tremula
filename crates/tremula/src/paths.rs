//! The two absolute spellings a platform can have for one directory.
//!
//! On this platform a handful of top-level directories are links into `/private`, so
//! one directory has two absolute names and a process prints whichever of them it was
//! handed. Anything that compares a path it was given against a path something else
//! printed has to know both, and the rule is the same wherever that happens: a bundle
//! taking a machine's directories out of a log, and a selection working out which of
//! the project's files a coverage document is talking about.

use std::path::Path;

/// Where the directories a platform reaches through a link of its own actually live.
const PRIVATE: &str = "/private";

/// The directories that are links into [`PRIVATE`] here. One directory, two absolute
/// spellings, and a process prints whichever of them it was handed.
const LINKED: [&str; 2] = ["/tmp", "/var"];

/// The other absolute spelling of one directory, when this platform has two.
///
/// Derived from whichever form is in hand, so it covers both directions: a path given
/// as `/var/folders/...` yields the `/private` one, and one given as
/// `/private/var/folders/...` yields the short one.
///
/// The prefix has to end where a component of the path ends. `/variant` begins with
/// `/var` and is not inside it, and answering with `/private/variant` would name a
/// directory that does not exist.
pub(crate) fn sibling_spelling(trimmed: &str) -> Option<String> {
    for linked in LINKED {
        if let Some(rest) = trimmed.strip_prefix(linked)
            && a_component_ends_there(rest)
        {
            return Some(format!("{PRIVATE}{trimmed}"));
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{PRIVATE}{linked}"))
            && a_component_ends_there(rest)
        {
            return Some(format!("{linked}{rest}"));
        }
    }
    None
}

/// Whether a prefix ended where a component of the path ends, rather than inside a name.
fn a_component_ends_there(rest: &str) -> bool {
    rest.is_empty() || rest.starts_with('/')
}

/// Every absolute spelling of `directory` this platform has, in the form a comparison
/// against another absolute path can use: no trailing separator, and nothing relative.
///
/// A relative path contributes nothing, and that is not a shortcoming: the thing being
/// compared against is absolute, so a root that is not absolute cannot be a prefix of
/// it. Resolving the directory is what makes the default `--project .` usable, since
/// the only absolute form of it there is to learn from is the resolved one.
pub(crate) fn absolute_spellings(directory: &Path) -> Vec<String> {
    let mut spellings: Vec<String> = Vec::new();
    remember(&mut spellings, directory);
    if let Ok(resolved) = std::fs::canonicalize(directory) {
        remember(&mut spellings, &resolved);
    }
    spellings
}

/// Keep one directory's spelling, and its sibling, if it is one a comparison can use.
fn remember(spellings: &mut Vec<String>, directory: &Path) {
    let spelled = directory.to_string_lossy();
    let trimmed = spelled.trim_end_matches('/');
    if trimmed.len() < 2 || !trimmed.starts_with('/') {
        return;
    }
    for spelling in [Some(trimmed.to_owned()), sibling_spelling(trimmed)]
        .into_iter()
        .flatten()
    {
        if !spellings.contains(&spelling) {
            spellings.push(spelling);
        }
    }
}
