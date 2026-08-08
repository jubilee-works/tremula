//! Command-line entry point for tremula. This layer parses arguments and hands
//! them to the orchestration; the decisions themselves live in the library.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tremula::orchestrate::{self, RestoreArgs, RunArgs, ValidateArgs};

#[derive(Parser)]
#[command(name = "tremula", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run mutants from a manifest against the project's test suite.
    Run(RunArgs),
    /// Validate a mutant manifest without running anything.
    Validate(ValidateArgs),
    /// Restore target files from a run's snapshot.
    Restore(RestoreArgs),
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Run(args) => orchestrate::run(&args),
        Command::Validate(args) => orchestrate::validate(&args),
        Command::Restore(args) => orchestrate::restore(&args),
    }
}
