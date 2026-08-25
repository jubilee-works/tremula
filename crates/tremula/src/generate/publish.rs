//! Holding what a generation produced to the rules a manifest keeps, and putting it
//! where a run will look for it.
//!
//! # Why an empty manifest is sometimes written and sometimes not
//!
//! A person who named a function and got no mutants asked for something specific and did
//! not get it; a manifest with nothing in it would read like one worth running, so none is
//! written. A selection that found nothing to mutate is the opposite: what it found out is
//! the document — which lines of the change no test reaches, which files the coverage
//! document has never heard of — and refusing to write that would destroy the most useful
//! thing such a run produces.

use std::{fs, io, path::Path};

use time::OffsetDateTime;
use tremula_contracts::{
    SCHEMA_VERSION,
    manifest::{Base, Language, Manifest, Mutant, Selection},
};

use crate::{
    generate::{
        Usage,
        command::{FunctionOutcome, GenerateArgs, Generated},
        enrich::SourceFile,
        failures::GenerationFailure,
        plan::Plan,
    },
    provenance,
    validation::{read_target_file, validate_manifest},
};

/// Hold the manifest to the neutral validation and write it, or say why not.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when a file moved under the generation, when the manifest
/// as a whole does not hold together, or when it cannot be written.
pub fn write(
    args: &GenerateArgs,
    out: &Path,
    plan: &Plan,
    functions: Vec<FunctionOutcome>,
) -> Result<Generated, GenerationFailure> {
    // Read again rather than trusting the first read: everything below names bytes by
    // offset, and a file that moved under the generation makes every one of those offsets
    // a lie. Once per file, because each file's mutants are measured against its own bytes.
    for planned in &plan.files {
        let now = read_target_file(&args.project, &planned.source.file)?;
        if SourceFile::new(planned.source.file.clone(), now).sha256 != planned.source.sha256 {
            return Err(GenerationFailure::SourcesChanged {
                file: planned.source.file.clone(),
            });
        }
    }
    let mutants: Vec<Mutant> = functions
        .iter()
        .flat_map(|outcome| outcome.gathered.mutants.iter().cloned())
        .collect();
    let tokens = spent(&functions);
    let recorded = mutants.len();
    let selection = plan
        .selection
        .clone()
        .map(|selection| accounted_for(selection, &functions));
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        language: Language::Python,
        base: Base {
            revision: provenance::observe(&args.project, OffsetDateTime::now_utc()).revision,
        },
        mutants,
        selection: selection.clone(),
    };
    // Every mutant was checked one at a time, so this is the invariant rather than the
    // check: what a manifest says as a whole — no repeated identifier, every span still
    // where it was said to be — is not something a per-mutant answer can establish, and it
    // is what a run will hold this document to.
    validate_manifest(&manifest, &args.project)?;
    let written = if recorded == 0 && manifest.selection.is_none() {
        None
    } else {
        write_manifest(out, &manifest)?;
        Some(out.to_path_buf())
    };
    Ok(Generated {
        files: plan
            .files
            .iter()
            .map(|planned| planned.source.file.clone())
            .collect(),
        model: args.model.clone(),
        functions,
        manifest: written,
        recorded,
        tokens,
        selection,
        aside: plan.aside,
    })
}

/// The selection with what became of each function filled in.
///
/// Both numbers, because their difference is what says a green run produced no evidence:
/// a function with proposals and nothing recorded had every proposal refused, and a
/// reader who saw only the empty manifest could not tell that from a function nobody
/// asked about.
fn accounted_for(mut selection: Selection, functions: &[FunctionOutcome]) -> Selection {
    for chosen in &mut selection.functions {
        let Some(outcome) = functions
            .iter()
            .find(|outcome| outcome.file == chosen.file && outcome.span == chosen.span)
        else {
            continue;
        };
        chosen.generation.proposed = u32::try_from(outcome.gathered.proposed).unwrap_or(u32::MAX);
        chosen.generation.recorded =
            u32::try_from(outcome.gathered.mutants.len()).unwrap_or(u32::MAX);
    }
    selection
}

/// What every call of every function cost.
fn spent(functions: &[FunctionOutcome]) -> Usage {
    let mut total = Usage::default();
    for attempt in functions
        .iter()
        .flat_map(|outcome| outcome.gathered.attempts.iter())
    {
        total.prompt += attempt.usage.prompt;
        total.completion += attempt.usage.completion;
        total.total += attempt.usage.total;
    }
    total
}

/// Put the manifest where a run will look for it, and only where nothing is.
///
/// Two things have to hold at once, and one operation gives both. The document has to
/// appear complete or not at all, because a reader who found half of one would take it
/// for a manifest. And it may not replace a file: the existence check this generation
/// started with was minutes and one model call ago, and a rename would silently write over
/// whatever appeared in that time. So the document is written under a name of its own and
/// the manifest is linked to it — a link the filesystem either makes or refuses because
/// the name is taken, which is the same claim the project lock is made with.
///
/// The staging file is removed whichever way that went. It is the only thing here that
/// could outlive a failure, and a `.part` file left in a project is something its owner
/// has to identify before they can delete it.
fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), GenerationFailure> {
    let unwritable = |reason: String| GenerationFailure::ManifestUnwritable {
        path: path.to_path_buf(),
        reason,
    };
    let mut document =
        serde_json::to_string_pretty(manifest).map_err(|err| unwritable(err.to_string()))?;
    document.push('\n');
    // A name of this process's own, so that two generations writing the same manifest
    // cannot stage over each other's document and link the mixture.
    let staging = path.with_extension(format!("json.part.{}", std::process::id()));
    if let Err(err) = fs::write(&staging, document) {
        drop(fs::remove_file(&staging));
        return Err(unwritable(err.to_string()));
    }
    let committed = fs::hard_link(&staging, path);
    drop(fs::remove_file(&staging));
    match committed {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Err(GenerationFailure::ManifestExists {
                path: path.to_path_buf(),
            })
        }
        Err(err) => Err(unwritable(err.to_string())),
    }
}
