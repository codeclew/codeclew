use super::super::model::{Observation, ServiceEvidence};
use super::*;
use crate::canonical;
use tree_sitter::{Node, Parser};

fn evidence(text: &str) -> ServiceEvidence {
    let mut e = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: "example".into(),
        revision: "fixture-revision".into(),
        service_digest: "fixture".into(),
        extractor: "test-retained-source".into(),
        runtime_mode: "COMMITTED_SOURCE_NO_BUILD".into(),
        coverage: "SYNTAX".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    };
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(text, None).unwrap();
    fn visit(n: Node<'_>, text: &str, owner: &str, e: &mut ServiceEvidence) {
        let mut next = owner.to_owned();
        if n.kind() == "class_declaration" {
            next = format!(
                "example.{}",
                &text[n.child_by_field_name("name").unwrap().byte_range()]
            );
        }
        if matches!(n.kind(), "method_declaration" | "constructor_declaration") {
            let name = &text[n.child_by_field_name("name").unwrap().byte_range()];
            let id = format!("source-{}", e.sources.len());
            let exact = &text[n.byte_range()];
            e.sources.insert(
                id.clone(),
                Source {
                    id: id.clone(),
                    service: e.service.clone(),
                    revision: e.revision.clone(),
                    file: "Example.java".into(),
                    start_line: n.start_position().row as u64 + 1,
                    end_line: n.end_position().row as u64 + 1,
                    text: exact.into(),
                    text_digest: canonical::hash_bytes(exact.as_bytes()),
                    evidence_digest: "fixture".into(),
                    authority: "EXACT_SNAPSHOT_TEXT".into(),
                    occurrence: None,
                    url: None,
                },
            );
            e.observations.insert(id.clone(),Observation{id:id.clone(),kind:"SYMBOL".into(),service:e.service.clone(),symbol:format!("{owner}.{name}"),normalized:json!({"kind":"DECLARATION","authority":"SYNTAX","name":name,"ownerIdentity":format!("class:{owner}")}),digest:"fixture".into(),source_ids:vec![id]});
        }
        let mut cursor = n.walk();
        for c in n.named_children(&mut cursor) {
            visit(c, text, &next, e);
        }
    }
    visit(tree.root_node(), text, "", &mut e);
    e
}
fn fixture() -> ServiceEvidence {
    evidence(include_str!(
        "../../../../../fixtures/flow-dsl/Example.java"
    ))
}
fn profile(queue: bool) -> Profile {
    serde_json::from_str(if queue {
        include_str!("../../../../../fixtures/flow-dsl/queue-profile.json")
    } else {
        include_str!("../../../../../fixtures/flow-dsl/builder-profile.json")
    })
    .unwrap()
}

#[test]
fn two_flow_families_preserve_semantic_distinctions() {
    let e = fixture();
    let builder = extract::project(&e, "original-snapshot", &profile(false)).unwrap();
    let groups: Vec<_> = builder
        .items
        .iter()
        .filter(|i| i.kind == "callback-group")
        .collect();
    assert_eq!(groups.len(), 8);
    assert_eq!(groups[2].members, vec!["this::fetchA", "this::fetchB"]);
    assert!(!builder.items.iter().any(|i| {
        i.members
            .iter()
            .any(|m| m.contains("notReal") || m.contains("unrelated"))
    }));
    assert!(builder.phases.len() >= 5 && builder.phases.len() <= 7);
    let queue = extract::project(&e, "original-snapshot", &profile(true)).unwrap();
    assert_eq!(
        queue
            .items
            .iter()
            .find(|i| i.kind == "initial-queue")
            .unwrap()
            .members,
        vec!["INIT", "DECISION"]
    );
    let registry: Vec<_> = queue
        .items
        .iter()
        .filter(|i| i.kind == "registry-binding")
        .collect();
    assert_eq!(
        registry[0].members,
        vec!["DECISION", "QueueContext::decision"]
    );
    assert_eq!(registry[1].members[0], "INIT");
    assert!(
        registry
            .iter()
            .all(|i| i.limitations.iter().any(|l| l.contains("Unordered")))
    );
    let selections: Vec<_> = queue
        .items
        .iter()
        .filter(|i| i.kind == "selected-next-operations")
        .collect();
    assert_eq!(selections.len(), 3);
    assert!(selections.iter().all(|i| !i.conditions.is_empty()));
    assert_eq!(selections[1].members, vec!["SEND", "FINISH"]);
    assert!(selections[1].conditions[0].starts_with("else of"));
    assert!(
        !queue
            .items
            .iter()
            .any(|i| i.members.contains(&"GHOST".into()) || i.members.contains(&"FAKE".into()))
    );
    assert!(queue.items.iter().any(|i| i.kind == "mapping"
        && i.expression.contains("selection")
        && i.conditions.is_empty()));
    assert!(
        queue
            .items
            .iter()
            .any(|i| i.kind == "opaque-external-call" && i.limitations[0].contains("opaque"))
    );
    assert!(
        queue
            .items
            .iter()
            .any(|i| i.kind == "gap" && i.expression.starts_with("while"))
    );
    assert!(
        queue
            .items
            .iter()
            .any(|i| i.kind == "gap" && i.expression.contains("computeNext"))
    );
    assert!(
        queue
            .items
            .iter()
            .filter(|i| i.kind == "condition")
            .any(|i| i
                .limitations
                .iter()
                .any(|l| l.contains("helper semantics unresolved")))
    );
}
#[test]
fn unsupported_control_and_mixed_scope_are_not_promoted() {
    let mut p = profile(true);
    let mut e = fixture();
    let decision = e
        .sources
        .values_mut()
        .find(|s| s.text.starts_with("Object decision()"))
        .unwrap();
    let decision_id = decision.id.clone();
    decision.text="Object decision() { boolean accepted = flag && odmClient.evaluate(context); Object next = flag ? queue.addOperations(Operation.SEND) : queue.addOperations(Operation.FINISH); return computeNext(); }".into();
    decision.text_digest = canonical::hash_bytes(decision.text.as_bytes());
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .unwrap();
    assert!(
        !parser
            .parse(&decision.text, None)
            .unwrap()
            .root_node()
            .has_error()
    );
    let projection = extract::project(&e, "snapshot", &p).unwrap();
    assert!(
        !projection
            .items
            .iter()
            .any(|i| i.kind == "selected-next-operations")
    );
    assert!(
        !projection
            .items
            .iter()
            .any(|i| i.kind == "opaque-external-call" && i.reference.source_id == decision_id)
    );
    assert!(projection.items.iter().filter(|i| i.kind == "gap").count() >= 2);
    p.context.as_mut().unwrap().owner = "example.Unrelated".into();
    let projection = extract::project(&fixture(), "snapshot", &p).unwrap();
    assert!(
        projection
            .items
            .iter()
            .any(|i| i.members.contains(&"FAKE".into()))
    ); // explicit context selection, not name guessing
    e.sources.values_mut().next().unwrap().revision = "other-revision".into();
    assert!(extract::project(&e, "snapshot", &p).is_err());
}
#[test]
fn publication_is_inert_has_parity_and_preserves_existing_files() {
    let mut p = profile(false);
    p.title = "A <script> & {danger()} request".into();
    p.labels.insert(
        "readHistory".into(),
        "Read {danger()} <img src=x> & data".into(),
    );
    let projection = extract::project(&fixture(), "snapshot", &p).unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("manual.txt"), "authored").unwrap();
    publish::write(temp.path(), &projection).unwrap();
    let html = std::fs::read_to_string(temp.path().join("page.html")).unwrap();
    let mdx = std::fs::read_to_string(temp.path().join("page.mdx")).unwrap();
    let body = publish::body(&projection);
    assert!(html.contains(&body) && mdx.contains(&body));
    assert!(mdx.contains("&#123;danger()&#125;"));
    assert!(!mdx.contains("<script>"));
    assert!(!html.contains("<script"));
    assert!(publish::write(temp.path(), &projection).is_err());
    assert_eq!(
        std::fs::read_to_string(temp.path().join("manual.txt")).unwrap(),
        "authored"
    );
}
#[test]
fn overloaded_method_and_conditional_factory_fail_closed() {
    let mut e = fixture();
    let o = e
        .observations
        .values()
        .find(|o| {
            o.normalized["name"] == "construct"
                && o.normalized["ownerIdentity"] == "class:example.BuilderFactory"
        })
        .unwrap()
        .clone();
    let mut duplicate = o.clone();
    duplicate.id = "duplicate".into();
    e.observations.insert(duplicate.id.clone(), duplicate);
    assert!(extract::project(&e, "snapshot", &profile(false)).is_err());
    let mut e = fixture();
    let s = e
        .sources
        .values_mut()
        .find(|s| s.text.contains("return TaskType.BUILDER_EXAMPLE"))
        .unwrap();
    s.text =
        "TaskType getTaskType(){ if (ready) return TaskType.BUILDER_EXAMPLE; return resolve(); }"
            .into();
    s.text_digest = canonical::hash_bytes(s.text.as_bytes());
    let p = extract::project(&e, "snapshot", &profile(false)).unwrap();
    assert!(!p.items.iter().any(|i| i.kind == "factory-task-type"));
    assert!(
        p.items
            .iter()
            .any(|i| i.kind == "gap" && i.label.contains("factory"))
    );
}

#[test]
fn compiler_only_callbacks_and_construction_control_are_bounded() {
    let mut e = fixture();
    for o in e.observations.values_mut() {
        o.normalized["authority"] = json!("COMPILER");
        o.normalized["scope"] = json!("main");
    }
    let mut p = profile(true);
    for binding in [&mut p.construction]
        .into_iter()
        .chain(p.factory.iter_mut())
        .chain(p.registry.iter_mut())
    {
        let o = e
            .observations
            .values()
            .find(|o| {
                o.normalized["ownerIdentity"] == format!("class:{}", binding.owner)
                    && o.normalized["name"] == binding.method
            })
            .unwrap();
        binding.compiler_symbol = Some(o.symbol.clone());
        binding.scope = Some("main".into());
    }
    let projection = extract::project(&e, "snapshot", &p).unwrap();
    assert!(
        projection
            .items
            .iter()
            .any(|i| i.kind == "selected-next-operations"
                && i.reference.binding_authority == "COMPILER_DECLARATION_MATCHED")
    );
    let mut e = fixture();
    let s = e
        .sources
        .values_mut()
        .find(|s| s.text.starts_with("void construct"))
        .unwrap();
    s.text="void construct() { root = FlowBuilder.build(this::init); boolean ready = enabled && root.then(this::fetchA).isReady(); }".into();
    s.text_digest = canonical::hash_bytes(s.text.as_bytes());
    let projection = extract::project(&e, "snapshot", &profile(false)).unwrap();
    assert_eq!(
        projection
            .items
            .iter()
            .filter(|i| i.kind == "callback-group")
            .count(),
        1
    );
    assert!(
        projection
            .items
            .iter()
            .any(|i| i.kind == "gap" && i.label.contains("Conditional"))
    );
}

#[test]
fn compiler_call_relations_require_exact_occurrence_not_same_line_text() {
    use super::super::model::SourceOccurrence;
    let mut e = fixture();
    let mut p = profile(true);
    p.effects
        .iter_mut()
        .find(|e| e.call.receiver == "odmClient")
        .unwrap()
        .call
        .compiler_target = Some("method:external".into());
    let method = e
        .sources
        .values_mut()
        .find(|s| s.text.starts_with("void send()"))
        .unwrap();
    method.text =
        "void send() { odmClient.evaluate(context); odmClient.evaluate(context); }".into();
    method.text_digest = canonical::hash_bytes(method.text.as_bytes());
    method.end_line = method.start_line;
    method.occurrence = Some(SourceOccurrence {
        snapshot: "capture".into(),
        blob: "blob".into(),
        start_byte: 100,
        end_byte: 100 + method.text.len(),
    });
    let source = method.clone();
    let call = "odmClient.evaluate(context)";
    for (i, (offset, _)) in source.text.match_indices(call).enumerate() {
        let id = format!("relation-source-{i}");
        let mut src = source.clone();
        src.id = id.clone();
        src.text = call.into();
        src.text_digest = canonical::hash_bytes(call.as_bytes());
        src.occurrence.as_mut().unwrap().start_byte = 100 + offset;
        src.occurrence.as_mut().unwrap().end_byte = 100 + offset + call.len();
        e.sources.insert(id.clone(), src);
        e.observations.insert(id.clone(),Observation{id:id.clone(),kind:"CALL_RELATION".into(),service:e.service.clone(),symbol:"send".into(),normalized:json!({"relationKind":"CALLS","targetIdentity":if i==0{"method:unrelated"}else{"method:external"}}),digest:"fixture".into(),source_ids:vec![id]});
    }
    let projection = extract::project(&e, "snapshot", &p).unwrap();
    let calls: Vec<_> = projection
        .items
        .iter()
        .filter(|i| i.kind == "opaque-external-call" && i.expression == call)
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].reference.compiler_call_relation.as_deref(),
        Some("relation-source-1")
    );
    for src in e
        .sources
        .values_mut()
        .filter(|s| s.id.starts_with("relation-source"))
    {
        src.occurrence = None;
    }
    let projection = extract::project(&e, "snapshot", &p).unwrap();
    let calls: Vec<_> = projection
        .items
        .iter()
        .filter(|i| i.kind == "opaque-external-call" && i.expression == call)
        .collect();
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|i| i.reference.call_target_authority == "DECLARED_RECEIVER_AST_SYNTAX")
    );
}
