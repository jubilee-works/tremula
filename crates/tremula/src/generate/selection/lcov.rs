//! What a coverage document says about the project's lines.
//!
//! # What is read, and what is not
//!
//! Three records: `SF:` names the file a block is about, `DA:<line>,<hits>` says a line
//! is a statement and how many times it ran, and `end_of_record` closes the block.
//! Every other record a coverage tool writes — `TN`, `FN`, `FNDA`, `FNF`, `FNH`, `LF`,
//! `LH`, `BRDA`, `BRF`, `BRH` — is ignored, and that is a decision rather than an
//! omission. A function record says where a coverage tool thinks a function is, which
//! is a question the project's own language pack answers authoritatively; a branch
//! record is about a decision inside a line, which nothing here selects on; and the
//! summary counts are derivable from the lines. `coverage.py` emits all of them, so a
//! reader that refused what it did not recognise would refuse every real document.
//!
//! # The three things a line can be
//!
//! A line with hits is a candidate. A line the document measured and nothing reached
//! is a gap — reported, because a changed line no test covers is the finding even
//! though it is not a target. A line the document does not mention at all is neither:
//! it is a comment, a blank, or a continuation, and no mutation could land there.
//!
//! # Where the files are
//!
//! `SF:` may be absolute, which is what a coverage tool writes by default, or relative,
//! which is what `relative_files = true` produces. Both are read. An absolute path is
//! taken relative to the project root, in either of the absolute spellings this
//! platform has for that root. A path that leads somewhere else — a monorepo sibling, a
//! dependency in a virtual environment — is left out and counted, so that a selection
//! which found nothing can be told apart from a document about another tree.
//!
//! A duplicated `SF:` block is a partial account of the same file rather than a
//! contradiction: a line reached under either block is a line a test reached.
//!
//! A block that names a file and records not one line of it accounts for nothing, and is
//! read as a file the document has never heard of. Nothing in such a file could be a
//! candidate or a gap, so the alternative is a changed file whose lines fall out of every
//! number a selection reports.

use std::{collections::BTreeMap, path::Path};

use crate::paths::absolute_spellings;

/// The record that names the file a block is about.
const FILE: &str = "SF:";

/// The record that says a line is a statement, and how many times it ran.
const LINE: &str = "DA:";

/// The record that closes a block.
const END: &str = "end_of_record";

/// What one coverage document says about the project's own files.
#[derive(Debug, Default)]
pub struct Coverage {
    /// Per file, every measured line and whether anything reached it.
    files: BTreeMap<String, BTreeMap<u32, bool>>,
    /// How many blocks were about a file outside the project.
    outside: usize,
}

/// Why a coverage document could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CoverageError {
    /// A line record that is not one.
    #[error(
        "line {at} of the coverage document is `{record}`, which is not a line record this can read; a line record is `DA:<line>,<hits>`"
    )]
    UnreadableRecord {
        /// Which line of the document it was.
        at: usize,
        /// The record, as it stands.
        record: String,
    },
    /// A line record standing outside any file's block.
    #[error(
        "line {at} of the coverage document reports a covered line of no file: `{record}` stands outside any `SF:` block, so there is nothing to attribute it to"
    )]
    NoFileYet {
        /// Which line of the document it was.
        at: usize,
        /// The record, as it stands.
        record: String,
    },
}

impl Coverage {
    /// Read one coverage document, taking its files as relative to `project`.
    ///
    /// # Errors
    ///
    /// Returns [`CoverageError`] for a line record that cannot be read or that stands
    /// outside any file's block. Both are the document saying something this cannot
    /// interpret, and a reader that guessed would attribute one file's coverage to
    /// another or call an unmeasured line covered.
    pub fn read(document: &str, project: &Path) -> Result<Self, CoverageError> {
        let roots = absolute_spellings(project);
        let mut read = Self::default();
        // `None` between blocks; `Some(None)` inside a block about somebody else's file.
        let mut block: Option<Option<String>> = None;
        for (index, raw) in document.lines().enumerate() {
            let at = index + 1;
            let record = raw.trim_end_matches('\r');
            if let Some(path) = record.strip_prefix(FILE) {
                let inside = relative(path.trim(), &roots);
                if inside.is_none() {
                    read.outside += 1;
                }
                block = Some(inside);
                continue;
            }
            if record.trim() == END {
                block = None;
                continue;
            }
            let Some(measured) = record.strip_prefix(LINE) else {
                continue;
            };
            let (line, hits) =
                statement(measured).ok_or_else(|| CoverageError::UnreadableRecord {
                    at,
                    record: record.to_owned(),
                })?;
            let Some(about) = block.as_ref() else {
                return Err(CoverageError::NoFileYet {
                    at,
                    record: record.to_owned(),
                });
            };
            let Some(file) = about else {
                continue;
            };
            let reached = read
                .files
                .entry(file.clone())
                .or_default()
                .entry(line)
                .or_insert(false);
            *reached |= hits > 0;
        }
        Ok(read)
    }

    /// Whether the document measured any line of this file.
    ///
    /// A block that names a file and records not one line of it is not an account of that
    /// file: nothing in it can be a candidate, and nothing in it can be a gap, so a change
    /// to the file would have every one of its lines fall out of every number a selection
    /// reports. Saying the document knows nothing about it is what puts those lines back on
    /// the record — as a file the coverage document does not account for, which it is.
    #[must_use]
    pub fn knows(&self, file: &str) -> bool {
        self.files.contains_key(file)
    }

    /// Whether a test reached this line.
    #[must_use]
    pub fn covered(&self, file: &str, line: u32) -> bool {
        self.files
            .get(file)
            .and_then(|lines| lines.get(&line))
            .copied()
            .unwrap_or(false)
    }

    /// Whether the document measured this line at all, whatever it reached.
    #[must_use]
    pub fn measured(&self, file: &str, line: u32) -> bool {
        self.files
            .get(file)
            .is_some_and(|lines| lines.contains_key(&line))
    }

    /// How many of the document's blocks were about a file outside the project.
    #[must_use]
    pub fn outside_the_project(&self) -> usize {
        self.outside
    }
}

/// The line and the hits a `DA:` record carries.
///
/// A checksum may follow the hits, and is ignored: it is about the source the coverage
/// was measured against, and the manifest holds this project to that comparison itself
/// through each mutant's own hash of the file.
fn statement(record: &str) -> Option<(u32, u64)> {
    let (line, rest) = record.split_once(',')?;
    let hits = rest.split(',').next()?;
    Some((line.trim().parse().ok()?, hits.trim().parse().ok()?))
}

/// The file a `SF:` record names, as a path inside the project, or nothing when it is
/// not inside it.
fn relative(path: &str, roots: &[String]) -> Option<String> {
    if !path.starts_with('/') {
        return normalized(path);
    }
    for root in roots {
        let Some(rest) = path.strip_prefix(root.as_str()) else {
            continue;
        };
        if let Some(inside) = rest.strip_prefix('/') {
            return normalized(inside);
        }
    }
    None
}

/// One relative path in the single spelling everything else uses: POSIX separators, no
/// `.` components, and nothing that climbs out of the project.
fn normalized(path: &str) -> Option<String> {
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => return None,
            named => components.push(named),
        }
    }
    if components.is_empty() {
        return None;
    }
    Some(components.join("/"))
}
