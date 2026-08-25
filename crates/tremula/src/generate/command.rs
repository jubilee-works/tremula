//! The whole of a generation, in the order the steps have to happen.
//!
//! Most of that order is forced. Every file is read and held to every rule a target file
//! keeps before a model is paid to look at it. The language pack is asked where a mutation
//! may land, and its report is checked against the bytes that were read, because a report
//! about some other version of the file would place every span somewhere it is not. Only
//! then is anything asked of a model.
//!
//! Nothing here claims the project's lock, and that is a decision rather than an omission.
//! A generation reads; it never patches a source. What a concurrent run could do is mutate
//! a file while this is reading it, and every way that could go wrong is caught: each
//! file's hash is read again before the manifest is written, and a manifest that got past
//! that would still be refused by the run that tried to use it, because a mutant's
//! identifier is derived from the hash of the file it was generated against. Both failures
//! are safe ones — nothing is written, or nothing is applied — and taking the lock would
//! instead make a generation and a run of two different projects' worth of work exclude
//! each other.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::ArgGroup;
use tremula_contracts::manifest::{Selection, Span};

use crate::{
    console,
    generate::failures::GenerationFailure,
    generate::{
        MutantGenerator, Usage,
        asking::{
            ask_about, check_with_the_pack, covering_tests, landed_where_nothing_runs,
            set_aside_what_was_dismissed, shown_for,
        },
        choose::choose,
        enrich::Target,
        openai::OpenAiGenerator,
        plan::{Aside, plan},
        publish::write,
        round::{Gathered, Round, RoundFailure},
        selection::lines::Lines,
    },
    orchestrate::EXIT_FAILURE,
    pack, python_env,
    suppressions::{DEFAULT_SUPPRESSIONS, Dismissals},
    validation::read_target_file,
};

/// How many mutants to ask for per function when the caller does not say.
///
/// Three to five is the range that was measured. Four is the middle of it.
pub const DEFAULT_COUNT: usize = 4;

/// How many functions one selection asks about when the caller does not say.
///
/// Five, which is what a pull request's worth of mutation was costed against: a handful of
/// mutants for each of them, and a run of the whole suite for every one of those.
pub const DEFAULT_MAX_FUNCTIONS: usize = 5;

/// Where a manifest goes when the caller does not say.
pub const DEFAULT_MANIFEST: &str = "tremula-manifest.json";

/// What `tremula generate` was asked to do.
///
/// The two ways of saying what to mutate exclude each other, and one of them is required.
/// A person names a file and its functions; a selection names a revision to compare
/// against and works the rest out for itself. There is no default and no combination of
/// the two: a generation that guessed what to mutate would be inventing a policy nobody
/// asked for, and one that took both would be following two.
#[derive(Debug, clap::Args)]
#[command(group(ArgGroup::new("targets").required(true).args(["file", "diff_base"])))]
pub struct GenerateArgs {
    /// Source file to mutate, POSIX-style and relative to the project root.
    #[arg(long, value_name = "PATH", requires = "functions")]
    pub file: Option<String>,
    /// Function to mutate, by the name the language pack reports for it. Repeat
    /// for more than one. Add `@START:END` — the function's own byte span — when
    /// the file spells that name more than once.
    #[arg(long = "function", value_name = "NAME[@START:END]", requires = "file")]
    pub functions: Vec<String>,
    /// Revision to compare this one against, so that the functions this change touched
    /// are the ones mutated. The comparison runs from where the two parted rather than
    /// from the revision itself, so a base that has moved on since does not put its own
    /// commits into this change.
    #[arg(long, value_name = "REF")]
    pub diff_base: Option<String>,
    /// An LCOV document from the project's own test run, which holds the selection to
    /// changed lines a test really reaches. Without it every changed function is a
    /// target, and a mutant that survives may have survived because nothing runs it.
    #[arg(long, value_name = "PATH", requires = "diff_base")]
    pub coverage: Option<PathBuf>,
    /// How many functions one selection asks about at most. What the limit leaves out is
    /// recorded rather than dropped in silence.
    #[arg(long, value_name = "COUNT", default_value_t = DEFAULT_MAX_FUNCTIONS, value_parser = at_least_one)]
    pub max_functions: usize,
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

/// A count, refused unless there is at least one of whatever is being counted.
///
/// Zero mutants would be obeyed exactly: every function asked about, every answer empty,
/// and a manifest with nothing in it that reads like a model with nothing to say. Zero
/// functions would work out a change's worth of targets and then ask about none of them.
fn at_least_one(spelling: &str) -> Result<usize, String> {
    match spelling.parse::<usize>() {
        Ok(count) if count >= 1 => Ok(count),
        _ => Err(format!(
            "a count has to be a whole number above zero, not `{spelling}`"
        )),
    }
}

/// What a generation did about one function.
#[derive(Debug)]
pub struct FunctionOutcome {
    /// The file it is in, as the file was spelled.
    pub file: String,
    /// The function, as the language pack names it.
    pub function: String,
    /// Where the function is, which is what identifies it when a name is not unique.
    pub span: Span,
    /// Everything the round produced, including why it stopped if it did.
    pub gathered: Gathered,
}

/// Everything a generation produced.
#[derive(Debug)]
pub struct Generated {
    /// The files the functions live in, as they were spelled, once each.
    pub files: Vec<String>,
    /// The model that was asked, as the caller named it.
    pub model: String,
    /// Each function that was asked about, in the order it was asked about.
    pub functions: Vec<FunctionOutcome>,
    /// Where the manifest was written. Absent when there was nothing to write.
    pub manifest: Option<PathBuf>,
    /// How many mutants it carries.
    pub recorded: usize,
    /// What every call cost, added up.
    pub tokens: Usage,
    /// The record of how the targets were chosen, when they were chosen here.
    pub selection: Option<Selection>,
    /// What the selection left aside, for the console to say.
    pub aside: Option<Aside>,
}

/// Ask a model for mutants and write a manifest.
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
/// A refused proposal is the ordinary course of asking a model for mutants and does not
/// fail a generation. What an empty manifest means depends on who chose the targets, and
/// the two answers are not a matter of taste:
///
/// * A person named the functions. They asked for something specific and did not get it,
///   so a manifest with nothing in it is a failure, as is a function whose generator or
///   whose check gave out. The manifest is still written when anything survived: what the
///   other functions produced is real, and the exit code and the summary together are what
///   say the output is partial.
/// * A selection chose the targets. Nothing to mutate is a finding and exits zero; every
///   proposal being refused is an ordinary bad day for a model and exits zero, loudly. The
///   one failure is every selected function dying of something that was never about the
///   code — a key the provider will not take, a network that is not there, a language pack
///   that will not run. That has to fail, or a broken credential becomes a green build
///   that reports nothing, forever.
#[must_use]
pub fn exit_code(generated: &Generated) -> u8 {
    if generated.selection.is_some() {
        return selected_exit(generated);
    }
    let unfinished = generated
        .functions
        .iter()
        .any(|outcome| outcome.gathered.failure.is_some());
    if unfinished || generated.recorded == 0 {
        return EXIT_FAILURE;
    }
    0
}

/// What a generation that chose its own targets exits with.
fn selected_exit(generated: &Generated) -> u8 {
    if generated.functions.is_empty() {
        return 0;
    }
    let every_one_died = generated.functions.iter().all(|outcome| {
        outcome
            .gathered
            .failure
            .as_ref()
            .is_some_and(never_the_code)
    });
    if every_one_died {
        return EXIT_FAILURE;
    }
    0
}

/// Whether a round stopped for a reason that was never about the code being mutated.
///
/// The line is drawn at whether a model ever got to answer. A provider that would not take
/// the key, could not be reached, is rate limiting, or answered with something that was not
/// its own protocol has said nothing about anybody's tests. A model that refused, ran out of
/// room, or answered badly has: it read the question and did that, and another function or
/// another day may go differently.
#[must_use]
pub fn never_the_code(failure: &RoundFailure) -> bool {
    use crate::generate::GenerateFailure::{
        CredentialRejected, MissingCredential, RateLimited, Rejected, Unreachable,
        UnreadableEnvelope,
    };
    match failure {
        RoundFailure::Pack(_) => true,
        RoundFailure::Generator(error) => matches!(
            error.kind,
            MissingCredential { .. }
                | CredentialRejected { .. }
                | Unreachable { .. }
                | RateLimited { .. }
                | Rejected { .. }
                | UnreadableEnvelope { .. }
        ),
    }
}

/// The generation itself. Each step is either the reason the next one is safe, or the
/// reason it is meaningful.
///
/// # Errors
///
/// Returns [`GenerationFailure`] when the files, the project's environment, the language
/// pack, the named functions, or the manifest that came out of it all make going on
/// impossible. A model that refused to answer about one function is not one of those: it is
/// recorded against that function and the rest go on.
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
    // A file somebody named is known before anything else is, and is held to being one of
    // this project's files before an interpreter is looked for: a project with no
    // environment yet would otherwise be told about its environment when what is wrong is
    // the path. A selection has no file to check here — working out which files a change
    // touched is what the pack is needed for.
    if let Some(named) = &args.file {
        read_target_file(&args.project, named)?;
    }
    let env = python_env::discover(args.python.as_deref(), &args.project)?;
    env.verify_pack()?;
    pack::handshake_for_generation(&env)?;
    let project = absolute(&args.project)?;
    let settled = plan(args, &env, &project, &dismissals)?;
    let named_tests = covering_tests(&args.project, &args.tests)?;
    let mut functions = Vec::new();
    for planned in &settled.files {
        let spellings: Vec<String> = planned
            .functions
            .iter()
            .map(|asked| asked.spelling.clone())
            .collect();
        let chosen = choose(&planned.report, &spellings)?;
        let lines = Lines::of(&planned.source.bytes);
        for (function, asked) in chosen.iter().zip(planned.functions.iter()) {
            let target = Target::new(function, &planned.report.functions);
            let request = ask_about(
                &planned.source,
                &target,
                &shown_for(args, &named_tests, asked)?,
                args.count,
            )?;
            let mut gathered = Round {
                generator,
                file: &planned.source,
                target: &target,
                inspect: &|mutant| {
                    if let Some(refusal) =
                        landed_where_nothing_runs(settled.coverage.as_ref(), &lines, mutant)
                    {
                        return Ok(Some(refusal));
                    }
                    check_with_the_pack(&env, &project, mutant)
                },
            }
            .run(&request);
            // After the round has finished asking and before anything about it is
            // counted, so that the manifest, the console's tally, and the exit code all
            // describe the same set of mutants.
            set_aside_what_was_dismissed(&mut gathered, &dismissals);
            functions.push(FunctionOutcome {
                file: planned.source.file.clone(),
                function: function.qualified_name.clone(),
                span: function.span,
                gathered,
            });
        }
    }
    write(args, &out, &settled, functions)
}

/// A path the pack can use from a working directory of its own.
fn absolute(path: &Path) -> Result<PathBuf, GenerationFailure> {
    std::path::absolute(path).map_err(|err| GenerationFailure::PathUnresolvable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })
}
