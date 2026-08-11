use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&source, &target);
        } else {
            fs::copy(source, target).unwrap();
        }
    }
}

fn committed_fixture(
    mutator: impl FnOnce(&mut String, &mut String),
) -> (tempfile::TempDir, PathBuf) {
    let workspace = clew::worker::workspace_root();
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    for name in ["build.gradle.kts", "settings.gradle.kts", ".gitignore"] {
        fs::copy(
            workspace.join("fixtures/kotlin-2-1").join(name),
            repo.join(name),
        )
        .unwrap();
    }
    fs::copy(
        workspace.join("fixtures/kotlin-basic/gradlew"),
        repo.join("gradlew"),
    )
    .unwrap();
    let mut executable = fs::metadata(repo.join("gradlew")).unwrap().permissions();
    executable.set_mode(0o755);
    fs::set_permissions(repo.join("gradlew"), executable).unwrap();
    copy_tree(
        &workspace.join("fixtures/kotlin-basic/gradle"),
        &repo.join("gradle"),
    );
    copy_tree(
        &workspace.join("fixtures/kotlin-2-1/src/main/kotlin"),
        &repo.join("src/main/kotlin"),
    );
    copy_tree(
        &workspace.join("fixtures/kotlin-2-1/src/test/kotlin"),
        &repo.join("src/test/kotlin"),
    );
    let main_path = repo.join("src/main/kotlin/com/acme/Runner.kt");
    let test_path = repo.join("src/test/kotlin/com/acme/RunnerTest.kt");
    let mut main = fs::read_to_string(&main_path).unwrap();
    let mut test = fs::read_to_string(&test_path).unwrap();
    mutator(&mut main, &mut test);
    fs::write(main_path, main).unwrap();
    fs::write(test_path, test).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch=main"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "base",
        ],
    );
    (temporary, repo)
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn apply(repo: &Path, workflow_symbol: &str, test_symbol: &str, output: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_clew"));
    command.args([
        "apply",
        "map-edge-with-context",
        "--repo",
        repo.to_str().unwrap(),
        "--workflow-symbol",
        workflow_symbol,
        "--test-symbol",
        test_symbol,
        "--target-ref",
        "refs/heads/main",
    ]);
    if let Some(output) = output {
        command.args(["--output", output.to_str().unwrap()]);
    }
    command.output().unwrap()
}

fn parsed(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
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
fn clew_apply_materializes_the_proved_change_as_one_verified_commit() {
    let (temporary, repo) = committed_fixture(|_, _| {});
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let artifact = temporary.path().join("transaction.json");
    let result = parsed(&apply(
        &repo,
        "com.acme.valuesAwaitingContext",
        "applies the mapping context to one value",
        Some(&artifact),
    ));
    assert_eq!(result["status"], "COMMITTED", "{result:#}");
    assert_eq!(
        result["changedFiles"],
        serde_json::json!(["src/main/kotlin/com/acme/Runner.kt"])
    );
    let head = git(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(base, head);
    assert_eq!(result["finalCommit"], head);
    assert_eq!(
        git(&repo, &["diff", "--name-only", &format!("{base}..{head}")]),
        "src/main/kotlin/com/acme/Runner.kt"
    );
    let source = fs::read_to_string(repo.join("src/main/kotlin/com/acme/Runner.kt")).unwrap();
    assert!(source.contains("val __codeclewContext = com.acme.mappingContext()"));
    assert!(source.contains(
        "return values.map { __codeclewValue -> com.acme.applyMappingContext(__codeclewValue, __codeclewContext) }"
    ));
    let transaction: Value = serde_json::from_slice(&fs::read(artifact).unwrap()).unwrap();
    let operation = &transaction["edit"]["operations"][0];
    assert_eq!(operation["kind"], "MAP_EDGE_WITH_CONTEXT");
    assert_eq!(operation["replacement"]["kotlin"], "");
    assert_eq!(
        operation["semanticOperation"]["kind"],
        "MAP_EDGE_WITH_CONTEXT"
    );
    assert!(
        transaction["validationEvidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["kind"] == "AUTHORITY_MAP_EDGE_PROOF" && item["invariantCount"] == 12
            })
    );
    assert_eq!(
        git(&repo, &["show", "-s", "--format=%an", "HEAD"]),
        "Codeclew"
    );

    let mut tests = OpenOptions::new()
        .append(true)
        .open(repo.join("src/test/kotlin/com/acme/RunnerTest.kt"))
        .unwrap();
    writeln!(
        tests,
        "\nclass MaterializedWorkflowAcceptance {{\n    @Test\n    fun `workflow applies the context to every value`() {{\n        assertEquals(listOf(6, 7), valuesAwaitingContext(listOf(4, 5)))\n    }}\n}}"
    )
    .unwrap();
    let hidden = Command::new("./gradlew")
        .args(["test", "--rerun-tasks", "--no-daemon"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        hidden.status.success(),
        "{}",
        String::from_utf8_lossy(&hidden.stderr)
    );
}

#[test]
fn clew_apply_leaves_no_commit_or_source_change_when_not_bound() {
    let (_ambiguous_temporary, ambiguous_repo) = committed_fixture(|main, _| {
        main.push_str("\nfun anotherCompatibleContext(): Int = 3\n");
    });
    let ambiguous_head = git(&ambiguous_repo, &["rev-parse", "HEAD"]);
    let ambiguity = parsed(&apply(
        &ambiguous_repo,
        "com.acme.valuesAwaitingContext",
        "unused because binding is ambiguous",
        None,
    ));
    assert_eq!(ambiguity["status"], "AMBIGUOUS");
    assert_eq!(git(&ambiguous_repo, &["rev-parse", "HEAD"]), ambiguous_head);
    assert_eq!(git(&ambiguous_repo, &["status", "--porcelain"]), "");

    let (_refused_temporary, refused_repo) = committed_fixture(|main, _| {
        main.push_str(
            "\nfun sequenceAwaitingContext(values: Sequence<Int>): Sequence<Int> = values\n",
        );
    });
    let refused_head = git(&refused_repo, &["rev-parse", "HEAD"]);
    let refusal = parsed(&apply(
        &refused_repo,
        "com.acme.sequenceAwaitingContext",
        "unused because the collection is lazy",
        None,
    ));
    assert_eq!(refusal["status"], "REFUSED");
    assert_eq!(git(&refused_repo, &["rev-parse", "HEAD"]), refused_head);
    assert_eq!(git(&refused_repo, &["status", "--porcelain"]), "");
}

#[test]
fn an_authority_receipt_cannot_overwrite_a_newer_worktree() {
    let (temporary, repo) = committed_fixture(|_, _| {});
    let revision = git(&repo, &["rev-parse", "HEAD"]);
    let thread = live_thread(&repo, "com.acme.valuesAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&repo, &revision).unwrap();
    let verified = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &verified,
            "applies the mapping context to one value",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Bound(receipt) = decision else {
        panic!("fixture must bind before the concurrent change: {decision:?}")
    };

    let target_worktree = temporary.path().join("target-worktree");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "target",
            target_worktree.to_str().unwrap(),
        ],
    );
    let target_source = target_worktree.join("src/main/kotlin/com/acme/Runner.kt");
    let target_changed = fs::read_to_string(&target_source)
        .unwrap()
        .replace(
            "fun applyMappingContext(value: Int, context: Int): Int = value + context",
            "fun applyMappingContext(value: Int, context: Int): Int { println(\"new side effect\"); return value + context }",
        );
    fs::write(&target_source, target_changed).unwrap();
    git(&target_worktree, &["add", "."]);
    git(
        &target_worktree,
        &[
            "-c",
            "user.name=Concurrent User",
            "-c",
            "user.email=concurrent@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "change transformer after proof",
        ],
    );
    let target_head = git(&repo, &["rev-parse", "refs/heads/target"]);
    let target_error = authority
        .commit_map_edge_with_context(&receipt, "codeclew-test", "refs/heads/target", &mut worker)
        .unwrap_err();
    assert_eq!(target_error.code, ErrorCode::StaleRequiresReslice);
    assert_eq!(git(&repo, &["rev-parse", "refs/heads/target"]), target_head);

    let source = repo.join("src/main/kotlin/com/acme/Runner.kt");
    let mut changed = fs::read_to_string(&source).unwrap();
    changed.push_str("\n// concurrent user change\n");
    fs::write(&source, &changed).unwrap();
    let error = authority
        .commit_map_edge_with_context(&receipt, "codeclew-test", "refs/heads/main", &mut worker)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleRequiresReslice);
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), revision);
    assert_eq!(fs::read_to_string(source).unwrap(), changed);
    worker.shutdown().unwrap();
}

#[test]
fn differential_validation_accepts_candidate_and_failing_omission() {
    let (_temporary, repo) = committed_fixture(|_, test| {
        test.push_str(
            r#"

class DifferentialWorkflowAcceptance {
    @Test
    fun `workflow result requires the production change`() {
        assertEquals(listOf(6), valuesAwaitingContext(listOf(4)))
    }
}
"#,
        );
    });
    let revision = git(&repo, &["rev-parse", "HEAD"]);
    let thread = live_thread(&repo, "com.acme.valuesAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&repo, &revision).unwrap();
    let verified = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &verified,
            "applies the mapping context to one value",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Bound(proof) = decision else {
        panic!("fixture must bind: {decision:?}")
    };
    let overlay = authority
        .materialize_candidate_overlay(&proof, &mut worker)
        .unwrap();
    let _oracle = authority
        .issue_candidate_behavioral_test(
            &overlay,
            "workflow result requires the production change",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let differential = authority
        .run_differential_validation(&overlay, &mut worker)
        .unwrap();
    assert!(
        authority
            .recognizes_differential_validation(&differential)
            .unwrap()
    );
    assert_eq!(differential.summary().production_write_count, 1);
    assert_eq!(differential.summary().test_write_count, 0);
    assert_ne!(
        differential.summary().candidate_artifact_hash,
        differential.summary().omission_artifact_hash
    );
    worker.shutdown().unwrap();
}

#[test]
fn differential_validation_negative_omission_passes() {
    let (_temporary, repo) = committed_fixture(|_, test| {
        test.push_str(
            r#"

class DifferentialNonDiscriminatingOracle {
    @Test
    fun `workflow remains non empty`() {
        assertEquals(1, valuesAwaitingContext(listOf(4)).size)
    }
}
"#,
        );
    });
    let revision = git(&repo, &["rev-parse", "HEAD"]);
    let thread = live_thread(&repo, "com.acme.valuesAwaitingContext");
    let mut worker = WorkerClient::start(&workspace_root()).unwrap();
    let mut authority = EvidenceAuthority::open(&repo, &revision).unwrap();
    let verified = authority.verify_thread(&thread, &mut worker).unwrap();
    let decision = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &verified,
            "applies the mapping context to one value",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let MapEdgeWithContextDecision::Bound(proof) = decision else {
        panic!("fixture must bind: {decision:?}")
    };
    let overlay = authority
        .materialize_candidate_overlay(&proof, &mut worker)
        .unwrap();
    let _oracle = authority
        .issue_candidate_behavioral_test(
            &overlay,
            "workflow remains non empty",
            ":/test",
            &mut worker,
        )
        .unwrap();
    let error = authority
        .run_differential_validation(&overlay, &mut worker)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PreconditionFailed);
    assert!(error.message.contains("omission mutant did not fail"));
    worker.shutdown().unwrap();
}
use clew::error::ErrorCode;
use clew::evidence_authority::{EvidenceAuthority, MapEdgeWithContextDecision};
use clew::model::ThreadIr;
use clew::semantic_goal::SemanticGoal;
use clew::worker::{WorkerClient, workspace_root};
