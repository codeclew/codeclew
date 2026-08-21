use clew::error::ErrorCode;
use clew::graph;
use clew::index::RepositoryIndex;
use clew::model::{
    EditIr, EditOperation, LocalGraph, Replacement, SlicePolicy, Snapshot, Transaction,
};
use clew::proto::RequestKind;
use clew::transaction;
use clew::worker::{WorkerClient, workspace_root};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

mod support;

fn copy_fixture(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from).into_iter().map(Result::unwrap) {
        let relative = entry.path().strip_prefix(from).unwrap();
        if relative.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some(".gradle" | "build" | ".semantic-thread")
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let wrapper = to.join("gradlew");
        let mut permissions = std::fs::metadata(&wrapper).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(wrapper, permissions).unwrap();
    }
    support::seed_build_caches(to);
}

fn init_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    copy_fixture(&workspace_root().join("fixtures/kotlin-basic"), &repo);
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
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn make_tx(
    worker: &mut WorkerClient,
    repo: &Path,
    project_hash: &str,
    symbol: &str,
    replacement: &str,
    id: &str,
) -> Transaction {
    let base = git_output(repo, &["rev-parse", "refs/heads/main"]);
    let index_facts = worker
        .index_files_verified(&json!({"repo":repo,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(repo, Some(":/main")).unwrap();
    let index_snapshot = repository_index
        .update_verified(&index_facts, worker)
        .unwrap();
    let raw = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":repo,"symbol":symbol}),
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
                .and_then(|value| value.as_u64())
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
            project_model_hash: project_hash.into(),
            compiler_version: "2.4.10".into(),
            build_system: clew::model::BuildSystem::Gradle,
            build_launcher: "./gradlew".into(),
            index_snapshot: index_snapshot.clone(),
            compilation: ":/main".into(),
            compile_task: ":compileKotlin".into(),
            // Concurrency fixtures exercise publication, not project test
            // behavior. Default configured tests are covered independently.
            test_tasks: vec![],
        },
        json!({"kind":"FUNCTION_RETURN","symbol":symbol,"nodeId":seed_id}),
    )
    .unwrap();
    let resolved = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":repo,"symbol":symbol}),
        )
        .unwrap();
    let edit = EditIr {
        schema: "semantic-edit/0.1".into(),
        thread_id: thread.thread_id.clone(),
        base_revision: base.clone(),
        operations: vec![EditOperation {
            op_id: format!("op:{id}"),
            kind: "REPLACE_FUNCTION_BODY".into(),
            target: resolved["bodyAnchor"].clone(),
            replacement: Replacement {
                kotlin: replacement.into(),
            },
            semantic_operation: None,
            preconditions: BTreeMap::new(),
            postconditions: BTreeMap::new(),
        }],
        expected_write_set: vec![],
    };
    Transaction {
        schema: "semantic-transaction/0.1".into(),
        tx_id: format!("tx:{id}"),
        actor_id: "test:concurrency".into(),
        intent: id.into(),
        base_revision: base,
        project_model_hash: project_hash.into(),
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
    }
}

fn make_import_tx(
    worker: &mut WorkerClient,
    repo: &Path,
    project_hash: &str,
    id: &str,
) -> Transaction {
    let mut transaction = make_tx(
        worker,
        repo,
        project_hash,
        "com.acme.total",
        "{ return base }",
        id,
    );
    transaction.edit.operations = vec![EditOperation {
        op_id: format!("op:{id}"),
        kind: "ADD_IMPORT".into(),
        target: json!({"fileId":"src/main/kotlin/com/acme/Samples.kt","sourceText":""}),
        replacement: Replacement {
            kotlin: "java.time.Instant".into(),
        },
        semantic_operation: None,
        preconditions: BTreeMap::new(),
        postconditions: BTreeMap::new(),
    }];
    transaction
}

#[test]
fn unproven_edit_is_rejected_before_publication() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let repo = init_repo(temp.path(), "unproven-edit");
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":repo}))
        .unwrap();
    let hash = project["projectModelHash"].as_str().unwrap();
    let mut transaction = make_tx(
        &mut worker,
        &repo,
        hash,
        "com.acme.total",
        "{ var value = base; if (premium) value = value + value; return value }",
        "unproven",
    );
    let before_ref = git_output(&repo, &["rev-parse", "refs/heads/main"]);
    let before_worktrees = git_output(&repo, &["worktree", "list", "--porcelain"]);
    let error =
        transaction::commit(&repo, &mut transaction, "refs/heads/main", &mut worker).unwrap_err();
    assert_eq!(error.code, ErrorCode::BindingChanged, "{error:?}");
    assert_eq!(
        git_output(&repo, &["rev-parse", "refs/heads/main"]),
        before_ref
    );
    assert_eq!(
        git_output(&repo, &["worktree", "list", "--porcelain"]),
        before_worktrees
    );
    worker.shutdown().unwrap();
}

#[test]
fn unproven_required_thread_edit_is_rejected_before_publication() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let repo = init_repo(temp.path(), "goal-wide-read-set");
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":repo}))
        .unwrap();
    let project_hash = project["projectModelHash"].as_str().unwrap();

    // The edit is anchored in `total`, while the second required semantic root
    // is `namedCall`. A concurrent change to `decorate` affects only that
    // second root's ReadSet.
    let mut goal = make_tx(
        &mut worker,
        &repo,
        project_hash,
        "com.acme.total",
        "{ var value = base; if (premium) { value *= 2 }; return value }",
        "goal-wide",
    );
    let dependent_root = make_tx(
        &mut worker,
        &repo,
        project_hash,
        "com.acme.namedCall",
        "{ return value.decorate(prefix = \"{\") }",
        "required-root",
    );
    goal.required_threads = vec![goal.thread.clone(), dependent_root.thread.clone()];
    let second_thread_id = dependent_root.thread.thread_id;
    let mut legacy_wire = serde_json::to_value(&goal).unwrap();
    legacy_wire
        .as_object_mut()
        .unwrap()
        .remove("requiredThreads");
    let legacy: Transaction = serde_json::from_value(legacy_wire).unwrap();
    assert!(legacy.required_threads.is_empty());

    let mut concurrent = make_tx(
        &mut worker,
        &repo,
        project_hash,
        "String.com.acme.decorate",
        "{ return \"$prefix$this]\" }",
        "concurrent-callee",
    );
    let before_ref = git_output(&repo, &["rev-parse", "refs/heads/main"]);
    let error =
        transaction::commit(&repo, &mut concurrent, "refs/heads/main", &mut worker).unwrap_err();
    assert_eq!(error.code, ErrorCode::BindingChanged, "{error:?}");
    assert_eq!(
        git_output(&repo, &["rev-parse", "refs/heads/main"]),
        before_ref
    );

    let error = transaction::commit(&repo, &mut goal, "refs/heads/main", &mut worker).unwrap_err();
    assert_eq!(error.code, ErrorCode::BindingChanged, "{error:?}");
    assert_eq!(
        git_output(&repo, &["rev-parse", "refs/heads/main"]),
        before_ref
    );
    assert!(!second_thread_id.is_empty());
    worker.shutdown().unwrap();
}

#[test]
fn semantic_binding_failure_precedes_configured_tests_and_blocks_publication() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let repo = init_repo(temp.path(), "default-tests");
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":repo}))
        .unwrap();
    let hash = project["projectModelHash"].as_str().unwrap();
    let mut transaction = make_tx(
        &mut worker,
        &repo,
        hash,
        "com.acme.classify",
        "{ return if (value < 0) \"below\" else if (value == 0) \"zero\" else \"positive\" }",
        "default-tests",
    );
    transaction.thread.snapshot.test_tasks = vec![":test".into()];
    assert!(transaction.test_tasks.is_empty());
    let before_ref = git_output(&repo, &["rev-parse", "refs/heads/main"]);
    let before_worktrees = git_output(&repo, &["worktree", "list", "--porcelain"]);
    let before_index = RepositoryIndex::open_compilation(&repo, Some(":/main"))
        .unwrap()
        .hash()
        .unwrap();

    let error =
        transaction::commit(&repo, &mut transaction, "refs/heads/main", &mut worker).unwrap_err();
    assert_eq!(error.code, ErrorCode::BindingChanged, "{error:?}");
    assert_eq!(
        git_output(&repo, &["rev-parse", "refs/heads/main"]),
        before_ref
    );
    assert_eq!(
        git_output(&repo, &["worktree", "list", "--porcelain"]),
        before_worktrees
    );
    assert_eq!(
        RepositoryIndex::open_compilation(&repo, Some(":/main"))
            .unwrap()
            .hash()
            .unwrap(),
        before_index
    );
    worker.shutdown().unwrap();
}

#[test]
fn callee_formatting_requires_reslice_and_same_import_merges_idempotently() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let mut worker = WorkerClient::start(&root).unwrap();

    let formatting_repo = init_repo(temp.path(), "formatting");
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":formatting_repo}))
        .unwrap();
    let hash = project["projectModelHash"].as_str().unwrap();
    let mut caller = make_tx(
        &mut worker,
        &formatting_repo,
        hash,
        "com.acme.namedCall",
        "{ return value.decorate(prefix = \"(\") }",
        "formatting-caller",
    );
    let source_path = formatting_repo.join("src/main/kotlin/com/acme/Samples.kt");
    let source = std::fs::read_to_string(&source_path).unwrap();
    std::fs::write(
        &source_path,
        source.replace(
            "= \"$prefix$this]\"",
            "= /* formatting only */ \"$prefix$this]\"",
        ),
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "src/main/kotlin/com/acme/Samples.kt"])
            .current_dir(&formatting_repo)
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
                "callee formatting"
            ])
            .current_dir(&formatting_repo)
            .status()
            .unwrap()
            .success()
    );
    let error = transaction::commit(
        &formatting_repo,
        &mut caller,
        "refs/heads/main",
        &mut worker,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleRequiresReslice, "{error:?}");

    let import_repo = init_repo(temp.path(), "imports");
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":import_repo}))
        .unwrap();
    let hash = project["projectModelHash"].as_str().unwrap();
    let mut first = make_import_tx(&mut worker, &import_repo, hash, "import-first");
    transaction::commit(&import_repo, &mut first, "refs/heads/main", &mut worker).unwrap();
    let candidate = first.candidate_commit.clone().unwrap();
    let candidate_parent = git_output(&import_repo, &["rev-parse", &format!("{candidate}^")]);
    assert!(
        Command::new("git")
            .args([
                "update-ref",
                "refs/heads/main",
                &candidate_parent,
                &candidate
            ])
            .current_dir(&import_repo)
            .status()
            .unwrap()
            .success()
    );
    let mut pre_cas = first.clone();
    pre_cas.status = "COMMITTING".into();
    pre_cas.final_commit = None;
    transaction::ledger(&import_repo)
        .unwrap()
        .append(
            &pre_cas,
            "simulated crash after candidate commit before CAS",
        )
        .unwrap();
    let recovered_pre_cas = transaction::ledger(&import_repo)
        .unwrap()
        .inspect("tx:import-first")
        .unwrap();
    assert_eq!(recovered_pre_cas["reconciledStatus"], "COMMITTED");
    assert_eq!(
        recovered_pre_cas["recoveryAction"],
        "RECOVERED_COMMITTED_CANDIDATE_CAS"
    );
    assert_eq!(
        git_output(&import_repo, &["rev-parse", "refs/heads/main"]),
        candidate
    );
    let mut interrupted = first.clone();
    interrupted.status = "COMMITTING".into();
    transaction::ledger(&import_repo)
        .unwrap()
        .append(&interrupted, "simulated crash after candidate publication")
        .unwrap();
    let recovered = transaction::ledger(&import_repo)
        .unwrap()
        .inspect("tx:import-first")
        .unwrap();
    assert_eq!(recovered["reconciledStatus"], "COMMITTED");
    assert_eq!(
        recovered["recoveryAction"],
        "RECOVERED_COMMITTED_FROM_TRAILER"
    );
    let before_second = git_output(&import_repo, &["rev-parse", "refs/heads/main"]);
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":import_repo}))
        .unwrap();
    let current_hash = project["projectModelHash"].as_str().unwrap();
    let mut second = make_import_tx(&mut worker, &import_repo, current_hash, "import-second");
    let result =
        transaction::commit(&import_repo, &mut second, "refs/heads/main", &mut worker).unwrap();
    assert_eq!(result["idempotent"], true);
    assert_eq!(
        before_second,
        git_output(&import_repo, &["rev-parse", "refs/heads/main"])
    );
    let source = git_output(
        &import_repo,
        &[
            "show",
            "refs/heads/main:src/main/kotlin/com/acme/Samples.kt",
        ],
    );
    assert_eq!(source.matches("import java.time.Instant").count(), 1);

    worker.shutdown().unwrap();
}
