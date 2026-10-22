//! Conservative, bounded docs-local reclamation. All supported root files and
//! their transitive immutable objects remain live; unknown/missing roots block.
use super::{
    cache, digest, invalid, io_error,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::PathBuf,
    sync::Arc,
};

const FILES: usize = 100_000;
const BYTES: u64 = 512 * 1024 * 1024;
const PORTABLE: &str = "docs/evidence/objects";

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Read-only deterministic accounting and cleanup plan; nothing is deleted.
    Plan {
        #[arg(long)]
        root: PathBuf,
    },
    /// Recompute the exact plan; --apply is required for destructive changes.
    Prune {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        expected_plan: String,
        #[arg(long)]
        apply: bool,
    },
}

/// Held throughout each Repository lifetime, including cache writes before root
/// publication. Pruning refuses concurrent readers/writers rather than waiting
/// indefinitely or collecting objects between creation and publication.
pub(super) fn lease(repo: &Repository, exclusive: bool) -> Result<Arc<File>, ClewError> {
    let parent = repo.path(".codeclew")?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(repo.path(".codeclew/storage-lifecycle.lock")?)
        .map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid("storage lifecycle lock is not a regular file"));
    }
    let mode = if exclusive {
        libc::LOCK_EX | libc::LOCK_NB
    } else {
        libc::LOCK_SH | libc::LOCK_NB
    };
    if unsafe { libc::flock(file.as_raw_fd(), mode) } != 0 {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "documentation storage is in use; finish active readers/writers and retry the reviewed plan",
        ));
    }
    Ok(Arc::new(file))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    schema: String,
    root_identity: String,
    root_files: BTreeMap<String, String>,
    category_bytes: BTreeMap<String, u64>,
    sqlite_objects: BTreeMap<String, u64>,
    portable_objects: BTreeMap<String, u64>,
    retained: BTreeMap<String, BTreeSet<String>>,
    remove_sqlite: BTreeMap<String, u64>,
    remove_portable: BTreeMap<String, u64>,
    scanned_bytes: u64,
}

fn charge(total: &mut u64, bytes: u64) -> Result<(), ClewError> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| invalid("storage accounting overflow"))?;
    if *total > BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "documentation storage reference traversal exceeds 512 MiB; no cleanup performed",
        ));
    }
    Ok(())
}

fn references(text: &str, pending: &mut Vec<(String, String)>, reason: &str) {
    for tail in text.split("sha256:").skip(1) {
        if let Some(hex) = tail.get(..64)
            && hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            pending.push((format!("sha256:{hex}"), reason.into()));
        }
    }
}

fn required_references(
    value: &Value,
    available: &BTreeMap<String, u64>,
    portable: &BTreeMap<String, u64>,
) -> Result<(), ClewError> {
    match value {
        Value::Object(map) => {
            if map.get("schema").is_some_and(Value::is_string)
                && let (Some(id), Some(size)) = (
                    map.get("digest").and_then(Value::as_str),
                    map.get("size").and_then(Value::as_u64),
                )
                && id.starts_with("sha256:")
                && available.get(id).or_else(|| portable.get(id)) != Some(&size)
            {
                return Err(invalid(
                    "retained immutable object is missing or has changed size; restore its closure before cleanup",
                ));
            }
            for value in map.values() {
                required_references(value, available, portable)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                required_references(item, available, portable)?;
            }
        }
        Value::String(text) => {
            if let Some((id, size)) = text.split_once('/')
                && id.starts_with("sha256:")
                && id.len() == 71
                && let Ok(size) = size.parse::<u64>()
                && available.get(id).or_else(|| portable.get(id)) != Some(&size)
            {
                return Err(invalid(
                    "retained snapshot handle is missing; restore or explicitly retire that root before cleanup",
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn scan(repo: &Repository) -> Result<Plan, ClewError> {
    let sqlite_objects = cache::owned_objects(repo, FILES)?;
    let mut plan = Plan {
        schema: "codeclew-documentation-storage-plan/1.0".into(),
        root_identity: digest(&repo.root.to_string_lossy())?,
        root_files: BTreeMap::new(),
        category_bytes: BTreeMap::new(),
        sqlite_objects,
        portable_objects: BTreeMap::new(),
        retained: BTreeMap::new(),
        remove_sqlite: BTreeMap::new(),
        remove_portable: BTreeMap::new(),
        scanned_bytes: 0,
    };
    let mut pending = Vec::new();
    let mut files = 0;
    // The entire documentation root is conservative authority, except Git's own
    // administration and content-addressed object payloads traversed below.
    for entry in walkdir::WalkDir::new(&repo.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry.map_err(io_error)?;
        let relative = entry
            .path()
            .strip_prefix(&repo.root)
            .map_err(io_error)?
            .to_str()
            .ok_or_else(|| invalid("non-UTF-8 documentation storage path"))?
            .replace('\\', "/");
        if relative.is_empty() {
            continue;
        }
        let checked_path = repo.path(&relative)?; // Reject all symlinks and traversal.
        if !entry.file_type().is_file() {
            if !entry.file_type().is_dir() {
                return Err(invalid("unsupported documentation storage entry"));
            }
            continue;
        }
        files += 1;
        if files > FILES {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "documentation storage exceeds 100000 files; no cleanup performed",
            ));
        }
        let size = entry.metadata().map_err(io_error)?.len();
        if relative.starts_with(".codeclew/cache/objects-")
            && (relative.ends_with(".sqlite3")
                || relative.ends_with(".sqlite3-wal")
                || relative.ends_with(".sqlite3-shm"))
        {
            continue;
        }

        let category = if relative.starts_with(&format!("{PORTABLE}/")) {
            "portableObjects"
        } else if relative.starts_with(".codeclew/cache/objects-") {
            "sqliteFiles"
        } else if relative.starts_with("docs/generated/") {
            "frozenPublications"
        } else if relative.starts_with(".codeclew/") {
            "localMetadata"
        } else {
            "authoredAndOtherFiles"
        };
        *plan.category_bytes.entry(category.into()).or_default() += size;

        if let Some(name) = relative.strip_prefix(&format!("{PORTABLE}/")) {
            let hex = name
                .strip_suffix(".json")
                .ok_or_else(|| invalid("unknown portable evidence object; cleanup refused"))?;
            if hex.len() != 64
                || !hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err(invalid("unknown portable evidence object; cleanup refused"));
            }
            plan.portable_objects.insert(format!("sha256:{hex}"), size);
            continue;
        }
        // SQLite/WAL/SHM bytes are engine-managed, never unlink them. Logical
        // object inventory binds the plan; pruning releases rows, not file size.
        if relative.starts_with(".codeclew/cache/objects-")
            && (relative.ends_with(".sqlite3")
                || relative.ends_with(".sqlite3-wal")
                || relative.ends_with(".sqlite3-shm"))
        {
            continue;
        }
        if relative.ends_with(".lock") {
            continue;
        }
        if size > store::MAX_RECORD {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "storage root file exceeds 128 MiB; no cleanup performed",
            ));
        }
        charge(&mut plan.scanned_bytes, size)?;
        let bytes = fs::read(checked_path).map_err(io_error)?;
        if bytes.len() as u64 != size {
            return Err(invalid("storage root changed during planning; retry"));
        }
        plan.root_files
            .insert(relative.clone(), cache::content_digest(&bytes));
        if let Ok(text) = std::str::from_utf8(&bytes) {
            references(text, &mut pending, &format!("root:{relative}"));
        }
    }
    for name in plan.root_files.keys() {
        if matches!(
            std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str()),
            Some("json" | "yaml" | "yml")
        ) {
            charge(
                &mut plan.scanned_bytes,
                fs::metadata(repo.path(name)?).map_err(io_error)?.len(),
            )?;
            let value: Value = store::read(&repo.path(name)?, store::MAX_RECORD)?;
            required_references(&value, &plan.sqlite_objects, &plan.portable_objects)?;
        }
    }
    // Missing typed immutable references are not treated as unreferenced data.
    // Raw content digests and external source hashes need not be object IDs.
    let mut visited = BTreeSet::new();
    while let Some((id, reason)) = pending.pop() {
        if !plan.sqlite_objects.contains_key(&id) && !plan.portable_objects.contains_key(&id) {
            continue;
        }
        plan.retained.entry(id.clone()).or_default().insert(reason);
        if !visited.insert(id.clone()) {
            continue;
        }
        if visited.len() > FILES {
            return Err(invalid("storage closure exceeds object bound"));
        }
        if let Some(size) = plan.portable_objects.get(&id) {
            charge(&mut plan.scanned_bytes, *size)?;
            if *size > store::MAX_RECORD {
                return Err(invalid("portable object exceeds storage read bound"));
            }
            let bytes =
                fs::read(repo.path(&format!("{PORTABLE}/{}.json", &id[7..]))?).map_err(io_error)?;
            if bytes.len() as u64 != *size || cache::content_digest(&bytes) != id {
                return Err(invalid(
                    "referenced portable evidence is corrupt; cleanup refused",
                ));
            }
        }
        let bytes = if let Some(size) = plan.sqlite_objects.get(&id) {
            charge(&mut plan.scanned_bytes, *size)?;
            let reference =
                cache::ObjectRef::new("storage-reference-traversal/1.0".into(), id.clone(), *size);
            cache::get(repo, &reference, store::MAX_RECORD)?
                .ok_or_else(|| invalid("referenced storage object is missing; cleanup refused"))?
        } else {
            let size = plan.portable_objects[&id];
            charge(&mut plan.scanned_bytes, size)?;
            if size > store::MAX_RECORD {
                return Err(invalid("portable object exceeds storage read bound"));
            }
            let path = repo.path(&format!("{PORTABLE}/{}.json", &id[7..]))?;
            let bytes = fs::read(path).map_err(io_error)?;
            if cache::content_digest(&bytes) != id || bytes.len() as u64 != size {
                return Err(invalid(
                    "referenced portable evidence is corrupt; cleanup refused",
                ));
            }
            bytes
        };
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            required_references(&value, &plan.sqlite_objects, &plan.portable_objects)?;
        }
        if let Ok(text) = std::str::from_utf8(&bytes) {
            references(text, &mut pending, &format!("object:{id}"));
        }
    }
    plan.remove_sqlite = plan
        .sqlite_objects
        .iter()
        .filter(|(d, _)| !plan.retained.contains_key(*d))
        .map(|(d, n)| (d.clone(), *n))
        .collect();
    plan.remove_portable = plan
        .portable_objects
        .iter()
        .filter(|(d, _)| !plan.retained.contains_key(*d))
        .map(|(d, n)| (d.clone(), *n))
        .collect();
    Ok(plan)
}

fn report(plan: &Plan, applied: bool) -> Result<Value, ClewError> {
    let sqlite: u64 = plan.remove_sqlite.values().sum();
    let portable: u64 = plan.remove_portable.values().sum();
    Ok(
        json!({"schema":plan.schema,"status":if applied {"PRUNED"} else {"DRY_RUN"},"planDigest":digest(plan)?,"plan":plan,
        "accounting":{"sqlitePayloadBytes":plan.sqlite_objects.values().sum::<u64>(),"portablePayloadBytes":plan.portable_objects.values().sum::<u64>(),"reclaimableSqlitePayloadBytes":sqlite,"reclaimablePortableFileBytes":portable,"retainedSqlitePayloadBytes":plan.sqlite_objects.values().sum::<u64>()-sqlite,"retainedPortableFileBytes":plan.portable_objects.values().sum::<u64>()-portable},
        "policy":"All current and historical root files, snapshots, pins, proposals and active Work are retained transitively; no implicit retention expiry. SQLite row deletion creates reusable database space, not a promised file-size reduction. Private CODECLEW_HOME is separate: use clew storage gc.",
        "nextAction":if applied {"Recompute the plan to confirm idempotence; retained documentation can be read without recapture."} else {"Review plan.removeSqlite/removePortable and reference reasons, then use docs storage prune --expected-plan <planDigest> --apply on this root."}}),
    )
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    let (root, expected, apply) = match command {
        Command::Plan { root } => (root, None, false),
        Command::Prune {
            root,
            expected_plan,
            apply,
        } => (root, Some(expected_plan), apply),
    };
    let repo = Repository::open_storage(&root, apply)?;
    let _lock = repo.lock()?;
    let plan = scan(&repo)?;
    if let Some(expected) = expected
        && expected != digest(&plan)?
    {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "storage plan changed or belongs to another root; review a new plan before applying",
        ));
    }
    if apply {
        super::object_layout::with_store(&repo, |store| store.remove_exact(&plan.remove_sqlite))?;
        for id in plan.remove_portable.keys() {
            fs::remove_file(repo.path(&format!("{PORTABLE}/{}.json", &id[7..]))?)
                .map_err(io_error)?;
        }
    }
    report(&plan, apply)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Repository) {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "Storage acceptance").unwrap();
        let r = Repository::open(t.path()).unwrap();
        (t, r)
    }
    #[test]
    fn shared_history_work_and_pins_preserve_closure_and_prune_only_orphans() {
        let (t, repo) = setup();
        let live = cache::put(&repo, "source/1", b"shared source").unwrap();
        let parent = cache::put_json(&repo, "snapshot/1", &json!({"source":live})).unwrap();
        let orphan = cache::put(&repo, "source/1", b"orphan").unwrap();
        for name in [
            ".codeclew/work/active.json",
            ".codeclew/pins/retained.json",
            "docs/generated/old/publication.json",
        ] {
            repo.atomic(
                name,
                &super::super::bytes(&json!({"snapshot":parent})).unwrap(),
            )
            .unwrap();
        }
        let portable = b"{\"owned\":\"unreferenced\"}";
        let portable_id = cache::content_digest(portable);
        repo.atomic(&format!("{PORTABLE}/{}.json", &portable_id[7..]), portable)
            .unwrap();
        let plan = scan(&repo).unwrap();
        assert_eq!(
            plan.remove_sqlite,
            BTreeMap::from([(orphan.digest, orphan.size)])
        );
        assert_eq!(plan.remove_portable[&portable_id], portable.len() as u64);
        assert!(plan.retained[&live.digest].contains(&format!("object:{}", parent.digest)));
        let expected = digest(&plan).unwrap();
        assert_eq!(
            run(Command::Prune {
                root: t.path().into(),
                expected_plan: expected.clone(),
                apply: true
            })
            .unwrap_err()
            .code,
            ErrorCode::WwConflict
        );
        drop(repo);
        let applied = run(Command::Prune {
            root: t.path().into(),
            expected_plan: expected,
            apply: true,
        })
        .unwrap();
        assert_eq!(applied["status"], "PRUNED");
        assert_eq!(applied["accounting"]["reclaimableSqlitePayloadBytes"], 6);
        let repo = Repository::open(t.path()).unwrap();
        assert_eq!(
            cache::get(&repo, &live, 1024).unwrap().unwrap(),
            b"shared source"
        );
        assert!(scan(&repo).unwrap().remove_sqlite.is_empty());
        assert!(scan(&repo).unwrap().remove_portable.is_empty());
    }
    #[test]
    fn new_reference_wrong_root_and_missing_closure_refuse_stale_plan() {
        let (t, repo) = setup();
        let orphan = cache::put(&repo, "source/1", b"orphan").unwrap();
        let before = digest(&scan(&repo).unwrap()).unwrap();
        repo.atomic(
            ".codeclew/work/new.json",
            &super::super::bytes(&json!({"evidence":orphan})).unwrap(),
        )
        .unwrap();
        drop(repo);
        assert_eq!(
            run(Command::Prune {
                root: t.path().into(),
                expected_plan: before.clone(),
                apply: true
            })
            .unwrap_err()
            .code,
            ErrorCode::WwConflict
        );
        let (other, other_repo) = setup();
        drop(other_repo);
        assert_eq!(
            run(Command::Prune {
                root: other.path().into(),
                expected_plan: before,
                apply: true
            })
            .unwrap_err()
            .code,
            ErrorCode::WwConflict
        );
        let repo = Repository::open(t.path()).unwrap();
        repo.atomic(".codeclew/pins/missing.json",br#"{"snapshot":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/123"}"#).unwrap();
        assert!(
            scan(&repo)
                .unwrap_err()
                .message
                .contains("snapshot handle is missing")
        );
    }
    #[test]
    fn symlinks_and_corrupt_referenced_portable_objects_are_not_collected() {
        let (t, repo) = setup();
        let id = format!("sha256:{}", "a".repeat(64));
        repo.atomic(
            ".codeclew/work/live.json",
            &super::super::bytes(&json!({"evidence":{"schema":"owned/1","digest":id,"size":3}}))
                .unwrap(),
        )
        .unwrap();
        repo.atomic(&format!("{PORTABLE}/{}.json", &id[7..]), b"bad")
            .unwrap();
        assert!(scan(&repo).is_err());
        std::os::unix::fs::symlink("/tmp", t.path().join("linked")).unwrap();
        assert!(scan(&repo).is_err());
    }
}
