use super::process_extract::{Budgets, Query, extract};
use super::{check::Check, model::ServiceEvidence};
use serde_json::{Value, json};

fn fixture(provider: &str) -> Check {
    let mut evidence: ServiceEvidence = serde_json::from_value(json!({
        "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
        "serviceDigest":"digest","extractor":provider,"runtimeMode":"RETAINED_TEST_FIXTURE","coverage":"PARTIAL",
        "boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}
    })).unwrap();
    for (id, symbol, events) in [
        (
            "root",
            "method:class:Example#root()V",
            vec![
                json!({"kind":"IF","condition":"ready"}),
                json!({"kind":"CALL","target":"method:class:Example#helper()V","resolution":"COMPILER_EXACT"}),
                json!({"kind":"ELSE"}),
                json!({"kind":"THROW"}),
                json!({"kind":"END"}),
                json!({"kind":"CALL","target":"external:send","resolution":"UNKNOWN"}),
            ],
        ),
        (
            "helper",
            "method:class:Example#helper()V",
            vec![
                json!({"kind":"CALL","target":"method:class:Example#root()V","resolution":"COMPILER_EXACT"}),
                json!({"kind":"RETURN"}),
            ],
        ),
        (
            "unrelated",
            "method:class:Example#unrelated()V",
            vec![json!({"kind":"RETURN"})],
        ),
    ] {
        let text = if provider == "JAVAC" {
            format!("void {id}() {{}}")
        } else {
            format!("fun {id}() {{}}")
        };
        evidence.sources.insert(id.into(), serde_json::from_value(json!({"id":id,"service":"svc","revision":"rev","file":if provider == "JAVAC" { "Example.java" } else { "Example.kt" },"startLine":1,"endLine":1,"text":text,"textDigest":crate::canonical::hash_bytes(text.as_bytes()),"evidenceDigest":crate::canonical::hash_bytes(b"fixture"),"authority":"SYNTHETIC_RETAINED_FIXTURE"})).unwrap());
        evidence.observations.insert(id.into(),serde_json::from_value(json!({"id":id,"kind":"SYMBOL","service":"svc","symbol":symbol,"digest":format!("digest-{id}"),"sourceIds":[id],"normalized":{"declarationKind":"METHOD","scope":":main","name":id,"ownerIdentity":"class:Example","documentation":{"events":events}}})).unwrap());
        for (ordinal, event) in events.into_iter().enumerate() {
            let mut normalized = event;
            normalized["scope"] = json!(":main");
            normalized["ordinal"] = json!(ordinal);
            let event_id = format!("{id}-event-{ordinal}");
            evidence.observations.insert(event_id.clone(),serde_json::from_value(json!({"id":event_id,"kind":"FLOW","service":"svc","symbol":symbol,"digest":"event-digest","sourceIds":[id],"normalized":normalized})).unwrap());
        }
    }
    serde_json::from_value(json!({"schema":"test","inputDigest":"input","contextDigest":"context","services":{"svc":evidence},"dependencies":evidence.observations,"interactions":{},"unresolved":{},"scenarios":{}})).unwrap()
}
fn query() -> Query {
    Query {
        purpose: crate::thread_question::Purpose::Process,
        service: "svc".into(),
        root_declaration: "root".into(),
        configuration_scope: "UNSPECIFIED".into(),
        outcome_event_ids: vec!["helper-event-1".into()],
        budgets: Budgets::default(),
    }
}
fn has(packet: &Value, code: &str) -> bool {
    packet["frontier"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["code"] == code || f["kind"] == code)
}

#[test]
fn java_and_kotlin_packets_preserve_recursion_exceptions_and_unknown_transport() {
    for provider in ["JAVAC", "K2_FIR", "SYNTAX_ONLY"] {
        let packet = extract(&fixture(provider), "snapshot", &query()).unwrap();
        assert_eq!(packet["methods"].as_array().unwrap().len(), 2);
        assert_eq!(packet["events"].as_array().unwrap().len(), 8);
        assert_eq!(
            packet["outcomes"][0]["discovery"],
            "FOUND_IN_RETAINED_ENVELOPE"
        );
        assert!(has(&packet, "EXTERNAL_OR_UNRETAINED_TARGET"));
        assert!(has(
            &packet,
            "DATA_EFFECT_ALIAS_AND_RUNTIME_CLOSURE_NOT_ESTABLISHED"
        ));
        assert_eq!(packet["programSemanticClosure"], false);
        assert_eq!(packet["extractor"], provider);
        assert_eq!(packet["interpretationPlan"]["actualModelCalls"], 0);
        assert!(!packet.to_string().contains("DELIVERED"));
    }
}
#[test]
fn cuts_are_budget_frontiers_and_missing_outcomes_are_unknown() {
    for (budget, code) in [
        (
            Budgets {
                max_methods: 1,
                ..Budgets::default()
            },
            "METHOD_BUDGET",
        ),
        (
            Budgets {
                max_events: 1,
                ..Budgets::default()
            },
            "EVENT_BUDGET",
        ),
        (
            Budgets {
                max_source_bytes: 1,
                ..Budgets::default()
            },
            "SOURCE_BYTE_BUDGET",
        ),
    ] {
        let mut q = query();
        q.budgets = budget;
        let packet = extract(&fixture("JAVAC"), "snapshot", &q).unwrap();
        assert_eq!(packet["status"], "TRUNCATED");
        assert!(has(&packet, code));
        assert_eq!(packet["outcomes"][0]["discovery"], "NOT_FOUND_UNKNOWN");
        assert_eq!(packet["programSemanticClosure"], false);
    }
}
#[test]
fn identity_configuration_policy_and_budgets_participate_in_freshness() {
    let c = fixture("JAVAC");
    let q = query();
    let p = extract(&c, "snapshot", &q).unwrap();
    assert_eq!(p, extract(&c, "snapshot", &q).unwrap());
    let mut changed = q.clone();
    changed.configuration_scope = "dev-config-digest".into();
    assert_ne!(
        p["packetDigest"],
        extract(&c, "snapshot", &changed).unwrap()["packetDigest"]
    );
    assert_ne!(
        p["packetDigest"],
        extract(&c, "changed-snapshot", &q).unwrap()["packetDigest"]
    );
    let mut forged = c;
    forged.dependencies.get_mut("root").unwrap().digest = "forged".into();
    assert!(extract(&forged, "snapshot", &q).is_err());
    changed.budgets.max_methods = 0;
    assert!(changed.validate().is_err());
}
#[test]
fn ambiguous_targets_and_other_scopes_cannot_become_unique_by_aggregation() {
    let mut c = fixture("JAVAC");
    let mut duplicate = c.services["svc"].observations["helper"].clone();
    duplicate.id = "duplicate".into();
    c.services
        .get_mut("svc")
        .unwrap()
        .observations
        .insert(duplicate.id.clone(), duplicate);
    let p = extract(&c, "snapshot", &query()).unwrap();
    assert!(has(&p, "SAME_SCOPE_AMBIGUOUS"));
    assert_eq!(p["outcomes"][0]["discovery"], "NOT_FOUND_UNKNOWN");
    assert_eq!(p["outcomes"][0]["structuralMethodIds"], json!([]));
    assert!(
        p["methods"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["structuralOutcomeEventIds"] == json!([]))
    );
}

#[test]
fn deterministic_benchmark_records_full_read_and_bounded_envelope_bytes() {
    let c = fixture("JAVAC");
    let q = query();
    let full = extract(&c, "snapshot", &q).unwrap();
    let mut bounded = q.clone();
    bounded.budgets.max_methods = 1;
    let cut = extract(&c, "snapshot", &bounded).unwrap();
    let all_bytes: usize = c.services["svc"]
        .sources
        .values()
        .map(|s| s.text.len())
        .sum();
    let full_bytes = full["interpretationPlan"]["uniqueSourceBytes"]
        .as_u64()
        .unwrap();
    let cut_bytes = cut["interpretationPlan"]["uniqueSourceBytes"]
        .as_u64()
        .unwrap();
    assert!(cut_bytes < full_bytes && full_bytes < all_bytes as u64);
    println!(
        "PROCESS_FIXTURE_BENCHMARK {}",
        json!({"fullReadBytes":all_bytes,"fullEnvelopeBytes":full_bytes,"boundedEnvelopeBytes":cut_bytes,"boundedOutcome":"UNKNOWN","modelCalls":0,"duplicateBytes":0,"tokenMetric":"NOT_MEASURED"})
    );
}

#[test]
fn damaged_source_binding_fails_closed_instead_of_upgrading_provenance() {
    let mut c = fixture("JAVAC");
    c.services
        .get_mut("svc")
        .unwrap()
        .sources
        .get_mut("root")
        .unwrap()
        .text
        .push_str(" changed");
    assert!(extract(&c, "snapshot", &query()).is_err());
}

#[test]
fn multiple_outcomes_preserve_separate_call_corridors_and_unknowns() {
    let mut c = fixture("JAVAC");
    let service = c.services.get_mut("svc").unwrap();
    // Remove the helper -> root cycle, then add an independent root -> sibling.
    service.observations.get_mut("helper").unwrap().normalized["documentation"]["events"][0] =
        json!({"kind":"RETURN"});
    service
        .observations
        .get_mut("helper-event-0")
        .unwrap()
        .normalized = json!({"kind":"RETURN","scope":":main","ordinal":0});
    let root = service.observations.get_mut("root").unwrap();
    root.normalized["documentation"]["events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"CALL","target":"method:class:Example#unrelated()V"}));
    let mut call = service.observations["root-event-5"].clone();
    call.id = "root-event-6".into();
    call.normalized = json!({"kind":"CALL","target":"method:class:Example#unrelated()V","scope":":main","ordinal":6});
    service.observations.insert(call.id.clone(), call);
    c.dependencies = service.observations.clone();
    let mut q = query();
    q.outcome_event_ids = vec![
        "helper-event-1".into(),
        "unrelated-event-0".into(),
        "missing".into(),
    ];
    let p = extract(&c, "snapshot", &q).unwrap();
    assert_eq!(
        p["outcomes"][0]["structuralMethodIds"],
        json!(["helper", "root"])
    );
    assert_eq!(
        p["outcomes"][1]["structuralMethodIds"],
        json!(["root", "unrelated"])
    );
    assert_eq!(p["outcomes"][2]["structuralMethodIds"], json!([]));
    let helper = p["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "helper")
        .unwrap();
    assert_eq!(
        helper["structuralOutcomeEventIds"],
        json!(["helper-event-1"])
    );
    assert_eq!(p["programSemanticClosure"], false);
    q.budgets.max_methods = 1;
    let cut = extract(&c, "snapshot", &q).unwrap();
    assert_eq!(cut["outcomes"][0]["discovery"], "NOT_FOUND_UNKNOWN");
    assert_eq!(cut["outcomes"][0]["structuralMethodIds"], json!([]));
}

#[test]
fn empty_criteria_keep_forward_envelope_and_query_order_controls_separate_memberships() {
    let c = fixture("JAVAC");
    let mut q = query();
    q.outcome_event_ids.clear();
    let empty = extract(&c, "snapshot", &q).unwrap();
    assert_eq!(empty["outcomes"], json!([]));
    assert_eq!(empty["methods"].as_array().unwrap().len(), 2);
    for method in empty["methods"].as_array().unwrap() {
        assert_eq!(method["onStructuralOutcomePath"], false);
        assert_eq!(method["structuralOutcomeEventIds"], json!([]));
    }
    q.outcome_event_ids = vec!["helper-event-1".into(), "root-event-3".into()];
    let first = extract(&c, "snapshot", &q).unwrap();
    q.outcome_event_ids.reverse();
    let reversed = extract(&c, "snapshot", &q).unwrap();
    for method in reversed["methods"].as_array().unwrap() {
        assert_eq!(
            method["structuralOutcomeEventIds"],
            json!(q.outcome_event_ids)
        );
    }
    assert_eq!(first["events"], reversed["events"]);
    assert_eq!(first["edges"], reversed["edges"]);
    assert_eq!(first["sourceRefs"], reversed["sourceRefs"]);
    assert_eq!(first["programSemanticClosure"], false);
    q.outcome_event_ids.push(q.outcome_event_ids[0].clone());
    assert!(q.validate().is_err());
    q.outcome_event_ids = (0..33).map(|i| format!("criterion-{i}")).collect();
    assert!(q.validate().is_err());
}
