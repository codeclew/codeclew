use clew::thread_flow_cfg::{
    LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNode, LocalCfgNodeRole, LocalCfgPayload,
    LocalCfgSourceRange, seal, validate,
};

#[test]
fn branch_loop_return_and_throw_edges_are_retained_without_path_expansion() {
    let graph = seal(LocalCfgPayload {
        schema: "local-cfg/0.1".into(),
        graph_id: String::new(),
        owner_symbol_identity: "callable:com/acme/loops#jvm:loops(I)I".into(),
        file: "src/main/kotlin/com/acme/Samples.kt".into(),
        compiler_graph_name: "com/acme/loops".into(),
        provider: "K2_FIR_CFG".into(),
        source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
        nodes: vec![
            node(0, LocalCfgNodeRole::Entry, 270, 270),
            node(1, LocalCfgNodeRole::LoopCondition, 330, 349),
            node(2, LocalCfgNodeRole::Decision, 380, 393),
            node(3, LocalCfgNodeRole::Operation, 420, 435),
            node(4, LocalCfgNodeRole::Throw, 440, 455),
            node(5, LocalCfgNodeRole::Return, 560, 575),
            node(6, LocalCfgNodeRole::Exit, 580, 580),
        ],
        edges: vec![
            edge(0, 1, LocalCfgEdgeKind::Next, None),
            edge(1, 2, LocalCfgEdgeKind::True, Some("loop-body")),
            edge(1, 5, LocalCfgEdgeKind::False, Some("loop-exit")),
            edge(2, 3, LocalCfgEdgeKind::True, Some("continue-path")),
            edge(2, 4, LocalCfgEdgeKind::False, Some("exception-path")),
            edge(3, 1, LocalCfgEdgeKind::LoopBack, None),
            edge(4, 6, LocalCfgEdgeKind::Exception, None),
            edge(5, 6, LocalCfgEdgeKind::Return, None),
        ],
    })
    .unwrap();

    validate(&graph).unwrap();
    assert_eq!(graph.nodes.len(), 7);
    assert_eq!(graph.edges.len(), 8);
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.kind == LocalCfgEdgeKind::LoopBack)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.kind == LocalCfgEdgeKind::Exception)
    );
}

#[test]
fn dangling_unsorted_and_unreachable_non_dead_graphs_fail_closed() {
    let valid = simple_graph();

    let mut dangling = valid.clone();
    dangling.edges[0].target_node_id = 99;
    assert!(seal(dangling).is_err());

    let mut unsorted = valid.clone();
    unsorted.nodes.swap(0, 1);
    assert!(seal(unsorted).is_err());

    let mut unreachable = valid;
    unreachable
        .nodes
        .insert(1, node(1, LocalCfgNodeRole::Operation, 1, 2));
    assert!(seal(unreachable).is_err());
}

#[test]
fn only_explicit_dead_nodes_may_be_unreachable() {
    let mut graph = simple_graph();
    graph.nodes.insert(1, node(1, LocalCfgNodeRole::Dead, 1, 2));
    let graph = seal(graph).unwrap();
    assert_eq!(graph.nodes[1].role, LocalCfgNodeRole::Dead);
}

fn simple_graph() -> LocalCfgPayload {
    LocalCfgPayload {
        schema: "local-cfg/0.1".into(),
        graph_id: String::new(),
        owner_symbol_identity: "callable:com/acme/total#jvm:total(IZ)I".into(),
        file: "src/main/kotlin/com/acme/Samples.kt".into(),
        compiler_graph_name: "com/acme/total".into(),
        provider: "K2_FIR_CFG".into(),
        source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
        nodes: vec![
            node(0, LocalCfgNodeRole::Entry, 0, 0),
            node(2, LocalCfgNodeRole::Return, 100, 110),
        ],
        edges: vec![edge(0, 2, LocalCfgEdgeKind::Return, None)],
    }
}

fn node(id: u64, role: LocalCfgNodeRole, start: u64, end: u64) -> LocalCfgNode {
    LocalCfgNode {
        node_id: id,
        role,
        source: (end > start).then_some(LocalCfgSourceRange { start, end }),
    }
}

fn edge(source: u64, target: u64, kind: LocalCfgEdgeKind, label: Option<&str>) -> LocalCfgEdge {
    LocalCfgEdge {
        source_node_id: source,
        target_node_id: target,
        kind,
        label: label.map(str::to_owned),
    }
}
