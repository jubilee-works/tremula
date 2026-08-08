//! The whole of a run, in the order the steps have to happen.
//!
//! Most of that order is forced rather than chosen. The project is claimed
//! before anything else, so no second run can start patching the same sources.
//! The revision is observed before the first artifact is written, or the run
//! directory itself would make a clean project look modified. The sources are
//! read once, during validation, and those same bytes become the snapshot. And
//! the snapshot is published as the newest run *before* the pack starts, because
//! the run that most needs recovering is the one that never finished.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tremula_contracts::{
    SCHEMA_VERSION, capabilities::Capabilities, manifest::Manifest, report::RunMeta,
};

use crate::{
    artifacts::{ArtifactError, Artifacts},
    console,
    pack::{self, PackError},
    provenance::{self, Observed},
    python_env::{self, EnvError, PythonEnv},
    report::{self, ReportError},
    run_dir::{self, Lock, LockError, RunDir, RunId, TREMULA_DIR},
    validation::{ValidationError, ValidationWarning, Verified, validate_manifest},
};

/// Every failure that stops the work reports the same code; the message says
/// which failure it was.
pub const EXIT_FAILURE: u8 = 2;

/// Where run directories go when the caller does not say.
const RUNS_DIR: &str = "runs";

/// The symlink `restore` follows when no run is named.
const LATEST: &str = "latest";

/// What `tremula run` was asked to do.
#[derive(Debug, clap::Args)]
pub struct RunArgs {
    /// Manifest of mutants to run.
    #[arg(long, value_name = "PATH")]
    pub manifest: PathBuf,
    /// Project root the manifest's paths are relative to.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
    /// Python interpreter of the project's environment. Discovered if omitted.
    #[arg(long, value_name = "PATH")]
    pub python: Option<PathBuf>,
    /// Passed straight to the test runner; repeat for more than one. Omit for
    /// the project's own default collection.
    #[arg(long, value_name = "PATH")]
    pub tests: Vec<String>,
    /// Time limit for each run of the suite. Derived from the baseline if
    /// omitted.
    #[arg(long, value_name = "SECONDS", value_parser = positive_seconds)]
    pub timeout: Option<f64>,
    /// Where run directories are created. Defaults to `.tremula/runs` inside the
    /// project.
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// Print what the pack says while it is saying it.
    #[arg(short, long)]
    pub verbose: bool,
}

/// What `tremula restore` was asked to do.
#[derive(Debug, clap::Args)]
pub struct RestoreArgs {
    /// Run to restore from. Defaults to the project's most recent run.
    #[arg(long, value_name = "DIR")]
    pub run: Option<PathBuf>,
    /// Project root to restore into.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
}

/// What `tremula validate` was asked to do.
#[derive(Debug, clap::Args)]
pub struct ValidateArgs {
    /// Manifest to validate.
    #[arg(long, value_name = "PATH")]
    pub manifest: PathBuf,
    /// Project root the manifest's paths are relative to.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
    /// Python interpreter of the project's environment, for `--deep`.
    #[arg(long, value_name = "PATH")]
    pub python: Option<PathBuf>,
    /// Also run the language pack's checks, such as whether each replacement
    /// parses.
    #[arg(long)]
    pub deep: bool,
}

/// A time limit, refused unless it is a positive number of seconds. Zero would
/// be obeyed exactly: every suite killed as it started, every mutant a timeout,
/// and a report that blamed the code for it.
fn positive_seconds(spelling: &str) -> Result<f64, String> {
    let seconds: f64 = spelling
        .parse()
        .map_err(|_| format!("`{spelling}` is not a number of seconds"))?;
    if seconds > 0.0 {
        return Ok(seconds);
    }
    Err(format!(
        "a time limit has to be more than zero seconds, not `{spelling}`"
    ))
}

/// Anything that stops a run before it can produce a report.
#[derive(Debug, thiserror::Error)]
pub enum RunFailure {
    /// The manifest could not be read.
    #[error("cannot read manifest `{}`: {reason}; check the path and try again", path.display())]
    ManifestUnreadable {
        /// The path that was given.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The manifest is not a manifest.
    #[error(
        "`{}` is not a valid manifest: {reason}; compare it against the manifest schema",
        path.display()
    )]
    ManifestInvalid {
        /// The path that was given.
        path: PathBuf,
        /// Why it could not be read as a manifest.
        reason: String,
    },
    /// The manifest was written against another version of the contract.
    #[error(
        "the manifest declares schema version {found} and this tremula reads {SCHEMA_VERSION}; regenerate the manifest, or use a tremula that reads {found}"
    )]
    ManifestVersion {
        /// The version the manifest declares.
        found: String,
    },
    /// A path on the command line could not be resolved.
    #[error("cannot resolve `{}`: {reason}; check the path and try again", path.display())]
    PathUnresolvable {
        /// The path that was given.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The run's own directory could not be prepared.
    #[error(
        "cannot prepare the run directory under `{}`: {reason}; check that the directory is writable",
        parent.display()
    )]
    RunDirUnusable {
        /// Where run directories were to be created.
        parent: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The documents the pack left cannot be judged.
    #[error(transparent)]
    Artifacts(#[from] ArtifactError),
    /// The report could not be written.
    #[error("cannot write `{}`: {reason}; check that the run directory is writable", path.display())]
    ReportUnwritable {
        /// The document that could not be written.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The project is already being run.
    #[error(transparent)]
    Locked(#[from] LockError),
    /// The manifest does not agree with the project's files.
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    /// The project's Python could not be used.
    #[error(transparent)]
    Environment(#[from] EnvError),
    /// The pack could not do its work.
    #[error(transparent)]
    Pack(#[from] PackError),
    /// The pack's results do not cover the manifest.
    #[error(transparent)]
    Judgement(#[from] ReportError),
}

/// Run every mutant in a manifest and report what the suite did to them.
#[must_use]
pub fn run(args: &RunArgs) -> ExitCode {
    match attempt(args) {
        Ok(code) => code,
        Err(failure) => {
            eprintln!("error: {failure}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// The run itself. Each step is either the reason the next one is safe, or the
/// reason it is meaningful.
fn attempt(args: &RunArgs) -> Result<ExitCode, RunFailure> {
    let started = OffsetDateTime::now_utc();
    let document = read_manifest(&args.manifest)?;
    let manifest = parse_manifest(&args.manifest, &document)?;
    let id = RunId::new(started, &sha256_hex(document.as_bytes()));
    let lock = Lock::acquire(&args.project, &id)?;
    if let Some(previous) = lock.reclaimed_from() {
        eprintln!("warning: took over the lock left by {previous}, which is no longer running");
    }
    let observed = provenance::observe(&args.project);
    let out_parent = args
        .out
        .clone()
        .unwrap_or_else(|| args.project.join(TREMULA_DIR).join(RUNS_DIR));
    if manifest.mutants.is_empty() {
        return nothing_to_test(args, &out_parent, &id, started, &observed);
    }
    let verified = validate_manifest(&manifest, &args.project)?;
    warn(&verified);
    let env = python_env::discover(args.python.as_deref(), &args.project)?;
    env.verify_pack()?;
    let capabilities = pack::handshake(&env)?;
    let run = reserve(&out_parent, &id, &verified)?;
    let recorded = meta(args, &run, started, &observed);
    let judged = execute(args, &env, &run, &manifest, &capabilities, recorded)?;
    drop(lock);
    Ok(judged)
}

/// Drive the pack and judge what it produced.
fn execute(
    args: &RunArgs,
    env: &PythonEnv,
    run: &RunDir,
    manifest: &Manifest,
    capabilities: &Capabilities,
    recorded: RunMeta,
) -> Result<ExitCode, RunFailure> {
    let outcome = pack::run(
        env,
        &absolute(&args.manifest)?,
        &absolute(&args.project)?,
        run.path(),
        &args.tests,
        args.timeout,
        args.verbose,
    );
    if let Err(failure) = outcome {
        say_whether_the_sources_survived(&args.project, manifest);
        return Err(failure.into());
    }
    let left = Artifacts::load(run, capabilities)?;
    let report = report::build_report(manifest, &left.results, &left.baseline, recorded)?;
    write_json(&run.path().join("report.json"), &report)?;
    println!("{}", console::render(&report, Some(&left.baseline)));
    println!("run_dir={}", run.path().display());
    Ok(ExitCode::from(report.exit_code))
}

/// A manifest with no mutants is valid input, and still gets a run of its own:
/// a report, a run directory, and a place in `latest`. What it does not get is a
/// pack — there is nothing to ask one — or a snapshot, since nothing will be
/// touched.
fn nothing_to_test(
    args: &RunArgs,
    out_parent: &Path,
    id: &RunId,
    started: OffsetDateTime,
    observed: &Observed,
) -> Result<ExitCode, RunFailure> {
    let run = run_dir::create(out_parent, id).map_err(|err| RunFailure::RunDirUnusable {
        parent: out_parent.to_path_buf(),
        reason: err.to_string(),
    })?;
    let report = report::build_empty_report(meta(args, &run, started, observed));
    write_json(&run.path().join("report.json"), &report)?;
    publish(out_parent, &run)?;
    println!("{}", console::render(&report, None));
    println!("run_dir={}", run.path().display());
    Ok(ExitCode::from(report.exit_code))
}

/// Reserve the run's directory, keep the sources it is about to disturb, and
/// announce it as the newest run.
///
/// Announcing it now rather than after the pack finishes is deliberate: the run
/// a reader needs `tremula restore` to find is precisely the one that did not
/// finish, and pointing `latest` at the previous run would restore the wrong
/// sources.
fn reserve(out_parent: &Path, id: &RunId, verified: &Verified) -> Result<RunDir, RunFailure> {
    let unusable = |err: &io::Error| RunFailure::RunDirUnusable {
        parent: out_parent.to_path_buf(),
        reason: err.to_string(),
    };
    let run = run_dir::create(out_parent, id).map_err(|err| unusable(&err))?;
    run_dir::snapshot(run.path(), &verified.targets).map_err(|err| unusable(&err))?;
    publish(out_parent, &run)?;
    Ok(run)
}

fn publish(out_parent: &Path, run: &RunDir) -> Result<(), RunFailure> {
    run_dir::publish_latest(out_parent, run.path()).map_err(|err| RunFailure::RunDirUnusable {
        parent: out_parent.to_path_buf(),
        reason: err.to_string(),
    })
}

/// Report a manifest's non-fatal observations. Advice, so it does not change the
/// outcome.
fn warn(verified: &Verified) {
    for warning in &verified.warnings {
        match warning {
            ValidationWarning::LargeReplacement { mutant_id, bytes } => eprintln!(
                "warning: mutant {mutant_id}: replacement is {bytes} bytes; large replacements are carried through the run twice and will bloat the run directory"
            ),
        }
    }
}

/// Everything the report records about the run itself.
fn meta(args: &RunArgs, run: &RunDir, started: OffsetDateTime, observed: &Observed) -> RunMeta {
    provenance::run_meta(
        run.run_id(),
        &args.project.display().to_string(),
        started,
        observed,
    )
}

/// Say whether a failed run left the sources as it found them.
///
/// tremula does not put them back on its own — restoring is a command the reader
/// runs, so that a run which failed for a reason worth inspecting can be
/// inspected in the state it failed in. What it must not do is leave the reader
/// guessing, so the files are compared against what the manifest expected.
fn say_whether_the_sources_survived(project_root: &Path, manifest: &Manifest) {
    let intact = manifest.mutants.iter().all(|mutant| {
        fs::read(project_root.join(&mutant.file))
            .is_ok_and(|bytes| sha256_hex(&bytes) == mutant.base_file_sha256)
    });
    if intact {
        eprintln!("note: your source files are intact");
    } else {
        eprintln!("note: source files were left modified — run `tremula restore` to put them back");
    }
}

/// Put a document back where machines will look for it, by renaming a finished
/// file over it. A reader never finds a half-written report.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), RunFailure> {
    let unwritable = |reason: String| RunFailure::ReportUnwritable {
        path: path.to_path_buf(),
        reason,
    };
    let mut document =
        serde_json::to_string_pretty(value).map_err(|err| unwritable(err.to_string()))?;
    document.push('\n');
    let staging = path.with_extension("json.part");
    fs::write(&staging, document).map_err(|err| unwritable(err.to_string()))?;
    fs::rename(&staging, path).map_err(|err| unwritable(err.to_string()))
}

/// Read the manifest as text, keeping the exact bytes: the run's identity is
/// derived from them, and the pack is given the same file to copy.
fn read_manifest(path: &Path) -> Result<String, RunFailure> {
    fs::read_to_string(path).map_err(|err| RunFailure::ManifestUnreadable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })
}

/// The manifest's Rust type is the schema, so deserializing it is the schema
/// check. The declared version is checked separately, because a document from a
/// future contract can still happen to parse.
fn parse_manifest(path: &Path, document: &str) -> Result<Manifest, RunFailure> {
    let manifest: Manifest =
        serde_json::from_str(document).map_err(|err| RunFailure::ManifestInvalid {
            path: path.to_path_buf(),
            reason: err.to_string(),
        })?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(RunFailure::ManifestVersion {
            found: manifest.schema_version,
        });
    }
    Ok(manifest)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A path the pack can use from a working directory of its own.
fn absolute(path: &Path) -> Result<PathBuf, RunFailure> {
    fs::canonicalize(path).map_err(|err| RunFailure::PathUnresolvable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })
}

/// Put a run's target files back the way they were before it started.
#[must_use]
pub fn restore(args: &RestoreArgs) -> ExitCode {
    let run = args
        .run
        .clone()
        .unwrap_or_else(|| args.project.join(TREMULA_DIR).join(RUNS_DIR).join(LATEST));
    match run_dir::restore(&run, &args.project) {
        Ok(restored) if restored.is_empty() => {
            println!("nothing to restore: the run changed no files");
            ExitCode::SUCCESS
        }
        Ok(restored) => {
            println!(
                "restored {} file(s) from {}:",
                restored.len(),
                run.display()
            );
            for file in &restored {
                println!("  {file}");
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("error: {failure}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Check a manifest against the project's files, and optionally against the
/// rules of its language.
#[must_use]
pub fn validate(args: &ValidateArgs) -> ExitCode {
    match check(args) {
        Ok(count) => {
            println!("manifest is valid: {count} mutant(s)");
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("error: {failure}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// The neutral checks always, the language pack's checks on request. The neutral
/// ones come first: a pack asked about a manifest that does not match the bytes
/// on disk would report a language problem that is really a stale manifest.
fn check(args: &ValidateArgs) -> Result<usize, RunFailure> {
    let document = read_manifest(&args.manifest)?;
    let manifest = parse_manifest(&args.manifest, &document)?;
    let verified = validate_manifest(&manifest, &args.project)?;
    warn(&verified);
    if args.deep {
        let env = python_env::discover(args.python.as_deref(), &args.project)?;
        env.verify_pack()?;
        pack::validate_deep(&env, &absolute(&args.manifest)?, &absolute(&args.project)?)?;
    }
    Ok(manifest.mutants.len())
}
