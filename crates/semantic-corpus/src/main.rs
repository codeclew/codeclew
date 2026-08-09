use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use semantic_corpus::{BuildSystem, GenerateOptions, TaskFamily, generate};

#[derive(Debug, Parser)]
#[command(name = "semantic-corpus")]
#[command(about = "Generate deterministic neutral Kotlin semantic-change tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate an agent-visible project and a separate controller oracle.
    Generate {
        #[arg(long)]
        seed: u64,
        #[arg(long, value_enum)]
        family: TaskFamily,
        #[arg(long, value_enum)]
        build_system: BuildSystem,
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Generate {
            seed,
            family,
            build_system,
            output,
        } => {
            let generated = generate(&GenerateOptions {
                seed,
                family,
                build_system,
                output,
            })?;
            println!("agent package: {}", generated.agent_dir.display());
            println!("controller package: {}", generated.controller_dir.display());
        }
    }
    Ok(())
}
