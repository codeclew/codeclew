use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const JAVA_MODEL_SCHEMA: &str = "codeclew-java-project-model/1.0";
const GRADLE_MARKER: &str = "__CODECLEW_JAVA_MODEL__";
const MAX_MODEL_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_JAVA_SOURCES: usize = 16_384;
const MAX_CLASSPATH_ENTRIES: usize = 4_096;
const MAX_CLASSPATH_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CLASSPATH_DIRECTORY_FILES: usize = 65_536;

const GRADLE_MODEL_SCRIPT: &str = r#"
import groovy.json.JsonOutput
import org.gradle.api.tasks.compile.JavaCompile

gradle.beforeProject { project ->
    project.tasks.register("codeclewJavaModel") {
        doLast {
            def requested = System.getProperty("codeclew.java.compileTask", "compileJava")
            def compile = project.tasks.findByName(requested)
            if (!(compile instanceof JavaCompile)) {
                throw new GradleException("selected JavaCompile task is unavailable")
            }
            def compiler = compile.javaCompiler.orNull
            if (compiler == null) {
                throw new GradleException("selected JavaCompile toolchain is unavailable")
            }
            def release = compile.options.release.orNull
            def model = [
                projectPath: project.path,
                compileTask: compile.name,
                sourceFiles: compile.source.files.collect { it.absolutePath }.sort(),
                classpath: compile.classpath.files.collect { it.absolutePath },
                release: release == null ? null : release,
                sourceCompatibility: compile.sourceCompatibility,
                targetCompatibility: compile.targetCompatibility,
                compilerArgs: compile.options.compilerArgs.collect { it.toString() },
                jdkHome: compiler.metadata.installationPath.asFile.absolutePath,
                jdkLanguageVersion: compiler.metadata.languageVersion.asInt(),
                generatedSourcesDirectory: compile.options.generatedSourceOutputDirectory.orNull?.asFile?.absolutePath,
            ]
            println("__CODECLEW_JAVA_MODEL__" + JsonOutput.toJson(model))
        }
    }
}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JavaBuildSystem {
    Gradle,
    Maven,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaClasspathAuthority {
    pub logical_name: String,
    pub digest: String,
    pub size: u64,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaProjectModel {
    pub schema: String,
    pub model_digest: String,
    pub build_system: JavaBuildSystem,
    pub compilation: String,
    pub source_files: Vec<String>,
    pub classpath: Vec<JavaClasspathAuthority>,
    pub release: u16,
    pub compiler_version: String,
    pub compiler_options: Vec<String>,
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct JavaOperationalModel {
    pub authority: JavaProjectModel,
    pub source_paths: Vec<PathBuf>,
    pub classpath_paths: Vec<PathBuf>,
    pub java_executable: PathBuf,
}

pub fn extract_java_model(
    repository: &Path,
    compilation: &str,
) -> Result<JavaOperationalModel, ClewError> {
    let repository = repository.canonicalize().map_err(io_error)?;
    let selector = JavaCompilationSelector::parse(compilation)?;
    let gradle = repository.join("gradlew").is_file()
        && (repository.join("settings.gradle").is_file()
            || repository.join("settings.gradle.kts").is_file());
    let maven = repository.join("pom.xml").is_file();
    match (gradle, maven) {
        (true, false) => extract_gradle(&repository, &selector),
        (false, true) => extract_maven(&repository, &selector),
        (true, true) => Err(unsupported(
            "Java build authority is ambiguous between Gradle and Maven",
        )),
        (false, false) => Err(unsupported(
            "Java profile requires a Gradle wrapper or Maven project",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JavaCompilationSelector {
    project_path: String,
    source_set: String,
}

impl JavaCompilationSelector {
    fn parse(value: &str) -> Result<Self, ClewError> {
        if value.len() > 256 || !value.starts_with(':') {
            return Err(invalid("Java compilation selector is invalid"));
        }
        let Some((project_path, source_set)) = value.split_once('/') else {
            return Err(invalid("Java compilation selector is invalid"));
        };
        if !matches!(source_set, "main" | "test")
            || project_path[1..]
                .split(':')
                .any(|segment| !segment.is_empty() && !safe_segment(segment))
        {
            return Err(invalid("Java compilation selector is invalid"));
        }
        Ok(Self {
            project_path: if project_path.is_empty() {
                ":".into()
            } else {
                project_path.into()
            },
            source_set: source_set.into(),
        })
    }

    fn canonical(&self) -> String {
        format!("{}/{}", self.project_path, self.source_set)
    }

    fn gradle_compile_task(&self) -> String {
        if self.source_set == "main" {
            "compileJava".into()
        } else {
            "compileTestJava".into()
        }
    }

    fn gradle_model_task(&self) -> String {
        if self.project_path == ":" {
            ":codeclewJavaModel".into()
        } else {
            format!("{}:codeclewJavaModel", self.project_path)
        }
    }

    fn maven_project_directory(&self, repository: &Path) -> Result<PathBuf, ClewError> {
        let relative = self.project_path.trim_start_matches(':').replace(':', "/");
        let directory = if relative.is_empty() {
            repository.to_owned()
        } else {
            repository.join(relative)
        };
        let normalized = directory
            .canonicalize()
            .map_err(|_| unsupported("selected Maven module directory is unavailable"))?;
        if !normalized.starts_with(repository) || !normalized.join("pom.xml").is_file() {
            return Err(unsupported("selected Maven module has no pom.xml"));
        }
        Ok(normalized)
    }
}

fn extract_gradle(
    repository: &Path,
    selector: &JavaCompilationSelector,
) -> Result<JavaOperationalModel, ClewError> {
    let script = tempfile::Builder::new()
        .prefix("codeclew-java-model-")
        .suffix(".init.gradle")
        .tempfile()
        .map_err(io_error)?;
    fs::write(script.path(), GRADLE_MODEL_SCRIPT).map_err(io_error)?;
    let output = bounded_output(
        Command::new(repository.join("gradlew"))
            .args([
                "-p",
                repository
                    .to_str()
                    .ok_or_else(|| unsupported("Java repository path is not UTF-8"))?,
                "--no-daemon",
                "--quiet",
                "-I",
                script
                    .path()
                    .to_str()
                    .ok_or_else(|| internal("temporary model path is not UTF-8"))?,
                &format!(
                    "-Dcodeclew.java.compileTask={}",
                    selector.gradle_compile_task()
                ),
                &selector.gradle_model_task(),
            ])
            .current_dir(repository),
        "Gradle Java model extraction failed",
    )?;
    let line = output
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(GRADLE_MARKER))
        .ok_or_else(|| unsupported("Gradle Java model marker is unavailable"))?;
    let value: Value =
        serde_json::from_str(line).map_err(|_| unsupported("Gradle Java model is invalid"))?;
    let object = value
        .as_object()
        .ok_or_else(|| unsupported("Gradle Java model is not an object"))?;
    if object.get("projectPath").and_then(Value::as_str) != Some(&selector.project_path)
        || object.get("compileTask").and_then(Value::as_str)
            != Some(selector.gradle_compile_task().as_str())
        || object.get("jdkLanguageVersion").and_then(Value::as_u64) != Some(21)
    {
        return Err(unsupported(
            "Gradle Java model differs from the selected Java 21 compilation",
        ));
    }
    let release = object
        .get("release")
        .and_then(Value::as_u64)
        .or_else(|| {
            object
                .get("targetCompatibility")
                .and_then(Value::as_str)
                .and_then(parse_java_level)
                .map(u64::from)
        })
        .ok_or_else(|| unsupported("Gradle Java release authority is unavailable"))?;
    if release != 21 {
        return Err(unsupported("Java v1 supports only release 21"));
    }
    let source_paths = string_paths(object.get("sourceFiles"), MAX_JAVA_SOURCES)?;
    let classpath_paths = string_paths(object.get("classpath"), MAX_CLASSPATH_ENTRIES)?;
    let jdk_home = object
        .get("jdkHome")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| unsupported("Gradle Java toolchain home is unavailable"))?;
    let compiler_options = string_values(object.get("compilerArgs"), 4_096)?;
    canonical_model(
        repository,
        selector,
        JavaBuildSystem::Gradle,
        source_paths,
        classpath_paths,
        jdk_home.join("bin/java"),
        jdk_home.join("bin/javac"),
        compiler_options,
        Vec::new(),
    )
}

fn extract_maven(
    repository: &Path,
    selector: &JavaCompilationSelector,
) -> Result<JavaOperationalModel, ClewError> {
    let project = selector.maven_project_directory(repository)?;
    let pom = fs::read_to_string(project.join("pom.xml")).map_err(io_error)?;
    if pom.contains("<sourceDirectory>")
        || pom.contains("<testSourceDirectory>")
        || pom.contains("generated-sources")
        || pom.contains("maven-toolchains-plugin")
    {
        return Err(unsupported(
            "Maven Java v1 does not admit custom/generated sources or toolchains",
        ));
    }
    let source_root = project.join(if selector.source_set == "main" {
        "src/main/java"
    } else {
        "src/test/java"
    });
    let source_paths = java_sources(&source_root)?;
    let temporary = tempfile::tempdir().map_err(io_error)?;
    let classpath_file = temporary.path().join("classpath.txt");
    let launcher = if repository.join("mvnw").is_file() {
        repository.join("mvnw")
    } else {
        PathBuf::from("mvn")
    };
    let scope = if selector.source_set == "main" {
        "compile"
    } else {
        "test"
    };
    bounded_output(
        Command::new(&launcher)
            .args([
                "-f",
                project
                    .join("pom.xml")
                    .to_str()
                    .ok_or_else(|| unsupported("Maven pom path is not UTF-8"))?,
                "-q",
                "-DskipTests",
                "-Dstyle.color=never",
                &format!("-Dmdep.outputFile={}", classpath_file.display()),
                &format!("-Dmdep.includeScope={scope}"),
                "dependency:build-classpath",
            ])
            .current_dir(repository),
        "Maven Java classpath extraction failed",
    )?;
    let classpath = fs::read_to_string(&classpath_file)
        .map_err(|_| unsupported("Maven Java classpath output is unavailable"))?;
    let mut classpath_paths = if classpath.trim().is_empty() {
        Vec::new()
    } else {
        std::env::split_paths(classpath.trim()).collect::<Vec<_>>()
    };
    if selector.source_set == "test" {
        classpath_paths.push(project.join("target/classes"));
    }
    let release_output = bounded_output(
        Command::new(&launcher)
            .args([
                "-f",
                project
                    .join("pom.xml")
                    .to_str()
                    .ok_or_else(|| unsupported("Maven pom path is not UTF-8"))?,
                "-q",
                "-DforceStdout",
                "-Dstyle.color=never",
                "help:evaluate",
                "-Dexpression=maven.compiler.release",
            ])
            .current_dir(repository),
        "Maven Java release extraction failed",
    )?;
    let release = release_output
        .lines()
        .filter_map(|line| parse_java_level(line.trim()))
        .next_back()
        .ok_or_else(|| unsupported("Maven Java release authority is unavailable"))?;
    if release != 21 {
        return Err(unsupported("Java v1 supports only release 21"));
    }
    let java_home = std::env::var_os("JAVA_HOME").map(PathBuf::from);
    let java = java_home
        .as_ref()
        .map(|home| home.join("bin/java"))
        .unwrap_or_else(|| PathBuf::from("java"));
    let javac = java_home
        .as_ref()
        .map(|home| home.join("bin/javac"))
        .unwrap_or_else(|| PathBuf::from("javac"));
    canonical_model(
        repository,
        selector,
        JavaBuildSystem::Maven,
        source_paths,
        classpath_paths,
        java,
        javac,
        vec!["--release=21".into()],
        Vec::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn canonical_model(
    repository: &Path,
    selector: &JavaCompilationSelector,
    build_system: JavaBuildSystem,
    mut source_paths: Vec<PathBuf>,
    classpath_paths: Vec<PathBuf>,
    java_executable: PathBuf,
    javac_executable: PathBuf,
    mut compiler_options: Vec<String>,
    mut boundaries: Vec<String>,
) -> Result<JavaOperationalModel, ClewError> {
    let repository = repository.canonicalize().map_err(io_error)?;
    if source_paths.is_empty() || source_paths.len() > MAX_JAVA_SOURCES {
        return Err(unsupported(
            "selected Java compilation has no bounded source set",
        ));
    }
    source_paths.sort();
    source_paths.dedup();
    if source_paths.len() > MAX_JAVA_SOURCES {
        return Err(resource("Java source count exceeds its bounded profile"));
    }
    let source_files = source_paths
        .iter()
        .map(|path| relative_source(&repository, path))
        .collect::<Result<Vec<_>, _>>()?;
    if source_files
        .iter()
        .any(|path| path.contains("/build/generated/") || path.contains("/target/generated-"))
    {
        return Err(unsupported(
            "generated Java sources are outside the Java v1 authority",
        ));
    }
    let compiler_version = compiler_version(&javac_executable, &repository)?;
    if !compiler_version.starts_with("javac 21") {
        return Err(unsupported("Java v1 requires a JDK 21 compiler"));
    }
    let mut classpath = Vec::with_capacity(classpath_paths.len());
    for path in &classpath_paths {
        classpath.push(classpath_authority(path)?);
    }
    compiler_options.sort();
    compiler_options.dedup();
    boundaries.sort();
    boundaries.dedup();
    let mut authority = JavaProjectModel {
        schema: JAVA_MODEL_SCHEMA.into(),
        model_digest: String::new(),
        build_system,
        compilation: selector.canonical(),
        source_files,
        classpath,
        release: 21,
        compiler_version,
        compiler_options,
        boundaries,
    };
    authority.model_digest = canonical::hash(&authority).map_err(internal)?;
    verify_model(&authority)?;
    Ok(JavaOperationalModel {
        authority,
        source_paths,
        classpath_paths,
        java_executable,
    })
}

pub fn verify_model(model: &JavaProjectModel) -> Result<(), ClewError> {
    if model.schema != JAVA_MODEL_SCHEMA
        || model.release != 21
        || model.source_files.is_empty()
        || model.source_files.len() > MAX_JAVA_SOURCES
        || model.source_files.windows(2).any(|pair| pair[0] >= pair[1])
        || model.classpath.len() > MAX_CLASSPATH_ENTRIES
        || model
            .compiler_options
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || model.boundaries.windows(2).any(|pair| pair[0] >= pair[1])
        || JavaCompilationSelector::parse(&model.compilation).is_err()
    {
        return Err(invalid("Java project model authority is invalid"));
    }
    let mut unsigned = model.clone();
    unsigned.model_digest.clear();
    if model.model_digest != canonical::hash(&unsigned).map_err(internal)? {
        return Err(invalid("Java project model digest is invalid"));
    }
    Ok(())
}

fn java_sources(root: &Path) -> Result<Vec<PathBuf>, ClewError> {
    if !root.is_dir() {
        return Err(unsupported(
            "selected Maven Java source root is unavailable",
        ));
    }
    let mut sources = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|_| unsupported("Maven Java source traversal failed"))?;
        if entry.file_type().is_symlink() {
            return Err(unsupported("Java source set contains a symlink"));
        }
        if entry.file_type().is_file() && entry.path().extension() == Some(OsStr::new("java")) {
            sources.push(entry.into_path());
            if sources.len() > MAX_JAVA_SOURCES {
                return Err(resource("Java source count exceeds its bounded profile"));
            }
        }
    }
    sources.sort();
    Ok(sources)
}

fn relative_source(repository: &Path, path: &Path) -> Result<String, ClewError> {
    let normalized = path
        .canonicalize()
        .map_err(|_| unsupported("Java source file is unavailable"))?;
    let relative = normalized
        .strip_prefix(repository)
        .map_err(|_| unsupported("Java source file escapes the repository"))?;
    if normalized.extension() != Some(OsStr::new("java"))
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(unsupported(
            "Java source path is outside its bounded profile",
        ));
    }
    relative
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| unsupported("Java source path is not UTF-8"))
}

fn classpath_authority(path: &Path) -> Result<JavaClasspathAuthority, ClewError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| unsupported("Java classpath entry is unavailable"))?;
    if metadata.file_type().is_symlink() {
        return Err(unsupported("Java classpath entry is a symlink"));
    }
    if metadata.is_file() {
        if metadata.len() > MAX_CLASSPATH_FILE_BYTES {
            return Err(resource("Java classpath artifact exceeds its byte budget"));
        }
        let digest = hash_file(path)?;
        return Ok(JavaClasspathAuthority {
            logical_name: format!(
                "artifact:{}:{}",
                path.file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("unnamed"),
                digest.trim_start_matches("sha256:")
            ),
            digest,
            size: metadata.len(),
            kind: "FILE".into(),
        });
    }
    if metadata.is_dir() {
        return hash_directory(path);
    }
    Err(unsupported("Java classpath entry has an unsupported kind"))
}

fn hash_directory(root: &Path) -> Result<JavaClasspathAuthority, ClewError> {
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|_| unsupported("Java classpath directory is unreadable"))?;
        if entry.file_type().is_symlink() {
            return Err(unsupported("Java classpath directory contains a symlink"));
        }
        if entry.file_type().is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| internal("classpath directory traversal escaped"))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| unsupported("Java classpath path is not UTF-8"))?
                .replace('\\', "/");
            entries.push((
                relative,
                hash_file(entry.path())?,
                entry
                    .metadata()
                    .map_err(|_| unsupported("Java classpath metadata is unavailable"))?
                    .len(),
            ));
            if entries.len() > MAX_CLASSPATH_DIRECTORY_FILES {
                return Err(resource("Java classpath directory exceeds its file budget"));
            }
        }
    }
    entries.sort();
    let size = entries.iter().map(|entry| entry.2).sum();
    let digest = canonical::hash(&entries).map_err(internal)?;
    Ok(JavaClasspathAuthority {
        logical_name: format!("directory:{}", digest.trim_start_matches("sha256:")),
        digest,
        size,
        kind: "DIRECTORY".into(),
    })
}

fn hash_file(path: &Path) -> Result<String, ClewError> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex::encode(digest.finalize())))
}

fn compiler_version(executable: &Path, repository: &Path) -> Result<String, ClewError> {
    let output = Command::new(executable)
        .arg("-version")
        .current_dir(repository)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| unsupported("JDK compiler is unavailable"))?;
    if !output.status.success() {
        return Err(unsupported("JDK compiler version query failed"));
    }
    let bytes = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    let version = std::str::from_utf8(bytes)
        .map_err(|_| unsupported("JDK compiler version is not UTF-8"))?
        .trim();
    if version.is_empty() || version.len() > 256 {
        return Err(unsupported("JDK compiler version is invalid"));
    }
    Ok(version.into())
}

fn bounded_output(command: &mut Command, message: &str) -> Result<String, ClewError> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| unsupported(message))?;
    if !output.status.success()
        || output.stdout.len().saturating_add(output.stderr.len()) > MAX_MODEL_OUTPUT_BYTES
    {
        return Err(unsupported(message));
    }
    String::from_utf8(output.stdout).map_err(|_| unsupported("Java model output is not UTF-8"))
}

fn string_paths(value: Option<&Value>, max: usize) -> Result<Vec<PathBuf>, ClewError> {
    let values = string_values(value, max)?;
    Ok(values.into_iter().map(PathBuf::from).collect())
}

fn string_values(value: Option<&Value>, max: usize) -> Result<Vec<String>, ClewError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| unsupported("Java model string list is unavailable"))?;
    if values.len() > max {
        return Err(resource("Java model string list exceeds its bound"));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= 16 * 1024)
                .map(str::to_owned)
                .ok_or_else(|| unsupported("Java model contains an invalid string"))
        })
        .collect()
}

fn parse_java_level(value: &str) -> Option<u16> {
    value
        .strip_prefix("1.")
        .unwrap_or(value)
        .parse::<u16>()
        .ok()
}

fn safe_segment(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_is_exact_and_bounded() {
        let root = JavaCompilationSelector::parse(":/main").unwrap();
        assert_eq!(root.gradle_compile_task(), "compileJava");
        assert_eq!(root.gradle_model_task(), ":codeclewJavaModel");
        let nested = JavaCompilationSelector::parse(":app/test").unwrap();
        assert_eq!(nested.gradle_compile_task(), "compileTestJava");
        assert_eq!(nested.gradle_model_task(), ":app:codeclewJavaModel");
        for invalid in ["main", ":/integration", ":../x/main", ":app/main/extra"] {
            assert!(
                JavaCompilationSelector::parse(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn canonical_model_has_no_operational_paths() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src/main/java/example/App.java");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "package example; class App {}").unwrap();
        let javac = PathBuf::from("javac");
        let java = PathBuf::from("java");
        let model = canonical_model(
            root.path(),
            &JavaCompilationSelector::parse(":/main").unwrap(),
            JavaBuildSystem::Maven,
            vec![source],
            vec![],
            java,
            javac,
            vec!["--release=21".into()],
            vec![],
        )
        .unwrap();
        let bytes = canonical::bytes(&model.authority).unwrap();
        assert!(
            !String::from_utf8(bytes)
                .unwrap()
                .contains(root.path().to_str().unwrap())
        );
        verify_model(&model.authority).unwrap();
    }

    #[test]
    fn classpath_authority_is_content_not_location() {
        let left = tempfile::tempdir().unwrap();
        let right = tempfile::tempdir().unwrap();
        let left_file = left.path().join("dependency.jar");
        let right_file = right.path().join("dependency.jar");
        fs::write(&left_file, b"same").unwrap();
        fs::write(&right_file, b"same").unwrap();
        assert_eq!(
            classpath_authority(&left_file).unwrap(),
            classpath_authority(&right_file).unwrap()
        );
    }

    #[test]
    #[ignore = "qualification launches project-native Gradle and Maven model extraction"]
    fn java_fixtures_extract_project_native_models() {
        let workspace = crate::worker::workspace_root();
        for fixture in ["java-gradle", "java-maven"] {
            let model = extract_java_model(&workspace.join("fixtures").join(fixture), ":/main")
                .unwrap_or_else(|error| panic!("{fixture}: {error}"));
            assert_eq!(model.authority.release, 21);
            assert!(model.authority.source_files.len() >= 2);
            assert!(model.authority.compiler_version.starts_with("javac 21"));
            assert!(!model.authority.model_digest.is_empty());
            let encoded = serde_json::to_string(&model.authority).unwrap();
            assert!(!encoded.contains(workspace.to_str().unwrap()));
        }
    }
}
