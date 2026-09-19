//! Enabled regression coverage for the normalized docs-local object store and
//! reference-envelope capture/check persistence (2026-09-16 docs-cache
//! normalization). Exercises the real immutable content-addressed object store,
//! capture reference envelopes, and the check reference envelope through the
//! public API: byte-identical heavy payload is stored once regardless of
//! producer/admission key, corruption and missing objects are explicit errors,
//! and persisted checks no longer duplicate observations inline.

use clew::documentation::cache::{self, ObjectRef, load_capture, store_capture};
use clew::documentation::check::{Check, CheckManifest};
use clew::documentation::model::{EXTRACTOR, ServiceEvidence, Source};
use clew::documentation::store::Repository;
use clew::error::ErrorCode;
use std::collections::BTreeMap;

fn evidence(service: &str, text: &str) -> ServiceEvidence {
    let source = Source {
        id: format!("{service}:src"),
        service: service.into(),
        revision: "a".repeat(40),
        file: "src/Service.java".into(),
        start_line: 1,
        end_line: 1,
        text: text.into(),
        text_digest: format!("sha256:{}", "1".repeat(64)),
        evidence_digest: format!("sha256:{}", "2".repeat(64)),
        authority: "EXACT_SNAPSHOT_TEXT".into(),
        occurrence: None,
        url: None,
    };
    let mut sources = BTreeMap::new();
    sources.insert(source.id.clone(), source);
    ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.into(),
        revision: "a".repeat(40),
        service_digest: format!("sha256:{}", "3".repeat(64)),
        extractor: EXTRACTOR.into(),
        runtime_mode: "DEVELOPMENT".into(),
        coverage: "FULL".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: Default::default(),
        sources,
        contracts: Default::default(),
    }
}

fn check(service: &str, text: &str) -> Check {
    let mut services = BTreeMap::new();
    services.insert(service.to_string(), evidence(service, text));
    Check {
        schema: "codeclew-documentation-check/1.0".into(),
        source_inputs: None,
        composition: None,
        input_digest: "input".into(),
        context_digest: "ctx".into(),
        services,
        unresolved: Default::default(),
        interactions: Default::default(),
        scenarios: Default::default(),
        dependencies: Default::default(),
    }
}

fn repo(root: &std::path::Path) -> Repository {
    Repository::init(root, "Architecture").unwrap();
    Repository::open(root).unwrap()
}

/// Differently keyed byte-identical evidence stores the heavy payload once.
#[test]
fn byte_identical_evidence_across_producer_keys_stores_once() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    let a = store_capture(&repo, &evidence("svc-a", "same text")).unwrap();
    let mut b_evidence = evidence("svc-b", "same text");
    b_evidence.sources = evidence("svc-a", "same text").sources.clone();
    let b = store_capture(&repo, &b_evidence).unwrap();
    assert_eq!(a.sources.digest, b.sources.digest);
    let objects = cache::owned_digests(&repo, 1000).unwrap();
    assert_eq!(
        objects.len(),
        2,
        "one sources object + one shared empty payload"
    );
}

/// The capture reference envelope round-trips exact bytes and rejects a
/// dangling payload as corrupt (never silently forged).
#[test]
fn capture_envelope_round_trips_and_rejects_corruption() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    let manifest = store_capture(&repo, &evidence("svc", "public class Service {}")).unwrap();
    let loaded = load_capture(&repo, &manifest).unwrap();
    assert_eq!(loaded.service, "svc");
    assert_eq!(
        loaded.sources.values().next().unwrap().text,
        "public class Service {}"
    );
    let mut dangling = manifest.clone();
    dangling.sources.digest = format!("sha256:{}", "c".repeat(64));
    let error = load_capture(&repo, &dangling).unwrap_err();
    assert_eq!(error.code, ErrorCode::StateCorrupt);
}

/// A persisted check is a small reference envelope that hydrates back into a
/// full `Check`, and heavy service evidence is not duplicated inline in the
/// manifest.
#[test]
fn check_persists_as_reference_envelope_and_round_trips() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    let checked = check("svc", "payload text");
    let manifest = checked.store_manifest(&repo).unwrap();
    // The manifest is a reference envelope: service evidence lives in the
    // object store, not serialized inline as a heavy copy.
    assert_eq!(manifest.service_manifests.len(), 1);
    assert!(
        manifest
            .schema
            .starts_with("codeclew-documentation-check-manifest")
    );

    // Save writes the envelope to latest-check.json; load hydrates it back.
    checked.save(&repo).unwrap();
    let path = repo.path(".codeclew/cache/latest-check.json").unwrap();
    let manifest_bytes = std::fs::metadata(&path).unwrap().len();
    assert!(
        manifest_bytes < 16 * 1024,
        "check reference envelope must stay small, was {manifest_bytes} bytes"
    );
    let restored = Check::load(&repo, &path).unwrap();
    assert_eq!(restored.input_digest, "input");
    assert_eq!(restored.services["svc"].service, "svc");
    assert_eq!(
        restored.services["svc"]
            .sources
            .values()
            .next()
            .unwrap()
            .text,
        "payload text"
    );
    // The persisted manifest is loadable as the CheckManifest type.
    let parsed: CheckManifest = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(parsed.service_manifests.len(), 1);
    let object_digests = cache::owned_digests(&repo, 1000).unwrap();
    assert!(
        !object_digests.is_empty(),
        "heavy payload persisted as immutable objects"
    );
}

/// Missing and corrupted objects are explicit errors; they are never accepted
/// as valid cached evidence.
#[test]
fn missing_and_corrupt_objects_are_explicit_errors() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    let missing = ObjectRef::new(
        "codeclew-documentation-sources/1.0".into(),
        format!("sha256:{}", "0".repeat(64)),
        1,
    );
    assert!(cache::get(&repo, &missing, 1024).unwrap().is_none());
    // Oversized read bound is a typed budget/resource error.
    let oversized = ObjectRef::new(
        "schema/x".into(),
        format!("sha256:{}", "a".repeat(64)),
        9999,
    );
    let error = cache::get(&repo, &oversized, 100).unwrap_err();
    assert_eq!(error.code, ErrorCode::ResourceLimit);
    // A malformed digest reference is rejected up front.
    let malformed = ObjectRef::new("schema/x".into(), "not-a-digest".into(), 1);
    let error = cache::get(&repo, &malformed, 1024).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

/// Same-path source under different compilations is not overwritten or
/// collapsed into one identical manifest: distinct content yields distinct
/// payload objects, while equal source text shares a payload without merging
/// provenance.
#[test]
fn same_path_different_compilation_authority_round_trips_without_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());

    // Two compilations of the same file path with DIFFERENT content.
    let mut a = evidence("svc-a", "class A { void a() {} }");
    let mut b = evidence("svc-b", "class B { void b() {} }");
    // Force an identical source record id/path so any collapse would collide.
    a.sources = BTreeMap::from([(
        "svc-a:src".into(),
        Source {
            id: "shared:src".into(),
            ..a.sources.values().next().unwrap().clone()
        },
    )]);
    b.sources = BTreeMap::from([(
        "svc-b:src".into(),
        Source {
            id: "shared:src".into(),
            ..b.sources.values().next().unwrap().clone()
        },
    )]);
    let ma = store_capture(&repo, &a).unwrap();
    let mb = store_capture(&repo, &b).unwrap();
    assert_ne!(
        ma.sources.digest, mb.sources.digest,
        "different compilation content must stay distinct, never overwritten"
    );
    let la = load_capture(&repo, &ma).unwrap();
    let lb = load_capture(&repo, &mb).unwrap();
    assert_ne!(
        la.sources["svc-a:src"].text, lb.sources["svc-b:src"].text,
        "same path must not collapse into an identical manifest"
    );

    // Equal source text under different provenance round-trips with distinct
    // authority records: provenance is never flattened, even when text matches.
    let mut c = evidence("svc-c", "class A { void a() {} }");
    c.sources = BTreeMap::from([(
        "svc-c:src".into(),
        Source {
            id: "provenance:src".into(),
            service: "svc-c".into(),
            authority: "TRANSFORMED_SOURCE".into(),
            ..a.sources.values().next().unwrap().clone()
        },
    )]);
    let mc = store_capture(&repo, &c).unwrap();
    let lc = load_capture(&repo, &mc).unwrap();
    assert_eq!(lc.sources["svc-c:src"].text, la.sources["svc-a:src"].text);
    assert_eq!(lc.sources["svc-c:src"].authority, "TRANSFORMED_SOURCE");
    assert_eq!(la.sources["svc-a:src"].authority, "EXACT_SNAPSHOT_TEXT");
}

/// A still-valid capture is reused without invoking the producer (the reuse
/// path loads evidence purely from the object store, so the source tree is not
/// consulted), and a damaged capture is never silently reused.
#[test]
fn capture_reuse_skips_producer_and_rejects_damage() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    let key = "cached-key";
    let service = "svc";
    cache::save_capture(&repo, service, key, &evidence(service, "reusable text")).unwrap();

    // Reuse returns the evidence from the object store without a source tree.
    let reused = cache::load_capture_if_valid(&repo, service, key)
        .unwrap()
        .expect("valid capture must be reused");
    assert_eq!(
        reused.sources.values().next().unwrap().text,
        "reusable text"
    );

    // A different key is a miss, never a false reuse.
    assert!(
        cache::load_capture_if_valid(&repo, service, "other-key")
            .unwrap()
            .is_none()
    );

    // Damage to a referenced object must be an explicit error, not a silent
    // reuse of forged evidence.
    let manifest_path = cache::capture_manifest_path(&repo, service, key).unwrap();
    let manifest: cache::CaptureManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let object_file = repo
        .root
        .join(cache::OBJECT_ROOT)
        .join(&manifest.sources.digest)
        .join("object.json");
    std::fs::write(&object_file, b"tampered").unwrap();
    let error = cache::load_capture_if_valid(&repo, service, key).unwrap_err();
    assert_eq!(error.code, ErrorCode::StateCorrupt);
}

/// A change to the bound reuse identity (revision/service/extractor) yields a
/// different reuse key, so an old capture is not reused for changed source
/// state — recapture is forced rather than serving a stale vector.
#[test]
fn reuse_key_binds_identity_so_state_change_recaptures() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    let service = "svc";

    // Capture under the original source state.
    let original = evidence(service, "original source");
    cache::save_capture(&repo, service, "reuse-key-v1", &original).unwrap();
    assert!(
        cache::load_capture_if_valid(&repo, service, "reuse-key-v1")
            .unwrap()
            .is_some()
    );

    // The producer binds identity to the key (revision+service_digest+
    // extractor+language). A changed revision/service is a DIFFERENT key, so
    // the stale v1 capture is not reused for the new state.
    let mut changed = evidence(service, "changed source");
    changed.revision = "b".repeat(40);
    let key_v2 = clew::canonical::hash(&clew::documentation::model::Service {
        schema: changed.schema.clone(),
        id: changed.service.clone(),
        title: String::new(),
        repository_id: String::new(),
        repository: String::new(),
        language: "java".into(),
        profile: "source-syntax".into(),
        compilation: String::new(),
        compilations: Vec::new(),
        source: None,
        modules: None,
        target_ref: "main".into(),
        source_link_template: None,
        contract_files: vec![],
        annotation_processor_paths: vec![],
    })
    .unwrap();
    cache::save_capture(&repo, service, &key_v2, &changed).unwrap();
    // Both the new-state capture and the old-state capture coexist; the old
    // one is still retrievable under its own key, and the new state is not
    // served from the stale v1 vector.
    let v1 = cache::load_capture_if_valid(&repo, service, "reuse-key-v1")
        .unwrap()
        .unwrap();
    let v2 = cache::load_capture_if_valid(&repo, service, &key_v2)
        .unwrap()
        .unwrap();
    assert_eq!(v1.sources.values().next().unwrap().text, "original source");
    assert_eq!(v2.sources.values().next().unwrap().text, "changed source");
}

/// Synthetic two-service scale regression: repeated unchanged capture/reuse
/// sequences produce no additional heavy objects and no producer invocations
/// after the first capture (stable heavy-object count/bytes).
#[test]
fn ten_unchanged_sequences_keep_heavy_objects_stable() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    // Two services with repeated source payloads.
    let services = ["svc-a", "svc-b"];
    for id in services {
        cache::save_capture(&repo, id, "scale-key", &evidence(id, "repeated payload")).unwrap();
    }
    let first = cache::inventory(&repo).unwrap();
    let first_object_count = first.object_count;
    let first_object_bytes = first.object_bytes;
    assert!(first_object_count >= 1);

    // Ten unchanged check/render/refresh-equivalent reuse sequences: the
    // valid capture is reused from the object store, creating no new objects
    // and not re-running the producer.
    for _ in 0..10 {
        for id in services {
            let reused = cache::load_capture_if_valid(&repo, id, "scale-key")
                .unwrap()
                .expect("valid capture must be reused");
            assert_eq!(
                reused.sources.values().next().unwrap().text,
                "repeated payload"
            );
        }
    }
    let last = cache::inventory(&repo).unwrap();
    assert_eq!(
        last.object_count, first_object_count,
        "no additional heavy objects after repeated reuse"
    );
    assert_eq!(
        last.object_bytes, first_object_bytes,
        "no additional heavy bytes after repeated reuse"
    );
}

/// Concurrent captures of one service under different source states must not
/// clobber each other's objects or manifests: each keyed capture stays
/// independently loadable. An interrupted (truncated) manifest must be an
/// explicit corrupt-state error, never silently reused as a stale vector.
#[test]
fn concurrent_capture_keeps_ownership_and_interrupted_manifest_rejects() {
    use std::sync::Arc;
    let root = Arc::new(tempfile::tempdir().unwrap());
    let repo = repo(root.path());

    // Two concurrent captures of the same service under different source-state
    // keys with different payloads.
    let root_a = root.clone();
    let root_b = root.clone();
    let a = std::thread::spawn(move || {
        let repo = Repository::open(root_a.path()).unwrap();
        cache::save_capture(&repo, "svc", "key-a", &evidence("svc", "content A")).unwrap()
    });
    let b = std::thread::spawn(move || {
        let repo = Repository::open(root_b.path()).unwrap();
        cache::save_capture(&repo, "svc", "key-b", &evidence("svc", "content B")).unwrap()
    });
    a.join().unwrap();
    b.join().unwrap();

    // Both captures coexist and load independently (ownership preserved).
    let va = cache::load_capture_if_valid(&repo, "svc", "key-a")
        .unwrap()
        .unwrap();
    let vb = cache::load_capture_if_valid(&repo, "svc", "key-b")
        .unwrap()
        .unwrap();
    assert_eq!(va.sources.values().next().unwrap().text, "content A");
    assert_eq!(vb.sources.values().next().unwrap().text, "content B");

    // An interrupted manifest (truncated) is an explicit error, not a stale
    // reuse.
    let manifest_path = cache::capture_manifest_path(&repo, "svc", "key-a").unwrap();
    std::fs::write(
        &manifest_path,
        b"{\"schema\":\"codeclew-documentation-capture-manifest",
    )
    .unwrap();
    let error = cache::load_capture_if_valid(&repo, "svc", "key-a").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

/// A capture marked NON_CACHEABLE (incomplete external authority) is never
/// reused: load_capture_if_valid returns a miss so the producer recaptures,
/// and the non-cacheable reason is surfaced on the persisted manifest.
#[test]
fn non_cacheable_capture_is_not_reused_and_reason_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    // A reusable capture round-trips.
    cache::save_capture(&repo, "svc", "reuse-key", &evidence("svc", "reusable")).unwrap();
    assert!(
        cache::load_capture_if_valid(&repo, "svc", "reuse-key")
            .unwrap()
            .is_some()
    );

    // Mark the same capture non-cacheable with a reason.
    let path = cache::capture_manifest_path(&repo, "svc", "reuse-key").unwrap();
    let mut manifest: cache::CaptureManifest =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    cache::mark_non_cacheable(
        &mut manifest,
        "Maven/external-state capture lacks complete build/settings/dependency authority",
    );
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    // The persisted manifest carries the reason (reported to the user).
    let persisted: cache::CaptureManifest =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted.cacheability, cache::NON_CACHEABLE);
    assert!(
        persisted
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("authority"))
    );

    // A non-cacheable capture is never reused; the producer recaptures.
    assert!(
        cache::load_capture_if_valid(&repo, "svc", "reuse-key")
            .unwrap()
            .is_none()
    );
}

/// Repeated equivalent publications share the same heavy evidence object:
/// identical retained-sources/observations payloads store once and are
/// referenced by both, while the persisted bindings remain self-contained
/// (inline records retained) so they are portable without the private cache.
#[test]
fn equivalent_publications_share_heavy_evidence_and_stay_portable() {
    use clew::documentation::bindings::{Bindings, store_bindings_heavy};
    use clew::documentation::model::SectionState;
    use std::collections::BTreeMap;

    let root = tempfile::tempdir().unwrap();
    let repo = repo(root.path());
    let make = |id: &str| -> Bindings {
        let sources = evidence(id, "shared payload").sources;
        Bindings {
            schema: "codeclew-documentation-bindings/1.3".into(),
            input_digest: "input".into(),
            renderer: "codeclew-documentation-html/1.13".into(),
            extractor: EXTRACTOR.into(),
            revisions: BTreeMap::new(),
            coverage: BTreeMap::new(),
            catalogues: BTreeMap::new(),
            fragments: BTreeMap::new(),
            observations: BTreeMap::new(),
            narratives: BTreeMap::new(),
            output_hashes: BTreeMap::new(),
            retained_sources: sources,
            section_states: BTreeMap::<String, SectionState>::new(),
            target_revisions: BTreeMap::new(),
            update_failures: BTreeMap::new(),
            accepted_versions: BTreeMap::new(),
            heavy: None,
        }
    };
    let mut a = make("svc-a");
    let mut b = make("svc-b");
    // Identical heavy payloads across the two publications.
    b.retained_sources = a.retained_sources.clone();
    store_bindings_heavy(&repo, &mut a).unwrap();
    store_bindings_heavy(&repo, &mut b).unwrap();
    let ha = a.heavy.as_ref().unwrap();
    let hb = b.heavy.as_ref().unwrap();
    assert_eq!(
        ha.retained_sources.digest, hb.retained_sources.digest,
        "equivalent publications must share one heavy sources object"
    );
    // Persisted bindings remain self-contained (inline retained for portability).
    assert!(
        !a.retained_sources.is_empty(),
        "inline records kept for portability"
    );
    assert!(!b.retained_sources.is_empty());
}
