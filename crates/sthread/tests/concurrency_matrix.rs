use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use sthread::error::ErrorCode;
use sthread::graph;
use sthread::index::RepositoryIndex;
use sthread::model::{
    EditIr, EditOperation, LocalGraph, Replacement, SlicePolicy, Snapshot, Transaction,
};
use sthread::proto::RequestKind;
use sthread::transaction;
use sthread::worker::{WorkerClient, workspace_root};

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
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":repo,"compilation":":/main"}),
        )
        .unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(repo, Some(":/main")).unwrap();
    let index_snapshot = repository_index.update(&index_facts).unwrap();
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
            build_system: sthread::model::BuildSystem::Gradle,
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
        preconditions: BTreeMap::new(),
        postconditions: BTreeMap::new(),
    }];
    transaction
}

#[test]
fn mandatory_concurrency_matrix() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let mut worker = WorkerClient::start(&root).unwrap();

    let repo_ab = init_repo(temp.path(), "ab");
    let project_ab = worker
        .request(RequestKind::OpenProject, &json!({"repo":repo_ab}))
        .unwrap();
    let hash_ab = project_ab["projectModelHash"].as_str().unwrap();
    let mut a = make_tx(
        &mut worker,
        &repo_ab,
        hash_ab,
        "com.acme.total",
        "{ var value = base; if (premium) value = value + value; return value }",
        "a",
    );
    let mut same_target = make_tx(
        &mut worker,
        &repo_ab,
        hash_ab,
        "com.acme.total",
        "{ var value = base; if (premium) value = value * 3; return value }",
        "same-target",
    );
    let mut b = make_tx(
        &mut worker,
        &repo_ab,
        hash_ab,
        "com.acme.classify",
        "{ return if (value < 0) \"below\" else if (value == 0) \"zero\" else \"positive\" }",
        "b",
    );
    let mut callee = make_tx(
        &mut worker,
        &repo_ab,
        hash_ab,
        "String.com.acme.decorate",
        "{ return \"$prefix$this]!\" }",
        "callee",
    );
    let mut caller = make_tx(
        &mut worker,
        &repo_ab,
        hash_ab,
        "com.acme.namedCall",
        "{ return value.decorate(prefix = \"(\") }",
        "caller",
    );
    transaction::commit(&repo_ab, &mut a, "refs/heads/main", &mut worker).unwrap();
    let conflict = transaction::commit(&repo_ab, &mut same_target, "refs/heads/main", &mut worker)
        .unwrap_err();
    assert_eq!(conflict.code, ErrorCode::WwConflict, "{conflict:?}");
    transaction::commit(&repo_ab, &mut b, "refs/heads/main", &mut worker).unwrap();
    let source_ab_independent = git_output(
        &repo_ab,
        &[
            "show",
            "refs/heads/main:src/main/kotlin/com/acme/Samples.kt",
        ],
    );
    transaction::commit(&repo_ab, &mut callee, "refs/heads/main", &mut worker).unwrap();
    let stale =
        transaction::commit(&repo_ab, &mut caller, "refs/heads/main", &mut worker).unwrap_err();
    assert_eq!(stale.code, ErrorCode::StaleRequiresReslice);
    let source_ab = git_output(
        &repo_ab,
        &[
            "show",
            "refs/heads/main:src/main/kotlin/com/acme/Samples.kt",
        ],
    );
    assert!(source_ab.contains("value = value + value"));
    assert!(source_ab.contains("\"below\""));

    let repo_ba = init_repo(temp.path(), "ba");
    let project_ba = worker
        .request(RequestKind::OpenProject, &json!({"repo":repo_ba}))
        .unwrap();
    let hash_ba = project_ba["projectModelHash"].as_str().unwrap();
    let mut reverse_a = make_tx(
        &mut worker,
        &repo_ba,
        hash_ba,
        "com.acme.total",
        "{ var value = base; if (premium) value = value + value; return value }",
        "reverse-a",
    );
    let mut reverse_b = make_tx(
        &mut worker,
        &repo_ba,
        hash_ba,
        "com.acme.classify",
        "{ return if (value < 0) \"below\" else if (value == 0) \"zero\" else \"positive\" }",
        "reverse-b",
    );
    transaction::commit(&repo_ba, &mut reverse_b, "refs/heads/main", &mut worker).unwrap();
    transaction::commit(&repo_ba, &mut reverse_a, "refs/heads/main", &mut worker).unwrap();
    let source_ba = git_output(
        &repo_ba,
        &[
            "show",
            "refs/heads/main:src/main/kotlin/com/acme/Samples.kt",
        ],
    );
    assert_eq!(source_ab_independent, source_ba);

    let repo_model = init_repo(temp.path(), "model");
    let project_model = worker
        .request(RequestKind::OpenProject, &json!({"repo":repo_model}))
        .unwrap();
    let hash_model = project_model["projectModelHash"].as_str().unwrap();
    let mut old_tx = make_tx(
        &mut worker,
        &repo_model,
        hash_model,
        "com.acme.total",
        "{ return base }",
        "old-model",
    );
    let build = repo_model.join("build.gradle.kts");
    let contents = std::fs::read_to_string(&build).unwrap();
    std::fs::write(&build, format!("{contents}\n// project model changed\n")).unwrap();
    assert!(
        Command::new("git")
            .args(["add", "build.gradle.kts"])
            .current_dir(&repo_model)
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
                "model change"
            ])
            .current_dir(&repo_model)
            .status()
            .unwrap()
            .success()
    );
    let stale =
        transaction::commit(&repo_model, &mut old_tx, "refs/heads/main", &mut worker).unwrap_err();
    assert_eq!(stale.code, ErrorCode::StaleRequiresReslice);
    worker.shutdown().unwrap();
}

#[test]
fn configured_snapshot_tests_run_by_default_and_block_publication() {
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
        "com.acme.total",
        "{ return base }",
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
    assert_eq!(error.code, ErrorCode::TestFailed, "{error:?}");
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
fn callee_formatting_replays_and_same_import_merges_idempotently() {
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
    transaction::commit(
        &formatting_repo,
        &mut caller,
        "refs/heads/main",
        &mut worker,
    )
    .unwrap();

    let import_repo = init_repo(temp.path(), "imports");
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":import_repo}))
        .unwrap();
    let hash = project["projectModelHash"].as_str().unwrap();
    let mut first = make_import_tx(&mut worker, &import_repo, hash, "import-first");
    let mut second = make_import_tx(&mut worker, &import_repo, hash, "import-second");
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

    // Advancing the target after the older transaction must never let inspect
    // or an idempotent retry publish the older commit's index snapshot.
    let mut later = make_tx(
        &mut worker,
        &import_repo,
        hash,
        "com.acme.total",
        "{ var value = base; if (premium) value = value + value; return value }",
        "after-import",
    );
    let later_result =
        transaction::commit(&import_repo, &mut later, "refs/heads/main", &mut worker).unwrap();
    let later_commit = later_result["finalCommit"].as_str().unwrap();
    let mut old_interrupted = first.clone();
    old_interrupted.status = "COMMITTING".into();
    old_interrupted.final_commit = None;
    transaction::ledger(&import_repo)
        .unwrap()
        .append(&old_interrupted, "simulated late inspection of ancestor")
        .unwrap();
    transaction::ledger(&import_repo)
        .unwrap()
        .inspect("tx:import-first")
        .unwrap();
    assert_eq!(
        RepositoryIndex::open_compilation(&import_repo, Some(":/main"))
            .unwrap()
            .published_revision()
            .unwrap()
            .as_deref(),
        Some(later_commit)
    );
    let mut retry_old = first.clone();
    transaction::commit(&import_repo, &mut retry_old, "refs/heads/main", &mut worker).unwrap();
    assert_eq!(
        RepositoryIndex::open_compilation(&import_repo, Some(":/main"))
            .unwrap()
            .published_revision()
            .unwrap()
            .as_deref(),
        Some(later_commit)
    );
    worker.shutdown().unwrap();
}
