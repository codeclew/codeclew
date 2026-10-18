//! Addressed reconciliation of an already collected legacy recovery session.
//! Original histories stay immutable; a separate archive-bound receipt releases
//! only the exact verified roots. This is not a repair of missing CAS bytes.
use super::*;
use crate::state::{ManagedDirectory, ManagedEntryKind};
use std::collections::BTreeMap;
use std::ffi::OsStr;

const ARCHIVE_SCHEMA: &str = "codeclew-session-retention-archive/1.0";
const PLAN_SCHEMA: &str = "codeclew-session-retention-plan/1.0";
const RECEIPT_SCHEMA: &str = "codeclew-session-retention-reconciliation/1.0";
const MAX_FILES: usize = 4096;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Archive {
    schema: String,
    session_id: String,
    root_identities: BTreeMap<String, (u64, u64)>,
    /// Exact metadata bytes, not a checkout or a reconstructed Git repository.
    files: BTreeMap<String, Vec<u8>>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    schema: String,
    session_id: String,
    authority_digest: String,
    lifecycle_event_hash: String,
    run_heads: BTreeMap<String, String>,
    archive_digest: String,
    files: usize,
    bytes: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema: String,
    plan: Plan,
    plan_digest: String,
    applied_unix_ms: u128,
}
fn paths(state: &StateAuthority, session_id: &str) -> Result<(PathBuf, PathBuf), ClewError> {
    let base = state
        .root()
        .join("gc/retention-reconciliations")
        .join(id_component(session_id, "session:")?);
    Ok((base.join("archive.json"), base.join("receipt.json")))
}
fn require_terminal(
    lifecycle: &SessionLifecycle,
    loaded: &SessionRunProjections,
) -> Result<(), ClewError> {
    if lifecycle.status != SessionStatus::GarbageCollected
        || loaded.legacy_recovery_cancellations == 0
    {
        return Err(invalid(
            "retention reconciliation requires an already collected legacy recovery session",
        ));
    }
    if loaded.records.iter().any(|run| {
        !matches!(
            run.status,
            RunStatus::Cancelled
                | RunStatus::Failed
                | RunStatus::Published
                | RunStatus::PublishedConditional
        ) || run.process_id.is_some()
            || run.process_start_token.is_some()
    }) {
        return Err(invalid(
            "retention reconciliation refuses active or process-owned runs",
        ));
    }
    Ok(())
}
fn collect_files(
    directory: &ManagedDirectory,
    relative: &str,
    depth: usize,
    archive: &mut Archive,
    total: &mut usize,
) -> Result<(), ClewError> {
    if depth > 32 {
        return Err(invalid("retention archive directory depth exceeds 32"));
    }
    for name in directory.entries()? {
        let text = name
            .to_str()
            .ok_or_else(|| invalid("retention archive name is not UTF-8"))?;
        let path = format!("{relative}/{text}");
        if text == ".git" {
            return Err(invalid("retention archive refuses Git repository contents"));
        }
        match directory.entry_kind(&name)? {
            ManagedEntryKind::Directory => collect_files(
                &directory.existing_child(&name)?,
                &path,
                depth + 1,
                archive,
                total,
            )?,
            ManagedEntryKind::File => {
                if matches!(text, "lifecycle.lock" | "ledger.lock") {
                    continue;
                }
                if archive.files.len() >= MAX_FILES {
                    return Err(invalid("retention archive exceeds 4096 files"));
                }
                let remaining = MAX_BYTES
                    .checked_sub(*total)
                    .ok_or_else(|| invalid("retention archive exceeds 16 MiB"))?;
                let bytes = directory.read_file(&name, remaining)?;
                *total = total
                    .checked_add(bytes.len())
                    .ok_or_else(|| invalid("retention archive byte count overflow"))?;
                archive.files.insert(path, bytes);
            }
        }
    }
    Ok(())
}
fn snapshot(
    state: &StateAuthority,
    root: &Path,
    authority: &SessionAuthority,
    lifecycle: &SessionLifecycle,
    loaded: &SessionRunProjections,
) -> Result<(Plan, Vec<u8>), ClewError> {
    require_terminal(lifecycle, loaded)?;
    let directory = state.directory_at(root)?;
    // Reconciliation never removes a worktree or an unpublished candidate.
    if directory
        .entries()?
        .iter()
        .any(|name| name == OsStr::new("source"))
    {
        return Err(invalid(
            "retention reconciliation refuses a remaining session source",
        ));
    }
    let candidates = directory.existing_child(OsStr::new("candidates"))?;
    for name in candidates.entries()? {
        let candidate = candidates.existing_child(&name)?;
        if !loaded
            .records
            .iter()
            .any(|r| id_component(&r.run_id, "run:").is_ok_and(|id| Some(id) == name.to_str()))
        {
            return Err(invalid(
                "retention reconciliation refuses an unregistered candidate",
            ));
        }
        for entry in candidate.entries()? {
            if !candidate_derived_state_name(&entry) || !candidate.file_exists(&entry)? {
                return Err(invalid(
                    "retention reconciliation refuses remaining worktree or unknown candidate state",
                ));
            }
        }
    }
    let component = id_component(&authority.session_id, "session:")?;
    let mut archive = Archive {
        schema: ARCHIVE_SCHEMA.into(),
        session_id: authority.session_id.clone(),
        root_identities: BTreeMap::new(),
        files: BTreeMap::new(),
    };
    let relative = format!("sessions/{component}");
    archive
        .root_identities
        .insert(relative.clone(), directory.identity()?);
    let mut bytes = 0;
    collect_files(&directory, &relative, 0, &mut archive, &mut bytes)?;
    let mut heads = BTreeMap::new();
    for run in &loaded.records {
        let relative = format!("runs/{}", id_component(&run.run_id, "run:")?);
        let directory = state.directory(Path::new(&relative))?;
        archive
            .root_identities
            .insert(relative.clone(), directory.identity()?);
        collect_files(&directory, &relative, 0, &mut archive, &mut bytes)?;
        heads.insert(run.run_id.clone(), run.ledger_head.clone());
    }
    let archive_bytes = canonical::bytes(&archive).map_err(internal)?;
    if archive_bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(invalid("serialized retention archive exceeds 128 MiB"));
    }
    let plan = Plan {
        schema: PLAN_SCHEMA.into(),
        session_id: authority.session_id.clone(),
        authority_digest: authority.authority_digest.clone(),
        lifecycle_event_hash: lifecycle.event_hash.clone(),
        run_heads: heads,
        archive_digest: canonical::hash_bytes(&archive_bytes),
        files: archive.files.len(),
        bytes,
    };
    Ok((plan, archive_bytes))
}

/// Called only while the owning session lifecycle and run ledger locks are held.
pub(super) fn release_authorized(
    state: &StateAuthority,
    root: &Path,
    authority: &SessionAuthority,
    lifecycle: &SessionLifecycle,
    loaded: &SessionRunProjections,
) -> Result<bool, ClewError> {
    let (archive_path, receipt_path) = paths(state, &authority.session_id)?;
    let gc = state.directory(Path::new("gc"))?;
    if !gc
        .entries()?
        .iter()
        .any(|name| name == OsStr::new("retention-reconciliations"))
    {
        return Ok(false);
    }
    let reconciliations = gc.existing_child(OsStr::new("retention-reconciliations"))?;
    let component = id_component(&authority.session_id, "session:")?;
    if !reconciliations
        .entries()?
        .iter()
        .any(|name| name == OsStr::new(component))
    {
        return Ok(false);
    }
    if !reconciliations
        .existing_child(OsStr::new(component))?
        .file_exists(OsStr::new("receipt.json"))?
    {
        return Ok(false);
    }
    let receipt: Receipt = read_managed_json(state, &receipt_path, MAX_PLAN_BYTES)?;
    let (current, bytes) = snapshot(state, root, authority, lifecycle, loaded)?;
    if receipt.schema != RECEIPT_SCHEMA
        || receipt.plan.schema != PLAN_SCHEMA
        || receipt.plan_digest != canonical::hash(&receipt.plan).map_err(internal)?
        || canonical::bytes(&receipt.plan).map_err(internal)?
            != canonical::bytes(&current).map_err(internal)?
    {
        return Err(invalid(
            "retention reconciliation receipt differs from current terminal authority",
        ));
    }
    let retained = state.read_private_file(&archive_path, MAX_ARCHIVE_BYTES)?;
    if retained != bytes || canonical::hash_bytes(&retained) != receipt.plan.archive_digest {
        return Err(invalid(
            "retention reconciliation archive binding is invalid",
        ));
    }
    Ok(true)
}

pub fn reconcile_retention(
    session_id: &str,
    expected_plan: Option<&str>,
    apply: bool,
) -> Result<Value, ClewError> {
    reconcile_with_state(
        &StateAuthority::process_default()?,
        session_id,
        expected_plan,
        apply,
    )
}
fn reconcile_with_state(
    state: &StateAuthority,
    session_id: &str,
    expected_plan: Option<&str>,
    apply: bool,
) -> Result<Value, ClewError> {
    let component = id_component(session_id, "session:")?;
    let root = state.root().join("sessions").join(component);
    let _lock = SessionLifecycleLock::acquire(state, &root)?;
    let authority =
        read_session_authority_for_retention(state, &root.join("authority.json"), session_id)?;
    let lifecycle = verify_session_lifecycle_unlocked(state, &root, &authority)?;
    let loaded = load_session_runs_in_mode(state, &root, &authority, RunReadMode::Retention)?;
    let (plan, bytes) = snapshot(state, &root, &authority, &lifecycle, &loaded)?;
    let digest = canonical::hash(&plan).map_err(internal)?;
    let plan_value = canonical::value(&plan).map_err(internal)?;
    if expected_plan.is_some_and(|expected| expected != digest)
        || (apply && expected_plan.is_none())
    {
        return Err(invalid(
            "retention reconciliation requires the unchanged dry-run plan digest",
        ));
    }
    let (archive_path, receipt_path) = paths(state, session_id)?;
    let already_applied = release_authorized(state, &root, &authority, &lifecycle, &loaded)?;
    if apply && !already_applied {
        if state.private_file_exists(&archive_path)? {
            if state.read_private_file(&archive_path, MAX_ARCHIVE_BYTES)? != bytes {
                return Err(invalid(
                    "existing retention archive differs from dry-run authority",
                ));
            }
        } else {
            state.write_private_atomic(&archive_path, &bytes)?;
        }
        let receipt = Receipt {
            schema: RECEIPT_SCHEMA.into(),
            plan,
            plan_digest: digest.clone(),
            applied_unix_ms: unix_ms(),
        };
        // Atomic publication after the exact archive is durable; no history edits.
        state.write_private_atomic(
            &receipt_path,
            &canonical::bytes(&receipt).map_err(internal)?,
        )?;
        if !release_authorized(state, &root, &authority, &lifecycle, &loaded)? {
            return Err(internal("retention receipt was not published"));
        }
    }
    Ok(
        json!({"schema":"codeclew-session-retention-result/1.0","sessionId":session_id,"action":if apply{"APPLIED"}else{"DRY_RUN"},"planDigest":digest,"plan":plan_value,"archive":archive_path,"receipt":receipt_path,"alreadyApplied":already_applied,"scope":"Releases only exact terminal session/run CAS roots. Does not delete metadata, worktrees or CAS objects; run storage gc dry-run before applying collection."}),
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::retained_recovery_fixture;
    use super::*;
    fn plan(state: &StateAuthority, run: &RunRecord) -> Value {
        reconcile_with_state(state, &run.session_id, None, false).unwrap()
    }
    #[test]
    fn addressed_reconciliation_archives_exact_history_and_gc_keeps_unrelated_live_roots() {
        let (_temp, state, root, run_root, run) = retained_recovery_fixture(true, true);
        let (live, dead, missing) = {
            let store = CasStore::open(&state).unwrap();
            (
                store.put("test/live", b"current work").unwrap(),
                store.put("test/dead", b"old work").unwrap(),
                CasObject::for_bytes("test/missing", b"lost").unwrap(),
            )
        };
        state
            .write_private_atomic(
                &root.join("evidence.json"),
                &canonical::bytes(&json!({"absent":missing,"dead":dead})).unwrap(),
            )
            .unwrap();
        state
            .write_private_atomic(
                &state.root().join("threads/current.json"),
                &canonical::bytes(&json!({"live":live})).unwrap(),
            )
            .unwrap();
        let metadata = [
            root.join("authority.json"),
            root.join("lifecycle.jsonl"),
            root.join("lifecycle.json"),
            run_root.join("ledger.jsonl"),
            run_root.join("record.json"),
        ];
        let before = metadata
            .iter()
            .map(fs::read)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            crate::cas::storage_status(&state)
                .unwrap()
                .collection_blocked
        );
        let dry = plan(&state, &run);
        let (archive_path, receipt_path) = paths(&state, &run.session_id).unwrap();
        assert!(!archive_path.exists());
        assert!(!receipt_path.exists());
        assert!(reconcile_with_state(&state, &run.session_id, None, true).is_err());
        let digest = dry["planDigest"].as_str().unwrap();
        reconcile_with_state(&state, &run.session_id, Some(digest), true).unwrap();
        let receipt_before = fs::read(&receipt_path).unwrap();
        assert!(reconcile_with_state(&state,&run.session_id,Some(digest),true).unwrap()["alreadyApplied"].as_bool().unwrap());
        assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
        let archive: Archive = read_managed_json(&state, &archive_path, MAX_ARCHIVE_BYTES).unwrap();
        for (path, bytes) in &archive.files {
            assert_eq!(&fs::read(state.root().join(path)).unwrap(), bytes);
        }
        let report = crate::cas::storage_status(&state).unwrap();
        assert!(!report.collection_blocked);
        assert_eq!(report.released_sessions, 1);
        assert_eq!(report.released_runs, 1);
        crate::cas::garbage_collect_storage(&state).unwrap();
        let store = CasStore::open(&state).unwrap();
        assert!(store.read(&live, 4096).is_ok());
        assert!(store.read(&dead, 4096).is_err());
        assert_eq!(
            metadata
                .iter()
                .map(fs::read)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            before
        );
        assert!(receipt_path.exists());
        assert!(archive_path.exists());
    }
    #[test]
    fn reconciliation_refuses_process_identity_and_candidate_state() {
        for kind in ["pid", "token", "candidate-worktree", "unknown-candidate"] {
            let (_temp, state, root, run_root, run) = retained_recovery_fixture(true, true);
            if matches!(kind, "pid" | "token") {
                let authority = read_session_authority_for_retention(
                    &state,
                    &root.join("authority.json"),
                    &run.session_id,
                )
                .unwrap();
                let lifecycle =
                    verify_session_lifecycle_unlocked(&state, &root, &authority).unwrap();
                let mut loaded =
                    load_session_runs_in_mode(&state, &root, &authority, RunReadMode::Retention)
                        .unwrap();
                if kind == "pid" {
                    loaded.records[0].process_id = Some(std::process::id());
                } else {
                    loaded.records[0].process_start_token = Some("retained-token".into());
                }
                assert!(snapshot(&state, &root, &authority, &lifecycle, &loaded).is_err());
            } else {
                let id = if kind == "unknown-candidate" {
                    "unknown"
                } else {
                    id_component(&run.run_id, "run:").unwrap()
                };
                create_private_directory(&root.join("candidates").join(id).join("worktree"))
                    .unwrap();
                assert!(reconcile_with_state(&state, &run.session_id, None, false).is_err());
            }
            assert!(!paths(&state, &run.session_id).unwrap().1.exists());
            assert!(run_root.join("ledger.jsonl").exists());
        }
    }

    #[test]
    fn reconciliation_refuses_noncollected_recovery_and_remaining_work() {
        for (legacy, collected, worktree) in [
            (true, false, false),
            (false, true, false),
            (true, true, true),
        ] {
            let (_temp, state, root, _run_root, run) = retained_recovery_fixture(legacy, collected);
            if worktree {
                create_private_directory(&root.join("source")).unwrap();
            }
            assert!(reconcile_with_state(&state, &run.session_id, None, false).is_err());
            assert!(!paths(&state, &run.session_id).unwrap().1.exists());
        }
    }
    #[test]
    fn reconciliation_rejects_changed_metadata_identity_archive_and_hashes() {
        for case in ["stale-plan", "archive", "receipt", "ledger", "identity"] {
            let (_temp, state, root, run_root, run) = retained_recovery_fixture(true, true);
            let dry = plan(&state, &run);
            let digest = dry["planDigest"].as_str().unwrap();
            if case == "stale-plan" {
                state
                    .write_private_atomic(&root.join("new.json"), b"{}")
                    .unwrap();
                assert!(reconcile_with_state(&state, &run.session_id, Some(digest), true).is_err());
                continue;
            }
            reconcile_with_state(&state, &run.session_id, Some(digest), true).unwrap();
            let (archive, receipt) = paths(&state, &run.session_id).unwrap();
            match case {
                "archive" => state.write_private_atomic(&archive, b"{}").unwrap(),
                "receipt" => state.write_private_atomic(&receipt, b"{}").unwrap(),
                "ledger" => state
                    .write_private_atomic(&run_root.join("ledger.jsonl"), b"{}\n")
                    .unwrap(),
                "identity" => {
                    let moved = root.with_extension("old");
                    fs::rename(&root, &moved).unwrap();
                    create_private_directory(&root).unwrap();
                    for e in walkdir::WalkDir::new(&moved).min_depth(1) {
                        let e = e.unwrap();
                        let to = root.join(e.path().strip_prefix(&moved).unwrap());
                        if e.file_type().is_dir() {
                            create_private_directory(&to).unwrap()
                        } else {
                            fs::copy(e.path(), to).unwrap();
                        }
                    }
                }
                _ => unreachable!(),
            }
            assert!(released_session_cas_roots(&state).is_err(), "{case}");
            assert!(
                crate::cas::garbage_collect_storage(&state).is_err(),
                "{case}"
            );
        }
    }
    #[test]
    fn shared_reference_from_current_root_still_blocks_after_old_owner_reconciliation() {
        let (_temp, state, root, _run_root, run) = retained_recovery_fixture(true, true);
        let missing = CasObject::for_bytes("test/missing", b"lost").unwrap();
        let bytes = canonical::bytes(&json!({"ref":missing})).unwrap();
        state
            .write_private_atomic(&root.join("evidence.json"), &bytes)
            .unwrap();
        state
            .write_private_atomic(&state.root().join("threads/current.json"), &bytes)
            .unwrap();
        let dry = plan(&state, &run);
        reconcile_with_state(&state, &run.session_id, dry["planDigest"].as_str(), true).unwrap();
        let diagnostic = crate::cas::diagnose_storage(&state).unwrap();
        assert!(diagnostic.report.collection_blocked);
        assert_eq!(diagnostic.missing.len(), 1);
        assert_eq!(diagnostic.missing[0].owners.len(), 1);
        assert_eq!(
            diagnostic.missing[0].owners[0].root_file,
            "threads/current.json"
        );
        assert!(crate::cas::garbage_collect_storage(&state).is_err());
    }
}
