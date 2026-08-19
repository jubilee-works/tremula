//! The whole of a generation, in the order the steps have to happen.
//!
//! Most of that order is forced. The file is read and held to every rule a target
//! file keeps before a model is paid to look at it. The language pack is asked
//! where a mutation may land, and its report is checked against the bytes that
//! were read, because a report about some other version of the file would place
//! every span somewhere it is not. Only then is anything asked of a model.
//!
//! Nothing here claims the project's lock, and that is a decision rather than an
//! omission. A generation reads; it never patches a source. What a concurrent run
//! could do is mutate a file while this is reading it, and every way that could go
//! wrong is caught: the file's hash is read again before the manifest is written,
//! and a manifest that got past that would still be refused by the run that tried
//! to use it, because a mutant's identifier is derived from the hash of the file it
//! was generated against. Both failures are safe ones — nothing is written, or
//! nothing is applied — and taking the lock would instead make a generation and a
//! run of two different projects' worth of work exclude each other.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};

use time::OffsetDateTime;
use tremula_contracts::{
    SCHEMA_VERSION,
    manifest::{Base, Language, Manifest, Mutant},
};

use crate::{
    console,
    generate::failures::GenerationFailure,
    generate::{
        CoveringTest, Defect, GenerationRequest, MutantGenerator, Usage,
        choose::choose,
        enrich::{SourceFile, Target},
        openai::OpenAiGenerator,
        round::{Gathered, Round},
    },
    orchestrate::EXIT_FAILURE,
    pack::{self, PackError},
    provenance,
    python_env::{self, PythonEnv},
    suppressions::{DEFAULT_SUPPRESSIONS, Dismissals, warn_about_stale},
    validation::{read_target_file, validate_manifest},
};

/// How many mutants to ask for per function when the caller does not say.
///
/// Three to five is the range that was measured. Four is the middle of it.
pub const DEFAULT_COUNT: usize = 4;

/// Where a manifest goes when the caller does not say.
pub const DEFAULT_MANIFEST: &str = "tremula-manifest.json";

/// What `tremula generate` was asked to do.
#[derive(Debug, clap::Args)]
pub struct GenerateArgs {
    /// Source file to mutate, POSIX-style and relative to the project root.
    #[arg(long, value_name = "PATH")]
    pub file: String,
    /// Function to mutate, by the name the language pack reports for it. Repeat
    /// for more than one. Add `@START:END` — the function's own byte span — when
    /// the file spells that name more than once.
    #[arg(long = "function", required = true, value_name = "NAME[@START:END]")]
    pub functions: Vec<String>,
    /// Test file covering those functions, POSIX-style and relative to the project
    /// root; repeat for more than one. Unlike `run --tests`, which passes whatever
    /// it is given to the project's test runner, these are files to read and show
    /// a model. Whether they really cover the functions is not checked.
    #[arg(long, value_name = "PATH")]
    pub tests: Vec<String>,
    /// Model snapshot to ask, spelled the way its provider names that exact
    /// version. A family name resolves to whichever version is current, which is
    /// the one thing a reproduction cannot rely on.
    #[arg(long, value_name = "MODEL")]
    pub model: String,
    /// Python interpreter of the project's environment. Discovered if omitted.
    #[arg(long, value_name = "PATH")]
    pub python: Option<PathBuf>,
    /// Project root the file and test paths are relative to.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,
    /// How many mutants to ask for per function.
    #[arg(long, value_name = "COUNT", default_value_t = DEFAULT_COUNT, value_parser = at_least_one)]
    pub count: usize,
    /// Where to write the manifest. Defaults to `tremula-manifest.json` in the
    /// project, and an existing file is never overwritten.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,
    /// Where the project's dismissals are kept. Defaults to
    /// `tremula-suppressions.json` in the project.
    #[arg(long, value_name = "PATH")]
    pub suppressions: Option<PathBuf>,
}

/// A count, refused unless there is at least one mutant in it.
///
/// Zero would be obeyed exactly: every function asked about, every answer empty,
/// and a manifest with nothing in it that reads like a model with nothing to say.
fn at_least_one(spelling: &str) -> Result<usize, String> {
    match spelling.parse::<usize>() {
        Ok(count) if count >= 1 => Ok(count),
        _ => Err(format!(
            "a count has to be a whole number of mutants above zero, not `{spelling}`"
        )),
    }
}

/// What a generation did about one function.
#[derive(Debug)]
pub struct FunctionOutcome {
    /// The function, as the language pack names it.
    pub function: String,
    /// Everything the round produced, including why it stopped if it did.
    pub gathered: Gathered,
}

/// Everything a generation produced.
#[derive(Debug)]
pub struct Generated {
    /// The file the functions live in, as the caller spelled it.
    pub file: String,
    /// The model that was asked, as the caller named it.
    pub model: String,
    /// Each function that was asked about, in the order the caller named them.
    pub functions: Vec<FunctionOutcome>,
    /// Where the manifest was written. Absent when there was nothing to write.
    pub manifest: Option<PathBuf>,
    /// How many mutants it carries.
    pub recorded: usize,
    /// What every call cost, added up.
    pub tokens: Usage,
}

/// Ask a model for mutants of the functions `args` names, and write a manifest.
#[must_use]
pub fn generate(args: &GenerateArgs) -> ExitCode {
    generate_with(args, &OpenAiGenerator::new(&args.model))
}

/// The same, from a generator the caller has already built.
///
/// The seam a test replays a recorded answer through, and the seam a second
/// provider would arrive at.
#[must_use]
pub fn generate_with(args: &GenerateArgs, generator: &dyn MutantGenerator) -> ExitCode {
    match compose(args, generator) {
        Ok(generated) => {
            println!("{}", console::render_generation(&generated));
            ExitCode::from(exit_code(&generated))
        }
        Err(failure) => {
            eprintln!("error: {failure}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// What a generation exits with.
///
/// A refused proposal is the ordinary course of asking a model for mutants and
/// does not fail a generation; a manifest with nothing in it does, because there
/// was nothing to generate for and a reader who did not look would take a written
/// manifest as one worth running. A function whose generator or whose check gave
/// out fails it too, and the manifest is still written: what the other functions
/// produced is real, and the exit code and the summary together are what say the
/// output is partial.
#[must_use]
pub fn exit_code(generated: &Generated) -> u8 {
    let unfinished = generated
        .functions
        .iter()
        .any(|outcome| outcome.gathered.failure.is_some());
    if unfinished || generated.recorded == 0 {
        return EXIT_FAILURE;
    }
    0
}

/// The generation itself. Each step is either the reason the next one is safe, or
/// the reason it is meaningful.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when the file, the project's environment, the
/// language pack, the named functions, or the manifest that came out of it all
/// make going on impossible. A model that refused to answer about one function is
/// not one of those: it is recorded against that function and the rest go on.
pub fn compose(
    args: &GenerateArgs,
    generator: &dyn MutantGenerator,
) -> Result<Generated, GenerationFailure> {
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| args.project.join(DEFAULT_MANIFEST));
    // Asked here so that a taken path costs nobody a model call. It is not the
    // promise, though: the promise is kept at the commit, which refuses the same
    // way for a file that appears while this is running.
    if out.exists() {
        return Err(GenerationFailure::ManifestExists { path: out });
    }
    // Read here, before a model is asked anything, for two reasons: a candidate a
    // person has already dismissed is one this generation must not record, and a
    // record of dismissals that cannot be read is a failure worth having before the
    // first call rather than after the last one.
    let dismissals = Dismissals::load(
        &args
            .suppressions
            .clone()
            .unwrap_or_else(|| args.project.join(DEFAULT_SUPPRESSIONS)),
    )?;
    let file = SourceFile::new(
        args.file.clone(),
        read_target_file(&args.project, &args.file)?,
    );
    let env = python_env::discover(args.python.as_deref(), &args.project)?;
    env.verify_pack()?;
    pack::handshake_for_generation(&env)?;
    let project = absolute(&args.project)?;
    let report = pack::spans(&env, &project, &args.file)?;
    // Which file the report is about, before whether it is about the same bytes:
    // two files with the same contents hash the same, so the hash below cannot tell
    // one from the other, and every span in the answer is an offset into whichever
    // one it names.
    if report.file != args.file {
        return Err(GenerationFailure::ReportIsAboutAnotherFile {
            asked: args.file.clone(),
            answered: report.file.clone(),
        });
    }
    if report.file_sha256 != file.sha256 {
        return Err(GenerationFailure::ReportIsAboutOtherBytes {
            file: args.file.clone(),
        });
    }
    warn_about_stale(
        &dismissals,
        &args.file,
        &String::from_utf8_lossy(&file.bytes),
    );
    let chosen = choose(&report, &args.functions)?;
    let tests = covering_tests(&args.project, &args.tests)?;
    let mut functions = Vec::new();
    for function in &chosen {
        let target = Target::new(function, &report.functions);
        let request = ask_about(&file, &target, &tests, args.count)?;
        let mut gathered = Round {
            generator,
            file: &file,
            target: &target,
            inspect: &|mutant| check_with_the_pack(&env, &project, mutant),
        }
        .run(&request);
        // After the round has finished asking and before anything about it is
        // counted, so that the manifest, the console's tally, and the exit code all
        // describe the same set of mutants.
        set_aside_what_was_dismissed(&mut gathered, &dismissals);
        functions.push(FunctionOutcome {
            function: function.qualified_name.clone(),
            gathered,
        });
    }
    write(args, &out, &file, functions)
}

/// Hold the manifest to the neutral validation and write it, or say why not.
fn write(
    args: &GenerateArgs,
    out: &Path,
    file: &SourceFile,
    functions: Vec<FunctionOutcome>,
) -> Result<Generated, GenerationFailure> {
    // Read again rather than trusting the first read: everything below names bytes
    // by offset, and a file that moved under the generation makes every one of
    // those offsets a lie.
    let now = read_target_file(&args.project, &args.file)?;
    if SourceFile::new(args.file.clone(), now).sha256 != file.sha256 {
        return Err(GenerationFailure::SourcesChanged {
            file: args.file.clone(),
        });
    }
    let mutants: Vec<Mutant> = functions
        .iter()
        .flat_map(|outcome| outcome.gathered.mutants.iter().cloned())
        .collect();
    let tokens = spent(&functions);
    let recorded = mutants.len();
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        language: Language::Python,
        base: Base {
            revision: provenance::observe(&args.project, OffsetDateTime::now_utc()).revision,
        },
        mutants,
        selection: None,
    };
    // Every mutant was checked one at a time, so this is the invariant rather than
    // the check: what a manifest says as a whole — no repeated identifier, every
    // span still where it was said to be — is not something a per-mutant answer can
    // establish, and it is what a run will hold this document to.
    validate_manifest(&manifest, &args.project)?;
    let written = if recorded == 0 {
        None
    } else {
        write_manifest(out, &manifest)?;
        Some(out.to_path_buf())
    };
    Ok(Generated {
        file: args.file.clone(),
        model: args.model.clone(),
        functions,
        manifest: written,
        recorded,
        tokens,
    })
}

/// Take out the mutants a person has already dismissed, and count them.
///
/// By the mutation and not by the identifier: an identifier is derived from the
/// file's hash and would be orphaned by the next unrelated edit, so a decision keyed
/// by one would stop applying without anybody deciding that it should.
fn set_aside_what_was_dismissed(gathered: &mut Gathered, dismissals: &Dismissals) {
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
fn ask_about(
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
fn check_with_the_pack(
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
fn covering_tests(
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
/// Two things have to hold at once, and one operation gives both. The document has
/// to appear complete or not at all, because a reader who found half of one would
/// take it for a manifest. And it may not replace a file: the existence check this
/// generation started with was minutes and one model call ago, and a rename would
/// silently write over whatever appeared in that time. So the document is written
/// under a name of its own and the manifest is linked to it — a link the filesystem
/// either makes or refuses because the name is taken, which is the same claim the
/// project lock is made with.
///
/// The staging file is removed whichever way that went. It is the only thing here
/// that could outlive a failure, and a `.part` file left in a project is something
/// its owner has to identify before they can delete it.
fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), GenerationFailure> {
    let unwritable = |reason: String| GenerationFailure::ManifestUnwritable {
        path: path.to_path_buf(),
        reason,
    };
    let mut document =
        serde_json::to_string_pretty(manifest).map_err(|err| unwritable(err.to_string()))?;
    document.push('\n');
    // A name of this process's own, so that two generations writing the same
    // manifest cannot stage over each other's document and link the mixture.
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

/// A path the pack can use from a working directory of its own.
fn absolute(path: &Path) -> Result<PathBuf, GenerationFailure> {
    std::path::absolute(path).map_err(|err| GenerationFailure::PathUnresolvable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })
}
