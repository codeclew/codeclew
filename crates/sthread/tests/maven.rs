use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use sthread::canonical;
use sthread::graph;
use sthread::index::RepositoryIndex;
use sthread::model::{
    BuildSystem, EditIr, EditOperation, LocalGraph, Replacement, SlicePolicy, Snapshot, Transaction,
};
use sthread::proto::RequestKind;
use sthread::transaction;
use sthread::worker::{WorkerClient, workspace_root};

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

#[test]
fn opens_maven_kotlin_23_project_with_exact_worker_and_build_plan() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
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
fn agent_context_renders_maven_targeted_test_command() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-maven");
    let temporary = tempfile::tempdir().unwrap();
    let evidence = temporary.path().join("maven-evidence.json");

    let output = Command::new(env!("CARGO_BIN_EXE_sthread"))
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
    use std::os::unix::fs::PermissionsExt;

    let root = workspace_root();
    let temporary = tempfile::tempdir().unwrap();
    let fixture = temporary.path().join("maven-without-launcher");
    copy_maven_fixture(&root.join("fixtures/kotlin-maven"), &fixture);
    let wrapper = fixture.join("mvnw");
    let mut permissions = std::fs::metadata(&wrapper).unwrap().permissions();
    permissions.set_mode(0o644);
    std::fs::set_permissions(&wrapper, permissions).unwrap();
    let java_home = std::env::var("JAVA_HOME").expect("JAVA_HOME is required by the test suite");
    let restricted_path = format!("{java_home}/bin:/usr/bin:/bin");

    let output = Command::new(env!("CARGO_BIN_EXE_sthread"))
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
        .request(
            RequestKind::IndexFiles,
            &json!({"repo": repo, "compilation": ":/main"}),
        )
        .unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(&repo, Some(":/main")).unwrap();
    let index_snapshot = repository_index.update(&index_facts).unwrap();
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
                preconditions: BTreeMap::new(),
                postconditions: BTreeMap::new(),
            },
            EditOperation {
                op_id: "op:create-production-helper".into(),
                kind: "CREATE_FILE".into(),
                target: json!({"fileId": "src/main/kotlin/com/acme/archive/ArchiveFormatter.kt"}),
                replacement: Replacement {
                    kotlin: "package com.acme.archive\n\ninternal fun formatArchive(product: ProductIdentity): String =\n    \"${product.id}:${product.code}:${product.title}\"\n".into(),
                },
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
