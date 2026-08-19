//! Choosing what to mutate out of a real pull request, on a real repository, with the real
//! language pack — and then handing what was chosen to a run, a bundle, and a comment.
//!
//! The repository has to be real here. A selection's first input is a comparison against a
//! revision, and the whole of what could go wrong is in that comparison: which lines git says
//! a change added, whether the base having moved on since matters, whether a rename or a
//! deletion contributes anything. None of that can be faked without faking the answer.
//!
//! The model is replayed. Two of the four scenarios use an answer a provider really sent about
//! this exact function, which is what makes the run downstream of it meaningful: three of those
//! four mutations are caught by this suite and one is not, and a pipeline that mislaid a span
//! or wrote a manifest a run cannot read would fail here rather than in front of somebody.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod scripted_selection;

use std::fs;

use tremula::{
    comment,
    generate::command::{self, DEFAULT_MAX_FUNCTIONS},
};
use tremula_contracts::{
    bundle::BundleIndex,
    manifest::{LineRange, Manifest},
    report::Verdict,
};

use scripted_selection::{BUSY_BODY, COVERED, Change, Fixture, Proposing, Replay, UNCOVERED};

/// The canonical path, end to end: a change to one function is found, mutated, run, and
/// packaged — and the record of why that function was chosen travels the whole way.
#[test]
fn a_pull_request_selects_its_own_changed_function_and_the_reason_travels() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(COVERED));

    let generated = command::compose(&args, &Replay::of_overlaps()).unwrap();

    assert_eq!(command::exit_code(&generated), 0);
    assert_eq!(generated.recorded, 4, "every recorded mutation survived");
    let selection = generated.selection.as_ref().expect("a recorded selection");
    assert_eq!(selection.diff_base, "base");
    assert_eq!(selection.coverage.as_deref(), Some("lcov.info"));
    assert_eq!(
        selection
            .functions
            .iter()
            .map(|chosen| (
                chosen.file.as_str(),
                chosen.function.as_str(),
                chosen.candidate_lines
            ))
            .collect::<Vec<(&str, &str, u32)>>(),
        vec![("ranges.py", "overlaps", 1)],
        "the one function whose line this change rewrote, and not the one beside it"
    );
    let overlaps = &selection.functions[0];
    assert_eq!(
        overlaps.lines,
        LineRange {
            start_line: 4,
            end_line: 6
        },
        "the lines a person would open the file at"
    );
    assert_eq!(
        overlaps.inferred_tests,
        vec!["tests/test_ranges.py".to_owned()],
        "the tests the project's own convention names, found without being told"
    );
    assert_eq!(overlaps.generation.proposed, 4);
    assert_eq!(overlaps.generation.recorded, 4);
    assert!(selection.skipped_over_limit.is_empty());
    assert!(selection.coverage_gaps.is_empty());
    assert!(selection.files_not_in_coverage.is_empty());
    assert_eq!(selection.lines_outside_functions, 0);

    // The manifest is a document, and what makes it usable is that a run reads it — and that
    // the run does not have to know anything about selection to do so.
    let run = fixture.run();
    assert_eq!(
        run.status.code(),
        Some(1),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let report = fixture.report();
    assert_eq!(report.score.total, 4);
    assert_eq!(report.score.killed, 3);
    assert_eq!(report.score.survived, 1);
    assert_eq!(
        fs::read(fixture.run_dir().join("manifest.json")).unwrap(),
        fs::read(fixture.manifest()).unwrap(),
        "a run keeps the manifest it was given byte for byte, selection and all"
    );

    // And a bundle carries the same bytes to whoever the evidence is handed to.
    let bundled = fixture.bundle();
    assert_eq!(
        bundled.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&bundled.stderr)
    );
    let index: BundleIndex = Fixture::read(&fixture.bundled().join("bundle.json"));
    let carried: Manifest = Fixture::read(&fixture.bundled().join(&index.documents.manifest.path));
    assert_eq!(
        carried.selection.as_ref(),
        Some(selection),
        "the reason a function was chosen is in the evidence package, unchanged"
    );

    // Which is what the comment is written from, together with the run's own report.
    let body = fixture.comment(&[]);
    assert!(
        body.contains("**4 mutants · 3 killed · 1 survived**"),
        "{body}"
    );
    assert!(
        body.contains("#### Mutants the suite did not catch"),
        "{body}"
    );
    assert!(body.contains("| `ranges.py:6` |"), "{body}");
    assert!(
        body.contains("1 of 1 function(s) this change touched were mutated"),
        "{body}"
    );
}

/// The empty path, which is the one an implementation is likeliest to get wrong. A change every
/// line of which is uncovered has nothing worth mutating — and the record of that is the most
/// valuable thing the run produces, so it is written, and the exit code says nothing went
/// wrong, because nothing did.
#[test]
fn a_pull_request_no_test_reaches_writes_its_reasons_and_reports_success() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(UNCOVERED));

    let generated = command::compose(&args, &Replay::of_overlaps()).unwrap();

    assert_eq!(
        command::exit_code(&generated),
        0,
        "nothing to do is not a failure"
    );
    assert_eq!(generated.recorded, 0);
    assert!(
        generated.functions.is_empty(),
        "nothing was asked of a model"
    );
    assert!(
        fixture.manifest().exists(),
        "the manifest is written even with nothing in it: it is the only record of the gap"
    );
    let written = fixture.written();
    assert!(written.mutants.is_empty());
    let selection = written
        .selection
        .expect("the reasons are the whole document");
    assert!(selection.functions.is_empty());
    assert_eq!(
        selection.coverage_gaps.len(),
        1,
        "{:?}",
        selection.coverage_gaps
    );
    assert_eq!(selection.coverage_gaps[0].file, "ranges.py");
    assert_eq!(
        selection.coverage_gaps[0].ranges,
        vec![LineRange {
            start_line: 6,
            end_line: 6
        }]
    );

    // A run of it writes a report and stops, which is what makes the workflow one list of
    // commands rather than a branch.
    let run = fixture.run();
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(fixture.run_dir().join("report.json").is_file());

    // And the comment says what was looked at, with no bundle in sight — an empty run produces
    // none, and the comment was never reading one.
    let body = fixture.comment(&[]);
    assert!(body.contains("**Nothing was mutated.**"), "{body}");
    assert!(
        body.contains("#### Changed lines no test reaches"),
        "{body}"
    );
    assert!(body.contains("- `ranges.py` — 6"), "{body}");
}

/// Without coverage the selection still works and is worth less, and both the console and the
/// record say so: a mutant that survives may have survived because nothing runs it.
#[test]
fn a_selection_without_coverage_says_so_in_the_record_and_on_the_console() {
    let fixture = Fixture::new();

    let generated = command::compose(&fixture.selecting(), &Replay::of_overlaps()).unwrap();

    assert_eq!(command::exit_code(&generated), 0);
    let selection = generated.selection.as_ref().unwrap();
    assert!(selection.coverage.is_none());
    assert!(
        selection.coverage_gaps.is_empty(),
        "nothing to compare against"
    );
    assert_eq!(
        selection
            .functions
            .iter()
            .map(|chosen| chosen.function.as_str())
            .collect::<Vec<&str>>(),
        vec!["overlaps"]
    );
    let said = tremula::console::render_generation(&generated);
    assert!(said.contains("selected 1 of 1 functions"), "{said}");
    assert!(
        said.contains("warning: selected without coverage"),
        "{said}"
    );
}

/// The blind spot a measurement found, closed. A pull request that changed only a decorator has
/// a changed line that is covered — a decorator runs when the module is imported — and that no
/// function's own span contains, because a language pack places a function at its `def`.
#[test]
fn a_pull_request_that_changed_only_a_decorator_selects_the_function_under_it() {
    let fixture = Fixture::changing(Change::Decorator);
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(COVERED));

    let generated = command::compose(
        &args,
        &Proposing::about(BUSY_BODY, &format!("not {BUSY_BODY}")),
    )
    .unwrap();

    assert_eq!(command::exit_code(&generated), 0);
    let selection = generated.selection.as_ref().expect("a recorded selection");
    assert_eq!(
        selection
            .functions
            .iter()
            .map(|chosen| (chosen.file.as_str(), chosen.function.as_str()))
            .collect::<Vec<(&str, &str)>>(),
        vec![("routes.py", "busy")],
        "the function the changed decorator decorates"
    );
    assert_eq!(
        selection.lines_outside_functions, 0,
        "the decorator line belongs to the function under it, not to nothing"
    );
    assert_eq!(
        selection.functions[0].lines,
        LineRange {
            start_line: 32,
            end_line: 34
        },
        "the function's own extent, which is where a mutation may land"
    );
    assert_eq!(generated.recorded, 1, "and a mutation of it was recorded");
}

/// The limit, on the real thing: a change that touched two functions and a run told to ask
/// about one of them records which one it left out.
#[test]
fn the_functions_a_limit_leaves_out_are_named_in_the_record() {
    let fixture = Fixture::changing(Change::TwoFunctions);
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(COVERED));
    args.max_functions = 1;

    let generated = command::compose(&args, &Replay::of_overlaps()).unwrap();

    let selection = generated.selection.as_ref().unwrap();
    assert_eq!(selection.functions.len(), 1);
    assert_eq!(
        selection
            .skipped_over_limit
            .iter()
            .map(|over| over.function.as_str())
            .collect::<Vec<&str>>(),
        vec!["merge"]
    );
    assert_eq!(
        DEFAULT_MAX_FUNCTIONS, 5,
        "and the default limit is the one a pull request's worth of cost was reckoned against"
    );
}

/// What a run makes of the mutants a selection chose, named by the mutation rather than by an
/// identifier: the tally alone is the same tally two swapped verdicts would produce.
#[test]
fn the_suite_catches_three_of_the_four_mutations_the_selection_produced() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(COVERED));

    command::compose(&args, &Replay::of_overlaps()).unwrap();
    fixture.run();

    let report = fixture.report();
    let manifest = fixture.written();
    let mut named: Vec<(&str, Verdict)> = report
        .verdicts
        .iter()
        .map(|verdict| {
            let mutant = manifest
                .mutants
                .iter()
                .find(|mutant| mutant.id == verdict.mutant_id)
                .expect("every verdict is about a mutant the manifest carries");
            (mutant.replacement.as_str(), verdict.verdict)
        })
        .collect();
    named.sort_unstable_by_key(|(replacement, _)| *replacement);
    assert_eq!(
        named,
        vec![
            (
                "start < other_end and other_start < other_end",
                Verdict::Killed
            ),
            ("start < other_end and other_start <= end", Verdict::Killed),
            ("start < other_end or other_start < end", Verdict::Killed),
            // The one the suite misses: it pins the touch boundary in the other direction
            // only, so an inclusive `start <= other_end` goes unnoticed.
            (
                "start <= other_end and other_start < end",
                Verdict::Survived
            ),
        ]
    );
}

/// The comment reads the working tree's manifest and the run directory, and a project that has
/// no bundle — which every empty run is — still gets one.
#[test]
fn a_comment_is_made_from_the_manifest_and_the_run_and_nothing_else() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(UNCOVERED));
    command::compose(&args, &Replay::of_overlaps()).unwrap();
    fixture.run();

    let asking = comment::CommentArgs {
        run: None,
        manifest: None,
        project: fixture.project(),
        evidence_url: Some("https://example.invalid/evidence/42".to_owned()),
        github_pr: None,
        github_repo: None,
    };
    let body = comment::body(&asking).expect("a comment about a run that mutated nothing");

    assert!(body.contains("**Nothing was mutated.**"), "{body}");
    assert!(
        body.contains("[The evidence for this run](https://example.invalid/evidence/42)"),
        "{body}"
    );
    assert!(
        !fixture.project().join("tremula-bundle").exists(),
        "and no bundle was needed to write it"
    );
}
