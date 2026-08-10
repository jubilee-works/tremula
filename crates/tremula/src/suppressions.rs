//! Reading and writing the record of what a person has dismissed.
//!
//! Two consumers, and both of them read the file before anything is paid for. A
//! generation drops a candidate somebody has already dismissed rather than
//! recording it into a manifest; a triage leaves a dismissed survivor out of the
//! judging rather than asking a model about it again. Both of those are cheaper the
//! earlier they happen, and both would be worse than useless if the file turned out
//! to be unreadable half way through — so a document that cannot be read is a
//! failure before the first call and never after it.
//!
//! Matching is by the mutation and not by the mutant's identifier. See the contract
//! for why; what follows from it here is [`Dismissals::stale_for`], which counts the
//! decisions whose text is no longer in the file and keeps every one of them.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use tremula_contracts::{
    SCHEMA_VERSION,
    suppressions::{Suppression, Suppressions},
};

/// Where a project keeps its dismissals unless the caller says otherwise.
///
/// Beside the manifest and meant to be committed: it is the project's decision, and
/// a decision one person's machine remembers is one the next person makes again.
pub const DEFAULT_SUPPRESSIONS: &str = "tremula-suppressions.json";

/// Why the record of dismissals could not be used.
#[derive(Debug, thiserror::Error)]
pub enum SuppressionError {
    /// The file is there and could not be read.
    #[error("cannot read `{}`: {reason}; check the path and its permissions", path.display())]
    Unreadable {
        /// The file that could not be read.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
    /// The file is there and is not this document.
    #[error(
        "`{}` is not a list of dismissals this version can read: {reason}; compare it against the suppressions schema, or move it aside",
        path.display()
    )]
    Invalid {
        /// The file that could not be read as itself.
        path: PathBuf,
        /// Why it could not be.
        reason: String,
    },
    /// The file was written against another version of the contract.
    #[error(
        "`{}` declares schema version {found} and this tremula reads {SCHEMA_VERSION}; use a tremula that reads {found}, or move the file aside",
        path.display()
    )]
    Version {
        /// The file that declares another version.
        path: PathBuf,
        /// The version it declares.
        found: String,
    },
    /// The file could not be written.
    #[error("cannot write `{}`: {reason}; check that the directory is writable", path.display())]
    Unwritable {
        /// The file that could not be written.
        path: PathBuf,
        /// What the operating system reported.
        reason: String,
    },
}

/// Every dismissal a project has recorded.
#[derive(Debug, Clone, Default)]
pub struct Dismissals {
    entries: Vec<Suppression>,
}

impl Dismissals {
    /// Read the dismissals at `path`.
    ///
    /// A file that is not there is not an error and not an absence to warn about:
    /// a project that has dismissed nothing has no such file, which is the ordinary
    /// state of every project until somebody dismisses something.
    ///
    /// # Errors
    ///
    /// Returns [`SuppressionError`] when the file is there and cannot be read, is
    /// not this document, or declares another version of the contract.
    pub fn load(path: &Path) -> Result<Self, SuppressionError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => {
                return Err(SuppressionError::Unreadable {
                    path: path.to_path_buf(),
                    reason: err.to_string(),
                });
            }
        };
        let document: Suppressions =
            serde_json::from_str(&text).map_err(|err| SuppressionError::Invalid {
                path: path.to_path_buf(),
                reason: err.to_string(),
            })?;
        if document.schema_version != SCHEMA_VERSION {
            return Err(SuppressionError::Version {
                path: path.to_path_buf(),
                found: document.schema_version,
            });
        }
        Ok(Self {
            entries: document.suppressions,
        })
    }

    /// Every dismissal, in the order they were made.
    #[must_use]
    pub fn all(&self) -> &[Suppression] {
        &self.entries
    }

    /// Whether anything has been dismissed at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The decision covering this mutation, if a person has made one.
    #[must_use]
    pub fn covering(&self, file: &str, original: &str, replacement: &str) -> Option<&Suppression> {
        self.entries.iter().find(|entry| {
            entry.file == file && entry.original == original && entry.replacement == replacement
        })
    }

    /// How many decisions about `file` name text that is no longer in it.
    ///
    /// Counted and reported, never acted on. A decision may be stale because the
    /// code moved on, or because the reader is on a branch where it has not moved
    /// yet, and there is nothing here that can tell those apart — so the number is
    /// said out loud and every decision is kept.
    #[must_use]
    pub fn stale_for(&self, file: &str, source: &str) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.file == file && !source.contains(&entry.original))
            .count()
    }

    /// The same decisions with one more, or unchanged when it is already there.
    ///
    /// False means the decision was already recorded. Dismissing twice is something
    /// a person does — the survivor came back on a later run and they said so again —
    /// and it must not put the same line in the file twice.
    pub fn add(&mut self, decision: Suppression) -> bool {
        if self
            .covering(&decision.file, &decision.original, &decision.replacement)
            .is_some()
        {
            return false;
        }
        self.entries.push(decision);
        true
    }

    /// Write the decisions to `path`, by renaming a finished file over it.
    ///
    /// # Errors
    ///
    /// Returns [`SuppressionError::Unwritable`] when the document cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), SuppressionError> {
        let unwritable = |reason: String| SuppressionError::Unwritable {
            path: path.to_path_buf(),
            reason,
        };
        let document = Suppressions {
            schema_version: SCHEMA_VERSION.to_owned(),
            suppressions: self.entries.clone(),
        };
        let mut text =
            serde_json::to_string_pretty(&document).map_err(|err| unwritable(err.to_string()))?;
        text.push('\n');
        let staging = path.with_extension("json.part");
        fs::write(&staging, text).map_err(|err| unwritable(err.to_string()))?;
        fs::rename(&staging, path).map_err(|err| unwritable(err.to_string()))
    }
}

/// Say how many decisions no longer name anything in the file.
///
/// On the warning channel and never on the outcome: a stale decision changes nothing
/// about what a command does, and a reader who is told the number can go and look.
pub fn warn_about_stale(dismissals: &Dismissals, file: &str, source: &str) {
    let stale = dismissals.stale_for(file, source);
    if stale > 0 {
        eprintln!(
            "warning: {stale} stale suppression(s): the text they name is no longer in `{file}`; \
             they are kept, and nothing was dropped"
        );
    }
}
