//! Semantic source-syntax captures must not reuse or persist incomplete provider authority.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{
        analysis, cache,
        check::Check,
        model::{SOURCE_EXTRACTOR, Service},
        store::Repository,
    },
};
use serde_json::json;
use std::fs;
use support::{Fixture, read};

#[test]
fn enabled_semantic_source_capture_bypasses_old_reusable_overlay() {
    check_semantic_capture(false);
}

#[test]
fn legacy_semantic_source_capture_bypasses_old_reusable_overlay() {
    check_semantic_capture(true);
}

fn check_semantic_capture(legacy: bool) {
    let f = Fixture::new();
    let source = f.service("orders");
    let baseline = f.checked();
    let mut configured = read(f.docs.join("catalog/services/orders.json"));
    configured["modules"] = json!({
        "schema":"codeclew-documentation-modules/1.0",
        "semantic":{
            "module":"javac",
            "enabled":true,
            "profile":"java-17plus-maven-read-only",
            "compilation":":/main"
        }
    });
    if legacy {
        configured.as_object_mut().unwrap().remove("modules");
        configured["source"]["semantic"] = json!({
            "profile":"java-17plus-maven-read-only", "compilation":":/main"
        });
    }
    let configured_service: Service = serde_json::from_value(configured.clone()).unwrap();
    let revision = analysis::git(&source, &["rev-parse", "--verify", "HEAD^{commit}"]).unwrap();
    let service_digest = canonical::hash(&configured_service).unwrap();
    let cache_key = canonical::hash(&json!([
        revision,
        service_digest,
        SOURCE_EXTRACTOR,
        &configured_service.language
    ]))
    .unwrap();

    let mut old = baseline.services["orders"].clone();
    old.service_digest = canonical::hash(&configured_service).unwrap();
    let scope = old
        .observations
        .values_mut()
        .find(|observation| observation.kind == "SOURCE_SCOPE")
        .unwrap();
    scope.normalized["semantic"] = json!({
        "provider":{"status":"AVAILABLE","reason":"OLD_REUSABLE_OVERLAY"},
        "facts":{}
    });
    scope.digest = canonical::hash(&scope.normalized).unwrap();
    analysis::verify_evidence(&old).unwrap();
    cache::save_capture(
        &Repository::open(&f.docs).unwrap(),
        "orders",
        &cache_key,
        &old,
    )
    .unwrap();

    let input = f.input("semantic-service.json", &configured);
    let current_digest = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        &current_digest,
    ]);

    let checked = f.checked();
    let evidence = &checked.services["orders"];
    let scope = evidence
        .observations
        .values()
        .find(|observation| observation.kind == "SOURCE_SCOPE")
        .unwrap();
    assert_eq!(
        scope.normalized["semantic"]["provider"]["status"],
        "UNAVAILABLE"
    );
    assert_ne!(
        scope.normalized["semantic"]["provider"]["reason"],
        "OLD_REUSABLE_OVERLAY"
    );

    let repo = Repository::open(&f.docs).unwrap();
    let manifest_path = cache::capture_manifest_path(&repo, "orders", &cache_key).unwrap();
    let manifest = read(manifest_path);
    assert_eq!(manifest["cacheability"], "NON_CACHEABLE");
    assert!(
        manifest["reason"]
            .as_str()
            .unwrap()
            .contains("semantic provider")
    );

    let snapshot = checked.save_snapshot(&repo).unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let reopened = Check::load_snapshot(&repo, &snapshot).unwrap();
    assert_eq!(reopened.context_digest, checked.context_digest);
}
