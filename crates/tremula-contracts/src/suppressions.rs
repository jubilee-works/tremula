//! What a person has decided not to be shown again.
//!
//! A run's survivors are a list a person works through, and some of what is on it
//! is not worth being shown twice: a mutation that changes nothing, or one that
//! changes something nobody would want a test for. This is the record of those
//! decisions, kept in the project and meant to be committed — it is the project's
//! judgement and not one machine's.
//!
//! # Why a mutation is named by its text and not by its identifier
//!
//! A mutant's identifier is derived from the file's hash, so it changes when
//! anything anywhere in that file changes. A decision keyed by identifier would be
//! orphaned by the next unrelated edit, and the survivor a person dismissed would
//! come back with a new name. So the key is the mutation itself — the file, the text
//! it replaces, and the text it puts there — which is what a person actually
//! decided about. [`Suppression::mutant_id`] is kept beside it as provenance: it
//! says which run's survivor prompted the decision, and it is never matched on.
//!
//! # Why a decision that no longer applies is not deleted
//!
//! The text a suppression names can stop being in the file, and then the decision
//! matches nothing. That is worth saying out loud rather than acting on: a
//! suppression may be stale because the code moved on, or because somebody is on a
//! branch where it has not happened yet. A consumer reports how many it found and
//! keeps every one of them, because silently dropping a person's decision is the
//! one thing a record of decisions must not do.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Every mutation a project has decided not to be shown again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Suppressions {
    /// Contract version of this document.
    pub schema_version: String,
    /// The decisions, in the order they were made. An empty list is valid and
    /// means nothing has been dismissed yet.
    pub suppressions: Vec<Suppression>,
}

/// One mutation a person has dismissed, and what they said about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Suppression {
    /// The file the mutation changes, POSIX-style and relative to the project root.
    pub file: String,
    /// The text it replaces. With `file` and `replacement`, the whole of the key.
    pub original: String,
    /// The text it puts there.
    pub replacement: String,
    /// Why it was dismissed.
    pub reason: DismissalReason,
    /// When, RFC 3339.
    pub dismissed_at: String,
    /// Whatever the person wanted to say about it, for whoever reads this next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The identifier the mutant had when it was dismissed.
    ///
    /// Provenance only, and never matched on: it names the run's survivor that
    /// prompted the decision, and it stops being that mutation's identifier the
    /// next time anything in the file changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutant_id: Option<String>,
}

/// Why a mutation was dismissed.
///
/// Two reasons, and the difference between them is worth keeping: one says the
/// mutation could not have been caught by any test, and the other says it could and
/// that a test for it would not be worth having. Only the second is a statement
/// about what the project wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = crate::open_enum::accepts_any_string)]
pub enum DismissalReason {
    /// It cannot change what the program does, so no test could have caught it.
    Equivalent,
    /// It can, and a test pinning that is not one this project wants.
    NotUseful,
    /// A reason this consumer does not know, written by a newer producer.
    #[serde(other)]
    Unknown,
}
