//! Bounded, stopped-store diagnostics. No Repository::open, barriers, capture,
//! source hydration, or writes to the documentation root are permitted here.
use super::{check::Check, invalid, source_inputs, sqlite_objects::ImmutableSqliteObjects};
use crate::{
    canonical,
    error::{ClewError, ErrorCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const METADATA_LIMIT: u64 = 8 * 1024 * 1024;
pub const FILE_LIMIT: usize = 1024 * 1024;
pub const REPORT_SCHEMA: &str = "codeclew-support-report/1.0";
pub const BUNDLE_SCHEMA: &str = "codeclew-support-bundle/1.0";
const README: &str = "# Codeclew diagnostic bundle\n\nreport.json contains allowlisted metadata only. Snapshot/content digests and hashed\nservice identities can correlate a repository; review before sharing. No source,\nenvironment values, arguments, arbitrary error text or URLs are included there.\n\nPRIVATE_SAVED_DETAILS.json, when present, contains original private error text.\nDo not share it without review. All files are mode 0600 in a mode 0700 directory.\n\nInspection is MANIFEST_METADATA only. Sources, compiler facts, dependencies and\nthe full object closure are NOT_VERIFIED. Current declarations are not checked;\na historical snapshot remains diagnostic evidence after declarations change.\nA nonempty SQLite WAL/journal yields PARTIAL: finish the active writer and retry.\nNo store files are copied or changed, and no compiler, doctor, model or network\noperation is started by collection. A cold source launcher may build its runtime.\nNo upload is performed. manifest.json is finalized last.\n";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Layout {
    schema: String,
    policy: String,
    store_id: String,
    database: String,
}

type Signature = (u64, u64, u64, i64, i64, i64, i64);
fn signature(path: &Path) -> Result<Signature, ClewError> {
    let m =
        fs::symlink_metadata(path).map_err(|_| invalid("diagnostic metadata is unavailable"))?;
    if !m.is_file() {
        return Err(invalid("diagnostic metadata must be a regular file"));
    }
    Ok((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}

fn path(root: &Path, relative: &str) -> Result<PathBuf, ClewError> {
    super::store::relative(relative)?;
    let mut result = root.to_path_buf();
    for component in Path::new(relative).components() {
        result.push(component);
        match fs::symlink_metadata(&result) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid("diagnostic metadata symlinks are refused"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid("diagnostic metadata is unavailable")),
        }
    }
    Ok(result)
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, ClewError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| invalid("diagnostic metadata is unavailable"))?;
    let m = file
        .metadata()
        .map_err(|_| invalid("diagnostic metadata is unavailable"))?;
    if !m.is_file() || m.len() > limit {
        return Err(invalid("diagnostic metadata exceeds its read budget"));
    }
    let mut data = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|_| invalid("diagnostic metadata is unavailable"))?;
    if data.len() as u64 != m.len() || data.len() as u64 > limit {
        return Err(invalid("diagnostic metadata changed while reading"));
    }
    Ok(data)
}

fn quiescent(root: &Path, database: &str) -> Result<(), ClewError> {
    for suffix in ["-wal", "-journal"] {
        let p = path(root, &format!("{database}{suffix}"))?;
        match fs::symlink_metadata(p) {
            Ok(m) if !m.is_file() || m.len() != 0 => return Err(invalid("ACTIVE_STORE")),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid("ACTIVE_STORE")),
        }
    }
    // SQLite immutable mode does not create/read shm. Reject a symlink anyway.
    path(root, &format!("{database}-shm"))?;
    Ok(())
}

fn handle(value: &str) -> Result<(&str, u64), ClewError> {
    let (digest, bytes) = value
        .rsplit_once('/')
        .ok_or_else(|| invalid("invalid diagnostic snapshot handle"))?;
    let size: u64 = bytes
        .parse()
        .map_err(|_| invalid("invalid diagnostic snapshot size"))?;
    if !valid_digest(digest)
        || size == 0
        || value != format!("{digest}/{size}")
        || size > METADATA_LIMIT
    {
        return Err(invalid("invalid or oversized diagnostic snapshot handle"));
    }
    Ok((digest, size))
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn identity(value: &str) -> Value {
    json!(canonical::hash_bytes(value.as_bytes()))
}

/// Only the live invocation-owned directory is eligible. Historical evidence
/// paths and external --debug-output directories are never imported.
pub(crate) fn live_private_directory() -> Option<PathBuf> {
    if std::env::var_os("CLEW_DIAGNOSTIC_PRIVATE")? != "1" {
        return None;
    }
    let root = PathBuf::from(std::env::var_os("CLEW_DIAGNOSTIC_REPORT_DIR")?);
    let directory = PathBuf::from(std::env::var_os("CLEW_DIAGNOSTIC_DEBUG_DIR")?);
    (directory == root.join("private")).then_some(directory)
}

/// A shared live worker/Maven budget. Scan only our small private output
/// directory under one process mutex, never any documentation/source closure.
pub(crate) fn write_private_artifacts(
    directory: &Path,
    artifacts: &[(&str, &[u8])],
) -> Result<(), ClewError> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = LOCK
        .lock()
        .map_err(|_| invalid("private diagnostic budget unavailable"))?;
    let metadata = fs::symlink_metadata(directory)
        .map_err(|_| invalid("private diagnostic directory unavailable"))?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(invalid(
            "private diagnostic directory must be caller-owned mode 0700",
        ));
    }
    let mut total = 0u64;
    let mut count = 0usize;
    for entry in
        fs::read_dir(directory).map_err(|_| invalid("private diagnostic directory unavailable"))?
    {
        count += 1;
        if count > 128 {
            return Err(invalid("private diagnostic artifact count exceeded"));
        }
        let entry = entry.map_err(|_| invalid("private diagnostic artifact unavailable"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|_| invalid("private diagnostic artifact unavailable"))?;
        if !metadata.is_file() || metadata.len() > FILE_LIMIT as u64 {
            return Err(invalid("private diagnostic artifact invalid"));
        }
        total = total.saturating_add(metadata.len());
    }
    let additional: usize = artifacts.iter().map(|(_, bytes)| bytes.len()).sum();
    if count + artifacts.len() > 128
        || total.saturating_add(additional as u64) > (2 * FILE_LIMIT) as u64
    {
        return Err(invalid("private diagnostic total budget exceeded"));
    }
    for (name, data) in artifacts {
        write_file(directory, name, data)?;
    }
    Ok(())
}

fn safe_failure(value: &Value) -> Value {
    let mut result = json!({"originalDetails":"WITHHELD_PRIVATE_TEXT"});
    if value["reason"] == "SERVICE_NOT_SELECTED" {
        result["reason"] = json!("SERVICE_NOT_SELECTED");
        result["remediationId"] = json!("SELECT_SERVICE_EXPLICITLY");
    } else if let Ok(code) = serde_json::from_value::<ErrorCode>(value["reason"].clone()) {
        let summary = crate::operations::support_summary(
            &json!({"schema":"codeclew-error/2.0", "error":{"code":code}}),
        )
        .unwrap_or(Value::Null);
        result["reason"] = json!(code);
        result["remediationId"] = summary["remediationId"].clone();
        result["retryable"] = summary["retryable"].clone();
    } else {
        result["reason"] = json!("UNKNOWN_FAILURE_CODE");
        result["remediationId"] = json!("INSPECT_PRIVATE_DIAGNOSTICS");
    }
    if let Some(safe) = crate::worker_diagnostics::safe_summary(&value["workerFailure"]) {
        result["workerFailure"] = safe;
    }
    if let Some(safe) = crate::maven_diagnostics::saved_summary(&value["mavenFailure"]) {
        result["mavenFailure"] = safe;
    }
    result
}

pub fn empty_report() -> Value {
    let runtime = crate::runtime::RuntimeAuthority::diagnostic_metadata().ok().flatten()
        .unwrap_or_else(|| json!({"version":env!("CARGO_PKG_VERSION"),"mode":"NOT_RECORDED","platform":std::env::consts::OS,"architecture":std::env::consts::ARCH}));
    json!({"schema":REPORT_SCHEMA, "status":"PARTIAL", "inspectionScope":"MANIFEST_METADATA",
        "heavyObjects":"NOT_VERIFIED", "currentDeclarations":"NOT_INSPECTED",
        "runtime":runtime,
        "privacy":{"source":false,"arbitraryText":false,"arguments":false,"environmentValues":false,"containsContentDigests":true,"serviceIdentities":"SHA256"},
        "snapshot":null,"manifestBytes":null,"checkStatus":null,"services":[],"issues":[]})
}

fn issue(report: &mut Value, code: &'static str) {
    report["issues"]
        .as_array_mut()
        .expect("report issue array")
        .push(json!({"code":code}));
    report["status"] = json!("PARTIAL");
}

/// Read only two metadata objects (Check and source-selection envelope). Errors
/// become typed partial results; arbitrary storage/SQL error text is withheld.
pub fn inspect(
    root: &Path,
    snapshot: Option<&str>,
    service: Option<&str>,
    private: bool,
) -> (Value, Option<Value>) {
    let mut report = empty_report();
    let mut private_details = None;
    let attempt = (|| -> Result<(), ClewError> {
        // Resolve the caller's root once; every selected child rejects symlinks.
        let root = root
            .canonicalize()
            .map_err(|_| invalid("ROOT_UNAVAILABLE"))?;
        let pinned = if let Some(selected) = snapshot {
            handle(selected)?;
            selected.to_owned()
        } else {
            let raw_latest = read(
                &path(&root, ".codeclew/cache/latest-check.json")?,
                METADATA_LIMIT,
            )?;
            format!(
                "{}/{}",
                canonical::hash_bytes(&raw_latest),
                raw_latest.len()
            )
        };
        let (digest, size) = handle(&pinned)?;
        report["snapshot"] = json!(pinned);
        report["manifestBytes"] = json!(size);
        let marker = path(&root, ".codeclew/cache/object-layout.json")?;
        let marker_signature = signature(&marker)?;
        let layout: Layout =
            serde_json::from_slice(&read(&marker, 4096)?).map_err(|_| invalid("INVALID_LAYOUT"))?;
        if layout.schema != "codeclew-documentation-object-layout/2.0"
            || layout.policy != "SQLITE_ONLY"
            || uuid::Uuid::parse_str(&layout.store_id).is_err()
            || layout.database != format!(".codeclew/cache/objects-{}.sqlite3", layout.store_id)
        {
            return Err(invalid("INVALID_LAYOUT"));
        }
        quiescent(&root, &layout.database)?;
        let database = path(&root, &layout.database)?;
        let database_signature = signature(&database)?;
        let objects = ImmutableSqliteObjects::open(&database, &layout.store_id)?;
        let raw = objects
            .read(digest, size, METADATA_LIMIT)?
            .ok_or_else(|| invalid("MANIFEST_UNAVAILABLE"))?;
        let manifest = Check::decode_manifest(&raw)?;
        if manifest.schema != super::check::CHECK_MANIFEST_SCHEMA
            || !valid_digest(&manifest.input_digest)
            || !valid_digest(&manifest.context_digest)
            || manifest.service_manifests.len() > super::store::MAX_RECORDS
            || manifest.unresolved.len() > super::store::MAX_RECORDS
        {
            return Err(invalid("MANIFEST_INVALID"));
        }
        report["status"] = json!("COMPLETE");
        report["checkStatus"] = json!(if manifest.unresolved.is_empty() {
            "CHECKED"
        } else {
            "UNRESOLVED"
        });
        report["inputDigest"] = json!(manifest.input_digest);
        report["contextDigest"] = json!(manifest.context_digest);
        let selection = (|| {
            let r = &manifest.source_inputs;
            if r.schema != source_inputs::MANIFEST_SCHEMA {
                return Err(invalid("INVALID_SELECTION"));
            }
            let data = objects
                .read(&r.digest, r.size, METADATA_LIMIT)?
                .ok_or_else(|| invalid("SELECTION_UNAVAILABLE"))?;
            source_inputs::diagnostic_selection(&data, &manifest.input_digest)
        })();
        let mut ids: BTreeSet<String> = manifest
            .service_manifests
            .keys()
            .chain(manifest.unresolved.keys())
            .cloned()
            .collect();
        if let Ok((declared, _, _)) = &selection {
            ids.extend(declared.iter().cloned());
        }
        if selection.is_err() {
            issue(&mut report, "SELECTION_METADATA_UNAVAILABLE");
        }
        if let Some(selected) = service {
            if !ids.contains(selected) {
                issue(&mut report, "SERVICE_ABSENT_FROM_SNAPSHOT");
            }
            ids.retain(|id| id == selected);
        }
        let service_records = ids.len();
        let mut services = Vec::new();
        let mut report_bytes = 0usize;
        for id in ids {
            let capture = manifest.service_manifests.get(&id);
            let failure = manifest.unresolved.get(&id);
            if capture.is_some() && failure.is_some() {
                issue(&mut report, "SERVICE_METADATA_INCONSISTENT");
            }
            let selected = selection
                .as_ref()
                .ok()
                .map(|(_, selected, _)| selected.contains(&id));
            let retained = selection
                .as_ref()
                .ok()
                .map(|(_, _, retained)| retained.contains(&id));
            let outcome = if failure.is_some_and(|v| v["reason"] == "SERVICE_NOT_SELECTED") {
                "NOT_SELECTED"
            } else if failure.is_some() {
                "FAILED"
            } else if capture.is_some() {
                "CAPTURED"
            } else {
                "UNAVAILABLE"
            };
            let mut item = json!({"serviceIdHash":identity(&id),"selected":selected,"retained":retained,"outcome":outcome});
            if let Some(capture) = capture {
                item["entrypoints"] = json!(capture.entrypoints.len());
                item["revisionHash"] = identity(&capture.revision);
                item["producerHash"] = identity(&capture.extractor);
                item["producer"] = match capture.extractor.as_str() {
                    super::model::EXTRACTOR | super::model::SOURCE_EXTRACTOR => {
                        json!(capture.extractor)
                    }
                    _ => json!("NOT_RECORDED"),
                };
                item["serviceDigest"] = if valid_digest(&capture.service_digest) {
                    json!(capture.service_digest)
                } else {
                    Value::Null
                };
                item["runtimeMode"] = match capture.runtime_mode.as_str() {
                    "RELEASE" | "DEVELOPMENT" => json!(capture.runtime_mode),
                    _ => json!("NOT_RECORDED"),
                };
            }
            if let Some(failure) = failure {
                item["failure"] = safe_failure(failure);
            }
            let item_bytes = super::bytes(&item)?.len();
            if services.len() < 256 && report_bytes + item_bytes <= FILE_LIMIT - 64 * 1024 {
                report_bytes += item_bytes;
                services.push(item);
            }
        }
        if services.len() < service_records {
            issue(&mut report, "SERVICE_REPORT_TRUNCATED");
        }
        report["serviceRecordTruncation"] = json!({"observed":service_records,"retained":services.len(),"dropped":service_records-services.len()});
        report["counts"] = json!({"captured":manifest.service_manifests.len(),"failed":manifest.unresolved.values().filter(|v| v["reason"] != "SERVICE_NOT_SELECTED").count(),
            "notSelected":manifest.unresolved.values().filter(|v| v["reason"] == "SERVICE_NOT_SELECTED").count(),"reported":services.len()});
        report["services"] = json!(services);
        report["counts"]["selected"] = selection
            .as_ref()
            .ok()
            .map(|(_, selected, _)| selected.len())
            .into();
        report["counts"]["retained"] = selection
            .as_ref()
            .ok()
            .map(|(_, _, retained)| retained.len())
            .into();
        if private {
            let failures: std::collections::BTreeMap<_, _> = manifest
                .unresolved
                .into_iter()
                .filter(|(id, _)| service.is_none_or(|selected| id == selected))
                .collect();
            private_details =
                Some(json!({"schema":"codeclew-private-saved-details/1.0","unresolved":failures}));
        }
        quiescent(&root, &layout.database)?;
        if signature(&marker)? != marker_signature || signature(&database)? != database_signature {
            return Err(invalid("STORE_CHANGED"));
        }
        Ok(())
    })();
    if let Err(error) = attempt {
        let code = match error.message.as_str() {
            "ROOT_UNAVAILABLE" => "ROOT_UNAVAILABLE",
            "ACTIVE_STORE" => "ACTIVE_STORE",
            "STORE_CHANGED" => "STORE_CHANGED",
            "INVALID_LAYOUT" => "INVALID_LAYOUT",
            "MANIFEST_UNAVAILABLE" => "MANIFEST_UNAVAILABLE",
            "MANIFEST_INVALID" => "MANIFEST_INVALID",
            _ => "METADATA_UNAVAILABLE_OR_INVALID",
        };
        // Never publish an apparently complete selection assembled across a race.
        if matches!(code, "STORE_CHANGED" | "ACTIVE_STORE") {
            report["services"] = json!([]);
            private_details = None;
        }
        issue(&mut report, code);
    }
    if report["runtime"]["mode"] == "NOT_RECORDED" {
        issue(&mut report, "RUNTIME_METADATA_UNAVAILABLE");
    }
    (report, private_details)
}

pub fn fresh_directory(output: &Path) -> Result<(), ClewError> {
    // Resolve the existing parent; the newly created leaf never follows a symlink.
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| invalid("diagnostic output parent is unavailable"))?;
    let name = output
        .file_name()
        .ok_or_else(|| invalid("diagnostic output must name a new directory"))?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(canonical_parent.join(name))
        .map_err(|_| invalid("diagnostic output must be a new directory"))?;
    Ok(())
}

pub fn write_file(output: &Path, name: &str, data: &[u8]) -> Result<Value, ClewError> {
    if data.len() > FILE_LIMIT {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "diagnostic artifact exceeds its budget",
        ));
    }
    super::store::relative(name)?;
    let metadata =
        fs::symlink_metadata(output).map_err(|_| invalid("diagnostic output is unavailable"))?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(invalid("diagnostic output must be caller-owned mode 0700"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(output.join(name))
        .map_err(|_| invalid("diagnostic artifact could not be created"))?;
    file.write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|_| invalid("diagnostic artifact could not be saved"))?;
    Ok(json!({"path":name,"bytes":data.len(),"digest":canonical::hash_bytes(data)}))
}

pub fn write_private_details(output: &Path, details: &Value) -> Result<Value, ClewError> {
    let raw = super::bytes(details)?;
    let retained = raw.len().min(FILE_LIMIT / 8);
    let payload = json!({"schema":"codeclew-private-saved-details-tail/1.0","encoding":"UTF8_LOSSY_TAIL","observedBytes":raw.len(),"retainedBytes":retained,
        "droppedBytes":raw.len()-retained,"tail":String::from_utf8_lossy(&raw[raw.len()-retained..])});
    let artifact = write_file(
        output,
        "PRIVATE_SAVED_DETAILS.json",
        &super::bytes(&payload)?,
    )?;
    Ok(
        json!({"artifact":artifact,"observedBytes":raw.len(),"retainedBytes":retained,"droppedBytes":raw.len()-retained}),
    )
}

/// A hook for the one normal invocation. Only a Check handle returned by that
/// invocation is inspected; failures before save never follow an older latest.
pub fn invocation_report(
    root: Option<&Path>,
    selected: &[String],
    result: &Result<Value, ClewError>,
) -> Result<(), ClewError> {
    let Some(directory) = std::env::var_os("CLEW_DIAGNOSTIC_REPORT_DIR") else {
        return Ok(());
    };
    let directory = PathBuf::from(directory);
    let private = std::env::var_os("CLEW_DIAGNOSTIC_PRIVATE").is_some_and(|v| v == "1");
    let mut details = None;
    let mut report = match (root, result) {
        (Some(root), Ok(value)) if value["snapshot"].as_str().is_some() => {
            let (report, saved) = inspect(root, value["snapshot"].as_str(), None, private);
            details = saved.map(|mut details| {
                if !selected.is_empty()
                    && let Some(failures) = details["unresolved"].as_object_mut()
                {
                    failures.retain(|id, _| selected.contains(id));
                }
                details
            });
            report
        }
        _ => empty_report(),
    };
    report["sourceStage"] = json!("CORE");
    report["commandResult"] = match result {
        Ok(value) => {
            if root.is_some() {
                json!({"kind":"DOCUMENTATION_CHECK","freshness":match value["freshness"]["status"].as_str() {
                    Some("CURRENT") => "CURRENT", Some("PARTIALLY_STALE") => "PARTIALLY_STALE", Some("STALE") => "STALE", _ => "UNRESOLVED"}})
            } else {
                json!({"kind":"COMPLETED"})
            }
        }
        Err(error) => {
            if let Some(snapshot) = error.snapshot_id.as_deref().filter(|s| handle(s).is_ok()) {
                report["snapshot"] = json!(snapshot);
                report["manifestBytes"] = json!(handle(snapshot)?.1);
                report["snapshotSource"] = json!("ERROR_CONTEXT_NOT_RECOLLECTED");
            }
            for evidence in &error.evidence {
                if let Some(record) = evidence
                    .strip_prefix("documentation-retained-service-failure:")
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    && record["schema"] == "codeclew-documentation-retained-service-failure/1.0"
                {
                    report["recordedFailure"] = safe_failure(&record["recordedFailure"]);
                }
            }
            let summary = crate::operations::support_summary(
                &json!({"schema":"codeclew-error/2.0","error":error}),
            )?;
            if private {
                details = Some(json!({"schema":"codeclew-private-error/1.0","error":error}));
            }
            summary
        }
    };
    if report["snapshot"].is_null() {
        issue(&mut report, "NO_CHECK_SAVED_BY_THIS_INVOCATION");
    }
    if let Some(details) = details {
        report["privateTruncation"] = write_private_details(&directory, &details)?;
    }
    write_file(&directory, "core-report.json", &super::bytes(&report)?)?;
    Ok(())
}

pub fn collect(
    root: &Path,
    snapshot: Option<&str>,
    service: Option<&str>,
    output: &Path,
    private: bool,
) -> Result<Value, ClewError> {
    // Refuse writes anywhere beneath the inspected root, even through a symlink.
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if let (Ok(root), Ok(parent)) = (root.canonicalize(), parent.canonicalize())
        && parent.starts_with(root)
    {
        return Err(invalid(
            "diagnostic output must be outside the documentation root",
        ));
    }
    if let Some(snapshot) = snapshot {
        handle(snapshot)?;
    }
    fresh_directory(output)?;
    let (report, private_details) = inspect(root, snapshot, service, private);
    let mut artifacts = vec![
        write_file(output, "report.json", &super::bytes(&report)?)?,
        write_file(output, "README.md", README.as_bytes())?,
    ];
    let mut truncation = Value::Null;
    if let Some(details) = private_details {
        truncation = write_private_details(output, &details)?;
        artifacts.push(truncation["artifact"].clone());
    }
    let manifest = json!({"schema":BUNDLE_SCHEMA,"status":report["status"],"sharing":if private {"PRIVATE_REVIEW_REQUIRED"} else {"ALLOWLISTED_METADATA"},
        "inspectionScope":"MANIFEST_METADATA","artifacts":artifacts,"limits":{"fileBytes":FILE_LIMIT,"totalBytes":4*FILE_LIMIT},"privateTruncation":truncation});
    write_file(output, "manifest.pending", &super::bytes(&manifest)?)?;
    fs::rename(
        output.join("manifest.pending"),
        output.join("manifest.json"),
    )
    .map_err(|_| invalid("diagnostic manifest could not be finalized"))?;
    File::open(output)
        .and_then(|file| file.sync_all())
        .map_err(|_| invalid("diagnostic directory could not be finalized"))?;
    Ok(
        json!({"schema":"codeclew-support-collect/1.0","status":report["status"],"snapshot":report["snapshot"],"manifestBytes":report["manifestBytes"],"sharing":manifest["sharing"]}),
    )
}
