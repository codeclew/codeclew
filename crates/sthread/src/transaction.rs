use crate::canonical;
use crate::error::{ErrorCode, SthreadError};
use crate::graph;
use crate::index::{RepositoryIndex, StagedIndex};
use crate::model::*;
use crate::proto::RequestKind;
use crate::worker::{WorkerClient, workspace_root};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet};
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
    let mut expected_writes = Vec::new();
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
                &json!({"repo":repo,"symbol":owner,"compilation":thread.snapshot.compilation}),
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
            "compilation": thread.snapshot.compilation,
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
        let owner = target
            .get("ownerSymbolId")
            .and_then(Value::as_str)
            .unwrap_or(file);
        if operation.kind == "ADD_IMPORT" || operation.kind == "REMOVE_IMPORT" {
            let key = format!(
                "{file}:import:{}",
                operation
                    .replacement
                    .kotlin
                    .trim()
                    .trim_start_matches("import ")
            );
            expected_writes.push(ExpectedWriteFact {
                kind: "IMPORT".into(),
                key: key.clone(),
            });
            writes.push(WriteFact {
                kind: "IMPORT".into(),
                key,
                before_hash: canonical::hash_bytes(current.as_bytes()),
                after_hash: canonical::hash_bytes(candidate.as_bytes()),
            });
        } else {
            let anchor = target
                .get("anchorId")
                .and_then(Value::as_str)
                .unwrap_or(owner)
                .to_owned();
            expected_writes.extend([
                ExpectedWriteFact {
                    kind: "TARGET_ANCHOR".into(),
                    key: anchor.clone(),
                },
                ExpectedWriteFact {
                    kind: "BODY".into(),
                    key: owner.into(),
                },
                ExpectedWriteFact {
                    kind: "SUMMARY".into(),
                    key: owner.into(),
                },
            ]);
            if operation
                .postconditions
                .contains_key("allowedEffectChanges")
            {
                expected_writes.push(ExpectedWriteFact {
                    kind: "EFFECTS".into(),
                    key: owner.into(),
                });
            }
            writes.push(WriteFact {
                kind: "TARGET_ANCHOR".into(),
                key: anchor,
                before_hash: target
                    .get("exactTextHash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                after_hash: canonical::hash_bytes(operation.replacement.kotlin.as_bytes()),
            });
            if let Some(delta) = response.get("semanticDelta").and_then(Value::as_object) {
                for (field, kind) in [
                    ("body", "BODY"),
                    ("signature", "SIGNATURE"),
                    ("abi", "ABI"),
                    ("summary", "SUMMARY"),
                    ("effects", "EFFECTS"),
                ] {
                    let Some(change) = delta.get(field) else {
                        continue;
                    };
                    let before = change
                        .get("beforeHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let after = change
                        .get("afterHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if before == after {
                        continue;
                    }
                    if kind == "ABI" || kind == "SIGNATURE" {
                        return Err(SthreadError::new(
                            ErrorCode::AbiChanged,
                            format!("edit changes protected {kind} for {owner}"),
                        ));
                    }
                    writes.push(WriteFact {
                        kind: kind.into(),
                        key: change
                            .get("key")
                            .and_then(Value::as_str)
                            .unwrap_or(owner)
                            .into(),
                        before_hash: before.into(),
                        after_hash: after.into(),
                    });
                }
            }
        }
    }
    candidates.retain(|file, candidate| {
        std::fs::read_to_string(safe_join(repo, file).unwrap())
            .map(|original| original != *candidate)
            .unwrap_or(true)
    });
    writes.retain(|write| write.before_hash != write.after_hash);
    if !edit.expected_write_set.is_empty() {
        expected_writes = edit.expected_write_set.clone();
    }
    expected_writes.sort();
    expected_writes.dedup();
    let expected_scope: BTreeSet<_> = expected_writes
        .iter()
        .map(|fact| (&fact.kind, &fact.key))
        .collect();
    if let Some(exceeded) = writes
        .iter()
        .find(|fact| !expected_scope.contains(&(&fact.kind, &fact.key)))
    {
        return Err(SthreadError::new(
            ErrorCode::WritesetExceeded,
            format!(
                "actual write {}:{} is outside ExpectedWriteSet",
                exceeded.kind, exceeded.key
            ),
        ));
    }
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
        expected_write_set: expected_writes,
        diagnostics,
        formatting_windows: windows,
    })
}

pub fn commit(
    repo: &Path,
    transaction: &mut Transaction,
    target_ref: &str,
    worker: &mut WorkerClient,
) -> Result<Value, SthreadError> {
    let current = git_output(repo, &["rev-parse", target_ref])?;
    transaction.target_ref = Some(target_ref.into());
    let base_index_snapshot = transaction
        .base_index_snapshot
        .clone()
        .filter(|snapshot| !snapshot.is_empty())
        .or_else(|| {
            (!transaction.thread.snapshot.index_snapshot.is_empty())
                .then(|| transaction.thread.snapshot.index_snapshot.clone())
        })
        .ok_or_else(|| {
            SthreadError::new(
                ErrorCode::InvalidInput,
                "transaction must start with an immutable repository index snapshot",
            )
        })?;
    transaction.base_index_snapshot = Some(base_index_snapshot.clone());
    let current_index_snapshot =
        RepositoryIndex::open_compilation(repo, Some(&transaction.thread.snapshot.compilation))?
            .hash()?;
    if current == transaction.base_revision
        && current_index_snapshot.as_deref() != Some(base_index_snapshot.as_str())
    {
        return Err(SthreadError::new(
            ErrorCode::StaleRequiresReslice,
            format!(
                "repository index snapshot changed since transaction start: expected {base_index_snapshot}, current {}",
                current_index_snapshot.as_deref().unwrap_or("<missing>")
            ),
        ));
    }
    let edit_hash = canonical::hash(&transaction.edit).map_err(internal)?;
    if let Some(existing) =
        find_matching_transaction_commit(repo, target_ref, &transaction.tx_id, &edit_hash)?
    {
        let compilation = &transaction.thread.snapshot.compilation;
        let repository_index = RepositoryIndex::open_compilation(repo, Some(compilation))?;
        let (final_index_snapshot, invalidations) =
            if repository_index.published_revision()?.as_deref() == Some(current.as_str()) {
                (
                    repository_index.hash()?.ok_or_else(|| {
                        SthreadError::new(
                            ErrorCode::TransactionRecoveryRequired,
                            "published revision has no repository index hash",
                        )
                    })?,
                    repository_index.invalidations()?,
                )
            } else {
                // The transaction commit may now be an ancestor of the target
                // ref. The repository index always follows the current target,
                // never the older idempotent transaction commit.
                publish_index_for_revision(repo, &current, compilation, worker)?
            };
        transaction.final_commit = Some(existing.clone());
        transaction.status = "COMMITTED".into();
        let ledger_recorded = ledger(repo)
            .and_then(|ledger| {
                ledger.append(
                    transaction,
                    "idempotent retry matched reachable Git trailers",
                )
            })
            .is_ok();
        return Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":transaction.base_revision,"finalCommit":existing,"currentRevision":current,"finalIndexSnapshot":final_index_snapshot,"appliedInvalidations":invalidations,"targetRef":target_ref,"status":"COMMITTED","idempotent":true,"ledgerRecorded":ledger_recorded}),
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
    transaction.expected_write_set_hash =
        Some(canonical::hash(&report.expected_write_set).map_err(internal)?);
    transaction.actual_write_set_hash =
        Some(canonical::hash(&report.actual_write_set).map_err(internal)?);
    transaction.validation_evidence.push(json!({
        "kind":"SEMANTIC_PREVIEW",
        "valid":report.valid,
        "diagnosticsHash":canonical::hash(&report.diagnostics).map_err(internal)?,
        "expectedWriteSetHash":transaction.expected_write_set_hash,
        "actualWriteSetHash":transaction.actual_write_set_hash
    }));
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
        let configured_test_tasks = if transaction.test_tasks.is_empty() {
            &transaction.thread.snapshot.test_tasks
        } else {
            &transaction.test_tasks
        };
        let (compile_duration_ms, test_duration_ms) = validate_worktree(
            &worktree_path,
            &transaction.thread.snapshot.compile_task,
            configured_test_tasks,
        )?;
        transaction.validation_evidence.push(json!({
            "kind":"GRADLE",
            "compileTask":transaction.thread.snapshot.compile_task,
            "testTasks":configured_test_tasks,
            "compileDurationMs":compile_duration_ms,
            "testDurationMs":test_duration_ms,
            "status":"PASSED"
        }));
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
        let index_facts = worker.request(
            RequestKind::IndexFiles,
            &json!({
                "repo":worktree_path,
                "compilation":transaction.thread.snapshot.compilation
            }),
        )?;
        let staged_index = RepositoryIndex::stage_update(
            repo,
            Some(&transaction.thread.snapshot.compilation),
            &index_facts,
            &worktree_path,
            &candidate,
        )?;
        git(repo, &["update-ref", target_ref, &candidate, &current]).map_err(|_| {
            SthreadError::new(
                ErrorCode::RefCompareAndSwapFailed,
                "target ref changed during commit CAS",
            )
        })?;
        let (final_index_snapshot, invalidations) = match staged_index.publish() {
            Ok(published) => published,
            Err(publication_error) => {
                if git(repo, &["update-ref", target_ref, &current, &candidate]).is_ok() {
                    return Err(SthreadError::new(
                        ErrorCode::Internal,
                        format!(
                            "repository index publication failed; target ref was rolled back: {}",
                            publication_error.message
                        ),
                    ));
                }
                return Err(index_recovery_error(SthreadError::new(
                    ErrorCode::TransactionRecoveryRequired,
                    format!(
                        "index publication failed and target ref rollback also failed: {}",
                        publication_error.message
                    ),
                )));
            }
        };
        transaction.validation_evidence.push(json!({
            "kind":"INDEX_PUBLICATION",
            "baseIndexSnapshot":base_index_snapshot,
            "finalIndexSnapshot":final_index_snapshot,
            "appliedInvalidations":invalidations
        }));
        transaction.final_commit = Some(candidate.clone());
        transaction.status = "COMMITTED".into();
        // Ref + index publication is the commit point. A later ledger write
        // cannot turn that committed outcome into a reported failed
        // transaction; Git trailers let inspection reconstruct the event.
        let ledger_recorded = ledger(repo)
            .and_then(|ledger| {
                ledger.append(transaction, "target ref and index updated atomically")
            })
            .is_ok();
        Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":current,"baseIndexSnapshot":base_index_snapshot,"finalCommit":candidate,"finalIndexSnapshot":final_index_snapshot,"appliedInvalidations":invalidations,"targetRef":target_ref,"status":"COMMITTED","gradleValidationDurationMs":compile_duration_ms + test_duration_ms,"ledgerRecorded":ledger_recorded}),
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

fn publish_index_for_revision(
    repo: &Path,
    revision: &str,
    compilation: &str,
    worker: &mut WorkerClient,
) -> Result<(String, Vec<String>), SthreadError> {
    stage_index_for_revision(repo, revision, compilation, worker)
        .and_then(StagedIndex::publish)
        .map_err(index_recovery_error)
}

fn stage_index_for_revision(
    repo: &Path,
    revision: &str,
    compilation: &str,
    worker: &mut WorkerClient,
) -> Result<StagedIndex, SthreadError> {
    let temporary = tempfile::tempdir().map_err(io_error)?;
    let path = temporary.path().join("index-recovery");
    git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            path.to_str().unwrap(),
            revision,
        ],
    )?;
    let result = (|| {
        let facts = worker.request(
            RequestKind::IndexFiles,
            &json!({"repo":path,"compilation":compilation}),
        )?;
        RepositoryIndex::stage_update(repo, Some(compilation), &facts, &path, revision)
    })()
    .map_err(index_recovery_error);
    let _ = git(
        repo,
        &["worktree", "remove", "--force", path.to_str().unwrap()],
    );
    result
}

fn index_recovery_error(mut error: SthreadError) -> SthreadError {
    error.code = ErrorCode::TransactionRecoveryRequired;
    error.retryable = true;
    error.message = format!(
        "repository index publication requires recovery: {}",
        error.message
    );
    error
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
        let current_model = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":replay_path,"compilation":transaction.thread.snapshot.compilation}),
        )?;
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
        &json!({"repo":repo,"symbol":symbol,"compilation":transaction.thread.snapshot.compilation}),
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
        index_snapshot: transaction.thread.snapshot.index_snapshot.clone(),
        compilation: transaction.thread.snapshot.compilation.clone(),
        compile_task: transaction.thread.snapshot.compile_task.clone(),
        test_tasks: transaction.thread.snapshot.test_tasks.clone(),
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

fn validate_worktree(
    worktree: &Path,
    compile_task: &str,
    tests: &[String],
) -> Result<(u64, u64), SthreadError> {
    let compile_started = std::time::Instant::now();
    let output = Command::new(worktree.join("gradlew"))
        .arg(compile_task)
        .arg("--no-daemon")
        .arg("--quiet")
        .current_dir(worktree)
        .output()
        .map_err(io_error)?;
    let compile_duration_ms = compile_started.elapsed().as_millis() as u64;
    log_output(&output);
    if !output.status.success() {
        let mut error = SthreadError::new(
            ErrorCode::CompileFailed,
            format!("candidate worktree Gradle compile task {compile_task} failed"),
        );
        error
            .evidence
            .push(format!("gradleCompileDurationMs={compile_duration_ms}"));
        return Err(error);
    }
    if tests.is_empty() {
        return Ok((compile_duration_ms, 0));
    }
    let test_started = std::time::Instant::now();
    let output = Command::new(worktree.join("gradlew"))
        .args(tests)
        .arg("--no-daemon")
        .arg("--quiet")
        .current_dir(worktree)
        .output()
        .map_err(io_error)?;
    let test_duration_ms = test_started.elapsed().as_millis() as u64;
    log_output(&output);
    if output.status.success() {
        Ok((compile_duration_ms, test_duration_ms))
    } else {
        let mut error = SthreadError::new(
            ErrorCode::TestFailed,
            format!(
                "candidate worktree Gradle test tasks {} failed",
                tests.join(", ")
            ),
        );
        error
            .evidence
            .push(format!("gradleCompileDurationMs={compile_duration_ms}"));
        error
            .evidence
            .push(format!("gradleTestDurationMs={test_duration_ms}"));
        Err(error)
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
            let target_revision = transaction
                .target_ref
                .as_deref()
                .map(|target_ref| git_output(&self.repo, &["rev-parse", target_ref]))
                .transpose()?;
            let reachable_commit = target_revision
                .as_deref()
                .map(|revision| find_transaction_commit(&self.repo, revision, id))
                .transpose()?
                .flatten();
            if let Some(commit) = reachable_commit {
                let compilation = &transaction.thread.snapshot.compilation;
                let index = RepositoryIndex::open_compilation(&self.repo, Some(compilation))?;
                let current_target = target_revision
                    .as_deref()
                    .expect("reachable commit has target");
                if index.published_revision()?.as_deref() != Some(current_target) {
                    drop(index);
                    let mut worker = WorkerClient::start(&workspace_root())?;
                    let publication = publish_index_for_revision(
                        &self.repo,
                        current_target,
                        compilation,
                        &mut worker,
                    );
                    let _ = worker.shutdown();
                    let (index_snapshot, invalidations) = publication?;
                    transaction.validation_evidence.push(json!({
                        "kind":"INDEX_RECOVERY",
                        "finalIndexSnapshot":index_snapshot,
                        "appliedInvalidations":invalidations
                    }));
                }
                transaction.status = "COMMITTED".into();
                transaction.final_commit = Some(commit);
                self.append(
                    &transaction,
                    "recovered committed status from reachable Git trailer",
                )?;
                status = "COMMITTED".into();
                action = "RECOVERED_COMMITTED_FROM_TRAILER";
            } else if status == "COMMITTING" {
                match recover_candidate_commit(&self.repo, &transaction)? {
                    CandidateRecovery::Published(commit) => {
                        transaction.status = "COMMITTED".into();
                        transaction.final_commit = Some(commit);
                        self.append(
                            &transaction,
                            "recovered candidate commit with compare-and-swap",
                        )?;
                        status = "COMMITTED".into();
                        action = "RECOVERED_COMMITTED_CANDIDATE_CAS";
                    }
                    CandidateRecovery::Conflicted(reason) => {
                        transaction.status = "CONFLICTED".into();
                        self.append(&transaction, &reason)?;
                        status = "CONFLICTED".into();
                        action = "RECOVERED_CONFLICTED_REF_MOVED";
                    }
                    CandidateRecovery::Aborted(reason) => {
                        transaction.status = "ABORTED".into();
                        self.append(&transaction, &reason)?;
                        status = "ABORTED".into();
                        action = "RECOVERED_ABORTED_CANDIDATE_MISSING";
                    }
                }
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
                transaction.status = "ABORTED".into();
                self.append(
                    &transaction,
                    "recovered unknown unfinished state as aborted",
                )?;
                status = "ABORTED".into();
                action = "RECOVERED_ABORTED_UNKNOWN_STATE";
            }
        }
        let mut statement=self.connection.prepare("SELECT sequence,status,timestamp,evidence FROM events WHERE tx_id=?1 ORDER BY sequence").map_err(db_error)?;
        let rows=statement.query_map([id],|r|Ok(json!({"sequence":r.get::<_,i64>(0)?,"status":r.get::<_,String>(1)?,"timestamp":r.get::<_,String>(2)?,"evidence":r.get::<_,String>(3)?}))).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
        Ok(
            json!({"schema":"semantic-ledger/0.1","transactionId":id,"events":rows,"reconciledStatus":status,"recoveryAction":action,"recoverable":true}),
        )
    }
}

enum CandidateRecovery {
    Published(String),
    Conflicted(String),
    Aborted(String),
}

fn recover_candidate_commit(
    repo: &Path,
    transaction: &Transaction,
) -> Result<CandidateRecovery, SthreadError> {
    let Some(candidate) = transaction.candidate_commit.as_deref() else {
        return Ok(CandidateRecovery::Aborted(
            "COMMITTING record has no candidate commit; no ref was changed".into(),
        ));
    };
    let Some(target_ref) = transaction.target_ref.as_deref() else {
        return Ok(CandidateRecovery::Aborted(
            "COMMITTING record has no target ref; candidate left unpublished".into(),
        ));
    };
    let message = match git_output(repo, &["show", "-s", "--format=%B", candidate]) {
        Ok(message) => message,
        Err(_) => {
            return Ok(CandidateRecovery::Aborted(
                "candidate commit object is unavailable; no ref was changed".into(),
            ));
        }
    };
    let edit_hash = canonical::hash(&transaction.edit).map_err(internal)?;
    if !message
        .lines()
        .any(|line| line.trim() == format!("Semantic-Transaction-Id: {}", transaction.tx_id))
        || !message
            .lines()
            .any(|line| line.trim() == format!("Semantic-Edit-Hash: {edit_hash}"))
    {
        return Ok(CandidateRecovery::Aborted(
            "candidate trailers do not match ledger transaction".into(),
        ));
    }
    let parent = git_output(repo, &["rev-parse", &format!("{candidate}^")])?;
    let current = git_output(repo, &["rev-parse", target_ref])?;
    if current != parent {
        return Ok(CandidateRecovery::Conflicted(format!(
            "target ref moved from candidate parent {parent} to {current} before recovery"
        )));
    }
    let compilation = &transaction.thread.snapshot.compilation;
    let mut worker = WorkerClient::start(&workspace_root())?;
    let staged = stage_index_for_revision(repo, candidate, compilation, &mut worker);
    let _ = worker.shutdown();
    let staged = staged?;
    git(repo, &["update-ref", target_ref, candidate, &current])?;
    if let Err(publication_error) = staged.publish() {
        if git(repo, &["update-ref", target_ref, &current, candidate]).is_ok() {
            return Err(index_recovery_error(SthreadError::new(
                ErrorCode::TransactionRecoveryRequired,
                format!(
                    "candidate index publication failed; target ref was rolled back: {}",
                    publication_error.message
                ),
            )));
        }
        return Err(index_recovery_error(SthreadError::new(
            ErrorCode::TransactionRecoveryRequired,
            format!(
                "candidate index publication failed and target ref rollback failed: {}",
                publication_error.message
            ),
        )));
    }
    Ok(CandidateRecovery::Published(candidate.into()))
}

fn find_transaction_commit(
    repo: &Path,
    revision: &str,
    id: &str,
) -> Result<Option<String>, SthreadError> {
    let output = Command::new("git")
        .args(["log", revision, "--format=%H%x1f%B%x1e"])
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
