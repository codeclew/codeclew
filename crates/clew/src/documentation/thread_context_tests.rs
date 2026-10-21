use super::{
    analysis, digest, model::*, store::Repository, syntax, thread_context::*, thread_inventory,
    work,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/documentation-thread-completeness")
}
fn files() -> BTreeMap<String, String> {
    let base = fixture_root().join("source");
    walkdir::WalkDir::new(base.join("src/main/java"))
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path()
                    .strip_prefix(&base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read_to_string(e.path()).unwrap(),
            )
        })
        .collect()
}
fn service(mode: &str) -> Service {
    serde_json::from_slice(&fs::read(fixture_root().join(format!("service-{mode}.json"))).unwrap())
        .unwrap()
}
fn syntax_fixture() -> ServiceEvidence {
    let service = service("syntax");
    let files = files();
    let mut evidence = None;
    for (path, text) in &files {
        let part = syntax::audit_java_file(&service, path, text, &"1".repeat(40)).unwrap();
        if let Some(e) = evidence.as_mut() {
            let e: &mut ServiceEvidence = e;
            e.observations.extend(part.observations);
            e.entrypoints.extend(part.entrypoints);
            e.sources.extend(part.sources);
            e.boundaries.extend(part.boundaries);
        } else {
            evidence = Some(part);
        }
    }
    let mut e = evidence.unwrap();
    e.runtime_mode = "COMMITTED_SOURCE_NO_BUILD".into();
    thread_inventory::attach_syntax_surfaces(&mut e).unwrap();
    syntax::observe(&mut e,"SOURCE_SCOPE","source-scope",json!({"roots":["src/main/java"],"inventory":files.keys().map(|p|(p.clone(),json!({"digest":crate::canonical::hash_bytes(files[p].as_bytes()),"coverage":"SYNTAX"}))).collect::<BTreeMap<_,_>>(),"boundaries":[]}),vec![]).unwrap();
    analysis::verify_evidence(&e).unwrap();
    e
}
fn method<'a>(e: &'a ServiceEvidence, owner: &str, name: &str) -> &'a Observation {
    let rows: Vec<_> = e
        .observations
        .values()
        .filter(|o| {
            o.kind == "SYMBOL"
                && o.normalized["name"] == name
                && o.normalized["ownerIdentity"]
                    .as_str()
                    .is_some_and(|s| s.ends_with(owner))
        })
        .collect();
    assert_eq!(rows.len(), 1, "{owner}.{name}: {rows:?}");
    rows[0]
}
fn context_for(e: &ServiceEvidence, owner: &str, name: &str) -> Vec<Value> {
    context(
        e,
        &method(e, owner, name).id,
        "snapshot",
        "input",
        &Default::default(),
    )
    .unwrap()
}
fn bodies(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .filter(|r| r["obligationKind"] == "BODY")
        .filter_map(|r| r["record"]["symbol"].as_str().map(str::to_owned))
        .collect()
}
fn assert_oracle_inventory(e: &ServiceEvidence) {
    let rows = inventory(e).unwrap();
    let entries: Vec<_> = rows
        .iter()
        .filter(|r| r["kind"] == "THREAD_ENTRY")
        .collect();
    assert_eq!(
        entries.len(),
        7,
        "expected exactly five direct roots and two supported/possible surfaces: {entries:#?}"
    );
    let oracle: Value =
        serde_json::from_slice(&fs::read(fixture_root().join("oracle.json")).unwrap()).unwrap();
    for expected in oracle["expectedRoots"].as_array().unwrap() {
        let file = expected["file"].as_str().unwrap();
        let name = expected["method"].as_str().unwrap();
        assert!(
            entries.iter().any(|r| r["record"]["sourceIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|id| e
                    .sources
                    .get(id)
                    .is_some_and(|s| s.file == file && s.text.contains(&format!("{name}("))))),
            "missing expected {file}:{name}: {entries:#?}"
        );
    }
    for entry in &entries {
        let sources = entry["record"]["sourceIds"].as_array().unwrap();
        assert!(
            !sources
                .iter()
                .filter_map(Value::as_str)
                .any(|id| e.sources[id].file.ends_with("UnannotatedFacade.java")
                    || e.sources[id].file.starts_with("other-scope/"))
        );
    }
    let mut cursor = None;
    let mut found = BTreeSet::new();
    loop {
        let page = super::cli::page(
            &digest(&rows).unwrap(),
            rows.clone(),
            cursor.as_deref(),
            2,
            json!({}),
        )
        .unwrap();
        assert!(page["omitted"].as_array().unwrap().is_empty());
        for row in page["items"].as_array().unwrap() {
            assert!(found.insert(row["id"].as_str().unwrap().to_owned()));
        }
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(found.len(), rows.len());
}
#[test]
fn syntax_inventory_matches_independent_oracle_and_paging() {
    assert_oracle_inventory(&syntax_fixture());
}
#[test]
fn syntax_navigation_reaches_effects_and_keeps_overloads_interfaces_and_deferred_work() {
    let e = syntax_fixture();
    let rows = context_for(&e, "OrderController", "save");
    let names = bodies(&rows).into_iter().collect::<Vec<_>>().join(" ");
    for name in [
        "OrderService/save/",
        "Validator/normalize/",
        "OrderRepository/write/",
    ] {
        assert!(names.contains(name), "missing {name}: {names}");
    }
    let serialized = serde_json::to_string(&rows).unwrap();
    for text in [
        "attempts++",
        "client.send(value)",
        "IllegalArgumentException",
        "StorageFailure",
        "saved",
        "failed",
    ] {
        assert!(serialized.contains(text), "missing {text}");
    }
    assert!(
        rows.iter()
            .filter(|r| r["obligationKind"] == "CANDIDATE")
            .all(
                |r| r["record"]["relationshipAuthority"] == "NAVIGATION_ONLY"
                    && r["record"]["targetStatus"] == "UNRESOLVED"
            )
    );
    let lookup = context_for(&e, "OrderController", "lookup");
    let decorate: Vec<_> = lookup
        .iter()
        .filter(|r| {
            r["obligationKind"] == "CANDIDATE"
                && r["record"]["candidateDeclaration"]
                    .as_str()
                    .is_some_and(|id| e.observations[id].normalized["name"] == "decorate")
        })
        .collect();
    assert_eq!(decorate.len(), 2);
    assert!(decorate.iter().all(|r| r["record"]["candidateCount"] == 2));
    let port = context_for(&e, "AdminController", "lookupPort");
    assert!(port.iter().any(|r| {
        r["record"]["candidateCount"]
            .as_u64()
            .is_some_and(|n| n >= 3)
    }));
    let asynchronous = context_for(&e, "OrderController", "submit");
    assert!(
        asynchronous
            .iter()
            .any(|r| r["record"]["event"]["kind"] == "DEFERRED")
    );
    assert!(asynchronous.iter().any(|r| {
        r["record"]["event"]["text"]
            .as_str()
            .is_some_and(|s| s.contains("accepted"))
    }));
    let original = &e.observations[&method(&e, "OrderService", "save").id];
    let events = original.normalized["documentation"]["events"]
        .as_array()
        .unwrap();
    let validation = events
        .iter()
        .position(|v| {
            v["text"]
                .as_str()
                .is_some_and(|s| s.starts_with("validator.normalize"))
        })
        .unwrap();
    let guarded = events.iter().position(|v| v["kind"] == "TRY").unwrap();
    assert!(validation < guarded);
}
#[test]
fn syntax_budget_and_missing_source_do_not_become_explained_by_reading() {
    let mut e = syntax_fixture();
    let root = method(&e, "OrderController", "save").id.clone();
    let budget = super::process_extract::Budgets {
        max_methods: 2,
        ..Default::default()
    };
    let rows = context(&e, &root, "snapshot", "input", &budget).unwrap();
    assert!(rows.iter().any(|r| r["obligationKind"] == "BUDGET"));
    let accounts: BTreeMap<_, _> = rows
        .iter()
        .map(|r| {
            let id = r["id"].as_str().unwrap().to_owned();
            (
                id.clone(),
                Account {
                    obligation: id,
                    explanation: "Read everything; no remaining behavior.".into(),
                    references: vec!["source".into()],
                },
            )
        })
        .collect();
    assert!(
        accounts_status(&rows, Some(&accounts))["open"]
            .as_u64()
            .unwrap()
            > 0
    );
    let id = method(&e, "OrderRepository", "write").id.clone();
    e.observations.get_mut(&id).unwrap().source_ids.clear();
    // Provider-level missing body is valid portable evidence, unlike deleting
    // a referenced private cache object.
    analysis::verify_evidence(&e).unwrap();
    let rows = context_for(&e, "OrderController", "save");
    assert!(
        rows.iter()
            .any(|r| r["obligationKind"] == "BODY" && r["record"]["actionable"] == true)
    );
    assert_eq!(accounts_status(&rows, None)["status"], "NOT_STARTED");
}

#[test]
#[ignore = "focused real JDK/Maven capture of the owned Java/Spring fixture"]
fn owned_fixture_real_compiler_and_source_capture() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("source");
    fs::create_dir(&repo).unwrap();
    for item in walkdir::WalkDir::new(fixture_root().join("source"))
        .into_iter()
        .map(Result::unwrap)
    {
        let relative = item
            .path()
            .strip_prefix(fixture_root().join("source"))
            .unwrap()
            .to_owned();
        let target = repo.join(relative);
        if item.file_type().is_dir() {
            fs::create_dir_all(target).unwrap();
        } else {
            fs::copy(item.path(), target).unwrap();
        }
    }
    for args in [
        &["init", "-q"][..],
        &["add", "."][..],
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "Owned fixture",
        ][..],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    }
    let revision = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    let cas = crate::cas::CasStore::open(
        &crate::state::StateAuthority::open(temp.path().join("state")).unwrap(),
    )
    .unwrap();
    let syntax = syntax::capture_with_store(&service("syntax"), &repo, &revision, &cas).unwrap();
    assert_oracle_inventory(&syntax);
    let model = crate::java_project_model::extract_java_model(&repo, ":/main").unwrap();
    let files: BTreeMap<_, _> = model
        .authority
        .source_files
        .iter()
        .map(|p| (p.clone(), fs::read_to_string(repo.join(p)).unwrap()))
        .collect();
    let digests = files
        .iter()
        .map(|(p, s)| (p.clone(), crate::canonical::hash_bytes(s.as_bytes())))
        .collect();
    let index = crate::java_adapter_v2::build_java_compiler_index(
        &repo,
        &model,
        &digests,
        false,
        None,
        &[],
        None,
    )
    .unwrap();
    let facts = index
        .facts
        .iter()
        .map(|f| {
            let mut v = serde_json::to_value(f).unwrap();
            let binding = crate::cas::CasObject::for_bytes(
                crate::java_adapter_v2::JAVA_FACT_SCHEMA,
                &super::bytes(&v).unwrap(),
            )
            .unwrap()
            .digest;
            v["scope"] = json!({"compilation":":/main"});
            (v, binding)
        })
        .collect();
    let compiler_service = service("compiler");
    let mut compiler = analysis::project(
        &compiler_service,
        &revision,
        &digest(&compiler_service).unwrap(),
        "PROJECT_NATIVE",
        "FULL",
        facts,
        &files,
        false,
    )
    .unwrap();
    let admitted_inputs = files
        .iter()
        .map(|(p, s)| (p.clone(), Arc::<str>::from(s.as_str())))
        .collect();
    let committed =
        super::source_origin::capture(&repo, &revision, &admitted_inputs, &cas).unwrap();
    thread_inventory::attach_java_inputs(
        &mut compiler,
        &compiler_service,
        ":/main",
        &files
            .iter()
            .map(|(p, s)| (p.clone(), Arc::<str>::from(s.as_str())))
            .collect(),
        &BTreeSet::from(["other-scope/java/example/contour/OrderService.java".into()]),
        "OWNED_JAVA_MODEL",
        Some(&committed),
    )
    .unwrap();
    analysis::verify_evidence(&compiler).unwrap();
    assert_oracle_inventory(&compiler);
    if let Ok(output) = std::env::var("CODECLEW_THREAD_FIXTURE_OUTPUT") {
        let output = Path::new(&output);
        fs::create_dir_all(output).unwrap();
        for (mode, e) in [("compiler", &compiler), ("syntax", &syntax)] {
            fs::write(
                output.join(format!("{mode}-evidence.json")),
                super::bytes(&e).unwrap(),
            )
            .unwrap();
            fs::write(
                output.join(format!("{mode}-inventory.json")),
                super::bytes(&inventory(e).unwrap()).unwrap(),
            )
            .unwrap();
            fs::write(
                output.join(format!("{mode}-save-thread.json")),
                super::bytes(&context_for(e, "OrderController", "save")).unwrap(),
            )
            .unwrap();
        }
    }
    let save = context_for(&compiler, "OrderController", "save");
    assert!(save.iter().any(
        |r| r["obligationKind"] == "DATA_EFFECT" && r["record"]["effect"]["name"] == "attempts"
    ));
    for (owner, name) in [("OrderController", "save"), ("AdminController", "status")] {
        let docs = tempfile::tempdir().unwrap();
        Repository::init(docs.path(), "Compiler source accounts").unwrap();
        let repo = Repository::open(docs.path()).unwrap();
        let w = persisted_work(&repo, compiler.clone());
        let root = method(&compiler, owner, name).id.clone();
        let rows = query(&w, "THREAD_CONTEXT", &root).unwrap();
        assert!(
            rows.iter().all(|r| r["record"]["actionable"] != true),
            "{owner}.{name}: {rows:#?}"
        );
        assert!(!serde_json::to_string(&rows).unwrap().contains("LAMBDA"));
        assert_complete_accounts(&repo, &w, &root, &rows);
    }
    let mut missing = compiler.clone();
    missing
        .observations
        .get_mut(&method(&compiler, "OrderRepository", "write").id)
        .unwrap()
        .source_ids
        .clear();
    let missing_rows = context_for(&missing, "OrderController", "save");
    assert!(
        missing_rows
            .iter()
            .any(|r| r["record"]["actionable"] == true)
    );
    assert!(
        accounts_status(&missing_rows, Some(&all_accounts(&missing_rows)))["open"]
            .as_u64()
            .unwrap()
            > 0
    );
    let mut unknown = compiler.clone();
    unknown
        .boundaries
        .push("UNCLASSIFIED_PROVIDER_FAILURE".into());
    let unknown_rows = context_for(&unknown, "AdminController", "status");
    assert!(
        accounts_status(&unknown_rows, Some(&all_accounts(&unknown_rows)))["open"]
            .as_u64()
            .unwrap()
            > 0
    );
    let lookup = context_for(&compiler, "OrderController", "lookup");
    let targets = bodies(&lookup).into_iter().collect::<Vec<_>>().join(" ");
    assert!(targets.contains("#decorate(Ljava/lang/String;)"));
    assert!(!targets.contains("#decorate(I)"));
    let port = context_for(&compiler, "AdminController", "lookupPort");
    let targets = bodies(&port).into_iter().collect::<Vec<_>>().join(" ");
    assert!(targets.contains("LookupPort#lookup"));
    assert!(!targets.contains("MemoryLookup#"));
    assert!(!targets.contains("RemoteLookup#"));
    let asynchronous = context_for(&compiler, "OrderController", "submit");
    assert!(
        asynchronous
            .iter()
            .any(|r| r["record"]["edge"]["relationshipAuthority"]
                == "JAVAC_SELECTED_DECLARATION_NOT_CONTROL_FLOW")
    );
    assert!(
        asynchronous
            .iter()
            .any(|r| r["record"]["event"]["kind"] == "BOUNDARY")
    );
    for e in [&compiler, &syntax] {
        let docs = tempfile::tempdir().unwrap();
        Repository::init(docs.path(), "Real capture Work").unwrap();
        let docs = Repository::open(docs.path()).unwrap();
        let w = persisted_work(&docs, e.clone());
        assert_full_source_exports(&docs, &w);

        let root = method(e, "OrderController", "save").id.clone();
        let limited = context(
            e,
            &root,
            "snapshot",
            "input",
            &super::process_extract::Budgets {
                max_methods: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(limited.iter().any(
            |r| r["obligationKind"] == "BUDGET" || r["record"]["frontier"]["kind"] == "BUDGET"
        ));
        assert!(
            accounts_status(&limited, Some(&all_accounts(&limited)))["open"]
                .as_u64()
                .unwrap()
                > 0
        );
        let mut cursor = None;
        let mut supplied = BTreeSet::new();
        loop {
            let page = work::read(
                &docs,
                &w.id,
                work::Selection {
                    query: Some(work::Query {
                        kind: "THREAD_CONTEXT".into(),
                        symbol_contains: root.clone(),
                        projection: Default::default(),
                    }),
                    cursor,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(page["omitted"].as_array().unwrap().is_empty());
            for row in page["items"].as_array().unwrap() {
                assert!(supplied.insert(row["id"].as_str().unwrap().to_owned()));
            }
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            supplied.len(),
            query(&w, "THREAD_CONTEXT", &root).unwrap().len()
        );
        let rows = context_for(e, "OrderController", "save");
        let methods = bodies(&rows).into_iter().collect::<Vec<_>>().join(" ");
        for name in ["OrderService", "Validator", "OrderRepository"] {
            assert!(methods.contains(name), "{name} not reached: {methods}");
        }
    }
}

fn persisted_work(repo: &Repository, e: ServiceEvidence) -> work::Work {
    let service = service(if e.extractor == SOURCE_EXTRACTOR {
        "syntax"
    } else {
        "compiler"
    });
    repo.service_add(service, Some(&repo.input_digest().unwrap()))
        .unwrap();
    let inputs = repo.inputs().unwrap();
    let input_digest = digest(&inputs).unwrap();
    let mut checked = super::check::assemble(
        input_digest.clone(),
        BTreeMap::from([("contour".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    checked.source_inputs = Some(super::check::SourceInputs {
        schema: super::check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest,
        inputs,
        selected_services: BTreeSet::from(["contour".into()]),
        retained_services: BTreeSet::new(),
    });
    checked.refresh_digest().unwrap();
    let snapshot = checked.save_snapshot(repo).unwrap();
    let request=serde_json::from_value(json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers","maxItems":2,"maxBytes":49152})).unwrap();
    let prepared =
        work::prepare_with_snapshot(repo, "service:contour".into(), request, Some(&snapshot))
            .unwrap();
    work::load(repo, prepared["work"].as_str().unwrap()).unwrap()
}
#[test]
fn work_resume_keeps_inventory_delivery_separate_from_source_bound_explanations() {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Thread Work fixture").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    let w = persisted_work(&repo, syntax_fixture());
    let root = method(&w.checked.services["contour"], "OrderController", "save")
        .id
        .clone();
    let mut cursor = None;
    let mut membership = BTreeSet::new();
    loop {
        let selection = work::Selection {
            query: Some(work::Query {
                kind: "THREAD_INVENTORY".into(),
                symbol_contains: String::new(),
                projection: Default::default(),
            }),
            cursor,
            ..Default::default()
        };
        let page = work::read(&repo, &w.id, selection).unwrap();
        for row in page["items"].as_array().unwrap() {
            assert!(membership.insert(row["id"].as_str().unwrap().to_owned()));
        }
        // Reopening a Work resumes the exact immutable inventory with the cursor.
        let restored = work::load(&Repository::open(temp.path()).unwrap(), &w.id).unwrap();
        assert_eq!(restored.snapshot, w.snapshot);
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        membership.len(),
        inventory(&w.checked.services["contour"]).unwrap().len()
    );
    let before = status(&repo, &w.id, None, 100).unwrap();
    assert!(
        before["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["progress"]["status"] == "NOT_STARTED")
    );
    let rows = query(&w, "THREAD_CONTEXT", &root).unwrap();
    let downstream = rows
        .iter()
        .find(|r| {
            r["obligationKind"] == "BODY"
                && r["record"]["symbol"]
                    .as_str()
                    .is_some_and(|s| s.contains("OrderRepository/write"))
        })
        .unwrap();
    let source =
        method(&w.checked.services["contour"], "OrderController", "save").source_ids[0].clone();
    let controller_reference = w
        .handles
        .iter()
        .find(|(_, h)| h.kind == "SOURCE" && h.id == source)
        .unwrap()
        .0
        .clone();
    work::read(
        &repo,
        &w.id,
        work::Selection {
            references: vec![controller_reference.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        assess(
            &repo,
            &w.id,
            Assessment {
                schema: "codeclew-thread-assessment/1.0".into(),
                root_declaration: root.clone(),
                accounts: vec![Account {
                    obligation: downstream["id"].as_str().unwrap().into(),
                    explanation: "Controller delegates to a repository and therefore saves.".into(),
                    references: vec![controller_reference.clone()]
                }]
            }
        )
        .is_err()
    );
    let body = rows
        .iter()
        .find(|r| r["obligationKind"] == "BODY" && r["record"]["declaration"] == root)
        .unwrap();
    assess(
        &repo,
        &w.id,
        Assessment {
            schema: "codeclew-thread-assessment/1.0".into(),
            root_declaration: root.clone(),
            accounts: vec![Account {
                obligation: body["id"].as_str().unwrap().into(),
                explanation:
                    "The controller delegates; downstream effects and failures remain to explain."
                        .into(),
                references: vec![controller_reference],
            }],
        },
    )
    .unwrap();
    let restored = status(&Repository::open(temp.path()).unwrap(), &w.id, None, 100).unwrap();
    let entry = restored["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["rootDeclaration"] == root)
        .unwrap();
    assert_eq!(entry["progress"]["status"], "IN_PROGRESS");
    assert!(!restored["fullScopeComplete"].as_bool().unwrap());
}

#[test]
fn partial_parse_and_legacy_missing_inventory_never_claim_full_scope() {
    let service = service("syntax");
    let broken =
        fs::read_to_string(fixture_root().join("faults/BrokenController.java.txt")).unwrap();
    let parsed = syntax::audit_java_file(
        &service,
        "src/main/java/BrokenController.java",
        &broken,
        &"1".repeat(40),
    )
    .unwrap();
    assert_eq!(parsed.coverage, "PARTIAL");
    assert!(
        parsed
            .boundaries
            .iter()
            .any(|b| b.starts_with("PARSE_ERROR:"))
    );
    for legacy in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Partial thread fixture").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let mut e = syntax_fixture();
        if legacy {
            e.observations.retain(|_, o| o.kind != "SOURCE_SCOPE");
        } else {
            e.coverage = "PARTIAL".into();
            e.boundaries
                .push("MISSING_SOURCE_ROOT:unavailable-module".into());
        }
        let w = persisted_work(&repo, e);
        let status = status(&repo, &w.id, None, 100).unwrap();
        assert_eq!(
            status["inventoryStatus"],
            if legacy { "UNKNOWN" } else { "PARTIAL" }
        );
        assert_eq!(status["boundedExplanationAccounted"], false);
        assert_eq!(status["fullScopeComplete"], false);
    }
}

#[test]
fn complete_local_accounts_can_finish_bounded_thread_without_claiming_meaning_or_runtime_review() {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Bounded account fixture").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    let w = persisted_work(&repo, syntax_fixture());
    let root = method(&w.checked.services["contour"], "AdminController", "status")
        .id
        .clone();
    let rows = query(&w, "THREAD_CONTEXT", &root).unwrap();
    let mut accounts = Vec::new();
    for row in &rows {
        let source_ids: BTreeSet<_> = row["record"]["sourceIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let references: Vec<_> = w
            .handles
            .iter()
            .filter(|(_, h)| h.kind == "SOURCE" && source_ids.contains(h.id.as_str()))
            .map(|(r, _)| r.clone())
            .collect();
        assert!(!references.is_empty());
        work::read(
            &repo,
            &w.id,
            work::Selection {
                references: references.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        accounts.push(Account{obligation:row["id"].as_str().unwrap().into(),explanation:"The local status handler returns the literal ready and has no downstream call in its retained body. Runtime registration is not established by this source account.".into(),references});
    }
    let state = work::read_state(&repo, &w.id).unwrap();
    assert!(!state.receipts.is_empty());
    assert_eq!(
        status(&repo, &w.id, None, 100).unwrap()["accountedCount"],
        0
    );
    let result = assess(
        &repo,
        &w.id,
        Assessment {
            schema: "codeclew-thread-assessment/1.0".into(),
            root_declaration: root.clone(),
            accounts,
        },
    )
    .unwrap();
    assert_eq!(result["progress"]["status"], "ACCOUNTED_WITH_BOUNDARIES");
    assert_eq!(result["progress"]["open"], 0);
    assert_eq!(result["progress"]["meaningReview"], "UNASSESSED");
    assert_eq!(result["progress"]["runtimeClosure"], "NOT_ESTABLISHED");
    let resumed = status(&repo, &w.id, None, 100).unwrap();
    assert_eq!(resumed["accountedCount"], 1);
    assert_eq!(resumed["boundedExplanationAccounted"], false);
}

#[test]
fn compilation_audit_does_not_invent_git_urls_for_transformed_inputs() {
    let mut e = syntax_fixture();
    let service = service("compiler");
    thread_inventory::attach_java_inputs(
        &mut e,
        &service,
        ":/main",
        &files()
            .into_iter()
            .map(|(p, s)| (p, Arc::<str>::from(s)))
            .collect(),
        &BTreeSet::new(),
        "owned-sealed-input-receipt",
        None,
    )
    .unwrap();
    let audit: Vec<_> = e
        .sources
        .values()
        .filter(|s| s.authority == "ADMITTED_COMPILATION_SOURCE_AUDIT_NOT_CALL_RESOLUTION")
        .collect();
    assert!(!audit.is_empty());
    assert!(
        audit
            .iter()
            .all(|s| s.url.is_none() && s.occurrence.is_none())
    );
    analysis::verify_evidence(&e).unwrap();
}

fn all_accounts(rows: &[Value]) -> BTreeMap<String, Account> {
    rows.iter()
        .map(|row| {
            let obligation = row["id"].as_str().unwrap().to_owned();
            (
                obligation.clone(),
                Account {
                    obligation,
                    explanation:
                        "Retained evidence remains bounded and does not establish runtime closure."
                            .into(),
                    references: vec![],
                },
            )
        })
        .collect()
}
fn assert_complete_accounts(repo: &Repository, w: &work::Work, root: &str, rows: &[Value]) {
    let mut accounts = Vec::new();
    for row in rows {
        let sources: BTreeSet<_> = row["record"]["sourceIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let references: Vec<_> = w
            .handles
            .iter()
            .filter(|(_, h)| h.kind == "SOURCE" && sources.contains(h.id.as_str()))
            .map(|(r, _)| r.clone())
            .collect();
        assert!(!references.is_empty(), "{row}");
        for reference in &references {
            let page = work::read(
                repo,
                &w.id,
                work::Selection {
                    references: vec![reference.clone()],
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(page["omitted"].as_array().unwrap().is_empty());
        }
        accounts.push(Account { obligation: row["id"].as_str().unwrap().into(), explanation: "Source-bound accounting records the retained behavior and provider limitations; runtime execution and semantic approval remain unknown.".into(), references });
    }
    let result = assess(
        repo,
        &w.id,
        Assessment {
            schema: "codeclew-thread-assessment/1.0".into(),
            root_declaration: root.into(),
            accounts,
        },
    )
    .unwrap();
    assert_eq!(result["progress"]["status"], "ACCOUNTED_WITH_BOUNDARIES");
    assert_eq!(result["progress"]["open"], 0);
    assert_eq!(result["progress"]["runtimeClosure"], "NOT_ESTABLISHED");
    assert_eq!(result["progress"]["meaningReview"], "UNASSESSED");
}
#[test]
fn small_thread_work_budget_exposes_same_snapshot_recovery_without_claiming_delivery() {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Small Work").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    let w = persisted_work(&repo, syntax_fixture());
    let mut request = w.request.clone();
    request.max_bytes = 2048;
    let small =
        work::prepare_with_snapshot(&repo, w.subject.clone(), request, w.snapshot.as_deref())
            .unwrap();
    let mut cursor = None;
    let mut omitted = 0;
    let mut recovery = None;
    loop {
        let page = work::read(
            &repo,
            small["work"].as_str().unwrap(),
            work::Selection {
                query: Some(work::Query {
                    kind: "THREAD_INVENTORY".into(),
                    symbol_contains: String::new(),
                    projection: Default::default(),
                }),
                cursor,
                ..Default::default()
            },
        )
        .unwrap();
        for row in page["omitted"].as_array().unwrap() {
            omitted += 1;
            assert_eq!(row["recovery"]["command"], "docs work prepare");
            assert_eq!(row["recovery"]["snapshot"], json!(w.snapshot));
            recovery = Some(row["recovery"]["input"].clone());
        }
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert!(omitted > 0);
    let recovered = work::prepare_with_snapshot(
        &repo,
        w.subject.clone(),
        serde_json::from_value(recovery.unwrap()).unwrap(),
        w.snapshot.as_deref(),
    )
    .unwrap();
    let mut count = 0;
    let mut cursor = None;
    loop {
        let page = work::read(
            &repo,
            recovered["work"].as_str().unwrap(),
            work::Selection {
                query: Some(work::Query {
                    kind: "THREAD_INVENTORY".into(),
                    symbol_contains: String::new(),
                    projection: Default::default(),
                }),
                cursor,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(page["omitted"].as_array().unwrap().is_empty());
        count += page["items"].as_array().unwrap().len();
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        count,
        inventory(&w.checked.services["contour"]).unwrap().len()
    );
}

// Exercise the complete frozen publication path, not just the source URL helper.
fn assert_full_source_exports(repo: &Repository, w: &work::Work) {
    use super::{source_export, static_pages};
    let snapshot = w.snapshot.as_deref().unwrap();
    let publication =
        super::render::publish_from_snapshot(repo, vec![], false, BTreeMap::new(), snapshot)
            .unwrap();
    let output = tempfile::tempdir().unwrap();
    let mapping = output.path().join("repositories.json");
    fs::write(&mapping, super::bytes(&json!({"schema":"codeclew-source-repositories/1.0","repositories":{
        "thread-completeness-fixture":{"provider":"github","webBase":"https://github.com/example/thread-completeness-fixture"}
    }})).unwrap()).unwrap();
    for (name, format) in [
        ("standalone", static_pages::PageFormat::Standalone),
        ("docusaurus", static_pages::PageFormat::Docusaurus),
    ] {
        for mode in [
            source_export::SourceMode::Embedded,
            source_export::SourceMode::Remote,
        ] {
            let result = static_pages::run(static_pages::Command::Render {
                source_export: source_export::Args {
                    source_mode: mode,
                    source_map: Some(mapping.clone()),
                },
                language: None,
                root: repo.root.clone(),
                snapshot: snapshot.into(),
                input: None,
                publication: Some(publication["bundle"].as_str().unwrap().into()),
                reading_plan: None,
                contract_comparison: None,
                data_model: None,
                format,
                output: output.path().join(format!("{name}-{mode:?}")),
            })
            .unwrap();
            assert_eq!(result["status"], "RENDERED");
        }
    }
}
