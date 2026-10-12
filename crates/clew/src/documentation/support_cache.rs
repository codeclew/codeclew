//! Explicit private copies of docs-local state. Never follows source bindings,
//! visits a Git checkout, opens the source SQLite store, or invokes a producer.
use super::{
    invalid,
    support_collect::{Signature, path, read, signature},
};
use crate::error::{ClewError, ErrorCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

const MAX_FILES: usize = 4096;
const WORK_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn work_snapshot(root: &Path, work: &str) -> Result<String, ClewError> {
    let directory = super::work::directory(work)?;
    let file = path(root, &format!("{directory}/work.json"))?;
    let before = signature(&file)?;
    let value: Value = serde_json::from_slice(&read(&file, WORK_BYTES)?)
        .map_err(|_| invalid("invalid diagnostic Work"))?;
    if signature(&file)? != before || value["id"] != work {
        return Err(invalid(
            "diagnostic Work changed or has a different identity",
        ));
    }
    value["snapshot"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("diagnostic Work has no snapshot"))
}

fn files(
    root: &Path,
    relative: &str,
    names: &mut BTreeSet<String>,
    cache: bool,
) -> Result<(), ClewError> {
    let directory = path(root, relative)?;
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(invalid("diagnostic state directory is unavailable")),
    };
    let mut count = 0;
    for entry in entries {
        count += 1;
        if count > MAX_FILES {
            return Err(invalid("diagnostic state file count exceeded"));
        }
        let entry = entry.map_err(|_| invalid("diagnostic state entry is unavailable"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("diagnostic state name is not UTF-8"))?;
        let selected = name.ends_with(".json")
            || (cache
                && name.starts_with("objects-")
                && (name.ends_with(".sqlite3")
                    || name.ends_with(".sqlite3-wal")
                    || name.ends_with(".sqlite3-journal")));
        if selected {
            names.insert(format!("{relative}/{name}"));
        }
    }
    Ok(())
}

fn optional(root: &Path, relative: &str, names: &mut BTreeSet<String>) -> Result<(), ClewError> {
    let p = path(root, relative)?;
    if fs::symlink_metadata(p).is_ok() {
        names.insert(relative.into());
    }
    Ok(())
}

fn identifier(value: &Value, length: usize) -> Option<&str> {
    value
        .as_str()
        .filter(|s| s.len() == length && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn plan(root: &Path, work: Option<&str>) -> Result<BTreeSet<String>, ClewError> {
    let mut names = BTreeSet::new();
    files(root, ".codeclew/cache", &mut names, true)?;
    if !names.iter().any(|name| name.ends_with(".sqlite3")) {
        return Err(invalid("documentation cache database is unavailable"));
    }
    files(root, ".codeclew/cache/pins", &mut names, false)?;
    optional(root, "codeclew-docs.yaml", &mut names)?;
    files(root, "catalog", &mut names, false)?;
    for category in ["services", "scenarios", "interactions", "models"] {
        files(root, &format!("catalog/{category}"), &mut names, false)?;
    }
    if let Some(work) = work {
        let directory = super::work::directory(work)?;
        files(root, &directory, &mut names, false)?;
        let pointer = path(root, &format!("{directory}/latest-run.json"))?;
        if let Ok(bytes) = read(&pointer, 1024 * 1024)
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && let Some(run) = identifier(&value["run"], 32)
        {
            let report = format!(".codeclew/jobs/{run}.json");
            optional(root, &report, &mut names)?;
            optional(
                root,
                &format!(".codeclew/jobs/{run}/cancel.json"),
                &mut names,
            )?;
            files(
                root,
                &format!(".codeclew/jobs/{run}/checkpoints"),
                &mut names,
                false,
            )?;
            files(root, &format!(".codeclew/drafts/{work}"), &mut names, false)?;
            files(
                root,
                &format!(".codeclew/drafts/{work}/{run}"),
                &mut names,
                false,
            )?;
            for role in ["author", "reviewer", "fallback"] {
                files(
                    root,
                    &format!(".codeclew/model-id-maps/{run}/{role}"),
                    &mut names,
                    false,
                )?;
            }
            if let Ok(bytes) = read(&path(root, &report)?, 8 * 1024 * 1024)
                && let Ok(report) = serde_json::from_slice::<Value>(&bytes)
            {
                if let Some(id) = identifier(&report["proposal"], 64) {
                    optional(root, &format!(".codeclew/proposals/{id}.json"), &mut names)?;
                }
                for attempt in report["attempts"].as_array().into_iter().flatten() {
                    if let Some(invocation) = identifier(&attempt["invocation"], 32) {
                        for category in [
                            "job-inputs",
                            "job-results",
                            "job-model-inputs",
                            "job-model-wire-results",
                        ] {
                            optional(
                                root,
                                &format!(".codeclew/{category}/{invocation}.json"),
                                &mut names,
                            )?;
                        }
                    }
                }
            }
        }
    }
    if names.len() > MAX_FILES {
        return Err(invalid("diagnostic state file count exceeded"));
    }
    Ok(names)
}

fn directory(path: &Path) -> Result<(), ClewError> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| invalid("private cache directory could not be created"))
}

fn copy_file(
    source: &Path,
    target: &Path,
    budget: u64,
    expected: &Signature,
) -> Result<(u64, String), ClewError> {
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)
        .map_err(|_| invalid("diagnostic state file is unavailable"))?;
    let metadata = input
        .metadata()
        .map_err(|_| invalid("diagnostic state file is unavailable"))?;
    if (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    ) != *expected
    {
        return Err(invalid("diagnostic state changed before copying"));
    }
    if !metadata.is_file() || metadata.len() > budget {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "diagnostic cache exceeds the byte budget",
        ));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(target)
        .map_err(|_| invalid("diagnostic state copy could not be created"))?;
    let mut hash = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let bytes = input
            .read(&mut buffer)
            .map_err(|_| invalid("diagnostic state copy could not be read"))?;
        if bytes == 0 {
            break;
        }
        copied += bytes as u64;
        if copied > budget || copied > metadata.len() {
            return Err(invalid("diagnostic state changed while copying"));
        }
        output
            .write_all(&buffer[..bytes])
            .map_err(|_| invalid("diagnostic state copy could not be written"))?;
        hash.update(&buffer[..bytes]);
    }
    if copied != metadata.len() {
        return Err(invalid("diagnostic state changed while copying"));
    }
    output
        .sync_all()
        .map_err(|_| invalid("diagnostic state copy could not be finalized"))?;
    Ok((copied, format!("sha256:{}", hex::encode(hash.finalize()))))
}

pub(super) fn copy(
    root: &Path,
    output: &Path,
    work: Option<&str>,
    max_bytes: u64,
) -> Result<(Vec<Value>, Value), ClewError> {
    let root = root
        .canonicalize()
        .map_err(|_| invalid("diagnostic root is unavailable"))?;
    let names = plan(&root, work)?;
    let mut signatures = vec![];
    let mut total = 0u64;
    for name in &names {
        let source = path(&root, name)?;
        let state = signature(&source)?;
        total = total
            .checked_add(state.2)
            .ok_or_else(|| invalid("diagnostic byte count overflowed"))?;
        if total > max_bytes {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "diagnostic cache exceeds the byte budget",
            ));
        }
        signatures.push((name.clone(), source, state));
    }
    let stage = output.join(".private-cache-pending");
    super::support_collect::fresh_directory(&stage)?;
    let attempt = (|| {
        let mut artifacts = vec![];
        let mut copied = 0;
        for (name, source, state) in &signatures {
            let target = stage.join(name);
            directory(target.parent().unwrap())?;
            let (bytes, digest) = copy_file(source, &target, max_bytes - copied, state)?;
            copied += bytes;
            artifacts.push(
                json!({"path":format!("PRIVATE_CACHE/{name}"),"bytes":bytes,"digest":digest}),
            );
        }
        if plan(&root, work)? != names
            || signatures
                .iter()
                .any(|(_, source, state)| signature(source).ok().as_ref() != Some(state))
        {
            return Err(invalid(
                "diagnostic cache changed while copying; finish active runs and retry",
            ));
        }
        fs::rename(&stage, output.join("PRIVATE_CACHE"))
            .map_err(|_| invalid("diagnostic cache could not be finalized"))?;
        Ok((
            artifacts,
            json!({"status":"COPIED_NOT_HYDRATED","scope":"ENTIRE_DOCUMENTATION_CACHE_AND_SELECTED_WORK","work":work,"root":"PRIVATE_CACHE","copiedBytes":copied,"files":names.len(),"sourceFragmentsMayBeIncluded":true,"gitRepositoryIncluded":false,"sourceStoreModified":false,"walPolicy":"COPY_STABLE_RAW_DATABASE_AND_WAL_WITHOUT_CHECKPOINT","closureVerification":"NOT_PERFORMED"}),
        ))
    })();
    if attempt.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    attempt
}
