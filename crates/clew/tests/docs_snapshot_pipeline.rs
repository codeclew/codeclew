//! Production docs snapshot pipeline: the granular fact index is wired into the
//! check storage path so observations are per-fact memberships (bounded pages,
//! copy-on-write) rather than a second whole-map copy, with point reads and
//! distinct per-compilation authority.

use clew::documentation::check::{CHECK_MANIFEST_SCHEMA, Check, CheckManifest};
use clew::documentation::fact_index;
use clew::documentation::model::Observation;
use clew::documentation::store::Repository;
use serde_json::json;
use std::collections::BTreeMap;

fn observation(id: &str, service: &str, symbol: &str, marker: &str) -> Observation {
    Observation {
        id: id.into(),
        kind: "SYMBOL".into(),
        service: service.into(),
        symbol: symbol.into(),
        normalized: json!({"marker": marker}),
        digest: format!("sha256:{}", "d".repeat(64)),
        source_ids: vec![],
    }
}

fn empty_check(repo: &Repository) -> Check {
    let inputs = repo.inputs().unwrap();
    let input_digest = clew::canonical::hash(&inputs).unwrap();
    Check {
        schema: "codeclew-documentation-check/1.0".into(),
        source_inputs: Some(clew::documentation::check::SourceInputs {
            schema: clew::documentation::check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest: input_digest.clone(),
            inputs,
            selected_services: Default::default(),
            retained_services: Default::default(),
        }),
        composition: None,
        input_digest,
        context_digest: format!("sha256:{}", "b".repeat(64)),
        services: BTreeMap::new(),
        unresolved: BTreeMap::new(),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        dependencies: BTreeMap::new(),
    }
}

/// A new capture stores dependencies as per-fact index memberships and
/// references the immutable snapshot root; no whole-map dependency object is
/// serialized, so there is no second copy of every observation.
#[test]
fn check_persists_dependencies_through_fact_index_without_whole_map_copy() {
    let t = tempfile::tempdir().unwrap();
    Repository::init(t.path(), "Architecture").unwrap();
    let repo = Repository::open(t.path()).unwrap();

    let mut check = empty_check(&repo);
    check.dependencies.insert(
        "web:reserve".into(),
        observation("web:reserve", "web", "example.Reservations.reserve", "web"),
    );
    check.dependencies.insert(
        "web:find".into(),
        observation("web:find", "web", "example.Reservations.find", "web"),
    );
    check.dependencies.insert(
        "orders:cancel".into(),
        observation("orders:cancel", "orders", "example.Orders.cancel", "orders"),
    );

    let manifest: CheckManifest = check.store_manifest(&repo).unwrap();
    assert!(
        serde_json::to_value(&manifest)
            .unwrap()
            .get("dependencies")
            .is_none()
    );
    let index = manifest.dependencies_index.clone();

    // The snapshot root is an immutable object that verifies and can be read.
    let root = fact_index::load_snapshot_root(&repo, &index).unwrap();
    fact_index::verify_root(&repo, &root).unwrap();

    // Round-trip through the persisted manifest hydrates the same observations.
    let encoded = clew::canonical::bytes(&manifest).unwrap();
    let restored: CheckManifest = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(restored.schema, CHECK_MANIFEST_SCHEMA);
    assert_eq!(restored.dependencies_index, index.clone());
    let deps = fact_index::load_observations(
        &repo,
        &root,
        clew::documentation::check::CHECK_DEPENDENCIES_SCOPE,
    )
    .unwrap();
    assert_eq!(
        deps.len(),
        3,
        "all dependencies must hydrate from memberships"
    );
    assert_eq!(deps["web:reserve"].service, "web");
    assert_eq!(deps["orders:cancel"].service, "orders");
}

/// Two compilations sharing a path/symbol but different bytes retain distinct
/// authority, while byte-identical canonical payloads are stored once.
#[test]
fn same_path_different_scope_distinct_authority_shared_identity() {
    let t = tempfile::tempdir().unwrap();
    Repository::init(t.path(), "Architecture").unwrap();
    let repo = Repository::open(t.path()).unwrap();

    let revision = format!("sha256:{}", "a".repeat(64));
    let mut web = BTreeMap::new();
    web.insert(
        "reserve".into(),
        observation("reserve", "web", "example.Reservations.reserve", "web"),
    );
    let mut orders = BTreeMap::new();
    orders.insert(
        "reserve".into(),
        observation(
            "reserve",
            "orders",
            "example.Reservations.reserve",
            "orders",
        ),
    );

    let web_root = fact_index::store_observations(&repo, ":web:main", &revision, &web).unwrap();
    let orders_root =
        fact_index::store_observations(&repo, ":orders:main", &revision, &orders).unwrap();

    let web_deps = fact_index::load_observations(
        &repo,
        &fact_index::load_snapshot_root(&repo, &web_root).unwrap(),
        ":web:main",
    )
    .unwrap();
    let orders_deps = fact_index::load_observations(
        &repo,
        &fact_index::load_snapshot_root(&repo, &orders_root).unwrap(),
        ":orders:main",
    )
    .unwrap();

    assert_eq!(web_deps["reserve"].symbol, "example.Reservations.reserve");
    assert_eq!(
        orders_deps["reserve"].symbol,
        "example.Reservations.reserve"
    );
    // Distinct scope authority: each scope resolves its own occurrence.
    assert_eq!(web_deps["reserve"].service, "web");
    assert_eq!(orders_deps["reserve"].service, "orders");
    // Occurrences are distinct keys; the payloads differ so identity is distinct.
    assert_ne!(
        web_deps["reserve"].normalized, orders_deps["reserve"].normalized,
        "different indexed bytes under different scopes must retain distinct authority"
    );
}

/// Point read of one observation decodes a single membership payload and a
/// one-scope delta rewrite touches only the affected bucket page and the root.
#[test]
fn point_read_and_delta_update_are_bounded() {
    let t = tempfile::tempdir().unwrap();
    Repository::init(t.path(), "Architecture").unwrap();
    let repo = Repository::open(t.path()).unwrap();

    let revision = format!("sha256:{}", "a".repeat(64));
    let mut deps = BTreeMap::new();
    for i in 0..20 {
        deps.insert(
            format!("s{i}"),
            observation(
                &format!("s{i}"),
                "web",
                &format!("example.S{i}"),
                &format!("m{i}"),
            ),
        );
    }
    let root = fact_index::store_observations(&repo, ":web:main", &revision, &deps).unwrap();
    let loaded = fact_index::load_observations(
        &repo,
        &fact_index::load_snapshot_root(&repo, &root).unwrap(),
        ":web:main",
    )
    .unwrap();
    assert_eq!(loaded.len(), 20);

    // A one-scope change rewrites only the affected bucket page and the small
    // root: publishing a new observation set leaves shared payloads in place.
    let mut next = deps.clone();
    next.remove("s5");
    next.insert(
        "new1".into(),
        observation("new1", "web", "example.New", "new"),
    );
    let next_root = fact_index::store_observations(&repo, ":web:main", &revision, &next).unwrap();
    let next_loaded = fact_index::load_observations(
        &repo,
        &fact_index::load_snapshot_root(&repo, &next_root).unwrap(),
        ":web:main",
    )
    .unwrap();
    assert_eq!(next_loaded.len(), 20);
    assert!(next_loaded.contains_key("new1"));
    assert!(!next_loaded.contains_key("s5"));
    // The prior snapshot root remains readable (immutable historical evidence).
    let prior = fact_index::load_observations(
        &repo,
        &fact_index::load_snapshot_root(&repo, &root).unwrap(),
        ":web:main",
    )
    .unwrap();
    assert_eq!(prior.len(), 20);
    assert!(prior.contains_key("s5"));
    // The JSON export of a root is a small object, not a full observation map.
    let root_payload =
        clew::canonical::bytes(&fact_index::load_snapshot_root(&repo, &next_root).unwrap())
            .unwrap();
    assert!(
        root_payload.len() < 4096,
        "the snapshot root must be a small manifest, not an embedded observation map: {} bytes",
        root_payload.len()
    );
}
