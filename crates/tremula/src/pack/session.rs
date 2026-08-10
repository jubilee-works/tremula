//! Running one pack call and keeping everything it said.
//!
//! The mechanics every call shares: how the interpreter is invoked, where a call that
//! belongs to no project runs, and the reading of a process that has to be over —
//! however it ended — by the time anything is made of its output.

use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

use tremula_contracts::pack_error::PackError as PackErrorDocument;

use crate::{child::Reaped, pack::failures::PackError, python_env::PythonEnv};

/// How much of a silent pack's output to quote back when it fails.
const TAIL_LINES: usize = 5;

/// Nobody is recording the process of a call that belongs to no run: the
/// handshake and a validation hold no lock to record it in.
pub(super) fn unwatched(_pack: Option<u32>) {}

/// How every pack subcommand is invoked.
pub(super) fn pack_command(env: &PythonEnv) -> Command {
    let mut command = Command::new(env.interpreter());
    command.args(["-I", "-m", "tremula_python"]);
    command
}

/// A directory that belongs to no project, for calls that read nothing.
pub(super) fn neutral() -> PathBuf {
    std::env::temp_dir()
}

/// What one pack call did and said.
pub(super) struct Transcript {
    status: ExitStatus,
    lines: Vec<String>,
}

impl Transcript {
    pub(super) fn succeeded(&self) -> bool {
        self.status.success()
    }

    pub(super) fn last_line(&self) -> Option<String> {
        self.lines.last().cloned()
    }

    /// Read the pack's own diagnosis out of its last line, or report that it
    /// gave none. Only the last line counts: everything before it is
    /// diagnostics, however much it may look like a document.
    pub(super) fn into_failure(self, log: Option<&Path>) -> PackError {
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
pub(super) fn drive(
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
pub(super) fn listen(
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
