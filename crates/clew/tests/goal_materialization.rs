mod support;

use serde_json::Value;
use std::fs;
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
    fs::remove_file(repo.join("src/main/kotlin/com/acme/RelationFacts.kt")).unwrap();
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
    support::seed_build_caches(&repo);
    let prepared = Command::new("./gradlew")
        .args(["compileTestKotlin", "--no-daemon", "--quiet"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(prepared.success());
    for relative in ["build/classes/java/main", "build/resources/main"] {
        fs::create_dir_all(repo.join(relative)).unwrap();
    }
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
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
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
fn clew_apply_surfaces_conditional_evidence_without_publishing_a_change() {
    let (temporary, repo) = committed_fixture(|_, _| {});
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let artifact = temporary.path().join("transaction.json");
    let result = parsed(&apply(
        &repo,
        "com.acme.valuesAwaitingContext",
        "applies the mapping context to one value",
        Some(&artifact),
    ));
    assert_eq!(result["status"], "CONDITIONAL", "{result:#}");
    assert_eq!(result["unresolvedObligations"].as_array().unwrap().len(), 2);
    assert!(
        result["unresolvedObligations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|obligation| obligation["publicationBlocking"] == true)
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), base);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert!(!artifact.exists());
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
fn conditional_evidence_must_be_recomputed_after_a_source_change() {
    let (_temporary, repo) = committed_fixture(|_, _| {});
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
    let MapEdgeWithContextDecision::Conditional(_conditional) = decision else {
        panic!(
            "fixture must expose conditional evidence before the concurrent change: {decision:?}"
        )
    };

    let source = repo.join("src/main/kotlin/com/acme/Runner.kt");
    let mut changed = fs::read_to_string(&source).unwrap();
    changed.push_str("\n// concurrent user change\n");
    fs::write(&source, &changed).unwrap();
    let error = authority
        .bind_map_edge_with_context(
            &SemanticGoal::map_edge_with_context(&revision),
            &verified,
            "applies the mapping context to one value",
            ":/test",
            &mut worker,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleRequiresReslice);
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), revision);
    assert_eq!(fs::read_to_string(source).unwrap(), changed);
    worker.shutdown().unwrap();
}
use clew::error::ErrorCode;
use clew::evidence_authority::{EvidenceAuthority, MapEdgeWithContextDecision};
use clew::model::ThreadIr;
use clew::semantic_goal::SemanticGoal;
use clew::worker::{WorkerClient, workspace_root};
