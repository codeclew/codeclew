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
//! Every current documentation root uses one durable SQLite object store.
//! Legacy loose-object roots are rejected at admission and are never read,
//! rewritten, migrated, or deleted by this process.

use super::{check, invalid, io_error};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;

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

/// Content address of `payload` (lowercase `sha256:`-prefixed canonical hash).
pub fn content_digest(payload: &[u8]) -> String {
    crate::canonical::hash_bytes(payload)
}

/// Write immutable payloads to the selected SQLite object store.
pub fn put(
    repo: &super::store::Repository,
    schema: &str,
    payload: &[u8],
) -> Result<ObjectRef, ClewError> {
    let mut result = put_batch(repo, schema, &[payload])?;
    Ok(result.remove(0))
}

/// Explicit bounded durable unit. The caller bounds the serialized payloads.
pub fn put_batch(
    repo: &super::store::Repository,
    schema: &str,
    payloads: &[&[u8]],
) -> Result<Vec<ObjectRef>, ClewError> {
    let references = payloads
        .iter()
        .map(|payload| {
            if payload.len() as u64 > check::PORTABLE_CACHE_MAX_BYTES {
                return Err(ClewError::new(
                    ErrorCode::SliceBudgetExceeded,
                    "documentation object exceeds the portable record budget",
                ));
            }
            Ok(ObjectRef::new(
                schema.into(),
                content_digest(payload),
                payload.len() as u64,
            ))
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    super::object_layout::with_store(repo, |store| {
        let rows: Vec<_> = references
            .iter()
            .zip(payloads)
            .map(|(reference, payload)| (reference.digest.as_str(), *payload))
            .collect();
        store.put_many(&rows)?;
        Ok(references)
    })
}

pub fn get(
    repo: &super::store::Repository,
    reference: &ObjectRef,
    limit: u64,
) -> Result<Option<Vec<u8>>, ClewError> {
    if reference.size > limit {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "documentation object exceeds the read bound",
        ));
    }
    super::object_layout::with_store(repo, |store| {
        store.read(&reference.digest, reference.size, limit)
    })
}

/// Verify an object is present and intact within the caller's portable bound.
pub fn verify(repo: &super::store::Repository, reference: &ObjectRef) -> Result<(), ClewError> {
    if get(repo, reference, check::PORTABLE_CACHE_MAX_BYTES)?.is_none() {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "documentation object reference is missing from the store",
        ));
    }
    Ok(())
}

/// Enumerate immutable SQLite payload identities with an explicit bound.
pub fn owned_digests(
    repo: &super::store::Repository,
    limit: usize,
) -> Result<Vec<String>, ClewError> {
    Ok(owned_objects(repo, limit)?.into_keys().collect())
}

pub fn owned_objects(
    repo: &super::store::Repository,
    limit: usize,
) -> Result<std::collections::BTreeMap<String, u64>, ClewError> {
    super::object_layout::with_store(repo, |store| {
        Ok(store.enumerate(limit)?.into_iter().collect())
    })
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
    pub cacheability: String,
    #[serde(default)]
    pub reason: Option<String>,
}

pub const REUSABLE: &str = "REUSABLE";
pub const NON_CACHEABLE: &str = "NON_CACHEABLE";

fn validate_capture(manifest: &CaptureManifest) -> Result<(), ClewError> {
    let valid_cacheability = match manifest.cacheability.as_str() {
        REUSABLE => manifest.reason.is_none(),
        NON_CACHEABLE => manifest
            .reason
            .as_ref()
            .is_some_and(|reason| !reason.trim().is_empty()),
        _ => false,
    };
    if manifest.schema != CAPTURE_MANIFEST_SCHEMA
        || manifest.sources.schema != SOURCES_OBJECT_SCHEMA
        || manifest.observations.schema != super::fact_index::FACT_INDEX_SCHEMA
        || manifest.contracts.schema != CONTRACTS_OBJECT_SCHEMA
        || !valid_cacheability
    {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "unsupported capture envelope or cacheability authority; saved evidence cannot be reused",
        ));
    }
    Ok(())
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
    // Share canonical observation payloads with the Check dependency index.
    // The root is immutable and exact; removed whole-map references are rejected.
    let observations = super::fact_index::store_dependency_map(repo, &evidence.observations)?;
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
    validate_capture(manifest)?;
    let limit = check::PORTABLE_CACHE_MAX_BYTES;
    let sources = get_json(repo, &manifest.sources, limit)?.ok_or_else(|| {
        ClewError::new(ErrorCode::StateCorrupt, "capture sources object is missing")
    })?;
    let observations = match manifest.observations.schema.as_str() {
        super::fact_index::FACT_INDEX_SCHEMA => super::fact_index::load_snapshot_observations(
            repo,
            &manifest.observations,
            super::check::CHECK_DEPENDENCIES_SCOPE,
        )?,
        _ => {
            return Err(invalid(
                "unsupported capture observation storage schema; reindex sources",
            ));
        }
    };
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
        serde_json::from_slice(&std::fs::read(&path).map_err(io_error)?).map_err(|error| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                format!("invalid saved capture envelope: {error}"),
            )
        })?;
    validate_capture(&manifest)?;
    if manifest.service != service_id {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "saved capture service does not match its cache selection",
        ));
    }
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

/// Read-only docs cache inventory: conservative bounded accounting that
/// distinguishes disposable immutable objects from retained publication
/// evidence and the current root. This never deletes or migrates data and
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
    let objects = owned_objects(repo, usize::MAX)?;
    let object_count = objects.len();
    let object_bytes = objects.values().sum();
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
    fn new_object_put_get_uses_sqlite_only() {
        let (_t, repo) = setup();
        let reference = put(&repo, "schema/evidence/1", b"sqlite payload").unwrap();
        assert!(
            !repo
                .root
                .join(OBJECT_ROOT)
                .join(&reference.digest)
                .join("object.json")
                .exists()
        );
        assert_eq!(
            get(&repo, &reference, 1024).unwrap(),
            Some(b"sqlite payload".to_vec())
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
        let marker = repo.root.join(".codeclew/cache/object-layout.json");
        let layout: serde_json::Value = serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
        let database = repo.root.join(layout["database"].as_str().unwrap());
        let connection = rusqlite::Connection::open(database).unwrap();
        connection
            .execute(
                "UPDATE objects SET payload = ?1 WHERE digest = ?2",
                rusqlite::params![b"tampered".as_slice(), reference.digest],
            )
            .unwrap();
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
        // Two manifests share sources, the empty immutable observation index,
        // and the empty contracts object.
        let digests = owned_digests(&repo, 1000).unwrap();
        assert_eq!(digests.len(), 3, "deduplicated heavy payload objects");
        // A missing payload object is a corrupt-state error, never forged.
        let mut dangling = a.clone();
        dangling.sources.digest = format!("sha256:{}", "c".repeat(64));
        let error = load_capture(&repo, &dangling).unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    #[test]
    fn capture_reuse_requires_explicit_current_envelope_authority() {
        let (_t, repo) = setup();
        let key = format!("sha256:{}", "a".repeat(64));
        let manifest = store_capture(&repo, &evidence("svc")).unwrap();
        let current = serde_json::to_value(&manifest).unwrap();
        let path = capture_manifest_path(&repo, "svc", &key).unwrap();
        let mut variants = Vec::new();
        let mut missing = current.clone();
        missing.as_object_mut().unwrap().remove("cacheability");
        variants.push(missing);
        for (field, value) in [
            ("schema", "codeclew-documentation-capture-manifest/0.9"),
            ("cacheability", "UNKNOWN"),
            ("cacheability", NON_CACHEABLE),
            ("service", "another-service"),
        ] {
            let mut invalid = current.clone();
            invalid[field] = serde_json::json!(value);
            variants.push(invalid);
        }
        for field in ["sources", "observations", "contracts"] {
            let mut invalid = current.clone();
            invalid[field]["schema"] = serde_json::json!("unsupported-object/0.9");
            variants.push(invalid);
        }
        let mut contradictory = current.clone();
        contradictory["reason"] = serde_json::json!("authority is incomplete");
        variants.push(contradictory);
        for invalid in variants {
            let bytes = serde_json::to_vec(&invalid).unwrap();
            fs::write(&path, &bytes).unwrap();
            // Invalid authority must fail, never become a miss that repeats analysis.
            assert_eq!(
                load_capture_if_valid(&repo, "svc", &key).unwrap_err().code,
                ErrorCode::StateCorrupt,
                "{invalid}"
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::write(&path, serde_json::to_vec(&current).unwrap()).unwrap();
        assert!(load_capture_if_valid(&repo, "svc", &key).unwrap().is_some());
        let mut non_cacheable = manifest;
        mark_non_cacheable(&mut non_cacheable, "external authority is incomplete");
        fs::write(&path, serde_json::to_vec(&non_cacheable).unwrap()).unwrap();
        assert!(load_capture_if_valid(&repo, "svc", &key).unwrap().is_none());
        // Non-cacheable captures remain valid immutable snapshot evidence.
        assert!(load_capture(&repo, &non_cacheable).is_ok());
        non_cacheable.schema = "unsupported-capture/0.9".into();
        assert_eq!(
            load_capture(&repo, &non_cacheable).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn capture_and_check_share_fact_payloads_and_reject_old_map_schema() {
        let (_t, repo) = setup();
        let mut evidence = evidence("svc");
        let normalized = serde_json::json!({"name":"process","body":"retained"});
        let observation = super::super::model::Observation {
            id: "svc:symbol:process".into(),
            kind: "SYMBOL".into(),
            service: "svc".into(),
            symbol: "process".into(),
            digest: super::super::digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![],
        };
        evidence
            .observations
            .insert(observation.id.clone(), observation);
        let manifest = store_capture(&repo, &evidence).unwrap();
        assert_eq!(
            manifest.observations.schema,
            super::super::fact_index::FACT_INDEX_SCHEMA
        );
        let before = owned_objects(&repo, 1000).unwrap();
        let check_index =
            super::super::fact_index::store_dependency_map(&repo, &evidence.observations).unwrap();
        assert_eq!(check_index, manifest.observations);
        assert_eq!(
            before,
            owned_objects(&repo, 1000).unwrap(),
            "same facts create no extra payloads or pages"
        );
        assert_eq!(
            serde_json::to_value(load_capture(&repo, &manifest).unwrap().observations).unwrap(),
            serde_json::to_value(&evidence.observations).unwrap()
        );
        // Whole-map observation objects are from the removed pre-index format.
        let mut unsupported = manifest.clone();
        unsupported.observations.schema = OBSERVATIONS_OBJECT_SCHEMA.into();
        assert!(load_capture(&repo, &unsupported).is_err());
        let mut broken = manifest;
        broken.observations.digest = format!("sha256:{}", "d".repeat(64));
        assert!(load_capture(&repo, &broken).is_err());
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
        // Read-only: no deletion or migration occurs. Fresh roots use the
        // selected SQLite layout and need not create a legacy object tree.
        assert!(
            repo.root
                .join(".codeclew/cache/object-layout.json")
                .is_file()
        );
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
