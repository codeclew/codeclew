use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::ffi::CStr;
#[cfg(unix)]
use std::os::fd::RawFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

pub const RUNTIME_SCHEMA: &str = "codeclew-runtime-capsule/4.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAuthority {
    pub schema: String,
    pub runtime_key: String,
    pub mode: RuntimeMode,
    pub manifest_digest: String,
    pub components: BTreeMap<String, String>,
    pub artifacts: BTreeMap<String, RuntimeArtifact>,
    pub workers: BTreeMap<String, RuntimeWorker>,
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuntimeMode {
    Release,
    Development,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeArtifact {
    pub mode: u32,
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWorker {
    pub protocol: String,
    pub compiler_version: String,
    pub distribution: String,
    pub tree_hash: String,
    pub files: Vec<RuntimeArtifact>,
}

impl RuntimeAuthority {
    pub fn load(root: &Path) -> Result<Self, ClewError> {
        let metadata = fs::symlink_metadata(root).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("runtime root must be a real directory"));
        }
        let canonical_root = root.canonicalize().map_err(io_error)?;
        if canonical_root != root {
            return Err(invalid("runtime root must use canonical spelling"));
        }
        Self::load_root(canonical_root)
    }

    fn load_root(root: PathBuf) -> Result<Self, ClewError> {
        let manifest_path = root.join("runtime.json");
        let bytes = read_regular(&manifest_path, 1024 * 1024, None)?;
        let mut manifest: Value = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(&format!("runtime manifest is invalid: {error}")))?;
        if !manifest
            .get("platformAuthority")
            .is_some_and(Value::is_object)
            || !manifest
                .get("toolchainAuthority")
                .is_some_and(Value::is_object)
            || !manifest
                .get("inputDigest")
                .and_then(Value::as_str)
                .is_some_and(is_digest)
        {
            return Err(invalid("runtime platform/toolchain authority is missing"));
        }
        let expected_digest = manifest
            .get("manifestDigest")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("runtime manifest digest is missing"))?
            .to_owned();
        manifest["manifestDigest"] = Value::String(String::new());
        let actual_digest = canonical::hash(&manifest).map_err(internal)?;
        if expected_digest != actual_digest {
            return Err(invalid("runtime manifest digest mismatch"));
        }
        manifest["manifestDigest"] = Value::String(expected_digest.clone());
        if !manifest
            .get("workers")
            .and_then(Value::as_object)
            .is_some_and(|workers| {
                workers.values().all(|worker| {
                    worker.get("protocol").and_then(Value::as_str)
                        == Some("semantic-thread.worker.v1")
                })
            })
        {
            return Err(invalid("runtime worker protocol authority is invalid"));
        }
        let mut authority: RuntimeAuthority = serde_json::from_value(manifest)
            .map_err(|error| invalid(&format!("runtime manifest is invalid: {error}")))?;
        if authority.schema != RUNTIME_SCHEMA
            || !is_digest(&authority.runtime_key)
            || authority.manifest_digest != expected_digest
            || authority.components.is_empty()
            || authority
                .components
                .iter()
                .any(|(name, key)| !is_component_id(name) || !is_digest(key))
        {
            return Err(invalid("runtime manifest identity is invalid"));
        }
        authority.root = root;
        for artifact in authority.artifacts.values() {
            verify_artifact(&authority.root, artifact)?;
        }
        let ready = authority.root.join("READY");
        let ready_bytes = read_regular(&ready, 80, None)?;
        if ready_bytes != format!("{}\n", authority.runtime_key).as_bytes() {
            return Err(invalid("runtime capsule is not ready"));
        }
        Ok(authority)
    }

    pub fn from_environment() -> Result<Option<Self>, ClewError> {
        let Some(value) = std::env::var_os("CODECLEW_RUNTIME_ROOT_FD") else {
            return Ok(None);
        };
        #[cfg(unix)]
        {
            let runtime_fd = parse_fd(&value, "runtime root")?;
            let lease_value = std::env::var_os("CODECLEW_RUNTIME_LEASE_FD")
                .ok_or_else(|| invalid("runtime lease descriptor is unavailable"))?;
            let lease_fd = parse_fd(&lease_value, "runtime lease")?;
            validate_directory_fd(runtime_fd, 0o077, "runtime root")?;
            validate_lease_fd(lease_fd)?;
            let root = descriptor_path(runtime_fd)?;
            let authority = Self::load_root(root.clone())?;
            validate_lease_binding(lease_fd, &root, &authority.runtime_key)?;
            let executable = std::env::current_exe()
                .map_err(io_error)?
                .canonicalize()
                .map_err(io_error)?;
            let expected = root.join("bin/clew").canonicalize().map_err(io_error)?;
            if executable != expected {
                return Err(invalid(
                    "runtime descriptor does not contain the executing core binary",
                ));
            }
            Ok(Some(authority))
        }
        #[cfg(not(unix))]
        {
            let _ = value;
            Err(invalid("runtime descriptor authority requires POSIX"))
        }
    }

    pub fn worker(&self, name: &str) -> Result<&RuntimeWorker, ClewError> {
        self.workers
            .get(name)
            .ok_or_else(|| invalid("runtime capsule does not contain the requested worker"))
    }

    pub fn verify_worker(&self, name: &str) -> Result<PathBuf, ClewError> {
        let worker = self.worker(name)?;
        let root = safe_relative(&self.root, &worker.distribution)?;
        let metadata = fs::symlink_metadata(&root).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("runtime worker distribution is unsafe"));
        }
        let mut manifest = BTreeMap::new();
        for artifact in &worker.files {
            verify_artifact(&root, artifact)?;
            manifest.insert(
                artifact.path.clone(),
                format!("{}:{}:{}", artifact.mode, artifact.size, artifact.sha256),
            );
        }
        if tree_digest(&manifest) != worker.tree_hash {
            return Err(invalid("runtime worker tree digest mismatch"));
        }
        Ok(root)
    }
}

#[cfg(unix)]
fn parse_fd(value: &std::ffi::OsStr, label: &str) -> Result<RawFd, ClewError> {
    let text = value
        .to_str()
        .ok_or_else(|| invalid(&format!("{label} descriptor is not UTF-8")))?;
    let descriptor: RawFd = text
        .parse()
        .map_err(|_| invalid(&format!("{label} descriptor is invalid")))?;
    if descriptor < 3 || unsafe { libc::fcntl(descriptor, libc::F_GETFD) } < 0 {
        return Err(invalid(&format!("{label} descriptor is not open")));
    }
    Ok(descriptor)
}

#[cfg(unix)]
fn validate_directory_fd(
    fd: RawFd,
    forbidden_mode: libc::mode_t,
    label: &str,
) -> Result<(), ClewError> {
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, status.as_mut_ptr()) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let status = unsafe { status.assume_init() };
    if status.st_mode & libc::S_IFMT != libc::S_IFDIR
        || status.st_uid != unsafe { libc::geteuid() }
        || status.st_mode & forbidden_mode != 0
    {
        return Err(invalid(&format!("{label} descriptor is unsafe")));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_lease_fd(fd: RawFd) -> Result<(), ClewError> {
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, status.as_mut_ptr()) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let status = unsafe { status.assume_init() };
    if status.st_mode & libc::S_IFMT != libc::S_IFREG
        || status.st_uid != unsafe { libc::geteuid() }
        || status.st_mode & 0o077 != 0
    {
        return Err(invalid("runtime lease descriptor is unsafe"));
    }
    if unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) } != 0 {
        return Err(invalid("runtime lease descriptor cannot be acquired"));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_lease_binding(fd: RawFd, root: &Path, runtime_key: &str) -> Result<(), ClewError> {
    let state = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| invalid("runtime root has no state authority"))?;
    let digest = runtime_key
        .strip_prefix("sha256:")
        .ok_or_else(|| invalid("runtime key is invalid"))?;
    let expected = state.join("locks").join(format!("runtime-{digest}.lease"));
    let expected_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&expected)
        .map_err(io_error)?;
    let metadata = expected_file.metadata().map_err(io_error)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(invalid("runtime lease authority is unsafe"));
    }
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, status.as_mut_ptr()) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let status = unsafe { status.assume_init() };
    if status.st_dev != metadata.dev() as libc::dev_t
        || status.st_ino != metadata.ino() as libc::ino_t
    {
        return Err(invalid(
            "runtime lease descriptor does not match the runtime authority",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn descriptor_path(fd: RawFd) -> Result<PathBuf, ClewError> {
    let path = fs::read_link(format!("/proc/self/fd/{fd}")).map_err(io_error)?;
    if !path.is_absolute() || path.as_os_str().as_bytes().ends_with(b" (deleted)") {
        return Err(invalid("runtime root descriptor has no stable path"));
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
fn descriptor_path(fd: RawFd) -> Result<PathBuf, ClewError> {
    let mut buffer = [0 as libc::c_char; 4096];
    if unsafe { libc::fcntl(fd, libc::F_GETPATH, buffer.as_mut_ptr()) } < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let bytes = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes();
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

fn verify_artifact(root: &Path, artifact: &RuntimeArtifact) -> Result<(), ClewError> {
    if !is_digest(&artifact.sha256) || !matches!(artifact.mode, 0 | 0o111) {
        return Err(invalid("runtime artifact digest is invalid"));
    }
    let path = safe_relative(root, &artifact.path)?;
    let bytes = read_regular(&path, artifact.size, Some(artifact.size))?;
    #[cfg(unix)]
    {
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        let expected = if artifact.mode == 0o111 { 0o500 } else { 0o400 };
        if metadata.permissions().mode() & 0o7777 != expected {
            return Err(invalid("runtime artifact executable mode mismatch"));
        }
    }
    let actual = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    if actual != artifact.sha256 {
        return Err(invalid("runtime artifact digest mismatch"));
    }
    Ok(())
}

fn read_regular(path: &Path, limit: u64, exact_size: Option<u64>) -> Result<Vec<u8>, ClewError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file()
        || metadata.len() > limit
        || exact_size.is_some_and(|size| metadata.len() != size)
    {
        return Err(invalid("runtime file is missing or unsafe"));
    }
    #[cfg(unix)]
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o022 != 0 {
        return Err(invalid("runtime file ownership or mode is unsafe"));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| invalid("runtime file exceeds host address space"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(invalid("runtime file changed while it was read"));
    }
    Ok(bytes)
}

fn safe_relative(root: &Path, relative: &str) -> Result<PathBuf, ClewError> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(invalid("runtime artifact path is not canonical relative"));
    }
    let joined = root.join(path);
    if !joined.starts_with(root) {
        return Err(invalid("runtime artifact path escapes the capsule"));
    }
    Ok(joined)
}

fn tree_digest(manifest: &BTreeMap<String, String>) -> String {
    let mut digest = Sha256::new();
    for (path, identity) in manifest {
        let mut fields = identity.splitn(3, ':');
        let mode = fields.next().unwrap_or("");
        let size = fields.next().unwrap_or("");
        let hash = fields.next().unwrap_or("");
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(mode.as_bytes());
        digest.update([0]);
        digest.update(size.as_bytes());
        digest.update([0]);
        digest.update(hash.as_bytes());
        digest.update([0]);
    }
    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
}

fn is_component_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_' | b'.' | b':')
        })
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::WorkerPreparationRequired, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    internal(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::fd::AsRawFd;

    #[test]
    fn rejects_manifest_with_unbound_digest() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        FileForTest::write(&path, b"{}\n");
        assert!(RuntimeAuthority::load(root.path()).is_err());
    }

    #[test]
    fn worker_tree_digest_matches_distribution_manifest_contract() {
        let manifest = BTreeMap::from([
            (
                "bin/a".to_owned(),
                format!("73:3:sha256:{}", "0".repeat(64)),
            ),
            ("lib/b".to_owned(), format!("0:5:sha256:{}", "1".repeat(64))),
        ]);
        assert_eq!(
            tree_digest(&manifest),
            "sha256:6fd9755d0c290c62d1e09d5f6f13387c889754193b9ff8d30ff15b1e21b6ccdd"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runtime_lease_fd_must_match_the_runtime_key() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("v2");
        let digest = "1".repeat(64);
        let runtime = state.join("runtimes").join(&digest);
        let locks = state.join("locks");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&locks).unwrap();
        let expected = locks.join(format!("runtime-{digest}.lease"));
        let arbitrary = locks.join("arbitrary.lease");
        FileForTest::write(&expected, b"");
        FileForTest::write(&arbitrary, b"");
        fs::set_permissions(&expected, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&arbitrary, fs::Permissions::from_mode(0o600)).unwrap();
        let expected_file = fs::File::open(&expected).unwrap();
        let arbitrary_file = fs::File::open(&arbitrary).unwrap();
        let runtime_key = format!("sha256:{digest}");
        assert!(validate_lease_binding(expected_file.as_raw_fd(), &runtime, &runtime_key).is_ok());
        assert!(
            validate_lease_binding(arbitrary_file.as_raw_fd(), &runtime, &runtime_key).is_err()
        );
    }

    struct FileForTest;
    impl FileForTest {
        fn write(path: &Path, bytes: &[u8]) {
            let mut file = fs::File::create(path).unwrap();
            file.write_all(bytes).unwrap();
        }
    }
}
