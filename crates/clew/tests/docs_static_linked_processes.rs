//! Real Maven/compiler fixture -> bounded linked bodies -> mutations -> offline bytes.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::{
    canonical,
    documentation::{
        check::Check, model::ServiceEvidence, static_pages::model::BundleProjection,
        store::Repository,
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use support::{Fixture, commit, git};

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let destination = target.join(name);
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            assert!(entry.file_type().unwrap().is_file());
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn declaration(e: &ServiceEvidence, owner: &str, name: &str) -> String {
    let rows: Vec<_> = e
        .observations
        .values()
        .filter(|o| {
            o.kind == "SYMBOL"
                && o.normalized["declarationKind"] == "METHOD"
                && o.normalized["ownerIdentity"] == format!("class:example.linked.{owner}")
                && o.normalized["name"] == name
                && o.normalized["scope"] == ":/main"
        })
        .collect();
    assert_eq!(rows.len(), 1, "{owner}.{name}: {rows:?}");
    rows[0].id.clone()
}

fn capture(
    f: &Fixture,
    label: &str,
    sink: Option<&Path>,
    timings: &mut BTreeMap<String, u128>,
) -> (String, Check) {
    let began = Instant::now();
    let output = f.run_raw_with_path(&["docs", "check", "--service", "linked"], f.tools_dir());
    timings.insert(format!("capture-{label}-ms"), began.elapsed().as_millis());
    if let Some(sink) = sink {
        fs::write(sink.join(format!("capture-{label}.stdout")), &output.stdout).unwrap();
        fs::write(sink.join(format!("capture-{label}.stderr")), &output.stderr).unwrap();
        fs::write(
            sink.join("timings.json"),
            serde_json::to_vec_pretty(timings).unwrap(),
        )
        .unwrap();
    }
    assert!(
        matches!(output.status.code(), Some(0 | 3 | 4)),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "CHECKED", "{result}");
    assert_eq!(result["unresolved"], json!({}), "{result}");
    let snapshot = result["snapshot"].as_str().unwrap().to_owned();
    let checked = Check::load_snapshot(&Repository::open(&f.docs).unwrap(), &snapshot).unwrap();
    (snapshot, checked)
}

fn selections(checked: &Check) -> Value {
    let e = &checked.services["linked"];
    let wiring = declaration(e, "Composition", "wire");
    json!([
        {"id":"parent-a","service":"linked","endpointDeclaration":declaration(e,"ParentAEndpoint","submit"),"workerDeclaration":declaration(e,"ParentAWorker","runOnce"),"wiringDeclaration":wiring,"expandSourceCalls":true},
        {"id":"parent-b","service":"linked","endpointDeclaration":declaration(e,"ParentBEndpoint","enqueue"),"workerDeclaration":declaration(e,"ParentBWorker","runOnce"),"wiringDeclaration":wiring,"expandSourceCalls":true},
        {"id":"child","service":"linked","endpointDeclaration":declaration(e,"ChildEndpoint","submit"),"workerDeclaration":declaration(e,"ChildWorker","runOnce"),"wiringDeclaration":wiring,"expandSourceCalls":true},
        {"id":"cycle","service":"linked","endpointDeclaration":declaration(e,"CycleProbe","first"),"workerDeclaration":declaration(e,"CycleProbe","second"),"expandSourceCalls":true}
    ])
}

fn files(path: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            assert!(entry.file_type().unwrap().is_file());
            (
                entry.file_name().to_str().unwrap().to_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn verify(path: &Path, p: &BundleProjection) {
    let manifest: Value =
        serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
    for row in manifest["files"].as_array().unwrap() {
        assert_eq!(
            canonical::hash_bytes(&fs::read(path.join(row["path"].as_str().unwrap())).unwrap()),
            row["digest"]
        );
    }
    let ids: BTreeSet<_> = manifest["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    for rows in manifest["reverseExaminedPages"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for row in rows.as_array().unwrap() {
            assert!(ids.contains(row["pageId"].as_str().unwrap()));
        }
    }
    for row in manifest["pages"].as_array().unwrap() {
        let slug = row["id"].as_str().unwrap();
        assert_eq!(
            row["contentDigest"],
            canonical::hash(&(p, slug)).unwrap(),
            "Existing whole-bundle contentDigest meaning changed"
        );
        let html = fs::read_to_string(path.join(row["html"].as_str().unwrap())).unwrap();
        let mdx = fs::read_to_string(path.join(row["mdx"].as_str().unwrap())).unwrap();
        let body = html
            .split_once("<body>\n")
            .unwrap()
            .1
            .strip_suffix("</body></html>\n")
            .unwrap();
        let mut normalized = String::new();
        let mut pieces = mdx.split("href=\"");
        normalized.push_str(pieces.next().unwrap());
        for piece in pieces {
            let (href, rest) = piece.split_once('"').unwrap();
            normalized.push_str("href=\"");
            normalized.push_str(&href.replace(".mdx", ".html"));
            normalized.push('"');
            normalized.push_str(rest);
        }
        assert_eq!(body, normalized);
        assert!(
            !mdx.contains("<script")
                && !mdx.contains("import ")
                && !mdx.contains('{')
                && !mdx.contains('`')
        );
        for document in [&html, &mdx] {
            for rest in document.split("href=\"").skip(1) {
                let href = rest.split('"').next().unwrap();
                if href.starts_with("http://") || href.starts_with("https://") {
                    continue;
                }
                let (file, fragment) = href
                    .split_once('#')
                    .map(|(a, b)| (a, Some(b)))
                    .unwrap_or((href, None));
                let target = fs::read_to_string(path.join(file)).unwrap();
                if let Some(fragment) = fragment {
                    assert!(
                        target.contains(&format!("id=\"{fragment}\"")),
                        "broken {href}"
                    );
                }
            }
        }
    }
}

fn render(
    f: &Fixture,
    snapshot: &str,
    selected: &Value,
    label: &str,
    sink: Option<&Path>,
    timings: &mut BTreeMap<String, u128>,
) -> (PathBuf, BundleProjection) {
    let input = f.input(&format!("selection-{label}.json"), selected);
    let output = sink
        .map(|p| p.join(label))
        .unwrap_or_else(|| f.temp.path().join(label));
    let began = Instant::now();
    let result = f.ok(&[
        "docs",
        "pages",
        "render",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    timings.insert(format!("render-{label}-ms"), began.elapsed().as_millis());
    let p = serde_json::from_slice(&fs::read(output.join("projection.json")).unwrap()).unwrap();
    assert_eq!(result["status"], "RENDERED");
    if let Some(sink) = sink {
        fs::write(
            sink.join("timings.json"),
            serde_json::to_vec_pretty(timings).unwrap(),
        )
        .unwrap();
    }
    verify(&output, &p);
    (output, p)
}

fn reviewed(p: &BundleProjection, id: &str) -> String {
    p.pages
        .iter()
        .find(|p| p.id == id)
        .unwrap()
        .examined_sources
        .as_ref()
        .unwrap()
        .examined_source_digest
        .clone()
}

fn child_calls(p: &BundleProjection, parent: &str) -> Vec<String> {
    p.source_call_graph
        .as_ref()
        .unwrap()
        .process_links
        .iter()
        .filter(|l| l.from_process == parent && l.to_process == "child")
        .map(|l| l.relation_id.clone())
        .collect()
}

#[test]
#[ignore = "runs real Maven/compiler capture for four committed variants; JDK21 and Maven required"]
fn shared_child_expansion_local_review_mutations_and_offline_snapshot() {
    let sink = std::env::var_os("CODECLEW_LINKED_PROCESS_TEST_OUTPUT").map(PathBuf::from);
    if let Some(sink) = &sink {
        assert!(!sink.exists(), "artifact directory must be new");
        fs::create_dir_all(sink).unwrap();
    }
    let f = Fixture::new();
    let source = f.temp.path().join("linked-source");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/native-linked-processes"),
        &source,
    );
    git(&source, &["init", "-q", "-b", "main"]);
    git(
        &source,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/native-linked-processes",
        ],
    );
    commit(&source);
    let record = f.input(
        "linked-service.json",
        &serde_json::from_slice(&fs::read(source.join("service.json")).unwrap()).unwrap(),
    );
    let current = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        record.to_str().unwrap(),
        "--expected-input-digest",
        &current,
    ]);
    f.ok(&[
        "docs",
        "bind",
        "--service",
        "linked",
        "--repo",
        source.to_str().unwrap(),
    ]);
    let mut timings = BTreeMap::new();
    let (snapshot, checked) = capture(&f, "baseline", sink.as_deref(), &mut timings);
    let selected = selections(&checked);
    let (baseline_path, baseline) = render(
        &f,
        &snapshot,
        &selected,
        "baseline",
        sink.as_deref(),
        &mut timings,
    );
    let frozen = files(&baseline_path);
    for page in baseline.pages.iter().filter(|p| p.id != "cycle") {
        assert_eq!(
            page.handoff.status, "SOURCE_DECLARED_SHARED_QUEUE",
            "{:?}",
            page.handoff
        );
    }
    assert_eq!(child_calls(&baseline, "parent-a").len(), 1);
    assert_eq!(child_calls(&baseline, "parent-b").len(), 1);
    assert_ne!(
        child_calls(&baseline, "parent-a"),
        child_calls(&baseline, "parent-b")
    );
    let graph = baseline.source_call_graph.as_ref().unwrap();
    for page in baseline.pages.iter().filter(|p| p.id != "cycle") {
        let constructors: Vec<_> = page
            .examined_sources
            .as_ref()
            .unwrap()
            .memberships
            .iter()
            .filter(|m| m.reason == "SELECTED_HANDOFF_CONSTRUCTOR")
            .collect();
        assert_eq!(
            constructors.len(),
            2,
            "{} handoff constructor membership",
            page.id
        );
        assert!(constructors.iter().all(|m| {
            let n = &graph.nodes[&m.node];
            n.observations[&n.callable.declaration_id].normalized["declarationKind"]
                == "CONSTRUCTOR"
        }));
    }
    let helper_id = declaration(&checked.services["linked"], "ChildWorker", "prepare");
    let helpers: Vec<_> = graph
        .nodes
        .values()
        .filter(|n| n.callable.declaration_id == helper_id)
        .collect();
    assert_eq!(helpers.len(), 1);
    let helper = helpers[0];
    assert!(
        helper
            .callable
            .state
            .iter()
            .any(|s| s.expression == "chosen.trim()")
    );
    assert!(
        helper
            .callable
            .state
            .iter()
            .any(|s| s.expression == "\"anonymous\"")
    );
    assert_eq!(
        graph.reverse_examined_processes[&helper.id]
            .iter()
            .map(|r| r.process_id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["parent-a", "parent-b", "child"])
    );
    assert!(graph.nodes.values().flat_map(|n| &n.calls).any(|e| {
        e.frontiers
            .iter()
            .any(|g| g.code == "SOURCE_CALL_CYCLE_FRONTIER")
    }));
    let child = graph
        .nodes
        .values()
        .find(|n| {
            n.callable.declaration_id
                == declaration(&checked.services["linked"], "ChildWorker", "runOnce")
        })
        .unwrap();
    let gateway = child
        .calls
        .iter()
        .find(|e| e.call.name == "deliver")
        .unwrap();
    assert_eq!(gateway.status, "BODY_UNAVAILABLE");
    assert_eq!(gateway.runtime_dispatch, "UNRESOLVED");
    assert!(gateway.target_node.is_none());
    assert!(
        !gateway
            .conditions
            .iter()
            .any(|c| c.expression.contains("response"))
    );
    assert!(
        child
            .calls
            .iter()
            .find(|e| e.call.name == "prepare")
            .unwrap()
            .target_node
            .is_some()
    );
    for (parent, conditions) in [
        (
            "parent-a",
            ["!enabled", "!task.eligible", "task.priority < 1"].as_slice(),
        ),
        (
            "parent-b",
            ["task.priority < 3", "task.name == null", "useAlternative"].as_slice(),
        ),
    ] {
        let relation = graph
            .process_links
            .iter()
            .find(|l| l.from_process == parent && l.to_process == "child")
            .unwrap();
        let edge = graph.nodes[&relation.caller_node]
            .calls
            .iter()
            .find(|e| e.occurrence_path == relation.occurrence_path)
            .unwrap();
        assert_eq!(edge.call.arguments, ["task"]);
        for expected in conditions {
            assert!(
                edge.conditions
                    .iter()
                    .any(|c| c.expression.contains(expected) && !c.holds),
                "{parent}: {:?}",
                edge.conditions
            );
        }
        let html = fs::read_to_string(baseline_path.join(format!("{parent}-worker.html"))).unwrap();
        assert!(html.contains("href=\"child-overview.html\""));
    }
    let child_html = fs::read_to_string(baseline_path.join("child-worker.html")).unwrap();
    assert!(child_html.contains("href=\"source-calls.html#ref-"));
    assert!(
        fs::read_to_string(baseline_path.join("source-calls.html"))
            .unwrap()
            .contains("anonymous")
    );
    let mut explicit_false = selected.clone();
    for row in explicit_false.as_array_mut().unwrap() {
        row["expandSourceCalls"] = json!(false);
    }
    let (false_path, _) = render(
        &f,
        &snapshot,
        &explicit_false,
        "explicit-false",
        sink.as_deref(),
        &mut timings,
    );
    let mut omitted = explicit_false;
    for row in omitted.as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("expandSourceCalls");
    }
    let (omitted_path, _) = render(
        &f,
        &snapshot,
        &omitted,
        "omitted",
        sink.as_deref(),
        &mut timings,
    );
    assert_eq!(files(&false_path), files(&omitted_path));
    let java = source.join("src/main/java/example/linked");
    let child_file = java.join("ChildWorker.java");
    let a_file = java.join("ParentAWorker.java");
    let b_file = java.join("ParentBWorker.java");
    let child_original = fs::read_to_string(&child_file).unwrap();
    let a_original = fs::read_to_string(&a_file).unwrap();
    let b_original = fs::read_to_string(&b_file).unwrap();
    fs::write(
        &child_file,
        child_original.replace(
            "chosen.trim()",
            "chosen.strip().toUpperCase(java.util.Locale.ROOT)",
        ),
    )
    .unwrap();
    commit(&source);
    let (child_snapshot, changed) = capture(&f, "child-transform", sink.as_deref(), &mut timings);
    let (_, child_changed) = render(
        &f,
        &child_snapshot,
        &selections(&changed),
        "child-transform",
        sink.as_deref(),
        &mut timings,
    );
    for id in ["parent-a", "parent-b", "child"] {
        assert_ne!(reviewed(&baseline, id), reviewed(&child_changed, id));
    }
    assert_eq!(
        reviewed(&baseline, "cycle"),
        reviewed(&child_changed, "cycle")
    );
    assert!(
        child_changed
            .source_call_graph
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .any(|n| n
                .callable
                .state
                .iter()
                .any(|s| s.expression == "chosen.strip().toUpperCase(java.util.Locale.ROOT)"))
    );
    fs::write(&child_file, &child_original).unwrap();
    fs::write(
        &a_file,
        a_original.replace("task.priority < 1", "task.priority < 2"),
    )
    .unwrap();
    // Move an unrelated body in its file: full call occurrence IDs change, but
    // the body-relative review fingerprint and distinct call ordinals remain.
    fs::write(&b_file, format!("\n\n{b_original}")).unwrap();
    commit(&source);
    let (a_snapshot, changed) = capture(&f, "a-guard-b-relocation", sink.as_deref(), &mut timings);
    let (_, a_changed) = render(
        &f,
        &a_snapshot,
        &selections(&changed),
        "a-guard-b-relocation",
        sink.as_deref(),
        &mut timings,
    );
    assert_ne!(
        reviewed(&baseline, "parent-a"),
        reviewed(&a_changed, "parent-a")
    );
    for id in ["parent-b", "child", "cycle"] {
        assert_eq!(reviewed(&baseline, id), reviewed(&a_changed, id));
    }
    assert_ne!(
        child_calls(&baseline, "parent-b"),
        child_calls(&a_changed, "parent-b")
    );
    fs::write(&a_file, &a_original).unwrap();
    fs::write(
        &b_file,
        b_original.replace("child.submit(task)", "alternative.submit(task)"),
    )
    .unwrap();
    commit(&source);
    let (b_snapshot, changed) = capture(&f, "b-retarget", sink.as_deref(), &mut timings);
    let (_, b_changed) = render(
        &f,
        &b_snapshot,
        &selections(&changed),
        "b-retarget",
        sink.as_deref(),
        &mut timings,
    );
    assert_eq!(child_calls(&b_changed, "parent-a").len(), 1);
    assert!(child_calls(&b_changed, "parent-b").is_empty());
    assert_ne!(
        reviewed(&baseline, "parent-b"),
        reviewed(&b_changed, "parent-b")
    );
    for id in ["parent-a", "child", "cycle"] {
        assert_eq!(reviewed(&baseline, id), reviewed(&b_changed, id));
    }
    let helper = b_changed
        .source_call_graph
        .as_ref()
        .unwrap()
        .nodes
        .values()
        .find(|n| n.callable.declaration_id == helper_id)
        .unwrap();
    assert!(
        !b_changed
            .source_call_graph
            .as_ref()
            .unwrap()
            .reverse_examined_processes[&helper.id]
            .iter()
            .any(|r| r.process_id == "parent-b")
    );
    assert_eq!(files(&baseline_path), frozen);
    fs::remove_dir_all(&source).unwrap();
    let (offline_path, offline) = render(
        &f,
        &snapshot,
        &selected,
        "offline-baseline",
        sink.as_deref(),
        &mut timings,
    );
    assert_eq!(offline, baseline);
    assert_eq!(files(&offline_path), frozen);
    if let Some(sink) = sink {
        fs::write(sink.join("journey.json"), serde_json::to_vec_pretty(&json!({"schema":"codeclew-native-linked-process-journey/1.0",
            "authority":"OWNED_SYNTHETIC_SOURCE_AND_COMPILER_NOT_RUNTIME_OR_CORPORATE_ACCEPTANCE",
            "baselineSnapshot":snapshot,"childTransformSnapshot":child_snapshot,"aGuardSnapshot":a_snapshot,"bRetargetSnapshot":b_snapshot,
            "baselineExaminedDigests":baseline.pages.iter().map(|p|(&p.id,reviewed(&baseline,&p.id))).collect::<BTreeMap<_,_>>(),
            "timings":timings,"offlineBytesEqual":true})).unwrap()).unwrap();
    }
}
