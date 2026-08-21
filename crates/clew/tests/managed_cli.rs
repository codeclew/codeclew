use clew::canonical;
use clew::runtime::{RUNTIME_SCHEMA, RuntimeAuthority, RuntimeMode};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn run_git(repo: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?} failed");
}

fn fake_runtime(root: &Path) {
    fs::create_dir(root).unwrap();
    let mut authority = RuntimeAuthority {
        schema: RUNTIME_SCHEMA.into(),
        runtime_key: format!("sha256:{}", "1".repeat(64)),
        mode: RuntimeMode::Development,
        manifest_digest: String::new(),
        artifacts: BTreeMap::new(),
        workers: BTreeMap::new(),
        root: root.to_path_buf(),
    };
    authority.manifest_digest = canonical::hash(&authority).unwrap();
    fs::write(
        root.join("runtime.json"),
        canonical::bytes(&authority).unwrap(),
    )
    .unwrap();
}

#[test]
fn session_open_ignores_legacy_state_and_keeps_paths_private() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("repository-with-private-name");
    let state = temporary.path().join("state");
    let runtime = temporary.path().join("runtime");
    fs::create_dir(&repo).unwrap();
    fs::write(repo.join("README.md"), b"fixture\n").unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["add", "."]);
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@localhost",
            "commit",
            "-q",
            "-m",
            "baseline",
        ],
    );
    let legacy = repo.join(".semantic-thread");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("poison"), b"must not be observed").unwrap();
    fake_runtime(&runtime);
    let runtime = runtime.canonicalize().unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&legacy, fs::Permissions::from_mode(0o000)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "session",
            "open",
            "--repo",
            repo.to_str().unwrap(),
            "--target-ref",
            "main",
        ])
        .env("CODECLEW_HOME", &state)
        .env("CODECLEW_RUNTIME_ROOT", &runtime)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&legacy, fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "OPEN");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains(repo.to_str().unwrap()));
    assert!(!stdout.contains("semantic-thread"));

    let locators = walk_named(&state, "locator.json");
    assert_eq!(locators.len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            locators[0].metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

fn walk_named(root: &Path, name: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().and_then(|value| value.to_str()) == Some(name) {
                found.push(path);
            }
        }
    }
    found
}
