//! What a triage is allowed to read, which is one run directory and nothing else.
//!
//! The working tree is not an input. A run's `manifest.json` says what each mutant
//! replaces, its `snapshot/` holds the bytes the run was measured against, and
//! `report.json` says which mutants the suite failed to catch — and those three,
//! joined on the run's own identifier, are the whole of it. Reading the project
//! instead would make a triage mean something different depending on what had been
//! edited since the run, and a survivor's evidence would go stale in a way nobody
//! could see.
//!
//! Three things are therefore checked before anything is judged: that the report in
//! the directory is the directory's own run, that the manifest carries every mutant
//! the report gives a verdict on, and that the snapshot's bytes are the bytes each
//! mutant was generated against. Each of those is a way for two documents of one
//! run to disagree, and every one of them would otherwise be discovered as a
//! confusing classification rather than as the mismatch it is.

use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tremula_contracts::{
    manifest::{Manifest, Mutant},
    report::{Report, Verdict},
    spans::{FunctionSpan, SpansReport},
};

use crate::{pack, python_env::PythonEnv, triage::failures::TriageFailure};

/// Where a run keeps the bytes it was measured against.
const SNAPSHOT: &str = "snapshot";

/// The report a triage is a second opinion on.
pub const REPORT: &str = "report.json";

/// The copy of the manifest a run keeps beside its results.
const MANIFEST: &str = "manifest.json";

/// One survivor, with everything needed to ask about it.
#[derive(Debug, Clone)]
pub struct Survivor {
    /// The mutation, as the run's own manifest holds it.
    pub mutant: Mutant,
    /// The source of the function the mutation lands in, cut from the snapshot.
    pub function: String,
    /// That function's name, for a person reading the console.
    pub function_name: String,
}

/// Everything one run directory offers a triage.
#[derive(Debug)]
pub struct Inputs {
    /// The run directory, resolved.
    pub run_dir: PathBuf,
    /// The run's identifier, which is that directory's name.
    pub run_id: String,
    /// The root a witness is run under: the run's snapshot.
    pub snapshot: PathBuf,
    /// The survivors, in the report's order.
    pub survivors: Vec<Survivor>,
    /// How many mutants the run had in all, for the console's sense of proportion.
    pub mutants: usize,
}

/// Read one run directory and join its three documents.
///
/// # Errors
///
/// Returns [`TriageFailure`] when the directory is not a run's, when its documents
/// are missing, unreadable, or about another run, or when the pack cannot say where
/// the functions of a snapshotted file are.
pub fn read(env: &PythonEnv, run_dir: &Path) -> Result<Inputs, TriageFailure> {
    if !run_dir.is_dir() {
        return Err(TriageFailure::NoRunDirectory {
            path: run_dir.to_path_buf(),
        });
    }
    // Resolved, because the run this is pointed at is routinely the `latest` link
    // and the run's identifier is the name of the directory that link leads to.
    let resolved = fs::canonicalize(run_dir).map_err(|err| TriageFailure::PathUnresolvable {
        path: run_dir.to_path_buf(),
        reason: err.to_string(),
    })?;
    let run_id = resolved
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_owned();
    let report: Report = document(&resolved.join(REPORT), "report")?;
    if report.run.run_id != run_id {
        return Err(TriageFailure::AnotherRun {
            directory: resolved.display().to_string(),
            found: report.run.run_id,
            expected: run_id,
        });
    }
    let manifest: Manifest = document(&resolved.join(MANIFEST), "manifest")?;
    let snapshot = resolved.join(SNAPSHOT);
    if !snapshot.is_dir() {
        return Err(TriageFailure::NoSnapshot { run_dir: resolved });
    }
    let survivors = surviving(env, &resolved, &snapshot, &report, &manifest)?;
    Ok(Inputs {
        run_dir: resolved,
        run_id,
        snapshot,
        survivors,
        mutants: manifest.mutants.len(),
    })
}

/// Read one of the run's documents as itself.
fn document<T: serde::de::DeserializeOwned>(
    path: &Path,
    what: &'static str,
) -> Result<T, TriageFailure> {
    let text = fs::read_to_string(path).map_err(|err| TriageFailure::Unreadable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;
    serde_json::from_str(&text).map_err(|err| TriageFailure::Invalid {
        path: path.to_path_buf(),
        what,
        reason: err.to_string(),
    })
}

/// Every mutant the suite failed to catch, with the function it lands in.
fn surviving(
    env: &PythonEnv,
    run_dir: &Path,
    snapshot: &Path,
    report: &Report,
    manifest: &Manifest,
) -> Result<Vec<Survivor>, TriageFailure> {
    let mut reported: Vec<&Mutant> = Vec::new();
    for verdict in &report.verdicts {
        if verdict.verdict != Verdict::Survived {
            continue;
        }
        let found = manifest
            .mutants
            .iter()
            .find(|mutant| mutant.id == verdict.mutant_id)
            .ok_or_else(|| TriageFailure::MutantNotInManifest {
                run_dir: run_dir.to_path_buf(),
                mutant_id: verdict.mutant_id.clone(),
            })?;
        reported.push(found);
    }
    let mut described: Vec<(String, SpansReport, Vec<u8>)> = Vec::new();
    let mut survivors = Vec::with_capacity(reported.len());
    for mutant in reported {
        if !described.iter().any(|(file, _, _)| file == &mutant.file) {
            described.push(describe(env, snapshot, mutant)?);
        }
        let (_, spans, bytes) = described
            .iter()
            .find(|(file, _, _)| file == &mutant.file)
            .ok_or_else(|| TriageFailure::MutantNotInManifest {
                run_dir: run_dir.to_path_buf(),
                mutant_id: mutant.id.clone(),
            })?;
        let (function_name, function) = enclosing(spans, bytes, mutant);
        survivors.push(Survivor {
            mutant: mutant.clone(),
            function,
            function_name,
        });
    }
    Ok(survivors)
}

/// Ask the pack about one snapshotted file, and check it is the run's own bytes.
fn describe(
    env: &PythonEnv,
    snapshot: &Path,
    mutant: &Mutant,
) -> Result<(String, SpansReport, Vec<u8>), TriageFailure> {
    let path = snapshot.join(&mutant.file);
    let bytes = fs::read(&path).map_err(|err| TriageFailure::Unreadable {
        path: path.clone(),
        reason: err.to_string(),
    })?;
    if format!("{:x}", Sha256::digest(&bytes)) != mutant.base_file_sha256 {
        return Err(TriageFailure::SnapshotIsAnotherFile {
            file: mutant.file.clone(),
            mutant_id: mutant.id.clone(),
        });
    }
    let spans = pack::spans(env, snapshot, &mutant.file)?;
    Ok((mutant.file.clone(), spans, bytes))
}

/// The innermost function the mutation lands in, and its source.
///
/// Innermost because a nested function's body is inside its parent's, and it is the
/// nested one whose guards decide what a witness can reach. An empty name and an
/// empty source is the honest answer for a mutation inside no function at all: what
/// follows will report that nothing could be asked about it, rather than send a
/// model the whole file and call the answer a judgement about a function.
fn enclosing(spans: &SpansReport, bytes: &[u8], mutant: &Mutant) -> (String, String) {
    let innermost = spans
        .functions
        .iter()
        .filter(|function| contains(function, mutant))
        .max_by_key(|function| function.span.start_byte);
    let Some(function) = innermost else {
        return (String::new(), String::new());
    };
    let source = text(bytes, function).unwrap_or_default();
    (function.qualified_name.clone(), source)
}

/// Whether the mutation's span falls inside the function's.
fn contains(function: &FunctionSpan, mutant: &Mutant) -> bool {
    function.span.start_byte <= mutant.span.start_byte
        && mutant.span.end_byte <= function.span.end_byte
}

/// One function's source, cut from the snapshot's bytes.
fn text(bytes: &[u8], function: &FunctionSpan) -> Option<String> {
    let start = usize::try_from(function.span.start_byte).ok()?;
    let end = usize::try_from(function.span.end_byte).ok()?;
    let slice = bytes.get(start..end)?;
    std::str::from_utf8(slice).ok().map(str::to_owned)
}
