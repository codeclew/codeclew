//! Portable bindings are a projection of immutable evidence, not a second cache.
//! All consumers resolve the same explicit closure before semantic validation.
use super::{bytes, cache::ObjectRef, digest, invalid, io_error, store::Repository};
use crate::{
    canonical,
    error::{ClewError, ErrorCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
};

#[path = "bindings_storage_json.rs"]
mod json;

pub(super) const SCHEMA: &str = "codeclew-documentation-bindings/1.6";
pub(super) const READING_SCHEMA: &str = "codeclew-reading-bindings/1.0";
pub(super) const OBJECT_ROOT: &str = "docs/evidence/objects";
const LEAF: &str = "codeclew-bindings-record-chunk/1.0";
const PARTITION: &str = "codeclew-bindings-partition/1.0";
const INDEX_MAX: u64 = 1024 * 1024;
const TARGET: usize = 1024 * 1024;
const MAX: u64 = super::check::PORTABLE_CACHE_MAX_BYTES;

fn budget(message: &str) -> ClewError {
    ClewError::new(
        ErrorCode::SliceBudgetExceeded,
        format!("DOCS_BINDINGS_BUDGET: {message}"),
    )
}
fn corrupt(message: &str) -> ClewError {
    invalid(format!(
        "DOCS_BINDINGS_CLOSURE_INVALID: {message}; restore the complete publication and its declared portable object closure"
    ))
}
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Owner {
    kind: String,
    id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Link {
    chunk: ObjectRef,
    record: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceLink {
    payload: Link,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<Link>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Member {
    table: String,
    key: String,
    payload: Link,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_text: Option<Link>,
    #[serde(default, skip_serializing_if = "is_false")]
    retained: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fragment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Index {
    schema: String,
    owner: Owner,
    members: Vec<Member>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    kind: String,
    value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Chunk {
    schema: String,
    records: BTreeMap<String, Record>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Partition {
    owner: Owner,
    index: ObjectRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    object_root: String,
    logical_digest: String,
    logical_bytes: u64,
    header: Map<String, Value>,
    tables: Vec<String>,
    partitions: Vec<Partition>,
    objects: Vec<ObjectRef>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReadStats {
    pub objects: usize,
    pub bytes: u64,
    pub logical_bytes: u64,
}

pub(super) struct Encoded {
    pub manifest: Vec<u8>,
    pub new_objects: BTreeMap<String, Vec<u8>>,
    references: Vec<ObjectRef>,
}

fn hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}
pub(super) fn object_path(reference: &ObjectRef) -> Result<String, ClewError> {
    if !hash(&reference.digest) || !matches!(reference.schema.as_str(), LEAF | PARTITION) {
        return Err(corrupt("invalid typed object reference"));
    }
    let limit = if reference.schema == PARTITION {
        INDEX_MAX
    } else {
        MAX
    };
    if reference.size > limit {
        return Err(budget("portable object exceeds its individual bound"));
    }
    Ok(format!("{OBJECT_ROOT}/{}.json", &reference.digest[7..]))
}

fn parse<T: serde::de::DeserializeOwned>(raw: &[u8]) -> Result<T, ClewError> {
    json::preflight(raw)?;
    let value = canonical::parse_json_strict(raw)
        .map_err(|_| corrupt("malformed or duplicate JSON members"))?;
    serde_json::from_value(value).map_err(|_| corrupt("unsupported typed object schema"))
}

fn manifest(raw: &[u8], reading: bool) -> Result<Option<Manifest>, ClewError> {
    if raw.len() as u64 > MAX {
        return Err(budget("bindings record exceeds 128 MiB"));
    }
    #[derive(Deserialize)]
    struct SchemaProbe {
        #[serde(default)]
        schema: Option<String>,
    }
    let Ok(probe) = serde_yaml_ng::from_slice::<SchemaProbe>(raw) else {
        return Ok(None);
    };
    let schema = probe.schema.as_deref().unwrap_or("");
    if schema == READING_SCHEMA && !reading {
        return Err(invalid(
            "DOCS_BINDINGS_READING_ONLY: remote reading bindings cannot be admitted as evidence; use the original publication with its complete embedded closure and required authoring snapshots",
        ));
    }
    if schema != SCHEMA && schema != READING_SCHEMA {
        return Ok(None);
    }
    if raw.len() as u64 > INDEX_MAX {
        return Err(budget("bindings manifest exceeds 1 MiB"));
    }
    let m: Manifest = parse(raw)?;
    if m.objects.len() > 8192
        || m.partitions.len() > 8192
        || m.tables.len() > 32
        || m.header.len() > 32
    {
        return Err(budget("manifest membership exceeds its bound"));
    }
    if m.object_root != OBJECT_ROOT
        || !hash(&m.logical_digest)
        || !matches!(
            m.header.get("schema").and_then(Value::as_str),
            Some("codeclew-documentation-bindings/1.4" | "codeclew-documentation-bindings/1.5")
        )
    {
        return Err(corrupt(
            "unsupported portable bindings header or object root",
        ));
    }
    if m.logical_bytes > MAX {
        return Err(budget("compact logical bindings exceed 128 MiB"));
    }
    let mut refs = BTreeMap::new();
    let mut total = raw.len() as u64;
    for reference in &m.objects {
        object_path(reference)?;
        if refs.insert(&reference.digest, reference).is_some() {
            return Err(corrupt("duplicate or conflicting closure reference"));
        }
        total = total
            .checked_add(reference.size)
            .ok_or_else(|| budget("closure size overflow"))?;
        if total > MAX {
            return Err(budget("complete portable bindings closure exceeds 128 MiB"));
        }
    }
    let mut tables = BTreeSet::new();
    for table in &m.tables {
        if table.is_empty() || m.header.contains_key(table) || !tables.insert(table) {
            return Err(corrupt("conflicting table membership"));
        }
    }
    for partition in &m.partitions {
        if partition.index.schema != PARTITION
            || refs.get(&partition.index.digest).copied() != Some(&partition.index)
        {
            return Err(corrupt("partition is absent from its declared closure"));
        }
        if !matches!(
            partition.owner.kind.as_str(),
            "service" | "subject" | "shared"
        ) || partition.owner.id.is_empty()
        {
            return Err(corrupt("invalid partition owner"));
        }
    }
    Ok(Some(m))
}

pub(super) fn closure(raw: &[u8]) -> Result<Vec<ObjectRef>, ClewError> {
    Ok(manifest(raw, false)?.map(|m| m.objects).unwrap_or_default())
}

fn read_object(repo: &Repository, reference: &ObjectRef) -> Result<Vec<u8>, ClewError> {
    let path = repo.path(&object_path(reference)?)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| corrupt("portable object is missing"))?;
    let limit = if reference.schema == PARTITION {
        INDEX_MAX
    } else {
        MAX
    };
    if metadata.len() > limit {
        return Err(budget("portable object file exceeds its individual bound"));
    }
    if !metadata.is_file() || metadata.len() != reference.size {
        return Err(corrupt("portable object size or file type differs"));
    }
    let mut raw = Vec::new();
    fs::File::open(path)
        .map_err(io_error)?
        .take(reference.size + 1)
        .read_to_end(&mut raw)
        .map_err(io_error)?;
    if raw.len() as u64 != reference.size || canonical::hash_bytes(&raw) != reference.digest {
        return Err(corrupt("portable object digest or size differs"));
    }
    Ok(raw)
}

fn owner(table: &str, key: &str, value: &Value) -> Owner {
    let subject = match table {
        "fragments" => value["subject"].as_str(),
        "narratives" => Some(key),
        "acceptedVersions" | "sectionStates" => key.split('/').next(),
        _ => None,
    };
    if let Some(id) = subject {
        return Owner {
            kind: "subject".into(),
            id: id.into(),
        };
    }
    let service = match table {
        "observations" | "retainedSources" => value["service"].as_str().filter(|s| !s.is_empty()),
        "revisions" | "coverage" | "catalogues" | "targetRevisions" | "updateFailures" => Some(key),
        _ => None,
    };
    service
        .map(|id| Owner {
            kind: "service".into(),
            id: id.into(),
        })
        .unwrap_or_else(|| Owner {
            kind: "shared".into(),
            id: table.into(),
        })
}

fn record_kind(table: &str) -> &str {
    match table {
        "observations" | "fragmentObservations" => "observation",
        "retainedSources" | "fragmentSources" => "source",
        other => other,
    }
}

struct PendingSource {
    payload: String,
    text: Option<String>,
}
struct Pending {
    owner: Owner,
    table: String,
    key: String,
    payload: String,
    source_text: Option<String>,
    retained: bool,
    fragment: Option<String>,
}

struct Builder {
    known: BTreeMap<String, Link>,
    records: BTreeMap<String, (Owner, Record)>,
}

impl Builder {
    fn record(&mut self, owner: &Owner, kind: &str, value: Value) -> Result<String, ClewError> {
        let record = Record {
            kind: kind.into(),
            value,
        };
        let identity = digest(&record)?;
        if !self.known.contains_key(&identity) && !self.records.contains_key(&identity) {
            self.records
                .insert(identity.clone(), (owner.clone(), record));
        }
        Ok(identity)
    }
    fn source(&mut self, owner: &Owner, mut value: Value) -> Result<PendingSource, ClewError> {
        let text = if let Some(text) = value.get("text").cloned() {
            if !text.is_string() {
                return Err(corrupt("source text is not a string"));
            }
            value
                .as_object_mut()
                .ok_or_else(|| corrupt("source is not a record"))?
                .remove("text");
            Some(self.record(
                &Owner {
                    kind: "shared".into(),
                    id: "source-text".into(),
                },
                "source-text",
                text,
            )?)
        } else {
            None
        };
        Ok(PendingSource {
            payload: self.record(owner, "source", value)?,
            text,
        })
    }
}

fn reference(schema: &str, raw: &[u8]) -> ObjectRef {
    ObjectRef::new(schema.into(), canonical::hash_bytes(raw), raw.len() as u64)
}

fn checked_chunk(raw: &[u8]) -> Result<Chunk, ClewError> {
    let chunk: Chunk = parse(raw)?;
    if chunk.schema != LEAF || chunk.records.is_empty() || chunk.records.len() > 1024 {
        return Err(corrupt("invalid leaf chunk membership"));
    }
    for (identity, record) in &chunk.records {
        if !hash(identity) || digest(record)? != *identity {
            return Err(corrupt("record identity differs"));
        }
    }
    Ok(chunk)
}

fn prior_locations(
    repo: &Repository,
    raw: Option<&[u8]>,
) -> Result<BTreeMap<String, Link>, ClewError> {
    let mut known = BTreeMap::new();
    let Some(m) = raw.map(|r| manifest(r, false)).transpose()?.flatten() else {
        return Ok(known);
    };
    // The previous closure is only a reuse hint. Every reused payload is still
    // hash/type/size verified, and new membership never comes from this hint.
    for reference in m.objects.iter().filter(|r| r.schema == LEAF) {
        let chunk = checked_chunk(&read_object(repo, reference)?)?;
        for identity in chunk.records.keys() {
            known.entry(identity.clone()).or_insert_with(|| Link {
                chunk: reference.clone(),
                record: identity.clone(),
            });
        }
    }
    Ok(known)
}

fn link(known: &BTreeMap<String, Link>, id: &str) -> Result<Link, ClewError> {
    known
        .get(id)
        .cloned()
        .ok_or_else(|| corrupt("record location is missing"))
}

fn add_ref(
    references: &mut BTreeMap<String, ObjectRef>,
    reference: &ObjectRef,
) -> Result<(), ClewError> {
    object_path(reference)?;
    if let Some(previous) = references.insert(reference.digest.clone(), reference.clone())
        && previous != *reference
    {
        return Err(corrupt("conflicting closure reference"));
    }
    Ok(())
}

fn member_refs(member: &Member) -> Vec<&ObjectRef> {
    let mut refs = vec![&member.payload.chunk];
    if let Some(text) = &member.source_text {
        refs.push(&text.chunk);
    }
    refs
}

/// Encode compact logical bindings; all heavy bytes remain in typed leaf units.
/// No file is written until the complete plan has passed both aggregate budgets.
pub(super) fn encode(
    repo: &Repository,
    value: Value,
    previous: Option<&[u8]>,
    reading: bool,
) -> Result<Encoded, ClewError> {
    if value_size(&value)? > MAX {
        return Err(budget("compact logical bindings exceed 128 MiB"));
    }
    let original = bytes(&value)?;
    if original.len() as u64 > MAX {
        return Err(budget("compact logical bindings exceed 128 MiB"));
    }
    let logical_digest = canonical::hash_bytes(&original);
    let logical_bytes = original.len() as u64;
    drop(original);
    let mut builder = Builder {
        known: prior_locations(repo, previous)?,
        records: BTreeMap::new(),
    };
    let mut header = Map::new();
    let mut tables = Vec::new();
    let mut pending = Vec::new();
    for (table, values) in value
        .as_object()
        .ok_or_else(|| corrupt("logical bindings are not an object"))?
    {
        let Some(values) = values.as_object() else {
            header.insert(table.clone(), values.clone());
            continue;
        };
        tables.push(table.clone());
        for (key, value) in values {
            let owner = owner(table, key, value);
            let mut body = value.clone();
            let mut retained = false;
            let mut source_text = None;
            let payload = if table == "retainedSources" {
                let source = builder.source(&owner, body)?;
                source_text = source.text;
                source.payload
            } else {
                if table == "fragments"
                    && let Some(evidence) = body.get_mut("evidence").and_then(Value::as_object_mut)
                {
                    let observations = evidence
                        .insert("observations".into(), Value::Object(Map::new()))
                        .unwrap_or_else(|| Value::Object(Map::new()));
                    let sources = evidence
                        .insert("sources".into(), Value::Object(Map::new()))
                        .unwrap_or_else(|| Value::Object(Map::new()));
                    for (id, value) in observations
                        .as_object()
                        .ok_or_else(|| corrupt("invalid retained observations"))?
                    {
                        pending.push(Pending {
                            owner: owner.clone(),
                            table: "fragmentObservations".into(),
                            key: id.clone(),
                            payload: builder.record(&owner, "observation", value.clone())?,
                            source_text: None,
                            retained: false,
                            fragment: Some(key.clone()),
                        });
                    }
                    for (id, value) in sources
                        .as_object()
                        .ok_or_else(|| corrupt("invalid retained sources"))?
                    {
                        let source = builder.source(&owner, value.clone())?;
                        pending.push(Pending {
                            owner: owner.clone(),
                            table: "fragmentSources".into(),
                            key: id.clone(),
                            payload: source.payload,
                            source_text: source.text,
                            retained: false,
                            fragment: Some(key.clone()),
                        });
                    }
                    retained = true;
                }
                builder.record(&owner, record_kind(table), body)?
            };
            pending.push(Pending {
                owner,
                table: table.clone(),
                key: key.clone(),
                payload,
                source_text,
                retained,
                fragment: None,
            });
        }
    }
    let mut groups = BTreeMap::<Owner, Vec<(String, Record)>>::new();
    for (id, (owner, record)) in std::mem::take(&mut builder.records) {
        groups.entry(owner).or_default().push((id, record));
    }
    let mut new_objects = BTreeMap::new();
    for records in groups.into_values() {
        let mut chunk = Chunk {
            schema: LEAF.into(),
            records: BTreeMap::new(),
        };
        let mut estimate = 64usize;
        let flush = |chunk: &mut Chunk,
                     known: &mut BTreeMap<String, Link>,
                     objects: &mut BTreeMap<String, Vec<u8>>|
         -> Result<(), ClewError> {
            if chunk.records.is_empty() {
                return Ok(());
            }
            let raw = bytes(chunk)?;
            if raw.len() as u64 > MAX {
                return Err(budget("leaf chunk exceeds 128 MiB"));
            }
            let r = reference(LEAF, &raw);
            for id in chunk.records.keys() {
                known.insert(
                    id.clone(),
                    Link {
                        chunk: r.clone(),
                        record: id.clone(),
                    },
                );
            }
            objects.insert(r.digest.clone(), raw);
            chunk.records.clear();
            Ok(())
        };
        for (id, record) in records {
            let size = bytes(&record)?.len() + id.len() + 8;
            if !chunk.records.is_empty()
                && (estimate + size > TARGET || chunk.records.len() == 1024)
            {
                flush(&mut chunk, &mut builder.known, &mut new_objects)?;
                estimate = 64;
            }
            chunk.records.insert(id, record);
            estimate += size;
        }
        flush(&mut chunk, &mut builder.known, &mut new_objects)?;
    }
    let mut groups = BTreeMap::<Owner, Vec<Member>>::new();
    for p in pending {
        groups.entry(p.owner).or_default().push(Member {
            table: p.table,
            key: p.key,
            payload: link(&builder.known, &p.payload)?,
            source_text: p
                .source_text
                .map(|id| link(&builder.known, &id))
                .transpose()?,
            retained: p.retained,
            fragment: p.fragment,
        });
    }
    let mut partitions = Vec::new();
    let mut references = BTreeMap::new();
    for (owner, mut members) in groups {
        members.sort_by(|a, b| {
            (&a.table, &a.fragment, &a.key, &a.payload.record).cmp(&(
                &b.table,
                &b.fragment,
                &b.key,
                &b.payload.record,
            ))
        });
        let mut index = Index {
            schema: PARTITION.into(),
            owner: owner.clone(),
            members: Vec::new(),
        };
        let mut estimate = bytes(&index)?.len();
        let flush = |index: &mut Index,
                     partitions: &mut Vec<Partition>,
                     objects: &mut BTreeMap<String, Vec<u8>>,
                     refs: &mut BTreeMap<String, ObjectRef>|
         -> Result<(), ClewError> {
            if index.members.is_empty() {
                return Ok(());
            }
            let raw = bytes(index)?;
            if raw.len() as u64 > INDEX_MAX {
                return Err(budget("partition membership exceeds 1 MiB"));
            }
            let r = reference(PARTITION, &raw);
            add_ref(refs, &r)?;
            for member in &index.members {
                for r in member_refs(member) {
                    add_ref(refs, r)?;
                }
            }
            objects.insert(r.digest.clone(), raw);
            partitions.push(Partition {
                owner: index.owner.clone(),
                index: r,
            });
            index.members.clear();
            Ok(())
        };
        for member in members {
            let size = bytes(&member)?.len() + 1;
            if !index.members.is_empty()
                && (estimate + size > TARGET || index.members.len() == 1024)
            {
                flush(
                    &mut index,
                    &mut partitions,
                    &mut new_objects,
                    &mut references,
                )?;
                estimate = bytes(&index)?.len();
            }
            index.members.push(member);
            estimate += size;
        }
        flush(
            &mut index,
            &mut partitions,
            &mut new_objects,
            &mut references,
        )?;
    }
    let manifest = bytes(&Manifest {
        schema: if reading { READING_SCHEMA } else { SCHEMA }.into(),
        object_root: OBJECT_ROOT.into(),
        logical_digest,
        logical_bytes,
        header,
        tables,
        partitions,
        objects: references.into_values().collect(),
    })?;
    let m = self::manifest(&manifest, true)?
        .ok_or_else(|| corrupt("encoder did not produce a manifest"))?;
    Ok(Encoded {
        manifest,
        new_objects,
        references: m.objects,
    })
}

impl Encoded {
    /// Called under the ordinary publication lock, before the pointer switch.
    pub fn persist(&self, repo: &Repository) -> Result<(), ClewError> {
        for reference in &self.references {
            if let Some(raw) = self.new_objects.get(&reference.digest) {
                repo.immutable(&object_path(reference)?, raw)?;
            }
            read_object(repo, reference)?;
        }
        resolve_with(&self.manifest, true, |r| read_object(repo, r))?;
        Ok(())
    }

    pub fn files(&self) -> Result<BTreeMap<String, Vec<u8>>, ClewError> {
        self.references
            .iter()
            .map(|r| {
                let raw = self
                    .new_objects
                    .get(&r.digest)
                    .ok_or_else(|| corrupt("export contains a private or omitted object"))?;
                Ok((object_path(r)?, raw.clone()))
            })
            .collect()
    }
}

/// Publication IDs describe logical state. A previously written immutable bundle
/// may have a different packing history; keep its exact verified representation
/// instead of repacking it under the same ID or forcing a new render generation.
pub(super) fn encode_publication(
    repo: &Repository,
    bundle: &str,
    value: Value,
    previous: Option<&[u8]>,
) -> Result<Encoded, ClewError> {
    let path = repo.path(&format!("docs/generated/{bundle}/bindings.json"))?;
    if path.exists() {
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.is_file() || metadata.len() > MAX {
            return Err(corrupt("existing bindings are not a bounded regular file"));
        }
        let mut raw = Vec::new();
        fs::File::open(path)
            .map_err(io_error)?
            .take(MAX + 1)
            .read_to_end(&mut raw)
            .map_err(io_error)?;
        if let Some(m) = manifest(&raw, false)? {
            let (logical, _) = resolve(repo, &raw)?;
            if logical == value {
                return Ok(Encoded {
                    manifest: raw,
                    new_objects: BTreeMap::new(),
                    references: m.objects,
                });
            }
        }
    }
    encode(repo, value, previous, false)
}

struct Count(u64);
impl Write for Count {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len() as u64);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn value_size(value: &Value) -> Result<u64, ClewError> {
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_err(io_error)?;
    Ok(count.0)
}

fn record<'a>(
    chunks: &'a BTreeMap<String, Chunk>,
    link: &Link,
    kind: &str,
) -> Result<&'a Value, ClewError> {
    if link.chunk.schema != LEAF {
        return Err(corrupt("membership points to a non-leaf object"));
    }
    let record = chunks
        .get(&link.chunk.digest)
        .and_then(|c| c.records.get(&link.record))
        .ok_or_else(|| corrupt("selected record is missing"))?;
    if record.kind != kind {
        return Err(corrupt("selected record has a different type"));
    }
    Ok(&record.value)
}

fn source_size(chunks: &BTreeMap<String, Chunk>, source: &SourceLink) -> Result<u64, ClewError> {
    let meta = record(chunks, &source.payload, "source")?;
    let mut size = value_size(meta)?;
    if let Some(text) = &source.text {
        if meta.get("text").is_some() || !record(chunks, text, "source-text")?.is_string() {
            return Err(corrupt("source text has conflicting membership"));
        }
        size = size
            .saturating_add(value_size(record(chunks, text, "source-text")?)?)
            .saturating_add(16);
    }
    Ok(size)
}

fn source_value(chunks: &BTreeMap<String, Chunk>, source: &SourceLink) -> Result<Value, ClewError> {
    let mut value = record(chunks, &source.payload, "source")?.clone();
    if let Some(text) = &source.text {
        value
            .as_object_mut()
            .ok_or_else(|| corrupt("source metadata is not an object"))?
            .insert("text".into(), record(chunks, text, "source-text")?.clone());
    }
    Ok(value)
}

/// Common resolver for a repository closure and a copied/exported closure.
/// All payloads are currently hydrated eagerly; stats describe actual object reads.
pub(super) fn resolve_with(
    raw: &[u8],
    reading: bool,
    get: impl FnMut(&ObjectRef) -> Result<Vec<u8>, ClewError>,
) -> Result<(Value, ReadStats), ClewError> {
    resolve_limited_with(raw, reading, MAX, get)
}

fn resolve_limited_with(
    raw: &[u8],
    reading: bool,
    logical_limit: u64,
    mut get: impl FnMut(&ObjectRef) -> Result<Vec<u8>, ClewError>,
) -> Result<(Value, ReadStats), ClewError> {
    let Some(m) = manifest(raw, reading)? else {
        return Ok((
            serde_yaml_ng::from_slice(raw).map_err(|_| corrupt("malformed legacy bindings"))?,
            ReadStats::default(),
        ));
    };
    if m.logical_bytes > logical_limit {
        return Err(budget(
            "compact logical bindings exceed the selected input budget",
        ));
    }
    let mut stats = ReadStats::default();
    let mut chunks = BTreeMap::new();
    let mut indexes = BTreeMap::new();
    let mut member_count = 0usize;
    for r in &m.objects {
        let raw = get(r)?;
        if raw.len() as u64 != r.size || canonical::hash_bytes(&raw) != r.digest {
            return Err(corrupt("object identity or size differs"));
        }
        stats.objects += 1;
        stats.bytes += raw.len() as u64;
        if r.schema == LEAF {
            chunks.insert(r.digest.clone(), checked_chunk(&raw)?);
        } else {
            let index: Index = parse(&raw)?;
            if index.schema != PARTITION || index.members.is_empty() || index.members.len() > 1024 {
                return Err(corrupt("invalid partition membership"));
            }
            member_count = member_count.saturating_add(index.members.len());
            if member_count > 262144 {
                return Err(budget("binding membership count exceeds 262144"));
            }
            indexes.insert(r.digest.clone(), index);
        }
    }
    let mut used = BTreeMap::new();
    let mut selected = BTreeSet::new();
    let mut members = Vec::new();
    let mut logical = Value::Object(m.header.clone());
    for table in &m.tables {
        logical
            .as_object_mut()
            .unwrap()
            .insert(table.clone(), Value::Object(Map::new()));
    }
    let mut parents = BTreeMap::new();
    for partition in &m.partitions {
        let index = indexes
            .get(&partition.index.digest)
            .ok_or_else(|| corrupt("partition missing"))?;
        for member in index
            .members
            .iter()
            .filter(|member| member.table == "fragments" && member.fragment.is_none())
        {
            let value = record(&chunks, &member.payload, "fragments")?;
            parents.insert(
                member.key.as_str(),
                (owner("fragments", &member.key, value), member.retained),
            );
        }
    }
    // Conservative serialized-value charge precedes cloning. Auxiliary retained
    // memberships are independently sharded; a large historical scope is not a
    // single unbounded fragment index. The graph has only index -> leaf edges.
    let mut charge = value_size(&logical)?;
    for partition in &m.partitions {
        add_ref(&mut used, &partition.index)?;
        let index = indexes
            .get(&partition.index.digest)
            .ok_or_else(|| corrupt("partition missing"))?;
        if index.owner != partition.owner {
            return Err(corrupt("partition owner differs"));
        }
        for member in &index.members {
            if !selected.insert((
                member.table.clone(),
                member.fragment.clone(),
                member.key.clone(),
            )) {
                return Err(corrupt("duplicate or conflicting table member"));
            }
            if selected.len() > 262144 {
                return Err(budget("binding membership count exceeds 262144"));
            }
            for r in member_refs(member) {
                add_ref(&mut used, r)?;
            }
            let value = record(&chunks, &member.payload, record_kind(&member.table))?;
            let auxiliary = matches!(
                member.table.as_str(),
                "fragmentObservations" | "fragmentSources"
            );
            if auxiliary {
                let parent = member
                    .fragment
                    .as_deref()
                    .and_then(|id| parents.get(id))
                    .ok_or_else(|| corrupt("historical membership has no owning fragment"))?;
                if !parent.1 || parent.0 != partition.owner || member.retained {
                    return Err(corrupt("historical membership has a conflicting owner"));
                }
            } else if !m.tables.contains(&member.table)
                || member.fragment.is_some()
                || owner(&member.table, &member.key, value) != partition.owner
            {
                return Err(corrupt(
                    "member has a different service, subject or table owner",
                ));
            }
            if member.source_text.is_some()
                && !matches!(member.table.as_str(), "retainedSources" | "fragmentSources")
            {
                return Err(corrupt("text membership belongs only to a source"));
            }
            charge = charge
                .saturating_add(value_size(value)?)
                .saturating_add(value_size(&Value::String(member.key.clone()))? + 16);
            if matches!(member.table.as_str(), "retainedSources" | "fragmentSources") {
                let source = SourceLink {
                    payload: member.payload.clone(),
                    text: member.source_text.clone(),
                };
                // source_size validates the text type and absence of competing
                // inline text before source_value is allowed to allocate it.
                charge = charge.saturating_add(
                    source_size(&chunks, &source)?.saturating_sub(value_size(value)?),
                );
            }
            if member.retained {
                if member.table != "fragments"
                    || !value["evidence"].is_object()
                    || value["evidence"]["observations"]
                        .as_object()
                        .is_none_or(|m| !m.is_empty())
                    || value["evidence"]["sources"]
                        .as_object()
                        .is_none_or(|m| !m.is_empty())
                {
                    return Err(corrupt("conflicting retained evidence membership"));
                }
            } else if member.table == "fragments" && value["evidence"].is_object() {
                return Err(corrupt("retained evidence membership is absent"));
            }
            if charge > logical_limit {
                return Err(budget("logical hydration exceeds 128 MiB"));
            }
            members.push(member);
        }
    }
    if used.len() != m.objects.len() || m.objects.iter().any(|r| used.get(&r.digest) != Some(r)) {
        return Err(corrupt("closure is incomplete or contains unowned objects"));
    }
    let reconstruct = |member: &Member| -> Result<Value, ClewError> {
        if matches!(member.table.as_str(), "retainedSources" | "fragmentSources") {
            source_value(
                &chunks,
                &SourceLink {
                    payload: member.payload.clone(),
                    text: member.source_text.clone(),
                },
            )
        } else {
            Ok(record(&chunks, &member.payload, record_kind(&member.table))?.clone())
        }
    };
    for member in members.iter().filter(|m| m.fragment.is_none()) {
        logical[&member.table]
            .as_object_mut()
            .unwrap()
            .insert(member.key.clone(), reconstruct(member)?);
    }
    for member in members.iter().filter(|m| m.fragment.is_some()) {
        let field = if member.table == "fragmentSources" {
            "sources"
        } else {
            "observations"
        };
        let target =
            &mut logical["fragments"][member.fragment.as_ref().unwrap()]["evidence"][field];
        target
            .as_object_mut()
            .ok_or_else(|| corrupt("historical membership target is missing"))?
            .insert(member.key.clone(), reconstruct(member)?);
    }
    let encoded = bytes(&logical)?;
    if encoded.len() as u64 > logical_limit {
        return Err(budget("reconstructed compact bindings exceed 128 MiB"));
    }
    if encoded.len() as u64 != m.logical_bytes
        || canonical::hash_bytes(&encoded) != m.logical_digest
    {
        return Err(corrupt(
            "logical binding identity or historical occurrence differs",
        ));
    }
    stats.logical_bytes = encoded.len() as u64;
    Ok((logical, stats))
}

pub(super) fn resolve(repo: &Repository, raw: &[u8]) -> Result<(Value, ReadStats), ClewError> {
    resolve_with(raw, false, |r| read_object(repo, r))
}

pub(super) fn preflight_frozen(
    repo: &Repository,
    raw: &[u8],
    limit: u64,
) -> Result<u64, ClewError> {
    let Some(m) = manifest(raw, false)? else {
        return Ok(0);
    };
    resolve_limited_with(raw, false, limit, |r| read_object(repo, r))?;
    let physical = raw.len() as u64 + m.objects.iter().map(|r| r.size).sum::<u64>();
    Ok(m.logical_bytes.saturating_sub(physical))
}

pub(super) fn logical_size(raw: &[u8]) -> Result<Option<u64>, ClewError> {
    Ok(manifest(raw, false)?.map(|m| m.logical_bytes))
}

/// Reading export packs only selected logical members after source projection.
/// It must never attach the original reused chunks to a remote directory.
pub(super) fn export_files(
    repo: &Repository,
    raw: &[u8],
    projected: Option<Value>,
) -> Result<BTreeMap<String, Vec<u8>>, ClewError> {
    let files = if let Some(value) = projected {
        let encoded = encode(repo, value, None, true)?;
        let mut files = encoded.files()?;
        files.insert("frozen-bindings.json".into(), encoded.manifest);
        files
    } else if let Some(m) = manifest(raw, false)? {
        let mut files = BTreeMap::new();
        for r in &m.objects {
            files.insert(object_path(r)?, read_object(repo, r)?);
        }
        files.insert("frozen-bindings.json".into(), raw.to_vec());
        files
    } else {
        let (value, _) = resolve(repo, raw)?;
        let encoded = encode(repo, value, None, false)?;
        let mut files = encoded.files()?;
        files.insert("frozen-bindings.json".into(), encoded.manifest);
        files
    };
    resolve_export(&files, "")?;
    Ok(files)
}

pub(super) fn export_file(name: &str) -> bool {
    let name = name.strip_prefix("static/clew-bindings/").unwrap_or(name);
    name == "frozen-bindings.json" || name.starts_with(&format!("{OBJECT_ROOT}/"))
}

pub(super) fn resolve_export(
    files: &BTreeMap<String, Vec<u8>>,
    prefix: &str,
) -> Result<Value, ClewError> {
    let raw = files
        .get(&format!("{prefix}frozen-bindings.json"))
        .ok_or_else(|| corrupt("exported bindings manifest missing"))?;
    Ok(resolve_with(raw, true, |r| {
        files
            .get(&format!("{prefix}{}", object_path(r)?))
            .cloned()
            .ok_or_else(|| corrupt("exported closure object missing"))
    })?
    .0)
}

pub(super) fn resolve_optional(repo: &Repository, raw: &[u8]) -> Result<Option<Value>, ClewError> {
    if manifest(raw, false)?.is_none() {
        return Ok(None);
    }
    Ok(Some(resolve(repo, raw)?.0))
}

#[cfg(test)]
#[path = "bindings_storage_tests.rs"]
mod tests;
