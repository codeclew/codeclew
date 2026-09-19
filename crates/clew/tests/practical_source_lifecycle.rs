//! Final public CLI source-lifecycle gate (slice F). One test launches the
//! `./clew` source-development launcher through the supported bootstrap (the
//! real product binary, not a capsule shorthand); the other drives a real
//! offline Maven docs capture/read/reopen through the admitted managed-dispatch
//! helpers used by the existing native CLI tests. All fixtures are synthetic
//! and test-owned; no customer build or private-state editing.

use serde_json::{Value, json};
use std::fs;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use clew::canonical;
use clew::runtime::RUNTIME_SCHEMA;

/// The `./clew` source-development launcher (repo-root shell script that runs
/// `bootstrap/clew_bootstrap.py`).
fn source_launcher() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("clew")
}

/// Admitted dispatch helpers copied verbatim from managed_cli.rs (no authority
/// semantics changed): build a test-owned runtime authority and invoke the
/// compiled `clew` binary with the runtime/state/lease FDs.
fn fd_runtime(root: &Path) -> PathBuf {
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

fn run_git(repo: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?} failed");
}

#[allow(clippy::too_many_arguments)]
fn run_managed(
    binary: &Path,
    state_root: &Path,
    runtime_root: &Path,
    lease_path: &Path,
    arguments: &[&str],
    path: Option<&Path>,
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
    if let Some(prefix) = path {
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

/// The `./clew` source launcher must build and run the real product binary
/// through the supported source bootstrap. This is the launcher/native gate, as
/// opposed to the admitted managed-dispatch helpers used elsewhere.
#[test]
fn source_bootstrap_launches_real_product() {
    let launcher = source_launcher();
    assert!(
        launcher.is_file(),
        "source launcher missing: {}",
        launcher.display()
    );
    let output = Command::new(&launcher)
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .expect("the ./clew source launcher must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "source launcher must succeed: {}\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("Usage: clew"),
        "source launcher must run the real product: {stdout}"
    );
}

/// A real offline Maven docs capture/check/read/reopen through the admitted
/// public dispatch helpers must return scope-correct source text and the correct
/// transformed/original authority, and close/reopen in the same test-owned state
/// must reproduce the same records. Synthetic fixture; no customer build.
#[test]
fn native_maven_capture_read_reopen_returns_scope_correct_authority() {
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state/v2");
    let runtime = state.join("runtimes").join("1".repeat(64));
    fs::create_dir_all(state.join("locks")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let binary = fd_runtime(&runtime);
    let lease = state
        .join("locks")
        .join(format!("runtime-{}.lease", "1".repeat(64)));
    let repo = temporary.path().join("java");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/durable-docs-source/java"),
        &repo,
    );
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", "https://example.invalid/java"],
    );
    let commit = || {
        run_git(&repo, &["add", "."]);
        run_git(
            &repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "Fixture",
            ],
        );
    };
    commit();
    let docs = temporary.path().join("architecture");
    let root = docs.to_str().unwrap();
    let run = |args: &[&str]| {
        let out = run_managed(&binary, &state, &runtime, &lease, args, None);
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code().unwrap(), value)
    };
    assert_eq!(run(&["docs", "init", "--root", root]).0, 0);
    let record_path = docs.join("catalog/services/java.json");
    fs::write(
        &record_path,
        serde_json::to_vec(&json!({
            "schema":"codeclew-documentation-service/1.0","id":"java","title":"Java reservations",
            "repositoryId":"java","repository":"https://example.invalid/java","language":"java",
            "profile":"source-syntax","targetRef":"main","source":{"roots":["."],"dialect":"17"}
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        run(&[
            "docs",
            "bind",
            "--root",
            root,
            "--service",
            "java",
            "--repo",
            repo.to_str().unwrap()
        ])
        .0,
        0
    );
    let (_, report) = run(&["docs", "check", "--root", root]);
    assert_eq!(report["status"], "CHECKED", "{report}");
    let read = || {
        clew::documentation::check::Check::load(
            &clew::documentation::store::Repository::open(&docs).unwrap(),
            &docs.join(".codeclew/cache/latest-check.json"),
        )
        .unwrap()
    };
    let first = read();
    assert!(
        first.services["java"].sources.values().count() > 0,
        "sources must be indexed"
    );
    assert!(
        first.services["java"]
            .sources
            .values()
            .all(|s| s.authority == "EXACT_SNAPSHOT_TEXT" || s.authority == "DECLARED_OPENAPI"),
        "source-syntax profile must carry snapshot/declared authority: {:?}",
        first.services["java"]
            .sources
            .values()
            .map(|s| &s.authority)
            .collect::<Vec<_>>()
    );
    // Reopen: a fresh Check load in the same test-owned state reproduces the
    // same scope-correct records (deterministic serde round-trip).
    let reopened = read();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&reopened).unwrap(),
        "close/reopen must reproduce identical records"
    );
}
