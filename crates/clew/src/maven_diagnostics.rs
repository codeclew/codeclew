//! Opt-in private Maven build-failure diagnostics.
//!
//! Captured bytes remain in the caller-selected directory. Evidence only records
//! a small safe projection, never artifact names, paths, command arguments, or text.
use crate::error::{ClewError, ErrorCode};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub(crate) const TAIL_LIMIT: usize = 64 * 1024;
const PREFIX: &str = "maven-build-diagnostic:";
const SCHEMA: &str = "codeclew-maven-build-diagnostic/1.0";

/// A validated caller-owned directory which is allowed to receive private raw logs.
#[derive(Debug)]
pub struct DebugOutput {
    #[cfg(unix)]
    directory: File,
}

impl DebugOutput {
    pub(crate) fn open(path: &Path) -> Result<Self, ClewError> {
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err(invalid(
                "debug output directory must use normalized absolute spelling",
            ));
        }
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| invalid("debug output directory must already exist and be readable"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid(
                "debug output path must be a directory, not a symlink",
            ));
        }
        #[cfg(unix)]
        {
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err(invalid("debug output directory must be caller-owned"));
            }
            if metadata.permissions().mode() & 0o777 != 0o700 {
                return Err(invalid("debug output directory must have mode 0700"));
            }
            let encoded = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| invalid("debug output directory path is invalid"))?;
            let fd = unsafe {
                libc::open(
                    encoded.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if fd < 0 {
                return Err(invalid("debug output directory cannot be opened safely"));
            }
            let directory = unsafe { File::from_raw_fd(fd) };
            let metadata = directory
                .metadata()
                .map_err(|_| invalid("debug output directory cannot be inspected safely"))?;
            if metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o777 != 0o700
            {
                return Err(invalid("debug output directory changed during validation"));
            }
            Ok(Self { directory })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err(invalid("debug output capture requires POSIX"))
        }
    }

    fn persist(
        &self,
        stage: &str,
        status: &std::process::ExitStatus,
        stdout: &Tail,
        stderr: &Tail,
    ) -> Value {
        let metadata = json!({
            "schema": SCHEMA,
            "stage": stage,
            "process": process(status),
            "stdout": stream_metadata(stdout),
            "stderr": stream_metadata(stderr),
        });
        let id = uuid::Uuid::new_v4().simple().to_string();
        let stdout_name = format!("maven-{id}.stdout");
        let stderr_name = format!("maven-{id}.stderr");
        let manifest_name = format!("maven-{id}.json");
        let manifest = json!({
            "schema": SCHEMA,
            "stage": stage,
            "process": process(status),
            "stdout": {"artifact": stdout_name, "tail": stream_metadata(stdout)},
            "stderr": {"artifact": stderr_name, "tail": stream_metadata(stderr)},
        });
        // Publish the manifest last: its presence means both raw tails were
        // durably written. Interrupted writes can leave inaccessible orphan
        // artifacts, but never a false completed capture record.
        let stored = serde_json::to_vec(&manifest)
            .map_err(|_| ())
            .and_then(|manifest| {
                self.write_new(&stdout_name, &stdout.bytes)
                    .and_then(|_| self.write_new(&stderr_name, &stderr.bytes))
                    .and_then(|_| self.write_new(&manifest_name, &manifest))
            })
            .is_ok();
        let mut evidence = metadata;
        evidence["status"] = json!(if stored {
            "CAPTURED_PRIVATE"
        } else {
            "UNAVAILABLE"
        });
        if !stored {
            evidence["stdout"]["retainedBytes"] = json!(0);
            evidence["stderr"]["retainedBytes"] = json!(0);
        }
        evidence
    }

    fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), ()> {
        #[cfg(unix)]
        {
            let name = CString::new(name).map_err(|_| ())?;
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            file.write_all(bytes).map_err(|_| ())?;
            file.sync_all().map_err(|_| ())?;
            let mode = file.metadata().map_err(|_| ())?.permissions().mode() & 0o777;
            if mode != 0o600 {
                return Err(());
            }
            drop(file);
            self.directory.sync_all().map_err(|_| ())?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (name, bytes);
            Err(())
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Tail {
    pub(crate) bytes: Vec<u8>,
    pub(crate) observed: u64,
}

impl Tail {
    pub(crate) fn append(&mut self, next: &[u8]) {
        self.observed = self.observed.saturating_add(next.len() as u64);
        let excess = self
            .bytes
            .len()
            .saturating_add(next.len())
            .saturating_sub(TAIL_LIMIT);
        self.bytes.drain(..excess.min(self.bytes.len()));
        self.bytes
            .extend_from_slice(&next[next.len().saturating_sub(TAIL_LIMIT)..]);
    }
}

pub(crate) fn annotate_failure(
    mut error: ClewError,
    output: Option<&DebugOutput>,
    stage: &str,
    status: &std::process::ExitStatus,
    stdout: &Tail,
    stderr: &Tail,
) -> ClewError {
    let Some(output) = output else {
        return error;
    };
    let diagnostic = output.persist(stage, status, stdout, stderr);
    error.evidence.push(format!("{PREFIX}{diagnostic}"));
    error
}

pub(crate) fn from_evidence(evidence: &[String]) -> Option<Value> {
    evidence.iter().find_map(|entry| {
        let value: Value = serde_json::from_str(entry.strip_prefix(PREFIX)?).ok()?;
        (value["schema"] == SCHEMA).then_some(value)
    })
}

/// Return a shareable allowlisted summary. Private artifact names and locations
/// deliberately cannot be reconstructed from it.
pub(crate) fn safe_summary(value: &Value) -> Option<Value> {
    if value["schema"] != SCHEMA {
        return None;
    }
    let stage = value["stage"]
        .as_str()
        .filter(|stage| matches!(*stage, "EFFECTIVE_POM" | "COMPILE_CLASSPATH" | "RELEASE"))?;
    let status = value["status"]
        .as_str()
        .filter(|status| matches!(*status, "CAPTURED_PRIVATE" | "UNAVAILABLE"))?;
    let exit_code = value["process"]["exitCode"]
        .as_i64()
        .filter(|code| (0..=255).contains(code));
    let signal = value["process"]["signal"]
        .as_i64()
        .filter(|signal| (1..=127).contains(signal));
    let stream = |name: &str| {
        let observed = value[name]["observedBytes"].as_u64()?;
        let retained = value[name]["retainedBytes"].as_u64()?;
        let truncated = value[name]["truncated"].as_bool()?;
        if retained > TAIL_LIMIT as u64 || retained > observed {
            return None;
        }
        if status == "UNAVAILABLE" && retained != 0 {
            return None;
        }
        Some(json!({
            "observedBytes": observed, "retainedBytes": retained, "truncated": truncated,
        }))
    };
    Some(json!({
        "stage": stage, "status": status, "exitCode": exit_code, "signal": signal,
        "stdout": stream("stdout")?, "stderr": stream("stderr")?,
    }))
}

fn stream_metadata(tail: &Tail) -> Value {
    json!({
        "observedBytes": tail.observed,
        "retainedBytes": tail.bytes.len(),
        "limitBytes": TAIL_LIMIT,
        "truncated": tail.observed > tail.bytes.len() as u64,
    })
}

fn process(status: &std::process::ExitStatus) -> Value {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    json!({"exitCode": status.code(), "signal": signal})
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_directory_rejects_unsafe_permissions() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(DebugOutput::open(directory.path()).is_err());
    }

    #[test]
    fn relative_debug_directory_is_rejected() {
        assert!(DebugOutput::open(Path::new("relative-debug-output")).is_err());
    }

    #[test]
    fn persists_only_private_opaque_artifacts_and_projects_safely() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let output = DebugOutput::open(directory.path()).unwrap();
        let stdout = Tail {
            bytes: b"stdout-private".to_vec(),
            observed: 14,
        };
        let stderr = Tail {
            bytes: b"stderr-private".to_vec(),
            observed: 14,
        };
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .status()
            .unwrap();
        let error = annotate_failure(
            ClewError::new(ErrorCode::UnsupportedProjectConfiguration, "failure"),
            Some(&output),
            "EFFECTIVE_POM",
            &status,
            &stdout,
            &stderr,
        );
        let diagnostic = from_evidence(&error.evidence).unwrap();
        let safe = safe_summary(&diagnostic).unwrap();
        assert_eq!(safe["status"], "CAPTURED_PRIVATE");
        assert!(!safe.to_string().contains("private"));
        assert!(!error.to_string().contains("stdout-private"));
        let entries = fs::read_dir(directory.path())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 3);
        for entry in entries {
            let metadata = entry.metadata().unwrap();
            assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn symlink_debug_directory_is_rejected() {
        let parent = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::set_permissions(target.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let link = parent.path().join("debug");
        std::os::unix::fs::symlink(target.path(), &link).unwrap();
        assert!(DebugOutput::open(&link).is_err());
    }

    #[test]
    fn failure_capture_is_opt_in_and_bounded() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let output = DebugOutput::open(directory.path()).unwrap();
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 17"])
            .status()
            .unwrap();
        let error = annotate_failure(
            ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "BUILD_COMMAND_FAILED: Maven Java effective model extraction failed",
            ),
            Some(&output),
            "EFFECTIVE_POM",
            &status,
            &Tail {
                bytes: b"private-stdout".to_vec(),
                observed: 14,
            },
            &Tail {
                bytes: b"private-stderr".to_vec(),
                observed: 14,
            },
        );
        assert!(error.message.starts_with("BUILD_COMMAND_FAILED"));
        assert!(!error.message.contains("private-stdout"));
        assert!(!error.message.contains("private-stderr"));
        let diagnostic = from_evidence(&error.evidence).unwrap();
        let safe = safe_summary(&diagnostic).unwrap();
        assert_eq!(safe["stage"], "EFFECTIVE_POM");
        assert_eq!(safe["exitCode"], 17);
        assert!(!safe.to_string().contains("private-"));
        assert_eq!(
            fs::read_dir(directory.path()).unwrap().count(),
            3,
            "failed Maven command writes exactly three private artifacts"
        );

        let disabled = tempfile::tempdir().unwrap();
        fs::set_permissions(disabled.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let error = annotate_failure(
            ClewError::new(ErrorCode::UnsupportedProjectConfiguration, "failure"),
            None,
            "EFFECTIVE_POM",
            &status,
            &Tail::default(),
            &Tail::default(),
        );
        assert!(from_evidence(&error.evidence).is_none());
        assert_eq!(fs::read_dir(disabled.path()).unwrap().count(), 0);
    }

    #[test]
    fn analyzer_failure_never_leaks_private_sentinel() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let output = DebugOutput::open(directory.path()).unwrap();
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .status()
            .unwrap();
        // A fake failing analyzer writes a private sentinel to both streams.
        const SENTINEL: &str = "PRIVATE_SENTINEL_9f8e /private/example/etc/passwd";
        let error = annotate_failure(
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "Java compiler analyzer did not produce bounded facts",
            ),
            Some(&output),
            "JAVA_ANALYZER",
            &status,
            &Tail {
                bytes: SENTINEL.as_bytes().to_vec(),
                observed: SENTINEL.len() as u64,
            },
            &Tail {
                bytes: SENTINEL.as_bytes().to_vec(),
                observed: SENTINEL.len() as u64,
            },
        );
        // The public error message never carries the raw analyzer payload.
        assert!(!error.message.contains(SENTINEL));
        assert!(error.message.contains("did not produce bounded facts"));
        // Portable evidence carries only allowlisted metadata, never the sentinel.
        let diagnostic = from_evidence(&error.evidence).unwrap();
        assert!(!diagnostic.to_string().contains(SENTINEL));
        assert_eq!(diagnostic["stage"], "JAVA_ANALYZER");
        assert_eq!(diagnostic["process"]["exitCode"], 7);
        assert_eq!(diagnostic["stdout"]["observedBytes"], SENTINEL.len() as u64);
        assert_eq!(diagnostic["stderr"]["observedBytes"], SENTINEL.len() as u64);
        // Raw bytes are retained only as private byte-bounded artifacts.
        let artifacts: Vec<Vec<u8>> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| fs::read(entry.unwrap().path()).ok())
            .collect();
        assert!(!artifacts.is_empty(), "raw capture is persisted privately");
        for artifact in &artifacts {
            assert!(
                artifact.len() <= TAIL_LIMIT,
                "raw capture must be byte-bounded"
            );
        }
    }
}
