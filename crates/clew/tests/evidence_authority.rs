use clew::error::ErrorCode;
use clew::evidence_authority::EvidenceAuthority;
use clew::model::{Edge, ThreadIr};
use clew::worker::{WorkerClient, workspace_root};
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

mod support;

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
    support::seed_build_caches(&fixture);
    let prepared = Command::new("./gradlew")
        .args([
            "compileTestKotlin",
            "--offline",
            "--gradle-user-home",
            ".gradle",
            "--project-cache-dir",
            ".gradle",
            "--no-daemon",
            "--quiet",
        ])
        .current_dir(&fixture)
        .status()
        .unwrap();
    assert!(prepared.success());
    for relative in ["build/classes/java/main", "build/resources/main"] {
        std::fs::create_dir_all(fixture.join(relative)).unwrap();
    }
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
fn authority_rejects_stale_or_unmapped_kotlin21_evidence() {
    let (_temporary, fixture) = committed_fixture();
    let thread = live_thread(&fixture, "com.acme.transformAndConsume");
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
    let unrelated = authority
        .verify_behavioral_test("applies configured limit", ":/test", &verified, &mut worker)
        .unwrap_err();
    assert_eq!(
        unrelated.code,
        ErrorCode::IncompleteSemanticAnalysis,
        "{unrelated:?}"
    );
    let unmapped = authority
        .verify_behavioral_test(
            "transforms the produced value before consumption",
            ":/test",
            &verified,
            &mut worker,
        )
        .unwrap_err();
    assert_eq!(unmapped.code, ErrorCode::IncompleteSemanticAnalysis);
    worker.shutdown().unwrap();
}
