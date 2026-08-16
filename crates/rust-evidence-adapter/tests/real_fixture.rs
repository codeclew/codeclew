use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rust_evidence_adapter::adapter::{
    CollectConfig, PinnedExecutable, collect, impact_from_output, verify_output_digest,
};
use rust_evidence_adapter::protocol::sha256_bytes;

fn executable(name: &str) -> Result<PathBuf> {
    let output = std::process::Command::new("which").arg(name).output()?;
    anyhow::ensure!(output.status.success(), "missing executable {name}");
    Ok(PathBuf::from(String::from_utf8(output.stdout)?.trim()).canonicalize()?)
}

fn pin(path: PathBuf) -> Result<PinnedExecutable> {
    let bytes = std::fs::read(&path)?;
    Ok(PinnedExecutable {
        path,
        expected_sha256: sha256_bytes(&bytes),
    })
}

fn rust_analyzer_from_environment() -> Result<Option<PinnedExecutable>> {
    let Some(path) = std::env::var_os("CODECLEW_TEST_RUST_ANALYZER") else {
        return Ok(None);
    };
    let path = PathBuf::from(path).canonicalize()?;
    let expected_sha256 = std::env::var("CODECLEW_TEST_RUST_ANALYZER_SHA256")
        .context("CODECLEW_TEST_RUST_ANALYZER_SHA256 is required with the executable")?;
    Ok(Some(PinnedExecutable {
        path,
        expected_sha256,
    }))
}

#[test]
fn real_rust_analyzer_fixture_emits_resolved_partial_evidence() -> Result<()> {
    let Some(rust_analyzer) = rust_analyzer_from_environment()? else {
        eprintln!("skipped: set CODECLEW_TEST_RUST_ANALYZER and its SHA-256");
        return Ok(());
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/rust-evidence-basic")
        .canonicalize()?;
    let output = collect(&CollectConfig {
        repo: fixture,
        rust_analyzer,
        cargo: pin(executable("cargo")?)?,
        rustc: pin(executable("rustc")?)?,
        git: pin(executable("git")?)?,
        seed_entity: None,
        max_depth: 2,
        max_entities: 100,
        allow_trusted_workspace_code_execution: true,
    })?;

    verify_output_digest(&output)?;
    assert_eq!(output.compiler_receipt.status, "ACCEPTED");
    assert!(output.entities.iter().any(|entity| {
        entity.display_name.as_deref() == Some("normalize")
            || entity.native_identity.contains("normalize")
    }));
    assert!(output.occurrences.iter().any(|occurrence| {
        occurrence.native_identity.contains("normalize") && occurrence.role == "REFERENCE"
    }));
    assert!(
        output
            .facts
            .iter()
            .all(|fact| { fact.grade == "COMPILER_RESOLVED" && fact.enumeration == "PARTIAL" })
    );
    assert!(
        output
            .boundaries
            .iter()
            .any(|boundary| boundary.kind_uri.ends_with(":dynamic-dispatch"))
    );
    assert_eq!(output.impact.status, "UNKNOWN");
    assert_eq!(output.impact.reason, "NO_SEED_ENTITY");
    let normalize = output
        .entities
        .iter()
        .find(|entity| entity.native_identity.contains("normalize()."))
        .context("missing normalize entity")?;
    let impact = impact_from_output(&output, Some(&normalize.opaque_id), 2, 100)?;
    assert_eq!(impact.status, "PARTIAL_BOUNDARY");
    assert!(
        impact.affected.iter().any(|affected| affected
            .documents
            .iter()
            .any(|path| path == "src/consumer.rs")),
        "facts={:#?} impact={:#?}",
        output.facts,
        impact
    );
    Ok(())
}

#[test]
fn wrong_provider_digest_fails_closed_before_execution() -> Result<()> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/rust-evidence-basic")
        .canonicalize()?;
    let cargo = executable("cargo")?;
    let error = collect(&CollectConfig {
        repo: fixture,
        rust_analyzer: PinnedExecutable {
            path: cargo.clone(),
            expected_sha256: "0".repeat(64),
        },
        cargo: pin(cargo)?,
        rustc: pin(executable("rustc")?)?,
        git: pin(executable("git")?)?,
        seed_entity: None,
        max_depth: 1,
        max_entities: 10,
        allow_trusted_workspace_code_execution: true,
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("MISSING_PROVIDER:rust-analyzer:digest mismatch")
    );
    Ok(())
}
