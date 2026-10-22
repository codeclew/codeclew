//! Read-only selected publication admission. Hash files as streams and hydrate
//! only the selected immutable bindings closure; never scan a cache directory.
use crate::{
    documentation::{bindings_storage, bytes, history, invalid, io_error, store::Repository},
    error::{ClewError, ErrorCode},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Read};

pub(super) const FILE_BYTES: u64 = 64 * 1024 * 1024;
const WORK_BYTES: u64 = 512 * 1024 * 1024;

pub(super) struct Input {
    pub original: history::Publication,
    pub selected: history::Publication,
    pub bindings: Vec<u8>,
    pub accounting: Value,
}

pub(super) fn subject(name: &str) -> Option<String> {
    let (kind, path) = if let Some(path) = name.strip_prefix("services/") {
        ("service", path)
    } else {
        ("scenario", name.strip_prefix("scenarios/")?)
    };
    let id = path.split('.').next()?;
    crate::documentation::store::valid_id(id).then(|| format!("{kind}:{id}"))
}

fn charge(total: &mut u64, size: u64) -> Result<(), ClewError> {
    *total = total
        .checked_add(size)
        .ok_or_else(|| invalid("export input size overflow"))?;
    if *total > WORK_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "remote publication verification exceeds 512 MiB work budget; select fewer export-subjects",
        ));
    }
    Ok(())
}

/// Every selected original byte is verified, with fixed-size streaming buffers.
/// Each later consumer still checks its read against the immutable hash.
fn verify(repo: &Repository, id: &str, name: &str, expected: &str) -> Result<u64, ClewError> {
    let path = repo.path(&format!("docs/generated/{id}/{name}"))?;
    let meta = fs::symlink_metadata(&path).map_err(io_error)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(invalid("remote publication input is not a regular file"));
    }
    if meta.len() > FILE_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            format!("remote publication record {name} exceeds 64 MiB individual bound"),
        ));
    }
    let mut file = fs::File::open(path).map_err(io_error)?.take(FILE_BYTES + 1);
    let mut buffer = [0u8; 65536];
    let mut hash = Sha256::new();
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hash.update(&buffer[..n]);
    }
    if size != meta.len()
        || size > FILE_BYTES
        || format!("sha256:{:x}", hash.finalize()) != expected
    {
        return Err(invalid(
            "selected frozen publication is damaged or changed during export",
        ));
    }
    Ok(size)
}

pub(super) fn read(repo: &Repository, id: &str, requested: &[String]) -> Result<Input, ClewError> {
    let manifest_path = repo.path(&format!("docs/generated/{id}/publication.json"))?;
    let meta = fs::symlink_metadata(&manifest_path).map_err(io_error)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > FILE_BYTES {
        return Err(invalid(
            "remote publication manifest is unsafe or exceeds 64 MiB",
        ));
    }
    let original = history::load(repo, id)?;
    let known: BTreeSet<_> = original
        .files
        .keys()
        .filter(|n| n.ends_with(".json"))
        .filter_map(|n| subject(n))
        .collect();
    let requested: BTreeSet<_> = requested.iter().cloned().collect();
    if !requested.is_subset(&known) {
        return Err(invalid(
            "export-subject is not present in the selected saved publication",
        ));
    }
    let effective = if requested.is_empty() {
        known.clone()
    } else {
        requested.clone()
    };
    let mut selected = original.clone();
    // Shared overview/status/answer artifacts remain global retained context.
    // Service/process pages and their named diagrams are explicitly selected.
    let mut selected_names = BTreeSet::new();
    for name in original.files.keys() {
        let include = if requested.is_empty() {
            true
        } else if let Some(subject) = subject(name) {
            effective.contains(&subject)
        } else if let Some(diagram) = name.strip_prefix("diagrams/") {
            let owners: Vec<_> = known
                .iter()
                .filter(|subject| {
                    let stem = subject.replace(':', "-");
                    diagram.starts_with(&(stem + "-"))
                })
                .collect();
            let retained = owners.iter().filter(|s| effective.contains(**s)).count();
            if retained > 0 && retained != owners.len() {
                return Err(invalid(
                    "diagram subject prefixes are ambiguous; select all involved subjects or export the full publication",
                ));
            }
            owners.is_empty() || retained > 0
        } else {
            true
        };
        if include {
            selected_names.insert(name.clone());
        }
    }
    selected
        .files
        .retain(|name, _| selected_names.contains(name));
    let mut total = 0;
    charge(&mut total, meta.len().saturating_mul(2))?;
    let mut file_bytes = 0;
    for (name, expected) in &selected.files {
        let size = fs::symlink_metadata(repo.path(&format!("docs/generated/{id}/{name}"))?)
            .map_err(io_error)?
            .len();
        charge(&mut total, size.saturating_mul(2))?;
        if verify(repo, id, name, expected)? != size {
            return Err(invalid("selected file changed during verification"));
        }
        file_bytes += size;
    }
    let path = repo.path(&format!("docs/generated/{id}/bindings.json"))?;
    let binding_size = fs::symlink_metadata(&path).map_err(io_error)?.len();
    if binding_size > FILE_BYTES {
        return Err(invalid("frozen bindings exceed individual file bound"));
    }
    charge(&mut total, binding_size)?;
    let mut raw = Vec::new();
    fs::File::open(path)
        .map_err(io_error)?
        .take(FILE_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(io_error)?;
    if raw.len() as u64 > FILE_BYTES
        || selected.files.get("bindings.json") != Some(&crate::canonical::hash_bytes(&raw))
    {
        return Err(invalid(
            "frozen bindings changed after streaming verification",
        ));
    }
    if raw.len() as u64 != binding_size {
        return Err(invalid("frozen bindings size changed during read"));
    }
    let partitioned = bindings_storage::logical_size(&raw)?.is_some();
    let (logical, stats) =
        bindings_storage::resolve_subjects(repo, &raw, &requested, WORK_BYTES - total)?;
    charge(&mut total, stats.bytes)?;
    let binding = bytes(&logical)?;
    let omitted: Vec<_> = known.difference(&effective).cloned().collect();
    Ok(Input {
        original,
        selected,
        bindings: binding,
        accounting: json!({
            "requestedSubjects":requested,"effectiveSubjects":effective,"omittedSubjects":omitted,
            "selection":"SAVED_PUBLICATION_SUBJECTS_WITH_SHARED_CONTEXT",
            "partitionedBindings":partitioned,"legacyFallback":!partitioned,
            "verifiedFileBytes":file_bytes,"reservedRenderBytes":file_bytes + meta.len(),"retainedReadingInputLimitBytes":crate::documentation::check::PORTABLE_CACHE_MAX_BYTES,"bindingObjectsRead":stats.objects,
            "bindingPhysicalBytesRead":stats.bytes,"bindingLogicalBytes":stats.logical_bytes,
            "admissionBytes":total,"workLimitBytes":WORK_BYTES,"individualFileLimitBytes":FILE_BYTES,
            "bindingLimitBytes":crate::documentation::check::PORTABLE_CACHE_MAX_BYTES,
            "unrelatedCacheScanned":false,"snapshotAdmission":"EXACT_RETAINED_CHECK_BEFORE_PUBLICATION_SELECTION","originalEvidence":"UNCHANGED"
        }),
    })
}

/// Omitted page links are inert, while immutable external source URLs retain
/// their exact revision/line fragment. Authored prose itself is not rewritten.
pub(super) fn annotate(html: &str, name: &str, subjects: &[String], omitted: &Value) -> String {
    let omitted: BTreeSet<_> = omitted
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|s| s.split_once(':'))
        .map(|(kind, id)| {
            format!(
                "{}/{id}.html",
                if kind == "service" {
                    "services"
                } else {
                    "scenarios"
                }
            )
        })
        .collect();
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("href=\"") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 6..];
        let Some(end) = rest.find('"') else {
            out.push_str("href=\"");
            out.push_str(rest);
            rest = "";
            break;
        };
        let href = &rest[..end];
        let mut route = href.split(['#', '?']).next().unwrap_or(href);
        while let Some(tail) = route.strip_prefix("../") {
            route = tail;
        }
        let relative = if !route.contains('/') {
            name.rsplit_once('/')
                .map(|(folder, _)| format!("{folder}/{route}"))
        } else {
            None
        };
        if omitted.contains(route) || relative.as_ref().is_some_and(|r| omitted.contains(r)) {
            out.push_str(
                "aria-disabled=\"true\" title=\"Outside selected export\" data-omitted-target=\"",
            );
        } else {
            out.push_str("href=\"");
        }
        out.push_str(href);
        out.push('"');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    let notice = format!(
        "<aside class=\"export-selection\"><strong>Selected saved publication</strong><p>Exported subjects: {}. Other service/process pages are omitted; shared overview and answer context is retained.</p></aside>",
        crate::documentation::render::escape(&subjects.join(", "))
    );
    if let Some(start) = out.find("<main").or_else(|| out.find("<body"))
        && let Some(end) = out[start..].find('>')
    {
        out.insert_str(start + end + 1, &notice);
    }
    out
}
