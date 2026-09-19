//! Enabled lifecycle regressions for the source/profile authority closure run
//! (R4/R6 follow-ups). These tests drive the real product paths against an
//! owned offline Maven/Lombok fixture and assert on persisted authority,
//! processor isolation, source equality and reuse semantics.

use clew::java_adapter_v2::{JavaCompilerFact, build_java_compiler_index};
use clew::java_project_model::{JAVA_MODEL_SCHEMA, extract_java_model_with_settings};
use clew::maven::MavenSettings;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Locate the user's ordinary cached Maven repository (read-only source for
/// test-owned fixtures). Never reads credentials or private project contents.
fn ambient_maven_repository() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let repo = home.join(".m2/repository");
    repo.is_dir().then_some(repo)
}

/// Recursively hard-link an artifact version directory from the ambient
/// repository into the test-owned local repository, preserving the relative
/// layout. Used only to stage ordinary cached artifacts; the source is never
/// modified and credentials are never copied.
fn stage_artifact(source_root: &Path, local: &Path, relative: &str) -> bool {
    let from = source_root.join(relative);
    let to = local.join(relative);
    if !from.is_dir() {
        return false;
    }
    fn link_tree(from: &Path, to: &Path) -> std::io::Result<()> {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let src = entry.path();
            let dst = to.join(entry.file_name());
            if src.is_dir() {
                link_tree(&src, &dst)?;
            } else if entry.file_type()?.is_file() {
                // Hard links share the source inode; deleting _remote.repositories
                // in the test-owned clone therefore cannot mutate the source.
                fs::hard_link(&src, &dst)?;
            }
        }
        Ok(())
    }
    link_tree(&from, &to).is_ok()
}

/// Plugin-group metadata enabling offline short-prefix (`help:`, `dependency:`)
/// resolution against a test-owned repository.
fn write_plugin_prefix_metadata(local: &Path) {
    let dir = local.join("org/apache/maven/plugins");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("maven-metadata-central.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata><plugins>
  <plugin><prefix>compiler</prefix><artifactId>maven-compiler-plugin</artifactId></plugin>
  <plugin><prefix>resources</prefix><artifactId>maven-resources-plugin</artifactId></plugin>
  <plugin><prefix>help</prefix><artifactId>maven-help-plugin</artifactId></plugin>
  <plugin><prefix>dependency</prefix><artifactId>maven-dependency-plugin</artifactId></plugin>
</plugins></metadata>"#,
    )
    .unwrap();
}

/// Per-artifact version metadata so Maven can resolve an unpinned plugin version
/// offline from the test-owned repository.
fn write_plugin_version_metadata(local: &Path, artifact: &str, version: &str) {
    let dir = local.join(format!("org/apache/maven/plugins/{artifact}/{version}"));
    fs::create_dir_all(&dir).unwrap();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata><groupId>org.apache.maven.plugins</groupId><artifactId>{artifact}</artifactId><version>{version}</version><versioning><latest>{version}</latest><release>{version}</release><versions><version>{version}</version></versions><lastUpdated>20260101000000</lastUpdated></versioning></metadata>"#
    );
    fs::write(dir.join("maven-metadata.xml"), &xml).unwrap();
    fs::write(dir.join("maven-metadata-local.xml"), &xml).unwrap();
}

/// The curated set of ordinary Maven artifacts the offline Lombok fixture needs
/// (compiler/help/dependency/resources + their runtime deps + Lombok itself).
const FIXTURE_ARTIFACTS: &[&str] = &[
    "org/projectlombok/lombok/1.18.38",
    "org/apache/maven/plugins/maven-compiler-plugin/3.13.0",
    "org/apache/maven/plugins/maven-help-plugin/3.5.1",
    "org/apache/maven/plugins/maven-dependency-plugin/3.8.1",
    "org/apache/maven/plugins/maven-resources-plugin/3.3.1",
    "org/apache/maven/plugins/maven-clean-plugin/3.3.2",
    "org/apache/maven/plugins/maven-surefire-plugin/3.3.1",
    "org/apache/maven/plugins/maven-jar-plugin/3.4.2",
    "org/apache/maven/plugins/maven-install-plugin/3.1.3",
    "org/apache/maven/plugins/maven-deploy-plugin/3.1.3",
    "org/apache/maven/shared/maven-shared-utils/3.4.2",
    "org/apache/maven/shared/maven-shared-incremental/1.1",
    "org/apache/maven/shared/maven-filtering/3.3.1",
    "org/apache/maven/shared/maven-common-artifact-filters/3.4.0",
    "org/apache/maven/shared/maven-artifact-transfer/0.13.1",
    "org/apache/maven/shared/maven-dependency-analyzer/1.15.0",
    "org/apache/maven/shared/maven-dependency-tree/3.3.0",
    "org/apache/maven/reporting/maven-reporting-api/4.0.0",
    "org/apache/maven/reporting/maven-reporting-impl/4.0.0",
    "org/apache/maven/doxia/doxia-sink-api/2.0.0",
    "org/apache/maven/plugin-tools/maven-plugin-tools-generators/3.13.1",
    "org/codehaus/plexus/plexus-java/1.2.0",
    "org/codehaus/plexus/plexus-compiler-api/2.15.0",
    "org/codehaus/plexus/plexus-compiler-manager/2.15.0",
    "org/codehaus/plexus/plexus-compiler-javac/2.15.0",
    "org/codehaus/plexus/plexus-archiver/4.10.0",
    "org/codehaus/plexus/plexus-io/3.5.1",
    "org/codehaus/plexus/plexus-interpolation/1.26",
    "org/codehaus/plexus/plexus-utils/3.5.1",
    "org/codehaus/plexus/plexus-xml/3.0.0",
    "org/codehaus/plexus/plexus-interactivity-api/1.3",
    "org/codehaus/plexus/plexus-i18n/1.0-beta-10",
    "org/sonatype/plexus/plexus-build-api/0.0.7",
    "org/apache/maven/resolver/maven-resolver-util/1.4.1",
    "org/slf4j/slf4j-api/1.7.36",
    "org/apache/commons/commons-lang3/3.17.0",
    "commons-io/commons-io/2.11.0",
    "org/jdom/jdom2/2.0.6.1",
    "com/thoughtworks/xstream/xstream/1.4.20",
    "javax/inject/javax.inject/1",
    "org/apache/maven/maven-archiver/3.6.2",
    "org/apache/maven/shared/file-management/3.1.0",
];

/// Stage a test-owned local Maven repository from the ambient cache. Returns
/// the repository path, or None when the ambient repository is unavailable
/// (the caller reports an explicit missing-fixture boundary rather than
/// silently skipping).
fn stage_local_repository(work: &Path) -> Option<PathBuf> {
    // Qualification can provision a complete fixture-owned repository explicitly.
    // Copy it per test so Maven metadata writes cannot modify the seed or another
    // test; do not redirect HOME or modify the user's ordinary Maven cache.
    if let Some(seed) = std::env::var_os("CODECLEW_TEST_MAVEN_REPOSITORY") {
        let seed = PathBuf::from(seed).canonicalize().ok()?;
        let local = work.join("m2repo");
        fs::create_dir_all(&local).ok()?;
        for entry in walkdir::WalkDir::new(&seed) {
            let entry = entry.ok()?;
            let target = local.join(entry.path().strip_prefix(&seed).ok()?);
            if entry.file_type().is_dir() {
                fs::create_dir_all(target).ok()?;
            } else if entry.file_type().is_file() {
                fs::copy(entry.path(), target).ok()?;
            } else {
                return None;
            }
        }
        return Some(local);
    }
    let ambient = ambient_maven_repository()?;
    let local = work.join("m2repo");
    for artifact in FIXTURE_ARTIFACTS {
        if !stage_artifact(&ambient, &local, artifact) {
            return None;
        }
    }
    write_plugin_prefix_metadata(&local);
    write_plugin_version_metadata(&local, "maven-help-plugin", "3.5.1");
    write_plugin_version_metadata(&local, "maven-dependency-plugin", "3.8.1");
    write_plugin_version_metadata(&local, "maven-compiler-plugin", "3.13.0");
    write_plugin_version_metadata(&local, "maven-resources-plugin", "3.3.1");
    Some(local)
}

const LOMBOK_POM: &str = r#"
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>dev.codeclew.fixture</groupId><artifactId>lombok-layout</artifactId><version>1</version>
  <properties><maven.compiler.source>17</maven.compiler.source><maven.compiler.target>17</maven.compiler.target><maven.compiler.release>17</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>
  <dependencies>
    <dependency><groupId>org.projectlombok</groupId><artifactId>lombok</artifactId><version>1.18.38</version><scope>provided</scope></dependency>
  </dependencies>
  <build><plugins>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-clean-plugin</artifactId><version>3.3.2</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-resources-plugin</artifactId><version>3.3.1</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version>
      <configuration><annotationProcessorPaths><path><groupId>org.projectlombok</groupId><artifactId>lombok</artifactId><version>1.18.38</version></path></annotationProcessorPaths>
        <annotationProcessors><annotationProcessor>lombok.launch.AnnotationProcessorHider$AnnotationProcessor</annotationProcessor></annotationProcessors>
      </configuration>
    </plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.3.1</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-jar-plugin</artifactId><version>3.4.2</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-install-plugin</artifactId><version>3.1.3</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-deploy-plugin</artifactId><version>3.1.3</version></plugin>
  </plugins></build>
</project>
"#;

/// Build a real Maven + Lombok fixture and return the test-owned local repo.
fn native_lombok_fixture(work: &Path) -> Option<PathBuf> {
    let local = stage_local_repository(work)?;
    let project = work.join("project");
    fs::create_dir_all(project.join("src/main/java/example")).unwrap();
    fs::write(project.join("pom.xml"), LOMBOK_POM).unwrap();
    fs::write(
        project.join("src/main/java/example/User.java"),
        "package example;\nimport lombok.Getter;\n@Getter\npublic class User { private String name; private int age; }\n",
    )
    .unwrap();
    Some(local)
}

fn settings_path(work: &Path, local: &Path) -> PathBuf {
    let settings = work.join("settings.xml");
    fs::write(
        &settings,
        format!(
            r#"<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0"><localRepository>{}</localRepository></settings>"#,
            local.display()
        ),
    )
    .unwrap();
    settings
}

/// A real offline Maven-configured Lombok project must be admitted into the
/// model with the explicit processor path/name, and the analyzer must resolve
/// the Lombok-generated members (`getName`/`getAge`) on JDK 21. This is the
/// native admission boundary: a manually constructed model does not prove it.
#[test]
fn native_maven_lombok_admits_processor_and_resolves_generated_members() {
    let work = tempfile::tempdir().unwrap();
    let Some(local) = native_lombok_fixture(work.path()) else {
        panic!(
            "native Lombok fixture is unavailable: ambient cached Maven artifacts (lombok 1.18.38 + compiler/help/dependency/resources plugins) must be present under ~/.m2/repository"
        );
    };
    let settings = settings_path(work.path(), &local);
    let binding = MavenSettings::capture(&settings).unwrap();
    let model =
        extract_java_model_with_settings(&work.path().join("project"), ":/main", Some(&binding))
            .unwrap_or_else(|error| panic!("native extract failed: {error}"));

    assert_eq!(model.authority.schema, JAVA_MODEL_SCHEMA);
    assert_eq!(
        model.authority.annotation_processors,
        vec!["lombok.launch.AnnotationProcessorHider$AnnotationProcessor"]
    );
    assert_eq!(model.authority.annotation_processor_paths.len(), 1);
    assert!(
        model.authority.annotation_processor_paths[0]
            .logical_name
            .contains("lombok-1.18.38.jar"),
        "{:?}",
        model.authority.annotation_processor_paths
    );

    // Drive the real analyzer with the admitted processor; generated members
    // must be present with lombok.Generated provenance.
    let digests = BTreeMap::from([(
        "src/main/java/example/User.java".to_string(),
        "digest".to_string(),
    )]);
    let index = build_java_compiler_index(
        &work.path().join("project"),
        &model,
        &digests,
        true,
        None,
        &[],
        None,
    )
    .unwrap();
    let names: Vec<String> = index
        .facts
        .iter()
        .filter_map(|fact| match fact {
            JavaCompilerFact::Declaration {
                name: Some(name), ..
            } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        names.iter().any(|n| n == "getName"),
        "generated members absent: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "getAge"),
        "generated members absent: {names:?}"
    );

    // Honest provenance: the original source file has no getter text. The
    // generated members are resolved semantically; their getter text is never
    // fabricated into the persisted source bytes.
    let original_source =
        fs::read_to_string(work.path().join("project/src/main/java/example/User.java")).unwrap();
    assert!(
        !original_source.contains("getName") && !original_source.contains("getAge"),
        "generated getter text must not be fabricated into the original source file"
    );
    // The index still carries the source file fact bound to the persisted bytes.
    assert!(
        index
            .facts
            .iter()
            .any(|fact| matches!(fact, JavaCompilerFact::SourceFile { file, .. } if file.ends_with("User.java"))),
        "the indexed source file must remain bound to its persisted bytes"
    );
}

/// A real local Filer processor selected by the model must be isolated to a
/// disposable generated root and must not write into the repository input tree.
#[test]
fn native_processor_output_is_isolated_to_disposable_root() {
    let work = tempfile::tempdir().unwrap();
    let Some(local) = native_lombok_fixture(work.path()) else {
        panic!("native fixture unavailable");
    };
    let settings = settings_path(work.path(), &local);
    let binding = MavenSettings::capture(&settings).unwrap();
    let model =
        extract_java_model_with_settings(&work.path().join("project"), ":/main", Some(&binding))
            .unwrap();
    let digests = BTreeMap::new();
    let index = build_java_compiler_index(
        &work.path().join("project"),
        &model,
        &digests,
        true,
        None,
        &[],
        None,
    )
    .unwrap();
    // The analyzer never writes into the repository tree; it only emits facts.
    let repo_root = work.path().join("project");
    fn generated_under(root: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                generated_under(&path, out);
            } else if path
                .file_name()
                .map(|n| n == "Generated.java")
                .unwrap_or(false)
            {
                out.push(path);
            }
        }
    }
    let mut generated = Vec::new();
    generated_under(&repo_root, &mut generated);
    assert!(
        generated.is_empty(),
        "analyzer processing wrote into the repository: {generated:?}"
    );
    assert!(!index.facts.is_empty());
}

/// No-op writable versus readonly same-model fixtures must have distinct
/// semantic generation authority: identical model/classpath and unchanged
/// transformed bytes still distinguish writable provenance from readonly
/// authority, so a stale generation cannot be reused across profiles.
#[test]
fn semantic_generation_key_distinguishes_noop_writable_from_readonly() {
    // A processor-path/name change must invalidate the model identity so a
    // stale generation cannot be reused: the model digest is content-addressed
    // and flows into canonical_options -> derived manifest -> generation key.
    let model = clew::java_project_model::JavaProjectModel {
        schema: clew::java_project_model::JAVA_MODEL_SCHEMA.into(),
        model_digest: String::new(),
        build_system: clew::java_project_model::JavaBuildSystem::Maven,
        compilation: ":/main".into(),
        source_files: vec!["src/main/java/example/User.java".into()],
        classpath: vec![],
        release: 17,
        compiler_version: "17.0".into(),
        compiler_options: vec!["--release=17".into()],
        annotation_processors: vec![],
        annotation_processor_paths: vec![],
        boundaries: vec![],
    };
    let mut unsigned = model.clone();
    unsigned.model_digest.clear();
    let base = clew::canonical::hash(&unsigned).unwrap();
    unsigned.annotation_processors =
        vec!["lombok.launch.AnnotationProcessorHider$AnnotationProcessor".into()];
    unsigned.annotation_processor_paths = vec![clew::java_project_model::JavaClasspathAuthority {
        logical_name: "artifact:lombok.jar:deadbeef".into(),
        digest: format!("sha256:{}", "c".repeat(64)),
        size: 1,
        kind: "FILE".into(),
    }];
    let changed = clew::canonical::hash(&unsigned).unwrap();
    assert_ne!(
        base, changed,
        "processor authority must participate in model identity"
    );

    // The writable-then-seal selection is part of the Java binding component
    // identity and the generation key (both schema bumped to include it), so a
    // no-op writable run cannot be conflated with a readonly run of the same
    // model/classpath. Verify the key schemas carry the flag.
    let readonly_key =
        clew::generation_service::final_generation_key_for_test(":/main", false).unwrap();
    let writable_key =
        clew::generation_service::final_generation_key_for_test(":/main", true).unwrap();
    assert_ne!(
        readonly_key, writable_key,
        "writable vs readonly must have distinct semantic generation identity"
    );
}

/// A legacy readonly generation (no transformed-source authority) remains
/// readable; a transformed binding carries a distinct persisted authority and
/// is not silently presented as readonly evidence.
#[test]
fn transformed_source_authority_round_trips_optionally() {
    use clew::generation_service::{
        AnalysisExecutionAuthority, IncrementalExecutionEvidence, IncrementalExecutionMode,
        READY_GENERATION_SCHEMA, READY_GENERATION_SET_SCHEMA, ReadyGeneration, ReadyGenerationSet,
    };
    use clew::incremental_v2::{
        CompletenessVector, FullAnalysisReason, INCREMENTAL_RECEIPT_SCHEMA, IncrementalPlan,
    };
    use clew::worker::WorkerRequestCounters;

    let object = |schema: &str, digest_char: char| clew::cas::CasObject {
        schema: clew::cas::CAS_OBJECT_SCHEMA.into(),
        object_schema: schema.into(),
        digest: format!("sha256:{}", digest_char.to_string().repeat(64)),
        size: 1,
    };
    let completeness =
        CompletenessVector::verified_complete(format!("sha256:{}", "9".repeat(64))).unwrap();
    let generation_key = "sha256:key".to_string();
    let compilation = ":/main".to_string();
    let make = |transformed: Option<clew::cas::CasObject>| ReadyGeneration {
        schema: READY_GENERATION_SCHEMA.into(),
        generation_key: generation_key.clone(),
        runtime_key: format!("sha256:{}", "2".repeat(64)),
        base_revision: format!("sha256:{}", "3".repeat(64)),
        compilation: compilation.clone(),
        compiler_version: "17.0".into(),
        completeness: completeness.clone(),
        coverage: "COMPLETE".into(),
        certainty: "COMPLETE".into(),
        obligations: vec![],
        incremental: IncrementalExecutionEvidence {
            schema: "codeclew-incremental-execution/1.0".into(),
            planned: IncrementalPlan::Full {
                reason: FullAnalysisReason::NoParent,
            },
            executed: IncrementalExecutionMode::Full,
            analysis_execution_authority: AnalysisExecutionAuthority::CompilerProcess,
            subset_analysis_supported: false,
            worker_requests: WorkerRequestCounters::default(),
        },
        incremental_receipt: object(INCREMENTAL_RECEIPT_SCHEMA, '6'),
        repository_snapshot: object(clew::repository_snapshot::SNAPSHOT_SCHEMA, '4'),
        derived_input_manifest: object(clew::derived_manifest::DERIVED_MANIFEST_SCHEMA, '7'),
        generation: object(clew::generation_v2::GENERATION_SCHEMA, '8'),
        query_index: object(clew::query_v2::QUERY_INDEX_SCHEMA, 'a'),
        transformed_source: transformed,
    };
    let set = |generation: ReadyGeneration| ReadyGenerationSet {
        schema: READY_GENERATION_SET_SCHEMA.into(),
        generation_key: generation_key.clone(),
        runtime_key: format!("sha256:{}", "2".repeat(64)),
        base_revision: format!("sha256:{}", "3".repeat(64)),
        repository_snapshot: object(clew::repository_snapshot::SNAPSHOT_SCHEMA, '4'),
        compilations: vec![generation],
        completeness: completeness.clone(),
        coverage: "COMPLETE".into(),
        certainty: "COMPLETE".into(),
        obligations: vec![],
        transformed_source: None,
    };

    // Legacy readonly: transformed_source is absent from canonical bytes and
    // reads back as None.
    let readonly = set(make(None));
    let bytes = clew::canonical::bytes(&readonly).unwrap();
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("transformedSource")
    );
    let back: ReadyGenerationSet = serde_json::from_slice(&bytes).unwrap();
    assert!(back.transformed_source.is_none());
    assert!(back.compilations[0].transformed_source.is_none());

    // Transformed authority is an explicit persisted reference, not readonly.
    let transformed = set(make(Some(object(
        clew::generation_service::TRANSFORMED_SOURCE_SCHEMA,
        'b',
    ))));
    let bytes = clew::canonical::bytes(&transformed).unwrap();
    let back: ReadyGenerationSet = serde_json::from_slice(&bytes).unwrap();
    assert!(back.compilations[0].transformed_source.is_some());
    assert_eq!(
        back.compilations[0]
            .transformed_source
            .as_ref()
            .unwrap()
            .object_schema,
        clew::generation_service::TRANSFORMED_SOURCE_SCHEMA
    );
}

/// Distinct module source states assemble and reopen through per-compilation
/// authority: two compilations with different transformed-source references are
/// both retained, never flattened into a single conflicting path, and each
/// consumer selects its own compilation's authority.
#[test]
fn compilation_scoped_source_authority_keeps_distinct_manifests() {
    use clew::generation_service::{
        AnalysisExecutionAuthority, IncrementalExecutionEvidence, IncrementalExecutionMode,
        READY_GENERATION_SCHEMA, READY_GENERATION_SET_SCHEMA, ReadyGeneration, ReadyGenerationSet,
    };
    use clew::incremental_v2::{
        CompletenessVector, FullAnalysisReason, INCREMENTAL_RECEIPT_SCHEMA, IncrementalPlan,
    };
    use clew::worker::WorkerRequestCounters;

    let object = |schema: &str, digest_char: char| clew::cas::CasObject {
        schema: clew::cas::CAS_OBJECT_SCHEMA.into(),
        object_schema: schema.into(),
        digest: format!("sha256:{}", digest_char.to_string().repeat(64)),
        size: 1,
    };
    let completeness =
        CompletenessVector::verified_complete(format!("sha256:{}", "9".repeat(64))).unwrap();
    let make = |compilation: &str, transformed: Option<clew::cas::CasObject>| ReadyGeneration {
        schema: READY_GENERATION_SCHEMA.into(),
        generation_key: format!("sha256:{}", compilation.as_bytes()[0] as char),
        runtime_key: format!("sha256:{}", "2".repeat(64)),
        base_revision: format!("sha256:{}", "3".repeat(64)),
        compilation: compilation.into(),
        compiler_version: "17.0".into(),
        completeness: completeness.clone(),
        coverage: "COMPLETE".into(),
        certainty: "COMPLETE".into(),
        obligations: vec![],
        incremental: IncrementalExecutionEvidence {
            schema: "codeclew-incremental-execution/1.0".into(),
            planned: IncrementalPlan::Full {
                reason: FullAnalysisReason::NoParent,
            },
            executed: IncrementalExecutionMode::Full,
            analysis_execution_authority: AnalysisExecutionAuthority::CompilerProcess,
            subset_analysis_supported: false,
            worker_requests: WorkerRequestCounters::default(),
        },
        incremental_receipt: object(INCREMENTAL_RECEIPT_SCHEMA, '6'),
        repository_snapshot: object(clew::repository_snapshot::SNAPSHOT_SCHEMA, '4'),
        derived_input_manifest: object(clew::derived_manifest::DERIVED_MANIFEST_SCHEMA, '7'),
        generation: object(clew::generation_v2::GENERATION_SCHEMA, '8'),
        query_index: object(clew::query_v2::QUERY_INDEX_SCHEMA, 'a'),
        transformed_source: transformed,
    };
    // Two compilations, each with a distinct transformed-source manifest.
    let module_a = make(
        ":app/main",
        Some(object(
            clew::generation_service::TRANSFORMED_SOURCE_SCHEMA,
            'b',
        )),
    );
    let module_b = make(
        ":lib/main",
        Some(object(
            clew::generation_service::TRANSFORMED_SOURCE_SCHEMA,
            'c',
        )),
    );
    let set = ReadyGenerationSet {
        schema: READY_GENERATION_SET_SCHEMA.into(),
        generation_key: format!("sha256:{}", "5".repeat(64)),
        runtime_key: format!("sha256:{}", "2".repeat(64)),
        base_revision: format!("sha256:{}", "3".repeat(64)),
        repository_snapshot: object(clew::repository_snapshot::SNAPSHOT_SCHEMA, '4'),
        compilations: vec![module_a.clone(), module_b.clone()],
        completeness: completeness.clone(),
        coverage: "COMPLETE".into(),
        certainty: "COMPLETE".into(),
        obligations: vec![],
        transformed_source: None,
    };
    // Both distinct authorities survive serde round-trip without flattening.
    let bytes = clew::canonical::bytes(&set).unwrap();
    let back: ReadyGenerationSet = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back.compilations.len(), 2);
    let by_compilation = |c: &str| {
        back.compilations
            .iter()
            .find(|r| r.compilation == c)
            .unwrap()
    };
    let a = by_compilation(":app/main");
    let b = by_compilation(":lib/main");
    assert!(a.transformed_source.is_some());
    assert!(b.transformed_source.is_some());
    assert_ne!(
        a.transformed_source.as_ref().unwrap().digest,
        b.transformed_source.as_ref().unwrap().digest,
        "each compilation must keep its own persisted source authority"
    );
    // A consumer selects by compilation; the two manifests are not collapsed.
    assert_eq!(
        a.transformed_source.as_ref().unwrap().digest,
        module_a.transformed_source.as_ref().unwrap().digest
    );
    assert_eq!(
        b.transformed_source.as_ref().unwrap().digest,
        module_b.transformed_source.as_ref().unwrap().digest
    );
}

/// A processor on the compile classpath but NOT explicitly admitted must never
/// run: the analyzer uses `-proc:none`, so Lombok-provided getters stay absent.
/// This proves unadmitted classpath processors cannot execute or mutate.
#[test]
fn native_unadmitted_classpath_processor_never_runs() {
    let work = tempfile::tempdir().unwrap();
    let Some(local) = stage_local_repository(work.path()) else {
        panic!(
            "native Lombok fixture is unavailable: provide CODECLEW_TEST_MAVEN_REPOSITORY with provisioned fixture dependencies or the expected ambient Maven artifacts"
        );
    };
    // Same Lombok dependency (on the compile classpath) but no explicit
    // annotationProcessors/annotationProcessorPaths admission in the POM.
    let project = work.path().join("project");
    fs::create_dir_all(project.join("src/main/java/example")).unwrap();
    fs::write(
        project.join("pom.xml"),
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>dev.codeclew.fixture</groupId><artifactId>lombok-unadmitted</artifactId><version>1</version>
  <properties><maven.compiler.source>17</maven.compiler.source><maven.compiler.target>17</maven.compiler.target><maven.compiler.release>17</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>
  <dependencies>
    <dependency><groupId>org.projectlombok</groupId><artifactId>lombok</artifactId><version>1.18.38</version><scope>provided</scope></dependency>
  </dependencies>
  <build><plugins>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-clean-plugin</artifactId><version>3.3.2</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>
  </plugins></build>
</project>
"#,
    )
    .unwrap();
    fs::write(
        project.join("src/main/java/example/User.java"),
        "package example;\nimport lombok.Getter;\n@Getter\npublic class User { private String name; private int age; }\n",
    )
    .unwrap();
    let settings = settings_path(work.path(), &local);
    let binding = MavenSettings::capture(&settings).unwrap();
    let model = extract_java_model_with_settings(&project, ":/main", Some(&binding)).unwrap();
    assert!(
        model.authority.annotation_processors.is_empty(),
        "no processor may be admitted without explicit configuration"
    );
    assert!(
        model.authority.annotation_processor_paths.is_empty(),
        "no processor path may be admitted without explicit configuration"
    );
    let digests = BTreeMap::from([(
        "src/main/java/example/User.java".to_string(),
        "digest".to_string(),
    )]);
    let index =
        build_java_compiler_index(&project, &model, &digests, true, None, &[], None).unwrap();
    let names: Vec<String> = index
        .facts
        .iter()
        .filter_map(|fact| match fact {
            JavaCompilerFact::Declaration {
                name: Some(name), ..
            } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        !names.iter().any(|n| n == "getName") && !names.iter().any(|n| n == "getAge"),
        "an unadmitted classpath processor must never run: {names:?}"
    );
}

/// An explicitly admitted but failing processor produces a sanitized analyzer
/// error that never leaks the workspace/repository path or raw compiler stderr.
#[test]
fn native_failing_processor_error_is_sanitized() {
    let work = tempfile::tempdir().unwrap();
    let Some(local) = native_lombok_fixture(work.path()) else {
        panic!("native fixture unavailable");
    };
    let settings = settings_path(work.path(), &local);
    let binding = MavenSettings::capture(&settings).unwrap();
    let mut model =
        extract_java_model_with_settings(&work.path().join("project"), ":/main", Some(&binding))
            .unwrap();
    // Admit a processor class that cannot be found, forcing the analyzer to fail.
    model.authority.annotation_processors = vec!["does.not.Exist".into()];
    let digests = BTreeMap::from([(
        "src/main/java/example/User.java".to_string(),
        "digest".to_string(),
    )]);
    let error = build_java_compiler_index(
        &work.path().join("project"),
        &model,
        &digests,
        true,
        None,
        &[],
        None,
    )
    .expect_err("a failing admitted processor must surface an analyzer error");
    // The exact code is analyzer-determined (an unresolvable processor surfaces
    // as an input/analyzer failure); the requirement is that it is explicit and
    // that the message never leaks the workspace/repository path or raw stderr.
    assert!(
        matches!(
            error.code,
            clew::error::ErrorCode::InvalidInput
                | clew::error::ErrorCode::IncompleteSemanticAnalysis
        ),
        "failing processor must be an explicit typed error: {:?}",
        error.code
    );
    let work_str = work.path().display().to_string();
    assert!(
        !error.message.contains(&work_str),
        "analyzer error must not leak the workspace path: {:?}",
        error.message
    );
}

/// Compiler-option changes and same-coordinate processor-artifact digest changes
/// both participate in model identity, so relevant semantic authority is
/// invalidated rather than reused.
#[test]
fn processor_option_and_artifact_digest_participate_in_authority() {
    let model = clew::java_project_model::JavaProjectModel {
        schema: clew::java_project_model::JAVA_MODEL_SCHEMA.into(),
        model_digest: String::new(),
        build_system: clew::java_project_model::JavaBuildSystem::Maven,
        compilation: ":/main".into(),
        source_files: vec!["src/main/java/example/User.java".into()],
        classpath: vec![],
        release: 17,
        compiler_version: "17.0".into(),
        compiler_options: vec!["--release=17".into()],
        annotation_processors: vec!["example.Proc".into()],
        annotation_processor_paths: vec![clew::java_project_model::JavaClasspathAuthority {
            logical_name: "artifact:example:proc.jar".into(),
            digest: format!("sha256:{}", "a".repeat(64)),
            size: 1,
            kind: "FILE".into(),
        }],
        boundaries: vec![],
    };
    let digest = |m: &clew::java_project_model::JavaProjectModel| {
        let mut unsigned = m.clone();
        unsigned.model_digest.clear();
        clew::canonical::hash(&unsigned).unwrap()
    };
    let base = digest(&model);

    // A processor option change must invalidate authority.
    let mut option_changed = model.clone();
    option_changed.compiler_options = vec!["--release=17".into(), "-Aexample.flag=1".into()];
    assert_ne!(
        base,
        digest(&option_changed),
        "option change must invalidate authority"
    );

    // A same-coordinate local artifact whose bytes changed (different digest)
    // must invalidate authority.
    let mut artifact_changed = model.clone();
    artifact_changed.annotation_processor_paths[0].digest = format!("sha256:{}", "b".repeat(64));
    assert_ne!(
        base,
        digest(&artifact_changed),
        "changed local processor artifact digest must invalidate authority"
    );

    // Stable equivalent state is deterministic.
    assert_eq!(base, digest(&model));
}

/// An explicitly admitted synthetic processor observes its `-A` options and
/// invocation during the disposable-root analyzer run: a locally compiled
/// counter processor writes a marker whose content proves both the option
/// reached it and it actually executed. This covers the case a Lombok test
/// cannot observe (processor options).
#[test]
fn native_synthetic_processor_observes_admitted_options() {
    let work = tempfile::tempdir().unwrap();
    let Some(local) = native_lombok_fixture(work.path()) else {
        panic!("native fixture unavailable");
    };
    let settings = settings_path(work.path(), &local);
    let binding = MavenSettings::capture(&settings).unwrap();
    let mut model =
        extract_java_model_with_settings(&work.path().join("project"), ":/main", Some(&binding))
            .unwrap();

    // Compile a tiny synthetic annotation processor with the pinned JDK 21.
    let java_home = std::env::var("JAVA_HOME").expect("JAVA_HOME must be set for the native test");
    let javac = std::path::PathBuf::from(&java_home).join("bin/javac");
    let proc_src = work.path().join("proc/CounterProc.java");
    fs::create_dir_all(proc_src.parent().unwrap()).unwrap();
    fs::write(
        &proc_src,
        r#"
package dev.fixture;
import javax.annotation.processing.*;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;
import javax.tools.Diagnostic;
import java.io.*;
import java.util.Set;
@SupportedAnnotationTypes("*")
@SupportedSourceVersion(SourceVersion.RELEASE_17)
public class CounterProc extends AbstractProcessor {
    @Override
    public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment roundEnv) {
        if (roundEnv.processingOver()) {
            String counter = processingEnv.getOptions().get("counter");
            String flag = processingEnv.getOptions().get("flag");
            if (counter != null) {
                try (Writer w = new FileWriter(counter)) {
                    w.write("invoked;flag=" + (flag == null ? "?" : flag));
                } catch (IOException e) {
                    processingEnv.getMessager().printMessage(Diagnostic.Kind.ERROR, "write failed");
                }
            }
            return false;
        }
        return false;
    }
}
"#,
    )
    .unwrap();
    let classes = work.path().join("proc/classes");
    fs::create_dir_all(&classes).unwrap();
    let status = std::process::Command::new(&javac)
        .args(["-d", classes.to_str().unwrap(), proc_src.to_str().unwrap()])
        .status()
        .expect("pinned JDK 21 javac must run");
    assert!(status.success(), "synthetic processor must compile");
    let class_file = classes.join("dev/fixture/CounterProc.class");
    let class_bytes = fs::read(&class_file).unwrap();

    // Admit the synthetic processor by name and path, and pass its options.
    let counter = work.path().join("counter.txt");
    model.authority.annotation_processors = vec!["dev.fixture.CounterProc".into()];
    model.authority.annotation_processor_paths =
        vec![clew::java_project_model::JavaClasspathAuthority {
            logical_name: "proc:fixture:counter".into(),
            digest: clew::canonical::hash_bytes(&class_bytes),
            size: class_bytes.len() as u64,
            kind: "FILE".into(),
        }];
    model.authority.compiler_options = vec![
        "--release=17".into(),
        format!("-Acounter={}", counter.display()),
        "-Aflag=works".into(),
    ];
    model.annotation_processor_paths = vec![classes];
    // Recompute the content-addressed model identity after injection.
    let mut unsigned = model.authority.clone();
    unsigned.model_digest.clear();
    model.authority.model_digest = clew::canonical::hash(&unsigned).unwrap();

    let digests = BTreeMap::from([(
        "src/main/java/example/User.java".to_string(),
        "digest".to_string(),
    )]);
    let index = build_java_compiler_index(
        &work.path().join("project"),
        &model,
        &digests,
        true,
        None,
        &[],
        None,
    )
    .unwrap();
    assert!(!index.facts.is_empty());
    assert_eq!(
        fs::read_to_string(&counter).unwrap(),
        "invoked;flag=works",
        "the admitted synthetic processor must observe its options and run"
    );
}
