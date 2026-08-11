//! The document a bundle is opened at.
//!
//! `bundle.json` is an index, which makes it the right first file for a machine and
//! the wrong one for a reader — human or agent. The first question either of them
//! has is "what was missed", and the answer is in `report.json`. So the bundle names
//! a starting point, and it says four things: which order to read the documents in,
//! how to reproduce one of the mutations, which suite the run measured, and the one
//! thing the bundle cannot do.
//!
//! That last part is the reason this is generated rather than committed as a
//! template. A bundle carries evidence and not a project, so the reproduction steps
//! depend on whether the run had a revision to name at all, and a fixed text would
//! be wrong for half the bundles ever written.
//!
//! # Why every command carries a placeholder
//!
//! Because two directories are involved and neither is where the other is. The paths in
//! this bundle are the bundle's, and a checkout of the project is somewhere else
//! altogether; a command that named a patch relative to the bundle and expected a
//! repository around it could not be run from either place. So each of the two is a name
//! the reader substitutes, and every command says which directory it acts on rather than
//! leaving it to a working directory nobody wrote down.

use std::fmt::Write as _;

use tremula_contracts::bundle::BundleIndex;

use crate::bundle::{
    INDEX, START_HERE,
    collect::{MANIFEST, PATCHES, REPORT, RESULTS, TRIAGE},
    logs::{BASELINE_LOG, LOGS},
};

/// This bundle, wherever the reader has put it.
const BUNDLE: &str = "<BUNDLE>";

/// A checkout of the project the run measured, which the bundle does not carry.
const CHECKOUT: &str = "<CHECKOUT>";

/// The mutation the reader is working on.
const MUTANT: &str = "<mutant-id>";

/// Render the document a reader of `index` should meet first.
#[must_use]
pub fn render(index: &BundleIndex) -> String {
    let mut said = String::from("# Start here\n\n");
    let _ = writeln!(
        said,
        "This is the evidence one tremula run produced: the planted mutations the project's test suite did not kill, and what it takes to make one of them fail a test. In every command below, substitute `{BUNDLE}` (this directory), `{CHECKOUT}` (a checkout of the project) and `{MUTANT}` (the mutation you are working on).\n"
    );
    said.push_str("Read in this order:\n\n");
    for (number, item) in reading_order(index).iter().enumerate() {
        let _ = writeln!(said, "{}. {item}", number + 1);
    }
    said.push('\n');
    said.push_str(&reproducing(index));
    said
}

/// The documents, in the order the questions about them arise.
///
/// The triage comes first when there is one. A survivor is a mutation the suite did not
/// kill, which is not the same as one it could have killed: some mutations cannot change
/// what the program does at all. The triage is the only thing in the bundle that separates
/// the two, so a reader who meets it last has already spent their time on the wrong
/// mutation.
fn reading_order(index: &BundleIndex) -> Vec<String> {
    let mut order = Vec::new();
    if index.documents.triage.is_some() {
        order.push(format!(
            "`{TRIAGE}` — start at the `entries` classified `distinguished_at_function_level`: an input was found that told those mutations apart from the original, so they are the ones to spend time on first."
        ));
    }
    order.push(format!(
        "`{REPORT}` — every `verdicts` entry whose `verdict` is `survived` is a mutation the suite did not kill. That is a candidate gap rather than a proven one, since a mutation that changes nothing cannot be killed by any test.{}",
        if index.documents.triage.is_some() {
            ""
        } else {
            " Nothing here separates the two; `tremula triage` is what does."
        }
    ));
    order.push(format!(
        "`{MANIFEST}` — the same `mutant_id` under `mutants` gives the file, the byte span, and the exact text the mutation replaces."
    ));
    order.push(format!(
        "`{PATCHES}/{MUTANT}.patch` — that change, ready for `git apply`. A `patch` of `null` in `{INDEX}` means no file was written and `patch_error` says why; the diff itself is still in `{RESULTS}` under `entries[].diff`."
    ));
    if index.exposure.standalone_logs {
        order.push(format!(
            "`{LOGS}/{MUTANT}.txt` — what the suite printed under that mutation, with this machine's paths taken out; `{LOGS}/{BASELINE_LOG}` is the same for the unmutated run."
        ));
    }
    order
}

/// How to reproduce one of the mutations, which depends on there being a revision.
fn reproducing(index: &BundleIndex) -> String {
    let patch = applying("");
    let mut said = match &index.base.revision {
        Some(revision) if index.base.reproducible_from_revision => {
            format!("Reproduce one:\n\n    git -C {CHECKOUT} checkout {revision}\n    {patch}\n\n")
        }
        Some(revision) => format!(
            "The run measured a working tree with uncommitted changes in it, so `{revision}` is where it started and not what it measured. Expect the patch to need a hand:\n\n    git -C {CHECKOUT} checkout {revision}\n    {patch}\n\n"
        ),
        None => format!(
            "The run was made outside a version-controlled checkout, so `base.revision` is null and there is no revision to return to. Apply the patch to the project as the run measured it:\n\n    {patch}\n\n"
        ),
    };
    let _ = writeln!(
        said,
        "Write a test in `{CHECKOUT}` that fails while that patch is applied. Then take the patch back out — `{}` — keep the test, and run:\n\n    tremula run --manifest {BUNDLE}/{MANIFEST} --project {CHECKOUT}{}\n",
        applying(" -R"),
        selectors(index)
    );
    let _ = writeln!(
        said,
        "That reports the mutation as killed. {} collected {} tests before your test was added, so a checkout collecting far fewer is running something else. What this bundle cannot do is run that suite: it carries the evidence and not the project. Every path it names is relative to `{BUNDLE}`, and `{INDEX}` holds the SHA-256 of every file in it but itself and `{START_HERE}`.",
        collection(index),
        index.suite.collected
    );
    said
}

/// The command that puts one mutation into a checkout, or takes it back out.
fn applying(flag: &str) -> String {
    format!("git -C {CHECKOUT} apply{flag} {BUNDLE}/{PATCHES}/{MUTANT}.patch")
}

/// The suite the run measured, named the way the run named it.
fn collection(index: &BundleIndex) -> &'static str {
    if index.suite.tests.is_empty() {
        "The project's own default test collection"
    } else {
        "The selection above"
    }
}

/// The same selectors, spelled as the arguments that reproduce them.
///
/// Quoted, because a selector is whatever was passed to `tremula run --tests` and a shell
/// is what reads this back: a `::` node identifier or a path with a space in it is one
/// argument only while the quotes are there. A quote of its own inside one cannot be
/// quoted, so the run is closed, the quote given on its own, and the run opened again —
/// which a shell reads as the one argument it was.
fn selectors(index: &BundleIndex) -> String {
    let mut spelled = String::new();
    for test in &index.suite.tests {
        let _ = write!(spelled, " --tests '{}'", test.replace('\'', r"'\''"));
    }
    spelled
}
