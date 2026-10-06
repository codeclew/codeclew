//! Same-service, same-path syntax extraction, with current source receipts rebuilt by the caller.
use super::{bytes, cache, digest, io_error, model::*, store};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, sync::OnceLock};

const SCHEMA: &str = "codeclew-documentation-syntax-file/1.0";
const ENVELOPE_SCHEMA: &str = "codeclew-documentation-syntax-file-manifest/1.0";
const MAX_PAYLOAD: u64 = 64 * 1024 * 1024;

/// Ordered calls, rather than distinct sources, preserve the overlapping source-byte budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SourceRecipe {
    pub identity: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Payload {
    pub schema: String,
    pub key: String,
    pub service: String,
    pub path: String,
    pub fingerprint: String,
    pub sources: Vec<SourceRecipe>,
    pub observations: BTreeMap<String, Observation>,
    pub entrypoints: Vec<Entrypoint>,
    pub boundaries: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    schema: String,
    key: String,
    payload: cache::ObjectRef,
}

fn corrupt() -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, "invalid syntax-file cache binding")
}

/// Build inputs, not just a manually maintained extractor label, admit this producer.
/// Hashing the complete pinned lock and relevant implementation files is conservative:
/// unrelated edits can cause misses but cannot preserve an obsolete normalization policy.
pub(super) fn producer_identity() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        digest(&[
            include_str!("../../../../Cargo.lock"),
            include_str!("../../../../rust-toolchain.toml"),
            include_str!("../../../../Cargo.toml"),
            include_str!("../../Cargo.toml"),
            include_str!("syntax.rs"),
            include_str!("syntax_file_cache.rs"),
            include_str!("source_annotations.rs"),
            include_str!("analysis.rs"),
            include_str!("model.rs"),
            include_str!("../canonical.rs"),
            include_str!("../spring_entrypoints.rs"),
            include_str!("../../../clew-framework-spring/src/lib.rs"),
            include_str!("../../../clew-facts/src/lib.rs"),
        ])
        .expect("bundled syntax producer identity is serializable")
    })
}

pub(super) fn producer_key(service: &Service, path: &str, text: &str) -> Result<String, ClewError> {
    key_with_producer(service, path, text, producer_identity())
}

pub(super) fn key_with_producer(
    service: &Service,
    path: &str,
    text: &str,
    producer: &str,
) -> Result<String, ClewError> {
    digest(
        &json!({"schema":SCHEMA,"producer":producer,"service":service.id,
        "path":path,"language":service.language,"dialect":service.source.as_ref().map(|s| &s.dialect),
        "bytes":crate::canonical::hash_bytes(text.as_bytes())}),
    )
}

pub(super) fn manifest_path(key: &str) -> String {
    format!(
        ".codeclew/cache/syntax-file-{}.json",
        key.trim_start_matches("sha256:")
    )
}

pub(super) fn load(
    repo: &store::Repository,
    key: &str,
    service: &str,
    path: &str,
) -> Result<Option<Payload>, ClewError> {
    let manifest = repo.path(&manifest_path(key))?;
    if !manifest.try_exists().map_err(io_error)? {
        return Ok(None);
    }
    let envelope: Envelope = store::read(&manifest, 4096).map_err(|_| corrupt())?;
    if envelope.schema != ENVELOPE_SCHEMA
        || envelope.key != key
        || envelope.payload.schema != SCHEMA
    {
        return Err(corrupt());
    }
    let payload: Payload =
        cache::get_json(repo, &envelope.payload, MAX_PAYLOAD)?.ok_or_else(corrupt)?;
    if payload.schema != SCHEMA
        || payload.key != key
        || payload.service != service
        || payload.path != path
    {
        return Err(corrupt());
    }
    Ok(Some(payload))
}

pub(super) fn save(repo: &store::Repository, payload: &Payload) -> Result<(), ClewError> {
    let encoded = bytes(payload)?;
    // A large but valid capture remains usable; its optional file cache is skipped.
    if encoded.len() as u64 > MAX_PAYLOAD {
        return Ok(());
    }
    let reference = cache::put(repo, SCHEMA, &encoded)?;
    repo.atomic(
        &manifest_path(&payload.key),
        &bytes(&Envelope {
            schema: ENVELOPE_SCHEMA.into(),
            key: payload.key.clone(),
            payload: reference,
        })?,
    )
}

pub(super) fn payload(
    key: String,
    path: &str,
    fingerprint: String,
    sources: Vec<SourceRecipe>,
    evidence: ServiceEvidence,
) -> Payload {
    Payload {
        schema: SCHEMA.into(),
        key,
        service: evidence.service,
        path: path.into(),
        fingerprint,
        sources,
        observations: evidence.observations,
        entrypoints: evidence.entrypoints,
        boundaries: evidence.boundaries,
    }
}
