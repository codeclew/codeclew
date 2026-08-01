use serde_json::json;
use sthread::canonical;
use sthread::graph;
use sthread::model::{CompletenessStatus, Direction, LocalGraph, SlicePolicy, Snapshot};
use sthread::proto::RequestKind;
use sthread::worker::{WorkerClient, workspace_root};

#[test]
fn k2_fir_golden_language_and_slice_matrix() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    let mut worker = WorkerClient::start(&root).unwrap();

    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["languageVersion"], "2.4");
    assert_eq!(project["jvmTarget"], "21");
    assert_eq!(project["compileTask"], ":compileKotlin");
    assert!(!project["compileClasspath"].as_array().unwrap().is_empty());
    assert!(
        project["modelInputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|input| input["path"] == "gradle/wrapper/gradle-wrapper.jar")
    );
    let test_index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/test"}),
        )
        .unwrap();
    assert_eq!(test_index["compilation"], ":/test");
    assert!(
        test_index["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["path"].as_str().unwrap().contains("/test/"))
    );
    let main_index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    let indexed_file = &main_index["files"][0];
    for field in [
        "fileId",
        "module",
        "sourceSet",
        "normalizedRelativePath",
        "declarationIds",
        "inheritance",
        "overrides",
        "functionSummaries",
        "diagnostics",
    ] {
        assert!(!indexed_file[field].is_null(), "file fact lacks {field}");
    }
    for declaration in indexed_file["declarations"].as_array().unwrap() {
        for field in [
            "declarationId",
            "symbolId",
            "sourceOrigin",
            "sourceSignatureHash",
            "bodyHash",
            "abiHash",
            "semanticSummaryHash",
        ] {
            assert!(!declaration[field].is_null(), "declaration lacks {field}");
        }
    }

    let call = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":"com.acme.namedCall"}),
        )
        .unwrap();
    assert_eq!(call["k2Validated"], true);
    let resolved = call["resolvedCalls"].as_array().unwrap();
    assert!(
        resolved
            .iter()
            .any(|fact| fact["symbol"] == "com/acme/decorate")
    );
    assert!(
        resolved
            .iter()
            .any(|fact| fact["receiverType"] == "kotlin/String")
    );
    assert!(resolved.iter().any(|fact| {
        fact["argumentToParameter"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mapping| mapping["parameter"] == "prefix")
    }));

    let overloaded = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":"com.acme.overloaded(Int)"}),
        )
        .unwrap();
    assert_eq!(
        overloaded["declaration"]["symbolId"],
        "com.acme.overloaded(Int)"
    );

    let total_raw = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.total"}),
        )
        .unwrap();
    assert_eq!(total_raw["graphSource"], "K2_FIR_CFG");
    let local: LocalGraph = serde_json::from_value(total_raw).unwrap();
    let mut permuted = local.clone();
    permuted.nodes.reverse();
    permuted.edges.reverse();
    let total = graph::enrich(local);
    let total_permuted = graph::enrich(permuted);
    assert_eq!(
        canonical::bytes(&total).unwrap(),
        canonical::bytes(&total_permuted).unwrap()
    );
    assert!(total.edges.iter().any(|edge| edge.kind == "CFG_TRUE"));
    assert!(total.edges.iter().any(|edge| edge.kind == "CFG_FALSE"));
    assert!(total.edges.iter().any(|edge| edge.kind == "CONTROL_DEP"));
    assert!(
        total
            .nodes
            .iter()
            .any(|node| node.kind == "PHI" && node.defines.as_deref() == Some("value"))
    );

    let return_id = total
        .nodes
        .iter()
        .find(|node| node.kind == "RETURN" && node.uses.iter().any(|name| name == "value"))
        .unwrap()
        .id
        .clone();
    let snapshot = Snapshot {
        base_revision: "test".into(),
        project_model_hash: project["projectModelHash"].as_str().unwrap().into(),
        compiler_version: "2.4.10".into(),
    };
    let thread = graph::slice(
        &total,
        &return_id,
        SlicePolicy {
            direction: Direction::Both,
            ..Default::default()
        },
        snapshot,
        json!({"kind":"FUNCTION_RETURN","symbol":"com.acme.total","nodeId":return_id}),
    )
    .unwrap();
    for required in ["base", "premium"] {
        assert!(
            thread
                .nodes
                .iter()
                .any(|node| node.kind == "PARAMETER" && node.defines.as_deref() == Some(required)),
            "missing parameter {required}"
        );
    }
    assert!(thread.nodes.iter().any(|node| node.kind == "PHI"));
    assert!(thread.nodes.iter().any(|node| node.kind == "ASSIGNMENT"));
    assert!(thread.nodes.iter().any(|node| node.kind == "RETURN"));
    assert_eq!(
        thread.completeness.status,
        CompletenessStatus::CompleteSupportedSubset
    );
    for kind in [
        "SOURCE_NODE",
        "OWNER_SIGNATURE",
        "RESOLVED_SYMBOL",
        "EXPRESSION_TYPE",
        "PROJECT_MODEL",
        "DIAGNOSTICS",
    ] {
        assert!(
            thread.read_set.iter().any(|fact| fact.kind == kind),
            "missing ReadSet fact {kind}"
        );
    }

    for (symbol, edge) in [
        ("com.acme.loops", "CFG_BACK"),
        ("com.acme.guarded", "CFG_EXCEPTION"),
        ("com.acme.classify", "CFG_TRUE"),
    ] {
        let raw = worker
            .request(
                RequestKind::BuildLocalGraph,
                &json!({"repo":fixture,"symbol":symbol}),
            )
            .unwrap();
        let graph = graph::enrich(serde_json::from_value(raw).unwrap());
        assert!(
            graph.edges.iter().any(|candidate| candidate.kind == edge),
            "{symbol} lacks {edge}"
        );
    }
    let guarded = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.guarded"}),
        )
        .unwrap();
    let guarded = graph::enrich(serde_json::from_value(guarded).unwrap());
    assert!(guarded.nodes.iter().any(|node| node.kind == "THROW"));
    assert!(guarded.nodes.iter().any(|node| {
        node.attributes
            .get("firNodeKind")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| kind.contains("SafeCall"))
    }));
    assert!(guarded.nodes.iter().any(|node| {
        node.attributes
            .get("firNodeKind")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| kind.contains("Elvis"))
    }));

    let calls = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.namedCall"}),
        )
        .unwrap();
    let calls = graph::enrich(serde_json::from_value(calls).unwrap());
    assert!(
        calls
            .nodes
            .iter()
            .any(|node| node.attributes.contains_key("calleeSummaryHash"))
    );
    assert!(
        calls
            .nodes
            .iter()
            .any(|node| node.attributes.contains_key("receiverType"))
    );
    assert_eq!(
        calls
            .nodes
            .iter()
            .filter(|node| node.kind == "CALL")
            .count(),
        1,
        "one Kotlin call-site must normalize to exactly one CALL node"
    );
    for edge in ["CALL", "RETURN", "ARG_PARAM", "RECEIVER"] {
        assert!(
            calls.edges.iter().any(|candidate| candidate.kind == edge),
            "named/default extension call lacks {edge}"
        );
    }
    let call_seed = calls
        .nodes
        .iter()
        .find(|node| node.kind == "RETURN")
        .unwrap()
        .id
        .clone();
    let call_thread = graph::slice(
        &calls,
        &call_seed,
        SlicePolicy::default(),
        Snapshot {
            base_revision: "test".into(),
            project_model_hash: project["projectModelHash"].as_str().unwrap().into(),
            compiler_version: "2.4.10".into(),
        },
        json!({"kind":"FUNCTION_RETURN","symbol":"com.acme.namedCall","nodeId":call_seed}),
    )
    .unwrap();
    assert_eq!(
        call_thread.completeness.status,
        CompletenessStatus::PartialExternalBoundary
    );
    assert!(!call_thread.completeness.boundaries.is_empty());
    assert!(!call_thread.external_summaries.is_empty());
    assert_eq!(call_thread.external_summaries.len(), 1);
    for kind in ["COMPILER_OPTIONS", "CLASSPATH", "INHERITANCE"] {
        assert!(
            call_thread.read_set.iter().any(|fact| fact.kind == kind),
            "call Thread ReadSet lacks {kind}"
        );
    }

    let capture = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.capture"}),
        )
        .unwrap();
    let capture = graph::enrich(serde_json::from_value(capture).unwrap());
    assert!(capture.edges.iter().any(|edge| edge.kind == "CAPTURE"));
    let counter = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.Counter.increment"}),
        )
        .unwrap();
    let counter = graph::enrich(serde_json::from_value(counter).unwrap());
    for effect in ["READ_STATE", "WRITE_STATE"] {
        assert!(
            counter.edges.iter().any(|edge| edge.kind == effect),
            "counter lacks {effect}"
        );
    }
    assert!(counter.nodes.iter().any(|node| {
        node.attributes
            .get("memoryKind")
            .and_then(|value| value.as_str())
            == Some("THIS_PROPERTY")
    }));
    let suspend = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.boundary"}),
        )
        .unwrap();
    let suspend = graph::enrich(serde_json::from_value(suspend).unwrap());
    assert!(suspend.edges.iter().any(|edge| edge.kind == "SUSPEND"));

    let source =
        std::fs::read_to_string(fixture.join("src/main/kotlin/com/acme/Samples.kt")).unwrap();
    let offset = source.find("value *= 2").unwrap() + 2;
    let expression = worker
        .request(
            RequestKind::ResolveExpression,
            &json!({"repo":fixture,"file":"src/main/kotlin/com/acme/Samples.kt","offset":offset}),
        )
        .unwrap();
    assert_eq!(expression["anchor"]["sourceText"], "value *= 2");
    worker.shutdown().unwrap();
}
