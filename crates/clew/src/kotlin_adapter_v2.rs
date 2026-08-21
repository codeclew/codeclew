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
use crate::repository_snapshot::{RepositoryInputSnapshot, materialize};
use crate::state::{StateAuthority, create_private_directory};
use crate::worker::{WorkerClient, workspace_root};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

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
    exclusive: Mutex<()>,
}

impl LegacyKotlinWorkerDriver {
    pub fn new(state: StateAuthority, store: CasStore, line: KotlinCompilerLine) -> Self {
        Self {
            state,
            store,
            line,
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
        let attempt_root = tempfile::Builder::new()
            .prefix("kotlin-generation-")
            .tempdir_in(self.state.attempts_root())
            .map_err(io_error)?;
        let repo = attempt_root.path().join("repo");
        materialize(&snapshot, &self.store, &repo)?;
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
        let digest = request
            .compilation
            .toolchain
            .digest
            .strip_prefix("sha256:")
            .ok_or_else(|| invalid("Kotlin toolchain digest is invalid"))?;
        let compiler_store = self
            .state
            .root()
            .join("generations/compiler-store")
            .join(digest);
        create_private_directory(&compiler_store)?;
        let compiler_store = compiler_store.canonicalize().map_err(io_error)?;
        let mut worker =
            WorkerClient::start_with_states(&workspace_root(), None, Some(&compiler_store))?;
        let verified = worker.index_files_verified(&json!({
            "repo":repo,
            "compilation":native_compilation,
            "syntaxOnly":false,
        }))?;
        let index = worker.inspect_verified_index(&verified)?.clone();
        if index.get("compilerVersion").and_then(Value::as_str)
            != Some(self.line.compiler_version())
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker selected another Kotlin compiler line",
            ));
        }
        worker.shutdown()?;
        Ok(index)
    }
}

fn translate_facts(store: &CasStore, index: &Value) -> Result<Vec<FactRecord>, ClewError> {
    let capability = CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY)?;
    let mut facts = Vec::new();
    let metadata = json!({
        "schema":"codeclew-kotlin-index-metadata/2.0",
        "compilation":index.get("compilation"),
        "compilerVersion":index.get("compilerVersion"),
        "projectModelHash":index.get("projectModelHash"),
        "classpathHash":index.get("classpathHash"),
        "compilerOptionsHash":index.get("compilerOptionsHash"),
        "semanticInputManifestHash":index.get("semanticInputManifestHash"),
    });
    push_fact(store, &capability, "metadata", &metadata, &mut facts)?;
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
            push_fact(store, &capability, category, row, &mut facts)?;
        }
    }
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

fn push_fact(
    store: &CasStore,
    capability: &CapabilityUri,
    category: &str,
    value: &Value,
    output: &mut Vec<FactRecord>,
) -> Result<(), ClewError> {
    let bytes = canonical::bytes(value).map_err(internal)?;
    let hash = canonical::hash_bytes(&bytes);
    output.push(FactRecord {
        fact_key: format!("kotlin:{category}:{}", hash.trim_start_matches("sha256:")),
        domain_uri: capability.clone(),
        payload: store.put(FACT_PAYLOAD_SCHEMA, &bytes)?,
    });
    Ok(())
}

fn semantic_scope_digest(index: &Value) -> Result<String, ClewError> {
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

fn completeness_receipt(
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
    let complete = descriptor_coverage == "COMPLETE_SUPPORTED_SUBSET"
        && relation_coverage == "COMPLETE_SUPPORTED_SUBSET";
    let receipt = json!({
        "schema":RECEIPT_SCHEMA,
        "scopeDigest":scope_digest,
        "domains":[
            {
                "domain":KOTLIN_FACTS_CAPABILITY,
                "support":"SUPPORTED",
                "coverage":if complete { "COMPLETE" } else { "PARTIAL" },
                "certainty":"VERIFIED",
            }
        ],
        "obligations":if complete { Vec::<String>::new() } else { vec!["verify-partial-kotlin-boundaries".to_owned()] },
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
        let driver = LegacyKotlinWorkerDriver::new(state, store.clone(), KotlinCompilerLine::K24);
        let adapter = KotlinAdapterV2::new(
            KotlinCompilerLine::K24,
            format!("sha256:{}", "a".repeat(64)),
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
