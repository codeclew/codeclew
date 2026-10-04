//! Retained edits exercise real reads, review coverage and the atomic publisher.
use super::super::{review, work_retained_parts};
use super::*;
use std::fs;

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
            },
            Participant {
                id: "service".into(),
                label: "Orders".into(),
                service: Some("orders".into()),
            },
        ],
        events: vec![Event {
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
    render::validate(&narrative, &checked).unwrap();
    let request: work::Request =
        serde_json::from_value(json!({"schema":"codeclew-documentation-work-request/1.0",
        "audience":"Maintainers","maxItems":100,"maxBytes":49152}))
        .unwrap();
    let external_inputs = work::capture_inputs(&repo, &request).unwrap();
    let work = Work {
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

fn input(work: &Work) -> Proposal {
    let operation = work
        .retained
        .as_ref()
        .unwrap()
        .operations
        .iter()
        .find(|o| o.id == "reserve")
        .unwrap();
    serde_json::from_value(json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],"retainedEdits":[{
        "kind":"RETAINED_OPERATION","id":operation.id,"recordDigest":digest(operation).unwrap(),
        "target":"operationTitle","expectedOldValue":operation.title,"replacement":"Normalize requested quantity"
    }]})).unwrap()
}

fn artifact(
    work: &Work,
    input: Proposal,
    narrative: Narrative,
    claims: BTreeMap<String, Value>,
) -> Artifact {
    Artifact {
        schema: "codeclew-documentation-proposal-result/1.0".into(),
        id: "b".repeat(64),
        work: work.id.clone(),
        input,
        narrative: Some(narrative),
        status: "READY_FOR_REVIEW".into(),
        diagnostics: vec![],
        claims,
        read_digest: "sha256:fixture-reads".into(),
        influence: work.influence.clone(),
        meaning_review: "UNASSESSED".into(),
    }
}

fn approval(work: &Work, proposal: &Artifact) -> review::MeaningReview {
    review::MeaningReview {
        schema: "codeclew-documentation-review/1.0".into(),
        work: work.id.clone(),
        proposal: proposal.id.clone(),
        evidence_digest: "sha256:fixture-dispatch".into(),
        verdict: "APPROVE".into(),
        assessed_claims: proposal.claims.keys().cloned().collect(),
        assessed_operations: proposal
            .narrative
            .as_ref()
            .unwrap()
            .operations
            .iter()
            .map(|o| o.id.clone())
            .collect(),
        issues: vec![],
        limitations: vec![],
    }
}

fn publish(repo: &Repository, work: &Work, proposal: &Artifact) -> Value {
    let review = approval(work, proposal);
    let versions = review::versions(
        work,
        proposal,
        &review,
        "fixture-invocation",
        "sha256:fixture-driver",
        &review.evidence_digest,
        &proposal.read_digest,
    )
    .unwrap();
    render::publish_reviewed(
        repo,
        proposal.narrative.clone().unwrap(),
        versions,
        work.snapshot.as_deref(),
    )
    .unwrap()
}

fn parts(repo: &Repository, work: &Work) -> work::ReadState {
    let mut cursor = None;
    loop {
        let response = work_retained_parts::read_retained_part_loaded(
            repo,
            work,
            work::RetainedPartRequest {
                schema: "codeclew-documentation-retained-part-request/1.0".into(),
                kind: "RETAINED_OPERATION".into(),
                id: "reserve".into(),
                cursor,
            },
        )
        .unwrap();
        cursor = response["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    work::read_state(repo, &work.id).unwrap()
}

#[test]
fn title_edit_preserves_large_operation_and_requires_exact_parts_and_review() {
    let (_temp, repo, mut work, narrative) = fixture();
    work.retained = Some(narrative.clone());
    work.request.entrypoint = Some("reserve".into());
    let proposal = input(&work);
    assert!(
        materialize(&work, &proposal, &work::ReadState::default())
            .unwrap_err()
            .message
            .contains("every exact retained operation part")
    );
    let state = parts(&repo, &work);
    assert!(state.retained_part_receipts.len() > 1);
    let (changed, claims, diagnostics) = materialize(&work, &proposal, &state).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut expected = narrative.operations[0].clone();
    expected.title = "Normalize requested quantity".into();
    assert_eq!(
        bytes(&changed.operations[0]).unwrap(),
        bytes(&expected).unwrap()
    );
    assert_eq!(
        bytes(work.retained.as_ref().unwrap()).unwrap(),
        bytes(&narrative).unwrap()
    );
    assert_eq!(claims.len(), 1);
    let claim = claims.values().next().unwrap();
    assert_eq!(claim["kind"], "RETAINED_PRESENTATION_EDIT");
    assert_eq!(claim["authority"], "PRESENTATION_PROPOSAL");
    assert!(claim.get("fragment").is_none());
    assert!(claim.get("sourceIds").is_none());
    assert!(claim.get("dependencyIds").is_none());
    assert_eq!(
        claim["edit"],
        serde_json::to_value(&proposal.retained_edits[0]).unwrap()
    );
    let artifact = artifact(&work, proposal, changed, claims);
    let approved = approval(&work, &artifact);
    review::validate(&work, &artifact, &approved, &approved.evidence_digest).unwrap();
    let mut missing = approved.clone();
    missing.assessed_claims.clear();
    assert!(
        review::validate(&work, &artifact, &missing, &approved.evidence_digest)
            .unwrap_err()
            .message
            .contains("REVIEW_COVERAGE_INCOMPLETE")
    );
    let mut missing = approved.clone();
    missing.assessed_operations.clear();
    assert!(review::validate(&work, &artifact, &missing, &approved.evidence_digest).is_err());
}

#[test]
fn reviewed_title_edit_publishes_atomically_retains_history_and_rejects_baseline_conflict() {
    let (_temp, repo, initial_work, narrative) = fixture();
    let baseline = artifact(
        &initial_work,
        serde_json::from_value(
            json!({"schema":"codeclew-documentation-proposal/1.0","operations":[]}),
        )
        .unwrap(),
        narrative,
        BTreeMap::new(),
    );
    let published = publish(&repo, &initial_work, &baseline);
    let old_bundle = published["bundle"].as_str().unwrap();
    let old_json_path = repo
        .path(&format!("docs/generated/{old_bundle}/services/orders.json"))
        .unwrap();
    let old_html_path = repo
        .path(&format!("docs/generated/{old_bundle}/services/orders.html"))
        .unwrap();
    let old_json = fs::read(&old_json_path).unwrap();
    let old_html = fs::read(&old_html_path).unwrap();
    let old_binding = bindings::baseline(&repo).unwrap().unwrap().1;
    let request =
        serde_json::from_value(json!({"schema":"codeclew-documentation-work-request/1.0",
        "audience":"Maintainers","entrypoint":"reserve","maxItems":100,"maxBytes":49152}))
        .unwrap();
    let mut page = work::prepare_with_snapshot(
        &repo,
        "service:orders".into(),
        request,
        initial_work.snapshot.as_deref(),
    )
    .unwrap();
    let id = page["work"].as_str().unwrap().to_owned();
    while let Some(cursor) = page["nextCursor"].as_str() {
        page = work::read(
            &repo,
            &id,
            work::Selection {
                cursor: Some(cursor.into()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let work = work::load(&repo, &id).unwrap();
    let state = parts(&repo, &work);
    assert!(work::initial_context_complete_with_parts(&work, &state).unwrap());
    let submitted = submit(&repo, &id, input(&work)).unwrap();
    assert!(
        submitted["status"].as_str().unwrap().starts_with("READY_"),
        "{submitted}"
    );
    let artifact = load(&repo, submitted["proposal"].as_str().unwrap()).unwrap();
    // The complete persisted manual ledger admits this manual proposal, but
    // cannot substitute for delivery to an automatic author or reviewer.
    let admission =
        super::super::agent_jobs::validate_automatic_proposal(&artifact.input).unwrap_err();
    assert!(
        admission
            .message
            .contains("AUTOMATIC_RETAINED_EDIT_UNSUPPORTED")
    );
    assert!(work_retained_parts::retained_part_complete(&work, &state, "reserve").unwrap());
    assert!(
        !super::super::work_parts::initial_context_complete_with_packet_parts(
            &work,
            &state,
            &BTreeSet::new()
        )
        .unwrap()
    );
    let result = publish(&repo, &work, &artifact);
    assert_eq!(result["updateFailures"], json!({}));
    let new_binding = bindings::baseline(&repo).unwrap().unwrap().1;
    let old = &old_binding.narratives["service:orders"];
    let new = &new_binding.narratives["service:orders"];
    for original in &old.operations {
        let actual = new.operations.iter().find(|o| o.id == original.id).unwrap();
        let mut expected = original.clone();
        if expected.id == "reserve" {
            expected.title = "Normalize requested quantity".into();
        }
        assert_eq!(bytes(actual).unwrap(), bytes(&expected).unwrap());
    }
    for (key, binding) in &old_binding.fragments {
        if !key.ends_with("/title") || !key.starts_with("service:orders/reserve/") {
            assert_eq!(new_binding.fragments[key].content, binding.content, "{key}");
        }
    }
    assert_eq!(
        serde_json::to_value(&new_binding.accepted_versions["service:orders/unrelated"]).unwrap(),
        serde_json::to_value(&old_binding.accepted_versions["service:orders/unrelated"]).unwrap()
    );
    assert_eq!(
        new_binding.accepted_versions["service:orders/reserve"].verification,
        "VERIFIED_WITH_LIMITATIONS"
    );
    assert_eq!(
        new_binding.accepted_versions["service:orders/reserve"].proposal,
        artifact.id
    );
    assert_eq!(fs::read(&old_json_path).unwrap(), old_json);
    assert_eq!(fs::read(&old_html_path).unwrap(), old_html);
    assert_eq!(
        current(&repo, &work).unwrap_err().code,
        ErrorCode::WwConflict
    );
    assert!(submit(&repo, &id, input(&work)).is_err());
    assert!(publish_conflict(&repo, &work, &artifact));
}

fn publish_conflict(repo: &Repository, work: &Work, artifact: &Artifact) -> bool {
    let review = approval(work, artifact);
    let versions = review::versions(
        work,
        artifact,
        &review,
        "fixture-invocation",
        "sha256:fixture-driver",
        &review.evidence_digest,
        &artifact.read_digest,
    )
    .unwrap();
    render::publish_reviewed(
        repo,
        artifact.narrative.clone().unwrap(),
        versions,
        work.snapshot.as_deref(),
    )
    .is_err()
}

#[test]
fn retained_edit_rejects_stale_values_targets_duplicates_and_replacement_collisions() {
    let (_temp, repo, mut work, narrative) = fixture();
    work.retained = Some(narrative);
    work.request.entrypoint = Some("reserve".into());
    let state = parts(&repo, &work);
    let proposal = input(&work);
    for (change, expected) in [
        ("digest", "digest is stale"),
        ("value", "old value is stale"),
        ("duplicate", "duplicate retained"),
        ("scope", "outside the Work"),
        ("empty", "must be nonblank"),
        ("control", "control characters"),
        ("gap", "gap conflicts"),
    ] {
        let mut invalid = proposal.clone();
        match change {
            "digest" => invalid.retained_edits[0].record_digest = "sha256:wrong".into(),
            "value" => invalid.retained_edits[0].expected_old_value = "Outdated title".into(),
            "duplicate" => invalid
                .retained_edits
                .push(invalid.retained_edits[0].clone()),
            "scope" => invalid.retained_edits[0].id = "unrelated".into(),
            "empty" => invalid.retained_edits[0].replacement = " ".into(),
            "control" => invalid.retained_edits[0].replacement = "A\nB".into(),
            "gap" => {
                work.handles.insert(
                    "root".into(),
                    work::Handle {
                        kind: "ENTRYPOINT".into(),
                        id: "reserve".into(),
                    },
                );
                invalid.gaps.insert("root".into(), "Deferred".into());
            }
            _ => unreachable!(),
        }
        let error = materialize(&work, &invalid, &state).unwrap_err();
        assert!(
            error.message.contains(expected),
            "{change}: {}",
            error.message
        );
    }
    let value = serde_json::to_value(&proposal).unwrap();
    for target in [
        "participantLabel",
        "visualTitle",
        "summary",
        "evidenceIds",
        "/title",
    ] {
        let mut invalid = value.clone();
        invalid["retainedEdits"][0]["target"] = json!(target);
        assert!(serde_json::from_value::<Proposal>(invalid).is_err());
    }
    let mut invalid = value.clone();
    invalid["retainedEdits"][0]["path"] = json!("/title");
    assert!(serde_json::from_value::<Proposal>(invalid).is_err());
    let mut invalid = value;
    invalid["retainedEdits"][0]["kind"] = json!("DEPENDENCY");
    assert!(serde_json::from_value::<Proposal>(invalid).is_err());
    let mut replacement = proposal;
    replacement.operations.push(
        serde_json::from_value(json!({"entrypoint":"root","title":"Replacement",
        "summary":{"text":"Replacement explanation.","evidence":["root"]},"steps":[],"visuals":[]}))
        .unwrap(),
    );
    let mut collision_state = state;
    collision_state.receipts.insert(
        "root-read".into(),
        work::ReadReceipt {
            selection: Default::default(),
            requested_selection: None,
            result_digest: "read".into(),
            supplied: vec!["root".into()],
            membership_digest: "membership".into(),
            omitted: vec![],
            next_cursor: None,
        },
    );
    assert!(
        materialize(&work, &replacement, &collision_state)
            .unwrap_err()
            .message
            .contains("conflicts with an operation replacement")
    );
}

#[test]
fn empty_retained_edits_preserve_legacy_proposal_serialization() {
    let legacy = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],"gaps":{},"uncertainties":[]});
    let proposal: Proposal = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(proposal).unwrap(), legacy);
}

#[test]
fn paragraph_edit_preserves_canonical_fields_and_declares_unverified_context() {
    let (_temp, repo, mut work, narrative) = fixture();
    work.retained = Some(narrative.clone());
    work.request.entrypoint = Some("reserve".into());
    let original = &narrative.operations[0];
    let mut proposal: Proposal = serde_json::from_value(json!({
        "schema":"codeclew-documentation-proposal/1.0", "operations":[],
        "retainedEdits":[{"kind":"RETAINED_OPERATION", "id":"reserve",
            "recordDigest":digest(original).unwrap(), "target":"explanationText",
            "fragmentId":"paragraph-1", "author":"Fixture editor",
            "expectedOldValue":original.explanation[1].text,
            "replacement":"The caller chooses the requested quantity. This is the editor's explanation."}]
    })).unwrap();
    let state = parts(&repo, &work);
    let (edited, claims, diagnostics) = materialize(&work, &proposal, &state).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let paragraph = &edited.operations[0].explanation[1];
    let authorship = paragraph.authorship.as_ref().unwrap();
    assert_eq!(authorship.author, "Fixture editor");
    assert_eq!(
        serde_json::to_value(authorship).unwrap()["meaningReview"],
        "UNASSESSED"
    );
    assert_eq!(
        authorship.source_snapshot,
        work.snapshot.as_ref().unwrap().as_str()
    );
    assert_eq!(
        authorship.source_refs["source-reserve"],
        digest(&work.checked.sources()["source-reserve"]).unwrap()
    );
    let mut expected = original.clone();
    expected.explanation[1] = paragraph.clone();
    assert_eq!(
        bytes(&edited.operations[0]).unwrap(),
        bytes(&expected).unwrap()
    );
    let claim = claims.values().next().unwrap();
    assert_eq!(claim["authority"], "USER_DOCUMENTATION");
    assert!(claim.get("sourceIds").is_none());
    assert!(claim.get("evidence").is_none());
    // A distinct title edit may share the same immutable record, while duplicate
    // paragraph targets and undeclared authors cannot pass deterministic checks.
    proposal
        .retained_edits
        .push(input(&work).retained_edits.remove(0));
    assert!(materialize(&work, &proposal, &state).unwrap().2.is_empty());
    proposal
        .retained_edits
        .push(proposal.retained_edits[0].clone());
    assert!(
        materialize(&work, &proposal, &state)
            .unwrap_err()
            .message
            .contains("duplicate")
    );
    proposal.retained_edits.truncate(1);
    proposal.retained_edits[0].author = None;
    assert!(
        materialize(&work, &proposal, &state)
            .unwrap_err()
            .message
            .contains("declared author")
    );
    let mut forged_refs = paragraph.clone();
    forged_refs.authorship.as_mut().unwrap().source_refs.insert(
        "source-reserve".into(),
        format!("sha256:{}", "0".repeat(64)),
    );
    assert!(
        super::super::explanation_authorship::validate(&forged_refs, &work.checked)
            .unwrap_err()
            .message
            .contains("linked context changed")
    );
    let protected = &edited.operations[0];
    let mut regenerated = protected.clone();
    regenerated.explanation[1].authorship = None;
    assert!(
        super::super::explanation_authorship::preserve(protected, &regenerated, &BTreeSet::new())
            .is_err()
    );
    let encoded = serde_json::to_value(&original.explanation[1]).unwrap();
    assert!(
        encoded.get("authorship").is_none(),
        "historic serialization must stay unchanged"
    );
}

#[test]
fn host_preserves_protected_paragraph_and_requires_surviving_event_anchor() {
    let (_temp, _repo, mut work, narrative) = fixture();
    work.retained = Some(narrative.clone());
    let mut old = narrative.operations[0].clone();
    let mut edit = input(&work).retained_edits.remove(0);
    edit.target = RetainedTarget::ExplanationText;
    edit.fragment_id = Some(old.explanation[0].id.clone());
    edit.author = Some("Fixture editor".into());
    edit.expected_old_value = old.explanation[0].text.clone();
    edit.replacement = "Protected documentation stays unchanged during regeneration.".into();
    old.explanation[0].authorship = Some(
        super::super::explanation_authorship::from_edit(&work, &old.explanation[0], &edit).unwrap(),
    );
    old.explanation[0].text = edit.replacement;
    let mut generated = narrative.operations[0].clone();
    generated.explanation.truncate(1);
    generated.explanation[0].text = "Current generated interpretation of the changed step.".into();
    generated.summary.text = "New generated summary.".into();
    super::super::explanation_authorship::merge(&work.subject, &old, &mut generated).unwrap();
    assert_eq!(
        generated
            .explanation
            .iter()
            .find(|p| p.id == old.explanation[0].id)
            .unwrap(),
        &old.explanation[0]
    );
    assert_eq!(generated.explanation.len(), 2);
    assert_ne!(generated.explanation[0].id, old.explanation[0].id);
    assert!(generated.explanation[0].authorship.is_none());
    assert_eq!(
        generated.explanation[0].source_ids,
        old.explanation[0].source_ids
    );
    let once = generated.clone();
    super::super::explanation_authorship::merge(&work.subject, &old, &mut generated).unwrap();
    assert_eq!(generated, once, "host preservation is idempotent");
    generated.events.clear();
    assert!(
        super::super::explanation_authorship::merge(&work.subject, &old, &mut generated)
            .unwrap_err()
            .message
            .contains("event anchor")
    );
}

#[test]
fn retained_scope_exception_requires_exact_canonical_authored_provenance_and_covering_scope() {
    let (_temp, repo, mut work, mut narrative) = fixture();
    work.retained = Some(narrative.clone());
    let mut edit = input(&work).retained_edits.remove(0);
    edit.target = RetainedTarget::ExplanationText;
    edit.fragment_id = Some(narrative.operations[0].explanation[0].id.clone());
    edit.author = Some("Fixture editor".into());
    let paragraph = &mut narrative.operations[0].explanation[0];
    paragraph.authorship =
        Some(super::super::explanation_authorship::from_edit(&work, paragraph, &edit).unwrap());
    let key = format!("{}/reserve/{}", work.subject, paragraph.id);
    let mut binding = render::make_bindings(
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

#[test]
fn context_migration_requires_exact_full_source_receipts_and_delivered_dependencies() {
    use super::super::{explanation_authorship as auth, work_parts};
    let (_temp, repo, mut work, mut narrative) = fixture();
    work.retained = Some(narrative.clone());
    work.request.entrypoint = Some("reserve".into());
    let mut text = input(&work).retained_edits.remove(0);
    text.target = RetainedTarget::ExplanationText;
    text.author = Some("Text author".into());
    text.fragment_id = Some(narrative.operations[0].explanation[0].id.clone());
    let paragraph = &mut narrative.operations[0].explanation[0];
    paragraph.authorship = Some(auth::from_edit(&work, paragraph, &text).unwrap());
    let paragraph = paragraph.clone();
    work.retained = Some(narrative.clone());
    work.handles.insert(
        "s1".into(),
        work::Handle {
            kind: "SOURCE".into(),
            id: "source-reserve".into(),
        },
    );
    work.handles.insert(
        "d1".into(),
        work::Handle {
            kind: "DEPENDENCY".into(),
            id: "symbol-reserve".into(),
        },
    );
    work::read_loaded(
        &repo,
        &work,
        work::Selection {
            query: Some(work::Query {
                kind: "SYMBOL".into(),
                symbol_contains: String::new(),
                projection: work::QueryProjection::Raw,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let mut state = parts(&repo, &work);
    let proposal:Proposal=serde_json::from_value(json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],"retainedEdits":[{
        "kind":"RETAINED_OPERATION","id":"reserve","recordDigest":digest(&narrative.operations[0]).unwrap(),"target":"explanationContext","fragmentId":paragraph.id,
        "expectedParagraphDigest":digest(&paragraph).unwrap(),"expectedContextDigest":auth::context_digest(&paragraph).unwrap(),"contextEditor":"Context editor",
        "sourceReferences":["s1"],"dependencyReferences":["d1"],"anchors":[{"eventId":"event","expectedEventDigest":digest(&narrative.operations[0].events[0]).unwrap()}]
    }]})).unwrap();
    let edit = &proposal.retained_edits[0];
    assert!(
        auth::from_context_edit(&work, &paragraph, edit, &state)
            .unwrap_err()
            .message
            .contains("COMPLETE")
    );
    work_parts::read_part_loaded(
        &repo,
        &work,
        work_parts::SourcePartRequest {
            schema: work_parts::REQUEST_SCHEMA.into(),
            reference: "s1".into(),
            cursor: None,
        },
    )
    .unwrap();
    state = work::read_state(&repo, &work.id).unwrap();
    let migrated = auth::from_context_edit(&work, &paragraph, edit, &state).unwrap();
    assert_eq!(
        migrated.author,
        paragraph.authorship.as_ref().unwrap().author
    );
    assert_eq!(
        migrated.edit_digest,
        paragraph.authorship.as_ref().unwrap().edit_digest
    );
    assert_eq!(
        migrated.context_migration.as_ref().unwrap().editor,
        "Context editor"
    );
    for change in ["foreign", "partial", "stale-snapshot"] {
        let mut invalid = state.clone();
        let receipt = invalid.source_part_receipts.values_mut().next().unwrap();
        match change {
            "foreign" => receipt.work = "b".repeat(64),
            "partial" => receipt.end_byte = receipt.total_text_bytes / 2,
            _ => receipt.snapshot = "foreign-snapshot".into(),
        }
        assert!(
            auth::from_context_edit(&work, &paragraph, edit, &invalid)
                .unwrap_err()
                .message
                .contains("COMPLETE"),
            "{change}"
        );
    }
    let binding = render::make_bindings(
        &work.checked,
        BTreeMap::from([(work.subject.clone(), narrative.clone())]),
    )
    .unwrap();
    for change in ["scope", "kind", "provider"] {
        let mut incompatible = work.clone();
        let dependency = incompatible
            .checked
            .dependencies
            .get_mut("symbol-reserve")
            .unwrap();
        match change {
            "scope" => dependency.normalized["scope"] = json!("other-compilation"),
            "kind" => dependency.kind = "FLOW".into(),
            _ => dependency.normalized["semantic"] = json!({"provider":"other-provider"}),
        }
        dependency.digest = digest(&dependency.normalized).unwrap();
        incompatible
            .influence
            .insert(dependency.id.clone(), dependency.digest.clone());
        assert!(
            auth::validate_destination(
                &binding,
                &incompatible,
                &paragraph,
                edit,
                &work.checked,
                &state
            )
            .unwrap_err()
            .message
            .contains("provider, symbol, source association or compiler scope"),
            "{change}"
        );
    }
    let mut unread = state.clone();
    unread.receipts.clear();
    assert!(
        auth::from_context_edit(&work, &paragraph, edit, &unread)
            .unwrap_err()
            .message
            .contains("DEPENDENCY")
    );
    let mut unscoped = work.clone();
    unscoped.influence.remove("symbol-reserve");
    assert!(
        auth::from_context_edit(&unscoped, &paragraph, edit, &state)
            .unwrap_err()
            .message
            .contains("in-scope")
    );
    let mut foreign_state = state.clone();
    foreign_state.work = "foreign-work".into();
    assert!(
        auth::from_context_edit(&work, &paragraph, edit, &foreign_state)
            .unwrap_err()
            .message
            .contains("this Work")
    );
}
