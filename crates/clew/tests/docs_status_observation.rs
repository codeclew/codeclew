//! Status observations preserve authored evidence and never reacquire source.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{bindings, model::Narrative, render, status, store::Repository};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use support::{Fixture, commit, git};

fn published() -> (Fixture, PathBuf) {
    published_with_other(false)
}

fn published_with_other(other: bool) -> (Fixture, PathBuf) {
    let f = Fixture::new();
    let source = f.service("orders");
    if other {
        f.service("billing");
    }
    git(&source, &["branch", "selected"]);
    let catalog = f.docs.join("catalog/services/orders.json");
    let mut service: Value = serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    service["targetRef"] = json!("selected");
    fs::write(&catalog, serde_json::to_vec(&service).unwrap()).unwrap();
    let checked = f.checked();
    let input = f.author("orders", &checked);
    let narrative: Narrative = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    render::publish_from_snapshot(&repo, vec![narrative], false, Default::default(), &snapshot)
        .unwrap();
    (f, source)
}

fn policy(f: &Fixture, service: &str, source_ref: &str) {
    let directory = f.docs.join("catalog/update-policy");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(format!("{service}.json")),
        serde_json::to_vec(&json!({
            "schema":"codeclew-documentation-update-policy/1.0", "service":service,
            "repositoryId":service, "acceptedRefs":[source_ref]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn target(f: &Fixture, service: &str, revision: &str, sequence: u64) {
    fs::write(f.docs.join("catalog/update-state.json"), serde_json::to_vec(&json!({
        "schema":"codeclew-documentation-update-state/1.0", "targets":{service:{
            "schema":"codeclew-documentation-update-event/1.0", "id":"new-target", "service":service,
            "repositoryId":service, "sourceRef":"refs/heads/selected", "revision":revision, "sequence":sequence
        }}
    })).unwrap()).unwrap();
}

#[test]
fn offline_status_preserves_content_and_latest_and_repeated_observation_is_idempotent() {
    let (f, source) = published();
    let repo = Repository::open(&f.docs).unwrap();
    let (_, before) = bindings::baseline(&repo).unwrap().unwrap();
    assert!(!before.narratives["service:orders"].operations.is_empty());
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    // Observation must neither hydrate nor replace the acquisition convenience pointer.
    fs::write(&latest, b"unavailable acquisition pointer").unwrap();
    let first = status::refresh(&repo).unwrap();
    assert_eq!(first["captures"], 0);
    let (bundle, after) = bindings::baseline(&repo).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&before.narratives).unwrap(),
        serde_json::to_value(&after.narratives).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&before.fragments).unwrap(),
        serde_json::to_value(&after.fragments).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&before.observations).unwrap(),
        serde_json::to_value(&after.observations).unwrap()
    );
    assert_eq!(before.revisions, after.revisions);
    for (id, original) in &before.section_states {
        let observed = &after.section_states[id];
        assert_eq!(original.content_revisions, observed.content_revisions);
        assert_eq!(original.mixed_revisions, observed.mixed_revisions);
        assert_eq!(original.verification, observed.verification);
    }
    let section = &after.section_states["service:orders"];
    assert_eq!(section.freshness.as_str(), "UNVERIFIED");
    assert!(
        section
            .reasons
            .iter()
            .any(|r| r["reason"] == "LOCAL_TARGET_UNAVAILABLE")
    );
    let count = fs::read_dir(f.docs.join("docs/generated")).unwrap().count();
    let index = fs::read(f.docs.join("docs/index.html")).unwrap();
    let second = status::refresh(&repo).unwrap();
    assert_eq!(second["status"], "UNCHANGED");
    assert_eq!(second["bundle"], bundle);
    assert_eq!(
        count,
        fs::read_dir(f.docs.join("docs/generated")).unwrap().count()
    );
    assert_eq!(index, fs::read(f.docs.join("docs/index.html")).unwrap());
    assert_eq!(
        fs::read(latest).unwrap(),
        b"unavailable acquisition pointer"
    );
}

#[test]
fn observed_target_ref_is_not_head_and_stale_cannot_be_cleared_by_ref_reversion() {
    let (f, source) = published();
    let repo = Repository::open(&f.docs).unwrap();
    let (_, before) = bindings::baseline(&repo).unwrap().unwrap();
    let old = before.revisions["orders"].clone();
    fs::write(source.join("unrelated.txt"), "new head").unwrap();
    commit(&source);
    let unchanged = status::refresh(&repo).unwrap();
    assert_eq!(
        unchanged["sections"]["service:orders"]["targetRevisions"]["orders"],
        old
    );
    assert_eq!(
        unchanged["sections"]["service:orders"]["freshness"],
        "UNVERIFIED"
    );
    git(&source, &["branch", "-f", "selected", "HEAD"]);
    let changed = status::refresh(&repo).unwrap();
    assert_eq!(changed["sections"]["service:orders"]["freshness"], "STALE");
    git(&source, &["branch", "-f", "selected", &old]);
    let reverted = status::refresh(&repo).unwrap();
    assert_eq!(reverted["sections"]["service:orders"]["freshness"], "STALE");
    let (_, after) = bindings::baseline(&repo).unwrap().unwrap();
    assert_eq!(before.revisions, after.revisions);
}

#[test]
fn coordinator_target_is_observed_even_without_a_local_checkout() {
    let (f, source) = published();
    let repo = Repository::open(&f.docs).unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let target = "f".repeat(40);
    policy(&f, "orders", "refs/heads/selected");
    self::target(&f, "orders", &target, 1);
    let observed = status::refresh(&repo).unwrap();
    assert_eq!(
        observed["sections"]["service:orders"]["targetRevisions"]["orders"],
        target
    );
    assert_eq!(observed["sections"]["service:orders"]["freshness"], "STALE");
    assert_eq!(
        observed["observation"]["observations"]["orders"]["reason"],
        "SELECTED_TARGET_NOT_RECHECKED"
    );
}

#[test]
fn status_refresh_preserves_existing_manual_output_conflict_guard() {
    let (f, _) = published();
    let repo = Repository::open(&f.docs).unwrap();
    let (bundle, _) = bindings::baseline(&repo).unwrap().unwrap();
    let index = fs::read(f.docs.join("docs/index.html")).unwrap();
    fs::write(f.bundle(&bundle, "services/orders.md"), "manual edit").unwrap();
    assert!(status::refresh(&repo).is_err());
    assert_eq!(index, fs::read(f.docs.join("docs/index.html")).unwrap());
}

#[test]
fn unrelated_coordinator_event_and_same_revision_sequence_do_not_make_orders_stale() {
    let (f, _) = published_with_other(true);
    let repo = Repository::open(&f.docs).unwrap();
    let (_, before) = bindings::baseline(&repo).unwrap().unwrap();
    assert!(
        !before.section_states["service:orders"]
            .content_revisions
            .contains_key("billing")
    );
    policy(&f, "billing", "refs/heads/selected");
    target(&f, "billing", &before.revisions["billing"], 1);
    let first = status::refresh(&repo).unwrap();
    assert_eq!(
        first["sections"]["service:orders"]["freshness"],
        "UNVERIFIED"
    );
    assert_eq!(
        first["sections"]["service:billing"]["freshness"],
        "UNVERIFIED"
    );
    target(&f, "billing", &before.revisions["billing"], 2);
    let repeated = status::refresh(&repo).unwrap();
    assert_eq!(repeated["status"], "UNCHANGED");
    assert_eq!(repeated["bundle"], first["bundle"]);
    target(&f, "billing", &"f".repeat(40), 3);
    let changed = status::refresh(&repo).unwrap();
    assert_eq!(
        changed["sections"]["service:orders"]["freshness"],
        "UNVERIFIED"
    );
    assert_eq!(changed["sections"]["service:billing"]["freshness"], "STALE");
    assert!(
        changed["sections"]["service:orders"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason["reason"] == "DECLARATION_SCOPE_NOT_RECHECKED")
    );
}

#[test]
fn policy_retarget_rejects_the_old_event_without_falling_back_to_local_ref() {
    let (f, _) = published();
    let repo = Repository::open(&f.docs).unwrap();
    let (_, before) = bindings::baseline(&repo).unwrap().unwrap();
    policy(&f, "orders", "refs/heads/selected");
    target(&f, "orders", &before.revisions["orders"], 1);
    let admitted = status::refresh(&repo).unwrap();
    assert_eq!(
        admitted["sections"]["service:orders"]["targetRevisions"]["orders"],
        before.revisions["orders"]
    );
    policy(&f, "orders", "refs/heads/replacement");
    let rejected = status::refresh(&repo).unwrap();
    assert!(rejected["sections"]["service:orders"]["targetRevisions"]["orders"].is_null());
    assert_eq!(
        rejected["observation"]["observations"]["orders"]["reason"],
        "SELECTED_TARGET_NOT_ADMITTED"
    );
    assert_eq!(
        rejected["sections"]["service:orders"]["freshness"],
        "UNVERIFIED"
    );
    assert!(
        !rejected["sections"]["service:orders"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason["reason"] == "SELECTED_TARGET_NOT_RECHECKED")
    );
}

#[test]
fn observation_reasons_replace_prior_targets_but_preserve_review_obligations() {
    let (f, _) = published();
    let repo = Repository::open(&f.docs).unwrap();
    let (_, mut binding) = bindings::baseline(&repo).unwrap().unwrap();
    let original = binding.revisions["orders"].clone();
    binding
        .section_states
        .get_mut("service:orders")
        .unwrap()
        .reasons
        .push(json!({"reason":"MEANING_REVIEW_REQUIRED"}));
    policy(&f, "orders", "refs/heads/selected");
    let mut count = None;
    for (sequence, digit) in ['a', 'b', 'c', 'd', 'e', 'f'].into_iter().enumerate() {
        target(
            &f,
            "orders",
            &digit.to_string().repeat(40),
            sequence as u64 + 1,
        );
        status::observe(&repo, &mut binding).unwrap();
        let state = &binding.section_states["service:orders"];
        assert_eq!(state.freshness.as_str(), "STALE");
        assert_eq!(
            state
                .reasons
                .iter()
                .filter(|reason| reason["reason"] == "TARGET_REVISION_CHANGED")
                .count(),
            1
        );
        assert!(
            state
                .reasons
                .iter()
                .any(|reason| reason["reason"] == "MEANING_REVIEW_REQUIRED")
        );
        if let Some(count) = count {
            assert_eq!(state.reasons.len(), count);
        }
        count = Some(state.reasons.len());
    }
    target(&f, "orders", &original, 7);
    status::observe(&repo, &mut binding).unwrap();
    let state = &binding.section_states["service:orders"];
    assert_eq!(state.freshness.as_str(), "STALE");
    assert!(
        !state
            .reasons
            .iter()
            .any(|reason| reason["reason"] == "TARGET_REVISION_CHANGED")
    );
    assert!(
        state
            .reasons
            .iter()
            .any(|reason| reason["reason"] == "PRIOR_RECHECK_REQUIRED")
    );
}
