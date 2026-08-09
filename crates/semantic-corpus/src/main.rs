use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use semantic_corpus::{
    BuildSystem, GenerateOptions, TaskFamily, e04, generate, verify_hidden_package,
};

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
    /// Verify a public task package against its controller-owned hidden manifest.
    VerifyHidden {
        #[arg(long)]
        agent_dir: PathBuf,
        #[arg(long)]
        controller_dir: PathBuf,
    },
    /// Materialize the 42 post-freeze E04 agent/controller packages.
    MaterializeE04 {
        #[arg(long)]
        experiment_root: PathBuf,
        #[arg(long, default_value = "a6ae1e48359eccef15060c1bb249a648857f30c9")]
        binder_freeze: String,
        #[arg(long)]
        binder_tree_sha256: String,
        #[arg(
            long,
            default_value = "a209f115b0a175bb74859b0539f75932cd664a495332ccf10b634b3cf1c2b9f2"
        )]
        population_sha256: String,
        #[arg(long)]
        tooling_root: Option<PathBuf>,
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
        Command::VerifyHidden {
            agent_dir,
            controller_dir,
        } => {
            verify_hidden_package(&agent_dir, &controller_dir)?;
            println!("hidden package verification: ok");
        }
        Command::MaterializeE04 {
            experiment_root,
            binder_freeze,
            binder_tree_sha256,
            population_sha256,
            tooling_root,
        } => {
            let result = e04::materialize(&e04::MaterializeOptions {
                experiment_root,
                population_json: include_str!(
                    "../../../benchmarks/semantic-change/editing-population-v1.json"
                )
                .into(),
                binder_freeze,
                binder_tree_sha256,
                population_sha256,
                tooling_root,
            })?;
            println!("materialized {} E04 tasks", result.tasks);
            println!("agent packages: {}", result.agent_root.display());
            println!("controller packages: {}", result.controller_root.display());
        }
    }
    Ok(())
}
