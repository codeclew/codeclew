use clew::evidence_authority::{
    EvidenceAuthority, MapEdgeRefusalReason, MapEdgeWithContextDecision,
};
use clew::model::ThreadIr;
use clew::semantic_goal::SemanticGoal;
use clew::worker::{WorkerClient, workspace_root};
use serde_json::to_value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn committed_fixture(
    mutator: impl FnOnce(&mut String, &mut String),
) -> (tempfile::TempDir, PathBuf) {
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
    let main_path = checkout.join("fixtures/kotlin-2-1/src/main/kotlin/com/acme/Runner.kt");
    let test_path = checkout.join("fixtures/kotlin-2-1/src/test/kotlin/com/acme/RunnerTest.kt");
    let mut main = std::fs::read_to_string(&main_path).unwrap();
    let mut test = std::fs::read_to_string(&test_path).unwrap();
    mutator(&mut main, &mut test);
    std::fs::write(main_path, main).unwrap();
    std::fs::write(test_path, test).unwrap();
    let relocated_main = checkout.join("fixtures/kotlin-2-1/src/main/kotlin/relocated/Feature.kt");
    let relocated_test =
        checkout.join("fixtures/kotlin-2-1/src/test/kotlin/relocated/FeatureTest.kt");
    std::fs::create_dir_all(relocated_main.parent().unwrap()).unwrap();
    std::fs::create_dir_all(relocated_test.parent().unwrap()).unwrap();
    std::fs::rename(
        checkout.join("fixtures/kotlin-2-1/src/main/kotlin/com/acme/Runner.kt"),
        relocated_main,
    )
    .unwrap();
    std::fs::rename(
        checkout.join("fixtures/kotlin-2-1/src/test/kotlin/com/acme/RunnerTest.kt"),
        relocated_test,
    )
    .unwrap();
    let add = Command::new("git")
        .args(["add", "fixtures/kotlin-2-1"])
        .current_dir(&checkout)
        .status()
        .unwrap();
    assert!(add.success());
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "goal binding fixture",
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
fn map_edge_with_context_binds_renamed_layout_and_computes_every_invariant() {
    let (_temporary, fixture) = committed_fixture(|main, test| {
        for (from, to) in [
            ("mappingContext", "environmentSeed"),
            ("applyMappingContext", "mergeSeed"),
            ("valuesAwaitingContext", "pendingValues"),
        ] {
            *main = main.replace(from, to);
            *test = test.replace(from, to);
        }
    });
    let revision = git_head(&fixture);
    let thread = live_thread(&fixture, "com.acme.pendingValues");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&fixture, &revision).unwrap();
    let workflow = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &workflow,
            "applies the mapping context to one value",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Bound(receipt) = decision else {
        panic!("renamed semantic shape must bind: {decision:?}")
    };
    let proof = receipt.summary();
    assert_eq!(proof.invariants.len(), 12);
    assert_eq!(proof.change_graph.obligations.len(), 15);
    assert!(proof.change_graph.validate_closure().is_ok());
    assert_eq!(proof.bindings.element_type, "kotlin/Int");
    assert_eq!(proof.bindings.context_type, "kotlin/Int");
    assert_eq!(
        proof.bindings.strategy,
        "KOTLIN_EAGER_LIST_MAP_WITH_CONTEXT_ONCE"
    );
    assert!(
        authority
            .recognizes_map_edge_with_context(&receipt)
            .unwrap()
    );
    let serialized = to_value(proof).unwrap().to_string();
    for forbidden in ["sourceText", "replacement", "regex", "EditIR"] {
        assert!(!serialized.contains(forbidden));
    }
    worker.shutdown().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "prove",
            "map-edge-with-context",
            "--repo",
            fixture.to_str().unwrap(),
            "--workflow-symbol",
            "com.acme.pendingValues",
            "--test-symbol",
            "applies the mapping context to one value",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "BOUND");
    assert_eq!(result["proof"]["invariants"].as_array().unwrap().len(), 12);
    assert_eq!(
        result["proof"]["changeGraph"]["obligations"]
            .as_array()
            .unwrap()
            .len(),
        15
    );
}

#[test]
fn map_edge_with_context_returns_bounded_ambiguity_and_structured_refusals() {
    let (_ambiguous_temp, ambiguous_fixture) = committed_fixture(|main, _| {
        main.push_str("\nfun anotherCompatibleContext(): Int = 3\n");
    });
    let revision = git_head(&ambiguous_fixture);
    let thread = live_thread(&ambiguous_fixture, "com.acme.valuesAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&ambiguous_fixture, &revision).unwrap();
    let workflow = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &workflow,
            "unused for ambiguity",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Ambiguous(ambiguity) = decision else {
        panic!("two semantic context candidates must remain ambiguous")
    };
    assert_eq!(ambiguity.choices.len(), 2);
    worker.shutdown().unwrap();

    let (_sequence_temp, sequence_fixture) = committed_fixture(|main, _| {
        main.push_str(
            "\nfun sequenceAwaitingContext(values: Sequence<Int>): Sequence<Int> = values\n",
        );
    });
    let revision = git_head(&sequence_fixture);
    let thread = live_thread(&sequence_fixture, "com.acme.sequenceAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&sequence_fixture, &revision).unwrap();
    let workflow = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &workflow,
            "unused for refusal",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Refused(refusal) = decision else {
        panic!("lazy sequence must refuse")
    };
    assert_eq!(
        refusal.reason,
        MapEdgeRefusalReason::UnsupportedCollectionModality
    );
    worker.shutdown().unwrap();

    let (_effect_temp, effect_fixture) = committed_fixture(|main, _| {
        *main = main.replace(
            "fun applyMappingContext(value: Int, context: Int): Int = value + context",
            "fun applyMappingContext(value: Int, context: Int): Int { println(context); return value }",
        );
    });
    let revision = git_head(&effect_fixture);
    let thread = live_thread(&effect_fixture, "com.acme.valuesAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&effect_fixture, &revision).unwrap();
    let workflow = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &workflow,
            "unused for refusal",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Refused(refusal) = decision else {
        panic!("unknown transformer effect must refuse")
    };
    assert_eq!(refusal.reason, MapEdgeRefusalReason::UnknownEffects);
    worker.shutdown().unwrap();

    let (_oracle_temp, oracle_fixture) = committed_fixture(|_, _| {});
    let revision = git_head(&oracle_fixture);
    let thread = live_thread(&oracle_fixture, "com.acme.valuesAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&oracle_fixture, &revision).unwrap();
    let workflow = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &workflow,
            "applies configured limit",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Refused(refusal) = decision else {
        panic!("unrelated test must refuse")
    };
    assert_eq!(
        refusal.reason,
        MapEdgeRefusalReason::MissingBehavioralOracle
    );
    worker.shutdown().unwrap();
}

#[test]
fn map_edge_with_context_refuses_callables_with_an_unbound_receiver() {
    let (_temporary, fixture) = committed_fixture(|main, test| {
        *main = main
            .replace(
                "fun mappingContext(): Int = 2",
                "class MappingHelpers {\n    fun mappingContext(): Int = 2",
            )
            .replace(
                "fun applyMappingContext(value: Int, context: Int): Int = value + context",
                "    fun applyMappingContext(value: Int, context: Int): Int = value + context\n}",
            );
        *test = test.replace(
            "assertEquals(6, applyMappingContext(4, mappingContext()))",
            "val helper = MappingHelpers()\n        assertEquals(6, helper.applyMappingContext(4, helper.mappingContext()))",
        );
    });
    let revision = git_head(&fixture);
    let thread = live_thread(&fixture, "com.acme.valuesAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&fixture, &revision).unwrap();
    let workflow = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &workflow,
            "applies the mapping context to one value",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Refused(refusal) = decision else {
        panic!("unbound dispatch receiver must not produce BOUND")
    };
    assert_eq!(
        refusal.reason,
        MapEdgeRefusalReason::NoCompatibleContextAndTransformer
    );
    worker.shutdown().unwrap();
}
