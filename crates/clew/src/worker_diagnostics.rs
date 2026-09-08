//! Bounded, private process diagnostics. Stderr is never a semantic authority.
use crate::error::{ClewError, ErrorCode};
use crate::state::StateAuthority;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STDERR_LIMIT: usize = 64 * 1024;
const PREFIX: &str = "worker-process-diagnostic:";
const SCHEMA: &str = "codeclew-worker-process-diagnostic/1.0";

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
    result
}

pub(crate) fn annotate_failure(
    mut error: ClewError,
    child: Option<&mut Child>,
    stderr: &mut WorkerStderr,
    stage: &str,
    identity: Value,
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
    let state = StateAuthority::process_default().ok();
    let diagnostic = json!({
        "schema":SCHEMA, "stage":stage, "identity":identity,
        "process":process, "stderr":persist_tail(state.as_ref(), &tail),
    });
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
    Some(json!({"stage":stage,"processStatus":status,"exitCode":code,"signal":signal}))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

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
