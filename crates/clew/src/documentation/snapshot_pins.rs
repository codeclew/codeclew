//! Explicit names for immutable documentation snapshots.
//!
//! Pins are deliberately small mutable records.  They register the reader
//! closure that callers may later use; they do not copy a check or create a
//! second object-store authority.

use super::{
    check::Check,
    invalid,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "codeclew-documentation-snapshot-pin/1.0";
const READER_CONTRACT: &str = "codeclew-documentation-check-reader/1.0";
const DIRECTORY: &str = ".codeclew/cache/pins";
const MAX_PINS: usize = 4096;
const MAX_BYTES: u64 = 4096;
const STAGING_PREFIX: &str = ".pin-staging-";
const MAX_ENTRIES: usize = MAX_PINS * 2;
const MAX_PARENT_DEPTH: usize = 64;
const RETENTION_SCOPE: &str = "DOCUMENTATION_SNAPSHOT_READER_DATA";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PinRecord {
    schema: String,
    name: String,
    snapshot: String,
    reader_contract: String,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Recover an immutable historical snapshot from explicitly named captures.
    Recover {
        #[arg(long)]
        root: PathBuf,
        /// Basename of a current-format manifest in .codeclew/cache; repeat per service.
        #[arg(long = "capture", required = true)]
        captures: Vec<String>,
    },
    Pin {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        snapshot: String,
    },
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        name: String,
    },
    List {
        #[arg(long)]
        root: PathBuf,
    },
    Unpin {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        name: String,
    },
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Recover { root, captures } => super::capture_recovery::run(&root, &captures),
        Command::Pin {
            root,
            name,
            snapshot,
        } => pin(&root, &name, &snapshot),
        Command::Show { root, name } => show(&root, &name),
        Command::List { root } => list(&root),
        Command::Unpin { root, name } => unpin(&root, &name),
    }
}

fn open(root: &Path) -> Result<Repository, ClewError> {
    Repository::open(root)
}

fn marker_name(name: &str) -> Result<String, ClewError> {
    if !store::valid_id(name) {
        return Err(invalid(
            "snapshot pin name is not a valid documentation identifier",
        ));
    }
    Ok(format!("{name}.json"))
}

fn pin_path(repo: &Repository, name: &str) -> Result<PathBuf, ClewError> {
    let file = marker_name(name)?;
    repo.path(&format!("{DIRECTORY}/{file}"))
}

fn pins_dir(repo: &Repository) -> Result<PathBuf, ClewError> {
    repo.path(DIRECTORY)
}

fn read_marker(repo: &Repository, name: &str) -> Result<Option<PinRecord>, ClewError> {
    let path = pin_path(repo, name)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ClewError::new(ErrorCode::StateCorrupt, error.to_string())),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "snapshot pin is not a bounded regular file",
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|_| ClewError::new(ErrorCode::StateCorrupt, "snapshot pin cannot be read"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "snapshot pin exceeds its size bound",
        ));
    }
    let record: PinRecord = serde_json::from_slice(&bytes)
        .map_err(|_| ClewError::new(ErrorCode::StateCorrupt, "snapshot pin record is malformed"))?;
    validate_record(name, &record)?;
    Ok(Some(record))
}

fn validate_record(filename_name: &str, record: &PinRecord) -> Result<(), ClewError> {
    if record.schema != SCHEMA
        || record.name != filename_name
        || !store::valid_id(&record.name)
        || record.reader_contract != READER_CONTRACT
        || !canonical_handle(&record.snapshot)
    {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "snapshot pin record is invalid",
        ));
    }
    Ok(())
}

fn canonical_handle(handle: &str) -> bool {
    let Some((digest, size)) = handle.rsplit_once('/') else {
        return false;
    };
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return false;
    };
    let Ok(size_value) = size.parse::<u64>() else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        && size == size_value.to_string()
        && size_value > 0
        && size_value <= super::check::PORTABLE_CACHE_MAX_BYTES
}

fn verify_snapshot(repo: &Repository, handle: &str) -> Result<Check, ClewError> {
    let checked = Check::load_snapshot(repo, handle)?;
    let mut seen = BTreeSet::from([handle.to_owned()]);
    let mut parent = checked
        .composition
        .as_ref()
        .map(|value| value.parent.clone());
    while let Some(handle) = parent {
        if seen.len() >= MAX_PARENT_DEPTH {
            return Err(invalid(
                "snapshot composition parent closure exceeds its bound",
            ));
        }
        if !seen.insert(handle.clone()) {
            return Err(invalid("snapshot composition parent cycle detected"));
        }
        let original = Check::load_snapshot(repo, &handle)?;
        parent = original
            .composition
            .as_ref()
            .map(|value| value.parent.clone());
    }
    Ok(checked)
}

fn pin(root: &Path, name: &str, snapshot: &str) -> Result<Value, ClewError> {
    let repo = open(root)?;
    let _lock = repo.lock()?;
    let existing = read_marker(&repo, name)?;
    if let Some(record) = &existing
        && record.snapshot != snapshot
    {
        return Err(invalid(
            "snapshot pin name already refers to a different snapshot; unpin it before rebinding",
        ));
    }
    if !canonical_handle(snapshot) {
        return Err(invalid(
            "snapshot pin requires a canonical immutable snapshot handle",
        ));
    }
    if existing.is_none() {
        let (pins, staging) = registered_pins(&repo)?;
        if pins.len() >= MAX_PINS || pins.len() + staging >= MAX_ENTRIES {
            return Err(invalid(
                "snapshot pin registry has no capacity for another root",
            ));
        }
    }
    let checked = verify_snapshot(&repo, snapshot)?;
    if existing.is_none() {
        let record = PinRecord {
            schema: SCHEMA.into(),
            name: name.into(),
            snapshot: snapshot.into(),
            reader_contract: READER_CONTRACT.into(),
        };
        let bytes = super::bytes(&record)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("snapshot pin record exceeds its size bound"));
        }
        let path = pin_path(&repo, name)?;
        let parent = pins_dir(&repo)?;
        fs::create_dir_all(&parent).map_err(super::io_error)?;
        // Recognizable uncommitted staging never participates in root enumeration.
        let mut temp = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .tempfile_in(&parent)
            .map_err(super::io_error)?;
        temp.write_all(&bytes).map_err(super::io_error)?;
        temp.as_file().sync_all().map_err(super::io_error)?;
        pin_path(&repo, name)?;
        temp.persist(&path).map_err(super::io_error)?;
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(super::io_error)?;
    }
    Ok(pin_result(&repo, name, snapshot, &checked, "PINNED"))
}

fn show(root: &Path, name: &str) -> Result<Value, ClewError> {
    let repo = open(root)?;
    let _lock = repo.lock()?;
    let record = read_marker(&repo, name)?.ok_or_else(|| invalid("snapshot pin is absent"))?;
    let checked = verify_snapshot(&repo, &record.snapshot)?;
    Ok(pin_result(
        &repo,
        name,
        &record.snapshot,
        &checked,
        "READABLE",
    ))
}

fn pin_result(
    repo: &Repository,
    name: &str,
    snapshot: &str,
    checked: &Check,
    status: &str,
) -> Value {
    let services: BTreeMap<_, _> = checked
        .services
        .iter()
        .map(|(id, evidence)| {
            (
                id.clone(),
                json!({"revision": evidence.revision, "coverage": evidence.coverage}),
            )
        })
        .collect();
    json!({
        "schema": SCHEMA,
        "status": status,
        "name": name,
        "snapshot": snapshot,
        "namedRoot": {"name": name, "snapshot": snapshot, "root": repo.root},
        "readability": "READABLE_NOW_CURRENT_READER",
        "readerContract": READER_CONTRACT,
        "sourceFreshness": "UNVERIFIED",
        "services": services,
        "retentionScope": RETENTION_SCOPE,
        "retention": {"registered": true, "payloadReclamation": "NOT_IMPLEMENTED", "activeReaderLifetime": "NOT_IMPLEMENTED"}
    })
}

fn registered_pins(repo: &Repository) -> Result<(Vec<Value>, usize), ClewError> {
    let directory = pins_dir(repo)?;
    let mut pins = Vec::new();
    let mut staging = 0;
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(error) => return Err(ClewError::new(ErrorCode::StateCorrupt, error.to_string())),
    };
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ENTRIES {
            return Err(invalid(
                "snapshot pin directory entry count exceeds its bound",
            ));
        }
        if pins.len() > MAX_PINS {
            return Err(invalid("snapshot pin count exceeds its bound"));
        }
        let entry = entry.map_err(|_| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                "snapshot pin directory cannot be read",
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                "snapshot pin entry cannot be inspected",
            )
        })?;
        if !metadata.is_file() {
            return Err(ClewError::new(
                ErrorCode::StateCorrupt,
                "snapshot pin directory contains a non-regular entry",
            ));
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        if filename.starts_with(STAGING_PREFIX) {
            if metadata.len() > MAX_BYTES {
                return Err(invalid("snapshot pin staging entry exceeds its bound"));
            }
            staging += 1;
            continue;
        }
        if pins.len() >= MAX_PINS {
            return Err(invalid("snapshot pin count exceeds its bound"));
        }
        let Some(name) = filename.strip_suffix(".json") else {
            return Err(invalid("snapshot pin filename is malformed"));
        };
        let record = read_marker(repo, name)?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                "snapshot pin disappeared during listing",
            )
        })?;
        pins.push(
            json!({"name": record.name, "snapshot": record.snapshot, "readability": "NOT_CHECKED"}),
        );
    }
    pins.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok((pins, staging))
}

fn list(root: &Path) -> Result<Value, ClewError> {
    let repo = open(root)?;
    let _lock = repo.lock()?;
    let (pins, staging) = registered_pins(&repo)?;
    Ok(
        json!({"schema": SCHEMA, "status": "READY", "pins": pins, "interruptedStagingEntries": staging,
        "retentionScope": RETENTION_SCOPE, "hint": "use show to verify snapshot readability"}),
    )
}

fn unpin(root: &Path, name: &str) -> Result<Value, ClewError> {
    let repo = open(root)?;
    let _lock = repo.lock()?;
    let path = pin_path(&repo, name)?;
    let Some(record) = read_marker(&repo, name)? else {
        return Ok(json!({"schema": SCHEMA, "status": "ABSENT", "name": name}));
    };
    let _ = record;
    fs::remove_file(&path).map_err(|_| invalid("snapshot pin could not be removed"))?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| invalid("snapshot pin directory could not be synchronized"))?;
    }
    Ok(json!({"schema": SCHEMA, "status": "REMOVED", "name": name}))
}
