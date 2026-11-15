//! Bounded immutable map parts over the existing verified SQLite object store.
//! Small maps retain the 0.15.6 wire format. Large maps never serialize a second
//! whole-map copy: only one record and one part are encoded at a time. Hydration
//! still owns the complete logical map required by ServiceEvidence.
use super::{cache, check, io_error, store::Repository};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;

pub(super) const INDEX_SCHEMA: &str = "codeclew-documentation-capture-map-index/1.0";
const TARGET: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Part {
    first: String,
    last: String,
    records: usize,
    object: cache::ObjectRef,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Index {
    schema: String,
    map_schema: String,
    records: usize,
    parts: Vec<Part>,
}

fn corrupt() -> ClewError {
    ClewError::new(
        ErrorCode::StateCorrupt,
        "capture map closure is missing, corrupt or inconsistent",
    )
}

pub(super) fn accepts(reference: &cache::ObjectRef, schema: &str) -> bool {
    reference.schema == schema || reference.schema == INDEX_SCHEMA
}

fn index(raw: &[u8], schema: &str) -> Result<Index, ClewError> {
    let index: Index = serde_json::from_slice(raw).map_err(|_| corrupt())?;
    if index.schema != INDEX_SCHEMA || index.map_schema != schema || index.parts.len() < 2 {
        return Err(corrupt());
    }
    let mut previous = None;
    let mut total = 0usize;
    for part in &index.parts {
        if part.records == 0
            || part.first > part.last
            || part.object.schema != schema
            || previous.is_some_and(|last| last >= &part.first)
        {
            return Err(corrupt());
        }
        total = total.checked_add(part.records).ok_or_else(corrupt)?;
        previous = Some(&part.last);
    }
    if total != index.records {
        return Err(corrupt());
    }
    Ok(index)
}

/// Explicit closure edges for recovery from a stopped SQLite export. Payload
/// maps are validated by the ordinary loader before the staged capture is used.
pub(super) fn part_references(
    reference: &cache::ObjectRef,
    raw: &[u8],
    schema: &str,
) -> Result<Vec<cache::ObjectRef>, ClewError> {
    if !accepts(reference, schema) {
        return Err(corrupt());
    }
    if reference.schema == schema {
        return Ok(Vec::new());
    }
    Ok(index(raw, schema)?
        .parts
        .into_iter()
        .map(|p| p.object)
        .collect())
}

pub(super) fn store<T: Serialize>(
    repo: &Repository,
    schema: &str,
    values: &BTreeMap<String, T>,
) -> Result<cache::ObjectRef, ClewError> {
    let mut parts = Vec::new();
    let mut payload = vec![b'{'];
    let mut first = String::new();
    let mut last = String::new();
    let mut count = 0;
    let flush = |payload: &mut Vec<u8>, first: &str, last: &str, count, parts: &mut Vec<Part>| {
        payload.push(b'}');
        let object = cache::put(repo, schema, payload)?;
        parts.push(Part {
            first: first.into(),
            last: last.into(),
            records: count,
            object,
        });
        payload.clear();
        payload.push(b'{');
        Ok::<_, ClewError>(())
    };
    for (key, value) in values {
        let encoded_key = serde_json::to_vec(key).map_err(io_error)?;
        let encoded_value = crate::canonical::bytes(value).map_err(io_error)?;
        let record_bytes = encoded_key.len() + 1 + encoded_value.len();
        // Include the minimal object envelope. A single record cannot be split
        // without changing its meaning; never raise the physical record cap.
        if record_bytes as u64 + 2 > check::PORTABLE_CACHE_MAX_BYTES {
            return Err(cache::budget_error(
                schema,
                "SERIALIZE_CAPTURE_MAP_RECORD",
                record_bytes as u64 + 2,
            ));
        }
        if count > 0 && payload.len() + 1 + record_bytes + 1 > TARGET {
            flush(&mut payload, &first, &last, count, &mut parts)?;
            count = 0;
        }
        if count == 0 {
            first = key.clone();
        } else {
            payload.push(b',');
        }
        last = key.clone();
        payload.extend_from_slice(&encoded_key);
        payload.push(b':');
        payload.extend_from_slice(&encoded_value);
        count += 1;
    }
    if count > 0 || parts.is_empty() {
        flush(&mut payload, &first, &last, count, &mut parts)?;
    }
    if parts.len() == 1 {
        return Ok(parts.remove(0).object);
    }
    cache::put_json(
        repo,
        INDEX_SCHEMA,
        &Index {
            schema: INDEX_SCHEMA.into(),
            map_schema: schema.into(),
            records: values.len(),
            parts,
        },
    )
}

pub(super) fn load<T: DeserializeOwned>(
    repo: &Repository,
    reference: &cache::ObjectRef,
    schema: &str,
) -> Result<BTreeMap<String, T>, ClewError> {
    if !accepts(reference, schema) {
        return Err(corrupt());
    }
    cache::with_read_session(repo, |session| {
        let read = |reference: &cache::ObjectRef| {
            session
                .get(reference, check::PORTABLE_CACHE_MAX_BYTES)?
                .ok_or_else(corrupt)
        };
        let raw = read(reference)?;
        if reference.schema == schema {
            return serde_json::from_slice(&raw).map_err(|_| corrupt());
        }
        let index = index(&raw, schema)?;
        drop(raw);
        let mut result = BTreeMap::new();
        for part in index.parts {
            let raw = read(&part.object)?;
            let values: BTreeMap<String, T> =
                serde_json::from_slice(&raw).map_err(|_| corrupt())?;
            drop(raw);
            if values.len() != part.records
                || values.first_key_value().map(|(k, _)| k) != Some(&part.first)
                || values.last_key_value().map(|(k, _)| k) != Some(&part.last)
            {
                return Err(corrupt());
            }
            result.extend(values);
        }
        Ok(result)
    })
}

#[cfg(test)]
#[path = "capture_maps_tests.rs"]
mod tests;
