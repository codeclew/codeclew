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
fn legacy_source_inputs_remain_absent_and_invalid_contracts_cannot_be_saved() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let original = checked.save_snapshot(&repo).unwrap();
    let mut legacy = checked.clone();
    legacy.source_inputs = None;
    let legacy_handle = legacy.save_snapshot(&repo).unwrap();
    assert!(
        Check::load_snapshot(&repo, &legacy_handle)
            .unwrap()
            .source_inputs
            .is_none()
    );
    let encoded = serde_json::to_value(&legacy).unwrap();
    assert!(encoded.get("sourceInputs").is_none());
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
fn missing_source_input_object_fails_without_reacquiring_source() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let handle = checked.save_snapshot(&repo).unwrap();
    let manifest = checked.store_manifest(&repo).unwrap();
    let reference = manifest.source_inputs.unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    fs::remove_file(
        f.docs
            .join(".codeclew/cache/objects")
            .join(reference.digest)
            .join("object.json"),
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
