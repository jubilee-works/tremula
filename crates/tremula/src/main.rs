use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
    Validate,
    /// Restore target files from a run's snapshot.
    Restore,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let name = match cli.command {
        Command::Run => "run",
        Command::Validate => "validate",
        Command::Restore => "restore",
    };
    eprintln!("error: `tremula {name}` is not implemented yet");
    ExitCode::from(2)
}
