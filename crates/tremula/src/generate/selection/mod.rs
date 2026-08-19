//! Choosing what to mutate out of what a change touched and what a test suite reached.
//!
//! Two inputs, each answering a question the other cannot. A diff says which lines this
//! change is responsible for, without which every run would be about the whole project
//! again. Coverage says which of those lines a test ever reached, without which a
//! mutation is planted where no suite could have caught it and its survival says
//! nothing about anybody's tests.
//!
//! Nothing here asks a model anything. Every step is arithmetic over documents, so the
//! same change and the same coverage select the same functions every time — and the one
//! step that is not arithmetic, asking the language pack where a file's functions are,
//! is asked of the project rather than decided here.
//!
//! # What coverage is, and is not, used for
//!
//! It decides which functions are worth asking about. It does **not** confine a model to
//! the changed lines inside one: a mutation may be proposed anywhere in a function that
//! was selected, which is what makes the proposals worth having. So the claim that a
//! covered selection plants no noise is kept somewhere else — at the moment a mutant is
//! recorded, where one that replaces no line any test reached is refused.

use std::{fs, path::Path};

use tremula_contracts::{
    manifest::{CoverageGap, Generation, LineRange, SelectedFunction, Selection},
    spans::SpansReport,
};

use crate::{
    generate::{
        command::GenerateArgs,
        enrich::SourceFile,
        failures::GenerationFailure,
        plan::{Asked, Planned, describe},
        selection::{
            conventions::inferred_tests,
            lcov::Coverage,
            lines::Lines,
            promote::{Candidate, candidates, ordered},
        },
    },
    paths::absolute_spellings,
    python_env::PythonEnv,
    suppressions::Dismissals,
};

pub mod conventions;
pub mod diff;
pub mod git;
pub mod lcov;
pub mod lines;
pub mod promote;

/// What a selection came to: the record to keep, the files to ask about, and the
/// coverage a mutant is held to before it is recorded.
#[derive(Debug)]
pub struct Selected {
    /// The record of the decision, with the generation counts still to be filled in.
    pub selection: Selection,
    /// The files to ask about, one entry each, in the order their paths sort in.
    pub files: Vec<Planned>,
    /// The coverage document, kept for the check made before a mutant is recorded.
    pub coverage: Option<Coverage>,
    /// How many test files the change touched.
    pub tests_excluded: usize,
}

/// Choose the functions this change is answerable for.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when git cannot make the comparison, when the coverage
/// document cannot be read, or when a changed file cannot be read or described by the
/// language pack. None of those is an empty answer: an empty answer is a selection with
/// nothing in it, which is a success and is recorded as one.
pub fn select(
    args: &GenerateArgs,
    env: &PythonEnv,
    project: &Path,
    dismissals: &Dismissals,
    diff_base: &str,
) -> Result<Selected, GenerationFailure> {
    let merge_base = git::merge_base(project, diff_base)?;
    let changed = diff::changed(&git::since(project, &merge_base)?);
    let coverage = read_coverage(args.coverage.as_deref(), project)?;
    let mut found: Vec<Candidate> = Vec::new();
    let mut gaps: Vec<CoverageGap> = Vec::new();
    let mut absent: Vec<String> = Vec::new();
    let mut outside = 0u32;
    let mut described: Vec<(SourceFile, SpansReport)> = Vec::new();
    for file in &changed.files {
        if let Some(coverage) = &coverage
            && !coverage.knows(&file.file)
        {
            absent.push(file.file.clone());
            continue;
        }
        let (touched, uncovered) = sift(coverage.as_ref(), &file.file, &file.lines);
        if !uncovered.is_empty() {
            gaps.push(CoverageGap {
                file: file.file.clone(),
                ranges: merged(&uncovered),
            });
        }
        if touched.is_empty() {
            continue;
        }
        let (source, report) = describe(args, env, project, &file.file, dismissals)?;
        let promoted = candidates(&file.file, &report, &Lines::of(&source.bytes), &touched);
        outside = outside.saturating_add(promoted.outside);
        found.extend(promoted.candidates);
        described.push((source, report));
    }
    let mut kept = ordered(found);
    let over = kept.split_off(kept.len().min(args.max_functions));
    let functions: Vec<SelectedFunction> = kept
        .iter()
        .map(|candidate| chosen(candidate, &args.project, true))
        .collect();
    Ok(Selected {
        files: to_ask(&functions, described),
        selection: Selection {
            diff_base: diff_base.to_owned(),
            merge_base: Some(merge_base),
            coverage: args.coverage.as_deref().map(|path| recorded(path, project)),
            functions,
            skipped_over_limit: over
                .iter()
                .map(|candidate| chosen(candidate, &args.project, false))
                .collect(),
            coverage_gaps: gaps,
            files_not_in_coverage: absent,
            lines_outside_functions: outside,
        },
        coverage,
        tests_excluded: changed.tests_excluded,
    })
}

/// The changed lines worth asking about, and the changed lines nothing reached.
///
/// Without coverage every changed line is a candidate, which is the degraded selection
/// and is recorded as one. With it, three answers rather than two: a line with hits is a
/// candidate, a line the document measured and nothing reached is a gap, and a line the
/// document does not mention is neither — it is a comment, a blank, or a continuation,
/// and no mutation could land on it.
fn sift(coverage: Option<&Coverage>, file: &str, touched: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let Some(coverage) = coverage else {
        return (touched.to_vec(), Vec::new());
    };
    let mut candidates = Vec::new();
    let mut uncovered = Vec::new();
    for line in touched {
        if coverage.covered(file, *line) {
            candidates.push(*line);
        } else if coverage.measured(file, *line) {
            uncovered.push(*line);
        }
    }
    (candidates, uncovered)
}

/// A rising run of lines as the stretches it makes up, merged where they touch.
fn merged(lines: &[u32]) -> Vec<LineRange> {
    let mut ranges: Vec<LineRange> = Vec::new();
    for line in lines {
        match ranges.last_mut() {
            Some(last) if last.end_line.saturating_add(1) >= *line => last.end_line = *line,
            _ => ranges.push(LineRange {
                start_line: *line,
                end_line: *line,
            }),
        }
    }
    ranges
}

/// One function the selection settled on, or one the limit cut.
///
/// A function the limit cut is recorded without its tests, and that is not an omission:
/// nothing was asked about it, so nothing was read for it either, and a record naming a
/// test file that was never sent anywhere would describe a generation that did not
/// happen.
fn chosen(candidate: &Candidate, project: &Path, asked_about: bool) -> SelectedFunction {
    SelectedFunction {
        file: candidate.file.clone(),
        function: candidate.function.clone(),
        span: candidate.span,
        lines: candidate.lines,
        candidate_lines: candidate.candidate_lines,
        inferred_tests: if asked_about {
            inferred_tests(project, &candidate.file)
        } else {
            Vec::new()
        },
        generation: Generation {
            proposed: 0,
            recorded: 0,
        },
    }
}

/// The chosen functions, grouped under the files they are in.
///
/// One entry per file, in the order the paths sort in, and within a file the functions in
/// the order the selection put them. A file every function of which the limit cut is not
/// asked about at all, and drops out here.
fn to_ask(
    functions: &[SelectedFunction],
    described: Vec<(SourceFile, SpansReport)>,
) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (source, report) in described {
        let asking: Vec<Asked> = functions
            .iter()
            .filter(|function| function.file == source.file)
            .map(|function| Asked {
                spelling: format!(
                    "{}@{}:{}",
                    function.function, function.span.start_byte, function.span.end_byte
                ),
                tests: function.inferred_tests.clone(),
            })
            .collect();
        if asking.is_empty() {
            continue;
        }
        planned.push(Planned {
            source,
            report,
            functions: asking,
        });
    }
    planned
}

/// Read the coverage document, when one was given.
fn read_coverage(
    path: Option<&Path>,
    project: &Path,
) -> Result<Option<Coverage>, GenerationFailure> {
    let Some(path) = path else {
        return Ok(None);
    };
    let document =
        fs::read_to_string(path).map_err(|err| GenerationFailure::CoverageUnreadable {
            path: path.to_path_buf(),
            reason: err.to_string(),
        })?;
    Ok(Some(Coverage::read(&document, project)?))
}

/// The coverage document's path as the record holds it: relative to the project when it
/// is inside it, so that the record travels with the manifest rather than naming a
/// directory on the machine the generation happened to run on.
fn recorded(path: &Path, project: &Path) -> String {
    let spelled = path.to_string_lossy().into_owned();
    if !spelled.starts_with('/') {
        return spelled;
    }
    for root in absolute_spellings(project) {
        if let Some(rest) = spelled.strip_prefix(&root)
            && let Some(inside) = rest.strip_prefix('/')
        {
            return inside.to_owned();
        }
    }
    spelled
}
