//! Real compiler dependency metadata survives both portable docs lifecycles.
#![cfg(unix)]

#[path = "support/java_dependency.rs"]
#[allow(dead_code)]
mod java_dependency;
#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{
        analysis, cache,
        check::Check,
        model::{Observation, Service, ServiceEvidence},
        store::Repository,
        work,
    },
};
use serde_json::json;
use std::{fs, io::Write, path::Path, process::Command};
use support::{Fixture, commit, git};
use zip::write::{SimpleFileOptions, ZipWriter};

const GATEWAY_SOURCE: &str = "package external;\npublic interface Gateway {\n String lookup(String key);\n String lookup(int key);\n}\n";

fn write(root: &Path, path: &str, text: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, text).unwrap();
}

fn tool(name: &str) -> std::path::PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(|home| Path::new(&home).join("bin").join(name))
        .unwrap_or_else(|| name.into())
}

fn jar(root: &Path) {
    write(
        root,
        "dependency-src/external/Gateway.java",
        "package external; public interface Gateway { String lookup(String key); String lookup(int key); }\n",
    );
    let classes = root.join("dependency-classes");
    fs::create_dir(&classes).unwrap();
    let output = Command::new(tool("javac"))
        .args(["--release", "17", "-d"])
        .arg(&classes)
        .arg(root.join("dependency-src/external/Gateway.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    write(
        &classes,
        "META-INF/maven/example/gateway/pom.properties",
        "groupId=example\nartifactId=gateway\nversion=1.2.3\n",
    );
    let manifest = root.join("dependency-manifest.mf");
    fs::write(&manifest, "Manifest-Version: 1.0\nAutomatic-Module-Name: example.gateway\nImplementation-Version: 1.2.3\n\n").unwrap();
    fs::create_dir(root.join("libs")).unwrap();
    let output = Command::new(tool("jar"))
        .arg("--create")
        .arg("--file")
        .arg(root.join("libs/gateway.jar"))
        .arg("--manifest")
        .arg(&manifest)
        .arg("-C")
        .arg(&classes)
        .arg(".")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(root.join("dependency-src")).unwrap();
    fs::remove_dir_all(classes).unwrap();
    fs::remove_file(manifest).unwrap();
}

fn service(f: &Fixture, attached: bool) -> std::path::PathBuf {
    let root = f.temp.path().join("orders");
    fs::create_dir(&root).unwrap();
    jar(&root);
    if attached {
        let archive = fs::File::create(root.join("libs/gateway-1.2.3-sources.jar")).unwrap();
        let mut zip = ZipWriter::new(archive);
        zip.start_file("external/Gateway.java", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(GATEWAY_SOURCE.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    write(
        &root,
        "pom.xml",
        "<project><modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>orders</artifactId><version>1</version></project>\n",
    );
    write(&root, ".gitignore", "target/\n");
    write(
        &root,
        "src/main/java/org/springframework/stereotype/Controller.java",
        "package org.springframework.stereotype; public @interface Controller {}\n",
    );
    write(
        &root,
        "src/main/java/org/springframework/web/bind/annotation/RestController.java",
        "package org.springframework.web.bind.annotation; @org.springframework.stereotype.Controller public @interface RestController {}\n",
    );
    write(
        &root,
        "src/main/java/org/springframework/web/bind/annotation/RequestMapping.java",
        "package org.springframework.web.bind.annotation; public @interface RequestMapping { String[] value() default {}; String[] path() default {}; }\n",
    );
    write(
        &root,
        "src/main/java/example/OrderController.java",
        "package example;\nimport external.Gateway;\nimport org.springframework.web.bind.annotation.*;\n@RestController\npublic class OrderController {\n    private final Gateway gateway;\n    public OrderController(Gateway gateway) { this.gateway = gateway; }\n    @RequestMapping(path = \"/orders\")\n    public String find(String key) { return gateway.lookup(key); }\n}\n",
    );
    // Exercise normal Maven admission and the real javac provider without a
    // network dependency. The wrapper supplies the same Maven goal outputs
    // used by other native-provider integration fixtures.
    write(
        &root,
        "mvnw",
        r#"#!/bin/sh
set -eu
case "$*" in
  *help:effective-pom*)
    for argument in "$@"; do
      case "$argument" in -Doutput=*) effective_output=${argument#-Doutput=} ;; esac
    done
    module=$(pwd -P)
    cat > "$effective_output" <<EOF
<project><build><directory>$module/target</directory><sourceDirectory>$module/src/main/java</sourceDirectory><testSourceDirectory>$module/src/test/java</testSourceDirectory><outputDirectory>$module/target/classes</outputDirectory><testOutputDirectory>$module/target/test-classes</testOutputDirectory></build></project>
EOF
    ;;
  *dependency:build-classpath*)
    mkdir -p target/classes
    printf '%s\n' "$(pwd -P)/libs/gateway.jar" > target/codeclew-classpath.txt
    ;;
  *help:evaluate*) printf '17\n' ;;
  *) exit 25 ;;
esac
"#,
    );
    git(&root, &["init", "-q", "-b", "main"]);
    git(
        &root,
        &["remote", "add", "origin", "https://example.invalid/orders"],
    );
    commit(&root);
    let record = f.input("orders-service.json", &json!({
        "schema":"codeclew-documentation-service/1.0", "id":"orders", "title":"Orders",
        "repositoryId":"orders", "repository":"https://example.invalid/orders",
        "language":"java", "profile":"java-17plus-maven-read-only", "compilations":[":/main"], "targetRef":"main"
    }));
    add_service(f, &record);
    f.ok(&[
        "docs",
        "bind",
        "--service",
        "orders",
        "--repo",
        root.to_str().unwrap(),
    ]);
    root
}

fn add_service(f: &Fixture, record: &Path) {
    let digest = f.ok(&["docs", "service", "list"])["inputDigest"]
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
        &digest,
    ]);
}

fn admit(f: &Fixture, package: &Path) {
    let (code, inspected) = f.run_unrooted(&[
        "docs",
        "evidence",
        "inspect",
        "--input",
        package.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(inspected["outcome"], "CAPTURED", "{inspected}");
    let expectation = f.input("expectation.json", &json!({
        "schema":"codeclew-documentation-evidence-expectation/1.0", "service":"orders",
        "repositoryId":"orders", "serviceDigest":inspected["serviceDigest"],
        "revision":inspected["revision"], "manifestDigest":inspected["manifestDigest"], "sequence":1
    }));
    let digest = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "evidence",
        "expect",
        "--input",
        expectation.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ]);
    f.ok(&[
        "docs",
        "evidence",
        "import",
        "--input",
        package.to_str().unwrap(),
    ]);
}

fn target<'a>(evidence: &'a ServiceEvidence, source_digest: Option<&str>) -> &'a Observation {
    let targets: Vec<_> = evidence
        .observations
        .values()
        .filter(|o| {
            o.kind == "DEPENDENCY_TARGET"
                && o.normalized["qualifiedName"] == "external.Gateway"
                && o.normalized["declarationKind"] == "METHOD"
        })
        .collect();
    assert_eq!(targets.len(), 1, "only the invoked overload is retained");
    let target = targets[0];
    assert_eq!(target.normalized["name"], "lookup");
    assert_eq!(
        target.normalized["jvmDescriptor"],
        "(Ljava/lang/String;)Ljava/lang/String;"
    );
    assert_eq!(target.normalized["ownerIdentity"], "class:external.Gateway");
    assert_eq!(target.normalized["resolution"], "COMPILER_EXACT");
    source_binding(
        evidence,
        target,
        source_digest,
        "String lookup(String key);",
    );
    assert_eq!(target.normalized["bodyStatus"], "BODY_UNAVAILABLE");
    assert_eq!(
        target.normalized["runtimeImplementationStatus"],
        "UNRESOLVED"
    );
    assert_eq!(
        target.normalized["binaryMetadata"]["coordinates"],
        json!(["example:gateway:1.2.3"])
    );
    assert_eq!(
        target.normalized["binaryMetadata"]["automaticModuleName"],
        "example.gateway"
    );
    assert_eq!(target.normalized["binaryMetadata"]["version"], "1.2.3");
    assert_eq!(
        target.normalized["binaryMetadata"]["versionStatus"],
        "METADATA_EXACT"
    );
    let artifact = &target.normalized["binaryOrigin"]["artifact"];
    assert!(
        artifact["logicalName"]
            .as_str()
            .is_some_and(|name| !name.starts_with('/'))
    );
    assert!(
        artifact["digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:"))
    );
    assert!(artifact["size"].as_u64().is_some_and(|size| size > 0));
    let relation = evidence
        .observations
        .values()
        .find(|o| {
            o.kind == "CALL_RELATION"
                && o.normalized["targetIdentity"] == target.symbol
                && o.normalized["scope"] == target.normalized["scope"]
        })
        .unwrap();
    assert_eq!(relation.normalized["relationKind"], "CALLS");
    assert_eq!(
        relation.normalized["callSite"]["sourceStatus"],
        "SOURCE_RETAINED"
    );
    assert!(!relation.source_ids.is_empty());
    analysis::verify_evidence(evidence).unwrap();
    target
}

fn source_binding(
    evidence: &ServiceEvidence,
    target: &Observation,
    source_digest: Option<&str>,
    expected_text: &str,
) {
    assert_eq!(target.normalized["resolution"], "COMPILER_EXACT");
    assert_eq!(
        target.normalized["runtimeImplementationStatus"],
        "UNRESOLVED"
    );
    if let Some(source_digest) = source_digest {
        assert_eq!(target.normalized["sourceStatus"], "SOURCE_ATTACHED");
        assert_eq!(target.source_ids.len(), 1);
        let source = &evidence.sources[&target.source_ids[0]];
        let attachment = &target.normalized["dependencySource"];
        assert_eq!(attachment["sourceId"], source.id);
        assert_eq!(attachment["sourceArchive"]["digest"], source_digest);
        assert_eq!(
            attachment["sourceContentDigest"],
            canonical::hash_bytes(GATEWAY_SOURCE.as_bytes())
        );
        assert_eq!(
            attachment["matchingBasis"],
            "MAVEN_CLASSIFIER_PATH_AND_COMPILER_SIGNATURE"
        );
        assert!(attachment.get("text").is_none());
        assert_eq!(source.text, expected_text);
        assert_eq!(source.file, "external/Gateway.java");
        assert_eq!(source.authority, "EXACT_DEPENDENCY_SOURCE_ARCHIVE");
        assert_eq!(source.text_digest, attachment["textDigest"]);
        assert_eq!(
            source.text_digest,
            canonical::hash_bytes(expected_text.as_bytes())
        );
        assert!(source.url.is_none());
    } else {
        assert_eq!(target.normalized["sourceStatus"], "SOURCE_NOT_ATTACHED");
        assert!(target.source_ids.is_empty());
    }
}

fn dependency_type<'a>(
    evidence: &'a ServiceEvidence,
    source_digest: Option<&str>,
) -> &'a Observation {
    let target = evidence
        .observations
        .values()
        .find(|observation| {
            observation.kind == "DEPENDENCY_TARGET"
                && observation.symbol == "class:external.Gateway"
        })
        .unwrap();
    assert_eq!(target.normalized["declarationKind"], "INTERFACE");
    assert_eq!(target.normalized["jvmDescriptor"], "Lexternal/Gateway;");
    assert_eq!(target.normalized["bodyStatus"], "BODY_NOT_APPLICABLE");
    source_binding(
        evidence,
        target,
        source_digest,
        GATEWAY_SOURCE
            .strip_prefix("package external;\n")
            .unwrap()
            .trim_end_matches('\n'),
    );
    let constructor = evidence
        .observations
        .values()
        .find(|observation| {
            observation.kind == "SYMBOL"
                && observation.normalized["declarationKind"] == "CONSTRUCTOR"
                && observation.normalized["ownerIdentity"] == "class:example.OrderController"
        })
        .unwrap();
    assert!(evidence.observations.values().any(|relation| {
        relation.kind == "TYPE_RELATION"
            && relation.normalized["sourceIdentity"] == constructor.symbol
            && relation.normalized["targetIdentity"] == target.symbol
            && relation.normalized["scope"] == target.normalized["scope"]
            && relation.normalized["resolution"] == "COMPILER_EXACT"
            && relation.normalized["typeSite"]["sourceStatus"] == "SOURCE_RETAINED"
    }));
    target
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination.join(entry.file_name()));
        } else {
            assert!(entry.file_type().unwrap().is_file());
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "runs actual javac binary/source signature matching; requires the repository JDK"]
fn attached_dependency_source_projects_as_shared_exact_archive_evidence() {
    let mut fixture = java_dependency::Fixture::new();
    fixture.attach_sources("package example.integration;\npublic interface InventoryGateway {\n boolean reserve(String sku, int quantity);\n boolean reserve(String sku);\n}\n");
    let index = fixture.index();
    let service: Service = serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0", "id":"orders", "title":"Orders",
        "repositoryId":"orders", "repository":"https://example.invalid/orders",
        "language":"java", "profile":"java-17plus-maven-read-only", "compilations":[":/main"], "targetRef":"main"
    })).unwrap();
    let facts = index
        .facts
        .iter()
        .map(|fact| {
            let mut value = serde_json::to_value(fact).unwrap();
            value["scope"] = json!({"compilation":":/main"});
            let binding = canonical::hash(&value).unwrap();
            (value, binding)
        })
        .collect();
    let evidence = analysis::project(
        &service,
        &"a".repeat(40),
        &canonical::hash(&service).unwrap(),
        "DEVELOPMENT",
        "FULL",
        facts,
        &std::collections::BTreeMap::from([(
            java_dependency::SERVICE_FILE.into(),
            java_dependency::SERVICE_SOURCE.into(),
        )]),
        false,
    )
    .unwrap();
    analysis::verify_evidence(&evidence).unwrap();
    let target = evidence
        .observations
        .values()
        .find(|row| row.kind == "DEPENDENCY_TARGET" && row.symbol == java_dependency::TARGET)
        .unwrap();
    assert_eq!(target.normalized["sourceStatus"], "SOURCE_ATTACHED");
    assert_eq!(target.normalized["bodyStatus"], "BODY_UNAVAILABLE");
    assert_eq!(
        target.normalized["runtimeImplementationStatus"],
        "UNRESOLVED"
    );
    assert!(target.normalized["dependencySource"].get("text").is_none());
    assert_eq!(target.source_ids.len(), 1);
    let source = &evidence.sources[&target.source_ids[0]];
    assert_eq!(source.text, "boolean reserve(String sku, int quantity);");
    assert_eq!(source.authority, "EXACT_DEPENDENCY_SOURCE_ARCHIVE");
    assert_eq!(source.file, "example/integration/InventoryGateway.java");
    assert_eq!(
        source.text_digest,
        target.normalized["dependencySource"]["textDigest"]
    );
    assert!(source.url.is_none());
    let docs = tempfile::tempdir().unwrap();
    Repository::init(docs.path(), "Dependency evidence").unwrap();
    let repo = Repository::open(docs.path()).unwrap();
    let capture = cache::store_capture(&repo, &evidence).unwrap();
    let objects = cache::owned_digests(&repo, 100_000).unwrap();
    let repeated = cache::store_capture(&repo, &evidence).unwrap();
    assert_eq!(capture.sources.digest, repeated.sources.digest);
    assert_eq!(capture.observations.digest, repeated.observations.digest);
    assert_eq!(objects, cache::owned_digests(&repo, 100_000).unwrap());
    let reopened = cache::load_capture(&repo, &capture).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.observations).unwrap(),
        serde_json::to_value(evidence.observations).unwrap()
    );
    assert_eq!(
        serde_json::to_value(reopened.sources).unwrap(),
        serde_json::to_value(evidence.sources).unwrap()
    );
}

#[test]
#[ignore = "runs real admitted Maven/javac capture; requires the repository JDK"]
fn external_overload_metadata_survives_package_and_capture_export() {
    portable_dependency_capture(false);
}

#[test]
#[ignore = "runs real admitted Maven/javac source attachment capture; requires the repository JDK"]
fn attached_source_survives_native_package_and_capture_export() {
    portable_dependency_capture(true);
}

fn portable_dependency_capture(attached: bool) {
    let source = Fixture::new();
    let checkout = service(&source, attached);
    let source_digest = attached.then(|| {
        canonical::hash_bytes(&fs::read(checkout.join("libs/gateway-1.2.3-sources.jar")).unwrap())
    });
    let package = source.temp.path().join("package");
    let output = source.run_raw_with_path(
        &[
            "docs",
            "evidence",
            "capture",
            "--service",
            "orders",
            "--output",
            package.to_str().unwrap(),
        ],
        source.tools_dir(),
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // A native Check and its native capture envelope must share the same
    // authority. Package selection adds coordinator influence and therefore
    // belongs to the separate destination, not this export's source Check.
    let checked_output = source.run_raw_with_path(&["docs", "check"], source.tools_dir());
    assert!(
        matches!(checked_output.status.code(), Some(0 | 3 | 4)),
        "{}\n{}",
        String::from_utf8_lossy(&checked_output.stdout),
        String::from_utf8_lossy(&checked_output.stderr)
    );
    let checked = Check::load(
        &Repository::open(&source.docs).unwrap(),
        &source.docs.join(".codeclew/cache/latest-check.json"),
    )
    .unwrap();
    let expected = target(&checked.services["orders"], source_digest.as_deref()).clone();
    let expected_type =
        dependency_type(&checked.services["orders"], source_digest.as_deref()).clone();
    let expected_sources: std::collections::BTreeMap<_, _> = expected
        .source_ids
        .iter()
        .chain(&expected_type.source_ids)
        .map(|id| (id.clone(), checked.services["orders"].sources[id].clone()))
        .collect();
    let jar_bytes = fs::read(checkout.join("libs/gateway.jar")).unwrap();
    assert_eq!(
        expected.normalized["binaryOrigin"]["artifact"]["digest"],
        canonical::hash_bytes(&jar_bytes)
    );
    assert_eq!(
        expected.normalized["binaryOrigin"]["artifact"]["size"],
        jar_bytes.len() as u64
    );
    assert_eq!(
        expected.normalized["binaryOrigin"]["artifact"]["kind"],
        "FILE"
    );
    let source_repo = Repository::open(&source.docs).unwrap();
    let snapshot = checked.save_snapshot(&source_repo).unwrap();
    let owned_before = cache::owned_digests(&source_repo, 100_000).unwrap();
    let repeated_snapshot = checked.save_snapshot(&source_repo).unwrap();
    assert_eq!(snapshot, repeated_snapshot);
    assert_eq!(
        owned_before,
        cache::owned_digests(&source_repo, 100_000).unwrap()
    );

    let request = serde_json::from_value(json!({
        "schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers",
        "entrypoint":checked.services["orders"].entrypoints[0].id,
        "contextProfile":"endpoint-context-v3", "maxItems":100, "maxBytes":49152
    }))
    .unwrap();
    let mut page = work::prepare_with_snapshot(
        &source_repo,
        "service:orders".into(),
        request,
        Some(&snapshot),
    )
    .unwrap();
    let mut rows = Vec::new();
    loop {
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        let Some(cursor) = page["nextCursor"].as_str().map(str::to_owned) else {
            break;
        };
        page = work::read(
            &source_repo,
            page["work"].as_str().unwrap(),
            work::Selection {
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "DEPENDENCY" && row["id"] == expected.id)
    );
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "DEPENDENCY" && row["id"] == expected_type.id)
    );
    assert_eq!(
        rows.iter()
            .filter(|row| {
                row["kind"] == "SOURCE"
                    && row["record"]["file"]
                        .as_str()
                        .is_some_and(|file| file.contains("external/Gateway"))
            })
            .count(),
        if attached { 2 } else { 0 }
    );

    let destination = Fixture::new();
    let record = source.input(
        "portable-service.json",
        &serde_json::to_value(&source_repo.services().unwrap()["orders"]).unwrap(),
    );
    add_service(&destination, &record);
    // Remove the source checkout and both archives before package inspection
    // or admission. Offline consumption must use only the portable package.
    fs::remove_dir_all(&checkout).unwrap();
    assert!(!checkout.exists());
    admit(&destination, &package);
    let imported = destination.checked();
    for (id, source) in &expected_sources {
        assert_eq!(
            serde_json::to_value(&imported.services["orders"].sources[id]).unwrap(),
            serde_json::to_value(source).unwrap()
        );
    }
    assert_eq!(
        serde_json::to_value(dependency_type(
            &imported.services["orders"],
            source_digest.as_deref()
        ))
        .unwrap(),
        serde_json::to_value(&expected_type).unwrap()
    );
    assert_eq!(
        serde_json::to_value(target(
            &imported.services["orders"],
            source_digest.as_deref()
        ))
        .unwrap(),
        serde_json::to_value(&expected).unwrap()
    );

    let captures: Vec<_> = fs::read_dir(source.docs.join(".codeclew/cache"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("orders-")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .collect();
    assert_eq!(captures.len(), 1);
    let manifest: cache::CaptureManifest =
        serde_json::from_slice(&fs::read(&captures[0]).unwrap()).unwrap();
    assert_eq!(manifest.cacheability, "NON_CACHEABLE");
    assert_eq!(
        serde_json::to_value(target(
            &cache::load_capture(&source_repo, &manifest).unwrap(),
            source_digest.as_deref()
        ))
        .unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let export = source.temp.path().join("export");
    drop(source_repo);
    copy_tree(&source.docs.join(".codeclew/cache"), &export.join("cache"));
    let export = export.canonicalize().unwrap();
    let mut recovered = Fixture::new();
    recovered.docs = recovered.docs.canonicalize().unwrap();
    let result = recovered.ok(&[
        "docs",
        "snapshot",
        "recover",
        "--from-export",
        export.to_str().unwrap(),
        "--source-snapshot",
        &snapshot,
        "--capture",
        captures[0].file_name().unwrap().to_str().unwrap(),
    ]);
    let reopened = Check::load_snapshot(
        &Repository::open(&recovered.docs).unwrap(),
        result["recoveredSnapshot"].as_str().unwrap(),
    )
    .unwrap();
    for (id, source) in &expected_sources {
        assert_eq!(
            serde_json::to_value(&reopened.services["orders"].sources[id]).unwrap(),
            serde_json::to_value(source).unwrap()
        );
    }
    assert_eq!(
        serde_json::to_value(target(
            &reopened.services["orders"],
            source_digest.as_deref()
        ))
        .unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(
        reopened.source_authorities()["orders"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    assert_eq!(
        serde_json::to_value(dependency_type(
            &reopened.services["orders"],
            source_digest.as_deref()
        ))
        .unwrap(),
        serde_json::to_value(&expected_type).unwrap()
    );
    assert_eq!(
        expected.digest,
        canonical::hash(&expected.normalized).unwrap()
    );
}
