//! Current documentation object layout: one SQLite store per documentation root.
use super::{invalid, io_error, sqlite_objects::SqliteObjects, store::Repository};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    os::fd::AsRawFd,
    os::unix::fs::OpenOptionsExt,
    sync::Arc,
};

const MARKER: &str = ".codeclew/cache/object-layout.json";
const SCHEMA: &str = "codeclew-documentation-object-layout/2.0";
const POLICY: &str = "SQLITE_ONLY";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Layout {
    schema: String,
    policy: String,
    store_id: String,
    database: String,
}

struct Barrier(File);
impl Drop for Barrier {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn barrier(repo: &Repository, exclusive: bool) -> Result<Barrier, ClewError> {
    fs::create_dir_all(repo.path(".codeclew/cache")?).map_err(io_error)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(repo.path(".codeclew/cache/object-store.lock")?)
        .map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid(
            "documentation object barrier is not a regular file",
        ));
    }
    let operation = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    if unsafe { libc::flock(file.as_raw_fd(), operation) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(Barrier(file))
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn reindex_required() -> ClewError {
    invalid(
        "DOCS_REINDEX_REQUIRED: documentation root uses an unsupported legacy object store; create a fresh documentation root and reindex its sources",
    )
}

fn read_selected(repo: &Repository) -> Result<Option<Arc<SqliteObjects>>, ClewError> {
    let marker = repo.path(MARKER)?;
    if !marker.try_exists().map_err(io_error)? {
        return Ok(None);
    }
    let layout: Layout = super::store::read(&marker, 4096)?;
    if layout.schema != SCHEMA
        || layout.policy != POLICY
        || uuid::Uuid::parse_str(&layout.store_id).is_err()
        || layout.database != format!(".codeclew/cache/objects-{}.sqlite3", layout.store_id)
    {
        return Err(reindex_required());
    }
    let database = repo.path(&layout.database)?;
    for suffix in ["-wal", "-shm"] {
        repo.path(&format!("{}{suffix}", layout.database))?;
    }
    if !fs::symlink_metadata(&database)
        .map_err(|_| corrupt("selected documentation object database is missing"))?
        .is_file()
    {
        return Err(corrupt(
            "selected documentation object database is not a regular file",
        ));
    }
    let mut cached = repo
        .object_database
        .lock()
        .map_err(|_| corrupt("documentation object store lock failed"))?;
    if let Some((id, store)) = cached.as_ref() {
        if *id != layout.store_id {
            return Err(corrupt(
                "documentation object database identity changed during use",
            ));
        }
        return Ok(Some(Arc::clone(store)));
    }
    let store = Arc::new(SqliteObjects::open_existing(&database, &layout.store_id)?);
    *cached = Some((layout.store_id, Arc::clone(&store)));
    Ok(Some(store))
}

pub(crate) fn ensure_current(repo: &Repository) -> Result<(), ClewError> {
    if read_selected(repo)?.is_none() {
        return Err(reindex_required());
    }
    Ok(())
}

/// All object operations use the current SQLite store under the shared barrier.
pub(super) fn with_store<T>(
    repo: &Repository,
    operation: impl FnOnce(&SqliteObjects) -> Result<T, ClewError>,
) -> Result<T, ClewError> {
    let _barrier = barrier(repo, false)?;
    let store = read_selected(repo)?.ok_or_else(reindex_required)?;
    operation(&store)
}

fn activate_locked(repo: &Repository) -> Result<Arc<SqliteObjects>, ClewError> {
    if let Some(store) = read_selected(repo)? {
        return Ok(store);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let layout = Layout {
        schema: SCHEMA.into(),
        policy: POLICY.into(),
        database: format!(".codeclew/cache/objects-{id}.sqlite3"),
        store_id: id.clone(),
    };
    let store = Arc::new(SqliteObjects::create(&repo.path(&layout.database)?, &id)?);
    #[cfg(test)]
    if std::env::var_os("CODECLEW_ABORT_AFTER_SQLITE_OBJECT_CREATE").is_some() {
        std::process::abort();
    }
    File::open(repo.path(".codeclew/cache")?)
        .and_then(|file| file.sync_all())
        .map_err(io_error)?;
    repo.atomic(MARKER, &super::bytes(&layout)?)?;
    *repo
        .object_database
        .lock()
        .map_err(|_| corrupt("documentation object store lock failed"))? =
        Some((id, Arc::clone(&store)));
    Ok(store)
}

pub(crate) fn activate(repo: &Repository) -> Result<(), ClewError> {
    let _barrier = barrier(repo, true)?;
    activate_locked(repo).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::cache;
    use std::process::{Command, Stdio};

    fn setup() -> (tempfile::TempDir, Repository) {
        let root = tempfile::tempdir().unwrap();
        Repository::init(root.path(), "Storage fixture").unwrap();
        let repo = Repository::open(root.path()).unwrap();
        (root, repo)
    }

    fn layout(repo: &Repository) -> Layout {
        super::super::store::read(&repo.path(MARKER).unwrap(), 4096).unwrap()
    }

    #[test]
    fn new_repository_defaults_to_sqlite_and_reopens() {
        let (root, repo) = setup();
        let object = cache::put(&repo, "test", b"stored in sqlite").unwrap();
        let current = layout(&repo);
        assert_eq!(current.schema, SCHEMA);
        assert_eq!(current.policy, POLICY);
        assert!(repo.root.join(&current.database).is_file());
        drop(repo);
        let reopened = Repository::open(root.path()).unwrap();
        assert_eq!(
            cache::get(&reopened, &object, 1024).unwrap(),
            Some(b"stored in sqlite".to_vec())
        );
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_new_root_activation_retries_after_process_kill() {
        if let Some(path) = std::env::var_os("CODECLEW_CRASH_ROOT") {
            Repository::init(std::path::Path::new(&path), "Storage fixture").unwrap();
            std::process::abort();
        }
        let root = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("interrupted_new_root_activation_retries_after_process_kill")
            .env("CODECLEW_CRASH_ROOT", root.path())
            .env("CODECLEW_ABORT_AFTER_SQLITE_OBJECT_CREATE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(!child.wait().unwrap().success());
        assert!(!root.path().join("codeclew-docs.yaml").exists());
        assert!(!root.path().join(MARKER).exists());
        fs::remove_file(root.path().join(".codeclew/write.lock")).unwrap();
        let initialized = Repository::init(root.path(), "Storage fixture").unwrap();
        assert_eq!(initialized["status"], "READY");
        let repo = Repository::open(root.path()).unwrap();
        assert!(repo.root.join(&layout(&repo).database).is_file());
    }

    #[test]
    fn markerless_legacy_root_is_rejected_without_deleting_payload() {
        let (root, repo) = setup();
        let payload = b"legacy payload that must remain";
        let digest = cache::content_digest(payload);
        let legacy = repo
            .root
            .join(cache::OBJECT_ROOT)
            .join(&digest)
            .join("object.json");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, payload).unwrap();
        fs::remove_file(repo.path(MARKER).unwrap()).unwrap();
        drop(repo);
        let error = Repository::open(root.path()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("fresh documentation root"));
        assert_eq!(fs::read(legacy).unwrap(), payload);
    }

    #[test]
    fn selected_database_missing_is_rejected_without_recreation() {
        let (_root, repo) = setup();
        let database = repo.root.join(&layout(&repo).database);
        fs::remove_file(&database).unwrap();
        let error = cache::owned_digests(&repo, 100).unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
        assert!(!database.exists());
    }
}
