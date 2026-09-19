//! Durable successful Java subprocess output, independent of generation projection.
//!
//! The executor policy must change whenever invocation flags, environment,
//! working-directory isolation or raw-output semantics change. Rust projection
//! and repository revision are deliberately outside this compiler-input key.
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::java_adapter_v2::{
    JAVA_ANALYZER_SOURCE, MAX_ANALYZER_OUTPUT_BYTES, parse_java_compiler_output,
};
use crate::java_analysis_inputs::{
    JavaPreparedClasspathAuthority, JavaPreparedSourceAuthority, PreparedJavaAnalysisInputs,
};
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;

pub(crate) const INPUT_SCHEMA: &str = "codeclew-java-executor-input/1.0";
pub(crate) const RECEIPT_SCHEMA: &str = "codeclew-java-executor-success/1.0";
const OUTPUT_SCHEMA: &str = "codeclew-java-analyzer-ndjson/1.0";
const CHECKPOINT_SCHEMA: &str = "codeclew-java-analyzer-output-checkpoint/1.0";
// launch --source 17; UTF-8/en/US/UTC; explicit owned JDK/source/class paths;
// env_clear + LANG/LC_ALL/TZ/JAVA_HOME/TMPDIR/PATH; owned working, home and temporary directories;
// empty implicit source/launch classpath; CLOSED_NO_AP emitter mode.
const JAVA_EXECUTOR_POLICY: &str = "JAVA_CLOSED_EXECUTOR_SOURCE17_ENV_V1";
const MAX_AUTHORITY_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENVELOPE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompilerInput {
    schema: String,
    compilation: String,
    executor_policy: String,
    emitter_digest: String,
    input_policy: String,
    os: String,
    arch: String,
    os_build: String,
    jdk: CasObject,
    sources: Vec<JavaPreparedSourceAuthority>,
    classpath: Vec<JavaPreparedClasspathAuthority>,
    compiler_options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SuccessReceipt {
    schema: String,
    execution_id: String,
    input: CasObject,
    output: CasObject,
    exit_code: i32,
    java_analyzer_starts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Checkpoint {
    schema: String,
    input: CasObject,
    output: CasObject,
    success_receipt: CasObject,
}

pub(crate) struct CompilerOutput {
    pub bytes: Vec<u8>,
    pub success_receipt: CasObject,
    pub hit: bool,
}

pub(crate) fn prepare_input(
    store: &CasStore,
    prepared: &PreparedJavaAnalysisInputs,
    compilation: &str,
) -> Result<CasObject, ClewError> {
    prepared.require_sealed()?;
    let authority = &prepared.authority;
    let jdk = store.put(
        "codeclew-java-execution-image/1.0",
        &canonical::bytes(&authority.jdk).map_err(internal)?,
    )?;
    let input = CompilerInput {
        schema: INPUT_SCHEMA.into(),
        compilation: compilation.into(),
        executor_policy: JAVA_EXECUTOR_POLICY.into(),
        emitter_digest: canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
        input_policy: authority.policy.clone(),
        os: authority.os.clone(),
        arch: authority.arch.clone(),
        os_build: authority.os_build.clone(),
        jdk,
        sources: authority.sources.clone(),
        classpath: authority.classpath.clone(),
        compiler_options: authority.compiler_options.clone(),
    };
    store.put(INPUT_SCHEMA, &canonical::bytes(&input).map_err(internal)?)
}

pub(crate) fn get_or_execute(
    state: &StateAuthority,
    store: &CasStore,
    repository_root: &Path,
    input: &CasObject,
    execute: impl FnOnce() -> Result<Vec<u8>, ClewError>,
) -> Result<CompilerOutput, ClewError> {
    let component = digest_component(&input.digest)?;
    let directory = repository_root.join("generations/java-analyzer-output");
    state.directory_at(&directory)?;
    let path = directory.join(format!("{component}.json"));
    // Store retains a shared CAS-world lease through atomic root publication.
    // Independent generation keys must serialize on the compiler input itself.
    let _lock = InputLock::acquire(state, component)?;
    if state.private_file_exists(&path)? {
        let bytes = state.read_private_file(&path, MAX_ENVELOPE_BYTES)?;
        let checkpoint: Checkpoint = serde_json::from_slice(&bytes)
            .map_err(|_| corrupt("Java raw checkpoint is invalid"))?;
        if canonical::bytes(&checkpoint).map_err(internal)? != bytes
            || checkpoint.schema != CHECKPOINT_SCHEMA
            || checkpoint.input != *input
        {
            return Err(corrupt("Java raw checkpoint input is inconsistent"));
        }
        let output = verify_receipt(store, &checkpoint.success_receipt, input)?;
        let receipt: SuccessReceipt =
            read_canonical(store, &checkpoint.success_receipt, MAX_ENVELOPE_BYTES)?;
        if checkpoint.output != receipt.output {
            return Err(corrupt("Java raw checkpoint output is inconsistent"));
        }
        return Ok(CompilerOutput {
            bytes: output,
            success_receipt: checkpoint.success_receipt,
            hit: true,
        });
    }
    let bytes = execute()?;
    validate_output(store, input, &bytes)?;
    let output = store.put(OUTPUT_SCHEMA, &bytes)?;
    let receipt = SuccessReceipt {
        schema: RECEIPT_SCHEMA.into(),
        execution_id: uuid::Uuid::new_v4().to_string(),
        input: input.clone(),
        output: output.clone(),
        exit_code: 0,
        java_analyzer_starts: 1,
    };
    let success_receipt = store.put(
        RECEIPT_SCHEMA,
        &canonical::bytes(&receipt).map_err(internal)?,
    )?;
    let checkpoint = Checkpoint {
        schema: CHECKPOINT_SCHEMA.into(),
        input: input.clone(),
        output,
        success_receipt: success_receipt.clone(),
    };
    state.write_private_atomic(&path, &canonical::bytes(&checkpoint).map_err(internal)?)?;
    Ok(CompilerOutput {
        bytes,
        success_receipt,
        hit: false,
    })
}

/// Link the reusable computation to the current prepared source/classpath/image
/// closure. The projector adapter digest is intentionally not part of this key.
pub(crate) fn verify_input_binding(
    store: &CasStore,
    input: &CasObject,
    closed: &CasObject,
    compilation: &str,
) -> Result<(), ClewError> {
    if input.object_schema != INPUT_SCHEMA
        || closed.object_schema != "codeclew-java-closed-analysis-authority/1.0"
    {
        return Err(corrupt("Java compiler input binding schema is invalid"));
    }
    let authority: CompilerInput = read_canonical(store, input, MAX_AUTHORITY_BYTES)?;
    let closed: serde_json::Value = read_canonical(store, closed, MAX_AUTHORITY_BYTES)?;
    if authority.schema != INPUT_SCHEMA
        || authority.compilation != compilation
        || closed.get("schema").and_then(serde_json::Value::as_str)
            != Some("codeclew-java-closed-analysis-authority/1.0")
        || closed.get("executionImage")
            != Some(&serde_json::to_value(&authority.jdk).map_err(internal)?)
    {
        return Err(corrupt(
            "Java compiler input is bound to another execution closure",
        ));
    }
    let mut expected = serde_json::to_value(&authority).map_err(internal)?;
    let expected = expected
        .as_object_mut()
        .ok_or_else(|| internal("Java compiler input is not an object"))?;
    for key in [
        "schema",
        "compilation",
        "executorPolicy",
        "emitterDigest",
        "jdk",
    ] {
        expected.remove(key);
    }
    let policy = expected
        .remove("inputPolicy")
        .ok_or_else(|| internal("Java compiler input omits policy"))?;
    expected.insert("policy".into(), policy);
    let mut actual = closed
        .get("policy")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .ok_or_else(|| corrupt("Java closed input policy is invalid"))?;
    actual.remove("adapterDigest");
    if *expected != actual {
        return Err(corrupt(
            "Java compiler input differs from current sealed input authority",
        ));
    }
    Ok(())
}

/// Verify original execution against the *current* derived compiler-input ref.
/// The execution receipt is historical; caller records current hit/start count.
pub(crate) fn verify_receipt(
    store: &CasStore,
    reference: &CasObject,
    expected_input: &CasObject,
) -> Result<Vec<u8>, ClewError> {
    if reference.object_schema != RECEIPT_SCHEMA || expected_input.object_schema != INPUT_SCHEMA {
        return Err(corrupt("Java compiler-output receipt schema is invalid"));
    }
    let receipt: SuccessReceipt = read_canonical(store, reference, MAX_ENVELOPE_BYTES)?;
    if receipt.schema != RECEIPT_SCHEMA
        || receipt.input != *expected_input
        || receipt.output.object_schema != OUTPUT_SCHEMA
        || receipt.exit_code != 0
        || receipt.java_analyzer_starts != 1
        || uuid::Uuid::parse_str(&receipt.execution_id).is_err()
    {
        return Err(corrupt(
            "Java compiler-output execution receipt is inconsistent",
        ));
    }
    let output = store
        .read(&receipt.output, MAX_ANALYZER_OUTPUT_BYTES)?
        .bytes()
        .to_vec();
    validate_output(store, expected_input, &output)?;
    Ok(output)
}

fn validate_output(store: &CasStore, input: &CasObject, bytes: &[u8]) -> Result<(), ClewError> {
    if input.object_schema != INPUT_SCHEMA {
        return Err(corrupt("Java compiler input schema is invalid"));
    }
    let authority: CompilerInput = read_canonical(store, input, MAX_AUTHORITY_BYTES)?;
    if authority.schema != INPUT_SCHEMA {
        return Err(corrupt("Java compiler input authority is invalid"));
    }
    let paths = authority
        .sources
        .iter()
        .map(|source| source.path.as_str())
        .collect::<BTreeSet<_>>();
    for fact in parse_java_compiler_output(bytes)? {
        if fact.path().is_some_and(|path| !paths.contains(path)) {
            return Err(corrupt(
                "Java raw compiler fact is outside admitted source membership",
            ));
        }
    }
    Ok(())
}

fn read_canonical<T: for<'de> Deserialize<'de> + Serialize>(
    store: &CasStore,
    object: &CasObject,
    max: usize,
) -> Result<T, ClewError> {
    let bytes = store.read(object, max)?;
    let value: T = serde_json::from_slice(bytes.bytes())
        .map_err(|_| corrupt("Java compiler checkpoint CAS object is invalid"))?;
    if canonical::bytes(&value).map_err(internal)? != bytes.bytes() {
        return Err(corrupt(
            "Java compiler checkpoint CAS object is not canonical",
        ));
    }
    Ok(value)
}

struct InputLock {
    _file: File,
}
impl InputLock {
    fn acquire(state: &StateAuthority, component: &str) -> Result<Self, ClewError> {
        let name = format!("java-analyzer-input-{component}.lock");
        let file = state
            .directory(Path::new("locks"))?
            .open_lock(OsStr::new(&name))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(internal(std::io::Error::last_os_error()));
        }
        Ok(Self { _file: file })
    }
}
fn digest_component(digest: &str) -> Result<&str, ClewError> {
    let component = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| corrupt("Java compiler input digest prefix is invalid"))?;
    if component.len() != 64
        || !component
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(corrupt("Java compiler input digest is invalid"));
    }
    Ok(component)
}
fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}
fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    fn input(store: &CasStore) -> CompilerInput {
        CompilerInput {
            schema: INPUT_SCHEMA.into(),
            compilation: ":/main".into(),
            executor_policy: JAVA_EXECUTOR_POLICY.into(),
            emitter_digest: canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
            input_policy: "JAVA_CLOSED_NO_AP_JDK17_MACOS_V1".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            os_build: "test-build".into(),
            jdk: store
                .put("codeclew-java-execution-image/1.0", b"{}")
                .unwrap(),
            sources: vec![JavaPreparedSourceAuthority {
                path: "src/Main.java".into(),
                digest: canonical::hash_bytes(b"class Main {}"),
            }],
            classpath: Vec::new(),
            compiler_options: vec!["--release=17".into(), "-proc:none".into()],
        }
    }
    fn put_input(store: &CasStore, input: &CompilerInput) -> CasObject {
        store
            .put(INPUT_SCHEMA, &canonical::bytes(input).unwrap())
            .unwrap()
    }
    fn raw() -> Vec<u8> {
        br#"{"schema":"codeclew-java-compiler-fact/1.0","kind":"BOUNDARY","code":"TEST_BOUNDARY","file":"src/Main.java","requiredChecks":[],"resolution":"TEST"}"#.to_vec()
    }
    fn setup() -> (
        tempfile::TempDir,
        StateAuthority,
        CasStore,
        std::path::PathBuf,
        CasObject,
    ) {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let repository = state
            .directory(Path::new("repos/test"))
            .unwrap()
            .path()
            .to_owned();
        let reference = put_input(&store, &input(&store));
        (root, state, store, repository, reference)
    }

    #[test]
    fn saved_raw_survives_reopen_and_gc_without_any_ready_generation() {
        let (_root, state, store, repository, input) = setup();
        let produced = get_or_execute(&state, &store, &repository, &input, || Ok(raw())).unwrap();
        assert!(!produced.hit);
        // Simulate interruption before projection/DAG/Ready: only the raw root exists.
        drop(store);
        crate::cas::garbage_collect_storage(&state).unwrap();
        let reopened = CasStore::open(&state).unwrap();
        let reused = get_or_execute(&state, &reopened, &repository, &input, || {
            panic!("saved analysis must not execute again")
        })
        .unwrap();
        assert!(reused.hit);
        assert_eq!(reused.bytes, raw());
        assert_eq!(reused.success_receipt, produced.success_receipt);
    }

    #[test]
    fn process_exit_after_raw_publication_recovers_without_ready_or_reexecution() {
        const CHILD_ROOT: &str = "CODECLEW_TEST_JAVA_RAW_CRASH_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let state = StateAuthority::open(std::path::PathBuf::from(root)).unwrap();
            let store = CasStore::open(&state).unwrap();
            let repository = state
                .directory(Path::new("repos/test"))
                .unwrap()
                .path()
                .to_owned();
            let reference = put_input(&store, &input(&store));
            let output =
                get_or_execute(&state, &store, &repository, &reference, || Ok(raw())).unwrap();
            assert!(!output.hit);
            // Bypass Rust destructors: the OS must release the CAS/input leases.
            // No projector, DAG, Ready object or session binding has run.
            std::process::exit(37);
        }
        let root = tempfile::tempdir().unwrap();
        let state_root = root.path().join("v2");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "java_analyzer_output::tests::process_exit_after_raw_publication_recovers_without_ready_or_reexecution", "--nocapture"])
            .env(CHILD_ROOT, &state_root).output().unwrap();
        assert_eq!(
            status.status.code(),
            Some(37),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let state = StateAuthority::open(state_root).unwrap();
        crate::cas::garbage_collect_storage(&state).unwrap();
        let store = CasStore::open(&state).unwrap();
        let repository = state
            .directory(Path::new("repos/test"))
            .unwrap()
            .path()
            .to_owned();
        let reference = put_input(&store, &input(&store));
        let output = get_or_execute(&state, &store, &repository, &reference, || {
            panic!("persisted successful output must survive process exit")
        })
        .unwrap();
        assert!(output.hit);
        assert_eq!(output.bytes, raw());
    }

    #[test]
    fn concurrent_different_callers_execute_one_compiler_input_once() {
        let (_root, state, store, repository, input) = setup();
        let starts = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let state = state.clone();
                let store = store.clone();
                let repository = repository.clone();
                let input = input.clone();
                let starts = Arc::clone(&starts);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    get_or_execute(&state, &store, &repository, &input, || {
                        starts.fetch_add(1, Ordering::SeqCst);
                        Ok(raw())
                    })
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let outputs = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(outputs.iter().filter(|output| output.hit).count(), 1);
        assert_eq!(outputs[0].success_receipt, outputs[1].success_receipt);
    }

    #[test]
    fn input_changes_miss_and_failed_or_invalid_outputs_are_not_saved() {
        let (_root, state, store, repository, original) = setup();
        get_or_execute(&state, &store, &repository, &original, || Ok(raw())).unwrap();
        let baseline = input(&store);
        for field in 0..9 {
            let mut changed = baseline.clone();
            match field {
                0 => changed.sources[0].digest = canonical::hash_bytes(b"changed source"),
                1 => changed.classpath.push(JavaPreparedClasspathAuthority {
                    authority: crate::java_project_model::JavaClasspathAuthority {
                        logical_name: "dependency.jar".into(),
                        digest: canonical::hash_bytes(b"changed classpath"),
                        size: 17,
                        kind: "JAR".into(),
                    },
                }),
                2 => {
                    changed.jdk = store
                        .put("codeclew-java-execution-image/1.0", b"{\"changed\":true}")
                        .unwrap()
                }
                3 => changed.compiler_options.push("--release=21".into()),
                4 => changed.executor_policy.push_str("-changed"),
                5 => changed.emitter_digest = canonical::hash_bytes(b"changed emitter"),
                6 => changed.os_build.push_str("-changed"),
                7 => changed.compilation = ":other/main".into(),
                _ => changed.input_policy.push_str("-changed"),
            }
            let reference = put_input(&store, &changed);
            assert_ne!(reference, original);
            assert!(
                !get_or_execute(&state, &store, &repository, &reference, || Ok(raw()))
                    .unwrap()
                    .hit
            );
        }
        let mut changed = baseline;
        changed.sources[0].digest = canonical::hash_bytes(b"unsaved");
        let reference = put_input(&store, &changed);
        assert!(
            get_or_execute(&state, &store, &repository, &reference, || Err(corrupt(
                "executor failed"
            )))
            .is_err()
        );
        assert!(
            get_or_execute(&state, &store, &repository, &reference, || Ok(
                b"not ndjson".to_vec()
            ))
            .is_err()
        );
        assert!(
            !get_or_execute(&state, &store, &repository, &reference, || Ok(raw()))
                .unwrap()
                .hit
        );
    }

    #[test]
    fn source_path_membership_and_classpath_order_are_compiler_inputs() {
        let (_root, state, store, repository, original) = setup();
        get_or_execute(&state, &store, &repository, &original, || Ok(raw())).unwrap();
        let baseline = input(&store);
        let mut renamed = baseline.clone();
        renamed.sources[0].path = "src/Renamed.java".into();
        let renamed_input = put_input(&store, &renamed);
        assert_ne!(original, renamed_input);
        assert!(
            !get_or_execute(&state, &store, &repository, &renamed_input, || Ok(
                String::from_utf8(raw())
                    .unwrap()
                    .replace("src/Main.java", "src/Renamed.java")
                    .into_bytes()
            ))
            .unwrap()
            .hit
        );
        let mut membership = baseline.clone();
        membership.sources.push(JavaPreparedSourceAuthority {
            path: "src/Other.java".into(),
            digest: canonical::hash_bytes(b"class Other {}"),
        });
        let membership_input = put_input(&store, &membership);
        assert_ne!(original, membership_input);
        assert!(
            !get_or_execute(&state, &store, &repository, &membership_input, || Ok(raw()))
                .unwrap()
                .hit
        );
        let mut ordered = baseline;
        for name in ["first.jar", "second.jar"] {
            ordered.classpath.push(JavaPreparedClasspathAuthority {
                authority: crate::java_project_model::JavaClasspathAuthority {
                    logical_name: name.into(),
                    digest: canonical::hash_bytes(name.as_bytes()),
                    size: name.len() as u64,
                    kind: "JAR".into(),
                },
            });
        }
        let ordered_input = put_input(&store, &ordered);
        assert!(
            !get_or_execute(&state, &store, &repository, &ordered_input, || Ok(raw()))
                .unwrap()
                .hit
        );
        ordered.classpath.reverse();
        let reversed_input = put_input(&store, &ordered);
        assert_ne!(ordered_input, reversed_input);
        assert!(
            !get_or_execute(&state, &store, &repository, &reversed_input, || Ok(raw()))
                .unwrap()
                .hit
        );
    }

    #[test]
    fn corrupt_raw_receipt_is_explicit_error_without_reexecution() {
        let (_root, state, store, repository, input) = setup();
        get_or_execute(&state, &store, &repository, &input, || Ok(raw())).unwrap();
        let path = repository
            .join("generations/java-analyzer-output")
            .join(format!("{}.json", digest_component(&input.digest).unwrap()));
        let bytes = state.read_private_file(&path, MAX_ENVELOPE_BYTES).unwrap();
        let mut checkpoint: Checkpoint = serde_json::from_slice(&bytes).unwrap();
        let invalid = store.put(OUTPUT_SCHEMA, b"not valid NDJSON").unwrap();
        let mut receipt: SuccessReceipt =
            read_canonical(&store, &checkpoint.success_receipt, MAX_ENVELOPE_BYTES).unwrap();
        receipt.output = invalid.clone();
        checkpoint.output = invalid;
        checkpoint.success_receipt = store
            .put(RECEIPT_SCHEMA, &canonical::bytes(&receipt).unwrap())
            .unwrap();
        state
            .write_private_atomic(&path, &canonical::bytes(&checkpoint).unwrap())
            .unwrap();
        let error = get_or_execute(&state, &store, &repository, &input, || {
            panic!("corruption must not trigger analysis")
        })
        .err()
        .unwrap();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    #[test]
    fn original_execution_cannot_be_rebound_to_different_current_inputs() {
        let (_root, _state, store, _repository, reference) = setup();
        let original = input(&store);
        let mut policy = serde_json::to_value(&original).unwrap();
        let fields = policy.as_object_mut().unwrap();
        for key in [
            "schema",
            "compilation",
            "executorPolicy",
            "emitterDigest",
            "jdk",
        ] {
            fields.remove(key);
        }
        let value = fields.remove("inputPolicy").unwrap();
        fields.insert("policy".into(), value);
        fields.insert(
            "adapterDigest".into(),
            serde_json::json!(canonical::hash_bytes(b"current projector")),
        );
        let make_closed = |policy: &serde_json::Value| {
            store.put("codeclew-java-closed-analysis-authority/1.0", &canonical::bytes(&serde_json::json!({
            "schema":"codeclew-java-closed-analysis-authority/1.0", "policy":policy, "executionImage":original.jdk,
        })).unwrap()).unwrap()
        };
        verify_input_binding(&store, &reference, &make_closed(&policy), ":/main").unwrap();
        policy["adapterDigest"] = serde_json::json!(canonical::hash_bytes(b"another projector"));
        verify_input_binding(&store, &reference, &make_closed(&policy), ":/main").unwrap();
        assert!(
            verify_input_binding(&store, &reference, &make_closed(&policy), ":other/main").is_err()
        );
        policy["sources"][0]["digest"] =
            serde_json::json!(canonical::hash_bytes(b"different current source"));
        assert!(verify_input_binding(&store, &reference, &make_closed(&policy), ":/main").is_err());
    }

    #[test]
    fn raw_membership_and_closed_schema_reject_injected_projection() {
        let (_root, _state, store, _repository, input) = setup();
        let external = String::from_utf8(raw())
            .unwrap()
            .replace("src/Main.java", "src/Unadmitted.java");
        assert!(validate_output(&store, &input, external.as_bytes()).is_err());
        let schema = String::from_utf8(raw())
            .unwrap()
            .replace("codeclew-java-compiler-fact/1.0", "unknown/1");
        assert!(validate_output(&store, &input, schema.as_bytes()).is_err());
        let projected = br#"{"schema":"codeclew-java-compiler-fact/1.0","kind":"SOURCE_FILE","file":"src/Main.java","sourceContentDigest":"forged","resolution":"SOURCE_MEMBERSHIP_EXACT"}"#;
        assert!(validate_output(&store, &input, projected).is_err());
        assert!(parse_java_compiler_output(&[0xff]).is_err());
    }
}
