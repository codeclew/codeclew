//! Bounded, private process diagnostics. Stderr is never a semantic authority.
use crate::error::{ClewError, ErrorCode};
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STDERR_LIMIT: usize = 64 * 1024;
pub(crate) const MAX_WORKER_FRAME_BYTES: u32 = 64 * 1024 * 1024;
const INDEX_CHECKPOINT_LIMIT: u64 = 1024;
const PREFIX: &str = "worker-process-diagnostic:";
const SCHEMA: &str = "codeclew-worker-process-diagnostic/1.0";

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum TransportReason {
    StdoutEofBeforeHeader,
    FrameHeaderReadFailed,
    FrameLengthExceedsLimit,
    FrameBodyReadFailed,
    InvalidProtobuf,
    JvmHeapExhausted,
    JvmMetaspaceExhausted,
    JvmGcOverheadLimit,
    JvmCompressedClassSpaceExhausted,
    OtherJvmOom,
    ResponseRequestIdMismatch,
    ResponseProtocolIdentityMismatch,
    ResponsePayloadMismatch,
    ResponseValidationFailed,
    VerifiedIndexValidationFailed,
}

impl TransportReason {
    fn is_jvm_oom(self) -> bool {
        matches!(
            self,
            Self::JvmHeapExhausted
                | Self::JvmMetaspaceExhausted
                | Self::JvmGcOverheadLimit
                | Self::JvmCompressedClassSpaceExhausted
                | Self::OtherJvmOom
        )
    }
}

#[derive(Debug)]
pub(crate) struct TransportObservation {
    pub reason: TransportReason,
    pub frame: Option<FrameSizes>,
    // "Term" can also be the first four bytes of an invalid frame length.
    // Keep this hint in memory until the child exit and complete line agree.
    pub possible_vm_termination: bool,
}

impl From<TransportReason> for TransportObservation {
    fn from(reason: TransportReason) -> Self {
        Self {
            reason,
            frame: None,
            possible_vm_termination: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FrameSizes {
    pub header_bytes_read: u8,
    pub declared_body_bytes: Option<u32>,
    pub body_bytes_read: u32,
    pub body_limit_bytes: u32,
}

impl FrameSizes {
    fn valid(&self) -> bool {
        self.header_bytes_read <= 4
            && self.body_limit_bytes == MAX_WORKER_FRAME_BYTES
            && self.body_bytes_read <= self.body_limit_bytes
            && self
                .declared_body_bytes
                .map_or(self.body_bytes_read == 0, |declared| {
                    self.header_bytes_read == 4
                        && self.body_bytes_read <= declared
                        && (declared <= self.body_limit_bytes || self.body_bytes_read == 0)
                })
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum LineObservation {
    Match,
    NoMatch,
    Unavailable,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JvmTermination {
    stdout: LineObservation,
    stderr: LineObservation,
    heap_exhaustion_observed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    oom_reason: Option<TransportReason>,
}

const HOTSPOT_OOM_PREFIX: &[u8] = b"Terminating due to java.lang.OutOfMemoryError: ";
const MAX_HOTSPOT_OOM_LINE_BYTES: usize = 192;

fn hotspot_oom_line(line: &[u8]) -> Option<TransportReason> {
    if line.len() > MAX_HOTSPOT_OOM_LINE_BYTES {
        return None;
    }
    let cause = line.strip_suffix(b"\n")?.strip_prefix(HOTSPOT_OOM_PREFIX)?;
    let cause = cause.strip_suffix(b"\r").unwrap_or(cause);
    if cause.is_empty()
        || cause.iter().all(|byte| *byte == b' ')
        || !cause.iter().all(|byte| (b' '..=b'~').contains(byte))
    {
        return None;
    }
    Some(match cause {
        b"Java heap space" => TransportReason::JvmHeapExhausted,
        b"Metaspace" => TransportReason::JvmMetaspaceExhausted,
        b"GC overhead limit exceeded" => TransportReason::JvmGcOverheadLimit,
        b"Compressed class space" => TransportReason::JvmCompressedClassSpaceExhausted,
        _ => TransportReason::OtherJvmOom,
    })
}

fn exited_with_code_three(process: &Value) -> bool {
    process["status"] == "EXITED" && process["exitCode"] == 3 && process["signal"].is_null()
}

fn observe_hotspot_stdout(stdout: &mut ChildStdout) -> (LineObservation, Option<TransportReason>) {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // A descendant can still hold the pipe after the JVM has exited.
        // Read the fixed-size suffix only when all writers have closed it.
        let mut pipe = libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pipe, 1, 0) } != 1 || pipe.revents & libc::POLLHUP == 0 {
            return (LineObservation::Unavailable, None);
        }
        let mut line = Vec::from(&HOTSPOT_OOM_PREFIX[..4]);
        if stdout
            .take((MAX_HOTSPOT_OOM_LINE_BYTES - 3) as u64)
            .read_to_end(&mut line)
            .is_err()
        {
            return (LineObservation::Unavailable, None);
        }
        return if let Some(reason) = hotspot_oom_line(&line) {
            (LineObservation::Match, Some(reason))
        } else {
            (LineObservation::NoMatch, None)
        };
    }
    #[cfg(not(unix))]
    {
        let _ = stdout;
        (LineObservation::Unavailable, None)
    }
}

fn merge_oom_reasons(left: TransportReason, right: TransportReason) -> TransportReason {
    if left == right {
        left
    } else {
        TransportReason::OtherJvmOom
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum IndexPhase {
    ReceiveRequest,
    ResolveInputs,
    CollectCompilerFacts,
    BuildIndex,
    AttachLocalCfg,
    SerializeResponse,
    EncodeResponse,
    WriteResponse,
    ResponseWritten,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IndexProgress {
    phase: IndexPhase,
    request_bytes: u32,
    heap_used_bytes: u64,
    heap_limit_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_files: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compilation_files: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_files: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_source_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compiler_facts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_frame_bytes: Option<u32>,
}

impl IndexProgress {
    fn valid(&self) -> bool {
        self.request_bytes <= MAX_WORKER_FRAME_BYTES
            && self.heap_limit_bytes > 0
            && self.heap_limit_bytes <= (1 << 40)
            && self.heap_used_bytes <= self.heap_limit_bytes
            && self
                .selected_source_bytes
                .is_none_or(|bytes| bytes <= (1 << 50))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IndexCheckpoint {
    schema: String,
    request_id: u64,
    progress: IndexProgress,
}

pub(crate) struct IndexContext<'a> {
    pub root: &'a Path,
    pub request_id: u64,
}

fn read_index_progress(context: IndexContext<'_>) -> Option<IndexProgress> {
    let path = context.root.join("index-progress.json");
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    if !std::fs::symlink_metadata(&path).ok()?.is_file() {
        return None;
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(INDEX_CHECKPOINT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > INDEX_CHECKPOINT_LIMIT {
        return None;
    }
    let row: IndexCheckpoint = serde_json::from_slice(&bytes).ok()?;
    (row.schema == "codeclew-worker-index-progress/1"
        && row.request_id == context.request_id
        && row.progress.valid())
    .then_some(row.progress)
}

#[derive(Default, Clone)]
struct Tail {
    bytes: Vec<u8>,
    total: u64,
    read_failed: bool,
}

impl Tail {
    fn append(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len() as u64);
        let excess = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(STDERR_LIMIT);
        self.bytes.drain(..excess.min(self.bytes.len()));
        self.bytes
            .extend_from_slice(&bytes[bytes.len().saturating_sub(STDERR_LIMIT)..]);
    }
}

pub(crate) struct WorkerStderr {
    tail: Arc<Mutex<Tail>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl WorkerStderr {
    pub(crate) fn start(mut stderr: ChildStderr) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let fd = stderr.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        #[cfg(not(unix))]
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "worker diagnostics require POSIX",
        ));

        let tail = Arc::new(Mutex::new(Tail::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let retained = tail.clone();
        let requested = stop.clone();
        let reader = std::thread::Builder::new()
            .name("clew-worker-stderr".into())
            .spawn(move || {
                let mut buffer = [0u8; 8 * 1024];
                let mut final_bytes = 0;
                loop {
                    match stderr.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            retained
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .append(&buffer[..count]);
                            // Drain the final pipe contents, but never wait forever
                            // for a descendant that inherited the stderr descriptor.
                            if requested.load(Ordering::Acquire) {
                                final_bytes += count;
                                if final_bytes >= STDERR_LIMIT {
                                    break;
                                }
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if requested.load(Ordering::Acquire) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => {
                            retained
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .read_failed = true;
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            tail,
            stop,
            reader: Some(reader),
        })
    }

    fn finish(&mut self) -> Tail {
        self.stop.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.tail
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Drop for WorkerStderr {
    fn drop(&mut self) {
        self.finish();
    }
}

fn termination(child: Option<&mut Child>) -> Value {
    let Some(child) = child else {
        return json!({"status":"UNAVAILABLE"});
    };
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                let signal = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal()
                };
                #[cfg(not(unix))]
                let signal: Option<i32> = None;
                return json!({"status":"EXITED","exitCode":status.code(),"signal":signal});
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => return json!({"status":"RUNNING"}),
            Err(_) => return json!({"status":"UNAVAILABLE"}),
        }
    }
}

fn persist_tail(state: Option<&StateAuthority>, tail: &Tail) -> Value {
    let directory = crate::documentation::support_collect::live_private_directory();
    persist_tail_with_invocation(state, tail, directory.as_deref())
}

fn persist_tail_with_invocation(
    state: Option<&StateAuthority>,
    tail: &Tail,
    directory: Option<&Path>,
) -> Value {
    let metadata = json!({
        "retainedBytes":tail.bytes.len(), "observedBytes":tail.total,
        "limitBytes":STDERR_LIMIT, "truncated":tail.total > tail.bytes.len() as u64,
        "readFailed":tail.read_failed,
    });
    let mut result = metadata;
    let stored = state.and_then(|state| {
        let relative =
            Path::new("attempts/worker-failures").join(format!("{}.stderr", uuid::Uuid::new_v4()));
        let mut file = state.create_private_file(&relative).ok()?;
        file.write_all(&tail.bytes).ok()?;
        file.sync_all().ok()?;
        Some(state.root().join(relative))
    });
    if let Some(path) = stored {
        result["status"] = json!("CAPTURED_PRIVATE");
        result["path"] = json!(path);
    } else {
        result["status"] = json!("UNAVAILABLE");
    }
    result["invocationTail"] = persist_live_tail(directory, tail);
    result
}

fn persist_live_tail(directory: Option<&Path>, tail: &Tail) -> Value {
    let Some(directory) = directory else {
        return json!({"status":"NOT_CAPTURED"});
    };
    let name = format!("worker-{}.stderr", uuid::Uuid::new_v4().simple());
    let stored = crate::documentation::support_collect::write_private_artifacts(
        directory,
        &[(&name, &tail.bytes)],
    )
    .is_ok();
    json!({"status":if stored { "CAPTURED_PRIVATE" } else { "UNAVAILABLE" },
        "observedBytes":tail.total,"retainedBytes":if stored {tail.bytes.len()} else {0},
        "truncated":!stored || tail.total > tail.bytes.len() as u64})
}

/// Preserve compiler messages only in the caller's opt-in private bundle.
/// The public evidence receives bounded counts and status, never message text.
pub(crate) fn capture_compiler_failure(facts: &Value) -> Value {
    let directory = crate::documentation::support_collect::live_private_directory();
    capture_compiler_failure_in(facts, directory.as_deref())
}

fn capture_compiler_failure_in(facts: &Value, directory: Option<&Path>) -> Value {
    let Some(directory) = directory else {
        return json!({"status":"NOT_CAPTURED"});
    };
    let rows = facts.get("diagnostics").and_then(Value::as_array);
    let mut retained = Vec::new();
    let mut size = 0usize;
    for row in rows.into_iter().flatten() {
        let Ok(bytes) = serde_json::to_vec(row) else {
            break;
        };
        if retained.len() == 128 || size.saturating_add(bytes.len()) > STDERR_LIMIT {
            break;
        }
        size += bytes.len();
        retained.push(row.clone());
    }
    let observed = rows.map_or(0, Vec::len);
    let retained_count = retained.len();
    let body = json!({
        "schema":"codeclew-private-compiler-diagnostics/1.0",
        "observedCount":observed,"retainedCount":retained_count,
        "truncated":retained_count < observed,"diagnostics":retained,
    });
    let Ok(bytes) = serde_json::to_vec(&body) else {
        return json!({"status":"UNAVAILABLE"});
    };
    let name = format!("compiler-{}.json", uuid::Uuid::new_v4().simple());
    let saved = crate::documentation::support_collect::write_private_artifacts(
        directory,
        &[(&name, &bytes)],
    )
    .is_ok();
    json!({"status":if saved {"CAPTURED_PRIVATE"} else {"UNAVAILABLE"},
        "observedCount":observed,"retainedCount":if saved {retained_count} else {0},
        "truncated":!saved || retained_count < observed})
}

pub(crate) fn annotate_failure(
    mut error: ClewError,
    child: Option<&mut Child>,
    stderr: &mut WorkerStderr,
    stage: &str,
    identity: Value,
    mut observation: Option<(TransportObservation, &mut ChildStdout)>,
    index: Option<IndexContext<'_>>,
) -> ClewError {
    if !matches!(
        error.code,
        ErrorCode::WorkerCrashed | ErrorCode::WorkerProtocolMismatch
    ) {
        return error;
    }
    // Observe the process before OwnedWorkerProcess cancels its group. Never
    // describe our cleanup SIGKILL as the cause of the original protocol error.
    let process = termination(child);
    let tail = stderr.finish();
    let (stdout, stdout_oom) = match observation.as_mut() {
        Some((observation, pipe))
            if observation.possible_vm_termination && exited_with_code_three(&process) =>
        {
            observe_hotspot_stdout(pipe)
        }
        Some((observation, _)) if observation.reason == TransportReason::StdoutEofBeforeHeader => {
            (LineObservation::NoMatch, None)
        }
        _ => (LineObservation::Unavailable, None),
    };
    let truncated = tail.total > tail.bytes.len() as u64;
    let stderr_oom = tail
        .bytes
        .split_inclusive(|byte| *byte == b'\n')
        // The first retained segment may start in the middle of a line.
        .skip(usize::from(truncated))
        .filter_map(hotspot_oom_line)
        .reduce(merge_oom_reasons);
    let stderr_line = if stderr_oom.is_some() {
        LineObservation::Match
    } else if tail.read_failed || truncated {
        LineObservation::Unavailable
    } else {
        LineObservation::NoMatch
    };
    let oom_reason = if exited_with_code_three(&process) {
        stdout_oom
            .into_iter()
            .chain(stderr_oom)
            .reduce(merge_oom_reasons)
    } else {
        None
    };
    let state = StateAuthority::process_default().ok();
    let mut diagnostic = json!({
        "schema":SCHEMA, "stage":stage, "identity":identity,
        "process":process, "stderr":persist_tail(state.as_ref(), &tail),
        "jvmTermination":JvmTermination{stdout,stderr:stderr_line,
            heap_exhaustion_observed:oom_reason == Some(TransportReason::JvmHeapExhausted),oom_reason},
    });
    if let Some((mut observation, _)) = observation {
        let reason = if let Some(oom_reason) = oom_reason
            && (stdout == LineObservation::Match
                || observation.reason == TransportReason::StdoutEofBeforeHeader)
        {
            error.code = ErrorCode::WorkerCrashed;
            error.retryable = false;
            error.message = "worker JVM reported out-of-memory termination".to_owned();
            if stdout == LineObservation::Match
                && let Some(frame) = &mut observation.frame
            {
                // The four ASCII bytes were the start of a line, not a header declaration.
                frame.declared_body_bytes = None;
            }
            oom_reason
        } else {
            observation.reason
        };
        diagnostic["transport"] = json!({"reason":reason});
        if let Some(frame) = observation.frame {
            diagnostic["transport"]["frame"] = json!(frame);
        }
    }
    if stage == "INDEX_FILES" {
        diagnostic["indexProgress"] = match index.and_then(read_index_progress) {
            Some(progress) => json!({"status":"AVAILABLE","checkpoint":progress}),
            None => json!({"status":"UNAVAILABLE"}),
        };
    }
    error.message.push_str(&format!(
        "; worker stage={stage}, process={}, exitCode={}, signal={}; inspect private worker-process-diagnostic evidence",
        process["status"].as_str().unwrap_or("UNAVAILABLE"),
        process["exitCode"], process["signal"],
    ));
    error.evidence.push(format!("{PREFIX}{diagnostic}"));
    error
}

pub(crate) fn from_evidence(evidence: &[String]) -> Option<Value> {
    evidence.iter().find_map(|entry| {
        let value: Value = serde_json::from_str(entry.strip_prefix(PREFIX)?).ok()?;
        (value["schema"] == SCHEMA).then_some(value)
    })
}

/// Whitelist diagnostics that can be shared. Never copy a path, runtime
/// identity, stderr bytes, or arbitrary strings from the private envelope.
pub(crate) fn safe_summary(value: &Value) -> Option<Value> {
    if value["schema"] != SCHEMA {
        return None;
    }
    let stage = value["stage"].as_str().filter(|stage| {
        matches!(
            *stage,
            "STARTUP" | "OPEN_PROJECT" | "INDEX_FILES" | "SHUTDOWN"
        )
    })?;
    let status = value["process"]["status"]
        .as_str()
        .filter(|status| matches!(*status, "EXITED" | "RUNNING" | "UNAVAILABLE"))?;
    let code = value["process"]["exitCode"]
        .as_u64()
        .filter(|code| *code <= 255);
    let signal = value["process"]["signal"]
        .as_u64()
        .filter(|signal| (1..=127).contains(signal));
    let mut safe = json!({"stage":stage,"processStatus":status,"exitCode":code,"signal":signal});
    if let Ok(reason) =
        serde_json::from_value::<TransportReason>(value["transport"]["reason"].clone())
        && (!reason.is_jvm_oom() || exited_with_code_three(&value["process"]))
    {
        safe["transport"] = json!({"reason":reason});
        if let Ok(frame) = serde_json::from_value::<FrameSizes>(value["transport"]["frame"].clone())
            && frame.valid()
        {
            safe["transport"]["frame"] = json!(frame);
        }
    }
    if let Ok(jvm) = serde_json::from_value::<JvmTermination>(value["jvmTermination"].clone()) {
        let signature_confirmed = exited_with_code_three(&value["process"])
            && (jvm.stdout == LineObservation::Match || jvm.stderr == LineObservation::Match);
        let valid = match jvm.oom_reason {
            Some(reason) => {
                signature_confirmed
                    && reason.is_jvm_oom()
                    && jvm.heap_exhaustion_observed == (reason == TransportReason::JvmHeapExhausted)
            }
            // Older records used the Boolean for the heap-only signature.
            None => jvm.heap_exhaustion_observed == signature_confirmed,
        };
        if valid {
            safe["jvmTermination"] = json!(jvm);
        }
    }
    if stage == "INDEX_FILES" {
        match value["indexProgress"]["status"].as_str() {
            Some("AVAILABLE") => {
                safe["indexProgress"] = match serde_json::from_value::<IndexProgress>(
                    value["indexProgress"]["checkpoint"].clone(),
                ) {
                    Ok(checkpoint) if checkpoint.valid() => {
                        json!({"status":"AVAILABLE","checkpoint":checkpoint})
                    }
                    _ => json!({"status":"UNAVAILABLE"}),
                };
            }
            Some("UNAVAILABLE") => safe["indexProgress"] = json!({"status":"UNAVAILABLE"}),
            _ => {}
        }
    }
    if let (Some(observed), Some(retained)) = (
        value["stderr"]["observedBytes"].as_u64(),
        value["stderr"]["retainedBytes"].as_u64(),
    ) && retained <= STDERR_LIMIT as u64
        && retained <= observed
    {
        safe["stderrCounts"] = json!({"observedBytes":observed,"retainedBytes":retained,"truncated":observed > retained});
    }
    Some(safe)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    #[test]
    fn worker_transport_oom_signature_requires_a_complete_bounded_hotspot_line() {
        fn check(tail: &[u8]) -> Option<TransportReason> {
            hotspot_oom_line(&[HOTSPOT_OOM_PREFIX, tail].concat())
        }
        assert_eq!(
            check(b"Metaspace\n"),
            Some(TransportReason::JvmMetaspaceExhausted)
        );
        assert_eq!(
            check(b"Requested array size exceeds VM limit\n"),
            Some(TransportReason::OtherJvmOom)
        );
        for tail in [
            b"Metaspace".as_slice(),
            b"\n",
            b"\t\n",
            b"Metaspace\nextra\n",
            b"Metaspace\x00\n",
        ] {
            assert_eq!(check(tail), None);
        }
        assert_eq!(
            check(&[vec![b'x'; MAX_HOTSPOT_OOM_LINE_BYTES], vec![b'\n']].concat()),
            None
        );
    }

    #[test]
    fn compiler_messages_are_opt_in_private_bounded_and_never_inline() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("private");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let row = json!({"severity":"ERROR","message":"PRIVATE_COMPILER_SENTINEL","file":"/private/project/Probe.kt"});
        let facts = json!({"diagnostics":vec![row; 200]});
        assert_eq!(
            capture_compiler_failure_in(&facts, None)["status"],
            "NOT_CAPTURED"
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        let status = capture_compiler_failure_in(&facts, Some(&directory));
        assert_eq!(status["status"], "CAPTURED_PRIVATE");
        assert_eq!(status["observedCount"], 200);
        assert_eq!(status["retainedCount"], 128);
        assert_eq!(status["truncated"], true);
        assert!(!status.to_string().contains("PRIVATE_COMPILER_SENTINEL"));
        assert!(!status.to_string().contains("/private/"));
        let artifact = std::fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            std::fs::metadata(&artifact).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let private: Value = serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap();
        assert_eq!(
            private["diagnostics"][0]["message"],
            "PRIVATE_COMPILER_SENTINEL"
        );
        let oversized = json!({"diagnostics":[{"message":"x".repeat(STDERR_LIMIT + 1)}]});
        let status = capture_compiler_failure_in(&oversized, Some(&directory));
        assert_eq!(status["retainedCount"], 0);
        assert_eq!(status["truncated"], true);
    }

    #[test]
    fn live_private_worker_tail_is_opt_in_bounded_and_preserves_state_capture() {
        let mut child = Command::new("/bin/sh").args(["-c", "i=0; while [ $i -lt 4000 ]; do printf '%064d' 0 >&2; i=$((i+1)); done; printf 'PRIVATE_WORKER_SENTINEL' >&2; exit 17"])
            .stderr(Stdio::piped()).spawn().unwrap();
        let mut stderr = WorkerStderr::start(child.stderr.take().unwrap()).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(17));
        let tail = stderr.finish();
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("state")).unwrap();
        let directory = temporary.path().join("private");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let default = persist_tail_with_invocation(Some(&state), &tail, None);
        assert_eq!(default["invocationTail"]["status"], "NOT_CAPTURED");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        let private = persist_tail_with_invocation(Some(&state), &tail, Some(&directory));
        assert_eq!(private["status"], "CAPTURED_PRIVATE");
        assert_eq!(private["invocationTail"]["status"], "CAPTURED_PRIVATE");
        assert_eq!(private["invocationTail"]["retainedBytes"], STDERR_LIMIT);
        assert_eq!(private["invocationTail"]["truncated"], true);
        for diagnostic in [&default, &private] {
            assert_eq!(
                std::fs::read(diagnostic["path"].as_str().unwrap()).unwrap(),
                tail.bytes
            );
            assert!(!diagnostic.to_string().contains("PRIVATE_WORKER_SENTINEL"));
        }
        let artifact = std::fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            std::fs::metadata(&artifact).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            std::fs::read(artifact)
                .unwrap()
                .ends_with(b"PRIVATE_WORKER_SENTINEL")
        );
        // Worker and Maven share one live directory budget, including leftovers.
        let filler = vec![b'x'; crate::documentation::support_collect::FILE_LIMIT];
        crate::documentation::support_collect::write_private_artifacts(
            &directory,
            &[("filler", &filler)],
        )
        .unwrap();
        let more = vec![b'y'; crate::documentation::support_collect::FILE_LIMIT - STDERR_LIMIT];
        crate::documentation::support_collect::write_private_artifacts(
            &directory,
            &[("more", &more)],
        )
        .unwrap();
        assert_eq!(
            persist_live_tail(Some(&directory), &tail)["status"],
            "UNAVAILABLE"
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 3);
    }

    #[test]
    fn stderr_is_drained_bounded_and_saved_only_in_a_private_file() {
        let mut child = Command::new("/bin/sh").args(["-c", "i=0; while [ $i -lt 4000 ]; do printf '%064d' 0 >&2; i=$((i+1)); done; printf 'private-secret-marker' >&2; exit 17"])
            .stderr(Stdio::piped()).spawn().unwrap();
        let mut stderr = WorkerStderr::start(child.stderr.take().unwrap()).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(17));
        let tail = stderr.finish();
        assert_eq!(tail.bytes.len(), STDERR_LIMIT);
        assert!(tail.total > STDERR_LIMIT as u64);
        assert!(tail.bytes.ends_with(b"private-secret-marker"));
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("state")).unwrap();
        let saved = persist_tail(Some(&state), &tail);
        let file = Path::new(saved["path"].as_str().unwrap());
        assert_eq!(std::fs::read(file).unwrap(), tail.bytes);
        assert_eq!(
            std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!saved.to_string().contains("private-secret-marker"));
        assert_eq!(termination(Some(&mut child))["exitCode"], 17);
    }

    #[test]
    fn process_signal_is_observed_without_claiming_oom() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "kill -KILL $$"])
            .spawn()
            .unwrap();
        child.wait().unwrap();
        let observed = termination(Some(&mut child));
        assert_eq!(observed["signal"], 9);
        assert!(observed["exitCode"].is_null());
        assert!(!observed.to_string().contains("OOM"));
    }

    #[test]
    fn collector_shutdown_does_not_wait_for_a_live_writer() {
        let mut child = Command::new("sleep")
            .arg("5")
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = WorkerStderr::start(child.stderr.take().unwrap()).unwrap();
        let start = Instant::now();
        stderr.finish();
        assert!(start.elapsed() < Duration::from_secs(1));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn support_projection_ignores_private_fields_and_unrecognized_strings() {
        let mut private = json!({"schema":SCHEMA,"stage":"INDEX_FILES","process":{"status":"EXITED","exitCode":17,"signal":null},"identity":{"runtimeKey":"private-digest"},"stderr":{"path":"/private/failure.stderr","text":"secret"}});
        let safe = safe_summary(&private).unwrap();
        assert_eq!(safe["exitCode"], 17);
        for secret in ["private", "secret", "runtimeKey", "stderr"] {
            assert!(!safe.to_string().contains(secret));
        }
        private["stage"] = json!("/private/project");
        assert!(safe_summary(&private).is_none());
    }
}
