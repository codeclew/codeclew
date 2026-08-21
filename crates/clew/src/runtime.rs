use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const RUNTIME_SCHEMA: &str = "codeclew-runtime-capsule/1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAuthority {
    pub schema: String,
    pub runtime_key: String,
    pub mode: RuntimeMode,
    pub manifest_digest: String,
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
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWorker {
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
        let manifest_path = canonical_root.join("runtime.json");
        let manifest_metadata = fs::symlink_metadata(&manifest_path).map_err(io_error)?;
        if manifest_metadata.file_type().is_symlink()
            || !manifest_metadata.is_file()
            || manifest_metadata.len() > 1024 * 1024
        {
            return Err(invalid("runtime manifest is missing or unsafe"));
        }
        let bytes = fs::read(&manifest_path).map_err(io_error)?;
        let mut authority: RuntimeAuthority = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(&format!("runtime manifest is invalid: {error}")))?;
        if authority.schema != RUNTIME_SCHEMA
            || !is_digest(&authority.runtime_key)
            || !is_digest(&authority.manifest_digest)
        {
            return Err(invalid("runtime manifest identity is invalid"));
        }
        let expected_digest = authority.manifest_digest.clone();
        authority.manifest_digest.clear();
        let actual_digest = canonical::hash(&authority).map_err(internal)?;
        if expected_digest != actual_digest {
            return Err(invalid("runtime manifest digest mismatch"));
        }
        authority.manifest_digest = expected_digest;
        authority.root = canonical_root;
        for artifact in authority.artifacts.values() {
            verify_artifact(&authority.root, artifact)?;
        }
        Ok(authority)
    }

    pub fn from_environment() -> Result<Option<Self>, ClewError> {
        let Some(value) = std::env::var_os("CODECLEW_RUNTIME_ROOT") else {
            return Ok(None);
        };
        if value.is_empty() {
            return Err(invalid("CODECLEW_RUNTIME_ROOT cannot be empty"));
        }
        Self::load(&PathBuf::from(value)).map(Some)
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
                format!("{}:{}", artifact.size, artifact.sha256),
            );
        }
        if tree_digest(&manifest) != worker.tree_hash {
            return Err(invalid("runtime worker tree digest mismatch"));
        }
        Ok(root)
    }
}

fn verify_artifact(root: &Path, artifact: &RuntimeArtifact) -> Result<(), ClewError> {
    if !is_digest(&artifact.sha256) {
        return Err(invalid("runtime artifact digest is invalid"));
    }
    let path = safe_relative(root, &artifact.path)?;
    let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != artifact.size {
        return Err(invalid("runtime artifact is missing or unsafe"));
    }
    let bytes = fs::read(path).map_err(io_error)?;
    let actual = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    if actual != artifact.sha256 {
        return Err(invalid("runtime artifact digest mismatch"));
    }
    Ok(())
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
        let (size, hash) = identity.split_once(':').unwrap_or(("", identity));
        digest.update(path.as_bytes());
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
            ("bin/a".to_owned(), format!("3:sha256:{}", "0".repeat(64))),
            ("lib/b".to_owned(), format!("5:sha256:{}", "1".repeat(64))),
        ]);
        assert_eq!(
            tree_digest(&manifest),
            "sha256:17991e194c0c77b4a7ff59263df0339e2a26c7e8bc5556e11a3afeb2510c6177"
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
