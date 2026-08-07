//! Command-line entry point for tremula. This layer parses arguments and turns
//! the core's typed failures into messages a reader can act on; the decisions
//! themselves live in the library.

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Args, Parser, Subcommand};
use tremula::validation::{ValidationWarning, validate_manifest};
use tremula_contracts::manifest::Manifest;

/// Every failure that stops the work reports the same code; the message says
/// which failure it was.
const EXIT_FAILURE: u8 = 2;

#[derive(Parser)]
#[command(name = "tremula", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run mutants from a manifest against the project's test suite.
    Run,
    /// Validate a mutant manifest without running anything.
    Validate(ValidateArgs),
    /// Restore target files from a run's snapshot.
    Restore,
}

#[derive(Args)]
struct ValidateArgs {
    /// Manifest to validate.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,
    /// Project root the manifest's paths are relative to.
    #[arg(long, value_name = "DIR", default_value = ".")]
    project: PathBuf,
    /// Also run the language pack's checks, such as whether each replacement
    /// parses.
    #[arg(long)]
    deep: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate(args) => validate(&args),
        Command::Run => not_implemented("run"),
        Command::Restore => not_implemented("restore"),
    }
}

fn not_implemented(name: &str) -> ExitCode {
    eprintln!("error: `tremula {name}` is not implemented yet");
    ExitCode::from(EXIT_FAILURE)
}

/// Check a manifest against the project's files. Warnings go to stderr and do
/// not change the outcome; a defect does.
fn validate(args: &ValidateArgs) -> ExitCode {
    if args.deep {
        eprintln!(
            "error: `--deep` requires the Python pack; not available yet — run `tremula validate` without it for the language-neutral checks"
        );
        return ExitCode::from(EXIT_FAILURE);
    }
    let manifest = match read_manifest(&args.manifest) {
        Ok(manifest) => manifest,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    match validate_manifest(&manifest, &args.project) {
        Ok(warnings) => {
            for warning in &warnings {
                eprintln!("warning: {}", describe(warning));
            }
            println!("manifest is valid: {} mutant(s)", manifest.mutants.len());
            ExitCode::SUCCESS
        }
        Err(defect) => {
            eprintln!("error: {defect}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// The manifest's Rust type is the schema, so deserializing it is the schema
/// check: anything serde accepts is a well-formed manifest.
fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(path).map_err(|err| {
        format!(
            "cannot read manifest `{}`: {err}; check the path and try again",
            path.display()
        )
    })?;
    serde_json::from_str(&text).map_err(|err| {
        format!(
            "`{}` is not a valid manifest: {err}; compare it against the manifest schema",
            path.display()
        )
    })
}

/// Turn a warning into the sentence a reader sees. Warnings are advice, so each
/// one says what it noticed and why that might matter.
fn describe(warning: &ValidationWarning) -> String {
    match warning {
        ValidationWarning::LargeReplacement { mutant_id, bytes } => format!(
            "mutant {mutant_id}: replacement is {bytes} bytes; large replacements are carried through the run twice and will bloat the run directory"
        ),
    }
}
