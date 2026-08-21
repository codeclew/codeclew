mod support;

use clew::graph;
use clew::model::LocalGraph;
use clew::proto::RequestKind;
use clew::worker::{WorkerClient, workspace_root};
use serde_json::json;

#[test]
fn worker_vertical_resolves_and_builds_total_graph() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();
    let batch = worker
        .validate_candidates_batch(&[
            ("One.kt".into(), "fun one() = 1".into()),
            ("Two.kt".into(), "fun two() = 2".into()),
        ])
        .unwrap();
    assert_eq!(batch.len(), 2);
    assert!(batch.iter().all(|candidate| candidate["valid"] == true));
    let large_source = format!("fun large() = 1\n/*{}*/\n", "x".repeat(70 * 1024));
    let large = worker
        .request(
            RequestKind::ValidateCandidate,
            &json!({"repo":fixture,"file":"Large.kt","source":large_source}),
        )
        .unwrap();
    assert_eq!(
        large["valid"], true,
        "large source BlobRef round-trip failed"
    );
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
        symbol["declaration"]["legacySymbolId"],
        "com.acme.total(Int,Boolean)"
    );
    assert_eq!(symbol["declaration"]["symbolIdentity"]["module"], ":");
    assert_eq!(symbol["declaration"]["symbolIdentity"]["sourceSet"], "main");
    let anchor = &symbol["bodyAnchor"];
    let fixture_source =
        std::fs::read_to_string(fixture.join("src/main/kotlin/com/acme/Samples.kt")).unwrap();
    let oversized_file = format!("{fixture_source}\n/*{}*/\n", "x".repeat(70 * 1024));
    let oversized_candidate = worker
        .request(
            RequestKind::ApplyEdit,
            &json!({
                "repo":fixture,"file":anchor["fileId"],"source":oversized_file,
                "ownerSymbolId":anchor["ownerSymbolId"],"exactTextHash":anchor["exactTextHash"],
                "syntaxKind":anchor["syntaxKind"],"normalizedTokenHash":anchor["normalizedTokenHash"],
                "ancestorPathHash":anchor["ancestorPathHash"],"localOrdinal":anchor["localOrdinal"],
                "leftContextHash":anchor["leftContextHash"],"rightContextHash":anchor["rightContextHash"],
                "kind":"REPLACE_FUNCTION_BODY","replacement":anchor["sourceText"],
                "preconditions":{},"postconditions":{}
            }),
        )
        .unwrap();
    assert!(oversized_candidate["sourceBlob"].is_object());
    assert!(oversized_candidate["source"].as_str().unwrap().len() > 64 * 1024);
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
