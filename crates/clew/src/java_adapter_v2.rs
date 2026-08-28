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
        "jdkCompilerApi":"jdk.compiler/21",
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
            "21",
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
            "21",
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
    fn analyzer_source_and_adapter_authority_are_content_bound() {
        assert!(digest(&java_adapter_digest().unwrap()));
        assert_eq!(
            canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
            canonical::hash_bytes(include_str!("java_analyzer.java").as_bytes())
        );
    }
}
