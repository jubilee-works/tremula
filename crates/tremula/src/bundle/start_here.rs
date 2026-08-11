//! The document a bundle is opened at.
//!
//! `bundle.json` is an index, which makes it the right first file for a machine and
//! the wrong one for a reader — human or agent. The first question either of them
//! has is "what was missed", and the answer is in `report.json`. So the bundle names
//! a starting point, and it says four things: which order to read the documents in,
//! how to reproduce one of the bugs, which suite the run measured, and the one thing
//! the bundle cannot do.
//!
//! That last part is the reason this is generated rather than committed as a
//! template. A bundle carries evidence and not a project, so the reproduction steps
//! depend on whether the run had a revision to name at all, and a fixed text would
//! be wrong for half the bundles ever written.

use std::fmt::Write as _;

use tremula_contracts::bundle::BundleIndex;

use crate::bundle::{
    collect::{MANIFEST, PATCHES, REPORT, RESULTS, TRIAGE},
    logs::{BASELINE_LOG, LOGS},
};

/// Render the document a reader of `index` should meet first.
#[must_use]
pub fn render(index: &BundleIndex) -> String {
    let mut said = String::from("# Start here\n\n");
    said.push_str(
        "This is the evidence one tremula run produced: which planted bugs the project's test suite failed to notice, and what it takes to make one of them fail a test.\n\nRead in this order:\n\n",
    );
    for (number, item) in reading_order(index).iter().enumerate() {
        let _ = writeln!(said, "{}. {item}", number + 1);
    }
    said.push('\n');
    said.push_str(&reproducing(index));
    said.push_str(
        "\nWhat this bundle cannot do is run that suite. It carries the evidence and not the project, so every step above needs a checkout of your own. Paths here are relative to this directory, and `bundle.json` holds the SHA-256 of every document and patch it names.\n",
    );
    said
}

/// The documents, in the order the questions about them arise.
fn reading_order(index: &BundleIndex) -> Vec<String> {
    let mut order = vec![
        format!(
            "`{REPORT}` — every `verdicts` entry whose `verdict` is `survived` is a bug the suite missed."
        ),
        format!(
            "`{MANIFEST}` — the same `mutant_id` under `mutants` gives the file, the byte span, and the exact text the mutation replaces."
        ),
        format!(
            "`{PATCHES}/<mutant-id>.patch` — that change, ready for `git apply`. A `patch` of `null` in `bundle.json` means no file was written and `patch_error` says why; the diff itself is still in `{RESULTS}` under `entries[].diff`."
        ),
    ];
    if index.exposure.standalone_logs {
        order.push(format!(
            "`{LOGS}/<mutant-id>.txt` — what the suite printed under that mutation, with this machine's paths taken out; `{LOGS}/{BASELINE_LOG}` is the same for the unmutated run."
        ));
    }
    if index.documents.triage.is_some() {
        order.push(format!(
            "`{TRIAGE}` — what a second look made of each survivor, and whether any input was found that separates it from the original."
        ));
    }
    order
}

/// How to reproduce one of the bugs, which depends on there being a revision.
fn reproducing(index: &BundleIndex) -> String {
    let mut said = match &index.base.revision {
        Some(revision) if index.base.reproducible_from_revision => format!(
            "Reproduce one from a checkout of the project:\n\n    git checkout {revision}\n    git apply {PATCHES}/<mutant-id>.patch\n\n"
        ),
        Some(revision) => format!(
            "The run measured a working tree with uncommitted changes in it, so `{revision}` is where it started and not what it measured. Expect the patch to need a hand:\n\n    git checkout {revision}\n    git apply {PATCHES}/<mutant-id>.patch\n\n"
        ),
        None => format!(
            "The run was made outside a version-controlled checkout, so `base.revision` is null and there is no revision to return to. Apply the patch to the project as the run measured it:\n\n    git apply {PATCHES}/<mutant-id>.patch\n\n"
        ),
    };
    let _ = writeln!(
        said,
        "Run {} and write a test that fails while the patch is applied. Revert the patch, keep the test, and `tremula run --manifest {MANIFEST} --project <checkout>{}` will report that mutant as killed. That selection collected {} tests before your test was added, so a checkout collecting far fewer is running something else.",
        suite(index),
        selectors(index),
        index.suite.collected
    );
    said
}

/// The suite the run measured, named the way the run named it.
fn suite(index: &BundleIndex) -> String {
    if index.suite.tests.is_empty() {
        return "the project's own default test collection".to_owned();
    }
    let named: Vec<String> = index
        .suite
        .tests
        .iter()
        .map(|test| format!("`{test}`"))
        .collect();
    format!("the suite the run collected — {} —", named.join(" "))
}

/// The same selectors, spelled as the arguments that reproduce them.
fn selectors(index: &BundleIndex) -> String {
    let mut spelled = String::new();
    for test in &index.suite.tests {
        let _ = write!(spelled, " --tests {test}");
    }
    spelled
}
