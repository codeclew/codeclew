//! Native publication selection uses retained evidence without a source checkout.
use super::*;
use crate::documentation::{
    check::{SOURCE_INPUTS_SCHEMA, SourceInputs},
    endpoint_publication::{self, Policy},
    model::{Observation, Service, ServiceEvidence, Source},
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

fn evidence(service: &str) -> ServiceEvidence {
    let mut evidence = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.into(),
        revision: "a".repeat(40),
        service_digest: "synthetic".into(),
        extractor: crate::documentation::model::EXTRACTOR.into(),
        runtime_mode: "STATIC".into(),
        coverage: "SEMANTIC".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    };
    for (id, scope, name, text) in [
        ("endpoint-a", "A", "submit", "void submit() {}"),
        ("endpoint-b", "B", "submit", "void submit() {}"),
        ("neighbor", "A", "neighbor", "void neighbor() { submit(); }"),
        ("worker", "A", "tick", "void tick() {}"),
    ] {
        let symbol = format!("method:class:Fixture#{name}()V");
        let normalized = json!({"schema":"codeclew-java-compiler-fact/1.0",
            "kind":"DECLARATION", "declarationKind":"METHOD", "name":name,
            "symbolIdentity":symbol, "ownerIdentity":"class:Fixture", "scope":scope,
            "resolution":"COMPILER_EXACT", "jvmDescriptor":"()V"});
        let source_id = format!("src-{id}");
        let source: Source = serde_json::from_value(json!({
            "id":source_id, "service":service, "revision":evidence.revision,
            "file":format!("{id}.java"), "startLine":1, "endLine":1, "text":text,
            "textDigest":crate::canonical::hash_bytes(text.as_bytes()),
            "evidenceDigest":"retained-compiler-receipt", "authority":"EXACT_SNAPSHOT_TEXT",
            "occurrence":{"snapshot":"fixture", "blob":crate::canonical::hash_bytes(text.as_bytes()),
                "startByte":0,"endByte":text.len()}, "url":null
        })).unwrap();
        evidence.sources.insert(source_id.clone(), source);
        evidence.observations.insert(
            id.into(),
            Observation {
                id: id.into(),
                kind: "SYMBOL".into(),
                service: service.into(),
                symbol,
                digest: crate::documentation::digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![source_id],
            },
        );
    }
    // This retained call still reaches an excluded endpoint's body. Its
    // standalone process membership must disappear while this source link stays.
    let owner = evidence.observations["neighbor"].clone();
    let mut source = evidence.sources["src-neighbor"].clone();
    let start = source.text.find("submit()").unwrap();
    source.id = "call-site".into();
    source.text = "submit()".into();
    source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
    source.occurrence.as_mut().unwrap().start_byte = start;
    source.occurrence.as_mut().unwrap().end_byte = start + source.text.len();
    let normalized = json!({"sourceIdentity":owner.symbol,
        "targetIdentity":evidence.observations["endpoint-a"].symbol,
        "scope":"A", "resolution":"COMPILER_EXACT", "relationKind":"CALLS",
        "callSite":{"sourceId":source.id, "sourceStatus":"SOURCE_RETAINED",
            "sourceDigest":source.text_digest,"evidenceDigest":source.evidence_digest,
            "byteStart":start,"byteEnd":start + source.text.len()}});
    evidence.observations.insert(
        "relation".into(),
        Observation {
            id: "relation".into(),
            kind: "CALL_RELATION".into(),
            service: service.into(),
            symbol: owner.symbol,
            digest: crate::documentation::digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![source.id.clone()],
        },
    );
    evidence.sources.insert(source.id.clone(), source);
    evidence
}

fn checked() -> Check {
    let services = ["sample", "sibling"]
        .into_iter()
        .map(|service| (service.into(), evidence(service)))
        .collect::<BTreeMap<_, _>>();
    Check {
        schema: "codeclew-documentation-check/1.0".into(),
        input_digest: "synthetic".into(),
        context_digest: "synthetic".into(),
        dependencies: services
            .values()
            .flat_map(|e| e.observations.clone())
            .collect(),
        services,
        unresolved: BTreeMap::new(),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        source_inputs: None,
        composition: None,
    }
}

fn selection(id: &str, service: &str, endpoint: &str) -> model::Selection {
    model::Selection {
        id: id.into(),
        service: service.into(),
        endpoint_declaration: endpoint.into(),
        worker_declaration: "worker".into(),
        wiring_declaration: None,
        question: None,
        note_ids: vec![],
        authored_paragraphs: vec![],
        expand_source_calls: true,
        expand_data_state: true,
    }
}

fn excluded(checked: &Check) -> Policy {
    let mut policy = Policy::default();
    policy.exclusions.insert(
        endpoint_publication::selector_for_declaration(&checked.services["sample"], "endpoint-a")
            .unwrap(),
    );
    policy
}

#[test]
fn exact_callable_excludes_all_selection_ids_without_scope_service_or_callee_bleed() {
    let checked = checked();
    let before = crate::documentation::digest(&checked).unwrap();
    let selections = vec![
        selection("first", "sample", "endpoint-a"),
        selection("second", "sample", "endpoint-a"),
        selection("other-scope", "sample", "endpoint-b"),
        selection("other-service", "sibling", "endpoint-a"),
        selection("neighbor", "sample", "neighbor"),
    ];
    let policy = excluded(&checked);
    let projection = project::project_with_policy(&checked, &selections, &policy).unwrap();
    assert_eq!(
        projection
            .pages
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["other-scope", "other-service", "neighbor"]
    );
    let effective: Vec<_> = selections[2..].to_vec();
    assert_eq!(
        projection.selection_digest,
        crate::documentation::digest(&effective).unwrap()
    );
    assert_eq!(
        projection.requested_selection_digest,
        Some(crate::documentation::digest(&selections).unwrap())
    );
    assert_eq!(
        projection.endpoint_publication_policy_digest,
        Some(policy.digest().unwrap())
    );
    let graph = projection.source_call_graph.as_ref().unwrap();
    assert!(graph.nodes.values().any(|node| node.service == "sample"
        && node.scope == "A"
        && node.callable.declaration_id == "endpoint-a"));
    assert!(
        graph
            .nodes
            .values()
            .flat_map(|node| &node.calls)
            .any(
                |call| call.target_declaration.as_deref() == Some("endpoint-a")
                    && call.target_node.is_some()
            )
    );
    assert!(
        graph
            .process_links
            .iter()
            .all(|link| link.from_process != "first"
                && link.from_process != "second"
                && link.to_process != "first"
                && link.to_process != "second")
    );
    assert!(
        graph
            .reverse_examined_processes
            .values()
            .flatten()
            .all(|row| row.process_id != "first" && row.process_id != "second")
    );
    assert!(
        projection
            .pages
            .iter()
            .all(|page| page.data_state.is_some())
    );
    assert_eq!(crate::documentation::digest(&checked).unwrap(), before);
    let temporary = tempfile::tempdir().unwrap();
    let output = temporary.path().join("mixed");
    let manifest = publish::write(&output, "fixture/1", &projection).unwrap();
    assert!(!output.join("first-overview.html").exists());
    assert!(!output.join("second-overview.html").exists());
    assert!(output.join("neighbor-overview.html").is_file());
    let index = fs::read_to_string(output.join("index.html")).unwrap();
    assert!(!index.contains("first-overview.html") && !index.contains("second-overview.html"));
    assert_eq!(
        manifest["effectiveSelectionIds"],
        json!(["other-scope", "other-service", "neighbor"])
    );
    assert!(
        manifest["reverseExaminedPages"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|rows| rows.as_array().unwrap())
            .all(|row| row["processId"] != "first" && row["processId"] != "second")
    );
}

#[test]
fn default_and_reincluded_policy_keep_the_original_projection_bytes() {
    let checked = checked();
    let selections = vec![selection("first", "sample", "endpoint-a")];
    let original = project::project_unresolved(&checked, &selections).unwrap();
    let mut policy = excluded(&checked);
    policy.exclusions.clear();
    let included = project::project_with_policy(&checked, &selections, &policy).unwrap();
    assert_eq!(
        serde_json::to_vec(&original).unwrap(),
        serde_json::to_vec(&included).unwrap()
    );
    let serialized = serde_json::to_value(&included).unwrap();
    assert!(serialized.get("endpointPublicationPolicyDigest").is_none());
    assert!(serialized.get("requestedSelectionDigest").is_none());
}

#[test]
fn exclusion_cannot_hide_invalid_declarations_notes_or_unsafe_selection_ids() {
    let checked = checked();
    let policy = excluded(&checked);
    let valid = selection("first", "sample", "endpoint-a");
    let mut missing = valid.clone();
    missing.worker_declaration = "missing".into();
    let mut unsafe_id = valid.clone();
    unsafe_id.id = "../outside".into();
    let mut note = valid.clone();
    note.note_ids = vec!["missing".into()];
    let mut invalid_expansion = valid.clone();
    invalid_expansion.expand_source_calls = false;
    for invalid in [missing, unsafe_id, note, invalid_expansion] {
        assert!(project::project_with_policy(&checked, &[invalid], &policy).is_err());
    }
    assert!(project::project_with_policy(&checked, &[valid.clone(), valid], &policy).is_err());
}

#[test]
fn unrelated_policy_keeps_valid_native_declaration_with_unresolved_publication_scope() {
    let mut checked = checked();
    let mut policy = Policy::default();
    policy.exclusions.insert(
        endpoint_publication::selector_for_declaration(&checked.services["sibling"], "endpoint-a")
            .unwrap(),
    );
    let observation = checked
        .services
        .get_mut("sample")
        .unwrap()
        .observations
        .get_mut("endpoint-a")
        .unwrap();
    observation
        .normalized
        .as_object_mut()
        .unwrap()
        .remove("scope");
    // The shared reader now requires compiler scope for compiler authority.
    // Keep this publication-policy case on genuine retained Java syntax.
    observation.normalized["schema"] = json!("syntax-only");
    observation.normalized["authority"] = json!("SYNTAX");
    observation.normalized["syntaxKind"] = json!("method_declaration");
    observation.digest = crate::documentation::digest(&observation.normalized).unwrap();
    let mut selected = selection("syntax-page", "sample", "endpoint-a");
    selected.expand_source_calls = false;
    selected.expand_data_state = false;
    assert!(project::project_unresolved(&checked, std::slice::from_ref(&selected)).is_ok());
    assert!(
        endpoint_publication::selector_for_declaration(&checked.services["sample"], "endpoint-a",)
            .is_err()
    );
    let projection = project::project_with_policy(&checked, &[selected], &policy).unwrap();
    assert_eq!(projection.pages.len(), 1);
    assert_eq!(projection.pages[0].id, "syntax-page");
    assert_eq!(projection.pages[0].endpoint.authority, "SYNTAX_SOURCE");
    assert_eq!(
        projection.endpoint_publication_policy_digest,
        Some(policy.digest().unwrap())
    );
}

fn repository_snapshot() -> (tempfile::TempDir, Repository, String) {
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Native publication fixture").unwrap();
    let repo = Repository::open(temporary.path()).unwrap();
    let mut checked = checked();
    for id in ["sample", "sibling"] {
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0","id":id,"title":id,
            "repositoryId":id,"repository":"https://example.invalid/fixture",
            "language":"java","profile":"source-syntax","targetRef":"main",
            "source":{"roots":["missing-checkout"],"dialect":"17"}
        }))
        .unwrap();
        repo.service_add(service.clone(), Some(&repo.input_digest().unwrap()))
            .unwrap();
        checked.services.get_mut(id).unwrap().service_digest =
            crate::documentation::digest(&service).unwrap();
    }
    checked.input_digest = repo.input_digest().unwrap();
    checked.source_inputs = Some(SourceInputs {
        schema: SOURCE_INPUTS_SCHEMA.into(),
        input_digest: checked.input_digest.clone(),
        inputs: repo.inputs().unwrap(),
        selected_services: BTreeSet::from(["sample".into(), "sibling".into()]),
        retained_services: BTreeSet::new(),
    });
    checked.refresh_digest().unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    (temporary, repo, snapshot)
}

fn edit(repo: &Repository, snapshot: &str, exclude: bool) {
    let args = endpoint_publication::EditArgs {
        root: repo.root.clone(),
        service: "sample".into(),
        snapshot: Some(snapshot.into()),
        endpoint: None,
        declaration: Some("endpoint-a".into()),
        scope: None,
        symbol: None,
        expected_policy_digest: None,
    };
    endpoint_publication::run(if exclude {
        endpoint_publication::Command::Exclude(args)
    } else {
        endpoint_publication::Command::Include(args)
    })
    .unwrap();
}

#[test]
fn persistent_policy_renders_all_excluded_into_new_or_empty_output_and_restores_without_checkout() {
    let (temporary, repo, snapshot) = repository_snapshot();
    let inputs_before = repo.input_digest().unwrap();
    let check_before =
        crate::documentation::digest(&Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap();
    let selections = vec![selection("first", "sample", "endpoint-a")];
    let input = temporary.path().join("selections.json");
    fs::write(&input, serde_json::to_vec(&selections).unwrap()).unwrap();
    edit(&repo, &snapshot, true);
    for name in ["new", "empty"] {
        let output = temporary.path().join(name);
        if name == "empty" {
            fs::create_dir(&output).unwrap();
        }
        let response = run(Command::Render {
            root: repo.root.clone(),
            snapshot: snapshot.clone(),
            input: input.clone(),
            output: output.clone(),
        })
        .unwrap();
        assert_eq!(response["selectedSelections"], 0);
        assert_eq!(response["excludedSelections"], 1);
        assert!(output.join("index.html").is_file());
        assert!(!output.join("first-overview.html").exists());
        let projection: Value = store::read(&output.join("projection.json"), 1024 * 1024).unwrap();
        let manifest: Value = store::read(&output.join("manifest.json"), 1024 * 1024).unwrap();
        let catalogue: Value = store::read(&output.join("catalogue.json"), 1024 * 1024).unwrap();
        assert_eq!(projection["pages"], json!([]));
        assert!(projection.get("sourceCallGraph").is_none());
        assert_eq!(manifest["effectiveSelectionIds"], json!([]));
        assert_eq!(
            manifest["endpointPublicationPolicyDigest"],
            response["endpointPublicationPolicyDigest"]
        );
        assert_eq!(catalogue["rows"], json!([]));
        assert!(
            run(Command::Render {
                root: repo.root.clone(),
                snapshot: snapshot.clone(),
                input: input.clone(),
                output: output.clone(),
            })
            .is_err()
        );
    }
    edit(&repo, &snapshot, false);
    let output = temporary.path().join("restored");
    let restored = run(Command::Render {
        root: repo.root.clone(),
        snapshot: snapshot.clone(),
        input: input.clone(),
        output: output.clone(),
    })
    .unwrap();
    assert_eq!(restored["selectedSelections"], 1);
    assert_eq!(restored["excludedSelections"], 0);
    assert!(output.join("first-overview.html").is_file());
    assert_eq!(repo.input_digest().unwrap(), inputs_before);
    assert_eq!(
        crate::documentation::digest(&Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap(),
        check_before
    );
    fs::write(&input, b"[]").unwrap();
    let rejected = temporary.path().join("invalid-empty-input");
    assert!(
        run(Command::Render {
            root: repo.root.clone(),
            snapshot,
            input,
            output: rejected.clone()
        })
        .is_err()
    );
    assert!(!rejected.exists());
}

#[test]
fn native_write_rejects_policy_race_and_preserves_nonempty_or_symlink_output() {
    let (temporary, repo, snapshot) = repository_snapshot();
    let checked = Check::load_snapshot(&repo, &snapshot).unwrap();
    let policy = endpoint_publication::load(&repo).unwrap();
    let projection = project::project_with_policy(
        &checked,
        &[selection("first", "sample", "endpoint-a")],
        &policy,
    )
    .unwrap();
    edit(&repo, &snapshot, true);
    let absent = temporary.path().join("race-output");
    let error = write_if_policy_current(
        &repo,
        &policy.digest().unwrap(),
        &absent,
        &snapshot,
        &projection,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::WwConflict);
    assert!(!absent.exists());
    let current = endpoint_publication::load(&repo).unwrap();
    let empty_projection = project::project_with_policy(
        &checked,
        &[selection("first", "sample", "endpoint-a")],
        &current,
    )
    .unwrap();
    let nonempty = temporary.path().join("nonempty");
    fs::create_dir(&nonempty).unwrap();
    fs::write(nonempty.join("user.txt"), "preserve").unwrap();
    assert!(
        write_if_policy_current(
            &repo,
            &current.digest().unwrap(),
            &nonempty,
            &snapshot,
            &empty_projection
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(nonempty.join("user.txt")).unwrap(),
        "preserve"
    );
    assert_eq!(fs::read_dir(&nonempty).unwrap().count(), 1);
    let link = temporary.path().join("symlink");
    let target = temporary.path().join("target");
    fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(
        write_if_policy_current(
            &repo,
            &current.digest().unwrap(),
            &link,
            &snapshot,
            &empty_projection
        )
        .is_err()
    );
    assert!(fs::read_dir(&target).unwrap().next().is_none());
}
