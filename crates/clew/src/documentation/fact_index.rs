//! Versioned paged membership/index layer over the docs-local immutable store.
//!
//! This extends the predecessor immutable object store with a membership index
//! without introducing a competing evidence database or an uncached dependency.
//! Payload objects stay content-addressed and stored once; a membership binds a
//! stable contextual occurrence key (repository/revision + source state +
//! compilation scope + domain + semantic fact identity) to a payload `ObjectRef`.
//! Occurrence keys are scope-aware, so identical symbol strings under
//! incompatible scopes never collide or overwrite, and byte-identical payloads
//! shared across memberships are persisted once.
//!
//! Entries are partitioned into a bounded number of bucket pages by a
//! deterministic hash of the occurrence key. A point lookup reads exactly one
//! bucket page; an upsert/remove copies only the affected page and the small
//! root manifest, sharing every unchanged page/object. The current root is
//! published by one atomic write under the existing docs lock. Roots are
//! verified from immutable membership objects and fail closed on corruption.

use super::{cache, invalid, store::Repository};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::fs;

#[cfg(test)]
use std::cell::RefCell;

pub const FACT_INDEX_SCHEMA: &str = "codeclew-documentation-fact-index/1.0";
pub const FACT_PAGE_SCHEMA: &str = "codeclew-documentation-fact-page/1.0";
/// Bounded bucket count: point lookups and updates touch one page plus the
/// small root manifest, never every payload or an unbounded catalog.
pub const BUCKETS: usize = 64;

#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageReadStats {
    pub attempts: usize,
    pub bytes: u64,
}

#[cfg(test)]
thread_local! {
    static PAGE_READ_STATS: RefCell<PageReadStats> = RefCell::new(PageReadStats::default());
}

#[cfg(test)]
pub(crate) fn reset_page_read_stats() {
    PAGE_READ_STATS.with(|stats| *stats.borrow_mut() = PageReadStats::default());
}

#[cfg(test)]
pub(crate) fn page_read_stats() -> PageReadStats {
    PAGE_READ_STATS.with(|stats| *stats.borrow())
}

/// Path of the journaled/atomic current membership root.
const ROOT_PATH: &str = ".codeclew/cache/fact-index.json";

/// Stable contextual occurrence identity. `scope` is the compilation scope;
/// `semantic` is the domain/semantic fact identity. Two occurrences are
/// distinct unless every field (including scope) agrees, so same-symbol
/// candidates under different scopes are independent memberships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccurrenceKey {
    pub repository: String,
    pub revision: String,
    pub source_state: String,
    pub scope: String,
    pub domain: String,
    pub semantic: String,
}

impl OccurrenceKey {
    /// A deterministic canonical identity string used for bucketing and
    /// equality. Field order is fixed by the struct, so the serialization is
    /// stable across processes.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// One contextual membership: an occurrence key bound to a payload object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactOccurrence {
    pub key: OccurrenceKey,
    pub payload: cache::ObjectRef,
    pub kind: String,
    pub file: String,
    pub symbol: String,
}

/// A bounded bucket page, stored as an immutable content-addressed object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactPage {
    pub schema: String,
    pub protocol: String,
    pub bucket: usize,
    pub entries: Vec<FactOccurrence>,
}

/// Current membership root: bounded page references plus per-bucket counts.
/// A snapshot membership is the set of bucket pages referenced by this root;
/// it is reconstructible from immutable membership objects, not from untrusted
/// process memory, and never embeds a full catalog of every fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactIndexRoot {
    pub schema: String,
    pub protocol: String,
    pub buckets: Vec<Option<cache::ObjectRef>>,
    pub bucket_counts: Vec<u64>,
}

/// The deterministic bucket for a canonical occurrence key.
fn bucket_of(canonical_key: &str) -> usize {
    let hash = crate::canonical::hash_bytes(canonical_key.as_bytes());
    let byte = u8::from_str_radix(hash.get(7..9).unwrap_or("00"), 16).unwrap_or(0);
    (byte as usize) % BUCKETS
}

/// A fresh, empty membership root.
pub fn empty_root() -> FactIndexRoot {
    FactIndexRoot {
        schema: FACT_INDEX_SCHEMA.into(),
        protocol: FACT_INDEX_SCHEMA.into(),
        buckets: vec![None; BUCKETS],
        bucket_counts: vec![0; BUCKETS],
    }
}

/// Read and verify one bucket page. A missing page object is a corrupt store
/// (fail closed), never a forged empty result.
fn read_page(
    repo: &Repository,
    root: &FactIndexRoot,
    bucket: usize,
) -> Result<Option<FactPage>, ClewError> {
    if bucket >= BUCKETS || root.buckets.len() != BUCKETS || root.bucket_counts.len() != BUCKETS {
        return Err(invalid("fact index root has an invalid bucket shape"));
    }
    let Some(reference) = &root.buckets[bucket] else {
        return Ok(None);
    };
    #[cfg(test)]
    PAGE_READ_STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.attempts += 1;
        stats.bytes = stats.bytes.saturating_add(reference.size);
    });
    let payload =
        cache::get(repo, reference, super::check::PORTABLE_CACHE_MAX_BYTES)?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                "fact index page reference is missing from the store",
            )
        })?;
    let page: FactPage =
        serde_json::from_slice(&payload).map_err(|error| invalid(error.to_string()))?;
    if page.schema != FACT_PAGE_SCHEMA
        || page.protocol != FACT_INDEX_SCHEMA
        || page.bucket != bucket
    {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "fact index page identity is inconsistent",
        ));
    }
    Ok(Some(page))
}

/// Verify a root and every referenced bucket page. Corruption or a missing
/// object is an explicit error so a consumer never accepts forged membership.
pub fn verify_root(repo: &Repository, root: &FactIndexRoot) -> Result<(), ClewError> {
    if root.schema != FACT_INDEX_SCHEMA || root.protocol != FACT_INDEX_SCHEMA {
        return Err(invalid("fact index root schema or protocol is invalid"));
    }
    for bucket in 0..BUCKETS {
        let Some(page) = read_page(repo, root, bucket)? else {
            if root.bucket_counts[bucket] != 0 {
                return Err(ClewError::new(
                    ErrorCode::StateCorrupt,
                    "fact index bucket count disagrees with a missing page",
                ));
            }
            continue;
        };
        if page.entries.len() as u64 != root.bucket_counts[bucket] {
            return Err(ClewError::new(
                ErrorCode::StateCorrupt,
                "fact index page entry count disagrees with the root",
            ));
        }
    }
    Ok(())
}

/// Load the current membership root from disk. A missing root is `Ok(None)`;
/// a corrupt or inconsistent root fails closed.
pub fn load_root(repo: &Repository) -> Result<Option<FactIndexRoot>, ClewError> {
    let path = repo.path(ROOT_PATH)?;
    if !path.exists() {
        return Ok(None);
    }
    let root: FactIndexRoot = serde_json::from_slice(&fs::read(&path).map_err(super::io_error)?)
        .map_err(|error| invalid(error.to_string()))?;
    verify_root(repo, &root)?;
    Ok(Some(root))
}

/// Publish a root by one atomic write under the existing docs lock (journaled
/// current-manifest update). The root is verified before it is installed.
pub fn publish_root(repo: &Repository, root: &FactIndexRoot) -> Result<(), ClewError> {
    verify_root(repo, root)?;
    let data = super::bytes(root)?;
    let _lock = repo.lock()?;
    repo.atomic(ROOT_PATH, &data)
}

/// Point lookup: read exactly the single bucket page for `key`.
pub fn lookup(
    repo: &Repository,
    root: &FactIndexRoot,
    key: &OccurrenceKey,
) -> Result<Option<FactOccurrence>, ClewError> {
    let bucket = bucket_of(&key.canonical());
    let Some(page) = read_page(repo, root, bucket)? else {
        return Ok(None);
    };
    Ok(page.entries.into_iter().find(|entry| entry.key == *key))
}

/// Copy-on-write update of exactly one bucket page. `change` receives the
/// current bucket entries and returns the new entries plus whether the page
/// changed. Only the affected page object and the small root manifest change;
/// all other pages and payload objects are shared unchanged.
fn with_bucket(
    repo: &Repository,
    root: &FactIndexRoot,
    key: &OccurrenceKey,
    change: impl FnOnce(Vec<FactOccurrence>) -> Vec<FactOccurrence>,
) -> Result<FactIndexRoot, ClewError> {
    let bucket = bucket_of(&key.canonical());
    let existing = read_page(repo, root, bucket)?
        .map(|page| page.entries)
        .unwrap_or_default();
    let new_entries = change(existing);
    let count = new_entries.len() as u64;
    let page = FactPage {
        schema: FACT_PAGE_SCHEMA.into(),
        protocol: FACT_INDEX_SCHEMA.into(),
        bucket,
        entries: new_entries,
    };
    let reference = cache::put(
        repo,
        FACT_PAGE_SCHEMA,
        &serde_json::to_vec(&page).map_err(|error| invalid(error.to_string()))?,
    )?;
    let mut next = root.clone();
    next.buckets[bucket] = Some(reference);
    next.bucket_counts[bucket] = count;
    Ok(next)
}

/// Add or replace the membership for `key` bound to `payload`. Existing
/// memberships with the same key are replaced; payloads are content-addressed,
/// so a byte-identical payload shared across memberships is stored once.
pub fn put_membership(
    repo: &Repository,
    root: &FactIndexRoot,
    membership: FactOccurrence,
) -> Result<FactIndexRoot, ClewError> {
    let key = membership.key.clone();
    with_bucket(repo, root, &key, |mut entries| {
        entries.retain(|entry| entry.key != key);
        entries.push(membership);
        entries
    })
}

/// Remove the membership for `key`. A missing key is a no-op (returns an
/// equivalent root). This is an immutable index change: the payload object is
/// left in place for other memberships and historical roots.
pub fn remove_membership(
    repo: &Repository,
    root: &FactIndexRoot,
    key: &OccurrenceKey,
) -> Result<FactIndexRoot, ClewError> {
    with_bucket(repo, root, key, |entries| {
        entries
            .into_iter()
            .filter(|entry| entry.key != *key)
            .collect()
    })
}

/// A fact-granular snapshot transaction: apply a set of membership upserts and
/// removals to produce a new current root. Changes are grouped by bucket so
/// each affected bucket page is rewritten exactly once (plus the small root
/// manifest); unchanged payloads/pages are reused and untouched payloads are
/// never read or rewritten. The old root remains a valid historical snapshot.
pub fn apply_delta(
    repo: &Repository,
    root: &FactIndexRoot,
    upserts: &[FactOccurrence],
    removals: &[OccurrenceKey],
) -> Result<FactIndexRoot, ClewError> {
    // Group changes by bucket to rewrite each affected page once.
    let mut upserts_by_bucket: Vec<Vec<FactOccurrence>> = vec![Vec::new(); BUCKETS];
    let mut removals_by_bucket: Vec<Vec<OccurrenceKey>> = vec![Vec::new(); BUCKETS];
    let mut touched: std::collections::BTreeSet<usize> = Default::default();
    for membership in upserts {
        let bucket = bucket_of(&membership.key.canonical());
        upserts_by_bucket[bucket].push(membership.clone());
        touched.insert(bucket);
    }
    for key in removals {
        let bucket = bucket_of(&key.canonical());
        removals_by_bucket[bucket].push(key.clone());
        touched.insert(bucket);
    }
    let mut next = root.clone();
    for bucket in touched {
        let existing = read_page(repo, root, bucket)?
            .map(|page| page.entries)
            .unwrap_or_default();
        let mut entries = existing;
        for key in &removals_by_bucket[bucket] {
            entries.retain(|entry| entry.key != *key);
        }
        for membership in &upserts_by_bucket[bucket] {
            entries.retain(|entry| entry.key != membership.key);
            entries.push(membership.clone());
        }
        let count = entries.len() as u64;
        let page = FactPage {
            schema: FACT_PAGE_SCHEMA.into(),
            protocol: FACT_INDEX_SCHEMA.into(),
            bucket,
            entries,
        };
        let reference = cache::put(
            repo,
            FACT_PAGE_SCHEMA,
            &serde_json::to_vec(&page).map_err(|error| invalid(error.to_string()))?,
        )?;
        next.buckets[bucket] = Some(reference);
        next.bucket_counts[bucket] = count;
    }
    Ok(next)
}

/// Replace the authoritative complete fact set for one scope. `complete` must
/// be the full, successfully analyzed membership set for `scope`. When
/// `complete_analysis` is false the caller asserts the analysis was NOT
/// complete: no previously known fact for that scope may be deleted (only
/// upserts apply), so an incomplete capture can never pretend to be a complete
/// replacement and erase known facts. Deletion is authorized only by a
/// successful complete replacement or an explicit removal request.
pub fn replace_scope(
    repo: &Repository,
    root: &FactIndexRoot,
    scope: &str,
    complete: &[FactOccurrence],
    complete_analysis: bool,
) -> Result<FactIndexRoot, ClewError> {
    if !complete_analysis {
        // Incomplete analysis: only upsert; never delete previously known facts.
        return apply_delta(repo, root, complete, &[]);
    }
    // Complete replacement: drop every current membership for `scope` that is
    // not in the provided complete set, then upsert the complete set.
    let removals: Vec<OccurrenceKey> = root
        .buckets
        .iter()
        .enumerate()
        .filter(|(_, reference)| reference.is_some())
        .flat_map(|(bucket, _)| {
            read_page(repo, root, bucket)
                .ok()
                .flatten()
                .map(|page| page.entries)
                .unwrap_or_default()
        })
        .filter(|entry| entry.key.scope == scope)
        .map(|entry| entry.key)
        .collect();
    apply_delta(repo, root, complete, &removals)
}

/// Read-only observability of logical removal: how many owned payload objects
/// are unreferenced by the current root, and their bytes. This never deletes
/// anything; physical reclamation, pack compaction and expiry remain deferred.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReclaimReport {
    pub schema: String,
    pub owned_payloads: usize,
    pub referenced_payloads: usize,
    pub unreferenced_payloads: usize,
    pub reclaimable_bytes: u64,
}

pub const RECLAIM_SCHEMA: &str = "codeclew-documentation-fact-reclaim/1.0";

/// Compute the set of payload object digests referenced by the current root's
/// memberships, then report owned-but-unreferenced payloads and bytes. This is
/// an inventory-style read-only reachability check, not a deletion.
pub fn reclaimable(repo: &Repository, root: &FactIndexRoot) -> Result<ReclaimReport, ClewError> {
    let mut referenced_payloads = std::collections::BTreeSet::new();
    // Reachability compares owned objects against the union of bucket page
    // objects and the membership payloads they bind.
    let mut reachable = std::collections::BTreeSet::new();
    for reference in root.buckets.iter().flatten() {
        reachable.insert(reference.digest.clone());
    }
    for bucket in 0..BUCKETS {
        if let Some(page) = read_page(repo, root, bucket)? {
            for entry in page.entries {
                referenced_payloads.insert(entry.payload.digest.clone());
                reachable.insert(entry.payload.digest.clone());
            }
        }
    }
    let owned = cache::owned_digests(repo, 8192)?;
    let mut unreferenced_bytes = 0u64;
    let mut unreferenced = 0usize;
    for digest in &owned {
        if !reachable.contains(digest) {
            unreferenced += 1;
            let dir = repo.root.join(cache::OBJECT_ROOT).join(digest);
            if let Ok(metadata) = fs::metadata(dir.join("object.json")) {
                unreferenced_bytes += metadata.len();
            }
        }
    }
    Ok(ReclaimReport {
        schema: RECLAIM_SCHEMA.into(),
        owned_payloads: owned.len(),
        referenced_payloads: referenced_payloads.len(),
        unreferenced_payloads: unreferenced,
        reclaimable_bytes: unreferenced_bytes,
    })
}

/// The membership domain under which documentation observations are indexed.
pub const OBSERVATION_DOMAIN: &str = "documentation:observation";
/// Immutable object schema for one observation payload referenced by a fact
/// index membership.
pub const OBSERVATION_OBJECT_SCHEMA: &str = "codeclew-documentation-observation/1.0";

/// Store the exact dependency map of one Check, independently of the mutable
/// generic fact index. Keys identify stable map slots; the immutable root binds
/// their complete payloads. Source revision and input authority belong to the
/// enclosing Check manifest, not to this storage identity. In particular, a
/// declaration-only edit must not re-key every unchanged observation.
///
/// Starting from an empty root also makes removals exact and avoids importing
/// unrelated ambient scopes. Deterministic BTreeMap iteration reproduces the
/// same pages for unchanged buckets. This still serializes the complete map;
/// it avoids storage amplification, not all O(N) composition work.
pub(super) fn store_dependency_map(
    repo: &Repository,
    observations: &std::collections::BTreeMap<String, super::model::Observation>,
) -> Result<cache::ObjectRef, ClewError> {
    let mut memberships = Vec::with_capacity(observations.len());
    for (id, observation) in observations {
        memberships.push(FactOccurrence {
            key: OccurrenceKey {
                repository: observation.service.clone(),
                revision: "check-map/1.0".into(),
                source_state: "CHECK_MAP_SLOT_V1".into(),
                scope: super::check::CHECK_DEPENDENCIES_SCOPE.into(),
                domain: OBSERVATION_DOMAIN.into(),
                semantic: id.clone(),
            },
            payload: cache::put_json(repo, OBSERVATION_OBJECT_SCHEMA, observation)?,
            kind: observation.kind.clone(),
            file: String::new(),
            symbol: observation.symbol.clone(),
        });
    }
    let root = apply_delta(repo, &empty_root(), &memberships, &[])?;
    put_snapshot_root(repo, &root)
}

/// Store a `FactIndexRoot` as an immutable snapshot object so a manifest can
/// reference a specific historical root without depending on the single current
/// `.codeclew/cache/fact-index.json` path.
pub fn put_snapshot_root(
    repo: &Repository,
    root: &FactIndexRoot,
) -> Result<cache::ObjectRef, ClewError> {
    verify_root(repo, root)?;
    cache::put_json(repo, FACT_INDEX_SCHEMA, root)
}

/// Load and verify an immutable snapshot root from an object reference.
pub fn load_snapshot_root(
    repo: &Repository,
    reference: &cache::ObjectRef,
) -> Result<FactIndexRoot, ClewError> {
    let payload =
        cache::get(repo, reference, super::check::PORTABLE_CACHE_MAX_BYTES)?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                "fact index snapshot root is missing",
            )
        })?;
    let root: FactIndexRoot =
        serde_json::from_slice(&payload).map_err(|error| invalid(error.to_string()))?;
    verify_root(repo, &root)?;
    Ok(root)
}

/// Load one immutable snapshot root and one observation scope while reading
/// each referenced bucket page exactly once. Every page is verified before any
/// selected payload is decoded, preserving the fail-closed ordering of
/// `load_snapshot_root` followed by `load_observations` without rereading the
/// page objects.
pub fn load_snapshot_observations(
    repo: &Repository,
    reference: &cache::ObjectRef,
    scope: &str,
) -> Result<std::collections::BTreeMap<String, super::model::Observation>, ClewError> {
    let payload =
        cache::get(repo, reference, super::check::PORTABLE_CACHE_MAX_BYTES)?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                "fact index snapshot root is missing",
            )
        })?;
    let root: FactIndexRoot =
        serde_json::from_slice(&payload).map_err(|error| invalid(error.to_string()))?;
    if root.schema != FACT_INDEX_SCHEMA || root.protocol != FACT_INDEX_SCHEMA {
        return Err(invalid("fact index root schema or protocol is invalid"));
    }

    let mut selected = Vec::new();
    for bucket in 0..BUCKETS {
        let Some(page) = read_page(repo, &root, bucket)? else {
            if root.bucket_counts[bucket] != 0 {
                return Err(ClewError::new(
                    ErrorCode::StateCorrupt,
                    "fact index bucket count disagrees with a missing page",
                ));
            }
            continue;
        };
        if page.entries.len() as u64 != root.bucket_counts[bucket] {
            return Err(ClewError::new(
                ErrorCode::StateCorrupt,
                "fact index page entry count disagrees with the root",
            ));
        }
        selected.extend(
            page.entries
                .into_iter()
                .filter(|entry| entry.key.domain == OBSERVATION_DOMAIN && entry.key.scope == scope)
                .map(|entry| (entry.key.semantic, entry.payload)),
        );
    }

    let mut out = std::collections::BTreeMap::new();
    for (semantic, reference) in selected {
        let payload = cache::get(repo, &reference, super::check::PORTABLE_CACHE_MAX_BYTES)?
            .ok_or_else(|| {
                ClewError::new(ErrorCode::StateCorrupt, "observation payload is missing")
            })?;
        let observation: super::model::Observation =
            serde_json::from_slice(&payload).map_err(|error| invalid(error.to_string()))?;
        out.insert(semantic, observation);
    }
    Ok(out)
}

/// Store a scope's observations as per-fact memberships and return the
/// immutable snapshot root object. Identical canonical payload bytes are stored
/// once and shared across occurrences; occurrences remain distinct by
/// compilation scope and observation identity. A one-scope change rewrites only
/// the affected bucket page and the small root.
pub fn store_observations(
    repo: &Repository,
    scope: &str,
    revision: &str,
    observations: &std::collections::BTreeMap<String, super::model::Observation>,
) -> Result<cache::ObjectRef, ClewError> {
    let base = load_root(repo)?.unwrap_or_else(empty_root);
    let mut upserts = Vec::with_capacity(observations.len());
    for (id, observation) in observations {
        let payload = cache::put_json(repo, OBSERVATION_OBJECT_SCHEMA, observation)?;
        upserts.push(FactOccurrence {
            key: OccurrenceKey {
                repository: observation.service.clone(),
                revision: revision.to_string(),
                source_state: "INDEXED".into(),
                scope: scope.to_string(),
                domain: OBSERVATION_DOMAIN.into(),
                semantic: id.clone(),
            },
            payload,
            kind: observation.kind.clone(),
            file: String::new(),
            symbol: observation.symbol.clone(),
        });
    }
    // This snapshot contains exactly the supplied observation set for this
    // scope. Unrelated scopes remain shared, but previous same-scope members
    // must not leak into a subsequently hydrated frozen Check.
    let next = replace_scope(repo, &base, scope, &upserts, true)?;
    put_snapshot_root(repo, &next)
}

/// Load a scope's observations from a snapshot root, decoding only the
/// matching memberships' payloads. Point reads use `lookup` (one bucket page);
/// this scope-level read traverses bucket page objects but never decodes
/// unrelated payloads.
pub fn load_observations(
    repo: &Repository,
    root: &FactIndexRoot,
    scope: &str,
) -> Result<std::collections::BTreeMap<String, super::model::Observation>, ClewError> {
    let mut out = std::collections::BTreeMap::new();
    for bucket in 0..BUCKETS {
        let Some(page) = read_page(repo, root, bucket)? else {
            continue;
        };
        for entry in page.entries {
            if entry.key.domain != OBSERVATION_DOMAIN || entry.key.scope != scope {
                continue;
            }
            let payload = cache::get(repo, &entry.payload, super::check::PORTABLE_CACHE_MAX_BYTES)?
                .ok_or_else(|| {
                    ClewError::new(ErrorCode::StateCorrupt, "observation payload is missing")
                })?;
            let observation: super::model::Observation =
                serde_json::from_slice(&payload).map_err(|error| invalid(error.to_string()))?;
            out.insert(entry.key.semantic, observation);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::store::Repository;
    use serde_json::json;
    use std::{collections::BTreeMap, fs};

    fn setup() -> (tempfile::TempDir, Repository) {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "Architecture").unwrap();
        let repo = Repository::open(t.path()).unwrap();
        (t, repo)
    }

    fn key(scope: &str, semantic: &str) -> OccurrenceKey {
        OccurrenceKey {
            repository: "orders".into(),
            revision: "a".repeat(40),
            source_state: "EXACT_SNAPSHOT_TEXT".into(),
            scope: scope.into(),
            domain: "analysis:java-compiler-facts".into(),
            semantic: semantic.into(),
        }
    }

    fn payload(repo: &Repository, bytes: &[u8]) -> cache::ObjectRef {
        cache::put(repo, "schema/payload/1", bytes).unwrap()
    }

    fn observation(id: &str, value: &str) -> super::super::model::Observation {
        let normalized = json!({"value":value});
        super::super::model::Observation {
            id: id.into(),
            kind: "SYNTHETIC_FACT".into(),
            service: "orders".into(),
            symbol: id.into(),
            digest: crate::canonical::hash(&normalized).unwrap(),
            normalized,
            source_ids: Vec::new(),
        }
    }

    fn populated_snapshot(repo: &Repository) -> (cache::ObjectRef, FactIndexRoot) {
        let mut first = BTreeMap::new();
        let mut second = BTreeMap::new();
        for index in 0..96 {
            first.insert(
                format!("scope-a-{index}"),
                observation(&format!("scope-a-{index}"), &format!("a-{index}")),
            );
            second.insert(
                format!("scope-b-{index}"),
                observation(&format!("scope-b-{index}"), &format!("b-{index}")),
            );
        }
        let first_snapshot = store_observations(repo, "scope-a", "revision-a", &first).unwrap();
        let first_root = load_snapshot_root(repo, &first_snapshot).unwrap();
        publish_root(repo, &first_root).unwrap();
        let snapshot = store_observations(repo, "scope-b", "revision-a", &second).unwrap();
        let root = load_snapshot_root(repo, &snapshot).unwrap();
        assert!(
            root.buckets
                .iter()
                .filter(|bucket| bucket.is_some())
                .count()
                >= 2
        );
        (snapshot, root)
    }

    #[test]
    fn saved_observation_scope_excludes_old_members_without_mutating_current_root() {
        let (_t, repo) = setup();
        let old = BTreeMap::from([("old".into(), observation("old", "old value"))]);
        let first = store_observations(&repo, "scope-a", "revision-old", &old).unwrap();
        let old_root = load_snapshot_root(&repo, &first).unwrap();
        publish_root(&repo, &old_root).unwrap();
        let current_before = fs::read(repo.root.join(ROOT_PATH)).unwrap();
        let fresh = BTreeMap::from([("fresh".into(), observation("fresh", "new value"))]);
        let next = store_observations(&repo, "scope-a", "revision-new", &fresh).unwrap();
        assert_eq!(
            serde_json::to_value(load_snapshot_observations(&repo, &next, "scope-a").unwrap())
                .unwrap(),
            serde_json::to_value(&fresh).unwrap()
        );
        assert_eq!(
            serde_json::to_value(load_snapshot_observations(&repo, &first, "scope-a").unwrap())
                .unwrap(),
            serde_json::to_value(&old).unwrap()
        );
        assert_eq!(fs::read(repo.root.join(ROOT_PATH)).unwrap(), current_before);
        let empty = store_observations(&repo, "scope-a", "revision-new", &BTreeMap::new()).unwrap();
        assert!(
            load_snapshot_observations(&repo, &empty, "scope-a")
                .unwrap()
                .is_empty()
        );
    }

    fn snapshot_for_root(repo: &Repository, root: &FactIndexRoot) -> cache::ObjectRef {
        cache::put_json(repo, FACT_INDEX_SCHEMA, root).unwrap()
    }

    fn page(repo: &Repository, reference: &cache::ObjectRef) -> FactPage {
        serde_json::from_slice(
            &cache::get(
                repo,
                reference,
                super::super::check::PORTABLE_CACHE_MAX_BYTES,
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap()
    }

    /// A payload bound under several distinct contextual memberships is stored
    /// once, and the page objects are bounded by the bucket count.
    #[test]
    fn one_payload_many_memberships_stored_once() {
        let (_t, repo) = setup();
        let root = empty_root();
        let shared = payload(&repo, b"shared fact bytes");
        let root = put_membership(
            &repo,
            &root,
            FactOccurrence {
                key: key(":/web:main", "example.Common"),
                payload: shared.clone(),
                kind: "SYMBOL".into(),
                file: "web/Common.java".into(),
                symbol: "example.Common".into(),
            },
        )
        .unwrap();
        let root = put_membership(
            &repo,
            &root,
            FactOccurrence {
                key: key(":/common:main", "example.Common"),
                payload: shared.clone(),
                kind: "SYMBOL".into(),
                file: "common/Common.java".into(),
                symbol: "example.Common".into(),
            },
        )
        .unwrap();
        verify_root(&repo, &root).unwrap();
        // Two distinct contextual memberships reference the SAME payload object
        // (content identity), which is stored exactly once.
        let digests = cache::owned_digests(&repo, 1000).unwrap();
        let payload_objects = digests.iter().filter(|d| **d == shared.digest).count();
        assert_eq!(payload_objects, 1, "a payload is persisted once");
        let a = lookup(&repo, &root, &key(":/web:main", "example.Common"))
            .unwrap()
            .unwrap();
        let b = lookup(&repo, &root, &key(":/common:main", "example.Common"))
            .unwrap()
            .unwrap();
        assert_eq!(a.payload, shared);
        assert_eq!(b.payload, shared);
    }

    /// Identical symbol strings under incompatible scopes are distinct
    /// memberships that never collide or overwrite.
    #[test]
    fn same_symbol_different_scope_does_not_collide() {
        let (_t, repo) = setup();
        let root = empty_root();
        let root = put_membership(
            &repo,
            &root,
            FactOccurrence {
                key: key(":/web:main", "example.Common"),
                payload: payload(&repo, b"web candidate"),
                kind: "SYMBOL".into(),
                file: "web/Common.java".into(),
                symbol: "example.Common".into(),
            },
        )
        .unwrap();
        let root = put_membership(
            &repo,
            &root,
            FactOccurrence {
                key: key(":/common:main", "example.Common"),
                payload: payload(&repo, b"common candidate"),
                kind: "SYMBOL".into(),
                file: "common/Common.java".into(),
                symbol: "example.Common".into(),
            },
        )
        .unwrap();
        let web = lookup(&repo, &root, &key(":/web:main", "example.Common"))
            .unwrap()
            .unwrap();
        let common = lookup(&repo, &root, &key(":/common:main", "example.Common"))
            .unwrap()
            .unwrap();
        assert_eq!(web.payload.digest, payload(&repo, b"web candidate").digest);
        assert_ne!(
            web.payload.digest, common.payload.digest,
            "incompatible scopes must retain distinct candidate payloads"
        );
        // A full overwrite of one scope does not touch the other scope's entry.
        let root = put_membership(
            &repo,
            &root,
            FactOccurrence {
                key: key(":/web:main", "example.Common"),
                payload: payload(&repo, b"web v2"),
                kind: "SYMBOL".into(),
                file: "web/Common.java".into(),
                symbol: "example.Common".into(),
            },
        )
        .unwrap();
        assert_eq!(
            lookup(&repo, &root, &key(":/web:main", "example.Common"))
                .unwrap()
                .unwrap()
                .payload
                .digest,
            payload(&repo, b"web v2").digest
        );
        assert_eq!(
            lookup(&repo, &root, &key(":/common:main", "example.Common"))
                .unwrap()
                .unwrap()
                .payload
                .digest,
            common.payload.digest,
            "the other scope's membership must survive"
        );
    }

    /// An update changes only the affected bucket page and the small root
    /// manifest, not other pages or any payload; removal is a bounded index
    /// change and leaves payload objects in place.
    #[test]
    fn update_touches_only_affected_bucket_and_removal_is_bounded() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        // Spread memberships across buckets.
        for i in 0..20 {
            let k = key(":/main", &format!("symbol{i}"));
            root = put_membership(
                &repo,
                &root,
                FactOccurrence {
                    key: k,
                    payload: payload(&repo, format!("payload{i}").as_bytes()),
                    kind: "SYMBOL".into(),
                    file: "f.java".into(),
                    symbol: format!("symbol{i}"),
                },
            )
            .unwrap();
        }
        // Capture the set of page object digests referenced before the change.
        let before_digests: Vec<String> = root
            .buckets
            .iter()
            .flatten()
            .map(|r| r.digest.clone())
            .collect();

        // Remove one membership; only its bucket page and the root change.
        let target = key(":/main", "symbol5");
        let after = remove_membership(&repo, &root, &target).unwrap();
        let after_digests: Vec<String> = after
            .buckets
            .iter()
            .flatten()
            .map(|r| r.digest.clone())
            .collect();
        let changed = before_digests
            .iter()
            .filter(|d| !after_digests.contains(d))
            .count()
            + after_digests
                .iter()
                .filter(|d| !before_digests.contains(d))
                .count();
        assert_eq!(
            changed, 2,
            "removal must change exactly one page object (old+new), not every page"
        );
        verify_root(&repo, &after).unwrap();
        assert!(
            lookup(&repo, &after, &target).unwrap().is_none(),
            "the removed membership must no longer resolve"
        );
        // The payload object for symbol5 is still present (other roots may use it).
        assert!(
            cache::owned_digests(&repo, 1000)
                .unwrap()
                .iter()
                .any(|d| *d == payload(&repo, b"payload5").digest),
            "logical removal must not physically delete the payload"
        );
    }

    /// Corrupting a referenced page object fails closed on load/verify.
    #[test]
    fn corrupt_page_fails_closed() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        root = put_membership(
            &repo,
            &root,
            FactOccurrence {
                key: key(":/main", "symbol0"),
                payload: payload(&repo, b"p0"),
                kind: "SYMBOL".into(),
                file: "f.java".into(),
                symbol: "symbol0".into(),
            },
        )
        .unwrap();
        publish_root(&repo, &root).unwrap();
        // Tamper with the referenced page object in the store.
        let page_ref = root.buckets[bucket_of(&key(":/main", "symbol0").canonical())]
            .clone()
            .unwrap();
        let dir = repo.root.join(cache::OBJECT_ROOT).join(&page_ref.digest);
        fs::write(dir.join("object.json"), b"tampered").unwrap();
        let error = load_root(&repo).unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    /// The current root round-trips through publish/load and stays consistent.
    #[test]
    fn root_round_trips_through_atomic_publish() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        for i in 0..5 {
            root = put_membership(
                &repo,
                &root,
                FactOccurrence {
                    key: key(":/main", &format!("s{i}")),
                    payload: payload(&repo, format!("p{i}").as_bytes()),
                    kind: "SYMBOL".into(),
                    file: "f.java".into(),
                    symbol: format!("s{i}"),
                },
            )
            .unwrap();
        }
        publish_root(&repo, &root).unwrap();
        let loaded = load_root(&repo).unwrap().expect("root present");
        assert_eq!(loaded.bucket_counts, root.bucket_counts);
        for i in 0..5 {
            let got = lookup(&repo, &loaded, &key(":/main", &format!("s{i}")))
                .unwrap()
                .expect("membership present after reload");
            assert_eq!(
                got.payload.digest,
                payload(&repo, format!("p{i}").as_bytes()).digest
            );
        }
    }

    fn mem(repo: &Repository, scope: &str, semantic: &str, bytes: &[u8]) -> FactOccurrence {
        FactOccurrence {
            key: key(scope, semantic),
            payload: payload(repo, bytes),
            kind: "SYMBOL".into(),
            file: "f.java".into(),
            symbol: semantic.into(),
        }
    }

    /// Updating one fact changes only the affected bucket page(s) and root;
    /// unrelated buckets' page references are reused unchanged.
    #[test]
    fn single_fact_update_touches_only_affected_buckets() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        for i in 0..30 {
            root = put_membership(
                &repo,
                &root,
                mem(
                    &repo,
                    ":/web:main",
                    &format!("symbol{i}"),
                    format!("p{i}").as_bytes(),
                ),
            )
            .unwrap();
        }
        let before: Vec<Option<String>> = root
            .buckets
            .iter()
            .map(|r| r.as_ref().map(|r| r.digest.clone()))
            .collect();
        // Upsert one fact (replacing its payload) via a delta.
        let target = key(":/web:main", "symbol15");
        let replacement = FactOccurrence {
            key: target.clone(),
            payload: payload(&repo, b"replaced"),
            kind: "SYMBOL".into(),
            file: "f.java".into(),
            symbol: "symbol15".into(),
        };
        let after = apply_delta(&repo, &root, &[replacement], &[]).unwrap();
        let after_refs: Vec<Option<String>> = after
            .buckets
            .iter()
            .map(|r| r.as_ref().map(|r| r.digest.clone()))
            .collect();
        // Only the bucket holding symbol15 changes (old page gone, new page).
        let changed = before
            .iter()
            .zip(after_refs.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            changed, 1,
            "a single-fact delta must change exactly one bucket page reference"
        );
        assert_eq!(
            lookup(&repo, &after, &target)
                .unwrap()
                .unwrap()
                .payload
                .digest,
            payload(&repo, b"replaced").digest
        );
        // Unrelated buckets reuse the identical page object (no rewrite).
        assert!(
            after_refs.iter().filter(|r| r.is_some()).count() >= 1,
            "other buckets still referenced"
        );
    }

    /// Removing a shared scope from one service leaves other service
    /// memberships and the historical snapshot valid.
    #[test]
    fn removing_shared_scope_keeps_other_scope_and_history_valid() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        root = put_membership(&repo, &root, mem(&repo, ":/web:main", "Web", b"web")).unwrap();
        root = put_membership(
            &repo,
            &root,
            mem(&repo, ":/common:main", "Common", b"common"),
        )
        .unwrap();
        publish_root(&repo, &root).unwrap();
        let historical = root.clone();

        // A complete replacement of the web scope drops web memberships.
        let next = replace_scope(&repo, &root, ":/web:main", &[], true).unwrap();
        assert!(
            lookup(&repo, &next, &key(":/web:main", "Web"))
                .unwrap()
                .is_none(),
            "complete web-scope replacement authorizes web absence"
        );
        assert!(
            lookup(&repo, &next, &key(":/common:main", "Common"))
                .unwrap()
                .is_some(),
            "the shared common scope must survive removal of web"
        );
        // The historical snapshot still resolves both scopes.
        assert!(
            lookup(&repo, &historical, &key(":/web:main", "Web"))
                .unwrap()
                .is_some(),
            "historical snapshot must retain the removed web membership"
        );
    }

    /// An incomplete capture cannot delete previously known facts: replace_scope
    /// with complete_analysis=false only upserts and never removes.
    #[test]
    fn incomplete_capture_cannot_delete_known_facts() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        root = put_membership(&repo, &root, mem(&repo, ":/web:main", "Web", b"web-v1")).unwrap();
        // A partial (incomplete) re-analysis must not erase the known fact.
        let next = replace_scope(&repo, &root, ":/web:main", &[], false).unwrap();
        assert!(
            lookup(&repo, &next, &key(":/web:main", "Web"))
                .unwrap()
                .is_some(),
            "incomplete analysis must not delete previously known facts"
        );
        assert_eq!(
            lookup(&repo, &next, &key(":/web:main", "Web"))
                .unwrap()
                .unwrap()
                .payload
                .digest,
            payload(&repo, b"web-v1").digest
        );
    }

    /// A crash before root publication leaves the old snapshot current; after
    /// publication the on-disk root is the complete new snapshot.
    #[test]
    fn crash_before_publication_leaves_old_snapshot_current() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        root = put_membership(&repo, &root, mem(&repo, ":/web:main", "Web", b"web-v1")).unwrap();
        publish_root(&repo, &root).unwrap();
        assert_eq!(
            load_root(&repo).unwrap().unwrap().bucket_counts,
            root.bucket_counts
        );

        // Compute a new root but never publish it (simulated crash/abort).
        let next =
            put_membership(&repo, &root, mem(&repo, ":/web:main", "Web", b"web-v2")).unwrap();
        let on_disk = load_root(&repo).unwrap().unwrap();
        assert_eq!(
            on_disk.bucket_counts, root.bucket_counts,
            "an unpublished root must not become the current snapshot"
        );
        assert_eq!(
            lookup(&repo, &on_disk, &key(":/web:main", "Web"))
                .unwrap()
                .unwrap()
                .payload
                .digest,
            payload(&repo, b"web-v1").digest,
            "readers observe the old complete snapshot until publication"
        );
        let _ = next;
    }

    /// Logical removal is observable without physical deletion: a replaced
    /// scope's payload is reported unreferenced but remains on disk.
    #[test]
    fn logical_removal_is_observable_without_physical_delete() {
        let (_t, repo) = setup();
        let mut root = empty_root();
        root = put_membership(
            &repo,
            &root,
            mem(&repo, ":/web:main", "Web", b"web-payload"),
        )
        .unwrap();
        root = put_membership(
            &repo,
            &root,
            mem(&repo, ":/common:main", "Common", b"common-payload"),
        )
        .unwrap();
        let web_digest = payload(&repo, b"web-payload").digest;

        let next = replace_scope(&repo, &root, ":/web:main", &[], true).unwrap();
        let report = reclaimable(&repo, &next).unwrap();
        assert!(
            report.unreferenced_payloads >= 1,
            "the replaced web payload must be reported unreferenced: {report:?}"
        );
        assert!(
            report.reclaimable_bytes > 0,
            "reclaimable bytes must be observable"
        );
        // Physical objects are never deleted: the web payload still exists.
        assert!(
            cache::owned_digests(&repo, 8192)
                .unwrap()
                .contains(&web_digest),
            "logical removal must not physically delete the retained object"
        );
    }

    #[test]
    fn single_pass_snapshot_observations_match_legacy_reads_and_read_once() {
        let (_t, repo) = setup();
        let (snapshot, root) = populated_snapshot(&repo);
        let page_bytes: u64 = root
            .buckets
            .iter()
            .flatten()
            .map(|reference| reference.size)
            .sum();
        let bucket_count = root.buckets.iter().flatten().count();

        for scope in ["scope-a", "scope-b", "scope-does-not-exist"] {
            reset_page_read_stats();
            let old_root = load_snapshot_root(&repo, &snapshot).unwrap();
            let old = load_observations(&repo, &old_root, scope).unwrap();
            let old_stats = page_read_stats();

            reset_page_read_stats();
            let new = load_snapshot_observations(&repo, &snapshot, scope).unwrap();
            let new_stats = page_read_stats();

            assert_eq!(new, old, "single-pass output differs for {scope}");
            assert_eq!(old_stats.attempts, bucket_count * 2);
            assert_eq!(new_stats.attempts, bucket_count);
            assert_eq!(old_stats.bytes, page_bytes * 2);
            assert_eq!(new_stats.bytes, page_bytes);
        }
        println!(
            "FACT_INDEX_SNAPSHOT_READ_METRICS {}",
            json!({
                "referencedPages":bucket_count,
                "legacyPageReads":bucket_count * 2,
                "singlePassPageReads":bucket_count,
                "legacyPageBytes":page_bytes * 2,
                "singlePassPageBytes":page_bytes
            })
        );
    }

    #[test]
    fn single_pass_snapshot_rejects_unrelated_corruption_and_root_shape_errors() {
        let (_t, repo) = setup();
        let (snapshot, root) = populated_snapshot(&repo);
        let corrupt_bucket = root
            .buckets
            .iter()
            .enumerate()
            .rev()
            .find_map(|(bucket, reference)| reference.as_ref().map(|_| bucket))
            .unwrap();
        let corrupt_reference = root.buckets[corrupt_bucket].as_ref().unwrap();
        let corrupt_path = repo
            .root
            .join(cache::OBJECT_ROOT)
            .join(&corrupt_reference.digest)
            .join("object.json");
        fs::write(corrupt_path, b"corrupted page").unwrap();
        assert!(
            load_snapshot_observations(&repo, &snapshot, "scope-does-not-exist").is_err(),
            "an empty selected scope must still validate every referenced page"
        );

        let (_t, repo) = setup();
        let (_snapshot, root) = populated_snapshot(&repo);
        let bucket = root
            .buckets
            .iter()
            .position(|reference| reference.is_some())
            .unwrap();
        let mut wrong_count = root.clone();
        wrong_count.bucket_counts[bucket] += 1;
        let wrong_count_ref = snapshot_for_root(&repo, &wrong_count);
        assert!(load_snapshot_observations(&repo, &wrong_count_ref, "scope-a").is_err());

        let mut missing_with_count = root.clone();
        missing_with_count.buckets[bucket] = None;
        missing_with_count.bucket_counts[bucket] = 1;
        let missing_with_count_ref = snapshot_for_root(&repo, &missing_with_count);
        assert!(load_snapshot_observations(&repo, &missing_with_count_ref, "scope-a").is_err());

        let (_t, repo) = setup();
        let (_snapshot, root) = populated_snapshot(&repo);
        let bucket = root
            .buckets
            .iter()
            .position(|reference| reference.is_some())
            .unwrap();
        let missing_reference = root.buckets[bucket].as_ref().unwrap().clone();
        let missing_path = repo
            .root
            .join(cache::OBJECT_ROOT)
            .join(&missing_reference.digest)
            .join("object.json");
        fs::remove_file(missing_path).unwrap();
        let missing_ref = snapshot_for_root(&repo, &root);
        assert!(load_snapshot_observations(&repo, &missing_ref, "scope-a").is_err());

        let (_t, repo) = setup();
        let (_snapshot, root) = populated_snapshot(&repo);
        let bucket = root
            .buckets
            .iter()
            .position(|reference| reference.is_some())
            .unwrap();
        let reference = root.buckets[bucket].as_ref().unwrap();
        let mut bad_page = page(&repo, reference);
        bad_page.schema = "wrong-fact-page-schema".into();
        let bad_page_ref = cache::put_json(&repo, FACT_PAGE_SCHEMA, &bad_page).unwrap();
        let mut bad_schema = root;
        bad_schema.buckets[bucket] = Some(bad_page_ref);
        let bad_schema_ref = snapshot_for_root(&repo, &bad_schema);
        assert!(load_snapshot_observations(&repo, &bad_schema_ref, "scope-a").is_err());
    }

    #[test]
    fn late_page_corruption_precedes_selected_malformed_payload() {
        let (_t, repo) = setup();
        let (_snapshot, root) = populated_snapshot(&repo);
        let first_bucket = root
            .buckets
            .iter()
            .enumerate()
            .filter_map(|(bucket, reference)| {
                reference
                    .as_ref()
                    .map(|reference| (bucket, page(&repo, reference)))
            })
            .find(|(_, page)| {
                page.entries
                    .iter()
                    .any(|entry| entry.key.scope == "scope-a")
            })
            .map(|(bucket, _)| bucket)
            .unwrap();
        let late_bucket = root
            .buckets
            .iter()
            .enumerate()
            .rev()
            .find_map(|(bucket, reference)| {
                (bucket > first_bucket)
                    .then(|| {
                        reference
                            .as_ref()
                            .map(|reference| (bucket, reference.clone()))
                    })
                    .flatten()
            })
            .unwrap();

        let first_reference = root.buckets[first_bucket].as_ref().unwrap();
        let mut first_page = page(&repo, first_reference);
        let selected = first_page
            .entries
            .iter()
            .position(|entry| entry.key.scope == "scope-a")
            .unwrap();
        let malformed_payload = cache::put(
            &repo,
            OBSERVATION_OBJECT_SCHEMA,
            b"not a serialized observation",
        )
        .unwrap();
        first_page.entries[selected].payload = malformed_payload;
        let first_page_ref = cache::put_json(&repo, FACT_PAGE_SCHEMA, &first_page).unwrap();

        let late_reference = late_bucket.1;
        let late_path = repo
            .root
            .join(cache::OBJECT_ROOT)
            .join(&late_reference.digest)
            .join("object.json");
        let mut late_bytes = fs::read(&late_path).unwrap();
        late_bytes[0] ^= 1;
        fs::write(late_path, late_bytes).unwrap();

        let mut forged = root;
        forged.buckets[first_bucket] = Some(first_page_ref);
        let forged_snapshot = snapshot_for_root(&repo, &forged);
        let old_error = load_snapshot_root(&repo, &forged_snapshot)
            .and_then(|root| load_observations(&repo, &root, "scope-a"))
            .unwrap_err();
        let error = load_snapshot_observations(&repo, &forged_snapshot, "scope-a").unwrap_err();
        assert_eq!(error.code, old_error.code);
        assert_eq!(error.message, old_error.message);
        assert!(
            error
                .message
                .contains("content does not match its reference digest"),
            "late page corruption must fail before malformed selected payload decoding: {}",
            error.message
        );
    }
}

#[cfg(test)]
#[path = "dependency_map_tests.rs"]
mod dependency_map_tests;
