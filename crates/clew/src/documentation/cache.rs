//! Versioned docs-local immutable content-addressed object store.
//!
//! This is the first storage slice for cache normalization: heavy evidence
//! payloads (source text, observations, service evidence) are stored once by
//! content identity, while small capture/check manifests hold validated object
//! references. One unchanged payload has one content identity even when
//! producer/admission keys differ. Corruption, missing objects, schema
//! mismatches, unsafe paths, and conflicting concurrent writes yield explicit
//! miss/error behavior, never silently forged evidence.
//!
//! Deletion, automatic retention, pack compaction, and migration are out of
//! scope here; see the operational lifecycle runbook.

use super::{check, invalid, io_error};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Root for immutable owned objects, relative to the documentation root.
pub const OBJECT_ROOT: &str = ".codeclew/cache/objects";

/// A validated reference to one immutable object. Object identity is the
/// content digest; `schema` is a producer/admission attribute on the reference
/// (provenance is separate from payload identity, so equal source text may
/// share a payload without collapsing different provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectRef {
    pub schema: String,
    pub digest: String,
    pub size: u64,
}

impl ObjectRef {
    pub fn new(schema: String, digest: String, size: u64) -> Self {
        Self {
            schema,
            digest,
            size,
        }
    }
}

fn canonical_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn object_dir(root: &Path, digest: &str) -> Result<std::path::PathBuf, ClewError> {
    if !canonical_digest(digest) {
        return Err(invalid(
            "object reference digest must be a lowercase sha256 content identity",
        ));
    }
    Ok(root.join(OBJECT_ROOT).join(digest))
}

/// Content address of `payload` (lowercase `sha256:`-prefixed canonical hash).
pub fn content_digest(payload: &[u8]) -> String {
    crate::canonical::hash_bytes(payload)
}

/// Write an immutable object. Byte-identical payloads share one object
/// regardless of the producer key or schema. Returns the validated reference.
/// Existing objects are reused after their size is checked; advisory metadata
/// sidecars are legacy state and are left untouched.
pub fn put(
    repo: &super::store::Repository,
    schema: &str,
    payload: &[u8],
) -> Result<ObjectRef, ClewError> {
    if payload.len() as u64 > check::PORTABLE_CACHE_MAX_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "documentation object exceeds the portable record budget",
        ));
    }
    let digest = content_digest(payload);
    let dir = object_dir(&repo.root, &digest)?;
    let object = dir.join("object.json");
    if object.exists() {
        // Content-addressed idempotency: same digest means same bytes. A
        // pre-existing object with a mismatched size is a corrupt store.
        let metadata = fs::metadata(&object).map_err(io_error)?;
        if metadata.len() != payload.len() as u64 {
            return Err(ClewError::new(
                ErrorCode::StateCorrupt,
                "documentation object store has a size-conflicting object",
            ));
        }
        return Ok(ObjectRef::new(
            schema.to_string(),
            digest,
            payload.len() as u64,
        ));
    }
    fs::create_dir_all(&dir).map_err(io_error)?;
    atomic_write(&object, payload)?;
    Ok(ObjectRef::new(
        schema.to_string(),
        digest,
        payload.len() as u64,
    ))
}

/// Read and verify an object, honoring a per-read bound. Missing objects are a
/// distinct miss (Ok(None)); corrupt, unsafe, or oversized objects are
/// explicit errors.
pub fn get(
    repo: &super::store::Repository,
    reference: &ObjectRef,
    limit: u64,
) -> Result<Option<Vec<u8>>, ClewError> {
    if !canonical_digest(&reference.digest) {
        return Err(invalid(
            "object reference digest must be a lowercase sha256 content identity",
        ));
    }
    if reference.size > limit {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "documentation object exceeds the read bound",
        ));
    }
    let dir = object_dir(&repo.root, &reference.digest)?;
    let object = dir.join("object.json");
    if !object.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(&object).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid(
            "documentation object is not a bounded regular file",
        ));
    }
    if metadata.len() != reference.size {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "documentation object size does not match its reference",
        ));
    }
    let payload = fs::read(&object).map_err(io_error)?;
    if crate::canonical::hash_bytes(&payload) != reference.digest {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "documentation object content does not match its reference digest",
        ));
    }
    Ok(Some(payload))
}

/// Verify an object is present and intact (bounded read). A corrupt or missing
/// object is an explicit error, so a consumer can never accept forged evidence.
pub fn verify(repo: &super::store::Repository, reference: &ObjectRef) -> Result<(), ClewError> {
    if get(repo, reference, check::PORTABLE_CACHE_MAX_BYTES)?.is_none() {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "documentation object reference is missing from the store",
        ));
    }
    Ok(())
}

/// Enumerate all owned object digests present in the store (read-only, for
/// inventory/reachability). Returns an upper bound on enumeration size.
pub fn owned_digests(
    repo: &super::store::Repository,
    limit: usize,
) -> Result<Vec<String>, ClewError> {
    let root = repo.root.join(OBJECT_ROOT);
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(&root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !canonical_digest(&name) || !entry.path().join("object.json").exists() {
            continue;
        }
        out.push(name);
        if out.len() >= limit {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "documentation object enumeration exceeds the bound",
            ));
        }
    }
    Ok(out)
}

/// Serialize `value` canonically, store it as an immutable object, and return
/// its validated reference.
pub fn put_json<T: Serialize>(
    repo: &super::store::Repository,
    schema: &str,
    value: &T,
) -> Result<ObjectRef, ClewError> {
    let payload = crate::canonical::bytes(value).map_err(io_error)?;
    put(repo, schema, &payload)
}

/// Read and deserialize an object reference. A miss returns `Ok(None)`; a
/// corrupt or oversized object is an explicit error.
pub fn get_json<T: serde::de::DeserializeOwned>(
    repo: &super::store::Repository,
    reference: &ObjectRef,
    limit: u64,
) -> Result<Option<T>, ClewError> {
    match get(repo, reference, limit)? {
        None => Ok(None),
        Some(payload) => serde_json::from_slice(&payload)
            .map(Some)
            .map_err(|error| invalid(error.to_string())),
    }
}

/// Schema for a per-service capture reference envelope (schema
/// `codeclew-documentation-capture-manifest/1.0`). The manifest carries light
/// identity and the heavy payload as validated immutable object references, so
/// byte-identical evidence is stored once regardless of producer/admission key.
pub const CAPTURE_MANIFEST_SCHEMA: &str = "codeclew-documentation-capture-manifest/1.0";
pub const SOURCES_OBJECT_SCHEMA: &str = "codeclew-documentation-sources/1.0";
pub const OBSERVATIONS_OBJECT_SCHEMA: &str = "codeclew-documentation-observations/1.0";
pub const CONTRACTS_OBJECT_SCHEMA: &str = "codeclew-documentation-contracts/1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureManifest {
    pub schema: String,
    pub service: String,
    pub revision: String,
    pub service_digest: String,
    pub extractor: String,
    pub runtime_mode: String,
    pub coverage: String,
    pub boundaries: Vec<String>,
    pub entrypoints: Vec<super::model::Entrypoint>,
    pub sources: ObjectRef,
    pub observations: ObjectRef,
    pub contracts: ObjectRef,
    /// Whether this capture is reusable. Fully admitted stable inputs are
    /// "REUSABLE"; incomplete external authority is "NON_CACHEABLE" with a
    /// reason, so the user is told why a capture will be re-run.
    #[serde(default = "default_cacheability")]
    pub cacheability: String,
    #[serde(default)]
    pub reason: Option<String>,
}

pub const REUSABLE: &str = "REUSABLE";
pub const NON_CACHEABLE: &str = "NON_CACHEABLE";

fn default_cacheability() -> String {
    REUSABLE.into()
}

/// Mark a capture manifest as non-cacheable with a reason (e.g. incomplete
/// external build/settings/dependency authority).
pub fn mark_non_cacheable(manifest: &mut CaptureManifest, reason: &str) {
    manifest.cacheability = NON_CACHEABLE.into();
    manifest.reason = Some(reason.into());
}

/// Persist a service capture as a reference envelope: heavy `sources`,
/// `observations` and `contracts` become immutable objects, and a small
/// manifest is returned for the caller to write at the keyed cache path.
pub fn store_capture(
    repo: &super::store::Repository,
    evidence: &super::model::ServiceEvidence,
) -> Result<CaptureManifest, ClewError> {
    let sources = put_json(repo, SOURCES_OBJECT_SCHEMA, &evidence.sources)?;
    let observations = put_json(repo, OBSERVATIONS_OBJECT_SCHEMA, &evidence.observations)?;
    let contracts = put_json(repo, CONTRACTS_OBJECT_SCHEMA, &evidence.contracts)?;
    Ok(CaptureManifest {
        schema: CAPTURE_MANIFEST_SCHEMA.into(),
        service: evidence.service.clone(),
        revision: evidence.revision.clone(),
        service_digest: evidence.service_digest.clone(),
        extractor: evidence.extractor.clone(),
        runtime_mode: evidence.runtime_mode.clone(),
        coverage: evidence.coverage.clone(),
        boundaries: evidence.boundaries.clone(),
        entrypoints: evidence.entrypoints.clone(),
        sources,
        observations,
        contracts,
        cacheability: REUSABLE.into(),
        reason: None,
    })
}

/// Hydrate a `ServiceEvidence` from a capture reference envelope, reading the
/// heavy payload back through the verified object store.
pub fn load_capture(
    repo: &super::store::Repository,
    manifest: &CaptureManifest,
) -> Result<super::model::ServiceEvidence, ClewError> {
    let limit = check::PORTABLE_CACHE_MAX_BYTES;
    let sources = get_json(repo, &manifest.sources, limit)?.ok_or_else(|| {
        ClewError::new(ErrorCode::StateCorrupt, "capture sources object is missing")
    })?;
    let observations = get_json(repo, &manifest.observations, limit)?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::StateCorrupt,
            "capture observations object is missing",
        )
    })?;
    let contracts = get_json(repo, &manifest.contracts, limit)?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::StateCorrupt,
            "capture contracts object is missing",
        )
    })?;
    Ok(super::model::ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: manifest.service.clone(),
        revision: manifest.revision.clone(),
        service_digest: manifest.service_digest.clone(),
        extractor: manifest.extractor.clone(),
        runtime_mode: manifest.runtime_mode.clone(),
        coverage: manifest.coverage.clone(),
        boundaries: manifest.boundaries.clone(),
        entrypoints: manifest.entrypoints.clone(),
        observations,
        sources,
        contracts,
    })
}

/// Resolve the keyed capture-manifest path (matching the producer's keyed
/// cache filename `.codeclew/cache/{service}-{key-short}.json`).
pub fn capture_manifest_path(
    repo: &super::store::Repository,
    service_id: &str,
    cache_key: &str,
) -> Result<std::path::PathBuf, ClewError> {
    super::store::relative(service_id)?;
    Ok(repo.root.join(".codeclew/cache").join(format!(
        "{service_id}-{}.json",
        cache_key.trim_start_matches("sha256:")
    )))
}

/// Reuse a still-valid capture for a fully admitted, stable input: loads the
/// keyed reference envelope and verifies every referenced object before
/// returning evidence. Corruption or a missing object is an explicit error, so
/// a stale or damaged capture is never silently reused. This is the safe reuse
/// gate; callers decide which inputs are cacheable (deterministic extractors),
/// and Maven/external-state inputs remain non-cacheable here.
pub fn load_capture_if_valid(
    repo: &super::store::Repository,
    service_id: &str,
    cache_key: &str,
) -> Result<Option<super::model::ServiceEvidence>, ClewError> {
    let path = capture_manifest_path(repo, service_id, cache_key)?;
    if !path.exists() {
        return Ok(None);
    }
    let manifest: CaptureManifest =
        serde_json::from_slice(&std::fs::read(&path).map_err(io_error)?)
            .map_err(|error| invalid(error.to_string()))?;
    // A non-cacheable capture (incomplete external authority) is never reused;
    // the caller must recapture.
    if manifest.cacheability == NON_CACHEABLE {
        return Ok(None);
    }
    // Validate current freshness before reuse: every referenced heavy object
    // must be present and intact.
    verify(repo, &manifest.sources)?;
    verify(repo, &manifest.observations)?;
    verify(repo, &manifest.contracts)?;
    Ok(Some(load_capture(repo, &manifest)?))
}

/// Persist a service capture as a keyed reference envelope for later reuse.
pub fn save_capture(
    repo: &super::store::Repository,
    service_id: &str,
    cache_key: &str,
    evidence: &super::model::ServiceEvidence,
) -> Result<(), ClewError> {
    let manifest = store_capture(repo, evidence)?;
    let path = capture_manifest_path(repo, service_id, cache_key)?;
    let payload = crate::canonical::bytes(&manifest).map_err(io_error)?;
    let dir = path
        .parent()
        .ok_or_else(|| invalid("capture path has no parent"))?;
    std::fs::create_dir_all(dir).map_err(io_error)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(io_error)?;
    temp.write_all(&payload).map_err(io_error)?;
    temp.as_file().sync_all().map_err(io_error)?;
    temp.persist(&path).map_err(io_error)?;
    Ok(())
}

fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), ClewError> {
    let dir = path
        .parent()
        .ok_or_else(|| invalid("object path has no parent"))?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(io_error)?;
    temp.write_all(data).map_err(io_error)?;
    temp.as_file().sync_all().map_err(io_error)?;
    temp.persist(path).map_err(io_error)?;
    Ok(())
}

/// Read-only docs cache inventory: conservative bounded accounting that
/// distinguishes disposable immutable objects from retained publication
/// evidence and current/legacy roots. This never deletes or migrates data and
/// never reports an unknown or active root as safely deletable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Inventory {
    pub schema: String,
    pub object_count: usize,
    pub object_bytes: u64,
    pub manifest_bytes: u64,
    pub publication_count: usize,
    pub publication_bytes: u64,
    pub current_root: Option<String>,
}

pub const INVENTORY_SCHEMA: &str = "codeclew-documentation-cache-inventory/1.0";

/// Compute a read-only inventory of the docs-local cache.
pub fn inventory(repo: &super::store::Repository) -> Result<Inventory, ClewError> {
    let limit = 4096usize;
    let mut object_count = 0usize;
    let mut object_bytes = 0u64;
    for digest in owned_digests(repo, limit)? {
        let dir = repo.root.join(OBJECT_ROOT).join(&digest);
        if let Ok(metadata) = fs::metadata(dir.join("object.json")) {
            object_count += 1;
            object_bytes += metadata.len();
        }
    }
    // Keyed capture manifests + latest-check are reference envelopes (small).
    let mut manifest_bytes = 0u64;
    let cache_dir = repo.root.join(".codeclew/cache");
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(metadata) = fs::metadata(&path)
            {
                manifest_bytes += metadata.len();
            }
        }
    }
    // Retained publication generations under docs/generated (read-only count).
    let mut publication_count = 0usize;
    let mut publication_bytes = 0u64;
    let generated = repo.root.join("docs/generated");
    if let Ok(entries) = fs::read_dir(&generated) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("publication.json").exists() {
                publication_count += 1;
                publication_bytes += directory_bytes(&path, 8192)?;
            }
        }
    }
    let current_root = current_bundle(repo)?;
    Ok(Inventory {
        schema: INVENTORY_SCHEMA.into(),
        object_count,
        object_bytes,
        manifest_bytes,
        publication_count,
        publication_bytes,
        current_root,
    })
}

/// Parse the current bundle id from the first line of `docs/index.html`
/// (read-only; a missing index is simply no current root).
fn current_bundle(repo: &super::store::Repository) -> Result<Option<String>, ClewError> {
    let index = repo.root.join("docs/index.html");
    if !index.exists() {
        return Ok(None);
    }
    let first = fs::read_to_string(&index).map_err(io_error)?;
    let line = first.lines().next().unwrap_or_default().trim();
    Ok(line
        .strip_prefix("<!-- codeclew-bundle ")
        .and_then(|id| id.strip_suffix(" -->"))
        .map(str::to_owned))
}

fn directory_bytes(root: &std::path::Path, limit: usize) -> Result<u64, ClewError> {
    let mut total = 0u64;
    let mut visited = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(io_error)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(metadata) = fs::metadata(&path) {
                total += metadata.len();
                visited += 1;
                if visited >= limit {
                    return Err(ClewError::new(
                        ErrorCode::ResourceLimit,
                        "documentation publication enumeration exceeds the bound",
                    ));
                }
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::store::Repository;

    fn setup() -> (tempfile::TempDir, Repository) {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "Architecture").unwrap();
        let repo = Repository::open(t.path()).unwrap();
        (t, repo)
    }

    #[test]
    fn new_object_put_get_and_verify_do_not_emit_advisory_sidecar() {
        let (_t, repo) = setup();
        let reference = put(&repo, "schema/evidence/1", b"sidecar-free payload").unwrap();
        let dir = repo.root.join(OBJECT_ROOT).join(&reference.digest);

        assert!(dir.join("object.json").is_file());
        assert!(!dir.join("meta.json").exists());
        assert_eq!(
            get(&repo, &reference, 1024).unwrap(),
            Some(b"sidecar-free payload".to_vec())
        );
        verify(&repo, &reference).unwrap();
    }

    #[test]
    fn differently_keyed_identical_payloads_share_one_object() {
        let (_t, repo) = setup();
        let a = put(&repo, "schema/evidence/1", b"same payload").unwrap();
        let b = put(&repo, "schema/evidence/OTHER", b"same payload").unwrap();
        assert_eq!(
            a.digest, b.digest,
            "content identity must ignore producer key"
        );
        assert_eq!(a.size, b.size);
        // Schema differs but payload identity is shared (provenance is separate).
        assert_ne!(a.schema, b.schema);
        assert_eq!(a.schema, "schema/evidence/1");
        assert_eq!(b.schema, "schema/evidence/OTHER");
        assert!(
            !repo
                .root
                .join(OBJECT_ROOT)
                .join(&a.digest)
                .join("meta.json")
                .exists()
        );
        let digests = owned_digests(&repo, 1000).unwrap();
        assert_eq!(digests.len(), 1, "byte-identical evidence stored once");
    }

    #[test]
    fn legacy_advisory_sidecar_remains_readable_and_reusable() {
        let (_t, repo) = setup();
        let payload = b"legacy payload";
        let digest = content_digest(payload);
        let dir = repo.root.join(OBJECT_ROOT).join(&digest);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("object.json"), payload).unwrap();
        let legacy_meta = br#"{"schema":"legacy/schema","size":14}"#;
        fs::write(dir.join("meta.json"), legacy_meta).unwrap();
        let reference = ObjectRef::new("legacy/schema".into(), digest, payload.len() as u64);

        assert_eq!(
            get(&repo, &reference, 1024).unwrap(),
            Some(payload.to_vec())
        );
        verify(&repo, &reference).unwrap();
        let reused = put(&repo, "new/schema", payload).unwrap();
        assert_eq!(reused.schema, "new/schema");
        assert_eq!(fs::read(dir.join("meta.json")).unwrap(), legacy_meta);
    }

    #[test]
    fn round_trip_returns_exact_bytes_and_missing_is_distinct() {
        let (_t, repo) = setup();
        let reference = put(&repo, "schema/evidence/1", b"exact bytes").unwrap();
        let loaded = get(&repo, &reference, 1024)
            .unwrap()
            .expect("object present");
        assert_eq!(loaded, b"exact bytes");
        let missing = ObjectRef::new(
            "schema/x".to_string(),
            format!("sha256:{}", "0".repeat(64)),
            1,
        );
        assert!(get(&repo, &missing, 1024).unwrap().is_none());
    }

    #[test]
    fn corruption_and_mismatched_size_are_explicit_errors() {
        let (_t, repo) = setup();
        let reference = put(&repo, "schema/evidence/1", b"integrity").unwrap();
        let dir = repo.root.join(OBJECT_ROOT).join(&reference.digest);
        fs::write(dir.join("object.json"), b"tampered").unwrap();
        let error = get(&repo, &reference, 1024).unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    #[test]
    fn oversized_and_malformed_references_are_rejected() {
        let (_t, repo) = setup();
        let big = ObjectRef::new(
            "schema/x".to_string(),
            format!("sha256:{}", "a".repeat(64)),
            9999,
        );
        let error = get(&repo, &big, 100).unwrap_err();
        assert_eq!(error.code, ErrorCode::ResourceLimit);
        let malformed = ObjectRef::new("schema/x".to_string(), "not-a-digest".to_string(), 1);
        assert_eq!(
            put(&repo, "schema/x", b"payload").unwrap().schema,
            "schema/x"
        );
        let error = get(&repo, &malformed, 1024).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn oversized_payload_put_is_a_typed_budget_spill() {
        let (_t, repo) = setup();
        let size = (check::PORTABLE_CACHE_MAX_BYTES as usize) + 1;
        let payload = vec![0u8; size];
        let error = put(&repo, "schema/x", &payload).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }

    fn evidence(id: &str) -> crate::documentation::model::ServiceEvidence {
        use crate::documentation::model::Source;
        let source = Source {
            id: format!("{id}:src"),
            service: id.into(),
            revision: "a".repeat(40),
            file: "src/Service.java".into(),
            start_line: 1,
            end_line: 2,
            text: "public class Service {}".into(),
            text_digest: "sha256:1111".into(),
            evidence_digest: "sha256:2222".into(),
            authority: "EXACT_SNAPSHOT_TEXT".into(),
            occurrence: None,
            url: None,
        };
        let mut sources = std::collections::BTreeMap::new();
        sources.insert(source.id.clone(), source);
        crate::documentation::model::ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: id.into(),
            revision: "a".repeat(40),
            service_digest: "sha256:3333".into(),
            extractor: crate::documentation::model::EXTRACTOR.into(),
            runtime_mode: "DEVELOPMENT".into(),
            coverage: "FULL".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: Default::default(),
            sources,
            contracts: Default::default(),
        }
    }

    #[test]
    fn capture_envelope_round_trips_and_heavy_payload_dedups() {
        let (_t, repo) = setup();
        let a = store_capture(&repo, &evidence("svc-a")).unwrap();
        // Same heavy payload (identical sources) under a different service key
        // shares the same immutable objects (content identity).
        let mut b_evidence = evidence("svc-b");
        b_evidence.sources = evidence("svc-a").sources.clone();
        let b = store_capture(&repo, &b_evidence).unwrap();
        assert_eq!(
            a.sources.digest, b.sources.digest,
            "byte-identical heavy payload stored once across producer keys"
        );
        // Hydration reproduces the evidence, preserving authority/content.
        let loaded = load_capture(&repo, &a).unwrap();
        assert_eq!(loaded.service, "svc-a");
        assert_eq!(
            loaded.sources.values().next().unwrap().text,
            "public class Service {}"
        );
        // Objects are enumerated once despite two manifests: one for the
        // sources payload and one shared for the (empty) observations and
        // contracts payloads, which are byte-identical and deduplicate.
        let digests = owned_digests(&repo, 1000).unwrap();
        assert_eq!(digests.len(), 2, "deduplicated heavy payload objects");
        // A missing payload object is a corrupt-state error, never forged.
        let mut dangling = a.clone();
        dangling.sources.digest = format!("sha256:{}", "c".repeat(64));
        let error = load_capture(&repo, &dangling).unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    #[test]
    fn read_only_inventory_reports_objects_and_retained_roots() {
        let (_t, repo) = setup();
        let evidence = evidence_for_test("svc", "inventory text");
        store_capture(&repo, &evidence).unwrap();
        // A current root marker and a retained publication.
        let bundle = "a".repeat(64);
        let generated = repo.root.join("docs/generated").join(&bundle);
        fs::create_dir_all(&generated).unwrap();
        fs::write(generated.join("publication.json"), b"{}").unwrap();
        fs::write(generated.join("page.html"), b"<html></html>").unwrap();
        fs::write(
            repo.root.join("docs/index.html"),
            format!("<!-- codeclew-bundle {bundle} -->\n"),
        )
        .unwrap();
        let inventory = inventory(&repo).unwrap();
        assert!(inventory.object_count >= 1, "owned objects counted");
        assert!(inventory.object_bytes > 0);
        assert_eq!(inventory.publication_count, 1);
        assert!(inventory.publication_bytes > 0);
        assert_eq!(inventory.current_root.as_deref(), Some(bundle.as_str()));
        // Read-only: no deletion or migration occurs.
        assert!(repo.root.join(OBJECT_ROOT).exists());
    }

    fn evidence_for_test(id: &str, text: &str) -> crate::documentation::model::ServiceEvidence {
        use crate::documentation::model::Source;
        let source = Source {
            id: format!("{id}:src"),
            service: id.into(),
            revision: "a".repeat(40),
            file: "src/Service.java".into(),
            start_line: 1,
            end_line: 2,
            text: text.into(),
            text_digest: "sha256:1111".into(),
            evidence_digest: "sha256:2222".into(),
            authority: "EXACT_SNAPSHOT_TEXT".into(),
            occurrence: None,
            url: None,
        };
        let mut sources = std::collections::BTreeMap::new();
        sources.insert(source.id.clone(), source);
        crate::documentation::model::ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: id.into(),
            revision: "a".repeat(40),
            service_digest: "sha256:3333".into(),
            extractor: crate::documentation::model::EXTRACTOR.into(),
            runtime_mode: "DEVELOPMENT".into(),
            coverage: "FULL".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: Default::default(),
            sources,
            contracts: Default::default(),
        }
    }
}
