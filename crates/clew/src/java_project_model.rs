use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    /// Explicitly admitted annotation-processor class names. Empty means no
    /// annotation processing is authorized. Derived only from admitted project
    /// compiler configuration (a `-processor:...`/`-processor` option or the
    /// Maven compiler-plugin `annotationProcessors`/`-processor` compilerArg),
    /// never inferred from classpath presence alone.
    #[serde(default)]
    pub annotation_processors: Vec<String>,
    /// Immutable processor-path artifacts explicitly admitted for annotation
    /// processing (for example the Maven compiler-plugin `annotationProcessorPaths`).
    /// Their digests participate in model identity so a processor change
    /// invalidates semantic authority. Empty means no processor paths admitted.
    #[serde(default)]
    pub annotation_processor_paths: Vec<JavaClasspathAuthority>,
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct JavaOperationalModel {
    pub authority: JavaProjectModel,
    pub source_paths: Vec<PathBuf>,
    pub classpath_paths: Vec<PathBuf>,
    pub annotation_processor_paths: Vec<PathBuf>,
    pub java_executable: PathBuf,
}

pub fn extract_java_model(
    repository: &Path,
    compilation: &str,
) -> Result<JavaOperationalModel, ClewError> {
    extract_java_model_with_settings(repository, compilation, None)
}

pub fn extract_java_model_with_settings(
    repository: &Path,
    compilation: &str,
    settings: Option<&crate::maven::MavenSettings>,
) -> Result<JavaOperationalModel, ClewError> {
    extract_java_model_with_settings_and_diagnostics(repository, compilation, settings, None, &[])
}

pub(crate) fn extract_java_model_with_settings_and_diagnostics(
    repository: &Path,
    compilation: &str,
    settings: Option<&crate::maven::MavenSettings>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
    extra_annotation_processors: &[String],
) -> Result<JavaOperationalModel, ClewError> {
    let mut models = extract_java_models_with_settings_and_diagnostics(
        repository,
        &[compilation.to_owned()],
        settings,
        debug_output,
        extra_annotation_processors,
    )?;
    models
        .pop()
        .ok_or_else(|| internal("single Java model extraction returned no model"))
}

/// Extract all selected Java compilations in deterministic order. Maven model
/// preflights complete before any cohort compilation starts; Gradle retains its
/// existing one-task-per-selector behavior.
pub(crate) fn extract_java_models_with_settings_and_diagnostics(
    repository: &Path,
    compilations: &[String],
    settings: Option<&crate::maven::MavenSettings>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
    extra_annotation_processors: &[String],
) -> Result<Vec<JavaOperationalModel>, ClewError> {
    if compilations.is_empty() {
        return Err(invalid("Java compilation selection is empty"));
    }
    let repository = repository.canonicalize().map_err(io_error)?;
    let mut selectors = compilations
        .iter()
        .map(|compilation| JavaCompilationSelector::parse(compilation))
        .collect::<Result<Vec<_>, _>>()?;
    selectors.sort_by_key(JavaCompilationSelector::canonical);
    if selectors
        .windows(2)
        .any(|pair| pair[0].canonical() == pair[1].canonical())
    {
        return Err(invalid("Java compilation selection contains a duplicate"));
    }
    let gradle = repository.join("gradlew").is_file()
        && (repository.join("settings.gradle").is_file()
            || repository.join("settings.gradle.kts").is_file());
    let maven = repository.join("pom.xml").is_file();
    match (gradle, maven) {
        (true, false) if settings.is_none() => selectors
            .iter()
            .map(|selector| extract_gradle(&repository, selector))
            .collect(),
        (true, false) => Err(invalid("--maven-settings requires a Java Maven project")),
        (false, true) => extract_maven_batch(
            &repository,
            &selectors,
            settings,
            debug_output,
            extra_annotation_processors,
        ),
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
        "GRADLE_MODEL",
        None,
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
    {
        return Err(unsupported(
            "Gradle Java model differs from the selected compilation",
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
    let release = u16::try_from(release)
        .map_err(|_| unsupported("Java release authority exceeds the supported numeric range"))?;
    let toolchain = object
        .get("jdkLanguageVersion")
        .and_then(Value::as_u64)
        .and_then(|version| u16::try_from(version).ok())
        .ok_or_else(|| unsupported("Gradle Java toolchain version is unavailable"))?;
    validate_release(release, toolchain)?;
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
        release,
        compiler_options,
        Vec::new(),
        Vec::new(),
    )
}

fn maven_scope(source_set: &str) -> &'static str {
    if source_set == "main" {
        "compile"
    } else {
        "test"
    }
}

fn maven_command(
    repository: &Path,
    settings_path: Option<&Path>,
    source_set: &str,
) -> Result<Command, ClewError> {
    let mut command = crate::maven::command(repository)?;
    if let Some(settings_path) = settings_path {
        command.arg("--settings").arg(settings_path);
    }
    let scope_property = format!("-Dmdep.includeScope={}", maven_scope(source_set));
    command.args([
        "-DskipTests",
        "-Dstyle.color=never",
        "-Dmdep.outputFile=target/codeclew-classpath.txt",
        "-Dmdep.regenerateFile=true",
        &scope_property,
    ]);
    Ok(command)
}

struct MavenPreflight {
    selector: JavaCompilationSelector,
    project: PathBuf,
    source_root: PathBuf,
    processor_names: Vec<String>,
    processor_coordinates: Vec<String>,
}

struct MavenCaptured {
    index: usize,
    model: JavaOperationalModel,
    source_digests: BTreeMap<PathBuf, String>,
    classpath_paths: Vec<PathBuf>,
}

fn extract_maven_batch(
    repository: &Path,
    selectors: &[JavaCompilationSelector],
    admitted_settings: Option<&crate::maven::MavenSettings>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
    extra_annotation_processors: &[String],
) -> Result<Vec<JavaOperationalModel>, ClewError> {
    let settings_file = admitted_settings
        .map(crate::maven::MavenSettings::materialize)
        .transpose()?;
    let settings_path = settings_file.as_ref().map(|file| file.path().to_owned());
    let mut effective_cache = BTreeMap::<(PathBuf, String), String>::new();
    let mut preflights = Vec::with_capacity(selectors.len());
    for selector in selectors {
        let project = selector.maven_project_directory(repository)?;
        let key = (project.clone(), selector.source_set.clone());
        let effective_xml = if let Some(xml) = effective_cache.get(&key) {
            xml.clone()
        } else {
            let effective_pom = tempfile::NamedTempFile::new().map_err(io_error)?;
            discard_build_output(
                maven_command(repository, settings_path.as_deref(), &selector.source_set)?
                    .arg("-f")
                    .arg(project.join("pom.xml"))
                    .args(["-B", "-q", "-N", "help:effective-pom"])
                    .arg(format!("-Doutput={}", effective_pom.path().display()))
                    .current_dir(repository),
                "Maven Java effective model extraction failed",
                "EFFECTIVE_POM",
                debug_output,
            )?;
            let mut effective_xml = String::new();
            effective_pom
                .reopen()
                .map_err(io_error)?
                .take(MAX_MODEL_OUTPUT_BYTES as u64 + 1)
                .read_to_string(&mut effective_xml)
                .map_err(io_error)?;
            if effective_xml.len() > MAX_MODEL_OUTPUT_BYTES {
                return Err(resource(
                    "Maven Java effective model exceeds the byte limit",
                ));
            }
            effective_cache.insert(key, effective_xml.clone());
            effective_xml
        };
        validate_maven_build_layout(&project, &effective_xml)?;
        let source_root = project.join(if selector.source_set == "main" {
            "src/main/java"
        } else {
            "src/test/java"
        });
        // Keep early bounds/symlink validation for existing roots without
        // treating their pre-build membership as the final source authority.
        if source_root.exists() {
            java_sources(&source_root)?;
        }
        let (processor_names, processor_coordinates) =
            maven_compiler_processor_declarations(&effective_xml)?;
        preflights.push(MavenPreflight {
            selector: selector.clone(),
            project,
            source_root,
            processor_names,
            processor_coordinates,
        });
    }

    let mut cohorts = BTreeMap::<String, Vec<usize>>::new();
    for (index, preflight) in preflights.iter().enumerate() {
        cohorts
            .entry(preflight.selector.source_set.clone())
            .or_default()
            .push(index);
    }
    let mut captured = Vec::with_capacity(preflights.len());
    for (source_set, indexes) in cohorts {
        let mut classpath_files = Vec::with_capacity(indexes.len());
        for index in &indexes {
            let path = preflights[*index]
                .project
                .join("target/codeclew-classpath.txt");
            remove_stale_classpath(&path)?;
            classpath_files.push(path);
        }
        let mut build = maven_command(repository, settings_path.as_deref(), &source_set)?;
        build.arg("-f").arg(repository.join("pom.xml"));
        let mut modules = indexes
            .iter()
            .map(|index| {
                let project = &preflights[*index].project;
                if project == repository {
                    Ok(".".to_owned())
                } else {
                    project
                        .strip_prefix(repository)
                        .map_err(internal)
                        .and_then(|relative| {
                            relative
                                .to_str()
                                .map(|value| value.replace('\\', "/"))
                                .ok_or_else(|| unsupported("Maven module path is not UTF-8"))
                        })
                }
            })
            .collect::<Result<Vec<_>, ClewError>>()?;
        modules.sort();
        modules.dedup();
        build.arg("-pl").arg(modules.join(",")).arg("-am");
        discard_build_output(
            build
                .args([
                    "-B",
                    "-q",
                    if source_set == "main" {
                        "compile"
                    } else {
                        "test-compile"
                    },
                    "dependency:build-classpath",
                ])
                .current_dir(repository),
            "Maven Java compilation and classpath extraction failed",
            "COMPILE_CLASSPATH",
            debug_output,
        )?;
        let local_repo = maven_local_repository(settings_path.as_deref())?;

        // Membership is an output of the completed build: generate-sources
        // may legitimately add Java files under the admitted source root.
        // Freeze every selected scope before subsequent model commands or
        // another cohort can change its membership or bytes.
        let source_captures = indexes
            .iter()
            .map(|index| {
                let paths = java_sources(&preflights[*index].source_root)?;
                let digests = source_digest_authority(&paths)?;
                Ok((paths, digests))
            })
            .collect::<Result<Vec<_>, ClewError>>()?;

        for ((index, classpath_file), (source_paths, source_digests)) in
            indexes.iter().zip(classpath_files).zip(source_captures)
        {
            let preflight = &preflights[*index];
            let classpath = fs::read_to_string(&classpath_file).map_err(|_| {
                unsupported("Maven Java classpath output is unavailable after a successful build")
            })?;
            let mut classpath_paths = if classpath.trim().is_empty() {
                Vec::new()
            } else {
                std::env::split_paths(classpath.trim()).collect::<Vec<_>>()
            };
            if preflight.project.join("target/classes").is_dir() {
                classpath_paths.push(preflight.project.join("target/classes"));
            }
            if source_set == "test" && preflight.project.join("target/test-classes").is_dir() {
                classpath_paths.push(preflight.project.join("target/test-classes"));
            }
            let release_output = bounded_output(
                maven_command(repository, settings_path.as_deref(), &source_set)?
                    .args([
                        "-f",
                        preflight
                            .project
                            .join("pom.xml")
                            .to_str()
                            .ok_or_else(|| unsupported("Maven pom path is not UTF-8"))?,
                        "-q",
                        "-DforceStdout",
                        "help:evaluate",
                        "-Dexpression=maven.compiler.release",
                    ])
                    .current_dir(repository),
                "Maven Java release extraction failed",
                "RELEASE",
                debug_output,
            )?;
            let release = release_output
                .lines()
                .filter_map(|line| parse_java_level(line.trim()))
                .next_back()
                .ok_or_else(|| unsupported("Maven Java release authority is unavailable"))?;
            if release < 17 {
                return Err(unsupported("Java analysis requires release 17 or newer"));
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
            let mut boundaries = Vec::new();
            for directory in ["target/generated-sources", "target/generated-test-sources"] {
                let root = preflight.project.join(directory);
                if root.is_dir() && !java_sources(&root)?.is_empty() {
                    boundaries.push("JAVA_GENERATED_DECLARATIONS_NOT_INDEXED".into());
                }
            }
            let mut processor_paths = preflight
                .processor_coordinates
                .iter()
                .map(|coordinate| resolve_annotation_processor_coordinate(&local_repo, coordinate))
                .collect::<Result<Vec<_>, ClewError>>()?;
            for coordinate in extra_annotation_processors {
                let resolved = resolve_annotation_processor_coordinate(&local_repo, coordinate)?;
                if !processor_paths.contains(&resolved) {
                    processor_paths.push(resolved);
                }
            }
            let mut compiler_options = vec![format!("--release={release}")];
            if !preflight.processor_names.is_empty() {
                compiler_options.push(format!(
                    "-processor:{}",
                    preflight.processor_names.join(",")
                ));
            }
            for path in &processor_paths {
                if !classpath_paths.contains(path) {
                    classpath_paths.push(path.clone());
                }
            }
            if processor_paths.is_empty() && !preflight.processor_names.is_empty() {
                boundaries.push("JAVA_PROCESSOR_PATH_UNRESOLVED".into());
            }
            let model = canonical_model(
                repository,
                &preflight.selector,
                JavaBuildSystem::Maven,
                source_paths,
                classpath_paths,
                java,
                javac,
                release,
                compiler_options,
                processor_paths,
                boundaries,
            )?;
            captured.push(MavenCaptured {
                index: *index,
                source_digests,
                classpath_paths: model.classpath_paths.clone(),
                model,
            });
        }
    }
    for capture in &captured {
        let preflight = &preflights[capture.index];
        let compilation = &capture.model.authority.compilation;
        if java_sources(&preflight.source_root)? != capture.model.source_paths {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                format!("Java source membership changed after Maven cohort capture: {compilation}"),
            ));
        }
        if source_digest_authority(&capture.model.source_paths)? != capture.source_digests {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                format!("Java source bytes changed after Maven cohort capture: {compilation}"),
            ));
        }
        if classpath_authority_sequence(&capture.classpath_paths)?
            != capture.model.authority.classpath
        {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                format!("Java classpath bytes changed after Maven cohort capture: {compilation}"),
            ));
        }
    }
    captured.sort_by_key(|capture| capture.index);
    Ok(captured.into_iter().map(|capture| capture.model).collect())
}

fn remove_stale_classpath(path: &Path) -> Result<(), ClewError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            fs::remove_file(path).map_err(io_error)?;
        }
        Ok(_) => {
            return Err(unsupported(
                "Maven classpath output path is not a regular file",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    Ok(())
}

fn source_digest_authority(paths: &[PathBuf]) -> Result<BTreeMap<PathBuf, String>, ClewError> {
    paths
        .iter()
        .map(|path| {
            Ok((
                path.clone(),
                hash_file(path).map_err(|_| {
                    ClewError::new(
                        ErrorCode::InputMutated,
                        "Java source bytes changed during Maven model extraction",
                    )
                })?,
            ))
        })
        .collect()
}

fn classpath_authority_sequence(
    paths: &[PathBuf],
) -> Result<Vec<JavaClasspathAuthority>, ClewError> {
    paths
        .iter()
        .map(|path| {
            classpath_authority(path).map_err(|_| {
                ClewError::new(
                    ErrorCode::InputMutated,
                    "Java classpath bytes changed during Maven model extraction",
                )
            })
        })
        .collect()
}

fn validate_maven_build_layout(project: &Path, effective_xml: &str) -> Result<(), ClewError> {
    let document = roxmltree::Document::parse(effective_xml)
        .map_err(|_| unsupported("Maven Java effective model is invalid XML"))?;
    let root = document.root_element();
    if !root.has_tag_name("project") {
        return Err(unsupported(
            "Maven Java effective model must select one project",
        ));
    }
    let build = root
        .children()
        .find(|node| node.has_tag_name("build"))
        .ok_or_else(|| unsupported("Maven Java effective build model is unavailable"))?;
    for (name, expected) in [
        ("directory", "target"),
        ("sourceDirectory", "src/main/java"),
        ("testSourceDirectory", "src/test/java"),
        ("outputDirectory", "target/classes"),
        ("testOutputDirectory", "target/test-classes"),
    ] {
        let actual = build
            .children()
            .find(|node| node.has_tag_name(name))
            .and_then(|node| node.text())
            .map(str::trim);
        if actual.map(Path::new) != Some(project.join(expected).as_path()) {
            return Err(unsupported(&format!(
                "Maven Java effective build.{name} must resolve to the selected module's {expected}; plugin output directories do not control this check"
            )));
        }
    }
    let plugins = build.children().find(|node| node.has_tag_name("plugins"));
    if plugins
        .into_iter()
        .flat_map(|node| node.children())
        .any(|plugin| {
            plugin.has_tag_name("plugin")
                && plugin.children().any(|node| {
                    node.has_tag_name("artifactId")
                        && node.text().map(str::trim) == Some("maven-toolchains-plugin")
                })
        })
    {
        return Err(unsupported(
            "Maven Java analysis requires the selected process JDK; the effective build includes maven-toolchains-plugin",
        ));
    }
    Ok(())
}

/// Parse processor declarations without resolving artifacts. Resolution waits
/// until the cohort build has completed so Maven may materialize the artifacts.
fn maven_compiler_processor_declarations(
    effective_xml: &str,
) -> Result<(Vec<String>, Vec<String>), ClewError> {
    let document = roxmltree::Document::parse(effective_xml)
        .map_err(|_| unsupported("Maven Java effective model is invalid XML"))?;
    let root = document.root_element();
    let Some(configuration) = root
        .children()
        .find(|node| node.has_tag_name("build"))
        .into_iter()
        .flat_map(|node| node.children())
        .find(|node| node.has_tag_name("plugins"))
        .into_iter()
        .flat_map(|node| node.children())
        .find(|plugin| {
            plugin.has_tag_name("plugin")
                && plugin.children().any(|node| {
                    node.has_tag_name("artifactId")
                        && node.text().map(str::trim) == Some("maven-compiler-plugin")
                })
        })
        .into_iter()
        .flat_map(|plugin| plugin.children())
        .find(|node| node.has_tag_name("configuration"))
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    let names = configuration
        .children()
        .find(|node| node.has_tag_name("annotationProcessors"))
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name("annotationProcessor"))
        .filter_map(|node| node.text())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    let mut coordinates = Vec::new();
    for path in configuration
        .children()
        .find(|node| node.has_tag_name("annotationProcessorPaths"))
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name("path"))
    {
        let child_text = |tag: &str| {
            path.children()
                .find(|node| node.has_tag_name(tag))
                .and_then(|node| node.text())
                .map(str::trim)
                .filter(|text| !text.is_empty())
        };
        if let (Some(group), Some(artifact), Some(version)) = (
            child_text("groupId"),
            child_text("artifactId"),
            child_text("version"),
        ) {
            coordinates.push(format!("{group}:{artifact}:{version}"));
        }
    }
    Ok((names, coordinates))
}

#[cfg(test)]
fn maven_compiler_processor_authority(
    effective_xml: &str,
    local_repo: &Path,
) -> Result<(Vec<String>, Vec<PathBuf>), ClewError> {
    let (names, coordinates) = maven_compiler_processor_declarations(effective_xml)?;
    let paths = coordinates
        .iter()
        .map(|coordinate| resolve_annotation_processor_coordinate(local_repo, coordinate))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((names, paths))
}

/// Resolve a per-service admitted annotation-processor Maven coordinate
/// (`group:artifact:version`) to its owned jar in the local repository. Used so
/// a module relying on a classpath-discovered processor (e.g. Lombok) can be
/// analyzed without editing its pom.xml.
fn resolve_annotation_processor_coordinate(
    local_repo: &Path,
    coordinate: &str,
) -> Result<PathBuf, ClewError> {
    let mut parts = coordinate.split(':');
    let group = parts.next().unwrap_or_default();
    let artifact = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if group.is_empty() || artifact.is_empty() || version.is_empty() || parts.next().is_some() {
        return Err(invalid(&format!(
            "annotation processor coordinate must be group:artifact:version: {coordinate}"
        )));
    }
    let local_repo = local_repo
        .canonicalize()
        .map_err(|_| unsupported("Maven local repository is unavailable"))?;
    let artifact_path = local_repo
        .join(group.replace('.', "/"))
        .join(artifact)
        .join(version)
        .join(format!("{artifact}-{version}.jar"));
    let resolved = artifact_path.canonicalize().map_err(|_| unsupported(&format!(
        "per-service annotation processor artifact is unavailable in the local repository: {coordinate}"
    )))?;
    if !resolved.starts_with(local_repo) || !resolved.is_file() {
        return Err(unsupported(&format!(
            "per-service annotation processor artifact escapes the local repository: {coordinate}"
        )));
    }
    Ok(resolved)
}

/// Resolve the effective Maven local repository from the pinned settings file
/// (`<localRepository>`) or fall back to the standard user repository, so
/// annotationProcessorPaths artifacts can be resolved to owned paths.
fn maven_local_repository(settings_path: Option<&Path>) -> Result<PathBuf, ClewError> {
    if let Some(settings_path) = settings_path {
        let bytes = std::fs::read(settings_path)
            .map_err(|_| unsupported("Maven settings are unreadable"))?;
        if let Ok(document) = roxmltree::Document::parse(
            std::str::from_utf8(&bytes).map_err(|_| unsupported("Maven settings are not UTF-8"))?,
        ) && let Some(local) = document
            .root_element()
            .children()
            .find(|node| node.has_tag_name("localRepository"))
            .and_then(|node| node.text())
        {
            let local = local.trim();
            if !local.is_empty() {
                let resolved = PathBuf::from(local)
                    .canonicalize()
                    .map_err(|_| unsupported("Maven settings localRepository is unavailable"))?;
                return Ok(resolved);
            }
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| unsupported("Java Maven local repository is unavailable"))?;
    Ok(home.join(".m2/repository"))
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
    release: u16,
    compiler_options: Vec<String>,
    annotation_processor_paths: Vec<PathBuf>,
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
    let compiler_major = javac_major(&compiler_version)
        .ok_or_else(|| unsupported("JDK compiler version authority is invalid"))?;
    validate_release(release, compiler_major)?;
    let mut classpath = Vec::with_capacity(classpath_paths.len());
    for path in &classpath_paths {
        classpath.push(classpath_authority(path)?);
    }
    // Explicit processor authority comes only from admitted compiler options
    // (or the native Maven compiler-plugin configuration surfaced as such).
    // Never infer processors from classpath presence alone. We extract the
    // -processor/-processor: value pairs from the RAW option sequence so that
    // an option followed by its value is never reordered into a different
    // meaning by sorting.
    let mut annotation_processors = Vec::new();
    let mut option_iter = compiler_options.iter().peekable();
    while let Some(option) = option_iter.next() {
        if let Some(names) = option.strip_prefix("-processor:") {
            annotation_processors.extend(
                names
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
            );
        } else if option == "-processor"
            && let Some(names) = option_iter.peek()
        {
            annotation_processors.extend(
                names
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    // The stored option sequence is sorted/deduped for canonical identity, but
    // the extracted processor names already captured the value pairs above.
    let mut compiler_options = compiler_options;
    compiler_options.sort();
    compiler_options.dedup();
    // Processor-path artifacts admitted for annotation processing. Their
    // content digests participate in model identity. The artifacts are also
    // available to the analyzer through the emitted classpath manifest.
    let mut annotation_processor_paths = annotation_processor_paths;
    annotation_processor_paths.sort();
    annotation_processor_paths.dedup();
    let mut annotation_processor_authority = Vec::with_capacity(annotation_processor_paths.len());
    for path in &annotation_processor_paths {
        annotation_processor_authority.push(classpath_authority(path)?);
    }
    annotation_processor_authority.sort();
    annotation_processor_authority.dedup();
    boundaries.sort();
    boundaries.dedup();
    annotation_processors.sort();
    annotation_processors.dedup();
    let mut authority = JavaProjectModel {
        schema: JAVA_MODEL_SCHEMA.into(),
        model_digest: String::new(),
        build_system,
        compilation: selector.canonical(),
        source_files,
        classpath,
        release,
        compiler_version,
        compiler_options,
        annotation_processors,
        annotation_processor_paths: annotation_processor_authority,
        boundaries,
    };
    authority.model_digest = canonical::hash(&authority).map_err(internal)?;
    verify_model(&authority)?;
    Ok(JavaOperationalModel {
        authority,
        source_paths,
        classpath_paths,
        annotation_processor_paths,
        java_executable,
    })
}

pub fn verify_model(model: &JavaProjectModel) -> Result<(), ClewError> {
    if model.schema != JAVA_MODEL_SCHEMA
        || javac_major(&model.compiler_version)
            .is_none_or(|major| validate_release(model.release, major).is_err())
        || model.source_files.is_empty()
        || model.source_files.len() > MAX_JAVA_SOURCES
        || model.source_files.windows(2).any(|pair| pair[0] >= pair[1])
        || model.classpath.len() > MAX_CLASSPATH_ENTRIES
        || model
            .annotation_processor_paths
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || model
            .compiler_options
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || model
            .annotation_processors
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

pub(crate) fn classpath_authority(path: &Path) -> Result<JavaClasspathAuthority, ClewError> {
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

fn javac_major(version: &str) -> Option<u16> {
    let number = version.strip_prefix("javac ")?.split_whitespace().next()?;
    let major = number.split(['.', '-', '+']).next()?;
    if major.is_empty() || !major.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    major.parse().ok()
}

fn validate_release(release: u16, compiler_major: u16) -> Result<(), ClewError> {
    if release < crate::analysis_modules::JAVA_MIN_MAJOR
        || compiler_major < crate::analysis_modules::JAVA_MIN_MAJOR
    {
        return Err(unsupported(
            "Java analysis requires release 17 and JDK 17 or newer",
        ));
    }
    if release > compiler_major {
        return Err(unsupported(
            "Java release exceeds the selected JDK compiler version",
        ));
    }
    Ok(())
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

// Logs are not model payloads. Drain both pipes to EOF concurrently, retaining
// only the bounded stdout needed by stdout-based model protocols. Never infer
// process failure from output size, encoding, or a broken protocol frame.
#[derive(Debug)]
struct BuildStream {
    retained: Vec<u8>,
    bytes: u64,
    diagnostic_tail: crate::maven_diagnostics::Tail,
}

fn drain_build_stream(
    mut reader: impl Read,
    retain: bool,
    retain_diagnostics: bool,
) -> std::io::Result<BuildStream> {
    let mut stream = BuildStream {
        retained: Vec::new(),
        bytes: 0,
        diagnostic_tail: crate::maven_diagnostics::Tail::default(),
    };
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(stream);
        }
        stream.bytes = stream.bytes.saturating_add(count as u64);
        if retain {
            let remaining = MAX_MODEL_OUTPUT_BYTES.saturating_sub(stream.retained.len());
            stream
                .retained
                .extend_from_slice(&buffer[..count.min(remaining)]);
        }
        if retain_diagnostics {
            stream.diagnostic_tail.append(&buffer[..count]);
        }
    }
}

fn discard_build_output(
    command: &mut Command,
    message: &str,
    stage: &str,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<(), ClewError> {
    run_build_command(command, message, false, stage, debug_output).map(|_| ())
}

fn bounded_output(
    command: &mut Command,
    message: &str,
    stage: &str,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<String, ClewError> {
    run_build_command(command, message, true, stage, debug_output)
}

fn run_build_command(
    command: &mut Command,
    message: &str,
    retain: bool,
    stage: &str,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<String, ClewError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| build_command_failure(message, "BUILD_LAUNCHER_START_FAILED", "Verify the build launcher is executable and available in the same terminal or agent PATH; check JAVA_HOME."))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let capture_diagnostics = debug_output.is_some();
    let (status, stdout, stderr) = std::thread::scope(|scope| {
        let stdout = scope.spawn(move || drain_build_stream(stdout, retain, capture_diagnostics));
        let stderr = scope.spawn(move || drain_build_stream(stderr, false, capture_diagnostics));
        let status = child.wait();
        (status, stdout.join(), stderr.join())
    });
    let status = status.map_err(|_| {
        build_command_failure(
            message,
            "BUILD_PROCESS_WAIT_FAILED",
            "The native exit status is unavailable.",
        )
    })?;
    let read_error = || {
        build_command_failure(
            message,
            "BUILD_OUTPUT_READ_FAILED",
            &format!(
                "Native status: {status}. Could not drain build output; no model was accepted."
            ),
        )
    };
    let stdout = stdout
        .map_err(|_| read_error())?
        .map_err(|_| read_error())?;
    let stderr = stderr
        .map_err(|_| read_error())?
        .map_err(|_| read_error())?;
    let measurements = format!(
        "Native status: {status}; stdoutBytes={}; stderrBytes={}; stdoutRetainedBytes={}; stdoutLimitBytes={MAX_MODEL_OUTPUT_BYTES}.",
        stdout.bytes,
        stderr.bytes,
        stdout.retained.len()
    );
    if !status.success() {
        let error = build_command_failure(
            message,
            "BUILD_COMMAND_FAILED",
            &format!(
                "{measurements} Resolve the native build's first error (including dependency access, credentials or JDK configuration), then retry Codeclew. Build output is omitted because it may contain private data."
            ),
        );
        return Err(crate::maven_diagnostics::annotate_failure(
            error,
            debug_output,
            stage,
            &status,
            &stdout.diagnostic_tail,
            &stderr.diagnostic_tail,
        ));
    }
    if retain && stdout.bytes > MAX_MODEL_OUTPUT_BYTES as u64 {
        return Err(resource(&format!(
            "BUILD_MODEL_OUTPUT_LIMIT: {message}. {measurements} The build succeeded but its stdout model exceeds the supported size. Select a narrower module/compilation and retry.",
        )));
    }
    String::from_utf8(stdout.retained).map_err(|_| unsupported(&format!("BUILD_MODEL_OUTPUT_ENCODING: {message}. {measurements} The build succeeded but its stdout model is not UTF-8.")))
}

fn build_command_failure(stage: &str, reason: &str, action: &str) -> ClewError {
    let diagnostic = if stage.starts_with("Maven") {
        let goals = if stage.contains("effective model") {
            "-B -q -N help:effective-pom -Doutput=<temporary-file>"
        } else if stage.contains("compilation and classpath") {
            "-B -q compile dependency:build-classpath (test-compile for a test compilation)"
        } else {
            "-q -DforceStdout help:evaluate -Dexpression=maven.compiler.release"
        };
        format!(
            "Reproduce this stage from the repository root with the same JAVA_HOME, PATH, Maven settings and repository mvnw (or Maven on PATH). Check mvn --version. Run {goals} with -DskipTests -Dstyle.color=never -Dmdep.outputFile=target/codeclew-classpath.txt -Dmdep.regenerateFile=true -Dmdep.includeScope=compile (test for a test compilation). For compilation use -f <repository-pom> and, for a selected submodule, -pl <module> -am; for effective model/release use -f <selected-module-pom>. A successful dependency:build-classpath/help:evaluate alone does not verify compilation or effective model extraction."
        )
    } else {
        "From the repository root run ./gradlew --version and ./gradlew --stacktrace tasks --all."
            .into()
    };
    unsupported(&format!("{reason}: {stage}. {action} {diagnostic}"))
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

    fn effective_maven_fixture(project: &Path) -> String {
        format!(
            r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><build>
                <directory>{0}/target</directory>
                <sourceDirectory>{0}/src/main/java</sourceDirectory>
                <testSourceDirectory>{0}/src/test/java</testSourceDirectory>
                <outputDirectory>{0}/target/classes</outputDirectory>
                <testOutputDirectory>{0}/target/test-classes</testOutputDirectory>
                <plugins><plugin><artifactId>spring-boot-maven-plugin</artifactId>
                    <configuration><outputDirectory>web-build</outputDirectory></configuration>
                </plugin></plugins>
                <!-- <outputDirectory>ignored</outputDirectory> maven-toolchains-plugin -->
            </build></project>"#,
            project.display()
        )
    }

    #[test]
    fn maven_processor_authority_admits_explicit_paths_and_names() {
        let repo = tempfile::tempdir().unwrap();
        let local = repo.path().join("repo");
        let artifact = local.join("org/projectlombok/lombok/1.18.38/lombok-1.18.38.jar");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::write(&artifact, b"lombok-bytes").unwrap();
        let xml = r#"
            <project xmlns="http://maven.apache.org/POM/4.0.0">
              <modelVersion>4.0.0</modelVersion>
              <groupId>dev.codeclew.fixture</groupId><artifactId>processor-layout</artifactId><version>1</version>
              <build>
                <plugins>
                  <plugin>
                    <artifactId>maven-compiler-plugin</artifactId>
                    <version>3.13.0</version>
                    <configuration>
                      <annotationProcessorPaths>
                        <path><groupId>org.projectlombok</groupId><artifactId>lombok</artifactId><version>1.18.38</version></path>
                      </annotationProcessorPaths>
                      <annotationProcessors>
                        <annotationProcessor>lombok.launch.AnnotationProcessorHider$AnnotationProcessor</annotationProcessor>
                      </annotationProcessors>
                    </configuration>
                  </plugin>
                </plugins>
              </build>
            </project>"#;
        let (names, paths) = maven_compiler_processor_authority(xml, &local).unwrap();
        assert_eq!(
            names,
            vec!["lombok.launch.AnnotationProcessorHider$AnnotationProcessor"]
        );
        assert_eq!(paths, vec![artifact.canonicalize().unwrap()]);
    }

    #[test]
    fn maven_processor_paths_missing_in_local_repo_are_an_explicit_boundary() {
        let repo = tempfile::tempdir().unwrap();
        let local = repo.path().join("repo");
        fs::create_dir_all(&local).unwrap();
        let xml = r#"
            <project xmlns="http://maven.apache.org/POM/4.0.0">
              <modelVersion>4.0.0</modelVersion>
              <groupId>dev.codeclew.fixture</groupId><artifactId>processor-layout</artifactId><version>1</version>
              <build><plugins><plugin>
                <artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version>
                <configuration><annotationProcessorPaths>
                  <path><groupId>org.projectlombok</groupId><artifactId>lombok</artifactId><version>9.9.9</version></path>
                </annotationProcessorPaths></configuration>
              </plugin></plugins></build>
            </project>"#;
        let error = maven_compiler_processor_authority(xml, &local).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
        assert!(
            error
                .message
                .contains("unavailable in the local repository"),
            "{error}"
        );
    }

    #[test]
    fn maven_processor_authority_is_empty_when_no_compiler_plugin() {
        let repo = tempfile::tempdir().unwrap();
        let xml = r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion></project>"#;
        let (names, paths) = maven_compiler_processor_authority(xml, repo.path()).unwrap();
        assert!(names.is_empty());
        assert!(paths.is_empty());
    }

    #[test]
    fn maven_layout_uses_effective_build_paths_not_plugin_configuration() {
        let project = Path::new("/selected/module");
        let xml = effective_maven_fixture(project);
        validate_maven_build_layout(project, &xml).unwrap();
        for (field, expected) in [
            ("directory", "target"),
            ("sourceDirectory", "src/main/java"),
            ("testSourceDirectory", "src/test/java"),
            ("outputDirectory", "target/classes"),
            ("testOutputDirectory", "target/test-classes"),
        ] {
            let changed = xml.replace(
                &format!("<{field}>/selected/module/{expected}</{field}>"),
                &format!("<{field}>/private/custom-layout</{field}>"),
            );
            let error = validate_maven_build_layout(project, &changed).unwrap_err();
            assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
            assert!(error.message.contains(&format!("build.{field}")));
            assert!(!error.message.contains("/private/custom-layout"));
        }
    }

    #[test]
    fn maven_layout_rejects_missing_authority_and_active_toolchains() {
        let project = Path::new("/selected/module");
        for xml in ["<project/>", "<projects/>", "not xml"] {
            assert!(validate_maven_build_layout(project, xml).is_err());
        }
        let xml = effective_maven_fixture(project)
            .replace("spring-boot-maven-plugin", "maven-toolchains-plugin");
        let error = validate_maven_build_layout(project, &xml).unwrap_err();
        assert!(error.message.contains("selected process JDK"));
    }

    #[test]
    fn missing_build_launcher_has_a_recovery_command() {
        let missing = tempfile::tempdir().unwrap();
        let error = super::bounded_output(
            &mut Command::new(missing.path().join("missing-launcher")),
            "Maven Java classpath extraction failed",
            "COMPILE_CLASSPATH",
            None,
        )
        .unwrap_err();
        assert!(error.message.starts_with("BUILD_LAUNCHER_START_FAILED"));
        assert!(error.message.contains("mvn --version"));
        assert!(error.message.contains("same terminal or agent PATH"));
        assert!(!error.message.contains(missing.path().to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn native_build_failure_preserves_action_without_private_output() {
        let error = super::bounded_output(
            Command::new("/bin/sh").args([
                "-c",
                "echo 'https://private.invalid?token=secret' >&2; exit 1",
            ]),
            "Gradle Java model extraction failed",
            "GRADLE_MODEL",
            None,
        )
        .unwrap_err();
        assert!(error.message.starts_with("BUILD_COMMAND_FAILED"));
        assert!(error.message.contains("./gradlew --stacktrace tasks --all"));
        assert!(!error.message.contains("private.invalid"));
        assert!(!error.message.contains("secret"));
    }

    #[cfg(unix)]
    #[test]
    fn build_logs_are_drained_but_never_treated_as_model_or_failure() {
        let noise = "head -c 5000000 /dev/zero; head -c 6000000 /dev/zero >&2";
        discard_build_output(
            Command::new("/bin/sh").args(["-c", noise]),
            "Maven Java compilation and classpath extraction failed",
            "COMPILE_CLASSPATH",
            None,
        )
        .unwrap();
        // stderr is a log even when stdout carries the release/model protocol.
        assert_eq!(
            bounded_output(
                Command::new("/bin/sh").args(["-c", "head -c 6000000 /dev/zero >&2; printf 17"]),
                "Maven Java release extraction failed",
                "RELEASE",
                None,
            )
            .unwrap(),
            "17"
        );
        let error = bounded_output(
            Command::new("/bin/sh").args(["-c", noise]),
            "Maven Java release extraction failed",
            "RELEASE",
            None,
        )
        .unwrap_err();
        assert!(error.message.starts_with("BUILD_MODEL_OUTPUT_LIMIT"));
        assert!(
            error
                .message
                .contains("stdoutBytes=5000000; stderrBytes=6000000; stdoutRetainedBytes=4194304")
        );
        assert!(error.message.contains("exit status: 0"));
        let error = bounded_output(
            Command::new("/bin/sh").args(["-c", &format!("{noise}; exit 7")]),
            "Maven Java compilation and classpath extraction failed",
            "COMPILE_CLASSPATH",
            None,
        )
        .unwrap_err();
        assert!(error.message.starts_with("BUILD_COMMAND_FAILED"));
        assert!(error.message.contains("exit status: 7"));
        assert!(
            error
                .message
                .contains("stdoutBytes=5000000; stderrBytes=6000000")
        );
        assert!(error.message.contains("-pl <module> -am"));
        assert!(error.message.contains("compile dependency:build-classpath"));
        assert!(error.message.contains("repository root"));
    }

    #[cfg(unix)]
    #[test]
    fn build_signal_and_model_encoding_remain_distinct_from_output_limits() {
        let error = bounded_output(
            Command::new("/bin/sh").args(["-c", "kill -TERM $$"]),
            "Maven Java release extraction failed",
            "RELEASE",
            None,
        )
        .unwrap_err();
        assert!(error.message.starts_with("BUILD_COMMAND_FAILED"));
        assert!(error.message.contains("signal"));
        let error = bounded_output(
            Command::new("/bin/sh").args(["-c", "printf '\\377'"]),
            "Maven Java release extraction failed",
            "RELEASE",
            None,
        )
        .unwrap_err();
        assert!(error.message.starts_with("BUILD_MODEL_OUTPUT_ENCODING"));
        assert!(error.message.contains("exit status: 0"));
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "launches real Maven and JDK 21 with a public fixture and large wrapper logs"]
    fn maven_large_logs_and_native_compile_failure_are_distinguished() {
        let repository = tempfile::tempdir().unwrap();
        let root = repository.path();
        let native = crate::maven::command(root).unwrap();
        let settings = root.join("settings.xml");
        fs::write(&settings, "<settings/>").unwrap();
        let settings = crate::maven::MavenSettings::capture(&settings).unwrap();
        fs::write(root.join("pom.xml"), r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
          <modelVersion>4.0.0</modelVersion><groupId>dev.codeclew.fixture</groupId><artifactId>large-logs</artifactId><version>1</version>
          <properties><maven.compiler.release>17</maven.compiler.release></properties>
          <dependencies><dependency><groupId>com.google.code.findbugs</groupId><artifactId>jsr305</artifactId><version>3.0.2</version></dependency></dependencies>
          <build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin></plugins></build>
        </project>"#).unwrap();
        let source = root.join("src/main/java/App.java");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "public class App {}").unwrap();
        // Non-executable wrapper exercises interpreter launch and repository cwd.
        // Emit binary logs only for compilation, leaving the release protocol intact.
        let launcher = native
            .get_program()
            .to_str()
            .unwrap()
            .replace('\'', "'\"'\"'");
        fs::write(root.join("mvnw"), format!("#!/bin/sh\n[ -f pom.xml ] || exit 91\ncase \"$*\" in *dependency:build-classpath*) head -c 5000000 /dev/zero; head -c 6000000 /dev/zero >&2;; esac\nexec '{launcher}' \"$@\"\n")).unwrap();
        let model = extract_java_model_with_settings(root, ":/main", Some(&settings)).unwrap();
        assert_eq!(model.authority.release, 17);
        assert!(model.authority.compiler_version.starts_with("javac 21"));
        let classpath_bytes = fs::metadata(root.join("target/codeclew-classpath.txt"))
            .unwrap()
            .len();
        let jar_bytes: u64 = model
            .authority
            .classpath
            .iter()
            .filter(|entry| entry.kind == "FILE")
            .map(|entry| entry.size)
            .sum();
        println!(
            "public Maven fixture: wrapper stdoutBytes=5000000 stderrBytes=6000000 classpathTextBytes={classpath_bytes} jarBytes={jar_bytes}"
        );
        assert!(classpath_bytes > 0 && classpath_bytes < 4096);
        // The old recovery command succeeds even with a source compilation error.
        fs::write(&source, "public class App { MissingType field; }").unwrap();
        let output = native_command_for_test(root, &settings)
            .args([
                "-q",
                "-DskipTests",
                "dependency:build-classpath",
                "help:evaluate",
                "-Dexpression=maven.compiler.release",
            ])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success());
        let error = extract_java_model_with_settings(root, ":/main", Some(&settings)).unwrap_err();
        assert!(
            error.message.starts_with(
                "BUILD_COMMAND_FAILED: Maven Java compilation and classpath extraction failed"
            ),
            "{error}"
        );
        assert!(error.message.contains("exit status: 1"));
        assert!(!error.message.contains("MissingType"));
        assert!(!error.message.contains(root.to_str().unwrap()));
    }

    #[cfg(unix)]
    fn native_command_for_test(root: &Path, settings: &crate::maven::MavenSettings) -> Command {
        // Resolve PATH Maven in a directory without the noisy wrapper.
        let directory = tempfile::tempdir().unwrap();
        let mut command = crate::maven::command(directory.path()).unwrap();
        command
            .arg("--settings")
            .arg(&settings.path)
            .arg("-f")
            .arg(root.join("pom.xml"));
        command
    }

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
    fn java_release_is_preserved_and_bounded_by_observed_compiler() {
        for (release, compiler) in [(17, 17), (17, 21), (21, 21), (25, 25)] {
            assert!(validate_release(release, compiler).is_ok());
        }
        for (release, compiler) in [(8, 21), (16, 17), (17, 16), (22, 21)] {
            assert!(validate_release(release, compiler).is_err());
        }
        assert_eq!(javac_major("javac 17.0.12"), Some(17));
        assert_eq!(javac_major("javac 21.0.8"), Some(21));
        assert_eq!(javac_major("javac 25-ea"), Some(25));
        assert_eq!(javac_major("javac 210.0.1"), Some(210));
        assert_eq!(javac_major("javac 21invalid"), None);
        assert_eq!(javac_major("java 21.0.1"), None);
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
            21,
            vec!["--release=21".into()],
            vec![],
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

    #[test]
    #[ignore = "qualification resolves a property-activated native Maven profile"]
    fn maven_effective_profile_matches_native_compile_properties() {
        let repository = tempfile::tempdir().unwrap();
        fs::write(repository.path().join("pom.xml"), r#"
            <project xmlns="http://maven.apache.org/POM/4.0.0">
              <modelVersion>4.0.0</modelVersion>
              <groupId>dev.codeclew.fixture</groupId><artifactId>profile-layout</artifactId><version>1</version>
              <properties>
                <maven.compiler.release>21</maven.compiler.release>
                <fixture.output>${project.basedir}/target/classes</fixture.output>
              </properties>
              <build><outputDirectory>${fixture.output}</outputDirectory></build>
              <profiles><profile><id>native-compile-layout</id>
                <activation><property><name>skipTests</name></property></activation>
                <properties><fixture.output>${project.basedir}/alternate-classes</fixture.output></properties>
              </profile></profiles>
            </project>"#).unwrap();
        let error = extract_java_model(repository.path(), ":/main").unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
        assert!(error.message.contains("build.outputDirectory"), "{error}");
        assert!(!repository.path().join("alternate-classes").exists());
    }

    #[test]
    fn per_service_annotation_processor_coordinate_resolves_from_local_repo() {
        let local_repo = tempfile::tempdir().unwrap();
        let jar = local_repo
            .path()
            .join("org/example/processor/1.2.3/processor-1.2.3.jar");
        fs::create_dir_all(jar.parent().unwrap()).unwrap();
        fs::write(&jar, "jar-bytes").unwrap();

        let resolved = resolve_annotation_processor_coordinate(
            local_repo.path(),
            "org.example:processor:1.2.3",
        )
        .unwrap();
        assert_eq!(resolved, jar.canonicalize().unwrap());
        assert!(resolved.is_file());

        // A coordinate whose artifact is absent from the local repository is
        // rejected rather than silently ignored.
        let missing = resolve_annotation_processor_coordinate(
            local_repo.path(),
            "org.example:processor:9.9.9",
        )
        .unwrap_err();
        assert!(
            missing
                .message
                .contains("unavailable in the local repository")
        );

        // Malformed coordinates are rejected before any filesystem access.
        for bad in ["org.example", "org.example:proc:", ":proc:1.0", "a:b:c:d"] {
            let err = resolve_annotation_processor_coordinate(local_repo.path(), bad).unwrap_err();
            assert!(err.message.contains("group:artifact:version"), "{err}");
        }
    }

    #[test]
    fn processor_repository_uses_materialized_settings_after_original_changes() {
        let workspace = tempfile::tempdir().unwrap();
        let original_repo = workspace.path().join("admitted");
        let changed_repo = workspace.path().join("changed");
        fs::create_dir_all(&original_repo).unwrap();
        fs::create_dir_all(&changed_repo).unwrap();
        let path = workspace.path().join("settings.xml");
        let write = |repo: &Path| {
            fs::write(
                &path,
                format!(
                    "<settings><localRepository>{}</localRepository></settings>",
                    repo.display()
                ),
            )
            .unwrap()
        };
        write(&original_repo);
        let settings = crate::maven::MavenSettings::capture(&path).unwrap();
        let pinned = settings.materialize().unwrap();
        write(&changed_repo);
        assert_eq!(
            maven_local_repository(Some(pinned.path())).unwrap(),
            original_repo.canonicalize().unwrap()
        );
        assert_eq!(
            settings.materialize().unwrap_err().code,
            ErrorCode::InputMutated
        );
    }

    #[cfg(unix)]
    fn generated_source_maven_fixture(
        mode: &str,
    ) -> (tempfile::TempDir, crate::maven::MavenSettings) {
        use std::os::unix::fs::PermissionsExt;

        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        fs::create_dir_all(root.join("src/main/java/example")).unwrap();
        fs::create_dir_all(root.join("src/test/java/example")).unwrap();
        fs::create_dir_all(root.join("local-repository")).unwrap();
        fs::write(root.join("pom.xml"), "<project/>").unwrap();
        fs::write(root.join("fixture-mode"), mode).unwrap();
        fs::write(
            root.join(".gitignore"),
            "/src/main/java/example/Generated.java\n/target/\n",
        )
        .unwrap();
        fs::write(
            root.join("src/main/java/example/Main.java"),
            "package example; class Main { Generated value; }",
        )
        .unwrap();
        fs::write(
            root.join("src/test/java/example/Test.java"),
            "package example; class Test {}",
        )
        .unwrap();
        let settings_path = root.join("settings.xml");
        fs::write(
            &settings_path,
            format!(
                "<settings><localRepository>{}</localRepository></settings>",
                root.join("local-repository").display()
            ),
        )
        .unwrap();
        let settings = crate::maven::MavenSettings::capture(&settings_path).unwrap();
        let wrapper = r#"#!/bin/sh
set -eu
mode=$(cat fixture-mode)
case "$*" in
  *help:effective-pom*)
    output=
    for argument in "$@"; do
      case "$argument" in -Doutput=*) output=${argument#-Doutput=} ;; esac
    done
    project=$(pwd -P)
    cat > "$output" <<EOF
<project><build>
<directory>$project/target</directory>
<sourceDirectory>$project/src/main/java</sourceDirectory>
<testSourceDirectory>$project/src/test/java</testSourceDirectory>
<outputDirectory>$project/target/classes</outputDirectory>
<testOutputDirectory>$project/target/test-classes</testOutputDirectory>
</build></project>
EOF
    ;;
  *dependency:build-classpath*)
    mkdir -p target/classes
    printf '%s\n' 'package example; class Generated {}' > src/main/java/example/Generated.java
    printf '%s\n' 'stable-class-bytes' > target/classes/Main.class
    : > target/codeclew-classpath.txt
    case "$*" in *test-compile*)
      case "$mode" in
        membership-test) printf '%s\n' 'package example; class Late {}' > src/main/java/example/Late.java ;;
        source-test) printf '%s\n' 'package example; class Main { int changed; }' > src/main/java/example/Main.java ;;
        classpath-test) printf '%s\n' 'changed-class-bytes' > target/classes/Main.class ;;
      esac
      ;;
    esac
    if test "$mode" = "target-generated"; then
      mkdir -p target/generated-sources/example
      printf '%s\n' 'package example; class TargetGenerated {}' > target/generated-sources/example/TargetGenerated.java
    fi
    ;;
  *help:evaluate*)
    if test "$mode" = "source-during-model"; then
      printf '%s\n' 'package example; class Main { int changed; }' > src/main/java/example/Main.java
    fi
    printf '17\n'
    ;;
  *) exit 2 ;;
esac
"#;
        fs::write(root.join("mvnw"), wrapper).unwrap();
        fs::set_permissions(root.join("mvnw"), fs::Permissions::from_mode(0o700)).unwrap();
        (workspace, settings)
    }

    #[cfg(unix)]
    #[test]
    fn maven_batch_captures_generated_source_membership_after_build() {
        let (workspace, settings) = generated_source_maven_fixture("stable");
        assert!(
            !workspace
                .path()
                .join("src/main/java/example/Generated.java")
                .exists()
        );
        let selected = vec![":/main".into(), ":/test".into()];
        let models = extract_java_models_with_settings_and_diagnostics(
            workspace.path(),
            &selected,
            Some(&settings),
            None,
            &[],
        )
        .unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0].authority.source_files,
            [
                "src/main/java/example/Generated.java",
                "src/main/java/example/Main.java"
            ]
        );
        assert_eq!(
            models[1].authority.source_files,
            ["src/test/java/example/Test.java"]
        );
        let repeated = extract_java_models_with_settings_and_diagnostics(
            workspace.path(),
            &selected,
            Some(&settings),
            None,
            &[],
        )
        .unwrap();
        assert_eq!(models[0].authority, repeated[0].authority);
        assert_eq!(models[1].authority, repeated[1].authority);
    }

    #[cfg(unix)]
    #[test]
    fn maven_batch_rejects_membership_source_and_classpath_drift_after_capture() {
        for (mode, reason) in [
            ("membership-test", "source membership"),
            ("source-test", "source bytes"),
            ("classpath-test", "classpath bytes"),
            ("source-during-model", "source bytes"),
        ] {
            let (workspace, settings) = generated_source_maven_fixture(mode);
            let error = extract_java_models_with_settings_and_diagnostics(
                workspace.path(),
                &[":/main".into(), ":/test".into()],
                Some(&settings),
                None,
                &[],
            )
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::InputMutated, "{mode}: {error}");
            assert!(error.message.contains(reason), "{mode}: {error}");
            assert!(error.message.contains(":/main"), "{mode}: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn maven_target_generated_sources_remain_an_explicit_boundary() {
        let (workspace, settings) = generated_source_maven_fixture("target-generated");
        let model =
            extract_java_model_with_settings(workspace.path(), ":/main", Some(&settings)).unwrap();
        assert!(
            model
                .authority
                .source_files
                .iter()
                .all(|path| !path.starts_with("target/"))
        );
        assert!(
            model
                .authority
                .boundaries
                .iter()
                .any(|code| code == "JAVA_GENERATED_DECLARATIONS_NOT_INDEXED")
        );
    }

    #[cfg(unix)]
    #[test]
    fn maven_batch_preflights_all_scopes_and_batches_same_source_set() {
        use std::os::unix::fs::PermissionsExt;

        let workspace = tempfile::tempdir().unwrap();
        let repository = workspace.path();
        let log = workspace.path().join("maven.log");
        let local_repo = workspace.path().join("local-repository");
        fs::create_dir_all(&local_repo).unwrap();
        let settings_path = workspace.path().join("settings.xml");
        fs::write(
            &settings_path,
            format!(
                "<settings><localRepository>{}</localRepository></settings>",
                local_repo.display()
            ),
        )
        .unwrap();
        let settings = crate::maven::MavenSettings::capture(&settings_path).unwrap();
        fs::write(repository.join("pom.xml"), "<project/>").unwrap();
        for module in ["common", "service", "api"] {
            let project = repository.join(module);
            fs::create_dir_all(project.join("src/main/java/example")).unwrap();
            fs::write(project.join("pom.xml"), "<project/>").unwrap();
            fs::write(
                project.join(format!("src/main/java/example/{module}.java")),
                format!("package example; class {module} {{}}"),
            )
            .unwrap();
        }
        let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
        let wrapper = format!(
            r#"#!/bin/sh
set -eu
LOG={log}
printf '%s\n' "$*" >> "$LOG"
pom=
previous=
for argument in "$@"; do
  if test "$previous" = "-f"; then pom=$argument; fi
  previous=$argument
done
case "$*" in
  *help:effective-pom*)
    output=
    for argument in "$@"; do
      case "$argument" in -Doutput=*) output=${{argument#-Doutput=}} ;; esac
    done
    project=$(CDPATH= cd -- "$(dirname "$pom")" && pwd -P)
    cat > "$output" <<EOF
<project><build>
<directory>$project/target</directory>
<sourceDirectory>$project/src/main/java</sourceDirectory>
<testSourceDirectory>$project/src/test/java</testSourceDirectory>
<outputDirectory>$project/target/classes</outputDirectory>
<testOutputDirectory>$project/target/test-classes</testOutputDirectory>
</build></project>
EOF
    ;;
  *dependency:build-classpath*)
    modules=.
    previous=
    for argument in "$@"; do
      if test "$previous" = "-pl"; then modules=$argument; fi
      previous=$argument
    done
    old_ifs=$IFS
    IFS=,
    for module in $modules; do
      IFS=$old_ifs
      if test "$module" = "."; then project=$(pwd -P); else project=$(pwd -P)/$module; fi
      mkdir -p "$project/target"
      : > "$project/target/codeclew-classpath.txt"
      IFS=,
    done
    IFS=$old_ifs
    ;;
  *help:evaluate*) printf '17\n' ;;
  *) exit 2 ;;
esac
"#,
            log = quote(&log),
        );
        let wrapper_path = repository.join("mvnw");
        fs::write(&wrapper_path, wrapper).unwrap();
        fs::set_permissions(&wrapper_path, fs::Permissions::from_mode(0o700)).unwrap();

        let selected = vec![
            ":api/main".to_owned(),
            ":common/main".to_owned(),
            ":service/main".to_owned(),
        ];
        let models = extract_java_models_with_settings_and_diagnostics(
            repository,
            &selected,
            Some(&settings),
            None,
            &[],
        )
        .unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.authority.compilation.as_str())
                .collect::<Vec<_>>(),
            vec![":api/main", ":common/main", ":service/main"]
        );
        let calls = fs::read_to_string(&log).unwrap();
        assert_eq!(calls.matches("help:effective-pom").count(), 3);
        assert_eq!(calls.matches("dependency:build-classpath").count(), 1);
        assert_eq!(calls.matches("help:evaluate").count(), 3);
        assert!(calls.contains("-pl api,common,service -am"), "{calls}");
        assert!(
            repository
                .join("common/target/codeclew-classpath.txt")
                .is_file()
        );

        fs::write(&log, b"").unwrap();
        let error = extract_java_models_with_settings_and_diagnostics(
            repository,
            &[":common/main".into(), ":missing/main".into()],
            Some(&settings),
            None,
            &[],
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
        let calls = fs::read_to_string(&log).unwrap();
        assert!(!calls.contains("dependency:build-classpath"), "{calls}");
    }
}
