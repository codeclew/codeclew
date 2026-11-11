//! Historical retained schemas and canonical provenance remain verifiable.
use super::super::bytes;
use super::super::review;
use super::*;
use std::collections::BTreeSet;

fn fixture() -> (tempfile::TempDir, Repository, Work, Narrative) {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Retained edit fixture").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    let service: Service = serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0", "id":"orders", "title":"Orders",
        "repositoryId":"orders", "repository":"https://example.invalid/orders",
        "language":"java", "profile":"source-syntax", "targetRef":"HEAD",
        "source":{"roots":["."],"dialect":"17"}
    }))
    .unwrap();
    let input_digest = repo.input_digest().unwrap();
    repo.service_add(service.clone(), Some(&input_digest))
        .unwrap();
    let source_text = "int reserve(int quantity) { return quantity; }";
    let normalized = json!({"declarationKind":"METHOD", "name":"reserve", "owner":"Orders"});
    let observation = Observation {
        id: "symbol-reserve".into(),
        kind: "SYMBOL".into(),
        service: "orders".into(),
        symbol: "Orders.reserve(int)".into(),
        digest: digest(&normalized).unwrap(),
        normalized,
        source_ids: vec!["source-reserve".into()],
    };
    let source = Source {
        id: "source-reserve".into(),
        service: "orders".into(),
        revision: "fixture-revision".into(),
        file: "Orders.java".into(),
        start_line: 1,
        end_line: 1,
        text: source_text.into(),
        text_digest: crate::canonical::hash_bytes(source_text.as_bytes()),
        evidence_digest: digest(&observation).unwrap(),
        authority: "TEST_FIXTURE".into(),
        library_origin: None,
        text_origin: None,
        occurrence: None,
        url: None,
    };
    let entry = |id: &str| Entrypoint {
        id: id.into(),
        service: "orders".into(),
        symbol: "Orders.reserve(int)".into(),
        kind: "method".into(),
        trigger: json!({}),
        source_ids: vec![source.id.clone()],
        dependency_ids: vec![observation.id.clone()],
        boundaries: vec![],
    };
    let evidence = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: "orders".into(),
        revision: source.revision.clone(),
        service_digest: digest(&service).unwrap(),
        extractor: SOURCE_EXTRACTOR.into(),
        runtime_mode: "TEST_FIXTURE".into(),
        coverage: "SOURCE_SYNTAX".into(),
        boundaries: vec![],
        entrypoints: vec![entry("reserve"), entry("unrelated")],
        observations: BTreeMap::from([(observation.id.clone(), observation.clone())]),
        sources: BTreeMap::from([(source.id.clone(), source.clone())]),
        contracts: BTreeMap::new(),
    };
    let mut checked = check::assemble(
        repo.input_digest().unwrap(),
        BTreeMap::from([("orders".into(), evidence)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    checked.source_inputs = Some(check::SourceInputs {
        schema: check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest: checked.input_digest.clone(),
        inputs: repo.inputs().unwrap(),
        selected_services: BTreeSet::from(["orders".into()]),
        retained_services: BTreeSet::new(),
    });
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let fragment = |id: &str, text: &str| Fragment {
        id: id.into(),
        text: text.into(),
        dependency_ids: vec![observation.id.clone()],
        source_ids: vec![source.id.clone()],
    };
    let visual_claim = |id: &str, text: &str| serde_json::to_value(fragment(id, text)).unwrap();
    let visual = serde_json::from_value(json!({
        "schema":super::super::visuals::SCHEMA, "generator":super::super::visuals::GENERATOR,
        "id":"quantity-flow", "kind":"execution-flow", "title":"Quantity flow",
        "purpose":visual_claim("purpose","Explain quantity handling."),
        "scope":visual_claim("scope","The selected method."), "limitations":["Static interpretation only."],
        "nodes":[{"id":"input","meaning":visual_claim("input-label","Receive quantity.")},
                 {"id":"output","meaning":visual_claim("output-label","Return quantity.")}],
        "edges":[{"id":"return-edge","from":"input","to":"output","meaning":visual_claim("edge-label","Return the supplied value.")}]
    })).unwrap();
    let operation = Operation {
        documentation_language: Some("en".into()),
        id: "reserve".into(),
        title: "Reserve quantity".into(),
        summary: fragment("summary", "Returns the requested quantity."),
        assessment: None,
        dataflow: None,
        explanation: (0..32)
            .map(|index| Explanation {
                id: format!("paragraph-{index}"),
                text: format!(
                    "Retained paragraph {index}. {}",
                    "Quantity explanation remains preserved. ".repeat(60)
                ),
                event_ids: vec!["event".into()],
                dependency_ids: vec![observation.id.clone()],
                source_ids: vec![source.id.clone()],
                detail: index % 2 == 0,
                authorship: None,
            })
            .collect(),
        interface_contracts: vec![InterfaceContract {
            id: "output-contract".into(),
            title: "Output".into(),
            kind: "payload".into(),
            rows: vec![InterfaceContractRow {
                field: None,
                id: "quantity-row".into(),
                label: "quantity".into(),
                value: "Requested quantity.".into(),
                dependency_ids: vec![observation.id.clone()],
                source_ids: vec![source.id.clone()],
            }],
            boundaries: vec!["Source interpretation.".into()],
        }],
        overview_diagram: None,
        participants: vec![
            Participant {
                id: "caller".into(),
                label: "Caller".into(),
                service: None,
                external_service: None,
            },
            Participant {
                id: "service".into(),
                label: "Orders".into(),
                service: Some("orders".into()),
                external_service: None,
            },
        ],
        events: vec![Event {
            reader: None,
            id: "event".into(),
            kind: "note".into(),
            text: "Returns quantity.".into(),
            from: None,
            to: None,
            interaction: None,
            dependency_ids: vec![observation.id.clone()],
            source_ids: vec![source.id.clone()],
        }],
        findings: vec![fragment("finding", "No delivery guarantee is established.")],
        boundaries: vec!["Static source interpretation.".into()],
        visuals: vec![visual],
    };
    assert!(bytes(&operation).unwrap().len() > 49_152);
    let mut unrelated = operation.clone();
    unrelated.id = "unrelated".into();
    unrelated.title = "Unrelated operation".into();
    unrelated.explanation.truncate(1);
    let narrative = Narrative {
        schema: "codeclew-documentation-narrative/1.3".into(),
        subject: "service:orders".into(),
        context_digest: checked.context_digest.clone(),
        operations: vec![operation, unrelated],
        gaps: BTreeMap::new(),
    };
    retained_validation::validate(&narrative, &checked).unwrap();
    let request: work::Request =
        serde_json::from_value(json!({"schema":"codeclew-documentation-work-request/1.0",
        "audience":"Maintainers","maxItems":100,"maxBytes":49152}))
        .unwrap();
    let external_inputs = work::capture_inputs(&repo, &request).unwrap();
    let work = Work {
        identity_references: Default::default(),
        schema: "codeclew-documentation-work/1.0".into(),
        id: "a".repeat(64),
        subject: narrative.subject.clone(),
        request,
        checked: checked.clone(),
        snapshot: Some(snapshot),
        retained: None,
        maintained_context: None,
        external_inputs,
        handles: BTreeMap::new(),
        influence: checked
            .dependencies
            .iter()
            .map(|(id, observation)| (id.clone(), observation.digest.clone()))
            .collect(),
        obligations: vec![],
        review_reasons: vec![],
    };
    (temp, repo, work, narrative)
}

#[test]
fn internal_method_flow_requirements_keep_exact_owner_and_scope() {
    let mut work = work::api_contract_tests::endpoint_context_fixture();
    let root_id = "service-method-0";
    let evidence = work.checked.services.get_mut("orders").unwrap();
    evidence.entrypoints.clear();
    let root = evidence.observations.get_mut(root_id).unwrap();
    root.normalized["documentationIdentity"] = json!(root_id);
    root.digest = digest(&root.normalized).unwrap();
    let root = root.clone();
    work.checked
        .dependencies
        .insert(root.id.clone(), root.clone());
    for (id, owner, scope) in [
        (
            "owned-branch",
            root_id,
            root.normalized["scope"].as_str().unwrap(),
        ),
        (
            "other-owner",
            "another-declaration",
            root.normalized["scope"].as_str().unwrap(),
        ),
        ("other-scope", "another-declaration", ":/test"),
    ] {
        let mut flow = root.clone();
        flow.id = id.into();
        flow.kind = "FLOW".into();
        flow.normalized = json!({"kind":"IF","scope":scope,"ownerDeclarationId":owner});
        flow.digest = digest(&flow.normalized).unwrap();
        evidence.observations.insert(id.into(), flow.clone());
        work.checked.dependencies.insert(id.into(), flow);
    }
    let target = super::super::process_graph::internal_operation_id(&root);
    let required =
        retained_validation::required_sequence_flows(&work.checked, "service:orders", &target)
            .unwrap();
    assert_eq!(
        required
            .iter()
            .map(|flow| flow.id.as_str())
            .collect::<Vec<_>>(),
        ["owned-branch"]
    );
}

#[test]
fn empty_retained_edits_preserve_legacy_proposal_serialization() {
    let legacy = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],"gaps":{},"uncertainties":[]});
    let proposal: Proposal = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(proposal).unwrap(), legacy);
}

#[test]
fn retained_scope_exception_requires_exact_canonical_authored_provenance_and_covering_scope() {
    let (_temp, repo, mut work, mut narrative) = fixture();
    work.retained = Some(narrative.clone());
    let paragraph = &mut narrative.operations[0].explanation[0];
    paragraph.authorship = Some(ExplanationAuthorship {
        authority: ExplanationAuthority::UserDocumentation,
        author: "Fixture editor".into(),
        meaning_review: AuthoredMeaningReview::Unassessed,
        context_role: AuthoredContextRole::RetainedUnverifiedContext,
        edit_digest: digest(&"historical owned edit").unwrap(),
        source_snapshot: work.snapshot.clone().unwrap(),
        source_refs: paragraph
            .source_ids
            .iter()
            .map(|id| (id.clone(), digest(&work.checked.sources()[id]).unwrap()))
            .collect(),
        dependency_refs: paragraph
            .dependency_ids
            .iter()
            .map(|id| (id.clone(), digest(&work.checked.dependencies[id]).unwrap()))
            .collect(),
        context_migration: None,
    });
    let key = format!("{}/reserve/{}", work.subject, paragraph.id);
    let mut binding = retained_validation::make_bindings(
        &work.checked,
        BTreeMap::from([(work.subject.clone(), narrative)]),
    )
    .unwrap();
    let scope = review::InfluenceScope {
        dependencies: work.influence.clone(),
        declarations: BTreeMap::new(),
    };
    let id = digest(&scope).unwrap();
    binding.influence_scopes.insert(id.clone(), scope);
    binding.fragments.get_mut(&key).unwrap().influence_scope = Some(id);
    assert!(super::super::explanation_authorship::retained_scope(&binding, &key).unwrap());
    super::super::explanation_authorship::validate_binding_pin(
        &binding,
        &key,
        &check::Check::load_snapshot(&repo, work.snapshot.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    let generated = format!("{}/reserve/summary", work.subject);
    assert!(!super::super::explanation_authorship::retained_scope(&binding, &generated).unwrap());
    let mut forged = binding.clone();
    forged.fragments.get_mut(&key).unwrap().content["authorship"]["author"] = json!("Forged actor");
    assert!(super::super::explanation_authorship::retained_scope(&forged, &key).is_err());
    let empty = review::InfluenceScope {
        dependencies: BTreeMap::new(),
        declarations: BTreeMap::new(),
    };
    let empty_id = digest(&empty).unwrap();
    binding.influence_scopes.insert(empty_id.clone(), empty);
    binding.fragments.get_mut(&key).unwrap().influence_scope = Some(empty_id);
    assert!(
        super::super::explanation_authorship::retained_scope(&binding, &key)
            .unwrap_err()
            .message
            .contains("does not cover")
    );
}
