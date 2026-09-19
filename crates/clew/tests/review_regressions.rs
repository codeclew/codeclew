//! Compiler-backed regression coverage for the 2026-09-15 commit-and-cache
//! review (R1, R2, R7 containment run). Each test compiles synthetic Java with
//! the real JDK analyzer (JAVA_HOME) and asserts on the emitted facts, the
//! shared annotation registry, and Spring consumer interpretation.

use clew::java_adapter_v2::JAVA_ANALYZER_SOURCE;
use clew::spring_entrypoints::{annotation_registry, metadata_for_fact, with_annotation_registry};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Write fixtures, run the analyzer over them, and return the parsed stdout
/// facts as JSON values (plus the analyzer stderr for diagnostics).
fn run_analyzer(fixtures: &[(&str, &str)]) -> (Vec<Value>, String) {
    // A unique tempdir per invocation keeps the parallel analyzer tests from
    // clobbering each other's source fixtures (a shared per-process dir let
    // one test's Holder.java overwrite another's mid-analysis).
    let temp = tempfile::tempdir().expect("temporary analyzer workspace");
    let root = temp.path().join("src");
    std::fs::create_dir_all(&root).unwrap();
    let mut sources = Vec::new();
    for (relative, body) in fixtures {
        let file = root.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, body).unwrap();
        sources.push(relative.to_string());
    }
    let analyzer = temp.path().join("CodeclewJavaAnalyzer.java");
    std::fs::write(&analyzer, JAVA_ANALYZER_SOURCE).unwrap();
    let manifest = temp.path().join("sources.txt");
    std::fs::write(&manifest, sources.join("\n")).unwrap();
    let classpath = temp.path().join("classpath.txt");
    std::fs::write(&classpath, "").unwrap();
    let java: PathBuf = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("bin/java"))
        .unwrap_or_else(|| "java".into());
    let output = std::process::Command::new(&java)
        .arg("--source")
        .arg("17")
        .arg(&analyzer)
        .arg(&root)
        .arg(&manifest)
        .arg(&classpath)
        .arg("17")
        .arg("")
        .arg("")
        .output()
        .expect("failed to launch the JDK analyzer");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "analyzer failed (status {:?}): {stderr}",
        output.status.code()
    );
    let facts = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (facts, stderr)
}

/// Union of every definition across all ANNOTATION_REGISTRY shards.
fn registry_union(facts: &[Value]) -> BTreeMap<String, Value> {
    annotation_registry(facts.iter())
}

/// Find the DECLARATION fact whose symbol_identity ends with `suffix`.
fn declaration<'a>(facts: &'a [Value], suffix: &str) -> &'a Value {
    facts
        .iter()
        .find(|fact| {
            fact["kind"] == "DECLARATION"
                && fact["symbolIdentity"]
                    .as_str()
                    .is_some_and(|identity| identity.ends_with(suffix))
        })
        .unwrap_or_else(|| panic!("missing declaration ending with {suffix}"))
}

/// R1: a multi-shard registry must retain every definition (the previous
/// shard algorithm cleared maps retained by already-emitted shards).
#[test]
fn multi_shard_registry_preserves_every_definition() {
    let big = "v".repeat(8000);
    let mut src = String::from("package app;\nclass Holder { ");
    for i in 0..8 {
        src.push_str(&format!("@A{i} "));
    }
    src.push_str("void m() {} }\n");
    for i in 0..8 {
        src.push_str(&format!(
            "@interface A{i} {{ String value() default \"{big}\"; }}\n"
        ));
    }
    let (facts, _stderr) = run_analyzer(&[("app/Holder.java", &src)]);

    let shard_count = facts
        .iter()
        .filter(|fact| fact["kind"] == "ANNOTATION_REGISTRY")
        .count();
    assert!(
        shard_count >= 3,
        "expected at least 3 shards, got {shard_count}"
    );

    let union = registry_union(&facts);
    for i in 0..8 {
        let id = format!("app.A{i}");
        let definition = union.get(&id).unwrap_or_else(|| panic!("missing {id}"));
        let members = definition["members"].as_object().expect("members object");
        assert!(members.contains_key("value"), "{id} missing value member");
        assert_eq!(
            members["value"]["defaultValue"]["value"], big,
            "{id} default must survive sharding intact"
        );
    }
}

/// R2: a class implementing an annotated interface must retain the interface's
/// route prefix and the overridden method's mapping after registry reattachment.
#[test]
fn interface_base_mapping_preserves_route_prefix() {
    let stubs = [
        (
            "org/springframework/web/bind/annotation/GetMapping.java",
            "package org.springframework.web.bind.annotation;\npublic @interface GetMapping { String[] value() default {}; String[] path() default {}; }\n",
        ),
        (
            "org/springframework/web/bind/annotation/RequestMapping.java",
            "package org.springframework.web.bind.annotation;\npublic @interface RequestMapping { String[] value() default {}; String[] path() default {}; }\n",
        ),
        (
            "org/springframework/web/bind/annotation/RestController.java",
            "package org.springframework.web.bind.annotation;\npublic @interface RestController {}\n",
        ),
    ];
    let api = "package app;\nimport org.springframework.web.bind.annotation.*;\n@RequestMapping(\"/base\")\npublic interface Api { @GetMapping(\"/items\") String item(int id); }\n";
    let impl_body = "package app;\nimport org.springframework.web.bind.annotation.*;\n@RestController\npublic class Impl implements Api { @Override public String item(int id) { return \"x\"; } }\n";
    let mut fixtures = stubs.to_vec();
    fixtures.push(("app/Api.java", api));
    fixtures.push(("app/Impl.java", impl_body));
    let (facts, _stderr) = run_analyzer(&fixtures);

    let union = registry_union(&facts);
    let method = declaration(&facts, "Impl#item(I)Ljava/lang/String;");
    let payload = with_annotation_registry(method, &union).unwrap();
    let meta = metadata_for_fact(&payload, "JAVAC_RESOLVED_ANNOTATIONS")
        .unwrap()
        .unwrap();
    let meta = serde_json::to_value(meta).unwrap();
    let entry = serde_json::to_string(&meta["entries"][0]).unwrap();
    assert!(
        entry.contains("\"/base\""),
        "interface route prefix lost from hierarchy: {entry}"
    );
    assert!(
        entry.contains("\"/items\""),
        "overridden method mapping lost: {entry}"
    );
}

/// R7-adjacent: an individually oversized annotation definition is bounded
/// explicitly and must not be silently dropped or presented as COMPLETE.
#[test]
fn oversized_definition_is_bounded_not_complete() {
    let big = "w".repeat(62_000); // > DEFINITION_MEMBERS_BYTES (60 KiB), < string literal limit
    let src = format!(
        "package app;\nclass Holder {{ @Huge void m() {{}} }}\n@interface Huge {{ String value() default \"{big}\"; }}\n"
    );
    let (facts, _stderr) = run_analyzer(&[("app/Holder.java", &src)]);

    let union = registry_union(&facts);
    let definition = union.get("app.Huge").expect("Huge definition present");
    let bounded = definition["bounded"]
        .as_array()
        .expect("bounded marker must be an array");
    assert!(
        bounded
            .iter()
            .any(|reason| reason == "DEFINITION_MEMBER_BUDGET"),
        "oversized definition must carry an explicit bounded marker: {definition}"
    );

    // Reattaching and interpreting must surface an explicit boundary, so the
    // evidence is PARTIAL rather than falsely COMPLETE.
    let holder = declaration(&facts, "Holder#m()V");
    let payload = with_annotation_registry(holder, &union).unwrap();
    let meta = metadata_for_fact(&payload, "JAVAC_RESOLVED_ANNOTATIONS")
        .unwrap()
        .unwrap();
    let meta = serde_json::to_value(meta).unwrap();
    assert!(
        meta["boundaries"]
            .as_array()
            .is_some_and(|boundaries| boundaries
                .iter()
                .any(|code| code == "ANNOTATION_DEFINITION_BOUNDED")),
        "bounded definition must not claim complete extraction: {meta}"
    );
}

/// A minimal file-emitting annotation processor: it writes `Generated.java`
/// from `Filer` during annotation processing, exactly the sink that must be
/// disabled (default profile) or isolated (writable-then-seal profile).
const EMITTING_PROCESSOR_SOURCE: &str = r#"
package emitter;
import javax.annotation.processing.*;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;
import java.util.Set;
@SupportedAnnotationTypes("*")
public class Processor extends AbstractProcessor {
    public SourceVersion getSupportedSourceVersion() { return SourceVersion.latestSupported(); }
    public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment round) {
        if (round.processingOver()) return true;
        try {
            processingEnv.getFiler().createSourceFile("Generated")
                .openWriter().append("package x; public class Generated {}").close();
        } catch (Exception ignored) {}
        return true;
    }
}
"#;

/// Recursively collect relative paths of files named `Generated.java`.
fn generated_files(root: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else if path
                .file_name()
                .map(|n| n == "Generated.java")
                .unwrap_or(false)
            {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    walk(root, root, &mut found);
    found
}

/// R3: arbitrary annotation processors must not run in the default (read-only)
/// profile, and in the writable-then-seal profile their emitted files must be
/// isolated to a disposable output root, never written into the repository.
#[test]
fn annotation_processor_output_is_disabled_or_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("src");
    fs::create_dir_all(root.join("app")).unwrap();
    fs::write(
        root.join("app/App.java"),
        "package app;\n@Deprecated\npublic class App { @Deprecated void m() {} }\n",
    )
    .unwrap();
    fs::write(temp.path().join("sources.txt"), "app/App.java\n").unwrap();

    // Compile a file-emitting processor and place it (with its services
    // descriptor) on the analyzer classpath so javac can discover it.
    let procsrc = temp.path().join("procsrc");
    let procclasses = temp.path().join("procclasses");
    fs::create_dir_all(procsrc.join("emitter")).unwrap();
    fs::create_dir_all(procsrc.join("META-INF/services")).unwrap();
    fs::create_dir_all(procclasses.join("META-INF/services")).unwrap();
    fs::write(
        procsrc.join("emitter/Processor.java"),
        EMITTING_PROCESSOR_SOURCE,
    )
    .unwrap();
    fs::write(
        procsrc.join("META-INF/services/javax.annotation.processing.Processor"),
        "emitter.Processor\n",
    )
    .unwrap();
    let javac: PathBuf = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("bin/javac"))
        .unwrap_or_else(|| "javac".into());
    assert!(
        std::process::Command::new(&javac)
            .arg("-d")
            .arg(&procclasses)
            .arg(procsrc.join("emitter/Processor.java"))
            .status()
            .unwrap()
            .success(),
        "test processor must compile"
    );
    fs::copy(
        procsrc.join("META-INF/services/javax.annotation.processing.Processor"),
        procclasses.join("META-INF/services/javax.annotation.processing.Processor"),
    )
    .unwrap();
    fs::write(
        temp.path().join("classpath.txt"),
        procclasses.to_str().unwrap(),
    )
    .unwrap();

    let analyzer = temp.path().join("CodeclewJavaAnalyzer.java");
    fs::write(&analyzer, JAVA_ANALYZER_SOURCE).unwrap();
    let java: PathBuf = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("bin/java"))
        .unwrap_or_else(|| "java".into());
    let manifest = temp.path().join("sources.txt");
    let classpath = temp.path().join("classpath.txt");
    let run = |gen_dir: &str, processors: &str| {
        std::process::Command::new(&java)
            .arg("--source")
            .arg("17")
            .arg(&analyzer)
            .arg(&root)
            .arg(&manifest)
            .arg(&classpath)
            .arg("17")
            .arg(gen_dir)
            .arg(processors)
            .output()
            .expect("failed to launch the JDK analyzer")
    };

    // No admitted processors: the emitting processor never runs.
    let output = run("", "");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        generated_files(&root).is_empty(),
        "-proc:none must not run arbitrary processors into the repository"
    );

    // A generated root alone does not admit a processor: empty allowlist keeps
    // -proc:none, so the emitting processor (on the classpath) still does not run.
    let generated = temp.path().join("generated");
    fs::create_dir_all(&generated).unwrap();
    let output = run(generated.to_str().unwrap(), "");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        generated_files(&root).is_empty() && generated_files(&generated).is_empty(),
        "a processor not on the explicit allowlist must not run"
    );

    // Explicitly admitted processor in the writable profile: it runs, but its
    // emitted file is isolated to the disposable generated root, never the repo.
    let output = run(generated.to_str().unwrap(), "emitter.Processor");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        generated_files(&root).is_empty(),
        "an emitting processor must not write into the repository input tree"
    );
    assert!(
        generated_files(&generated)
            .iter()
            .any(|p| p == "Generated.java"),
        "an emitting processor must be isolated to the disposable generated root"
    );
}

/// Lifecycle contract for the writable-then-seal profile (R4/R6): the analyzer
/// runs under the pinned JDK 21 and emits coordinates relative to the exact
/// bytes it is given. When a synthetic transform inserts a line before a
/// declaration, the emitted `startLine` must reflect the transformed bytes, so
/// consumers must read the persisted transformed source (never slice the
/// original snapshot). This covers "line insertion survives with exact indexed
/// bytes" at the compiler boundary.
#[test]
fn writable_then_seal_coordinates_track_transformed_bytes_on_jdk21() {
    // Pin the runtime to the declared verification JDK 21.
    let java_home = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .expect("JAVA_HOME must point at the pinned JDK 21");
    let release = java_home.join("release").to_string_lossy().into_owned();
    let release_text = fs::read_to_string(&release).unwrap_or_default();
    assert!(
        release_text.contains("JAVA_VERSION=\"21"),
        "this regression must run on the pinned JDK 21 (got {release})"
    );

    // Original bytes: the declaration is on line 2.
    let original = "package app;\npublic class Service { void run() {} }\n";
    assert_eq!(
        original.lines().collect::<Vec<_>>().len(),
        2,
        "original has exactly two lines"
    );

    // Transformed bytes: a synthetic writable-then-seal build inserts a line
    // before the class, so the declaration moves to line 3.
    let transformed = "package app;\n\npublic class Service { void run() {} }\n";

    let (facts, _stderr) = run_analyzer(&[("app/Service.java", transformed)]);
    let service = declaration(&facts, "Service");
    assert_eq!(
        service["startLine"].as_u64(),
        Some(3),
        "transformed coordinate must track the inserted line: {service}"
    );
    assert!(
        service["startLine"].as_u64().unwrap_or(0) != original.lines().count() as u64,
        "coordinate must not silently match the original snapshot line"
    );
}
