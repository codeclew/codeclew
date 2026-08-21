use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    QueryGenerationRequest, QueryGenerationResult, ToolchainConstraint, ValidateCandidateRequest,
    ValidateCandidateResult,
};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::derived_manifest::DerivedAnalysisInputManifest;
use crate::error::{ClewError, ErrorCode};
use crate::generation_v2::GenerationManifest;
use crate::incremental_v2::CompilerStoreKey;
use crate::repository_snapshot::{RepositoryInputSnapshot, capture, materialize};
use crate::state::{StateAuthority, create_private_directory};
use crate::worker::{WorkerClient, workspace_root};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

pub const KOTLIN_LANGUAGE: &str = "language:kotlin";
pub const KOTLIN_FACTS_CAPABILITY: &str = "analysis:kotlin-semantic-facts";
const FACT_PAYLOAD_SCHEMA: &str = "codeclew-kotlin-semantic-fact/2.0";
const RECEIPT_SCHEMA: &str = "codeclew-completeness-receipt/2.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KotlinCompilerLine {
    K21,
    K23,
    K24,
}

impl KotlinCompilerLine {
    pub fn compiler_version(self) -> &'static str {
        match self {
            Self::K21 => "2.1.21",
            Self::K23 => "2.3.0",
            Self::K24 => "2.4.10",
        }
    }

    fn adapter_id(self) -> &'static str {
        match self {
            Self::K21 => "kotlin-2.1",
            Self::K23 => "kotlin-2.3",
            Self::K24 => "kotlin-2.4",
        }
    }
}

pub trait KotlinGenerationDriver: Send + Sync {
    fn analyze(&self, request: &AnalyzeGenerationRequest) -> Result<Value, ClewError>;
}

pub struct KotlinAdapterV2<D> {
    line: KotlinCompilerLine,
    adapter_digest: String,
    toolchain_digest: String,
    store: CasStore,
    driver: D,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl<D> KotlinAdapterV2<D> {
    pub fn new(
        line: KotlinCompilerLine,
        adapter_digest: String,
        toolchain_digest: String,
        store: CasStore,
        driver: D,
    ) -> Result<Self, ClewError> {
        require_digest(&adapter_digest)?;
        require_digest(&toolchain_digest)?;
        Ok(Self {
            line,
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
            adapter_id: self.line.adapter_id().into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(KOTLIN_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?],
            toolchains: vec![ToolchainConstraint {
                authority_digest: self.toolchain_digest.clone(),
                minimum_version: Some(self.line.compiler_version().into()),
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
        if index.get("compilerVersion").and_then(Value::as_str)
            != Some(self.line.compiler_version())
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

    fn query_generation(
        &self,
        _request: &QueryGenerationRequest,
    ) -> Result<QueryGenerationResult, ClewError> {
        Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "generation queries are owned by the core query index",
        ))
    }

    fn validate_candidate(
        &self,
        _request: &ValidateCandidateRequest,
    ) -> Result<ValidateCandidateResult, ClewError> {
        Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "candidate validation is not available before candidate snapshot v2",
        ))
    }

    fn cancel(&self, attempt_id: &str) -> Result<(), ClewError> {
        if attempt_id.is_empty() || attempt_id.len() > 128 {
            return Err(invalid("Kotlin attempt identity is invalid"));
        }
        self.cancelled_attempts
            .lock()
            .map_err(poisoned)?
            .insert(attempt_id.into());
        Ok(())
    }

    fn shutdown(&self) -> Result<(), ClewError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
}

pub struct LegacyKotlinWorkerDriver {
    state: StateAuthority,
    store: CasStore,
    line: KotlinCompilerLine,
    adapter_digest: String,
    exclusive: Mutex<()>,
}

impl LegacyKotlinWorkerDriver {
    pub fn new(
        state: StateAuthority,
        store: CasStore,
        line: KotlinCompilerLine,
        adapter_digest: String,
    ) -> Self {
        Self {
            state,
            store,
            line,
            adapter_digest,
            exclusive: Mutex::new(()),
        }
    }

    fn load_snapshot(
        &self,
        request: &AnalyzeGenerationRequest,
    ) -> Result<RepositoryInputSnapshot, ClewError> {
        let manifest_limit =
            usize::try_from(request.derived_input_manifest.size).map_err(|_| {
                ClewError::new(
                    ErrorCode::ResourceLimit,
                    "derived manifest exceeds host size",
                )
            })?;
        let lease = self
            .store
            .read(&request.derived_input_manifest, manifest_limit)?;
        let manifest: DerivedAnalysisInputManifest = serde_json::from_slice(lease.bytes())
            .map_err(|_| invalid("derived input manifest is invalid"))?;
        manifest.verify(&self.store)?;
        let snapshot_limit = usize::try_from(manifest.repository_snapshot.size).map_err(|_| {
            ClewError::new(
                ErrorCode::ResourceLimit,
                "repository snapshot exceeds host size",
            )
        })?;
        let lease = self
            .store
            .read(&manifest.repository_snapshot, snapshot_limit)?;
        serde_json::from_slice(lease.bytes())
            .map_err(|_| invalid("repository input snapshot is invalid"))
    }
}

impl KotlinGenerationDriver for LegacyKotlinWorkerDriver {
    fn analyze(&self, request: &AnalyzeGenerationRequest) -> Result<Value, ClewError> {
        let _exclusive = self.exclusive.lock().map_err(poisoned)?;
        let snapshot = self.load_snapshot(request)?;
        let options_limit =
            usize::try_from(request.compilation.canonical_options.size).map_err(|_| {
                ClewError::new(ErrorCode::ResourceLimit, "Kotlin options exceed host size")
            })?;
        let options = self
            .store
            .read(&request.compilation.canonical_options, options_limit)?;
        let options: Value = serde_json::from_slice(options.bytes())
            .map_err(|_| invalid("Kotlin canonical options are invalid"))?;
        let native_compilation = options
            .get("nativeCompilation")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Kotlin canonical options have no nativeCompilation"))?;
        if let Some(parent) = &request.parent_generation {
            let limit = usize::try_from(parent.size).map_err(|_| {
                ClewError::new(
                    ErrorCode::ResourceLimit,
                    "parent generation exceeds host size",
                )
            })?;
            let lease = self.store.read(parent, limit)?;
            let parent: GenerationManifest = serde_json::from_slice(lease.bytes())
                .map_err(|_| invalid("parent generation is invalid"))?;
            parent.verify(&self.store)?;
        }
        let compiler_store_key = CompilerStoreKey::create(
            self.line.adapter_id(),
            self.adapter_digest.clone(),
            &request.compilation,
        )?;
        let index = analyze_project_native_index(
            &self.state,
            &self.store,
            &snapshot,
            native_compilation,
            compiler_store_key.path_component()?,
        )?;
        if index.get("compilerVersion").and_then(Value::as_str)
            != Some(self.line.compiler_version())
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker selected another Kotlin compiler line",
            ));
        }
        Ok(index)
    }
}

pub(crate) fn analyze_project_native_index(
    state: &StateAuthority,
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    native_compilation: &str,
    compiler_store_component: &str,
) -> Result<Value, ClewError> {
    if compiler_store_component.len() != 64
        || !compiler_store_component
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("compiler store component is invalid"));
    }
    let attempt_root = tempfile::Builder::new()
        .prefix("kotlin-generation-")
        .tempdir_in(state.attempts_root())
        .map_err(io_error)?;
    let repo = attempt_root.path().join("repo");
    materialize(snapshot, store, &repo)?;
    let derived_mounts = mount_project_derived_state(attempt_root.path(), &repo, snapshot)?;
    let compiler_store = state
        .root()
        .join("generations/compiler-store")
        .join(compiler_store_component);
    create_private_directory(&compiler_store)?;
    let compiler_store = compiler_store.canonicalize().map_err(io_error)?;
    let compiler_store_namespace = format!("sha256:{compiler_store_component}");
    let mut worker = WorkerClient::start_with_managed_states(
        &workspace_root(),
        None,
        Some(&compiler_store),
        &compiler_store_namespace,
    )?;
    let request = json!({
        "repo":repo,
        "compilation":native_compilation,
        "syntaxOnly":false,
    });
    let index = match worker.index_files_verified(&request) {
        Ok(verified) => worker.inspect_verified_index(&verified)?.clone(),
        Err(error) if error.code == ErrorCode::IncompleteSemanticAnalysis => {
            let files = snapshot
                .index
                .iter()
                .filter(|entry| entry.stage == 0 && entry.path.ends_with(".kt"))
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>();
            if files.is_empty() {
                return Err(error);
            }
            let syntax = worker.index_files_source_syntax_verified(&json!({
                "repo":repo,
                "compilation":native_compilation,
                "syntaxOnly":true,
                "files":files,
            }))?;
            let mut syntax = worker.inspect_verified_source_syntax(&syntax)?.clone();
            normalize_source_syntax_fallback(&mut syntax, &error)?;
            syntax
        }
        Err(error) => return Err(error),
    };
    worker.shutdown()?;
    unmount_project_derived_state(&repo, &derived_mounts)?;
    let (observed_snapshot, _) = capture(&repo, store)?;
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
            "project-native model extraction modified sealed repository inputs",
        ));
    }
    Ok(index)
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
        "schema":"codeclew-kotlin-index-metadata/2.0",
        "compilation":index.get("compilation"),
        "compilerVersion":index.get("compilerVersion"),
        "projectModelHash":index.get("projectModelHash"),
        "classpathHash":index.get("classpathHash"),
        "compilerOptionsHash":index.get("compilerOptionsHash"),
        "semanticInputManifestHash":index.get("semanticInputManifestHash"),
    });
    push_fact(&capability, "metadata", &metadata, &mut pending)?;
    for (category, pointer) in [
        ("file", "/files"),
        ("descriptor", "/declarationDescriptors/descriptors"),
        ("descriptor-boundary", "/declarationDescriptors/boundaries"),
        ("relation", "/declarationRelations/relations"),
        ("relation-boundary", "/declarationRelations/boundaries"),
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
            push_fact(&capability, category, row, &mut pending)?;
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
        "projectModelHash":index.get("projectModelHash"),
        "semanticInputManifestHash":index.get("semanticInputManifestHash"),
        "declarationDescriptorHash":index.get("declarationDescriptorHash"),
        "declarationRelationHash":index.get("declarationRelationHash"),
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
    let unsure = index.get("analysisCertainty").and_then(Value::as_str) == Some("UNSURE");
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
        ANALYSIS_REQUEST_SCHEMA, BuildModel, COMPILATION_SCHEMA, CompilationDescriptor,
        ConformanceSink, DescriptorCompleteness, DescriptorOrigin, PROVIDER_PROTOCOL,
        ProviderHandshake, ProviderModel, SourceRootDescriptor,
    };
    use crate::derived_manifest::DerivedAnalysisInputManifest;
    use crate::repository_snapshot;

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
                "files":[{"path":"src/Main.kt","contentHash":format!("sha256:{}", "7".repeat(64)),"declarations":[]}],
                "declarationDescriptors":{"coverage":"COMPLETE_SUPPORTED_SUBSET","descriptors":[],"boundaries":[]},
                "declarationRelations":{"coverage":"COMPLETE_SUPPORTED_SUBSET","relations":[],"boundaries":[]},
            }))
        }
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
            KotlinCompilerLine::K21,
            KotlinCompilerLine::K23,
            KotlinCompilerLine::K24,
        ] {
            let root = tempfile::tempdir().unwrap();
            let state = StateAuthority::open(root.path().join("v2")).unwrap();
            let store = CasStore::open(&state).unwrap();
            let toolchain = store
                .put("test/toolchain/1", line.compiler_version().as_bytes())
                .unwrap();
            let adapter = KotlinAdapterV2::new(
                line,
                format!("sha256:{}", "a".repeat(64)),
                toolchain.digest.clone(),
                store.clone(),
                FakeDriver {
                    version: line.compiler_version(),
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
                Some(line.compiler_version())
            );
        }
    }

    #[test]
    fn cancellation_is_checked_before_driver_execution() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let toolchain = store.put("test/toolchain/1", b"2.4.10").unwrap();
        let adapter = KotlinAdapterV2::new(
            KotlinCompilerLine::K24,
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

    #[test]
    #[ignore = "explicit cold-start acceptance using a real trusted Kotlin worker"]
    fn sealed_k24_snapshot_reaches_verified_streamed_generation() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let fixture = workspace_root().join("fixtures/kotlin-basic");
        let (_snapshot, snapshot_object) = repository_snapshot::capture(&fixture, &store).unwrap();
        let toolchain = store.put("test/toolchain/1", b"2.4.10").unwrap();
        let options = store
            .put("test/options/1", br#"{"nativeCompilation":":/main"}"#)
            .unwrap();
        let compilation = CompilationDescriptor {
            schema: COMPILATION_SCHEMA.into(),
            compilation_id: "root-main".into(),
            language_uri: LanguageUri::parse(KOTLIN_LANGUAGE).unwrap(),
            source_roots: vec![SourceRootDescriptor {
                logical_name: "main".into(),
                tree: store.put("test/tree/1", b"fixture").unwrap(),
            }],
            generated_source_roots: vec![],
            classpath: vec![],
            toolchain: toolchain.clone(),
            plugins: vec![],
            canonical_options: options,
            dependency_compilation_ids: vec![],
            operations: vec![],
            origin: DescriptorOrigin::ProjectNative,
            completeness: DescriptorCompleteness::Complete,
        };
        let provider = ProviderModel {
            handshake: ProviderHandshake {
                protocol: PROVIDER_PROTOCOL.into(),
                provider_id: "fixture-gradle".into(),
                provider_digest: format!("sha256:{}", "b".repeat(64)),
                build_system_uris: vec!["build:gradle".into()],
            },
            build_model: BuildModel {
                provider_id: "fixture-gradle".into(),
                model: store.put("test/model/1", b"fixture-model").unwrap(),
                compilations: vec![compilation.clone()],
            },
        };
        let (_manifest, manifest_object) =
            DerivedAnalysisInputManifest::create(&store, snapshot_object, vec![provider]).unwrap();
        let adapter_digest = format!("sha256:{}", "a".repeat(64));
        let driver = LegacyKotlinWorkerDriver::new(
            state,
            store.clone(),
            KotlinCompilerLine::K24,
            adapter_digest.clone(),
        );
        let adapter = KotlinAdapterV2::new(
            KotlinCompilerLine::K24,
            adapter_digest,
            toolchain.digest.clone(),
            store.clone(),
            driver,
        )
        .unwrap();
        let request = AnalyzeGenerationRequest {
            schema: ANALYSIS_REQUEST_SCHEMA.into(),
            attempt_id: "attempt:k24-sealed".into(),
            generation_key: format!("sha256:{}", "9".repeat(64)),
            capability: CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY).unwrap(),
            compilation,
            derived_input_manifest: manifest_object,
            parent_generation: None,
        };
        let mut sink =
            ConformanceSink::for_capabilities([
                CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY).unwrap()
            ]);
        adapter
            .analyze_generation(&request, &mut sink, &AtomicBool::new(false))
            .unwrap();
        assert!(sink.finish().unwrap().fact_count > 10);
    }
}
