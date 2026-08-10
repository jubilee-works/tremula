//! The calls a pack answers, one function each.
//!
//! Every one of them is the same shape: build the command, drive it to the end, and
//! read the pack's last line of stdout as the document it promised — or as the
//! failure it diagnosed. What differs between them is only what the call is for, and
//! the rules they all keep are in the module above.

use std::path::Path;

use tremula_contracts::{manifest::Span, probe::ProbeReport, spans::SpansReport};

use crate::{
    pack::{
        PROBE_SUBCOMMAND, SPANS_SUBCOMMAND,
        failures::PackError,
        session::{drive, neutral, pack_command, unwatched},
    },
    python_env::PythonEnv,
};

/// Where a run keeps what the pack printed.
const PACK_LOG: &str = "pack.txt";

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

/// Ask the pack where a mutation may land in one file.
///
/// The file is a spelling and not a path the core resolves for the pack: the
/// contract's paths are POSIX and project-relative, and the pack draws the line
/// where the core draws it — refusing, through its own error channel, a file whose
/// bytes no run could address by offset and a path that leads out of the project.
///
/// # Errors
///
/// Returns [`PackError`] when the pack cannot be started, refuses the file, or
/// answers with something that is not a spans document.
pub fn spans(env: &PythonEnv, project_root: &Path, file: &str) -> Result<SpansReport, PackError> {
    let mut command = pack_command(env);
    command
        .arg(SPANS_SUBCOMMAND)
        .args(["--file", file])
        .args(["--project".as_ref(), project_root.as_os_str()])
        .current_dir(project_root);
    let transcript = drive(env, command, None, false, &unwatched)?;
    if !transcript.succeeded() {
        return Err(transcript.into_failure(None));
    }
    serde_json::from_str(&transcript.last_line().unwrap_or_default()).map_err(|err| {
        PackError::UnreadableSpans {
            file: file.to_owned(),
            reason: err.to_string(),
        }
    })
}

/// What one probe is asked to run.
#[derive(Debug)]
pub struct ProbeRequest<'a> {
    /// The root the file is found under, absolute. Triage passes a run's
    /// `snapshot/`, so what is probed is the bytes the run was measured against
    /// rather than whatever the working tree holds now.
    pub root: &'a Path,
    /// The file holding the function, POSIX-style and relative to that root.
    pub file: &'a str,
    /// The half-open byte range the mutation replaces.
    pub span: Span,
    /// The text to put in the span's place.
    pub replacement: &'a str,
    /// One call of the function, with literal arguments only.
    pub call: &'a str,
}

/// Ask the pack to run one input against both versions of one function.
///
/// The answer is an observation and not a verdict: it says what each version did,
/// and it is the caller that decides what that is worth. A witness the pack will
/// not evaluate, a value it cannot compare, and a run it had to kill all come back
/// as an answer rather than as a failure, because each of those is a fact about the
/// witness and the pack was right to report it.
///
/// # Errors
///
/// Returns [`PackError`] when the pack cannot be started, refuses the file or the
/// span, could not import either version, or answers with something that is not a
/// probe report.
pub fn probe(env: &PythonEnv, request: &ProbeRequest<'_>) -> Result<ProbeReport, PackError> {
    let mut command = pack_command(env);
    command
        .arg(PROBE_SUBCOMMAND)
        .args(["--file", request.file])
        .args(["--project".as_ref(), request.root.as_os_str()])
        .args([
            "--span".to_owned(),
            format!("{}:{}", request.span.start_byte, request.span.end_byte),
        ])
        .args(["--replacement", request.replacement])
        .args(["--call", request.call])
        .current_dir(neutral());
    let transcript = drive(env, command, None, false, &unwatched)?;
    if !transcript.succeeded() {
        return Err(transcript.into_failure(None));
    }
    serde_json::from_str(&transcript.last_line().unwrap_or_default()).map_err(|err| {
        PackError::UnreadableProbe {
            file: request.file.to_owned(),
            reason: err.to_string(),
        }
    })
}
