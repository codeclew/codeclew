//! Recompose declaration inputs over an immutable source capture.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{cache, check::Check, store::Repository},
};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;
use support::Fixture;

fn write_json(repo: &Repository, path: &str, value: &Value) {
    let path = repo.root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, canonical::bytes(value).unwrap()).unwrap();
}

fn declaration_set(repo: &Repository, prefix: &str) {
    let entity = format!("{prefix}-entity");
    let scenario = format!("{prefix}-flow");
    let note = format!("{prefix}-note");
    write_json(
        repo,
        &format!("catalog/entities/{entity}.json"),
        &json!({
            "schema":"codeclew-documentation-entity/1.0",
            "id":entity,
            "title":format!("{prefix} entity"),
            "description":"An explicitly declared domain identity.",
            "relations":[{"service":"orders","kind":"owned","origin":"human","rationale":"The service owns this declaration.","confidence":"declared"}],
            "relatedEntities":[],
            "limitations":[]
        }),
    );
    write_json(
        repo,
        &format!("scenarios/{scenario}.yaml"),
        &json!({
            "schema":"codeclew-documentation-scenario/1.0",
            "id":scenario,
            "title":format!("{prefix} flow"),
            "summary":"A declared scenario retained as authored input.",
            "root":{"service":"orders"},
            "interactions":[],
            "maxDepth":4,
            "maxNodes":64
        }),
    );
    let note_path = repo.root.join(format!("notes/{note}.md"));
    fs::create_dir_all(note_path.parent().unwrap()).unwrap();
    fs::write(&note_path, format!("Original {prefix} note\n")).unwrap();
    write_json(
        repo,
        &format!("catalog/notes/{note}.json"),
        &json!({
            "schema":"codeclew-documentation-note-association/1.0",
            "id":note,
            "title":format!("{prefix} note"),
            "service":"orders",
            "path":format!("notes/{note}.md"),
            "targets":[format!("entity:{entity}"),format!("scenario:{scenario}")],
            "classification":"fact",
            "period":"2026",
            "tags":[],
            "metadata":{}
        }),
    );
}

fn remove_declaration_set(repo: &Repository, prefix: &str) {
    let entity = format!("{prefix}-entity");
    let scenario = format!("{prefix}-flow");
    let note = format!("{prefix}-note");
    for path in [
        format!("catalog/entities/{entity}.json"),
        format!("scenarios/{scenario}.yaml"),
        format!("catalog/notes/{note}.json"),
        format!("notes/{note}.md"),
    ] {
        fs::remove_file(repo.root.join(path)).unwrap();
    }
}

fn recompose(f: &Fixture, parent: &str) -> (i32, Value) {
    f.run(&["docs", "recompose", "--snapshot", parent])
}

fn object_path(repo: &Repository, handle: &str) -> std::path::PathBuf {
    let digest = handle.rsplit_once('/').unwrap().0;
    repo.root
        .join(cache::OBJECT_ROOT)
        .join(digest)
        .join("object.json")
}

fn original_capture(f: &Fixture) -> (Repository, Check, String) {
    let (code, report) = f.run(&["docs", "check"]);
    assert!(matches!(code, 0 | 3 | 4), "{report}");
    let parent = report["snapshot"].as_str().unwrap().to_owned();
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, &parent).unwrap();
    (repo, checked, parent)
}

#[test]
fn recompose_preserves_capture_and_replaces_only_declarations_offline() {
    let f = Fixture::new();
    let orders = f.service("orders");
    let inventory = f.service("inventory");
    let repo = Repository::open(&f.docs).unwrap();
    declaration_set(&repo, "old");
    let (_repo, original, parent) = original_capture(&f);
    let parent_bytes = fs::read(object_path(&repo, &parent)).unwrap();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_before = fs::read(&latest_path).unwrap();

    remove_declaration_set(&repo, "old");
    declaration_set(&repo, "new");
    fs::write(
        repo.root.join("codeclew-docs.yaml"),
        "schema: codeclew-documentation/1.0\ntitle: Recomposition title\n",
    )
    .unwrap();
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    fs::rename(&inventory, inventory.with_extension("offline")).unwrap();
    let marker = f.temp.path().join("unexpected-source-git");
    let git = f.temp.path().join("tools/git");
    fs::remove_file(&git).unwrap();
    fs::write(
        &git,
        format!(
            "#!/bin/sh\necho attempted >> '{}'\nexit 99\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
    let before = cache::inventory(&Repository::open(&f.docs).unwrap()).unwrap();
    let started = Instant::now();
    let (code, report) = recompose(&f, &parent);
    let first_seconds = started.elapsed().as_secs_f64();
    assert!(matches!(code, 0 | 3 | 4), "{report}");
    let derived = report["snapshot"].as_str().unwrap().to_owned();
    assert_ne!(derived, parent);

    let current_repo = Repository::open(&f.docs).unwrap();
    let after_recompose = cache::inventory(&current_repo).unwrap();
    let derived_check = Check::load_snapshot(&current_repo, &derived).unwrap();
    assert_eq!(
        serde_json::to_value(&derived_check.source_inputs).unwrap(),
        serde_json::to_value(&original.source_inputs).unwrap()
    );
    assert_eq!(derived_check.services, original.services);
    let parent_manifest: Value = serde_json::from_slice(&parent_bytes).unwrap();
    let derived_manifest: Value =
        serde_json::from_slice(&fs::read(object_path(&current_repo, &derived)).unwrap()).unwrap();
    assert_eq!(
        parent_manifest["serviceManifests"],
        derived_manifest["serviceManifests"]
    );
    assert_eq!(
        derived_check.services["orders"].sources,
        original.services["orders"].sources
    );
    assert_eq!(
        derived_check.input_digest,
        current_repo.input_digest().unwrap()
    );
    assert!(derived_check.dependencies.contains_key("entity:new-entity"));
    assert!(derived_check.dependencies.contains_key("note:new-note"));
    assert!(derived_check.dependencies.contains_key("scenario:new-flow"));
    assert!(!derived_check.dependencies.contains_key("entity:old-entity"));
    assert!(!derived_check.dependencies.contains_key("note:old-note"));
    assert!(!derived_check.dependencies.contains_key("scenario:old-flow"));
    assert_eq!(
        f.ok(&[
            "docs",
            "context",
            "--service",
            "orders",
            "--snapshot",
            &derived,
        ])["snapshot"],
        derived
    );
    let request = f.input(
        "recompose-work.json",
        &json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Declaration reviewers","maxItems":20,"maxBytes":40960}),
    );
    assert_eq!(
        f.ok(&[
            "docs",
            "work",
            "prepare",
            "--subject",
            "service:orders",
            "--input",
            request.to_str().unwrap(),
            "--snapshot",
            &derived,
        ])["snapshot"],
        derived
    );
    assert_eq!(fs::read(&latest_path).unwrap(), latest_before);
    assert_eq!(
        fs::read(object_path(&current_repo, &parent)).unwrap(),
        parent_bytes
    );

    let after_first = cache::inventory(&current_repo).unwrap();
    let started = Instant::now();
    let (repeat_code, repeat_report) = recompose(&f, &parent);
    let repeat_seconds = started.elapsed().as_secs_f64();
    assert!(matches!(repeat_code, 0 | 3 | 4), "{repeat_report}");
    assert_eq!(repeat_report["snapshot"], derived);
    let after_second = cache::inventory(&current_repo).unwrap();
    assert_eq!(after_first.object_count, after_second.object_count);
    assert_eq!(after_first.object_bytes, after_second.object_bytes);
    assert!(
        !marker.exists(),
        "recomposition or its consumers attempted source Git"
    );
    println!(
        "RECOMPOSITION_MEASUREMENT {}",
        json!({"firstSeconds":first_seconds,"repeatSeconds":repeat_seconds,"firstAddedObjects":after_recompose.object_count-before.object_count,"firstAddedPayloadBytes":after_recompose.object_bytes-before.object_bytes,"repeatAddedObjects":after_second.object_count-after_first.object_count,"repeatAddedPayloadBytes":after_second.object_bytes-after_first.object_bytes,"sourceGitAttempts":0,"sourceServices":2,"scope":"synthetic CLI fixture, warm installed test binary; compile/setup excluded"})
    );
}

#[test]
fn recompose_rejects_legacy_service_changed_derived_and_missing_parents() {
    let f = Fixture::new();
    f.service("orders");
    let (repo, original, parent) = original_capture(&f);
    let (derived_code, derived_report) = recompose(&f, &parent);
    assert!(matches!(derived_code, 0 | 3 | 4), "{derived_report}");
    let derived = derived_report["snapshot"].as_str().unwrap().to_owned();
    let latest_before = fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap();
    let mut tampered = Check::load_snapshot(&repo, &derived).unwrap();
    tampered.services.get_mut("orders").unwrap().revision = "f".repeat(40);
    assert!(
        tampered
            .save_snapshot(&repo)
            .unwrap_err()
            .message
            .contains("original capture parent")
    );
    assert_eq!(
        fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
        latest_before
    );

    let mut legacy = original.clone();
    legacy.source_inputs = None;
    let legacy_handle = legacy.save_snapshot(&repo).unwrap();
    let (legacy_code, legacy_error) = recompose(&f, &legacy_handle);
    assert_ne!(legacy_code, 0);
    assert!(
        legacy_error
            .to_string()
            .contains("RECOMPOSITION_INPUTS_UNAVAILABLE")
    );

    let service_path = f.docs.join("catalog/services/orders.json");
    let service_before = fs::read(&service_path).unwrap();
    let mut changed: Value = serde_json::from_slice(&service_before).unwrap();
    changed["title"] = json!("Changed declaration");
    fs::write(&service_path, canonical::bytes(&changed).unwrap()).unwrap();
    let (changed_code, changed_error) = recompose(&f, &parent);
    assert_ne!(changed_code, 0);
    assert!(
        changed_error
            .to_string()
            .contains("RECOMPOSITION_SOURCE_INPUTS_CHANGED")
    );
    fs::write(&service_path, service_before).unwrap();

    let (derived_code, derived_error) = recompose(&f, &derived);
    assert_ne!(derived_code, 0);
    assert!(
        derived_error
            .to_string()
            .contains("RECOMPOSITION_REQUIRES_CAPTURE_PARENT")
    );

    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_before = fs::read(&latest_path).unwrap();
    let missing_handle = format!("sha256:{}/1", "f".repeat(64));
    for missing in ["not-a-snapshot", missing_handle.as_str()] {
        let (code, error) = recompose(&f, missing);
        assert_ne!(code, 0);
        assert!(!error.is_null());
    }
    assert_eq!(fs::read(latest_path).unwrap(), latest_before);
}

#[test]
fn derived_missing_inputs_and_inconsistent_parent_fail_without_source_fallback() {
    let f = Fixture::new();
    let orders = f.service("orders");
    let (repo, original, parent) = original_capture(&f);
    let (_, result) = recompose(&f, &parent);
    let derived = result["snapshot"].as_str().unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(object_path(&repo, derived)).unwrap()).unwrap();
    let composition_ref: cache::ObjectRef =
        serde_json::from_value(manifest["composition"].clone()).unwrap();
    let mut composition: Value = cache::get_json(&repo, &composition_ref, 64 * 1024 * 1024)
        .unwrap()
        .unwrap();
    let declaration_ref: cache::ObjectRef =
        serde_json::from_value(composition["inputs"].clone()).unwrap();
    let declaration_path = repo
        .root
        .join(cache::OBJECT_ROOT)
        .join(&declaration_ref.digest)
        .join("object.json");
    let declarations = fs::read(&declaration_path).unwrap();
    let latest = fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap();
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    let marker = f.temp.path().join("corrupt-input-source-git");
    let git = f.temp.path().join("tools/git");
    fs::remove_file(&git).unwrap();
    fs::write(
        &git,
        format!(
            "#!/bin/sh\necho attempted >> '{}'\nexit 99\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_file(&declaration_path).unwrap();
    let (code, error) = f.run(&[
        "docs",
        "context",
        "--service",
        "orders",
        "--snapshot",
        derived,
    ]);
    assert_ne!(code, 0);
    assert!(
        error
            .to_string()
            .contains("source input manifest is missing"),
        "{error}"
    );
    fs::write(&declaration_path, declarations).unwrap();

    // Same source references with a false parent inputDigest are not a valid
    // original capture. Reader validation must not accept its derivative.
    let mut false_parent: Value =
        serde_json::from_slice(&fs::read(object_path(&repo, &parent)).unwrap()).unwrap();
    false_parent["inputDigest"] = json!(format!("sha256:{}", "f".repeat(64)));
    let false_parent_ref = cache::put_json(
        &repo,
        "codeclew-documentation-check-manifest/1.0",
        &false_parent,
    )
    .unwrap();
    composition["parent"] = json!(format!(
        "{}/{}",
        false_parent_ref.digest, false_parent_ref.size
    ));
    let changed_ref = cache::put_json(&repo, &composition_ref.schema, &composition).unwrap();
    let mut false_derived = manifest;
    false_derived["composition"] = serde_json::to_value(changed_ref).unwrap();
    let false_derived_ref = cache::put_json(
        &repo,
        "codeclew-documentation-check-manifest/1.0",
        &false_derived,
    )
    .unwrap();
    let error = Check::load_snapshot(
        &repo,
        &format!("{}/{}", false_derived_ref.digest, false_derived_ref.size),
    )
    .unwrap_err();
    assert!(error.message.contains("original capture parent"));
    assert!(
        !marker.exists(),
        "damaged derived input attempted source fallback"
    );
    assert_eq!(
        fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
        latest
    );
    assert_eq!(
        Check::load_snapshot(&repo, &parent).unwrap().services,
        original.services
    );
}
