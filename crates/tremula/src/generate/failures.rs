//! Why a generation could not produce a manifest.
//!
//! Every message names the cause and the next action. They are what a person who
//! ran `tremula generate` and got nothing actually reads, so they are a public
//! surface and are pinned by tests as one.

use std::path::PathBuf;

use crate::{
    generate::choose::AT,
    pack::PackError,
    python_env::EnvError,
    suppressions::SuppressionError,
    validation::{TargetFileError, ValidationError},
};

/// Anything that stops a generation before it can write a manifest.
#[derive(Debug, thiserror::Error)]
pub enum GenerationFailure {
    /// There is already a manifest where this one would go.
    #[error(
        "`{}` already exists; a generated manifest is never written over an existing file — move it aside, or pass `--out` with somewhere else to write",
        path.display()
    )]
    ManifestExists {
        /// The path that is taken.
        path: PathBuf,
    },
    /// The file to mutate cannot be used.
    #[error(transparent)]
    TargetFile(#[from] TargetFileError),
    /// The record of what a person has dismissed cannot be used.
    #[error(transparent)]
    Suppressions(#[from] SuppressionError),
    /// The project's Python could not be used.
    #[error(transparent)]
    Environment(#[from] EnvError),
    /// The pack could not do its work.
    #[error(transparent)]
    Pack(#[from] PackError),
    /// The pack's report is about another file than the one that was asked about.
    #[error(
        "the language pack was asked where a mutation may land in `{asked}` and answered about `{answered}`; every span in that answer is an offset into another file, so nothing was generated — check that the pack is the one this project installed, and if it repeats, report it"
    )]
    ReportIsAboutAnotherFile {
        /// The file the generation asked about.
        asked: String,
        /// The file the report names.
        answered: String,
    },
    /// The pack's report is about other bytes than the ones that were read.
    #[error(
        "`{file}` changed while it was being read: the language pack and tremula hashed different bytes for it; run again with nothing else editing the project"
    )]
    ReportIsAboutOtherBytes {
        /// The file the two disagreed about.
        file: String,
    },
    /// No function of that name is in the file.
    #[error("`{file}` has no function called `{name}`; it has {available}")]
    NoSuchFunction {
        /// The name that was asked for.
        name: String,
        /// The file that was searched.
        file: String,
        /// What it does have, for the reader to choose from.
        available: String,
    },
    /// More than one function of that name is in the file.
    #[error(
        "`{file}` spells `{name}` {count} times, so the name does not say which one to mutate; add the span of the one you mean — {choices}"
    )]
    AmbiguousFunction {
        /// The name that is not unique.
        name: String,
        /// The file that spells it more than once.
        file: String,
        /// How many of them there are.
        count: usize,
        /// The spellings that would each name exactly one.
        choices: String,
    },
    /// The disambiguator is not a span.
    #[error(
        "`{spelling}` is not a function and a span; write it as `name{AT}START:END`, with the byte offsets the language pack reports for that function"
    )]
    UnreadableDisambiguator {
        /// What was written.
        spelling: String,
    },
    /// The sources changed under the generation.
    #[error(
        "`{file}` changed during generation, so every mutant derived from it names bytes that are no longer there; nothing was written — run again once the file has settled"
    )]
    SourcesChanged {
        /// The file that changed.
        file: String,
    },
    /// The manifest that was produced does not agree with the project's files.
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    /// A path on the command line could not be resolved.
    #[error("cannot resolve `{}`: {reason}; check the path and try again", path.display())]
    PathUnresolvable {
        /// The path that was given.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// A function's own source could not be cut out of the file.
    #[error(
        "the language pack places `{name}` at bytes {start}..{end} of `{file}`, which is not text this file holds; run again, and if it repeats, report it"
    )]
    FunctionSourceUnreadable {
        /// The function whose span could not be cut.
        name: String,
        /// The file it is in.
        file: String,
        /// Where the pack said it starts.
        start: u64,
        /// Where the pack said it ends.
        end: u64,
    },
    /// The manifest could not be written.
    #[error("cannot write `{}`: {reason}; check that the directory is writable", path.display())]
    ManifestUnwritable {
        /// The document that could not be written.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
}
