//! Turning a run's evidence into something a person reads on a pull request.
//!
//! # Where the evidence comes from
//!
//! The manifest in the working tree, and the run directory. Not a bundle: a run with nothing
//! to test never produces one, and that is precisely the run whose comment is most worth
//! having — a change every line of which is uncovered has no mutants and one real finding.
//! So the two documents this reads are the two that always exist.
//!
//! # The two modes, and why the first one is the product
//!
//! By default the body goes to standard output and nothing is called, reached, or
//! authenticated. That output is the whole of what this command is: a project on another
//! platform, or a person at a terminal, pipes it wherever it belongs. Posting it to GitHub is
//! an option on top, for the platform most of its users are on.
//!
//! Reporting is done here rather than left to whoever runs this, and that is a boundary drawn
//! deliberately: changing somebody's code is theirs to do, and saying what a test run found
//! is ours.
//!
//! # Exit code
//!
//! Zero when the body was produced, whatever it says — a run that mutated nothing is not a
//! failure of this command. Two when a document cannot be read, or when a comment was asked
//! for and could not be posted: a tool that swallowed a failed post would leave a reviewer
//! looking at last push's results, and forgiveness about that is the caller's to grant with
//! `continue-on-error` rather than ours to assume.

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use tremula_contracts::{manifest::Manifest, report::Report, triage::Triage};

use crate::{
    comment::{
        github::{GitHub, PostError, Poster, marker, repository, with_marker},
        render::{Evidence, render},
    },
    generate::command::DEFAULT_MANIFEST,
    orchestrate::EXIT_FAILURE,
    run_dir::TREMULA_DIR,
    triage::{TRIAGE_DOCUMENT, inputs::REPORT},
};

pub mod github;
pub mod render;

/// Where run directories live under a project.
const RUNS_DIR: &str = "runs";

/// The link that points at the newest run.
const LATEST: &str = "latest";

/// What `tremula comment` was asked to do.
#[derive(Debug, clap::Args)]
pub struct CommentArgs {
    /// Run to report on. Defaults to the project's most recent run.
    #[arg(long, value_name = "DIR")]
    pub run: Option<PathBuf>,
    /// The manifest the run was given, which is where the mutations and the record of how
    /// they were chosen are. Defaults to `tremula-manifest.json` in the project.
    #[arg(long, value_name = "PATH")]
    pub manifest: Option<PathBuf>,
    /// Project the run and the manifest belong to.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
    /// Where the evidence for this run was published, if it was published anywhere. The
    /// comment links to it when there is a link and says nothing about it when there is not,
    /// so an upload that failed its quota costs a line rather than the comment.
    #[arg(long, value_name = "URL")]
    pub evidence_url: Option<String>,
    /// Post the comment on this pull request of GitHub, replacing this tool's own previous
    /// comment on it. Without this the body goes to standard output and nothing is called.
    #[arg(long, value_name = "N")]
    pub github_pr: Option<u64>,
    /// The repository the pull request is in, as `OWNER/NAME`. Taken from
    /// `GITHUB_REPOSITORY` when it is not given, which every workflow of the platform sets.
    #[arg(long, value_name = "OWNER/NAME", requires = "github_pr")]
    pub github_repo: Option<String>,
}

/// Why a comment could not be made.
#[derive(Debug, thiserror::Error)]
pub enum CommentFailure {
    /// A document could not be read.
    #[error(
        "cannot read `{}`: {reason}; a comment is made from the manifest a run was given and the run's own report, so both have to be there",
        path.display()
    )]
    Unreadable {
        /// The document that could not be read.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// A document is not the document it should be.
    #[error("`{}` is not a valid {what}: {reason}", path.display())]
    Invalid {
        /// The document that could not be understood.
        path: PathBuf,
        /// What it was meant to be.
        what: &'static str,
        /// Why it could not be read as that.
        reason: String,
    },
    /// The run directory is not one.
    #[error(
        "`{}` is not a run directory; pass `--run` with one, or run this where the project's own runs are",
        path.display()
    )]
    NoRunDirectory {
        /// The path that was given.
        path: PathBuf,
    },
    /// The comment could not be posted.
    #[error(transparent)]
    Post(#[from] PostError),
}

/// Write the comment, and post it when that was asked for.
#[must_use]
pub fn comment(args: &CommentArgs) -> ExitCode {
    comment_with(args, &GitHub::new())
}

/// The same, through a poster the caller has already built.
///
/// The seam a test answers through, and the seam a second platform would arrive at.
#[must_use]
pub fn comment_with(args: &CommentArgs, poster: &dyn Poster) -> ExitCode {
    match write(args, poster) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("error: {failure}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Make the body, and do whatever was asked with it.
///
/// # Errors
///
/// Returns [`CommentFailure`] when a document cannot be read or understood, or when a comment
/// was asked for and the platform would not take it.
pub fn write(args: &CommentArgs, poster: &dyn Poster) -> Result<(), CommentFailure> {
    let body = body(args)?;
    let Some(pull_request) = args.github_pr else {
        println!("{body}");
        return Ok(());
    };
    let repo = repository(args.github_repo.as_deref())?;
    let put = poster.upsert(
        &repo,
        pull_request,
        &with_marker(&marker(&repo, pull_request), &body),
    )?;
    println!(
        "{} {}",
        if put.replaced {
            "replaced the comment at"
        } else {
            "commented at"
        },
        put.url
    );
    Ok(())
}

/// The body alone, from the documents this run left behind.
///
/// # Errors
///
/// Returns [`CommentFailure`] when the manifest or the run's report cannot be read or
/// understood. A triage is different: a run that has not been triaged is an ordinary run, so
/// its absence costs one column of one table rather than the comment.
pub fn body(args: &CommentArgs) -> Result<String, CommentFailure> {
    let run = where_to_look(args);
    if !run.is_dir() {
        return Err(CommentFailure::NoRunDirectory { path: run });
    }
    // Resolved, because the run this is pointed at is routinely the `latest` link, and every
    // document read below is read out of the directory that link leads to.
    let resolved = fs::canonicalize(&run).unwrap_or(run);
    let manifest: Manifest = document(&which_manifest(args), "manifest")?;
    let report: Report = document(&resolved.join(REPORT), "report")?;
    let triage: Option<Triage> = optional(&resolved.join(TRIAGE_DOCUMENT), "triage")?;
    Ok(render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: triage.as_ref(),
        evidence_url: args.evidence_url.as_deref(),
    }))
}

/// Which run to read: the one named, or the newest.
fn where_to_look(args: &CommentArgs) -> PathBuf {
    args.run
        .clone()
        .unwrap_or_else(|| args.project.join(TREMULA_DIR).join(RUNS_DIR).join(LATEST))
}

/// Which manifest to read: the one named, or the project's own.
fn which_manifest(args: &CommentArgs) -> PathBuf {
    args.manifest
        .clone()
        .unwrap_or_else(|| args.project.join(DEFAULT_MANIFEST))
}

/// One document, as itself.
fn document<T: serde::de::DeserializeOwned>(
    path: &Path,
    what: &'static str,
) -> Result<T, CommentFailure> {
    let text = fs::read_to_string(path).map_err(|err| CommentFailure::Unreadable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;
    serde_json::from_str(&text).map_err(|err| CommentFailure::Invalid {
        path: path.to_path_buf(),
        what,
        reason: err.to_string(),
    })
}

/// One document that a run may not have. Absent is an answer; unreadable is not.
fn optional<T: serde::de::DeserializeOwned>(
    path: &Path,
    what: &'static str,
) -> Result<Option<T>, CommentFailure> {
    if !path.exists() {
        return Ok(None);
    }
    document(path, what).map(Some)
}
