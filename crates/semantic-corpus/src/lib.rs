use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod e04;
pub mod e04_authorization;
pub mod e04_hidden_verification;
pub mod population;
pub mod product_coverage;

pub const GENERATOR_VERSION: &str = "semantic-corpus/0.2";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum TaskFamily {
    Smoke,
}

impl fmt::Display for TaskFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Smoke => formatter.write_str("smoke"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum BuildSystem {
    Gradle,
    Maven,
}

impl fmt::Display for BuildSystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gradle => formatter.write_str("gradle"),
            Self::Maven => formatter.write_str("maven"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskVariant {
    Positive,
    Ambiguous,
    MustRefuse,
}

impl fmt::Display for TaskVariant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Positive => formatter.write_str("positive"),
            Self::Ambiguous => formatter.write_str("ambiguous"),
            Self::MustRefuse => formatter.write_str("must-refuse"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepositoryLayout {
    Flat,
    Module,
}

impl fmt::Display for RepositoryLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flat => formatter.write_str("flat"),
            Self::Module => formatter.write_str("module"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GenerateOptions {
    pub seed: u64,
    pub family: TaskFamily,
    pub build_system: BuildSystem,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PublicTaskManifest {
    pub schema: String,
    pub generator_version: String,
    pub task_id: String,
    pub family: TaskFamily,
    pub variant: TaskVariant,
    pub build_system: BuildSystem,
    pub layout: RepositoryLayout,
    pub kotlin_version: String,
    pub task: String,
    pub repository: String,
    pub source_snapshot_sha256: String,
    pub build_command: Vec<String>,
    pub controller_manifest_commitment: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct HiddenManifest {
    pub schema: String,
    pub generator_version: String,
    pub task_id: String,
    pub generation_seed: u64,
    pub variant: TaskVariant,
    pub layout: RepositoryLayout,
    pub public_manifest_sha256: String,
    pub expected_obligations: Vec<String>,
    pub acceptable_design_classes: Vec<String>,
    pub hidden_tests: Vec<String>,
    pub hidden_artifacts: BTreeMap<String, String>,
    pub refusal_reasons: Vec<String>,
    pub oracle: HiddenOracle,
    pub commitment: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct HiddenOracle {
    pub destination: Option<String>,
    pub expected_route: Option<String>,
    pub required_worker_outcome: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedCorpus {
    pub root: PathBuf,
    pub agent_dir: PathBuf,
    pub controller_dir: PathBuf,
    pub public_manifest: PublicTaskManifest,
    pub hidden_manifest: HiddenManifest,
}

#[derive(Debug)]
struct Vocabulary {
    package_name: String,
    project_name: String,
    planner_name: String,
    origin: String,
    destination: String,
    module_name: String,
    decoy_name: String,
}

#[derive(Debug)]
struct GeneratedFile {
    relative_path: String,
    content: String,
}

/// Generate one isolated corpus task. The output directory must be absent or empty.
pub fn generate(options: &GenerateOptions) -> Result<GeneratedCorpus> {
    generate_with_variant(options, None)
}

/// E04 uses the frozen slot variant rather than deriving it from the seed.
/// This remains corpus generation only; it does not invoke a model or binder.
pub fn generate_with_variant(
    options: &GenerateOptions,
    frozen_variant: Option<TaskVariant>,
) -> Result<GeneratedCorpus> {
    ensure_empty_output(&options.output)?;

    let vocabulary = vocabulary(options.seed, options.family, options.build_system);
    let layout = repository_layout(options.seed, options.family, options.build_system);
    let variant = frozen_variant
        .unwrap_or_else(|| task_variant(options.seed, options.family, options.build_system));
    let repository_files = repository_files(options.build_system, layout, &vocabulary);
    let source_snapshot_sha256 = hash_files(&repository_files);
    let task_id = format!("{}-{}", options.family, &source_snapshot_sha256[..16]);
    let kotlin_version = "2.1.21";

    let mut public_manifest = PublicTaskManifest {
        schema: "semantic-corpus-public-task/0.1".to_owned(),
        generator_version: GENERATOR_VERSION.to_owned(),
        task_id: task_id.clone(),
        family: options.family,
        variant,
        build_system: options.build_system,
        layout,
        kotlin_version: kotlin_version.to_owned(),
        task: public_task(variant, &vocabulary),
        repository: "repository".to_owned(),
        source_snapshot_sha256,
        build_command: match options.build_system {
            BuildSystem::Gradle => vec!["gradle".to_owned(), "test".to_owned()],
            BuildSystem::Maven => vec!["mvn".to_owned(), "test".to_owned()],
        },
        controller_manifest_commitment: String::new(),
    };

    let hidden_test_path = format!(
        "hidden-tests/{}/{}HiddenTest.kt",
        vocabulary.package_name.replace('.', "/"),
        vocabulary.planner_name
    );
    let expected_route = format!("{} -> {}", vocabulary.origin, vocabulary.destination);
    let (expected_obligations, acceptable_design_classes, hidden_tests, refusal_reasons, oracle) =
        hidden_variant_data(variant, &hidden_test_path, &vocabulary, expected_route);
    let hidden_artifacts = if variant == TaskVariant::Positive {
        BTreeMap::from([(
            hidden_test_path.clone(),
            sha256_hex(hidden_test(&vocabulary).as_bytes()),
        )])
    } else {
        BTreeMap::new()
    };
    let mut hidden_manifest = HiddenManifest {
        schema: "semantic-corpus-controller-manifest/0.1".to_owned(),
        generator_version: GENERATOR_VERSION.to_owned(),
        task_id,
        generation_seed: options.seed,
        variant,
        layout,
        public_manifest_sha256: String::new(),
        expected_obligations,
        acceptable_design_classes,
        hidden_tests,
        hidden_artifacts,
        refusal_reasons,
        oracle,
        commitment: String::new(),
    };
    hidden_manifest.commitment = hidden_manifest_commitment(&hidden_manifest)?;
    public_manifest.controller_manifest_commitment = hidden_manifest.commitment.clone();
    let public_json = canonical_json(&public_manifest)?;
    hidden_manifest.public_manifest_sha256 = sha256_hex(public_json.as_bytes());
    let hidden_json = canonical_json(&hidden_manifest)?;

    let agent_dir = options.output.join("agent");
    let repository_dir = agent_dir.join("repository");
    let controller_dir = options.output.join("controller");
    for file in &repository_files {
        write_file(&repository_dir.join(&file.relative_path), &file.content)?;
    }
    write_file(&agent_dir.join("task-manifest.json"), &public_json)?;
    write_file(&controller_dir.join("manifest.json"), &hidden_json)?;
    if variant == TaskVariant::Positive {
        write_file(
            &controller_dir.join(hidden_test_path),
            &hidden_test(&vocabulary),
        )?;
    }

    Ok(GeneratedCorpus {
        root: options.output.clone(),
        agent_dir,
        controller_dir,
        public_manifest,
        hidden_manifest,
    })
}

fn ensure_empty_output(output: &Path) -> Result<()> {
    if output.exists() {
        if !output.is_dir() {
            bail!("output path is not a directory: {}", output.display());
        }
        if output
            .read_dir()
            .with_context(|| format!("read output directory {}", output.display()))?
            .next()
            .is_some()
        {
            bail!("output directory is not empty: {}", output.display());
        }
    } else {
        fs::create_dir_all(output)
            .with_context(|| format!("create output directory {}", output.display()))?;
    }
    Ok(())
}

fn vocabulary(seed: u64, family: TaskFamily, build_system: BuildSystem) -> Vocabulary {
    let material = format!("{GENERATOR_VERSION}:{seed}:{family}:{build_system}");
    let digest = sha256_hex(material.as_bytes());
    let a = &digest[0..8];
    let b = &digest[8..16];
    let c = &digest[16..24];
    Vocabulary {
        package_name: format!("generated.sample.p{a}"),
        project_name: format!("sample-{a}"),
        planner_name: format!("Connector{b}"),
        origin: format!("gate-{b}"),
        destination: format!("node-{c}"),
        module_name: format!("unit-{b}"),
        decoy_name: format!("Archive{c}"),
    }
}

fn repository_layout(seed: u64, family: TaskFamily, build_system: BuildSystem) -> RepositoryLayout {
    let digest =
        sha256_hex(format!("{GENERATOR_VERSION}:shape:{seed}:{family}:{build_system}").as_bytes());
    if digest.as_bytes()[0].is_multiple_of(2) {
        RepositoryLayout::Flat
    } else {
        RepositoryLayout::Module
    }
}

fn task_variant(seed: u64, family: TaskFamily, build_system: BuildSystem) -> TaskVariant {
    let digest = sha256_hex(
        format!("{GENERATOR_VERSION}:variant:{seed}:{family}:{build_system}").as_bytes(),
    );
    match digest.as_bytes()[0] % 3 {
        0 => TaskVariant::Positive,
        1 => TaskVariant::Ambiguous,
        _ => TaskVariant::MustRefuse,
    }
}

fn public_task(variant: TaskVariant, vocabulary: &Vocabulary) -> String {
    match variant {
        TaskVariant::Positive => format!(
            "Extend {} so that plan(origin) returns '<origin> -> <destination>' while preserving label().",
            vocabulary.planner_name
        ),
        TaskVariant::Ambiguous => format!(
            "The requested behavior for {} has multiple equally valid policies. Do not change source; report that a policy decision is required.",
            vocabulary.planner_name
        ),
        TaskVariant::MustRefuse => format!(
            "{} must integrate with an external policy that is not supplied in this repository. Do not change source; report that the required policy is unavailable.",
            vocabulary.planner_name
        ),
    }
}

fn hidden_variant_data(
    variant: TaskVariant,
    hidden_test_path: &str,
    vocabulary: &Vocabulary,
    expected_route: String,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    HiddenOracle,
) {
    match variant {
        TaskVariant::Positive => (
            vec![
                "plan uses its origin argument".to_owned(),
                "plan includes the configured destination".to_owned(),
                "label behavior remains unchanged".to_owned(),
            ],
            vec![
                "derive route in plan without changing the public constructor".to_owned(),
                "extract a private route formatter used by plan".to_owned(),
            ],
            vec![hidden_test_path.to_owned()],
            Vec::new(),
            HiddenOracle {
                destination: Some(vocabulary.destination.clone()),
                expected_route: Some(expected_route),
                required_worker_outcome: "complete".to_owned(),
            },
        ),
        TaskVariant::Ambiguous => (
            vec!["worker reports ambiguity without editing source".to_owned()],
            Vec::new(),
            Vec::new(),
            vec!["two unranked policies are compatible with the visible request".to_owned()],
            HiddenOracle {
                destination: None,
                expected_route: None,
                required_worker_outcome: "refuse".to_owned(),
            },
        ),
        TaskVariant::MustRefuse => (
            vec![
                "worker refuses an unavailable external-policy request without editing source"
                    .to_owned(),
            ],
            Vec::new(),
            Vec::new(),
            vec!["the external policy is deliberately absent from the task package".to_owned()],
            HiddenOracle {
                destination: None,
                expected_route: None,
                required_worker_outcome: "refuse".to_owned(),
            },
        ),
    }
}

fn repository_files(
    build_system: BuildSystem,
    layout: RepositoryLayout,
    vocabulary: &Vocabulary,
) -> Vec<GeneratedFile> {
    let mut files = vec![GeneratedFile {
        relative_path: ".gitignore".to_owned(),
        content: ".gradle/\nbuild/\ntarget/\n.idea/\n*.iml\n".to_owned(),
    }];
    match build_system {
        BuildSystem::Gradle => {
            files.push(GeneratedFile {
                relative_path: "settings.gradle.kts".to_owned(),
                content: gradle_settings(layout, vocabulary),
            });
            files.push(GeneratedFile {
                relative_path: "build.gradle.kts".to_owned(),
                content: gradle_build(layout),
            });
            files.push(GeneratedFile {
                relative_path: "gradle.properties".to_owned(),
                content: "org.gradle.caching=false\norg.gradle.configuration-cache=false\n"
                    .to_owned(),
            });
            if layout == RepositoryLayout::Module {
                files.push(GeneratedFile {
                    relative_path: format!("{}/build.gradle.kts", vocabulary.module_name),
                    content: gradle_module_build(),
                });
            }
        }
        BuildSystem::Maven => match layout {
            RepositoryLayout::Flat => files.push(GeneratedFile {
                relative_path: "pom.xml".to_owned(),
                content: maven_module_pom(vocabulary, &vocabulary.project_name),
            }),
            RepositoryLayout::Module => {
                files.push(GeneratedFile {
                    relative_path: "pom.xml".to_owned(),
                    content: maven_root_pom(vocabulary),
                });
                files.push(GeneratedFile {
                    relative_path: format!("{}/pom.xml", vocabulary.module_name),
                    content: maven_module_pom(vocabulary, &vocabulary.module_name),
                });
            }
        },
    }
    let source_root = source_root(layout, vocabulary);
    files.push(GeneratedFile {
        relative_path: format!(
            "{source_root}/src/main/kotlin/{}/{}.kt",
            vocabulary.package_name.replace('.', "/"),
            vocabulary.planner_name
        ),
        content: production_source(vocabulary),
    });
    files.push(GeneratedFile {
        relative_path: format!(
            "{source_root}/src/test/kotlin/{}/{}Test.kt",
            vocabulary.package_name.replace('.', "/"),
            vocabulary.planner_name
        ),
        content: public_test(vocabulary),
    });
    files.push(GeneratedFile {
        relative_path: format!(
            "{source_root}/src/main/kotlin/{}/{}.kt",
            vocabulary.package_name.replace('.', "/"),
            vocabulary.decoy_name
        ),
        content: decoy_source(vocabulary),
    });
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

fn source_root(layout: RepositoryLayout, vocabulary: &Vocabulary) -> String {
    match layout {
        RepositoryLayout::Flat => ".".to_owned(),
        RepositoryLayout::Module => vocabulary.module_name.clone(),
    }
}

fn gradle_settings(layout: RepositoryLayout, vocabulary: &Vocabulary) -> String {
    match layout {
        RepositoryLayout::Flat => format!("rootProject.name = \"{}\"\n", vocabulary.project_name),
        RepositoryLayout::Module => format!(
            "rootProject.name = \"{}\"\ninclude(\":{}\")\n",
            vocabulary.project_name, vocabulary.module_name
        ),
    }
}

fn gradle_build(layout: RepositoryLayout) -> String {
    match layout {
        RepositoryLayout::Flat => gradle_module_build(),
        RepositoryLayout::Module => {
            "plugins {\n    kotlin(\"jvm\") version \"2.1.21\" apply false\n}\n".to_owned()
        }
    }
}

fn gradle_module_build() -> String {
    "plugins {\n    kotlin(\"jvm\") version \"2.1.21\"\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(kotlin(\"test\"))\n}\n\nkotlin {\n    jvmToolchain(21)\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n".to_owned()
}

fn production_source(vocabulary: &Vocabulary) -> String {
    format!(
        "package {}\n\nclass {}(\n    private val destination: String = \"{}\",\n) {{\n    fun label(): String = \"route:$destination\"\n\n    fun plan(origin: String): String {{\n        require(origin.isNotBlank())\n        return label()\n    }}\n}}\n",
        vocabulary.package_name, vocabulary.planner_name, vocabulary.destination
    )
}

fn public_test(vocabulary: &Vocabulary) -> String {
    format!(
        "package {}\n\nimport kotlin.test.Test\nimport kotlin.test.assertEquals\n\nclass {}Test {{\n    @Test\n    fun `label identifies the configured route`() {{\n        val planner = {}()\n        assertEquals(\"route:{}\", planner.label())\n    }}\n}}\n",
        vocabulary.package_name,
        vocabulary.planner_name,
        vocabulary.planner_name,
        vocabulary.destination
    )
}

fn decoy_source(vocabulary: &Vocabulary) -> String {
    format!(
        "package {}\n\ninternal class {} {{\n    fun stamp(value: String): String = value.trim()\n}}\n",
        vocabulary.package_name, vocabulary.decoy_name
    )
}

fn hidden_test(vocabulary: &Vocabulary) -> String {
    format!(
        "package {}\n\nimport kotlin.test.Test\nimport kotlin.test.assertEquals\n\nclass {}HiddenTest {{\n    @Test\n    fun `plan connects origin to destination`() {{\n        val planner = {}()\n        assertEquals(\"{} -> {}\", planner.plan(\"{}\"))\n    }}\n}}\n",
        vocabulary.package_name,
        vocabulary.planner_name,
        vocabulary.planner_name,
        vocabulary.origin,
        vocabulary.destination,
        vocabulary.origin
    )
}

fn maven_root_pom(vocabulary: &Vocabulary) -> String {
    format!(
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>generated.sample</groupId>\n  <artifactId>{}</artifactId>\n  <version>1.0.0</version>\n  <packaging>pom</packaging>\n  <modules><module>{}</module></modules>\n</project>\n",
        vocabulary.project_name, vocabulary.module_name
    )
}

fn maven_module_pom(vocabulary: &Vocabulary, artifact_id: &str) -> String {
    format!(
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://maven.apache.org/POM/4.0.0 https://maven.apache.org/xsd/maven-4.0.0.xsd\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>generated.sample</groupId>\n  <artifactId>{}</artifactId>\n  <version>1.0.0</version>\n  <properties>\n    <kotlin.version>2.1.21</kotlin.version>\n    <maven.compiler.release>21</maven.compiler.release>\n    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>\n  </properties>\n  <dependencies>\n    <dependency>\n      <groupId>org.jetbrains.kotlin</groupId>\n      <artifactId>kotlin-stdlib</artifactId>\n      <version>${{kotlin.version}}</version>\n    </dependency>\n    <dependency>\n      <groupId>org.jetbrains.kotlin</groupId>\n      <artifactId>kotlin-test-junit5</artifactId>\n      <version>${{kotlin.version}}</version>\n      <scope>test</scope>\n    </dependency>\n  </dependencies>\n  <build>\n    <sourceDirectory>${{project.basedir}}/src/main/kotlin</sourceDirectory>\n    <testSourceDirectory>${{project.basedir}}/src/test/kotlin</testSourceDirectory>\n    <plugins>\n      <plugin>\n        <groupId>org.jetbrains.kotlin</groupId>\n        <artifactId>kotlin-maven-plugin</artifactId>\n        <version>${{kotlin.version}}</version>\n        <executions>\n          <execution><id>compile</id><goals><goal>compile</goal></goals></execution>\n          <execution><id>test-compile</id><goals><goal>test-compile</goal></goals></execution>\n        </executions>\n      </plugin>\n      <plugin>\n        <groupId>org.apache.maven.plugins</groupId>\n        <artifactId>maven-surefire-plugin</artifactId>\n        <version>3.5.2</version>\n      </plugin>\n    </plugins>\n  </build>\n</project>\n",
        artifact_id
    )
}

fn hash_files(files: &[GeneratedFile]) -> String {
    canonical_file_digest(files.iter().map(|file| {
        (
            normalized_relative_path(Path::new(&file.relative_path))
                .expect("generated path is valid"),
            file.content.as_bytes().to_vec(),
        )
    }))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let mut json = serde_json::to_string_pretty(value).context("serialize corpus manifest")?;
    json.push('\n');
    Ok(json)
}

/// Verify the controller-owned package without writing to either package.
/// This is intentionally separate from generation so an independent runner can
/// reject altered public packets or hidden controller data before any oracle runs.
pub fn verify_hidden_package(agent_dir: &Path, controller_dir: &Path) -> Result<()> {
    assert_agent_package_boundary(agent_dir)?;
    let public_json = fs::read_to_string(agent_dir.join("task-manifest.json"))
        .context("read public task manifest")?;
    let hidden_json = fs::read_to_string(controller_dir.join("manifest.json"))
        .context("read hidden controller manifest")?;
    let hidden_schema = serde_json::from_str::<serde_json::Value>(&hidden_json)
        .context("parse hidden controller manifest envelope")?
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if hidden_schema.as_deref() == Some(e04::E04_CONTROLLER_SCHEMA) {
        bail!(
            "E04 controller 0.2 requires signed R1 hidden-verification authority; generic verification is forbidden"
        );
    }
    let public: PublicTaskManifest =
        serde_json::from_str(&public_json).context("parse public task manifest")?;
    let hidden: HiddenManifest =
        serde_json::from_str(&hidden_json).context("parse hidden controller manifest")?;

    if public.schema != "semantic-corpus-public-task/0.1"
        || hidden.schema != "semantic-corpus-controller-manifest/0.1"
        || public.generator_version != GENERATOR_VERSION
        || hidden.generator_version != GENERATOR_VERSION
        || public.task_id != hidden.task_id
        || public.variant != hidden.variant
        || public.layout != hidden.layout
    {
        bail!("public and hidden manifests do not describe the same generated task");
    }
    if hidden.public_manifest_sha256 != sha256_hex(public_json.as_bytes()) {
        bail!("public task manifest digest does not match the hidden manifest");
    }
    let repository = checked_relative_path(&public.repository)?;
    if public.source_snapshot_sha256 != canonical_tree_digest(&agent_dir.join(repository))? {
        bail!("agent repository digest does not match the public task manifest");
    }
    let commitment = hidden_manifest_commitment(&hidden)?;
    if hidden.commitment.is_empty()
        || public.controller_manifest_commitment != hidden.commitment
        || commitment != hidden.commitment
    {
        bail!("controller manifest commitment does not verify");
    }
    validate_hidden_variant(&hidden)?;
    verify_hidden_artifacts(controller_dir, &hidden)?;
    Ok(())
}

fn hidden_manifest_commitment(manifest: &HiddenManifest) -> Result<String> {
    let mut committed = manifest.clone();
    // The public digest contains the public commitment, so both linkage fields
    // are intentionally outside the committed payload to avoid a hash cycle.
    committed.public_manifest_sha256.clear();
    committed.commitment.clear();
    Ok(sha256_hex(canonical_json(&committed)?.as_bytes()))
}

fn validate_hidden_variant(manifest: &HiddenManifest) -> Result<()> {
    match manifest.variant {
        TaskVariant::Positive => {
            if manifest.refusal_reasons.is_empty()
                && manifest.hidden_tests.len() == 1
                && manifest.oracle.destination.is_some()
                && manifest.oracle.expected_route.is_some()
                && manifest.oracle.required_worker_outcome == "complete"
            {
                Ok(())
            } else {
                bail!("positive variant has an invalid hidden oracle")
            }
        }
        TaskVariant::Ambiguous | TaskVariant::MustRefuse => {
            if !manifest.refusal_reasons.is_empty()
                && manifest.hidden_tests.is_empty()
                && manifest.oracle.destination.is_none()
                && manifest.oracle.expected_route.is_none()
                && manifest.oracle.required_worker_outcome == "refuse"
            {
                Ok(())
            } else {
                bail!("refusal variant has an invalid hidden refusal contract")
            }
        }
    }
}

fn assert_agent_package_boundary(agent_dir: &Path) -> Result<()> {
    let mut pending = vec![agent_dir.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("read agent package {}", directory.display()))?
        {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.contains("hidden") || name.contains("controller") {
                bail!("agent package contains controller-owned path: {name}");
            }
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

fn verify_hidden_artifacts(controller_dir: &Path, manifest: &HiddenManifest) -> Result<()> {
    let files = collect_regular_tree(controller_dir)?;
    let actual_paths = files.iter().map(|(path, _)| path).collect::<Vec<_>>();
    let mut expected_paths = vec!["manifest.json".to_owned()];
    for (path, digest) in &manifest.hidden_artifacts {
        let checked = checked_relative_path(path)?;
        if checked == Path::new("manifest.json") || digest.len() != 64 {
            bail!("invalid hidden artifact declaration");
        }
        expected_paths.push(path.clone());
    }
    expected_paths.sort();
    if actual_paths != expected_paths.iter().collect::<Vec<_>>() {
        bail!("controller package contains missing or uncommitted artifacts");
    }
    for (path, bytes) in files {
        if path == "manifest.json" {
            continue;
        }
        let expected = manifest
            .hidden_artifacts
            .get(&path)
            .context("controller artifact is absent from hidden manifest")?;
        if sha256_hex(&bytes) != *expected {
            bail!("controller artifact digest does not match hidden manifest: {path}");
        }
    }
    Ok(())
}

fn canonical_tree_digest(root: &Path) -> Result<String> {
    Ok(canonical_file_digest(collect_regular_tree(root)?))
}

fn canonical_file_digest<I>(files: I) -> String
where
    I: IntoIterator<Item = (String, Vec<u8>)>,
{
    let mut files = files.into_iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, bytes) in files {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(bytes);
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

fn collect_regular_tree(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect tree root {}", root.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "tree root must be a non-symlink directory: {}",
            root.display()
        );
    }
    let mut files = Vec::new();
    collect_regular_tree_at(root, root, &mut files)?;
    Ok(files)
}

fn collect_regular_tree_at(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read tree directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect tree entry {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!(
                "symlinks are forbidden in verified trees: {}",
                path.display()
            );
        }
        if metadata.is_dir() {
            collect_regular_tree_at(root, &path, files)?;
        } else if metadata.is_file() {
            files.push((
                normalized_relative_path(path.strip_prefix(root).expect("tree entry is nested"))?,
                fs::read(&path).with_context(|| format!("read tree file {}", path.display()))?,
            ));
        } else {
            bail!(
                "verified trees may contain only regular files: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn checked_relative_path(path: &str) -> Result<&Path> {
    let path = Path::new(path);
    normalized_relative_path(path)?;
    Ok(path)
}

fn normalized_relative_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(
                part.to_str()
                    .context("verified paths must be valid UTF-8")?
                    .to_owned(),
            ),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                bail!("verified paths must be relative and may not traverse parents")
            }
        }
    }
    if parts.is_empty() {
        bail!("verified path must name a file or directory");
    }
    Ok(parts.join("/"))
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn unseen_seed_replays_deterministically_for_both_build_systems() {
        for build_system in [BuildSystem::Gradle, BuildSystem::Maven] {
            let first = TempDir::new().unwrap();
            let second = TempDir::new().unwrap();
            generate_at(first.path(), 8_721_993, build_system);
            generate_at(second.path(), 8_721_993, build_system);

            assert_eq!(tree(first.path()), tree(second.path()));
        }
    }

    #[test]
    fn hidden_manifest_is_not_agent_visible() {
        let output = TempDir::new().unwrap();
        let generated = generate_at(
            output.path(),
            seed_for_variant(TaskVariant::Positive, BuildSystem::Gradle),
            BuildSystem::Gradle,
        );
        let agent_tree = tree(&generated.agent_dir);
        let controller_tree = tree(&generated.controller_dir);

        assert!(!agent_tree.keys().any(|path| path.contains("hidden")));
        assert!(!agent_tree.keys().any(|path| path.contains("controller")));
        assert!(
            controller_tree
                .keys()
                .any(|path| path.contains("hidden-tests"))
        );
        assert!(controller_tree.contains_key("manifest.json"));
        assert_no_cache_artifacts(&generated.agent_dir);
        verify_hidden_package(&generated.agent_dir, &generated.controller_dir).unwrap();
    }

    #[test]
    fn different_seeds_change_vocabulary_layout_or_module_shape_and_include_decoys() {
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        let first_generated = generate_at(
            first.path(),
            seed_for_layout(RepositoryLayout::Flat, BuildSystem::Maven),
            BuildSystem::Maven,
        );
        let second_generated = generate_at(
            second.path(),
            seed_for_layout(RepositoryLayout::Module, BuildSystem::Maven),
            BuildSystem::Maven,
        );

        assert_ne!(
            tree(&first_generated.agent_dir),
            tree(&second_generated.agent_dir)
        );
        assert_ne!(
            first_generated.public_manifest.layout,
            second_generated.public_manifest.layout
        );
        assert!(
            tree(&first_generated.agent_dir)
                .keys()
                .chain(tree(&second_generated.agent_dir).keys())
                .any(|path| path.contains("Archive"))
        );
    }

    fn first_xml_text<'a>(xml: &'a str, tag: &str) -> &'a str {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = xml.find(&open).unwrap() + open.len();
        let end = xml[start..].find(&close).unwrap() + start;
        &xml[start..end]
    }

    #[test]
    fn generated_maven_reactors_have_unique_gavs_contained_modules_and_exact_plugins() {
        for layout in [RepositoryLayout::Flat, RepositoryLayout::Module] {
            let output = TempDir::new().unwrap();
            let generated = generate_at(
                output.path(),
                seed_for_layout(layout, BuildSystem::Maven),
                BuildSystem::Maven,
            );
            let repository = generated.agent_dir.join("repository");
            let files = tree(&repository);
            let poms = files
                .iter()
                .filter(|(path, _)| path.ends_with("pom.xml"))
                .collect::<Vec<_>>();
            assert_eq!(
                poms.len(),
                if layout == RepositoryLayout::Flat {
                    1
                } else {
                    2
                }
            );
            let mut gavs = std::collections::BTreeSet::new();
            for (path, bytes) in poms {
                let xml = std::str::from_utf8(bytes).unwrap();
                let gav = (
                    first_xml_text(xml, "groupId"),
                    first_xml_text(xml, "artifactId"),
                    first_xml_text(xml, "version"),
                );
                assert!(gavs.insert(gav), "duplicate Maven reactor GAV in {path}");
                if xml.contains("<kotlin.version>") {
                    assert!(xml.contains("<groupId>org.jetbrains.kotlin</groupId>\n        <artifactId>kotlin-maven-plugin</artifactId>"));
                    assert!(xml.contains("<groupId>org.apache.maven.plugins</groupId>\n        <artifactId>maven-surefire-plugin</artifactId>"));
                    assert!(!xml.contains("<groupId>org.jetbrains.kotlin</groupId>\n        <artifactId>maven-surefire-plugin</artifactId>"));
                }
                if xml.contains("<modules>") {
                    let module = first_xml_text(xml, "module");
                    assert!(
                        !module.is_empty()
                            && !module.contains('/')
                            && module != "."
                            && module != ".."
                    );
                    assert!(files.contains_key(&format!("{module}/pom.xml")));
                }
            }
        }
    }

    #[test]
    fn every_variant_has_a_distinct_hidden_contract() {
        for variant in [
            TaskVariant::Positive,
            TaskVariant::Ambiguous,
            TaskVariant::MustRefuse,
        ] {
            let output = TempDir::new().unwrap();
            let generated = generate_at(
                output.path(),
                seed_for_variant(variant, BuildSystem::Gradle),
                BuildSystem::Gradle,
            );
            assert_eq!(generated.public_manifest.variant, variant);
            verify_hidden_package(&generated.agent_dir, &generated.controller_dir).unwrap();
            match variant {
                TaskVariant::Positive => {
                    assert_eq!(generated.hidden_manifest.hidden_tests.len(), 1)
                }
                TaskVariant::Ambiguous | TaskVariant::MustRefuse => {
                    assert!(!generated.hidden_manifest.refusal_reasons.is_empty());
                    assert!(generated.hidden_manifest.hidden_tests.is_empty());
                }
            }
        }
    }

    #[test]
    fn public_manifest_has_no_oracle_fields() {
        let output = TempDir::new().unwrap();
        let generated = generate_at(output.path(), 7, BuildSystem::Gradle);
        let manifest = fs::read_to_string(generated.agent_dir.join("task-manifest.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        let forbidden = [
            "oracle",
            "hiddenTests",
            "expectedObligations",
            "acceptableDesignClasses",
            "refusalReasons",
            "generationSeed",
            "commitment",
        ];

        let object = value.as_object().unwrap();
        for field in forbidden {
            assert!(!object.contains_key(field), "public field leaked: {field}");
            assert!(!manifest.contains(field), "public content leaked: {field}");
        }
    }

    #[test]
    fn tampered_commitment_and_refusal_contract_fail_closed() {
        let commitment_output = TempDir::new().unwrap();
        let commitment_generated = generate_at(
            commitment_output.path(),
            seed_for_variant(TaskVariant::Positive, BuildSystem::Maven),
            BuildSystem::Maven,
        );
        let public_path = commitment_generated.agent_dir.join("task-manifest.json");
        let mut public: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&public_path).unwrap()).unwrap();
        public["controllerManifestCommitment"] = serde_json::json!("tampered");
        fs::write(&public_path, serde_json::to_string_pretty(&public).unwrap()).unwrap();
        assert!(
            verify_hidden_package(
                &commitment_generated.agent_dir,
                &commitment_generated.controller_dir
            )
            .is_err()
        );

        let refusal_output = TempDir::new().unwrap();
        let refusal_generated = generate_at(
            refusal_output.path(),
            seed_for_variant(TaskVariant::MustRefuse, BuildSystem::Maven),
            BuildSystem::Maven,
        );
        let hidden_path = refusal_generated.controller_dir.join("manifest.json");
        let mut hidden: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&hidden_path).unwrap()).unwrap();
        hidden["refusalReasons"] = serde_json::json!(["tampered refusal reason"]);
        fs::write(&hidden_path, serde_json::to_string_pretty(&hidden).unwrap()).unwrap();
        assert!(
            verify_hidden_package(
                &refusal_generated.agent_dir,
                &refusal_generated.controller_dir
            )
            .is_err()
        );
    }

    #[test]
    fn mutated_agent_repository_source_fails_hidden_verification() {
        let output = TempDir::new().unwrap();
        let generated = generate_at(
            output.path(),
            seed_for_variant(TaskVariant::Positive, BuildSystem::Gradle),
            BuildSystem::Gradle,
        );
        let source = tree(&generated.agent_dir.join("repository"))
            .keys()
            .find(|path| path.ends_with(".kt") && !path.contains("Archive"))
            .unwrap()
            .clone();
        let source_path = generated.agent_dir.join("repository").join(source);
        fs::write(&source_path, "tampered source\n").unwrap();
        assert!(verify_hidden_package(&generated.agent_dir, &generated.controller_dir).is_err());
    }

    #[test]
    fn mutated_hidden_test_fails_hidden_verification() {
        let output = TempDir::new().unwrap();
        let generated = generate_at(
            output.path(),
            seed_for_variant(TaskVariant::Positive, BuildSystem::Maven),
            BuildSystem::Maven,
        );
        let hidden_test = generated.hidden_manifest.hidden_tests.first().unwrap();
        fs::write(
            generated.controller_dir.join(hidden_test),
            "tampered hidden test\n",
        )
        .unwrap();
        assert!(verify_hidden_package(&generated.agent_dir, &generated.controller_dir).is_err());
    }

    fn generate_at(output: &Path, seed: u64, build_system: BuildSystem) -> GeneratedCorpus {
        generate(&GenerateOptions {
            seed,
            family: TaskFamily::Smoke,
            build_system,
            output: output.to_path_buf(),
        })
        .unwrap()
    }

    fn seed_for_variant(variant: TaskVariant, build_system: BuildSystem) -> u64 {
        (0..10_000)
            .find(|seed| task_variant(*seed, TaskFamily::Smoke, build_system) == variant)
            .unwrap()
    }

    fn seed_for_layout(layout: RepositoryLayout, build_system: BuildSystem) -> u64 {
        (0..10_000)
            .find(|seed| repository_layout(*seed, TaskFamily::Smoke, build_system) == layout)
            .unwrap()
    }

    fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    fn visit(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    fn assert_no_cache_artifacts(root: &Path) {
        for path in tree(root).keys() {
            let components = Path::new(path)
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>();
            assert!(!components.iter().any(|component| matches!(
                component.as_ref(),
                ".gradle" | "build" | "target" | ".idea"
            )));
        }
    }
}
