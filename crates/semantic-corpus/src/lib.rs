use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GENERATOR_VERSION: &str = "semantic-corpus/0.1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
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

#[derive(Clone, Debug)]
pub struct GenerateOptions {
    pub seed: u64,
    pub family: TaskFamily,
    pub build_system: BuildSystem,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicTaskManifest {
    pub schema: String,
    pub generator_version: String,
    pub task_id: String,
    pub family: TaskFamily,
    pub build_system: BuildSystem,
    pub kotlin_version: String,
    pub task: String,
    pub repository: String,
    pub source_snapshot_sha256: String,
    pub build_command: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenManifest {
    pub schema: String,
    pub generator_version: String,
    pub task_id: String,
    pub generation_seed: u64,
    pub public_manifest_sha256: String,
    pub expected_obligations: Vec<String>,
    pub acceptable_design_classes: Vec<String>,
    pub hidden_tests: Vec<String>,
    pub refusal_reasons: Vec<String>,
    pub oracle: HiddenOracle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenOracle {
    pub destination: String,
    pub expected_route: String,
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
}

#[derive(Debug)]
struct GeneratedFile {
    relative_path: String,
    content: String,
}

/// Generate one isolated corpus task. The output directory must be absent or empty.
pub fn generate(options: &GenerateOptions) -> Result<GeneratedCorpus> {
    ensure_empty_output(&options.output)?;

    let vocabulary = vocabulary(options.seed, options.family, options.build_system);
    let repository_files = repository_files(options.build_system, &vocabulary);
    let source_snapshot_sha256 = hash_files(&repository_files);
    let task_id = format!("{}-{}", options.family, &source_snapshot_sha256[..16]);
    let kotlin_version = match options.build_system {
        BuildSystem::Gradle => "2.1.0",
        BuildSystem::Maven => "2.3.0",
    };

    let public_manifest = PublicTaskManifest {
        schema: "semantic-corpus-public-task/0.1".to_owned(),
        generator_version: GENERATOR_VERSION.to_owned(),
        task_id: task_id.clone(),
        family: options.family,
        build_system: options.build_system,
        kotlin_version: kotlin_version.to_owned(),
        task: format!(
            "Extend {} so that plan(origin) returns '<origin> -> <destination>' while preserving label().",
            vocabulary.planner_name
        ),
        repository: "repository".to_owned(),
        source_snapshot_sha256,
        build_command: match options.build_system {
            BuildSystem::Gradle => vec!["gradle".to_owned(), "test".to_owned()],
            BuildSystem::Maven => vec!["mvn".to_owned(), "test".to_owned()],
        },
    };
    let public_json = canonical_json(&public_manifest)?;

    let hidden_test_path = format!(
        "hidden-tests/{}/{}HiddenTest.kt",
        vocabulary.package_name.replace('.', "/"),
        vocabulary.planner_name
    );
    let expected_route = format!("{} -> {}", vocabulary.origin, vocabulary.destination);
    let hidden_manifest = HiddenManifest {
        schema: "semantic-corpus-controller-manifest/0.1".to_owned(),
        generator_version: GENERATOR_VERSION.to_owned(),
        task_id,
        generation_seed: options.seed,
        public_manifest_sha256: sha256_hex(public_json.as_bytes()),
        expected_obligations: vec![
            "plan uses its origin argument".to_owned(),
            "plan includes the configured destination".to_owned(),
            "label behavior remains unchanged".to_owned(),
        ],
        acceptable_design_classes: vec![
            "derive route in plan without changing the public constructor".to_owned(),
            "extract a private route formatter used by plan".to_owned(),
        ],
        hidden_tests: vec![hidden_test_path.clone()],
        refusal_reasons: Vec::new(),
        oracle: HiddenOracle {
            destination: vocabulary.destination.clone(),
            expected_route,
        },
    };
    let hidden_json = canonical_json(&hidden_manifest)?;

    let agent_dir = options.output.join("agent");
    let repository_dir = agent_dir.join("repository");
    let controller_dir = options.output.join("controller");
    for file in &repository_files {
        write_file(&repository_dir.join(&file.relative_path), &file.content)?;
    }
    write_file(&agent_dir.join("task-manifest.json"), &public_json)?;
    write_file(&controller_dir.join("manifest.json"), &hidden_json)?;
    write_file(
        &controller_dir.join(hidden_test_path),
        &hidden_test(&vocabulary),
    )?;

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
        package_name: format!("org.codeclew.corpus.s{a}"),
        project_name: format!("neutral-route-{a}"),
        planner_name: format!("RoutePlanner{b}"),
        origin: format!("harbor-{b}"),
        destination: format!("beacon-{c}"),
    }
}

fn repository_files(build_system: BuildSystem, vocabulary: &Vocabulary) -> Vec<GeneratedFile> {
    let mut files = vec![GeneratedFile {
        relative_path: ".gitignore".to_owned(),
        content: ".gradle/\nbuild/\ntarget/\n.idea/\n*.iml\n".to_owned(),
    }];
    match build_system {
        BuildSystem::Gradle => {
            files.push(GeneratedFile {
                relative_path: "settings.gradle.kts".to_owned(),
                content: format!("rootProject.name = \"{}\"\n", vocabulary.project_name),
            });
            files.push(GeneratedFile {
                relative_path: "build.gradle.kts".to_owned(),
                content: "plugins {\n    kotlin(\"jvm\") version \"2.1.0\"\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(kotlin(\"test\"))\n}\n\nkotlin {\n    jvmToolchain(21)\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n".to_owned(),
            });
            files.push(GeneratedFile {
                relative_path: "gradle.properties".to_owned(),
                content: "org.gradle.caching=false\norg.gradle.configuration-cache=false\n"
                    .to_owned(),
            });
        }
        BuildSystem::Maven => files.push(GeneratedFile {
            relative_path: "pom.xml".to_owned(),
            content: maven_pom(vocabulary),
        }),
    }
    files.push(GeneratedFile {
        relative_path: format!(
            "src/main/kotlin/{}/{}.kt",
            vocabulary.package_name.replace('.', "/"),
            vocabulary.planner_name
        ),
        content: production_source(vocabulary),
    });
    files.push(GeneratedFile {
        relative_path: format!(
            "src/test/kotlin/{}/{}Test.kt",
            vocabulary.package_name.replace('.', "/"),
            vocabulary.planner_name
        ),
        content: public_test(vocabulary),
    });
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
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

fn maven_pom(vocabulary: &Vocabulary) -> String {
    format!(
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://maven.apache.org/POM/4.0.0 https://maven.apache.org/xsd/maven-4.0.0.xsd\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>org.codeclew.corpus</groupId>\n  <artifactId>{}</artifactId>\n  <version>1.0.0</version>\n  <properties>\n    <kotlin.version>2.3.0</kotlin.version>\n    <maven.compiler.release>21</maven.compiler.release>\n    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>\n  </properties>\n  <dependencies>\n    <dependency>\n      <groupId>org.jetbrains.kotlin</groupId>\n      <artifactId>kotlin-stdlib</artifactId>\n      <version>${{kotlin.version}}</version>\n    </dependency>\n    <dependency>\n      <groupId>org.jetbrains.kotlin</groupId>\n      <artifactId>kotlin-test-junit5</artifactId>\n      <version>${{kotlin.version}}</version>\n      <scope>test</scope>\n    </dependency>\n  </dependencies>\n  <build>\n    <sourceDirectory>${{project.basedir}}/src/main/kotlin</sourceDirectory>\n    <testSourceDirectory>${{project.basedir}}/src/test/kotlin</testSourceDirectory>\n    <plugins>\n      <plugin>\n        <groupId>org.jetbrains.kotlin</groupId>\n        <artifactId>kotlin-maven-plugin</artifactId>\n        <version>${{kotlin.version}}</version>\n        <executions>\n          <execution><id>compile</id><goals><goal>compile</goal></goals></execution>\n          <execution><id>test-compile</id><goals><goal>test-compile</goal></goals></execution>\n        </executions>\n      </plugin>\n      <plugin>\n        <groupId>org.apache.maven.plugins</groupId>\n        <artifactId>maven-surefire-plugin</artifactId>\n        <version>3.5.2</version>\n      </plugin>\n    </plugins>\n  </build>\n</project>\n",
        vocabulary.project_name
    )
}

fn hash_files(files: &[GeneratedFile]) -> String {
    let mut digest = Sha256::new();
    for file in files {
        digest.update(file.relative_path.as_bytes());
        digest.update([0]);
        digest.update(file.content.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let mut json = serde_json::to_string_pretty(value).context("serialize corpus manifest")?;
    json.push('\n');
    Ok(json)
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
    fn deterministic_generation() {
        for build_system in [BuildSystem::Gradle, BuildSystem::Maven] {
            let first = TempDir::new().unwrap();
            let second = TempDir::new().unwrap();
            generate_at(first.path(), 42, build_system);
            generate_at(second.path(), 42, build_system);

            assert_eq!(tree(first.path()), tree(second.path()));
        }
    }

    #[test]
    fn hidden_manifest_is_not_agent_visible() {
        let output = TempDir::new().unwrap();
        let generated = generate_at(output.path(), 91, BuildSystem::Gradle);
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
    }

    #[test]
    fn different_seed_changes_vocabulary() {
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        let first_generated = generate_at(first.path(), 100, BuildSystem::Maven);
        let second_generated = generate_at(second.path(), 101, BuildSystem::Maven);

        assert_ne!(
            tree(&first_generated.agent_dir),
            tree(&second_generated.agent_dir)
        );
        assert_ne!(
            first_generated.hidden_manifest.oracle.destination,
            second_generated.hidden_manifest.oracle.destination
        );
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
        ];

        let object = value.as_object().unwrap();
        for field in forbidden {
            assert!(!object.contains_key(field), "public field leaked: {field}");
            assert!(!manifest.contains(field), "public content leaked: {field}");
        }
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
