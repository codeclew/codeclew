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
use std::io::{Read, Seek, SeekFrom};
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

fn read_cas_bytes(state_root: &Path, object: &CasObject) -> Vec<u8> {
    let component = object.digest.strip_prefix("sha256:").unwrap();
    let loose = state_root
        .join("objects/sha256")
        .join(&component[..2])
        .join(&component[2..]);
    let bytes = if loose.is_file() {
        fs::read(loose).unwrap()
    } else {
        let packs = state_root.join("objects/packs-v3");
        let mut indexes = fs::read_dir(&packs)
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("json")
            })
            .collect::<Vec<_>>();
        indexes.sort_by_key(|entry| entry.file_name());
        let mut found = None;
        for index in indexes {
            let manifest: Value = serde_json::from_slice(&fs::read(index.path()).unwrap()).unwrap();
            let Some(entries) = manifest.get("objects").and_then(Value::as_array) else {
                continue;
            };
            for entry in entries {
                let candidate: CasObject = serde_json::from_value(entry["object"].clone()).unwrap();
                if candidate != *object {
                    continue;
                }
                let mut pack = File::open(index.path().with_extension("pack")).unwrap();
                pack.seek(SeekFrom::Start(entry["offset"].as_u64().unwrap()))
                    .unwrap();
                let mut bytes = vec![0; usize::try_from(object.size).unwrap()];
                pack.read_exact(&mut bytes).unwrap();
                found = Some(bytes);
                break;
            }
            if found.is_some() {
                break;
            }
        }
        found.unwrap_or_else(|| panic!("missing CAS object {}", object.digest))
    };
    assert_eq!(bytes.len() as u64, object.size);
    assert_eq!(
        CasObject::for_bytes(&object.object_schema, &bytes).unwrap(),
        *object
    );
    bytes
}

fn rooted_cas_closure(state_root: &Path, roots: &[Vec<u8>]) -> BTreeMap<String, (String, Vec<u8>)> {
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
        let bytes = read_cas_bytes(state_root, &reference);
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            let mut nested = Vec::new();
            collect_cas_references(&value, &mut nested);
            queue.extend(nested);
        }
        closure.insert(reference.digest, (reference.object_schema, bytes));
    }
    closure
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

    let doctor = run_managed(
        &runtime_binary,
        &state_root,
        &runtime,
        &lease,
        &[
            "doctor",
            "--repo",
            repository.to_str().unwrap(),
            "--target-ref",
            "main",
        ],
        None,
    );
    assert!(doctor.status.success());
    let doctor_value: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(doctor_value["schema"], "codeclew-doctor/1.0");
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
            "--repo",
            repository.to_str().unwrap(),
            "--target-ref",
            "main",
            "--human",
        ],
        None,
    );
    assert!(doctor_human.status.success());
    assert!(doctor_human.stderr.is_empty());
    let doctor_report = String::from_utf8(doctor_human.stdout).unwrap();
    assert!(doctor_report.contains("Codeclew doctor"));
    assert!(doctor_report.contains("Status: ACTION REQUIRED"));
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
