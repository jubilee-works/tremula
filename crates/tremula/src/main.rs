//! Command-line entry point for tremula. This layer parses arguments and hands
//! them to the orchestration; the decisions themselves live in the library.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tremula::{
    dismiss::{self, DismissArgs},
    generate::command::{self, GenerateArgs},
    orchestrate::{self, RestoreArgs, RunArgs, ValidateArgs},
    triage::{self, TriageArgs},
};

#[derive(Parser)]
#[command(name = "tremula", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ask a model for mutants of named functions and write a manifest.
    Generate(GenerateArgs),
    /// Run mutants from a manifest against the project's test suite.
    Run(RunArgs),
    /// Sort the survivors a run left behind, for a person to decide about.
    Triage(TriageArgs),
    /// Record that a survivor is not worth being shown again.
    Dismiss(DismissArgs),
    /// Validate a mutant manifest without running anything.
    Validate(ValidateArgs),
    /// Restore target files from a run's snapshot.
    Restore(RestoreArgs),
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Generate(args) => command::generate(&args),
        Command::Run(args) => orchestrate::run(&args),
        Command::Triage(args) => triage::triage(&args),
        Command::Dismiss(args) => dismiss::dismiss(&args),
        Command::Validate(args) => orchestrate::validate(&args),
        Command::Restore(args) => orchestrate::restore(&args),
    }
}
