use crate::error::{ClewError, ErrorCode};
use crate::kotlin_engine::{KotlinEngineRegistry, KotlinProjectSemantics, KotlinSemanticEngine};
use crate::process_isolation::isolate_controller_authority;
use crate::proto::{
    BlobRef, IndexFilesRequest, OpenProjectRequest, ProtocolVersion, RequestKind, SchemaVersion,
    ShutdownRequest, SnapshotId, WorkerRequest, WorkerResponse, worker_request, worker_response,
};
use crate::runtime::RuntimeAuthority;
use crate::state::ManagedDirectory;
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[cfg(test)]
include!(concat!(env!("OUT_DIR"), "/worker_build_inputs.rs"));

#[cfg(test)]
pub(crate) fn workspace_worker_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompilerIndexBackend {
    BtaPersistent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompilerIndexStatus {
    UnchangedHit,
    ColdFull,
    Incremental,
    RecoveredFull,
    Busy,
    FailedRecoverable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilerIndexProfile {
    pub backend: CompilerIndexBackend,
    pub status: CompilerIndexStatus,
    pub valid: bool,
    pub total_micros: u64,
    pub compiler_micros: u64,
    pub fir_extraction_micros: u64,
    pub total_files: u64,
    pub compiled_files: u64,
    pub reused_files: u64,
    pub recovered: bool,
    pub fallback_used: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_digest: Option<String>,
    pub semantic_input_manifest_digest: String,
    pub facts_plugin_digest: String,
    pub extractor_authority_digest: String,
    pub semantic_configuration_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectModelCacheStatus {
    MemoryHit,
    PersistentHit,
    ExtractedPublished,
    ExtractedNotPublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectModelPublishOutcome {
    NotAttempted,
    Published,
    InvalidModel,
    RootUnavailable,
    WriteFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectModelInvalidReason {
    NotApplicable,
    MissingSemanticInputManifestHash,
    InvalidSemanticInputManifestHash,
    SemanticInputManifestHashMismatch,
    MissingSemanticInputManifest,
    ModelInputsManifestMismatch,
    JdkFingerprintManifestMismatch,
    ModelInputsInvalid,
    ResourceIdentitiesInvalid,
    JdkHomeInvalid,
    JdkHomeMismatch,
    JdkFingerprintMissing,
    JdkFingerprintInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectModelCacheProfile {
    pub status: ProjectModelCacheStatus,
    pub publish_outcome: ProjectModelPublishOutcome,
    pub publish_invalid_reason: ProjectModelInvalidReason,
    pub total_micros: u64,
    pub key_micros: u64,
    pub load_micros: u64,
    pub extraction_micros: u64,
    pub publish_micros: u64,
    pub persistent_configured: bool,
    pub published: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RequestProfile {
    pub serialization_micros: u64,
    pub ipc_micros: u64,
    pub worker_processing_micros: u64,
    pub cache_requests: u64,
    pub cache_hits: u64,
    pub psi_parse_micros: u64,
    pub k2_analysis_micros: u64,
    pub fir_extraction_micros: u64,
    pub compiler_index: Option<CompilerIndexProfile>,
    pub project_model_cache: Option<ProjectModelCacheProfile>,
}

pub struct WorkerClient {
    workspace: PathBuf,
    engine: KotlinSemanticEngine,
    qualification_engine: Option<KotlinSemanticEngine>,
    process: OwnedWorkerProcess,
    stdin: ChildStdin,
    stdout: ChildStdout,
    next_id: u64,
    snapshot: Option<SnapshotId>,
    pub capabilities: crate::proto::WorkerCapabilities,
    pub last_profile: RequestProfile,
    authority_session: Uuid,
    trusted_distribution: TrustedWorkerDistribution,
    build_state_root: Option<PathBuf>,
    compiler_index_root: Option<ManagedDirectory>,
    build_namespace_digest: String,
    _transport_root: tempfile::TempDir,
    transport_root: PathBuf,
    issued_index_facts: BTreeMap<Uuid, String>,
    issued_source_syntax: BTreeMap<Uuid, String>,
    request_counters: WorkerRequestCounters,
}

#[derive(Clone)]
pub(crate) struct WorkerCancellationHandle {
    authority: Arc<Mutex<WorkerProcessGroupAuthority>>,
}

static TASK_RUN_PROCESS_TREE_ENABLED: AtomicBool = AtomicBool::new(false);
static TASK_RUN_CANCELLATION_REQUESTED: AtomicBool = AtomicBool::new(false);
static TASK_RUN_WORKER_SPAWN_GATE: Mutex<()> = Mutex::new(());
static TASK_RUN_WORKER_REGISTRY: OnceLock<Mutex<BTreeMap<u64, WorkerCancellationHandle>>> =
    OnceLock::new();
static NEXT_TASK_RUN_WORKER_ID: AtomicU64 = AtomicU64::new(1);

struct TaskRunWorkerSpawnPermit {
    _gate: MutexGuard<'static, ()>,
}

fn task_run_worker_registry() -> &'static Mutex<BTreeMap<u64, WorkerCancellationHandle>> {
    TASK_RUN_WORKER_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn task_run_worker_spawn_permit() -> Result<Option<TaskRunWorkerSpawnPermit>, ClewError> {
    if !TASK_RUN_PROCESS_TREE_ENABLED.load(Ordering::Acquire) {
        return Ok(None);
    }
    let gate = TASK_RUN_WORKER_SPAWN_GATE.lock().map_err(|_| {
        ClewError::new(
            ErrorCode::Internal,
            "task-run worker spawn authority is poisoned",
        )
    })?;
    if TASK_RUN_CANCELLATION_REQUESTED.load(Ordering::Acquire) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "task-run cancellation is already in progress",
        ));
    }
    Ok(Some(TaskRunWorkerSpawnPermit { _gate: gate }))
}

fn register_task_run_worker(
    cancellation: &WorkerCancellationHandle,
) -> Result<Option<u64>, ClewError> {
    if !TASK_RUN_PROCESS_TREE_ENABLED.load(Ordering::Acquire) {
        return Ok(None);
    }
    let id = NEXT_TASK_RUN_WORKER_ID.fetch_add(1, Ordering::Relaxed);
    task_run_worker_registry()
        .lock()
        .map_err(|_| ClewError::new(ErrorCode::Internal, "task-run worker registry is poisoned"))?
        .insert(id, cancellation.clone());
    Ok(Some(id))
}

fn unregister_task_run_worker(registration: &mut Option<u64>) {
    if let Some(id) = registration.take()
        && let Ok(mut workers) = task_run_worker_registry().lock()
    {
        workers.remove(&id);
    }
}

struct WorkerProcessGroupAuthority {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(unix)]
    leader_start_token: String,
    active: bool,
}

impl WorkerCancellationHandle {
    pub(crate) fn cancel(&self) -> Result<(), ClewError> {
        let mut authority = self.authority.lock().map_err(|_| {
            ClewError::new(ErrorCode::Internal, "worker process authority is poisoned")
        })?;
        if !authority.active {
            return Ok(());
        }
        #[cfg(unix)]
        terminate_worker_process_group(authority.pgid, &authority.leader_start_token)?;
        authority.active = false;
        Ok(())
    }
}

struct OwnedWorkerProcess {
    child: Child,
    cancellation: WorkerCancellationHandle,
    task_run_registration: Option<u64>,
}

impl OwnedWorkerProcess {
    fn new(mut child: Child, register_with_task_run: bool) -> Result<Self, ClewError> {
        #[cfg(unix)]
        let pgid = i32::try_from(child.id())
            .ok()
            .filter(|pid| *pid > 1)
            .ok_or_else(|| {
                ClewError::new(ErrorCode::WorkerCrashed, "worker process id is invalid")
            })?;
        #[cfg(unix)]
        let leader_start_token = match worker_process_start_token(child.id()) {
            Ok(Some(start_token)) => start_token,
            result => {
                // `spawn()` already crossed the child-side process_group(0)
                // boundary while the task-run spawn gate is held. Close that
                // fresh authority before returning an admission error.
                let _ = terminate_fresh_worker_process_group(pgid);
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| {
                    ClewError::new(
                        ErrorCode::WorkerCrashed,
                        "worker process identity disappeared during admission",
                    )
                }));
            }
        };
        let cancellation = WorkerCancellationHandle {
            authority: Arc::new(Mutex::new(WorkerProcessGroupAuthority {
                #[cfg(unix)]
                pgid,
                #[cfg(unix)]
                leader_start_token,
                active: true,
            })),
        };
        let task_run_registration = if register_with_task_run {
            match register_task_run_worker(&cancellation) {
                Ok(registration) => registration,
                Err(error) => {
                    let _ = cancellation.cancel();
                    let _ = child.wait();
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(Self {
            child,
            cancellation,
            task_run_registration,
        })
    }

    fn cancellation_handle(&self) -> WorkerCancellationHandle {
        self.cancellation.clone()
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait()?;
        // The worker may have exited while a project-native launcher remained.
        // A successful protocol shutdown therefore still closes the complete
        // process-group authority before the runtime can release its lease.
        let _ = self.cancellation.cancel();
        unregister_task_run_worker(&mut self.task_run_registration);
        Ok(status)
    }
}

impl Drop for OwnedWorkerProcess {
    fn drop(&mut self) {
        let _ = self.cancellation.cancel();
        let _ = self.child.wait();
        unregister_task_run_worker(&mut self.task_run_registration);
    }
}

#[cfg(unix)]
extern "C" fn task_run_termination_handler(_signal: libc::c_int) {
    TASK_RUN_CANCELLATION_REQUESTED.store(true, Ordering::Release);
}

/// Installs process-tree ownership for the detached task-run supervisor.
///
/// The signal handler only closes admission. A dedicated thread then takes the
/// spawn gate, which proves that every worker that crossed `spawn()` is already
/// registered, terminates all registered worker process groups, and exits the
/// supervisor. Direct context commands never install this authority and retain
/// independent per-worker cancellation.
#[cfg(unix)]
pub fn install_task_run_process_tree_supervisor() -> Result<(), ClewError> {
    if TASK_RUN_PROCESS_TREE_ENABLED.swap(true, Ordering::AcqRel) {
        return Err(ClewError::new(
            ErrorCode::Internal,
            "task-run process-tree supervisor is already installed",
        ));
    }
    TASK_RUN_CANCELLATION_REQUESTED.store(false, Ordering::Release);
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    let mut previous_action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = task_run_termination_handler as usize;
    action.sa_flags = 0;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGTERM, &action, &mut previous_action) != 0 {
            TASK_RUN_PROCESS_TREE_ENABLED.store(false, Ordering::Release);
            return Err(ClewError::new(
                ErrorCode::Internal,
                format!(
                    "cannot install task-run termination authority: {}",
                    std::io::Error::last_os_error().kind()
                ),
            ));
        }
    }
    if let Err(error) = std::thread::Builder::new()
        .name("clew-task-run-cancellation".to_owned())
        .spawn(|| {
            while !TASK_RUN_CANCELLATION_REQUESTED.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(10));
            }
            // Holding the gate closes the only interval in which a worker has
            // spawned into its own process group but is not yet registered.
            let _gate = TASK_RUN_WORKER_SPAWN_GATE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let workers = task_run_worker_registry()
                .lock()
                .map(|registry| registry.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let _ = terminate_worker_process_groups(&workers);
            unsafe { libc::_exit(128 + libc::SIGTERM) }
        })
    {
        unsafe {
            libc::sigaction(libc::SIGTERM, &previous_action, std::ptr::null_mut());
        }
        TASK_RUN_PROCESS_TREE_ENABLED.store(false, Ordering::Release);
        return Err(ClewError::new(
            ErrorCode::Internal,
            format!(
                "cannot start task-run cancellation authority: {}",
                error.kind()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn install_task_run_process_tree_supervisor() -> Result<(), ClewError> {
    Err(ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        "task-run process-tree supervision requires Unix",
    ))
}

#[cfg(unix)]
fn terminate_worker_process_group(pgid: i32, expected_start_token: &str) -> Result<(), ClewError> {
    require_worker_group_identity(pgid, expected_start_token)?;
    signal_worker_process_group(pgid, libc::SIGTERM, false)?;
    for _ in 0..40 {
        if !worker_process_group_exists(pgid)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // If the original group disappeared during the grace period and its id was
    // reused by another user, EPERM is a safe terminal observation: never
    // broaden cancellation beyond the process group we created.
    require_worker_group_identity(pgid, expected_start_token)?;
    signal_worker_process_group(pgid, libc::SIGKILL, true)
}

#[cfg(unix)]
fn terminate_fresh_worker_process_group(pgid: i32) -> Result<(), ClewError> {
    signal_worker_process_group(pgid, libc::SIGTERM, false)?;
    for _ in 0..40 {
        if !worker_process_group_exists(pgid)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    signal_worker_process_group(pgid, libc::SIGKILL, true)
}

#[cfg(unix)]
fn terminate_worker_process_groups(workers: &[WorkerCancellationHandle]) -> Result<(), ClewError> {
    let mut authorities = Vec::new();
    for worker in workers {
        let mut authority = worker.authority.lock().map_err(|_| {
            ClewError::new(ErrorCode::Internal, "worker process authority is poisoned")
        })?;
        if authority.active {
            authority.active = false;
            authorities.push((authority.pgid, authority.leader_start_token.clone()));
        }
    }
    authorities.sort_unstable();
    authorities.dedup();

    let mut first_error = None;
    for (pgid, start_token) in &authorities {
        if let Err(error) = require_worker_group_identity(*pgid, start_token)
            .and_then(|()| signal_worker_process_group(*pgid, libc::SIGTERM, false))
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    for _ in 0..40 {
        let mut any_active = false;
        for (pgid, _) in &authorities {
            match worker_process_group_exists(*pgid) {
                Ok(true) => any_active = true,
                Ok(false) => {}
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        if !any_active {
            return first_error.map_or(Ok(()), Err);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for (pgid, start_token) in authorities {
        if let Err(error) = require_worker_group_identity(pgid, &start_token)
            .and_then(|()| signal_worker_process_group(pgid, libc::SIGKILL, true))
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(unix)]
fn require_worker_group_identity(pgid: i32, expected_start_token: &str) -> Result<(), ClewError> {
    match worker_process_start_token(pgid as u32)? {
        Some(start_token) if start_token == expected_start_token => Ok(()),
        None => Ok(()),
        Some(_) if !worker_process_group_exists(pgid)? => Ok(()),
        Some(_) => Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "worker process-group identity changed before cancellation",
        )),
    }
}

#[cfg(target_os = "macos")]
fn worker_process_start_token(pid: u32) -> Result<Option<String>, ClewError> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let result = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size as i32,
        )
    };
    if result != size as i32 || info.pbi_pid != pid {
        return Ok(None);
    }
    Ok(Some(format!(
        "{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    )))
}

#[cfg(target_os = "linux")]
fn worker_process_start_token(pid: u32) -> Result<Option<String>, ClewError> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(internal(error)),
    };
    let fields = stat
        .get(
            stat.rfind(')')
                .ok_or_else(|| internal("invalid process stat"))?
                + 1..,
        )
        .ok_or_else(|| internal("invalid process stat"))?
        .split_whitespace()
        .collect::<Vec<_>>();
    Ok(fields.get(19).map(|start| (*start).to_owned()))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn worker_process_start_token(pid: u32) -> Result<Option<String>, ClewError> {
    let output = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(internal)?;
    if !output.status.success() {
        return Ok(None);
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|_| internal("worker process identity is not UTF-8"))?
        .trim()
        .to_owned();
    Ok((!token.is_empty()).then_some(token))
}

#[cfg(unix)]
fn signal_worker_process_group(
    pgid: i32,
    signal: i32,
    permission_denied_is_terminal: bool,
) -> Result<(), ClewError> {
    let result = unsafe { libc::kill(-pgid, signal) };
    let error = std::io::Error::last_os_error();
    if result == 0
        || error.raw_os_error() == Some(libc::ESRCH)
        || (permission_denied_is_terminal && error.raw_os_error() == Some(libc::EPERM))
    {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("cannot terminate worker process group: {}", error.kind()),
        ))
    }
}

#[cfg(unix)]
fn worker_process_group_exists(pgid: i32) -> Result<bool, ClewError> {
    let result = unsafe { libc::kill(-pgid, 0) };
    if result == 0 {
        Ok(true)
    } else {
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            Some(libc::EPERM) => Ok(true),
            _ => Err(ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("cannot inspect worker process group: {}", error.kind()),
            )),
        }
    }
}

const MAX_WORKER_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_WORKER_RESPONSE_BLOB_BYTES: u64 = 256 * 1024 * 1024;
const SEALED_WORKER_JVM_OPTIONS: &str = "-Xms64m -Xmx1024m -XX:MaxMetaspaceSize=384m -XX:MaxDirectMemorySize=256m -XX:+ExitOnOutOfMemoryError";

fn configure_sealed_worker_process(command: &mut Command, transport_root: &Path) {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // The Kotlin worker wraps native build tools whose diagnostics are
        // neither bounded nor an evidence authority. Never forward them to
        // the controller terminal. The typed protocol result is the only
        // supported error boundary.
        .stderr(Stdio::null())
        // Generated Gradle application launchers apply KOTLIN_OPTS only to
        // this JVM. Preserve the project's ambient JAVA_* variables for
        // PROJECT_NATIVE children while bounding worker/model memory.
        .env("KOTLIN_OPTS", SEALED_WORKER_JVM_OPTIONS)
        .env("CODECLEW_WORKER_TRANSPORT_ROOT", transport_root);
}

/// Opaque `COMPILER_SEMANTIC` authority capability for one exact, live
/// `IndexFiles` response.
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
    local_cfg_hash: String,
    payload: Value,
}

/// Opaque `SOURCE_SYNTAX` capability for current source declarations.
///
/// This type is intentionally disjoint from [`VerifiedIndexFacts`]. It cannot
/// authorize compiler facts, relation graphs, descriptor graphs, edits, or
/// transactions. The issuing live worker session rechecks the exact files on
/// every inspection, so a source change invalidates the capability even when
/// Git HEAD does not change.
pub struct VerifiedSourceSyntax {
    receipt_id: Uuid,
    authority_session: Uuid,
    repo: PathBuf,
    compilation: String,
    distribution_tree_hash: String,
    build_input_digest: String,
    requested_files: Vec<String>,
    source_manifest_hash: String,
    payload_hash: String,
    payload: Value,
}

pub const COMPILER_SEMANTIC_AUTHORITY: &str = "COMPILER_SEMANTIC";
pub const SOURCE_SYNTAX_AUTHORITY: &str = "SOURCE_SYNTAX";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerRequestCounters {
    pub open_project_requests: u64,
    pub index_files_requests: u64,
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
    let worker_diagnostics = facts
        .and_then(|value| value.get("diagnostics"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .take(16)
                .map(|row| {
                    let message = row
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let mut bounded = message.chars().take(1_024).collect::<String>();
                    if message.chars().count() > 1_024 {
                        bounded.push('…');
                    }
                    serde_json::json!({
                        "severity":row.get("severity").and_then(Value::as_str).unwrap_or("INFO"),
                        "message":bounded,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
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
        "workerDiagnosticCount":facts.and_then(|value| value.get("diagnostics")).and_then(Value::as_array).map_or(0, Vec::len),
        "workerDiagnostics":worker_diagnostics,
        "descriptorFailure":if stage == "DESCRIPTOR_GRAPH" {
            facts
                .map(crate::semantic_validation::descriptor_validation_diagnostic)
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        },
    });
    if let Ok(encoded) = serde_json::to_string(&diagnostic) {
        error.evidence.push(encoded);
    }
    error
}

fn require_k2_validated(facts: &Value) -> Result<(), ClewError> {
    if facts.get("k2Validated").and_then(Value::as_bool) != Some(true) {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "verified compiler facts require a successful K2 analysis",
        ));
    }
    Ok(())
}

fn source_syntax_protocol_error(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::WorkerProtocolMismatch, message)
}

fn require_absent_or_empty_rows(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<(), ClewError> {
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    if value.as_array().is_some_and(Vec::is_empty) {
        return Ok(());
    }
    Err(source_syntax_protocol_error(format!(
        "SOURCE_SYNTAX response contains {context}"
    )))
}

fn require_empty_semantic_graph(
    facts: &Value,
    graph_key: &str,
    row_keys: &[&str],
    hash_key: &str,
) -> Result<(), ClewError> {
    let Some(graph) = facts.get(graph_key) else {
        if facts.get(hash_key).is_some_and(|value| !value.is_null()) {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX response has {hash_key} without {graph_key}"
            )));
        }
        return Ok(());
    };
    if let Some(rows) = graph.as_array() {
        if !rows.is_empty() {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX {graph_key} contains semantic rows"
            )));
        }
        if let Some(expected) = facts.get(hash_key) {
            let expected = expected.as_str().ok_or_else(|| {
                source_syntax_protocol_error(format!("SOURCE_SYNTAX {hash_key} is not a string"))
            })?;
            if crate::canonical::hash(graph).map_err(internal)? != expected {
                return Err(source_syntax_protocol_error(format!(
                    "SOURCE_SYNTAX {graph_key} hash differs"
                )));
            }
        }
        return Ok(());
    }
    let object = graph.as_object().ok_or_else(|| {
        source_syntax_protocol_error(format!(
            "SOURCE_SYNTAX {graph_key} must be absent or an empty graph"
        ))
    })?;
    for key in row_keys {
        require_absent_or_empty_rows(object, key, &format!("{graph_key}.{key} rows"))?;
    }
    if let Some(provenance) = object.get("provenance") {
        let empty = provenance.is_null()
            || provenance
                .as_object()
                .is_some_and(serde_json::Map::is_empty);
        if !empty {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX {graph_key} carries semantic provenance"
            )));
        }
    }
    if let Some(expected) = facts.get(hash_key) {
        let expected = expected.as_str().ok_or_else(|| {
            source_syntax_protocol_error(format!("SOURCE_SYNTAX {hash_key} is not a string"))
        })?;
        if crate::canonical::hash(graph).map_err(internal)? != expected {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX {graph_key} hash differs"
            )));
        }
    }
    Ok(())
}

fn source_syntax_relative_path(repo: &Path, raw: &str) -> Result<(PathBuf, Vec<u8>), ClewError> {
    let relative = Path::new(raw);
    if raw.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX file path is not a normalized repository-relative path",
        ));
    }
    let joined = repo.join(relative);
    let metadata = std::fs::symlink_metadata(&joined).map_err(|error| {
        ClewError::new(
            ErrorCode::ProjectModelChanged,
            format!("SOURCE_SYNTAX file is no longer current: {error}"),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "SOURCE_SYNTAX file is no longer a regular source file",
        ));
    }
    let canonical = joined.canonicalize().map_err(internal)?;
    if canonical != joined || !canonical.starts_with(repo) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX file resolves through a symlink or outside the repository",
        ));
    }
    let bytes = std::fs::read(&canonical).map_err(internal)?;
    Ok((canonical, bytes))
}

/// Validate the deliberately narrow claim made by a `SOURCE_SYNTAX` response
/// and return a stable manifest hash for the exact current declarations.
fn validate_source_syntax_response(
    repo: &Path,
    compilation: &str,
    requested_files: &[String],
    facts: &Value,
) -> Result<String, ClewError> {
    let repo = repo.canonicalize().map_err(internal)?;
    if facts.get("schema").and_then(Value::as_str) != Some("semantic-index/0.1")
        || facts.get("compilation").and_then(Value::as_str) != Some(compilation)
    {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response schema or compilation differs from the request",
        ));
    }
    if facts.get("analysisMode").and_then(Value::as_str) != Some("SYNTAX_DECLARATIONS") {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response requires analysisMode SYNTAX_DECLARATIONS",
        ));
    }
    if facts.get("k2Validated").and_then(Value::as_bool) != Some(false) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response must explicitly report k2Validated false",
        ));
    }
    if facts.get("partial").and_then(Value::as_bool) != Some(!requested_files.is_empty()) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response partial flag differs from the exact request selector",
        ));
    }
    require_empty_semantic_graph(
        facts,
        "declarationRelations",
        &["relations", "boundaries"],
        "declarationRelationHash",
    )?;
    require_empty_semantic_graph(
        facts,
        "declarationDescriptors",
        &["descriptors", "boundaries"],
        "declarationDescriptorHash",
    )?;
    let top = facts.as_object().ok_or_else(|| {
        source_syntax_protocol_error("SOURCE_SYNTAX response is not a JSON object")
    })?;
    require_absent_or_empty_rows(top, "semanticFacts", "top-level semantic facts")?;
    require_absent_or_empty_rows(top, "localCfgs", "compiler local CFG rows")?;
    require_absent_or_empty_rows(top, "localCfgBoundaries", "compiler local CFG boundaries")?;
    if top
        .get("localCfgHash")
        .is_some_and(|value| !value.is_null())
    {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response contains a compiler local CFG hash",
        ));
    }

    let files = facts
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            source_syntax_protocol_error("SOURCE_SYNTAX response has no file payload")
        })?;
    let mut manifest = BTreeMap::<String, Value>::new();
    for file in files {
        let object = file.as_object().ok_or_else(|| {
            source_syntax_protocol_error("SOURCE_SYNTAX file payload is not an object")
        })?;
        for (key, context) in [
            ("semanticFacts", "per-file semantic facts"),
            ("inheritance", "per-file inheritance facts"),
            ("overrides", "per-file override facts"),
            ("functionSummaries", "per-file function summaries"),
        ] {
            require_absent_or_empty_rows(object, key, context)?;
        }
        let relative = object.get("path").and_then(Value::as_str).ok_or_else(|| {
            source_syntax_protocol_error("SOURCE_SYNTAX file payload has no path")
        })?;
        if object.get("normalizedRelativePath").and_then(Value::as_str) != Some(relative) {
            return Err(source_syntax_protocol_error(
                "SOURCE_SYNTAX file path is not its normalizedRelativePath",
            ));
        }
        let (_, bytes) = source_syntax_relative_path(&repo, relative)?;
        let content_hash = crate::canonical::hash_bytes(&bytes);
        if object.get("contentHash").and_then(Value::as_str) != Some(content_hash.as_str()) {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!("SOURCE_SYNTAX content hash is stale for {relative}"),
            ));
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX Kotlin source is not UTF-8: {relative}"
            ))
        })?;
        let source_utf16_len = source.encode_utf16().count() as u64;
        let declarations = object
            .get("declarations")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                source_syntax_protocol_error(format!(
                    "SOURCE_SYNTAX file has no declaration payload: {relative}"
                ))
            })?;
        // SOURCE_SYNTAX identities are deliberately non-semantic: overloads
        // and declarations from an unknown compilation may share the worker's
        // provisional declarationId/symbolId.  The authority is the exact
        // current source occurrence, which must still be unique.
        let mut declaration_origins = BTreeSet::<(u64, u64)>::new();
        let mut previous_range: Option<(u64, u64)> = None;
        for declaration in declarations {
            let declaration = declaration.as_object().ok_or_else(|| {
                source_syntax_protocol_error("SOURCE_SYNTAX declaration is not an object")
            })?;
            if declaration.contains_key("compilerSymbol")
                || declaration.contains_key("compilerAuthority")
            {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration contains compiler-semantic authority",
                ));
            }
            for key in ["declarationId", "symbolId", "kind"] {
                if declaration
                    .get(key)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(source_syntax_protocol_error(format!(
                        "SOURCE_SYNTAX declaration has no {key}"
                    )));
                }
            }
            if declaration.get("name").and_then(Value::as_str).is_none() {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration has no string name",
                ));
            }
            if declaration.get("file").and_then(Value::as_str) != Some(relative) {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration file differs from its containing file",
                ));
            }
            let start = declaration.get("rangeStart").and_then(Value::as_u64);
            let end = declaration.get("rangeEnd").and_then(Value::as_u64);
            let (Some(start), Some(end)) = (start, end) else {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration has no source range",
                ));
            };
            if start > end || end > source_utf16_len {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration range is outside current source",
                ));
            }
            if previous_range.is_some_and(|previous| previous > (start, end)) {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declarations are not in source order",
                ));
            }
            previous_range = Some((start, end));
            let origin = declaration
                .get("sourceOrigin")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    source_syntax_protocol_error("SOURCE_SYNTAX declaration has no source origin")
                })?;
            if origin.get("file").and_then(Value::as_str) != Some(relative)
                || origin.get("rangeStart").and_then(Value::as_u64) != Some(start)
                || origin.get("rangeEnd").and_then(Value::as_u64) != Some(end)
            {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration source origin differs from its range",
                ));
            }
            if !declaration_origins.insert((start, end)) {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration source occurrences are not unique within a file",
                ));
            }
        }
        let declaration_hash =
            crate::canonical::hash(&Value::Array(declarations.clone())).map_err(internal)?;
        if manifest
            .insert(
                relative.to_owned(),
                serde_json::json!({
                    "contentHash":content_hash,
                    "declarationHash":declaration_hash,
                }),
            )
            .is_some()
        {
            return Err(source_syntax_protocol_error(
                "SOURCE_SYNTAX response contains a duplicate file path",
            ));
        }
    }

    let mut requested = BTreeMap::<String, ()>::new();
    for relative in requested_files {
        let _ = source_syntax_relative_path(&repo, relative)?;
        if requested.insert(relative.clone(), ()).is_some() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX request contains duplicate files",
            ));
        }
    }
    if requested.keys().ne(manifest.keys()) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response does not contain exactly the requested files",
        ));
    }

    crate::canonical::hash(&serde_json::json!({
        "authority":SOURCE_SYNTAX_AUTHORITY,
        "compilation":compilation,
        "files":manifest,
    }))
    .map_err(internal)
}

/// Preserve compiler-proven base relations while quarantining optional
/// relation enrichments whose coordinate/CFG contract is not yet reliable.
///
/// The worker's original graph hash is verified before any transformation.
/// Nothing is upgraded to PROVEN here: uncertain enrichments are removed and
/// represented by typed UNKNOWN boundaries, then the ordinary strict graph
/// validator authorizes the remaining facts.
fn normalize_optional_relation_evidence(facts: &mut Value) -> Result<(), ClewError> {
    fn invalid(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::InvalidInput, message)
    }

    let expected_hash = facts
        .get("declarationRelationHash")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("worker index has no declarationRelationHash"))?
        .to_owned();
    let graph = facts
        .get_mut("declarationRelations")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("worker index has no declaration relation graph"))?;
    if crate::canonical::hash(graph).map_err(internal)? != expected_hash {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration relation hash differs before optional evidence normalization",
        ));
    }

    for label in ["relations", "boundaries"] {
        let rows = graph
            .get(label)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(format!("declaration relation graph has no {label}")))?;
        let encoded = rows
            .iter()
            .map(crate::canonical::bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid(format!(
                "declaration relation {label} must be canonical before normalization"
            )));
        }
    }

    let original_relations = graph["relations"]
        .as_array_mut()
        .ok_or_else(|| invalid("declaration relation graph has no relations"))?;
    let mut retained = Vec::with_capacity(original_relations.len());
    let mut normalized_rows = BTreeMap::<String, Vec<String>>::new();
    for mut relation in std::mem::take(original_relations) {
        let kind = relation
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let raw_row_hash = crate::canonical::hash(&relation).map_err(internal)?;
        let boundary_code = match kind.as_str() {
            "NULL_COALESCES" => Some("NULL_COALESCING_FLOW_UNAVAILABLE"),
            "RETURNS_VALUE_FROM" => Some("RETURN_VALUE_FLOW_UNAVAILABLE"),
            _ => None,
        };
        if let Some(code) = boundary_code {
            normalized_rows
                .entry(code.to_owned())
                .or_default()
                .push(raw_row_hash);
            continue;
        }

        let argument_mapping = relation.get("argumentToParameter");
        let has_argument_mapping_evidence = argument_mapping
            .is_some_and(|value| value.as_array().is_none_or(|rows| !rows.is_empty()));
        let has_exact_target_contract = matches!(kind.as_str(), "CALLS" | "CONSTRUCTS")
            && [
                "targetCompilerCallableId",
                "targetJvmDescriptor",
                "receiverSelection",
                "omittedDefaultParameterIndices",
            ]
            .into_iter()
            .all(|field| relation.get(field).is_some());
        if matches!(kind.as_str(), "CALLS" | "CONSTRUCTS")
            && argument_mapping.is_some()
            && !has_exact_target_contract
        {
            relation
                .as_object_mut()
                .expect("relation was already an object")
                .remove("argumentToParameter");
        }
        if matches!(kind.as_str(), "CALLS" | "CONSTRUCTS")
            && has_argument_mapping_evidence
            && !has_exact_target_contract
        {
            normalized_rows
                .entry("ARGUMENT_MAPPING_UNAVAILABLE".to_owned())
                .or_default()
                .push(raw_row_hash);
        }
        retained.push(relation);
    }

    let mut unique_relations = BTreeMap::new();
    for relation in retained {
        unique_relations.insert(
            crate::canonical::bytes(&relation).map_err(internal)?,
            relation,
        );
    }
    *original_relations = unique_relations.into_values().collect();

    if !normalized_rows.is_empty() {
        let boundaries = graph["boundaries"]
            .as_array_mut()
            .ok_or_else(|| invalid("declaration relation graph has no boundaries"))?;
        for (code, mut row_hashes) in normalized_rows {
            row_hashes.sort();
            row_hashes.dedup();
            boundaries.push(serde_json::json!({
                "schema":"declaration-relation-boundary/0.1",
                "stage":"OPTIONAL_RELATION_EVIDENCE",
                "code":code,
                "resolution":"UNKNOWN",
                "provider":"CODECLEW_RELATION_NORMALIZER",
                "affectedRowCount":row_hashes.len(),
                "rawRowsHash":crate::canonical::hash(&serde_json::json!(row_hashes)).map_err(internal)?,
            }));
        }
        let mut unique_boundaries = BTreeMap::new();
        for boundary in std::mem::take(boundaries) {
            unique_boundaries.insert(
                crate::canonical::bytes(&boundary).map_err(internal)?,
                boundary,
            );
        }
        *boundaries = unique_boundaries.into_values().collect();
        graph["coverage"] = Value::String("PARTIAL".to_owned());
    }

    let normalized_hash = crate::canonical::hash(graph).map_err(internal)?;
    facts["declarationRelationHash"] = Value::String(normalized_hash);
    Ok(())
}

/// Quarantine exact descriptor rows whose only invalid claim is a non-JVMS
/// compiler suffix. The raw worker graph is hash-verified before mutation and
/// the row becomes typed UNKNOWN evidence; it is never admitted as PROVEN.
struct DescriptorSourceLineIndex {
    source: String,
    line_starts: Vec<usize>,
}

fn verify_descriptor_line_coordinates(repo: &Path, facts: &Value) -> Result<(), ClewError> {
    let protocol = |message: &str| ClewError::new(ErrorCode::WorkerProtocolMismatch, message);
    let files = facts
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol("Kotlin descriptor line proof has no source files"))?;
    let mut content_hashes = BTreeMap::new();
    for file in files {
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| protocol("Kotlin descriptor line proof has no source path"))?;
        let content_hash = file
            .get("contentHash")
            .and_then(Value::as_str)
            .filter(|hash| hash.starts_with("sha256:"))
            .ok_or_else(|| protocol("Kotlin descriptor line proof has no content hash"))?;
        if content_hashes
            .insert(path.to_owned(), content_hash.to_owned())
            .is_some()
        {
            return Err(protocol(
                "Kotlin descriptor line proof repeats a source path",
            ));
        }
    }
    let descriptors = facts
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol("Kotlin descriptor line proof has no descriptors"))?;
    let canonical_repo = repo.canonicalize().map_err(internal)?;
    let mut indexes = BTreeMap::<String, DescriptorSourceLineIndex>::new();
    for descriptor in descriptors {
        let has_lines = ["startLine", "endLine", "lineProvenance"]
            .iter()
            .any(|field| descriptor.get(*field).is_some());
        if !has_lines {
            continue;
        }
        let path = descriptor
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol("Kotlin descriptor line proof has no file"))?;
        let expected_hash = content_hashes
            .get(path)
            .ok_or_else(|| protocol("Kotlin descriptor line proof file is not bound"))?;
        if !indexes.contains_key(path) {
            let (_, bytes) = source_syntax_relative_path(&canonical_repo, path)?;
            if crate::canonical::hash_bytes(&bytes) != *expected_hash {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "Kotlin descriptor line proof source content changed",
                ));
            }
            let source = String::from_utf8(bytes)
                .map_err(|_| protocol("Kotlin descriptor line proof source is not valid UTF-8"))?;
            let mut line_starts = vec![0];
            line_starts.extend(
                source
                    .as_bytes()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, byte)| (*byte == b'\n').then_some(index + 1)),
            );
            indexes.insert(
                path.to_owned(),
                DescriptorSourceLineIndex {
                    source,
                    line_starts,
                },
            );
        }
        let index = &indexes[path];
        let start = descriptor
            .get("start")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| protocol("Kotlin descriptor line proof has no byte start"))?;
        let end = descriptor
            .get("end")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| protocol("Kotlin descriptor line proof has no byte end"))?;
        if start >= end
            || end > index.source.len()
            || !index.source.is_char_boundary(start)
            || !index.source.is_char_boundary(end)
        {
            return Err(protocol(
                "Kotlin descriptor line proof has an invalid UTF-8 byte range",
            ));
        }
        let line_at = |offset: usize| {
            u64::try_from(index.line_starts.partition_point(|line| *line <= offset))
                .expect("source line count fits u64")
        };
        let expected_start = line_at(start);
        let expected_end = line_at(end - 1);
        if descriptor.get("startLine").and_then(Value::as_u64) != Some(expected_start)
            || descriptor.get("endLine").and_then(Value::as_u64) != Some(expected_end)
            || descriptor.get("lineProvenance").and_then(Value::as_str)
                != Some("UTF8_BYTE_RANGE_OVER_COMPILATION_SOURCE")
        {
            return Err(protocol(
                "Kotlin descriptor line proof differs from bound source bytes",
            ));
        }
    }
    Ok(())
}

fn normalize_invalid_jvm_descriptors(facts: &mut Value) -> Result<usize, ClewError> {
    fn invalid(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::InvalidInput, message)
    }

    let expected_hash = facts
        .get("declarationDescriptorHash")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("worker index has no declarationDescriptorHash"))?
        .to_owned();
    let graph = facts
        .get_mut("declarationDescriptors")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("worker index has no declaration descriptor graph"))?;
    if crate::canonical::hash(graph).map_err(internal)? != expected_hash {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration descriptor hash differs before JVM descriptor normalization",
        ));
    }
    for label in ["descriptors", "boundaries"] {
        let rows = graph
            .get(label)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(format!("declaration descriptor graph has no {label}")))?;
        let encoded = rows
            .iter()
            .map(crate::canonical::bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid(format!(
                "declaration descriptor {label} must be canonical before normalization"
            )));
        }
    }
    if graph["boundaries"].as_array().is_some_and(|boundaries| {
        boundaries.iter().any(|boundary| {
            boundary.get("provider").and_then(Value::as_str)
                == Some("CODECLEW_DESCRIPTOR_NORMALIZER")
                || boundary.get("code").and_then(Value::as_str) == Some("INVALID_JVM_DESCRIPTOR")
        })
    }) {
        return Err(invalid(
            "raw worker graph impersonates Codeclew descriptor normalization",
        ));
    }

    let mut added_boundaries = Vec::new();
    let mut quarantined_count = 0_usize;
    {
        let descriptors = graph["descriptors"]
            .as_array_mut()
            .ok_or_else(|| invalid("declaration descriptor graph has no descriptors"))?;
        let mut retained = Vec::with_capacity(descriptors.len());
        for descriptor in std::mem::take(descriptors) {
            if !crate::semantic_validation::has_quarantinable_exact_jvm_descriptor(&descriptor) {
                retained.push(descriptor);
                continue;
            }
            let raw_row_hash = crate::canonical::hash(&descriptor).map_err(internal)?;
            let symbol_identity = descriptor["symbolIdentity"]
                .as_str()
                .expect("quarantinable descriptor has a validated symbolIdentity")
                .to_owned();
            quarantined_count += 1;
            added_boundaries.push(serde_json::json!({
                "schema":"declaration-descriptor-boundary/0.1",
                "file":descriptor["file"],
                "start":descriptor["start"],
                "end":descriptor["end"],
                "symbolIdentity":symbol_identity,
                "stage":"NORMALIZE",
                "code":"INVALID_JVM_DESCRIPTOR",
                "resolution":"UNKNOWN",
                "provider":"CODECLEW_DESCRIPTOR_NORMALIZER",
                "module":descriptor["module"],
                "sourceSet":descriptor["sourceSet"],
                "sourceProvenance":descriptor["sourceProvenance"],
                "compilerAuthority":descriptor["compilerAuthority"],
                "rawRowHash":raw_row_hash,
            }));
        }
        *descriptors = retained;
    }

    if !added_boundaries.is_empty() {
        let boundaries = graph["boundaries"]
            .as_array_mut()
            .ok_or_else(|| invalid("declaration descriptor graph has no boundaries"))?;
        boundaries.extend(added_boundaries);
        let mut unique_boundaries = BTreeMap::new();
        for boundary in std::mem::take(boundaries) {
            unique_boundaries.insert(
                crate::canonical::bytes(&boundary).map_err(internal)?,
                boundary,
            );
        }
        *boundaries = unique_boundaries.into_values().collect();
        graph["coverage"] = Value::String("PARTIAL".to_owned());
    }

    facts["declarationDescriptorHash"] =
        Value::String(crate::canonical::hash(graph).map_err(internal)?);
    Ok(quarantined_count)
}

fn canonical_local_cfg_rows(rows: &[Value], label: &str) -> Result<(), ClewError> {
    let encoded = rows
        .iter()
        .map(crate::canonical::bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("local CFG {label} must be canonical, sorted, and unique"),
        ));
    }
    Ok(())
}

fn local_cfg_snapshot_hash(graphs: &[Value], boundaries: &[Value]) -> Result<String, ClewError> {
    crate::canonical::hash(&serde_json::json!({
        "graphs":graphs,
        "boundaries":boundaries,
    }))
    .map_err(internal)
}

fn proven_descriptor_identities(descriptor_graph: &Value) -> BTreeSet<String> {
    descriptor_graph
        .get("descriptors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("symbolIdentity").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn validate_local_cfg_snapshot(
    facts: &Value,
    descriptor_graph: &Value,
) -> Result<String, ClewError> {
    let graphs = facts
        .get("localCfgs")
        .and_then(Value::as_array)
        .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "worker index has no localCfgs"))?;
    let boundaries = facts
        .get("localCfgBoundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "worker index has no localCfgBoundaries",
            )
        })?;
    canonical_local_cfg_rows(graphs, "graphs")?;
    canonical_local_cfg_rows(boundaries, "boundaries")?;
    let proven = proven_descriptor_identities(descriptor_graph);
    let mut owners = BTreeSet::new();
    for graph in graphs {
        let payload: crate::thread_flow_cfg::LocalCfgPayload =
            serde_json::from_value(graph.clone()).map_err(|_| {
                ClewError::new(ErrorCode::InvalidInput, "local CFG payload is malformed")
            })?;
        crate::thread_flow_cfg::validate(&payload)?;
        if !proven.contains(&payload.owner_symbol_identity)
            || !owners.insert(payload.owner_symbol_identity)
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "local CFG owner is not a unique proven declaration descriptor",
            ));
        }
    }
    for boundary in boundaries {
        crate::thread_flow_cfg::validate_boundary(boundary)?;
    }
    let hash = facts
        .get("localCfgHash")
        .and_then(Value::as_str)
        .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "worker index has no localCfgHash"))?
        .to_owned();
    if local_cfg_snapshot_hash(graphs, boundaries)? != hash {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "local CFG hash differs from canonical Rust snapshot hash",
        ));
    }
    Ok(hash)
}

/// Verify the worker's original local-CFG snapshot before mutation, retain only
/// graphs linked to a proven declaration descriptor, and quarantine every
/// unsupported graph as typed UNKNOWN evidence. Numeric node IDs are never
/// inspected here to invent control-flow order.
fn normalize_local_cfg_evidence(
    facts: &mut Value,
    descriptor_graph: &Value,
) -> Result<String, ClewError> {
    let expected_hash = facts
        .get("localCfgHash")
        .and_then(Value::as_str)
        .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "worker index has no localCfgHash"))?
        .to_owned();
    let raw_graphs = facts
        .get("localCfgs")
        .and_then(Value::as_array)
        .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "worker index has no localCfgs"))?
        .clone();
    let raw_boundaries = facts
        .get("localCfgBoundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "worker index has no localCfgBoundaries",
            )
        })?
        .clone();
    canonical_local_cfg_rows(&raw_graphs, "graphs")?;
    canonical_local_cfg_rows(&raw_boundaries, "boundaries")?;
    if local_cfg_snapshot_hash(&raw_graphs, &raw_boundaries)? != expected_hash {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "local CFG hash differs before admission normalization",
        ));
    }
    let mut normalized_raw_boundaries = Vec::with_capacity(raw_boundaries.len());
    for mut boundary in raw_boundaries {
        if !boundary.is_object() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "raw worker local CFG boundary is not an object",
            ));
        }
        if boundary.get("provider").and_then(Value::as_str) == Some("CODECLEW_LOCAL_CFG_NORMALIZER")
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "raw worker snapshot impersonates Codeclew local CFG normalization",
            ));
        }
        if boundary
            .get("ownerSymbolIdentity")
            .and_then(Value::as_str)
            .is_some_and(|owner| {
                crate::semantic_validation::validate_kotlin_full_symbol_identity(owner).is_err()
            })
        {
            boundary
                .as_object_mut()
                .expect("checked local CFG boundary object")
                .remove("ownerSymbolIdentity");
        }
        crate::thread_flow_cfg::validate_boundary(&boundary)?;
        normalized_raw_boundaries.push(boundary);
    }

    let proven = proven_descriptor_identities(descriptor_graph);
    let mut owner_counts = BTreeMap::<String, usize>::new();
    for graph in &raw_graphs {
        if let Ok(payload) =
            serde_json::from_value::<crate::thread_flow_cfg::LocalCfgPayload>(graph.clone())
        {
            *owner_counts
                .entry(payload.owner_symbol_identity)
                .or_default() += 1;
        }
    }
    let mut retained = Vec::new();
    let mut boundaries = normalized_raw_boundaries;
    for graph in raw_graphs {
        let raw_row_hash = crate::canonical::hash(&graph).map_err(internal)?;
        let parsed =
            serde_json::from_value::<crate::thread_flow_cfg::LocalCfgPayload>(graph.clone());
        let accepted = parsed.as_ref().is_ok_and(|payload| {
            crate::thread_flow_cfg::validate(payload).is_ok()
                && proven.contains(&payload.owner_symbol_identity)
                && owner_counts.get(&payload.owner_symbol_identity) == Some(&1)
        });
        if accepted {
            retained.push(graph);
            continue;
        }
        let code = parsed
            .as_ref()
            .ok()
            .filter(|payload| crate::thread_flow_cfg::validate(payload).is_ok())
            .map_or("INVALID_LOCAL_CFG", |_| "UNPROVEN_LOCAL_CFG_OWNER");
        boundaries.push(serde_json::json!({
            "schema":crate::thread_flow_cfg::LOCAL_CFG_BOUNDARY_SCHEMA,
            "stage":"NORMALIZE",
            "code":code,
            "resolution":"UNKNOWN",
            "provider":"CODECLEW_LOCAL_CFG_NORMALIZER",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "rawRowHash":raw_row_hash,
        }));
    }
    retained.sort_by_key(|row| crate::canonical::bytes(row).unwrap_or_default());
    boundaries.sort_by_key(|row| crate::canonical::bytes(row).unwrap_or_default());
    boundaries.dedup_by(|left, right| {
        crate::canonical::bytes(left).ok() == crate::canonical::bytes(right).ok()
    });
    facts["localCfgs"] = Value::Array(retained);
    facts["localCfgBoundaries"] = Value::Array(boundaries);
    let normalized_hash = local_cfg_snapshot_hash(
        facts["localCfgs"]
            .as_array()
            .expect("assigned local CFG graphs"),
        facts["localCfgBoundaries"]
            .as_array()
            .expect("assigned local CFG boundaries"),
    )?;
    facts["localCfgHash"] = Value::String(normalized_hash);
    validate_local_cfg_snapshot(facts, descriptor_graph)
}

impl VerifiedIndexFacts {
    pub fn authority(&self) -> &'static str {
        COMPILER_SEMANTIC_AUTHORITY
    }
}

impl VerifiedSourceSyntax {
    pub fn authority(&self) -> &'static str {
        SOURCE_SYNTAX_AUTHORITY
    }

    pub fn compilation(&self) -> &str {
        &self.compilation
    }
}

struct TrustedWorkerDistribution {
    workspace: PathBuf,
    #[cfg(test)]
    _private_root: Option<tempfile::TempDir>,
    distribution_root: PathBuf,
    launcher: PathBuf,
    #[cfg(test)]
    tree_manifest: BTreeMap<String, String>,
    tree_hash: String,
    build_input_digest: String,
    plugin_fingerprint: String,
}

impl KotlinSemanticEngine {
    #[cfg(test)]
    fn install_task(self) -> &'static str {
        match self {
            Self::Kotlin21 => ":workers:kotlin21:installDist",
            Self::Kotlin23 => ":workers:kotlin23:installDist",
            Self::Kotlin24 => ":workers:kotlin:installDist",
        }
    }

    #[cfg(test)]
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

    #[cfg(test)]
    fn pinned_inputs(self) -> PinnedInputs {
        match self {
            Self::Kotlin21 => panic!("Kotlin 2.1 is qualification-only and not a packaged runtime"),
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

#[cfg(test)]
struct PinnedInputs {
    roots: &'static [&'static str],
    files: &'static [&'static str],
    entries: &'static [(&'static str, &'static str)],
    digest: &'static str,
    outputs: &'static [(&'static str, u32, u64, &'static str)],
    output_digest: &'static str,
}

#[cfg(test)]
fn validate_compiler_index_root(workspace: &Path, root: &Path) -> Result<PathBuf, ClewError> {
    if !root.is_absolute() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be absolute",
        ));
    }
    let metadata = std::fs::symlink_metadata(root).map_err(internal)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "compiler index root must be private",
            ));
        }
    }
    let canonical = root.canonicalize().map_err(internal)?;
    if canonical != root {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be canonical and have no symlinked ancestor",
        ));
    }
    let workspace = workspace.canonicalize().map_err(internal)?;
    if canonical.starts_with(&workspace) || workspace.starts_with(&canonical) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be external to the worker workspace",
        ));
    }
    Ok(canonical)
}

fn configure_worker_state_environment(
    command: &mut Command,
    build_state_root: Option<&Path>,
    compiler_index_root: Option<&Path>,
    build_namespace_digest: &str,
) {
    command
        .env_remove("CODECLEW_K1_BUILD_STATE_ROOT")
        .env_remove("CODECLEW_K2_INDEX_ROOT")
        .env_remove("CODECLEW_K1_BUILD_STATE_NAMESPACE")
        .env("CODECLEW_K1_BUILD_STATE_NAMESPACE", build_namespace_digest);
    if let Some(root) = build_state_root {
        command.env("CODECLEW_K1_BUILD_STATE_ROOT", root);
    }
    if let Some(root) = compiler_index_root {
        command.env("CODECLEW_K2_INDEX_ROOT", root);
    }
}

fn validate_build_namespace_digest(value: &str) -> Result<String, ClewError> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "worker build namespace must be a canonical digest",
        ));
    }
    Ok(value.to_owned())
}

impl WorkerClient {
    #[cfg(test)]
    pub fn start(workspace: &Path) -> Result<Self, ClewError> {
        Self::start_engine(
            workspace,
            KotlinSemanticEngine::Kotlin24,
            None,
            None,
            None,
            None,
        )
    }

    pub(crate) fn start_with_managed_states(
        workspace: &Path,
        build_state_root: Option<&Path>,
        compiler_index_root: Option<&ManagedDirectory>,
        build_namespace_digest: &str,
    ) -> Result<Self, ClewError> {
        Self::start_engine(
            workspace,
            KotlinSemanticEngine::Kotlin24,
            build_state_root,
            compiler_index_root,
            Some(build_namespace_digest),
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn start_qualification_engine_with_managed_states(
        workspace: &Path,
        engine: KotlinSemanticEngine,
        build_state_root: Option<&Path>,
        compiler_index_root: Option<&ManagedDirectory>,
        build_namespace_digest: &str,
    ) -> Result<Self, ClewError> {
        Self::start_engine(
            workspace,
            engine,
            build_state_root,
            compiler_index_root,
            Some(build_namespace_digest),
            Some(engine),
        )
    }

    fn start_engine(
        workspace: &Path,
        engine: KotlinSemanticEngine,
        build_state_root: Option<&Path>,
        compiler_index_root: Option<&ManagedDirectory>,
        build_namespace_digest: Option<&str>,
        qualification_engine: Option<KotlinSemanticEngine>,
    ) -> Result<Self, ClewError> {
        #[cfg(unix)]
        use std::os::unix::process::CommandExt;
        let build_namespace_digest = build_namespace_digest
            .map(validate_build_namespace_digest)
            .transpose()?
            .unwrap_or_else(|| {
                crate::canonical::hash_bytes(b"codeclew-non-product-worker-namespace/2.0")
            });
        let trusted_distribution = prepare_trusted_worker_distribution(workspace, engine)?;
        let launcher = trusted_distribution.launcher.clone();
        let canonical_build_state = build_state_root
            .map(|root| root.canonicalize().map_err(internal))
            .transpose()?;
        let resolved_compiler_index = compiler_index_root
            .map(ManagedDirectory::resolved_path)
            .transpose()?;
        if let (Some(build_state), Some(compiler_index)) = (
            &canonical_build_state,
            compiler_index_root.map(ManagedDirectory::path),
        ) && (build_state.starts_with(compiler_index) || compiler_index.starts_with(build_state))
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "compiler index root and sealed build-state root must be disjoint",
            ));
        }
        let transport_root = tempfile::Builder::new()
            .prefix("codeclew-worker-transport-")
            .tempdir()
            .map_err(internal)?;
        let canonical_transport_root = transport_root.path().canonicalize().map_err(internal)?;
        let transport_metadata =
            std::fs::symlink_metadata(transport_root.path()).map_err(internal)?;
        if transport_metadata.file_type().is_symlink() || !transport_metadata.is_dir() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker transport root is not a private real directory",
            ));
        }
        let mut command = Command::new(&launcher);
        isolate_controller_authority(&mut command)?;
        configure_sealed_worker_process(&mut command, &canonical_transport_root);
        configure_worker_state_environment(
            &mut command,
            canonical_build_state.as_deref(),
            resolved_compiler_index.as_deref(),
            &build_namespace_digest,
        );
        if let Some(qualification_engine) = qualification_engine {
            if qualification_engine != engine {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "qualification engine must match the launched Kotlin engine",
                ));
            }
            command.env(
                "CODECLEW_KOTLIN_QUALIFICATION_ENGINE",
                qualification_engine.engine_id(),
            );
        }
        #[cfg(unix)]
        command.process_group(0);
        let task_run_spawn_permit = task_run_worker_spawn_permit()?;
        let mut child = command.spawn().map_err(|e| {
            ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("cannot start sealed Kotlin worker: {}", e.kind()),
            )
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = child.stdout.take().expect("piped stdout");
        let process = OwnedWorkerProcess::new(child, task_run_spawn_permit.is_some())?;
        drop(task_run_spawn_permit);
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
        if capabilities.compiler_version != engine.analyzer_compiler_version()
            || !capabilities.protocol_versions.iter().any(|v| v.major == 1)
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker compiler/protocol version mismatch",
            ));
        }
        Ok(Self {
            workspace: workspace.to_path_buf(),
            engine,
            qualification_engine,
            process,
            stdin,
            stdout,
            next_id: 1,
            snapshot: None,
            capabilities,
            last_profile: RequestProfile::default(),
            authority_session: Uuid::new_v4(),
            trusted_distribution,
            build_state_root: canonical_build_state,
            compiler_index_root: compiler_index_root.cloned(),
            build_namespace_digest,
            _transport_root: transport_root,
            transport_root: canonical_transport_root,
            issued_index_facts: BTreeMap::new(),
            issued_source_syntax: BTreeMap::new(),
            request_counters: WorkerRequestCounters::default(),
        })
    }

    pub(crate) fn cancellation_handle(&self) -> WorkerCancellationHandle {
        self.process.cancellation_handle()
    }

    fn switch_engine(&mut self, engine: KotlinSemanticEngine) -> Result<(), ClewError> {
        if self.engine == engine {
            return Ok(());
        }
        let mut replacement = Self::start_engine(
            &self.workspace,
            engine,
            self.build_state_root.as_deref(),
            self.compiler_index_root.as_ref(),
            Some(&self.build_namespace_digest),
            None,
        )?;
        replacement.request_counters = self.request_counters;
        let previous = std::mem::replace(self, replacement);
        previous.shutdown()
    }

    fn request(&mut self, kind: RequestKind, payload: &Value) -> Result<Value, ClewError> {
        match kind {
            RequestKind::OpenProject => {
                self.request_counters.open_project_requests = self
                    .request_counters
                    .open_project_requests
                    .saturating_add(1);
            }
            RequestKind::IndexFiles => {
                self.request_counters.index_files_requests =
                    self.request_counters.index_files_requests.saturating_add(1);
            }
            _ => {}
        }
        self.request_with_discovery_variants(kind, payload, 0)
    }

    fn request_with_discovery_variants(
        &mut self,
        kind: RequestKind,
        payload: &Value,
        tried_discovery_variants: u8,
    ) -> Result<Value, ClewError> {
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
                if kind == RequestKind::IndexFiles
                    && payload.get("syntaxOnly").and_then(Value::as_bool) == Some(true)
                {
                    snapshot_from(payload)
                } else {
                    self.snapshot
                        .clone()
                        .unwrap_or_else(|| snapshot_from(payload))
                },
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
                let code = parse_worker_code(&error.code);
                let failure = ClewError {
                    message: safe_worker_error_message(&code).into(),
                    code,
                    transaction_id: None,
                    snapshot_id: self.snapshot.as_ref().map(snapshot_label),
                    // The worker may wrap third-party compiler/build output.
                    // It is not an evidence authority and can contain absolute
                    // paths, environment values, or unbounded diagnostics.
                    evidence: Vec::new(),
                    relevant_anchors_or_symbols: relevant.into_boxed_slice(),
                    retryable: error.retryable,
                };
                if self.qualification_engine.is_none()
                    && kind == RequestKind::OpenProject
                    && failure.code == ErrorCode::UnsupportedCompilerPluginAbi
                {
                    let tried = tried_discovery_variants | self.engine.discovery_bit();
                    let available = available_project_engines()?;
                    if let Some(next) = next_available_discovery_engine(tried, &available) {
                        self.switch_engine(next)?;
                        return self.request_with_discovery_variants(kind, payload, tried);
                    }
                }
                return Err(failure);
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
                let canonical_json =
                    index_response_body(value.canonical_json, &value.blobs, &self.transport_root)?;
                validate_typed_string(&canonical_json, "/indexHash", &value.index_hash)?;
                validate_typed_string(
                    &canonical_json,
                    "/projectModelHash",
                    &value.project_model_hash,
                )?;
                validate_typed_string(&canonical_json, "/compilation", &value.compilation)?;
                validate_typed_count(&canonical_json, "/files", value.file_count)?;
                validate_typed_bool(&canonical_json, "/partial", value.partial)?;
                canonical_json
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
        if kind == RequestKind::OpenProject {
            let project_semantics = KotlinProjectSemantics::from_project_model(&value)?;
            let desired = if let Some(engine) = self.qualification_engine {
                KotlinEngineRegistry.qualify(&project_semantics, engine)?
            } else {
                let available = available_project_engines()?;
                select_available_project_engine(&project_semantics, &available)?
            };
            if desired != self.engine {
                let tried = tried_discovery_variants | self.engine.discovery_bit();
                if tried & desired.discovery_bit() != 0 {
                    return Err(ClewError::new(
                        ErrorCode::UnsupportedCompilerPluginAbi,
                        "qualified Kotlin semantic engine could not open the project with its compiler plugins",
                    ));
                }
                self.switch_engine(desired)?;
                return self.request_with_discovery_variants(kind, payload, tried);
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
        let profiling = take_worker_profiling(&mut value);
        let worker_processing_micros = profiling
            .as_ref()
            .and_then(|profile| profile.get("workerProcessingMicros"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.last_profile = RequestProfile {
            serialization_micros: request_construction_micros
                + encode_micros
                + decode_micros
                + json_micros,
            ipc_micros: (write_micros + read_micros).saturating_sub(worker_processing_micros),
            worker_processing_micros,
            cache_requests: profiling
                .as_ref()
                .and_then(|profile| profile.get("cacheRequests"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_hits: profiling
                .as_ref()
                .and_then(|profile| profile.get("cacheHits"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            psi_parse_micros: profiling
                .as_ref()
                .and_then(|profile| profile.get("psiParseMicros"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            k2_analysis_micros: profiling
                .as_ref()
                .and_then(|profile| profile.get("k2AnalysisMicros"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            fir_extraction_micros: profiling
                .as_ref()
                .and_then(|profile| profile.get("firExtractionMicros"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            compiler_index: (kind == RequestKind::IndexFiles
                && payload.get("syntaxOnly").and_then(Value::as_bool) != Some(true))
            .then(|| profiling.as_ref().and_then(parse_compiler_index_profile))
            .flatten(),
            project_model_cache: profiling
                .as_ref()
                .and_then(parse_project_model_cache_profile),
        };
        Ok(value)
    }

    /// Issue a `SOURCE_SYNTAX` capability without opening or trusting a build
    /// model. The capability proves only the current file/declaration payload;
    /// compiler-semantic consumers cannot accept this distinct type.
    pub fn index_files_source_syntax_verified(
        &mut self,
        payload: &Value,
    ) -> Result<VerifiedSourceSyntax, ClewError> {
        if payload.get("syntaxOnly").and_then(Value::as_bool) != Some(true) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX indexing requires syntaxOnly true",
            ));
        }
        let repo = payload
            .get("repo")
            .and_then(Value::as_str)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "SOURCE_SYNTAX needs repo"))?;
        let repo = Path::new(repo).canonicalize().map_err(internal)?;
        let metadata = std::fs::symlink_metadata(&repo).map_err(internal)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX repo must be a real directory",
            ));
        }
        let requested_compilation = payload
            .get("compilation")
            .and_then(Value::as_str)
            .unwrap_or(":/main")
            .to_owned();
        let requested_files = match payload.get("files") {
            Some(Value::Array(files)) => files
                .iter()
                .map(|file| {
                    file.as_str().map(str::to_owned).ok_or_else(|| {
                        ClewError::new(
                            ErrorCode::InvalidInput,
                            "SOURCE_SYNTAX files must be strings",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "SOURCE_SYNTAX requires an exact files array",
                ));
            }
        };
        let trusted = &self.trusted_distribution;
        if trusted.workspace != workspace_root() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "SOURCE_SYNTAX worker workspace identity changed",
            ));
        }
        verify_trusted_distribution(trusted)?;
        let trusted_tree_hash = trusted.tree_hash.clone();
        let trusted_build_input_digest = trusted.build_input_digest.clone();

        let mut exact_payload = payload.clone();
        if !exact_payload.is_object() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX request must be an object",
            ));
        }
        exact_payload["repo"] = Value::String(repo.to_string_lossy().into_owned());
        exact_payload["compilation"] = Value::String(requested_compilation.clone());
        exact_payload["syntaxOnly"] = Value::Bool(true);
        let facts = self.request(RequestKind::IndexFiles, &exact_payload)?;
        let source_manifest_hash = validate_source_syntax_response(
            &repo,
            &requested_compilation,
            &requested_files,
            &facts,
        )?;
        let payload_hash = crate::canonical::hash(&facts).map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        let seal = crate::canonical::hash(&serde_json::json!({
            "authority":SOURCE_SYNTAX_AUTHORITY,
            "receiptId":receipt_id,
            "session":self.authority_session,
            "repo":repo,
            "compilation":requested_compilation,
            "distributionTree":trusted_tree_hash,
            "buildInputs":trusted_build_input_digest,
            "requestedFiles":requested_files,
            "sourceManifestHash":source_manifest_hash,
            "payloadHash":payload_hash,
        }))
        .map_err(internal)?;
        self.issued_source_syntax.insert(receipt_id, seal);
        Ok(VerifiedSourceSyntax {
            receipt_id,
            authority_session: self.authority_session,
            repo,
            compilation: requested_compilation,
            distribution_tree_hash: trusted_tree_hash,
            build_input_digest: trusted_build_input_digest,
            requested_files,
            source_manifest_hash,
            payload_hash,
            payload: facts,
        })
    }

    /// Inspect current declaration-only facts. This grants no persistence or
    /// transaction authority and fails if any bound source changed.
    pub fn inspect_verified_source_syntax<'a>(
        &self,
        syntax: &'a VerifiedSourceSyntax,
    ) -> Result<&'a Value, ClewError> {
        let trusted = &self.trusted_distribution;
        verify_trusted_distribution(trusted)?;
        let current_manifest_hash = validate_source_syntax_response(
            &syntax.repo,
            &syntax.compilation,
            &syntax.requested_files,
            &syntax.payload,
        )?;
        let seal = crate::canonical::hash(&serde_json::json!({
            "authority":SOURCE_SYNTAX_AUTHORITY,
            "receiptId":syntax.receipt_id,
            "session":syntax.authority_session,
            "repo":syntax.repo,
            "compilation":syntax.compilation,
            "distributionTree":syntax.distribution_tree_hash,
            "buildInputs":syntax.build_input_digest,
            "requestedFiles":syntax.requested_files,
            "sourceManifestHash":syntax.source_manifest_hash,
            "payloadHash":syntax.payload_hash,
        }))
        .map_err(internal)?;
        if syntax.authority_session != self.authority_session
            || syntax.distribution_tree_hash != trusted.tree_hash
            || syntax.build_input_digest != trusted.build_input_digest
            || current_manifest_hash != syntax.source_manifest_hash
            || crate::canonical::hash(&syntax.payload).map_err(internal)? != syntax.payload_hash
            || self.issued_source_syntax.get(&syntax.receipt_id) != Some(&seal)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "SOURCE_SYNTAX capability is stale, altered, forged, or belongs to another session",
            ));
        }
        Ok(&syntax.payload)
    }

    /// Execute OpenProject + IndexFiles through the pinned worker distribution
    /// and issue an unforgeable, session-local capability for the exact result.
    pub fn index_files_verified(
        &mut self,
        payload: &Value,
    ) -> Result<VerifiedIndexFacts, ClewError> {
        self.open_project_and_index_verified(payload)
            .map(|(_, facts)| facts)
    }

    pub fn request_counters(&self) -> WorkerRequestCounters {
        self.request_counters
    }

    /// Establish one exact live OpenProject authority without executing
    /// semantic compiler indexing. The returned model is session-bound and is
    /// accepted by `index_files_verified_after_project` only on this client.
    pub fn open_project_verified(&mut self, payload: &Value) -> Result<Value, ClewError> {
        let repo = payload.get("repo").and_then(Value::as_str).ok_or_else(|| {
            ClewError::new(ErrorCode::InvalidInput, "verified project needs repo")
        })?;
        let repo = Path::new(repo).canonicalize().map_err(internal)?;
        let requested_compilation = payload
            .get("compilation")
            .and_then(Value::as_str)
            .unwrap_or(":/main")
            .to_owned();
        self.request(
            RequestKind::OpenProject,
            &serde_json::json!({"repo":repo,"compilation":requested_compilation}),
        )
    }

    /// Execute one live OpenProject followed by IndexFiles and return both the
    /// exact project authority and its sealed semantic facts. Callers that need
    /// the project model must use this combined contour instead of issuing a
    /// second independent OpenProject, which is intentionally refreshed in
    /// project-native mode.
    pub fn open_project_and_index_verified(
        &mut self,
        payload: &Value,
    ) -> Result<(Value, VerifiedIndexFacts), ClewError> {
        let project = self.open_project_verified(payload)?;
        let facts = self.index_files_verified_after_project(payload, &project)?;
        Ok((project, facts))
    }

    /// Index against the exact live OpenProject currently held by this worker.
    /// This supports cache lookup between build discovery and semantic indexing
    /// without a second native Maven/Gradle model extraction. The supplied model
    /// must still match the worker's live snapshot and the IndexFiles response.
    pub fn index_files_verified_after_project(
        &mut self,
        payload: &Value,
        project: &Value,
    ) -> Result<VerifiedIndexFacts, ClewError> {
        if payload.get("syntaxOnly").and_then(Value::as_bool) == Some(true) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "COMPILER_SEMANTIC indexing cannot use syntaxOnly",
            ));
        }
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
        let open_project_model_cache = self.last_profile.project_model_cache.clone();
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
        let provided_project_model_hash = required_payload_string(project, "projectModelHash")?;
        if self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.project_model_hash.as_str())
            != Some(provided_project_model_hash)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "provided OpenProject is not the worker's live project authority",
            ));
        }
        // OpenProject may discover and switch to the project's Kotlin worker
        // engine. Bind the semantic receipt to that selected distribution,
        // never to the bootstrap engine that happened to start the session.
        let trusted = &self.trusted_distribution;
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
        let project_model_hash = provided_project_model_hash.to_owned();
        let semantic_input_manifest_hash =
            required_payload_string(project, "semanticInputManifestHash")?.to_owned();
        let manifest = project.get("semanticInputManifest").ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "OpenProject has no semantic input manifest",
            )
        })?;
        if crate::canonical::hash(manifest).map_err(internal)? != semantic_input_manifest_hash {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "OpenProject semantic input manifest hash is invalid",
            ));
        }
        let mut exact_payload = payload.clone();
        exact_payload["repo"] = Value::String(repo.to_string_lossy().into_owned());
        exact_payload["compilation"] = Value::String(requested_compilation.clone());
        let index_result = self.request(RequestKind::IndexFiles, &exact_payload);
        retain_verified_index_project_model_profile(
            &mut self.last_profile,
            open_project_model_cache,
        );
        let mut facts = index_result
            .map_err(|error| attach_verified_index_failure(error, "RAW_SCHEMA_HASH", None))?;
        if facts.get("compilation").and_then(Value::as_str) != Some(requested_compilation.as_str())
            || facts.get("projectModelHash").and_then(Value::as_str)
                != Some(project_model_hash.as_str())
            || facts.get("compilerVersion").and_then(Value::as_str)
                != Some(self.capabilities.compiler_version.as_str())
            || facts
                .get("semanticInputManifestHash")
                .and_then(Value::as_str)
                != Some(semantic_input_manifest_hash.as_str())
            || facts.get("semanticInputManifest") != project.get("semanticInputManifest")
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
        require_k2_validated(&facts)
            .map_err(|error| attach_verified_index_failure(error, "K2_VALIDATION", Some(&facts)))?;
        if facts.get("analysisMode").and_then(Value::as_str) != Some("K2_SEMANTIC") {
            return Err(attach_verified_index_failure(
                ClewError::new(
                    ErrorCode::IncompleteSemanticAnalysis,
                    "COMPILER_SEMANTIC response requires analysisMode K2_SEMANTIC",
                ),
                "K2_VALIDATION",
                Some(&facts),
            ));
        }
        normalize_invalid_jvm_descriptors(&mut facts).map_err(|error| {
            attach_verified_index_failure(error, "DESCRIPTOR_NORMALIZATION", Some(&facts))
        })?;
        verify_descriptor_line_coordinates(&repo, &facts).map_err(|error| {
            attach_verified_index_failure(error, "DESCRIPTOR_LINE_PROOF", Some(&facts))
        })?;
        let descriptor = crate::semantic_validation::validate_declaration_descriptor_snapshot(
            &facts,
        )
        .map_err(|error| attach_verified_index_failure(error, "DESCRIPTOR_GRAPH", Some(&facts)))?;
        let local_cfg_hash =
            normalize_local_cfg_evidence(&mut facts, &descriptor.graph).map_err(|error| {
                attach_verified_index_failure(error, "LOCAL_CFG_GRAPH", Some(&facts))
            })?;
        normalize_optional_relation_evidence(&mut facts).map_err(|error| {
            attach_verified_index_failure(error, "RELATION_NORMALIZATION", Some(&facts))
        })?;
        let relation = crate::semantic_validation::validate_declaration_relation_snapshot(&facts)
            .map_err(|error| {
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
            "localCfgHash":local_cfg_hash,
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
            local_cfg_hash,
            payload: facts,
        })
    }

    /// Read-only projection of a capability issued by this exact session.
    pub fn inspect_verified_index<'a>(
        &self,
        facts: &'a VerifiedIndexFacts,
    ) -> Result<&'a Value, ClewError> {
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
            "localCfgHash":facts.local_cfg_hash,
        }))
        .map_err(internal)?;
        if facts.authority_session != self.authority_session
            || self.issued_index_facts.get(&facts.receipt_id) != Some(&seal)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "verified index capability is not issued by this live session",
            ));
        }
        Ok(&facts.payload)
    }

    #[cfg(test)]
    fn authorize_index_facts<'a>(
        &self,
        facts: &'a VerifiedIndexFacts,
        source_root: &Path,
        compilation: &str,
    ) -> Result<&'a Value, ClewError> {
        let source_root = source_root.canonicalize().map_err(internal)?;
        let trusted = &self.trusted_distribution;
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
            "localCfgHash":facts.local_cfg_hash,
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
        require_k2_validated(&facts.payload)?;
        verify_descriptor_line_coordinates(&source_root, &facts.payload)?;
        let relation =
            crate::semantic_validation::validate_declaration_relation_snapshot(&facts.payload)?;
        let descriptor =
            crate::semantic_validation::validate_declaration_descriptor_snapshot(&facts.payload)?;
        let local_cfg_hash = validate_local_cfg_snapshot(&facts.payload, &descriptor.graph)?;
        if relation.hash != facts.relation_hash
            || descriptor.hash != facts.descriptor_hash
            || local_cfg_hash != facts.local_cfg_hash
        {
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
            .process
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

fn index_response_body(
    inline: Vec<u8>,
    blobs: &[BlobRef],
    transport_root: &Path,
) -> Result<Vec<u8>, ClewError> {
    match (inline.is_empty(), blobs) {
        (false, []) => Ok(inline),
        (true, [blob]) => read_worker_transport_blob(transport_root, blob),
        (false, _) => Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "IndexFiles response contains both inline and blob bodies",
        )),
        (true, []) => Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "IndexFiles response has no canonical body",
        )),
        (true, _) => Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "IndexFiles response has multiple canonical body blobs",
        )),
    }
}

fn read_worker_transport_blob(root: &Path, blob: &BlobRef) -> Result<Vec<u8>, ClewError> {
    fn mismatch(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::WorkerProtocolMismatch, message)
    }

    if !root.is_absolute() {
        return Err(mismatch("worker transport root is not absolute"));
    }
    let root_metadata =
        std::fs::symlink_metadata(root).map_err(|error| mismatch(error.to_string()))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(mismatch("worker transport root is not a real directory"));
    }
    if root
        .canonicalize()
        .map_err(|error| mismatch(error.to_string()))?
        != root
    {
        return Err(mismatch("worker transport root canonical identity changed"));
    }

    let digest = blob
        .content_hash
        .strip_prefix("sha256:")
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| mismatch("worker transport BlobRef hash is malformed"))?;
    let expected_relative = format!("sha256/{digest}");
    if blob.relative_path != expected_relative {
        return Err(mismatch(
            "worker transport BlobRef path is not its content-addressed name",
        ));
    }
    if blob.size_bytes == 0 || blob.size_bytes > MAX_WORKER_RESPONSE_BLOB_BYTES {
        return Err(mismatch(
            "worker transport BlobRef size exceeds the bounded response policy",
        ));
    }

    let directory = root.join("sha256");
    let directory_metadata =
        std::fs::symlink_metadata(&directory).map_err(|error| mismatch(error.to_string()))?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(mismatch("worker transport CAS directory is unsafe"));
    }
    if directory
        .canonicalize()
        .map_err(|error| mismatch(error.to_string()))?
        .parent()
        != Some(root)
    {
        return Err(mismatch("worker transport CAS directory escapes its root"));
    }

    let path = root.join(&blob.relative_path);
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| mismatch(error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != blob.size_bytes
    {
        return Err(mismatch(
            "worker transport blob is not the declared regular file",
        ));
    }
    let file = std::fs::File::open(&path).map_err(|error| mismatch(error.to_string()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| mismatch(error.to_string()))?;
    if !opened_metadata.is_file() || opened_metadata.len() != blob.size_bytes {
        return Err(mismatch("worker transport blob changed while opening"));
    }
    let mut bytes = Vec::with_capacity(blob.size_bytes as usize);
    file.take(MAX_WORKER_RESPONSE_BLOB_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| mismatch(error.to_string()))?;
    if bytes.len() as u64 != blob.size_bytes {
        return Err(mismatch("worker transport blob size changed while reading"));
    }
    if crate::canonical::hash_bytes(&bytes) != blob.content_hash {
        return Err(mismatch("worker transport blob content hash mismatch"));
    }
    let _ = std::fs::remove_file(path);
    Ok(bytes)
}

#[cfg(test)]
fn worker_launcher(workspace: &Path, engine: KotlinSemanticEngine) -> PathBuf {
    match engine {
        KotlinSemanticEngine::Kotlin21 => {
            workspace.join("workers/kotlin21/build/install/kotlin21/bin/kotlin21")
        }
        KotlinSemanticEngine::Kotlin23 => {
            workspace.join("workers/kotlin23/build/install/kotlin23/bin/kotlin23")
        }
        KotlinSemanticEngine::Kotlin24 => {
            workspace.join("workers/kotlin/build/install/kotlin/bin/kotlin")
        }
    }
}

fn available_project_engines() -> Result<Vec<KotlinSemanticEngine>, ClewError> {
    let runtime = RuntimeAuthority::from_environment()?;
    Ok(KotlinSemanticEngine::all_known()
        .into_iter()
        .filter(|engine| {
            runtime.as_ref().is_none_or(|runtime| {
                runtime
                    .workers
                    .get(engine.runtime_name())
                    .is_some_and(|worker| {
                        worker.compiler_version == engine.analyzer_compiler_version()
                    })
            })
        })
        .collect())
}

// A compiler-plugin rejection must not trigger preparation of an optional
// engine absent from the verified runtime. Preserve the original rejection
// when no available engine remains instead of masking it with a missing pack.
fn next_available_discovery_engine(
    tried: u8,
    available: &[KotlinSemanticEngine],
) -> Option<KotlinSemanticEngine> {
    KotlinSemanticEngine::packaged_by_preference()
        .into_iter()
        .find(|engine| tried & engine.discovery_bit() == 0 && available.contains(engine))
}

// Default discovery may use the core analysis engine when an optional exact
// engine is absent. Explicit qualification requests bypass this selection.
fn select_available_project_engine(
    project: &KotlinProjectSemantics,
    available: &[KotlinSemanticEngine],
) -> Result<KotlinSemanticEngine, ClewError> {
    let desired = KotlinEngineRegistry.select(project)?;
    if desired == KotlinSemanticEngine::Kotlin23
        && !available.contains(&desired)
        && available.contains(&KotlinSemanticEngine::Kotlin24)
    {
        return KotlinEngineRegistry.qualify(project, KotlinSemanticEngine::Kotlin24);
    }
    Ok(desired)
}

fn prepare_trusted_worker_distribution(
    workspace: &Path,
    engine: KotlinSemanticEngine,
) -> Result<TrustedWorkerDistribution, ClewError> {
    let canonical = workspace.canonicalize().map_err(internal)?;
    let runtime = match RuntimeAuthority::from_environment()? {
        Some(runtime) => runtime,
        None => {
            #[cfg(test)]
            return prepare_checkout_worker_distribution_for_test(&canonical, engine);
            #[cfg(not(test))]
            return Err(preparation_required(
                "sealed runtime capsule authority is required to start a Kotlin worker",
            ));
        }
    };
    if runtime.root != canonical {
        return Err(preparation_required(
            "runtime workspace differs from the verified capsule root",
        ));
    }
    let runtime_worker = runtime.worker(engine.runtime_name())?;
    if runtime_worker.compiler_version != engine.analyzer_compiler_version() {
        return Err(preparation_required(
            "runtime worker compiler identity differs from the selected semantic engine",
        ));
    }
    let source_distribution = runtime.verify_worker(engine.runtime_name())?;
    let tree_manifest = runtime_worker_manifest(runtime_worker);
    let tree_hash = hash_string_manifest(&tree_manifest);
    if tree_hash != runtime_worker.tree_hash {
        return Err(preparation_required(
            "runtime worker manifest differs from runtime authority",
        ));
    }
    let launcher = source_distribution.join("bin").join(engine.launcher_name());
    let plugin = source_distribution
        .join("lib")
        .join(engine.plugin_jar_name());
    if !launcher.is_file() || !plugin.is_file() {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "runtime worker distribution is incomplete",
        ));
    }
    Ok(TrustedWorkerDistribution {
        workspace: canonical,
        #[cfg(test)]
        _private_root: None,
        distribution_root: source_distribution,
        launcher,
        #[cfg(test)]
        tree_manifest,
        tree_hash,
        build_input_digest: runtime.runtime_key,
        plugin_fingerprint: crate::canonical::hash_bytes(&std::fs::read(plugin).map_err(internal)?),
    })
}

#[cfg(test)]
fn prepare_checkout_worker_distribution_for_test(
    canonical: &Path,
    engine: KotlinSemanticEngine,
) -> Result<TrustedWorkerDistribution, ClewError> {
    if canonical != workspace_root() {
        return Err(preparation_required(
            "test worker workspace differs from the checkout fixture root",
        ));
    }
    let pinned = engine.pinned_inputs();
    verify_pinned_build_inputs(canonical, &pinned)?;
    reject_unpinned_workspace_build_initialization(canonical)?;
    let source_distribution = canonical.join(engine.distribution_relative());
    if bootstrap_trusted_worker_distribution_if_missing(canonical, engine, &source_distribution)? {
        verify_pinned_build_inputs(canonical, &pinned)?;
    }
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
    let launcher = distribution_root.join("bin").join(engine.launcher_name());
    let plugin = distribution_root.join("lib").join(engine.plugin_jar_name());
    if !launcher.is_file() || !plugin.is_file() {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "rebuilt pinned worker distribution is incomplete",
        ));
    }
    Ok(TrustedWorkerDistribution {
        workspace: canonical.to_path_buf(),
        _private_root: Some(private_root),
        distribution_root,
        launcher,
        tree_manifest,
        tree_hash,
        build_input_digest: pinned.digest.to_owned(),
        plugin_fingerprint: crate::canonical::hash_bytes(&std::fs::read(plugin).map_err(internal)?),
    })
}

fn runtime_worker_manifest(worker: &crate::runtime::RuntimeWorker) -> BTreeMap<String, String> {
    worker
        .files
        .iter()
        .map(|artifact| {
            (
                artifact.path.clone(),
                format!("{}:{}:{}", artifact.mode, artifact.size, artifact.sha256),
            )
        })
        .collect()
}

#[cfg(test)]
fn bootstrap_trusted_worker_distribution_if_missing(
    workspace: &Path,
    engine: KotlinSemanticEngine,
    distribution: &Path,
) -> Result<bool, ClewError> {
    bootstrap_trusted_worker_distribution_if_missing_with_environment(
        workspace,
        engine,
        distribution,
        std::env::vars_os(),
    )
}

#[cfg(test)]
fn bootstrap_trusted_worker_distribution_if_missing_with_environment(
    workspace: &Path,
    engine: KotlinSemanticEngine,
    distribution: &Path,
    build_environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<bool, ClewError> {
    match std::fs::symlink_metadata(distribution) {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(internal(error)),
    }

    require_safe_missing_distribution_path(workspace, distribution)?;
    let wrapper = workspace.join("gradlew");
    let metadata = std::fs::symlink_metadata(&wrapper).map_err(internal)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(preparation_required(
            "trusted worker Gradle wrapper is not a regular pinned file",
        ));
    }

    let mut command = Command::new(&wrapper);
    command
        .args([engine.install_task(), "--no-daemon", "--quiet"])
        .current_dir(workspace)
        // A cold start must resolve the worker with the same Gradle/JVM setup
        // as the repository wrapper: caller caches, mirrors and JVM options are
        // part of build availability. They are not trusted as authority. The
        // pinned source closure is checked before and after this command and
        // the resulting installDist must match the embedded output manifest.
        .env_clear()
        .envs(build_environment);
    let output = command.output().map_err(|error| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("cannot build trusted Kotlin worker: {error}"),
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let diagnostic = stderr.chars().take(2_048).collect::<String>();
        return Err(ClewError::new(
            ErrorCode::WorkerCrashed,
            format!(
                "trusted Kotlin worker build failed with {}{}{}",
                output.status,
                if diagnostic.trim().is_empty() {
                    ""
                } else {
                    ": "
                },
                diagnostic.trim()
            ),
        ));
    }
    Ok(true)
}

#[cfg(test)]
fn require_safe_missing_distribution_path(
    workspace: &Path,
    distribution: &Path,
) -> Result<(), ClewError> {
    let relative = distribution.strip_prefix(workspace).map_err(|_| {
        preparation_required("trusted worker distribution path escapes its workspace")
    })?;
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(preparation_required(
                        "trusted worker distribution path has a non-directory or symlinked ancestor",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(internal(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
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

#[cfg(test)]
fn reject_unpinned_workspace_build_initialization(workspace: &Path) -> Result<(), ClewError> {
    if workspace.join("init.gradle").exists() || workspace.join("init.gradle.kts").exists() {
        return Err(preparation_required(
            "trusted worker start refuses caller Gradle initialization",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn verify_pinned_distribution(root: &Path, pinned: &PinnedInputs) -> Result<(), ClewError> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            preparation_required(
                "worker installDist is absent after the trusted distribution build",
            )
        } else {
            internal(error)
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(preparation_required(
            "worker installDist differs from embedded expected output manifest",
        ));
    }
    let actual = regular_tree_manifest(root)?;
    let expected = pinned
        .outputs
        .iter()
        .map(|(path, mode, size, hash)| ((*path).to_owned(), format!("{mode}:{size}:{hash}")))
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 != 0 {
                    0o111
                } else {
                    0
                }
            };
            #[cfg(not(unix))]
            let mode = 0;
            manifest.insert(
                path,
                format!(
                    "{mode}:{size}:{}",
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
    for (path, mode_size_and_hash) in manifest {
        let (mode, size_and_hash) = mode_size_and_hash
            .split_once(':')
            .unwrap_or(("", mode_size_and_hash));
        let (size, hash) = size_and_hash.split_once(':').unwrap_or(("", size_and_hash));
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(mode.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(size.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(hash.as_bytes());
        bytes.push(0);
    }
    crate::canonical::hash_bytes(&bytes)
}

#[cfg(test)]
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
    let metadata = std::fs::symlink_metadata(&trusted.distribution_root).map_err(internal)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "sealed runtime worker distribution lost its directory identity",
        ));
    }
    #[cfg(test)]
    if trusted._private_root.is_some() {
        let actual = regular_tree_manifest(&trusted.distribution_root)?;
        if actual != trusted.tree_manifest || hash_string_manifest(&actual) != trusted.tree_hash {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "private worker distribution changed after trusted launch",
            ));
        }
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
    if size > MAX_WORKER_FRAME_BYTES {
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
    if size > MAX_WORKER_FRAME_BYTES {
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

fn take_worker_profiling(value: &mut Value) -> Option<Value> {
    value
        .as_object_mut()
        .and_then(|object| object.remove("profiling"))
}

fn parse_compiler_index_profile(profiling: &Value) -> Option<CompilerIndexProfile> {
    let object = profiling.as_object()?;
    let backend = match object.get("backend")?.as_str()? {
        "BTA_PERSISTENT" => CompilerIndexBackend::BtaPersistent,
        _ => return None,
    };
    let status = match object.get("status")?.as_str()? {
        "UNCHANGED_HIT" => CompilerIndexStatus::UnchangedHit,
        "COLD_FULL" => CompilerIndexStatus::ColdFull,
        "INCREMENTAL" => CompilerIndexStatus::Incremental,
        "RECOVERED_FULL" => CompilerIndexStatus::RecoveredFull,
        "BUSY" => CompilerIndexStatus::Busy,
        "FAILED_RECOVERABLE" => CompilerIndexStatus::FailedRecoverable,
        _ => return None,
    };
    let valid = object.get("valid")?.as_bool()?;
    let total_micros = object.get("totalMicros")?.as_u64()?;
    let compiler_micros = object.get("compilerMicros")?.as_u64()?;
    let fir_extraction_micros = object.get("firExtractionMicros")?.as_u64()?;
    let total_files = object.get("totalFiles")?.as_u64()?;
    let compiled_files = object.get("compiledFiles")?.as_u64()?;
    let reused_files = object.get("reusedFiles")?.as_u64()?;
    let recovered = object.get("recovered")?.as_bool()?;
    let fallback_used = object.get("fallbackUsed")?.as_bool()?;
    let failure_code = match object.get("failureCode") {
        None | Some(Value::Null) => None,
        Some(Value::String(code))
            if !code.is_empty()
                && code.len() <= 96
                && code.bytes().all(|byte| {
                    byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                }) =>
        {
            Some(code.clone())
        }
        Some(_) => return None,
    };
    let graph_digest = match object.get("graphDigest") {
        None | Some(Value::Null) => None,
        Some(Value::String(digest)) if is_lowercase_sha256(digest) => Some(digest.clone()),
        Some(_) => return None,
    };
    let semantic_input_manifest_digest = object
        .get("semanticInputManifestDigest")?
        .as_str()?
        .to_owned();
    let facts_plugin_digest = object.get("factsPluginDigest")?.as_str()?.to_owned();
    let extractor_authority_digest = object.get("extractorAuthorityDigest")?.as_str()?.to_owned();
    let semantic_configuration_digest = object
        .get("semanticConfigurationDigest")?
        .as_str()?
        .to_owned();
    if ![
        &semantic_input_manifest_digest,
        &facts_plugin_digest,
        &extractor_authority_digest,
        &semantic_configuration_digest,
    ]
    .into_iter()
    .all(|digest| is_prefixed_lowercase_sha256(digest))
    {
        return None;
    }

    let covered_files = compiled_files.checked_add(reused_files)?;
    let no_compiled_graph = compiled_files == 0 && reused_files == 0 && graph_digest.is_none();
    let profile_is_consistent = match (status, valid) {
        (CompilerIndexStatus::UnchangedHit, true) => {
            compiled_files == 0 && reused_files == total_files && !recovered && !fallback_used
        }
        (CompilerIndexStatus::ColdFull, true) => {
            compiled_files == total_files && reused_files == 0 && !recovered && !fallback_used
        }
        (CompilerIndexStatus::Incremental, true) => {
            covered_files == total_files && !recovered && !fallback_used
        }
        (CompilerIndexStatus::RecoveredFull, true) => {
            compiled_files == total_files && reused_files == 0 && recovered && !fallback_used
        }
        (CompilerIndexStatus::ColdFull | CompilerIndexStatus::Incremental, false) => {
            no_compiled_graph && !recovered && !fallback_used
        }
        (CompilerIndexStatus::RecoveredFull, false) => {
            no_compiled_graph && recovered && !fallback_used
        }
        (CompilerIndexStatus::Busy | CompilerIndexStatus::FailedRecoverable, false) => {
            no_compiled_graph && fallback_used
        }
        (CompilerIndexStatus::UnchangedHit, false)
        | (CompilerIndexStatus::Busy | CompilerIndexStatus::FailedRecoverable, true) => false,
    };
    if !profile_is_consistent {
        return None;
    }

    Some(CompilerIndexProfile {
        backend,
        status,
        valid,
        total_micros,
        compiler_micros,
        fir_extraction_micros,
        total_files,
        compiled_files,
        reused_files,
        recovered,
        fallback_used,
        failure_code,
        graph_digest,
        semantic_input_manifest_digest,
        facts_plugin_digest,
        extractor_authority_digest,
        semantic_configuration_digest,
    })
}

fn parse_project_model_cache_profile(profiling: &Value) -> Option<ProjectModelCacheProfile> {
    let object = profiling.as_object()?;
    let status = match object.get("projectModelCacheStatus")?.as_str()? {
        "MEMORY_HIT" => ProjectModelCacheStatus::MemoryHit,
        "PERSISTENT_HIT" => ProjectModelCacheStatus::PersistentHit,
        "EXTRACTED_PUBLISHED" => ProjectModelCacheStatus::ExtractedPublished,
        "EXTRACTED_NOT_PUBLISHED" => ProjectModelCacheStatus::ExtractedNotPublished,
        _ => return None,
    };
    let publish_outcome = match object.get("projectModelPublishOutcome")?.as_str()? {
        "NOT_ATTEMPTED" => ProjectModelPublishOutcome::NotAttempted,
        "PUBLISHED" => ProjectModelPublishOutcome::Published,
        "INVALID_MODEL" => ProjectModelPublishOutcome::InvalidModel,
        "ROOT_UNAVAILABLE" => ProjectModelPublishOutcome::RootUnavailable,
        "WRITE_FAILED" => ProjectModelPublishOutcome::WriteFailed,
        _ => return None,
    };
    let publish_invalid_reason = match object.get("projectModelPublishInvalidReason")?.as_str()? {
        "NOT_APPLICABLE" => ProjectModelInvalidReason::NotApplicable,
        "MISSING_SEMANTIC_INPUT_MANIFEST_HASH" => {
            ProjectModelInvalidReason::MissingSemanticInputManifestHash
        }
        "INVALID_SEMANTIC_INPUT_MANIFEST_HASH" => {
            ProjectModelInvalidReason::InvalidSemanticInputManifestHash
        }
        "SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH" => {
            ProjectModelInvalidReason::SemanticInputManifestHashMismatch
        }
        "MISSING_SEMANTIC_INPUT_MANIFEST" => {
            ProjectModelInvalidReason::MissingSemanticInputManifest
        }
        "MODEL_INPUTS_MANIFEST_MISMATCH" => ProjectModelInvalidReason::ModelInputsManifestMismatch,
        "JDK_FINGERPRINT_MANIFEST_MISMATCH" => {
            ProjectModelInvalidReason::JdkFingerprintManifestMismatch
        }
        "MODEL_INPUTS_INVALID" => ProjectModelInvalidReason::ModelInputsInvalid,
        "RESOURCE_IDENTITIES_INVALID" => ProjectModelInvalidReason::ResourceIdentitiesInvalid,
        "JDK_HOME_INVALID" => ProjectModelInvalidReason::JdkHomeInvalid,
        "JDK_HOME_MISMATCH" => ProjectModelInvalidReason::JdkHomeMismatch,
        "JDK_FINGERPRINT_MISSING" => ProjectModelInvalidReason::JdkFingerprintMissing,
        "JDK_FINGERPRINT_INVALID" => ProjectModelInvalidReason::JdkFingerprintInvalid,
        _ => return None,
    };
    let total_micros = object.get("projectModelTotalMicros")?.as_u64()?;
    let key_micros = object.get("projectModelKeyMicros")?.as_u64()?;
    let load_micros = object.get("projectModelLoadMicros")?.as_u64()?;
    let extraction_micros = object.get("projectModelExtractionMicros")?.as_u64()?;
    let publish_micros = object.get("projectModelPublishMicros")?.as_u64()?;
    let persistent_configured = object.get("projectModelPersistentConfigured")?.as_bool()?;
    let published = object.get("projectModelPublished")?.as_bool()?;
    let measured = key_micros
        .checked_add(load_micros)?
        .checked_add(extraction_micros)?
        .checked_add(publish_micros)?;
    if measured > total_micros {
        return None;
    }
    let consistent = match status {
        ProjectModelCacheStatus::MemoryHit => {
            publish_outcome == ProjectModelPublishOutcome::NotAttempted
                && publish_invalid_reason == ProjectModelInvalidReason::NotApplicable
                && load_micros == 0
                && extraction_micros == 0
                && publish_micros == 0
                && !published
        }
        ProjectModelCacheStatus::PersistentHit => {
            publish_outcome == ProjectModelPublishOutcome::NotAttempted
                && publish_invalid_reason == ProjectModelInvalidReason::NotApplicable
                && persistent_configured
                && extraction_micros == 0
                && publish_micros == 0
                && !published
        }
        ProjectModelCacheStatus::ExtractedPublished => {
            publish_outcome == ProjectModelPublishOutcome::Published
                && publish_invalid_reason == ProjectModelInvalidReason::NotApplicable
                && persistent_configured
                && published
        }
        ProjectModelCacheStatus::ExtractedNotPublished => {
            matches!(
                publish_outcome,
                ProjectModelPublishOutcome::InvalidModel
                    | ProjectModelPublishOutcome::RootUnavailable
                    | ProjectModelPublishOutcome::WriteFailed
            ) && (publish_outcome == ProjectModelPublishOutcome::InvalidModel
                || publish_invalid_reason == ProjectModelInvalidReason::NotApplicable)
                && (publish_outcome != ProjectModelPublishOutcome::InvalidModel
                    || publish_invalid_reason != ProjectModelInvalidReason::NotApplicable)
                && !published
        }
    };
    consistent.then_some(ProjectModelCacheProfile {
        status,
        publish_outcome,
        publish_invalid_reason,
        total_micros,
        key_micros,
        load_micros,
        extraction_micros,
        publish_micros,
        persistent_configured,
        published,
    })
}

fn retain_verified_index_project_model_profile(
    profile: &mut RequestProfile,
    open_project: Option<ProjectModelCacheProfile>,
) {
    if open_project.is_some() {
        profile.project_model_cache = open_project;
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_prefixed_lowercase_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(is_lowercase_sha256)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn parse_worker_code(code: &str) -> ErrorCode {
    match code {
        "UNSUPPORTED_KOTLIN_VERSION" => ErrorCode::UnsupportedKotlinVersion,
        "UNSUPPORTED_COMPILER_PLUGIN_ABI" => ErrorCode::UnsupportedCompilerPluginAbi,
        "UNSUPPORTED_PROJECT_CONFIGURATION" => ErrorCode::UnsupportedProjectConfiguration,
        "PROJECT_MODEL_CHANGED" => ErrorCode::ProjectModelChanged,
        "WORKER_PROTOCOL_MISMATCH" => ErrorCode::WorkerProtocolMismatch,
        "WORKER_PREPARATION_REQUIRED" => ErrorCode::WorkerPreparationRequired,
        "WORKER_CRASHED" => ErrorCode::WorkerCrashed,
        "SYMBOL_NOT_FOUND" => ErrorCode::SymbolNotFound,
        "AMBIGUOUS_SYMBOL" => ErrorCode::AmbiguousSymbol,
        "EXPRESSION_NOT_FOUND" => ErrorCode::ExpressionNotFound,
        "STALE_TARGET" => ErrorCode::StaleTarget,
        "AMBIGUOUS_TARGET" => ErrorCode::AmbiguousTarget,
        "PRECONDITION_FAILED" => ErrorCode::PreconditionFailed,
        "REPLACEMENT_PARSE_ERROR" => ErrorCode::ReplacementParseError,
        "UNSUPPORTED_CONTROL_FLOW" => ErrorCode::UnsupportedControlFlow,
        "INCOMPLETE_SEMANTIC_ANALYSIS" => ErrorCode::IncompleteSemanticAnalysis,
        "SLICE_BUDGET_EXCEEDED" => ErrorCode::SliceBudgetExceeded,
        "TYPE_MISMATCH" => ErrorCode::TypeMismatch,
        "BINDING_CHANGED" => ErrorCode::BindingChanged,
        "NEW_DIAGNOSTICS" => ErrorCode::NewDiagnostics,
        "EFFECT_CHANGED" => ErrorCode::EffectChanged,
        "WRITESET_EXCEEDED" => ErrorCode::WritesetExceeded,
        "COMPILE_FAILED" => ErrorCode::CompileFailed,
        "TEST_FAILED" => ErrorCode::TestFailed,
        "ABI_CHANGED" => ErrorCode::AbiChanged,
        "RW_CONFLICT" => ErrorCode::RwConflict,
        "WW_CONFLICT" => ErrorCode::WwConflict,
        "STALE_REQUIRES_RESLICE" => ErrorCode::StaleRequiresReslice,
        "REF_COMPARE_AND_SWAP_FAILED" => ErrorCode::RefCompareAndSwapFailed,
        "TRANSACTION_RECOVERY_REQUIRED" => ErrorCode::TransactionRecoveryRequired,
        "INVALID_INPUT" => ErrorCode::InvalidInput,
        "INTERNAL" => ErrorCode::Internal,
        _ => ErrorCode::WorkerProtocolMismatch,
    }
}

fn safe_worker_error_message(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnsupportedProjectConfiguration => {
            "project model extraction failed; verify the explicit compilation and run the project wrapper directly for local diagnostics"
        }
        ErrorCode::UnsupportedKotlinVersion => "the project Kotlin version is unsupported",
        ErrorCode::UnsupportedCompilerPluginAbi => {
            "the project compiler plugin ABI is unsupported by the selected worker"
        }
        ErrorCode::ProjectModelChanged => "the project model changed during semantic analysis",
        ErrorCode::WorkerPreparationRequired => "the sealed Kotlin worker is unavailable",
        ErrorCode::IncompleteSemanticAnalysis => {
            "the Kotlin worker could not prove complete semantic analysis"
        }
        ErrorCode::WorkerCrashed => {
            "the Kotlin worker terminated before producing a verified response"
        }
        _ => "the Kotlin worker rejected the request; inspect private run diagnostics",
    }
}

pub fn workspace_root() -> PathBuf {
    #[cfg(not(test))]
    {
        RuntimeAuthority::from_environment()
            .expect("runtime descriptor authority must be valid")
            .expect("runtime descriptor authority is required")
            .root
    }
    #[cfg(test)]
    {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .expect("git must resolve the test workspace root");
        assert!(
            output.status.success(),
            "git must resolve the test workspace root"
        );
        PathBuf::from(
            std::str::from_utf8(&output.stdout)
                .expect("test workspace root must be UTF-8")
                .trim(),
        )
        .canonicalize()
        .expect("test workspace root must be canonical")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::ffi::OsStr;
    use walkdir::WalkDir;

    #[cfg(unix)]
    #[test]
    fn worker_cancellation_terminates_the_owned_descendant_process_group() {
        use std::os::unix::process::CommandExt;

        let root = tempfile::tempdir().unwrap();
        let descendant_file = root.path().join("descendant.pid");
        let child = Command::new("sh")
            .args([
                "-c",
                "sleep 30 & descendant=$!; echo $descendant > \"$1\"; wait",
                "codeclew-worker-test",
            ])
            .arg(&descendant_file)
            .process_group(0)
            .spawn()
            .unwrap();
        let mut process = OwnedWorkerProcess::new(child, false).unwrap();
        for _ in 0..40 {
            if descendant_file.is_file() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let descendant = std::fs::read_to_string(&descendant_file)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        process.cancellation_handle().cancel().unwrap();
        assert!(process.child.wait().unwrap().code().is_none());

        for _ in 0..40 {
            let status = Command::new("ps")
                .args(["-o", "stat=", "-p", &descendant.to_string()])
                .output()
                .unwrap();
            if !status.status.success()
                || String::from_utf8_lossy(&status.stdout)
                    .trim_start()
                    .starts_with('Z')
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("worker descendant survived process-group cancellation");
    }

    #[cfg(unix)]
    #[test]
    fn worker_cancellation_refuses_a_changed_process_identity() {
        use std::os::unix::process::CommandExt;

        let mut child = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let error =
            terminate_worker_process_group(child.id() as i32, "not-the-start-token").unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);
        assert!(child.try_wait().unwrap().is_none());
        let start_token = worker_process_start_token(child.id()).unwrap().unwrap();
        terminate_worker_process_group(child.id() as i32, &start_token).unwrap();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn task_run_supervisor_cancels_separate_worker_group_and_descendant() {
        use std::os::unix::process::CommandExt;

        const CHILD_MODE: &str = "CODECLEW_PROCESS_TREE_TEST_CHILD";
        const PID_FILE: &str = "CODECLEW_PROCESS_TREE_TEST_PID_FILE";
        if std::env::var_os(CHILD_MODE).is_some() {
            install_task_run_process_tree_supervisor().unwrap();
            let pid_file = PathBuf::from(std::env::var_os(PID_FILE).unwrap());
            let permit = task_run_worker_spawn_permit().unwrap().unwrap();
            let child = Command::new("sh")
                .args([
                    "-c",
                    "sleep 30 & descendant=$!; printf '%s %s\\n' \"$$\" \"$descendant\" > \"$1\"; wait",
                    "codeclew-task-run-worker-test",
                ])
                .arg(&pid_file)
                .process_group(0)
                .spawn()
                .unwrap();
            // Keep the worker in the exact spawn-before-registration interval.
            // TERM must wait on the spawn gate; otherwise this separate PG
            // would escape when the supervisor exits.
            while !pid_file.is_file() {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(250));
            let _worker = OwnedWorkerProcess::new(child, true).unwrap();
            drop(permit);
            loop {
                std::thread::sleep(Duration::from_secs(30));
            }
        }

        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("worker-tree.pid");
        let mut unrelated = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let mut supervisor = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worker::tests::task_run_supervisor_cancels_separate_worker_group_and_descendant",
                "--nocapture",
            ])
            .env(CHILD_MODE, "1")
            .env(PID_FILE, &pid_file)
            .process_group(0)
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if pid_file.is_file() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let pids = std::fs::read_to_string(&pid_file).unwrap();
        let pids = pids
            .split_whitespace()
            .map(|value| value.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pids.len(), 2);

        assert_eq!(
            unsafe { libc::kill(-(supervisor.id() as i32), libc::SIGTERM) },
            0
        );
        for _ in 0..240 {
            if supervisor.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(supervisor.try_wait().unwrap().is_some());
        for pid in pids {
            let mut alive = true;
            for _ in 0..120 {
                let status = Command::new("ps")
                    .args(["-o", "stat=", "-p", &pid.to_string()])
                    .output()
                    .unwrap();
                if !status.status.success()
                    || String::from_utf8_lossy(&status.stdout)
                        .trim_start()
                        .starts_with('Z')
                {
                    alive = false;
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            assert!(!alive, "worker-tree process {pid} survived cancellation");
        }
        assert!(unrelated.try_wait().unwrap().is_none());
        let unrelated_start = worker_process_start_token(unrelated.id()).unwrap().unwrap();
        terminate_worker_process_group(unrelated.id() as i32, &unrelated_start).unwrap();
        let _ = unrelated.wait();
    }

    fn valid_compiler_index_profiling() -> Value {
        json!({
            "backend":"BTA_PERSISTENT",
            "status":"INCREMENTAL",
            "valid":true,
            "totalMicros":120,
            "compilerMicros":80,
            "firExtractionMicros":30,
            "totalFiles":5,
            "compiledFiles":2,
            "reusedFiles":3,
            "recovered":false,
            "fallbackUsed":false,
            "graphDigest":"a".repeat(64),
            "semanticInputManifestDigest":format!("sha256:{}", "b".repeat(64)),
            "factsPluginDigest":format!("sha256:{}", "c".repeat(64)),
            "extractorAuthorityDigest":format!("sha256:{}", "d".repeat(64)),
            "semanticConfigurationDigest":format!("sha256:{}", "e".repeat(64)),
            "workerProcessingMicros":140,
            "cacheRequests":1,
            "privatePath":"/must/not/escape",
        })
    }

    fn valid_project_model_cache_profiling() -> Value {
        json!({
            "projectModelCacheStatus":"PERSISTENT_HIT",
            "projectModelPublishOutcome":"NOT_ATTEMPTED",
            "projectModelPublishInvalidReason":"NOT_APPLICABLE",
            "projectModelTotalMicros":120,
            "projectModelKeyMicros":20,
            "projectModelLoadMicros":90,
            "projectModelExtractionMicros":0,
            "projectModelPublishMicros":0,
            "projectModelPersistentConfigured":true,
            "projectModelPublished":false,
            "privatePath":"/must/not/escape",
        })
    }

    #[test]
    fn project_model_cache_profile_is_typed_and_operational_only() {
        let profile = parse_project_model_cache_profile(&valid_project_model_cache_profiling())
            .expect("valid persistent hit");
        assert_eq!(profile.status, ProjectModelCacheStatus::PersistentHit);
        assert_eq!(
            profile.publish_outcome,
            ProjectModelPublishOutcome::NotAttempted
        );
        assert_eq!(
            profile.publish_invalid_reason,
            ProjectModelInvalidReason::NotApplicable
        );
        assert_eq!(profile.total_micros, 120);
        assert_eq!(profile.key_micros, 20);
        assert_eq!(profile.load_micros, 90);
        let transported = serde_json::to_value(profile).unwrap();
        assert_eq!(transported["status"], "PERSISTENT_HIT");
        assert_eq!(transported["publishInvalidReason"], "NOT_APPLICABLE");
        assert!(transported.get("privatePath").is_none());
    }

    #[test]
    fn project_model_cache_profile_rejects_unknown_partial_and_inconsistent_rows() {
        let mut unknown = valid_project_model_cache_profiling();
        unknown["projectModelCacheStatus"] = Value::String("NEW_STATUS".to_owned());
        assert!(parse_project_model_cache_profile(&unknown).is_none());

        let mut partial = valid_project_model_cache_profiling();
        partial
            .as_object_mut()
            .unwrap()
            .remove("projectModelLoadMicros");
        assert!(parse_project_model_cache_profile(&partial).is_none());

        let mut impossible = valid_project_model_cache_profiling();
        impossible["projectModelExtractionMicros"] = Value::from(1);
        assert!(parse_project_model_cache_profile(&impossible).is_none());

        let mut over_total = valid_project_model_cache_profiling();
        over_total["projectModelKeyMicros"] = Value::from(121);
        assert!(parse_project_model_cache_profile(&over_total).is_none());

        let mut published_hit = valid_project_model_cache_profiling();
        published_hit["projectModelPublished"] = Value::Bool(true);
        assert!(parse_project_model_cache_profile(&published_hit).is_none());

        let mut inconsistent_outcome = valid_project_model_cache_profiling();
        inconsistent_outcome["projectModelPublishOutcome"] = Value::String("WRITE_FAILED".into());
        assert!(parse_project_model_cache_profile(&inconsistent_outcome).is_none());

        let mut unknown_reason = valid_project_model_cache_profiling();
        unknown_reason["projectModelPublishInvalidReason"] = Value::String("NEW_REASON".into());
        assert!(parse_project_model_cache_profile(&unknown_reason).is_none());
    }

    #[test]
    fn project_model_cache_profile_accepts_each_consistent_status() {
        for (status, outcome, persistent, published, load, extraction, publish) in [
            ("MEMORY_HIT", "NOT_ATTEMPTED", false, false, 0, 0, 0),
            ("PERSISTENT_HIT", "NOT_ATTEMPTED", true, false, 10, 0, 0),
            ("EXTRACTED_PUBLISHED", "PUBLISHED", true, true, 10, 30, 20),
            (
                "EXTRACTED_NOT_PUBLISHED",
                "INVALID_MODEL",
                true,
                false,
                10,
                30,
                20,
            ),
            (
                "EXTRACTED_NOT_PUBLISHED",
                "ROOT_UNAVAILABLE",
                false,
                false,
                10,
                30,
                20,
            ),
            (
                "EXTRACTED_NOT_PUBLISHED",
                "WRITE_FAILED",
                true,
                false,
                10,
                30,
                20,
            ),
        ] {
            let mut value = valid_project_model_cache_profiling();
            value["projectModelCacheStatus"] = Value::String(status.to_owned());
            value["projectModelPublishOutcome"] = Value::String(outcome.to_owned());
            value["projectModelPublishInvalidReason"] = Value::String(
                if outcome == "INVALID_MODEL" {
                    "SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH"
                } else {
                    "NOT_APPLICABLE"
                }
                .to_owned(),
            );
            value["projectModelPersistentConfigured"] = Value::Bool(persistent);
            value["projectModelPublished"] = Value::Bool(published);
            value["projectModelLoadMicros"] = Value::from(load);
            value["projectModelExtractionMicros"] = Value::from(extraction);
            value["projectModelPublishMicros"] = Value::from(publish);
            assert!(
                parse_project_model_cache_profile(&value).is_some(),
                "valid {status} profile was rejected"
            );
        }
    }

    #[test]
    fn verified_index_retains_open_project_cache_observation_over_memory_hit() {
        let persistent = parse_project_model_cache_profile(&valid_project_model_cache_profiling())
            .expect("persistent profile");
        let mut memory_value = valid_project_model_cache_profiling();
        memory_value["projectModelCacheStatus"] = Value::String("MEMORY_HIT".to_owned());
        memory_value["projectModelLoadMicros"] = Value::from(0);
        memory_value["projectModelPersistentConfigured"] = Value::Bool(false);
        let memory = parse_project_model_cache_profile(&memory_value).expect("memory profile");
        let mut request = RequestProfile {
            project_model_cache: Some(memory.clone()),
            ..RequestProfile::default()
        };

        retain_verified_index_project_model_profile(&mut request, Some(persistent.clone()));
        assert_eq!(request.project_model_cache, Some(persistent));

        request.project_model_cache = Some(memory.clone());
        retain_verified_index_project_model_profile(&mut request, None);
        assert_eq!(request.project_model_cache, Some(memory));
    }

    #[test]
    fn compiler_index_profile_is_typed_and_profiling_is_removed_from_body() {
        let mut body = json!({
            "indexHash":"semantic-index",
            "facts":[],
            "profiling":valid_compiler_index_profiling(),
        });

        let profiling = take_worker_profiling(&mut body).unwrap();
        let profile = parse_compiler_index_profile(&profiling).unwrap();

        assert_eq!(body, json!({"indexHash":"semantic-index","facts":[]}));
        assert_eq!(profile.backend, CompilerIndexBackend::BtaPersistent);
        assert_eq!(profile.status, CompilerIndexStatus::Incremental);
        assert!(profile.valid);
        assert_eq!(profile.compiled_files, 2);
        assert_eq!(profile.reused_files, 3);
        assert_eq!(
            profile.graph_digest.as_deref(),
            Some("a".repeat(64).as_str())
        );
        let transported = serde_json::to_value(profile).unwrap();
        assert_eq!(transported["backend"], "BTA_PERSISTENT");
        assert_eq!(transported["status"], "INCREMENTAL");
        assert_eq!(transported["valid"], true);
        assert_eq!(
            transported["semanticConfigurationDigest"],
            format!("sha256:{}", "e".repeat(64))
        );
        assert!(transported.get("privatePath").is_none());
    }

    #[test]
    fn unknown_or_partial_compiler_index_profile_is_ignored() {
        let mut unknown = valid_compiler_index_profiling();
        unknown["backend"] = Value::String("OTHER_BACKEND".to_owned());
        let mut body = json!({"semantic":"preserved","profiling":unknown});
        let profiling = take_worker_profiling(&mut body).unwrap();
        assert!(parse_compiler_index_profile(&profiling).is_none());
        assert_eq!(body, json!({"semantic":"preserved"}));

        let mut partial = valid_compiler_index_profiling();
        partial.as_object_mut().unwrap().remove("valid");
        assert!(parse_compiler_index_profile(&partial).is_none());

        let mut unknown_status = valid_compiler_index_profiling();
        unknown_status["status"] = Value::String("NEW_STATUS".to_owned());
        assert!(parse_compiler_index_profile(&unknown_status).is_none());
    }

    #[test]
    fn malformed_compiler_index_profile_is_ignored() {
        let mut negative = valid_compiler_index_profiling();
        negative["totalFiles"] = Value::from(-1);
        assert!(parse_compiler_index_profile(&negative).is_none());

        let mut inconsistent = valid_compiler_index_profiling();
        inconsistent["reusedFiles"] = Value::from(4);
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        let mut uppercase_digest = valid_compiler_index_profiling();
        uppercase_digest["graphDigest"] = Value::String("A".repeat(64));
        assert!(parse_compiler_index_profile(&uppercase_digest).is_none());

        let mut null_digest = valid_compiler_index_profiling();
        null_digest["graphDigest"] = Value::Null;
        let normalized = parse_compiler_index_profile(&null_digest).unwrap();
        assert!(normalized.graph_digest.is_none());
        assert!(
            serde_json::to_value(normalized)
                .unwrap()
                .get("graphDigest")
                .is_none()
        );

        let mut wrong_digest_type = valid_compiler_index_profiling();
        wrong_digest_type["graphDigest"] = Value::from(7);
        assert!(parse_compiler_index_profile(&wrong_digest_type).is_none());

        let mut failed = valid_compiler_index_profiling();
        failed["status"] = Value::String("FAILED_RECOVERABLE".into());
        failed["valid"] = Value::Bool(false);
        failed["compiledFiles"] = Value::from(0);
        failed["reusedFiles"] = Value::from(0);
        failed["fallbackUsed"] = Value::Bool(true);
        failed["graphDigest"] = Value::Null;
        failed["failureCode"] = Value::String("K2_BACKEND_ANALYZE_EXCEPTION".into());
        assert_eq!(
            parse_compiler_index_profile(&failed)
                .unwrap()
                .failure_code
                .as_deref(),
            Some("K2_BACKEND_ANALYZE_EXCEPTION"),
        );
        failed["failureCode"] = Value::String("private/path".into());
        assert!(parse_compiler_index_profile(&failed).is_none());

        for key in [
            "semanticInputManifestDigest",
            "factsPluginDigest",
            "extractorAuthorityDigest",
            "semanticConfigurationDigest",
        ] {
            let mut missing = valid_compiler_index_profiling();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                parse_compiler_index_profile(&missing).is_none(),
                "missing {key} was accepted"
            );

            let mut malformed = valid_compiler_index_profiling();
            malformed[key] = Value::String(format!("sha256:{}", "A".repeat(64)));
            assert!(
                parse_compiler_index_profile(&malformed).is_none(),
                "malformed {key} was accepted"
            );
        }
    }

    #[test]
    fn valid_compiler_index_statuses_require_complete_counts_and_no_fallback() {
        for (status, compiled, reused, recovered) in [
            ("UNCHANGED_HIT", 0, 5, false),
            ("COLD_FULL", 5, 0, false),
            ("INCREMENTAL", 2, 3, false),
            ("RECOVERED_FULL", 5, 0, true),
        ] {
            let mut value = valid_compiler_index_profiling();
            value["status"] = Value::String(status.to_owned());
            value["compiledFiles"] = Value::from(compiled);
            value["reusedFiles"] = Value::from(reused);
            value["recovered"] = Value::Bool(recovered);
            assert!(
                parse_compiler_index_profile(&value).is_some(),
                "valid {status} profile was rejected"
            );
        }

        let mut invalid_hit = valid_compiler_index_profiling();
        invalid_hit["status"] = Value::String("UNCHANGED_HIT".to_owned());
        invalid_hit["valid"] = Value::Bool(false);
        invalid_hit["reusedFiles"] = Value::from(0);
        invalid_hit["graphDigest"] = Value::Null;
        assert!(parse_compiler_index_profile(&invalid_hit).is_none());

        let mut fallback_success = valid_compiler_index_profiling();
        fallback_success["fallbackUsed"] = Value::Bool(true);
        assert!(parse_compiler_index_profile(&fallback_success).is_none());
    }

    #[test]
    fn attempted_compiler_failures_retain_status_without_claiming_facts() {
        for (status, recovered) in [
            ("COLD_FULL", false),
            ("INCREMENTAL", false),
            ("RECOVERED_FULL", true),
        ] {
            let mut value = valid_compiler_index_profiling();
            value["status"] = Value::String(status.to_owned());
            value["valid"] = Value::Bool(false);
            value["compiledFiles"] = Value::from(0);
            value["reusedFiles"] = Value::from(0);
            value["recovered"] = Value::Bool(recovered);
            value["graphDigest"] = Value::Null;
            let profile = parse_compiler_index_profile(&value)
                .unwrap_or_else(|| panic!("honest failed {status} attempt was rejected"));
            assert!(!profile.valid);
            assert!(profile.graph_digest.is_none());
        }

        let mut invalid_with_graph = valid_compiler_index_profiling();
        invalid_with_graph["valid"] = Value::Bool(false);
        invalid_with_graph["compiledFiles"] = Value::from(0);
        invalid_with_graph["reusedFiles"] = Value::from(0);
        assert!(parse_compiler_index_profile(&invalid_with_graph).is_none());

        invalid_with_graph["graphDigest"] = Value::Null;
        invalid_with_graph["compiledFiles"] = Value::from(1);
        assert!(parse_compiler_index_profile(&invalid_with_graph).is_none());

        invalid_with_graph["compiledFiles"] = Value::from(0);
        invalid_with_graph["fallbackUsed"] = Value::Bool(true);
        assert!(parse_compiler_index_profile(&invalid_with_graph).is_none());
    }

    #[test]
    fn infrastructure_failures_are_invalid_fallbacks_with_no_fact_counts() {
        for (status, total, recovered) in [
            ("BUSY", 0, false),
            ("BUSY", 0, true),
            ("FAILED_RECOVERABLE", 0, false),
            ("FAILED_RECOVERABLE", 5, false),
            ("FAILED_RECOVERABLE", 5, true),
        ] {
            let mut value = valid_compiler_index_profiling();
            value["status"] = Value::String(status.to_owned());
            value["valid"] = Value::Bool(false);
            value["totalFiles"] = Value::from(total);
            value["compiledFiles"] = Value::from(0);
            value["reusedFiles"] = Value::from(0);
            value["recovered"] = Value::Bool(recovered);
            value["fallbackUsed"] = Value::Bool(true);
            value["graphDigest"] = Value::Null;
            assert!(
                parse_compiler_index_profile(&value).is_some(),
                "honest {status} profile with recovered={recovered} was rejected"
            );
        }

        let mut inconsistent = valid_compiler_index_profiling();
        inconsistent["status"] = Value::String("FAILED_RECOVERABLE".to_owned());
        inconsistent["valid"] = Value::Bool(false);
        inconsistent["totalFiles"] = Value::from(0);
        inconsistent["compiledFiles"] = Value::from(0);
        inconsistent["reusedFiles"] = Value::from(0);
        inconsistent["graphDigest"] = Value::Null;
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        inconsistent["fallbackUsed"] = Value::Bool(true);
        inconsistent["valid"] = Value::Bool(true);
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        inconsistent["valid"] = Value::Bool(false);
        inconsistent["reusedFiles"] = Value::from(1);
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        inconsistent["reusedFiles"] = Value::from(0);
        inconsistent["totalFiles"] = Value::from(1);
        assert!(parse_compiler_index_profile(&inconsistent).is_some());

        inconsistent["graphDigest"] = Value::String("a".repeat(64));
        assert!(parse_compiler_index_profile(&inconsistent).is_none());
    }

    #[test]
    fn compiler_index_root_is_private_canonical_and_external() {
        let workspace = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let state_path = state.path().canonicalize().unwrap();
        assert_eq!(
            validate_compiler_index_root(workspace.path(), &state_path).unwrap(),
            state_path
        );

        let nested = workspace.path().join("compiler-index");
        std::fs::create_dir(&nested).unwrap();
        assert_eq!(
            validate_compiler_index_root(workspace.path(), &nested)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let public = tempfile::tempdir().unwrap();
            std::fs::set_permissions(public.path(), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            assert_eq!(
                validate_compiler_index_root(workspace.path(), public.path())
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
            let link = state_path.parent().unwrap().join("compiler-index-link");
            let _ = std::fs::remove_file(&link);
            symlink(&state_path, &link).unwrap();
            assert_eq!(
                validate_compiler_index_root(workspace.path(), &link)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
            std::fs::remove_file(link).unwrap();
        }
    }

    fn configured_command_environment(command: &Command, key: &str) -> Option<Option<PathBuf>> {
        command
            .get_envs()
            .find_map(|(name, value)| (name == OsStr::new(key)).then(|| value.map(PathBuf::from)))
    }

    #[test]
    fn worker_state_environment_scrubs_ambient_authority_when_disabled() {
        let namespace = format!("sha256:{}", "a".repeat(64));
        let mut command = Command::new("not-executed");
        command
            .env("CODECLEW_K1_BUILD_STATE_ROOT", "ambient-k1")
            .env("CODECLEW_K2_INDEX_ROOT", "ambient-k2")
            .env("CODECLEW_K1_BUILD_STATE_NAMESPACE", "ambient-namespace");

        configure_worker_state_environment(&mut command, None, None, &namespace);

        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K1_BUILD_STATE_ROOT"),
            Some(None)
        );
        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K2_INDEX_ROOT"),
            Some(None)
        );
        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K1_BUILD_STATE_NAMESPACE"),
            Some(Some(PathBuf::from(namespace)))
        );
    }

    #[test]
    fn worker_state_environment_passes_only_explicit_canonical_roots() {
        let namespace = format!("sha256:{}", "b".repeat(64));
        let build_state = tempfile::tempdir().unwrap();
        let compiler_index = tempfile::tempdir().unwrap();
        let canonical_build_state = build_state.path().canonicalize().unwrap();
        let canonical_compiler_index = compiler_index.path().canonicalize().unwrap();
        let mut command = Command::new("not-executed");
        command
            .env("CODECLEW_K1_BUILD_STATE_ROOT", "ambient-k1")
            .env("CODECLEW_K2_INDEX_ROOT", "ambient-k2");

        configure_worker_state_environment(
            &mut command,
            Some(&canonical_build_state),
            Some(&canonical_compiler_index),
            &namespace,
        );

        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K1_BUILD_STATE_ROOT"),
            Some(Some(canonical_build_state))
        );
        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K2_INDEX_ROOT"),
            Some(Some(canonical_compiler_index))
        );
        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K1_BUILD_STATE_NAMESPACE"),
            Some(Some(PathBuf::from(namespace)))
        );
    }

    fn source_syntax_fixture() -> (tempfile::TempDir, String, Value) {
        let repo = tempfile::tempdir().unwrap();
        let relative = "src/main/kotlin/p/Value.kt".to_owned();
        let path = repo.path().join(&relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = "package p\nfun value() = 1\n";
        std::fs::write(&path, source).unwrap();
        let start = source.find("fun").unwrap() as u64;
        let end = source.trim_end().encode_utf16().count() as u64;
        let declaration = json!({
            "declarationId":"declaration:syntax-value",
            "symbolId":"p.value()",
            "name":"value",
            "kind":"KtNamedFunction",
            "file":relative,
            "rangeStart":start,
            "rangeEnd":end,
            "sourceOrigin":{"file":relative,"rangeStart":start,"rangeEnd":end},
        });
        let files = json!([{
            "path":relative,
            "normalizedRelativePath":relative,
            "contentHash":crate::canonical::hash_bytes(source.as_bytes()),
            "declarations":[declaration],
            "semanticFacts":[],
        }]);
        let response = json!({
            "schema":"semantic-index/0.1",
            "compilation":":/main",
            "partial":true,
            "analysisMode":"SYNTAX_DECLARATIONS",
            "files":files,
            "indexHash":"sha256:transport-bound",
            "projectModelHash":"SOURCE_SYNTAX",
            "k2Validated":false,
            "diagnostics":[],
        });
        (repo, relative, response)
    }

    #[test]
    fn source_syntax_accepts_current_declaration_only_response() {
        let (repo, relative, response) = source_syntax_fixture();
        let manifest = validate_source_syntax_response(
            repo.path(),
            ":/main",
            std::slice::from_ref(&relative),
            &response,
        )
        .unwrap();
        assert!(manifest.starts_with("sha256:"));

        let mut partial = response.clone();
        partial["partial"] = Value::Bool(false);
        assert!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &partial,
            )
            .is_err()
        );
        assert!(validate_source_syntax_response(repo.path(), ":/main", &[], &response).is_err());

        let mut empty_graphs = response.clone();
        empty_graphs["declarationRelations"] = json!([]);
        empty_graphs["declarationRelationHash"] =
            Value::String(crate::canonical::hash(&json!([])).unwrap());
        empty_graphs["declarationDescriptors"] = json!([]);
        empty_graphs["declarationDescriptorHash"] =
            Value::String(crate::canonical::hash(&json!([])).unwrap());
        assert!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &empty_graphs,
            )
            .is_ok()
        );
    }

    #[test]
    fn source_syntax_allows_provisional_id_collisions_but_not_duplicate_occurrences() {
        let (repo, relative, response) = source_syntax_fixture();
        let mut colliding_ids = response.clone();
        let mut second = colliding_ids["files"][0]["declarations"][0].clone();
        second["name"] = json!("packageOccurrence");
        second["rangeStart"] = json!(0);
        second["rangeEnd"] = json!(7);
        second["sourceOrigin"] = json!({"file":relative,"rangeStart":0,"rangeEnd":7});
        colliding_ids["files"][0]["declarations"]
            .as_array_mut()
            .unwrap()
            .insert(0, second);
        assert!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &colliding_ids,
            )
            .is_ok()
        );

        let mut duplicate_occurrence = response;
        let mut duplicate = duplicate_occurrence["files"][0]["declarations"][0].clone();
        duplicate["declarationId"] = json!("provisional:distinct-declaration");
        duplicate["symbolId"] = json!("provisional:distinct-symbol");
        duplicate_occurrence["files"][0]["declarations"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &duplicate_occurrence,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn source_syntax_rejects_semantic_row_forgery() {
        let (repo, relative, response) = source_syntax_fixture();
        let mut forged_fact = response.clone();
        forged_fact["files"][0]["semanticFacts"] = json!([{"kind":"CALL"}]);
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_fact,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut forged_override = response.clone();
        forged_override["files"][0]["overrides"] = json!([{"symbolId":"p.value()"}]);
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_override,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut forged_relation = response.clone();
        forged_relation["declarationRelations"] = json!({
            "relations":[{"kind":"CALLS"}],
            "boundaries":[],
        });
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_relation,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut forged_descriptor = response;
        forged_descriptor["declarationDescriptors"] = json!({
            "descriptors":[{"symbolId":"p.value()"}],
            "boundaries":[],
        });
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_descriptor,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn source_syntax_worker_indexes_without_a_build_model() {
        let workspace = workspace_root();
        let repo = tempfile::tempdir().unwrap();
        let relative = "src/main/kotlin/p/Value.kt";
        let source = repo.path().join(relative);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "package p\n// 😀\nfun value() = 1\n").unwrap();
        assert!(!repo.path().join("build.gradle.kts").exists());
        assert!(!repo.path().join("pom.xml").exists());

        let mut worker = WorkerClient::start(&workspace).unwrap();
        let verified = worker
            .index_files_source_syntax_verified(&json!({
                "repo":repo.path(),
                "compilation":":/main",
                "syntaxOnly":true,
                "files":[relative],
            }))
            .unwrap();
        let facts = worker.inspect_verified_source_syntax(&verified).unwrap();
        assert_eq!(facts["analysisMode"], "SYNTAX_DECLARATIONS");
        assert_eq!(facts["k2Validated"], false);
        assert_eq!(facts["partial"], true);
        assert_eq!(facts["files"].as_array().unwrap().len(), 1);
        assert!(
            facts["files"][0]["declarations"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        );
        assert_eq!(facts["declarationRelations"], json!([]));
        assert_eq!(facts["declarationDescriptors"], json!([]));
        std::fs::write(&source, "package p\n// changed\nfun value() = 2\n").unwrap();
        assert_eq!(
            worker
                .inspect_verified_source_syntax(&verified)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
        worker.shutdown().unwrap();
    }

    #[test]
    fn descriptor_line_proof_is_recomputed_from_bound_source_bytes() {
        let repo = tempfile::tempdir().unwrap();
        let source = "// 😀\r\nfun answer() =\r\n    42\r\n";
        std::fs::write(repo.path().join("A.kt"), source).unwrap();
        let start = source.find("fun").unwrap();
        let end = source.len();
        let mut facts = json!({
            "files":[{
                "path":"A.kt",
                "contentHash":crate::canonical::hash_bytes(source.as_bytes()),
            }],
            "declarationDescriptors":{
                "descriptors":[{
                    "file":"A.kt",
                    "start":start,
                    "end":end,
                    "startLine":2,
                    "endLine":3,
                    "lineProvenance":"UTF8_BYTE_RANGE_OVER_COMPILATION_SOURCE",
                }]
            }
        });

        verify_descriptor_line_coordinates(repo.path(), &facts).unwrap();

        facts["declarationDescriptors"]["descriptors"][0]["endLine"] = json!(4);
        assert_eq!(
            verify_descriptor_line_coordinates(repo.path(), &facts)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        facts["declarationDescriptors"]["descriptors"][0]["endLine"] = json!(3);
        facts["declarationDescriptors"]["descriptors"][0]["end"] = json!(start);
        assert_eq!(
            verify_descriptor_line_coordinates(repo.path(), &facts)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        facts["declarationDescriptors"]["descriptors"][0]["end"] = json!(end);
        std::fs::write(repo.path().join("A.kt"), "fun changed() = 0\n").unwrap();
        assert_eq!(
            verify_descriptor_line_coordinates(repo.path(), &facts)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
    }

    #[test]
    fn source_syntax_rejects_k2_authority_forgery() {
        let (repo, relative, mut response) = source_syntax_fixture();
        response["k2Validated"] = Value::Bool(true);
        assert_eq!(
            validate_source_syntax_response(repo.path(), ":/main", &[relative], &response)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let (repo, relative, mut response) = source_syntax_fixture();
        response["localCfgs"] = json!([{"schema":"local-cfg/0.1"}]);
        response["localCfgBoundaries"] = json!([]);
        response["localCfgHash"] = json!(format!("sha256:{}", "0".repeat(64)));
        assert_eq!(
            validate_source_syntax_response(repo.path(), ":/main", &[relative], &response)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn compiler_plugin_abi_failure_is_not_collapsed_into_generic_k2_failure() {
        assert_eq!(
            parse_worker_code("UNSUPPORTED_COMPILER_PLUGIN_ABI"),
            ErrorCode::UnsupportedCompilerPluginAbi,
        );
        assert_eq!(
            parse_worker_code("INCOMPLETE_SEMANTIC_ANALYSIS"),
            ErrorCode::IncompleteSemanticAnalysis,
        );
        assert_eq!(parse_worker_code("INVALID_INPUT"), ErrorCode::InvalidInput);
        assert_eq!(
            parse_worker_code("FUTURE_OR_MISSPELLED_CODE"),
            ErrorCode::WorkerProtocolMismatch,
        );
        let message = safe_worker_error_message(&ErrorCode::UnsupportedProjectConfiguration);
        assert!(!message.contains('/'));
        assert!(!message.contains("HOME"));
        assert!(message.contains("explicit compilation"));
    }

    #[test]
    fn sealed_worker_process_has_a_fixed_jvm_memory_authority() {
        let mut command = Command::new("not-executed");
        configure_sealed_worker_process(&mut command, Path::new("/private/transport"));
        let environment = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            environment.get("KOTLIN_OPTS").and_then(Option::as_deref),
            Some(SEALED_WORKER_JVM_OPTIONS),
        );
        assert_eq!(
            environment
                .get("CODECLEW_WORKER_TRANSPORT_ROOT")
                .and_then(Option::as_deref),
            Some("/private/transport"),
        );
        assert!(!environment.contains_key("JAVA_OPTS"));
        assert!(SEALED_WORKER_JVM_OPTIONS.contains("-XX:+ExitOnOutOfMemoryError"));
    }

    #[test]
    fn compiler_plugin_abi_discovery_visits_each_packaged_engine_once() {
        let mut tried = 0;
        let first = KotlinEngineRegistry::next_untried_for_discovery(tried).unwrap();
        assert_eq!(first, KotlinSemanticEngine::Kotlin24);
        tried |= first.discovery_bit();

        let second = KotlinEngineRegistry::next_untried_for_discovery(tried).unwrap();
        assert_eq!(second, KotlinSemanticEngine::Kotlin23);
        tried |= second.discovery_bit();

        assert!(KotlinEngineRegistry::next_untried_for_discovery(tried).is_none());
    }

    #[test]
    fn kotlin_23_discovery_switches_once_and_preserves_logical_request_count() {
        let _workspace_worker_guard = workspace_worker_test_lock();
        let workspace = workspace_root();
        let source = workspace.join("fixtures/kotlin-basic");
        let temporary = tempfile::Builder::new()
            .prefix("kotlin23-discovery-")
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
        let build_file = temporary.path().join("build.gradle.kts");
        let build = std::fs::read_to_string(&build_file)
            .unwrap()
            .replace("2.4.10", "2.3.0");
        std::fs::write(build_file, build).unwrap();

        let mut worker = WorkerClient::start(&workspace).unwrap();
        let project = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":temporary.path(),"compilation":":/main"}),
            )
            .unwrap();
        assert_eq!(project["declaredCompilerVersion"], "2.3.0");
        assert_eq!(project["compilerVersion"], "2.3.0");
        assert_eq!(worker.engine, KotlinSemanticEngine::Kotlin23);
        assert_eq!(worker.capabilities.compiler_version, "2.3.0");
        assert_eq!(
            worker.request_counters(),
            WorkerRequestCounters {
                open_project_requests: 1,
                index_files_requests: 0,
            }
        );
        worker.shutdown().unwrap();
    }

    #[test]
    fn plugin_discovery_never_selects_an_absent_optional_engine() {
        let core = KotlinSemanticEngine::Kotlin24;
        let optional = KotlinSemanticEngine::Kotlin23;
        assert_eq!(next_available_discovery_engine(0, &[core]), Some(core));
        assert_eq!(
            next_available_discovery_engine(core.discovery_bit(), &[core]),
            None
        );
        assert_eq!(
            next_available_discovery_engine(core.discovery_bit(), &[core, optional]),
            Some(optional)
        );
        assert_eq!(
            next_available_discovery_engine(
                core.discovery_bit() | optional.discovery_bit(),
                &[core, optional]
            ),
            None
        );
        assert_eq!(next_available_discovery_engine(0, &[]), None);
    }

    #[test]
    fn default_project_engine_uses_core_when_optional_kotlin23_is_absent() {
        let mut model = serde_json::json!({
            "declaredCompilerVersion":"2.3.0", "languageVersion":"2.3", "apiVersion":"2.3",
            "jvmTarget":"17", "requestedCompilerPlugins":[], "freeCompilerArguments":[]
        });
        let project = KotlinProjectSemantics::from_project_model(&model).unwrap();
        assert_eq!(
            select_available_project_engine(&project, &[KotlinSemanticEngine::Kotlin24]).unwrap(),
            KotlinSemanticEngine::Kotlin24
        );
        assert_eq!(
            select_available_project_engine(
                &project,
                &[
                    KotlinSemanticEngine::Kotlin23,
                    KotlinSemanticEngine::Kotlin24
                ]
            )
            .unwrap(),
            KotlinSemanticEngine::Kotlin23
        );
        assert_eq!(
            select_available_project_engine(&project, &[]).unwrap(),
            KotlinSemanticEngine::Kotlin23
        );
        assert_eq!(
            KotlinEngineRegistry
                .qualify(&project, KotlinSemanticEngine::Kotlin23)
                .unwrap(),
            KotlinSemanticEngine::Kotlin23
        );
        model["freeCompilerArguments"] = serde_json::json!(["-Xcontext-parameters"]);
        let project = KotlinProjectSemantics::from_project_model(&model).unwrap();
        assert!(
            select_available_project_engine(&project, &[KotlinSemanticEngine::Kotlin24]).is_err()
        );
    }

    #[test]
    fn project_semantics_route_through_qualified_engine_registry() {
        for (version, expected) in [
            ("2.3.0", KotlinSemanticEngine::Kotlin23),
            ("2.4.0", KotlinSemanticEngine::Kotlin24),
            ("2.4.10", KotlinSemanticEngine::Kotlin24),
        ] {
            let project = KotlinProjectSemantics::from_project_model(&serde_json::json!({
                "declaredCompilerVersion":version,
                "languageVersion":version.split('.').take(2).collect::<Vec<_>>().join("."),
                "apiVersion":version.split('.').take(2).collect::<Vec<_>>().join("."),
                "jvmTarget":"21",
                "requestedCompilerPlugins":[],
                "freeCompilerArguments":[],
            }))
            .unwrap();
            assert_eq!(KotlinEngineRegistry.select(&project).unwrap(), expected);
        }
        for version in ["2.1.21", "1.9.25", "1.8.22", "2.5.0"] {
            let project = KotlinProjectSemantics::from_project_model(&serde_json::json!({
                "declaredCompilerVersion":version,
                "languageVersion":"2.3",
                "apiVersion":"2.3",
                "jvmTarget":"21",
                "requestedCompilerPlugins":[],
                "freeCompilerArguments":[],
            }))
            .unwrap();
            let error = KotlinEngineRegistry.select(&project).unwrap_err();
            assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
            assert!(error.message.contains("qualified semantic engine"));
        }
    }

    fn relation_normalization_facts(relations: Vec<Value>) -> Value {
        let mut relations = relations;
        relations.sort_by_key(|row| crate::canonical::bytes(row).unwrap());
        let graph = json!({
            "schema":"declaration-relation-graph/0.1",
            "compilation":":/main",
            "coverage":"COMPLETE_SUPPORTED_SUBSET",
            "relations":relations,
            "boundaries":[],
            "provenance":{},
        });
        json!({
            "declarationRelationHash":crate::canonical::hash(&graph).unwrap(),
            "declarationRelations":graph,
        })
    }

    fn descriptor_normalization_facts(jvm_descriptor: &str) -> Value {
        let descriptor = json!({
            "schema":"declaration-descriptor/0.1",
            "file":"A.kt","start":0,"end":12,
            "symbolIdentity":format!("constructor:p/Box.<init>#jvm:{jvm_descriptor}"),
            "declarationKind":"CONSTRUCTOR","ownerIdentity":"class:p/Box",
            "containment":["class:p/Box"],"visibility":"public",
            "effectiveVisibility":"public","exportBoundary":"PUBLIC_API",
            "modality":"FINAL","compilerCallableId":"p/Box.<init>",
            "compilerClassId":"p/Box","isPrimary":true,
            "jvmDescriptor":jvm_descriptor,
            "parameterTypes":[{"index":0,"type":"kotlin/String","nullable":false}],
            "typeParameters":[],"module":":","sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "resolution":"PROVEN","provider":"K2_FIR"
        });
        let graph = json!({
            "schema":"declaration-descriptor-graph/0.1",
            "compilation":":/main",
            "coverage":"COMPLETE_SUPPORTED_SUBSET",
            "descriptors":[descriptor],
            "boundaries":[],
            "provenance":{},
        });
        json!({
            "declarationDescriptorHash":crate::canonical::hash(&graph).unwrap(),
            "declarationDescriptors":graph,
        })
    }

    fn function_descriptor_normalization_facts(jvm_descriptor: &str) -> Value {
        let descriptor = json!({
            "schema":"declaration-descriptor/0.1",
            "file":"A.kt","start":0,"end":12,
            "symbolIdentity":format!("callable:p/read#jvm:{jvm_descriptor}"),
            "declarationKind":"FUNCTION","ownerIdentity":"package:p",
            "containment":[],"visibility":"public",
            "effectiveVisibility":"public","exportBoundary":"PUBLIC_API",
            "modality":"FINAL","compilerCallableId":"p/read","isOverride":false,
            "jvmDescriptor":jvm_descriptor,
            "returnType":"kotlin/Unit","returnNullable":false,
            "parameterTypes":[{"index":0,"type":"p/Outer.Inner","nullable":false}],
            "typeParameters":[],"module":":","sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "resolution":"PROVEN","provider":"K2_FIR"
        });
        let graph = json!({
            "schema":"declaration-descriptor-graph/0.1",
            "compilation":":/main",
            "coverage":"COMPLETE_SUPPORTED_SUBSET",
            "descriptors":[descriptor],
            "boundaries":[],
            "provenance":{},
        });
        json!({
            "declarationDescriptorHash":crate::canonical::hash(&graph).unwrap(),
            "declarationDescriptors":graph,
        })
    }

    fn local_cfg_graph(owner: &str) -> Value {
        let mut graph = json!({
            "schema":"local-cfg/0.1",
            "graphId":"",
            "ownerSymbolIdentity":owner,
            "file":"src/main/kotlin/p/Box.kt",
            "compilerGraphName":"Box.save",
            "provider":"K2_FIR_CFG",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "nodes":[
                {"nodeId":2,"role":"DECISION"},
                {"nodeId":7,"role":"RETURN"},
                {"nodeId":9,"role":"ENTRY"}
            ],
            "edges":[
                {"sourceNodeId":2,"targetNodeId":7,"kind":"TRUE","label":"CFG_TRUE"},
                {"sourceNodeId":9,"targetNodeId":2,"kind":"NEXT"}
            ]
        });
        graph["graphId"] = json!(crate::canonical::hash(&graph).unwrap());
        let payload: crate::thread_flow_cfg::LocalCfgPayload =
            serde_json::from_value(graph.clone()).unwrap();
        crate::thread_flow_cfg::validate(&payload).unwrap();
        graph
    }

    fn local_cfg_facts(graphs: Vec<Value>, boundaries: Vec<Value>) -> Value {
        let hash = local_cfg_snapshot_hash(&graphs, &boundaries).unwrap();
        json!({
            "localCfgs":graphs,
            "localCfgBoundaries":boundaries,
            "localCfgHash":hash,
        })
    }

    fn local_cfg_descriptor_graph(owner: Option<&str>) -> Value {
        json!({
            "descriptors":owner.into_iter().map(|symbol| json!({
                "symbolIdentity":symbol,
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn local_cfg_admission_preserves_explicit_edges_without_numeric_ordering() {
        let owner = "callable:p/Box.save#jvm:()V";
        let graph = local_cfg_graph(owner);
        let mut facts = local_cfg_facts(vec![graph.clone()], vec![]);

        let hash =
            normalize_local_cfg_evidence(&mut facts, &local_cfg_descriptor_graph(Some(owner)))
                .unwrap();

        assert_eq!(facts["localCfgs"][0]["edges"][0]["sourceNodeId"], 2);
        assert_eq!(facts["localCfgs"][0]["edges"][1]["sourceNodeId"], 9);
        assert_eq!(facts["localCfgs"][0]["edges"][1]["targetNodeId"], 2);
        assert_eq!(facts["localCfgHash"], hash);
        assert!(facts["localCfgBoundaries"].as_array().unwrap().is_empty());
        assert_eq!(facts["localCfgs"][0], graph);
    }

    #[test]
    fn local_cfg_unproven_owner_becomes_typed_unknown() {
        let graph = local_cfg_graph("callable:p/Box.save#jvm:()V");
        let raw_hash = crate::canonical::hash(&graph).unwrap();
        let mut facts = local_cfg_facts(vec![graph], vec![]);

        normalize_local_cfg_evidence(&mut facts, &local_cfg_descriptor_graph(None)).unwrap();

        assert!(facts["localCfgs"].as_array().unwrap().is_empty());
        let boundary = &facts["localCfgBoundaries"][0];
        assert_eq!(boundary["code"], "UNPROVEN_LOCAL_CFG_OWNER");
        assert_eq!(boundary["resolution"], "UNKNOWN");
        assert_eq!(boundary["rawRowHash"], raw_hash);
        crate::thread_flow_cfg::validate_boundary(boundary).unwrap();
    }

    #[test]
    fn local_cfg_unknown_boundary_drops_only_an_invalid_optional_owner() {
        let boundary = json!({
            "schema":"local-cfg-boundary/0.1",
            "ownerSymbolIdentity":"callable:<local>/Box.save#jvm:(Lrendered.Type;)V",
            "stage":"NORMALIZE",
            "code":"NO_SOURCE_FUNCTION",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR_CFG",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "rawRowHash":format!("sha256:{}", "4".repeat(64)),
        });
        let mut facts = local_cfg_facts(vec![], vec![boundary]);

        normalize_local_cfg_evidence(&mut facts, &local_cfg_descriptor_graph(None)).unwrap();

        let normalized = &facts["localCfgBoundaries"][0];
        assert!(normalized.get("ownerSymbolIdentity").is_none());
        assert_eq!(normalized["code"], "NO_SOURCE_FUNCTION");
        assert_eq!(normalized["resolution"], "UNKNOWN");
        crate::thread_flow_cfg::validate_boundary(normalized).unwrap();
    }

    #[test]
    fn local_cfg_rejects_forged_hash_and_normalizer_impersonation() {
        let owner = "callable:p/Box.save#jvm:()V";
        let mut forged = local_cfg_facts(vec![local_cfg_graph(owner)], vec![]);
        forged["localCfgHash"] = json!(format!("sha256:{}", "0".repeat(64)));
        assert_eq!(
            normalize_local_cfg_evidence(&mut forged, &local_cfg_descriptor_graph(Some(owner)),)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );

        let boundary = json!({
            "schema":"local-cfg-boundary/0.1",
            "stage":"NORMALIZE",
            "code":"INVALID_LOCAL_CFG",
            "resolution":"UNKNOWN",
            "provider":"CODECLEW_LOCAL_CFG_NORMALIZER",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "rawRowHash":format!("sha256:{}", "1".repeat(64)),
        });
        let mut impersonated = local_cfg_facts(vec![], vec![boundary]);
        assert_eq!(
            normalize_local_cfg_evidence(
                &mut impersonated,
                &local_cfg_descriptor_graph(Some(owner)),
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn compiler_local_cfg_snapshot_hash_matches_rust_canonical_bytes() {
        let _workspace_worker_guard = workspace_worker_test_lock();
        let workspace = workspace_root();
        let repo = workspace
            .join("fixtures/kotlin-basic")
            .canonicalize()
            .unwrap();
        let mut worker = WorkerClient::start(&workspace).unwrap();
        worker
            .open_project_verified(&json!({"repo":repo,"compilation":":/main"}))
            .unwrap();
        let facts = worker
            .request(
                RequestKind::IndexFiles,
                &json!({"repo":repo,"compilation":":/main","syntaxOnly":false}),
            )
            .unwrap();
        let graphs = facts["localCfgs"].as_array().unwrap();
        let boundaries = facts["localCfgBoundaries"].as_array().unwrap();
        assert_eq!(
            facts["localCfgHash"].as_str().unwrap(),
            local_cfg_snapshot_hash(graphs, boundaries).unwrap(),
        );
        worker.shutdown().unwrap();
    }

    #[test]
    fn non_jvms_exact_descriptor_is_quarantined_as_typed_unknown() {
        let malformed = "(Lcompiler.rendered.Type;)V";
        let mut facts = descriptor_normalization_facts(malformed);
        let raw_hash =
            crate::canonical::hash(&facts["declarationDescriptors"]["descriptors"][0]).unwrap();

        let quarantined = normalize_invalid_jvm_descriptors(&mut facts).unwrap();

        assert_eq!(quarantined, 1);
        let graph = &facts["declarationDescriptors"];
        assert_eq!(graph["coverage"], "PARTIAL");
        assert!(graph["descriptors"].as_array().unwrap().is_empty());
        let boundary = &graph["boundaries"][0];
        assert_eq!(boundary["code"], "INVALID_JVM_DESCRIPTOR");
        assert_eq!(boundary["provider"], "CODECLEW_DESCRIPTOR_NORMALIZER");
        assert_eq!(boundary["rawRowHash"], raw_hash);
        crate::semantic_validation::validate_declaration_descriptor_boundary(boundary).unwrap();
        assert_eq!(
            facts["declarationDescriptorHash"],
            crate::canonical::hash(graph).unwrap()
        );
    }

    #[test]
    fn non_jvms_exact_function_descriptor_is_quarantined_as_typed_unknown() {
        let malformed = "(Lp/Outer.Inner;)V";
        let mut facts = function_descriptor_normalization_facts(malformed);
        let raw_hash =
            crate::canonical::hash(&facts["declarationDescriptors"]["descriptors"][0]).unwrap();

        let quarantined = normalize_invalid_jvm_descriptors(&mut facts).unwrap();

        assert_eq!(quarantined, 1);
        let graph = &facts["declarationDescriptors"];
        assert_eq!(graph["coverage"], "PARTIAL");
        assert!(graph["descriptors"].as_array().unwrap().is_empty());
        let boundary = &graph["boundaries"][0];
        assert_eq!(boundary["code"], "INVALID_JVM_DESCRIPTOR");
        assert_eq!(boundary["provider"], "CODECLEW_DESCRIPTOR_NORMALIZER");
        assert_eq!(boundary["rawRowHash"], raw_hash);
        crate::semantic_validation::validate_declaration_descriptor_boundary(boundary).unwrap();
        assert_eq!(
            facts["declarationDescriptorHash"],
            crate::canonical::hash(graph).unwrap()
        );
    }

    #[test]
    fn valid_exact_descriptor_is_not_normalized_and_forged_hash_is_rejected() {
        let mut valid = descriptor_normalization_facts("(Ljava/lang/String;)V");
        let original = valid.clone();
        assert_eq!(normalize_invalid_jvm_descriptors(&mut valid).unwrap(), 0);
        assert_eq!(valid, original);

        let mut valid_function = function_descriptor_normalization_facts("(Ljava/lang/String;)V");
        let original_function = valid_function.clone();
        assert_eq!(
            normalize_invalid_jvm_descriptors(&mut valid_function).unwrap(),
            0
        );
        assert_eq!(valid_function, original_function);

        let mut forged = descriptor_normalization_facts("(Lcompiler.rendered.Type;)V");
        forged["declarationDescriptorHash"] = json!("sha256:forged");
        assert_eq!(
            normalize_invalid_jvm_descriptors(&mut forged)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );

        let mut impersonated = descriptor_normalization_facts("(Ljava/lang/String;)V");
        impersonated["declarationDescriptors"]["boundaries"] = json!([{
            "schema":"declaration-descriptor-boundary/0.1",
            "stage":"NORMALIZE","code":"INVALID_JVM_DESCRIPTOR",
            "resolution":"UNKNOWN","provider":"CODECLEW_DESCRIPTOR_NORMALIZER"
        }]);
        impersonated["declarationDescriptorHash"] =
            json!(crate::canonical::hash(&impersonated["declarationDescriptors"]).unwrap());
        assert_eq!(
            normalize_invalid_jvm_descriptors(&mut impersonated)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn optional_relation_evidence_becomes_typed_unknown_without_losing_base_call() {
        let call = json!({
            "schema":"declaration-relation/0.1",
            "kind":"CALLS",
            "owner":"p/caller",
            "target":"p/callee",
            "argumentToParameter":[{"argumentStart":9,"parameterIndex":0,"parameterType":"kotlin/String"}],
        });
        let raw_hash = crate::canonical::hash(&call).unwrap();
        let mut facts = relation_normalization_facts(vec![call]);

        normalize_optional_relation_evidence(&mut facts).unwrap();

        let graph = &facts["declarationRelations"];
        assert_eq!(graph["coverage"], "PARTIAL");
        assert_eq!(graph["relations"].as_array().unwrap().len(), 1);
        assert!(graph["relations"][0].get("argumentToParameter").is_none());
        assert_eq!(
            graph["boundaries"][0]["code"],
            "ARGUMENT_MAPPING_UNAVAILABLE"
        );
        assert_eq!(graph["boundaries"][0]["affectedRowCount"], 1);
        assert_eq!(
            graph["boundaries"][0]["rawRowsHash"],
            crate::canonical::hash(&json!([raw_hash])).unwrap()
        );
        assert_eq!(
            facts["declarationRelationHash"],
            crate::canonical::hash(graph).unwrap()
        );
    }

    #[test]
    fn exact_call_argument_mapping_survives_optional_evidence_normalization() {
        let call = json!({
            "schema":"declaration-relation/0.1",
            "file":"A.kt","start":10,"end":40,
            "kind":"CALLS","owner":"p/Caller.run",
            "target":"callable:p/Api.pick#jvm:(Ljava/lang/String;I)Ljava/lang/String;",
            "targetCompilerCallableId":"p/Api.pick",
            "targetJvmDescriptor":"(Ljava/lang/String;I)Ljava/lang/String;",
            "resolution":"PROVEN","provider":"K2_FIR","cfgNodeIds":[7],
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "orderProvenance":"K2_FIR_CFG","orderKey":10,
            "resultType":"kotlin/String","receiverSelection":"EXPLICIT",
            "receiverType":"p/Api","omittedDefaultParameterIndices":[1],
            "argumentToParameter":[{
                "argumentStart":20,"argumentEnd":25,"argumentName":"value",
                "argumentType":"kotlin/String","parameter":"value",
                "parameterIndex":0,"parameterType":"kotlin/String"
            }]
        });
        crate::semantic_validation::validate_declaration_relation_fact(&call).unwrap();
        let mut facts = relation_normalization_facts(vec![call.clone()]);

        normalize_optional_relation_evidence(&mut facts).unwrap();

        let graph = &facts["declarationRelations"];
        assert_eq!(graph["coverage"], "COMPLETE_SUPPORTED_SUBSET");
        assert!(graph["boundaries"].as_array().unwrap().is_empty());
        assert_eq!(graph["relations"][0], call);
        crate::semantic_validation::validate_declaration_relation_fact(&graph["relations"][0])
            .unwrap();
        assert_eq!(
            facts["declarationRelationHash"],
            crate::canonical::hash(graph).unwrap()
        );
    }

    #[test]
    fn unreliable_flow_rows_do_not_discard_independent_compiler_relations() {
        let mut facts = relation_normalization_facts(vec![
            json!({"schema":"declaration-relation/0.1","kind":"OVERRIDES","owner":"p/impl","target":"p/api"}),
            json!({"schema":"declaration-relation/0.1","kind":"NULL_COALESCES","owner":"p/f","target":"p/fallback"}),
            json!({"schema":"declaration-relation/0.1","kind":"RETURNS_VALUE_FROM","owner":"p/f","target":"p/source"}),
        ]);

        normalize_optional_relation_evidence(&mut facts).unwrap();

        let relations = facts["declarationRelations"]["relations"]
            .as_array()
            .unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0]["kind"], "OVERRIDES");
        let codes = facts["declarationRelations"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            codes,
            std::collections::BTreeSet::from([
                "NULL_COALESCING_FLOW_UNAVAILABLE",
                "RETURN_VALUE_FLOW_UNAVAILABLE",
            ])
        );
    }

    #[test]
    fn optional_relation_normalization_rejects_a_forged_input_hash() {
        let mut facts = relation_normalization_facts(vec![json!({
            "schema":"declaration-relation/0.1",
            "kind":"CALLS",
            "owner":"p/caller",
            "target":"p/callee",
        })]);
        facts["declarationRelationHash"] = json!("sha256:forged");
        assert_eq!(
            normalize_optional_relation_evidence(&mut facts)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
    }

    #[test]
    fn embedded_worker_distribution_rejects_workspace_and_private_tree_mutation() {
        use std::os::unix::fs::PermissionsExt;

        let _workspace_worker_guard = workspace_worker_test_lock();

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

        let trusted = &worker.trusted_distribution;
        let private_launcher = trusted.launcher.clone();
        let launcher_bytes = std::fs::read(&private_launcher).unwrap();
        let launcher_permissions = std::fs::metadata(&private_launcher).unwrap().permissions();
        std::fs::write(&private_launcher, "#!/bin/sh\nexit 88\n").unwrap();
        assert_eq!(
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::write(&private_launcher, launcher_bytes).unwrap();
        std::fs::set_permissions(&private_launcher, launcher_permissions).unwrap();

        let plugin = trusted
            .distribution_root
            .join("lib")
            .join(KotlinSemanticEngine::Kotlin24.plugin_jar_name());
        let plugin_bytes = std::fs::read(&plugin).unwrap();
        std::fs::write(&plugin, b"changed").unwrap();
        assert_eq!(
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::write(&plugin, plugin_bytes).unwrap();

        let extra = trusted.distribution_root.join("unexpected-authority-input");
        std::fs::write(&extra, b"extra").unwrap();
        assert_eq!(
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::remove_file(extra).unwrap();
        assert!(worker.inspect_verified_index(&verified).is_ok());
        worker.shutdown().unwrap();

        let workspace_launcher = worker_launcher(&workspace, KotlinSemanticEngine::Kotlin24);
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
            .join(KotlinSemanticEngine::Kotlin24.distribution_relative())
            .join("lib")
            .join(KotlinSemanticEngine::Kotlin24.plugin_jar_name());
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

        let distribution = workspace.join(KotlinSemanticEngine::Kotlin24.distribution_relative());
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

    #[cfg(unix)]
    #[test]
    fn missing_trusted_distribution_runs_the_exact_wrapper_task_once() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().canonicalize().unwrap();
        let wrapper = workspace.join("gradlew");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > bootstrap-args\nmkdir -p workers/kotlin/build/install/kotlin\n",
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let distribution = workspace.join(KotlinSemanticEngine::Kotlin24.distribution_relative());

        assert!(
            bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                KotlinSemanticEngine::Kotlin24,
                &distribution,
            )
            .unwrap()
        );
        assert!(distribution.is_dir());
        assert_eq!(
            std::fs::read_to_string(workspace.join("bootstrap-args")).unwrap(),
            ":workers:kotlin:installDist\n--no-daemon\n--quiet\n"
        );

        assert!(
            !bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                KotlinSemanticEngine::Kotlin24,
                &distribution,
            )
            .unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn missing_trusted_distribution_uses_the_caller_build_environment() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().canonicalize().unwrap();
        let wrapper = workspace.join("gradlew");
        std::fs::write(
            &wrapper,
            r#"#!/bin/sh
{
  printf 'GRADLE_USER_HOME=%s\n' "$GRADLE_USER_HOME"
  printf 'GRADLE_OPTS=%s\n' "$GRADLE_OPTS"
  printf 'JAVA_OPTS=%s\n' "$JAVA_OPTS"
  printf 'JAVA_TOOL_OPTIONS=%s\n' "$JAVA_TOOL_OPTIONS"
  printf 'JDK_JAVA_OPTIONS=%s\n' "$JDK_JAVA_OPTIONS"
  printf '_JAVA_OPTIONS=%s\n' "$_JAVA_OPTIONS"
  printf 'ORG_GRADLE_PROJECT_codeclewBootstrapMarker=%s\n' "$ORG_GRADLE_PROJECT_codeclewBootstrapMarker"
} > bootstrap-environment
/bin/mkdir -p workers/kotlin/build/install/kotlin
"#,
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let distribution = workspace.join(KotlinSemanticEngine::Kotlin24.distribution_relative());
        let gradle_home = workspace.join("caller-gradle-home");

        // Inject an isolated snapshot instead of mutating the process environment:
        // Rust tests execute concurrently, so global set_var/remove_var would race.
        let build_environment = vec![
            (
                OsString::from("GRADLE_USER_HOME"),
                gradle_home.clone().into_os_string(),
            ),
            (
                OsString::from("GRADLE_OPTS"),
                OsString::from("-Dcodeclew.gradle.marker=caller"),
            ),
            (
                OsString::from("JAVA_OPTS"),
                OsString::from("-Dcodeclew.java.marker=java-opts"),
            ),
            (
                OsString::from("JAVA_TOOL_OPTIONS"),
                OsString::from("-Dcodeclew.java.marker=tool-options"),
            ),
            (
                OsString::from("JDK_JAVA_OPTIONS"),
                OsString::from("-Dcodeclew.java.marker=jdk-options"),
            ),
            (
                OsString::from("_JAVA_OPTIONS"),
                OsString::from("-Dcodeclew.java.marker=legacy-options"),
            ),
            (
                OsString::from("ORG_GRADLE_PROJECT_codeclewBootstrapMarker"),
                OsString::from("caller-project-property"),
            ),
        ];

        assert!(
            bootstrap_trusted_worker_distribution_if_missing_with_environment(
                &workspace,
                KotlinSemanticEngine::Kotlin24,
                &distribution,
                build_environment,
            )
            .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("bootstrap-environment")).unwrap(),
            format!(
                "GRADLE_USER_HOME={}\n\
GRADLE_OPTS=-Dcodeclew.gradle.marker=caller\n\
JAVA_OPTS=-Dcodeclew.java.marker=java-opts\n\
JAVA_TOOL_OPTIONS=-Dcodeclew.java.marker=tool-options\n\
JDK_JAVA_OPTIONS=-Dcodeclew.java.marker=jdk-options\n\
_JAVA_OPTIONS=-Dcodeclew.java.marker=legacy-options\n\
ORG_GRADLE_PROJECT_codeclewBootstrapMarker=caller-project-property\n",
                gradle_home.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_or_symlinked_distribution_state_is_never_rebuilt() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().canonicalize().unwrap();
        let wrapper = workspace.join("gradlew");
        std::fs::write(&wrapper, "#!/bin/sh\ntouch wrapper-invoked\nexit 99\n").unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let distribution = workspace.join(KotlinSemanticEngine::Kotlin24.distribution_relative());
        std::fs::create_dir_all(&distribution).unwrap();
        std::fs::write(distribution.join("drifted"), b"not trusted").unwrap();

        assert!(
            !bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                KotlinSemanticEngine::Kotlin24,
                &distribution,
            )
            .unwrap()
        );
        assert!(!workspace.join("wrapper-invoked").exists());

        std::fs::remove_dir_all(workspace.join("workers/kotlin/build")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.join("workers/kotlin")).unwrap();
        symlink(outside.path(), workspace.join("workers/kotlin/build")).unwrap();
        assert_eq!(
            bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                KotlinSemanticEngine::Kotlin24,
                &distribution,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerPreparationRequired
        );
        assert!(!workspace.join("wrapper-invoked").exists());
    }

    #[test]
    fn compiler_receipt_requires_explicit_successful_k2_validation() {
        assert_eq!(
            require_k2_validated(&json!({"k2Validated":false}))
                .unwrap_err()
                .code,
            ErrorCode::IncompleteSemanticAnalysis
        );
        assert_eq!(
            require_k2_validated(&json!({})).unwrap_err().code,
            ErrorCode::IncompleteSemanticAnalysis
        );
        require_k2_validated(&json!({"k2Validated":true})).unwrap();
    }

    fn transport_blob(root: &Path, bytes: &[u8]) -> BlobRef {
        let hash = crate::canonical::hash_bytes(bytes);
        let digest = hash.trim_start_matches("sha256:").to_owned();
        std::fs::create_dir_all(root.join("sha256")).unwrap();
        std::fs::write(root.join("sha256").join(&digest), bytes).unwrap();
        BlobRef {
            content_hash: hash,
            relative_path: format!("sha256/{digest}"),
            size_bytes: bytes.len() as u64,
        }
    }

    #[test]
    fn index_response_body_supports_bounded_inline_or_one_verified_blob() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let inline = br#"{"schema":"semantic-index/0.1"}"#.to_vec();
        assert_eq!(
            index_response_body(inline.clone(), &[], &canonical_root).unwrap(),
            inline
        );

        let body = br#"{"schema":"semantic-index/0.1","large":true}"#;
        let blob = transport_blob(&canonical_root, body);
        let path = canonical_root.join(&blob.relative_path);
        assert_eq!(
            index_response_body(Vec::new(), std::slice::from_ref(&blob), &canonical_root).unwrap(),
            body
        );
        assert!(!path.exists());
    }

    #[test]
    fn index_response_body_rejects_ambiguous_or_tampered_blob_authority() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let body = b"bounded canonical body";
        let blob = transport_blob(&canonical_root, body);
        assert_eq!(
            index_response_body(
                b"inline".to_vec(),
                std::slice::from_ref(&blob),
                &canonical_root,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );
        assert_eq!(
            index_response_body(Vec::new(), &[], &canonical_root)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        assert_eq!(
            index_response_body(Vec::new(), &[blob.clone(), blob.clone()], &canonical_root,)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut wrong_size = blob.clone();
        wrong_size.size_bytes += 1;
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &wrong_size)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        let mut escaping = blob.clone();
        escaping.relative_path = "../body".into();
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &escaping)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );

        std::fs::write(
            canonical_root.join(&blob.relative_path),
            b"tampered canonical bod",
        )
        .unwrap();
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &blob)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[cfg(unix)]
    #[test]
    fn index_response_body_rejects_symlinked_cas_objects() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let body = b"bounded canonical body";
        let blob = transport_blob(&canonical_root, body);
        let target = canonical_root.join(&blob.relative_path);
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::remove_file(&target).unwrap();
        symlink(outside.path(), target).unwrap();
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &blob)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
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
        let inspected = worker.inspect_verified_index(&verified).unwrap();
        crate::semantic_validation::validate_declaration_relation_snapshot(inspected).unwrap();
        crate::semantic_validation::validate_declaration_descriptor_snapshot(inspected).unwrap();

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
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
        worker.shutdown().unwrap();
    }
}
