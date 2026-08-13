//! Why a triage command could not complete.
//!
//! Every one of these is an operational failure that prevents the command from
//! completing. It may occur before or during judge attempts, or while publishing a
//! requested derivative manifest after a completed triage. None is a classification:
//! a completed classification, including one that established nothing, is never itself
//! a failure. When derivative publication fails, the command fails but the
//! already-written `triage.json` remains as evidence.

use std::path::PathBuf;

use crate::{pack::PackError, python_env::EnvError, suppressions::SuppressionError};

/// Anything that prevents a triage command from completing.
///
/// This includes failures before or during judge attempts and failures publishing an
/// optional derivative after classification and `triage.json` creation.
#[derive(Debug, thiserror::Error)]
pub enum TriageFailure {
    /// The optional filtering controls must be supplied together.
    #[error("{option} requires {companion}; pass both options or neither")]
    PairedOptions {
        /// The option whose presence was detected.
        option: &'static str,
        /// The required companion option.
        companion: &'static str,
    },
    /// There is already a filesystem object where the derivative would go.
    #[error(
        "`{}` already exists; a filtered manifest is never written over an existing path — move it aside, or choose another --out-manifest",
        path.display()
    )]
    OutputExists {
        /// The occupied destination.
        path: PathBuf,
    },
    /// The derivative destination or its parent cannot be used.
    #[error(
        "cannot write filtered manifest `{}`: {reason}; choose a new path in an existing writable directory",
        path.display()
    )]
    OutputUnusable {
        /// The requested destination.
        path: PathBuf,
        /// What prevented preflight, staging, or commit.
        reason: String,
    },
    /// There is no such run.
    #[error(
        "no run directory at `{}`; pass --run with the run to triage, or check `.tremula/runs`",
        path.display()
    )]
    NoRunDirectory {
        /// The path that was looked for.
        path: PathBuf,
    },
    /// A path could not be resolved.
    #[error("cannot resolve `{}`: {reason}; check the path and try again", path.display())]
    PathUnresolvable {
        /// The path that was given.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// A document the run should have left is not there or cannot be read.
    #[error(
        "cannot read `{}`: {reason}; a triage reads the run's own documents and nothing else, so a run missing one cannot be triaged",
        path.display()
    )]
    Unreadable {
        /// The document that could not be read.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// A document the run left is not the document it should be.
    #[error("`{}` is not a valid {what}: {reason}", path.display())]
    Invalid {
        /// The document that could not be read as itself.
        path: PathBuf,
        /// What it was expected to be.
        what: &'static str,
        /// Why it could not be read as that.
        reason: String,
    },
    /// The report in the directory is about another run.
    #[error(
        "the report in `{directory}` is about run {found} and the directory is named {expected}; triage reads a run's own documents by name, so a directory holding another run's report is one nothing can be joined in"
    )]
    AnotherRun {
        /// The directory that was triaged.
        directory: String,
        /// The run the report names.
        found: String,
        /// The run the directory is named after.
        expected: String,
    },
    /// The run kept no snapshot, so there are no original bytes to run against.
    #[error(
        "the run in `{}` kept no snapshot of the project's files, so there is nothing to run a witness against; a run that stopped before its sources were read keeps none",
        run_dir.display()
    )]
    NoSnapshot {
        /// The run with no snapshot.
        run_dir: PathBuf,
    },
    /// The report names a mutant the manifest beside it does not.
    #[error(
        "the report in `{}` gives a verdict on mutant {mutant_id}, which the manifest beside it does not carry; the two documents are of one run and cannot disagree about which mutants it had",
        run_dir.display()
    )]
    MutantNotInManifest {
        /// The run whose documents disagree.
        run_dir: PathBuf,
        /// The mutant the report names.
        mutant_id: String,
    },
    /// The snapshot does not hold the bytes the manifest was checked against.
    #[error(
        "the snapshot's `{file}` is not the file mutant {mutant_id} was generated against; the snapshot is what a witness runs on, so bytes that are not the run's own would be measuring another file"
    )]
    SnapshotIsAnotherFile {
        /// The file whose bytes do not match.
        file: String,
        /// A mutant that expected other bytes.
        mutant_id: String,
    },
    /// The project's Python could not be used.
    #[error(transparent)]
    Environment(#[from] EnvError),
    /// The pack could not do its work.
    #[error(transparent)]
    Pack(#[from] PackError),
    /// The record of what a person has dismissed cannot be used.
    #[error(transparent)]
    Suppressions(#[from] SuppressionError),
    /// There is no key to call a model with, or the provider would not take it.
    #[error("{reason}")]
    NoCredential {
        /// What the judge said about it, which already names the variable and what
        /// to do.
        reason: String,
    },
    /// Every survivor failed for a reason that was not about the survivor.
    #[error(
        "none of the {survivors} survivor(s) could be judged, and every failure was of the tools rather than of a mutation — the last was: {last}"
    )]
    NothingCouldBeJudged {
        /// How many survivors were tried.
        survivors: usize,
        /// The last failure, as it was reported.
        last: String,
    },
    /// The document could not be written.
    #[error("cannot write `{}`: {reason}; check that the run directory is writable", path.display())]
    Unwritable {
        /// The document that could not be written.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
}
