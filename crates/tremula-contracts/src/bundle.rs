//! The index of a packaged run: what a bundle carries, and how to check it.
//!
//! A run directory is a working space — it holds a backend's own database, a
//! generated configuration, and the sources as they were before the run. A bundle
//! is what is handed to somebody else, and it is a different thing: an immutable
//! set of documents, each named and hashed, with enough beside them to reproduce
//! the bug one of them describes.
//!
//! # This document is an index, not a summary
//!
//! It copies no verdict and no count. `report.json` is the record of what the
//! suite did, and a second copy of a verdict is a second thing that can be wrong:
//! a bundle damaged in transit, or uploaded in part, would disagree with itself in
//! a way nothing here could detect. So a consumer is pointed at the report and
//! given its hash, and everything about outcomes is read from there.
//!
//! The same reasoning retires a rendered command line. A string spelling out how
//! to re-run the suite goes stale the moment the command-line surface changes, and
//! nothing would fail when it did; [`Suite::tests`] is the selectors themselves,
//! and whoever renders a command renders it from those.
//!
//! # What a bundle does not promise
//!
//! It does not promise that the suite can be run from the bundle alone. The
//! evidence is self-contained; running the suite needs a checkout of the project,
//! which is what [`Base::revision`] is for.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Everything a bundle carries, and where in it each thing is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BundleIndex {
    /// Contract version of this document.
    pub schema_version: String,
    /// The run whose evidence this is, shared with every document in the bundle.
    pub run_id: String,
    /// Version of the tremula core that packaged it.
    pub tremula_version: String,
    /// The project's directory name, and only its name. A bundle travels, and the
    /// path it was built at names somebody's machine rather than the project.
    pub project: String,
    /// The contract documents, each with the hash of the bytes as packaged.
    pub documents: Documents,
    /// The checkout a patch applies to.
    pub base: Base,
    /// What suite the run measured, in the terms needed to measure it again.
    pub suite: Suite,
    /// What the suite printed with nothing mutated, or `null` when no log travels.
    ///
    /// Its own field because it belongs to no mutant: it is the run-wide log a reader
    /// compares every mutant's output against, and nothing under [`BundleIndex::attachments`]
    /// could hold it. Written even when absent, for the same reason
    /// [`Attachment::log`] is: "no log travelled" is a fact about the bundle.
    #[serde(default)]
    pub baseline_log: Option<Attached>,
    /// One entry per mutant in the run, in the report's order.
    ///
    /// `default` without the empty-skip the other lists in these contracts use, and
    /// deliberately: one entry per mutant is what this list is for, so a bundle that
    /// omitted it would read as a bundle of no mutants rather than as a shorter list.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// What kinds of content a reader is about to distribute.
    pub exposure: Exposure,
    /// Whether any log in the bundle was shortened. Said once for the bundle,
    /// because the question a reader has is whether to trust the logs at all.
    pub logs_truncated: bool,
}

/// The contract documents a bundle carries, by path and by hash.
///
/// Four of them always, because each answers a question the others cannot: the
/// report holds the verdicts, the manifest holds the text and span every mutant
/// identifier is derived from, the results hold the machine-readable execution
/// signals, and the baseline says what the suite did unmutated.
///
/// The suite's own output is not here. A log is not a contract document — it is
/// whatever a test framework printed — so the logs are named beside the mutant they
/// belong to, or in [`BundleIndex::baseline_log`] when they belong to none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Documents {
    /// The run's verdicts. This is what says which mutants survived.
    pub report: Attached,
    /// What each mutant replaces, and where.
    pub manifest: Attached,
    /// The neutral execution signals, per mutant.
    pub results: Attached,
    /// What the suite did with nothing mutated.
    pub baseline: Attached,
    /// A second opinion on the survivors, when one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<Attached>,
}

/// One file in the bundle: where it is, and what it must hash to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Attached {
    /// Path inside the bundle, POSIX-style and relative to its root.
    pub path: String,
    /// SHA-256 of the file's raw bytes, as lowercase hexadecimal.
    pub sha256: String,
}

/// The revision a patch in this bundle applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Base {
    /// The revision the run observed, or `null` when the project was not a
    /// repository. Written either way rather than left out, because it is what
    /// [`Base::reproducible_from_revision`] is a statement about.
    #[serde(default)]
    pub revision: Option<String>,
    /// Whether the working tree had changes of its own when the run started.
    pub dirty: bool,
    /// Whether checking out `revision` reproduces what the run measured, which
    /// holds when there is a revision and the tree was clean.
    pub reproducible_from_revision: bool,
}

/// What the run measured, in the terms needed to measure it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Suite {
    /// The selectors the run was told to collect, in the order it was given them.
    /// Empty means the project's own default collection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    /// How many tests the unmutated suite collected. A reader whose own run
    /// collects a different number is measuring a different suite.
    pub collected: u32,
}

/// What travels with one mutant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Attachment {
    /// The mutant this is about, as the manifest and the report name it.
    pub id: String,
    /// The patch that reproduces it, or `null` when none was written.
    ///
    /// `null` means the file was not created, and never that a diff was withheld:
    /// the unified diff of every mutant is in `results.json` regardless, and
    /// `patch_error` says what was wrong with this one.
    #[serde(default)]
    pub patch: Option<Attached>,
    /// What the suite printed under this mutant, or `null` when no log travels.
    #[serde(default)]
    pub log: Option<Attached>,
    /// Why no patch was written. Present when, and only when, `patch` is absent
    /// for a reason — in the document rather than only on a console, because the
    /// reader who needs it is not the one who ran the command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_error: Option<String>,
}

/// What kinds of content a bundle holds.
///
/// One boolean cannot say whether a bundle is safe to publish, so this says what
/// is actually in it and leaves the judgement to whoever is about to distribute
/// it. Every field is about content that is present, never about a promise.
///
/// Four independent facts and not a state: any combination of them can happen, and
/// collapsing them into fewer values is exactly the false labelling this replaced.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Exposure {
    /// A patch carries the source lines around the change, so some of the
    /// project's source travels with the bundle. Always true of a bundle with any
    /// patch in it.
    pub patch_context: bool,
    /// Whether log files travel beside the documents. False when they were left
    /// out, which does not make the bundle free of test output — see
    /// `backend_raw_output`.
    pub standalone_logs: bool,
    /// Always true: `results.json` is kept byte for byte, and the execution
    /// backend's own output is inside it, markers and absolute paths included.
    /// Leaving out the log files does not change that.
    pub backend_raw_output: bool,
    /// Whether absolute paths remain anywhere in the bundle. True while
    /// `report.json` is carried unchanged, since its `project` is the root as it
    /// was spelled on the command line.
    pub absolute_paths: bool,
}
