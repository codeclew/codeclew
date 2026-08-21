mod support;

use clew::canonical;
use clew::error::ErrorCode;
use clew::graph;
use clew::model::{CompletenessStatus, Direction, LocalGraph, SlicePolicy, Snapshot};
use clew::proto::RequestKind;
use clew::worker::{WorkerClient, workspace_root};
use serde_json::json;

#[test]
fn k2_fir_golden_language_and_slice_matrix() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    support::seed_build_caches(&fixture);
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
    assert!(
        test_index["files"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|file| file["declarations"].as_array().into_iter().flatten())
            .all(|declaration| declaration["symbolIdentity"]["sourceSet"] == "test")
    );
    let main_index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    let indexed_files = main_index["files"].as_array().unwrap();
    assert!(
        indexed_files
            .iter()
            .flat_map(|file| file["declarations"].as_array().into_iter().flatten())
            .all(|declaration| declaration["symbolIdentity"]["sourceSet"] == "main")
    );
    for indexed_file in indexed_files {
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
    }
    let declarations = indexed_files
        .iter()
        .flat_map(|file| file["declarations"].as_array().into_iter().flatten())
        .collect::<Vec<_>>();
    for declaration in &declarations {
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
    let inferred_function = declarations
        .iter()
        .find(|declaration| declaration["name"] == "inferredAnswer")
        .unwrap();
    assert_eq!(
        inferred_function["symbolIdentity"]["returnType"],
        "kotlin/Int"
    );
    assert_eq!(inferred_function["symbolIdentity"]["jvmDescriptor"], "()I");
    let inferred_property = declarations
        .iter()
        .find(|declaration| declaration["name"] == "inferredBanner")
        .unwrap();
    assert_eq!(
        inferred_property["symbolIdentity"]["returnType"],
        "kotlin/String"
    );
    assert_eq!(
        inferred_property["symbolIdentity"]["jvmDescriptor"],
        "Ljava/lang/String;"
    );

    let call = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":"com.acme.namedCall"}),
        )
        .unwrap();
    assert_eq!(call["k2Validated"], true);
    assert!(
        call["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "decorate")
    );
    let resolved = call["resolvedCalls"].as_array().unwrap();
    if !resolved.is_empty() {
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
    }

    let overloaded = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":"com.acme.overloaded(Int)"}),
        )
        .unwrap();
    let overloaded_identity = &overloaded["declaration"]["symbolIdentity"];
    assert_eq!(overloaded_identity["module"], ":");
    assert_eq!(overloaded_identity["sourceSet"], "main");
    assert_eq!(overloaded_identity["declarationKind"], "FUNCTION");
    assert_eq!(overloaded_identity["parameterTypes"], json!(["kotlin/Int"]));
    assert_eq!(overloaded_identity["returnType"], "kotlin/Int");
    assert_eq!(overloaded_identity["typeParameterArity"], 0);
    assert_eq!(overloaded_identity["suspendFlag"], false);
    assert_eq!(overloaded_identity["jvmDescriptor"], "(I)I");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            overloaded["declaration"]["symbolId"].as_str().unwrap()
        )
        .unwrap(),
        *overloaded_identity
    );
    let full_symbol = overloaded["declaration"]["symbolId"].as_str().unwrap();
    let round_trip = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":full_symbol}),
        )
        .unwrap();
    assert_eq!(round_trip["symbol"], full_symbol);
    let mut tampered_identity = overloaded_identity.clone();
    tampered_identity["returnType"] = json!("kotlin/String");
    tampered_identity["jvmDescriptor"] = json!("(I)Ljava/lang/String;");
    let tampered = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":tampered_identity.to_string()}),
        )
        .unwrap_err();
    assert_eq!(tampered.code, ErrorCode::SymbolNotFound);
    for (symbol, descriptor) in [
        ("com.acme.capture", "(Ljava/util/List;)I"),
        (
            "com.acme.boxedArray",
            "([Ljava/lang/Integer;)[Ljava/lang/Integer;",
        ),
        (
            "com.acme.genericNumber",
            "(Ljava/lang/Number;)Ljava/lang/Number;",
        ),
        (
            "com.acme.genericArray",
            "([Ljava/lang/Number;)[Ljava/lang/Number;",
        ),
        (
            "String.com.acme.decorate",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
        ),
    ] {
        let resolved = worker
            .request(
                RequestKind::ResolveSymbol,
                &json!({"repo":fixture,"symbol":symbol}),
            )
            .unwrap();
        assert_eq!(
            resolved["declaration"]["symbolIdentity"]["jvmDescriptor"], descriptor,
            "wrong JVM descriptor for {symbol}"
        );
    }

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
    for edge in ["AST_CHILD", "TYPE"] {
        assert!(
            total.edges.iter().any(|candidate| candidate.kind == edge),
            "minimal semantic graph lacks {edge}"
        );
    }
    assert!(total.nodes.iter().any(|node| {
        node.attributes
            .get("memoryKind")
            .and_then(|value| value.as_str())
            == Some("LOCAL")
    }));
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
        ..Snapshot::default()
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
        CompletenessStatus::PartialExternalBoundary
    );
    assert!(
        thread
            .completeness
            .boundaries
            .iter()
            .any(|boundary| { boundary["reason"] == "unresolvedCallTarget" })
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
    let has_exact_callee = calls
        .nodes
        .iter()
        .any(|node| node.attributes.contains_key("calleeSummaryHash"));
    assert_eq!(
        calls
            .nodes
            .iter()
            .filter(|node| node.kind == "CALL")
            .count(),
        1,
        "one Kotlin call-site must normalize to exactly one CALL node"
    );
    if has_exact_callee {
        assert!(
            calls
                .nodes
                .iter()
                .any(|node| node.attributes.contains_key("receiverType"))
        );
        for edge in ["CALL", "RETURN", "ARG_PARAM", "RECEIVER"] {
            assert!(
                calls.edges.iter().any(|candidate| candidate.kind == edge),
                "named/default extension call lacks {edge}"
            );
        }
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
            ..Snapshot::default()
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
    assert_eq!(
        call_thread
            .nodes
            .iter()
            .filter(|node| node.kind == "CALL")
            .count(),
        1,
        "call slice must retain its unique CALL-enter node"
    );
    if has_exact_callee {
        for edge in ["CALL", "RETURN", "ARG_PARAM", "RECEIVER"] {
            assert!(
                call_thread
                    .edges
                    .iter()
                    .any(|candidate| candidate.kind == edge),
                "call slice lacks {edge}"
            );
        }
    } else {
        assert!(call_thread.completeness.boundaries.iter().any(|boundary| {
            boundary.get("reason").and_then(|value| value.as_str()) == Some("unresolvedCallTarget")
        }));
    }
    for summary in &call_thread.external_summaries {
        let node_id = summary["nodeId"].as_str().unwrap();
        assert!(
            call_thread.nodes.iter().any(|node| node.id == node_id),
            "external summary refers to a node outside the slice"
        );
    }
    assert!(
        call_thread
            .read_set
            .iter()
            .any(|fact| fact.kind == "CALL_TARGET"),
        "call Thread ReadSet lacks CALL_TARGET"
    );
    for kind in ["COMPILER_OPTIONS", "CLASSPATH", "INHERITANCE"] {
        assert!(
            call_thread.read_set.iter().any(|fact| fact.kind == kind),
            "call Thread ReadSet lacks {kind}"
        );
    }

    let flow_fixture = root.join("fixtures/kotlin-control-flow");
    support::seed_build_caches(&flow_fixture);
    let short_circuit: LocalGraph = serde_json::from_value(
        worker
            .request(
                RequestKind::BuildLocalGraph,
                &json!({"repo":flow_fixture,"symbol":"flow.shortCircuit"}),
            )
            .unwrap(),
    )
    .unwrap();
    for edge in ["CFG_TRUE", "CFG_FALSE"] {
        assert!(
            short_circuit
                .edges
                .iter()
                .any(|candidate| candidate.kind == edge),
            "short-circuit FIR CFG lacks {edge}"
        );
    }
    let calls_fixture = root.join("fixtures/kotlin-calls");
    support::seed_build_caches(&calls_fixture);
    let java_boundary = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":calls_fixture,"symbol":"calls.javaBoundary"}),
        )
        .unwrap();
    let java_boundary = graph::enrich(serde_json::from_value::<LocalGraph>(java_boundary).unwrap());
    let java_calls = java_boundary
        .nodes
        .iter()
        .filter(|node| node.kind == "CALL")
        .collect::<Vec<_>>();
    assert!(!java_calls.is_empty(), "Java boundary call is missing");
    if let Some(java_call) = java_calls.iter().find(|node| {
        node.attributes
            .get("symbol")
            .and_then(|value| value.as_str())
            .is_some_and(|symbol| symbol.starts_with("java/"))
    }) {
        assert!(
            java_boundary
                .edges
                .iter()
                .any(|edge| edge.from == java_call.id && edge.kind == "CALL")
        );
        assert!(
            java_boundary
                .edges
                .iter()
                .any(|edge| { edge.from == java_call.id && edge.kind == "READ_STATE" })
        );
    } else {
        let return_seed = java_boundary
            .nodes
            .iter()
            .find(|node| node.kind == "RETURN")
            .expect("Java boundary graph lacks RETURN")
            .id
            .clone();
        let partial = graph::slice(
            &java_boundary,
            &return_seed,
            SlicePolicy::default(),
            Snapshot::default(),
            json!({"kind":"FUNCTION_RETURN","nodeId":return_seed}),
        )
        .unwrap();
        assert_eq!(
            partial.completeness.status,
            CompletenessStatus::PartialExternalBoundary
        );
        assert!(partial.completeness.boundaries.iter().any(|boundary| {
            boundary.get("reason").and_then(|value| value.as_str()) == Some("unresolvedCallTarget")
        }));
    }
    let guarded: LocalGraph = serde_json::from_value(
        worker
            .request(
                RequestKind::BuildLocalGraph,
                &json!({"repo":fixture,"symbol":"com.acme.guarded"}),
            )
            .unwrap(),
    )
    .unwrap();
    for edge in ["CFG_TRUE", "CFG_FALSE"] {
        assert!(
            guarded.edges.iter().any(|candidate| candidate.kind == edge),
            "safe-call/Elvis FIR CFG lacks {edge}"
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
    for (symbol, memory_kind) in [
        ("com.acme.objectProperty", "OBJECT_PROPERTY"),
        ("com.acme.staticProperty", "STATIC_PROPERTY"),
    ] {
        let memory_graph = worker
            .request(
                RequestKind::BuildLocalGraph,
                &json!({"repo":fixture,"symbol":symbol}),
            )
            .unwrap();
        let memory_graph = graph::enrich(serde_json::from_value(memory_graph).unwrap());
        assert!(
            memory_graph.nodes.iter().any(|node| {
                node.attributes
                    .get("memoryKind")
                    .and_then(|value| value.as_str())
                    == Some(memory_kind)
            }),
            "{symbol} lacks {memory_kind}"
        );
        assert!(
            memory_graph
                .edges
                .iter()
                .any(|edge| edge.kind == "READ_STATE"),
            "{symbol} lacks conservative state dependency"
        );
    }
    let external = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.externalProperty"}),
        )
        .unwrap();
    let external = graph::enrich(serde_json::from_value(external).unwrap());
    assert!(external.nodes.iter().any(|node| {
        node.attributes
            .get("memoryKind")
            .and_then(|value| value.as_str())
            == Some("UNKNOWN_HEAP")
    }));
    for effect in ["READ_STATE", "WRITE_STATE"] {
        assert!(
            external.edges.iter().any(|edge| edge.kind == effect),
            "external instance property lacks {effect}"
        );
    }
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
