//! Packaging one run's evidence for somebody who was not there.
//!
//! A run directory is a working space. It holds the execution backend's own
//! database, a generated configuration, a cache of bytecode, and the project's
//! sources as they were before the run — none of which is anybody's business but
//! this tool's, and handing the whole directory over would make every one of them a
//! published contract. A bundle is the other thing: an immutable set of documents,
//! each named and hashed, with the patches beside them that reproduce what the
//! documents describe.
//!
//! # Why this is its own command
//!
//! Two reasons, and the second is the decisive one. A run mostly ends at the
//! console, and forcing a decision about what to publish on every one of them would
//! be a cost paid by the many for the few. And `triage` happens *after* a run: a
//! bundle written by the run itself would be missing `triage.json` every time, so it
//! would have to be built again afterwards, and then either the no-replace rule
//! refuses the second one or a project ends up with two bundles of one run.
//!
//! # Why it is assembled somewhere else first
//!
//! Everything is written into `<out>.part` and renamed into place once every hash
//! has been checked against the bytes on disk. A bundle is a thing that gets copied
//! and sent, and a half-written one looks exactly like a whole one; the rename is
//! what makes the published path appear complete or not at all. A failure anywhere
//! leaves no final path and no staging directory either.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use sha2::{Digest, Sha256};
use tremula_contracts::{
    SCHEMA_VERSION,
    bundle::{Attached, Base, BundleIndex, Documents, Exposure, Suite},
};

use crate::{console, run_dir::TREMULA_DIR};

pub mod collect;
pub mod failures;
pub mod logs;
pub mod start_here;

use collect::{BASELINE, Evidence, MANIFEST, REPORT, RESULTS, Read, TRIAGE};
use failures::BundleFailure;
use logs::Machine;

/// What a bundle exits with when it could not be written. The same code every
/// other operational failure in this tool reports.
pub const EXIT_FAILURE: u8 = 2;

/// Where run directories live under a project.
const RUNS_DIR: &str = "runs";

/// The link that points at the newest run.
const LATEST: &str = "latest";

/// The bundle's own index.
pub const INDEX: &str = "bundle.json";

/// The document a reader of a bundle is meant to open first.
pub const START_HERE: &str = "START_HERE.md";

/// What a bundle's directory is called when the caller does not say.
const OUT_PREFIX: &str = "tremula-bundle-";

/// The name everything is assembled under before it is published.
const STAGING_SUFFIX: &str = ".part";

/// What `tremula bundle` was asked to do.
#[derive(Debug, clap::Args)]
pub struct BundleArgs {
    /// Run to package. Defaults to the project's most recent run.
    #[arg(long, value_name = "DIR")]
    pub run: Option<PathBuf>,
    /// Project the run belongs to. Its name goes into the bundle, and the run and
    /// the bundle are looked for under it.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
    /// Where to write the bundle. Defaults to `tremula-bundle-<run-id>` in the
    /// project, and an existing path is never written over.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,
    /// Leave the suite's own output out of the bundle. `results.json` still holds
    /// the execution backend's raw output either way.
    #[arg(long)]
    pub no_logs: bool,
}

/// What packaging a run came to.
#[derive(Debug)]
pub enum Packaged {
    /// The run tested nothing, so there was nothing to package. A successful run
    /// with an empty manifest is this, and it is not a failure.
    Nothing {
        /// The run that had nothing in it.
        run_id: String,
    },
    /// A bundle was written.
    Written(Box<Written>),
}

/// A bundle, and what a reader of the console needs to know about it.
#[derive(Debug)]
pub struct Written {
    /// Where it was published.
    pub path: PathBuf,
    /// Its own index, which is also what was written into it.
    pub index: BundleIndex,
    /// Survivors whose patch git would not accept. The bundle is written anyway —
    /// the rest of the evidence is real — and the exit code says it is not the
    /// bundle that was asked for.
    pub unusable: Vec<String>,
}

/// Package a run's evidence into a directory of its own.
#[must_use]
pub fn bundle(args: &BundleArgs) -> ExitCode {
    ExitCode::from(exit_status(args))
}

/// The whole of the command, and which of the three things happened.
///
/// The number rather than an [`ExitCode`], because this is the decision and an
/// `ExitCode` is a value nothing can read back. Zero when a bundle was written and
/// every survivor's patch was accepted, or when there was nothing to package;
/// [`EXIT_FAILURE`] when nothing could be written, and also when a survivor's patch
/// was refused — a bundle exists to let somebody reproduce a survivor, and one that
/// cannot is not a success however complete the rest of it is.
#[must_use]
pub fn exit_status(args: &BundleArgs) -> u8 {
    match package(args) {
        Ok(Packaged::Nothing { run_id }) => {
            println!(
                "nothing to package: run {run_id} tested no mutants, so it left no evidence a bundle could carry"
            );
            0
        }
        Ok(Packaged::Written(written)) => {
            println!("{}", console::render_bundle(&written));
            if written.unusable.is_empty() {
                0
            } else {
                EXIT_FAILURE
            }
        }
        Err(failure) => {
            eprintln!("error: {failure}");
            EXIT_FAILURE
        }
    }
}

/// Read the run, assemble the bundle beside where it goes, and publish it.
///
/// # Errors
///
/// Returns [`BundleFailure`] when the run cannot be read, when its documents
/// disagree with each other, when something is already at the output path, or when
/// anything could not be written. In every one of those the output path does not
/// exist afterwards and no staging directory is left behind.
pub fn package(args: &BundleArgs) -> Result<Packaged, BundleFailure> {
    let evidence = match collect::read(&where_to_look(args))? {
        Read::Nothing { run_id } => return Ok(Packaged::Nothing { run_id }),
        Read::Evidence(evidence) => evidence,
    };
    let out = args.out.clone().unwrap_or_else(|| {
        args.project
            .join(format!("{OUT_PREFIX}{}", evidence.run_id))
    });
    if out.exists() {
        return Err(BundleFailure::BundleExists { path: out });
    }
    // Absolute, because `git apply --check` is run with the snapshot as its working
    // directory and is handed the path of the patch to read.
    let staging = absolute(&staging_beside(&out))?;
    clear(&staging)?;
    let assembled = assemble(args, &evidence, &staging)
        .and_then(|written| publish(&staging, &out).map(|()| written));
    match assembled {
        Ok((index, unusable)) => Ok(Packaged::Written(Box::new(Written {
            path: out,
            index,
            unusable,
        }))),
        Err(failure) => {
            drop(fs::remove_dir_all(&staging));
            Err(failure)
        }
    }
}

/// Which run to package: the one named, or the newest.
fn where_to_look(args: &BundleArgs) -> PathBuf {
    args.run
        .clone()
        .unwrap_or_else(|| args.project.join(TREMULA_DIR).join(RUNS_DIR).join(LATEST))
}

/// Where a bundle is assembled: a sibling of where it will end up, so the rename
/// that publishes it stays on one filesystem.
fn staging_beside(out: &Path) -> PathBuf {
    let mut name = OsString::from(out.as_os_str());
    name.push(STAGING_SUFFIX);
    PathBuf::from(name)
}

/// Take away whatever a previous attempt left under the staging name.
///
/// It is never the published bundle — that name is the output path without the
/// suffix — so there is nothing here a reader could be relying on, and refusing to
/// start because an interrupted run left one behind would need a person to delete a
/// directory they did not create.
fn clear(staging: &Path) -> Result<(), BundleFailure> {
    let unwritable = |reason: String| BundleFailure::Unwritable {
        path: staging.to_path_buf(),
        reason,
    };
    if staging.exists() {
        fs::remove_dir_all(staging).map_err(|err| unwritable(err.to_string()))?;
    }
    fs::create_dir_all(staging).map_err(|err| unwritable(err.to_string()))
}

/// Put everything into the staging directory, and check what came out.
fn assemble(
    args: &BundleArgs,
    evidence: &Evidence,
    staging: &Path,
) -> Result<(BundleIndex, Vec<String>), BundleFailure> {
    let documents = Documents {
        report: copy(staging, REPORT, &evidence.report.bytes)?,
        manifest: copy(staging, MANIFEST, &evidence.manifest.bytes)?,
        results: copy(staging, RESULTS, &evidence.results.bytes)?,
        baseline: copy(staging, BASELINE, &evidence.baseline.bytes)?,
        triage: match &evidence.triage {
            Some(triage) => Some(copy(staging, TRIAGE, &triage.bytes)?),
            None => None,
        },
    };
    let mut attaching = collect::patches(evidence, staging)?;
    // The logs go through the machine the run happened on, which is the project as the
    // caller spelled it and the run as the caller pointed at it — both spellings of both,
    // since a log carries whichever form the process that printed it had.
    let carried = if args.no_logs {
        logs::Carried::default()
    } else {
        let machine = Machine::around(&args.project, &where_to_look(args));
        logs::attach(evidence, staging, &machine, &mut attaching.attachments)?
    };
    let run = &evidence.report.value.run;
    let index = BundleIndex {
        schema_version: SCHEMA_VERSION.to_owned(),
        run_id: evidence.run_id.clone(),
        tremula_version: env!("CARGO_PKG_VERSION").to_owned(),
        project: project_name(&args.project)?,
        documents,
        base: Base {
            revision: run.observed_revision.clone(),
            dirty: run.dirty,
            reproducible_from_revision: run.observed_revision.is_some() && !run.dirty,
        },
        suite: Suite {
            tests: run.tests.clone(),
            collected: evidence.baseline.value.runner.collected,
        },
        exposure: Exposure {
            patch_context: attaching
                .attachments
                .iter()
                .any(|attachment| attachment.patch.is_some()),
            standalone_logs: attaching
                .attachments
                .iter()
                .any(|attachment| attachment.log.is_some()),
            backend_raw_output: true,
            absolute_paths: true,
        },
        attachments: attaching.attachments,
        logs_truncated: carried.truncated,
    };
    write(staging, START_HERE, start_here::render(&index).as_bytes())?;
    let mut document =
        serde_json::to_string_pretty(&index).map_err(|err| BundleFailure::Unwritable {
            path: staging.join(INDEX),
            reason: err.to_string(),
        })?;
    document.push('\n');
    write(staging, INDEX, document.as_bytes())?;
    as_the_index_says(staging, &index)?;
    Ok((index, attaching.unusable))
}

/// The project's name, and only its name.
///
/// Resolved rather than taken from the report: the report records the root as it was
/// spelled on the command line, which is routinely `.` and names nothing. This is
/// the reason the command takes `--project` at all — a run directory can be outside
/// the project, so there is no way to work it out from the run alone.
fn project_name(project: &Path) -> Result<String, BundleFailure> {
    let resolved = fs::canonicalize(project).map_err(|err| BundleFailure::PathUnresolvable {
        path: project.to_path_buf(),
        reason: err.to_string(),
    })?;
    Ok(resolved
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_owned())
}

/// Copy one document into the bundle, byte for byte, and hash what was copied.
///
/// Byte for byte and never re-serialized: a document written out again through this
/// version's own types is a document this version agrees with, and the hash a reader
/// checks has to be the hash of what the run actually wrote.
fn copy(staging: &Path, name: &str, bytes: &[u8]) -> Result<Attached, BundleFailure> {
    write(staging, name, bytes)?;
    Ok(Attached {
        path: name.to_owned(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}

/// Write one file into the staging directory.
pub(crate) fn write(staging: &Path, name: &str, bytes: &[u8]) -> Result<(), BundleFailure> {
    let path = staging.join(name);
    let unwritable = |reason: String| BundleFailure::Unwritable {
        path: path.clone(),
        reason,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| unwritable(err.to_string()))?;
    }
    fs::write(&path, bytes).map_err(|err| unwritable(err.to_string()))
}

/// Read every file the index names back off the disk and check its hash.
///
/// Before the rename and not after: the point of the hashes is that a reader can
/// trust them, and a bundle whose own index is wrong about it should never become a
/// bundle at all.
fn as_the_index_says(staging: &Path, index: &BundleIndex) -> Result<(), BundleFailure> {
    let mut named = vec![
        &index.documents.report,
        &index.documents.manifest,
        &index.documents.results,
        &index.documents.baseline,
    ];
    named.extend(index.documents.triage.as_ref());
    for attachment in &index.attachments {
        named.extend(attachment.patch.as_ref());
        named.extend(attachment.log.as_ref());
    }
    for attached in named {
        let path = staging.join(&attached.path);
        let bytes = fs::read(&path).map_err(|err| BundleFailure::Unreadable {
            path: path.clone(),
            reason: err.to_string(),
        })?;
        if format!("{:x}", Sha256::digest(&bytes)) != attached.sha256 {
            return Err(BundleFailure::IndexDisagrees {
                path: attached.path.clone(),
            });
        }
    }
    Ok(())
}

/// Move the finished bundle to where it was asked for.
fn publish(staging: &Path, out: &Path) -> Result<(), BundleFailure> {
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| BundleFailure::Unwritable {
            path: parent.to_path_buf(),
            reason: err.to_string(),
        })?;
    }
    fs::rename(staging, out).map_err(|err| BundleFailure::Unwritable {
        path: out.to_path_buf(),
        reason: err.to_string(),
    })
}

/// A path that survives a change of working directory.
fn absolute(path: &Path) -> Result<PathBuf, BundleFailure> {
    std::path::absolute(path).map_err(|err| BundleFailure::PathUnresolvable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })
}
