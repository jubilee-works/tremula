//! The two questions a selection asks git, and nothing else.
//!
//! Both are read-only and neither touches the working tree. The comparison runs from
//! the merge base rather than from the named reference, because a branch whose base has
//! moved on since it was taken would otherwise have the base's own later commits
//! reported as its change — and a pull request is answerable for what it wrote, not for
//! what happened on main while it was open.

use std::{
    path::Path,
    process::{Command, Output},
};

/// How much of git's own complaint is quoted back.
const QUOTED: usize = 400;

/// Why git could not answer.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// git could not be run at all.
    #[error(
        "cannot run git in `{}`: {reason}; a selection compares this project against a revision, so git has to be available and the project has to be a repository",
        project.display()
    )]
    Unusable {
        /// The project it was asked about.
        project: std::path::PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// There is no common ancestor of the named reference and the working revision.
    #[error(
        "git cannot find where this revision and `{reference}` parted: {reason}; check that the reference exists here — a shallow clone often has too little history for one, and `fetch --deepen` or `fetch --unshallow` is what gives it enough"
    )]
    NoMergeBase {
        /// The reference the caller named.
        reference: String,
        /// What git said.
        reason: String,
    },
    /// The comparison itself failed.
    #[error("git cannot compare `{from}` with this revision: {reason}")]
    NoDiff {
        /// The commit the comparison was to run from.
        from: String,
        /// What git said.
        reason: String,
    },
}

/// The commit this revision and `reference` parted at.
///
/// # Errors
///
/// Returns [`GitError`] when git cannot be run, or when there is no common ancestor to
/// find — which is what a reference that does not exist here looks like, and what a
/// clone too shallow to hold one looks like too.
pub fn merge_base(project: &Path, reference: &str) -> Result<String, GitError> {
    let answered = ask(project, &["merge-base", reference, "HEAD"])?;
    if !answered.status.success() {
        return Err(GitError::NoMergeBase {
            reference: reference.to_owned(),
            reason: complaint(&answered),
        });
    }
    Ok(String::from_utf8_lossy(&answered.stdout).trim().to_owned())
}

/// What this revision changed since `from`, with no context lines around it.
///
/// No context, because every line of context is a line this change did not write, and a
/// reader of the output cannot tell one from the other. Renames are found rather than
/// left to be reported as a whole file deleted and a whole file added: a file that only
/// moved wrote none of its own lines.
///
/// # Errors
///
/// Returns [`GitError`] when git cannot be run or cannot make the comparison.
pub fn since(project: &Path, from: &str) -> Result<String, GitError> {
    let answered = ask(
        project,
        &[
            "diff",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            from,
            "HEAD",
        ],
    )?;
    if !answered.status.success() {
        return Err(GitError::NoDiff {
            from: from.to_owned(),
            reason: complaint(&answered),
        });
    }
    Ok(String::from_utf8_lossy(&answered.stdout).into_owned())
}

/// Ask git something about the project.
fn ask(project: &Path, arguments: &[&str]) -> Result<Output, GitError> {
    Command::new("git")
        .arg("-C")
        .arg(project)
        .args(arguments)
        .output()
        .map_err(|err| GitError::Unusable {
            project: project.to_path_buf(),
            reason: err.to_string(),
        })
}

/// What git said about it, shortened.
fn complaint(answered: &Output) -> String {
    let said = String::from_utf8_lossy(&answered.stderr);
    let said = said.trim();
    if said.is_empty() {
        return "it said nothing".to_owned();
    }
    match said.char_indices().nth(QUOTED) {
        Some((cut, _)) => format!("{}…", &said[..cut]),
        None => said.to_owned(),
    }
}
