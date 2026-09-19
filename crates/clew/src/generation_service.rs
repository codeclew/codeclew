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
use crate::java_adapter_v2::{
    JAVA_COMPILER_FACTS_CAPABILITY, JAVA_LANGUAGE, JavaAdapterV2, JavaCompilerFact,
    JavaCompilerIndex, build_java_compiler_index, execute_prepared_java_output,
    java_adapter_digest, java_scope_digest, project_java_compiler_output,
};
use crate::java_analysis_inputs::{
    JavaAnalysisInputPool, JavaPreparedInputsResult, JavaPreparedRefusal,
    PreparedJavaAnalysisInputs,
};
use crate::java_project_model::{
    JAVA_MODEL_SCHEMA, JavaOperationalModel, extract_java_models_with_settings_and_diagnostics,
};
use crate::kotlin_adapter_v2::{
    KOTLIN_FACTS_CAPABILITY, KOTLIN_LANGUAGE, KotlinAdapterV2, KotlinGenerationDriver,
    ProjectNativeKotlinAttempt, ProjectNativeKotlinOperationalTiming, ProjectNativeKotlinWorkspace,
    ProjectNativeKotlinWorkspaceProfile, kotlin_adapter_digest, semantic_scope_digest,
};
use crate::kotlin_engine::{
    KOTLIN_ADAPTER_CONTRACT_ID, KotlinEngineCapabilities, KotlinProjectSemantics,
    KotlinSemanticEngine,
};
use crate::maven_diagnostics::DebugOutput;
use crate::python_adapter_v2::{
    MAX_SOURCE_FILE_BYTES as MAX_PYTHON_SOURCE_FILE_BYTES,
    MAX_SOURCE_FILES as MAX_PYTHON_SOURCE_FILES,
    MAX_TOTAL_SOURCE_BYTES as MAX_PYTHON_TOTAL_SOURCE_BYTES, PYTHON_LANGUAGE,
    PYTHON_SYNTAX_FACTS_CAPABILITY, PythonAdapterV2, PythonSyntaxAuthority,
    build_syntax_index as build_python_syntax_index, python_adapter_digest, python_scope_digest,
};
use crate::python_project_model::{
    PYTHON_GRAMMAR_AUTHORITY, PYTHON_MODEL_SCHEMA, PythonCompilationSelector, PythonProjectModel,
};
use crate::query_v2::{
    QUERY_INDEX_SCHEMA, QueryIndexManifest, build_query_index, verify_index, verify_index_manifest,
};
use crate::repository_snapshot::{
    RepositoryInputSnapshot, SNAPSHOT_SCHEMA, TrackedScopeLimits, WorktreeKind, capture,
    capture_commit_scope,
};
use crate::runtime::RuntimeAuthority;
use crate::rust_adapter_v2::{
    RUST_LANGUAGE, RUST_SYNTAX_FACTS_CAPABILITY, RustAdapterV2, RustSyntaxAuthority,
    build_syntax_index, rust_adapter_digest, rust_scope_digest,
};
use crate::rust_project_model::{CargoProjectModel, CargoTargetModel, extract_cargo_model};
use crate::session::{ModelCachePolicy, SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use crate::typescript_adapter_v2::{
    JAVASCRIPT_COMPILER_FACTS_CAPABILITY, JAVASCRIPT_LANGUAGE,
    TYPESCRIPT_COMPILER_FACTS_CAPABILITY, TYPESCRIPT_LANGUAGE, TypeScriptAdapterV2,
    TypeScriptCompilerFact, TypeScriptCompilerIndex, build_typescript_compiler_index,
    typescript_adapter_digest, typescript_scope_digest,
};
use crate::typescript_project_model::{
    JAVASCRIPT_MODEL_SCHEMA, TYPESCRIPT_MODEL_SCHEMA, TypeScriptOperationalModel,
    extract_javascript_model, extract_typescript_model,
};
use crate::worker::WorkerRequestCounters;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::fd::AsRawFd;

pub const READY_GENERATION_SCHEMA: &str = "codeclew-ready-generation/2.0";
pub const READY_GENERATION_SET_SCHEMA: &str = "codeclew-ready-generation-set/1.0";
/// Immutable persisted authority for the exact source bytes a Java generation
/// was indexed against. Present only for the writable-then-seal profile, where
/// the build may rewrite source in place and the original repository snapshot
/// no longer matches the coordinates emitted by the analyzer.
pub const TRANSFORMED_SOURCE_SCHEMA: &str = "codeclew-transformed-source/1.0";
/// Schema for a single persisted transformed source file object inside a
/// transformed-source manifest.
pub const TRANSFORMED_SOURCE_FILE_SCHEMA: &str = "codeclew-transformed-source-file/1.0";
/// Authority label for source text read from the persisted transformed bytes.
pub const TRANSFORMED_SOURCE_AUTHORITY: &str = "TRANSFORMED_SOURCE";
pub(crate) const JAVA_MAVEN_WRITABLE_THEN_SEAL_PROFILE: &str =
    "java-17plus-maven-writable-then-seal";

/// Whether the given profile opts into writable-then-seal Maven materialization.
pub(crate) fn wants_writable_then_seal(profile: &str) -> bool {
    profile == JAVA_MAVEN_WRITABLE_THEN_SEAL_PROFILE
}
const PREPARED_AUTHORITY_SCHEMA: &str = "codeclew-prepared-generation-authority/3.0";
const MODEL_ANALYSIS_SCHEMA: &str = "codeclew-project-native-analysis/2.0";
const INCREMENTAL_HEAD_SCHEMA: &str = "codeclew-incremental-head/2.0";
const INCREMENTAL_EVIDENCE_SCHEMA: &str = "codeclew-incremental-execution/3.0";
const WORKSPACE_PROFILE_SCHEMA: &str = "codeclew-project-native-workspace-profile/3.0";
const MAX_BINDING_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GenerationWorkspaceEvidence {
    schema: String,
    base_revision: String,
    compilation_count: usize,
    materializations: u64,
    derived_mount_sets: u64,
    repository_snapshot: CasObject,
    runtime_key: String,
    session_authority_digest: String,
    workspace_set_authority_digest: String,
    workspace_set_authorizations: u64,
    authorized_compilation_count: u64,
    legacy_open_project_calls: u64,
    /// Additive operational timing only; absent in older private evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operational_timing: Option<ProjectNativeKotlinOperationalTiming>,
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
    /// Immutable transformed source bytes this generation was indexed against
    /// (writable-then-seal only). Absent for read-only generations and for all
    /// non-Java language authorities. When present, documentation must read
    /// source from these bytes, never slice the original repository snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transformed_source: Option<CasObject>,
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
    /// Shared immutable transformed source bytes for the set (writable-then-seal
    /// only). Mirrors each compilation's `transformed_source` and is used by
    /// documentation to read the exact bytes a generation was indexed against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transformed_source: Option<CasObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedGenerationAuthority {
    schema: String,
    runtime_key: String,
    repository_snapshot: CasObject,
    compilation: String,
    project_semantics: KotlinProjectSemantics,
    semantic_engine: KotlinEngineCapabilities,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisExecutionAuthority {
    CompilerWorker,
    CompilerProcess,
    CompilerOutputCheckpoint,
    InProcessSyntax,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncrementalExecutionEvidence {
    pub schema: String,
    pub planned: IncrementalPlan,
    pub executed: IncrementalExecutionMode,
    pub analysis_execution_authority: AnalysisExecutionAuthority,
    pub subset_analysis_supported: bool,
    pub worker_requests: WorkerRequestCounters,
    /// Original successful subprocess evidence; current projection may reuse it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_output_receipt: Option<CasObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IncrementalHead {
    schema: String,
    compiler_store_key: String,
    receipt: CasObject,
    ready: CasObject,
}

const JAVA_ANALYSIS_CHECKPOINT_SCHEMA: &str = "codeclew-java-analysis-checkpoint/2.0";
const JAVA_ANALYSIS_REQUEST_SCHEMA: &str = "codeclew-java-analysis-request/1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JavaAnalysisCheckpoint {
    schema: String,
    input: CasObject,
    ready: CasObject,
}

/// A request observation is separate from the immutable production receipt.
/// Ready reuse preserves prior production evidence; a newly projected raw hit
/// separately records its original subprocess receipt and zero current starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JavaAnalysisRequest {
    schema: String,
    compilation: String,
    eligibility: String,
    lookup: String,
    java_analyzer_starts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compiler_output_receipt: Option<CasObject>,
    input: CasObject,
    generation: CasObject,
}

struct LoadedIncrementalHead {
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
    // Derive the writable-then-seal gate from durable session authority: the
    // committed-context profile field or the working-tree binding profile.
    // This keeps generation admission consistent across context-open and
    // working-tree entrypoints rather than a documentation-only boolean.
    let profile = session.profile.as_deref().or_else(|| {
        session
            .working_tree
            .as_ref()
            .map(|binding| binding.profile_id.as_str())
    });
    let writable_then_seal = profile.map(wants_writable_then_seal).unwrap_or(false);
    ensure_session_generation_with_diagnostics(session, None, writable_then_seal, &[])
}

pub(crate) fn ensure_session_generation_with_diagnostics(
    session: &SessionAuthority,
    debug_output: Option<&DebugOutput>,
    writable_then_seal: bool,
    extra_annotation_processors: &[String],
) -> Result<ReadyGenerationSet, ClewError> {
    let state = StateAuthority::process_default()?;
    let session_root = state.session_root(&session.session_id)?;
    let binding_path = session_root.join("generation.json");
    let store = CasStore::open(&state)?;
    if state.private_file_exists(&binding_path)? {
        return load_ready_set(&state, &store, &binding_path, session, false);
    }
    if session.language == SessionLanguage::Python {
        let (snapshot, snapshot_object) = session.python_source_snapshot(&store)?;
        let compilation_root = session_root.join("compilations");
        state.directory_at(&compilation_root)?;
        return ensure_python_generation_set(
            session,
            &state,
            &store,
            &snapshot,
            snapshot_object,
            &compilation_root,
            &binding_path,
            "",
        );
    }
    if matches!(
        session.language,
        SessionLanguage::JavaScript | SessionLanguage::TypeScript
    ) {
        if session.freshness()?.status != "FRESH" {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                "ECMAScript target is no longer fresh for this session",
            ));
        }
        let target_repo = session.target_repository_path()?;
        let (snapshot, snapshot_object) = capture(&target_repo, &store)?;
        let compilation_root = session_root.join("compilations");
        state.directory_at(&compilation_root)?;
        return ensure_typescript_generation_set(
            session,
            &state,
            &store,
            &target_repo,
            &snapshot,
            snapshot_object,
            &compilation_root,
            &binding_path,
            "",
        );
    }
    let repo = session.repository_path()?;
    let (snapshot, snapshot_object) = if let Some(binding) = &session.working_tree {
        (
            session.working_tree_snapshot(&store)?,
            binding.snapshot.clone(),
        )
    } else {
        capture(&repo, &store)?
    };
    let compilation_root = session_root.join("compilations");
    state.directory_at(&compilation_root)?;
    match session.language {
        SessionLanguage::Java => {
            return ensure_java_generation_set(
                session,
                &state,
                &store,
                &snapshot,
                snapshot_object,
                &compilation_root,
                &binding_path,
                "",
                debug_output,
                writable_then_seal,
                extra_annotation_processors,
            );
        }
        SessionLanguage::JavaScript => {
            unreachable!("JavaScript generation returned above")
        }
        SessionLanguage::Rust => {
            return ensure_rust_generation_set(
                session,
                &state,
                &store,
                &repo,
                &snapshot,
                snapshot_object,
                &compilation_root,
                &binding_path,
                session.working_tree.is_none(),
                "",
            );
        }
        SessionLanguage::TypeScript => unreachable!("TypeScript generation returned above"),
        SessionLanguage::Python => unreachable!("Python generation returned above"),
        SessionLanguage::Kotlin => {}
    }
    let pool = generation_pool(session)?;
    let workspace =
        ProjectNativeKotlinWorkspace::prepare(&state, &store, &snapshot, &session.compilations)?;
    // The exact selected set is authorized once. The private bridge retains
    // legacy per-compilation OpenProject calls until the post-G1 worker
    // protocol cutover; no caller can bypass or widen the set authority.
    let lane = GenerationLaneContext {
        session,
        repo: &repo,
        publish_head: session.working_tree.is_none(),
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
        GenerationWorkspaceAuthority {
            base_revision: &session.base_revision,
            runtime_key: &session.runtime_key,
            session_authority_digest: &session.authority_digest,
            repository_snapshot: &snapshot_object,
        },
    )?;
    let ready = assemble_ready_set(session, snapshot_object, results)?;
    write_ready_set(&state, &binding_path, &ready)?;
    Ok(ready)
}

#[allow(clippy::too_many_arguments)]
fn ensure_rust_generation_set(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    repo: &Path,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: CasObject,
    compilation_root: &Path,
    binding_path: &Path,
    publish_head: bool,
    binding_prefix: &str,
) -> Result<ReadyGenerationSet, ClewError> {
    if publish_head
        && let Some(ready) = bind_exact_rust_incremental_heads(
            session,
            state,
            store,
            repo,
            &snapshot_object,
            compilation_root,
            binding_path,
            binding_prefix,
        )?
    {
        return Ok(ready);
    }
    let model = extract_cargo_model(repo, &session.compilations)?;
    let snapshot_matches = if session.working_tree.is_some() {
        crate::repository_snapshot::verify_materialized_working_tree(snapshot, store, repo)?;
        true
    } else {
        capture(repo, store)?.1 == snapshot_object
    };
    if !snapshot_matches {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "Cargo model extraction changed the sealed repository input",
        ));
    }
    let model_object = store.put(
        crate::rust_project_model::CARGO_MODEL_SCHEMA,
        &canonical::bytes(&model).map_err(internal)?,
    )?;
    let mut results = Vec::with_capacity(session.compilations.len());
    for compilation in &session.compilations {
        let component = digest_component(
            &canonical::hash(&json!({
                "schema":"codeclew-session-rust-compilation-binding/1.0",
                "compilation":compilation,
            }))
            .map_err(internal)?,
        )?
        .to_owned();
        let target = model
            .targets
            .iter()
            .find(|target| target.selector.canonical() == *compilation)
            .ok_or_else(|| corrupt("Cargo model misses a selected Rust target"))?;
        results.push(ensure_rust_generation(
            session,
            state,
            store,
            snapshot,
            &snapshot_object,
            &model,
            &model_object,
            target,
            compilation,
            &compilation_root.join(format!("{binding_prefix}{component}.json")),
            publish_head,
        )?);
    }
    let ready = assemble_ready_set(session, snapshot_object, results)?;
    write_ready_set(state, binding_path, &ready)?;
    Ok(ready)
}

#[allow(clippy::too_many_arguments)]
fn bind_exact_rust_incremental_heads(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    repo: &Path,
    snapshot: &CasObject,
    compilation_root: &Path,
    binding_path: &Path,
    binding_prefix: &str,
) -> Result<Option<ReadyGenerationSet>, ClewError> {
    let repository = if session.working_tree.is_some() {
        state.repository_by_key(&session.repository_key)?
    } else {
        state.repository(repo)?
    };
    if repository.key != session.repository_key {
        return Err(corrupt(
            "Rust generation repository differs from session Git authority",
        ));
    }
    let mut compilations = Vec::with_capacity(session.compilations.len());
    for compilation in &session.compilations {
        let head_path = incremental_head_path(&repository.root, compilation)?;
        let head = load_incremental_head_for_planning(state, store, &head_path)?;
        let Some(head) = head.ready() else {
            return Ok(None);
        };
        if !rust_head_matches_session(&head.ready, session, snapshot, compilation) {
            return Ok(None);
        }
        verify_ready(store, &head.ready, session, compilation, true)?;
        compilations.push(head.ready.clone());
    }

    let ready = assemble_ready_set(session, snapshot.clone(), compilations.clone())?;
    for (compilation, ready) in session.compilations.iter().zip(compilations) {
        let component = digest_component(
            &canonical::hash(&json!({
                "schema":"codeclew-session-rust-compilation-binding/1.0",
                "compilation":compilation,
            }))
            .map_err(internal)?,
        )?
        .to_owned();
        write_private_atomic(
            state,
            &compilation_root.join(format!("{binding_prefix}{component}.json")),
            &ready,
        )?;
    }
    write_ready_set(state, binding_path, &ready)?;
    Ok(Some(ready))
}

fn rust_head_matches_session(
    ready: &ReadyGeneration,
    session: &SessionAuthority,
    snapshot: &CasObject,
    compilation: &str,
) -> bool {
    ready.runtime_key == session.runtime_key
        && ready.base_revision == session.base_revision
        && ready.repository_snapshot == *snapshot
        && ready.compilation == compilation
}

#[allow(clippy::too_many_arguments)]
/// Copy a directory tree, preserving files, directories and symlinks, without
/// following symlinks (so derived mounts are never pulled in).
fn copy_tree(src: &std::path::Path, dst: &std::path::Path) -> Result<(), ClewError> {
    // The classes directory lives under the derived-state `target` mount (a
    // symlink), so resolve it to the real build output before walking.
    let src = src.canonicalize().map_err(io_error)?;
    std::fs::create_dir_all(dst).map_err(io_error)?;
    for entry in walkdir::WalkDir::new(&src) {
        let entry = entry.map_err(|error| internal(error.to_string()))?;
        let relative = entry
            .path()
            .strip_prefix(&src)
            .map_err(|_| internal("build class path escapes its root"))?;
        let target = dst.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(io_error)?;
        } else if entry.file_type().is_symlink() {
            #[cfg(unix)]
            {
                let link = std::fs::read_link(entry.path()).map_err(io_error)?;
                std::os::unix::fs::symlink(link, &target).map_err(io_error)?;
            }
            #[cfg(not(unix))]
            {
                let _ = entry;
                let _ = &target;
            }
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &target).map_err(io_error)?;
        }
    }
    Ok(())
}

/// Preserve a compilation's compiled output (`target/classes` and, for test
/// compilations, `target/test-classes`) into a stable directory under the
/// attempt root. The derived-state mounts (`target`/`build`) are unmounted
/// before analysis, so a classpath entry pointing at `.../target/classes`
/// becomes dangling and the analyzer cannot resolve build-generated types
/// (jaxws, annotation processors, MapStruct impls). Copying the compiled
/// output before the unmount lets the analyzer reach it in Phase 2.
fn preserve_compiled_classes(
    attempt_root: &std::path::Path,
    model: &mut JavaOperationalModel,
) -> Result<(), ClewError> {
    if model.classpath_paths.len() != model.authority.classpath.len() {
        return Err(internal(
            "Java classpath paths differ from admitted entries",
        ));
    }
    let mut next = Vec::with_capacity(model.classpath_paths.len());
    for (path, admitted) in model.classpath_paths.iter().zip(&model.authority.classpath) {
        let is_classes = path.ends_with("target/classes") || path.ends_with("target/test-classes");
        if is_classes {
            if crate::java_project_model::classpath_authority(path)? != *admitted {
                return Err(ClewError::new(
                    ErrorCode::InputMutated,
                    "compiled Java output changed after model admission",
                ));
            }
            // All compilation models share this attempt. A per-model numeric
            // slot can overwrite another module's classes. Content identity
            // keeps distinct outputs separate and shares genuinely equal ones.
            let dest = attempt_root
                .join("analysis-classes")
                .join(digest_component(&admitted.digest)?);
            if !dest.exists() {
                copy_tree(path, &dest)?;
            }
            if crate::java_project_model::classpath_authority(&dest)? != *admitted {
                return Err(ClewError::new(
                    ErrorCode::InputMutated,
                    "preserved Java output differs from admitted classpath",
                ));
            }
            next.push(dest);
        } else {
            next.push(path.clone());
        }
    }
    model.classpath_paths = next;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ensure_java_generation_set(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: CasObject,
    compilation_root: &Path,
    binding_path: &Path,
    binding_prefix: &str,
    debug_output: Option<&DebugOutput>,
    writable_then_seal: bool,
    extra_annotation_processors: &[String],
) -> Result<ReadyGenerationSet, ClewError> {
    let mut workspace = ProjectNativeKotlinWorkspace::prepare_language(
        state,
        store,
        snapshot,
        &session.compilations,
        "java",
    )?;
    if writable_then_seal {
        eprintln!(
            "CODEDEBUG ensure_java_generation_set writable_then_seal=true repo={}",
            workspace.repository().display()
        );
        crate::repository_snapshot::make_files_writable(workspace.repository())?;
        eprintln!("CODEDEBUG make_files_writable returned OK");
        workspace.set_allow_materialization_mutation(true);
    }
    let sources = effective_java_sources(snapshot)?;
    // Phase 1: extract the model and capture the (transformed) source state for
    // every compilation. Transformations run here while the tree is writable.
    // No indexing or publication happens yet.
    let settings = session.maven_settings()?;
    let models = extract_java_models_with_settings_and_diagnostics(
        workspace.repository(),
        &session.compilations,
        settings.as_ref(),
        debug_output,
        extra_annotation_processors,
    )?;
    let mut captured = Vec::with_capacity(models.len());
    for model in models {
        let compilation = model.authority.compilation.clone();
        let component = digest_component(
            &canonical::hash(&json!({
                "schema":"codeclew-session-java-compilation-binding/1.0",
                "compilation":compilation,
                "writableThenSeal":writable_then_seal,
            }))
            .map_err(internal)?,
        )?
        .to_owned();
        let (source_content_digests, before_digests, changed_files) = if writable_then_seal {
            let after = transformed_java_source_digests(
                workspace.repository(),
                &model.authority.source_files,
            )?;
            // Original authority exists only for files present in the CAS
            // snapshot. Build-added files appear only in the transformed
            // after-map; never fabricate original-snapshot provenance for them.
            let before = original_java_source_content_digests(store, &sources, &model)?;
            let changed = before
                .iter()
                .filter_map(|(path, d)| {
                    after
                        .get(path)
                        .filter(|a| *a != d)
                        .map(|a| (path.clone(), d.clone(), a.clone()))
                })
                .collect::<Vec<_>>();
            (after, Some(before), changed)
        } else {
            let digests = java_source_content_digests(store, &sources, &model)?;
            (digests, None, Vec::new())
        };
        captured.push((
            model,
            source_content_digests,
            before_digests,
            changed_files,
            compilation.clone(),
            compilation_root.join(format!("{binding_prefix}{component}.json")),
        ));
    }
    // Seal the transformed materialization read-only BEFORE indexing. The
    // derived-state mounts (build/target/.gradle symlink stubs) must be removed
    // first: sealing before unmount makes the parent directories read-only, so
    // removing the mounts would fail with Permission denied. Unmounting first
    // (without dropping the workspace) keeps the repo path alive so it can be
    // sealed, then finish() verifies/cleans up.
    if writable_then_seal {
        // Preserve each compilation's compiled output before unmounting the
        // derived-state mounts, so the Phase-2 analyzer can still resolve the
        // build-generated types (jaxws, annotation processors, MapStruct impls)
        // even though `.../target/classes` becomes a dangling path after
        // `unmount_derived_state`.
        let attempt_root = workspace
            .repository()
            .parent()
            .ok_or_else(|| internal("attempt workspace root is unavailable"))?
            .to_owned();
        for (model, _, _, _, _, _) in captured.iter_mut() {
            preserve_compiled_classes(&attempt_root, model)?;
        }
        workspace.unmount_derived_state()?;
        eprintln!("CODEDEBUG seal_tree PRE-INDEX START");
        if let Err(e) = crate::repository_snapshot::seal_tree(workspace.repository()) {
            eprintln!("CODEDEBUG seal_tree PRE-INDEX ERR {e}");
            return Err(e);
        }
        eprintln!("CODEDEBUG seal_tree PRE-INDEX OK");
    }
    // Prepare the whole selected set before sealing the shared input pool.
    // One request owns one JDK image and content-deduplicated classpath copies.
    let mut input_pool = JavaAnalysisInputPool::new(state, java_adapter_digest()?)?;
    let mut prepared_inputs = Vec::with_capacity(captured.len());
    for (model, source_digests, _, _, _, _) in &captured {
        prepared_inputs.push(if session.working_tree.is_some() {
            JavaPreparedInputsResult::Refused(JavaPreparedRefusal::LegacyInputAuthority)
        } else {
            input_pool.prepare(
                workspace.repository(),
                model,
                source_digests,
                writable_then_seal,
            )?
        });
    }
    input_pool.seal()?;
    // Phase 2: index/validate/publish every compilation against the sealed,
    // immutable source state. The analyzer only reads; emitted processor output
    // is isolated to disposable dirs, so sealing before analysis is safe.
    let mut results = Vec::with_capacity(captured.len());
    for (
        (model, source_content_digests, before_digests, changed_files, compilation, binding_path),
        preparation,
    ) in captured.into_iter().zip(prepared_inputs)
    {
        let (prepared, eligibility) = match &preparation {
            JavaPreparedInputsResult::Eligible(inputs) => (Some(inputs.as_ref()), "ELIGIBLE"),
            JavaPreparedInputsResult::Refused(reason) => (None, reason.code()),
        };
        if writable_then_seal {
            // The sealed tree must still match the captured transformed state;
            // otherwise a transform mutated sources after capture and the index
            // would be attributed to bytes it never analyzed.
            let sealed = transformed_java_source_digests(
                workspace.repository(),
                &model.authority.source_files,
            )?;
            if source_content_digests
                .iter()
                .any(|(path, expected)| sealed.get(path) != Some(expected))
            {
                return Err(ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "transformed source changed between capture and seal; refusing to publish a generation attributed to other bytes",
                ));
            }
        }
        results.push(ensure_java_generation(
            session,
            state,
            store,
            snapshot_object.clone(),
            workspace.repository(),
            model,
            source_content_digests,
            &compilation,
            &binding_path,
            writable_then_seal,
            before_digests.as_ref(),
            &changed_files,
            debug_output,
            prepared,
            eligibility,
        )?);
    }
    input_pool.close()?;
    workspace.finish()?;
    let ready = assemble_ready_set(session, snapshot_object, results)?;
    write_ready_set(state, binding_path, &ready)?;
    Ok(ready)
}

#[allow(clippy::too_many_arguments)]
fn ensure_java_generation(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    snapshot_object: CasObject,
    repository: &Path,
    model: JavaOperationalModel,
    source_content_digests: BTreeMap<String, String>,
    compilation: &str,
    binding_path: &Path,
    writable_then_seal: bool,
    before_digests: Option<&BTreeMap<String, String>>,
    changed_files: &[(String, String, String)],
    debug_output: Option<&DebugOutput>,
    prepared: Option<&PreparedJavaAnalysisInputs>,
    eligibility: &str,
) -> Result<ReadyGeneration, ClewError> {
    let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "generation service must run through ./clew",
        )
    })?;
    let model_object = store.put(
        JAVA_MODEL_SCHEMA,
        &canonical::bytes(&model.authority).map_err(internal)?,
    )?;
    let legacy_toolchain = store.put(
        "codeclew-java-toolchain-authority/1.0",
        &canonical::bytes(&json!({
            "schema":"codeclew-java-toolchain-authority/1.0",
            "compilerVersion":model.authority.compiler_version,
            "release":model.authority.release,
            "analyzer":"jdk.compiler/17+",
        }))
        .map_err(internal)?,
    )?;
    let compiler_input = prepared
        .map(|inputs| crate::java_analyzer_output::prepare_input(store, inputs, compilation))
        .transpose()?;
    let (toolchain, canonical_options) = if let Some(prepared) = prepared {
        prepared.require_sealed()?;
        let image = store.put(
            "codeclew-java-execution-image/1.0",
            &canonical::bytes(&prepared.authority.jdk).map_err(internal)?,
        )?;
        // Store the shared JDK manifest once; each scope binds a CAS reference.
        let mut policy = serde_json::to_value(&prepared.authority).map_err(internal)?;
        let fields = policy
            .as_object_mut()
            .ok_or_else(|| internal("Java analysis authority is not an object"))?;
        fields.remove("jdk");
        fields.remove("schema");
        let inputs = store.put(
            "codeclew-java-closed-analysis-authority/1.0",
            &canonical::bytes(&json!({
                "schema":"codeclew-java-closed-analysis-authority/1.0",
                "policy":policy,
                "executionImage":image,
            }))
            .map_err(internal)?,
        )?;
        let options = store.put(
            "codeclew-java-analysis-options/1.0",
            &canonical::bytes(&json!({
                "schema":"codeclew-java-analysis-options/1.0",
                "model":model_object,
                "analysisInputs":inputs,
                "compilerInput":compiler_input,
            }))
            .map_err(internal)?,
        )?;
        (image, options)
    } else {
        (legacy_toolchain, model_object.clone())
    };
    let mut classpath = model
        .authority
        .classpath
        .iter()
        .map(|authority| {
            store.put(
                "codeclew-java-classpath-authority/1.0",
                &canonical::bytes(authority).map_err(internal)?,
            )
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    classpath.sort_by(|left, right| left.digest.cmp(&right.digest));
    classpath.dedup_by(|left, right| left.digest == right.digest);
    let descriptor = CompilationDescriptor {
        schema: COMPILATION_SCHEMA.into(),
        compilation_id: safe_compilation_id(compilation),
        language_uri: LanguageUri::parse(JAVA_LANGUAGE)?,
        source_roots: vec![SourceRootDescriptor {
            logical_name: "project".into(),
            tree: snapshot_object.clone(),
        }],
        generated_source_roots: Vec::new(),
        classpath,
        toolchain,
        plugins: Vec::new(),
        canonical_options,
        dependency_compilation_ids: Vec::new(),
        operations: Vec::new(),
        origin: DescriptorOrigin::ProjectNative,
        completeness: DescriptorCompleteness::Complete,
    };
    let provider = ProviderModel {
        handshake: ProviderHandshake {
            protocol: PROVIDER_PROTOCOL.into(),
            provider_id: "project-native-java".into(),
            provider_digest: model.authority.model_digest.clone(),
            build_system_uris: vec![match model.authority.build_system {
                crate::java_project_model::JavaBuildSystem::Gradle => "build:gradle".into(),
                crate::java_project_model::JavaBuildSystem::Maven => "build:maven".into(),
            }],
        },
        build_model: BuildModel {
            provider_id: "project-native-java".into(),
            model: model_object,
            compilations: vec![descriptor.clone()],
        },
    };
    let (_, derived_input_manifest) =
        DerivedAnalysisInputManifest::create(store, snapshot_object.clone(), vec![provider])?;
    let generation_key = final_generation_key(
        &runtime.runtime_key,
        &session.base_revision,
        &snapshot_object,
        compilation,
        &derived_input_manifest,
        writable_then_seal,
    )?;
    let _lock = GenerationLock::acquire(state, &generation_key)?;
    let adapter_digest = java_adapter_digest()?;
    let compiler_store =
        CompilerStoreKey::create("java-compiler-1", adapter_digest.clone(), &descriptor)?;
    // A partial session binding is only a publication side effect. Fresh native
    // modeling may change external classpath inputs while snapshot/revision stay
    // equal. Recover only through the current complete-input checkpoint key.
    let checkpoint_path = if prepared.is_some() {
        let path = java_analysis_checkpoint_path(state, session, &generation_key)?;
        if let Some(ready) = load_java_analysis_checkpoint(
            state,
            store,
            &path,
            session,
            compilation,
            &generation_key,
            &derived_input_manifest,
            &compiler_store,
        )? {
            write_java_analysis_request(state, binding_path, &ready, eligibility, true)?;
            write_private_atomic(state, binding_path, &ready)?;
            return Ok(ready);
        }
        Some(path)
    } else {
        None
    };
    let mut journal = AttemptJournal::create(state.clone(), &generation_key, 0)?;
    journal.transition(AttemptState::Snapshotted, snapshot_object.digest.clone())?;
    journal.transition(AttemptState::Modeled, derived_input_manifest.digest.clone())?;
    journal.transition(AttemptState::Analyzing, "Java compiler facts requested")?;
    let mut raw_hit = false;
    let mut compiler_output_receipt = None;
    let indexed = if let Some(prepared) = prepared {
        let repository_state = state.repository_by_key(&session.repository_key)?;
        let input = compiler_input
            .as_ref()
            .ok_or_else(|| corrupt("prepared Java compiler input is missing"))?;
        crate::java_analyzer_output::get_or_execute(
            state,
            store,
            &repository_state.root,
            input,
            || execute_prepared_java_output(&model, prepared, debug_output),
        )
        .and_then(|output| {
            raw_hit = output.hit;
            compiler_output_receipt = Some(output.success_receipt);
            project_java_compiler_output(
                &output.bytes,
                &model,
                &source_content_digests,
                writable_then_seal,
                before_digests,
                changed_files,
            )
        })
    } else {
        build_java_compiler_index(
            repository,
            &model,
            &source_content_digests,
            writable_then_seal,
            before_digests,
            changed_files,
            debug_output,
        )
    };
    let index = match indexed {
        Ok(index) => index,
        Err(error) => {
            journal.transition(AttemptState::Failed, "Java compiler analyzer failed")?;
            return Err(error);
        }
    };
    let adapter = JavaAdapterV2::new(
        adapter_digest,
        descriptor.toolchain.digest.clone(),
        descriptor.compilation_id.clone(),
        store.clone(),
        index.clone(),
    )?;
    let mut registry = AdapterRegistry::default();
    registry.register_adapter(Arc::new(adapter))?;
    let request = AnalyzeGenerationRequest {
        schema: ANALYSIS_REQUEST_SCHEMA.into(),
        attempt_id: journal.attempt().attempt_id.clone(),
        generation_key: generation_key.clone(),
        capability: CapabilityUri::parse(JAVA_COMPILER_FACTS_CAPABILITY)?,
        compilation: descriptor,
        derived_input_manifest: derived_input_manifest.clone(),
        parent_generation: None,
    };
    let analysis = match HostResources::detect().and_then(|resources| {
        execute_analysis_dag_with_jobs(state, Arc::new(registry), request, resources, 1)
    }) {
        Ok(analysis) => analysis,
        Err(error) => {
            journal.transition(AttemptState::Failed, "Java compiler adapter DAG failed")?;
            return Err(error);
        }
    };
    journal.transition(AttemptState::Finalizing, "Java deterministic merge started")?;
    let result = (|| {
        let (generation, generation_object) = finalize_generation(
            store,
            derived_input_manifest.clone(),
            vec![AttemptAuthority {
                compilation_id: safe_compilation_id(compilation),
                capability: CapabilityUri::parse(JAVA_COMPILER_FACTS_CAPABILITY)?,
                completion: analysis.completion,
            }],
            analysis.runs,
        )?;
        let (_, query_index) = build_query_index(store, &generation, generation_object.clone())?;
        let scope_digest = java_scope_digest(&index)?;
        let completeness = crate::java_adapter_v2::java_completeness(&index, &scope_digest)?;
        let incremental_receipt = java_incremental_receipt(
            store,
            &index,
            &source_content_digests,
            &compiler_store,
            &generation,
            completeness.clone(),
        )?;
        // Persist the exact transformed source bytes (writable-then-seal only)
        // so documentation never slices the original snapshot with transformed
        // coordinates. None for read-only generations and all non-Java paths.
        let transformed_source = persist_transformed_source(
            store,
            repository,
            &model.authority.source_files,
            index.provenance.as_deref(),
            index.source_state.as_ref(),
        )?;
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key,
            runtime_key: runtime.runtime_key.clone(),
            base_revision: session.base_revision.clone(),
            compilation: compilation.into(),
            compiler_version: model.authority.compiler_version.clone(),
            completeness: completeness.clone(),
            coverage: coverage_label(&completeness).into(),
            certainty: certainty_label(&completeness).into(),
            obligations: obligation_codes(&completeness),
            incremental: {
                let mut evidence = full_execution_evidence(
                    IncrementalPlan::Full {
                        reason: FullAnalysisReason::NoParent,
                    },
                    WorkerRequestCounters {
                        open_project_requests: 0,
                        index_files_requests: 0,
                    },
                    if raw_hit {
                        AnalysisExecutionAuthority::CompilerOutputCheckpoint
                    } else {
                        AnalysisExecutionAuthority::CompilerProcess
                    },
                );
                evidence.compiler_output_receipt = compiler_output_receipt.clone();
                evidence
            },
            incremental_receipt,
            repository_snapshot: snapshot_object,
            derived_input_manifest,
            generation: generation_object,
            query_index,
            transformed_source,
        };
        verify_ready(store, &ready, session, compilation, true)?;
        Ok(ready)
    })();
    match result {
        Ok(ready) => {
            journal.transition(AttemptState::Ready, ready.generation.digest.clone())?;
            if let Some(path) = &checkpoint_path {
                publish_java_analysis_checkpoint(state, store, path, &ready)?;
            }
            write_java_analysis_request(state, binding_path, &ready, eligibility, raw_hit)?;
            write_private_atomic(state, binding_path, &ready)?;
            Ok(ready)
        }
        Err(error) => {
            journal.transition(AttemptState::Failed, "Java generation finalization failed")?;
            Err(error)
        }
    }
}

fn java_analysis_checkpoint_path(
    state: &StateAuthority,
    session: &SessionAuthority,
    generation_key: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let repository = state.repository_by_key(&session.repository_key)?;
    let root = repository.root.join("generations/java-analysis");
    state.directory_at(&root)?;
    Ok(root.join(format!("{}.json", digest_component(generation_key)?)))
}

#[allow(clippy::too_many_arguments)]
fn load_java_analysis_checkpoint(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    session: &SessionAuthority,
    compilation: &str,
    generation_key: &str,
    input: &CasObject,
    compiler_store: &CompilerStoreKey,
) -> Result<Option<ReadyGeneration>, ClewError> {
    if !state.private_file_exists(path)? {
        return Ok(None);
    }
    let bytes = state.read_private_file(path, MAX_BINDING_BYTES)?;
    let checkpoint: JavaAnalysisCheckpoint = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("Java analysis checkpoint is invalid"))?;
    if canonical::bytes(&checkpoint).map_err(internal)? != bytes
        || checkpoint.schema != JAVA_ANALYSIS_CHECKPOINT_SCHEMA
        || checkpoint.input != *input
        || checkpoint.ready.object_schema != READY_GENERATION_SCHEMA
    {
        return Err(corrupt(
            "Java analysis checkpoint input authority is invalid",
        ));
    }
    let ready: ReadyGeneration = read_canonical_object(store, &checkpoint.ready)?;
    if ready.generation_key != generation_key
        || ready.incremental.compiler_output_receipt.is_none()
        || ready.derived_input_manifest != *input
        || !matches!(
            ready.incremental.analysis_execution_authority,
            AnalysisExecutionAuthority::CompilerProcess
                | AnalysisExecutionAuthority::CompilerOutputCheckpoint
        )
        || ready.incremental.executed != IncrementalExecutionMode::Full
    {
        return Err(corrupt(
            "Java analysis checkpoint result authority is invalid",
        ));
    }
    verify_ready(store, &ready, session, compilation, true)?;
    let receipt: IncrementalReceipt = read_canonical_object(store, &ready.incremental_receipt)?;
    if receipt.compiler_store_key != compiler_store.key {
        return Err(corrupt(
            "Java analysis checkpoint compiler-store authority is invalid",
        ));
    }
    Ok(Some(ready))
}

fn publish_java_analysis_checkpoint(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    ready: &ReadyGeneration,
) -> Result<(), ClewError> {
    // CasStore holds its shared world lease until the durable repository root
    // is published. A crash before this atomic write leaves an unsaved unit.
    let checkpoint = JavaAnalysisCheckpoint {
        schema: JAVA_ANALYSIS_CHECKPOINT_SCHEMA.into(),
        input: ready.derived_input_manifest.clone(),
        ready: store.put(
            READY_GENERATION_SCHEMA,
            &canonical::bytes(ready).map_err(internal)?,
        )?,
    };
    write_canonical_atomic(state, path, &checkpoint)
}

fn write_java_analysis_request(
    state: &StateAuthority,
    binding_path: &Path,
    ready: &ReadyGeneration,
    eligibility: &str,
    hit: bool,
) -> Result<(), ClewError> {
    write_canonical_atomic(
        state,
        &binding_path.with_extension("java-analysis.json"),
        &JavaAnalysisRequest {
            schema: JAVA_ANALYSIS_REQUEST_SCHEMA.into(),
            compilation: ready.compilation.clone(),
            eligibility: eligibility.into(),
            lookup: if hit {
                "HIT"
            } else if eligibility == "ELIGIBLE" {
                "MISS"
            } else {
                "INELIGIBLE"
            }
            .into(),
            java_analyzer_starts: u64::from(!hit),
            compiler_output_receipt: ready.incremental.compiler_output_receipt.clone(),
            input: ready.derived_input_manifest.clone(),
            generation: ready.generation.clone(),
        },
    )
}

fn effective_java_sources(
    snapshot: &RepositoryInputSnapshot,
) -> Result<BTreeMap<String, CasObject>, ClewError> {
    let mut sources = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0 && entry.path.ends_with(".java"))
        .map(|entry| (entry.path.clone(), entry.content.clone()))
        .collect::<BTreeMap<_, _>>();
    for entry in snapshot
        .worktree
        .iter()
        .filter(|entry| entry.path.ends_with(".java"))
    {
        match entry.kind {
            WorktreeKind::Missing => {
                sources.remove(&entry.path);
            }
            WorktreeKind::Regular => {
                sources.insert(
                    entry.path.clone(),
                    entry
                        .content
                        .clone()
                        .ok_or_else(|| corrupt("Java source has no content authority"))?,
                );
            }
            WorktreeKind::Symlink => {
                return Err(ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "Java source symlinks are unsupported",
                ));
            }
        }
    }
    Ok(sources)
}

fn java_source_content_digests(
    store: &CasStore,
    sources: &BTreeMap<String, CasObject>,
    model: &JavaOperationalModel,
) -> Result<BTreeMap<String, String>, ClewError> {
    let digests = original_java_source_content_digests(store, sources, model)?;
    if digests.len() != model.authority.source_files.len() {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            format!(
                "Java model selected a source outside the sealed snapshot for {}; build-added sources require the java-17plus-maven-writable-then-seal profile",
                model.authority.compilation
            ),
        ));
    }
    Ok(digests)
}

/// Original snapshot authority for the admitted files that existed before the
/// build. Writable-then-seal may add files: those have only transformed after
/// authority, persisted and verified separately. Read-only callers must use
/// `java_source_content_digests`, which rejects any absent original member.
fn original_java_source_content_digests(
    store: &CasStore,
    sources: &BTreeMap<String, CasObject>,
    model: &JavaOperationalModel,
) -> Result<BTreeMap<String, String>, ClewError> {
    model
        .authority
        .source_files
        .iter()
        .filter_map(|path| {
            sources.get(path).map(|source| {
                source_content_digest(store, source).map(|digest| (path.clone(), digest))
            })
        })
        .collect()
}

/// Re-hash the model's source files from the (sealed) transformed worktree,
/// keyed by repository-relative path. Used only for the writable-then-seal
/// profile, where the build may rewrite source in place. This indexes only the
/// compiled source set (not generated sources under target/); a model source
/// that disappeared during transformation is a hard error.
fn transformed_java_source_digests(
    repository: &Path,
    source_files: &[String],
) -> Result<BTreeMap<String, String>, ClewError> {
    let mut out = BTreeMap::new();
    for path in source_files {
        let file = repository.join(path);
        if !file.is_file() {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                format!("Java model selected a source absent after transformation: {path}"),
            ));
        }
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "CODEDEBUG transformed_java_source_digests read FAIL {}: {e}",
                    file.display()
                );
                return Err(io_error(e));
            }
        };
        out.insert(path.clone(), canonical::hash_bytes(&bytes));
    }
    eprintln!(
        "CODEDEBUG transformed_java_source_digests OK count={}",
        out.len()
    );
    Ok(out)
}

/// A transformed source path must be a safe repository-relative path: nonempty,
/// not absolute, and made only of normal components (no `.`, `..`, or backslash
/// escapes). Rejects traversal that could escape the sealed materialization.
fn is_safe_relative_source_path(path: &str) -> bool {
    use std::path::{Component, Path};
    !path.is_empty()
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Extract the authoritative `after` digest map from a transformed source
/// state. The `after` map records the exact persisted-byte digests for every
/// admitted source file and is authoritative for what is trusted on reopen;
/// `before`/`changedFiles` are attribution only. A missing or malformed state
/// fails closed: it cannot become trusted transformed evidence.
fn transformed_source_state_after(
    source_state: &serde_json::Value,
) -> Result<&serde_json::Map<String, serde_json::Value>, ClewError> {
    if source_state.get("kind").and_then(Value::as_str) != Some("TRANSFORMED_WORKSPACE") {
        return Err(corrupt("transformed source state kind is invalid"));
    }
    source_state
        .get("after")
        .and_then(Value::as_object)
        .ok_or_else(|| corrupt("transformed source state has no authoritative after map"))
}

/// The admitted file set must match the authoritative after-map exactly:
/// every admitted source has a digest and every after entry is admitted. A
/// missing or extra path fails before any manifest reference is published.
fn transformed_paths_match_source_files(
    source_files: &[String],
    after: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ClewError> {
    let admitted: BTreeSet<&String> = source_files.iter().collect();
    let indexed: BTreeSet<&String> = after.keys().collect();
    if admitted.len() != source_files.len()
        || admitted != indexed
        || after.values().any(|v| !v.is_string())
    {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "transformed source state paths do not exactly match admitted source files",
        ));
    }
    Ok(())
}

/// Persist the immutable transformed source bytes a writable-then-seal Java
/// generation was indexed against, together with its source_state/provenance
/// marker. Returns the CAS reference (or None when not transformed). The bytes
/// are read from the sealed repository materialization after transformation.
///
/// The authoritative `after` map (indexed byte digests) must exactly match the
/// admitted source files, and each file's bytes must hash to its declared
/// digest BEFORE any manifest reference is published. A source that changed
/// after index-state construction, or any missing/extra/malformed state, fails
/// before publication. Unreachable CAS writes are not publication.
fn persist_transformed_source(
    store: &CasStore,
    repository: &Path,
    source_files: &[String],
    provenance: Option<&str>,
    source_state: Option<&serde_json::Value>,
) -> Result<Option<CasObject>, ClewError> {
    if provenance != Some("TRANSFORMED_WORKSPACE") {
        return Ok(None);
    }
    let state = source_state
        .ok_or_else(|| corrupt("transformed source provenance lacks the indexed source state"))?;
    let after = transformed_source_state_after(state)?;
    transformed_paths_match_source_files(source_files, after)?;
    // Validate every file's bytes against its authoritative digest before
    // publishing the manifest reference; a transform that mutated sources after
    // index-state construction must never be persisted as transformed evidence.
    let mut files = BTreeMap::new();
    for path in source_files {
        if !is_safe_relative_source_path(path) {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                "transformed source path is not a safe relative path",
            ));
        }
        let expected = after
            .get(path)
            .and_then(Value::as_str)
            .ok_or_else(|| corrupt("transformed source state has no digest for admitted file"))?;
        let bytes = std::fs::read(repository.join(path)).map_err(io_error)?;
        let digest = canonical::hash_bytes(&bytes);
        if digest != expected {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                "transformed source bytes changed after index-state construction",
            ));
        }
        let content = store.put(TRANSFORMED_SOURCE_FILE_SCHEMA, &bytes)?;
        files.insert(path.clone(), content);
    }
    let manifest = store.put(
        TRANSFORMED_SOURCE_SCHEMA,
        &canonical::bytes(&json!({
            "schema": TRANSFORMED_SOURCE_SCHEMA,
            "kind": "TRANSFORMED_WORKSPACE",
            "provenance": "TRANSFORMED_WORKSPACE",
            "sourceState": source_state,
            "files": files,
        }))
        .map_err(internal)?,
    )?;
    Ok(Some(manifest))
}

/// Load a persisted transformed-source manifest and return the indexed bytes
/// keyed by repository-relative path. Callers must bound reads to their own
/// evidence budget.
///
/// The manifest is trusted only if it carries the authoritative source state
/// and every persisted file matches it: safe relative paths, exact
/// sourceState.after membership, per-file schema, and byte hashes. A legacy or
/// well-formed-but-inconsistent manifest fails closed rather than being
/// silently promoted to trusted transformed evidence.
pub(crate) fn load_transformed_source(
    store: &CasStore,
    reference: &CasObject,
) -> Result<BTreeMap<String, Vec<u8>>, ClewError> {
    if reference.object_schema != TRANSFORMED_SOURCE_SCHEMA {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "transformed source reference schema is invalid",
        ));
    }
    let limit = usize::try_from(reference.size)
        .map_err(|_| resource("transformed source manifest exceeds host size"))?;
    let lease = store.read(reference, limit)?;
    let manifest: serde_json::Value = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("transformed source is invalid"))?;
    if manifest["schema"].as_str() != Some(TRANSFORMED_SOURCE_SCHEMA)
        || manifest["kind"].as_str() != Some("TRANSFORMED_WORKSPACE")
    {
        return Err(corrupt("transformed source manifest authority is invalid"));
    }
    let state = manifest
        .get("sourceState")
        .ok_or_else(|| corrupt("transformed source manifest lacks integrity source state"))?;
    let after = transformed_source_state_after(state)?;
    let files = manifest["files"]
        .as_object()
        .ok_or_else(|| corrupt("transformed source manifest has no file map"))?;
    if files.len() != after.len() || files.keys().any(|k| !after.contains_key(k)) {
        return Err(corrupt(
            "transformed source manifest files do not match indexed source state",
        ));
    }
    let mut out = BTreeMap::new();
    for (path, content) in files {
        if !is_safe_relative_source_path(path) {
            return Err(corrupt("transformed source file path is unsafe"));
        }
        let object: CasObject = serde_json::from_value(content.clone())
            .map_err(|_| corrupt("transformed source file"))?;
        if object.object_schema != TRANSFORMED_SOURCE_FILE_SCHEMA {
            return Err(corrupt("transformed source file object schema is invalid"));
        }
        let expected = after
            .get(path)
            .and_then(Value::as_str)
            .ok_or_else(|| corrupt("transformed source file has no indexed digest"))?;
        let limit = usize::try_from(object.size)
            .map_err(|_| resource("transformed source file exceeds host size"))?;
        let bytes = store.read(&object, limit)?.bytes().to_vec();
        if canonical::hash_bytes(&bytes) != expected {
            return Err(corrupt(
                "transformed source file hash does not match indexed source state",
            ));
        }
        out.insert(path.clone(), bytes);
    }
    Ok(out)
}

fn java_incremental_receipt(
    store: &CasStore,
    index: &JavaCompilerIndex,
    source_content_digests: &BTreeMap<String, String>,
    compiler_store: &CompilerStoreKey,
    generation: &GenerationManifest,
    completeness: CompletenessVector,
) -> Result<CasObject, ClewError> {
    let mut surfaces = BTreeMap::<String, Vec<&JavaCompilerFact>>::new();
    for fact in &index.facts {
        if let JavaCompilerFact::Declaration { file, .. } = fact {
            surfaces.entry(file.clone()).or_default().push(fact);
        }
    }
    let files = source_content_digests
        .iter()
        .map(|(path, content_digest)| {
            let surface = surfaces.remove(path).unwrap_or_default();
            Ok(FileReceipt {
                path: path.clone(),
                content_digest: content_digest.clone(),
                exported_surface_digest: canonical::hash(&surface).map_err(internal)?,
                dependencies: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    let receipt = IncrementalReceipt {
        schema: INCREMENTAL_RECEIPT_SCHEMA.into(),
        compiler_store_key: compiler_store.key.clone(),
        generation_id: generation.generation_id.clone(),
        files,
        boundaries: Vec::<BoundaryReceipt>::new(),
        completeness,
    };
    receipt.validate()?;
    store.put(
        INCREMENTAL_RECEIPT_SCHEMA,
        &canonical::bytes(&receipt).map_err(internal)?,
    )
}

#[allow(clippy::too_many_arguments)]
fn ensure_typescript_generation_set(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    repository: &Path,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: CasObject,
    compilation_root: &Path,
    binding_path: &Path,
    binding_prefix: &str,
) -> Result<ReadyGenerationSet, ClewError> {
    let (target_snapshot, target_before) = capture(repository, store)?;
    if target_snapshot.index != snapshot.index || target_snapshot.worktree != snapshot.worktree {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "TypeScript target differs from the sealed session snapshot",
        ));
    }
    let sources = effective_typescript_sources(snapshot, session.language)?;
    let mut results = Vec::with_capacity(session.compilations.len());
    for compilation in &session.compilations {
        let component = digest_component(
            &canonical::hash(&json!({
                "schema":"codeclew-session-typescript-compilation-binding/1.0",
                "compilation":compilation,
            }))
            .map_err(internal)?,
        )?
        .to_owned();
        let model = match session.language {
            SessionLanguage::JavaScript => extract_javascript_model(repository, compilation)?,
            SessionLanguage::TypeScript => extract_typescript_model(repository, compilation)?,
            _ => unreachable!("ECMAScript generation requires an ECMAScript session"),
        };
        let source_content_digests = typescript_source_content_digests(store, &sources, &model)?;
        results.push(ensure_typescript_generation(
            session,
            state,
            store,
            snapshot_object.clone(),
            model,
            source_content_digests,
            compilation,
            &compilation_root.join(format!("{binding_prefix}{component}.json")),
        )?);
    }
    let (_, target_after) = capture(repository, store)?;
    if target_after != target_before {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "TypeScript analysis changed the sealed repository input",
        ));
    }
    let ready = assemble_ready_set(session, snapshot_object, results)?;
    write_ready_set(state, binding_path, &ready)?;
    Ok(ready)
}

#[allow(clippy::too_many_arguments)]
fn ensure_typescript_generation(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    snapshot_object: CasObject,
    model: TypeScriptOperationalModel,
    source_content_digests: BTreeMap<String, String>,
    compilation: &str,
    binding_path: &Path,
) -> Result<ReadyGeneration, ClewError> {
    if state.private_file_exists(binding_path)? {
        return load_ready(state, store, binding_path, session, compilation, false);
    }
    let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "generation service must run through ./clew",
        )
    })?;
    let javascript = session.language == SessionLanguage::JavaScript;
    let language_uri = if javascript {
        JAVASCRIPT_LANGUAGE
    } else {
        TYPESCRIPT_LANGUAGE
    };
    let capability = if javascript {
        JAVASCRIPT_COMPILER_FACTS_CAPABILITY
    } else {
        TYPESCRIPT_COMPILER_FACTS_CAPABILITY
    };
    let model_schema = if javascript {
        JAVASCRIPT_MODEL_SCHEMA
    } else {
        TYPESCRIPT_MODEL_SCHEMA
    };
    let profile_name = if javascript {
        "javascript"
    } else {
        "typescript"
    };
    if model.authority.language != language_uri {
        return Err(corrupt(
            "ECMAScript model language differs from its session",
        ));
    }
    let model_object = store.put(
        model_schema,
        &canonical::bytes(&model.authority).map_err(internal)?,
    )?;
    let toolchain = store.put(
        &format!("codeclew-{profile_name}-toolchain-authority/1.0"),
        &canonical::bytes(&json!({
            "schema":format!("codeclew-{profile_name}-toolchain-authority/1.0"),
            "compilerVersion":model.authority.compiler_version,
            "compilerModuleDigest":model.authority.compiler_module_digest,
            "nodeVersion":model.authority.node_version,
            "analyzerDigest":crate::typescript_project_model::analyzer_digest(),
        }))
        .map_err(internal)?,
    )?;
    let mut classpath = model
        .authority
        .external_files
        .iter()
        .map(|authority| {
            store.put(
                &format!("codeclew-{profile_name}-declaration-authority/1.0"),
                &canonical::bytes(authority).map_err(internal)?,
            )
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    classpath.sort_by(|left, right| left.digest.cmp(&right.digest));
    classpath.dedup_by(|left, right| left.digest == right.digest);
    let descriptor = CompilationDescriptor {
        schema: COMPILATION_SCHEMA.into(),
        compilation_id: safe_compilation_id(compilation),
        language_uri: LanguageUri::parse(language_uri)?,
        source_roots: vec![SourceRootDescriptor {
            logical_name: "project".into(),
            tree: snapshot_object.clone(),
        }],
        generated_source_roots: Vec::new(),
        classpath,
        toolchain,
        plugins: Vec::new(),
        canonical_options: model_object.clone(),
        dependency_compilation_ids: Vec::new(),
        operations: Vec::new(),
        origin: DescriptorOrigin::ProjectNative,
        completeness: if model.authority.boundaries.is_empty() {
            DescriptorCompleteness::Complete
        } else {
            DescriptorCompleteness::Partial
        },
    };
    let provider = ProviderModel {
        handshake: ProviderHandshake {
            protocol: PROVIDER_PROTOCOL.into(),
            provider_id: format!("project-native-{profile_name}"),
            provider_digest: model.authority.model_digest.clone(),
            build_system_uris: vec!["build:tsconfig".into()],
        },
        build_model: BuildModel {
            provider_id: format!("project-native-{profile_name}"),
            model: model_object,
            compilations: vec![descriptor.clone()],
        },
    };
    let (_, derived_input_manifest) =
        DerivedAnalysisInputManifest::create(store, snapshot_object.clone(), vec![provider])?;
    let generation_key = final_generation_key(
        &runtime.runtime_key,
        &session.base_revision,
        &snapshot_object,
        compilation,
        &derived_input_manifest,
        false,
    )?;
    let _lock = GenerationLock::acquire(state, &generation_key)?;
    if state.private_file_exists(binding_path)? {
        return load_ready(state, store, binding_path, session, compilation, false);
    }
    let adapter_digest = typescript_adapter_digest()?;
    let compiler_store = CompilerStoreKey::create(
        format!("{profile_name}-compiler-1"),
        adapter_digest.clone(),
        &descriptor,
    )?;
    let index = build_typescript_compiler_index(model, &source_content_digests)?;
    let adapter = TypeScriptAdapterV2::new(
        adapter_digest,
        descriptor.toolchain.digest.clone(),
        descriptor.compilation_id.clone(),
        store.clone(),
        index.clone(),
    )?;
    let mut registry = AdapterRegistry::default();
    registry.register_adapter(Arc::new(adapter))?;
    let mut journal = AttemptJournal::create(state.clone(), &generation_key, 0)?;
    journal.transition(AttemptState::Snapshotted, snapshot_object.digest.clone())?;
    journal.transition(AttemptState::Modeled, derived_input_manifest.digest.clone())?;
    journal.transition(
        AttemptState::Analyzing,
        "ECMAScript compiler adapter DAG started",
    )?;
    let request = AnalyzeGenerationRequest {
        schema: ANALYSIS_REQUEST_SCHEMA.into(),
        attempt_id: journal.attempt().attempt_id.clone(),
        generation_key: generation_key.clone(),
        capability: CapabilityUri::parse(capability)?,
        compilation: descriptor,
        derived_input_manifest: derived_input_manifest.clone(),
        parent_generation: None,
    };
    let analysis = match HostResources::detect().and_then(|resources| {
        execute_analysis_dag_with_jobs(state, Arc::new(registry), request, resources, 1)
    }) {
        Ok(analysis) => analysis,
        Err(error) => {
            journal.transition(
                AttemptState::Failed,
                "ECMAScript compiler adapter DAG failed",
            )?;
            return Err(error);
        }
    };
    journal.transition(
        AttemptState::Finalizing,
        "ECMAScript deterministic merge started",
    )?;
    let result = (|| {
        let (generation, generation_object) = finalize_generation(
            store,
            derived_input_manifest.clone(),
            vec![AttemptAuthority {
                compilation_id: safe_compilation_id(compilation),
                capability: CapabilityUri::parse(capability)?,
                completion: analysis.completion,
            }],
            analysis.runs,
        )?;
        let (_, query_index) = build_query_index(store, &generation, generation_object.clone())?;
        let scope_digest = typescript_scope_digest(&index)?;
        let completeness = typescript_completeness(&index, &scope_digest)?;
        let incremental_receipt = typescript_incremental_receipt(
            store,
            &index,
            &source_content_digests,
            &compiler_store,
            &generation,
            completeness.clone(),
        )?;
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key,
            runtime_key: runtime.runtime_key.clone(),
            base_revision: session.base_revision.clone(),
            compilation: compilation.into(),
            compiler_version: index.model.compiler_version.clone(),
            completeness: completeness.clone(),
            coverage: coverage_label(&completeness).into(),
            certainty: certainty_label(&completeness).into(),
            obligations: obligation_codes(&completeness),
            incremental: full_execution_evidence(
                IncrementalPlan::Full {
                    reason: FullAnalysisReason::NoParent,
                },
                WorkerRequestCounters {
                    open_project_requests: 0,
                    index_files_requests: 0,
                },
                AnalysisExecutionAuthority::CompilerProcess,
            ),
            incremental_receipt,
            repository_snapshot: snapshot_object,
            derived_input_manifest,
            generation: generation_object,
            query_index,
            transformed_source: None,
        };
        verify_ready(store, &ready, session, compilation, true)?;
        Ok(ready)
    })();
    match result {
        Ok(ready) => {
            journal.transition(AttemptState::Ready, ready.generation.digest.clone())?;
            write_private_atomic(state, binding_path, &ready)?;
            Ok(ready)
        }
        Err(error) => {
            journal.transition(
                AttemptState::Failed,
                "TypeScript generation finalization failed",
            )?;
            Err(error)
        }
    }
}

fn effective_typescript_sources(
    snapshot: &RepositoryInputSnapshot,
    language: SessionLanguage,
) -> Result<BTreeMap<String, CasObject>, ClewError> {
    fn is_source(path: &str, language: SessionLanguage) -> bool {
        path.ends_with(".d.ts")
            || match language {
                SessionLanguage::JavaScript => [".js", ".jsx", ".mjs", ".cjs"]
                    .iter()
                    .any(|extension| path.ends_with(extension)),
                SessionLanguage::TypeScript => [".ts", ".tsx", ".mts", ".cts"]
                    .iter()
                    .any(|extension| path.ends_with(extension)),
                _ => false,
            }
    }
    let mut sources = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0 && is_source(&entry.path, language))
        .map(|entry| (entry.path.clone(), entry.content.clone()))
        .collect::<BTreeMap<_, _>>();
    for entry in snapshot
        .worktree
        .iter()
        .filter(|entry| is_source(&entry.path, language))
    {
        match entry.kind {
            WorktreeKind::Missing => {
                sources.remove(&entry.path);
            }
            WorktreeKind::Regular => {
                sources.insert(
                    entry.path.clone(),
                    entry
                        .content
                        .clone()
                        .ok_or_else(|| corrupt("TypeScript source has no content authority"))?,
                );
            }
            WorktreeKind::Symlink => {
                return Err(ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "ECMAScript source symlinks are unsupported",
                ));
            }
        }
    }
    Ok(sources)
}

fn typescript_source_content_digests(
    store: &CasStore,
    sources: &BTreeMap<String, CasObject>,
    model: &TypeScriptOperationalModel,
) -> Result<BTreeMap<String, String>, ClewError> {
    model
        .authority
        .source_files
        .iter()
        .map(|path| {
            let source = sources.get(path).ok_or_else(|| {
                ClewError::new(
                    ErrorCode::InputMutated,
                    "TypeScript model selected a source outside the sealed snapshot",
                )
            })?;
            Ok((path.clone(), source_content_digest(store, source)?))
        })
        .collect()
}

fn typescript_completeness(
    index: &TypeScriptCompilerIndex,
    scope_digest: &str,
) -> Result<CompletenessVector, ClewError> {
    if !index
        .facts
        .iter()
        .any(|fact| matches!(fact, TypeScriptCompilerFact::Boundary { .. }))
    {
        return CompletenessVector::verified_complete(scope_digest.into());
    }
    let completeness = CompletenessVector {
        schema: COMPLETENESS_VECTOR_SCHEMA.into(),
        support: Support::Supported,
        coverage: Coverage::Partial {
            observed_scopes: vec![scope_digest.into()],
            boundaries: vec!["TYPESCRIPT_COMPILER_BOUNDARY".into()],
        },
        certainty: Certainty::Unsure {
            check_set: vec!["typescript-configuration-dependencies-and-diagnostics".into()],
        },
        obligations: vec![VerificationObligation {
            code: "FIX_TYPESCRIPT_CONFIGURATION_DEPENDENCY_OR_DIAGNOSTIC".into(),
            subject: vec![scope_digest.into()],
            publication_blocking: true,
        }],
    };
    completeness.validate()?;
    Ok(completeness)
}

fn typescript_incremental_receipt(
    store: &CasStore,
    index: &TypeScriptCompilerIndex,
    source_content_digests: &BTreeMap<String, String>,
    compiler_store: &CompilerStoreKey,
    generation: &GenerationManifest,
    completeness: CompletenessVector,
) -> Result<CasObject, ClewError> {
    let mut surfaces = BTreeMap::<String, Vec<&TypeScriptCompilerFact>>::new();
    for fact in &index.facts {
        if let TypeScriptCompilerFact::Declaration { file, .. } = fact {
            surfaces.entry(file.clone()).or_default().push(fact);
        }
    }
    let files = source_content_digests
        .iter()
        .map(|(path, content_digest)| {
            let surface = surfaces.remove(path).unwrap_or_default();
            Ok(FileReceipt {
                path: path.clone(),
                content_digest: content_digest.clone(),
                exported_surface_digest: canonical::hash(&surface).map_err(internal)?,
                dependencies: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    let receipt = IncrementalReceipt {
        schema: INCREMENTAL_RECEIPT_SCHEMA.into(),
        compiler_store_key: compiler_store.key.clone(),
        generation_id: generation.generation_id.clone(),
        files,
        boundaries: Vec::new(),
        completeness,
    };
    receipt.validate()?;
    store.put(
        INCREMENTAL_RECEIPT_SCHEMA,
        &canonical::bytes(&receipt).map_err(internal)?,
    )
}

#[allow(clippy::too_many_arguments)]
fn ensure_python_generation_set(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: CasObject,
    compilation_root: &Path,
    binding_path: &Path,
    binding_prefix: &str,
) -> Result<ReadyGenerationSet, ClewError> {
    let model = PythonProjectModel::create(&session.compilations)?;
    let model_object = store.put(
        PYTHON_MODEL_SCHEMA,
        &canonical::bytes(&model).map_err(internal)?,
    )?;
    let mut results = Vec::with_capacity(session.compilations.len());
    for compilation in &session.compilations {
        let component = digest_component(
            &canonical::hash(&json!({
                "schema":"codeclew-session-python-compilation-binding/1.0",
                "compilation":compilation,
            }))
            .map_err(internal)?,
        )?
        .to_owned();
        let selector = model
            .selectors
            .iter()
            .find(|selector| selector.canonical() == *compilation)
            .ok_or_else(|| corrupt("Python model misses a selected source scope"))?;
        results.push(ensure_python_generation(
            session,
            state,
            store,
            snapshot,
            &snapshot_object,
            &model,
            &model_object,
            selector,
            compilation,
            &compilation_root.join(format!("{binding_prefix}{component}.json")),
        )?);
    }
    let ready = assemble_ready_set(session, snapshot_object, results)?;
    write_ready_set(state, binding_path, &ready)?;
    Ok(ready)
}

#[allow(clippy::too_many_arguments)]
fn ensure_python_generation(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: &CasObject,
    model: &PythonProjectModel,
    model_object: &CasObject,
    selector: &crate::python_project_model::PythonCompilationSelector,
    compilation: &str,
    binding_path: &Path,
) -> Result<ReadyGeneration, ClewError> {
    if state.private_file_exists(binding_path)? {
        return load_ready(state, store, binding_path, session, compilation, false);
    }
    let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "generation service must run through ./clew",
        )
    })?;
    let toolchain = store.put(
        "codeclew-python-grammar-authority/1.0",
        &canonical::bytes(&json!({
            "schema":"codeclew-python-grammar-authority/1.0",
            "grammarAuthority":PYTHON_GRAMMAR_AUTHORITY,
            "adapterProtocol":"python-syntax-1",
        }))
        .map_err(internal)?,
    )?;
    let options = store.put(
        "codeclew-python-source-scope/1.0",
        &canonical::bytes(selector).map_err(internal)?,
    )?;
    let descriptor = CompilationDescriptor {
        schema: COMPILATION_SCHEMA.into(),
        compilation_id: safe_compilation_id(compilation),
        language_uri: LanguageUri::parse(PYTHON_LANGUAGE)?,
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
            provider_id: "project-native-python-syntax".into(),
            provider_digest: model.model_digest.clone(),
            build_system_uris: vec!["build:python-syntax".into()],
        },
        build_model: BuildModel {
            provider_id: "project-native-python-syntax".into(),
            model: model_object.clone(),
            compilations: vec![descriptor.clone()],
        },
    };
    let (_, derived_input_manifest) =
        DerivedAnalysisInputManifest::create(store, snapshot_object.clone(), vec![provider])?;
    let generation_key = final_generation_key(
        &runtime.runtime_key,
        &session.base_revision,
        snapshot_object,
        compilation,
        &derived_input_manifest,
        false,
    )?;
    let _lock = GenerationLock::acquire(state, &generation_key)?;
    if state.private_file_exists(binding_path)? {
        return load_ready(state, store, binding_path, session, compilation, false);
    }
    let adapter_digest = python_adapter_digest()?;
    let compiler_store =
        CompilerStoreKey::create("python-syntax-1", adapter_digest.clone(), &descriptor)?;
    let index = build_python_syntax_index(
        store,
        snapshot,
        &PythonSyntaxAuthority {
            compilation_id: &descriptor.compilation_id,
            model_digest: &model.model_digest,
            selector,
        },
    )?;
    let adapter = PythonAdapterV2::new(
        adapter_digest,
        descriptor.toolchain.digest.clone(),
        store.clone(),
        index.clone(),
    )?;
    let mut registry = AdapterRegistry::default();
    registry.register_adapter(Arc::new(adapter))?;
    let mut journal = AttemptJournal::create(state.clone(), &generation_key, 0)?;
    journal.transition(AttemptState::Snapshotted, snapshot_object.digest.clone())?;
    journal.transition(AttemptState::Modeled, derived_input_manifest.digest.clone())?;
    journal.transition(AttemptState::Analyzing, "Python syntax adapter DAG started")?;
    let request = AnalyzeGenerationRequest {
        schema: ANALYSIS_REQUEST_SCHEMA.into(),
        attempt_id: journal.attempt().attempt_id.clone(),
        generation_key: generation_key.clone(),
        capability: CapabilityUri::parse(PYTHON_SYNTAX_FACTS_CAPABILITY)?,
        compilation: descriptor,
        derived_input_manifest: derived_input_manifest.clone(),
        parent_generation: None,
    };
    let analysis = match HostResources::detect().and_then(|resources| {
        execute_analysis_dag_with_jobs(state, Arc::new(registry), request, resources, 1)
    }) {
        Ok(analysis) => analysis,
        Err(error) => {
            journal.transition(AttemptState::Failed, "Python adapter DAG failed")?;
            return Err(error);
        }
    };
    journal.transition(
        AttemptState::Finalizing,
        "Python deterministic merge started",
    )?;
    let result = (|| {
        let (generation, generation_object) = finalize_generation(
            store,
            derived_input_manifest.clone(),
            vec![AttemptAuthority {
                compilation_id: safe_compilation_id(compilation),
                capability: CapabilityUri::parse(PYTHON_SYNTAX_FACTS_CAPABILITY)?,
                completion: analysis.completion,
            }],
            analysis.runs,
        )?;
        let (_, query_index) = build_query_index(store, &generation, generation_object.clone())?;
        let scope_digest = python_scope_digest(&index)?;
        let completeness = python_syntax_completeness(&scope_digest)?;
        let (_, incremental_receipt) = create_incremental_receipt(
            store,
            &index,
            &compiler_store,
            &generation,
            completeness.clone(),
        )?;
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key,
            runtime_key: runtime.runtime_key.clone(),
            base_revision: session.base_revision.clone(),
            compilation: compilation.into(),
            compiler_version: PYTHON_GRAMMAR_AUTHORITY.into(),
            completeness: completeness.clone(),
            coverage: coverage_label(&completeness).into(),
            certainty: certainty_label(&completeness).into(),
            obligations: obligation_codes(&completeness),
            incremental: full_execution_evidence(
                IncrementalPlan::Full {
                    reason: FullAnalysisReason::NoParent,
                },
                WorkerRequestCounters {
                    open_project_requests: 0,
                    index_files_requests: 0,
                },
                AnalysisExecutionAuthority::InProcessSyntax,
            ),
            incremental_receipt,
            repository_snapshot: snapshot_object.clone(),
            derived_input_manifest,
            generation: generation_object,
            query_index,
            transformed_source: None,
        };
        verify_ready(store, &ready, session, compilation, true)?;
        Ok(ready)
    })();
    match result {
        Ok(ready) => {
            journal.transition(AttemptState::Ready, ready.generation.digest.clone())?;
            write_private_atomic(state, binding_path, &ready)?;
            Ok(ready)
        }
        Err(error) => {
            journal.transition(
                AttemptState::Failed,
                "Python generation finalization failed",
            )?;
            Err(error)
        }
    }
}

fn python_syntax_completeness(scope_digest: &str) -> Result<CompletenessVector, ClewError> {
    let completeness = CompletenessVector {
        schema: COMPLETENESS_VECTOR_SCHEMA.into(),
        support: Support::Supported,
        coverage: Coverage::Partial {
            observed_scopes: vec![scope_digest.into()],
            boundaries: vec![
                "PYTHON_DYNAMIC_SEMANTICS_UNMODELED".into(),
                "PYTHON_IMPORT_RUNTIME_UNMODELED".into(),
                "PYTHON_SYNTAX_ONLY".into(),
            ],
        },
        certainty: Certainty::Unsure {
            check_set: vec![
                "python-runtime-imports-and-types".into(),
                "python-runtime-tests".into(),
            ],
        },
        obligations: vec![
            VerificationObligation {
                code: "VERIFY_DECORATORS_METACLAS_AND_DYNAMIC_EXECUTION".into(),
                subject: vec![scope_digest.into()],
                publication_blocking: true,
            },
            VerificationObligation {
                code: "VERIFY_PYTHON_RUNTIME_IMPORTS_AND_TYPES".into(),
                subject: vec![scope_digest.into()],
                publication_blocking: true,
            },
        ],
    };
    completeness.validate()?;
    Ok(completeness)
}

#[allow(clippy::too_many_arguments)]
fn ensure_rust_generation(
    session: &SessionAuthority,
    state: &StateAuthority,
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    snapshot_object: &CasObject,
    model: &CargoProjectModel,
    model_object: &CasObject,
    target: &CargoTargetModel,
    compilation: &str,
    binding_path: &Path,
    publish_head: bool,
) -> Result<ReadyGeneration, ClewError> {
    if state.private_file_exists(binding_path)? {
        return load_ready(state, store, binding_path, session, compilation, false);
    }
    let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "generation service must run through ./clew",
        )
    })?;
    let toolchain = store.put(
        "codeclew-rust-toolchain-authority/1.0",
        &canonical::bytes(&json!({
            "schema":"codeclew-rust-toolchain-authority/1.0",
            "cargoVersion":model.cargo_version,
            "rustcVersion":model.rustc_version,
        }))
        .map_err(internal)?,
    )?;
    let options = store.put(
        "codeclew-cargo-target-options/1.0",
        &canonical::bytes(&target).map_err(internal)?,
    )?;
    let descriptor = CompilationDescriptor {
        schema: COMPILATION_SCHEMA.into(),
        compilation_id: safe_compilation_id(compilation),
        language_uri: LanguageUri::parse(RUST_LANGUAGE)?,
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
            provider_id: "project-native-cargo".into(),
            provider_digest: model.model_digest.clone(),
            build_system_uris: vec!["build:cargo".into()],
        },
        build_model: BuildModel {
            provider_id: "project-native-cargo".into(),
            model: model_object.clone(),
            compilations: vec![descriptor.clone()],
        },
    };
    let (_, derived_input_manifest) =
        DerivedAnalysisInputManifest::create(store, snapshot_object.clone(), vec![provider])?;
    let generation_key = final_generation_key(
        &runtime.runtime_key,
        &session.base_revision,
        snapshot_object,
        compilation,
        &derived_input_manifest,
        false,
    )?;
    let _lock = GenerationLock::acquire(state, &generation_key)?;
    if state.private_file_exists(binding_path)? {
        return load_ready(state, store, binding_path, session, compilation, false);
    }
    let adapter_digest = rust_adapter_digest()?;
    let compiler_store =
        CompilerStoreKey::create("rust-syntax-1", adapter_digest.clone(), &descriptor)?;
    let index = rust_model_index(store, snapshot, model, target, &descriptor.compilation_id)?;
    let adapter = RustAdapterV2::new(
        adapter_digest,
        descriptor.toolchain.digest.clone(),
        store.clone(),
        index.clone(),
    )?;
    let mut registry = AdapterRegistry::default();
    registry.register_adapter(Arc::new(adapter))?;
    let mut journal = AttemptJournal::create(state.clone(), &generation_key, 0)?;
    journal.transition(AttemptState::Snapshotted, snapshot_object.digest.clone())?;
    journal.transition(AttemptState::Modeled, derived_input_manifest.digest.clone())?;
    journal.transition(AttemptState::Analyzing, "Rust syntax adapter DAG started")?;
    let request = AnalyzeGenerationRequest {
        schema: ANALYSIS_REQUEST_SCHEMA.into(),
        attempt_id: journal.attempt().attempt_id.clone(),
        generation_key: generation_key.clone(),
        capability: CapabilityUri::parse(RUST_SYNTAX_FACTS_CAPABILITY)?,
        compilation: descriptor,
        derived_input_manifest: derived_input_manifest.clone(),
        parent_generation: None,
    };
    let analysis = match HostResources::detect().and_then(|resources| {
        execute_analysis_dag_with_jobs(state, Arc::new(registry), request, resources, 1)
    }) {
        Ok(analysis) => analysis,
        Err(error) => {
            journal.transition(AttemptState::Failed, "Rust adapter DAG failed")?;
            return Err(error);
        }
    };
    journal.transition(AttemptState::Finalizing, "Rust deterministic merge started")?;
    let result = (|| {
        let (generation, generation_object) = finalize_generation(
            store,
            derived_input_manifest.clone(),
            vec![AttemptAuthority {
                compilation_id: safe_compilation_id(compilation),
                capability: CapabilityUri::parse(RUST_SYNTAX_FACTS_CAPABILITY)?,
                completion: analysis.completion,
            }],
            analysis.runs,
        )?;
        let (_, query_index) = build_query_index(store, &generation, generation_object.clone())?;
        let scope_digest = rust_scope_digest(&index)?;
        let completeness = rust_syntax_completeness(&scope_digest)?;
        let (_, incremental_receipt) = create_incremental_receipt(
            store,
            &index,
            &compiler_store,
            &generation,
            completeness.clone(),
        )?;
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key,
            runtime_key: runtime.runtime_key.clone(),
            base_revision: session.base_revision.clone(),
            compilation: compilation.into(),
            compiler_version: model
                .rustc_version
                .lines()
                .next()
                .unwrap_or("rustc-unknown")
                .into(),
            completeness: completeness.clone(),
            coverage: coverage_label(&completeness).into(),
            certainty: certainty_label(&completeness).into(),
            obligations: obligation_codes(&completeness),
            incremental: full_execution_evidence(
                IncrementalPlan::Full {
                    reason: FullAnalysisReason::NoParent,
                },
                WorkerRequestCounters {
                    open_project_requests: 0,
                    index_files_requests: 0,
                },
                AnalysisExecutionAuthority::InProcessSyntax,
            ),
            incremental_receipt,
            repository_snapshot: snapshot_object.clone(),
            derived_input_manifest,
            generation: generation_object,
            query_index,
            transformed_source: None,
        };
        verify_ready(store, &ready, session, compilation, true)?;
        Ok(ready)
    })();
    match result {
        Ok(ready) => {
            journal.transition(AttemptState::Ready, ready.generation.digest.clone())?;
            if publish_head {
                let head_path = incremental_head_path(
                    &state.repository(&session.repository_path()?)?.root,
                    compilation,
                )?;
                publish_incremental_head(state, store, &head_path, &ready, &compiler_store.key)?;
            }
            write_private_atomic(state, binding_path, &ready)?;
            Ok(ready)
        }
        Err(error) => {
            journal.transition(AttemptState::Failed, "Rust generation finalization failed")?;
            Err(error)
        }
    }
}

fn rust_model_index(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    model: &CargoProjectModel,
    target: &CargoTargetModel,
    compilation_id: &str,
) -> Result<Value, ClewError> {
    build_syntax_index(
        store,
        snapshot,
        &RustSyntaxAuthority {
            compilation_id,
            model_digest: &model.model_digest,
            package: &target.selector.package,
            target_kind: &target.selector.target_kind,
            target_name: &target.selector.target_name,
            source_path: &target.source_path,
            cargo_version: &model.cargo_version,
            rustc_version: &model.rustc_version,
        },
    )
}

fn rust_syntax_completeness(scope_digest: &str) -> Result<CompletenessVector, ClewError> {
    let completeness = CompletenessVector {
        schema: COMPLETENESS_VECTOR_SCHEMA.into(),
        support: Support::Supported,
        coverage: Coverage::Partial {
            observed_scopes: vec![scope_digest.into()],
            boundaries: vec![
                "RUST_CFG_AND_MACRO_UNKNOWN".into(),
                "RUST_SYNTAX_ONLY".into(),
            ],
        },
        certainty: Certainty::Unsure {
            check_set: vec!["cargo-check".into(), "rust-name-resolution".into()],
        },
        obligations: vec![
            VerificationObligation {
                code: "VERIFY_CFG_AND_MACRO_EXPANSION".into(),
                subject: vec![scope_digest.into()],
                publication_blocking: true,
            },
            VerificationObligation {
                code: "VERIFY_RUST_NAME_RESOLUTION".into(),
                subject: vec![scope_digest.into()],
                publication_blocking: true,
            },
        ],
    };
    completeness.validate()?;
    Ok(completeness)
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
    let parent = binding_path
        .parent()
        .ok_or_else(|| corrupt("candidate generation binding has no parent"))?;
    if session.language == SessionLanguage::Python {
        let selectors = candidate
            .compilations
            .iter()
            .map(|value| PythonCompilationSelector::parse(value))
            .collect::<Result<Vec<_>, _>>()?;
        let source_roots = selectors
            .iter()
            .map(|selector| selector.source_root.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let (snapshot, snapshot_object) = capture_commit_scope(
            repository,
            candidate_revision,
            &source_roots,
            &store,
            |path| selectors.iter().any(|selector| selector.contains(path)),
            TrackedScopeLimits {
                max_files: MAX_PYTHON_SOURCE_FILES,
                max_file_bytes: MAX_PYTHON_SOURCE_FILE_BYTES,
                max_total_bytes: MAX_PYTHON_TOTAL_SOURCE_BYTES,
                max_tree_entries: 262_144,
                max_tree_bytes: 64 * 1024 * 1024,
                max_tree_path_bytes: 4096,
            },
        )?;
        return ensure_python_generation_set(
            &candidate,
            &state,
            &store,
            &snapshot,
            snapshot_object,
            parent,
            binding_path,
            "staged-generation-",
        );
    }
    let (snapshot, snapshot_object) = capture(repository, &store)?;
    if session.language == SessionLanguage::Rust {
        return ensure_rust_generation_set(
            &candidate,
            &state,
            &store,
            repository,
            &snapshot,
            snapshot_object,
            parent,
            binding_path,
            false,
            "staged-generation-",
        );
    }
    if session.language == SessionLanguage::Java {
        return Err(ClewError::new(
            ErrorCode::UnsupportedLanguage,
            "Java v1 is read-only and has no candidate generation path",
        ));
    }
    if session.language == SessionLanguage::JavaScript {
        return Err(ClewError::new(
            ErrorCode::UnsupportedLanguage,
            "JavaScript v1 is read-only and has no candidate generation path",
        ));
    }
    if session.language == SessionLanguage::TypeScript {
        return Err(ClewError::new(
            ErrorCode::UnsupportedLanguage,
            "TypeScript v1 is read-only and has no candidate generation path",
        ));
    }
    let pool = generation_pool(&candidate)?;
    let workspace =
        ProjectNativeKotlinWorkspace::prepare(&state, &store, &snapshot, &candidate.compilations)?;
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
        GenerationWorkspaceAuthority {
            base_revision: &candidate.base_revision,
            runtime_key: &candidate.runtime_key,
            session_authority_digest: &candidate.authority_digest,
            repository_snapshot: &snapshot_object,
        },
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

struct GenerationWorkspaceAuthority<'a> {
    base_revision: &'a str,
    runtime_key: &'a str,
    session_authority_digest: &'a str,
    repository_snapshot: &'a CasObject,
}

fn write_generation_workspace_evidence(
    state: &StateAuthority,
    path: &Path,
    compilation_count: usize,
    profile: ProjectNativeKotlinWorkspaceProfile,
    authority: GenerationWorkspaceAuthority<'_>,
) -> Result<(), ClewError> {
    if compilation_count == 0
        || digest_component(&profile.workspace_set_authority_digest).is_err()
        || digest_component(authority.runtime_key).is_err()
        || digest_component(authority.session_authority_digest).is_err()
        || authority.base_revision.is_empty()
        || authority.repository_snapshot.object_schema != SNAPSHOT_SCHEMA
        || profile.materializations != 1
        || profile.derived_mount_sets != 1
        || profile.workspace_set_authorizations != 1
        || profile.authorized_compilation_count != compilation_count as u64
        || profile.legacy_open_project_calls > compilation_count as u64
    {
        return Err(corrupt("project-native workspace profile is inconsistent"));
    }
    write_canonical_atomic(
        state,
        path,
        &GenerationWorkspaceEvidence {
            schema: WORKSPACE_PROFILE_SCHEMA.into(),
            base_revision: authority.base_revision.into(),
            compilation_count,
            materializations: profile.materializations,
            derived_mount_sets: profile.derived_mount_sets,
            repository_snapshot: authority.repository_snapshot.clone(),
            runtime_key: authority.runtime_key.into(),
            session_authority_digest: authority.session_authority_digest.into(),
            workspace_set_authority_digest: profile.workspace_set_authority_digest,
            workspace_set_authorizations: profile.workspace_set_authorizations,
            authorized_compilation_count: profile.authorized_compilation_count,
            legacy_open_project_calls: profile.legacy_open_project_calls,
            operational_timing: profile.operational_timing,
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
    let repository = if session.working_tree.is_some() {
        state.repository_by_key(&session.repository_key)?
    } else {
        state.repository(repo)?
    };
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
    let head_path = incremental_head_path(&repository.root, compilation)?;
    let preferred_engine = cached_engine_hint(
        &state,
        &store,
        &head_path,
        &runtime.runtime_key,
        compilation,
    );
    let live_attempt = workspace.open_compilation_from_set_with_hint(
        &state,
        compilation,
        digest_component(&compiler_namespace)?,
        external_build_state.as_deref(),
        preferred_engine,
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
        false,
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
        KOTLIN_ADAPTER_CONTRACT_ID,
        prepared.adapter_digest.clone(),
        &prepared.descriptor,
    )?;
    let head_lock_key = canonical::hash(&json!({
        "schema":"codeclew-incremental-head-lock/2.0",
        "repositoryKey":session.repository_key,
        "compilation":compilation,
    }))
    .map_err(internal)?;
    let _head_lock = GenerationLock::acquire(&state, &head_lock_key)?;
    let history_path = analysis_history_path(&repository.root, &prepared, &compiler_store)?;
    // Model admission is still fresh: only after OpenProject and complete current
    // derived authority may an older exact analysis replace another IndexFiles.
    // A -> B -> A must not lose A merely because B is now the incremental head.
    let history = load_content_analysis(
        &state,
        &store,
        &history_path,
        session,
        &prepared,
        &compiler_store,
    )?;
    let history_was_present = history.is_some();
    let archived = match history {
        Some(saved) => Some(saved),
        None => load_exact_analysis(
            &state,
            &store,
            &cache_path,
            session,
            compilation,
            &generation_key,
            &prepared,
            &compiler_store,
        )?,
    };
    let head = if archived.is_some() {
        IncrementalHeadState::Missing
    } else {
        load_incremental_head_for_planning(&state, &store, &head_path)?
    };
    let previous = head.ready();
    let (plan, unchanged_is_exact) = if let Some((_, receipt)) = &archived {
        (
            IncrementalPlan::UnchangedHit {
                parent_generation_id: receipt.generation_id.clone(),
            },
            true,
        )
    } else {
        match head.forced_full_plan() {
            Some(forced) => forced,
            None => incremental_plan_for(
                &store,
                snapshot,
                snapshot_object,
                &prepared,
                &compiler_store,
                previous,
            )?,
        }
    };
    let reusable = archived
        .as_ref()
        .map(|(ready, _)| ready)
        .or_else(|| previous.map(|value| &value.ready));
    let ready = if unchanged_is_exact {
        build_unchanged_ready(
            &state,
            &store,
            session,
            &runtime,
            snapshot_object.clone(),
            generation_key,
            &prepared,
            reusable.expect("exact unchanged analysis"),
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
    // Publish the content lookup before revision/session bindings. A crash after
    // this durable root must not make a saved analysis undiscoverable for another
    // revision. Keep the first verified result; later revisions only add their
    // own bindings, never another copy of its analysis payload.
    if !history_was_present {
        publish_incremental_head(&state, &store, &history_path, &ready, &compiler_store.key)?;
    }
    // NonCacheable governs model reuse, not a verified result after live model
    // admission. The managed repository root retains these exact CAS references.
    write_private_atomic(&state, &cache_path, &ready)?;
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
    let engine = KotlinSemanticEngine::from_analyzer_compiler_version(
        &prepared.semantic_engine.analyzer_compiler_version,
    )?;
    let semantic_output = Arc::new(Mutex::new(None));
    let cancellation = live_attempt.cancellation_handle();
    let driver = LiveKotlinDriver {
        attempt: Mutex::new(Some(live_attempt)),
        cancellation,
        store: store.clone(),
        output: Arc::clone(&semantic_output),
    };
    let adapter = KotlinAdapterV2::new(
        engine,
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
            compiler_version: prepared.semantic_engine.analyzer_compiler_version,
            completeness: completeness.clone(),
            coverage: coverage_label(&completeness).into(),
            certainty: certainty_label(&completeness).into(),
            obligations: obligation_codes(&completeness),
            incremental: full_execution_evidence(
                planned,
                semantic.worker_requests,
                AnalysisExecutionAuthority::CompilerWorker,
            ),
            incremental_receipt: incremental_receipt_object,
            repository_snapshot: snapshot_object,
            derived_input_manifest: prepared.derived_input_manifest,
            generation: generation_object,
            query_index: query_index_object,
            transformed_source: None,
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
    previous: &ReadyGeneration,
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
    let mut ready = previous.clone();
    ready.generation_key = generation_key;
    ready.runtime_key = runtime.runtime_key.clone();
    ready.base_revision = session.base_revision.clone();
    ready.compilation = prepared.compilation.clone();
    ready.compiler_version = prepared.semantic_engine.analyzer_compiler_version.clone();
    ready.repository_snapshot = snapshot_object;
    ready.derived_input_manifest = prepared.derived_input_manifest.clone();
    ready.incremental = IncrementalExecutionEvidence {
        schema: INCREMENTAL_EVIDENCE_SCHEMA.into(),
        planned,
        executed: IncrementalExecutionMode::UnchangedHit,
        analysis_execution_authority: AnalysisExecutionAuthority::CompilerWorker,
        compiler_output_receipt: None,
        subset_analysis_supported: false,
        worker_requests: counters,
    };
    ready.incremental_receipt = previous.incremental_receipt.clone();
    verify_ready(store, &ready, session, &ready.compilation, true)?;
    journal.transition(AttemptState::Ready, ready.generation.digest.clone())?;
    Ok(ready)
}

fn full_execution_evidence(
    planned: IncrementalPlan,
    worker_requests: WorkerRequestCounters,
    analysis_execution_authority: AnalysisExecutionAuthority,
) -> IncrementalExecutionEvidence {
    IncrementalExecutionEvidence {
        schema: INCREMENTAL_EVIDENCE_SCHEMA.into(),
        planned,
        executed: IncrementalExecutionMode::Full,
        analysis_execution_authority,
        compiler_output_receipt: None,
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
    let analyzer_compiler_version = project
        .get("analyzerCompilerVersion")
        .or_else(|| project.get("compilerVersion"))
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt("OpenProject has no Kotlin semantic engine identity"))?
        .to_owned();
    let engine = KotlinSemanticEngine::from_analyzer_compiler_version(&analyzer_compiler_version)?;
    let project_semantics = KotlinProjectSemantics::from_project_model(project)?;
    let semantic_engine = engine.authority();
    let worker = runtime.worker(engine.runtime_name())?;
    if worker.compiler_version != analyzer_compiler_version {
        return Err(corrupt("runtime worker compiler identity changed"));
    }
    let project_model_hash = required_digest(project, "projectModelHash")?;
    let semantic_input_manifest_hash = required_digest(project, "semanticInputManifestHash")?;
    let toolchain = store.put(
        "codeclew-kotlin-toolchain-authority/2.0",
        &canonical::bytes(worker).map_err(internal)?,
    )?;
    let options = store.put(
        "codeclew-project-native-options/3.0",
        &canonical::bytes(&json!({
            "nativeCompilation":compilation,
            "projectSemantics":&project_semantics,
            "semanticEngine":&semantic_engine,
            "semanticAuthorityDigest":project_semantics.authority_digest(engine)?,
        }))
        .map_err(internal)?,
    )?;
    let model = store.put(
        "codeclew-project-native-model/2.0",
        &canonical::bytes(&json!({
            "schema":"codeclew-project-native-model/2.0",
            "snapshotId":snapshot.snapshot_id,
            "compilation":compilation,
            "projectSemantics":&project_semantics,
            "semanticEngine":&semantic_engine,
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
        project_semantics,
        semantic_engine,
        adapter_digest: kotlin_adapter_digest(&worker.tree_hash)?,
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
    let engine = KotlinSemanticEngine::from_analyzer_compiler_version(
        &prepared.semantic_engine.analyzer_compiler_version,
    )?;
    let worker = runtime.worker(engine.runtime_name())?;
    if prepared.schema != PREPARED_AUTHORITY_SCHEMA
        || prepared.runtime_key != runtime.runtime_key
        || prepared.repository_snapshot != *snapshot
        || prepared.compilation != compilation
        || prepared.adapter_digest != kotlin_adapter_digest(&worker.tree_hash)?
        || prepared.semantic_engine != engine.authority()
        || prepared.project_semantics.schema
            != crate::kotlin_engine::KOTLIN_PROJECT_SEMANTICS_SCHEMA
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
    writable_then_seal: bool,
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-generation-key/2.3",
        "runtimeKey":runtime_key,
        "baseRevision":base_revision,
        "snapshot":snapshot,
        "compilation":compilation,
        "derivedInputManifest":derived_input_manifest,
        "writableThenSeal":writable_then_seal,
    }))
    .map_err(internal)
}

/// Public test-only wrapper so integration tests can assert that the semantic
/// generation identity distinguishes writable-then-seal from read-only for
/// otherwise identical inputs (no-op transformation). Not part of product API.
#[doc(hidden)]
pub fn final_generation_key_for_test(
    compilation: &str,
    writable_then_seal: bool,
) -> Result<String, ClewError> {
    let snapshot = CasObject {
        schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
        object_schema: crate::repository_snapshot::SNAPSHOT_SCHEMA.into(),
        digest: format!("sha256:{}", "a".repeat(64)),
        size: 1,
    };
    let derived = CasObject {
        schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
        object_schema: crate::derived_manifest::DERIVED_MANIFEST_SCHEMA.into(),
        digest: format!("sha256:{}", "b".repeat(64)),
        size: 1,
    };
    final_generation_key(
        &format!("sha256:{}", "c".repeat(64)),
        &format!("sha256:{}", "d".repeat(64)),
        &snapshot,
        compilation,
        &derived,
        writable_then_seal,
    )
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

const FACT_RUN_BATCH_FACTS: usize = 256;

enum FactRunTarget {
    Direct(Box<FactRunWriter>),
    Channel(crossbeam_channel::Sender<Vec<FactRecord>>),
}

struct StreamingAnalysisSink<'a> {
    target: FactRunTarget,
    buffer: Vec<FactRecord>,
    fact_count: u64,
    completion: Option<AnalysisAttemptComplete>,
    cancelled: &'a std::sync::atomic::AtomicBool,
}

fn check_analysis_cancelled(cancelled: &std::sync::atomic::AtomicBool) -> Result<(), ClewError> {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err(resource("analysis fact stream cancelled"));
    }
    Ok(())
}

impl StreamingAnalysisSink<'_> {
    fn flush(&mut self) -> Result<(), ClewError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let FactRunTarget::Channel(sender) = &self.target else {
            return Err(internal("direct fact writer unexpectedly buffered facts"));
        };
        let mut batch = std::mem::take(&mut self.buffer);
        loop {
            check_analysis_cancelled(self.cancelled)?;
            match sender.send_timeout(batch, std::time::Duration::from_millis(50)) {
                Ok(()) => return Ok(()),
                Err(crossbeam_channel::SendTimeoutError::Timeout(pending)) => batch = pending,
                Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                    return Err(internal("analysis fact writers stopped"));
                }
            }
        }
    }

    fn seal(&mut self, completion: &AnalysisAttemptComplete) -> Result<(), ClewError> {
        check_analysis_cancelled(self.cancelled)?;
        if self.completion.as_ref() != Some(completion)
            || completion.fact_count != self.fact_count
            || self.fact_count == 0
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "adapter completion differs from the spooled fact stream",
            ));
        }
        self.flush()
    }
}

impl AnalysisSink for StreamingAnalysisSink<'_> {
    fn accept(&mut self, event: AnalysisEvent) -> Result<(), ClewError> {
        check_analysis_cancelled(self.cancelled)?;
        if self.completion.is_some() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "adapter emitted data after completion",
            ));
        }
        match event {
            AnalysisEvent::FactShard(shard) => {
                for fact in shard.facts {
                    check_analysis_cancelled(self.cancelled)?;
                    match &mut self.target {
                        FactRunTarget::Direct(writer) => writer.push(&fact)?,
                        FactRunTarget::Channel(_) => {
                            self.buffer.push(fact);
                            if self.buffer.len() == FACT_RUN_BATCH_FACTS {
                                self.flush()?;
                            }
                        }
                    }
                    self.fact_count = self
                        .fact_count
                        .checked_add(1)
                        .ok_or_else(|| resource("analysis fact count overflow"))?;
                }
            }
            AnalysisEvent::AttemptComplete(completion) => self.completion = Some(completion),
        }
        Ok(())
    }
}

struct AnalysisDagResult {
    completion: AnalysisAttemptComplete,
    runs: Vec<FactRun>,
}

/// Spool the already sorted, conformance-checked stream into private runs. The
/// returned runs become eligible for finalization only after completion is sealed.
fn stream_analysis_runs(
    state: &StateAuthority,
    jobs: usize,
    cancelled: &std::sync::atomic::AtomicBool,
    analyze: impl FnOnce(&mut dyn AnalysisSink) -> Result<AnalysisAttemptComplete, ClewError>,
) -> Result<AnalysisDagResult, ClewError> {
    if jobs == 0 {
        return Err(resource("analysis fact writer count is empty"));
    }
    if jobs == 1 {
        let mut sink = StreamingAnalysisSink {
            target: FactRunTarget::Direct(Box::new(FactRunWriter::create(state)?)),
            buffer: Vec::new(),
            fact_count: 0,
            completion: None,
            cancelled,
        };
        let completion = analyze(&mut sink)?;
        sink.seal(&completion)?;
        let FactRunTarget::Direct(writer) = sink.target else {
            unreachable!()
        };
        return Ok(AnalysisDagResult {
            completion,
            runs: vec![writer.finish()?],
        });
    }
    // Reserve one admitted CPU for the adapter/producer; the others normalize
    // and write runs concurrently. The queue is bounded by twice the consumers.
    std::thread::scope(|scope| {
        let writers = jobs - 1;
        let (sender, receiver) = crossbeam_channel::bounded::<Vec<FactRecord>>(writers * 2);
        let mut handles = Vec::with_capacity(writers);
        for partition in 0..writers {
            let receiver = receiver.clone();
            handles.push(
                std::thread::Builder::new()
                    .name(format!("clew-fact-run-{partition}"))
                    .spawn_scoped(scope, move || -> Result<FactRun, ClewError> {
                        let mut writer = FactRunWriter::create(state)?;
                        for batch in receiver {
                            for fact in batch {
                                check_analysis_cancelled(cancelled)?;
                                writer.push(&fact)?;
                            }
                        }
                        check_analysis_cancelled(cancelled)?;
                        writer.finish()
                    })
                    .map_err(io_error)?,
            );
        }
        drop(receiver);
        let produced = {
            let mut sink = StreamingAnalysisSink {
                target: FactRunTarget::Channel(sender),
                buffer: Vec::new(),
                fact_count: 0,
                completion: None,
                cancelled,
            };
            analyze(&mut sink).and_then(|completion| {
                sink.seal(&completion)?;
                Ok(completion)
            })
            // Dropping the sender lets every writer exit, including after failure.
        };
        let results = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| internal("analysis fact writer panicked"))?
            })
            .collect::<Vec<Result<_, ClewError>>>();
        let written = results.into_iter().collect::<Result<Vec<_>, ClewError>>();
        let completion = produced?;
        Ok(AnalysisDagResult {
            completion,
            runs: written?,
        })
    })
}

fn execute_analysis_dag_with_jobs(
    state: &StateAuthority,
    registry: Arc<AdapterRegistry>,
    request: AnalyzeGenerationRequest,
    resources: HostResources,
    jobs: usize,
) -> Result<AnalysisDagResult, ClewError> {
    if jobs == 0 || jobs > resources.logical_cpu {
        return Err(resource("analysis job count exceeds the host authority"));
    }
    let analysis = Arc::new(Mutex::new(None::<AnalysisDagResult>));
    let worker_rss = resources
        .codeclew_memory_budget_bytes
        .clamp(1, 2 * 1024 * 1024 * 1024 + jobs as u64 * 8 * 1024 * 1024);
    let stages = vec![StageSpec {
        id: "adapter-analysis".into(),
        dependencies: Vec::new(),
        resources: ResourceDescriptor {
            class: "language-adapter".into(),
            min_rss_bytes: 1,
            expected_rss_bytes: worker_rss,
            max_rss_bytes: worker_rss,
            min_cpu: jobs,
            max_cpu: jobs,
            max_instances: 1,
            exclusivity_key: Some(format!(
                "compiler-store-{}",
                digest_component(&request.compilation.toolchain.digest)?
            )),
        },
        operation_uri: "core:adapter-analysis".into(),
        input: Value::Null,
    }];
    let observer = Arc::new(CompositeProgress::new(vec![
        Arc::new(PersistentProgress::open(state, &request.attempt_id)?),
        Arc::new(StderrProgress),
    ])?);
    let scheduler = DagScheduler::new(resources, observer)?;
    let attempt_id = request.attempt_id.clone();
    let state_for_executor = state.clone();
    let analysis_for_executor = Arc::clone(&analysis);
    let report = scheduler.execute(
        DagPlan {
            schema: DAG_SCHEMA.into(),
            stages,
        },
        move |stage, cancelled| {
            if stage.operation_uri != "core:adapter-analysis" {
                return Err(corrupt("cold-start DAG contains an unknown operation"));
            }
            let result = stream_analysis_runs(&state_for_executor, jobs, cancelled, |sink| {
                registry.analyze_generation_into(&request, sink, cancelled)
            })?;
            let output = json!({
                "factCount":result.completion.fact_count, "sealedCompilerStreams":1,
                "factRunWriters":jobs.saturating_sub(1).max(1),
                "maxQueuedFactBatches":if jobs == 1 { 0 } else { (jobs - 1) * 2 },
                "maxFactsPerBatch":FACT_RUN_BATCH_FACTS,
            });
            *analysis_for_executor.lock().map_err(poisoned)? = Some(result);
            Ok(output)
        },
    )?;
    crate::cold_start::persist_dag_report(state, &attempt_id, &report)?;
    analysis
        .lock()
        .map_err(poisoned)?
        .take()
        .ok_or_else(|| corrupt("adapter DAG produced no sealed fact runs"))
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
    if let Some(previous) = previous.filter(|previous| {
        exact_generation_authority(
            &previous.ready.repository_snapshot,
            &previous.ready.derived_input_manifest,
            &previous.receipt.compiler_store_key,
            snapshot_object,
            &prepared.derived_input_manifest,
            &compiler_store.key,
        )
    }) {
        return Ok((
            IncrementalPlan::UnchangedHit {
                parent_generation_id: previous.receipt.generation_id.clone(),
            },
            true,
        ));
    }
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

fn exact_generation_authority(
    previous_snapshot: &CasObject,
    previous_derived_input_manifest: &CasObject,
    previous_compiler_store_key: &str,
    current_snapshot: &CasObject,
    current_derived_input_manifest: &CasObject,
    current_compiler_store_key: &str,
) -> bool {
    previous_snapshot == current_snapshot
        && previous_derived_input_manifest == current_derived_input_manifest
        && previous_compiler_store_key == current_compiler_store_key
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

    let mut symbol_identities = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    let mut callable_aliases = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    let mut surfaces = BTreeMap::<String, Vec<Value>>::new();
    for descriptor in descriptors {
        let path = required_safe_path(descriptor, "file")?;
        surfaces
            .entry(path.clone())
            .or_default()
            .push(descriptor.clone());
        if let Some(identity) = descriptor.get("symbolIdentity").and_then(Value::as_str) {
            symbol_identities
                .entry(identity.into())
                .or_default()
                .insert(path.clone());
        }
        if let Some(callable) = descriptor.get("compilerCallableId").and_then(Value::as_str) {
            callable_aliases
                .entry(callable.into())
                .or_default()
                .insert(path.clone());
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
        let Some(targets) = symbol_identities
            .get(target_symbol)
            .or_else(|| callable_aliases.get(target_symbol))
        else {
            continue;
        };
        let Some(relation_digest) = targets
            .iter()
            .any(|target| target != &source)
            .then(|| canonical::hash(relation).map_err(internal))
            .transpose()?
        else {
            continue;
        };
        for target in targets.iter().filter(|target| *target != &source) {
            dependencies
                .entry(source.clone())
                .or_default()
                .insert(target.clone());
            boundary_rows
                .entry((source.clone(), target.clone()))
                .or_default()
                .push(relation_digest.clone());
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
    Ok(Some(LoadedIncrementalHead { receipt, ready }))
}

/// An old result may suggest which worker to start, never which model to trust.
/// Keep this optional read small and independent of the analysis closure: the
/// ordinary live OpenProject still qualifies and can switch the selected engine.
fn cached_engine_hint(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    runtime_key: &str,
    compilation: &str,
) -> Option<KotlinSemanticEngine> {
    const MAX_HINT_BYTES: usize = 64 * 1024;
    let bytes = state.read_private_file(path, MAX_HINT_BYTES).ok()?;
    let head: IncrementalHead = serde_json::from_slice(&bytes).ok()?;
    if canonical::bytes(&head).ok()? != bytes
        || head.schema != INCREMENTAL_HEAD_SCHEMA
        || head.ready.object_schema != READY_GENERATION_SCHEMA
        || head.ready.size > MAX_HINT_BYTES as u64
    {
        return None;
    }
    let lease = store.read(&head.ready, MAX_HINT_BYTES).ok()?;
    let ready: ReadyGeneration = serde_json::from_slice(lease.bytes()).ok()?;
    if canonical::bytes(&ready).ok()? != lease.bytes()
        || ready.schema != READY_GENERATION_SCHEMA
        || ready.runtime_key != runtime_key
        || ready.compilation != compilation
    {
        return None;
    }
    KotlinSemanticEngine::from_analyzer_compiler_version(&ready.compiler_version).ok()
}

/// Reuse an exact historical analysis only after the caller obtained current
/// OpenProject authority. This never authorizes a project-model cache hit.
#[allow(clippy::too_many_arguments)]
fn load_exact_analysis(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    session: &SessionAuthority,
    compilation: &str,
    generation_key: &str,
    prepared: &PreparedGenerationAuthority,
    compiler_store: &CompilerStoreKey,
) -> Result<Option<(ReadyGeneration, IncrementalReceipt)>, ClewError> {
    if !state.private_file_exists(path)? {
        return Ok(None);
    }
    let ready = load_ready(state, store, path, session, compilation, true)?;
    let receipt: IncrementalReceipt = read_canonical_object(store, &ready.incremental_receipt)?;
    if ready.generation_key != generation_key
        || ready.compiler_version != prepared.semantic_engine.analyzer_compiler_version
        || ready.incremental.analysis_execution_authority
            != AnalysisExecutionAuthority::CompilerWorker
        || ready.transformed_source.is_some()
        || !exact_generation_authority(
            &ready.repository_snapshot,
            &ready.derived_input_manifest,
            &receipt.compiler_store_key,
            &prepared.repository_snapshot,
            &prepared.derived_input_manifest,
            &compiler_store.key,
        )
    {
        return Err(corrupt(
            "saved analysis differs from current exact OpenProject authority",
        ));
    }
    Ok(Some((ready, receipt)))
}

/// Repository-scoped lookup for complete computational inputs. Revision remains
/// part of the published generation binding, not the immutable analysis lookup.
fn analysis_history_path(
    repository_root: &Path,
    prepared: &PreparedGenerationAuthority,
    compiler_store: &CompilerStoreKey,
) -> Result<std::path::PathBuf, ClewError> {
    let key = canonical::hash(&json!({
        "schema":"codeclew-native-analysis-history-key/1.0",
        "runtimeKey":prepared.runtime_key,
        "compilation":prepared.compilation,
        "compilerVersion":prepared.semantic_engine.analyzer_compiler_version,
        "repositorySnapshot":prepared.repository_snapshot,
        "derivedInputManifest":prepared.derived_input_manifest,
        "compilerStoreKey":compiler_store.key,
        "writableSurface":false,
    }))
    .map_err(internal)?;
    Ok(repository_root
        .join("generations/analysis")
        .join(format!("{}.json", digest_component(&key)?)))
}

fn load_content_analysis(
    state: &StateAuthority,
    store: &CasStore,
    path: &Path,
    session: &SessionAuthority,
    prepared: &PreparedGenerationAuthority,
    compiler_store: &CompilerStoreKey,
) -> Result<Option<(ReadyGeneration, IncrementalReceipt)>, ClewError> {
    // This validates the old revision binding and its complete immutable closure
    // against their own authority. Current session verification happens only
    // after build_unchanged_ready creates the new revision binding.
    let Some(saved) = load_incremental_head(state, store, path)? else {
        return Ok(None);
    };
    let ready = saved.ready;
    if ready.runtime_key != session.runtime_key
        || ready.runtime_key != prepared.runtime_key
        || ready.compilation != prepared.compilation
        || !session.compilations.contains(&ready.compilation)
        || ready.compiler_version != prepared.semantic_engine.analyzer_compiler_version
        || ready.incremental.analysis_execution_authority
            != AnalysisExecutionAuthority::CompilerWorker
        || ready.transformed_source.is_some()
        || !exact_generation_authority(
            &ready.repository_snapshot,
            &ready.derived_input_manifest,
            &saved.receipt.compiler_store_key,
            &prepared.repository_snapshot,
            &prepared.derived_input_manifest,
            &compiler_store.key,
        )
    {
        return Err(corrupt(
            "saved analysis differs from current complete OpenProject authority",
        ));
    }
    Ok(Some((ready, saved.receipt)))
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
    // The set's transformed-source authority is a per-compilation concern:
    // distinct compilation manifests and equal payload sharing are both valid,
    // and consumers select the source authority by compilation rather than
    // flattening conflicting paths. The set-level field is only a convenience
    // aggregation (first present) used when all compilations share it; each
    // ReadyGeneration.transformed_source remains authoritative for its own
    // compilation. We do not reject distinct per-compilation authority here.
    let transformed_source = compilations
        .iter()
        .find_map(|ready| ready.transformed_source.clone());
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
        transformed_source,
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
        || ready.compilations.len() > crate::limits::MAX_SELECTED_COMPILATIONS
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
        || ready
            .transformed_source
            .as_ref()
            .is_some_and(|reference| reference.object_schema != TRANSFORMED_SOURCE_SCHEMA)
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
                ready.transformed_source.is_some(),
            )?
    {
        return Err(corrupt("ready generation authority is invalid"));
    }
    ready.completeness.validate()?;
    match (
        ready.incremental.executed.clone(),
        ready.incremental.analysis_execution_authority,
    ) {
        (IncrementalExecutionMode::Full, AnalysisExecutionAuthority::CompilerWorker)
            if ready.incremental.worker_requests.open_project_requests != 1
                || ready.incremental.worker_requests.index_files_requests == 0 =>
        {
            return Err(corrupt(
                "full compiler generation request counters are invalid",
            ));
        }
        (IncrementalExecutionMode::Full, AnalysisExecutionAuthority::InProcessSyntax)
            if ready.incremental.worker_requests.open_project_requests != 0
                || ready.incremental.worker_requests.index_files_requests != 0 =>
        {
            return Err(corrupt(
                "in-process generation request counters are invalid",
            ));
        }
        (
            IncrementalExecutionMode::Full,
            AnalysisExecutionAuthority::CompilerProcess
            | AnalysisExecutionAuthority::CompilerOutputCheckpoint,
        ) if ready.incremental.worker_requests.open_project_requests != 0
            || ready.incremental.worker_requests.index_files_requests != 0 =>
        {
            return Err(corrupt(
                "compiler-process generation request counters are invalid",
            ));
        }
        (IncrementalExecutionMode::UnchangedHit, authority)
            if authority != AnalysisExecutionAuthority::CompilerWorker
                || !matches!(
                    ready.incremental.planned,
                    IncrementalPlan::UnchangedHit { .. }
                )
                || ready.incremental.worker_requests.open_project_requests != 1
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
    if let Some(reference) = &ready.incremental.compiler_output_receipt {
        if !matches!(
            ready.incremental.analysis_execution_authority,
            AnalysisExecutionAuthority::CompilerProcess
                | AnalysisExecutionAuthority::CompilerOutputCheckpoint
        ) || ready.incremental.executed != IncrementalExecutionMode::Full
        {
            return Err(corrupt(
                "Java raw execution receipt has another execution authority",
            ));
        }
        let descriptor = derived
            .provider_models
            .iter()
            .flat_map(|provider| &provider.build_model.compilations)
            .find(|descriptor| {
                descriptor.compilation_id == safe_compilation_id(&ready.compilation)
                    && descriptor.language_uri.as_str() == JAVA_LANGUAGE
            })
            .ok_or_else(|| corrupt("Java raw execution receipt has no current Java compilation"))?;
        let options: serde_json::Value =
            read_canonical_object(store, &descriptor.canonical_options)?;
        let input: CasObject = serde_json::from_value(
            options
                .get("compilerInput")
                .cloned()
                .ok_or_else(|| corrupt("current Java options omit compiler input"))?,
        )
        .map_err(|_| corrupt("current Java compiler input is invalid"))?;
        let closed: CasObject = serde_json::from_value(
            options
                .get("analysisInputs")
                .cloned()
                .ok_or_else(|| corrupt("current Java options omit sealed inputs"))?,
        )
        .map_err(|_| corrupt("current sealed Java input is invalid"))?;
        crate::java_analyzer_output::verify_input_binding(
            store,
            &input,
            &closed,
            &ready.compilation,
        )?;
        let _ = crate::java_analyzer_output::verify_receipt(store, reference, &input)?;
    } else if ready.incremental.analysis_execution_authority
        == AnalysisExecutionAuthority::CompilerOutputCheckpoint
        || derived
            .provider_models
            .iter()
            .flat_map(|provider| &provider.build_model.compilations)
            .any(|descriptor| {
                descriptor.compilation_id == safe_compilation_id(&ready.compilation)
                    && descriptor.language_uri.as_str() == JAVA_LANGUAGE
                    && descriptor.canonical_options.object_schema
                        == "codeclew-java-analysis-options/1.0"
            })
    {
        return Err(corrupt(
            "Java checkpoint projection omits original execution receipt",
        ));
    }
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
    use crate::runtime::{RUNTIME_SCHEMA, RuntimeMode, RuntimeWorker};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn non_prepared_incremental_evidence_omits_java_raw_receipt() {
        let non_prepared = full_execution_evidence(
            IncrementalPlan::Full {
                reason: FullAnalysisReason::NoParent,
            },
            WorkerRequestCounters {
                open_project_requests: 0,
                index_files_requests: 0,
            },
            AnalysisExecutionAuthority::CompilerProcess,
        );
        let bytes = canonical::bytes(&non_prepared).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("compilerOutputReceipt").is_none());
        let decoded: IncrementalExecutionEvidence = serde_json::from_slice(&bytes).unwrap();
        assert!(decoded.compiler_output_receipt.is_none());
        assert_eq!(canonical::bytes(&decoded).unwrap(), bytes);
    }

    fn compiled_output_model(path: &Path) -> JavaOperationalModel {
        use crate::java_project_model::{JavaBuildSystem, JavaProjectModel, classpath_authority};
        JavaOperationalModel {
            authority: JavaProjectModel {
                schema: JAVA_MODEL_SCHEMA.into(),
                model_digest: String::new(),
                build_system: JavaBuildSystem::Maven,
                compilation: "java:maven:module:main".into(),
                source_files: Vec::new(),
                classpath: vec![classpath_authority(path).unwrap()],
                release: 17,
                compiler_version: "javac 17".into(),
                compiler_options: Vec::new(),
                annotation_processors: Vec::new(),
                annotation_processor_paths: Vec::new(),
                boundaries: Vec::new(),
            },
            source_paths: Vec::new(),
            classpath_paths: vec![path.to_owned()],
            annotation_processor_paths: Vec::new(),
            java_executable: PathBuf::from("java"),
        }
    }

    #[test]
    fn compiled_outputs_keep_distinct_modules_separate_and_share_equal_content() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("module-one/target/classes");
        let second = root.path().join("module-two/target/classes");
        let same = root.path().join("module-copy/target/classes");
        for (path, bytes) in [(&first, b"first"), (&second, b"other"), (&same, b"first")] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("Type.class"), bytes).unwrap();
        }
        let mut a = compiled_output_model(&first);
        let mut b = compiled_output_model(&second);
        let mut c = compiled_output_model(&same);
        preserve_compiled_classes(root.path(), &mut a).unwrap();
        preserve_compiled_classes(root.path(), &mut b).unwrap();
        assert_ne!(a.classpath_paths, b.classpath_paths);
        assert_eq!(
            std::fs::read(a.classpath_paths[0].join("Type.class")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(b.classpath_paths[0].join("Type.class")).unwrap(),
            b"other"
        );
        preserve_compiled_classes(root.path(), &mut c).unwrap();
        assert_eq!(a.classpath_paths, c.classpath_paths);
        assert_eq!(
            std::fs::read_dir(root.path().join("analysis-classes"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn compiled_outputs_reject_changed_source_and_corrupt_retained_copy() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("module/target/classes");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("Type.class"), b"admitted").unwrap();
        let mut model = compiled_output_model(&source);
        let mut another = model.clone();
        std::fs::write(source.join("Type.class"), b"changed").unwrap();
        assert_eq!(
            preserve_compiled_classes(root.path(), &mut model)
                .unwrap_err()
                .code,
            ErrorCode::InputMutated
        );
        std::fs::write(source.join("Type.class"), b"admitted").unwrap();
        preserve_compiled_classes(root.path(), &mut model).unwrap();
        std::fs::write(model.classpath_paths[0].join("Type.class"), b"corrupt").unwrap();
        assert_eq!(
            preserve_compiled_classes(root.path(), &mut another)
                .unwrap_err()
                .code,
            ErrorCode::InputMutated
        );
    }

    #[test]
    fn analyzer_engine_identity_is_separate_from_adapter_contract() {
        assert_eq!(
            KotlinSemanticEngine::from_analyzer_compiler_version("2.3.0").unwrap(),
            KotlinSemanticEngine::Kotlin23,
        );
        assert_eq!(
            KotlinSemanticEngine::from_analyzer_compiler_version("2.4.10").unwrap(),
            KotlinSemanticEngine::Kotlin24,
        );
        assert_eq!(KOTLIN_ADAPTER_CONTRACT_ID, "kotlin-semantic-facts");
    }

    #[test]
    fn writable_then_seal_profile_matches_gate_string() {
        assert_eq!(
            JAVA_MAVEN_WRITABLE_THEN_SEAL_PROFILE,
            "java-17plus-maven-writable-then-seal"
        );
    }

    #[test]
    fn wants_writable_then_seal_gates_on_profile() {
        assert!(wants_writable_then_seal(
            JAVA_MAVEN_WRITABLE_THEN_SEAL_PROFILE
        ));
        assert!(!wants_writable_then_seal("java-17plus-maven-read-only"));
        assert!(!wants_writable_then_seal("java-17plus-gradle-read-only"));
    }

    #[test]
    fn transformed_java_source_digests_indexes_only_model_sources() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::create_dir_all(root.path().join("target")).unwrap();
        std::fs::write(root.path().join("src/A.java"), "v1").unwrap();
        std::fs::write(root.path().join("target/gen.java"), "generated").unwrap();

        // Only the model source set is indexed; target/ is excluded.
        let sources = vec!["src/A.java".to_string()];
        let digests = transformed_java_source_digests(root.path(), &sources).unwrap();
        assert_eq!(digests.len(), 1);
        assert!(digests.contains_key("src/A.java"));
        assert!(!digests.contains_key("target/gen.java"));
        assert_eq!(digests["src/A.java"], canonical::hash_bytes(b"v1"));

        // Modified content produces a different digest.
        std::fs::write(root.path().join("src/A.java"), "v2").unwrap();
        let digests = transformed_java_source_digests(root.path(), &sources).unwrap();
        assert_eq!(digests["src/A.java"], canonical::hash_bytes(b"v2"));
        assert_ne!(digests["src/A.java"], canonical::hash_bytes(b"v1"));

        // A model source absent after transformation is a hard error.
        let error = transformed_java_source_digests(root.path(), &["src/Missing.java".to_string()])
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InputMutated);
    }

    #[test]
    fn exact_generation_authority_requires_every_match() {
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        let object = |schema: &str, character: char| CasObject {
            schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
            object_schema: schema.into(),
            digest: digest(character),
            size: 1,
        };
        let snapshot = object(SNAPSHOT_SCHEMA, '1');
        let derived = object(DERIVED_MANIFEST_SCHEMA, '2');
        let store_key = digest('3');
        assert!(exact_generation_authority(
            &snapshot, &derived, &store_key, &snapshot, &derived, &store_key,
        ));
        assert!(!exact_generation_authority(
            &snapshot,
            &derived,
            &store_key,
            &object(SNAPSHOT_SCHEMA, '4'),
            &derived,
            &store_key,
        ));
        assert!(!exact_generation_authority(
            &snapshot,
            &derived,
            &store_key,
            &snapshot,
            &object(DERIVED_MANIFEST_SCHEMA, '5'),
            &store_key,
        ));
        assert!(!exact_generation_authority(
            &snapshot,
            &derived,
            &store_key,
            &snapshot,
            &derived,
            &digest('6'),
        ));
    }

    fn runtime(runtime_key: &str, binary_byte: u8) -> RuntimeAuthority {
        RuntimeAuthority {
            schema: RUNTIME_SCHEMA.into(),
            runtime_key: format!("sha256:{}", runtime_key.repeat(64)),
            mode: RuntimeMode::Release,
            manifest_digest: format!("sha256:{}", runtime_key.repeat(64)),
            components: BTreeMap::from([(
                "clew".into(),
                format!("sha256:{}", runtime_key.repeat(64)),
            )]),
            artifacts: BTreeMap::from([(
                "clew".into(),
                crate::runtime::RuntimeArtifact {
                    mode: 0o111,
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
                analysis_execution_authority: AnalysisExecutionAuthority::CompilerWorker,
                compiler_output_receipt: None,
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
            transformed_source: None,
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
            transformed_source: None,
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
    fn streamed_runs_are_private_until_completion_and_removed_on_failure_or_cancel() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/fact/1", b"payload").unwrap();
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let facts = (0..4096)
            .map(|index| FactRecord {
                fact_key: format!("stream:{index:06}"),
                domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
                payload: payload.clone(),
            })
            .collect::<Vec<_>>();
        let completion = AnalysisAttemptComplete {
            scope_digest: format!("sha256:{}", "a".repeat(64)),
            completeness_receipt: receipt,
            fact_count: facts.len() as u64,
        };
        let run_root = root.path().join("v2/attempts/fact-runs");
        for jobs in [1, 4] {
            for failure in ["producer", "count", "missing-completion", "cancel"] {
                let cancelled = AtomicBool::new(false);
                let result = stream_analysis_runs(&state, jobs, &cancelled, |sink| {
                    sink.accept(AnalysisEvent::FactShard(crate::adapter_v2::FactShard {
                        sequence: 0,
                        facts: facts.clone(),
                    }))?;
                    // More than the queue capacity has been consumed before any receipt exists.
                    assert!(
                        std::fs::read_dir(&run_root)
                            .unwrap()
                            .any(|entry| { entry.unwrap().metadata().unwrap().len() > 0 })
                    );
                    if failure == "producer" {
                        return Err(internal("fixture producer failed"));
                    }
                    if failure == "cancel" {
                        cancelled.store(true, Ordering::Release);
                    }
                    let mut emitted = completion.clone();
                    if failure == "count" {
                        emitted.fact_count += 1;
                    }
                    if failure != "missing-completion" {
                        sink.accept(AnalysisEvent::AttemptComplete(emitted.clone()))?;
                    }
                    Ok(emitted)
                });
                assert!(result.is_err(), "{failure} with {jobs} jobs must fail");
                assert_eq!(
                    std::fs::read_dir(&run_root).unwrap().count(),
                    0,
                    "{failure} leaked private runs"
                );
            }
        }
    }

    #[test]
    fn streamed_runs_match_a_serial_reference_across_batch_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/fact/1", b"payload").unwrap();
        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let completion = AnalysisAttemptComplete {
            scope_digest: format!("sha256:{}", "b".repeat(64)),
            completeness_receipt: store.put("test/receipt/1", b"complete").unwrap(),
            fact_count: 4097,
        };
        let facts = (0..completion.fact_count)
            .map(|index| FactRecord {
                fact_key: format!("stream:{index:06}"),
                domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
                payload: payload.clone(),
            })
            .collect::<Vec<_>>();
        let mut reference_writer = FactRunWriter::create(&state).unwrap();
        for fact in &facts {
            reference_writer.push(fact).unwrap();
        }
        let authority = |completion| {
            vec![AttemptAuthority {
                compilation_id: "main".into(),
                capability: CapabilityUri::parse("analysis:test").unwrap(),
                completion,
            }]
        };
        let reference = finalize_generation(
            &store,
            derived.clone(),
            authority(completion.clone()),
            vec![reference_writer.finish().unwrap()],
        )
        .unwrap();
        for jobs in [1, 4] {
            let result = stream_analysis_runs(&state, jobs, &AtomicBool::new(false), |sink| {
                for (sequence, chunk) in facts.chunks(777).enumerate() {
                    sink.accept(AnalysisEvent::FactShard(crate::adapter_v2::FactShard {
                        sequence: sequence as u32,
                        facts: chunk.to_vec(),
                    }))?;
                }
                sink.accept(AnalysisEvent::AttemptComplete(completion.clone()))?;
                Ok(completion.clone())
            })
            .unwrap();
            let actual = finalize_generation(
                &store,
                derived.clone(),
                authority(result.completion),
                result.runs,
            )
            .unwrap();
            assert_eq!(actual, reference);
        }
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
            false,
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
            false,
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
                false,
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

        let evidence = full_execution_evidence(
            planned.clone(),
            requests,
            AnalysisExecutionAuthority::CompilerWorker,
        );

        assert_eq!(evidence.planned, planned);
        assert_eq!(evidence.executed, IncrementalExecutionMode::Full);
        assert!(!evidence.subset_analysis_supported);
        assert_eq!(evidence.worker_requests, requests);
    }

    #[test]
    fn workspace_profile_is_private_canonical_and_rejects_impossible_counts() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let repository_snapshot = store.put(SNAPSHOT_SCHEMA, b"snapshot").unwrap();
        let path = state.root().join("sessions/test/workspace-profile.json");
        write_generation_workspace_evidence(
            &state,
            &path,
            12,
            ProjectNativeKotlinWorkspaceProfile {
                materializations: 1,
                derived_mount_sets: 1,
                workspace_set_authority_digest: format!("sha256:{}", "a".repeat(64)),
                workspace_set_authorizations: 1,
                authorized_compilation_count: 12,
                legacy_open_project_calls: 1,
                operational_timing: Some(ProjectNativeKotlinOperationalTiming {
                    materialization_micros: Some(3),
                    derived_mount_micros: Some(5),
                    final_unmount_micros: Some(7),
                    final_verification_micros: Some(11),
                    disposal_micros: Some(13),
                    compilations: BTreeMap::from([(
                        ":/main".into(),
                        crate::kotlin_adapter_v2::ProjectNativeKotlinCompilationTiming {
                            initial_worker_startup_micros: Some(17),
                            open_project_wall_micros: Some(19),
                            open_project_physical_requests: Some(2),
                            open_project_logical_requests: Some(1),
                            open_project_profile: None,
                            index_profile: None,
                            final_shutdown_micros: Some(23),
                            final_input_verification_micros: Some(29),
                        },
                    )]),
                }),
            },
            GenerationWorkspaceAuthority {
                base_revision: "base",
                runtime_key: &format!("sha256:{}", "b".repeat(64)),
                session_authority_digest: &format!("sha256:{}", "c".repeat(64)),
                repository_snapshot: &repository_snapshot,
            },
        )
        .unwrap();
        let value: GenerationWorkspaceEvidence =
            serde_json::from_slice(&state.read_private_file(&path, MAX_BINDING_BYTES).unwrap())
                .unwrap();
        assert_eq!(value.schema, WORKSPACE_PROFILE_SCHEMA);
        assert_eq!(value.base_revision, "base");
        assert_eq!(value.compilation_count, 12);
        assert_eq!(value.repository_snapshot, repository_snapshot);
        assert_eq!(value.runtime_key, format!("sha256:{}", "b".repeat(64)));
        assert_eq!(
            value.session_authority_digest,
            format!("sha256:{}", "c".repeat(64))
        );
        assert_eq!(value.workspace_set_authorizations, 1);
        assert_eq!(value.authorized_compilation_count, 12);
        assert_eq!(value.legacy_open_project_calls, 1);
        assert_eq!(
            value
                .operational_timing
                .as_ref()
                .and_then(|timing| timing.materialization_micros),
            Some(3)
        );
        assert_eq!(
            value
                .operational_timing
                .as_ref()
                .and_then(|timing| timing.compilations.get(":/main"))
                .and_then(|timing| timing.open_project_physical_requests),
            Some(2)
        );

        let mut old_value = serde_json::to_value(&value).unwrap();
        old_value
            .as_object_mut()
            .unwrap()
            .remove("operationalTiming");
        let old_value: GenerationWorkspaceEvidence =
            serde_json::from_value(old_value).expect("legacy evidence without timing");
        assert!(old_value.operational_timing.is_none());

        let error = write_generation_workspace_evidence(
            &state,
            &path,
            12,
            ProjectNativeKotlinWorkspaceProfile {
                materializations: 2,
                derived_mount_sets: 1,
                workspace_set_authority_digest: format!("sha256:{}", "a".repeat(64)),
                workspace_set_authorizations: 1,
                authorized_compilation_count: 12,
                legacy_open_project_calls: 13,
                operational_timing: None,
            },
            GenerationWorkspaceAuthority {
                base_revision: "base",
                runtime_key: &format!("sha256:{}", "b".repeat(64)),
                session_authority_digest: &format!("sha256:{}", "c".repeat(64)),
                repository_snapshot: &repository_snapshot,
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
            adapter_id: KOTLIN_ADAPTER_CONTRACT_ID.into(),
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
                {"path":"src/B.kt","contentHash":digest('2')},
                {"path":"src/C.kt","contentHash":digest('3')}
            ],
            "declarationDescriptors":{"descriptors":[
                {"file":"src/A.kt","symbolIdentity":"callable:p/shared.read#jvm:()V","compilerCallableId":"p/shared.read"},
                {"file":"src/B.kt","symbolIdentity":"callable:p/shared.read#jvm:()I","compilerCallableId":"p/shared.read"},
                {"file":"src/C.kt","symbolIdentity":"callable:p/shared.read#jvm:()S","compilerCallableId":"p/shared.read"}
            ]},
            "declarationRelations":{"relations":[
                {"file":"src/A.kt","owner":"p/A.call","target":"p/shared.read","kind":"CALLS"}
            ]}
        });
        let completeness = CompletenessVector::verified_complete(digest('9')).unwrap();
        let (receipt, reference) =
            create_incremental_receipt(&store, &index, &compiler_store, &generation, completeness)
                .unwrap();
        assert_eq!(receipt.files[0].dependencies, vec!["src/B.kt", "src/C.kt"]);
        assert_eq!(receipt.boundaries.len(), 2);
        assert_eq!(receipt.boundaries[0].source_path, "src/A.kt");
        assert_eq!(receipt.boundaries[0].target_path, "src/B.kt");
        assert_eq!(receipt.boundaries[1].source_path, "src/A.kt");
        assert_eq!(receipt.boundaries[1].target_path, "src/C.kt");

        let mut exact_identity = index.clone();
        exact_identity["declarationRelations"]["relations"][0]["target"] =
            Value::String("callable:p/shared.read#jvm:()I".into());
        let (exact_receipt, _) = create_incremental_receipt(
            &store,
            &exact_identity,
            &compiler_store,
            &generation,
            CompletenessVector::verified_complete(digest('7')).unwrap(),
        )
        .unwrap();
        assert_eq!(exact_receipt.files[0].dependencies, vec!["src/B.kt"]);
        assert_eq!(exact_receipt.boundaries.len(), 1);
        assert_eq!(exact_receipt.boundaries[0].target_path, "src/B.kt");

        let mut reordered = index.clone();
        reordered["declarationDescriptors"]["descriptors"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let (_, reordered_reference) = create_incremental_receipt(
            &store,
            &reordered,
            &compiler_store,
            &generation,
            CompletenessVector::verified_complete(digest('9')).unwrap(),
        )
        .unwrap();
        assert_eq!(reordered_reference.digest, reference.digest);

        let mut duplicate_identity = exact_identity.clone();
        duplicate_identity["declarationDescriptors"]["descriptors"][2]["symbolIdentity"] =
            duplicate_identity["declarationDescriptors"]["descriptors"][1]["symbolIdentity"]
                .clone();
        let (duplicate_receipt, _) = create_incremental_receipt(
            &store,
            &duplicate_identity,
            &compiler_store,
            &generation,
            CompletenessVector::verified_complete(digest('8')).unwrap(),
        )
        .unwrap();
        assert_eq!(
            duplicate_receipt.files[0].dependencies,
            vec!["src/B.kt", "src/C.kt"]
        );

        let mut namespace_collision = exact_identity.clone();
        namespace_collision["declarationDescriptors"]["descriptors"][2]["compilerCallableId"] =
            Value::String("callable:p/shared.read#jvm:()I".into());
        let (collision_receipt, _) = create_incremental_receipt(
            &store,
            &namespace_collision,
            &compiler_store,
            &generation,
            CompletenessVector::verified_complete(digest('6')).unwrap(),
        )
        .unwrap();
        assert_eq!(collision_receipt.files[0].dependencies, vec!["src/B.kt"]);

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

    #[test]
    fn build_added_java_sources_have_only_transformed_authority() {
        let (_root, store, repo, original_path) = transformed_fixture();
        let generated_path = "src/main/java/example/Generated.java";
        let original = b"package example; public class Service {}";
        let transformed = b"package example; public class Service { Generated value; }";
        let generated = b"package example; public class Generated {}";
        let original_object = store.put("source-file/1.0", original).unwrap();
        let snapshot_sources = BTreeMap::from([(original_path.to_owned(), original_object)]);
        std::fs::write(repo.join(original_path), transformed).unwrap();
        std::fs::write(repo.join(generated_path), generated).unwrap();
        let mut model = compiled_output_model(&repo.join(original_path));
        model.authority.source_files = vec![generated_path.into(), original_path.into()];

        let readonly = java_source_content_digests(&store, &snapshot_sources, &model).unwrap_err();
        assert_eq!(readonly.code, ErrorCode::InputMutated);
        assert!(readonly.message.contains("writable-then-seal"));
        let before =
            original_java_source_content_digests(&store, &snapshot_sources, &model).unwrap();
        let after = transformed_java_source_digests(&repo, &model.authority.source_files).unwrap();
        assert_eq!(
            before,
            BTreeMap::from([(original_path.into(), canonical::hash_bytes(original))])
        );
        assert!(
            !before.contains_key(generated_path),
            "new files have no original snapshot digest"
        );
        assert_eq!(after.len(), 2);
        assert_eq!(after[original_path], canonical::hash_bytes(transformed));
        assert_eq!(after[generated_path], canonical::hash_bytes(generated));
        let source_state = json!({
            "kind":"TRANSFORMED_WORKSPACE", "before":before, "after":after,
            "changedFiles":[{"path":original_path,"before":before[original_path],"after":after[original_path]}],
        });
        let reference = persist_transformed_source(
            &store,
            &repo,
            &model.authority.source_files,
            Some("TRANSFORMED_WORKSPACE"),
            Some(&source_state),
        )
        .unwrap()
        .unwrap();
        let reopened = load_transformed_source(&store, &reference).unwrap();
        assert_eq!(reopened[original_path], transformed);
        assert_eq!(reopened[generated_path], generated);
        std::fs::write(repo.join(generated_path), b"changed after capture").unwrap();
        assert_eq!(
            persist_transformed_source(
                &store,
                &repo,
                &model.authority.source_files,
                Some("TRANSFORMED_WORKSPACE"),
                Some(&source_state),
            )
            .unwrap_err()
            .code,
            ErrorCode::InputMutated
        );
        assert_eq!(
            load_transformed_source(&store, &reference).unwrap(),
            reopened
        );

        // A file that did exist originally remains required in the original
        // CAS closure: allowing added files must not swallow corrupt originals.
        let mut corrupt_originals = snapshot_sources.clone();
        corrupt_originals.get_mut(original_path).unwrap().digest =
            format!("sha256:{}", "f".repeat(64));
        assert!(original_java_source_content_digests(&store, &corrupt_originals, &model).is_err());
    }

    #[test]
    fn transformed_source_persists_exact_bytes_and_round_trips() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join("src/main/java/example")).unwrap();
        let service_text = b"package example;\npublic class Service { void m() {} }\n";
        let unchanged_text = b"package example;\npublic class Unchanged {}\n";
        std::fs::write(
            repo.join("src/main/java/example/Service.java"),
            service_text,
        )
        .unwrap();
        std::fs::write(
            repo.join("src/main/java/example/Unchanged.java"),
            unchanged_text,
        )
        .unwrap();

        // The authoritative after-map must carry real byte digests and cover
        // exactly the admitted source files.
        let source_state = json!({
            "kind": "TRANSFORMED_WORKSPACE",
            "before": {"src/main/java/example/Service.java": canonical::hash_bytes(service_text)},
            "after": {
                "src/main/java/example/Service.java": canonical::hash_bytes(service_text),
                "src/main/java/example/Unchanged.java": canonical::hash_bytes(unchanged_text),
            },
            "changedFiles": [],
        });
        let reference = persist_transformed_source(
            &store,
            &repo,
            &[
                "src/main/java/example/Service.java".into(),
                "src/main/java/example/Unchanged.java".into(),
            ],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&source_state),
        )
        .unwrap()
        .expect("transformed source must persist");

        assert_eq!(reference.object_schema, TRANSFORMED_SOURCE_SCHEMA);
        let loaded = load_transformed_source(&store, &reference).unwrap();
        assert_eq!(
            loaded["src/main/java/example/Service.java"],
            b"package example;\npublic class Service { void m() {} }\n"
        );
        assert_eq!(
            loaded["src/main/java/example/Unchanged.java"],
            b"package example;\npublic class Unchanged {}\n"
        );

        // Read-only generations persist nothing.
        let none = persist_transformed_source(
            &store,
            &repo,
            &["src/main/java/example/Service.java".into()],
            None,
            None,
        )
        .unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn transformed_source_rejects_foreign_or_malformed_reference() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let foreign = store.put("some-other-schema", b"x").unwrap();
        let error = load_transformed_source(&store, &foreign).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn chained_lifecycle_persists_reopens_and_consumes_transformed_source() {
        // Real transform-to-persist-to-reopen-to-documentation chain: a
        // writable-then-seal build transforms source (line inserted), the real
        // CAS persistence writes the exact transformed bytes, reopen reads them
        // back unchanged, and the real documentation consumer labels them
        // TRANSFORMED_SOURCE (no original link) rather than read-only
        // EXACT_SNAPSHOT_TEXT. Each stage calls the real product code.
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let repo = root.path().join("repo");
        let relative = "src/main/java/example/Service.java";
        let file = repo.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = "package example;\npublic class Service { void m() {} }\n";
        // Synthetic writable transform: a line is inserted before the class, so
        // the declaration moves from line 2 to line 3 of the transformed bytes.
        let transformed = "package example;\n\npublic class Service { void m() {} }\n";
        std::fs::write(&file, transformed).unwrap();

        let source_state = json!({
            "kind": "TRANSFORMED_WORKSPACE",
            "before": {relative: "old"},
            "after": {relative: canonical::hash_bytes(transformed.as_bytes())},
            "changedFiles": [],
        });
        let reference = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&source_state),
        )
        .unwrap()
        .expect("writable-then-seal provenance must persist transformed authority");
        let loaded = load_transformed_source(&store, &reference).unwrap();
        assert_eq!(
            String::from_utf8(loaded[relative].clone()).unwrap(),
            transformed,
            "reopened persisted bytes must equal the exact indexed transformed bytes"
        );

        // A read-only provenance persists nothing, so a stale writable
        // generation can never be reused as readonly authority.
        assert!(
            persist_transformed_source(&store, &repo, &[relative.into()], None, None)
                .unwrap()
                .is_none()
        );

        // Real documentation consumer reads the persisted transformed bytes and
        // labels them TRANSFORMED_SOURCE with no original-commit link.
        let service: crate::documentation::model::Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0", "id":"svc", "title":"S",
            "repositoryId":"svc", "repository":"https://example.invalid/svc",
            "language":"java", "profile":"java-17plus-maven-writable-then-seal",
            "compilation":":/main", "targetRef":"main",
            "sourceLinkTemplate":"{repository}/blob/{revision}/{file}",
        }))
        .unwrap();
        let declaration = json!({
            "kind":"DECLARATION","symbolIdentity":"example.Service",
            "ownerIdentity":"example","name":"Service","file":relative,
            "startLine":3,"endLine":3,"resolution":"RESOLVED",
            "documentation":{"events":[]},
        });
        let facts = vec![(declaration, "binding-digest".into())];
        let transformed_files = BTreeMap::from([(
            relative.to_string(),
            String::from_utf8(loaded[relative].clone()).unwrap(),
        )]);
        let transformed_evidence = crate::documentation::analysis::project(
            &service,
            &"a".repeat(40),
            &crate::documentation::digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            facts.clone(),
            &transformed_files,
            true,
        )
        .unwrap();
        let transformed_src = transformed_evidence.sources.values().next().unwrap();
        assert_eq!(
            transformed_src.authority, TRANSFORMED_SOURCE_AUTHORITY,
            "persisted transformed bytes must be consumed as TRANSFORMED_SOURCE"
        );
        assert!(
            transformed_src.url.is_none(),
            "transformed source must omit the original-commit link"
        );
        assert!(
            transformed_src.text.contains("public class Service"),
            "consumer must read the persisted transformed bytes, not the original snapshot"
        );

        // The same project read-only consumes the original bytes as
        // EXACT_SNAPSHOT_TEXT with an original-commit link. The read-only
        // consumer uses the original snapshot coordinates (line 2), not the
        // transformed coordinates (line 3).
        let original_declaration = json!({
            "kind":"DECLARATION","symbolIdentity":"example.Service",
            "ownerIdentity":"example","name":"Service","file":relative,
            "startLine":2,"endLine":2,"resolution":"RESOLVED",
            "documentation":{"events":[]},
        });
        let original_facts = vec![(original_declaration, "binding-digest".into())];
        let original_files = BTreeMap::from([(relative.to_string(), original.to_string())]);
        let original_evidence = crate::documentation::analysis::project(
            &service,
            &"a".repeat(40),
            &crate::documentation::digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            original_facts,
            &original_files,
            false,
        )
        .unwrap();
        let original_src = original_evidence.sources.values().next().unwrap();
        assert_eq!(original_src.authority, "EXACT_SNAPSHOT_TEXT");
        assert!(
            original_src
                .url
                .as_deref()
                .is_some_and(|url| url.contains("/blob/")),
            "read-only source must keep its original-commit link: {:?}",
            original_src.url
        );
    }

    fn transformed_fixture() -> (
        tempfile::TempDir,
        CasStore,
        std::path::PathBuf,
        &'static str,
    ) {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let repo = root.path().join("repo");
        let relative = "src/main/java/example/Service.java";
        std::fs::create_dir_all(repo.join(relative).parent().unwrap()).unwrap();
        (root, store, repo, relative)
    }

    fn transformed_state(_relative: &str, after: serde_json::Value) -> serde_json::Value {
        json!({"kind": "TRANSFORMED_WORKSPACE", "before": {}, "after": after, "changedFiles": []})
    }

    /// Build an outer well-formed transformed-source manifest referencing
    /// `files` and carrying `source_state`, so reopen integrity can be tested
    /// with a deliberately inconsistent payload.
    fn put_transformed_manifest(
        store: &CasStore,
        files: &BTreeMap<String, CasObject>,
        source_state: &serde_json::Value,
    ) -> CasObject {
        store
            .put(
                TRANSFORMED_SOURCE_SCHEMA,
                &canonical::bytes(&json!({
                    "schema": TRANSFORMED_SOURCE_SCHEMA,
                    "kind": "TRANSFORMED_WORKSPACE",
                    "provenance": "TRANSFORMED_WORKSPACE",
                    "sourceState": source_state,
                    "files": files,
                }))
                .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn readonly_and_noop_writable_generation_keys_differ() {
        // The existing writable flag already separates read-only from a no-op
        // writable-then-seal generation for otherwise identical inputs. Retain
        // and prove that separation rather than redesigning the key.
        let readonly = final_generation_key_for_test(":main", false).unwrap();
        let writable = final_generation_key_for_test(":main", true).unwrap();
        assert_ne!(readonly, writable);
        assert_eq!(
            final_generation_key_for_test(":main", false).unwrap(),
            readonly,
            "equivalent read-only inputs must be stable"
        );
    }

    #[test]
    fn two_indexed_after_states_reject_incompatible_and_keep_previous_binding() {
        let (_root, store, repo, relative) = transformed_fixture();
        let bytes: &[u8] = b"package example;\npublic class Service {}\n";
        std::fs::write(repo.join(relative), bytes).unwrap();
        let after_a = json!({relative: canonical::hash_bytes(bytes)});
        let reference = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&transformed_state(relative, after_a)),
        )
        .unwrap()
        .expect("first after-state must persist");
        // A second indexed after-state for the same original snapshot whose
        // digest does not match the persisted bytes must fail before any
        // publication, leaving the previous binding usable and unchanged.
        let after_b =
            json!({relative: canonical::hash_bytes(b"package example;\npublic class Other {}\n")});
        let err = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&transformed_state(relative, after_b)),
        )
        .unwrap_err();
        assert!(err.message.contains("changed after index-state"), "{err}");
        assert_eq!(
            load_transformed_source(&store, &reference).unwrap()[relative],
            bytes,
            "previous binding must remain unchanged and usable"
        );
    }

    #[test]
    fn persist_rejects_missing_and_extra_source_state_paths() {
        let (_root, store, repo, relative) = transformed_fixture();
        let bytes: &[u8] = b"package example;\npublic class Service {}\n";
        std::fs::write(repo.join(relative), bytes).unwrap();
        // Missing: the after-map does not cover the admitted source file.
        let missing = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&transformed_state(relative, json!({}))),
        )
        .unwrap_err();
        assert!(
            missing.message.contains("do not exactly match"),
            "{missing}"
        );
        // Extra: an after entry names a path that is not admitted.
        let extra = json!({
            relative: canonical::hash_bytes(bytes),
            "src/Unadmitted.java": canonical::hash_bytes(b"x"),
        });
        let err = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&transformed_state(relative, extra)),
        )
        .unwrap_err();
        assert!(err.message.contains("do not exactly match"), "{err}");
    }

    #[test]
    fn persist_rejects_bytes_changed_since_index_state_construction() {
        let (_root, store, repo, relative) = transformed_fixture();
        // The authoritative after-map records the digest of the intended bytes,
        // but the materialized file differs: a transform mutated source after
        // index-state construction.
        let declared: &[u8] = b"package example;\npublic class Service { void m() {} }\n";
        let on_disk: &[u8] = b"package example;\npublic class Service {}\n";
        std::fs::write(repo.join(relative), on_disk).unwrap();
        let after = json!({relative: canonical::hash_bytes(declared)});
        let err = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&transformed_state(relative, after)),
        )
        .unwrap_err();
        assert!(err.message.contains("changed after index-state"), "{err}");
    }

    #[test]
    fn persist_rejects_unsafe_source_path() {
        let (_root, store, repo, _relative) = transformed_fixture();
        let unsafe_path = "../escape/Service.java";
        std::fs::create_dir_all(repo.join("..").join("escape")).unwrap();
        std::fs::write(repo.join(unsafe_path), b"x").unwrap();
        let after = json!({unsafe_path: canonical::hash_bytes(b"x")});
        let err = persist_transformed_source(
            &store,
            &repo,
            &[unsafe_path.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&transformed_state(unsafe_path, after)),
        )
        .unwrap_err();
        assert!(err.message.contains("safe relative path"), "{err}");
    }

    #[test]
    fn load_rejects_legacy_manifest_without_source_state() {
        let (_root, store, _repo, relative) = transformed_fixture();
        // A well-formed outer manifest with no integrity source state is a
        // legacy artifact and must be rejected for trusted reuse, not promoted.
        let file = store
            .put(TRANSFORMED_SOURCE_FILE_SCHEMA, b"package example;")
            .unwrap();
        let mut files = BTreeMap::new();
        files.insert(relative.to_string(), file);
        let manifest = put_transformed_manifest(&store, &files, &json!({}));
        let err = load_transformed_source(&store, &manifest).unwrap_err();
        assert!(
            err.message.contains("integrity source state")
                || err.message.contains("kind is invalid"),
            "{err}"
        );
    }

    #[test]
    fn load_rejects_manifest_without_authoritative_after_map() {
        let (_root, store, _repo, relative) = transformed_fixture();
        // kind present but `after` absent: no authoritative digest map to trust.
        let file = store
            .put(TRANSFORMED_SOURCE_FILE_SCHEMA, b"package example;")
            .unwrap();
        let mut files = BTreeMap::new();
        files.insert(relative.to_string(), file);
        let source_state = json!({"kind": "TRANSFORMED_WORKSPACE", "before": {}});
        let manifest = put_transformed_manifest(&store, &files, &source_state);
        let err = load_transformed_source(&store, &manifest).unwrap_err();
        assert!(err.message.contains("authoritative after map"), "{err}");
    }

    #[test]
    fn load_rejects_wellformed_manifest_with_state_file_mismatch() {
        let (_root, store, _repo, relative) = transformed_fixture();
        // The outer manifest is well-formed, but the persisted file bytes do not
        // match the authoritative after digest carried in the same manifest.
        let persisted: &[u8] = b"package example;\npublic class Service {}\n";
        let declared: &[u8] = b"package example;\npublic class Other {}\n";
        let file = store
            .put(TRANSFORMED_SOURCE_FILE_SCHEMA, persisted)
            .unwrap();
        let mut files = BTreeMap::new();
        files.insert(relative.to_string(), file);
        let after = json!({relative: canonical::hash_bytes(declared)});
        let manifest =
            put_transformed_manifest(&store, &files, &transformed_state(relative, after));
        let err = load_transformed_source(&store, &manifest).unwrap_err();
        assert!(
            err.message
                .contains("hash does not match indexed source state"),
            "{err}"
        );
    }

    #[test]
    fn load_rejects_manifest_with_extra_file_beyond_indexed_state() {
        let (_root, store, _repo, relative) = transformed_fixture();
        // files set and after-map membership must match exactly; an extra file
        // beyond the indexed state is a mismatch, not trusted evidence.
        let extra_path = "src/Extra.java";
        let persisted: &[u8] = b"package example;\npublic class Service {}\n";
        let file = store
            .put(TRANSFORMED_SOURCE_FILE_SCHEMA, persisted)
            .unwrap();
        let mut files = BTreeMap::new();
        files.insert(relative.to_string(), file.clone());
        files.insert(extra_path.to_string(), file);
        let after = json!({relative: canonical::hash_bytes(persisted)});
        let manifest =
            put_transformed_manifest(&store, &files, &transformed_state(relative, after));
        let err = load_transformed_source(&store, &manifest).unwrap_err();
        assert!(
            err.message.contains("do not match indexed source state"),
            "{err}"
        );
    }

    #[test]
    fn load_rejects_unsafe_path_in_reopened_manifest() {
        let (_root, store, _repo, _relative) = transformed_fixture();
        let unsafe_path = "../escape/Service.java";
        let file = store
            .put(TRANSFORMED_SOURCE_FILE_SCHEMA, b"package example;")
            .unwrap();
        let mut files = BTreeMap::new();
        files.insert(unsafe_path.to_string(), file);
        let after = json!({unsafe_path: canonical::hash_bytes(b"package example;")});
        let manifest =
            put_transformed_manifest(&store, &files, &transformed_state(unsafe_path, after));
        let err = load_transformed_source(&store, &manifest).unwrap_err();
        assert!(err.message.contains("path is unsafe"), "{err}");
    }

    #[test]
    fn stable_equivalent_state_round_trips_deterministically() {
        let (_root, store, repo, relative) = transformed_fixture();
        let bytes: &[u8] = b"package example;\npublic class Service {}\n";
        std::fs::write(repo.join(relative), bytes).unwrap();
        let after = json!({relative: canonical::hash_bytes(bytes)});
        let state = transformed_state(relative, after);
        let first = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&state),
        )
        .unwrap()
        .expect("first persist");
        let second = persist_transformed_source(
            &store,
            &repo,
            &[relative.into()],
            Some("TRANSFORMED_WORKSPACE"),
            Some(&state),
        )
        .unwrap()
        .expect("second persist");
        assert_eq!(
            first, second,
            "equivalent stable state must be deterministic"
        );
        assert_eq!(
            load_transformed_source(&store, &first).unwrap()[relative],
            bytes,
            "exact state must round-trip"
        );
    }
}

#[cfg(test)]
#[path = "generation_reuse_tests.rs"]
mod reuse_tests;
