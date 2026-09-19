//! Crash-reclaimable managed scratch for closed Java analysis.

use crate::error::{ClewError, ErrorCode};
use crate::state::{ManagedDirectory, ManagedTemporaryDirectory, StateAuthority};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

const NAMESPACE: &str = "java-analysis-inputs";
const LEASE_PREFIX: &str = "java-inputs";
const REGISTRY_LOCK: &str = ".registry.lock";
const LIVENESS_LOCK: &str = ".liveness.lock";
const READY_MARKER: &str = ".lease.ready";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaScratchReport {
    pub active: u64,
    pub reclaimable: u64,
    pub reclaimed: u64,
    pub unknown: u64,
}

impl JavaScratchReport {
    fn add_unknown(&mut self) {
        self.unknown = self.unknown.saturating_add(1);
    }
}

/// Inspect the managed Java scratch namespace and optionally reclaim leases
/// whose liveness lock is no longer held. Dry runs never create the namespace.
pub fn inspect_or_reclaim(
    state: &StateAuthority,
    apply: bool,
) -> Result<JavaScratchReport, ClewError> {
    let attempts = state.directory(Path::new("attempts"))?;
    let namespace_name = OsStr::new(NAMESPACE);
    let names = attempts.entries()?;
    if !names.iter().any(|name| name == namespace_name) {
        return Ok(JavaScratchReport::default());
    }
    let namespace = attempts.existing_child(namespace_name)?;
    let _registry = exclusive_lock(&namespace, REGISTRY_LOCK)?;
    inspect_locked(&namespace, apply)
}

/// Allocate one request-scoped managed lease and reclaim stale leases in the
/// same namespace while the registry lock is held.
pub(crate) fn open(state: &StateAuthority) -> Result<JavaAnalysisScratch, ClewError> {
    let attempts = state.directory(Path::new("attempts"))?;
    let namespace = attempts.child(Path::new(NAMESPACE))?;
    let _registry = exclusive_lock(&namespace, REGISTRY_LOCK)?;
    let _ = inspect_locked(&namespace, true)?;

    let lease = namespace.temporary_child(LEASE_PREFIX)?;
    let directory = lease.directory();
    let liveness = directory.open_lock(OsStr::new(LIVENESS_LOCK))?;
    lock_blocking(&liveness)?;
    // Reclaimers only touch leases with this marker. It is written after the
    // lock is held, so an active bootstrap cannot be mistaken for a stale one.
    let _ready = directory.create_file(OsStr::new(READY_MARKER))?;
    let root = directory.child(Path::new("root"))?.resolved_path()?;
    let runtime = directory.child(Path::new("runtime"))?.resolved_path()?;
    Ok(JavaAnalysisScratch {
        namespace,
        lease: Some(lease),
        liveness: Some(liveness),
        root,
        runtime,
    })
}

pub(crate) struct JavaAnalysisScratch {
    namespace: ManagedDirectory,
    lease: Option<ManagedTemporaryDirectory>,
    liveness: Option<File>,
    root: PathBuf,
    runtime: PathBuf,
}

impl std::fmt::Debug for JavaAnalysisScratch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JavaAnalysisScratch")
            .field("root", &self.root)
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl JavaAnalysisScratch {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn runtime(&self) -> &Path {
        &self.runtime
    }

    pub(crate) fn duplicate_liveness(&self) -> Result<File, ClewError> {
        self.liveness
            .as_ref()
            .ok_or_else(|| ClewError::new(ErrorCode::Internal, "Java scratch lease is closed"))?
            .try_clone()
            .map_err(io_error)
    }

    pub(crate) fn close(mut self) -> Result<(), ClewError> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), ClewError> {
        let Some(lease) = self.lease.as_ref() else {
            return Ok(());
        };
        let _registry = exclusive_lock(&self.namespace, REGISTRY_LOCK)?;
        // Drop only our descriptor. A surviving child must retain the lock,
        // including on an I/O error that returns before wait completes.
        drop(self.liveness.take());
        let claim = lease
            .directory()
            .open_existing_lock(OsStr::new(LIVENESS_LOCK))?;
        let removable = try_lock(&claim)?;
        let lease = self
            .lease
            .take()
            .expect("lease is present under the registry lock");
        if removable {
            lease.close()
        } else {
            lease.defer_cleanup();
            Ok(())
        }
    }
}

impl Drop for JavaAnalysisScratch {
    fn drop(&mut self) {
        if self.finish().is_err()
            && let Some(lease) = self.lease.take()
        {
            // Preserve ownership for supported reclamation if the registry or
            // lease cannot be checked. Never remove a possibly active tree.
            lease.defer_cleanup();
        }
    }
}

/// Arrange for a spawned child to retain a duplicate of the request lease.
/// The duplicate is kept alive by the pre-exec closure until fork/exec; the
/// child then keeps the open file description after the parent disappears.
pub(crate) fn attach_child_lease(command: &mut Command, lease: File) -> Result<(), ClewError> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        let fd = lease.as_raw_fd();
        // SAFETY: the closure performs only async-signal-safe fcntl calls and
        // owns `lease` so the duplicated descriptor remains open through exec.
        unsafe {
            command.pre_exec(move || {
                let _keep_alive = &lease;
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (command, lease);
        Err(unsupported(
            "Java scratch leases require POSIX process handles",
        ))
    }
}

fn inspect_locked(
    namespace: &ManagedDirectory,
    apply: bool,
) -> Result<JavaScratchReport, ClewError> {
    let mut report = JavaScratchReport::default();
    for name in namespace.entries()? {
        if name == OsStr::new(REGISTRY_LOCK) {
            continue;
        }
        let Some(_) = lease_uuid(&name) else {
            report.add_unknown();
            continue;
        };
        let child = match namespace.existing_child(&name) {
            Ok(child) => child,
            Err(_) => {
                report.add_unknown();
                continue;
            }
        };
        if !child.file_exists(OsStr::new(READY_MARKER))? {
            if child.entries()?.is_empty() && apply {
                let identity = child.identity()?;
                if namespace.remove_tree_if_identity(&name, identity)? {
                    report.reclaimed = report.reclaimed.saturating_add(1);
                } else {
                    report.add_unknown();
                }
            } else {
                report.add_unknown();
            }
            continue;
        }
        let liveness = match child.open_existing_lock(OsStr::new(LIVENESS_LOCK)) {
            Ok(liveness) => liveness,
            Err(_) => {
                report.add_unknown();
                continue;
            }
        };
        if !try_lock(&liveness)? {
            report.active = report.active.saturating_add(1);
            continue;
        }
        report.reclaimable = report.reclaimable.saturating_add(1);
        if apply {
            let identity = child.identity()?;
            if namespace.remove_tree_if_identity(&name, identity)? {
                report.reclaimed = report.reclaimed.saturating_add(1);
            } else {
                report.add_unknown();
            }
        }
    }
    Ok(report)
}

fn lease_uuid(name: &OsStr) -> Option<uuid::Uuid> {
    let text = name.to_str()?.strip_prefix("java-inputs-")?;
    let uuid = uuid::Uuid::parse_str(text).ok()?;
    (uuid.to_string() == text).then_some(uuid)
}

fn exclusive_lock(namespace: &ManagedDirectory, name: &str) -> Result<File, ClewError> {
    let lock = namespace.open_lock(OsStr::new(name))?;
    lock_blocking(&lock)?;
    Ok(lock)
}

#[cfg(unix)]
fn lock_blocking(file: &File) -> Result<(), ClewError> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn lock_blocking(_file: &File) -> Result<(), ClewError> {
    Err(unsupported("Java scratch leases require POSIX file locks"))
}

#[cfg(unix)]
fn try_lock(file: &File) -> Result<bool, ClewError> {
    use std::os::fd::AsRawFd;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK
    ) {
        Ok(false)
    } else {
        Err(io_error(error))
    }
}

#[cfg(not(unix))]
fn try_lock(_file: &File) -> Result<bool, ClewError> {
    Err(unsupported("Java scratch leases require POSIX file locks"))
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(not(unix))]
fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn state() -> (tempfile::TempDir, StateAuthority) {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("state")).unwrap();
        (root, state)
    }

    #[test]
    fn dry_run_does_not_create_namespace() {
        let (_root, state) = state();
        let report = inspect_or_reclaim(&state, false).unwrap();
        assert_eq!(report.active, 0);
        assert!(!state.root().join("attempts/java-analysis-inputs").exists());
    }

    #[test]
    fn normal_drop_removes_managed_lease() {
        let (_root, state) = state();
        let scratch = open(&state).unwrap();
        assert!(scratch.root().is_dir());
        let namespace = state
            .directory(Path::new("attempts"))
            .unwrap()
            .existing_child(OsStr::new(NAMESPACE))
            .unwrap();
        assert_eq!(namespace.entries().unwrap().len(), 2);
        drop(scratch);
        let report = inspect_or_reclaim(&state, true).unwrap();
        assert_eq!(report.active, 0);
        assert_eq!(report.reclaimable, 0);
        assert_eq!(report.reclaimed, 0);
    }

    #[test]
    fn stale_ready_lease_is_reclaimed_but_active_is_busy() {
        let (_root, state) = state();
        let namespace = state
            .directory(Path::new("attempts"))
            .unwrap()
            .child(Path::new(NAMESPACE))
            .unwrap();
        let active = open(&state).unwrap();
        let stale = namespace.temporary_child(LEASE_PREFIX).unwrap();
        let stale_dir = stale.directory();
        let stale_lock = stale_dir.open_lock(OsStr::new(LIVENESS_LOCK)).unwrap();
        lock_blocking(&stale_lock).unwrap();
        stale_dir.create_file(OsStr::new(READY_MARKER)).unwrap();
        let stale_name = stale_dir
            .resolved_path()
            .unwrap()
            .file_name()
            .unwrap()
            .to_owned();
        drop(stale_lock);
        std::mem::forget(stale);
        let report = inspect_or_reclaim(&state, false).unwrap();
        assert_eq!(report.active, 1);
        assert_eq!(report.reclaimable, 1);
        drop(active);
        let report = inspect_or_reclaim(&state, true).unwrap();
        assert_eq!(report.reclaimed, 1);
        assert!(!namespace.resolved_path().unwrap().join(stale_name).exists());
    }

    #[cfg(unix)]
    #[test]
    fn unknown_symlink_is_reported_and_victim_survives() {
        use std::os::unix::fs::symlink;
        let (_root, state) = state();
        let namespace = state
            .directory(Path::new("attempts"))
            .unwrap()
            .child(Path::new(NAMESPACE))
            .unwrap();
        let path = namespace.resolved_path().unwrap();
        let victim = _root.path().join("victim");
        fs::create_dir(&victim).unwrap();
        fs::write(victim.join("keep"), b"sentinel").unwrap();
        symlink(&victim, path.join("java-inputs-not-a-uuid")).unwrap();
        let report = inspect_or_reclaim(&state, true).unwrap();
        assert_eq!(report.unknown, 1);
        assert!(victim.join("keep").exists());
    }
    #[test]
    fn marker_without_existing_lock_is_preserved() {
        let (_root, state) = state();
        let namespace = state
            .directory(Path::new("attempts"))
            .unwrap()
            .child(Path::new(NAMESPACE))
            .unwrap();
        let unknown = namespace.temporary_child(LEASE_PREFIX).unwrap();
        unknown
            .directory()
            .create_file(OsStr::new(READY_MARKER))
            .unwrap();
        fs::write(
            unknown.directory().resolved_path().unwrap().join("keep"),
            b"unknown",
        )
        .unwrap();
        let report = inspect_or_reclaim(&state, true).unwrap();
        assert_eq!(report.unknown, 1);
        assert_eq!(report.reclaimed, 0);
        assert!(
            !unknown
                .directory()
                .file_exists(OsStr::new(LIVENESS_LOCK))
                .unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn sealed_tree_cleanup_does_not_follow_internal_symlink() {
        use std::os::unix::fs::symlink;
        let (root, state) = state();
        let scratch = open(&state).unwrap();
        let victim = root.path().join("victim");
        fs::create_dir(&victim).unwrap();
        fs::write(victim.join("keep"), b"sentinel").unwrap();
        fs::create_dir(scratch.root().join("nested")).unwrap();
        fs::write(scratch.root().join("nested/source"), b"source").unwrap();
        symlink(&victim, scratch.root().join("outside")).unwrap();
        crate::repository_snapshot::seal_tree(scratch.root()).unwrap();
        scratch.close().unwrap();
        assert_eq!(fs::read(victim.join("keep")).unwrap(), b"sentinel");
        assert_eq!(inspect_or_reclaim(&state, true).unwrap().reclaimed, 0);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "launches the configured native JDK to qualify inherited lease lifetime"]
    fn native_java_child_retains_lease_after_parent_handles_close() {
        use std::time::{Duration, Instant};
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let (_root, state) = state();
        let mut scratch = open(&state).unwrap();
        let source = scratch.runtime().join("LeaseHolder.java");
        let ready = scratch.runtime().join("ready");
        let release = scratch.runtime().join("release");
        fs::write(
            &source,
            r#"import java.nio.file.*;
class LeaseHolder {
  public static void main(String[] args) throws Exception {
    Files.writeString(Path.of(args[0]), "ready");
    while (!Files.exists(Path.of(args[1]))) Thread.sleep(20);
  }
}
"#,
        )
        .unwrap();
        let java = PathBuf::from(std::env::var_os("JAVA_HOME").expect("JAVA_HOME is required"))
            .join("bin/java");
        let mut command = Command::new(java);
        command
            .env_clear()
            .arg(&source)
            .arg(&ready)
            .arg(&release)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        attach_child_lease(&mut command, scratch.duplicate_liveness().unwrap()).unwrap();
        let mut child = ChildGuard(command.spawn().unwrap());
        drop(command); // Release the parent's duplicate captured by pre_exec.
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.is_file() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "JVM exited before readiness"
            );
            assert!(Instant::now() < deadline, "JVM readiness timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        // Model process death: preserve the directory but close every parent
        // liveness descriptor without unlocking the shared file description.
        std::mem::forget(scratch.lease.take().unwrap());
        drop(scratch);
        let while_child_alive = inspect_or_reclaim(&state, true).unwrap();
        assert_eq!(while_child_alive.active, 1);
        assert_eq!(while_child_alive.reclaimed, 0);
        assert!(source.is_file());
        fs::write(&release, b"release").unwrap();
        assert!(child.0.wait().unwrap().success());
        let after_exit = inspect_or_reclaim(&state, true).unwrap();
        assert_eq!(after_exit.active, 0);
        assert_eq!(after_exit.reclaimed, 1);
        assert!(!source.exists());
    }
    #[test]
    fn concurrent_creation_close_and_sweep_preserve_active_leases() {
        let (_root, state) = state();
        std::thread::scope(|threads| {
            for _ in 0..4 {
                let state = &state;
                threads.spawn(move || {
                    for _ in 0..12 {
                        let scratch = open(state).unwrap();
                        let path = scratch.root().to_owned();
                        let report = inspect_or_reclaim(state, true).unwrap();
                        assert!(report.active >= 1);
                        assert!(path.is_dir());
                        scratch.close().unwrap();
                    }
                });
            }
        });
        assert_eq!(
            inspect_or_reclaim(&state, true).unwrap(),
            JavaScratchReport::default()
        );
    }
    #[test]
    fn normal_close_defers_cleanup_until_all_child_descriptors_close() {
        let (_root, state) = state();
        let scratch = open(&state).unwrap();
        let path = scratch.root().to_owned();
        let child_descriptor = scratch.duplicate_liveness().unwrap();
        scratch.close().unwrap();
        assert!(path.is_dir());
        assert_eq!(inspect_or_reclaim(&state, true).unwrap().active, 1);
        drop(child_descriptor);
        assert_eq!(inspect_or_reclaim(&state, true).unwrap().reclaimed, 1);
        assert!(!path.exists());
    }
}
