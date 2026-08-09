use crate::canonical;
use crate::error::{ErrorCode, SthreadError};
use crate::identity::{
    IdentityLifecycle, IdentityReport, SnapshotProvenance, decide_identity_delta,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct RepositoryIndex {
    connection: Connection,
    repo: PathBuf,
    blobs: PathBuf,
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
}
