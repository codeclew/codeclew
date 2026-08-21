mod support;

use clew::canonical;
use clew::evidence_authority::EvidenceAuthority;
use clew::graph;
use clew::index::RepositoryIndex;
use clew::model::{
    BuildSystem, EditIr, EditOperation, LocalGraph, Replacement, SlicePolicy, Snapshot, Transaction,
};
use clew::proto::RequestKind;
use clew::transaction;
use clew::worker::{WorkerClient, workspace_root};
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn copy_maven_fixture(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from).into_iter().map(Result::unwrap) {
        let relative = entry.path().strip_prefix(from).unwrap();
        if relative.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some("target" | ".semantic-thread")
            )
        }) {
            continue;
        }
        let target = to.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
    support::seed_build_caches(to);
}

fn copy_worker_distribution(source: &Path, target: &Path) {
    for entry in walkdir::WalkDir::new(source)
        .into_iter()
        .map(Result::unwrap)
    {
        let relative = entry.path().strip_prefix(source).unwrap();
        let destination = target.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&destination).unwrap();
        } else if entry.file_type().is_file() {
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(entry.path(), &destination).unwrap();
            std::fs::set_permissions(&destination, entry.metadata().unwrap().permissions())
                .unwrap();
        } else {
            panic!(
                "worker distribution contains unsupported path: {}",
                entry.path().display()
            );
        }
    }
}

fn declaration_relation_rows<'a>(index: &'a Value, kind: &str) -> Vec<&'a Value> {
    index["declarationRelations"]["relations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|relation| relation["kind"] == kind)
        .collect()
}

fn declaration_descriptor_rows(index: &Value) -> &[Value] {
    index["declarationDescriptors"]["descriptors"]
        .as_array()
        .unwrap()
}

fn init_maven_repo(root: &Path) -> PathBuf {
    let repo = root.join("maven-repo");
    copy_maven_fixture(&workspace_root().join("fixtures/kotlin-maven"), &repo);
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-qm",
            "baseline",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    }
    repo
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn live_thread(repo: &Path, symbol: &str) -> clew::model::ThreadIr {
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
fn indexes_constructor_and_null_coalescing_facts_on_kotlin_23_maven() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    support::seed_build_caches(&fixture);
    let diagnostic_workspace = tempfile::tempdir().unwrap();
    copy_worker_distribution(
        &root.join("workers/kotlin/build/install/kotlin"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin/build/install/kotlin"),
    );
    copy_worker_distribution(
        &root.join("workers/kotlin23/build/install/kotlin23"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin23/build/install/kotlin23"),
    );
    let mut worker = WorkerClient::start(diagnostic_workspace.path()).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.3.0");
    let index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    let repeated = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();

    assert_eq!(index["k2Validated"], true, "{:#}", index["diagnostics"]);
    for graph in ["declarationDescriptors", "declarationRelations"] {
        assert_eq!(
            index[graph]["provenance"]["extractorSchema"],
            "fir-facts-extractor/0.6"
        );
        assert_eq!(index[graph]["provenance"]["compilerVersion"], "2.3.0");
        assert_eq!(index[graph], repeated[graph]);
    }
    assert_eq!(
        index["declarationDescriptorHash"],
        repeated["declarationDescriptorHash"]
    );
    assert_eq!(
        index["declarationRelationHash"],
        repeated["declarationRelationHash"]
    );

    let descriptors = declaration_descriptor_rows(&index);
    let constructor = descriptors
        .iter()
        .find(|descriptor| {
            descriptor["declarationKind"] == "CONSTRUCTOR"
                && descriptor["compilerClassId"] == "com/acme/relations/NullableConstruction"
        })
        .expect("compiler constructor descriptor");
    assert_eq!(constructor["resolution"], "PROVEN");
    assert_eq!(constructor["provider"], "K2_FIR");
    assert_eq!(constructor["compilerAuthority"], "fir-facts-extractor/0.6");
    assert_eq!(
        constructor["ownerIdentity"],
        "class:com/acme/relations/NullableConstruction"
    );
    assert!(
        constructor["symbolIdentity"]
            .as_str()
            .is_some_and(
                |identity| identity.starts_with("constructor:") && identity.contains("#jvm:")
            )
    );
    let parameters = constructor["parameterTypes"].as_array().unwrap();
    assert_eq!(parameters.len(), 2);
    assert!(parameters.iter().enumerate().all(|(index, parameter)| {
        parameter["index"] == index
            && parameter["type"] == "kotlin/String"
            && parameter["nullable"] == false
    }));

    let construction = declaration_relation_rows(&index, "CONSTRUCTS")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("constructWithNullPolicy"))
                && relation["target"] == constructor["compilerCallableId"]
        })
        .expect("exact constructor occurrence");
    let mapping = construction["argumentToParameter"].as_array().unwrap();
    assert_eq!(
        mapping
            .iter()
            .map(|row| row["parameterIndex"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(mapping.iter().all(|row| {
        let index = row["parameterIndex"].as_u64().unwrap() as usize;
        row["parameterType"] == parameters[index]["type"]
    }));

    let null_policy = declaration_relation_rows(&index, "NULL_COALESCES")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("constructWithNullPolicy"))
        })
        .expect("exact compiler null-coalescing relation");
    assert_eq!(null_policy["resolution"], "PROVEN");
    assert_eq!(null_policy["provider"], "K2_FIR");
    assert_eq!(null_policy["sourceOccurrence"]["type"], "kotlin/String?");
    assert_eq!(null_policy["sourceOccurrence"]["nullable"], true);
    assert_eq!(null_policy["fallbackOccurrence"]["type"], "kotlin/String");
    assert_eq!(null_policy["fallbackOccurrence"]["nullable"], false);
    assert_eq!(null_policy["mergedOccurrence"]["type"], "kotlin/String");
    assert_eq!(null_policy["mergedOccurrence"]["nullable"], false);
    assert_eq!(
        null_policy["branchProvenance"]["kind"],
        "FIR_ELVIS_EXPRESSION"
    );
    for occurrence in ["sourceOccurrence", "fallbackOccurrence", "mergedOccurrence"] {
        assert!(null_policy[occurrence]["start"].as_u64().is_some());
        assert!(
            null_policy[occurrence]["end"].as_u64().unwrap()
                > null_policy[occurrence]["start"].as_u64().unwrap()
        );
    }
    assert_eq!(
        mapping
            .iter()
            .find(|row| row["parameterIndex"] == 1)
            .unwrap()["argumentStart"],
        null_policy["mergedOccurrence"]["start"]
    );
    assert!(
        null_policy["sourceTarget"]
            .as_str()
            .is_some_and(|target| target.contains("compilerNullableSource"))
    );
    assert!(
        null_policy["fallbackTarget"]
            .as_str()
            .is_some_and(|target| target.contains("compilerFallback") && !target.contains("Decoy"))
    );
    assert!(
        null_policy["cfgNodeIds"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty())
    );
    assert_eq!(null_policy["orderProvenance"], "K2_FIR_CFG");

    assert!(
        index["declarationRelations"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["resolution"] == "UNKNOWN"
                    && boundary["stage"] == "NULL_POLICY"
                    && matches!(
                        boundary["code"].as_str(),
                        Some(
                            "SAFE_CALL_POLICY_UNSUPPORTED"
                                | "UNRESOLVED_NULLABLE_SOURCE_OCCURRENCE"
                        )
                    )
            })
    );
    assert!(
        index["declarationDescriptors"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["resolution"] == "UNKNOWN"
                    && matches!(
                        boundary["code"].as_str(),
                        Some(
                            "LOCAL_CONSTRUCTOR_UNSUPPORTED"
                                | "GENERATED_OR_NO_SOURCE"
                                | "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR"
                        )
                    )
            })
    );
    worker.shutdown().unwrap();
}

#[test]
fn indexes_direct_return_value_relations_on_kotlin_23_maven() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    support::seed_build_caches(&fixture);
    let diagnostic_workspace = tempfile::tempdir().unwrap();
    copy_worker_distribution(
        &root.join("workers/kotlin/build/install/kotlin"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin/build/install/kotlin"),
    );
    copy_worker_distribution(
        &root.join("workers/kotlin23/build/install/kotlin23"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin23/build/install/kotlin23"),
    );
    let mut worker = WorkerClient::start(diagnostic_workspace.path()).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.3.0");
    let index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    let repeated = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    assert_eq!(index["k2Validated"], true, "{:#}", index["diagnostics"]);
    assert_eq!(
        index["declarationRelations"]["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.6"
    );
    assert_eq!(
        index["declarationRelations"]["provenance"]["compilerVersion"],
        "2.3.0"
    );
    assert_eq!(
        index["declarationRelationHash"],
        repeated["declarationRelationHash"]
    );
    assert_eq!(
        index["declarationRelations"],
        repeated["declarationRelations"]
    );

    let returned = declaration_relation_rows(&index, "RETURNS_VALUE_FROM");
    assert_eq!(returned.len(), 2);
    let property = returned
        .iter()
        .copied()
        .find(|row| row["sourceKind"] == "PROPERTY_READ")
        .expect("direct returned property read");
    assert_eq!(
        property["owner"],
        "com/acme/relations/DirectReturnProjection.returnedProperty"
    );
    assert_eq!(
        property["target"],
        "com/acme/relations/DirectReturnProjection.projected"
    );
    assert_eq!(property["resultType"], "kotlin/String");
    assert_eq!(property["resultNullable"], false);
    assert_eq!(property["evaluationCount"], 1);
    assert_eq!(property["valueProvenance"], "FIR_RETURN_RESULT_IDENTITY");
    assert_eq!(property["cfgProvenance"]["sourceReachesReturn"], true);
    assert_eq!(property["cfgProvenance"]["sourceDominatesReturn"], true);
    assert_eq!(
        property["cfgProvenance"]["sourceNodeKind"],
        "QualifiedAccessNode"
    );
    assert_eq!(property["cfgProvenance"]["returnNodeKind"], "JumpNode");
    assert_eq!(property["orderProvenance"], "K2_FIR_CFG");
    let source_start = property["sourceOccurrence"]["start"].as_u64().unwrap();
    let source_end = property["sourceOccurrence"]["end"].as_u64().unwrap();
    let return_start = property["returnOccurrence"]["start"].as_u64().unwrap();
    let return_end = property["returnOccurrence"]["end"].as_u64().unwrap();
    assert!(return_start <= source_start && source_end <= return_end);
    assert_eq!(property["sourceOccurrence"], property["resultOccurrence"]);
    let cfg_ids = property["cfgNodeIds"].as_array().unwrap();
    assert!(cfg_ids.contains(&property["sourceOccurrence"]["cfgNodeId"]));
    assert!(cfg_ids.contains(&property["returnOccurrence"]["cfgNodeId"]));
    assert!(
        !property["target"]
            .as_str()
            .unwrap()
            .contains("sameTypedDecoy")
    );

    let call = returned
        .iter()
        .copied()
        .find(|row| row["sourceKind"] == "FUNCTION_CALL_RESULT")
        .expect("direct returned function call");
    assert_eq!(call["owner"], "com/acme/relations/directReturnedCall");
    assert_eq!(call["target"], "com/acme/relations/internalDescriptor");
    assert_eq!(call["resultType"], "kotlin/String");
    assert_eq!(call["resultNullable"], false);
    assert_eq!(call["evaluationCount"], 1);
    assert_eq!(call["cfgProvenance"]["sourceReachesReturn"], true);
    assert_eq!(call["cfgProvenance"]["sourceDominatesReturn"], true);
    assert_eq!(
        call["cfgProvenance"]["sourceNodeKind"],
        "FunctionCallExitNode"
    );
    assert_eq!(call["cfgProvenance"]["returnNodeKind"], "JumpNode");

    let boundaries = index["declarationRelations"]["boundaries"]
        .as_array()
        .unwrap();
    let has_boundary = |owner_fragment: &str, codes: &[&str]| {
        boundaries.iter().any(|boundary| {
            boundary["resolution"] == "UNKNOWN"
                && boundary["stage"] == "RETURN_VALUE"
                && boundary["owner"]
                    .as_str()
                    .is_some_and(|owner| owner.contains(owner_fragment))
                && boundary["code"]
                    .as_str()
                    .is_some_and(|code| codes.contains(&code))
        })
    };
    for (owner, codes) in [
        (
            "aliasedProperty",
            &["LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"][..],
        ),
        (
            "branchedProperty",
            &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"][..],
        ),
        (
            "implicitProperty",
            &[
                "IMPLICIT_RETURN_UNSUPPORTED",
                "IMPLICIT_OR_MISSING_RETURN_SOURCE",
            ][..],
        ),
        (
            "multipleReturnedCalls",
            &["MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES"][..],
        ),
        (
            "safeReturnedProperty",
            &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"][..],
        ),
        (
            "elvisReturnedProperty",
            &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"][..],
        ),
        (
            "unresolvedSourceReturn",
            &["LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"][..],
        ),
    ] {
        assert!(
            has_boundary(owner, codes),
            "missing UNKNOWN boundary for rejected contour"
        );
        assert!(returned.iter().all(|relation| {
            !relation["owner"]
                .as_str()
                .is_some_and(|candidate| candidate.contains(owner))
        }));
    }
    worker.shutdown().unwrap();
}

#[test]
fn maven_authority_refuses_behavioral_test_without_exact_call_linkage() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = init_maven_repo(temporary.path());
    let thread = live_thread(&repo, "com.acme.flow.transformAndConsume");
    let revision = git_output(&repo, &["rev-parse", "HEAD"]);
    let root = workspace_root();
    let mut worker = WorkerClient::start(&root).unwrap();
    let mut authority = EvidenceAuthority::open(&repo, &revision).unwrap();
    let verified = authority.verify_thread(&thread, &mut worker).unwrap();
    let error = authority
        .verify_behavioral_test(
            "transformsProducedValueBeforeConsumption",
            ":/test",
            &verified,
            &mut worker,
        )
        .unwrap_err();
    assert_eq!(
        error.code,
        clew::error::ErrorCode::IncompleteSemanticAnalysis
    );
    assert!(error.message.contains("exact production call"));
    worker.shutdown().unwrap();
}

#[test]
fn disabled_maven_test_cannot_bypass_missing_exact_call_linkage() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = init_maven_repo(temporary.path());
    let source = repo.join("src/test/kotlin/com/acme/flow/MavenFlowTest.kt");
    let original = std::fs::read_to_string(&source).unwrap();
    let disabled = original
        .replace(
            "import kotlin.test.assertEquals",
            "import kotlin.test.assertEquals\nimport org.junit.jupiter.api.Disabled",
        )
        .replace(
            "    @Test\n    fun transformsProducedValueBeforeConsumption()",
            "    @Test\n    @Disabled(\"authority regression\")\n    fun transformsProducedValueBeforeConsumption()",
        );
    assert_ne!(disabled, original);
    std::fs::write(&source, disabled).unwrap();
    assert!(
        Command::new("git")
            .args(["add", "src/test/kotlin/com/acme/flow/MavenFlowTest.kt"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@localhost",
                "commit",
                "-qm",
                "disable linked test"
            ])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );

    let thread = live_thread(&repo, "com.acme.flow.transformAndConsume");
    let revision = git_output(&repo, &["rev-parse", "HEAD"]);
    let root = workspace_root();
    let mut worker = WorkerClient::start(&root).unwrap();
    let mut authority = EvidenceAuthority::open(&repo, &revision).unwrap();
    let verified = authority.verify_thread(&thread, &mut worker).unwrap();
    let error = authority
        .verify_behavioral_test(
            "transformsProducedValueBeforeConsumption",
            ":/test",
            &verified,
            &mut worker,
        )
        .unwrap_err();
    assert_eq!(
        error.code,
        clew::error::ErrorCode::IncompleteSemanticAnalysis
    );
    assert!(error.message.contains("exact production call"));
    worker.shutdown().unwrap();
}

#[test]
fn opens_maven_kotlin_23_project_with_exact_worker_and_build_plan() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();

    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo": fixture, "compilation": ":/main"}),
        )
        .unwrap();

    assert_eq!(project["buildSystem"], "MAVEN");
    assert_eq!(project["buildLauncher"], "./mvnw");
    assert_eq!(project["compilerVersion"], "2.3.0");
    assert_eq!(project["workerCompilerVersion"], "2.3.0");
    assert_eq!(worker.capabilities.compiler_version, "2.3.0");
    assert_eq!(project["languageVersion"], "2.3");
    assert_eq!(project["apiVersion"], "2.3");
    assert_eq!(project["jvmTarget"], "21");
    assert_eq!(project["compileTask"], "compile");
    assert_eq!(project["testTasks"], json!(["test"]));
    assert_eq!(project["sourceRoots"], json!(["src/main/kotlin"]));
    assert!(!project["compileClasspath"].as_array().unwrap().is_empty());
    assert_eq!(project["compilerPlugins"].as_array().unwrap().len(), 1);
    assert_eq!(
        project["compilerPluginOptions"],
        json!(["plugin:org.jetbrains.kotlin.allopen:annotation=com.acme.archive.OpenForTesting"])
    );
    assert!(
        project["modelInputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|input| input["path"] == "pom.xml")
    );

    worker.shutdown().unwrap();
}

#[test]
fn invalidates_maven_project_snapshot_when_pom_changes() {
    let root = workspace_root();
    let temporary = tempfile::tempdir().unwrap();
    let fixture = temporary.path().join("maven-model-invalidation");
    copy_maven_fixture(&root.join("fixtures/kotlin-maven"), &fixture);
    let mut worker = WorkerClient::start(&root).unwrap();
    let before = worker
        .request(RequestKind::OpenProject, &json!({"repo": fixture}))
        .unwrap();
    let pom = fixture.join("pom.xml");
    let changed = std::fs::read_to_string(&pom).unwrap().replace(
        "<java.version>21</java.version>",
        "<java.version>17</java.version>",
    );
    std::fs::write(&pom, changed).unwrap();

    let after = worker
        .request(RequestKind::OpenProject, &json!({"repo": fixture}))
        .unwrap();

    assert_eq!(after["jvmTarget"], "17");
    assert_ne!(before["projectModelHash"], after["projectModelHash"]);
    worker.shutdown().unwrap();
}

#[test]
fn indexes_and_resolves_maven_sources_with_k2() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();

    let index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo": fixture, "compilation": ":/main"}),
        )
        .unwrap();
    assert_eq!(index["analysisMode"], "K2_SEMANTIC");
    assert_eq!(index["k2Validated"], true, "{:#}", index["diagnostics"]);
    assert!(
        index["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "src/main/kotlin/com/acme/archive/ArchiveService.kt")
    );

    let resolved = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({
                "repo": fixture,
                "compilation": ":/main",
                "symbol": "com.acme.archive.ArchiveService.archiveEvent"
            }),
        )
        .unwrap();
    assert_eq!(resolved["declaration"]["name"], "archiveEvent");
    assert_eq!(resolved["k2Validated"], true);

    let test_index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo": fixture, "compilation": ":/test"}),
        )
        .unwrap();
    assert_eq!(
        test_index["k2Validated"], true,
        "{:#}",
        test_index["diagnostics"]
    );
    assert!(
        test_index["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "src/test/kotlin/com/acme/archive/ArchiveServiceTest.kt")
    );
    assert!(
        test_index["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| !file["path"].as_str().unwrap().starts_with("src/main/")),
        "main files must not be published as test-source declarations"
    );

    worker.shutdown().unwrap();
}

#[test]
fn indexes_compiler_derived_declaration_descriptors_on_kotlin_23_maven() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    let verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let index = worker.inspect_verified_index(&verified).unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(&fixture, Some(":/main")).unwrap();
    repository_index
        .update_verified(&verified, &worker)
        .unwrap();
    assert!(
        repository_index
            .declaration_descriptors()
            .unwrap()
            .is_some()
    );
    let repeated_verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let repeated = worker.inspect_verified_index(&repeated_verified).unwrap();

    assert_eq!(index["k2Validated"], true);
    let graph = &index["declarationDescriptors"];
    assert_eq!(graph["schema"], "declaration-descriptor-graph/0.1");
    assert_eq!(graph["compilation"], ":/main");
    assert_eq!(graph["provenance"]["provider"], "COMPILER_SEMANTIC_FACTS");
    assert_eq!(graph["provenance"]["compilerVersion"], "2.3.0");
    assert_eq!(
        graph["provenance"]["projectModelHash"],
        project["projectModelHash"]
    );
    assert_eq!(
        graph["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.6"
    );
    assert!(
        graph["provenance"]["pluginArtifactFingerprint"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:"))
    );
    assert_eq!(
        index["declarationDescriptorHash"],
        repeated["declarationDescriptorHash"]
    );
    assert_eq!(graph, &repeated["declarationDescriptors"]);
    let rows = declaration_descriptor_rows(index);
    assert!(!rows.is_empty());
    let canonical_rows = rows.iter().map(Value::to_string).collect::<Vec<_>>();
    assert!(canonical_rows.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(rows.iter().all(|descriptor| {
        descriptor["resolution"] == "PROVEN"
            && descriptor["provider"] == "K2_FIR"
            && descriptor["module"] == ":"
            && descriptor["sourceSet"] == "main"
            && descriptor["sourceProvenance"] == "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
            && descriptor["compilerAuthority"] == "fir-facts-extractor/0.6"
            && descriptor["symbolIdentity"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["declarationKind"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["ownerIdentity"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["containment"].is_array()
            && descriptor["effectiveVisibility"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["modality"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor.get("sourceText").is_none()
    }));
    let by_callable = |needle: &str| {
        rows.iter()
            .filter(|descriptor| {
                descriptor["compilerCallableId"]
                    .as_str()
                    .is_some_and(|identity| identity.contains(needle))
            })
            .collect::<Vec<_>>()
    };
    assert!(by_callable("publicDescriptor").iter().any(|descriptor| {
        descriptor["visibility"] == "public"
            && descriptor["exportBoundary"] == "PUBLIC_API"
            && descriptor["declarationKind"] == "FUNCTION"
            && descriptor["ownerIdentity"] == "package:com.acme.relations"
            && descriptor["containment"]
                .as_array()
                .is_some_and(Vec::is_empty)
            && descriptor["modality"] == "FINAL"
            && descriptor["symbolIdentity"]
                .as_str()
                .is_some_and(|identity| {
                    identity.starts_with(&format!(
                        "callable:{}#jvm:",
                        descriptor["compilerCallableId"].as_str().unwrap()
                    ))
                })
            && descriptor["parameterTypes"][0]["type"] == "kotlin/String"
            && descriptor["parameterTypes"][0]["nullable"] == false
            && descriptor["returnType"] == "kotlin/String?"
            && descriptor["returnNullable"] == true
    }));
    assert!(by_callable("internalDescriptor").iter().any(|descriptor| {
        descriptor["visibility"] == "internal"
            && descriptor["exportBoundary"] == "MODULE_API"
            && descriptor["parameterTypes"][0]["nullable"] == true
    }));
    assert!(by_callable("privateDescriptor").iter().any(|descriptor| {
        descriptor["visibility"] == "private" && descriptor["exportBoundary"] == "PRIVATE_API"
    }));
    let overloads = by_callable("overloadedDescriptor");
    assert_eq!(overloads.len(), 2);
    assert_ne!(
        overloads[0]["symbolIdentity"],
        overloads[1]["symbolIdentity"]
    );
    assert!(overloads.iter().any(|descriptor| {
        descriptor["parameterTypes"][0]["nullable"] == true && descriptor["returnNullable"] == true
    }));
    assert!(by_callable("genericDescriptor").iter().any(|descriptor| {
        descriptor["typeParameters"]
            .as_array()
            .is_some_and(|items| {
                items.len() == 1
                    && items[0]["index"] == 0
                    && items[0]["compilerName"] == "T"
                    && items[0]["bounds"].as_array().is_some_and(|bounds| {
                        bounds.iter().any(|bound| bound == "kotlin/CharSequence")
                    })
            })
    }));
    assert!(by_callable("IntegerSource.read").iter().any(|descriptor| {
        descriptor["isOverride"] == true
            && descriptor["returnType"] == "kotlin/Int"
            && descriptor["ownerIdentity"] == "class:com/acme/relations/IntegerSource"
            && descriptor["containment"].as_array().is_some_and(|owners| {
                owners
                    .iter()
                    .any(|owner| owner == "class:com/acme/relations/IntegerSource")
            })
    }));
    assert!(by_callable("NumericSource.read").iter().any(|descriptor| {
        descriptor["isOverride"] == false && descriptor["returnType"] == "kotlin/Number"
    }));
    assert!(
        by_callable("LexicalDecoy.read")
            .iter()
            .all(|descriptor| descriptor["isOverride"] == false)
    );
    let override_relation = declaration_relation_rows(index, "OVERRIDES")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("IntegerSource.read"))
                && relation["target"]
                    .as_str()
                    .is_some_and(|target| target.contains("NumericSource.read"))
        })
        .unwrap();
    assert!(
        by_callable(override_relation["owner"].as_str().unwrap())
            .iter()
            .any(|descriptor| { descriptor["isOverride"] == true })
    );
    assert!(
        by_callable(override_relation["target"].as_str().unwrap())
            .iter()
            .any(|descriptor| { descriptor["isOverride"] == false })
    );
    assert!(by_callable("RelationState.field").iter().any(|descriptor| {
        descriptor["declarationKind"] == "MUTABLE_PROPERTY"
            && descriptor["declaredType"] == "kotlin/String"
            && descriptor["declaredNullable"] == false
            && descriptor["ownerIdentity"] == "class:com/acme/relations/RelationState"
    }));
    assert!(
        graph["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["resolution"] == "UNKNOWN"
                    && matches!(
                        boundary["code"].as_str(),
                        Some(
                            "GENERATED_OR_NO_SOURCE"
                                | "LOCAL_DECLARATION_UNSUPPORTED"
                                | "LOCAL_GENERATED_OR_NO_SOURCE"
                        )
                    )
                    && boundary["provider"] == "K2_FIR"
                    && boundary["module"] == ":"
                    && boundary["sourceSet"] == "main"
                    && boundary["compilerAuthority"] == "fir-facts-extractor/0.6"
            })
    );
    worker.shutdown().unwrap();
}

#[test]
fn indexes_compiler_derived_declaration_relations_on_kotlin_23() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.3.0");
    assert_eq!(project["workerCompilerVersion"], "2.3.0");
    assert_eq!(worker.capabilities.compiler_version, "2.3.0");
    let verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let index = worker.inspect_verified_index(&verified).unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(&fixture, Some(":/main")).unwrap();
    repository_index
        .update_verified(&verified, &worker)
        .unwrap();
    assert!(repository_index.declaration_relations().unwrap().is_some());
    let repeated_verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let repeated = worker.inspect_verified_index(&repeated_verified).unwrap();

    assert_eq!(index["k2Validated"], true, "{:#}", index["diagnostics"]);
    let graph = &index["declarationRelations"];
    assert_eq!(graph["schema"], "declaration-relation-graph/0.1");
    assert_eq!(graph["compilation"], ":/main");
    assert_eq!(graph["provenance"]["provider"], "COMPILER_SEMANTIC_FACTS");
    assert_eq!(graph["provenance"]["compilerVersion"], "2.3.0");
    assert_eq!(
        graph["provenance"]["projectModelHash"],
        project["projectModelHash"]
    );
    assert_eq!(
        index["declarationRelationHash"],
        repeated["declarationRelationHash"]
    );
    assert_eq!(graph, &repeated["declarationRelations"]);
    let serialized = graph["relations"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>();
    assert!(serialized.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(
        graph["relations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|relation| {
                relation["file"]
                    .as_str()
                    .is_some_and(|file| !file.starts_with('/') && !file.contains(".."))
                    && relation["resolution"] == "PROVEN"
                    && relation["provider"] == "K2_FIR"
                    && relation.get("sourceText").is_none()
            })
    );

    let overrides = declaration_relation_rows(index, "OVERRIDES");
    assert!(overrides.iter().any(|relation| {
        relation["owner"]
            .as_str()
            .unwrap_or_default()
            .contains("IntegerSource.read")
            && relation["target"]
                .as_str()
                .unwrap_or_default()
                .contains("NumericSource.read")
            && relation["sourceReturnType"] == "kotlin/Int"
            && relation["baseReturnType"] == "kotlin/Number"
    }));
    assert!(overrides.iter().all(|relation| {
        !relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("LexicalDecoy")
    }));
    for kind in [
        "CALLS",
        "REFERENCES",
        "CONSTRUCTS",
        "READS",
        "WRITES",
        "INITIALIZES",
    ] {
        assert!(
            !declaration_relation_rows(index, kind).is_empty(),
            "missing {kind}"
        );
    }
    let argument_mapping_unavailable =
        graph["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["stage"] == "OPTIONAL_RELATION_EVIDENCE"
                    && boundary["code"] == "ARGUMENT_MAPPING_UNAVAILABLE"
            });
    let calls = declaration_relation_rows(index, "CALLS");
    let call_source = calls
        .iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .unwrap_or_default()
                .contains("callSource")
                && relation["target"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("NumericSource.read")
        })
        .expect("compiler must retain the exact callSource target");
    if let Some(arguments) = call_source["argumentToParameter"].as_array() {
        assert!(!arguments.is_empty());
    } else {
        assert!(argument_mapping_unavailable);
    }
    let reordered = declaration_relation_rows(index, "CALLS")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("callEqualTypesByName"))
                && relation["target"]
                    .as_str()
                    .is_some_and(|target| target.contains("combineEqualTypes"))
        })
        .expect("compiler must emit the reversed named-argument call");
    if let Some(arguments) = reordered["argumentToParameter"].as_array() {
        assert_eq!(
            arguments
                .iter()
                .map(|argument| argument["parameterIndex"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 0],
        );
    } else {
        assert!(argument_mapping_unavailable);
    }
    assert!(
        declaration_relation_rows(index, "CONSTRUCTS")
            .iter()
            .any(|relation| {
                relation["target"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("Envelope")
            })
    );
    assert!(
        declaration_relation_rows(index, "WRITES")
            .iter()
            .any(|relation| {
                relation["target"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("RelationState.field")
                    && relation["orderKey"].as_i64().is_some()
            })
    );
    assert!(
        declaration_relation_rows(index, "INITIALIZES")
            .iter()
            .any(|relation| {
                relation["target"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("RelationState.initial")
                    && relation["orderProvenance"] == "FIR_SOURCE_RANGE"
            })
    );
    let boundary_codes = graph["boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|boundary| boundary["code"].as_str())
        .collect::<Vec<_>>();
    assert!(boundary_codes.contains(&"NON_FUNCTION_OVERRIDE_UNSUPPORTED"));
    assert!(boundary_codes.contains(&"DYNAMIC_REFLECTION_BOUNDARY"));
    assert!(boundary_codes.contains(&"EXTERNAL_OR_LOCAL_ARGUMENT_TARGET"));
    for relation in declaration_relation_rows(index, "CALLS")
        .into_iter()
        .filter(|relation| {
            relation["target"]
                .as_str()
                .is_some_and(|target| target.starts_with("java/lang/reflect/"))
        })
    {
        assert_eq!(relation["attributeCoverage"], "PARTIAL");
        assert!(
            graph["boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|boundary| {
                    boundary["code"] == "UNRESOLVED_RELATION_TYPE"
                        && boundary["relationKind"] == "CALLS"
                        && boundary["owner"] == relation["owner"]
                        && boundary["target"] == relation["target"]
                        && boundary["retainedRelationHash"]
                            .as_str()
                            .is_some_and(|digest| digest.starts_with("sha256:"))
                })
        );
    }

    let temporary = tempfile::tempdir().unwrap();
    let unresolved_fixture = temporary.path().join("kotlin-maven-relations");
    copy_maven_fixture(&fixture, &unresolved_fixture);
    let source = unresolved_fixture.join("src/main/kotlin/com/acme/relations/RelationFacts.kt");
    let mut content = std::fs::read_to_string(&source).unwrap();
    content.push_str("\nfun unresolvedRelation(): String = missingCompilerTarget()\n");
    std::fs::write(source, content).unwrap();
    worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":unresolved_fixture,"compilation":":/main"}),
        )
        .unwrap();
    let unresolved = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":unresolved_fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    assert_eq!(unresolved["k2Validated"], false);
    if let Some(relation_graph) = unresolved["declarationRelations"].as_object() {
        assert!(
            relation_graph["boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|boundary| boundary["code"] == "UNRESOLVED_CALLABLE_TARGET")
        );
    } else {
        assert_eq!(unresolved["declarationRelations"], json!([]));
        assert_eq!(unresolved["declarationDescriptors"], json!([]));
        assert!(
            unresolved.get("semanticFacts").is_none()
                || unresolved["semanticFacts"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
        );
    }

    worker.shutdown().unwrap();
}

#[test]
fn agent_context_renders_maven_targeted_test_command() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    support::seed_build_caches(&fixture);
    let temporary = tempfile::tempdir().unwrap();
    let evidence = temporary.path().join("maven-evidence.json");

    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "agent-context",
            "--repo",
            fixture.to_str().unwrap(),
            "--term",
            "archiveEvent",
            "--term",
            "ArchiveService",
            "--intent",
            "Archive event must expose typed id/code/title payload and preserve Maven tests",
            "--max-bytes",
            "16384",
            "--evidence",
            evidence.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    let published_task_index = RepositoryIndex::open_compilation(&fixture, Some(":/main"))
        .unwrap()
        .hash()
        .unwrap();
    assert_eq!(
        published_task_index.as_deref(),
        context["snapshot"]["indexSnapshot"].as_str()
    );
    assert_eq!(context["validationPlan"]["buildSystem"], "MAVEN");
    assert_eq!(
        context["validationPlan"]["targetedArgs"],
        json!(["-Dtest=ArchiveServiceTest", "test"])
    );
    assert!(
        context["validationPlan"]["targetedArgs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| !command.as_str().unwrap().contains("gradlew"))
    );
    assert!(
        context["editSurfaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|surface| surface["name"] == "archiveEvent")
    );
    assert!(
        context["contracts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|contract| {
                contract["name"] == "ProductIdentity"
                    && contract["sourceText"]
                        .as_str()
                        .is_some_and(|source| source.contains("code: String?"))
            })
    );
    assert!(
        context["tests"][0]["declarationTargetId"]
            .as_str()
            .is_some()
    );
}

#[cfg(unix)]
#[test]
fn fails_closed_when_neither_wrapper_nor_maven_is_executable() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = workspace_root();
    let temporary = tempfile::tempdir().unwrap();
    let fixture = temporary.path().join("maven-without-launcher");
    copy_maven_fixture(&root.join("fixtures/kotlin-maven"), &fixture);
    let wrapper = fixture.join("mvnw");
    let mut permissions = std::fs::metadata(&wrapper).unwrap().permissions();
    permissions.set_mode(0o644);
    std::fs::set_permissions(&wrapper, permissions).unwrap();
    let java_home = std::env::var("JAVA_HOME").expect("JAVA_HOME is required by the test suite");
    let launcher_bin = temporary.path().join("launcher-bin");
    std::fs::create_dir(&launcher_bin).unwrap();
    for command in ["ls", "sed", "tr", "uname", "xargs"] {
        let executable = ["/usr/bin", "/bin"]
            .into_iter()
            .map(|directory| std::path::Path::new(directory).join(command))
            .find(|candidate| candidate.is_file())
            .unwrap_or_else(|| panic!("{command} is required by the worker launcher"));
        symlink(executable, launcher_bin.join(command)).unwrap();
    }
    let restricted_path = format!("{}:{java_home}/bin", launcher_bin.display());

    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["project", "inspect", "--repo", fixture.to_str().unwrap()])
        .env("PATH", restricted_path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let response = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        response.contains("UNSUPPORTED_PROJECT_CONFIGURATION"),
        "{response}"
    );
    assert!(
        !response.contains("INCOMPLETE_SEMANTIC_ANALYSIS"),
        "{response}"
    );
}

#[test]
fn semantic_transaction_commits_structured_multifile_candidates_after_clean_maven_validation() {
    let root = workspace_root();
    let temporary = tempfile::tempdir().unwrap();
    let repo = init_maven_repo(temporary.path());
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo": repo}))
        .unwrap();
    let base = git_output(&repo, &["rev-parse", "refs/heads/main"]);
    let index_facts = worker
        .index_files_verified(&json!({"repo": repo, "compilation": ":/main"}))
        .unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(&repo, Some(":/main")).unwrap();
    let index_snapshot = repository_index
        .update_verified(&index_facts, &worker)
        .unwrap();
    let raw = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo": repo, "symbol": "com.acme.archive.ArchiveService.archiveEvent"}),
        )
        .unwrap();
    let graph = graph::enrich(serde_json::from_value::<LocalGraph>(raw).unwrap());
    let seed_id = graph
        .nodes
        .iter()
        .filter(|node| node.kind == "RETURN")
        .max_by_key(|node| {
            node.origin
                .as_ref()
                .and_then(|origin| origin.pointer("/rangeHint/1"))
                .and_then(Value::as_u64)
                .unwrap_or_default()
        })
        .unwrap()
        .id
        .clone();
    let thread = graph::slice(
        &graph,
        &seed_id,
        SlicePolicy::default(),
        Snapshot {
            base_revision: base.clone(),
            project_model_hash: project["projectModelHash"].as_str().unwrap().into(),
            compiler_version: "2.3.0".into(),
            build_system: BuildSystem::Maven,
            build_launcher: "./mvnw".into(),
            index_snapshot: index_snapshot.clone(),
            compilation: ":/main".into(),
            compile_task: "compile".into(),
            test_tasks: vec!["test".into()],
        },
        json!({
            "kind": "FUNCTION_RETURN",
            "symbol": "com.acme.archive.ArchiveService.archiveEvent",
            "nodeId": seed_id
        }),
    )
    .unwrap();
    let resolved = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo": repo, "symbol": "com.acme.archive.ArchiveService.archiveEvent"}),
        )
        .unwrap();
    let declaration_source =
        std::fs::read_to_string(repo.join("src/main/kotlin/com/acme/archive/ArchiveService.kt"))
            .unwrap();
    let old_declaration = "data class ProductIdentity(\n    val id: String,\n    val code: String?,\n    val title: String,\n)";
    assert!(declaration_source.contains(old_declaration));
    let edit = EditIr {
        schema: "semantic-edit/0.1".into(),
        thread_id: thread.thread_id.clone(),
        base_revision: base.clone(),
        operations: vec![
            EditOperation {
                op_id: "op:rewrite-declaration".into(),
                kind: "REWRITE_DECLARATION".into(),
                target: json!({
                    "fileId": "src/main/kotlin/com/acme/archive/ArchiveService.kt",
                    "ownerSymbolId": "com.acme.archive.ProductIdentity",
                    "syntaxKind": "KtClass",
                    "exactTextHash": canonical::hash_bytes(old_declaration.as_bytes()),
                }),
                replacement: Replacement {
                    kotlin: String::new(),
                },
                semantic_operation: None,
                preconditions: BTreeMap::from([(
                    "substitutions".into(),
                    json!([
                        {"old":"String", "new":"kotlin.String", "occurrence":2},
                        {"old":")", "new":") : java.io.Serializable"}
                    ]),
                )]),
                postconditions: BTreeMap::new(),
            },
            EditOperation {
                op_id: "op:replace-body-before-created-helper-exists".into(),
                kind: "REPLACE_FUNCTION_BODY".into(),
                target: resolved["bodyAnchor"].clone(),
                replacement: Replacement {
                    kotlin: "{ return formatArchive(product) }".into(),
                },
                semantic_operation: None,
                preconditions: BTreeMap::from([(
                    "nodeTextHash".into(),
                    resolved["bodyAnchor"]["exactTextHash"].clone(),
                )]),
                postconditions: BTreeMap::new(),
            },
            EditOperation {
                op_id: "op:create-production-helper".into(),
                kind: "CREATE_FILE".into(),
                target: json!({"fileId": "src/main/kotlin/com/acme/archive/ArchiveFormatter.kt"}),
                replacement: Replacement {
                    kotlin: "package com.acme.archive\n\ninternal fun formatArchive(product: ProductIdentity): String =\n    \"${product.id}:${product.code}:${product.title}\"\n".into(),
                },
                semantic_operation: None,
                preconditions: BTreeMap::new(),
                postconditions: BTreeMap::new(),
            },
            EditOperation {
                op_id: "op:create-test-source".into(),
                kind: "CREATE_FILE".into(),
                target: json!({"fileId": "src/test/kotlin/com/acme/archive/GeneratedArchiveMarker.kt"}),
                replacement: Replacement {
                    kotlin: "package com.acme.archive\n\ninternal class GeneratedArchiveMarker\n".into(),
                },
                semantic_operation: None,
                preconditions: BTreeMap::new(),
                postconditions: BTreeMap::new(),
            },
        ],
        expected_write_set: vec![],
    };
    let mut transaction = Transaction {
        schema: "semantic-transaction/0.1".into(),
        tx_id: "tx:maven".into(),
        actor_id: "test:maven".into(),
        intent: "maven validation".into(),
        base_revision: base,
        project_model_hash: project["projectModelHash"].as_str().unwrap().into(),
        base_index_snapshot: Some(index_snapshot),
        status: "CREATED".into(),
        thread,
        required_threads: vec![],
        edit,
        preview: None,
        expected_write_set_hash: None,
        actual_write_set_hash: None,
        validation_evidence: vec![],
        test_tasks: vec![],
        candidate_commit: None,
        final_commit: None,
        target_ref: None,
    };

    let committed =
        transaction::commit(&repo, &mut transaction, "refs/heads/main", &mut worker).unwrap();

    assert_eq!(committed["status"], "COMMITTED");
    assert!(
        transaction
            .validation_evidence
            .iter()
            .any(|evidence| { evidence["kind"] == "BUILD" && evidence["buildSystem"] == "MAVEN" })
    );
    let preview = transaction.preview.as_ref().unwrap();
    assert!(
        preview
            .candidates
            .contains_key("src/main/kotlin/com/acme/archive/ArchiveService.kt")
    );
    assert!(
        preview
            .candidates
            .contains_key("src/test/kotlin/com/acme/archive/GeneratedArchiveMarker.kt")
    );
    assert!(
        preview
            .candidates
            .contains_key("src/main/kotlin/com/acme/archive/ArchiveFormatter.kt")
    );
    assert!(
        preview
            .actual_write_set
            .iter()
            .any(|write| write.kind == "DECLARATION")
    );
    assert!(
        preview
            .actual_write_set
            .iter()
            .any(|write| write.kind == "FILE")
    );
    let committed_files = git_output(&repo, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(
        !committed_files
            .lines()
            .any(|file| file.starts_with("target/"))
    );
    let committed_source = git_output(
        &repo,
        &[
            "show",
            "HEAD:src/main/kotlin/com/acme/archive/ArchiveService.kt",
        ],
    );
    assert!(committed_source.contains("return formatArchive(product)"));
    assert!(committed_source.contains(") : java.io.Serializable"));
    assert!(committed_source.contains("val code: kotlin.String?"));
    let generated = git_output(
        &repo,
        &[
            "show",
            "HEAD:src/test/kotlin/com/acme/archive/GeneratedArchiveMarker.kt",
        ],
    );
    assert!(generated.contains("internal class GeneratedArchiveMarker"));
    assert_eq!(git_output(&repo, &["status", "--short"]), "");
    worker.shutdown().unwrap();
}
