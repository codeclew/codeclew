use crate::adapter_v2::{
    ANALYSIS_REQUEST_SCHEMA, AdapterRegistry, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, BuildModel, COMPILATION_SCHEMA, CapabilityUri, CompilationDescriptor,
    DescriptorCompleteness, DescriptorOrigin, FactRecord, LanguageUri, PROVIDER_PROTOCOL,
    ProviderHandshake, ProviderModel, SourceRootDescriptor,
};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::cold_start::{
    AttemptJournal, AttemptState, CompositeProgress, DAG_SCHEMA, DagPlan, DagScheduler,
    HostResources, PersistentProgress, ResourceDescriptor, StageSpec, StderrProgress,
};
use crate::derived_manifest::DerivedAnalysisInputManifest;
use crate::error::{ClewError, ErrorCode};
use crate::generation_v2::{
    AttemptAuthority, FactRun, FactRunWriter, GENERATION_SCHEMA, GenerationManifest,
    finalize_generation,
};
use crate::incremental_v2::{
    BoundaryReceipt, COMPLETENESS_VECTOR_SCHEMA, Certainty, CompilerStoreKey, CompletenessVector,
    Coverage, FileReceipt, FullAnalysisReason, INCREMENTAL_RECEIPT_SCHEMA, IncrementalPlan,
    IncrementalReceipt, Support, VerificationObligation, plan_incremental,
};
use crate::kotlin_adapter_v2::{
    KOTLIN_FACTS_CAPABILITY, KOTLIN_LANGUAGE, KotlinAdapterV2, KotlinCompilerLine,
    KotlinGenerationDriver, ProjectNativeKotlinAttempt, ProjectNativeKotlinWorkspace,
    ProjectNativeKotlinWorkspaceProfile, semantic_scope_digest,
};
use crate::query_v2::{
    QUERY_INDEX_SCHEMA, QueryIndexManifest, build_query_index, verify_index, verify_index_manifest,
};
use crate::repository_snapshot::{RepositoryInputSnapshot, SNAPSHOT_SCHEMA, WorktreeKind, capture};
use crate::runtime::RuntimeAuthority;
use crate::session::{ModelCachePolicy, SessionAuthority};
use crate::state::StateAuthority;
use crate::worker::WorkerRequestCounters;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::fd::AsRawFd;

pub const READY_GENERATION_SCHEMA: &str = "codeclew-ready-generation/2.0";
pub const READY_GENERATION_SET_SCHEMA: &str = "codeclew-ready-generation-set/1.0";
const PREPARED_AUTHORITY_SCHEMA: &str = "codeclew-prepared-generation-authority/2.0";
const MODEL_ANALYSIS_SCHEMA: &str = "codeclew-project-native-analysis/2.0";
const INCREMENTAL_HEAD_SCHEMA: &str = "codeclew-incremental-head/2.0";
const INCREMENTAL_EVIDENCE_SCHEMA: &str = "codeclew-incremental-execution/2.0";
const WORKSPACE_PROFILE_SCHEMA: &str = "codeclew-project-native-workspace-profile/1.0";
const MAX_BINDING_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GenerationWorkspaceEvidence {
    schema: String,
    compilation_count: usize,
    materializations: u64,
    derived_mount_sets: u64,
    open_project_calls: u64,
}

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
    pub incremental: IncrementalExecutionEvidence,
    pub incremental_receipt: CasObject,
    pub repository_snapshot: CasObject,
    pub derived_input_manifest: CasObject,
    pub generation: CasObject,
    pub query_index: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadyGenerationSet {
    pub schema: String,
    pub generation_key: String,
    pub runtime_key: String,
    pub base_revision: String,
    pub repository_snapshot: CasObject,
    pub compilations: Vec<ReadyGeneration>,
    pub completeness: CompletenessVector,
    pub coverage: String,
    pub certainty: String,
    pub obligations: Vec<String>,
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
    descriptor: CompilationDescriptor,
    derived_input_manifest: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IncrementalExecutionMode {
    Full,
    UnchangedHit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncrementalExecutionEvidence {
    pub schema: String,
    pub planned: IncrementalPlan,
    pub executed: IncrementalExecutionMode,
    pub subset_analysis_supported: bool,
    pub worker_requests: WorkerRequestCounters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IncrementalHead {
    schema: String,
    compiler_store_key: String,
    receipt: CasObject,
    ready: CasObject,
}

struct LoadedIncrementalHead {
    head: IncrementalHead,
    receipt: IncrementalReceipt,
    ready: ReadyGeneration,
}

enum IncrementalHeadState {
    Missing,
    Ready(Box<LoadedIncrementalHead>),
    Corrupt,
}

impl IncrementalHeadState {
    fn ready(&self) -> Option<&LoadedIncrementalHead> {
        match self {
            Self::Ready(ready) => Some(ready.as_ref()),
            Self::Missing | Self::Corrupt => None,
        }
    }

    fn forced_full_plan(&self) -> Option<(IncrementalPlan, bool)> {
        matches!(self, Self::Corrupt).then_some((
            IncrementalPlan::Full {
                reason: FullAnalysisReason::InvalidReceipt,
            },
            false,
        ))
    }
}

pub fn ensure_session_generation(
    session: &SessionAuthority,
) -> Result<ReadyGenerationSet, ClewError> {
    let state = StateAuthority::process_default()?;
    let session_root = state.session_root(&session.session_id)?;
    let binding_path = session_root.join("generation.json");
    let store = CasStore::open(&state)?;
    if state.private_file_exists(&binding_path)? {
        return load_ready_set(&state, &store, &binding_path, session, false);
    }
    let repo = session.repository_path()?;
    let (snapshot, snapshot_object) = capture(&repo, &store)?;
    let compilation_root = session_root.join("compilations");
    state.directory_at(&compilation_root)?;
    let pool = generation_pool(session)?;
    let workspace = ProjectNativeKotlinWorkspace::prepare(&state, &store, &snapshot)?;
    // Only the immutable repository materialization and derived mounts are
    // shared. Until Kotlin exposes OpenProjectSet, every compilation below
    // still performs one OpenProject and retains that call in its own request
    // counters/evidence.
    let lane = GenerationLaneContext {
        session,
        repo: &repo,
        publish_head: true,
        snapshot: &snapshot,
        snapshot_object: &snapshot_object,
        workspace: &workspace,
    };
    let results = pool.install(|| {
        session
            .compilations
            .par_iter()
            .map(|compilation| {
                let component = digest_component(
                    &canonical::hash(&json!({
                        "schema":"codeclew-session-compilation-binding/1.0",
                        "compilation":compilation,
                    }))
                    .map_err(internal)?,
                )?
                .to_owned();
                ensure_generation(
                    &lane,
                    compilation,
                    &compilation_root.join(format!("{component}.json")),
                )
            })
            .collect::<Result<Vec<_>, ClewError>>()
    });
    let (results, profile) = finish_generation_workspace(results, workspace)?;
    write_generation_workspace_evidence(
        &state,
        &compilation_root.join("workspace-profile.json"),
        session.compilations.len(),
        profile,
    )?;
    let ready = assemble_ready_set(session, snapshot_object, results)?;
    write_ready_set(&state, &binding_path, &ready)?;
    Ok(ready)
}

pub fn ensure_candidate_generation(
    session: &SessionAuthority,
    repository: &Path,
    candidate_revision: &str,
    binding_path: &Path,
) -> Result<ReadyGenerationSet, ClewError> {
    if !git_oid(candidate_revision) {
        return Err(corrupt("candidate generation revision is invalid"));
    }
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let mut candidate = session.clone();
    candidate.base_revision = candidate_revision.into();
    if state.private_file_exists(binding_path)? {
        return load_ready_set(&state, &store, binding_path, &candidate, false);
    }
    let (snapshot, snapshot_object) = capture(repository, &store)?;
    let parent = binding_path
        .parent()
        .ok_or_else(|| corrupt("candidate generation binding has no parent"))?;
    let pool = generation_pool(&candidate)?;
    let workspace = ProjectNativeKotlinWorkspace::prepare(&state, &store, &snapshot)?;
    // This is one shared filesystem authority, not one project-model call.
    // Each component preserves its independent OpenProject counter.
    let lane = GenerationLaneContext {
        session: &candidate,
        repo: repository,
        publish_head: false,
        snapshot: &snapshot,
        snapshot_object: &snapshot_object,
        workspace: &workspace,
    };
    let results = pool.install(|| {
        candidate
            .compilations
            .par_iter()
            .map(|compilation| {
                let component = digest_component(
                    &canonical::hash(&json!({
                        "schema":"codeclew-candidate-compilation-binding/1.0",
                        "compilation":compilation,
                    }))
                    .map_err(internal)?,
                )?
                .to_owned();
                ensure_generation(
                    &lane,
                    compilation,
                    &parent.join(format!("staged-generation-{component}.json")),
                )
            })
            .collect::<Result<Vec<_>, ClewError>>()
    });
    let (results, profile) = finish_generation_workspace(results, workspace)?;
    write_generation_workspace_evidence(
        &state,
        &parent.join("staged-workspace-profile.json"),
        candidate.compilations.len(),
        profile,
    )?;
    let ready = assemble_ready_set(&candidate, snapshot_object, results)?;
    write_ready_set(&state, binding_path, &ready)?;
    Ok(ready)
}

fn generation_pool(session: &SessionAuthority) -> Result<rayon::ThreadPool, ClewError> {
    let resources = HostResources::detect()?;
    let admitted = admitted_generation_jobs(resources, session.compilations.len());
    let jobs = session.generation_jobs.unwrap_or(admitted);
    if jobs > admitted {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "generation job count exceeds CPU or memory admission",
        ));
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .thread_name(|index| format!("clew-generation-{index}"))
        .build()
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))
}

fn admitted_generation_jobs(resources: HostResources, compilation_count: usize) -> usize {
    let memory_jobs = usize::try_from(
        resources
            .codeclew_memory_budget_bytes
            .checked_div(2 * 1024 * 1024 * 1024)
            .unwrap_or(0),
    )
    .unwrap_or(usize::MAX)
    .max(1);
    resources
        .logical_cpu
        .min(memory_jobs)
        .min(compilation_count)
        .clamp(1, 16)
}

fn finish_generation_workspace<T>(
    results: Result<T, ClewError>,
    workspace: ProjectNativeKotlinWorkspace,
) -> Result<(T, ProjectNativeKotlinWorkspaceProfile), ClewError> {
    let workspace_result = workspace.finish();
    match results {
        Ok(results) => Ok((results, workspace_result?)),
        Err(error) => {
            let _ = workspace_result;
            Err(error)
        }
    }
}

fn write_generation_workspace_evidence(
    state: &StateAuthority,
    path: &Path,
    compilation_count: usize,
    profile: ProjectNativeKotlinWorkspaceProfile,
) -> Result<(), ClewError> {
    if compilation_count == 0
        || profile.materializations != 1
        || profile.derived_mount_sets != 1
        || profile.open_project_calls > compilation_count as u64
    {
        return Err(corrupt("project-native workspace profile is inconsistent"));
    }
    write_canonical_atomic(
        state,
        path,
        &GenerationWorkspaceEvidence {
            schema: WORKSPACE_PROFILE_SCHEMA.into(),
            compilation_count,
            materializations: profile.materializations,
            derived_mount_sets: profile.derived_mount_sets,
            open_project_calls: profile.open_project_calls,
        },
    )
}

struct GenerationLaneContext<'a> {
    session: &'a SessionAuthority,
    repo: &'a Path,
    publish_head: bool,
    snapshot: &'a RepositoryInputSnapshot,
    snapshot_object: &'a CasObject,
    workspace: &'a ProjectNativeKotlinWorkspace,
}

fn ensure_generation(
    lane: &GenerationLaneContext<'_>,
    compilation: &str,
    binding_path: &Path,
) -> Result<ReadyGeneration, ClewError> {
    let GenerationLaneContext {
        session,
        repo,
        publish_head,
        snapshot,
        snapshot_object,
        workspace,
    } = *lane;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    if state.private_file_exists(binding_path)? {
        return load_ready(&state, &store, binding_path, session, compilation, false);
    }
    let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "generation service must run through ./clew",
        )
    })?;
    let repository = state.repository(repo)?;
    if repository.key != session.repository_key {
        return Err(corrupt(
            "generation repository differs from session Git authority",
        ));
    }
    let preparation_key = project_model_key(&runtime.runtime_key, snapshot_object, compilation)?;
    let _preparation_lock = GenerationLock::acquire(&state, &preparation_key)?;
    if state.private_file_exists(binding_path)? {
        return load_ready(&state, &store, binding_path, session, compilation, false);
    }
    let compiler_namespace = compiler_store_key(&runtime, compilation)?;
    let external_build_state = session.external_build_state_path()?;
    let live_attempt = workspace.open_compilation(
        &state,
        compilation,
        digest_component(&compiler_namespace)?,
        external_build_state.as_deref(),
    )?;
    let prepared = ensure_prepared_authority(
        &state,
        &store,
        &repository.root,
        session,
        &runtime,
        snapshot,
        snapshot_object,
        compilation,
        live_attempt.project_authority(),
    )?;
    let generation_key = final_generation_key(
        &runtime.runtime_key,
        &session.base_revision,
        snapshot_object,
        compilation,
        &prepared.derived_input_manifest,
    )?;
    let _lock = GenerationLock::acquire(&state, &generation_key)?;
    if state.private_file_exists(binding_path)? {
        live_attempt.close_without_analysis()?;
        return load_ready(&state, &store, binding_path, session, compilation, false);
    }
    let cache_root = repository.root.join("generations");
    state.directory_at(&cache_root)?;
    let cache_path = cache_root.join(format!("{}.json", digest_component(&generation_key)?));
    let compiler_store = CompilerStoreKey::create(
        compiler_line(&prepared.compiler_version)?.2,
        prepared.adapter_digest.clone(),
        &prepared.descriptor,
    )?;
    let head_path = incremental_head_path(&repository.root, compilation)?;
    let head_lock_key = canonical::hash(&json!({
        "schema":"codeclew-incremental-head-lock/2.0",
        "repositoryKey":session.repository_key,
        "compilation":compilation,
    }))
    .map_err(internal)?;
    let _head_lock = GenerationLock::acquire(&state, &head_lock_key)?;
    let head = load_incremental_head_for_planning(&state, &store, &head_path)?;
    let previous = head.ready();
    let (plan, unchanged_is_exact) = match head.forced_full_plan() {
        Some(forced) => forced,
        None => incremental_plan_for(
            &store,
            snapshot,
            snapshot_object,
            &prepared,
            &compiler_store,
            previous,
        )?,
    };
    let ready = if unchanged_is_exact {
        build_unchanged_ready(
            &state,
            &store,
            session,
            &runtime,
            snapshot_object.clone(),
            generation_key,
            &prepared,
            previous.expect("exact unchanged head"),
            plan,
            live_attempt,
        )?
    } else {
        build_ready(
            &state,
            &store,
            session,
            &runtime,
            snapshot_object.clone(),
            generation_key,
            prepared,
            compiler_store.clone(),
            plan,
            live_attempt,
        )?
    };
    if session.model_cache_policy != ModelCachePolicy::NonCacheable {
        write_private_atomic(&state, &cache_path, &ready)?;
    }
    if publish_head {
        publish_incremental_head(&state, &store, &head_path, &ready, &compiler_store.key)?;
    }
    write_private_atomic(&state, binding_path, &ready)?;
    Ok(ready)
}

pub fn store_ready_generation(
    store: &CasStore,
    ready: &ReadyGenerationSet,
) -> Result<CasObject, ClewError> {
    store.put(
        READY_GENERATION_SET_SCHEMA,
        &canonical::bytes(ready).map_err(internal)?,
    )
}

pub fn load_candidate_generation(
    store: &CasStore,
    object: &CasObject,
    session: &SessionAuthority,
    candidate_revision: &str,
    deep: bool,
) -> Result<ReadyGenerationSet, ClewError> {
    if object.object_schema != READY_GENERATION_SET_SCHEMA {
        return Err(corrupt("candidate generation CAS schema is invalid"));
    }
    let ready: ReadyGenerationSet = read_canonical_object(store, object)?;
    let mut candidate = session.clone();
    candidate.base_revision = candidate_revision.into();
    verify_ready_set(store, &ready, &candidate, deep)?;
    Ok(ready)
}

pub fn publish_candidate_generation(
    session: &SessionAuthority,
    ready: &ReadyGenerationSet,
) -> Result<(), ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let mut candidate = session.clone();
    candidate.base_revision = ready.base_revision.clone();
    verify_ready_set(&store, ready, &candidate, true)?;
    let repository = state.repository(&session.target_repository_path()?)?;
    if repository.key != session.repository_key {
        return Err(corrupt(
            "candidate generation publish repository authority changed",
        ));
    }
    for compilation in &ready.compilations {
        let receipt: IncrementalReceipt =
            read_canonical_object(&store, &compilation.incremental_receipt)?;
        receipt.validate()?;
        let head_path = incremental_head_path(&repository.root, &compilation.compilation)?;
        let lock_key = canonical::hash(&json!({
            "schema":"codeclew-incremental-head-lock/2.0",
            "repositoryKey":session.repository_key,
            "compilation":compilation.compilation,
        }))
        .map_err(internal)?;
        let _lock = GenerationLock::acquire(&state, &lock_key)?;
        publish_incremental_head(
            &state,
            &store,
            &head_path,
            compilation,
            &receipt.compiler_store_key,
        )?;
    }
    Ok(())
}

pub fn load_session_generation(
    session: &SessionAuthority,
) -> Result<ReadyGenerationSet, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let path = state
        .session_root(&session.session_id)?
        .join("generation.json");
    load_ready_set(&state, &store, &path, session, false)
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

fn load_compilation_snapshot(
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

pub fn load_snapshot(
    store: &CasStore,
    ready: &ReadyGenerationSet,
) -> Result<RepositoryInputSnapshot, ClewError> {
    let first = ready
        .compilations
        .first()
        .ok_or_else(|| corrupt("ready generation set is empty"))?;
    if first.repository_snapshot != ready.repository_snapshot {
        return Err(corrupt(
            "ready generation set snapshot authority is inconsistent",
        ));
    }
    load_compilation_snapshot(store, first)
}

#[allow(clippy::too_many_arguments)]
fn build_ready(
    state: &StateAuthority,
    store: &CasStore,
    session: &SessionAuthority,
    runtime: &RuntimeAuthority,
    snapshot_object: CasObject,
    generation_key: String,
    prepared: PreparedGenerationAuthority,
    compiler_store: CompilerStoreKey,
    planned: IncrementalPlan,
    live_attempt: ProjectNativeKotlinAttempt,
) -> Result<ReadyGeneration, ClewError> {
    let line = compiler_line(&prepared.compiler_version)?.1;
    let semantic_output = Arc::new(Mutex::new(None));
    let cancellation = live_attempt.cancellation_handle();
    let driver = LiveKotlinDriver {
        attempt: Mutex::new(Some(live_attempt)),
        cancellation,
        store: store.clone(),
        output: Arc::clone(&semantic_output),
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
    let analysis = match HostResources::detect().and_then(|resources| {
        let jobs = if session.compilations.len() > 1 {
            1
        } else {
            resources.logical_cpu.min(16)
        };
        execute_analysis_dag_with_jobs(state, Arc::new(registry), request, resources, jobs)
    }) {
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
                compilation_id: safe_compilation_id(&prepared.compilation),
                capability: CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?,
                completion: analysis.completion,
            }],
            analysis.runs,
        )?;
        let (_, query_index_object) =
            build_query_index(store, &generation, generation_object.clone())?;
        let semantic = semantic_output
            .lock()
            .map_err(poisoned)?
            .clone()
            .ok_or_else(|| corrupt("semantic adapter produced no execution authority"))?;
        let index = load_analysis(store, &semantic.analysis)?;
        let scope_digest = semantic_scope_digest(&index)?;
        let completeness = completeness_from_index(&index, &scope_digest)?;
        let (incremental_receipt, incremental_receipt_object) = create_incremental_receipt(
            store,
            &index,
            &compiler_store,
            &generation,
            completeness.clone(),
        )?;
        incremental_receipt.validate()?;
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key,
            runtime_key: runtime.runtime_key.clone(),
            base_revision: session.base_revision.clone(),
            compilation: prepared.compilation.clone(),
            compiler_version: prepared.compiler_version,
            completeness: completeness.clone(),
            coverage: coverage_label(&completeness).into(),
            certainty: certainty_label(&completeness).into(),
            obligations: obligation_codes(&completeness),
            incremental: full_execution_evidence(planned, semantic.worker_requests),
            incremental_receipt: incremental_receipt_object,
            repository_snapshot: snapshot_object,
            derived_input_manifest: prepared.derived_input_manifest,
            generation: generation_object,
            query_index: query_index_object,
        };
        verify_ready(store, &ready, session, &ready.compilation, true)?;
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

#[allow(clippy::too_many_arguments)]
fn build_unchanged_ready(
    state: &StateAuthority,
    store: &CasStore,
    session: &SessionAuthority,
    runtime: &RuntimeAuthority,
    snapshot_object: CasObject,
    generation_key: String,
    prepared: &PreparedGenerationAuthority,
    previous: &LoadedIncrementalHead,
    planned: IncrementalPlan,
    live_attempt: ProjectNativeKotlinAttempt,
) -> Result<ReadyGeneration, ClewError> {
    if !matches!(planned, IncrementalPlan::UnchangedHit { .. }) {
        return Err(corrupt("unchanged generation has a non-unchanged plan"));
    }
    let mut journal = AttemptJournal::create(state.clone(), &generation_key, 0)?;
    journal.transition(AttemptState::Snapshotted, snapshot_object.digest.clone())?;
    journal.transition(
        AttemptState::Modeled,
        prepared.derived_input_manifest.digest.clone(),
    )?;
    journal.transition(
        AttemptState::Analyzing,
        "incremental UNCHANGED_HIT; IndexFiles skipped",
    )?;
    let counters = match live_attempt.close_without_analysis() {
        Ok(counters) => counters,
        Err(error) => {
            journal.transition(AttemptState::Failed, "unchanged worker close failed")?;
            return Err(error);
        }
    };
    if counters.open_project_requests != 1 || counters.index_files_requests != 0 {
        journal.transition(
            AttemptState::Failed,
            "unchanged request counters are invalid",
        )?;
        return Err(corrupt(
            "UNCHANGED_HIT executed an unexpected worker request contour",
        ));
    }
    journal.transition(AttemptState::Finalizing, "reusing immutable generation")?;
    let mut ready = previous.ready.clone();
    ready.generation_key = generation_key;
    ready.runtime_key = runtime.runtime_key.clone();
    ready.base_revision = session.base_revision.clone();
    ready.compilation = prepared.compilation.clone();
    ready.compiler_version = prepared.compiler_version.clone();
    ready.repository_snapshot = snapshot_object;
    ready.derived_input_manifest = prepared.derived_input_manifest.clone();
    ready.incremental = IncrementalExecutionEvidence {
        schema: INCREMENTAL_EVIDENCE_SCHEMA.into(),
        planned,
        executed: IncrementalExecutionMode::UnchangedHit,
        subset_analysis_supported: false,
        worker_requests: counters,
    };
    ready.incremental_receipt = previous.head.receipt.clone();
    verify_ready(store, &ready, session, &ready.compilation, true)?;
    journal.transition(AttemptState::Ready, ready.generation.digest.clone())?;
    Ok(ready)
}

fn full_execution_evidence(
    planned: IncrementalPlan,
    worker_requests: WorkerRequestCounters,
) -> IncrementalExecutionEvidence {
    IncrementalExecutionEvidence {
        schema: INCREMENTAL_EVIDENCE_SCHEMA.into(),
        planned,
        executed: IncrementalExecutionMode::Full,
        subset_analysis_supported: false,
        worker_requests,
    }
}

#[allow(clippy::too_many_arguments)]
fn ensure_prepared_authority(
    state: &StateAuthority,
    store: &CasStore,
    repository_root: &Path,
    session: &SessionAuthority,
    runtime: &RuntimeAuthority,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: &CasObject,
    compilation: &str,
    project: &Value,
) -> Result<PreparedGenerationAuthority, ClewError> {
    let model_key = project_model_key(&runtime.runtime_key, snapshot_object, compilation)?;
    let root = repository_root.join("generations/models");
    state.directory_at(&root)?;
    let path = root.join(format!("{}.json", digest_component(&model_key)?));
    let current = prepare_authority(
        store,
        runtime,
        snapshot,
        snapshot_object.clone(),
        compilation,
        project,
    )?;
    if session.model_cache_policy != ModelCachePolicy::NonCacheable
        && state.private_file_exists(&path)?
    {
        let cached =
            load_prepared_authority(state, store, &path, runtime, snapshot_object, compilation)?;
        if cached != current {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "cached build model differs from the exact live OpenProject authority",
            ));
        }
        return Ok(cached);
    }
    if session.model_cache_policy != ModelCachePolicy::NonCacheable {
        write_canonical_atomic(state, &path, &current)?;
    }
    Ok(current)
}

fn project_model_key(
    runtime_key: &str,
    snapshot: &CasObject,
    compilation: &str,
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-project-native-model-key/2.0",
        "runtimeKey":runtime_key,
        "snapshot":snapshot,
        "compilation":compilation,
    }))
    .map_err(internal)
}

fn prepare_authority(
    store: &CasStore,
    runtime: &RuntimeAuthority,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: CasObject,
    compilation: &str,
    project: &Value,
) -> Result<PreparedGenerationAuthority, ClewError> {
    let compiler_version = project
        .get("compilerVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt("OpenProject has no Kotlin compiler identity"))?
        .to_owned();
    let (worker_name, _, _) = compiler_line(&compiler_version)?;
    let worker = runtime.worker(worker_name)?;
    if worker.compiler_version != compiler_version {
        return Err(corrupt("runtime worker compiler identity changed"));
    }
    let project_model_hash = required_digest(project, "projectModelHash")?;
    let semantic_input_manifest_hash = required_digest(project, "semanticInputManifestHash")?;
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
            "projectModelHash":project_model_hash,
            "semanticInputManifestHash":semantic_input_manifest_hash,
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
        completeness: DescriptorCompleteness::Unknown,
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
        descriptor,
        derived_input_manifest,
    };
    Ok(prepared)
}

fn load_prepared_authority(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    runtime: &RuntimeAuthority,
    snapshot: &CasObject,
    compilation: &str,
) -> Result<PreparedGenerationAuthority, ClewError> {
    let bytes = state
        .read_private_file(path, MAX_BINDING_BYTES)
        .map_err(|_| corrupt("prepared authority binding is unsafe"))?;
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
        || prepared.descriptor.completeness != DescriptorCompleteness::Unknown
    {
        return Err(corrupt("prepared generation authority is inconsistent"));
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
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-generation-key/2.2",
        "runtimeKey":runtime_key,
        "baseRevision":base_revision,
        "snapshot":snapshot,
        "compilation":compilation,
        "derivedInputManifest":derived_input_manifest,
    }))
    .map_err(internal)
}

#[derive(Clone)]
struct SemanticExecutionOutput {
    analysis: CasObject,
    worker_requests: WorkerRequestCounters,
}

struct LiveKotlinDriver {
    attempt: Mutex<Option<ProjectNativeKotlinAttempt>>,
    cancellation: crate::worker::WorkerCancellationHandle,
    store: CasStore,
    output: Arc<Mutex<Option<SemanticExecutionOutput>>>,
}

impl KotlinGenerationDriver for LiveKotlinDriver {
    fn analyze(&self, _request: &AnalyzeGenerationRequest) -> Result<Value, ClewError> {
        let attempt = self
            .attempt
            .lock()
            .map_err(poisoned)?
            .take()
            .ok_or_else(|| corrupt("live Kotlin analysis was consumed more than once"))?;
        let (index, _profile, worker_requests) = attempt.analyze()?;
        let analysis = self.store.put(
            MODEL_ANALYSIS_SCHEMA,
            &canonical::bytes(&index).map_err(internal)?,
        )?;
        *self.output.lock().map_err(poisoned)? = Some(SemanticExecutionOutput {
            analysis,
            worker_requests,
        });
        Ok(index)
    }

    fn cancel(&self) -> Result<(), ClewError> {
        self.cancellation.cancel()
    }
}

fn required_digest(value: &Value, field: &str) -> Result<String, ClewError> {
    let digest = value.get(field).and_then(Value::as_str).ok_or_else(|| {
        ClewError::new(
            ErrorCode::StateCorrupt,
            format!("OpenProject has no {field}"),
        )
    })?;
    if digest.len() != 71
        || !digest.starts_with("sha256:")
        || !digest[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            format!("OpenProject {field} is not a digest"),
        ));
    }
    Ok(digest.into())
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
    let observer = Arc::new(CompositeProgress::new(vec![
        Arc::new(PersistentProgress::open(state, &request.attempt_id)?),
        Arc::new(StderrProgress),
    ])?);
    let scheduler = DagScheduler::new(resources, observer)?;
    let attempt_id = request.attempt_id.clone();
    let state_for_executor = state.clone();
    let analysis_for_executor = Arc::clone(&analysis);
    let runs_for_executor = Arc::clone(&runs);
    let report = scheduler.execute(
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
                Ok(json!({"factCount":completion.fact_count,"sealedCompilerStreams":1}))
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
    crate::cold_start::persist_dag_report(state, &attempt_id, &report)?;
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

fn incremental_head_path(
    repository_root: &Path,
    compilation: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let key = canonical::hash(&json!({
        "schema":"codeclew-incremental-head-path/2.0",
        "compilation":compilation,
    }))
    .map_err(internal)?;
    Ok(repository_root
        .join("incremental")
        .join(format!("{}.json", digest_component(&key)?)))
}

fn incremental_plan_for(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: &CasObject,
    prepared: &PreparedGenerationAuthority,
    compiler_store: &CompilerStoreKey,
    previous: Option<&LoadedIncrementalHead>,
) -> Result<(IncrementalPlan, bool), ClewError> {
    let receipt_paths = previous
        .map(|value| {
            value
                .receipt
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    let current_files = current_file_digests(store, snapshot, &receipt_paths)?;
    let plan = plan_incremental(
        compiler_store,
        previous.map(|value| &value.receipt),
        &current_files,
        true,
    )?;
    let exact = matches!(plan, IncrementalPlan::UnchangedHit { .. })
        && previous.is_some_and(|value| {
            value.ready.repository_snapshot == *snapshot_object
                && value.ready.derived_input_manifest == prepared.derived_input_manifest
        });
    if matches!(plan, IncrementalPlan::UnchangedHit { .. }) && !exact {
        return Ok((
            IncrementalPlan::Full {
                reason: FullAnalysisReason::UnknownInvalidation,
            },
            false,
        ));
    }
    // The current worker protocol cannot analyze a proven subset. DELTA remains
    // useful planning evidence, but execution is deliberately a full analysis.
    Ok((plan, exact))
}

fn current_file_digests(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    receipt_paths: &std::collections::BTreeSet<&str>,
) -> Result<BTreeMap<String, String>, ClewError> {
    let mut files = BTreeMap::new();
    for entry in snapshot.index.iter().filter(|entry| {
        entry.stage == 0
            && (kotlin_source_path(&entry.path) || receipt_paths.contains(entry.path.as_str()))
    }) {
        files.insert(
            entry.path.clone(),
            source_content_digest(store, &entry.content)?,
        );
    }
    for entry in &snapshot.worktree {
        if !kotlin_source_path(&entry.path) && !receipt_paths.contains(entry.path.as_str()) {
            continue;
        }
        match entry.kind {
            WorktreeKind::Missing => {
                files.remove(&entry.path);
            }
            WorktreeKind::Regular => {
                let content = entry
                    .content
                    .as_ref()
                    .ok_or_else(|| corrupt("regular worktree input has no content authority"))?;
                files.insert(entry.path.clone(), source_content_digest(store, content)?);
            }
            WorktreeKind::Symlink => {
                return Err(ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "symlinked incremental inputs require full analysis",
                ));
            }
        }
    }
    Ok(files)
}

fn source_content_digest(store: &CasStore, object: &CasObject) -> Result<String, ClewError> {
    let limit = usize::try_from(object.size)
        .map_err(|_| resource("repository source input exceeds host size"))?;
    let lease = store.read(object, limit)?;
    Ok(canonical::hash_bytes(lease.bytes()))
}

fn kotlin_source_path(path: &str) -> bool {
    path.ends_with(".kt")
}

pub(crate) fn create_incremental_receipt(
    store: &CasStore,
    index: &Value,
    compiler_store: &CompilerStoreKey,
    generation: &GenerationManifest,
    completeness: CompletenessVector,
) -> Result<(IncrementalReceipt, CasObject), ClewError> {
    let files = index
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt("verified compiler index has no file manifest"))?;
    let descriptors = index
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt("verified compiler index has no descriptor rows"))?;
    let relations = index
        .pointer("/declarationRelations/relations")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt("verified compiler index has no relation rows"))?;

    let mut aliases = BTreeMap::<String, String>::new();
    let mut surfaces = BTreeMap::<String, Vec<Value>>::new();
    for descriptor in descriptors {
        let path = required_safe_path(descriptor, "file")?;
        surfaces
            .entry(path.clone())
            .or_default()
            .push(descriptor.clone());
        for field in ["symbolIdentity", "compilerCallableId"] {
            if let Some(symbol) = descriptor.get(field).and_then(Value::as_str) {
                match aliases.insert(symbol.into(), path.clone()) {
                    Some(previous) if previous != path => {
                        return Err(corrupt("compiler symbol maps to multiple source files"));
                    }
                    _ => {}
                }
            }
        }
    }
    for rows in surfaces.values_mut() {
        rows.sort_by_key(|row| canonical::bytes(row).unwrap_or_default());
    }

    let mut dependencies = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    let mut boundary_rows = BTreeMap::<(String, String), Vec<String>>::new();
    for relation in relations {
        let source = required_safe_path(relation, "file")?;
        let Some(target_symbol) = relation.get("target").and_then(Value::as_str) else {
            continue;
        };
        let Some(target) = aliases.get(target_symbol) else {
            continue;
        };
        if target != &source {
            dependencies
                .entry(source.clone())
                .or_default()
                .insert(target.clone());
            boundary_rows
                .entry((source, target.clone()))
                .or_default()
                .push(canonical::hash(relation).map_err(internal)?);
        }
    }

    let mut file_receipts = Vec::with_capacity(files.len());
    for file in files {
        let path = required_safe_path(file, "path")?;
        let content_digest = required_digest(file, "contentHash")?;
        let surface = surfaces.remove(&path).unwrap_or_default();
        file_receipts.push(FileReceipt {
            path: path.clone(),
            content_digest,
            exported_surface_digest: canonical::hash(&surface).map_err(internal)?,
            dependencies: dependencies
                .remove(&path)
                .unwrap_or_default()
                .into_iter()
                .collect(),
        });
    }
    file_receipts.sort_by(|left, right| left.path.cmp(&right.path));
    if file_receipts
        .windows(2)
        .any(|pair| pair[0].path == pair[1].path)
    {
        return Err(corrupt("verified compiler index has duplicate file paths"));
    }
    let mut boundaries = boundary_rows
        .into_iter()
        .map(|((source_path, target_path), mut rows)| {
            rows.sort();
            Ok(BoundaryReceipt {
                source_path,
                target_path,
                boundary_digest: canonical::hash(&rows).map_err(internal)?,
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    boundaries.sort_by(|left, right| {
        (&left.source_path, &left.target_path).cmp(&(&right.source_path, &right.target_path))
    });
    let receipt = IncrementalReceipt {
        schema: INCREMENTAL_RECEIPT_SCHEMA.into(),
        compiler_store_key: compiler_store.key.clone(),
        generation_id: generation.generation_id.clone(),
        files: file_receipts,
        boundaries,
        completeness,
    };
    receipt.validate()?;
    let object = store.put(
        INCREMENTAL_RECEIPT_SCHEMA,
        &canonical::bytes(&receipt).map_err(internal)?,
    )?;
    Ok((receipt, object))
}

fn required_safe_path(value: &Value, field: &str) -> Result<String, ClewError> {
    let path = value.get(field).and_then(Value::as_str).ok_or_else(|| {
        ClewError::new(
            ErrorCode::StateCorrupt,
            format!("compiler index row has no {field}"),
        )
    })?;
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(corrupt("compiler index contains an unsafe source path"));
    }
    Ok(path.into())
}

fn publish_incremental_head(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    ready: &ReadyGeneration,
    compiler_store_key: &str,
) -> Result<(), ClewError> {
    let _ = digest_component(compiler_store_key)?;
    let parent = path
        .parent()
        .ok_or_else(|| corrupt("incremental head has no managed parent"))?;
    state.directory_at(parent)?;
    let ready_object = store.put(
        READY_GENERATION_SCHEMA,
        &canonical::bytes(ready).map_err(internal)?,
    )?;
    let head = IncrementalHead {
        schema: INCREMENTAL_HEAD_SCHEMA.into(),
        compiler_store_key: compiler_store_key.into(),
        receipt: ready.incremental_receipt.clone(),
        ready: ready_object,
    };
    write_canonical_atomic(state, path, &head)
}

fn load_incremental_head(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
) -> Result<Option<LoadedIncrementalHead>, ClewError> {
    if !state.private_file_exists(path)? {
        return Ok(None);
    }
    let bytes = state
        .read_private_file(path, MAX_BINDING_BYTES)
        .map_err(|_| corrupt("incremental head binding is unsafe"))?;
    let head: IncrementalHead = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("incremental head binding is invalid"))?;
    if canonical::bytes(&head).map_err(internal)? != bytes
        || head.schema != INCREMENTAL_HEAD_SCHEMA
        || head.receipt.object_schema != INCREMENTAL_RECEIPT_SCHEMA
        || head.ready.object_schema != READY_GENERATION_SCHEMA
    {
        return Err(corrupt("incremental head authority is invalid"));
    }
    let receipt: IncrementalReceipt = read_canonical_object(store, &head.receipt)?;
    receipt
        .validate()
        .map_err(|_| corrupt("incremental receipt authority is invalid"))?;
    let ready: ReadyGeneration = read_canonical_object(store, &head.ready)?;
    verify_ready_authority(store, &ready, true)
        .map_err(|_| corrupt("incremental ready authority is invalid"))?;
    if head.compiler_store_key != receipt.compiler_store_key
        || head.receipt != ready.incremental_receipt
        || receipt.generation_id != load_generation(store, &ready.generation)?.generation_id
    {
        return Err(corrupt("incremental head objects are not mutually bound"));
    }
    Ok(Some(LoadedIncrementalHead {
        head,
        receipt,
        ready,
    }))
}

fn load_incremental_head_for_planning(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
) -> Result<IncrementalHeadState, ClewError> {
    match load_incremental_head(state, store, path) {
        Ok(Some(ready)) => Ok(IncrementalHeadState::Ready(Box::new(ready))),
        Ok(None) => Ok(IncrementalHeadState::Missing),
        Err(error) if error.code == ErrorCode::StateCorrupt => Ok(IncrementalHeadState::Corrupt),
        Err(error) => Err(error),
    }
}

fn read_canonical_object<T: for<'de> Deserialize<'de> + Serialize>(
    store: &CasStore,
    object: &CasObject,
) -> Result<T, ClewError> {
    let limit =
        usize::try_from(object.size).map_err(|_| resource("CAS object exceeds host size"))?;
    let lease = store.read(object, limit)?;
    let value = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("CAS object payload is invalid"))?;
    if canonical::bytes(&value).map_err(internal)? != lease.bytes() {
        return Err(corrupt("CAS object payload is not canonical"));
    }
    Ok(value)
}

fn load_generation(store: &CasStore, object: &CasObject) -> Result<GenerationManifest, ClewError> {
    read_canonical_object(store, object)
}

fn ready_set_key(
    runtime_key: &str,
    base_revision: &str,
    repository_snapshot: &CasObject,
    compilations: &[ReadyGeneration],
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-ready-generation-set-key/1.0",
        "runtimeKey":runtime_key,
        "baseRevision":base_revision,
        "repositorySnapshot":repository_snapshot,
        "compilations":compilations.iter().map(|ready| json!({
            "compilation":ready.compilation,
            "generationKey":ready.generation_key,
        })).collect::<Vec<_>>(),
    }))
    .map_err(internal)
}

fn assemble_ready_set(
    session: &SessionAuthority,
    repository_snapshot: CasObject,
    mut compilations: Vec<ReadyGeneration>,
) -> Result<ReadyGenerationSet, ClewError> {
    compilations.sort_by(|left, right| left.compilation.cmp(&right.compilation));
    let observed = compilations
        .iter()
        .map(|ready| ready.compilation.clone())
        .collect::<Vec<_>>();
    if observed != session.compilations
        || compilations
            .iter()
            .any(|ready| ready.repository_snapshot != repository_snapshot)
    {
        return Err(corrupt(
            "compilation generation set is incomplete or inconsistent",
        ));
    }
    let completeness = aggregate_completeness(&compilations)?;
    let generation_key = ready_set_key(
        &session.runtime_key,
        &session.base_revision,
        &repository_snapshot,
        &compilations,
    )?;
    let ready = ReadyGenerationSet {
        schema: READY_GENERATION_SET_SCHEMA.into(),
        generation_key,
        runtime_key: session.runtime_key.clone(),
        base_revision: session.base_revision.clone(),
        repository_snapshot,
        compilations,
        coverage: coverage_label(&completeness).into(),
        certainty: certainty_label(&completeness).into(),
        obligations: obligation_codes(&completeness),
        completeness,
    };
    verify_ready_set_authority(
        &CasStore::open(&StateAuthority::process_default()?)?,
        &ready,
        false,
    )?;
    Ok(ready)
}

fn aggregate_completeness(
    compilations: &[ReadyGeneration],
) -> Result<CompletenessVector, ClewError> {
    if compilations.is_empty() {
        return Err(corrupt("ready generation set is empty"));
    }
    for compilation in compilations {
        compilation.completeness.validate()?;
    }
    if compilations
        .iter()
        .all(|ready| ready.completeness.publishable())
    {
        let scopes = compilations
            .iter()
            .map(|ready| {
                json!({
                    "compilation":ready.compilation,
                    "coverage":ready.completeness.coverage,
                })
            })
            .collect::<Vec<_>>();
        CompletenessVector::verified_complete(canonical::hash(&scopes).map_err(internal)?)
    } else {
        let mut values = compilations.iter().map(|ready| ready.completeness.clone());
        let first = values
            .next()
            .ok_or_else(|| corrupt("ready generation set is empty"))?;
        values.try_fold(first, |combined, value| combined.meet(&value))
    }
}

fn load_ready_set(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    session: &SessionAuthority,
    deep: bool,
) -> Result<ReadyGenerationSet, ClewError> {
    let bytes = state
        .read_private_file(path, MAX_BINDING_BYTES)
        .map_err(|_| corrupt("ready generation-set binding is unsafe"))?;
    let ready: ReadyGenerationSet = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("ready generation-set binding is invalid"))?;
    if canonical::bytes(&ready).map_err(internal)? != bytes {
        return Err(corrupt("ready generation-set binding is not canonical"));
    }
    verify_ready_set(store, &ready, session, deep)?;
    Ok(ready)
}

fn verify_ready_set(
    store: &CasStore,
    ready: &ReadyGenerationSet,
    session: &SessionAuthority,
    deep: bool,
) -> Result<(), ClewError> {
    if ready.runtime_key != session.runtime_key
        || ready.base_revision != session.base_revision
        || ready
            .compilations
            .iter()
            .map(|value| &value.compilation)
            .ne(session.compilations.iter())
    {
        return Err(corrupt("ready generation set session authority is invalid"));
    }
    verify_ready_set_authority(store, ready, deep)
}

fn verify_ready_set_authority(
    store: &CasStore,
    ready: &ReadyGenerationSet,
    deep: bool,
) -> Result<(), ClewError> {
    let aggregate = aggregate_completeness(&ready.compilations)?;
    if ready.schema != READY_GENERATION_SET_SCHEMA
        || ready.compilations.is_empty()
        || ready.compilations.len() > 64
        || !ready
            .compilations
            .windows(2)
            .all(|pair| pair[0].compilation < pair[1].compilation)
        || ready.compilations.iter().any(|compilation| {
            compilation.runtime_key != ready.runtime_key
                || compilation.base_revision != ready.base_revision
                || compilation.repository_snapshot != ready.repository_snapshot
        })
        || ready.completeness != aggregate
        || ready.coverage != coverage_label(&aggregate)
        || ready.certainty != certainty_label(&aggregate)
        || ready.obligations != obligation_codes(&aggregate)
        || ready.generation_key
            != ready_set_key(
                &ready.runtime_key,
                &ready.base_revision,
                &ready.repository_snapshot,
                &ready.compilations,
            )?
    {
        return Err(corrupt("ready generation set authority is invalid"));
    }
    for compilation in &ready.compilations {
        verify_ready_authority(store, compilation, deep)?;
    }
    Ok(())
}

fn write_ready_set(
    state: &StateAuthority,
    path: &Path,
    ready: &ReadyGenerationSet,
) -> Result<(), ClewError> {
    write_canonical_atomic(state, path, ready)
}

fn load_ready(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    session: &SessionAuthority,
    compilation: &str,
    deep: bool,
) -> Result<ReadyGeneration, ClewError> {
    let bytes = state
        .read_private_file(path, MAX_BINDING_BYTES)
        .map_err(|_| corrupt("ready generation binding is unsafe"))?;
    let ready: ReadyGeneration =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("ready generation is invalid"))?;
    if canonical::bytes(&ready).map_err(internal)? != bytes {
        return Err(corrupt("ready generation binding is not canonical"));
    }
    verify_ready(store, &ready, session, compilation, deep)?;
    Ok(ready)
}

fn verify_ready(
    store: &CasStore,
    ready: &ReadyGeneration,
    session: &SessionAuthority,
    compilation: &str,
    deep: bool,
) -> Result<(), ClewError> {
    if ready.runtime_key != session.runtime_key
        || ready.base_revision != session.base_revision
        || ready.compilation != compilation
        || !session
            .compilations
            .iter()
            .any(|value| value == compilation)
    {
        return Err(corrupt("ready generation session authority is invalid"));
    }
    verify_ready_authority(store, ready, deep)
}

fn verify_ready_authority(
    store: &CasStore,
    ready: &ReadyGeneration,
    deep: bool,
) -> Result<(), ClewError> {
    if ready.schema != READY_GENERATION_SCHEMA
        || ready.repository_snapshot.object_schema != SNAPSHOT_SCHEMA
        || ready.generation.object_schema != GENERATION_SCHEMA
        || ready.query_index.object_schema != QUERY_INDEX_SCHEMA
        || ready.incremental.schema != INCREMENTAL_EVIDENCE_SCHEMA
        || ready.incremental_receipt.object_schema != INCREMENTAL_RECEIPT_SCHEMA
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
            )?
    {
        return Err(corrupt("ready generation authority is invalid"));
    }
    ready.completeness.validate()?;
    match ready.incremental.executed {
        IncrementalExecutionMode::Full
            if ready.incremental.worker_requests.open_project_requests != 1
                || ready.incremental.worker_requests.index_files_requests == 0 =>
        {
            return Err(corrupt("full generation request counters are invalid"));
        }
        IncrementalExecutionMode::UnchangedHit
            if !matches!(
                ready.incremental.planned,
                IncrementalPlan::UnchangedHit { .. }
            ) || ready.incremental.worker_requests.open_project_requests != 1
                || ready.incremental.worker_requests.index_files_requests != 0 =>
        {
            return Err(corrupt("unchanged generation request counters are invalid"));
        }
        _ => {}
    }
    let _ = load_compilation_snapshot(store, ready)?;
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
    let receipt: IncrementalReceipt = read_canonical_object(store, &ready.incremental_receipt)?;
    receipt.validate()?;
    if receipt.generation_id != generation.generation_id
        || receipt.completeness != ready.completeness
    {
        return Err(corrupt(
            "incremental receipt is bound to another generation",
        ));
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

fn git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

struct GenerationLock {
    _file: File,
}

impl GenerationLock {
    fn acquire(state: &StateAuthority, key: &str) -> Result<Self, ClewError> {
        let name = format!("generation-{}.lock", digest_component(key)?);
        let file = state
            .directory(Path::new("locks"))?
            .open_lock(OsStr::new(&name))?;
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
        ADAPTER_PROTOCOL, AdapterHandshake, FactShard, LanguageAdapter, ToolchainConstraint,
    };
    use crate::derived_manifest::DERIVED_MANIFEST_SCHEMA;
    use crate::generation_v2::GenerationKind;
    use crate::incremental_v2::COMPILER_STORE_KEY_SCHEMA;
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
    fn generation_admission_is_bounded_by_cpu_memory_compilations_and_global_cap() {
        let abundant = HostResources {
            logical_cpu: 64,
            total_memory_bytes: 128 * 1024 * 1024 * 1024,
            codeclew_memory_budget_bytes: 96 * 1024 * 1024 * 1024,
        };
        assert_eq!(admitted_generation_jobs(abundant, 64), 16);
        assert_eq!(admitted_generation_jobs(abundant, 3), 3);

        let cpu_bound = HostResources {
            logical_cpu: 4,
            ..abundant
        };
        assert_eq!(admitted_generation_jobs(cpu_bound, 12), 4);

        let memory_bound = HostResources {
            codeclew_memory_budget_bytes: 5 * 1024 * 1024 * 1024,
            ..abundant
        };
        assert_eq!(admitted_generation_jobs(memory_bound, 12), 2);

        let constrained = HostResources {
            logical_cpu: 1,
            total_memory_bytes: 512 * 1024 * 1024,
            codeclew_memory_budget_bytes: 0,
        };
        assert_eq!(admitted_generation_jobs(constrained, 12), 1);
    }

    #[test]
    fn ready_generation_set_refuses_a_forged_completeness_upgrade() {
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        let object = |schema: &str, character: char| CasObject {
            schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
            object_schema: schema.into(),
            digest: digest(character),
            size: 1,
        };
        let partial = CompletenessVector {
            schema: COMPLETENESS_VECTOR_SCHEMA.into(),
            support: Support::Supported,
            coverage: Coverage::Partial {
                observed_scopes: vec![digest('1')],
                boundaries: vec!["VERIFY_BOUNDARY".into()],
            },
            certainty: Certainty::Unsure {
                check_set: vec!["VERIFY_BOUNDARY".into()],
            },
            obligations: vec![VerificationObligation {
                code: "VERIFY_BOUNDARY".into(),
                subject: vec![digest('1')],
                publication_blocking: true,
            }],
        };
        partial.validate().unwrap();
        let snapshot = object(SNAPSHOT_SCHEMA, '2');
        let component = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key: digest('3'),
            runtime_key: digest('4'),
            base_revision: "1".repeat(40),
            compilation: ":/main".into(),
            compiler_version: "2.4.10".into(),
            completeness: partial.clone(),
            coverage: coverage_label(&partial).into(),
            certainty: certainty_label(&partial).into(),
            obligations: obligation_codes(&partial),
            incremental: IncrementalExecutionEvidence {
                schema: INCREMENTAL_EVIDENCE_SCHEMA.into(),
                planned: IncrementalPlan::Full {
                    reason: FullAnalysisReason::NoParent,
                },
                executed: IncrementalExecutionMode::Full,
                subset_analysis_supported: false,
                worker_requests: WorkerRequestCounters {
                    open_project_requests: 1,
                    index_files_requests: 1,
                },
            },
            incremental_receipt: object(INCREMENTAL_RECEIPT_SCHEMA, '5'),
            repository_snapshot: snapshot.clone(),
            derived_input_manifest: object(DERIVED_MANIFEST_SCHEMA, '6'),
            generation: object(GENERATION_SCHEMA, '7'),
            query_index: object(QUERY_INDEX_SCHEMA, '8'),
        };
        let compilations = vec![component];
        let forged = CompletenessVector::verified_complete(digest('9')).unwrap();
        let ready = ReadyGenerationSet {
            schema: READY_GENERATION_SET_SCHEMA.into(),
            generation_key: ready_set_key(&digest('4'), &"1".repeat(40), &snapshot, &compilations)
                .unwrap(),
            runtime_key: digest('4'),
            base_revision: "1".repeat(40),
            repository_snapshot: snapshot,
            compilations,
            coverage: coverage_label(&forged).into(),
            certainty: certainty_label(&forged).into(),
            obligations: obligation_codes(&forged),
            completeness: forged,
        };
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        assert_eq!(
            verify_ready_set_authority(&store, &ready, false)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
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
    fn final_generation_key_is_fixed_before_output_completeness() {
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
        let first = final_generation_key(
            &format!("sha256:{}", "4".repeat(64)),
            &format!("sha256:{}", "5".repeat(64)),
            &snapshot,
            ":/main",
            &derived,
        )
        .unwrap();
        let changed_completeness = CompletenessVector {
            schema: COMPLETENESS_VECTOR_SCHEMA.into(),
            support: Support::Supported,
            coverage: Coverage::Unknown,
            certainty: Certainty::Unsure {
                check_set: vec!["verify".into()],
            },
            obligations: vec![VerificationObligation {
                code: "VERIFY".into(),
                subject: vec!["scope".into()],
                publication_blocking: true,
            }],
        };
        changed_completeness.validate().unwrap();
        let second = final_generation_key(
            &format!("sha256:{}", "4".repeat(64)),
            &format!("sha256:{}", "5".repeat(64)),
            &snapshot,
            ":/main",
            &derived,
        )
        .unwrap();
        assert_eq!(first, second);

        let mut changed_derived = derived;
        changed_derived.digest = format!("sha256:{}", "6".repeat(64));
        assert_ne!(
            first,
            final_generation_key(
                &format!("sha256:{}", "4".repeat(64)),
                &format!("sha256:{}", "5".repeat(64)),
                &snapshot,
                ":/main",
                &changed_derived,
            )
            .unwrap()
        );
    }

    #[test]
    fn delta_plan_executes_full_until_subset_protocol_exists() {
        let planned = IncrementalPlan::Delta {
            parent_generation_id: format!("sha256:{}", "1".repeat(64)),
            changed_files: vec!["src/main/kotlin/Sample.kt".into()],
            invalidated_files: vec![
                "src/main/kotlin/Sample.kt".into(),
                "src/test/kotlin/SampleTest.kt".into(),
            ],
        };
        let requests = WorkerRequestCounters {
            open_project_requests: 1,
            index_files_requests: 1,
        };

        let evidence = full_execution_evidence(planned.clone(), requests);

        assert_eq!(evidence.planned, planned);
        assert_eq!(evidence.executed, IncrementalExecutionMode::Full);
        assert!(!evidence.subset_analysis_supported);
        assert_eq!(evidence.worker_requests, requests);
    }

    #[test]
    fn workspace_profile_is_private_canonical_and_rejects_impossible_counts() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let path = state.root().join("sessions/test/workspace-profile.json");
        write_generation_workspace_evidence(
            &state,
            &path,
            12,
            ProjectNativeKotlinWorkspaceProfile {
                materializations: 1,
                derived_mount_sets: 1,
                open_project_calls: 1,
            },
        )
        .unwrap();
        let value: GenerationWorkspaceEvidence =
            serde_json::from_slice(&state.read_private_file(&path, MAX_BINDING_BYTES).unwrap())
                .unwrap();
        assert_eq!(value.schema, WORKSPACE_PROFILE_SCHEMA);
        assert_eq!(value.compilation_count, 12);
        assert_eq!(value.open_project_calls, 1);

        let error = write_generation_workspace_evidence(
            &state,
            &path,
            12,
            ProjectNativeKotlinWorkspaceProfile {
                materializations: 2,
                derived_mount_sets: 1,
                open_project_calls: 13,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    #[test]
    fn corrupt_incremental_head_forces_invalid_receipt_full_plan() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let head_path = state
            .root()
            .join("repos/test-repository/generations/incremental.json");
        state
            .write_private_atomic(&head_path, b"{not-json\n")
            .unwrap();

        let head = load_incremental_head_for_planning(&state, &store, &head_path).unwrap();
        assert!(matches!(head, IncrementalHeadState::Corrupt));
        let (plan, exact) = head.forced_full_plan().expect("corrupt head fallback");
        assert_eq!(
            plan,
            IncrementalPlan::Full {
                reason: FullAnalysisReason::InvalidReceipt,
            },
        );
        assert!(!exact);
    }

    #[test]
    fn compiler_index_receipt_is_per_file_cross_boundary_and_corruption_refuses() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        let object = |schema: &str, character: char| CasObject {
            schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
            object_schema: schema.into(),
            digest: digest(character),
            size: 1,
        };
        let compiler_store = CompilerStoreKey {
            schema: COMPILER_STORE_KEY_SCHEMA.into(),
            key: digest('a'),
            adapter_id: "kotlin-2.4".into(),
            adapter_digest: digest('b'),
            language_uri: KOTLIN_LANGUAGE.into(),
            toolchain: object("test/toolchain/1", 'c'),
            canonical_options: object("test/options/1", 'd'),
            classpath: vec![],
            plugins: vec![],
        };
        let generation = GenerationManifest {
            schema: GENERATION_SCHEMA.into(),
            generation_id: digest('e'),
            derived_input_manifest: object(DERIVED_MANIFEST_SCHEMA, 'f'),
            parent_generation: None,
            generation_kind: GenerationKind::Full,
            attempts: vec![],
            shards: vec![],
            fact_count: 0,
        };
        let index = json!({
            "files":[
                {"path":"src/A.kt","contentHash":digest('1')},
                {"path":"src/B.kt","contentHash":digest('2')}
            ],
            "declarationDescriptors":{"descriptors":[
                {"file":"src/A.kt","symbolIdentity":"callable:p/A.call#jvm:()V","compilerCallableId":"p/A.call"},
                {"file":"src/B.kt","symbolIdentity":"callable:p/B.read#jvm:()I","compilerCallableId":"p/B.read"}
            ]},
            "declarationRelations":{"relations":[
                {"file":"src/A.kt","owner":"p/A.call","target":"p/B.read","kind":"CALLS"}
            ]}
        });
        let completeness = CompletenessVector::verified_complete(digest('9')).unwrap();
        let (receipt, reference) =
            create_incremental_receipt(&store, &index, &compiler_store, &generation, completeness)
                .unwrap();
        assert_eq!(receipt.files[0].dependencies, vec!["src/B.kt"]);
        assert_eq!(receipt.boundaries.len(), 1);
        assert_eq!(receipt.boundaries[0].source_path, "src/A.kt");
        assert_eq!(receipt.boundaries[0].target_path, "src/B.kt");

        let hex = reference.digest.strip_prefix("sha256:").unwrap();
        let path = state.objects_root().join(&hex[..2]).join(&hex[2..]);
        std::fs::write(path, b"corrupt").unwrap();
        assert_eq!(
            read_canonical_object::<IncrementalReceipt>(&store, &reference)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
    }
}
