//! Private, bounded failure records. References are identities, never paths.
use crate::{canonical, error::ClewError, state::StateAuthority};
use serde_json::{Value, json};
use std::{ffi::OsStr, os::fd::AsRawFd, path::Path};

pub(crate) const LIMIT: usize = 96 * 1024;
const DIRECTORY: &str = "attempts/compiler-failures";
const SCHEMA: &str = "codeclew-private-compiler-failure/1.0";

fn invalid() -> ClewError {
    ClewError::new(
        crate::error::ErrorCode::InvalidInput,
        "private compiler failure unavailable or invalid",
    )
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn capture(facts: &Value) -> Value {
    let Ok(state) = StateAuthority::process_default() else {
        return json!({"status":"UNAVAILABLE"});
    };
    capture_in(&state, facts).unwrap_or_else(|_| json!({"status":"UNAVAILABLE"}))
}

pub(crate) fn capture_in(state: &StateAuthority, facts: &Value) -> Result<Value, ClewError> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut diagnostics = Vec::new();
    let rows = facts["diagnostics"].as_array();
    let mut size = 0;
    for row in rows.into_iter().flatten() {
        let bytes = canonical::bytes(row).map_err(|_| invalid())?;
        if diagnostics.len() == 128 || size + bytes.len() > 64 * 1024 {
            break;
        }
        size += bytes.len();
        diagnostics.push(row.clone());
    }
    let context = &facts["privateCompilationContext"];
    let context_available =
        context.is_object() && canonical::bytes(context).map_err(|_| invalid())?.len() <= 24 * 1024;
    let body = json!({"schema":SCHEMA,"id":id,
        "payloadHash":canonical::hash(facts).map_err(|_| invalid())?,
        "projectModelHash":facts["projectModelHash"],
        "semanticInputManifestHash":facts["semanticInputManifestHash"],
        "compilation":facts["compilation"],"stage":"K2_VALIDATION",
        "observedCount":rows.map_or(0,Vec::len),"retainedCount":diagnostics.len(),
        "truncated":diagnostics.len() < rows.map_or(0,Vec::len),"diagnostics":diagnostics,
        "contextStatus":if context_available {"AVAILABLE"} else {"UNAVAILABLE_OR_OVERSIZED"},
        "compilationContext":if context_available {context.clone()} else {Value::Null}});
    let bytes = canonical::bytes(&body).map_err(|_| invalid())?;
    if bytes.len() > LIMIT {
        return Err(invalid());
    }
    let directory = state.directory(Path::new(DIRECTORY))?;
    let lock = directory.open_lock(OsStr::new("lock"))?;
    // Dropping this file releases the process lock, including every error path.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(invalid());
    }
    let mut records = Vec::new();
    for name in directory.entries()? {
        if name == "lock" {
            continue;
        }
        let id = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| valid_id(id))
            .ok_or_else(invalid)?;
        let file = directory.open_file(OsStr::new(&format!("{id}.json")))?;
        let metadata = file.metadata().map_err(|_| invalid())?;
        if metadata.len() > LIMIT as u64 || records.len() >= 32 {
            return Err(invalid());
        }
        records.push((metadata.modified().map_err(|_| invalid())?, name));
    }
    if records.len() == 32 {
        records.sort();
        directory.remove_file(&records[0].1)?;
    }
    if !directory.atomic_create(OsStr::new(&format!("{id}.json")), &bytes)? {
        return Err(invalid());
    }
    Ok(
        json!({"status":"CAPTURED_PRIVATE","id":id,"digest":canonical::hash_bytes(&bytes),
        "payloadHash":body["payloadHash"],"bytes":bytes.len()}),
    )
}

pub(crate) fn read(state: &StateAuthority, reference: &Value) -> Result<Value, ClewError> {
    let id = reference["id"]
        .as_str()
        .filter(|id| valid_id(id))
        .ok_or_else(invalid)?;
    if reference["status"] != "CAPTURED_PRIVATE" {
        return Err(invalid());
    }
    let bytes = state
        .directory(Path::new(DIRECTORY))?
        .read_file(OsStr::new(&format!("{id}.json")), LIMIT)?;
    if reference["digest"] != canonical::hash_bytes(&bytes) || reference["bytes"] != bytes.len() {
        return Err(invalid());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if value["schema"] != SCHEMA
        || value["id"] != id
        || value["stage"] != "K2_VALIDATION"
        || value["payloadHash"] != reference["payloadHash"]
    {
        return Err(invalid());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn facts() -> Value {
        json!({"compilation":":/main","projectModelHash":"model","semanticInputManifestHash":"inputs",
            "diagnostics":[{"severity":"ERROR","message":"PRIVATE_UNRESOLVED_REFERENCE","file":"/private/Probe.kt"}],
            "privateCompilationContext":{"projectJdkHome":"/private/jdk25","analyzerJdkHome":"/private/jdk21",
                "projectCompilerVersion":"2.3.21","analyzerCompilerVersion":"2.4.10","jvmTarget":"JVM_25",
                "freeCompilerArguments":["-Xjvm-enable-preview","-Xconsistent-data-class-copy-visibility"]}})
    }

    #[test]
    fn retains_exact_failure_privately_with_context_and_refuses_modified_references() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("state")).unwrap();
        let reference = capture_in(&state, &facts()).unwrap();
        assert!(
            !reference
                .to_string()
                .contains("PRIVATE_UNRESOLVED_REFERENCE")
        );
        assert!(!reference.to_string().contains("/private/"));
        let record = read(&state, &reference).unwrap();
        assert_eq!(
            record["diagnostics"][0]["message"],
            "PRIVATE_UNRESOLVED_REFERENCE"
        );
        assert_eq!(record["compilationContext"]["jvmTarget"], "JVM_25");
        let path = state
            .root()
            .join(DIRECTORY)
            .join(format!("{}.json", reference["id"].as_str().unwrap()));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for (key, value) in [
            ("id", json!("../outside")),
            ("digest", json!("wrong")),
            ("payloadHash", json!("another failure")),
        ] {
            let mut changed = reference.clone();
            changed[key] = value;
            assert!(read(&state, &changed).is_err());
        }
        std::fs::remove_file(&path).unwrap();
        let outside = temp.path().join("outside");
        std::fs::write(&outside, "PRIVATE_OUTSIDE").unwrap();
        symlink(&outside, &path).unwrap();
        assert!(read(&state, &reference).is_err());
    }

    #[test]
    fn bounds_rows_context_and_retention_without_substituting_evicted_failures() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("state")).unwrap();
        let mut input = facts();
        input["diagnostics"] = json!(vec![json!({"message":"x".repeat(1000)}); 200]);
        input["privateCompilationContext"]["oversized"] = json!("x".repeat(25 * 1024));
        let first = capture_in(&state, &input).unwrap();
        let record = read(&state, &first).unwrap();
        assert_eq!(record["truncated"], true);
        assert_eq!(record["contextStatus"], "UNAVAILABLE_OR_OVERSIZED");
        assert!(record["compilationContext"].is_null());
        assert!(first["bytes"].as_u64().unwrap() <= LIMIT as u64);
        for _ in 0..32 {
            capture_in(&state, &facts()).unwrap();
        }
        assert!(read(&state, &first).is_err());
        assert_eq!(
            state
                .directory(Path::new(DIRECTORY))
                .unwrap()
                .entries()
                .unwrap()
                .len(),
            33
        );
    }
}
