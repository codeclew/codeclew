use crate::error::{ClewError, ErrorCode};
use crate::proto::{
    ApplyEditRequest, BatchRequest, BlobRef, BuildLocalGraphRequest, IndexFilesRequest,
    OpenProjectRequest, ProtocolVersion, RequestKind, ResolveExpressionRequest,
    ResolveSymbolRequest, SchemaVersion, ShutdownRequest, SnapshotId, ValidateCandidateRequest,
    WorkerRequest, WorkerResponse, worker_request, worker_response,
};
use crate::runtime::RuntimeAuthority;
use prost::Message;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/worker_build_inputs.rs"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompilerIndexBackend {
    BtaPersistent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompilerIndexStatus {
    UnchangedHit,
    ColdFull,
    Incremental,
    RecoveredFull,
    Busy,
    FailedRecoverable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilerIndexProfile {
    pub backend: CompilerIndexBackend,
    pub status: CompilerIndexStatus,
    pub valid: bool,
    pub total_micros: u64,
    pub compiler_micros: u64,
    pub fir_extraction_micros: u64,
    pub total_files: u64,
    pub compiled_files: u64,
    pub reused_files: u64,
    pub recovered: bool,
    pub fallback_used: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_digest: Option<String>,
    pub semantic_input_manifest_digest: String,
    pub facts_plugin_digest: String,
    pub extractor_authority_digest: String,
    pub semantic_configuration_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectModelCacheStatus {
    MemoryHit,
    PersistentHit,
    ExtractedPublished,
    ExtractedNotPublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectModelPublishOutcome {
    NotAttempted,
    Published,
    InvalidModel,
    RootUnavailable,
    WriteFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectModelInvalidReason {
    NotApplicable,
    MissingSemanticInputManifestHash,
    InvalidSemanticInputManifestHash,
    SemanticInputManifestHashMismatch,
    MissingSemanticInputManifest,
    ModelInputsManifestMismatch,
    JdkFingerprintManifestMismatch,
    ModelInputsInvalid,
    ResourceIdentitiesInvalid,
    JdkHomeInvalid,
    JdkHomeMismatch,
    JdkFingerprintMissing,
    JdkFingerprintInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectModelCacheProfile {
    pub status: ProjectModelCacheStatus,
    pub publish_outcome: ProjectModelPublishOutcome,
    pub publish_invalid_reason: ProjectModelInvalidReason,
    pub total_micros: u64,
    pub key_micros: u64,
    pub load_micros: u64,
    pub extraction_micros: u64,
    pub publish_micros: u64,
    pub persistent_configured: bool,
    pub published: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RequestProfile {
    pub serialization_micros: u64,
    pub ipc_micros: u64,
    pub worker_processing_micros: u64,
    pub cache_requests: u64,
    pub cache_hits: u64,
    pub psi_parse_micros: u64,
    pub k2_analysis_micros: u64,
    pub fir_extraction_micros: u64,
    pub compiler_index: Option<CompilerIndexProfile>,
    pub project_model_cache: Option<ProjectModelCacheProfile>,
}

pub struct WorkerClient {
    workspace: PathBuf,
    variant: WorkerVariant,
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    next_id: u64,
    snapshot: Option<SnapshotId>,
    pub capabilities: crate::proto::WorkerCapabilities,
    pub last_profile: RequestProfile,
    authority_session: Uuid,
    trusted_distribution: Option<TrustedWorkerDistribution>,
    build_state_root: Option<PathBuf>,
    compiler_index_root: Option<PathBuf>,
    _transport_root: tempfile::TempDir,
    transport_root: PathBuf,
    issued_index_facts: BTreeMap<Uuid, String>,
    issued_source_syntax: BTreeMap<Uuid, String>,
}

const MAX_WORKER_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_WORKER_RESPONSE_BLOB_BYTES: u64 = 256 * 1024 * 1024;

/// Opaque `COMPILER_SEMANTIC` authority capability for one exact, live
/// `IndexFiles` response.
///
/// It deliberately has no `Clone`, serde implementation, public fields, or
/// constructor. Reading the index response does not grant authority to persist
/// proof-bearing compiler facts; only the issuing live `WorkerClient` can
/// replay this capability.
pub struct VerifiedIndexFacts {
    receipt_id: Uuid,
    authority_session: Uuid,
    repo: PathBuf,
    compilation: String,
    base_revision: String,
    project_model_hash: String,
    distribution_fingerprint: String,
    distribution_tree_hash: String,
    build_input_digest: String,
    payload_hash: String,
    relation_hash: String,
    descriptor_hash: String,
    payload: Value,
}

/// Opaque `SOURCE_SYNTAX` capability for current source declarations.
///
/// This type is intentionally disjoint from [`VerifiedIndexFacts`]. It cannot
/// authorize compiler facts, relation graphs, descriptor graphs, edits, or
/// transactions. The issuing live worker session rechecks the exact files on
/// every inspection, so a source change invalidates the capability even when
/// Git HEAD does not change.
pub struct VerifiedSourceSyntax {
    receipt_id: Uuid,
    authority_session: Uuid,
    repo: PathBuf,
    compilation: String,
    distribution_tree_hash: String,
    build_input_digest: String,
    requested_files: Vec<String>,
    source_manifest_hash: String,
    payload_hash: String,
    payload: Value,
}

pub const COMPILER_SEMANTIC_AUTHORITY: &str = "COMPILER_SEMANTIC";
pub const SOURCE_SYNTAX_AUTHORITY: &str = "SOURCE_SYNTAX";

/// Stable, read-only identity of the exact worker distribution trusted by this
/// client.  Callers may use it as a cache-key input, but it grants no authority
/// to issue or replay compiler receipts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedDistributionIdentity {
    pub tree_hash: String,
    pub build_input_digest: String,
    pub plugin_fingerprint: String,
}

fn verified_index_failure_stage(error: &ClewError, default: &str) -> String {
    error
        .evidence
        .iter()
        .find_map(|value| value.strip_prefix("verified-index-stage:"))
        .unwrap_or(default)
        .to_owned()
}

fn attach_verified_index_failure(
    mut error: ClewError,
    default_stage: &str,
    facts: Option<&Value>,
) -> ClewError {
    let stage = verified_index_failure_stage(&error, default_stage);
    let relation_graph = facts.and_then(|value| value.get("declarationRelations"));
    let descriptor_graph = facts.and_then(|value| value.get("declarationDescriptors"));
    let relation_provenance = relation_graph.and_then(|value| value.get("provenance"));
    let descriptor_provenance = descriptor_graph.and_then(|value| value.get("provenance"));
    let safe_hash = |value: Option<&Value>| {
        crate::canonical::hash(value.unwrap_or(&Value::Null))
            .unwrap_or_else(|_| "unavailable".into())
    };
    let worker_diagnostics = facts
        .and_then(|value| value.get("diagnostics"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .take(16)
                .map(|row| {
                    let message = row
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let mut bounded = message.chars().take(1_024).collect::<String>();
                    if message.chars().count() > 1_024 {
                        bounded.push('…');
                    }
                    serde_json::json!({
                        "severity":row.get("severity").and_then(Value::as_str).unwrap_or("INFO"),
                        "message":bounded,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let diagnostic = serde_json::json!({
        "schema":"verified-index-failure-diagnostic/0.1",
        "stage":stage,
        "rawSchemaHash":safe_hash(facts.and_then(|value| value.get("schema"))),
        "payloadHash":safe_hash(facts),
        "relationGraphHash":safe_hash(relation_graph),
        "descriptorGraphHash":safe_hash(descriptor_graph),
        "relationProvenanceHash":safe_hash(relation_provenance),
        "descriptorProvenanceHash":safe_hash(descriptor_provenance),
        "relationCount":relation_graph.and_then(|value| value.get("relations")).and_then(Value::as_array).map_or(0, Vec::len),
        "relationBoundaryCount":relation_graph.and_then(|value| value.get("boundaries")).and_then(Value::as_array).map_or(0, Vec::len),
        "descriptorCount":descriptor_graph.and_then(|value| value.get("descriptors")).and_then(Value::as_array).map_or(0, Vec::len),
        "descriptorBoundaryCount":descriptor_graph.and_then(|value| value.get("boundaries")).and_then(Value::as_array).map_or(0, Vec::len),
        "workerDiagnosticCount":facts.and_then(|value| value.get("diagnostics")).and_then(Value::as_array).map_or(0, Vec::len),
        "workerDiagnostics":worker_diagnostics,
        "descriptorFailure":if stage == "DESCRIPTOR_GRAPH" {
            facts.map(crate::index::descriptor_validation_diagnostic).unwrap_or(Value::Null)
        } else {
            Value::Null
        },
    });
    if let Ok(encoded) = serde_json::to_string(&diagnostic) {
        error.evidence.push(encoded);
    }
    error
}

fn require_k2_validated(facts: &Value) -> Result<(), ClewError> {
    if facts.get("k2Validated").and_then(Value::as_bool) != Some(true) {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "verified compiler facts require a successful K2 analysis",
        ));
    }
    Ok(())
}

fn source_syntax_protocol_error(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::WorkerProtocolMismatch, message)
}

fn require_absent_or_empty_rows(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<(), ClewError> {
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    if value.as_array().is_some_and(Vec::is_empty) {
        return Ok(());
    }
    Err(source_syntax_protocol_error(format!(
        "SOURCE_SYNTAX response contains {context}"
    )))
}

fn require_empty_semantic_graph(
    facts: &Value,
    graph_key: &str,
    row_keys: &[&str],
    hash_key: &str,
) -> Result<(), ClewError> {
    let Some(graph) = facts.get(graph_key) else {
        if facts.get(hash_key).is_some_and(|value| !value.is_null()) {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX response has {hash_key} without {graph_key}"
            )));
        }
        return Ok(());
    };
    if let Some(rows) = graph.as_array() {
        if !rows.is_empty() {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX {graph_key} contains semantic rows"
            )));
        }
        if let Some(expected) = facts.get(hash_key) {
            let expected = expected.as_str().ok_or_else(|| {
                source_syntax_protocol_error(format!("SOURCE_SYNTAX {hash_key} is not a string"))
            })?;
            if crate::canonical::hash(graph).map_err(internal)? != expected {
                return Err(source_syntax_protocol_error(format!(
                    "SOURCE_SYNTAX {graph_key} hash differs"
                )));
            }
        }
        return Ok(());
    }
    let object = graph.as_object().ok_or_else(|| {
        source_syntax_protocol_error(format!(
            "SOURCE_SYNTAX {graph_key} must be absent or an empty graph"
        ))
    })?;
    for key in row_keys {
        require_absent_or_empty_rows(object, key, &format!("{graph_key}.{key} rows"))?;
    }
    if let Some(provenance) = object.get("provenance") {
        let empty = provenance.is_null()
            || provenance
                .as_object()
                .is_some_and(serde_json::Map::is_empty);
        if !empty {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX {graph_key} carries semantic provenance"
            )));
        }
    }
    if let Some(expected) = facts.get(hash_key) {
        let expected = expected.as_str().ok_or_else(|| {
            source_syntax_protocol_error(format!("SOURCE_SYNTAX {hash_key} is not a string"))
        })?;
        if crate::canonical::hash(graph).map_err(internal)? != expected {
            return Err(source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX {graph_key} hash differs"
            )));
        }
    }
    Ok(())
}

fn source_syntax_relative_path(repo: &Path, raw: &str) -> Result<(PathBuf, Vec<u8>), ClewError> {
    let relative = Path::new(raw);
    if raw.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX file path is not a normalized repository-relative path",
        ));
    }
    let joined = repo.join(relative);
    let metadata = std::fs::symlink_metadata(&joined).map_err(|error| {
        ClewError::new(
            ErrorCode::ProjectModelChanged,
            format!("SOURCE_SYNTAX file is no longer current: {error}"),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "SOURCE_SYNTAX file is no longer a regular source file",
        ));
    }
    let canonical = joined.canonicalize().map_err(internal)?;
    if canonical != joined || !canonical.starts_with(repo) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX file resolves through a symlink or outside the repository",
        ));
    }
    let bytes = std::fs::read(&canonical).map_err(internal)?;
    Ok((canonical, bytes))
}

/// Validate the deliberately narrow claim made by a `SOURCE_SYNTAX` response
/// and return a stable manifest hash for the exact current declarations.
fn validate_source_syntax_response(
    repo: &Path,
    compilation: &str,
    requested_files: &[String],
    facts: &Value,
) -> Result<String, ClewError> {
    let repo = repo.canonicalize().map_err(internal)?;
    if facts.get("schema").and_then(Value::as_str) != Some("semantic-index/0.1")
        || facts.get("compilation").and_then(Value::as_str) != Some(compilation)
    {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response schema or compilation differs from the request",
        ));
    }
    if facts.get("analysisMode").and_then(Value::as_str) != Some("SYNTAX_DECLARATIONS") {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response requires analysisMode SYNTAX_DECLARATIONS",
        ));
    }
    if facts.get("k2Validated").and_then(Value::as_bool) != Some(false) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response must explicitly report k2Validated false",
        ));
    }
    if facts.get("partial").and_then(Value::as_bool) != Some(!requested_files.is_empty()) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response partial flag differs from the exact request selector",
        ));
    }
    require_empty_semantic_graph(
        facts,
        "declarationRelations",
        &["relations", "boundaries"],
        "declarationRelationHash",
    )?;
    require_empty_semantic_graph(
        facts,
        "declarationDescriptors",
        &["descriptors", "boundaries"],
        "declarationDescriptorHash",
    )?;
    let top = facts.as_object().ok_or_else(|| {
        source_syntax_protocol_error("SOURCE_SYNTAX response is not a JSON object")
    })?;
    require_absent_or_empty_rows(top, "semanticFacts", "top-level semantic facts")?;

    let files = facts
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            source_syntax_protocol_error("SOURCE_SYNTAX response has no file payload")
        })?;
    let mut manifest = BTreeMap::<String, Value>::new();
    for file in files {
        let object = file.as_object().ok_or_else(|| {
            source_syntax_protocol_error("SOURCE_SYNTAX file payload is not an object")
        })?;
        for (key, context) in [
            ("semanticFacts", "per-file semantic facts"),
            ("inheritance", "per-file inheritance facts"),
            ("overrides", "per-file override facts"),
            ("functionSummaries", "per-file function summaries"),
        ] {
            require_absent_or_empty_rows(object, key, context)?;
        }
        let relative = object.get("path").and_then(Value::as_str).ok_or_else(|| {
            source_syntax_protocol_error("SOURCE_SYNTAX file payload has no path")
        })?;
        if object.get("normalizedRelativePath").and_then(Value::as_str) != Some(relative) {
            return Err(source_syntax_protocol_error(
                "SOURCE_SYNTAX file path is not its normalizedRelativePath",
            ));
        }
        let (_, bytes) = source_syntax_relative_path(&repo, relative)?;
        let content_hash = crate::canonical::hash_bytes(&bytes);
        if object.get("contentHash").and_then(Value::as_str) != Some(content_hash.as_str()) {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!("SOURCE_SYNTAX content hash is stale for {relative}"),
            ));
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            source_syntax_protocol_error(format!(
                "SOURCE_SYNTAX Kotlin source is not UTF-8: {relative}"
            ))
        })?;
        let source_utf16_len = source.encode_utf16().count() as u64;
        let declarations = object
            .get("declarations")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                source_syntax_protocol_error(format!(
                    "SOURCE_SYNTAX file has no declaration payload: {relative}"
                ))
            })?;
        // SOURCE_SYNTAX identities are deliberately non-semantic: overloads
        // and declarations from an unknown compilation may share the worker's
        // provisional declarationId/symbolId.  The authority is the exact
        // current source occurrence, which must still be unique.
        let mut declaration_origins = BTreeSet::<(u64, u64)>::new();
        let mut previous_range: Option<(u64, u64)> = None;
        for declaration in declarations {
            let declaration = declaration.as_object().ok_or_else(|| {
                source_syntax_protocol_error("SOURCE_SYNTAX declaration is not an object")
            })?;
            if declaration.contains_key("compilerSymbol")
                || declaration.contains_key("compilerAuthority")
            {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration contains compiler-semantic authority",
                ));
            }
            for key in ["declarationId", "symbolId", "kind"] {
                if declaration
                    .get(key)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(source_syntax_protocol_error(format!(
                        "SOURCE_SYNTAX declaration has no {key}"
                    )));
                }
            }
            if declaration.get("name").and_then(Value::as_str).is_none() {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration has no string name",
                ));
            }
            if declaration.get("file").and_then(Value::as_str) != Some(relative) {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration file differs from its containing file",
                ));
            }
            let start = declaration.get("rangeStart").and_then(Value::as_u64);
            let end = declaration.get("rangeEnd").and_then(Value::as_u64);
            let (Some(start), Some(end)) = (start, end) else {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration has no source range",
                ));
            };
            if start > end || end > source_utf16_len {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration range is outside current source",
                ));
            }
            if previous_range.is_some_and(|previous| previous > (start, end)) {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declarations are not in source order",
                ));
            }
            previous_range = Some((start, end));
            let origin = declaration
                .get("sourceOrigin")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    source_syntax_protocol_error("SOURCE_SYNTAX declaration has no source origin")
                })?;
            if origin.get("file").and_then(Value::as_str) != Some(relative)
                || origin.get("rangeStart").and_then(Value::as_u64) != Some(start)
                || origin.get("rangeEnd").and_then(Value::as_u64) != Some(end)
            {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration source origin differs from its range",
                ));
            }
            if !declaration_origins.insert((start, end)) {
                return Err(source_syntax_protocol_error(
                    "SOURCE_SYNTAX declaration source occurrences are not unique within a file",
                ));
            }
        }
        let declaration_hash =
            crate::canonical::hash(&Value::Array(declarations.clone())).map_err(internal)?;
        if manifest
            .insert(
                relative.to_owned(),
                serde_json::json!({
                    "contentHash":content_hash,
                    "declarationHash":declaration_hash,
                }),
            )
            .is_some()
        {
            return Err(source_syntax_protocol_error(
                "SOURCE_SYNTAX response contains a duplicate file path",
            ));
        }
    }

    let mut requested = BTreeMap::<String, ()>::new();
    for relative in requested_files {
        let _ = source_syntax_relative_path(&repo, relative)?;
        if requested.insert(relative.clone(), ()).is_some() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX request contains duplicate files",
            ));
        }
    }
    if requested.keys().ne(manifest.keys()) {
        return Err(source_syntax_protocol_error(
            "SOURCE_SYNTAX response does not contain exactly the requested files",
        ));
    }

    crate::canonical::hash(&serde_json::json!({
        "authority":SOURCE_SYNTAX_AUTHORITY,
        "compilation":compilation,
        "files":manifest,
    }))
    .map_err(internal)
}

/// Preserve compiler-proven base relations while quarantining optional
/// relation enrichments whose coordinate/CFG contract is not yet reliable.
///
/// The worker's original graph hash is verified before any transformation.
/// Nothing is upgraded to PROVEN here: uncertain enrichments are removed and
/// represented by typed UNKNOWN boundaries, then the ordinary strict graph
/// validator authorizes the remaining facts.
fn normalize_optional_relation_evidence(facts: &mut Value) -> Result<(), ClewError> {
    fn invalid(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::InvalidInput, message)
    }

    let expected_hash = facts
        .get("declarationRelationHash")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("worker index has no declarationRelationHash"))?
        .to_owned();
    let graph = facts
        .get_mut("declarationRelations")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("worker index has no declaration relation graph"))?;
    if crate::canonical::hash(graph).map_err(internal)? != expected_hash {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration relation hash differs before optional evidence normalization",
        ));
    }

    for label in ["relations", "boundaries"] {
        let rows = graph
            .get(label)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(format!("declaration relation graph has no {label}")))?;
        let encoded = rows
            .iter()
            .map(crate::canonical::bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid(format!(
                "declaration relation {label} must be canonical before normalization"
            )));
        }
    }

    let original_relations = graph["relations"]
        .as_array_mut()
        .ok_or_else(|| invalid("declaration relation graph has no relations"))?;
    let mut retained = Vec::with_capacity(original_relations.len());
    let mut normalized_rows = BTreeMap::<String, Vec<String>>::new();
    for mut relation in std::mem::take(original_relations) {
        let kind = relation
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let raw_row_hash = crate::canonical::hash(&relation).map_err(internal)?;
        let boundary_code = match kind.as_str() {
            "NULL_COALESCES" => Some("NULL_COALESCING_FLOW_UNAVAILABLE"),
            "RETURNS_VALUE_FROM" => Some("RETURN_VALUE_FLOW_UNAVAILABLE"),
            _ => None,
        };
        if let Some(code) = boundary_code {
            normalized_rows
                .entry(code.to_owned())
                .or_default()
                .push(raw_row_hash);
            continue;
        }

        let argument_mapping = relation.get("argumentToParameter");
        let has_argument_mapping_evidence = argument_mapping
            .is_some_and(|value| value.as_array().is_none_or(|rows| !rows.is_empty()));
        if matches!(kind.as_str(), "CALLS" | "CONSTRUCTS") && argument_mapping.is_some() {
            relation
                .as_object_mut()
                .expect("relation was already an object")
                .remove("argumentToParameter");
        }
        if matches!(kind.as_str(), "CALLS" | "CONSTRUCTS") && has_argument_mapping_evidence {
            normalized_rows
                .entry("ARGUMENT_MAPPING_UNAVAILABLE".to_owned())
                .or_default()
                .push(raw_row_hash);
        }
        retained.push(relation);
    }

    let mut unique_relations = BTreeMap::new();
    for relation in retained {
        unique_relations.insert(
            crate::canonical::bytes(&relation).map_err(internal)?,
            relation,
        );
    }
    *original_relations = unique_relations.into_values().collect();

    if !normalized_rows.is_empty() {
        let boundaries = graph["boundaries"]
            .as_array_mut()
            .ok_or_else(|| invalid("declaration relation graph has no boundaries"))?;
        for (code, mut row_hashes) in normalized_rows {
            row_hashes.sort();
            row_hashes.dedup();
            boundaries.push(serde_json::json!({
                "schema":"declaration-relation-boundary/0.1",
                "stage":"OPTIONAL_RELATION_EVIDENCE",
                "code":code,
                "resolution":"UNKNOWN",
                "provider":"CODECLEW_RELATION_NORMALIZER",
                "affectedRowCount":row_hashes.len(),
                "rawRowsHash":crate::canonical::hash(&serde_json::json!(row_hashes)).map_err(internal)?,
            }));
        }
        let mut unique_boundaries = BTreeMap::new();
        for boundary in std::mem::take(boundaries) {
            unique_boundaries.insert(
                crate::canonical::bytes(&boundary).map_err(internal)?,
                boundary,
            );
        }
        *boundaries = unique_boundaries.into_values().collect();
        graph["coverage"] = Value::String("PARTIAL".to_owned());
    }

    let normalized_hash = crate::canonical::hash(graph).map_err(internal)?;
    facts["declarationRelationHash"] = Value::String(normalized_hash);
    Ok(())
}

impl VerifiedIndexFacts {
    pub fn authority(&self) -> &'static str {
        COMPILER_SEMANTIC_AUTHORITY
    }

    pub(crate) fn compilation(&self) -> &str {
        &self.compilation
    }

    pub(crate) fn project_model_hash(&self) -> &str {
        &self.project_model_hash
    }
}

impl VerifiedSourceSyntax {
    pub fn authority(&self) -> &'static str {
        SOURCE_SYNTAX_AUTHORITY
    }

    pub fn compilation(&self) -> &str {
        &self.compilation
    }
}

struct TrustedWorkerDistribution {
    workspace: PathBuf,
    _private_root: tempfile::TempDir,
    distribution_root: PathBuf,
    launcher: PathBuf,
    tree_manifest: BTreeMap<String, String>,
    tree_hash: String,
    build_input_digest: String,
    plugin_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerVariant {
    Kotlin21,
    Kotlin23,
    Kotlin24,
}

impl WorkerVariant {
    fn runtime_name(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin21",
            Self::Kotlin23 => "kotlin23",
            Self::Kotlin24 => "kotlin24",
        }
    }

    fn for_project(version: &str) -> Result<Self, ClewError> {
        match version
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".")
            .as_str()
        {
            "2.1" => Ok(Self::Kotlin21),
            "2.3" => Ok(Self::Kotlin23),
            "2.4" => Ok(Self::Kotlin24),
            _ => Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                format!(
                    "unsupported Kotlin compiler {version}; supported compiler lines are 2.1, 2.3, and 2.4"
                ),
            )),
        }
    }

    fn compiler_version(self) -> &'static str {
        match self {
            Self::Kotlin21 => "2.1.21",
            Self::Kotlin23 => "2.3.0",
            Self::Kotlin24 => "2.4.10",
        }
    }

    fn discovery_bit(self) -> u8 {
        match self {
            Self::Kotlin21 => 1 << 0,
            Self::Kotlin23 => 1 << 1,
            Self::Kotlin24 => 1 << 2,
        }
    }

    fn next_untried_for_abi_discovery(tried: u8) -> Option<Self> {
        [Self::Kotlin24, Self::Kotlin23, Self::Kotlin21]
            .into_iter()
            .find(|variant| tried & variant.discovery_bit() == 0)
    }

    fn install_task(self) -> &'static str {
        match self {
            Self::Kotlin21 => ":workers:kotlin21:installDist",
            Self::Kotlin23 => ":workers:kotlin23:installDist",
            Self::Kotlin24 => ":workers:kotlin:installDist",
        }
    }

    fn distribution_relative(self) -> &'static str {
        match self {
            Self::Kotlin21 => "workers/kotlin21/build/install/kotlin21",
            Self::Kotlin23 => "workers/kotlin23/build/install/kotlin23",
            Self::Kotlin24 => "workers/kotlin/build/install/kotlin",
        }
    }

    fn launcher_name(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin21",
            Self::Kotlin23 => "kotlin23",
            Self::Kotlin24 => "kotlin",
        }
    }

    fn plugin_jar_name(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin21-0.1.0.jar",
            Self::Kotlin23 => "kotlin23-0.1.0.jar",
            Self::Kotlin24 => "kotlin-0.1.0.jar",
        }
    }

    fn pinned_inputs(self) -> PinnedInputs {
        match self {
            Self::Kotlin21 => PinnedInputs {
                roots: PINNED_KOTLIN21_INPUT_ROOTS,
                files: PINNED_KOTLIN21_INPUT_FILES,
                entries: PINNED_KOTLIN21_INPUTS,
                digest: PINNED_KOTLIN21_INPUT_DIGEST,
                outputs: PINNED_KOTLIN21_OUTPUTS,
                output_digest: PINNED_KOTLIN21_OUTPUT_DIGEST,
            },
            Self::Kotlin23 => PinnedInputs {
                roots: PINNED_KOTLIN23_INPUT_ROOTS,
                files: PINNED_KOTLIN23_INPUT_FILES,
                entries: PINNED_KOTLIN23_INPUTS,
                digest: PINNED_KOTLIN23_INPUT_DIGEST,
                outputs: PINNED_KOTLIN23_OUTPUTS,
                output_digest: PINNED_KOTLIN23_OUTPUT_DIGEST,
            },
            Self::Kotlin24 => PinnedInputs {
                roots: PINNED_KOTLIN24_INPUT_ROOTS,
                files: PINNED_KOTLIN24_INPUT_FILES,
                entries: PINNED_KOTLIN24_INPUTS,
                digest: PINNED_KOTLIN24_INPUT_DIGEST,
                outputs: PINNED_KOTLIN24_OUTPUTS,
                output_digest: PINNED_KOTLIN24_OUTPUT_DIGEST,
            },
        }
    }
}

struct PinnedInputs {
    roots: &'static [&'static str],
    files: &'static [&'static str],
    entries: &'static [(&'static str, &'static str)],
    digest: &'static str,
    outputs: &'static [(&'static str, u64, &'static str)],
    output_digest: &'static str,
}

fn validate_compiler_index_root(workspace: &Path, root: &Path) -> Result<PathBuf, ClewError> {
    if !root.is_absolute() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be absolute",
        ));
    }
    let metadata = std::fs::symlink_metadata(root).map_err(internal)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "compiler index root must be private",
            ));
        }
    }
    let canonical = root.canonicalize().map_err(internal)?;
    if canonical != root {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be canonical and have no symlinked ancestor",
        ));
    }
    let workspace = workspace.canonicalize().map_err(internal)?;
    if canonical.starts_with(&workspace) || workspace.starts_with(&canonical) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "compiler index root must be external to the worker workspace",
        ));
    }
    Ok(canonical)
}

fn configure_worker_state_environment(
    command: &mut Command,
    build_state_root: Option<&Path>,
    compiler_index_root: Option<&Path>,
) {
    command
        .env_remove("CODECLEW_K1_BUILD_STATE_ROOT")
        .env_remove("CODECLEW_K2_INDEX_ROOT");
    if let Some(root) = build_state_root {
        command.env("CODECLEW_K1_BUILD_STATE_ROOT", root);
    }
    if let Some(root) = compiler_index_root {
        command.env("CODECLEW_K2_INDEX_ROOT", root);
    }
}

fn build_state_root_from_environment_value(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// Select the optional sealed build-state authority from the caller environment.
/// An explicitly empty variable is equivalent to it being unset so CI wrappers
/// can enable the project-native default without manufacturing an invalid path.
pub fn inherited_build_state_root() -> Option<PathBuf> {
    build_state_root_from_environment_value(std::env::var_os("CODECLEW_K1_BUILD_STATE_ROOT"))
}

impl WorkerClient {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn trusted_distribution_identity(&self) -> Option<TrustedDistributionIdentity> {
        self.trusted_distribution
            .as_ref()
            .map(|trusted| TrustedDistributionIdentity {
                tree_hash: trusted.tree_hash.clone(),
                build_input_digest: trusted.build_input_digest.clone(),
                plugin_fingerprint: trusted.plugin_fingerprint.clone(),
            })
    }

    pub fn start(workspace: &Path) -> Result<Self, ClewError> {
        let inherited_build_state = inherited_build_state_root();
        Self::start_variant(
            workspace,
            WorkerVariant::Kotlin24,
            inherited_build_state.as_deref(),
            None,
        )
    }

    pub fn start_with_build_state(
        workspace: &Path,
        build_state_root: &Path,
    ) -> Result<Self, ClewError> {
        Self::start_variant(
            workspace,
            WorkerVariant::Kotlin24,
            Some(build_state_root),
            None,
        )
    }

    /// Start with distinct immutable build-input authority and mutable
    /// compiler-owned derived state. The latter is never an input proof and
    /// must be a private external directory.
    pub fn start_with_states(
        workspace: &Path,
        build_state_root: Option<&Path>,
        compiler_index_root: Option<&Path>,
    ) -> Result<Self, ClewError> {
        Self::start_variant(
            workspace,
            WorkerVariant::Kotlin24,
            build_state_root,
            compiler_index_root,
        )
    }

    /// Start in repository-local development mode without consulting ambient
    /// build-state environment. This is intentionally distinct from `start`,
    /// whose inherited environment behavior remains for legacy callers.
    pub fn start_without_build_state(workspace: &Path) -> Result<Self, ClewError> {
        Self::start_variant(workspace, WorkerVariant::Kotlin24, None, None)
    }

    fn start_variant(
        workspace: &Path,
        variant: WorkerVariant,
        build_state_root: Option<&Path>,
        compiler_index_root: Option<&Path>,
    ) -> Result<Self, ClewError> {
        let trusted_distribution = prepare_trusted_worker_distribution(workspace, variant)?;
        let launcher = trusted_distribution
            .as_ref()
            .map(|trusted| trusted.launcher.clone())
            .unwrap_or_else(|| worker_launcher(workspace, variant));
        if trusted_distribution.is_none() && !launcher.is_file() {
            let output = Command::new(workspace.join("gradlew"))
                .args([variant.install_task(), "--no-daemon", "--quiet"])
                .current_dir(workspace)
                .output()
                .map_err(|e| {
                    ClewError::new(
                        ErrorCode::WorkerCrashed,
                        format!("cannot build Kotlin worker: {e}"),
                    )
                })?;
            if !output.stdout.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
            }
            if !output.status.success() {
                return Err(ClewError::new(
                    ErrorCode::WorkerCrashed,
                    "Kotlin worker build failed",
                ));
            }
        }
        let canonical_build_state = build_state_root
            .map(|root| root.canonicalize().map_err(internal))
            .transpose()?;
        let canonical_compiler_index = compiler_index_root
            .map(|root| validate_compiler_index_root(workspace, root))
            .transpose()?;
        if let (Some(build_state), Some(compiler_index)) =
            (&canonical_build_state, &canonical_compiler_index)
            && (build_state.starts_with(compiler_index) || compiler_index.starts_with(build_state))
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "compiler index root and sealed build-state root must be disjoint",
            ));
        }
        let transport_root = tempfile::Builder::new()
            .prefix("codeclew-worker-transport-")
            .tempdir()
            .map_err(internal)?;
        let canonical_transport_root = transport_root.path().canonicalize().map_err(internal)?;
        let transport_metadata =
            std::fs::symlink_metadata(transport_root.path()).map_err(internal)?;
        if transport_metadata.file_type().is_symlink() || !transport_metadata.is_dir() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker transport root is not a private real directory",
            ));
        }
        let mut command = Command::new(&launcher);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env("CODECLEW_WORKER_TRANSPORT_ROOT", &canonical_transport_root);
        configure_worker_state_environment(
            &mut command,
            canonical_build_state.as_deref(),
            canonical_compiler_index.as_deref(),
        );
        let mut child = command.spawn().map_err(|e| {
            ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("cannot start {}: {e}", launcher.display()),
            )
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = child.stdout.take().expect("piped stdout");
        let hello = read_message(&mut stdout)?;
        let capabilities = match hello.payload {
            Some(worker_response::Payload::Capabilities(value)) => value,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker did not send startup capabilities",
                ));
            }
        };
        if capabilities.compiler_version != variant.compiler_version()
            || !capabilities.protocol_versions.iter().any(|v| v.major == 1)
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker compiler/protocol version mismatch",
            ));
        }
        Ok(Self {
            workspace: workspace.to_path_buf(),
            variant,
            child,
            stdin,
            stdout,
            next_id: 1,
            snapshot: None,
            capabilities,
            last_profile: RequestProfile::default(),
            authority_session: Uuid::new_v4(),
            trusted_distribution,
            build_state_root: canonical_build_state,
            compiler_index_root: canonical_compiler_index,
            _transport_root: transport_root,
            transport_root: canonical_transport_root,
            issued_index_facts: BTreeMap::new(),
            issued_source_syntax: BTreeMap::new(),
        })
    }

    fn switch_variant(&mut self, variant: WorkerVariant) -> Result<(), ClewError> {
        if self.variant == variant {
            return Ok(());
        }
        let replacement = Self::start_variant(
            &self.workspace,
            variant,
            self.build_state_root.as_deref(),
            self.compiler_index_root.as_deref(),
        )?;
        let previous = std::mem::replace(self, replacement);
        previous.shutdown()
    }

    pub fn request(&mut self, kind: RequestKind, payload: &Value) -> Result<Value, ClewError> {
        self.request_with_discovery_variants(kind, payload, 0)
    }

    fn request_with_discovery_variants(
        &mut self,
        kind: RequestKind,
        payload: &Value,
        tried_discovery_variants: u8,
    ) -> Result<Value, ClewError> {
        if self.snapshot.is_none()
            && request_requires_project_snapshot(kind, payload)
            && payload.get("repo").and_then(Value::as_str).is_some()
        {
            let bootstrap = serde_json::json!({
                "repo":payload.get("repo"),
                "compilation":payload.get("compilation")
            });
            let _ = self.request(RequestKind::OpenProject, &bootstrap)?;
        }
        let request_id = self.next_id;
        self.next_id += 1;
        let request_serialization_started = Instant::now();
        let schema_version = || Some(SchemaVersion { major: 1, minor: 0 });
        let repo = || json_string(payload, "repo");
        let compilation = || json_optional_string(payload, "compilation");
        let request_payload = match kind {
            RequestKind::OpenProject => worker_request::Payload::OpenProject(OpenProjectRequest {
                schema_version: schema_version(),
                repo: repo(),
                compilation: compilation(),
            }),
            RequestKind::IndexFiles => worker_request::Payload::IndexFiles(IndexFilesRequest {
                schema_version: schema_version(),
                repo: repo(),
                compilation: compilation(),
                syntax_only: payload
                    .get("syntaxOnly")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                relative_files: payload
                    .get("files")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            }),
            RequestKind::ResolveSymbol => {
                worker_request::Payload::ResolveSymbol(ResolveSymbolRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    symbol: json_string(payload, "symbol"),
                    compilation: compilation(),
                })
            }
            RequestKind::ResolveExpression => {
                worker_request::Payload::ResolveExpression(ResolveExpressionRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    file: json_string(payload, "file"),
                    offset: payload.get("offset").and_then(Value::as_u64).unwrap_or(0),
                    compilation: compilation(),
                })
            }
            RequestKind::BuildLocalGraph => {
                worker_request::Payload::BuildLocalGraph(BuildLocalGraphRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    symbol: json_string(payload, "symbol"),
                    compilation: compilation(),
                })
            }
            RequestKind::ApplyEdit => {
                let (source_inline, source_blob) =
                    source_transport(payload, self.build_state_root.as_deref())?;
                worker_request::Payload::ApplyEdit(ApplyEditRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    file: json_string(payload, "file"),
                    source_inline,
                    source_blob,
                    owner_symbol_id: json_string(payload, "ownerSymbolId"),
                    exact_text_hash: json_string(payload, "exactTextHash"),
                    syntax_kind: json_string(payload, "syntaxKind"),
                    normalized_token_hash: json_string(payload, "normalizedTokenHash"),
                    ancestor_path_hash: json_string(payload, "ancestorPathHash"),
                    local_ordinal: payload.get("localOrdinal").and_then(Value::as_u64),
                    left_context_hash: json_string(payload, "leftContextHash"),
                    right_context_hash: json_string(payload, "rightContextHash"),
                    operation_kind: json_string(payload, "kind"),
                    replacement: json_string(payload, "replacement"),
                    preconditions_json: serde_json::to_vec(
                        payload.get("preconditions").unwrap_or(&Value::Null),
                    )
                    .map_err(internal)?,
                    postconditions_json: serde_json::to_vec(
                        payload.get("postconditions").unwrap_or(&Value::Null),
                    )
                    .map_err(internal)?,
                    compilation: compilation(),
                    defer_semantic_validation: payload
                        .get("deferSemanticValidation")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    semantic_operation_json: serde_json::to_vec(
                        payload.get("semanticOperation").unwrap_or(&Value::Null),
                    )
                    .map_err(internal)?,
                })
            }
            RequestKind::ValidateCandidate => {
                let (source_inline, source_blob) =
                    source_transport(payload, self.build_state_root.as_deref())?;
                worker_request::Payload::ValidateCandidate(ValidateCandidateRequest {
                    schema_version: schema_version(),
                    repo: repo(),
                    file: json_string(payload, "file"),
                    source_inline,
                    source_blob,
                })
            }
            RequestKind::Shutdown => worker_request::Payload::Shutdown(ShutdownRequest {
                schema_version: schema_version(),
            }),
            _ => {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "unspecified worker request kind",
                ));
            }
        };
        let request = WorkerRequest {
            request_id,
            protocol_version: Some(ProtocolVersion { major: 1, minor: 0 }),
            snapshot: Some(
                if kind == RequestKind::IndexFiles
                    && payload.get("syntaxOnly").and_then(Value::as_bool) == Some(true)
                {
                    snapshot_from(payload)
                } else {
                    self.snapshot
                        .clone()
                        .unwrap_or_else(|| snapshot_from(payload))
                },
            ),
            payload: Some(request_payload),
        };
        let request_construction_micros =
            request_serialization_started.elapsed().as_micros() as u64;
        let (encode_micros, write_micros) = write_message_profiled(&mut self.stdin, &request)?;
        let (response, read_micros, decode_micros) = read_message_profiled(&mut self.stdout)?;
        if response.request_id != request_id {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker response request_id mismatch",
            ));
        }
        if response
            .protocol_version
            .as_ref()
            .is_none_or(|version| version.major != 1 || version.minor != 0)
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker response protocol identity mismatch",
            ));
        }
        let canonical_json = match response.payload {
            Some(worker_response::Payload::Error(error)) => {
                let relevant: Vec<String> = payload
                    .get("ownerSymbolId")
                    .or_else(|| payload.get("symbol"))
                    .or_else(|| payload.get("exactTextHash"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .into_iter()
                    .collect();
                let failure = ClewError {
                    code: parse_worker_code(&error.code),
                    message: error.message,
                    transaction_id: None,
                    snapshot_id: self.snapshot.as_ref().map(snapshot_label),
                    evidence: error.evidence,
                    relevant_anchors_or_symbols: relevant.into_boxed_slice(),
                    retryable: error.retryable,
                };
                if kind == RequestKind::OpenProject
                    && failure.code == ErrorCode::UnsupportedCompilerPluginAbi
                {
                    let tried = tried_discovery_variants | self.variant.discovery_bit();
                    if let Some(next) = WorkerVariant::next_untried_for_abi_discovery(tried) {
                        self.switch_variant(next)?;
                        return self.request_with_discovery_variants(kind, payload, tried);
                    }
                }
                return Err(failure);
            }
            Some(worker_response::Payload::OpenProject(value)) => {
                validate_typed_string(
                    &value.canonical_json,
                    "/projectModelHash",
                    &value.project_model_hash,
                )?;
                if value.compilation.is_empty() {
                    return Err(ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "typed OpenProject response has no compilation identity",
                    ));
                }
                let mut canonical: Value =
                    serde_json::from_slice(&value.canonical_json).map_err(internal)?;
                bind_open_project_compilation(&mut canonical, &value.compilation)?;
                crate::canonical::bytes(&canonical).map_err(internal)?
            }
            Some(worker_response::Payload::IndexFiles(value)) => {
                let canonical_json =
                    index_response_body(value.canonical_json, &value.blobs, &self.transport_root)?;
                validate_typed_string(&canonical_json, "/indexHash", &value.index_hash)?;
                validate_typed_string(
                    &canonical_json,
                    "/projectModelHash",
                    &value.project_model_hash,
                )?;
                validate_typed_string(&canonical_json, "/compilation", &value.compilation)?;
                validate_typed_count(&canonical_json, "/files", value.file_count)?;
                validate_typed_bool(&canonical_json, "/partial", value.partial)?;
                canonical_json
            }
            Some(worker_response::Payload::ResolveSymbol(value)) => {
                validate_typed_string(
                    &value.canonical_json,
                    "/declaration/symbolId",
                    &value.symbol_id,
                )?;
                value.canonical_json
            }
            Some(worker_response::Payload::ResolveExpression(value)) => {
                validate_typed_string(&value.canonical_json, "/anchor/anchorId", &value.anchor_id)?;
                value.canonical_json
            }
            Some(worker_response::Payload::BuildLocalGraph(value)) => {
                validate_typed_string(&value.canonical_json, "/symbol", &value.symbol_id)?;
                validate_typed_count(&value.canonical_json, "/nodes", value.node_count)?;
                validate_typed_count(&value.canonical_json, "/edges", value.edge_count)?;
                value.canonical_json
            }
            Some(worker_response::Payload::ApplyEdit(value)) => {
                validate_typed_string(
                    &value.canonical_json,
                    "/candidateHash",
                    &value.candidate_hash,
                )?;
                value.canonical_json
            }
            Some(worker_response::Payload::ValidateCandidate(value)) => {
                let canonical: Value =
                    serde_json::from_slice(&value.canonical_json).map_err(internal)?;
                if canonical.get("valid").and_then(Value::as_bool) != Some(value.valid) {
                    return Err(ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "typed validation result disagrees with canonical payload",
                    ));
                }
                value.canonical_json
            }
            Some(worker_response::Payload::Shutdown(value)) => value.canonical_json,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker response has unexpected payload",
                ));
            }
        };
        let json_started = Instant::now();
        let mut value: Value = serde_json::from_slice(&canonical_json).map_err(internal)?;
        let json_micros = json_started.elapsed().as_micros() as u64;
        if kind == RequestKind::ApplyEdit && value.get("source").is_none() {
            let blob = value.get("sourceBlob").ok_or_else(|| {
                ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "candidate response has neither inline source nor BlobRef",
                )
            })?;
            let source = read_response_blob(payload, blob)?;
            value["source"] = Value::String(String::from_utf8(source).map_err(|error| {
                ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
            })?);
        }
        if kind == RequestKind::OpenProject {
            let project_compiler = value
                .get("declaredCompilerVersion")
                .or_else(|| value.get("compilerVersion"))
                .and_then(Value::as_str)
                .unwrap_or(self.capabilities.compiler_version.as_str());
            let desired = WorkerVariant::for_project(project_compiler)?;
            if desired != self.variant {
                let tried = tried_discovery_variants | self.variant.discovery_bit();
                if tried & desired.discovery_bit() != 0 {
                    return Err(ClewError::new(
                        ErrorCode::UnsupportedCompilerPluginAbi,
                        "declared Kotlin compiler variant could not open the project with its own compiler plugins",
                    ));
                }
                self.switch_variant(desired)?;
                return self.request_with_discovery_variants(kind, payload, tried);
            }
            self.snapshot = Some(SnapshotId {
                base_revision: snapshot_from(payload).base_revision,
                project_model_hash: value
                    .get("projectModelHash")
                    .and_then(Value::as_str)
                    .unwrap_or("UNRESOLVED")
                    .into(),
            });
        }
        let profiling = take_worker_profiling(&mut value);
        let worker_processing_micros = profiling
            .as_ref()
            .and_then(|profile| profile.get("workerProcessingMicros"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.last_profile = RequestProfile {
            serialization_micros: request_construction_micros
                + encode_micros
                + decode_micros
                + json_micros,
            ipc_micros: (write_micros + read_micros).saturating_sub(worker_processing_micros),
            worker_processing_micros,
            cache_requests: profiling
                .as_ref()
                .and_then(|profile| profile.get("cacheRequests"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_hits: profiling
                .as_ref()
                .and_then(|profile| profile.get("cacheHits"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            psi_parse_micros: profiling
                .as_ref()
                .and_then(|profile| profile.get("psiParseMicros"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            k2_analysis_micros: profiling
                .as_ref()
                .and_then(|profile| profile.get("k2AnalysisMicros"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            fir_extraction_micros: profiling
                .as_ref()
                .and_then(|profile| profile.get("firExtractionMicros"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            compiler_index: (kind == RequestKind::IndexFiles
                && payload.get("syntaxOnly").and_then(Value::as_bool) != Some(true))
            .then(|| profiling.as_ref().and_then(parse_compiler_index_profile))
            .flatten(),
            project_model_cache: profiling
                .as_ref()
                .and_then(parse_project_model_cache_profile),
        };
        Ok(value)
    }

    /// Issue a `SOURCE_SYNTAX` capability without opening or trusting a build
    /// model. The capability proves only the current file/declaration payload;
    /// compiler-semantic consumers cannot accept this distinct type.
    pub fn index_files_source_syntax_verified(
        &mut self,
        payload: &Value,
    ) -> Result<VerifiedSourceSyntax, ClewError> {
        if payload.get("syntaxOnly").and_then(Value::as_bool) != Some(true) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX indexing requires syntaxOnly true",
            ));
        }
        let repo = payload
            .get("repo")
            .and_then(Value::as_str)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "SOURCE_SYNTAX needs repo"))?;
        let repo = Path::new(repo).canonicalize().map_err(internal)?;
        let metadata = std::fs::symlink_metadata(&repo).map_err(internal)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX repo must be a real directory",
            ));
        }
        let requested_compilation = payload
            .get("compilation")
            .and_then(Value::as_str)
            .unwrap_or(":/main")
            .to_owned();
        let requested_files = match payload.get("files") {
            Some(Value::Array(files)) => files
                .iter()
                .map(|file| {
                    file.as_str().map(str::to_owned).ok_or_else(|| {
                        ClewError::new(
                            ErrorCode::InvalidInput,
                            "SOURCE_SYNTAX files must be strings",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "SOURCE_SYNTAX requires an exact files array",
                ));
            }
        };
        let trusted = self.trusted_distribution.as_ref().ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "SOURCE_SYNTAX requires the pinned workspace worker distribution",
            )
        })?;
        if trusted.workspace != workspace_root() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "SOURCE_SYNTAX worker workspace identity changed",
            ));
        }
        verify_trusted_distribution(trusted)?;
        let trusted_tree_hash = trusted.tree_hash.clone();
        let trusted_build_input_digest = trusted.build_input_digest.clone();

        let mut exact_payload = payload.clone();
        if !exact_payload.is_object() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "SOURCE_SYNTAX request must be an object",
            ));
        }
        exact_payload["repo"] = Value::String(repo.to_string_lossy().into_owned());
        exact_payload["compilation"] = Value::String(requested_compilation.clone());
        exact_payload["syntaxOnly"] = Value::Bool(true);
        let facts = self.request(RequestKind::IndexFiles, &exact_payload)?;
        let source_manifest_hash = validate_source_syntax_response(
            &repo,
            &requested_compilation,
            &requested_files,
            &facts,
        )?;
        let payload_hash = crate::canonical::hash(&facts).map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        let seal = crate::canonical::hash(&serde_json::json!({
            "authority":SOURCE_SYNTAX_AUTHORITY,
            "receiptId":receipt_id,
            "session":self.authority_session,
            "repo":repo,
            "compilation":requested_compilation,
            "distributionTree":trusted_tree_hash,
            "buildInputs":trusted_build_input_digest,
            "requestedFiles":requested_files,
            "sourceManifestHash":source_manifest_hash,
            "payloadHash":payload_hash,
        }))
        .map_err(internal)?;
        self.issued_source_syntax.insert(receipt_id, seal);
        Ok(VerifiedSourceSyntax {
            receipt_id,
            authority_session: self.authority_session,
            repo,
            compilation: requested_compilation,
            distribution_tree_hash: trusted_tree_hash,
            build_input_digest: trusted_build_input_digest,
            requested_files,
            source_manifest_hash,
            payload_hash,
            payload: facts,
        })
    }

    /// Inspect current declaration-only facts. This grants no persistence or
    /// transaction authority and fails if any bound source changed.
    pub fn inspect_verified_source_syntax<'a>(
        &self,
        syntax: &'a VerifiedSourceSyntax,
    ) -> Result<&'a Value, ClewError> {
        let trusted = self.trusted_distribution.as_ref().ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "SOURCE_SYNTAX worker session is not trusted",
            )
        })?;
        verify_trusted_distribution(trusted)?;
        let current_manifest_hash = validate_source_syntax_response(
            &syntax.repo,
            &syntax.compilation,
            &syntax.requested_files,
            &syntax.payload,
        )?;
        let seal = crate::canonical::hash(&serde_json::json!({
            "authority":SOURCE_SYNTAX_AUTHORITY,
            "receiptId":syntax.receipt_id,
            "session":syntax.authority_session,
            "repo":syntax.repo,
            "compilation":syntax.compilation,
            "distributionTree":syntax.distribution_tree_hash,
            "buildInputs":syntax.build_input_digest,
            "requestedFiles":syntax.requested_files,
            "sourceManifestHash":syntax.source_manifest_hash,
            "payloadHash":syntax.payload_hash,
        }))
        .map_err(internal)?;
        if syntax.authority_session != self.authority_session
            || syntax.distribution_tree_hash != trusted.tree_hash
            || syntax.build_input_digest != trusted.build_input_digest
            || current_manifest_hash != syntax.source_manifest_hash
            || crate::canonical::hash(&syntax.payload).map_err(internal)? != syntax.payload_hash
            || self.issued_source_syntax.get(&syntax.receipt_id) != Some(&seal)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "SOURCE_SYNTAX capability is stale, altered, forged, or belongs to another session",
            ));
        }
        Ok(&syntax.payload)
    }

    /// Execute OpenProject + IndexFiles through the pinned worker distribution
    /// and issue an unforgeable, session-local capability for the exact result.
    pub fn index_files_verified(
        &mut self,
        payload: &Value,
    ) -> Result<VerifiedIndexFacts, ClewError> {
        self.open_project_and_index_verified(payload)
            .map(|(_, facts)| facts)
    }

    /// Execute one live OpenProject followed by IndexFiles and return both the
    /// exact project authority and its sealed semantic facts. Callers that need
    /// the project model must use this combined contour instead of issuing a
    /// second independent OpenProject, which is intentionally refreshed in
    /// project-native mode.
    pub fn open_project_and_index_verified(
        &mut self,
        payload: &Value,
    ) -> Result<(Value, VerifiedIndexFacts), ClewError> {
        let repo = payload
            .get("repo")
            .and_then(Value::as_str)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "verified index needs repo"))?;
        let repo = Path::new(repo).canonicalize().map_err(internal)?;
        let requested_compilation = payload
            .get("compilation")
            .and_then(Value::as_str)
            .unwrap_or(":/main")
            .to_owned();
        let project = self.request(
            RequestKind::OpenProject,
            &serde_json::json!({"repo":repo,"compilation":requested_compilation}),
        )?;
        let facts = self.index_files_verified_after_project(payload, &project)?;
        Ok((project, facts))
    }

    /// Index against the exact live OpenProject currently held by this worker.
    /// This supports cache lookup between build discovery and semantic indexing
    /// without a second native Maven/Gradle model extraction. The supplied model
    /// must still match the worker's live snapshot and the IndexFiles response.
    pub fn index_files_verified_after_project(
        &mut self,
        payload: &Value,
        project: &Value,
    ) -> Result<VerifiedIndexFacts, ClewError> {
        if payload.get("syntaxOnly").and_then(Value::as_bool) == Some(true) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "COMPILER_SEMANTIC indexing cannot use syntaxOnly",
            ));
        }
        let repo = payload
            .get("repo")
            .and_then(Value::as_str)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "verified index needs repo"))?;
        let repo = Path::new(repo).canonicalize().map_err(internal)?;
        let requested_compilation = payload
            .get("compilation")
            .and_then(Value::as_str)
            .unwrap_or(":/main")
            .to_owned();
        let open_project_model_cache = self.last_profile.project_model_cache.clone();
        if project.get("compilation").and_then(Value::as_str)
            != Some(requested_compilation.as_str())
            || project.get("compilerVersion").and_then(Value::as_str)
                != Some(self.capabilities.compiler_version.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "OpenProject identity differs from verified index request",
            ));
        }
        let provided_project_model_hash = required_payload_string(project, "projectModelHash")?;
        if self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.project_model_hash.as_str())
            != Some(provided_project_model_hash)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "provided OpenProject is not the worker's live project authority",
            ));
        }
        // OpenProject may discover and switch to the project's Kotlin worker
        // variant. Bind the semantic receipt to that selected distribution,
        // never to the bootstrap variant that happened to start the session.
        let trusted = self.trusted_distribution.as_ref().ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "verified index facts require the pinned workspace worker distribution",
            )
        })?;
        if trusted.workspace != workspace_root() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "verified index worker workspace identity changed",
            ));
        }
        let trusted_plugin_fingerprint = trusted.plugin_fingerprint.clone();
        verify_trusted_distribution(trusted)?;
        let trusted_tree_hash = trusted.tree_hash.clone();
        let trusted_build_input_digest = trusted.build_input_digest.clone();
        let project_model_hash = provided_project_model_hash.to_owned();
        let semantic_input_manifest_hash =
            required_payload_string(project, "semanticInputManifestHash")?.to_owned();
        let manifest = project.get("semanticInputManifest").ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "OpenProject has no semantic input manifest",
            )
        })?;
        if crate::canonical::hash(manifest).map_err(internal)? != semantic_input_manifest_hash {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "OpenProject semantic input manifest hash is invalid",
            ));
        }
        let mut exact_payload = payload.clone();
        exact_payload["repo"] = Value::String(repo.to_string_lossy().into_owned());
        exact_payload["compilation"] = Value::String(requested_compilation.clone());
        let index_result = self.request(RequestKind::IndexFiles, &exact_payload);
        retain_verified_index_project_model_profile(
            &mut self.last_profile,
            open_project_model_cache,
        );
        let mut facts = index_result
            .map_err(|error| attach_verified_index_failure(error, "RAW_SCHEMA_HASH", None))?;
        if facts.get("compilation").and_then(Value::as_str) != Some(requested_compilation.as_str())
            || facts.get("projectModelHash").and_then(Value::as_str)
                != Some(project_model_hash.as_str())
            || facts.get("compilerVersion").and_then(Value::as_str)
                != Some(self.capabilities.compiler_version.as_str())
            || facts
                .get("semanticInputManifestHash")
                .and_then(Value::as_str)
                != Some(semantic_input_manifest_hash.as_str())
            || facts.get("semanticInputManifest") != project.get("semanticInputManifest")
        {
            return Err(attach_verified_index_failure(
                ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "IndexFiles identity differs from OpenProject/request",
                ),
                "SOURCE_BINDING",
                Some(&facts),
            ));
        }
        require_k2_validated(&facts)
            .map_err(|error| attach_verified_index_failure(error, "K2_VALIDATION", Some(&facts)))?;
        if facts.get("analysisMode").and_then(Value::as_str) != Some("K2_SEMANTIC") {
            return Err(attach_verified_index_failure(
                ClewError::new(
                    ErrorCode::IncompleteSemanticAnalysis,
                    "COMPILER_SEMANTIC response requires analysisMode K2_SEMANTIC",
                ),
                "K2_VALIDATION",
                Some(&facts),
            ));
        }
        let descriptor =
            crate::index::validate_declaration_descriptor_snapshot(&facts).map_err(|error| {
                attach_verified_index_failure(error, "DESCRIPTOR_GRAPH", Some(&facts))
            })?;
        normalize_optional_relation_evidence(&mut facts).map_err(|error| {
            attach_verified_index_failure(error, "RELATION_NORMALIZATION", Some(&facts))
        })?;
        let relation =
            crate::index::validate_declaration_relation_snapshot(&facts).map_err(|error| {
                attach_verified_index_failure(error, "RELATION_GRAPH", Some(&facts))
            })?;
        for provenance in [&relation.provenance, &descriptor.provenance] {
            if provenance
                .get("pluginArtifactFingerprint")
                .and_then(Value::as_str)
                != Some(trusted_plugin_fingerprint.as_str())
                || provenance.get("workerVersion").and_then(Value::as_str)
                    != Some(self.capabilities.worker_version.as_str())
                || provenance
                    .get("workerCompilerVersion")
                    .and_then(Value::as_str)
                    != Some(self.capabilities.compiler_version.as_str())
                || provenance
                    .get("workerProtocolVersion")
                    .and_then(Value::as_str)
                    != Some("1.0")
            {
                return Err(attach_verified_index_failure(
                    ClewError::new(
                        ErrorCode::WorkerProtocolMismatch,
                        "semantic graph provenance differs from live worker distribution",
                    ),
                    "DISTRIBUTION_PROVENANCE",
                    Some(&facts),
                ));
            }
        }
        let base_revision = git_revision(&repo)?;
        let payload_hash = crate::canonical::hash(&facts).map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        let seal = crate::canonical::hash(&serde_json::json!({
            "receiptId":receipt_id,
            "session":self.authority_session,
            "repo":repo,
            "compilation":requested_compilation,
            "baseRevision":base_revision,
            "projectModelHash":project_model_hash,
            "distribution":trusted_plugin_fingerprint,
            "distributionTree":trusted_tree_hash,
            "buildInputs":trusted_build_input_digest,
            "payloadHash":payload_hash,
            "relationHash":relation.hash,
            "descriptorHash":descriptor.hash,
        }))
        .map_err(internal)?;
        self.issued_index_facts.insert(receipt_id, seal);
        Ok(VerifiedIndexFacts {
            receipt_id,
            authority_session: self.authority_session,
            repo,
            compilation: requested_compilation,
            base_revision,
            project_model_hash,
            distribution_fingerprint: trusted_plugin_fingerprint,
            distribution_tree_hash: trusted_tree_hash,
            build_input_digest: trusted_build_input_digest,
            payload_hash,
            relation_hash: relation.hash,
            descriptor_hash: descriptor.hash,
            payload: facts,
        })
    }

    /// Read-only projection of a capability issued by this exact session.
    pub fn inspect_verified_index<'a>(
        &self,
        facts: &'a VerifiedIndexFacts,
    ) -> Result<&'a Value, ClewError> {
        let seal = crate::canonical::hash(&serde_json::json!({
            "receiptId":facts.receipt_id,
            "session":facts.authority_session,
            "repo":facts.repo,
            "compilation":facts.compilation,
            "baseRevision":facts.base_revision,
            "projectModelHash":facts.project_model_hash,
            "distribution":facts.distribution_fingerprint,
            "distributionTree":facts.distribution_tree_hash,
            "buildInputs":facts.build_input_digest,
            "payloadHash":facts.payload_hash,
            "relationHash":facts.relation_hash,
            "descriptorHash":facts.descriptor_hash,
        }))
        .map_err(internal)?;
        if facts.authority_session != self.authority_session
            || self.issued_index_facts.get(&facts.receipt_id) != Some(&seal)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "verified index capability is not issued by this live session",
            ));
        }
        Ok(&facts.payload)
    }

    pub(crate) fn safe_verified_index_diagnostic(&self, facts: &VerifiedIndexFacts) -> Value {
        serde_json::json!({
            "authoritySessionHash": crate::canonical::hash(&facts.authority_session).unwrap_or_else(|_| "unavailable".into()),
            "currentSessionHash": crate::canonical::hash(&self.authority_session).unwrap_or_else(|_| "unavailable".into()),
            "distributionFingerprintHash": crate::canonical::hash(&facts.distribution_fingerprint).unwrap_or_else(|_| "unavailable".into()),
            "distributionTreeHash": crate::canonical::hash(&facts.distribution_tree_hash).unwrap_or_else(|_| "unavailable".into()),
            "buildInputDigestHash": crate::canonical::hash(&facts.build_input_digest).unwrap_or_else(|_| "unavailable".into()),
            "pluginFingerprintHash": crate::canonical::hash(&facts.distribution_fingerprint).unwrap_or_else(|_| "unavailable".into()),
            "projectModelHashHash": crate::canonical::hash(&facts.project_model_hash).unwrap_or_else(|_| "unavailable".into()),
            "sourceSnapshotHash": crate::canonical::hash(&facts.base_revision).unwrap_or_else(|_| "unavailable".into()),
            "payloadHashHash": crate::canonical::hash(&facts.payload_hash).unwrap_or_else(|_| "unavailable".into()),
            "receiptIssued": self.issued_index_facts.contains_key(&facts.receipt_id),
            "sessionMatches": facts.authority_session == self.authority_session,
        })
    }

    pub(crate) fn authorize_index_facts<'a>(
        &self,
        facts: &'a VerifiedIndexFacts,
        source_root: &Path,
        compilation: &str,
    ) -> Result<&'a Value, ClewError> {
        let source_root = source_root.canonicalize().map_err(internal)?;
        let trusted = self.trusted_distribution.as_ref().ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker session is not trusted",
            )
        })?;
        verify_trusted_distribution(trusted)?;
        let seal = crate::canonical::hash(&serde_json::json!({
            "receiptId":facts.receipt_id,
            "session":facts.authority_session,
            "repo":facts.repo,
            "compilation":facts.compilation,
            "baseRevision":facts.base_revision,
            "projectModelHash":facts.project_model_hash,
            "distribution":facts.distribution_fingerprint,
            "distributionTree":facts.distribution_tree_hash,
            "buildInputs":facts.build_input_digest,
            "payloadHash":facts.payload_hash,
            "relationHash":facts.relation_hash,
            "descriptorHash":facts.descriptor_hash,
        }))
        .map_err(internal)?;
        if facts.authority_session != self.authority_session
            || source_root != facts.repo
            || compilation != facts.compilation
            || facts.distribution_fingerprint != trusted.plugin_fingerprint
            || facts.distribution_tree_hash != trusted.tree_hash
            || facts.build_input_digest != trusted.build_input_digest
            || crate::canonical::hash(&facts.payload).map_err(internal)? != facts.payload_hash
            || facts
                .payload
                .get("projectModelHash")
                .and_then(Value::as_str)
                != Some(facts.project_model_hash.as_str())
            || git_revision(&source_root)? != facts.base_revision
            || self.issued_index_facts.get(&facts.receipt_id) != Some(&seal)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "verified index capability is stale, altered, forged, or belongs to another session",
            ));
        }
        require_k2_validated(&facts.payload)?;
        let relation = crate::index::validate_declaration_relation_snapshot(&facts.payload)?;
        let descriptor = crate::index::validate_declaration_descriptor_snapshot(&facts.payload)?;
        if relation.hash != facts.relation_hash || descriptor.hash != facts.descriptor_hash {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "verified semantic graph hash changed after issuance",
            ));
        }
        Ok(&facts.payload)
    }

    pub fn shutdown(mut self) -> Result<(), ClewError> {
        let _ = self.request(RequestKind::Shutdown, &serde_json::json!({}))?;
        let status = self
            .child
            .wait()
            .map_err(|e| ClewError::new(ErrorCode::WorkerCrashed, e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("worker exited with {status}"),
            ))
        }
    }

    pub fn validate_candidates_batch(
        &mut self,
        candidates: &[(String, String)],
    ) -> Result<Vec<Value>, ClewError> {
        let snapshot = self.snapshot.clone().unwrap_or(SnapshotId {
            base_revision: "UNVERSIONED".into(),
            project_model_hash: "UNRESOLVED".into(),
        });
        let mut requests = Vec::with_capacity(candidates.len());
        for (file, source) in candidates {
            let request_id = self.next_id;
            self.next_id += 1;
            requests.push(WorkerRequest {
                request_id,
                protocol_version: Some(ProtocolVersion { major: 1, minor: 0 }),
                snapshot: Some(snapshot.clone()),
                payload: Some(worker_request::Payload::ValidateCandidate(
                    ValidateCandidateRequest {
                        schema_version: Some(SchemaVersion { major: 1, minor: 0 }),
                        repo: String::new(),
                        file: file.clone(),
                        source_inline: source.as_bytes().to_vec(),
                        source_blob: None,
                    },
                )),
            });
        }
        let request_id = self.next_id;
        self.next_id += 1;
        write_message(
            &mut self.stdin,
            &WorkerRequest {
                request_id,
                protocol_version: Some(ProtocolVersion { major: 1, minor: 0 }),
                snapshot: Some(snapshot),
                payload: Some(worker_request::Payload::Batch(BatchRequest {
                    schema_version: Some(SchemaVersion { major: 1, minor: 0 }),
                    requests,
                })),
            },
        )?;
        let response = read_message(&mut self.stdout)?;
        if response.request_id != request_id {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "batch response request_id mismatch",
            ));
        }
        let batch = match response.payload {
            Some(worker_response::Payload::Batch(batch)) => batch,
            _ => {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker returned no BatchResponse",
                ));
            }
        };
        batch
            .responses
            .into_iter()
            .map(|response| match response.payload {
                Some(worker_response::Payload::ValidateCandidate(value)) => {
                    serde_json::from_slice(&value.canonical_json).map_err(internal)
                }
                Some(worker_response::Payload::Error(error)) => Err(ClewError {
                    code: parse_worker_code(&error.code),
                    message: error.message,
                    transaction_id: None,
                    snapshot_id: None,
                    evidence: error.evidence,
                    relevant_anchors_or_symbols: vec!["batch-validation".into()].into_boxed_slice(),
                    retryable: error.retryable,
                }),
                _ => Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "unexpected response inside batch",
                )),
            })
            .collect()
    }
}

fn request_requires_project_snapshot(kind: RequestKind, payload: &Value) -> bool {
    match kind {
        RequestKind::OpenProject | RequestKind::ValidateCandidate | RequestKind::Shutdown => false,
        RequestKind::IndexFiles => payload.get("syntaxOnly").and_then(Value::as_bool) != Some(true),
        _ => true,
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn json_string(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}

fn json_optional_string(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn snapshot_from(payload: &Value) -> SnapshotId {
    let repo = payload.get("repo").and_then(Value::as_str);
    let base_revision = repo
        .and_then(|repo| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(repo)
                .output()
                .ok()
                .filter(|output| output.status.success())
        })
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "UNVERSIONED".into());
    SnapshotId {
        base_revision,
        project_model_hash: payload
            .get("projectModelHash")
            .and_then(Value::as_str)
            .unwrap_or("UNRESOLVED")
            .into(),
    }
}

fn snapshot_label(snapshot: &SnapshotId) -> String {
    format!(
        "snapshot:{}:{}",
        snapshot.base_revision, snapshot.project_model_hash
    )
}

fn source_transport(
    payload: &Value,
    _build_state_root: Option<&Path>,
) -> Result<(Vec<u8>, Option<BlobRef>), ClewError> {
    let source = payload
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .as_bytes()
        .to_vec();
    // Request transport is always inline. Mutable transport CAS belongs to the
    // private worker process state; the target repository is never a blob store.
    Ok((source, None))
}

fn validate_typed_string(canonical: &[u8], pointer: &str, typed: &str) -> Result<(), ClewError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value.pointer(pointer).and_then(Value::as_str) != Some(typed) {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response field disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn validate_typed_count(canonical: &[u8], pointer: &str, typed: u64) -> Result<(), ClewError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| items.len() as u64)
        != Some(typed)
    {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response count disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn validate_typed_bool(canonical: &[u8], pointer: &str, typed: bool) -> Result<(), ClewError> {
    let value: Value = serde_json::from_slice(canonical).map_err(internal)?;
    if value.pointer(pointer).and_then(Value::as_bool) != Some(typed) {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            format!("typed response boolean disagrees with canonical payload at {pointer}"),
        ));
    }
    Ok(())
}

fn read_response_blob(payload: &Value, blob: &Value) -> Result<Vec<u8>, ClewError> {
    let repo = payload
        .get("repo")
        .and_then(Value::as_str)
        .ok_or_else(|| ClewError::new(ErrorCode::WorkerProtocolMismatch, "BlobRef has no repo"))?;
    let relative = blob
        .get("relativePath")
        .and_then(Value::as_str)
        .ok_or_else(|| ClewError::new(ErrorCode::WorkerProtocolMismatch, "BlobRef has no path"))?;
    if relative.starts_with('/') || relative.split('/').any(|part| part == "..") {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "response BlobRef escapes repository",
        ));
    }
    let bytes = std::fs::read(Path::new(repo).join(relative))
        .map_err(|error| ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string()))?;
    let expected = blob
        .get("contentHash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if crate::canonical::hash_bytes(&bytes) != expected {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "response BlobRef content hash mismatch",
        ));
    }
    Ok(bytes)
}

fn index_response_body(
    inline: Vec<u8>,
    blobs: &[BlobRef],
    transport_root: &Path,
) -> Result<Vec<u8>, ClewError> {
    match (inline.is_empty(), blobs) {
        (false, []) => Ok(inline),
        (true, [blob]) => read_worker_transport_blob(transport_root, blob),
        (false, _) => Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "IndexFiles response contains both inline and blob bodies",
        )),
        (true, []) => Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "IndexFiles response has no canonical body",
        )),
        (true, _) => Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "IndexFiles response has multiple canonical body blobs",
        )),
    }
}

fn read_worker_transport_blob(root: &Path, blob: &BlobRef) -> Result<Vec<u8>, ClewError> {
    fn mismatch(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::WorkerProtocolMismatch, message)
    }

    if !root.is_absolute() {
        return Err(mismatch("worker transport root is not absolute"));
    }
    let root_metadata =
        std::fs::symlink_metadata(root).map_err(|error| mismatch(error.to_string()))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(mismatch("worker transport root is not a real directory"));
    }
    if root
        .canonicalize()
        .map_err(|error| mismatch(error.to_string()))?
        != root
    {
        return Err(mismatch("worker transport root canonical identity changed"));
    }

    let digest = blob
        .content_hash
        .strip_prefix("sha256:")
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| mismatch("worker transport BlobRef hash is malformed"))?;
    let expected_relative = format!("sha256/{digest}");
    if blob.relative_path != expected_relative {
        return Err(mismatch(
            "worker transport BlobRef path is not its content-addressed name",
        ));
    }
    if blob.size_bytes == 0 || blob.size_bytes > MAX_WORKER_RESPONSE_BLOB_BYTES {
        return Err(mismatch(
            "worker transport BlobRef size exceeds the bounded response policy",
        ));
    }

    let directory = root.join("sha256");
    let directory_metadata =
        std::fs::symlink_metadata(&directory).map_err(|error| mismatch(error.to_string()))?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(mismatch("worker transport CAS directory is unsafe"));
    }
    if directory
        .canonicalize()
        .map_err(|error| mismatch(error.to_string()))?
        .parent()
        != Some(root)
    {
        return Err(mismatch("worker transport CAS directory escapes its root"));
    }

    let path = root.join(&blob.relative_path);
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| mismatch(error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != blob.size_bytes
    {
        return Err(mismatch(
            "worker transport blob is not the declared regular file",
        ));
    }
    let file = std::fs::File::open(&path).map_err(|error| mismatch(error.to_string()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| mismatch(error.to_string()))?;
    if !opened_metadata.is_file() || opened_metadata.len() != blob.size_bytes {
        return Err(mismatch("worker transport blob changed while opening"));
    }
    let mut bytes = Vec::with_capacity(blob.size_bytes as usize);
    file.take(MAX_WORKER_RESPONSE_BLOB_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| mismatch(error.to_string()))?;
    if bytes.len() as u64 != blob.size_bytes {
        return Err(mismatch("worker transport blob size changed while reading"));
    }
    if crate::canonical::hash_bytes(&bytes) != blob.content_hash {
        return Err(mismatch("worker transport blob content hash mismatch"));
    }
    let _ = std::fs::remove_file(path);
    Ok(bytes)
}

fn worker_launcher(workspace: &Path, variant: WorkerVariant) -> PathBuf {
    match variant {
        WorkerVariant::Kotlin21 => {
            workspace.join("workers/kotlin21/build/install/kotlin21/bin/kotlin21")
        }
        WorkerVariant::Kotlin23 => {
            workspace.join("workers/kotlin23/build/install/kotlin23/bin/kotlin23")
        }
        WorkerVariant::Kotlin24 => workspace.join("workers/kotlin/build/install/kotlin/bin/kotlin"),
    }
}

fn prepare_trusted_worker_distribution(
    workspace: &Path,
    variant: WorkerVariant,
) -> Result<Option<TrustedWorkerDistribution>, ClewError> {
    let canonical = workspace.canonicalize().map_err(internal)?;
    if let Some(runtime) = RuntimeAuthority::from_environment()? {
        if runtime.root != canonical {
            return Err(preparation_required(
                "runtime workspace differs from the verified capsule root",
            ));
        }
        let runtime_worker = runtime.worker(variant.runtime_name())?;
        if runtime_worker.compiler_version != variant.compiler_version() {
            return Err(preparation_required(
                "runtime worker compiler identity differs from the selected variant",
            ));
        }
        let source_distribution = runtime.verify_worker(variant.runtime_name())?;
        let private_root = tempfile::Builder::new()
            .prefix("codeclew-worker-authority-")
            .tempdir()
            .map_err(internal)?;
        let distribution_root = private_root.path().join("distribution");
        copy_regular_tree(&source_distribution, &distribution_root)?;
        let tree_manifest = regular_tree_manifest(&distribution_root)?;
        let tree_hash = hash_string_manifest(&tree_manifest);
        if tree_hash != runtime_worker.tree_hash {
            return Err(preparation_required(
                "private worker copy differs from runtime authority",
            ));
        }
        let launcher = distribution_root.join("bin").join(variant.launcher_name());
        let plugin = distribution_root
            .join("lib")
            .join(variant.plugin_jar_name());
        if !launcher.is_file() || !plugin.is_file() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "runtime worker distribution is incomplete",
            ));
        }
        return Ok(Some(TrustedWorkerDistribution {
            workspace: canonical,
            _private_root: private_root,
            distribution_root,
            launcher,
            tree_manifest,
            tree_hash,
            build_input_digest: runtime.runtime_key,
            plugin_fingerprint: crate::canonical::hash_bytes(
                &std::fs::read(plugin).map_err(internal)?,
            ),
        }));
    }
    if canonical != workspace_root() {
        return Ok(None);
    }
    let pinned = variant.pinned_inputs();
    verify_pinned_build_inputs(&canonical, &pinned)?;
    reject_unpinned_workspace_build_initialization(&canonical)?;
    let source_distribution = canonical.join(variant.distribution_relative());
    if bootstrap_trusted_worker_distribution_if_missing(&canonical, variant, &source_distribution)?
    {
        // The build is not trusted merely because Gradle exited successfully.
        // Recheck the exact source closure before comparing its output with the
        // committed distribution manifest.
        verify_pinned_build_inputs(&canonical, &pinned)?;
    }
    verify_pinned_distribution(&source_distribution, &pinned)?;
    let private_root = tempfile::Builder::new()
        .prefix("codeclew-worker-authority-")
        .tempdir()
        .map_err(internal)?;
    let distribution_root = private_root.path().join("distribution");
    copy_regular_tree(&source_distribution, &distribution_root)?;
    let tree_manifest = regular_tree_manifest(&distribution_root)?;
    let tree_hash = hash_string_manifest(&tree_manifest);
    if tree_hash != pinned.output_digest {
        return Err(preparation_required(
            "private worker copy differs from embedded expected distribution",
        ));
    }
    let launcher = distribution_root.join("bin").join(variant.launcher_name());
    let plugin = distribution_root
        .join("lib")
        .join(variant.plugin_jar_name());
    if !launcher.is_file() || !plugin.is_file() {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "rebuilt pinned worker distribution is incomplete",
        ));
    }
    Ok(Some(TrustedWorkerDistribution {
        workspace: canonical,
        _private_root: private_root,
        distribution_root,
        launcher,
        tree_manifest,
        tree_hash,
        build_input_digest: pinned.digest.to_owned(),
        plugin_fingerprint: crate::canonical::hash_bytes(&std::fs::read(plugin).map_err(internal)?),
    }))
}

fn bootstrap_trusted_worker_distribution_if_missing(
    workspace: &Path,
    variant: WorkerVariant,
    distribution: &Path,
) -> Result<bool, ClewError> {
    bootstrap_trusted_worker_distribution_if_missing_with_environment(
        workspace,
        variant,
        distribution,
        std::env::vars_os(),
    )
}

fn bootstrap_trusted_worker_distribution_if_missing_with_environment(
    workspace: &Path,
    variant: WorkerVariant,
    distribution: &Path,
    build_environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<bool, ClewError> {
    match std::fs::symlink_metadata(distribution) {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(internal(error)),
    }

    require_safe_missing_distribution_path(workspace, distribution)?;
    let wrapper = workspace.join("gradlew");
    let metadata = std::fs::symlink_metadata(&wrapper).map_err(internal)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(preparation_required(
            "trusted worker Gradle wrapper is not a regular pinned file",
        ));
    }

    let mut command = Command::new(&wrapper);
    command
        .args([variant.install_task(), "--no-daemon", "--quiet"])
        .current_dir(workspace)
        // A cold start must resolve the worker with the same Gradle/JVM setup
        // as the repository wrapper: caller caches, mirrors and JVM options are
        // part of build availability. They are not trusted as authority. The
        // pinned source closure is checked before and after this command and
        // the resulting installDist must match the embedded output manifest.
        .env_clear()
        .envs(build_environment);
    let output = command.output().map_err(|error| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("cannot build trusted Kotlin worker: {error}"),
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let diagnostic = stderr.chars().take(2_048).collect::<String>();
        return Err(ClewError::new(
            ErrorCode::WorkerCrashed,
            format!(
                "trusted Kotlin worker build failed with {}{}{}",
                output.status,
                if diagnostic.trim().is_empty() {
                    ""
                } else {
                    ": "
                },
                diagnostic.trim()
            ),
        ));
    }
    Ok(true)
}

fn require_safe_missing_distribution_path(
    workspace: &Path,
    distribution: &Path,
) -> Result<(), ClewError> {
    let relative = distribution.strip_prefix(workspace).map_err(|_| {
        preparation_required("trusted worker distribution path escapes its workspace")
    })?;
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(preparation_required(
                        "trusted worker distribution path has a non-directory or symlinked ancestor",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(internal(error)),
        }
    }
    Ok(())
}

fn verify_pinned_build_inputs(workspace: &Path, pinned: &PinnedInputs) -> Result<(), ClewError> {
    let actual = collect_regular_inputs(workspace, pinned.roots, pinned.files)?;
    let expected = pinned
        .entries
        .iter()
        .map(|(path, hash)| ((*path).to_owned(), (*hash).to_owned()))
        .collect::<BTreeMap<_, _>>();
    if actual != expected || hash_input_manifest(&actual) != pinned.digest {
        return Err(preparation_required(
            "worker build input closure differs from embedded pinned manifest",
        ));
    }
    Ok(())
}

fn reject_unpinned_workspace_build_initialization(workspace: &Path) -> Result<(), ClewError> {
    if workspace.join("init.gradle").exists() || workspace.join("init.gradle.kts").exists() {
        return Err(preparation_required(
            "trusted worker start refuses caller Gradle initialization",
        ));
    }
    Ok(())
}

fn verify_pinned_distribution(root: &Path, pinned: &PinnedInputs) -> Result<(), ClewError> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            preparation_required(
                "worker installDist is absent after the trusted distribution build",
            )
        } else {
            internal(error)
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(preparation_required(
            "worker installDist differs from embedded expected output manifest",
        ));
    }
    let actual = regular_tree_manifest(root)?;
    let expected = pinned
        .outputs
        .iter()
        .map(|(path, size, hash)| ((*path).to_owned(), format!("{size}:{hash}")))
        .collect::<BTreeMap<_, _>>();
    if actual != expected || hash_string_manifest(&actual) != pinned.output_digest {
        return Err(preparation_required(
            "worker installDist differs from embedded expected output manifest; run the trusted distribution build tool",
        ));
    }
    Ok(())
}

fn preparation_required(message: &str) -> ClewError {
    ClewError::new(ErrorCode::WorkerPreparationRequired, message)
}

fn collect_regular_inputs(
    workspace: &Path,
    roots: &[&str],
    files: &[&str],
) -> Result<BTreeMap<String, String>, ClewError> {
    let mut paths = Vec::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(workspace.join(root)).follow_links(false) {
            let entry = entry.map_err(internal)?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(internal)?;
            if metadata.file_type().is_symlink() {
                return Err(ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "worker build input closure contains a symlink",
                ));
            }
            if metadata.is_file() {
                paths.push(entry.path().to_path_buf());
            }
        }
    }
    paths.extend(files.iter().map(|path| workspace.join(path)));
    paths.sort();
    paths.dedup();
    let mut manifest = BTreeMap::new();
    for path in paths {
        let metadata = std::fs::symlink_metadata(&path).map_err(internal)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "worker build input is missing or is not a regular file",
            ));
        }
        let relative = path
            .strip_prefix(workspace)
            .map_err(internal)?
            .to_string_lossy()
            .replace('\\', "/");
        manifest.insert(
            relative,
            crate::canonical::hash_bytes(&std::fs::read(path).map_err(internal)?),
        );
    }
    Ok(manifest)
}

fn copy_regular_tree(source: &Path, target: &Path) -> Result<(), ClewError> {
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(internal)?;
        let relative = entry.path().strip_prefix(source).map_err(internal)?;
        let destination = target.join(relative);
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(internal)?;
        if metadata.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "rebuilt worker distribution contains a symlink",
            ));
        }
        if metadata.is_dir() {
            std::fs::create_dir_all(&destination).map_err(internal)?;
        } else if metadata.is_file() {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(internal)?;
            }
            std::fs::copy(entry.path(), &destination).map_err(internal)?;
            std::fs::set_permissions(&destination, metadata.permissions()).map_err(internal)?;
        } else {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "rebuilt worker distribution contains a special file",
            ));
        }
    }
    Ok(())
}

fn regular_tree_manifest(root: &Path) -> Result<BTreeMap<String, String>, ClewError> {
    let mut manifest = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(internal)?;
        let relative = entry.path().strip_prefix(root).map_err(internal)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let path = relative.to_string_lossy().replace('\\', "/");
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(internal)?;
        if metadata.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "private worker distribution contains a symlink",
            ));
        }
        if metadata.is_dir() {
            continue;
        } else if metadata.is_file() {
            let size = metadata.len();
            manifest.insert(
                path,
                format!(
                    "{size}:{}",
                    crate::canonical::hash_bytes(&std::fs::read(entry.path()).map_err(internal)?)
                ),
            );
        } else {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "private worker distribution contains a special file",
            ));
        }
    }
    Ok(manifest)
}

fn hash_string_manifest(manifest: &BTreeMap<String, String>) -> String {
    let mut bytes = Vec::new();
    for (path, size_and_hash) in manifest {
        let (size, hash) = size_and_hash.split_once(':').unwrap_or(("", size_and_hash));
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(size.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(hash.as_bytes());
        bytes.push(0);
    }
    crate::canonical::hash_bytes(&bytes)
}

fn hash_input_manifest(manifest: &BTreeMap<String, String>) -> String {
    let mut bytes = Vec::new();
    for (path, hash) in manifest {
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(hash.as_bytes());
        bytes.push(0);
    }
    crate::canonical::hash_bytes(&bytes)
}

fn verify_trusted_distribution(trusted: &TrustedWorkerDistribution) -> Result<(), ClewError> {
    let actual = regular_tree_manifest(&trusted.distribution_root)?;
    if actual != trusted.tree_manifest || hash_string_manifest(&actual) != trusted.tree_hash {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "private worker distribution changed after trusted launch",
        ));
    }
    Ok(())
}

fn required_payload_string<'a>(payload: &'a Value, field: &str) -> Result<&'a str, ClewError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                format!("worker response has no {field}"),
            )
        })
}

/// Return the mount-independent identity of one exact OpenProject response.
///
/// Gradle exposes provider digests that include absolute cache paths.  When it
/// also proves that both providers selected the exact ordered classpath, that
/// ordered, content-addressed classpath is the semantic authority; a detached
/// worktree or private CoW cache must not look like a model change.  A real
/// provider disagreement remains part of the identity and therefore remains
/// fail-closed.
pub(crate) fn stable_project_model_identity(project: &Value) -> Result<String, ClewError> {
    let raw_manifest = project.get("semanticInputManifest").ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "OpenProject response has no semantic input manifest",
        )
    })?;
    let raw_manifest_hash = required_payload_string(project, "semanticInputManifestHash")?;
    if crate::canonical::hash(raw_manifest).map_err(internal)? != raw_manifest_hash {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "OpenProject semantic input manifest hash is invalid",
        ));
    }

    let mut stable_manifest = raw_manifest.clone();
    let ordered_classpath = stable_manifest
        .get("orderedCompileClasspath")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "semantic input manifest has no ordered compile classpath",
            )
        })?;
    let mut ordered_bytes = Vec::new();
    for entry in ordered_classpath {
        let entry = entry.as_str().ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "semantic input classpath contains a non-string entry",
            )
        })?;
        ordered_bytes.extend_from_slice(entry.as_bytes());
        ordered_bytes.push(0);
    }
    let ordered_digest = Value::String(crate::canonical::hash_bytes(&ordered_bytes));
    if let Some(authority) = stable_manifest
        .get_mut("classpathAuthority")
        .and_then(Value::as_object_mut)
    {
        if authority.contains_key("orderedDigest") {
            authority.insert("orderedDigest".to_owned(), ordered_digest.clone());
        }
        if authority.get("orderedEquivalent").and_then(Value::as_bool) == Some(true) {
            for field in ["taskLibrariesDigest", "configurationDigest"] {
                if authority.get(field).is_some_and(|value| !value.is_null()) {
                    authority.insert(field.to_owned(), ordered_digest.clone());
                }
            }
        }
    }
    let stable_manifest_hash = crate::canonical::hash(&stable_manifest).map_err(internal)?;

    let mut stable_project = project.clone();
    let object = stable_project.as_object_mut().ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "OpenProject response is not an object",
        )
    })?;
    object.remove("projectModelHash");
    object.insert("semanticInputManifest".to_owned(), stable_manifest.clone());
    object.insert(
        "semanticInputManifestHash".to_owned(),
        Value::String(stable_manifest_hash),
    );
    if let Some(authority) = stable_manifest.get("classpathAuthority") {
        object.insert("classpathAuthority".to_owned(), authority.clone());
    }
    crate::canonical::hash(&stable_project).map_err(internal)
}

fn bind_open_project_compilation(
    canonical: &mut Value,
    typed_compilation: &str,
) -> Result<(), ClewError> {
    if typed_compilation.is_empty() {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "typed OpenProject response has no compilation identity",
        ));
    }
    let module = required_payload_string(canonical, "module")?;
    let source_set = required_payload_string(canonical, "sourceSet")?;
    let canonical_compilation = required_payload_string(canonical, "compilation")?;
    let derived = format!("{module}/{source_set}");
    if typed_compilation != derived || canonical_compilation != derived {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "typed OpenProject compilation differs from canonical module/sourceSet",
        ));
    }
    Ok(())
}

fn git_revision(repo: &Path) -> Result<String, ClewError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(internal)?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "verified index requires a committed Git revision",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn write_message<M: Message>(writer: &mut impl Write, message: &M) -> Result<(), ClewError> {
    let bytes = message.encode_to_vec();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|e| {
            ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("worker write failed: {e}"),
            )
        })
}

fn write_message_profiled<M: Message>(
    writer: &mut impl Write,
    message: &M,
) -> Result<(u64, u64), ClewError> {
    let encode_started = Instant::now();
    let bytes = message.encode_to_vec();
    let encode_micros = encode_started.elapsed().as_micros() as u64;
    let write_started = Instant::now();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|e| {
            ClewError::new(
                ErrorCode::WorkerCrashed,
                format!("worker write failed: {e}"),
            )
        })?;
    Ok((encode_micros, write_started.elapsed().as_micros() as u64))
}

fn read_message(reader: &mut impl Read) -> Result<WorkerResponse, ClewError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame header failed: {e}"),
        )
    })?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_WORKER_FRAME_BYTES {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "worker frame exceeds 64MiB",
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame body failed: {e}"),
        )
    })?;
    WorkerResponse::decode(bytes.as_slice())
        .map_err(|e| ClewError::new(ErrorCode::WorkerProtocolMismatch, e.to_string()))
}

fn read_message_profiled(reader: &mut impl Read) -> Result<(WorkerResponse, u64, u64), ClewError> {
    let read_started = Instant::now();
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame header failed: {e}"),
        )
    })?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_WORKER_FRAME_BYTES {
        return Err(ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "worker frame exceeds 64MiB",
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(|e| {
        ClewError::new(
            ErrorCode::WorkerCrashed,
            format!("worker frame body failed: {e}"),
        )
    })?;
    let read_micros = read_started.elapsed().as_micros() as u64;
    let decode_started = Instant::now();
    let response = WorkerResponse::decode(bytes.as_slice())
        .map_err(|e| ClewError::new(ErrorCode::WorkerProtocolMismatch, e.to_string()))?;
    Ok((
        response,
        read_micros,
        decode_started.elapsed().as_micros() as u64,
    ))
}

fn take_worker_profiling(value: &mut Value) -> Option<Value> {
    value
        .as_object_mut()
        .and_then(|object| object.remove("profiling"))
}

fn parse_compiler_index_profile(profiling: &Value) -> Option<CompilerIndexProfile> {
    let object = profiling.as_object()?;
    let backend = match object.get("backend")?.as_str()? {
        "BTA_PERSISTENT" => CompilerIndexBackend::BtaPersistent,
        _ => return None,
    };
    let status = match object.get("status")?.as_str()? {
        "UNCHANGED_HIT" => CompilerIndexStatus::UnchangedHit,
        "COLD_FULL" => CompilerIndexStatus::ColdFull,
        "INCREMENTAL" => CompilerIndexStatus::Incremental,
        "RECOVERED_FULL" => CompilerIndexStatus::RecoveredFull,
        "BUSY" => CompilerIndexStatus::Busy,
        "FAILED_RECOVERABLE" => CompilerIndexStatus::FailedRecoverable,
        _ => return None,
    };
    let valid = object.get("valid")?.as_bool()?;
    let total_micros = object.get("totalMicros")?.as_u64()?;
    let compiler_micros = object.get("compilerMicros")?.as_u64()?;
    let fir_extraction_micros = object.get("firExtractionMicros")?.as_u64()?;
    let total_files = object.get("totalFiles")?.as_u64()?;
    let compiled_files = object.get("compiledFiles")?.as_u64()?;
    let reused_files = object.get("reusedFiles")?.as_u64()?;
    let recovered = object.get("recovered")?.as_bool()?;
    let fallback_used = object.get("fallbackUsed")?.as_bool()?;
    let graph_digest = match object.get("graphDigest") {
        None | Some(Value::Null) => None,
        Some(Value::String(digest)) if is_lowercase_sha256(digest) => Some(digest.clone()),
        Some(_) => return None,
    };
    let semantic_input_manifest_digest = object
        .get("semanticInputManifestDigest")?
        .as_str()?
        .to_owned();
    let facts_plugin_digest = object.get("factsPluginDigest")?.as_str()?.to_owned();
    let extractor_authority_digest = object.get("extractorAuthorityDigest")?.as_str()?.to_owned();
    let semantic_configuration_digest = object
        .get("semanticConfigurationDigest")?
        .as_str()?
        .to_owned();
    if ![
        &semantic_input_manifest_digest,
        &facts_plugin_digest,
        &extractor_authority_digest,
        &semantic_configuration_digest,
    ]
    .into_iter()
    .all(|digest| is_prefixed_lowercase_sha256(digest))
    {
        return None;
    }

    let covered_files = compiled_files.checked_add(reused_files)?;
    let no_compiled_graph = compiled_files == 0 && reused_files == 0 && graph_digest.is_none();
    let profile_is_consistent = match (status, valid) {
        (CompilerIndexStatus::UnchangedHit, true) => {
            compiled_files == 0 && reused_files == total_files && !recovered && !fallback_used
        }
        (CompilerIndexStatus::ColdFull, true) => {
            compiled_files == total_files && reused_files == 0 && !recovered && !fallback_used
        }
        (CompilerIndexStatus::Incremental, true) => {
            covered_files == total_files && !recovered && !fallback_used
        }
        (CompilerIndexStatus::RecoveredFull, true) => {
            compiled_files == total_files && reused_files == 0 && recovered && !fallback_used
        }
        (CompilerIndexStatus::ColdFull | CompilerIndexStatus::Incremental, false) => {
            no_compiled_graph && !recovered && !fallback_used
        }
        (CompilerIndexStatus::RecoveredFull, false) => {
            no_compiled_graph && recovered && !fallback_used
        }
        (CompilerIndexStatus::Busy | CompilerIndexStatus::FailedRecoverable, false) => {
            no_compiled_graph && fallback_used
        }
        (CompilerIndexStatus::UnchangedHit, false)
        | (CompilerIndexStatus::Busy | CompilerIndexStatus::FailedRecoverable, true) => false,
    };
    if !profile_is_consistent {
        return None;
    }

    Some(CompilerIndexProfile {
        backend,
        status,
        valid,
        total_micros,
        compiler_micros,
        fir_extraction_micros,
        total_files,
        compiled_files,
        reused_files,
        recovered,
        fallback_used,
        graph_digest,
        semantic_input_manifest_digest,
        facts_plugin_digest,
        extractor_authority_digest,
        semantic_configuration_digest,
    })
}

fn parse_project_model_cache_profile(profiling: &Value) -> Option<ProjectModelCacheProfile> {
    let object = profiling.as_object()?;
    let status = match object.get("projectModelCacheStatus")?.as_str()? {
        "MEMORY_HIT" => ProjectModelCacheStatus::MemoryHit,
        "PERSISTENT_HIT" => ProjectModelCacheStatus::PersistentHit,
        "EXTRACTED_PUBLISHED" => ProjectModelCacheStatus::ExtractedPublished,
        "EXTRACTED_NOT_PUBLISHED" => ProjectModelCacheStatus::ExtractedNotPublished,
        _ => return None,
    };
    let publish_outcome = match object.get("projectModelPublishOutcome")?.as_str()? {
        "NOT_ATTEMPTED" => ProjectModelPublishOutcome::NotAttempted,
        "PUBLISHED" => ProjectModelPublishOutcome::Published,
        "INVALID_MODEL" => ProjectModelPublishOutcome::InvalidModel,
        "ROOT_UNAVAILABLE" => ProjectModelPublishOutcome::RootUnavailable,
        "WRITE_FAILED" => ProjectModelPublishOutcome::WriteFailed,
        _ => return None,
    };
    let publish_invalid_reason = match object.get("projectModelPublishInvalidReason")?.as_str()? {
        "NOT_APPLICABLE" => ProjectModelInvalidReason::NotApplicable,
        "MISSING_SEMANTIC_INPUT_MANIFEST_HASH" => {
            ProjectModelInvalidReason::MissingSemanticInputManifestHash
        }
        "INVALID_SEMANTIC_INPUT_MANIFEST_HASH" => {
            ProjectModelInvalidReason::InvalidSemanticInputManifestHash
        }
        "SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH" => {
            ProjectModelInvalidReason::SemanticInputManifestHashMismatch
        }
        "MISSING_SEMANTIC_INPUT_MANIFEST" => {
            ProjectModelInvalidReason::MissingSemanticInputManifest
        }
        "MODEL_INPUTS_MANIFEST_MISMATCH" => ProjectModelInvalidReason::ModelInputsManifestMismatch,
        "JDK_FINGERPRINT_MANIFEST_MISMATCH" => {
            ProjectModelInvalidReason::JdkFingerprintManifestMismatch
        }
        "MODEL_INPUTS_INVALID" => ProjectModelInvalidReason::ModelInputsInvalid,
        "RESOURCE_IDENTITIES_INVALID" => ProjectModelInvalidReason::ResourceIdentitiesInvalid,
        "JDK_HOME_INVALID" => ProjectModelInvalidReason::JdkHomeInvalid,
        "JDK_HOME_MISMATCH" => ProjectModelInvalidReason::JdkHomeMismatch,
        "JDK_FINGERPRINT_MISSING" => ProjectModelInvalidReason::JdkFingerprintMissing,
        "JDK_FINGERPRINT_INVALID" => ProjectModelInvalidReason::JdkFingerprintInvalid,
        _ => return None,
    };
    let total_micros = object.get("projectModelTotalMicros")?.as_u64()?;
    let key_micros = object.get("projectModelKeyMicros")?.as_u64()?;
    let load_micros = object.get("projectModelLoadMicros")?.as_u64()?;
    let extraction_micros = object.get("projectModelExtractionMicros")?.as_u64()?;
    let publish_micros = object.get("projectModelPublishMicros")?.as_u64()?;
    let persistent_configured = object.get("projectModelPersistentConfigured")?.as_bool()?;
    let published = object.get("projectModelPublished")?.as_bool()?;
    let measured = key_micros
        .checked_add(load_micros)?
        .checked_add(extraction_micros)?
        .checked_add(publish_micros)?;
    if measured > total_micros {
        return None;
    }
    let consistent = match status {
        ProjectModelCacheStatus::MemoryHit => {
            publish_outcome == ProjectModelPublishOutcome::NotAttempted
                && publish_invalid_reason == ProjectModelInvalidReason::NotApplicable
                && load_micros == 0
                && extraction_micros == 0
                && publish_micros == 0
                && !published
        }
        ProjectModelCacheStatus::PersistentHit => {
            publish_outcome == ProjectModelPublishOutcome::NotAttempted
                && publish_invalid_reason == ProjectModelInvalidReason::NotApplicable
                && persistent_configured
                && extraction_micros == 0
                && publish_micros == 0
                && !published
        }
        ProjectModelCacheStatus::ExtractedPublished => {
            publish_outcome == ProjectModelPublishOutcome::Published
                && publish_invalid_reason == ProjectModelInvalidReason::NotApplicable
                && persistent_configured
                && published
        }
        ProjectModelCacheStatus::ExtractedNotPublished => {
            matches!(
                publish_outcome,
                ProjectModelPublishOutcome::InvalidModel
                    | ProjectModelPublishOutcome::RootUnavailable
                    | ProjectModelPublishOutcome::WriteFailed
            ) && (publish_outcome == ProjectModelPublishOutcome::InvalidModel
                || publish_invalid_reason == ProjectModelInvalidReason::NotApplicable)
                && (publish_outcome != ProjectModelPublishOutcome::InvalidModel
                    || publish_invalid_reason != ProjectModelInvalidReason::NotApplicable)
                && !published
        }
    };
    consistent.then_some(ProjectModelCacheProfile {
        status,
        publish_outcome,
        publish_invalid_reason,
        total_micros,
        key_micros,
        load_micros,
        extraction_micros,
        publish_micros,
        persistent_configured,
        published,
    })
}

fn retain_verified_index_project_model_profile(
    profile: &mut RequestProfile,
    open_project: Option<ProjectModelCacheProfile>,
) {
    if open_project.is_some() {
        profile.project_model_cache = open_project;
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_prefixed_lowercase_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(is_lowercase_sha256)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn parse_worker_code(code: &str) -> ErrorCode {
    match code {
        "UNSUPPORTED_KOTLIN_VERSION" => ErrorCode::UnsupportedKotlinVersion,
        "UNSUPPORTED_COMPILER_PLUGIN_ABI" => ErrorCode::UnsupportedCompilerPluginAbi,
        "UNSUPPORTED_PROJECT_CONFIGURATION" => ErrorCode::UnsupportedProjectConfiguration,
        "PROJECT_MODEL_CHANGED" => ErrorCode::ProjectModelChanged,
        "WORKER_PROTOCOL_MISMATCH" => ErrorCode::WorkerProtocolMismatch,
        "WORKER_PREPARATION_REQUIRED" => ErrorCode::WorkerPreparationRequired,
        "WORKER_CRASHED" => ErrorCode::WorkerCrashed,
        "SYMBOL_NOT_FOUND" => ErrorCode::SymbolNotFound,
        "AMBIGUOUS_SYMBOL" => ErrorCode::AmbiguousSymbol,
        "EXPRESSION_NOT_FOUND" => ErrorCode::ExpressionNotFound,
        "STALE_TARGET" => ErrorCode::StaleTarget,
        "AMBIGUOUS_TARGET" => ErrorCode::AmbiguousTarget,
        "PRECONDITION_FAILED" => ErrorCode::PreconditionFailed,
        "REPLACEMENT_PARSE_ERROR" => ErrorCode::ReplacementParseError,
        "UNSUPPORTED_CONTROL_FLOW" => ErrorCode::UnsupportedControlFlow,
        "INCOMPLETE_SEMANTIC_ANALYSIS" => ErrorCode::IncompleteSemanticAnalysis,
        "SLICE_BUDGET_EXCEEDED" => ErrorCode::SliceBudgetExceeded,
        "TYPE_MISMATCH" => ErrorCode::TypeMismatch,
        "BINDING_CHANGED" => ErrorCode::BindingChanged,
        "NEW_DIAGNOSTICS" => ErrorCode::NewDiagnostics,
        "EFFECT_CHANGED" => ErrorCode::EffectChanged,
        "WRITESET_EXCEEDED" => ErrorCode::WritesetExceeded,
        "COMPILE_FAILED" => ErrorCode::CompileFailed,
        "TEST_FAILED" => ErrorCode::TestFailed,
        "ABI_CHANGED" => ErrorCode::AbiChanged,
        "RW_CONFLICT" => ErrorCode::RwConflict,
        "WW_CONFLICT" => ErrorCode::WwConflict,
        "STALE_REQUIRES_RESLICE" => ErrorCode::StaleRequiresReslice,
        "REF_COMPARE_AND_SWAP_FAILED" => ErrorCode::RefCompareAndSwapFailed,
        "TRANSACTION_RECOVERY_REQUIRED" => ErrorCode::TransactionRecoveryRequired,
        "INVALID_INPUT" => ErrorCode::InvalidInput,
        "INTERNAL" => ErrorCode::Internal,
        _ => ErrorCode::WorkerProtocolMismatch,
    }
}

pub fn workspace_root() -> PathBuf {
    if let Some(root) = std::env::var_os("CODECLEW_RUNTIME_ROOT") {
        let root = PathBuf::from(root);
        return root
            .canonicalize()
            .expect("CODECLEW_RUNTIME_ROOT must be a real runtime capsule");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[cfg(test)]
pub(crate) fn seed_test_build_caches(repo: &Path) {
    fn merge_tree(source: &Path, destination: &Path) {
        if !source.is_dir() {
            return;
        }
        for entry in walkdir::WalkDir::new(source)
            .follow_links(false)
            .into_iter()
            .map(Result::unwrap)
        {
            let relative = entry.path().strip_prefix(source).unwrap();
            if relative.components().any(|component| {
                matches!(
                    component.as_os_str().to_str(),
                    Some("daemon" | ".tmp" | "notifications")
                )
            }) {
                continue;
            }
            let target = destination.join(relative);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
            } else if entry.file_type().is_file() && !target.exists() {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                if std::fs::hard_link(entry.path(), &target).is_err() {
                    std::fs::copy(entry.path(), &target).unwrap();
                }
            }
        }
    }

    let workspace = workspace_root();
    if repo.join("gradlew").is_file() {
        merge_tree(
            &workspace.join("fixtures/kotlin-basic/.gradle"),
            &repo.join(".gradle"),
        );
    }
    if repo.join("mvnw").is_file() {
        merge_tree(
            &workspace.join("fixtures/kotlin-maven/.semantic-thread/maven-repository"),
            &repo.join(".semantic-thread/maven-repository"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::RepositoryIndex;
    use serde_json::json;
    use std::ffi::OsStr;
    use walkdir::WalkDir;

    #[test]
    fn source_syntax_index_does_not_bootstrap_a_project_model() {
        assert!(!request_requires_project_snapshot(
            RequestKind::ValidateCandidate,
            &json!({}),
        ));
        assert!(!request_requires_project_snapshot(
            RequestKind::IndexFiles,
            &json!({"syntaxOnly":true}),
        ));
        assert!(request_requires_project_snapshot(
            RequestKind::IndexFiles,
            &json!({"syntaxOnly":false}),
        ));
        assert!(request_requires_project_snapshot(
            RequestKind::IndexFiles,
            &json!({}),
        ));
        assert!(request_requires_project_snapshot(
            RequestKind::ApplyEdit,
            &json!({"syntaxOnly":true}),
        ));
    }

    #[test]
    fn optional_build_state_environment_treats_empty_as_native() {
        assert_eq!(build_state_root_from_environment_value(None), None);
        assert_eq!(
            build_state_root_from_environment_value(Some(OsString::new())),
            None
        );
        assert_eq!(
            build_state_root_from_environment_value(Some(OsString::from("/sealed/state"))),
            Some(PathBuf::from("/sealed/state"))
        );
    }

    fn valid_compiler_index_profiling() -> Value {
        json!({
            "backend":"BTA_PERSISTENT",
            "status":"INCREMENTAL",
            "valid":true,
            "totalMicros":120,
            "compilerMicros":80,
            "firExtractionMicros":30,
            "totalFiles":5,
            "compiledFiles":2,
            "reusedFiles":3,
            "recovered":false,
            "fallbackUsed":false,
            "graphDigest":"a".repeat(64),
            "semanticInputManifestDigest":format!("sha256:{}", "b".repeat(64)),
            "factsPluginDigest":format!("sha256:{}", "c".repeat(64)),
            "extractorAuthorityDigest":format!("sha256:{}", "d".repeat(64)),
            "semanticConfigurationDigest":format!("sha256:{}", "e".repeat(64)),
            "workerProcessingMicros":140,
            "cacheRequests":1,
            "privatePath":"/must/not/escape",
        })
    }

    fn valid_project_model_cache_profiling() -> Value {
        json!({
            "projectModelCacheStatus":"PERSISTENT_HIT",
            "projectModelPublishOutcome":"NOT_ATTEMPTED",
            "projectModelPublishInvalidReason":"NOT_APPLICABLE",
            "projectModelTotalMicros":120,
            "projectModelKeyMicros":20,
            "projectModelLoadMicros":90,
            "projectModelExtractionMicros":0,
            "projectModelPublishMicros":0,
            "projectModelPersistentConfigured":true,
            "projectModelPublished":false,
            "privatePath":"/must/not/escape",
        })
    }

    #[test]
    fn project_model_cache_profile_is_typed_and_operational_only() {
        let profile = parse_project_model_cache_profile(&valid_project_model_cache_profiling())
            .expect("valid persistent hit");
        assert_eq!(profile.status, ProjectModelCacheStatus::PersistentHit);
        assert_eq!(
            profile.publish_outcome,
            ProjectModelPublishOutcome::NotAttempted
        );
        assert_eq!(
            profile.publish_invalid_reason,
            ProjectModelInvalidReason::NotApplicable
        );
        assert_eq!(profile.total_micros, 120);
        assert_eq!(profile.key_micros, 20);
        assert_eq!(profile.load_micros, 90);
        let transported = serde_json::to_value(profile).unwrap();
        assert_eq!(transported["status"], "PERSISTENT_HIT");
        assert_eq!(transported["publishInvalidReason"], "NOT_APPLICABLE");
        assert!(transported.get("privatePath").is_none());
    }

    #[test]
    fn project_model_cache_profile_rejects_unknown_partial_and_inconsistent_rows() {
        let mut unknown = valid_project_model_cache_profiling();
        unknown["projectModelCacheStatus"] = Value::String("NEW_STATUS".to_owned());
        assert!(parse_project_model_cache_profile(&unknown).is_none());

        let mut partial = valid_project_model_cache_profiling();
        partial
            .as_object_mut()
            .unwrap()
            .remove("projectModelLoadMicros");
        assert!(parse_project_model_cache_profile(&partial).is_none());

        let mut impossible = valid_project_model_cache_profiling();
        impossible["projectModelExtractionMicros"] = Value::from(1);
        assert!(parse_project_model_cache_profile(&impossible).is_none());

        let mut over_total = valid_project_model_cache_profiling();
        over_total["projectModelKeyMicros"] = Value::from(121);
        assert!(parse_project_model_cache_profile(&over_total).is_none());

        let mut published_hit = valid_project_model_cache_profiling();
        published_hit["projectModelPublished"] = Value::Bool(true);
        assert!(parse_project_model_cache_profile(&published_hit).is_none());

        let mut inconsistent_outcome = valid_project_model_cache_profiling();
        inconsistent_outcome["projectModelPublishOutcome"] = Value::String("WRITE_FAILED".into());
        assert!(parse_project_model_cache_profile(&inconsistent_outcome).is_none());

        let mut unknown_reason = valid_project_model_cache_profiling();
        unknown_reason["projectModelPublishInvalidReason"] = Value::String("NEW_REASON".into());
        assert!(parse_project_model_cache_profile(&unknown_reason).is_none());
    }

    #[test]
    fn project_model_cache_profile_accepts_each_consistent_status() {
        for (status, outcome, persistent, published, load, extraction, publish) in [
            ("MEMORY_HIT", "NOT_ATTEMPTED", false, false, 0, 0, 0),
            ("PERSISTENT_HIT", "NOT_ATTEMPTED", true, false, 10, 0, 0),
            ("EXTRACTED_PUBLISHED", "PUBLISHED", true, true, 10, 30, 20),
            (
                "EXTRACTED_NOT_PUBLISHED",
                "INVALID_MODEL",
                true,
                false,
                10,
                30,
                20,
            ),
            (
                "EXTRACTED_NOT_PUBLISHED",
                "ROOT_UNAVAILABLE",
                false,
                false,
                10,
                30,
                20,
            ),
            (
                "EXTRACTED_NOT_PUBLISHED",
                "WRITE_FAILED",
                true,
                false,
                10,
                30,
                20,
            ),
        ] {
            let mut value = valid_project_model_cache_profiling();
            value["projectModelCacheStatus"] = Value::String(status.to_owned());
            value["projectModelPublishOutcome"] = Value::String(outcome.to_owned());
            value["projectModelPublishInvalidReason"] = Value::String(
                if outcome == "INVALID_MODEL" {
                    "SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH"
                } else {
                    "NOT_APPLICABLE"
                }
                .to_owned(),
            );
            value["projectModelPersistentConfigured"] = Value::Bool(persistent);
            value["projectModelPublished"] = Value::Bool(published);
            value["projectModelLoadMicros"] = Value::from(load);
            value["projectModelExtractionMicros"] = Value::from(extraction);
            value["projectModelPublishMicros"] = Value::from(publish);
            assert!(
                parse_project_model_cache_profile(&value).is_some(),
                "valid {status} profile was rejected"
            );
        }
    }

    #[test]
    fn verified_index_retains_open_project_cache_observation_over_memory_hit() {
        let persistent = parse_project_model_cache_profile(&valid_project_model_cache_profiling())
            .expect("persistent profile");
        let mut memory_value = valid_project_model_cache_profiling();
        memory_value["projectModelCacheStatus"] = Value::String("MEMORY_HIT".to_owned());
        memory_value["projectModelLoadMicros"] = Value::from(0);
        memory_value["projectModelPersistentConfigured"] = Value::Bool(false);
        let memory = parse_project_model_cache_profile(&memory_value).expect("memory profile");
        let mut request = RequestProfile {
            project_model_cache: Some(memory.clone()),
            ..RequestProfile::default()
        };

        retain_verified_index_project_model_profile(&mut request, Some(persistent.clone()));
        assert_eq!(request.project_model_cache, Some(persistent));

        request.project_model_cache = Some(memory.clone());
        retain_verified_index_project_model_profile(&mut request, None);
        assert_eq!(request.project_model_cache, Some(memory));
    }

    #[test]
    fn compiler_index_profile_is_typed_and_profiling_is_removed_from_body() {
        let mut body = json!({
            "indexHash":"semantic-index",
            "facts":[],
            "profiling":valid_compiler_index_profiling(),
        });

        let profiling = take_worker_profiling(&mut body).unwrap();
        let profile = parse_compiler_index_profile(&profiling).unwrap();

        assert_eq!(body, json!({"indexHash":"semantic-index","facts":[]}));
        assert_eq!(profile.backend, CompilerIndexBackend::BtaPersistent);
        assert_eq!(profile.status, CompilerIndexStatus::Incremental);
        assert!(profile.valid);
        assert_eq!(profile.compiled_files, 2);
        assert_eq!(profile.reused_files, 3);
        assert_eq!(
            profile.graph_digest.as_deref(),
            Some("a".repeat(64).as_str())
        );
        let transported = serde_json::to_value(profile).unwrap();
        assert_eq!(transported["backend"], "BTA_PERSISTENT");
        assert_eq!(transported["status"], "INCREMENTAL");
        assert_eq!(transported["valid"], true);
        assert_eq!(
            transported["semanticConfigurationDigest"],
            format!("sha256:{}", "e".repeat(64))
        );
        assert!(transported.get("privatePath").is_none());
    }

    #[test]
    fn unknown_or_partial_compiler_index_profile_is_ignored() {
        let mut unknown = valid_compiler_index_profiling();
        unknown["backend"] = Value::String("OTHER_BACKEND".to_owned());
        let mut body = json!({"semantic":"preserved","profiling":unknown});
        let profiling = take_worker_profiling(&mut body).unwrap();
        assert!(parse_compiler_index_profile(&profiling).is_none());
        assert_eq!(body, json!({"semantic":"preserved"}));

        let mut partial = valid_compiler_index_profiling();
        partial.as_object_mut().unwrap().remove("valid");
        assert!(parse_compiler_index_profile(&partial).is_none());

        let mut unknown_status = valid_compiler_index_profiling();
        unknown_status["status"] = Value::String("NEW_STATUS".to_owned());
        assert!(parse_compiler_index_profile(&unknown_status).is_none());
    }

    #[test]
    fn malformed_compiler_index_profile_is_ignored() {
        let mut negative = valid_compiler_index_profiling();
        negative["totalFiles"] = Value::from(-1);
        assert!(parse_compiler_index_profile(&negative).is_none());

        let mut inconsistent = valid_compiler_index_profiling();
        inconsistent["reusedFiles"] = Value::from(4);
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        let mut uppercase_digest = valid_compiler_index_profiling();
        uppercase_digest["graphDigest"] = Value::String("A".repeat(64));
        assert!(parse_compiler_index_profile(&uppercase_digest).is_none());

        let mut null_digest = valid_compiler_index_profiling();
        null_digest["graphDigest"] = Value::Null;
        let normalized = parse_compiler_index_profile(&null_digest).unwrap();
        assert!(normalized.graph_digest.is_none());
        assert!(
            serde_json::to_value(normalized)
                .unwrap()
                .get("graphDigest")
                .is_none()
        );

        let mut wrong_digest_type = valid_compiler_index_profiling();
        wrong_digest_type["graphDigest"] = Value::from(7);
        assert!(parse_compiler_index_profile(&wrong_digest_type).is_none());

        for key in [
            "semanticInputManifestDigest",
            "factsPluginDigest",
            "extractorAuthorityDigest",
            "semanticConfigurationDigest",
        ] {
            let mut missing = valid_compiler_index_profiling();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                parse_compiler_index_profile(&missing).is_none(),
                "missing {key} was accepted"
            );

            let mut malformed = valid_compiler_index_profiling();
            malformed[key] = Value::String(format!("sha256:{}", "A".repeat(64)));
            assert!(
                parse_compiler_index_profile(&malformed).is_none(),
                "malformed {key} was accepted"
            );
        }
    }

    #[test]
    fn valid_compiler_index_statuses_require_complete_counts_and_no_fallback() {
        for (status, compiled, reused, recovered) in [
            ("UNCHANGED_HIT", 0, 5, false),
            ("COLD_FULL", 5, 0, false),
            ("INCREMENTAL", 2, 3, false),
            ("RECOVERED_FULL", 5, 0, true),
        ] {
            let mut value = valid_compiler_index_profiling();
            value["status"] = Value::String(status.to_owned());
            value["compiledFiles"] = Value::from(compiled);
            value["reusedFiles"] = Value::from(reused);
            value["recovered"] = Value::Bool(recovered);
            assert!(
                parse_compiler_index_profile(&value).is_some(),
                "valid {status} profile was rejected"
            );
        }

        let mut invalid_hit = valid_compiler_index_profiling();
        invalid_hit["status"] = Value::String("UNCHANGED_HIT".to_owned());
        invalid_hit["valid"] = Value::Bool(false);
        invalid_hit["reusedFiles"] = Value::from(0);
        invalid_hit["graphDigest"] = Value::Null;
        assert!(parse_compiler_index_profile(&invalid_hit).is_none());

        let mut fallback_success = valid_compiler_index_profiling();
        fallback_success["fallbackUsed"] = Value::Bool(true);
        assert!(parse_compiler_index_profile(&fallback_success).is_none());
    }

    #[test]
    fn attempted_compiler_failures_retain_status_without_claiming_facts() {
        for (status, recovered) in [
            ("COLD_FULL", false),
            ("INCREMENTAL", false),
            ("RECOVERED_FULL", true),
        ] {
            let mut value = valid_compiler_index_profiling();
            value["status"] = Value::String(status.to_owned());
            value["valid"] = Value::Bool(false);
            value["compiledFiles"] = Value::from(0);
            value["reusedFiles"] = Value::from(0);
            value["recovered"] = Value::Bool(recovered);
            value["graphDigest"] = Value::Null;
            let profile = parse_compiler_index_profile(&value)
                .unwrap_or_else(|| panic!("honest failed {status} attempt was rejected"));
            assert!(!profile.valid);
            assert!(profile.graph_digest.is_none());
        }

        let mut invalid_with_graph = valid_compiler_index_profiling();
        invalid_with_graph["valid"] = Value::Bool(false);
        invalid_with_graph["compiledFiles"] = Value::from(0);
        invalid_with_graph["reusedFiles"] = Value::from(0);
        assert!(parse_compiler_index_profile(&invalid_with_graph).is_none());

        invalid_with_graph["graphDigest"] = Value::Null;
        invalid_with_graph["compiledFiles"] = Value::from(1);
        assert!(parse_compiler_index_profile(&invalid_with_graph).is_none());

        invalid_with_graph["compiledFiles"] = Value::from(0);
        invalid_with_graph["fallbackUsed"] = Value::Bool(true);
        assert!(parse_compiler_index_profile(&invalid_with_graph).is_none());
    }

    #[test]
    fn infrastructure_failures_are_invalid_fallbacks_with_no_fact_counts() {
        for (status, total, recovered) in [
            ("BUSY", 0, false),
            ("BUSY", 0, true),
            ("FAILED_RECOVERABLE", 0, false),
            ("FAILED_RECOVERABLE", 5, false),
            ("FAILED_RECOVERABLE", 5, true),
        ] {
            let mut value = valid_compiler_index_profiling();
            value["status"] = Value::String(status.to_owned());
            value["valid"] = Value::Bool(false);
            value["totalFiles"] = Value::from(total);
            value["compiledFiles"] = Value::from(0);
            value["reusedFiles"] = Value::from(0);
            value["recovered"] = Value::Bool(recovered);
            value["fallbackUsed"] = Value::Bool(true);
            value["graphDigest"] = Value::Null;
            assert!(
                parse_compiler_index_profile(&value).is_some(),
                "honest {status} profile with recovered={recovered} was rejected"
            );
        }

        let mut inconsistent = valid_compiler_index_profiling();
        inconsistent["status"] = Value::String("FAILED_RECOVERABLE".to_owned());
        inconsistent["valid"] = Value::Bool(false);
        inconsistent["totalFiles"] = Value::from(0);
        inconsistent["compiledFiles"] = Value::from(0);
        inconsistent["reusedFiles"] = Value::from(0);
        inconsistent["graphDigest"] = Value::Null;
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        inconsistent["fallbackUsed"] = Value::Bool(true);
        inconsistent["valid"] = Value::Bool(true);
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        inconsistent["valid"] = Value::Bool(false);
        inconsistent["reusedFiles"] = Value::from(1);
        assert!(parse_compiler_index_profile(&inconsistent).is_none());

        inconsistent["reusedFiles"] = Value::from(0);
        inconsistent["totalFiles"] = Value::from(1);
        assert!(parse_compiler_index_profile(&inconsistent).is_some());

        inconsistent["graphDigest"] = Value::String("a".repeat(64));
        assert!(parse_compiler_index_profile(&inconsistent).is_none());
    }

    #[test]
    fn compiler_index_root_is_private_canonical_and_external() {
        let workspace = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let state_path = state.path().canonicalize().unwrap();
        assert_eq!(
            validate_compiler_index_root(workspace.path(), &state_path).unwrap(),
            state_path
        );

        let nested = workspace.path().join("compiler-index");
        std::fs::create_dir(&nested).unwrap();
        assert_eq!(
            validate_compiler_index_root(workspace.path(), &nested)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let public = tempfile::tempdir().unwrap();
            std::fs::set_permissions(public.path(), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            assert_eq!(
                validate_compiler_index_root(workspace.path(), public.path())
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
            let link = state_path.parent().unwrap().join("compiler-index-link");
            let _ = std::fs::remove_file(&link);
            symlink(&state_path, &link).unwrap();
            assert_eq!(
                validate_compiler_index_root(workspace.path(), &link)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
            std::fs::remove_file(link).unwrap();
        }
    }

    fn configured_command_environment(command: &Command, key: &str) -> Option<Option<PathBuf>> {
        command
            .get_envs()
            .find_map(|(name, value)| (name == OsStr::new(key)).then(|| value.map(PathBuf::from)))
    }

    #[test]
    fn worker_state_environment_scrubs_ambient_authority_when_disabled() {
        let mut command = Command::new("not-executed");
        command
            .env("CODECLEW_K1_BUILD_STATE_ROOT", "ambient-k1")
            .env("CODECLEW_K2_INDEX_ROOT", "ambient-k2");

        configure_worker_state_environment(&mut command, None, None);

        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K1_BUILD_STATE_ROOT"),
            Some(None)
        );
        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K2_INDEX_ROOT"),
            Some(None)
        );
    }

    #[test]
    fn worker_state_environment_passes_only_explicit_canonical_roots() {
        let build_state = tempfile::tempdir().unwrap();
        let compiler_index = tempfile::tempdir().unwrap();
        let canonical_build_state = build_state.path().canonicalize().unwrap();
        let canonical_compiler_index = compiler_index.path().canonicalize().unwrap();
        let mut command = Command::new("not-executed");
        command
            .env("CODECLEW_K1_BUILD_STATE_ROOT", "ambient-k1")
            .env("CODECLEW_K2_INDEX_ROOT", "ambient-k2");

        configure_worker_state_environment(
            &mut command,
            Some(&canonical_build_state),
            Some(&canonical_compiler_index),
        );

        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K1_BUILD_STATE_ROOT"),
            Some(Some(canonical_build_state))
        );
        assert_eq!(
            configured_command_environment(&command, "CODECLEW_K2_INDEX_ROOT"),
            Some(Some(canonical_compiler_index))
        );
    }

    fn source_syntax_fixture() -> (tempfile::TempDir, String, Value) {
        let repo = tempfile::tempdir().unwrap();
        let relative = "src/main/kotlin/p/Value.kt".to_owned();
        let path = repo.path().join(&relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = "package p\nfun value() = 1\n";
        std::fs::write(&path, source).unwrap();
        let start = source.find("fun").unwrap() as u64;
        let end = source.trim_end().encode_utf16().count() as u64;
        let declaration = json!({
            "declarationId":"declaration:syntax-value",
            "symbolId":"p.value()",
            "name":"value",
            "kind":"KtNamedFunction",
            "file":relative,
            "rangeStart":start,
            "rangeEnd":end,
            "sourceOrigin":{"file":relative,"rangeStart":start,"rangeEnd":end},
        });
        let files = json!([{
            "path":relative,
            "normalizedRelativePath":relative,
            "contentHash":crate::canonical::hash_bytes(source.as_bytes()),
            "declarations":[declaration],
            "semanticFacts":[],
        }]);
        let response = json!({
            "schema":"semantic-index/0.1",
            "compilation":":/main",
            "partial":true,
            "analysisMode":"SYNTAX_DECLARATIONS",
            "files":files,
            "indexHash":"sha256:transport-bound",
            "projectModelHash":"SOURCE_SYNTAX",
            "k2Validated":false,
            "diagnostics":[],
        });
        (repo, relative, response)
    }

    #[test]
    fn source_syntax_accepts_current_declaration_only_response() {
        let (repo, relative, response) = source_syntax_fixture();
        let manifest = validate_source_syntax_response(
            repo.path(),
            ":/main",
            std::slice::from_ref(&relative),
            &response,
        )
        .unwrap();
        assert!(manifest.starts_with("sha256:"));

        let mut partial = response.clone();
        partial["partial"] = Value::Bool(false);
        assert!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &partial,
            )
            .is_err()
        );
        assert!(validate_source_syntax_response(repo.path(), ":/main", &[], &response).is_err());

        let mut empty_graphs = response.clone();
        empty_graphs["declarationRelations"] = json!([]);
        empty_graphs["declarationRelationHash"] =
            Value::String(crate::canonical::hash(&json!([])).unwrap());
        empty_graphs["declarationDescriptors"] = json!([]);
        empty_graphs["declarationDescriptorHash"] =
            Value::String(crate::canonical::hash(&json!([])).unwrap());
        assert!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &empty_graphs,
            )
            .is_ok()
        );
    }

    #[test]
    fn source_syntax_allows_provisional_id_collisions_but_not_duplicate_occurrences() {
        let (repo, relative, response) = source_syntax_fixture();
        let mut colliding_ids = response.clone();
        let mut second = colliding_ids["files"][0]["declarations"][0].clone();
        second["name"] = json!("packageOccurrence");
        second["rangeStart"] = json!(0);
        second["rangeEnd"] = json!(7);
        second["sourceOrigin"] = json!({"file":relative,"rangeStart":0,"rangeEnd":7});
        colliding_ids["files"][0]["declarations"]
            .as_array_mut()
            .unwrap()
            .insert(0, second);
        assert!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &colliding_ids,
            )
            .is_ok()
        );

        let mut duplicate_occurrence = response;
        let mut duplicate = duplicate_occurrence["files"][0]["declarations"][0].clone();
        duplicate["declarationId"] = json!("provisional:distinct-declaration");
        duplicate["symbolId"] = json!("provisional:distinct-symbol");
        duplicate_occurrence["files"][0]["declarations"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &duplicate_occurrence,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn source_syntax_rejects_semantic_row_forgery() {
        let (repo, relative, response) = source_syntax_fixture();
        let mut forged_fact = response.clone();
        forged_fact["files"][0]["semanticFacts"] = json!([{"kind":"CALL"}]);
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_fact,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut forged_override = response.clone();
        forged_override["files"][0]["overrides"] = json!([{"symbolId":"p.value()"}]);
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_override,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut forged_relation = response.clone();
        forged_relation["declarationRelations"] = json!({
            "relations":[{"kind":"CALLS"}],
            "boundaries":[],
        });
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_relation,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut forged_descriptor = response;
        forged_descriptor["declarationDescriptors"] = json!({
            "descriptors":[{"symbolId":"p.value()"}],
            "boundaries":[],
        });
        assert_eq!(
            validate_source_syntax_response(
                repo.path(),
                ":/main",
                std::slice::from_ref(&relative),
                &forged_descriptor,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn source_syntax_worker_indexes_without_a_build_model() {
        let workspace = workspace_root();
        let repo = tempfile::tempdir().unwrap();
        let relative = "src/main/kotlin/p/Value.kt";
        let source = repo.path().join(relative);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "package p\n// 😀\nfun value() = 1\n").unwrap();
        assert!(!repo.path().join("build.gradle.kts").exists());
        assert!(!repo.path().join("pom.xml").exists());

        let mut worker = WorkerClient::start(&workspace).unwrap();
        let verified = worker
            .index_files_source_syntax_verified(&json!({
                "repo":repo.path(),
                "compilation":":/main",
                "syntaxOnly":true,
                "files":[relative],
            }))
            .unwrap();
        let facts = worker.inspect_verified_source_syntax(&verified).unwrap();
        assert_eq!(facts["analysisMode"], "SYNTAX_DECLARATIONS");
        assert_eq!(facts["k2Validated"], false);
        assert_eq!(facts["partial"], true);
        assert_eq!(facts["files"].as_array().unwrap().len(), 1);
        assert!(
            facts["files"][0]["declarations"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        );
        assert_eq!(facts["declarationRelations"], json!([]));
        assert_eq!(facts["declarationDescriptors"], json!([]));
        std::fs::write(&source, "package p\n// changed\nfun value() = 2\n").unwrap();
        assert_eq!(
            worker
                .inspect_verified_source_syntax(&verified)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
        worker.shutdown().unwrap();
    }

    #[test]
    fn source_syntax_rejects_k2_authority_forgery() {
        let (repo, relative, mut response) = source_syntax_fixture();
        response["k2Validated"] = Value::Bool(true);
        assert_eq!(
            validate_source_syntax_response(repo.path(), ":/main", &[relative], &response)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    fn project_model_fixture(
        classpath: &[&str],
        task_digest: &str,
        configuration_digest: &str,
        ordered_equivalent: bool,
    ) -> Value {
        let manifest = json!({
            "orderedCompileClasspath":classpath,
            "classpathAuthority":{
                "orderedDigest":"sha256:raw-ordered",
                "taskLibrariesDigest":task_digest,
                "configurationDigest":configuration_digest,
                "orderedEquivalent":ordered_equivalent,
            },
            "declaredCompilerVersion":"2.4.10",
        });
        json!({
            "schema":"semantic-project/0.1",
            "compilation":":/main",
            "projectModelHash":"sha256:raw-project",
            "semanticInputManifestHash":crate::canonical::hash(&manifest).unwrap(),
            "semanticInputManifest":manifest.clone(),
            "classpathAuthority":manifest["classpathAuthority"].clone(),
        })
    }

    #[test]
    fn stable_project_model_identity_ignores_equivalent_cache_mounts() {
        let first = project_model_fixture(
            &["repo:.gradle/a.jar:sha256:a", "repo:.gradle/b.jar:sha256:b"],
            "sha256:/first/task",
            "sha256:/first/configuration",
            true,
        );
        let second = project_model_fixture(
            &["repo:.gradle/a.jar:sha256:a", "repo:.gradle/b.jar:sha256:b"],
            "sha256:/second/task",
            "sha256:/second/configuration",
            true,
        );
        assert_eq!(
            stable_project_model_identity(&first).unwrap(),
            stable_project_model_identity(&second).unwrap()
        );
    }

    #[test]
    fn stable_project_model_identity_preserves_semantic_disagreement() {
        let baseline = project_model_fixture(
            &["repo:.gradle/a.jar:sha256:a", "repo:.gradle/b.jar:sha256:b"],
            "sha256:task",
            "sha256:configuration",
            true,
        );
        let reordered = project_model_fixture(
            &["repo:.gradle/b.jar:sha256:b", "repo:.gradle/a.jar:sha256:a"],
            "sha256:task",
            "sha256:configuration",
            true,
        );
        let provider_disagreement = project_model_fixture(
            &["repo:.gradle/a.jar:sha256:a", "repo:.gradle/b.jar:sha256:b"],
            "sha256:other-task",
            "sha256:other-configuration",
            false,
        );
        assert_ne!(
            stable_project_model_identity(&baseline).unwrap(),
            stable_project_model_identity(&reordered).unwrap()
        );
        assert_ne!(
            stable_project_model_identity(&baseline).unwrap(),
            stable_project_model_identity(&provider_disagreement).unwrap()
        );
    }

    #[test]
    fn compiler_plugin_abi_failure_is_not_collapsed_into_generic_k2_failure() {
        assert_eq!(
            parse_worker_code("UNSUPPORTED_COMPILER_PLUGIN_ABI"),
            ErrorCode::UnsupportedCompilerPluginAbi,
        );
        assert_eq!(
            parse_worker_code("INCOMPLETE_SEMANTIC_ANALYSIS"),
            ErrorCode::IncompleteSemanticAnalysis,
        );
        assert_eq!(parse_worker_code("INVALID_INPUT"), ErrorCode::InvalidInput);
        assert_eq!(
            parse_worker_code("FUTURE_OR_MISSPELLED_CODE"),
            ErrorCode::WorkerProtocolMismatch,
        );
    }

    #[test]
    fn compiler_plugin_abi_discovery_visits_each_supported_variant_once() {
        let mut tried = 0;
        let first = WorkerVariant::next_untried_for_abi_discovery(tried).unwrap();
        assert_eq!(first, WorkerVariant::Kotlin24);
        tried |= first.discovery_bit();

        let second = WorkerVariant::next_untried_for_abi_discovery(tried).unwrap();
        assert_eq!(second, WorkerVariant::Kotlin23);
        tried |= second.discovery_bit();

        let third = WorkerVariant::next_untried_for_abi_discovery(tried).unwrap();
        assert_eq!(third, WorkerVariant::Kotlin21);
        tried |= third.discovery_bit();
        assert!(WorkerVariant::next_untried_for_abi_discovery(tried).is_none());
    }
    fn relation_normalization_facts(relations: Vec<Value>) -> Value {
        let mut relations = relations;
        relations.sort_by_key(|row| crate::canonical::bytes(row).unwrap());
        let graph = json!({
            "schema":"declaration-relation-graph/0.1",
            "compilation":":/main",
            "coverage":"COMPLETE_SUPPORTED_SUBSET",
            "relations":relations,
            "boundaries":[],
            "provenance":{},
        });
        json!({
            "declarationRelationHash":crate::canonical::hash(&graph).unwrap(),
            "declarationRelations":graph,
        })
    }

    #[test]
    fn optional_relation_evidence_becomes_typed_unknown_without_losing_base_call() {
        let call = json!({
            "schema":"declaration-relation/0.1",
            "kind":"CALLS",
            "owner":"p/caller",
            "target":"p/callee",
            "argumentToParameter":[{"argumentStart":9,"parameterIndex":0,"parameterType":"kotlin/String"}],
        });
        let raw_hash = crate::canonical::hash(&call).unwrap();
        let mut facts = relation_normalization_facts(vec![call]);

        normalize_optional_relation_evidence(&mut facts).unwrap();

        let graph = &facts["declarationRelations"];
        assert_eq!(graph["coverage"], "PARTIAL");
        assert_eq!(graph["relations"].as_array().unwrap().len(), 1);
        assert!(graph["relations"][0].get("argumentToParameter").is_none());
        assert_eq!(
            graph["boundaries"][0]["code"],
            "ARGUMENT_MAPPING_UNAVAILABLE"
        );
        assert_eq!(graph["boundaries"][0]["affectedRowCount"], 1);
        assert_eq!(
            graph["boundaries"][0]["rawRowsHash"],
            crate::canonical::hash(&json!([raw_hash])).unwrap()
        );
        assert_eq!(
            facts["declarationRelationHash"],
            crate::canonical::hash(graph).unwrap()
        );
    }

    #[test]
    fn unreliable_flow_rows_do_not_discard_independent_compiler_relations() {
        let mut facts = relation_normalization_facts(vec![
            json!({"schema":"declaration-relation/0.1","kind":"OVERRIDES","owner":"p/impl","target":"p/api"}),
            json!({"schema":"declaration-relation/0.1","kind":"NULL_COALESCES","owner":"p/f","target":"p/fallback"}),
            json!({"schema":"declaration-relation/0.1","kind":"RETURNS_VALUE_FROM","owner":"p/f","target":"p/source"}),
        ]);

        normalize_optional_relation_evidence(&mut facts).unwrap();

        let relations = facts["declarationRelations"]["relations"]
            .as_array()
            .unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0]["kind"], "OVERRIDES");
        let codes = facts["declarationRelations"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            codes,
            std::collections::BTreeSet::from([
                "NULL_COALESCING_FLOW_UNAVAILABLE",
                "RETURN_VALUE_FLOW_UNAVAILABLE",
            ])
        );
    }

    #[test]
    fn optional_relation_normalization_rejects_a_forged_input_hash() {
        let mut facts = relation_normalization_facts(vec![json!({
            "schema":"declaration-relation/0.1",
            "kind":"CALLS",
            "owner":"p/caller",
            "target":"p/callee",
        })]);
        facts["declarationRelationHash"] = json!("sha256:forged");
        assert_eq!(
            normalize_optional_relation_evidence(&mut facts)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
    }

    #[test]
    fn verified_index_binds_distribution_after_project_variant_selection() {
        let workspace = workspace_root();
        let repo = workspace
            .join("fixtures/kotlin-2-1")
            .canonicalize()
            .unwrap();
        seed_test_build_caches(&repo);
        let mut worker = WorkerClient::start(&workspace).unwrap();
        assert_eq!(worker.capabilities.compiler_version, "2.4.10");

        let (project, verified) = worker
            .open_project_and_index_verified(&json!({
                "repo":repo,
                "compilation":":/main",
                "syntaxOnly":false,
            }))
            .unwrap();
        assert_eq!(worker.capabilities.compiler_version, "2.1.21");
        let selected = worker.trusted_distribution.as_ref().unwrap();
        assert_eq!(
            verified.distribution_fingerprint,
            selected.plugin_fingerprint
        );
        assert_eq!(verified.distribution_tree_hash, selected.tree_hash);
        assert_eq!(
            project["projectModelHash"].as_str(),
            Some(verified.project_model_hash())
        );
        worker.inspect_verified_index(&verified).unwrap();
        worker.shutdown().unwrap();
    }

    #[test]
    fn embedded_worker_distribution_rejects_workspace_and_private_tree_mutation() {
        use std::os::unix::fs::PermissionsExt;

        struct RestoreFile {
            path: PathBuf,
            bytes: Vec<u8>,
            permissions: std::fs::Permissions,
        }
        impl Drop for RestoreFile {
            fn drop(&mut self) {
                let _ = std::fs::write(&self.path, &self.bytes);
                let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
            }
        }
        struct RemoveFile(PathBuf);
        impl Drop for RemoveFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        struct RestoreMove {
            original: PathBuf,
            held: PathBuf,
        }
        impl Drop for RestoreMove {
            fn drop(&mut self) {
                if self.held.exists() {
                    let _ = std::fs::rename(&self.held, &self.original);
                }
            }
        }

        let workspace = workspace_root();
        let mut worker = WorkerClient::start(&workspace).unwrap();
        assert_eq!(worker.capabilities.compiler_version, "2.4.10");
        let repo = workspace
            .join("fixtures/kotlin-basic")
            .canonicalize()
            .unwrap();
        let verified = worker
            .index_files_verified(&json!({
                "repo":repo,
                "compilation":":/main",
                "syntaxOnly":false
            }))
            .unwrap();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap()["k2Validated"],
            true
        );

        let trusted = worker.trusted_distribution.as_ref().unwrap();
        let private_launcher = trusted.launcher.clone();
        let launcher_bytes = std::fs::read(&private_launcher).unwrap();
        let launcher_permissions = std::fs::metadata(&private_launcher).unwrap().permissions();
        std::fs::write(&private_launcher, "#!/bin/sh\nexit 88\n").unwrap();
        assert_eq!(
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::write(&private_launcher, launcher_bytes).unwrap();
        std::fs::set_permissions(&private_launcher, launcher_permissions).unwrap();

        let plugin = trusted
            .distribution_root
            .join("lib")
            .join(WorkerVariant::Kotlin24.plugin_jar_name());
        let plugin_bytes = std::fs::read(&plugin).unwrap();
        std::fs::write(&plugin, b"changed").unwrap();
        assert_eq!(
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::write(&plugin, plugin_bytes).unwrap();

        let extra = trusted.distribution_root.join("unexpected-authority-input");
        std::fs::write(&extra, b"extra").unwrap();
        assert_eq!(
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        std::fs::remove_file(extra).unwrap();
        assert!(worker.inspect_verified_index(&verified).is_ok());
        worker.shutdown().unwrap();

        let workspace_launcher = worker_launcher(&workspace, WorkerVariant::Kotlin24);
        {
            let original = RestoreFile {
                bytes: std::fs::read(&workspace_launcher).unwrap(),
                permissions: std::fs::metadata(&workspace_launcher)
                    .unwrap()
                    .permissions(),
                path: workspace_launcher.clone(),
            };
            std::fs::write(&workspace_launcher, "#!/bin/sh\nexit 77\n").unwrap();
            std::fs::set_permissions(&workspace_launcher, std::fs::Permissions::from_mode(0o755))
                .unwrap();
            assert_eq!(
                WorkerClient::start(&workspace).err().unwrap().code,
                ErrorCode::WorkerPreparationRequired
            );
            drop(original);
        }

        let workspace_plugin = workspace
            .join(WorkerVariant::Kotlin24.distribution_relative())
            .join("lib")
            .join(WorkerVariant::Kotlin24.plugin_jar_name());
        {
            let original = RestoreFile {
                bytes: std::fs::read(&workspace_plugin).unwrap(),
                permissions: std::fs::metadata(&workspace_plugin).unwrap().permissions(),
                path: workspace_plugin.clone(),
            };
            std::fs::write(&workspace_plugin, b"changed").unwrap();
            assert_eq!(
                WorkerClient::start(&workspace).err().unwrap().code,
                ErrorCode::WorkerPreparationRequired
            );
            drop(original);
        }

        let distribution = workspace.join(WorkerVariant::Kotlin24.distribution_relative());
        let extra = distribution.join("unexpected-authority-input");
        std::fs::write(&extra, b"extra").unwrap();
        let extra_guard = RemoveFile(extra.clone());
        assert_eq!(
            WorkerClient::start(&workspace).err().unwrap().code,
            ErrorCode::WorkerPreparationRequired
        );
        std::fs::remove_file(&extra).unwrap();
        drop(extra_guard);

        let missing = distribution.join("lib/kotlin-0.1.0.jar");
        let held = distribution.join("lib/.kotlin-0.1.0.jar.held");
        std::fs::rename(&missing, &held).unwrap();
        let missing_guard = RestoreMove {
            original: missing.clone(),
            held: held.clone(),
        };
        assert_eq!(
            WorkerClient::start(&workspace).err().unwrap().code,
            ErrorCode::WorkerPreparationRequired
        );
        std::fs::rename(&held, &missing).unwrap();
        drop(missing_guard);

        let caller_init = workspace.join("init.gradle");
        std::fs::write(
            &caller_init,
            "throw new GradleException('caller injection')\n",
        )
        .unwrap();
        let init_guard = RemoveFile(caller_init.clone());
        assert_eq!(
            WorkerClient::start(&workspace).err().unwrap().code,
            ErrorCode::WorkerPreparationRequired
        );
        std::fs::remove_file(caller_init).unwrap();
        drop(init_guard);

        let warm_started = Instant::now();
        let restored = WorkerClient::start(&workspace).unwrap();
        eprintln!(
            "embedded_worker_distribution_warm_start_ms={}",
            warm_started.elapsed().as_millis()
        );
        restored.shutdown().unwrap();
    }

    #[test]
    fn trusted_distribution_identity_is_read_only_cache_key_material() {
        let workspace = workspace_root();
        let worker = WorkerClient::start(&workspace).unwrap();
        let identity = worker.trusted_distribution_identity().unwrap();
        assert!(identity.tree_hash.starts_with("sha256:"));
        assert!(identity.build_input_digest.starts_with("sha256:"));
        assert!(identity.plugin_fingerprint.starts_with("sha256:"));
        worker.shutdown().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn missing_trusted_distribution_runs_the_exact_wrapper_task_once() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().canonicalize().unwrap();
        let wrapper = workspace.join("gradlew");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > bootstrap-args\nmkdir -p workers/kotlin/build/install/kotlin\n",
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let distribution = workspace.join(WorkerVariant::Kotlin24.distribution_relative());

        assert!(
            bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                WorkerVariant::Kotlin24,
                &distribution,
            )
            .unwrap()
        );
        assert!(distribution.is_dir());
        assert_eq!(
            std::fs::read_to_string(workspace.join("bootstrap-args")).unwrap(),
            ":workers:kotlin:installDist\n--no-daemon\n--quiet\n"
        );

        assert!(
            !bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                WorkerVariant::Kotlin24,
                &distribution,
            )
            .unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn missing_trusted_distribution_uses_the_caller_build_environment() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().canonicalize().unwrap();
        let wrapper = workspace.join("gradlew");
        std::fs::write(
            &wrapper,
            r#"#!/bin/sh
{
  printf 'GRADLE_USER_HOME=%s\n' "$GRADLE_USER_HOME"
  printf 'GRADLE_OPTS=%s\n' "$GRADLE_OPTS"
  printf 'JAVA_OPTS=%s\n' "$JAVA_OPTS"
  printf 'JAVA_TOOL_OPTIONS=%s\n' "$JAVA_TOOL_OPTIONS"
  printf 'JDK_JAVA_OPTIONS=%s\n' "$JDK_JAVA_OPTIONS"
  printf '_JAVA_OPTIONS=%s\n' "$_JAVA_OPTIONS"
  printf 'ORG_GRADLE_PROJECT_codeclewBootstrapMarker=%s\n' "$ORG_GRADLE_PROJECT_codeclewBootstrapMarker"
} > bootstrap-environment
/bin/mkdir -p workers/kotlin/build/install/kotlin
"#,
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let distribution = workspace.join(WorkerVariant::Kotlin24.distribution_relative());
        let gradle_home = workspace.join("caller-gradle-home");

        // Inject an isolated snapshot instead of mutating the process environment:
        // Rust tests execute concurrently, so global set_var/remove_var would race.
        let build_environment = vec![
            (
                OsString::from("GRADLE_USER_HOME"),
                gradle_home.clone().into_os_string(),
            ),
            (
                OsString::from("GRADLE_OPTS"),
                OsString::from("-Dcodeclew.gradle.marker=caller"),
            ),
            (
                OsString::from("JAVA_OPTS"),
                OsString::from("-Dcodeclew.java.marker=java-opts"),
            ),
            (
                OsString::from("JAVA_TOOL_OPTIONS"),
                OsString::from("-Dcodeclew.java.marker=tool-options"),
            ),
            (
                OsString::from("JDK_JAVA_OPTIONS"),
                OsString::from("-Dcodeclew.java.marker=jdk-options"),
            ),
            (
                OsString::from("_JAVA_OPTIONS"),
                OsString::from("-Dcodeclew.java.marker=legacy-options"),
            ),
            (
                OsString::from("ORG_GRADLE_PROJECT_codeclewBootstrapMarker"),
                OsString::from("caller-project-property"),
            ),
        ];

        assert!(
            bootstrap_trusted_worker_distribution_if_missing_with_environment(
                &workspace,
                WorkerVariant::Kotlin24,
                &distribution,
                build_environment,
            )
            .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("bootstrap-environment")).unwrap(),
            format!(
                "GRADLE_USER_HOME={}\n\
GRADLE_OPTS=-Dcodeclew.gradle.marker=caller\n\
JAVA_OPTS=-Dcodeclew.java.marker=java-opts\n\
JAVA_TOOL_OPTIONS=-Dcodeclew.java.marker=tool-options\n\
JDK_JAVA_OPTIONS=-Dcodeclew.java.marker=jdk-options\n\
_JAVA_OPTIONS=-Dcodeclew.java.marker=legacy-options\n\
ORG_GRADLE_PROJECT_codeclewBootstrapMarker=caller-project-property\n",
                gradle_home.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_or_symlinked_distribution_state_is_never_rebuilt() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().canonicalize().unwrap();
        let wrapper = workspace.join("gradlew");
        std::fs::write(&wrapper, "#!/bin/sh\ntouch wrapper-invoked\nexit 99\n").unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let distribution = workspace.join(WorkerVariant::Kotlin24.distribution_relative());
        std::fs::create_dir_all(&distribution).unwrap();
        std::fs::write(distribution.join("drifted"), b"not trusted").unwrap();

        assert!(
            !bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                WorkerVariant::Kotlin24,
                &distribution,
            )
            .unwrap()
        );
        assert!(!workspace.join("wrapper-invoked").exists());

        std::fs::remove_dir_all(workspace.join("workers/kotlin/build")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.join("workers/kotlin")).unwrap();
        symlink(outside.path(), workspace.join("workers/kotlin/build")).unwrap();
        assert_eq!(
            bootstrap_trusted_worker_distribution_if_missing(
                &workspace,
                WorkerVariant::Kotlin24,
                &distribution,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerPreparationRequired
        );
        assert!(!workspace.join("wrapper-invoked").exists());
    }

    #[test]
    fn compiler_receipt_requires_explicit_successful_k2_validation() {
        assert_eq!(
            require_k2_validated(&json!({"k2Validated":false}))
                .unwrap_err()
                .code,
            ErrorCode::IncompleteSemanticAnalysis
        );
        assert_eq!(
            require_k2_validated(&json!({})).unwrap_err().code,
            ErrorCode::IncompleteSemanticAnalysis
        );
        require_k2_validated(&json!({"k2Validated":true})).unwrap();
    }

    #[test]
    fn external_build_state_uses_inline_large_source_transport() {
        let source = "x".repeat(64 * 1024 + 1);
        let state = tempfile::tempdir().unwrap();
        let (inline, blob) =
            source_transport(&json!({"source":source}), Some(state.path())).unwrap();
        assert_eq!(inline.len(), 64 * 1024 + 1);
        assert!(blob.is_none());
    }

    fn transport_blob(root: &Path, bytes: &[u8]) -> BlobRef {
        let hash = crate::canonical::hash_bytes(bytes);
        let digest = hash.trim_start_matches("sha256:").to_owned();
        std::fs::create_dir_all(root.join("sha256")).unwrap();
        std::fs::write(root.join("sha256").join(&digest), bytes).unwrap();
        BlobRef {
            content_hash: hash,
            relative_path: format!("sha256/{digest}"),
            size_bytes: bytes.len() as u64,
        }
    }

    #[test]
    fn index_response_body_supports_bounded_inline_or_one_verified_blob() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let inline = br#"{"schema":"semantic-index/0.1"}"#.to_vec();
        assert_eq!(
            index_response_body(inline.clone(), &[], &canonical_root).unwrap(),
            inline
        );

        let body = br#"{"schema":"semantic-index/0.1","large":true}"#;
        let blob = transport_blob(&canonical_root, body);
        let path = canonical_root.join(&blob.relative_path);
        assert_eq!(
            index_response_body(Vec::new(), std::slice::from_ref(&blob), &canonical_root).unwrap(),
            body
        );
        assert!(!path.exists());
    }

    #[test]
    fn index_response_body_rejects_ambiguous_or_tampered_blob_authority() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let body = b"bounded canonical body";
        let blob = transport_blob(&canonical_root, body);
        assert_eq!(
            index_response_body(
                b"inline".to_vec(),
                std::slice::from_ref(&blob),
                &canonical_root,
            )
            .unwrap_err()
            .code,
            ErrorCode::WorkerProtocolMismatch
        );
        assert_eq!(
            index_response_body(Vec::new(), &[], &canonical_root)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        assert_eq!(
            index_response_body(Vec::new(), &[blob.clone(), blob.clone()], &canonical_root,)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );

        let mut wrong_size = blob.clone();
        wrong_size.size_bytes += 1;
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &wrong_size)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        let mut escaping = blob.clone();
        escaping.relative_path = "../body".into();
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &escaping)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );

        std::fs::write(
            canonical_root.join(&blob.relative_path),
            b"tampered canonical bod",
        )
        .unwrap();
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &blob)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[cfg(unix)]
    #[test]
    fn index_response_body_rejects_symlinked_cas_objects() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let body = b"bounded canonical body";
        let blob = transport_blob(&canonical_root, body);
        let target = canonical_root.join(&blob.relative_path);
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::remove_file(&target).unwrap();
        symlink(outside.path(), target).unwrap();
        assert_eq!(
            read_worker_transport_blob(&canonical_root, &blob)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn open_project_protocol_preserves_full_compilation_identity_kotlin_24() {
        let workspace = workspace_root();
        let source = workspace.join("fixtures/kotlin-basic");
        let temporary = tempfile::Builder::new()
            .prefix("open-project-protocol-")
            .tempdir_in(workspace.join("fixtures"))
            .unwrap();
        for entry in WalkDir::new(&source).into_iter().map(Result::unwrap) {
            let relative = entry.path().strip_prefix(&source).unwrap();
            if relative.components().any(|part| {
                matches!(
                    part.as_os_str().to_str(),
                    Some(".gradle" | ".semantic-thread" | "build")
                )
            }) {
                continue;
            }
            let destination = temporary.path().join(relative);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&destination).unwrap();
            } else {
                std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
        std::fs::write(
            temporary.path().join("settings.gradle.kts"),
            "pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }\n\
             dependencyResolutionManagement { repositories { mavenCentral() } }\n\
             rootProject.name = \"protocol-fixture\"\ninclude(\":service\")\n",
        )
        .unwrap();
        std::fs::create_dir_all(temporary.path().join("service/src/main/kotlin/p")).unwrap();
        std::fs::write(
            temporary.path().join("service/build.gradle.kts"),
            "plugins { kotlin(\"jvm\") version \"2.4.10\" }\nkotlin { jvmToolchain(21) }\n",
        )
        .unwrap();
        std::fs::write(
            temporary
                .path()
                .join("service/src/main/kotlin/p/Service.kt"),
            "package p\nfun serviceValue() = 1\n",
        )
        .unwrap();
        seed_test_build_caches(temporary.path());

        let mut worker = WorkerClient::start(&workspace).unwrap();
        let root = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":temporary.path(),"compilation":":/main"}),
            )
            .unwrap();
        assert_eq!(root["compilation"], ":/main");
        assert_eq!(root["module"], ":");
        assert_eq!(root["sourceSet"], "main");
        let module = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":temporary.path(),"compilation":":service/main"}),
            )
            .unwrap();
        assert_eq!(module["compilation"], ":service/main");
        assert_eq!(module["module"], ":service");
        assert_eq!(module["sourceSet"], "main");

        let mut malformed = module.clone();
        malformed["compilation"] = Value::String(":/main".into());
        assert_eq!(
            bind_open_project_compilation(&mut malformed, ":service/main")
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
        worker.shutdown().unwrap();
    }

    #[test]
    fn verified_index_facts_are_live_session_bound_and_tamper_evident() {
        let workspace = workspace_root();
        let repo = workspace
            .join("fixtures/kotlin-basic")
            .canonicalize()
            .unwrap();
        let mut worker = WorkerClient::start(&workspace).unwrap();
        let mut verified = worker
            .index_files_verified(&json!({
                "repo":repo,
                "compilation":":/main",
                "syntaxOnly":false
            }))
            .unwrap();
        let mut index = RepositoryIndex::open_compilation(&repo, Some(":/main")).unwrap();
        let snapshot = index.update_verified(&verified, &worker).unwrap();
        assert!(!snapshot.is_empty());
        assert!(index.declaration_relations().unwrap().is_some());
        assert!(index.declaration_descriptors().unwrap().is_some());

        let original_session = verified.authority_session;
        verified.authority_session = Uuid::new_v4();
        assert_eq!(
            worker.inspect_verified_index(&verified).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );
        verified.authority_session = original_session;

        verified.payload["declarationDescriptors"]["descriptors"][0]["exportBoundary"] =
            Value::String("PRIVATE_API".into());
        verified.payload["declarationDescriptorHash"] = Value::String(
            crate::canonical::hash(&verified.payload["declarationDescriptors"]).unwrap(),
        );
        verified.payload_hash = crate::canonical::hash(&verified.payload).unwrap();
        assert_eq!(
            worker
                .authorize_index_facts(&verified, &repo, ":/main")
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
        worker.shutdown().unwrap();
    }
}
