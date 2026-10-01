//! A compact process view cannot authorize provider facts hidden by its projection.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use serde_json::json;
use support::Fixture;

#[test]
fn process_profile_defers_provider_authority_until_recorded_expansion() {
    let f = Fixture::new();
    let source = f.service("orders");
    let definition=f.input("process.json",&json!({"schema":"codeclew-documentation-process/1.0","id":"quantity","title":"Quantity flow","summary":"One explicitly selected method.","root":{"service":"orders","selector":{"language":"java","owner":"Orders","name":"reserve","parameterTypes":["int"]}},"interactions":[],"maxDepth":3,"maxNodes":32,"process":{"scope":"One method","participants":["orders"],"objects":[],"trigger":"An unspecified caller provides quantity.","outcomes":["Returns a quantity."],"linkedSubviews":[]}}));
    let before = f.ok(&["docs", "process", "list"]);
    f.ok(&[
        "docs",
        "process",
        "put",
        "--input",
        definition.to_str().unwrap(),
        "--expected-input-digest",
        before["inputDigest"].as_str().unwrap(),
    ]);
    f.checked();
    std::fs::rename(&source, source.with_extension("offline")).unwrap();
    let first = f.ok(&[
        "docs",
        "process",
        "prepare",
        "--id",
        "quantity",
        "--overview",
    ]);
    assert_eq!(first["contextProfile"], "process-v1");
    let work = first["work"].as_str().unwrap().to_owned();
    let mut page = first;
    let mut rows = Vec::new();
    loop {
        assert!(page["omitted"].as_array().unwrap().is_empty());
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        let Some(cursor) = page["nextCursor"].as_str() else {
            break;
        };
        let selection = f.input("next-page.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &work,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let deferred = rows
        .iter()
        .find(|row| row["kind"] == "CALLABLE_SUMMARY")
        .unwrap()["record"]["fullRecordReference"]
        .as_str()
        .unwrap();
    assert!(!rows.iter().any(|row| row["reference"] == deferred));
    let proposal=f.input("proposal.json",&json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{"entrypoint":"scenario:quantity","title":"Quantity method","summary":{"text":"The selected method returns a quantity.","evidence":[deferred]},"steps":[]}],"gaps":{},"uncertainties":["Call targets and runtime activation are not established by syntax evidence."]}));
    let rejected = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        proposal.to_str().unwrap(),
    ]);
    assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
    assert!(rejected.to_string().contains("not supplied"), "{rejected}");
    let selection = f.input("expand.json", &json!({"references":[deferred]}));
    let expanded = f.ok(&[
        "docs",
        "work",
        "expand",
        "--work",
        &work,
        "--input",
        selection.to_str().unwrap(),
    ]);
    let full = expanded["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["reference"] == deferred)
        .unwrap();
    assert_eq!(full["record"]["kind"], "SYMBOL");
    assert!(full["record"]["normalized"].get("documentation").is_some());
    let accepted = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        proposal.to_str().unwrap(),
    ]);
    assert!(
        accepted["status"].as_str().unwrap().starts_with("READY_"),
        "{accepted}"
    );
    // The overview host contract accepts the supported summary above but not
    // sequence, contract, note-assessment or typed-view content in that slot.
    let good: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&proposal).unwrap()).unwrap();
    for (field, value) in [
        (
            "steps",
            json!([{"kind":"note","meaning":{"text":"A step","evidence":[deferred]}}]),
        ),
        (
            "contracts",
            json!([{"title":"Payload","kind":"payload","rows":[{"label":"Quantity","description":{"text":"A quantity","evidence":[deferred]}}]}]),
        ),
        (
            "assessment",
            json!({"outcome":"UNKNOWN","period":"Current"}),
        ),
        ("dataflow", json!({"nodes":[],"edges":[]})),
    ] {
        let mut invalid = good.clone();
        invalid["operations"][0][field] = value;
        let path = f.input(&format!("invalid-overview-{field}.json"), &invalid);
        let rejected = f.ok(&[
            "docs",
            "proposal",
            "submit",
            "--work",
            &work,
            "--input",
            path.to_str().unwrap(),
        ]);
        assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
        assert!(
            rejected["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["kind"] == "DIAGNOSTIC"
                    && row["record"]["code"] == "INVALID_PROPOSAL"),
            "{rejected}"
        );
    }
    for key in ["activation", "provider-behavior", "runtime"] {
        let mut invalid = good.clone();
        invalid["gaps"] = json!({(key):"This limitation is not an operation root."});
        let path = f.input(&format!("invalid-gap-{key}.json"), &invalid);
        let rejected = f.ok(&[
            "docs",
            "proposal",
            "submit",
            "--work",
            &work,
            "--input",
            path.to_str().unwrap(),
        ]);
        assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
        assert!(
            rejected
                .to_string()
                .contains("gap requires an entrypoint reference or its scenario subject"),
            "{rejected}"
        );
    }
    let mut duplicate = good.clone();
    duplicate["gaps"] = json!({"scenario:quantity":"Runtime activation is not established."});
    let path = f.input("duplicate-overview-gap.json", &duplicate);
    let rejected = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        path.to_str().unwrap(),
    ]);
    assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
    assert!(
        rejected
            .to_string()
            .contains("gap is duplicate, out of scope, or lacks an actionable reason"),
        "{rejected}"
    );
    let mut supported = good;
    supported["operations"][0]["summary"]["uncertainty"] =
        json!("Runtime activation and provider behavior remain unverified.");
    let path = f.input("supported-overview-uncertainties.json", &supported);
    let limited = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        path.to_str().unwrap(),
    ]);
    assert_eq!(limited["status"], "READY_WITH_LIMITATIONS", "{limited}");
    let path = f.input("unsupported-whole-overview.json", &json!({
        "schema":"codeclew-documentation-proposal/1.0", "operations":[],
        "gaps":{"scenario:quantity":"No supported overview is offered; further retained evidence is needed."},
    }));
    let unavailable = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        path.to_str().unwrap(),
    ]);
    assert_eq!(
        unavailable["status"], "READY_WITH_LIMITATIONS",
        "{unavailable}"
    );
}

#[test]
fn process_profile_cannot_be_requested_for_service_sections() {
    let f = Fixture::new();
    f.service("orders");
    f.checked();
    let request=f.input("request.json",&json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers","entrypoint":"section-overview","contextProfile":"process-v1"}));
    let (code, error) = f.run(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(
        error.to_string().contains("CONTEXT_PROFILE_INCOMPATIBLE"),
        "{error}"
    );
}

#[test]
fn saved_scenario_question_prepares_full_graph_work_from_frozen_selection() {
    let f = Fixture::new();
    let source_repo = f.service("orders");
    f.service("inventory");
    let source_text = "public class Orders {\n  public int reserve(int quantity) {\n    if (quantity > 0) {\n      int normalized = normalize(quantity);\n      return finish(normalized);\n    }\n    return -1;\n  }\n  private int normalize(int quantity) { return quantity + 1; }\n  private int finish(int quantity) { return quantity * 2; }\n}\n";
    std::fs::write(source_repo.join("Orders.java"), source_text).unwrap();
    support::commit(&source_repo);

    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    let initial_digest = repo.input_digest().unwrap();
    repo.put(
        "catalog/interactions/handoff.json",
        &json!({
            "schema":"codeclew-documentation-interaction/1.0",
            "id":"handoff",
            "title":"Declared handoff",
            "from":{"service":"orders"},
            "to":{"service":"inventory"},
            "transport":{"kind":"http","method":"POST","path":"/handoff"},
            "declaration":{"origin":"human","rationale":"A declared continuation for the saved-process explanation."}
        }),
        Some(&initial_digest),
    )
    .unwrap();
    let process = f.input(
        "saved-process.json",
        &json!({
            "schema":"codeclew-documentation-process/1.0",
            "id":"quantity",
            "title":"Quantity flow",
            "summary":"Explain the selected operation and declared continuation.",
            "root":{"service":"orders","selector":{"language":"java","scope":":main","owner":"Orders","name":"reserve","parameterTypes":["int"]}},
            "interactions":["handoff"],
            "maxDepth":1,
            "maxNodes":1,
            "process":{"scope":"One selected method","participants":["orders","inventory"],"objects":[],"trigger":"A request supplies a quantity.","outcomes":["A result is returned."],"linkedSubviews":[]}
        }),
    );
    let expected = f.ok(&["docs", "process", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "process",
        "put",
        "--input",
        process.to_str().unwrap(),
        "--expected-input-digest",
        &expected,
    ]);
    let (check_code, check_report) = f.run(&["docs", "check"]);
    assert!(matches!(check_code, 0 | 3 | 4), "{check_report}");
    let raw_snapshot = check_report["snapshot"].as_str().unwrap();
    // The public source-syntax fixture has no compiler scope or exact target
    // facts. Add three retained compiler-shaped declarations to the immutable
    // test snapshot so the CLI path exercises the exact-scope graph contract.
    let mut checked =
        clew::documentation::check::Check::load_snapshot(&repo, raw_snapshot).unwrap();
    let evidence = checked.services.get_mut("orders").unwrap();
    let source_id = "orders-full-source".to_owned();
    let revision = evidence.revision.clone();
    evidence.sources.insert(
        source_id.clone(),
        clew::documentation::model::Source {
            id: source_id.clone(),
            service: "orders".into(),
            revision,
            file: "Orders.java".into(),
            start_line: 1,
            end_line: source_text.lines().count() as u64,
            text: source_text.into(),
            text_digest: clew::canonical::hash_bytes(source_text.as_bytes()),
            evidence_digest: clew::canonical::hash(&source_text).unwrap(),
            authority: "RETAINED_SOURCE".into(),
            occurrence: None,
            url: None,
        },
    );
    for (id, name, modifiers) in [
        ("graph-reserve", "reserve", json!(["PUBLIC"])),
        ("graph-normalize", "normalize", json!(["PRIVATE"])),
        ("graph-finish", "finish", json!(["PRIVATE"])),
    ] {
        let identity = format!("method:class:Orders#{name}(I)I");
        let normalized = json!({
            "schema":"codeclew-java-compiler-fact/1.0",
            "declarationKind":"METHOD",
            "symbolIdentity":identity,
            "ownerIdentity":"class:Orders",
            "name":name,
            "scope":":main",
            "jvmDescriptor":"(I)I",
            "modifiers":modifiers,
            "annotations":[],
            "documentation":{"events":[],"parameterTypes":["int"]}
        });
        let observation = clew::documentation::model::Observation {
            id: id.into(),
            kind: "SYMBOL".into(),
            service: "orders".into(),
            symbol: identity,
            digest: clew::canonical::hash(&normalized).unwrap(),
            normalized,
            source_ids: vec![source_id.clone()],
        };
        evidence
            .observations
            .insert(observation.id.clone(), observation.clone());
        checked
            .dependencies
            .insert(observation.id.clone(), observation);
    }
    let snapshot_handle = checked.save_snapshot(&repo).unwrap();

    let prepare = [
        "docs",
        "process",
        "prepare",
        "--id",
        "quantity",
        "--question",
        "How does this quantity operation reach its result?",
        "--language",
        "ru",
        "--snapshot",
        &snapshot_handle,
    ];
    let first = f.ok(&prepare);
    assert_eq!(first["contextProfile"], "process-graph-v1");
    assert_eq!(first["snapshot"], snapshot_handle);
    let work = first["work"].as_str().unwrap().to_owned();
    let repeated = f.ok(&prepare);
    assert_eq!(repeated["work"], work, "identical input must reuse Work");

    let mut page = first;
    let mut rows = Vec::new();
    loop {
        assert!(page["omitted"].as_array().unwrap().is_empty());
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        let Some(cursor) = page["nextCursor"].as_str() else {
            break;
        };
        let selection = f.input("process-graph-page.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &work,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let process_context = rows
        .iter()
        .find(|row| row["kind"] == "PROCESS_CONTEXT_PACKET")
        .expect("process graph packet is retained in Work")
        .clone();
    assert_eq!(process_context["record"]["profile"], "process-graph-v1");
    assert_eq!(
        process_context["record"]["root"]["symbolIdentity"],
        "method:class:Orders#reserve(I)I"
    );
    let nodes = process_context["record"]["callGraph"]["nodes"]
        .as_array()
        .unwrap();
    assert!(
        nodes.len() > 1,
        "maxNodes=1 is scenario context only; retained record: {}",
        process_context["record"]
    );
    assert!(
        process_context["record"]["callGraph"]["nodes"]
            .to_string()
            .contains("normalize")
    );
    assert!(
        process_context["record"]["callGraph"]["nodes"]
            .to_string()
            .contains("finish")
    );
    let frozen_scenario = rows
        .iter()
        .find(|row| row["record"]["kind"] == "SCENARIO_SELECTION")
        .expect("frozen scenario dependency is included");
    assert_eq!(
        frozen_scenario["record"]["normalized"]["title"],
        "Quantity flow"
    );
    let declared_interaction = rows
        .iter()
        .find(|row| row["record"]["kind"] == "DECLARED_INTERACTION")
        .expect("selected declared interaction dependency is included");
    assert_eq!(
        declared_interaction["record"]["normalized"]["id"],
        "handoff"
    );
}

#[test]
fn source_update_preserves_saved_scope_protected_note_and_old_work() {
    let f = Fixture::new();
    let source_repo = f.service("orders");
    let source_s0 = "public class Orders {\n  public int reserve(int quantity) { return normalize(quantity); }\n  private int normalize(int quantity) { return quantity; }\n}\n";
    std::fs::write(source_repo.join("Orders.java"), source_s0).unwrap();
    support::commit(&source_repo);

    let process = f.input(
        "quantity-process.json",
        &json!({
            "schema":"codeclew-documentation-process/1.0",
            "id":"quantity",
            "title":"Quantity flow",
            "summary":"Explain the manually selected quantity operation.",
            "root":{"service":"orders","selector":{"language":"java","owner":"Orders","name":"reserve","parameterTypes":["int"]}},
            "interactions":[],
            "maxDepth":1,
            "maxNodes":1,
            "process":{"scope":"Manual scope: one reservation quantity through normalization","participants":["orders"],"objects":[],"trigger":"A caller supplies one quantity.","outcomes":["The selected operation returns a quantity."],"linkedSubviews":[]}
        }),
    );
    let expected_input = f.ok(&["docs", "process", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "process",
        "put",
        "--input",
        process.to_str().unwrap(),
        "--expected-input-digest",
        &expected_input,
    ]);

    let note_text = "# Historical quantity note\n\n| Period | Reported behavior | Historical source URL | Maintainer context |\n| --- | --- | --- | --- |\n| Earlier implementation | The quantity was normalized before return. | https://history.example.invalid/orders/quantity | fixture-maintainer: preserve this imported human context; it remains unverified. |\n";
    let note_source = f.temp.path().join("quantity-history.md");
    std::fs::write(&note_source, note_text.as_bytes()).unwrap();
    let note_association = f.input(
        "quantity-note.json",
        &json!({
            "schema":"codeclew-documentation-note-association/1.0",
            "id":"quantity-history",
            "title":"Historical quantity context",
            "service":"orders",
            "path":"notes/quantity-history.md",
            "targets":["scenario:quantity"],
            "classification":"historical-context",
            "period":"Historical behavior; date not independently verified",
            "tags":["history","quantity"],
            "metadata":{"author":"fixture-maintainer","authority":"HUMAN_OR_IMPORTED_UNVERIFIED"}
        }),
    );
    let expected_input = f.ok(&["docs", "note", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "note",
        "import",
        "--source",
        note_source.to_str().unwrap(),
        "--input",
        note_association.to_str().unwrap(),
        "--expected-input-digest",
        &expected_input,
    ]);

    let process_path = f.docs.join("scenarios/quantity.yaml");
    let association_path = f.docs.join("catalog/notes/quantity-history.json");
    let protected_process = std::fs::read(&process_path).unwrap();
    let protected_association = std::fs::read(&association_path).unwrap();
    let protected_note = std::fs::read(f.docs.join("notes/quantity-history.md")).unwrap();

    let (check_code, check_report) = f.run(&["docs", "check"]);
    assert!(matches!(check_code, 0 | 3 | 4), "{check_report}");
    let snapshot_s0 = check_report["snapshot"].as_str().unwrap().to_owned();
    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    let s0 = clew::documentation::check::Check::load_snapshot(&repo, &snapshot_s0).unwrap();
    assert!(
        s0.services["orders"]
            .sources
            .values()
            .any(|source| source.text.contains("return normalize(quantity);"))
    );

    let prepare = [
        "docs",
        "process",
        "prepare",
        "--id",
        "quantity",
        "--overview",
        "--snapshot",
        &snapshot_s0,
    ];
    let first = f.ok(&prepare);
    assert_eq!(first["contextProfile"], "process-v1");
    let work0_id = first["work"].as_str().unwrap().to_owned();
    let work0_path = f.docs.join(format!(".codeclew/work/{work0_id}/work.json"));
    let work0_bytes = std::fs::read(&work0_path).unwrap();
    let work0 = clew::documentation::work::load(&repo, &work0_id).unwrap();
    assert_eq!(
        serde_json::to_value(&work0.checked).unwrap(),
        serde_json::to_value(&s0).unwrap()
    );

    let saved = &work0.checked.dependencies["process:quantity"].normalized["definition"];
    assert_eq!(saved["title"], "Quantity flow");
    assert_eq!(
        saved["summary"],
        "Explain the manually selected quantity operation."
    );
    assert_eq!(saved["root"]["service"], "orders");
    assert_eq!(saved["root"]["selector"]["language"], "java");
    assert_eq!(saved["root"]["selector"]["owner"], "Orders");
    assert_eq!(saved["root"]["selector"]["name"], "reserve");
    assert_eq!(saved["root"]["selector"]["parameterTypes"], json!(["int"]));
    assert!(saved["root"]["selector"].get("scope").is_none());
    assert_eq!(
        saved["process"]["scope"],
        "Manual scope: one reservation quantity through normalization"
    );
    let note0 = &work0.checked.dependencies["note:quantity-history"];
    assert_eq!(note0.kind, "NOTE_ASSOCIATION");
    assert_eq!(
        note0.normalized["authority"],
        "HUMAN_OR_IMPORTED_UNVERIFIED"
    );
    assert_eq!(
        note0.normalized["association"]["metadata"]["author"],
        "fixture-maintainer"
    );
    assert_eq!(
        note0.normalized["association"]["metadata"]["authority"],
        "HUMAN_OR_IMPORTED_UNVERIFIED"
    );
    assert_eq!(note0.normalized["original"]["text"], note_text);
    assert!(
        note0.normalized["original"]["text"]
            .as_str()
            .unwrap()
            .contains("https://history.example.invalid/orders/quantity")
    );
    assert!(
        note0.normalized["original"]["text"]
            .as_str()
            .unwrap()
            .contains("| Historical source URL | Maintainer context |")
    );
    assert!(
        note0.source_ids.is_empty(),
        "a protected human note has no source authority"
    );

    let source_s1 = "public class Orders {\n  public int reserve(int quantity) { return normalize(quantity) + 1; }\n  private int normalize(int quantity) { return quantity; }\n}\n";
    std::fs::write(source_repo.join("Orders.java"), source_s1).unwrap();
    support::commit(&source_repo);
    let (check_code, check_report) = f.run(&["docs", "check"]);
    assert!(matches!(check_code, 0 | 3 | 4), "{check_report}");
    let snapshot_s1 = check_report["snapshot"].as_str().unwrap().to_owned();
    assert_ne!(snapshot_s1, snapshot_s0);
    assert_eq!(std::fs::read(&process_path).unwrap(), protected_process);
    assert_eq!(
        std::fs::read(&association_path).unwrap(),
        protected_association
    );
    assert_eq!(
        std::fs::read(f.docs.join("notes/quantity-history.md")).unwrap(),
        protected_note
    );

    let s1 = clew::documentation::check::Check::load_snapshot(&repo, &snapshot_s1).unwrap();
    let source0 = s0.services["orders"]
        .sources
        .values()
        .find(|source| source.text.contains("return normalize(quantity);"))
        .unwrap();
    let source1 = s1.services["orders"]
        .sources
        .values()
        .find(|source| source.text.contains("return normalize(quantity) + 1;"))
        .unwrap();
    assert_ne!(source0.revision, source1.revision);
    assert_ne!(source0.text, source1.text);
    assert!(source1.text.contains("return normalize(quantity) + 1;"));

    let prepare_s1 = [
        "docs",
        "process",
        "prepare",
        "--id",
        "quantity",
        "--overview",
        "--snapshot",
        &snapshot_s1,
    ];
    let second = f.ok(&prepare_s1);
    assert_eq!(second["contextProfile"], "process-v1");
    let work1_id = second["work"].as_str().unwrap();
    assert_ne!(work1_id, work0_id);
    let work1 = clew::documentation::work::load(&repo, work1_id).unwrap();
    assert_eq!(
        work1.checked.services["orders"].revision,
        s1.services["orders"].revision
    );
    assert!(
        work1.checked.services["orders"]
            .sources
            .values()
            .any(|source| source.text.contains("return normalize(quantity) + 1;"))
    );
    assert_eq!(
        work1.checked.dependencies["process:quantity"].normalized["definition"],
        work0.checked.dependencies["process:quantity"].normalized["definition"]
    );
    let note1 = &work1.checked.dependencies["note:quantity-history"];
    assert_eq!(
        serde_json::to_value(note1).unwrap(),
        serde_json::to_value(note0).unwrap()
    );
    assert_eq!(
        note1.normalized["authority"],
        "HUMAN_OR_IMPORTED_UNVERIFIED"
    );
    assert!(note1.source_ids.is_empty());

    let repeated = f.ok(&prepare_s1);
    assert_eq!(repeated["work"], work1_id);
    assert_eq!(std::fs::read(&work0_path).unwrap(), work0_bytes);
    let old_work_after = clew::documentation::work::load(&repo, &work0_id).unwrap();
    assert_eq!(
        serde_json::to_value(&old_work_after.checked).unwrap(),
        serde_json::to_value(&s0).unwrap()
    );
}
