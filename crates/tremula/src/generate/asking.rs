//! What one function's ask is made of, and what every answer to it is held to.
//!
//! Three things stand between a proposal and a manifest, and they are here rather than in
//! the command because each of them is a rule about one mutation rather than a step of the
//! whole. The project's own language pack says whether the mutation is one this language can
//! express. Coverage, when there is any, says whether it replaces anything a test ever ran.
//! And the project's record of dismissals says whether somebody has already decided they do
//! not want to see it again.

use std::{fs, path::Path};

use tremula_contracts::{
    SCHEMA_VERSION,
    manifest::{Base, Language, Manifest, Mutant},
};

use crate::{
    generate::{
        CoveringTest, Defect, GenerationRequest,
        command::GenerateArgs,
        enrich::{SourceFile, Target},
        failures::GenerationFailure,
        plan::Asked,
        round::Gathered,
        selection::{lcov::Coverage, lines::Lines},
    },
    pack::{self, PackError},
    python_env::PythonEnv,
    suppressions::Dismissals,
    validation::read_target_file,
};

/// What a mutation is refused for when no test ever reached the lines it replaces.
pub const ON_UNCOVERED_LINE: &str = "mutant_on_uncovered_line";

/// Whether a mutation replaces anything a test ever ran.
///
/// The last thing asked before a mutant is recorded, and only when a coverage document was
/// given. Coverage decides which functions are worth asking about; it does not confine a
/// model to the changed lines inside one, and it should not — the proposal worth having is
/// often elsewhere in the function. But a mutation of a line no test reaches survives
/// whatever the suite does, and a survivor that was never executed is noise wearing the
/// clothes of a finding.
#[must_use]
pub fn landed_where_nothing_runs(
    coverage: Option<&Coverage>,
    lines: &Lines<'_>,
    mutant: &Mutant,
) -> Option<Defect> {
    let coverage = coverage?;
    let extent = lines.range_of(mutant.span);
    if (extent.start_line..=extent.end_line).any(|line| coverage.covered(&mutant.file, line)) {
        return None;
    }
    Some(Defect {
        defect: ON_UNCOVERED_LINE.to_owned(),
        detail: format!(
            "no test reaches line {} of `{}`, so a mutation there would survive whatever the suite does; propose one on a line the suite runs",
            extent.start_line, mutant.file
        ),
    })
}

/// The test files one function's ask may show a model: the ones the caller named, and the
/// one the project's convention names for the file this function is in.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when one of them is not a file this project owns or cannot
/// be read.
pub fn shown_for(
    args: &GenerateArgs,
    named: &[CoveringTest],
    asked: &Asked,
) -> Result<Vec<CoveringTest>, GenerationFailure> {
    if asked.tests.is_empty() {
        return Ok(named.to_vec());
    }
    let mut shown = named.to_vec();
    for found in covering_tests(&args.project, &asked.tests)? {
        if !shown.iter().any(|already| already.file == found.file) {
            shown.push(found);
        }
    }
    Ok(shown)
}

/// Take out the mutants a person has already dismissed, and count them.
///
/// By the mutation and not by the identifier: an identifier is derived from the
/// file's hash and would be orphaned by the next unrelated edit, so a decision keyed
/// by one would stop applying without anybody deciding that it should.
pub fn set_aside_what_was_dismissed(gathered: &mut Gathered, dismissals: &Dismissals) {
    if dismissals.is_empty() {
        return;
    }
    let before = gathered.mutants.len();
    gathered.mutants.retain(|mutant| {
        dismissals
            .covering(&mutant.file, &mutant.original, &mutant.replacement)
            .is_none()
    });
    gathered.suppressed = before - gathered.mutants.len();
}

/// What one function is asked about.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when the span the language pack reported for the function
/// is not text this file holds.
pub fn ask_about(
    file: &SourceFile,
    target: &Target,
    tests: &[CoveringTest],
    count: usize,
) -> Result<GenerationRequest, GenerationFailure> {
    let function = target.function();
    let source =
        file.text(function.span)
            .ok_or_else(|| GenerationFailure::FunctionSourceUnreadable {
                name: function.qualified_name.clone(),
                file: file.file.clone(),
                start: function.span.start_byte,
                end: function.span.end_byte,
            })?;
    Ok(GenerationRequest {
        file: file.file.clone(),
        source: source.to_owned(),
        tests: tests.to_vec(),
        excluded: target
            .function()
            .excluded
            .iter()
            .filter_map(|excluded| file.text(excluded.span))
            .map(str::to_owned)
            .collect(),
        mutant_count: count,
        feedback: None,
    })
}

/// Ask the language pack whether one mutant is one this language can express.
///
/// One mutant per ask, because the pack's own validation stops at the first defect
/// it finds: a batch would report one and say nothing about the rest, and every
/// defect that is not reported is a correction that cannot be asked for.
///
/// # Errors
///
/// Returns [`PackError`] when the check itself could not be made, which is nobody's mistake
/// and must not be corrected as one. A mutation the language refuses comes back as a defect.
pub fn check_with_the_pack(
    env: &PythonEnv,
    project: &Path,
    mutant: &Mutant,
) -> Result<Option<Defect>, PackError> {
    let one = Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        language: Language::Python,
        base: Base { revision: None },
        mutants: vec![mutant.clone()],
        selection: None,
    };
    let directory = tempfile::tempdir().map_err(|err| PackError::Unwritable {
        path: std::env::temp_dir(),
        reason: err.to_string(),
    })?;
    let path = directory.path().join("mutant.json");
    let unwritable = |reason: String| PackError::Unwritable {
        path: path.clone(),
        reason,
    };
    let document = serde_json::to_string(&one).map_err(|err| unwritable(err.to_string()))?;
    fs::write(&path, document).map_err(|err| unwritable(err.to_string()))?;
    match pack::validate_deep(env, &path, project) {
        Ok(()) => Ok(None),
        Err(PackError::Reported { code, message, .. }) => Ok(Some(Defect {
            defect: code,
            detail: message,
        })),
        Err(other) => Err(other),
    }
}

/// The test files, read and held to the same rules the file to mutate is.
///
/// The same rules on purpose, though nothing mutates a test file: what these are
/// for is being shown to a model, and a path that leads out of the project is a
/// path that would show it somebody else's code.
///
/// # Errors
///
/// Returns [`GenerationFailure`] for a path that is not a file of this project, or a file
/// that cannot be read.
pub fn covering_tests(
    project_root: &Path,
    tests: &[String],
) -> Result<Vec<CoveringTest>, GenerationFailure> {
    let mut covering = Vec::with_capacity(tests.len());
    for file in tests {
        let bytes = read_target_file(project_root, file)?;
        covering.push(CoveringTest {
            file: file.clone(),
            source: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    Ok(covering)
}
