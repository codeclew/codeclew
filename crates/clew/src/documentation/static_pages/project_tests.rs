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
    let mut normalized = json!({"schema":JAVA_SCHEMA,"kind":"DECLARATION","declarationKind":kind,"symbolIdentity":symbol,"ownerIdentity":format!("class:{owner}"),"name":name,"scope":"compile-scope","jvmDescriptor":if kind == "FIELD" { "Ljava/util/concurrent/BlockingQueue;" } else { "()V" }});
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
