//! Fresh native compiler capture -> public linked static pages, then offline reuse.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::{
    canonical,
    documentation::{check::Check, model::ServiceEvidence, store::Repository},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};
use support::{Fixture, commit, git};
use zip::write::{SimpleFileOptions, ZipWriter};
const GATEWAY: &str =
    "package external;\npublic interface Gateway {\n int deliver(String value);\n}\n";
fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn tool(name: &str) -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(|home| Path::new(&home).join("bin").join(name))
        .unwrap_or_else(|| name.into())
}
fn setup(f: &Fixture, id: &str, attached: bool) -> PathBuf {
    let root = f.temp.path().join(id);
    fs::create_dir(&root).unwrap();
    write(&root, "dependency-src/external/Gateway.java", GATEWAY);
    fs::create_dir(root.join("dependency-classes")).unwrap();
    let output = Command::new(tool("javac"))
        .args(["--release", "17", "-d"])
        .arg(root.join("dependency-classes"))
        .arg(root.join("dependency-src/external/Gateway.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    write(
        &root,
        "dependency-classes/META-INF/maven/example/gateway/pom.properties",
        "groupId=example\nartifactId=gateway\nversion=1.2.3\n",
    );
    write(
        &root,
        "dependency-manifest.mf",
        "Manifest-Version: 1.0\nAutomatic-Module-Name: example.gateway\nImplementation-Version: 1.2.3\n\n",
    );
    fs::create_dir(root.join("libs")).unwrap();
    let output = Command::new(tool("jar"))
        .args(["--create", "--file"])
        .arg(root.join("libs/gateway.jar"))
        .arg("--manifest")
        .arg(root.join("dependency-manifest.mf"))
        .arg("-C")
        .arg(root.join("dependency-classes"))
        .arg(".")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if attached {
        let mut zip =
            ZipWriter::new(fs::File::create(root.join("libs/gateway-1.2.3-sources.jar")).unwrap());
        zip.start_file("external/Gateway.java", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(GATEWAY.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    fs::remove_file(root.join("dependency-manifest.mf")).unwrap();
    fs::remove_dir_all(root.join("dependency-src")).unwrap();
    fs::remove_dir_all(root.join("dependency-classes")).unwrap();
    let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../fixtures/native-static-pages/{id}/Pipeline.java"
    )))
    .unwrap();
    write(&root, "src/main/java/Pipeline.java", &source);
    write(
        &root,
        "pom.xml",
        "<project><modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>pipeline</artifactId><version>1</version></project>\n",
    );
    write(&root, ".gitignore", "target/\n");
    // The wrapper supplies read-only javac model metadata, not a build result.
    write(
        &root,
        "mvnw",
        r#"#!/bin/sh
set -eu
case "$*" in
 *help:effective-pom*)
  for argument in "$@"; do case "$argument" in -Doutput=*) effective_output=${argument#-Doutput=} ;; esac; done
  module=$(pwd -P)
  cat > "$effective_output" <<XML
<project><build><directory>$module/target</directory><sourceDirectory>$module/src/main/java</sourceDirectory><testSourceDirectory>$module/src/test/java</testSourceDirectory><outputDirectory>$module/target/classes</outputDirectory><testOutputDirectory>$module/target/test-classes</testOutputDirectory></build></project>
XML
  ;;
 *dependency:build-classpath*) mkdir -p target/classes; printf '%s\n' "$(pwd -P)/libs/gateway.jar" > target/codeclew-classpath.txt ;;
 *help:evaluate*) printf '17\n' ;;
 *) exit 25 ;;
esac
"#,
    );
    fs::set_permissions(root.join("mvnw"), fs::Permissions::from_mode(0o755)).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &format!("https://example.invalid/{id}"),
        ],
    );
    commit(&root);
    let record = f.input(&format!("{id}.json"),&json!({"schema":"codeclew-documentation-service/1.0","id":id,"title":id,"repositoryId":id,"repository":format!("https://example.invalid/{id}"),"language":"java","profile":"java-17plus-maven-read-only","compilations":[":/main"],"targetRef":"main"}));
    let digest = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_string();
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        record.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ]);
    f.ok(&[
        "docs",
        "bind",
        "--service",
        id,
        "--repo",
        root.to_str().unwrap(),
    ]);
    root
}
fn capture(f: &Fixture) -> (String, Check) {
    let output = f.run_raw_with_path(&["docs", "check"], f.tools_dir());
    assert!(
        matches!(output.status.code(), Some(0 | 3 | 4)),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let snapshot = value["snapshot"].as_str().unwrap().to_string();
    let check = Check::load_snapshot(&Repository::open(&f.docs).unwrap(), &snapshot).unwrap();
    (snapshot, check)
}
fn declaration(e: &ServiceEvidence, owner: &str, method: &str) -> String {
    let matches: Vec<_> = e
        .observations
        .values()
        .filter(|o| {
            o.kind == "SYMBOL"
                && o.normalized["declarationKind"] == "METHOD"
                && o.normalized["ownerIdentity"] == format!("class:{owner}")
                && o.normalized["name"] == method
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "owner={owner} method={method}, symbols={:?}",
        e.observations
            .values()
            .filter(|o| o.kind == "SYMBOL")
            .map(|o| (&o.id, &o.normalized))
            .collect::<Vec<_>>()
    );
    matches[0].id.clone()
}
const NOTE_TEXT: &str = "Before retrying, inspect the gateway response and consult the on-call guide.\r\nKeep café☕ available. <Panel>{probe()}</Panel> <script> `quoted`\r\n";
fn import_operational_note(f: &Fixture) -> (PathBuf, Value) {
    let source = f.temp.path().join("on-call-original.md");
    fs::write(&source, NOTE_TEXT.as_bytes()).unwrap();
    let association = json!({
        "schema":"codeclew-documentation-note-association/1.0","id":"on-call",
        "title":"Gateway retry instructions","service":"alpha","path":"notes/on-call.md",
        "targets":["service:alpha/section-egress"],"classification":"policy",
        "period":"2026-10 onward; maintainer review required","tags":["operations"],
        "metadata":{"author":"Example on-call maintainer","origin":"fixture import"}
    });
    let input = f.input("on-call-association.json", &association);
    let digest = f.ok(&["docs", "note", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "note",
        "import",
        "--input",
        input.to_str().unwrap(),
        "--source",
        source.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ]);
    (input, association)
}
fn selections(check: &Check) -> Value {
    json!([{"id":"alpha-flow","service":"alpha","endpointDeclaration":declaration(&check.services["alpha"],"alpha.DispatchEndpoint","submit"),"workerDeclaration":declaration(&check.services["alpha"],"alpha.ProcessingLoop","runOnce"),"wiringDeclaration":declaration(&check.services["alpha"],"alpha.Composition","assemble"),"question":"Why might the gateway call remain unreached?","noteIds":["on-call"]},
    {"id":"beta-flow","service":"beta","endpointDeclaration":declaration(&check.services["beta"],"beta.IntakeEndpoint","enqueue"),"workerDeclaration":declaration(&check.services["beta"],"beta.DeliveryLoop","consume"),"wiringDeclaration":declaration(&check.services["beta"],"beta.Assembly","wire"),"question":"What should be inspected when no delivery call is reached?"}])
}
fn render(f: &Fixture, snapshot: &str, input: &Path, out: &Path) -> Value {
    f.ok(&[
        "docs",
        "pages",
        "render",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ])
}
fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            assert!(entry.file_type().unwrap().is_file());
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn files(out: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(out)
        .unwrap()
        .map(|entry| {
            let p = entry.unwrap().path();
            (
                p.file_name().unwrap().to_str().unwrap().to_string(),
                fs::read(p).unwrap(),
            )
        })
        .collect()
}
fn verify_bundle(out: &Path) {
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    for file in manifest["files"].as_array().unwrap() {
        assert_eq!(
            canonical::hash_bytes(&fs::read(out.join(file["path"].as_str().unwrap())).unwrap()),
            file["digest"]
        );
    }
    for view in manifest["pages"].as_array().unwrap() {
        let mdx = fs::read_to_string(out.join(view["mdx"].as_str().unwrap())).unwrap();
        let html = fs::read_to_string(out.join(view["html"].as_str().unwrap())).unwrap();
        let body = html
            .split_once("<body>\n")
            .unwrap()
            .1
            .strip_suffix("</body></html>\n")
            .unwrap();
        let mut normalized = String::new();
        let mut parts = mdx.split("href=\"");
        normalized.push_str(parts.next().unwrap());
        for part in parts {
            let (href, rest) = part.split_once('"').unwrap();
            normalized.push_str("href=\"");
            normalized.push_str(&href.replace(".mdx", ".html"));
            normalized.push('"');
            normalized.push_str(rest);
        }
        assert_eq!(body, normalized);
        assert!(!mdx.contains("<script") && !mdx.contains("import "));
        for document in [&mdx, &html] {
            for rest in document.split("href=\"").skip(1) {
                let href = rest.split('"').next().unwrap();
                let (path, fragment) = href
                    .split_once('#')
                    .map(|(p, a)| (p, Some(a)))
                    .unwrap_or((href, None));
                if path.starts_with("https://") || path.starts_with("http://") {
                    continue;
                }
                let target = fs::read_to_string(out.join(path)).unwrap();
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
#[test]
#[ignore = "runs real native Maven/javac capture; requires repository JDK"]
fn fresh_native_capture_mutation_and_offline_snapshot_produce_linked_pages() {
    let f = Fixture::new();
    let alpha = setup(&f, "alpha", true);
    let beta = setup(&f, "beta", false);
    let (_note_input, note_association) = import_operational_note(&f);
    let original_note = fs::read(f.docs.join("notes/on-call.md")).unwrap();
    let original_association = fs::read(f.docs.join("catalog/notes/on-call.json")).unwrap();
    let capture_started = Instant::now();
    let (v1, c1) = capture(&f);
    let capture_v1_ms = capture_started.elapsed().as_millis();
    let selection = f.input("selection.json", &selections(&c1));
    let out1 = f.temp.path().join("pages-v1");
    let render_started = Instant::now();
    let result1 = render(&f, &v1, &selection, &out1);
    let render_v1_ms = render_started.elapsed().as_millis();
    verify_bundle(&out1);
    let original = files(&out1);
    let p1: Value = serde_json::from_slice(&original["projection.json"]).unwrap();
    let instruction1 = &p1["pages"][0]["humanInstructions"][0];
    assert_eq!(instruction1["id"], "on-call");
    assert_eq!(instruction1["declaredAuthor"], "Example on-call maintainer");
    assert_eq!(instruction1["text"], NOTE_TEXT);
    assert_eq!(instruction1["classification"], "policy");
    assert_eq!(instruction1["period"], note_association["period"]);
    assert_eq!(instruction1["authority"], "HUMAN_OR_IMPORTED_UNVERIFIED");
    assert_eq!(instruction1["sourceClaimStatus"], "UNASSESSED");
    assert_eq!(
        instruction1["contentDigest"],
        canonical::hash(&NOTE_TEXT).unwrap()
    );
    assert!(p1["pages"][1].get("humanInstructions").is_none());
    let manifest1: Value = serde_json::from_slice(&original["manifest.json"]).unwrap();
    assert_eq!(manifest1["selectedNotes"].as_array().unwrap().len(), 1);
    assert_eq!(manifest1["selectedNotes"][0]["noteId"], "on-call");
    assert_eq!(
        manifest1["selectedNotes"][0]["versionDigest"],
        instruction1["versionDigest"]
    );
    let inclusion = &manifest1["selectedNotes"][0];
    assert_eq!(inclusion["pageId"], "alpha-flow-overview");
    assert_eq!(inclusion["processId"], "alpha-flow");
    let included_page = manifest1["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["id"] == inclusion["pageId"])
        .expect("selected note inclusion must name a manifest page");
    assert_eq!(inclusion["html"], included_page["html"]);
    assert_eq!(inclusion["mdx"], included_page["mdx"]);
    for ext in ["html", "mdx"] {
        let overview = fs::read_to_string(out1.join(format!("alpha-flow-overview.{ext}"))).unwrap();
        assert!(
            overview.contains("Operational instructions")
                && overview.contains("Declared author: Example on-call maintainer")
        );
        assert!(overview.contains(
            "café☕ available. &lt;Panel&gt;&#123;probe()&#125;&lt;/Panel&gt; &lt;script&gt;"
        ));
        assert!(overview.contains("&#13;&#10;"));
        for view in ["endpoint", "worker", "fields-state", "diagnostic"] {
            let text = fs::read_to_string(out1.join(format!("alpha-flow-{view}.{ext}"))).unwrap();
            assert!(text.contains(&format!(
                "alpha-flow-overview.{ext}#operational-instructions"
            )));
        }
    }
    // Unavailable opted-in material refuses before creating the bundle directory.
    let mut missing = selections(&c1);
    missing[0]["noteIds"] = json!(["missing-note"]);
    let missing = f.input("missing-note-selection.json", &missing);
    let refused = f.temp.path().join("refused-note-bundle");
    let (code, _) = f.run(&[
        "docs",
        "pages",
        "render",
        "--snapshot",
        &v1,
        "--input",
        missing.to_str().unwrap(),
        "--output",
        refused.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(!refused.exists());
    assert_eq!(
        p1["pages"][0]["handoff"]["status"],
        "SOURCE_DECLARED_SHARED_QUEUE"
    );
    assert_ne!(
        p1["pages"][1]["handoff"]["status"],
        "SOURCE_DECLARED_SHARED_QUEUE"
    );
    let text1 = serde_json::to_string(&p1).unwrap();
    assert!(text1.contains("taskType, body.type, body.name, body.eligible"));
    assert!(text1.contains("task.name == null") && text1.contains("anonymous"));
    assert!(text1.contains("status != 0") && text1.contains("rejected"));
    assert!(text1.contains("missing-settings") && text1.contains("fallback"));
    let worker1 = &p1["pages"][0]["worker"];
    assert!(worker1["steps"].as_array().unwrap().iter().any(|step| {
        step["expression"]
            .as_str()
            .is_some_and(|s| s.contains("gateway.deliver(transformed)"))
    }));
    let gateway1 = worker1["steps"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|step| step["calls"].as_array().unwrap())
        .find(|call| call["name"] == "deliver")
        .unwrap();
    assert_eq!(
        gateway1["externalBoundary"]["sourceStatus"],
        "SOURCE_ATTACHED"
    );
    assert!(
        !gateway1["externalBoundary"]["citationIds"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(p1["pages"][0]["sources"].as_object().unwrap().values().any(
        |source| source["authority"] == "EXACT_DEPENDENCY_SOURCE_ARCHIVE"
            && source["url"].is_null()
            && source["file"] == "external/Gateway.java"
    ));
    let gateway2 = p1["pages"][1]["worker"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|step| step["calls"].as_array().unwrap())
        .find(|call| call["name"] == "deliver")
        .unwrap();
    assert!(
        gateway2["externalBoundary"]["citationIds"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_ne!(
        gateway2["externalBoundary"]["sourceStatus"],
        "SOURCE_ATTACHED"
    );
    assert!(
        p1["pages"][1]["observations"]
            .as_object()
            .unwrap()
            .values()
            .any(|o| o["kind"] == "DEPENDENCY_TARGET"
                && o["normalized"]["name"] == "deliver"
                && o["normalized"]["binaryOrigin"]["artifact"]["digest"]
                    .as_str()
                    .is_some())
    );
    assert!(
        fs::read_to_string(out1.join("alpha-flow-worker.mdx"))
            .unwrap()
            .contains("&#64;EXT&#64; .mdx &#123;probe()&#125; &lt;script&gt;")
    );
    let source = fs::read_to_string(alpha.join("src/main/java/Pipeline.java")).unwrap();
    let changed = source
        .replace(
            "if (!current.enabled)",
            "if (!current.enabled || current.prefix == null)",
        )
        .replace("chosen.trim()", "chosen.toUpperCase()");
    write(&alpha, "src/main/java/Pipeline.java", &changed);
    commit(&alpha);
    let capture_started = Instant::now();
    let (v2, c2) = capture(&f);
    let capture_v2_ms = capture_started.elapsed().as_millis();
    assert_ne!(v1, v2);
    assert_ne!(c1.context_digest, c2.context_digest);
    let selection2 = f.input("selection-v2.json", &selections(&c2));
    let out2 = f.temp.path().join("pages-v2");
    let render_started = Instant::now();
    let result2 = render(&f, &v2, &selection2, &out2);
    let render_v2_ms = render_started.elapsed().as_millis();
    verify_bundle(&out2);
    assert_ne!(result1["projectionDigest"], result2["projectionDigest"]);
    let p2: Value =
        serde_json::from_slice(&fs::read(out2.join("projection.json")).unwrap()).unwrap();
    assert_eq!(
        p1["pages"][0]["humanInstructions"],
        p2["pages"][0]["humanInstructions"]
    );
    assert_eq!(
        fs::read(f.docs.join("notes/on-call.md")).unwrap(),
        original_note
    );
    assert_eq!(
        fs::read(f.docs.join("catalog/notes/on-call.json")).unwrap(),
        original_association
    );
    let saved_v1 = Check::load_snapshot(&Repository::open(&f.docs).unwrap(), &v1).unwrap();
    assert_eq!(
        saved_v1.source_inputs.as_ref().unwrap().inputs.notes["on-call"],
        c1.source_inputs.as_ref().unwrap().inputs.notes["on-call"]
    );
    assert_ne!(p1["pages"][0]["diagnostics"], p2["pages"][0]["diagnostics"]);
    assert!(
        serde_json::to_string(&p2["pages"][0]["diagnostics"])
            .unwrap()
            .contains("current.prefix == null")
    );
    assert_ne!(p1["pages"][0]["citations"], p2["pages"][0]["citations"]);
    assert!(
        fs::read_to_string(out2.join("alpha-flow-worker.html"))
            .unwrap()
            .contains("chosen.toUpperCase()")
    );
    assert_eq!(original, files(&out1));
    // Change the live association through the public lifecycle, then remove it.
    // Neither operation changes the protected original or the selected old capture.
    let inspected = f.ok(&["docs", "note", "inspect", "--path", "notes/on-call.md"]);
    let mut changed_association = note_association;
    changed_association["title"] = json!("Changed live instruction title");
    changed_association["metadata"]["author"] = json!("Another declared maintainer");
    let changed_association = f.input("changed-note-association.json", &changed_association);
    let associated = f.ok(&[
        "docs",
        "note",
        "associate",
        "--input",
        changed_association.to_str().unwrap(),
        "--expected-input-digest",
        inspected["inputDigest"].as_str().unwrap(),
        "--expected-note-digest",
        inspected["original"]["digest"].as_str().unwrap(),
    ]);
    f.ok(&[
        "docs",
        "note",
        "remove",
        "--id",
        "on-call",
        "--expected-input-digest",
        associated["inputDigest"].as_str().unwrap(),
    ]);
    assert!(!f.docs.join("catalog/notes/on-call.json").exists());
    assert_eq!(
        fs::read(f.docs.join("notes/on-call.md")).unwrap(),
        original_note
    );
    // Source checkouts and latest pointer become unavailable. Rendering v1 stays exact.
    fs::remove_dir_all(alpha).unwrap();
    fs::remove_dir_all(beta).unwrap();
    fs::write(
        f.docs.join(".codeclew/cache/latest-check.json"),
        b"unavailable latest",
    )
    .unwrap();
    let offline = f.temp.path().join("pages-offline");
    let render_started = Instant::now();
    let offline_result = render(&f, &v1, &selection, &offline);
    let render_offline_ms = render_started.elapsed().as_millis();
    assert_eq!(
        offline_result["projectionDigest"],
        result1["projectionDigest"]
    );
    assert_eq!(original, files(&offline));
    if let Some(destination) = std::env::var_os("CODECLEW_NATIVE_PAGE_TEST_OUTPUT") {
        let destination = PathBuf::from(destination);
        assert!(!destination.exists(), "artifact destination must be new");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("timings.json"), serde_json::to_vec_pretty(&json!({"schema":"codeclew-native-page-test-timings/1.0","scope":"authenticated CLI integration fixture; tiny synthetic Java services; includes subprocess startup","captureV1Ms":capture_v1_ms,"captureV2Ms":capture_v2_ms,"renderV1Ms":render_v1_ms,"renderV2Ms":render_v2_ms,"renderOfflineMs":render_offline_ms})).unwrap()).unwrap();
        copy_tree(&f.docs, &destination.join("docs-root"));
        fs::copy(&selection, destination.join("selection-v1.json")).unwrap();
        fs::copy(&selection2, destination.join("selection-v2.json")).unwrap();
        for (name, source) in [("v1", &out1), ("v2", &out2), ("offline", &offline)] {
            let dir = destination.join(name);
            fs::create_dir(&dir).unwrap();
            for (name, bytes) in files(source) {
                fs::write(dir.join(name), bytes).unwrap();
            }
        }
    }
    let (code, _) = f.run(&[
        "docs",
        "pages",
        "render",
        "--snapshot",
        &v1,
        "--input",
        selection.to_str().unwrap(),
        "--output",
        out1.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert_eq!(original, files(&out1));
}

fn public_work(f: &Fixture, snapshot: &str, entrypoint: &str, audience: &str) -> String {
    let input = f.input(
        "native-authored-work.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0","audience":audience,
            "entrypoint":entrypoint,"maxItems":100,"maxBytes":49152
        }),
    );
    f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:alpha",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
    ])["work"]
        .as_str()
        .unwrap()
        .to_owned()
}
// Public preparation freezes selected human context once; packet reads use that Work.
fn public_maintained_packet(
    f: &Fixture,
    snapshot: &str,
    root: &str,
    selection: Option<&Value>,
    name: &str,
) -> (String, Value) {
    let mut request = json!({"schema":"codeclew-documentation-work-request/1.0", "audience":"Synthetic maintained context consumer", "contextProfile":"process-graph-v1", "rootDeclaration":root, "question":"Explain the selected current endpoint; keep historical human context attributed and unassessed.", "maxItems":100, "maxBytes":49152});
    if let Some(selected) = selection {
        request["maintainedParagraph"] = selected.clone();
    }
    let input = f.input(&format!("maintained-work-{name}.json"), &request);
    let prepared = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:alpha",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
    ]);
    let id = prepared["work"].as_str().unwrap().to_owned();
    let packet = f.ok(&["docs", "work", "packet", "--work", &id]);
    (id, packet)
}

fn complete_work(f: &Fixture, id: &str, operation: Option<&str>) -> Option<String> {
    let mut omitted = false;
    let mut cursor = None;
    loop {
        let input = f.input(
            "native-authored-read.json",
            &cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({"cursor":c})),
        );
        let output = f.run_raw(&[
            "docs",
            "work",
            "read",
            "--work",
            id,
            "--input",
            input.to_str().unwrap(),
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stdout.len() <= 49152);
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        if let Some(operation) = operation {
            omitted |= response["omitted"].as_array().unwrap().iter().any(|row| {
                row["kind"] == "RETAINED_OPERATION"
                    && row["id"] == operation
                    && row["reason"] == "ITEM_EXCEEDS_WORK_BYTE_BUDGET"
            });
        }
        cursor = response["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    if let Some(operation) = operation {
        assert!(
            omitted,
            "oversized retained operation must be declared omitted"
        );
        let mut text = String::new();
        loop {
            let mut request = json!({"schema":"codeclew-documentation-retained-part-request/1.0","kind":"RETAINED_OPERATION","id":operation});
            if let Some(c) = &cursor {
                request["cursor"] = json!(c);
            }
            let input = f.input("native-authored-part.json", &request);
            let output = f.run_raw(&[
                "docs",
                "work",
                "read-retained-part",
                "--work",
                id,
                "--input",
                input.to_str().unwrap(),
            ]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(output.stdout.len() <= 49152);
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["work"], id);
            assert_eq!(response["startByte"].as_u64().unwrap() as usize, text.len());
            let fragment = response["text"].as_str().unwrap();
            assert_eq!(
                response["fragmentDigest"],
                canonical::hash_bytes(fragment.as_bytes())
            );
            text.push_str(fragment);
            assert_eq!(response["endByte"].as_u64().unwrap() as usize, text.len());
            cursor = response["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Some(text)
    } else {
        None
    }
}
fn submit_and_publish(f: &Fixture, work: &str, proposal: &Value) -> Value {
    let input = f.input("native-authored-proposal.json", proposal);
    let submitted = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        work,
        "--input",
        input.to_str().unwrap(),
    ]);
    assert!(
        submitted["status"].as_str().unwrap().starts_with("READY_"),
        "{submitted}"
    );
    let publication = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        submitted["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(publication["meaningReview"], "UNASSESSED");
    assert_eq!(publication["updateFailures"], json!({}), "{publication}");
    publication
}
fn frozen_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), out);
            } else {
                assert!(entry.file_type().unwrap().is_file());
                out.insert(
                    entry.path().strip_prefix(root).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
#[ignore = "runs public manual Work and real native Maven/javac capture; requires repository JDK"]
fn frozen_authored_paragraph_consumes_original_sources_beside_current_native_page_offline() {
    use clew::documentation::{model::Operation, work};
    let assert_source_calls = |out: &Path, projection: &Value| {
        verify_bundle(out);
        let page = &projection["pages"][0];
        assert_eq!(page["selection"]["expandSourceCalls"], true);
        let graph = &projection["sourceCallGraph"];
        assert_eq!(graph["schema"], "codeclew-native-source-calls/1.0");
        let nodes = graph["nodes"].as_object().unwrap();
        // The graph examines current native roots, independently of the authored
        // paragraph's original or explicitly migrated source pins.
        for role in ["endpoint", "worker", "wiring"] {
            let callable = &page[role];
            let node = nodes
                .values()
                .find(|n| n["callable"]["declarationId"] == callable["declarationId"])
                .expect("selected native root has a canonical source-call node");
            assert_eq!(node["callable"], *callable);
            let observation = &node["observations"][callable["declarationId"].as_str().unwrap()];
            for source in observation["sourceIds"].as_array().unwrap() {
                let id = source.as_str().unwrap();
                assert_eq!(node["sources"][id], page["sources"][id]);
                assert!(node["sources"][id]["text"].is_string());
            }
        }
        let manifest = support::read(out.join("manifest.json"));
        let rows = manifest["pages"].as_array().unwrap();
        assert!(rows.iter().any(|row| row["id"] == "source-calls"));
        let examined_digest = page["examinedSources"]["examinedSourceDigest"]
            .as_str()
            .unwrap();
        assert!(examined_digest.starts_with("sha256:"));
        let process_rows: Vec<_> = rows
            .iter()
            .filter(|row| row["id"].as_str().unwrap().starts_with("alpha-flow-"))
            .collect();
        assert_eq!(process_rows.len(), 5);
        for row in process_rows {
            assert_eq!(
                row["examinedSourceDigest"],
                page["examinedSources"]["examinedSourceDigest"]
            );
        }
        for ext in ["html", "mdx"] {
            let calls = fs::read_to_string(out.join(format!("source-calls.{ext}"))).unwrap();
            assert!(
                calls.contains("Retained source-call bodies") && calls.contains("ProcessingLoop")
            );
            let overview =
                fs::read_to_string(out.join(format!("alpha-flow-overview.{ext}"))).unwrap();
            let paragraph = &page["authoredParagraphs"][0]["paragraph"];
            let author_label = if paragraph["authorship"]["contextMigration"].is_object() {
                "Declared text author: Fixture &lt;maintainer&gt;"
            } else {
                "Declared author: Fixture &lt;maintainer&gt;"
            };
            assert!(overview.contains("Linked process source context"));
            assert!(overview.contains(author_label));
        }
    };
    let f = Fixture::new();
    let alpha = setup(&f, "alpha", true);
    // Compiler-resolved Spring annotations create a public discovered endpoint;
    // these are ordinary synthetic fixture sources, not answer annotations.
    write(
        &alpha,
        "src/main/java/org/springframework/stereotype/Controller.java",
        "package org.springframework.stereotype; public @interface Controller {}\n",
    );
    write(
        &alpha,
        "src/main/java/org/springframework/web/bind/annotation/RestController.java",
        "package org.springframework.web.bind.annotation; @org.springframework.stereotype.Controller public @interface RestController {}\n",
    );
    write(
        &alpha,
        "src/main/java/org/springframework/web/bind/annotation/RequestMapping.java",
        "package org.springframework.web.bind.annotation; public @interface RequestMapping { String[] path() default {}; String[] value() default {}; }\n",
    );
    let path = alpha.join("src/main/java/Pipeline.java");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("class DispatchEndpoint {", "@org.springframework.web.bind.annotation.RestController\nclass DispatchEndpoint {")
            .replace("    boolean submit(", "    @org.springframework.web.bind.annotation.RequestMapping(path=\"/submit\")\n    public boolean submit("),
    )
    .unwrap();
    commit(&alpha);
    let (v1, c1) = capture(&f);
    let service = &c1.services["alpha"];
    let declaration_v1 = declaration(service, "alpha.DispatchEndpoint", "submit");
    let symbol = &service.observations[&declaration_v1].symbol;
    let entry = service
        .entrypoints
        .iter()
        .find(|e| &e.symbol == symbol)
        .expect("public submit discovered entrypoint");
    let initial = public_work(
        &f,
        &v1,
        &entry.id,
        "Synthetic native authored paragraph baseline",
    );
    complete_work(&f, &initial, None);
    let repo = Repository::open(&f.docs).unwrap();
    let saved = work::load(&repo, &initial).unwrap();
    let reference = saved
        .handles
        .iter()
        .find(|(_, h)| h.kind == "ENTRYPOINT" && h.id == entry.id)
        .unwrap()
        .0;
    let flows: Vec<_> = saved
        .handles
        .iter()
        .filter(|(_, h)| {
            h.kind == "DEPENDENCY"
                && saved
                    .checked
                    .dependencies
                    .get(&h.id)
                    .is_some_and(|d| d.kind == "FLOW" && d.symbol == entry.symbol)
        })
        .map(|(r, _)| r)
        .collect();
    assert!(!flows.is_empty());
    let seed = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{
        "entrypoint":reference,"title":"Submit a task","summary":{"text":"The endpoint submits a task to its declared queue.","evidence":[reference]},
        "steps":flows.iter().enumerate().map(|(i,r)|json!({"kind":"note","meaning":{"text":format!("Captured source event {i}."),"evidence":[reference,r]}})).collect::<Vec<_>>(),
        "explanation":(0..14).map(|i|json!({"text":format!("Unrelated retained paragraph {i}. {}","Synthetic maintained explanation λ☕. ".repeat(120)),"evidence":[reference]})).collect::<Vec<_>>()
    }]});
    let seeded = submit_and_publish(&f, &initial, &seed);
    let seeded_data =
        support::read(f.bundle(seeded["bundle"].as_str().unwrap(), "services/alpha.json"));
    let original: Operation = serde_json::from_value(seeded_data["operations"][0].clone()).unwrap();
    assert!(canonical::bytes(&original).unwrap().len() > 49152);
    let paragraph = &original.explanation[0];
    let human = public_work(
        &f,
        &v1,
        &entry.id,
        "Declare maintained endpoint explanation",
    );
    let reconstructed = complete_work(&f, &human, Some(&entry.id)).unwrap();
    assert_eq!(
        reconstructed.as_bytes(),
        canonical::bytes(&original).unwrap()
    );
    let text = "Maintained λ☕ instruction: inspect {queue} and [policy](local). This is user documentation, not a source-verified assertion.\nKeep the original context.";
    let authored = submit_and_publish(
        &f,
        &human,
        &json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],"retainedEdits":[{
            "kind":"RETAINED_OPERATION","id":entry.id,"recordDigest":canonical::hash(&original).unwrap(),"target":"explanationText",
            "fragmentId":paragraph.id,"author":"Fixture <maintainer>","expectedOldValue":paragraph.text,"replacement":text
        }]}),
    );
    let bundle = authored["bundle"].as_str().unwrap();
    let old_bundle = frozen_tree(&f.bundle(bundle, ""));
    let old_data = support::read(f.bundle(bundle, "services/alpha.json"));
    let maintained: Operation = serde_json::from_value(old_data["operations"][0].clone()).unwrap();
    let protected = maintained
        .explanation
        .iter()
        .find(|p| p.id == paragraph.id)
        .unwrap();
    let mut selected = json!([{"id":"alpha-flow","service":"alpha","endpointDeclaration":declaration_v1,
        "workerDeclaration":declaration(service,"alpha.ProcessingLoop","runOnce"),"wiringDeclaration":declaration(service,"alpha.Composition","assemble"),
        "authoredParagraphs":[{"bundle":bundle,"operation":entry.id,"fragment":paragraph.id}],"expandSourceCalls":true}]);
    let old_selection = f.input("native-authored-v1-selection.json", &selected);
    let before = f.temp.path().join("native-authored-v1");
    render(&f, &v1, &old_selection, &before);
    let before_projection = support::read(before.join("projection.json"));
    assert_source_calls(&before, &before_projection);
    assert_eq!(
        before_projection["pages"][0]["authoredParagraphs"][0]["contextFreshness"],
        "CURRENT"
    );
    assert_eq!(
        before_projection["pages"][0]["authoredParagraphs"][0]["paragraph"],
        serde_json::to_value(protected).unwrap()
    );
    let before_files = files(&before);
    let (maintained_v1_work, maintained_v1_packet) = public_maintained_packet(
        &f,
        &v1,
        &declaration_v1,
        Some(&selected[0]["authoredParagraphs"][0]),
        "v1",
    );
    let before_context = &maintained_v1_packet["maintainedContext"];
    assert_eq!(before_context["contextFreshness"], "CURRENT");
    assert_eq!(
        before_context["paragraph"],
        serde_json::to_value(protected).unwrap()
    );
    assert_eq!(
        before_context["sourceRecords"],
        before_projection["pages"][0]["authoredParagraphs"][0]["sourceRecords"]
    );
    assert_eq!(
        before_context["paragraph"]["authorship"]["meaningReview"],
        "UNASSESSED"
    );
    assert_eq!(
        serde_json::to_value(
            work::load(&repo, &maintained_v1_work)
                .unwrap()
                .maintained_context
                .unwrap()
        )
        .unwrap(),
        *before_context
    );
    for event_id in &protected.event_ids {
        assert_eq!(
            before_context["anchors"][event_id],
            serde_json::to_value(
                maintained
                    .events
                    .iter()
                    .find(|e| &e.id == event_id)
                    .unwrap()
            )
            .unwrap()
        );
    }
    let changed = fs::read_to_string(&path)
        .unwrap()
        .replace(
            "body.name, body.eligible",
            "body.name + \" updated\", body.eligible",
        )
        .replace(
            "if (!current.enabled)",
            "if (!current.enabled || current.prefix == null)",
        )
        .replace("chosen.trim()", "chosen.toUpperCase()");
    fs::write(&path, changed).unwrap();
    commit(&alpha);
    let (v2, c2) = capture(&f);
    assert_ne!(v1, v2);
    selected[0]["endpointDeclaration"] = json!(declaration(
        &c2.services["alpha"],
        "alpha.DispatchEndpoint",
        "submit"
    ));
    selected[0]["workerDeclaration"] = json!(declaration(
        &c2.services["alpha"],
        "alpha.ProcessingLoop",
        "runOnce"
    ));
    selected[0]["wiringDeclaration"] = json!(declaration(
        &c2.services["alpha"],
        "alpha.Composition",
        "assemble"
    ));
    // A supported publication moves the live index. The exact original bundle is
    // deliberately retained in the native selection; no latest fallback is allowed.
    let moved = f.ok(&["docs", "render", "--snapshot", &v2, "--publish"]);
    assert_ne!(moved["bundle"], bundle);
    let input = f.input("native-authored-current-selection.json", &selected);
    let current = f.temp.path().join("native-authored-current");
    render(&f, &v2, &input, &current);
    let projection = support::read(current.join("projection.json"));
    assert_source_calls(&current, &projection);
    let page = &projection["pages"][0];
    let row = &page["authoredParagraphs"][0];
    assert_eq!(row["selection"], selected[0]["authoredParagraphs"][0]);
    assert_eq!(row["paragraph"], serde_json::to_value(protected).unwrap());
    assert_eq!(row["contextFreshness"], "STALE");
    assert_eq!(
        row["paragraph"]["authorship"]["meaningReview"],
        "UNASSESSED"
    );
    assert_eq!(row["paragraph"]["authorship"]["sourceSnapshot"], v1);
    let source_id = protected
        .source_ids
        .iter()
        .find(|id| {
            page["sources"][*id].is_object()
                && row["sourceRecords"][*id]["text"] != page["sources"][*id]["text"]
        })
        .expect("same logical SOURCE id has changed bytes");
    assert_eq!(
        row["sourceRecords"][source_id]["text"],
        c1.sources()[source_id].text
    );
    assert_eq!(
        page["sources"][source_id]["text"],
        c2.sources()[source_id].text
    );
    let current_worker = page["worker"].to_string();
    assert!(
        current_worker.contains("!current.enabled || current.prefix == null")
            && current_worker.contains("chosen.toUpperCase()")
    );
    let manifest = support::read(current.join("manifest.json"));
    assert_eq!(manifest["selectedAuthoredParagraphs"][0]["bundle"], bundle);
    assert_eq!(
        manifest["selectedAuthoredParagraphs"][0]["sourceSnapshot"],
        v1
    );
    let html = fs::read_to_string(current.join("alpha-flow-overview.html")).unwrap();
    let mdx = fs::read_to_string(current.join("alpha-flow-overview.mdx")).unwrap();
    assert!(
        html.contains("Fixture &lt;maintainer&gt;") && html.contains("Meaning review: UNASSESSED")
    );
    assert!(mdx.contains("&#123;queue&#125;") && mdx.contains("&#91;policy&#93;"));
    let appendix = fs::read_to_string(current.join("sources.html")).unwrap();
    assert!(
        appendix.contains("Original authored paragraph context")
            && appendix.contains("Logical SOURCE ID:")
    );
    let expected_files = files(&current);
    let root_v2 = selected[0]["endpointDeclaration"].as_str().unwrap();
    let (maintained_v2_work, maintained_v2_packet) = public_maintained_packet(
        &f,
        &v2,
        root_v2,
        Some(&selected[0]["authoredParagraphs"][0]),
        "v2-stale",
    );
    let stale_context = &maintained_v2_packet["maintainedContext"];
    assert_eq!(stale_context["contextFreshness"], "STALE");
    for key in [
        "paragraph",
        "root",
        "rootSources",
        "anchors",
        "sourceRecords",
        "dependencyRecords",
        "publicationDigest",
        "bindingsDigest",
        "operationDigest",
        "paragraphDigest",
        "contextDigest",
    ] {
        assert_eq!(stale_context[key], before_context[key], "historical {key}");
    }
    assert_ne!(
        maintained_v2_packet["packetDigest"],
        maintained_v1_packet["packetDigest"]
    );
    let reject_work = |selection: &Value, root: &str, name: &str, expected: &str| {
        let records_before = frozen_tree(&repo.path(".codeclew/work").unwrap());
        let input = f.input(&format!("reject-maintained-work-{name}.json"), &json!({"schema":"codeclew-documentation-work-request/1.0", "audience":"Synthetic negative maintained selection", "contextProfile":"process-graph-v1", "rootDeclaration":root, "question":"Explain current source with selected human context", "maintainedParagraph":selection}));
        let (code, response) = f.run(&[
            "docs",
            "work",
            "prepare",
            "--subject",
            "service:alpha",
            "--snapshot",
            &v2,
            "--input",
            input.to_str().unwrap(),
        ]);
        assert_ne!(code, 0, "{response}");
        assert!(response.to_string().contains(expected), "{response}");
        assert_eq!(
            records_before,
            frozen_tree(&repo.path(".codeclew/work").unwrap())
        );
        assert!(!repo.path(".codeclew/jobs").unwrap().exists());
        assert!(!repo.path("execution/accounts").unwrap().exists());
    };
    let reject = |selection: &Value, name: &str, expected: &str| {
        let input = f.input(&format!("reject-{name}.json"), selection);
        let out = f.temp.path().join(format!("reject-{name}"));
        let (code, response) = f.run(&[
            "docs",
            "pages",
            "render",
            "--snapshot",
            &v2,
            "--input",
            input.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ]);
        assert_ne!(code, 0, "{response}");
        assert!(response.to_string().contains(expected), "{response}");
        assert!(!out.exists());
    };
    let mut invalid = selected.clone();
    invalid[0]["authoredParagraphs"][0]["fragment"] = json!("missing");
    reject(&invalid, "missing-fragment", "missing or ambiguous");
    reject_work(
        &invalid[0]["authoredParagraphs"][0],
        root_v2,
        "missing-fragment",
        "missing or ambiguous",
    );
    invalid = selected.clone();
    invalid[0]["endpointDeclaration"] = invalid[0]["workerDeclaration"].clone();
    reject(&invalid, "unrelated-endpoint", "endpoint compiler symbol");
    reject_work(
        &selected[0]["authoredParagraphs"][0],
        invalid[0]["endpointDeclaration"].as_str().unwrap(),
        "unrelated-root",
        "exact discovered endpoint",
    );
    invalid = selected.clone();
    invalid[0]["authoredParagraphs"][0]["fragment"] = json!(original.explanation[1].id);
    reject(&invalid, "unauthored", "no declared user authorship");
    reject_work(
        &invalid[0]["authoredParagraphs"][0],
        root_v2,
        "unauthored",
        "no declared human authorship",
    );
    invalid = selected.clone();
    invalid[0]["authoredParagraphs"]
        .as_array_mut()
        .unwrap()
        .push(selected[0]["authoredParagraphs"][0].clone());
    reject(&invalid, "duplicate", "duplicate native authored");
    invalid = selected.clone();
    invalid[0]["authoredParagraphs"][0]["bundle"] = json!("0".repeat(64));
    reject(&invalid, "missing-bundle", "publication");
    reject_work(
        &invalid[0]["authoredParagraphs"][0],
        root_v2,
        "missing-bundle",
        "publication",
    );
    // Tampering with public frozen output bytes is detected by its manifest.
    let binding_file = f.bundle(bundle, "bindings.json");
    let binding_bytes = fs::read(&binding_file).unwrap();
    fs::write(&binding_file, b"{}").unwrap();
    reject(&selected, "damaged-bindings", "damaged or incomplete");
    reject_work(
        &selected[0]["authoredParagraphs"][0],
        root_v2,
        "damaged-bindings",
        "damaged or incomplete",
    );
    // Omission and null remain identical and do not read the damaged old bundle.
    let (legacy_id, legacy_packet) =
        public_maintained_packet(&f, &v2, root_v2, None, "legacy-omitted");
    let (null_id, null_packet) =
        public_maintained_packet(&f, &v2, root_v2, Some(&Value::Null), "legacy-null");
    assert_eq!(legacy_id, null_id);
    assert_eq!(
        canonical::bytes(&legacy_packet).unwrap(),
        canonical::bytes(&null_packet).unwrap()
    );
    assert!(legacy_packet.get("maintainedContext").is_none());
    assert!(
        serde_json::to_value(work::load(&repo, &legacy_id).unwrap())
            .unwrap()
            .get("maintainedContext")
            .is_none()
    );
    fs::write(&binding_file, &binding_bytes).unwrap();
    // A manifest-consistent forged map still cannot borrow another source digest.
    let fake = "f".repeat(64);
    copy_tree(&f.bundle(bundle, ""), &f.bundle(&fake, ""));
    let mut binding: Value = serde_json::from_slice(&binding_bytes).unwrap();
    let forged = {
        let p = &mut binding["narratives"]["service:alpha"]["operations"][0]["explanation"];
        let forged = p
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["id"] == paragraph.id)
            .unwrap();
        forged["authorship"]["sourceRefs"][source_id] = json!(format!("sha256:{}", "0".repeat(64)));
        forged.clone()
    };
    let key = format!("service:alpha/{}/{}", entry.id, paragraph.id);
    binding["fragments"][&key]["content"] = forged.clone();
    binding["fragments"][&key]["contentDigest"] =
        json!(canonical::hash(&binding["fragments"][&key]["content"]).unwrap());
    let forged_bytes = canonical::bytes(&binding).unwrap();
    fs::write(f.bundle(&fake, "bindings.json"), &forged_bytes).unwrap();
    let mut publication = support::read(f.bundle(&fake, "publication.json"));
    publication["id"] = json!(fake);
    publication["files"]["bindings.json"] = json!(canonical::hash_bytes(&forged_bytes));
    publication["explanationVersions"][format!("service:alpha/{}", entry.id)] =
        json!(canonical::hash(&binding["narratives"]["service:alpha"]["operations"][0]).unwrap());
    fs::write(
        f.bundle(&fake, "publication.json"),
        canonical::bytes(&publication).unwrap(),
    )
    .unwrap();
    invalid = selected.clone();
    invalid[0]["authoredParagraphs"][0]["bundle"] = json!(fake);
    reject(&invalid, "forged-refs", "pinned provenance");
    reject_work(
        &invalid[0]["authoredParagraphs"][0],
        root_v2,
        "forged-refs",
        "pinned provenance",
    );
    // Unselected paragraphs still count toward the original-Check budget. These
    // deliberately unavailable snapshots must reject before any Check is loaded.
    let budget_bundle = "e".repeat(64);
    copy_tree(&f.bundle(bundle, ""), &f.bundle(&budget_bundle, ""));
    let mut many: Value = serde_json::from_slice(&binding_bytes).unwrap();
    for i in 1..=9 {
        let mut fragment = many["fragments"][&key].clone();
        fragment["content"]["authorship"]["sourceSnapshot"] = json!(format!("{:064x}", i));
        many["fragments"][format!("unselected/op/fragment{i}")] = fragment;
    }
    let many_bytes = canonical::bytes(&many).unwrap();
    fs::write(f.bundle(&budget_bundle, "bindings.json"), &many_bytes).unwrap();
    let mut budget_manifest = support::read(f.bundle(&budget_bundle, "publication.json"));
    budget_manifest["id"] = json!(budget_bundle);
    budget_manifest["files"]["bindings.json"] = json!(canonical::hash_bytes(&many_bytes));
    fs::write(
        f.bundle(&budget_bundle, "publication.json"),
        canonical::bytes(&budget_manifest).unwrap(),
    )
    .unwrap();
    invalid = selected.clone();
    invalid[0]["authoredParagraphs"][0]["bundle"] = json!(budget_bundle);
    reject(
        &invalid,
        "unselected-context-budget",
        "eight original source contexts",
    );
    // A sparse oversized frozen file is rejected by metadata preflight, before
    // hashing its intentionally unusable content or resolving original contexts.
    let oversized = f.bundle(&budget_bundle, "oversized.txt");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(65 * 1024 * 1024)
        .unwrap();
    budget_manifest["files"]["oversized.txt"] = json!("sha256:not-read");
    fs::write(
        f.bundle(&budget_bundle, "publication.json"),
        canonical::bytes(&budget_manifest).unwrap(),
    )
    .unwrap();
    reject(&invalid, "frozen-input-budget", "input exceeds 64 MiB");
    let regeneration_work = public_work(
        &f,
        &v2,
        &entry.id,
        "Refresh native endpoint source fields before manual context selection",
    );
    complete_work(&f, &regeneration_work, Some(&entry.id));
    let generated_work = work::load(&repo, &regeneration_work).unwrap();
    let root = generated_work
        .handles
        .iter()
        .find(|(_, h)| h.kind == "ENTRYPOINT" && h.id == entry.id)
        .unwrap()
        .0;
    let mut steps = maintained
        .events
        .iter()
        .filter(|e| e.kind != "end")
        .map(|event| {
            let mut evidence = vec![root.clone()];
            evidence.extend(event.dependency_ids.iter().map(|id| {
                generated_work
                    .handles
                    .iter()
                    .find(|(_, h)| h.kind == "DEPENDENCY" && &h.id == id)
                    .unwrap()
                    .0
                    .clone()
            }));
            json!({"kind":"note","meaning":{"text":event.text,"evidence":evidence}})
        })
        .collect::<Vec<_>>();
    let previously_covered: std::collections::BTreeSet<_> = maintained
        .events
        .iter()
        .flat_map(|e| e.dependency_ids.iter())
        .collect();
    for (reference, handle) in &generated_work.handles {
        if handle.kind == "DEPENDENCY"
            && !previously_covered.contains(&handle.id)
            && generated_work
                .checked
                .dependencies
                .get(&handle.id)
                .is_some_and(|d| d.kind == "FLOW" && d.symbol == entry.symbol)
        {
            steps.push(json!({"kind":"note","meaning":{"text":"Additional current compiler source event.","evidence":[root,reference]}}));
        }
    }
    let generated_input = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{"entrypoint":root,"title":"Submit a task","summary":{"text":"Current compiler source context for this synthetic endpoint.","evidence":[root]},"steps":steps,"explanation":(0..14).map(|i|json!({"text":format!("Current synthetic paragraph {i}. {}","Synthetic maintained explanation λ☕. ".repeat(120)),"evidence":[root]})).collect::<Vec<_>>()}]});
    let generated_publication = submit_and_publish(&f, &regeneration_work, &generated_input);
    let generated_data = support::read(f.bundle(
        generated_publication["bundle"].as_str().unwrap(),
        "services/alpha.json",
    ));
    let current_operation: Operation =
        serde_json::from_value(generated_data["operations"][0].clone()).unwrap();
    let original = current_operation
        .explanation
        .iter()
        .find(|p| p.id == paragraph.id)
        .unwrap();
    let migration_work = public_work(
        &f,
        &v2,
        &entry.id,
        "Explicitly select current native endpoint paragraph context",
    );
    complete_work(&f, &migration_work, Some(&entry.id));
    let saved_migration = work::load(&repo, &migration_work).unwrap();
    let references = |kind: &str, ids: &[String]| {
        ids.iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|id| {
                saved_migration
                    .handles
                    .iter()
                    .find(|(_, h)| h.kind == kind && &h.id == id)
                    .unwrap()
                    .0
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    let source_refs = references("SOURCE", &original.source_ids);
    for reference in &source_refs {
        let mut cursor = None;
        loop {
            let mut request = json!({"schema":"codeclew-documentation-source-part-request/1.0","reference":reference});
            if let Some(c) = &cursor {
                request["cursor"] = json!(c);
            }
            let part_input = f.input("native-migration-source-part.json", &request);
            let output = f.run_raw(&[
                "docs",
                "work",
                "read-part",
                "--work",
                &migration_work,
                "--input",
                part_input.to_str().unwrap(),
            ]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(output.stdout.len() <= 49152);
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            cursor = value["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
    }
    let auth = original.authorship.as_ref().unwrap();
    let migration_input = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],"retainedEdits":[{"kind":"RETAINED_OPERATION","id":entry.id,"recordDigest":canonical::hash(&current_operation).unwrap(),"target":"explanationContext","fragmentId":original.id,"expectedParagraphDigest":canonical::hash(original).unwrap(),"expectedContextDigest":canonical::hash(&(&auth.source_snapshot,&auth.source_refs,&auth.dependency_refs,&auth.context_role,&auth.context_migration)).unwrap(),"contextEditor":"Native context <editor>","sourceReferences":source_refs,"dependencyReferences":references("DEPENDENCY",&original.dependency_ids),"anchors":original.event_ids.iter().map(|id|json!({"eventId":id,"expectedEventDigest":canonical::hash(current_operation.events.iter().find(|e|&e.id==id).unwrap()).unwrap()})).collect::<Vec<_>>()}]});
    let migration_publication = submit_and_publish(&f, &migration_work, &migration_input);
    let mut migrated_selection = selected.clone();
    migrated_selection[0]["authoredParagraphs"][0]["bundle"] =
        migration_publication["bundle"].clone();
    let migration_selection_input = f.input("native-migrated-selection.json", &migrated_selection);
    let native_migrated = f.temp.path().join("native-migrated-current");
    render(&f, &v2, &migration_selection_input, &native_migrated);
    let native_migrated_data = support::read(native_migrated.join("projection.json"));
    assert_source_calls(&native_migrated, &native_migrated_data);
    let migrated_row = &native_migrated_data["pages"][0]["authoredParagraphs"][0];
    assert_eq!(migrated_row["contextFreshness"], "CURRENT");
    assert_eq!(
        migrated_row["paragraph"]["authorship"]["sourceSnapshot"],
        v2
    );
    assert_eq!(
        migrated_row["paragraph"]["authorship"]["author"],
        auth.author
    );
    assert_eq!(
        migrated_row["paragraph"]["authorship"]["editDigest"],
        auth.edit_digest
    );
    assert_eq!(migrated_row["paragraph"]["text"], original.text);
    let migration = &migrated_row["paragraph"]["authorship"]["contextMigration"];
    assert_eq!(migration["editor"], "Native context <editor>");
    assert_eq!(migration["previousSourceSnapshot"], auth.source_snapshot);
    assert_eq!(migration["contextReview"], "UNASSESSED");
    for (id, pinned) in migrated_row["sourceRecords"].as_object().unwrap() {
        assert_eq!(pinned["textDigest"], c2.sources()[id].text_digest);
        assert_eq!(pinned["text"], c2.sources()[id].text);
    }
    let html = fs::read_to_string(native_migrated.join("alpha-flow-overview.html")).unwrap();
    assert!(
        html.contains("Declared text author: Fixture &lt;maintainer&gt;")
            && html.contains("Context selected by Native context &lt;editor&gt;")
            && html.contains("Context review: UNASSESSED")
            && html.contains("Explicitly selected source")
    );
    assert!(!html.contains("Originally linked source"));
    assert!(
        fs::read_to_string(native_migrated.join("sources.mdx"))
            .unwrap()
            .contains("Explicitly selected authored paragraph context")
    );
    let native_migrated_files = files(&native_migrated);
    let (maintained_migrated_work, maintained_migrated_packet) = public_maintained_packet(
        &f,
        &v2,
        root_v2,
        Some(&migrated_selection[0]["authoredParagraphs"][0]),
        "migrated-current",
    );
    let migrated_context = &maintained_migrated_packet["maintainedContext"];
    assert_eq!(migrated_context["contextFreshness"], "CURRENT");
    assert_eq!(migrated_context["paragraph"], migrated_row["paragraph"]);
    assert_eq!(
        migrated_context["sourceRecords"],
        migrated_row["sourceRecords"]
    );
    assert_eq!(
        migrated_context["paragraph"]["authorship"]["author"],
        "Fixture <maintainer>"
    );
    assert_eq!(
        migrated_context["paragraph"]["authorship"]["contextMigration"]["editor"],
        "Native context <editor>"
    );
    assert_eq!(
        migrated_context["paragraph"]["authorship"]["contextMigration"]["contextReview"],
        "UNASSESSED"
    );
    // A further real compiler capture makes the selected paragraph context stale.
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("body.name + \" updated\"", "body.name + \" newer\""),
    )
    .unwrap();
    commit(&alpha);
    let (v3, c3) = capture(&f);
    assert_ne!(v2, v3);
    let mut newer = migrated_selection.clone();
    newer[0]["endpointDeclaration"] = json!(declaration(
        &c3.services["alpha"],
        "alpha.DispatchEndpoint",
        "submit"
    ));
    newer[0]["workerDeclaration"] = json!(declaration(
        &c3.services["alpha"],
        "alpha.ProcessingLoop",
        "runOnce"
    ));
    newer[0]["wiringDeclaration"] = json!(declaration(
        &c3.services["alpha"],
        "alpha.Composition",
        "assemble"
    ));
    let newer_input = f.input("native-migrated-stale-selection.json", &newer);
    let native_stale = f.temp.path().join("native-migrated-stale");
    render(&f, &v3, &newer_input, &native_stale);
    let stale_projection = support::read(native_stale.join("projection.json"));
    assert_source_calls(&native_stale, &stale_projection);
    let stale_row = &stale_projection["pages"][0]["authoredParagraphs"][0];
    assert_eq!(stale_row["contextFreshness"], "STALE");
    assert_eq!(stale_row["paragraph"], migrated_row["paragraph"]);
    assert_eq!(stale_row["sourceRecords"], migrated_row["sourceRecords"]);
    let (maintained_newer_work, maintained_newer_packet) = public_maintained_packet(
        &f,
        &v3,
        newer[0]["endpointDeclaration"].as_str().unwrap(),
        Some(&newer[0]["authoredParagraphs"][0]),
        "migrated-stale",
    );
    assert_eq!(
        maintained_newer_packet["maintainedContext"]["contextFreshness"],
        "STALE"
    );
    assert_eq!(
        maintained_newer_packet["maintainedContext"]["paragraph"],
        migrated_context["paragraph"]
    );
    assert_eq!(
        maintained_newer_packet["maintainedContext"]["sourceRecords"],
        migrated_context["sourceRecords"]
    );
    assert_eq!(files(&native_migrated), native_migrated_files);
    assert_eq!(files(&before), before_files);
    fs::remove_dir_all(&alpha).unwrap();
    let original_offline = f.temp.path().join("native-authored-original-offline");
    render(&f, &v1, &old_selection, &original_offline);
    assert_source_calls(
        &original_offline,
        &support::read(original_offline.join("projection.json")),
    );
    assert_eq!(files(&original_offline), before_files);
    let offline = f.temp.path().join("native-authored-offline");
    render(&f, &v2, &input, &offline);
    let migrated_offline = f.temp.path().join("native-migrated-offline");
    render(&f, &v2, &migration_selection_input, &migrated_offline);
    assert_source_calls(&offline, &support::read(offline.join("projection.json")));
    assert_source_calls(
        &migrated_offline,
        &support::read(migrated_offline.join("projection.json")),
    );
    assert_eq!(files(&migrated_offline), native_migrated_files);
    assert_eq!(files(&offline), expected_files);
    for (id, packet) in [
        (&maintained_v1_work, &maintained_v1_packet),
        (&maintained_v2_work, &maintained_v2_packet),
        (&maintained_migrated_work, &maintained_migrated_packet),
        (&maintained_newer_work, &maintained_newer_packet),
    ] {
        let offline_packet = f.ok(&["docs", "work", "packet", "--work", id]);
        assert_eq!(
            canonical::bytes(&offline_packet).unwrap(),
            canonical::bytes(packet).unwrap()
        );
        assert_eq!(
            serde_json::to_value(work::load(&repo, id).unwrap().maintained_context.unwrap())
                .unwrap(),
            packet["maintainedContext"]
        );
    }
    assert!(!repo.path(".codeclew/jobs").unwrap().exists());
    assert!(!repo.path("execution/accounts").unwrap().exists());
    assert_eq!(frozen_tree(&f.bundle(bundle, "")), old_bundle);
    if let Some(destination) = std::env::var_os("CODECLEW_NATIVE_AUTHORED_TEST_ARTIFACTS") {
        let destination = Path::new(&destination);
        assert!(!destination.exists());
        fs::create_dir(destination).unwrap();
        copy_tree(&before, &destination.join("original"));
        copy_tree(&original_offline, &destination.join("original-offline"));
        copy_tree(&current, &destination.join("current"));
        copy_tree(&offline, &destination.join("offline"));
        copy_tree(&native_migrated, &destination.join("migrated-current"));
        copy_tree(&native_stale, &destination.join("migrated-stale"));
        copy_tree(&migrated_offline, &destination.join("migrated-offline"));
        for (name, packet) in [
            ("work-original", &maintained_v1_packet),
            ("work-stale", &maintained_v2_packet),
            ("work-migrated-current", &maintained_migrated_packet),
            ("work-migrated-stale", &maintained_newer_packet),
        ] {
            fs::write(
                destination.join(format!("{name}-packet.json")),
                canonical::bytes(packet).unwrap(),
            )
            .unwrap();
        }
        fs::write(destination.join("journey.json"),serde_json::to_vec_pretty(&json!({"schema":"codeclew-native-authored-journey/1.0","authority":"PUBLIC_MANUAL_UNASSESSED_NOT_SEMANTIC_REVIEW","originalSnapshot":v1,"currentSnapshot":v2,"authoredPublication":authored,"livePointerMovedPublication":moved,"selection":selected,"contextMigrationInput":migration_input,"contextMigrationPublication":migration_publication,"migratedSelection":migrated_selection,"subsequentSourceSnapshot":v3,"checkoutAndArchivesRemoved":true,"byteIdenticalOffline":true})).unwrap()).unwrap();
    }
}
