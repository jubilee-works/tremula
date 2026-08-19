//! The body of a pull request comment, in full, for each of the four runs worth writing one
//! about: the ordinary one, the one nothing could be mutated in, the one somebody triaged,
//! and the one whose evidence was published somewhere.
//!
//! Whole bodies rather than substrings. What this command produces *is* the product — a person
//! reads it on a pull request and decides what to do — so a change to any of it is a change
//! somebody should have to look at.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula_contracts::{
    manifest::{
        Base, CoverageGap, Generation, Language, LineRange, Manifest, Mutant, SelectedFunction,
        Selection, Span,
    },
    report::{MutantVerdict, Report, RunMeta, Score, Verdict},
    results::Location,
    triage::{
        Classification, Judge, Spend, Triage, TriageEntry, TriageRun, TriageScore, Undecided,
    },
};

use tremula::comment::render::{Evidence, render};

/// A manifest carrying one mutation of one selected function.
fn manifest(mutants: Vec<Mutant>, selection: Option<Selection>) -> Manifest {
    Manifest {
        schema_version: "0.1".to_owned(),
        language: Language::Python,
        base: Base {
            revision: Some("9f2c1a4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b".to_owned()),
        },
        mutants,
        selection,
    }
}

fn mutant(id: &str, original: &str, replacement: &str) -> Mutant {
    Mutant {
        id: id.to_owned(),
        file: "ranges.py".to_owned(),
        base_file_sha256: "3cfd5cb2b71ccbb3e57f07c29f6e228748ce2a7caacffbc9c9e020ce31a5f761"
            .to_owned(),
        span: Span {
            start_byte: 118,
            end_byte: 158,
        },
        original: original.to_owned(),
        replacement: replacement.to_owned(),
        description: Some("an off-by-one at a boundary a test might not pin".to_owned()),
        provenance: serde_json::Map::new(),
    }
}

fn selected(function: &str, proposed: u32, recorded: u32) -> SelectedFunction {
    SelectedFunction {
        file: "ranges.py".to_owned(),
        function: function.to_owned(),
        span: Span {
            start_byte: 96,
            end_byte: 214,
        },
        lines: LineRange {
            start_line: 4,
            end_line: 7,
        },
        candidate_lines: 2,
        inferred_tests: vec!["test_ranges.py".to_owned()],
        generation: Generation { proposed, recorded },
    }
}

/// The selection of an ordinary pull request: two functions touched, one of them cut by the
/// limit, one changed line no test reaches.
fn selection() -> Selection {
    Selection {
        diff_base: "origin/main".to_owned(),
        merge_base: Some("9f2c1a4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b".to_owned()),
        coverage: Some("lcov.info".to_owned()),
        functions: vec![selected("overlaps", 4, 1)],
        skipped_over_limit: vec![SelectedFunction {
            function: "merge".to_owned(),
            ..selected("merge", 0, 0)
        }],
        coverage_gaps: vec![CoverageGap {
            file: "ranges.py".to_owned(),
            ranges: vec![LineRange {
                start_line: 6,
                end_line: 6,
            }],
        }],
        files_not_in_coverage: Vec::new(),
        lines_outside_functions: 0,
    }
}

fn report(verdicts: Vec<MutantVerdict>, score: Score) -> Report {
    Report {
        schema_version: "0.1".to_owned(),
        run: RunMeta {
            run_id: "20260819T091244Z-a1b2c3".to_owned(),
            tremula_version: "0.1.0".to_owned(),
            decision_rules_version: "1".to_owned(),
            project: ".".to_owned(),
            tests: Vec::new(),
            observed_revision: Some("9f2c1a4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b".to_owned()),
            dirty: false,
            started_at: "2026-08-19T09:12:44Z".to_owned(),
            finished_at: "2026-08-19T09:14:02Z".to_owned(),
        },
        verdicts,
        score,
        exit_code: 1,
        caveats: Vec::new(),
    }
}

fn survived(id: &str, line: u32) -> MutantVerdict {
    MutantVerdict {
        mutant_id: id.to_owned(),
        file: "ranges.py".to_owned(),
        span: Span {
            start_byte: 118,
            end_byte: 158,
        },
        location: Some(Location { line, column: 4 }),
        verdict: Verdict::Survived,
        label: None,
        detail: "0 failed".to_owned(),
    }
}

fn empty_score() -> Score {
    Score {
        total: 0,
        killed: 0,
        timeout: 0,
        survived: 0,
        runtime_error: 0,
        not_applied: 0,
        not_run: 0,
        skipped: 0,
    }
}

fn triage(classification: Classification, undecided: Option<Undecided>) -> Triage {
    Triage {
        schema_version: "0.1".to_owned(),
        run: TriageRun {
            run_id: "20260819T091244Z-a1b2c3".to_owned(),
            report: "report.json".to_owned(),
            judged_at: "2026-08-19T09:20:00Z".to_owned(),
        },
        judge: Judge {
            model: "gpt-5.2-2025-12-11".to_owned(),
            model_resolved: Some("gpt-5.2-2025-12-11".to_owned()),
            prompt_version: "1".to_owned(),
        },
        entries: vec![TriageEntry {
            mutant_id: "0e2f4a6b".to_owned(),
            file: "ranges.py".to_owned(),
            span: Span {
                start_byte: 118,
                end_byte: 158,
            },
            original: "start < other_end".to_owned(),
            replacement: "start <= other_end".to_owned(),
            classification,
            undecided,
            claim: None,
            witness: None,
            probe: None,
            detail: "one input separated the two versions".to_owned(),
        }],
        dismissed: Vec::new(),
        score: TriageScore {
            survivors: 1,
            distinguished_at_function_level: 1,
            suspected_equivalent: 0,
            undecided: 0,
        },
        spend: Spend::default(),
        caveats: Vec::new(),
    }
}

#[test]
fn the_comment_about_an_ordinary_pull_request() {
    let manifest = manifest(
        vec![mutant(
            "0e2f4a6b",
            "start < other_end",
            "start <= other_end",
        )],
        Some(selection()),
    );
    let report = report(
        vec![survived("0e2f4a6b", 7)],
        Score {
            total: 4,
            killed: 3,
            timeout: 0,
            survived: 1,
            runtime_error: 0,
            not_applied: 0,
            not_run: 0,
            skipped: 0,
        },
    );

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    assert_eq!(
        body,
        concat!(
            "### tremula · mutation testing\n",
            "\n",
            "**4 mutants · 3 killed · 1 survived**\n",
            "\n",
            "#### Mutants the suite did not catch\n",
            "\n",
            "| where | mutation | triage |\n",
            "| --- | --- | --- |\n",
            "| `ranges.py:7` | `start < other_end` → `start <= other_end` | not triaged |\n",
            "\n",
            "_SURVIVED = not killed by the existing suite; whether the program can reach the mutation is unverified._\n",
            "\n",
            "#### What was mutated\n",
            "\n",
            "1 of 2 function(s) this change touched were mutated, and 1 were left out by the limit, measured against `origin/main`.\n",
            "\n",
            "- `ranges.py` · `merge` — left out by the limit\n",
            "\n",
            "#### Changed lines no test reaches\n",
            "\n",
            "- `ranges.py` — 6\n",
            "\n",
            "_Nothing was mutated on these: a mutant there survives whatever the suite does._\n",
        )
    );
}

/// The comment that matters most, and the one an implementation is likeliest to get wrong: a
/// pull request whose every changed line is uncovered has no mutants, no bundle, and one real
/// finding.
#[test]
fn the_comment_about_a_change_nothing_could_be_mutated_in() {
    let manifest = manifest(
        Vec::new(),
        Some(Selection {
            functions: Vec::new(),
            skipped_over_limit: Vec::new(),
            coverage_gaps: vec![CoverageGap {
                file: "ranges.py".to_owned(),
                ranges: vec![
                    LineRange {
                        start_line: 5,
                        end_line: 7,
                    },
                    LineRange {
                        start_line: 10,
                        end_line: 11,
                    },
                ],
            }],
            files_not_in_coverage: vec!["sync.py".to_owned()],
            lines_outside_functions: 2,
            ..selection()
        }),
    );
    let report = report(Vec::new(), empty_score());

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    assert_eq!(
        body,
        concat!(
            "### tremula · mutation testing\n",
            "\n",
            "**Nothing was mutated.** No mutant could be planted in what this change touched.\n",
            "\n",
            "#### What was mutated\n",
            "\n",
            "0 of 0 function(s) this change touched were mutated, measured against `origin/main`. 2 changed line(s) belong to no function and were not mutated. The coverage document says nothing about `sync.py`.\n",
            "\n",
            "#### Changed lines no test reaches\n",
            "\n",
            "- `ranges.py` — 5–7, 10–11\n",
            "\n",
            "_Nothing was mutated on these: a mutant there survives whatever the suite does._\n",
        )
    );
}

#[test]
fn a_triaged_survivor_carries_what_was_established_about_it() {
    let manifest = manifest(
        vec![mutant(
            "0e2f4a6b",
            "start < other_end",
            "start <= other_end",
        )],
        Some(selection()),
    );
    let report = report(
        vec![survived("0e2f4a6b", 7)],
        Score {
            total: 1,
            killed: 0,
            timeout: 0,
            survived: 1,
            runtime_error: 0,
            not_applied: 0,
            not_run: 0,
            skipped: 0,
        },
    );
    let judged = triage(Classification::DistinguishedAtFunctionLevel, None);

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: Some(&judged),
        evidence_url: Some("https://github.com/owner/name/actions/runs/42"),
    });

    assert!(
        body.contains("| `ranges.py:7` | `start < other_end` → `start <= other_end` | distinguished at function level |"),
        "{body}"
    );
    assert!(
        body.ends_with("[The evidence for this run](https://github.com/owner/name/actions/runs/42) — the patch, the suite's output, and the documents behind every line above.\n"),
        "{body}"
    );
}

/// An undecided survivor's classification is worth nothing without the reason, which is the
/// half that says whether looking again would help.
#[test]
fn an_undecided_survivor_says_why_nothing_was_established() {
    let manifest = manifest(
        vec![mutant(
            "0e2f4a6b",
            "start < other_end",
            "start <= other_end",
        )],
        Some(selection()),
    );
    let report = report(
        vec![survived("0e2f4a6b", 7)],
        Score {
            total: 1,
            killed: 0,
            timeout: 0,
            survived: 1,
            runtime_error: 0,
            not_applied: 0,
            not_run: 0,
            skipped: 0,
        },
    );
    let judged = triage(
        Classification::Undecided,
        Some(Undecided::WitnessShowedNoDifference),
    );

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: Some(&judged),
        evidence_url: None,
    });

    assert!(
        body.contains("undecided — the input offered showed no difference"),
        "{body}"
    );
}

/// The section that makes a green result readable. A function every proposal about which was
/// refused produced no evidence, and nothing else in the comment would say so.
#[test]
fn a_function_nothing_survived_for_is_named() {
    let manifest = manifest(
        Vec::new(),
        Some(Selection {
            functions: vec![selected("overlaps", 8, 0), selected("merge", 0, 0)],
            skipped_over_limit: Vec::new(),
            coverage_gaps: Vec::new(),
            ..selection()
        }),
    );
    let report = report(Vec::new(), empty_score());

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    assert!(
        body.contains("#### Functions this run says nothing about"),
        "{body}"
    );
    assert!(
        body.contains(
            "- `ranges.py` · `overlaps` — 8 proposal(s) were made and none of them survived the checks"
        ),
        "{body}"
    );
    assert!(
        body.contains("- `ranges.py` · `merge` — nothing was proposed for it"),
        "{body}"
    );
    assert!(
        body.contains(
            "_The `generate` step's own console output says why for each of them, and a model \
             asked again may well answer differently — so rerunning is the next thing to try \
             before reading this as a clean result._"
        ),
        "a reader told the result is evidence of nothing is owed the next move: {body}"
    );
}

#[test]
fn a_selection_made_without_coverage_warns_the_reader_of_the_comment_too() {
    let manifest = manifest(
        Vec::new(),
        Some(Selection {
            coverage: None,
            coverage_gaps: Vec::new(),
            functions: vec![selected("overlaps", 4, 1)],
            skipped_over_limit: Vec::new(),
            ..selection()
        }),
    );
    let report = report(Vec::new(), empty_score());

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    assert!(body.contains("> **Selected without coverage.**"), "{body}");
    assert!(
        !body.contains("Changed lines no test reaches"),
        "there is nothing to say about coverage gaps without coverage: {body}"
    );
}

/// A manifest somebody wrote by hand has no selection in it, and a comment about it says
/// what the run did without inventing reasons nobody recorded.
#[test]
fn a_run_of_a_manifest_nobody_selected_for_still_gets_a_comment() {
    let manifest = manifest(
        vec![mutant(
            "0e2f4a6b",
            "start < other_end",
            "start <= other_end",
        )],
        None,
    );
    let report = report(
        vec![survived("0e2f4a6b", 7)],
        Score {
            total: 2,
            killed: 1,
            timeout: 0,
            survived: 1,
            runtime_error: 0,
            not_applied: 0,
            not_run: 0,
            skipped: 0,
        },
    );

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    assert_eq!(
        body,
        concat!(
            "### tremula · mutation testing\n",
            "\n",
            "**2 mutants · 1 killed · 1 survived**\n",
            "\n",
            "#### Mutants the suite did not catch\n",
            "\n",
            "| where | mutation | triage |\n",
            "| --- | --- | --- |\n",
            "| `ranges.py:7` | `start < other_end` → `start <= other_end` | not triaged |\n",
            "\n",
            "_SURVIVED = not killed by the existing suite; whether the program can reach the mutation is unverified._\n",
        )
    );
}

/// A table cell is one line and a table row ends at a pipe. A mutation that spans lines, or
/// holds a pipe, has to arrive as a cell rather than as the end of the table.
#[test]
fn a_mutation_that_would_break_the_table_is_made_into_a_cell() {
    let manifest = manifest(
        vec![mutant(
            "0e2f4a6b",
            "if a\n    and b",
            "if a | b or something considerably longer than any table would ever want to show",
        )],
        None,
    );
    let report = report(
        vec![survived("0e2f4a6b", 7)],
        Score {
            total: 1,
            killed: 0,
            timeout: 0,
            survived: 1,
            runtime_error: 0,
            not_applied: 0,
            not_run: 0,
            skipped: 0,
        },
    );

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    let row = body
        .lines()
        .find(|line| line.contains("ranges.py:7"))
        .expect("the survivor's row");
    assert!(row.contains("`if a and b`"), "{row}");
    assert!(row.contains("\\|"), "a pipe is escaped: {row}");
    assert!(row.ends_with('|'), "the row still ends the row: {row}");
    assert!(row.contains('…'), "and a long mutation is cut short: {row}");
}

/// A verdict the language pack gave no position for still names where it is, because the byte
/// offset is the one thing every mutation has.
#[test]
fn a_survivor_with_no_line_reported_for_it_is_named_by_its_offset() {
    let manifest = manifest(
        vec![mutant(
            "0e2f4a6b",
            "start < other_end",
            "start <= other_end",
        )],
        None,
    );
    let report = report(
        vec![MutantVerdict {
            location: None,
            ..survived("0e2f4a6b", 7)
        }],
        Score {
            total: 1,
            killed: 0,
            timeout: 0,
            survived: 1,
            runtime_error: 0,
            not_applied: 0,
            not_run: 0,
            skipped: 0,
        },
    );

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    assert!(body.contains("`ranges.py at byte 118`"), "{body}");
}

/// Mutants that were never really run are neither killed nor survived, and a headline that
/// left them out would not add up to the number of mutants beside it.
#[test]
fn the_headline_accounts_for_the_mutants_that_were_excluded() {
    let manifest = manifest(Vec::new(), None);
    let report = report(
        Vec::new(),
        Score {
            total: 5,
            killed: 3,
            timeout: 1,
            survived: 1,
            runtime_error: 1,
            not_applied: 0,
            not_run: 0,
            skipped: 0,
        },
    );

    let body = render(&Evidence {
        manifest: &manifest,
        report: &report,
        triage: None,
        evidence_url: None,
    });

    assert!(
        body.contains("**5 mutants · 3 killed · 1 survived · 1 excluded**"),
        "{body}"
    );
}
