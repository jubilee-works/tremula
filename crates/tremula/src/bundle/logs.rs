//! Taking a machine out of a log, and a log down to a size worth sending.
//!
//! A suite's output is the most useful thing in a bundle and the most revealing thing
//! in a run directory. It names the interpreter it ran under, the directory it ran in,
//! and the person whose home both are inside; and it carries tremula's own protocol in
//! the lines the runner wrote to report the suite back. None of that is evidence about
//! anybody's tests, and all of it travels unless it is taken out.
//!
//! # Why the order is fixed
//!
//! Clean first, shorten second. A log shortened first has a cut somewhere in the middle
//! of it, and whatever absolute path happened to straddle that cut is now two halves —
//! neither of which matches anything a replacement is looking for, so whichever half
//! fell inside the kept region is published. The count in the notice is therefore a
//! count of cleaned bytes, which is also the only count a reader could check.
//!
//! # Why this works on bytes
//!
//! Because a log is whatever a suite printed, and a suite can print anything. A stray
//! byte in a traceback makes the output not text, and an implementation that read a log
//! as a string would either refuse it or quietly rewrite it. Only the two cut points
//! care about characters at all, and they step off a partial one rather than through it.
//!
//! # What is deliberately kept
//!
//! Paths into a system interpreter — `/Library/...`, `/opt/homebrew/...` — name nobody.
//! A project's own virtual environment is normally under a home directory and goes with
//! it; one somewhere else stays, and that is the honest limit of a bundle that never
//! reads the pack's configuration.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tremula_contracts::bundle::{Attached, Attachment};

use crate::bundle::{collect::Evidence, failures::BundleFailure, write};

/// Where the logs are, both in a run directory and in a bundle.
pub const LOGS: &str = "logs";

/// The log of the run with nothing mutated.
pub const BASELINE_LOG: &str = "baseline.txt";

/// How much of each end of a log survives being shortened. A log of exactly twice this
/// is not shortened at all.
const KEPT_EACH_END: usize = 32 * 1024;

/// The line the test runner writes to report a suite back to the pack. Internal
/// protocol, and of no use to anybody reading a bundle.
const MARKER_PREFIX: &[u8] = b"TREMULA-RESULT: ";

/// The temporary report the runner asks the test framework for. Its directory is named
/// after nothing and exists for the length of one suite.
const RUNNER_REPORT: &[u8] = b"tremula-junit-";

/// What each of a machine's own directories is replaced with.
const RUN: &[u8] = b"<run>";
const PROJECT: &[u8] = b"<project>";
const TEMPORARY: &[u8] = b"<tmp>";
const HOME: &[u8] = b"<home>";

/// Where the directories a platform reaches through a link of its own actually live.
const PRIVATE: &str = "/private";

/// The directories that are links into [`PRIVATE`] here. One directory, two absolute
/// spellings, and a process prints whichever of them it was handed.
const LINKED: [&str; 2] = ["/tmp", "/var"];

/// The directories of the machine a run happened on, ready to be taken out of a log.
///
/// Each one is held in every absolute spelling this platform has for it: the one it was
/// given, the one it resolves to, and the sibling of each of those across `/private`. A
/// log carries whichever form the process that printed it happened to have — a project
/// root comes off the command line, and a test framework prints the resolved one.
#[derive(Debug, Default)]
pub struct Machine {
    /// Longest first, so a run directory inside a project is recognised as the run
    /// directory rather than as the project with a suffix.
    replacements: Vec<(Vec<u8>, &'static [u8])>,
}

impl Machine {
    /// The machine a run of `project` in `run` happened on.
    ///
    /// `run` is the path as the caller gave it, which is routinely the `latest` link;
    /// its resolved form is the run directory itself, and both are taken out.
    #[must_use]
    pub fn around(project: &Path, run: &Path) -> Self {
        let mut machine = Self::default();
        for (directory, replacement) in [
            (run.to_path_buf(), RUN),
            (project.to_path_buf(), PROJECT),
            (std::env::temp_dir(), TEMPORARY),
            (home(), HOME),
        ] {
            machine.learn(&directory, replacement);
            if let Ok(resolved) = fs::canonicalize(&directory) {
                machine.learn(&resolved, replacement);
            }
        }
        // Longest first is what makes nesting come out right, and every directory here
        // nests inside at least one of the others.
        machine
            .replacements
            .sort_by_key(|(needle, _)| std::cmp::Reverse(needle.len()));
        machine
    }

    /// Remember one directory, in every absolute spelling this platform has for it.
    fn learn(&mut self, directory: &Path, replacement: &'static [u8]) {
        let spelled = directory.to_string_lossy();
        let trimmed = spelled.trim_end_matches('/');
        self.remember(trimmed, replacement);
        if let Some(sibling) = sibling_spelling(trimmed) {
            self.remember(&sibling, replacement);
        }
    }

    /// Remember one spelling, if it is one a log could name unambiguously.
    ///
    /// A relative path is refused, and `.` is why: it is what `--project` defaults to,
    /// and taking every full stop out of a suite's output would destroy the output.
    fn remember(&mut self, trimmed: &str, replacement: &'static [u8]) {
        if trimmed.len() < 2 || !trimmed.starts_with('/') {
            return;
        }
        let needle = trimmed.as_bytes().to_vec();
        if self.replacements.iter().any(|(known, _)| known == &needle) {
            return;
        }
        self.replacements.push((needle, replacement));
    }
}

/// The other absolute spelling of one directory, when this platform has two.
///
/// This is what makes the default invocation safe. `--project .` is relative, so the only
/// absolute form of the project there is to learn from is the resolved one — and a suite
/// handed the other spelling would otherwise have it published. Deriving the sibling from
/// whichever form is in hand covers both directions, and both map to one placeholder.
fn sibling_spelling(trimmed: &str) -> Option<String> {
    for linked in LINKED {
        if let Some(rest) = trimmed.strip_prefix(linked)
            && a_component_ends_there(rest)
        {
            return Some(format!("{PRIVATE}{trimmed}"));
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{PRIVATE}{linked}"))
            && a_component_ends_there(rest)
        {
            return Some(format!("{linked}{rest}"));
        }
    }
    None
}

/// Whether a prefix ended where a component of the path ends, rather than inside a name.
fn a_component_ends_there(rest: &str) -> bool {
    rest.is_empty() || rest.starts_with('/')
}

/// Somebody's home directory, when the environment says which.
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// One log, as it will travel.
#[derive(Debug)]
pub struct Cleaned {
    /// The bytes to write.
    pub bytes: Vec<u8>,
    /// Whether anything was left out of them.
    pub truncated: bool,
}

/// Take the machine and the protocol out of `raw`, then shorten what is left.
#[must_use]
pub fn clean(machine: &Machine, raw: &[u8]) -> Cleaned {
    shortened(&sanitized(machine, raw))
}

/// Everything that is about the machine or about tremula rather than about the suite.
fn sanitized(machine: &Machine, raw: &[u8]) -> Vec<u8> {
    let mut kept = Vec::with_capacity(raw.len());
    for line in lines(raw) {
        if internal(line) {
            continue;
        }
        kept.extend_from_slice(line);
    }
    for (needle, replacement) in &machine.replacements {
        kept = replaced(&kept, needle, replacement);
    }
    kept
}

/// The lines of a log, each still carrying its own terminator.
fn lines(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    raw.split_inclusive(|byte| *byte == b'\n')
}

/// Whether a line is tremula talking to itself rather than the suite talking.
fn internal(line: &[u8]) -> bool {
    line.starts_with(MARKER_PREFIX) || contains(line, RUNNER_REPORT)
}

/// Whether `needle` occurs in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    at(haystack, needle).is_some()
}

/// Where `needle` first occurs in `haystack`.
fn at(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Every occurrence of `needle` in `haystack` that is really that directory, replaced.
///
/// A directory is only the one named where the path ends or goes on with a separator. A
/// sibling whose name begins with the project's — `sample_projectile` beside
/// `sample_project` — is a different directory, and rewriting it into `<project>ile` would
/// put a claim in the log that describes nothing that exists.
fn replaced(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut written = Vec::with_capacity(haystack.len());
    let mut rest = haystack;
    while let Some(found) = at(rest, needle) {
        let after = found + needle.len();
        written.extend_from_slice(&rest[..found]);
        if extends_a_name(rest.get(after).copied()) {
            written.extend_from_slice(&rest[found..after]);
        } else {
            written.extend_from_slice(replacement);
        }
        rest = &rest[after..];
    }
    written.extend_from_slice(rest);
    written
}

/// Whether a byte carries on the name a needle ended in.
///
/// A separator ends a component and so does everything a log puts after a path — a colon,
/// a space, a quote, a newline. What continues one is a letter, a digit, the underscore
/// that a language's own names are full of, or a byte of some encoding this does not read.
fn extends_a_name(byte: Option<u8>) -> bool {
    byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80)
}

/// Keep both ends of a log and say how much of the middle went.
///
/// Both ends, because the two things a reader wants are at opposite ends: what the suite
/// was doing when it started, and what it concluded. The count is of the bytes actually
/// left out of the file, so a reader can check it against the length of the file.
fn shortened(sanitized: &[u8]) -> Cleaned {
    if sanitized.len() <= KEPT_EACH_END * 2 {
        return Cleaned {
            bytes: sanitized.to_vec(),
            truncated: false,
        };
    }
    let head = boundary_at_or_before(sanitized, KEPT_EACH_END);
    let tail = boundary_at_or_after(sanitized, sanitized.len() - KEPT_EACH_END);
    let mut bytes = sanitized[..head].to_vec();
    bytes.extend_from_slice(format!("\n… {} bytes omitted …\n", tail - head).as_bytes());
    bytes.extend_from_slice(&sanitized[tail..]);
    Cleaned {
        bytes,
        truncated: true,
    }
}

/// Step back off a byte that continues a character, so none is cut in half.
///
/// Three steps at most, which is as far as a continuation run in UTF-8 goes. A log that
/// is not text moves the cut by up to three bytes and the count stays right, which is
/// the whole of what is promised about output that is not text.
fn boundary_at_or_before(bytes: &[u8], at: usize) -> usize {
    let mut at = at.min(bytes.len());
    for _ in 0..3 {
        if at == 0 || at >= bytes.len() || !continues(bytes[at]) {
            break;
        }
        at -= 1;
    }
    at
}

/// The same, forwards, for the byte the tail starts at.
fn boundary_at_or_after(bytes: &[u8], at: usize) -> usize {
    let mut at = at.min(bytes.len());
    for _ in 0..3 {
        if at >= bytes.len() || !continues(bytes[at]) {
            break;
        }
        at += 1;
    }
    at
}

/// Whether a byte is one that continues a character rather than starting one.
fn continues(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// What carrying the logs came to.
#[derive(Debug, Default)]
pub struct Carried {
    /// Whether any of them was shortened.
    pub truncated: bool,
}

/// Write every log the run kept into `staging`, cleaned, and name it in `attachments`.
///
/// The unmutated run's own log travels too. It is not one mutant's, so nothing in the
/// index names it — a reader is sent to it by the bundle's own starting document, and
/// the hashes in the index are of the documents and the patches, which is what the index
/// says they are.
///
/// # Errors
///
/// Returns [`BundleFailure`] when a log cannot be read or cannot be written into the
/// bundle. A log the run never kept is not an error: a mutant that was never executed has
/// no output. One that is there and cannot be read is, because a bundle one log short with
/// nothing saying so reads exactly like a run that never produced it.
pub fn attach(
    evidence: &Evidence,
    staging: &Path,
    machine: &Machine,
    attachments: &mut [Attachment],
) -> Result<Carried, BundleFailure> {
    let mut carried = Carried::default();
    let from = evidence.run_dir.join(LOGS);
    for attachment in attachments.iter_mut() {
        // The identifier was held to being 64 hexadecimal digits before anything built a
        // path out of it, which is what makes this join safe.
        let name = format!("{LOGS}/{}.txt", attachment.id);
        let Some(raw) = kept(&from.join(format!("{}.txt", attachment.id)))? else {
            continue;
        };
        let cleaned = clean(machine, &raw);
        attachment.log = Some(carry(staging, &name, &cleaned)?);
        carried.truncated |= cleaned.truncated;
    }
    if let Some(raw) = kept(&from.join(BASELINE_LOG))? {
        let cleaned = clean(machine, &raw);
        carry(staging, &format!("{LOGS}/{BASELINE_LOG}"), &cleaned)?;
        carried.truncated |= cleaned.truncated;
    }
    Ok(carried)
}

/// One log the run kept, or nothing when it kept none.
fn kept(path: &Path) -> Result<Option<Vec<u8>>, BundleFailure> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(BundleFailure::Unreadable {
            path: path.to_path_buf(),
            reason: err.to_string(),
        }),
    }
}

/// Write one cleaned log into the bundle and say what it hashes to.
fn carry(staging: &Path, name: &str, cleaned: &Cleaned) -> Result<Attached, BundleFailure> {
    write(staging, name, &cleaned.bytes)?;
    Ok(Attached {
        path: name.to_owned(),
        sha256: format!("{:x}", Sha256::digest(&cleaned.bytes)),
    })
}
