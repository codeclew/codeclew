use clew::error::ErrorCode;
use clew::evidence_authority::{EvidenceAuthority, ProducerTransformConsumerGoal};
use clew::model::{Edge, ThreadIr};
use clew::worker::{WorkerClient, workspace_root};
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn committed_fixture() -> (tempfile::TempDir, PathBuf) {
    let workspace = workspace_root();
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path().join("checkout");
    let clone = Command::new("git")
        .args(["clone", "--quiet", "--no-hardlinks"])
        .arg(&workspace)
        .arg(&checkout)
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "{}",
        String::from_utf8_lossy(&clone.stderr)
    );
    for relative in [
        "fixtures/kotlin-2-1/build.gradle.kts",
        "fixtures/kotlin-2-1/src/main/kotlin/com/acme/Runner.kt",
        "fixtures/kotlin-2-1/src/test/kotlin/com/acme/RunnerTest.kt",
    ] {
        std::fs::copy(workspace.join(relative), checkout.join(relative)).unwrap();
    }
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@example.invalid",
            "add",
            "fixtures/kotlin-2-1",
        ])
        .current_dir(&checkout)
        .status()
        .unwrap();
    assert!(commit.success());
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "authority fixture",
        ])
        .current_dir(&checkout)
        .status()
        .unwrap();
    assert!(commit.success());
    let fixture = checkout.join("fixtures/kotlin-2-1");
    (temporary, fixture)
}

fn git_head(repo: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn live_thread(repo: &Path, symbol: &str) -> ThreadIr {
    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "slice",
            "--repo",
            repo.to_str().unwrap(),
            "--compilation",
            ":/main",
            "--symbol",
            symbol,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn authority_requires_live_worker_source_and_validation_receipts() {
    let (_temporary, fixture) = committed_fixture();
    let thread = live_thread(&fixture, "com.acme.transformAndConsume");
    let second_thread = live_thread(&fixture, "com.acme.main");
    let revision = git_head(&fixture);
    let production_source = fixture.join("src/main/kotlin/com/acme/Runner.kt");
    let original_production_source = std::fs::read(&production_source).unwrap();
    let mut dirty_production_source = original_production_source.clone();
    dirty_production_source.extend_from_slice(b"\n// not in the claimed HEAD\n");
    std::fs::write(&production_source, dirty_production_source).unwrap();
    let dirty = EvidenceAuthority::open(&fixture, &revision).err().unwrap();
    assert_eq!(dirty.code, ErrorCode::StaleRequiresReslice);
    std::fs::write(&production_source, original_production_source).unwrap();
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&fixture, &revision).unwrap();

    assert!(thread.nodes.iter().any(|node| {
        node.kind == "DEFINITION" && node.defines.as_deref() == Some("transformed")
    }));
    assert!(thread.edges.iter().any(|edge| edge.kind == "DEF_USE"));

    let mut fabricated = thread.clone();
    fabricated.edges.push(Edge {
        from: fabricated.nodes[0].id.clone(),
        to: "invented-consumer".into(),
        kind: "INVENTED_FAMILY_EDGE".into(),
    });
    let error = authority
        .verify_thread(&fabricated, &mut worker)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleRequiresReslice);

    let verified = authority.verify_thread(&thread, &mut worker).unwrap();
    let second_verified = authority
        .verify_thread(&second_thread, &mut worker)
        .unwrap();
    let unrelated = authority
        .verify_behavioral_test("applies configured limit", ":/test", &verified, &mut worker)
        .unwrap_err();
    assert_eq!(unrelated.code, ErrorCode::IncompleteSemanticAnalysis);
    let behavioral_test = authority
        .verify_behavioral_test(
            "transforms the produced value before consumption",
            ":/test",
            &verified,
            &mut worker,
        )
        .unwrap();
    let validation = authority
        .run_validation(&[&verified], &[&behavioral_test], &mut worker)
        .unwrap();
    let bundle = authority
        .authorize_bundle(&[&verified], &[&behavioral_test], &validation)
        .unwrap();
    assert_eq!(
        bundle.summary().schema,
        "authoritative-semantic-evidence/0.1"
    );
    assert_eq!(bundle.summary().revision, revision);
    assert_eq!(bundle.summary().thread_count, 1);
    assert_eq!(bundle.summary().behavioral_test_count, 1);
    assert!(!bundle.summary().evidence_fingerprint.is_empty());
    assert!(!bundle.summary().validation_artifact_hash.is_empty());
    assert!(bundle.summary().executed_test_count >= 2);

    let test_source = fixture.join("src/test/kotlin/com/acme/RunnerTest.kt");
    let original_test_source = std::fs::read(&test_source).unwrap();
    let mut changed_test_source = original_test_source.clone();
    changed_test_source.extend_from_slice(b"\n// changed after validation\n");
    std::fs::write(&test_source, changed_test_source).unwrap();
    let stale = authority
        .authorize_bundle(&[&verified], &[&behavioral_test], &validation)
        .unwrap_err();
    assert_eq!(stale.code, ErrorCode::StaleRequiresReslice);
    std::fs::write(&test_source, original_test_source).unwrap();

    let complete = authority
        .complete_for_producer_transform_consumer(
            &ProducerTransformConsumerGoal::new(&revision),
            &[&verified],
            &[&behavioral_test],
            &validation,
        )
        .unwrap();
    assert_eq!(complete.summary().schema, "complete-for-authority/0.1");
    assert_eq!(complete.summary().producer_node, "param:0");
    assert_eq!(complete.summary().transformer_node, "fir:9");
    assert_eq!(complete.summary().consumer_node, "fir:11");
    assert!(!complete.summary().goal_fingerprint.is_empty());
    assert!(authority.recognizes_complete_for(&complete).unwrap());
    let original_production_source = std::fs::read(&production_source).unwrap();
    let mut changed_production_source = original_production_source.clone();
    changed_production_source.extend_from_slice(b"\n// stale theorem\n");
    std::fs::write(&production_source, changed_production_source).unwrap();
    assert_eq!(
        authority
            .recognizes_complete_for(&complete)
            .unwrap_err()
            .code,
        ErrorCode::StaleRequiresReslice
    );
    std::fs::write(&production_source, original_production_source).unwrap();

    let wrong_goal = authority
        .complete_for_producer_transform_consumer(
            &ProducerTransformConsumerGoal::new("different-revision"),
            &[&verified],
            &[&behavioral_test],
            &validation,
        )
        .unwrap_err();
    assert_eq!(wrong_goal.code, ErrorCode::PreconditionFailed);

    let mismatch = authority
        .authorize_bundle(
            &[&verified, &second_verified],
            &[&behavioral_test],
            &validation,
        )
        .unwrap_err();
    assert_eq!(mismatch.code, ErrorCode::PreconditionFailed);

    let other = EvidenceAuthority::open(&fixture, &revision).unwrap();
    assert!(!other.recognizes_complete_for(&complete).unwrap());
    let error = other
        .authorize_bundle(&[&verified], &[&behavioral_test], &validation)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PreconditionFailed);

    let summary: Value = serde_json::to_value(
        authority
            .authorize_bundle(&[&verified], &[&behavioral_test], &validation)
            .unwrap()
            .summary(),
    )
    .unwrap();
    assert_eq!(summary["threadCount"], 1);
    assert!(summary.get("sessionId").is_none());
    assert!(summary.get("receiptId").is_none());
    worker.shutdown().unwrap();
}
