//! What a generation is going to ask about, settled before a model is asked anything.
//!
//! Two ways in, and one shape out. A person names a file and the functions in it. A
//! selection reads a change and a coverage document and names them for itself. Past this
//! point nothing knows which of the two happened, except that a selection leaves the
//! record of its reasoning behind — and that record is the whole difference in what the
//! command promises: a person who named a function and got nothing has a failure, and a
//! selection that found nothing to do has a finding.
//!
//! Every file is held to the same three things before anything is spent on it: it is a
//! file of this project, the language pack's report is about that file, and the report is
//! about the bytes that were read. The last two matter because everything below names
//! bytes by offset, and a report about another version of the file would place every span
//! somewhere it is not.

use std::path::Path;

use tremula_contracts::{manifest::Selection, spans::SpansReport};

use crate::{
    generate::{
        command::GenerateArgs,
        enrich::SourceFile,
        failures::GenerationFailure,
        selection::{lcov::Coverage, select},
    },
    pack,
    python_env::PythonEnv,
    suppressions::{Dismissals, warn_about_stale},
    validation::read_target_file,
};

/// One function to ask about, and what the ask is allowed to show a model about it.
#[derive(Debug, Clone)]
pub struct Asked {
    /// The function, spelled the way `--function` spells one: a name, and the span that
    /// says which one when a file spells the name more than once.
    pub spelling: String,
    /// Test files to read and show a model, POSIX-style and relative to the project.
    pub tests: Vec<String>,
}

/// One file's part of a generation.
#[derive(Debug)]
pub struct Planned {
    /// The file, read once, with the hash everything below is held to.
    pub source: SourceFile,
    /// Where the language pack says this file's functions are.
    pub report: SpansReport,
    /// The functions of it to ask about, in the order they will be asked.
    pub functions: Vec<Asked>,
}

/// Everything a generation is going to do, and why.
#[derive(Debug)]
pub struct Plan {
    /// The files, one entry each.
    pub files: Vec<Planned>,
    /// The record of how the targets were chosen, when they were chosen here.
    pub selection: Option<Selection>,
    /// What the selection left out, for the console to say.
    pub aside: Option<Aside>,
    /// The coverage a mutant is held to before it is recorded.
    pub coverage: Option<Coverage>,
}

/// What a selection left aside, in the numbers a reader is owed.
///
/// Kept beside the record rather than derived from it, because the count of test files a
/// change touched is not something the manifest holds: a change that touched only its own
/// tests has to be distinguishable from one nothing was found in, and the manifest of the
/// second says the same as the manifest of the first.
#[derive(Debug, Clone, Copy)]
pub struct Aside {
    /// How many functions were asked about.
    pub selected: usize,
    /// How many the limit cut.
    pub capped: usize,
    /// How many test files the change touched.
    pub tests_excluded: usize,
    /// Whether the selection ran with no coverage to filter by.
    pub degraded: bool,
}

/// Settle what to ask about, whichever way the caller said it.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when a file cannot be used, when the language pack
/// cannot describe one, or when a selection cannot be made at all.
pub fn plan(
    args: &GenerateArgs,
    env: &PythonEnv,
    project: &Path,
    dismissals: &Dismissals,
) -> Result<Plan, GenerationFailure> {
    if let Some(diff_base) = &args.diff_base {
        let selected = select(args, env, project, dismissals, diff_base)?;
        let aside = Aside {
            selected: selected.selection.functions.len(),
            capped: selected.selection.skipped_over_limit.len(),
            tests_excluded: selected.tests_excluded,
            degraded: selected.selection.coverage.is_none(),
        };
        return Ok(Plan {
            files: selected.files,
            selection: Some(selected.selection),
            aside: Some(aside),
            coverage: selected.coverage,
        });
    }
    let named = args
        .file
        .clone()
        .ok_or(GenerationFailure::NothingToGenerateFor)?;
    let (source, report) = describe(args, env, project, &named, dismissals)?;
    Ok(Plan {
        files: vec![Planned {
            source,
            report,
            functions: args
                .functions
                .iter()
                .map(|spelling| Asked {
                    spelling: spelling.clone(),
                    tests: Vec::new(),
                })
                .collect(),
        }],
        selection: None,
        aside: None,
        coverage: None,
    })
}

/// Read one file and ask the language pack where its functions are.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when the file is not one of this project's, when the
/// pack cannot describe it, or when the pack's answer is about another file or about
/// other bytes than the ones that were read.
pub fn describe(
    args: &GenerateArgs,
    env: &PythonEnv,
    project: &Path,
    file: &str,
    dismissals: &Dismissals,
) -> Result<(SourceFile, SpansReport), GenerationFailure> {
    let source = SourceFile::new(file.to_owned(), read_target_file(&args.project, file)?);
    let report = pack::spans(env, project, file)?;
    // Which file the report is about, before whether it is about the same bytes: two
    // files with the same contents hash the same, so the hash below cannot tell one from
    // the other, and every span in the answer is an offset into whichever one it names.
    if report.file != file {
        return Err(GenerationFailure::ReportIsAboutAnotherFile {
            asked: file.to_owned(),
            answered: report.file.clone(),
        });
    }
    if report.file_sha256 != source.sha256 {
        return Err(GenerationFailure::ReportIsAboutOtherBytes {
            file: file.to_owned(),
        });
    }
    warn_about_stale(dismissals, file, &String::from_utf8_lossy(&source.bytes));
    Ok((source, report))
}
