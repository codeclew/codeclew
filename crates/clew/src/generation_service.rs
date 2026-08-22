use crate::adapter_v2::{
    ANALYSIS_REQUEST_SCHEMA, AdapterRegistry, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, BuildModel, COMPILATION_SCHEMA, CapabilityUri, CompilationDescriptor,
    DescriptorCompleteness, DescriptorOrigin, FactRecord, LanguageUri, PROVIDER_PROTOCOL,
    ProviderHandshake, ProviderModel, SourceRootDescriptor,
};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::cold_start::{
    AttemptJournal, AttemptState, DAG_SCHEMA, DagPlan, DagScheduler, HostResources,
    PersistentProgress, ResourceDescriptor, StageSpec,
};
use crate::derived_manifest::DerivedAnalysisInputManifest;
use crate::error::{ClewError, ErrorCode};
use crate::generation_v2::{
    AttemptAuthority, FactRun, FactRunWriter, GENERATION_SCHEMA, GenerationManifest,
    finalize_generation,
};
use crate::incremental_v2::{
    COMPLETENESS_VECTOR_SCHEMA, Certainty, CompletenessVector, Coverage, Support,
    VerificationObligation,
};
use crate::kotlin_adapter_v2::{
    KOTLIN_FACTS_CAPABILITY, KOTLIN_LANGUAGE, KotlinAdapterV2, KotlinCompilerLine,
    KotlinGenerationDriver, analyze_project_native_index, semantic_scope_digest,
};
use crate::query_v2::{
    QUERY_INDEX_SCHEMA, QueryIndexManifest, build_query_index, verify_index, verify_index_manifest,
};
use crate::repository_snapshot::{RepositoryInputSnapshot, SNAPSHOT_SCHEMA, capture};
use crate::runtime::RuntimeAuthority;
use crate::session::{ModelCachePolicy, SessionAuthority};
use crate::state::{StateAuthority, create_private_directory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub const READY_GENERATION_SCHEMA: &str = "codeclew-ready-generation/2.0";
const PREPARED_AUTHORITY_SCHEMA: &str = "codeclew-prepared-generation-authority/2.0";
const MODEL_ANALYSIS_SCHEMA: &str = "codeclew-project-native-analysis/2.0";
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
    pub completeness: CompletenessVector,
    pub coverage: String,
    pub certainty: String,
    pub obligations: Vec<String>,
    pub repository_snapshot: CasObject,
    pub derived_input_manifest: CasObject,
    pub generation: CasObject,
    pub query_index: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedGenerationAuthority {
    schema: String,
    runtime_key: String,
    repository_snapshot: CasObject,
    compilation: String,
    compiler_version: String,
    adapter_digest: String,
    analysis: CasObject,
    descriptor: CompilationDescriptor,
    derived_input_manifest: CasObject,
    completeness: CompletenessVector,
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
    let repository = state.repository(&repo)?;
    let prepared = ensure_prepared_authority(
        &state,
        &store,
        &repository.root,
        session,
        &runtime,
        &snapshot,
        &snapshot_object,
    )?;
    let generation_key = final_generation_key(
        &runtime.runtime_key,
        &session.base_revision,
        &snapshot_object,
        &session.compilation,
        &prepared.derived_input_manifest,
        &prepared.completeness,
    )?;
    let _lock = GenerationLock::acquire(&state, &generation_key)?;
    if binding_path.exists() {
        return load_ready(&store, &binding_path, session, false);
    }
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
                snapshot_object,
                generation_key,
                prepared,
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
    snapshot_object: CasObject,
    generation_key: String,
    prepared: PreparedGenerationAuthority,
) -> Result<ReadyGeneration, ClewError> {
    let index = load_analysis(store, &prepared.analysis)?;
    let line = compiler_line(&prepared.compiler_version)?.1;
    let driver = PreparedKotlinDriver {
        index: Mutex::new(Some(index)),
    };
    let adapter = KotlinAdapterV2::new(
        line,
        prepared.adapter_digest.clone(),
        prepared.descriptor.toolchain.digest.clone(),
        store.clone(),
        driver,
    )?;
    let mut registry = AdapterRegistry::default();
    registry.register_adapter(Arc::new(adapter))?;
    let mut journal = AttemptJournal::create(state.clone(), &generation_key, 0)?;
    journal.transition(AttemptState::Snapshotted, snapshot_object.digest.clone())?;
    journal.transition(
        AttemptState::Modeled,
        prepared.derived_input_manifest.digest.clone(),
    )?;
    journal.transition(AttemptState::Analyzing, "registered adapter DAG started")?;
    let request = AnalyzeGenerationRequest {
        schema: ANALYSIS_REQUEST_SCHEMA.into(),
        attempt_id: journal.attempt().attempt_id.clone(),
        generation_key: generation_key.clone(),
        capability: CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?,
        compilation: prepared.descriptor.clone(),
        derived_input_manifest: prepared.derived_input_manifest.clone(),
        parent_generation: None,
    };
    let analysis = match HostResources::detect()
        .and_then(|resources| execute_analysis_dag(state, Arc::new(registry), request, resources))
    {
        Ok(analysis) => analysis,
        Err(error) => {
            journal.transition(AttemptState::Failed, "adapter DAG failed")?;
            return Err(error);
        }
    };
    journal.transition(AttemptState::Finalizing, "deterministic merge started")?;
    let result = (|| {
        let (generation, generation_object) = finalize_generation(
            store,
            prepared.derived_input_manifest.clone(),
            vec![AttemptAuthority {
                compilation_id: safe_compilation_id(&session.compilation),
                capability: CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?,
                completion: analysis.completion,
            }],
            analysis.runs,
        )?;
        let (_, query_index_object) =
            build_query_index(store, &generation, generation_object.clone())?;
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key,
            runtime_key: runtime.runtime_key.clone(),
            base_revision: session.base_revision.clone(),
            compilation: session.compilation.clone(),
            compiler_version: prepared.compiler_version,
            completeness: prepared.completeness.clone(),
            coverage: coverage_label(&prepared.completeness).into(),
            certainty: certainty_label(&prepared.completeness).into(),
            obligations: obligation_codes(&prepared.completeness),
            repository_snapshot: snapshot_object,
            derived_input_manifest: prepared.derived_input_manifest,
            generation: generation_object,
            query_index: query_index_object,
        };
        verify_ready(store, &ready, session, true)?;
        Ok(ready)
    })();
    match result {
        Ok(ready) => {
            journal.transition(AttemptState::Ready, ready.generation.digest.clone())?;
            Ok(ready)
        }
        Err(error) => {
            journal.transition(AttemptState::Failed, "generation finalization failed")?;
            Err(error)
        }
    }
}

fn ensure_prepared_authority(
    state: &StateAuthority,
    store: &CasStore,
    repository_root: &Path,
    session: &SessionAuthority,
    runtime: &RuntimeAuthority,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: &CasObject,
) -> Result<PreparedGenerationAuthority, ClewError> {
    let model_key = canonical::hash(&json!({
        "schema":"codeclew-project-native-model-key/2.0",
        "runtimeKey":runtime.runtime_key,
        "snapshot":snapshot_object,
        "compilation":session.compilation,
    }))
    .map_err(internal)?;
    let _lock = GenerationLock::acquire(state, &model_key)?;
    let root = repository_root.join("generations/models");
    create_private_directory(&root)?;
    let path = root.join(format!("{}.json", digest_component(&model_key)?));
    if session.model_cache_policy != ModelCachePolicy::NonCacheable && path.exists() {
        return load_prepared_authority(
            store,
            &path,
            runtime,
            snapshot_object,
            &session.compilation,
        );
    }
    let prepared = prepare_authority(
        state,
        store,
        runtime,
        snapshot,
        snapshot_object.clone(),
        &session.compilation,
    )?;
    if session.model_cache_policy != ModelCachePolicy::NonCacheable {
        write_canonical_atomic(state, &path, &prepared)?;
    }
    Ok(prepared)
}

fn prepare_authority(
    state: &StateAuthority,
    store: &CasStore,
    runtime: &RuntimeAuthority,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: CasObject,
    compilation: &str,
) -> Result<PreparedGenerationAuthority, ClewError> {
    let compiler_store_key = compiler_store_key(runtime, compilation)?;
    let index = analyze_project_native_index(
        state,
        store,
        snapshot,
        compilation,
        digest_component(&compiler_store_key)?,
    )?;
    let compiler_version = index
        .get("compilerVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt("Kotlin generation has no compiler identity"))?
        .to_owned();
    let (worker_name, _, _) = compiler_line(&compiler_version)?;
    let worker = runtime.worker(worker_name)?;
    if worker.compiler_version != compiler_version {
        return Err(corrupt("runtime worker compiler identity changed"));
    }
    let analysis = store.put(
        MODEL_ANALYSIS_SCHEMA,
        &canonical::bytes(&index).map_err(internal)?,
    )?;
    let scope_digest = semantic_scope_digest(&index)?;
    let completeness = completeness_from_index(&index, &scope_digest)?;
    let toolchain = store.put(
        "codeclew-kotlin-toolchain-authority/2.0",
        &canonical::bytes(worker).map_err(internal)?,
    )?;
    let options = store.put(
        "codeclew-project-native-options/2.0",
        &canonical::bytes(&json!({"nativeCompilation":compilation})).map_err(internal)?,
    )?;
    let model = store.put(
        "codeclew-project-native-model/2.0",
        &canonical::bytes(&json!({
            "schema":"codeclew-project-native-model/2.0",
            "snapshotId":snapshot.snapshot_id,
            "compilation":compilation,
            "compilerVersion":compiler_version,
            "analysis":analysis,
            "completeness":completeness,
        }))
        .map_err(internal)?,
    )?;
    let descriptor = CompilationDescriptor {
        schema: COMPILATION_SCHEMA.into(),
        compilation_id: safe_compilation_id(compilation),
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
        completeness: match completeness.coverage {
            Coverage::Complete { .. } => DescriptorCompleteness::Complete,
            Coverage::Partial { .. } => DescriptorCompleteness::Partial,
            Coverage::Unknown => DescriptorCompleteness::Unknown,
        },
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
            compilations: vec![descriptor.clone()],
        },
    };
    let (_, derived_input_manifest) =
        DerivedAnalysisInputManifest::create(store, snapshot_object.clone(), vec![provider])?;
    let prepared = PreparedGenerationAuthority {
        schema: PREPARED_AUTHORITY_SCHEMA.into(),
        runtime_key: runtime.runtime_key.clone(),
        repository_snapshot: snapshot_object.clone(),
        compilation: compilation.into(),
        compiler_version,
        adapter_digest: worker.tree_hash.clone(),
        analysis,
        descriptor,
        derived_input_manifest,
        completeness,
    };
    Ok(prepared)
}

fn load_prepared_authority(
    store: &CasStore,
    path: &Path,
    runtime: &RuntimeAuthority,
    snapshot: &CasObject,
    compilation: &str,
) -> Result<PreparedGenerationAuthority, ClewError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_BINDING_BYTES as u64
    {
        return Err(corrupt("prepared authority binding is unsafe"));
    }
    let bytes = fs::read(path).map_err(io_error)?;
    let prepared: PreparedGenerationAuthority = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("prepared authority binding is invalid"))?;
    if canonical::bytes(&prepared).map_err(internal)? != bytes {
        return Err(corrupt("prepared authority binding is not canonical"));
    }
    verify_prepared_authority(store, &prepared, runtime, snapshot, compilation)?;
    Ok(prepared)
}

fn verify_prepared_authority(
    store: &CasStore,
    prepared: &PreparedGenerationAuthority,
    runtime: &RuntimeAuthority,
    snapshot: &CasObject,
    compilation: &str,
) -> Result<(), ClewError> {
    prepared.descriptor.validate()?;
    prepared.completeness.validate()?;
    let (worker_name, _, _) = compiler_line(&prepared.compiler_version)?;
    let worker = runtime.worker(worker_name)?;
    if prepared.schema != PREPARED_AUTHORITY_SCHEMA
        || prepared.runtime_key != runtime.runtime_key
        || prepared.repository_snapshot != *snapshot
        || prepared.compilation != compilation
        || prepared.adapter_digest != worker.tree_hash
        || prepared.descriptor.language_uri.as_str() != KOTLIN_LANGUAGE
        || prepared.descriptor.compilation_id != safe_compilation_id(compilation)
        || prepared.descriptor.source_roots.len() != 1
        || prepared.descriptor.source_roots[0].tree != *snapshot
        || prepared.analysis.object_schema != MODEL_ANALYSIS_SCHEMA
    {
        return Err(corrupt("prepared generation authority is inconsistent"));
    }
    let index = load_analysis(store, &prepared.analysis)?;
    if index.get("compilerVersion").and_then(Value::as_str)
        != Some(prepared.compiler_version.as_str())
    {
        return Err(corrupt("prepared analysis compiler identity changed"));
    }
    let scope_digest = semantic_scope_digest(&index)?;
    if completeness_from_index(&index, &scope_digest)? != prepared.completeness {
        return Err(corrupt("prepared completeness authority changed"));
    }
    let limit = usize::try_from(prepared.derived_input_manifest.size)
        .map_err(|_| resource("derived input manifest exceeds host size"))?;
    let lease = store.read(&prepared.derived_input_manifest, limit)?;
    let manifest: DerivedAnalysisInputManifest = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("derived input manifest is invalid"))?;
    if canonical::bytes(&manifest).map_err(internal)? != lease.bytes()
        || manifest.repository_snapshot != *snapshot
        || manifest.provider_models.len() != 1
        || manifest.provider_models[0].build_model.compilations != vec![prepared.descriptor.clone()]
    {
        return Err(corrupt("prepared derived authority is inconsistent"));
    }
    manifest.verify(store)
}

fn load_analysis(store: &CasStore, object: &CasObject) -> Result<Value, ClewError> {
    if object.object_schema != MODEL_ANALYSIS_SCHEMA {
        return Err(corrupt("prepared analysis has the wrong schema"));
    }
    let limit = usize::try_from(object.size)
        .map_err(|_| resource("prepared analysis exceeds host size"))?;
    let lease = store.read(object, limit)?;
    let value: Value = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("prepared analysis is invalid"))?;
    if canonical::bytes(&value).map_err(internal)? != lease.bytes() {
        return Err(corrupt("prepared analysis is not canonical"));
    }
    Ok(value)
}

fn final_generation_key(
    runtime_key: &str,
    base_revision: &str,
    snapshot: &CasObject,
    compilation: &str,
    derived_input_manifest: &CasObject,
    completeness: &CompletenessVector,
) -> Result<String, ClewError> {
    completeness.validate()?;
    canonical::hash(&json!({
        "schema":"codeclew-generation-key/2.1",
        "runtimeKey":runtime_key,
        "baseRevision":base_revision,
        "snapshot":snapshot,
        "compilation":compilation,
        "derivedInputManifest":derived_input_manifest,
        "completeness":completeness,
    }))
    .map_err(internal)
}

struct PreparedKotlinDriver {
    index: Mutex<Option<Value>>,
}

impl KotlinGenerationDriver for PreparedKotlinDriver {
    fn analyze(&self, _request: &AnalyzeGenerationRequest) -> Result<Value, ClewError> {
        self.index
            .lock()
            .map_err(poisoned)?
            .take()
            .ok_or_else(|| corrupt("prepared Kotlin analysis was consumed more than once"))
    }
}

struct CollectedAnalysisSink {
    facts: Vec<FactRecord>,
    completion: Option<AnalysisAttemptComplete>,
}

impl AnalysisSink for CollectedAnalysisSink {
    fn accept(&mut self, event: AnalysisEvent) -> Result<(), ClewError> {
        match event {
            AnalysisEvent::FactShard(shard) => self.facts.extend(shard.facts),
            AnalysisEvent::AttemptComplete(completion) => self.completion = Some(completion),
        }
        Ok(())
    }
}

struct AnalysisDagResult {
    completion: AnalysisAttemptComplete,
    runs: Vec<FactRun>,
}

fn execute_analysis_dag(
    state: &StateAuthority,
    registry: Arc<AdapterRegistry>,
    request: AnalyzeGenerationRequest,
    resources: HostResources,
) -> Result<AnalysisDagResult, ClewError> {
    let jobs = resources.logical_cpu.min(16);
    execute_analysis_dag_with_jobs(state, registry, request, resources, jobs)
}

fn execute_analysis_dag_with_jobs(
    state: &StateAuthority,
    registry: Arc<AdapterRegistry>,
    request: AnalyzeGenerationRequest,
    resources: HostResources,
    jobs: usize,
) -> Result<AnalysisDagResult, ClewError> {
    if jobs == 0 || jobs > resources.logical_cpu {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "analysis job count exceeds the host authority",
        ));
    }
    let analysis = Arc::new(Mutex::new(
        None::<(Arc<Vec<FactRecord>>, AnalysisAttemptComplete)>,
    ));
    let runs = Arc::new(Mutex::new(BTreeMap::<usize, FactRun>::new()));
    let worker_rss = resources
        .codeclew_memory_budget_bytes
        .clamp(1, 2 * 1024 * 1024 * 1024);
    let run_rss = resources
        .codeclew_memory_budget_bytes
        .checked_div(jobs as u64)
        .unwrap_or(0)
        .clamp(1, 128 * 1024 * 1024);
    let mut stages = vec![StageSpec {
        id: "adapter-analysis".into(),
        dependencies: Vec::new(),
        resources: ResourceDescriptor {
            class: "language-adapter".into(),
            min_rss_bytes: 1,
            expected_rss_bytes: worker_rss,
            max_rss_bytes: worker_rss,
            min_cpu: 1,
            max_cpu: 1,
            max_instances: 1,
            exclusivity_key: Some(format!(
                "compiler-store-{}",
                digest_component(&request.compilation.toolchain.digest)?
            )),
        },
        operation_uri: "core:adapter-analysis".into(),
        input: Value::Null,
    }];
    for partition in 0..jobs {
        stages.push(StageSpec {
            id: format!("fact-run-{partition:04}"),
            dependencies: vec!["adapter-analysis".into()],
            resources: ResourceDescriptor {
                class: "fact-run-writer".into(),
                min_rss_bytes: 1,
                expected_rss_bytes: run_rss,
                max_rss_bytes: run_rss,
                min_cpu: 1,
                max_cpu: 1,
                max_instances: jobs,
                exclusivity_key: None,
            },
            operation_uri: "core:fact-run".into(),
            input: json!({"partition":partition,"partitions":jobs}),
        });
    }
    let observer = Arc::new(PersistentProgress::open(state, &request.attempt_id)?);
    let scheduler = DagScheduler::new(resources, observer)?;
    let state_for_executor = state.clone();
    let analysis_for_executor = Arc::clone(&analysis);
    let runs_for_executor = Arc::clone(&runs);
    scheduler.execute(
        DagPlan {
            schema: DAG_SCHEMA.into(),
            stages,
        },
        move |stage, cancelled| match stage.operation_uri.as_str() {
            "core:adapter-analysis" => {
                let mut sink = CollectedAnalysisSink {
                    facts: Vec::new(),
                    completion: None,
                };
                let completion =
                    registry.analyze_generation_into(&request, &mut sink, cancelled)?;
                if sink.completion.as_ref() != Some(&completion)
                    || completion.fact_count != sink.facts.len() as u64
                    || sink.facts.is_empty()
                {
                    return Err(ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "adapter completion differs from the collected fact stream",
                    ));
                }
                *analysis_for_executor.lock().map_err(poisoned)? =
                    Some((Arc::new(sink.facts), completion.clone()));
                Ok(json!({"factCount":completion.fact_count}))
            }
            "core:fact-run" => {
                let partition = stage
                    .input
                    .get("partition")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| corrupt("fact-run partition is invalid"))?;
                let (facts, _) = analysis_for_executor
                    .lock()
                    .map_err(poisoned)?
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| corrupt("fact-run started before adapter completion"))?;
                let start = facts.len().saturating_mul(partition) / jobs;
                let end = facts.len().saturating_mul(partition + 1) / jobs;
                let mut writer = FactRunWriter::create(&state_for_executor)?;
                for fact in &facts[start..end] {
                    writer.push(fact)?;
                }
                runs_for_executor
                    .lock()
                    .map_err(poisoned)?
                    .insert(partition, writer.finish()?);
                Ok(json!({"factCount":end-start,"partition":partition}))
            }
            _ => Err(corrupt("cold-start DAG contains an unknown operation")),
        },
    )?;
    let completion = analysis
        .lock()
        .map_err(poisoned)?
        .as_ref()
        .map(|(_, completion)| completion.clone())
        .ok_or_else(|| corrupt("adapter DAG produced no completion"))?;
    let runs = std::mem::take(&mut *runs.lock().map_err(poisoned)?)
        .into_values()
        .collect::<Vec<_>>();
    if runs.len() != jobs {
        return Err(corrupt("adapter DAG produced an incomplete fact-run set"));
    }
    Ok(AnalysisDagResult { completion, runs })
}

fn completeness_from_index(
    index: &Value,
    scope_digest: &str,
) -> Result<CompletenessVector, ClewError> {
    let descriptor_coverage = index
        .pointer("/declarationDescriptors/coverage")
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt("Kotlin descriptor coverage authority is missing"))?;
    let relation_coverage = index
        .pointer("/declarationRelations/coverage")
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt("Kotlin relation coverage authority is missing"))?;
    for value in [descriptor_coverage, relation_coverage] {
        if !matches!(value, "COMPLETE_SUPPORTED_SUBSET" | "PARTIAL") {
            return Err(corrupt("Kotlin coverage authority is unsupported"));
        }
    }
    let k2_validated = index.get("k2Validated").and_then(Value::as_bool) == Some(true);
    let certainty = match index.get("analysisCertainty").and_then(Value::as_str) {
        Some("UNSURE") => Certainty::Unsure {
            check_set: vec!["restore-k2-semantic-analysis".into()],
        },
        Some("VERIFIED") if k2_validated => Certainty::Verified,
        None if k2_validated => Certainty::Verified,
        Some("VERIFIED") | None => {
            return Err(corrupt(
                "Kotlin semantic certainty has no validated compiler authority",
            ));
        }
        Some(_) => return Err(corrupt("Kotlin semantic certainty is unsupported")),
    };
    let complete = descriptor_coverage == "COMPLETE_SUPPORTED_SUBSET"
        && relation_coverage == "COMPLETE_SUPPORTED_SUBSET"
        && certainty == Certainty::Verified;
    if let Some(declared) = index.get("analysisCoverage").and_then(Value::as_str)
        && !matches!(declared, "COMPLETE" | "PARTIAL")
    {
        return Err(corrupt("Kotlin declared analysis coverage is unsupported"));
    }
    if index.get("analysisCoverage").and_then(Value::as_str) == Some("COMPLETE") && !complete {
        return Err(corrupt(
            "Kotlin declared complete coverage lacks complete domain evidence",
        ));
    }
    let coverage = if complete {
        Coverage::Complete {
            scope_digest: scope_digest.into(),
        }
    } else {
        Coverage::Partial {
            observed_scopes: vec![scope_digest.into()],
            boundaries: vec![if certainty == Certainty::Verified {
                "KOTLIN_PARTIAL_BOUNDARY".into()
            } else {
                "KOTLIN_SEMANTIC_UNSURE".into()
            }],
        }
    };
    let obligations = if complete {
        Vec::new()
    } else if certainty == Certainty::Verified {
        vec![VerificationObligation {
            code: "VERIFY_PARTIAL_KOTLIN_BOUNDARIES".into(),
            subject: vec![scope_digest.into()],
            publication_blocking: true,
        }]
    } else {
        vec![VerificationObligation {
            code: "RESTORE_K2_SEMANTIC_ANALYSIS".into(),
            subject: vec![scope_digest.into()],
            publication_blocking: true,
        }]
    };
    let completeness = CompletenessVector {
        schema: COMPLETENESS_VECTOR_SCHEMA.into(),
        support: Support::Supported,
        coverage,
        certainty,
        obligations,
    };
    completeness.validate()?;
    Ok(completeness)
}

fn coverage_label(completeness: &CompletenessVector) -> &'static str {
    match completeness.coverage {
        Coverage::Complete { .. } => "COMPLETE",
        Coverage::Partial { .. } => "PARTIAL",
        Coverage::Unknown => "UNKNOWN",
    }
}

fn certainty_label(completeness: &CompletenessVector) -> &'static str {
    if completeness.publishable() {
        "VERIFIED"
    } else {
        "UNSURE"
    }
}

fn obligation_codes(completeness: &CompletenessVector) -> Vec<String> {
    completeness
        .obligations
        .iter()
        .map(|obligation| obligation.code.clone())
        .collect()
}

fn compiler_line(
    compiler_version: &str,
) -> Result<(&'static str, KotlinCompilerLine, &'static str), ClewError> {
    match compiler_version
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["2", "1"] => Ok(("kotlin21", KotlinCompilerLine::K21, "kotlin-2.1")),
        ["2", "3"] => Ok(("kotlin23", KotlinCompilerLine::K23, "kotlin-2.3")),
        ["2", "4"] => Ok(("kotlin24", KotlinCompilerLine::K24, "kotlin-2.4")),
        _ => Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "project Kotlin compiler line is unsupported",
        )),
    }
}

fn compiler_store_key(runtime: &RuntimeAuthority, compilation: &str) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-project-native-compiler-store/3.0",
        "workers":&runtime.workers,
        "compilation":compilation,
    }))
    .map_err(internal)
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
        || ready.coverage != coverage_label(&ready.completeness)
        || ready.certainty != certainty_label(&ready.completeness)
        || ready.obligations != obligation_codes(&ready.completeness)
        || ready.generation_key
            != final_generation_key(
                &ready.runtime_key,
                &ready.base_revision,
                &ready.repository_snapshot,
                &ready.compilation,
                &ready.derived_input_manifest,
                &ready.completeness,
            )?
    {
        return Err(corrupt("ready generation authority is invalid"));
    }
    ready.completeness.validate()?;
    let _ = load_snapshot(store, ready)?;
    let derived_limit = usize::try_from(ready.derived_input_manifest.size)
        .map_err(|_| resource("derived input manifest exceeds host size"))?;
    let derived_lease = store.read(&ready.derived_input_manifest, derived_limit)?;
    let derived: DerivedAnalysisInputManifest = serde_json::from_slice(derived_lease.bytes())
        .map_err(|_| corrupt("derived input manifest binding is invalid"))?;
    if canonical::bytes(&derived).map_err(internal)? != derived_lease.bytes()
        || derived.repository_snapshot != ready.repository_snapshot
    {
        return Err(corrupt("ready derived input authority is invalid"));
    }
    derived.verify(store)?;
    let generation_limit = usize::try_from(ready.generation.size)
        .map_err(|_| resource("generation exceeds host size"))?;
    let lease = store.read(&ready.generation, generation_limit)?;
    let generation: GenerationManifest = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("generation binding is invalid"))?;
    if generation.derived_input_manifest != ready.derived_input_manifest {
        return Err(corrupt("generation is bound to another derived authority"));
    }
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
    write_canonical_atomic(state, path, ready)
}

fn write_canonical_atomic<T: Serialize>(
    state: &StateAuthority,
    path: &Path,
    value: &T,
) -> Result<(), ClewError> {
    state.write_private_atomic(path, &canonical::bytes(value).map_err(internal)?)
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

fn poisoned<T>(_error: std::sync::PoisonError<T>) -> ClewError {
    ClewError::new(ErrorCode::Internal, "generation analysis lock poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter_v2::{
        ADAPTER_PROTOCOL, AdapterHandshake, FactShard, LanguageAdapter, QueryGenerationRequest,
        QueryGenerationResult, ToolchainConstraint, ValidateCandidateRequest,
        ValidateCandidateResult,
    };
    use crate::derived_manifest::DERIVED_MANIFEST_SCHEMA;
    use crate::runtime::{RuntimeMode, RuntimeWorker};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn runtime(runtime_key: &str, binary_byte: u8) -> RuntimeAuthority {
        RuntimeAuthority {
            schema: "codeclew-runtime-capsule/2.0".into(),
            runtime_key: format!("sha256:{}", runtime_key.repeat(64)),
            mode: RuntimeMode::Release,
            manifest_digest: format!("sha256:{}", runtime_key.repeat(64)),
            artifacts: BTreeMap::from([(
                "clew".into(),
                crate::runtime::RuntimeArtifact {
                    path: "bin/clew".into(),
                    size: 1,
                    sha256: format!("sha256:{binary_byte:064x}"),
                },
            )]),
            workers: BTreeMap::from([(
                "kotlin24".into(),
                RuntimeWorker {
                    protocol: "semantic-thread.worker.v1".into(),
                    compiler_version: "2.4.10".into(),
                    distribution: "workers/kotlin24".into(),
                    tree_hash: format!("sha256:{}", "a".repeat(64)),
                    files: Vec::new(),
                },
            )]),
            root: PathBuf::new(),
        }
    }

    struct ProductAdapter {
        toolchain_digest: String,
        facts: Vec<FactRecord>,
        receipt: CasObject,
        calls: Arc<AtomicUsize>,
    }

    impl LanguageAdapter for ProductAdapter {
        fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
            Ok(AdapterHandshake {
                protocol: ADAPTER_PROTOCOL.into(),
                adapter_id: "product-test-adapter".into(),
                adapter_digest: format!("sha256:{}", "a".repeat(64)),
                languages: vec![LanguageUri::parse("language:test")?],
                capabilities: vec![CapabilityUri::parse("analysis:test")?],
                toolchains: vec![ToolchainConstraint {
                    authority_digest: self.toolchain_digest.clone(),
                    minimum_version: None,
                    maximum_version_exclusive: None,
                }],
            })
        }

        fn analyze_generation(
            &self,
            _request: &AnalyzeGenerationRequest,
            sink: &mut dyn AnalysisSink,
            _cancelled: &AtomicBool,
        ) -> Result<(), ClewError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: 0,
                facts: self.facts.clone(),
            }))?;
            sink.accept(AnalysisEvent::AttemptComplete(AnalysisAttemptComplete {
                scope_digest: format!("sha256:{}", "b".repeat(64)),
                completeness_receipt: self.receipt.clone(),
                fact_count: self.facts.len() as u64,
            }))
        }

        fn query_generation(
            &self,
            request: &QueryGenerationRequest,
        ) -> Result<QueryGenerationResult, ClewError> {
            Ok(QueryGenerationResult {
                generation: request.generation.clone(),
                facts: Vec::new(),
            })
        }

        fn validate_candidate(
            &self,
            _request: &ValidateCandidateRequest,
        ) -> Result<ValidateCandidateResult, ClewError> {
            Ok(ValidateCandidateResult {
                validated: true,
                evidence: Vec::new(),
            })
        }

        fn cancel(&self, _attempt_id: &str) -> Result<(), ClewError> {
            Ok(())
        }

        fn shutdown(&self) -> Result<(), ClewError> {
            Ok(())
        }
    }

    fn test_descriptor(store: &CasStore) -> CompilationDescriptor {
        CompilationDescriptor {
            schema: COMPILATION_SCHEMA.into(),
            compilation_id: "test-main".into(),
            language_uri: LanguageUri::parse("language:test").unwrap(),
            source_roots: vec![SourceRootDescriptor {
                logical_name: "main".into(),
                tree: store.put("test/tree/1", b"tree").unwrap(),
            }],
            generated_source_roots: Vec::new(),
            classpath: Vec::new(),
            toolchain: store.put("test/toolchain/1", b"toolchain").unwrap(),
            plugins: Vec::new(),
            canonical_options: store.put("test/options/1", b"options").unwrap(),
            dependency_compilation_ids: Vec::new(),
            operations: Vec::new(),
            origin: DescriptorOrigin::ProjectNative,
            completeness: DescriptorCompleteness::Complete,
        }
    }

    #[test]
    fn compiler_store_survives_unrelated_runtime_rebuilds() {
        let first = runtime("a", 1);
        let rebuilt = runtime("b", 2);
        assert_eq!(
            compiler_store_key(&first, ":workers:kotlin/main").unwrap(),
            compiler_store_key(&rebuilt, ":workers:kotlin/main").unwrap(),
        );
    }

    #[test]
    fn compiler_store_changes_with_worker_or_compilation_authority() {
        let first = runtime("a", 1);
        let mut changed_worker = runtime("b", 2);
        changed_worker
            .workers
            .get_mut("kotlin24")
            .unwrap()
            .tree_hash = format!("sha256:{}", "c".repeat(64));
        assert_ne!(
            compiler_store_key(&first, ":workers:kotlin/main").unwrap(),
            compiler_store_key(&changed_worker, ":workers:kotlin/main").unwrap(),
        );
        assert_ne!(
            compiler_store_key(&first, ":workers:kotlin/main").unwrap(),
            compiler_store_key(&first, ":workers:kotlin/test").unwrap(),
        );
    }

    #[test]
    fn production_dag_calls_registered_adapter_and_is_jobs_deterministic() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let descriptor = test_descriptor(&store);
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let facts = (0..32)
            .map(|index| FactRecord {
                fact_key: format!("test:{index:04}"),
                domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
                payload: store
                    .put("test/fact/1", format!("payload-{index}").as_bytes())
                    .unwrap(),
            })
            .collect::<Vec<_>>();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = AdapterRegistry::default();
        registry
            .register_adapter(Arc::new(ProductAdapter {
                toolchain_digest: descriptor.toolchain.digest.clone(),
                facts,
                receipt,
                calls: Arc::clone(&calls),
            }))
            .unwrap();
        let registry = Arc::new(registry);
        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let request = AnalyzeGenerationRequest {
            schema: ANALYSIS_REQUEST_SCHEMA.into(),
            attempt_id: "attempt:production-dag-test".into(),
            generation_key: format!("sha256:{}", "9".repeat(64)),
            capability: CapabilityUri::parse("analysis:test").unwrap(),
            compilation: descriptor,
            derived_input_manifest: derived.clone(),
            parent_generation: None,
        };
        let resources = HostResources::bounded(4, 8 * 1024 * 1024 * 1024);
        let single = execute_analysis_dag_with_jobs(
            &state,
            Arc::clone(&registry),
            request.clone(),
            resources,
            1,
        )
        .unwrap();
        let parallel =
            execute_analysis_dag_with_jobs(&state, registry, request, resources, 4).unwrap();
        assert_eq!(calls.load(Ordering::Acquire), 2);
        let authority = |completion| AttemptAuthority {
            compilation_id: "test-main".into(),
            capability: CapabilityUri::parse("analysis:test").unwrap(),
            completion,
        };
        let (single_generation, single_object) = finalize_generation(
            &store,
            derived.clone(),
            vec![authority(single.completion)],
            single.runs,
        )
        .unwrap();
        let (parallel_generation, parallel_object) = finalize_generation(
            &store,
            derived,
            vec![authority(parallel.completion)],
            parallel.runs,
        )
        .unwrap();
        assert_eq!(single_generation, parallel_generation);
        assert_eq!(single_object, parallel_object);
    }

    #[test]
    fn missing_semantic_authority_never_defaults_to_verified_complete() {
        let scope = format!("sha256:{}", "c".repeat(64));
        let missing_coverage = json!({
            "k2Validated":true,
            "declarationDescriptors":{"coverage":"COMPLETE_SUPPORTED_SUBSET"},
        });
        assert_eq!(
            completeness_from_index(&missing_coverage, &scope)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
        let unvalidated = json!({
            "k2Validated":false,
            "declarationDescriptors":{"coverage":"COMPLETE_SUPPORTED_SUBSET"},
            "declarationRelations":{"coverage":"COMPLETE_SUPPORTED_SUBSET"},
        });
        assert_eq!(
            completeness_from_index(&unvalidated, &scope)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn final_generation_key_binds_derived_authority_and_completeness() {
        let snapshot = CasObject {
            schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
            object_schema: SNAPSHOT_SCHEMA.into(),
            digest: format!("sha256:{}", "1".repeat(64)),
            size: 1,
        };
        let derived = CasObject {
            schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
            object_schema: crate::derived_manifest::DERIVED_MANIFEST_SCHEMA.into(),
            digest: format!("sha256:{}", "2".repeat(64)),
            size: 1,
        };
        let complete =
            CompletenessVector::verified_complete(format!("sha256:{}", "3".repeat(64))).unwrap();
        let first = final_generation_key(
            &format!("sha256:{}", "4".repeat(64)),
            &format!("sha256:{}", "5".repeat(64)),
            &snapshot,
            ":/main",
            &derived,
            &complete,
        )
        .unwrap();
        let mut changed = complete;
        changed.certainty = Certainty::Unsure {
            check_set: vec!["verify".into()],
        };
        changed.obligations = vec![VerificationObligation {
            code: "VERIFY".into(),
            subject: vec!["scope".into()],
            publication_blocking: true,
        }];
        let second = final_generation_key(
            &format!("sha256:{}", "4".repeat(64)),
            &format!("sha256:{}", "5".repeat(64)),
            &snapshot,
            ":/main",
            &derived,
            &changed,
        )
        .unwrap();
        assert_ne!(first, second);
    }
}
