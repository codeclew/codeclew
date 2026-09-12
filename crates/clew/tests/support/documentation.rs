//! Public CLI fixture using the repository's authenticated runtime-descriptor contract.
#![allow(dead_code)]
use clew::{canonical, runtime::RUNTIME_SCHEMA};
use serde_json::{Value, json};
use std::os::{
    fd::AsRawFd,
    unix::{
        fs::{PermissionsExt, symlink},
        process::CommandExt,
    },
};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn make_tree_removable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_dir() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                make_tree_removable(&entry.path());
            }
        }
    } else if metadata.is_file() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
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

fn run_managed_exact_path(
    binary: &Path,
    state_root: &Path,
    runtime_root: &Path,
    lease_path: &Path,
    arguments: &[&str],
    path: &Path,
) -> std::process::Output {
    run_managed_with_path(
        binary,
        state_root,
        runtime_root,
        lease_path,
        arguments,
        Some(path),
        true,
    )
}

fn run_managed_with_path(
    binary: &Path,
    state_root: &Path,
    runtime_root: &Path,
    lease_path: &Path,
    arguments: &[&str],
    path: Option<&Path>,
    exact_path: bool,
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
        if exact_path {
            command.env("PATH", prefix);
        } else {
            let mut paths = vec![prefix.to_path_buf()];
            if let Some(ambient) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&ambient));
            }
            command.env("PATH", std::env::join_paths(paths).unwrap());
        }
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

pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub docs: PathBuf,
    state: PathBuf,
    runtime: PathBuf,
    binary: PathBuf,
    lease: PathBuf,
    tools: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state/v2");
        fs::create_dir_all(state.join("locks")).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = state.join("runtimes").join("1".repeat(64));
        let binary = fd_runtime(&runtime);
        let lease = state
            .join("locks")
            .join(format!("runtime-{}.lease", "1".repeat(64)));
        let tools = temp.path().join("tools");
        fs::create_dir(&tools).unwrap();
        let git = Command::new("/bin/sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap();
        symlink(
            String::from_utf8(git.stdout).unwrap().trim(),
            tools.join("git"),
        )
        .unwrap();
        let docs = temp.path().join("docs");
        let fixture = Self {
            temp,
            docs,
            state,
            runtime,
            binary,
            lease,
            tools,
        };
        fixture.ok(&["docs", "init"]);
        fixture
    }
    pub fn run(&self, args: &[&str]) -> (i32, Value) {
        let mut args = args.to_vec();
        args.extend(["--root", self.docs.to_str().unwrap()]);
        let out = run_managed_exact_path(
            &self.binary,
            &self.state,
            &self.runtime,
            &self.lease,
            &args,
            &self.tools,
        );
        let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code().unwrap(), value)
    }
    pub fn ok(&self, args: &[&str]) -> Value {
        let (code, value) = self.run(args);
        assert_eq!(code, 0, "{args:?}: {value}");
        value
    }
    pub fn input(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.temp.path().join(name);
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }
    pub fn service(&self, id: &str) -> PathBuf {
        let repo = self.temp.path().join(id);
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("Orders.java"), "public class Orders { public int reserve(int quantity) { return normalize(quantity); } private int normalize(int quantity) { return quantity; } }\n").unwrap();
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                &format!("https://example.invalid/{id}"),
            ],
        );
        commit(&repo);
        let record=self.input(&format!("{id}.json"), &json!({"schema":"codeclew-documentation-service/1.0","id":id,"title":id,"repositoryId":id,"repository":format!("https://example.invalid/{id}"),"language":"java","profile":"source-syntax","targetRef":"HEAD","source":{"roots":["."],"dialect":"17"}}));
        let digest = self.ok(&["docs", "service", "list"])["inputDigest"]
            .as_str()
            .unwrap()
            .to_owned();
        self.ok(&[
            "docs",
            "service",
            "add",
            "--input",
            record.to_str().unwrap(),
            "--expected-input-digest",
            &digest,
        ]);
        self.ok(&[
            "docs",
            "bind",
            "--service",
            id,
            "--repo",
            repo.to_str().unwrap(),
        ]);
        repo
    }
    pub fn checked(&self) -> clew::documentation::check::Check {
        let (code, report) = self.run(&["docs", "check"]);
        assert!(matches!(code, 0 | 3 | 4), "{report}");
        assert_eq!(report["status"], "CHECKED");
        serde_json::from_slice(
            &fs::read(self.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
        )
        .unwrap()
    }
    pub fn author(&self, service: &str, checked: &clew::documentation::check::Check) -> PathBuf {
        let e = &checked.services[service];
        let entry = e
            .entrypoints
            .iter()
            .find(|e| e.symbol.contains("reserve"))
            .unwrap();
        let events:Vec<_>=e.observations.values().filter(|o|o.kind=="FLOW" && o.symbol==entry.symbol).enumerate().map(|(i,o)|json!({"id":format!("step-{i}"),"kind":"note","text":"The source returns a normalized quantity.","from":null,"to":null,"dependencyIds":[o.id],"sourceIds":o.source_ids})).collect();
        self.input(&format!("{service}-narrative.json"),&json!({"schema":"codeclew-documentation-narrative/1.0","subject":format!("service:{service}"),"contextDigest":checked.context_digest,"operations":[{"id":entry.id,"title":"Reserve quantity","summary":{"id":"summary","text":"Returns the normalized requested quantity.","dependencyIds":entry.dependency_ids,"sourceIds":entry.source_ids},"participants":[{"id":"caller","label":"Caller","service":null},{"id":"service","label":"Orders","service":service}],"events":events,"boundaries":["Source syntax only."]}],"gaps":e.entrypoints.iter().filter(|e|e.id!=entry.id).map(|e|(&e.id,"Not authored in this fixture.")).collect::<std::collections::BTreeMap<_,_>>()}))
    }
    pub fn bundle(&self, id: &str, path: &str) -> PathBuf {
        self.docs.join("docs/generated").join(id).join(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        make_tree_removable(self.temp.path());
    }
}
pub fn git(repo: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .args(args)
            .current_dir(repo)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success(),
        "git {args:?}"
    );
}
pub fn commit(repo: &Path) {
    git(repo, &["add", "."]);
    git(
        repo,
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
}
pub fn read(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
