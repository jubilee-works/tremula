//! A conservative derivative of the manifest a triage read.
//!
//! Projection removes only entries the model classified as suspected equivalent.
//! Everything else stays, including mutations a person already dismissed and
//! classifications this version does not know. Publication is separate: preflight
//! claims a process-unique staging file before model work, and commit hard-links the
//! completed document into an unoccupied destination without ever replacing it.

use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use tremula_contracts::{
    manifest::Manifest,
    triage::{Classification, Triage},
};

use crate::triage::failures::TriageFailure;

/// A source manifest projected through one completed triage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    /// The derivative manifest, retaining the source's order and typed fields.
    pub manifest: Manifest,
    /// Every source mutant identifier, in manifest order.
    pub source_ids: Vec<String>,
    /// Every retained mutant identifier, in manifest order.
    pub kept_ids: Vec<String>,
    /// Every removed mutant identifier, in manifest order.
    pub excluded_ids: Vec<String>,
}

impl Projection {
    /// Number of mutants in the source manifest.
    #[must_use]
    pub fn source_count(&self) -> usize {
        self.source_ids.len()
    }

    /// Number of mutants retained in the derivative manifest.
    #[must_use]
    pub fn kept_count(&self) -> usize {
        self.kept_ids.len()
    }

    /// Number of mutants removed from the derivative manifest.
    #[must_use]
    pub fn excluded_count(&self) -> usize {
        self.excluded_ids.len()
    }
}

/// What publication wrote and the exact projection it represents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationSummary {
    /// Exact destination path supplied at preflight.
    pub path: PathBuf,
    /// Number of mutants in the source manifest.
    pub source_count: usize,
    /// Number of mutants retained in the derivative manifest.
    pub kept_count: usize,
    /// Number of mutants removed from the derivative manifest.
    pub excluded_count: usize,
    /// Every source mutant identifier, in manifest order.
    pub source_ids: Vec<String>,
    /// Every retained mutant identifier, in manifest order.
    pub kept_ids: Vec<String>,
    /// Every removed mutant identifier, in manifest order.
    pub excluded_ids: Vec<String>,
}

/// A destination checked before model work, with its staging file already claimed.
///
/// Dropping this without publishing removes the staging file, so any later failure
/// in orchestration leaves no half-written derivative beside the requested output.
#[derive(Debug)]
pub struct Destination {
    path: PathBuf,
    staging: PathBuf,
    file: Option<File>,
}

impl Destination {
    #[cfg(test)]
    fn staging_path(&self) -> &Path {
        &self.staging
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        drop(self.file.take());
        drop(fs::remove_file(&self.staging));
    }
}

/// Project a source manifest through a completed triage.
///
/// Only a triage entry whose classification is exactly
/// [`Classification::SuspectedEquivalent`] removes its matching mutant. IDs named
/// only by `dismissed`, unknown classifications, and mutants absent from the triage
/// are retained.
#[must_use]
pub fn project(source: &Manifest, triage: &Triage) -> Projection {
    let suspected: HashSet<&str> = triage
        .entries
        .iter()
        .filter(|entry| entry.classification == Classification::SuspectedEquivalent)
        .map(|entry| entry.mutant_id.as_str())
        .collect();
    let source_ids = source
        .mutants
        .iter()
        .map(|mutant| mutant.id.clone())
        .collect();
    let mut kept_ids = Vec::new();
    let mut excluded_ids = Vec::new();
    let mut mutants = Vec::new();
    for mutant in &source.mutants {
        if suspected.contains(mutant.id.as_str()) {
            excluded_ids.push(mutant.id.clone());
        } else {
            kept_ids.push(mutant.id.clone());
            mutants.push(mutant.clone());
        }
    }
    let mut manifest = source.clone();
    manifest.mutants = mutants;
    Projection {
        manifest,
        source_ids,
        kept_ids,
        excluded_ids,
    }
}

/// Refuse an occupied, malformed, or unusable destination before model work.
///
/// The parent must already exist and be a directory. This also creates the empty,
/// process-unique sibling staging file now, proving the parent accepts a new file;
/// [`Destination`]'s drop removes it if later work does not reach publication.
///
/// # Errors
///
/// Returns [`TriageFailure::OutputExists`] when any filesystem object already has
/// the destination name, or [`TriageFailure::OutputUnusable`] when the destination
/// or its existing parent cannot be used.
pub fn preflight(path: &Path) -> Result<Destination, TriageFailure> {
    if path.file_name().is_none() {
        return Err(unusable(path, "the path has no file name"));
    }
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(TriageFailure::OutputExists {
                path: path.to_path_buf(),
            });
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(unusable(path, err.to_string())),
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = fs::metadata(parent).map_err(|err| unusable(path, err.to_string()))?;
    if !metadata.is_dir() {
        return Err(unusable(path, "the parent is not a directory"));
    }
    loop {
        let staging = named_beside(path);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
        {
            Ok(file) => {
                return Ok(Destination {
                    path: path.to_path_buf(),
                    staging,
                    file: Some(file),
                });
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(unusable(path, err.to_string())),
        }
    }
}

/// Write a pretty, newline-terminated derivative and commit it without replacement.
///
/// # Errors
///
/// Returns [`TriageFailure::OutputExists`] if the destination was claimed after
/// preflight, or [`TriageFailure::OutputUnusable`] if staging or commit fails.
pub fn publish(
    destination: Destination,
    projection: &Projection,
) -> Result<PublicationSummary, TriageFailure> {
    publish_with(destination, projection, || {})
}

fn publish_with(
    mut destination: Destination,
    projection: &Projection,
    before_commit: impl FnOnce(),
) -> Result<PublicationSummary, TriageFailure> {
    let path = destination.path.clone();
    let Some(file) = destination.file.as_mut() else {
        return Err(unusable(&path, "the staging file is unavailable"));
    };
    serde_json::to_writer_pretty(&mut *file, &projection.manifest)
        .map_err(|err| unusable(&path, err.to_string()))?;
    file.write_all(b"\n")
        .and_then(|()| file.flush())
        .map_err(|err| unusable(&path, err.to_string()))?;
    before_commit();
    match fs::hard_link(&destination.staging, &path) {
        Ok(()) => Ok(summary(path, projection)),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Err(TriageFailure::OutputExists { path })
        }
        Err(err) => Err(unusable(&path, err.to_string())),
    }
}

fn summary(path: PathBuf, projection: &Projection) -> PublicationSummary {
    PublicationSummary {
        path,
        source_count: projection.source_count(),
        kept_count: projection.kept_count(),
        excluded_count: projection.excluded_count(),
        source_ids: projection.source_ids.clone(),
        kept_ids: projection.kept_ids.clone(),
        excluded_ids: projection.excluded_ids.clone(),
    }
}

fn named_beside(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().map_or_else(OsString::new, OsString::from);
    name.push(format!(".part.{}.{ordinal}", process::id()));
    path.with_file_name(name)
}

fn unusable(path: &Path, reason: impl Into<String>) -> TriageFailure {
    TriageFailure::OutputUnusable {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use std::{fs, path::Path};

    use serde_json::{Map, json};
    use tempfile::TempDir;
    use tremula_contracts::{
        manifest::{Base, Language, Manifest, Mutant, Span},
        suppressions::DismissalReason,
        triage::{
            Classification, Dismissed, Judge, Spend, Triage, TriageEntry, TriageRun, TriageScore,
        },
    };

    use super::{preflight, project, publish, publish_with};
    use crate::triage::failures::TriageFailure;

    fn mutant(id: &str, ordinal: u64) -> Mutant {
        let mut provenance = Map::new();
        provenance.insert("ordinal".to_owned(), json!(ordinal));
        Mutant {
            id: id.to_owned(),
            file: format!("src/{id}.py"),
            base_file_sha256: format!("hash-{ordinal}"),
            span: Span {
                start_byte: ordinal * 10,
                end_byte: ordinal * 10 + 4,
            },
            original: format!("old-{ordinal}"),
            replacement: format!("new-{ordinal}"),
            description: Some(format!("description-{ordinal}")),
            provenance,
        }
    }

    fn manifest(ids: &[&str]) -> Manifest {
        Manifest {
            schema_version: "0.1".to_owned(),
            language: Language::Python,
            base: Base {
                revision: Some("abc123".to_owned()),
            },
            mutants: ids
                .iter()
                .enumerate()
                .map(|(ordinal, id)| mutant(id, u64::try_from(ordinal + 1).unwrap()))
                .collect(),
            selection: None,
        }
    }

    fn entry(id: &str, classification: Classification) -> TriageEntry {
        TriageEntry {
            mutant_id: id.to_owned(),
            file: format!("src/{id}.py"),
            span: Span {
                start_byte: 0,
                end_byte: 1,
            },
            original: "old".to_owned(),
            replacement: "new".to_owned(),
            classification,
            undecided: None,
            claim: None,
            witness: None,
            probe: None,
            detail: "fixture".to_owned(),
        }
    }

    fn triage(entries: Vec<TriageEntry>) -> Triage {
        Triage {
            schema_version: "0.1".to_owned(),
            run: TriageRun {
                run_id: "20260813T000000Z-abc123".to_owned(),
                report: "report.json".to_owned(),
                judged_at: "2026-08-13T00:00:00Z".to_owned(),
            },
            judge: Judge {
                model: "fixture".to_owned(),
                model_resolved: None,
                prompt_version: "1".to_owned(),
            },
            score: TriageScore {
                survivors: u32::try_from(entries.len()).unwrap(),
                distinguished_at_function_level: 0,
                suspected_equivalent: 0,
                undecided: 0,
            },
            spend: Spend::default(),
            entries,
            dismissed: Vec::new(),
            caveats: Vec::new(),
        }
    }

    fn staging_beside(path: &Path) -> Vec<String> {
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap();
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".part."))
            .collect()
    }

    #[test]
    fn projection_excludes_exactly_suspected_equivalents_and_preserves_every_typed_field() {
        let source = manifest(&[
            "suspected",
            "distinguished",
            "undecided",
            "unknown",
            "unseen",
        ]);
        let judged = triage(vec![
            entry("unknown", Classification::Unknown),
            entry("suspected", Classification::SuspectedEquivalent),
            entry("undecided", Classification::Undecided),
            entry(
                "distinguished",
                Classification::DistinguishedAtFunctionLevel,
            ),
        ]);

        let filtered = project(&source, &judged);

        assert_eq!(
            filtered.source_ids,
            [
                "suspected",
                "distinguished",
                "undecided",
                "unknown",
                "unseen"
            ]
        );
        assert_eq!(
            filtered.kept_ids,
            ["distinguished", "undecided", "unknown", "unseen"]
        );
        assert_eq!(filtered.excluded_ids, ["suspected"]);
        assert_eq!(filtered.source_count(), 5);
        assert_eq!(filtered.kept_count(), 4);
        assert_eq!(filtered.excluded_count(), 1);
        assert_eq!(filtered.manifest.schema_version, source.schema_version);
        assert_eq!(filtered.manifest.language, source.language);
        assert_eq!(filtered.manifest.base, source.base);
        assert_eq!(filtered.manifest.mutants, source.mutants[1..]);
    }

    #[test]
    fn a_persons_dismissed_mutant_is_retained_even_if_no_entry_was_made_for_it() {
        let source = manifest(&["dismissed", "suspected"]);
        let mut judged = triage(vec![entry(
            "suspected",
            Classification::SuspectedEquivalent,
        )]);
        judged.dismissed.push(Dismissed {
            mutant_id: "dismissed".to_owned(),
            file: "src/dismissed.py".to_owned(),
            original: "old".to_owned(),
            replacement: "new".to_owned(),
            reason: DismissalReason::Equivalent,
            dismissed_at: "2026-08-13T00:00:00Z".to_owned(),
        });

        let filtered = project(&source, &judged);

        assert_eq!(filtered.kept_ids, ["dismissed"]);
        assert_eq!(filtered.excluded_ids, ["suspected"]);
    }

    #[test]
    fn excluding_every_source_mutant_produces_a_valid_empty_manifest() {
        let source = manifest(&["one", "two"]);
        let judged = triage(vec![
            entry("two", Classification::SuspectedEquivalent),
            entry("one", Classification::SuspectedEquivalent),
        ]);

        let filtered = project(&source, &judged);

        assert!(filtered.manifest.mutants.is_empty());
        assert!(filtered.kept_ids.is_empty());
        assert_eq!(filtered.excluded_ids, ["one", "two"]);
    }

    #[test]
    fn publication_is_pretty_typed_newline_terminated_and_reports_exact_ids() {
        let workspace = TempDir::new().unwrap();
        let out = workspace.path().join("derivative.json");
        let source = manifest(&["keep", "exclude"]);
        let filtered = project(
            &source,
            &triage(vec![entry("exclude", Classification::SuspectedEquivalent)]),
        );

        let destination = preflight(&out).unwrap();
        let summary = publish(destination, &filtered).unwrap();

        let bytes = fs::read(&out).unwrap();
        assert!(bytes.ends_with(b"\n"));
        assert!(String::from_utf8_lossy(&bytes).contains("\n  \"schema_version\""));
        let written: Manifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(written, filtered.manifest);
        assert_eq!(summary.path, out);
        assert_eq!(
            (
                summary.source_count,
                summary.kept_count,
                summary.excluded_count
            ),
            (2, 1, 1)
        );
        assert_eq!(summary.source_ids, ["keep", "exclude"]);
        assert_eq!(summary.kept_ids, ["keep"]);
        assert_eq!(summary.excluded_ids, ["exclude"]);
        assert!(staging_beside(&summary.path).is_empty());
    }

    #[test]
    fn preflight_refuses_an_occupied_destination_without_touching_it() {
        let workspace = TempDir::new().unwrap();
        let out = workspace.path().join("derivative.json");
        fs::write(&out, "somebody else's bytes").unwrap();

        let failure = preflight(&out).unwrap_err();

        assert!(matches!(failure, TriageFailure::OutputExists { .. }));
        assert_eq!(fs::read_to_string(&out).unwrap(), "somebody else's bytes");
        assert!(staging_beside(&out).is_empty());
    }

    #[test]
    fn preflight_refuses_a_missing_or_unusable_parent() {
        let workspace = TempDir::new().unwrap();
        let missing = workspace.path().join("missing").join("derivative.json");
        let obstruction = workspace.path().join("not-a-directory");
        fs::write(&obstruction, "file").unwrap();

        for out in [missing, obstruction.join("derivative.json")] {
            let failure = preflight(&out).unwrap_err();
            assert!(matches!(failure, TriageFailure::OutputUnusable { .. }));
            assert!(!out.exists());
        }
        let malformed = preflight(Path::new("")).unwrap_err();
        assert!(matches!(malformed, TriageFailure::OutputUnusable { .. }));
    }

    #[test]
    fn a_destination_created_between_preflight_and_commit_is_never_overwritten() {
        let workspace = TempDir::new().unwrap();
        let out = workspace.path().join("derivative.json");
        let filtered = project(&manifest(&["keep"]), &triage(Vec::new()));
        let destination = preflight(&out).unwrap();

        let failure = publish_with(destination, &filtered, || {
            fs::write(&out, "arrived during model work").unwrap();
        })
        .unwrap_err();

        assert!(matches!(failure, TriageFailure::OutputExists { .. }));
        assert_eq!(
            fs::read_to_string(&out).unwrap(),
            "arrived during model work"
        );
        assert!(staging_beside(&out).is_empty());
    }

    #[test]
    fn abandoning_a_preflight_removes_its_process_unique_staging_file() {
        let workspace = TempDir::new().unwrap();
        let first = workspace.path().join("first.json");
        let second = workspace.path().join("second.json");
        let first_destination = preflight(&first).unwrap();
        let second_destination = preflight(&second).unwrap();

        let staged = staging_beside(&first);
        assert_eq!(staged.len(), 2);
        assert_ne!(
            first_destination.staging_path(),
            second_destination.staging_path()
        );
        drop(first_destination);
        drop(second_destination);

        assert!(staging_beside(&first).is_empty());
        assert!(!first.exists());
        assert!(!second.exists());
    }
}
