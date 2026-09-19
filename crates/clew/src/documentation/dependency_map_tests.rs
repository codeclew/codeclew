use super::*;
use crate::documentation::{
    check::{self, Check},
    model::Observation,
    store::Repository,
};
use serde_json::json;
use std::{collections::BTreeMap, fs};

fn setup() -> (tempfile::TempDir, Repository) {
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Dependency map tests").unwrap();
    let repo = Repository::open(temporary.path()).unwrap();
    (temporary, repo)
}

fn observation(
    id: &str,
    service: &str,
    symbol: &str,
    ordinal: usize,
    source_ids: Vec<String>,
) -> Observation {
    let normalized = json!({"ordinal": ordinal, "symbol": symbol});
    Observation {
        id: id.into(),
        kind: "SYNTHETIC_FACT".into(),
        service: service.into(),
        symbol: symbol.into(),
        digest: crate::canonical::hash(&normalized).unwrap(),
        normalized,
        source_ids,
    }
}

fn check_with_dependencies(
    repo: &Repository,
    title: &str,
    dependencies: BTreeMap<String, Observation>,
) -> Check {
    let mut inputs = repo.inputs().unwrap();
    inputs.manifest.title = title.into();
    let input_digest = crate::canonical::hash(&inputs).unwrap();
    Check {
        schema: "codeclew-documentation-check/1.0".into(),
        input_digest: input_digest.clone(),
        context_digest: "context".into(),
        services: BTreeMap::new(),
        unresolved: BTreeMap::new(),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        dependencies,
        source_inputs: Some(check::SourceInputs {
            schema: check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest,
            inputs,
            selected_services: Default::default(),
            retained_services: Default::default(),
        }),
        composition: None,
    }
}

#[test]
fn check_dependency_index_is_independent_of_check_input_digest() {
    let (_temporary, repo) = setup();
    let dependencies = BTreeMap::from([(
        "orders:Widget".into(),
        observation("orders:Widget", "orders", "Widget", 1, Vec::new()),
    )]);

    // Leave an unrelated ambient scope in the mutable index. The exact Check
    // writer must still start from its own empty root.
    let ambient = store_observations(
        &repo,
        "ambient-scope",
        "ambient-revision",
        &BTreeMap::from([(
            "ambient".into(),
            observation("ambient", "ambient", "Ambient", 0, Vec::new()),
        )]),
    )
    .unwrap();
    publish_root(&repo, &load_snapshot_root(&repo, &ambient).unwrap()).unwrap();
    let ambient_before = fs::read(repo.root.join(ROOT_PATH)).unwrap();

    let first = check_with_dependencies(&repo, "input-A", dependencies.clone())
        .store_manifest(&repo)
        .unwrap();
    let second = check_with_dependencies(&repo, "input-B", dependencies)
        .store_manifest(&repo)
        .unwrap();
    assert_eq!(
        first.dependencies_index, second.dependencies_index,
        "dependency-map identity must not include the enclosing input digest"
    );
    assert_ne!(
        crate::canonical::hash(&first).unwrap(),
        crate::canonical::hash(&second).unwrap(),
        "the enclosing Check identity still binds its inputs"
    );
    assert_eq!(fs::read(repo.root.join(ROOT_PATH)).unwrap(), ambient_before);
    let reference = first.dependencies_index;
    let loaded =
        load_snapshot_observations(&repo, &reference, check::CHECK_DEPENDENCIES_SCOPE).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded["orders:Widget"].symbol, "Widget");
}

#[test]
fn dependency_map_changes_one_bucket_for_one_full_observation_update() {
    let (_temporary, repo) = setup();
    let mut original = BTreeMap::new();
    for index in 0..160 {
        let service = if index % 3 == 0 { "orders" } else { "payments" };
        let id = format!("{service}:Fact{index}");
        original.insert(
            id.clone(),
            observation(&id, service, &format!("Fact{index}"), index, vec![]),
        );
    }
    let first_reference = store_dependency_map(&repo, &original).unwrap();
    let first_root = load_snapshot_root(&repo, &first_reference).unwrap();
    assert!(first_root.buckets.iter().flatten().count() > 1);

    let mut updated = original.clone();
    updated
        .get_mut("payments:Fact37")
        .unwrap()
        .source_ids
        .push("source-only-change".into());
    let second_reference = store_dependency_map(&repo, &updated).unwrap();
    let second_root = load_snapshot_root(&repo, &second_reference).unwrap();
    let changed_buckets = first_root
        .buckets
        .iter()
        .zip(&second_root.buckets)
        .filter(|(old, new)| old != new)
        .count();
    assert_eq!(changed_buckets, 1, "one payload update rewrites one page");
    assert!(
        first_root
            .buckets
            .iter()
            .zip(&second_root.buckets)
            .any(|(old, new)| old.is_some() && old == new)
    );
    assert_eq!(
        load_snapshot_observations(&repo, &first_reference, check::CHECK_DEPENDENCIES_SCOPE,)
            .unwrap(),
        original
    );
    assert_eq!(
        load_snapshot_observations(&repo, &second_reference, check::CHECK_DEPENDENCIES_SCOPE,)
            .unwrap(),
        updated
    );

    let before_repeat = cache::inventory(&repo).unwrap();
    let repeated = store_dependency_map(&repo, &updated).unwrap();
    let after_repeat = cache::inventory(&repo).unwrap();
    assert_eq!(repeated, second_reference);
    assert_eq!(before_repeat.object_count, after_repeat.object_count);
    assert_eq!(before_repeat.object_bytes, after_repeat.object_bytes);
}

#[test]
fn dependency_map_is_exact_and_survives_ambient_root_corruption() {
    let (_temporary, repo) = setup();
    let old = BTreeMap::from([
        (
            "orders:Shared".into(),
            observation("orders:Shared", "orders", "Shared", 1, vec!["a".into()]),
        ),
        (
            "payments:Shared".into(),
            observation("payments:Shared", "payments", "Shared", 2, vec!["b".into()]),
        ),
    ]);

    // Preserve the generic writer's legacy snapshot contract while making its
    // mutable root unrelated to the dedicated check map.
    let generic = store_observations(&repo, "legacy-scope", "legacy-revision", &old).unwrap();
    assert_eq!(
        load_snapshot_observations(&repo, &generic, "legacy-scope").unwrap(),
        old
    );

    let old_reference = store_dependency_map(&repo, &old).unwrap();
    let next = BTreeMap::from([
        (
            "payments:Shared".into(),
            observation("payments:Shared", "payments", "Shared", 2, vec!["b".into()]),
        ),
        (
            "warehouse:New".into(),
            observation("warehouse:New", "warehouse", "New", 3, vec!["c".into()]),
        ),
    ]);
    let next_reference = store_dependency_map(&repo, &next).unwrap();
    assert_eq!(
        load_snapshot_observations(&repo, &old_reference, check::CHECK_DEPENDENCIES_SCOPE,)
            .unwrap(),
        old
    );
    assert_eq!(
        load_snapshot_observations(&repo, &next_reference, check::CHECK_DEPENDENCIES_SCOPE,)
            .unwrap(),
        next
    );

    let empty_reference = store_dependency_map(&repo, &BTreeMap::new()).unwrap();
    assert!(
        load_snapshot_observations(&repo, &empty_reference, check::CHECK_DEPENDENCIES_SCOPE,)
            .unwrap()
            .is_empty()
    );

    // Immutable snapshot reads and dedicated writes do not consult the
    // mutable generic root, even when that root is corrupt.
    fs::write(
        repo.root.join(".codeclew/cache/fact-index.json"),
        b"corrupt",
    )
    .unwrap();
    let corrupt_root = fs::read(repo.root.join(".codeclew/cache/fact-index.json")).unwrap();
    assert_eq!(
        load_snapshot_observations(&repo, &old_reference, check::CHECK_DEPENDENCIES_SCOPE,)
            .unwrap(),
        old
    );
    let after_corruption = store_dependency_map(&repo, &next).unwrap();
    assert_eq!(after_corruption, next_reference);
    assert_eq!(
        fs::read(repo.root.join(".codeclew/cache/fact-index.json")).unwrap(),
        corrupt_root,
        "standalone snapshots must not repair or publish the mutable root"
    );
}
