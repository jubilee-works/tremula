//! The console report: a view over a judged report, for people. Machines read
//! the report document instead, so nothing here is meant to be parsed.

mod bundle;

use std::fmt::Write as _;

use tremula_contracts::{
    baseline::Baseline,
    report::{MutantVerdict, Report, Score, Verdict},
    runner::RunnerResult,
    triage::{Classification, Triage, TriageEntry},
};

pub use bundle::render_bundle;

use crate::generate::command::{Generated, exit_code};

/// How much of a mutant's identifier is enough to tell it apart on screen.
const SHORT_ID_CHARS: usize = 8;

/// Minimum width of the id, file, span, and verdict columns. The last column
/// takes whatever is left. A wider cell widens its column for the whole table,
/// so the columns stay aligned with each other.
const MIN_COLUMN_WIDTHS: [usize; 4] = [10, 17, 10, 11];

/// Blank space kept between a cell and the next column.
const COLUMN_GAP: usize = 2;

/// Indent that sets the table apart from the summary lines.
const TABLE_INDENT: &str = "  ";

/// How much of a stretch of source is shown before it is cut short.
const QUOTED_SOURCE: usize = 72;

/// The one thing a reader must not misunderstand about a survived mutant.
const SURVIVED_NOTE: &str =
    "SURVIVED = not killed by the existing suite (execution/coverage unverified)";

/// Render a judged report.
///
/// The baseline is optional because a manifest with nothing to test never runs
/// one: there is no suite result to summarize, so that line is left out along
/// with the verdict table.
#[must_use]
pub fn render(report: &Report, baseline: Option<&Baseline>) -> String {
    let mut lines = vec![format!(
        "tremula run · {} mutants · project: {}",
        report.score.total, report.run.project
    )];
    if let Some(baseline) = baseline {
        lines.push(render_baseline(&baseline.runner));
    }
    lines.push(String::new());
    if !report.verdicts.is_empty() {
        lines.extend(render_table(&report.verdicts));
        lines.push(String::new());
    }
    lines.push(render_score(report.score));
    lines.push(format!("note: {SURVIVED_NOTE}"));
    if report.score.timeout > 0 {
        lines.push(format!(
            "warning: {} mutant(s) timed out — inspect before trusting the score",
            report.score.timeout
        ));
    }
    lines.push(format!(
        "exit {} ({})",
        report.exit_code,
        exit_reason(report.score)
    ));
    if let Some(said) = what_next(report.score) {
        lines.push(said);
    }
    lines.join("\n")
}

/// What a reader who has just seen a score would do next, when there is anything.
///
/// Here rather than in a README because the two commands that follow a run are not
/// discoverable from it: a run says what the suite did and stops, and the reader who
/// needs `triage` is the one who just got a list of survivors they have no order to
/// read in. Nothing is suggested for a run that tested nothing, since nothing
/// happened to follow up.
///
/// They are numbered where both apply, because they are not alternatives: a bundle built
/// before a triage is a bundle with `triage.json` missing from it, and this line is the
/// only place a reader would find that out in time.
fn what_next(score: Score) -> Option<String> {
    if score.total == 0 {
        return None;
    }
    let bundling = "`tremula bundle` packages this run's evidence for somebody else";
    if score.survived > 0 {
        return Some(format!(
            "next: 1. `tremula triage --model <model>` sorts the survivors, then 2. {bundling} — in that order, so the triage travels with it"
        ));
    }
    Some(format!("next: {bundling}"))
}

/// Render what a generation produced.
///
/// Every function gets a line whether or not it produced anything, because the
/// exit code cannot say which one was the problem and this is the only place that
/// can. A function whose round stopped early gets a second line saying so: the
/// manifest is written anyway, and a reader who saw only the exit code would not
/// know that what it holds is part of what was asked for.
#[must_use]
pub fn render_generation(generated: &Generated) -> String {
    let mut lines = vec![
        format!(
            "tremula generate · {} · {} function(s) · model: {}",
            generated.file,
            generated.functions.len(),
            generated.model
        ),
        String::new(),
    ];
    for outcome in &generated.functions {
        let gathered = &outcome.gathered;
        let mut line = format!(
            "  {}: {} proposed · {} recorded",
            outcome.function,
            gathered.proposed,
            gathered.mutants.len()
        );
        if gathered.duplicates > 0 {
            let _ = write!(line, " · {} repeated", gathered.duplicates);
        }
        if gathered.suppressed > 0 {
            let _ = write!(line, " · {} suppressed", gathered.suppressed);
        }
        if !gathered.refused.is_empty() {
            let refused: Vec<String> = gathered
                .refused
                .iter()
                .map(|(defect, count)| format!("{defect} ×{count}"))
                .collect();
            let _ = write!(line, " · refused: {}", refused.join(", "));
        }
        lines.push(line);
        if let Some(failure) = &gathered.failure {
            lines.push(format!("  {}: stopped — {failure}", outcome.function));
        }
    }
    lines.push(String::new());
    lines.push(match &generated.manifest {
        Some(path) => format!(
            "wrote {} mutant(s) to {}",
            generated.recorded,
            path.display()
        ),
        None => "wrote nothing: no proposal survived the checks".to_owned(),
    });
    lines.push(format!(
        "tokens: {} prompt · {} completion · {} total",
        generated.tokens.prompt, generated.tokens.completion, generated.tokens.total
    ));
    lines.push(format!(
        "exit {} ({})",
        exit_code(generated),
        generation_exit_reason(generated)
    ));
    lines.join("\n")
}

/// Render what a triage made of a run's survivors.
///
/// The order is the point of the whole command. The survivors one input separated
/// come first and come with their evidence, because those are the ones worth a
/// person's next minute; then the ones nothing was established about, with the
/// reason, because a reason is what says whether looking again would help; then the
/// ones a model called equivalent, which are last because the only thing behind
/// them is that a model said so.
#[must_use]
pub fn render_triage(judged: &Triage) -> String {
    let score = judged.score;
    let mut lines = vec![
        format!(
            "tremula triage · run {} · {} survivor(s) · model: {}",
            judged.run.run_id, score.survivors, judged.judge.model
        ),
        String::new(),
    ];
    for (classification, heading) in [
        (
            Classification::DistinguishedAtFunctionLevel,
            "distinguished at function level",
        ),
        (Classification::Undecided, "undecided"),
        (Classification::SuspectedEquivalent, "suspected equivalent"),
    ] {
        let listed: Vec<&TriageEntry> = judged
            .entries
            .iter()
            .filter(|entry| entry.classification == classification)
            .collect();
        if listed.is_empty() {
            continue;
        }
        lines.push(format!("{heading} ({}):", listed.len()));
        for entry in listed {
            lines.push(format!(
                "  {}  {}  {}–{}",
                entry
                    .mutant_id
                    .chars()
                    .take(SHORT_ID_CHARS)
                    .collect::<String>(),
                entry.file,
                entry.span.start_byte,
                entry.span.end_byte
            ));
            lines.push(format!(
                "    {} → {}",
                one_line(&entry.original),
                one_line(&entry.replacement)
            ));
            lines.push(format!("    {}", entry.detail));
        }
        lines.push(String::new());
    }
    if !judged.dismissed.is_empty() {
        lines.push(format!(
            "already dismissed ({}), not asked about again:",
            judged.dismissed.len()
        ));
        for decision in &judged.dismissed {
            lines.push(format!(
                "  {}  {}  {} → {}",
                decision
                    .mutant_id
                    .chars()
                    .take(SHORT_ID_CHARS)
                    .collect::<String>(),
                decision.file,
                one_line(&decision.original),
                one_line(&decision.replacement)
            ));
        }
        lines.push(String::new());
    }
    lines.push(format!(
        "score: {} distinguished at function level · {} suspected equivalent · {} undecided",
        score.distinguished_at_function_level, score.suspected_equivalent, score.undecided
    ));
    for caveat in &judged.caveats {
        lines.push(format!("note: {caveat}"));
    }
    lines.push(format!(
        "tokens: {} prompt · {} completion · {} total over {} call(s)",
        judged.spend.prompt_tokens,
        judged.spend.completion_tokens,
        judged.spend.total_tokens,
        judged.spend.calls
    ));
    lines.join("\n")
}

/// A stretch of source as one line, so a table stays a table.
fn one_line(source: &str) -> String {
    let joined = source.split_whitespace().collect::<Vec<_>>().join(" ");
    match joined.char_indices().nth(QUOTED_SOURCE) {
        Some((cut, _)) => format!("{}…", &joined[..cut]),
        None => joined,
    }
}

/// Why a generation exits the way it does, in the order the exit code decides it.
fn generation_exit_reason(generated: &Generated) -> String {
    let stopped: Vec<&str> = generated
        .functions
        .iter()
        .filter(|outcome| outcome.gathered.failure.is_some())
        .map(|outcome| outcome.function.as_str())
        .collect();
    if !stopped.is_empty() {
        return format!("partial output: {} did not finish", stopped.join(", "));
    }
    if generated.recorded == 0 {
        return "nothing to run".to_owned();
    }
    format!("{} mutant(s) to run", generated.recorded)
}

fn render_baseline(runner: &RunnerResult) -> String {
    format!(
        "baseline: {} passed in {} ✓ (collected={})",
        runner.passed,
        seconds(runner.duration_ms),
        runner.collected
    )
}

/// Render a duration as whole seconds and tenths, without going through a
/// floating-point value.
fn seconds(duration_ms: u64) -> String {
    let tenths = (duration_ms + 50) / 100;
    format!("{}.{}s", tenths / 10, tenths % 10)
}

fn render_score(score: Score) -> String {
    let excluded = score.runtime_error + score.not_applied + score.not_run + score.skipped;
    format!(
        "score: {}/{} killed ({} timeout) · {} survived · {excluded} excluded",
        score.killed, score.total, score.timeout, score.survived
    )
}

/// Why the run exits the way it does, in the same order the exit code itself is
/// decided.
fn exit_reason(score: Score) -> &'static str {
    if score.total == 0 {
        "nothing to test"
    } else if score.not_applied > 0 {
        "mutations were never applied"
    } else if score.not_run > 0 {
        "mutants were never run"
    } else if score.killed + score.survived == 0 {
        "no usable verdict"
    } else if score.survived > 0 {
        "survived present"
    } else {
        "no survivors"
    }
}

fn render_table(verdicts: &[MutantVerdict]) -> Vec<String> {
    let mut rows = vec![[
        "id".to_owned(),
        "file".to_owned(),
        "span".to_owned(),
        "verdict".to_owned(),
        "detail".to_owned(),
    ]];
    rows.extend(verdicts.iter().map(cells));
    let widths = column_widths(&rows);
    rows.iter().map(|row| render_row(row, &widths)).collect()
}

fn cells(verdict: &MutantVerdict) -> [String; 5] {
    [
        verdict.mutant_id.chars().take(SHORT_ID_CHARS).collect(),
        verdict.file.clone(),
        format!("{}–{}", verdict.span.start_byte, verdict.span.end_byte),
        console_verdict(verdict.verdict).to_owned(),
        verdict.detail.clone(),
    ]
}

fn column_widths(rows: &[[String; 5]]) -> [usize; 4] {
    let mut widths = MIN_COLUMN_WIDTHS;
    for row in rows {
        for (index, width) in widths.iter_mut().enumerate() {
            *width = (*width).max(row[index].chars().count() + COLUMN_GAP);
        }
    }
    widths
}

fn render_row(row: &[String; 5], widths: &[usize; 4]) -> String {
    let mut line = String::from(TABLE_INDENT);
    for (index, width) in widths.iter().enumerate() {
        let cell = &row[index];
        line.push_str(cell);
        line.push_str(&" ".repeat(width.saturating_sub(cell.chars().count())));
    }
    line.push_str(&row[4]);
    line
}

/// Verdicts are `lower_snake_case` everywhere they are stored; the console is
/// the one place they are shouted.
fn console_verdict(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Killed => "KILLED",
        Verdict::Survived => "SURVIVED",
        Verdict::Timeout => "TIMEOUT",
        Verdict::RuntimeError => "RUNTIME_ERROR",
        Verdict::NotApplied => "NOT_APPLIED",
        Verdict::NotRun => "NOT_RUN",
        Verdict::Skipped => "SKIPPED",
    }
}
