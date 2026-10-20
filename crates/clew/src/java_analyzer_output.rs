//! Durable successful Java subprocess output, independent of generation projection.
//!
//! The executor policy must change whenever invocation flags, environment,
//! working-directory isolation or raw-output semantics change. Rust projection
//! and repository revision are deliberately outside this compiler-input key.
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::java_adapter_v2::{
    JAVA_ANALYZER_SOURCE, JavaCompilerFact, MAX_ANALYZER_OUTPUT_BYTES, parse_java_compiler_reader,
};
use crate::java_analysis_inputs::{
    JavaPreparedClasspathAuthority, JavaPreparedSourceAuthority, PreparedJavaAnalysisInputs,
};
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufReader, Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::path::Path;

pub(crate) const INPUT_SCHEMA: &str = "codeclew-java-executor-input/1.0";
pub(crate) const RECEIPT_SCHEMA: &str = "codeclew-java-executor-success/1.0";
const LEGACY_OUTPUT_SCHEMA: &str = "codeclew-java-analyzer-ndjson/1.0";
const OUTPUT_SCHEMA: &str = "codeclew-java-analyzer-output-chunks/1.0";
const CHUNK_SCHEMA: &str = "codeclew-java-analyzer-output-chunk/1.0";
const CHUNK_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_MANIFEST_BYTES: usize = 256 * 1024;
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
    pub facts: Vec<JavaCompilerFact>,
    pub success_receipt: CasObject,
    pub hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChunkedOutput {
    schema: String,
    total_bytes: u64,
    content_digest: String,
    chunks: Vec<CasObject>,
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
    execute: impl FnOnce() -> Result<File, ClewError>,
) -> Result<CompilerOutput, ClewError> {
    let component = digest_component(&input.digest)?;
    // Keep old successful computations reusable without making an older binary
    // mistake a chunk manifest for raw NDJSON after a downgrade.
    let directory = repository_root.join("generations/java-analyzer-stream-output");
    state.directory_at(&directory)?;
    let path = directory.join(format!("{component}.json"));
    // Store retains a shared CAS-world lease through atomic root publication.
    // Independent generation keys must serialize on the compiler input itself.
    let _lock = InputLock::acquire(state, component)?;
    let legacy_path = repository_root
        .join("generations/java-analyzer-output")
        .join(format!("{component}.json"));
    let saved_path = if state.private_file_exists(&path)? {
        Some(&path)
    } else if state.private_file_exists(&legacy_path)? {
        Some(&legacy_path)
    } else {
        None
    };
    if let Some(saved_path) = saved_path {
        let bytes = state.read_private_file(saved_path, MAX_ENVELOPE_BYTES)?;
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
            facts: output,
            success_receipt: checkpoint.success_receipt,
            hit: true,
        });
    }
    let mut raw = execute()?;
    let facts = validate_output(store, input, &mut raw)?;
    raw.rewind().map_err(internal)?;
    let output = save_chunked_output(store, &mut raw)?;
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
        facts,
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
) -> Result<Vec<JavaCompilerFact>, ClewError> {
    if reference.object_schema != RECEIPT_SCHEMA || expected_input.object_schema != INPUT_SCHEMA {
        return Err(corrupt("Java compiler-output receipt schema is invalid"));
    }
    let receipt: SuccessReceipt = read_canonical(store, reference, MAX_ENVELOPE_BYTES)?;
    if receipt.schema != RECEIPT_SCHEMA
        || receipt.input != *expected_input
        || ![OUTPUT_SCHEMA, LEGACY_OUTPUT_SCHEMA].contains(&receipt.output.object_schema.as_str())
        || receipt.exit_code != 0
        || receipt.java_analyzer_starts != 1
        || uuid::Uuid::parse_str(&receipt.execution_id).is_err()
    {
        return Err(corrupt(
            "Java compiler-output execution receipt is inconsistent",
        ));
    }
    if receipt.output.object_schema == LEGACY_OUTPUT_SCHEMA {
        let output = store.read(&receipt.output, 64 * 1024 * 1024)?;
        return validate_output(store, expected_input, output.bytes());
    }
    let mut output = restore_chunked_output(store, &receipt.output)?;
    validate_output(store, expected_input, &mut output)
}

fn save_chunked_output(store: &CasStore, raw: &mut File) -> Result<CasObject, ClewError> {
    use sha2::{Digest, Sha256};
    let mut manifest = ChunkedOutput {
        schema: OUTPUT_SCHEMA.into(),
        total_bytes: 0,
        content_digest: String::new(),
        chunks: Vec::new(),
    };
    let mut digest = Sha256::new();
    loop {
        let mut bytes = Vec::with_capacity(CHUNK_BYTES);
        raw.take(CHUNK_BYTES as u64)
            .read_to_end(&mut bytes)
            .map_err(internal)?;
        if bytes.is_empty() {
            break;
        }
        manifest.total_bytes += bytes.len() as u64;
        if manifest.total_bytes > MAX_ANALYZER_OUTPUT_BYTES as u64 {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "Java compiler output exceeds its byte limit",
            ));
        }
        digest.update(&bytes);
        manifest.chunks.push(store.put(CHUNK_SCHEMA, &bytes)?);
    }
    manifest.content_digest = format!("sha256:{:x}", digest.finalize());
    store.put(
        OUTPUT_SCHEMA,
        &canonical::bytes(&manifest).map_err(internal)?,
    )
}

fn restore_chunked_output(store: &CasStore, reference: &CasObject) -> Result<File, ClewError> {
    use sha2::{Digest, Sha256};
    let manifest: ChunkedOutput = read_canonical(store, reference, MAX_OUTPUT_MANIFEST_BYTES)?;
    if manifest.schema != OUTPUT_SCHEMA
        || manifest.total_bytes > MAX_ANALYZER_OUTPUT_BYTES as u64
        || manifest.chunks.len() != (manifest.total_bytes as usize).div_ceil(CHUNK_BYTES)
    {
        return Err(corrupt(
            "Java output chunk manifest has invalid size or membership",
        ));
    }
    let mut output = tempfile::tempfile().map_err(internal)?;
    let mut digest = Sha256::new();
    let mut total = 0u64;
    for (index, chunk) in manifest.chunks.iter().enumerate() {
        let expected = (manifest.total_bytes - total).min(CHUNK_BYTES as u64);
        if chunk.object_schema != CHUNK_SCHEMA || chunk.size != expected || expected == 0 {
            return Err(corrupt(
                "Java output chunk has invalid schema, order or size",
            ));
        }
        let bytes = store.read(chunk, CHUNK_BYTES)?;
        output.write_all(bytes.bytes()).map_err(internal)?;
        digest.update(bytes.bytes());
        total += bytes.bytes().len() as u64;
        if index + 1 < manifest.chunks.len() && bytes.bytes().len() != CHUNK_BYTES {
            return Err(corrupt("Java output contains a short intermediate chunk"));
        }
    }
    if total != manifest.total_bytes
        || format!("sha256:{:x}", digest.finalize()) != manifest.content_digest
    {
        return Err(corrupt(
            "Java output chunks differ from the complete stream authority",
        ));
    }
    output.rewind().map_err(internal)?;
    Ok(output)
}

fn validate_output(
    store: &CasStore,
    input: &CasObject,
    reader: impl Read,
) -> Result<Vec<JavaCompilerFact>, ClewError> {
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
    let facts = parse_java_compiler_reader(BufReader::new(reader))?;
    for fact in &facts {
        if let crate::java_adapter_v2::JavaCompilerFact::DependencyTarget { binary_origin, .. } =
            &fact
            && binary_origin.classpath_index >= authority.classpath.len()
        {
            return Err(corrupt(
                "Java raw binary target is outside admitted classpath",
            ));
        }
        if fact.path().is_some_and(|path| !paths.contains(path)) {
            return Err(corrupt(
                "Java raw compiler fact is outside admitted source membership",
            ));
        }
    }
    Ok(facts)
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
    fn spool(bytes: Vec<u8>) -> File {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        file.rewind().unwrap();
        file
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
        let produced =
            get_or_execute(&state, &store, &repository, &input, || Ok(spool(raw()))).unwrap();
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
        assert_eq!(
            reused.facts,
            parse_java_compiler_reader(std::io::Cursor::new(raw())).unwrap()
        );
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
                get_or_execute(&state, &store, &repository, &reference, || Ok(spool(raw())))
                    .unwrap();
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
        assert_eq!(
            output.facts,
            parse_java_compiler_reader(std::io::Cursor::new(raw())).unwrap()
        );
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
                        Ok(spool(raw()))
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
        get_or_execute(&state, &store, &repository, &original, || Ok(spool(raw()))).unwrap();
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
                !get_or_execute(&state, &store, &repository, &reference, || Ok(spool(raw())))
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
            get_or_execute(&state, &store, &repository, &reference, || Ok(spool(
                b"not ndjson".to_vec()
            )))
            .is_err()
        );
        assert!(
            !get_or_execute(&state, &store, &repository, &reference, || Ok(spool(raw())))
                .unwrap()
                .hit
        );
    }

    #[test]
    fn source_path_membership_and_classpath_order_are_compiler_inputs() {
        let (_root, state, store, repository, original) = setup();
        get_or_execute(&state, &store, &repository, &original, || Ok(spool(raw()))).unwrap();
        let baseline = input(&store);
        let mut renamed = baseline.clone();
        renamed.sources[0].path = "src/Renamed.java".into();
        let renamed_input = put_input(&store, &renamed);
        assert_ne!(original, renamed_input);
        assert!(
            !get_or_execute(&state, &store, &repository, &renamed_input, || Ok(spool(
                String::from_utf8(raw())
                    .unwrap()
                    .replace("src/Main.java", "src/Renamed.java")
                    .into_bytes()
            )))
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
            !get_or_execute(&state, &store, &repository, &membership_input, || Ok(
                spool(raw())
            ))
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
            !get_or_execute(&state, &store, &repository, &ordered_input, || Ok(spool(
                raw()
            )))
            .unwrap()
            .hit
        );
        ordered.classpath.reverse();
        let reversed_input = put_input(&store, &ordered);
        assert_ne!(ordered_input, reversed_input);
        assert!(
            !get_or_execute(&state, &store, &repository, &reversed_input, || Ok(spool(
                raw()
            )))
            .unwrap()
            .hit
        );
    }

    #[test]
    fn corrupt_raw_receipt_is_explicit_error_without_reexecution() {
        let (_root, state, store, repository, input) = setup();
        get_or_execute(&state, &store, &repository, &input, || Ok(spool(raw()))).unwrap();
        let path = repository
            .join("generations/java-analyzer-stream-output")
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
        assert!(validate_output(&store, &input, projected.as_slice()).is_err());
        assert!(parse_java_compiler_reader(std::io::Cursor::new([0xff])).is_err());
    }

    #[test]
    fn large_stream_survives_gc_and_warm_reuse_with_all_facts() {
        let (_root, state, store, repository, input) = setup();
        let line = String::from_utf8(raw())
            .unwrap()
            .replace("TEST_BOUNDARY", &"x".repeat(64 * 1024));
        let count = 1100;
        let produced = get_or_execute(&state, &store, &repository, &input, || {
            let mut file = tempfile::tempfile().unwrap();
            for _ in 0..count {
                file.write_all(line.as_bytes()).unwrap();
                file.write_all(b"\r\n").unwrap();
            }
            assert!(file.metadata().unwrap().len() > 64 * 1024 * 1024);
            file.rewind().unwrap();
            Ok(file)
        })
        .unwrap();
        assert_eq!(produced.facts.len(), count);
        let receipt: SuccessReceipt =
            read_canonical(&store, &produced.success_receipt, MAX_ENVELOPE_BYTES).unwrap();
        let manifest: ChunkedOutput =
            read_canonical(&store, &receipt.output, MAX_OUTPUT_MANIFEST_BYTES).unwrap();
        assert!(manifest.chunks.len() > 64);
        assert!(
            manifest
                .chunks
                .iter()
                .all(|chunk| chunk.size <= CHUNK_BYTES as u64)
        );
        drop(produced);
        drop(store);
        crate::cas::garbage_collect_storage(&state).unwrap();
        let store = CasStore::open(&state).unwrap();
        let reused = get_or_execute(&state, &store, &repository, &input, || {
            panic!("warm stream must not execute")
        })
        .unwrap();
        assert!(reused.hit);
        assert_eq!(reused.facts.len(), count);
        assert_eq!(
            canonical::bytes(&reused.facts[0]).unwrap(),
            canonical::bytes(&reused.facts[count - 1]).unwrap()
        );
    }

    #[test]
    fn chunk_membership_order_length_and_digest_fail_closed() {
        let (_root, _state, store, _repository, _input) = setup();
        let mut file = spool(vec![b'x'; CHUNK_BYTES * 2 + 17]);
        file.seek(std::io::SeekFrom::Start(CHUNK_BYTES as u64))
            .unwrap();
        file.write_all(&vec![b'y'; CHUNK_BYTES]).unwrap();
        file.rewind().unwrap();
        let reference = save_chunked_output(&store, &mut file).unwrap();
        let original: ChunkedOutput =
            read_canonical(&store, &reference, MAX_OUTPUT_MANIFEST_BYTES).unwrap();
        for attack in 0..5 {
            let mut manifest = original.clone();
            match attack {
                0 => {
                    manifest.chunks.swap(0, 1);
                }
                1 => {
                    manifest.chunks.pop();
                }
                2 => {
                    manifest.total_bytes -= 1;
                }
                3 => {
                    manifest.content_digest = canonical::hash_bytes(b"forged");
                }
                _ => {
                    manifest.total_bytes = MAX_ANALYZER_OUTPUT_BYTES as u64 + 1;
                }
            }
            let forged = store
                .put(OUTPUT_SCHEMA, &canonical::bytes(&manifest).unwrap())
                .unwrap();
            assert_eq!(
                restore_chunked_output(&store, &forged).unwrap_err().code,
                ErrorCode::StateCorrupt
            );
        }
    }

    #[test]
    fn legacy_success_is_reused_without_reexecution() {
        let (_root, state, store, repository, input) = setup();
        let output = store.put(LEGACY_OUTPUT_SCHEMA, &raw()).unwrap();
        let receipt = SuccessReceipt {
            schema: RECEIPT_SCHEMA.into(),
            execution_id: uuid::Uuid::new_v4().to_string(),
            input: input.clone(),
            output: output.clone(),
            exit_code: 0,
            java_analyzer_starts: 1,
        };
        let success_receipt = store
            .put(RECEIPT_SCHEMA, &canonical::bytes(&receipt).unwrap())
            .unwrap();
        let checkpoint = Checkpoint {
            schema: CHECKPOINT_SCHEMA.into(),
            input: input.clone(),
            output,
            success_receipt,
        };
        let directory = repository.join("generations/java-analyzer-output");
        state.directory_at(&directory).unwrap();
        state
            .write_private_atomic(
                &directory.join(format!("{}.json", digest_component(&input.digest).unwrap())),
                &canonical::bytes(&checkpoint).unwrap(),
            )
            .unwrap();
        let reused = get_or_execute(&state, &store, &repository, &input, || {
            panic!("legacy success must be reusable")
        })
        .unwrap();
        assert!(reused.hit);
        assert_eq!(reused.facts.len(), 1);
    }
}
