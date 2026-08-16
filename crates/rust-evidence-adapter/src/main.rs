use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use rust_evidence_adapter::adapter::{CollectConfig, PinnedExecutable, collect};
use rust_evidence_adapter::protocol::canonical_json;

#[derive(Debug, Parser)]
#[command(name = "codeclew-rust-evidence-adapter")]
#[command(about = "Pinned Rust semantic evidence adapter for Codeclew")]
struct Args {
    /// Absolute path to the Rust repository.
    #[arg(long, alias = "workspace")]
    repo: PathBuf,

    /// Absolute path to the rust-analyzer executable.
    #[arg(long)]
    rust_analyzer: PathBuf,

    /// Expected SHA-256 of the rust-analyzer executable (raw hex or sha256:<hex>).
    #[arg(long)]
    rust_analyzer_sha256: String,

    /// Absolute path to the cargo executable.
    #[arg(long)]
    cargo: PathBuf,

    /// Expected SHA-256 of cargo.
    #[arg(long)]
    cargo_sha256: String,

    /// Absolute path to the rustc executable.
    #[arg(long)]
    rustc: PathBuf,

    /// Expected SHA-256 of rustc.
    #[arg(long)]
    rustc_sha256: String,

    /// Absolute path to git. Git provenance is captured even outside a Git repository.
    #[arg(long)]
    git: PathBuf,

    /// Expected SHA-256 of git.
    #[arg(long)]
    git_sha256: String,

    /// Optional exact opaque entity id or native SCIP identity for the bundled impact query.
    #[arg(long)]
    seed_entity: Option<String>,

    #[arg(long, default_value_t = 2)]
    max_depth: u32,

    #[arg(long, default_value_t = 200)]
    max_entities: u32,

    /// Acknowledge that Cargo build scripts and procedural macros are trusted code execution.
    #[arg(long)]
    allow_trusted_workspace_code_execution: bool,

    /// Optional destination for canonical adapter-output JSON. Defaults to stdout.
    #[arg(long)]
    output: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let output = collect(&CollectConfig {
        repo: args.repo,
        rust_analyzer: PinnedExecutable {
            path: args.rust_analyzer,
            expected_sha256: args.rust_analyzer_sha256,
        },
        cargo: PinnedExecutable {
            path: args.cargo,
            expected_sha256: args.cargo_sha256,
        },
        rustc: PinnedExecutable {
            path: args.rustc,
            expected_sha256: args.rustc_sha256,
        },
        git: PinnedExecutable {
            path: args.git,
            expected_sha256: args.git_sha256,
        },
        seed_entity: args.seed_entity,
        max_depth: args.max_depth,
        max_entities: args.max_entities,
        allow_trusted_workspace_code_execution: args.allow_trusted_workspace_code_execution,
    })?;
    let bytes = canonical_json(&output)?;
    if let Some(path) = args.output {
        fs::write(&path, bytes)
            .with_context(|| format!("write adapter output {}", path.display()))?;
    } else {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        lock.write_all(&bytes)?;
        lock.write_all(b"\n")?;
    }
    Ok(())
}
