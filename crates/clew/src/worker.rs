use crate::error::{ClewError, ErrorCode};
use crate::proto::{
    ApplyEditRequest, BatchRequest, BlobRef, BuildLocalGraphRequest, IndexFilesRequest,
    OpenProjectRequest, ProtocolVersion, RequestKind, ResolveExpressionRequest,
    ResolveSymbolRequest, SchemaVersion, ShutdownRequest, SnapshotId, ValidateCandidateRequest,
    WorkerRequest, WorkerResponse, worker_request, worker_response,
};
use prost::Message;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/worker_build_inputs.rs"));

#[derive(Debug, Clone, Copy, Default)]
pub struct RequestProfile {
    pub serialization_micros: u64,
    pub ipc_micros: u64,
    pub worker_processing_micros: u64,
    pub cache_requests: u64,
    pub cache_hits: u64,
    pub psi_parse_micros: u64,
    pub k2_analysis_micros: u64,
    pub fir_extraction_micros: u64,
}

pub struct WorkerClient {
    workspace: PathBuf,
    variant: WorkerVariant,
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    next_id: u64,
    snapshot: Option<SnapshotId>,
    pub capabilities: crate::proto::WorkerCapabilities,
    pub last_profile: RequestProfile,
    authority_session: Uuid,
    trusted_distribution: Option<TrustedWorkerDistribution>,
    issued_index_facts: BTreeMap<Uuid, String>,
}

/// Opaque authority capability for one exact, live `IndexFiles` response.
///
/// It deliberately has no `Clone`, serde implementation, public fields, or
/// constructor. Reading the index response does not grant authority to persist
/// proof-bearing compiler facts; only the issuing live `WorkerClient` can
/// replay this capability.
pub struct VerifiedIndexFacts {
    receipt_id: Uuid,
    authority_session: Uuid,
    repo: PathBuf,
    compilation: String,
    base_revision: String,
    project_model_hash: String,
    distribution_fingerprint: String,
    distribution_tree_hash: String,
    build_input_digest: String,
    payload_hash: String,
    relation_hash: String,
    descriptor_hash: String,
    payload: Value,
}

fn verified_index_failure_stage(error: &ClewError, default: &str) -> String {
    error
        .evidence
        .iter()
        .find_map(|value| value.strip_prefix("verified-index-stage:"))
        .unwrap_or(default)
        .to_owned()
}

fn attach_verified_index_failure(
    mut error: ClewError,
    default_stage: &str,
    facts: Option<&Value>,
) -> ClewError {
    let stage = verified_index_failure_stage(&error, default_stage);
    let relation_graph = facts.and_then(|value| value.get("declarationRelations"));
    let descriptor_graph = facts.and_then(|value| value.get("declarationDescriptors"));
    let relation_provenance = relation_graph.and_then(|value| value.get("provenance"));
    let descriptor_provenance = descriptor_graph.and_then(|value| value.get("provenance"));
    let safe_hash = |value: Option<&Value>| {
        crate::canonical::hash(value.unwrap_or(&Value::Null))
            .unwrap_or_else(|_| "unavailable".into())
    };
    let diagnostic = serde_json::json!({
        "schema":"verified-index-failure-diagnostic/0.1",
        "stage":stage,
        "rawSchemaHash":safe_hash(facts.and_then(|value| value.get("schema"))),
        "payloadHash":safe_hash(facts),
        "relationGraphHash":safe_hash(relation_graph),
        "descriptorGraphHash":safe_hash(descriptor_graph),
        "relationProvenanceHash":safe_hash(relation_provenance),
        "descriptorProvenanceHash":safe_hash(descriptor_provenance),
        "relationCount":relation_graph.and_then(|value| value.get("relations")).and_then(Value::as_array).map_or(0, Vec::len),
        "relationBoundaryCount":relation_graph.and_then(|value| value.get("boundaries")).and_then(Value::as_array).map_or(0, Vec::len),
        "descriptorCount":descriptor_graph.and_then(|value| value.get("descriptors")).and_then(Value::as_array).map_or(0, Vec::len),
        "descriptorBoundaryCount":descriptor_graph.and_then(|value| value.get("boundaries")).and_then(Value::as_array).map_or(0, Vec::len),
        "descriptorFailure":if stage == "DESCRIPTOR_GRAPH" {
            facts.map(crate::index::descriptor_validation_diagnostic).unwrap_or(Value::Null)
        } else {
            Value::Null
        },
    });
    if let Ok(encoded) = serde_json::to_string(&diagnostic) {
        error.evidence.push(encoded);
    }
    error
}

impl VerifiedIndexFacts {
    pub(crate) fn compilation(&self) -> &str {
        &self.compilation
    }
}

struct TrustedWorkerDistribution {
    workspace: PathBuf,
    _private_root: tempfile::TempDir,
    distribution_root: PathBuf,
    launcher: PathBuf,
    tree_manifest: BTreeMap<String, String>,
    tree_hash: String,
    build_input_digest: String,
    plugin_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerVariant {
    Kotlin21,
    Kotlin23,
    Kotlin24,
}

impl WorkerVariant {
    fn for_project(version: &str) -> Result<Self, ClewError> {
        match version
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".")
            .as_str()
        {
            "2.1" => Ok(Self::Kotlin21),
            "2.3" => Ok(Self::Kotlin23),
            "2.4" => Ok(Self::Kotlin24),
            _ => Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                format!(
                    "unsupported Kotlin compiler {version}; supported compiler lines are 2.1, 2.3, and 2.4"
                ),
            )),
        }
    }

    fn compiler_version(self) -> &'static str {
        match self {
            Self::Kotlin21 => "2.1.21",
            Self::Kotlin23 => "2.3.0",
            Self::Kotlin24 => "2.4.10",
        }
    }

    fn install_task(self) -> &'static str {
        match self {
            Self::Kotlin21 => ":workers:kotlin21:installDist",
            Self::Kotlin23 => ":workers:kotlin23:installDist",
            Self::Kotlin24 => ":workers:kotlin:installDist",
        }
    }

    fn distribution_relative(self) -> &'static str {
        match self {
            Self::Kotlin21 => "workers/kotlin21/build/install/kotlin21",
            Self::Kotlin23 => "workers/kotlin23/build/install/kotlin23",
            Self::Kotlin24 => "workers/kotlin/build/install/kotlin",
        }
    }

    fn launcher_name(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin21",
            Self::Kotlin23 => "kotlin23",
            Self::Kotlin24 => "kotlin",
        }
    }

    fn plugin_jar_name(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin21-0.1.0.jar",
            Self::Kotlin23 => "kotlin23-0.1.0.jar",
            Self::Kotlin24 => "kotlin-0.1.0.jar",
        }
    }

    fn pinned_inputs(self) -> PinnedInputs {
        match self {
            Self::Kotlin21 => PinnedInputs {
                roots: PINNED_KOTLIN21_INPUT_ROOTS,
                files: PINNED_KOTLIN21_INPUT_FILES,
                entries: PINNED_KOTLIN21_INPUTS,
                digest: PINNED_KOTLIN21_INPUT_DIGEST,
                outputs: PINNED_KOTLIN21_OUTPUTS,
                output_digest: PINNED_KOTLIN21_OUTPUT_DIGEST,
            },
            Self::Kotlin23 => PinnedInputs {
                roots: PINNED_KOTLIN23_INPUT_ROOTS,
                files: PINNED_KOTLIN23_INPUT_FILES,
                entries: PINNED_KOTLIN23_INPUTS,
                digest: PINNED_KOTLIN23_INPUT_DIGEST,
                outputs: PINNED_KOTLIN23_OUTPUTS,
                output_digest: PINNED_KOTLIN23_OUTPUT_DIGEST,
            },
            Self::Kotlin24 => PinnedInputs {
                roots: PINNED_KOTLIN24_INPUT_ROOTS,
                files: PINNED_KOTLIN24_INPUT_FILES,
                entries: PINNED_KOTLIN24_INPUTS,
                digest: PINNED_KOTLIN24_INPUT_DIGEST,
                outputs: PINNED_KOTLIN24_OUTPUTS,
                output_digest: PINNED_KOTLIN24_OUTPUT_DIGEST,
            },
        }
    }
}

struct PinnedInputs {
    roots: &'static [&'static str],
    files: &'static [&'static str],
    entries: &'static [(&'static str, &'static str)],
    digest: &'static str,
    outputs: &'static [(&'static str, u64, &'static str)],
    output_digest: &'static str,
}

impl WorkerClient {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn start(workspace: &Path) -> Result<Self, ClewError> {
        Self::start_variant(workspace, WorkerVariant::Kotlin24)
    }

    fn start_variant(workspace: &Path, variant: WorkerVariant) -> Result<Self, ClewError> {
        let trusted_distribution = prepare_trusted_worker_distribution(workspace, variant)?;
        let launcher = trusted_distribution
            .as_ref()
            .map(|trusted| trusted.launcher.clone())
            .unwrap_or_else(|| worker_launcher(workspace, variant));
        if trusted_distribution.is_none() && !launcher.is_file() {
            let output = Command::new(workspace.join("gradlew"))
                .args([variant.install_task(), "--no-daemon", "--quiet"])
                .current_dir(workspace)
                .output()
                .map_err(|e| {
                    ClewError::new(
                        ErrorCode::WorkerCrashed,
                        format!("cannot build Kotlin worker: {e}"),
                    )
                })?;
            if !output.stdout.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
            }
            if !output.status.success() {
                return Err(ClewError::new(
                    ErrorCode::WorkerCrashed,
                    "Kotlin worker build failed",
                ));
            }
        }
        let mut child = Command::new(&launcher)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                ClewError::new(
                    ErrorCode::WorkerCrashed,
                    format!("cannot start {}: {e}", launcher.display()),
                )
            })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = child.stdout.take().expect("piped stdout");
        let hello = read_message(&mut stdout)?;
        let capabilities = match hello.payload {
            Some(worker_response::Payload::Capabilities(value)) => value,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker did not send startup capabilities",
                ));
            }
        };
        if capabilities.compiler_version != variant.compiler_version()
            || !capabilities.protocol_versions.iter().any(|v| v.major == 1)
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker compiler/protocol version mismatch",
            ));
        }
        Ok(Self {
            workspace: workspace.to_path_buf(),
            variant,
            child,
            stdin,
            stdout,
            next_id: 1,
            snapshot: None,
            capabilities,
            last_profile: RequestProfile::default(),
            authority_session: Uuid::new_v4(),
            trusted_distribution,
            issued_index_facts: BTreeMap::new(),
        })
    }

    fn switch_variant(&mut self, variant: WorkerVariant) -> Result<(), ClewError> {
        if self.variant == variant {
            return Ok(());
        }
        let replacement = Self::start_variant(&self.workspace, variant)?;
        let previous = std::mem::replace(self, replacement);
        previous.shutdown()
    }

    pub fn request(&mut self, kind: RequestKind, payload: &Value) -> Result<Value, ClewError> {
        if self.snapshot.is_none()
            && !matches!(kind, RequestKind::OpenProject | RequestKind::Shutdown)
            && payload.get("repo").and_then(Value::as_str).is_some()
        {
            let bootstrap = serde_json::json!({
                "repo":payload.get("repo"),
                "compilation":payload.get("compilation")
            });
            let _ = self.request(RequestKind::OpenProject, &bootstrap)?;
        }
        let request_id = self.next_id;
        self.next_id += 1;
        let request_serialization_started = Instant::now();
        let schema_version = || Some(SchemaVersion { major: 1, minor: 0 });
        let repo = || json_string(payload, "repo");
        let compilation = || json_optional_string(payload, "compilation");
        let request_payload = match kind {
            RequestKind::OpenProject => worker_request::Payload::OpenProject(OpenProjectRequest {
                schema_version: schema_version(),
                repo: repo(),
                compilation: compilation(),
            }),
            RequestKind::IndexFiles => worker_request::Payload::IndexFiles(IndexFilesRequest {
                schema_version: schema_version(),
                repo: repo(),
                compilation: compilation(),
                syntax_only: payload
                    .get("syntaxOnly")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                relative_files: payload
                    .get("files")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            }),
            RequestKind::ResolveSymbol => {
                worker_request::Payload::ResolveSymbol(ResolveSymbolRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    symbol: json_string(payload, "symbol"),
                    compilation: compilation(),
                })
            }
            RequestKind::ResolveExpression => {
                worker_request::Payload::ResolveExpression(ResolveExpressionRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    file: json_string(payload, "file"),
                    offset: payload.get("offset").and_then(Value::as_u64).unwrap_or(0),
                    compilation: compilation(),
                })
            }
            RequestKind::BuildLocalGraph => {
                worker_request::Payload::BuildLocalGraph(BuildLocalGraphRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    symbol: json_string(payload, "symbol"),
                    compilation: compilation(),
                })
            }
            RequestKind::ApplyEdit => {
                let (source_inline, source_blob) = source_transport(payload)?;
                worker_request::Payload::ApplyEdit(ApplyEditRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    file: json_string(payload, "file"),
                    source_inline,
                    source_blob,
                    owner_symbol_id: json_string(payload, "ownerSymbolId"),
                    exact_text_hash: json_string(payload, "exactTextHash"),
                    syntax_kind: json_string(payload, "syntaxKind"),
                    normalized_token_hash: json_string(payload, "normalizedTokenHash"),
                    ancestor_path_hash: json_string(payload, "ancestorPathHash"),
                    local_ordinal: payload.get("localOrdinal").and_then(Value::as_u64),
                    left_context_hash: json_string(payload, "leftContextHash"),
                    right_context_hash: json_string(payload, "rightContextHash"),
                    operation_kind: json_string(payload, "kind"),
                    replacement: json_string(payload, "replacement"),
                    preconditions_json: serde_json::to_vec(
                        payload.get("preconditions").unwrap_or(&Value::Null),
                    )
                    .map_err(internal)?,
                    postconditions_json: serde_json::to_vec(
                        payload.get("postconditions").unwrap_or(&Value::Null),
                    )
                    .map_err(internal)?,
                    compilation: compilation(),
                    defer_semantic_validation: payload
                        .get("deferSemanticValidation")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    semantic_operation_json: serde_json::to_vec(
                        payload.get("semanticOperation").unwrap_or(&Value::Null),
                    )
                    .map_err(internal)?,
                })
            }
            RequestKind::ValidateCandidate => {
                let (source_inline, source_blob) = source_transport(payload)?;
                worker_request::Payload::ValidateCandidate(ValidateCandidateRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    file: json_string(payload, "file"),
                    source_inline,
                    source_blob,
                })
            }
            RequestKind::Shutdown => worker_request::Payload::Shutdown(ShutdownRequest {
                schema_version: schema_version(),
            }),
            _ => {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "unspecified worker request kind",
                ));
            }
        };
        let request = WorkerRequest {
            request_id,
            protocol_version: Some(ProtocolVersion { major: 1, minor: 0 }),
            snapshot: Some(
                self.snapshot
                    .clone()
                    .unwrap_or_else(|| snapshot_from(payload)),
            ),
            payload: Some(request_payload),
        };
        let request_construction_micros =
            request_serialization_started.elapsed().as_micros() as u64;
        let (encode_micros, write_micros) = write_message_profiled(&mut self.stdin, &request)?;
        let (response, read_micros, decode_micros) = read_message_profiled(&mut self.stdout)?;
        if response.request_id != request_id {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker response request_id mismatch",
            ));
        }
        if response
            .protocol_version
            .as_ref()
            .is_none_or(|version| version.major != 1 || version.minor != 0)
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker response protocol identity mismatch",
            ));
        }
        let canonical_json = match response.payload {
            Some(worker_response::Payload::Error(error)) => {
                let relevant: Vec<String> = payload
                    .get("ownerSymbolId")
                    .or_else(|| payload.get("symbol"))
                    .or_else(|| payload.get("exactTextHash"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .into_iter()
                    .collect();
                return Err(ClewError {
                    code: parse_worker_code(&error.code),
                    message: error.message,
                    transaction_id: None,
                    snapshot_id: self.snapshot.as_ref().map(snapshot_label),
                    evidence: error.evidence,
                    relevant_anchors_or_symbols: relevant.into_boxed_slice(),
                    retryable: error.retryable,
                });
            }
            Some(worker_response::Payload::OpenProject(value)) => {
                validate_typed_string(
                    &value.canonical_json,
                    "/projectModelHash",
                    &value.project_model_hash,
                )?;
                if value.compilation.is_empty() {
                    return Err(ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "typed OpenProject response has no compilation identity",
                    ));
                }
                let mut canonical: Value =
                    serde_json::from_slice(&value.canonical_json).map_err(internal)?;
                bind_open_project_compilation(&mut canonical, &value.compilation)?;
                crate::canonical::bytes(&canonical).map_err(internal)?
            }
            Some(worker_response::Payload::IndexFiles(value)) => {
                validate_typed_string(&value.canonical_json, "/indexHash", &value.index_hash)?;
                validate_typed_string(
                    &value.canonical_json,
                    "/projectModelHash",
                    &value.project_model_hash,
                )?;
                validate_typed_string(&value.canonical_json, "/compilation", &value.compilation)?;
                validate_typed_count(&value.canonical_json, "/files", value.file_count)?;
                validate_typed_bool(&value.canonical_json, "/partial", value.partial)?;
                value.canonical_json
            }
            Some(worker_response::Payload::ResolveSymbol(value)) => {
                validate_typed_string(
                    &value.canonical_json,
                    "/declaration/symbolId",
                    &value.symbol_id,
                )?;
                value.canonical_json
            }
            Some(worker_response::Payload::ResolveExpression(value)) => {
                validate_typed_string(&value.canonical_json, "/anchor/anchorId", &value.anchor_id)?;
                value.canonical_json
            }
            Some(worker_response::Payload::BuildLocalGraph(value)) => {
                validate_typed_string(&value.canonical_json, "/symbol", &value.symbol_id)?;
                validate_typed_count(&value.canonical_json, "/nodes", value.node_count)?;
                validate_typed_count(&value.canonical_json, "/edges", value.edge_count)?;
                value.canonical_json
            }
            Some(worker_response::Payload::ApplyEdit(value)) => {
                validate_typed_string(
                    &value.canonical_json,
                    "/candidateHash",
                    &value.candidate_hash,
                )?;
                value.canonical_json
            }
            Some(worker_response::Payload::ValidateCandidate(value)) => {
                let canonical: Value =
                    serde_json::from_slice(&value.canonical_json).map_err(internal)?;
                if canonical.get("valid").and_then(Value::as_bool) != Some(value.valid) {
                    return Err(ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "typed validation result disagrees with canonical payload",
                    ));
                }
                value.canonical_json
            }
            Some(worker_response::Payload::Shutdown(value)) => value.canonical_json,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker response has unexpected payload",
                ));
            }
        };
        let json_started = Instant::now();
        let mut value: Value = serde_json::from_slice(&canonical_json).map_err(internal)?;
        let json_micros = json_started.elapsed().as_micros() as u64;
        if kind == RequestKind::ApplyEdit && value.get("source").is_none() {
            let blob = value.get("sourceBlob").ok_or_else(|| {
                ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "candidate response has neither inline source nor BlobRef",
                )
            })?;
            let source = read_response_blob(payload, blob)?;
            value["source"] = Value::String(String::from_utf8(source).map_err(|error| {
                ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
            })?);
        }
        if kind == RequestKind::OpenProject {
            let project_compiler = value
                .get("compilerVersion")
                .and_then(Value::as_str)
                .unwrap_or(self.capabilities.compiler_version.as_str());
            let desired = WorkerVariant::for_project(project_compiler)?;
            if desired != self.variant {
                self.switch_variant(desired)?;
                return self.request(kind, payload);
            }
            self.snapshot = Some(SnapshotId {
                base_revision: snapshot_from(payload).base_revision,
                project_model_hash: value
                    .get("projectModelHash")
                    .and_then(Value::as_str)
                    .unwrap_or("UNRESOLVED")
                    .into(),
            });
        }
        let worker_processing_micros = value
            .pointer("/profiling/workerProcessingMicros")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.last_profile = RequestProfile {
            serialization_micros: request_construction_micros
                + encode_micros
                + decode_micros
                + json_micros,
            ipc_micros: (write_micros + read_micros).saturating_sub(worker_processing_micros),
            worker_processing_micros,
            cache_requests: value
                .pointer("/profiling/cacheRequests")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_hits: value
                .pointer("/profiling/cacheHits")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            psi_parse_micros: value
                .pointer("/profiling/psiParseMicros")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            k2_analysis_micros: value
                .pointer("/profiling/k2AnalysisMicros")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            fir_extraction_micros: value
                .pointer("/profiling/firExtractionMicros")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        };
        if let Some(object) = value.as_object_mut() {
            object.remove("profiling");
        }
        Ok(value)
    }

    /// Execute OpenProject + IndexFiles through the pinned worker distribution
    /// and issue an unforgeable, session-local capability for the exact result.
    pub fn index_files_verified(
        &mut self,
        payload: &Value,
    ) -> Result<VerifiedIndexFacts, ClewError> {
        let repo = payload
            .get("repo")
            .and_then(Value::as_str)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "verified index needs repo"))?;
        let repo = Path::new(repo).canonicalize().map_err(internal)?;
        let requested_compilation = payload
            .get("compilation")
            .and_then(Value::as_str)
            .unwrap_or(":/main")
            .to_owned();
        let trusted = self.trusted_distribution.as_ref().ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "verified index facts require the pinned workspace worker distribution",
            )
        })?;
        if trusted.workspace != workspace_root() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "verified index worker workspace identity changed",
            ));
        }
        let trusted_plugin_fingerprint = trusted.plugin_fingerprint.clone();
        verify_trusted_distribution(trusted)?;
        let trusted_tree_hash = trusted.tree_hash.clone();
        let trusted_build_input_digest = trusted.build_input_digest.clone();
        let project = self.request(
            RequestKind::OpenProject,
            &serde_json::json!({"repo":repo,"compilation":requested_compilation}),
        )?;
        if project.get("compilation").and_then(Value::as_str)
            != Some(requested_compilation.as_str())
            || project.get("compilerVersion").and_then(Value::as_str)
                != Some(self.capabilities.compiler_version.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "OpenProject identity differs from verified index request",
            ));
        }
        let project_model_hash = required_payload_string(&project, "projectModelHash")?.to_owned();
        let mut exact_payload = payload.clone();
        exact_payload["repo"] = Value::String(repo.to_string_lossy().into_owned());
        exact_payload["compilation"] = Value::String(requested_compilation.clone());
        let facts = self
            .request(RequestKind::IndexFiles, &exact_payload)
            .map_err(|error| attach_verified_index_failure(error, "RAW_SCHEMA_HASH", None))?;
        if facts.get("compilation").and_then(Value::as_str) != Some(requested_compilation.as_str())
            || facts.get("projectModelHash").and_then(Value::as_str)
                != Some(project_model_hash.as_str())
            || facts.get("compilerVersion").and_then(Value::as_str)
                != Some(self.capabilities.compiler_version.as_str())
        {
            return Err(attach_verified_index_failure(
                ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "IndexFiles identity differs from OpenProject/request",
                ),
                "SOURCE_BINDING",
                Some(&facts),
            ));
        }
        let descriptor =
            crate::index::validate_declaration_descriptor_snapshot(&facts).map_err(|error| {
                attach_verified_index_failure(error, "DESCRIPTOR_GRAPH", Some(&facts))
            })?;
        let relation =
            crate::index::validate_declaration_relation_snapshot(&facts).map_err(|error| {
                attach_verified_index_failure(error, "RELATION_GRAPH", Some(&facts))
            })?;
        for provenance in [&relation.provenance, &descriptor.provenance] {
            if provenance
                .get("pluginArtifactFingerprint")
                .and_then(Value::as_str)
                != Some(trusted_plugin_fingerprint.as_str())
                || provenance.get("workerVersion").and_then(Value::as_str)
                    != Some(self.capabilities.worker_version.as_str())
                || provenance
                    .get("workerCompilerVersion")
                    .and_then(Value::as_str)
                    != Some(self.capabilities.compiler_version.as_str())
                || provenance
                    .get("workerProtocolVersion")
                    .and_then(Value::as_str)
                    != Some("1.0")
            {
                return Err(attach_verified_index_failure(
                    ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "semantic graph provenance differs from live worker distribution",
                    ),
                    "DISTRIBUTION_PROVENANCE",
                    Some(&facts),
                ));
            }
        }
        let base_revision = git_revision(&repo)?;
        let payload_hash = crate::canonical::hash(&facts).map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        let seal = crate::canonical::hash(&serde_json::json!({
            "receiptId":receipt_id,
            "session":self.authority_session,
            "repo":repo,
            "compilation":requested_compilation,
            "baseRevision":base_revision,
            "projectModelHash":project_model_hash,
            "distribution":trusted_plugin_fingerprint,
            "distributionTree":trusted_tree_hash,
            "buildInputs":trusted_build_input_digest,
            "payloadHash":payload_hash,
            "relationHash":relation.hash,
            "descriptorHash":descriptor.hash,
        }))
        .map_err(internal)?;
        self.issued_index_facts.insert(receipt_id, seal);
        Ok(VerifiedIndexFacts {
            receipt_id,
            authority_session: self.authority_session,
            repo,
            compilation: requested_compilation,
            base_revision,
            project_model_hash,
            distribution_fingerprint: trusted_plugin_fingerprint,
            distribution_tree_hash: trusted_tree_hash,
            build_input_digest: trusted_build_input_digest,
            payload_hash,
            relation_hash: relation.hash,
            descriptor_hash: descriptor.hash,
            payload: facts,
        })
    }

    /// Read-only projection of a capability issued by this exact session.
    pub fn inspect_verified_index<'a>(
        &self,
        facts: &'a VerifiedIndexFacts,
    ) -> Result<&'a Value, ClewError> {
        self.authorize_index_facts(facts, &facts.repo, &facts.compilation)
    }

    pub(crate) fn safe_verified_index_diagnostic(&self, facts: &VerifiedIndexFacts) -> Value {
        serde_json::json!({
            "authoritySessionHash": crate::canonical::hash(&facts.authority_session).unwrap_or_else(|_| "unavailable".into()),
            "currentSessionHash": crate::canonical::hash(&self.authority_session).unwrap_or_else(|_| "unavailable".into()),
            "distributionFingerprintHash": crate::canonical::hash(&facts.distribution_fingerprint).unwrap_or_else(|_| "unavailable".into()),
            "distributionTreeHash": crate::canonical::hash(&facts.distribution_tree_hash).unwrap_or_else(|_| "unavailable".into()),
            "buildInputDigestHash": crate::canonical::hash(&facts.build_input_digest).unwrap_or_else(|_| "unavailable".into()),
            "pluginFingerprintHash": crate::canonical::hash(&facts.distribution_fingerprint).unwrap_or_else(|_| "unavailable".into()),
            "projectModelHashHash": crate::canonical::hash(&facts.project_model_hash).unwrap_or_else(|_| "unavailable".into()),
            "sourceSnapshotHash": crate::canonical::hash(&facts.base_revision).unwrap_or_else(|_| "unavailable".into()),
            "payloadHashHash": crate::canonical::hash(&facts.payload_hash).unwrap_or_else(|_| "unavailable".into()),
            "receiptIssued": self.issued_index_facts.contains_key(&facts.receipt_id),
            "sessionMatches": facts.authority_session == self.authority_session,
        })
    }

    pub(crate) fn authorize_index_facts<'a>(
        &self,
        facts: &'a VerifiedIndexFacts,
        source_root: &Path,
        compilation: &str,
    ) -> Result<&'a Value, ClewError> {
        let source_root = source_root.canonicalize().map_err(internal)?;
        let trusted = self.trusted_distribution.as_ref().ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker session is not trusted",
            )
        })?;
        verify_trusted_distribution(trusted)?;
        let seal = crate::canonical::hash(&serde_json::json!({
            "receiptId":facts.receipt_id,
            "session":facts.authority_session,
            "repo":facts.repo,
            "compilation":facts.compilation,
            "baseRevision":facts.base_revision,
            "projectModelHash":facts.project_model_hash,
            "distribution":facts.distribution_fingerprint,
            "distributionTree":facts.distribution_tree_hash,
            "buildInputs":facts.build_input_digest,
            "payloadHash":facts.payload_hash,
            "relationHash":facts.relation_hash,
            "descriptorHash":facts.descriptor_hash,
        }))
        .map_err(internal)?;
        if facts.authority_session != self.authority_session
            || source_root != facts.repo
            || compilation != facts.compilation
            || facts.distribution_fingerprint != trusted.plugin_fingerprint
            || facts.distribution_tree_hash != trusted.tree_hash
            || facts.build_input_digest != trusted.build_input_digest
            || crate::canonical::hash(&facts.payload).map_err(internal)? != facts.payload_hash
            || facts
                .payload
                .get("projectModelHash")
                .and_then(Value::as_str)
                != Some(facts.project_model_hash.as_str())
            || git_revision(&source_root)? != facts.base_revision
            || self.issued_index_facts.get(&facts.receipt_id) != Some(&seal)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "verified index capability is stale, altered, forged, or belongs to another session",
            ));
        }
        let relation = crate::index::validate_declaration_relation_snapshot(&facts.payload)?;
        let descriptor = crate::index::validate_declaration_descriptor_snapshot(&facts.payload)?;
        if relation.hash != facts.relation_hash || descriptor.hash != facts.descriptor_hash {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "verified semantic graph hash changed after issuance",
            ));
        }
        Ok(&facts.payload)
    }

    pub fn shutdown(mut self) -> Result<(), ClewError> {
        let _ = self.request(RequestKind::Shutdown, &serde_json::json!({}))?;
        let status = self
            .child
            .wait()
            .map_err(|e| ClewError::new(ErrorCode::WorkerCrashed, e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("worker exited with {status}"),
            ))
        }
    }

    pub fn validate_candidates_batch(
        &mut self,
        candidates: &[(String, String)],
    ) -> Result<Vec<Value>, ClewError> {
        let snapshot = self.snapshot.clone().unwrap_or(SnapshotId {
            base_revision: "UNVERSIONED".into(),
            project_model_hash: "UNRESOLVED".into(),
        });
        let mut requests = Vec::with_capacity(candidates.len());
        for (file, source) in candidates {
            let request_id = self.next_id;
            self.next_id += 1;
            requests.push(WorkerRequest {
                request_id,
                protocol_version: Some(ProtocolVersion { major: 1, minor: 0 }),
                snapshot: Some(snapshot.clone()),
                payload: Some(worker_request::Payload::ValidateCandidate(
                    ValidateCandidateRequest {
                        schema_version: Some(SchemaVersion { major: 1, minor: 0 }),
                        repo: String::new(),
                        file: file.clone(),
                        source_inline: source.as_bytes().to_vec(),
                        source_blob: None,
                    },
                )),
            });
        }
        let request_id = self.next_id;
        self.next_id += 1;
        write_message(
            &mut self.stdin,
            &WorkerRequest {
                request_id,
                protocol_version: Some(ProtocolVersion { major: 1, minor: 0 }),
                snapshot: Some(snapshot),
                payload: Some(worker_request::Payload::Batch(BatchRequest {
                    schema_version: Some(SchemaVersion { major: 1, minor: 0 }),
                    requests,
                })),
            },
        )?;
        let response = read_message(&mut self.stdout)?;
        if response.request_id != request_id {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "batch response request_id mismatch",
            ));
        }
        let batch = match response.payload {
            Some(worker_response::Payload::Batch(batch)) => batch,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker returned no BatchResponse",
                ));
            }
        };
        batch
            .responses
            .into_iter()
            .map(|response| match response.payload {
                Some(worker_response::Payload::ValidateCandidate(value)) => {
                    serde_json::from_slice(&value.canonical_json).map_err(internal)
                }
                Some(worker_response::Payload::Error(error)) => Err(ClewError {
                    code: parse_worker_code(&error.code),
                    message: error.message,
                    transaction_id: None,
                    snapshot_id: None,
                    evidence: error.evidence,
                    relevant_anchors_or_symbols: vec!["batch-validation".into()].into_boxed_slice(),
                    retryable: error.retryable,
                }),
                _ => Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "unexpected response inside batch",
                )),
            })
            .collect()
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn json_string(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}

fn json_optional_string(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn snapshot_from(payload: &Value) -> SnapshotId {
    let repo = payload.get("repo").and_then(Value::as_str);
    let base_revision = repo
        .and_then(|repo| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(repo)
                .output()
                .ok()
                .filter(|output| output.status.success())
        })
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "UNVERSIONED".into());
    SnapshotId {
        base_revision,
        project_model_hash: payload
            .get("projectModelHash")
            .and_then(Value::as_str)
            .unwrap_or("UNRESOLVED")
            .into(),
    }
}

fn snapshot_label(snapshot: &SnapshotId) -> String {
    format!(
        "snapshot:{}:{}",
        snapshot.base_revision, snapshot.project_model_hash
    )
}

fn source_transport(payload: &Value) -> Result<(Vec<u8>, Option<BlobRef>), ClewError> {
    let source = payload
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .as_bytes()
        .to_vec();
    if source.len() <= 64 * 1024 {
        return Ok((source, None));
    }
    let repo = payload
        .get("repo")
        .and_then(Value::as_str)
        .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "large source needs repo"))?;
    let hash = crate::canonical::hash_bytes(&source);
    let relative = format!(
        ".semantic-thread/blobs/sha256/{}",
        hash.trim_start_matches("sha256:")
    );
    let path = Path::new(repo).join(&relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ClewError::new(ErrorCode::Internal, format!("blob store: {error}")))?;
    }
    if !path.exists() {
        std::fs::write(&path, &source)
            .map_err(|error| ClewError::new(ErrorCode::Internal, format!("blob store: {error}")))?;
    }
    Ok((
        vec![],
        Some(BlobRef {
            content_hash: hash,
            relative_path: relative,
            size_bytes: source.len() as u64,
        }),
    ))
}

fn validate_typed_string(canonical: &[u8], pointer: &str, typed: &str) -> Result<(), ClewError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value.pointer(pointer).and_then(Value::as_str) != Some(typed) {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response field disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn validate_typed_count(canonical: &[u8], pointer: &str, typed: u64) -> Result<(), ClewError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| items.len() as u64)
        != Some(typed)
    {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response count disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn validate_typed_bool(canonical: &[u8], pointer: &str, typed: bool) -> Result<(), ClewError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value.pointer(pointer).and_then(Value::as_bool) != Some(typed) {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response boolean disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn read_response_blob(payload: &Value, blob: &Value) -> Result<Vec<u8>, ClewError> {
    let repo = payload
        .get("repo")
        .and_then(Value::as_str)
        .ok_or_else(|| ClewError::new(ErrorCode::WorkerProtocolMismatch, "BlobRef has no repo"))?;
    let relative = blob
        .get("relativePath")
        .and_then(Value::as_str)
        .ok_or_else(|| ClewError::new(ErrorCode::WorkerProtocolMismatch, "BlobRef has no path"))?;
    if relative.starts_with('/') || relative.split('/').any(|part| part == "..") {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "response BlobRef escapes repository",
        ));
    }
    let bytes = std::fs::read(Path::new(repo).join(relative))
        .map_err(|error| ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string()))?;
    let expected = blob
        .get("contentHash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if crate::canonical::hash_bytes(&bytes) != expected {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "response BlobRef content hash mismatch",
        ));
    }
    Ok(bytes)
}

fn worker_launcher(workspace: &Path, variant: WorkerVariant) -> PathBuf {
    match variant {
        WorkerVariant::Kotlin21 => {
            workspace.join("workers/kotlin21/build/install/kotlin21/bin/kotlin21")
        }
        WorkerVariant::Kotlin23 => {
            workspace.join("workers/kotlin23/build/install/kotlin23/bin/kotlin23")
        }
        WorkerVariant::Kotlin24 => workspace.join("workers/kotlin/build/install/kotlin/bin/kotlin"),
    }
}

fn prepare_trusted_worker_distribution(
    workspace: &Path,
    variant: WorkerVariant,
) -> Result<Option<TrustedWorkerDistribution>, ClewError> {
    let canonical = workspace.canonicalize().map_err(internal)?;
    if canonical != workspace_root() {
        return Ok(None);
    }
    let pinned = variant.pinned_inputs();
    reject_build_injection_environment(&canonical)?;
    verify_pinned_build_inputs(&canonical, &pinned)?;
    let source_distribution = canonical.join(variant.distribution_relative());
    verify_pinned_distribution(&source_distribution, &pinned)?;
    let private_root = tempfile::Builder::new()
        .prefix("codeclew-worker-authority-")
        .tempdir()
        .map_err(internal)?;
    let distribution_root = private_root.path().join("distribution");
    copy_regular_tree(&source_distribution, &distribution_root)?;
    let tree_manifest = regular_tree_manifest(&distribution_root)?;
    let tree_hash = hash_string_manifest(&tree_manifest);
    if tree_hash != pinned.output_digest {
        return Err(preparation_required(
            "private worker copy differs from embedded expected distribution",
        ));
    }
    let launcher = distribution_root.join("bin").join(variant.launcher_name());
    let plugin = distribution_root
        .join("lib")
        .join(variant.plugin_jar_name());
    if !launcher.is_file() || !plugin.is_file() {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "rebuilt pinned worker distribution is incomplete",
        ));
    }
    Ok(Some(TrustedWorkerDistribution {
        workspace: canonical,
        _private_root: private_root,
        distribution_root,
        launcher,
        tree_manifest,
        tree_hash,
        build_input_digest: pinned.digest.to_owned(),
        plugin_fingerprint: crate::canonical::hash_bytes(&std::fs::read(plugin).map_err(internal)?),
    }))
}

fn verify_pinned_build_inputs(workspace: &Path, pinned: &PinnedInputs) -> Result<(), ClewError> {
    let actual = collect_regular_inputs(workspace, pinned.roots, pinned.files)?;
    let expected = pinned
        .entries
        .iter()
        .map(|(path, hash)| ((*path).to_owned(), (*hash).to_owned()))
        .collect::<BTreeMap<_, _>>();
    if actual != expected || hash_input_manifest(&actual) != pinned.digest {
        return Err(preparation_required(
            "worker build input closure differs from embedded pinned manifest",
        ));
    }
    Ok(())
}

fn reject_build_injection_environment(workspace: &Path) -> Result<(), ClewError> {
    for key in [
        "GRADLE_OPTS",
        "GRADLE_USER_HOME",
        "JAVA_TOOL_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "_JAVA_OPTIONS",
    ] {
        if std::env::var_os(key).is_some_and(|value| !value.is_empty()) {
            return Err(preparation_required(
                "trusted worker start refuses Gradle/JVM injection environment",
            ));
        }
    }
    if std::env::vars_os().any(|(key, _)| key.to_string_lossy().starts_with("ORG_GRADLE_PROJECT_"))
        || workspace.join("init.gradle").exists()
        || workspace.join("init.gradle.kts").exists()
    {
        return Err(preparation_required(
            "trusted worker start refuses caller Gradle initialization",
        ));
    }
    Ok(())
}

fn verify_pinned_distribution(root: &Path, pinned: &PinnedInputs) -> Result<(), ClewError> {
    let actual = regular_tree_manifest(root)?;
    let expected = pinned
        .outputs
        .iter()
        .map(|(path, size, hash)| ((*path).to_owned(), format!("{size}:{hash}")))
        .collect::<BTreeMap<_, _>>();
    if actual != expected || hash_string_manifest(&actual) != pinned.output_digest {
        return Err(preparation_required(
            "worker installDist differs from embedded expected output manifest; run the trusted distribution build tool",
        ));
    }
    Ok(())
}

fn preparation_required(message: &str) -> ClewError {
    ClewError::new(ErrorCode::WorkerPreparationRequired, message)
}

fn collect_regular_inputs(
    workspace: &Path,
    roots: &[&str],
    files: &[&str],
) -> Result<BTreeMap<String, String>, ClewError> {
    let mut paths = Vec::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(workspace.join(root)).follow_links(false) {
            let entry = entry.map_err(internal)?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(internal)?;
            if metadata.file_type().is_symlink() {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker build input closure contains a symlink",
                ));
            }
            if metadata.is_file() {
                paths.push(entry.path().to_path_buf());
            }
        }
    }
    paths.extend(files.iter().map(|path| workspace.join(path)));
    paths.sort();
    paths.dedup();
    let mut manifest = BTreeMap::new();
    for path in paths {
        let metadata = std::fs::symlink_metadata(&path).map_err(internal)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker build input is missing or is not a regular file",
            ));
        }
        let relative = path
            .strip_prefix(workspace)
            .map_err(internal)?
            .to_string_lossy()
            .replace('\\', "/");
        manifest.insert(
            relative,
            crate::canonical::hash_bytes(&std::fs::read(path).map_err(internal)?),
        );
    }
    Ok(manifest)
}

fn copy_regular_tree(source: &Path, target: &Path) -> Result<(), ClewError> {
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(internal)?;
        let relative = entry.path().strip_prefix(source).map_err(internal)?;
        let destination = target.join(relative);
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(internal)?;
        if metadata.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "rebuilt worker distribution contains a symlink",
            ));
        }
        if metadata.is_dir() {
            std::fs::create_dir_all(&destination).map_err(internal)?;
        } else if metadata.is_file() {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(internal)?;
            }
            std::fs::copy(entry.path(), &destination).map_err(internal)?;
            std::fs::set_permissions(&destination, metadata.permissions()).map_err(internal)?;
        } else {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "rebuilt worker distribution contains a special file",
            ));
        }
    }
    Ok(())
}

fn regular_tree_manifest(root: &Path) -> Result<BTreeMap<String, String>, ClewError> {
    let mut manifest = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(internal)?;
        let relative = entry.path().strip_prefix(root).map_err(internal)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let path = relative.to_string_lossy().replace('\\', "/");
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(internal)?;
        if metadata.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "private worker distribution contains a symlink",
            ));
        }
        if metadata.is_dir() {
            continue;
        } else if metadata.is_file() {
            let size = metadata.len();
            manifest.insert(
                path,
                format!(
                    "{size}:{}",
                    crate::canonical::hash_bytes(&std::fs::read(entry.path()).map_err(internal)?)
                ),
            );
        } else {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "private worker distribution contains a special file",
            ));
        }
    }
    Ok(manifest)
}

fn hash_string_manifest(manifest: &BTreeMap<String, String>) -> String {
    let mut bytes = Vec::new();
    for (path, size_and_hash) in manifest {
        let (size, hash) = size_and_hash.split_once(':').unwrap_or(("", size_and_hash));
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(size.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(hash.as_bytes());
        bytes.push(0);
    }
    crate::canonical::hash_bytes(&bytes)
}

fn hash_input_manifest(manifest: &BTreeMap<String, String>) -> String {
    let mut bytes = Vec::new();
    for (path, hash) in manifest {
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(hash.as_bytes());
        bytes.push(0);
    }
    crate::canonical::hash_bytes(&bytes)
}

fn verify_trusted_distribution(trusted: &TrustedWorkerDistribution) -> Result<(), ClewError> {
    let actual = regular_tree_manifest(&trusted.distribution_root)?;
    if actual != trusted.tree_manifest || hash_string_manifest(&actual) != trusted.tree_hash {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "private worker distribution changed after trusted launch",
        ));
    }
    Ok(())
}

fn required_payload_string<'a>(payload: &'a Value, field: &str) -> Result<&'a str, ClewError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                format!("worker response has no {field}"),
            )
        })
}

fn bind_open_project_compilation(
    canonical: &mut Value,
    typed_compilation: &str,
) -> Result<(), ClewError> {
    if typed_compilation.is_empty() {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "typed OpenProject response has no compilation identity",
        ));
    }
    let module = required_payload_string(canonical, "module")?;
    let source_set = required_payload_string(canonical, "sourceSet")?;
    let canonical_compilation = required_payload_string(canonical, "compilation")?;
    let derived = format!("{module}/{source_set}");
    if typed_compilation != derived || canonical_compilation != derived {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "typed OpenProject compilation differs from canonical module/sourceSet",
        ));
    }
    Ok(())
}

fn git_revision(repo: &Path) -> Result<String, ClewError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(internal)?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "verified index requires a committed Git revision",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn write_message<M: Message>(writer: &mut impl Write, message: &M) -> Result<(), ClewError> {
    let bytes = message.encode_to_vec();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|e| {
            ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("worker write failed: {e}"),
            )
        })
}

fn write_message_profiled<M: Message>(
    writer: &mut impl Write,
    message: &M,
) -> Result<(u64, u64), ClewError> {
    let encode_started = Instant::now();
    let bytes = message.encode_to_vec();
    let encode_micros = encode_started.elapsed().as_micros() as u64;
    let write_started = Instant::now();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|e| {
            ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("worker write failed: {e}"),
            )
        })?;
    Ok((encode_micros, write_started.elapsed().as_micros() as u64))
}

fn read_message(reader: &mut impl Read) -> Result<WorkerResponse, ClewError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame header failed: {e}"),
        )
    })?;
    let size = u32::from_be_bytes(header) as usize;
    if size > 64 * 1024 * 1024 {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "worker frame exceeds 64MiB",
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame body failed: {e}"),
        )
    })?;
    WorkerResponse::decode(bytes.as_slice())
        .map_err(|e| ClewError::new(ErrorCode::WorkerProtocolMismatch, e.to_string()))
}

fn read_message_profiled(reader: &mut impl Read) -> Result<(WorkerResponse, u64, u64), ClewError> {
    let read_started = Instant::now();
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame header failed: {e}"),
        )
    })?;
    let size = u32::from_be_bytes(header) as usize;
    if size > 64 * 1024 * 1024 {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "worker frame exceeds 64MiB",
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame body failed: {e}"),
        )
    })?;
    let read_micros = read_started.elapsed().as_micros() as u64;
    let decode_started = Instant::now();
    let response = WorkerResponse::decode(bytes.as_slice())
        .map_err(|e| ClewError::new(ErrorCode::WorkerProtocolMismatch, e.to_string()))?;
    Ok((
        response,
        read_micros,
        decode_started.elapsed().as_micros() as u64,
    ))
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn parse_worker_code(code: &str) -> ErrorCode {
    match code {
        "SYMBOL_NOT_FOUND" => ErrorCode::SymbolNotFound,
        "AMBIGUOUS_SYMBOL" => ErrorCode::AmbiguousSymbol,
        "EXPRESSION_NOT_FOUND" => ErrorCode::ExpressionNotFound,
        "STALE_TARGET" => ErrorCode::StaleTarget,
        "AMBIGUOUS_TARGET" => ErrorCode::AmbiguousTarget,
        "REPLACEMENT_PARSE_ERROR" => ErrorCode::ReplacementParseError,
        "UNSUPPORTED_CONTROL_FLOW" => ErrorCode::UnsupportedControlFlow,
        "UNSUPPORTED_PROJECT_CONFIGURATION" => ErrorCode::UnsupportedProjectConfiguration,
        "TYPE_MISMATCH" => ErrorCode::TypeMismatch,
        "BINDING_CHANGED" => ErrorCode::BindingChanged,
        "NEW_DIAGNOSTICS" => ErrorCode::NewDiagnostics,
        "EFFECT_CHANGED" => ErrorCode::EffectChanged,
        _ => ErrorCode::IncompleteSemanticAnalysis,
    }
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::RepositoryIndex;
    use serde_json::json;
    use walkdir::WalkDir;

    #[test]
    fn embedded_worker_distribution_rejects_workspace_and_private_tree_mutation() {
        use std::os::unix::fs::PermissionsExt;

        struct RestoreFile {
            path: PathBuf,
            bytes: Vec<u8>,
            permissions: std::fs::Permissions,
        }
        impl Drop for RestoreFile {
            fn drop(&mut self) {
                let _ = std::fs::write(&self.path, &self.bytes);
                let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
            }
        }
        struct RemoveFile(PathBuf);
        impl Drop for RemoveFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        struct RestoreMove {
            original: PathBuf,
            held: PathBuf,
        }
        impl Drop for RestoreMove {
            fn drop(&mut self) {
                if self.held.exists() {
                    let _ = std::fs::rename(&self.held, &self.original);
                }
            }
        }

        let workspace = workspace_root();
        let mut worker = WorkerClient::start(&workspace).unwrap();
        assert_eq!(worker.capabilities.compiler_version, "2.4.10");
        let repo = workspace
            .join("fixtures/kotlin-basic")
            .canonicalize()
            .unwrap();
        let verified = worker
            .index_files_verified(&json!({
                "repo":repo,
                "compilation":":/main",
                "syntaxOnly":false
            }))
            .unwrap();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap()["k2Validated"],
            true
        );

        let trusted = worker.trusted_distribution.as_ref().unwrap();
        let private_launcher = trusted.launcher.clone();
        let launcher_bytes = std::fs::read(&private_launcher).unwrap();
        let launcher_permissions = std::fs::metadata(&private_launcher).unwrap().permissions();
        std::fs::write(&private_launcher, "#!/bin/sh\nexit 88\n").unwrap();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap_err().code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::write(&private_launcher, launcher_bytes).unwrap();
        std::fs::set_permissions(&private_launcher, launcher_permissions).unwrap();

        let plugin = trusted
            .distribution_root
            .join("lib")
            .join(WorkerVariant::Kotlin24.plugin_jar_name());
        let plugin_bytes = std::fs::read(&plugin).unwrap();
        std::fs::write(&plugin, b"changed").unwrap();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap_err().code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::write(&plugin, plugin_bytes).unwrap();

        let extra = trusted.distribution_root.join("unexpected-authority-input");
        std::fs::write(&extra, b"extra").unwrap();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap_err().code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::remove_file(extra).unwrap();
        assert!(worker.inspect_verified_index(&verified).is_ok());
        worker.shutdown().unwrap();

        let workspace_launcher = worker_launcher(&workspace, WorkerVariant::Kotlin24);
        {
            let original = RestoreFile {
                bytes: std::fs::read(&workspace_launcher).unwrap(),
                permissions: std::fs::metadata(&workspace_launcher)
                    .unwrap()
                    .permissions(),
                path: workspace_launcher.clone(),
            };
            std::fs::write(&workspace_launcher, "#!/bin/sh\nexit 77\n").unwrap();
            std::fs::set_permissions(&workspace_launcher, std::fs::Permissions::from_mode(0o755))
                .unwrap();
            assert_eq!(
                WorkerClient::start(&workspace).err().unwrap().code,
                ErrorCode::WorkerPreparationRequired
            );
            drop(original);
        }

        let workspace_plugin = workspace
            .join(WorkerVariant::Kotlin24.distribution_relative())
            .join("lib")
            .join(WorkerVariant::Kotlin24.plugin_jar_name());
        {
            let original = RestoreFile {
                bytes: std::fs::read(&workspace_plugin).unwrap(),
                permissions: std::fs::metadata(&workspace_plugin).unwrap().permissions(),
                path: workspace_plugin.clone(),
            };
            std::fs::write(&workspace_plugin, b"changed").unwrap();
            assert_eq!(
                WorkerClient::start(&workspace).err().unwrap().code,
                ErrorCode::WorkerPreparationRequired
            );
            drop(original);
        }

        let distribution = workspace.join(WorkerVariant::Kotlin24.distribution_relative());
        let extra = distribution.join("unexpected-authority-input");
        std::fs::write(&extra, b"extra").unwrap();
        let extra_guard = RemoveFile(extra.clone());
        assert_eq!(
            WorkerClient::start(&workspace).err().unwrap().code,
            ErrorCode::WorkerPreparationRequired
        );
        std::fs::remove_file(&extra).unwrap();
        drop(extra_guard);

        let missing = distribution.join("lib/kotlin-0.1.0.jar");
        let held = distribution.join("lib/.kotlin-0.1.0.jar.held");
        std::fs::rename(&missing, &held).unwrap();
        let missing_guard = RestoreMove {
            original: missing.clone(),
            held: held.clone(),
        };
        assert_eq!(
            WorkerClient::start(&workspace).err().unwrap().code,
            ErrorCode::WorkerPreparationRequired
        );
        std::fs::rename(&held, &missing).unwrap();
        drop(missing_guard);

        let caller_init = workspace.join("init.gradle");
        std::fs::write(
            &caller_init,
            "throw new GradleException('caller injection')\n",
        )
        .unwrap();
        let init_guard = RemoveFile(caller_init.clone());
        assert_eq!(
            WorkerClient::start(&workspace).err().unwrap().code,
            ErrorCode::WorkerPreparationRequired
        );
        std::fs::remove_file(caller_init).unwrap();
        drop(init_guard);

        let warm_started = Instant::now();
        let restored = WorkerClient::start(&workspace).unwrap();
        eprintln!(
            "embedded_worker_distribution_warm_start_ms={}",
            warm_started.elapsed().as_millis()
        );
        restored.shutdown().unwrap();
    }

    #[test]
    fn open_project_protocol_preserves_full_compilation_identity_kotlin_24() {
        let workspace = workspace_root();
        let source = workspace.join("fixtures/kotlin-basic");
        let temporary = tempfile::Builder::new()
            .prefix("open-project-protocol-")
            .tempdir_in(workspace.join("fixtures"))
            .unwrap();
        for entry in WalkDir::new(&source).into_iter().map(Result::unwrap) {
            let relative = entry.path().strip_prefix(&source).unwrap();
            if relative.components().any(|part| {
                matches!(
                    part.as_os_str().to_str(),
                    Some(".gradle" | ".semantic-thread" | "build")
                )
            }) {
                continue;
            }
            let destination = temporary.path().join(relative);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&destination).unwrap();
            } else {
                std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
        std::fs::write(
            temporary.path().join("settings.gradle.kts"),
            "pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }\n\
             dependencyResolutionManagement { repositories { mavenCentral() } }\n\
             rootProject.name = \"protocol-fixture\"\ninclude(\":service\")\n",
        )
        .unwrap();
        std::fs::create_dir_all(temporary.path().join("service/src/main/kotlin/p")).unwrap();
        std::fs::write(
            temporary.path().join("service/build.gradle.kts"),
            "plugins { kotlin(\"jvm\") version \"2.4.10\" }\nkotlin { jvmToolchain(21) }\n",
        )
        .unwrap();
        std::fs::write(
            temporary
                .path()
                .join("service/src/main/kotlin/p/Service.kt"),
            "package p\nfun serviceValue() = 1\n",
        )
        .unwrap();

        let mut worker = WorkerClient::start(&workspace).unwrap();
        let root = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":temporary.path(),"compilation":":/main"}),
            )
            .unwrap();
        assert_eq!(root["compilation"], ":/main");
        assert_eq!(root["module"], ":");
        assert_eq!(root["sourceSet"], "main");
        let module = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":temporary.path(),"compilation":":service/main"}),
            )
            .unwrap();
        assert_eq!(module["compilation"], ":service/main");
        assert_eq!(module["module"], ":service");
        assert_eq!(module["sourceSet"], "main");

        let mut malformed = module.clone();
        malformed["compilation"] = Value::String(":/main".into());
        assert_eq!(
            bind_open_project_compilation(&mut malformed, ":service/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        worker.shutdown().unwrap();
    }

    #[test]
    fn verified_index_facts_are_live_session_bound_and_tamper_evident() {
        let workspace = workspace_root();
        let repo = workspace
            .join("fixtures/kotlin-basic")
            .canonicalize()
            .unwrap();
        let mut worker = WorkerClient::start(&workspace).unwrap();
        let mut verified = worker
            .index_files_verified(&json!({
                "repo":repo,
                "compilation":":/main",
                "syntaxOnly":false
            }))
            .unwrap();
        let mut index = RepositoryIndex::open_compilation(&repo, Some(":/main")).unwrap();
        let snapshot = index.update_verified(&verified, &worker).unwrap();
        assert!(!snapshot.is_empty());
        assert!(index.declaration_relations().unwrap().is_some());
        assert!(index.declaration_descriptors().unwrap().is_some());

        let original_session = verified.authority_session;
        verified.authority_session = Uuid::new_v4();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );
        verified.authority_session = original_session;

        verified.payload["declarationDescriptors"]["descriptors"][0]["exportBoundary"] =
            Value::String("PRIVATE_API".into());
        verified.payload["declarationDescriptorHash"] = Value::String(
            crate::canonical::hash(&verified.payload["declarationDescriptors"]).unwrap(),
        );
        verified.payload_hash = crate::canonical::hash(&verified.payload).unwrap();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );
        worker.shutdown().unwrap();
    }
}
