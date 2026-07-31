use serde_json::json;
use sthread::graph;
use sthread::model::LocalGraph;
use sthread::proto::RequestKind;
use sthread::worker::{WorkerClient, workspace_root};

#[test]
fn worker_vertical_resolves_and_builds_total_graph() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":fixture}))
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.4.10");
    let symbol = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":"com.acme.total"}),
        )
        .unwrap();
    assert_eq!(
        symbol["declaration"]["symbolId"],
        "com.acme.total(Int,Boolean)"
    );
    let raw = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.total"}),
        )
        .unwrap();
    let enriched = graph::enrich(serde_json::from_value::<LocalGraph>(raw).unwrap());
    assert!(
        enriched
            .nodes
            .iter()
            .any(|n| n.kind == "PHI" && n.defines.as_deref() == Some("value"))
    );
    assert!(enriched.edges.iter().any(|e| e.kind == "CONTROL_DEP"));
    assert!(enriched.edges.iter().any(|e| e.kind == "DEF_USE"));
    worker.shutdown().unwrap();
}
