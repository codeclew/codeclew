//! Retained evidence, read admission and authored catalogue compatibility.
#![cfg(unix)]
#[path = "support/documentation_capture_export.rs"]
mod capture_export;
#[path = "support/documentation.rs"]
mod support;
use std::fs;
use support::{Fixture, commit, read};

fn retain_legacy_binding_fixture(
    f: &Fixture,
    checked: &clew::documentation::check::Check,
    input: &std::path::Path,
) -> String {
    f.retain_legacy(checked, input)
}

fn frozen_work(f: &Fixture, id: &str) -> serde_json::Value {
    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    serde_json::to_value(clew::documentation::work::load(&repo, id).unwrap()).unwrap()
}

fn package_capture(f: &Fixture, name: &str) -> (std::path::PathBuf, serde_json::Value) {
    let path = f.temp.path().join(name);
    let result = f.ok(&[
        "docs",
        "evidence",
        "capture",
        "--service",
        "orders",
        "--output",
        path.to_str().unwrap(),
    ]);
    (path, result)
}
fn package_service(to: &Fixture, from: &Fixture) {
    let record = read(from.docs.join("catalog/services/orders.json"));
    let input = to.input("service-import.json", &record);
    let before = to.ok(&["docs", "service", "list"]);
    to.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        before["inputDigest"].as_str().unwrap(),
    ]);
}
fn package_expect(_f: &Fixture, capture: &serde_json::Value, sequence: u64) -> serde_json::Value {
    serde_json::json!({"schema":"codeclew-documentation-evidence-expectation/1.0","service":capture["service"],"repositoryId":capture["service"],"serviceDigest":capture["serviceDigest"],"revision":capture["revision"],"manifestDigest":capture["manifestDigest"],"sequence":sequence})
}

fn package_set_expected(f: &Fixture, expectation: &serde_json::Value) -> (i32, serde_json::Value) {
    let input = f.input("expectation.json", expectation);
    let current = f.ok(&["docs", "service", "list"]);
    f.run(&[
        "docs",
        "evidence",
        "expect",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        current["inputDigest"].as_str().unwrap(),
    ])
}
fn package_import(f: &Fixture, path: &std::path::Path) -> (i32, serde_json::Value) {
    f.run(&[
        "docs",
        "evidence",
        "import",
        "--input",
        path.to_str().unwrap(),
    ])
}
fn package_copy(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir(to).unwrap();
    fs::create_dir(to.join("parts")).unwrap();
    fs::copy(from.join("manifest.json"), to.join("manifest.json")).unwrap();
    for part in fs::read_dir(from.join("parts")).unwrap() {
        let part = part.unwrap();
        fs::copy(part.path(), to.join("parts").join(part.file_name())).unwrap();
    }
}

#[test]
fn docsys_t12_offline_index_without_checkout_or_compiler() {
    let producer = Fixture::new();
    producer.service("orders");
    let (package, capture) = package_capture(&producer, "portable");
    assert_eq!(capture["status"], "CAPTURED");
    let consumer = Fixture::new();
    package_service(&consumer, &producer);
    fs::remove_file(consumer.temp.path().join("tools/git")).unwrap();
    assert_ne!(package_import(&consumer, &package).0, 0);
    assert_eq!(
        package_set_expected(&consumer, &package_expect(&consumer, &capture, 1)).0,
        0
    );
    assert_eq!(package_import(&consumer, &package).1["status"], "IMPORTED");
    assert_eq!(package_import(&consumer, &package).1["status"], "CURRENT");
    let inspect = consumer.run_unrooted(&[
        "docs",
        "evidence",
        "inspect",
        "--input",
        package.to_str().unwrap(),
    ]);
    assert_eq!(inspect.0, 0, "{inspect:?}");
    assert_eq!(inspect.1["status"], "INTEGRITY_CHECKED_NOT_ADMITTED");
    assert_eq!(inspect.1["report"]["projectJavaMinimum"], 17);
    assert_eq!(inspect.1["report"]["kotlinWorkerJava"], 21);
    let page = consumer.run_unrooted(&[
        "docs",
        "evidence",
        "read",
        "--input",
        package.to_str().unwrap(),
        "--kind",
        "sources",
        "--limit",
        "1",
    ]);
    assert_eq!(page.0, 0, "{page:?}");
    assert_eq!(page.1["items"].as_array().unwrap().len(), 1);
    assert_eq!(page.1["items"][0]["authority"], "EXACT_SNAPSHOT_TEXT");
    let checked = consumer.checked();
    assert!(checked.unresolved.is_empty(), "{:?}", checked.unresolved);
    assert_eq!(
        checked.services["orders"].revision,
        capture["revision"].as_str().unwrap()
    );
    assert!(
        checked.services["orders"]
            .boundaries
            .contains(&"PORTABLE_EVIDENCE_AT_SELECTED_REVISION".into())
    );
    assert!(
        !consumer
            .docs
            .join(".codeclew/bindings/orders.json")
            .exists()
    );
    let narrative = consumer.author("orders", &checked);
    retain_legacy_binding_fixture(&consumer, &checked, &narrative);
    assert_eq!(
        consumer.checked().services["orders"].revision,
        checked.services["orders"].revision
    );
}

#[test]
fn docsys_t12_rejects_corruption_forged_authority_paths_and_wrong_identity() {
    use serde_json::json;
    let f = Fixture::new();
    f.service("orders");
    let (package, capture) = package_capture(&f, "original");
    assert_eq!(
        package_set_expected(&f, &package_expect(&f, &capture, 1)).0,
        0
    );
    assert_eq!(package_import(&f, &package).0, 0);
    let selected = f.docs.join(".codeclew/evidence/selected/orders.json");
    let before = fs::read(&selected).unwrap();
    for case in [
        "missing",
        "corrupt",
        "path",
        "compressed",
        "schema",
        "service",
        "revision",
        "authority",
        "limit",
        "symlink",
    ] {
        let bad = f.temp.path().join(case);
        package_copy(&package, &bad);
        let mut m = read(bad.join("manifest.json"));
        let first = m["parts"][0]["path"].as_str().unwrap().to_owned();
        match case {
            "missing" => {
                fs::remove_file(bad.join(&first)).unwrap();
            }
            "corrupt" => {
                fs::write(bad.join(&first), b"{}").unwrap();
            }
            "path" => m["parts"][0]["path"] = json!("../manifest.json"),
            "compressed" => m["parts"][0]["encoding"] = json!("gzip"),
            "schema" => m["schema"] = json!("codeclew-documentation-evidence-package/999.0"),
            "service" => {
                m["service"]["id"] = json!("other");
                m["serviceDigest"] = json!(clew::canonical::hash(&m["service"]).unwrap());
            }
            "revision" => m["revision"] = json!("0".repeat(40)),
            "limit" => m["parts"][0]["bytes"] = json!(129 * 1024 * 1024),
            "symlink" => {
                fs::remove_file(bad.join(&first)).unwrap();
                std::os::unix::fs::symlink(package.join(&first), bad.join(&first)).unwrap();
            }
            "authority" => {
                let reference = m["parts"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|p| p["kind"] == "sources")
                    .unwrap();
                let old = reference["path"].as_str().unwrap().to_owned();
                let mut part = read(bad.join(&old));
                part["records"][0]["authority"] = json!("RUNTIME_VERIFIED");
                let data = serde_json::to_vec(&part).unwrap();
                let digest = clew::canonical::hash_bytes(&data);
                let path = format!("parts/{}.json", &digest[7..]);
                fs::write(bad.join(&path), &data).unwrap();
                reference["path"] = json!(path);
                reference["digest"] = json!(digest);
                reference["bytes"] = json!(data.len());
            }
            _ => unreachable!(),
        }
        fs::write(bad.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        assert_ne!(package_import(&f, &bad).0, 0, "accepted {case}");
        assert_eq!(
            fs::read(&selected).unwrap(),
            before,
            "{case} replaced valid selection"
        );
    }
    // A forger who changes an interpretation and rehashes every part still lacks coordinator trust.
    let bad = f.temp.path().join("self-rehashed");
    package_copy(&package, &bad);
    let mut m = read(bad.join("manifest.json"));
    m["producerVersion"] = json!("untrusted-producer");
    fs::write(bad.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    assert_ne!(package_import(&f, &bad).0, 0);
    assert_eq!(fs::read(selected).unwrap(), before);
}

#[test]
fn docsys_t12_new_expectation_failure_and_replay_preserve_previous_result() {
    let f = Fixture::new();
    let source = f.service("orders");
    let (old, old_capture) = package_capture(&f, "old");
    let first = package_expect(&f, &old_capture, 1);
    assert_eq!(package_set_expected(&f, &first).0, 0);
    assert_eq!(package_import(&f, &old).0, 0);
    let pointer = f.docs.join(".codeclew/evidence/selected/orders.json");
    let previous = fs::read(&pointer).unwrap();
    fs::write(
        source.join("Orders.java"),
        "public class Orders { public int reserve(int quantity) { return quantity + 1; } }\n",
    )
    .unwrap();
    commit(&source);
    let (new, new_capture) = package_capture(&f, "new");
    let second = package_expect(&f, &new_capture, 2);
    assert_eq!(package_set_expected(&f, &second).0, 0);
    assert_ne!(package_import(&f, &old).0, 0);
    assert_eq!(fs::read(&pointer).unwrap(), previous);
    assert!(f.checked().unresolved.contains_key("orders"));
    assert_eq!(package_import(&f, &new).0, 0);
    assert_ne!(package_set_expected(&f, &first).0, 0);
    assert_eq!(
        f.checked().services["orders"].revision,
        new_capture["revision"].as_str().unwrap()
    );
    let newest = fs::read(&pointer).unwrap();
    // A producer failure is a result, not guessed successful facts.
    fs::remove_file(f.temp.path().join("tools/git")).unwrap();
    let (_, missing) = package_capture(&f, "failure-no-git");
    assert_eq!(missing["status"], "PRODUCER_FAILURE");
    assert!(
        !missing
            .to_string()
            .contains(f.temp.path().to_str().unwrap())
    );
    assert_eq!(fs::read(&pointer).unwrap(), newest);
}

#[test]
fn docsys_t12_support_report_is_source_free_and_failed_capture_is_an_offline_gap() {
    use serde_json::json;
    let producer = Fixture::new();
    producer.service("orders");
    let output = producer.temp.path().join("support-report");
    let report = producer.ok(&[
        "docs",
        "evidence",
        "report",
        "--service",
        "orders",
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(report["status"], "DIAGNOSTICS_ONLY");
    assert_eq!(report["report"]["sourceIncluded"], false);
    let manifest = read(output.join("manifest.json"));
    assert!(
        manifest["parts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["kind"] == "report")
    );
    assert_ne!(package_import(&producer, &output).0, 0);
    for part in fs::read_dir(output.join("parts")).unwrap() {
        let text = fs::read_to_string(part.unwrap().path()).unwrap();
        assert!(!text.contains("return quantity"));
        assert!(!text.contains(producer.temp.path().to_str().unwrap()));
    }
    let mut service = read(producer.docs.join("catalog/services/orders.json"));
    service["language"] = json!("kotlin");
    service["profile"] = json!("kotlin-jvm-maven-analysis");
    service["compilations"] = json!([":/main"]);
    service.as_object_mut().unwrap().remove("source");
    let input = producer.input("compiler-service.json", &service);
    let current = producer.ok(&["docs", "service", "list"]);
    producer.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        current["inputDigest"].as_str().unwrap(),
    ]);
    let (failed, capture) = package_capture(&producer, "failed-compiler");
    assert_eq!(capture["status"], "PRODUCER_FAILURE");
    assert!(capture["revision"].is_string());
    let consumer = Fixture::new();
    package_service(&consumer, &producer);
    fs::remove_file(consumer.temp.path().join("tools/git")).unwrap();
    assert_eq!(
        package_set_expected(&consumer, &package_expect(&consumer, &capture, 1)).0,
        0
    );
    assert_eq!(
        package_import(&consumer, &failed).1["status"],
        "PRODUCER_FAILURE_RECORDED"
    );
    drop(producer);
    let checked = consumer.checked();
    assert!(!checked.services.contains_key("orders"));
    assert_eq!(
        checked.unresolved["orders"]["reason"],
        capture["report"]["errorCode"]
    );
    assert_eq!(
        checked.unresolved["orders"]["evidencePackage"]["manifestDigest"],
        capture["manifestDigest"]
    );
    assert_eq!(
        checked.unresolved["orders"]["evidencePackage"]["report"]["kotlinWorkerJava"],
        21
    );
}

fn work_request(f: &Fixture, bytes: usize, items: u32) -> std::path::PathBuf {
    f.input("work-request.json",&serde_json::json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Service maintainers","maxBytes":bytes,"maxItems":items}))
}

fn work_read(f: &Fixture, id: &str, selection: serde_json::Value) -> serde_json::Value {
    let path = f.input("selection.json", &selection);
    f.ok(&[
        "docs",
        "work",
        "expand",
        "--work",
        id,
        "--input",
        path.to_str().unwrap(),
    ])
}

#[test]
fn docsys_t02_freezes_evidence_and_tracks_negative_queries_and_notes() {
    use serde_json::json;
    let f = Fixture::new();
    let source = f.service("orders");
    fs::create_dir(f.docs.join("notes")).unwrap();
    fs::write(
        f.docs.join("notes/context.md"),
        "Human decision, retained exactly.\n",
    )
    .unwrap();
    let checked = f.checked();
    let baseline = f.author("orders", &checked);
    retain_legacy_binding_fixture(&f, &checked, &baseline);
    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let request = work_request(&f, 40 * 1024, 20);
    let page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let id = page["work"].as_str().unwrap();
    let original_work_bytes =
        fs::read(f.docs.join(format!(".codeclew/work/{id}/work.json"))).unwrap();
    let frozen = frozen_work(&f, id);
    assert!(
        frozen["retained"]["operations"]
            .as_array()
            .is_some_and(|ops| !ops.is_empty())
    );
    assert_eq!(
        frozen["externalInputs"]["notes/context.md"]["text"],
        "Human decision, retained exactly.\n"
    );
    assert!(
        frozen["influence"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| frozen["checked"]["dependencies"][k]["kind"] == "SOURCE_SCOPE")
    );
    let query = json!({"query":{"kind":"SYMBOL","symbolContains":"newlyAdded"}});
    let negative = work_read(&f, id, query.clone());
    assert_eq!(negative["total"], 0);
    fs::write(
        source.join("Added.java"),
        "public class Added { public int newlyAdded() { return 2; } }\n",
    )
    .unwrap();
    commit(&source);
    fs::write(f.docs.join("notes/context.md"), "Updated human decision.\n").unwrap();
    let still_negative = work_read(&f, id, query.clone());
    assert_eq!(negative, still_negative);
    let checked = f.checked();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let new = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let new_id = new["work"].as_str().unwrap();
    assert_ne!(new_id, id);
    let positive = work_read(&f, new_id, query);
    assert!(positive["total"].as_u64().unwrap() > 0);
    assert_ne!(positive["membershipDigest"], negative["membershipDigest"]);
    assert_eq!(frozen_work(&f, id), frozen);
    assert_eq!(
        fs::read(f.docs.join(format!(".codeclew/work/{id}/work.json"))).unwrap(),
        original_work_bytes
    );
    let ledger = read(f.docs.join(format!(".codeclew/work/{id}/reads.json")));
    assert!(ledger["receipts"].as_object().unwrap().values().any(
        |r| r["selection"]["query"]["symbolContains"] == "newlyAdded"
            && r["supplied"].as_array().unwrap().is_empty()
    ));
}

#[test]
fn docsys_t02_enforces_selection_cursors_and_untracked_read_limitations() {
    use serde_json::json;
    let f = Fixture::new();
    f.service("orders");
    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    let snapshot = f.checked().save_snapshot(&repo).unwrap();
    let request = work_request(&f, 40 * 1024, 1);
    let page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let id = page["work"].as_str().unwrap();
    assert!(page["nextCursor"].is_string());
    let path = f.input("forged.json", &json!({"references":["s999999"]}));
    assert_ne!(
        f.run(&[
            "docs",
            "work",
            "read",
            "--work",
            id,
            "--input",
            path.to_str().unwrap()
        ])
        .0,
        0
    );
    let path = f.input(
        "cursor.json",
        &json!({"query":{"kind":"SYMBOL"},"cursor":page["nextCursor"]}),
    );
    assert_ne!(
        f.run(&[
            "docs",
            "work",
            "read",
            "--work",
            id,
            "--input",
            path.to_str().unwrap()
        ])
        .0,
        0
    );
    let next = work_read(&f, id, json!({"cursor":page["nextCursor"]}));
    assert_ne!(next["items"], page["items"]);
    assert_eq!(
        work_read(&f, id, json!({"untrackedReads":true}))["influenceCoverage"],
        "INCOMPLETE_UNTRACKED_READS"
    );
    assert_eq!(
        work_read(&f, id, json!({}))["influenceCoverage"],
        "INCOMPLETE_UNTRACKED_READS"
    );
    let frozen = frozen_work(&f, id);
    let dependency = frozen["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, h)| h["kind"] == "DEPENDENCY")
        .unwrap()
        .0;
    let expanded = work_read(&f, id, json!({"references":[dependency]}));
    assert!(expanded["total"].as_u64().unwrap() > 0);
}

#[test]
fn docsys_t02_oversized_records_are_explicit_and_pages_stay_bounded() {
    use serde_json::json;
    let f = Fixture::new();
    f.service("orders");
    fs::create_dir(f.docs.join("notes")).unwrap();
    fs::write(
        f.docs.join("notes/long.md"),
        "long human input ".repeat(700),
    )
    .unwrap();
    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    let snapshot = f.checked().save_snapshot(&repo).unwrap();
    let request = work_request(&f, 2048, 100);
    let mut page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let id = page["work"].as_str().unwrap().to_owned();
    let mut omitted = Vec::new();
    for _ in 0..100 {
        assert!(serde_json::to_vec(&page).unwrap().len() < 2048, "{page}");
        omitted.extend(page["omitted"].as_array().unwrap().iter().cloned());
        let Some(cursor) = page["nextCursor"].as_str() else {
            break;
        };
        page = work_read(&f, &id, json!({"cursor":cursor}));
    }
    assert!(page["nextCursor"].is_null());
    assert!(
        omitted
            .iter()
            .any(|o| o["id"] == "notes/long.md" && o["reason"] == "ITEM_EXCEEDS_WORK_BYTE_BUDGET")
    );
    let ledger = read(f.docs.join(format!(".codeclew/work/{id}/reads.json")));
    assert!(
        ledger["receipts"]
            .as_object()
            .unwrap()
            .values()
            .any(|r| !r["omitted"].as_array().unwrap().is_empty())
    );
}

#[cfg(target_os = "macos")]
fn execution_config(
    f: &Fixture,
    author: serde_json::Value,
    reviewer: serde_json::Value,
    fallback: Option<serde_json::Value>,
) -> serde_json::Value {
    use serde_json::json;
    let output=std::process::Command::new("python3").args(["-I","-S","-c","import json,sys,pathlib; app=pathlib.Path(sys.base_prefix)/'Resources/Python.app/Contents/MacOS/Python'; print(json.dumps([str(app) if app.is_file() else sys.executable,sys.base_prefix,list(sys.version_info[:2])]))"]).output().unwrap();
    assert!(output.status.success());
    let python: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(python[2][0] == 3 && python[2][1].as_u64().unwrap() >= 11);
    let script = f.temp.path().join("isolated-driver.py");
    fs::write(
        &script,
        include_str!("../../../fixtures/documentation-system/agents/driver.py"),
    )
    .unwrap();
    let mut runtime_reads =
        std::collections::BTreeSet::from([python[1].as_str().unwrap().to_owned()]);
    runtime_reads.insert(script.to_string_lossy().into_owned());
    if std::path::Path::new("/opt/homebrew").is_dir() {
        runtime_reads.insert("/opt/homebrew".into());
    }
    let runtime_reads: Vec<_> = runtime_reads.into_iter().collect();
    let role = |options: serde_json::Value| json!({"adapter":"macos-seatbelt-stdio/1.0","model":"deterministic-fixture","usageAuthority":"TRANSPORT_METADATA","command":[python[0],"-I","-S",script,options.to_string()],"runtimeReads":runtime_reads.clone(),"cap":{"maximum":{"inputTokens":500000,"outputTokens":20000,"costUnits":10},"overheadInputTokens":10,"timeoutMs":5000,"outputBytes":65536}});
    json!({"schema":"codeclew-documentation-execution/1.0","author":role(author),"reviewer":role(reviewer),"fallback":fallback.clone().map(role),"authorCalls":2,"reviewerCalls":if fallback.is_some(){3}else{2},"fallbackCalls":if fallback.is_some(){1}else{0},"repairAttempts":1,"expansions":0,"budget":{"account":"fixture","costUnit":"fixture-unit","ceiling":{"inputTokens":10000000,"outputTokens":1000000,"costUnits":1000},"stopLoss":{"inputTokens":9999999,"outputTokens":999999,"costUnits":999}}})
}

#[test]
#[cfg(target_os = "macos")]
fn docsys_t04_competing_reservations_cannot_overdraw_one_account() {
    use clew::documentation::{agent_jobs, store::Repository};
    use serde_json::json;
    let f = Fixture::new();
    let mut config = execution_config(&f, json!({}), json!({}), None);
    config["budget"]["stopLoss"]["costUnits"] = json!(60);
    let config: agent_jobs::Config = serde_json::from_value(config).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|run| {
                let config = &config;
                let root = &f.docs;
                let barrier = &barrier;
                scope.spawn(move || {
                    let repo = Repository::open(root).unwrap();
                    barrier.wait();
                    agent_jobs::reserve(&repo, config, run)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let ledger = agent_jobs::account(&Repository::open(&f.docs).unwrap(), &config.budget).unwrap();
    assert_eq!(ledger.reservations.len(), 4);
    assert_eq!(
        ledger
            .reservations
            .values()
            .map(|r| r.charged.cost_units)
            .sum::<u64>(),
        40
    );
}

#[test]
fn docsys_t05_lists_real_producer_capabilities_without_build_tools() {
    let f = Fixture::new();
    f.service("orders");
    let listed = f.ok(&["docs", "modules", "list", "--service", "orders"]);
    let rows = listed["records"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .filter_map(|r| r["id"].as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        rows.len()
    );
    let openapi = rows.iter().find(|r| r["id"] == "openapi").unwrap();
    assert_eq!(openapi["configured"], false);
    assert_eq!(openapi["authority"], "DECLARED_OPENAPI");
    let source = rows.iter().find(|r| r["id"] == "source-syntax").unwrap();
    assert_eq!(source["configured"], true);
    assert_eq!(source["authority"], "SYNTAX_ONLY");
    let javac = f.ok(&[
        "docs",
        "modules",
        "show",
        "--id",
        "javac",
        "--service",
        "orders",
    ]);
    assert_eq!(javac["record"]["configured"], false);
    assert_eq!(javac["record"]["producer"]["id"], "java17");
    assert_eq!(javac["record"]["projectJavaMinimum"], 17);
    let kotlin = f.ok(&[
        "docs",
        "modules",
        "show",
        "--id",
        "kotlin-k2",
        "--service",
        "orders",
    ]);
    assert_eq!(kotlin["record"]["applicable"], false);
    assert_eq!(kotlin["record"]["availability"], "WORKER_NOT_INSTALLED");
    assert_eq!(kotlin["record"]["workerJavaMajor"], 21);
    assert!(!f.checked().services["orders"].entrypoints.is_empty());
}

#[test]
fn docsys_t05_optional_provider_loss_and_disable_keep_source_roots_readable() {
    use serde_json::json;
    let f = Fixture::new();
    f.service("orders");
    let first = f.checked();
    let old_scope = first.services["orders"]
        .observations
        .values()
        .find(|o| o.kind == "SOURCE_SCOPE")
        .unwrap();
    let mut record = read(f.docs.join("catalog/services/orders.json"));
    record["modules"] = json!({"schema":"codeclew-documentation-modules/1.0","semantic":{"module":"javac","enabled":true,"profile":"java-17plus-maven-read-only","compilation":":/main"}});
    let input = f.input("module-service.json", &record);
    let update = |input: &std::path::Path| {
        let current = f.ok(&["docs", "service", "list"])["inputDigest"]
            .as_str()
            .unwrap()
            .to_owned();
        f.ok(&[
            "docs",
            "service",
            "add",
            "--input",
            input.to_str().unwrap(),
            "--expected-input-digest",
            &current,
        ]);
    };
    update(&input);
    let missing = f.checked();
    let missing = &missing.services["orders"];
    assert_eq!(first.services["orders"].revision, missing.revision);
    assert_eq!(first.services["orders"].entrypoints, missing.entrypoints);
    let scope = missing
        .observations
        .values()
        .find(|o| o.kind == "SOURCE_SCOPE")
        .unwrap();
    assert_ne!(old_scope.digest, scope.digest);
    assert_eq!(
        scope.normalized["semantic"]["provider"]["status"],
        "UNAVAILABLE"
    );
    assert!(
        scope.normalized["modules"]["modules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "javac")
    );
    record["modules"]["semantic"] = json!({"module":"javac","enabled":false});
    let input = f.input("module-service.json", &record);
    update(&input);
    let disabled = f.checked();
    let disabled = &disabled.services["orders"];
    assert_eq!(first.services["orders"].entrypoints, disabled.entrypoints);
    let scope = disabled
        .observations
        .values()
        .find(|o| o.kind == "SOURCE_SCOPE")
        .unwrap();
    assert!(scope.normalized["semantic"].is_null());
    assert!(
        !disabled
            .boundaries
            .iter()
            .any(|b| b == "SEMANTIC_PROVIDER_UNAVAILABLE_SOURCE_REMAINS_READABLE")
    );
}

#[test]
fn docsys_t05_rejects_wrong_language_ambiguous_and_executable_module_configuration() {
    use serde_json::json;
    let f = Fixture::new();
    f.service("orders");
    let original = read(f.docs.join("catalog/services/orders.json"));
    let current = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    for module in [
        json!({"module":"kotlin-k2","enabled":true,"profile":"kotlin-jvm-maven-analysis","compilation":":/main"}),
        json!({"module":"javac","enabled":false,"profile":"java-17plus-maven-read-only"}),
        json!({"module":"javac","enabled":true,"command":["/bin/sh"]}),
    ] {
        let mut record = original.clone();
        record["modules"] =
            json!({"schema":"codeclew-documentation-modules/1.0","semantic":module});
        let input = f.input("invalid-module.json", &record);
        assert_ne!(
            f.run(&[
                "docs",
                "service",
                "add",
                "--input",
                input.to_str().unwrap(),
                "--expected-input-digest",
                &current
            ])
            .0,
            0
        );
    }
    let mut ambiguous = original;
    ambiguous["source"]["semantic"] =
        json!({"profile":"java-17plus-maven-read-only","compilation":":/main"});
    ambiguous["modules"] = json!({"schema":"codeclew-documentation-modules/1.0"});
    let input = f.input("ambiguous-module.json", &ambiguous);
    let (code, error) = f.run(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        &current,
    ]);
    assert_ne!(code, 0);
    assert_eq!(error["error"]["details"]["reason"], "UNKNOWN_FIELD");
    assert!(
        error["error"]["details"]["field"]
            .as_str()
            .unwrap()
            .ends_with("semantic")
    );
}

fn spring_source_fixture(
    language: &str,
    text: &str,
) -> (Fixture, clew::documentation::check::Check) {
    use serde_json::json;
    let f = Fixture::new();
    let repo = f.service("orders");
    fs::remove_file(repo.join("Orders.java")).unwrap();
    fs::write(
        repo.join(if language == "java" {
            "Orders.java"
        } else {
            "Orders.kt"
        }),
        text,
    )
    .unwrap();
    commit(&repo);
    let mut record = read(f.docs.join("catalog/services/orders.json"));
    record["language"] = json!(language);
    record["source"]["dialect"] = json!(if language == "java" { "17" } else { "1.9" });
    let input = f.input("spring-service.json", &record);
    let current = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        &current,
    ]);
    let checked = f.checked();
    (f, checked)
}
#[test]
fn docsys_t06_shared_rules_derive_equivalent_java_and_kotlin_source_endpoints() {
    for (language, text) in [
        (
            "java",
            include_str!("../../../fixtures/documentation-system/spring/Orders.java"),
        ),
        (
            "kotlin",
            include_str!("../../../fixtures/documentation-system/spring/Orders.kt"),
        ),
    ] {
        let (_f, checked) = spring_source_fixture(language, text);
        let service = &checked.services["orders"];
        let endpoint = service
            .entrypoints
            .iter()
            .find(|e| e.kind == "HTTP_ENDPOINT")
            .unwrap_or_else(|| panic!("{language}: {service:?}"));
        assert_eq!(endpoint.trigger["methods"], serde_json::json!(["POST"]));
        assert_eq!(
            endpoint.trigger["paths"],
            serde_json::json!(["/orders/reserve"])
        );
        assert_eq!(endpoint.trigger["authority"], "FRAMEWORK_DERIVED_SOURCE");
        assert_eq!(
            endpoint.trigger["frameworkDerivation"]["inputSchema"],
            "source-annotation-facts/1.0"
        );
        assert!(
            endpoint
                .boundaries
                .iter()
                .any(|b| b == "SOURCE_NAMES_NOT_COMPILER_RESOLVED")
        );
        assert_eq!(service.runtime_mode, "COMMITTED_SOURCE_NO_BUILD");
    }
}
#[test]
fn docsys_t06_unrelated_or_ambiguous_annotations_never_become_spring_routes() {
    let java = include_str!("../../../fixtures/documentation-system/spring/Orders.java");
    for text in [java.replace("import org.springframework.web.bind.annotation.PostMapping;","import other.PostMapping;"),java.replace("import org.springframework.web.bind.annotation.PostMapping;","@interface PostMapping { String path(); }"),java.replace("import org.springframework.web.bind.annotation.PostMapping;","import org.springframework.web.bind.annotation.PostMapping;\nimport other.PostMapping;")] {
        let (_,checked)=spring_source_fixture("java",&text);assert!(!checked.services["orders"].entrypoints.iter().any(|e|e.kind=="HTTP_ENDPOINT"));
    }
}
#[test]
fn wildcard_spring_names_remain_source_candidates_without_compiler_authority() {
    let java = include_str!("../../../fixtures/documentation-system/spring/Orders.java").replace(
        "import org.springframework.web.bind.annotation.PostMapping;",
        "import org.springframework.web.bind.annotation.*;",
    );
    let (_fixture, checked) = spring_source_fixture("java", &java);
    let evidence = &checked.services["orders"];
    let endpoint = evidence
        .entrypoints
        .iter()
        .find(|e| e.kind == "HTTP_ENDPOINT")
        .unwrap();
    assert_eq!(endpoint.trigger["authority"], "FRAMEWORK_DERIVED_SOURCE");
    assert!(
        endpoint
            .boundaries
            .iter()
            .any(|b| b == "SOURCE_WILDCARD_ANNOTATION_CANDIDATE")
    );
    assert!(
        endpoint
            .boundaries
            .iter()
            .any(|b| b == "SOURCE_NAMES_NOT_COMPILER_RESOLVED")
    );
    assert_eq!(
        evidence.extractor,
        clew::documentation::model::SOURCE_EXTRACTOR
    );
}

#[test]
fn docsys_t06_dynamic_values_alias_conflicts_and_inheritance_keep_named_gaps() {
    let java = include_str!("../../../fixtures/documentation-system/spring/Orders.java");
    for (text, gap) in [
        (
            java.replace("path = \"/reserve\"", "path = ROUTE"),
            "UNRESOLVED_ANNOTATION_VALUE",
        ),
        (
            java.replace(
                "path = \"/reserve\"",
                "value = \"/other\", path = \"/reserve\"",
            ),
            "CONFLICTING_PATH_ALIASES",
        ),
        (
            java.replace(
                "public class Orders {",
                "public class Orders extends BaseOrders {",
            ),
            "SOURCE_INHERITANCE_UNRESOLVED",
        ),
        (
            java.replace("path = \"/reserve\"", "path = \"${route}\""),
            "RUNTIME_EXPRESSION",
        ),
    ] {
        let (_, checked) = spring_source_fixture("java", &text);
        assert!(
            checked.services["orders"]
                .boundaries
                .iter()
                .any(|b| b == gap),
            "{gap}: {checked:?}"
        );
    }
}

fn openapi_fixture() -> (Fixture, std::path::PathBuf) {
    use serde_json::json;
    let f = Fixture::new();
    let repo = f.service("orders");
    fs::create_dir(repo.join("api")).unwrap();
    fs::write(
        repo.join("api/api.yaml"),
        include_str!("../../../fixtures/documentation-system/openapi/api.yaml"),
    )
    .unwrap();
    fs::write(
        repo.join("api/types.yaml"),
        include_str!("../../../fixtures/documentation-system/openapi/types.yaml"),
    )
    .unwrap();
    commit(&repo);
    let mut record = read(f.docs.join("catalog/services/orders.json"));
    record["source"]["roots"] = json!(["Orders.java"]);
    record["contractFiles"] = json!(["api/api.yaml", "api/types.yaml"]);
    let input = f.input("contract-service.json", &record);
    let current = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        &current,
    ]);
    (f, repo)
}

#[test]
fn docsys_t07_reference_gaps_and_actual_parse_or_shape_failures_remain_visible() {
    let (f, repo) = openapi_fixture();
    let doc = serde_json::json!({"openapi":"3.0.0","paths":{"/unknown":{"get":{"responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"object","properties":{"cycle":{"$ref":"#/components/schemas/Loop"},"missing":{"$ref":"#/components/schemas/Absent"},"external":{"$ref":"http://127.0.0.1:1/private"},"file":{"$ref":"unregistered.yaml#/Secret"},"escape":{"$ref":"../../outside.yaml"}}}}}}}}}},"components":{"schemas":{"Loop":{"$ref":"#/components/schemas/Loop"}}}});
    fs::write(repo.join("api/api.yaml"), serde_json::to_vec(&doc).unwrap()).unwrap();
    fs::write(
        repo.join("api/unregistered.yaml"),
        "Secret: {private: do-not-import}\n",
    )
    .unwrap();
    commit(&repo);
    let checked = f.checked();
    let e = checked
        .services
        .get("orders")
        .unwrap_or_else(|| panic!("{checked:?}"));
    let n = &e
        .observations
        .values()
        .find(|o| o.kind == "CONTRACT_OPERATION")
        .unwrap()
        .normalized;
    let gaps = n["boundaries"].to_string();
    for gap in [
        "CYCLIC_CONTRACT_REFERENCE",
        "MISSING_CONTRACT_REFERENCE",
        "EXTERNAL_CONTRACT_REFERENCE",
        "UNREGISTERED_OR_UNAVAILABLE_CONTRACT_REFERENCE",
        "UNSAFE_CONTRACT_REFERENCE",
    ] {
        assert!(gaps.contains(gap), "{gaps}");
    }
    assert!(!serde_json::to_string(e).unwrap().contains("do-not-import"));
    for content in ["openapi: 3.2.0\npaths: []\n", "openapi: [broken\n"] {
        fs::write(repo.join("api/api.yaml"), content).unwrap();
        commit(&repo);
        let checked = f.checked();
        let e = checked
            .services
            .get("orders")
            .unwrap_or_else(|| panic!("{checked:?}"));
        assert!(
            e.observations
                .values()
                .all(|o| o.kind != "CONTRACT_OPERATION")
        );
        assert!(
            e.boundaries
                .iter()
                .any(|g| g.starts_with(if content.contains("3.2.0") {
                    "UNREADABLE_CONTRACT_PATHS"
                } else {
                    "INVALID_CONTRACT_JSON_YAML"
                })),
            "{:?}",
            e.boundaries
        );
    }
    fs::remove_file(repo.join("api/types.yaml")).unwrap();
    commit(&repo);
    assert!(
        f.checked().services["orders"]
            .boundaries
            .iter()
            .any(|g| g == "CONTRACT_SOURCE_UNAVAILABLE:api/types.yaml")
    );
}

#[test]
fn protected_page_binding_tracks_exact_source_identity_and_explicit_rebind() {
    use serde_json::json;
    let f = Fixture::new();
    let source_repo = f.service("orders");
    let checked = f.checked();
    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let rows = f.ok(&["docs", "note", "list"]);
    let select = |checked: &clew::documentation::check::Check, name: &str, snapshot: &str| {
        let evidence = &checked.services["orders"];
        let observation = evidence
            .observations
            .values()
            .find(|o| o.kind == "SYMBOL" && o.symbol.contains(name) && !o.source_ids.is_empty())
            .unwrap();
        let source = &evidence.sources[&observation.source_ids[0]];
        json!({"schema":"codeclew-note-source-binding/1.0","id":"entry","rootNamespace":rows["rootNamespace"],"repositoryId":"orders","service":"orders","provider":evidence.extractor,"profile":"source-syntax","compilations":[],"snapshot":snapshot,"revision":source.revision,"sourceId":source.id,"observationId":observation.id,"startLine":source.start_line,"endLine":source.end_line,"textDigest":source.text_digest})
    };
    let binding = select(&checked, "reserve", &snapshot);
    let original=b"---\r\ntitle: Fixture page\r\n---\r\n\r\nField | Meaning\r\n--- | ---\r\nquantity | Requested quantity\r\n";
    let source = f.temp.path().join("page.md");
    fs::write(&source, original).unwrap();
    let mut association = json!({"schema":"codeclew-documentation-note-association/1.0","id":"page","title":"Fixture page","service":"orders","path":"notes/page.md","targets":["service:orders"],"classification":"mixed","period":"Fixture only","sourceBindings":[binding]});
    let input = f.input("page-association.json", &association);
    f.ok(&[
        "docs",
        "note",
        "import",
        "--source",
        source.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        rows["inputDigest"].as_str().unwrap(),
    ]);
    let original_code = fs::read_to_string(source_repo.join("Orders.java")).unwrap();
    fs::write(
        source_repo.join("Orders.java"),
        format!("\n\n{original_code}"),
    )
    .unwrap();
    commit(&source_repo);
    let moved = f.checked();
    let target = moved.save_snapshot(&repo).unwrap();
    let resolved = f.ok(&[
        "docs",
        "note",
        "bindings",
        "--id",
        "page",
        "--snapshot",
        &target,
    ]);
    assert_eq!(resolved["bindings"][0]["state"], "RELOCATED_ONLY");
    assert_eq!(
        resolved["bindings"][0]["selection"],
        association["sourceBindings"][0]
    );
    fs::write(
        source_repo.join("Orders.java"),
        original_code.replace("reserve(", "renamed("),
    )
    .unwrap();
    commit(&source_repo);
    let renamed = f.checked();
    let target = renamed.save_snapshot(&repo).unwrap();
    let missing = f.ok(&[
        "docs",
        "note",
        "bindings",
        "--id",
        "page",
        "--snapshot",
        &target,
    ]);
    assert_eq!(missing["bindings"][0]["state"], "MISSING");
    let old_digest = f.ok(&["docs", "note", "list"])["items"][0]["associationDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    association["sourceBindings"] = json!([select(&renamed, "renamed", &target)]);
    let input = f.input("rebound.json", &association);
    let current = f.ok(&["docs", "note", "list"]);
    f.ok(&[
        "docs",
        "note",
        "associate",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        current["inputDigest"].as_str().unwrap(),
        "--expected-note-digest",
        current["items"][0]["original"]["digest"].as_str().unwrap(),
    ]);
    assert!(
        f.docs
            .join(format!(
                "catalog/note-history/page/{}.json",
                &old_digest[7..]
            ))
            .is_file()
    );
    assert_eq!(fs::read(f.docs.join("notes/page.md")).unwrap(), original);
}

fn process_definition(id: &str, children: &[&str]) -> serde_json::Value {
    serde_json::json!({"schema":"codeclew-documentation-process/1.0","id":id,"title":format!("Maintained {id}"),"summary":"An explicitly saved quantity process.","root":{"service":"orders","selector":{"language":"java","owner":"Orders","name":"reserve","parameterTypes":["int"]}},"interactions":[],"maxDepth":4,"maxNodes":64,"process":{"scope":"Quantity handling across explicitly declared participants.","participants":["orders"],"objects":[],"trigger":"A caller requests a quantity.","outcomes":["Return a normalized quantity; failure paths remain evidence-bounded."],"linkedSubviews":children}})
}
fn save_process(f: &Fixture, definition: &serde_json::Value) -> serde_json::Value {
    let path = f.input("process.json", definition);
    let digest = f.ok(&["docs", "process", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "process",
        "put",
        "--input",
        path.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ])
}

#[test]
fn docsys_t10_linked_cycles_missing_children_and_negative_interaction_scope() {
    use serde_json::json;
    let f = Fixture::new();
    f.service("orders");
    f.service("other");
    save_process(&f, &process_definition("parent", &["child", "missing"]));
    save_process(&f, &process_definition("child", &["parent"]));
    let checked = f.checked();
    assert!(checked.dependencies.values().any(|d|d.kind=="PROCESS_COMPONENT"&&d.normalized["gap"]=="LINKED_PROCESS_CYCLE"));
    assert_eq!(
        checked.dependencies["process-component:parent:missing"].normalized["gap"],
        "LINKED_PROCESS_MISSING"
    );
    assert!(
        checked
            .dependencies
            .values()
            .filter(|d| d.kind == "PROCESS_COMPONENT")
            .all(|d| d.source_ids.is_empty() && d.normalized["accepted"].is_null())
    );
    let old = clew::documentation::bindings::fragment(
        "scenario:parent",
        &json!({"negative":"No interaction was declared."}),
        &["process-scope:parent".into()],
        &[],
        &checked,
    )
    .unwrap();
    fs::write(f.docs.join("catalog/interactions/new-link.json"),serde_json::to_vec(&json!({"schema":"codeclew-documentation-interaction/1.0","id":"new-link","title":"Declared quantity forwarding","from":{"service":"orders"},"to":{"service":"other"},"transport":{"kind":"http","method":"POST","path":"/quantity"},"declaration":{"origin":"human","rationale":"An explicitly declared relationship."}})).unwrap()).unwrap();
    let changed = f.checked();
    assert_ne!(
        old.dependencies["process-scope:parent"],
        changed.dependencies["process-scope:parent"].digest
    );
    assert_eq!(
        changed.dependencies["process-scope:parent"].normalized["interactionMembership"],
        json!(["new-link"])
    );
    assert!(
        old.dependencies.contains_key("scenario:child")
            && old
                .dependencies
                .contains_key("process-component:child:parent")
    );
}

#[test]
fn docsys_t10_two_service_conditional_definition_preserves_unresolved_transport() {
    use serde_json::json;
    let f = Fixture::new();
    let orders = f.service("orders");
    f.service("other");
    fs::write(orders.join("Orders.java"),"public class Orders { public int reserve(int quantity) { if (quantity < 0) { throw new IllegalArgumentException(); } return normalize(quantity); } private int normalize(int quantity) { return quantity; } }\n").unwrap();
    commit(&orders);
    fs::write(f.docs.join("catalog/interactions/reserve-link.json"),serde_json::to_vec(&json!({"schema":"codeclew-documentation-interaction/1.0","id":"reserve-link","title":"Declared transfer","from":{"service":"orders","selector":{"language":"java","owner":"Orders","name":"reserve","parameterTypes":["int"]}},"to":{"service":"other","selector":{"language":"java","owner":"Orders","name":"reserve","parameterTypes":["int"]}},"transport":{"kind":"http","method":"POST","path":"/reserve"},"declaration":{"origin":"human","rationale":"Declared link; static call-site authority remains required."}})).unwrap()).unwrap();
    let mut definition = process_definition("conditional", &[]);
    definition["process"]["participants"] = json!(["orders", "other"]);
    definition["process"]["outcomes"] = json!([
        "Reject a negative quantity.",
        "Return a normalized quantity."
    ]);
    definition["interactions"] = json!(["reserve-link"]);
    save_process(&f, &definition);
    let checked = f.checked();
    let context = &checked.scenarios["conditional"];
    assert!(context.steps.iter().any(|s| s.kind == "IF"));
    assert!(context.steps.iter().any(|s| s.kind == "THROW"));
    assert!(context.steps.iter().any(|s| s.kind == "RETURN"));
    assert!(
        context
            .boundaries
            .iter()
            .any(|b| b == "DECLARED_TRANSITION_NOT_REACHED:reserve-link")
    );
    assert!(
        !context
            .steps
            .iter()
            .any(|s| s.kind == "DECLARED_HTTP_TRANSITION")
    );
    assert_eq!(checked.interactions["reserve-link"].runtime, "UNKNOWN");
    assert!(
        context
            .dependency_ids
            .contains(&"interaction:reserve-link".into())
    );
}
