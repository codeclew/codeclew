//! Physical relocation preserves saved documentation, Work and pins without sources.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{cache, check::Check, snapshot_pins, store::Repository, work};
use serde_json::json;
use std::fs;
use support::Fixture;

#[test]
fn current_git_clone_explicitly_initializes_binds_and_reindexes() {
    let mut f = Fixture::new();
    let source = f.service("orders");
    let original_digest = Repository::open(&f.docs).unwrap().input_digest().unwrap();
    support::git(&f.docs, &["init", "-q"]);
    support::git(&f.docs, &["add", "."]);
    support::git(
        &f.docs,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "Current documentation",
        ],
    );
    let clone = f.temp.path().join("cloned-docs");
    support::git(
        f.temp.path(),
        &["clone", f.docs.to_str().unwrap(), clone.to_str().unwrap()],
    );
    f.docs = clone;
    assert!(!f.docs.join(".codeclew").exists());
    f.ok(&["docs", "init"]);
    assert_eq!(
        Repository::open(&f.docs).unwrap().input_digest().unwrap(),
        original_digest
    );
    f.ok(&[
        "docs",
        "bind",
        "--service",
        "orders",
        "--repo",
        source.to_str().unwrap(),
    ]);
    let (code, result) = f.run(&["docs", "check", "--service", "orders"]);
    assert!(matches!(code, 0 | 3), "{result}");
    let snapshot = result["snapshot"].as_str().unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, snapshot).unwrap();
    assert!(checked.unresolved.is_empty(), "{:?}", checked.unresolved);
    assert!(checked.services.contains_key("orders"));
}

#[test]
fn sqlite_store_preserves_pins_work_and_snapshot_without_source() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    assert!(f.docs.join(".codeclew/cache/object-layout.json").is_file());
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let prepared = work::prepare_with_snapshot(&repo, "service:orders".into(), serde_json::from_value(json!({
        "schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers", "entrypoint":"section-overview", "maxItems":20, "maxBytes":40960
    })).unwrap(), Some(&snapshot)).unwrap();
    let work_id = prepared["work"].as_str().unwrap();
    snapshot_pins::run(snapshot_pins::Command::Pin {
        root: f.docs.clone(),
        name: "release".into(),
        snapshot: snapshot.clone(),
    })
    .unwrap();
    let pin_before = fs::read(f.docs.join(".codeclew/cache/pins/release.json")).unwrap();
    let inventory_before = cache::inventory(&repo).unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    drop(repo);
    let reopened = Repository::open(&f.docs).unwrap();
    let restored = Check::load_snapshot(&reopened, &snapshot).unwrap();
    assert_eq!(restored.context_digest, checked.context_digest);
    assert_eq!(restored.services, checked.services);
    assert_eq!(
        work::load(&reopened, work_id).unwrap().snapshot.as_deref(),
        Some(snapshot.as_str())
    );
    snapshot_pins::run(snapshot_pins::Command::Show {
        root: f.docs.clone(),
        name: "release".into(),
    })
    .unwrap();
    assert_eq!(
        fs::read(f.docs.join(".codeclew/cache/pins/release.json")).unwrap(),
        pin_before
    );
    let inventory_after = cache::inventory(&reopened).unwrap();
    assert_eq!(inventory_before.object_count, inventory_after.object_count);
    assert_eq!(inventory_before.object_bytes, inventory_after.object_bytes);
    let again = restored.save_snapshot(&reopened).unwrap();
    assert_eq!(again, snapshot);
    assert_eq!(
        cache::inventory(&reopened).unwrap().object_count,
        inventory_after.object_count
    );
}
