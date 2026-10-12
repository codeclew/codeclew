use clew::thread_values::{FrontierKind, Nullness, ValueQuery, analyze};
use clew_facts::values::{
    ModelBinding, ObservationAuthority, SCHEMA, ValueModel, ValueNode, ValueOperation as Op,
};

fn model(mut nodes: Vec<(&str, Op)>) -> ValueModel {
    nodes.sort_by_key(|n| n.0);
    ValueModel {
        schema: SCHEMA.into(),
        binding: ModelBinding {
            language: "language:java".into(),
            producer: "synthetic-equations".into(),
            snapshot_digest: format!("sha256:{}", "a".repeat(64)),
            facts_digest: format!("sha256:{}", "b".repeat(64)),
            configuration_digest: format!("sha256:{}", "c".repeat(64)),
            policy_digest: format!("sha256:{}", "d".repeat(64)),
            observation_authority: ObservationAuthority::Declared,
        },
        nodes: nodes
            .into_iter()
            .map(|(id, operation)| ValueNode {
                id: id.into(),
                operation,
                evidence_ids: vec![format!("evidence:{id}")],
                guards: vec![],
            })
            .collect(),
        global_boundaries: vec![],
    }
}
fn query(target: &str) -> ValueQuery {
    ValueQuery {
        target: target.into(),
        max_nodes: 128,
        max_transfers: 2048,
    }
}
fn copy(id: &str) -> Op {
    Op::Copy { value: id.into() }
}
fn coalesce(left: &str, right: &str) -> Op {
    Op::Coalesce {
        left: left.into(),
        right: right.into(),
    }
}
fn value(model: &ValueModel, target: &str) -> Nullness {
    analyze(model, &query(target))
        .unwrap()
        .observations
        .into_iter()
        .find(|n| n.id == target)
        .unwrap()
        .normal_value_nullness
}

#[test]
fn dto_input_overrides_null_default_and_preserves_input_dependency() {
    let m = model(vec![
        ("input-status", Op::Input),
        ("pending-default", Op::NonNull),
        ("entity-status", coalesce("input-status", "pending-default")),
        ("unrelated", Op::Null),
    ]);
    let r = analyze(&m, &query("entity-status")).unwrap();
    assert_eq!(value(&m, "entity-status"), Nullness::NonNull);
    assert_eq!(
        r.observations
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["entity-status", "input-status", "pending-default"]
    );
    // Non-null says nothing about which business status the caller selected.
    assert!(!r.program_semantic_closure);
    assert_eq!(r.calculation_authority, "DECLARED_MODEL_EQUATIONS");
}
#[test]
fn branch_merge_never_turns_nullable_alternative_into_nonnull() {
    let m = model(vec![
        ("null", Op::Null),
        ("nonnull", Op::NonNull),
        (
            "merge",
            Op::Join {
                values: vec!["nonnull".into(), "null".into()],
            },
        ),
        ("return", copy("merge")),
    ]);
    assert_eq!(value(&m, "return"), Nullness::Unknown);
}
#[test]
fn guards_remain_relevant_without_proving_feasibility() {
    let mut m = model(vec![
        ("input", Op::Input),
        ("result", Op::NonNull),
        ("unrelated", Op::Null),
    ]);
    m.nodes
        .iter_mut()
        .find(|n| n.id == "result")
        .unwrap()
        .guards
        .push("input".into());
    let r = analyze(&m, &query("result")).unwrap();
    assert_eq!(r.observations.len(), 2);
    assert_eq!(value(&m, "result"), Nullness::NonNull);
}
#[test]
fn loop_fixpoint_and_unseeded_recursion_are_distinct() {
    let seeded = model(vec![
        (
            "loop",
            Op::Join {
                values: vec!["loop".into(), "seed".into()],
            },
        ),
        ("seed", Op::NonNull),
    ]);
    assert_eq!(value(&seeded, "loop"), Nullness::NonNull);
    let unseeded = model(vec![("a", copy("b")), ("b", copy("a"))]);
    let r = analyze(&unseeded, &query("a")).unwrap();
    assert!(r.equations_converged);
    assert!(
        r.observations
            .iter()
            .all(|n| n.normal_value_nullness == Nullness::Unknown)
    );
    assert!(
        r.frontier
            .iter()
            .any(|f| f.code == "NO_MODELED_NORMAL_VALUE_UNKNOWN")
    );
}
#[test]
fn unknown_mapper_and_exception_gaps_remain_explicit() {
    let mut m = model(vec![
        (
            "mapper",
            Op::Unknown {
                boundary: "MAPPER_TRANSFER_UNAVAILABLE".into(),
            },
        ),
        ("return", copy("mapper")),
    ]);
    m.global_boundaries
        .push("EXCEPTION_MODEL_UNAVAILABLE".into());
    let r = analyze(&m, &query("return")).unwrap();
    assert_eq!(value(&m, "return"), Nullness::Unknown);
    assert!(
        r.frontier
            .iter()
            .any(|f| f.code == "EXCEPTION_MODEL_UNAVAILABLE")
    );
    assert!(
        r.frontier
            .iter()
            .any(|f| f.code == "MAPPER_TRANSFER_UNAVAILABLE")
    );
}
#[test]
fn node_and_transfer_cuts_never_publish_partial_fixpoint_as_proof() {
    let m = model(vec![
        ("a-result", copy("b-default")),
        ("b-default", Op::NonNull),
    ]);
    let mut q = query("a-result");
    q.max_nodes = 1;
    let r = analyze(&m, &q).unwrap();
    assert_eq!(r.observations[0].normal_value_nullness, Nullness::Unknown);
    assert!(r.frontier.iter().any(|f| f.kind == FrontierKind::Budget));
    q.max_nodes = 128;
    q.max_transfers = 1;
    let r = analyze(&m, &q).unwrap();
    assert!(!r.equations_converged);
    assert!(
        r.observations
            .iter()
            .all(|n| n.normal_value_nullness == Nullness::Unknown)
    );
}
#[test]
fn producer_language_and_authority_do_not_change_equations_or_upgrade_closure() {
    let mut m = model(vec![
        ("input", Op::Input),
        ("fallback", Op::NonNull),
        ("result", coalesce("input", "fallback")),
    ]);
    for language in [
        "java",
        "kotlin",
        "csharp",
        "typescript",
        "javascript",
        "python",
        "rust",
    ] {
        for authority in [
            ObservationAuthority::CompilerObserved,
            ObservationAuthority::SyntaxObserved,
            ObservationAuthority::Declared,
        ] {
            m.binding.language = format!("language:{language}");
            m.binding.observation_authority = authority;
            let r = analyze(&m, &query("result")).unwrap();
            assert_eq!(r.binding.observation_authority, authority);
            assert!(!r.program_semantic_closure);
            assert_eq!(value(&m, "result"), Nullness::NonNull);
            assert_eq!(r, analyze(&m, &query("result")).unwrap());
        }
    }
}
#[test]
fn absent_value_is_discovery_unknown_and_bindings_invalidate_digest() {
    let mut m = model(vec![("input", Op::Input)]);
    let a = analyze(&m, &query("missing")).unwrap();
    assert!(a.observations.is_empty());
    assert!(a.frontier.iter().any(|f| f.kind == FrontierKind::Discovery));
    m.binding.configuration_digest = format!("sha256:{}", "e".repeat(64));
    let b = analyze(&m, &query("missing")).unwrap();
    assert_ne!(a.model_digest, b.model_digest);
}
#[test]
fn definitions_and_calls_use_occurrence_ids_not_names() {
    let m = model(vec![
        (
            "call-one",
            Op::Unknown {
                boundary: "CALL_TRANSFER_UNAVAILABLE".into(),
            },
        ),
        ("call-two", Op::NonNull),
        ("return", copy("call-one")),
    ]);
    let r = analyze(&m, &query("return")).unwrap();
    assert!(r.observations.iter().all(|n| n.id != "call-two"));
    assert_eq!(value(&m, "return"), Nullness::Unknown);
}
#[test]
fn invalid_graph_evidence_and_budgets_fail_closed() {
    let m = model(vec![("input", Op::Input)]);
    let mut invalid = m.clone();
    invalid.nodes.push(invalid.nodes[0].clone());
    assert!(analyze(&invalid, &query("input")).is_err());
    let mut invalid = m.clone();
    invalid.nodes[0].operation = copy("absent");
    assert!(analyze(&invalid, &query("input")).is_err());
    let mut invalid = m.clone();
    invalid.nodes[0].evidence_ids.clear();
    assert!(analyze(&invalid, &query("input")).is_err());
    let mut invalid = m.clone();
    invalid.binding.facts_digest = "unbound".into();
    assert!(analyze(&invalid, &query("input")).is_err());
    let mut q = query("input");
    q.max_nodes = 0;
    assert!(analyze(&m, &q).is_err());
    let mut json = serde_json::to_value(&m).unwrap();
    json["fir"] = true.into();
    assert!(serde_json::from_value::<ValueModel>(json).is_err());
}

#[test]
fn unseeded_cycle_unknown_propagates_into_previously_nonnull_merge() {
    let m = model(vec![
        ("a-cycle", copy("b-cycle")),
        ("b-cycle", copy("a-cycle")),
        (
            "c-result",
            Op::Join {
                values: vec!["a-cycle".into(), "d-seed".into()],
            },
        ),
        ("d-seed", Op::NonNull),
    ]);
    let complete = analyze(&m, &query("c-result")).unwrap();
    assert_eq!(value(&m, "c-result"), Nullness::Unknown);
    assert!(complete.equations_converged);
    // Five transfers finish phase one; the next visit is bottom fallback propagation.
    let mut q = query("c-result");
    q.max_transfers = 5;
    let cut = analyze(&m, &q).unwrap();
    assert!(!cut.equations_converged);
    assert!(
        cut.observations
            .iter()
            .all(|n| n.normal_value_nullness == Nullness::Unknown)
    );
    assert!(
        cut.frontier
            .iter()
            .any(|f| f.code == "FIXPOINT_TRANSFER_BUDGET")
    );
}

#[test]
fn actual_project_digest_binds_and_oversized_references_fail_before_scan() {
    let mut m = model(vec![("input", Op::Input)]);
    m.binding.facts_digest = clew_facts::digest(&serde_json::json!({"retained": true})).unwrap();
    assert!(analyze(&m, &query("input")).is_ok());
    let mut oversized = m.clone();
    oversized.nodes[0].evidence_ids =
        vec!["unscanned".into(); clew_facts::values::MAX_DEPENDENCIES + 1];
    assert_eq!(oversized.validate(), Err("value reference budget exceeded"));
    let mut oversized = m;
    oversized.nodes[0].operation = Op::Join {
        values: vec!["unscanned".into(); clew_facts::values::MAX_DEPENDENCIES + 1],
    };
    assert_eq!(
        oversized.validate(),
        Err("value dependency budget exceeded")
    );
}
