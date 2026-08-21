mod support;

use clew::evidence_authority::{
    EvidenceAuthority, MapEdgeRefusalReason, MapEdgeWithContextDecision,
    TYPED_GOAL_BINDING_REQUEST_SCHEMA, TypedGoalBindingDecision, TypedGoalBindingRequest,
    TypedGoalRefusalReason,
};
use clew::model::ThreadIr;
use clew::proto::RequestKind;
use clew::semantic_goal::{
    OperatorApplication, PrimitiveConstraint, SemanticGoal, TypedGoalLanguageSchema,
    TypedSemanticGoal, TypedVariableDomain, typed_goal_language_schema,
};
use clew::worker::{WorkerClient, workspace_root};
use serde_json::{json, to_value};
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

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
    let build = checkout.join("fixtures/kotlin-2-1/build.gradle.kts");
    let build_source = std::fs::read_to_string(&build).unwrap();
    std::fs::write(
        &build,
        build_source
            .replace(
                "kotlin(\"jvm\") version \"2.1.21\"",
                "kotlin(\"jvm\") version \"2.4.10\"",
            )
            .replace(
                "    kotlin(\"plugin.serialization\") version \"2.1.21\"\n",
                "",
            ),
    )
    .unwrap();
    for relative in [
        "fixtures/kotlin-2-1/src/main/kotlin/com/acme/Adaptive.kt",
        "fixtures/kotlin-2-1/src/main/kotlin/com/acme/RelationFacts.kt",
    ] {
        std::fs::remove_file(checkout.join(relative)).unwrap();
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

fn committed_multimodule_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temporary, source_fixture) = committed_fixture(|_, _| {});
    let checkout = source_fixture
        .parent()
        .and_then(Path::parent)
        .expect("fixture belongs to checkout");
    let repository = checkout.join("multi-module-fixture");
    let module = repository.join("service");
    std::fs::create_dir_all(&module).unwrap();
    std::fs::write(
        repository.join("settings.gradle.kts"),
        r#"pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }
dependencyResolutionManagement { repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS); repositories { mavenCentral() } }
rootProject.name = "typed-goal-multimodule"
include(":service")
"#,
    )
    .unwrap();
    std::fs::write(
        repository.join(".gitignore"),
        "/.gradle/\n**/build/\n/.semantic-thread/\n",
    )
    .unwrap();
    std::fs::write(
        repository.join("gradlew"),
        "#!/bin/sh\nexec \"$(dirname \"$0\")/../fixtures/kotlin-basic/gradlew\" \"$@\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(repository.join("gradlew"))
            .unwrap()
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(repository.join("gradlew"), permissions).unwrap();
    }
    std::fs::copy(
        source_fixture.join("build.gradle.kts"),
        module.join("build.gradle.kts"),
    )
    .unwrap();
    for entry in WalkDir::new(source_fixture.join("src")) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(&source_fixture).unwrap();
        let target = module.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(entry.path(), target).unwrap();
    }
    assert!(
        Command::new("git")
            .args(["add", "multi-module-fixture"])
            .current_dir(checkout)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "multi-module typed goal fixture",
            ])
            .current_dir(checkout)
            .status()
            .unwrap()
            .success()
    );
    support::seed_build_caches(&repository);
    let prepared = Command::new("./gradlew")
        .args([
            ":service:compileTestKotlin",
            "--offline",
            "--gradle-user-home",
            ".gradle",
            "--project-cache-dir",
            ".gradle",
            "--no-daemon",
            "--quiet",
        ])
        .current_dir(&repository)
        .status()
        .unwrap();
    assert!(prepared.success());
    for relative in [
        "service/build/classes/java/main",
        "service/build/resources/main",
    ] {
        std::fs::create_dir_all(repository.join(relative)).unwrap();
    }
    (temporary, repository)
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

fn map_operator_goal(revision: &str, names: [&str; 3]) -> TypedSemanticGoal {
    TypedSemanticGoal::new(
        revision,
        [
            (names[0].into(), TypedVariableDomain::Callable),
            (names[1].into(), TypedVariableDomain::Callable),
            (names[2].into(), TypedVariableDomain::ValueEdge),
        ],
        [OperatorApplication {
            operator: PrimitiveConstraint::MapEdge,
            operands: names.into_iter().map(str::to_owned).collect(),
        }],
    )
}

fn typed_request(revision: &str, hints: Vec<String>) -> TypedGoalBindingRequest {
    TypedGoalBindingRequest {
        schema: TYPED_GOAL_BINDING_REQUEST_SCHEMA.into(),
        goal: map_operator_goal(revision, ["alpha", "beta", "gamma"]),
        hints,
        compilation: None,
    }
}

#[test]
fn signed_external_spec_cli_refuses_unsigned_document() {
    let (temporary, fixture) = committed_fixture(|_, _| {});
    let revision = git_head(&fixture);
    let request = typed_request(&revision, vec![]);
    let request_path = temporary.path().join("typed-request.json");
    std::fs::write(&request_path, clew::canonical::bytes(&request).unwrap()).unwrap();
    let unsigned_path = temporary.path().join("unsigned-external-spec.json");
    std::fs::write(
        &unsigned_path,
        clew::canonical::bytes(&json!({
            "schema":"external-task-spec/0.1",
            "task":"unsigned caller statement",
            "repository":"repository",
            "sourceSnapshotSha256":"0".repeat(64)
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["prove", "typed-goal", "--repo"])
        .arg(&fixture)
        .arg("--request")
        .arg(&request_path)
        .arg("--external-spec")
        .arg(&unsigned_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "REFUSED");
    assert_eq!(result["reason"], "EXTERNAL_SPECIFICATION_MISMATCH");
}

#[test]
fn typed_goal_entrypoint_surfaces_conditional_mapping_and_keeps_strict_proofs_unforgeable() {
    let (temporary, fixture) = committed_fixture(|main, test| {
        for (from, to) in [
            ("mappingContext", "stableSeed"),
            ("applyMappingContext", "mergeStableSeed"),
            ("valuesAwaitingContext", "pendingStableValues"),
        ] {
            *main = main.replace(from, to);
            *test = test.replace(from, to);
        }
    });
    let revision = git_head(&fixture);
    let goal = map_operator_goal(&revision, ["alpha", "beta", "gamma"]);
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&fixture, &revision).unwrap();
    let decision = authority
        .bind_typed_goal(&goal, &["non-authoritative".into()], None, &mut worker)
        .unwrap();
    let TypedGoalBindingDecision::Conditional(conditional) = decision else {
        panic!("missing compiler call mapping must be conditional: {decision:?}")
    };
    assert_eq!(
        conditional.bindings.keys().cloned().collect::<Vec<_>>(),
        vec!["alpha", "beta", "gamma"]
    );
    assert_eq!(conditional.unresolved_obligations.len(), 2);
    assert!(
        conditional
            .unresolved_obligations
            .iter()
            .all(|obligation| obligation.publication_blocking)
    );

    let unseen = TypedSemanticGoal::new(
        &revision,
        [
            ("call".into(), TypedVariableDomain::Callable),
            ("flow".into(), TypedVariableDomain::ValueEdge),
        ],
        [OperatorApplication {
            operator: PrimitiveConstraint::TypeAssignable,
            operands: vec!["call".into(), "flow".into()],
        }],
    );
    let decision = authority
        .bind_typed_goal(&unseen, &[], None, &mut worker)
        .unwrap();
    let TypedGoalBindingDecision::Bound(unseen_receipt) = decision else {
        panic!("unseen TYPE_ASSIGNABLE composition must use the same operator executor")
    };
    assert!(unseen_receipt.summary().is_complete_for(&unseen));
    assert_eq!(unseen_receipt.summary().discharged_operators.len(), 3);
    assert!(authority.recognizes_typed_goal(&unseen_receipt).unwrap());
    let mut forged = unseen_receipt.summary().clone();
    forged.goal_fingerprint = "forged".into();
    assert!(!forged.is_complete_for(&unseen));
    assert!(!authority.recognizes_typed_goal_summary(&forged).unwrap());
    let mut current_unknown = unseen_receipt.summary().clone();
    current_unknown.evidence_relations[0].unknown = true;
    assert!(!current_unknown.is_complete_for(&unseen));
    assert!(
        !authority
            .recognizes_typed_goal_summary(&current_unknown)
            .unwrap()
    );

    let production = fixture.join("src/main/kotlin/relocated/Feature.kt");
    let original = std::fs::read(&production).unwrap();
    let mut stale = original.clone();
    stale.extend_from_slice(b"\n// stale proof\n");
    std::fs::write(&production, stale).unwrap();
    assert!(authority.recognizes_typed_goal(&unseen_receipt).is_err());
    std::fs::write(&production, original).unwrap();
    worker.shutdown().unwrap();

    let request = typed_request(&revision, vec![]);
    let request_json = serde_json::to_value(&request).unwrap();
    assert!(request_json.get("family").is_none());
    assert!(request_json["goal"].get("family").is_none());
    assert!(request_json.get("roots").is_none());
    let request_path = temporary.path().join("typed-goal-request.json");
    std::fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "prove",
            "typed-goal",
            "--repo",
            fixture.to_str().unwrap(),
            "--request",
            request_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "CONDITIONAL");
    assert_eq!(result["bindings"].as_object().unwrap().len(), 3);
    assert_eq!(result["unresolvedObligations"].as_array().unwrap().len(), 2);
    assert!(result.get("proof").is_none());
}

#[test]
fn typed_goal_entrypoint_preserves_ambiguity_and_refuses_unknown_composition() {
    let (_temporary, fixture) = committed_fixture(|main, _| {
        main.push_str("\nfun anotherTypedContext(): Int = 3\n");
    });
    let revision = git_head(&fixture);
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&fixture, &revision).unwrap();
    let decision = authority
        .bind_typed_goal(
            &map_operator_goal(&revision, ["x", "y", "z"]),
            &[],
            None,
            &mut worker,
        )
        .unwrap();
    let TypedGoalBindingDecision::Ambiguous(ambiguity) = decision else {
        panic!("multiple compiler-compatible bindings must remain ambiguous")
    };
    assert_eq!(ambiguity.choices.len(), 2);
    assert!(ambiguity.choices.iter().all(|choice| {
        choice
            .bindings
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            == vec!["x", "y", "z"]
    }));

    let unsupported = TypedSemanticGoal::new(
        &revision,
        [("edge".into(), TypedVariableDomain::ValueEdge)],
        [OperatorApplication {
            operator: PrimitiveConstraint::PreserveResourceLifetime,
            operands: vec!["edge".into()],
        }],
    );
    let decision = authority
        .bind_typed_goal(&unsupported, &[], None, &mut worker)
        .unwrap();
    let TypedGoalBindingDecision::Refused(refusal) = decision else {
        panic!("unknown constraint composition must fail closed")
    };
    assert_eq!(
        refusal.reason,
        TypedGoalRefusalReason::UnsupportedConstraintDomain
    );

    let bare = TypedSemanticGoal::new(
        &revision,
        [("only".into(), TypedVariableDomain::Callable)],
        [OperatorApplication {
            operator: PrimitiveConstraint::BindUnique,
            operands: vec!["only".into()],
        }],
    );
    let decision = authority
        .bind_typed_goal(&bare, &[], None, &mut worker)
        .unwrap();
    let TypedGoalBindingDecision::Refused(refusal) = decision else {
        panic!("bare BIND_UNIQUE must never be executable")
    };
    assert_eq!(refusal.reason, TypedGoalRefusalReason::InvalidGoal);
    worker.shutdown().unwrap();
}

#[test]
fn typed_goal_refuses_when_compiler_effect_fact_is_not_safe() {
    let (_temporary, fixture) = committed_fixture(|main, _| {
        *main = main.replace(
            "fun applyMappingContext(value: Int, context: Int): Int = value + context",
            "fun applyMappingContext(value: Int, context: Int): Int { println(value); return value + context }",
        );
    });
    let revision = git_head(&fixture);
    let goal = map_operator_goal(&revision, ["context", "transform", "edge"]);
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&fixture, &revision).unwrap();
    let decision = authority
        .bind_typed_goal(&goal, &[], None, &mut worker)
        .unwrap();
    let TypedGoalBindingDecision::Refused(refusal) = decision else {
        panic!("an edge without a safe compiler effect fact must be refused")
    };
    assert_eq!(refusal.reason, TypedGoalRefusalReason::UnknownEffects);
    worker.shutdown().unwrap();
}

#[test]
fn gradle_model_routes_module_test_classpath_to_production_output() {
    let (_temporary, fixture) = committed_multimodule_fixture();
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let test_project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":service/test"}),
        )
        .unwrap();
    assert_eq!(test_project["module"], ":service");
    assert_eq!(test_project["sourceSet"], "test");
    assert!(
        test_project["compileClasspath"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(|identity| identity.starts_with("repo:service/build/classes/kotlin/main:")),
        "test compilation must contain its module's production output: {}",
        test_project["compileClasspath"]
    );
    worker.shutdown().unwrap();
}

#[test]
fn typed_goal_routes_explicit_multimodule_compilation_and_same_module_test() {
    let (_temporary, fixture) = committed_multimodule_fixture();
    let revision = git_head(&fixture);
    let goal = map_operator_goal(&revision, ["context", "transform", "edge"]);
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&fixture, &revision).unwrap();
    let decision = authority
        .bind_typed_goal(&goal, &[], Some(":service/main"), &mut worker)
        .unwrap();
    let TypedGoalBindingDecision::Bound(receipt) = decision else {
        panic!(
            "explicit production compilation must bind in its same-module test contour: {decision:?}"
        )
    };
    assert!(receipt.summary().is_complete_for(&goal));
    assert!(
        receipt
            .summary()
            .change_graph
            .obligations
            .iter()
            .all(|obligation| !obligation.evidence.is_empty())
    );
    worker.shutdown().unwrap();
}

#[test]
fn typed_goal_cli_inline_matches_file_transport() {
    let (temporary, fixture) = committed_fixture(|_, _| {});
    let revision = git_head(&fixture);
    let request = typed_request(&revision, vec![]);
    let canonical_request = String::from_utf8(clew::canonical::bytes(&request).unwrap()).unwrap();
    let request_path = temporary.path().join("typed-goal-cli-request.json");
    std::fs::write(&request_path, canonical_request.as_bytes()).unwrap();

    let file = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["prove", "typed-goal", "--repo"])
        .arg(&fixture)
        .arg("--request")
        .arg(&request_path)
        .output()
        .unwrap();
    assert!(
        file.status.success(),
        "{}",
        String::from_utf8_lossy(&file.stderr)
    );
    let file_result: serde_json::Value = serde_json::from_slice(&file.stdout).unwrap();

    let inline = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["prove", "typed-goal", "--repo"])
        .arg(&fixture)
        .arg("--request-json")
        .arg(&canonical_request)
        .output()
        .unwrap();
    assert!(
        inline.status.success(),
        "{}",
        String::from_utf8_lossy(&inline.stderr)
    );
    let inline_result: serde_json::Value = serde_json::from_slice(&inline.stdout).unwrap();
    assert_eq!(file_result["status"], "BOUND");
    assert_eq!(inline_result["status"], "BOUND");
    assert_eq!(
        file_result["proof"]["bindings"],
        inline_result["proof"]["bindings"]
    );
    assert_eq!(
        file_result["proof"]["dischargedOperators"],
        inline_result["proof"]["dischargedOperators"]
    );
}

#[test]
fn typed_goal_cli_rejects_malformed_oversize_and_dual_transport() {
    let workspace = workspace_root();
    let malformed = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["prove", "typed-goal", "--repo"])
        .arg(&workspace)
        .args(["--request-json", "{"])
        .output()
        .unwrap();
    assert!(!malformed.status.success());

    let oversized = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["prove", "typed-goal", "--repo"])
        .arg(&workspace)
        .arg("--request-json")
        .arg("x".repeat(16 * 1024 + 1))
        .output()
        .unwrap();
    assert!(!oversized.status.success());

    let temporary = tempfile::tempdir().unwrap();
    let request_path = temporary.path().join("request.json");
    std::fs::write(&request_path, b"{}").unwrap();
    let dual = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["prove", "typed-goal", "--repo"])
        .arg(&workspace)
        .arg("--request")
        .arg(&request_path)
        .args(["--request-json", "{}"])
        .output()
        .unwrap();
    assert!(!dual.status.success());
}

#[test]
fn typed_goal_cli_refuses_explicit_compilation_mismatch() {
    let (temporary, fixture) = committed_fixture(|_, _| {});
    let revision = git_head(&fixture);
    let mut request = typed_request(&revision, vec![]);
    request.compilation = Some(":/main".into());
    let request_path = temporary
        .path()
        .join("typed-goal-compilation-mismatch.json");
    std::fs::write(&request_path, clew::canonical::bytes(&request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["prove", "typed-goal", "--repo"])
        .arg(&fixture)
        .arg("--request")
        .arg(&request_path)
        .args(["--compilation", ":service/main"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "REFUSED");
    assert_eq!(result["reason"], "INVALID_GOAL");
}

#[test]
fn typed_goal_schema_cli_emits_the_canonical_product_registry() {
    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["schema", "typed-goal"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let emitted: TypedGoalLanguageSchema = serde_json::from_slice(&output.stdout).unwrap();
    let expected = typed_goal_language_schema();
    assert_eq!(emitted, expected);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        clew::canonical::pretty(&expected).unwrap()
    );
}

#[test]
fn map_edge_with_context_surfaces_renamed_layout_with_blocking_oracle_obligations() {
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
    let MapEdgeWithContextDecision::Conditional(proof) = decision else {
        panic!("renamed semantic shape must be actionable conditional: {decision:?}")
    };
    assert_eq!(proof.established_invariants.len(), 11);
    assert_eq!(proof.change_graph.obligations.len(), 15);
    assert!(proof.change_graph.validate_closure().is_ok());
    assert_eq!(proof.bindings.element_type, "kotlin/Int");
    assert_eq!(proof.bindings.context_type, "kotlin/Int");
    assert_eq!(
        proof.bindings.strategy,
        "KOTLIN_EAGER_LIST_MAP_WITH_CONTEXT_ONCE"
    );
    assert_eq!(proof.unresolved_obligations.len(), 2);
    assert!(proof.change_graph.obligations.iter().any(|obligation| {
        obligation.kind == clew::semantic_goal::ObligationKind::RequireOracle
            && obligation.status == clew::semantic_goal::DischargeStatus::Unproved
    }));
    let serialized = to_value(&proof).unwrap().to_string();
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
    assert_eq!(result["status"], "CONDITIONAL");
    assert_eq!(
        result["establishedInvariants"].as_array().unwrap().len(),
        11
    );
    assert_eq!(
        result["changeGraph"]["obligations"]
            .as_array()
            .unwrap()
            .len(),
        15
    );
    assert!(result.get("proof").is_none());
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
