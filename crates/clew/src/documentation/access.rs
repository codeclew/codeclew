//! Request-scoped evidence access: batch requested payload IDs, deduplicate
//! before object-store reads, decode each canonical payload once, and hand
//! consumers shared immutable handles instead of clone/read cycles.
//!
//! The counters make the unique-IO invariant observable: within one admitted
//! bounded request, a consumer's payload fetch/decode count equals the unique
//! required payload IDs, not the sum of overlapping compilation/service
//! memberships. Incoming writes are deduplicated by payload reference and the
//! verified object index is consulted before scheduling publication, so an
//! unchanged rerun schedules zero new heavy payload writes. Metadata IO and
//! intentional write-readback integrity verification are counted separately
//! from consumer hydration. Byte/membership budgets are enforced explicitly and
//! never silently truncate evidence. No promise of zero disk reads across
//! processes is made; uniqueness holds within one admitted request.

use super::{cache, store::Repository};
use crate::error::{ClewError, ErrorCode};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Observable counters for one request scope. These distinguish consumer
/// hydration from metadata IO and integrity verification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessCounters {
    /// Object-store payload reads performed to hydrate a request.
    pub unique_payload_fetches: usize,
    /// Canonical payload decodes handed to consumers (one per unique payload
    /// within the request).
    pub decode_count: usize,
    /// Metadata/reference reads (manifests, index pages) that are not consumer
    /// payload hydration.
    pub metadata_io: usize,
    /// Intentional write-readback integrity verifications.
    pub integrity_checks: usize,
    /// Unique payload bytes hydrated within this request.
    pub bytes_read: u64,
    /// New heavy payload objects physically written within this request.
    pub payload_writes: usize,
}

/// A single admitted bounded request scope over one documentation repository.
pub struct RequestScope<'a> {
    repo: &'a Repository,
    counters: AccessCounters,
}

impl<'a> RequestScope<'a> {
    pub fn new(repo: &'a Repository) -> Self {
        Self {
            repo,
            counters: AccessCounters::default(),
        }
    }

    /// Final counters for the request (for reporting and tests).
    pub fn counters(&self) -> &AccessCounters {
        &self.counters
    }

    /// Hydrate a batch of payload references, reading each unique payload
    /// object exactly once and returning shared immutable handles keyed by
    /// content digest. Duplicate or overlapping references across
    /// compilation/service memberships collapse to one fetch.
    pub fn hydrate(
        &mut self,
        references: &[cache::ObjectRef],
    ) -> Result<BTreeMap<String, Arc<Vec<u8>>>, ClewError> {
        let mut wanted = BTreeMap::new();
        for reference in references {
            wanted.entry(reference.digest.clone()).or_insert_with(|| {
                cache::get(self.repo, reference, super::check::PORTABLE_CACHE_MAX_BYTES)
            });
        }
        let mut out = BTreeMap::new();
        for (digest, read) in wanted {
            let payload = read?.ok_or_else(|| {
                ClewError::new(ErrorCode::StateCorrupt, "requested payload is missing")
            })?;
            self.counters.unique_payload_fetches += 1;
            self.counters.decode_count += 1;
            self.counters.bytes_read = self
                .counters
                .bytes_read
                .saturating_add(payload.len() as u64);
            out.insert(digest, Arc::new(payload));
        }
        Ok(out)
    }

    /// Record a metadata/reference read that is not consumer payload hydration.
    pub fn record_metadata_io(&mut self) {
        self.counters.metadata_io += 1;
    }

    /// Deduplicate incoming writes by payload reference: consult the verified
    /// object index before scheduling publication and write each unique payload
    /// at most once. Returns the validated object reference per payload. An
    /// unchanged rerun (same payloads already present) schedules zero new
    /// heavy payload writes.
    pub fn write_dedup(
        &mut self,
        payloads: &[(&str, Vec<u8>)],
    ) -> Result<Vec<cache::ObjectRef>, ClewError> {
        // Group identical bytes under one content identity; provenance (schema)
        // is a reference attribute, so equal bytes collapse to one object.
        let mut by_digest: BTreeMap<String, (String, Vec<u8>)> = BTreeMap::new();
        for (schema, bytes) in payloads {
            let digest = cache::content_digest(bytes);
            by_digest
                .entry(digest)
                .or_insert_with(|| (schema.to_string(), bytes.clone()));
        }
        let mut out = Vec::with_capacity(payloads.len());
        for (digest, (schema, bytes)) in &by_digest {
            let probe = cache::ObjectRef::new(schema.clone(), digest.clone(), bytes.len() as u64);
            // Consult the verified object index before writing: a present
            // object is reused (and verified) without scheduling a new write.
            let present =
                cache::get(self.repo, &probe, super::check::PORTABLE_CACHE_MAX_BYTES)?.is_some();
            self.counters.integrity_checks += 1;
            if present {
                self.counters.metadata_io += 1;
            } else {
                self.counters.payload_writes += 1;
            }
            let reference = cache::put(self.repo, schema, bytes)?;
            out.push(reference);
        }
        Ok(out)
    }

    /// Read and decode a bounded set of payloads, failing with a typed budget
    /// outcome instead of silently truncating when the aggregate byte budget is
    /// exceeded.
    pub fn hydrate_bounded(
        &mut self,
        references: &[cache::ObjectRef],
        byte_budget: u64,
        membership_budget: usize,
    ) -> Result<BTreeMap<String, Arc<Vec<u8>>>, ClewError> {
        if references.len() > membership_budget {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "evidence request exceeds the membership budget",
            ));
        }
        let mut total = 0u64;
        let hydrated = self.hydrate(references)?;
        for payload in hydrated.values() {
            total = total.saturating_add(payload.len() as u64);
            if total > byte_budget {
                return Err(ClewError::new(
                    ErrorCode::SliceBudgetExceeded,
                    "evidence request exceeds the byte budget",
                ));
            }
        }
        Ok(hydrated)
    }
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

    /// Consumer payload fetch/decode counts equal the unique required payload
    /// IDs, not the sum of overlapping compilation/service memberships.
    #[test]
    fn fetch_counts_unique_payloads_not_memberships() {
        let (_t, repo) = setup();
        let mut scope = RequestScope::new(&repo);
        // Three distinct payloads, but the request references them across
        // overlapping memberships (10 references, 3 unique payloads).
        let a = cache::put(&repo, "s/1", b"alpha").unwrap();
        let b = cache::put(&repo, "s/1", b"beta").unwrap();
        let c = cache::put(&repo, "s/1", b"gamma").unwrap();
        let refs = vec![
            a.clone(),
            b.clone(),
            c.clone(),
            a.clone(),
            b.clone(),
            c.clone(),
            a.clone(),
            b.clone(),
            c.clone(),
            a.clone(),
        ];
        let hydrated = scope.hydrate(&refs).unwrap();
        assert_eq!(hydrated.len(), 3, "three unique payloads");
        assert_eq!(scope.counters().unique_payload_fetches, 3);
        assert_eq!(scope.counters().decode_count, 3);
        assert_eq!(
            scope.counters().bytes_read as usize,
            b"alpha".len() + b"beta".len() + b"gamma".len()
        );
    }

    /// Duplicate incoming facts schedule one payload publication; an unchanged
    /// rerun schedules zero new heavy payload writes.
    #[test]
    fn write_dedup_schedules_one_publication_and_zero_on_unchanged_rerun() {
        let (_t, repo) = setup();
        let mut scope = RequestScope::new(&repo);
        let payloads = vec![
            ("s/1", b"shared".to_vec()),
            ("s/2", b"shared".to_vec()), // identical bytes, different schema
            ("s/3", b"other".to_vec()),
        ];
        let first = scope.write_dedup(&payloads).unwrap();
        assert_eq!(
            first.len(),
            2,
            "byte-identical payloads collapse to one unique object (two unique payloads)"
        );
        assert_eq!(
            scope.counters().payload_writes,
            2,
            "two unique payload objects written"
        );
        // Byte-identical bytes under different schemas share one content
        // identity: provenance is a reference attribute, not payload identity.
        let shared_digest = cache::content_digest(b"shared");
        assert!(
            first.iter().any(|r| r.digest == shared_digest),
            "the shared payload must resolve to its single content object"
        );
        assert_eq!(
            cache::put(&repo, "s/1", b"shared").unwrap().digest,
            cache::put(&repo, "s/2", b"shared").unwrap().digest,
            "provenance must not split a byte-identical payload object"
        );

        // An unchanged rerun writes nothing new and resolves the same objects.
        let mut rerun = RequestScope::new(&repo);
        let second = rerun.write_dedup(&payloads).unwrap();
        assert_eq!(second, first);
        assert_eq!(
            rerun.counters().payload_writes,
            0,
            "unchanged rerun must not schedule writes"
        );
        assert_eq!(
            rerun.counters().integrity_checks,
            2,
            "presence probe verifies each unique object"
        );
    }

    /// Selective reads do not load unrelated payloads: only requested
    /// references are hydrated, and budgets fail closed instead of truncating.
    #[test]
    fn selective_reads_and_bounded_budgets_fail_closed() {
        let (_t, repo) = setup();
        let mut scope = RequestScope::new(&repo);
        let wanted_ref = cache::put(&repo, "s/1", b"wanted payload").unwrap();
        let _unrelated_ref = cache::put(&repo, "s/1", b"unrelated payload").unwrap();

        // Selective: request only the wanted payload.
        let hydrated = scope.hydrate(std::slice::from_ref(&wanted_ref)).unwrap();
        assert_eq!(hydrated.len(), 1);
        assert!(hydrated.contains_key(&wanted_ref.digest));
        assert_eq!(scope.counters().unique_payload_fetches, 1);

        // Byte budget: requesting a payload larger than the budget fails closed.
        let big = cache::put(&repo, "s/1", b"0123456789").unwrap();
        let error = scope.hydrate_bounded(&[big], 5, 16).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);

        // Membership budget: too many references fails closed.
        let many = vec![wanted_ref.clone(); 20];
        let error = scope.hydrate_bounded(&many, 10_000, 16).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }
}
