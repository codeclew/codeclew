use super::endpoint_publication::Selector;
use super::{
    check::{Check, SOURCE_INPUTS_SCHEMA, SourceInputs},
    cli,
    endpoint_publication::*,
    model::*,
    store::Repository,
};
use clap::Parser;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn evidence(service: &str) -> ServiceEvidence {
    let symbol = "method:class:Fixture#submit()V";
    let mut evidence = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.into(),
        revision: "a".repeat(40),
        service_digest: "synthetic".into(),
        extractor: EXTRACTOR.into(),
        runtime_mode: "STATIC".into(),
        coverage: "SEMANTIC".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    };
    for scope in ["A", "B"] {
        let id = format!("decl:{scope}");
        let normalized = json!({"scope":scope,"declarationKind":"METHOD","symbolIdentity":symbol});
        evidence.observations.insert(
            id.clone(),
            Observation {
                id: id.clone(),
                kind: "SYMBOL".into(),
                service: service.into(),
                symbol: symbol.into(),
                digest: super::digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
        for ordinal in 0..2 {
            evidence.entrypoints.push(Entrypoint {
                id: format!("entry:{scope}:{ordinal}"),
                service: service.into(),
                symbol: symbol.into(),
                kind: "METHOD".into(),
                trigger: json!({"path":format!("/{scope}/{ordinal}")}),
                source_ids: vec![],
                dependency_ids: vec![id.clone()],
                boundaries: vec![],
            });
        }
    }
    evidence
}

fn check(services: Vec<ServiceEvidence>) -> Check {
    Check {
        schema: "codeclew-documentation-check/1.0".into(),
        input_digest: "synthetic".into(),
        context_digest: "synthetic".into(),
        dependencies: services
            .iter()
            .flat_map(|e| e.observations.clone())
            .collect(),
        services: services
            .into_iter()
            .map(|e| (e.service.clone(), e))
            .collect(),
        unresolved: BTreeMap::new(),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        source_inputs: None,
        composition: None,
    }
}

fn root() -> (tempfile::TempDir, Repository) {
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Endpoint selection fixture").unwrap();
    let repository = Repository::open(temporary.path()).unwrap();
    (temporary, repository)
}

fn snapshot(repo: &Repository) -> String {
    let service: Service = serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0", "id":"fixture", "title":"Fixture",
        "repositoryId":"fixture", "repository":"https://example.invalid/fixture",
        "language":"java", "profile":"source-syntax", "targetRef":"main",
        "source":{"roots":["src"],"dialect":"17"}
    }))
    .unwrap();
    repo.service_add(service.clone(), Some(&repo.input_digest().unwrap()))
        .unwrap();
    let mut evidence = evidence("fixture");
    evidence.service_digest = super::digest(&service).unwrap();
    let mut checked = check(vec![evidence]);
    checked.input_digest = repo.input_digest().unwrap();
    checked.source_inputs = Some(SourceInputs {
        schema: SOURCE_INPUTS_SCHEMA.into(),
        input_digest: checked.input_digest.clone(),
        inputs: repo.inputs().unwrap(),
        selected_services: BTreeSet::from(["fixture".into()]),
        retained_services: BTreeSet::new(),
    });
    checked.refresh_digest().unwrap();
    checked.save_snapshot(repo).unwrap()
}

fn edit_args(repo: &Repository, snapshot: Option<&str>, endpoint: Option<&str>) -> EditArgs {
    EditArgs {
        root: repo.root.clone(),
        service: "fixture".into(),
        snapshot: snapshot.map(str::to_owned),
        endpoint: endpoint.map(str::to_owned),
        declaration: None,
        scope: None,
        symbol: None,
        expected_policy_digest: None,
    }
}

#[test]
fn absent_policy_defaults_to_inclusion_and_groups_without_scope_or_service_bleed() {
    let (_temporary, repo) = root();
    let mut policy = load(&repo).unwrap();
    assert!(policy.exclusions.is_empty());
    let checked = check(vec![evidence("fixture"), evidence("sibling")]);
    assert!(excluded_entrypoint_ids(&policy, &checked, "fixture").is_empty());
    let selector = selector_for_entrypoint(
        &checked.services["fixture"],
        &checked.services["fixture"].entrypoints[0],
    )
    .unwrap();
    assert_eq!(selector.scope, "A");
    policy.exclusions.insert(selector);
    assert_eq!(
        excluded_entrypoint_ids(&policy, &checked, "fixture"),
        BTreeSet::from(["entry:A:0".into(), "entry:A:1".into()])
    );
    assert!(excluded_entrypoint_ids(&policy, &checked, "sibling").is_empty());
}

#[test]
fn publication_edits_are_idempotent_and_leave_saved_check_inputs_unchanged() {
    let (_temporary, repo) = root();
    let snapshot = snapshot(&repo);
    let input_digest = repo.input_digest().unwrap();
    let original = super::bytes(&Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap();
    let result = cli::run(cli::Command::Endpoint {
        command: Command::Exclude(edit_args(&repo, Some(&snapshot), Some("entry:A:0"))),
    })
    .unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(result["entrypointIds"], json!(["entry:A:0", "entry:A:1"]));
    assert_eq!(
        run(Command::Exclude(edit_args(
            &repo,
            Some(&snapshot),
            Some("entry:A:1")
        )))
        .unwrap()["changed"],
        false
    );
    assert_eq!(repo.input_digest().unwrap(), input_digest);
    assert_eq!(
        super::bytes(&Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap(),
        original
    );
    assert!(
        repo.path("publication/endpoint-selection.json")
            .unwrap()
            .is_file()
    );
    assert_eq!(
        run(Command::Include(edit_args(
            &repo,
            Some(&snapshot),
            Some("entry:A:0")
        )))
        .unwrap()["changed"],
        true
    );
    assert_eq!(
        run(Command::Include(edit_args(
            &repo,
            Some(&snapshot),
            Some("entry:A:0")
        )))
        .unwrap()["changed"],
        false
    );
}

#[test]
fn policy_digest_race_prevents_any_mutation() {
    let (_temporary, repo) = root();
    let snapshot = snapshot(&repo);
    let before = load(&repo).unwrap().digest().unwrap();
    run(Command::Exclude(edit_args(
        &repo,
        Some(&snapshot),
        Some("entry:A:0"),
    )))
    .unwrap();
    let bytes = std::fs::read(repo.path("publication/endpoint-selection.json").unwrap()).unwrap();
    let mut args = edit_args(&repo, Some(&snapshot), Some("entry:B:0"));
    args.expected_policy_digest = Some(before);
    let error = run(Command::Exclude(args)).unwrap_err();
    assert_eq!(error.code, crate::error::ErrorCode::WwConflict);
    assert_eq!(
        std::fs::read(repo.path("publication/endpoint-selection.json").unwrap()).unwrap(),
        bytes
    );
}

#[test]
fn list_exposes_grouping_and_include_removes_unmatched_saved_key() {
    let (_temporary, repo) = root();
    let snapshot = snapshot(&repo);
    let list = run(Command::List(ListArgs {
        root: repo.root.clone(),
        service: "fixture".into(),
        snapshot: snapshot.clone(),
    }))
    .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    assert_eq!(
        list["items"][0]["entrypointIds"],
        json!(["entry:A:0", "entry:A:1"])
    );
    assert_eq!(list["items"][0]["selector"]["scope"], "A");
    let selector = Selector {
        service: "fixture".into(),
        scope: "disappeared".into(),
        symbol: "method:class:Fixture#removed()V".into(),
    };
    let mut policy = load(&repo).unwrap();
    policy.exclusions.insert(selector.clone());
    repo.atomic(
        "publication/endpoint-selection.json",
        &super::bytes(&policy).unwrap(),
    )
    .unwrap();
    let list = run(Command::List(ListArgs {
        root: repo.root.clone(),
        service: "fixture".into(),
        snapshot: snapshot.clone(),
    }))
    .unwrap();
    assert_eq!(list["exclusions"][0]["matched"], false);
    let mut args = edit_args(&repo, None, None);
    args.scope = Some(selector.scope);
    args.symbol = Some(selector.symbol);
    assert_eq!(
        run(Command::Include(args.clone())).unwrap()["changed"],
        true
    );
    assert_eq!(run(Command::Include(args)).unwrap()["changed"], false);
    assert!(load(&repo).unwrap().exclusions.is_empty());
}

#[test]
fn invalid_policy_records_and_unknown_endpoint_ids_fail_closed() {
    let (_temporary, repo) = root();
    let snapshot = snapshot(&repo);
    assert!(
        run(Command::Exclude(edit_args(
            &repo,
            Some(&snapshot),
            Some("unknown")
        )))
        .is_err()
    );
    assert!(run(Command::Exclude(edit_args(&repo, None, Some("entry:A:0")))).is_err());
    let selector =
        json!({"service":"fixture","scope":"A","symbol":"method:class:Fixture#submit()V"});
    for record in [
        json!({"schema":"wrong","exclusions":[]}),
        json!({"schema":"codeclew-endpoint-publication-policy/1.0","exclusions":[],"unknown":true}),
        json!({"schema":"codeclew-endpoint-publication-policy/1.0","exclusions":[selector.clone(),selector.clone()]}),
        json!({"schema":"codeclew-endpoint-publication-policy/1.0","exclusions":[{"service":"fixture","scope":"A","symbol":""}]}),
    ] {
        repo.atomic(
            "publication/endpoint-selection.json",
            &super::bytes(&record).unwrap(),
        )
        .unwrap();
        assert!(load(&repo).is_err());
    }
    repo.atomic("publication/endpoint-selection.json", b"{broken")
        .unwrap();
    assert!(load(&repo).is_err());
}

#[test]
fn ambiguous_identity_cannot_be_excluded_and_unrelated_unsupported_endpoints_are_safe() {
    let mut evidence = evidence("fixture");
    evidence.entrypoints[0].dependency_ids.push("decl:B".into());
    assert!(selector_for_entrypoint(&evidence, &evidence.entrypoints[0]).is_err());
    evidence.entrypoints[0].dependency_ids = vec!["missing".into()];
    let checked = check(vec![evidence]);
    assert!(excluded_entrypoint_ids(&Policy::default(), &checked, "fixture").is_empty());
    assert_eq!(
        selectors_for_entrypoints(&checked.services["fixture"]).len(),
        3
    );
}

#[test]
fn bulk_entrypoint_resolution_skips_duplicate_ids_and_preserves_exact_scopes() {
    let mut evidence = evidence("fixture");
    let selectors = selectors_for_entrypoints(&evidence);
    assert_eq!(selectors.len(), 4);
    assert_eq!(selectors["entry:A:0"].scope, "A");
    assert_eq!(selectors["entry:B:0"].scope, "B");
    let mut conflicting = evidence.entrypoints[2].clone();
    conflicting.id = evidence.entrypoints[0].id.clone();
    evidence.entrypoints.push(conflicting);
    assert!(!selectors_for_entrypoints(&evidence).contains_key("entry:A:0"));
}

#[test]
fn syntax_callables_have_an_explicit_empty_scope_and_native_declarations_resolve() {
    let mut evidence = evidence("fixture");
    evidence.extractor = SOURCE_EXTRACTOR.into();
    let declaration = evidence.observations.get_mut("decl:A").unwrap();
    declaration.normalized =
        json!({"authority":"SYNTAX","syntaxKind":"method_declaration","documentation":{}});
    let selector = selector_for_declaration(&evidence, "decl:A").unwrap();
    assert_eq!(selector.scope, "");
    assert_eq!(selector.symbol, evidence.entrypoints[0].symbol);
    assert_eq!(
        selector_for_entrypoint(&evidence, &evidence.entrypoints[0]).unwrap(),
        selector
    );
}

#[test]
fn cli_requires_explicit_snapshot_for_listing_and_exact_selector_pairs() {
    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: cli::Command,
    }
    assert!(
        Cli::try_parse_from([
            "clew-docs",
            "endpoint",
            "list",
            "--root",
            "/tmp/docs",
            "--service",
            "fixture"
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "clew-docs",
            "endpoint",
            "exclude",
            "--root",
            "/tmp/docs",
            "--service",
            "fixture",
            "--scope",
            "A"
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "clew-docs",
            "endpoint",
            "exclude",
            "--root",
            "/tmp/docs",
            "--service",
            "fixture",
            "--endpoint",
            "entry:A:0",
            "--declaration",
            "decl:A"
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "clew-docs",
            "endpoint",
            "exclude",
            "--root",
            "/tmp/docs",
            "--service",
            "fixture",
            "--snapshot",
            "sha256:saved/1",
            "--declaration",
            "decl:A"
        ])
        .is_ok()
    );
}

fn status_baseline(
    repo: &Repository,
    selection: Option<super::endpoint_display::PublicationSelection>,
    narrative: Option<Narrative>,
) -> String {
    let bundle = "b".repeat(64);
    let overview = format!("<!-- codeclew-bundle {bundle} -->\n");
    let mut binding: super::bindings::Bindings = serde_json::from_value(json!({
        "schema":"codeclew-documentation-bindings/1.4", "inputDigest":repo.input_digest().unwrap(),
        "renderer":RENDERER, "extractor":EXTRACTOR, "endpointPublication":selection,
        "influenceScopes":{}, "revisions":{}, "coverage":{}, "catalogues":{}, "fragments":{},
        "observations":{}, "narratives":{}, "outputHashes":{}, "retainedSources":{},
        "sectionStates":{}, "targetRevisions":{}, "updateFailures":{}
    }))
    .unwrap();
    binding.output_hashes.insert(
        "root-overview.html".into(),
        crate::canonical::hash_bytes(overview.as_bytes()),
    );
    if let Some(narrative) = narrative {
        let ids: Vec<_> = narrative
            .operations
            .iter()
            .map(|operation| json!({"id":operation.id}))
            .collect();
        let data = json!({"title":"Fixture", "subject":narrative.subject, "operations":narrative.operations,
            "catalogue":ids, "notes":[], "gaps":{}, "translationGaps":{},
            "operationSources":{"hidden-op":[],"visible-op":[]},
            "operationContracts":{"hidden-op":[],"visible-op":[]}});
        let path = format!("docs/generated/{bundle}/services/fixture.json");
        let encoded = super::bytes(&data).unwrap();
        repo.atomic(&path, &encoded).unwrap();
        binding.output_hashes.insert(
            "services/fixture.json".into(),
            crate::canonical::hash_bytes(&encoded),
        );
        binding
            .narratives
            .insert(narrative.subject.clone(), narrative);
    }
    repo.atomic(
        &format!("docs/generated/{bundle}/root-overview.html"),
        overview.as_bytes(),
    )
    .unwrap();
    repo.atomic(
        &format!("docs/generated/{bundle}/bindings.json"),
        &super::bytes(&binding).unwrap(),
    )
    .unwrap();
    repo.atomic("docs/index.html", overview.as_bytes()).unwrap();
    bundle
}

#[test]
fn status_refresh_rejects_policy_change_before_source_observation() {
    let (_temporary, repo) = root();
    let bundle = status_baseline(&repo, None, None);
    let original = std::fs::read(repo.path("docs/index.html").unwrap()).unwrap();
    let mut policy = Policy::default();
    policy.exclusions.insert(Selector {
        service: "fixture".into(),
        scope: "A".into(),
        symbol: "method:Fixture#submit()V".into(),
    });
    repo.atomic(
        "publication/endpoint-selection.json",
        &super::bytes(&policy).unwrap(),
    )
    .unwrap();
    // Source observation would fail on this input. Policy rejection must happen first.
    repo.atomic("services/fixture.yaml", b"{broken").unwrap();
    let error = super::status::refresh(&repo).unwrap_err();
    assert_eq!(error.code, crate::error::ErrorCode::WwConflict);
    assert!(
        error
            .message
            .contains("docs render --snapshot <snapshot> --publish")
    );
    assert_eq!(
        std::fs::read(repo.path("docs/index.html").unwrap()).unwrap(),
        original
    );
    assert_eq!(super::bindings::baseline(&repo).unwrap().unwrap().0, bundle);
}

#[test]
fn status_refresh_accepts_absent_policy_with_legacy_or_explicit_default_selection() {
    for explicit_selection in [false, true] {
        let (_temporary, repo) = root();
        let selection = explicit_selection.then(|| super::endpoint_display::PublicationSelection {
            policy_digest: Policy::default().digest().unwrap(),
            retained_pages: BTreeMap::new(),
            selectors: BTreeMap::new(),
        });
        let bundle = status_baseline(&repo, selection, None);
        let result = super::status::refresh(&repo).unwrap();
        assert_eq!(result["status"], "UNCHANGED");
        assert_eq!(result["bundle"], bundle);
        assert_eq!(result["agentInvocations"], 0);
        assert_eq!(result["captures"], 0);
    }
}

#[test]
fn status_refresh_keeps_hidden_operations_out_of_reader_files_and_diagrams() {
    let (_temporary, repo) = root();
    let mut policy = Policy::default();
    let hidden = Selector {
        service: "fixture".into(),
        scope: "A".into(),
        symbol: "method:Fixture#submit()V".into(),
    };
    let visible = Selector {
        scope: "B".into(),
        ..hidden.clone()
    };
    policy.exclusions.insert(hidden.clone());
    repo.atomic(
        "publication/endpoint-selection.json",
        &super::bytes(&policy).unwrap(),
    )
    .unwrap();
    let selection = super::endpoint_display::PublicationSelection {
        policy_digest: policy.digest().unwrap(),
        retained_pages: BTreeMap::new(),
        selectors: BTreeMap::from([(
            "fixture".into(),
            BTreeMap::from([("hidden-op".into(), hidden), ("visible-op".into(), visible)]),
        )]),
    };
    let operations = ["hidden-op", "visible-op"]
        .into_iter()
        .map(|id| Operation {
            documentation_language: None,
            visuals: vec![],
            dataflow: None,
            id: id.into(),
            title: id.into(),
            summary: Fragment {
                id: "summary".into(),
                text: format!("{id} explanation"),
                dependency_ids: vec![],
                source_ids: vec![],
            },
            assessment: None,
            explanation: vec![],
            interface_contracts: vec![],
            overview_diagram: None,
            participants: vec![],
            events: vec![],
            findings: vec![],
            boundaries: vec![],
        })
        .collect();
    let narrative = Narrative {
        schema: "synthetic".into(),
        subject: "service:fixture".into(),
        context_digest: "synthetic".into(),
        operations,
        gaps: BTreeMap::new(),
    };
    status_baseline(&repo, Some(selection), Some(narrative));
    let result = super::status::refresh(&repo).unwrap();
    assert_eq!(result["status"], "PUBLISHED");
    let bundle = result["bundle"].as_str().unwrap();
    let base = format!("docs/generated/{bundle}");
    let data: serde_json::Value = super::store::read(
        &repo.path(&format!("{base}/services/fixture.json")).unwrap(),
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(data["operations"].as_array().unwrap().len(), 1);
    assert_eq!(data["operations"][0]["id"], "visible-op");
    assert!(data["operationStates"].get("hidden-op").is_none());
    assert!(data["operationSources"].get("hidden-op").is_none());
    for suffix in ["md", "html"] {
        let text = std::fs::read_to_string(
            repo.path(&format!("{base}/services/fixture.{suffix}"))
                .unwrap(),
        )
        .unwrap();
        assert!(!text.contains("hidden-op"));
        assert!(text.contains("visible-op"));
    }
    assert!(
        !repo
            .path(&format!("{base}/diagrams/service-fixture-hidden-op.mmd"))
            .unwrap()
            .exists()
    );
    assert!(
        repo.path(&format!("{base}/diagrams/service-fixture-visible-op.mmd"))
            .unwrap()
            .is_file()
    );
    let binding = super::bindings::baseline(&repo).unwrap().unwrap().1;
    assert_eq!(binding.narratives["service:fixture"].operations.len(), 2);
    assert_eq!(
        super::status::refresh(&repo).unwrap()["status"],
        "UNCHANGED"
    );
}
