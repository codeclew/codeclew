use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::java_project_model::{JavaOperationalModel, JavaProjectModel, verify_model};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub const JAVA_LANGUAGE: &str = "language:java";
pub const JAVA_COMPILER_FACTS_CAPABILITY: &str = "analysis:java-compiler-facts";
pub const JAVA_INDEX_SCHEMA: &str = "codeclew-java-compiler-index/1.0";
pub const JAVA_FACT_SCHEMA: &str = "codeclew-java-compiler-fact/1.0";
const JAVA_RECEIPT_SCHEMA: &str = "codeclew-java-compiler-completeness/1.0";
const JAVA_ADAPTER_AUTHORITY_SCHEMA: &str = "codeclew-java-compiler-adapter/1.0";
const JAVA_ANALYZER_SOURCE: &str = include_str!("java_analyzer.java");
const MAX_ANALYZER_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_JAVA_FACTS: usize = 262_144;
const MAX_FACT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum JavaCompilerFact {
    SourceFile {
        schema: String,
        file: String,
        source_content_digest: String,
        resolution: String,
    },
    Declaration {
        schema: String,
        declaration_kind: String,
        symbol_identity: String,
        owner_identity: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        jvm_descriptor: Option<String>,
        modifiers: Vec<String>,
        annotations: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spring: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        interfaces: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        superclass: Option<String>,
        file: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end: Option<u64>,
        resolution: String,
    },
    Relation {
        schema: String,
        relation_kind: String,
        source_identity: String,
        target_identity: String,
        file: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end: Option<u64>,
        resolution: String,
    },
    Boundary {
        schema: String,
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostic_code: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end: Option<u64>,
        required_checks: Vec<String>,
        resolution: String,
    },
}

impl JavaCompilerFact {
    fn schema(&self) -> &str {
        match self {
            Self::SourceFile { schema, .. }
            | Self::Declaration { schema, .. }
            | Self::Relation { schema, .. }
            | Self::Boundary { schema, .. } => schema,
        }
    }

    fn path(&self) -> Option<&str> {
        match self {
            Self::SourceFile { file, .. }
            | Self::Declaration { file, .. }
            | Self::Relation { file, .. } => Some(file),
            Self::Boundary { file, .. } => file.as_deref(),
        }
    }

    fn is_boundary(&self) -> bool {
        matches!(self, Self::Boundary { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaCompilerIndex {
    pub schema: String,
    pub compilation: String,
    pub model: JavaProjectModel,
    pub analyzer_digest: String,
    pub facts: Vec<JavaCompilerFact>,
}

pub fn java_adapter_digest() -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":JAVA_ADAPTER_AUTHORITY_SCHEMA,
        "indexSchema":JAVA_INDEX_SCHEMA,
        "factSchema":JAVA_FACT_SCHEMA,
        "capability":JAVA_COMPILER_FACTS_CAPABILITY,
        "analyzerDigest":canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
        "jdkCompilerApi":"jdk.compiler/17+",
    }))
    .map_err(internal)
}

pub fn build_java_compiler_index(
    repository: &Path,
    operational: &JavaOperationalModel,
    source_content_digests: &BTreeMap<String, String>,
) -> Result<JavaCompilerIndex, ClewError> {
    verify_model(&operational.authority)?;
    let repository = repository.canonicalize().map_err(io_error)?;
    let temporary = tempfile::tempdir().map_err(io_error)?;
    let analyzer = temporary.path().join("CodeclewJavaAnalyzer.java");
    let sources = temporary.path().join("sources.txt");
    let classpath = temporary.path().join("classpath.txt");
    fs::write(&analyzer, JAVA_ANALYZER_SOURCE).map_err(io_error)?;
    fs::write(
        &sources,
        manifest_lines(
            operational
                .authority
                .source_files
                .iter()
                .map(String::as_str),
        )?,
    )
    .map_err(io_error)?;
    fs::write(
        &classpath,
        manifest_lines(
            operational
                .classpath_paths
                .iter()
                .map(|path| path.to_str().unwrap_or("")),
        )?,
    )
    .map_err(io_error)?;
    let output = Command::new(&operational.java_executable)
        .args([
            "--source",
            "17",
            analyzer
                .to_str()
                .ok_or_else(|| internal("Java analyzer path is not UTF-8"))?,
            repository
                .to_str()
                .ok_or_else(|| unsupported("Java repository path is not UTF-8"))?,
            sources
                .to_str()
                .ok_or_else(|| internal("Java source manifest path is not UTF-8"))?,
            classpath
                .to_str()
                .ok_or_else(|| internal("Java classpath manifest path is not UTF-8"))?,
            &operational.authority.release.to_string(),
        ])
        .current_dir(&repository)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| unsupported("Java compiler analyzer could not start"))?;
    if !output.status.success()
        || output.stdout.len() > MAX_ANALYZER_OUTPUT_BYTES
        || output.stderr.len() > MAX_ANALYZER_OUTPUT_BYTES
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "Java compiler analyzer did not produce bounded facts",
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| corrupt("Java compiler facts are not UTF-8"))?;
    let mut facts = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            if line.len() > MAX_FACT_BYTES {
                return Err(resource("Java compiler fact exceeds its byte budget"));
            }
            serde_json::from_str::<JavaCompilerFact>(line)
                .map_err(|_| corrupt("Java compiler fact schema is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if facts.len() > MAX_JAVA_FACTS {
        return Err(resource(
            "Java compiler fact count exceeds its bounded profile",
        ));
    }
    for (path, digest) in source_content_digests {
        facts.push(JavaCompilerFact::SourceFile {
            schema: JAVA_FACT_SCHEMA.into(),
            file: path.clone(),
            source_content_digest: digest.clone(),
            resolution: "SOURCE_MEMBERSHIP_EXACT".into(),
        });
    }
    facts.sort_by_cached_key(|fact| canonical::bytes(fact).expect("serializable Java fact"));
    facts.dedup();
    let index = JavaCompilerIndex {
        schema: JAVA_INDEX_SCHEMA.into(),
        compilation: operational.authority.compilation.clone(),
        model: operational.authority.clone(),
        analyzer_digest: canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
        facts,
    };
    validate_index(&index)?;
    Ok(index)
}

pub fn java_scope_digest(index: &JavaCompilerIndex) -> Result<String, ClewError> {
    validate_index(index)?;
    let facts_digest = canonical::hash(&index.facts).map_err(internal)?;
    canonical::hash(&json!({
        "schema":"codeclew-java-compiler-scope/1.0",
        "compilation":index.compilation,
        "modelDigest":index.model.model_digest,
        "analyzerDigest":index.analyzer_digest,
        "factsDigest":facts_digest,
        "factCount":index.facts.len(),
    }))
    .map_err(internal)
}

pub struct JavaAdapterV2 {
    adapter_digest: String,
    toolchain_digest: String,
    compilation_id: String,
    store: CasStore,
    index: JavaCompilerIndex,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl JavaAdapterV2 {
    pub fn new(
        adapter_digest: String,
        toolchain_digest: String,
        compilation_id: String,
        store: CasStore,
        index: JavaCompilerIndex,
    ) -> Result<Self, ClewError> {
        validate_index(&index)?;
        if !digest(&adapter_digest)
            || !digest(&toolchain_digest)
            || compilation_id.is_empty()
            || compilation_id.len() > 120
        {
            return Err(invalid("Java adapter authority is invalid"));
        }
        Ok(Self {
            adapter_digest,
            toolchain_digest,
            compilation_id,
            store,
            index,
            cancelled_attempts: Mutex::new(BTreeSet::new()),
            stopped: AtomicBool::new(false),
        })
    }
}

impl LanguageAdapter for JavaAdapterV2 {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
        Ok(AdapterHandshake {
            protocol: ADAPTER_PROTOCOL.into(),
            adapter_id: "java-compiler-1".into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(JAVA_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(JAVA_COMPILER_FACTS_CAPABILITY)?],
            toolchains: vec![ToolchainConstraint {
                authority_digest: self.toolchain_digest.clone(),
                minimum_version: Some("21".into()),
                maximum_version_exclusive: Some("22".into()),
            }],
        })
    }

    fn analyze_generation(
        &self,
        request: &AnalyzeGenerationRequest,
        sink: &mut dyn AnalysisSink,
        cancelled: &AtomicBool,
    ) -> Result<(), ClewError> {
        if self.stopped.load(Ordering::Acquire)
            || cancelled.load(Ordering::Acquire)
            || self
                .cancelled_attempts
                .lock()
                .map_err(poisoned)?
                .contains(&request.attempt_id)
        {
            return Err(cancelled_error());
        }
        if request.compilation.language_uri.as_str() != JAVA_LANGUAGE
            || request.capability.as_str() != JAVA_COMPILER_FACTS_CAPABILITY
            || request.compilation.toolchain.digest != self.toolchain_digest
            || request.compilation.compilation_id != self.compilation_id
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "Java request differs from its compiler authority",
            ));
        }
        let capability = CapabilityUri::parse(JAVA_COMPILER_FACTS_CAPABILITY)?;
        let mut records = Vec::with_capacity(self.index.facts.len());
        for fact in &self.index.facts {
            let bytes = canonical::bytes(fact).map_err(internal)?;
            let payload = self.store.put(JAVA_FACT_SCHEMA, &bytes)?;
            records.push(FactRecord {
                fact_key: format!(
                    "java:{}",
                    canonical::hash_bytes(&bytes).trim_start_matches("sha256:")
                ),
                domain_uri: capability.clone(),
                payload,
            });
        }
        records.sort_by(|left, right| left.fact_key.cmp(&right.fact_key));
        for (sequence, chunk) in records.chunks(1024).enumerate() {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: u32::try_from(sequence)
                    .map_err(|_| resource("Java fact shard sequence overflow"))?,
                facts: chunk.to_vec(),
            }))?;
        }
        let scope_digest = java_scope_digest(&self.index)?;
        let boundaries = self
            .index
            .facts
            .iter()
            .filter(|fact| fact.is_boundary())
            .count();
        let receipt = self.store.put(
            JAVA_RECEIPT_SCHEMA,
            &canonical::bytes(&json!({
                "schema":JAVA_RECEIPT_SCHEMA,
                "scopeDigest":scope_digest,
                "coverage":if boundaries == 0 { "COMPLETE_SUPPORTED_SUBSET" } else { "PARTIAL" },
                "certainty":if boundaries == 0 { "VERIFIED" } else { "UNSURE" },
                "boundaryCount":boundaries,
                "obligations":if boundaries == 0 { Vec::<String>::new() } else { vec!["FIX_JAVA_CLASSPATH_OR_DIAGNOSTIC".to_owned()] },
            }))
            .map_err(internal)?,
        )?;
        sink.accept(AnalysisEvent::AttemptComplete(AnalysisAttemptComplete {
            scope_digest,
            completeness_receipt: receipt,
            fact_count: records.len() as u64,
        }))
    }

    fn cancel(&self, attempt_id: &str) -> Result<(), ClewError> {
        if attempt_id.is_empty() || attempt_id.len() > 128 {
            return Err(invalid("Java attempt identity is invalid"));
        }
        self.cancelled_attempts
            .lock()
            .map_err(poisoned)?
            .insert(attempt_id.into());
        Ok(())
    }

    fn shutdown(&self) -> Result<(), ClewError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
}

fn validate_index(index: &JavaCompilerIndex) -> Result<(), ClewError> {
    verify_model(&index.model)?;
    if index.schema != JAVA_INDEX_SCHEMA
        || index.compilation != index.model.compilation
        || index.analyzer_digest != canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes())
        || index.facts.len() > MAX_JAVA_FACTS
    {
        return Err(corrupt("Java compiler index authority is invalid"));
    }
    let mut previous = None;
    for fact in &index.facts {
        if fact.schema() != JAVA_FACT_SCHEMA
            || fact.path().is_some_and(|path| !safe_relative_path(path))
        {
            return Err(corrupt("Java compiler fact authority is invalid"));
        }
        if let JavaCompilerFact::Declaration {
            spring: Some(spring),
            ..
        } = fact
        {
            crate::spring_entrypoints::validate_metadata(spring, "JAVAC_RESOLVED_ANNOTATIONS")?;
        }
        let bytes = canonical::bytes(fact).map_err(internal)?;
        if bytes.len() > MAX_FACT_BYTES
            || previous.as_ref().is_some_and(|previous| previous >= &bytes)
        {
            return Err(corrupt("Java compiler facts are not canonical"));
        }
        previous = Some(bytes);
    }
    Ok(())
}

fn manifest_lines<'a>(values: impl Iterator<Item = &'a str>) -> Result<Vec<u8>, ClewError> {
    let mut bytes = Vec::new();
    for value in values {
        if value.is_empty() || value.contains(['\n', '\r', '\0']) {
            return Err(invalid("Java analyzer manifest contains an invalid path"));
        }
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn cancelled_error() -> ClewError {
    ClewError::new(
        ErrorCode::IncompleteSemanticAnalysis,
        "Java analysis was cancelled",
    )
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
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

fn poisoned<T>(error: std::sync::PoisonError<T>) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java_project_model::extract_java_model;

    #[test]
    #[ignore = "qualification launches the JDK compiler analyzer"]
    fn gradle_and_maven_fixtures_have_exact_facts_without_private_paths() {
        let workspace = crate::worker::workspace_root();
        for fixture in ["java-gradle", "java-maven"] {
            let repository = workspace.join("fixtures").join(fixture);
            let model = extract_java_model(&repository, ":/main").unwrap();
            let digests = model
                .authority
                .source_files
                .iter()
                .map(|path| {
                    let bytes = fs::read(repository.join(path)).unwrap();
                    (path.clone(), canonical::hash_bytes(&bytes))
                })
                .collect();
            let index = build_java_compiler_index(&repository, &model, &digests).unwrap();
            assert!(index.facts.iter().any(|fact| matches!(
                fact,
                JavaCompilerFact::Declaration { symbol_identity, .. }
                    if symbol_identity.contains("example.Service")
            )));
            assert!(index.facts.iter().any(|fact| matches!(
                fact,
                JavaCompilerFact::Relation { relation_kind, target_identity, .. }
                    if relation_kind == "CALLS" && target_identity.contains("Gateway#load")
            )));
            assert!(!index.facts.iter().any(JavaCompilerFact::is_boundary));
            let encoded = serde_json::to_string(&index).unwrap();
            assert!(!encoded.contains(workspace.to_str().unwrap()));
        }
    }

    #[test]
    #[ignore = "qualification launches project-native Gradle and the JDK compiler analyzer"]
    fn compiler_error_emits_only_typed_boundaries() {
        let workspace = crate::worker::workspace_root();
        let source = workspace.join("fixtures/java-gradle");
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        for entry in walkdir::WalkDir::new(&source) {
            let entry = entry.unwrap();
            let relative = entry.path().strip_prefix(&source).unwrap();
            let target = repository.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir_all(target).unwrap();
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
        fs::write(
            repository.join("src/main/java/example/Service.java"),
            b"package example; public final class Service { MissingType value; }",
        )
        .unwrap();
        let model = extract_java_model(&repository, ":/main").unwrap();
        let digests = model
            .authority
            .source_files
            .iter()
            .map(|path| {
                let bytes = fs::read(repository.join(path)).unwrap();
                (path.clone(), canonical::hash_bytes(&bytes))
            })
            .collect();
        let index = build_java_compiler_index(&repository, &model, &digests).unwrap();
        assert!(!index.facts.is_empty());
        assert!(index.facts.iter().all(|fact| matches!(
            fact,
            JavaCompilerFact::Boundary { .. } | JavaCompilerFact::SourceFile { .. }
        )));
        assert!(index.facts.iter().any(|fact| matches!(
            fact,
            JavaCompilerFact::Boundary {
                code,
                diagnostic_code: Some(_),
                required_checks,
                resolution,
                ..
            } if code == "JAVA_COMPILER_DIAGNOSTIC"
                && required_checks == &["FIX_JAVA_CLASSPATH_OR_DIAGNOSTIC"]
                && resolution == "UNKNOWN"
        )));
        let encoded = serde_json::to_string(&index).unwrap();
        assert!(!encoded.contains(temporary.path().to_str().unwrap()));
    }

    #[test]
    #[ignore = "qualification launches the JDK compiler analyzer"]
    fn spring_entrypoints_use_resolved_annotations_on_java_17_and_21() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repository");
        fs::create_dir_all(&root).unwrap();
        let annotations = [
            (
                "org.springframework.stereotype",
                "Controller",
                "String value() default \"\";",
                "",
            ),
            (
                "org.springframework.web.bind.annotation",
                "RestController",
                "",
                "@org.springframework.stereotype.Controller",
            ),
            (
                "org.springframework.web.bind.annotation",
                "RequestMapping",
                "String[] value() default {}; String[] path() default {}; RequestMethod[] method() default {}; String[] produces() default {};",
                "",
            ),
            (
                "org.springframework.web.bind.annotation",
                "GetMapping",
                "String[] value() default {}; String[] path() default {};",
                "",
            ),
            (
                "org.springframework.core.annotation",
                "AliasFor",
                "String value() default \"\"; String attribute() default \"\"; Class<? extends java.lang.annotation.Annotation> annotation() default java.lang.annotation.Annotation.class;",
                "",
            ),
            (
                "org.springframework.kafka.annotation",
                "KafkaListener",
                "String[] topics() default {}; String groupId() default \"\";",
                "@java.lang.annotation.Repeatable(KafkaListeners.class)",
            ),
            (
                "org.springframework.kafka.annotation",
                "KafkaListeners",
                "KafkaListener[] value();",
                "",
            ),
            (
                "org.springframework.kafka.annotation",
                "KafkaHandler",
                "boolean isDefault() default false;",
                "",
            ),
            (
                "org.springframework.scheduling.annotation",
                "Scheduled",
                "String cron() default \"\"; long fixedDelay() default -1; long fixedRate() default -1;",
                "@java.lang.annotation.Repeatable(Schedules.class)",
            ),
            (
                "org.springframework.scheduling.annotation",
                "Schedules",
                "Scheduled[] value();",
                "",
            ),
        ];
        let mut sources = Vec::new();
        for (package, name, members, meta) in annotations {
            let relative = format!("{}/{name}.java", package.replace('.', "/"));
            let file = root.join(&relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(
                file,
                format!("package {package}; {meta} public @interface {name} {{ {members} }}"),
            )
            .unwrap();
            sources.push(relative);
        }
        let fixtures = [
            (
                "org/springframework/web/bind/annotation/RequestMethod.java",
                "package org.springframework.web.bind.annotation; public enum RequestMethod { GET, POST }",
            ),
            (
                "example/Handlers.java",
                r#"
package example;
import org.springframework.web.bind.annotation.*;
import org.springframework.kafka.annotation.*;
import org.springframework.scheduling.annotation.*;
import org.springframework.core.annotation.AliasFor;
@RequestMapping(method = RequestMethod.POST, produces = "application/json")
@interface PostJson {
    @AliasFor(annotation = RequestMapping.class, attribute = "path") String[] value();
}
@RequestMapping(method = RequestMethod.GET)
@interface JsonGet {
    @AliasFor(annotation = RequestMapping.class, attribute = "path") String[] route() default {};
}
@JsonGet
@interface Route {
    @AliasFor(annotation = JsonGet.class, attribute = "route") String[] value();
}
@RequestMapping(path = "/default")
@interface ValueRoute {
    @AliasFor(annotation = RequestMapping.class, attribute = "value") String[] value();
}
interface Api { @GetMapping("/inherited") String inherited(int id); }
@RestController @RequestMapping("/v1") @KafkaListener(topics = "events")
class Handlers implements Api {
    static final String PATH = "/items";
    @GetMapping(PATH) String load() { return ""; }
    @GetMapping(path = "/items/{id}") String load(int id) { return ""; }
    @PostJson("/composed") void post() {}
    @Route("/multi-level") void multiLevel() {}
    @ValueRoute("/override") void valueAlias() {}
    @Override public String inherited(int id) { return ""; }
    @KafkaListener(topics = {"one", "two"}, groupId = "${group}")
    @KafkaListener(topics = "three") void consume(String message) {}
    @KafkaHandler(isDefault = true) void dispatch(Object message) {}
    @Scheduled(fixedDelay = 1000 * 60) @Scheduled(cron = "0 * * * * *") void tick() {}
}
@interface Scheduled { long fixedRate() default 0; }
class Impostor { @example.Scheduled(fixedRate = 1) void fake() {} }
@RestController @RequestMapping("/child") @KafkaListener(topics = "child-events")
class InheritedHandlers extends Handlers { @Override String load(int id) { return ""; } }
abstract class AbstractInherited extends Handlers {}
"#,
            ),
        ];
        // The impostor uses a distinct simple name in another package so the real import remains unambiguous.
        for (relative, body) in fixtures {
            let file = root.join(relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(
                file,
                body.replace(
                    "@Scheduled(fixedDelay",
                    "@org.springframework.scheduling.annotation.Scheduled(fixedDelay",
                )
                .replace(
                    "@Scheduled(cron",
                    "@org.springframework.scheduling.annotation.Scheduled(cron",
                ),
            )
            .unwrap();
            sources.push(relative.into());
        }
        let analyzer = temp.path().join("CodeclewJavaAnalyzer.java");
        fs::write(&analyzer, JAVA_ANALYZER_SOURCE).unwrap();
        let manifest = temp.path().join("sources.txt");
        fs::write(&manifest, sources.join("\n")).unwrap();
        let classpath = temp.path().join("classpath.txt");
        fs::write(&classpath, "").unwrap();
        for release in ["17", "21"] {
            let java = std::env::var_os("JAVA_HOME")
                .map(|home| std::path::PathBuf::from(home).join("bin/java"))
                .unwrap_or_else(|| "java".into());
            let output = Command::new(java)
                .arg("--source")
                .arg("17")
                .arg(&analyzer)
                .arg(&root)
                .arg(&manifest)
                .arg(&classpath)
                .arg(release)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let facts: Vec<JavaCompilerFact> = String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let boundaries: Vec<_> = facts
                .iter()
                .filter_map(|fact| match fact {
                    JavaCompilerFact::Boundary { code, .. } => Some(code.as_str()),
                    _ => None,
                })
                .collect();
            assert!(boundaries.is_empty(), "{boundaries:?}");
            let spring_for = |suffix: &str| {
                facts
                    .iter()
                    .find_map(|fact| match fact {
                        JavaCompilerFact::Declaration {
                            symbol_identity,
                            spring: Some(spring),
                            file,
                            start: Some(_),
                            end: Some(_),
                            ..
                        } if symbol_identity.ends_with(suffix) => {
                            assert_eq!(file, "example/Handlers.java");
                            assert_eq!(spring["authority"], "JAVAC_RESOLVED_ANNOTATIONS");
                            Some(spring)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {suffix}"))
            };
            let load = spring_for("Handlers#load()Ljava/lang/String;");
            assert_eq!(load["entries"][0]["attributes"]["path"], json!(["/items"]));
            assert_eq!(
                load["entries"][0]["classAttributes"][0]["path"],
                json!(["/v1"])
            );
            assert_eq!(load["entries"][0]["controller"], true);
            assert_eq!(load["entries"][0]["registration"], "RUNTIME_CONDITIONAL");
            assert_eq!(
                spring_for("Handlers#load(I)Ljava/lang/String;")["entries"][0]["attributes"]["path"],
                json!(["/items/{id}"])
            );
            let post = spring_for("Handlers#post()V");
            assert_eq!(
                post["entries"][0]["attributes"]["path"],
                json!(["/composed"])
            );
            assert_eq!(post["entries"][0]["attributes"]["method"], json!(["POST"]));
            assert_eq!(
                post["entries"][0]["annotationChain"],
                json!([
                    "example.PostJson",
                    "org.springframework.web.bind.annotation.RequestMapping"
                ])
            );
            assert_eq!(
                spring_for("Handlers#inherited(I)Ljava/lang/String;")["entries"][0]["attributes"]["path"],
                json!(["/inherited"])
            );
            let multi_level = spring_for("Handlers#multiLevel()V");
            assert_eq!(
                multi_level["entries"][0]["attributes"]["path"],
                json!(["/multi-level"])
            );
            assert_eq!(
                multi_level["entries"][0]["attributes"]["method"],
                json!(["GET"])
            );
            assert!(
                multi_level["entries"][0]["attributes"]
                    .get("route")
                    .is_none()
            );
            assert_eq!(
                multi_level["entries"][0]["annotationChain"],
                json!([
                    "example.Route",
                    "example.JsonGet",
                    "org.springframework.web.bind.annotation.RequestMapping"
                ])
            );
            assert_eq!(
                spring_for("Handlers#valueAlias()V")["entries"][0]["attributes"]["path"],
                json!(["/override"])
            );
            let consume = spring_for("Handlers#consume(Ljava/lang/String;)V");
            assert_eq!(consume["entries"].as_array().unwrap().len(), 2);
            assert_eq!(consume["entries"][0]["kind"], "KAFKA_LISTENER");
            assert!(
                consume["boundaries"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("RUNTIME_EXPRESSION"))
            );
            assert_eq!(
                spring_for("Handlers#dispatch(Ljava/lang/Object;)V")["entries"][0]["handlerAttributes"]
                    ["isDefault"],
                true
            );
            let scheduled = spring_for("Handlers#tick()V");
            assert_eq!(scheduled["entries"].as_array().unwrap().len(), 2);
            assert_eq!(scheduled["entries"][0]["attributes"]["fixedDelay"], 60000);
            assert_eq!(spring_for("Impostor#fake()V")["entries"], json!([]));
            let inherited = spring_for("class:example.InheritedHandlers");
            let inherited_entries = inherited["entries"].as_array().unwrap();
            assert!(!inherited_entries.is_empty());
            assert!(
                inherited_entries
                    .iter()
                    .all(|entry| entry["beanClass"] == "class:example.InheritedHandlers")
            );
            let inherited_http = inherited_entries
                .iter()
                .find(|entry| {
                    entry["targetSymbol"]
                        == "method:class:example.Handlers#load()Ljava/lang/String;"
                })
                .unwrap();
            assert_eq!(
                inherited_http["classAttributes"][0]["path"],
                json!(["/child"])
            );
            assert!(!inherited_entries.iter().any(|entry| entry["targetSymbol"]
                == "method:class:example.Handlers#load(I)Ljava/lang/String;"));
            let inherited_kafka = inherited_entries
                .iter()
                .find(|entry| {
                    entry["targetSymbol"]
                        == "method:class:example.Handlers#dispatch(Ljava/lang/Object;)V"
                })
                .unwrap();
            assert_eq!(
                inherited_kafka["attributes"]["topics"],
                json!(["child-events"])
            );
            assert_eq!(inherited_kafka["handlerAttributes"]["isDefault"], true);
            assert_eq!(
                spring_for("class:example.AbstractInherited")["entries"],
                json!([])
            );
        }
    }

    #[test]
    fn analyzer_source_and_adapter_authority_are_content_bound() {
        assert!(digest(&java_adapter_digest().unwrap()));
        assert_eq!(
            canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
            canonical::hash_bytes(include_str!("java_analyzer.java").as_bytes())
        );
    }
}
