//! Retained consumers run in fresh CLI processes with the source unavailable.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{cache, check::Check, proposals, store::Repository, work};
use serde_json::{Value, json};
use std::fs;
use support::{Fixture, read};

fn capture(f: &Fixture, service: &str) -> String {
    let (_, value) = f.run(&["docs", "check", "--service", service]);
    value["snapshot"]
        .as_str()
        .unwrap_or_else(|| panic!("{value}"))
        .to_owned()
}
fn prepare(f: &Fixture, snapshot: &str, entrypoint: Option<&str>) -> Value {
    let request = f.input(
        "request.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers",
            "entrypoint":entrypoint, "maxItems":100, "maxBytes":49152
        }),
    );
    f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request.to_str().unwrap(),
        "--snapshot",
        snapshot,
    ])
}
fn read_all(f: &Fixture, mut page: Value) -> String {
    let id = page["work"].as_str().unwrap().to_owned();
    while let Some(cursor) = page["nextCursor"].as_str() {
        let input = f.input("read.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &id,
            "--input",
            input.to_str().unwrap(),
        ]);
    }
    id
}

fn object_database(repo: &Repository) -> std::path::PathBuf {
    let marker: Value = serde_json::from_slice(
        &fs::read(repo.root.join(".codeclew/cache/object-layout.json")).unwrap(),
    )
    .unwrap();
    repo.root.join(marker["database"].as_str().unwrap())
}

#[test]
fn retained_work_proposal_and_render_survive_latest_replacement_and_unavailable_sources() {
    let f = Fixture::new();
    let orders = f.service("orders");
    let other = f.service("other");
    let a = capture(&f, "orders");
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, &a).unwrap();
    let entry = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|e| e.symbol.contains("reserve"))
        .unwrap();
    let b = capture(&f, "other");
    assert_ne!(a, b);
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_b = fs::read(&latest).unwrap();
    // Successful reads must retain A even when acquisition cannot access either repository.
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    fs::rename(&other, other.with_extension("offline")).unwrap();
    let context = f.ok(&["docs", "context", "--service", "orders", "--snapshot", &a]);
    assert_eq!(context["contextDigest"], checked.context_digest);
    assert_eq!(context["authority"], "PINNED_SNAPSHOT_NOT_REVERIFIED");
    let page = prepare(&f, &a, Some(&entry.id));
    assert_eq!(page["snapshot"], a);
    let id = read_all(&f, page);
    let frozen = work::load(&repo, &id).unwrap();
    assert_eq!(frozen.checked.services.len(), 1);
    assert_eq!(
        frozen.checked.services["orders"].revision,
        checked.services["orders"].revision
    );
    proposals::current(&repo, &frozen).unwrap();
    let reference = |native: &str| {
        frozen
            .handles
            .iter()
            .find(|(_, h)| h.id == native)
            .unwrap()
            .0
            .clone()
    };
    let returned = checked.services["orders"]
        .observations
        .values()
        .find(|d| d.kind == "FLOW" && d.symbol == entry.symbol && d.normalized["kind"] == "RETURN")
        .unwrap();
    let proposed = f.input("proposal.json", &json!({"schema":"codeclew-documentation-proposal/1.0", "operations":[{
        "entrypoint":reference(&entry.id), "title":"Reserve quantity",
        "summary":{"text":"Processes the requested quantity.", "evidence":[reference(&entry.id)]},
        "steps":[{"kind":"note","meaning":{"text":"Returns the normalized quantity.","evidence":[reference(&returned.id)]}}]
    }]}));
    let submitted = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &id,
        "--input",
        proposed.to_str().unwrap(),
    ]);
    assert_eq!(submitted["status"], "READY_WITH_LIMITATIONS", "{submitted}");
    let published = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        submitted["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(published["snapshot"], a);
    assert_eq!(
        published["evidenceAuthority"],
        "PINNED_SNAPSHOT_NOT_REVERIFIED"
    );
    assert_eq!(published["documentedOperations"], 1);
    let data = read(f.bundle(
        published["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert_eq!(data["sectionState"]["freshness"], "UNVERIFIED");
    assert_eq!(data["sectionState"]["verification"], "UNASSESSED");
    assert!(
        data["sectionState"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "PINNED_SNAPSHOT_NOT_REVERIFIED")
    );
    let rerender = f.ok(&["docs", "render", "--snapshot", &a]);
    assert_eq!(rerender["documentedOperations"], 1);
    assert_eq!(fs::read(&latest).unwrap(), latest_b);
    assert_eq!(
        Check::load_snapshot(&repo, &a).unwrap().context_digest,
        checked.context_digest
    );
    assert_ne!(
        f.run(&["docs", "render", "--snapshot", &a, "--require-complete"])
            .0,
        0
    );
    // A later publication still invalidates work's optimistic content binding.
    assert!(proposals::current(&repo, &frozen).is_err());
}

#[test]
fn retained_snapshot_corruption_and_missing_payload_never_fall_back_to_latest() {
    let f = Fixture::new();
    f.service("orders");
    let snapshot = capture(&f, "orders");
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, &snapshot).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_bytes = fs::read(&latest).unwrap();
    let manifest = checked.store_manifest(&repo).unwrap();
    let digest = &manifest.service_manifests["orders"].sources.digest;
    let original = cache::get(
        &repo,
        &manifest.service_manifests["orders"].sources,
        128 * 1024 * 1024,
    )
    .unwrap()
    .unwrap();
    let mut corrupt = original.clone();
    corrupt[0] ^= 1;
    rusqlite::Connection::open(object_database(&repo))
        .unwrap()
        .execute(
            "UPDATE objects SET payload = ?1 WHERE digest = ?2",
            rusqlite::params![corrupt.as_slice(), digest],
        )
        .unwrap();
    assert!(Check::load_snapshot(&repo, &snapshot).is_err());
    assert_ne!(
        f.run(&[
            "docs",
            "context",
            "--service",
            "orders",
            "--snapshot",
            &snapshot
        ])
        .0,
        0
    );
    rusqlite::Connection::open(object_database(&repo))
        .unwrap()
        .execute(
            "DELETE FROM objects WHERE digest = ?1",
            rusqlite::params![digest],
        )
        .unwrap();
    assert_ne!(f.run(&["docs", "render", "--snapshot", &snapshot]).0, 0);
    assert!(!f.docs.join("docs/index.html").exists());
    assert_eq!(fs::read(latest).unwrap(), latest_bytes);
    for invalid in [
        "../latest-check.json",
        "sha256:bad/10",
        "sha256:bad/+10",
        "sha256:bad/010",
    ] {
        assert!(Check::load_snapshot(&repo, invalid).is_err());
    }
}

#[test]
fn retained_work_rejects_changed_declarations_and_unrelated_service_capture() {
    let f = Fixture::new();
    f.service("orders");
    f.service("other");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let all = checked.save_snapshot(&repo).unwrap();
    let request: work::Request = serde_json::from_value(
        json!({"schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers"}),
    )
    .unwrap();
    let prepared =
        work::prepare_with_snapshot(&repo, "service:orders".into(), request.clone(), Some(&all))
            .unwrap();
    let full = work::load(&repo, prepared["work"].as_str().unwrap()).unwrap();
    assert_eq!(full.snapshot.as_deref(), Some(all.as_str()));
    assert_eq!(
        serde_json::to_value(&full.checked).unwrap(),
        serde_json::to_value(&checked).unwrap()
    );
    let a = capture(&f, "orders");
    let id = read_all(&f, prepare(&f, &a, None));
    let frozen = work::load(&repo, &id).unwrap();
    fs::create_dir_all(f.docs.join("notes")).unwrap();
    fs::write(f.docs.join("notes/context.md"), "Changed author input").unwrap();
    assert!(proposals::current(&repo, &frozen).is_err());
    // Human files are separate authoring inputs; a new work may capture their
    // changed text. A changed service declaration invalidates the snapshot.
    let declaration_path = f.docs.join("catalog/services/orders.json");
    let mut declaration = read(&declaration_path);
    declaration["source"]["roots"] = json!(["src"]);
    fs::write(&declaration_path, serde_json::to_vec(&declaration).unwrap()).unwrap();
    assert!(
        work::prepare_with_snapshot(&repo, "service:orders".into(), request, Some(&a)).is_err()
    );
    assert_ne!(f.run(&["docs", "render", "--snapshot", &a]).0, 0);
}

#[test]
fn retained_work_generation_gap_never_reacquires_evidence() {
    let f = Fixture::new();
    let source = f.service("orders");
    let snapshot = capture(&f, "orders");
    let page = prepare(&f, &snapshot, None);
    let id = page["work"].as_str().unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    fs::write(&latest, b"generation gap must use retained snapshot").unwrap();
    let result = f.ok(&["docs", "work", "run", "--work", id]);
    assert_eq!(result["status"], "GENERATION_GAP");
    assert!(
        result["gap"]["reason"]
            .as_str()
            .unwrap()
            .contains("MISSING_EXECUTION_CONFIGURATION")
    );
    assert_eq!(
        fs::read(latest).unwrap(),
        b"generation gap must use retained snapshot"
    );
    let data = read(f.bundle(
        result["publication"]["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert!(
        data["updateFailures"]
            .to_string()
            .contains("GENERATION_GAP")
    );
    let repo = Repository::open(&f.docs).unwrap();
    assert_eq!(
        Check::load_snapshot(&repo, &snapshot)
            .unwrap()
            .services
            .len(),
        1
    );
}
