use super::*;
use crate::documentation::{
    bindings, check,
    model::{self, Service, ServiceEvidence, Source},
    review,
    store::{Repository, RepositoryInputs},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn service() -> Service {
    serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0",
        "id":"orders",
        "title":"Orders",
        "repositoryId":"orders",
        "repository":"https://example.invalid/orders",
        "language":"java",
        "profile":"java-17plus-maven-read-only",
        "compilations":[":/main"],
        "targetRef":"HEAD"
    }))
    .unwrap()
}

fn process(id: &str, linked_subviews: Vec<&str>) -> model::Scenario {
    serde_json::from_value(json!({
        "schema":"codeclew-documentation-process/1.0",
        "id":id,
        "title":id,
        "summary":format!("Summary for {id}"),
        "root":{"service":"orders"},
        "interactions":[],
        "maxDepth":4,
        "maxNodes":64,
        "process":{
            "scope":"Orders",
            "participants":["orders"],
            "objects":[],
            "trigger":"request",
            "outcomes":["accepted"],
            "linkedSubviews":linked_subviews
        }
    }))
    .unwrap()
}

fn source() -> Source {
    let text = "fun child() = 1";
    Source {
        id: "child-source".into(),
        service: "orders".into(),
        revision: "a".repeat(40),
        file: "src/Child.kt".into(),
        start_line: 1,
        end_line: 1,
        text: text.into(),
        text_digest: crate::canonical::hash_bytes(text.as_bytes()),
        evidence_digest: "sha256:evidence".into(),
        authority: "EXACT_SNAPSHOT_TEXT".into(),
        occurrence: None,
        url: None,
    }
}

fn operation() -> model::Operation {
    serde_json::from_value(json!({
        "id":"process-overview",
        "title":"Child operation",
        "summary":{
            "id":"child-summary",
            "text":"Child summary",
            "dependencyIds":[],
            "sourceIds":["child-source"]
        },
        "participants":[],
        "events":[],
        "findings":[],
        "boundaries":[]
    }))
    .unwrap()
}

fn request(path: &str) -> Value {
    json!({
        "schema":"codeclew-documentation-work-request/1.0",
        "audience":"Maintainers",
        "entrypoint":null,
        "contextProfile":null,
        "maxItems":20,
        "maxBytes":40960,
        "externalInputs":[path]
    })
}

fn accepted_version(path: &str, operation_digest: &str) -> review::AcceptedVersion {
    serde_json::from_value(json!({
        "schema":"codeclew-documentation-accepted-version/1.2",
        "previousNarrativeDigest":digest(&Option::<model::Narrative>::None).unwrap(),
        "work":"work-id",
        "proposal":"proposal-id",
        "invocation":null,
        "reviewDigest":null,
        "reviewerDriverDigest":null,
        "evidenceDigest":"sha256:evidence",
        "readDigest":"sha256:read",
        "operationDigest":operation_digest,
        "verification":"VERIFIED",
        "limitations":[],
        "sourceRevisions":{"orders":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
        "influence":{"scope":digest(&review::InfluenceScope::default()).unwrap()},
        "externalRequest":request(path),
        "externalFingerprint":"sha256:fingerprint"
    }))
    .unwrap()
}

fn inputs() -> RepositoryInputs {
    let services = BTreeMap::from([(String::from("orders"), service())]);
    RepositoryInputs {
        manifest: model::Manifest {
            schema: "codeclew-documentation/2.0".into(),
            title: "Architecture".into(),
        },
        services,
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::from([
            ("parent".into(), process("parent", vec!["child"])),
            ("child".into(), process("child", vec![])),
        ]),
        process_states: BTreeMap::new(),
        entities: BTreeMap::new(),
        notes: BTreeMap::new(),
        evidence_expectations: BTreeMap::new(),
        update_policies: BTreeMap::new(),
        update_state: crate::documentation::updates::State {
            schema: "codeclew-documentation-update-state/1.0".into(),
            targets: BTreeMap::new(),
        },
    }
}

fn process_state(title: &str) -> crate::documentation::process_states::ProcessStates {
    serde_yaml_ng::from_str(&format!(
        "schema: {}\nid: child\ntitle: {title}\ninitial: NEW\nfinal: [DONE]\nstates:\n  NEW: waiting\n  DONE: finished\ntransitions: []\n",
        crate::documentation::process_states::SCHEMA
    ))
    .unwrap()
}

fn bindings_bundle(index: &str, child_operation: &model::Operation) -> bindings::Bindings {
    let operation_digest = digest(child_operation).unwrap();
    let narrative = model::Narrative {
        schema: "codeclew-documentation-narrative/1.3".into(),
        subject: "scenario:child".into(),
        context_digest: "sha256:context".into(),
        operations: vec![child_operation.clone()],
        gaps: BTreeMap::new(),
    };
    let mut accepted_versions = BTreeMap::new();
    accepted_versions.insert(
        "scenario:child/process-overview".into(),
        accepted_version("manual/a.md", &operation_digest),
    );
    accepted_versions.insert(
        "scenario:unreachable/process-overview".into(),
        accepted_version("manual/b.md", "sha256:unreachable-operation"),
    );
    bindings::Bindings {
        documentation_language: None,
        influence_scopes: BTreeMap::from([(
            digest(&review::InfluenceScope::default()).unwrap(),
            review::InfluenceScope::default(),
        )]),
        schema: "codeclew-documentation-bindings/1.4".into(),
        input_digest: "sha256:bindings-input".into(),
        renderer: model::RENDERER.into(),
        extractor: "codeclew-documentation-source/1.0".into(),
        revisions: BTreeMap::from([("orders".into(), "a".repeat(40))]),
        coverage: BTreeMap::new(),
        catalogues: BTreeMap::new(),
        fragments: BTreeMap::new(),
        observations: BTreeMap::new(),
        narratives: BTreeMap::from([("scenario:child".into(), narrative)]),
        output_hashes: BTreeMap::from([(
            "root-overview.html".into(),
            crate::canonical::hash_bytes(index.as_bytes()),
        )]),
        retained_sources: BTreeMap::from([("child-source".into(), source())]),
        section_states: BTreeMap::new(),
        target_revisions: BTreeMap::new(),
        update_failures: BTreeMap::new(),
        accepted_versions,
    }
}

fn write_baseline(repo: &Repository, binding: &bindings::Bindings, index: &str) -> Vec<u8> {
    let bundle = "a".repeat(64);
    let raw = crate::canonical::bytes(binding).unwrap();
    repo.atomic("docs/index.html", index.as_bytes()).unwrap();
    repo.atomic(&format!("docs/generated/{bundle}/bindings.json"), &raw)
        .unwrap();
    raw
}

fn checked(inputs: &RepositoryInputs) -> Check {
    let source = source();
    let evidence = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: "orders".into(),
        revision: "a".repeat(40),
        service_digest: digest(&inputs.services["orders"]).unwrap(),
        extractor: "codeclew-documentation-source/1.0".into(),
        runtime_mode: "COMMITTED_SOURCE_NO_BUILD".into(),
        coverage: "SYNTAX".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::from([(source.id.clone(), source)]),
        contracts: BTreeMap::new(),
    };
    check::assemble(
        digest(inputs).unwrap(),
        BTreeMap::from([("orders".into(), evidence)]),
        BTreeMap::new(),
        &inputs.interactions,
        &inputs.scenarios,
    )
    .unwrap()
}

#[test]
fn captured_baseline_projects_child_versions_and_attach_is_pure() {
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Architecture").unwrap();
    let repo = Repository::open(temporary.path()).unwrap();
    repo.atomic("manual/a.md", b"A").unwrap();
    repo.atomic("manual/b.md", b"B").unwrap();
    let index = format!("<!-- codeclew-bundle {} -->\nroot\n", "a".repeat(64));
    let child_operation = operation();
    let binding = bindings_bundle(&index, &child_operation);
    let raw_binding = write_baseline(&repo, &binding, &index);
    let definitions = inputs();
    repo.service_add(service(), Some(&repo.input_digest().unwrap()))
        .unwrap();
    for (id, definition) in &definitions.scenarios {
        repo.atomic(
            &format!("scenarios/{id}.yaml"),
            &crate::canonical::bytes(definition).unwrap(),
        )
        .unwrap();
    }
    let association = json!({"schema":"codeclew-documentation-note-association/1.0", "id":"note", "title":"Note A", "service":"orders", "path":"notes/note.md", "targets":["service:orders"], "classification":"fact", "period":"Current"});
    repo.atomic(
        "catalog/notes/note.json",
        &crate::canonical::bytes(&association).unwrap(),
    )
    .unwrap();
    repo.atomic("notes/note.md", b"Protected note A").unwrap();
    let inputs_a = repo.inputs().unwrap();
    let retained_a = capture_retained(&repo, &inputs_a).unwrap();
    let receipt = retained_a.baseline.as_ref().unwrap();
    assert_eq!(
        receipt.index_digest,
        crate::canonical::hash_bytes(index.as_bytes())
    );
    assert_eq!(
        receipt.bindings_digest,
        crate::canonical::hash_bytes(&raw_binding)
    );
    assert_eq!(retained_a.versions.operations.len(), 1);
    assert_eq!(retained_a.versions.accepted_versions.len(), 2);
    assert_eq!(retained_a.scopes.len(), 2);

    let mut control = checked(&inputs_a);
    let mut captured = control.clone();
    attach(&inputs_a, &retained_a, &mut control).unwrap();
    assert_eq!(
        control.dependencies["process-component:parent:child"].normalized["status"],
        "ACCEPTED_CHILD"
    );
    let composition_a = Composition {
        schema: SCHEMA.into(),
        parent: "unused-by-guard".into(),
        composer: composer().unwrap(),
        input_digest: digest(&inputs_a).unwrap(),
        inputs: inputs_a.clone(),
        retained: retained_a.clone(),
    };

    repo.atomic("manual/a.md", b"B changed").unwrap();
    repo.atomic("manual/b.md", b"B changed too").unwrap();
    let mut binding_b = binding.clone();
    binding_b
        .accepted_versions
        .remove("scenario:child/process-overview");
    let index_b = index.replace("root", "root B");
    binding_b.output_hashes.insert(
        "root-overview.html".into(),
        crate::canonical::hash_bytes(index_b.as_bytes()),
    );
    write_baseline(&repo, &binding_b, &index_b);
    let mut child_b = process("child", vec![]);
    child_b.title = "Changed child B".into();
    repo.atomic(
        "scenarios/child.yaml",
        &crate::canonical::bytes(&child_b).unwrap(),
    )
    .unwrap();
    repo.atomic("notes/note.md", b"Protected note B").unwrap();
    let inputs_b = repo.inputs().unwrap();
    attach(&inputs_a, &retained_a, &mut captured).unwrap();
    assert_eq!(
        crate::canonical::bytes(&control).unwrap(),
        crate::canonical::bytes(&captured).unwrap()
    );

    let retained_b = capture_retained(&repo, &inputs_b).unwrap();
    let mut checked_b = checked(&inputs_b);
    attach(&inputs_b, &retained_b, &mut checked_b).unwrap();
    let component = checked_b
        .dependencies
        .get("process-component:parent:child")
        .unwrap();
    assert_eq!(component.normalized["status"], "GAP");
    assert_eq!(component.normalized["gap"], "LINKED_PROCESS_UNASSESSED");
    assert!(
        ensure_current(&repo, &composition_a)
            .unwrap_err()
            .message
            .contains("documentation input changed")
    );
    repo.atomic(
        "scenarios/child.yaml",
        &crate::canonical::bytes(&inputs_a.scenarios["child"]).unwrap(),
    )
    .unwrap();
    repo.atomic("notes/note.md", b"Protected note A").unwrap();
    assert!(
        ensure_current(&repo, &composition_a)
            .unwrap_err()
            .message
            .contains("retained documentation inputs changed")
    );
    write_baseline(&repo, &binding, &index);
    repo.atomic("manual/a.md", b"A").unwrap();
    repo.atomic("manual/b.md", b"B").unwrap();
    ensure_current(&repo, &composition_a).unwrap();
}

#[test]
fn recomposition_refreshes_state_declarations_from_the_captured_inputs() {
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Architecture").unwrap();
    let repo = Repository::open(temporary.path()).unwrap();
    repo.service_add(service(), Some(&repo.input_digest().unwrap()))
        .unwrap();
    let scenario = process("child", vec![]);
    repo.atomic(
        "scenarios/child.yaml",
        &crate::canonical::bytes(&scenario).unwrap(),
    )
    .unwrap();
    let original_states = process_state("Original child lifecycle");
    repo.atomic(
        "scenarios/child-states.yaml",
        serde_yaml_ng::to_string(&original_states)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();

    let original_inputs = repo.inputs().unwrap();
    let mut original_check = checked(&original_inputs);
    original_check.source_inputs = Some(check::SourceInputs {
        schema: check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest: digest(&original_inputs).unwrap(),
        inputs: original_inputs.clone(),
        selected_services: BTreeSet::from(["orders".into()]),
        retained_services: BTreeSet::new(),
    });
    let original_handle = original_check.save_snapshot(&repo).unwrap();

    let changed_states = process_state("Updated child lifecycle");
    repo.atomic(
        "scenarios/child-states.yaml",
        serde_yaml_ng::to_string(&changed_states)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let (recomposed, _) =
        crate::documentation::composition::recompose(&repo, &original_handle).unwrap();

    let retained_original = Check::load_snapshot(&repo, &original_handle).unwrap();
    assert_eq!(
        crate::documentation::process_states::captured(&retained_original, "child")
            .unwrap()
            .title,
        "Original child lifecycle"
    );
    assert_eq!(
        recomposed
            .source_inputs
            .as_ref()
            .unwrap()
            .inputs
            .process_states["child"]
            .title,
        "Original child lifecycle"
    );
    assert_eq!(
        crate::documentation::process_states::captured(&recomposed, "child")
            .unwrap()
            .title,
        "Updated child lifecycle"
    );
}
