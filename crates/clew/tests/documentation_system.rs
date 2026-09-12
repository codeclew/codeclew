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

#[test]
fn docsys_t01_valid_update_survives_unavailable_service_and_invalid_input() {
    let f = Fixture::new();
    let absent = f.service("absent");
    let updated = f.service("updated");
    let checked = f.checked();
    let a = f.author("absent", &checked);
    let b = f.author("updated", &checked);
    let first = f.ok(&[
        "docs",
        "render",
        "--input",
        a.to_str().unwrap(),
        "--input",
        b.to_str().unwrap(),
    ]);
    let old = first["bundle"].as_str().unwrap();
    let old_data = read(f.bundle(old, "services/absent.json"));
    fs::rename(&absent, absent.with_extension("offline")).unwrap();
    let source = updated.join("Orders.java");
    fs::write(
        &source,
        fs::read_to_string(&source)
            .unwrap()
            .replace("return quantity;", "return quantity + 1;"),
    )
    .unwrap();
    commit(&updated);
    let checked = f.checked();
    let input = f.author("updated", &checked);
    let broken = f.temp.path().join("broken.json");
    fs::write(&broken, b"{ broken ").unwrap();
    let result = f.ok(&[
        "docs",
        "render",
        "--input",
        input.to_str().unwrap(),
        "--input",
        broken.to_str().unwrap(),
    ]);
    assert_eq!(result["status"], "PARTIAL");
    assert!(result["updateFailures"]["input-1"].is_object());
    let bundle = result["bundle"].as_str().unwrap();
    let absent_data = read(f.bundle(bundle, "services/absent.json"));
    assert_eq!(absent_data["operations"], old_data["operations"]);
    assert_eq!(
        absent_data["operationSources"],
        old_data["operationSources"]
    );
    assert_eq!(absent_data["sectionState"]["freshness"], "UNVERIFIED");
    let updated_data = read(f.bundle(bundle, "services/updated.json"));
    assert_eq!(updated_data["sectionState"]["freshness"], "CURRENT");
    let before = fs::read(f.docs.join("docs/index.html")).unwrap();
    assert_ne!(f.run(&["docs", "render", "--require-complete"]).0, 0);
    assert_eq!(fs::read(f.docs.join("docs/index.html")).unwrap(), before);
    let selected = f.run(&["docs", "check", "--service", "updated"]).1;
    assert_eq!(
        selected["unresolved"]["absent"]["reason"],
        "SERVICE_NOT_SELECTED"
    );
}

#[test]
fn docsys_t01_operations_keep_distinct_evidence_versions_in_one_page() {
    let f = Fixture::new();
    let repo = f.service("orders");
    let checked = f.checked();
    let reserve = f.author_selected("orders", &checked, "reserve");
    let normalize = f.author_selected("orders", &checked, "normalize");
    let mut both = read(&reserve);
    both["operations"]
        .as_array_mut()
        .unwrap()
        .push(read(&normalize)["operations"][0].clone());
    both["gaps"] = serde_json::json!({});
    let input = f.input("both.json", &both);
    let first = f.ok(&["docs", "render", "--input", input.to_str().unwrap()]);
    let old = read(f.bundle(first["bundle"].as_str().unwrap(), "services/orders.json"));
    let reserve_id = both["operations"][0]["id"].as_str().unwrap().to_owned();
    let normalize_id = both["operations"][1]["id"].as_str().unwrap().to_owned();
    let source = repo.join("Orders.java");
    fs::write(
        &source,
        fs::read_to_string(&source)
            .unwrap()
            .replace("return quantity;", "return quantity + 1;"),
    )
    .unwrap();
    commit(&repo);
    let checked = f.checked();
    let valid = f.author_selected("orders", &checked, "reserve");
    let mut candidate = read(&valid);
    let mut invalid = both["operations"][1].clone();
    invalid["summary"]["sourceIds"] = serde_json::json!(["invented-source"]);
    candidate["operations"]
        .as_array_mut()
        .unwrap()
        .push(invalid);
    candidate["gaps"] = serde_json::json!({});
    let input = f.input("mixed.json", &candidate);
    let result = f.ok(&["docs", "render", "--input", input.to_str().unwrap()]);
    let data = read(f.bundle(result["bundle"].as_str().unwrap(), "services/orders.json"));
    assert_eq!(data["operations"].as_array().unwrap().len(), 2);
    assert_eq!(data["operationStates"][&reserve_id]["freshness"], "CURRENT");
    assert_eq!(data["operationStates"][&normalize_id]["freshness"], "STALE");
    assert_eq!(
        data["operationSources"][&normalize_id],
        old["operationSources"][&normalize_id]
    );
    assert_eq!(
        data["operationStates"][&normalize_id]["contentRevisions"],
        old["operationStates"][&normalize_id]["contentRevisions"]
    );
    assert_eq!(
        data["sectionState"]["mixedRevisions"]["orders"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(result["updateFailures"][format!("service:orders/{normalize_id}")].is_object());
    let dossier = f.ok(&[
        "docs",
        "changes",
        "--fragment",
        &format!("service:orders/{normalize_id}/summary"),
    ]);
    assert!(
        dossier["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "SOURCE_CHANGE" && i["before"] != i["after"])
    );
    let unchanged = f.ok(&["docs", "render"]);
    assert_eq!(unchanged["updateFailures"], result["updateFailures"]);
    let markdown =
        fs::read_to_string(f.bundle(unchanged["bundle"].as_str().unwrap(), "services/orders.md"))
            .unwrap();
    assert!(
        markdown.contains("Source freshness: CURRENT")
            && markdown.contains("Source freshness: STALE")
    );
    let refresh = f.ok(&["docs", "refresh", "--status-only"]);
    let retained = read(f.bundle(refresh["bundle"].as_str().unwrap(), "services/orders.json"));
    assert_eq!(retained["operationSources"], data["operationSources"]);
    assert_eq!(
        retained["operationStates"][&normalize_id]["contentRevisions"],
        data["operationStates"][&normalize_id]["contentRevisions"]
    );
}

#[test]
fn docsys_t01_concurrent_refreshes_leave_one_complete_snapshot() {
    let f = Fixture::new();
    let repo = f.service("orders");
    let checked = f.checked();
    let input = f.author("orders", &checked);
    f.ok(&["docs", "render", "--input", input.to_str().unwrap()]);
    let source = repo.join("Orders.java");
    fs::write(
        &source,
        fs::read_to_string(&source)
            .unwrap()
            .replace("return quantity;", "return quantity + 2;"),
    )
    .unwrap();
    commit(&repo);
    let outcomes = std::thread::scope(|scope| {
        let a = scope.spawn(|| f.run(&["docs", "refresh", "--status-only"]));
        let b = scope.spawn(|| f.run(&["docs", "refresh", "--status-only"]));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert!(outcomes.iter().any(|(code, _)| *code == 0), "{outcomes:?}");
    let repository = clew::documentation::store::Repository::open(&f.docs).unwrap();
    let (id, bindings) = clew::documentation::bindings::baseline(&repository)
        .unwrap()
        .unwrap();
    clew::documentation::bindings::verify_outputs(&repository, &id, &bindings).unwrap();
    assert_eq!(
        bindings.section_states["service:orders"].freshness,
        clew::documentation::model::Freshness::Stale
    );
    assert!(
        outcomes
            .iter()
            .filter(|(code, _)| *code == 0)
            .all(|(_, result)| result["bundle"] == id)
    );
}
