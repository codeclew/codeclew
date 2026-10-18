//! Address one derived repository incremental head, never its CAS descendants.
use super::*;
use crate::state::ManagedDirectory;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

const PLAN_SCHEMA: &str = "codeclew-incremental-retirement-plan/1.0";
const RECEIPT_SCHEMA: &str = "codeclew-incremental-retirement-receipt/1.0";
const MAX_HEAD_BYTES: usize = 64 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    schema: String,
    repository_key: String,
    head_key: String,
    repository_identity: (u64, u64),
    directory_identity: (u64, u64),
    file_identity: (u64, u64),
    file_change_time: (i64, i64),
    head_digest: String,
    head_bytes: usize,
    compiler_store_key: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema: String,
    plan: Plan,
    plan_digest: String,
    phase: String,
}
fn component(value: &str) -> Result<(), ClewError> {
    let prefixed = format!("sha256:{value}");
    digest_component(&prefixed).map(|_| ())
}
fn validate_head(bytes: &[u8]) -> Result<IncrementalHead, ClewError> {
    let head: IncrementalHead = serde_json::from_slice(bytes)
        .map_err(|_| corrupt("incremental retirement head is invalid"))?;
    if canonical::bytes(&head).map_err(internal)? != bytes
        || head.schema != INCREMENTAL_HEAD_SCHEMA
        || head.ready.object_schema != READY_GENERATION_SCHEMA
        || head.receipt.object_schema != INCREMENTAL_RECEIPT_SCHEMA
    {
        return Err(corrupt("incremental retirement head authority is invalid"));
    }
    digest_component(&head.compiler_store_key)?;
    crate::cas::validate_reference(&head.ready)?;
    crate::cas::validate_reference(&head.receipt)?;
    Ok(head)
}
fn snapshot(
    repository_key: &str,
    head_key: &str,
    repository: &ManagedDirectory,
    directory: &ManagedDirectory,
    name: &OsStr,
) -> Result<(Plan, Vec<u8>), ClewError> {
    repository.require_path_identity()?;
    directory.require_path_identity()?;
    let file = directory.open_file(name)?;
    let meta = file.metadata().map_err(io_error)?;
    if meta.len() > MAX_HEAD_BYTES as u64 {
        return Err(resource("incremental retirement head exceeds 64 KiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_HEAD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 != meta.len() {
        return Err(corrupt("incremental retirement head changed while reading"));
    }
    let head = validate_head(&bytes)?;
    Ok((
        Plan {
            schema: PLAN_SCHEMA.into(),
            repository_key: repository_key.into(),
            head_key: head_key.into(),
            repository_identity: repository.identity()?,
            directory_identity: directory.identity()?,
            file_identity: (meta.dev(), meta.ino()),
            file_change_time: (meta.ctime(), meta.ctime_nsec()),
            head_digest: canonical::hash_bytes(&bytes),
            head_bytes: bytes.len(),
            compiler_store_key: head.compiler_store_key,
        },
        bytes,
    ))
}
fn archive_paths(
    state: &StateAuthority,
    repository_key: &str,
    head_key: &str,
    digest: &str,
) -> Result<(PathBuf, PathBuf), ClewError> {
    let root = state
        .root()
        .join("gc/incremental-retirements")
        .join(repository_key)
        .join(head_key)
        .join(digest_component(digest)?);
    Ok((root.join("head.json"), root.join("receipt.json")))
}
fn verify_receipt(
    state: &StateAuthority,
    repository: &ManagedDirectory,
    directory: &ManagedDirectory,
    repository_key: &str,
    head_key: &str,
    expected: &str,
) -> Result<(Receipt, PathBuf, PathBuf), ClewError> {
    let (archive, path) = archive_paths(state, repository_key, head_key, expected)?;
    let bytes = state.read_private_file(&path, MAX_HEAD_BYTES)?;
    let receipt: Receipt = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("incremental retirement receipt is invalid"))?;
    if canonical::bytes(&receipt).map_err(internal)? != bytes
        || receipt.schema != RECEIPT_SCHEMA
        || receipt.plan.schema != PLAN_SCHEMA
        || receipt.plan.repository_key != repository_key
        || receipt.plan.head_key != head_key
        || receipt.plan_digest != expected
        || canonical::hash(&receipt.plan).map_err(internal)? != expected
        || !matches!(receipt.phase.as_str(), "PREPARED" | "APPLIED")
    {
        return Err(corrupt("incremental retirement receipt binding is invalid"));
    }
    repository.require_path_identity()?;
    directory.require_path_identity()?;
    if repository.identity()? != receipt.plan.repository_identity
        || directory.identity()? != receipt.plan.directory_identity
    {
        return Err(corrupt("incremental retirement directory identity changed"));
    }
    let archived = state.read_private_file(&archive, MAX_HEAD_BYTES)?;
    let head = validate_head(&archived)?;
    if canonical::hash_bytes(&archived) != receipt.plan.head_digest
        || archived.len() != receipt.plan.head_bytes
        || head.compiler_store_key != receipt.plan.compiler_store_key
    {
        return Err(corrupt("incremental retirement archive binding is invalid"));
    }
    Ok((receipt, archive, path))
}

pub fn retire(
    repository_key: &str,
    head_key: &str,
    expected_plan: Option<&str>,
    apply: bool,
) -> Result<Value, ClewError> {
    retire_with_state(
        &StateAuthority::process_default()?,
        repository_key,
        head_key,
        expected_plan,
        apply,
    )
}
fn retire_with_state(
    state: &StateAuthority,
    repository_key: &str,
    head_key: &str,
    expected_plan: Option<&str>,
    apply: bool,
) -> Result<Value, ClewError> {
    component(repository_key)?;
    component(head_key)?;
    if let Some(digest) = expected_plan {
        digest_component(digest)?;
    }
    if apply && expected_plan.is_none() {
        return Err(corrupt(
            "incremental retirement apply requires an exact dry-run plan",
        ));
    }
    // Every publisher takes a CasStore reference, hence holds a shared world
    // lease. Exclusive acquisition precedes all state reads and publication,
    // covers older writers too, and serializes with physical GC. Do not open a
    // second CasStore under this exclusive lease.
    let _world = crate::cas::storage_maintenance_lease(state)?;
    let repository = state
        .directory(Path::new("repos"))?
        .existing_child(OsStr::new(repository_key))?;
    let directory = repository.existing_child(OsStr::new("incremental"))?;
    let name = format!("{head_key}.json");
    let name = OsStr::new(&name);
    if !directory.file_exists(name)? {
        let expected =
            expected_plan.ok_or_else(|| corrupt("addressed incremental head is absent"))?;
        let (mut receipt, archive, path) = verify_receipt(
            state,
            &repository,
            &directory,
            repository_key,
            head_key,
            expected,
        )?;
        if apply && receipt.phase == "PREPARED" {
            receipt.phase = "APPLIED".into();
            write_canonical_atomic(state, &path, &receipt)?;
        }
        return Ok(
            json!({"schema":"codeclew-incremental-retirement-result/1.0","action":if apply{"APPLIED"}else{"DRY_RUN"},"planDigest":expected,"plan":receipt.plan,"alreadyAbsent":true,"archive":archive,"receipt":path,"casObjectsDeleted":0}),
        );
    }
    let (plan, bytes) = snapshot(repository_key, head_key, &repository, &directory, name)?;
    let digest = canonical::hash(&plan).map_err(internal)?;
    if expected_plan.is_some_and(|expected| expected != digest) {
        return Err(corrupt("incremental retirement dry-run plan is stale"));
    }
    let plan_value = canonical::value(&plan).map_err(internal)?;
    let (archive, receipt_path) = archive_paths(state, repository_key, head_key, &digest)?;
    if apply {
        if state.private_file_exists(&receipt_path)? {
            let (receipt, _, _) = verify_receipt(
                state,
                &repository,
                &directory,
                repository_key,
                head_key,
                &digest,
            )?;
            if receipt.phase == "APPLIED" {
                return Err(corrupt(
                    "incremental head reappeared after retirement; require a fresh plan",
                ));
            }
        } else {
            if state.private_file_exists(&archive)?
                && state.read_private_file(&archive, MAX_HEAD_BYTES)? != bytes
            {
                return Err(corrupt("incremental retirement archive changed"));
            }
            state.write_private_atomic(&archive, &bytes)?;
            write_canonical_atomic(
                state,
                &receipt_path,
                &Receipt {
                    schema: RECEIPT_SCHEMA.into(),
                    plan,
                    plan_digest: digest.clone(),
                    phase: "PREPARED".into(),
                },
            )?;
        }
        let (mut receipt, _, _) = verify_receipt(
            state,
            &repository,
            &directory,
            repository_key,
            head_key,
            &digest,
        )?;
        // Recheck exact bytes, file and parent identities immediately before
        // descriptor-relative unlink. Cooperative writers remain excluded.
        let (current, now) = snapshot(repository_key, head_key, &repository, &directory, name)?;
        if canonical::hash(&current).map_err(internal)? != digest || now != bytes {
            return Err(corrupt("incremental head changed before retirement"));
        }
        directory.remove_file(name)?;
        receipt.phase = "APPLIED".into();
        write_canonical_atomic(state, &receipt_path, &receipt)?;
    }
    Ok(
        json!({"schema":"codeclew-incremental-retirement-result/1.0","action":if apply{"APPLIED"}else{"DRY_RUN"},"planDigest":digest,"plan":plan_value,"archive":archive,"receipt":receipt_path,"alreadyAbsent":false,"casObjectsDeleted":0,"scope":"Retires one derived incremental head only. Other heads, history, sessions, source and CAS remain unchanged. Diagnose again and require an unblocked global storage GC dry-run before physical apply."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::mpsc, time::Duration};
    fn fixture() -> (
        tempfile::TempDir,
        StateAuthority,
        String,
        String,
        CasObject,
        CasObject,
        Vec<u8>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("v2")).unwrap();
        let repository = "a".repeat(64);
        let key = "b".repeat(64);
        let missing = CasObject::for_bytes("test/missing", b"lost").unwrap();
        let (ready, receipt, live) = {
            let store = CasStore::open(&state).unwrap();
            (
                store
                    .put(
                        READY_GENERATION_SCHEMA,
                        &canonical::bytes(&json!({"missing":missing})).unwrap(),
                    )
                    .unwrap(),
                store.put(INCREMENTAL_RECEIPT_SCHEMA, b"{}").unwrap(),
                store.put("test/live", b"current").unwrap(),
            )
        };
        let head = IncrementalHead {
            schema: INCREMENTAL_HEAD_SCHEMA.into(),
            compiler_store_key: format!("sha256:{}", "c".repeat(64)),
            ready: ready.clone(),
            receipt,
        };
        let bytes = canonical::bytes(&head).unwrap();
        state
            .write_private_atomic(
                &state
                    .root()
                    .join(format!("repos/{repository}/incremental/{key}.json")),
                &bytes,
            )
            .unwrap();
        state
            .write_private_atomic(
                &state.root().join("threads/current.json"),
                &canonical::bytes(&json!({"live":live})).unwrap(),
            )
            .unwrap();
        state
            .write_private_atomic(
                &state.root().join(format!(
                    "repos/{repository}/incremental/{}.json",
                    "d".repeat(64)
                )),
                &canonical::bytes(&json!({"live":live})).unwrap(),
            )
            .unwrap();
        (temp, state, repository, key, ready, live, bytes)
    }
    #[test]
    fn retirement_is_addressed_archived_idempotent_and_gc_keeps_live_data() {
        let (_t, state, repo, key, ready, live, original) = fixture();
        let head = state
            .root()
            .join(format!("repos/{repo}/incremental/{key}.json"));
        let other = state
            .root()
            .join(format!("repos/{repo}/incremental/{}.json", "d".repeat(64)));
        let other_before = fs::read(&other).unwrap();
        assert!(
            crate::cas::storage_status(&state)
                .unwrap()
                .collection_blocked
        );
        let dry = retire_with_state(&state, &repo, &key, None, false).unwrap();
        let digest = dry["planDigest"].as_str().unwrap();
        assert_eq!(fs::read(&head).unwrap(), original);
        assert!(!Path::new(dry["archive"].as_str().unwrap()).exists());
        assert!(retire_with_state(&state, &repo, &key, None, true).is_err());
        let applied = retire_with_state(&state, &repo, &key, Some(digest), true).unwrap();
        assert!(!head.exists());
        assert_eq!(applied["casObjectsDeleted"], 0);
        assert_eq!(
            fs::read(applied["archive"].as_str().unwrap()).unwrap(),
            original
        );
        assert!(
            retire_with_state(&state, &repo, &key, Some(digest), true).unwrap()["alreadyAbsent"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(fs::read(&other).unwrap(), other_before);
        {
            let store = CasStore::open(&state).unwrap();
            assert!(store.read(&ready, 4096).is_ok());
            assert!(store.read(&live, 4096).is_ok());
        }
        assert!(
            !crate::cas::storage_status(&state)
                .unwrap()
                .collection_blocked
        );
        crate::cas::garbage_collect_storage(&state).unwrap();
        let store = CasStore::open(&state).unwrap();
        assert!(store.read(&ready, 4096).is_err());
        assert!(store.read(&live, 4096).is_ok());
    }
    #[test]
    fn stale_bytes_file_and_parent_identity_never_remove_replacement() {
        for kind in ["bytes", "file", "directory"] {
            let (_t, state, repo, key, _ready, _live, original) = fixture();
            let dir = state.root().join(format!("repos/{repo}/incremental"));
            let head = dir.join(format!("{key}.json"));
            let dry = retire_with_state(&state, &repo, &key, None, false).unwrap();
            match kind {
                "bytes" => {
                    let mut v: Value = serde_json::from_slice(&original).unwrap();
                    v["compilerStoreKey"] = json!(format!("sha256:{}", "e".repeat(64)));
                    state
                        .write_private_atomic(&head, &canonical::bytes(&v).unwrap())
                        .unwrap();
                }
                "file" => state.write_private_atomic(&head, &original).unwrap(),
                "directory" => {
                    fs::rename(&dir, dir.with_extension("old")).unwrap();
                    state.write_private_atomic(&head, &original).unwrap();
                }
                _ => unreachable!(),
            }
            let replacement = fs::read(&head).unwrap();
            assert!(
                retire_with_state(&state, &repo, &key, dry["planDigest"].as_str(), true).is_err(),
                "{kind}"
            );
            assert_eq!(fs::read(&head).unwrap(), replacement);
        }
    }
    #[test]
    fn invalid_shape_or_archive_never_authorizes_retirement_or_recovery() {
        for kind in [
            "unknown",
            "schema",
            "canonical",
            "symlink",
            "archive",
            "receipt",
        ] {
            let (_t, state, repo, key, _ready, _live, original) = fixture();
            let head = state
                .root()
                .join(format!("repos/{repo}/incremental/{key}.json"));
            if matches!(kind, "archive" | "receipt") {
                let dry = retire_with_state(&state, &repo, &key, None, false).unwrap();
                let digest = dry["planDigest"].as_str();
                let result = retire_with_state(&state, &repo, &key, digest, true).unwrap();
                state
                    .write_private_atomic(Path::new(result[kind].as_str().unwrap()), b"{}")
                    .unwrap();
                assert!(retire_with_state(&state, &repo, &key, digest, true).is_err());
                continue;
            }
            match kind {
                "unknown" | "schema" => {
                    let mut value: Value = serde_json::from_slice(&original).unwrap();
                    if kind == "schema" {
                        value["schema"] = json!("unknown");
                    } else {
                        value["unknown"] = json!(true);
                    }
                    state
                        .write_private_atomic(&head, &canonical::bytes(&value).unwrap())
                        .unwrap();
                }
                "canonical" => {
                    let mut bytes = original.clone();
                    bytes.push(b'\n');
                    state.write_private_atomic(&head, &bytes).unwrap();
                }
                "symlink" => {
                    let outside = state.root().join("outside");
                    fs::write(&outside, &original).unwrap();
                    fs::remove_file(&head).unwrap();
                    std::os::unix::fs::symlink(outside, &head).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(retire_with_state(&state, &repo, &key, None, false).is_err());
            assert!(fs::symlink_metadata(&head).is_ok());
        }
    }
    #[test]
    fn shared_missing_reference_keeps_global_collection_blocked() {
        let (_t, state, repo, key, ready, _live, _) = fixture();
        state
            .write_private_atomic(
                &state.root().join("threads/shared.json"),
                &canonical::bytes(&json!({"ready":ready})).unwrap(),
            )
            .unwrap();
        let dry = retire_with_state(&state, &repo, &key, None, false).unwrap();
        retire_with_state(&state, &repo, &key, dry["planDigest"].as_str(), true).unwrap();
        assert!(
            crate::cas::storage_status(&state)
                .unwrap()
                .collection_blocked
        );
        assert!(crate::cas::garbage_collect_storage(&state).is_err());
    }
    #[test]
    fn prepared_receipt_recovers_before_or_after_exact_unlink() {
        for after_unlink in [false, true] {
            let (_t, state, repo, key, _ready, _live, original) = fixture();
            let dry = retire_with_state(&state, &repo, &key, None, false).unwrap();
            let digest = dry["planDigest"].as_str().unwrap();
            let plan: Plan = serde_json::from_value(dry["plan"].clone()).unwrap();
            let archive = Path::new(dry["archive"].as_str().unwrap());
            let receipt = Path::new(dry["receipt"].as_str().unwrap());
            state.write_private_atomic(archive, &original).unwrap();
            write_canonical_atomic(
                &state,
                receipt,
                &Receipt {
                    schema: RECEIPT_SCHEMA.into(),
                    plan,
                    plan_digest: digest.into(),
                    phase: "PREPARED".into(),
                },
            )
            .unwrap();
            let head = state
                .root()
                .join(format!("repos/{repo}/incremental/{key}.json"));
            if after_unlink {
                fs::remove_file(&head).unwrap();
            }
            retire_with_state(&state, &repo, &key, Some(digest), true).unwrap();
            assert!(!head.exists());
            let saved: Receipt = serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
            assert_eq!(saved.phase, "APPLIED");
            assert_eq!(fs::read(archive).unwrap(), original);
        }
    }

    #[test]
    fn retirement_waits_for_existing_cas_writer_world_lease() {
        let (_t, state, repo, key, _ready, _live, _) = fixture();
        let store = CasStore::open(&state).unwrap();
        let root = state.root().to_path_buf();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let state = StateAuthority::open(root).unwrap();
            tx.send(retire_with_state(&state, &repo, &key, None, false).is_ok())
                .unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(store);
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        worker.join().unwrap();
    }
}
