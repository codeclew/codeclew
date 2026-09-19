use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::incremental_v2::{
    COMPLETENESS_VECTOR_SCHEMA, Certainty, CompletenessVector, Coverage, Support,
    VerificationObligation,
};
use crate::java_analysis_inputs::PreparedJavaAnalysisInputs;
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
pub const JAVA_ANALYZER_SOURCE: &str = include_str!("java_analyzer.java");
pub(crate) const MAX_ANALYZER_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qualified_name: Option<String>,
        symbol_identity: String,
        owner_identity: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        jvm_descriptor: Option<String>,
        modifiers: Vec<String>,
        annotations: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spring: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jvm_annotations: Option<Box<serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        documentation: Option<Box<serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        interfaces: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        superclass: Option<String>,
        file: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_line: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_line: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_end: Option<u64>,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        start_line: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_line: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_end: Option<u64>,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        start_line: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_line: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_end: Option<u64>,
        required_checks: Vec<String>,
        resolution: String,
    },
    AnnotationRegistry {
        schema: String,
        authority: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        definitions: BTreeMap<String, serde_json::Value>,
    },
}

impl JavaCompilerFact {
    fn schema(&self) -> &str {
        match self {
            Self::SourceFile { schema, .. }
            | Self::Declaration { schema, .. }
            | Self::Relation { schema, .. }
            | Self::Boundary { schema, .. }
            | Self::AnnotationRegistry { schema, .. } => schema,
        }
    }

    pub(crate) fn path(&self) -> Option<&str> {
        match self {
            Self::SourceFile { file, .. }
            | Self::Declaration { file, .. }
            | Self::Relation { file, .. } => Some(file),
            Self::Boundary { file, .. } => file.as_deref(),
            Self::AnnotationRegistry { .. } => None,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_state: Option<serde_json::Value>,
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
    writable_then_seal: bool,
    before_digests: Option<&BTreeMap<String, String>>,
    changed_files: &[(String, String, String)],
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<JavaCompilerIndex, ClewError> {
    build_java_compiler_index_inner(
        repository,
        operational,
        source_content_digests,
        writable_then_seal,
        before_digests,
        changed_files,
        debug_output,
        None,
    )
}

/// Run the no-processor analyzer against the request-scoped closed inputs.
/// The ordinary path above remains the authority for writable and processor
/// enabled analysis.
pub fn build_java_compiler_index_with_prepared_inputs(
    operational: &JavaOperationalModel,
    prepared: &PreparedJavaAnalysisInputs,
    source_content_digests: &BTreeMap<String, String>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<JavaCompilerIndex, ClewError> {
    prepared.require_sealed()?;
    if !operational.authority.annotation_processors.is_empty()
        || !operational.authority.annotation_processor_paths.is_empty()
        || operational
            .authority
            .compiler_options
            .iter()
            .any(|option| option.starts_with("-A"))
    {
        return Err(unsupported(
            "closed Java analysis inputs cannot execute annotation processors",
        ));
    }
    build_java_compiler_index_inner(
        &prepared.analysis_root,
        operational,
        source_content_digests,
        false,
        None,
        &[],
        debug_output,
        Some(prepared),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_java_compiler_index_inner(
    repository: &Path,
    operational: &JavaOperationalModel,
    source_content_digests: &BTreeMap<String, String>,
    writable_then_seal: bool,
    before_digests: Option<&BTreeMap<String, String>>,
    changed_files: &[(String, String, String)],
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
    prepared: Option<&PreparedJavaAnalysisInputs>,
) -> Result<JavaCompilerIndex, ClewError> {
    let raw = execute_java_compiler(
        repository,
        operational,
        writable_then_seal,
        debug_output,
        prepared,
    )?;
    project_java_compiler_output(
        &raw,
        operational,
        source_content_digests,
        writable_then_seal,
        before_digests,
        changed_files,
    )
}

pub(crate) fn execute_prepared_java_output(
    operational: &JavaOperationalModel,
    prepared: &PreparedJavaAnalysisInputs,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<Vec<u8>, ClewError> {
    prepared.require_sealed()?;
    if !operational.authority.annotation_processors.is_empty()
        || !operational.authority.annotation_processor_paths.is_empty()
        || operational
            .authority
            .compiler_options
            .iter()
            .any(|option| option.starts_with("-A"))
    {
        return Err(unsupported(
            "closed Java analysis inputs cannot execute annotation processors",
        ));
    }
    execute_java_compiler(
        &prepared.analysis_root,
        operational,
        false,
        debug_output,
        Some(prepared),
    )
}

fn execute_java_compiler(
    repository: &Path,
    operational: &JavaOperationalModel,
    writable_then_seal: bool,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
    prepared: Option<&PreparedJavaAnalysisInputs>,
) -> Result<Vec<u8>, ClewError> {
    verify_model(&operational.authority)?;
    let repository = repository.canonicalize().map_err(io_error)?;
    let temporary = tempfile::tempdir().map_err(io_error)?;
    let analyzer = temporary.path().join("CodeclewJavaAnalyzer.java");
    fs::write(&analyzer, JAVA_ANALYZER_SOURCE).map_err(io_error)?;
    let sources = prepared
        .map(|inputs| inputs.source_manifest.clone())
        .unwrap_or_else(|| temporary.path().join("sources.txt"));
    let classpath = prepared
        .map(|inputs| inputs.classpath_manifest.clone())
        .unwrap_or_else(|| temporary.path().join("classpath.txt"));
    if prepared.is_none() {
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
    }
    // Annotation processors run only when explicitly admitted by the project
    // model (explicit `<annotationProcessors>` names OR the resolved
    // `<annotationProcessorPaths>` artifacts) AND the writable profile is
    // active. Their emitted sources/classes are isolated to a disposable
    // directory inside this auto-cleaned tempdir, never the repository tree.
    // Otherwise the analyzer runs with -proc:none so arbitrary classpath-
    // discovered processors cannot run or mutate.
    let admitted = operational.authority.annotation_processors.join(",");
    let admitted_paths = operational
        .annotation_processor_paths
        .iter()
        .map(|path| path.to_str().unwrap_or(""))
        .collect::<Vec<_>>();
    let generated_root = temporary.path().join("generated");
    let (generated_arg, processor_arg, processor_path_arg) =
        if writable_then_seal && (!admitted.is_empty() || !admitted_paths.is_empty()) {
            fs::create_dir_all(&generated_root).map_err(io_error)?;
            (
                generated_root
                    .to_str()
                    .ok_or_else(|| internal("Java generated-source root is not UTF-8"))?
                    .to_owned(),
                admitted,
                std::env::join_paths(&admitted_paths)
                    .map_err(|_| internal("Java processor path is not a valid list"))?
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            (String::new(), String::new(), String::new())
        };
    // Processor options (`-A...`) are surfaced from the admitted model
    // compiler options so an explicitly admitted processor can observe them
    // during its isolated disposable-root execution. They are part of model
    // identity (compiler_options) and only ever reach an admitted processor.
    let processor_options = operational
        .authority
        .compiler_options
        .iter()
        .filter(|option| option.starts_with("-A"))
        .cloned()
        .collect::<Vec<_>>();
    let mut analyzer_args = vec![
        "--source".to_owned(),
        "17".to_owned(),
        analyzer
            .to_str()
            .ok_or_else(|| internal("Java analyzer path is not UTF-8"))?
            .to_owned(),
        repository
            .to_str()
            .ok_or_else(|| unsupported("Java repository path is not UTF-8"))?
            .to_owned(),
        sources
            .to_str()
            .ok_or_else(|| internal("Java source manifest path is not UTF-8"))?
            .to_owned(),
        classpath
            .to_str()
            .ok_or_else(|| internal("Java classpath manifest path is not UTF-8"))?
            .to_owned(),
        operational.authority.release.to_string(),
        generated_arg,
        processor_arg,
        processor_path_arg,
        processor_options.join("\n"),
    ];
    if let Some(inputs) = prepared {
        analyzer_args.push("CLOSED_NO_AP".into());
        analyzer_args.push(
            inputs
                .empty_source_path
                .to_str()
                .ok_or_else(|| internal("Java empty source-path is not UTF-8"))?
                .to_owned(),
        );
    }
    let java_executable = prepared
        .map(|inputs| inputs.java_executable.as_path())
        .unwrap_or(&operational.java_executable);
    let working_dir = prepared
        .map(|inputs| inputs.working_dir.as_path())
        .unwrap_or(repository.as_path());
    let mut command = Command::new(java_executable);
    if let Some(inputs) = prepared {
        command.args([
            "-Dfile.encoding=UTF-8",
            "-Duser.language=en",
            "-Duser.country=US",
            "-Duser.timezone=UTC",
        ]);
        command.arg(format!("-Duser.home={}", inputs.working_dir.display()));
        command.arg(format!("-Djava.io.tmpdir={}", inputs.working_dir.display()));
        command.arg("--class-path").arg(&inputs.empty_source_path);
    }
    command
        .args(&analyzer_args)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(inputs) = prepared {
        let home = inputs
            .java_executable
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| unsupported("prepared Java launcher has no JDK home"))?;
        command
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .env("JAVA_HOME", home)
            .env("TMPDIR", inputs.working_dir.as_os_str())
            .env("PATH", home.join("bin"));
        crate::java_analysis_scratch::attach_child_lease(
            &mut command,
            inputs.duplicate_liveness()?,
        )?;
    }
    let output = command
        .output()
        .map_err(|_| unsupported("Java compiler analyzer could not start"))?;
    if !output.status.success()
        || output.stdout.len() > MAX_ANALYZER_OUTPUT_BYTES
        || output.stderr.len() > MAX_ANALYZER_OUTPUT_BYTES
    {
        // Only allowlisted metadata is ever surfaced publicly. Raw analyzer
        // output is retained solely through the opt-in private diagnostic
        // contract (byte-bounded, caller-owned 0700 directory).
        eprintln!(
            "Java compiler analyzer failed: status={:?} stdout_bytes={} stderr_bytes={}",
            output.status.code(),
            output.stdout.len(),
            output.stderr.len(),
        );
        let mut stdout_tail = crate::maven_diagnostics::Tail::default();
        stdout_tail.append(&output.stdout);
        let mut stderr_tail = crate::maven_diagnostics::Tail::default();
        stderr_tail.append(&output.stderr);
        let error = ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "Java compiler analyzer did not produce bounded facts",
        );
        return Err(crate::maven_diagnostics::annotate_failure(
            error,
            debug_output,
            "JAVA_ANALYZER",
            &output.status,
            &stdout_tail,
            &stderr_tail,
        ));
    }
    Ok(output.stdout)
}

/// Decode successful emitter output before persistence or current-model projection.
/// Source-membership records belong to projection, never to the Java subprocess.
pub(crate) fn parse_java_compiler_output(raw: &[u8]) -> Result<Vec<JavaCompilerFact>, ClewError> {
    if raw.len() > MAX_ANALYZER_OUTPUT_BYTES {
        return Err(resource("Java compiler output exceeds its byte budget"));
    }
    let text =
        std::str::from_utf8(raw).map_err(|_| corrupt("Java compiler facts are not UTF-8"))?;
    let facts = text
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
    for fact in &facts {
        if matches!(fact, JavaCompilerFact::SourceFile { .. }) {
            return Err(corrupt(
                "Java raw output contains projected source membership",
            ));
        }
        validate_fact(fact)?;
    }
    Ok(facts)
}

pub(crate) fn project_java_compiler_output(
    raw: &[u8],
    operational: &JavaOperationalModel,
    source_content_digests: &BTreeMap<String, String>,
    writable_then_seal: bool,
    before_digests: Option<&BTreeMap<String, String>>,
    changed_files: &[(String, String, String)],
) -> Result<JavaCompilerIndex, ClewError> {
    let mut facts = parse_java_compiler_output(raw)?;
    for (path, digest) in source_content_digests {
        facts.push(JavaCompilerFact::SourceFile {
            schema: JAVA_FACT_SCHEMA.into(),
            file: path.clone(),
            source_content_digest: digest.clone(),
            resolution: "SOURCE_MEMBERSHIP_EXACT".into(),
        });
    }
    for code in &operational.authority.boundaries {
        facts.push(JavaCompilerFact::Boundary {
            schema: JAVA_FACT_SCHEMA.into(),
            code: code.clone(),
            diagnostic_code: None,
            line: None,
            file: None,
            start: None,
            end: None,
            start_line: None,
            end_line: None,
            byte_start: None,
            byte_end: None,
            required_checks: vec!["LIMIT_DOCUMENTATION_TO_INDEXED_SOURCE_OBJECTS".into()],
            resolution: "SOURCE_SCOPE_LIMIT".into(),
        });
    }
    facts.sort_by_cached_key(|fact| canonical::bytes(fact).expect("serializable Java fact"));
    facts.dedup();
    let (provenance, source_state) = transformed_index_marker(
        writable_then_seal,
        before_digests,
        source_content_digests,
        changed_files,
    );
    let index = JavaCompilerIndex {
        schema: JAVA_INDEX_SCHEMA.into(),
        compilation: operational.authority.compilation.clone(),
        model: operational.authority.clone(),
        analyzer_digest: canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
        facts,
        provenance,
        source_state,
    };
    validate_index(&index)?;
    Ok(index)
}

/// Assemble the provenance/source_state marker for a writable-then-seal index.
///
/// Pure function so the marker shape can be unit-tested without launching the
/// JDK compiler analyzer. `changed` entries are `(path, before_digest,
/// after_digest)` triples.
fn transformed_index_marker(
    writable_then_seal: bool,
    before: Option<&BTreeMap<String, String>>,
    after: &BTreeMap<String, String>,
    changed: &[(String, String, String)],
) -> (Option<String>, Option<serde_json::Value>) {
    if !writable_then_seal {
        return (None, None);
    }
    let changed_files = changed
        .iter()
        .map(|(path, before, after)| json!({"path": path, "before": before, "after": after}))
        .collect::<Vec<_>>();
    let before_map = before
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    (
        Some("TRANSFORMED_WORKSPACE".to_string()),
        Some(json!({
            "kind": "TRANSFORMED_WORKSPACE",
            "before": before_map,
            "after": after,
            "changedFiles": changed_files,
        })),
    )
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

pub fn java_completeness(
    index: &JavaCompilerIndex,
    scope_digest: &str,
) -> Result<CompletenessVector, ClewError> {
    let mut boundaries = BTreeSet::new();
    let mut checks = BTreeSet::new();
    let mut unsure = false;
    for fact in &index.facts {
        if let JavaCompilerFact::Boundary {
            code,
            required_checks,
            resolution,
            ..
        } = fact
        {
            boundaries.insert(code.clone());
            checks.extend(required_checks.iter().cloned());
            unsure |= resolution != "SOURCE_SCOPE_LIMIT";
        }
    }
    if boundaries.is_empty() {
        return CompletenessVector::verified_complete(scope_digest.into());
    }
    let value = CompletenessVector {
        schema: COMPLETENESS_VECTOR_SCHEMA.into(),
        support: Support::Supported,
        coverage: Coverage::Partial {
            observed_scopes: vec![scope_digest.into()],
            boundaries: boundaries.into_iter().collect(),
        },
        certainty: if unsure {
            Certainty::Unsure {
                check_set: vec!["java-classpath-and-diagnostics".into()],
            }
        } else {
            Certainty::Verified
        },
        obligations: checks
            .into_iter()
            .map(|code| VerificationObligation {
                code,
                subject: vec![scope_digest.into()],
                publication_blocking: true,
            })
            .collect(),
    };
    value.validate()?;
    Ok(value)
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
                minimum_version: Some("17".into()),
                maximum_version_exclusive: None,
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
        let completeness = java_completeness(&self.index, &scope_digest)?;
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
                "certainty":if completeness.certainty == Certainty::Verified { "VERIFIED" } else { "UNSURE" },
                "boundaryCount":boundaries,
                "obligations":completeness.obligations.iter().map(|obligation| &obligation.code).collect::<Vec<_>>(),
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

fn validate_fact(fact: &JavaCompilerFact) -> Result<(), ClewError> {
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
    if let JavaCompilerFact::Declaration {
        jvm_annotations: Some(annotations),
        symbol_identity,
        ..
    } = fact
    {
        crate::spring_entrypoints::validate_annotation_facts(
            annotations,
            Some(symbol_identity),
            "JAVAC_RESOLVED_ANNOTATIONS",
        )?;
    }
    Ok(())
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
        validate_fact(fact)?;
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
            let index =
                build_java_compiler_index(&repository, &model, &digests, false, None, &[], None)
                    .unwrap();
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
        let index =
            build_java_compiler_index(&repository, &model, &digests, false, None, &[], None)
                .unwrap();
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
                "org.springframework.cloud.openfeign",
                "FeignClient",
                "String name();",
                "",
            ),
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
@org.springframework.cloud.openfeign.FeignClient(name = "remote")
interface OutboundClient { @GetMapping("/remote") String remote(); }
@RestController class LocalServer implements OutboundClient {
    @Override public String remote() { return "local"; }
}
@org.springframework.cloud.openfeign.FeignClient(name = "base")
interface DefaultClient { @GetMapping("/default") default String defaultRead() { return "local"; } }
@RestController class InheritedServer implements DefaultClient {}
@interface FeignClient {}
@FeignClient class UnknownMapping { @GetMapping("/unknown") String read() { return "unknown"; } }
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
                .arg("")
                .arg("")
                .arg("")
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
            let values: Vec<serde_json::Value> = facts
                .iter()
                .map(|fact| serde_json::to_value(fact).unwrap())
                .collect();
            for fact in values.iter().filter(|fact| fact["kind"] == "DECLARATION") {
                if let Some(annotations) = fact.get("jvm_annotations") {
                    assert!(
                        annotations["definitions"]
                            .as_object()
                            .is_none_or(|definitions| definitions.is_empty()),
                        "declaration facts no longer re-embed annotation definitions"
                    );
                }
            }
            let registry = crate::spring_entrypoints::annotation_registry(values.iter());
            assert!(
                !registry.is_empty(),
                "shared annotation registry must be non-empty"
            );
            assert!(
                values
                    .iter()
                    .filter(|fact| fact["kind"] == "ANNOTATION_REGISTRY")
                    .count()
                    >= 1,
                "annotation definitions are emitted once (possibly sharded) as a shared registry"
            );
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
                            jvm_annotations: Some(annotations),
                            file,
                            start: Some(_),
                            end: Some(_),
                            ..
                        } if symbol_identity.ends_with(suffix) => {
                            assert_eq!(file, "example/Handlers.java");
                            assert_eq!(annotations["authority"], "JAVAC_RESOLVED_ANNOTATIONS");
                            let payload = serde_json::to_value(fact).unwrap();
                            let spring_payload =
                                crate::spring_entrypoints::with_annotation_registry(
                                    &payload, &registry,
                                )
                                .unwrap();
                            let derived = crate::spring_entrypoints::metadata_for_fact(
                                &spring_payload,
                                "JAVAC_RESOLVED_ANNOTATIONS",
                            )
                            .unwrap()
                            .unwrap();
                            let derived = serde_json::to_value(derived).unwrap();
                            Some(derived)
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
            let outbound = spring_for("OutboundClient#remote()Ljava/lang/String;");
            assert_eq!(outbound["entries"], json!([]));
            assert!(
                outbound["boundaries"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("OUTBOUND_FEIGN_CLIENT_NOT_SERVER_ENTRYPOINT"))
            );
            assert_eq!(
                spring_for("LocalServer#remote()Ljava/lang/String;")["entries"][0]["kind"],
                "HTTP_ENDPOINT"
            );
            assert_eq!(
                spring_for("class:example.InheritedServer")["entries"][0]["kind"],
                "HTTP_ENDPOINT"
            );
            let unknown = spring_for("UnknownMapping#read()Ljava/lang/String;");
            assert_eq!(unknown["entries"][0]["kind"], "HTTP_ENDPOINT");
            assert!(
                unknown["boundaries"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("CONTROLLER_REGISTRATION_UNPROVEN"))
            );
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
    fn transformed_workspace_provenance_and_source_state_round_trip() {
        let index = JavaCompilerIndex {
            schema: JAVA_INDEX_SCHEMA.into(),
            compilation: "example".into(),
            model: JavaProjectModel {
                schema: "codeclew-java-project-model/1.0".into(),
                model_digest: "digest".into(),
                build_system: crate::java_project_model::JavaBuildSystem::Maven,
                compilation: "example".into(),
                source_files: vec!["src/main/java/example/Service.java".into()],
                classpath: vec![],
                release: 17,
                compiler_version: "17.0".into(),
                compiler_options: vec![],
                annotation_processors: vec![],
                annotation_processor_paths: vec![],
                boundaries: vec![],
            },
            analyzer_digest: "analyzer".into(),
            facts: vec![],
            provenance: Some("TRANSFORMED_WORKSPACE".into()),
            source_state: Some(json!({
                "kind": "TRANSFORMED_WORKSPACE",
                "before": {"Service.java": "old"},
                "after": {"Service.java": "new"},
                "changedFiles": [
                    {"path": "Service.java", "before": "old", "after": "new"}
                ],
            })),
        };
        let value = serde_json::to_value(&index).unwrap();
        assert_eq!(value["provenance"], "TRANSFORMED_WORKSPACE");
        assert_eq!(value["sourceState"]["kind"], "TRANSFORMED_WORKSPACE");
        assert_eq!(
            value["sourceState"]["changedFiles"][0]["path"],
            "Service.java"
        );
        let round_trip: JavaCompilerIndex = serde_json::from_value(value).unwrap();
        assert_eq!(
            round_trip.provenance.as_deref(),
            Some("TRANSFORMED_WORKSPACE")
        );
        let source_state = round_trip.source_state.unwrap();
        assert_eq!(
            source_state["changedFiles"]
                .as_array()
                .map(|v| v.len())
                .unwrap_or(0),
            1
        );
    }

    #[test]
    fn read_only_index_omits_provenance_and_source_state_and_keeps_old_index_compatible() {
        let index = JavaCompilerIndex {
            schema: JAVA_INDEX_SCHEMA.into(),
            compilation: "example".into(),
            model: JavaProjectModel {
                schema: "codeclew-java-project-model/1.0".into(),
                model_digest: "digest".into(),
                build_system: crate::java_project_model::JavaBuildSystem::Maven,
                compilation: "example".into(),
                source_files: vec![],
                classpath: vec![],
                release: 17,
                compiler_version: "17.0".into(),
                compiler_options: vec![],
                annotation_processors: vec![],
                annotation_processor_paths: vec![],
                boundaries: vec![],
            },
            analyzer_digest: "analyzer".into(),
            facts: vec![],
            provenance: None,
            source_state: None,
        };
        let value = serde_json::to_value(&index).unwrap();
        let object = value.as_object().unwrap();
        assert!(!object.contains_key("provenance"));
        assert!(!object.contains_key("sourceState"));
        // Old-index compatibility: a serialized index without the new keys
        // deserializes back with provenance/source_state both None.
        let old: JavaCompilerIndex = serde_json::from_value(value).unwrap();
        assert_eq!(old.provenance, None);
        assert_eq!(old.source_state, None);
    }

    #[test]
    fn transformed_index_marker_assembles_expected_source_state() {
        let after: BTreeMap<String, String> = [
            (
                "src/main/java/example/Service.java".to_string(),
                "sha256:after1".into(),
            ),
            (
                "src/main/java/example/Unchanged.java".to_string(),
                "sha256:same".into(),
            ),
        ]
        .into_iter()
        .collect();
        let before: BTreeMap<String, String> = [
            (
                "src/main/java/example/Service.java".to_string(),
                "sha256:before1".into(),
            ),
            (
                "src/main/java/example/Unchanged.java".to_string(),
                "sha256:same".into(),
            ),
        ]
        .into_iter()
        .collect();
        let changed = vec![(
            "src/main/java/example/Service.java".to_string(),
            "sha256:before1".to_string(),
            "sha256:after1".to_string(),
        )];

        let (provenance, source_state) =
            transformed_index_marker(true, Some(&before), &after, &changed);
        assert_eq!(provenance.as_deref(), Some("TRANSFORMED_WORKSPACE"));
        let state = source_state.expect("source_state present for writable_then_seal");
        assert_eq!(state["kind"], "TRANSFORMED_WORKSPACE");
        assert_eq!(
            state["before"]["src/main/java/example/Service.java"],
            "sha256:before1"
        );
        // `after` reflects the passed source_content_digests exactly.
        assert_eq!(state["after"], json!(after));
        let changed_files = state["changedFiles"]
            .as_array()
            .expect("changedFiles is an array");
        assert!(!changed_files.is_empty());
        assert_eq!(
            changed_files[0]["path"],
            "src/main/java/example/Service.java"
        );
        assert_eq!(changed_files[0]["before"], "sha256:before1");
        assert_eq!(changed_files[0]["after"], "sha256:after1");

        // Read-only path leaves both marker fields None.
        let (prov, state) = transformed_index_marker(false, Some(&before), &after, &changed);
        assert_eq!(prov, None);
        assert_eq!(state, None);
    }

    #[test]
    fn saved_output_projection_uses_current_model_boundaries_and_provenance() {
        let mut authority = JavaProjectModel {
            schema: crate::java_project_model::JAVA_MODEL_SCHEMA.into(),
            model_digest: String::new(),
            build_system: crate::java_project_model::JavaBuildSystem::Maven,
            compilation: ":/main".into(),
            source_files: vec!["src/Main.java".into()],
            classpath: Vec::new(),
            release: 17,
            compiler_version: "javac 17.0.20.1".into(),
            compiler_options: vec!["--release=17".into()],
            annotation_processors: Vec::new(),
            annotation_processor_paths: Vec::new(),
            boundaries: Vec::new(),
        };
        authority.model_digest = canonical::hash(&authority).unwrap();
        let mut model = JavaOperationalModel {
            authority,
            source_paths: Vec::new(),
            classpath_paths: Vec::new(),
            annotation_processor_paths: Vec::new(),
            java_executable: "java".into(),
        };
        let raw = br#"{"kind":"BOUNDARY","schema":"codeclew-java-compiler-fact/1.0","code":"EMITTER_BOUNDARY","file":"src/Main.java","requiredChecks":[],"resolution":"TEST"}"#;
        let after = BTreeMap::from([(
            "src/Main.java".into(),
            canonical::hash_bytes(b"current sealed bytes"),
        )]);
        let original = project_java_compiler_output(raw, &model, &after, false, None, &[]).unwrap();
        assert!(original.provenance.is_none());
        model
            .authority
            .boundaries
            .push("CURRENT_MODEL_BOUNDARY".into());
        model.authority.model_digest.clear();
        model.authority.model_digest = canonical::hash(&model.authority).unwrap();
        let before = BTreeMap::from([(
            "src/Main.java".into(),
            canonical::hash_bytes(b"current original bytes"),
        )]);
        let changed = vec![(
            "src/Main.java".into(),
            before["src/Main.java"].clone(),
            after["src/Main.java"].clone(),
        )];
        let projected =
            project_java_compiler_output(raw, &model, &after, true, Some(&before), &changed)
                .unwrap();
        assert_eq!(projected.model, model.authority);
        assert!(projected.facts.iter().any(|fact| matches!(fact, JavaCompilerFact::Boundary { code, .. } if code == "CURRENT_MODEL_BOUNDARY")));
        assert!(!original.facts.iter().any(|fact| matches!(fact, JavaCompilerFact::Boundary { code, .. } if code == "CURRENT_MODEL_BOUNDARY")));
        assert_eq!(
            projected.source_state.as_ref().unwrap()["before"],
            json!(before)
        );
        assert_eq!(
            projected.source_state.as_ref().unwrap()["after"],
            json!(after)
        );
        // This verifies the pure projector only; writable eligibility is unchanged.
        assert_eq!(
            projected.provenance.as_deref(),
            Some("TRANSFORMED_WORKSPACE")
        );
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
