//! Finding the Python that belongs to the project under test, and confirming
//! that it can host the language pack.
//!
//! Which interpreter runs the pack decides which project gets measured, so the
//! search order is fixed and narrow: what the caller named, then the virtual
//! environment that is active, then the one the project keeps beside its
//! sources. tremula's own interpreter is never a candidate — the pack has to be
//! installed alongside the project's test suite to be able to run it.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Distribution name of the Python language pack, as it is installed.
pub const PACK_DISTRIBUTION: &str = "tremula-python";

/// Directory name of the virtual environment a project keeps beside its
/// sources.
const PROJECT_VENV: &str = ".venv";

/// One line of Python that reports the installed pack's version.
///
/// The import comes first so that a missing pack fails as a missing module
/// rather than as missing metadata, and the version is read from installed
/// distribution metadata rather than from the package: metadata is what proves
/// the pack is *installed*, which is also what registers the entry point its
/// execution backend resolves it through.
const PROBE: &str = "import tremula_python, importlib.metadata as metadata; \
                     print(metadata.version('tremula-python'))";

/// Why the project's Python could not be used. Every message names a next
/// action, because each of these is something the reader has to change.
#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    /// Nothing in the search order exists.
    #[error(
        "no Python interpreter found for this project (looked for {}); create a virtual environment in the project, or pass --python with the interpreter to use",
        list(candidates)
    )]
    NoInterpreter {
        /// Every path that was looked for, in the order they were tried.
        candidates: Vec<PathBuf>,
    },
    /// The caller named an interpreter that is not there.
    #[error(
        "no interpreter at `{}`; pass --python with the path to the Python of the project's virtual environment",
        path.display()
    )]
    InterpreterMissing {
        /// The path that was named.
        path: PathBuf,
    },
    /// The interpreter is there but could not be executed.
    #[error(
        "cannot run `{}`: {reason}; pass --python with the path to a working Python interpreter",
        interpreter.display()
    )]
    InterpreterUnusable {
        /// The interpreter that would not run.
        interpreter: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The interpreter runs, but the pack is not installed in it.
    #[error(
        "the Python at `{}` cannot load the tremula language pack ({reason}); run `uv add --dev {PACK_DISTRIBUTION}` in the target project",
        interpreter.display()
    )]
    PackMissing {
        /// The interpreter that has no pack.
        interpreter: PathBuf,
        /// What the interpreter said when it tried.
        reason: String,
    },
    /// The pack is installed but reported no version.
    #[error(
        "the tremula language pack in `{}` reported no version; reinstall it with `uv add --dev {PACK_DISTRIBUTION}` in the target project",
        interpreter.display()
    )]
    PackUnreadable {
        /// The interpreter whose pack answered with nothing.
        interpreter: PathBuf,
    },
}

/// Render candidate paths for a message, in the order they were tried.
fn list(candidates: &[PathBuf]) -> String {
    candidates
        .iter()
        .map(|candidate| format!("`{}`", candidate.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The language pack installed in one interpreter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackInfo {
    /// Distribution name, which is always [`PACK_DISTRIBUTION`].
    pub distribution: String,
    /// Version the interpreter's installed metadata reports.
    pub version: String,
}

/// A Python interpreter the pack will be run through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonEnv {
    interpreter: PathBuf,
}

impl PythonEnv {
    /// The environment of the interpreter at `interpreter`, whether or not it
    /// exists yet.
    pub fn at(interpreter: impl Into<PathBuf>) -> Self {
        Self {
            interpreter: interpreter.into(),
        }
    }

    /// The interpreter every pack subprocess is started from.
    #[must_use]
    pub fn interpreter(&self) -> &Path {
        &self.interpreter
    }

    /// Confirm the pack is installed in this interpreter, and report its
    /// version.
    ///
    /// The check is "distribution metadata reports a version", not "the pack's
    /// source sits under the interpreter's own directories": an editable
    /// install — how the pack is developed, and a perfectly ordinary way to
    /// depend on it — keeps its source anywhere it likes. Metadata is also the
    /// stronger claim, because it is what registers the entry point the
    /// execution backend resolves the pack through.
    ///
    /// The probe runs isolated and from a neutral directory, so a directory
    /// named `tremula_python` next to whatever the user was standing in cannot
    /// answer for the installed pack.
    ///
    /// # Errors
    ///
    /// Returns [`EnvError`] when the interpreter cannot be run, or when it can
    /// but has no pack installed.
    pub fn verify_pack(&self) -> Result<PackInfo, EnvError> {
        let probed = Command::new(&self.interpreter)
            .args(["-I", "-c", PROBE])
            .current_dir(std::env::temp_dir())
            .output()
            .map_err(|err| EnvError::InterpreterUnusable {
                interpreter: self.interpreter.clone(),
                reason: err.to_string(),
            })?;
        if !probed.status.success() {
            return Err(EnvError::PackMissing {
                interpreter: self.interpreter.clone(),
                reason: last_line(&probed.stderr),
            });
        }
        let version = String::from_utf8_lossy(&probed.stdout).trim().to_owned();
        if version.is_empty() {
            return Err(EnvError::PackUnreadable {
                interpreter: self.interpreter.clone(),
            });
        }
        Ok(PackInfo {
            distribution: PACK_DISTRIBUTION.to_owned(),
            version,
        })
    }
}

/// The last thing a failing interpreter said, which is the line a traceback puts
/// its diagnosis on.
fn last_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("it reported nothing")
        .to_owned()
}

/// Find the interpreter to run the pack through.
///
/// The order is `explicit`, then the active virtual environment, then the
/// project's own `.venv`.
///
/// # Errors
///
/// Returns [`EnvError`] when `explicit` names nothing, or when no candidate
/// exists.
pub fn discover(explicit: Option<&Path>, project_root: &Path) -> Result<PythonEnv, EnvError> {
    let active = std::env::var_os("VIRTUAL_ENV").map(PathBuf::from);
    discover_from(explicit, active.as_deref(), project_root)
}

/// [`discover`], with the active virtual environment supplied rather than read
/// from the process environment.
///
/// Separating the two is what lets the search order be exercised: a test cannot
/// safely change the environment of a process it shares with other tests.
///
/// # Errors
///
/// Returns [`EnvError`] when `explicit` names nothing, or when no candidate
/// exists.
pub fn discover_from(
    explicit: Option<&Path>,
    active_virtualenv: Option<&Path>,
    project_root: &Path,
) -> Result<PythonEnv, EnvError> {
    if let Some(named) = explicit {
        if named.is_file() {
            return Ok(PythonEnv::at(named));
        }
        return Err(EnvError::InterpreterMissing {
            path: named.to_path_buf(),
        });
    }
    let mut candidates = Vec::with_capacity(2);
    if let Some(active) = active_virtualenv {
        candidates.push(interpreter_in(active));
    }
    candidates.push(interpreter_in(&project_root.join(PROJECT_VENV)));
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(PythonEnv::at(candidate));
        }
    }
    Err(EnvError::NoInterpreter { candidates })
}

/// Where a virtual environment keeps its interpreter.
fn interpreter_in(virtualenv: &Path) -> PathBuf {
    virtualenv.join("bin").join("python")
}
