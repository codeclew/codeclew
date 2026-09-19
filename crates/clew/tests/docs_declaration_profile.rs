//! Declaration profiles change transmission, not evidence or freshness authority.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{model::Observation, proposals, store::Repository, work},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs};
use support::{Fixture, commit};

fn request(profile: Option<&str>, max_items: u32, max_bytes: usize) -> work::Request {
    let mut value = json!({"schema":"codeclew-documentation-work-request/1.0",
        "audience":"Service maintainers", "entrypoint":"section-entities",
        "maxItems":max_items, "maxBytes":max_bytes});
    if let Some(profile) = profile {
        value["contextProfile"] = json!(profile);
    }
    serde_json::from_value(value).unwrap()
}

fn all_pages(repo: &Repository, mut page: Value) -> Vec<Value> {
    let id = page["work"].as_str().unwrap().to_owned();
    let mut pages = Vec::new();
    loop {
        let cursor = page["nextCursor"].as_str().map(str::to_owned);
        pages.push(page);
        let Some(cursor) = cursor else { break };
        page = work::read(
            repo,
            &id,
            work::Selection {
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .unwrap();
    }
    pages
}

fn items(pages: &[Value]) -> Vec<Value> {
    pages
        .iter()
        .flat_map(|p| p["items"].as_array().unwrap().iter().cloned())
        .collect()
}

fn proposal(target: &str, evidence: &str) -> proposals::Proposal {
    serde_json::from_value(json!({"schema":"codeclew-documentation-proposal/1.0",
        "operations":[{"entrypoint":target,"title":"Order declarations",
        "summary":{"text":"The selected source contains quantity processing declarations.",
        "evidence":[evidence]},"steps":[]}]}))
    .unwrap()
}

#[test]
fn profile_preserves_mandatory_inputs_full_influence_and_unknown_facts_offline() {
    let f = Fixture::new();
    let source = f.service("orders");
    fs::create_dir_all(f.docs.join("notes")).unwrap();
    fs::write(
        f.docs.join("notes/policy.md"),
        "HUMAN_POLICY_SENTINEL: ownership needs review.",
    )
    .unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let mut checked = f.checked();
    let domain = Observation {
        id: "entity:global-order".into(),
        kind: "DOMAIN_ENTITY".into(),
        service: "global-domain".into(),
        symbol: "global-order".into(),
        normalized: json!({"meaning":"GLOBAL_DOMAIN_SENTINEL"}),
        digest: canonical::hash(&json!({"meaning":"GLOBAL_DOMAIN_SENTINEL"})).unwrap(),
        source_ids: Vec::new(),
    };
    checked.dependencies.insert(domain.id.clone(), domain);
    let unknown = Observation {
        id: "orders:future-analyzer-fact".into(),
        kind: "FUTURE_ANALYZER_FACT".into(),
        service: "orders".into(),
        symbol: "unfamiliar-but-required".into(),
        normalized: json!({"meaning":"UNKNOWN_KIND_SENTINEL"}),
        digest: canonical::hash(&json!({"meaning":"UNKNOWN_KIND_SENTINEL"})).unwrap(),
        source_ids: Vec::new(),
    };
    checked
        .dependencies
        .insert(unknown.id.clone(), unknown.clone());
    checked
        .services
        .get_mut("orders")
        .unwrap()
        .observations
        .insert(unknown.id.clone(), unknown);
    let snapshot = checked.save_snapshot(&repo).unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    fs::write(&latest, b"Do not reacquire analysis").unwrap();
    let full = work::prepare_with_snapshot(
        &repo,
        "service:orders".into(),
        request(None, 5, 49152),
        Some(&snapshot),
    )
    .unwrap();
    let full_id = full["work"].as_str().unwrap().to_owned();
    let foreign_cursor = full["nextCursor"].as_str().unwrap().to_owned();
    let full_rows = items(&all_pages(&repo, full));
    let profile = work::prepare_with_snapshot(
        &repo,
        "service:orders".into(),
        request(Some("declarations-v1"), 5, 49152),
        Some(&snapshot),
    )
    .unwrap();
    let id = profile["work"].as_str().unwrap().to_owned();
    assert_ne!(id, full_id);
    assert!(
        work::read(
            &repo,
            &id,
            work::Selection {
                cursor: Some(foreign_cursor),
                ..Default::default()
            }
        )
        .is_err()
    );
    let pages = all_pages(&repo, profile);
    assert!(
        pages
            .iter()
            .all(|page| page["contextProfile"] == "declarations-v1")
    );
    let rows = items(&pages);
    let a = work::load(&repo, &full_id).unwrap();
    let b = work::load(&repo, &id).unwrap();
    assert_eq!(a.influence, b.influence);
    assert_eq!(
        serde_json::to_value(&a.handles).unwrap(),
        serde_json::to_value(&b.handles).unwrap()
    );
    assert!(
        serde_json::to_value(&a.request)
            .unwrap()
            .get("contextProfile")
            .is_none()
    );
    for kind in ["EXTERNAL_INPUT", "REVIEW_REASON", "OBLIGATION"] {
        let expected: Vec<_> = full_rows.iter().filter(|row| row["kind"] == kind).collect();
        assert!(!expected.is_empty(), "fixture must exercise {kind}");
        let delivered: Vec<_> = rows.iter().filter(|row| row["kind"] == kind).collect();
        assert_eq!(expected, delivered, "required {kind} changed");
    }
    assert!(
        rows.iter()
            .any(|row| row.to_string().contains("HUMAN_POLICY_SENTINEL"))
    );
    assert!(
        rows.iter()
            .any(|row| row["record"]["kind"] == "FUTURE_ANALYZER_FACT")
    );
    assert!(!rows.iter().any(|row| matches!(
        row["record"]["kind"].as_str(),
        Some("FLOW" | "SYNTAX_DETAIL")
    )));
    assert_eq!(
        rows.iter().filter(|row| row["kind"] == "SECTION").count(),
        1
    );
    let summaries: Vec<_> = rows
        .iter()
        .filter(|row| row["kind"] == "CONTEXT_PROFILE")
        .collect();
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0]["reference"].is_null());
    assert_eq!(summaries[0]["referenceRoles"], json!([]));
    let selected: Vec<_> = rows
        .iter()
        .filter(|row| row["kind"] != "CONTEXT_PROFILE")
        .map(|row| json!([row["kind"], row["id"]]))
        .collect();
    assert_eq!(summaries[0]["record"]["selectedCount"], selected.len());
    assert_eq!(
        summaries[0]["record"]["selectedMembershipDigest"],
        canonical::hash(&selected).unwrap()
    );
    assert!(rows.iter().any(|row| row["id"] == "entity:global-order"));
    let inventory = &full_rows
        .iter()
        .find(|row| row["kind"] == "BOUNDARY_INVENTORY")
        .unwrap()["record"];
    for field in ["gaps", "sourceBoundaries"] {
        assert_eq!(summaries[0]["record"][field], inventory[field]);
    }
    assert_eq!(
        summaries[0]["record"]["inventoryDigest"],
        canonical::hash(inventory).unwrap()
    );
    for field in ["publicBoundaries", "internalCallables"] {
        assert_eq!(
            summaries[0]["record"]["inventory"][field],
            inventory[field].as_array().unwrap().len()
        );
    }
    let source_rows: Vec<_> = rows.iter().filter(|row| row["kind"] == "SOURCE").collect();
    assert!(!source_rows.is_empty());
    let source_ids: BTreeSet<_> = source_rows
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(source_ids.len(), source_rows.len());
    for row in rows.iter().filter(|row| row["record"]["kind"] == "SYMBOL") {
        for source in row["record"]["sourceIds"].as_array().unwrap() {
            assert!(source_ids.contains(source.as_str().unwrap()));
        }
    }
    assert!(work::initial_context_complete(
        &work::read_state(&repo, &id).unwrap()
    ));
    // The compact summary is not a replacement for inspectable inventory.
    let section = b
        .handles
        .iter()
        .find(|(_, h)| h.kind == "SECTION" && h.id == "section-entities")
        .unwrap()
        .0
        .clone();
    let mut expanded = Vec::new();
    let mut cursor = None;
    loop {
        let page = work::read(
            &repo,
            &id,
            work::Selection {
                references: vec![section.clone()],
                cursor,
                ..Default::default()
            },
        )
        .unwrap();
        expanded.extend(page["items"].as_array().unwrap().iter().cloned());
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        expanded
            .iter()
            .find(|row| row["kind"] == "BOUNDARY_INVENTORY"),
        full_rows
            .iter()
            .find(|row| row["kind"] == "BOUNDARY_INVENTORY")
    );
    assert_eq!(fs::read(&latest).unwrap(), b"Do not reacquire analysis");
    proposals::current(&repo, &b).unwrap();
    fs::write(f.docs.join("notes/policy.md"), "Changed ownership policy.").unwrap();
    let stale = proposals::current(&repo, &b).unwrap_err();
    assert!(
        stale.message.contains("work inputs changed"),
        "{}",
        stale.message
    );
}

#[test]
fn deferred_facts_need_actual_expansion_and_full_work_receipts_do_not_transfer() {
    let f = Fixture::new();
    f.service("orders");
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = f.checked().save_snapshot(&repo).unwrap();
    let full = work::prepare_with_snapshot(
        &repo,
        "service:orders".into(),
        request(None, 100, 49152),
        Some(&snapshot),
    )
    .unwrap();
    all_pages(&repo, full);
    let first = work::prepare_with_snapshot(
        &repo,
        "service:orders".into(),
        request(Some("declarations-v1"), 1, 49152),
        Some(&snapshot),
    )
    .unwrap();
    let id = first["work"].as_str().unwrap().to_owned();
    let frozen = work::load(&repo, &id).unwrap();
    let target = frozen
        .handles
        .iter()
        .find(|(_, h)| h.kind == "SECTION" && h.id == "section-entities")
        .unwrap()
        .0;
    let deferred = frozen
        .handles
        .iter()
        .find(|(_, h)| h.kind == "DEPENDENCY" && frozen.checked.dependencies[&h.id].kind == "FLOW")
        .unwrap()
        .0;
    assert!(!work::initial_context_complete(
        &work::read_state(&repo, &id).unwrap()
    ));
    let incomplete = proposals::submit(&repo, &id, proposal(target, deferred)).unwrap();
    assert_eq!(incomplete["status"], "NEEDS_REPAIR");
    all_pages(&repo, first);
    let state = work::read_state(&repo, &id).unwrap();
    assert!(
        state
            .receipts
            .values()
            .all(|receipt| !receipt.supplied.contains(deferred))
    );
    let unread = proposals::submit(&repo, &id, proposal(target, deferred)).unwrap();
    assert_eq!(unread["status"], "NEEDS_REPAIR");
    let expanded = work::read(
        &repo,
        &id,
        work::Selection {
            references: vec![deferred.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    // maxItems=1 may paginate expansion; preserve its registered selection.
    let mut cursor = expanded["nextCursor"].as_str().map(str::to_owned);
    while let Some(next) = cursor {
        let page = work::read(
            &repo,
            &id,
            work::Selection {
                references: vec![deferred.clone()],
                cursor: Some(next),
                ..Default::default()
            },
        )
        .unwrap();
        cursor = page["nextCursor"].as_str().map(str::to_owned);
    }
    let ready = proposals::submit(&repo, &id, proposal(target, deferred)).unwrap();
    assert!(
        ready["status"].as_str().unwrap().starts_with("READY_"),
        "{ready}"
    );
}

#[test]
fn unsupported_profile_is_rejected_before_acquisition() {
    let f = Fixture::new();
    let source = f.service("orders");
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    for (profile, section) in [
        ("unknown-v1", "section-entities"),
        ("declarations-v1", "section-overview"),
    ] {
        let mut request = request(Some(profile), 20, 40960);
        request.entrypoint = Some(section.into());
        let error = work::prepare(&repo, "service:orders".into(), request).unwrap_err();
        assert!(error.message.contains("PROFILE"), "{}", error.message);
    }
    assert!(!f.docs.join(".codeclew/cache/latest-check.json").exists());
}

#[test]
fn oversized_declaration_source_is_required_and_never_marked_read() {
    let f = Fixture::new();
    let source = f.service("orders");
    fs::write(
        source.join("Orders.java"),
        format!(
            "class Orders {{ int quantity(int n) {{ /* {} */ return n; }} }}",
            "source evidence ".repeat(600)
        ),
    )
    .unwrap();
    commit(&source);
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = f.checked().save_snapshot(&repo).unwrap();
    let first = work::prepare_with_snapshot(
        &repo,
        "service:orders".into(),
        request(Some("declarations-v1"), 100, 4096),
        Some(&snapshot),
    )
    .unwrap();
    let id = first["work"].as_str().unwrap().to_owned();
    let pages = all_pages(&repo, first);
    let source_omissions: Vec<_> = pages
        .iter()
        .flat_map(|page| page["omitted"].as_array().unwrap())
        .filter(|row| row["kind"] == "SOURCE")
        .collect();
    assert!(
        !source_omissions.is_empty(),
        "large exact source must be a required profile member"
    );
    let state = work::read_state(&repo, &id).unwrap();
    assert!(!work::initial_context_complete(&state));
    for omitted in source_omissions {
        let reference = omitted["reference"].as_str().unwrap();
        assert!(
            state
                .receipts
                .values()
                .all(|receipt| !receipt.supplied.iter().any(|r| r == reference))
        );
    }
}
