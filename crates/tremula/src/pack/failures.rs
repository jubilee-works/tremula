//! Why work with a language pack could not go on.
//!
//! One error for every way a call can end other than with the document it was for.
//! `Reported` is the pack's own diagnosis of itself and every other variant is the
//! core's account of a pack that gave none — which is the distinction a reader needs
//! first, because the two are fixed in different places.

use std::path::PathBuf;

use tremula_contracts::{SCHEMA_VERSION, pack_error::Stage};

use crate::python_env::PACK_DISTRIBUTION;

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
    /// A document the pack was to read could not be written.
    #[error(
        "cannot write `{}` for the language pack to read: {reason}; check that the temporary directory is writable",
        path.display()
    )]
    Unwritable {
        /// The document that could not be written.
        path: PathBuf,
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
    /// The pack's answer about a file's functions was not that document.
    #[error(
        "the language pack's answer about `{file}` was not a report of where a mutation may land ({reason}); update `{PACK_DISTRIBUTION}` in the target project"
    )]
    UnreadableSpans {
        /// The file that was asked about.
        file: String,
        /// Why the document could not be read.
        reason: String,
    },
    /// The pack's answer about one witness was not that document.
    #[error(
        "the language pack's answer about running one input against `{file}` was not a report of what it observed ({reason}); update `{PACK_DISTRIBUTION}` in the target project"
    )]
    UnreadableProbe {
        /// The file the witness was about.
        file: String,
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
        "the language pack cannot {}, which this command needs; update `{PACK_DISTRIBUTION}` in the target project",
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
        Stage::Probe => "one input could not be run against both versions of a function",
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
