//! Why a run's evidence could not be packaged.
//!
//! Most of these are one run's documents disagreeing with each other. That is
//! worth its own vocabulary rather than one "invalid run": a bundle is read by
//! somebody who was not there, and evidence from two runs packaged as one would be
//! read as a single measurement that never happened.

use std::path::PathBuf;

use tremula_contracts::SCHEMA_VERSION;

use crate::{
    bundle::{LATEST, collect::REPORT},
    run_dir::TREMULA_DIR,
};

/// Anything that stops a bundle from being written.
#[derive(Debug, thiserror::Error)]
pub enum BundleFailure {
    /// There is no such run.
    #[error(
        "no run directory at `{}`; pass --run with the run to package, or check `{TREMULA_DIR}/runs`",
        path.display()
    )]
    NoRunDirectory {
        /// The path that was looked for.
        path: PathBuf,
    },
    /// A path on the command line could not be resolved.
    #[error("cannot resolve `{}`: {reason}; check the path and try again", path.display())]
    PathUnresolvable {
        /// The path that was given.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The directory is a run's and has no report in it yet.
    #[error(
        "the run in `{}` has written no `{REPORT}`, so it may still be in progress or may have failed; a run points `{TREMULA_DIR}/runs/{LATEST}` at itself before it writes its report — wait for it to finish, or pass --run with a run that has one",
        run_dir.display()
    )]
    RunUnfinished {
        /// The run with no report.
        run_dir: PathBuf,
    },
    /// One of the run's documents could not be read.
    #[error("cannot read `{}`: {reason}", path.display())]
    Unreadable {
        /// The document that could not be read.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// One of the run's documents is not what it claims to be.
    #[error(
        "`{}` is not a valid {what}: {reason}; compare it against the {what} schema",
        path.display()
    )]
    Invalid {
        /// The document that could not be parsed.
        path: PathBuf,
        /// Which document it was meant to be.
        what: &'static str,
        /// Why it could not be read as one.
        reason: String,
    },
    /// Two documents in one directory are about different runs.
    #[error(
        "the {document} in `{directory}` is about run {found}, and this directory is run {expected}; evidence from two runs must not travel in one bundle, because a reader has no way to tell them apart afterwards"
    )]
    AnotherRun {
        /// Which document it was.
        document: &'static str,
        /// The directory that was read.
        directory: String,
        /// The run the document names.
        found: String,
        /// The run the directory is.
        expected: String,
    },
    /// The report itself was written against another contract than this one reads.
    #[error(
        "the report in `{directory}` declares schema version {found} and this tremula reads {SCHEMA_VERSION}; every other document is checked against the report, so a report from another contract would make that agreement a statement about a version nothing here wrote — and the bundle would carry {SCHEMA_VERSION} over documents that are not it"
    )]
    ReportContract {
        /// The directory that was read.
        directory: String,
        /// The version the report declares.
        found: String,
    },
    /// The manifest beside the report is not the one the run ran.
    #[error(
        "the manifest in `{directory}` hashes to {found}… and run {run_id} was named after a manifest hashing to {expected}…; a run's name carries the first six hexadecimal digits of the manifest it ran, so this is another run's manifest — and the spans and replacements in it are not the ones the report's verdicts are about"
    )]
    ManifestOfAnotherRun {
        /// The directory that was read.
        directory: String,
        /// The run the directory is.
        run_id: String,
        /// What the manifest that is there hashes to.
        found: String,
        /// What the run's own name says its manifest hashed to.
        expected: String,
    },
    /// Two documents in one directory were written against different contracts.
    #[error(
        "the {document} in `{directory}` declares schema version {found} and the report declares {expected}; one run's documents are one contract's, and packaging both would publish a bundle that no single reader can be held to"
    )]
    ContractVersions {
        /// Which document it was.
        document: &'static str,
        /// The directory that was read.
        directory: String,
        /// The version that document declares.
        found: String,
        /// The version the report declares.
        expected: String,
    },
    /// The triage in the directory is a second opinion on some other report.
    #[error(
        "the triage in `{directory}` is about `{found}` rather than `{REPORT}`; a triage packaged beside a report it was not made from would read as a judgement of verdicts it never saw"
    )]
    TriageIsAboutAnotherReport {
        /// The directory that was read.
        directory: String,
        /// The document the triage names.
        found: String,
    },
    /// The report gives a verdict on a mutant the manifest does not hold.
    #[error(
        "mutant {mutant_id} has a verdict in the report and no entry in the manifest of run {run_id}; the manifest is where its span and its replacement are, so nothing in the bundle could say what that verdict was about"
    )]
    MutantNotInManifest {
        /// The run that was read.
        run_id: String,
        /// The mutant the report names.
        mutant_id: String,
    },
    /// The manifest holds a mutant the report says nothing about.
    #[error(
        "mutant {mutant_id} is in the manifest of run {run_id} and has no verdict in its report; a bundle whose manifest and report cover different mutants would leave a reader unable to account for one of them"
    )]
    MutantNotInReport {
        /// The run that was read.
        run_id: String,
        /// The mutant the manifest names.
        mutant_id: String,
    },
    /// A mutant identifier is not one, and a file would be named after it.
    #[error(
        "`{mutant_id}` cannot be a mutant identifier: an identifier is 64 lowercase hexadecimal digits, and a bundle will not make a file path out of anything else"
    )]
    MutantIdNotCanonical {
        /// The identifier as the document spells it.
        mutant_id: String,
    },
    /// The run kept no snapshot, so no patch can be checked against anything.
    #[error(
        "the run in `{}` kept no snapshot of the project's files; a patch is only worth packaging if it was checked against the bytes the run measured, and the snapshot is those bytes",
        run_dir.display()
    )]
    NoSnapshot {
        /// The run with no snapshot.
        run_dir: PathBuf,
    },
    /// Something is already at the output path.
    #[error(
        "`{}` already exists; a bundle is never written over one, because the one already there may be the copy somebody was sent — pass --out with a path of its own",
        path.display()
    )]
    BundleExists {
        /// The path that is taken.
        path: PathBuf,
    },
    /// The bundle could not be written.
    #[error("cannot write `{}`: {reason}; check that the directory is writable", path.display())]
    Unwritable {
        /// The path that could not be written.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// A file in the finished bundle does not hash to what the index says.
    #[error(
        "`{path}` does not hash to what the bundle's own index says it does; nothing was published, because the whole use of the hashes is that a reader can trust them"
    )]
    IndexDisagrees {
        /// The path inside the bundle.
        path: String,
    },
}
