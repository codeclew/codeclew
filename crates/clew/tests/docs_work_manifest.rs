//! Immutable Work manifests share saved evidence; legacy inline work stays readable.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::{
    canonical,
    documentation::{cache, check::Check, model::Observation, store::Repository, work},
};
use serde_json::{Value, json};
use std::{fs, time::Instant};
use support::{Fixture, read};

fn request(audience: &str) -> work::Request {
    work::Request {
        schema: "codeclew-documentation-work-request/1.0".into(),
        audience: audience.into(),
        entrypoint: None,
        context_profile: None,
        max_items: 20,
        max_bytes: 40 * 1024,
        external_inputs: Vec::new(),
    }
}
fn record_path(repo: &Repository, id: &str) -> std::path::PathBuf {
    repo.root.join(format!(".codeclew/work/{id}/work.json"))
}
fn object_path(repo: &Repository, handle: &str) -> std::path::PathBuf {
    repo.root
        .join(cache::OBJECT_ROOT)
        .join(handle.rsplit_once('/').unwrap().0)
        .join("object.json")
}
fn prepare(repo: &Repository, snapshot: &str, audience: &str) -> Value {
    work::prepare_with_snapshot(
        repo,
        "service:orders".into(),
        request(audience),
        Some(snapshot),
    )
    .unwrap()
}
fn write_inline(repo: &Repository, mut value: work::Work) -> String {
    value.schema = "codeclew-documentation-work/1.0".into();
    value.id.clear();
    value.id = canonical::hash(&value).unwrap()[7..].into();
    let path = record_path(repo, &value.id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, canonical::bytes(&value).unwrap()).unwrap();
    value.id
}

#[test]
fn large_saved_check_is_shared_by_small_independent_work_manifests() {
    let f = Fixture::new();
    let source = f.service("orders");
    let repo = Repository::open(&f.docs).unwrap();
    let mut checked = f.checked();
    // Synthetic unselected facts, not a claim of native service throughput.
    // Both the service observations and Check dependencies contain these facts,
    // so the former inline Work would exceed its 64 MiB record limit.
    for n in 0..34 {
        let mut observation = Observation {
            id: format!("orders:large:{n}"),
            kind: "SYNTHETIC_STORAGE_FIXTURE".into(),
            service: "orders".into(),
            symbol: format!("unselected{n}"),
            normalized: json!({"text":"x".repeat(1024 * 1024)}),
            digest: String::new(),
            source_ids: Vec::new(),
        };
        observation.digest = canonical::hash(&observation.normalized).unwrap();
        checked
            .dependencies
            .insert(observation.id.clone(), observation.clone());
        checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(observation.id.clone(), observation);
    }
    let snapshot = checked.save_snapshot(&repo).unwrap();
    drop(checked);
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    fs::write(&latest, b"not a readable latest pointer").unwrap();
    let before = cache::inventory(&repo).unwrap();
    let started = Instant::now();
    let first = prepare(&repo, &snapshot, "Maintainers");
    let first_ms = started.elapsed().as_millis();
    let a = first["work"].as_str().unwrap();
    let second = prepare(&repo, &snapshot, "Operators");
    let b = second["work"].as_str().unwrap();
    assert_ne!(a, b);
    let a_bytes = fs::read(record_path(&repo, a)).unwrap();
    let b_bytes = fs::read(record_path(&repo, b)).unwrap();
    for raw in [&a_bytes, &b_bytes] {
        let stored: Value = serde_json::from_slice(raw).unwrap();
        assert!(stored.get("checked").is_none());
        assert_eq!(stored["evidenceSnapshot"], snapshot);
        assert_eq!(stored["snapshot"], snapshot);
        assert!(raw.len() < 64 * 1024, "manifest bytes: {}", raw.len());
    }
    let after = cache::inventory(&repo).unwrap();
    assert_eq!(before.object_count, after.object_count);
    assert_eq!(before.object_bytes, after.object_bytes);
    assert_eq!(fs::read(&latest).unwrap(), b"not a readable latest pointer");
    let loaded = work::load(&repo, a).unwrap();
    let inline_bytes = canonical::bytes(&loaded).unwrap().len();
    assert!(inline_bytes > 64 * 1024 * 1024);
    assert!(a_bytes.len() * 100 < inline_bytes);
    let id = a.to_owned();
    drop(loaded);
    let selection = f.input("selection.json", &json!({}));
    let reloaded = f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        &id,
        "--input",
        selection.to_str().unwrap(),
    ]);
    assert_eq!(reloaded["work"], first["work"]);
    assert_eq!(reloaded["items"], first["items"]);
    assert_eq!(fs::read(record_path(&repo, a)).unwrap(), a_bytes);
    println!(
        "WORK_STORAGE_MEASUREMENT {}",
        json!({
            "fixture":"34 synthetic 1MiB observations mirrored in dependencies/service evidence",
            "formerInlineWorkBytes":inline_bytes,"manifestABytes":a_bytes.len(),
            "manifestBBytes":b_bytes.len(),"additionalEvidenceObjects":after.object_count-before.object_count,
            "additionalEvidenceBytes":after.object_bytes-before.object_bytes,
            "prepareAElapsedMs":first_ms,"firstPageBytes":canonical::bytes(&first).unwrap().len(),
            "nativeThroughputMeasured":false,"modelTokensMeasured":false
        })
    );
}

#[test]
fn legacy_inline_work_remains_readable_without_snapshot_objects() {
    let f = Fixture::new();
    let source = f.service("orders");
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = f.checked().save_snapshot(&repo).unwrap();
    let prepared = prepare(&repo, &snapshot, "Maintainers");
    let mut runtime = work::load(&repo, prepared["work"].as_str().unwrap()).unwrap();
    let pinned_id = write_inline(&repo, runtime.clone());
    runtime.snapshot = None;
    let original_id = write_inline(&repo, runtime);
    fs::rename(&source, source.with_extension("offline")).unwrap();
    fs::remove_dir_all(repo.root.join(cache::OBJECT_ROOT)).unwrap();
    for id in [&pinned_id, &original_id] {
        let path = record_path(&repo, id);
        let before = fs::read(&path).unwrap();
        let loaded = work::load(&repo, id).unwrap();
        assert_eq!(loaded.checked.services.len(), 1);
        assert_eq!(loaded.snapshot.is_some(), *id == pinned_id);
        let page = work::read(&repo, id, Default::default()).unwrap();
        assert_eq!(page["items"], prepared["items"]);
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn manifest_and_snapshot_damage_fail_without_reacquisition_or_latest_fallback() {
    let f = Fixture::new();
    let source = f.service("orders");
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = f.checked().save_snapshot(&repo).unwrap();
    let page = prepare(&repo, &snapshot, "Maintainers");
    let id = page["work"].as_str().unwrap();
    let path = record_path(&repo, id);
    let original = fs::read(&path).unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let latest = fs::read(repo.root.join(".codeclew/cache/latest-check.json")).unwrap();
    let mut bad = read(&path);
    bad["subject"] = json!("service:forged");
    fs::write(&path, canonical::bytes(&bad).unwrap()).unwrap();
    assert!(work::load(&repo, id).is_err());
    fs::write(&path, &original).unwrap();
    // Even a self-consistent new record cannot label one snapshot's facts with
    // another explicit snapshot authority.
    let mut mislabeled = read(&path);
    mislabeled["snapshot"] = json!(format!("sha256:{}/1", "9".repeat(64)));
    mislabeled["id"] = json!("");
    let mislabeled_id = canonical::hash(&mislabeled).unwrap()[7..].to_owned();
    mislabeled["id"] = json!(mislabeled_id);
    let mislabeled_path = record_path(&repo, &mislabeled_id);
    fs::create_dir_all(mislabeled_path.parent().unwrap()).unwrap();
    fs::write(mislabeled_path, canonical::bytes(&mislabeled).unwrap()).unwrap();
    assert!(work::load(&repo, &mislabeled_id).is_err());
    let object = object_path(&repo, &snapshot);
    let mut damaged = fs::read(&object).unwrap();
    damaged[0] ^= 1;
    fs::write(&object, damaged).unwrap();
    assert!(work::load(&repo, id).is_err());
    fs::remove_file(&object).unwrap();
    assert!(work::read(&repo, id, Default::default()).is_err());
    assert_eq!(
        fs::read(repo.root.join(".codeclew/cache/latest-check.json")).unwrap(),
        latest
    );
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(!repo.root.join("docs/index.html").exists());
}

#[test]
fn legacy_preparation_stores_a_reference_without_changing_recheck_semantics() {
    let f = Fixture::new();
    f.service("orders");
    let request = f.input(
        "request.json",
        &json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers"}),
    );
    let page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request.to_str().unwrap(),
    ]);
    let repo = Repository::open(&f.docs).unwrap();
    let id = page["work"].as_str().unwrap();
    let stored = read(record_path(&repo, id));
    assert!(stored.get("checked").is_none());
    assert!(stored["evidenceSnapshot"].as_str().is_some());
    let hydrated = work::load(&repo, id).unwrap();
    assert!(hydrated.snapshot.is_none());
    assert_eq!(hydrated.checked.services.len(), 1);
    assert!(Check::load_snapshot(&repo, stored["evidenceSnapshot"].as_str().unwrap()).is_ok());
}
