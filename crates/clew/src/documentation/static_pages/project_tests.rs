use super::*;
use crate::documentation::{check, model::SourceOccurrence};
use serde_json::json;

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
            .filter(|c| c.call.expression == "child.submit(task)" && c.target_node.is_some())
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
        .find(|e| e.occurrence_path == original.occurrence_path)
        .unwrap();
    assert!(
        original_edge
            .conditions
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
