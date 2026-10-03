use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ai", about = "All-in-one AI tool.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Permanent memory for AI agents.")]
    Memory(ai::memory::Cli),
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Memory(cli) => cli.exec(),
    };
    result.unwrap_or_else(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })
}
