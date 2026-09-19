//! New captures retain the inputs that selected their source evidence.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{
    cache,
    check::{Check, SOURCE_INPUTS_SCHEMA},
    store::Repository,
};
use serde_json::Value;
use std::fs;
use support::Fixture;

#[test]
fn source_inputs_round_trip_share_objects_and_record_selection() {
    let f = Fixture::new();
    f.service("orders");
    f.service("inventory");
    let (code, report) = f.run(&["docs", "check", "--service", "orders"]);
    assert_eq!(code, 3, "{report}");
    let repo = Repository::open(&f.docs).unwrap();
    let handle = report["snapshot"].as_str().unwrap();
    let checked = Check::load_snapshot(&repo, handle).unwrap();
    let record = checked.source_inputs.as_ref().unwrap();
    assert_eq!(record.schema, SOURCE_INPUTS_SCHEMA);
    assert_eq!(record.input_digest, repo.input_digest().unwrap());
    assert_eq!(record.inputs, repo.inputs().unwrap());
    assert_eq!(
        record.selected_services.iter().collect::<Vec<_>>(),
        vec!["orders"]
    );
    assert!(checked.services.contains_key("orders"));
    assert!(!checked.services.contains_key("inventory"));
    assert_eq!(
        checked.unresolved["inventory"]["reason"],
        "SERVICE_NOT_SELECTED"
    );
    let before = cache::inventory(&repo).unwrap();
    assert_eq!(checked.save_snapshot(&repo).unwrap(), handle);
    let after = cache::inventory(&repo).unwrap();
    assert_eq!(before.object_count, after.object_count);
    assert_eq!(before.object_bytes, after.object_bytes);
    let manifest: Value = serde_json::from_slice(
        &fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["sourceInputs"]["schema"],
        "codeclew-documentation-source-inputs-manifest/1.0"
    );
    assert!(manifest["sourceInputs"].get("inputs").is_none());
}

#[test]
fn missing_source_inputs_and_invalid_contracts_cannot_be_saved() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let original = checked.save_snapshot(&repo).unwrap();
    let mut legacy = checked.clone();
    legacy.source_inputs = None;
    assert!(
        legacy
            .save_snapshot(&repo)
            .unwrap_err()
            .message
            .contains("DOCS_REINDEX_REQUIRED")
    );
    let inline = f.input(
        "unsupported-inline-check.json",
        &serde_json::to_value(&legacy).unwrap(),
    );
    assert!(
        Check::load(&repo, &inline)
            .unwrap_err()
            .message
            .contains("DOCS_REINDEX_REQUIRED")
    );
    let before = cache::inventory(&repo).unwrap();
    let mut invalid = checked.clone();
    invalid.source_inputs.as_mut().unwrap().schema = "unsupported".into();
    assert!(
        invalid
            .save_snapshot(&repo)
            .unwrap_err()
            .message
            .contains("input contract")
    );
    let mut wrong_payload = checked.clone();
    wrong_payload
        .source_inputs
        .as_mut()
        .unwrap()
        .inputs
        .manifest
        .title = "Unbound title".into();
    assert!(wrong_payload.save_snapshot(&repo).is_err());
    let inline = f.input(
        "wrong-inline-source-inputs.json",
        &serde_json::to_value(&wrong_payload).unwrap(),
    );
    assert!(Check::load(&repo, &inline).is_err());
    let mut mismatched = checked;
    let mut contradictory_selection = mismatched.clone();
    contradictory_selection
        .source_inputs
        .as_mut()
        .unwrap()
        .retained_services
        .insert("orders".into());
    assert!(contradictory_selection.save_snapshot(&repo).is_err());
    mismatched
        .services
        .get_mut("orders")
        .unwrap()
        .service_digest = "wrong".into();
    assert!(
        mismatched
            .save_snapshot(&repo)
            .unwrap_err()
            .message
            .contains("captured service")
    );
    let after = cache::inventory(&repo).unwrap();
    assert_eq!(before.object_count, after.object_count);
    assert_eq!(before.object_bytes, after.object_bytes);
    assert!(
        Check::load_snapshot(&repo, &original)
            .unwrap()
            .source_inputs
            .is_some()
    );
}

#[test]
fn obsolete_check_envelopes_require_reindexing_without_rewriting_current_snapshot() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    let manifest = serde_json::to_value(checked.store_manifest(&repo).unwrap()).unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let mut variants = Vec::new();
    for field in ["sourceInputs", "dependenciesIndex"] {
        let mut missing = manifest.clone();
        missing.as_object_mut().unwrap().remove(field);
        variants.push(missing);
    }
    let mut whole_map = manifest.clone();
    whole_map["dependencies"] = whole_map["dependenciesIndex"].clone();
    whole_map
        .as_object_mut()
        .unwrap()
        .remove("dependenciesIndex");
    variants.push(whole_map);
    variants.push(serde_json::to_value(&checked).unwrap());
    for (i, variant) in variants.iter().enumerate() {
        let path = f.input(&format!("obsolete-{i}.json"), variant);
        let before = fs::read(&path).unwrap();
        let error = Check::load(&repo, &path).unwrap_err();
        assert!(error.message.contains("DOCS_REINDEX_REQUIRED"), "{error:?}");
        assert_eq!(fs::read(path).unwrap(), before);
    }
    assert_eq!(
        Check::load_snapshot(&repo, &handle).unwrap().context_digest,
        checked.context_digest
    );
}

#[test]
fn missing_source_input_object_fails_without_reacquiring_source() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    let manifest = checked.store_manifest(&repo).unwrap();
    let reference = manifest.source_inputs;
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let layout: Value = serde_json::from_slice(
        &fs::read(f.docs.join(".codeclew/cache/object-layout.json")).unwrap(),
    )
    .unwrap();
    let database = f.docs.join(layout["database"].as_str().unwrap());
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute(
            "DELETE FROM objects WHERE digest = ?1",
            rusqlite::params![reference.digest],
        )
        .unwrap();
    let latest = fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap();
    let error = Check::load_snapshot(&repo, &handle).unwrap_err();
    assert!(error.message.contains("source input manifest is missing"));
    let (code, error) = f.run(&["docs", "context", "--service", "orders"]);
    assert_ne!(code, 0);
    assert!(
        error
            .to_string()
            .contains("source input manifest is missing")
    );
    assert_eq!(
        latest,
        fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap()
    );
}
