use clew::thread_flow_cfg::{
    LocalCfgEdge, LocalCfgEdgeKind as E, LocalCfgNode, LocalCfgNodeRole as N, LocalCfgPayload, seal,
};
use clew::thread_question_control::{OrderQuery, observe};

fn graph() -> LocalCfgPayload {
    let mut payload = LocalCfgPayload {
        schema: "local-cfg/0.1".into(),
        graph_id: String::new(),
        owner_symbol_identity: "callable:sample/process#jvm:process()V".into(),
        file: "src/Process.kt".into(),
        compiler_graph_name: "sample/process".into(),
        provider: "K2_FIR_CFG".into(),
        source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
        nodes: vec![
            node(0, N::Entry),
            node(1, N::Decision),
            node(2, N::Operation),
            node(3, N::Operation),
            node(4, N::Exit),
            node(5, N::Dead),
        ],
        edges: vec![
            edge(0, 1, E::Next),
            edge(1, 2, E::True),
            edge(1, 3, E::False),
            edge(2, 3, E::Next),
            edge(3, 2, E::LoopBack),
            edge(3, 4, E::Exception),
            edge(0, 5, E::Dead),
            edge(5, 3, E::Dead),
        ],
    };
    payload.edges.sort();
    seal(payload).unwrap()
}
fn node(node_id: u64, role: N) -> LocalCfgNode {
    LocalCfgNode {
        node_id,
        role,
        source: None,
    }
}
fn edge(source_node_id: u64, target_node_id: u64, kind: E) -> LocalCfgEdge {
    LocalCfgEdge {
        source_node_id,
        target_node_id,
        kind,
        label: None,
    }
}
fn query(before: u64, after: u64) -> OrderQuery {
    OrderQuery {
        id: "order".into(),
        region_id: "selected-region".into(),
        before,
        after,
    }
}

#[test]
fn loop_reachability_is_not_must_order() {
    let g = graph();
    for (a, b) in [(2, 3), (3, 2)] {
        let result = observe(Some(&g), &query(a, b)).unwrap();
        assert_eq!(result.may_reach_in_active_cfg, Some(true));
    }
    assert_eq!(
        observe(Some(&g), &query(2, 3))
            .unwrap()
            .strictly_dominates_in_active_cfg,
        Some(false)
    );
    assert_eq!(
        observe(Some(&g), &query(1, 3))
            .unwrap()
            .strictly_dominates_in_active_cfg,
        Some(true)
    );
}

#[test]
fn exception_bypass_prevents_dominance() {
    let mut g = graph();
    g.edges.push(edge(0, 3, E::Exception));
    g.edges.sort();
    g = seal(g).unwrap();
    assert_eq!(
        observe(Some(&g), &query(1, 3))
            .unwrap()
            .strictly_dominates_in_active_cfg,
        Some(false)
    );
}

#[test]
fn dead_or_missing_occurrences_are_unknown_not_negative_execution_proofs() {
    for q in [query(5, 3), query(99, 3)] {
        let result = observe(Some(&graph()), &q).unwrap();
        assert_eq!(result.may_reach_in_active_cfg, None);
        assert!(result.unknown_reason.is_some());
    }
    assert!(
        observe(None, &query(1, 3))
            .unwrap()
            .unknown_reason
            .is_some()
    );
    let same = observe(Some(&graph()), &query(3, 3)).unwrap();
    assert_eq!(same.strictly_dominates_in_active_cfg, Some(false));
}

#[test]
fn local_control_queries_reject_corrupted_graphs() {
    let mut g = graph();
    g.edges[0].target_node_id = 100;
    assert!(observe(Some(&g), &query(1, 3)).is_err());
}

#[test]
fn conditional_acceptance_keeps_post_patch_and_send_graph_order_distinct() {
    // Synthetic source-free control fixture: validate -> POST -> decision,
    // then either PATCH -> send or return. Exceptions can bypass later effects.
    let mut g = graph();
    g.nodes = vec![
        node(0, N::Entry),
        node(1, N::Operation),
        node(2, N::Decision),
        node(3, N::Operation),
        node(4, N::Operation),
        node(5, N::Return),
        node(6, N::Throw),
        node(7, N::Exit),
    ];
    g.edges = vec![
        edge(0, 1, E::Next),
        edge(1, 2, E::Next),
        edge(1, 6, E::Exception),
        edge(2, 3, E::True),
        edge(2, 5, E::False),
        edge(3, 4, E::Next),
        edge(3, 6, E::Exception),
        edge(4, 5, E::Next),
        edge(4, 6, E::Exception),
        edge(5, 7, E::Return),
        edge(6, 7, E::Exception),
    ];
    g.edges.sort();
    let g = seal(g).unwrap();
    assert_eq!(
        observe(Some(&g), &query(1, 4))
            .unwrap()
            .strictly_dominates_in_active_cfg,
        Some(true)
    );
    assert_eq!(
        observe(Some(&g), &query(3, 4))
            .unwrap()
            .strictly_dominates_in_active_cfg,
        Some(true)
    );
    assert_eq!(
        observe(Some(&g), &query(3, 5))
            .unwrap()
            .strictly_dominates_in_active_cfg,
        Some(false)
    );
    assert_eq!(
        observe(Some(&g), &query(4, 6))
            .unwrap()
            .strictly_dominates_in_active_cfg,
        Some(false)
    );
    assert_eq!(
        observe(Some(&g), &query(4, 1))
            .unwrap()
            .may_reach_in_active_cfg,
        Some(false)
    );
}
