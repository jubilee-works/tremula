//! Driving a language pack over the protocol it publishes.
//!
//! Four rules hold for every call. The pack is run as an installed module
//! (`-m`) through an isolated interpreter (`-I`), so that neither the working
//! directory nor the environment can put a different `tremula_python` in front
//! of the one the core negotiated with. The paths the pack resolves for itself —
//! the manifest, the project root, the run directory — are handed over absolute,
//! because the pack starts subprocesses of its own with working directories of
//! their own; what the caller wrote after `--tests` is passed through exactly as
//! given, since it is the project's own test runner that interprets it. The
//! pack's own account of a failure — its last line of stdout — is the only
//! failure report the core reads, because an execution backend may discard
//! stderr. And no call leaves a pack process behind, however it ends.

use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

use tremula_contracts::{
    SCHEMA_VERSION,
    capabilities::Capabilities,
    pack_error::{PackError as PackErrorDocument, Stage},
};

use crate::{
    child::Reaped,
    python_env::{PACK_DISTRIBUTION, PythonEnv},
};

/// The subcommands a pack has to offer before the core will use it.
pub const REQUIRED_SUBCOMMANDS: [&str; 3] = ["run", "collect", "validate"];

/// Where a run keeps what the pack printed.
const PACK_LOG: &str = "pack.txt";

/// How much of a silent pack's output to quote back when it fails.
const TAIL_LINES: usize = 5;

/// Why work with the pack could not go on. `Reported` is the pack's own
/// diagnosis; everything else is the core's.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// The interpreter could not be started.
    #[error(
        "cannot start the language pack with `{}`: {reason}; check that the interpreter works and that `{PACK_DISTRIBUTION}` is installed in it",
        interpreter.display()
    )]
    NotStarted {
        /// The interpreter that would not start.
        interpreter: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// Reading the pack's output failed part way through.
    #[error("lost contact with the language pack while it was working: {reason}")]
    Unreadable {
        /// What the operating system reported.
        reason: String,
    },
    /// The handshake was not a capabilities document.
    #[error(
        "the language pack did not describe itself in a form this version understands ({reason}); update `{PACK_DISTRIBUTION}` in the target project"
    )]
    UnreadableHandshake {
        /// Why the document could not be read.
        reason: String,
    },
    /// The pack implements a different version of the contract.
    #[error(
        "the language pack speaks contract version {pack} and this tremula speaks {SCHEMA_VERSION}; update `{PACK_DISTRIBUTION}` in the target project so both sides match"
    )]
    ContractMismatch {
        /// The version the pack reported.
        pack: String,
    },
    /// The pack cannot do everything a run needs.
    #[error(
        "the language pack cannot {}, which a run needs; update `{PACK_DISTRIBUTION}` in the target project",
        missing.join(" or ")
    )]
    MissingSubcommands {
        /// The subcommands the pack did not offer.
        missing: Vec<String>,
    },
    /// The pack reports no language-level checks at all.
    #[error(
        "the language pack `{pack}` performs no checks of its own on a manifest, so nothing would confirm a mutant is valid for this language; update `{PACK_DISTRIBUTION}` in the target project"
    )]
    NoLanguageChecks {
        /// The pack that reported an empty list, for the reader to identify.
        pack: String,
    },
    /// The pack diagnosed its own failure.
    ///
    /// The stage is translated into what it means for the reader; the pack's own
    /// code is kept alongside, because that is what stays stable across
    /// rewordings and is what a bug report should carry.
    #[error("{}: {message} ({code})", meaning(*stage))]
    Reported {
        /// How far the pack got.
        stage: Stage,
        /// The pack's stable identifier for this kind of failure.
        code: String,
        /// The pack's own explanation.
        message: String,
    },
    /// The pack failed without diagnosing itself.
    #[error(
        "the language pack stopped without saying why ({status}){}{}",
        kept_in(log.as_ref()),
        quote(tail)
    )]
    Crashed {
        /// How the process ended.
        status: String,
        /// Where the whole transcript is, when the call had somewhere to keep
        /// one.
        log: Option<PathBuf>,
        /// The last few lines it printed.
        tail: Vec<String>,
    },
}

/// Point the reader at the transcript, when there is one to point at. Calls that
/// belong to no run — the handshake, a validation — keep nothing.
fn kept_in(log: Option<&PathBuf>) -> String {
    match log {
        Some(path) => format!("; its output is kept in `{}`", path.display()),
        None => String::new(),
    }
}

/// What a stage means to someone who has never read the pack's source. The stage
/// itself is the pack's vocabulary; this is the reader's.
fn meaning(stage: Stage) -> &'static str {
    match stage {
        Stage::Preflight => "the project's environment is not ready to run mutants",
        Stage::Spans => "the functions of a source file could not be read",
        Stage::Validate => "a mutant in the manifest is not valid for this language",
        Stage::Baseline => "your test suite failed before any mutants were tried",
        Stage::Plan => "the project layout could not be prepared",
        Stage::Execute => "the mutants could not be run",
        Stage::Collect => "the results of the run could not be read back",
        Stage::Unknown => "the pack stopped at a step this version of tremula does not know",
    }
}

/// Quote a few lines of output inside a one-line message.
fn quote(tail: &[String]) -> String {
    if tail.is_empty() {
        return String::new();
    }
    format!(", and it last said: {}", tail.join(" / "))
}

/// Negotiate with the pack installed in `env`.
///
/// The core refuses a pack it cannot rely on rather than discovering the
/// mismatch half way through a run: the contract version has to match exactly,
/// every subcommand a run uses has to be offered, and a pack that performs no
/// language checks is not a pack this core can validate a manifest with.
///
/// # Errors
///
/// Returns [`PackError`] when the pack cannot be run, does not describe itself,
/// or describes itself as something the core cannot use.
pub fn handshake(env: &PythonEnv) -> Result<Capabilities, PackError> {
    let mut command = pack_command(env);
    command.arg("--capabilities").current_dir(neutral());
    let transcript = drive(env, command, None, false, &unwatched)?;
    if !transcript.succeeded() {
        return Err(transcript.into_failure(None));
    }
    let document = transcript.last_line().unwrap_or_default();
    let capabilities: Capabilities =
        serde_json::from_str(&document).map_err(|err| PackError::UnreadableHandshake {
            reason: err.to_string(),
        })?;
    check(&capabilities)?;
    Ok(capabilities)
}

/// Everything the core requires of a pack it has just met.
fn check(capabilities: &Capabilities) -> Result<(), PackError> {
    if capabilities.contract_version != SCHEMA_VERSION {
        return Err(PackError::ContractMismatch {
            pack: capabilities.contract_version.clone(),
        });
    }
    let missing: Vec<String> = REQUIRED_SUBCOMMANDS
        .iter()
        .filter(|required| !capabilities.subcommands.iter().any(|had| had == *required))
        .map(|required| (*required).to_owned())
        .collect();
    if !missing.is_empty() {
        return Err(PackError::MissingSubcommands { missing });
    }
    if capabilities.validate_checks.is_empty() {
        return Err(PackError::NoLanguageChecks {
            pack: capabilities.name.clone(),
        });
    }
    Ok(())
}

/// What one pack run is asked to do.
#[derive(Debug)]
pub struct RunRequest<'a> {
    /// The manifest of mutants to run, absolute.
    pub manifest: &'a Path,
    /// The project the mutants belong to, absolute.
    pub project_root: &'a Path,
    /// The run's own directory, absolute.
    pub run_dir: &'a Path,
    /// What to hand the project's test runner, exactly as the caller wrote it.
    pub tests: &'a [String],
    /// How long one run of the suite may take. The pack derives a limit from the
    /// baseline when there is none.
    pub timeout: Option<f64>,
    /// Print what the pack says while it is saying it.
    pub verbose: bool,
}

/// Run every mutant in the request's manifest and leave the run's documents in
/// its run directory.
///
/// `tests` and `timeout` are passed through untouched: what to collect and how
/// long a suite may take are the project's business, not the core's.
///
/// `watch` is told the pack's process while it runs, and told there is none once
/// it has been collected. That is what lets a run record the process that is
/// touching the sources, so a run killed mid-flight does not read as one whose
/// pack is gone.
///
/// # Errors
///
/// Returns [`PackError`] when the pack cannot be started, reports a failure of
/// its own, or stops without reporting one.
pub fn run(
    env: &PythonEnv,
    request: &RunRequest<'_>,
    watch: &dyn Fn(Option<u32>),
) -> Result<(), PackError> {
    let mut command = pack_command(env);
    command
        .arg("run")
        .args(["--manifest".as_ref(), request.manifest.as_os_str()])
        .args(["--project".as_ref(), request.project_root.as_os_str()])
        .args(["--out".as_ref(), request.run_dir.as_os_str()])
        .current_dir(request.project_root);
    for named in request.tests {
        command.args(["--tests", named]);
    }
    if let Some(seconds) = request.timeout {
        command.args(["--timeout".to_owned(), seconds.to_string()]);
    }
    let log = request.run_dir.join("logs").join(PACK_LOG);
    let transcript = drive(env, command, Some(&log), request.verbose, watch)?;
    if transcript.succeeded() {
        return Ok(());
    }
    Err(transcript.into_failure(Some(&log)))
}

/// Ask the pack to check the manifest against the rules of its language.
///
/// # Errors
///
/// Returns [`PackError`] when the pack cannot be started, or when it rejects a
/// mutant.
pub fn validate_deep(
    env: &PythonEnv,
    manifest: &Path,
    project_root: &Path,
) -> Result<(), PackError> {
    let mut command = pack_command(env);
    command
        .arg("validate")
        .args(["--manifest".as_ref(), manifest.as_os_str()])
        .args(["--project".as_ref(), project_root.as_os_str()])
        .current_dir(project_root);
    let transcript = drive(env, command, None, false, &unwatched)?;
    if transcript.succeeded() {
        return Ok(());
    }
    Err(transcript.into_failure(None))
}

/// Nobody is recording the process of a call that belongs to no run: the
/// handshake and a validation hold no lock to record it in.
fn unwatched(_pack: Option<u32>) {}

/// How every pack subcommand is invoked.
fn pack_command(env: &PythonEnv) -> Command {
    let mut command = Command::new(env.interpreter());
    command.args(["-I", "-m", "tremula_python"]);
    command
}

/// A directory that belongs to no project, for calls that read nothing.
fn neutral() -> PathBuf {
    std::env::temp_dir()
}

/// What one pack call did and said.
struct Transcript {
    status: ExitStatus,
    lines: Vec<String>,
}

impl Transcript {
    fn succeeded(&self) -> bool {
        self.status.success()
    }

    fn last_line(&self) -> Option<String> {
        self.lines.last().cloned()
    }

    /// Read the pack's own diagnosis out of its last line, or report that it
    /// gave none. Only the last line counts: everything before it is
    /// diagnostics, however much it may look like a document.
    fn into_failure(self, log: Option<&Path>) -> PackError {
        if let Some(Ok(document)) = self
            .lines
            .last()
            .map(|last| serde_json::from_str::<PackErrorDocument>(last))
        {
            return PackError::Reported {
                stage: document.error.stage,
                code: document.error.code,
                message: document.error.message,
            };
        }
        let from = self.lines.len().saturating_sub(TAIL_LINES);
        PackError::Crashed {
            status: describe(self.status),
            log: log.map(Path::to_path_buf),
            tail: self.lines[from..].to_vec(),
        }
    }
}

/// How a process ended, in words rather than in a number nobody can place.
fn describe(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("it exited with status {code}"),
        None => "it was stopped by a signal".to_owned(),
    }
}

/// Run one pack call, keeping everything it printed.
///
/// Whatever happened, the pack's process is over by the time this returns: it
/// either finished on its own or was stopped by the guard [`listen`] holds. That
/// is what makes it safe to tell `watch` there is no pack process any more, and
/// what keeps a failed call from leaving one behind still editing the sources.
fn drive(
    env: &PythonEnv,
    command: Command,
    log: Option<&Path>,
    verbose: bool,
    watch: &dyn Fn(Option<u32>),
) -> Result<Transcript, PackError> {
    let listened = listen(env, command, verbose, watch);
    watch(None);
    let transcript = listened?;
    if let Some(path) = log {
        // A log nobody could write is not worth failing a finished run over: the
        // pack's own answer is in hand either way.
        let mut kept = transcript.lines.join("\n");
        kept.push('\n');
        drop(fs::write(path, kept));
    }
    Ok(transcript)
}

/// Start the pack and read it until it stops.
///
/// Only stdout is captured, because that is where the pack's protocol lives.
/// stderr is left alone: the pack promises silence there, and if it breaks that
/// promise the reader should see it rather than have it filed away.
fn listen(
    env: &PythonEnv,
    mut command: Command,
    verbose: bool,
    watch: &dyn Fn(Option<u32>),
) -> Result<Transcript, PackError> {
    command.stdout(Stdio::piped());
    let started = command.spawn().map_err(|err| PackError::NotStarted {
        interpreter: env.interpreter().to_path_buf(),
        reason: err.to_string(),
    })?;
    let mut pack = Reaped::new(started);
    watch(pack.pid());
    let mut lines = Vec::new();
    if let Some(stdout) = pack.stdout() {
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|err| PackError::Unreadable {
                reason: err.to_string(),
            })?;
            if verbose {
                println!("{line}");
            }
            lines.push(line);
        }
    }
    let status = pack.wait().map_err(|err| PackError::Unreadable {
        reason: err.to_string(),
    })?;
    Ok(Transcript { status, lines })
}
