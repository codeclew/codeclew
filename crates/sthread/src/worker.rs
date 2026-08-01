use crate::error::{ErrorCode, SthreadError};
use crate::proto::{
    ApplyEditRequest, BuildLocalGraphRequest, IndexFilesRequest, OpenProjectRequest,
    ProtocolVersion, RequestKind, ResolveExpressionRequest, ResolveSymbolRequest, ShutdownRequest,
    ValidateCandidateRequest, WorkerRequest, WorkerResponse, worker_request, worker_response,
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
            capabilities,
        })
    }

    pub fn request(&mut self, kind: RequestKind, payload: &Value) -> Result<Value, SthreadError> {
        let request_id = self.next_id;
        self.next_id += 1;
        let payload_json = serde_json::to_vec(payload).map_err(internal)?;
        let request_payload = match kind {
            RequestKind::OpenProject => {
                worker_request::Payload::OpenProject(OpenProjectRequest { payload_json })
            }
            RequestKind::IndexFiles => {
                worker_request::Payload::IndexFiles(IndexFilesRequest { payload_json })
            }
            RequestKind::ResolveSymbol => {
                worker_request::Payload::ResolveSymbol(ResolveSymbolRequest { payload_json })
            }
            RequestKind::ResolveExpression => {
                worker_request::Payload::ResolveExpression(ResolveExpressionRequest {
                    payload_json,
                })
            }
            RequestKind::BuildLocalGraph => {
                worker_request::Payload::BuildLocalGraph(BuildLocalGraphRequest { payload_json })
            }
            RequestKind::ApplyEdit => {
                worker_request::Payload::ApplyEdit(ApplyEditRequest { payload_json })
            }
            RequestKind::ValidateCandidate => {
                worker_request::Payload::ValidateCandidate(ValidateCandidateRequest {
                    payload_json,
                })
            }
            RequestKind::Shutdown => worker_request::Payload::Shutdown(ShutdownRequest {}),
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
            snapshot: None,
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
        let payload_json = match response.payload {
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
            Some(worker_response::Payload::OpenProject(value)) => value.payload_json,
            Some(worker_response::Payload::IndexFiles(value)) => value.payload_json,
            Some(worker_response::Payload::ResolveSymbol(value)) => value.payload_json,
            Some(worker_response::Payload::ResolveExpression(value)) => value.payload_json,
            Some(worker_response::Payload::BuildLocalGraph(value)) => value.payload_json,
            Some(worker_response::Payload::ApplyEdit(value)) => value.payload_json,
            Some(worker_response::Payload::ValidateCandidate(value)) => value.payload_json,
            Some(worker_response::Payload::Shutdown(value)) => value.payload_json,
            _ => {
                return Err(SthreadError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker response has unexpected payload",
                ));
            }
        };
        serde_json::from_slice(&payload_json).map_err(internal)
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
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
