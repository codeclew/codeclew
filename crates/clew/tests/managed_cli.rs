use clew::canonical;
use clew::cas::{CAS_OBJECT_SCHEMA, CasObject, CasStore};
use clew::runtime::RUNTIME_SCHEMA;
use clew::state::StateAuthority;
use clew::thread::ThreadAuthority;
use clew::thread_callables::{
    self, CallableBudgets, CallableBuildInput, CallableCompilationAuthority,
    CallableFactSetRequest, CallableMemberAuthority, CallablePairBinding,
    CallableSelectedCompilation, CallableTaskBinding, GraphCoverage, KOTLIN_SEMANTIC_FACT_SCHEMA,
    QualifiedCallablePayload, RelationshipAuthority,
};
use clew::thread_callables_service::{THREAD_CALLABLE_ROOT_SCHEMA, ThreadCallableRoot};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(unix)]
struct WritableTreeOnDrop(std::path::PathBuf);

#[cfg(unix)]
impl Drop for WritableTreeOnDrop {
    fn drop(&mut self) {
        make_tree_removable(&self.0);
    }
}

#[cfg(unix)]
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

fn managed_file_snapshot(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn collect(
        root: &Path,
        current: &Path,
        output: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                collect(root, &entry.path(), output);
            } else if kind.is_file() {
                output.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut output = std::collections::BTreeMap::new();
    collect(root, root, &mut output);
    output
}

fn collect_cas_references(value: &Value, output: &mut Vec<CasObject>) {
    if value.get("schema").and_then(Value::as_str) == Some(CAS_OBJECT_SCHEMA) {
        output.push(serde_json::from_value(value.clone()).unwrap());
        return;
    }
    match value {
        Value::Array(values) => {
            for value in values {
                collect_cas_references(value, output);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_cas_references(value, output);
            }
        }
        _ => {}
    }
}

fn read_cas_bytes(store: &CasStore, object: &CasObject) -> Vec<u8> {
    let bytes = store
        .read(object, usize::try_from(object.size).unwrap())
        .unwrap()
        .bytes()
        .to_vec();
    assert_eq!(bytes.len() as u64, object.size);
    assert_eq!(
        CasObject::for_bytes(&object.object_schema, &bytes).unwrap(),
        *object
    );
    bytes
}

fn rooted_cas_closure_from_store(
    store: &CasStore,
    roots: &[Vec<u8>],
) -> BTreeMap<String, (String, Vec<u8>)> {
    let mut queue = VecDeque::new();
    for bytes in roots {
        if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
            let mut references = Vec::new();
            collect_cas_references(&value, &mut references);
            queue.extend(references);
        }
    }
    let mut closure = BTreeMap::new();
    while let Some(reference) = queue.pop_front() {
        if closure.contains_key(&reference.digest) {
            continue;
        }
        let bytes = read_cas_bytes(store, &reference);
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            let mut nested = Vec::new();
            collect_cas_references(&value, &mut nested);
            queue.extend(nested);
        }
        closure.insert(reference.digest, (reference.object_schema, bytes));
    }
    closure
}

fn rooted_cas_closure(state_root: &Path, roots: &[Vec<u8>]) -> BTreeMap<String, (String, Vec<u8>)> {
    let transfer = tempfile::tempdir().unwrap();
    let request = transfer.path().join("roots.json");
    let result = transfer.path().join("closure.json");
    fs::write(&request, serde_json::to_vec(roots).unwrap()).unwrap();

    let state_handle = File::open(state_root).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "managed_cas_closure_helper", "--nocapture"])
        .env("CODECLEW_STATE_ROOT_FD", "100")
        .env("CODECLEW_CAS_CLOSURE_REQUEST", &request)
        .env("CODECLEW_CAS_CLOSURE_RESULT", &result)
        .stdin(Stdio::null());
    #[cfg(unix)]
    unsafe {
        let state_fd = state_handle.as_raw_fd();
        command.pre_exec(move || {
            if libc::dup2(state_fd, 100) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "CAS closure helper failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&fs::read(result).unwrap()).unwrap()
}

fn assert_bytes_hide_paths(bytes: &[u8], paths: &[&Path]) {
    for path in paths {
        let needle = path.to_string_lossy();
        assert!(
            !bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes()),
            "retained evidence leaked private path {needle}"
        );
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
    run_managed_with_path(
        binary,
        state_root,
        runtime_root,
        lease_path,
        arguments,
        path_prefix,
        false,
    )
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

#[allow(clippy::too_many_arguments)]
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

fn run_callable_seed_helper(state_root: &Path, thread_id: &str, result_path: &Path, variant: &str) {
    let state_handle = File::open(state_root).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "managed_thread_impact_seed_helper",
            "--nocapture",
        ])
        .env("CODECLEW_STATE_ROOT_FD", "100")
        .env("CODECLEW_SYNTHETIC_CALLABLE_THREAD", thread_id)
        .env("CODECLEW_SYNTHETIC_CALLABLE_RESULT", result_path)
        .env("CODECLEW_SYNTHETIC_CALLABLE_VARIANT", variant)
        .stdin(Stdio::null());
    #[cfg(unix)]
    unsafe {
        let state_fd = state_handle.as_raw_fd();
        command.pre_exec(move || {
            if libc::dup2(state_fd, 100) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "synthetic S1 seed helper failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn synthetic_digest(label: &str) -> String {
    canonical::hash(&label).unwrap()
}

fn synthetic_descriptor(file: &str, alias: &str, variant: &str) -> Value {
    let changed = alias == "provider" && variant == "after";
    let jvm_descriptor = if changed {
        "()I"
    } else {
        "()Ljava/lang/String;"
    };
    let return_type = if changed {
        "kotlin/Int"
    } else {
        "kotlin/String"
    };
    json!({
        "schema":"declaration-descriptor/0.1",
        "file":file,
        "start":0,
        "end":8,
        "symbolIdentity":format!("callable:p/Orders.findOrder#jvm:{jvm_descriptor}"),
        "declarationKind":"FUNCTION",
        "ownerIdentity":"class:p/Orders",
        "containment":["class:p/Orders"],
        "visibility":"public",
        "effectiveVisibility":"public",
        "exportBoundary":"PUBLIC_API",
        "modality":"FINAL",
        "resolution":"PROVEN",
        "provider":"K2_FIR",
        "module":":app",
        "sourceSet":"main",
        "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        "compilerAuthority":"fir-facts-extractor/0.6",
        "typeParameters":[],
        "compilerCallableId":"p/Orders.findOrder",
        "isOverride":false,
        "returnType":return_type,
        "returnNullable":false,
        "parameterTypes":[],
    })
}

fn seed_synthetic_callable_fact_set(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    variant: &str,
) -> ThreadCallableRoot {
    let mut external_objects = Vec::<(String, Vec<u8>)>::new();
    let mut selected_compilations = Vec::new();
    let mut payloads = Vec::new();
    for binding in &thread.members {
        let alias = &binding.member_alias;
        let snapshot_bytes = format!("sealed repository snapshot for {alias}\n").into_bytes();
        let snapshot_ref =
            CasObject::for_bytes("codeclew-repository-input-snapshot/1.0", &snapshot_bytes)
                .unwrap();
        external_objects.push((snapshot_ref.object_schema.clone(), snapshot_bytes));
        let generation_bytes = format!("sealed Kotlin generation for {alias}\n").into_bytes();
        let generation_ref =
            CasObject::for_bytes("codeclew-generation-manifest/2.0", &generation_bytes).unwrap();
        external_objects.push((generation_ref.object_schema.clone(), generation_bytes));
        let source_bytes = format!("fun findOrder(): String = \"{alias}\"\n").into_bytes();
        let source_ref =
            CasObject::for_bytes("codeclew-repository-source-content/1.0", &source_bytes).unwrap();
        external_objects.push((source_ref.object_schema.clone(), source_bytes));

        let member = CallableMemberAuthority {
            member_alias: alias.clone(),
            service_alias: binding.service_alias.clone(),
            session_id: binding.session.session_id.clone(),
            session_authority_digest: binding.session.authority_digest.clone(),
            repository_key: binding.session.repository_key.clone(),
            base_revision: binding.session.base_revision.clone(),
            snapshot_ref,
        };
        let compilation = CallableCompilationAuthority {
            compilation_id: ":app/main".into(),
            generation_id: synthetic_digest(&format!("generation-id-{alias}")),
            generation_ref,
            semantic_authority: "K2_FIR".into(),
            extractor_id: "fir-facts-extractor/0.6".into(),
            adapter_digest: synthetic_digest("synthetic-adapter"),
            runtime_digest: synthetic_digest("synthetic-runtime"),
            descriptor_coverage: GraphCoverage::CompleteSupportedSubset,
            relation_coverage: GraphCoverage::CompleteSupportedSubset,
        };
        let file = format!("src/{alias}Orders.kt");
        let payload = synthetic_descriptor(&file, alias, variant);
        let payload_bytes = canonical::bytes(&payload).unwrap();
        let payload_ref =
            CasObject::for_bytes(KOTLIN_SEMANTIC_FACT_SCHEMA, &payload_bytes).unwrap();
        external_objects.push((payload_ref.object_schema.clone(), payload_bytes.clone()));
        let payload_digest = canonical::hash_bytes(&payload_bytes);
        payloads.push(QualifiedCallablePayload {
            member: member.clone(),
            compilation: compilation.clone(),
            fact_key: format!(
                "kotlin:descriptor:{}",
                payload_digest.strip_prefix("sha256:").unwrap()
            ),
            payload_ref,
            source_ref: Some(source_ref),
            payload,
        });
        selected_compilations.push(CallableSelectedCompilation {
            member,
            compilation,
        });
    }
    let visited_payload_bytes = payloads
        .iter()
        .map(|payload| canonical::bytes(&payload.payload).unwrap().len())
        .sum();
    let prepared = thread_callables::build(
        CallableFactSetRequest {
            thread_id: thread.thread_id.clone(),
            thread_authority_digest: thread.authority_digest.clone(),
            thread_context_id: format!(
                "thread-context:{}",
                synthetic_digest("synthetic-thread-context")
            ),
            thread_context_authority_digest: synthetic_digest("synthetic-thread-context-authority"),
            profile_digest: synthetic_digest("synthetic-callable-profile"),
            tasks: vec![CallableTaskBinding {
                task_id: "impact-task".into(),
                pair_id: "provider-consumer".into(),
                terms: vec!["findOrder".into()],
            }],
            pairs: vec![CallablePairBinding {
                pair_id: "provider-consumer".into(),
                provider_member: "provider".into(),
                consumer_member: "consumer".into(),
                relationship_authority: RelationshipAuthority::DeclaredTopology,
                dependency_evidence_ref: None,
            }],
            budgets: CallableBudgets::frozen(),
        },
        CallableBuildInput {
            visited_fact_count: payloads.len(),
            visited_payload_bytes,
            selected_compilations,
            payloads,
        },
    )
    .unwrap();
    thread_callables::verify_prepared(&prepared).unwrap();

    let mut objects = external_objects;
    objects.extend(
        prepared
            .fact_shards
            .iter()
            .chain(&prepared.query_shards)
            .chain([&prepared.query_index_object, &prepared.evidence_object])
            .map(|object| (object.reference.object_schema.clone(), object.bytes.clone())),
    );
    let expected = objects
        .iter()
        .map(|(schema, bytes)| CasObject::for_bytes(schema, bytes).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(store.put_batch(objects).unwrap(), expected);
    let published = expected
        .into_iter()
        .map(|object| (object.digest, object.object_schema, object.size))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        prepared
            .authority
            .direct_cas_closure
            .iter()
            .map(|object| {
                (
                    object.digest.clone(),
                    object.object_schema.clone(),
                    object.size,
                )
            })
            .collect::<BTreeSet<_>>(),
        published,
    );

    let root = ThreadCallableRoot {
        schema: THREAD_CALLABLE_ROOT_SCHEMA.into(),
        fact_set_id: prepared.projection.fact_set_id.clone(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        thread_context_id: prepared.authority.thread_context_id.clone(),
        thread_context_authority_digest: prepared.authority.thread_context_authority_digest.clone(),
        authority: prepared.authority,
        projection: prepared.projection,
    };
    let digest = root
        .fact_set_id
        .strip_prefix("thread-callables:sha256:")
        .unwrap();
    let root_path = state
        .thread_root(&thread.thread_id)
        .unwrap()
        .join("callable-fact-sets")
        .join(format!("{digest}.json"));
    state
        .write_private_atomic(&root_path, &canonical::bytes(&root).unwrap())
        .unwrap();
    root
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
fn managed_python_context_rejects_missing_plan_without_project_processes() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("python-project");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/python-mixed");
    copy_tree(&fixture, &repo);
    fs::write(
        repo.join("src/example/build-trusted-worker.py"),
        b"def build_worker():\n    return compile_worker()\n",
    )
    .unwrap();
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
            "context",
            "open",
            "--repo",
            repo.to_str().unwrap(),
            "--target-ref",
            "main",
            "--language",
            "python",
            "--compilation",
            "python:.#src",
            "--profile",
            "python-syntax",
            "--operation",
            "analysis",
            "--intent",
            "Find service normalization behavior",
            "--term",
            "Service",
            "--term",
            "normalize",
            "--term",
            "build_worker",
        ],
        Some(&poison_bin),
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stdout)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
    assert_eq!(opened["schema"], "codeclew-context-open/1.0");
    assert_eq!(opened["admission"]["status"], "PASS");
    assert_eq!(
        opened["admission"]["agentContract"]["schema"],
        "codeclew-agent-contract/1.0"
    );
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

    assert!(opened["context"].to_string().len() <= 64 * 1024);
    let encoded = String::from_utf8(canonical::bytes(&opened).unwrap()).unwrap();
    assert!(encoded.contains("language:python"));
    assert!(encoded.contains("Service"));
    assert!(encoded.contains("normalize"));
    assert!(encoded.contains("build_worker"));
    assert!(encoded.contains("NON_IMPORTABLE_FILE"));
    assert!(encoded.contains("PARTIAL"));
    assert!(encoded.contains("UNSURE"));
    assert!(!encoded.contains("Codeclew must never execute"));
    assert!(!encoded.contains("Python analysis must not start"));
    assert!(!repo.join("PROJECT_RUNTIME_EXECUTED").exists());
    assert!(!poison_bin.join("python.executed").exists());
    assert!(!poison_bin.join("python3.executed").exists());
    assert!(!git_poison.with_extension("executed").exists());
    assert!(!checkout_hook.with_extension("executed").exists());
    let context_id = opened["context"]["contextId"].as_str().unwrap();

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
    assert_eq!(mutation["error"]["code"], "INVALID_INPUT");

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

#[test]
fn managed_thread_composes_two_warm_repositories_without_processes_or_session_mutation() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/python-mixed");
    let repositories = [
        temporary.path().join("provider-private-name"),
        temporary.path().join("consumer-private-name"),
    ];
    for (index, repository) in repositories.iter().enumerate() {
        copy_tree(&fixture, repository);
        fs::write(
            repository.join("service-marker.py"),
            format!("SERVICE_INDEX = {index}\n"),
        )
        .unwrap();
        run_git(repository, &["init", "-q", "-b", "main"]);
        run_git(repository, &["add", "."]);
        run_git(
            repository,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-q",
                "-m",
                "thread fixture",
            ],
        );
    }

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
    for name in [
        "cargo", "rustc", "gradle", "gradlew", "mvn", "mvnw", "java", "python", "python3",
    ] {
        let executable = poison_bin.join(name);
        fs::write(&executable, b"#!/bin/sh\ntouch \"$0.executed\"\nexit 97\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        }
    }

    let mut sessions = Vec::new();
    for repository in &repositories {
        let opened = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &[
                "session",
                "open",
                "--repo",
                repository.to_str().unwrap(),
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
        let session = opened["session"]["sessionId"].as_str().unwrap().to_owned();
        let primed = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &[
                "context",
                "create",
                "--session",
                &session,
                "--intent",
                "Prime deterministic analysis generation",
                "--term",
                "Service",
                "--term",
                "normalize",
            ],
            Some(&poison_bin),
        );
        assert!(
            primed.status.success(),
            "{}",
            String::from_utf8_lossy(&primed.stdout)
        );
        sessions.push(session);
    }

    let provider = format!("provider={}", sessions[0]);
    let consumer = format!("consumer={}", sessions[1]);
    let opened = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "open",
            "--member",
            &provider,
            "--member",
            &consumer,
            "--service-alias",
            "provider=orders",
            "--service-alias",
            "consumer=checkout",
        ],
        Some(&poison_bin),
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stdout)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
    let thread = opened["thread"]["threadId"].as_str().unwrap().to_owned();
    let thread_component = thread.strip_prefix("thread:").unwrap();
    let partial_provider = format!("a-provider={}", sessions[0]);
    let partial_consumer = format!("z-consumer={}", sessions[1]);
    let partial_opened = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "open",
            "--member",
            &partial_provider,
            "--member",
            &partial_consumer,
        ],
        Some(&poison_bin),
    );
    assert!(partial_opened.status.success());
    let partial_opened: Value = serde_json::from_slice(&partial_opened.stdout).unwrap();
    let partial_thread = partial_opened["thread"]["threadId"]
        .as_str()
        .unwrap()
        .to_owned();
    let context = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "context",
            "--thread",
            &thread,
            "--intent",
            "Trace normalization across service repositories",
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
    assert!(!encoded.contains(repositories[0].to_str().unwrap()));
    assert!(!encoded.contains(repositories[1].to_str().unwrap()));
    let context: Value = serde_json::from_slice(&context.stdout).unwrap();
    let thread_context = context["contextId"].as_str().unwrap();
    assert!(thread_context.starts_with("thread-context:sha256:"));
    let aliases = context["context"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| member["memberAlias"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        aliases,
        std::collections::BTreeSet::from(["consumer", "provider"])
    );
    assert_eq!(
        fs::read_dir(
            state_root
                .join("threads")
                .join(thread_component)
                .join("contexts")
        )
        .unwrap()
        .count(),
        1
    );

    let rejected_plan = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "plan",
            "validate",
            "--session",
            &sessions[0],
            "--context",
            thread_context,
            "--plan",
            repositories[0].join("service-marker.py").to_str().unwrap(),
        ],
        Some(&poison_bin),
    );
    assert!(!rejected_plan.status.success());
    let rejected_plan: Value = serde_json::from_slice(&rejected_plan.stdout).unwrap();
    assert_eq!(rejected_plan["error"]["code"], "PRECONDITION_FAILED");

    let rejected = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "task-run",
            "start",
            "--session",
            &sessions[0],
            "--context",
            thread_context,
            "--plan",
            "plan:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ],
        Some(&poison_bin),
    );
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected["error"]["code"], "PRECONDITION_FAILED");

    let rejected_publish = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "session",
            "publish",
            "--session",
            &thread,
            "--run",
            "run:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ],
        Some(&poison_bin),
    );
    assert!(!rejected_publish.status.success());
    let rejected_publish: Value = serde_json::from_slice(&rejected_publish.stdout).unwrap();
    assert_eq!(rejected_publish["error"]["code"], "PRECONDITION_FAILED");

    let before = sessions
        .iter()
        .map(|session| {
            let component = session.strip_prefix("session:").unwrap();
            managed_file_snapshot(&state_root.join("sessions").join(component))
        })
        .collect::<Vec<_>>();
    for operation in ["close", "gc"] {
        let output = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &["thread", operation, "--thread", &thread],
            Some(&poison_bin),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let after = sessions
        .iter()
        .map(|session| {
            let component = session.strip_prefix("session:").unwrap();
            managed_file_snapshot(&state_root.join("sessions").join(component))
        })
        .collect::<Vec<_>>();
    assert_eq!(before, after);

    let terminal_context = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "context",
            "--thread",
            &thread,
            "--intent",
            "Must not publish after terminal transition",
            "--term",
            "Service",
        ],
        Some(&poison_bin),
    );
    assert!(!terminal_context.status.success());
    let terminal_context: Value = serde_json::from_slice(&terminal_context.stdout).unwrap();
    assert_eq!(terminal_context["error"]["code"], "PRECONDITION_FAILED");
    for name in [
        "cargo", "rustc", "gradle", "gradlew", "mvn", "mvnw", "java", "python", "python3",
    ] {
        assert!(!poison_bin.join(format!("{name}.executed")).exists());
    }

    let closed_member = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &["session", "close", "--session", &sessions[1]],
        Some(&poison_bin),
    );
    assert!(closed_member.status.success());
    let partial = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "context",
            "--thread",
            &partial_thread,
            "--intent",
            "Partial member failure must not publish a composite",
            "--term",
            "Service",
        ],
        Some(&poison_bin),
    );
    assert!(!partial.status.success());
    let partial: Value = serde_json::from_slice(&partial.stdout).unwrap();
    assert_eq!(partial["error"]["code"], "PRECONDITION_FAILED");
    let partial_component = partial_thread.strip_prefix("thread:").unwrap();
    assert_eq!(
        fs::read_dir(
            state_root
                .join("threads")
                .join(partial_component)
                .join("contexts")
        )
        .unwrap()
        .count(),
        0
    );
}

#[test]
fn managed_thread_accepts_same_repository_python_and_rust_units_without_collision() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = temporary.path().join("mixed-language-repository");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::write(
        repository.join("Cargo.toml"),
        b"[package]\nname = \"mixed\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
    )
    .unwrap();
    fs::write(repository.join("src/lib.rs"), b"pub fn shared() {}\n").unwrap();
    fs::write(
        repository.join("src/module.py"),
        b"def shared():\n    pass\n",
    )
    .unwrap();
    let lock = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(&repository)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(lock.success());
    run_git(&repository, &["init", "-q", "-b", "main"]);
    run_git(&repository, &["add", "."]);
    run_git(
        &repository,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@localhost",
            "commit",
            "-q",
            "-m",
            "mixed fixture",
        ],
    );

    let state_root = temporary.path().join("state/v2");
    let digest = "1".repeat(64);
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
    let poison_bin = temporary.path().join("exact-poison-bin");
    fs::create_dir(&poison_bin).unwrap();
    let poisoned_tools = [
        "cargo", "rustc", "gradle", "gradlew", "mvn", "mvnw", "java", "python", "python3", "git",
        "sh", "bash",
    ];
    for name in poisoned_tools {
        let executable = poison_bin.join(name);
        fs::write(&executable, b"#!/bin/sh\ntouch \"$0.executed\"\nexit 97\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        }
    }
    let mut sessions = Vec::new();
    for (language, compilation) in [
        ("python", "python:.#src"),
        ("rust", "cargo:Cargo.toml#mixed#lib#mixed"),
    ] {
        let opened = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &[
                "session",
                "open",
                "--repo",
                repository.to_str().unwrap(),
                "--target-ref",
                "main",
                "--language",
                language,
                "--compilation",
                compilation,
            ],
            None,
        );
        assert!(
            opened.status.success(),
            "{}",
            String::from_utf8_lossy(&opened.stdout)
        );
        let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
        let session = opened["session"]["sessionId"].as_str().unwrap().to_owned();
        let primed = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &[
                "context",
                "create",
                "--session",
                &session,
                "--intent",
                "Prime mixed language generation",
                "--term",
                "shared",
            ],
            None,
        );
        assert!(
            primed.status.success(),
            "{}",
            String::from_utf8_lossy(&primed.stdout)
        );
        sessions.push(session);
    }

    let generation_before = sessions
        .iter()
        .map(|session| {
            fs::read(
                state_root
                    .join("sessions")
                    .join(session.strip_prefix("session:").unwrap())
                    .join("generation.json"),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    let python = format!("python={}", sessions[0]);
    let rust = format!("rust={}", sessions[1]);
    let opened = run_managed_exact_path(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &["thread", "open", "--member", &python, "--member", &rust],
        &poison_bin,
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stdout)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
    let members = opened["thread"]["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(
        members[0]["session"]["repositoryKey"],
        members[1]["session"]["repositoryKey"]
    );
    let languages = members
        .iter()
        .map(|member| member["session"]["language"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        languages,
        std::collections::BTreeSet::from(["PYTHON", "RUST"])
    );

    let thread = opened["thread"]["threadId"].as_str().unwrap().to_owned();
    let context = run_managed_exact_path(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "context",
            "--thread",
            &thread,
            "--intent",
            "Trace shared behavior across Python and Rust",
            "--term",
            "shared",
        ],
        &poison_bin,
    );
    assert!(
        context.status.success(),
        "{}",
        String::from_utf8_lossy(&context.stdout)
    );
    assert!(context.stdout.len() <= 64 * 1024);
    let context_value: Value = serde_json::from_slice(&context.stdout).unwrap();
    let context_languages = context_value["context"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| member["language"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        context_languages,
        BTreeSet::from(["language:python", "language:rust"])
    );
    let aliases = context_value["context"]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact["memberAlias"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(aliases, BTreeSet::from(["python", "rust"]));
    let generation_after = sessions
        .iter()
        .map(|session| {
            fs::read(
                state_root
                    .join("sessions")
                    .join(session.strip_prefix("session:").unwrap())
                    .join("generation.json"),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(generation_before, generation_after);

    let thread_root = state_root
        .join("threads")
        .join(thread.strip_prefix("thread:").unwrap());
    let context_record = fs::read(
        fs::read_dir(thread_root.join("contexts"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
    )
    .unwrap();
    let authority_record = fs::read(thread_root.join("authority.json")).unwrap();
    let roots = vec![authority_record.clone(), context_record.clone()];
    let closure_before = rooted_cas_closure(&state_root, &roots);
    let schemas = closure_before
        .values()
        .map(|(schema, _)| schema.as_str())
        .collect::<BTreeSet<_>>();
    assert!(schemas.contains("codeclew-thread-context-evidence/1.0"));
    assert!(schemas.contains("codeclew-context-evidence-object/3.0"));
    assert!(closure_before.len() >= 3);
    for bytes in roots
        .iter()
        .chain(closure_before.values().map(|(_, bytes)| bytes))
    {
        assert_bytes_hide_paths(
            bytes,
            &[temporary.path(), &repository, &state_root, &runtime],
        );
    }

    for operation in ["close", "gc"] {
        let output = run_managed_exact_path(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &["thread", operation, "--thread", &thread],
            &poison_bin,
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    assert_eq!(closure_before, rooted_cas_closure(&state_root, &roots));
    for name in poisoned_tools {
        assert!(!poison_bin.join(format!("{name}.executed")).exists());
    }
}

#[cfg(unix)]
#[test]
fn managed_source_locate_reads_the_bound_context_snapshot() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let repository = temporary.path().join("literal-locate-repository");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::write(
        repository.join("Cargo.toml"),
        b"[package]\nname='locate-fixture'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    let original = b"pub const FIRST: &str = \"prod-trace.json\";\r\npub const SECOND: &str = \"prod-trace.json\";\r\n";
    fs::write(repository.join("src/lib.rs"), original).unwrap();
    let lock = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(&repository)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(lock.success());
    run_git(&repository, &["init", "-q", "-b", "main"]);
    run_git(&repository, &["add", "."]);
    run_git(
        &repository,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@localhost",
            "commit",
            "-q",
            "-m",
            "literal fixture",
        ],
    );

    let state_root = temporary.path().join("state/v2");
    let runtime_digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&runtime_digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{runtime_digest}.lease"));

    let opened = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "session",
            "open",
            "--repo",
            repository.to_str().unwrap(),
            "--target-ref",
            "main",
            "--language",
            "rust",
            "--compilation",
            "cargo:Cargo.toml#locate-fixture#lib#locate_fixture",
        ],
        None,
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stdout)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
    let session = opened["session"]["sessionId"].as_str().unwrap();
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
            "Bind the exact literal source snapshot",
            "--term",
            "FIRST",
        ],
        None,
    );
    assert!(
        context.status.success(),
        "{}",
        String::from_utf8_lossy(&context.stdout)
    );
    let context: Value = serde_json::from_slice(&context.stdout).unwrap();
    let context_id = context["contextId"].as_str().unwrap();

    let request = temporary.path().join("source-locate.json");
    let request_value = json!({
        "schema":"codeclew-source-locate-request/1.0",
        "literal":"prod-trace.json",
        "paths":["src/lib.rs"],
        "maxMatches":2,
    });
    fs::write(&request, canonical::bytes(&request_value).unwrap()).unwrap();
    fs::set_permissions(&request, fs::Permissions::from_mode(0o600)).unwrap();

    fs::write(
        repository.join("src/lib.rs"),
        b"pub const LIVE_ONLY: &str = \"changed-after-context\";\n",
    )
    .unwrap();
    let located = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "nav",
            "locate",
            "--session",
            session,
            "--from",
            context_id,
            "--request",
            request.to_str().unwrap(),
        ],
        None,
    );
    assert!(
        located.status.success(),
        "{}",
        String::from_utf8_lossy(&located.stdout)
    );
    assert!(located.stdout.len() <= 64 * 1024);
    let located: Value = serde_json::from_slice(&located.stdout).unwrap();
    assert_eq!(located["schema"], "codeclew-source-locate-result/1.0");
    assert_eq!(located["status"], "COMPLETE");
    assert_eq!(located["completeness"], "QUERY_COMPLETE");
    assert_eq!(located["authority"], "SNAPSHOT_EXACT_BYTES");
    assert_eq!(located["semanticResolution"], "NONE");
    assert_eq!(located["source"]["contextId"], context_id);
    assert_eq!(located["observedMatchCount"], 2);
    assert_eq!(located["matches"][0]["path"], "src/lib.rs");
    assert_eq!(located["matches"][0]["byteStart"], 25);
    assert_eq!(located["matches"][1]["byteStart"], 70);
    assert_eq!(
        located["matches"][0]["contentDigest"],
        canonical::hash_bytes(original)
    );
    let stdout = located.to_string();
    assert!(!stdout.contains("changed-after-context"));
    assert!(!stdout.contains(repository.to_str().unwrap()));
    assert!(!stdout.contains("prod-trace.json"));
}

#[cfg(unix)]
#[test]
fn managed_direct_source_locate_reads_the_pinned_git_tree_without_context() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let repository = temporary.path().join("direct-literal-locate-repository");
    fs::create_dir(&repository).unwrap();
    fs::write(repository.join("Trace.kt"), b"before trace.json after\n").unwrap();
    run_git(&repository, &["init", "-q", "-b", "main"]);
    run_git(&repository, &["add", "."]);
    run_git(
        &repository,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@localhost",
            "commit",
            "-q",
            "-m",
            "direct literal fixture",
        ],
    );
    let repository = repository.canonicalize().unwrap();

    let state_root = temporary.path().join("state/v2");
    let runtime_digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&runtime_digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{runtime_digest}.lease"));

    let request = temporary.path().join("direct-source-locate.json");
    let request_value = json!({
        "schema":"codeclew-source-locate-request/1.0",
        "literal":"trace.json",
        "paths":["Trace.kt"],
        "maxMatches":2,
    });
    fs::write(&request, canonical::bytes(&request_value).unwrap()).unwrap();
    fs::set_permissions(&request, fs::Permissions::from_mode(0o600)).unwrap();

    let located = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "nav",
            "locate",
            "--repo",
            repository.to_str().unwrap(),
            "--target-ref",
            "main",
            "--request",
            request.to_str().unwrap(),
        ],
        None,
    );
    assert!(
        located.status.success(),
        "{}",
        String::from_utf8_lossy(&located.stdout)
    );
    assert!(located.stdout.len() <= 64 * 1024);
    let located: Value = serde_json::from_slice(&located.stdout).unwrap();
    assert_eq!(located["schema"], "codeclew-source-locate-result/1.1");
    assert_eq!(located["status"], "COMPLETE");
    assert_eq!(located["authority"], "SNAPSHOT_EXACT_BYTES");
    assert_eq!(located["semanticResolution"], "NONE");
    assert_eq!(located["source"]["mode"], "DIRECT_GIT_COMMIT");
    assert_eq!(
        located["source"]["schema"],
        "codeclew-direct-git-source-authority/1.0"
    );
    assert_eq!(located["observedMatchCount"], 1);
    assert_eq!(located["matches"][0]["path"], "Trace.kt");
    assert_eq!(located["matches"][0]["byteStart"], 7);
    assert_eq!(located["matches"][0]["byteEnd"], 17);
    assert_eq!(
        located["matches"][0]["contentDigest"],
        canonical::hash_bytes(b"before trace.json after\n")
    );
    for field in [
        "authorityDigest",
        "targetRefDigest",
        "baseRevision",
        "treeOid",
    ] {
        assert!(located["source"][field].as_str().is_some());
    }
    let stdout = located.to_string();
    assert!(!stdout.contains(repository.to_str().unwrap()));
    assert!(!stdout.contains("refs/heads/main"));
    assert!(!stdout.contains("trace.json"));
}

#[test]
fn managed_thread_impact_seed_helper() {
    let Some(thread_id) = std::env::var_os("CODECLEW_SYNTHETIC_CALLABLE_THREAD") else {
        return;
    };
    let result_path = std::env::var_os("CODECLEW_SYNTHETIC_CALLABLE_RESULT").unwrap();
    let variant =
        std::env::var("CODECLEW_SYNTHETIC_CALLABLE_VARIANT").unwrap_or_else(|_| "same".into());
    let state = StateAuthority::process_default().unwrap();
    let store = CasStore::open(&state).unwrap();
    let (thread, _) = ThreadAuthority::load(thread_id.to_str().unwrap()).unwrap();
    let root = seed_synthetic_callable_fact_set(&state, &store, &thread, &variant);
    fs::write(result_path, root.fact_set_id).unwrap();
}

#[test]
fn managed_cas_closure_helper() {
    let Some(request_path) = std::env::var_os("CODECLEW_CAS_CLOSURE_REQUEST") else {
        return;
    };
    let result_path = std::env::var_os("CODECLEW_CAS_CLOSURE_RESULT").unwrap();
    let roots: Vec<Vec<u8>> = serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap();
    let state = StateAuthority::process_default().unwrap();
    let store = CasStore::open(&state).unwrap();
    let closure = rooted_cas_closure_from_store(&store, &roots);
    fs::write(result_path, serde_json::to_vec(&closure).unwrap()).unwrap();
}

#[test]
fn managed_storage_gc_is_dry_run_until_apply() {
    let temporary = tempfile::tempdir().unwrap();
    let state_root = temporary.path().join("state");
    let object = CasObject::for_bytes("test/managed-storage/1", b"unrooted").unwrap();
    let component = object.digest.strip_prefix("sha256:").unwrap();
    let loose = state_root
        .join("objects/sha256")
        .join(&component[..2])
        .join(&component[2..]);
    fs::create_dir_all(loose.parent().unwrap()).unwrap();
    fs::write(&loose, b"unrooted").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&loose, fs::Permissions::from_mode(0o600)).unwrap();
    }

    let runtime_root = temporary.path().join("runtime");
    let binary = fd_runtime(&runtime_root);
    let _writable_runtime = WritableTreeOnDrop(runtime_root.clone());
    let lease = temporary.path().join("runtime.lease");

    let dry_run = run_managed(
        &binary,
        &state_root,
        &runtime_root,
        &lease,
        &["storage", "gc"],
        None,
    );
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stdout)
    );
    let dry_run: Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(dry_run["action"], "DRY_RUN");
    assert_eq!(dry_run["reclaimableLooseObjects"], 1);
    assert_eq!(dry_run["reclaimedBytes"], 0);

    let applied = run_managed(
        &binary,
        &state_root,
        &runtime_root,
        &lease,
        &["storage", "gc", "--apply"],
        None,
    );
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stdout)
    );
    let applied: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["schema"], "codeclew-storage-report/2.1");
    assert_eq!(applied["action"], "APPLIED");
    assert!(applied["reclaimedBytes"].as_u64().unwrap() > 0);
    assert_eq!(applied["catalogSnapshotDeferred"], false);
    assert_eq!(applied["deferredSnapshotBytes"], 0);
    assert_eq!(applied["catalogTailBytes"], 0);
}

#[test]
fn managed_thread_impact_uses_seeded_s1_without_project_processes() {
    let temporary = tempfile::tempdir().unwrap();
    let repositories = [
        temporary.path().join("provider-private-repository"),
        temporary.path().join("consumer-private-repository"),
    ];
    for repository in &repositories {
        fs::create_dir_all(repository.join("src")).unwrap();
        fs::write(repository.join("README.md"), b"synthetic impact fixture\n").unwrap();
        run_git(repository, &["init", "-q", "-b", "main"]);
        run_git(repository, &["add", "."]);
        run_git(
            repository,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-q",
                "-m",
                "impact fixture",
            ],
        );
    }

    let state_root = temporary.path().join("state/v2");
    let runtime_digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&runtime_digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime_binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{runtime_digest}.lease"));
    let poison_bin = temporary.path().join("project-process-poison");
    fs::create_dir(&poison_bin).unwrap();
    let poisoned_tools = [
        "cargo", "rustc", "gradle", "gradlew", "mvn", "mvnw", "java", "python", "python3",
    ];
    for name in poisoned_tools {
        let executable = poison_bin.join(name);
        fs::write(
            &executable,
            b"#!/bin/sh\n/usr/bin/touch \"$0.executed\"\nexit 97\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        }
    }

    let mut sessions = Vec::new();
    for repository in &repositories {
        let opened = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &[
                "session",
                "open",
                "--repo",
                repository.to_str().unwrap(),
                "--target-ref",
                "main",
                "--language",
                "kotlin",
                "--compilation",
                ":/main",
            ],
            Some(&poison_bin),
        );
        assert!(
            opened.status.success(),
            "{}",
            String::from_utf8_lossy(&opened.stdout)
        );
        let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
        sessions.push(opened["session"]["sessionId"].as_str().unwrap().to_owned());
    }
    let provider = format!("provider={}", sessions[0]);
    let consumer = format!("consumer={}", sessions[1]);
    let opened = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread", "open", "--member", &provider, "--member", &consumer,
        ],
        Some(&poison_bin),
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stdout)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
    let thread_id = opened["thread"]["threadId"].as_str().unwrap().to_owned();

    let seed_result = temporary.path().join("synthetic-fact-set-id");
    run_callable_seed_helper(&state_root, &thread_id, &seed_result, "same");
    let fact_set_id = fs::read_to_string(&seed_result).unwrap();
    assert!(fact_set_id.starts_with("thread-callables:sha256:"));

    let before_invalid = managed_file_snapshot(&state_root);
    let invalid = run_managed_exact_path(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "impact",
            "--thread",
            &thread_id,
            "--fact-set",
            &fact_set_id,
            "--pair-id",
            "provider-consumer",
            "--subject-kind",
            "callable-family",
            "--subject",
            "p/Orders.findOrder",
            "--member",
            "provider",
        ],
        &poison_bin,
    );
    assert!(!invalid.status.success());
    let invalid: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(invalid["error"]["code"], "INVALID_INPUT");
    assert_eq!(before_invalid, managed_file_snapshot(&state_root));

    let impact = run_managed_exact_path(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "impact",
            "--thread",
            &thread_id,
            "--fact-set",
            &fact_set_id,
            "--pair-id",
            "provider-consumer",
            "--subject-kind",
            "callable-family",
            "--subject",
            "p/Orders.findOrder",
        ],
        &poison_bin,
    );
    assert!(
        impact.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&impact.stdout),
        String::from_utf8_lossy(&impact.stderr),
    );
    assert!(impact.stdout.len() <= 64 * 1024);
    assert_bytes_hide_paths(
        &impact.stdout,
        &[
            temporary.path(),
            &repositories[0],
            &repositories[1],
            &state_root,
            &runtime,
        ],
    );
    let encoded = String::from_utf8(impact.stdout.clone()).unwrap();
    assert!(!encoded.contains("/Users/"));
    assert!(!encoded.contains("/private/"));
    assert!(!encoded.contains("://"));
    let impact: Value = serde_json::from_slice(&impact.stdout).unwrap();
    assert_eq!(impact["schema"], "codeclew-thread-impact-result/1.0");
    assert_eq!(impact["factSetId"], fact_set_id);
    assert_eq!(impact["impact"]["subjectKind"], "CALLABLE_FAMILY");
    assert_eq!(
        impact["impact"]["relationshipAuthority"],
        "DECLARED_TOPOLOGY"
    );
    assert_eq!(
        impact["impact"]["shapeStatus"],
        "EXACT_PROJECTED_SHAPE_EQUAL"
    );
    assert_eq!(impact["impact"]["certainty"], "UNSURE");
    let members = impact["impact"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| member["memberAlias"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(members, BTreeSet::from(["consumer", "provider"]));
    assert!(impact["impact"]["findingCount"].as_u64().unwrap() >= 2);
    assert!(!impact["impact"]["findings"].as_array().unwrap().is_empty());
    assert!(
        !impact["impact"]["sourceWindows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        impact["impact"]["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|obligation| obligation["code"] == "VERIFY_RELATIONSHIP_AUTHORITY")
    );
    let impact_component = impact["impactId"]
        .as_str()
        .unwrap()
        .strip_prefix("thread-impact:sha256:")
        .unwrap();
    let thread_component = thread_id.strip_prefix("thread:").unwrap();
    assert!(
        state_root
            .join("threads")
            .join(thread_component)
            .join("impacts")
            .join(format!("{impact_component}.json"))
            .is_file()
    );
    for name in poisoned_tools {
        assert!(!poison_bin.join(format!("{name}.executed")).exists());
    }
}

#[test]
fn managed_thread_validate_compares_two_revisions_without_project_processes() {
    let temporary = tempfile::tempdir().unwrap();
    let repositories = [
        temporary.path().join("provider-private-repository"),
        temporary.path().join("consumer-private-repository"),
    ];
    for repository in &repositories {
        fs::create_dir_all(repository.join("src")).unwrap();
        fs::write(repository.join("README.md"), b"before revision\n").unwrap();
        run_git(repository, &["init", "-q", "-b", "main"]);
        run_git(repository, &["add", "."]);
        run_git(
            repository,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-q",
                "-m",
                "before revision",
            ],
        );
    }

    let state_root = temporary.path().join("state/v2");
    let runtime_digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&runtime_digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime_binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{runtime_digest}.lease"));
    let poison_bin = temporary.path().join("validation-process-poison");
    fs::create_dir(&poison_bin).unwrap();
    let poisoned_tools = [
        "cargo",
        "rustc",
        "git",
        "gradle",
        "gradlew",
        "mvn",
        "mvnw",
        "java",
        "python",
        "python3",
        "kotlinc",
        "semanticd",
    ];
    for name in poisoned_tools {
        let executable = poison_bin.join(name);
        fs::write(
            &executable,
            b"#!/bin/sh\n/usr/bin/touch \"$0.executed\"\nexit 97\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        }
    }

    let open_sessions = |label: &str| -> Vec<String> {
        repositories
            .iter()
            .map(|repository| {
                let opened = run_managed(
                    &runtime_binary,
                    &state_root,
                    &runtime,
                    &lease,
                    &[
                        "session",
                        "open",
                        "--repo",
                        repository.to_str().unwrap(),
                        "--target-ref",
                        "main",
                        "--language",
                        "kotlin",
                        "--compilation",
                        ":/main",
                    ],
                    None,
                );
                assert!(
                    opened.status.success(),
                    "{label} session open failed: {}",
                    String::from_utf8_lossy(&opened.stdout)
                );
                serde_json::from_slice::<Value>(&opened.stdout).unwrap()["session"]["sessionId"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    let before_sessions = open_sessions("before");
    let open_thread = |sessions: &[String]| -> String {
        let provider = format!("provider={}", sessions[0]);
        let consumer = format!("consumer={}", sessions[1]);
        let opened = run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &[
                "thread", "open", "--member", &provider, "--member", &consumer,
            ],
            None,
        );
        assert!(
            opened.status.success(),
            "{}",
            String::from_utf8_lossy(&opened.stdout)
        );
        serde_json::from_slice::<Value>(&opened.stdout).unwrap()["thread"]["threadId"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let before_thread = open_thread(&before_sessions);
    let before_seed = temporary.path().join("before-fact-set-id");
    run_callable_seed_helper(&state_root, &before_thread, &before_seed, "before");
    let before_fact_set = fs::read_to_string(&before_seed).unwrap();
    let before_impact = run_managed_exact_path(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "impact",
            "--thread",
            &before_thread,
            "--fact-set",
            &before_fact_set,
            "--pair-id",
            "provider-consumer",
            "--subject-kind",
            "callable-family",
            "--subject",
            "p/Orders.findOrder",
        ],
        &poison_bin,
    );
    assert!(before_impact.status.success());
    let before_impact = serde_json::from_slice::<Value>(&before_impact.stdout).unwrap()["impactId"]
        .as_str()
        .unwrap()
        .to_owned();

    for repository in &repositories {
        fs::write(repository.join("README.md"), b"after revision\n").unwrap();
        run_git(repository, &["add", "README.md"]);
        run_git(
            repository,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-q",
                "-m",
                "after revision",
            ],
        );
    }
    let after_sessions = open_sessions("after");
    let after_thread = open_thread(&after_sessions);
    let after_seed = temporary.path().join("after-fact-set-id");
    run_callable_seed_helper(&state_root, &after_thread, &after_seed, "after");
    let after_fact_set = fs::read_to_string(&after_seed).unwrap();
    let after_impact = run_managed_exact_path(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "thread",
            "impact",
            "--thread",
            &after_thread,
            "--fact-set",
            &after_fact_set,
            "--pair-id",
            "provider-consumer",
            "--subject-kind",
            "callable-family",
            "--subject",
            "p/Orders.findOrder",
        ],
        &poison_bin,
    );
    assert!(after_impact.status.success());
    let after_impact = serde_json::from_slice::<Value>(&after_impact.stdout).unwrap()["impactId"]
        .as_str()
        .unwrap()
        .to_owned();

    let repository_before_validation = repositories
        .iter()
        .map(|repository| managed_file_snapshot(repository))
        .collect::<Vec<_>>();
    let empty_coverage = temporary.path().join("empty-coverage.json");
    fs::write(
        &empty_coverage,
        canonical::bytes(&json!({
            "schema":"codeclew-kotlin-change-coverage-document/1.0",
            "entries":[],
        }))
        .unwrap(),
    )
    .unwrap();
    let validate = |coverage: &Path| {
        run_managed_exact_path(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &[
                "thread",
                "validate",
                "--before-thread",
                &before_thread,
                "--before-impact",
                &before_impact,
                "--after-thread",
                &after_thread,
                "--after-impact",
                &after_impact,
                "--member-correspondence",
                "provider=provider",
                "--member-correspondence",
                "consumer=consumer",
                "--coverage",
                coverage.to_str().unwrap(),
            ],
            &poison_bin,
        )
    };
    let incomplete_output = validate(&empty_coverage);
    assert!(
        incomplete_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&incomplete_output.stdout),
        String::from_utf8_lossy(&incomplete_output.stderr),
    );
    assert!(incomplete_output.stdout.len() <= 64 * 1024);
    assert_bytes_hide_paths(
        &incomplete_output.stdout,
        &[
            temporary.path(),
            &repositories[0],
            &repositories[1],
            &state_root,
            &runtime,
        ],
    );
    let incomplete: Value = serde_json::from_slice(&incomplete_output.stdout).unwrap();
    assert_eq!(
        incomplete["schema"],
        "codeclew-thread-change-coverage-result/1.0"
    );
    assert_eq!(incomplete["coverage"]["status"], "INCOMPLETE");
    let missing = incomplete["coverage"]["missingTargets"].as_array().unwrap();
    assert!(missing.len() >= 3);
    let entries = missing
        .iter()
        .enumerate()
        .map(|(index, target)| {
            json!({
                "targetId":target["targetId"],
                "requiredCategories":target["requiredCategories"],
                "handling":{"kind":"EXTERNAL_WORK","id":format!("verify-{index}")},
            })
        })
        .collect::<Vec<_>>();
    let complete_coverage = temporary.path().join("complete-coverage.json");
    fs::write(
        &complete_coverage,
        canonical::bytes(&json!({
            "schema":"codeclew-kotlin-change-coverage-document/1.0",
            "entries":entries,
        }))
        .unwrap(),
    )
    .unwrap();
    let complete_first = validate(&complete_coverage);
    assert!(complete_first.status.success());
    let complete_second = validate(&complete_coverage);
    assert!(complete_second.status.success());
    assert_eq!(complete_first.stdout, complete_second.stdout);
    let complete: Value = serde_json::from_slice(&complete_first.stdout).unwrap();
    assert_eq!(complete["coverage"]["status"], "VALIDATED_CONDITIONAL");
    assert_eq!(
        complete["coverage"]["comparisonDigest"],
        incomplete["coverage"]["comparisonDigest"]
    );
    assert_ne!(complete["changeSetId"], incomplete["changeSetId"]);

    let mut omitted_entries = complete["coverage"]["coveredTargetIds"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let target_id = target.as_str().unwrap();
            let source = missing
                .iter()
                .find(|row| row["targetId"].as_str() == Some(target_id))
                .unwrap();
            json!({
                "targetId":target_id,
                "requiredCategories":source["requiredCategories"],
                "handling":{"kind":"ACTION","id":format!("review-{index}")},
            })
        })
        .collect::<Vec<_>>();
    omitted_entries.pop();
    let omitted_coverage = temporary.path().join("omitted-coverage.json");
    fs::write(
        &omitted_coverage,
        canonical::bytes(&json!({
            "schema":"codeclew-kotlin-change-coverage-document/1.0",
            "entries":omitted_entries,
        }))
        .unwrap(),
    )
    .unwrap();
    let omitted = validate(&omitted_coverage);
    assert!(omitted.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&omitted.stdout).unwrap()["coverage"]["status"],
        "INCOMPLETE"
    );

    let thread_component = after_thread.strip_prefix("thread:").unwrap();
    let change_set_directory = state_root
        .join("threads")
        .join(thread_component)
        .join("change-sets");
    let valid_root_count = fs::read_dir(&change_set_directory).unwrap().count();
    let invalid_coverage = temporary.path().join("invalid-coverage.json");
    fs::write(
        &invalid_coverage,
        canonical::bytes(&json!({
            "schema":"codeclew-kotlin-change-coverage-document/1.0",
            "entries":[{
                "targetId":missing[0]["targetId"],
                "requiredCategories":missing[0]["requiredCategories"],
                "handling":{"kind":"ACTION","id":"run;command"},
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    let invalid = validate(&invalid_coverage);
    assert!(!invalid.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stdout).unwrap()["error"]["code"],
        "INVALID_INPUT"
    );
    assert_eq!(
        fs::read_dir(&change_set_directory).unwrap().count(),
        valid_root_count
    );
    assert_eq!(
        repository_before_validation,
        repositories
            .iter()
            .map(|repository| managed_file_snapshot(repository))
            .collect::<Vec<_>>()
    );
    for name in poisoned_tools {
        assert!(!poison_bin.join(format!("{name}.executed")).exists());
    }

    let change_set_component = complete["changeSetId"]
        .as_str()
        .unwrap()
        .strip_prefix("thread-coverage:sha256:")
        .unwrap();
    let retained_root =
        fs::read(change_set_directory.join(format!("{change_set_component}.json"))).unwrap();
    let retained_closure = rooted_cas_closure(&state_root, std::slice::from_ref(&retained_root));
    assert!(!retained_closure.is_empty());
    for (_digest, (_schema, bytes)) in retained_closure {
        assert_bytes_hide_paths(
            &bytes,
            &[
                temporary.path(),
                &repositories[0],
                &repositories[1],
                &state_root,
                &runtime,
            ],
        );
    }
}

#[cfg(unix)]
#[test]
fn managed_java17_maven_local_config_returns_indexed_source_without_commits() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let repository = temporary.path().join("repository");
    let files = [
        (
            "pom.xml",
            "<project><modelVersion>4.0.0</modelVersion></project>",
        ),
        (
            "codeclew.yaml",
            "maven:\n  settings: missing-settings.xml\n",
        ),
        (
            "src/main/java/org/springframework/stereotype/Controller.java",
            "package org.springframework.stereotype; @java.lang.annotation.Retention(java.lang.annotation.RetentionPolicy.RUNTIME) public @interface Controller {}",
        ),
        (
            "src/main/java/org/springframework/web/bind/annotation/RestController.java",
            "package org.springframework.web.bind.annotation; @org.springframework.stereotype.Controller @java.lang.annotation.Retention(java.lang.annotation.RetentionPolicy.RUNTIME) public @interface RestController {}",
        ),
        (
            "src/main/java/org/springframework/web/bind/annotation/RequestMapping.java",
            "package org.springframework.web.bind.annotation; @java.lang.annotation.Retention(java.lang.annotation.RetentionPolicy.RUNTIME) public @interface RequestMapping { String[] path() default {}; String[] value() default {}; }",
        ),
        (
            "src/main/java/example/OwnerController.java",
            "package example;\n// Unicode before the declaration: 🩺 Café\nimport org.springframework.web.bind.annotation.*;\n@RestController\npublic class OwnerController {\n    @RequestMapping(path = \"/owners\")\n    public OwnerDto listOwners() {\n        return new OwnerDto(\"Ada\");\n    }\n}\n",
        ),
        (
            "mvnw",
            r##"#!/bin/sh
set -eu
test "$1" = --settings
grep -q '<validationRelease>17</validationRelease>' "$2"
case "$*" in
  *help:effective-pom*)
    effective_output=
    for argument in "$@"; do
      case "$argument" in -Doutput=*) effective_output=${argument#-Doutput=} ;; esac
    done
    test -n "$effective_output"
    module=$(pwd -P)
    cat > "$effective_output" <<EOF
<project><build>
  <directory>$module/target</directory>
  <sourceDirectory>$module/src/main/java</sourceDirectory>
  <testSourceDirectory>$module/src/test/java</testSourceDirectory>
  <outputDirectory>$module/target/classes</outputDirectory>
  <testOutputDirectory>$module/target/test-classes</testOutputDirectory>
</build></project>
EOF
    ;;
  *dependency:build-classpath*)
    mkdir -p target/classes target/generated-sources/example
    printf '%s\n' 'package example; public record OwnerDto(String name) {}' > target/generated-sources/example/OwnerDto.java
    if test -n "${JAVA_HOME:-}"; then compiler="$JAVA_HOME/bin/javac"; else compiler=javac; fi
    find src/main/java target/generated-sources -name '*.java' > target/sources.txt
    "$compiler" --release 17 -encoding UTF-8 -d target/classes @target/sources.txt
    printf '\n' > target/codeclew-classpath.txt
    ;;
  *help:evaluate*) printf '17\n' ;;
  *) exit 25 ;;
esac
"##,
        ),
    ];
    for (name, content) in files {
        let path = repository.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    fs::set_permissions(repository.join("mvnw"), fs::Permissions::from_mode(0o644)).unwrap();
    run_git(&repository, &["init", "-q", "-b", "main"]);
    run_git(&repository, &["add", "."]);
    run_git(
        &repository,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    let settings = temporary.path().join("private-settings.xml");
    fs::write(&settings, "<settings><profiles><profile><properties><validationRelease>17</validationRelease></properties></profile></profiles></settings>").unwrap();
    // This config differs from HEAD. Neither it nor the mode-0644 wrapper needs
    // a preparation commit, --committed, chmod, or a change to user settings.
    fs::write(
        repository.join("codeclew.yaml"),
        "version: 1\nmaven:\n  settings: ../private-settings.xml\n",
    )
    .unwrap();
    let status = || {
        Command::new("git")
            .args(["status", "--porcelain=v1", "-z"])
            .current_dir(&repository)
            .output()
            .unwrap()
            .stdout
    };
    let before = status();
    let state_root = temporary.path().join("state/v2");
    let runtime_digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&runtime_digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{runtime_digest}.lease"));
    let run = |args: &[&str]| run_managed(&binary, &state_root, &runtime, &lease, args, None);
    let discovery = run(&[
        "doctor",
        "repository",
        "--repo",
        repository.to_str().unwrap(),
    ]);
    assert!(
        discovery.status.success(),
        "{}",
        String::from_utf8_lossy(&discovery.stdout)
    );
    let discovery: Value = serde_json::from_slice(&discovery.stdout).unwrap();
    assert_eq!(discovery["repository"]["clean"], false);
    assert_eq!(discovery["repository"]["analysisInputsClean"], true);
    let opened = run(&[
        "nav",
        "query",
        "--repo",
        repository.to_str().unwrap(),
        "--target-ref",
        "main",
        "--language",
        "java",
        "--profile",
        "java-17plus-maven-read-only",
        "--compilation",
        ":/main",
        "--term",
        "OwnerController",
        "--decision-identifier",
        "OwnerController",
        "--source",
    ]);
    assert!(
        opened.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&opened.stdout),
        String::from_utf8_lossy(&opened.stderr)
    );
    let opened: Value = serde_json::from_slice(&opened.stdout).unwrap();
    assert_eq!(opened["admission"]["status"], "PASS");
    assert_eq!(
        opened["navigation"]["decisionAuthority"]["status"], "SUPPORTED",
        "{opened}"
    );
    assert_eq!(
        opened["navigation"]["decisionSource"]["sourceDelivery"]["status"],
        "RETURNED"
    );
    let source = &opened["navigation"]["decisionSource"]["source"];
    assert_eq!(
        source["fileId"],
        "src/main/java/example/OwnerController.java"
    );
    assert!(source["windows"].to_string().contains("new OwnerDto"));
    let session = opened["session"]["sessionId"].as_str().unwrap();
    let catalog = run(&["entrypoints", "--session", session]);
    assert!(
        catalog.status.success(),
        "{}",
        String::from_utf8_lossy(&catalog.stdout)
    );
    let catalog: Value = serde_json::from_slice(&catalog.stdout).unwrap();
    assert_eq!(catalog["total"], 1);
    assert_eq!(catalog["entries"][0]["startLine"], 6);
    assert_eq!(catalog["scopes"][0]["generationCoverage"], "PARTIAL");
    assert_eq!(
        catalog["scopes"][0]["generationCertainty"], "UNSURE",
        "{catalog}"
    );
    assert!(
        catalog["scopes"][0]["generationObligations"]
            .to_string()
            .contains("LIMIT_DOCUMENTATION_TO_INDEXED_SOURCE_OBJECTS")
    );
    assert!(
        !catalog
            .to_string()
            .contains("FIX_JAVA_CLASSPATH_OR_DIAGNOSTIC")
    );
    assert_eq!(status(), before);
    assert!(!repository.join("target").exists());
    assert_eq!(
        fs::metadata(repository.join("mvnw"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    assert!(!opened.to_string().contains(settings.to_str().unwrap()));
    assert!(
        run(&["session", "close", "--session", session])
            .status
            .success()
    );
    assert!(
        run(&["session", "gc", "--session", session])
            .status
            .success()
    );
}

#[cfg(unix)]
#[test]
fn managed_operational_commands_are_path_free_and_support_recovery() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let repository = temporary.path().join("private-operational-repository");
    fs::create_dir(&repository).unwrap();
    fs::write(repository.join("README.md"), b"baseline\n").unwrap();
    fs::write(
        repository.join("pyproject.toml"),
        b"[project]\nname='fixture'\n",
    )
    .unwrap();
    fs::write(repository.join("app.py"), b"value = 1\n").unwrap();
    run_git(&repository, &["init", "-q", "-b", "main"]);
    run_git(&repository, &["add", "."]);
    run_git(
        &repository,
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

    fs::create_dir_all(repository.join("docs/plans")).unwrap();
    fs::write(repository.join("docs/plans/local.md"), b"local plan\n").unwrap();

    let state_root = temporary.path().join("state/v2");
    let runtime_digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&runtime_digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{runtime_digest}.lease"));

    let capabilities = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &["capabilities"],
        None,
    );
    assert!(capabilities.status.success());
    let capabilities_value: Value = serde_json::from_slice(&capabilities.stdout).unwrap();
    assert_eq!(capabilities_value["schema"], "codeclew-capabilities/1.0");
    assert_eq!(
        capabilities_value["productVersion"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        capabilities_value["supportMatrix"]["profiles"][0]["profileId"],
        "kotlin-2.4.10-gradle-single"
    );
    assert_eq!(
        capabilities_value["agentContract"]["schema"],
        "codeclew-agent-contract/1.0"
    );
    assert_eq!(
        capabilities_value["agentContract"]["launcherAuthority"],
        "INSTALLED_RELEASE"
    );
    assert_eq!(
        capabilities_value["agentContract"]["readinessSchema"],
        "codeclew-doctor/2.0"
    );
    assert_eq!(
        capabilities_value["agentContract"]["sourceFallbackAllowed"],
        false
    );
    assert_eq!(
        capabilities_value["agentContract"]["primaryOpenCommand"],
        "context open"
    );
    assert_eq!(
        capabilities_value["agentContract"]["mutationPrepareWaitsForActionableState"],
        true
    );
    assert!(
        capabilities_value["agentContract"]["skillDigest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:") && value.len() == 71)
    );
    let capabilities_human = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &["capabilities", "--human"],
        None,
    );
    assert!(capabilities_human.status.success());
    assert!(capabilities_human.stderr.is_empty());
    let capabilities_report = String::from_utf8(capabilities_human.stdout).unwrap();
    assert!(capabilities_report.contains("Codeclew capabilities"));
    assert!(capabilities_report.contains("Kotlin 2.4.10"));
    assert!(!capabilities_report.contains("codeclew-capabilities/1.0"));

    let attach = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &["doctor", "attach"],
        None,
    );
    assert!(attach.status.success());
    let attach_value: Value = serde_json::from_slice(&attach.stdout).unwrap();
    assert_eq!(attach_value["schema"], "codeclew-doctor/2.0");
    assert_eq!(attach_value["scope"], "ATTACH");
    assert_eq!(attach_value["nextAction"], "NONE");
    assert!(
        attach_value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| {
                !check["checkId"].as_str().unwrap().starts_with("tool.")
                    && check["checkId"] != "state.free-space"
            })
    );

    let repository_doctor = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "doctor",
            "repository",
            "--repo",
            repository.to_str().unwrap(),
        ],
        None,
    );
    assert!(repository_doctor.status.success());
    let repository_doctor_value: Value = serde_json::from_slice(&repository_doctor.stdout).unwrap();
    assert_eq!(
        repository_doctor_value["schema"],
        "codeclew-repository-diagnostic/1.0"
    );
    assert_eq!(repository_doctor_value["status"], "READY_FOR_TASK_DOCTOR");
    assert_eq!(repository_doctor_value["nextAction"], "RUN_TASK_DOCTOR");
    assert_eq!(
        repository_doctor_value["contours"][0]["profileId"],
        "python-syntax"
    );
    assert_eq!(
        repository_doctor_value["contours"][0]["compilations"][0],
        "python:.#."
    );
    assert_eq!(
        repository_doctor_value["privacyAssertions"]["containsAbsolutePaths"],
        false
    );
    assert_eq!(
        repository_doctor_value["privacyAssertions"]["containsRepositoryIdentity"],
        true
    );
    assert!(
        !String::from_utf8_lossy(&repository_doctor.stdout).contains(repository.to_str().unwrap())
    );

    let repository_doctor_human = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "doctor",
            "repository",
            "--repo",
            repository.to_str().unwrap(),
            "--human",
        ],
        None,
    );
    assert!(repository_doctor_human.status.success());
    assert!(repository_doctor_human.stderr.is_empty());
    let repository_doctor_report = String::from_utf8(repository_doctor_human.stdout).unwrap();
    assert!(repository_doctor_report.contains("Codeclew repository diagnostic"));
    assert!(repository_doctor_report.contains("PYTHON / python-syntax"));
    assert!(repository_doctor_report.contains("Compilation: python:.#."));
    assert!(!repository_doctor_report.contains(repository.to_str().unwrap()));

    let doctor = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "doctor",
            "task",
            "--repo",
            repository.to_str().unwrap(),
            "--target-ref",
            "main",
            "--language",
            "python",
            "--profile",
            "python-syntax",
            "--compilation",
            "python:.#.",
            "--operation",
            "analysis",
        ],
        None,
    );
    assert!(doctor.status.success());
    let doctor_value: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(doctor_value["schema"], "codeclew-doctor/2.0");
    assert_eq!(doctor_value["scope"], "TASK");
    assert_eq!(doctor_value["status"], "PASS");
    assert_eq!(doctor_value["nextAction"], "NONE");
    assert!(
        doctor_value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| {
                !matches!(
                    check["checkId"].as_str().unwrap(),
                    "tool.java" | "tool.rustc" | "tool.cargo" | "state.free-space"
                )
            })
    );
    assert!(
        doctor_value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| {
                check["checkId"] == "repository.target-ref-at-head" && check["status"] == "PASS"
            })
    );
    let doctor_human = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "doctor",
            "task",
            "--repo",
            repository.to_str().unwrap(),
            "--target-ref",
            "main",
            "--language",
            "python",
            "--profile",
            "python-syntax",
            "--compilation",
            "python:.#.",
            "--operation",
            "analysis",
            "--human",
        ],
        None,
    );
    assert!(doctor_human.status.success());
    assert!(doctor_human.stderr.is_empty());
    let doctor_report = String::from_utf8(doctor_human.stdout).unwrap();
    assert!(doctor_report.contains("Codeclew doctor"));
    assert!(doctor_report.contains("Status: READY"));
    assert!(doctor_report.contains("Scope: TASK"));
    assert!(doctor_report.contains("Target ref points to HEAD"));
    assert!(!doctor_report.contains(repository.to_str().unwrap()));

    let opened = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "session",
            "open",
            "--repo",
            repository.to_str().unwrap(),
            "--target-ref",
            "main",
            "--language",
            "kotlin",
            "--compilation",
            ":/main",
        ],
        None,
    );
    assert!(opened.status.success());
    let opened_value: Value = serde_json::from_slice(&opened.stdout).unwrap();
    let session_id = opened_value["session"]["sessionId"].as_str().unwrap();

    let source = state_root
        .join("sessions")
        .join(session_id.strip_prefix("session:").unwrap())
        .join("source");
    assert!(source.join("README.md").is_file());
    assert!(!source.join("docs/plans/local.md").exists());
    assert_eq!(
        fs::read(repository.join("docs/plans/local.md")).unwrap(),
        b"local plan\n"
    );

    let freshness = |session_id: &str| {
        run_managed(
            &runtime_binary,
            &state_root,
            &runtime,
            &lease,
            &["change", "check-freshness", "--session", session_id],
            None,
        )
    };
    let fresh = freshness(session_id);
    assert!(fresh.status.success());
    let fresh_value: Value = serde_json::from_slice(&fresh.stdout).unwrap();
    assert_eq!(fresh_value["status"], "FRESH");
    assert_eq!(fresh_value["remediationId"], "NONE");

    fs::write(repository.join("notes.md"), b"notes after session open\n").unwrap();
    let with_notes = freshness(session_id);
    assert!(with_notes.status.success());
    let with_notes_value: Value = serde_json::from_slice(&with_notes.stdout).unwrap();
    assert_eq!(with_notes_value["status"], "FRESH");
    assert_eq!(with_notes_value["targetWorktreeClean"], true);

    fs::write(repository.join("README.md"), b"dirty\n").unwrap();
    let dirty = freshness(session_id);
    let dirty_value: Value = serde_json::from_slice(&dirty.stdout).unwrap();
    assert_eq!(dirty_value["status"], "DIRTY");
    assert_eq!(dirty_value["remediationId"], "CLEAN_TARGET_WORKTREE");

    run_git(&repository, &["add", "."]);
    run_git(
        &repository,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=codeclew@localhost",
            "commit",
            "-q",
            "-m",
            "external update",
        ],
    );
    let stale = freshness(session_id);
    let stale_value: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(stale_value["status"], "STALE");
    assert_eq!(stale_value["remediationId"], "OPEN_NEW_SESSION");

    for output in [
        &capabilities.stdout,
        &attach.stdout,
        &doctor.stdout,
        &fresh.stdout,
        &dirty.stdout,
        &stale.stdout,
    ] {
        assert_bytes_hide_paths(output, &[&repository, &state_root, &runtime]);
    }
}

#[cfg(unix)]
#[test]
fn managed_support_summary_requires_private_input_and_drops_private_material() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let state_root = temporary.path().join("state/v2");
    let runtime_digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&runtime_digest);
    fs::create_dir_all(state_root.join("locks")).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{runtime_digest}.lease"));
    let diagnostic = temporary.path().join("private-diagnostic.json");
    fs::write(
        &diagnostic,
        br#"{"schema":"codeclew-error/2.0","error":{"code":"WORKER_CRASHED","message":"/private/repository/src/Secret.kt failed","transactionId":"run:private","retryable":true}}"#,
    )
    .unwrap();
    fs::set_permissions(&diagnostic, fs::Permissions::from_mode(0o600)).unwrap();

    let summarized = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "support",
            "summarize",
            "--input",
            diagnostic.to_str().unwrap(),
        ],
        None,
    );
    assert!(summarized.status.success());
    let value: Value = serde_json::from_slice(&summarized.stdout).unwrap();
    assert_eq!(value["schema"], "codeclew-support-summary/1.0");
    assert_eq!(value["status"], "SAFE_TO_SHARE");
    assert_eq!(value["errorCode"], "WORKER_CRASHED");
    let stdout = String::from_utf8(summarized.stdout).unwrap();
    for forbidden in ["/private", "Secret.kt", "run:private"] {
        assert!(!stdout.contains(forbidden));
    }

    fs::write(&diagnostic, serde_json::to_vec(&json!({
        "schema":"codeclew-documentation-check/1.0","services":{},
        "unresolved":{"private-service":{"reason":"WORKER_CRASHED","nextAction":"/private/service failed"}},
        "freshness":{"status":"UNRESOLVED"}
    })).unwrap()).unwrap();
    let summarized_docs = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "support",
            "summarize",
            "--input",
            diagnostic.to_str().unwrap(),
        ],
        None,
    );
    assert!(summarized_docs.status.success());
    let docs_value: Value = serde_json::from_slice(&summarized_docs.stdout).unwrap();
    assert_eq!(docs_value["sourceStage"], "DOCUMENTATION");
    assert_eq!(
        docs_value["documentation"]["failures"][0]["errorCode"],
        "WORKER_CRASHED"
    );
    assert!(
        !String::from_utf8(summarized_docs.stdout)
            .unwrap()
            .contains("private")
    );

    fs::set_permissions(&diagnostic, fs::Permissions::from_mode(0o644)).unwrap();
    let rejected = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "support",
            "summarize",
            "--input",
            diagnostic.to_str().unwrap(),
        ],
        None,
    );
    assert!(!rejected.status.success());
    let rejected_value: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected_value["error"]["code"], "INVALID_INPUT");
    assert_bytes_hide_paths(&rejected.stdout, &[&diagnostic]);
}

#[cfg(unix)]
#[test]
fn working_tree_context_retains_saved_rust_files_and_cleans_its_own_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let repo = temporary.path().join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        b"[package]\nname=\"snapshot_fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(
        repo.join("Cargo.lock"),
        b"version = 4\n\n[[package]]\nname = \"snapshot_fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(repo.join("src/lib.rs"), b"pub fn before() -> i32 { 1 }\n").unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["add", "."]);
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=test@codeclew.invalid",
            "commit",
            "-qm",
            "base",
        ],
    );
    fs::write(repo.join("src/lib.rs"), b"pub fn staged() -> i32 { 2 }\n").unwrap();
    run_git(&repo, &["add", "."]);
    let saved = b"mod new;\npub fn saved() -> i32 { 3 }\n";
    fs::write(repo.join("src/lib.rs"), saved).unwrap();
    fs::write(
        repo.join("src/new.rs"),
        b"pub fn untracked() -> i32 { 4 }\n",
    )
    .unwrap();
    let index = fs::read(repo.join(".git/index")).unwrap();
    let state_root = temporary.path().join("state/v2");
    fs::create_dir_all(state_root.join("locks")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&digest);
    let binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{digest}.lease"));
    let run = |args: &[&str]| {
        let output = run_managed(&binary, &state_root, &runtime, &lease, args, None);
        let value: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        assert!(output.status.success(), "{value}");
        value
    };
    let opened = run(&[
        "context",
        "open",
        "--repo",
        repo.to_str().unwrap(),
        "--target-ref",
        "main",
        "--language",
        "rust",
        "--profile",
        "rust-syntax",
        "--compilation",
        "cargo:Cargo.toml#snapshot_fixture#lib#snapshot_fixture",
        "--operation",
        "analysis",
        "--working-tree",
        "--intent",
        "Inspect saved edits",
        "--term",
        "saved",
        "--term",
        "untracked",
    ]);
    assert_eq!(
        opened["admission"]["taskAuthority"]["sourceSelection"]["kind"],
        "WORKING_TREE"
    );
    assert_eq!(opened["session"]["schema"], "codeclew-session/6.0");
    assert!(opened["context"].to_string().contains("saved"));
    assert!(opened["context"].to_string().contains("untracked"));
    let session = opened["session"]["sessionId"].as_str().unwrap();
    let source = state_root
        .join("sessions")
        .join(session.strip_prefix("session:").unwrap())
        .join("source");
    assert_eq!(fs::read(source.join("src/lib.rs")).unwrap(), saved);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    let fresh = run(&["change", "check-freshness", "--session", session]);
    assert!(fresh.to_string().contains("FRESH"), "{fresh}");
    fs::write(repo.join("src/lib.rs"), b"pub fn later() -> i32 { 5 }\n").unwrap();
    let fresh = run(&["change", "check-freshness", "--session", session]);
    assert!(fresh.to_string().contains("LIVE_CHANGED"), "{fresh}");
    let expanded = run(&[
        "context",
        "expand",
        "--session",
        session,
        "--from",
        opened["context"]["contextId"].as_str().unwrap(),
        "--term",
        "saved",
    ]);
    assert!(expanded.to_string().contains("saved"));
    assert_eq!(fs::read(source.join("src/lib.rs")).unwrap(), saved);
    run(&["session", "close", "--session", session]);
    run(&["session", "gc", "--session", session]);
    assert!(!source.exists());
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read(repo.join("src/lib.rs")).unwrap(),
        b"pub fn later() -> i32 { 5 }\n"
    );
}

#[test]
fn working_tree_comparison_retains_exact_diff_after_session_and_storage_gc() {
    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let repo = temporary.path().join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        b"[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(
        repo.join("Cargo.lock"),
        b"version = 4\n\n[[package]]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        repo.join("src/lib.rs"),
        b"pub fn price() -> i32 { 1 }\npub fn stable() -> i32 { 9 }\n",
    )
    .unwrap();
    fs::write(repo.join("old.txt"), b"removed\n").unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["add", "."]);
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@codeclew.invalid",
            "commit",
            "-qm",
            "base",
        ],
    );
    fs::write(
        repo.join("src/lib.rs"),
        b"pub fn price() -> i32 { 2 }\npub fn stable() -> i32 { 9 }\n",
    )
    .unwrap();
    run_git(&repo, &["add", "."]);
    let saved = b"pub fn price() -> i32 { 3 }\npub fn stable() -> i32 { 9 }\n";
    fs::write(repo.join("src/lib.rs"), saved).unwrap();
    fs::remove_file(repo.join("old.txt")).unwrap();
    fs::write(repo.join("new.txt"), b"added\n").unwrap();
    let index = fs::read(repo.join(".git/index")).unwrap();
    let state_root = temporary.path().join("state/v2");
    fs::create_dir_all(state_root.join("locks")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let digest = "1".repeat(64);
    let runtime = state_root.join("runtimes").join(&digest);
    let binary = fd_runtime(&runtime);
    let lease = state_root
        .join("locks")
        .join(format!("runtime-{digest}.lease"));
    let run = |args: &[&str]| {
        let output = run_managed(&binary, &state_root, &runtime, &lease, args, None);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        assert!(output.status.success(), "{value}");
        value
    };
    let inspected = run(&[
        "change",
        "inspect",
        "--repo",
        repo.to_str().unwrap(),
        "--target-ref",
        "main",
        "--language",
        "rust",
        "--profile",
        "rust-syntax",
        "--compilation",
        "cargo:Cargo.toml#fixture#lib#fixture",
        "--working-tree",
    ]);
    assert_eq!(inspected["status"], "BOUNDED_COMPARISON", "{inspected}");
    assert_eq!(inspected["counts"]["changedFiles"], 3);
    assert_eq!(inspected["counts"]["changedDeclarations"], 1);
    assert_eq!(inspected["counts"]["unchangedDeclarations"], 1);
    assert_eq!(
        inspected["declarations"][0]["changes"][0],
        "DECLARATION_SOURCE_TEXT_CHANGED"
    );
    assert_eq!(inspected["testsExecuted"], false);
    assert!(
        inspected["cleanup"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "COLLECTED"),
        "{inspected}"
    );
    let id = inspected["comparisonId"].as_str().unwrap();
    let before_show = run(&["change", "show", "--comparison", id]);
    let fresh = run(&["change", "check-freshness", "--comparison", id]);
    assert_eq!(fresh["liveStatus"], "FRESH");
    assert_eq!(fresh["retainedEvidenceValid"], true);
    let rendered = temporary.path().join("change.html");
    run(&[
        "change",
        "render",
        "--comparison",
        id,
        "--output",
        rendered.to_str().unwrap(),
    ]);
    let initial_html = fs::read(&rendered).unwrap();
    let source = run(&[
        "change",
        "source",
        "--comparison",
        id,
        "--file",
        "src/lib.rs",
        "--side",
        "after",
        "--limit",
        "20",
    ]);
    assert_eq!(source["text"], std::str::from_utf8(&saved[..20]).unwrap());
    assert_eq!(source["nextOffset"], 20);
    fs::write(repo.join("src/lib.rs"), b"pub fn later() {}\n").unwrap();
    run(&["storage", "gc", "--apply"]);
    let retained = run(&["change", "show", "--comparison", id]);
    assert_eq!(retained, before_show);
    let stale = run(&["change", "check-freshness", "--comparison", id]);
    assert_eq!(stale["liveStatus"], "LIVE_CHANGED");
    assert_eq!(stale["retainedEvidenceValid"], true);
    let repeated = temporary.path().join("change-repeated.html");
    run(&[
        "change",
        "render",
        "--comparison",
        id,
        "--output",
        repeated.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(repeated).unwrap(), initial_html);
    let no_tools = temporary.path().join("no-tools");
    fs::create_dir(&no_tools).unwrap();
    let offline_report = temporary.path().join("offline.html");
    let offline = run_managed_exact_path(
        &binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "change",
            "render",
            "--comparison",
            id,
            "--output",
            offline_report.to_str().unwrap(),
        ],
        &no_tools,
    );
    assert!(
        offline.status.success(),
        "{}",
        String::from_utf8_lossy(&offline.stdout)
    );
    assert_eq!(fs::read(offline_report).unwrap(), initial_html);
    let source = run(&[
        "change",
        "source",
        "--comparison",
        id,
        "--file",
        "src/lib.rs",
        "--side",
        "after",
        "--offset",
        "20",
    ]);
    assert_eq!(source["text"], std::str::from_utf8(&saved[20..]).unwrap());
    assert_eq!(source["nextOffset"], Value::Null);
    assert!(
        retained["files"]
            .to_string()
            .contains("pub fn price() -> i32 { 3 }")
    );
    assert!(!retained["files"].to_string().contains("pub fn later"));
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    run(&["change", "forget", "--comparison", id]);
    let missing = run_managed(
        &binary,
        &state_root,
        &runtime,
        &lease,
        &["change", "show", "--comparison", id],
        None,
    );
    assert!(!missing.status.success());
}

#[cfg(unix)]
#[test]
fn durable_documentation_cli_recovers_and_reports_route_fragments() {
    use clew::documentation::{check::Check, model::*};
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/durable-docs");
    let docs = temporary.path().join("architecture");
    copy_tree(&fixture.join("architecture"), &docs);
    fs::create_dir_all(docs.join("docs")).unwrap();
    fs::write(
        docs.join("docs/manual.md"),
        "Engineer-maintained explanation.\n",
    )
    .unwrap();
    let mut repositories = BTreeMap::new();
    for id in ["orders", "inventory"] {
        let repo = temporary.path().join(id);
        copy_tree(&fixture.join(id), &repo);
        run_git(&repo, &["init", "-q", "-b", "main"]);
        run_git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                &format!("https://example.invalid/{id}"),
            ],
        );
        run_git(&repo, &["add", "."]);
        run_git(
            &repo,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        repositories.insert(id, repo);
    }
    let create_runtime = |name: &str| {
        let state = temporary.path().join(name).join("v2");
        let runtime = state.join("runtimes").join("1".repeat(64));
        fs::create_dir_all(state.join("locks")).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let binary = fd_runtime(&runtime);
        let lease = state
            .join("locks")
            .join(format!("runtime-{}.lease", "1".repeat(64)));
        (state, runtime, binary, lease)
    };
    let (state, runtime, binary, lease) = create_runtime("first-home");
    let run = |args: &[&str]| {
        let output = run_managed(&binary, &state, &runtime, &lease, args, None);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.code().unwrap(), value)
    };
    let root = docs.to_str().unwrap();
    let (code, value) = run(&[
        "docs",
        "init",
        "--root",
        root,
        "--title",
        "Checkout architecture",
    ]);
    assert_eq!(code, 0, "{value}");
    for (id, repo) in &repositories {
        let (code, value) = run(&[
            "docs",
            "bind",
            "--root",
            root,
            "--service",
            id,
            "--repo",
            repo.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{value}");
    }
    let (code, value) = run(&["docs", "check", "--root", root]);
    assert_eq!(code, 3, "{value}");
    assert_eq!(value["freshness"]["status"], "UNRESOLVED");
    assert_eq!(
        value["interactions"]["reserve-inventory"]["path"]["receiver"]["status"], "MATCH",
        "{value}"
    );
    let checked: Check =
        serde_json::from_slice(&fs::read(docs.join(".codeclew/cache/latest-check.json")).unwrap())
            .unwrap();
    let (code, context) = run(&[
        "docs",
        "context",
        "--root",
        root,
        "--service",
        "orders",
        "--entrypoint",
        &checked.services["orders"].entrypoints[0].id,
        "--limit",
        "100",
    ]);
    assert_eq!(code, 0, "{context}");
    assert!(
        context["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["kind"] == "SOURCE")
    );
    let declaration = checked.services["orders"]
        .observations
        .values()
        .find(|o| o.kind == "SYMBOL")
        .unwrap();
    let (code, symbol_context) = run(&[
        "docs",
        "context",
        "--root",
        root,
        "--service",
        "orders",
        "--symbol",
        &declaration.symbol,
        "--limit",
        "100",
    ]);
    assert_eq!(code, 0, "{symbol_context}");
    assert!(
        symbol_context["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "DEPENDENCY" && item["id"] == declaration.id)
    );
    assert!(
        !symbol_context["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "ENTRYPOINT")
    );
    let (code, _) = run(&[
        "docs",
        "context",
        "--root",
        root,
        "--service",
        "orders",
        "--symbol",
        "missing.ExactDeclaration",
        "--limit",
        "100",
    ]);
    assert_ne!(code, 0);
    let mut narratives = Vec::new();
    for id in ["orders", "inventory"] {
        let evidence = &checked.services[id];
        let entry = &evidence.entrypoints[0];
        let symbol = evidence
            .observations
            .values()
            .find(|o| o.kind == "SYMBOL" && o.symbol == entry.symbol)
            .unwrap();
        let participant = |id: &str, label: &str, service: Option<&str>| Participant {
            id: id.into(),
            label: label.into(),
            service: service.map(str::to_owned),
        };
        let summary = Fragment {
            id: "summary".into(),
            text: if id == "orders" {
                "Validate an order and request a reservation."
            } else {
                "Accept an available quantity and record its reservation."
            }
            .into(),
            dependency_ids: vec![symbol.id.clone()],
            source_ids: symbol.source_ids.clone(),
        };
        let guard = evidence
            .observations
            .values()
            .find(|o| o.kind == "FLOW" && o.symbol == entry.symbol && o.normalized["kind"] == "IF")
            .unwrap();
        let mut returns: Vec<_> = evidence
            .observations
            .values()
            .filter(|o| {
                o.kind == "FLOW" && o.symbol == entry.symbol && o.normalized["kind"] == "RETURN"
            })
            .collect();
        returns.sort_by_key(|o| evidence.sources[&o.source_ids[0]].start_line);
        let call = evidence
            .observations
            .values()
            .find(|o| {
                o.kind == "FLOW"
                    && o.symbol == entry.symbol
                    && o.normalized["target"].as_str().is_some_and(|t| {
                        t.contains("InventoryClient#reserve") || t.contains("ReservationStore#save")
                    })
            })
            .unwrap();
        let local_event = |id: &str,
                           kind: &str,
                           text: &str,
                           from: Option<&str>,
                           to: Option<&str>,
                           source: &Observation| Event {
            id: id.into(),
            kind: kind.into(),
            text: text.into(),
            from: from.map(str::to_owned),
            to: to.map(str::to_owned),
            dependency_ids: vec![source.id.clone()],
            source_ids: source.source_ids.clone(),
            interaction: None,
        };
        let events = vec![
            local_event(
                "request",
                "message",
                "Submit the request",
                Some("client"),
                Some("handler"),
                symbol,
            ),
            local_event(
                "guard",
                "alt",
                if id == "orders" {
                    "Quantity is invalid"
                } else {
                    "Quantity exceeds available stock"
                },
                None,
                None,
                guard,
            ),
            local_event(
                "rejected",
                "return",
                "Reject the request",
                Some("handler"),
                Some("client"),
                returns[0],
            ),
            local_event(
                "accepted",
                "else",
                "Quantity is acceptable",
                None,
                None,
                guard,
            ),
            local_event(
                "apply",
                "message",
                if id == "orders" {
                    "Request a reservation"
                } else {
                    "Save the reservation"
                },
                Some("handler"),
                Some("handler"),
                call,
            ),
            local_event(
                "result",
                "return",
                "Return the reservation outcome",
                Some("handler"),
                Some("client"),
                returns[1],
            ),
            local_event("end", "end", "", None, None, guard),
        ];
        narratives.push(Narrative{schema:"codeclew-documentation-narrative/1.0".into(),subject:format!("service:{id}"),context_digest:checked.context_digest.clone(),operations:vec![Operation{overview_diagram:None,interface_contracts:vec![],id:entry.id.clone(),title:if id=="orders"{"Check out an order"}else{"Reserve inventory"}.into(),summary,explanation:vec![],participants:vec![participant("client","Client",None),participant("handler","Request handler",Some(id))],events,findings:vec![],boundaries:vec!["The diagram stops at calls made by this controller; the separate checkout scenario connects both services.".into()]}],gaps:BTreeMap::new()});
    }
    let scenario = &checked.scenarios["checkout"];
    let first = scenario
        .steps
        .iter()
        .find(|s| s.kind == "IF" && s.service == "orders")
        .unwrap();
    let receiver = scenario
        .steps
        .iter()
        .find(|s| s.kind == "IF" && s.service == "inventory")
        .unwrap();
    let transition = scenario
        .steps
        .iter()
        .find(|s| s.kind == "DECLARED_HTTP_TRANSITION")
        .unwrap();
    let event = |id: &str,
                 kind: &str,
                 text: &str,
                 from: Option<&str>,
                 to: Option<&str>,
                 step: &clew::documentation::check::FlowStep| Event {
        id: id.into(),
        kind: kind.into(),
        text: text.into(),
        from: from.map(str::to_owned),
        to: to.map(str::to_owned),
        dependency_ids: step.dependency_ids.clone(),
        source_ids: step.source_ids.clone(),
        interaction: if kind == "declared" {
            Some("reserve-inventory".into())
        } else {
            None
        },
    };
    let mut scenario_events = vec![
        event(
            "valid-order",
            "alt",
            "Order quantity is valid",
            None,
            None,
            first,
        ),
        event(
            "reserve",
            "declared",
            "Request an inventory reservation",
            Some("orders"),
            Some("inventory"),
            transition,
        ),
        event(
            "stock-check",
            "alt",
            "Requested stock is available",
            None,
            None,
            receiver,
        ),
        event(
            "save",
            "note",
            "Record the reservation",
            Some("inventory"),
            None,
            receiver,
        ),
        event(
            "no-stock",
            "else",
            "Requested stock is unavailable",
            None,
            None,
            receiver,
        ),
        event(
            "reject-stock",
            "note",
            "Reject the reservation",
            Some("inventory"),
            None,
            receiver,
        ),
        event("end-stock", "end", "", None, None, receiver),
        event(
            "invalid-order",
            "else",
            "Order quantity is invalid",
            None,
            None,
            first,
        ),
        event(
            "reject-order",
            "note",
            "Reject the order",
            Some("orders"),
            None,
            first,
        ),
        event("end-order", "end", "", None, None, first),
    ];
    // Notes are agent-authored interpretations of the selected source, not compiler events.
    scenario_events[3].source_ids = checked.dependencies[&transition.dependency_ids[2]]
        .source_ids
        .clone();
    scenario_events[3].dependency_ids = vec![transition.dependency_ids[2].clone()];
    let returns: Vec<_> = scenario
        .steps
        .iter()
        .filter(|s| s.kind == "RETURN")
        .collect();
    let source_text = |step: &clew::documentation::check::FlowStep| {
        checked.services[&step.service].sources[&step.source_ids[0]]
            .text
            .as_str()
    };
    let rejected_stock = returns
        .iter()
        .find(|s| source_text(s).contains("insufficient stock"))
        .unwrap();
    let rejected_order = returns
        .iter()
        .find(|s| source_text(s).contains("invalid quantity"))
        .unwrap();
    scenario_events[5].dependency_ids = rejected_stock.dependency_ids.clone();
    scenario_events[5].source_ids = rejected_stock.source_ids.clone();
    scenario_events[8].dependency_ids = rejected_order.dependency_ids.clone();
    scenario_events[8].source_ids = rejected_order.source_ids.clone();
    let successful: Vec<_> = returns
        .iter()
        .filter(|s| {
            !source_text(s).contains("insufficient stock")
                && !source_text(s).contains("invalid quantity")
        })
        .collect();
    let mut response_dependencies: Vec<_> = successful
        .iter()
        .flat_map(|s| s.dependency_ids.clone())
        .collect();
    response_dependencies.push("interaction:reserve-inventory".into());
    scenario_events.insert(
        4,
        Event {
            id: "reservation-response".into(),
            kind: "return".into(),
            text: "Return the reservation outcome".into(),
            from: Some("inventory".into()),
            to: Some("orders".into()),
            dependency_ids: response_dependencies,
            source_ids: successful
                .iter()
                .flat_map(|s| s.source_ids.clone())
                .collect(),
            interaction: Some("reserve-inventory".into()),
        },
    );
    narratives.push(Narrative {
        schema: "codeclew-documentation-narrative/1.0".into(),
        subject: "scenario:checkout".into(),
        context_digest: checked.context_digest.clone(),
        operations: vec![Operation {
            dataflow: None,
            assessment: None,
            overview_diagram: None,
            interface_contracts: vec![],
            id: "checkout".into(),
            title: "Checkout and reserve inventory".into(),
            summary: Fragment {
                id: "summary".into(),
                text: "Validate the order and request available inventory.".into(),
                dependency_ids: first.dependency_ids.clone(),
                source_ids: first.source_ids.clone(),
            },
            participants: vec![
                Participant {
                    id: "orders".into(),
                    label: "Orders".into(),
                    service: Some("orders".into()),
                },
                Participant {
                    id: "inventory".into(),
                    label: "Inventory".into(),
                    service: Some("inventory".into()),
                },
            ],
            events: scenario_events,
            explanation: vec![],
            findings: vec![],
            boundaries: scenario.boundaries.clone(),
        }],
        gaps: BTreeMap::new(),
    });
    for narrative in &narratives {
        clew::documentation::render::validate(narrative, &checked).unwrap();
    }
    let mut missing_guard = narratives[0].clone();
    missing_guard.operations[0]
        .events
        .retain(|e| !matches!(e.kind.as_str(), "alt" | "else" | "end"));
    assert!(
        clew::documentation::render::validate(&missing_guard, &checked)
            .unwrap_err()
            .message
            .contains("omits a source-backed condition")
    );
    let mut inputs = Vec::new();
    for (index, n) in narratives.iter().enumerate() {
        let path = temporary.path().join(format!("narrative-{index}.json"));
        fs::write(&path, serde_json::to_vec(n).unwrap()).unwrap();
        inputs.push(path);
    }
    let mut args = vec!["docs", "render", "--root", root, "--require-complete"];
    for path in &inputs {
        args.extend(["--input", path.to_str().unwrap()]);
    }
    let (code, rendered) = run(&args);
    assert_eq!(code, 0, "{rendered}");
    assert_eq!(rendered["explicitGaps"], 0);
    let bundle_root = docs
        .join("docs/generated")
        .join(rendered["bundle"].as_str().unwrap());
    for page in [
        docs.join("docs/index.html"),
        bundle_root.join("overview.html"),
    ] {
        let html = fs::read_to_string(&page).unwrap();
        let links: Vec<_> = html
            .split("href=\"")
            .skip(1)
            .map(|part| part.split('"').next().unwrap())
            .collect();
        assert!(
            links
                .iter()
                .any(|link| link.ends_with("services/orders.html"))
        );
        assert!(
            links
                .iter()
                .any(|link| link.ends_with("scenarios/checkout.html"))
        );
        for link in links {
            if !link.contains("://") && !link.starts_with('#') {
                assert!(
                    page.parent().unwrap().join(link).is_file(),
                    "broken link in {}: {link}",
                    page.display()
                );
            }
        }
    }
    let before = fs::read(docs.join("docs/index.html")).unwrap();
    let (code, current) = run(&["docs", "check", "--root", root]);
    assert_eq!(code, 0, "{current}");
    assert_eq!(current["freshness"]["status"], "CURRENT");
    let (code, again) = run(&["docs", "render", "--root", root]);
    assert_eq!(code, 0, "{again}");
    assert_eq!(again["bundle"], rendered["bundle"]);
    assert_eq!(fs::read(docs.join("docs/index.html")).unwrap(), before);
    assert_eq!(
        fs::read_to_string(docs.join("docs/manual.md")).unwrap(),
        "Engineer-maintained explanation.\n"
    );
    for repo in repositories.values() {
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            status.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&status.stdout)
        );
    }
    // Optional reproducible fixture export for visual review; never export private runtime state.
    if let Some(destination) = std::env::var_os("CODECLEW_DOCS_EXAMPLE_OUTPUT") {
        let destination = Path::new(&destination);
        fs::create_dir(destination).expect("example output must be a new directory");
        copy_tree(&docs.join("docs"), &destination.join("docs"));
        for (index, narrative) in narratives.iter().enumerate() {
            fs::write(
                destination.join(format!("narrative-{index}.json")),
                serde_json::to_vec_pretty(narrative).unwrap(),
            )
            .unwrap();
        }
    }
    // Only inventory changes. The declared card, transport edge and contract row are affected.
    let inventory = &repositories["inventory"];
    let controller = inventory.join("src/main/java/example/inventory/ReservationController.java");
    fs::write(
        &controller,
        fs::read_to_string(&controller)
            .unwrap()
            .replace("/reservations", "/stock-reservations"),
    )
    .unwrap();
    run_git(inventory, &["add", "."]);
    run_git(
        inventory,
        &[
            "-c",
            "user.name=Codeclew Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "change inventory route",
        ],
    );
    let (code, stale) = run(&["docs", "check", "--root", root]);
    assert_eq!(code, 4, "{stale}");
    let affected = stale["freshness"]["affected"].as_array().unwrap();
    assert!(
        affected
            .iter()
            .any(|v| v["fragment"] == "interaction:reserve-inventory/card")
    );
    assert!(
        affected
            .iter()
            .any(|v| v["fragment"] == "scenario:checkout/checkout/reserve")
    );
    assert!(
        affected
            .iter()
            .any(|v| v["fragment"].as_str().unwrap().contains("contract"))
    );
    assert_eq!(
        stale["interactions"]["reserve-inventory"]["path"]["receiver"]["status"],
        "MISMATCH"
    );
    let (code, refused) = run(&["docs", "render", "--root", root, "--require-complete"]);
    assert_eq!(code, 4, "{refused}");
    assert_eq!(fs::read(docs.join("docs/index.html")).unwrap(), before);
    // Restore the old available revision without rewriting the first source checkout.
    let clone_root = temporary.path().join("relocated");
    fs::create_dir_all(&clone_root).unwrap();
    let cloned_docs = clone_root.join("architecture");
    copy_tree(&docs, &cloned_docs);
    fs::remove_dir_all(cloned_docs.join(".codeclew")).unwrap();
    let (state2, runtime2, binary2, lease2) = create_runtime("fresh-home");
    for (id, source) in &repositories {
        let relocated = clone_root.join(id);
        run_git(
            &clone_root,
            &[
                "clone",
                "-q",
                source.to_str().unwrap(),
                relocated.to_str().unwrap(),
            ],
        );
        run_git(
            &relocated,
            &[
                "remote",
                "set-url",
                "origin",
                &format!("https://example.invalid/{id}"),
            ],
        );
        if *id == "inventory" {
            run_git(&relocated, &["checkout", "-q", "-B", "main", "HEAD~1"]);
        }
        let out = run_managed(
            &binary2,
            &state2,
            &runtime2,
            &lease2,
            &[
                "docs",
                "bind",
                "--root",
                cloned_docs.to_str().unwrap(),
                "--service",
                id,
                "--repo",
                relocated.to_str().unwrap(),
            ],
            None,
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    let recovered = run_managed(
        &binary2,
        &state2,
        &runtime2,
        &lease2,
        &["docs", "check", "--root", cloned_docs.to_str().unwrap()],
        None,
    );
    let recovered_value: Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert!(recovered.status.success(), "{recovered_value}");
    assert_eq!(recovered_value["freshness"]["status"], "CURRENT");
    let declaration = run_managed(
        &binary2,
        &state2,
        &runtime2,
        &lease2,
        &[
            "docs",
            "interaction",
            "show",
            "--root",
            cloned_docs.to_str().unwrap(),
            "--id",
            "reserve-inventory",
        ],
        None,
    );
    assert!(declaration.status.success());
    assert!(String::from_utf8_lossy(&declaration.stdout).contains("Fixture engineer declaration"));
    fs::remove_file(cloned_docs.join(".codeclew/bindings/inventory.json")).unwrap();
    let missing = run_managed(
        &binary2,
        &state2,
        &runtime2,
        &lease2,
        &["docs", "check", "--root", cloned_docs.to_str().unwrap()],
        None,
    );
    assert_eq!(missing.status.code(), Some(3));
    let missing: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing["freshness"]["status"], "UNRESOLVED");
}

#[test]
#[cfg(unix)]
fn durable_source_documentation_without_build_tools_rebinds_and_preserves_publication() {
    use clew::documentation::check::Check;
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let state = temporary.path().join("state/v2");
    let runtime = state.join("runtimes").join("1".repeat(64));
    fs::create_dir_all(state.join("locks")).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let binary = fd_runtime(&runtime);
    let lease = state
        .join("locks")
        .join(format!("runtime-{}.lease", "1".repeat(64)));
    let tools = temporary.path().join("tools");
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
    let audit = temporary.path().join("unexpected-tool");
    for tool in [
        "java", "javac", "mvn", "gradle", "kotlinc", "python", "python3", "cargo", "curl", "wget",
        "node",
    ] {
        let path = tools.join(tool);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{tool}' >> '{}'\nexit 99\n",
                audit.display()
            ),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let docs = temporary.path().join("docs");
    let root = docs.to_str().unwrap();
    let run = |args: &[&str]| {
        let out = run_managed_exact_path(&binary, &state, &runtime, &lease, args, &tools);
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
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/durable-docs-source");
    let mut repositories = BTreeMap::new();
    for language in ["python", "java", "kotlin"] {
        let repo = temporary.path().join(language);
        copy_tree(&fixtures.join(language), &repo);
        run_git(&repo, &["init", "-q"]);
        run_git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                &format!("https://example.invalid/{language}"),
            ],
        );
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
        let record = json!({"schema":"codeclew-documentation-service/1.0","id":language,"title":format!("{language} reservations"),"repositoryId":language,"repository":format!("https://example.invalid/{language}"),"language":language,"profile":"source-syntax","targetRef":"HEAD","source":{"roots":["."],"dialect":if language=="kotlin"{"1.9"}else{"fixture"}}});
        let input = temporary.path().join(format!("{language}.json"));
        fs::write(&input, serde_json::to_vec(&record).unwrap()).unwrap();
        let (_, catalogue) = run(&["docs", "service", "list", "--root", root]);
        let (code, value) = run(&[
            "docs",
            "service",
            "add",
            "--root",
            root,
            "--input",
            input.to_str().unwrap(),
            "--expected-input-digest",
            catalogue["inputDigest"].as_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{value}");
        let (code, value) = run(&[
            "docs",
            "bind",
            "--root",
            root,
            "--service",
            language,
            "--repo",
            repo.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{value}");
        repositories.insert(language, repo);
    }
    let (_, value) = run(&["docs", "check", "--root", root]);
    assert_eq!(value["status"], "CHECKED", "{value}");
    for language in ["python", "java", "kotlin"] {
        assert_eq!(value["services"][language]["coverage"], "SYNTAX", "{value}");
        assert!(value["services"][language]["entrypoints"].as_u64().unwrap() > 1);
    }
    let checked: Check =
        serde_json::from_slice(&fs::read(docs.join(".codeclew/cache/latest-check.json")).unwrap())
            .unwrap();
    let mut inputs = Vec::new();
    for (language, evidence) in &checked.services {
        let entry = evidence
            .entrypoints
            .iter()
            .find(|e| e.trigger["name"] == "reserve")
            .unwrap();
        let flows: Vec<_> = evidence
            .observations
            .values()
            .filter(|o| o.kind == "FLOW" && o.symbol == entry.symbol)
            .collect();
        let mut flows = flows;
        flows.sort_by_key(|o| o.normalized["ordinal"].as_u64());
        let mut events = Vec::new();
        for (index, flow) in flows.iter().enumerate() {
            let kind = match flow.normalized["kind"].as_str().unwrap() {
                "IF" | "TRY" => "alt",
                "LOOP" => "loop",
                "DEFERRED" => "opt",
                _ => "note",
            };
            events.push(json!({"id":format!("event-{index}"),"kind":kind,"text":format!("Source contains {}. This view records lexical structure.",flow.normalized["syntaxKind"].as_str().unwrap()),"from":null,"to":null,"dependencyIds":[flow.id],"sourceIds":flow.source_ids}));
            if matches!(kind, "alt" | "loop" | "opt") {
                events.push(json!({"id":format!("end-{index}"),"kind":"end","text":"","from":null,"to":null,"dependencyIds":[flow.id],"sourceIds":flow.source_ids}));
            }
        }
        let narrative = json!({"schema":"codeclew-documentation-narrative/1.0","subject":format!("service:{language}"),"contextDigest":checked.context_digest,"operations":[{"id":entry.id,"title":"Reserve stock","summary":{"id":"summary","text":"The source checks quantity before recording a reservation in memory. It does not establish durable storage.","dependencyIds":entry.dependency_ids,"sourceIds":entry.source_ids},"participants":[{"id":"caller","label":"Caller","service":null},{"id":"service","label":"Reservations","service":language}],"events":events,"boundaries":["Source syntax only; call targets and runtime ordering remain unresolved."]}],"gaps":evidence.entrypoints.iter().filter(|e|e.id!=entry.id).map(|e|(&e.id,"This callable has source evidence but its behavior has not been authored.")).collect::<BTreeMap<_,_>>()});
        let input = temporary.path().join(format!("narrative-{language}.json"));
        fs::write(&input, serde_json::to_vec(&narrative).unwrap()).unwrap();
        inputs.push(input);
        let named = if language == "python" {
            "orders.Reservations.reserve"
        } else {
            "example.Reservations.reserve"
        };
        let (code, raw) = run(&[
            "docs",
            "context",
            "--root",
            root,
            "--service",
            language,
            "--symbol",
            named,
            "--format",
            "raw",
            "--limit",
            "100",
        ]);
        assert_eq!(code, 0, "{raw}");
        let (code, compact) = run(&[
            "docs",
            "context",
            "--root",
            root,
            "--service",
            language,
            "--symbol",
            named,
            "--format",
            "compact",
            "--limit",
            "100",
        ]);
        assert_eq!(code, 0, "{compact}");
        assert!(
            compact["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["kind"] == "COVERAGE")
        );
        assert_eq!(raw["contextDigest"], compact["contextDigest"]);
        let flow_ids = |page: &Value| {
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|i| i["kind"] == "DEPENDENCY" && i["record"]["kind"] == "FLOW")
                .map(|i| i["id"].clone())
                .collect::<Vec<_>>()
        };
        assert!(!flow_ids(&raw).is_empty());
        assert_eq!(flow_ids(&raw), flow_ids(&compact));
        assert!(
            serde_json::to_vec(&compact).unwrap().len() < serde_json::to_vec(&raw).unwrap().len()
        );
    }
    fs::write(docs.join("docs/manual.md"), "Engineer-owned explanation.\n").unwrap();
    let mut args = vec!["docs", "render", "--root", root];
    for path in &inputs {
        args.extend(["--input", path.to_str().unwrap()]);
    }
    let (code, rendered) = run(&args);
    assert_eq!(code, 0, "{rendered}");
    assert_eq!(rendered["documentedOperations"], 3);
    let initial = fs::read(docs.join("docs/index.html")).unwrap();
    assert_eq!(
        run(&["docs", "render", "--root", root]).1["bundle"],
        rendered["bundle"]
    );
    let repo = &repositories["python"];
    let source = repo.join("orders.py");
    let original = fs::read_to_string(&source).unwrap();
    fs::write(&source, format!("\n\n{original}")).unwrap();
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "Move lines",
        ],
    );
    let (code, value) = run(&["docs", "check", "--root", root]);
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["freshness"]["status"], "CURRENT", "{value}");
    assert!(
        !value["freshness"]["linkChanges"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (code, value) = run(&["docs", "render", "--root", root]);
    assert_eq!(code, 0, "{value}");
    assert_ne!(value["bundle"], rendered["bundle"]);
    let relocated = fs::read(docs.join("docs/index.html")).unwrap();
    assert_ne!(initial, relocated);
    fs::write(repo.join("policy.py"), "MAX_QUANTITY = 25\n").unwrap();
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "Change helper",
        ],
    );
    let (_, value) = run(&["docs", "check", "--root", root]);
    assert_eq!(value["freshness"]["status"], "PARTIALLY_STALE", "{value}");
    assert!(
        value["freshness"]["affected"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["subject"] == "service:python")
    );
    let (_, dossier) = run(&["docs", "changes", "--root", root, "--limit", "100"]);
    assert!(
        dossier["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "AFFECTED_CLAIM" && i["oldClaimAvailable"] == true),
        "{dossier}"
    );
    assert!(
        dossier["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "SOURCE_CHANGE" && i["beforeAvailable"] == true),
        "{dossier}"
    );
    assert_ne!(
        run(&["docs", "render", "--root", root, "--require-complete"]).0,
        0
    );
    assert_eq!(fs::read(docs.join("docs/index.html")).unwrap(), relocated);
    assert_eq!(
        fs::read_to_string(docs.join("docs/manual.md")).unwrap(),
        "Engineer-owned explanation.\n"
    );
    assert!(
        !audit.exists(),
        "build, runtime, or network tool was invoked: {:?}",
        fs::read_to_string(&audit)
    );
}

#[test]
#[cfg(unix)]
#[ignore = "runs the admitted Maven/javac provider for source-documentation enrichment"]
fn durable_source_documentation_java_enrichment_recovers_on_the_same_source_roots() {
    use clew::documentation::check::Check;
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let _cleanup = WritableTreeOnDrop(temporary.path().to_path_buf());
    let state = temporary.path().join("state/v2");
    let runtime = state.join("runtimes").join("1".repeat(64));
    fs::create_dir_all(state.join("locks")).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
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
    let mut record = json!({"schema":"codeclew-documentation-service/1.0","id":"java","title":"Java reservations","repositoryId":"java","repository":"https://example.invalid/java","language":"java","profile":"source-syntax","targetRef":"main","source":{"roots":["."],"dialect":"17"}});
    fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
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
        serde_json::from_slice::<Check>(
            &fs::read(docs.join(".codeclew/cache/latest-check.json")).unwrap(),
        )
        .unwrap()
    };
    let first = read();
    let root_id = first.services["java"]
        .entrypoints
        .iter()
        .find(|e| e.trigger["name"] == "reserve")
        .unwrap()
        .id
        .clone();
    fs::create_dir_all(repo.join("src/main/java/example/missing")).unwrap();
    fs::write(
        repo.join("src/main/java/example/missing/ExternalPolicy.java"),
        "package example.missing; public interface ExternalPolicy {}\n",
    )
    .unwrap();
    commit();
    record["source"]["semantic"] =
        json!({"profile":"java-17plus-maven-read-only","compilation":":/main"});
    fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
    let (_, report) = run(&["docs", "check", "--root", root]);
    assert_eq!(report["status"], "CHECKED", "{report}");
    let enriched = read();
    let e = &enriched.services["java"];
    assert!(e.entrypoints.iter().any(|entry| entry.id == root_id));
    assert!(
        e.observations
            .values()
            .any(|o| o.kind == "SEMANTIC_SYMBOL" && o.normalized["fact"]["name"] == "reserve"),
        "{}",
        serde_json::to_string(&e.boundaries).unwrap()
    );
    assert_eq!(run(&["docs", "render", "--root", root]).0, 0);
    let published = fs::read(docs.join("docs/index.html")).unwrap();
    fs::write(repo.join("pom.xml"), "<broken>\n").unwrap();
    commit();
    let (_, report) = run(&["docs", "check", "--root", root]);
    assert_eq!(report["status"], "CHECKED", "{report}");
    assert_ne!(report["freshness"]["status"], "CURRENT");
    let lost = read();
    assert!(
        lost.services["java"]
            .entrypoints
            .iter()
            .any(|entry| entry.id == root_id)
    );
    assert!(
        !lost.services["java"]
            .observations
            .values()
            .any(|o| o.kind == "SEMANTIC_SYMBOL")
    );
    assert!(
        lost.services["java"]
            .boundaries
            .iter()
            .any(|b| b == "SEMANTIC_PROVIDER_UNAVAILABLE_SOURCE_REMAINS_READABLE")
    );
    assert_ne!(
        run(&["docs", "render", "--root", root, "--require-complete"]).0,
        0
    );
    assert_eq!(fs::read(docs.join("docs/index.html")).unwrap(), published);
}
