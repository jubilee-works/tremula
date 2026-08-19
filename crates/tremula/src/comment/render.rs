//! The body of a pull request comment, as markdown.
//!
//! Sections, not a template. Every one of them stands or falls on its own and is left out
//! when it has nothing to say, because the projects this is for do not all want the same
//! ones: a project whose coverage tool already annotates uncovered lines in the diff does
//! not want them a second time here, and can cut that section without touching the rest.
//!
//! # Why it works with nothing to run
//!
//! The most valuable comment this writes is sometimes the one about a change nothing could
//! be mutated in. A pull request whose every changed line is uncovered produces an empty
//! manifest and a run with no verdicts in it — and the record of *that* is the finding. So
//! the headline says what happened rather than assuming something did, and the sections
//! about what was selected, what no test reaches, and what nothing was generated for are
//! built from the manifest alone.

use std::fmt::Write as _;

use tremula_contracts::{
    manifest::{LineRange, Manifest, SelectedFunction, Selection},
    report::{MutantVerdict, Report, Verdict},
    triage::{Classification, Triage, TriageEntry, Undecided},
};

/// How much of a stretch of source a table cell shows before it is cut short.
const QUOTED_SOURCE: usize = 48;

/// The one thing a reader must not misunderstand about a survived mutant.
const SURVIVED_NOTE: &str = "_SURVIVED = not killed by the existing suite; whether the program can reach the mutation is unverified._";

/// What to do about a function this run produced nothing for.
///
/// The section above it names a fact and stops, and a reader who has just been told that a
/// green result is evidence of nothing is owed the next move. The `generate` step said why
/// for each function at the time; asking a model the same question twice is genuinely not the
/// same question twice, so a rerun is the cheap thing to try before this is read as clean.
const BARREN_NEXT: &str = "_The `generate` step's own console output says why for each of them, and a model asked again may well answer differently — so rerunning is the next thing to try before reading this as a clean result._";

/// Everything a comment is written from.
///
/// The manifest comes from the working tree and the rest from the run directory. Not from a
/// bundle: a run with nothing to test produces no bundle at all, and that is exactly the run
/// whose comment matters most.
#[derive(Debug)]
pub struct Evidence<'a> {
    /// The manifest the run was given, including why its targets were chosen.
    pub manifest: &'a Manifest,
    /// What the suite did to each mutant.
    pub report: &'a Report,
    /// What a second look made of the survivors, when one was taken.
    pub triage: Option<&'a Triage>,
    /// Where the evidence for this run was published, when it was published anywhere.
    pub evidence_url: Option<&'a str>,
}

/// The comment body.
#[must_use]
pub fn render(evidence: &Evidence<'_>) -> String {
    let mut sections: Vec<String> = vec![
        "### tremula · mutation testing".to_owned(),
        headline(evidence.report),
    ];
    sections.extend(survivors(evidence));
    sections.extend(what_was_selected(evidence.manifest.selection.as_ref()));
    sections.extend(uncovered(evidence.manifest.selection.as_ref()));
    sections.extend(nothing_generated(evidence.manifest.selection.as_ref()));
    sections.extend(degraded(evidence.manifest.selection.as_ref()));
    sections.extend(evidence.evidence_url.map(|url| {
        format!("[The evidence for this run]({url}) — the patch, the suite's output, and the documents behind every line above.")
    }));
    let mut body = sections.join("\n\n");
    body.push('\n');
    body
}

/// What happened, in one line.
fn headline(report: &Report) -> String {
    let score = report.score;
    if score.total == 0 {
        return "**Nothing was mutated.** No mutant could be planted in what this change touched."
            .to_owned();
    }
    let mut said = format!(
        "{} mutants · {} killed · {} survived",
        score.total, score.killed, score.survived
    );
    let excluded = score.runtime_error + score.not_applied + score.not_run + score.skipped;
    if excluded > 0 {
        let _ = write!(said, " · {excluded} excluded");
    }
    format!("**{said}**")
}

/// The mutants the suite did not catch, and what a second look made of each.
fn survivors(evidence: &Evidence<'_>) -> Vec<String> {
    let survived: Vec<&MutantVerdict> = evidence
        .report
        .verdicts
        .iter()
        .filter(|verdict| verdict.verdict == Verdict::Survived)
        .collect();
    if survived.is_empty() {
        return Vec::new();
    }
    let mut table = String::from("| where | mutation | triage |\n| --- | --- | --- |");
    for verdict in survived {
        let _ = write!(
            table,
            "\n| `{}` | {} | {} |",
            where_it_is(verdict),
            mutation(evidence.manifest, verdict),
            judged(evidence.triage, &verdict.mutant_id)
        );
    }
    vec![
        "#### Mutants the suite did not catch".to_owned(),
        table,
        SURVIVED_NOTE.to_owned(),
    ]
}

/// Where a mutation is, as a person would look for it.
fn where_it_is(verdict: &MutantVerdict) -> String {
    match &verdict.location {
        Some(location) => format!("{}:{}", verdict.file, location.line),
        None => format!("{} at byte {}", verdict.file, verdict.span.start_byte),
    }
}

/// What the mutation changed, from the manifest the run was given.
///
/// The report names a mutant by its identifier and nothing else, which is what a machine
/// needs and what nobody can read. The manifest is where the text of the mutation is.
fn mutation(manifest: &Manifest, verdict: &MutantVerdict) -> String {
    let Some(mutant) = manifest
        .mutants
        .iter()
        .find(|mutant| mutant.id == verdict.mutant_id)
    else {
        return "not in the manifest".to_owned();
    };
    format!(
        "`{}` → `{}`",
        cell(&mutant.original),
        cell(&mutant.replacement)
    )
}

/// What a triage made of one survivor, or that none was taken.
fn judged(triage: Option<&Triage>, mutant_id: &str) -> String {
    let Some(triage) = triage else {
        return "not triaged".to_owned();
    };
    let Some(entry) = triage
        .entries
        .iter()
        .find(|entry| entry.mutant_id == mutant_id)
    else {
        return "not triaged".to_owned();
    };
    classified(entry)
}

/// One entry's classification, with the reason when the reason is the point.
fn classified(entry: &TriageEntry) -> String {
    match entry.classification {
        Classification::DistinguishedAtFunctionLevel => {
            "distinguished at function level".to_owned()
        }
        Classification::SuspectedEquivalent => "suspected equivalent (unverified)".to_owned(),
        Classification::Undecided => match entry.undecided {
            Some(reason) => format!("undecided — {}", why(reason)),
            None => "undecided".to_owned(),
        },
        Classification::Unknown => "a classification this version does not know".to_owned(),
    }
}

/// Why nothing was established about a survivor, in a person's words.
fn why(reason: Undecided) -> &'static str {
    match reason {
        Undecided::NoWitness => "no input was offered to separate the two versions",
        Undecided::WitnessShowedNoDifference => "the input offered showed no difference",
        Undecided::Nondeterministic => "the function did not agree with itself",
        Undecided::Incomparable => "the results cannot be compared across processes",
        Undecided::UnsafeWitness => "the input offered was not one a probe may evaluate",
        Undecided::Method => "the input named a method, which has no receiver a probe names",
        Undecided::NoSuchFunction => "the module defines no function of that name",
        Undecided::TimedOut => "a version did not finish inside the time limit",
        Undecided::NoJudgement => "no judgement could be obtained at all",
        Undecided::Unknown => "a reason this version does not know",
    }
}

/// What the change offered and what was taken from it.
///
/// The limit is named whenever it cut anything, because a silent cap is the one thing this
/// section exists to prevent: a reader who is told "two functions were mutated" and not
/// that a third was left out has been told something misleading.
fn what_was_selected(selection: Option<&Selection>) -> Vec<String> {
    let Some(selection) = selection else {
        return Vec::new();
    };
    let selected = selection.functions.len();
    let capped = selection.skipped_over_limit.len();
    let mut said = format!(
        "{selected} of {} function(s) this change touched were mutated",
        selected + capped
    );
    if capped > 0 {
        let _ = write!(said, ", and {capped} were left out by the limit");
    }
    let _ = write!(said, ", measured against `{}`.", selection.diff_base);
    if selection.lines_outside_functions > 0 {
        let _ = write!(
            said,
            " {} changed line(s) belong to no function and were not mutated.",
            selection.lines_outside_functions
        );
    }
    if !selection.files_not_in_coverage.is_empty() {
        let _ = write!(
            said,
            " The coverage document says nothing about {}.",
            listed(&selection.files_not_in_coverage)
        );
    }
    let mut sections = vec!["#### What was mutated".to_owned(), said];
    if capped > 0 {
        sections.push(
            selection
                .skipped_over_limit
                .iter()
                .map(|over| {
                    format!(
                        "- `{}` · `{}` — left out by the limit",
                        over.file, over.function
                    )
                })
                .collect::<Vec<String>>()
                .join("\n"),
        );
    }
    sections
}

/// The changed lines no test reaches.
fn uncovered(selection: Option<&Selection>) -> Vec<String> {
    let Some(selection) = selection else {
        return Vec::new();
    };
    if selection.coverage_gaps.is_empty() {
        return Vec::new();
    }
    let listed: Vec<String> = selection
        .coverage_gaps
        .iter()
        .map(|gap| {
            format!(
                "- `{}` — {}",
                gap.file,
                gap.ranges
                    .iter()
                    .copied()
                    .map(spelled)
                    .collect::<Vec<String>>()
                    .join(", ")
            )
        })
        .collect();
    vec![
        "#### Changed lines no test reaches".to_owned(),
        listed.join("\n"),
        "_Nothing was mutated on these: a mutant there survives whatever the suite does._"
            .to_owned(),
    ]
}

/// The functions that were asked about and produced nothing.
///
/// The section that makes a green result readable. A function every proposal about which was
/// refused, and a function nothing was ever proposed for, are both functions this run says
/// nothing about — and without this, an empty manifest and a clean suite look the same.
fn nothing_generated(selection: Option<&Selection>) -> Vec<String> {
    let Some(selection) = selection else {
        return Vec::new();
    };
    let barren: Vec<&SelectedFunction> = selection
        .functions
        .iter()
        .filter(|chosen| chosen.generation.recorded == 0)
        .collect();
    if barren.is_empty() {
        return Vec::new();
    }
    let listed: Vec<String> = barren
        .iter()
        .map(|chosen| {
            let how = if chosen.generation.proposed == 0 {
                "nothing was proposed for it".to_owned()
            } else {
                format!(
                    "{} proposal(s) were made and none of them survived the checks",
                    chosen.generation.proposed
                )
            };
            format!("- `{}` · `{}` — {how}", chosen.file, chosen.function)
        })
        .collect();
    vec![
        "#### Functions this run says nothing about".to_owned(),
        listed.join("\n"),
        BARREN_NEXT.to_owned(),
    ]
}

/// The warning a selection made without coverage owes its reader.
fn degraded(selection: Option<&Selection>) -> Vec<String> {
    let Some(selection) = selection else {
        return Vec::new();
    };
    if selection.coverage.is_some() {
        return Vec::new();
    }
    vec![
        "> **Selected without coverage.** A mutant that survived may have survived because no test runs it, rather than because the suite missed it. Pass `--coverage` with an LCOV document of the project's own test run to tell the two apart."
            .to_owned(),
    ]
}

/// One stretch of lines, as a person writes one.
fn spelled(range: LineRange) -> String {
    if range.start_line == range.end_line {
        return range.start_line.to_string();
    }
    format!("{}–{}", range.start_line, range.end_line)
}

/// A handful of file names, in prose.
fn listed(files: &[String]) -> String {
    files
        .iter()
        .map(|file| format!("`{file}`"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// A stretch of source as one table cell.
///
/// One line, because a table row is one line; shortened, because a cell as wide as a
/// function is a table nobody can read; and with the one character that would end a cell
/// early escaped.
fn cell(source: &str) -> String {
    let joined = source.split_whitespace().collect::<Vec<&str>>().join(" ");
    let shortened = match joined.char_indices().nth(QUOTED_SOURCE) {
        Some((cut, _)) => format!("{}…", joined.get(..cut).unwrap_or_default()),
        None => joined,
    };
    shortened.replace('|', "\\|")
}
