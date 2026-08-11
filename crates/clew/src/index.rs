use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::freshness::{
    DependencyFact, FRESHNESS_EVENT_SCHEMA, FactDomain, FactFreshness, FactProvenance,
    FreshnessCheckpoint, FreshnessEvent, FreshnessEventKind, FreshnessProjection, IngestOutcome,
};
use crate::identity::{
    IdentityLifecycle, IdentityReport, SnapshotProvenance, decide_identity_delta,
};
use crate::worker::{VerifiedIndexFacts, WorkerClient};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct RepositoryIndex {
    connection: Connection,
    repo: PathBuf,
    blobs: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryFreshnessCheckpoint {
    pub schema: String,
    pub projection: FreshnessCheckpoint,
    pub published_revision: Option<String>,
    pub index_snapshot_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarationRelationSnapshot {
    pub graph: Value,
    pub hash: String,
    pub provenance: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarationDescriptorSnapshot {
    pub graph: Value,
    pub hash: String,
    pub provenance: Value,
}

/// A fully built repository index that is not visible to readers yet.
///
/// The staged database lives beside the published database, so `publish` is a
/// same-filesystem atomic rename.  Dropping an unpublished stage only removes
/// the private staging file; the live index is never modified.
#[derive(Debug)]
pub struct StagedIndex {
    staging_path: PathBuf,
    published_path: PathBuf,
    hash: String,
    invalidations: Vec<String>,
    published: bool,
}

impl StagedIndex {
    pub fn hash(&self) -> &str {
        &self.hash
    }

    pub fn invalidations(&self) -> &[String] {
        &self.invalidations
    }

    pub fn publish(mut self) -> Result<(String, Vec<String>), ClewError> {
        std::fs::rename(&self.staging_path, &self.published_path).map_err(|error| {
            ClewError::new(
                ErrorCode::Internal,
                format!(
                    "cannot atomically publish repository index {}: {error}",
                    self.published_path.display()
                ),
            )
        })?;
        self.published = true;
        Ok((self.hash.clone(), self.invalidations.clone()))
    }
}

impl Drop for StagedIndex {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.staging_path);
        }
    }
}

impl RepositoryIndex {
    pub fn open(repo: &Path) -> Result<Self, ClewError> {
        Self::open_compilation(repo, None)
    }

    pub fn open_compilation(repo: &Path, compilation: Option<&str>) -> Result<Self, ClewError> {
        exclude_runtime_state(repo)?;
        let state = repo.join(".semantic-thread");
        std::fs::create_dir_all(&state).map_err(io_error)?;
        let blobs = state.join("blobs/sha256");
        std::fs::create_dir_all(&blobs).map_err(io_error)?;
        Self::open_database(repo, state.join(database_name(compilation)), blobs)
    }

    fn open_database(repo: &Path, database: PathBuf, blobs: PathBuf) -> Result<Self, ClewError> {
        let connection = Connection::open(database).map_err(db_error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(db_error)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(db_error)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS files (
               path TEXT PRIMARY KEY, content_hash TEXT NOT NULL, facts_json BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS declarations (
               symbol_id TEXT NOT NULL, file_path TEXT NOT NULL, facts_json BLOB NOT NULL,
               PRIMARY KEY(symbol_id, file_path), FOREIGN KEY(file_path) REFERENCES files(path) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS freshness_events (
               sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE NOT NULL,
               event_hash TEXT NOT NULL, event_json BLOB NOT NULL
             );"
        ).map_err(db_error)?;
        Ok(Self {
            connection,
            repo: repo.to_path_buf(),
            blobs,
        })
    }

    /// Build the complete post-commit index without changing the published
    /// database.  All fallible K2 and SQLite work therefore happens before the
    /// Git ref compare-and-swap.
    pub fn stage_update(
        repo: &Path,
        compilation: Option<&str>,
        verified: &VerifiedIndexFacts,
        worker: &WorkerClient,
        source_root: &Path,
        revision: &str,
    ) -> Result<StagedIndex, ClewError> {
        let compilation_identity = compilation.unwrap_or(":/main");
        let facts = worker.authorize_index_facts(verified, source_root, compilation_identity)?;
        let state = repo.join(".semantic-thread");
        std::fs::create_dir_all(&state).map_err(io_error)?;
        let blobs = state.join("blobs/sha256");
        std::fs::create_dir_all(&blobs).map_err(io_error)?;
        let published_path = state.join(database_name(compilation));
        let staging_path = state.join(format!(".index-stage-{}.sqlite3", uuid::Uuid::new_v4()));

        if published_path.exists() {
            // Materialize WAL contents into the database before taking the
            // private copy.  Failure is pre-publication and leaves both ref and
            // live index untouched.
            let live = Connection::open(&published_path).map_err(db_error)?;
            let (busy, log_frames, checkpointed_frames): (i64, i64, i64) = live
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(db_error)?;
            if busy != 0 || log_frames != checkpointed_frames {
                return Err(ClewError::new(
                    ErrorCode::Internal,
                    "published index is busy and cannot be staged consistently",
                ));
            }
            drop(live);
            std::fs::copy(&published_path, &staging_path).map_err(io_error)?;
        }

        let result = (|| {
            let mut staged = Self::open_database(repo, staging_path.clone(), blobs.clone())?;
            let hash = staged.update_from_root_impl(facts, source_root)?;
            staged.require_fresh(REPOSITORY_INDEX_FACT)?;
            let invalidations = staged.invalidations()?;
            staged.mark_published_revision(revision)?;
            // The renamed file must be self-contained; no staging WAL file may
            // be required after publication.
            staged
                .connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
                .map_err(db_error)?;
            drop(staged);
            Ok(StagedIndex {
                staging_path: staging_path.clone(),
                published_path,
                hash,
                invalidations,
                published: false,
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&staging_path);
        }
        result
    }

    pub fn update_verified(
        &mut self,
        verified: &VerifiedIndexFacts,
        worker: &WorkerClient,
    ) -> Result<String, ClewError> {
        let source_root = self.repo.clone();
        let compilation = verified.compilation();
        let facts = worker.authorize_index_facts(verified, &source_root, compilation)?;
        self.update_from_root_impl(facts, &source_root)
    }

    #[cfg(test)]
    fn update(&mut self, facts: &Value) -> Result<String, ClewError> {
        let source_root = self.repo.clone();
        self.update_from_root_impl(facts, &source_root)
    }

    #[cfg(test)]
    fn stage_update_unchecked_for_test(
        repo: &Path,
        compilation: Option<&str>,
        facts: &Value,
        source_root: &Path,
        revision: &str,
    ) -> Result<StagedIndex, ClewError> {
        let state = repo.join(".semantic-thread");
        std::fs::create_dir_all(&state).map_err(io_error)?;
        let blobs = state.join("blobs/sha256");
        std::fs::create_dir_all(&blobs).map_err(io_error)?;
        let published_path = state.join(database_name(compilation));
        let staging_path = state.join(format!(
            ".index-stage-test-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let mut staged = Self::open_database(repo, staging_path.clone(), blobs)?;
        let hash = staged.update_from_root_impl(facts, source_root)?;
        staged.mark_published_revision(revision)?;
        staged
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .map_err(db_error)?;
        drop(staged);
        Ok(StagedIndex {
            staging_path,
            published_path,
            hash,
            invalidations: vec![],
            published: false,
        })
    }

    fn update_from_root_impl(
        &mut self,
        facts: &Value,
        source_root: &Path,
    ) -> Result<String, ClewError> {
        let files = facts
            .get("files")
            .and_then(Value::as_array)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "worker index has no files"))?;
        let relation_snapshot = validate_declaration_relation_snapshot(facts)?;
        let descriptor_snapshot = validate_declaration_descriptor_snapshot(facts)?;
        let source_binding = declaration_source_binding(facts)?;
        let source_binding_hash = canonical::hash(&source_binding).map_err(internal)?;
        let tx = self.connection.transaction().map_err(db_error)?;
        let previous_metadata: BTreeMap<String, String> = [
            "project_model_hash",
            "classpath_hash",
            "compiler_options_hash",
            "index_hash",
            "declaration_relation_hash",
            "declaration_descriptor_hash",
        ]
        .into_iter()
        .filter_map(|key| {
            tx.query_row("SELECT value FROM metadata WHERE key=?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .transpose()
            .map(|result| result.map(|value| (key.to_owned(), value)))
        })
        .collect::<Result<_, _>>()
        .map_err(db_error)?;
        let previous_files: Vec<Value> = {
            let mut statement = tx
                .prepare("SELECT facts_json FROM files ORDER BY path")
                .map_err(db_error)?;
            statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .map_err(db_error)?
                .map(|bytes| {
                    bytes.map_err(db_error).and_then(|bytes| {
                        serde_json::from_slice(&bytes).map_err(|error| internal(error.into()))
                    })
                })
                .collect::<Result<_, _>>()?
        };
        let mut invalidations = BTreeSet::new();
        for (field, metadata_key, invalidation) in [
            (
                "projectModelHash",
                "project_model_hash",
                "COMPILATION_SEMANTICS",
            ),
            ("classpathHash", "classpath_hash", "COMPILATION_CLASSPATH"),
            ("compilerVersion", "compiler_version", "COMPILER_VERSION"),
            (
                "compilerOptionsHash",
                "compiler_options_hash",
                "COMPILER_OPTIONS",
            ),
            ("compilation", "compilation", "COMPILATION_IDENTITY"),
        ] {
            let incoming = facts.get(field).and_then(Value::as_str).unwrap_or_default();
            let previous: Option<String> = tx
                .query_row(
                    "SELECT value FROM metadata WHERE key=?1",
                    [metadata_key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if previous.as_deref().is_some_and(|value| value != incoming) {
                invalidations.insert(invalidation.to_owned());
            }
            tx.execute(
                "INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![metadata_key, incoming],
            )
            .map_err(db_error)?;
        }
        let previous_relation_hash: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_relation_hash'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if previous_relation_hash
            .as_deref()
            .is_some_and(|value| value != relation_snapshot.hash)
        {
            invalidations.insert("DECLARATION_RELATIONS".to_owned());
        }
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('declaration_relations',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [canonical::pretty(&relation_snapshot.graph).map_err(internal)?],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('declaration_relation_hash',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&relation_snapshot.hash],
        )
        .map_err(db_error)?;
        let previous_descriptor_hash: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_descriptor_hash'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if previous_descriptor_hash
            .as_deref()
            .is_some_and(|value| value != descriptor_snapshot.hash)
        {
            invalidations.insert("DECLARATION_DESCRIPTORS".to_owned());
        }
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('declaration_descriptors',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [canonical::pretty(&descriptor_snapshot.graph).map_err(internal)?],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('declaration_descriptor_hash',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&descriptor_snapshot.hash],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('declaration_source_binding',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [canonical::pretty(&source_binding).map_err(internal)?],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('declaration_source_binding_hash',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&source_binding_hash],
        )
        .map_err(db_error)?;
        let incoming: Vec<String> = files
            .iter()
            .filter_map(|f| f.get("path")?.as_str().map(str::to_owned))
            .collect();
        if facts.get("partial").and_then(Value::as_bool) != Some(true) {
            let mut statement = tx.prepare("SELECT path FROM files").map_err(db_error)?;
            let existing: Vec<String> = statement
                .query_map([], |r| r.get(0))
                .map_err(db_error)?
                .collect::<Result<_, _>>()
                .map_err(db_error)?;
            for path in existing.into_iter().filter(|p| !incoming.contains(p)) {
                invalidations.insert(format!("FILE_REMOVED:{path}"));
                tx.execute("DELETE FROM files WHERE path=?1", [&path])
                    .map_err(db_error)?;
            }
        }
        let mut changed_files = 0usize;
        for file in files {
            let path = file["path"].as_str().unwrap_or_default();
            let hash = file["contentHash"].as_str().unwrap_or_default();
            let source_path = source_root.join(path);
            let source = std::fs::read(&source_path).map_err(|error| {
                ClewError::new(
                    ErrorCode::Internal,
                    format!(
                        "cannot read indexed source {}: {error}",
                        source_path.display()
                    ),
                )
            })?;
            if canonical::hash_bytes(&source) != hash {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    format!("source changed while indexing: {path}"),
                ));
            }
            let blob = self.blobs.join(hash.trim_start_matches("sha256:"));
            if !blob.exists() {
                std::fs::write(&blob, &source).map_err(|error| {
                    ClewError::new(
                        ErrorCode::Internal,
                        format!("cannot publish source blob {}: {error}", blob.display()),
                    )
                })?;
            }
            let facts_bytes = canonical::bytes(file).map_err(internal)?;
            let previous_facts: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT facts_json FROM files WHERE path=?1",
                    [path],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            let unchanged = tx
                .query_row(
                    "SELECT content_hash=?2 AND facts_json=?3 FROM files WHERE path=?1",
                    params![path, hash, facts_bytes],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false);
            if unchanged {
                continue;
            }
            if let Some(previous) = previous_facts {
                let previous: Value =
                    serde_json::from_slice(&previous).map_err(|error| internal(error.into()))?;
                classify_file_change(path, &previous, file, &mut invalidations);
            } else {
                invalidations.insert(format!("FILE_ADDED:{path}"));
            }
            changed_files += 1;
            tx.execute("INSERT INTO files(path,content_hash,facts_json) VALUES(?1,?2,?3) ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,facts_json=excluded.facts_json", params![path, hash, facts_bytes]).map_err(db_error)?;
            tx.execute("DELETE FROM declarations WHERE file_path=?1", [path])
                .map_err(db_error)?;
            for declaration in file
                .get("declarations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                tx.execute(
                    "INSERT INTO declarations(symbol_id,file_path,facts_json) VALUES(?1,?2,?3)
                     ON CONFLICT(symbol_id,file_path) DO UPDATE SET facts_json=excluded.facts_json",
                    params![
                        declaration["symbolId"].as_str().unwrap_or_default(),
                        path,
                        canonical::bytes(declaration).map_err(internal)?
                    ],
                )
                .map_err(db_error)?;
            }
        }
        let stored_files: Vec<Value> = {
            let mut statement = tx
                .prepare("SELECT facts_json FROM files ORDER BY path")
                .map_err(db_error)?;
            statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .map_err(db_error)?
                .map(|bytes| {
                    bytes.map_err(db_error).and_then(|bytes| {
                        serde_json::from_slice(&bytes).map_err(|error| internal(error.into()))
                    })
                })
                .collect::<Result<_, _>>()?
        };
        let index_hash = canonical::hash(&serde_json::json!({
            "projectModelHash":facts.get("projectModelHash"),
            "classpathHash":facts.get("classpathHash"),
            "compilerVersion":facts.get("compilerVersion"),
            "compilerOptionsHash":facts.get("compilerOptionsHash"),
            "declarationRelations":relation_snapshot.graph,
            "declarationRelationHash":relation_snapshot.hash,
            "declarationDescriptors":descriptor_snapshot.graph,
            "declarationDescriptorHash":descriptor_snapshot.hash,
            "files":stored_files
        }))
        .map_err(internal)?;
        tx.execute("INSERT INTO metadata(key,value) VALUES('index_hash',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&index_hash]).map_err(db_error)?;
        tx.execute("INSERT INTO metadata(key,value) VALUES('last_changed_files',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [changed_files.to_string()]).map_err(db_error)?;
        let unavailable = "UNAVAILABLE".to_owned();
        let previous_project_model = previous_metadata
            .get("project_model_hash")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| unavailable.clone());
        let previous_classpath = previous_metadata
            .get("classpath_hash")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| unavailable.clone());
        let previous_options = previous_metadata
            .get("compiler_options_hash")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| unavailable.clone());
        let previous_index = previous_metadata
            .get("index_hash")
            .cloned()
            .unwrap_or_else(|| canonical::hash_bytes(b"EMPTY_INDEX"));
        let previous_relation = previous_metadata
            .get("declaration_relation_hash")
            .cloned()
            .unwrap_or_else(|| "UNAVAILABLE".to_owned());
        let previous_descriptor = previous_metadata
            .get("declaration_descriptor_hash")
            .cloned()
            .unwrap_or_else(|| "UNAVAILABLE".to_owned());
        let incoming_project_model = facts
            .get("projectModelHash")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("UNAVAILABLE")
            .to_owned();
        let incoming_classpath = facts
            .get("classpathHash")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("UNAVAILABLE")
            .to_owned();
        let incoming_options = facts
            .get("compilerOptionsHash")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("UNAVAILABLE")
            .to_owned();
        let incoming_compiler_version = facts
            .get("compilerVersion")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("UNAVAILABLE")
            .to_owned();
        let before_snapshot = SnapshotProvenance {
            composite_snapshot_hash: canonical::hash(&serde_json::json!({
                "index":previous_index,
                "declarationRelations":previous_relation,
                "declarationDescriptors":previous_descriptor,
                "projectModel":previous_project_model,
                "classpath":previous_classpath,
                "compilerOptions":previous_options,
            }))
            .map_err(internal)?,
            index_snapshot_hash: previous_index,
            project_model_hash: previous_project_model,
            classpath_hash: previous_classpath,
            compiler_options_hash: previous_options,
        };
        let after_snapshot = SnapshotProvenance {
            composite_snapshot_hash: canonical::hash(&serde_json::json!({
                "index":index_hash,
                "declarationRelations":relation_snapshot.hash,
                "declarationDescriptors":descriptor_snapshot.hash,
                "projectModel":incoming_project_model,
                "classpath":incoming_classpath,
                "compilerOptions":incoming_options,
            }))
            .map_err(internal)?,
            index_snapshot_hash: index_hash.clone(),
            project_model_hash: incoming_project_model,
            classpath_hash: incoming_classpath,
            compiler_options_hash: incoming_options,
        };
        let identity_report = decide_identity_delta(
            before_snapshot,
            after_snapshot,
            &previous_files,
            &stored_files,
        )
        .map_err(|error| internal(error.into()))?;
        record_identity_invalidations(&identity_report, &mut invalidations);
        let identity_json = canonical::pretty(&identity_report).map_err(internal)?;
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('last_identity_report',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&identity_json],
        )
        .map_err(db_error)?;
        record_freshness_update(
            &tx,
            facts,
            &stored_files,
            &index_hash,
            &identity_report,
            &incoming_compiler_version,
            &invalidations,
        )?;
        let invalidations_json = canonical::pretty(&invalidations).map_err(internal)?;
        tx.execute("INSERT INTO metadata(key,value) VALUES('last_invalidations',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&invalidations_json]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(index_hash)
    }

    pub fn hash(&self) -> Result<Option<String>, ClewError> {
        if self.declaration_relations()?.is_none() || self.declaration_descriptors()?.is_none() {
            return Ok(None);
        }
        let mut statement = self
            .connection
            .prepare("SELECT value FROM metadata WHERE key='index_hash'")
            .map_err(db_error)?;
        match statement.query_row([], |r| r.get(0)) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(db_error(e)),
        }
    }

    fn persisted_declaration_source_files(&self) -> Result<Vec<Value>, ClewError> {
        let stored_binding: String = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_source_binding'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .filter(|value: &String| !value.is_empty())
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "stored declaration snapshots have no source binding",
                )
            })?;
        let stored_hash: String = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_source_binding_hash'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .filter(|value: &String| !value.is_empty())
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "stored declaration snapshots have no source binding hash",
                )
            })?;
        let binding: Value = serde_json::from_str(&stored_binding).map_err(|error| {
            ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!("stored declaration source binding is malformed: {error}"),
            )
        })?;
        if binding.get("schema").and_then(Value::as_str) != Some("declaration-source-binding/0.1")
            || canonical::hash(&binding).map_err(internal)? != stored_hash
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "stored declaration source binding hash/schema differs",
            ));
        }
        let compilation: String = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='compilation'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .filter(|value: &String| !value.is_empty())
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "stored declaration source binding has no compilation metadata",
                )
            })?;
        if binding.get("compilation").and_then(Value::as_str) != Some(compilation.as_str()) {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "stored declaration source binding compilation differs from metadata",
            ));
        }
        let stored_files: Vec<Value> = {
            let mut statement = self
                .connection
                .prepare("SELECT facts_json FROM files ORDER BY path")
                .map_err(db_error)?;
            statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .map_err(db_error)?
                .map(|bytes| {
                    bytes.map_err(db_error).and_then(|bytes| {
                        let file: Value = serde_json::from_slice(&bytes)
                            .map_err(|error| internal(error.into()))?;
                        Ok(serde_json::json!({
                            "path":file.get("path"),
                            "module":file.get("module"),
                            "sourceSet":file.get("sourceSet"),
                        }))
                    })
                })
                .collect::<Result<_, _>>()?
        };
        let live_binding = declaration_source_binding(&serde_json::json!({
            "compilation":compilation,
            "files":stored_files,
        }))?;
        if live_binding != binding
            || canonical::hash(&live_binding).map_err(internal)? != stored_hash
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "stored declaration source binding differs from the persisted file table",
            ));
        }
        Ok(live_binding["files"].as_array().unwrap().clone())
    }

    pub fn declaration_relations(&self) -> Result<Option<DeclarationRelationSnapshot>, ClewError> {
        let graph: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_relations'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let hash: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_relation_hash'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let (Some(graph), Some(hash)) = (graph, hash) else {
            return Ok(None);
        };
        let graph: Value = serde_json::from_str(&graph).map_err(|error| {
            ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!("stored declaration relation graph is malformed: {error}"),
            )
        })?;
        let fields = |key: &str| -> Result<String, ClewError> {
            self.connection
                .query_row("SELECT value FROM metadata WHERE key=?1", [key], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(db_error)?
                .filter(|value: &String| !value.is_empty())
                .ok_or_else(|| {
                    ClewError::new(
                        ErrorCode::ProjectModelChanged,
                        format!("stored relation provenance has no {key}"),
                    )
                })
        };
        let descriptor = self.declaration_descriptors()?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::ProjectModelChanged,
                "stored declaration relations have no matching descriptor graph",
            )
        })?;
        let source_files = self.persisted_declaration_source_files()?;
        let facts = serde_json::json!({
            "compilation": fields("compilation")?,
            "projectModelHash": fields("project_model_hash")?,
            "classpathHash": fields("classpath_hash")?,
            "compilerVersion": fields("compiler_version")?,
            "compilerOptionsHash": fields("compiler_options_hash")?,
            "declarationRelations": graph,
            "declarationRelationHash": hash,
            "declarationDescriptors": descriptor.graph,
            "declarationDescriptorHash": descriptor.hash,
            "files":source_files,
        });
        validate_declaration_relation_snapshot(&facts).map(Some)
    }

    pub fn declaration_descriptors(
        &self,
    ) -> Result<Option<DeclarationDescriptorSnapshot>, ClewError> {
        let graph: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_descriptors'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let hash: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_descriptor_hash'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let (Some(graph), Some(hash)) = (graph, hash) else {
            return Ok(None);
        };
        let graph: Value = serde_json::from_str(&graph).map_err(|error| {
            ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!("stored declaration descriptor graph is malformed: {error}"),
            )
        })?;
        let source_files = self.persisted_declaration_source_files()?;
        let fields = |key: &str| -> Result<String, ClewError> {
            self.connection
                .query_row("SELECT value FROM metadata WHERE key=?1", [key], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(db_error)?
                .filter(|value: &String| !value.is_empty())
                .ok_or_else(|| {
                    ClewError::new(
                        ErrorCode::ProjectModelChanged,
                        format!("stored descriptor provenance has no {key}"),
                    )
                })
        };
        let facts = serde_json::json!({
            "compilation": fields("compilation")?,
            "projectModelHash": fields("project_model_hash")?,
            "classpathHash": fields("classpath_hash")?,
            "compilerVersion": fields("compiler_version")?,
            "compilerOptionsHash": fields("compiler_options_hash")?,
            "declarationDescriptors": graph,
            "declarationDescriptorHash": hash,
            "files":source_files,
        });
        validate_declaration_descriptor_snapshot(&facts).map(Some)
    }

    pub fn invalidations(&self) -> Result<Vec<String>, ClewError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='last_invalidations'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        value
            .map(|json| serde_json::from_str(&json).map_err(|error| internal(error.into())))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn identity_report(&self) -> Result<Option<IdentityReport>, ClewError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='last_identity_report'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        value
            .map(|json| serde_json::from_str(&json).map_err(|error| internal(error.into())))
            .transpose()
    }

    pub fn freshness_checkpoint(&self) -> Result<RepositoryFreshnessCheckpoint, ClewError> {
        Ok(RepositoryFreshnessCheckpoint {
            schema: "repository-freshness-checkpoint/0.1".to_owned(),
            projection: load_freshness_projection(&self.connection)?.checkpoint(),
            published_revision: self.published_revision()?,
            index_snapshot_hash: self.hash()?,
        })
    }

    pub fn freshness_status(&self, fact_id: &str) -> Result<FactFreshness, ClewError> {
        Ok(load_freshness_projection(&self.connection)?.status(fact_id))
    }

    pub fn require_fresh(&self, fact_id: &str) -> Result<(), ClewError> {
        let status = self.freshness_status(fact_id)?;
        if status == FactFreshness::Fresh {
            Ok(())
        } else {
            Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!("semantic fact {fact_id} is not fresh: {status:?}"),
            ))
        }
    }

    /// Persist one externally delivered at-least-once event. A sequence gap is
    /// itself durably checkpointed before the fail-closed error is returned, so
    /// no subsequent reader can continue serving the old projection as fresh.
    pub fn ingest_freshness_event(
        &mut self,
        event: FreshnessEvent,
    ) -> Result<IngestOutcome, ClewError> {
        let tx = self.connection.transaction().map_err(db_error)?;
        let mut projection = load_freshness_projection(&tx)?;
        match projection.ingest(event.clone()) {
            Ok(outcome) => {
                if matches!(outcome, IngestOutcome::Applied { .. }) {
                    insert_freshness_event(&tx, &event)?;
                }
                store_freshness_checkpoint(&tx, &projection.checkpoint())?;
                tx.commit().map_err(db_error)?;
                Ok(outcome)
            }
            Err(error @ crate::freshness::FreshnessError::OutOfOrder { .. }) => {
                store_freshness_checkpoint(&tx, &projection.checkpoint())?;
                tx.commit().map_err(db_error)?;
                Err(internal(error.into()))
            }
            Err(error) => Err(internal(error.into())),
        }
    }

    /// Verify that the durable contiguous log reproduces the checkpoint.
    /// A persisted gap cannot be cleared by replaying the older log; only a
    /// complete authoritative index rebuild may supersede missing input.
    pub fn recover_freshness_from_log(&mut self) -> Result<FreshnessCheckpoint, ClewError> {
        let tx = self.connection.transaction().map_err(db_error)?;
        let current = load_freshness_projection(&tx)?;
        if current.checkpoint().sequence_gap.is_some() {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "freshness stream has a gap; a complete index rebuild is required",
            ));
        }
        let events = load_freshness_events(&tx)?;
        let projection =
            FreshnessProjection::replay(events).map_err(|error| internal(error.into()))?;
        let checkpoint = projection.checkpoint();
        if checkpoint != current.checkpoint() {
            return Err(ClewError::new(
                ErrorCode::Internal,
                "freshness checkpoint differs from deterministic event replay",
            ));
        }
        tx.commit().map_err(db_error)?;
        Ok(checkpoint)
    }

    pub fn mark_published_revision(&self, revision: &str) -> Result<(), ClewError> {
        self.connection
            .execute(
                "INSERT INTO metadata(key,value) VALUES('published_revision',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [revision],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn published_revision(&self) -> Result<Option<String>, ClewError> {
        self.connection
            .query_row(
                "SELECT value FROM metadata WHERE key='published_revision'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)
    }
}

fn declaration_source_binding(facts: &Value) -> Result<Value, ClewError> {
    let invalid = |message: &str| ClewError::new(ErrorCode::InvalidInput, message);
    let compilation = facts
        .get("compilation")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("semantic source binding has no compilation"))?;
    let files = facts
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("semantic source binding has no source files"))?;
    let mut rows = Vec::with_capacity(files.len());
    let mut paths = BTreeSet::new();
    for file in files {
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("semantic source binding file has no path"))?;
        let module = file
            .get("module")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("semantic source binding file has no module"))?;
        let source_set = file
            .get("sourceSet")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("semantic source binding file has no sourceSet"))?;
        let path_value = Path::new(path);
        if path_value.is_absolute()
            || path_value.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
            || !paths.insert(path)
            || format!("{module}/{source_set}") != compilation
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "semantic source binding path/module/sourceSet differs from compilation",
            ));
        }
        rows.push(serde_json::json!({
            "path":path,
            "module":module,
            "sourceSet":source_set,
        }));
    }
    rows.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
    Ok(serde_json::json!({
        "schema":"declaration-source-binding/0.1",
        "compilation":compilation,
        "files":rows,
    }))
}

pub(crate) fn validate_declaration_relation_snapshot(
    facts: &Value,
) -> Result<DeclarationRelationSnapshot, ClewError> {
    fn invalid(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::InvalidInput, message)
    }
    fn staged(mut error: ClewError, stage: &str) -> ClewError {
        error.evidence.push(format!("verified-index-stage:{stage}"));
        error
    }
    fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
        value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(format!("declaration relation graph has no {field}")))
    }
    fn canonical_rows(rows: &[Value], label: &str) -> Result<(), ClewError> {
        let encoded = rows
            .iter()
            .map(canonical::bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid(format!(
                "declaration relation {label} must be canonical, sorted, and unique"
            )));
        }
        Ok(())
    }
    fn validate_occurrence(
        occurrence: &Value,
        label: &str,
        expected_nullable: bool,
    ) -> Result<(i64, i64), ClewError> {
        let start = occurrence
            .get("start")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source start")))?;
        let end = occurrence
            .get("end")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source end")))?;
        let rendered = required_string(occurrence, "type")?;
        let nullable = occurrence
            .get("nullable")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid(format!("{label} has no nullability")))?;
        if start < 0
            || end <= start
            || nullable != expected_nullable
            || nullable != rendered.trim_end().ends_with('?')
            || rendered.contains("..")
            || rendered.contains('!')
            || rendered.contains("<ERROR")
        {
            return Err(invalid(format!(
                "{label} source range, type, or nullability is inconsistent"
            )));
        }
        Ok((start, end))
    }
    fn rendered_nullability(rendered: &str) -> Result<bool, ClewError> {
        if rendered.contains("..")
            || rendered.contains('!')
            || rendered.contains("<ERROR")
            || rendered.contains("<unknown>")
            || rendered.contains("UNKNOWN")
        {
            return Err(invalid(
                "return-value relation contains an unresolved compiler type",
            ));
        }
        Ok(rendered.trim_end().ends_with('?'))
    }
    fn occurrence_range_and_node(
        occurrence: &Value,
        label: &str,
    ) -> Result<(i64, i64, u64), ClewError> {
        let object = occurrence
            .as_object()
            .ok_or_else(|| invalid(format!("{label} is not an object")))?;
        if object.len() != 3
            || !object.contains_key("start")
            || !object.contains_key("end")
            || !object.contains_key("cfgNodeId")
        {
            return Err(invalid(format!(
                "{label} has fields outside its exact contract"
            )));
        }
        let start = occurrence
            .get("start")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source start")))?;
        let end = occurrence
            .get("end")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source end")))?;
        let node = occurrence
            .get("cfgNodeId")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid(format!("{label} has no CFG node identity")))?;
        if start < 0 || end <= start {
            return Err(invalid(format!("{label} has an invalid source range")));
        }
        Ok((start, end, node))
    }
    fn validate_return_value_relation(
        relation: &Value,
        relations: &[Value],
        descriptors: &[Value],
    ) -> Result<(), ClewError> {
        let allowed = BTreeSet::from([
            "schema",
            "file",
            "start",
            "end",
            "kind",
            "owner",
            "target",
            "resolution",
            "provider",
            "sourceKind",
            "sourceOccurrence",
            "returnOccurrence",
            "resultOccurrence",
            "resultType",
            "resultNullable",
            "valueProvenance",
            "cfgProvenance",
            "evaluationCount",
            "orderKey",
            "cfgNodeIds",
            "sourceProvenance",
            "orderProvenance",
        ]);
        let object = relation
            .as_object()
            .ok_or_else(|| invalid("return-value relation is not an object"))?;
        if object.keys().any(|field| !allowed.contains(field.as_str()))
            || object.values().any(|value| match value {
                Value::String(value) => {
                    value.is_empty() || value == "UNKNOWN" || value.contains("<unknown>")
                }
                Value::Null => true,
                _ => false,
            })
        {
            return Err(invalid(
                "return-value PROVEN relation has unknown or out-of-contract fields",
            ));
        }
        let owner = required_string(relation, "owner")?;
        let target = required_string(relation, "target")?;
        let file = required_string(relation, "file")?;
        let source_kind = required_string(relation, "sourceKind")?;
        let source_relation_kind = match source_kind {
            "PROPERTY_READ" => "READS",
            "FUNCTION_CALL_RESULT" => "CALLS",
            _ => return Err(invalid("unknown return-value sourceKind")),
        };
        let source_occurrence = relation
            .get("sourceOccurrence")
            .ok_or_else(|| invalid("return-value relation has no source occurrence"))?;
        let return_occurrence = relation
            .get("returnOccurrence")
            .ok_or_else(|| invalid("return-value relation has no return occurrence"))?;
        let result_occurrence = relation
            .get("resultOccurrence")
            .ok_or_else(|| invalid("return-value relation has no result occurrence"))?;
        let (source_start, source_end, source_node) =
            occurrence_range_and_node(source_occurrence, "return-value source occurrence")?;
        let (return_start, return_end, return_node) =
            occurrence_range_and_node(return_occurrence, "return occurrence")?;
        let (result_start, result_end, result_node) =
            occurrence_range_and_node(result_occurrence, "return result occurrence")?;
        let start = relation.get("start").and_then(Value::as_i64).unwrap_or(-1);
        let end = relation.get("end").and_then(Value::as_i64).unwrap_or(-1);
        if (source_start, source_end, source_node) != (result_start, result_end, result_node)
            || start != return_start
            || end != return_end
            || return_start > source_start
            || source_end > return_end
            || relation.get("orderKey").and_then(Value::as_i64) != Some(source_start)
            || relation.get("valueProvenance").and_then(Value::as_str)
                != Some("FIR_RETURN_RESULT_IDENTITY")
            || relation.get("evaluationCount").and_then(Value::as_u64) != Some(1)
            || relation.get("orderProvenance").and_then(Value::as_str) != Some("K2_FIR_CFG")
        {
            return Err(invalid(
                "return-value occurrence ranges, identity, order, or evaluation are inconsistent",
            ));
        }
        let cfg_nodes = relation
            .get("cfgNodeIds")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("return-value relation has no CFG node set"))?;
        let cfg_node_values = cfg_nodes
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| invalid("return-value CFG node identity is not an integer"))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if cfg_node_values.len() != cfg_nodes.len()
            || !cfg_node_values.contains(&source_node)
            || !cfg_node_values.contains(&return_node)
        {
            return Err(invalid(
                "return-value relation does not contain its exact unique CFG nodes",
            ));
        }
        let cfg = relation
            .get("cfgProvenance")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("return-value relation has no CFG provenance"))?;
        let expected_cfg_fields = BTreeSet::from([
            "graphName",
            "sourceReachesReturn",
            "sourceDominatesReturn",
            "sourceNodeKind",
            "returnNodeKind",
        ]);
        if cfg.len() != expected_cfg_fields.len()
            || cfg
                .keys()
                .any(|field| !expected_cfg_fields.contains(field.as_str()))
            || cfg
                .get("graphName")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || cfg.get("sourceReachesReturn").and_then(Value::as_bool) != Some(true)
            || cfg.get("sourceDominatesReturn").and_then(Value::as_bool) != Some(true)
            || cfg.get("returnNodeKind").and_then(Value::as_str) != Some("JumpNode")
            || cfg.get("sourceNodeKind").and_then(Value::as_str)
                != Some(if source_kind == "PROPERTY_READ" {
                    "QualifiedAccessNode"
                } else {
                    "FunctionCallExitNode"
                })
        {
            return Err(invalid(
                "return-value CFG reachability or dominance provenance is incomplete",
            ));
        }
        let result_type = required_string(relation, "resultType")?;
        let result_nullable = relation
            .get("resultNullable")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("return-value relation has no result nullability"))?;
        if rendered_nullability(result_type)? != result_nullable {
            return Err(invalid(
                "return-value result type and nullability are inconsistent",
            ));
        }
        let owner_descriptors = descriptors
            .iter()
            .filter(|descriptor| {
                descriptor.get("declarationKind").and_then(Value::as_str) == Some("FUNCTION")
                    && descriptor.get("compilerCallableId").and_then(Value::as_str) == Some(owner)
            })
            .collect::<Vec<_>>();
        let source_descriptors = descriptors
            .iter()
            .filter(|descriptor| {
                matches!(
                    (
                        source_kind,
                        descriptor.get("declarationKind").and_then(Value::as_str)
                    ),
                    ("PROPERTY_READ", Some("PROPERTY" | "MUTABLE_PROPERTY"))
                        | ("FUNCTION_CALL_RESULT", Some("FUNCTION"))
                ) && descriptor.get("compilerCallableId").and_then(Value::as_str) == Some(target)
            })
            .collect::<Vec<_>>();
        let ([owner_descriptor], [source_descriptor]) =
            (owner_descriptors.as_slice(), source_descriptors.as_slice())
        else {
            return Err(staged(
                invalid("return-value owner or source descriptor is missing or ambiguous"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        };
        let (source_type, source_nullable) = if source_kind == "PROPERTY_READ" {
            (
                required_string(source_descriptor, "declaredType")?,
                source_descriptor
                    .get("declaredNullable")
                    .and_then(Value::as_bool),
            )
        } else {
            (
                required_string(source_descriptor, "returnType")?,
                source_descriptor
                    .get("returnNullable")
                    .and_then(Value::as_bool),
            )
        };
        if source_nullable != Some(result_nullable)
            || source_type != result_type
            || required_string(owner_descriptor, "returnType")? != result_type
            || owner_descriptor
                .get("returnNullable")
                .and_then(Value::as_bool)
                != Some(result_nullable)
        {
            return Err(staged(
                invalid("return-value descriptors disagree with the exact compiler result type"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        }
        let source_rows = relations
            .iter()
            .filter(|candidate| {
                candidate.get("kind").and_then(Value::as_str) == Some(source_relation_kind)
                    && candidate.get("owner").and_then(Value::as_str) == Some(owner)
                    && candidate.get("target").and_then(Value::as_str) == Some(target)
                    && candidate.get("file").and_then(Value::as_str) == Some(file)
                    && candidate.get("start").and_then(Value::as_i64) == Some(source_start)
                    && candidate.get("end").and_then(Value::as_i64) == Some(source_end)
            })
            .collect::<Vec<_>>();
        let [source_row] = source_rows.as_slice() else {
            return Err(staged(
                invalid("return-value source READS/CALLS occurrence is missing or ambiguous"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        };
        if required_string(source_row, "resultType")? != result_type
            || source_row
                .get("cfgNodeIds")
                .and_then(Value::as_array)
                .is_none_or(|nodes| !nodes.iter().any(|node| node.as_u64() == Some(source_node)))
            || source_row.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || source_row.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        {
            return Err(staged(
                invalid("return-value source row type or CFG identity is inconsistent"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        }
        Ok(())
    }

    let graph = facts
        .get("declarationRelations")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("worker index has no declaration relation graph"))?;
    if graph.get("schema").and_then(Value::as_str) != Some("declaration-relation-graph/0.1") {
        return Err(invalid("unsupported declaration relation graph schema"));
    }
    let compilation = required_string(graph, "compilation")?;
    if facts.get("compilation").and_then(Value::as_str) != Some(compilation) {
        return Err(staged(
            ClewError::new(
                ErrorCode::ProjectModelChanged,
                "declaration relation graph compilation differs from worker index snapshot",
            ),
            "SOURCE_BINDING",
        ));
    }
    if !matches!(
        graph.get("coverage").and_then(Value::as_str),
        Some("COMPLETE_SUPPORTED_SUBSET" | "PARTIAL")
    ) {
        return Err(invalid("unknown declaration relation coverage status"));
    }
    let relations = graph
        .get("relations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration relation graph has no relations"))?;
    let descriptor_rows = facts
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            staged(
                invalid("declaration relations have no authoritative descriptor graph"),
                "CROSS_GRAPH_CONSISTENCY",
            )
        })?;
    let source_binding =
        declaration_source_binding(facts).map_err(|error| staged(error, "SOURCE_BINDING"))?;
    let source_compilations = source_binding["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            let path = required_string(file, "path")?;
            let module = required_string(file, "module")?;
            let source_set = required_string(file, "sourceSet")?;
            let identity = format!("{module}/{source_set}");
            if identity != compilation {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "declaration relation source module/sourceSet differs from compilation",
                ));
            }
            Ok((path.to_owned(), (module.to_owned(), source_set.to_owned())))
        })
        .collect::<Result<BTreeMap<_, _>, ClewError>>()?;
    canonical_rows(relations, "rows")?;
    for relation in relations {
        if relation.get("schema").and_then(Value::as_str) != Some("declaration-relation/0.1")
            || relation.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || relation.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        {
            return Err(invalid(
                "malformed or non-authoritative declaration relation",
            ));
        }
        if !matches!(
            relation.get("kind").and_then(Value::as_str),
            Some(
                "OVERRIDES"
                    | "CALLS"
                    | "REFERENCES"
                    | "CONSTRUCTS"
                    | "READS"
                    | "WRITES"
                    | "INITIALIZES"
                    | "NULL_COALESCES"
                    | "RETURNS_VALUE_FROM"
            )
        ) {
            return Err(invalid("unknown declaration relation kind"));
        }
        let file = required_string(relation, "file")?;
        if Path::new(file).is_absolute()
            || Path::new(file)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            || required_string(relation, "owner").is_err()
            || required_string(relation, "target").is_err()
        {
            return Err(invalid(
                "declaration relation identity is not repository-contained",
            ));
        }
        let start = relation.get("start").and_then(Value::as_i64).unwrap_or(-1);
        let end = relation.get("end").and_then(Value::as_i64).unwrap_or(-1);
        if start < 0
            || end < start
            || !relation.get("cfgNodeIds").is_some_and(Value::is_array)
            || !source_compilations.contains_key(file)
            || relation.get("sourceProvenance").and_then(Value::as_str)
                != Some("COMPILER_SOURCE_RANGE")
            || !matches!(
                relation.get("orderProvenance").and_then(Value::as_str),
                Some("K2_FIR_CFG" | "FIR_SOURCE_RANGE" | "UNKNOWN")
            )
        {
            return Err(invalid(
                "declaration relation has invalid source/CFG provenance",
            ));
        }
        if let Some(arguments) = relation.get("argumentToParameter") {
            let arguments = arguments
                .as_array()
                .ok_or_else(|| invalid("argument mapping is not an array"))?;
            let mut indices = BTreeSet::new();
            for argument in arguments {
                let index = argument
                    .get("parameterIndex")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid("argument mapping has no nonnegative parameterIndex"))?;
                if !indices.insert(index) {
                    return Err(invalid("argument mapping repeats a parameterIndex"));
                }
                required_string(argument, "parameterType")?;
            }
            let relation_kind = relation.get("kind").and_then(Value::as_str);
            if matches!(relation_kind, Some("CALLS" | "CONSTRUCTS")) {
                let target = required_string(relation, "target")?;
                let expected_kind = if relation_kind == Some("CALLS") {
                    "FUNCTION"
                } else {
                    "CONSTRUCTOR"
                };
                let descriptors = descriptor_rows
                    .iter()
                    .filter(|descriptor| {
                        descriptor.get("declarationKind").and_then(Value::as_str)
                            == Some(expected_kind)
                            && descriptor.get("compilerCallableId").and_then(Value::as_str)
                                == Some(target)
                    })
                    .collect::<Vec<_>>();
                if descriptors.len() != 1 {
                    return Err(staged(
                        invalid(
                            "call/constructor argument mapping target is missing or overload-ambiguous",
                        ),
                        "CROSS_GRAPH_CONSISTENCY",
                    ));
                }
                let parameters = descriptors[0]
                    .get("parameterTypes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        staged(
                            invalid("call target descriptor has no parameter types"),
                            "CROSS_GRAPH_CONSISTENCY",
                        )
                    })?;
                for argument in arguments {
                    let index = argument["parameterIndex"].as_u64().unwrap() as usize;
                    let slot = parameters.get(index).ok_or_else(|| {
                        staged(
                            invalid("argument mapping parameterIndex is outside target descriptor"),
                            "CROSS_GRAPH_CONSISTENCY",
                        )
                    })?;
                    if required_string(argument, "parameterType")? != required_string(slot, "type")?
                    {
                        return Err(staged(
                            invalid("argument mapping parameterType differs from descriptor slot"),
                            "CROSS_GRAPH_CONSISTENCY",
                        ));
                    }
                    let argument_start = argument
                        .get("argumentStart")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| invalid("argument mapping has no source occurrence"))?;
                    if argument_start < start || argument_start >= end {
                        return Err(invalid(
                            "argument mapping source occurrence is outside the call range",
                        ));
                    }
                }
            }
        }
        if relation.get("kind").and_then(Value::as_str) == Some("NULL_COALESCES") {
            let source_target = required_string(relation, "sourceTarget")?;
            let fallback_target = required_string(relation, "fallbackTarget")?;
            if required_string(relation, "target")? != fallback_target
                || source_target == fallback_target
                || relation.get("orderProvenance").and_then(Value::as_str) != Some("K2_FIR_CFG")
                || relation
                    .get("cfgNodeIds")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty)
            {
                return Err(invalid(
                    "null coalescing relation lacks exact compiler/CFG authority",
                ));
            }
            let (lhs_start, lhs_end) = validate_occurrence(
                relation
                    .get("sourceOccurrence")
                    .ok_or_else(|| invalid("null coalescing relation has no source occurrence"))?,
                "null coalescing source occurrence",
                true,
            )?;
            let (fallback_start, fallback_end) = validate_occurrence(
                relation.get("fallbackOccurrence").ok_or_else(|| {
                    invalid("null coalescing relation has no fallback occurrence")
                })?,
                "null coalescing fallback occurrence",
                false,
            )?;
            let (merged_start, merged_end) = validate_occurrence(
                relation
                    .get("mergedOccurrence")
                    .ok_or_else(|| invalid("null coalescing relation has no merged occurrence"))?,
                "null coalescing merged occurrence",
                false,
            )?;
            let branch = relation
                .get("branchProvenance")
                .filter(|value| value.is_object())
                .ok_or_else(|| invalid("null coalescing relation has no branch provenance"))?;
            if branch.get("kind").and_then(Value::as_str) != Some("FIR_ELVIS_EXPRESSION")
                || branch.get("nullableBranchStart").and_then(Value::as_i64) != Some(lhs_start)
                || branch.get("fallbackBranchStart").and_then(Value::as_i64) != Some(fallback_start)
                || branch.get("mergeStart").and_then(Value::as_i64) != Some(merged_start)
                || branch.get("mergeEnd").and_then(Value::as_i64) != Some(merged_end)
                || merged_start != start
                || merged_end != end
                || lhs_start < merged_start
                || lhs_end > merged_end
                || fallback_start < merged_start
                || fallback_end > merged_end
                || lhs_end > fallback_start
                || relation.get("orderKey").and_then(Value::as_i64) != Some(merged_start)
            {
                return Err(invalid(
                    "null coalescing branch/merge ranges are inconsistent",
                ));
            }
        }
        if relation.get("kind").and_then(Value::as_str) == Some("RETURNS_VALUE_FROM") {
            validate_return_value_relation(relation, relations, descriptor_rows)?;
        }
    }

    let boundaries = graph
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration relation graph has no boundaries"))?;
    canonical_rows(boundaries, "boundaries")?;
    for boundary in boundaries {
        if boundary.get("schema").and_then(Value::as_str)
            != Some("declaration-relation-boundary/0.1")
            || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            || !matches!(
                boundary.get("provider").and_then(Value::as_str),
                Some("K2_FIR" | "K2_FIR_CFG" | "COMPILER_RELATION_NORMALIZER" | "WORKER")
            )
            || required_string(boundary, "stage").is_err()
            || required_string(boundary, "code").is_err()
        {
            return Err(invalid("malformed declaration relation Unknown boundary"));
        }
        if boundary.get("stage").and_then(Value::as_str) == Some("RETURN_VALUE") {
            let allowed_codes = BTreeSet::from([
                "IMPLICIT_RETURN_UNSUPPORTED",
                "IMPLICIT_OR_MISSING_RETURN_SOURCE",
                "UNRESOLVED_RETURN_OWNER",
                "LOCAL_OR_GENERATED_RETURN_OWNER",
                "RETURN_TARGET_IDENTITY_MISMATCH",
                "NON_LINEAR_OR_MULTIPLE_RETURN_FLOW",
                "RETURN_VALUE_NOT_DIRECT_RESOLVED_READ_OR_CALL",
                "MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES",
                "LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE",
                "MISSING_RETURN_CFG",
                "AMBIGUOUS_RETURN_CFG_NODE",
                "RETURN_VALUE_CFG_PROOF_UNAVAILABLE",
            ]);
            if boundary
                .get("code")
                .and_then(Value::as_str)
                .is_none_or(|code| !allowed_codes.contains(code))
            {
                return Err(invalid("unknown typed return-value boundary code"));
            }
            for digest in ["ownerIdentityHash", "rootFirKindHash"] {
                if let Some(value) = boundary.get(digest) {
                    let value = value
                        .as_str()
                        .ok_or_else(|| invalid("return-value diagnostic hash is not a string"))?;
                    if !value.starts_with("sha256:") || value.len() != 71 {
                        return Err(invalid("return-value diagnostic hash is malformed"));
                    }
                }
            }
            if let Some(count) = boundary.get("nestedResolvedOccurrenceCount") {
                count.as_u64().ok_or_else(|| {
                    invalid("return-value occurrence diagnostic count is not nonnegative")
                })?;
            }
            if let Some(kinds) = boundary.get("nestedResolvedOccurrenceKindHashes") {
                let kinds = kinds.as_array().ok_or_else(|| {
                    invalid("return-value occurrence kind diagnostics are not an array")
                })?;
                if kinds.iter().any(|value| {
                    value
                        .as_str()
                        .is_none_or(|value| !value.starts_with("sha256:") || value.len() != 71)
                }) {
                    return Err(invalid("return-value occurrence kind hash is malformed"));
                }
            }
        }
    }

    let provenance = graph
        .get("provenance")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("declaration relation graph has no authority provenance"))?;
    if provenance.get("provider").and_then(Value::as_str) != Some("COMPILER_SEMANTIC_FACTS")
        || provenance.get("extractorSchema").and_then(Value::as_str)
            != Some("fir-facts-extractor/0.6")
        || required_string(provenance, "workerVersion").is_err()
        || required_string(provenance, "workerProtocolVersion").is_err()
    {
        return Err(invalid(
            "declaration relation authority provenance is incomplete",
        ));
    }
    let plugin = required_string(provenance, "pluginArtifactFingerprint")?;
    if !plugin.starts_with("sha256:") || plugin.len() != 71 {
        return Err(invalid(
            "declaration relation plugin fingerprint is malformed",
        ));
    }
    for (provenance_field, index_field) in [
        ("compilerVersion", "compilerVersion"),
        ("workerCompilerVersion", "compilerVersion"),
        ("projectModelHash", "projectModelHash"),
        ("classpathHash", "classpathHash"),
        ("compilerOptionsHash", "compilerOptionsHash"),
    ] {
        if required_string(provenance, provenance_field)?
            != facts
                .get(index_field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid(format!("worker index has no {index_field}")))?
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!(
                    "declaration relation {provenance_field} differs from worker index snapshot"
                ),
            ));
        }
    }
    let hash = required_string(facts, "declarationRelationHash")?.to_owned();
    let computed = canonical::hash(graph).map_err(internal)?;
    if hash != computed {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration relation hash differs from canonical Rust graph hash",
        ));
    }
    Ok(DeclarationRelationSnapshot {
        graph: graph.clone(),
        hash,
        provenance: provenance.clone(),
    })
}

pub(crate) fn validate_declaration_descriptor_snapshot(
    facts: &Value,
) -> Result<DeclarationDescriptorSnapshot, ClewError> {
    fn invalid(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::InvalidInput, message)
    }
    fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
        value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(format!("declaration descriptor graph has no {field}")))
    }
    fn canonical_rows(rows: &[Value], label: &str) -> Result<(), ClewError> {
        let encoded = rows
            .iter()
            .map(canonical::bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid(format!(
                "declaration descriptor {label} must be canonical, sorted, and unique"
            )));
        }
        Ok(())
    }
    fn safe_file(value: &Value) -> Result<(), ClewError> {
        let file = required_string(value, "file")?;
        let path = Path::new(file);
        if path.is_absolute()
            || path.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err(invalid(
                "declaration descriptor source path is not repository-contained",
            ));
        }
        Ok(())
    }
    fn nonempty_string_array(value: &Value, field: &str) -> Result<(), ClewError> {
        let items = value
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(format!("declaration descriptor has no {field}")))?;
        if items
            .iter()
            .any(|item| item.as_str().is_none_or(str::is_empty))
        {
            return Err(invalid(format!(
                "declaration descriptor {field} contains a non-string identity"
            )));
        }
        Ok(())
    }
    fn validate_type_parameters(value: &Value) -> Result<(), ClewError> {
        let parameters = value
            .get("typeParameters")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("declaration descriptor has no typeParameters"))?;
        for (index, parameter) in parameters.iter().enumerate() {
            if parameter.get("index").and_then(Value::as_u64) != Some(index as u64)
                || required_string(parameter, "compilerName").is_err()
            {
                return Err(invalid(
                    "declaration descriptor type parameter identity is invalid",
                ));
            }
            nonempty_string_array(parameter, "bounds")?;
            let bounds = parameter["bounds"].as_array().unwrap();
            let encoded = bounds.iter().map(Value::to_string).collect::<Vec<_>>();
            if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(invalid(
                    "declaration descriptor type parameter bounds are not canonical",
                ));
            }
        }
        Ok(())
    }
    fn validate_typed_value(
        value: &Value,
        type_field: &str,
        null_field: &str,
    ) -> Result<(), ClewError> {
        let rendered = required_string(value, type_field)?;
        let nullable = value
            .get(null_field)
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                invalid(format!(
                    "declaration descriptor has no boolean {null_field}"
                ))
            })?;
        if rendered.contains("..") || rendered.contains('!') || rendered.contains("<ERROR") {
            return Err(invalid(
                "flexible, platform, or unresolved compiler type cannot be PROVEN",
            ));
        }
        if nullable != rendered.trim_end().ends_with('?') {
            return Err(invalid(format!(
                "declaration descriptor {null_field} disagrees with compiler type rendering"
            )));
        }
        Ok(())
    }
    fn validate_field_closure(value: &Value, kind: &str) -> Result<(), ClewError> {
        let mut allowed = BTreeSet::from([
            "schema",
            "file",
            "start",
            "end",
            "symbolIdentity",
            "declarationKind",
            "ownerIdentity",
            "containment",
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "resolution",
            "provider",
            "module",
            "sourceSet",
            "sourceProvenance",
            "compilerAuthority",
            "typeParameters",
        ]);
        match kind {
            "FUNCTION" => allowed.extend([
                "compilerCallableId",
                "isOverride",
                "returnType",
                "returnNullable",
                "parameterTypes",
                "receiverType",
            ]),
            "CONSTRUCTOR" => allowed.extend([
                "compilerCallableId",
                "compilerClassId",
                "isPrimary",
                "jvmDescriptor",
                "parameterTypes",
            ]),
            "PROPERTY" | "MUTABLE_PROPERTY" => allowed.extend([
                "compilerCallableId",
                "isOverride",
                "declaredType",
                "declaredNullable",
            ]),
            "CLASS" => allowed.extend(["compilerClassId"]),
            _ => return Err(invalid("unknown declaration descriptor kind")),
        }
        let object = value
            .as_object()
            .ok_or_else(|| invalid("declaration descriptor row is not an object"))?;
        if object.keys().any(|field| !allowed.contains(field.as_str())) {
            return Err(invalid(
                "declaration descriptor has a field outside its kind-specific contract",
            ));
        }
        Ok(())
    }

    let graph = facts
        .get("declarationDescriptors")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("worker index has no declaration descriptor graph"))?;
    if graph.get("schema").and_then(Value::as_str) != Some("declaration-descriptor-graph/0.1") {
        return Err(invalid("unsupported declaration descriptor graph schema"));
    }
    let compilation = required_string(graph, "compilation")?;
    if facts.get("compilation").and_then(Value::as_str) != Some(compilation) {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration descriptor graph compilation differs from worker index snapshot",
        ));
    }
    if !matches!(
        graph.get("coverage").and_then(Value::as_str),
        Some("COMPLETE_SUPPORTED_SUBSET" | "PARTIAL")
    ) {
        return Err(invalid("unknown declaration descriptor coverage status"));
    }
    let descriptors = graph
        .get("descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration descriptor graph has no descriptors"))?;
    let source_binding = declaration_source_binding(facts)?;
    let bound_sources = source_binding["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            Ok((
                required_string(file, "path")?.to_owned(),
                (
                    required_string(file, "module")?.to_owned(),
                    required_string(file, "sourceSet")?.to_owned(),
                ),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ClewError>>()?;
    canonical_rows(descriptors, "rows")?;
    for descriptor in descriptors {
        if descriptor.get("schema").and_then(Value::as_str) != Some("declaration-descriptor/0.1")
            || descriptor.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || descriptor.get("provider").and_then(Value::as_str) != Some("K2_FIR")
            || descriptor.get("sourceProvenance").and_then(Value::as_str)
                != Some("COMPILER_SOURCE_RANGE")
            || descriptor.get("compilerAuthority").and_then(Value::as_str)
                != Some("fir-facts-extractor/0.6")
        {
            return Err(invalid(
                "malformed or non-authoritative declaration descriptor",
            ));
        }
        safe_file(descriptor)?;
        let descriptor_file = required_string(descriptor, "file")?;
        let bound_source = bound_sources
            .get(descriptor_file)
            .ok_or_else(|| invalid("declaration descriptor file is absent from source binding"))?;
        if descriptor.get("module").and_then(Value::as_str) != Some(bound_source.0.as_str())
            || descriptor.get("sourceSet").and_then(Value::as_str) != Some(bound_source.1.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "declaration descriptor module/sourceSet differs from source binding",
            ));
        }
        let start = descriptor
            .get("start")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        let end = descriptor.get("end").and_then(Value::as_i64).unwrap_or(-1);
        if start < 0 || end < start {
            return Err(invalid("declaration descriptor has invalid source range"));
        }
        for field in ["symbolIdentity", "ownerIdentity", "module", "sourceSet"] {
            required_string(descriptor, field)?;
        }
        nonempty_string_array(descriptor, "containment")?;
        let containment = descriptor["containment"].as_array().unwrap();
        let owner = descriptor
            .get("ownerIdentity")
            .and_then(Value::as_str)
            .unwrap();
        if containment
            .last()
            .and_then(Value::as_str)
            .map_or_else(|| !owner.starts_with("package:"), |last| last != owner)
        {
            return Err(invalid(
                "declaration descriptor owner differs from compiler containment",
            ));
        }
        if !matches!(
            descriptor.get("visibility").and_then(Value::as_str),
            Some("public" | "internal" | "private" | "protected")
        ) || !matches!(
            descriptor
                .get("effectiveVisibility")
                .and_then(Value::as_str),
            Some("public" | "internal" | "private-in-class" | "private-in-file" | "protected")
        ) || !matches!(
            descriptor.get("exportBoundary").and_then(Value::as_str),
            Some("PUBLIC_API" | "MODULE_API" | "PRIVATE_API")
        ) || !matches!(
            descriptor.get("modality").and_then(Value::as_str),
            Some("FINAL" | "OPEN" | "ABSTRACT" | "SEALED")
        ) {
            return Err(invalid(
                "declaration descriptor has an unknown compiler enum",
            ));
        }
        let expected_export = match descriptor
            .get("effectiveVisibility")
            .and_then(Value::as_str)
        {
            Some("public" | "protected") => "PUBLIC_API",
            Some("internal") => "MODULE_API",
            Some("private-in-class" | "private-in-file") => "PRIVATE_API",
            _ => unreachable!(),
        };
        if descriptor.get("exportBoundary").and_then(Value::as_str) != Some(expected_export) {
            return Err(invalid(
                "declaration descriptor export boundary disagrees with effective visibility",
            ));
        }
        validate_type_parameters(descriptor)?;
        let declaration_kind = descriptor
            .get("declarationKind")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("declaration descriptor has no declarationKind"))?;
        validate_field_closure(descriptor, declaration_kind)?;
        match declaration_kind {
            "FUNCTION" => {
                let callable = required_string(descriptor, "compilerCallableId")?;
                let identity = required_string(descriptor, "symbolIdentity")?;
                if !identity.starts_with(&format!("callable:{callable}#jvm:"))
                    || !descriptor.get("isOverride").is_some_and(Value::is_boolean)
                {
                    return Err(invalid(
                        "function descriptor compiler identity is inconsistent",
                    ));
                }
                validate_typed_value(descriptor, "returnType", "returnNullable")?;
                let parameters = descriptor
                    .get("parameterTypes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("function descriptor has no parameterTypes"))?;
                for (index, parameter) in parameters.iter().enumerate() {
                    if parameter.get("index").and_then(Value::as_u64) != Some(index as u64) {
                        return Err(invalid("function parameter indexes are not canonical"));
                    }
                    validate_typed_value(parameter, "type", "nullable")?;
                }
                if let Some(receiver) = descriptor.get("receiverType") {
                    validate_typed_value(receiver, "type", "nullable")?;
                }
            }
            "CONSTRUCTOR" => {
                let callable = required_string(descriptor, "compilerCallableId")?;
                let class = required_string(descriptor, "compilerClassId")?;
                let jvm = required_string(descriptor, "jvmDescriptor")?;
                if required_string(descriptor, "symbolIdentity")?
                    != format!("constructor:{callable}#jvm:{jvm}")
                    || required_string(descriptor, "ownerIdentity")? != format!("class:{class}")
                    || !descriptor.get("isPrimary").is_some_and(Value::is_boolean)
                    || !jvm.starts_with('(')
                    || !jvm.contains(')')
                {
                    return Err(invalid(
                        "constructor descriptor compiler/JVM identity is inconsistent",
                    ));
                }
                let parameters = descriptor
                    .get("parameterTypes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("constructor descriptor has no parameterTypes"))?;
                let mut indices = BTreeSet::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    let actual = parameter
                        .get("index")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| invalid("constructor parameter has no index"))?;
                    if actual != index as u64 || !indices.insert(actual) {
                        return Err(invalid(
                            "constructor parameter indexes are not canonical and unique",
                        ));
                    }
                    validate_typed_value(parameter, "type", "nullable")?;
                }
            }
            "PROPERTY" | "MUTABLE_PROPERTY" => {
                let callable = required_string(descriptor, "compilerCallableId")?;
                if required_string(descriptor, "symbolIdentity")? != format!("property:{callable}")
                    || !descriptor.get("isOverride").is_some_and(Value::is_boolean)
                {
                    return Err(invalid(
                        "property descriptor compiler identity is inconsistent",
                    ));
                }
                validate_typed_value(descriptor, "declaredType", "declaredNullable")?;
            }
            "CLASS" => {
                let class = required_string(descriptor, "compilerClassId")?;
                if required_string(descriptor, "symbolIdentity")? != format!("class:{class}") {
                    return Err(invalid(
                        "class descriptor compiler identity is inconsistent",
                    ));
                }
            }
            _ => return Err(invalid("unknown declaration descriptor kind")),
        }
    }

    let boundaries = graph
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration descriptor graph has no boundaries"))?;
    canonical_rows(boundaries, "boundaries")?;
    for boundary in boundaries {
        if boundary.get("schema").and_then(Value::as_str)
            != Some("declaration-descriptor-boundary/0.1")
            || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            || !matches!(
                boundary.get("provider").and_then(Value::as_str),
                Some("K2_FIR" | "COMPILER_DESCRIPTOR_NORMALIZER" | "WORKER")
            )
            || !matches!(
                boundary.get("stage").and_then(Value::as_str),
                Some("DECLARATION" | "CONSTRUCTOR_DECLARATION" | "NORMALIZE" | "ANALYSIS")
            )
            || !matches!(
                boundary.get("code").and_then(Value::as_str),
                Some(
                    "GENERATED_OR_NO_SOURCE"
                        | "LOCAL_DECLARATION_UNSUPPORTED"
                        | "LOCAL_GENERATED_OR_NO_SOURCE"
                        | "UNRESOLVED_DESCRIPTOR_BOUNDARY"
                        | "NO_COMPILER_CALLABLE_ID"
                        | "LOCAL_CONSTRUCTOR_UNSUPPORTED"
                        | "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR"
                        | "INCOMPLETE_COMPILER_DESCRIPTOR"
                        | "SYNTAX_ONLY"
                )
            )
            || required_string(boundary, "module").is_err()
            || required_string(boundary, "sourceSet").is_err()
            || boundary.get("compilerAuthority").and_then(Value::as_str)
                != Some("fir-facts-extractor/0.6")
        {
            return Err(invalid("malformed declaration descriptor Unknown boundary"));
        }
        if boundary.get("file").is_some() {
            safe_file(boundary)?;
            let file = required_string(boundary, "file")?;
            let bound_source = bound_sources.get(file).ok_or_else(|| {
                invalid("declaration descriptor boundary file is absent from source binding")
            })?;
            if boundary.get("module").and_then(Value::as_str) != Some(bound_source.0.as_str())
                || boundary.get("sourceSet").and_then(Value::as_str)
                    != Some(bound_source.1.as_str())
            {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "declaration descriptor boundary module/sourceSet differs from source binding",
                ));
            }
        }
    }

    let provenance = graph
        .get("provenance")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("declaration descriptor graph has no authority provenance"))?;
    if provenance.get("provider").and_then(Value::as_str) != Some("COMPILER_SEMANTIC_FACTS")
        || provenance.get("extractorSchema").and_then(Value::as_str)
            != Some("fir-facts-extractor/0.6")
        || required_string(provenance, "workerVersion").is_err()
        || required_string(provenance, "workerProtocolVersion").is_err()
    {
        return Err(invalid(
            "declaration descriptor authority provenance is incomplete",
        ));
    }
    let plugin = required_string(provenance, "pluginArtifactFingerprint")?;
    if !plugin.starts_with("sha256:") || plugin.len() != 71 {
        return Err(invalid(
            "declaration descriptor plugin fingerprint is malformed",
        ));
    }
    for (provenance_field, index_field) in [
        ("compilerVersion", "compilerVersion"),
        ("workerCompilerVersion", "compilerVersion"),
        ("projectModelHash", "projectModelHash"),
        ("classpathHash", "classpathHash"),
        ("compilerOptionsHash", "compilerOptionsHash"),
    ] {
        if required_string(provenance, provenance_field)?
            != facts
                .get(index_field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid(format!("worker index has no {index_field}")))?
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!(
                    "declaration descriptor {provenance_field} differs from worker index snapshot"
                ),
            ));
        }
    }
    let hash = required_string(facts, "declarationDescriptorHash")?.to_owned();
    if hash != canonical::hash(graph).map_err(internal)? {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration descriptor hash differs from canonical Rust graph hash",
        ));
    }
    Ok(DeclarationDescriptorSnapshot {
        graph: graph.clone(),
        hash,
        provenance: provenance.clone(),
    })
}

pub(crate) fn descriptor_validation_diagnostic(facts: &Value) -> Value {
    fn shape(value: Option<&Value>) -> Value {
        let json_type = match value {
            None => "MISSING",
            Some(Value::Null) => "NULL",
            Some(Value::Bool(_)) => "BOOLEAN",
            Some(Value::Number(_)) => "NUMBER",
            Some(Value::String(_)) => "STRING",
            Some(Value::Array(_)) => "ARRAY",
            Some(Value::Object(_)) => "OBJECT",
        };
        serde_json::json!({
            "present":value.is_some(),
            "jsonType":json_type,
            "arrayLength":value.and_then(Value::as_array).map(Vec::len),
            "valueHash":canonical::hash(value.unwrap_or(&Value::Null)).unwrap_or_else(|_| "unavailable".into()),
        })
    }
    fn typed(value: &Value, type_field: &str, nullable_field: &str) -> bool {
        let Some(rendered) = value.get(type_field).and_then(Value::as_str) else {
            return false;
        };
        let Some(nullable) = value.get(nullable_field).and_then(Value::as_bool) else {
            return false;
        };
        !rendered.contains("..")
            && !rendered.contains('!')
            && !rendered.contains("<ERROR")
            && nullable == rendered.trim_end().ends_with('?')
    }
    fn report(stage: &str, ordinal: usize, row: &Value) -> Value {
        let fields = [
            "schema",
            "resolution",
            "provider",
            "compilerAuthority",
            "declarationKind",
            "symbolIdentity",
            "ownerIdentity",
            "containment",
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "compilerCallableId",
            "compilerClassId",
            "jvmDescriptor",
            "typeParameters",
            "parameterTypes",
            "returnType",
            "returnNullable",
            "declaredType",
            "declaredNullable",
        ];
        let shapes = fields
            .into_iter()
            .map(|field| (field.to_owned(), shape(row.get(field))))
            .collect::<serde_json::Map<_, _>>();
        serde_json::json!({
            "schema":"descriptor-validation-diagnostic/0.1",
            "stage":stage,
            "ordinal":ordinal,
            "rowHash":canonical::hash(row).unwrap_or_else(|_| "unavailable".into()),
            "kind":row.get("declarationKind").and_then(Value::as_str).filter(|kind| matches!(*kind, "FUNCTION"|"CONSTRUCTOR"|"PROPERTY"|"MUTABLE_PROPERTY"|"CLASS")),
            "shapes":shapes,
        })
    }
    let descriptors = facts
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array);
    let Some(descriptors) = descriptors else {
        return report("DESCRIPTOR_KIND_IDENTITY", 0, &Value::Null);
    };
    for (ordinal, descriptor) in descriptors.iter().enumerate() {
        let kind = descriptor.get("declarationKind").and_then(Value::as_str);
        if descriptor.get("schema").and_then(Value::as_str) != Some("declaration-descriptor/0.1")
            || descriptor.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || descriptor.get("provider").and_then(Value::as_str) != Some("K2_FIR")
            || descriptor.get("sourceProvenance").and_then(Value::as_str)
                != Some("COMPILER_SOURCE_RANGE")
            || descriptor.get("compilerAuthority").and_then(Value::as_str)
                != Some("fir-facts-extractor/0.6")
            || !matches!(
                kind,
                Some("FUNCTION" | "CONSTRUCTOR" | "PROPERTY" | "MUTABLE_PROPERTY" | "CLASS")
            )
        {
            return report("DESCRIPTOR_KIND_IDENTITY", ordinal, descriptor);
        }
        let owner = descriptor.get("ownerIdentity").and_then(Value::as_str);
        let containment = descriptor.get("containment").and_then(Value::as_array);
        if owner.is_none()
            || containment.is_none()
            || containment.is_some_and(|values| {
                values.last().and_then(Value::as_str).map_or_else(
                    || !owner.is_some_and(|value| value.starts_with("package:")),
                    |last| Some(last) != owner,
                )
            })
        {
            return report("OWNER_CONTAINMENT", ordinal, descriptor);
        }
        let effective = descriptor
            .get("effectiveVisibility")
            .and_then(Value::as_str);
        let expected_export = match effective {
            Some("public" | "protected") => Some("PUBLIC_API"),
            Some("internal") => Some("MODULE_API"),
            Some("private-in-class" | "private-in-file") => Some("PRIVATE_API"),
            _ => None,
        };
        if !matches!(
            descriptor.get("visibility").and_then(Value::as_str),
            Some("public" | "internal" | "private" | "protected")
        ) || expected_export.is_none()
            || descriptor.get("exportBoundary").and_then(Value::as_str) != expected_export
            || !matches!(
                descriptor.get("modality").and_then(Value::as_str),
                Some("FINAL" | "OPEN" | "ABSTRACT" | "SEALED")
            )
        {
            return report("VISIBILITY_MODALITY", ordinal, descriptor);
        }
        let jvm_valid = match kind.unwrap() {
            "FUNCTION" => match (
                descriptor.get("compilerCallableId").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
            ) {
                (Some(callable), Some(identity)) => {
                    identity.starts_with(&format!("callable:{callable}#jvm:"))
                }
                _ => false,
            },
            "CONSTRUCTOR" => match (
                descriptor.get("compilerCallableId").and_then(Value::as_str),
                descriptor.get("compilerClassId").and_then(Value::as_str),
                descriptor.get("jvmDescriptor").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
                owner,
            ) {
                (Some(callable), Some(class), Some(jvm), Some(identity), Some(owner)) => {
                    identity == format!("constructor:{callable}#jvm:{jvm}")
                        && owner == format!("class:{class}")
                        && jvm.starts_with('(')
                        && jvm.contains(')')
                }
                _ => false,
            },
            "PROPERTY" | "MUTABLE_PROPERTY" => match (
                descriptor.get("compilerCallableId").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
            ) {
                (Some(callable), Some(identity)) => identity == format!("property:{callable}"),
                _ => false,
            },
            "CLASS" => match (
                descriptor.get("compilerClassId").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
            ) {
                (Some(class), Some(identity)) => identity == format!("class:{class}"),
                _ => false,
            },
            _ => false,
        };
        if !jvm_valid {
            return report("JVM_SIGNATURE", ordinal, descriptor);
        }
        if matches!(kind, Some("FUNCTION" | "CONSTRUCTOR")) {
            let Some(parameters) = descriptor.get("parameterTypes").and_then(Value::as_array)
            else {
                return report("PARAMETER_SLOTS", ordinal, descriptor);
            };
            if parameters.iter().enumerate().any(|(index, parameter)| {
                parameter.get("index").and_then(Value::as_u64) != Some(index as u64)
            }) {
                return report("PARAMETER_SLOTS", ordinal, descriptor);
            }
            if parameters
                .iter()
                .any(|parameter| !typed(parameter, "type", "nullable"))
            {
                return report("TYPE_NULLABILITY", ordinal, descriptor);
            }
        }
        if kind == Some("FUNCTION") && !typed(descriptor, "returnType", "returnNullable") {
            return report("TYPE_NULLABILITY", ordinal, descriptor);
        }
        if matches!(kind, Some("PROPERTY" | "MUTABLE_PROPERTY"))
            && !typed(descriptor, "declaredType", "declaredNullable")
        {
            return report("TYPE_NULLABILITY", ordinal, descriptor);
        }
    }
    if let Some(boundaries) = facts
        .pointer("/declarationDescriptors/boundaries")
        .and_then(Value::as_array)
    {
        for (ordinal, boundary) in boundaries.iter().enumerate() {
            if boundary.get("schema").and_then(Value::as_str)
                != Some("declaration-descriptor-boundary/0.1")
                || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            {
                return report("UNKNOWN_BOUNDARY", ordinal, boundary);
            }
        }
    }
    report("DESCRIPTOR_KIND_IDENTITY", descriptors.len(), &Value::Null)
}

fn exclude_runtime_state(repo: &Path) -> Result<(), ClewError> {
    let exclude = repo.join(".git/info/exclude");
    let Some(parent) = exclude.parent() else {
        return Ok(());
    };
    if !parent.is_dir() {
        // A linked worktree stores .git as a file; its common repository owns
        // the exclude policy, and leaving the cache visible is preferable to
        // guessing that location here.
        return Ok(());
    }
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if existing
        .lines()
        .any(|line| line.trim() == ".semantic-thread/")
    {
        return Ok(());
    }
    let separator = if !existing.is_empty() && !existing.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    std::fs::write(
        &exclude,
        format!("{existing}{separator}.semantic-thread/\n"),
    )
    .map_err(io_error)
}

fn database_name(compilation: Option<&str>) -> String {
    compilation.map_or_else(
        || "index.sqlite3".to_owned(),
        |unit| {
            let safe: String = unit
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() {
                        character
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("index{safe}.sqlite3")
        },
    )
}

const PROJECT_MODEL_FACT: &str = "input:project-model";
const CLASSPATH_FACT: &str = "input:classpath";
const COMPILER_VERSION_FACT: &str = "input:compiler-version";
const COMPILER_OPTIONS_FACT: &str = "input:compiler-options";
const SOURCE_SET_FACT: &str = "input:source-set";
pub const REPOSITORY_INDEX_FACT: &str = "view:repository-index";
pub const IDENTITY_VIEW_FACT: &str = "view:identity";
pub const INVALIDATION_VIEW_FACT: &str = "view:invalidations";

fn record_freshness_update(
    tx: &Transaction<'_>,
    facts: &Value,
    stored_files: &[Value],
    index_hash: &str,
    identity_report: &IdentityReport,
    compiler_version: &str,
    invalidations: &BTreeSet<String>,
) -> Result<(), ClewError> {
    let mut projection = load_freshness_projection(tx)?;
    let partial = facts.get("partial").and_then(Value::as_bool) == Some(true);
    if projection.checkpoint().sequence_gap.is_some() {
        if partial {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "partial index cannot recover a gapped freshness stream",
            ));
        }
        append_freshness_event(
            tx,
            &mut projection,
            provenance_for_reset(index_hash, identity_report, compiler_version)?,
            FreshnessEventKind::AuthoritativeReset {
                reason: "full-rebuild-after-gap".to_owned(),
            },
        )?;
    }
    let project_model_hash = identity_report.after.project_model_hash.clone();
    let classpath_hash = identity_report.after.classpath_hash.clone();
    let compiler_options_hash = identity_report.after.compiler_options_hash.clone();
    let provenance = FactProvenance {
        producer: "repository-index".to_owned(),
        composite_snapshot_hash: canonical::hash(&serde_json::json!({
            "index": index_hash,
            "projectModel": project_model_hash,
            "classpath": classpath_hash,
            "compilerVersion": compiler_version,
            "compilerOptions": compiler_options_hash,
        }))
        .map_err(internal)?,
        index_snapshot_hash: index_hash.to_owned(),
        project_model_hash: project_model_hash.clone(),
        classpath_hash: classpath_hash.clone(),
        compiler_version: compiler_version.to_owned(),
        compiler_options_hash: compiler_options_hash.clone(),
    };

    for fact in [
        DependencyFact {
            id: PROJECT_MODEL_FACT.to_owned(),
            domain: FactDomain::Build,
            fingerprint: project_model_hash,
            provenance: provenance.clone(),
            depends_on: vec![],
        },
        DependencyFact {
            id: CLASSPATH_FACT.to_owned(),
            domain: FactDomain::Classpath,
            fingerprint: classpath_hash,
            provenance: provenance.clone(),
            depends_on: vec![PROJECT_MODEL_FACT.to_owned()],
        },
        DependencyFact {
            id: COMPILER_VERSION_FACT.to_owned(),
            domain: FactDomain::Compiler,
            fingerprint: compiler_version.to_owned(),
            provenance: provenance.clone(),
            depends_on: vec![],
        },
        DependencyFact {
            id: COMPILER_OPTIONS_FACT.to_owned(),
            domain: FactDomain::Compiler,
            fingerprint: compiler_options_hash,
            provenance: provenance.clone(),
            depends_on: vec![COMPILER_VERSION_FACT.to_owned()],
        },
    ] {
        append_observation(tx, &mut projection, fact)?;
    }

    if partial {
        if projection.fact(SOURCE_SET_FACT).is_some() {
            append_invalidation(
                tx,
                &mut projection,
                provenance,
                SOURCE_SET_FACT,
                "partial-source-observation",
            )?;
        }
    } else {
        append_observation(
            tx,
            &mut projection,
            DependencyFact {
                id: SOURCE_SET_FACT.to_owned(),
                domain: FactDomain::Source,
                fingerprint: canonical::hash(&Value::Array(stored_files.to_vec()))
                    .map_err(internal)?,
                provenance: provenance.clone(),
                depends_on: vec![
                    PROJECT_MODEL_FACT.to_owned(),
                    CLASSPATH_FACT.to_owned(),
                    COMPILER_VERSION_FACT.to_owned(),
                    COMPILER_OPTIONS_FACT.to_owned(),
                ],
            },
        )?;
        append_observation(
            tx,
            &mut projection,
            DependencyFact {
                id: REPOSITORY_INDEX_FACT.to_owned(),
                domain: FactDomain::Source,
                fingerprint: index_hash.to_owned(),
                provenance: provenance.clone(),
                depends_on: vec![SOURCE_SET_FACT.to_owned()],
            },
        )?;
        append_observation(
            tx,
            &mut projection,
            DependencyFact {
                id: IDENTITY_VIEW_FACT.to_owned(),
                domain: FactDomain::Source,
                fingerprint: canonical::hash(identity_report).map_err(internal)?,
                provenance: provenance.clone(),
                depends_on: vec![REPOSITORY_INDEX_FACT.to_owned()],
            },
        )?;
        append_observation(
            tx,
            &mut projection,
            DependencyFact {
                id: INVALIDATION_VIEW_FACT.to_owned(),
                domain: FactDomain::Source,
                fingerprint: canonical::hash(invalidations).map_err(internal)?,
                provenance,
                depends_on: vec![
                    REPOSITORY_INDEX_FACT.to_owned(),
                    IDENTITY_VIEW_FACT.to_owned(),
                ],
            },
        )?;
    }
    store_freshness_checkpoint(tx, &projection.checkpoint())
}

fn provenance_for_reset(
    index_hash: &str,
    identity_report: &IdentityReport,
    compiler_version: &str,
) -> Result<FactProvenance, ClewError> {
    let project_model_hash = identity_report.after.project_model_hash.clone();
    let classpath_hash = identity_report.after.classpath_hash.clone();
    let compiler_options_hash = identity_report.after.compiler_options_hash.clone();
    Ok(FactProvenance {
        producer: "repository-index".to_owned(),
        composite_snapshot_hash: canonical::hash(&serde_json::json!({
            "index": index_hash,
            "projectModel": project_model_hash,
            "classpath": classpath_hash,
            "compilerVersion": compiler_version,
            "compilerOptions": compiler_options_hash,
        }))
        .map_err(internal)?,
        index_snapshot_hash: index_hash.to_owned(),
        project_model_hash,
        classpath_hash,
        compiler_version: compiler_version.to_owned(),
        compiler_options_hash,
    })
}

fn append_observation(
    tx: &Transaction<'_>,
    projection: &mut FreshnessProjection,
    fact: DependencyFact,
) -> Result<(), ClewError> {
    let provenance = fact.provenance.clone();
    append_freshness_event(
        tx,
        projection,
        provenance,
        FreshnessEventKind::Observed { fact },
    )
}

fn append_invalidation(
    tx: &Transaction<'_>,
    projection: &mut FreshnessProjection,
    provenance: FactProvenance,
    fact_id: &str,
    reason: &str,
) -> Result<(), ClewError> {
    append_freshness_event(
        tx,
        projection,
        provenance,
        FreshnessEventKind::Invalidated {
            fact_id: fact_id.to_owned(),
            reason: reason.to_owned(),
        },
    )
}

fn append_freshness_event(
    tx: &Transaction<'_>,
    projection: &mut FreshnessProjection,
    provenance: FactProvenance,
    event: FreshnessEventKind,
) -> Result<(), ClewError> {
    let sequence = projection
        .last_sequence()
        .checked_add(1)
        .ok_or_else(|| internal(crate::freshness::FreshnessError::SequenceOverflow.into()))?;
    let event_id = canonical::hash(&serde_json::json!({
        "schema":"freshness-event-id/0.1",
        "sequence":sequence,
        "provenance":provenance,
        "event":event,
    }))
    .map_err(internal)?;
    let event = FreshnessEvent {
        schema: FRESHNESS_EVENT_SCHEMA.to_owned(),
        event_id,
        sequence,
        provenance,
        event,
    };
    projection
        .ingest(event.clone())
        .map_err(|error| internal(error.into()))?;
    insert_freshness_event(tx, &event)
}

fn insert_freshness_event(
    connection: &Connection,
    event: &FreshnessEvent,
) -> Result<(), ClewError> {
    let bytes = canonical::bytes(event).map_err(internal)?;
    let hash = canonical::hash(event).map_err(internal)?;
    connection
        .execute(
            "INSERT INTO freshness_events(sequence,event_id,event_hash,event_json) VALUES(?1,?2,?3,?4)",
            params![
                i64::try_from(event.sequence).map_err(|error| internal(error.into()))?,
                event.event_id,
                hash,
                bytes
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

fn store_freshness_checkpoint(
    connection: &Connection,
    checkpoint: &FreshnessCheckpoint,
) -> Result<(), ClewError> {
    let json = canonical::pretty(checkpoint).map_err(internal)?;
    connection
        .execute(
            "INSERT INTO metadata(key,value) VALUES('freshness_checkpoint',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&json],
        )
        .map_err(db_error)?;
    Ok(())
}

fn load_freshness_projection(connection: &Connection) -> Result<FreshnessProjection, ClewError> {
    let checkpoint_json: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='freshness_checkpoint'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let events = load_freshness_events(connection)?;
    let replayed = FreshnessProjection::replay(events).map_err(|error| internal(error.into()))?;
    let Some(json) = checkpoint_json else {
        if replayed.last_sequence() != 0 {
            return Err(ClewError::new(
                ErrorCode::Internal,
                "freshness event log exists without a checkpoint",
            ));
        }
        return Ok(replayed);
    };
    let checkpoint: FreshnessCheckpoint =
        serde_json::from_str(&json).map_err(|error| internal(error.into()))?;
    let projection = FreshnessProjection::from_checkpoint(checkpoint.clone())
        .map_err(|error| internal(error.into()))?;
    let mut replayed_checkpoint = replayed.checkpoint();
    // The rejected future event is deliberately absent from the contiguous
    // log. Its durable gap marker overlays that otherwise reproducible state.
    replayed_checkpoint.sequence_gap = checkpoint.sequence_gap.clone();
    if replayed_checkpoint != checkpoint {
        return Err(ClewError::new(
            ErrorCode::Internal,
            "freshness checkpoint differs from deterministic event replay",
        ));
    }
    Ok(projection)
}

fn load_freshness_events(connection: &Connection) -> Result<Vec<FreshnessEvent>, ClewError> {
    let mut statement = connection
        .prepare("SELECT sequence,event_hash,event_json FROM freshness_events ORDER BY sequence")
        .map_err(db_error)?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(db_error)?
        .map(|row| {
            let (stored_sequence, expected_hash, bytes) = row.map_err(db_error)?;
            let event: FreshnessEvent =
                serde_json::from_slice(&bytes).map_err(|error| internal(error.into()))?;
            if u64::try_from(stored_sequence).ok() != Some(event.sequence) {
                return Err(ClewError::new(
                    ErrorCode::Internal,
                    format!("freshness event {} sequence mismatch", event.event_id),
                ));
            }
            let actual_hash = canonical::hash(&event).map_err(internal)?;
            if actual_hash != expected_hash {
                return Err(ClewError::new(
                    ErrorCode::Internal,
                    format!("freshness event {} hash mismatch", event.event_id),
                ));
            }
            Ok(event)
        })
        .collect()
}

fn record_identity_invalidations(report: &IdentityReport, invalidations: &mut BTreeSet<String>) {
    for decision in &report.decisions {
        let before = decision
            .before
            .iter()
            .map(|identity| identity.symbol_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let after = decision
            .after
            .iter()
            .map(|identity| identity.symbol_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        match decision.lifecycle {
            IdentityLifecycle::Same => {}
            IdentityLifecycle::Renamed => {
                invalidations.insert(format!("IDENTITY_RENAMED:{before}->{after}"));
            }
            IdentityLifecycle::Moved => {
                invalidations.insert(format!("IDENTITY_MOVED:{before}->{after}"));
            }
            IdentityLifecycle::Split => {
                invalidations.insert(format!("IDENTITY_SPLIT:{before}->{after}"));
            }
            IdentityLifecycle::Merged => {
                invalidations.insert(format!("IDENTITY_MERGED:{before}->{after}"));
            }
            IdentityLifecycle::Deleted => {
                invalidations.insert(format!("IDENTITY_DELETED:{before}"));
            }
            IdentityLifecycle::Ambiguous => {
                invalidations.insert(format!("IDENTITY_AMBIGUOUS:{before}->{after}"));
            }
        }
    }
    for identity in &report.introduced {
        invalidations.insert(format!("IDENTITY_INTRODUCED:{}", identity.symbol_id));
    }
}

fn classify_file_change(
    path: &str,
    before: &Value,
    after: &Value,
    invalidations: &mut BTreeSet<String>,
) {
    if before.get("package") != after.get("package")
        || before.get("imports") != after.get("imports")
    {
        invalidations.insert(format!("FILE_SEMANTICS:{path}"));
    }
    fn declarations(value: &Value) -> BTreeMap<String, &Value> {
        value
            .get("declarations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|declaration| {
                declaration
                    .get("symbolId")
                    .and_then(Value::as_str)
                    .map(|symbol| (symbol.to_owned(), declaration))
            })
            .collect()
    }
    let old = declarations(before);
    let new = declarations(after);
    for symbol in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        let Some(previous) = old.get(symbol) else {
            invalidations.insert(format!("DECLARATION_ADDED:{symbol}"));
            invalidations.insert(format!("CALLSITES:{symbol}"));
            continue;
        };
        let Some(current) = new.get(symbol) else {
            invalidations.insert(format!("DECLARATION_REMOVED:{symbol}"));
            invalidations.insert(format!("DOWNSTREAM_ABI:{symbol}"));
            continue;
        };
        if previous.get("bodyHash") != current.get("bodyHash") {
            for scope in ["LOCAL_CFG", "SSA", "REFERENCES", "FUNCTION_BODY"] {
                invalidations.insert(format!("{scope}:{symbol}"));
            }
        }
        if previous.get("semanticSummaryHash") != current.get("semanticSummaryHash") {
            invalidations.insert(format!("CALLER_SUMMARIES:{symbol}"));
        }
        if previous.get("sourceSignatureHash") != current.get("sourceSignatureHash") {
            for scope in ["CALLSITES", "OVERRIDES", "IMPLEMENTATIONS"] {
                invalidations.insert(format!("{scope}:{symbol}"));
            }
        }
        if previous.get("abiHash") != current.get("abiHash") {
            invalidations.insert(format!("DOWNSTREAM_ABI:{symbol}"));
        }
    }
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
fn db_error(error: rusqlite::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
fn internal(error: anyhow::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn relation_ingestion_facts(source: &str, target: &str) -> Value {
        let graph = json!({
            "schema":"declaration-relation-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "relations":[{
                "schema":"declaration-relation/0.1",
                "file":"A.kt",
                "start":0,
                "end":12,
                "kind":"OVERRIDES",
                "owner":"p/Derived.read",
                "target":target,
                "resolution":"PROVEN",
                "provider":"K2_FIR",
                "cfgNodeIds":[],
                "sourceProvenance":"COMPILER_SOURCE_RANGE",
                "orderProvenance":"UNKNOWN"
            }],
            "boundaries":[{
                "schema":"declaration-relation-boundary/0.1",
                "file":"A.kt",
                "start":0,
                "end":12,
                "owner":"p/Derived.label",
                "stage":"OVERRIDE",
                "code":"NON_FUNCTION_OVERRIDE_UNSUPPORTED",
                "resolution":"UNKNOWN",
                "provider":"K2_FIR"
            }],
            "provenance":{
                "provider":"COMPILER_SEMANTIC_FACTS",
                "extractorSchema":"fir-facts-extractor/0.6",
                "pluginArtifactFingerprint":format!("sha256:{}", "a".repeat(64)),
                "workerCompilerVersion":"2.4.10",
                "workerVersion":"0.1.0",
                "workerProtocolVersion":"1.0",
                "compilerVersion":"2.4.10",
                "projectModelHash":"model",
                "classpathHash":"classpath",
                "compilerOptionsHash":"options"
            }
        });
        json!({
            "compilation":":/main",
            "partial":false,
            "projectModelHash":"model",
            "classpathHash":"classpath",
            "compilerVersion":"2.4.10",
            "compilerOptionsHash":"options",
            "declarationRelationHash":canonical::hash(&graph).unwrap(),
            "declarationRelations":graph,
            "files":[{
                "path":"A.kt",
                "module":":",
                "sourceSet":"main",
                "contentHash":canonical::hash_bytes(source.as_bytes()),
                "declarations":[{"symbolId":"p/Derived.read"}],
                "semanticFacts":[]
            }]
        })
    }

    fn descriptor_ingestion_facts(source: &str, return_type: &str) -> Value {
        let mut facts = relation_ingestion_facts(source, "p/Base.read");
        let graph = json!({
            "schema":"declaration-descriptor-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "descriptors":[{
                "schema":"declaration-descriptor/0.1",
                "file":"A.kt",
                "start":0,
                "end":12,
                "symbolIdentity":"callable:p/Derived.read#jvm:()I",
                "declarationKind":"FUNCTION",
                "ownerIdentity":"class:p/Derived",
                "containment":["class:p/Derived"],
                "visibility":"public",
                "effectiveVisibility":"public",
                "exportBoundary":"PUBLIC_API",
                "modality":"FINAL",
                "compilerCallableId":"p/Derived.read",
                "isOverride":true,
                "returnType":return_type,
                "returnNullable":false,
                "parameterTypes":[],
                "typeParameters":[],
                "module":":",
                "sourceSet":"main",
                "sourceProvenance":"COMPILER_SOURCE_RANGE",
                "compilerAuthority":"fir-facts-extractor/0.6",
                "resolution":"PROVEN",
                "provider":"K2_FIR"
            }],
            "boundaries":[{
                "schema":"declaration-descriptor-boundary/0.1",
                "file":"A.kt",
                "start":4,
                "end":8,
                "stage":"DECLARATION",
                "code":"NO_COMPILER_CALLABLE_ID",
                "resolution":"UNKNOWN",
                "provider":"K2_FIR",
                "module":":",
                "sourceSet":"main",
                "compilerAuthority":"fir-facts-extractor/0.6"
            }],
            "provenance":{
                "provider":"COMPILER_SEMANTIC_FACTS",
                "extractorSchema":"fir-facts-extractor/0.6",
                "pluginArtifactFingerprint":format!("sha256:{}", "b".repeat(64)),
                "workerCompilerVersion":"2.4.10",
                "workerVersion":"0.1.0",
                "workerProtocolVersion":"1.0",
                "compilerVersion":"2.4.10",
                "projectModelHash":"model",
                "classpathHash":"classpath",
                "compilerOptionsHash":"options"
            }
        });
        facts["declarationDescriptorHash"] = Value::String(canonical::hash(&graph).unwrap());
        facts["declarationDescriptors"] = graph;
        facts
    }

    fn constructor_null_ingestion_facts(source: &str, fallback: &str) -> Value {
        let descriptor_graph = json!({
            "schema":"declaration-descriptor-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "descriptors":[{
                "schema":"declaration-descriptor/0.1",
                "file":"A.kt",
                "start":0,
                "end":20,
                "symbolIdentity":"constructor:p/NullableConstruction.<init>#jvm:(Ljava/lang/String;Ljava/lang/String;)V",
                "declarationKind":"CONSTRUCTOR",
                "ownerIdentity":"class:p/NullableConstruction",
                "containment":["class:p/NullableConstruction"],
                "visibility":"public",
                "effectiveVisibility":"public",
                "exportBoundary":"PUBLIC_API",
                "modality":"FINAL",
                "compilerCallableId":"p/NullableConstruction.<init>",
                "compilerClassId":"p/NullableConstruction",
                "isPrimary":true,
                "jvmDescriptor":"(Ljava/lang/String;Ljava/lang/String;)V",
                "parameterTypes":[
                    {"index":0,"type":"kotlin/String","nullable":false},
                    {"index":1,"type":"kotlin/String","nullable":false}
                ],
                "typeParameters":[],
                "module":":",
                "sourceSet":"main",
                "sourceProvenance":"COMPILER_SOURCE_RANGE",
                "compilerAuthority":"fir-facts-extractor/0.6",
                "resolution":"PROVEN",
                "provider":"K2_FIR"
            }],
            "boundaries":[{
                "schema":"declaration-descriptor-boundary/0.1",
                "file":"A.kt",
                "start":105,
                "end":120,
                "stage":"CONSTRUCTOR_DECLARATION",
                "code":"LOCAL_CONSTRUCTOR_UNSUPPORTED",
                "resolution":"UNKNOWN",
                "provider":"K2_FIR",
                "module":":",
                "sourceSet":"main",
                "compilerAuthority":"fir-facts-extractor/0.6"
            }],
            "provenance":{
                "provider":"COMPILER_SEMANTIC_FACTS",
                "extractorSchema":"fir-facts-extractor/0.6",
                "pluginArtifactFingerprint":format!("sha256:{}", "c".repeat(64)),
                "workerCompilerVersion":"2.4.10",
                "workerVersion":"0.1.0",
                "workerProtocolVersion":"1.0",
                "compilerVersion":"2.4.10",
                "projectModelHash":"model",
                "classpathHash":"classpath",
                "compilerOptionsHash":"options"
            }
        });
        let mut relation_graph = json!({
            "schema":"declaration-relation-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "relations":[
                {
                    "schema":"declaration-relation/0.1",
                    "file":"A.kt",
                    "start":20,
                    "end":100,
                    "kind":"CONSTRUCTS",
                    "owner":"p/build",
                    "target":"p/NullableConstruction.<init>",
                    "argumentToParameter":[
                        {"argumentStart":60,"argumentType":"kotlin/String","parameter":"second","parameterIndex":1,"parameterType":"kotlin/String"},
                        {"argumentStart":30,"argumentType":"kotlin/String","parameter":"first","parameterIndex":0,"parameterType":"kotlin/String"}
                    ],
                    "resultType":"p/NullableConstruction",
                    "orderKey":20,
                    "cfgNodeIds":[1],
                    "sourceProvenance":"COMPILER_SOURCE_RANGE",
                    "orderProvenance":"K2_FIR_CFG",
                    "resolution":"PROVEN",
                    "provider":"K2_FIR"
                },
                {
                    "schema":"declaration-relation/0.1",
                    "file":"A.kt",
                    "start":60,
                    "end":90,
                    "kind":"NULL_COALESCES",
                    "owner":"p/build",
                    "target":fallback,
                    "sourceTarget":"p/nullableSource",
                    "fallbackTarget":fallback,
                    "sourceOccurrence":{"start":60,"end":70,"type":"kotlin/String?","nullable":true},
                    "fallbackOccurrence":{"start":75,"end":90,"type":"kotlin/String","nullable":false},
                    "mergedOccurrence":{"start":60,"end":90,"type":"kotlin/String","nullable":false},
                    "branchProvenance":{"kind":"FIR_ELVIS_EXPRESSION","nullableBranchStart":60,"fallbackBranchStart":75,"mergeStart":60,"mergeEnd":90},
                    "orderKey":60,
                    "cfgNodeIds":[2,3],
                    "sourceProvenance":"COMPILER_SOURCE_RANGE",
                    "orderProvenance":"K2_FIR_CFG",
                    "resolution":"PROVEN",
                    "provider":"K2_FIR"
                }
            ],
            "boundaries":[{
                "schema":"declaration-relation-boundary/0.1",
                "file":"A.kt",
                "start":100,
                "end":120,
                "owner":"p/unsupported",
                "stage":"NULL_POLICY",
                "code":"SAFE_CALL_POLICY_UNSUPPORTED",
                "resolution":"UNKNOWN",
                "provider":"K2_FIR"
            }],
            "provenance":{
                "provider":"COMPILER_SEMANTIC_FACTS",
                "extractorSchema":"fir-facts-extractor/0.6",
                "pluginArtifactFingerprint":format!("sha256:{}", "d".repeat(64)),
                "workerCompilerVersion":"2.4.10",
                "workerVersion":"0.1.0",
                "workerProtocolVersion":"1.0",
                "compilerVersion":"2.4.10",
                "projectModelHash":"model",
                "classpathHash":"classpath",
                "compilerOptionsHash":"options"
            }
        });
        relation_graph["relations"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|value| canonical::bytes(value).unwrap());
        json!({
            "compilation":":/main",
            "partial":false,
            "projectModelHash":"model",
            "classpathHash":"classpath",
            "compilerVersion":"2.4.10",
            "compilerOptionsHash":"options",
            "declarationRelationHash":canonical::hash(&relation_graph).unwrap(),
            "declarationRelations":relation_graph,
            "declarationDescriptorHash":canonical::hash(&descriptor_graph).unwrap(),
            "declarationDescriptors":descriptor_graph,
            "files":[{
                "path":"A.kt",
                "module":":",
                "sourceSet":"main",
                "contentHash":canonical::hash_bytes(source.as_bytes()),
                "declarations":[{"symbolId":"p/build"}],
                "semanticFacts":[]
            }]
        })
    }

    fn refresh_constructor_null_hashes(facts: &mut Value) {
        facts["declarationDescriptorHash"] =
            Value::String(canonical::hash(&facts["declarationDescriptors"]).unwrap());
        facts["declarationRelationHash"] =
            Value::String(canonical::hash(&facts["declarationRelations"]).unwrap());
    }

    fn return_value_ingestion_facts(source: &str, source_target: &str) -> Value {
        let function_descriptor = |callable: &str, start: u64, end: u64| {
            json!({
                "schema":"declaration-descriptor/0.1",
                "file":"A.kt",
                "start":start,
                "end":end,
                "symbolIdentity":format!("callable:{callable}#jvm:()Ljava/lang/String;"),
                "declarationKind":"FUNCTION",
                "ownerIdentity":"package:p",
                "containment":[],
                "visibility":"internal",
                "effectiveVisibility":"internal",
                "exportBoundary":"MODULE_API",
                "modality":"FINAL",
                "compilerCallableId":callable,
                "isOverride":false,
                "returnType":"kotlin/String",
                "returnNullable":false,
                "parameterTypes":[],
                "typeParameters":[],
                "module":":",
                "sourceSet":"main",
                "sourceProvenance":"COMPILER_SOURCE_RANGE",
                "compilerAuthority":"fir-facts-extractor/0.6",
                "resolution":"PROVEN",
                "provider":"K2_FIR"
            })
        };
        let mut descriptors = vec![
            function_descriptor("p/project", 0, 32),
            function_descriptor("p/read", 33, 40),
            function_descriptor("p/projectCall", 41, 72),
            json!({
                "schema":"declaration-descriptor/0.1",
                "file":"A.kt",
                "start":73,
                "end":88,
                "symbolIdentity":format!("property:{source_target}"),
                "declarationKind":"PROPERTY",
                "ownerIdentity":"package:p",
                "containment":[],
                "visibility":"internal",
                "effectiveVisibility":"internal",
                "exportBoundary":"MODULE_API",
                "modality":"FINAL",
                "compilerCallableId":source_target,
                "isOverride":false,
                "declaredType":"kotlin/String",
                "declaredNullable":false,
                "typeParameters":[],
                "module":":",
                "sourceSet":"main",
                "sourceProvenance":"COMPILER_SOURCE_RANGE",
                "compilerAuthority":"fir-facts-extractor/0.6",
                "resolution":"PROVEN",
                "provider":"K2_FIR"
            }),
        ];
        descriptors.sort_by_key(|value| canonical::bytes(value).unwrap());
        let descriptor_graph = json!({
            "schema":"declaration-descriptor-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "descriptors":descriptors,
            "boundaries":[],
            "provenance":{
                "provider":"COMPILER_SEMANTIC_FACTS",
                "extractorSchema":"fir-facts-extractor/0.6",
                "pluginArtifactFingerprint":format!("sha256:{}", "e".repeat(64)),
                "workerCompilerVersion":"2.4.10",
                "workerVersion":"0.1.0",
                "workerProtocolVersion":"1.0",
                "compilerVersion":"2.4.10",
                "projectModelHash":"model",
                "classpathHash":"classpath",
                "compilerOptionsHash":"options"
            }
        });
        let mut relations = vec![
            json!({
                "schema":"declaration-relation/0.1",
                "file":"A.kt","start":12,"end":20,"kind":"READS",
                "owner":"p/project","target":source_target,
                "resultType":"kotlin/String","argumentToParameter":[],"orderKey":12,
                "cfgNodeIds":[2],"sourceProvenance":"COMPILER_SOURCE_RANGE",
                "orderProvenance":"K2_FIR_CFG","resolution":"PROVEN","provider":"K2_FIR"
            }),
            json!({
                "schema":"declaration-relation/0.1",
                "file":"A.kt","start":8,"end":24,"kind":"RETURNS_VALUE_FROM",
                "owner":"p/project","target":source_target,"sourceKind":"PROPERTY_READ",
                "sourceOccurrence":{"start":12,"end":20,"cfgNodeId":2},
                "returnOccurrence":{"start":8,"end":24,"cfgNodeId":3},
                "resultOccurrence":{"start":12,"end":20,"cfgNodeId":2},
                "resultType":"kotlin/String","resultNullable":false,
                "valueProvenance":"FIR_RETURN_RESULT_IDENTITY",
                "cfgProvenance":{"graphName":"p/project","sourceReachesReturn":true,
                    "sourceDominatesReturn":true,"sourceNodeKind":"QualifiedAccessNode",
                    "returnNodeKind":"JumpNode"},
                "evaluationCount":1,"orderKey":12,"cfgNodeIds":[2,3],
                "sourceProvenance":"COMPILER_SOURCE_RANGE","orderProvenance":"K2_FIR_CFG",
                "resolution":"PROVEN","provider":"K2_FIR"
            }),
            json!({
                "schema":"declaration-relation/0.1",
                "file":"A.kt","start":52,"end":60,"kind":"CALLS",
                "owner":"p/projectCall","target":"p/read","resultType":"kotlin/String",
                "argumentToParameter":[],"orderKey":52,"cfgNodeIds":[4],
                "sourceProvenance":"COMPILER_SOURCE_RANGE","orderProvenance":"K2_FIR_CFG",
                "resolution":"PROVEN","provider":"K2_FIR"
            }),
            json!({
                "schema":"declaration-relation/0.1",
                "file":"A.kt","start":48,"end":64,"kind":"RETURNS_VALUE_FROM",
                "owner":"p/projectCall","target":"p/read","sourceKind":"FUNCTION_CALL_RESULT",
                "sourceOccurrence":{"start":52,"end":60,"cfgNodeId":4},
                "returnOccurrence":{"start":48,"end":64,"cfgNodeId":5},
                "resultOccurrence":{"start":52,"end":60,"cfgNodeId":4},
                "resultType":"kotlin/String","resultNullable":false,
                "valueProvenance":"FIR_RETURN_RESULT_IDENTITY",
                "cfgProvenance":{"graphName":"p/projectCall","sourceReachesReturn":true,
                    "sourceDominatesReturn":true,"sourceNodeKind":"FunctionCallExitNode",
                    "returnNodeKind":"JumpNode"},
                "evaluationCount":1,"orderKey":52,"cfgNodeIds":[4,5],
                "sourceProvenance":"COMPILER_SOURCE_RANGE","orderProvenance":"K2_FIR_CFG",
                "resolution":"PROVEN","provider":"K2_FIR"
            }),
        ];
        relations.sort_by_key(|value| canonical::bytes(value).unwrap());
        let relation_graph = json!({
            "schema":"declaration-relation-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "relations":relations,
            "boundaries":[{
                "schema":"declaration-relation-boundary/0.1","file":"A.kt",
                "start":90,"end":96,"owner":"p/unsupported","stage":"RETURN_VALUE",
                "code":"NON_LINEAR_OR_MULTIPLE_RETURN_FLOW","resolution":"UNKNOWN",
                "provider":"K2_FIR"
            }],
            "provenance":{
                "provider":"COMPILER_SEMANTIC_FACTS",
                "extractorSchema":"fir-facts-extractor/0.6",
                "pluginArtifactFingerprint":format!("sha256:{}", "f".repeat(64)),
                "workerCompilerVersion":"2.4.10",
                "workerVersion":"0.1.0",
                "workerProtocolVersion":"1.0",
                "compilerVersion":"2.4.10",
                "projectModelHash":"model",
                "classpathHash":"classpath",
                "compilerOptionsHash":"options"
            }
        });
        json!({
            "compilation":":/main","partial":false,"projectModelHash":"model",
            "classpathHash":"classpath","compilerVersion":"2.4.10",
            "compilerOptionsHash":"options",
            "declarationRelationHash":canonical::hash(&relation_graph).unwrap(),
            "declarationRelations":relation_graph,
            "declarationDescriptorHash":canonical::hash(&descriptor_graph).unwrap(),
            "declarationDescriptors":descriptor_graph,
            "files":[{"path":"A.kt","module":":","sourceSet":"main",
                "contentHash":canonical::hash_bytes(source.as_bytes()),
                "declarations":[{"symbolId":"p/project"}],"semanticFacts":[]}]
        })
    }

    fn refresh_return_value_hashes(facts: &mut Value) {
        for graph in ["declarationDescriptors", "declarationRelations"] {
            let rows = if graph == "declarationDescriptors" {
                &mut facts[graph]["descriptors"]
            } else {
                &mut facts[graph]["relations"]
            };
            rows.as_array_mut()
                .unwrap()
                .sort_by_key(|value| canonical::bytes(value).unwrap());
        }
        facts["declarationDescriptorHash"] =
            Value::String(canonical::hash(&facts["declarationDescriptors"]).unwrap());
        facts["declarationRelationHash"] =
            Value::String(canonical::hash(&facts["declarationRelations"]).unwrap());
    }

    fn return_relation_mut<'a>(facts: &'a mut Value, owner: &str) -> &'a mut Value {
        facts["declarationRelations"]["relations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["kind"] == "RETURNS_VALUE_FROM" && row["owner"] == owner)
            .unwrap()
    }

    fn source_relation_mut<'a>(facts: &'a mut Value, owner: &str, kind: &str) -> &'a mut Value {
        facts["declarationRelations"]["relations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["kind"] == kind && row["owner"] == owner)
            .unwrap()
    }

    fn reject_return_value_mutation(valid: &Value, mutate: impl FnOnce(&mut Value)) {
        let mut malformed = valid.clone();
        mutate(&mut malformed);
        refresh_return_value_hashes(&mut malformed);
        assert!(
            validate_declaration_descriptor_snapshot(&malformed).is_err()
                || validate_declaration_relation_snapshot(&malformed).is_err(),
            "malformed return-value fact unexpectedly passed authority validation"
        );
    }

    #[test]
    fn return_value_fact_ingestion_roundtrips_and_commits_fact_only_delta() {
        let temp = tempfile::tempdir().unwrap();
        let source = " ".repeat(128);
        std::fs::write(temp.path().join("A.kt"), &source).unwrap();
        let first = return_value_ingestion_facts(&source, "p/source");
        validate_declaration_descriptor_snapshot(&first).unwrap();
        validate_declaration_relation_snapshot(&first).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        let first_snapshot = index.update(&first).unwrap();
        let stored = index.declaration_relations().unwrap().unwrap();
        assert_eq!(stored.graph, first["declarationRelations"]);
        assert_eq!(stored.hash, first["declarationRelationHash"]);
        assert_eq!(stored.graph["boundaries"][0]["resolution"], "UNKNOWN");

        let second = return_value_ingestion_facts(&source, "p/otherSource");
        let second_snapshot = index.update(&second).unwrap();
        assert_ne!(first_snapshot, second_snapshot);
        assert_eq!(
            index.declaration_relations().unwrap().unwrap().graph,
            second["declarationRelations"]
        );
        assert!(
            index
                .invalidations()
                .unwrap()
                .contains(&"DECLARATION_RELATIONS".to_owned())
        );
    }

    #[test]
    fn return_value_fact_ingestion_rejects_malformed_or_unproven_facts() {
        let source = " ".repeat(128);
        let valid = return_value_ingestion_facts(&source, "p/source");
        validate_declaration_descriptor_snapshot(&valid).unwrap();
        validate_declaration_relation_snapshot(&valid).unwrap();

        let mut hash = valid.clone();
        hash["declarationRelationHash"] = json!("sha256:forged");
        assert_eq!(
            validate_declaration_relation_snapshot(&hash)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );

        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["unexpected"] = json!(true);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["target"] = json!("p/decoy");
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["owner"] = json!("p/decoyOwner");
        });
        reject_return_value_mutation(&valid, |facts| {
            facts["declarationRelations"]["provenance"]["extractorSchema"] =
                json!("fir-facts-extractor/0.5");
        });
        reject_return_value_mutation(&valid, |facts| {
            facts["declarationRelations"]["provenance"]["provider"] = json!("CALLER");
        });
        reject_return_value_mutation(&valid, |facts| {
            facts["declarationRelations"]["relations"]
                .as_array_mut()
                .unwrap()
                .retain(|row| !(row["kind"] == "READS" && row["owner"] == "p/project"));
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["sourceOccurrence"]["start"] = json!(13);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["returnOccurrence"]["end"] = json!(20);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["resultOccurrence"]["end"] = json!(19);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["resultType"] = json!("kotlin/Any");
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["resultNullable"] = json!(true);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["sourceOccurrence"]["cfgNodeId"] = json!(99);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["cfgProvenance"]["sourceDominatesReturn"] =
                json!(false);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["cfgProvenance"]["sourceReachesReturn"] =
                json!(false);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["evaluationCount"] = json!(2);
        });
        reject_return_value_mutation(&valid, |facts| {
            return_relation_mut(facts, "p/project")["resolution"] = json!("UNKNOWN");
        });
        reject_return_value_mutation(&valid, |facts| {
            source_relation_mut(facts, "p/project", "READS")["resultType"] = json!("kotlin/Any");
        });
        reject_return_value_mutation(&valid, |facts| {
            source_relation_mut(facts, "p/project", "READS")["cfgNodeIds"] = json!([99]);
        });
        reject_return_value_mutation(&valid, |facts| {
            facts["declarationDescriptors"]["descriptors"]
                .as_array_mut()
                .unwrap()
                .retain(|row| row["compilerCallableId"] != "p/source");
        });
        reject_return_value_mutation(&valid, |facts| {
            facts["declarationRelations"]["boundaries"][0]["code"] = json!("UNTYPED_CALLER_REASON");
        });
    }

    #[test]
    fn constructor_null_fact_ingestion_roundtrips_and_commits_fact_delta() {
        let temp = tempfile::tempdir().unwrap();
        let source = " ".repeat(160);
        std::fs::write(temp.path().join("A.kt"), &source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        let first = constructor_null_ingestion_facts(&source, "p/fallback");
        let first_snapshot = index.update(&first).unwrap();
        assert_eq!(
            index.declaration_descriptors().unwrap().unwrap().graph,
            first["declarationDescriptors"]
        );
        assert_eq!(
            index.declaration_relations().unwrap().unwrap().graph,
            first["declarationRelations"]
        );
        assert_eq!(
            index.declaration_descriptors().unwrap().unwrap().graph["boundaries"][0]["resolution"],
            "UNKNOWN"
        );
        assert_eq!(
            index.declaration_relations().unwrap().unwrap().graph["boundaries"][0]["resolution"],
            "UNKNOWN"
        );
        let binding_hash: String = index
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='declaration_source_binding_hash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        index
            .connection
            .execute(
                "UPDATE metadata SET value='sha256:forged' WHERE key='declaration_source_binding_hash'",
                [],
            )
            .unwrap();
        assert_eq!(
            index.declaration_relations().unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );
        index
            .connection
            .execute(
                "UPDATE metadata SET value=?1 WHERE key='declaration_source_binding_hash'",
                [&binding_hash],
            )
            .unwrap();

        let second = constructor_null_ingestion_facts(&source, "p/otherFallback");
        let second_snapshot = index.update(&second).unwrap();
        assert_ne!(first_snapshot, second_snapshot);
        assert!(
            index
                .invalidations()
                .unwrap()
                .contains(&"DECLARATION_RELATIONS".to_owned())
        );
    }

    #[test]
    fn constructor_null_fact_ingestion_rejects_malformed_or_unproven_facts() {
        let source = " ".repeat(160);
        let valid = constructor_null_ingestion_facts(&source, "p/fallback");
        validate_declaration_descriptor_snapshot(&valid).unwrap();
        validate_declaration_relation_snapshot(&valid).unwrap();

        let mut hash = valid.clone();
        hash["declarationRelationHash"] = json!("sha256:forged");
        assert_eq!(
            validate_declaration_relation_snapshot(&hash)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );

        let mutations = [
            (
                "/declarationDescriptors/descriptors/0/declarationKind",
                json!("FUNCTION"),
            ),
            (
                "/declarationDescriptors/descriptors/0/ownerIdentity",
                json!("class:p/Decoy"),
            ),
            (
                "/declarationDescriptors/descriptors/0/compilerAuthority",
                json!("fir-facts-extractor/0.4"),
            ),
            (
                "/declarationDescriptors/descriptors/0/parameterTypes/1/index",
                json!(0),
            ),
            (
                "/declarationDescriptors/descriptors/0/parameterTypes/1/nullable",
                json!(true),
            ),
            (
                "/declarationRelations/relations/0/argumentToParameter/0/parameterIndex",
                json!(4),
            ),
            (
                "/declarationRelations/relations/1/sourceOccurrence/nullable",
                json!(false),
            ),
            (
                "/declarationRelations/relations/1/fallbackOccurrence/nullable",
                json!(true),
            ),
            (
                "/declarationRelations/relations/1/branchProvenance/fallbackBranchStart",
                json!(76),
            ),
            (
                "/declarationRelations/relations/1/mergedOccurrence/end",
                json!(89),
            ),
            (
                "/declarationRelations/relations/1/resolution",
                json!("UNKNOWN"),
            ),
            (
                "/declarationRelations/provenance/extractorSchema",
                json!("fir-facts-extractor/0.4"),
            ),
            ("/files/0/sourceSet", json!("test")),
        ];
        for (pointer, replacement) in mutations {
            let mut malformed = valid.clone();
            *malformed.pointer_mut(pointer).unwrap() = replacement;
            refresh_constructor_null_hashes(&mut malformed);
            assert!(
                validate_declaration_descriptor_snapshot(&malformed).is_err()
                    || validate_declaration_relation_snapshot(&malformed).is_err(),
                "malformed fact unexpectedly accepted at {pointer}"
            );
        }

        let mut missing_slot = valid.clone();
        missing_slot["declarationDescriptors"]["descriptors"][0]["parameterTypes"]
            .as_array_mut()
            .unwrap()
            .pop();
        refresh_constructor_null_hashes(&mut missing_slot);
        assert!(validate_declaration_relation_snapshot(&missing_slot).is_err());
    }

    #[test]
    fn declaration_descriptor_ingestion_roundtrips_unknown_and_commits_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun read() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        let first = descriptor_ingestion_facts(source, "kotlin/Int");
        let first_snapshot = index.update(&first).unwrap();
        let stored = index.declaration_descriptors().unwrap().unwrap();
        assert_eq!(stored.graph, first["declarationDescriptors"]);
        assert_eq!(stored.hash, first["declarationDescriptorHash"]);
        assert_eq!(stored.graph["boundaries"][0]["resolution"], "UNKNOWN");
        assert_eq!(
            index.hash().unwrap().as_deref(),
            Some(first_snapshot.as_str())
        );

        let second = descriptor_ingestion_facts(source, "kotlin/Number");
        let second_snapshot = index.update(&second).unwrap();
        assert_ne!(first_snapshot, second_snapshot);
        assert_eq!(
            index.declaration_descriptors().unwrap().unwrap().graph,
            second["declarationDescriptors"]
        );
        assert!(
            index
                .invalidations()
                .unwrap()
                .contains(&"DECLARATION_DESCRIPTORS".to_owned())
        );
    }

    #[test]
    fn declaration_descriptor_ingestion_rejects_malformed_hash_and_provenance() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun read() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();

        let mut hash_mismatch = descriptor_ingestion_facts(source, "kotlin/Int");
        hash_mismatch["declarationDescriptorHash"] = Value::String("sha256:forged".into());
        assert_eq!(
            index.update(&hash_mismatch).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );

        for (pointer, replacement) in [
            (
                "/declarationDescriptors/descriptors/0/declarationKind",
                json!("SOURCE_RECIPE"),
            ),
            (
                "/declarationDescriptors/descriptors/0/provider",
                json!("CALLER"),
            ),
            (
                "/declarationDescriptors/descriptors/0/file",
                json!("../A.kt"),
            ),
            (
                "/declarationDescriptors/descriptors/0/visibility",
                json!("package"),
            ),
            (
                "/declarationDescriptors/descriptors/0/modality",
                json!("DYNAMIC"),
            ),
            (
                "/declarationDescriptors/descriptors/0/returnNullable",
                json!("false"),
            ),
            (
                "/declarationDescriptors/boundaries/0/code",
                json!("CALLER_DECISION"),
            ),
        ] {
            let mut malformed = descriptor_ingestion_facts(source, "kotlin/Int");
            *malformed.pointer_mut(pointer).unwrap() = replacement;
            malformed["declarationDescriptorHash"] =
                Value::String(canonical::hash(&malformed["declarationDescriptors"]).unwrap());
            assert_eq!(
                index.update(&malformed).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }

        let mut provenance = descriptor_ingestion_facts(source, "kotlin/Int");
        provenance["declarationDescriptors"]["provenance"]["projectModelHash"] =
            Value::String("other-model".into());
        provenance["declarationDescriptorHash"] =
            Value::String(canonical::hash(&provenance["declarationDescriptors"]).unwrap());
        assert_eq!(
            index.update(&provenance).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );
        assert!(index.hash().unwrap().is_none());
    }

    #[test]
    fn verified_index_facts_reject_descriptor_semantic_inconsistency() {
        let source = "fun read() = 1\n";
        for (pointer, replacement) in [
            (
                "/declarationDescriptors/descriptors/0/exportBoundary",
                json!("PRIVATE_API"),
            ),
            (
                "/declarationDescriptors/descriptors/0/returnNullable",
                json!(true),
            ),
            (
                "/declarationDescriptors/descriptors/0/ownerIdentity",
                json!("class:p/Decoy"),
            ),
        ] {
            let mut facts = descriptor_ingestion_facts(source, "kotlin/Int");
            *facts.pointer_mut(pointer).unwrap() = replacement;
            facts["declarationDescriptorHash"] =
                Value::String(canonical::hash(&facts["declarationDescriptors"]).unwrap());
            assert_eq!(
                validate_declaration_descriptor_snapshot(&facts)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
        }

        let mut platform = descriptor_ingestion_facts(source, "kotlin/String!");
        platform["declarationDescriptorHash"] =
            Value::String(canonical::hash(&platform["declarationDescriptors"]).unwrap());
        assert_eq!(
            validate_declaration_descriptor_snapshot(&platform)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn declaration_relation_ingestion_roundtrips_typed_unknown_and_commits_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun read() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        let first = relation_ingestion_facts(source, "p/Base.read");
        let first_snapshot = index.update(&first).unwrap();
        let stored = index.declaration_relations().unwrap().unwrap();
        assert_eq!(stored.graph, first["declarationRelations"]);
        assert_eq!(stored.hash, first["declarationRelationHash"]);
        assert_eq!(stored.graph["boundaries"][0]["resolution"], "UNKNOWN");
        assert_eq!(
            index.hash().unwrap().as_deref(),
            Some(first_snapshot.as_str())
        );

        let second = relation_ingestion_facts(source, "p/OtherBase.read");
        let second_snapshot = index.update(&second).unwrap();
        assert_ne!(first_snapshot, second_snapshot);
        assert_eq!(
            index.declaration_relations().unwrap().unwrap().graph,
            second["declarationRelations"]
        );
        assert!(
            index
                .invalidations()
                .unwrap()
                .contains(&"DECLARATION_RELATIONS".to_owned())
        );
    }

    #[test]
    fn declaration_relation_ingestion_rejects_hash_malformed_and_snapshot_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun read() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();

        let mut hash_mismatch = relation_ingestion_facts(source, "p/Base.read");
        hash_mismatch["declarationRelationHash"] = Value::String("sha256:forged".into());
        assert_eq!(
            index.update(&hash_mismatch).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );

        for (pointer, replacement) in [
            (
                "/declarationRelations/relations/0/kind",
                json!("LEXICAL_GUESS"),
            ),
            (
                "/declarationRelations/relations/0/resolution",
                json!("UNKNOWN"),
            ),
            (
                "/declarationRelations/relations/0/provider",
                json!("CALLER"),
            ),
            (
                "/declarationRelations/boundaries/0/provider",
                json!("CALLER"),
            ),
        ] {
            let mut malformed = relation_ingestion_facts(source, "p/Base.read");
            *malformed.pointer_mut(pointer).unwrap() = replacement;
            malformed["declarationRelationHash"] =
                Value::String(canonical::hash(&malformed["declarationRelations"]).unwrap());
            assert_eq!(
                index.update(&malformed).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }

        let mut snapshot_mismatch = relation_ingestion_facts(source, "p/Base.read");
        snapshot_mismatch["declarationRelations"]["provenance"]["projectModelHash"] =
            Value::String("other-model".into());
        snapshot_mismatch["declarationRelationHash"] =
            Value::String(canonical::hash(&snapshot_mismatch["declarationRelations"]).unwrap());
        assert_eq!(
            index.update(&snapshot_mismatch).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );
        assert!(index.hash().unwrap().is_none());
    }

    fn freshness_facts(source: &str, partial: bool, compiler_version: &str) -> Value {
        json!({
            "partial":partial,
            "projectModelHash":"model",
            "classpathHash":"classpath",
            "compilerVersion":compiler_version,
            "compilerOptionsHash":"options",
            "files":[{
                "path":"A.kt",
                "contentHash":canonical::hash_bytes(source.as_bytes()),
                "declarations":[{"symbolId":"a"}],
                "semanticFacts":[]
            }]
        })
    }

    #[test]
    fn staged_index_is_invisible_until_atomic_publish() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("A.kt");
        let facts = |source: &str| {
            json!({"files":[{
                "path":"A.kt",
                "contentHash":canonical::hash_bytes(source.as_bytes()),
                "declarations":[{"symbolId":"a"}],
                "semanticFacts":[]
            }]})
        };
        std::fs::write(&source_path, "fun a() = 1\n").unwrap();
        let mut live = RepositoryIndex::open(temp.path()).unwrap();
        let old_hash = live.update(&facts("fun a() = 1\n")).unwrap();
        let old_identity_snapshot = live
            .identity_report()
            .unwrap()
            .unwrap()
            .after
            .index_snapshot_hash;
        live.mark_published_revision("old").unwrap();
        let old_freshness = live.freshness_checkpoint().unwrap();
        drop(live);

        std::fs::write(&source_path, "fun a() = 2\n").unwrap();
        let stage = RepositoryIndex::stage_update_unchecked_for_test(
            temp.path(),
            None,
            &facts("fun a() = 2\n"),
            temp.path(),
            "new",
        )
        .unwrap();
        let new_hash = stage.hash().to_owned();
        let visible = RepositoryIndex::open(temp.path()).unwrap();
        assert_eq!(visible.hash().unwrap().as_deref(), Some(old_hash.as_str()));
        assert_eq!(
            visible.published_revision().unwrap().as_deref(),
            Some("old")
        );
        assert_eq!(
            visible
                .identity_report()
                .unwrap()
                .unwrap()
                .after
                .index_snapshot_hash,
            old_identity_snapshot
        );
        assert_eq!(visible.freshness_checkpoint().unwrap(), old_freshness);
        drop(visible);

        stage.publish().unwrap();
        let visible = RepositoryIndex::open(temp.path()).unwrap();
        assert_eq!(visible.hash().unwrap().as_deref(), Some(new_hash.as_str()));
        assert_eq!(
            visible.published_revision().unwrap().as_deref(),
            Some("new")
        );
        assert_eq!(
            visible
                .identity_report()
                .unwrap()
                .unwrap()
                .after
                .index_snapshot_hash,
            new_hash
        );
        let published_freshness = visible.freshness_checkpoint().unwrap();
        assert_eq!(
            published_freshness.published_revision.as_deref(),
            Some("new")
        );
        assert_eq!(
            published_freshness.index_snapshot_hash.as_deref(),
            Some(new_hash.as_str())
        );
        assert!(
            published_freshness.projection.last_sequence > old_freshness.projection.last_sequence
        );
    }

    #[test]
    fn failed_stage_preserves_published_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("A.kt"), "fun a() = 1\n").unwrap();
        let facts = |source: &str| {
            json!({"files":[{
                "path":"A.kt",
                "contentHash":canonical::hash_bytes(source.as_bytes()),
                "declarations":[{"symbolId":"a"}],
                "semanticFacts":[]
            }]})
        };
        let mut live = RepositoryIndex::open(temp.path()).unwrap();
        let old_hash = live.update(&facts("fun a() = 1\n")).unwrap();
        live.mark_published_revision("old").unwrap();
        drop(live);

        let error = RepositoryIndex::stage_update_unchecked_for_test(
            temp.path(),
            None,
            &facts("fun a() = 2\n"),
            temp.path(),
            "never-published",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProjectModelChanged);
        let visible = RepositoryIndex::open(temp.path()).unwrap();
        assert_eq!(visible.hash().unwrap().as_deref(), Some(old_hash.as_str()));
        assert_eq!(
            visible.published_revision().unwrap().as_deref(),
            Some("old")
        );
    }

    #[test]
    fn staged_publication_refuses_a_partial_repository_view() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun a() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let error = RepositoryIndex::stage_update_unchecked_for_test(
            temp.path(),
            None,
            &freshness_facts(source, true, "2.1.21"),
            temp.path(),
            "candidate",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProjectModelChanged);
        assert!(!temp.path().join(".semantic-thread/index.sqlite3").exists());
    }

    #[test]
    fn unchanged_files_are_not_rewritten() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("A.kt"), "fun a() = 1\n").unwrap();
        let file = |source: &str| {
            json!({
                "path":"A.kt","contentHash":canonical::hash_bytes(source.as_bytes()),
                "declarations":[{"symbolId":"a"}],"semanticFacts":[]
            })
        };
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        let facts = json!({"files":[file("fun a() = 1\n")]});
        index.update(&facts).unwrap();
        assert_eq!(
            index
                .connection
                .query_row(
                    "SELECT value FROM metadata WHERE key='last_changed_files'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "1"
        );
        index.update(&facts).unwrap();
        assert_eq!(
            index
                .connection
                .query_row(
                    "SELECT value FROM metadata WHERE key='last_changed_files'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "0"
        );
        std::fs::write(temp.path().join("A.kt"), "fun a() = 2\n").unwrap();
        index
            .update(&json!({"files":[file("fun a() = 2\n")]}))
            .unwrap();
        assert_eq!(
            index
                .connection
                .query_row(
                    "SELECT value FROM metadata WHERE key='last_changed_files'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "1"
        );

        let duplicate_symbols = json!({"files":[{
            "path":"A.kt",
            "contentHash":canonical::hash_bytes("fun a() = 2\n".as_bytes()),
            "declarations":[
                {"symbolId":"a.local","rangeStart":4},
                {"symbolId":"a.local","rangeStart":8}
            ],
            "semanticFacts":[]
        }]});
        index.update(&duplicate_symbols).unwrap();
        assert_eq!(
            index
                .connection
                .query_row(
                    "SELECT count(*) FROM declarations WHERE symbol_id='a.local'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn classifies_body_summary_signature_and_abi_invalidation() {
        let before = json!({"package":"p","imports":[],"declarations":[{
            "symbolId":"p.f()","bodyHash":"body:1","semanticSummaryHash":"summary:1",
            "sourceSignatureHash":"signature:1","abiHash":"abi:1"
        }]});
        let after = json!({"package":"p","imports":[],"declarations":[{
            "symbolId":"p.f()","bodyHash":"body:2","semanticSummaryHash":"summary:2",
            "sourceSignatureHash":"signature:2","abiHash":"abi:2"
        }]});
        let mut invalidations = BTreeSet::new();
        classify_file_change("A.kt", &before, &after, &mut invalidations);
        for expected in [
            "LOCAL_CFG:p.f()",
            "SSA:p.f()",
            "REFERENCES:p.f()",
            "CALLER_SUMMARIES:p.f()",
            "CALLSITES:p.f()",
            "OVERRIDES:p.f()",
            "IMPLEMENTATIONS:p.f()",
            "DOWNSTREAM_ABI:p.f()",
        ] {
            assert!(invalidations.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn publishes_identity_reports_and_fails_closed_on_decoys() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("A.kt");
        let declaration = |symbol: &str, name: &str| {
            json!({
                "declarationId":format!("declaration:{symbol}"),
                "symbolId":symbol,
                "name":name,
                "kind":"FUNCTION",
                "symbolIdentity":{
                    "module":":","sourceSet":"main","package":"p","declarationName":name,
                    "containingDeclarations":[],"declarationKind":"FUNCTION","typeParameterArity":0,
                    "receiverTypes":[],"contextReceiverTypes":[],
                    "parameterTypes":["String"],
                    "returnType":"String","suspendFlag":false,
                    "jvmDescriptor":"(Ljava/lang/String;)Ljava/lang/String;"
                },
                "sourceOrigin":{"file":"A.kt","rangeStart":0,"rangeEnd":20},
                "sourceSignatureHash":format!("signature:{name}"),
                "bodyHash":"body:stable",
                "abiHash":format!("abi:{symbol}"),
                "semanticSummaryHash":"summary:stable"
            })
        };
        let facts = |source: &str, declarations: Vec<Value>| {
            json!({
                "projectModelHash":"model",
                "classpathHash":"classpath",
                "compilerOptionsHash":"options",
                "files":[{
                    "path":"A.kt",
                    "contentHash":canonical::hash_bytes(source.as_bytes()),
                    "declarations":declarations,
                    "semanticFacts":[]
                }]
            })
        };
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        std::fs::write(&source_path, "fun old() = value\n").unwrap();
        index
            .update(&facts(
                "fun old() = value\n",
                vec![declaration("p.old", "old")],
            ))
            .unwrap();

        std::fs::write(&source_path, "fun renamed() = value\n").unwrap();
        index
            .update(&facts(
                "fun renamed() = value\n",
                vec![declaration("p.renamed", "renamed")],
            ))
            .unwrap();
        let renamed = index.identity_report().unwrap().unwrap();
        assert_eq!(renamed.decisions.len(), 1);
        assert_eq!(renamed.decisions[0].lifecycle, IdentityLifecycle::Renamed);
        assert!(
            index
                .invalidations()
                .unwrap()
                .contains(&"IDENTITY_RENAMED:p.old->p.renamed".to_owned())
        );

        let decoy_source = "fun first() = value\nfun second() = value\n";
        std::fs::write(&source_path, decoy_source).unwrap();
        index
            .update(&facts(
                decoy_source,
                vec![
                    declaration("p.first", "first"),
                    declaration("p.second", "second"),
                ],
            ))
            .unwrap();
        let ambiguous = index.identity_report().unwrap().unwrap();
        assert_eq!(ambiguous.decisions.len(), 1);
        assert_eq!(
            ambiguous.decisions[0].lifecycle,
            IdentityLifecycle::Ambiguous
        );
        assert_eq!(ambiguous.decisions[0].after.len(), 2);
        assert!(ambiguous.introduced.is_empty());
    }

    #[test]
    fn full_and_partial_updates_publish_defensible_freshness() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun a() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();

        index
            .update(&freshness_facts(source, false, "2.1.21"))
            .unwrap();
        assert_eq!(
            index.freshness_status(REPOSITORY_INDEX_FACT).unwrap(),
            FactFreshness::Fresh
        );
        assert_eq!(
            index.freshness_status(IDENTITY_VIEW_FACT).unwrap(),
            FactFreshness::Fresh
        );

        index
            .update(&freshness_facts(source, true, "2.1.21"))
            .unwrap();
        assert!(matches!(
            index.freshness_status(REPOSITORY_INDEX_FACT).unwrap(),
            FactFreshness::Stale { .. } | FactFreshness::PartiallyFresh { .. }
        ));
        assert!(matches!(
            index.freshness_status(IDENTITY_VIEW_FACT).unwrap(),
            FactFreshness::Stale { .. } | FactFreshness::PartiallyFresh { .. }
        ));

        index
            .update(&freshness_facts(source, false, "2.1.21"))
            .unwrap();
        assert_eq!(
            index.freshness_status(REPOSITORY_INDEX_FACT).unwrap(),
            FactFreshness::Fresh
        );
    }

    #[test]
    fn compiler_change_never_leaves_a_derived_view_fresh() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun a() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        index
            .update(&freshness_facts(source, false, "2.1.21"))
            .unwrap();
        let checkpoint = index.freshness_checkpoint().unwrap();
        let identity = checkpoint
            .projection
            .facts
            .iter()
            .find(|fact| fact.fact.id == IDENTITY_VIEW_FACT)
            .unwrap();
        let sequence = checkpoint.projection.last_sequence + 1;
        index
            .ingest_freshness_event(FreshnessEvent {
                schema: FRESHNESS_EVENT_SCHEMA.to_owned(),
                event_id: "task-context-observed".to_owned(),
                sequence,
                provenance: identity.fact.provenance.clone(),
                event: FreshnessEventKind::Observed {
                    fact: DependencyFact {
                        id: "view:task-context".to_owned(),
                        domain: FactDomain::Source,
                        fingerprint: "task-context-v1".to_owned(),
                        provenance: identity.fact.provenance.clone(),
                        depends_on: vec![IDENTITY_VIEW_FACT.to_owned()],
                    },
                },
            })
            .unwrap();
        assert_eq!(
            index.freshness_status("view:task-context").unwrap(),
            FactFreshness::Fresh
        );

        index
            .update(&freshness_facts(source, false, "2.3.0"))
            .unwrap();
        assert!(matches!(
            index.freshness_status("view:task-context").unwrap(),
            FactFreshness::Stale { .. } | FactFreshness::PartiallyFresh { .. }
        ));
    }

    #[test]
    fn durable_gap_blocks_reads_and_log_recovery_is_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun a() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        index
            .update(&freshness_facts(source, false, "2.1.21"))
            .unwrap();
        let before = index.freshness_checkpoint().unwrap();
        let repository = before
            .projection
            .facts
            .iter()
            .find(|fact| fact.fact.id == REPOSITORY_INDEX_FACT)
            .unwrap();
        let gap = FreshnessEvent {
            schema: FRESHNESS_EVENT_SCHEMA.to_owned(),
            event_id: "future-event".to_owned(),
            sequence: before.projection.last_sequence + 2,
            provenance: repository.fact.provenance.clone(),
            event: FreshnessEventKind::Invalidated {
                fact_id: REPOSITORY_INDEX_FACT.to_owned(),
                reason: "future-input".to_owned(),
            },
        };
        assert!(index.ingest_freshness_event(gap).is_err());
        assert!(matches!(
            index.freshness_status(REPOSITORY_INDEX_FACT).unwrap(),
            FactFreshness::Unknown { .. }
        ));
        drop(index);

        let mut recovered = RepositoryIndex::open(temp.path()).unwrap();
        assert!(matches!(
            recovered.freshness_status(REPOSITORY_INDEX_FACT).unwrap(),
            FactFreshness::Unknown { .. }
        ));
        assert!(recovered.recover_freshness_from_log().is_err());
        assert!(
            recovered
                .update(&freshness_facts(source, true, "2.1.21"))
                .is_err()
        );
        assert!(matches!(
            recovered.freshness_status(REPOSITORY_INDEX_FACT).unwrap(),
            FactFreshness::Unknown { .. }
        ));
        recovered
            .update(&freshness_facts(source, false, "2.1.21"))
            .unwrap();
        let rebuilt = recovered.freshness_checkpoint().unwrap();
        assert!(rebuilt.projection.last_sequence > before.projection.last_sequence);
        assert_eq!(
            recovered.freshness_status(REPOSITORY_INDEX_FACT).unwrap(),
            FactFreshness::Fresh
        );
    }

    #[test]
    fn durable_duplicate_is_a_noop_and_conflict_preserves_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun a() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        index
            .update(&freshness_facts(source, false, "2.1.21"))
            .unwrap();
        let checkpoint = index.freshness_checkpoint().unwrap();
        let identity = checkpoint
            .projection
            .facts
            .iter()
            .find(|fact| fact.fact.id == IDENTITY_VIEW_FACT)
            .unwrap();
        let event = FreshnessEvent {
            schema: FRESHNESS_EVENT_SCHEMA.to_owned(),
            event_id: "derived-view-observed".to_owned(),
            sequence: checkpoint.projection.last_sequence + 1,
            provenance: identity.fact.provenance.clone(),
            event: FreshnessEventKind::Observed {
                fact: DependencyFact {
                    id: "view:derived".to_owned(),
                    domain: FactDomain::Source,
                    fingerprint: "derived-v1".to_owned(),
                    provenance: identity.fact.provenance.clone(),
                    depends_on: vec![IDENTITY_VIEW_FACT.to_owned()],
                },
            },
        };
        assert!(matches!(
            index.ingest_freshness_event(event.clone()).unwrap(),
            IngestOutcome::Applied { .. }
        ));
        let applied = index.freshness_checkpoint().unwrap();
        assert!(matches!(
            index.ingest_freshness_event(event.clone()).unwrap(),
            IngestOutcome::Duplicate { .. }
        ));
        assert_eq!(index.freshness_checkpoint().unwrap(), applied);

        let mut conflicting = event;
        if let FreshnessEventKind::Observed { fact } = &mut conflicting.event {
            fact.fingerprint = "derived-v2".to_owned();
        }
        assert!(index.ingest_freshness_event(conflicting).is_err());
        assert_eq!(index.freshness_checkpoint().unwrap(), applied);
    }

    #[test]
    fn clean_rebuilds_have_identical_observable_freshness() {
        let left = tempfile::tempdir().unwrap();
        let right = tempfile::tempdir().unwrap();
        let source = "fun a() = 1\n";
        std::fs::write(left.path().join("A.kt"), source).unwrap();
        std::fs::write(right.path().join("A.kt"), source).unwrap();
        let facts = freshness_facts(source, false, "2.1.21");
        let mut left_index = RepositoryIndex::open(left.path()).unwrap();
        let mut right_index = RepositoryIndex::open(right.path()).unwrap();

        assert_eq!(
            left_index.update(&facts).unwrap(),
            right_index.update(&facts).unwrap()
        );
        assert_eq!(
            left_index.identity_report().unwrap(),
            right_index.identity_report().unwrap()
        );
        assert_eq!(
            left_index.invalidations().unwrap(),
            right_index.invalidations().unwrap()
        );
        assert_eq!(
            left_index.freshness_checkpoint().unwrap(),
            right_index.freshness_checkpoint().unwrap()
        );
    }

    #[test]
    fn sequence_consistent_checkpoint_corruption_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let source = "fun a() = 1\n";
        std::fs::write(temp.path().join("A.kt"), source).unwrap();
        let mut index = RepositoryIndex::open(temp.path()).unwrap();
        index
            .update(&freshness_facts(source, false, "2.1.21"))
            .unwrap();
        let json: String = index
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='freshness_checkpoint'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut checkpoint: Value = serde_json::from_str(&json).unwrap();
        checkpoint["facts"] = json!([]);
        index
            .connection
            .execute(
                "UPDATE metadata SET value=?1 WHERE key='freshness_checkpoint'",
                [canonical::pretty(&checkpoint).unwrap()],
            )
            .unwrap();

        assert!(index.freshness_status(REPOSITORY_INDEX_FACT).is_err());
        assert!(index.require_fresh(REPOSITORY_INDEX_FACT).is_err());
    }
}
