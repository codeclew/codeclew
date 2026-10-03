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
