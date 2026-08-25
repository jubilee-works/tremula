//! What a bundle reads, which is one run directory and nothing else.
//!
//! The project is not an input. A bundle is a record of a measurement that already
//! happened, and reading the working tree would let it describe sources that were
//! never measured — the one failure a reader of the bundle could not possibly
//! detect. So everything here comes out of the run's own documents.
//!
//! Seven things are checked before anything is written, and each is a way for one
//! run's documents to be two runs': that the report in the directory is the
//! directory's own run, that the report was written against the contract this
//! tremula reads, that the results, the baseline, and the triage name that same run,
//! that all of them declare the same contract version, that the triage is about this
//! report and not another, that the manifest and the report cover exactly the same
//! mutants, and that the manifest is the one the run's own name was derived from.
//!
//! A directory with no report in it is none of those. A run points `latest` at itself
//! before it writes its report, so that is a run still going on — or one that failed —
//! and it is said as such rather than as a document that cannot be read.
//!
//! Patches come from `results.entries[].diff`, which is a published contract, and
//! not from the working file a pack keeps beside it. Each one is checked by running
//! `git apply --check` with the run's `snapshot/` as the working directory: the
//! snapshot holds the target files under their project-relative paths, so it is
//! already the tree the patch was made against, and no temporary worktree is
//! needed. One patch per invocation, because git applies a list of them in
//! sequence and every patch after the first would then be checked against a tree
//! the others had already changed.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use sha2::{Digest, Sha256};
use tremula_contracts::{
    SCHEMA_VERSION,
    baseline::Baseline,
    bundle::{Attached, Attachment},
    manifest::Manifest,
    report::Report,
    results::Results,
    triage::Triage,
};

use crate::{
    bundle::failures::BundleFailure,
    run_dir::{fingerprint, fingerprint_of},
};

/// The document a bundle's reader starts from.
pub const REPORT: &str = "report.json";

/// What each mutant replaces, and where.
pub const MANIFEST: &str = "manifest.json";

/// The neutral execution signals, and the diffs.
pub const RESULTS: &str = "results.json";

/// What the suite did with nothing mutated.
pub const BASELINE: &str = "baseline.json";

/// A second opinion on the survivors, when one was asked for.
pub const TRIAGE: &str = "triage.json";

/// Where a run keeps the bytes it was measured against.
const SNAPSHOT: &str = "snapshot";

/// Where the patches go inside a bundle.
pub const PATCHES: &str = "patches";

/// How many characters a mutant identifier has.
const ID_CHARS: usize = 64;

/// One document, kept as bytes as well as as itself.
///
/// The bytes are what a bundle carries: a document re-serialized from its own type
/// is a document this version of tremula happens to agree with, and the hash a
/// reader checks has to be the hash of what the run wrote.
#[derive(Debug)]
pub struct Document<T> {
    /// The file's bytes, exactly as the run left them.
    pub bytes: Vec<u8>,
    /// The document, read as itself.
    pub value: T,
}

/// What one run directory offers a bundle.
#[derive(Debug)]
pub struct Evidence {
    /// The run directory, resolved.
    pub run_dir: PathBuf,
    /// The run's identifier, which is that directory's name.
    pub run_id: String,
    /// The bytes the run was measured against, which is where a patch is checked.
    pub snapshot: PathBuf,
    /// The verdicts.
    pub report: Document<Report>,
    /// The mutants those verdicts are about.
    pub manifest: Document<Manifest>,
    /// What the run observed.
    pub results: Document<Results>,
    /// What the suite did unmutated.
    pub baseline: Document<Baseline>,
    /// What a second look made of the survivors, when one was taken.
    pub triage: Option<Document<Triage>>,
}

/// What reading a run directory came to.
#[derive(Debug)]
pub enum Read {
    /// A run that tested nothing. It is a finished, successful run, and there is
    /// simply no evidence in it to package.
    Nothing {
        /// The run that had nothing in it.
        run_id: String,
    },
    /// A run with mutants, and every document a bundle needs.
    Evidence(Box<Evidence>),
}

/// Read one run directory and check its documents against each other.
///
/// # Errors
///
/// Returns [`BundleFailure`] when the directory is not a run's, when a document is
/// missing, unreadable, or about another run, when two of them disagree about the
/// contract version, when the triage is about another report, or when the manifest
/// and the report cover different mutants.
pub fn read(run_dir: &Path) -> Result<Read, BundleFailure> {
    if !run_dir.is_dir() {
        return Err(BundleFailure::NoRunDirectory {
            path: run_dir.to_path_buf(),
        });
    }
    // Resolved, because a bundle is routinely pointed at the `latest` link and the
    // run's identifier is the name of the directory that link leads to. Comparing
    // the link's own name would make every default invocation fail.
    let resolved = fs::canonicalize(run_dir).map_err(|err| BundleFailure::PathUnresolvable {
        path: run_dir.to_path_buf(),
        reason: err.to_string(),
    })?;
    let run_id = resolved
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_owned();
    let directory = resolved.display().to_string();
    // A run publishes `latest` before it writes its report, so a directory with no report
    // in it is routinely a run that is still going rather than one that broke.
    if !resolved.join(REPORT).exists() {
        return Err(BundleFailure::RunUnfinished { run_dir: resolved });
    }
    let report: Document<Report> = document(&resolved.join(REPORT), "report")?;
    // Before anything is compared against it: every other document is checked against the
    // report's own declared version, so a report from another contract would make the whole
    // agreement a statement about a version this tremula never wrote.
    if report.value.schema_version != SCHEMA_VERSION {
        return Err(BundleFailure::ReportContract {
            directory,
            found: report.value.schema_version.clone(),
        });
    }
    if report.value.run.run_id != run_id {
        return Err(BundleFailure::AnotherRun {
            document: "report",
            directory,
            found: report.value.run.run_id.clone(),
            expected: run_id,
        });
    }
    // A run of a manifest with nothing in it writes a report and no other document,
    // and it is a successful run. Saying so beats reporting the missing manifest as
    // a broken one.
    if !resolved.join(MANIFEST).exists() && report.value.verdicts.is_empty() {
        return Ok(Read::Nothing { run_id });
    }
    let manifest: Document<Manifest> = document(&resolved.join(MANIFEST), "manifest")?;
    let results: Document<Results> = document(&resolved.join(RESULTS), "results")?;
    let baseline: Document<Baseline> = document(&resolved.join(BASELINE), "baseline")?;
    let triage: Option<Document<Triage>> = if resolved.join(TRIAGE).exists() {
        Some(document(&resolved.join(TRIAGE), "triage")?)
    } else {
        None
    };
    let snapshot = resolved.join(SNAPSHOT);
    if !snapshot.is_dir() {
        return Err(BundleFailure::NoSnapshot { run_dir: resolved });
    }
    let evidence = Evidence {
        run_dir: resolved,
        run_id,
        snapshot,
        report,
        manifest,
        results,
        baseline,
        triage,
    };
    of_one_run(&evidence, &directory)?;
    covering_the_same_mutants(&evidence)?;
    // Last of the three, because it is the one whose complaint names no mutant: a manifest
    // that holds other mutants than the report fails the check above, which can say which
    // mutant it was, and this is left with the case where the two agree on every mutant and
    // the bytes are still another run's.
    from_the_manifest_the_run_ran(&evidence, &directory)?;
    Ok(Read::Evidence(Box::new(evidence)))
}

/// Read one of the run's documents as bytes and as itself.
fn document<T: serde::de::DeserializeOwned>(
    path: &Path,
    what: &'static str,
) -> Result<Document<T>, BundleFailure> {
    let bytes = fs::read(path).map_err(|err| BundleFailure::Unreadable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;
    let value = serde_json::from_slice(&bytes).map_err(|err| BundleFailure::Invalid {
        path: path.to_path_buf(),
        what,
        reason: err.to_string(),
    })?;
    Ok(Document { bytes, value })
}

/// Every document names this run, and one contract version.
fn of_one_run(evidence: &Evidence, directory: &str) -> Result<(), BundleFailure> {
    let expected = &evidence.run_id;
    let mut named: Vec<(&'static str, &str)> = vec![
        ("results", evidence.results.value.run_id.as_str()),
        ("baseline", evidence.baseline.value.run_id.as_str()),
    ];
    let declared = &evidence.report.value.schema_version;
    let mut versions: Vec<(&'static str, &str)> = vec![
        ("manifest", evidence.manifest.value.schema_version.as_str()),
        ("results", evidence.results.value.schema_version.as_str()),
        ("baseline", evidence.baseline.value.schema_version.as_str()),
    ];
    if let Some(triage) = &evidence.triage {
        named.push(("triage", triage.value.run.run_id.as_str()));
        versions.push(("triage", triage.value.schema_version.as_str()));
        if triage.value.run.report != REPORT {
            return Err(BundleFailure::TriageIsAboutAnotherReport {
                directory: directory.to_owned(),
                found: triage.value.run.report.clone(),
            });
        }
    }
    for (document, found) in named {
        if found != expected {
            return Err(BundleFailure::AnotherRun {
                document,
                directory: directory.to_owned(),
                found: found.to_owned(),
                expected: expected.clone(),
            });
        }
    }
    for (document, found) in versions {
        if found != declared {
            return Err(BundleFailure::ContractVersions {
                document,
                directory: directory.to_owned(),
                found: found.to_owned(),
                expected: declared.clone(),
            });
        }
    }
    Ok(())
}

/// The manifest in the directory is the one the run's own name was derived from.
///
/// The comparison is the run-id rule itself, which [`fingerprint`] keeps: the pack keeps the
/// manifest it was given byte for byte, so hashing what is there and reading the name are two
/// ways of asking the same question. A manifest swapped for another run's is the substitution
/// nothing else here would notice, and a directory whose name carries no fingerprint has
/// nothing to compare against — refusing that would be a rule about names rather than about
/// evidence, so it passes.
fn from_the_manifest_the_run_ran(
    evidence: &Evidence,
    directory: &str,
) -> Result<(), BundleFailure> {
    let Some(expected) = fingerprint(&evidence.run_id) else {
        return Ok(());
    };
    let found = fingerprint_of(&evidence.manifest.bytes);
    if found != expected {
        return Err(BundleFailure::ManifestOfAnotherRun {
            directory: directory.to_owned(),
            run_id: evidence.run_id.clone(),
            found,
            expected: expected.to_owned(),
        });
    }
    Ok(())
}

/// The manifest and the report account for exactly the same mutants, and every
/// identifier is one a file may be named after.
///
/// The identifiers are checked here rather than where a path is built, because a
/// path built out of an unchecked one has already left the bundle: `..` and a
/// separator in an identifier are all it takes.
fn covering_the_same_mutants(evidence: &Evidence) -> Result<(), BundleFailure> {
    for verdict in &evidence.report.value.verdicts {
        if !canonical(&verdict.mutant_id) {
            return Err(BundleFailure::MutantIdNotCanonical {
                mutant_id: verdict.mutant_id.clone(),
            });
        }
        if !evidence
            .manifest
            .value
            .mutants
            .iter()
            .any(|mutant| mutant.id == verdict.mutant_id)
        {
            return Err(BundleFailure::MutantNotInManifest {
                run_id: evidence.run_id.clone(),
                mutant_id: verdict.mutant_id.clone(),
            });
        }
    }
    for mutant in &evidence.manifest.value.mutants {
        if !canonical(&mutant.id) {
            return Err(BundleFailure::MutantIdNotCanonical {
                mutant_id: mutant.id.clone(),
            });
        }
        if !evidence
            .report
            .value
            .verdicts
            .iter()
            .any(|verdict| verdict.mutant_id == mutant.id)
        {
            return Err(BundleFailure::MutantNotInReport {
                run_id: evidence.run_id.clone(),
                mutant_id: mutant.id.clone(),
            });
        }
    }
    Ok(())
}

/// Whether an identifier is one a mutant's own derivation could have produced:
/// sixty-four lowercase hexadecimal digits, and nothing else at all.
#[must_use]
pub fn canonical(id: &str) -> bool {
    id.len() == ID_CHARS && hexadecimal(id)
}

/// Whether every character is a lowercase hexadecimal digit.
fn hexadecimal(said: &str) -> bool {
    said.bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The patches of one run, written and checked, with what came of each.
#[derive(Debug, Default)]
pub struct Attaching {
    /// One entry per mutant, in the report's order.
    pub attachments: Vec<Attachment>,
    /// Survivors whose patch could not be applied to the run's own snapshot. A
    /// survivor is the one mutant a reader of the bundle has work to do about, so a
    /// patch of one that does not apply is the bundle failing at its purpose.
    pub unusable: Vec<String>,
}

/// Write every mutant's patch into `staging`, keeping only the ones git accepts.
///
/// # Errors
///
/// Returns [`BundleFailure`] when a patch cannot be written.
pub fn patches(evidence: &Evidence, staging: &Path) -> Result<Attaching, BundleFailure> {
    let mut attaching = Attaching::default();
    for verdict in &evidence.report.value.verdicts {
        let id = &verdict.mutant_id;
        let diff = evidence
            .results
            .value
            .entries
            .iter()
            .find(|entry| entry.mutant_id == *id)
            .and_then(|entry| entry.diff.as_deref());
        let (patch, patch_error) = match diff {
            None => (
                None,
                Some("the run recorded no diff for this mutant".to_owned()),
            ),
            Some(diff) => written(staging, &evidence.snapshot, id, diff)?,
        };
        if patch.is_none() && verdict.verdict == tremula_contracts::report::Verdict::Survived {
            attaching.unusable.push(id.clone());
        }
        attaching.attachments.push(Attachment {
            id: id.clone(),
            patch,
            log: None,
            patch_error,
        });
    }
    Ok(attaching)
}

/// One patch, written and then checked, or removed and explained.
fn written(
    staging: &Path,
    snapshot: &Path,
    id: &str,
    diff: &str,
) -> Result<(Option<Attached>, Option<String>), BundleFailure> {
    let relative = format!("{PATCHES}/{id}.patch");
    let path = staging.join(&relative);
    let unwritable = |reason: String| BundleFailure::Unwritable {
        path: path.clone(),
        reason,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| unwritable(err.to_string()))?;
    }
    // A patch whose last line has no newline is one git refuses to read, and a pack
    // that wrote the diff without one meant the same change either way.
    let mut bytes = diff.as_bytes().to_vec();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    fs::write(&path, &bytes).map_err(|err| unwritable(err.to_string()))?;
    if let Some(said) = refusal(snapshot, &path) {
        drop(fs::remove_file(&path));
        return Ok((None, Some(said)));
    }
    Ok((
        Some(Attached {
            path: relative,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        }),
        None,
    ))
}

/// What git said about a patch, if it refused it.
fn refusal(snapshot: &Path, patch: &Path) -> Option<String> {
    let asked = Command::new("git")
        .current_dir(snapshot)
        .arg("apply")
        .arg("--check")
        .arg(patch)
        .output();
    match asked {
        Ok(answer) if answer.status.success() => None,
        Ok(answer) => Some(format!("git apply --check refused it: {}", said(&answer))),
        Err(err) => Some(format!("git apply --check could not be run: {err}")),
    }
}

/// git's own complaint, on one line so it fits a document and a console alike.
fn said(answer: &Output) -> String {
    let complaint = String::from_utf8_lossy(&answer.stderr);
    let lines: Vec<&str> = complaint
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return format!("it exited {}", answer.status.code().unwrap_or(-1));
    }
    lines.join("; ")
}
