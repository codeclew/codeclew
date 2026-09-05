use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::kotlin_engine::{KOTLIN_ADAPTER_CONTRACT_ID, KotlinSemanticEngine};
use crate::repository_snapshot::{
    RepositoryInputSnapshot, capture, capture_ignoring_derived_mounts, materialize,
};
use crate::state::{ManagedTemporaryDirectory, StateAuthority, create_private_directory};
use crate::worker::{WorkerClient, WorkerRequestCounters, workspace_root};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

pub const KOTLIN_LANGUAGE: &str = "language:kotlin";
pub const KOTLIN_FACTS_CAPABILITY: &str = "analysis:kotlin-semantic-facts";
const FACT_PAYLOAD_SCHEMA: &str = "codeclew-kotlin-semantic-fact/3.0";
const TRANSLATION_AUTHORITY_SCHEMA: &str = "codeclew-kotlin-fact-translation/3.3";
const RECEIPT_SCHEMA: &str = "codeclew-completeness-receipt/2.0";
const WORKSPACE_SET_AUTHORIZATION_SCHEMA: &str = "codeclew-kotlin-workspace-set-authorization/1.0";

pub(crate) fn kotlin_adapter_digest(worker_tree_hash: &str) -> Result<String, ClewError> {
    require_digest(worker_tree_hash)?;
    canonical::hash(&json!({
        "schema": TRANSLATION_AUTHORITY_SCHEMA,
        "workerTreeHash": worker_tree_hash,
        "factPayloadSchema": FACT_PAYLOAD_SCHEMA,
    }))
    .map_err(internal)
}

pub trait KotlinGenerationDriver: Send + Sync {
    fn analyze(&self, request: &AnalyzeGenerationRequest) -> Result<Value, ClewError>;

    fn cancel(&self) -> Result<(), ClewError> {
        Ok(())
    }
}

pub struct KotlinAdapterV2<D> {
    engine: KotlinSemanticEngine,
    adapter_digest: String,
    toolchain_digest: String,
    store: CasStore,
    driver: D,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl<D> KotlinAdapterV2<D> {
    pub fn new(
        engine: KotlinSemanticEngine,
        adapter_digest: String,
        toolchain_digest: String,
        store: CasStore,
        driver: D,
    ) -> Result<Self, ClewError> {
        require_digest(&adapter_digest)?;
        require_digest(&toolchain_digest)?;
        Ok(Self {
            engine,
            adapter_digest,
            toolchain_digest,
            store,
            driver,
            cancelled_attempts: Mutex::new(BTreeSet::new()),
            stopped: AtomicBool::new(false),
        })
    }
}

impl<D: KotlinGenerationDriver> LanguageAdapter for KotlinAdapterV2<D> {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
        Ok(AdapterHandshake {
            protocol: ADAPTER_PROTOCOL.into(),
            adapter_id: KOTLIN_ADAPTER_CONTRACT_ID.into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(KOTLIN_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?],
            toolchains: vec![ToolchainConstraint {
                authority_digest: self.toolchain_digest.clone(),
                minimum_version: Some(self.engine.analyzer_compiler_version().into()),
                maximum_version_exclusive: None,
            }],
        })
    }

    fn analyze_generation(
        &self,
        request: &AnalyzeGenerationRequest,
        sink: &mut dyn AnalysisSink,
        cancelled: &AtomicBool,
    ) -> Result<(), ClewError> {
        if self.stopped.load(Ordering::Acquire)
            || cancelled.load(Ordering::Acquire)
            || self
                .cancelled_attempts
                .lock()
                .map_err(poisoned)?
                .contains(&request.attempt_id)
        {
            return Err(cancelled_error());
        }
        if request.compilation.language_uri.as_str() != KOTLIN_LANGUAGE
            || request.capability.as_str() != KOTLIN_FACTS_CAPABILITY
            || request.compilation.toolchain.digest != self.toolchain_digest
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "Kotlin adapter request does not match its exact language/toolchain authority",
            ));
        }
        let index = self.driver.analyze(request)?;
        if cancelled.load(Ordering::Acquire)
            || self
                .cancelled_attempts
                .lock()
                .map_err(poisoned)?
                .contains(&request.attempt_id)
        {
            return Err(cancelled_error());
        }
        if index
            .get("analyzerCompilerVersion")
            .or_else(|| index.get("compilerVersion"))
            .and_then(Value::as_str)
            != Some(self.engine.analyzer_compiler_version())
            || index.get("k2Validated").and_then(Value::as_bool) != Some(true)
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "Kotlin worker result differs from the selected compiler authority",
            ));
        }
        let facts = translate_facts(&self.store, &index)?;
        let fact_count = facts.len() as u64;
        for (sequence, chunk) in facts.chunks(1024).enumerate() {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: u32::try_from(sequence).map_err(|_| {
                    ClewError::new(
                        ErrorCode::ResourceLimit,
                        "Kotlin fact shard sequence overflow",
                    )
                })?,
                facts: chunk.to_vec(),
            }))?;
        }
        let scope_digest = semantic_scope_digest(&index)?;
        let receipt = completeness_receipt(&self.store, &index, &scope_digest)?;
        sink.accept(AnalysisEvent::AttemptComplete(AnalysisAttemptComplete {
            scope_digest,
            completeness_receipt: receipt,
            fact_count,
        }))
    }

    fn cancel(&self, attempt_id: &str) -> Result<(), ClewError> {
        if attempt_id.is_empty() || attempt_id.len() > 128 {
            return Err(invalid("Kotlin attempt identity is invalid"));
        }
        self.cancelled_attempts
            .lock()
            .map_err(poisoned)?
            .insert(attempt_id.into());
        self.driver.cancel()
    }

    fn shutdown(&self) -> Result<(), ClewError> {
        self.stopped.store(true, Ordering::Release);
        self.driver.cancel()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProjectNativeKotlinWorkspaceProfile {
    pub(crate) materializations: u64,
    pub(crate) derived_mount_sets: u64,
    pub(crate) workspace_set_authority_digest: String,
    pub(crate) workspace_set_authorizations: u64,
    pub(crate) authorized_compilation_count: u64,
    pub(crate) legacy_open_project_calls: u64,
}

pub(crate) struct ProjectNativeKotlinWorkspace {
    store: CasStore,
    snapshot: RepositoryInputSnapshot,
    _attempt_root: ManagedTemporaryDirectory,
    repo: std::path::PathBuf,
    derived_mounts: Vec<std::path::PathBuf>,
    preparation_profile: ProjectNativeKotlinWorkspaceProfile,
    authorized_compilations: BTreeSet<String>,
    issued_compilations: Mutex<BTreeSet<String>>,
    model_extraction_gate: Mutex<()>,
    legacy_open_project_calls: AtomicU64,
}

pub(crate) struct ProjectNativeKotlinAttempt {
    store: CasStore,
    snapshot: RepositoryInputSnapshot,
    repo: std::path::PathBuf,
    derived_mounts: Vec<std::path::PathBuf>,
    worker: Option<WorkerClient>,
    request: Value,
    project: Value,
}

impl ProjectNativeKotlinWorkspace {
    pub(crate) fn prepare(
        state: &StateAuthority,
        store: &CasStore,
        snapshot: &RepositoryInputSnapshot,
        compilations: &[String],
    ) -> Result<Self, ClewError> {
        Self::prepare_language(state, store, snapshot, compilations, "kotlin")
    }

    pub(crate) fn prepare_language(
        state: &StateAuthority,
        store: &CasStore,
        snapshot: &RepositoryInputSnapshot,
        compilations: &[String],
        language: &str,
    ) -> Result<Self, ClewError> {
        if compilations.is_empty()
            || compilations
                .windows(2)
                .any(|pair| pair[0].as_str() >= pair[1].as_str())
            || !matches!(language, "kotlin" | "java")
        {
            return Err(invalid(
                "workspace-set compilations must be non-empty, sorted, and unique",
            ));
        }
        let authorized_compilations = compilations.iter().cloned().collect::<BTreeSet<_>>();
        let workspace_set_authority_digest = canonical::hash(&json!({
            "schema":WORKSPACE_SET_AUTHORIZATION_SCHEMA,
            "compilations":compilations,
            "language":language,
            "providerMode":"PROJECT_NATIVE_LEGACY_BRIDGE",
            "repositorySnapshot":snapshot.snapshot_id,
        }))
        .map_err(internal)?;
        let attempt_root = state
            .directory(std::path::Path::new("attempts"))?
            .temporary_child("kotlin-generation-set")?;
        let attempt_path = attempt_root.directory().resolved_path()?;
        let repo = attempt_path.join("repo");
        let mut preparation_profile = ProjectNativeKotlinWorkspaceProfile::default();
        materialize(snapshot, store, &repo)?;
        preparation_profile.materializations += 1;
        let derived_mounts = mount_project_derived_state(&attempt_path, &repo, snapshot)?;
        preparation_profile.derived_mount_sets += 1;
        preparation_profile.workspace_set_authority_digest = workspace_set_authority_digest;
        preparation_profile.workspace_set_authorizations = 1;
        preparation_profile.authorized_compilation_count =
            u64::try_from(compilations.len()).map_err(internal)?;
        Ok(Self {
            store: store.clone(),
            snapshot: snapshot.clone(),
            _attempt_root: attempt_root,
            repo,
            derived_mounts,
            preparation_profile,
            authorized_compilations,
            issued_compilations: Mutex::new(BTreeSet::new()),
            model_extraction_gate: Mutex::new(()),
            legacy_open_project_calls: AtomicU64::new(0),
        })
    }

    pub(crate) fn repository(&self) -> &std::path::Path {
        &self.repo
    }

    pub(crate) fn open_compilation_from_set(
        &self,
        state: &StateAuthority,
        native_compilation: &str,
        compiler_store_component: &str,
        build_state_root: Option<&std::path::Path>,
    ) -> Result<ProjectNativeKotlinAttempt, ClewError> {
        self.open_compilation_with_engine(
            state,
            native_compilation,
            compiler_store_component,
            build_state_root,
            None,
        )
    }

    #[cfg(test)]
    fn open_qualification_compilation_from_set(
        &self,
        state: &StateAuthority,
        native_compilation: &str,
        compiler_store_component: &str,
        build_state_root: Option<&std::path::Path>,
        engine: KotlinSemanticEngine,
    ) -> Result<ProjectNativeKotlinAttempt, ClewError> {
        self.open_compilation_with_engine(
            state,
            native_compilation,
            compiler_store_component,
            build_state_root,
            Some(engine),
        )
    }

    fn open_compilation_with_engine(
        &self,
        state: &StateAuthority,
        native_compilation: &str,
        compiler_store_component: &str,
        build_state_root: Option<&std::path::Path>,
        qualification_engine: Option<KotlinSemanticEngine>,
    ) -> Result<ProjectNativeKotlinAttempt, ClewError> {
        validate_compiler_store_component(compiler_store_component)?;
        if !self.authorized_compilations.contains(native_compilation) {
            return Err(invalid(
                "compilation is outside the exact workspace-set authority",
            ));
        }
        if !self
            .issued_compilations
            .lock()
            .map_err(poisoned)?
            .insert(native_compilation.to_owned())
        {
            return Err(invalid(
                "compilation was opened twice within one workspace-set authorization",
            ));
        }
        // This is the only private legacy bridge. Until the post-G1 worker
        // cutover replaces it with one protocol request, Gradle/Maven model
        // extraction is serialized while independent compiler lanes continue
        // concurrently after their exact OpenProject response returns.
        let _model_extraction = self.model_extraction_gate.lock().map_err(poisoned)?;
        let compiler_store = state
            .directory(std::path::Path::new("generations/compiler-store"))?
            .child(std::path::Path::new(compiler_store_component))?;
        let compiler_store_namespace = format!("sha256:{compiler_store_component}");
        let mut worker = if let Some(engine) = qualification_engine {
            #[cfg(test)]
            {
                WorkerClient::start_qualification_engine_with_managed_states(
                    &workspace_root(),
                    engine,
                    build_state_root,
                    Some(&compiler_store),
                    &compiler_store_namespace,
                )?
            }
            #[cfg(not(test))]
            {
                let _ = engine;
                unreachable!("qualification engine is test-only")
            }
        } else {
            WorkerClient::start_with_managed_states(
                &workspace_root(),
                build_state_root,
                Some(&compiler_store),
                &compiler_store_namespace,
            )?
        };
        let request = json!({
            "repo":self.repo,
            "compilation":native_compilation,
            "syntaxOnly":false,
        });
        let project = worker.open_project_verified(&request)?;
        self.legacy_open_project_calls
            .fetch_add(1, Ordering::AcqRel);
        Ok(ProjectNativeKotlinAttempt {
            store: self.store.clone(),
            snapshot: self.snapshot.clone(),
            repo: self.repo.clone(),
            derived_mounts: self.derived_mounts.clone(),
            worker: Some(worker),
            request,
            project,
        })
    }

    #[cfg(test)]
    fn profile(&self) -> ProjectNativeKotlinWorkspaceProfile {
        self.current_profile()
    }

    fn current_profile(&self) -> ProjectNativeKotlinWorkspaceProfile {
        ProjectNativeKotlinWorkspaceProfile {
            legacy_open_project_calls: self.legacy_open_project_calls.load(Ordering::Acquire),
            ..self.preparation_profile.clone()
        }
    }

    pub(crate) fn finish(mut self) -> Result<ProjectNativeKotlinWorkspaceProfile, ClewError> {
        let unmount = unmount_project_derived_state(&self.repo, &self.derived_mounts);
        if unmount.is_ok() {
            self.derived_mounts.clear();
        }
        let verification = verify_materialized_inputs(
            &self.repo,
            &self.snapshot,
            &self.store,
            &self.derived_mounts,
        );
        unmount?;
        verification?;
        Ok(self.current_profile())
    }
}

impl Drop for ProjectNativeKotlinWorkspace {
    fn drop(&mut self) {
        let _ = unmount_project_derived_state(&self.repo, &self.derived_mounts);
    }
}

impl ProjectNativeKotlinAttempt {
    pub(crate) fn project_authority(&self) -> &Value {
        &self.project
    }

    pub(crate) fn cancellation_handle(&self) -> crate::worker::WorkerCancellationHandle {
        self.worker
            .as_ref()
            .expect("live project-native worker")
            .cancellation_handle()
    }

    pub(crate) fn analyze(
        mut self,
    ) -> Result<
        (
            Value,
            Option<crate::worker::CompilerIndexProfile>,
            WorkerRequestCounters,
        ),
        ClewError,
    > {
        let worker = self
            .worker
            .as_mut()
            .ok_or_else(|| invalid("project-native worker is unavailable"))?;
        let index = match worker.index_files_verified_after_project(&self.request, &self.project) {
            Ok(verified) => worker.inspect_verified_index(&verified)?.clone(),
            Err(error) if error.code == ErrorCode::IncompleteSemanticAnalysis => {
                let files = self
                    .snapshot
                    .index
                    .iter()
                    .filter(|entry| entry.stage == 0 && entry.path.ends_with(".kt"))
                    .map(|entry| entry.path.clone())
                    .collect::<Vec<_>>();
                if files.is_empty() {
                    return Err(error);
                }
                let syntax = worker.index_files_source_syntax_verified(&json!({
                    "repo":self.repo,
                    "compilation":self.request["compilation"],
                    "syntaxOnly":true,
                    "files":files,
                }))?;
                let mut syntax = worker.inspect_verified_source_syntax(&syntax)?.clone();
                normalize_source_syntax_fallback(&mut syntax, &error)?;
                syntax
            }
            Err(error) => return Err(error),
        };
        let profile = worker.last_profile.compiler_index.clone();
        let counters = self.finish()?;
        Ok((index, profile, counters))
    }

    #[cfg(test)]
    fn analyze_strict(
        mut self,
    ) -> Result<
        (
            Value,
            Option<crate::worker::CompilerIndexProfile>,
            WorkerRequestCounters,
        ),
        ClewError,
    > {
        let worker = self
            .worker
            .as_mut()
            .ok_or_else(|| invalid("project-native worker is unavailable"))?;
        let verified = worker
            .index_files_verified_after_project(&self.request, &self.project)
            .map_err(|error| {
                panic!(
                    "strict Kotlin semantic analysis failed: code={:?}; evidence={:?}",
                    error.code, error.evidence
                )
            })?;
        let index = worker.inspect_verified_index(&verified)?.clone();
        let profile = worker.last_profile.compiler_index.clone();
        let counters = self.finish()?;
        Ok((index, profile, counters))
    }

    pub(crate) fn close_without_analysis(mut self) -> Result<WorkerRequestCounters, ClewError> {
        self.finish()
    }

    fn finish(&mut self) -> Result<WorkerRequestCounters, ClewError> {
        let worker = self
            .worker
            .take()
            .ok_or_else(|| invalid("project-native worker was already closed"))?;
        let counters = worker.request_counters();
        let shutdown = worker.shutdown();
        let verification = verify_materialized_inputs(
            &self.repo,
            &self.snapshot,
            &self.store,
            &self.derived_mounts,
        );
        shutdown?;
        verification?;
        Ok(counters)
    }
}

impl Drop for ProjectNativeKotlinAttempt {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.shutdown();
        }
    }
}

fn validate_compiler_store_component(value: &str) -> Result<(), ClewError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("compiler store component is invalid"));
    }
    Ok(())
}

fn verify_materialized_inputs(
    repo: &std::path::Path,
    snapshot: &RepositoryInputSnapshot,
    store: &CasStore,
    derived_mounts: &[std::path::PathBuf],
) -> Result<(), ClewError> {
    let (observed_snapshot, _) = if derived_mounts.is_empty() {
        capture(repo, store)?
    } else {
        capture_ignoring_derived_mounts(repo, store, derived_mounts)?
    };
    let unchanged_inputs = observed_snapshot.staged_view_digest == snapshot.staged_view_digest
        && observed_snapshot.cached_view_digest == snapshot.cached_view_digest
        && observed_snapshot.untracked_view_digest == snapshot.untracked_view_digest
        && observed_snapshot.index == snapshot.index
        && observed_snapshot.worktree.len() == snapshot.worktree.len()
        && observed_snapshot
            .worktree
            .iter()
            .zip(&snapshot.worktree)
            .all(|(observed, expected)| {
                observed.path == expected.path
                    && observed.kind == expected.kind
                    && observed.content == expected.content
            });
    if !unchanged_inputs {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "project-native analysis modified sealed repository inputs",
        ));
    }
    Ok(())
}

fn normalize_source_syntax_fallback(
    index: &mut Value,
    semantic_error: &ClewError,
) -> Result<(), ClewError> {
    let object = index
        .as_object_mut()
        .ok_or_else(|| invalid("SOURCE_SYNTAX fallback is not an object"))?;
    object.insert("analysisAuthority".into(), json!("SOURCE_SYNTAX"));
    object.insert("analysisCertainty".into(), json!("UNSURE"));
    object.insert("analysisCoverage".into(), json!("PARTIAL"));
    object.insert(
        "analysisFallback".into(),
        json!({
            "code":format!("{:?}", semantic_error.code),
            "obligation":"restore successful compiler-semantic K2 analysis before publication",
        }),
    );
    if object
        .get("declarationDescriptors")
        .and_then(|value| value.get("descriptors"))
        .and_then(Value::as_array)
        .is_none()
    {
        object.insert(
            "declarationDescriptors".into(),
            json!({
                "coverage":"PARTIAL",
                "descriptors":[],
                "boundaries":[{"code":"SOURCE_SYNTAX_ONLY","resolution":"UNKNOWN"}],
            }),
        );
    }
    if object
        .get("declarationRelations")
        .and_then(|value| value.get("relations"))
        .and_then(Value::as_array)
        .is_none()
    {
        object.insert(
            "declarationRelations".into(),
            json!({
                "coverage":"PARTIAL",
                "relations":[],
                "boundaries":[{"code":"SOURCE_SYNTAX_ONLY","resolution":"UNKNOWN"}],
            }),
        );
    }
    Ok(())
}

fn unmount_project_derived_state(
    repo: &std::path::Path,
    mounts: &[std::path::PathBuf],
) -> Result<(), ClewError> {
    for relative in mounts {
        let path = repo.join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                "project-native derived mount authority changed",
            ));
        }
        fs::remove_file(path).map_err(io_error)?;
    }
    Ok(())
}

fn mount_project_derived_state(
    attempt_root: &std::path::Path,
    repo: &std::path::Path,
    snapshot: &RepositoryInputSnapshot,
) -> Result<Vec<std::path::PathBuf>, ClewError> {
    let mut mounts = std::collections::BTreeSet::from([
        std::path::PathBuf::from(".gradle"),
        std::path::PathBuf::from("build"),
    ]);
    for entry in &snapshot.index {
        let path = std::path::Path::new(&entry.path);
        let file = path.file_name().and_then(|value| value.to_str());
        let Some(parent) = path.parent() else {
            continue;
        };
        match file {
            Some("build.gradle" | "build.gradle.kts") => {
                mounts.insert(parent.join("build"));
            }
            Some("pom.xml") => {
                mounts.insert(parent.join("target"));
            }
            _ => {}
        }
    }
    for entry in walkdir::WalkDir::new(repo)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.path() == repo || entry.file_name() != std::ffi::OsStr::new(".git")
        })
    {
        let entry = entry.map_err(|error| io_error(std::io::Error::other(error)))?;
        if entry.file_type().is_dir()
            && entry.path().file_name().and_then(|value| value.to_str()) != Some(".git")
        {
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700))
                .map_err(io_error)?;
        }
    }
    #[cfg(unix)]
    {
        for relative in &mounts {
            let mount = repo.join(relative);
            if fs::symlink_metadata(&mount).is_ok() {
                return Err(ClewError::new(
                    ErrorCode::InputMutated,
                    "derived output path overlaps a repository input",
                ));
            }
            let identity = canonical::hash(&relative.to_string_lossy()).map_err(internal)?;
            let target = attempt_root
                .join("derived")
                .join(identity.strip_prefix("sha256:").unwrap_or(&identity));
            create_private_directory(&target)?;
            if let Err(error) = symlink(&target, &mount) {
                return Err(io_error(error));
            }
        }
    }
    Ok(mounts.into_iter().collect())
}

pub(crate) fn translate_facts(
    store: &CasStore,
    index: &Value,
) -> Result<Vec<FactRecord>, ClewError> {
    let capability = CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?;
    let mut pending = Vec::new();
    let metadata = json!({
        "schema":"codeclew-kotlin-index-metadata/2.2",
        "compilation":index.get("compilation"),
        "compilerVersion":index.get("compilerVersion"),
        "projectCompilerVersion":index.get("projectCompilerVersion").or_else(|| index.get("declaredCompilerVersion")),
        "analyzerCompilerVersion":index.get("analyzerCompilerVersion").or_else(|| index.get("compilerVersion")),
        "kotlinProjectSemantics":index.get("kotlinProjectSemantics"),
        "kotlinSemanticEngine":index.get("kotlinSemanticEngine"),
        "buildModelBoundaries":index.get("buildModelBoundaries"),
        "projectModelHash":index.get("projectModelHash"),
        "classpathHash":index.get("classpathHash"),
        "compilerOptionsHash":index.get("compilerOptionsHash"),
        "semanticInputManifestHash":index.get("semanticInputManifestHash"),
        "localCfgHash":index.get("localCfgHash"),
    });
    push_fact(&capability, "metadata", &metadata, &mut pending)?;
    for (category, pointer) in [
        ("file", "/files"),
        ("descriptor", "/declarationDescriptors/descriptors"),
        ("descriptor-boundary", "/declarationDescriptors/boundaries"),
        ("relation", "/declarationRelations/relations"),
        ("relation-boundary", "/declarationRelations/boundaries"),
        ("local-cfg", "/localCfgs"),
        ("local-cfg-boundary", "/localCfgBoundaries"),
    ] {
        let rows = index
            .pointer(pointer)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    format!("Kotlin worker result has no {category} rows"),
                )
            })?;
        for row in rows {
            if category == "file" {
                let normalized = normalize_file_fact(row)?;
                push_fact(&capability, category, &normalized, &mut pending)?;
            } else if category == "local-cfg" {
                let payload: crate::thread_flow_cfg::LocalCfgPayload =
                    serde_json::from_value(row.clone()).map_err(|_| {
                        ClewError::new(
                            ErrorCode::WorkerProtocolMismatch,
                            "Kotlin local CFG payload is malformed",
                        )
                    })?;
                crate::thread_flow_cfg::validate(&payload).map_err(|_| {
                    ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "Kotlin local CFG payload failed admission",
                    )
                })?;
                push_fact(&capability, category, row, &mut pending)?;
            } else if category == "local-cfg-boundary" {
                crate::thread_flow_cfg::validate_boundary(row).map_err(|_| {
                    ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "Kotlin local CFG boundary failed admission",
                    )
                })?;
                push_fact(&capability, category, row, &mut pending)?;
            } else {
                push_fact(&capability, category, row, &mut pending)?;
            }
        }
    }
    let payloads = store.put_batch(
        pending
            .iter()
            .map(|fact| (FACT_PAYLOAD_SCHEMA.to_owned(), fact.bytes.clone()))
            .collect(),
    )?;
    let mut facts = pending
        .into_iter()
        .zip(payloads)
        .map(|(fact, payload)| FactRecord {
            fact_key: fact.fact_key,
            domain_uri: fact.domain_uri,
            payload,
        })
        .collect::<Vec<_>>();
    facts.sort_by(|left, right| left.fact_key.cmp(&right.fact_key));
    if !facts
        .windows(2)
        .all(|pair| pair[0].fact_key < pair[1].fact_key)
    {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "Kotlin normalized facts contain duplicate identities",
        ));
    }
    Ok(facts)
}

fn normalize_file_fact(row: &Value) -> Result<Value, ClewError> {
    let relative = row
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| safe_relative_source_path(path))
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "Kotlin file fact has no safe repository-relative path",
            )
        })?
        .to_owned();
    let mut normalized = row.clone();
    normalize_operational_paths(&mut normalized, &relative, 0)?;
    let object = normalized.as_object_mut().ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "Kotlin file fact is not an object",
        )
    })?;
    let semantic_facts = object
        .remove("semanticFacts")
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let semantic_fact_count = semantic_facts
        .as_array()
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "Kotlin file semantic facts are not an array",
            )
        })?
        .len();
    if object
        .insert(
            "semanticFactCount".into(),
            Value::from(u64::try_from(semantic_fact_count).map_err(internal)?),
        )
        .is_some()
        || object
            .insert(
                "semanticFactsDigest".into(),
                Value::String(canonical::hash(&semantic_facts).map_err(internal)?),
            )
            .is_some()
    {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "Kotlin file fact contains reserved semantic summary fields",
        ));
    }
    Ok(normalized)
}

fn normalize_operational_paths(
    value: &mut Value,
    outer_relative: &str,
    depth: usize,
) -> Result<(), ClewError> {
    if depth > 64 {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "Kotlin file fact path structure is too deeply nested",
        ));
    }
    match value {
        Value::Array(values) => {
            for value in values {
                normalize_operational_paths(value, outer_relative, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                let lowercase_key = key.to_ascii_lowercase();
                if let Some(observed) = value.as_str().and_then(|text| {
                    if operational_path_key(key, &lowercase_key) {
                        Some(Ok(text))
                    } else if operational_uri_key(&lowercase_key)
                        && text
                            .get(..5)
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
                    {
                        Some(file_uri_path(text))
                    } else {
                        None
                    }
                }) {
                    let observed = observed?;
                    *value = Value::String(normalize_operational_path(observed, outer_relative)?);
                }
                normalize_operational_paths(value, outer_relative, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn operational_path_key(key: &str, lowercase_key: &str) -> bool {
    matches!(
        lowercase_key,
        "file" | "fileid" | "path" | "normalizedrelativepath" | "relativepath" | "sourcepath"
    ) || field_suffix(key, "path")
        || field_suffix(key, "file")
}

fn operational_uri_key(lowercase_key: &str) -> bool {
    matches!(lowercase_key, "uri" | "fileuri" | "sourceuri") || lowercase_key.ends_with("uri")
}

fn field_suffix(key: &str, suffix: &str) -> bool {
    let Some(prefix) = key.get(..key.len().saturating_sub(suffix.len())) else {
        return false;
    };
    let Some(tail) = key.get(prefix.len()..) else {
        return false;
    };
    tail.eq_ignore_ascii_case(suffix)
        && (prefix.ends_with('_')
            || prefix.ends_with('-')
            || tail.as_bytes().first().is_some_and(u8::is_ascii_uppercase))
}

fn file_uri_path(uri: &str) -> Result<&str, ClewError> {
    let path = if uri
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"))
    {
        &uri[7..]
    } else {
        &uri[5..]
    };
    if uri
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"))
        && !path.starts_with('/')
    {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "Kotlin file fact contains a file URI authority",
        ));
    }
    Ok(path)
}

fn normalize_operational_path(path: &str, outer_relative: &str) -> Result<String, ClewError> {
    let observed = Path::new(path);
    if observed.is_absolute() {
        if !observed.ends_with(Path::new(outer_relative)) {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "Kotlin file fact contains an out-of-scope operational path",
            ));
        }
        Ok(outer_relative.to_owned())
    } else {
        if !safe_relative_source_path(path) {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "Kotlin file fact contains an unsafe relative path",
            ));
        }
        Ok(path.to_owned())
    }
}

fn safe_relative_source_path(path: &str) -> bool {
    !path.is_empty()
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

struct PendingFact {
    fact_key: String,
    domain_uri: CapabilityUri,
    bytes: Vec<u8>,
}

fn push_fact(
    capability: &CapabilityUri,
    category: &str,
    value: &Value,
    output: &mut Vec<PendingFact>,
) -> Result<(), ClewError> {
    let bytes = canonical::bytes(value).map_err(internal)?;
    let hash = canonical::hash_bytes(&bytes);
    output.push(PendingFact {
        fact_key: format!("kotlin:{category}:{}", hash.trim_start_matches("sha256:")),
        domain_uri: capability.clone(),
        bytes,
    });
    Ok(())
}

pub(crate) fn semantic_scope_digest(index: &Value) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "compilation":index.get("compilation"),
        "compilerVersion":index.get("compilerVersion"),
        "projectCompilerVersion":index.get("projectCompilerVersion").or_else(|| index.get("declaredCompilerVersion")),
        "analyzerCompilerVersion":index.get("analyzerCompilerVersion").or_else(|| index.get("compilerVersion")),
        "kotlinProjectSemantics":index.get("kotlinProjectSemantics"),
        "kotlinSemanticEngine":index.get("kotlinSemanticEngine"),
        "projectModelHash":index.get("projectModelHash"),
        "semanticInputManifestHash":index.get("semanticInputManifestHash"),
        "declarationDescriptorHash":index.get("declarationDescriptorHash"),
        "declarationRelationHash":index.get("declarationRelationHash"),
        "localCfgHash":index.get("localCfgHash"),
    }))
    .map_err(internal)
}

pub(crate) fn completeness_receipt(
    store: &CasStore,
    index: &Value,
    scope_digest: &str,
) -> Result<CasObject, ClewError> {
    let descriptor_coverage = index
        .pointer("/declarationDescriptors/coverage")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Kotlin descriptor coverage is unavailable"))?;
    let relation_coverage = index
        .pointer("/declarationRelations/coverage")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Kotlin relation coverage is unavailable"))?;
    let compatible_analysis = index
        .get("buildModelBoundaries")
        .and_then(Value::as_array)
        .is_some_and(|boundaries| {
            boundaries.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|value| value.starts_with("KOTLIN_ANALYSIS_"))
            })
        });
    let unsure = compatible_analysis
        || index.get("analysisCertainty").and_then(Value::as_str) == Some("UNSURE");
    let complete = !unsure
        && descriptor_coverage == "COMPLETE_SUPPORTED_SUBSET"
        && relation_coverage == "COMPLETE_SUPPORTED_SUBSET";
    let receipt = json!({
        "schema":RECEIPT_SCHEMA,
        "scopeDigest":scope_digest,
        "domains":[
            {
                "domain":KOTLIN_FACTS_CAPABILITY,
                "support":"SUPPORTED",
                "coverage":if complete { "COMPLETE" } else { "PARTIAL" },
                "certainty":if unsure { "UNSURE" } else { "VERIFIED" },
            }
        ],
        "obligations":if complete {
            Vec::<String>::new()
        } else if compatible_analysis {
            vec!["verify-compatible-kotlin-analysis".to_owned()]
        } else if unsure {
            vec!["restore-k2-semantic-analysis".to_owned()]
        } else {
            vec!["verify-partial-kotlin-boundaries".to_owned()]
        },
    });
    let bytes = canonical::bytes(&receipt).map_err(internal)?;
    store.put(RECEIPT_SCHEMA, &bytes)
}

fn require_digest(value: &str) -> Result<(), ClewError> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("Kotlin adapter authority digest is invalid"));
    }
    Ok(())
}

fn cancelled_error() -> ClewError {
    ClewError::new(
        ErrorCode::TransactionRecoveryRequired,
        "Kotlin analysis was cancelled",
    )
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> ClewError {
    ClewError::new(ErrorCode::Internal, "Kotlin adapter lock is poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter_v2::{
        ANALYSIS_REQUEST_SCHEMA, COMPILATION_SCHEMA, CompilationDescriptor, ConformanceSink,
        DescriptorCompleteness, DescriptorOrigin, SourceRootDescriptor,
    };
    use crate::generation_v2::{GenerationKind, GenerationManifest};
    use crate::incremental_v2::{COMPILER_STORE_KEY_SCHEMA, CompilerStoreKey, CompletenessVector};
    use crate::repository_snapshot;
    use crate::worker::CompilerIndexStatus;
    use std::path::Path;

    fn empty_local_cfg_hash() -> String {
        canonical::hash(&json!({"graphs":[],"boundaries":[]})).unwrap()
    }

    #[derive(Clone)]
    struct FakeDriver {
        version: &'static str,
    }

    impl KotlinGenerationDriver for FakeDriver {
        fn analyze(&self, _request: &AnalyzeGenerationRequest) -> Result<Value, ClewError> {
            Ok(json!({
                "schema":"semantic-index/0.1",
                "compilation":":/main",
                "compilerVersion":self.version,
                "k2Validated":true,
                "projectModelHash":format!("sha256:{}", "1".repeat(64)),
                "classpathHash":format!("sha256:{}", "2".repeat(64)),
                "compilerOptionsHash":format!("sha256:{}", "3".repeat(64)),
                "semanticInputManifestHash":format!("sha256:{}", "4".repeat(64)),
                "declarationDescriptorHash":format!("sha256:{}", "5".repeat(64)),
                "declarationRelationHash":format!("sha256:{}", "6".repeat(64)),
                "localCfgHash":empty_local_cfg_hash(),
                "files":[{"path":"src/Main.kt","contentHash":format!("sha256:{}", "7".repeat(64)),"declarations":[]}],
                "declarationDescriptors":{"coverage":"COMPLETE_SUPPORTED_SUBSET","descriptors":[],"boundaries":[]},
                "declarationRelations":{"coverage":"COMPLETE_SUPPORTED_SUBSET","relations":[],"boundaries":[]},
                "localCfgs":[],
                "localCfgBoundaries":[],
            }))
        }
    }

    #[test]
    fn file_facts_replace_private_operational_paths_with_repository_identity() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let private_file = "/private/codeclew/attempt/repo/src/Main.kt";
        let index = json!({
            "compilation":":/main",
            "compilerVersion":"2.3.0",
            "projectModelHash":format!("sha256:{}", "1".repeat(64)),
            "classpathHash":format!("sha256:{}", "2".repeat(64)),
            "compilerOptionsHash":format!("sha256:{}", "3".repeat(64)),
            "semanticInputManifestHash":format!("sha256:{}", "4".repeat(64)),
            "files":[{
                "path":"src/Main.kt",
                "normalizedRelativePath":"src/Main.kt",
                "declarations":[{
                    "sourceOrigin":{
                        "file":private_file,
                        "fileUri":format!("file://{private_file}"),
                        "relatedFile":"src/Other.kt",
                        "relatedUri":"file:src/Other.kt",
                        "semanticUri":"symbol:kotlin/Main",
                        "rangeStart":0,
                        "rangeEnd":4
                    }
                }],
                "semanticFacts":[{
                    "file":private_file,
                    "kind":"FirCall",
                    "start":0,
                    "end":4,
                    "details":{
                        "sourcePath":private_file,
                        "nested":[{"generatedFile":private_file}]
                    }
                }]
            }],
            "declarationDescriptors":{"descriptors":[],"boundaries":[]},
            "declarationRelations":{"relations":[],"boundaries":[]},
            "localCfgHash":empty_local_cfg_hash(),
            "localCfgs":[],
            "localCfgBoundaries":[],
        });
        let facts = translate_facts(&store, &index).unwrap();
        let file = facts
            .iter()
            .find(|fact| fact.fact_key.starts_with("kotlin:file:"))
            .unwrap();
        let lease = store.read(&file.payload, 4096).unwrap();
        let payload: Value = serde_json::from_slice(lease.bytes()).unwrap();
        assert_eq!(payload["path"], "src/Main.kt");
        assert_eq!(
            payload["declarations"][0]["sourceOrigin"]["file"],
            "src/Main.kt"
        );
        assert_eq!(
            payload["declarations"][0]["sourceOrigin"]["fileUri"],
            "src/Main.kt"
        );
        assert_eq!(
            payload["declarations"][0]["sourceOrigin"]["relatedFile"],
            "src/Other.kt"
        );
        assert_eq!(
            payload["declarations"][0]["sourceOrigin"]["relatedUri"],
            "src/Other.kt"
        );
        assert_eq!(
            payload["declarations"][0]["sourceOrigin"]["semanticUri"],
            "symbol:kotlin/Main"
        );
        assert!(payload.get("semanticFacts").is_none());
        assert_eq!(payload["semanticFactCount"], 1);
        assert_eq!(
            payload["semanticFactsDigest"],
            canonical::hash(&json!([{
                "file":"src/Main.kt",
                "kind":"FirCall",
                "start":0,
                "end":4,
                "details":{
                    "sourcePath":"src/Main.kt",
                    "nested":[{"generatedFile":"src/Main.kt"}]
                }
            }]))
            .unwrap()
        );
        for fact in &facts {
            let lease = store.read(&fact.payload, 4096).unwrap();
            let bytes = String::from_utf8_lossy(lease.bytes());
            assert!(!bytes.contains("/private/"));
            assert!(!bytes.contains("/Users/"));
        }

        let mut mismatched = index.clone();
        mismatched["files"][0]["semanticFacts"][0]["file"] =
            Value::String("/private/codeclew/attempt/repo/src/Other.kt".into());
        assert_eq!(
            translate_facts(&store, &mismatched).unwrap_err().code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut mismatched_uri = index.clone();
        mismatched_uri["files"][0]["declarations"][0]["sourceOrigin"]["fileUri"] =
            Value::String("file:///private/codeclew/attempt/repo/src/Other.kt".into());
        assert_eq!(
            translate_facts(&store, &mismatched_uri).unwrap_err().code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut unsafe_relative_uri = index;
        unsafe_relative_uri["files"][0]["declarations"][0]["sourceOrigin"]["fileUri"] =
            Value::String("file:../Other.kt".into());
        assert_eq!(
            translate_facts(&store, &unsafe_relative_uri)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn local_cfg_graphs_and_unknown_boundaries_are_persisted_as_distinct_cas_facts() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let mut graph = json!({
            "schema":"local-cfg/0.1",
            "graphId":"",
            "ownerSymbolIdentity":"callable:p/Box.save#jvm:()V",
            "file":"src/main/kotlin/p/Box.kt",
            "compilerGraphName":"Box.save",
            "provider":"K2_FIR_CFG",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "nodes":[{"nodeId":0,"role":"ENTRY"},{"nodeId":1,"role":"RETURN"}],
            "edges":[{"sourceNodeId":0,"targetNodeId":1,"kind":"RETURN","label":"CFG_RETURN"}],
        });
        graph["graphId"] = json!(canonical::hash(&graph).unwrap());
        let boundary = json!({
            "schema":"local-cfg-boundary/0.1",
            "stage":"NORMALIZE",
            "code":"UNSUPPORTED_LOCAL_CFG_EDGE",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR_CFG",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "rawRowHash":format!("sha256:{}", "9".repeat(64)),
        });
        let index = json!({
            "compilation":":/main",
            "compilerVersion":"2.4.10",
            "projectModelHash":format!("sha256:{}", "1".repeat(64)),
            "classpathHash":format!("sha256:{}", "2".repeat(64)),
            "compilerOptionsHash":format!("sha256:{}", "3".repeat(64)),
            "semanticInputManifestHash":format!("sha256:{}", "4".repeat(64)),
            "files":[],
            "declarationDescriptors":{"descriptors":[],"boundaries":[]},
            "declarationRelations":{"relations":[],"boundaries":[]},
            "localCfgHash":canonical::hash(&json!({"graphs":[graph.clone()],"boundaries":[boundary.clone()]})).unwrap(),
            "localCfgs":[graph.clone()],
            "localCfgBoundaries":[boundary.clone()],
        });

        let facts = translate_facts(&store, &index).unwrap();
        let graph_fact = facts
            .iter()
            .find(|fact| fact.fact_key.starts_with("kotlin:local-cfg:"))
            .unwrap();
        let boundary_fact = facts
            .iter()
            .find(|fact| fact.fact_key.starts_with("kotlin:local-cfg-boundary:"))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(store.read(&graph_fact.payload, 8192).unwrap().bytes())
                .unwrap(),
            graph
        );
        assert_eq!(
            serde_json::from_slice::<Value>(
                store.read(&boundary_fact.payload, 8192).unwrap().bytes()
            )
            .unwrap(),
            boundary
        );

        let mut invalid = index;
        invalid["localCfgs"][0]["graphId"] = json!(format!("sha256:{}", "0".repeat(64)));
        assert_eq!(
            translate_facts(&store, &invalid).unwrap_err().code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn adapter_digest_binds_worker_and_translation_contract() {
        let worker = format!("sha256:{}", "a".repeat(64));
        let first = kotlin_adapter_digest(&worker).unwrap();
        assert_eq!(first, kotlin_adapter_digest(&worker).unwrap());
        assert_ne!(first, worker);
        assert_ne!(
            first,
            kotlin_adapter_digest(&format!("sha256:{}", "b".repeat(64))).unwrap()
        );
    }

    fn request(store: &CasStore, toolchain: CasObject) -> AnalyzeGenerationRequest {
        let tree = store.put("test/tree/1", b"tree").unwrap();
        let options = store
            .put("test/options/1", br#"{"nativeCompilation":":/main"}"#)
            .unwrap();
        AnalyzeGenerationRequest {
            schema: ANALYSIS_REQUEST_SCHEMA.into(),
            attempt_id: "attempt:kotlin".into(),
            generation_key: format!("sha256:{}", "9".repeat(64)),
            capability: CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY).unwrap(),
            compilation: CompilationDescriptor {
                schema: COMPILATION_SCHEMA.into(),
                compilation_id: "root-main".into(),
                language_uri: LanguageUri::parse(KOTLIN_LANGUAGE).unwrap(),
                source_roots: vec![SourceRootDescriptor {
                    logical_name: "main".into(),
                    tree,
                }],
                generated_source_roots: vec![],
                classpath: vec![],
                toolchain,
                plugins: vec![],
                canonical_options: options,
                dependency_compilation_ids: vec![],
                operations: vec![],
                origin: DescriptorOrigin::ProjectNative,
                completeness: DescriptorCompleteness::Complete,
            },
            derived_input_manifest: store.put("test/derived/1", b"derived").unwrap(),
            parent_generation: None,
        }
    }

    #[test]
    fn k21_k23_and_k24_share_one_streaming_adapter_contract() {
        for line in [
            KotlinSemanticEngine::Kotlin21,
            KotlinSemanticEngine::Kotlin23,
            KotlinSemanticEngine::Kotlin24,
        ] {
            let root = tempfile::tempdir().unwrap();
            let state = StateAuthority::open(root.path().join("v2")).unwrap();
            let store = CasStore::open(&state).unwrap();
            let toolchain = store
                .put(
                    "test/toolchain/1",
                    line.analyzer_compiler_version().as_bytes(),
                )
                .unwrap();
            let adapter = KotlinAdapterV2::new(
                line,
                format!("sha256:{}", "a".repeat(64)),
                toolchain.digest.clone(),
                store.clone(),
                FakeDriver {
                    version: line.analyzer_compiler_version(),
                },
            )
            .unwrap();
            let mut sink =
                ConformanceSink::for_capabilities([
                    CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY).unwrap()
                ]);
            adapter
                .analyze_generation(
                    &request(&store, toolchain),
                    &mut sink,
                    &AtomicBool::new(false),
                )
                .unwrap();
            let completion = sink.finish().unwrap();
            assert_eq!(completion.fact_count, 2);
            let handshake = adapter.handshake().unwrap();
            assert_eq!(
                handshake.toolchains[0].minimum_version.as_deref(),
                Some(line.analyzer_compiler_version())
            );
            assert_eq!(handshake.adapter_id, KOTLIN_ADAPTER_CONTRACT_ID);
        }
    }

    #[test]
    fn cancellation_is_checked_before_driver_execution() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let toolchain = store.put("test/toolchain/1", b"2.4.10").unwrap();
        let adapter = KotlinAdapterV2::new(
            KotlinSemanticEngine::Kotlin24,
            format!("sha256:{}", "a".repeat(64)),
            toolchain.digest.clone(),
            store.clone(),
            FakeDriver { version: "2.4.10" },
        )
        .unwrap();
        adapter.cancel("attempt:kotlin").unwrap();
        let mut sink = ConformanceSink::default();
        assert_eq!(
            adapter
                .analyze_generation(
                    &request(&store, toolchain),
                    &mut sink,
                    &AtomicBool::new(false)
                )
                .unwrap_err()
                .code,
            ErrorCode::TransactionRecoveryRequired
        );
    }

    #[test]
    fn source_syntax_fallback_is_explicitly_unsure_and_publication_blocking() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let mut index = json!({
            "compilation":":/main",
            "compilerVersion":"2.4.10",
            "files":[],
            "declarationDescriptors":null,
            "declarationRelations":null,
        });
        normalize_source_syntax_fallback(
            &mut index,
            &ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "synthetic K2 refusal",
            ),
        )
        .unwrap();
        assert_eq!(index["analysisCertainty"], "UNSURE");
        assert_eq!(index["declarationDescriptors"]["coverage"], "PARTIAL");
        let receipt =
            completeness_receipt(&store, &index, &format!("sha256:{}", "1".repeat(64))).unwrap();
        let lease = store.read(&receipt, 4096).unwrap();
        let value: Value = serde_json::from_slice(lease.bytes()).unwrap();
        assert_eq!(value["domains"][0]["certainty"], "UNSURE");
        assert_eq!(value["obligations"][0], "restore-k2-semantic-analysis");
    }

    #[test]
    fn compatible_compiler_analysis_never_claims_verified_completeness() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let index = json!({
            "analysisCertainty":"VERIFIED",
            "declarationDescriptors":{"coverage":"COMPLETE_SUPPORTED_SUBSET"},
            "declarationRelations":{"coverage":"COMPLETE_SUPPORTED_SUBSET"},
            "buildModelBoundaries":["KOTLIN_ANALYSIS_USES_DIFFERENT_COMPILER"]
        });
        let receipt =
            completeness_receipt(&store, &index, &format!("sha256:{}", "1".repeat(64))).unwrap();
        let lease = store.read(&receipt, 4096).unwrap();
        let value: Value = serde_json::from_slice(lease.bytes()).unwrap();
        assert_eq!(value["domains"][0]["coverage"], "PARTIAL");
        assert_eq!(value["domains"][0]["certainty"], "UNSURE");
        assert_eq!(value["obligations"][0], "verify-compatible-kotlin-analysis");
    }

    #[cfg(unix)]
    #[test]
    fn sealed_multi_project_attempt_mounts_every_build_output() {
        let source = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(source.path().join("workers/kotlin")).unwrap();
        for (path, bytes) in [
            (
                "settings.gradle.kts",
                b"include(\":workers:kotlin\")\n".as_slice(),
            ),
            ("build.gradle.kts", b"plugins {}\n".as_slice()),
            ("workers/build.gradle.kts", b"plugins {}\n".as_slice()),
            (
                "workers/kotlin/build.gradle.kts",
                b"plugins {}\n".as_slice(),
            ),
        ] {
            std::fs::write(source.path().join(path), bytes).unwrap();
        }
        for arguments in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(arguments)
                    .current_dir(source.path())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let private = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(private.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = crate::repository_snapshot::capture(source.path(), &store).unwrap();
        let attempt = private.path().join("attempt");
        create_private_directory(&attempt).unwrap();
        let repo = attempt.join("repo");
        materialize(&snapshot, &store, &repo).unwrap();

        let mounts = mount_project_derived_state(&attempt, &repo, &snapshot).unwrap();
        assert_eq!(
            mounts,
            vec![
                std::path::PathBuf::from(".gradle"),
                std::path::PathBuf::from("build"),
                std::path::PathBuf::from("workers/build"),
                std::path::PathBuf::from("workers/kotlin/build"),
            ]
        );
        assert_eq!(
            std::fs::metadata(repo.join("workers"))
                .unwrap()
                .permissions()
                .mode()
                & 0o700,
            0o700
        );
        assert!(
            std::fs::symlink_metadata(repo.join("workers/kotlin/build"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        unmount_project_derived_state(&repo, &mounts).unwrap();
        let (observed, _) = crate::repository_snapshot::capture(&repo, &store).unwrap();
        assert_eq!(observed.staged_view_digest, snapshot.staged_view_digest);
        assert_eq!(observed.cached_view_digest, snapshot.cached_view_digest);
        assert_eq!(
            observed.untracked_view_digest,
            snapshot.untracked_view_digest
        );
        assert_eq!(observed.index, snapshot.index);
        assert!(
            observed
                .worktree
                .iter()
                .zip(&snapshot.worktree)
                .all(|(left, right)| left.path == right.path
                    && left.kind == right.kind
                    && left.content == right.content)
        );
    }

    fn copy_kotlin_basic_fixture(destination: &Path) {
        let source = workspace_root().join("fixtures/kotlin-basic");
        for entry in walkdir::WalkDir::new(&source)
            .into_iter()
            .map(Result::unwrap)
        {
            let relative = entry.path().strip_prefix(&source).unwrap();
            if relative.components().any(|component| {
                matches!(
                    component.as_os_str().to_str(),
                    Some(".git" | ".gradle" | ".semantic-thread" | "build")
                )
            }) {
                continue;
            }
            let target = destination.join(relative);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
            } else if entry.file_type().is_file() {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
        let empty = destination.join("src/main/kotlin/com/acme/Empty.kt");
        std::fs::write(empty, b"").unwrap();
        for arguments in [
            vec!["init", "-q", "-b", "main"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-qm",
                "K24 acceptance fixture",
            ],
        ] {
            let output = std::process::Command::new("git")
                .args(arguments)
                .current_dir(destination)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "fixture Git command failed: {}",
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }

    fn real_k24_analysis(
        state: &StateAuthority,
        store: &CasStore,
        fixture: &Path,
        component: &str,
    ) -> (
        Value,
        crate::worker::CompilerIndexProfile,
        WorkerRequestCounters,
    ) {
        let (snapshot, _) = repository_snapshot::capture(fixture, store).unwrap();
        let workspace =
            ProjectNativeKotlinWorkspace::prepare(state, store, &snapshot, &[":/main".into()])
                .unwrap();
        let attempt = workspace
            .open_compilation_from_set(state, ":/main", component, None)
            .unwrap();
        let (index, profile, requests) = attempt.analyze_strict().unwrap();
        let workspace_profile = workspace.finish().unwrap();
        assert_eq!(workspace_profile.workspace_set_authorizations, 1);
        assert_eq!(workspace_profile.legacy_open_project_calls, 1);
        (index, profile.expect("K24 compiler profile"), requests)
    }

    fn real_default_engine_index(
        state: &StateAuthority,
        store: &CasStore,
        fixture: &Path,
        component: &str,
    ) -> Value {
        let (snapshot, _) = repository_snapshot::capture(fixture, store).unwrap();
        let workspace =
            ProjectNativeKotlinWorkspace::prepare(state, store, &snapshot, &[":/main".into()])
                .unwrap();
        let attempt = workspace
            .open_compilation_from_set(state, ":/main", component, None)
            .unwrap();
        let (index, _, requests) = attempt.analyze_strict().unwrap();
        let workspace_profile = workspace.finish().unwrap();
        assert_eq!(workspace_profile.workspace_set_authorizations, 1);
        assert_eq!(workspace_profile.legacy_open_project_calls, 1);
        assert_one_real_index_request(&requests);
        index
    }

    fn real_qualification_analysis(
        state: &StateAuthority,
        store: &CasStore,
        fixture: &Path,
        component: &str,
        engine: KotlinSemanticEngine,
    ) -> (
        Value,
        Value,
        crate::worker::CompilerIndexProfile,
        WorkerRequestCounters,
    ) {
        let (snapshot, _) = repository_snapshot::capture(fixture, store).unwrap();
        let workspace =
            ProjectNativeKotlinWorkspace::prepare(state, store, &snapshot, &[":/main".into()])
                .unwrap();
        let attempt = workspace
            .open_qualification_compilation_from_set(state, ":/main", component, None, engine)
            .unwrap();
        let project = attempt.project_authority().clone();
        let (index, profile, requests) = attempt.analyze_strict().unwrap();
        let workspace_profile = workspace.finish().unwrap();
        assert_eq!(workspace_profile.workspace_set_authorizations, 1);
        assert_eq!(workspace_profile.legacy_open_project_calls, 1);
        let profile = profile.unwrap_or_else(|| {
            panic!("qualification compiler profile is absent; project={project}; index={index}")
        });
        (project, index, profile, requests)
    }

    fn assert_complete_qualification_index(index: &Value) {
        assert_eq!(
            index.get("k2Validated").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            index.get("analysisMode").and_then(Value::as_str),
            Some("K2_SEMANTIC")
        );
        assert_eq!(index.get("partial").and_then(Value::as_bool), Some(false));
        for graph in ["declarationDescriptors", "declarationRelations"] {
            assert_eq!(
                index
                    .pointer(&format!("/{graph}/coverage"))
                    .and_then(Value::as_str),
                Some("COMPLETE_SUPPORTED_SUBSET"),
                "incomplete {graph}: {index}",
            );
            assert_eq!(
                index
                    .pointer(&format!("/{graph}/boundaries"))
                    .and_then(Value::as_array)
                    .map(Vec::len),
                Some(0),
                "semantic boundaries in {graph}: {index}",
            );
        }
    }

    fn normalized_semantic_facts_digest(index: &Value) -> String {
        let files = index
            .get("files")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|file| normalize_file_fact(file).unwrap())
            .collect::<Vec<_>>();
        canonical::hash(&json!({
            "schema":"codeclew-kotlin-normalized-cross-engine-facts/1.0",
            "files":files,
            "descriptors":index.pointer("/declarationDescriptors/descriptors"),
            "descriptorBoundaries":index.pointer("/declarationDescriptors/boundaries"),
            "relations":index.pointer("/declarationRelations/relations"),
            "relationBoundaries":index.pointer("/declarationRelations/boundaries"),
            "localCfgs":index.get("localCfgs"),
        }))
        .unwrap()
    }

    fn mutable_authority(state: &StateAuthority, component: &str) -> std::path::PathBuf {
        let compiler_store = state
            .root()
            .join("generations/compiler-store")
            .join(component);
        let matches = walkdir::WalkDir::new(compiler_store)
            .into_iter()
            .map(Result::unwrap)
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry.file_name() == "AUTHORITY"
                    && entry
                        .path()
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|name| name == "mutable")
            })
            .map(|entry| entry.into_path())
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1, "one active K24 mutable authority");
        matches.into_iter().next().unwrap()
    }

    fn assert_one_real_index_request(requests: &WorkerRequestCounters) {
        assert_eq!(requests.open_project_requests, 1);
        assert_eq!(requests.index_files_requests, 1);
    }

    #[test]
    fn generation_set_workspace_materializes_and_mounts_one_shared_repository() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let fixture = workspace_root().join("fixtures/kotlin-basic");
        let (snapshot, _) = repository_snapshot::capture(&fixture, &store).unwrap();

        let workspace =
            ProjectNativeKotlinWorkspace::prepare(&state, &store, &snapshot, &[":/main".into()])
                .unwrap();
        let initial = workspace.profile();
        assert_eq!(initial.materializations, 1);
        assert_eq!(initial.derived_mount_sets, 1);
        assert_eq!(initial.workspace_set_authorizations, 1);
        assert_eq!(initial.authorized_compilation_count, 1);
        assert_eq!(initial.legacy_open_project_calls, 0);
        assert_eq!(
            initial.workspace_set_authority_digest,
            canonical::hash(&json!({
                "schema":WORKSPACE_SET_AUTHORIZATION_SCHEMA,
                "compilations":[":/main"],
                "language":"kotlin",
                "providerMode":"PROJECT_NATIVE_LEGACY_BRIDGE",
                "repositorySnapshot":snapshot.snapshot_id,
            }))
            .unwrap()
        );
        let outside = workspace
            .open_compilation_from_set(&state, ":other/main", &"a".repeat(64), None)
            .err()
            .expect("outside compilation must be rejected before worker start");
        assert_eq!(outside.code, ErrorCode::InvalidInput);
        assert!(
            std::fs::symlink_metadata(workspace.repo.join(".gradle"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::symlink_metadata(workspace.repo.join("build"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let profile = workspace.finish().unwrap();
        assert_eq!(profile.materializations, 1);
        assert_eq!(profile.derived_mount_sets, 1);
        assert_eq!(profile.workspace_set_authorizations, 1);
        assert_eq!(profile.legacy_open_project_calls, 0);

        let unsorted = ProjectNativeKotlinWorkspace::prepare(
            &state,
            &store,
            &snapshot,
            &[":b/main".into(), ":a/main".into()],
        )
        .err()
        .expect("unsorted workspace set must be rejected");
        assert_eq!(unsorted.code, ErrorCode::InvalidInput);
        let duplicate = ProjectNativeKotlinWorkspace::prepare(
            &state,
            &store,
            &snapshot,
            &[":a/main".into(), ":a/main".into()],
        )
        .err()
        .expect("duplicate workspace set must be rejected");
        assert_eq!(duplicate.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn k24_real_worker_cold_then_product_unchanged_skips_index_files() {
        let _workspace_worker_guard = crate::worker::workspace_worker_test_lock();
        let root = tempfile::tempdir().unwrap();
        let state_path = root.path().join("v2");
        let moved_state_path = root.path().join("v2-open-inode");
        let state = StateAuthority::open(state_path.clone()).unwrap();
        let store = CasStore::open(&state).unwrap();
        let fixture = workspace_root().join("fixtures/kotlin-basic");
        let (snapshot, _) = repository_snapshot::capture(&fixture, &store).unwrap();
        let component = "a".repeat(64);

        // Kotlin's path-only worker boundary must derive its attempt and
        // compiler-store paths from the pinned leaf descriptors, never by
        // walking StateAuthority's original root again.
        std::fs::rename(&state_path, &moved_state_path).unwrap();
        std::fs::create_dir(&state_path).unwrap();
        std::fs::set_permissions(
            &state_path,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        std::fs::create_dir(state_path.join("attempts")).unwrap();
        std::fs::create_dir_all(state_path.join("generations/compiler-store")).unwrap();

        let cold_workspace =
            ProjectNativeKotlinWorkspace::prepare(&state, &store, &snapshot, &[":/main".into()])
                .unwrap();
        let cold_attempt = cold_workspace
            .open_compilation_from_set(&state, ":/main", &component, None)
            .unwrap();
        let (cold_index, cold, cold_requests) = cold_attempt.analyze().unwrap();
        let cold = cold.expect("cold compiler profile");
        assert_eq!(
            cold.status,
            crate::worker::CompilerIndexStatus::ColdFull,
            "cold profiling: {cold:?}",
        );
        assert!(!cold.fallback_used, "cold profiling: {cold:?}");
        assert_eq!(cold_requests.open_project_requests, 1);
        assert_eq!(cold_requests.index_files_requests, 1);
        let cold_workspace_profile = cold_workspace.finish().unwrap();
        assert_eq!(cold_workspace_profile.workspace_set_authorizations, 1);
        assert_eq!(cold_workspace_profile.legacy_open_project_calls, 1);
        assert!(
            std::fs::read_dir(state_path.join("attempts"))
                .unwrap()
                .next()
                .is_none()
        );
        assert!(
            std::fs::read_dir(state_path.join("generations/compiler-store"))
                .unwrap()
                .next()
                .is_none()
        );

        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        let object = |schema: &str, character: char| CasObject {
            schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
            object_schema: schema.into(),
            digest: digest(character),
            size: 1,
        };
        let compiler_store = CompilerStoreKey {
            schema: COMPILER_STORE_KEY_SCHEMA.into(),
            key: digest('1'),
            adapter_id: KOTLIN_ADAPTER_CONTRACT_ID.into(),
            adapter_digest: digest('2'),
            language_uri: KOTLIN_LANGUAGE.into(),
            toolchain: object("test/toolchain/1", '3'),
            canonical_options: object("test/options/1", '4'),
            classpath: vec![],
            plugins: vec![],
        };
        let generation = GenerationManifest {
            schema: crate::generation_v2::GENERATION_SCHEMA.into(),
            generation_id: digest('5'),
            derived_input_manifest: object(crate::derived_manifest::DERIVED_MANIFEST_SCHEMA, '6'),
            parent_generation: None,
            generation_kind: GenerationKind::Full,
            attempts: vec![],
            shards: vec![],
            fact_count: 0,
        };
        let completeness =
            CompletenessVector::verified_complete(semantic_scope_digest(&cold_index).unwrap())
                .unwrap();
        let (_, receipt_object) = crate::generation_service::create_incremental_receipt(
            &store,
            &cold_index,
            &compiler_store,
            &generation,
            completeness,
        )
        .unwrap();

        // The generation service proves UNCHANGED from its sealed receipt
        // after OpenProject and closes this same worker without IndexFiles.
        let unchanged_workspace =
            ProjectNativeKotlinWorkspace::prepare(&state, &store, &snapshot, &[":/main".into()])
                .unwrap();
        let unchanged_attempt = unchanged_workspace
            .open_compilation_from_set(&state, ":/main", &component, None)
            .unwrap();
        let warm_requests = unchanged_attempt.close_without_analysis().unwrap();
        let workspace_profile = unchanged_workspace.finish().unwrap();
        assert_eq!(workspace_profile.materializations, 1);
        assert_eq!(workspace_profile.derived_mount_sets, 1);
        assert_eq!(workspace_profile.workspace_set_authorizations, 1);
        assert_eq!(workspace_profile.legacy_open_project_calls, 1);
        assert_eq!(warm_requests.open_project_requests, 1);
        assert_eq!(warm_requests.index_files_requests, 0);

        let hex = receipt_object.digest.strip_prefix("sha256:").unwrap();
        let receipt_path = moved_state_path
            .join("objects/sha256")
            .join(&hex[..2])
            .join(&hex[2..]);
        std::fs::write(receipt_path, b"corrupt").unwrap();
        assert_eq!(
            store.read(&receipt_object, 1024 * 1024).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    #[ignore = "release acceptance launches five real Kotlin 2.4 worker analyses"]
    fn k24_real_bta_acceptance_matrix() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let fixture_root = tempfile::tempdir().unwrap();
        let fixture = fixture_root.path().join("kotlin-basic");
        copy_kotlin_basic_fixture(&fixture);
        let component = "a".repeat(64);

        let (cold_index, cold, cold_requests) =
            real_k24_analysis(&state, &store, &fixture, &component);
        assert_eq!(cold.status, CompilerIndexStatus::ColdFull, "{cold:?}");
        assert!(!cold.fallback_used, "cold profiling: {cold:?}");
        assert_eq!(cold.compiled_files, cold.total_files);
        assert!(
            cold.total_files >= 3,
            "empty source must have a receipt: {cold:?}"
        );
        assert_one_real_index_request(&cold_requests);

        let (unchanged_index, unchanged, unchanged_requests) =
            real_k24_analysis(&state, &store, &fixture, &component);
        assert_eq!(
            unchanged.status,
            CompilerIndexStatus::UnchangedHit,
            "{unchanged:?}"
        );
        assert_eq!(unchanged.compiled_files, 0);
        assert_eq!(unchanged.reused_files, unchanged.total_files);
        assert_eq!(unchanged.graph_digest, cold.graph_digest);
        assert_eq!(
            semantic_scope_digest(&unchanged_index).unwrap(),
            semantic_scope_digest(&cold_index).unwrap(),
        );
        assert_one_real_index_request(&unchanged_requests);

        let changed_source = fixture.join("src/main/kotlin/com/acme/Samples.kt");
        let mut changed = std::fs::read_to_string(&changed_source).unwrap();
        changed.push_str("\nfun bta24Changed(value: Int): Int = value + 1\n");
        std::fs::write(&changed_source, changed).unwrap();
        let (incremental_index, incremental, incremental_requests) =
            real_k24_analysis(&state, &store, &fixture, &component);
        assert_eq!(
            incremental.status,
            CompilerIndexStatus::Incremental,
            "{incremental:?}"
        );
        assert!(incremental.compiled_files > 0);
        assert!(incremental.compiled_files < incremental.total_files);
        assert_eq!(
            incremental.compiled_files + incremental.reused_files,
            incremental.total_files,
        );
        assert_one_real_index_request(&incremental_requests);

        let fresh_root = tempfile::tempdir().unwrap();
        let fresh_state = StateAuthority::open(fresh_root.path().join("v2")).unwrap();
        let fresh_store = CasStore::open(&fresh_state).unwrap();
        let (fresh_index, fresh, fresh_requests) =
            real_k24_analysis(&fresh_state, &fresh_store, &fixture, &component);
        assert_eq!(fresh.status, CompilerIndexStatus::ColdFull, "{fresh:?}");
        assert_eq!(
            semantic_scope_digest(&fresh_index).unwrap(),
            semantic_scope_digest(&incremental_index).unwrap(),
            "incremental={incremental:?}; fresh={fresh:?}",
        );
        assert!(incremental.graph_digest.is_some());
        assert!(fresh.graph_digest.is_some());
        assert_one_real_index_request(&fresh_requests);

        std::fs::write(mutable_authority(&state, &component), b"corrupt\n").unwrap();
        let (recovered_index, recovered, recovered_requests) =
            real_k24_analysis(&state, &store, &fixture, &component);
        assert_eq!(
            recovered.status,
            CompilerIndexStatus::RecoveredFull,
            "{recovered:?}"
        );
        assert!(recovered.recovered);
        assert!(recovered.graph_digest.is_some());
        assert_eq!(
            semantic_scope_digest(&recovered_index).unwrap(),
            semantic_scope_digest(&fresh_index).unwrap(),
        );
        assert_one_real_index_request(&recovered_requests);
    }

    #[test]
    #[ignore = "qualification gate launches real version-specific Kotlin workers"]
    fn kotlin_engine_qualification_probe() {
        let fixture = std::env::var_os("CODECLEW_KOTLIN_QUALIFICATION_FIXTURE")
            .map(std::path::PathBuf::from)
            .expect("qualification fixture is required")
            .canonicalize()
            .unwrap();
        let expected_project_version =
            std::env::var("CODECLEW_KOTLIN_QUALIFICATION_PROJECT_VERSION")
                .expect("project version is required");
        let expected_outcome = std::env::var("CODECLEW_KOTLIN_QUALIFICATION_OUTCOME")
            .unwrap_or_else(|_| "QUALIFIED".into());
        let full_lifecycle =
            std::env::var_os("CODECLEW_KOTLIN_QUALIFICATION_FULL_LIFECYCLE").is_some();
        let component = "b".repeat(64);
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();

        if expected_outcome != "QUALIFIED" {
            let (snapshot, _) = repository_snapshot::capture(&fixture, &store).unwrap();
            let workspace = ProjectNativeKotlinWorkspace::prepare(
                &state,
                &store,
                &snapshot,
                &[":/main".into()],
            )
            .unwrap();
            let error = workspace
                .open_qualification_compilation_from_set(
                    &state,
                    ":/main",
                    &component,
                    None,
                    KotlinSemanticEngine::Kotlin24,
                )
                .err()
                .expect("negative qualification row must fail closed");
            let expected_code = if expected_outcome == "UNSUPPORTED_COMPILER_PLUGIN_ABI" {
                ErrorCode::UnsupportedCompilerPluginAbi
            } else {
                ErrorCode::UnsupportedProjectConfiguration
            };
            assert_eq!(error.code, expected_code);
            workspace.finish().unwrap();
            return;
        }

        let (project, cold_index, cold, cold_requests) = real_qualification_analysis(
            &state,
            &store,
            &fixture,
            &component,
            KotlinSemanticEngine::Kotlin24,
        );
        assert_eq!(
            project
                .get("declaredCompilerVersion")
                .and_then(Value::as_str),
            Some(expected_project_version.as_str()),
        );
        let expected_authority = std::env::var("CODECLEW_KOTLIN_QUALIFICATION_AUTHORITY")
            .unwrap_or_else(|_| "KGP_COMPILER_VERSION_PROVIDER".into());
        assert_eq!(
            project
                .pointer("/projectCompilerAuthority/source")
                .and_then(Value::as_str),
            Some(expected_authority.as_str()),
        );
        assert_eq!(
            project
                .pointer("/kotlinSemanticEngine/analyzerCompilerVersion")
                .and_then(Value::as_str),
            Some("2.4.10"),
        );
        assert_eq!(
            project
                .pointer("/engineCompatibility/status")
                .and_then(Value::as_str),
            Some("QUALIFIED"),
        );
        if std::env::var_os("CODECLEW_KOTLIN_QUALIFICATION_SERIALIZATION").is_some() {
            assert!(
                project
                    .get("buildModelBoundaries")
                    .and_then(Value::as_array)
                    .is_some_and(|boundaries| boundaries.iter().any(|boundary| {
                        boundary.as_str()
                            == Some("KOTLIN_SERIALIZATION_PLUGIN_REBOUND_TO_ANALYZER_PATCH")
                    })),
                "serialization plugin was not rebound to the analyzer ABI: {project}",
            );
        }
        assert_eq!(cold.status, CompilerIndexStatus::ColdFull, "{cold:?}");
        assert!(!cold.fallback_used, "{cold:?}");
        assert_complete_qualification_index(&cold_index);
        assert_one_real_index_request(&cold_requests);
        let cold_digest = normalized_semantic_facts_digest(&cold_index);

        if std::env::var_os("CODECLEW_KOTLIN_QUALIFICATION_K23_ORACLE").is_some() {
            let oracle_root = tempfile::tempdir().unwrap();
            let oracle_state = StateAuthority::open(oracle_root.path().join("v2")).unwrap();
            let oracle_store = CasStore::open(&oracle_state).unwrap();
            let oracle_index =
                real_default_engine_index(&oracle_state, &oracle_store, &fixture, &"c".repeat(64));
            assert_complete_qualification_index(&oracle_index);
            assert_eq!(
                normalized_semantic_facts_digest(&cold_index),
                normalized_semantic_facts_digest(&oracle_index),
                "K23 and K24 engines produced different normalized semantic facts",
            );
        }

        if let Ok(expected_digest) = std::env::var("CODECLEW_KOTLIN_QUALIFICATION_GOLDEN") {
            assert_eq!(cold_digest, expected_digest, "qualification golden changed");
        }

        if full_lifecycle {
            let (unchanged_index, unchanged_project, unchanged, unchanged_requests) = {
                let (project, index, profile, requests) = real_qualification_analysis(
                    &state,
                    &store,
                    &fixture,
                    &component,
                    KotlinSemanticEngine::Kotlin24,
                );
                (index, project, profile, requests)
            };
            assert_eq!(
                unchanged.status,
                CompilerIndexStatus::UnchangedHit,
                "{unchanged:?}"
            );
            assert_eq!(
                normalized_semantic_facts_digest(&unchanged_index),
                cold_digest,
            );
            assert_eq!(
                unchanged_project
                    .pointer("/engineCompatibility/status")
                    .and_then(Value::as_str),
                Some("QUALIFIED"),
            );
            assert_one_real_index_request(&unchanged_requests);

            let changed_source = fixture.join("src/main/kotlin/com/acme/QualifiedA.kt");
            let mut changed = std::fs::read_to_string(&changed_source).unwrap();
            changed.push_str("\nclass QualificationChanged(val value: Int)\n");
            std::fs::write(&changed_source, changed).unwrap();
            let (_, incremental_index, incremental, incremental_requests) =
                real_qualification_analysis(
                    &state,
                    &store,
                    &fixture,
                    &component,
                    KotlinSemanticEngine::Kotlin24,
                );
            assert_eq!(
                incremental.status,
                CompilerIndexStatus::Incremental,
                "{incremental:?}"
            );
            assert!(incremental.compiled_files > 0);
            assert!(incremental.compiled_files < incremental.total_files);
            assert_one_real_index_request(&incremental_requests);

            let fresh_root = tempfile::tempdir().unwrap();
            let fresh_state = StateAuthority::open(fresh_root.path().join("v2")).unwrap();
            let fresh_store = CasStore::open(&fresh_state).unwrap();
            let (_, fresh_index, fresh, fresh_requests) = real_qualification_analysis(
                &fresh_state,
                &fresh_store,
                &fixture,
                &component,
                KotlinSemanticEngine::Kotlin24,
            );
            assert_eq!(fresh.status, CompilerIndexStatus::ColdFull, "{fresh:?}");
            assert_eq!(
                normalized_semantic_facts_digest(&incremental_index),
                normalized_semantic_facts_digest(&fresh_index),
            );
            assert_one_real_index_request(&fresh_requests);

            std::fs::write(mutable_authority(&state, &component), b"corrupt\n").unwrap();
            let (_, recovered_index, recovered, recovered_requests) = real_qualification_analysis(
                &state,
                &store,
                &fixture,
                &component,
                KotlinSemanticEngine::Kotlin24,
            );
            assert_eq!(
                recovered.status,
                CompilerIndexStatus::RecoveredFull,
                "{recovered:?}"
            );
            assert!(recovered.recovered);
            assert_eq!(
                normalized_semantic_facts_digest(&recovered_index),
                normalized_semantic_facts_digest(&fresh_index),
            );
            assert_one_real_index_request(&recovered_requests);
        }

        println!(
            "CODECLEW_KOTLIN_QUALIFICATION_RESULT={{\"projectCompilerVersion\":\"{}\",\"engine\":\"{}\",\"normalizedFactsDigest\":\"{}\"}}",
            expected_project_version,
            KotlinSemanticEngine::Kotlin24.engine_id(),
            cold_digest,
        );
    }
}
