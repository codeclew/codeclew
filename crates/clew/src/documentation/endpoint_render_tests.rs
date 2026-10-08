//! Exercise publication selection through retained snapshots and real bundles.
use super::{
    bindings::{self, Bindings},
    check::{Check, SOURCE_INPUTS_SCHEMA, SourceInputs},
    cli,
    endpoint_publication::{self, Command, EditArgs},
    model::*,
    render,
    store::{self, Repository},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const SUBJECT: &str = "service:fixture";
const SELECTED: &str = "entry-submit";
const ALIAS: &str = "entry-submit-alias";
const NEIGHBOR: &str = "entry-inspect";
const CUSTOM_GAP: &str =
    "The alternate submission registration needs an accepted failure explanation.";

struct Fixture {
    _temporary: tempfile::TempDir,
    repo: Repository,
    checked: Check,
    snapshot: String,
    narrative: Narrative,
}

fn observe(
    evidence: &mut ServiceEvidence,
    id: &str,
    kind: &str,
    symbol: &str,
    normalized: Value,
    source: &str,
) {
    evidence.observations.insert(
        id.into(),
        Observation {
            id: id.into(),
            kind: kind.into(),
            service: evidence.service.clone(),
            symbol: symbol.into(),
            digest: super::digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![source.into()],
        },
    );
}

fn operation(id: &str, method: &str, prose: &str) -> Operation {
    let declaration = format!("decl-{method}");
    let source = format!("source-{method}");
    serde_json::from_value(json!({
        "id":id, "title":format!("Accepted {method} operation"),
        "summary":{"id":"summary","text":format!("Explain the retained {method} decision."),
            "dependencyIds":[declaration],"sourceIds":[source]},
        "explanation":[{"id":"decision","text":prose,"eventIds":["decision-step"],
            "dependencyIds":[declaration],"sourceIds":[source],"detail":false}],
        "participants":[{"id":"caller","label":"Caller","service":"fixture"},
            {"id":"handler","label":"Handler","service":"fixture"}],
        "events":[{"id":"decision-step","kind":"note","text":format!("Record the {method} decision"),
            "from":"handler","dependencyIds":[declaration],"sourceIds":[source]}],
        "findings":[], "boundaries":[]
    })).unwrap()
}

fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Endpoint render fixture").unwrap();
    let repo = Repository::open(temporary.path()).unwrap();
    // A saved render cannot obtain this service from a checkout or remote.
    let service: Service = serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0","id":"fixture","title":"Fixture",
        "repositoryId":"unavailable-fixture","repository":"https://example.invalid/fixture",
        "language":"java","profile":"source-syntax","targetRef":"main",
        "source":{"roots":["src"],"dialect":"17"}
    }))
    .unwrap();
    repo.service_add(service.clone(), Some(&repo.input_digest().unwrap()))
        .unwrap();
    let revision = "a".repeat(40);
    let mut evidence = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.id.clone(),
        revision: revision.clone(),
        service_digest: super::digest(&service).unwrap(),
        extractor: EXTRACTOR.into(),
        runtime_mode: "STATIC".into(),
        coverage: "SEMANTIC".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    };
    for (method, entry, route) in [
        ("submit", SELECTED, "/submit"),
        ("inspect", NEIGHBOR, "/inspect"),
    ] {
        let symbol = format!("method:class:Fixture#{method}()V");
        let declaration = format!("decl-{method}");
        let source_id = format!("source-{method}");
        let text = format!("void {method}() {{ recordDecision(); }}");
        let source: Source = serde_json::from_value(json!({
            "id":source_id,"service":"fixture","revision":revision,"file":format!("{method}.java"),
            "startLine":1,"endLine":1,"text":text,
            "textDigest":crate::canonical::hash_bytes(text.as_bytes()),
            "evidenceDigest":crate::canonical::hash_bytes(format!("retained-{method}").as_bytes()),
            "authority":"NATIVE_COMPILER"
        }))
        .unwrap();
        evidence.sources.insert(source_id.clone(), source);
        observe(
            &mut evidence,
            &declaration,
            "SYMBOL",
            &symbol,
            json!({
                "schema":"codeclew-java-compiler-fact/1.0","kind":"DECLARATION",
                "declarationKind":"METHOD","symbolIdentity":symbol,"name":method,
                "ownerIdentity":"class:Fixture","scope":":fixture:main","resolution":"COMPILER_EXACT",
                "documentation":{"authority":"JAVAC_SOURCE_STRUCTURE",
                    "events":[{"kind":"CALL","target":"recordDecision()","depth":0}],
                    "parameterTypes":[],"boundaries":[]}
            }),
            &source_id,
        );
        observe(
            &mut evidence,
            &format!("route-{method}"),
            "ENTRYPOINT",
            &symbol,
            json!({"scope":":fixture:main","trigger":{"methods":["POST"],"paths":[route]}}),
            &source_id,
        );
        evidence.entrypoints.push(Entrypoint {
            id: entry.into(),
            service: "fixture".into(),
            symbol: symbol.clone(),
            kind: "HTTP_ENDPOINT".into(),
            trigger: json!({"methods":["POST"],"paths":[route]}),
            source_ids: vec![source_id.clone()],
            dependency_ids: vec![declaration, format!("route-{method}")],
            boundaries: vec![],
        });
        observe(
            &mut evidence,
            &format!("contract-{method}"),
            "CONTRACT_OPERATION",
            &symbol,
            json!({"entrypoint":entry,"method":"POST","path":route,
                "operation":{"operationId":method,"responses":{"200":{"description":"Recorded decision"}}}}),
            &source_id,
        );
    }
    let mut alias = evidence
        .entrypoints
        .iter()
        .find(|e| e.id == SELECTED)
        .unwrap()
        .clone();
    alias.id = ALIAS.into();
    alias.trigger = json!({"methods":["POST"],"paths":["/alternate-submit"]});
    evidence.entrypoints.push(alias);
    super::analysis::verify_evidence(&evidence).unwrap();
    let input_digest = repo.input_digest().unwrap();
    let mut checked = Check {
        schema: "codeclew-documentation-check/1.0".into(),
        input_digest: input_digest.clone(),
        context_digest: String::new(),
        dependencies: evidence.observations.clone(),
        services: BTreeMap::from([("fixture".into(), evidence)]),
        unresolved: BTreeMap::new(),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        source_inputs: Some(SourceInputs {
            schema: SOURCE_INPUTS_SCHEMA.into(),
            input_digest,
            inputs: repo.inputs().unwrap(),
            selected_services: BTreeSet::from(["fixture".into()]),
            retained_services: BTreeSet::new(),
        }),
        composition: None,
    };
    checked.refresh_digest().unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let mut gaps: BTreeMap<_, _> = super::sections::ids()
        .map(|id| (id, "Outside this focused retained endpoint fixture.".into()))
        .collect();
    gaps.insert(ALIAS.into(), CUSTOM_GAP.into());
    let mut narrative = Narrative {
        schema: "codeclew-documentation-narrative/1.3".into(),
        subject: SUBJECT.into(),
        context_digest: checked.context_digest.clone(),
        operations: vec![
            operation(
                SELECTED,
                "submit",
                "A submitted request records its business decision before returning to the caller.",
            ),
            operation(
                NEIGHBOR,
                "inspect",
                "An inspection returns the recorded decision without changing the accepted submission explanation.",
            ),
        ],
        gaps,
    };
    narrative.operations.sort_by(|a, b| a.id.cmp(&b.id));
    render::validate(&narrative, &checked).unwrap();
    Fixture {
        _temporary: temporary,
        repo,
        checked,
        snapshot,
        narrative,
    }
}

fn publish(repo: &Repository, snapshot: &str, incoming: Vec<Narrative>) -> (Value, Bindings) {
    let result =
        render::publish_from_snapshot(repo, incoming, false, BTreeMap::new(), snapshot).unwrap();
    assert_eq!(result["updateFailures"], json!({}));
    let (bundle, binding) = bindings::baseline(repo).unwrap().unwrap();
    assert_eq!(result["bundle"], bundle);
    bindings::verify_outputs(repo, &bundle, &binding).unwrap();
    (result, binding)
}

fn render_narrative_files(fixture: &Fixture, inputs: &[Value]) -> Value {
    let paths = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let path = fixture
                ._temporary
                .path()
                .join(format!("incoming-{index}.json"));
            std::fs::write(&path, serde_json::to_vec(input).unwrap()).unwrap();
            path
        })
        .collect();
    cli::run(cli::Command::Render {
        root: fixture.repo.root.clone(),
        language: None,
        input: paths,
        require_complete: false,
        refresh: false,
        snapshot: Some(fixture.snapshot.clone()),
        publish: false,
    })
    .unwrap()
}

#[test]
fn narrative_rejection_reports_actual_retained_publication_without_applying_valid_siblings() {
    let fixture = fixture();
    let (first, before) = publish(
        &fixture.repo,
        &fixture.snapshot,
        vec![fixture.narrative.clone()],
    );
    let mut rejected = serde_json::to_value(&fixture.narrative).unwrap();
    rejected["operations"][0]["summary"]["text"] = json!("REJECTED_PRIVATE_SUMMARY");
    rejected["operations"][1]["interaction"] = json!("REJECTED_PRIVATE_VALUE");
    let result = render_narrative_files(&fixture, &[rejected]);
    assert_eq!(
        result["narrativeInputSummary"]["rejectedIncomingNarratives"],
        1
    );
    assert_eq!(
        result["narrativeInputSummary"]["retainedNarrativeUsed"],
        true
    );
    assert_eq!(result["narrativeInputSummary"]["scope"], "PUBLICATION");
    let failure = &result["updateFailures"]["input-0"];
    assert_eq!(failure["diagnostics"][0]["field"], "interaction");
    assert_eq!(failure["diagnostics"][0]["path"], "operations[1]");
    assert_eq!(failure["publication"]["retainedNarrativeUsed"], true);
    assert_eq!(result["inputDigest"], first["inputDigest"]);
    let (_, after) = bindings::baseline(&fixture.repo).unwrap().unwrap();
    assert_eq!(after.narratives[SUBJECT], before.narratives[SUBJECT]);
    for output in [
        result.to_string(),
        std::fs::read_to_string(fixture.repo.path("docs/index.html").unwrap()).unwrap(),
        std::fs::read_to_string(file(&fixture.repo, &result, "status.json")).unwrap(),
    ] {
        assert!(!output.contains("REJECTED_PRIVATE_SUMMARY"));
        assert!(!output.contains("REJECTED_PRIVATE_VALUE"));
        assert!(output.contains("retained authored narrative content"));
    }
}

#[test]
fn narrative_rejection_without_retained_authored_content_reports_no_fallback() {
    let fixture = fixture();
    let mut rejected = serde_json::to_value(&fixture.narrative).unwrap();
    rejected["operationId"] = json!("REJECTED_PRIVATE_VALUE");
    let result = render_narrative_files(&fixture, &[rejected.clone()]);
    assert_eq!(
        result["narrativeInputSummary"]["retainedNarrativeUsed"],
        false
    );
    let (_, binding) = bindings::baseline(&fixture.repo).unwrap().unwrap();
    assert!(binding.narratives[SUBJECT].operations.is_empty());
    // A previous bundle containing generated placeholders is also insufficient
    // to claim that authored narrative content was reused.
    let repeated = render_narrative_files(&fixture, &[rejected]);
    assert_eq!(
        repeated["narrativeInputSummary"]["retainedNarrativeUsed"],
        false
    );
}

#[test]
fn narrative_rejection_mixed_with_accepted_complete_input_does_not_claim_retained_fallback() {
    let fixture = fixture();
    publish(
        &fixture.repo,
        &fixture.snapshot,
        vec![fixture.narrative.clone()],
    );
    let mut rejected = serde_json::to_value(&fixture.narrative).unwrap();
    rejected["interaction"] = json!("REJECTED_PRIVATE_VALUE");
    let mut accepted = fixture.narrative.clone();
    for operation in &mut accepted.operations {
        operation.summary.text = "A replacement decision is accepted from the second input.".into();
    }
    let result = render_narrative_files(
        &fixture,
        &[rejected, serde_json::to_value(&accepted).unwrap()],
    );
    assert_eq!(
        result["narrativeInputSummary"]["retainedNarrativeUsed"],
        false
    );
    assert_eq!(
        result["narrativeInputSummary"]["rejectedIncomingNarratives"],
        1
    );
    let (_, binding) = bindings::baseline(&fixture.repo).unwrap().unwrap();
    assert_eq!(binding.narratives[SUBJECT].operations, accepted.operations);
    assert!(!result.to_string().contains("REJECTED_PRIVATE_VALUE"));
    let valid_result = render_narrative_files(&fixture, &[serde_json::to_value(accepted).unwrap()]);
    assert_eq!(valid_result["updateFailures"], json!({}));
    assert!(valid_result.get("narrativeInputSummary").is_none());
}

fn file(repo: &Repository, result: &Value, relative: &str) -> std::path::PathBuf {
    repo.path(&format!(
        "docs/generated/{}/{relative}",
        result["bundle"].as_str().unwrap()
    ))
    .unwrap()
}

fn page(repo: &Repository, result: &Value) -> Value {
    store::read(
        &file(repo, result, "services/fixture.json"),
        store::MAX_RECORD,
    )
    .unwrap()
}

fn catalog(repo: &Repository, result: &Value) -> Value {
    let html = std::fs::read_to_string(file(repo, result, "catalog.html")).unwrap();
    let payload = html
        .split("id=\"catalog-data\" type=\"application/json\">")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    serde_json::from_str(payload).unwrap()
}

fn edit(repo: &Repository, snapshot: &str, exclude: bool) {
    let args = EditArgs {
        root: repo.root.clone(),
        service: "fixture".into(),
        snapshot: Some(snapshot.into()),
        endpoint: Some(SELECTED.into()),
        declaration: None,
        scope: None,
        symbol: None,
        expected_policy_digest: None,
    };
    let command = if exclude {
        Command::Exclude(args)
    } else {
        Command::Include(args)
    };
    cli::run(cli::Command::Endpoint { command }).unwrap();
}

fn find_operation(data: &Value, id: &str) -> Value {
    data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()
        .clone()
}

fn assert_hidden(repo: &Repository, result: &Value, selected_ids: &[&str]) {
    let data = page(repo, result);
    for id in selected_ids {
        for field in ["operations", "catalogue"] {
            assert!(
                data[field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|row| row["id"] != *id),
                "{field}: {id}"
            );
        }
        for field in [
            "gaps",
            "operationSources",
            "operationContracts",
            "operationStates",
            "translationGaps",
        ] {
            assert!(data[field].get(*id).is_none(), "{field}: {id}");
        }
        assert!(
            data["boundaryInventory"]["publicBoundaries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["id"] != *id)
        );
        assert!(
            data["contracts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["normalized"]["entrypoint"] != *id)
        );
        for extension in ["mmd", "puml", "svg"] {
            assert!(
                !file(
                    repo,
                    result,
                    &format!("diagrams/service-fixture-{id}.{extension}")
                )
                .exists()
            );
        }
        let browse = catalog(repo, result);
        assert!(!browse.to_string().contains(&format!("#{id}")));
        let html = std::fs::read_to_string(file(repo, result, "services/fixture.html")).unwrap();
        assert!(!html.contains(&format!("href=\"#{id}\"")));
        assert!(!html.contains(&format!("id=\"{id}\"")));
    }
    for relative in [
        "services/fixture.html",
        "services/fixture.md",
        "catalog.html",
    ] {
        let text = std::fs::read_to_string(file(repo, result, relative)).unwrap();
        assert!(!text.contains("Accepted submit operation"), "{relative}");
        assert!(
            !text.contains("A submitted request records its business decision"),
            "{relative}"
        );
        assert!(!text.contains(CUSTOM_GAP), "{relative}");
    }
    let browse = catalog(repo, result);
    assert!(browse.to_string().contains(&format!("#{NEIGHBOR}")));
    assert!(
        std::fs::read_to_string(file(repo, result, "services/fixture.md"))
            .unwrap()
            .contains("An inspection returns the recorded decision")
    );
}

#[test]
fn saved_render_exclusion_and_inclusion_preserve_accepted_prose_and_exact_delivery() {
    let fixture = fixture();
    let before_check =
        super::bytes(&Check::load_snapshot(&fixture.repo, &fixture.snapshot).unwrap()).unwrap();
    let input_digest = fixture.repo.input_digest().unwrap();
    let (first, accepted) = publish(
        &fixture.repo,
        &fixture.snapshot,
        vec![fixture.narrative.clone()],
    );
    let first_page = page(&fixture.repo, &first);
    let neighbor = find_operation(&first_page, NEIGHBOR);
    assert_eq!(accepted.narratives[SUBJECT], fixture.narrative);
    assert!(
        file(
            &fixture.repo,
            &first,
            &format!("diagrams/service-fixture-{SELECTED}.mmd")
        )
        .is_file()
    );
    assert!(
        file(
            &fixture.repo,
            &first,
            &format!("diagrams/service-fixture-{ALIAS}.puml")
        )
        .is_file()
    );

    edit(&fixture.repo, &fixture.snapshot, true);
    let (excluded, retained) = publish(&fixture.repo, &fixture.snapshot, vec![]);
    assert_ne!(first["bundle"], excluded["bundle"]);
    assert_eq!(excluded["documentedOperations"], 1);
    assert_eq!(excluded["excludedEndpoints"], 2);
    assert_hidden(&fixture.repo, &excluded, &[SELECTED, ALIAS]);
    assert_eq!(
        find_operation(&page(&fixture.repo, &excluded), NEIGHBOR),
        neighbor
    );
    assert_eq!(retained.narratives[SUBJECT], accepted.narratives[SUBJECT]);
    assert_eq!(retained.retained_sources, accepted.retained_sources);
    assert_eq!(retained.observations, accepted.observations);
    for (id, fragment) in &accepted.fragments {
        assert_eq!(
            retained.fragments[id].content_digest, fragment.content_digest,
            "{id}"
        );
    }
    // Repeating a saved render must not resurrect accepted hidden content.
    let (again, repeated) = publish(&fixture.repo, &fixture.snapshot, vec![]);
    assert_hidden(&fixture.repo, &again, &[SELECTED, ALIAS]);
    assert_eq!(repeated.narratives[SUBJECT], accepted.narratives[SUBJECT]);

    edit(&fixture.repo, &fixture.snapshot, false);
    let (included, restored) = publish(&fixture.repo, &fixture.snapshot, vec![]);
    let included_page = page(&fixture.repo, &included);
    assert_eq!(included["documentedOperations"], 2);
    assert_eq!(included["excludedEndpoints"], 0);
    assert_eq!(restored.narratives[SUBJECT], accepted.narratives[SUBJECT]);
    assert_eq!(
        find_operation(&included_page, SELECTED),
        find_operation(&first_page, SELECTED)
    );
    assert_eq!(find_operation(&included_page, NEIGHBOR), neighbor);
    assert_eq!(
        included_page["operationSources"][SELECTED],
        first_page["operationSources"][SELECTED]
    );
    assert_eq!(
        included_page["operationContracts"][SELECTED],
        first_page["operationContracts"][SELECTED]
    );
    assert_eq!(included_page["gaps"][ALIAS], CUSTOM_GAP);
    assert!(
        std::fs::read_to_string(file(&fixture.repo, &included, "services/fixture.md"))
            .unwrap()
            .contains("A submitted request records its business decision")
    );
    assert!(
        catalog(&fixture.repo, &included)
            .to_string()
            .contains(&format!("#{SELECTED}"))
    );
    assert_eq!(fixture.repo.input_digest().unwrap(), input_digest);
    assert_eq!(
        super::bytes(&Check::load_snapshot(&fixture.repo, &fixture.snapshot).unwrap()).unwrap(),
        before_check
    );
}

#[test]
fn excluded_retained_operation_stays_hidden_when_callable_disappears_and_new_registrations_return()
{
    let fixture = fixture();
    let (first, accepted) = publish(
        &fixture.repo,
        &fixture.snapshot,
        vec![fixture.narrative.clone()],
    );
    let original_page = page(&fixture.repo, &first);
    assert!(accepted.endpoint_publication.is_none());

    let mut disappeared = fixture.checked.clone();
    let evidence = disappeared.services.get_mut("fixture").unwrap();
    evidence.entrypoints.retain(|entry| entry.id == NEIGHBOR);
    evidence
        .observations
        .retain(|_, observation| observation.symbol != "method:class:Fixture#submit()V");
    evidence.sources.remove("source-submit");
    disappeared.dependencies = evidence.observations.clone();
    disappeared.refresh_digest().unwrap();
    let disappeared_snapshot = disappeared.save_snapshot(&fixture.repo).unwrap();
    // The first filtered publication already lacks the callable. Its retained
    // operation must resolve against the original unfiltered publication.
    edit(&fixture.repo, &fixture.snapshot, true);
    let policy_digest = endpoint_publication::load(&fixture.repo)
        .unwrap()
        .digest()
        .unwrap();
    let (absent, retained) = publish(&fixture.repo, &disappeared_snapshot, vec![]);
    assert_hidden(&fixture.repo, &absent, &[SELECTED, ALIAS]);
    assert_eq!(
        retained.narratives[SUBJECT].operations,
        accepted.narratives[SUBJECT].operations
    );
    assert_eq!(
        retained.retained_sources["source-submit"],
        accepted.retained_sources["source-submit"]
    );
    assert_eq!(
        retained
            .endpoint_publication
            .as_ref()
            .unwrap()
            .policy_digest,
        policy_digest
    );

    // Registration IDs changed, while the saved exact callable identity did not.
    let mut reappeared = fixture.checked.clone();
    let evidence = reappeared.services.get_mut("fixture").unwrap();
    for entry in &mut evidence.entrypoints {
        if entry.id == SELECTED {
            entry.id = "entry-submit-new".into();
        }
        if entry.id == ALIAS {
            entry.id = "entry-submit-new-alias".into();
        }
    }
    let contract = evidence.observations.get_mut("contract-submit").unwrap();
    contract.normalized["entrypoint"] = json!("entry-submit-new");
    contract.digest = super::digest(&contract.normalized).unwrap();
    reappeared.dependencies = evidence.observations.clone();
    reappeared.refresh_digest().unwrap();
    let returned_snapshot = reappeared.save_snapshot(&fixture.repo).unwrap();
    let (returned, _) = publish(&fixture.repo, &returned_snapshot, vec![]);
    assert_hidden(
        &fixture.repo,
        &returned,
        &[
            SELECTED,
            ALIAS,
            "entry-submit-new",
            "entry-submit-new-alias",
        ],
    );
    assert_eq!(
        endpoint_publication::load(&fixture.repo)
            .unwrap()
            .digest()
            .unwrap(),
        policy_digest
    );

    // Including through the original snapshot restores the retained operation,
    // whose exact source and contract versions predate the new registrations.
    edit(&fixture.repo, &fixture.snapshot, false);
    let (included, restored) = publish(&fixture.repo, &returned_snapshot, vec![]);
    let included_page = page(&fixture.repo, &included);
    assert_eq!(
        restored.narratives[SUBJECT].operations,
        accepted.narratives[SUBJECT].operations
    );
    assert_eq!(
        included_page["operationSources"][SELECTED],
        original_page["operationSources"][SELECTED]
    );
    assert_eq!(
        included_page["operationContracts"][SELECTED],
        original_page["operationContracts"][SELECTED]
    );
    assert_eq!(
        find_operation(&included_page, SELECTED),
        find_operation(&original_page, SELECTED)
    );
    for id in [SELECTED, "entry-submit-new", "entry-submit-new-alias"] {
        assert!(
            included_page["catalogue"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["id"] == id)
        );
    }
}

#[test]
fn legacy_portable_bindings_without_selection_render_with_default_inclusion() {
    let fixture = fixture();
    let (first, binding) = publish(
        &fixture.repo,
        &fixture.snapshot,
        vec![fixture.narrative.clone()],
    );
    let mut legacy = serde_json::to_value(&binding).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("endpointPublication");
    let restored =
        bindings::validate_retained_bindings(&fixture.repo, &super::bytes(&legacy).unwrap())
            .unwrap();
    assert!(restored.endpoint_publication.is_none());
    let policy = endpoint_publication::load(&fixture.repo).unwrap();
    let selection = super::endpoint_display::PublicationSelection::prepare(
        &policy,
        &fixture.checked,
        Some(&restored),
        &BTreeMap::from([(SUBJECT.into(), page(&fixture.repo, &first))]),
    )
    .unwrap();
    assert!(selection.hidden(&policy, SUBJECT).is_empty());
    let (rendered, after) = publish(&fixture.repo, &fixture.snapshot, vec![]);
    assert_eq!(rendered["documentedOperations"], 2);
    assert_eq!(after.narratives[SUBJECT], restored.narratives[SUBJECT]);
    assert!(
        catalog(&fixture.repo, &rendered)
            .to_string()
            .contains(&format!("#{SELECTED}"))
    );
}

#[test]
fn excluded_endpoint_failures_leave_the_reader_but_remain_in_retained_analysis() {
    let fixture = fixture();
    publish(
        &fixture.repo,
        &fixture.snapshot,
        vec![fixture.narrative.clone()],
    );
    edit(&fixture.repo, &fixture.snapshot, true);
    let selected_failure = json!({"reason":"SELECTED_REFRESH_FAILED",
        "nextAction":"Submission explanation needs a fresh source review."});
    let neighbor_failure = json!({"reason":"NEIGHBOR_REFRESH_FAILED",
        "nextAction":"Inspection explanation needs a fresh source review."});
    let failures = BTreeMap::from([
        (format!("{SUBJECT}/{SELECTED}"), selected_failure),
        (format!("{SUBJECT}/{NEIGHBOR}"), neighbor_failure.clone()),
    ]);
    // An actual rejected gap proposal creates the renderer-owned gap- prefix.
    let mut invalid_gap_input = fixture.narrative.clone();
    invalid_gap_input.gaps.insert(ALIAS.into(), String::new());
    let result = render::publish_from_snapshot(
        &fixture.repo,
        vec![invalid_gap_input],
        false,
        failures.clone(),
        &fixture.snapshot,
    )
    .unwrap();
    let (bundle, binding) = bindings::baseline(&fixture.repo).unwrap().unwrap();
    assert_eq!(result["bundle"], bundle);
    bindings::verify_outputs(&fixture.repo, &bundle, &binding).unwrap();
    for (key, failure) in &failures {
        assert_eq!(&binding.update_failures[key], failure);
    }
    let invalid_gap_key = format!("{SUBJECT}/gap-{ALIAS}");
    assert_eq!(
        binding.update_failures[&invalid_gap_key]["reason"],
        "INVALID_GAP"
    );
    assert_eq!(binding.update_failures.len(), failures.len() + 1);
    let retained_failures = binding.update_failures.clone();
    let data = page(&fixture.repo, &result);
    assert!(
        data["updateFailures"]
            .get(format!("{SUBJECT}/{SELECTED}"))
            .is_none()
    );
    assert_eq!(
        data["updateFailures"][format!("{SUBJECT}/{NEIGHBOR}")],
        neighbor_failure
    );
    assert!(data["updateFailures"].get(&invalid_gap_key).is_none());
    for path in [
        fixture.repo.path("docs/index.html").unwrap(),
        file(&fixture.repo, &result, "root-overview.html"),
        file(&fixture.repo, &result, "overview.html"),
        file(&fixture.repo, &result, "services/fixture.html"),
    ] {
        let html = std::fs::read_to_string(path).unwrap();
        assert!(!html.contains("SELECTED_REFRESH_FAILED"));
        assert!(!html.contains("Submission explanation needs a fresh source review."));
        assert!(!html.contains("INVALID_GAP"));
        assert!(!html.contains(&format!("gap-{ALIAS}")));
        assert!(html.contains("NEIGHBOR_REFRESH_FAILED"));
        assert!(html.contains("Inspection explanation needs a fresh source review."));
    }
    // A later saved render retains the same failures without reintroducing the
    // excluded endpoint's failure into the current reader overview.
    let repeated = render::publish_from_snapshot(
        &fixture.repo,
        vec![],
        false,
        BTreeMap::new(),
        &fixture.snapshot,
    )
    .unwrap();
    let (_, repeated_binding) = bindings::baseline(&fixture.repo).unwrap().unwrap();
    assert_eq!(repeated_binding.update_failures, retained_failures);
    let overview =
        std::fs::read_to_string(file(&fixture.repo, &repeated, "overview.html")).unwrap();
    assert!(!overview.contains("SELECTED_REFRESH_FAILED"));
    assert!(!overview.contains("INVALID_GAP"));
    assert!(overview.contains("NEIGHBOR_REFRESH_FAILED"));
}

#[test]
fn source_declaration_inventory_exclusion_preserves_the_same_symbol_in_another_scope() {
    let fixture = fixture();
    let mut checked = fixture.checked.clone();
    let evidence = checked.services.get_mut("fixture").unwrap();
    for entry in &mut evidence.entrypoints {
        if [SELECTED, ALIAS].contains(&entry.id.as_str()) {
            entry.kind = "SOURCE_DECLARATION".into();
            entry.trigger = json!({"authority":"SYNTAX"});
        }
    }
    let mut another_scope = evidence.observations["decl-submit"].clone();
    another_scope.id = "decl-submit-test".into();
    another_scope.normalized["scope"] = json!(":fixture:test");
    another_scope.digest = super::digest(&another_scope.normalized).unwrap();
    evidence
        .observations
        .insert(another_scope.id.clone(), another_scope.clone());
    evidence.entrypoints.push(Entrypoint {
        id: "entry-submit-test".into(),
        service: "fixture".into(),
        symbol: another_scope.symbol,
        kind: "SOURCE_DECLARATION".into(),
        trigger: json!({"authority":"SYNTAX"}),
        source_ids: another_scope.source_ids,
        dependency_ids: vec!["decl-submit-test".into()],
        boundaries: vec![],
    });
    let catalogue = serde_json::to_value(&evidence.entrypoints).unwrap();
    let inventory = super::sections::inventory("fixture", &checked);
    let test_scope = inventory["internalCallables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "decl-submit-test")
        .unwrap()
        .clone();
    assert!(
        inventory["internalCallables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "decl-submit")
    );
    let mut data = json!({"catalogue":catalogue,"boundaryInventory":inventory});
    super::endpoint_display::filter_page(
        &mut data,
        &BTreeSet::from([SELECTED.into(), ALIAS.into()]),
    );
    let internal = data["boundaryInventory"]["internalCallables"]
        .as_array()
        .unwrap();
    assert!(internal.iter().all(|row| row["id"] != "decl-submit"));
    assert_eq!(
        internal.iter().find(|row| row["id"] == "decl-submit-test"),
        Some(&test_scope)
    );
    assert!(
        data["catalogue"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "entry-submit-test")
    );
    assert!(
        data["boundaryInventory"]["publicBoundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == NEIGHBOR)
    );
}
