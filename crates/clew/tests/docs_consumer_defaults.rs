//! Consumers retain one immutable saved check when current source is offline.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{cache, store::Repository, work},
};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use support::{Fixture, read};

fn request(f: &Fixture) -> std::path::PathBuf {
    f.input(
        "consumer-work-request.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0",
            "audience":"Service maintainers",
            "maxItems":20,
            "maxBytes":40960
        }),
    )
}

fn work_path(f: &Fixture, id: &str) -> std::path::PathBuf {
    f.docs.join(format!(".codeclew/work/{id}/work.json"))
}

fn write_inline(repo: &Repository, mut value: work::Work) -> String {
    value.schema = "codeclew-documentation-work/1.0".into();
    value.id.clear();
    value.id = canonical::hash(&value).unwrap()[7..].into();
    let path = work_path_for(repo, &value.id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, canonical::bytes(&value).unwrap()).unwrap();
    value.id
}

fn work_path_for(repo: &Repository, id: &str) -> std::path::PathBuf {
    repo.root.join(format!(".codeclew/work/{id}/work.json"))
}

fn install_git_sentinel(f: &Fixture) -> std::path::PathBuf {
    let marker = f.temp.path().join("git-sentinel.marker");
    let git = f.temp.path().join("tools/git");
    fs::remove_file(&git).unwrap();
    fs::write(
        &git,
        format!(
            "#!/bin/sh\necho attempted >> {}\nexit 99\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
    marker
}

#[test]
fn default_consumers_retain_one_saved_check_offline() {
    let f = Fixture::new();
    let orders = f.service("orders");
    let inventory = f.service("inventory");
    let checked = f.checked();
    assert_eq!(checked.services.len(), 2);
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_before = fs::read(&latest_path).unwrap();
    let marker = install_git_sentinel(&f);

    let rendered = f.ok(&["docs", "render"]);
    assert_eq!(
        rendered["evidenceAuthority"],
        "PINNED_SNAPSHOT_NOT_REVERIFIED"
    );
    let input = request(&f);
    let first = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        input.to_str().unwrap(),
    ]);
    let first_id = first["work"].as_str().unwrap();
    let stored = read(work_path(&f, first_id));
    assert_eq!(stored["evidenceSnapshot"], snapshot);
    assert!(stored.get("checked").is_none());
    assert_eq!(stored["snapshot"], snapshot);
    let retained = work::load(&repo, first_id).unwrap().checked;
    assert_eq!(
        canonical::hash(&retained).unwrap(),
        canonical::hash(&checked).unwrap()
    );
    assert_eq!(retained.services.len(), 2);

    let context = f.ok(&["docs", "context", "--service", "orders"]);
    assert_eq!(context["snapshot"], snapshot);
    assert_eq!(context["authority"], "PINNED_SNAPSHOT_NOT_REVERIFIED");
    assert!(!marker.exists(), "consumer attempted source acquisition");
    assert!(orders.exists() && inventory.exists());

    let after_first = cache::inventory(&repo).unwrap();
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    fs::rename(&inventory, inventory.with_extension("offline")).unwrap();
    let second = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        input.to_str().unwrap(),
    ]);
    let after_second = cache::inventory(&repo).unwrap();
    assert_eq!(second["work"], first["work"]);
    assert_eq!(after_second.object_count, after_first.object_count);
    assert_eq!(after_second.object_bytes, after_first.object_bytes);
    assert_eq!(fs::read(latest_path).unwrap(), latest_before);
}

#[test]
fn missing_corrupt_and_stale_latest_fail_without_fallback() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let _snapshot = checked.save_snapshot(&repo).unwrap();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let input = request(&f);

    fs::remove_file(&latest_path).unwrap();
    for args in [
        vec![
            "docs",
            "work",
            "prepare",
            "--subject",
            "service:orders",
            "--input",
            input.to_str().unwrap(),
        ],
        vec!["docs", "context", "--service", "orders"],
        vec!["docs", "render"],
    ] {
        let (code, value) = f.run(&args);
        assert_ne!(code, 0, "{args:?}: {value}");
        assert!(
            value.to_string().contains("docs check"),
            "{args:?}: {value}"
        );
    }

    let f = Fixture::new();
    f.service("orders");
    f.checked();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_before = fs::read(&latest_path).unwrap();
    fs::write(&latest_path, b"corrupt latest check").unwrap();
    let input = request(&f);
    let (code, value) = f.run(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(value.to_string().contains("docs check"));
    assert_eq!(fs::read(&latest_path).unwrap(), b"corrupt latest check");
    assert_ne!(fs::read(&latest_path).unwrap(), latest_before);

    let f = Fixture::new();
    f.service("orders");
    f.checked();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_before = fs::read(&latest_path).unwrap();
    f.service("inventory");
    let input = request(&f);
    let (code, value) = f.run(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(value.to_string().contains("stale") || value.to_string().contains("STALE"));
    assert_eq!(fs::read(latest_path).unwrap(), latest_before);
}

#[test]
fn legacy_inline_work_requires_reindex_before_agent_run() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let input = request(&f);
    let prepared = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        input.to_str().unwrap(),
        "--snapshot",
        &snapshot,
    ]);
    let mut runtime = work::load(&repo, prepared["work"].as_str().unwrap()).unwrap();
    runtime.snapshot = None;
    let legacy = write_inline(&repo, runtime);
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let (code, value) = f.run(&["docs", "work", "run", "--work", &legacy]);
    assert_ne!(code, 0);
    assert!(value.to_string().contains("DOCS_REINDEX_REQUIRED"));
    assert!(
        work::load(&repo, &legacy)
            .unwrap_err()
            .message
            .contains("DOCS_REINDEX_REQUIRED")
    );
}

#[test]
fn default_work_remains_pinned_through_validation_and_failure_publication() {
    let f = Fixture::new();
    let orders = f.service("orders");
    let inventory = f.service("inventory");
    let a = f.run(&["docs", "check", "--service", "orders"]).1["snapshot"]
        .as_str()
        .unwrap()
        .to_owned();
    let input = request(&f);
    let prepared = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        input.to_str().unwrap(),
    ]);
    let id = prepared["work"].as_str().unwrap();
    let (code, report) = f.run(&["docs", "check", "--service", "inventory"]);
    assert_eq!(code, 3, "{report}");
    assert!(report["snapshot"].is_string());
    let marker = install_git_sentinel(&f);
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    fs::rename(&inventory, inventory.with_extension("offline")).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    fs::write(&latest, b"unrelated unreadable latest").unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let frozen = work::load(&repo, id).unwrap();
    assert_eq!(frozen.snapshot.as_deref(), Some(a.as_str()));
    clew::documentation::proposals::current(&repo, &frozen).unwrap();
    let result = f.ok(&["docs", "work", "run", "--work", id]);
    assert_eq!(result["status"], "GENERATION_GAP");
    assert!(result["publication"]["bundle"].is_string(), "{result}");
    let data = read(f.bundle(
        result["publication"]["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert!(data["sectionState"]["reasons"].to_string().contains(&a));
    assert!(!marker.exists());
    assert_eq!(fs::read(latest).unwrap(), b"unrelated unreadable latest");
}

#[test]
fn unrelated_service_snapshot_fails_without_capturing_available_source() {
    let f = Fixture::new();
    f.service("orders");
    f.service("inventory");
    let (code, report) = f.run(&["docs", "check", "--service", "inventory"]);
    assert_eq!(code, 3, "{report}");
    assert!(report["snapshot"].is_string());
    let marker = install_git_sentinel(&f);
    let input = request(&f);
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    let before = fs::read(&latest).unwrap();
    for args in [
        vec![
            "docs",
            "work",
            "prepare",
            "--subject",
            "service:orders",
            "--input",
            input.to_str().unwrap(),
        ],
        vec!["docs", "context", "--service", "orders"],
    ] {
        let (code, value) = f.run(&args);
        assert_ne!(code, 0, "{value}");
        assert!(value.to_string().contains("missing from retained evidence"));
    }
    assert!(!marker.exists());
    assert_eq!(fs::read(latest).unwrap(), before);
}

#[test]
fn auxiliary_readers_use_saved_evidence_with_producers_unavailable() {
    let f = Fixture::new();
    f.service("orders");
    f.service("inventory");
    f.checked();
    f.ok(&["docs", "render"]);
    let marker = install_git_sentinel(&f);
    let interaction=f.input("candidate.json",&json!({"schema":"codeclew-documentation-interaction/1.0","id":"candidate","title":"Declared relationship","from":{"service":"orders"},"to":{"service":"inventory"},"transport":{"kind":"http","method":"POST","path":"/quantity"},"declaration":{"origin":"human","rationale":"An explicit candidate, not proven runtime transport."}}));
    let candidate = f.ok(&[
        "docs",
        "interaction",
        "candidates",
        "--input",
        interaction.to_str().unwrap(),
    ]);
    assert_eq!(candidate["authority"], "PINNED_SNAPSHOT_NOT_REVERIFIED");
    let process=f.input("transient.json",&json!({"schema":"codeclew-documentation-process/1.0","id":"transient","title":"Quantity flow","summary":"A transient process over saved evidence.","root":{"service":"orders","selector":{"language":"java","owner":"Orders","name":"reserve","parameterTypes":["int"]}},"interactions":[],"maxDepth":4,"maxNodes":64,"process":{"scope":"Saved quantity flow","participants":["orders"],"objects":[],"trigger":"A caller provides quantity.","outcomes":["Returns the quantity."],"linkedSubviews":[]}}));
    let inspected = f.ok(&[
        "docs",
        "process",
        "inspect",
        "--input",
        process.to_str().unwrap(),
    ]);
    assert_eq!(inspected["authority"], "PINNED_SNAPSHOT_NOT_REVERIFIED");
    assert_eq!(inspected["saved"], false);
    let changes = f.ok(&["docs", "changes"]);
    assert_eq!(changes["authority"], "PINNED_SNAPSHOT_NOT_REVERIFIED");
    assert!(!marker.exists(), "auxiliary consumer attempted acquisition");
}

#[test]
fn retained_render_does_not_promote_unknown_to_stale_or_clear_observed_staleness() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let narrative = f.author("orders", &checked);
    let first = f.ok(&["docs", "render", "--input", narrative.to_str().unwrap()]);
    let first_data = read(f.bundle(first["bundle"].as_str().unwrap(), "services/orders.json"));
    let operation = first_data["operations"][0]["id"].as_str().unwrap();
    assert_eq!(
        first_data["operationStates"][operation]["freshness"],
        "UNVERIFIED"
    );
    let repeated = f.ok(&["docs", "render"]);
    let repeated_data =
        read(f.bundle(repeated["bundle"].as_str().unwrap(), "services/orders.json"));
    assert_eq!(
        repeated_data["operationStates"][operation]["freshness"],
        "UNVERIFIED"
    );
    fs::write(source.join("change.txt"), b"A committed participant change").unwrap();
    support::commit(&source);
    let observed = f.ok(&["docs", "refresh", "--status-only"]);
    let observed_data =
        read(f.bundle(observed["bundle"].as_str().unwrap(), "services/orders.json"));
    assert_eq!(
        observed_data["operationStates"][operation]["freshness"],
        "STALE"
    );
    let republished = f.ok(&["docs", "render"]);
    let data = read(f.bundle(
        republished["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert_eq!(data["operationStates"][operation]["freshness"], "STALE");
    assert_eq!(data["operationSources"], first_data["operationSources"]);
    assert_eq!(data["operations"], first_data["operations"]);
}
