use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use semantic_corpus::{
    BuildSystem, GenerateOptions, TaskFamily, e04, e04_hidden_verification, generate,
    verify_hidden_package,
};

#[derive(Parser)]
#[command(name = "semantic-corpus")]
#[command(about = "Generate deterministic neutral Kotlin semantic-change tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
// This command is parsed once and immediately consumed; boxing individual CLI
// fields would complicate clap's generated interface without reducing retained
// memory.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Print the exact pinned R1 materializer identity as canonical JSON.
    E04MaterializerIdentity,
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
    /// Verify all 42 fresh R1 hidden packages under signed authority.
    VerifyE04Hidden {
        #[arg(long)]
        experiment_root: PathBuf,
        #[arg(long)]
        readiness_store: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
        #[arg(long)]
        root_receipt: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        annotation_a: PathBuf,
        #[arg(long)]
        annotation_b: PathBuf,
    },
    /// Materialize the 42 post-freeze E04 agent/controller packages.
    MaterializeE04 {
        #[arg(long)]
        experiment_root: PathBuf,
        #[arg(long)]
        readiness_store: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
        #[arg(long)]
        root_receipt: PathBuf,
        #[arg(long)]
        agent_seed: String,
        #[arg(long)]
        controller_seed: String,
        #[arg(long)]
        series_nonce: String,
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
        #[arg(long)]
        gradle_wrapper_script: Option<PathBuf>,
        #[arg(long)]
        gradle_wrapper_jar: Option<PathBuf>,
        #[arg(long)]
        gradle_wrapper_properties: Option<PathBuf>,
        #[arg(long)]
        tooling_manifest: Option<PathBuf>,
        #[arg(long)]
        codeclew_binary_sha256: Option<String>,
        #[arg(long)]
        typed_goal_catalog_sha256: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::E04MaterializerIdentity => {
            print!(
                "{}",
                e04::canonical_json(&e04::materializer_identity_report())?
            );
        }
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
        Command::VerifyE04Hidden {
            experiment_root,
            readiness_store,
            authorization,
            root_receipt,
            report,
            annotation_a,
            annotation_b,
        } => {
            let capability = e04_hidden_verification::authorize_hidden_verification(
                e04_hidden_verification::HiddenVerificationAuthorizationInput {
                    readiness_store,
                    authorization_path: authorization,
                    root_receipt_path: root_receipt,
                    experiment_path: experiment_root,
                    report_path: report,
                    annotation_a_path: annotation_a,
                    annotation_b_path: annotation_b,
                },
            )?;
            let result = e04_hidden_verification::verify_e04_hidden(capability)?;
            print!("{}", e04::canonical_json(&result)?);
        }
        Command::MaterializeE04 {
            experiment_root,
            readiness_store,
            authorization,
            root_receipt,
            agent_seed,
            controller_seed,
            series_nonce,
            binder_freeze,
            binder_tree_sha256,
            population_sha256,
            tooling_root,
            gradle_wrapper_script,
            gradle_wrapper_jar,
            gradle_wrapper_properties,
            tooling_manifest,
            codeclew_binary_sha256,
            typed_goal_catalog_sha256,
        } => {
            let gradle_wrapper_assets = match (
                tooling_root,
                gradle_wrapper_script,
                gradle_wrapper_jar,
                gradle_wrapper_properties,
                tooling_manifest,
                codeclew_binary_sha256,
                typed_goal_catalog_sha256,
            ) {
                (None, None, None, None, None, None, None) => None,
                (
                    Some(tooling_root),
                    Some(wrapper_script),
                    Some(wrapper_jar),
                    Some(wrapper_properties),
                    Some(manifest),
                    Some(codeclew_binary_sha256),
                    Some(typed_goal_catalog_sha256),
                ) => Some(e04::GradleWrapperAssets {
                    tooling_root,
                    wrapper_script,
                    wrapper_jar,
                    wrapper_properties,
                    manifest,
                    codeclew_binary_sha256,
                    typed_goal_catalog_sha256,
                }),
                _ => anyhow::bail!(
                    "Gradle tooling requires all explicit named wrapper paths, manifest, binary SHA, and catalog SHA"
                ),
            };
            let authorization =
                e04::authorize_materialization(e04::MaterializationAuthorizationInput {
                    readiness_store,
                    authorization_path: authorization,
                    root_receipt_path: root_receipt,
                    output_path: experiment_root.clone(),
                    agent_seed,
                    controller_seed,
                    series_nonce,
                })?;
            let result = e04::materialize(
                &e04::MaterializeOptions {
                    experiment_root,
                    population_json: include_str!(
                        "../../../benchmarks/semantic-change/editing-population-v1.json"
                    )
                    .into(),
                    binder_freeze,
                    binder_tree_sha256,
                    population_sha256,
                    gradle_wrapper_assets,
                },
                authorization,
            )?;
            print!("{}", e04::canonical_json(&result.result)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r1_materialization_cli_requires_authority_and_raw_series_inputs() {
        let missing = Cli::try_parse_from([
            "semantic-corpus",
            "materialize-e04",
            "--experiment-root",
            "/tmp/r1-output",
            "--binder-tree-sha256",
            "0",
        ]);
        assert!(missing.is_err());

        let parsed = Cli::try_parse_from([
            "semantic-corpus",
            "materialize-e04",
            "--experiment-root",
            "/tmp/r1-output",
            "--readiness-store",
            "/tmp/readiness",
            "--authorization",
            "/tmp/readiness/authorizations/a.json",
            "--root-receipt",
            "/tmp/readiness/objects/r.json",
            "--agent-seed",
            "agent-secret",
            "--controller-seed",
            "controller-secret",
            "--series-nonce",
            "fresh-series",
            "--binder-tree-sha256",
            "0",
        ]);
        assert!(parsed.is_ok());

        let identity = Cli::try_parse_from(["semantic-corpus", "e04-materializer-identity"]);
        assert!(matches!(
            identity.unwrap().command,
            Command::E04MaterializerIdentity
        ));
        let report = e04::materializer_identity_report();
        let canonical = e04::canonical_json(&report).unwrap();
        assert_eq!(canonical.as_bytes().last(), Some(&b'\n'));
        assert_eq!(
            serde_json::from_str::<e04::MaterializerIdentityReport>(&canonical).unwrap(),
            report
        );
    }

    #[test]
    fn r1_hidden_verification_cli_requires_signed_authority_paths() {
        assert!(
            Cli::try_parse_from([
                "semantic-corpus",
                "verify-e04-hidden",
                "--experiment-root",
                "/tmp/e04"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "semantic-corpus",
                "verify-e04-hidden",
                "--experiment-root",
                "/tmp/e04",
                "--readiness-store",
                "/tmp/readiness",
                "--authorization",
                "/tmp/readiness/authorizations/a.json",
                "--root-receipt",
                "/tmp/readiness/objects/r.json",
                "--report",
                "/tmp/report.json",
                "--annotation-a",
                "/tmp/annotation-a.json",
                "--annotation-b",
                "/tmp/annotation-b.json"
            ])
            .is_ok()
        );
    }
}
