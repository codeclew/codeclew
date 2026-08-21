use crate::adapter_v2::{
    AnalysisAttemptComplete, BuildModel, COMPILATION_SCHEMA, CapabilityUri, CompilationDescriptor,
    DescriptorCompleteness, DescriptorOrigin, LanguageUri, PROVIDER_PROTOCOL, ProviderHandshake,
    ProviderModel, SourceRootDescriptor,
};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::derived_manifest::DerivedAnalysisInputManifest;
use crate::error::{ClewError, ErrorCode};
use crate::generation_v2::{
    AttemptAuthority, FactRunWriter, GENERATION_SCHEMA, GenerationManifest, finalize_generation,
};
use crate::kotlin_adapter_v2::{
    KOTLIN_FACTS_CAPABILITY, KOTLIN_LANGUAGE, analyze_project_native_index, completeness_receipt,
    semantic_scope_digest, translate_facts,
};
use crate::query_v2::{
    QUERY_INDEX_SCHEMA, QueryIndexManifest, build_query_index, verify_index, verify_index_manifest,
};
use crate::repository_snapshot::{RepositoryInputSnapshot, SNAPSHOT_SCHEMA, capture};
use crate::runtime::RuntimeAuthority;
use crate::session::{ModelCachePolicy, SessionAuthority};
use crate::state::{StateAuthority, create_private_directory};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::path::Path;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub const READY_GENERATION_SCHEMA: &str = "codeclew-ready-generation/2.0";
const MAX_BINDING_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadyGeneration {
    pub schema: String,
    pub generation_key: String,
    pub runtime_key: String,
    pub base_revision: String,
    pub compilation: String,
    pub compiler_version: String,
    pub coverage: String,
    pub certainty: String,
    pub obligations: Vec<String>,
    pub repository_snapshot: CasObject,
    pub derived_input_manifest: CasObject,
    pub generation: CasObject,
    pub query_index: CasObject,
}

pub fn ensure_session_generation(session: &SessionAuthority) -> Result<ReadyGeneration, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let session_root = state.session_root(&session.session_id)?;
    let binding_path = session_root.join("generation.json");
    if binding_path.exists() {
        return load_ready(&store, &binding_path, session, false);
    }
    let repo = session.repository_path()?;
    let (snapshot, snapshot_object) = capture(&repo, &store)?;
    let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "generation service must run through ./clew",
        )
    })?;
    let generation_key = canonical::hash(&json!({
        "schema":"codeclew-generation-key/2.0",
        "runtimeKey":runtime.runtime_key,
        "snapshot":snapshot_object,
        "compilation":session.compilation,
    }))
    .map_err(internal)?;
    let _lock = GenerationLock::acquire(&state, &generation_key)?;
    if binding_path.exists() {
        return load_ready(&store, &binding_path, session, false);
    }
    let repository = state.repository(&repo)?;
    let cache_root = repository.root.join("generations");
    create_private_directory(&cache_root)?;
    let cache_path = cache_root.join(format!("{}.json", digest_component(&generation_key)?));
    let ready =
        if session.model_cache_policy != ModelCachePolicy::NonCacheable && cache_path.exists() {
            load_ready(&store, &cache_path, session, false)?
        } else {
            let ready = build_ready(
                &state,
                &store,
                session,
                &runtime,
                snapshot,
                snapshot_object,
                generation_key,
            )?;
            if session.model_cache_policy != ModelCachePolicy::NonCacheable {
                write_private_atomic(&state, &cache_path, &ready)?;
            }
            ready
        };
    write_private_atomic(&state, &binding_path, &ready)?;
    Ok(ready)
}

pub fn load_session_generation(session: &SessionAuthority) -> Result<ReadyGeneration, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let path = state
        .session_root(&session.session_id)?
        .join("generation.json");
    load_ready(&store, &path, session, false)
}

pub fn load_query_index(
    store: &CasStore,
    ready: &ReadyGeneration,
) -> Result<QueryIndexManifest, ClewError> {
    let limit = usize::try_from(ready.query_index.size)
        .map_err(|_| resource("query index exceeds host size"))?;
    let lease = store.read(&ready.query_index, limit)?;
    let index: QueryIndexManifest = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("query index binding is invalid"))?;
    if canonical::bytes(&index).map_err(internal)? != lease.bytes() {
        return Err(corrupt("query index binding is not canonical"));
    }
    Ok(index)
}

pub fn load_snapshot(
    store: &CasStore,
    ready: &ReadyGeneration,
) -> Result<RepositoryInputSnapshot, ClewError> {
    let limit = usize::try_from(ready.repository_snapshot.size)
        .map_err(|_| resource("repository snapshot exceeds host size"))?;
    let lease = store.read(&ready.repository_snapshot, limit)?;
    let snapshot: RepositoryInputSnapshot = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("repository snapshot binding is invalid"))?;
    snapshot.verify()?;
    Ok(snapshot)
}

fn build_ready(
    state: &StateAuthority,
    store: &CasStore,
    session: &SessionAuthority,
    runtime: &RuntimeAuthority,
    snapshot: RepositoryInputSnapshot,
    snapshot_object: CasObject,
    generation_key: String,
) -> Result<ReadyGeneration, ClewError> {
    let compiler_store_key = canonical::hash(&json!({
        "schema":"codeclew-project-native-compiler-store/2.0",
        "runtimeKey":runtime.runtime_key,
        "compilation":session.compilation,
    }))
    .map_err(internal)?;
    let index = analyze_project_native_index(
        state,
        store,
        &snapshot,
        &session.compilation,
        digest_component(&compiler_store_key)?,
    )?;
    let compiler_version = index
        .get("compilerVersion")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| corrupt("Kotlin generation has no compiler identity"))?
        .to_owned();
    let worker_name = match compiler_version
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["2", "1"] => "kotlin21",
        ["2", "3"] => "kotlin23",
        ["2", "4"] => "kotlin24",
        _ => {
            return Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "project Kotlin compiler line is unsupported",
            ));
        }
    };
    let worker = runtime.worker(worker_name)?;
    if worker.compiler_version != compiler_version {
        return Err(corrupt("runtime worker compiler identity changed"));
    }
    let toolchain = store.put(
        "codeclew-kotlin-toolchain-authority/2.0",
        &canonical::bytes(worker).map_err(internal)?,
    )?;
    let options = store.put(
        "codeclew-project-native-options/2.0",
        &canonical::bytes(&json!({"nativeCompilation":session.compilation})).map_err(internal)?,
    )?;
    let model = store.put(
        "codeclew-project-native-model/2.0",
        &canonical::bytes(&json!({
            "schema":"codeclew-project-native-model/2.0",
            "snapshotId":snapshot.snapshot_id,
            "compilation":session.compilation,
            "compilerVersion":compiler_version,
        }))
        .map_err(internal)?,
    )?;
    let descriptor = CompilationDescriptor {
        schema: COMPILATION_SCHEMA.into(),
        compilation_id: safe_compilation_id(&session.compilation),
        language_uri: LanguageUri::parse(KOTLIN_LANGUAGE)?,
        source_roots: vec![SourceRootDescriptor {
            logical_name: "project".into(),
            tree: snapshot_object.clone(),
        }],
        generated_source_roots: Vec::new(),
        classpath: Vec::new(),
        toolchain,
        plugins: Vec::new(),
        canonical_options: options,
        dependency_compilation_ids: Vec::new(),
        operations: Vec::new(),
        origin: DescriptorOrigin::ProjectNative,
        completeness: DescriptorCompleteness::Complete,
    };
    let provider = ProviderModel {
        handshake: ProviderHandshake {
            protocol: PROVIDER_PROTOCOL.into(),
            provider_id: "project-native-kotlin".into(),
            provider_digest: runtime.runtime_key.clone(),
            build_system_uris: vec!["build:project-native".into()],
        },
        build_model: BuildModel {
            provider_id: "project-native-kotlin".into(),
            model,
            compilations: vec![descriptor],
        },
    };
    let (_, derived_object) =
        DerivedAnalysisInputManifest::create(store, snapshot_object.clone(), vec![provider])?;
    let facts = translate_facts(store, &index)?;
    let scope_digest = semantic_scope_digest(&index)?;
    let receipt = completeness_receipt(store, &index, &scope_digest)?;
    let completion = AnalysisAttemptComplete {
        scope_digest,
        completeness_receipt: receipt,
        fact_count: facts.len() as u64,
    };
    let mut writer = FactRunWriter::create(state)?;
    for fact in &facts {
        writer.push(fact)?;
    }
    let (generation, generation_object) = finalize_generation(
        store,
        derived_object.clone(),
        vec![AttemptAuthority {
            compilation_id: safe_compilation_id(&session.compilation),
            capability: CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?,
            completion,
        }],
        vec![writer.finish()?],
    )?;
    let (_, query_index_object) = build_query_index(store, &generation, generation_object.clone())?;
    let ready = ReadyGeneration {
        schema: READY_GENERATION_SCHEMA.into(),
        generation_key,
        runtime_key: runtime.runtime_key.clone(),
        base_revision: session.base_revision.clone(),
        compilation: session.compilation.clone(),
        compiler_version,
        coverage: index
            .get("analysisCoverage")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("COMPLETE")
            .to_owned(),
        certainty: index
            .get("analysisCertainty")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("VERIFIED")
            .to_owned(),
        obligations: if index
            .get("analysisCertainty")
            .and_then(serde_json::Value::as_str)
            == Some("UNSURE")
        {
            vec!["restore-k2-semantic-analysis".into()]
        } else {
            Vec::new()
        },
        repository_snapshot: snapshot_object,
        derived_input_manifest: derived_object,
        generation: generation_object,
        query_index: query_index_object,
    };
    verify_ready(store, &ready, session, true)?;
    Ok(ready)
}

fn load_ready(
    store: &CasStore,
    path: &Path,
    session: &SessionAuthority,
    deep: bool,
) -> Result<ReadyGeneration, ClewError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_BINDING_BYTES as u64
    {
        return Err(corrupt("ready generation binding is unsafe"));
    }
    let bytes = fs::read(path).map_err(io_error)?;
    let ready: ReadyGeneration =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("ready generation is invalid"))?;
    if canonical::bytes(&ready).map_err(internal)? != bytes {
        return Err(corrupt("ready generation binding is not canonical"));
    }
    verify_ready(store, &ready, session, deep)?;
    Ok(ready)
}

fn verify_ready(
    store: &CasStore,
    ready: &ReadyGeneration,
    session: &SessionAuthority,
    deep: bool,
) -> Result<(), ClewError> {
    if ready.schema != READY_GENERATION_SCHEMA
        || ready.runtime_key != session.runtime_key
        || ready.base_revision != session.base_revision
        || ready.compilation != session.compilation
        || ready.repository_snapshot.object_schema != SNAPSHOT_SCHEMA
        || ready.generation.object_schema != GENERATION_SCHEMA
        || ready.query_index.object_schema != QUERY_INDEX_SCHEMA
        || !matches!(ready.coverage.as_str(), "COMPLETE" | "PARTIAL")
        || !matches!(ready.certainty.as_str(), "VERIFIED" | "UNSURE")
        || (ready.certainty == "UNSURE" && ready.obligations.is_empty())
        || (ready.certainty == "VERIFIED" && !ready.obligations.is_empty())
    {
        return Err(corrupt("ready generation authority is invalid"));
    }
    let _ = load_snapshot(store, ready)?;
    let generation_limit = usize::try_from(ready.generation.size)
        .map_err(|_| resource("generation exceeds host size"))?;
    let lease = store.read(&ready.generation, generation_limit)?;
    let generation: GenerationManifest = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("generation binding is invalid"))?;
    if deep {
        generation.verify(store)?;
    } else {
        generation.verify_manifest(store)?;
    }
    let query = load_query_index(store, ready)?;
    if query.generation != ready.generation {
        return Err(corrupt("query index is bound to another generation"));
    }
    if deep {
        verify_index(store, &query)
    } else {
        verify_index_manifest(store, &query)
    }
}

fn write_private_atomic(
    state: &StateAuthority,
    path: &Path,
    ready: &ReadyGeneration,
) -> Result<(), ClewError> {
    state.write_private_atomic(path, &canonical::bytes(ready).map_err(internal)?)
}

fn safe_compilation_id(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() || sanitized.len() > 120 {
        format!(
            "compilation-{}",
            &canonical::hash_bytes(value.as_bytes())[7..23]
        )
    } else {
        sanitized
    }
}

fn digest_component(value: &str) -> Result<&str, ClewError> {
    let component = value
        .strip_prefix("sha256:")
        .ok_or_else(|| corrupt("generation digest prefix is invalid"))?;
    if component.len() != 64
        || !component
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(corrupt("generation digest is invalid"));
    }
    Ok(component)
}

struct GenerationLock {
    _file: File,
}

impl GenerationLock {
    fn acquire(state: &StateAuthority, key: &str) -> Result<Self, ClewError> {
        let path = state
            .locks_root()
            .join(format!("generation-{}.lock", digest_component(key)?));
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path).map_err(io_error)?;
        #[cfg(unix)]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self { _file: file })
    }
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
