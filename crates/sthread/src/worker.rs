use crate::error::{ErrorCode, SthreadError};
use crate::proto::{
    ApplyEditRequest, BatchRequest, BlobRef, BuildLocalGraphRequest, IndexFilesRequest,
    OpenProjectRequest, ProtocolVersion, RequestKind, ResolveExpressionRequest,
    ResolveSymbolRequest, SchemaVersion, ShutdownRequest, SnapshotId, ValidateCandidateRequest,
    WorkerRequest, WorkerResponse, worker_request, worker_response,
};
use prost::Message;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct WorkerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    next_id: u64,
    snapshot: Option<SnapshotId>,
    pub capabilities: crate::proto::WorkerCapabilities,
}

impl WorkerClient {
    pub fn start(workspace: &Path) -> Result<Self, SthreadError> {
        let launcher = worker_launcher(workspace);
        if !launcher.is_file() {
            let output = Command::new(workspace.join("gradlew"))
                .args([":workers:kotlin:installDist", "--no-daemon", "--quiet"])
                .current_dir(workspace)
                .output()
                .map_err(|e| {
                    SthreadError::new(
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
                return Err(SthreadError::new(
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
                SthreadError::new(
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
                return Err(SthreadError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker did not send startup capabilities",
                ));
            }
        };
        if capabilities.compiler_version != "2.4.10"
            || !capabilities.protocol_versions.iter().any(|v| v.major == 1)
        {
            return Err(SthreadError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker compiler/protocol version mismatch",
            ));
        }
        Ok(Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            snapshot: None,
            capabilities,
        })
    }

    pub fn request(&mut self, kind: RequestKind, payload: &Value) -> Result<Value, SthreadError> {
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
                return Err(SthreadError::new(
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
        write_message(&mut self.stdin, &request)?;
        let response = read_message(&mut self.stdout)?;
        if response.request_id != request_id {
            return Err(SthreadError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker response request_id mismatch",
            ));
        }
        let canonical_json = match response.payload {
            Some(worker_response::Payload::Error(error)) => {
                return Err(SthreadError {
                    code: parse_worker_code(&error.code),
                    message: error.message,
                    transaction_id: None,
                    snapshot_id: None,
                    evidence: error.evidence,
                    retryable: error.retryable,
                });
            }
            Some(worker_response::Payload::OpenProject(value)) => {
                validate_typed_string(
                    &value.canonical_json,
                    "/projectModelHash",
                    &value.project_model_hash,
                )?;
                value.canonical_json
            }
            Some(worker_response::Payload::IndexFiles(value)) => {
                validate_typed_string(&value.canonical_json, "/indexHash", &value.index_hash)?;
                validate_typed_count(&value.canonical_json, "/files", value.file_count)?;
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
                    return Err(SthreadError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "typed validation result disagrees with canonical payload",
                    ));
                }
                value.canonical_json
            }
            Some(worker_response::Payload::Shutdown(value)) => value.canonical_json,
            _ => {
                return Err(SthreadError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker response has unexpected payload",
                ));
            }
        };
        let mut value: Value = serde_json::from_slice(&canonical_json).map_err(internal)?;
        if kind == RequestKind::ApplyEdit && value.get("source").is_none() {
            let blob = value.get("sourceBlob").ok_or_else(|| {
                SthreadError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "candidate response has neither inline source nor BlobRef",
                )
            })?;
            let source = read_response_blob(payload, blob)?;
            value["source"] = Value::String(String::from_utf8(source).map_err(|error| {
                SthreadError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
            })?);
        }
        if kind == RequestKind::OpenProject {
            self.snapshot = Some(SnapshotId {
                base_revision: snapshot_from(payload).base_revision,
                project_model_hash: value
                    .get("projectModelHash")
                    .and_then(Value::as_str)
                    .unwrap_or("UNRESOLVED")
                    .into(),
            });
        }
        Ok(value)
    }

    pub fn shutdown(mut self) -> Result<(), SthreadError> {
        let _ = self.request(RequestKind::Shutdown, &serde_json::json!({}))?;
        let status = self
            .child
            .wait()
            .map_err(|e| SthreadError::new(ErrorCode::WorkerCrashed, e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(SthreadError::new(
                ErrorCode::WorkerCrashed,
                format!("worker exited with {status}"),
            ))
        }
    }

    pub fn validate_candidates_batch(
        &mut self,
        candidates: &[(String, String)],
    ) -> Result<Vec<Value>, SthreadError> {
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
            return Err(SthreadError::new(
                ErrorCode::WorkerProtocolMismatch,
                "batch response request_id mismatch",
            ));
        }
        let batch = match response.payload {
            Some(worker_response::Payload::Batch(batch)) => batch,
            _ => {
                return Err(SthreadError::new(
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
                Some(worker_response::Payload::Error(error)) => Err(SthreadError {
                    code: parse_worker_code(&error.code),
                    message: error.message,
                    transaction_id: None,
                    snapshot_id: None,
                    evidence: error.evidence,
                    retryable: error.retryable,
                }),
                _ => Err(SthreadError::new(
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

fn source_transport(payload: &Value) -> Result<(Vec<u8>, Option<BlobRef>), SthreadError> {
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
        .ok_or_else(|| SthreadError::new(ErrorCode::InvalidInput, "large source needs repo"))?;
    let hash = crate::canonical::hash_bytes(&source);
    let relative = format!(
        ".semantic-thread/blobs/sha256/{}",
        hash.trim_start_matches("sha256:")
    );
    let path = Path::new(repo).join(&relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            SthreadError::new(ErrorCode::Internal, format!("blob store: {error}"))
        })?;
    }
    if !path.exists() {
        std::fs::write(&path, &source).map_err(|error| {
            SthreadError::new(ErrorCode::Internal, format!("blob store: {error}"))
        })?;
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

fn validate_typed_string(canonical: &[u8], pointer: &str, typed: &str) -> Result<(), SthreadError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value.pointer(pointer).and_then(Value::as_str) != Some(typed) {
        return Err(SthreadError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response field disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn validate_typed_count(canonical: &[u8], pointer: &str, typed: u64) -> Result<(), SthreadError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| items.len() as u64)
        != Some(typed)
    {
        return Err(SthreadError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response count disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn read_response_blob(payload: &Value, blob: &Value) -> Result<Vec<u8>, SthreadError> {
    let repo = payload.get("repo").and_then(Value::as_str).ok_or_else(|| {
        SthreadError::new(ErrorCode::WorkerProtocolMismatch, "BlobRef has no repo")
    })?;
    let relative = blob
        .get("relativePath")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SthreadError::new(ErrorCode::WorkerProtocolMismatch, "BlobRef has no path")
        })?;
    if relative.starts_with('/') || relative.split('/').any(|part| part == "..") {
        return Err(SthreadError::new(
            ErrorCode::WorkerProtocolMismatch,
            "response BlobRef escapes repository",
        ));
    }
    let bytes = std::fs::read(Path::new(repo).join(relative))
        .map_err(|error| SthreadError::new(ErrorCode::WorkerProtocolMismatch, error.to_string()))?;
    let expected = blob
        .get("contentHash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if crate::canonical::hash_bytes(&bytes) != expected {
        return Err(SthreadError::new(
            ErrorCode::WorkerProtocolMismatch,
            "response BlobRef content hash mismatch",
        ));
    }
    Ok(bytes)
}

fn worker_launcher(workspace: &Path) -> PathBuf {
    workspace.join("workers/kotlin/build/install/kotlin/bin/kotlin")
}

fn write_message<M: Message>(writer: &mut impl Write, message: &M) -> Result<(), SthreadError> {
    let bytes = message.encode_to_vec();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|e| {
            SthreadError::new(
                ErrorCode::WorkerCrashed,
                format!("worker write failed: {e}"),
            )
        })
}

fn read_message(reader: &mut impl Read) -> Result<WorkerResponse, SthreadError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).map_err(|e| {
        SthreadError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame header failed: {e}"),
        )
    })?;
    let size = u32::from_be_bytes(header) as usize;
    if size > 64 * 1024 * 1024 {
        return Err(SthreadError::new(
            ErrorCode::WorkerProtocolMismatch,
            "worker frame exceeds 64MiB",
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(|e| {
        SthreadError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame body failed: {e}"),
        )
    })?;
    WorkerResponse::decode(bytes.as_slice())
        .map_err(|e| SthreadError::new(ErrorCode::WorkerProtocolMismatch, e.to_string()))
}

fn internal(error: impl std::fmt::Display) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, error.to_string())
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
