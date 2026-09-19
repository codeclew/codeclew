//! Durable payload-only CAS backed by SQLite.

use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const SQLITE_OBJECTS_SCHEMA: &str = "codeclew-sqlite-objects/1.0";
const BUSY_TIMEOUT: Duration = Duration::from_millis(2_500);
const MAX_STORE_ID_BYTES: usize = 1024;
const META_SCHEMA: &str = "schemaVersion";
const META_STORE_ID: &str = "storeId";

/// A durable payload-only CAS backed by one SQLite database.
///
/// Callers own path validation and lifecycle placement. This type binds every
/// opened database to a schema marker and store identity before serving reads.
#[derive(Debug)]
pub(crate) struct SqliteObjects {
    connection: Mutex<Connection>,
    store_id: String,
}

impl SqliteObjects {
    pub(crate) fn create(path: &Path, store_id: &str) -> Result<Self, ClewError> {
        validate_store_id(store_id)?;
        let existed = path.exists();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | if existed {
                OpenFlags::empty()
            } else {
                OpenFlags::SQLITE_OPEN_CREATE
            };
        let connection = open_raw_connection(path, flags)?;
        if existed {
            verify_metadata(&connection, store_id)?;
            configure_connection(&connection)?;
        } else {
            configure_connection(&connection)?;
            initialize_schema(&connection)?;
            write_metadata(&connection, store_id)?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
            store_id: store_id.to_owned(),
        })
    }

    pub(crate) fn open_existing(path: &Path, store_id: &str) -> Result<Self, ClewError> {
        validate_store_id(store_id)?;
        if !path.is_file() {
            return Err(corrupt(format!(
                "SQLite object database is missing: {}",
                path.display()
            )));
        }
        let connection = open_raw_connection(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        verify_metadata(&connection, store_id)?;
        configure_connection(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            store_id: store_id.to_owned(),
        })
    }

    pub(crate) fn put_many(&self, objects: &[(&str, &[u8])]) -> Result<usize, ClewError> {
        for (digest, payload) in objects {
            validate_payload_digest(digest, payload)?;
        }
        if objects.is_empty() {
            return Ok(0);
        }

        let mut connection = self.lock_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sql_error("beginning SQLite object transaction", error))?;
        let mut inserted = 0usize;
        for (digest, payload) in objects {
            let existing = transaction
                .query_row(
                    "SELECT byte_len, length(payload) FROM objects WHERE digest = ?1",
                    params![digest],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()
                .map_err(|error| sql_error("checking SQLite object", error))?;
            if let Some((stored_size, actual_size)) = existing {
                verify_stored_lengths(digest, stored_size, actual_size)?;
                if u64::try_from(stored_size).ok() != Some(payload.len() as u64) {
                    return Err(corrupt(format!(
                        "SQLite object digest has conflicting payload size: {digest}"
                    )));
                }
                let existing_payload: Vec<u8> = transaction
                    .query_row(
                        "SELECT payload FROM objects WHERE digest = ?1",
                        params![digest],
                        |row| row.get(0),
                    )
                    .map_err(|error| sql_error("reading conflicting SQLite object", error))?;
                verify_row(digest, &existing_payload, stored_size)?;
                if existing_payload != *payload {
                    return Err(corrupt(format!(
                        "SQLite object digest has conflicting payload: {digest}"
                    )));
                }
                continue;
            }
            transaction
                .execute(
                    "INSERT INTO objects (digest, payload, byte_len) VALUES (?1, ?2, ?3)",
                    params![digest, payload, payload.len() as i64],
                )
                .map_err(|error| sql_error("inserting SQLite object", error))?;
            inserted += 1;
        }
        transaction
            .commit()
            .map_err(|error| sql_error("committing SQLite object transaction", error))?;
        Ok(inserted)
    }

    pub(crate) fn read(
        &self,
        digest: &str,
        expected_size: u64,
        limit: u64,
    ) -> Result<Option<Vec<u8>>, ClewError> {
        validate_digest(digest)?;
        if expected_size > limit {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "SQLite object exceeds the caller's read budget",
            ));
        }
        let mut connection = self.lock_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| sql_error("beginning SQLite object read", error))?;
        let row = transaction
            .query_row(
                "SELECT byte_len, length(payload) FROM objects WHERE digest = ?1",
                params![digest],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| sql_error("reading SQLite object metadata", error))?;
        let Some((stored_size, actual_size)) = row else {
            transaction
                .commit()
                .map_err(|error| sql_error("committing SQLite missing-object read", error))?;
            return Ok(None);
        };
        verify_stored_lengths(digest, stored_size, actual_size)?;
        let stored_size =
            u64::try_from(stored_size).map_err(|_| corrupt("SQLite object has a negative size"))?;
        if stored_size != expected_size || stored_size > limit {
            return Err(corrupt(format!(
                "SQLite object size mismatch for {digest}: stored={stored_size}, expected={expected_size}"
            )));
        }
        let payload: Vec<u8> = transaction
            .query_row(
                "SELECT payload FROM objects WHERE digest = ?1",
                params![digest],
                |row| row.get(0),
            )
            .map_err(|error| sql_error("reading SQLite object payload", error))?;
        verify_row(digest, &payload, stored_size as i64)?;
        transaction
            .commit()
            .map_err(|error| sql_error("committing SQLite object read", error))?;
        Ok(Some(payload))
    }

    #[cfg(test)]
    pub(crate) fn inventory(&self) -> Result<(u64, u64), ClewError> {
        let connection = self.lock_connection()?;
        let (count, bytes, negative_sizes): (i64, i64, i64) = connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(byte_len), 0), COALESCE(SUM(CASE WHEN byte_len < 0 THEN 1 ELSE 0 END), 0) FROM objects",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| sql_error("inventorying SQLite objects", error))?;
        if count < 0 || bytes < 0 || negative_sizes != 0 {
            return Err(corrupt("SQLite object inventory contains malformed rows"));
        }
        Ok((count as u64, bytes as u64))
    }

    pub(crate) fn enumerate(&self, limit: usize) -> Result<Vec<(String, u64)>, ClewError> {
        let requested = if limit == usize::MAX {
            i64::MAX
        } else {
            i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX)
        };
        let connection = self.lock_connection()?;
        let mut statement = connection
            .prepare(
                "SELECT digest, byte_len, length(payload) FROM objects ORDER BY rowid LIMIT ?1",
            )
            .map_err(|error| sql_error("preparing SQLite object enumeration", error))?;
        let rows = statement
            .query_map(params![requested], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| sql_error("enumerating SQLite objects", error))?;
        let mut objects = Vec::new();
        for row in rows {
            let (digest, stored_size, actual_size) =
                row.map_err(|error| sql_error("reading SQLite enumeration row", error))?;
            validate_digest(&digest)?;
            verify_stored_lengths(&digest, stored_size, actual_size)?;
            let size = u64::try_from(stored_size)
                .map_err(|_| corrupt("SQLite object has a negative size"))?;
            objects.push((digest, size));
            if objects.len() > limit {
                return Err(ClewError::new(
                    ErrorCode::ResourceLimit,
                    "SQLite object enumeration exceeds the caller's limit",
                ));
            }
        }
        Ok(objects)
    }
}

fn open_raw_connection(path: &Path, flags: OpenFlags) -> Result<Connection, ClewError> {
    let connection = Connection::open_with_flags(path, flags)
        .map_err(|error| sql_error("opening SQLite object database", error))?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| sql_error("setting SQLite busy timeout", error))?;
    Ok(connection)
}

fn configure_connection(connection: &Connection) -> Result<(), ClewError> {
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|error| sql_error("enabling SQLite WAL", error))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(ClewError::new(
            ErrorCode::Internal,
            format!("SQLite refused WAL journal mode: {journal_mode}"),
        ));
    }
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| sql_error("setting SQLite full synchronous mode", error))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| sql_error("enabling SQLite foreign keys", error))?;
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<(), ClewError> {
    connection
        .execute_batch(
            "CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);\
             CREATE TABLE objects (payload BLOB NOT NULL, byte_len INTEGER NOT NULL, digest TEXT NOT NULL);\
             CREATE UNIQUE INDEX objects_digest ON objects(digest);",
        )
        .map_err(|error| sql_error("initializing SQLite object schema", error))
}

fn write_metadata(connection: &Connection, store_id: &str) -> Result<(), ClewError> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| sql_error("starting SQLite metadata transaction", error))?;
    transaction
        .execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2), (?3, ?4)",
            params![META_SCHEMA, SQLITE_OBJECTS_SCHEMA, META_STORE_ID, store_id],
        )
        .map_err(|error| sql_error("writing SQLite object metadata", error))?;
    transaction
        .commit()
        .map_err(|error| sql_error("committing SQLite metadata", error))
}

fn verify_metadata(connection: &Connection, store_id: &str) -> Result<(), ClewError> {
    let schema: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            params![META_SCHEMA],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| corrupt(format!("reading SQLite schema metadata: {error}")))?;
    let persisted_store: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            params![META_STORE_ID],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| corrupt(format!("reading SQLite store metadata: {error}")))?;
    if schema.as_deref() != Some(SQLITE_OBJECTS_SCHEMA) {
        return Err(corrupt(
            "SQLite object schema marker is missing or unsupported",
        ));
    }
    if persisted_store.as_deref() != Some(store_id) {
        return Err(corrupt("SQLite object store identity does not match"));
    }
    Ok(())
}

fn validate_store_id(store_id: &str) -> Result<(), ClewError> {
    if store_id.is_empty() || store_id.len() > MAX_STORE_ID_BYTES || store_id.contains('\0') {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "SQLite object store identity is empty, too long, or contains NUL",
        ));
    }
    Ok(())
}

fn validate_payload_digest(digest: &str, payload: &[u8]) -> Result<(), ClewError> {
    validate_digest(digest)?;
    if canonical::hash_bytes(payload) != digest {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("SQLite object payload does not match digest: {digest}"),
        ));
    }
    Ok(())
}

fn validate_digest(digest: &str) -> Result<(), ClewError> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "SQLite object digest must use sha256:<lowercase-hex>",
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "SQLite object digest must use sha256:<lowercase-hex>",
        ));
    }
    Ok(())
}

fn verify_stored_lengths(
    digest: &str,
    stored_size: i64,
    actual_size: i64,
) -> Result<(), ClewError> {
    if stored_size < 0 || actual_size < 0 || stored_size != actual_size {
        return Err(corrupt(format!("SQLite object size is corrupt: {digest}")));
    }
    Ok(())
}

fn verify_row(digest: &str, payload: &[u8], stored_size: i64) -> Result<(), ClewError> {
    let size = u64::try_from(stored_size)
        .map_err(|_| corrupt(format!("SQLite object has a negative size: {digest}")))?;
    if size != payload.len() as u64 {
        return Err(corrupt(format!("SQLite object size is corrupt: {digest}")));
    }
    if canonical::hash_bytes(payload) != digest {
        return Err(corrupt(format!(
            "SQLite object payload hash is corrupt: {digest}"
        )));
    }
    Ok(())
}

fn lock_error() -> ClewError {
    ClewError::new(
        ErrorCode::Internal,
        "SQLite object connection mutex is poisoned",
    )
}

fn sql_error(context: &str, error: rusqlite::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, format!("{context}: {error}"))
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

impl SqliteObjects {
    fn lock_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, ClewError> {
        self.connection.lock().map_err(|_| lock_error())
    }

    #[allow(dead_code)]
    pub(crate) fn store_id(&self) -> &str {
        &self.store_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    fn object(payload: &[u8]) -> (String, Vec<u8>) {
        (canonical::hash_bytes(payload), payload.to_vec())
    }

    #[test]
    fn atomic_batch_rolls_back_after_corrupt_existing_row() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("objects.sqlite");
        let store = SqliteObjects::create(&database, "test-store").unwrap();
        let (digest, payload) = object(b"original");
        assert_eq!(store.put_many(&[(&digest, &payload)]).unwrap(), 1);
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE objects SET payload = ?1, byte_len = ?2 WHERE digest = ?3",
                    params![b"corrupt".as_slice(), 7i64, &digest],
                )
                .unwrap();
        }
        let (new_digest, new_payload) = object(b"new");
        let result = store.put_many(&[(&new_digest, &new_payload), (&digest, &payload)]);
        assert!(result.is_err());
        assert_eq!(store.inventory().unwrap().0, 1);
        assert_eq!(store.read(&new_digest, 3, 3).unwrap(), None);
    }

    #[test]
    fn reopen_requires_store_identity_and_preserves_payload() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("objects.sqlite");
        let (digest, payload) = object(b"persisted");
        {
            let store = SqliteObjects::create(&database, "store-a").unwrap();
            assert_eq!(store.put_many(&[(&digest, &payload)]).unwrap(), 1);
        }
        let reopened = SqliteObjects::open_existing(&database, "store-a").unwrap();
        assert_eq!(reopened.read(&digest, 9, 9).unwrap(), Some(payload));
        assert!(SqliteObjects::open_existing(&database, "store-b").is_err());
        assert_eq!(reopened.inventory().unwrap(), (1, 9));
    }

    #[test]
    fn same_payload_digest_is_independent_between_store_databases() {
        let directory = tempdir().unwrap();
        let (digest, payload) = object(b"same");
        let first = SqliteObjects::create(&directory.path().join("first.sqlite"), "first").unwrap();
        let second =
            SqliteObjects::create(&directory.path().join("second.sqlite"), "second").unwrap();
        assert_eq!(first.put_many(&[(&digest, &payload)]).unwrap(), 1);
        assert_eq!(second.put_many(&[(&digest, &payload)]).unwrap(), 1);
        assert_eq!(first.read(&digest, 4, 4).unwrap(), Some(payload.clone()));
        assert_eq!(second.read(&digest, 4, 4).unwrap(), Some(payload));
    }

    #[test]
    fn bounded_read_and_enumeration_are_explicit() {
        let directory = tempdir().unwrap();
        let store =
            SqliteObjects::create(&directory.path().join("objects.sqlite"), "store").unwrap();
        let (first_digest, first_payload) = object(b"first");
        let (second_digest, second_payload) = object(b"second");
        store
            .put_many(&[
                (&first_digest, &first_payload),
                (&second_digest, &second_payload),
            ])
            .unwrap();
        assert!(matches!(
            store.read(&first_digest, 5, 4),
            Err(ClewError {
                code: ErrorCode::ResourceLimit,
                ..
            })
        ));
        assert_eq!(store.enumerate(2).unwrap().len(), 2);
        assert_eq!(
            store.enumerate(1).unwrap_err().code,
            ErrorCode::ResourceLimit
        );
    }

    #[test]
    fn missing_database_is_not_created_by_open() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("missing.sqlite");
        assert!(SqliteObjects::open_existing(&database, "store").is_err());
        assert!(!database.exists());
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_sqlite_transaction_rolls_back_on_process_kill() {
        if let Some(database) = std::env::var_os("CODECLEW_SQLITE_TX_DB") {
            let database = std::path::PathBuf::from(database);
            let ready =
                std::path::PathBuf::from(std::env::var_os("CODECLEW_SQLITE_TX_READY").unwrap());
            let payload = b"transaction payload";
            let digest = canonical::hash_bytes(payload);
            let connection = Connection::open(&database).unwrap();
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            connection
                .execute(
                    "INSERT INTO objects (digest, payload, byte_len) VALUES (?1, ?2, ?3)",
                    params![&digest, payload.as_slice(), payload.len() as i64],
                )
                .unwrap();
            fs::write(ready, b"transaction-open").unwrap();
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }

        let directory = tempdir().unwrap();
        let database = directory.path().join("objects.sqlite");
        let store = SqliteObjects::create(&database, "store").unwrap();
        let ready = directory.path().join("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("interrupted_sqlite_transaction_rolls_back_on_process_kill")
            .env("CODECLEW_SQLITE_TX_DB", &database)
            .env("CODECLEW_SQLITE_TX_READY", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            ready.exists(),
            "child did not publish transaction checkpoint"
        );
        child.kill().unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());

        let payload = b"transaction payload";
        let digest = canonical::hash_bytes(payload);
        assert_eq!(
            store.read(&digest, payload.len() as u64, 1024).unwrap(),
            None
        );
        assert_eq!(store.put_many(&[(&digest, payload.as_slice())]).unwrap(), 1);
        assert_eq!(
            store.read(&digest, payload.len() as u64, 1024).unwrap(),
            Some(payload.to_vec())
        );
    }

    #[derive(Debug, Clone, Copy, serde::Serialize)]
    struct BenchmarkObservation {
        bytes: u64,
        #[serde(rename = "inodeCount")]
        inode_count: u64,
        #[serde(rename = "rssBytes")]
        rss_bytes: Option<u64>,
    }

    fn benchmark_payload(index: usize) -> Vec<u8> {
        if index.is_multiple_of(10) {
            let shared = index % 256;
            let size = match shared % 7 {
                0 => 17,
                1 => 127,
                2 => 512,
                3 => 2_048,
                4 => 4_096,
                5 => 16_384,
                _ => 65_536,
            };
            return deterministic_bytes(0x51_0000usize + shared, size);
        }
        let size = match index % 128 {
            0 => 16_384,
            1..=3 => 4_096,
            4..=15 => 1_024,
            16..=47 => 512,
            _ => 256,
        };
        deterministic_bytes(index, size)
    }

    fn benchmark_delta_payload(index: usize) -> Vec<u8> {
        deterministic_bytes(0xD1_0000usize + index, 1_024 + (index % 5) * 257)
    }

    fn deterministic_bytes(seed: usize, size: usize) -> Vec<u8> {
        let mut state = seed as u64 ^ 0x9E37_79B9_7F4A_7C15;
        let mut bytes = vec![0u8; size];
        for byte in &mut bytes {
            state ^= state << 7;
            state ^= state >> 9;
            state ^= state << 8;
            *byte = state as u8;
        }
        bytes
    }

    fn benchmark_put_range(
        store: &SqliteObjects,
        start: usize,
        end: usize,
        delta: bool,
        mut digests: Option<&mut Vec<String>>,
    ) -> (usize, usize) {
        const BATCH: usize = 1_024;
        let mut inserted = 0usize;
        let mut shared_inputs = 0usize;
        let mut cursor = start;
        while cursor < end {
            let batch_end = (cursor + BATCH).min(end);
            let mut owned = Vec::with_capacity(batch_end - cursor);
            for index in cursor..batch_end {
                if !delta && index % 10 == 0 {
                    shared_inputs += 1;
                }
                let payload = if delta {
                    benchmark_delta_payload(index - start)
                } else {
                    benchmark_payload(index)
                };
                let digest = canonical::hash_bytes(&payload);
                if let Some(digests) = digests.as_mut() {
                    (*digests).push(digest.clone());
                }
                owned.push((digest, payload));
            }
            let refs = owned
                .iter()
                .map(|(digest, payload)| (digest.as_str(), payload.as_slice()))
                .collect::<Vec<_>>();
            inserted += store.put_many(&refs).unwrap();
            cursor = batch_end;
        }
        (inserted, shared_inputs)
    }

    fn benchmark_observe(root: &std::path::Path) -> BenchmarkObservation {
        fn walk(path: &std::path::Path, bytes: &mut u64, inode_count: &mut u64) {
            let Ok(metadata) = std::fs::symlink_metadata(path) else {
                return;
            };
            *inode_count += 1;
            if metadata.is_file() {
                *bytes = bytes.saturating_add(metadata.len());
                return;
            }
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    walk(&entry.path(), bytes, inode_count);
                }
            }
        }
        let mut bytes = 0u64;
        let mut inode_count = 0u64;
        walk(root, &mut bytes, &mut inode_count);
        let rss_bytes = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|output| {
                std::str::from_utf8(&output.stdout)
                    .ok()?
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .map(|kilobytes| kilobytes.saturating_mul(1024))
            });
        BenchmarkObservation {
            bytes,
            inode_count,
            rss_bytes,
        }
    }

    #[test]
    #[ignore = "bounded low-level SQLite CAS qualification benchmark"]
    fn sqlite_objects_100k_benchmark() {
        const INPUTS: usize = 100_000;
        const DELTA: usize = 1_024;
        let directory = tempdir().unwrap();
        let database = directory.path().join("objects.sqlite");
        let started = std::time::Instant::now();
        let store = SqliteObjects::create(&database, "benchmark-store").unwrap();
        let create_seconds = started.elapsed().as_secs_f64();
        let mut digests = Vec::with_capacity(INPUTS);
        let ingest_started = std::time::Instant::now();
        let (ingest_new, shared_inputs) =
            benchmark_put_range(&store, 0, INPUTS, false, Some(&mut digests));
        let ingest_seconds = ingest_started.elapsed().as_secs_f64();
        let ingest_observation = benchmark_observe(directory.path());

        let mut root_payloads = Vec::with_capacity(3);
        for root in 0..3 {
            let mut payload = format!("logical-root-{root}/immutable-record/v1\n").into_bytes();
            for digest in digests.iter().skip(root * 3).step_by(257) {
                payload.extend_from_slice(digest.as_bytes());
                payload.push(b'\n');
            }
            root_payloads.push(payload);
        }
        let root_objects = root_payloads
            .iter()
            .map(|payload| (canonical::hash_bytes(payload), payload.as_slice()))
            .collect::<Vec<_>>();
        let root_started = std::time::Instant::now();
        let root_refs = root_objects
            .iter()
            .map(|(digest, payload)| (digest.as_str(), *payload))
            .collect::<Vec<_>>();
        let root_new = store.put_many(&root_refs).unwrap();
        let root_seconds = root_started.elapsed().as_secs_f64();
        let root_observation = benchmark_observe(directory.path());

        let replay_started = std::time::Instant::now();
        let (replay_new, _) = benchmark_put_range(&store, 0, INPUTS, false, None);
        let replay_seconds = replay_started.elapsed().as_secs_f64();
        let replay_observation = benchmark_observe(directory.path());

        drop(store);
        let reopen_started = std::time::Instant::now();
        let reopened = SqliteObjects::open_existing(&database, "benchmark-store").unwrap();
        let reopen_seconds = reopen_started.elapsed().as_secs_f64();
        let reopen_observation = benchmark_observe(directory.path());

        let reads_started = std::time::Instant::now();
        let mut selected_reads = 0usize;
        for index in (0..INPUTS).step_by(197) {
            let payload = benchmark_payload(index);
            let digest = canonical::hash_bytes(&payload);
            assert_eq!(
                reopened
                    .read(&digest, payload.len() as u64, payload.len() as u64)
                    .unwrap(),
                Some(payload)
            );
            selected_reads += 1;
        }
        let selected_read_seconds = reads_started.elapsed().as_secs_f64();
        let reads_observation = benchmark_observe(directory.path());

        let delta_started = std::time::Instant::now();
        let (delta_new, _) = benchmark_put_range(&reopened, 0, DELTA, true, None);
        let delta_seconds = delta_started.elapsed().as_secs_f64();
        let delta_observation = benchmark_observe(directory.path());
        let (object_count, payload_bytes) = reopened.inventory().unwrap();
        let unique_inputs = digests
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        let report = serde_json::json!({
            "schema": "codeclew-sqlite-objects-benchmark/1.0",
            "engine": "sqlite_objects_low_level",
            "qualificationBoundary": "SQLite CAS engine only; not a whole Codeclew workload",
            "inputObjects": INPUTS,
            "uniqueInputDigests": unique_inputs,
            "sharedPayloadInputs": shared_inputs,
            "batchSize": 1024,
            "logicalImmutableRoots": 3,
            "pinsClaimed": false,
            "deltaObjects": DELTA,
            "selectedReads": selected_reads,
            "counts": {
                "ingestNew": ingest_new,
                "rootRecordsNew": root_new,
                "replayNew": replay_new,
                "deltaNew": delta_new,
                "finalObjects": object_count,
                "finalPayloadBytes": payload_bytes,
            },
            "seconds": {
                "create": create_seconds,
                "ingest": ingest_seconds,
                "rootRecords": root_seconds,
                "replay": replay_seconds,
                "reopen": reopen_seconds,
                "selectedReads": selected_read_seconds,
                "delta": delta_seconds,
            },
            "observations": {
                "ingest": ingest_observation,
                "rootRecords": root_observation,
                "replay": replay_observation,
                "reopen": reopen_observation,
                "selectedReads": reads_observation,
                "delta": delta_observation,
            },
        });
        eprintln!(
            "SQLITE_OBJECTS_BENCHMARK_JSON {}",
            serde_json::to_string(&report).unwrap()
        );
        assert_eq!(replay_new, 0);
        assert_eq!(root_new, 3);
        assert_eq!(delta_new, DELTA);
    }
}
