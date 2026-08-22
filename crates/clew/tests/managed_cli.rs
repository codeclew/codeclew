use clew::canonical;
use clew::runtime::RUNTIME_SCHEMA;
use serde_json::Value;
use std::fs;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

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

fn fd_runtime(root: &Path) -> std::path::PathBuf {
    let binary = root.join("bin/clew");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_clew"), &binary).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o500)).unwrap();
    }
    let bytes = fs::read(&binary).unwrap();
    let runtime_key = format!("sha256:{}", "1".repeat(64));
    let mut manifest = serde_json::json!({
        "schema":RUNTIME_SCHEMA,
        "runtimeKey":runtime_key,
        "mode":"DEVELOPMENT",
        "manifestDigest":"",
        "inputDigest":format!("sha256:{}", "2".repeat(64)),
        "platformAuthority":{"fixture":true},
        "toolchainAuthority":{"fixture":true},
        "components":{"clew":format!("sha256:{}", "3".repeat(64))},
        "artifacts":{"clew":{
            "path":"bin/clew",
            "size":bytes.len(),
            "sha256":canonical::hash_bytes(&bytes),
        }},
        "workers":{},
    });
    manifest["manifestDigest"] = Value::String(canonical::hash(&manifest).unwrap());
    fs::write(
        root.join("runtime.json"),
        canonical::bytes(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(root.join("READY"), format!("{runtime_key}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o500)).unwrap();
    }
    binary
}

#[test]
fn fd_authority_opens_session_but_forged_paths_fail_without_observing_legacy_state() {
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
    fs::create_dir_all(state.join("v2")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(state.join("v2"), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime_binary = fd_runtime(&runtime);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&legacy, fs::Permissions::from_mode(0o000)).unwrap();
    }
    let forged = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "session",
            "open",
            "--repo",
            repo.to_str().unwrap(),
            "--target-ref",
            "main",
            "--compilation",
            ":/main",
        ])
        .env("CODECLEW_HOME", &state)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!forged.status.success());
    let value: Value = serde_json::from_slice(&forged.stdout).unwrap();
    assert_eq!(value["error"]["code"], "WORKER_PREPARATION_REQUIRED");
    let stdout = String::from_utf8(forged.stdout).unwrap();
    assert!(!stdout.contains(repo.to_str().unwrap()));
    assert!(!stdout.contains("semantic-thread"));

    let state_handle = File::open(state.join("v2")).unwrap();
    let runtime_handle = File::open(&runtime).unwrap();
    let lease_path = temporary.path().join("runtime.lease");
    let lease_handle = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lease_path)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&lease_path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut command = Command::new(runtime_binary);
    command
        .args([
            "session",
            "open",
            "--repo",
            repo.to_str().unwrap(),
            "--target-ref",
            "main",
            "--compilation",
            ":z/main",
            "--compilation",
            ":/main",
        ])
        .env("CODECLEW_STATE_ROOT_FD", "100")
        .env("CODECLEW_RUNTIME_ROOT_FD", "101")
        .env("CODECLEW_RUNTIME_LEASE_FD", "102")
        .stdin(Stdio::null());
    #[cfg(unix)]
    unsafe {
        let state_fd = state_handle.as_raw_fd();
        let runtime_fd = runtime_handle.as_raw_fd();
        let lease_fd = lease_handle.as_raw_fd();
        command.pre_exec(move || {
            for (source, target) in [(state_fd, 100), (runtime_fd, 101), (lease_fd, 102)] {
                if libc::dup2(source, target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let output = command.output().unwrap();
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
    assert_eq!(value["schema"], "codeclew-session-open/4.0");
    assert_eq!(value["status"], "OPEN");
    assert_eq!(
        value["session"]["compilations"],
        serde_json::json!([":/main", ":z/main"])
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains(repo.to_str().unwrap()));
    assert!(!stdout.contains("semantic-thread"));
}
