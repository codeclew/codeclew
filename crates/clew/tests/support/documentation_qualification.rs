//! Explicit source-syntax qualification boundaries, separate from model quality.
use super::*;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Instant};

fn publish(f: &Fixture, inputs: &[std::path::PathBuf]) -> Value {
    let mut args = vec!["docs", "render"];
    for input in inputs {
        args.extend(["--input", input.to_str().unwrap()]);
    }
    f.ok(&args)
}
fn directory_bytes(root: &Path) -> u64 {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(!meta.file_type().is_symlink());
            if meta.is_dir() {
                directory_bytes(&path)
            } else {
                meta.len()
            }
        })
        .sum()
}
fn register(f: &Fixture, service: &Value) {
    let path = f.input("qualified-service.json", service);
    let before = f.ok(&["docs", "service", "list"]);
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        path.to_str().unwrap(),
        "--expected-input-digest",
        before["inputDigest"].as_str().unwrap(),
    ]);
}

pub fn forty_services() {
    let corpus = std::path::PathBuf::from(
        std::env::var("CODECLEW_DOCSYS_CORPUS").expect("qualification corpus path"),
    );
    let manifest = read(corpus.join("corpus.json"));
    let rows = manifest["services"].as_array().unwrap();
    assert_eq!(rows.len(), 40);
    let f = Fixture::new();
    let mut sources = BTreeMap::new();
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let source = f.service(id);
        for name in ["Orders.java", "application.properties", "openapi.json"] {
            fs::copy(corpus.join(id).join(name), source.join(name)).unwrap();
        }
        commit(&source);
        let mut service = read(f.docs.join(format!("catalog/services/{id}.json")));
        service["contractFiles"] = json!(["openapi.json"]);
        register(&f, &service);
        sources.insert(id.to_owned(), source);
    }
    let cold = Instant::now();
    let first = f.checked();
    let cold_ms = cold.elapsed().as_millis();
    assert_eq!(first.services.len(), 40);
    assert_eq!(
        first.services["orders"]
            .entrypoints
            .iter()
            .filter(|e| e.kind == "HTTP_ENDPOINT")
            .count(),
        40
    );
    let warm = Instant::now();
    let second = f.checked();
    let warm_ms = warm.elapsed().as_millis();
    assert_eq!(first.context_digest, second.context_digest);
    let mut captures = BTreeMap::new();
    let capture_start = Instant::now();
    for id in sources.keys() {
        let path = f.temp.path().join(format!("package-{id}"));
        let capture = f.ok(&[
            "docs",
            "evidence",
            "capture",
            "--service",
            id,
            "--output",
            path.to_str().unwrap(),
        ]);
        assert_eq!(capture["status"], "CAPTURED");
        assert_eq!(
            package_set_expected(&f, &package_expect(&f, &capture, 1)).0,
            0
        );
        f.ok(&[
            "docs",
            "evidence",
            "import",
            "--input",
            path.to_str().unwrap(),
        ]);
        captures.insert(id.clone(), capture);
    }
    let package_ms = capture_start.elapsed().as_millis();
    for id in ["orders", "service02", "service03", "service04"] {
        update_configure(&f, id);
    }
    view_entity(&f, "quantity");
    view_entity(&f, "independent");
    for (id, service, entity) in [
        ("quantity-view", "orders", "quantity"),
        ("quantity-output", "orders", "quantity"),
        ("independent-view", "other", "independent"),
    ] {
        assert_eq!(
            put_view(&f, &view_definition(id, service, entity), false).0,
            0
        );
    }
    save_process(
        &f,
        &process_definition("quantity-process", &["quantity-view", "quantity-output"]),
    );
    let (_, human) = note_fixture(&f);
    let mut inputs = Vec::new();
    for (id, service, entity) in [
        ("quantity-view", "orders", "quantity"),
        ("quantity-output", "orders", "quantity"),
        ("independent-view", "other", "independent"),
    ] {
        let (work, frozen) = view_work(&f, id);
        let proposal = view_proposal(&frozen, id, service, entity);
        read_view_evidence(&f, &work, &frozen, &proposal);
        let ready = proposal_submit(&f, &work, &proposal);
        assert!(
            ready["status"].as_str().unwrap().starts_with("READY_"),
            "{ready}"
        );
        inputs.push(f.input(
            &format!("{id}-narrative.json"),
            &proposal_artifact(&f, &ready)["narrative"],
        ));
    }
    let (note_work, note_frozen) = note_work(&f);
    let note_ready = proposal_submit(&f, &note_work, &note_proposal(&note_frozen, "CONTRADICTED"));
    assert!(
        note_ready["status"].as_str().unwrap().starts_with("READY_"),
        "{note_ready}"
    );
    let assessment = proposal_artifact(&f, &note_ready)["narrative"]["operations"][0].clone();
    let checked = f.checked();
    let mut roots = BTreeMap::new();
    for id in sources.keys() {
        let path = f.author(id, &checked);
        let mut n = read(&path);
        roots.insert(
            id.clone(),
            n["operations"][0]["id"].as_str().unwrap().to_owned(),
        );
        if id == "orders" {
            n["schema"] = json!("codeclew-documentation-narrative/1.3");
            n["operations"]
                .as_array_mut()
                .unwrap()
                .push(assessment.clone());
            let mapper = checked.services[id]
                .observations
                .values()
                .find(|o| o.kind == "FLOW" && o.symbol.contains("normalize"))
                .unwrap();
            n["operations"][0]["interfaceContracts"] = json!([{"id":"quantity-contract","title":"Returned quantity","kind":"payload","rows":[{"id":"mapped-output","label":"quantity","value":"Returns the normalized quantity.","dependencyIds":[mapper.id],"sourceIds":mapper.source_ids}],"boundaries":["Source-derived output; no wire compatibility claim."]}]);
        }
        inputs.push(f.input(&format!("{id}-complete.json"), &n));
    }
    let process = &checked.scenarios["quantity-process"];
    let events:Vec<_>=process.steps.iter().enumerate().map(|(i,s)|json!({"id":format!("process-step-{i}"),"kind":"note","text":"Source-bound quantity processing with unresolved calls retained.","dependencyIds":s.dependency_ids,"sourceIds":s.source_ids})).collect();
    let process_narrative = json!({"schema":"codeclew-documentation-narrative/1.0","subject":"scenario:quantity-process","contextDigest":checked.context_digest,"operations":[{"id":"quantity-process","title":"Quantity process","summary":{"id":"summary","text":"Source-bound quantity process; linked views preserve explicit uncertainty.","dependencyIds":process.dependency_ids,"sourceIds":process.steps.iter().flat_map(|s|s.source_ids.clone()).collect::<Vec<_>>()},"participants":[],"events":events,"boundaries":["Static source interpretation; no runtime delivery proof."]}],"gaps":{}});
    inputs.push(f.input("process-narrative.json", &process_narrative));
    let publication_start = Instant::now();
    let published = publish(&f, &inputs);
    let publication_ms = publication_start.elapsed().as_millis();
    let initial = published["bundle"].as_str().unwrap();
    let initial_binding: clew::documentation::bindings::Bindings =
        serde_json::from_value(read(f.bundle(initial, "bindings.json"))).unwrap();
    assert_eq!(initial_binding.revisions.len(), 40);
    for (service, root) in &roots {
        assert_eq!(
            initial_binding.section_states[&format!("service:{service}/{root}")].freshness,
            clew::documentation::model::Freshness::Current
        );
    }
    let original = fs::read(f.bundle(initial, "services/orders.html")).unwrap();
    let mut events = Vec::new();
    let mut latest_captures = Vec::new();
    for id in ["orders", "service02", "service03", "service04"] {
        let source = &sources[id];
        let path = source.join("Orders.java");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("return quantity;", "return quantity + 1;"),
        )
        .unwrap();
        commit(source);
        let next = f.temp.path().join(format!("next-{id}"));
        let captured = f.ok(&[
            "docs",
            "evidence",
            "capture",
            "--service",
            id,
            "--output",
            next.to_str().unwrap(),
        ]);
        events.push(update_event(&captured, &format!("{id}-next"), 2, false));
        latest_captures.push((next, captured));
    }
    let status_start = Instant::now();
    let result = update_enqueue(
        &f,
        &json!({"schema":"codeclew-documentation-revision-set/1.0","events":events}),
    );
    assert_eq!(result.0, 0, "{}", result.1);
    let status_ms = status_start.elapsed().as_millis();
    let current = f.ok(&["docs", "history", "list", "--limit", "1"])["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let binding: clew::documentation::bindings::Bindings =
        serde_json::from_value(read(f.bundle(&current, "bindings.json"))).unwrap();
    let mut affected_roots = Vec::new();
    let mut reused = 0;
    for (service, root) in &roots {
        let key = format!("service:{service}/{root}");
        let changed = ["orders", "service02", "service03", "service04"].contains(&service.as_str());
        if changed {
            affected_roots.push(key.clone());
            assert_ne!(
                binding.section_states[&key].freshness,
                clew::documentation::model::Freshness::Current,
                "{key}"
            );
        } else {
            assert_eq!(
                binding.section_states[&key].freshness,
                clew::documentation::model::Freshness::Current,
                "{key}"
            );
            reused += 1;
        }
    }
    for key in [
        "scenario:quantity-view/entity-dataflow",
        "scenario:quantity-output/entity-dataflow",
        "scenario:quantity-process/quantity-process",
        "service:orders/assessment-policy",
    ] {
        assert_ne!(
            binding.section_states[key].freshness,
            clew::documentation::model::Freshness::Current,
            "{key}"
        );
        affected_roots.push(key.into());
    }
    assert_eq!(
        binding.section_states["scenario:independent-view/entity-dataflow"].freshness,
        clew::documentation::model::Freshness::Current
    );
    for (path, capture) in latest_captures {
        assert_eq!(
            package_set_expected(&f, &package_expect(&f, &capture, 2)).0,
            0
        );
        f.ok(&[
            "docs",
            "evidence",
            "import",
            "--input",
            path.to_str().unwrap(),
        ]);
    }
    let artifact_bytes = directory_bytes(&f.docs.join("evidence/packages"));
    fs::remove_dir_all(f.docs.join(".codeclew")).unwrap();
    for source in sources.values() {
        fs::remove_dir_all(source).unwrap();
    }
    let recovered = f.checked();
    assert_eq!(recovered.services.len(), 40);
    assert!(recovered.unresolved.is_empty());
    let history = f.ok(&["docs", "history", "show", "--id", initial]);
    assert_eq!(history["status"], "FROZEN_SNAPSHOT");
    assert_eq!(history["evidenceRetention"], "COMPLETE");
    assert_eq!(fs::read(f.docs.join("notes/history.md")).unwrap(), human);
    assert_eq!(
        fs::read(f.bundle(initial, "services/orders.html")).unwrap(),
        original
    );
    let old = captures["orders"]["manifestDigest"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:");
    fs::remove_dir_all(f.docs.join("evidence/packages").join(old)).unwrap();
    let expired = f.ok(&["docs", "history", "show", "--id", initial]);
    assert_eq!(expired["evidenceRetention"], "MISSING_PACKAGES");
    let result = json!({"schema":"codeclew-documentation-forty-service-result/1.0","status":"PASSED","sourceRevisions":initial_binding.revisions,"sourceBytes":manifest["bytes"],"sourceFiles":manifest["files"],"services":40,"largestServiceEndpoints":40,"changedRepositories":4,"knownAffectedRootDenominator":affected_roots.len(),"falseCurrentRoots":0,"unaffectedOperationRootsReused":reused,"independentViewReused":true,"artifactBytes":artifact_bytes,"captureColdMs":cold_ms,"captureWarmMs":warm_ms,"portableCaptureImportMs":package_ms,"publicationMs":publication_ms,"statusOnlyMs":status_ms,"recovery":{"withoutPrivateCache":true,"withoutSourceCheckouts":true,"servicesRecovered":40,"originalSourceRetained":true,"humanBytesPreserved":true,"expiredEvidenceReported":true},"affectedRoots":affected_roots});
    fs::write(
        std::env::var("CODECLEW_DOCSYS_QUALIFICATION").expect("qualification report path"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}

pub fn mutations() {
    for name in [
        "literal",
        "docstring",
        "helper",
        "import",
        "config",
        "contract",
        "added-endpoint",
        "deleted-endpoint",
        "added-file",
        "deleted-file",
        "rename",
        "split",
        "merge",
        "callback",
        "provider-selection",
    ] {
        let f = Fixture::new();
        let source = f.service("orders");
        f.service("other");
        let original = fs::read_to_string(source.join("Orders.java")).unwrap();
        fs::write(source.join("helper.properties"), "factor=1\n").unwrap();
        fs::write(source.join("api.json"),r#"{"openapi":"3.0.3","info":{"title":"Orders","version":"1"},"paths":{"/reserve":{"get":{"responses":{"200":{"description":"Quantity"}}}}}}"#).unwrap();
        let mut service = read(f.docs.join("catalog/services/orders.json"));
        service["contractFiles"] = json!(["api.json"]);
        register(&f, &service);
        commit(&source);
        let checked = f.checked();
        let order = f.author("orders", &checked);
        let other = f.author("other", &checked);
        let root = read(&order)["operations"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let other_root = read(&other)["operations"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let baseline = publish(&f, &[order, other]);
        let initial = baseline["bundle"].as_str().unwrap();
        let before = fs::read(f.bundle(initial, "services/orders.html")).unwrap();
        let changed=match name {
            "literal"|"helper"=>original.replace("return quantity;","return quantity + 1;"),
            "docstring"=>format!("/** Qualification documentation change. */\n{original}"),
            "import"=>format!("import java.util.List;\n{original}"),
            "added-endpoint"=>original.replacen("public class Orders {","public class Orders { public int added(int quantity) { return quantity; }",1),
            "deleted-endpoint"=>original.replace("public int reserve(int quantity) { return normalize(quantity); }",""),
            "rename"=>original.replace("reserve(","renamed("),
            "split"=>original.replace("return normalize(quantity);","int normalized = normalize(quantity); return normalized;"),
            "merge"=>original.replace("return normalize(quantity);","return quantity;"),
            "callback"=>original.replace("return normalize(quantity);","java.util.function.IntUnaryOperator callback = value -> normalize(value); return callback.applyAsInt(quantity);"),
            _=>original.clone(),
        };
        fs::write(source.join("Orders.java"), changed).unwrap();
        match name {
            "config" => fs::write(source.join("helper.properties"), "factor=2\n").unwrap(),
            "contract" => {
                let path = source.join("api.json");
                fs::write(
                    &path,
                    fs::read_to_string(&path)
                        .unwrap()
                        .replace("Quantity", "Changed quantity"),
                )
                .unwrap();
            }
            "added-file" => fs::write(
                source.join("Additional.java"),
                "public class Additional { public int empty() { return 0; } }\n",
            )
            .unwrap(),
            "deleted-file" => fs::remove_file(source.join("helper.properties")).unwrap(),
            "provider-selection" => {
                service["modules"] = json!({"schema":"codeclew-documentation-modules/1.0","semantic":{"module":"javac","enabled":false}});
                register(&f, &service);
            }
            _ => {}
        }
        if name != "provider-selection" {
            commit(&source);
        }
        let refreshed = f.ok(&["docs", "refresh", "--status-only"]);
        let bundle = refreshed["bundle"].as_str().unwrap();
        let changed = read(f.bundle(bundle, "services/orders.json"));
        assert_ne!(
            changed["operationStates"][&root]["freshness"], "CURRENT",
            "{name}: {changed}"
        );
        let independent = read(f.bundle(bundle, "services/other.json"));
        assert_eq!(
            independent["operationStates"][&other_root]["freshness"], "CURRENT",
            "{name}: {independent}"
        );
        assert_eq!(
            fs::read(f.bundle(initial, "services/orders.html")).unwrap(),
            before
        );
    }
}
