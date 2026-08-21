use crate::error::{ClewError, ErrorCode};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

pub const STATE_SCHEMA: &str = "codeclew-state-authority/2.0";

#[derive(Debug, Clone)]
pub struct StateAuthority {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RepositoryState {
    pub key: String,
    pub root: PathBuf,
    pub repository_index: PathBuf,
    pub blobs: PathBuf,
    pub model_cache: PathBuf,
    pub compiler_index: PathBuf,
    pub ledger: PathBuf,
}

impl StateAuthority {
    pub fn process_default() -> Result<Self, ClewError> {
        let root = if let Some(explicit) = std::env::var_os("CODECLEW_HOME") {
            let path = PathBuf::from(explicit);
            if !path.is_absolute() {
                return Err(invalid("CODECLEW_HOME must be absolute"));
            }
            path
        } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
            let path = PathBuf::from(xdg);
            if !path.is_absolute() {
                return Err(invalid("XDG_CACHE_HOME must be absolute"));
            }
            path.join("codeclew")
        } else {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| invalid("HOME is unavailable and CODECLEW_HOME is unset"))?;
            if !home.is_absolute() {
                return Err(invalid("HOME must be absolute"));
            }
            home.join(".cache/codeclew")
        };
        Self::open(root.join("v2"))
    }

    pub fn open(root: PathBuf) -> Result<Self, ClewError> {
        if !root.is_absolute()
            || root
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err(invalid("state root must use normalized absolute spelling"));
        }
        create_private_directory(&root)?;
        let metadata = fs::symlink_metadata(&root).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("state root must be a real directory"));
        }
        #[cfg(unix)]
        {
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err(invalid("state root must be owned by the current user"));
            }
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(invalid(
                    "state root must not be accessible by group or world",
                ));
            }
        }
        for child in [
            "runtimes",
            "repos",
            "sessions",
            "runs",
            "locks",
            "tmp",
            "quarantine",
            "objects",
            "objects/sha256",
            "generations",
            "attempts",
            "gc",
        ] {
            create_private_directory(&root.join(child))?;
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn objects_root(&self) -> PathBuf {
        self.root.join("objects/sha256")
    }

    pub fn locks_root(&self) -> PathBuf {
        self.root.join("locks")
    }

    pub fn quarantine_root(&self) -> PathBuf {
        self.root.join("quarantine")
    }

    pub fn repository(&self, repo: &Path) -> Result<RepositoryState, ClewError> {
        let canonical = repo.canonicalize().map_err(io_error)?;
        let key = repository_key(&canonical)?;
        let root = self.root.join("repos").join(&key);
        let repository_index = root.join("repository-index");
        let blobs = root.join("blobs/sha256");
        let model_cache = root.join("model-cache");
        let compiler_index = root.join("compiler-index");
        for directory in [
            &root,
            &repository_index,
            &blobs,
            &model_cache,
            &compiler_index,
        ] {
            create_private_directory(directory)?;
        }
        Ok(RepositoryState {
            key,
            ledger: root.join("ledger.sqlite3"),
            root,
            repository_index,
            blobs,
            model_cache,
            compiler_index,
        })
    }

    pub fn create_private_file(&self, relative: &Path) -> Result<File, ClewError> {
        let path = self.root.join(relative);
        if !path.starts_with(&self.root) {
            return Err(invalid("managed state file escapes its authority root"));
        }
        if let Some(parent) = path.parent() {
            create_private_directory(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        options.open(path).map_err(io_error)
    }

    pub fn session_root(&self, session_id: &str) -> Result<PathBuf, ClewError> {
        let name = managed_id_component(session_id, "session:")?;
        let path = self.root.join("sessions").join(name);
        create_private_directory(&path)?;
        Ok(path)
    }

    pub fn run_root(&self, run_id: &str) -> Result<PathBuf, ClewError> {
        let name = managed_id_component(run_id, "run:")?;
        let path = self.root.join("runs").join(name);
        create_private_directory(&path)?;
        Ok(path)
    }

    pub fn write_private_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), ClewError> {
        if !path.starts_with(&self.root) {
            return Err(invalid("managed state file escapes its authority root"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| invalid("managed state file has no parent"))?;
        create_private_directory(parent)?;
        let temporary = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::rename(&temporary, path).map_err(io_error)?;
        Ok(())
    }
}

fn managed_id_component<'a>(value: &'a str, prefix: &str) -> Result<&'a str, ClewError> {
    let component = value
        .strip_prefix(prefix)
        .ok_or_else(|| invalid("managed identifier has the wrong prefix"))?;
    if component.is_empty()
        || component.len() > 128
        || !component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(invalid("managed identifier is not a safe path component"));
    }
    Ok(component)
}

fn repository_key(repo: &Path) -> Result<String, ClewError> {
    let common = git(repo, &["rev-parse", "--git-common-dir"])
        .ok()
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repo.join(path)
            }
        })
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| repo.to_path_buf());
    let metadata = fs::metadata(&common).map_err(io_error)?;
    let object_format =
        git(repo, &["rev-parse", "--show-object-format"]).unwrap_or_else(|_| "sha1".to_owned());
    let root_commit = git(repo, &["rev-list", "--max-parents=0", "HEAD"])
        .ok()
        .and_then(|value| value.lines().next().map(str::to_owned))
        .unwrap_or_else(|| "NO_GIT_ROOT".to_owned());
    let mut digest = Sha256::new();
    digest.update(b"codeclew-repo/v1\0");
    #[cfg(unix)]
    {
        digest.update(metadata.dev().to_be_bytes());
        digest.update(metadata.ino().to_be_bytes());
    }
    #[cfg(not(unix))]
    digest.update(common.as_os_str().as_encoded_bytes());
    digest.update([0]);
    digest.update(object_format.as_bytes());
    digest.update([0]);
    digest.update(root_commit.as_bytes());
    Ok(hex::encode(digest.finalize()))
}

fn git(repo: &Path, arguments: &[&str]) -> Result<String, ClewError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("repository identity is unavailable"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| invalid("Git repository identity is not UTF-8"))
}

pub(crate) fn create_private_directory(path: &Path) -> Result<(), ClewError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("managed state path is not a real directory"));
        }
    } else {
        fs::create_dir_all(path).map_err(io_error)?;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    Ok(())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_state_is_external_private_and_path_free() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(home.path().join("state")).unwrap();
        let repository = state.repository(repo.path()).unwrap();
        assert!(repository.root.starts_with(state.root()));
        assert!(!repository.root.starts_with(repo.path()));
        assert_eq!(repository.key.len(), 64);
        assert!(
            !repository
                .key
                .contains(repo.path().to_string_lossy().as_ref())
        );
    }
}
