//! Native publication selection uses retained evidence without a source checkout.
use super::*;
use crate::documentation::{
    check::{SOURCE_INPUTS_SCHEMA, SourceInputs},
    endpoint_publication::{self, Policy},
    model::{Observation, Service, ServiceEvidence, Source},
    reader,
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
        documentation_language: None,
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
fn pages_cli_validates_an_explicit_full_publication_language() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: Command,
    }
    let args = [
        "pages",
        "render",
        "--root",
        "root",
        "--snapshot",
        "snapshot",
        "--publication",
        "publication",
        "--output",
        "output",
    ];
    for language in ["en", "ru", "de"] {
        let parsed = Cli::try_parse_from(args.into_iter().chain(["--language", language]));
        if language == "de" {
            assert!(parsed.is_err());
        } else {
            let Command::Render {
                language: selected, ..
            } = parsed.unwrap().command;
            assert_eq!(selected.as_deref(), Some(language));
        }
    }
}

#[test]
fn explicit_pages_language_preserves_readable_original_guides_in_both_formats() {
    let (temporary, repo, snapshot) = repository_snapshot();
    let published = crate::documentation::render::publish_from_snapshot(
        &repo,
        vec![],
        false,
        BTreeMap::new(),
        &snapshot,
    )
    .unwrap();
    let publication = published["bundle"].as_str().unwrap();
    let current_index = fs::read(repo.path("docs/index.html").unwrap()).unwrap();
    let check = Check::load_snapshot(&repo, &snapshot).unwrap();
    let source = &check.services["sample"].sources["src-endpoint-a"];
    let reference = json!({"kind":"SOURCE","service":"sample","sourceId":source.id,"textDigest":source.text_digest,"startLine":source.start_line,"endLine":source.end_line});
    for (case, available) in [Some("ru"), Some("en"), None].into_iter().enumerate() {
        let title = if available == Some("ru") {
            "Сохранённый обзор"
        } else {
            "Original saved guide"
        };
        let block = json!({"title":"Original block","text":"On this page","evidence":[reference]});
        let mut plan = json!({"schema":"codeclew-system-reading-plan/1.0","publication":publication,"subject":"system:overview","title":title,"purpose":"Original source-bound purpose.","sections":[{"id":"start","title":"First original section","blocks":[block]},{"id":"confidence","title":"Original source and scope","blocks":[block]}],"conclusion":block});
        if let Some(language) = available {
            plan["language"] = json!(language);
        }
        let input = temporary.path().join(format!("guide-{case}.json"));
        let original_input = serde_json::to_vec_pretty(&plan).unwrap();
        fs::write(&input, &original_input).unwrap();
        for (format_name, format) in [
            ("html", PageFormat::Standalone),
            ("mdx", PageFormat::Docusaurus),
        ] {
            let output = temporary.path().join(format!("guide-{case}-{format_name}"));
            let rendered = run(Command::Render {
                source_export: Default::default(),
                root: repo.root.clone(),
                snapshot: snapshot.clone(),
                publication: Some(publication.into()),
                input: None,
                reading_plan: Some(input.clone()),
                contract_comparison: None,
                data_model: None,
                format,
                language: Some("ru".into()),
                output: output.clone(),
            })
            .unwrap();
            assert_eq!(rendered["documentationLanguage"], "ru");
            assert_eq!(rendered["requestedDocumentationLanguage"], "ru");
            assert_eq!(
                rendered["readingGuideTranslationComplete"],
                available == Some("ru")
            );
            let (body, original) = if format == PageFormat::Standalone {
                (
                    fs::read_to_string(output.join("overview.html")).unwrap(),
                    output.join("system-reading-guide.original.json"),
                )
            } else {
                (
                    fs::read_to_string(output.join("docs/overview.mdx")).unwrap(),
                    output.join("static/clew-original/system-reading-guide.original.json"),
                )
            };
            assert!(
                body.contains(title) && body.contains("On this page"),
                "original prose must remain readable"
            );
            assert!(
                body.contains(if format == PageFormat::Standalone {
                    "services/sample.html"
                } else {
                    "sources/"
                }),
                "unrelated technical navigation must remain"
            );
            if format == PageFormat::Docusaurus {
                assert!(output.join("docs/services/sample.mdx").is_file());
            }
            if available != Some("ru") {
                assert!(body.contains(if available.is_none() {
                    "Доступность на выбранном языке не установлена"
                } else {
                    "Требуется перевод"
                }));
                assert!(body.contains("system-reading-guide.original.json"));
            }
            let original: Value = serde_json::from_slice(&fs::read(original).unwrap()).unwrap();
            assert_eq!(original["title"], title);
            assert_eq!(original["language"], json!(available));
            assert_eq!(original["sections"][0]["blocks"][0]["text"], "On this page");
            assert_eq!(fs::read(&input).unwrap(), original_input);
        }
    }
    assert_eq!(
        fs::read(repo.path("docs/index.html").unwrap()).unwrap(),
        current_index
    );
    assert_eq!(
        Check::load_snapshot(&repo, &snapshot)
            .unwrap()
            .context_digest,
        check.context_digest
    );
}

#[test]
fn full_publication_catalog_and_history_inherit_the_frozen_publication_language() {
    let (temporary, repo, snapshot) = repository_snapshot();
    for language in ["en", "ru"] {
        let published = crate::documentation::render::publish_language(
            &repo,
            vec![],
            false,
            BTreeMap::new(),
            Some(&snapshot),
            false,
            Some(language),
        )
        .unwrap();
        let publication = published["bundle"].as_str().unwrap();
        let original = fs::read(
            repo.path(&format!("docs/generated/{publication}/bindings.json"))
                .unwrap(),
        )
        .unwrap();
        let output = temporary.path().join(format!("full-{language}"));
        let rendered = run(Command::Render {
            source_export: Default::default(),
            language: None,
            root: repo.root.clone(),
            snapshot: snapshot.clone(),
            input: None,
            publication: Some(publication.into()),
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
            format: PageFormat::Standalone,
            output: output.clone(),
        })
        .unwrap();
        assert_eq!(rendered["documentationLanguage"], language);
        for file in ["catalog.html", "history.html"] {
            let html = fs::read_to_string(output.join(file)).unwrap();
            assert!(
                html.contains(&format!("<html lang=\"{language}\">")),
                "wrong wrapper: {file}"
            );
            assert!(html.contains(reader::SCRIPT));
        }
        let history = fs::read_to_string(output.join("history.html")).unwrap();
        let title = if language == "ru" {
            "<h1>Выбранная публикация</h1>"
        } else {
            "<h1>Selected publication</h1>"
        };
        assert!(history.contains(title));
        assert!(history.contains(publication) && history.contains(&snapshot));
        assert_eq!(
            fs::read(output.join("frozen-bindings.json")).unwrap(),
            original
        );
    }
}

#[test]
fn full_publication_refuses_endpoint_exclusions_before_reading_or_writing_output() {
    let (temporary, repo, snapshot) = repository_snapshot();
    edit(&repo, &snapshot, true);
    let output = temporary.path().join("full-excluded");
    let error = run(Command::Render {
        source_export: Default::default(),
        language: None,
        format: PageFormat::Standalone,
        root: repo.root.clone(),
        snapshot,
        input: None,
        publication: Some("a".repeat(64)),
        reading_plan: None,
        contract_comparison: None,
        data_model: None,
        output: output.clone(),
    })
    .unwrap_err();
    assert!(error.message.contains("endpoint exclusions"));
    assert!(!output.exists());
}

#[test]
fn full_publication_render_preserves_previous_data_and_refuses_damaged_frozen_input() {
    let (temporary, repo, snapshot) = repository_snapshot();
    let published = crate::documentation::render::publish_from_snapshot(
        &repo,
        vec![],
        false,
        BTreeMap::new(),
        &snapshot,
    )
    .unwrap();
    let publication = published["bundle"].as_str().unwrap().to_owned();
    let previous_index = fs::read(repo.path("docs/index.html").unwrap()).unwrap();
    let output = temporary.path().join("full-valid");
    let rendered = run(Command::Render {
        source_export: Default::default(),
        language: None,
        format: PageFormat::Standalone,
        root: repo.root.clone(),
        snapshot: snapshot.clone(),
        input: None,
        publication: Some(publication.clone()),
        reading_plan: None,
        contract_comparison: None,
        data_model: None,
        output: output.clone(),
    })
    .unwrap();
    assert_eq!(rendered["status"], "RENDERED");
    for page in ["catalog.html", "history.html"] {
        let html = fs::read_to_string(output.join(page)).unwrap();
        assert!(html.contains("class=\"reader-nav\""));
        assert!(html.contains(reader::SCRIPT));
        assert!(html.contains(reader::STYLE));
    }
    assert!(output.join("index.html").is_file());
    assert!(!output.join("root-overview.html").exists());
    assert_eq!(
        fs::read(repo.path("docs/index.html").unwrap()).unwrap(),
        previous_index
    );
    let sentinel = output.join("keep.txt");
    fs::write(&sentinel, "previous output").unwrap();
    assert!(
        run(Command::Render {
            source_export: Default::default(),
            language: None,
            format: PageFormat::Standalone,
            root: repo.root.clone(),
            snapshot: snapshot.clone(),
            input: None,
            publication: Some(publication.clone()),
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
            output: output.clone()
        })
        .is_err()
    );
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "previous output");
    fs::write(
        repo.path(&format!("docs/generated/{publication}/catalog.html"))
            .unwrap(),
        "damaged frozen input",
    )
    .unwrap();
    let refused = temporary.path().join("full-refused");
    fs::create_dir(&refused).unwrap();
    assert!(
        run(Command::Render {
            source_export: Default::default(),
            language: None,
            format: PageFormat::Standalone,
            root: repo.root.clone(),
            snapshot,
            input: None,
            publication: Some(publication),
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
            output: refused.clone()
        })
        .is_err()
    );
    assert_eq!(fs::read_dir(refused).unwrap().count(), 0);
    assert_eq!(
        fs::read(repo.path("docs/index.html").unwrap()).unwrap(),
        previous_index
    );
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
            source_export: Default::default(),
            language: None,
            format: PageFormat::Standalone,
            root: repo.root.clone(),
            snapshot: snapshot.clone(),
            input: Some(input.clone()),
            publication: None,
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
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
                source_export: Default::default(),
                language: None,
                format: PageFormat::Standalone,
                root: repo.root.clone(),
                snapshot: snapshot.clone(),
                input: Some(input.clone()),
                publication: None,
                reading_plan: None,
                contract_comparison: None,
                data_model: None,
                output: output.clone(),
            })
            .is_err()
        );
    }
    edit(&repo, &snapshot, false);
    let output = temporary.path().join("restored");
    let restored = run(Command::Render {
        source_export: Default::default(),
        language: None,
        format: PageFormat::Standalone,
        root: repo.root.clone(),
        snapshot: snapshot.clone(),
        input: Some(input.clone()),
        publication: None,
        reading_plan: None,
        contract_comparison: None,
        data_model: None,
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
            source_export: Default::default(),
            language: None,
            format: PageFormat::Standalone,
            root: repo.root.clone(),
            snapshot,
            input: Some(input),
            publication: None,
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
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
        None,
        None,
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
            &empty_projection,
            None,
            None,
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
            &empty_projection,
            None,
            None,
        )
        .is_err()
    );
    assert!(fs::read_dir(&target).unwrap().next().is_none());
}

#[test]
fn selected_language_preserves_question_sources_and_format_boundaries() {
    let (temporary, repo, snapshot) = repository_snapshot();
    for available in [None, Some("en"), Some("ru")] {
        let mut selected = selection("question", "sample", "endpoint-a");
        selected.question = Some("Source appendix <Probe>{kept} café\r\n".into());
        selected.documentation_language = available.map(str::to_owned);
        let input = temporary.path().join("language-selection.json");
        fs::write(&input, serde_json::to_vec(&vec![&selected]).unwrap()).unwrap();
        let checked = Check::load_snapshot(&repo, &snapshot).unwrap();
        let input_digest = checked.input_digest.clone();
        let before = crate::documentation::digest(&checked).unwrap();
        let output = temporary
            .path()
            .join(format!("selected-{}", available.unwrap_or("unknown")));
        let command = |format, language, output| Command::Render {
            source_export: Default::default(),
            root: repo.root.clone(),
            snapshot: snapshot.clone(),
            input: Some(input.clone()),
            publication: None,
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
            format,
            language,
            output,
        };
        let result = run(command(
            PageFormat::Standalone,
            Some("ru".into()),
            output.clone(),
        ))
        .unwrap();
        let matches = available == Some("ru");
        assert_eq!(result["translationComplete"], matches);
        assert_eq!(
            result["status"],
            if matches { "RENDERED" } else { "PARTIAL" }
        );
        assert_eq!(result["inputDigest"], input_digest);
        let original: Value =
            store::read(&output.join("question.question.original.json"), 1024 * 1024).unwrap();
        assert_eq!(
            original["selection"],
            serde_json::to_value(&selected).unwrap()
        );
        let projection: Value = store::read(&output.join("projection.json"), 1024 * 1024).unwrap();
        assert_eq!(
            projection["pages"][0]["selection"]["question"],
            selected.question.as_ref().unwrap().as_str()
        );
        assert_eq!(
            before,
            crate::documentation::digest(&Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap()
        );
        for name in ["question-diagnostic.html", "question-diagnostic.mdx"] {
            let rendered = fs::read_to_string(output.join(name)).unwrap();
            assert!(rendered.contains("lang=\"ru\""));
            assert!(
                rendered.contains("Source appendix &lt;Probe&gt;&#123;kept&#125; café&#13;&#10;")
            );
            assert!(!rendered.contains("<Probe>"));
            assert_eq!(
                rendered.contains("question.question.original.json"),
                !matches
            );
            if available.is_none() {
                assert!(rendered.contains("Доступность на выбранном языке не установлена"));
                assert!(!rendered.contains("Требуется перевод"));
            }
        }
        let refused = temporary
            .path()
            .join(format!("unsupported-{}", available.unwrap_or("unknown")));
        assert!(
            run(command(
                PageFormat::Docusaurus,
                Some("ru".into()),
                refused.clone()
            ))
            .is_err()
        );
        assert!(!refused.exists());
        assert!(
            run(command(
                PageFormat::Standalone,
                Some("de".into()),
                refused.clone()
            ))
            .is_err()
        );
        assert!(!refused.exists());
    }
    let mut invalid = selection("question", "sample", "endpoint-a");
    invalid.documentation_language = Some("de".into());
    assert!(project::project(&checked(), &[invalid]).is_err());
}

#[test]
fn selection_question_language_does_not_relabel_imported_prose() {
    let mut selected = selection("independent", "sample", "endpoint-a");
    selected.question = Some("Reviewed question".into());
    selected.documentation_language = Some("ru".into());
    let mut projection = project::project(&checked(), &[selected]).unwrap();
    // Synthetic immutable publisher input; this does not assert author acceptance.
    let paragraph: model::AuthoredParagraph = serde_json::from_value(json!({
        "selection":{"bundle":"frozen","operation":"op","fragment":"description"},
        "publicationDigest":"publication","bindingsDigest":"bindings","operationDigest":"operation",
        "paragraphDigest":"paragraph","documentationLanguage":"en","contextFreshness":"UNVERIFIED",
        "sourceRecords":{},"paragraph":{"id":"description","text":"Original paragraph <Data>{kept}",
        "eventIds":[],"dependencyIds":[],"sourceIds":[],"authorship":{"authority":"USER_DOCUMENTATION",
        "author":"Original author","meaningReview":"UNASSESSED","contextRole":"RETAINED_UNVERIFIED_CONTEXT",
        "editDigest":"edit","sourceSnapshot":"original-snapshot","sourceRefs":{},"dependencyRefs":{}}}
    })).unwrap();
    projection.pages[0]
        .authored_paragraphs
        .push(paragraph.clone());
    let note: model::HumanInstruction = serde_json::from_value(json!({
        "id":"operations", "title":"Original note", "declaredAuthor":"Maintainer", "classification":"policy",
        "period":"2026", "versionDigest":"version", "contentDigest":"content", "associationDigest":"association",
        "text":"Original instruction", "authority":"HUMAN_OR_IMPORTED_UNVERIFIED", "sourceClaimStatus":"UNASSESSED",
        "association":{"documentationLanguage":"en"}
    })).unwrap();
    projection.pages[0].human_instructions.push(note.clone());
    let before = serde_json::to_value(&projection).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let manifest =
        publish::write_language(temporary.path(), "snapshot", &projection, Some("ru")).unwrap();
    let gaps = manifest["translationGaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 2);
    assert_eq!(gaps[1]["kind"], "SELECTED_NOTE_TRANSLATION");
    assert_eq!(gaps[1]["availableLanguage"], "en");
    let original_note: Value = store::read(
        &temporary.path().join("independent.note-0.original.json"),
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(original_note, serde_json::to_value(note).unwrap());
    assert_eq!(gaps[0]["kind"], "SELECTED_PARAGRAPH_TRANSLATION");
    assert_eq!(gaps[0]["availableLanguage"], "en");
    let raw: Value = store::read(
        &temporary
            .path()
            .join("independent.paragraph-0.original.json"),
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(raw, serde_json::to_value(paragraph).unwrap());
    assert_eq!(serde_json::to_value(&projection).unwrap(), before);
    let overview = fs::read_to_string(temporary.path().join("independent-overview.mdx")).unwrap();
    assert!(overview.contains("Original paragraph &lt;Data&gt;&#123;kept&#125;"));
    assert!(overview.contains("UNASSESSED"));
}

#[test]
fn selected_native_locale_is_local_to_each_export_and_does_not_rewrite_evidence() {
    let mut selected = selection("locale", "sample", "endpoint-a");
    selected.question = Some("Source appendix".into());
    selected.documentation_language = Some("en".into());
    let projection = project::project(&checked(), &[selected]).unwrap();
    let original = serde_json::to_vec_pretty(&projection).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let mut english_files = BTreeMap::new();
    for (index, language) in ["en", "ru", "en"].into_iter().enumerate() {
        let folder = temporary.path().join(index.to_string());
        let manifest =
            publish::write_language(&folder, "same-snapshot", &projection, Some(language)).unwrap();
        assert_eq!(fs::read(folder.join("projection.json")).unwrap(), original);
        for name in [
            "locale-overview.html",
            "locale-diagnostic.mdx",
            "sources.html",
            "index.html",
        ] {
            let text = fs::read_to_string(folder.join(name)).unwrap();
            if language == "en" && index == 0 {
                english_files.insert(name, text);
            } else if language == "en" {
                assert_eq!(text, english_files[name]);
            } else {
                assert!(text.contains("lang=\"ru\""));
                if name == "index.html" {
                    assert!(text.contains("Документация по сохранённому исходному коду"));
                }
                if name == "locale-overview.html" {
                    assert!(text.contains("Страницы документации"));
                    assert!(text.contains("Приложение с исходными материалами"));
                    assert!(text.contains("Исходные материалы и версия"));
                }
                if name == "locale-diagnostic.mdx" {
                    assert!(text.contains("Source appendix"));
                }
            }
        }
        for file in manifest["files"].as_array().unwrap() {
            let bytes = fs::read(folder.join(file["path"].as_str().unwrap())).unwrap();
            assert_eq!(crate::canonical::hash_bytes(&bytes), file["digest"]);
        }
    }
    assert_eq!(serde_json::to_vec_pretty(&projection).unwrap(), original);
}

fn source_export_fixture() -> (
    tempfile::TempDir,
    Repository,
    Check,
    String,
    crate::documentation::source_export::Policy,
) {
    use crate::documentation::source_export::{
        Policy as ExportPolicy, RepositoryMapping, SourceMode,
    };
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Remote-source fixture").unwrap();
    let repo = Repository::open(temporary.path()).unwrap();
    let mut check = checked();
    let mut remote = ExportPolicy {
        mode: SourceMode::Remote,
        ..ExportPolicy::default()
    };
    for (id, origin, provider, base, file, start, text) in [
        (
            "sample",
            "https://github.com/example/render-fixture.git",
            "github",
            "https://github.com/example/render-fixture",
            "src/A file#.java",
            7,
            "void submit() {\n // CLEW_SOURCE_ONLY_20261020_0123456789\n int value=1;\n if (value>0) { return; }\n return;\n}",
        ),
        (
            "sibling",
            "ssh://git@git.example.invalid:2222/team/nested/render-fixture.git",
            "gitlab",
            "https://git.example.invalid:8443/gitlab/team/nested/render-fixture",
            "src/Пример.java",
            3,
            "void submit() {\n // CLEW_SOURCE_ONLY_20261020_0123456789\n int value=1;\n if (value>0) { return; }\n value++;\n return;\n}",
        ),
    ] {
        let service:Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":id,"title":id,"repositoryId":id,"repository":origin,"language":"java","profile":"source-syntax","targetRef":"main","source":{"roots":["not-a-live-checkout"],"dialect":"17"}})).unwrap();
        repo.service_add(service.clone(), Some(&repo.input_digest().unwrap()))
            .unwrap();
        let e = check.services.get_mut(id).unwrap();
        e.service_digest = crate::documentation::digest(&service).unwrap();
        let source = e.sources.get_mut("src-endpoint-a").unwrap();
        source.file = file.into();
        source.text = text.into();
        source.start_line = start;
        source.end_line = start + text.lines().count() as u64 - 1;
        source.text_digest = crate::canonical::hash_bytes(text.as_bytes());
        let occurrence = source.occurrence.as_mut().unwrap();
        occurrence.blob = source.text_digest.clone();
        occurrence.start_byte = 0;
        occurrence.end_byte = text.len();
        remote.repositories.insert(
            id.into(),
            RepositoryMapping {
                provider: provider.into(),
                web_base: base.into(),
                origin: None,
            },
        );
    }
    check.input_digest = repo.input_digest().unwrap();
    check.source_inputs = Some(SourceInputs {
        schema: SOURCE_INPUTS_SCHEMA.into(),
        input_digest: check.input_digest.clone(),
        inputs: repo.inputs().unwrap(),
        selected_services: BTreeSet::from(["sample".into(), "sibling".into()]),
        retained_services: BTreeSet::new(),
    });
    check.refresh_digest().unwrap();
    let snapshot = check.save_snapshot(&repo).unwrap();
    (temporary, repo, check, snapshot, remote)
}

#[test]
fn remote_source_exports_preserve_prose_without_any_source_body_attachment() {
    use crate::documentation::source_export::Policy as ExportPolicy;
    let (temporary, _repo, check, snapshot, remote) = source_export_fixture();
    let marker = "CLEW_SOURCE_ONLY_20261020_0123456789";
    let authored = "CLEW_AUTHORED_EXPLANATION_20261020_9876543210";
    let mut p = project::project(
        &check,
        &[
            selection("github", "sample", "endpoint-a"),
            selection("gitlab", "sibling", "endpoint-a"),
        ],
    )
    .unwrap();
    p.pages[0].human_instructions.push(model::HumanInstruction {
        id: "authored-note".into(),
        title: "Preserved authored explanation".into(),
        declared_author: "Fixture author".into(),
        classification: "USER_DOCUMENTATION".into(),
        period: "v1".into(),
        version_digest: "fixture-version".into(),
        content_digest: crate::canonical::hash_bytes(authored.as_bytes()),
        association_digest: "fixture-association".into(),
        text: authored.into(),
        authority: "HUMAN_OR_IMPORTED_UNVERIFIED".into(),
        source_claim_status: "UNASSESSED".into(),
        association: json!({}),
    });
    let output = std::env::var_os("CODECLEW_SOURCE_EXPORT_FIXTURE_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.path().join("exports"));
    fs::create_dir_all(&output).unwrap();
    let embedded = publish::write_source_policy(
        &output.join("embedded"),
        &snapshot,
        &p,
        Some("en"),
        &ExportPolicy::default(),
        &check,
    )
    .unwrap();
    let exported = publish::write_source_policy(
        &output.join("remote"),
        &snapshot,
        &p,
        Some("en"),
        &remote,
        &check,
    )
    .unwrap();
    assert_ne!(embedded["exportIdentity"], exported["exportIdentity"]);
    let all_files = |dir: &Path| {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>()
    };
    assert!(
        all_files(&output.join("embedded"))
            .iter()
            .any(|b| String::from_utf8_lossy(b).contains(marker))
    );
    let remote_files = all_files(&output.join("remote"));
    assert!(
        remote_files
            .iter()
            .all(|b| !String::from_utf8_lossy(b).contains(marker))
    );
    assert!(
        remote_files
            .iter()
            .any(|b| String::from_utf8_lossy(b).contains(authored))
    );
    let expected = [
        format!(
            "https://github.com/example/render-fixture/blob/{}/src/A%20file%23.java#L7-L12",
            "a".repeat(40)
        ),
        format!(
            "https://git.example.invalid:8443/gitlab/team/nested/render-fixture/-/blob/{}/src/%D0%9F%D1%80%D0%B8%D0%BC%D0%B5%D1%80.java#L3-9",
            "a".repeat(40)
        ),
    ];
    for url in &expected {
        assert!(
            remote_files
                .iter()
                .any(|b| String::from_utf8_lossy(b).contains(url)),
            "{url}"
        );
    }
    if std::env::var_os("CODECLEW_SOURCE_EXPORT_FIXTURE_OUTPUT").is_some() {
        fs::write(output.join("cases.json"),serde_json::to_vec_pretty(&json!({"cases":[{"id":"native-embedded","directory":output.join("embedded"),"mode":"embedded","sourceOnlyMarkers":[marker],"authoredProse":[authored]},{"id":"native-remote","directory":output.join("remote"),"mode":"remote","sourceOnlyMarkers":[marker],"authoredProse":[authored],"expectedUrls":expected}],"snapshot":snapshot,"repositoryBindings":remote.repositories,"scope":"Actual production native publisher over a synthetic retained compiler fixture; remote availability is not queried"})).unwrap()).unwrap();
        println!("Remote source fixture pair: {}", output.display());
    }
}

#[test]
fn remote_full_publication_exports_preserve_old_and_selected_bindings_in_both_formats() {
    use crate::documentation::source_export::{Args as ExportArgs, SourceMode};
    let (temporary, repo, mut check, _, remote) = source_export_fixture();
    let old_marker = "CLEW_SOURCE_ONLY_20261020_0123456789";
    let new_marker = "CLEW_SOURCE_ONLY_NEW_20261020_0123456789";
    let authored = "CLEW_AUTHORED_FULL_PUBLICATION_20261020_9876543210";
    // Global dependency identities must remain distinct across the two services.
    for (service, evidence) in &mut check.services {
        let ids = evidence
            .sources
            .keys()
            .map(|id| (id.clone(), format!("{service}-{id}")))
            .collect::<BTreeMap<_, _>>();
        evidence.sources = std::mem::take(&mut evidence.sources)
            .into_iter()
            .map(|(id, mut source)| {
                source.id = ids[&id].clone();
                (source.id.clone(), source)
            })
            .collect();
        for observation in evidence.observations.values_mut() {
            observation.source_ids = observation
                .source_ids
                .iter()
                .map(|id| ids[id].clone())
                .collect();
            replace_ids(&mut observation.normalized, &ids);
            observation.digest = crate::documentation::digest(&observation.normalized).unwrap();
        }
        evidence.observations = std::mem::take(&mut evidence.observations)
            .into_iter()
            .map(|(id, mut observation)| {
                observation.id = format!("{service}.{id}");
                (observation.id.clone(), observation)
            })
            .collect();
    }
    check.dependencies = check
        .services
        .values()
        .flat_map(|e| e.observations.clone())
        .collect();
    // Reproduce legacy URL construction, including SSH transport ports and
    // unescaped file names. Repository metadata, not targetRef, owns the origin.
    for (id, e) in &mut check.services {
        let repository = &check.source_inputs.as_ref().unwrap().inputs.services[id].repository;
        let base = if repository.starts_with("https://") {
            repository.trim_end_matches(".git")
        } else {
            repository
        };
        for source in e.sources.values_mut() {
            source.url = Some(format!(
                "{base}/blob/{}/{file}#L{}-L{}",
                source.revision,
                source.start_line,
                source.end_line,
                file = source.file
            ));
        }
    }
    check.refresh_digest().unwrap();
    let old_snapshot = check.save_snapshot(&repo).unwrap();
    let narratives = check.services.keys().map(|service| {
        serde_json::from_value(json!({
            "schema":"codeclew-documentation-narrative/1.3",
            "subject":format!("service:{service}"),
            "contextDigest":check.context_digest,
            "operations":crate::documentation::sections::REQUIRED.iter().map(|(id,title,_)| json!({
                "id":id,"title":title,"documentationLanguage":"en",
                "summary":{"id":"summary","text":authored,
                    "dependencyIds":[format!("{service}.endpoint-a")],"sourceIds":[format!("{service}-src-endpoint-a")]},
                "participants":[],"events":[]
            })).collect::<Vec<_>>(),
            "gaps":{}
        })).unwrap()
    }).collect();
    let published = crate::documentation::render::publish_from_snapshot(
        &repo,
        narratives,
        false,
        BTreeMap::new(),
        &old_snapshot,
    )
    .unwrap();
    let publication = published["bundle"].as_str().unwrap().to_owned();
    let original_bindings = fs::read(
        repo.path(&format!("docs/generated/{publication}/bindings.json"))
            .unwrap(),
    )
    .unwrap();
    fn replace_ids(value: &mut Value, ids: &BTreeMap<String, String>) {
        match value {
            Value::String(v) => {
                if let Some(new) = ids.get(v) {
                    *v = new.clone()
                }
            }
            Value::Object(o) => {
                for v in o.values_mut() {
                    replace_ids(v, ids)
                }
            }
            Value::Array(a) => {
                for v in a {
                    replace_ids(v, ids)
                }
            }
            _ => {}
        }
    }
    for e in check.services.values_mut() {
        e.revision = "b".repeat(40);
        let ids = e
            .sources
            .keys()
            .map(|id| (id.clone(), format!("selected-{id}")))
            .collect::<BTreeMap<_, _>>();
        let mut sources = BTreeMap::new();
        for (id, mut source) in std::mem::take(&mut e.sources) {
            source.id = ids[&id].clone();
            source.revision = e.revision.clone();
            source.text = source.text.replace(old_marker, new_marker);
            source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
            if let Some(o) = source.occurrence.as_mut() {
                o.blob = source.text_digest.clone();
                o.end_byte = o.start_byte + source.text.len();
            }
            source.url = source
                .url
                .map(|url| url.replace(&"a".repeat(40), &"b".repeat(40)));
            sources.insert(source.id.clone(), source);
        }
        e.sources = sources;
        for observation in e.observations.values_mut() {
            for source in &mut observation.source_ids {
                *source = ids[source].clone();
            }
            replace_ids(&mut observation.normalized, &ids);
            observation.digest = crate::documentation::digest(&observation.normalized).unwrap();
        }
    }
    check.dependencies = check
        .services
        .values()
        .flat_map(|e| e.observations.clone())
        .collect();
    check.refresh_digest().unwrap();
    let snapshot = check.save_snapshot(&repo).unwrap();
    let source = &check.services["sample"].sources["selected-sample-src-endpoint-a"];
    let reference = json!({"kind":"SOURCE","service":"sample","sourceId":source.id,"textDigest":source.text_digest,"startLine":source.start_line,"endLine":source.end_line});
    let block = json!({"title":"Authored reading guidance","text":authored,"evidence":[reference]});
    let guide = json!({"schema":"codeclew-system-reading-plan/1.0","publication":publication,"subject":"system:overview","language":"en","title":"Source links reference","purpose":authored,"sections":[{"id":"start","title":"Start","blocks":[block]},{"id":"confidence","title":"Source and scope","blocks":[block]}],"conclusion":block});
    let input = temporary.path().join("guide.json");
    fs::write(&input, serde_json::to_vec_pretty(&guide).unwrap()).unwrap();
    let mapping = temporary.path().join("source-map.json");
    fs::write(&mapping,serde_json::to_vec_pretty(&json!({"schema":"codeclew-source-repositories/1.0","repositories":remote.repositories})).unwrap()).unwrap();
    let output = std::env::var_os("CODECLEW_SOURCE_EXPORT_FULL_FIXTURE_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.path().join("full-exports"));
    fs::create_dir_all(&output).unwrap();
    let mut cases = Vec::new();
    let mut identities = BTreeSet::new();
    let formats = [
        ("standalone", PageFormat::Standalone),
        ("docusaurus", PageFormat::Docusaurus),
    ];
    let formats = if std::env::var_os("CODECLEW_SOURCE_EXPORT_STANDALONE_ONLY").is_some() {
        &formats[..1]
    } else {
        &formats[..]
    };
    for (format_name, format) in formats.iter().copied() {
        for (mode_name, mode) in [
            ("embedded", SourceMode::Embedded),
            ("remote", SourceMode::Remote),
        ] {
            let directory = output.join(format!("{format_name}-{mode_name}"));
            let result = run(Command::Render {
                root: repo.root.clone(),
                snapshot: snapshot.clone(),
                input: None,
                publication: Some(publication.clone()),
                reading_plan: Some(input.clone()),
                contract_comparison: None,
                data_model: None,
                format,
                language: Some("en".into()),
                source_export: ExportArgs {
                    source_mode: mode,
                    source_map: Some(mapping.clone()),
                },
                output: directory.clone(),
            })
            .unwrap();
            assert_eq!(result["sourceExport"]["mode"], mode_name);
            identities.insert(result["exportIdentity"].as_str().unwrap().to_owned());
            let mut files = Vec::new();
            fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
                for entry in fs::read_dir(dir).unwrap() {
                    let p = entry.unwrap().path();
                    if p.is_dir() {
                        collect(&p, files)
                    } else {
                        files.push(p)
                    }
                }
            }
            collect(&directory, &mut files);
            let bodies = files
                .iter()
                .map(|p| fs::read_to_string(p).unwrap_or_default())
                .collect::<Vec<_>>();
            for marker in [old_marker, new_marker] {
                assert_eq!(
                    bodies.iter().any(|text| text.contains(marker)),
                    mode == SourceMode::Embedded,
                    "{format_name}/{mode_name}: {marker}"
                );
            }
            assert!(bodies.iter().any(|text| text.contains(authored)));
            let expected = [
                format!(
                    "https://github.com/example/render-fixture/blob/{}/src/A%20file%23.java#L7-L12",
                    "a".repeat(40)
                ),
                format!(
                    "https://github.com/example/render-fixture/blob/{}/src/A%20file%23.java#L7-L12",
                    "b".repeat(40)
                ),
                format!(
                    "https://git.example.invalid:8443/gitlab/team/nested/render-fixture/-/blob/{}/src/%D0%9F%D1%80%D0%B8%D0%BC%D0%B5%D1%80.java#L3-9",
                    "a".repeat(40)
                ),
                format!(
                    "https://git.example.invalid:8443/gitlab/team/nested/render-fixture/-/blob/{}/src/%D0%9F%D1%80%D0%B8%D0%BC%D0%B5%D1%80.java#L3-9",
                    "b".repeat(40)
                ),
            ];
            if mode == SourceMode::Remote {
                for url in &expected {
                    assert!(
                        bodies.iter().any(|text| text.contains(url)),
                        "{format_name}: {url}"
                    );
                }
            }
            cases.push(json!({"id":format!("full-{format_name}-{mode_name}"),"directory":directory,"mode":mode_name,"sourceOnlyMarkers":[old_marker,new_marker],"authoredProse":[authored],"expectedUrls":if mode==SourceMode::Remote{expected.to_vec()}else{vec![]}}));
        }
    }
    let cli_output = output.join("cli-remote");
    let cli = crate::documentation::cli::run(crate::documentation::cli::Command::Render {
        root: repo.root.clone(),
        language: Some("en".into()),
        input: vec![],
        require_complete: false,
        refresh: false,
        snapshot: Some(snapshot.clone()),
        publish: false,
        source_export: ExportArgs {
            source_mode: SourceMode::Remote,
            source_map: Some(mapping.clone()),
        },
        output: Some(cli_output.clone()),
    })
    .unwrap();
    assert_eq!(cli["sourceExport"]["mode"], "remote");
    cases.push(json!({"id":"cli-remote","directory":cli_output,"mode":"remote","sourceOnlyMarkers":[old_marker,new_marker],"authoredProse":[authored]}));
    for (input, refresh, publish, require_complete) in [
        (vec![input.clone()], false, false, false),
        (vec![], true, false, false),
        (vec![], false, true, false),
        (vec![], false, false, true),
    ] {
        let forbidden = output.join("must-not-exist");
        assert!(
            crate::documentation::cli::run(crate::documentation::cli::Command::Render {
                root: repo.root.clone(),
                language: None,
                input,
                require_complete,
                refresh,
                snapshot: Some(snapshot.clone()),
                publish,
                source_export: ExportArgs {
                    source_mode: SourceMode::Remote,
                    source_map: Some(mapping.clone())
                },
                output: Some(forbidden.clone())
            })
            .is_err()
        );
        assert!(!forbidden.exists());
    }
    assert_eq!(
        identities.len(),
        formats.len() * 2,
        "format and mode have independent export identities"
    );
    assert_eq!(
        fs::read(
            repo.path(&format!("docs/generated/{publication}/bindings.json"))
                .unwrap()
        )
        .unwrap(),
        original_bindings
    );
    assert!(
        serde_json::to_string(&Check::load_snapshot(&repo, &old_snapshot).unwrap())
            .unwrap()
            .contains(old_marker)
    );
    if std::env::var_os("CODECLEW_SOURCE_EXPORT_FULL_FIXTURE_OUTPUT").is_some() {
        fs::write(output.join("cases.json"),serde_json::to_vec_pretty(&json!({"cases":cases,"retainedSnapshot":old_snapshot,"selectedSnapshot":snapshot,"publication":publication,"repositoryBindings":remote.repositories,"scope":"Actual full saved-publication rendering through pages command; synthetic retained compiler fixtures; no remote availability probe"})).unwrap()).unwrap();
        println!("Full publication source-mode fixture: {}", output.display());
    }
}
