use crate::canonical;
use crate::error::{ErrorCode, SthreadError};
use crate::graph;
use crate::model::*;
use crate::proto::RequestKind;
use crate::worker::WorkerClient;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use similar::TextDiff;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn preview(
    repo: &Path,
    thread: &ThreadIr,
    edit: &EditIr,
    worker: &mut WorkerClient,
) -> Result<PreviewReport, SthreadError> {
    let head = git_output(repo, &["rev-parse", "HEAD"])?;
    if head != edit.base_revision || head != thread.snapshot.base_revision {
        return Err(SthreadError::new(
            ErrorCode::ProjectModelChanged,
            format!(
                "snapshot base {} does not match HEAD {head}",
                edit.base_revision
            ),
        ));
    }
    if edit.thread_id != thread.thread_id {
        return Err(SthreadError::new(
            ErrorCode::PreconditionFailed,
            "Edit IR threadId does not match Thread IR",
        ));
    }
    if edit.operations.is_empty() {
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "Edit IR has no operations",
        ));
    }
    let mut candidates = BTreeMap::new();
    let mut writes = Vec::new();
    let mut windows = Vec::new();
    let mut diagnostics = Vec::new();
    for operation in &edit.operations {
        if operation.kind != "REPLACE_EXPRESSION"
            && operation.kind != "REPLACE_FUNCTION_BODY"
            && operation.kind != "ADD_IMPORT"
            && operation.kind != "REMOVE_IMPORT"
        {
            return Err(SthreadError::new(
                ErrorCode::InvalidInput,
                format!("unsupported edit operation {}", operation.kind),
            ));
        }
        let target = operation.target.as_object().ok_or_else(|| {
            SthreadError::new(
                ErrorCode::InvalidInput,
                "operation target must be an anchor object",
            )
        })?;
        let file = target
            .get("fileId")
            .and_then(Value::as_str)
            .ok_or_else(|| SthreadError::new(ErrorCode::InvalidInput, "target has no fileId"))?;
        if let Some(expected) = operation
            .preconditions
            .get("nodeTextHash")
            .and_then(Value::as_str)
            && target.get("exactTextHash").and_then(Value::as_str) != Some(expected)
        {
            return Err(SthreadError::new(
                ErrorCode::PreconditionFailed,
                "nodeTextHash precondition does not match the target anchor",
            ));
        }
        if let Some(expected) = operation
            .preconditions
            .get("ownerSignatureHash")
            .and_then(Value::as_str)
        {
            let owner = target
                .get("ownerSymbolId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let resolved = worker.request(
                RequestKind::ResolveSymbol,
                &json!({"repo":repo,"symbol":owner}),
            )?;
            if resolved
                .pointer("/declaration/signatureHash")
                .and_then(Value::as_str)
                != Some(expected)
            {
                return Err(SthreadError::new(
                    ErrorCode::PreconditionFailed,
                    "ownerSignatureHash precondition failed",
                ));
            }
        }
        let path = safe_join(repo, file)?;
        let original = std::fs::read_to_string(&path).map_err(io_error)?;
        let current = candidates
            .get(file)
            .cloned()
            .unwrap_or_else(|| original.clone());
        let request = json!({
            "repo": repo, "file": file, "source": current,
            "ownerSymbolId": target.get("ownerSymbolId").and_then(Value::as_str).unwrap_or_default(),
            "exactTextHash": target.get("exactTextHash").and_then(Value::as_str).unwrap_or_default(),
            "syntaxKind": target.get("syntaxKind").and_then(Value::as_str).unwrap_or_default(),
            "normalizedTokenHash": target.get("normalizedTokenHash").and_then(Value::as_str).unwrap_or_default(),
            "ancestorPathHash": target.get("ancestorPathHash").cloned().unwrap_or(Value::Null),
            "localOrdinal": target.get("localOrdinal").cloned().unwrap_or(Value::Null),
            "leftContextHash": target.get("leftContextHash").cloned().unwrap_or(Value::Null),
            "rightContextHash": target.get("rightContextHash").cloned().unwrap_or(Value::Null),
            "kind": operation.kind, "replacement": operation.replacement.kotlin,
            "preconditions": operation.preconditions, "postconditions": operation.postconditions
        });
        let response = worker.request(RequestKind::ApplyEdit, &request)?;
        diagnostics.extend(
            response
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        let candidate = response
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                SthreadError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "candidate response has no source",
                )
            })?
            .to_owned();
        let forbidden = operation
            .postconditions
            .get("mustNotIntroduceEffects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let introduced = response
            .get("introducedEffects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(effect) = introduced.iter().filter_map(Value::as_str).find(|effect| {
            forbidden
                .iter()
                .filter_map(Value::as_str)
                .any(|blocked| blocked == *effect)
        }) {
            return Err(SthreadError::new(
                ErrorCode::EffectChanged,
                format!("replacement introduces forbidden effect {effect}"),
            ));
        }
        let range = target.get("rangeHint").cloned().unwrap_or(json!([]));
        windows.push(json!({"file":file,"range":range}));
        candidates.insert(file.to_owned(), candidate.clone());
        writes.push(WriteFact {
            kind: operation.kind.clone(),
            key: target
                .get("anchorId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "import:{}",
                        operation
                            .replacement
                            .kotlin
                            .trim()
                            .trim_start_matches("import ")
                    )
                }),
            before_hash: canonical::hash_bytes(current.as_bytes()),
            after_hash: canonical::hash_bytes(candidate.as_bytes()),
        });
    }
    candidates.retain(|file, candidate| {
        std::fs::read_to_string(safe_join(repo, file).unwrap())
            .map(|original| original != *candidate)
            .unwrap_or(true)
    });
    writes.retain(|write| write.before_hash != write.after_hash);
    validate_in_copy(repo, &candidates, &[])?;
    let mut diff = String::new();
    for (file, candidate) in &candidates {
        let original = std::fs::read_to_string(safe_join(repo, file)?).map_err(io_error)?;
        diff.push_str(&unified_diff(file, &original, candidate));
    }
    writes.sort();
    windows.sort_by_key(|v| v.to_string());
    Ok(PreviewReport {
        schema: "semantic-preview/0.1".into(),
        transaction_id: format!("tx:{}", uuid::Uuid::new_v4()),
        base_revision: head,
        valid: true,
        changed_files: candidates.keys().cloned().collect(),
        diff,
        candidates,
        actual_write_set: writes,
        diagnostics,
        formatting_windows: windows,
    })
}

fn validate_in_copy(
    repo: &Path,
    candidates: &BTreeMap<String, String>,
    test_tasks: &[String],
) -> Result<(), SthreadError> {
    let temp = tempfile::tempdir().map_err(io_error)?;
    let copy = temp.path().join("candidate");
    copy_tree(repo, &copy)?;
    for (file, source) in candidates {
        std::fs::write(safe_join(&copy, file)?, source.as_bytes()).map_err(io_error)?;
    }
    let wrapper = copy.join("gradlew");
    if !wrapper.is_file() {
        return Err(SthreadError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "Gradle Wrapper ./gradlew is required for K2 validation",
        ));
    }
    let mut tasks = vec!["compileKotlin".to_owned()];
    tasks.extend(test_tasks.iter().cloned());
    let output = Command::new(&wrapper)
        .args(&tasks)
        .arg("--no-daemon")
        .arg("--quiet")
        .current_dir(&copy)
        .output()
        .map_err(io_error)?;
    log_output(&output);
    if !output.status.success() {
        return Err(SthreadError::new(
            if test_tasks.is_empty() {
                ErrorCode::CompileFailed
            } else {
                ErrorCode::TestFailed
            },
            format!("Gradle validation failed for tasks {}", tasks.join(", ")),
        ));
    }
    Ok(())
}

pub fn commit(
    repo: &Path,
    transaction: &mut Transaction,
    target_ref: &str,
    worker: &mut WorkerClient,
) -> Result<Value, SthreadError> {
    let current = git_output(repo, &["rev-parse", target_ref])?;
    let edit_hash = canonical::hash(&transaction.edit).map_err(internal)?;
    if let Some(existing) =
        find_matching_transaction_commit(repo, target_ref, &transaction.tx_id, &edit_hash)?
    {
        transaction.final_commit = Some(existing.clone());
        transaction.status = "COMMITTED".into();
        ledger(repo)?.append(
            transaction,
            "idempotent retry matched reachable Git trailers",
        )?;
        return Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":transaction.base_revision,"finalCommit":existing,"currentRevision":current,"targetRef":target_ref,"status":"COMMITTED","idempotent":true}),
        );
    }
    let report = preview_for_commit(repo, transaction, &current, worker).map_err(|mut e| {
        if current != transaction.base_revision
            && matches!(
                e.code,
                ErrorCode::StaleTarget
                    | ErrorCode::AmbiguousTarget
                    | ErrorCode::ProjectModelChanged
            )
        {
            e.code = ErrorCode::WwConflict;
        }
        e
    })?;
    transaction.preview = Some(report.clone());
    transaction.status = "VALIDATED".into();
    ledger(repo)?.append(transaction, "preview and Gradle validation passed")?;
    if report.candidates.is_empty() {
        transaction.final_commit = Some(current.clone());
        transaction.status = "COMMITTED".into();
        ledger(repo)?.append(transaction, "idempotent no-op merged at current ref")?;
        return Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":current,"finalCommit":current,"targetRef":target_ref,"status":"COMMITTED","idempotent":true}),
        );
    }
    let worktree = tempfile::tempdir().map_err(io_error)?;
    let worktree_path = worktree.path().join("worktree");
    git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            worktree_path.to_str().unwrap(),
            &current,
        ],
    )?;
    let result = (|| {
        for (file, source) in &report.candidates {
            std::fs::write(safe_join(&worktree_path, file)?, source.as_bytes())
                .map_err(io_error)?;
        }
        validate_worktree(&worktree_path, &transaction.test_tasks)?;
        git(&worktree_path, &["add", "--", "."])?;
        let message = format!(
            "semantic transaction {}\n\nSemantic-Transaction-Id: {}\nSemantic-Base-Revision: {}\nSemantic-Edit-Hash: {}",
            transaction.intent, transaction.tx_id, transaction.base_revision, edit_hash
        );
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Semantic Thread",
                "-c",
                "user.email=semantic-thread@localhost",
                "commit",
                "-m",
                &message,
            ])
            .current_dir(&worktree_path)
            .output()
            .map_err(io_error)?;
        log_output(&output);
        if !output.status.success() {
            return Err(SthreadError::new(
                ErrorCode::Internal,
                "candidate commit failed",
            ));
        }
        let candidate = git_output(&worktree_path, &["rev-parse", "HEAD"])?;
        transaction.candidate_commit = Some(candidate.clone());
        transaction.status = "COMMITTING".into();
        ledger(repo)?.append(transaction, "candidate commit created")?;
        git(repo, &["update-ref", target_ref, &candidate, &current]).map_err(|_| {
            SthreadError::new(
                ErrorCode::RefCompareAndSwapFailed,
                "target ref changed during commit CAS",
            )
        })?;
        transaction.final_commit = Some(candidate.clone());
        transaction.status = "COMMITTED".into();
        ledger(repo)?.append(transaction, "target ref updated atomically")?;
        Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":current,"finalCommit":candidate,"targetRef":target_ref,"status":"COMMITTED"}),
        )
    })();
    let _ = git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap(),
        ],
    );
    result
}

fn preview_for_commit(
    repo: &Path,
    transaction: &mut Transaction,
    current: &str,
    worker: &mut WorkerClient,
) -> Result<PreviewReport, SthreadError> {
    if current == transaction.base_revision {
        return preview(repo, &transaction.thread, &transaction.edit, worker);
    }
    transaction.status = "REBASING".into();
    ledger(repo)?.append(transaction, "target ref moved; semantic replay requested")?;
    let temp = tempfile::tempdir().map_err(io_error)?;
    let replay_path = temp.path().join("replay");
    git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            replay_path.to_str().unwrap(),
            current,
        ],
    )?;
    let result = (|| {
        let current_model =
            worker.request(RequestKind::OpenProject, &json!({"repo":replay_path}))?;
        if current_model
            .get("projectModelHash")
            .and_then(Value::as_str)
            != Some(transaction.project_model_hash.as_str())
        {
            return Err(SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                format!(
                    "project model changed since slice: expected {}, current {}",
                    transaction.project_model_hash,
                    current_model
                        .get("projectModelHash")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing>")
                ),
            ));
        }
        revalidate_semantic_read_set(&replay_path, transaction, &current_model, current, worker)?;
        let mut replay_thread = transaction.thread.clone();
        let mut replay_edit = transaction.edit.clone();
        replay_thread.snapshot.base_revision = current.to_owned();
        replay_edit.base_revision = current.to_owned();
        preview(&replay_path, &replay_thread, &replay_edit, worker)
    })();
    let _ = git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            replay_path.to_str().unwrap(),
        ],
    );
    if matches!(&result, Err(error) if error.code == ErrorCode::StaleRequiresReslice) {
        transaction.status = "STALE_REQUIRES_RESLICE".into();
        let _ = ledger(repo)?.append(transaction, "project model or read dependency changed");
    }
    result
}

fn revalidate_semantic_read_set(
    repo: &Path,
    transaction: &Transaction,
    project: &Value,
    current: &str,
    worker: &mut WorkerClient,
) -> Result<(), SthreadError> {
    let symbol = transaction
        .thread
        .seed
        .get("symbol")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                "thread seed has no owner symbol",
            )
        })?;
    let raw = worker.request(
        RequestKind::BuildLocalGraph,
        &json!({"repo":repo,"symbol":symbol}),
    )?;
    let graph = graph::enrich(serde_json::from_value::<LocalGraph>(raw).map_err(|error| {
        SthreadError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
    })?);
    let old_seed_id = transaction
        .thread
        .seed
        .get("nodeId")
        .and_then(Value::as_str);
    let seed_anchor = transaction
        .thread
        .seed
        .get("anchor")
        .and_then(|anchor| anchor.get("anchorId"))
        .and_then(Value::as_str);
    let seed_id = old_seed_id
        .filter(|id| graph.nodes.iter().any(|node| node.id == *id))
        .map(str::to_owned)
        .or_else(|| {
            graph
                .nodes
                .iter()
                .find(|node| {
                    node.origin
                        .as_ref()
                        .and_then(|origin| origin.get("anchorId"))
                        .and_then(Value::as_str)
                        == seed_anchor
                })
                .map(|node| node.id.clone())
        })
        .ok_or_else(|| {
            SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                "slice seed no longer resolves",
            )
        })?;
    let snapshot = Snapshot {
        base_revision: current.into(),
        project_model_hash: project
            .get("projectModelHash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        compiler_version: transaction.thread.snapshot.compiler_version.clone(),
    };
    let rebuilt = graph::slice(
        &graph,
        &seed_id,
        transaction.thread.policy.clone(),
        snapshot,
        transaction.thread.seed.clone(),
    )
    .map_err(internal)?;
    let old: std::collections::BTreeSet<_> = transaction
        .thread
        .read_set
        .iter()
        .filter(|fact| fact.kind != "PROJECT_MODEL")
        .cloned()
        .collect();
    let new: std::collections::BTreeSet<_> = rebuilt
        .read_set
        .iter()
        .filter(|fact| fact.kind != "PROJECT_MODEL")
        .cloned()
        .collect();
    if old != new {
        let removed: Vec<_> = old
            .difference(&new)
            .take(8)
            .map(|fact| format!("- {} {} {}", fact.kind, fact.key, fact.hash))
            .collect();
        let added: Vec<_> = new
            .difference(&old)
            .take(8)
            .map(|fact| format!("+ {} {} {}", fact.kind, fact.key, fact.hash))
            .collect();
        let target_anchors: std::collections::BTreeSet<_> = transaction
            .edit
            .operations
            .iter()
            .filter_map(|operation| operation.target.get("anchorId").and_then(Value::as_str))
            .collect();
        let target_text_changed = transaction.edit.operations.iter().any(|operation| {
            let Some(file) = operation.target.get("fileId").and_then(Value::as_str) else {
                return true;
            };
            let Some(text) = operation.target.get("sourceText").and_then(Value::as_str) else {
                return true;
            };
            std::fs::read_to_string(repo.join(file)).map_or(true, |source| !source.contains(text))
        });
        let write_conflict = target_text_changed
            || old.difference(&new).any(|fact| {
                fact.kind == "SOURCE_NODE" && target_anchors.contains(fact.key.as_str())
            });
        let mut error = SthreadError::new(
            if write_conflict {
                ErrorCode::WwConflict
            } else {
                ErrorCode::StaleRequiresReslice
            },
            if write_conflict {
                "concurrent write changed the target anchor"
            } else {
                "semantic ReadSet changed since slice"
            },
        );
        error.evidence = removed.into_iter().chain(added).collect();
        return Err(error);
    }
    Ok(())
}

fn validate_worktree(worktree: &Path, tests: &[String]) -> Result<(), SthreadError> {
    let mut tasks = vec!["compileKotlin".to_owned()];
    tasks.extend(tests.iter().cloned());
    let output = Command::new(worktree.join("gradlew"))
        .args(&tasks)
        .arg("--no-daemon")
        .arg("--quiet")
        .current_dir(worktree)
        .output()
        .map_err(io_error)?;
    log_output(&output);
    if output.status.success() {
        Ok(())
    } else {
        Err(SthreadError::new(
            ErrorCode::CompileFailed,
            "candidate worktree Gradle validation failed",
        ))
    }
}

pub struct Ledger {
    connection: Connection,
    repo: PathBuf,
}
impl Ledger {
    pub fn open(repo: &Path) -> Result<Self, SthreadError> {
        let dir = repo.join(".semantic-thread");
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        let connection = Connection::open(dir.join("ledger.sqlite3")).map_err(db_error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(db_error)?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS events(sequence INTEGER PRIMARY KEY AUTOINCREMENT, tx_id TEXT NOT NULL, status TEXT NOT NULL, timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, record_json BLOB NOT NULL, evidence TEXT NOT NULL);") .map_err(db_error)?;
        Ok(Self {
            connection,
            repo: repo.to_path_buf(),
        })
    }
    pub fn append(&self, tx: &Transaction, evidence: &str) -> Result<(), SthreadError> {
        self.connection
            .execute(
                "INSERT INTO events(tx_id,status,record_json,evidence) VALUES(?1,?2,?3,?4)",
                params![
                    tx.tx_id,
                    tx.status,
                    canonical::bytes(tx).map_err(internal)?,
                    evidence
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn inspect(&self, id: &str) -> Result<Value, SthreadError> {
        let latest: Option<(String, Vec<u8>)> = self.connection.query_row(
            "SELECT status,record_json FROM events WHERE tx_id=?1 ORDER BY sequence DESC LIMIT 1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(db_error)?;
        let (mut status, record) = latest.ok_or_else(|| {
            SthreadError::new(
                ErrorCode::InvalidInput,
                format!("transaction not found: {id}"),
            )
        })?;
        let mut transaction: Transaction = serde_json::from_slice(&record).map_err(|error| {
            SthreadError::new(ErrorCode::TransactionRecoveryRequired, error.to_string())
        })?;
        let terminal = matches!(
            status.as_str(),
            "COMMITTED" | "CONFLICTED" | "STALE_REQUIRES_RESLICE" | "VALIDATION_FAILED" | "ABORTED"
        );
        let mut action = "NONE";
        if !terminal {
            if let Some(commit) = find_transaction_commit(&self.repo, id)? {
                transaction.status = "COMMITTED".into();
                transaction.final_commit = Some(commit);
                self.append(
                    &transaction,
                    "recovered committed status from reachable Git trailer",
                )?;
                status = "COMMITTED".into();
                action = "RECOVERED_COMMITTED_FROM_TRAILER";
            } else if matches!(
                status.as_str(),
                "CREATED" | "SLICED" | "EDIT_PREVIEWED" | "VALIDATING" | "VALIDATED" | "REBASING"
            ) {
                transaction.status = "ABORTED".into();
                self.append(
                    &transaction,
                    "recovered unfinished pre-publication transaction as aborted",
                )?;
                status = "ABORTED".into();
                action = "RECOVERED_ABORTED_NO_REF_CHANGE";
            } else {
                return Err(SthreadError::new(
                    ErrorCode::TransactionRecoveryRequired,
                    "unfinished COMMITTING transaction has no reachable commit trailer",
                ));
            }
        }
        let mut statement=self.connection.prepare("SELECT sequence,status,timestamp,evidence FROM events WHERE tx_id=?1 ORDER BY sequence").map_err(db_error)?;
        let rows=statement.query_map([id],|r|Ok(json!({"sequence":r.get::<_,i64>(0)?,"status":r.get::<_,String>(1)?,"timestamp":r.get::<_,String>(2)?,"evidence":r.get::<_,String>(3)?}))).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
        Ok(
            json!({"schema":"semantic-ledger/0.1","transactionId":id,"events":rows,"reconciledStatus":status,"recoveryAction":action,"recoverable":true}),
        )
    }
}

fn find_transaction_commit(repo: &Path, id: &str) -> Result<Option<String>, SthreadError> {
    let output = Command::new("git")
        .args(["log", "--all", "--format=%H%x1f%B%x1e"])
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(SthreadError::new(
            ErrorCode::TransactionRecoveryRequired,
            "cannot scan Git history for transaction trailers",
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.split('\u{1e}').find_map(|record| {
        let (commit, message) = record.trim().split_once('\u{1f}')?;
        message
            .lines()
            .any(|line| line.trim() == format!("Semantic-Transaction-Id: {id}"))
            .then(|| commit.to_owned())
    }))
}

fn find_matching_transaction_commit(
    repo: &Path,
    revision: &str,
    id: &str,
    edit_hash: &str,
) -> Result<Option<String>, SthreadError> {
    let output = Command::new("git")
        .args(["log", revision, "--format=%H%x1f%B%x1e"])
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(SthreadError::new(
            ErrorCode::Internal,
            format!("cannot scan target history {revision} for transaction trailers"),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for record in text.split('\u{1e}') {
        let Some((commit, message)) = record.trim().split_once('\u{1f}') else {
            continue;
        };
        if !message
            .lines()
            .any(|line| line.trim() == format!("Semantic-Transaction-Id: {id}"))
        {
            continue;
        }
        let recorded_hash = message.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Semantic-Edit-Hash: ")
                .map(str::to_owned)
        });
        if recorded_hash.as_deref() != Some(edit_hash) {
            return Err(SthreadError::new(
                ErrorCode::InvalidInput,
                format!("transaction id {id} is already associated with a different edit"),
            ));
        }
        return Ok(Some(commit.to_owned()));
    }
    Ok(None)
}
pub fn ledger(repo: &Path) -> Result<Ledger, SthreadError> {
    Ledger::open(repo)
}

fn unified_diff(file: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{file}"), &format!("b/{file}"))
        .to_string()
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), SthreadError> {
    std::fs::create_dir_all(target).map_err(io_error)?;
    for entry in walkdir::WalkDir::new(source).into_iter().filter_entry(|e| {
        let n = e.file_name().to_string_lossy();
        n != ".git" && n != ".semantic-thread" && n != "build" && n != ".gradle"
    }) {
        let entry = entry.map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))?;
        let relative = entry.path().strip_prefix(source).unwrap();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let dest = target.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(io_error)?
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(io_error)?
            }
            std::fs::copy(entry.path(), &dest).map_err(io_error)?;
            let permissions = std::fs::metadata(entry.path())
                .map_err(io_error)?
                .permissions();
            std::fs::set_permissions(&dest, permissions).map_err(io_error)?;
        }
    }
    Ok(())
}
fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, SthreadError> {
    let path = root.join(relative);
    if relative.starts_with('/') || relative.split('/').any(|p| p == "..") {
        Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "path escapes repository",
        ))
    } else {
        Ok(path)
    }
}
fn git(repo: &Path, args: &[&str]) -> Result<(), SthreadError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    log_output(&output);
    if output.status.success() {
        Ok(())
    } else {
        Err(SthreadError::new(
            ErrorCode::Internal,
            format!("git {} failed", args.join(" ")),
        ))
    }
}
fn log_output(output: &std::process::Output) {
    if !output.stdout.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
}
fn git_output(repo: &Path, args: &[&str]) -> Result<String, SthreadError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(SthreadError::new(
            ErrorCode::InvalidInput,
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    }
}
fn io_error(e: std::io::Error) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, e.to_string())
}
fn db_error(e: rusqlite::Error) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, e.to_string())
}
fn internal(e: anyhow::Error) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, e.to_string())
}
