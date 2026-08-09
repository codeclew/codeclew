use crate::canonical;
use crate::error::{ErrorCode, SthreadError};
use crate::freshness::{
    DependencyFact, FRESHNESS_EVENT_SCHEMA, FactDomain, FactFreshness, FactProvenance,
    FreshnessCheckpoint, FreshnessEvent, FreshnessEventKind, FreshnessProjection, IngestOutcome,
};
use crate::identity::{
    IdentityLifecycle, IdentityReport, SnapshotProvenance, decide_identity_delta,
};
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

    pub fn publish(mut self) -> Result<(String, Vec<String>), SthreadError> {
        std::fs::rename(&self.staging_path, &self.published_path).map_err(|error| {
            SthreadError::new(
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
    pub fn open(repo: &Path) -> Result<Self, SthreadError> {
        Self::open_compilation(repo, None)
    }

    pub fn open_compilation(repo: &Path, compilation: Option<&str>) -> Result<Self, SthreadError> {
        exclude_runtime_state(repo)?;
        let state = repo.join(".semantic-thread");
        std::fs::create_dir_all(&state).map_err(io_error)?;
        let blobs = state.join("blobs/sha256");
        std::fs::create_dir_all(&blobs).map_err(io_error)?;
        Self::open_database(repo, state.join(database_name(compilation)), blobs)
    }

    fn open_database(repo: &Path, database: PathBuf, blobs: PathBuf) -> Result<Self, SthreadError> {
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
        facts: &Value,
        source_root: &Path,
        revision: &str,
    ) -> Result<StagedIndex, SthreadError> {
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
                return Err(SthreadError::new(
                    ErrorCode::Internal,
                    "published index is busy and cannot be staged consistently",
                ));
            }
            drop(live);
            std::fs::copy(&published_path, &staging_path).map_err(io_error)?;
        }

        let result = (|| {
            let mut staged = Self::open_database(repo, staging_path.clone(), blobs.clone())?;
            let hash = staged.update_from_root(facts, source_root)?;
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

    pub fn update(&mut self, facts: &Value) -> Result<String, SthreadError> {
        let source_root = self.repo.clone();
        self.update_from_root(facts, &source_root)
    }

    pub fn update_from_root(
        &mut self,
        facts: &Value,
        source_root: &Path,
    ) -> Result<String, SthreadError> {
        let files = facts
            .get("files")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                SthreadError::new(ErrorCode::InvalidInput, "worker index has no files")
            })?;
        let tx = self.connection.transaction().map_err(db_error)?;
        let previous_metadata: BTreeMap<String, String> = [
            "project_model_hash",
            "classpath_hash",
            "compiler_options_hash",
            "index_hash",
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
                SthreadError::new(
                    ErrorCode::Internal,
                    format!(
                        "cannot read indexed source {}: {error}",
                        source_path.display()
                    ),
                )
            })?;
            if canonical::hash_bytes(&source) != hash {
                return Err(SthreadError::new(
                    ErrorCode::ProjectModelChanged,
                    format!("source changed while indexing: {path}"),
                ));
            }
            let blob = self.blobs.join(hash.trim_start_matches("sha256:"));
            if !blob.exists() {
                std::fs::write(&blob, &source).map_err(|error| {
                    SthreadError::new(
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

    pub fn hash(&self) -> Result<Option<String>, SthreadError> {
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

    pub fn invalidations(&self) -> Result<Vec<String>, SthreadError> {
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

    pub fn identity_report(&self) -> Result<Option<IdentityReport>, SthreadError> {
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

    pub fn freshness_checkpoint(&self) -> Result<RepositoryFreshnessCheckpoint, SthreadError> {
        Ok(RepositoryFreshnessCheckpoint {
            schema: "repository-freshness-checkpoint/0.1".to_owned(),
            projection: load_freshness_projection(&self.connection)?.checkpoint(),
            published_revision: self.published_revision()?,
            index_snapshot_hash: self.hash()?,
        })
    }

    pub fn freshness_status(&self, fact_id: &str) -> Result<FactFreshness, SthreadError> {
        Ok(load_freshness_projection(&self.connection)?.status(fact_id))
    }

    pub fn require_fresh(&self, fact_id: &str) -> Result<(), SthreadError> {
        let status = self.freshness_status(fact_id)?;
        if status == FactFreshness::Fresh {
            Ok(())
        } else {
            Err(SthreadError::new(
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
    ) -> Result<IngestOutcome, SthreadError> {
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
    pub fn recover_freshness_from_log(&mut self) -> Result<FreshnessCheckpoint, SthreadError> {
        let tx = self.connection.transaction().map_err(db_error)?;
        let current = load_freshness_projection(&tx)?;
        if current.checkpoint().sequence_gap.is_some() {
            return Err(SthreadError::new(
                ErrorCode::ProjectModelChanged,
                "freshness stream has a gap; a complete index rebuild is required",
            ));
        }
        let events = load_freshness_events(&tx)?;
        let projection =
            FreshnessProjection::replay(events).map_err(|error| internal(error.into()))?;
        let checkpoint = projection.checkpoint();
        if checkpoint != current.checkpoint() {
            return Err(SthreadError::new(
                ErrorCode::Internal,
                "freshness checkpoint differs from deterministic event replay",
            ));
        }
        tx.commit().map_err(db_error)?;
        Ok(checkpoint)
    }

    pub fn mark_published_revision(&self, revision: &str) -> Result<(), SthreadError> {
        self.connection
            .execute(
                "INSERT INTO metadata(key,value) VALUES('published_revision',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [revision],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn published_revision(&self) -> Result<Option<String>, SthreadError> {
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

fn exclude_runtime_state(repo: &Path) -> Result<(), SthreadError> {
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
    let separator = (!existing.is_empty() && !existing.ends_with('\n'))
        .then_some("\n")
        .unwrap_or("");
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
) -> Result<(), SthreadError> {
    let mut projection = load_freshness_projection(tx)?;
    let partial = facts.get("partial").and_then(Value::as_bool) == Some(true);
    if projection.checkpoint().sequence_gap.is_some() {
        if partial {
            return Err(SthreadError::new(
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
) -> Result<FactProvenance, SthreadError> {
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
) -> Result<(), SthreadError> {
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
) -> Result<(), SthreadError> {
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
) -> Result<(), SthreadError> {
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
) -> Result<(), SthreadError> {
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
) -> Result<(), SthreadError> {
    let json = canonical::pretty(checkpoint).map_err(internal)?;
    connection
        .execute(
            "INSERT INTO metadata(key,value) VALUES('freshness_checkpoint',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&json],
        )
        .map_err(db_error)?;
    Ok(())
}

fn load_freshness_projection(connection: &Connection) -> Result<FreshnessProjection, SthreadError> {
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
            return Err(SthreadError::new(
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
        return Err(SthreadError::new(
            ErrorCode::Internal,
            "freshness checkpoint differs from deterministic event replay",
        ));
    }
    Ok(projection)
}

fn load_freshness_events(connection: &Connection) -> Result<Vec<FreshnessEvent>, SthreadError> {
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
                return Err(SthreadError::new(
                    ErrorCode::Internal,
                    format!("freshness event {} sequence mismatch", event.event_id),
                ));
            }
            let actual_hash = canonical::hash(&event).map_err(internal)?;
            if actual_hash != expected_hash {
                return Err(SthreadError::new(
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

fn io_error(error: std::io::Error) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, error.to_string())
}
fn db_error(error: rusqlite::Error) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, error.to_string())
}
fn internal(error: anyhow::Error) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let stage = RepositoryIndex::stage_update(
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

        let error = RepositoryIndex::stage_update(
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
        let error = RepositoryIndex::stage_update(
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
