//! Status observations preserve authored evidence and never reacquire source.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{
    bindings::Bindings, freshness as status, model::Narrative, retained_validation,
    store::Repository,
};
use serde_json::json;
use std::fs;
use support::Fixture;

fn fixture() -> (Fixture, Bindings) {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let input = f.author("orders", &checked);
    let narrative: Narrative = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    let binding = retained_validation::make_bindings(
        &checked,
        std::collections::BTreeMap::from([("service:orders".into(), narrative)]),
    )
    .unwrap();
    (f, binding)
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
fn observation_reasons_replace_prior_targets_but_preserve_review_obligations() {
    let (f, mut binding) = fixture();
    let repo = Repository::open(&f.docs).unwrap();
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
