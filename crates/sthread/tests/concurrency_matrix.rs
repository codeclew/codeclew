use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use sthread::error::ErrorCode;
use sthread::graph;
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
    };
    Transaction {
        schema: "semantic-transaction/0.1".into(),
        tx_id: format!("tx:{id}"),
        actor_id: "test:concurrency".into(),
        intent: id.into(),
        base_revision: base,
        project_model_hash: project_hash.into(),
        status: "CREATED".into(),
        thread,
        edit,
        preview: None,
        test_tasks: vec![],
        candidate_commit: None,
        final_commit: None,
    }
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
