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
            "mode":0o111,
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

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn run_managed(
    binary: &Path,
    state_root: &Path,
    runtime_root: &Path,
    lease_path: &Path,
    arguments: &[&str],
    path_prefix: Option<&Path>,
) -> std::process::Output {
    let state_handle = File::open(state_root).unwrap();
    let runtime_handle = File::open(runtime_root).unwrap();
    let lease_handle = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lease_path)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(lease_path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut command = Command::new(binary);
    command
        .args(arguments)
        .env("CODECLEW_STATE_ROOT_FD", "100")
        .env("CODECLEW_RUNTIME_ROOT_FD", "101")
        .env("CODECLEW_RUNTIME_LEASE_FD", "102")
        .stdin(Stdio::null());
    if let Some(prefix) = path_prefix {
        let mut paths = vec![prefix.to_path_buf()];
        if let Some(ambient) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&ambient));
        }
        command.env("PATH", std::env::join_paths(paths).unwrap());
    }
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
    command.output().unwrap()
}

#[test]
fn fd_authority_opens_session_but_forged_paths_fail_without_observing_legacy_state() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("repository-with-private-name");
    let state = temporary.path().join("state");
    let digest = "1".repeat(64);
    let runtime = state.join("v2").join("runtimes").join(&digest);
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
    fs::create_dir_all(state.join("v2").join("locks")).unwrap();
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
            "--language",
            "kotlin",
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
    let lease_path = state
        .join("v2")
        .join("locks")
        .join(format!("runtime-{digest}.lease"));
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
            "--language",
            "kotlin",
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

#[test]
fn managed_python_fixture_produces_read_only_partial_context_without_project_processes() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("python-project");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/python-mixed");
    copy_tree(&fixture, &repo);
    fs::write(repo.join("private.env"), vec![b's'; 5 * 1024 * 1024]).unwrap();
    fs::write(
        repo.join(".gitattributes"),
        b"private.env filter=codeclew\n",
    )
    .unwrap();
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
            "python fixture",
        ],
    );
    let git_poison = repo.join(".git/codeclew-git-poison");
    fs::write(&git_poison, b"#!/bin/sh\ntouch \"$0.executed\"\nexit 97\n").unwrap();
    let checkout_hook = repo.join(".git/hooks/post-checkout");
    fs::write(
        &checkout_hook,
        b"#!/bin/sh\ntouch \"$0.executed\"\nexit 97\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&git_poison, fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(&checkout_hook, fs::Permissions::from_mode(0o500)).unwrap();
    }
    run_git(
        &repo,
        &[
            "config",
            "filter.codeclew.smudge",
            git_poison.to_str().unwrap(),
        ],
    );
    run_git(
        &repo,
        &["config", "core.fsmonitor", git_poison.to_str().unwrap()],
    );

    let state = temporary.path().join("state");
    let digest = "1".repeat(64);
    let state_root = state.join("v2");
    let runtime = state_root.join("runtimes").join(&digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime_binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{digest}.lease"));
    let poison_bin = temporary.path().join("poison-bin");
    fs::create_dir(&poison_bin).unwrap();
    for name in ["python", "python3"] {
        let executable = poison_bin.join(name);
        fs::write(&executable, b"#!/bin/sh\ntouch \"$0.executed\"\nexit 97\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        }
    }

    let opened = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "session",
            "open",
            "--repo",
            repo.to_str().unwrap(),
            "--target-ref",
            "main",
            "--language",
            "python",
            "--compilation",
            "python:.#src",
        ],
        Some(&poison_bin),
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stdout)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
    let session = opened["session"]["sessionId"].as_str().unwrap();
    let session_component = session.strip_prefix("session:").unwrap();
    assert!(
        !state_root
            .join("sessions")
            .join(session_component)
            .join("source")
            .exists()
    );
    assert!(!git_poison.with_extension("executed").exists());
    assert!(!checkout_hook.with_extension("executed").exists());

    let context = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "context",
            "create",
            "--session",
            session,
            "--intent",
            "Find service normalization behavior",
            "--term",
            "Service",
            "--term",
            "normalize",
        ],
        Some(&poison_bin),
    );
    assert!(
        context.status.success(),
        "{}",
        String::from_utf8_lossy(&context.stdout)
    );
    assert!(context.stdout.len() <= 64 * 1024);
    let encoded = String::from_utf8(context.stdout.clone()).unwrap();
    assert!(encoded.contains("language:python"));
    assert!(encoded.contains("Service"));
    assert!(encoded.contains("normalize"));
    assert!(encoded.contains("PARTIAL"));
    assert!(encoded.contains("UNSURE"));
    assert!(!encoded.contains("Codeclew must never execute"));
    assert!(!encoded.contains("Python analysis must not start"));
    assert!(!repo.join("PROJECT_RUNTIME_EXECUTED").exists());
    assert!(!poison_bin.join("python.executed").exists());
    assert!(!poison_bin.join("python3.executed").exists());
    assert!(!git_poison.with_extension("executed").exists());
    assert!(!checkout_hook.with_extension("executed").exists());
    let context: Value = serde_json::from_slice(&context.stdout).unwrap();
    let context_id = context["contextId"].as_str().unwrap();

    let mutation = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "task-run",
            "start",
            "--session",
            session,
            "--context",
            context_id,
            "--plan",
            "plan:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ],
        Some(&poison_bin),
    );
    assert!(!mutation.status.success());
    let mutation: Value = serde_json::from_slice(&mutation.stdout).unwrap();
    assert_eq!(mutation["error"]["code"], "UNSUPPORTED_LANGUAGE");

    for operation in ["close", "gc"] {
        let output = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &["session", operation, "--session", session],
            Some(&poison_bin),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    assert!(!repo.join("PROJECT_RUNTIME_EXECUTED").exists());
    assert!(!poison_bin.join("python.executed").exists());
    assert!(!poison_bin.join("python3.executed").exists());
    assert!(!git_poison.with_extension("executed").exists());
    assert!(!checkout_hook.with_extension("executed").exists());
}
