#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use std::fs;
use support::{Fixture, commit, read};

#[test]
fn docsys_t00_stale_status_retains_content_without_agents() {
    let f = Fixture::new();
    let changed = f.service("orders");
    f.service("other");
    let checked = f.checked();
    let a = f.author("orders", &checked);
    let b = f.author("other", &checked);
    let published = f.ok(&[
        "docs",
        "render",
        "--input",
        a.to_str().unwrap(),
        "--input",
        b.to_str().unwrap(),
    ]);
    let old = published["bundle"].as_str().unwrap();
    let old_page = fs::read(f.bundle(old, "services/orders.html")).unwrap();
    let old_data = read(f.bundle(old, "services/orders.json"));
    let human = f.docs.join("notes.txt");
    fs::write(&human, b"Keep my original note\n").unwrap();
    let source = changed.join("Orders.java");
    fs::write(
        &source,
        fs::read_to_string(&source)
            .unwrap()
            .replace("return quantity;", "return quantity + 1;"),
    )
    .unwrap();
    commit(&changed);
    let refreshed = f.ok(&["docs", "refresh", "--status-only"]);
    assert_eq!(refreshed["agentInvocations"], 0);
    assert_eq!(
        refreshed["sections"]["service:orders"]["freshness"],
        "STALE"
    );
    assert_eq!(
        refreshed["sections"]["service:other"]["freshness"],
        "CURRENT"
    );
    let new = refreshed["bundle"].as_str().unwrap();
    assert_ne!(old, new);
    let data = read(f.bundle(new, "services/orders.json"));
    assert_eq!(data["operations"], old_data["operations"]);
    assert_eq!(data["sources"], old_data["sources"]);
    assert_ne!(
        data["sectionState"]["contentRevisions"],
        data["sectionState"]["targetRevisions"]
    );
    assert_eq!(data["sectionState"]["verification"], "UNASSESSED");
    for state in data["operationStates"].as_object().unwrap().values() {
        assert_eq!(state["freshness"], "STALE");
    }
    assert!(
        fs::read_to_string(f.bundle(new, "services/orders.md"))
            .unwrap()
            .contains("Source freshness: STALE")
    );
    let operation_id = data["operations"][0]["id"].as_str().unwrap();
    assert!(
        fs::read_to_string(f.bundle(new, &format!("diagrams/service-orders-{operation_id}.mmd")))
            .unwrap()
            .starts_with("%% Source freshness: STALE")
    );
    assert_eq!(
        fs::read(f.bundle(old, "services/orders.html")).unwrap(),
        old_page
    );
    assert_eq!(fs::read(&human).unwrap(), b"Keep my original note\n");
    if let Ok(path) = std::env::var("CODECLEW_DOCSYS_REVIEW_HTML") {
        fs::write(
            path,
            fs::read(f.bundle(new, "services/orders.html")).unwrap(),
        )
        .unwrap();
    }
    let repeated = f.ok(&["docs", "refresh", "--status-only"]);
    assert_eq!(repeated["bundle"], refreshed["bundle"]);
    let (code, report) = f.run(&["docs", "check"]);
    assert_eq!(code, 4, "{report}"); // A stale status snapshot remains a valid baseline.
    assert_eq!(report["status"], "CHECKED");
}

#[test]
fn docsys_t00_missing_source_is_local_and_preserves_originals() {
    let f = Fixture::new();
    let unavailable = f.service("orders");
    f.service("other");
    let checked = f.checked();
    let a = f.author("orders", &checked);
    let b = f.author("other", &checked);
    f.ok(&[
        "docs",
        "render",
        "--input",
        a.to_str().unwrap(),
        "--input",
        b.to_str().unwrap(),
    ]);
    fs::rename(&unavailable, unavailable.with_extension("offline")).unwrap();
    let result = f.ok(&["docs", "refresh", "--status-only"]);
    assert_eq!(
        result["sections"]["service:orders"]["freshness"],
        "UNVERIFIED"
    );
    assert!(result["sections"]["service:orders"]["targetRevisions"]["orders"].is_null());
    assert_eq!(result["sections"]["service:other"]["freshness"], "CURRENT");
    let index = fs::read(f.docs.join("docs/index.html")).unwrap();
    let bundle = result["bundle"].as_str().unwrap();
    fs::write(
        f.bundle(bundle, "services/other.md"),
        "Human modification\n",
    )
    .unwrap();
    assert_ne!(f.run(&["docs", "refresh", "--status-only"]).0, 0);
    assert_eq!(fs::read(f.docs.join("docs/index.html")).unwrap(), index);
}
