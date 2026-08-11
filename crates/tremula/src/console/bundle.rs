//! The console account of a bundle, for the person deciding whether to send it.
//!
//! Kept apart from the report's own rendering because it answers a different
//! question. A report says what the suite did; this says what is in a directory
//! somebody is about to hand over, which is a question about content and exposure
//! rather than about verdicts.

use tremula_contracts::bundle::BundleIndex;

use crate::{
    bundle::{START_HERE, Written},
    console::SHORT_ID_CHARS,
};

/// Render what a bundle was packaged with.
///
/// Every line is about content rather than about intent, because the reader of this
/// output is deciding whether to send the directory somewhere: what documents are in
/// it, how many patches were kept and how many refused, what kinds of thing the
/// bundle exposes, and whether a checkout will reproduce any of it.
#[must_use]
pub fn render_bundle(written: &Written) -> String {
    let index = &written.index;
    let kept = index
        .attachments
        .iter()
        .filter(|attachment| attachment.patch.is_some())
        .count();
    let mut lines = vec![
        format!(
            "tremula bundle · run {} · project: {}",
            index.run_id, index.project
        ),
        String::new(),
        format!("  documents: {}", packaged(index)),
        format!("  patches: {kept} of {} mutant(s)", index.attachments.len()),
        format!("  logs: {}", logs(index)),
        format!("  exposure: {}", exposed(index)),
        format!("  base: {}", reproducibility(index)),
        String::new(),
    ];
    for attachment in &index.attachments {
        if let Some(said) = &attachment.patch_error {
            let unusable = written.unusable.contains(&attachment.id);
            lines.push(format!(
                "{}: no patch for {} — {said}",
                if unusable { "error" } else { "note" },
                attachment
                    .id
                    .chars()
                    .take(SHORT_ID_CHARS)
                    .collect::<String>()
            ));
        }
    }
    lines.push(format!("wrote {}", written.path.display()));
    lines.push(format!(
        "next: hand this directory over; `{START_HERE}` in it says how to read it"
    ));
    lines.push(format!(
        "exit {} ({})",
        if written.unusable.is_empty() { 0 } else { 2 },
        bundle_exit_reason(written)
    ));
    lines.join("\n")
}

/// Why a bundle exits the way it does.
fn bundle_exit_reason(written: &Written) -> String {
    match written.unusable.len() {
        0 => format!("{} mutant(s) packaged", written.index.attachments.len()),
        refused => format!("{refused} survivor(s) have no patch that applies"),
    }
}

/// The contract documents the bundle carries, in reading order.
fn packaged(index: &BundleIndex) -> String {
    let documents = &index.documents;
    let mut named = vec![
        documents.report.path.as_str(),
        documents.manifest.path.as_str(),
        documents.results.path.as_str(),
        documents.baseline.path.as_str(),
    ];
    if let Some(triage) = &documents.triage {
        named.push(triage.path.as_str());
    }
    named.join(", ")
}

/// How many logs travel, and whether any of them was shortened.
fn logs(index: &BundleIndex) -> String {
    let carried = index
        .attachments
        .iter()
        .filter(|attachment| attachment.log.is_some())
        .count();
    if !index.exposure.standalone_logs {
        return "none".to_owned();
    }
    let shortened = if index.logs_truncated {
        ", some shortened"
    } else {
        ""
    };
    format!("{carried} file(s), machine paths removed{shortened}")
}

/// What kinds of content the bundle holds, named one by one.
fn exposed(index: &BundleIndex) -> String {
    let exposure = index.exposure;
    let mut named = Vec::new();
    if exposure.patch_context {
        named.push("source context in patches");
    }
    if exposure.standalone_logs {
        named.push("test output as log files");
    }
    if exposure.backend_raw_output {
        named.push("backend output inside results.json");
    }
    if exposure.absolute_paths {
        named.push("absolute paths in report.json");
    }
    if named.is_empty() {
        return "documents only".to_owned();
    }
    named.join(" · ")
}

/// Whether a checkout reproduces what the run measured.
fn reproducibility(index: &BundleIndex) -> String {
    match &index.base.revision {
        Some(revision) if index.base.reproducible_from_revision => {
            format!("{revision} — a checkout of it reproduces what was measured")
        }
        Some(revision) => format!(
            "{revision}, with uncommitted changes — a checkout of it is not what was measured"
        ),
        None => "no revision — the project was not a version-controlled checkout".to_owned(),
    }
}
