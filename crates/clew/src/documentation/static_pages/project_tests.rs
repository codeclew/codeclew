use super::*;
use crate::documentation::{
    check,
    model::{Observation, ServiceEvidence, Source, SourceOccurrence},
};
use serde_json::json;
use std::collections::BTreeMap;

const JAVA_SCHEMA: &str = "codeclew-java-compiler-fact/1.0";

fn evidence() -> ServiceEvidence {
    ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: "sample".into(),
        revision: "a".repeat(40),
        service_digest: "service-digest".into(),
        extractor: "synthetic-compiler-fixture".into(),
        runtime_mode: "STATIC".into(),
        coverage: "SEMANTIC".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    }
}

fn source(e: &ServiceEvidence, id: &str, text: &str) -> Source {
    Source {
        id: id.into(),
        service: e.service.clone(),
        revision: e.revision.clone(),
        file: format!("{id}.java"),
        start_line: 11,
        end_line: 10 + text.lines().count() as u64,
        text: text.into(),
        text_digest: crate::canonical::hash_bytes(text.as_bytes()),
        evidence_digest: "compiler-receipt".into(),
        authority: "EXACT_SNAPSHOT_TEXT".into(),
        occurrence: Some(SourceOccurrence {
            snapshot: "snapshot".into(),
            blob: crate::canonical::hash_bytes(text.as_bytes()),
            start_byte: 0,
            end_byte: text.len(),
        }),
        url: Some(format!(
            "https://example.invalid/sample/blob/{}/{id}.java#L11",
            e.revision
        )),
    }
}

fn add_declaration(
    e: &mut ServiceEvidence,
    id: &str,
    owner: &str,
    name: &str,
    kind: &str,
    text: &str,
) -> String {
    let symbol = format!(
        "{}:class:{owner}#{name}()V",
        if kind == "FIELD" { "field" } else { "method" }
    );
    let mut normalized = json!({"schema":JAVA_SCHEMA,"kind":"DECLARATION","declarationKind":kind,"symbolIdentity":symbol,"ownerIdentity":format!("class:{owner}"),"name":name,"scope":"compile-scope","resolution":"COMPILER_EXACT","jvmDescriptor":if kind == "FIELD" { "Ljava/util/concurrent/BlockingQueue;" } else { "()V" }});
    if kind == "CONSTRUCTOR" {
        normalized["name"] = json!("<init>");
    }
    let s = source(e, &format!("src-{id}"), text);
    let o = Observation {
        id: id.into(),
        kind: "SYMBOL".into(),
        service: e.service.clone(),
        symbol: symbol.clone(),
        digest: digest(&normalized).unwrap(),
        normalized,
        source_ids: vec![s.id.clone()],
    };
    e.sources.insert(s.id.clone(), s);
    e.observations.insert(id.into(), o);
    symbol
}

fn add_kotlin_declaration(e: &mut ServiceEvidence, id: &str, kind: &str, text: &str) -> String {
    let callable_id = format!("example/Sample.{id}");
    let symbol = format!("callable:{callable_id}#jvm:(Ljava/lang/String;)Ljava/lang/String;");
    let normalized = json!({
        "schema":"declaration-descriptor/0.1",
        "resolution":"PROVEN",
        "provider":"K2_FIR",
        "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        "compilerAuthority":"fir-facts-extractor/0.6",
        "declarationKind":kind,
        "compilerCallableId":callable_id,
        "jvmDescriptor":"(Ljava/lang/String;)Ljava/lang/String;",
        "symbolIdentity":symbol,
        "ownerIdentity":"class:example/Sample",
        "module":":",
        "sourceSet":"main",
        "scope":":/main"
    });
    let mut retained = source(e, &format!("kotlin-{id}"), text);
    retained.file = "src/main/kotlin/example/Sample.kt".into();
    retained.start_line = 23;
    retained.end_line = retained.start_line + text.lines().count() as u64 - 1;
    retained.url = Some(format!(
        "https://example.invalid/sample/blob/{}/{}#L23",
        e.revision, retained.file
    ));
    let observation = Observation {
        id: id.into(),
        kind: "SYMBOL".into(),
        service: e.service.clone(),
        symbol: symbol.clone(),
        digest: digest(&normalized).unwrap(),
        normalized,
        source_ids: vec![retained.id.clone()],
    };
    e.sources.insert(retained.id.clone(), retained);
    e.observations.insert(id.into(), observation);
    symbol
}

fn configure_kotlin_function(
    e: &mut ServiceEvidence,
    id: &str,
    callable_id: &str,
    descriptor: &str,
    scope: &str,
) -> String {
    let symbol = format!("callable:{callable_id}#jvm:{descriptor}");
    let observation = e.observations.get_mut(id).unwrap();
    observation.symbol = symbol.clone();
    observation.normalized["symbolIdentity"] = json!(symbol);
    observation.normalized["compilerCallableId"] = json!(callable_id);
    observation.normalized["jvmDescriptor"] = json!(descriptor);
    observation.normalized["ownerIdentity"] = json!(
        callable_id
            .split_once('.')
            .map(|(owner, _)| format!("class:{owner}"))
            .unwrap_or_else(|| "class:parity/Api".into())
    );
    observation.normalized["scope"] = json!(scope);
    observation.digest = digest(&observation.normalized).unwrap();
    symbol
}

fn add_kotlin_target(
    e: &mut ServiceEvidence,
    id: &str,
    callable_id: &str,
    descriptor: &str,
    scope: &str,
    source_text: &str,
    documentation: Option<serde_json::Value>,
) -> String {
    add_kotlin_declaration(e, id, "FUNCTION", source_text);
    let symbol = configure_kotlin_function(e, id, callable_id, descriptor, scope);
    let source_id = e.observations[id].source_ids[0].clone();
    let source = e.sources.get_mut(&source_id).unwrap();
    source.evidence_digest = format!("sha256:{}", "a".repeat(64));
    source.occurrence = None;
    if let Some(documentation) = documentation {
        let owner = e.observations.get_mut(id).unwrap();
        owner.normalized["documentation"] = documentation;
        owner.digest = digest(&owner.normalized).unwrap();
    }
    symbol
}

fn add_local_cfg(
    evidence: &mut ServiceEvidence,
    declaration_id: &str,
    graph_name: &str,
    node_roles: &[crate::thread_flow_cfg::LocalCfgNodeRole],
    source_ranges: &[Option<(usize, usize)>],
    edges: Vec<crate::thread_flow_cfg::LocalCfgEdge>,
) -> String {
    use crate::documentation::local_cfg::{
        LOCAL_CFG_EVIDENCE_SCHEMA, LocalCfgEvidence, LocalCfgNodeCitation, LocalCfgSourceSite,
    };
    use crate::thread_flow_cfg::{
        LOCAL_CFG_SCHEMA, LocalCfgNode, LocalCfgPayload, LocalCfgSourceRange,
    };

    let declaration = &evidence.observations[declaration_id];
    let owner = declaration.symbol.clone();
    let owner_source = evidence.sources[&declaration.source_ids[0]].clone();
    assert_eq!(node_roles.len(), source_ranges.len());
    let mut graph = LocalCfgPayload {
        schema: LOCAL_CFG_SCHEMA.into(),
        graph_id: String::new(),
        owner_symbol_identity: owner.clone(),
        file: owner_source.file.clone(),
        compiler_graph_name: graph_name.into(),
        provider: "K2_FIR_CFG".into(),
        source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
        nodes: node_roles
            .iter()
            .zip(source_ranges)
            .enumerate()
            .map(|(node_id, (role, range))| LocalCfgNode {
                node_id: node_id as u64,
                role: *role,
                source: range.map(|(start, end)| LocalCfgSourceRange {
                    start: start as u64,
                    end: end as u64,
                }),
            })
            .collect(),
        edges,
    };
    graph.graph_id = crate::canonical::hash(&graph).unwrap();

    let graph_binding = format!("sha256:{}", "b".repeat(64));
    let descriptor_binding = format!("sha256:{}", "c".repeat(64));
    let source_id = format!("cfg-source-{declaration_id}");
    let text = owner_source.text.clone();
    let text_digest = crate::canonical::hash_bytes(text.as_bytes());
    let start_line = owner_source.start_line;
    let end_line = start_line + text.lines().count() as u64 - 1;
    evidence.sources.insert(
        source_id.clone(),
        Source {
            id: source_id.clone(),
            service: evidence.service.clone(),
            revision: evidence.revision.clone(),
            file: graph.file.clone(),
            start_line,
            end_line,
            text: text.clone(),
            text_digest: text_digest.clone(),
            evidence_digest: graph_binding.clone(),
            authority: owner_source.authority.clone(),
            occurrence: None,
            url: owner_source.url.clone(),
        },
    );
    let node_citations = graph
        .nodes
        .iter()
        .filter_map(|node| {
            let range = node.source.as_ref()?;
            let start = range.start as usize;
            let end = range.end as usize;
            Some(LocalCfgNodeCitation {
                node_id: node.node_id,
                byte_start: range.start,
                byte_end: range.end,
                text_digest: crate::canonical::hash_bytes(text.get(start..end).unwrap().as_bytes()),
            })
        })
        .collect();
    let scope = declaration.normalized["scope"].as_str().unwrap().to_owned();
    let normalized = LocalCfgEvidence {
        schema: LOCAL_CFG_EVIDENCE_SCHEMA.into(),
        scope,
        owner_symbol_identity: owner.clone(),
        file: graph.file.clone(),
        graph,
        graph_evidence_binding: graph_binding.clone(),
        descriptor_evidence_binding: descriptor_binding,
        node_citations,
        source_site: LocalCfgSourceSite {
            source_id: source_id.clone(),
            source_digest: text_digest.clone(),
            source_evidence_digest: graph_binding,
            source_status: "SOURCE_RETAINED".into(),
            authority: evidence.sources[&source_id].authority.clone(),
            file: evidence.sources[&source_id].file.clone(),
            start_line,
            end_line,
            owner_byte_start: 0,
            owner_byte_end: text.len() as u64,
            source_content_digest: text_digest.clone(),
            full_compilation_source_digest: text_digest,
            span_digest: crate::canonical::hash_bytes(text.as_bytes()),
        },
    };
    normalized
        .validate_source(
            &evidence.sources[&source_id],
            &evidence.service,
            &evidence.revision,
        )
        .unwrap();
    let normalized = serde_json::to_value(normalized).unwrap();
    let id = format!("cfg-{declaration_id}");
    evidence.observations.insert(
        id.clone(),
        Observation {
            id: id.clone(),
            kind: "LOCAL_CFG".into(),
            service: evidence.service.clone(),
            symbol: owner,
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![source_id],
        },
    );
    id
}

fn add_local_cfg_boundary(
    evidence: &mut ServiceEvidence,
    owner: Option<&str>,
    file: Option<&str>,
    scope: &str,
    code: &str,
) -> String {
    use crate::documentation::local_cfg::{
        LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA, LocalCfgBoundaryEvidence,
    };

    let binding = format!("sha256:{}", "d".repeat(64));
    let boundary = LocalCfgBoundaryEvidence {
        schema: LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA.into(),
        kind: "LOCAL_CFG_BOUNDARY".into(),
        scope: scope.into(),
        owner_symbol_identity: owner.map(str::to_owned),
        file: file.map(str::to_owned),
        compiler_graph_name: Some("unavailable".into()),
        code: code.into(),
        provider: "CODECLEW_LOCAL_CFG_NORMALIZER".into(),
        evidence_binding: binding.clone(),
        descriptor_evidence_binding: Some(format!("sha256:{}", "e".repeat(64))),
        raw_row_hash: None,
    };
    boundary.validate(&binding).unwrap();
    let normalized = serde_json::to_value(boundary).unwrap();
    let id = format!("cfg-boundary-{code}");
    evidence.observations.insert(
        id.clone(),
        Observation {
            id: id.clone(),
            kind: "LOCAL_CFG_BOUNDARY".into(),
            service: evidence.service.clone(),
            symbol: owner.unwrap_or_default().into(),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![],
        },
    );
    id
}

fn kotlin_check(e: ServiceEvidence) -> Check {
    check::assemble(
        "input-digest".into(),
        BTreeMap::from([(e.service.clone(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap()
}

fn kotlin_control_flow_check() -> (Check, String, String, String) {
    use crate::thread_flow_cfg::{LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNodeRole};

    let mut evidence = evidence();
    let retained =
        "fun calculate(value: String): String {\r\n  // π\r\n  return value.trim()\r\n}".to_owned();
    let symbol = add_kotlin_declaration(&mut evidence, "calculate", "FUNCTION", &retained);
    let comment_start = retained.find("// π").unwrap();
    let return_start = retained.find("return value.trim()").unwrap();
    let cfg_id = add_local_cfg(
        &mut evidence,
        "calculate",
        "graph <{π}>",
        &[
            LocalCfgNodeRole::Entry,
            LocalCfgNodeRole::Operation,
            LocalCfgNodeRole::Return,
        ],
        &[
            None,
            Some((comment_start, comment_start + "// π".len())),
            Some((return_start, return_start + "return value.trim()".len())),
        ],
        vec![
            LocalCfgEdge {
                source_node_id: 0,
                target_node_id: 1,
                kind: LocalCfgEdgeKind::Next,
                label: Some("<entry>{π}".into()),
            },
            LocalCfgEdge {
                source_node_id: 1,
                target_node_id: 2,
                kind: LocalCfgEdgeKind::Return,
                label: Some("CompilerReturn".into()),
            },
        ],
    );
    (kotlin_check(evidence), symbol, cfg_id, retained)
}

fn refresh_observation_digest(e: &mut ServiceEvidence, id: &str) {
    let observation = e.observations.get_mut(id).unwrap();
    observation.digest = digest(&observation.normalized).unwrap();
}

fn add_kotlin_return_outline(e: &mut ServiceEvidence, declaration_id: &str) -> String {
    let binding = format!("sha256:{}", "b".repeat(64));
    let owner = e.observations.get_mut(declaration_id).unwrap();
    owner.symbol = "callable:parity/Answer.next#jvm:(I)I".into();
    owner.normalized["symbolIdentity"] = json!(owner.symbol);
    owner.normalized["compilerCallableId"] = json!("parity/Answer.next");
    owner.normalized["jvmDescriptor"] = json!("(I)I");
    owner.normalized["ownerIdentity"] = json!("class:parity/Answer");
    let owner_source_id = owner.source_ids[0].clone();
    let owner_source = e.sources.get_mut(&owner_source_id).unwrap();
    owner_source.file = "src/main/kotlin/parity/Answer.kt".into();
    owner_source.url = Some(format!(
        "https://example.invalid/sample/blob/{}/{}#L23",
        e.revision, owner_source.file
    ));
    owner_source.evidence_digest = binding.clone();
    owner_source.occurrence = None;
    set_kotlin_outline_events(e, declaration_id, vec![json!({"kind":"RETURN"})]).remove(0)
}

fn set_kotlin_outline_events(
    e: &mut ServiceEvidence,
    declaration_id: &str,
    events: Vec<serde_json::Value>,
) -> Vec<String> {
    let owner = e.observations.get_mut(declaration_id).unwrap();
    let symbol = owner.symbol.clone();
    let owner_source_id = owner.source_ids[0].clone();
    owner.normalized["documentation"] = json!({
        "schema":"codeclew-kotlin-documentation-flow/1.0",
        "authority":"KOTLIN_PSI_WITH_K2_CALL_TARGETS",
        "boundaries":[],
        "events":events
    });
    owner.digest = digest(&owner.normalized).unwrap();
    let owner_source = e.sources[&owner_source_id].clone();
    let event_text = owner_source.text.lines().nth(2).unwrap().to_owned();
    e.observations.retain(|_, observation| {
        observation.kind != "FLOW"
            || observation.service != e.service
            || observation.symbol != symbol
    });
    e.sources
        .retain(|id, _| !id.starts_with("flow-source-answer-next-"));

    events
        .into_iter()
        .enumerate()
        .map(|(ordinal, event)| {
            let flow_id = if ordinal == 0 {
                "flow-answer-next-return".to_owned()
            } else {
                format!("flow-answer-next-event-{ordinal}")
            };
            let source_id = if ordinal == 0 {
                "flow-source-answer-next-return".to_owned()
            } else {
                format!("flow-source-answer-next-event-{ordinal}")
            };
            let mut source = owner_source.clone();
            source.id = source_id.clone();
            source.start_line = owner_source.start_line + 2;
            source.end_line = source.start_line;
            source.text = event_text.clone();
            source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
            source.occurrence = None;
            e.sources.insert(source_id.clone(), source);

            let mut normalized = event;
            let object = normalized.as_object_mut().unwrap();
            object.insert("ordinal".into(), json!(ordinal));
            object.insert("scope".into(), json!(":/main"));
            e.observations.insert(
                flow_id.clone(),
                Observation {
                    id: flow_id.clone(),
                    kind: "FLOW".into(),
                    service: e.service.clone(),
                    symbol: symbol.clone(),
                    digest: digest(&normalized).unwrap(),
                    normalized,
                    source_ids: vec![source_id],
                },
            );
            flow_id
        })
        .collect()
}

fn add_kotlin_exact_call_relations(e: &mut ServiceEvidence, declaration_id: &str) -> Vec<String> {
    let owner = e.observations[declaration_id].clone();
    let owner_source = e.sources[&owner.source_ids[0]].clone();
    let expression = "api.pick(\"x\")";
    let prefix = "\n".repeat(22);
    let full_compilation_source = format!("{prefix}{}", owner_source.text);
    let full_compilation_source_digest =
        crate::canonical::hash_bytes(full_compilation_source.as_bytes());
    let positions = owner_source
        .text
        .match_indices(expression)
        .map(|(start, _)| prefix.len() + start)
        .collect::<Vec<_>>();
    assert_eq!(positions.len(), 2);

    positions
        .into_iter()
        .enumerate()
        .map(|(ordinal, byte_start)| {
            let (relation_id, source_id) = if declaration_id == "answer-next" {
                (
                    format!("call-relation-{ordinal}"),
                    format!("call-source-{ordinal}"),
                )
            } else {
                (
                    format!("call-relation-{declaration_id}-{ordinal}"),
                    format!("call-source-{declaration_id}-{ordinal}"),
                )
            };
            let evidence_binding = format!(
                "sha256:{}",
                (if ordinal == 0 { "c" } else { "d" }).repeat(64)
            );
            let mut source = owner_source.clone();
            source.id = source_id.clone();
            source.start_line = owner_source.start_line + 2;
            source.end_line = source.start_line;
            source.text = expression.into();
            source.text_digest = crate::canonical::hash_bytes(expression.as_bytes());
            source.evidence_digest = evidence_binding.clone();
            source.occurrence = None;
            e.sources.insert(source_id.clone(), source.clone());

            let normalized = json!({
                "schema":"codeclew-kotlin-documentation-call/1.0",
                "kind":"RELATION",
                "relationKind":"CALLS",
                "resolution":"COMPILER_EXACT",
                "compilerResolution":"PROVEN",
                "provider":"K2_FIR",
                "compilerSchema":"declaration-relation/0.1",
                "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "sourceIdentity":owner.symbol,
                "sourceCompilerCallableId":owner.normalized["compilerCallableId"],
                "sourceJvmDescriptor":owner.normalized["jvmDescriptor"],
                "targetCompilerCallableId":"parity/Api.pick",
                "targetJvmDescriptor":"(Ljava/lang/String;)Ljava/lang/String;",
                "targetIdentity":"callable:parity/Api.pick#jvm:(Ljava/lang/String;)Ljava/lang/String;",
                "scope":owner.normalized["scope"],
                "evidenceBinding":evidence_binding,
                "callSite":{
                    "file":owner_source.file,
                    "startLine":source.start_line,
                    "endLine":source.end_line,
                    "byteStart":byte_start,
                    "byteEnd":byte_start + expression.len(),
                    "sourceId":source_id,
                    "sourceDigest":source.text_digest,
                    "sourceStatus":"SOURCE_RETAINED",
                    "evidenceDigest":source.evidence_digest,
                    "fullCompilationSourceDigest":full_compilation_source_digest
                }
            });
            let relation = Observation {
                id: relation_id.clone(),
                kind: "CALL_RELATION".into(),
                service: e.service.clone(),
                symbol: owner.symbol.clone(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![source_id],
            };
            e.observations.insert(relation_id.clone(), relation);
            relation_id
        })
        .collect()
}

fn kotlin_retained_call_sites_evidence() -> ServiceEvidence {
    let retained = concat!(
        "fun next(value: Int): String {\r\n",
        "  // π🙂\r\n",
        "  val marker = \"π🙂\"; val first = api.pick(\"x\"); val second = api.pick(\"x\")\r\n",
        "  return marker + first + second\r\n",
        "}"
    );
    let mut evidence = evidence();
    add_kotlin_declaration(&mut evidence, "answer-next", "FUNCTION", retained);
    add_kotlin_return_outline(&mut evidence, "answer-next");
    add_kotlin_exact_call_relations(&mut evidence, "answer-next");
    evidence
}

fn kotlin_answer_next_outline_evidence() -> ServiceEvidence {
    use crate::thread_flow_cfg::{LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNodeRole};

    let mut evidence = evidence();
    let retained = "fun next(value: Int): Int {\r\n  // π\r\n  return value\r\n}";
    add_kotlin_declaration(&mut evidence, "answer-next", "FUNCTION", retained);
    let comment_start = retained.find("// π").unwrap();
    let return_start = retained.find("return value").unwrap();
    add_kotlin_return_outline(&mut evidence, "answer-next");
    add_local_cfg(
        &mut evidence,
        "answer-next",
        "next-outline-graph",
        &[
            LocalCfgNodeRole::Entry,
            LocalCfgNodeRole::Operation,
            LocalCfgNodeRole::Return,
        ],
        &[
            None,
            Some((comment_start, comment_start + "// π".len())),
            Some((return_start, return_start + "return value".len())),
        ],
        vec![
            LocalCfgEdge {
                source_node_id: 0,
                target_node_id: 1,
                kind: LocalCfgEdgeKind::Next,
                label: None,
            },
            LocalCfgEdge {
                source_node_id: 1,
                target_node_id: 2,
                kind: LocalCfgEdgeKind::Return,
                label: Some("CompilerReturn".into()),
            },
        ],
    );
    evidence
}

fn kotlin_argument_bindings_evidence() -> ServiceEvidence {
    let retained = concat!(
        "fun invoke(): String {\r\n",
        "  // π🙂\r\n",
        "  val first = api.pick(last = \"π🙂\", first = \"x\"); val second = api.pick(last = \"🌙\", first = \"y\")\r\n",
        "  return first + second\r\n",
        "}"
    );
    let mut evidence = evidence();
    add_kotlin_declaration(&mut evidence, "bindings-owner", "FUNCTION", retained);
    let owner = evidence.observations["bindings-owner"].clone();
    let mut owner_source = evidence.sources[&owner.source_ids[0]].clone();
    owner_source.evidence_digest = format!("sha256:{}", "b".repeat(64));
    owner_source.occurrence = None;
    evidence
        .sources
        .insert(owner_source.id.clone(), owner_source.clone());
    let prefix = "\n".repeat(22);
    let full_compilation_source = format!("{prefix}{}", owner_source.text);
    let full_compilation_source_digest =
        crate::canonical::hash_bytes(full_compilation_source.as_bytes());
    let target_callable = "parity/Api.pick";
    let target_descriptor = "(Ljava/lang/String;ILjava/lang/String;)Ljava/lang/String;";
    let target_identity = format!("callable:{target_callable}#jvm:{target_descriptor}");
    let calls = [
        (
            "api.pick(last = \"π🙂\", first = \"x\")",
            [("\"π🙂\"", "last", 2_u64), ("\"x\"", "first", 0_u64)],
        ),
        (
            "api.pick(last = \"🌙\", first = \"y\")",
            [("\"🌙\"", "last", 2_u64), ("\"y\"", "first", 0_u64)],
        ),
    ];
    for (ordinal, (expression, mapped_arguments)) in calls.into_iter().enumerate() {
        let relative_call_start = owner_source.text.find(expression).unwrap();
        let byte_start = prefix.len() + relative_call_start;
        let byte_end = byte_start + expression.len();
        let start_line = owner_source.start_line
            + owner_source.text.as_bytes()[..relative_call_start]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64;
        let source_id = format!("bindings-call-source-{ordinal}");
        let relation_id = format!("bindings-call-relation-{ordinal}");
        let evidence_binding =
            format!("sha256:{}", if ordinal == 0 { "e" } else { "f" }.repeat(64));
        let mut source = owner_source.clone();
        source.id = source_id.clone();
        source.start_line = start_line;
        source.end_line = start_line;
        source.text = expression.to_owned();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        source.evidence_digest = evidence_binding.clone();
        source.occurrence = None;
        evidence.sources.insert(source_id.clone(), source.clone());

        let argument_to_parameter = mapped_arguments
            .into_iter()
            .map(|(argument_expression, parameter, parameter_index)| {
                let relative_start = expression.find(argument_expression).unwrap();
                let start = byte_start + relative_start;
                let end = start + argument_expression.len();
                json!({
                    "argumentStart":start,
                    "argumentEnd":end,
                    "argumentType":"kotlin/String",
                    "parameter":parameter,
                    "parameterIndex":parameter_index,
                    "parameterType":"kotlin/String"
                })
            })
            .collect::<Vec<_>>();
        let normalized = json!({
            "schema":"codeclew-kotlin-documentation-call/1.0",
            "kind":"RELATION",
            "relationKind":"CALLS",
            "resolution":"COMPILER_EXACT",
            "compilerResolution":"PROVEN",
            "provider":"K2_FIR",
            "compilerSchema":"declaration-relation/0.1",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "sourceIdentity":owner.symbol,
            "sourceCompilerCallableId":owner.normalized["compilerCallableId"],
            "sourceJvmDescriptor":owner.normalized["jvmDescriptor"],
            "targetCompilerCallableId":target_callable,
            "targetJvmDescriptor":target_descriptor,
            "targetIdentity":target_identity,
            "scope":owner.normalized["scope"],
            "evidenceBinding":evidence_binding,
            "argumentBindings":{
                "schema":"codeclew-call-argument-bindings/1.0",
                "argumentToParameter":argument_to_parameter,
                "omittedDefaultParameterIndices":[1]
            },
            "callSite":{
                "file":owner_source.file,
                "startLine":start_line,
                "endLine":start_line,
                "byteStart":byte_start,
                "byteEnd":byte_end,
                "sourceId":source_id,
                "sourceDigest":source.text_digest,
                "sourceStatus":"SOURCE_RETAINED",
                "evidenceDigest":evidence_binding,
                "fullCompilationSourceDigest":full_compilation_source_digest
            }
        });
        evidence.observations.insert(
            relation_id.clone(),
            Observation {
                id: relation_id,
                kind: "CALL_RELATION".into(),
                service: evidence.service.clone(),
                symbol: owner.symbol.clone(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![source_id],
            },
        );
    }
    evidence
}

fn kotlin_selection(endpoint: &str, worker: &str) -> Selection {
    Selection {
        id: "kotlin-page".into(),
        service: "sample".into(),
        endpoint_declaration: endpoint.into(),
        worker_declaration: worker.into(),
        wiring_declaration: None,
        question: None,
        note_ids: vec![],
        authored_paragraphs: vec![],
        expand_source_calls: false,
        expand_data_state: false,
    }
}

/// Fixture compiler relations bind exact occurrences independently of the
/// projector. No answer labels, operation annotations, or Profile DSL exist.
fn bind_call(
    e: &mut ServiceEvidence,
    declaration: &str,
    expression: &str,
    target: &str,
    kind: &str,
    dependency_status: Option<&str>,
) {
    let owner = e.observations[declaration].clone();
    let body = e.sources[&owner.source_ids[0]].clone();
    let start = body.text.find(expression).unwrap();
    let end = start + expression.len();
    let sid = format!("site-{declaration}-{}", e.observations.len());
    let mut site = body.clone();
    site.id = sid.clone();
    site.text = expression.into();
    site.text_digest = crate::canonical::hash_bytes(expression.as_bytes());
    site.start_line =
        body.start_line + body.text[..start].bytes().filter(|b| *b == b'\n').count() as u64;
    site.end_line = site.start_line + expression.lines().count() as u64 - 1;
    site.occurrence.as_mut().unwrap().start_byte = start;
    site.occurrence.as_mut().unwrap().end_byte = end;
    let normalized = json!({"sourceIdentity":owner.symbol,"targetIdentity":target,"scope":"compile-scope","resolution":"COMPILER_EXACT","relationKind":kind,"callSite":{"sourceId":sid,"sourceStatus":"SOURCE_RETAINED","sourceDigest":site.text_digest,"evidenceDigest":site.evidence_digest,"byteStart":start,"byteEnd":end}});
    let rid = format!("relation-{sid}");
    e.observations.insert(
        rid.clone(),
        Observation {
            id: rid,
            kind: "CALL_RELATION".into(),
            service: e.service.clone(),
            symbol: owner.symbol,
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![sid.clone()],
        },
    );
    e.sources.insert(sid, site);
    if let Some(status) = dependency_status {
        let id = format!("dependency-{target}");
        let normalized = json!({"schema":JAVA_SCHEMA,"resolution":"COMPILER_EXACT","symbolIdentity":target,"scope":"compile-scope","sourceStatus":status});
        e.observations.insert(
            id.clone(),
            Observation {
                id,
                kind: "DEPENDENCY_TARGET".into(),
                service: e.service.clone(),
                symbol: target.into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
    }
}

fn scenario(
    ingress: &str,
    consumer: &str,
    queue: &str,
    transform: &str,
    condition: &str,
    conditional_wiring: bool,
) -> (Check, Selection) {
    let mut e = evidence();
    let endpoint = format!(
        "void accept(String sku) {{\n  Task created = new Task(sku);\n  {queue}.offer(created);\n}}"
    );
    let worker = format!(
        "void tick() {{\n  Task task = {queue}.poll();\n  if (task == null) return;\n  if (!enabled) return;\n  if ({condition}) return;\n  String prepared = {transform};\n  Response response = gateway.send(prepared);\n  if (!response.ok()) throw new IllegalStateException();\n  this.lastResponse = response;\n}}"
    );
    add_declaration(&mut e, "endpoint", ingress, "accept", "METHOD", &endpoint);
    add_declaration(&mut e, "worker", consumer, "tick", "METHOD", &worker);
    let ctor_e = add_declaration(
        &mut e,
        "constructor-endpoint",
        ingress,
        "<init>",
        "CONSTRUCTOR",
        &format!("{ingress}(BlockingQueue<Task> source) {{\n this.{queue} = source;\n}}"),
    );
    let ctor_w = add_declaration(
        &mut e,
        "constructor-worker",
        consumer,
        "<init>",
        "CONSTRUCTOR",
        &format!("{consumer}(BlockingQueue<Task> source) {{\n this.{queue} = source;\n}}"),
    );
    for (id, owner) in [("field-endpoint", ingress), ("field-worker", consumer)] {
        add_declaration(
            &mut e,
            id,
            owner,
            queue,
            "FIELD",
            &format!("final BlockingQueue<Task> {queue};"),
        );
    }
    let constructions = format!(
        "  {ingress} entry = new {ingress}(shared);\n  {consumer} worker = new {consumer}(pipe);\n"
    );
    let wiring = format!(
        "static void build() {{\n  BlockingQueue<Task> pipe = new LinkedBlockingQueue<>();\n  BlockingQueue<Task> shared = pipe;\n{}\n}}",
        if conditional_wiring {
            format!("if (enabled) {{\n{constructions}}}")
        } else {
            constructions
        }
    );
    add_declaration(&mut e, "wiring", "Bootstrap", "build", "METHOD", &wiring);
    bind_call(
        &mut e,
        "endpoint",
        &format!("{queue}.offer(created)"),
        "method:class:java.util.Queue#offer(Ljava/lang/Object;)Z",
        "CALLS",
        Some("SOURCE_UNAVAILABLE"),
    );
    bind_call(
        &mut e,
        "worker",
        &format!("{queue}.poll()"),
        "method:class:java.util.Queue#poll()Ljava/lang/Object;",
        "CALLS",
        Some("SOURCE_UNAVAILABLE"),
    );
    bind_call(
        &mut e,
        "worker",
        "gateway.send(prepared)",
        "method:class:publicapi.Gateway#send(Ljava/lang/String;)LResponse;",
        "CALLS",
        Some("SOURCE_UNAVAILABLE"),
    );
    bind_call(
        &mut e,
        "wiring",
        "new LinkedBlockingQueue<>()",
        "method:class:java.util.concurrent.LinkedBlockingQueue#<init>()V",
        "CONSTRUCTS",
        Some("SOURCE_UNAVAILABLE"),
    );
    bind_call(
        &mut e,
        "wiring",
        &format!("new {ingress}(shared)"),
        &ctor_e,
        "CONSTRUCTS",
        None,
    );
    bind_call(
        &mut e,
        "wiring",
        &format!("new {consumer}(pipe)"),
        &ctor_w,
        "CONSTRUCTS",
        None,
    );
    let checked = check::assemble(
        "input-digest".into(),
        BTreeMap::from([("sample".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    (
        checked,
        Selection {
            id: "journey".into(),
            service: "sample".into(),
            endpoint_declaration: "endpoint".into(),
            worker_declaration: "worker".into(),
            wiring_declaration: Some("wiring".into()),
            question: None,
            note_ids: vec![],
            authored_paragraphs: vec![],
            expand_source_calls: false,
            expand_data_state: false,
        },
    )
}

fn page(c: &Check, s: Selection) -> PageContent {
    project(c, &[s]).unwrap().pages.remove(0)
}

// Frozen before extraction from the Java-only static-pages projector.
const JAVA_PROJECTION_BASELINE: &str = r###"BASELINE_PROJECTION guarded-shared-queue sha256:881de0eca3fdb90299703b83589216d5a6b5c1bb0fb7028e7d40c1574b1d65bd
BASELINE_FILE guarded-shared-queue catalogue.json sha256:ce5385a366833bec74661a9e2055e55642af97b05f5d85444817e80181e82181
BASELINE_FILE guarded-shared-queue index.html sha256:539eacd7f83d38cff51178b78c65744cc40bf7ff0dd95fbb68afa34384ec61eb
BASELINE_FILE guarded-shared-queue index.mdx sha256:f374dc09b421ff040bc03626491aeb28382bb778523885ad1033dbf9ae9e8f7e
BASELINE_FILE guarded-shared-queue journey-diagnostic.html sha256:cf4fb297be8619ac6d2fa6d9d86b3945710350aa5123e9f2aef6554a4f7f2d09
BASELINE_FILE guarded-shared-queue journey-diagnostic.mdx sha256:22807392821f77807cb981716f4bfc7a08ed5c71b44c790401235245f3d9440c
BASELINE_FILE guarded-shared-queue journey-endpoint.html sha256:65cb61fa44ee11727ff67f033f54568f2c7a38e2d933f19b43f38bd94950f26c
BASELINE_FILE guarded-shared-queue journey-endpoint.mdx sha256:9063cd4d7073aaa4d421987d2cae030073e10fa17438c1a55accd1d6615ed524
BASELINE_FILE guarded-shared-queue journey-fields-state.html sha256:934c63298e6652729cf2a6eb0e7811c42053d34fc98776031e1af244d6fa07fe
BASELINE_FILE guarded-shared-queue journey-fields-state.mdx sha256:0868fc56a83cff245378ced9fceb1077125ca4d1feeef6614e0cb6e73348ff29
BASELINE_FILE guarded-shared-queue journey-overview.html sha256:e893e5c9456477f44d854dab583654290bb7498ea225e46ac84e3be83f0ed52f
BASELINE_FILE guarded-shared-queue journey-overview.mdx sha256:7b89ef4ecb3297e3f2da1a62498d99cff5c105c02b636b7e27f70a3c70e9d1b2
BASELINE_FILE guarded-shared-queue journey-worker.html sha256:db3a5d992b030a91d598be97592a310629afaa25cdb620f478348fbd8caf87c0
BASELINE_FILE guarded-shared-queue journey-worker.mdx sha256:796d391728f09094397eea054947cc0f9e42b8f983f475548b2af453a31a3600
BASELINE_FILE guarded-shared-queue manifest.json sha256:36b0c321d8726d1eb0ed926212856ad6438ad9f599e8ca135d5b6777d881a296
BASELINE_FILE guarded-shared-queue native-reader.js sha256:ce9995c761f68342e490f9d6c96d481c6a93c8c706f3bd336513fedda3108425
BASELINE_FILE guarded-shared-queue projection.json sha256:0a24e4f57e9a78a5121bab821430375dd642cea238227395fae7b365b767f1f6
BASELINE_FILE guarded-shared-queue sources.html sha256:764fca35142d69c01514bb7fa2e6f2b86f15b8bb77d9db55601ac49bbc66a997
BASELINE_FILE guarded-shared-queue sources.mdx sha256:3d4babfbaffe7ac6eca0f0f33fd09d8d14c060e5fdf1f726311be7345a44c2e7
BASELINE_FILE guarded-shared-queue style.css sha256:30f2a7247d667d5fc0ae6992a7aff6761faeef34b00a391252bb295c15ee4109
BASELINE_PROJECTION linked-shared-child-cycle sha256:2bfca86d6856ac6cf6325ca06021819c6bacb290a57c27f118e190a25c5f77de
BASELINE_FILE linked-shared-child-cycle catalogue.json sha256:af2c9b4dee5e02e84cfe3e3aae524c17d324c0737afca453c6a5f865960d303a
BASELINE_FILE linked-shared-child-cycle child-diagnostic.html sha256:241f5b5c7ed0ff25a6173d7ce4a49a2766912b61b0cbdf654d968bc918a41e2e
BASELINE_FILE linked-shared-child-cycle child-diagnostic.mdx sha256:1d0e4972c0cf8599cfd8ac06df431c2af17e1cd256da49e13b64d7db0044674f
BASELINE_FILE linked-shared-child-cycle child-endpoint.html sha256:a5f2281ac09e254f10d406c67b2dc6d523beebe3b160b2b7a03d4cd1c4c1aa35
BASELINE_FILE linked-shared-child-cycle child-endpoint.mdx sha256:100f3f077b9099c10b7748c2439bbc10885f88426d2d3dc2c416af18bf6cefd8
BASELINE_FILE linked-shared-child-cycle child-fields-state.html sha256:9dbdfa6395b996864ea0240ee8cea5d91173906edc8e995ada823443ce68d28d
BASELINE_FILE linked-shared-child-cycle child-fields-state.mdx sha256:38556a14f53e726c28184077aa2c8d71b163c8484b53f6db852103a8a3e3c7e2
BASELINE_FILE linked-shared-child-cycle child-overview.html sha256:49a8a2e4e17a3ebecd41937436337b4d177295ea58007e89a462eff3c44f628e
BASELINE_FILE linked-shared-child-cycle child-overview.mdx sha256:f00f79c7fa996811aed26393a30489e8092eeaa3cb73cdf45a87b4cc4779c553
BASELINE_FILE linked-shared-child-cycle child-worker.html sha256:6ff1b5ffbe2d112a7a2a10dc9cfd7bd922dd33044218b86d103b3e2f48e4f16b
BASELINE_FILE linked-shared-child-cycle child-worker.mdx sha256:5549885caabd13367452f5dda175db1dbe44db11bcb9d77c24b6e2e18fdc0f0a
BASELINE_FILE linked-shared-child-cycle cycle-diagnostic.html sha256:90a3d2ced80b9c152da04601da443f6608a29c171a3b1b22b939a331e0d8283f
BASELINE_FILE linked-shared-child-cycle cycle-diagnostic.mdx sha256:2f239cf20ea9f6d454aba13087d70d87d2db8c638fd46accd1c631dec0fd0f0a
BASELINE_FILE linked-shared-child-cycle cycle-endpoint.html sha256:ba05316c95040a776f1f548f70f379bdea3f7db6d00bae0099608d5c9a03212a
BASELINE_FILE linked-shared-child-cycle cycle-endpoint.mdx sha256:5c256946195279d4342569a970bf9f4df22ed3c3461653eac369aa5411873ad9
BASELINE_FILE linked-shared-child-cycle cycle-fields-state.html sha256:74c9b7c67f7ec36355d422d1486e6bb23630a90294b17ea04f9a8266ef6a837e
BASELINE_FILE linked-shared-child-cycle cycle-fields-state.mdx sha256:76b75b876ded11edb6ce750d75c49869a2baa59cc6d41ba4a608821eb95f3b6b
BASELINE_FILE linked-shared-child-cycle cycle-overview.html sha256:4e4aa74c9ffb331ec7e10628044f2fe591d09b2474e7210c6d17a8a52a15c28e
BASELINE_FILE linked-shared-child-cycle cycle-overview.mdx sha256:329428e123e6985ea3f868b0064a02174113a704aca2da4f50f80df8a7f14c2f
BASELINE_FILE linked-shared-child-cycle cycle-worker.html sha256:2fdafccfcc1811d2428ce94639eca73416365fddc738d50b209bbcd64825d534
BASELINE_FILE linked-shared-child-cycle cycle-worker.mdx sha256:bf5a7436ab20fff2b4a8ad34f964c21ab338f74ff708bc2994e01db9c3def7ce
BASELINE_FILE linked-shared-child-cycle index.html sha256:d0da473e8ebea7131646f18ec64a5ee3b3dd4283c9e5f3f2fa012b7627575a5c
BASELINE_FILE linked-shared-child-cycle index.mdx sha256:533375ae2f024c2dac08c75a59dda03ad8628ba82b876bed6d19ee93973634d2
BASELINE_FILE linked-shared-child-cycle manifest.json sha256:45c7a69ca6ec7d602aa384827876b532d099361338ef2ee9020db28fc42d5ed4
BASELINE_FILE linked-shared-child-cycle native-reader.js sha256:ce9995c761f68342e490f9d6c96d481c6a93c8c706f3bd336513fedda3108425
BASELINE_FILE linked-shared-child-cycle parent-a-diagnostic.html sha256:27054d11b2dcf36d9e4046c0f07f80fb3fb4d9a9de540e47a88175ab9961b6a0
BASELINE_FILE linked-shared-child-cycle parent-a-diagnostic.mdx sha256:384430b68fd2215210678c9ae30af847f32cd435726d5d17fea287d7203ab06b
BASELINE_FILE linked-shared-child-cycle parent-a-endpoint.html sha256:c919a6f1cfbe1669fe2fa48f3c3868f04b26e8d3edeb38fa52910385091eb4dd
BASELINE_FILE linked-shared-child-cycle parent-a-endpoint.mdx sha256:ce69da906bc9c8ea755bc175770e7a454d77cf301f735ec97f1db29360a5ccf9
BASELINE_FILE linked-shared-child-cycle parent-a-fields-state.html sha256:dbb1d55df450bb3dfdeab5c97fa50b4f0b3bf5ad7c3eb9086bb58718a055681e
BASELINE_FILE linked-shared-child-cycle parent-a-fields-state.mdx sha256:dc6197db24a8df796cf95e560111cdf5b896f56047980db3fbede1a6e2093ead
BASELINE_FILE linked-shared-child-cycle parent-a-overview.html sha256:b60ddf8070d549546ccd809cd018433cf43536f331a2bc36a6ec1046fafdbf51
BASELINE_FILE linked-shared-child-cycle parent-a-overview.mdx sha256:16cc4998161ab92118058b08a0545df433611741bb3193f895f115288ce8a2c3
BASELINE_FILE linked-shared-child-cycle parent-a-worker.html sha256:6b6195bbe3a7a911c9ce980ce2176c19adfe0056fb383427a7097f60db4dce38
BASELINE_FILE linked-shared-child-cycle parent-a-worker.mdx sha256:9240938b39d7dc1c508e751aa29efc46b780c3d47f03b8d95d96f50dd90e2ed0
BASELINE_FILE linked-shared-child-cycle parent-b-diagnostic.html sha256:e88112a02edd49655cffdf01938571e73feebf3d6a989f7a1db8c5b5e2e96173
BASELINE_FILE linked-shared-child-cycle parent-b-diagnostic.mdx sha256:7ceeb01b690e32cd129024421300a482d0a8a961872e3d16f1d07f83cf50f30b
BASELINE_FILE linked-shared-child-cycle parent-b-endpoint.html sha256:e65fc0fa8b41a1e1b907946bfedf3ff0aedcd34d05b2e31b027e58d1c9bf9225
BASELINE_FILE linked-shared-child-cycle parent-b-endpoint.mdx sha256:cd87a7f0154286e0dd729a75f03fc3a9e2cf9155c7177e7eda792846426b40bf
BASELINE_FILE linked-shared-child-cycle parent-b-fields-state.html sha256:5e46355f0e33ac3c3ac24401035255d785f6917aea6abb71aeee942c30d6c74b
BASELINE_FILE linked-shared-child-cycle parent-b-fields-state.mdx sha256:1e0a4c37a99f5cb8085e4e5a20a145a7313caaa58bd83357fd73d0780736e8a3
BASELINE_FILE linked-shared-child-cycle parent-b-overview.html sha256:f91813b370b3a100ead84b0f9191bcaf259e09adc82dc99125082d9c36862e5a
BASELINE_FILE linked-shared-child-cycle parent-b-overview.mdx sha256:3c52bbfbf039690bbf1c3d5397ce00ff1a973b57647c9684da11fd1f88a66ed2
BASELINE_FILE linked-shared-child-cycle parent-b-worker.html sha256:a5d8fd0764ed1bcb1f9483be7fd755e6985408cbe0375e7e2a88bb02a2c6235e
BASELINE_FILE linked-shared-child-cycle parent-b-worker.mdx sha256:c03819d986b79f0d32746bfffcf79093019ee4a489518c3fecfb35c302807c6c
BASELINE_FILE linked-shared-child-cycle projection.json sha256:282c6fbcf5c6faf722e094987589513c7f8a3535151af0250685e1555e40f416
BASELINE_FILE linked-shared-child-cycle source-calls.html sha256:23bf6cf1381895bf7807c8a04d04ed9be92ce82f8aa1c99028db0a925bfd4567
BASELINE_FILE linked-shared-child-cycle source-calls.mdx sha256:394623da9ce4c1bf20be46cc62a63bd450aff7344a11fbb3175b86ea5ed50ebd
BASELINE_FILE linked-shared-child-cycle sources.html sha256:cae07648cb2191431a565848edb63d24ade01f60956012f48bd24382bcc4d6c4
BASELINE_FILE linked-shared-child-cycle sources.mdx sha256:81fdb292f2043c1a331d85d14ed2d37a4c90b45cecc40dae2e45dff3a118f9d9
BASELINE_FILE linked-shared-child-cycle style.css sha256:30f2a7247d667d5fc0ae6992a7aff6761faeef34b00a391252bb295c15ee4109
BASELINE_PROJECTION unicode-unavailable-dependency sha256:bd96d75aa54eae141c90b8aab7ece14b9ae84ce70f2819872bc5ff1d3979406b
BASELINE_FILE unicode-unavailable-dependency catalogue.json sha256:a9ca4ac71b54f797069f38857ca354216f22d2d95279c3ba3ae2349eb60fd06a
BASELINE_FILE unicode-unavailable-dependency index.html sha256:e8f4a04d54a15ea15dd89d2e55e65f94e1078f2d5a1c158d377cec23623f5168
BASELINE_FILE unicode-unavailable-dependency index.mdx sha256:dc5d5a85a62d501c1bbc793e48dfb9bea6b6c505c9247a6fd275a9e023662143
BASELINE_FILE unicode-unavailable-dependency journey-diagnostic.html sha256:45b77d464d1b34e94a1630160948db6e6fb7df24cadddf62a29421cf337c78e3
BASELINE_FILE unicode-unavailable-dependency journey-diagnostic.mdx sha256:66d25591a2b933cfd7fe6d8b639dbc21562eaa8dcd53d1c53a2c4218389bc785
BASELINE_FILE unicode-unavailable-dependency journey-endpoint.html sha256:65cb61fa44ee11727ff67f033f54568f2c7a38e2d933f19b43f38bd94950f26c
BASELINE_FILE unicode-unavailable-dependency journey-endpoint.mdx sha256:9063cd4d7073aaa4d421987d2cae030073e10fa17438c1a55accd1d6615ed524
BASELINE_FILE unicode-unavailable-dependency journey-fields-state.html sha256:51716a8532149fd86a7deaf6381aa9c5d26c6cdf449aa060b9fbbe43954b19d0
BASELINE_FILE unicode-unavailable-dependency journey-fields-state.mdx sha256:49739f4ab5dedf581b3de460879836a8a06c971fa276e8b7fb7510773b9a1caa
BASELINE_FILE unicode-unavailable-dependency journey-overview.html sha256:55f00cd6f33304cdbe3af027be05d9134ef33a5d00ffa8f17763cf40e2f16e4a
BASELINE_FILE unicode-unavailable-dependency journey-overview.mdx sha256:a967c21aab1a513d1114552d3218841abde4bec84708449135c58adf257b3d64
BASELINE_FILE unicode-unavailable-dependency journey-worker.html sha256:85344b00af809a57657266b4e4ad9d1a6e9118179383611dd32582d3bafda4d9
BASELINE_FILE unicode-unavailable-dependency journey-worker.mdx sha256:4d7ce63ae654ce34428c93bedfb87e8384aecddb55e3ddaebe97eb1388b4543d
BASELINE_FILE unicode-unavailable-dependency manifest.json sha256:24d7c9667a4c285aee42187b847300daedbdba96e057c5cba8300357e8ebc057
BASELINE_FILE unicode-unavailable-dependency native-reader.js sha256:ce9995c761f68342e490f9d6c96d481c6a93c8c706f3bd336513fedda3108425
BASELINE_FILE unicode-unavailable-dependency projection.json sha256:76793d09f18e16ca3c0dc70bf2d438e5d10478cae5396dbf8edc920840a15e65
BASELINE_FILE unicode-unavailable-dependency sources.html sha256:3e70f6ed9a643f134e57e5eb4ecc86e8cf13782edc65b3bfa115c783bfe4ce88
BASELINE_FILE unicode-unavailable-dependency sources.mdx sha256:b80644a4dfa51318f27ea358954f79875b93ead2a64c3dd790eabbbdfb2ba81f
BASELINE_FILE unicode-unavailable-dependency style.css sha256:30f2a7247d667d5fc0ae6992a7aff6761faeef34b00a391252bb295c15ee4109"###;

fn java_projection_facts(case: &str, checked: &Check, selections: &[Selection]) -> String {
    let projection = project(checked, selections).unwrap();
    let encoded = crate::documentation::bytes(&projection).unwrap();
    let mut facts = vec![format!(
        "BASELINE_PROJECTION {case} {}",
        crate::canonical::hash_bytes(&encoded)
    )];
    let output = tempfile::tempdir().unwrap();
    super::super::publish::write(output.path(), "source-boundary-baseline-v1", &projection)
        .unwrap();
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(output.path()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        let bytes = std::fs::read(entry.path()).unwrap();
        files.insert(name, crate::canonical::hash_bytes(&bytes));
    }
    facts.extend(
        files
            .into_iter()
            .map(|(name, digest)| format!("BASELINE_FILE {case} {name} {digest}")),
    );
    facts.join("\n")
}

#[test]
fn java_projection_bytes_and_published_files_match_the_preboundary_baseline() {
    let mut actual = Vec::new();
    let (checked, selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku.trim()",
        "!task.eligible()",
        false,
    );
    let legacy = project(&checked, std::slice::from_ref(&selection)).unwrap();
    assert_eq!(legacy.schema, SCHEMA);
    assert!(
        serde_json::to_value(&legacy).unwrap()["pages"][0]
            .get("projectionKind")
            .is_none()
    );
    actual.push(java_projection_facts(
        "guarded-shared-queue",
        &checked,
        &[selection],
    ));

    let (checked, selections) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    actual.push(java_projection_facts(
        "linked-shared-child-cycle",
        &checked,
        &selections,
    ));

    let (checked, selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "\"café☕\"",
        "!task.eligible()",
        false,
    );
    actual.push(java_projection_facts(
        "unicode-unavailable-dependency",
        &checked,
        &[selection],
    ));

    assert_eq!(JAVA_PROJECTION_BASELINE, actual.join("\n"));
}

#[test]
fn kotlin_function_projection_uses_compiler_bound_source_and_explicit_gaps() {
    let mut evidence = evidence();
    let retained = r#"fun render(value: String) = "Hello, ${value}, $value `literal`""#;
    let symbol = add_kotlin_declaration(&mut evidence, "render", "FUNCTION", retained);
    let checked = kotlin_check(evidence);
    let projection = project(&checked, &[kotlin_selection("render", "render")]).unwrap();
    assert_eq!(projection.schema, DECLARATION_SCHEMA);
    let page = &projection.pages[0];
    assert_eq!(page.projection_kind, Some(ProjectionKind::DeclarationOnly));
    for callable in [&page.endpoint, &page.worker] {
        assert_eq!(callable.authority, "COMPILER_DECLARATION");
        assert_eq!(callable.symbol, symbol);
        assert!(callable.steps.is_empty());
        assert!(callable.state.is_empty());
        assert!(callable.citation_id.is_some());
        for code in [
            "KOTLIN_BEHAVIOR_PROJECTION_UNAVAILABLE",
            "KOTLIN_SOURCE_CALL_GRAPH_UNAVAILABLE",
            "KOTLIN_DATA_STATE_UNAVAILABLE",
        ] {
            assert!(callable.gaps.iter().any(|gap| gap.code == code), "{code}");
        }
    }
    let citation = page.citations[page.endpoint.citation_id.as_ref().unwrap()].clone();
    let source = &page.sources[&citation.source_id];
    assert_eq!(source.text, retained);
    assert_eq!(citation.file, "src/main/kotlin/example/Sample.kt");
    assert_eq!(citation.start_line, 23);
    assert_eq!(citation.end_line, 23);
    assert_eq!(citation.start_byte, 0);
    assert_eq!(citation.end_byte, retained.len());
    assert_eq!(
        citation.text_digest,
        crate::canonical::hash_bytes(retained.as_bytes())
    );
    assert_eq!(page.handoff.status, "DECLARATION_ONLY");
    assert_eq!(
        page.handoff.gaps[0].code,
        "DECLARATION_ONLY_NO_RELATIONSHIP"
    );
    assert_eq!(page.title, format!("Selected functions: {symbol}"));
}

#[test]
fn kotlin_answer_next_outline_is_cited_noncausal_and_keeps_cfg_independent() {
    let checked = kotlin_check(kotlin_answer_next_outline_evidence());
    let projection = project(&checked, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
    assert_eq!(projection.schema, CONTROL_FLOW_SCHEMA);
    let page = &projection.pages[0];
    assert_eq!(
        page.projection_kind,
        Some(ProjectionKind::CompilerControlFlow)
    );
    let outline = page.endpoint.source_outline.as_ref().unwrap();
    assert_eq!(outline.authority, "KOTLIN_PSI_WITH_K2_CALL_TARGETS");
    assert_eq!(outline.owner_key.scope, ":/main");
    assert_eq!(
        outline.owner_key.symbol,
        "callable:parity/Answer.next#jvm:(I)I"
    );
    assert_eq!(outline.events.len(), 1);
    assert_eq!(outline.events[0].observation_id, "flow-answer-next-return");
    assert_eq!(outline.events[0].ordinal, 0);
    assert_eq!(outline.events[0].kind, "RETURN");
    assert_eq!(outline.events[0].start_line, 25);
    assert_eq!(
        outline.tree.as_deref(),
        Some("Entry: parity/Answer.next\nreturn\n")
    );
    assert!(outline.gaps.is_empty());
    assert!(page.endpoint.steps.is_empty());
    assert!(page.endpoint.state.is_empty());
    let graph = page.endpoint.control_flow.as_ref().unwrap();
    assert_eq!(graph.graph_observation_id, "cfg-answer-next");
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.edges.len(), 2);
    let owner_source_id = page.observations["answer-next"].source_ids[0].clone();
    assert!(page.sources[&owner_source_id].occurrence.is_none());
    let citation = &page.citations[&outline.events[0].citation_id];
    assert_eq!(citation.file, "src/main/kotlin/parity/Answer.kt");
    assert_eq!((citation.start_line, citation.end_line), (25, 25));
    assert_eq!(citation.start_byte, 0);
    assert_eq!(citation.end_byte, "  return value".len());
    assert!(page.sources[&citation.source_id].occurrence.is_none());

    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let endpoint = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.html")).unwrap();
    assert!(endpoint.contains("Cited Kotlin source outline"));
    assert!(endpoint.contains("PSI conditions do not establish predicate truth"));
    assert!(endpoint.contains("Entry: parity/Answer.next&#10;return&#10;"));
    assert!(endpoint.contains("PSI event ordinal 0 · RETURN · 25-25"));
    assert!(endpoint.contains("sources.html#ref-"));

    let mut tampered = projection.clone();
    tampered.pages[0]
        .endpoint
        .source_outline
        .as_mut()
        .unwrap()
        .tree
        .as_mut()
        .unwrap()
        .push_str("invented text\n");
    let output = temp.path().join("tampered-outline");
    assert!(super::super::publish::write(&output, "snapshot", &tampered).is_err());
    assert!(!output.exists());
}

#[test]
fn kotlin_retained_call_sites_keep_exact_same_line_byte_occurrences() {
    let mut evidence = kotlin_retained_call_sites_evidence();
    let relation_ids = ["call-relation-0".to_owned(), "call-relation-1".to_owned()];
    let owner = evidence.observations.get_mut("answer-next").unwrap();
    owner
        .normalized
        .as_object_mut()
        .unwrap()
        .remove("jvmDescriptor");
    owner.digest = digest(&owner.normalized).unwrap();

    let checked = kotlin_check(evidence);
    let projection = project(&checked, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
    let page = &projection.pages[0];
    let sites = &page.endpoint.retained_call_sites.as_ref().unwrap().sites;
    assert_eq!(sites.len(), 2);
    assert_eq!(
        sites
            .iter()
            .map(|site| site.relation_id.as_str())
            .collect::<Vec<_>>(),
        relation_ids.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(
        sites
            .iter()
            .map(|site| site.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["api.pick(\"x\")", "api.pick(\"x\")"]
    );
    assert!(sites.iter().all(|site| site.argument_bindings.is_none()));
    assert!(
        serde_json::to_value(&sites[0])
            .unwrap()
            .get("argumentBindings")
            .is_none()
    );
    assert_eq!(sites[0].target_identity, sites[1].target_identity);
    assert_eq!(sites[0].file, "src/main/kotlin/parity/Answer.kt");
    assert_eq!((sites[0].start_line, sites[0].end_line), (25, 25));
    assert_eq!((sites[1].start_line, sites[1].end_line), (25, 25));
    assert_eq!(
        sites[0].compilation_byte_end - sites[0].compilation_byte_start,
        13
    );
    assert_eq!(
        sites[1].compilation_byte_end - sites[1].compilation_byte_start,
        13
    );
    assert!(sites[0].compilation_byte_start < sites[1].compilation_byte_start);
    assert_ne!(sites[0].source_id, sites[1].source_id);
    assert_ne!(sites[0].evidence_binding, sites[1].evidence_binding);
    assert_eq!(
        sites[0].full_compilation_source_digest,
        sites[1].full_compilation_source_digest
    );
    assert_ne!(
        sites[0].evidence_binding,
        page.sources[&page.observations["answer-next"].source_ids[0]].evidence_digest
    );
    assert!(
        sites
            .iter()
            .all(|site| page.sources[&site.source_id].occurrence.is_none())
    );
    assert!(page.endpoint.steps.is_empty());
    assert_eq!(page.handoff.status, "DECLARATION_ONLY");
    assert_eq!(
        page.endpoint.source_outline.as_ref().unwrap().events.len(),
        1
    );
    assert_eq!(
        page.endpoint
            .retained_call_sites
            .as_ref()
            .unwrap()
            .gaps
            .len(),
        0
    );

    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let endpoint = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.html")).unwrap();
    assert!(endpoint.contains("Retained exact call sites"));
    assert!(!endpoint.contains("Compiler argument bindings"));
    assert!(endpoint.contains("do not establish runtime execution, invocation count"));
    assert!(endpoint.contains("Target identity: callable:parity/Api.pick#jvm:"));
    assert!(endpoint.contains("Captured source span bytes ["));
    assert!(
        endpoint.contains("api.pick(&quot;x&quot;)") || endpoint.contains("api.pick(&#34;x&#34;)")
    );
    assert!(endpoint.contains("sources.html#ref-"));

    let mut tampered = projection.clone();
    tampered.pages[0]
        .endpoint
        .retained_call_sites
        .as_mut()
        .unwrap()
        .sites[0]
        .compilation_byte_end += 1;
    let output = temp.path().join("tampered-call-site");
    assert!(super::super::publish::write(&output, "snapshot", &tampered).is_err());
    assert!(!output.exists());
}

#[test]
fn kotlin_argument_bindings_keep_source_order_defaults_and_utf8_subspan_citations() {
    let mut selection = kotlin_selection("bindings-owner", "bindings-owner");
    selection.expand_source_calls = true;
    let checked = kotlin_check(kotlin_argument_bindings_evidence());
    let projection = project(&checked, &[selection]).unwrap();
    let page = &projection.pages[0];
    let retained = page.endpoint.retained_call_sites.as_ref().unwrap();
    assert_eq!(retained.sites.len(), 2, "{:#?}", retained.gaps);
    assert_eq!(
        retained.sites[0].start_line, retained.sites[1].start_line,
        "the retained calls share one CRLF source line"
    );

    for site in &retained.sites {
        let bindings = site.argument_bindings.as_ref().unwrap();
        assert_eq!(bindings.schema, "codeclew-call-argument-bindings/1.0");
        assert!(bindings.gaps.is_empty());
        assert_eq!(bindings.omitted_default_parameter_indices, vec![1]);
        assert_eq!(
            bindings
                .arguments
                .iter()
                .map(|argument| argument.parameter_index)
                .collect::<Vec<_>>(),
            vec![2, 0],
            "mapping rows preserve the compiler's source order rather than formal order"
        );
        assert!(
            bindings
                .arguments
                .iter()
                .all(|argument| argument.argument_name.is_none())
        );
        let source = &page.sources[&site.source_id];
        for argument in &bindings.arguments {
            let start =
                usize::try_from(argument.compilation_byte_start - site.compilation_byte_start)
                    .unwrap();
            let end = usize::try_from(argument.compilation_byte_end - site.compilation_byte_start)
                .unwrap();
            assert_eq!(
                source.text.get(start..end),
                Some(argument.expression.as_str())
            );
            let citation = &page.citations[&argument.citation_id];
            assert_eq!((citation.start_byte, citation.end_byte), (start, end));
            assert_eq!(
                &source.text[citation.start_byte..citation.end_byte],
                argument.expression
            );
        }
    }
    assert_ne!(
        retained.sites[0]
            .argument_bindings
            .as_ref()
            .unwrap()
            .arguments[0]
            .citation_id,
        retained.sites[1]
            .argument_bindings
            .as_ref()
            .unwrap()
            .arguments[0]
            .citation_id
    );
    assert!(
        retained.sites[0]
            .argument_bindings
            .as_ref()
            .unwrap()
            .arguments[0]
            .expression
            .contains('π')
    );
    let graph = projection.source_call_graph.as_ref().unwrap();
    let graph_owner = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "bindings-owner")
        .unwrap();
    let graph_sites = graph_owner
        .calls
        .iter()
        .filter_map(|edge| edge.exact_call_site.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(graph_sites.len(), 2);
    assert!(
        retained
            .sites
            .iter()
            .all(|site| graph_sites.contains(&site))
    );

    let output = tempfile::tempdir().unwrap();
    super::super::publish::write(output.path(), "snapshot", &projection).unwrap();
    let endpoint =
        std::fs::read_to_string(output.path().join("kotlin-page-endpoint.html")).unwrap();
    let source_calls = std::fs::read_to_string(output.path().join("source-calls.html")).unwrap();
    assert!(endpoint.contains("Compiler argument bindings"));
    assert!(endpoint.contains("Formal parameter last (index 2): kotlin/String"));
    assert!(endpoint.contains("Omitted default parameter indices reported by the compiler: 1"));
    assert!(source_calls.contains("Compiler argument bindings"));
    assert!(source_calls.contains("Formal parameter first (index 0): kotlin/String"));
}

#[test]
fn kotlin_malformed_argument_binding_children_keep_exact_sites_and_reject_tampering() {
    #[derive(Clone, Copy, Debug)]
    enum Mutation {
        UnsupportedSchema,
        OutOfRangeParameter,
        OutOfCallRange,
        NonUtf8Boundary,
        OverlappingRanges,
        IncompletePartition,
    }
    let cases = [
        (Mutation::UnsupportedSchema, "ARGUMENT_BINDINGS_UNSUPPORTED"),
        (Mutation::OutOfRangeParameter, "ARGUMENT_BINDINGS_REJECTED"),
        (Mutation::OutOfCallRange, "ARGUMENT_BINDINGS_REJECTED"),
        (Mutation::NonUtf8Boundary, "ARGUMENT_BINDINGS_REJECTED"),
        (Mutation::OverlappingRanges, "ARGUMENT_BINDINGS_REJECTED"),
        (Mutation::IncompletePartition, "ARGUMENT_BINDINGS_REJECTED"),
    ];
    for (mutation, expected_gap) in cases {
        let mut evidence = kotlin_argument_bindings_evidence();
        let relation = evidence
            .observations
            .get_mut("bindings-call-relation-0")
            .unwrap();
        match mutation {
            Mutation::UnsupportedSchema => {
                relation.normalized["argumentBindings"]["schema"] =
                    json!("codeclew-call-argument-bindings/2.0");
            }
            Mutation::OutOfRangeParameter => {
                relation.normalized["argumentBindings"]["argumentToParameter"][0]["parameterIndex"] =
                    json!(3);
            }
            Mutation::OutOfCallRange => {
                let call_end = relation.normalized["callSite"]["byteEnd"].as_u64().unwrap();
                relation.normalized["argumentBindings"]["argumentToParameter"][0]["argumentEnd"] =
                    json!(call_end + 1);
            }
            Mutation::NonUtf8Boundary => {
                let call_start = relation.normalized["callSite"]["byteStart"]
                    .as_u64()
                    .unwrap();
                let source = &evidence.sources["bindings-call-source-0"];
                let value_start = source.text.find("\"π🙂\"").unwrap();
                relation.normalized["argumentBindings"]["argumentToParameter"][0]["argumentStart"] =
                    json!(call_start + value_start as u64 + 2);
            }
            Mutation::OverlappingRanges => {
                let second_start = relation.normalized["argumentBindings"]["argumentToParameter"]
                    [1]["argumentStart"]
                    .as_u64()
                    .unwrap();
                relation.normalized["argumentBindings"]["argumentToParameter"][0]["argumentEnd"] =
                    json!(second_start + 1);
            }
            Mutation::IncompletePartition => {
                relation.normalized["argumentBindings"]["omittedDefaultParameterIndices"] =
                    json!([]);
            }
        }
        relation.digest = digest(&relation.normalized).unwrap();

        let mut selection = kotlin_selection("bindings-owner", "bindings-owner");
        selection.expand_source_calls = true;
        let checked = kotlin_check(evidence);
        let projection = project(&checked, &[selection]).unwrap();
        let sites = &projection.pages[0]
            .endpoint
            .retained_call_sites
            .as_ref()
            .unwrap()
            .sites;
        assert_eq!(
            sites.len(),
            2,
            "{mutation:?} rejected a child, not its call site"
        );
        let rejected = sites
            .iter()
            .find(|site| site.relation_id == "bindings-call-relation-0")
            .unwrap();
        let bindings = rejected.argument_bindings.as_ref().unwrap();
        assert!(bindings.arguments.is_empty());
        assert_eq!(bindings.gaps[0].code, expected_gap);
        assert_eq!(
            rejected.expression,
            "api.pick(last = \"π🙂\", first = \"x\")"
        );

        let graph = projection.source_call_graph.as_ref().unwrap();
        let graph_owner = graph
            .nodes
            .values()
            .find(|node| node.callable.declaration_id == "bindings-owner")
            .unwrap();
        assert_eq!(
            graph_owner
                .calls
                .iter()
                .filter(|edge| edge.exact_call_site.is_some())
                .count(),
            2,
            "{mutation:?} must preserve the exact graph edge"
        );
    }

    let mut selection = kotlin_selection("bindings-owner", "bindings-owner");
    selection.expand_source_calls = true;
    let checked = kotlin_check(kotlin_argument_bindings_evidence());
    let mut valid = project(&checked, &[selection]).unwrap();
    valid.pages[0]
        .endpoint
        .retained_call_sites
        .as_mut()
        .unwrap()
        .sites[0]
        .argument_bindings
        .as_mut()
        .unwrap()
        .arguments[0]
        .expression
        .push('!');
    let output = tempfile::tempdir().unwrap();
    let tampered_binding = output.path().join("tampered-binding");
    assert!(super::super::publish::write(&tampered_binding, "snapshot", &valid).is_err());
    assert!(!tampered_binding.exists());

    let mut valid = project(
        &checked,
        &[kotlin_selection("bindings-owner", "bindings-owner")],
    )
    .unwrap();
    valid.pages[0]
        .endpoint
        .retained_call_sites
        .as_mut()
        .unwrap()
        .sites[0]
        .argument_bindings
        .as_mut()
        .unwrap()
        .arguments[0]
        .citation_id = "citation-forged".into();
    let tampered_citation = output.path().join("tampered-citation");
    assert!(super::super::publish::write(&tampered_citation, "snapshot", &valid).is_err());
    assert!(!tampered_citation.exists());

    let mut valid = project(
        &checked,
        &[kotlin_selection("bindings-owner", "bindings-owner")],
    )
    .unwrap();
    let argument_citation_id = valid.pages[0]
        .endpoint
        .retained_call_sites
        .as_ref()
        .unwrap()
        .sites[0]
        .argument_bindings
        .as_ref()
        .unwrap()
        .arguments[0]
        .citation_id
        .clone();
    valid.pages[0]
        .citations
        .get_mut(&argument_citation_id)
        .unwrap()
        .start_byte += 1;
    let tampered_argument_subcitation = output.path().join("tampered-argument-subcitation");
    assert!(
        super::super::publish::write(&tampered_argument_subcitation, "snapshot", &valid).is_err()
    );
    assert!(!tampered_argument_subcitation.exists());
}

#[test]
fn kotlin_exact_source_calls_expand_two_sites_to_one_cached_function_body() {
    use crate::thread_flow_cfg::{LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNodeRole};

    let mut evidence = kotlin_retained_call_sites_evidence();
    let target_symbol = add_kotlin_declaration(
        &mut evidence,
        "api-pick",
        "FUNCTION",
        "    fun pick(value: String): String = value",
    );
    {
        let target = evidence.observations.get_mut("api-pick").unwrap();
        target.symbol =
            "callable:parity/Api.pick#jvm:(Ljava/lang/String;)Ljava/lang/String;".into();
        target.normalized["symbolIdentity"] = json!(target.symbol);
        target.normalized["compilerCallableId"] = json!("parity/Api.pick");
        target.normalized["ownerIdentity"] = json!("class:parity/Api");
        target.normalized["documentation"] = json!({
            "schema":"codeclew-kotlin-documentation-flow/1.0",
            "authority":"KOTLIN_PSI_WITH_K2_CALL_TARGETS",
            "events":[{"kind":"RETURN"}],
            "boundaries":[]
        });
        target.digest = digest(&target.normalized).unwrap();
    }
    assert_ne!(target_symbol, evidence.observations["api-pick"].symbol);
    let owner_source_id = evidence.observations["api-pick"].source_ids[0].clone();
    let revision = evidence.revision.clone();
    {
        let source = evidence.sources.get_mut(&owner_source_id).unwrap();
        source.file = "src/main/kotlin/parity/Api.kt".into();
        source.start_line = 4;
        source.end_line = 4;
        source.evidence_digest = format!("sha256:{}", "d".repeat(64));
        source.occurrence = None;
        source.url = Some(format!(
            "https://example.invalid/sample/blob/{}/{}#L4",
            revision, source.file
        ));
    }
    let flow_source_id = "api-pick-return-source".to_owned();
    let mut flow_source = evidence.sources[&owner_source_id].clone();
    flow_source.id = flow_source_id.clone();
    evidence.sources.insert(flow_source_id.clone(), flow_source);
    let flow_id = "api-pick-return".to_owned();
    let flow = json!({"kind":"RETURN","ordinal":0,"scope":":/main"});
    evidence.observations.insert(
        flow_id.clone(),
        Observation {
            id: flow_id,
            kind: "FLOW".into(),
            service: evidence.service.clone(),
            symbol: evidence.observations["api-pick"].symbol.clone(),
            digest: digest(&flow).unwrap(),
            normalized: flow,
            source_ids: vec![flow_source_id],
        },
    );
    let source_text = evidence.sources[&owner_source_id].text.clone();
    add_local_cfg(
        &mut evidence,
        "api-pick",
        "pick-target-graph",
        &[
            LocalCfgNodeRole::Entry,
            LocalCfgNodeRole::Operation,
            LocalCfgNodeRole::Return,
        ],
        &[None, Some((4, 7)), Some((0, source_text.len()))],
        vec![
            LocalCfgEdge {
                source_node_id: 0,
                target_node_id: 1,
                kind: LocalCfgEdgeKind::Next,
                label: None,
            },
            LocalCfgEdge {
                source_node_id: 1,
                target_node_id: 2,
                kind: LocalCfgEdgeKind::Return,
                label: Some("CompilerReturn".into()),
            },
        ],
    );

    let checked = kotlin_check(evidence.clone());
    let baseline = project(&checked, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
    assert!(baseline.source_call_graph.is_none());
    assert!(
        baseline.pages[0]
            .endpoint
            .gaps
            .iter()
            .any(|gap| gap.code == "KOTLIN_SOURCE_CALL_GRAPH_UNAVAILABLE")
    );
    let graph =
        super::super::linked::build_roots(&checked, &[("sample".into(), "answer-next".into())])
            .unwrap();
    assert_eq!(graph.schema, "codeclew-native-source-calls/1.1");
    assert_eq!(graph.nodes.len(), 2);
    let caller = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    assert_eq!(caller.calls.len(), 2);
    assert!(caller.calls.iter().all(|edge| {
        edge.call.is_none()
            && edge.occurrence_path.is_none()
            && edge.conditions.is_none()
            && edge.reachable.is_none()
            && edge.exact_call_site.is_some()
            && edge.target_node.is_some()
            && edge.status == "RETAINED_DECLARED_BODY"
    }));
    assert_eq!(caller.calls[0].target_node, caller.calls[1].target_node);
    let target = &graph.nodes[caller.calls[0].target_node.as_ref().unwrap()];
    assert_eq!(target.callable.declaration_id, "api-pick");
    assert_eq!(target.callable.symbol, evidence_symbol_for_kotlin_pick());
    assert_eq!(
        target.node_projection_kind,
        Some(ProjectionKind::CompilerControlFlow)
    );
    assert!(target.sources.values().any(|source| {
        source.file == "src/main/kotlin/parity/Api.kt"
            && source.start_line == 4
            && source.text == "    fun pick(value: String): String = value"
    }));
    assert_eq!(
        target
            .callable
            .source_outline
            .as_ref()
            .unwrap()
            .events
            .len(),
        1,
        "{:?}",
        target.callable.source_outline
    );
    assert!(target.callable.control_flow.is_some());
    assert!(graph.process_links.is_empty());

    let mut selection = kotlin_selection("answer-next", "answer-next");
    selection.expand_source_calls = true;
    let expanded = project(&checked, &[selection.clone()]).unwrap();
    let expanded_graph = expanded.source_call_graph.as_ref().unwrap();
    let expanded_caller = expanded_graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    assert_eq!(expanded_caller.calls.len(), 2);
    let expanded_target = expanded_graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "api-pick")
        .unwrap();
    assert!(
        !expanded_target
            .callable
            .gaps
            .iter()
            .any(|gap| gap.code == "KOTLIN_SOURCE_CALL_GRAPH_UNAVAILABLE")
    );
    assert!(
        !expanded.pages[0]
            .endpoint
            .gaps
            .iter()
            .any(|gap| gap.code == "KOTLIN_SOURCE_CALL_GRAPH_UNAVAILABLE")
    );
    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &expanded).unwrap();
    let overview = std::fs::read_to_string(temp.path().join("kotlin-page-overview.html")).unwrap();
    let endpoint = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.html")).unwrap();
    let source_calls = std::fs::read_to_string(temp.path().join("source-calls.html")).unwrap();
    assert_eq!(overview.matches("href=\"source-calls.html#ref-").count(), 2);
    assert_eq!(endpoint.matches("href=\"source-calls.html#ref-").count(), 2);
    assert!(source_calls.contains("Exact Kotlin source site"));
    assert!(source_calls.contains("Retained Kotlin declaration"));
    assert!(source_calls.contains("Retained target body"));
    assert!(source_calls.contains("compiler"));

    let mut bodyless_evidence = evidence;
    let bodyless_target = bodyless_evidence.observations.get_mut("api-pick").unwrap();
    bodyless_target
        .normalized
        .as_object_mut()
        .unwrap()
        .remove("documentation");
    bodyless_target.digest = digest(&bodyless_target.normalized).unwrap();
    let bodyless_checked = kotlin_check(bodyless_evidence);
    let bodyless_projection = project(&bodyless_checked, &[selection]).unwrap();
    let bodyless_graph = bodyless_projection.source_call_graph.as_ref().unwrap();
    assert_eq!(bodyless_graph.nodes.len(), 1);
    let bodyless_caller = bodyless_graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    assert_eq!(bodyless_caller.calls.len(), 2);
    assert!(bodyless_caller.calls.iter().all(|edge| {
        edge.target_declaration.as_deref() == Some("api-pick")
            && edge.target_node.is_none()
            && edge.status == "CALL_TARGET_BODY_NOT_CAPTURED"
            && edge
                .frontiers
                .iter()
                .any(|gap| gap.code == "CALL_TARGET_BODY_NOT_CAPTURED")
    }));
    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &bodyless_projection).unwrap();

    let mut missing_target_source = kotlin_retained_call_sites_evidence();
    add_exact_kotlin_pick_target(
        &mut missing_target_source,
        "api-pick",
        ":/main",
        "fun pick(value: String): String = value",
        json!([]),
    );
    let source_id = missing_target_source.observations["api-pick"].source_ids[0].clone();
    missing_target_source.sources.remove(&source_id);
    let missing_source_graph = expanded_kotlin_projection(missing_target_source)
        .source_call_graph
        .unwrap();
    let missing_source_edges = &missing_source_graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap()
        .calls;
    assert!(missing_source_edges.iter().all(|edge| {
        edge.target_declaration.as_deref() == Some("api-pick")
            && edge.target_node.is_none()
            && edge.status == "BODY_UNAVAILABLE"
            && edge
                .frontiers
                .iter()
                .any(|gap| gap.code == "CALL_TARGET_BODY_UNAVAILABLE")
    }));
}

fn evidence_symbol_for_kotlin_pick() -> String {
    "callable:parity/Api.pick#jvm:(Ljava/lang/String;)Ljava/lang/String;".into()
}

fn expanded_kotlin_projection(evidence: ServiceEvidence) -> BundleProjection {
    let checked = kotlin_check(evidence);
    let mut selection = kotlin_selection("answer-next", "answer-next");
    selection.expand_source_calls = true;
    project(&checked, &[selection]).unwrap()
}

fn kotlin_pick_documentation(events: serde_json::Value) -> serde_json::Value {
    json!({
        "schema":"codeclew-kotlin-documentation-flow/1.0",
        "authority":"KOTLIN_PSI_WITH_K2_CALL_TARGETS",
        "events":events,
        "boundaries":[]
    })
}

fn add_exact_kotlin_pick_target(
    evidence: &mut ServiceEvidence,
    id: &str,
    scope: &str,
    body: &str,
    events: serde_json::Value,
) -> String {
    add_kotlin_target(
        evidence,
        id,
        "parity/Api.pick",
        "(Ljava/lang/String;)Ljava/lang/String;",
        scope,
        body,
        Some(kotlin_pick_documentation(events)),
    )
}

#[test]
fn kotlin_exact_target_resolution_and_outline_gaps_preserve_admitted_bodies() {
    let body = "fun pick(value: String): String = value";
    let mut overloaded = kotlin_retained_call_sites_evidence();
    add_exact_kotlin_pick_target(&mut overloaded, "api-pick", ":/main", body, json!([]));
    add_kotlin_target(
        &mut overloaded,
        "api-pick-int",
        "parity/Api.pick",
        "(I)Ljava/lang/String;",
        ":/main",
        "fun pick(value: Int): String = value.toString()",
        Some(kotlin_pick_documentation(json!([]))),
    );
    let projection = expanded_kotlin_projection(overloaded);
    let graph = projection.source_call_graph.as_ref().unwrap();
    let caller = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    assert_eq!(caller.calls.len(), 2);
    assert!(caller.calls.iter().all(|edge| {
        edge.target_declaration.as_deref() == Some("api-pick")
            && edge.status == "RETAINED_DECLARED_BODY"
    }));
    assert!(
        graph
            .nodes
            .values()
            .any(|node| node.callable.declaration_id == "api-pick")
    );
    assert!(
        !graph
            .nodes
            .values()
            .any(|node| node.callable.declaration_id == "api-pick-int")
    );

    let missing = expanded_kotlin_projection(kotlin_retained_call_sites_evidence());
    let missing_edges = missing
        .source_call_graph
        .as_ref()
        .unwrap()
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap()
        .calls
        .clone();
    assert!(missing_edges.iter().all(|edge| {
        edge.target_declaration.is_none()
            && edge.target_node.is_none()
            && edge.status == "CALL_TARGET_BODY_NOT_CAPTURED"
    }));

    let mut wrong_scope = kotlin_retained_call_sites_evidence();
    add_exact_kotlin_pick_target(&mut wrong_scope, "api-pick", ":/test", body, json!([]));
    let scope_graph = expanded_kotlin_projection(wrong_scope)
        .source_call_graph
        .unwrap();
    assert!(
        scope_graph
            .nodes
            .values()
            .find(|node| node.callable.declaration_id == "answer-next")
            .unwrap()
            .calls
            .iter()
            .all(|edge| edge.status == "CALL_TARGET_SCOPE_MISMATCH")
    );

    let mut duplicate = kotlin_retained_call_sites_evidence();
    add_exact_kotlin_pick_target(&mut duplicate, "api-pick", ":/main", body, json!([]));
    add_exact_kotlin_pick_target(
        &mut duplicate,
        "api-pick-duplicate",
        ":/main",
        body,
        json!([]),
    );
    let duplicate_graph = expanded_kotlin_projection(duplicate)
        .source_call_graph
        .unwrap();
    assert!(
        duplicate_graph
            .nodes
            .values()
            .find(|node| node.callable.declaration_id == "answer-next")
            .unwrap()
            .calls
            .iter()
            .all(|edge| edge.status == "CALL_TARGET_AMBIGUOUS")
    );

    let mut unsupported_outline = kotlin_retained_call_sites_evidence();
    add_exact_kotlin_pick_target(
        &mut unsupported_outline,
        "api-pick",
        ":/main",
        "fun pick(value: String): String {\n  // retained body\n  return value\n}",
        json!([]),
    );
    set_kotlin_outline_events(
        &mut unsupported_outline,
        "api-pick",
        vec![json!({"kind":"LOOP"})],
    );
    let outline_projection = expanded_kotlin_projection(unsupported_outline);
    let graph = outline_projection.source_call_graph.as_ref().unwrap();
    let caller = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    let target_id = caller.calls[0].target_node.as_ref().unwrap();
    let target = &graph.nodes[target_id];
    assert_eq!(caller.calls[0].status, "RETAINED_DECLARED_BODY");
    assert!(
        target
            .callable
            .source_outline
            .as_ref()
            .unwrap()
            .gaps
            .iter()
            .any(|gap| { gap.code == "KOTLIN_SOURCE_OUTLINE_CONTROL_FORM_UNSUPPORTED" })
    );
    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &outline_projection).unwrap();
}

#[test]
fn kotlin_call_graph_enqueues_nested_cycles_and_exposes_each_fixed_frontier() {
    use std::collections::VecDeque;

    let mut evidence = kotlin_retained_call_sites_evidence();
    let recursive_body = "fun pick(value: String): String {\n  // retained\n  val first = api.pick(\"x\"); val second = api.pick(\"x\"); return value\n}";
    add_exact_kotlin_pick_target(
        &mut evidence,
        "api-pick",
        ":/main",
        recursive_body,
        json!([]),
    );
    add_kotlin_exact_call_relations(&mut evidence, "api-pick");
    let checked = kotlin_check(evidence);
    let mut selection = kotlin_selection("answer-next", "answer-next");
    selection.expand_source_calls = true;
    let projection = project(&checked, &[selection]).unwrap();
    let graph = projection.source_call_graph.as_ref().unwrap();
    assert_eq!(graph.nodes.len(), 2);
    let target = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "api-pick")
        .unwrap();
    assert_eq!(
        target.calls.len(),
        2,
        "{:?}",
        target.callable.retained_call_sites
    );
    assert!(target.calls.iter().all(|edge| {
        edge.target_node.as_deref() == Some(target.id.as_str())
            && edge
                .frontiers
                .iter()
                .any(|gap| gap.code == "SOURCE_CALL_CYCLE_FRONTIER")
    }));
    assert!(graph.process_links.is_empty());

    let root = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    let mut template = root.calls[0].clone();
    template.target_declaration = None;
    template.target_node = None;
    template.status = "UNEXPANDED".into();
    template.frontiers.clear();
    let evidence = &checked.services["sample"];
    let mut empty_graph = graph.clone();
    empty_graph.nodes.clear();

    let mut depth_edge = template.clone();
    let mut depth_graph = empty_graph.clone();
    let mut depth_bodies = 0;
    let mut depth_bytes = 0;
    let mut depth_pending = VecDeque::new();
    super::super::linked::expand_kotlin_edge(
        evidence,
        &mut depth_edge,
        usize::MAX,
        &mut depth_graph,
        &mut depth_bodies,
        &mut depth_bytes,
        &mut depth_pending,
    )
    .unwrap();
    assert_eq!(depth_edge.status, "DEPTH_FRONTIER");

    let mut body_edge = template.clone();
    let mut body_graph = empty_graph.clone();
    let mut body_count = usize::MAX;
    let mut body_bytes = 0;
    let mut body_pending = VecDeque::new();
    super::super::linked::expand_kotlin_edge(
        evidence,
        &mut body_edge,
        0,
        &mut body_graph,
        &mut body_count,
        &mut body_bytes,
        &mut body_pending,
    )
    .unwrap();
    assert_eq!(body_edge.status, "BODY_BUDGET_FRONTIER");

    let mut bytes_edge = template;
    let mut bytes_graph = empty_graph;
    let mut bytes_bodies = 0;
    let mut bytes_count = usize::MAX;
    let mut bytes_pending = VecDeque::new();
    super::super::linked::expand_kotlin_edge(
        evidence,
        &mut bytes_edge,
        0,
        &mut bytes_graph,
        &mut bytes_bodies,
        &mut bytes_count,
        &mut bytes_pending,
    )
    .unwrap();
    assert_eq!(bytes_edge.status, "SOURCE_BYTES_FRONTIER");
}

fn assert_source_call_preflight_rejects_before_output(projection: &BundleProjection) {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("pages");
    assert!(super::super::publish::write(&output, "snapshot", projection).is_err());
    assert!(!output.exists());
}

#[test]
fn kotlin_source_call_preflight_rejects_mixed_edges_owner_citation_and_missing_root() {
    let mut evidence = kotlin_retained_call_sites_evidence();
    add_exact_kotlin_pick_target(
        &mut evidence,
        "api-pick",
        ":/main",
        "fun pick(value: String): String = value",
        json!([]),
    );
    add_kotlin_target(
        &mut evidence,
        "spare-root",
        "parity/Api.spare",
        "()Ljava/lang/String;",
        ":/main",
        "fun spare(): String = \"unused\"",
        Some(kotlin_pick_documentation(json!([]))),
    );
    let checked = kotlin_check(evidence);
    let mut selection = kotlin_selection("answer-next", "answer-next");
    selection.expand_source_calls = true;
    selection.wiring_declaration = Some("spare-root".into());
    let baseline = project(&checked, &[selection]).unwrap();
    let root_id = baseline
        .source_call_graph
        .as_ref()
        .unwrap()
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "spare-root")
        .unwrap()
        .id
        .clone();

    let mut mixed_edge = baseline.clone();
    let caller_id = mixed_edge
        .source_call_graph
        .as_ref()
        .unwrap()
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap()
        .id
        .clone();
    mixed_edge
        .source_call_graph
        .as_mut()
        .unwrap()
        .nodes
        .get_mut(&caller_id)
        .unwrap()
        .calls[0]
        .occurrence_path = Some("body/0/call/0".into());
    assert_source_call_preflight_rejects_before_output(&mixed_edge);

    let mut bad_owner_citation = baseline.clone();
    let root = bad_owner_citation
        .source_call_graph
        .as_mut()
        .unwrap()
        .nodes
        .get_mut(&caller_id)
        .unwrap();
    let citation_id = root.callable.citation_id.clone().unwrap();
    root.citations.get_mut(&citation_id).unwrap().start_byte = 1;
    assert_source_call_preflight_rejects_before_output(&bad_owner_citation);

    let mut missing_root = baseline;
    missing_root
        .source_call_graph
        .as_mut()
        .unwrap()
        .nodes
        .remove(&root_id);
    assert_source_call_preflight_rejects_before_output(&missing_root);
}

fn assert_kotlin_callsite_gap(evidence: ServiceEvidence, expected: &str) {
    let checked = kotlin_check(evidence);
    let projection = project(&checked, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
    let retained = projection.pages[0]
        .endpoint
        .retained_call_sites
        .as_ref()
        .unwrap();
    assert!(retained.sites.is_empty(), "{expected}");
    assert_eq!(retained.gaps.len(), 1, "{expected}");
    assert_eq!(retained.gaps[0].code, expected, "{expected}");
    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let endpoint = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.html")).unwrap();
    assert!(endpoint.contains("Retained exact call sites"), "{expected}");
    assert!(
        endpoint.contains(&expected.replace('_', "&#95;")),
        "{expected}"
    );
    assert!(!endpoint.contains("<pre>api.pick"), "{expected}");
    if expected == "KOTLIN_RETAINED_CALL_SITES_NOT_PROVEN" {
        assert!(
            endpoint.contains("absence does not establish that this declaration has no calls"),
            "{expected}"
        );
    }
}

fn assert_expanded_kotlin_rejected_callsite_keeps_raw_gap(evidence: ServiceEvidence) {
    let checked = kotlin_check(evidence);
    let mut selection = kotlin_selection("answer-next", "answer-next");
    selection.expand_source_calls = true;
    let projection = project(&checked, &[selection]).unwrap();
    let graph = projection.source_call_graph.as_ref().unwrap();
    let caller = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    assert!(caller.calls.is_empty());
    assert!(caller.observations.contains_key("call-relation-0"));
    assert!(
        caller
            .callable
            .retained_call_sites
            .as_ref()
            .unwrap()
            .gaps
            .iter()
            .any(|gap| gap.code == "KOTLIN_RETAINED_CALL_SITES_REJECTED")
    );
    assert!(
        !caller
            .callable
            .gaps
            .iter()
            .any(|gap| gap.code == "KOTLIN_SOURCE_CALL_GRAPH_UNAVAILABLE")
    );
    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let source_calls = std::fs::read_to_string(temp.path().join("source-calls.html")).unwrap();
    assert!(source_calls.contains("KOTLIN&#95;RETAINED&#95;CALL&#95;SITES&#95;REJECTED"));
}

#[test]
fn kotlin_retained_call_sites_reject_unbound_evidence_and_publish_gaps() {
    use super::super::source::call_sites;

    const REJECTED: &str = "KOTLIN_RETAINED_CALL_SITES_REJECTED";
    const NOT_PROVEN: &str = "KOTLIN_RETAINED_CALL_SITES_NOT_PROVEN";
    const CONFLICT: &str = "KOTLIN_RETAINED_CALL_SITE_CONFLICT";

    let mut no_data = kotlin_retained_call_sites_evidence();
    no_data
        .observations
        .retain(|id, _| !id.starts_with("call-relation-"));
    no_data
        .sources
        .retain(|id, _| !id.starts_with("call-source-"));
    assert_kotlin_callsite_gap(no_data, NOT_PROVEN);

    let mut wrong_owner = kotlin_retained_call_sites_evidence();
    wrong_owner
        .observations
        .get_mut("call-relation-0")
        .unwrap()
        .symbol = "callable:parity/Other.call#jvm:()V".into();
    assert_kotlin_callsite_gap(wrong_owner, REJECTED);

    let mut wrong_scope = kotlin_retained_call_sites_evidence();
    for id in ["call-relation-0", "call-relation-1"] {
        let relation = wrong_scope.observations.get_mut(id).unwrap();
        relation.normalized["scope"] = json!(":/test");
        relation.digest = digest(&relation.normalized).unwrap();
    }
    assert_kotlin_callsite_gap(wrong_scope, NOT_PROVEN);

    let mut wrong_identity = kotlin_retained_call_sites_evidence();
    let relation = wrong_identity
        .observations
        .get_mut("call-relation-0")
        .unwrap();
    relation.normalized["sourceIdentity"] = json!("callable:parity/Other.call#jvm:()V");
    relation.digest = digest(&relation.normalized).unwrap();
    assert_kotlin_callsite_gap(wrong_identity, REJECTED);

    let mut wrong_target = kotlin_retained_call_sites_evidence();
    let relation = wrong_target
        .observations
        .get_mut("call-relation-0")
        .unwrap();
    relation.normalized["targetIdentity"] = json!("callable:parity/Api.other#jvm:()V");
    relation.digest = digest(&relation.normalized).unwrap();
    assert_kotlin_callsite_gap(wrong_target, REJECTED);

    let mut bad_digest = kotlin_retained_call_sites_evidence();
    bad_digest
        .observations
        .get_mut("call-relation-0")
        .unwrap()
        .digest = "sha256:invalid".into();
    assert_kotlin_callsite_gap(bad_digest.clone(), REJECTED);
    assert_expanded_kotlin_rejected_callsite_keeps_raw_gap(bad_digest);

    let mut missing_source = kotlin_retained_call_sites_evidence();
    let source_id = missing_source.observations["call-relation-0"].source_ids[0].clone();
    missing_source.sources.remove(&source_id);
    assert_expanded_kotlin_rejected_callsite_keeps_raw_gap(missing_source);

    let mut bad_evidence_binding = kotlin_retained_call_sites_evidence();
    let relation = bad_evidence_binding
        .observations
        .get_mut("call-relation-0")
        .unwrap();
    relation.normalized["evidenceBinding"] = json!(format!("sha256:{}", "e".repeat(64)));
    relation.digest = digest(&relation.normalized).unwrap();
    assert_kotlin_callsite_gap(bad_evidence_binding, REJECTED);

    let mut bad_source_binding = kotlin_retained_call_sites_evidence();
    let relation = bad_source_binding
        .observations
        .get_mut("call-relation-0")
        .unwrap();
    relation.normalized["callSite"]["sourceId"] = json!("missing-source");
    relation.digest = digest(&relation.normalized).unwrap();
    assert_kotlin_callsite_gap(bad_source_binding, REJECTED);

    let mut bad_text_binding = kotlin_retained_call_sites_evidence();
    let relation = bad_text_binding
        .observations
        .get_mut("call-relation-0")
        .unwrap();
    let source_id = relation.source_ids[0].clone();
    let changed_text = "api.pock(\"x\")";
    let source = bad_text_binding.sources.get_mut(&source_id).unwrap();
    source.text = changed_text.into();
    source.text_digest = crate::canonical::hash_bytes(changed_text.as_bytes());
    let relation = bad_text_binding
        .observations
        .get_mut("call-relation-0")
        .unwrap();
    relation.digest = digest(&relation.normalized).unwrap();
    assert_kotlin_callsite_gap(bad_text_binding, REJECTED);

    let mut bad_range = kotlin_retained_call_sites_evidence();
    let relation = bad_range.observations.get_mut("call-relation-0").unwrap();
    relation.normalized["callSite"]["byteEnd"] =
        json!(relation.normalized["callSite"]["byteEnd"].as_u64().unwrap() + 1);
    relation.digest = digest(&relation.normalized).unwrap();
    assert_kotlin_callsite_gap(bad_range, REJECTED);

    let mut duplicate_occurrence = kotlin_retained_call_sites_evidence();
    let mut duplicate = duplicate_occurrence.observations["call-relation-0"].clone();
    duplicate.id = "call-relation-duplicate".into();
    duplicate.digest = digest(&duplicate.normalized).unwrap();
    duplicate_occurrence
        .observations
        .insert(duplicate.id.clone(), duplicate);
    assert_kotlin_callsite_gap(duplicate_occurrence, CONFLICT);

    let mut legacy_projection = project(
        &kotlin_check(kotlin_retained_call_sites_evidence()),
        &[kotlin_selection("answer-next", "answer-next")],
    )
    .unwrap();
    legacy_projection.pages[0].endpoint.retained_call_sites = None;
    legacy_projection.pages[0]
        .observations
        .remove("answer-next");
    assert!(
        call_sites::validate_page(
            &legacy_projection.pages[0],
            &legacy_projection.pages[0].endpoint
        )
        .is_ok()
    );
}

fn assert_kotlin_outline_gap(evidence: ServiceEvidence, expected: &str) {
    let checked = kotlin_check(evidence);
    let projection = project(&checked, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
    let outline = projection.pages[0]
        .endpoint
        .source_outline
        .as_ref()
        .unwrap();
    assert!(outline.tree.is_none(), "{expected}");
    assert_eq!(outline.gaps.len(), 1, "{expected}");
    assert_eq!(outline.gaps[0].code, expected);
    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let endpoint = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.html")).unwrap();
    let escaped_code = expected.replace('_', "&#95;");
    assert!(endpoint.contains(&escaped_code), "{expected}");
    assert!(!endpoint.contains("<pre>Entry:"), "{expected}");
}

#[test]
fn kotlin_source_outline_keeps_same_line_call_citations_and_vetoes_incomplete_forms() {
    let mut call_evidence = evidence();
    let retained =
        "fun next(value: Int): Int {\r\n  // π\r\n  client.first(); client.second()\r\n}";
    add_kotlin_declaration(&mut call_evidence, "answer-next", "FUNCTION", retained);
    add_kotlin_return_outline(&mut call_evidence, "answer-next");
    let flow_ids = set_kotlin_outline_events(
        &mut call_evidence,
        "answer-next",
        vec![
            json!({"kind":"CALL","resolution":"COMPILER_EXACT","target":"<script>&"}),
            json!({"kind":"CALL","resolution":"COMPILER_EXACT","target":"callable:example/Client.second#jvm:()V"}),
        ],
    );
    let checked = kotlin_check(call_evidence);
    let projection = project(&checked, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
    let page = &projection.pages[0];
    let outline = page.endpoint.source_outline.as_ref().unwrap();
    assert_eq!(outline.events.len(), 2);
    assert_eq!(outline.events[0].observation_id, flow_ids[0]);
    assert_eq!(outline.events[1].observation_id, flow_ids[1]);
    assert_eq!(
        (outline.events[0].ordinal, outline.events[1].ordinal),
        (0, 1)
    );
    assert_ne!(outline.events[0].citation_id, outline.events[1].citation_id);
    let first_citation = &page.citations[&outline.events[0].citation_id];
    let second_citation = &page.citations[&outline.events[1].citation_id];
    assert_eq!(first_citation.file, second_citation.file);
    assert_eq!(
        (first_citation.start_line, first_citation.end_line),
        (25, 25)
    );
    assert_eq!(
        (second_citation.start_line, second_citation.end_line),
        (25, 25)
    );
    let first_source = &page.sources[&first_citation.source_id];
    let second_source = &page.sources[&second_citation.source_id];
    assert_eq!(first_source.text, "  client.first(); client.second()");
    assert_eq!(first_source.text, second_source.text);
    assert!(first_source.occurrence.is_none());
    assert!(second_source.occurrence.is_none());
    assert_eq!(
        outline.tree.as_deref(),
        Some(concat!(
            "Entry: parity/Answer.next\n",
            "CALL target metadata: <script>&\n",
            "CALL target metadata: callable:example/Client.second#jvm:()V\n"
        ))
    );
    assert!(page.endpoint.steps.is_empty());
    assert!(page.endpoint.control_flow.is_none());
    assert_eq!(page.handoff.status, "DECLARATION_ONLY");

    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let endpoint = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.html")).unwrap();
    assert!(endpoint.contains("&lt;script&gt;&amp;"));
    assert!(!endpoint.contains("<script>"));

    let mut missing_kind = evidence();
    add_kotlin_declaration(
        &mut missing_kind,
        "answer-next",
        "FUNCTION",
        "fun next() {\r\n  // π\r\n  return\r\n}",
    );
    add_kotlin_return_outline(&mut missing_kind, "answer-next");
    set_kotlin_outline_events(&mut missing_kind, "answer-next", vec![json!({})]);
    assert_kotlin_outline_gap(missing_kind, "KOTLIN_SOURCE_OUTLINE_EVENT_KIND_UNAVAILABLE");

    let mut unsupported = kotlin_answer_next_outline_evidence();
    set_kotlin_outline_events(
        &mut unsupported,
        "answer-next",
        vec![json!({"kind":"LOOP"})],
    );
    assert_kotlin_outline_gap(
        unsupported,
        "KOTLIN_SOURCE_OUTLINE_CONTROL_FORM_UNSUPPORTED",
    );

    let mut malformed = kotlin_answer_next_outline_evidence();
    set_kotlin_outline_events(
        &mut malformed,
        "answer-next",
        vec![json!({"kind":"IF","condition":"untrusted <condition>"})],
    );
    assert_kotlin_outline_gap(malformed, "KOTLIN_SOURCE_OUTLINE_MALFORMED_STRUCTURE");

    let mut absent_boundaries = kotlin_answer_next_outline_evidence();
    set_kotlin_outline_events(
        &mut absent_boundaries,
        "answer-next",
        vec![json!({"kind":"RETURN"})],
    );
    absent_boundaries
        .observations
        .get_mut("answer-next")
        .unwrap()
        .normalized["documentation"]
        .as_object_mut()
        .unwrap()
        .remove("boundaries");
    refresh_observation_digest(&mut absent_boundaries, "answer-next");
    assert_kotlin_outline_gap(
        absent_boundaries,
        "KOTLIN_SOURCE_OUTLINE_BOUNDARIES_UNAVAILABLE",
    );

    let mut missing_flow = kotlin_answer_next_outline_evidence();
    let ids = set_kotlin_outline_events(
        &mut missing_flow,
        "answer-next",
        vec![json!({"kind":"RETURN"})],
    );
    missing_flow.observations.remove(&ids[0]);
    assert_kotlin_outline_gap(missing_flow, "KOTLIN_SOURCE_OUTLINE_EVENT_SET_MISMATCH");

    let mut extra_flow = kotlin_answer_next_outline_evidence();
    let ids = set_kotlin_outline_events(
        &mut extra_flow,
        "answer-next",
        vec![json!({"kind":"RETURN"})],
    );
    let mut extra = extra_flow.observations[&ids[0]].clone();
    extra.id = "flow-answer-next-extra".into();
    extra.normalized["ordinal"] = json!(1);
    extra.digest = digest(&extra.normalized).unwrap();
    extra_flow.observations.insert(extra.id.clone(), extra);
    assert_kotlin_outline_gap(extra_flow, "KOTLIN_SOURCE_OUTLINE_EVENT_SET_MISMATCH");

    let mut wrong_scope = kotlin_answer_next_outline_evidence();
    let ids = set_kotlin_outline_events(
        &mut wrong_scope,
        "answer-next",
        vec![json!({"kind":"RETURN"})],
    );
    let flow = wrong_scope.observations.get_mut(&ids[0]).unwrap();
    flow.normalized["scope"] = json!(":/other");
    flow.digest = digest(&flow.normalized).unwrap();
    assert_kotlin_outline_gap(wrong_scope, "KOTLIN_SOURCE_OUTLINE_EVENT_SET_MISMATCH");

    let mut bad_digest = kotlin_answer_next_outline_evidence();
    let ids = set_kotlin_outline_events(
        &mut bad_digest,
        "answer-next",
        vec![json!({"kind":"RETURN"})],
    );
    bad_digest.observations.get_mut(&ids[0]).unwrap().digest = "sha256:bad".into();
    assert_kotlin_outline_gap(bad_digest, "KOTLIN_SOURCE_OUTLINE_EVENT_BINDING_MISMATCH");

    let mut bad_source = kotlin_answer_next_outline_evidence();
    let ids = set_kotlin_outline_events(
        &mut bad_source,
        "answer-next",
        vec![json!({"kind":"RETURN"})],
    );
    let source_id = bad_source.observations[&ids[0]].source_ids[0].clone();
    let source = bad_source.sources.get_mut(&source_id).unwrap();
    source.evidence_digest = format!("sha256:{}", "c".repeat(64));
    assert_kotlin_outline_gap(bad_source, "KOTLIN_SOURCE_OUTLINE_EVENT_SOURCE_MISMATCH");

    let mut duplicate_ordinal = kotlin_answer_next_outline_evidence();
    let ids = set_kotlin_outline_events(
        &mut duplicate_ordinal,
        "answer-next",
        vec![
            json!({"kind":"CALL","resolution":"COMPILER_EXACT","target":"a"}),
            json!({"kind":"CALL","resolution":"COMPILER_EXACT","target":"b"}),
        ],
    );
    let first_event = duplicate_ordinal.observations[&ids[0]].normalized.clone();
    let duplicate = duplicate_ordinal.observations.get_mut(&ids[1]).unwrap();
    duplicate.normalized = first_event;
    duplicate.digest = digest(&duplicate.normalized).unwrap();
    assert_kotlin_outline_gap(duplicate_ordinal, "KOTLIN_SOURCE_OUTLINE_ORDINAL_MISMATCH");
}

#[test]
fn kotlin_compiler_control_flow_binds_exact_graph_and_renders_cited_nodes() {
    let (checked, symbol, cfg_id, retained) = kotlin_control_flow_check();
    let comment_start = retained.find("// π").unwrap();
    let projection = project(&checked, &[kotlin_selection("calculate", "calculate")]).unwrap();

    assert_eq!(projection.schema, CONTROL_FLOW_SCHEMA);
    let page = &projection.pages[0];
    assert_eq!(page.title, format!("Selected functions: {symbol}"));
    assert_eq!(
        page.projection_kind,
        Some(ProjectionKind::CompilerControlFlow)
    );
    let panel = page.endpoint.control_flow.as_ref().unwrap();
    assert_eq!(panel.owner_key.service, "sample");
    assert_eq!(panel.owner_key.scope, ":/main");
    assert_eq!(panel.owner_key.symbol, symbol);
    assert_eq!(panel.graph_observation_id, cfg_id);
    assert_eq!(panel.nodes.len(), 3);
    assert_eq!(panel.edges.len(), 2);
    assert_eq!(panel.nodes[0].node_id, 0);
    assert_eq!(panel.nodes[0].citation_id, None);
    let comment_citation = &page.citations[panel.nodes[1].citation_id.as_ref().unwrap()];
    assert_eq!(comment_citation.start_byte, comment_start);
    assert_eq!(
        &page.sources[&comment_citation.source_id].text
            [comment_citation.start_byte..comment_citation.end_byte],
        "// π"
    );
    let return_citation = &page.citations[panel.nodes[2].citation_id.as_ref().unwrap()];
    assert_eq!(
        &page.sources[&return_citation.source_id].text
            [return_citation.start_byte..return_citation.end_byte],
        "return value.trim()"
    );
    assert!(page.observations.contains_key(&cfg_id));

    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let catalogue: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("catalogue.json")).unwrap())
            .unwrap();
    let declaration_row = catalogue["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "declarations-kotlin-page")
        .unwrap();
    assert_eq!(declaration_row["title"].as_str(), Some(page.title.as_str()));
    assert_eq!(
        declaration_row["kind"].as_str(),
        Some("Selected declarations")
    );
    let overview = std::fs::read_to_string(temp.path().join("kotlin-page-overview.html")).unwrap();
    assert!(overview.contains("Nodes: 3. Edges: 2."));
    assert!(!overview.contains("Retained compiler control-flow nodes and outgoing edges"));
    let endpoint = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.html")).unwrap();
    assert!(endpoint.contains("Retained compiler control-flow nodes and outgoing edges"));
    assert!(endpoint.contains("Node IDs are identifiers, not execution order."));
    assert!(endpoint.contains("labels are retained compiler metadata"));
    assert!(endpoint.contains("&lt;entry&gt;&#123;π&#125;"));
    let mdx = std::fs::read_to_string(temp.path().join("kotlin-page-endpoint.mdx")).unwrap();
    assert!(mdx.contains("&lt;entry&gt;&#123;π&#125;"));
    assert!(!mdx.contains("<entry>{π}"));
}

#[test]
fn local_cfg_boundaries_veto_only_matching_owner_scope_and_file() {
    let run = |owner: Option<&str>, file: Option<&str>, code: &str| {
        let (mut checked, _, _, _) = kotlin_control_flow_check();
        let boundary_id = add_local_cfg_boundary(
            checked.services.get_mut("sample").unwrap(),
            owner,
            file,
            ":/main",
            code,
        );
        let projection = project(&checked, &[kotlin_selection("calculate", "calculate")]).unwrap();
        (projection, boundary_id)
    };

    for (owner, file, code) in [
        (None, None, "SCOPE_UNKNOWN"),
        (
            None,
            Some("src/main/kotlin/example/Sample.kt"),
            "FILE_UNKNOWN",
        ),
        (
            Some("callable:example/Sample.calculate#jvm:(Ljava/lang/String;)Ljava/lang/String;"),
            None,
            "OWNER_UNKNOWN",
        ),
    ] {
        let (projection, boundary_id) = run(owner, file, code);
        let page = &projection.pages[0];
        assert_eq!(projection.schema, DECLARATION_SCHEMA);
        assert_eq!(page.projection_kind, Some(ProjectionKind::DeclarationOnly));
        assert!(page.endpoint.control_flow.is_none());
        assert!(page.observations.contains_key(&boundary_id));
        assert!(page.endpoint.gaps.iter().any(|gap| {
            gap.code == "KOTLIN_CONTROL_FLOW_UNAVAILABLE" && gap.detail.contains(code)
        }));
    }

    let (unrelated_file, _) = run(
        None,
        Some("src/main/kotlin/other/Other.kt"),
        "OTHER_FILE_UNKNOWN",
    );
    assert_eq!(unrelated_file.schema, CONTROL_FLOW_SCHEMA);
    assert!(unrelated_file.pages[0].endpoint.control_flow.is_some());
}

#[test]
fn kotlin_control_flow_preflight_rejects_binding_and_schema_mismatch_before_output() {
    let (checked, _, _, _) = kotlin_control_flow_check();
    let build = || project(&checked, &[kotlin_selection("calculate", "calculate")]).unwrap();
    let temp = tempfile::tempdir().unwrap();

    let mut wrong_scope = build();
    wrong_scope.pages[0]
        .endpoint
        .control_flow
        .as_mut()
        .unwrap()
        .owner_key
        .scope
        .push_str("/wrong");
    let wrong_scope_output = temp.path().join("wrong-scope");
    assert!(super::super::publish::write(&wrong_scope_output, "snapshot", &wrong_scope).is_err());
    assert!(!wrong_scope_output.exists());

    let mut wrong_citation = build();
    let citation_id = wrong_citation.pages[0]
        .endpoint
        .control_flow
        .as_ref()
        .unwrap()
        .nodes[1]
        .citation_id
        .clone()
        .unwrap();
    wrong_citation.pages[0]
        .citations
        .get_mut(&citation_id)
        .unwrap()
        .start_byte += 1;
    let wrong_citation_output = temp.path().join("wrong-citation");
    assert!(
        super::super::publish::write(&wrong_citation_output, "snapshot", &wrong_citation).is_err()
    );
    assert!(!wrong_citation_output.exists());

    let mut wrong_schema = build();
    wrong_schema.schema = DECLARATION_SCHEMA.into();
    let wrong_schema_output = temp.path().join("wrong-schema");
    assert!(super::super::publish::write(&wrong_schema_output, "snapshot", &wrong_schema).is_err());
    assert!(!wrong_schema_output.exists());
}

#[test]
fn kotlin_additional_only_control_flow_uses_declaration_sources_view() {
    use crate::thread_flow_cfg::{LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNodeRole};

    let mut evidence = evidence();
    add_kotlin_declaration(&mut evidence, "render", "FUNCTION", "fun render() = 1");
    let additional = "fun additional() = 2";
    add_kotlin_declaration(&mut evidence, "additional", "FUNCTION", additional);
    add_local_cfg(
        &mut evidence,
        "additional",
        "additional-graph",
        &[
            LocalCfgNodeRole::Entry,
            LocalCfgNodeRole::LoopCondition,
            LocalCfgNodeRole::Return,
        ],
        &[None, Some((0, additional.len())), None],
        vec![
            LocalCfgEdge {
                source_node_id: 0,
                target_node_id: 1,
                kind: LocalCfgEdgeKind::Next,
                label: None,
            },
            LocalCfgEdge {
                source_node_id: 1,
                target_node_id: 0,
                kind: LocalCfgEdgeKind::LoopBack,
                label: Some("loop <back>".into()),
            },
            LocalCfgEdge {
                source_node_id: 1,
                target_node_id: 2,
                kind: LocalCfgEdgeKind::False,
                label: None,
            },
        ],
    );
    let checked = kotlin_check(evidence);
    let mut selection = kotlin_selection("render", "render");
    selection.wiring_declaration = Some("additional".into());
    let mut expanded = selection.clone();
    expanded.expand_source_calls = true;
    let expanded_projection = project(&checked, &[expanded]).unwrap();
    let expanded_graph = expanded_projection.source_call_graph.as_ref().unwrap();
    assert!(
        expanded_graph
            .nodes
            .values()
            .any(|node| node.callable.declaration_id == "render")
    );
    assert!(
        expanded_graph
            .nodes
            .values()
            .any(|node| node.callable.declaration_id == "additional")
    );
    let mut expanded_state = selection.clone();
    expanded_state.expand_data_state = true;
    assert!(
        project(&checked, &[expanded_state])
            .unwrap_err()
            .message
            .contains("expandDataState is unavailable for declaration-only")
    );
    let projection = project(&checked, &[selection]).unwrap();

    assert_eq!(projection.schema, CONTROL_FLOW_SCHEMA);
    let page = &projection.pages[0];
    assert_eq!(
        page.projection_kind,
        Some(ProjectionKind::CompilerControlFlow)
    );
    assert!(page.endpoint.control_flow.is_none());
    assert!(page.worker.control_flow.is_none());
    assert!(page.wiring.as_ref().unwrap().control_flow.is_some());

    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    let fields = std::fs::read_to_string(temp.path().join("kotlin-page-fields-state.mdx")).unwrap();
    assert!(fields.contains("Additional retained declaration"));
    assert!(fields.contains("Compiler-provided local control flow"));
    assert!(fields.contains("Retained compiler control-flow nodes and outgoing edges"));
    assert!(fields.contains("LOOP_CONDITION"));
    assert!(fields.contains("LOOP_BACK"));
    assert!(fields.contains("loop &lt;back&gt;"));
    assert!(fields.contains("Node ID 1") || fields.contains("<td>1</td>"));
}

#[test]
fn source_behavior_java_empty_body_differs_from_declaration_only_kotlin() {
    let mut java = evidence();
    add_declaration(
        &mut java,
        "java-empty",
        "Example",
        "empty",
        "METHOD",
        "void empty() {}",
    );
    let checked = kotlin_check(java);
    let selection = kotlin_selection("java-empty", "java-empty");
    let java_projection = project(&checked, &[selection]).unwrap();
    assert_eq!(java_projection.schema, SCHEMA);
    assert_eq!(java_projection.pages[0].projection_kind, None);
    assert!(java_projection.pages[0].endpoint.steps.is_empty());
    assert!(java_projection.pages[0].endpoint.state.is_empty());

    let mut kotlin = evidence();
    add_kotlin_declaration(&mut kotlin, "kotlin-empty", "FUNCTION", "fun empty() {}");
    let checked = kotlin_check(kotlin);
    let selection = kotlin_selection("kotlin-empty", "kotlin-empty");
    let kotlin_projection = project(&checked, &[selection]).unwrap();
    assert_eq!(kotlin_projection.schema, DECLARATION_SCHEMA);
    assert_eq!(
        kotlin_projection.pages[0].projection_kind,
        Some(ProjectionKind::DeclarationOnly)
    );
    assert!(kotlin_projection.pages[0].endpoint.steps.is_empty());
    assert!(kotlin_projection.pages[0].endpoint.state.is_empty());
}

#[test]
fn declaration_schema_round_trips_and_missing_or_unknown_page_kind_fails_preflight() {
    let mut evidence = evidence();
    add_kotlin_declaration(&mut evidence, "render", "FUNCTION", "fun render() = 1");
    let checked = kotlin_check(evidence);
    let projection = project(&checked, &[kotlin_selection("render", "render")]).unwrap();
    let encoded = serde_json::to_value(&projection).unwrap();
    let decoded: BundleProjection = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded, projection);
    assert_eq!(encoded["pages"][0]["projectionKind"], "DECLARATION_ONLY");

    let temp = tempfile::tempdir().unwrap();
    let missing_output = temp.path().join("missing-kind");
    let mut missing_kind = encoded.clone();
    missing_kind["pages"][0]
        .as_object_mut()
        .unwrap()
        .remove("projectionKind");
    let missing_kind: BundleProjection = serde_json::from_value(missing_kind).unwrap();
    assert!(super::super::publish::write(&missing_output, "snapshot", &missing_kind).is_err());
    assert!(!missing_output.exists());

    let unknown_output = temp.path().join("unknown-kind");
    let mut unknown_kind = encoded;
    unknown_kind["pages"][0]["projectionKind"] = json!("UNKNOWN");
    assert!(serde_json::from_value::<BundleProjection>(unknown_kind).is_err());
    assert!(!unknown_output.exists());
}

#[test]
fn kotlin_without_source_occurrence_keeps_a_retained_text_line_span() {
    let mut evidence = evidence();
    let text = "fun render() = 1";
    add_kotlin_declaration(&mut evidence, "render", "FUNCTION", text);
    evidence
        .sources
        .get_mut("kotlin-render")
        .unwrap()
        .occurrence = None;
    let checked = kotlin_check(evidence);
    let projection = project(&checked, &[kotlin_selection("render", "render")]).unwrap();
    let page = &projection.pages[0];
    let citation = &page.citations[page.endpoint.citation_id.as_ref().unwrap()];
    let retained = &page.sources[&citation.source_id];
    assert!(retained.occurrence.is_none());
    assert_eq!(citation.start_line, retained.start_line);
    assert_eq!(citation.end_line, retained.end_line);
    assert_eq!(citation.start_byte, 0);
    assert_eq!(citation.end_byte, text.len());
}

#[test]
fn kotlin_function_admission_rejects_authority_identity_scope_and_source_mismatches() {
    let build = || {
        let mut evidence = evidence();
        add_kotlin_declaration(
            &mut evidence,
            "render",
            "FUNCTION",
            "fun render() = \"ready\"",
        );
        evidence
    };
    let reject = |mutate: fn(&mut ServiceEvidence), message: &str| {
        let mut evidence = build();
        mutate(&mut evidence);
        let checked = kotlin_check(evidence);
        let error = project(&checked, &[kotlin_selection("render", "render")]).unwrap_err();
        assert!(error.message.contains(message), "{}", error.message);
    };
    reject(
        |evidence| {
            evidence.observations.get_mut("render").unwrap().normalized["provider"] =
                json!("SYNTAX")
        },
        "lacks exact compiler authority",
    );
    reject(
        |evidence| evidence.observations.get_mut("render").unwrap().service = "other".into(),
        "service or observation identity",
    );
    reject(
        |evidence| evidence.observations.get_mut("render").unwrap().id = "other".into(),
        "observation ID does not match",
    );
    reject(
        |evidence| {
            evidence
                .observations
                .get_mut("render")
                .unwrap()
                .symbol
                .push_str("-other")
        },
        "symbol identity does not match",
    );
    reject(
        |evidence| {
            evidence.observations.get_mut("render").unwrap().normalized["scope"] = json!(":/test")
        },
        "scope does not match",
    );
    reject(
        |evidence| evidence.observations.get_mut("render").unwrap().digest = "wrong".into(),
        "normalized evidence digest is inconsistent",
    );
    reject(
        |evidence| evidence.sources.get_mut("kotlin-render").unwrap().end_line += 1,
        "valid retained compiler-bound source span",
    );
    reject(
        |evidence| {
            evidence
                .sources
                .get_mut("kotlin-render")
                .unwrap()
                .text_digest = "wrong".into()
        },
        "valid retained compiler-bound source span",
    );
    reject(
        |evidence| {
            evidence
                .observations
                .get_mut("render")
                .unwrap()
                .source_ids
                .clear()
        },
        "valid retained compiler-bound source span",
    );
    reject(
        |evidence| {
            evidence
                .observations
                .get_mut("render")
                .unwrap()
                .source_ids
                .push("kotlin-render".into())
        },
        "valid retained compiler-bound source span",
    );
}

#[test]
fn kotlin_symbol_identity_validates_jvm_signature_and_optional_descriptor() {
    let mut missing = evidence();
    add_kotlin_declaration(&mut missing, "render", "FUNCTION", "fun render() = 1");
    missing
        .observations
        .get_mut("render")
        .unwrap()
        .normalized
        .as_object_mut()
        .unwrap()
        .remove("jvmDescriptor");
    refresh_observation_digest(&mut missing, "render");
    assert!(
        project(
            &kotlin_check(missing),
            &[kotlin_selection("render", "render")]
        )
        .is_ok()
    );

    let invalid_descriptor = |descriptor: serde_json::Value, expected: &str| {
        let mut evidence = evidence();
        add_kotlin_declaration(&mut evidence, "render", "FUNCTION", "fun render() = 1");
        evidence.observations.get_mut("render").unwrap().normalized["jvmDescriptor"] = descriptor;
        refresh_observation_digest(&mut evidence, "render");
        let error = project(
            &kotlin_check(evidence),
            &[kotlin_selection("render", "render")],
        )
        .unwrap_err();
        assert!(error.message.contains(expected), "{}", error.message);
    };
    invalid_descriptor(json!(42), "JVM descriptor is not a string");
    invalid_descriptor(json!("()V"), "disagrees with the compiler symbol identity");

    let mut malformed = evidence();
    add_kotlin_declaration(&mut malformed, "render", "FUNCTION", "fun render() = 1");
    let observation = malformed.observations.get_mut("render").unwrap();
    observation.symbol = "callable:example/Sample.render#jvm:not-a-jvm-signature".into();
    observation.normalized["symbolIdentity"] = json!(observation.symbol);
    observation
        .normalized
        .as_object_mut()
        .unwrap()
        .remove("jvmDescriptor");
    observation.digest = digest(&observation.normalized).unwrap();
    let error = project(
        &kotlin_check(malformed),
        &[kotlin_selection("render", "render")],
    )
    .unwrap_err();
    assert!(error.message.contains("full symbol identity is invalid"));
}

#[test]
fn kotlin_function_admission_rejects_ambiguous_and_unsupported_descriptors() {
    let mut ambiguous = evidence();
    add_kotlin_declaration(&mut ambiguous, "render", "FUNCTION", "fun render() = 1");
    let mut duplicate = ambiguous.observations["render"].clone();
    duplicate.id = "duplicate-render".into();
    ambiguous
        .observations
        .insert(duplicate.id.clone(), duplicate);
    let checked = kotlin_check(ambiguous);
    let error = project(&checked, &[kotlin_selection("render", "render")]).unwrap_err();
    assert!(
        error
            .message
            .contains("ambiguous within its compilation scope")
    );

    for kind in ["CONSTRUCTOR", "PROPERTY", "MUTABLE_PROPERTY", "CLASS"] {
        let mut unsupported = evidence();
        add_kotlin_declaration(&mut unsupported, "unsupported", kind, "declaration");
        let checked = kotlin_check(unsupported);
        let error =
            project(&checked, &[kotlin_selection("unsupported", "unsupported")]).unwrap_err();
        assert!(
            error.message.contains("FUNCTION declarations only"),
            "{kind}: {}",
            error.message
        );
    }

    let mut syntax = evidence();
    let normalized = json!({"syntaxKind":"function_declaration", "symbolIdentity":"example.Sample.render", "scope":":/main"});
    syntax.observations.insert(
        "syntax-only".into(),
        Observation {
            id: "syntax-only".into(),
            kind: "SYMBOL".into(),
            service: syntax.service.clone(),
            symbol: "example.Sample.render".into(),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![],
        },
    );
    let checked = kotlin_check(syntax);
    let error = project(&checked, &[kotlin_selection("syntax-only", "syntax-only")]).unwrap_err();
    assert!(
        error
            .message
            .contains("syntax-only declarations are unsupported")
    );
}

#[test]
fn kotlin_graph_expansion_flags_are_rejected_during_projection() {
    let mut evidence = evidence();
    add_kotlin_declaration(&mut evidence, "render", "FUNCTION", "fun render() = 1");
    let checked = kotlin_check(evidence);
    let mut expanded = kotlin_selection("render", "render");
    expanded.expand_source_calls = true;
    let expanded = project(&checked, &[expanded]).unwrap();
    assert!(expanded.source_call_graph.is_some());
    assert!(
        !expanded.pages[0]
            .endpoint
            .gaps
            .iter()
            .any(|gap| gap.code == "KOTLIN_SOURCE_CALL_GRAPH_UNAVAILABLE")
    );
    for source_calls in [false, true] {
        let mut selection = kotlin_selection("render", "render");
        selection.expand_source_calls = source_calls;
        selection.expand_data_state = true;
        let error = project(&checked, &[selection]).unwrap_err();
        assert!(
            error
                .message
                .contains("expandDataState is unavailable for declaration-only"),
            "{}",
            error.message
        );
    }
}

#[test]
fn kotlin_source_escapes_to_paired_mdx_and_html_with_citation_links() {
    let mut evidence = evidence();
    let retained = r#"fun render(value: String) = "Hello, ${value}, $value `literal`""#;
    add_kotlin_declaration(&mut evidence, "render", "FUNCTION", retained);
    add_kotlin_declaration(
        &mut evidence,
        "additional",
        "FUNCTION",
        "fun additional() = 2",
    );
    let checked = kotlin_check(evidence);
    let mut selection = kotlin_selection("render", "render");
    selection.wiring_declaration = Some("additional".into());
    selection.question = Some("What source is retained?".into());
    let projection = project(&checked, &[selection]).unwrap();
    assert_eq!(
        projection.pages[0].projection_kind,
        Some(ProjectionKind::DeclarationOnly)
    );
    assert_eq!(
        projection.pages[0].selection.question.as_deref(),
        Some("What source is retained?")
    );
    assert_eq!(
        projection.pages[0].wiring.as_ref().unwrap().symbol,
        "callable:example/Sample.additional#jvm:(Ljava/lang/String;)Ljava/lang/String;"
    );
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("pages");
    super::super::publish::write(&output, "snapshot-kotlin", &projection).unwrap();

    let mdx = std::fs::read_to_string(output.join("kotlin-page-overview.mdx")).unwrap();
    let html = std::fs::read_to_string(output.join("kotlin-page-overview.html")).unwrap();
    let sources = std::fs::read_to_string(output.join("sources.mdx")).unwrap();
    let fields_state =
        std::fs::read_to_string(output.join("kotlin-page-fields-state.mdx")).unwrap();
    let diagnostic = std::fs::read_to_string(output.join("kotlin-page-diagnostic.mdx")).unwrap();
    assert!(mdx.contains("Selected functions:"));
    assert!(mdx.contains("Additional selected declaration"));
    assert!(mdx.contains("callable:example/Sample.additional"));
    assert!(mdx.contains("Selected declarations are retained without inferring behavior or a relationship between them."));
    assert!(mdx.contains("sources.mdx#ref-"));
    assert!(mdx.contains("KOTLIN&#95;BEHAVIOR&#95;PROJECTION&#95;UNAVAILABLE"));
    assert!(sources.contains("All selected declarations"));
    assert!(!sources.contains("All processes"));
    assert!(sources.contains("$&#123;value&#125;"));
    assert!(sources.contains("&#96;literal&#96;"));
    assert!(!sources.contains("${value}") && !sources.contains("`literal`"));
    assert!(fields_state.contains("KOTLIN&#95;DATA&#95;STATE&#95;UNAVAILABLE"));
    assert!(!fields_state.contains("<table "));
    assert!(diagnostic.contains("Source-condition analysis is unavailable"));
    assert!(diagnostic.contains("Question: What source is retained?"));
    assert!(!diagnostic.contains("Rows concern direct calls"));
    let body = html
        .split_once("<body>\n")
        .unwrap()
        .1
        .strip_suffix("</body></html>\n")
        .unwrap();
    assert_eq!(mdx, body.replace(".html", ".mdx"));
}

#[test]
fn mixed_java_and_kotlin_selection_never_runs_the_java_handoff_proof() {
    let mut evidence = evidence();
    add_declaration(
        &mut evidence,
        "java-endpoint",
        "Example",
        "run",
        "METHOD",
        "void run() {}",
    );
    add_kotlin_declaration(
        &mut evidence,
        "kotlin-worker",
        "FUNCTION",
        "fun render() = 1",
    );
    let checked = kotlin_check(evidence);
    let projected = project(
        &checked,
        &[kotlin_selection("java-endpoint", "kotlin-worker")],
    )
    .unwrap();
    assert_eq!(projected.schema, DECLARATION_SCHEMA);
    let page = &projected.pages[0];
    assert_eq!(page.projection_kind, Some(ProjectionKind::DeclarationOnly));
    assert_eq!(page.handoff.status, "DECLARATION_ONLY");
    assert!(
        !page
            .handoff
            .gaps
            .iter()
            .any(|gap| gap.code == "SOURCE_DECLARED_SHARED_QUEUE")
    );
    assert_eq!(page.endpoint.authority, "COMPILER_DECLARATION");
    assert_eq!(page.worker.authority, "COMPILER_DECLARATION");
}

#[test]
fn separate_java_graph_page_does_not_expand_a_kotlin_declaration_page() {
    let mut evidence = kotlin_retained_call_sites_evidence();
    add_declaration(
        &mut evidence,
        "java-endpoint",
        "Example",
        "accept",
        "METHOD",
        "void accept() {}",
    );
    add_declaration(
        &mut evidence,
        "java-worker",
        "Example",
        "tick",
        "METHOD",
        "void tick() {}",
    );
    let checked = kotlin_check(evidence);

    let mut java = kotlin_selection("java-endpoint", "java-worker");
    java.id = "java-page".into();
    java.expand_source_calls = true;
    let mut kotlin = kotlin_selection("answer-next", "answer-next");
    kotlin.id = "kotlin-page".into();
    kotlin.expand_source_calls = true;
    let projection = project(&checked, &[java, kotlin]).unwrap();
    assert_eq!(projection.schema, DECLARATION_SCHEMA);
    assert_eq!(
        projection.pages[0].projection_kind,
        Some(ProjectionKind::SourceBehavior)
    );
    assert_eq!(
        projection.pages[1].projection_kind,
        Some(ProjectionKind::DeclarationOnly)
    );
    let graph = projection.source_call_graph.as_ref().unwrap();
    assert!(
        graph
            .nodes
            .values()
            .all(|node| !node.callable.symbol.contains("kotlin-entry"))
    );
    assert!(projection.pages[0].examined_sources.is_some());
    assert!(projection.pages[1].examined_sources.is_some());
    assert!(projection.pages[1].data_state.is_none());
    assert!(
        graph
            .nodes
            .values()
            .any(|node| node.node_projection_kind.is_none())
    );
    let kotlin_caller = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == "answer-next")
        .unwrap();
    assert_eq!(kotlin_caller.calls.len(), 2);
    assert!(
        kotlin_caller
            .calls
            .iter()
            .all(|edge| edge.exact_call_site.is_some())
    );
    assert!(graph.process_links.is_empty());
}

#[test]
fn fresh_renamed_sources_drive_projection_and_shared_queue_proof() {
    for (ingress, consumer, queue) in [
        ("Ingress", "Consumer", "pending"),
        ("Intake", "DispatchLoop", "backlog"),
    ] {
        let (checked, selection) = scenario(
            ingress,
            consumer,
            queue,
            "task.sku.trim()",
            "!task.eligible()",
            false,
        );
        let p = page(&checked, selection.clone());
        assert_eq!(
            p.handoff.status, "SOURCE_DECLARED_SHARED_QUEUE",
            "{:?}",
            p.handoff
        );
        assert_eq!(p.handoff.endpoint_field.as_deref(), Some(queue));
        assert_eq!(p.handoff.worker_field.as_deref(), Some(queue));
        assert!(
            p.worker
                .state
                .iter()
                .any(|s| s.name == "prepared" && s.expression == "task.sku.trim()")
        );
        let gateway = all_steps(&p.worker.steps)
            .into_iter()
            .find(|s| {
                s.calls
                    .iter()
                    .any(|c| c.expression == "gateway.send(prepared)")
            })
            .unwrap();
        assert_eq!(gateway.conditions.len(), 3);
        assert!(gateway.conditions.iter().all(|c| !c.holds));
        assert!(
            p.diagnostics
                .iter()
                .any(|d| d.condition == "(!enabled) is true")
        );
        assert!(
            p.diagnostics
                .iter()
                .any(|d| d.inspect.iter().any(|i| i.contains("eligible")))
        );
        assert!(
            p.sources
                .values()
                .any(|s| s.text.contains("this.") && s.text.contains(" = source"))
        );
        let bundle = project(&checked, &[selection]).unwrap();
        assert_eq!(
            bundle,
            project(&checked, &[bundle.pages[0].selection.clone()]).unwrap()
        );
        assert_ne!(bundle.selection_digest, bundle.input_digest);
    }
}

#[test]
fn guarded_call_diagnostics_do_not_depend_on_target_source_availability() {
    let (binary, selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        false,
    );
    let target = "method:class:publicapi.Gateway#send(Ljava/lang/String;)LResponse;";
    let mut retained = binary.services["sample"].clone();
    retained.observations.retain(|_, observation| {
        !(observation.kind == "DEPENDENCY_TARGET"
            && observation.normalized["symbolIdentity"] == target)
    });
    add_declaration(
        &mut retained,
        "gateway-send",
        "publicapi.Gateway",
        "send",
        "METHOD",
        "Response send(String request) { return null; }",
    );
    let declaration = retained.observations.get_mut("gateway-send").unwrap();
    declaration.symbol = target.into();
    declaration.normalized["symbolIdentity"] = json!(target);
    declaration.normalized["jvmDescriptor"] = json!("(Ljava/lang/String;)LResponse;");
    declaration.digest = digest(&declaration.normalized).unwrap();
    let source = check::assemble(
        "input-digest".into(),
        BTreeMap::from([("sample".into(), retained)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let binary = page(&binary, selection.clone());
    let source = page(&source, selection);
    let find_call = |page: &PageContent| {
        all_steps(&page.worker.steps)
            .into_iter()
            .flat_map(|row| &row.calls)
            .find(|call| call.target.as_deref() == Some(target))
            .unwrap()
            .clone()
    };
    assert!(find_call(&binary).external_boundary.is_some());
    assert!(find_call(&source).external_boundary.is_none());
    let for_target = |page: &PageContent| {
        page.diagnostics
            .iter()
            .filter(|row| row.selected_call == target)
            .cloned()
            .collect::<Vec<_>>()
    };
    let binary_rows = for_target(&binary);
    let source_rows = for_target(&source);
    assert_eq!(source_rows, binary_rows);
    assert_eq!(source_rows.len(), 3);
    assert!(
        source_rows
            .iter()
            .any(|row| row.condition == "(!enabled) is true")
    );
    assert!(
        source_rows
            .iter()
            .all(|row| !row.condition.contains("response"))
    );
    for row in &source_rows {
        assert_eq!(row.citation_ids.len(), 2);
        assert!(
            row.citation_ids
                .iter()
                .all(|id| source.citations.contains_key(id))
        );
        assert!(
            row.inspect
                .iter()
                .any(|text| text == "gateway.send(prepared)")
        );
    }
    assert!(source.worker.state.iter().any(|row| {
        row.name == "this.lastResponse"
            && row
                .conditions
                .iter()
                .any(|condition| condition.expression == "(!response.ok())" && !condition.holds)
    }));
}

#[test]
fn guard_and_request_mutations_change_native_content_and_diagnostics() {
    let (a, s) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku.trim()",
        "!task.eligible()",
        false,
    );
    let (b, _) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku.toUpperCase()",
        "task.quantity <= 3",
        false,
    );
    let a = page(&a, s.clone());
    let b = page(&b, s);
    assert_ne!(a.worker.steps, b.worker.steps);
    assert!(
        b.worker
            .state
            .iter()
            .any(|s| s.expression == "task.sku.toUpperCase()")
    );
    assert!(
        b.diagnostics
            .iter()
            .any(|d| d.condition.contains("quantity <= 3"))
    );
    assert!(
        !b.diagnostics
            .iter()
            .any(|d| d.condition.contains("eligible"))
    );
}

#[test]
fn conditional_or_distinct_wiring_and_missing_compiler_remain_local_gaps() {
    let (checked, selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        true,
    );
    assert_eq!(
        page(&checked, selection.clone()).handoff.gaps[0].code,
        "WIRING_CONTROL_AMBIGUOUS"
    );
    let (mut checked, _) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        false,
    );
    checked
        .services
        .get_mut("sample")
        .unwrap()
        .observations
        .get_mut("worker")
        .unwrap()
        .normalized["schema"] = json!("syntax-only");
    let p = page(&checked, selection.clone());
    assert_eq!(p.handoff.status, "LOCAL_GAP");
    assert_eq!(p.worker.authority, "SYNTAX_SOURCE");
    assert!(
        all_steps(&p.worker.steps)
            .iter()
            .flat_map(|s| &s.calls)
            .all(|c| c.target.is_none())
    );
    let p = page(
        &checked,
        Selection {
            wiring_declaration: None,
            ..selection
        },
    );
    assert_eq!(p.handoff.gaps[0].code, "WIRING_NOT_SELECTED");
}

#[test]
fn citations_preserve_unicode_ranges_and_missing_dependency_source_status() {
    let (checked, selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "\"café☕\"",
        "!task.eligible()",
        false,
    );
    let p = page(&checked, selection);
    for citation in p.citations.values() {
        let source = &p.sources[&citation.source_id];
        assert_eq!(
            citation.text_digest,
            crate::canonical::hash_bytes(
                &source.text.as_bytes()[citation.start_byte..citation.end_byte]
            )
        );
        assert!(citation.start_line >= source.start_line && citation.end_line <= source.end_line);
        assert!(
            citation
                .url
                .as_ref()
                .unwrap()
                .ends_with(&format!("#L{}-L{}", citation.start_line, citation.end_line))
        );
    }
    let boundary = all_steps(&p.worker.steps)
        .into_iter()
        .flat_map(|s| &s.calls)
        .find(|c| c.name == "send")
        .unwrap()
        .external_boundary
        .as_ref()
        .unwrap();
    assert_eq!(boundary.source_status, "SOURCE_UNAVAILABLE");
    assert!(boundary.citation_ids.is_empty());
}

#[test]
fn negation_else_early_returns_and_unsupported_expressions_remain_ordered() {
    let mut e = evidence();
    let text = "void run() {\n if (!ready) { return; } else { input = 2; }\n if (valid) { input = 3; } else { throw new Failure(); }\n if (ready && probe()) return;\n input += 5;\n input++;\n Runnable callback = () -> { int deferred = 4; input = 8; hidden(); };\n int value = ready ? left() : right();\n while (ready) { loopCall(); }\n gateway.send(\"café☕\");\n}";
    add_declaration(&mut e, "endpoint", "Loop", "run", "METHOD", text);
    bind_call(
        &mut e,
        "endpoint",
        "probe()",
        "method:class:Loop#probe()Z",
        "CALLS",
        None,
    );
    bind_call(
        &mut e,
        "endpoint",
        "gateway.send(\"café☕\")",
        "method:class:publicapi.Gateway#send(Ljava/lang/String;)V",
        "CALLS",
        Some("SOURCE_UNAVAILABLE"),
    );
    let checked = check::assemble(
        "input".into(),
        BTreeMap::from([("sample".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let p = page(
        &checked,
        Selection {
            id: "guards".into(),
            service: "sample".into(),
            endpoint_declaration: "endpoint".into(),
            worker_declaration: "endpoint".into(),
            wiring_declaration: None,
            question: None,
            note_ids: vec![],
            authored_paragraphs: vec![],
            expand_source_calls: false,
            expand_data_state: false,
        },
    );
    assert_eq!(p.worker.steps[2].conditions[0].expression, "(!ready)");
    assert!(!p.worker.steps[2].conditions[0].holds);
    assert!(p.worker.steps[2].conditions[1].holds);
    assert!(
        all_steps(&p.worker.steps)
            .iter()
            .flat_map(|s| &s.calls)
            .all(|c| c.name != "probe"
                && c.name != "hidden"
                && c.name != "left"
                && c.name != "loopCall")
    );
    assert!(
        p.worker
            .state
            .iter()
            .any(|s| s.expression == "input += 5" && s.kind == "UNSUPPORTED_MUTATION")
    );
    for code in [
        "SHORT_CIRCUIT_CALLS_CONDITIONAL",
        "UPDATE_EXPRESSION_UNSUPPORTED",
        "UNSUPPORTED_EXPRESSION",
        "UNSUPPORTED_CONTROL",
    ] {
        assert!(
            all_steps(&p.worker.steps)
                .iter()
                .flat_map(|s| &s.gaps)
                .any(|g| g.code == code),
            "missing {code}"
        );
    }
    assert!(
        p.observations
            .values()
            .any(|o| o.normalized["targetIdentity"] == "method:class:Loop#probe()Z")
    );
    assert!(
        p.worker
            .state
            .iter()
            .all(|s| s.name != "deferred" && s.expression != "8")
    );
    assert!(
        p.worker
            .steps
            .last()
            .unwrap()
            .conditions
            .iter()
            .any(|c| c.expression.contains("unsupported while_statement"))
    );
}

#[test]
fn one_same_line_relation_cannot_be_borrowed_by_a_different_call() {
    let mut e = evidence();
    add_declaration(
        &mut e,
        "endpoint",
        "Calls",
        "run",
        "METHOD",
        "void run() { one(); two(); one(); }",
    );
    bind_call(
        &mut e,
        "endpoint",
        "one()",
        "method:class:Calls#one()V",
        "CALLS",
        None,
    );
    let relation = e
        .observations
        .values()
        .find(|o| o.kind == "CALL_RELATION")
        .unwrap()
        .clone();
    let body = e.sources["src-endpoint"].clone();
    let site = e.sources.get_mut(&relation.source_ids[0]).unwrap();
    site.text = body.text.clone();
    site.text_digest = body.text_digest;
    site.occurrence = None;
    let relation = e.observations.get_mut(&relation.id).unwrap();
    relation.normalized["callSite"]["sourceDigest"] = json!(site.text_digest);
    e.sources.get_mut("src-endpoint").unwrap().occurrence = None;
    let checked = check::assemble(
        "input".into(),
        BTreeMap::from([("sample".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let p = page(
        &checked,
        Selection {
            id: "calls".into(),
            service: "sample".into(),
            endpoint_declaration: "endpoint".into(),
            worker_declaration: "endpoint".into(),
            wiring_declaration: None,
            question: None,
            note_ids: vec![],
            authored_paragraphs: vec![],
            expand_source_calls: false,
            expand_data_state: false,
        },
    );
    assert!(
        all_steps(&p.worker.steps)
            .iter()
            .flat_map(|s| &s.calls)
            .all(|c| c.target.is_none())
    );
}

#[test]
fn constructor_parameter_reassignment_does_not_prove_the_incoming_queue() {
    let (mut checked, selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        false,
    );
    let e = checked.services.get_mut("sample").unwrap();
    let endpoint_ctor = add_declaration(
        e,
        "constructor-endpoint",
        "Ingress",
        "<init>",
        "CONSTRUCTOR",
        "Ingress(BlockingQueue<Task> source, BlockingQueue<Task> alternate) {\n source = alternate;\n this.pending = source;\n}",
    );
    let worker_ctor = e.observations["constructor-worker"].symbol.clone();
    add_declaration(
        e,
        "wiring",
        "Bootstrap",
        "build",
        "METHOD",
        "static void build() {\n BlockingQueue<Task> shared = new LinkedBlockingQueue<>();\n BlockingQueue<Task> other = new ArrayBlockingQueue<>(20);\n Ingress entry = new Ingress(shared, other);\n Consumer worker = new Consumer(shared);\n}",
    );
    bind_call(
        e,
        "wiring",
        "new LinkedBlockingQueue<>()",
        "method:class:java.util.concurrent.LinkedBlockingQueue#<init>()V",
        "CONSTRUCTS",
        Some("SOURCE_UNAVAILABLE"),
    );
    bind_call(
        e,
        "wiring",
        "new ArrayBlockingQueue<>(20)",
        "method:class:java.util.concurrent.ArrayBlockingQueue#<init>(I)V",
        "CONSTRUCTS",
        Some("SOURCE_UNAVAILABLE"),
    );
    bind_call(
        e,
        "wiring",
        "new Ingress(shared, other)",
        &endpoint_ctor,
        "CONSTRUCTS",
        None,
    );
    bind_call(
        e,
        "wiring",
        "new Consumer(shared)",
        &worker_ctor,
        "CONSTRUCTS",
        None,
    );
    let p = page(&checked, selection);
    assert_eq!(p.handoff.status, "LOCAL_GAP");
    assert_eq!(p.handoff.gaps[0].code, "CONSTRUCTOR_PARAMETER_REASSIGNED");
}

#[test]
fn nested_early_return_guards_restrict_the_following_call() {
    let mut e = evidence();
    add_declaration(
        &mut e,
        "endpoint",
        "Nested",
        "run",
        "METHOD",
        "void run() {\n if (ready) { if (disabled) return; } else { return; }\n gateway.send(value);\n}",
    );
    bind_call(
        &mut e,
        "endpoint",
        "gateway.send(value)",
        "method:class:publicapi.Gateway#send(Ljava/lang/String;)V",
        "CALLS",
        Some("SOURCE_UNAVAILABLE"),
    );
    let checked = check::assemble(
        "input".into(),
        BTreeMap::from([("sample".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let p = page(
        &checked,
        Selection {
            id: "nested".into(),
            service: "sample".into(),
            endpoint_declaration: "endpoint".into(),
            worker_declaration: "endpoint".into(),
            wiring_declaration: None,
            question: None,
            note_ids: vec![],
            authored_paragraphs: vec![],
            expand_source_calls: false,
            expand_data_state: false,
        },
    );
    let call = p.worker.steps.last().unwrap();
    assert!(
        call.conditions
            .iter()
            .any(|c| c.expression == "(ready)" && c.holds)
    );
    assert!(
        call.conditions
            .iter()
            .any(|c| c.expression == "(disabled)" && !c.holds)
    );
    assert!(
        p.diagnostics
            .iter()
            .any(|d| d.condition == "(disabled) is true")
    );
}

#[test]
fn switch_expression_arms_are_retained_without_unconditional_calls() {
    let mut e = evidence();
    add_declaration(
        &mut e,
        "endpoint",
        "Choices",
        "run",
        "METHOD",
        "void run() {\n int value = switch (key) { case 1 -> first(); default -> second(); };\n}",
    );
    bind_call(
        &mut e,
        "endpoint",
        "first()",
        "method:class:Choices#first()I",
        "CALLS",
        None,
    );
    let checked = check::assemble(
        "input".into(),
        BTreeMap::from([("sample".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let p = page(
        &checked,
        Selection {
            id: "switch".into(),
            service: "sample".into(),
            endpoint_declaration: "endpoint".into(),
            worker_declaration: "endpoint".into(),
            wiring_declaration: None,
            question: None,
            note_ids: vec![],
            authored_paragraphs: vec![],
            expand_source_calls: false,
            expand_data_state: false,
        },
    );
    assert!(p.worker.steps[0].calls.is_empty());
    assert!(
        p.worker.steps[0]
            .gaps
            .iter()
            .any(|g| g.code == "UNSUPPORTED_EXPRESSION")
    );
    assert!(
        p.observations
            .values()
            .any(|o| o.normalized["targetIdentity"] == "method:class:Choices#first()I")
    );
}

fn pin_note(checked: &mut Check, targets: &[&str]) {
    let temporary = tempfile::tempdir().unwrap();
    crate::documentation::store::Repository::init(temporary.path(), "Notes").unwrap();
    let repo = crate::documentation::store::Repository::open(temporary.path()).unwrap();
    let mut inputs = repo.inputs().unwrap();
    for id in ["sample", "other"] {
        let service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0","id":id,"title":id,
            "repositoryId":id,"repository":format!("https://example.invalid/{id}"),
            "language":"java","profile":"java-17plus-maven-read-only",
            "compilations":[":/main"],"targetRef":"main"
        }))
        .unwrap();
        inputs.services.insert(id.into(), service);
    }
    let association = json!({
        "schema":"codeclew-documentation-note-association/1.0","id":"operations",
        "title":"On-call instructions","service":"sample","path":"notes/operations.md",
        "targets":targets,"classification":"policy","period":"2026",
        "tags":["on-call"],"metadata":{"author":"Example maintainer","origin":"imported"}
    });
    let text = "Keep café☕ ready.\r\n<Panel>{danger()}</Panel>\r\n";
    inputs.notes.insert(
        "operations".into(),
        json!({
            "associationDigest":digest(&association).unwrap(),"association":association,
            "original":{"status":"CAPTURED","digest":digest(&text).unwrap(),"text":text},
            "authority":"HUMAN_OR_IMPORTED_UNVERIFIED"
        }),
    );
    checked.source_inputs = Some(check::SourceInputs {
        schema: check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest: digest(&inputs).unwrap(),
        inputs,
        selected_services: BTreeSet::from(["sample".into()]),
        retained_services: BTreeSet::new(),
    });
    refresh_note_input_identity(checked);
}

fn refresh_note_input_identity(checked: &mut Check) {
    let pinned = checked.source_inputs.as_mut().unwrap();
    pinned.input_digest = digest(&pinned.inputs).unwrap();
    checked.input_digest = pinned.input_digest.clone();
}

#[test]
fn selected_notes_preserve_captured_text_attribution_and_legacy_selector_identity() {
    let (mut checked, mut selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        false,
    );
    let legacy = serde_json::to_value(&selection).unwrap();
    assert!(legacy.get("noteIds").is_none());
    let mut explicit_empty = legacy.clone();
    explicit_empty["noteIds"] = json!([]);
    let explicit_empty: Selection = serde_json::from_value(explicit_empty).unwrap();
    assert_eq!(
        digest(&[selection.clone()]).unwrap(),
        digest(&[explicit_empty]).unwrap()
    );
    assert!(
        serde_json::to_value(page(&checked, selection.clone()))
            .unwrap()
            .get("humanInstructions")
            .is_none()
    );
    pin_note(&mut checked, &["service:sample/section-egress"]);
    selection.note_ids.push("operations".into());
    let note = &page(&checked, selection.clone()).human_instructions[0];
    assert_eq!(
        note.text,
        "Keep café☕ ready.\r\n<Panel>{danger()}</Panel>\r\n"
    );
    assert_eq!(note.declared_author, "Example maintainer");
    assert_eq!(note.classification, "policy");
    assert_eq!(note.period, "2026");
    assert_eq!(note.content_digest, digest(&note.text).unwrap());
    assert_eq!(
        note.version_digest,
        digest(&(&note.association_digest, &note.content_digest)).unwrap()
    );
    assert_eq!(note.authority, "HUMAN_OR_IMPORTED_UNVERIFIED");
    assert_eq!(note.source_claim_status, "UNASSESSED");
    assert_eq!(note.association["metadata"]["origin"], "imported");
    // Observation membership cannot substitute for pinned original material.
    checked.source_inputs = None;
    assert!(
        project(&checked, &[selection])
            .unwrap_err()
            .message
            .contains("pinned")
    );
}

#[test]
fn note_selection_rejects_unavailable_unattributed_unrelated_and_inconsistent_material() {
    let (mut checked, mut selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        false,
    );
    pin_note(&mut checked, &["service:sample"]);
    selection.note_ids = vec!["operations".into()];
    for author in [json!(null), json!(7), json!(" \t"), json!("x".repeat(513))] {
        let mut invalid = checked.clone();
        let captured = invalid
            .source_inputs
            .as_mut()
            .unwrap()
            .inputs
            .notes
            .get_mut("operations")
            .unwrap();
        captured["association"]["metadata"]["author"] = author;
        captured["associationDigest"] = json!(digest(&captured["association"]).unwrap());
        refresh_note_input_identity(&mut invalid);
        assert!(
            project(&invalid, &[selection.clone()])
                .unwrap_err()
                .message
                .contains("metadata.author")
        );
    }
    for targets in [
        vec!["service:other"],
        vec!["service:sample/not-a-section"],
        vec!["entity:operations"],
    ] {
        let mut invalid = checked.clone();
        pin_note(&mut invalid, &targets);
        assert!(project(&invalid, &[selection.clone()]).is_err());
    }
    for (field, value) in [
        ("status", json!("UNAVAILABLE")),
        ("digest", json!("sha256:wrong")),
        ("text", json!(null)),
    ] {
        let mut invalid = checked.clone();
        invalid
            .source_inputs
            .as_mut()
            .unwrap()
            .inputs
            .notes
            .get_mut("operations")
            .unwrap()["original"][field] = value;
        refresh_note_input_identity(&mut invalid);
        assert!(project(&invalid, &[selection.clone()]).is_err());
    }
    for (field, value) in [
        ("associationDigest", json!("sha256:wrong")),
        ("authority", json!("SOURCE")),
    ] {
        let mut invalid = checked.clone();
        invalid
            .source_inputs
            .as_mut()
            .unwrap()
            .inputs
            .notes
            .get_mut("operations")
            .unwrap()[field] = value;
        refresh_note_input_identity(&mut invalid);
        assert!(project(&invalid, &[selection.clone()]).is_err());
    }
    let mut invalid = checked.clone();
    let captured = invalid
        .source_inputs
        .as_mut()
        .unwrap()
        .inputs
        .notes
        .get_mut("operations")
        .unwrap();
    captured["association"]["id"] = json!("other");
    captured["associationDigest"] = json!(digest(&captured["association"]).unwrap());
    refresh_note_input_identity(&mut invalid);
    assert!(project(&invalid, &[selection.clone()]).is_err());
    let mut missing = selection.clone();
    missing.note_ids = vec!["missing".into()];
    assert!(project(&checked, &[missing]).is_err());
    selection.note_ids.push("operations".into());
    assert!(
        project(&checked, &[selection])
            .unwrap_err()
            .message
            .contains("unique")
    );
}

fn linked_fixture(guard: &str, transform: &str, b_target: &str) -> (Check, Vec<Selection>) {
    let mut e = evidence();
    for (id, owner, method, body) in [
        ("a-endpoint", "AEndpoint", "submit", "boolean submit(Task task) { return true; }".to_owned()),
        ("b-endpoint", "BEndpoint", "enqueue", "boolean enqueue(Task task) { return true; }".to_owned()),
        ("child-endpoint", "ChildEndpoint", "submit", "boolean submit(Task task) { if (task == null) return false; return true; }".to_owned()),
        ("a-worker", "AWorker", "runOnce", format!("void runOnce() {{ Task task = pending.poll(); if (task == null) return; if ({guard}) return; child.submit(task); }}")),
        ("b-worker", "BWorker", "runOnce", format!("void runOnce() {{ Task task = pending.poll(); if (task == null) return; if (task.priority < 3) return; {b_target}.submit(task); }}")),
        ("child-worker", "ChildWorker", "runOnce", "void runOnce() { Task task = pending.poll(); if (task == null) return; String request = prepare(task); int response = gateway.deliver(request); if (response != 0) return; }".to_owned()),
        ("prepare", "ChildWorker", "prepare", format!("String prepare(Task task) {{ String chosen = task.name; if (chosen == null) chosen = \"anonymous\"; String transformed = {transform}; return prefix + transformed; }}")),
        ("alternative", "RetargetEndpoint", "submit", "boolean submit(Task task) { return true; }".to_owned()),
        ("gateway", "Gateway", "deliver", "int deliver(String request);".to_owned()),
        ("first", "CycleProbe", "first", "int first(int n) { if (n <= 0) return 0; return second(n - 1); }".to_owned()),
        ("second", "CycleProbe", "second", "int second(int n) { if (n <= 0) return 0; return first(n - 1); }".to_owned()),
    ] {
        add_declaration(&mut e, id, owner, method, "METHOD", &body);
    }
    for (caller, expression, target) in [
        (
            "a-worker",
            "child.submit(task)".to_owned(),
            "child-endpoint",
        ),
        (
            "b-worker",
            format!("{b_target}.submit(task)"),
            if b_target == "child" {
                "child-endpoint"
            } else {
                "alternative"
            },
        ),
        ("child-worker", "prepare(task)".to_owned(), "prepare"),
        (
            "child-worker",
            "gateway.deliver(request)".to_owned(),
            "gateway",
        ),
        ("first", "second(n - 1)".to_owned(), "second"),
        ("second", "first(n - 1)".to_owned(), "first"),
    ] {
        let symbol = e.observations[target].symbol.clone();
        bind_call(&mut e, caller, &expression, &symbol, "CALLS", None);
    }
    let checked = check::assemble(
        "input".into(),
        BTreeMap::from([("sample".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let selections = [
        ("parent-a", "a-endpoint", "a-worker"),
        ("parent-b", "b-endpoint", "b-worker"),
        ("child", "child-endpoint", "child-worker"),
        ("cycle", "first", "second"),
    ]
    .into_iter()
    .map(|(id, endpoint, worker)| Selection {
        id: id.into(),
        service: "sample".into(),
        endpoint_declaration: endpoint.into(),
        worker_declaration: worker.into(),
        wiring_declaration: None,
        question: None,
        note_ids: vec![],
        authored_paragraphs: vec![],
        expand_source_calls: true,
        expand_data_state: false,
    })
    .collect();
    (checked, selections)
}

fn examined(p: &BundleProjection, id: &str) -> String {
    p.pages
        .iter()
        .find(|p| p.id == id)
        .unwrap()
        .examined_sources
        .as_ref()
        .unwrap()
        .examined_source_digest
        .clone()
}

#[test]
fn shared_child_body_expansion_and_review_membership_follow_exact_calls() {
    let (checked, selections) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let baseline = project(&checked, &selections).unwrap();
    let graph = baseline.source_call_graph.as_ref().unwrap();
    assert_eq!(graph.process_links.len(), 2);
    assert!(graph.process_links.iter().all(|l| l.to_process == "child"));
    assert_eq!(
        graph
            .nodes
            .values()
            .filter(|n| n.callable.declaration_id == "prepare")
            .count(),
        1
    );
    let prepare = graph
        .nodes
        .values()
        .find(|n| n.callable.declaration_id == "prepare")
        .unwrap();
    assert!(
        prepare
            .callable
            .state
            .iter()
            .any(|s| s.expression == "chosen.trim()")
    );
    assert_eq!(
        graph.reverse_examined_processes[&prepare.id]
            .iter()
            .map(|r| r.process_id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["parent-a", "parent-b", "child"])
    );
    assert!(graph.nodes.values().flat_map(|n| &n.calls).any(|e| {
        e.frontiers
            .iter()
            .any(|g| g.code == "CALL_TARGET_BODY_UNAVAILABLE")
    }));
    assert!(graph.nodes.values().flat_map(|n| &n.calls).any(|e| {
        e.frontiers
            .iter()
            .any(|g| g.code == "SOURCE_CALL_CYCLE_FRONTIER")
    }));
    let child = baseline.pages.iter().find(|p| p.id == "child").unwrap();
    let gateway = all_steps(&child.worker.steps)
        .into_iter()
        .flat_map(|s| s.calls.iter().map(move |c| (s, c)))
        .find(|(_, c)| c.name == "deliver")
        .unwrap();
    assert!(
        !gateway
            .0
            .conditions
            .iter()
            .any(|c| c.expression.contains("response"))
    );
    let (changed, selected) = linked_fixture("task.priority < 1", "chosen.strip()", "child");
    let changed = project(&changed, &selected).unwrap();
    for id in ["parent-a", "parent-b", "child"] {
        assert_ne!(examined(&baseline, id), examined(&changed, id));
    }
    assert_eq!(examined(&baseline, "cycle"), examined(&changed, "cycle"));
    let (changed, selected) = linked_fixture("task.priority < 2", "chosen.trim()", "child");
    let changed = project(&changed, &selected).unwrap();
    assert_ne!(
        examined(&baseline, "parent-a"),
        examined(&changed, "parent-a")
    );
    for id in ["parent-b", "child", "cycle"] {
        assert_eq!(examined(&baseline, id), examined(&changed, id));
    }
    let (changed, selected) = linked_fixture("task.priority < 1", "chosen.trim()", "alternative");
    let changed = project(&changed, &selected).unwrap();
    let links = &changed.source_call_graph.as_ref().unwrap().process_links;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].from_process, "parent-a");
    assert_eq!(
        examined(&baseline, "parent-a"),
        examined(&changed, "parent-a")
    );
    assert_eq!(examined(&baseline, "child"), examined(&changed, "child"));
    assert_ne!(
        examined(&baseline, "parent-b"),
        examined(&changed, "parent-b")
    );
}

#[test]
fn expansion_opt_out_preserves_legacy_bytes_and_provenance_only_changes_do_not_change_review_digest()
 {
    let (mut checked, selections) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let baseline = project(&checked, &selections).unwrap();
    for evidence in checked.services.values_mut() {
        evidence.revision = "b".repeat(40);
        for source in evidence.sources.values_mut() {
            source.revision = evidence.revision.clone();
            source.start_line += 100;
            source.end_line += 100;
            source.url = None;
            source.evidence_digest = "new-compiler-receipt".into();
        }
        for observation in evidence
            .observations
            .values_mut()
            .filter(|o| o.kind == "CALL_RELATION")
        {
            observation.normalized["callSite"]["evidenceDigest"] = json!("new-compiler-receipt");
            observation.digest = digest(&observation.normalized).unwrap();
        }
    }
    let moved = project(&checked, &selections).unwrap();
    assert_ne!(digest(&baseline).unwrap(), digest(&moved).unwrap());
    for id in ["parent-a", "parent-b", "child", "cycle"] {
        assert_eq!(examined(&baseline, id), examined(&moved, id));
    }
    let mut legacy = selections;
    for selection in &mut legacy {
        selection.expand_source_calls = false;
    }
    let value = serde_json::to_value(&legacy).unwrap();
    assert!(!value.to_string().contains("expandSourceCalls"));
    let omitted: Vec<Selection> = serde_json::from_value(value).unwrap();
    assert_eq!(digest(&legacy).unwrap(), digest(&omitted).unwrap());
    let explicit = project(&checked, &legacy).unwrap();
    let omitted = project(&checked, &omitted).unwrap();
    assert_eq!(
        crate::canonical::bytes(&explicit).unwrap(),
        crate::canonical::bytes(&omitted).unwrap()
    );
    assert!(explicit.source_call_graph.is_none());
    assert!(
        !serde_json::to_string(&explicit)
            .unwrap()
            .contains("examinedSources")
    );
}

#[test]
fn wrong_scope_invalid_occurrence_and_ambiguous_child_never_infer_process_links() {
    let (mut checked, mut selected) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let mut duplicate = selected[2].clone();
    duplicate.id = "other-child".into();
    selected.push(duplicate);
    let projected = project(&checked, &selected).unwrap();
    assert!(
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .process_links
            .is_empty()
    );
    assert!(
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .flat_map(|n| &n.calls)
            .any(|e| e
                .frontiers
                .iter()
                .any(|g| g.code == "SELECTED_PROCESS_AMBIGUOUS"))
    );
    selected.pop();
    let e = checked.services.get_mut("sample").unwrap();
    e.observations.get_mut("child-endpoint").unwrap().normalized["scope"] = json!("other-scope");
    let projected = project(&checked, &selected).unwrap();
    assert!(
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .process_links
            .is_empty()
    );
    assert!(
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .flat_map(|n| &n.calls)
            .any(|e| e
                .frontiers
                .iter()
                .any(|g| g.code == "CALL_TARGET_SCOPE_MISMATCH"))
    );
    let (mut checked, selected) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    for r in checked
        .services
        .get_mut("sample")
        .unwrap()
        .observations
        .values_mut()
        .filter(|r| r.kind == "CALL_RELATION" && r.symbol.contains("AWorker"))
    {
        r.normalized["callSite"]["sourceDigest"] = json!("forged");
    }
    let projected = project(&checked, &selected).unwrap();
    assert_eq!(
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .process_links
            .len(),
        1
    );
    assert_eq!(
        projected.source_call_graph.as_ref().unwrap().process_links[0].from_process,
        "parent-b"
    );
}

fn replace_body(e: &mut ServiceEvidence, declaration: &str, text: &str) {
    let sid = e.observations[declaration].source_ids[0].clone();
    let source = source(e, &sid, text);
    e.sources.insert(sid, source);
    let symbol = e.observations[declaration].symbol.clone();
    e.observations
        .retain(|_, o| !(o.kind == "CALL_RELATION" && o.symbol == symbol));
}

#[test]
fn fixed_depth_body_and_text_budgets_are_visible_frontiers() {
    let (mut checked, selected) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let e = checked.services.get_mut("sample").unwrap();
    replace_body(e, "a-worker", "void runOnce() { h0(task); }");
    for i in 0..4 {
        add_declaration(
            e,
            &format!("h{i}"),
            "AWorker",
            &format!("h{i}"),
            "METHOD",
            &format!("void h{i}(Task task) {{ h{}(task); }}", i + 1),
        );
    }
    for i in 0..3 {
        let target = e.observations[&format!("h{}", i + 1)].symbol.clone();
        bind_call(
            e,
            &format!("h{i}"),
            &format!("h{}(task)", i + 1),
            &target,
            "CALLS",
            None,
        );
    }
    let target = e.observations["h0"].symbol.clone();
    bind_call(e, "a-worker", "h0(task)", &target, "CALLS", None);
    let depth = project(&checked, &selected).unwrap();
    assert!(
        depth
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .flat_map(|n| &n.calls)
            .any(|e| e
                .frontiers
                .iter()
                .any(|g| g.code == "SOURCE_CALL_DEPTH_FRONTIER"))
    );
    assert!(
        !depth
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .any(|n| n.callable.declaration_id == "h2")
    );
    let before = examined(&depth, "parent-a");
    let e = checked.services.get_mut("sample").unwrap();
    replace_body(e, "h2", "void h2(Task task) { task = null; }");
    let behind_frontier = project(&checked, &selected).unwrap();
    assert_ne!(digest(&depth).unwrap(), digest(&behind_frontier).unwrap());
    assert_eq!(before, examined(&behind_frontier, "parent-a"));
    let graph = behind_frontier.source_call_graph.as_ref().unwrap();
    assert!(
        !graph
            .nodes
            .values()
            .any(|n| n.callable.declaration_id == "h2")
    );
    let e = checked.services.get_mut("sample").unwrap();
    replace_body(e, "h1", "void h1(Task task) { task = null; h2(task); }");
    let target = e.observations["h2"].symbol.clone();
    bind_call(e, "h1", "h2(task)", &target, "CALLS", None);
    let admitted_change = project(&checked, &selected).unwrap();
    assert_ne!(before, examined(&admitted_change, "parent-a"));
    let (mut checked, selected) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let e = checked.services.get_mut("sample").unwrap();
    let body = format!(
        "void runOnce() {{ {} }}",
        (0..70).map(|i| format!("h{i}(task);")).collect::<String>()
    );
    replace_body(e, "a-worker", &body);
    for i in 0..70 {
        let target = add_declaration(
            e,
            &format!("h{i}"),
            "AWorker",
            &format!("h{i}"),
            "METHOD",
            &format!("void h{i}(Task task) {{}}"),
        );
        bind_call(
            e,
            "a-worker",
            &format!("h{i}(task)"),
            &target,
            "CALLS",
            None,
        );
    }
    let bodies = project(&checked, &selected).unwrap();
    assert!(bodies.source_call_graph.as_ref().unwrap().nodes.len() <= 8 + 64);
    assert!(
        bodies
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .flat_map(|n| &n.calls)
            .any(|e| e
                .frontiers
                .iter()
                .any(|g| g.code == "SOURCE_CALL_BODY_BUDGET_FRONTIER"))
    );
    let (mut checked, selected) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let e = checked.services.get_mut("sample").unwrap();
    replace_body(e, "a-worker", "void runOnce() { giant(task); }");
    let target = add_declaration(
        e,
        "giant",
        "AWorker",
        "giant",
        "METHOD",
        &format!(
            "void giant(Task task) {{ /* {} */ }}",
            "x".repeat(1024 * 1024)
        ),
    );
    bind_call(e, "a-worker", "giant(task)", &target, "CALLS", None);
    let bytes = project(&checked, &selected).unwrap();
    assert!(
        !bytes
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .any(|n| n.callable.declaration_id == "giant")
    );
    assert!(
        bytes
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .flat_map(|n| &n.calls)
            .any(|e| e
                .frontiers
                .iter()
                .any(|g| g.code == "SOURCE_CALL_BYTES_FRONTIER"))
    );
}

#[test]
fn identical_calls_keep_distinct_body_paths_and_occurrence_provenance() {
    let (mut checked, selected) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let e = checked.services.get_mut("sample").unwrap();
    replace_body(
        e,
        "a-worker",
        "void runOnce() {\n child.submit(task);\n child.submit(task);\n}",
    );
    let target = e.observations["child-endpoint"].symbol.clone();
    bind_call(e, "a-worker", "child.submit(task)", &target, "CALLS", None);
    let next = e.observations.len();
    bind_call(e, "a-worker", "child.submit(task)", &target, "CALLS", None);
    let sid = format!("site-a-worker-{next}");
    let body = &e.sources[&e.observations["a-worker"].source_ids[0]];
    let start = body.text.rfind("child.submit(task)").unwrap();
    let line = body.start_line + body.text[..start].bytes().filter(|b| *b == b'\n').count() as u64;
    let source = e.sources.get_mut(&sid).unwrap();
    source.start_line = line;
    source.end_line = line;
    source.occurrence.as_mut().unwrap().start_byte = start;
    source.occurrence.as_mut().unwrap().end_byte = start + "child.submit(task)".len();
    let relation = e.observations.get_mut(&format!("relation-{sid}")).unwrap();
    relation.normalized["callSite"]["byteStart"] = json!(start);
    relation.normalized["callSite"]["byteEnd"] = json!(start + "child.submit(task)".len());
    relation.digest = digest(&relation.normalized).unwrap();
    let projected = project(&checked, &selected).unwrap();
    let links: Vec<_> = projected
        .source_call_graph
        .as_ref()
        .unwrap()
        .process_links
        .iter()
        .filter(|l| l.from_process == "parent-a")
        .collect();
    assert_eq!(links.len(), 2);
    assert_ne!(links[0].occurrence_path, links[1].occurrence_path);
    assert_ne!(links[0].relation_id, links[1].relation_id);
    let calls = &projected.source_call_graph.as_ref().unwrap().nodes[&links[0].caller_node].calls;
    assert_eq!(
        calls
            .iter()
            .filter(|c| c
                .call
                .as_ref()
                .is_some_and(|call| call.expression == "child.submit(task)")
                && c.target_node.is_some())
            .count(),
        2
    );
}

#[test]
fn unsupported_no_call_continuation_uses_stable_statement_path_after_source_identity_relocation() {
    let (mut checked, selected) = linked_fixture("task.priority < 1", "chosen.trim()", "child");
    let e = checked.services.get_mut("sample").unwrap();
    replace_body(
        e,
        "a-worker",
        "void runOnce() { if (flag) { while (flag) { flag = false; } } else { return; } child.submit(task); }",
    );
    let target = e.observations["child-endpoint"].symbol.clone();
    bind_call(e, "a-worker", "child.submit(task)", &target, "CALLS", None);
    let baseline = project(&checked, &selected).unwrap();
    let original = baseline
        .source_call_graph
        .as_ref()
        .unwrap()
        .process_links
        .iter()
        .find(|l| l.from_process == "parent-a")
        .unwrap();
    let original_edge = baseline.source_call_graph.as_ref().unwrap().nodes[&original.caller_node]
        .calls
        .iter()
        .find(|e| e.occurrence_path == Some(original.occurrence_path.clone()))
        .unwrap();
    assert!(
        original_edge
            .conditions
            .as_ref()
            .unwrap()
            .iter()
            .any(|c| c.expression.starts_with("unsupported statement step-"))
    );
    let e = checked.services.get_mut("sample").unwrap();
    let mut remapped = BTreeMap::new();
    for (old, mut source) in std::mem::take(&mut e.sources) {
        let id = format!("relocated-{old}");
        source.id = id.clone();
        source.start_line += 30;
        source.end_line += 30;
        remapped.insert(old, id.clone());
        e.sources.insert(id, source);
    }
    for observation in e.observations.values_mut() {
        for id in &mut observation.source_ids {
            *id = remapped[id].clone();
        }
        if observation.kind == "CALL_RELATION" {
            let old = observation.normalized["callSite"]["sourceId"]
                .as_str()
                .unwrap()
                .to_owned();
            observation.normalized["callSite"]["sourceId"] = json!(remapped[&old]);
            observation.digest = digest(&observation.normalized).unwrap();
        }
    }
    let relocated = project(&checked, &selected).unwrap();
    assert_ne!(digest(&baseline).unwrap(), digest(&relocated).unwrap());
    assert_eq!(
        examined(&baseline, "parent-a"),
        examined(&relocated, "parent-a")
    );
}

#[test]
fn examined_handoff_constructor_dependencies_are_explicit() {
    let (mut checked, mut selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        false,
    );
    selection.expand_source_calls = true;
    let baseline = project(&checked, &[selection.clone()]).unwrap();
    let page = &baseline.pages[0];
    assert_eq!(page.handoff.status, "SOURCE_DECLARED_SHARED_QUEUE");
    let graph = baseline.source_call_graph.as_ref().unwrap();
    let constructors: BTreeSet<_> = page
        .examined_sources
        .as_ref()
        .unwrap()
        .memberships
        .iter()
        .filter(|m| m.reason == "SELECTED_HANDOFF_CONSTRUCTOR")
        .map(|m| graph.nodes[&m.node].callable.declaration_id.as_str())
        .collect();
    assert_eq!(
        constructors,
        BTreeSet::from(["constructor-endpoint", "constructor-worker"])
    );
    for member in page
        .examined_sources
        .as_ref()
        .unwrap()
        .memberships
        .iter()
        .filter(|m| m.reason == "SELECTED_HANDOFF_CONSTRUCTOR")
    {
        assert!(
            graph.reverse_examined_processes[&member.node]
                .iter()
                .any(|r| r.process_id == "journey" && r.reasons.contains(member))
        );
    }
    let e = checked.services.get_mut("sample").unwrap();
    replace_body(
        e,
        "constructor-endpoint",
        "Ingress(BlockingQueue<Task> source) { source = null; this.pending = source; }",
    );
    let changed = project(&checked, &[selection]).unwrap();
    assert_eq!(changed.pages[0].handoff.status, "LOCAL_GAP");
    assert_ne!(
        examined(&baseline, "journey"),
        examined(&changed, "journey")
    );
    assert_ne!(
        baseline.pages[0]
            .examined_sources
            .as_ref()
            .unwrap()
            .handoff_context_digests,
        changed.pages[0]
            .examined_sources
            .as_ref()
            .unwrap()
            .handoff_context_digests
    );
}

#[test]
fn cited_constructor_source_does_not_admit_another_scope_or_owner() {
    let (mut checked, mut selection) = scenario(
        "Ingress",
        "Consumer",
        "pending",
        "task.sku",
        "!task.eligible()",
        false,
    );
    selection.expand_source_calls = true;
    let baseline = project(&checked, &[selection.clone()]).unwrap();
    let e = checked.services.get_mut("sample").unwrap();
    let mut another_scope = e.observations["constructor-endpoint"].clone();
    another_scope.id = "other-scope-constructor".into();
    another_scope.normalized["scope"] = json!("another-compile-scope");
    another_scope.digest = digest(&another_scope.normalized).unwrap();
    let mut another_owner = e.observations["constructor-endpoint"].clone();
    another_owner.id = "other-owner-constructor".into();
    another_owner.normalized["ownerIdentity"] = json!("class:Unrelated");
    another_owner.digest = digest(&another_owner.normalized).unwrap();
    for observation in [&another_scope, &another_owner] {
        e.observations
            .insert(observation.id.clone(), observation.clone());
    }
    let mut projected = project(&checked, &[selection]).unwrap();
    assert_eq!(
        baseline
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .keys()
            .collect::<Vec<_>>(),
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .keys()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        baseline
            .source_call_graph
            .as_ref()
            .unwrap()
            .reverse_examined_processes,
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .reverse_examined_processes
    );
    assert_eq!(
        examined(&baseline, "journey"),
        examined(&projected, "journey")
    );
    // Even additional retained metadata cannot broaden the selected proof scope.
    // Both declarations deliberately reuse the actual proof constructor's source.
    for observation in [another_scope, another_owner] {
        projected.pages[0]
            .observations
            .insert(observation.id.clone(), observation);
    }
    crate::documentation::static_pages::linked::attach(&checked, &mut projected).unwrap();
    assert_eq!(
        baseline
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .keys()
            .collect::<Vec<_>>(),
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .keys()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        baseline
            .source_call_graph
            .as_ref()
            .unwrap()
            .reverse_examined_processes,
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .reverse_examined_processes
    );
    assert_eq!(
        examined(&baseline, "journey"),
        examined(&projected, "journey")
    );
    assert!(
        projected
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .all(|n| { !n.callable.declaration_id.starts_with("other-") })
    );
}

fn exact_outline_check() -> Check {
    let mut evidence = kotlin_answer_next_outline_evidence();
    evidence
        .observations
        .retain(|_, row| row.kind != "LOCAL_CFG");
    let retained = "fun next(api: Api): String {\r\n  // π🙂\r\n  val first = api.pick(\"α🙂\"); val second = api.pick(\"α🙂\")\r\n  return first\r\n}";
    let prefix = "\r\n".repeat(22);
    let full = format!("{prefix}{retained}\r\n");
    let owner_start = prefix.len();
    let owner_end = owner_start + retained.len();
    let full_digest = crate::canonical::hash_bytes(full.as_bytes());
    let owner = evidence.observations["answer-next"].clone();
    let owner_id = owner.source_ids[0].clone();
    let owner_source = evidence.sources.get_mut(&owner_id).unwrap();
    owner_source.text = retained.into();
    owner_source.text_digest = crate::canonical::hash_bytes(retained.as_bytes());
    owner_source.end_line = owner_source.start_line + 4;
    let owner_site = json!({"file":owner_source.file,"byteStart":owner_start,"byteEnd":owner_end,
        "sourceStatus":"SOURCE_RETAINED","sourceId":owner_id,"sourceDigest":owner_source.text_digest,
        "evidenceDigest":owner_source.evidence_digest,"fullCompilationSourceDigest":full_digest});
    let call = "api.pick(\"α🙂\")";
    let first = retained.find(call).unwrap();
    let second = retained[first + call.len()..].find(call).unwrap() + first + call.len();
    let local_first = retained.find("val first").unwrap();
    let local_second = retained.find("val second").unwrap();
    let returned = retained.find("return first").unwrap();
    let specs = [
        ("CALL", first, first + call.len()),
        ("LOCAL", local_first, first + call.len()),
        ("CALL", second, second + call.len()),
        ("LOCAL", local_second, second + call.len()),
        ("RETURN", returned, returned + "return first".len()),
    ];
    let events = specs.iter().enumerate().map(|(ordinal,(kind,start,end))| {
        let mut event = json!({"kind":kind,"sourceSpan":{
            "schema":"codeclew-documentation-source-span/1.0","coordinateDomain":"ORIGINAL_UTF8_BYTES",
            "ownerSymbolIdentity":owner.symbol,"compilationScope":":/main","file":owner_site["file"],
            "ordinal":ordinal,"ownerByteStart":owner_start,"ownerByteEnd":owner_end,
            "byteStart":owner_start+start,"byteEnd":owner_start+end,"fullCompilationSourceDigest":full_digest
        }});
        if *kind == "CALL" {
            event["target"] = json!("callable:parity/Api.pick#jvm:(Ljava/lang/String;)Ljava/lang/String;");
            event["resolution"] = json!("COMPILER_EXACT");
        }
        event
    }).collect();
    let flow_ids = set_kotlin_outline_events(&mut evidence, "answer-next", events);
    evidence
        .observations
        .get_mut("answer-next")
        .unwrap()
        .normalized["outlineOwnerSource"] = owner_site.clone();
    refresh_observation_digest(&mut evidence, "answer-next");
    let owner_source = evidence.sources[&owner_id].clone();
    for (id, (_, start, end)) in flow_ids.iter().zip(specs) {
        let source_id = evidence.observations[id].source_ids[0].clone();
        let source = evidence.sources.get_mut(&source_id).unwrap();
        source.text = retained[start..end].into();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        source.start_line = owner_source.start_line
            + retained[..start].bytes().filter(|b| *b == b'\n').count() as u64;
        source.end_line = source.start_line;
        let site = json!({"file":source.file,"startLine":source.start_line,"endLine":source.end_line,
            "byteStart":owner_start+start,"byteEnd":owner_start+end,"sourceStatus":"SOURCE_RETAINED",
            "sourceId":source_id,"sourceDigest":source.text_digest,"evidenceDigest":source.evidence_digest,
            "fullCompilationSourceDigest":full_digest});
        evidence.observations.get_mut(id).unwrap().normalized["outlineSite"] = site;
        refresh_observation_digest(&mut evidence, id);
    }
    kotlin_check(evidence)
}

#[test]
fn kotlin_outline_exact_spans_preserve_same_line_occurrences_local_and_return_text() {
    let checked = exact_outline_check();
    let projection = project(&checked, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
    let page = &projection.pages[0];
    let outline = page.endpoint.source_outline.as_ref().unwrap();
    let expressions = outline
        .events
        .iter()
        .map(|event| {
            assert!(event.gaps.is_empty());
            let exact = event.exact_source.as_ref().unwrap();
            let cite = &page.citations[&event.citation_id];
            let source = &page.sources[&cite.source_id];
            assert_eq!(source.text, exact.expression);
            assert_eq!(
                cite.text_digest,
                crate::canonical::hash_bytes(exact.expression.as_bytes())
            );
            exact.expression.as_str()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        expressions,
        vec![
            "api.pick(\"α🙂\")",
            "val first = api.pick(\"α🙂\")",
            "api.pick(\"α🙂\")",
            "val second = api.pick(\"α🙂\")",
            "return first"
        ]
    );
    assert_ne!(outline.events[0].citation_id, outline.events[2].citation_id);
    assert_ne!(outline.events[1].citation_id, outline.events[3].citation_id);
    let tree = outline.tree.as_ref().unwrap();
    assert!(tree.contains("LOCAL PSI source: val second"));
    assert!(tree.contains("return PSI source: return first"));
    let temp = tempfile::tempdir().unwrap();
    super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    for ext in ["html", "mdx"] {
        let body = std::fs::read_to_string(temp.path().join(format!("kotlin-page-endpoint.{ext}")))
            .unwrap();
        assert!(body.contains("Retained source: <code>return first</code>"));
    }
}

#[test]
fn kotlin_outline_invalid_optional_spans_are_local_gaps_without_losing_events() {
    let checked = exact_outline_check();
    let cases = [
        ("ownerSymbolIdentity", json!("callable:wrong/owner#jvm:()V")),
        ("compilationScope", json!(":other/main")),
        ("ordinal", json!(99)),
        (
            "fullCompilationSourceDigest",
            json!(format!("sha256:{}", "a".repeat(64))),
        ),
        ("byteStart", json!(0)),
        ("byteEnd", json!(usize::MAX)),
        (
            "byteStart",
            json!(
                44 + "fun next(api: Api): String {\r\n  // π🙂\r\n  val first = api.pick(\"".len()
                    + 1
            ),
        ),
    ];
    for (key, value) in cases {
        let mut changed = checked.clone();
        let e = changed.services.get_mut("sample").unwrap();
        e.observations.get_mut("answer-next").unwrap().normalized["documentation"]["events"][0]["sourceSpan"]
            [key] = value.clone();
        e.observations
            .get_mut("flow-answer-next-return")
            .unwrap()
            .normalized["sourceSpan"][key] = value;
        refresh_observation_digest(e, "answer-next");
        refresh_observation_digest(e, "flow-answer-next-return");
        let projection =
            project(&changed, &[kotlin_selection("answer-next", "answer-next")]).unwrap();
        let outline = projection.pages[0]
            .endpoint
            .source_outline
            .as_ref()
            .unwrap();
        assert_eq!(outline.events.len(), 5, "{key}");
        assert!(outline.events[0].exact_source.is_none(), "{key}");
        assert_eq!(
            outline.events[0].gaps[0].code, "KOTLIN_SOURCE_OUTLINE_EXACT_SPAN_REJECTED",
            "{key}"
        );
        assert!(outline.events[1].exact_source.is_some(), "{key}");
        let temp = tempfile::tempdir().unwrap();
        super::super::publish::write(temp.path(), "snapshot", &projection).unwrap();
    }
}

/// Compiler-shaped test facts exercise the common capture contract. The separate
/// real Roslyn qualification uses the same committed fixture without synthetic facts.
fn csharp_common_check() -> Check {
    let text =
        include_str!("../../../../../fixtures/csharp-documentation-common-core/ProbeController.cs");
    let file = "ProbeController.cs";
    let scope = "csproj:Probe.csproj@net10.0";
    let service = serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0",
        "id":"sample","title":"Roslyn structure","repositoryId":"probe","repository":"https://example.invalid/probe",
        "language":"csharp","profile":"csharp-dotnet-msbuild-read-only","compilations":[scope],"targetRef":"main"})).unwrap();
    let descriptor = "(LSystem/String;)LSystem/String;";
    let symbol =
        |name: &str| format!("method:class:DocumentationProbe.ProbeController#{name}{descriptor}");
    let interface_symbol = format!("method:class:DocumentationProbe.IFormatter#Format{descriptor}");
    let anchors = |start: usize, end: usize| {
        json!({"file":file,
        "start":text[3..start].encode_utf16().count(),"end":text[3..end].encode_utf16().count(),
        "byteStart":start,"byteEnd":end,
        "startLine":1+text[..start].bytes().filter(|b| *b==b'\n').count(),
        "endLine":1+text[..end-1].bytes().filter(|b| *b==b'\n').count()})
    };
    let mut facts = Vec::new();
    for (name, marker, owner_end_marker) in [
        ("Render", "public string Render", "\r\n    }"),
        ("Prepare", "private static string Prepare", "\r\n    }"),
        ("Forward", "public string Forward", ";"),
        ("Unsupported", "public string Unsupported", "\r\n    }"),
        ("Format", "string Format", ";"),
    ] {
        let start = text.find(marker).unwrap();
        let end = start + text[start..].find(owner_end_marker).unwrap() + owner_end_marker.len();
        let mut declaration = anchors(start, end);
        let owner = if name == "Format" {
            "class:DocumentationProbe.IFormatter"
        } else {
            "class:DocumentationProbe.ProbeController"
        };
        let identity = if name == "Format" {
            interface_symbol.clone()
        } else {
            symbol(name)
        };
        declaration.as_object_mut().unwrap().extend(json!({"schema":crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA,
            "kind":"DECLARATION","declarationKind":"METHOD","resolution":"COMPILER_EXACT",
            "symbolIdentity":identity,"ownerIdentity":owner,"name":name,"jvmDescriptor":descriptor,
            "csharpIdentity":format!("csharp:M:DocumentationProbe.{}.{}(System.String)", if name=="Format" {"IFormatter"} else {"ProbeController"}, name),
            "scope":{"compilation":scope}}).as_object().unwrap().clone());
        let body = &text[start..end];
        let mut events = Vec::new();
        let mut event = |kind: &str, expression: &str, after: usize, target: Option<String>| {
            let local = body[after..].find(expression).unwrap() + after;
            let mut value = anchors(start + local, start + local + expression.len());
            value["kind"] = json!(kind);
            if kind == "IF" {
                value["condition"] = json!("value.Length == 0");
            }
            if let Some(target) = target.clone() {
                value["target"] = json!(target);
                value["resolution"] = json!("COMPILER_EXACT");
                let mut relation = value.clone();
                relation.as_object_mut().unwrap().remove("target");
                relation["schema"] = json!(crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA);
                relation["kind"] = json!("RELATION");
                relation["relationKind"] = json!("CALLS");
                relation["sourceIdentity"] = json!(identity);
                relation["targetIdentity"] = json!(target);
                relation["targetCsharpIdentity"] = json!(if name == "Forward" {
                    "csharp:M:DocumentationProbe.IFormatter.Format(System.String)"
                } else {
                    "csharp:M:DocumentationProbe.ProbeController.Prepare(System.String)"
                });
                relation["scope"] = json!({"compilation":scope});
                facts.push((relation.clone(), digest(&relation).unwrap()));
            }
            events.push(value);
            local + expression.len()
        };
        match name {
            "Render" => {
                event(
                    "LOCAL",
                    "var marker = \"π🙂 <script>{probe()}.mdx/@EXT@\";",
                    0,
                    None,
                );
                event("IF", "if (value.Length == 0) return marker;", 0, None);
                event("RETURN", "return marker;", 0, None);
                event("END", "if (value.Length == 0) return marker;", 0, None);
                let first = event("CALL", "Prepare(\"α🙂\")", 0, Some(symbol("Prepare")));
                event("LOCAL", "var first = Prepare(\"α🙂\");", 0, None);
                event("CALL", "Prepare(\"α🙂\")", first, Some(symbol("Prepare")));
                event("LOCAL", "var second = Prepare(\"α🙂\");", 0, None);
                event("RETURN", "return first;", 0, None);
            }
            "Prepare" => {
                event("LOCAL", "var result = value;", 0, None);
                event("RETURN", "return result;", 0, None);
            }
            "Forward" => {
                event(
                    "CALL",
                    "formatter.Format(value)",
                    0,
                    Some(interface_symbol.clone()),
                );
                event("RETURN", "formatter.Format(value)", 0, None);
            }
            "Unsupported" => {
                event("RETURN", "return \"error\";", 0, None);
            }
            _ => {}
        }
        if name != "Format" {
            declaration["documentation"] = json!({"schema":"codeclew-csharp-documentation-flow/1.0",
                "authority":"ROSLYN_SOURCE_STRUCTURE","events":events,
                "boundaries":if name=="Unsupported" {vec!["EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW","LAMBDA_EXECUTION_NOT_EXPANDED","SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW","NULL_CONDITIONAL_FLOW_REQUIRES_SOURCE_REVIEW"]} else {vec![]}});
        }
        facts.push((declaration.clone(), digest(&declaration).unwrap()));
    }
    let evidence = crate::documentation::analysis::project(
        &service,
        &"a".repeat(40),
        &digest(&service).unwrap(),
        "STATIC",
        "SEMANTIC",
        facts,
        &BTreeMap::from([(file.into(), text.into())]),
        false,
    )
    .unwrap();
    kotlin_check(evidence)
}

fn csharp_selection(checked: &Check, name: &str) -> Selection {
    let id = checked.services["sample"]
        .observations
        .values()
        .find(|row| row.kind == "SYMBOL" && row.normalized["name"] == name)
        .unwrap()
        .id
        .clone();
    Selection {
        id: format!("csharp-{}", name.to_lowercase()),
        expand_source_calls: true,
        ..kotlin_selection(&id, &id)
    }
}

#[test]
fn csharp_common_source_outline_and_exact_calls_expand_one_direct_helper() {
    let checked = csharp_common_check();
    let projection = project(&checked, &[csharp_selection(&checked, "Render")]).unwrap();
    let page = &projection.pages[0];
    let outline = page.endpoint.source_outline.as_ref().unwrap();
    assert_eq!(outline.authority, "ROSLYN_SOURCE_STRUCTURE");
    assert!(
        outline
            .tree
            .as_ref()
            .unwrap()
            .contains("LOCAL Source: var marker")
    );
    assert!(!outline.tree.as_ref().unwrap().contains("PSI"));
    assert!(
        outline
            .events
            .iter()
            .all(|event| event.exact_source.is_some() && event.gaps.is_empty())
    );
    let sites = &page.endpoint.retained_call_sites.as_ref().unwrap().sites;
    assert_eq!(sites.len(), 2);
    assert_eq!(sites[0].expression, "Prepare(\"α🙂\")");
    assert_ne!(sites[0].citation_id, sites[1].citation_id);
    let graph = projection.source_call_graph.as_ref().unwrap();
    assert_eq!(graph.nodes.len(), 2);
    let root = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == page.endpoint.declaration_id)
        .unwrap();
    assert_eq!(root.calls.len(), 2);
    assert_eq!(root.calls[0].target_node, root.calls[1].target_node);
    assert!(root.calls.iter().all(
        |edge| edge.status == "RETAINED_DECLARED_BODY" && edge.runtime_dispatch == "UNRESOLVED"
    ));
    let output = tempfile::tempdir().unwrap();
    super::super::publish::write(output.path(), "snapshot", &projection).unwrap();
    let html = std::fs::read_to_string(output.path().join("csharp-render-endpoint.html")).unwrap();
    assert!(html.contains("Cited C# source outline"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
}

#[test]
fn common_source_invocations_bind_exact_spans_without_mutation_authority() {
    let mut checked = csharp_common_check();
    let selection = csharp_selection(&checked, "Render");
    let evidence = checked.services.get_mut("sample").unwrap();
    let owner = evidence.observations[&selection.endpoint_declaration]
        .symbol
        .clone();
    let relation = evidence
        .observations
        .iter()
        .find(|(_, row)| row.kind == "CALL_RELATION" && row.symbol == owner)
        .unwrap()
        .0
        .clone();
    evidence.observations.remove(&relation);
    let projection = project(&checked, &[selection]).unwrap();
    assert_eq!(projection.schema, SOURCE_INVOCATION_SCHEMA);
    let page = &projection.pages[0];
    let sites = &page.endpoint.retained_call_sites.as_ref().unwrap().sites;
    assert_eq!(sites.len(), 2);
    assert_ne!(sites[0].citation_id, sites[1].citation_id);
    assert_eq!(
        sites
            .iter()
            .filter(|site| page.observations[&site.relation_id].kind == "FLOW")
            .count(),
        1
    );
    assert!(sites.iter().all(|site| site.argument_bindings.is_none()));
    assert!(page.endpoint.steps.is_empty() && page.endpoint.state.is_empty());
    let graph = projection.source_call_graph.as_ref().unwrap();
    assert_eq!(graph.schema, "codeclew-native-source-calls/1.2");
    let root = graph
        .nodes
        .values()
        .find(|node| node.callable.declaration_id == page.endpoint.declaration_id)
        .unwrap();
    assert_eq!(root.calls.len(), 2);
    assert_eq!(root.calls[0].target_node, root.calls[1].target_node);
    assert!(root.calls.iter().all(|edge| edge.call.is_none()
        && edge.conditions.is_none()
        && edge.reachable.is_none()
        && edge.receiver_lineage == "UNRESOLVED"
        && edge.runtime_dispatch == "UNRESOLVED"));
    let output = tempfile::tempdir().unwrap();
    super::super::publish::write(output.path(), "snapshot", &projection).unwrap();
    let html = std::fs::read_to_string(output.path().join("csharp-render-endpoint.html")).unwrap();
    assert!(html.contains("Compiler source event"));
    let mut old_schema = projection.clone();
    old_schema.schema = DECLARATION_SCHEMA.into();
    assert!(
        super::super::publish::write(&output.path().join("old-schema"), "snapshot", &old_schema)
            .is_err()
    );
    let mut forged = projection;
    let site = &mut forged.pages[0]
        .endpoint
        .retained_call_sites
        .as_mut()
        .unwrap()
        .sites[0];
    site.compilation_byte_start += 1;
    assert!(
        super::super::publish::write(&output.path().join("forged"), "snapshot", &forged).is_err()
    );
}

#[test]
fn source_invocation_target_conflicts_never_repair_compiler_relations() {
    let mut checked = csharp_common_check();
    let selection = csharp_selection(&checked, "Render");
    let evidence = checked.services.get_mut("sample").unwrap();
    let owner_symbol = evidence.observations[&selection.endpoint_declaration]
        .symbol
        .clone();
    let flow = evidence
        .observations
        .values_mut()
        .find(|row| {
            row.kind == "FLOW" && row.symbol == owner_symbol && row.normalized["kind"] == "CALL"
        })
        .unwrap();
    let ordinal = flow.normalized["ordinal"].as_u64().unwrap() as usize;
    let target = json!("method:class:Other#Prepare()V");
    flow.normalized["target"] = target.clone();
    flow.digest = digest(&flow.normalized).unwrap();
    let owner = evidence
        .observations
        .get_mut(&selection.endpoint_declaration)
        .unwrap();
    owner.normalized["documentation"]["events"][ordinal]["target"] = target;
    owner.digest = digest(&owner.normalized).unwrap();
    let projection = project(&checked, &[selection]).unwrap();
    let sites = projection.pages[0]
        .endpoint
        .retained_call_sites
        .as_ref()
        .unwrap();
    assert!(sites.sites.is_empty());
    assert!(
        sites
            .gaps
            .iter()
            .any(|gap| gap.code == "SOURCE_INVOCATION_TARGET_CONFLICT")
    );
}

#[test]
fn source_invocations_require_the_complete_owner_event_envelope() {
    let mut checked = csharp_common_check();
    let selection = csharp_selection(&checked, "Render");
    let evidence = checked.services.get_mut("sample").unwrap();
    let owner = evidence.observations[&selection.endpoint_declaration]
        .symbol
        .clone();
    let relation = evidence
        .observations
        .iter()
        .find(|(_, row)| row.kind == "CALL_RELATION" && row.symbol == owner)
        .unwrap()
        .0
        .clone();
    evidence.observations.remove(&relation);
    let local = evidence
        .observations
        .values_mut()
        .find(|row| row.kind == "FLOW" && row.symbol == owner && row.normalized["kind"] == "LOCAL")
        .unwrap();
    local.normalized["unexpected"] = json!("not in the compiler event list");
    local.digest = digest(&local.normalized).unwrap();
    let projection = project(&checked, &[selection]).unwrap();
    let page = &projection.pages[0];
    let sites = &page.endpoint.retained_call_sites.as_ref().unwrap().sites;
    assert_eq!(sites.len(), 1);
    assert!(
        sites
            .iter()
            .all(|site| page.observations[&site.relation_id].kind == "CALL_RELATION")
    );
}

#[test]
fn csharp_interface_and_unsupported_controls_remain_explicit_frontiers() {
    let checked = csharp_common_check();
    let projection = project(
        &checked,
        &[
            csharp_selection(&checked, "Forward"),
            csharp_selection(&checked, "Unsupported"),
        ],
    )
    .unwrap();
    let graph = projection.source_call_graph.as_ref().unwrap();
    let forward = graph
        .nodes
        .values()
        .find(|node| node.callable.symbol.contains("#Forward("))
        .unwrap();
    assert_eq!(forward.calls[0].status, "CALL_TARGET_BODY_NOT_CAPTURED");
    assert!(forward.calls[0].target_node.is_none());
    let outline = projection.pages[1]
        .endpoint
        .source_outline
        .as_ref()
        .unwrap();
    assert!(outline.tree.is_none());
    assert_eq!(outline.gaps[0].code, "SOURCE_OUTLINE_CONTROL_BOUNDARY");
    assert!(
        outline.gaps[0]
            .detail
            .contains("LAMBDA_EXECUTION_NOT_EXPANDED")
    );
}

#[test]
fn csharp_rejected_child_source_stays_local_and_preflight_rejects_changed_source() {
    let mut checked = csharp_common_check();
    let selection = csharp_selection(&checked, "Render");
    let evidence = checked.services.get_mut("sample").unwrap();
    let helper = evidence
        .observations
        .values()
        .find(|row| row.kind == "SYMBOL" && row.normalized["name"] == "Prepare")
        .unwrap()
        .id
        .clone();
    let source = evidence.observations[&helper].source_ids[0].clone();
    evidence.sources.get_mut(&source).unwrap().text_digest = "invalid".into();
    let projection = project(&checked, &[selection]).unwrap();
    let graph = projection.source_call_graph.as_ref().unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert!(
        graph
            .nodes
            .values()
            .next()
            .unwrap()
            .calls
            .iter()
            .all(|edge| edge.status == "BODY_UNAVAILABLE")
    );
    let fresh = csharp_common_check();
    let mut projection = project(&fresh, &[csharp_selection(&fresh, "Render")]).unwrap();
    projection.pages[0]
        .endpoint
        .source_outline
        .as_mut()
        .unwrap()
        .events[0]
        .exact_source
        .as_mut()
        .unwrap()
        .expression
        .push_str("invented");
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("tampered");
    assert!(super::super::publish::write(&output, "snapshot", &projection).is_err());
    assert!(!output.exists());
}
