//! Explicit named retention of immutable documentation evidence, never capture.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{cache, check::Check, store::Repository};
use serde_json::Value;
use std::{fs, path::PathBuf};
use support::Fixture;

fn marker(f: &Fixture, name: &str) -> PathBuf {
    f.docs.join(format!(".codeclew/cache/pins/{name}.json"))
}
fn pin(f: &Fixture, name: &str, handle: &str) -> Value {
    f.ok(&[
        "docs",
        "snapshot",
        "pin",
        "--name",
        name,
        "--snapshot",
        handle,
    ])
}
fn object(repo: &Repository, digest: &str) -> PathBuf {
    repo.root
        .join(cache::OBJECT_ROOT)
        .join(digest)
        .join("object.json")
}

#[test]
fn pins_share_objects_and_survive_latest_and_declaration_changes_without_source() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    let before = cache::inventory(&repo).unwrap();
    pin(&f, "release", &handle);
    let first = fs::read(marker(&f, "release")).unwrap();
    pin(&f, "release", &handle);
    pin(&f, "incident", &handle);
    assert_eq!(fs::read(marker(&f, "release")).unwrap(), first);
    assert!(first.len() < 4096);
    let after = cache::inventory(&repo).unwrap();
    assert_eq!(before.object_count, after.object_count);
    assert_eq!(before.object_bytes, after.object_bytes);
    fs::rename(&source, source.with_extension("offline")).unwrap();
    fs::write(
        f.docs.join(".codeclew/cache/latest-check.json"),
        "not a snapshot",
    )
    .unwrap();
    let manifest_path = f.docs.join("codeclew-docs.yaml");
    let mut current: Value =
        serde_yaml_ng::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    current["title"] = "Changed title after capture".into();
    fs::write(&manifest_path, serde_yaml_ng::to_string(&current).unwrap()).unwrap();
    assert_ne!(
        Repository::open(&f.docs).unwrap().input_digest().unwrap(),
        checked.input_digest
    );
    let shown = f.ok(&["docs", "snapshot", "show", "--name", "release"]);
    assert!(shown.to_string().contains(&handle));
    assert!(shown.to_string().contains("UNVERIFIED"));
    let listed = f.ok(&["docs", "snapshot", "list"]);
    assert!(listed.to_string().contains("NOT_CHECKED"));
    f.ok(&["docs", "snapshot", "unpin", "--name", "release"]);
    assert!(!marker(&f, "release").exists());
    f.ok(&["docs", "snapshot", "show", "--name", "incident"]);
    f.ok(&["docs", "snapshot", "unpin", "--name", "release"]);
    assert_eq!(
        cache::inventory(&repo).unwrap().object_count,
        before.object_count
    );
    assert_eq!(
        fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
        b"not a snapshot"
    );
}

#[test]
fn pin_conflict_and_writer_lock_preserve_the_existing_root() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    pin(&f, "release", &handle);
    let before = fs::read(marker(&f, "release")).unwrap();
    let mut legacy = checked;
    legacy.source_inputs = None;
    let other = legacy.save_snapshot(&repo).unwrap();
    assert_ne!(other, handle);
    let (code, _) = f.run(&[
        "docs",
        "snapshot",
        "pin",
        "--name",
        "release",
        "--snapshot",
        &other,
    ]);
    assert_ne!(code, 0);
    assert_eq!(fs::read(marker(&f, "release")).unwrap(), before);
    let _lock = repo.lock().unwrap();
    let (code, _) = f.run(&["docs", "snapshot", "unpin", "--name", "release"]);
    assert_ne!(code, 0);
    assert_eq!(fs::read(marker(&f, "release")).unwrap(), before);
}

#[test]
fn broken_evidence_never_registers_or_revalidates_but_unpin_still_works() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    pin(&f, "release", &handle);
    let manifest = checked.store_manifest(&repo).unwrap();
    let sources = &manifest.service_manifests["orders"].sources;
    fs::write(
        object(&repo, &sources.digest),
        vec![b'x'; sources.size as usize],
    )
    .unwrap();
    for command in [
        vec![
            "docs",
            "snapshot",
            "pin",
            "--name",
            "new",
            "--snapshot",
            &handle,
        ],
        vec!["docs", "snapshot", "show", "--name", "release"],
        vec![
            "docs",
            "snapshot",
            "pin",
            "--name",
            "release",
            "--snapshot",
            &handle,
        ],
    ] {
        let (code, _) = f.run(&command);
        assert_ne!(code, 0);
    }
    assert!(!marker(&f, "new").exists());
    assert!(marker(&f, "release").exists());
    let listed = f.ok(&["docs", "snapshot", "list"]);
    assert!(listed.to_string().contains("NOT_CHECKED"));
    f.ok(&["docs", "snapshot", "unpin", "--name", "release"]);
    assert!(object(&repo, &sources.digest).exists());
}

#[test]
fn pin_recomposed_snapshot_verifies_original_parent_reader_data() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let parent = checked.save_snapshot(&repo).unwrap();
    let parent_index = checked
        .store_manifest(&repo)
        .unwrap()
        .dependencies_index
        .unwrap();
    let title_path = f.docs.join("codeclew-docs.yaml");
    let mut title: Value =
        serde_yaml_ng::from_str(&fs::read_to_string(&title_path).unwrap()).unwrap();
    title["title"] = "Recomposed title".into();
    fs::write(title_path, serde_yaml_ng::to_string(&title).unwrap()).unwrap();
    // Change documentary membership so the original and derived indexes differ.
    let entities = f.docs.join("catalog/entities");
    fs::create_dir_all(&entities).unwrap();
    fs::write(entities.join("saved-contract.json"), serde_json::to_vec(&serde_json::json!({
        "schema":"codeclew-documentation-entity/1.0", "id":"saved-contract", "title":"Saved contract",
        "description":"Explicitly declared documentary concept", "relations":[], "relatedEntities":[], "limitations":[]
    })).unwrap()).unwrap();
    let (code, result) = f.run(&["docs", "recompose", "--snapshot", &parent]);
    assert!(matches!(code, 0 | 3), "{result}");
    let derived = result["snapshot"].as_str().unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    pin(&f, "derived", derived);
    let derived_path = object(&repo, derived.rsplit_once('/').unwrap().0);
    let derived_manifest: Value = serde_json::from_slice(&fs::read(derived_path).unwrap()).unwrap();
    assert_ne!(
        derived_manifest["dependenciesIndex"]["digest"],
        parent_index.digest
    );
    fs::remove_file(object(&repo, &parent_index.digest)).unwrap();
    // The normal derived read validates the parent manifest, not its dependency index.
    assert!(Check::load_snapshot(&repo, derived).is_ok());
    let (code, _) = f.run(&["docs", "snapshot", "show", "--name", "derived"]);
    assert_ne!(code, 0);
    let (code, _) = f.run(&[
        "docs",
        "snapshot",
        "pin",
        "--name",
        "new",
        "--snapshot",
        derived,
    ]);
    assert_ne!(code, 0);
    assert!(!marker(&f, "new").exists());
    f.ok(&["docs", "snapshot", "unpin", "--name", "derived"]);
}

#[test]
fn pin_paths_and_stored_records_fail_closed() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    let (code, _) = f.run(&[
        "docs",
        "snapshot",
        "pin",
        "--name",
        "../escape",
        "--snapshot",
        &handle,
    ]);
    assert_ne!(code, 0);
    pin(&f, "release", &handle);
    let original = fs::read(marker(&f, "release")).unwrap();
    let mut record: Value = serde_json::from_slice(&original).unwrap();
    record["name"] = "other".into();
    fs::write(marker(&f, "release"), serde_json::to_vec(&record).unwrap()).unwrap();
    for command in [
        vec!["docs", "snapshot", "list"],
        vec!["docs", "snapshot", "show", "--name", "release"],
        vec!["docs", "snapshot", "unpin", "--name", "release"],
    ] {
        assert_ne!(f.run(&command).0, 0);
    }
    fs::remove_file(marker(&f, "release")).unwrap();
    let outside = f.temp.path().join("outside.json");
    fs::write(&outside, &original).unwrap();
    std::os::unix::fs::symlink(&outside, marker(&f, "release")).unwrap();
    assert_ne!(
        f.run(&["docs", "snapshot", "show", "--name", "release"]).0,
        0
    );
    assert_ne!(
        f.run(&["docs", "snapshot", "unpin", "--name", "release"]).0,
        0
    );
    assert_eq!(fs::read(outside).unwrap(), original);
}

#[test]
fn interrupted_pin_staging_does_not_poison_registered_roots() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    pin(&f, "release", &handle);
    let interrupted = f.docs.join(".codeclew/cache/pins/.pin-staging-interrupted");
    fs::write(&interrupted, b"partial staged record").unwrap();
    let list = f.ok(&["docs", "snapshot", "list"]);
    assert_eq!(list["interruptedStagingEntries"], 1);
    pin(&f, "incident", &handle);
    f.ok(&["docs", "snapshot", "show", "--name", "incident"]);
    assert_eq!(fs::read(interrupted).unwrap(), b"partial staged record");
}
