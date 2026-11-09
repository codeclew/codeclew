//! Current documentation object layout: one SQLite store per documentation root.
use super::{invalid, io_error, sqlite_objects::SqliteObjects, store::Repository};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
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

pub(super) fn state_problem(reason: &str, path: &str, message: &str) -> ClewError {
    let mut error = corrupt(format!("{reason}: {message}"));
    error.details = Some(Box::new(
        json!({"reason":reason,"path":path,"automaticRepair":false,
        "nextAction":"Preserve authored files and local state. Restore a consistent backup into a separate directory, or create a separate fresh root and explicitly reindex available sources. Fresh indexing does not recover the deleted snapshot. See docs/operations/documentation-state-recovery.md."}),
    ));
    error
}

fn missing_marker(repo: &Repository) -> Result<ClewError, ClewError> {
    let cache = repo.path(".codeclew/cache")?;
    let modern = if cache.is_dir() {
        fs::read_dir(cache)
            .map_err(io_error)?
            .take(1024)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error)?
            .iter()
            .any(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("objects-") && name.ends_with(".sqlite3")
            })
    } else {
        false
    };
    let legacy = repo
        .path(super::cache::OBJECT_ROOT)?
        .try_exists()
        .map_err(io_error)?;
    let reason = if modern {
        "OBJECT_LAYOUT_MARKER_MISSING"
    } else {
        "OBJECT_LAYOUT_UNDETERMINED"
    };
    let mut error = state_problem(
        reason,
        MARKER,
        if modern {
            "modern database files exist but the selecting marker is absent; do not select a database by guessing"
        } else {
            "the selecting marker is absent; legacy layout and damaged/incomplete modern state cannot be distinguished"
        },
    );
    let details = error.details.as_mut().unwrap();
    details["modernDatabaseFilesObserved"] = json!(modern);
    details["legacyObjectDirectoryPresent"] = json!(legacy);
    details["classification"] = json!(if modern {
        "DAMAGED_MODERN_LAYOUT"
    } else {
        "AMBIGUOUS_LEGACY_OR_INCOMPLETE_LAYOUT"
    });
    Ok(error)
}

fn read_selected(repo: &Repository) -> Result<Option<Arc<SqliteObjects>>, ClewError> {
    let marker = repo.path(MARKER)?;
    if !marker.try_exists().map_err(io_error)? {
        return Ok(None);
    }
    let layout: Layout = super::store::read(&marker, 4096).map_err(|_| {
        state_problem(
            "OBJECT_LAYOUT_MARKER_INVALID",
            MARKER,
            "the selecting marker is unreadable or violates its closed schema",
        )
    })?;
    if layout.schema != SCHEMA
        || layout.policy != POLICY
        || uuid::Uuid::parse_str(&layout.store_id).is_err()
        || layout.database != format!(".codeclew/cache/objects-{}.sqlite3", layout.store_id)
    {
        return Err(state_problem(
            "OBJECT_LAYOUT_UNSUPPORTED",
            MARKER,
            "the recorded layout version, policy or database identity is unsupported; this does not establish a legacy layout",
        ));
    }
    let database = repo.path(&layout.database)?;
    for suffix in ["-wal", "-shm"] {
        repo.path(&format!("{}{suffix}", layout.database))?;
    }
    if !fs::symlink_metadata(&database)
        .map_err(|_| {
            state_problem(
                "OBJECT_DATABASE_MISSING",
                &layout.database,
                "the database selected by the marker is missing or inaccessible",
            )
        })?
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
    let store = Arc::new(
        SqliteObjects::open_existing(&database, &layout.store_id).map_err(|mut error| {
            error.code = ErrorCode::StateCorrupt;
            error.details = state_problem(
                "OBJECT_DATABASE_INVALID",
                &layout.database,
                "selected database is corrupt or incompatible; preserve it for diagnosis",
            )
            .details;
            error
        })?,
    );
    *cached = Some((layout.store_id, Arc::clone(&store)));
    Ok(Some(store))
}

pub(crate) fn ensure_current(repo: &Repository) -> Result<(), ClewError> {
    if read_selected(repo)?.is_none() {
        return Err(missing_marker(repo)?);
    }
    Ok(())
}

/// All object operations use the current SQLite store under the shared barrier.
pub(super) fn with_store<T>(
    repo: &Repository,
    operation: impl FnOnce(&SqliteObjects) -> Result<T, ClewError>,
) -> Result<T, ClewError> {
    let _barrier = barrier(repo, false)?;
    let store = match read_selected(repo)? {
        Some(store) => store,
        None => return Err(missing_marker(repo)?),
    };
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
    fn docs_state_classifies_uninitialized_missing_marker_and_corrupt_database() {
        let empty = tempfile::tempdir().unwrap();
        let e = Repository::open(empty.path()).unwrap_err();
        assert_eq!(e.details.unwrap()["reason"], "ROOT_NOT_INITIALIZED");
        assert!(!empty.path().join(".codeclew").exists());
        let (root, repo) = setup();
        let database = repo.root.join(&layout(&repo).database);
        fs::remove_file(repo.path(MARKER).unwrap()).unwrap();
        drop(repo);
        let e = Repository::open(root.path()).unwrap_err();
        assert_eq!(e.details.unwrap()["reason"], "OBJECT_LAYOUT_MARKER_MISSING");
        assert!(database.exists());
        assert!(!root.path().join(MARKER).exists());
        let (root, repo) = setup();
        let database = repo.root.join(&layout(&repo).database);
        drop(repo);
        fs::write(&database, b"invalid sqlite database").unwrap();
        let original = fs::read(&database).unwrap();
        let e = Repository::open(root.path()).unwrap_err();
        assert_eq!(e.code, ErrorCode::StateCorrupt);
        assert_eq!(e.details.unwrap()["reason"], "OBJECT_DATABASE_INVALID");
        assert_eq!(fs::read(database).unwrap(), original);
    }

    #[test]
    fn docs_state_absent_marker_without_database_stays_ambiguous() {
        let (root, repo) = setup();
        let database = repo.root.join(&layout(&repo).database);
        drop(repo);
        fs::remove_file(root.path().join(MARKER)).unwrap();
        fs::remove_file(database).unwrap();
        let e = Repository::open(root.path()).unwrap_err();
        assert_eq!(
            e.details.unwrap()["classification"],
            "AMBIGUOUS_LEGACY_OR_INCOMPLETE_LAYOUT"
        );
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
        use std::os::unix::fs::MetadataExt;

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
        let lock_path = root.path().join(".codeclew/write.lock");
        let crashed_lock = fs::metadata(&lock_path).unwrap();
        let initialized = Repository::init(root.path(), "Storage fixture").unwrap();
        assert_eq!(initialized["status"], "READY");
        let retried_lock = fs::metadata(&lock_path).unwrap();
        assert_eq!(retried_lock.dev(), crashed_lock.dev());
        assert_eq!(retried_lock.ino(), crashed_lock.ino());
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
        assert_eq!(error.code, ErrorCode::StateCorrupt);
        assert_eq!(
            error.details.unwrap()["reason"],
            "OBJECT_LAYOUT_MARKER_MISSING"
        );
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
