use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use syn::spanned::Spanned;
use syn::visit::Visit;

pub const RUST_LANGUAGE: &str = "language:rust";
pub const RUST_SYNTAX_FACTS_CAPABILITY: &str = "analysis:rust-syntax-facts";
pub const RUST_INDEX_SCHEMA: &str = "codeclew-rust-syntax-index/1.2";
const FACT_SCHEMA: &str = "codeclew-rust-syntax-fact/1.2";
const RECEIPT_SCHEMA: &str = "codeclew-rust-syntax-completeness/1.0";
const ADAPTER_AUTHORITY_SCHEMA: &str = "codeclew-rust-syntax-adapter/1.2";
const PARSER_AUTHORITY: &str =
    "syn-2.0.119/full+visit+proc-macro2-1.0.107/span-locations+direct-call-paths-v1+match-cases-v1";
const MAX_SOURCE_FILES: usize = 512;
const MAX_SOURCE_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_DECLARATIONS: usize = 65_536;
const MAX_DIRECT_REFERENCES_PER_DECLARATION: usize = 64;
const MAX_DIRECT_REFERENCES: usize = 65_536;
const MAX_DIRECT_REFERENCE_PATH_SEGMENTS: usize = 32;
const MAX_DIRECT_REFERENCE_PATH_SEGMENT_BYTES: usize = 256;
const MAX_DIRECT_REFERENCE_BYTES: usize = 4 * 1024;
const MAX_DIRECT_REFERENCES_PAYLOAD_BYTES: usize = 32 * 1024;
const MAX_MATCH_ARM_CASES_PER_DECLARATION: usize = 32;
const MAX_MATCH_ARM_CASE_BYTES: usize = 4 * 1024;
const MAX_MATCH_ARM_CASES_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_NESTING: usize = 64;
const MAX_FACT_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_FACT_BATCH_INPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_BOUNDARIES: usize = 4096;

pub fn rust_adapter_digest() -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":ADAPTER_AUTHORITY_SCHEMA,
        "indexSchema":RUST_INDEX_SCHEMA,
        "factSchema":FACT_SCHEMA,
        "capability":RUST_SYNTAX_FACTS_CAPABILITY,
        "parserAuthority":PARSER_AUTHORITY,
    }))
    .map_err(internal)
}

pub fn rust_scope_digest(index: &Value) -> Result<String, ClewError> {
    validate_index(index)?;
    canonical::hash(&json!({
        "schema":"codeclew-rust-syntax-scope/1.1",
        "compilation":index["compilation"],
        "modelDigest":index["modelDigest"],
        "files":index["files"],
        "declarationDescriptors":index["declarationDescriptors"],
        "boundaries":index["boundaries"],
    }))
    .map_err(internal)
}

pub struct RustAdapterV2 {
    adapter_digest: String,
    toolchain_digest: String,
    store: CasStore,
    index: Value,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl RustAdapterV2 {
    pub fn new(
        adapter_digest: String,
        toolchain_digest: String,
        store: CasStore,
        index: Value,
    ) -> Result<Self, ClewError> {
        require_digest(&adapter_digest)?;
        require_digest(&toolchain_digest)?;
        validate_index(&index)?;
        Ok(Self {
            adapter_digest,
            toolchain_digest,
            store,
            index,
            cancelled_attempts: Mutex::new(BTreeSet::new()),
            stopped: AtomicBool::new(false),
        })
    }
}

impl LanguageAdapter for RustAdapterV2 {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
        Ok(AdapterHandshake {
            protocol: ADAPTER_PROTOCOL.into(),
            adapter_id: "rust-syntax-1".into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(RUST_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(RUST_SYNTAX_FACTS_CAPABILITY)?],
            toolchains: vec![ToolchainConstraint {
                authority_digest: self.toolchain_digest.clone(),
                minimum_version: None,
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
        if request.compilation.language_uri.as_str() != RUST_LANGUAGE
            || request.capability.as_str() != RUST_SYNTAX_FACTS_CAPABILITY
            || request.compilation.toolchain.digest != self.toolchain_digest
            || self.index.get("compilation").and_then(Value::as_str)
                != Some(request.compilation.compilation_id.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "Rust syntax request differs from its exact language/toolchain authority",
            ));
        }
        let facts = translate_facts(&self.store, &self.index)?;
        let fact_count = facts.len() as u64;
        for (sequence, chunk) in facts.chunks(1024).enumerate() {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: u32::try_from(sequence)
                    .map_err(|_| resource("Rust fact shard sequence overflow"))?,
                facts: chunk.to_vec(),
            }))?;
        }
        let scope_digest = rust_scope_digest(&self.index)?;
        let receipt = self.store.put(
            RECEIPT_SCHEMA,
            &canonical::bytes(&json!({
                "schema":RECEIPT_SCHEMA,
                "scopeDigest":scope_digest,
                "coverage":"PARTIAL",
                "certainty":"UNSURE",
                "obligations":["VERIFY_RUST_NAME_RESOLUTION","VERIFY_CFG_AND_MACRO_EXPANSION"],
            }))
            .map_err(internal)?,
        )?;
        sink.accept(AnalysisEvent::AttemptComplete(AnalysisAttemptComplete {
            scope_digest,
            completeness_receipt: receipt,
            fact_count,
        }))
    }

    fn cancel(&self, attempt_id: &str) -> Result<(), ClewError> {
        if attempt_id.is_empty() || attempt_id.len() > 128 {
            return Err(invalid("Rust attempt identity is invalid"));
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

pub fn translate_facts(store: &CasStore, index: &Value) -> Result<Vec<FactRecord>, ClewError> {
    validate_index(index)?;
    let capability = CapabilityUri::parse(RUST_SYNTAX_FACTS_CAPABILITY)?;
    let mut batch = PreparedFactBatch::default();
    batch.push(
        json!({
            "schema":FACT_SCHEMA,
            "kind":"cargo-target",
            "package":index["package"],
            "targetKind":index["targetKind"],
            "targetName":index["targetName"],
            "sourcePath":index["sourcePath"],
            "cargoVersion":index["cargoVersion"],
            "rustcVersion":index["rustcVersion"],
            "resolution":"CARGO_MODEL_EXACT",
        }),
        MAX_FACT_BATCH_INPUT_BYTES,
    )?;
    for file in index["files"].as_array().expect("validated files") {
        batch.push(
            json!({
                "schema":FACT_SCHEMA,
                "kind":"source-file",
                "path":file["path"],
                "contentHash":file["contentHash"],
                "package":index["package"],
                "targetName":index["targetName"],
                "resolution":"SOURCE_MEMBERSHIP_EXACT",
            }),
            MAX_FACT_BATCH_INPUT_BYTES,
        )?;
    }
    for descriptor in index["declarationDescriptors"]["descriptors"]
        .as_array()
        .expect("validated descriptors")
    {
        let mut row = descriptor.clone();
        row.as_object_mut()
            .expect("validated descriptor object")
            .insert("schema".into(), Value::String(FACT_SCHEMA.into()));
        row.as_object_mut()
            .expect("validated descriptor object")
            .insert("kind".into(), Value::String("declaration".into()));
        batch.push(row, MAX_FACT_BATCH_INPUT_BYTES)?;
    }
    for boundary in index["boundaries"]
        .as_array()
        .expect("validated boundaries")
    {
        batch.push(
            json!({
                "schema":FACT_SCHEMA,
                "kind":"analysis-boundary",
                "code":boundary["code"],
                "file":boundary.get("file").cloned().unwrap_or(Value::Null),
                "subject":boundary.get("subject").cloned().unwrap_or(Value::Null),
                "resolution":"UNKNOWN",
            }),
            MAX_FACT_BATCH_INPUT_BYTES,
        )?;
    }
    let (fact_keys, inputs): (Vec<_>, Vec<_>) = batch
        .prepared
        .into_iter()
        .map(|(fact_key, bytes)| (fact_key, (FACT_SCHEMA.into(), bytes)))
        .unzip();
    let objects = store.put_batch(inputs)?;
    let mut facts = fact_keys
        .into_iter()
        .zip(objects)
        .map(|(fact_key, payload)| FactRecord {
            fact_key,
            domain_uri: capability.clone(),
            payload,
        })
        .collect::<Vec<_>>();
    facts.sort_by(|left, right| left.fact_key.cmp(&right.fact_key));
    Ok(facts)
}

#[derive(Default)]
struct PreparedFactBatch {
    prepared: Vec<(String, Vec<u8>)>,
    input_bytes: usize,
}

impl PreparedFactBatch {
    fn push(&mut self, payload: Value, input_limit: usize) -> Result<(), ClewError> {
        let bytes = canonical::bytes(&payload).map_err(internal)?;
        if bytes.len() > MAX_FACT_PAYLOAD_BYTES {
            return Err(resource("Rust syntax fact exceeds its payload budget"));
        }
        let next = self
            .input_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| resource("Rust syntax fact batch size overflow"))?;
        if next > input_limit {
            return Err(resource(
                "Rust syntax fact batch exceeds its input byte budget",
            ));
        }
        let digest = canonical::hash_bytes(&bytes);
        self.prepared.push((format!("rust-syntax:{digest}"), bytes));
        self.input_bytes = next;
        Ok(())
    }
}

fn validate_index(index: &Value) -> Result<(), ClewError> {
    let files = index
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no file manifest"))?;
    let descriptors = index
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no declaration descriptors"))?;
    let relations = index
        .pointer("/declarationRelations/relations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no relation boundary"))?;
    let boundaries = index
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no analysis boundaries"))?;
    let mut direct_reference_count = 0usize;
    let mut any_direct_references_truncated = false;
    let descriptors_valid = descriptors.iter().all(|descriptor| {
        validate_descriptor(
            descriptor,
            &mut direct_reference_count,
            &mut any_direct_references_truncated,
        )
    });
    let has_direct_reference_limit = boundaries.iter().any(|boundary| {
        matches!(
            boundary.get("code").and_then(Value::as_str),
            Some(
                "RUST_DIRECT_REFERENCE_DECLARATION_LIMIT"
                    | "RUST_DIRECT_REFERENCE_GLOBAL_LIMIT"
                    | "RUST_DIRECT_REFERENCE_PAYLOAD_LIMIT"
            )
        )
    });
    let has_boundary_limit = boundaries.iter().any(|boundary| {
        boundary.get("code").and_then(Value::as_str) == Some("RUST_BOUNDARY_LIMIT")
    });
    if index.get("schema").and_then(Value::as_str) != Some(RUST_INDEX_SCHEMA)
        || index.get("analysisCoverage").and_then(Value::as_str) != Some("PARTIAL")
        || index.get("analysisCertainty").and_then(Value::as_str) != Some("UNSURE")
        || index.get("parserAuthority").and_then(Value::as_str) != Some(PARSER_AUTHORITY)
        || index
            .pointer("/declarationDescriptors/coverage")
            .and_then(Value::as_str)
            != Some("PARTIAL")
        || index
            .pointer("/declarationRelations/coverage")
            .and_then(Value::as_str)
            != Some("PARTIAL")
        || files.is_empty()
        || files.len() > 4096
        || files.iter().any(|file| {
            file.get("path")
                .and_then(Value::as_str)
                .is_none_or(|path| !safe_path(path) || !path.ends_with(".rs"))
                || file
                    .get("contentHash")
                    .and_then(Value::as_str)
                    .is_none_or(|digest| require_digest(digest).is_err())
        })
        || files
            .windows(2)
            .any(|pair| pair[0]["path"].as_str() >= pair[1]["path"].as_str())
        || descriptors.len() > MAX_DECLARATIONS
        || !descriptors_valid
        || direct_reference_count > MAX_DIRECT_REFERENCES
        || (any_direct_references_truncated && !(has_direct_reference_limit || has_boundary_limit))
        || (!any_direct_references_truncated && has_direct_reference_limit)
        || descriptors
            .windows(2)
            .any(|pair| pair[0]["symbolIdentity"].as_str() >= pair[1]["symbolIdentity"].as_str())
        || !relations.is_empty()
        || boundaries.len() > MAX_BOUNDARIES
        || boundaries.iter().any(|boundary| {
            boundary.get("code").and_then(Value::as_str).is_none()
                || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
                || boundary
                    .get("file")
                    .and_then(Value::as_str)
                    .is_some_and(|path| !safe_path(path) || !path.ends_with(".rs"))
        })
        || [
            "compilation",
            "modelDigest",
            "package",
            "targetKind",
            "targetName",
            "sourcePath",
            "cargoVersion",
            "rustcVersion",
        ]
        .iter()
        .any(|field| {
            index
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        })
    {
        return Err(invalid("Rust syntax index authority is invalid"));
    }
    Ok(())
}

fn validate_descriptor(
    descriptor: &Value,
    direct_reference_count: &mut usize,
    any_direct_references_truncated: &mut bool,
) -> bool {
    let Some(object) = descriptor.as_object() else {
        return false;
    };
    let Some(symbol_identity) = object.get("symbolIdentity").and_then(Value::as_str) else {
        return false;
    };
    let Some(name) = object.get("name").and_then(Value::as_str) else {
        return false;
    };
    let Some(kind) = object.get("declarationKind").and_then(Value::as_str) else {
        return false;
    };
    let Some(file) = object.get("file").and_then(Value::as_str) else {
        return false;
    };
    let Some(range_start) = object.get("rangeStart").and_then(Value::as_u64) else {
        return false;
    };
    let Some(range_end) = object.get("rangeEnd").and_then(Value::as_u64) else {
        return false;
    };
    let Some(start_line) = object.get("startLine").and_then(Value::as_u64) else {
        return false;
    };
    let Some(end_line) = object.get("endLine").and_then(Value::as_u64) else {
        return false;
    };
    if symbol_identity.is_empty()
        || name.is_empty()
        || !matches!(
            kind,
            "function"
                | "struct"
                | "enum"
                | "trait"
                | "trait-method"
                | "type-alias"
                | "const"
                | "static"
                | "impl-method"
                | "module"
        )
        || !safe_path(file)
        || !file.ends_with(".rs")
        || object.get("resolution").and_then(Value::as_str) != Some("SYNTAX_EXACT")
        || !matches!(
            object.get("cfgStatus").and_then(Value::as_str),
            Some("UNKNOWN" | "UNCONDITIONAL")
        )
        || range_start > range_end
        || start_line == 0
        || start_line > end_line
    {
        return false;
    }
    let callable = matches!(kind, "function" | "impl-method" | "trait-method");
    let references = object.get("directReferences");
    let truncated = object.get("directReferencesTruncated");
    let match_arm_cases = object.get("matchArmCases");
    if !callable {
        return references.is_none() && truncated.is_none() && match_arm_cases.is_none();
    }
    let Some(references) = references.and_then(Value::as_array) else {
        return false;
    };
    let Some(truncated) = truncated.and_then(Value::as_bool) else {
        return false;
    };
    if references.len() > MAX_DIRECT_REFERENCES_PER_DECLARATION {
        return false;
    }
    *direct_reference_count = match direct_reference_count.checked_add(references.len()) {
        Some(count) => count,
        None => return false,
    };
    *any_direct_references_truncated |= truncated;
    let mut previous = None;
    let mut direct_reference_payload_bytes = 0usize;
    for reference in references {
        let Some(key) =
            validate_direct_reference(reference, range_start, range_end, start_line, end_line)
        else {
            return false;
        };
        if previous.as_ref().is_some_and(|previous| previous >= &key) {
            return false;
        }
        let Ok(bytes) = canonical::bytes(reference) else {
            return false;
        };
        let Some(next_payload_bytes) = direct_reference_payload_bytes.checked_add(bytes.len())
        else {
            return false;
        };
        if bytes.len() > MAX_DIRECT_REFERENCE_BYTES
            || next_payload_bytes > MAX_DIRECT_REFERENCES_PAYLOAD_BYTES
        {
            return false;
        }
        direct_reference_payload_bytes = next_payload_bytes;
        previous = Some(key);
    }
    validate_match_arm_cases(match_arm_cases, symbol_identity, range_start, range_end)
}

fn validate_match_arm_cases(
    cases: Option<&Value>,
    symbol_identity: &str,
    declaration_start: u64,
    declaration_end: u64,
) -> bool {
    let Some(object) = cases.and_then(Value::as_object) else {
        return false;
    };
    if object.len() != 3
        || object.get("authority").and_then(Value::as_str) != Some("EXACT_SNAPSHOT_TEXT_SYNTAX")
        || !matches!(
            object.get("coverage").and_then(Value::as_str),
            Some("VISIBLE_PARSED_MATCH_ARMS_COMPLETE" | "VISIBLE_PARSED_MATCH_ARMS_PARTIAL")
        )
    {
        return false;
    }
    let Some(entries) = object.get("cases").and_then(Value::as_array) else {
        return false;
    };
    if entries.len() > MAX_MATCH_ARM_CASES_PER_DECLARATION {
        return false;
    }
    let mut payload_bytes = 0usize;
    let mut previous = None;
    for entry in entries {
        let Some(case) = entry.as_object() else {
            return false;
        };
        if case.len() != 8
            || case.get("kind").and_then(Value::as_str) != Some("MATCH_ARM")
            || case.get("authority").and_then(Value::as_str) != Some("EXACT_SNAPSHOT_TEXT_SYNTAX")
        {
            return false;
        }
        let Some(case_id) = case.get("caseId").and_then(Value::as_str) else {
            return false;
        };
        let Some(group_start) = case.get("groupStart").and_then(Value::as_u64) else {
            return false;
        };
        let Some(pattern) = case.get("pattern").and_then(Value::as_object) else {
            return false;
        };
        let Some(body) = case.get("body").and_then(Value::as_object) else {
            return false;
        };
        let valid_optional_guard = match case.get("guard") {
            Some(Value::Null) => true,
            Some(Value::Object(guard)) => {
                guard.len() == 3
                    && guard.get("start").and_then(Value::as_u64).is_some()
                    && guard.get("end").and_then(Value::as_u64).is_some()
                    && guard.get("text").and_then(Value::as_str).is_some()
                    && guard["start"].as_u64().unwrap() < guard["end"].as_u64().unwrap()
                    && declaration_start <= guard["start"].as_u64().unwrap()
                    && guard["end"].as_u64().unwrap() <= declaration_end
            }
            _ => false,
        };
        let valid_slice = |slice: &serde_json::Map<String, Value>| {
            slice.len() == 3
                && slice.get("start").and_then(Value::as_u64).is_some()
                && slice.get("end").and_then(Value::as_u64).is_some()
                && slice.get("text").and_then(Value::as_str).is_some()
                && slice["start"].as_u64().unwrap() < slice["end"].as_u64().unwrap()
                && declaration_start <= slice["start"].as_u64().unwrap()
                && slice["end"].as_u64().unwrap() <= declaration_end
        };
        if case_id.is_empty()
            || !case_id.starts_with("rust-case:sha256:")
            || group_start < declaration_start
            || group_start >= declaration_end
            || !valid_slice(pattern)
            || !valid_slice(body)
            || !valid_optional_guard
            || case.get("declarationIdentity").and_then(Value::as_str) != Some(symbol_identity)
        {
            return false;
        }
        let key = (group_start, pattern["start"].as_u64().unwrap(), case_id);
        if previous.is_some_and(|previous| previous >= key) {
            return false;
        }
        let Ok(bytes) = canonical::bytes(entry) else {
            return false;
        };
        payload_bytes = match payload_bytes.checked_add(bytes.len()) {
            Some(bytes) if bytes <= MAX_MATCH_ARM_CASES_PAYLOAD_BYTES => bytes,
            _ => return false,
        };
        if bytes.len() > MAX_MATCH_ARM_CASE_BYTES {
            return false;
        }
        previous = Some(key);
    }
    true
}

fn validate_direct_reference(
    reference: &Value,
    declaration_start: u64,
    declaration_end: u64,
    declaration_start_line: u64,
    declaration_end_line: u64,
) -> Option<(u64, u64, Vec<String>)> {
    let object = reference.as_object()?;
    if object.len() != 8
        || object.get("kind").and_then(Value::as_str) != Some("CALL_PATH")
        || object.get("resolution").and_then(Value::as_str) != Some("SYNTAX_UNRESOLVED")
    {
        return None;
    }
    let segments = object.get("pathSegments")?.as_array()?;
    if segments.is_empty()
        || segments.len() > MAX_DIRECT_REFERENCE_PATH_SEGMENTS
        || segments.iter().any(|segment| {
            segment
                .as_str()
                .is_none_or(|segment| segment.len() > MAX_DIRECT_REFERENCE_PATH_SEGMENT_BYTES)
        })
    {
        return None;
    }
    let segments = segments
        .iter()
        .map(|segment| {
            segment
                .as_str()
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
        })
        .collect::<Option<Vec<_>>>()?;
    if object.get("terminalName")?.as_str()? != segments.last()? {
        return None;
    }
    let range_start = object.get("rangeStart")?.as_u64()?;
    let range_end = object.get("rangeEnd")?.as_u64()?;
    let start_line = object.get("startLine")?.as_u64()?;
    let end_line = object.get("endLine")?.as_u64()?;
    if range_start < declaration_start
        || range_start >= range_end
        || range_end > declaration_end
        || start_line < declaration_start_line
        || start_line == 0
        || start_line > end_line
        || end_line > declaration_end_line
    {
        return None;
    }
    Some((range_start, range_end, segments))
}

pub struct RustSyntaxAuthority<'a> {
    pub compilation_id: &'a str,
    pub model_digest: &'a str,
    pub package: &'a str,
    pub target_kind: &'a str,
    pub target_name: &'a str,
    pub source_path: &'a str,
    pub cargo_version: &'a str,
    pub rustc_version: &'a str,
}

pub fn build_syntax_index(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    authority: &RustSyntaxAuthority<'_>,
) -> Result<Value, ClewError> {
    snapshot.verify()?;
    let sources = effective_sources(snapshot)?;
    if !sources.contains_key(authority.source_path) {
        return Err(invalid(
            "Cargo target source is absent from the repository snapshot",
        ));
    }
    let mut queue = VecDeque::from([(
        authority.source_path.to_owned(),
        root_module_dir(authority.source_path)?,
    )]);
    let mut visited = BTreeSet::new();
    let mut files = BTreeMap::new();
    let mut descriptors = BTreeMap::new();
    let mut boundaries = BTreeSet::new();
    let mut direct_reference_count = 0usize;
    let mut total_bytes = 0usize;
    while let Some((path, module_dir)) = queue.pop_front() {
        if visited.contains(&path) {
            continue;
        }
        if visited.len() >= MAX_SOURCE_FILES {
            boundary(&mut boundaries, "RUST_SOURCE_FILE_LIMIT", None, None);
            break;
        }
        visited.insert(path.clone());
        let object = sources
            .get(&path)
            .ok_or_else(|| invalid("selected Rust module disappeared from its snapshot"))?;
        let size = usize::try_from(object.size)
            .map_err(|_| resource("Rust source input exceeds host size"))?;
        if size > MAX_SOURCE_FILE_BYTES {
            boundary(
                &mut boundaries,
                "RUST_SOURCE_FILE_TOO_LARGE",
                Some(&path),
                None,
            );
            continue;
        }
        if total_bytes.saturating_add(size) > MAX_TOTAL_SOURCE_BYTES {
            boundary(
                &mut boundaries,
                "RUST_TOTAL_SOURCE_LIMIT",
                Some(&path),
                None,
            );
            break;
        }
        let lease = store.read(object, MAX_SOURCE_FILE_BYTES)?;
        let bytes = lease.bytes();
        total_bytes += bytes.len();
        let content_hash = canonical::hash_bytes(bytes);
        files.insert(
            path.clone(),
            json!({"path":path,"contentHash":content_hash}),
        );
        let source = match std::str::from_utf8(bytes) {
            Ok(source) => source,
            Err(_) => {
                boundary(&mut boundaries, "RUST_SOURCE_NOT_UTF8", Some(&path), None);
                continue;
            }
        };
        let syntax = match syn::parse_file(source) {
            Ok(syntax) => syntax,
            Err(_) => {
                boundary(&mut boundaries, "RUST_PARSE_FAILED", Some(&path), None);
                continue;
            }
        };
        let line_index = source_line_index(source);
        let mut context = SyntaxContext {
            path: &path,
            source,
            line_index: &line_index,
            sources: &sources,
            descriptors: &mut descriptors,
            boundaries: &mut boundaries,
            queue: &mut queue,
            direct_reference_count: &mut direct_reference_count,
        };
        visit_items(&syntax.items, &module_dir, 0, &mut context)?;
    }
    let index = json!({
        "schema":RUST_INDEX_SCHEMA,
        "compilation":authority.compilation_id,
        "modelDigest":authority.model_digest,
        "package":authority.package,
        "targetKind":authority.target_kind,
        "targetName":authority.target_name,
        "sourcePath":authority.source_path,
        "cargoVersion":authority.cargo_version,
        "rustcVersion":authority.rustc_version,
        "parserAuthority":PARSER_AUTHORITY,
        "analysisCoverage":"PARTIAL",
        "analysisCertainty":"UNSURE",
        "files":files.into_values().collect::<Vec<_>>(),
        "declarationDescriptors":{
            "coverage":"PARTIAL",
            "descriptors":descriptors.into_values().collect::<Vec<_>>()
        },
        "declarationRelations":{"coverage":"PARTIAL","relations":[]},
        "boundaries":boundaries
            .into_iter()
            .map(|encoded| serde_json::from_str::<Value>(&encoded).expect("canonical boundary"))
            .collect::<Vec<_>>(),
    });
    validate_index(&index)?;
    Ok(index)
}

fn effective_sources(
    snapshot: &RepositoryInputSnapshot,
) -> Result<BTreeMap<String, crate::cas::CasObject>, ClewError> {
    let mut sources = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0 && entry.path.ends_with(".rs"))
        .map(|entry| (entry.path.clone(), entry.content.clone()))
        .collect::<BTreeMap<_, _>>();
    for entry in snapshot
        .worktree
        .iter()
        .filter(|entry| entry.path.ends_with(".rs"))
    {
        match entry.kind {
            WorktreeKind::Missing => {
                sources.remove(&entry.path);
            }
            WorktreeKind::Regular => {
                let content = entry
                    .content
                    .clone()
                    .ok_or_else(|| invalid("regular Rust snapshot input has no content"))?;
                sources.insert(entry.path.clone(), content);
            }
            WorktreeKind::Symlink => {
                sources.remove(&entry.path);
            }
        }
    }
    Ok(sources)
}

struct SyntaxContext<'a> {
    path: &'a str,
    source: &'a str,
    line_index: &'a SourceLineIndex,
    sources: &'a BTreeMap<String, crate::cas::CasObject>,
    descriptors: &'a mut BTreeMap<String, Value>,
    boundaries: &'a mut BTreeSet<String>,
    queue: &'a mut VecDeque<(String, String)>,
    direct_reference_count: &'a mut usize,
}

fn visit_items(
    items: &[syn::Item],
    module_dir: &str,
    depth: usize,
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    if depth > MAX_NESTING {
        boundary(
            context.boundaries,
            "RUST_NESTING_LIMIT",
            Some(context.path),
            None,
        );
        return Ok(());
    }
    for item in items {
        if context.descriptors.len() >= MAX_DECLARATIONS {
            boundary(context.boundaries, "RUST_DECLARATION_LIMIT", None, None);
            return Ok(());
        }
        match item {
            syn::Item::Fn(value) => add_callable_declaration(
                "function",
                &value.sig.ident,
                value,
                &value.attrs,
                Some(&value.block),
                context,
            )?,
            syn::Item::Struct(value) => {
                add_declaration("struct", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Enum(value) => {
                add_declaration("enum", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Trait(value) => {
                add_declaration("trait", &value.ident, value, &value.attrs, context)?;
                for member in &value.items {
                    if let syn::TraitItem::Fn(method) = member {
                        add_callable_declaration(
                            "trait-method",
                            &method.sig.ident,
                            method,
                            &method.attrs,
                            method.default.as_ref(),
                            context,
                        )?;
                    }
                }
            }
            syn::Item::Type(value) => {
                add_declaration("type-alias", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Const(value) => {
                add_declaration("const", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Static(value) => {
                add_declaration("static", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Impl(value) => {
                note_attributes(&value.attrs, context, "impl");
                for member in &value.items {
                    if let syn::ImplItem::Fn(method) = member {
                        add_callable_declaration(
                            "impl-method",
                            &method.sig.ident,
                            method,
                            &method.attrs,
                            Some(&method.block),
                            context,
                        )?;
                    }
                }
            }
            syn::Item::Mod(value) => {
                add_declaration("module", &value.ident, value, &value.attrs, context)?;
                if let Some((_, nested)) = &value.content {
                    let nested_dir = join_path(module_dir, &value.ident.to_string())?;
                    visit_items(nested, &nested_dir, depth + 1, context)?;
                } else {
                    resolve_external_module(value, module_dir, context)?;
                }
            }
            syn::Item::Macro(_) => boundary(
                context.boundaries,
                "RUST_MACRO_ITEM_NOT_EXPANDED",
                Some(context.path),
                None,
            ),
            _ => note_attributes(item_attrs(item), context, "item"),
        }
    }
    Ok(())
}

fn add_declaration(
    kind: &str,
    ident: &syn::Ident,
    spanned: &impl Spanned,
    attrs: &[syn::Attribute],
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    add_declaration_inner(kind, ident, spanned, attrs, None, context)
}

fn add_callable_declaration(
    kind: &str,
    ident: &syn::Ident,
    spanned: &impl Spanned,
    attrs: &[syn::Attribute],
    body: Option<&syn::Block>,
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    add_declaration_inner(kind, ident, spanned, attrs, Some(body), context)
}

fn add_declaration_inner(
    kind: &str,
    ident: &syn::Ident,
    spanned: &impl Spanned,
    attrs: &[syn::Attribute],
    callable_body: Option<Option<&syn::Block>>,
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    note_attributes(attrs, context, ident.to_string().as_str());
    let span = spanned.span();
    let start = offset(context.source, context.line_index, span.start())?;
    let end = offset(context.source, context.line_index, span.end())?;
    let name = ident.to_string();
    let identity = format!(
        "rust-syntax:{}#{}:{}@{}-{}",
        context.path, kind, name, start, end
    );
    let direct_references = callable_body
        .map(|body| collect_direct_references(body, &identity, context))
        .transpose()?;
    let match_arm_cases = callable_body
        .map(|body| collect_match_arm_cases(body, &identity, context))
        .transpose()?;
    let mut descriptor = json!({
        "symbolIdentity":identity,
        "name":name,
        "declarationKind":kind,
        "file":context.path,
        "rangeStart":start,
        "rangeEnd":end,
        "startLine":span.start().line,
        "endLine":span.end().line,
        "cfgStatus":if has_any_attribute(attrs, &["cfg", "cfg_attr"]) { "UNKNOWN" } else { "UNCONDITIONAL" },
        "resolution":"SYNTAX_EXACT",
    });
    if let Some((references, truncated)) = direct_references {
        let object = descriptor
            .as_object_mut()
            .expect("declaration descriptor is an object");
        object.insert("directReferences".into(), Value::Array(references));
        object.insert("directReferencesTruncated".into(), Value::Bool(truncated));
    }
    if let Some((cases, truncated)) = match_arm_cases {
        descriptor
            .as_object_mut()
            .expect("declaration descriptor is an object")
            .insert(
                "matchArmCases".into(),
                json!({
                    "authority":"EXACT_SNAPSHOT_TEXT_SYNTAX",
                    "coverage":if truncated { "VISIBLE_PARSED_MATCH_ARMS_PARTIAL" } else { "VISIBLE_PARSED_MATCH_ARMS_COMPLETE" },
                    "cases":cases,
                }),
            );
    }
    context.descriptors.insert(identity.clone(), descriptor);
    Ok(())
}

fn collect_match_arm_cases(
    body: Option<&syn::Block>,
    symbol_identity: &str,
    context: &mut SyntaxContext<'_>,
) -> Result<(Vec<Value>, bool), ClewError> {
    let Some(body) = body else {
        return Ok((Vec::new(), false));
    };
    let mut collector = MatchArmCaseCollector {
        source: context.source,
        line_index: context.line_index,
        symbol_identity,
        cases: BTreeMap::new(),
        payload_bytes: 0,
        truncated: false,
        error: None,
    };
    collector.visit_block(body);
    if let Some(error) = collector.error {
        return Err(error);
    }
    if collector.truncated {
        boundary(
            context.boundaries,
            "RUST_MATCH_ARM_CASE_LIMIT",
            Some(context.path),
            Some(symbol_identity),
        );
    }
    Ok((collector.cases.into_values().collect(), collector.truncated))
}

type MatchArmCaseKey = (usize, usize, String);

struct MatchArmCaseCollector<'a> {
    source: &'a str,
    line_index: &'a SourceLineIndex,
    symbol_identity: &'a str,
    cases: BTreeMap<MatchArmCaseKey, Value>,
    payload_bytes: usize,
    truncated: bool,
    error: Option<ClewError>,
}

impl MatchArmCaseCollector<'_> {
    fn source_slice(&mut self, span: proc_macro2::Span) -> Option<(usize, usize, String)> {
        let start = match offset(self.source, self.line_index, span.start()) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return None;
            }
        };
        let end = match offset(self.source, self.line_index, span.end()) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return None;
            }
        };
        self.source
            .get(start..end)
            .map(|text| (start, end, text.to_owned()))
            .or_else(|| {
                self.error = Some(invalid("Rust control-flow span is not valid UTF-8"));
                None
            })
    }

    fn record_match(&mut self, expression: &syn::ExprMatch) {
        let Some((group_start, _, _)) = self.source_slice(expression.span()) else {
            return;
        };
        for arm in &expression.arms {
            if self.error.is_some() {
                return;
            }
            if self.cases.len() >= MAX_MATCH_ARM_CASES_PER_DECLARATION {
                self.truncated = true;
                return;
            }
            let Some((pattern_start, pattern_end, pattern_text)) =
                self.source_slice(arm.pat.span())
            else {
                return;
            };
            let Some((body_start, body_end, body_text)) = self.source_slice(arm.body.span()) else {
                return;
            };
            let guard = match &arm.guard {
                Some((_, expression)) => {
                    let Some((start, end, text)) = self.source_slice(expression.span()) else {
                        return;
                    };
                    json!({"start":start,"end":end,"text":text})
                }
                None => Value::Null,
            };
            let case_authority = json!({
                "kind":"MATCH_ARM",
                "declarationIdentity":self.symbol_identity,
                "groupStart":group_start,
                "pattern":{"start":pattern_start,"end":pattern_end,"text":pattern_text},
                "guard":guard,
                "body":{"start":body_start,"end":body_end,"text":body_text},
                "authority":"EXACT_SNAPSHOT_TEXT_SYNTAX",
            });
            let case_digest = match canonical::hash(&case_authority) {
                Ok(digest) => digest,
                Err(error) => {
                    self.error = Some(internal(error));
                    return;
                }
            };
            let mut case = case_authority;
            case.as_object_mut()
                .expect("match arm case is an object")
                .insert(
                    "caseId".into(),
                    Value::String(format!("rust-case:{case_digest}")),
                );
            let bytes = match canonical::bytes(&case) {
                Ok(bytes) => bytes.len(),
                Err(error) => {
                    self.error = Some(internal(error));
                    return;
                }
            };
            let Some(next_payload_bytes) = self.payload_bytes.checked_add(bytes) else {
                self.truncated = true;
                return;
            };
            if bytes > MAX_MATCH_ARM_CASE_BYTES
                || next_payload_bytes > MAX_MATCH_ARM_CASES_PAYLOAD_BYTES
            {
                self.truncated = true;
                return;
            }
            self.payload_bytes = next_payload_bytes;
            self.cases
                .insert((group_start, pattern_start, case_digest), case);
        }
    }
}

impl<'ast> Visit<'ast> for MatchArmCaseCollector<'_> {
    fn visit_expr_match(&mut self, expression: &'ast syn::ExprMatch) {
        self.record_match(expression);
        syn::visit::visit_expr_match(self, expression);
    }

    fn visit_item(&mut self, _item: &'ast syn::Item) {
        // Nested items own their own control-flow inventory.
    }
}

fn collect_direct_references(
    body: Option<&syn::Block>,
    symbol_identity: &str,
    context: &mut SyntaxContext<'_>,
) -> Result<(Vec<Value>, bool), ClewError> {
    let Some(body) = body else {
        return Ok((Vec::new(), false));
    };
    let remaining_global = MAX_DIRECT_REFERENCES.saturating_sub(*context.direct_reference_count);
    let limit = remaining_global.min(MAX_DIRECT_REFERENCES_PER_DECLARATION);
    let mut collector = DirectReferenceCollector {
        source: context.source,
        line_index: context.line_index,
        limit,
        references: BTreeMap::new(),
        payload_bytes: 0,
        truncated: false,
        count_limit_reached: false,
        payload_limit_reached: false,
        error: None,
    };
    collector.visit_block(body);
    if let Some(error) = collector.error {
        return Err(error);
    }
    let truncated = collector.truncated;
    let count_limit_reached = collector.count_limit_reached;
    let payload_limit_reached = collector.payload_limit_reached;
    let references = collector.references.into_values().collect::<Vec<_>>();
    *context.direct_reference_count = context
        .direct_reference_count
        .checked_add(references.len())
        .ok_or_else(|| resource("Rust direct reference count overflow"))?;
    if truncated {
        if payload_limit_reached {
            boundary(
                context.boundaries,
                "RUST_DIRECT_REFERENCE_PAYLOAD_LIMIT",
                Some(context.path),
                Some(symbol_identity),
            );
        }
        if count_limit_reached && remaining_global <= MAX_DIRECT_REFERENCES_PER_DECLARATION {
            boundary(
                context.boundaries,
                "RUST_DIRECT_REFERENCE_GLOBAL_LIMIT",
                None,
                None,
            );
        }
        if count_limit_reached && remaining_global >= MAX_DIRECT_REFERENCES_PER_DECLARATION {
            boundary(
                context.boundaries,
                "RUST_DIRECT_REFERENCE_DECLARATION_LIMIT",
                Some(context.path),
                Some(symbol_identity),
            );
        }
    }
    Ok((references, truncated))
}

type DirectReferenceKey = (usize, usize, Vec<String>);

struct DirectReferenceCollector<'a> {
    source: &'a str,
    line_index: &'a SourceLineIndex,
    limit: usize,
    references: BTreeMap<DirectReferenceKey, Value>,
    payload_bytes: usize,
    truncated: bool,
    count_limit_reached: bool,
    payload_limit_reached: bool,
    error: Option<ClewError>,
}

impl DirectReferenceCollector<'_> {
    fn record(&mut self, path: &syn::ExprPath) {
        if self.error.is_some() || path.qself.is_some() {
            return;
        }
        let segments = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let Some(terminal_name) = segments.last().cloned() else {
            return;
        };
        if segments.len() > MAX_DIRECT_REFERENCE_PATH_SEGMENTS
            || segments
                .iter()
                .any(|segment| segment.len() > MAX_DIRECT_REFERENCE_PATH_SEGMENT_BYTES)
        {
            self.truncated = true;
            self.payload_limit_reached = true;
            return;
        }
        let span = path.span();
        let start = match offset(self.source, self.line_index, span.start()) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let end = match offset(self.source, self.line_index, span.end()) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let key = (start, end, segments.clone());
        if self.references.contains_key(&key) {
            return;
        }
        if self.references.len() >= self.limit {
            self.truncated = true;
            self.count_limit_reached = true;
            return;
        }
        let reference = json!({
            "kind":"CALL_PATH",
            "pathSegments":segments,
            "terminalName":terminal_name,
            "rangeStart":start,
            "rangeEnd":end,
            "startLine":span.start().line,
            "endLine":span.end().line,
            "resolution":"SYNTAX_UNRESOLVED",
        });
        let bytes = match canonical::bytes(&reference) {
            Ok(bytes) => bytes.len(),
            Err(error) => {
                self.error = Some(internal(error));
                return;
            }
        };
        let Some(next_payload_bytes) = self.payload_bytes.checked_add(bytes) else {
            self.truncated = true;
            self.payload_limit_reached = true;
            return;
        };
        if bytes > MAX_DIRECT_REFERENCE_BYTES
            || next_payload_bytes > MAX_DIRECT_REFERENCES_PAYLOAD_BYTES
        {
            self.truncated = true;
            self.payload_limit_reached = true;
            return;
        }
        self.payload_bytes = next_payload_bytes;
        self.references.insert(key, reference);
    }
}

impl<'ast> Visit<'ast> for DirectReferenceCollector<'_> {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            self.record(path);
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_item(&mut self, _item: &'ast syn::Item) {
        // A nested item has its own declaration authority. Its calls do not
        // belong to the enclosing callable descriptor.
    }
}

fn resolve_external_module(
    module: &syn::ItemMod,
    module_dir: &str,
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    let name = module.ident.to_string();
    if has_attribute(&module.attrs, "path") {
        boundary(
            context.boundaries,
            "RUST_CUSTOM_MODULE_PATH",
            Some(context.path),
            Some(&name),
        );
        return Ok(());
    }
    if has_attribute(&module.attrs, "cfg") {
        boundary(
            context.boundaries,
            "RUST_CFG_MODULE_NOT_EXPANDED",
            Some(context.path),
            Some(&name),
        );
        return Ok(());
    }
    if has_attribute(&module.attrs, "cfg_attr") {
        boundary(
            context.boundaries,
            "RUST_CFG_ATTR_MODULE_NOT_EXPANDED",
            Some(context.path),
            Some(&name),
        );
        return Ok(());
    }
    let flat = join_path(module_dir, &format!("{name}.rs"))?;
    let nested = join_path(module_dir, &format!("{name}/mod.rs"))?;
    let matches = [flat, nested]
        .into_iter()
        .filter(|candidate| context.sources.contains_key(candidate))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [selected] => context
            .queue
            .push_back((selected.clone(), join_path(module_dir, &name)?)),
        [] => boundary(
            context.boundaries,
            "RUST_MODULE_SOURCE_MISSING",
            Some(context.path),
            Some(&name),
        ),
        _ => boundary(
            context.boundaries,
            "RUST_MODULE_SOURCE_AMBIGUOUS",
            Some(context.path),
            Some(&name),
        ),
    }
    Ok(())
}

fn note_attributes(attrs: &[syn::Attribute], context: &mut SyntaxContext<'_>, subject: &str) {
    if has_attribute(attrs, "cfg") {
        boundary(
            context.boundaries,
            "RUST_CFG_NOT_EVALUATED",
            Some(context.path),
            Some(subject),
        );
    }
    if has_attribute(attrs, "cfg_attr") {
        boundary(
            context.boundaries,
            "RUST_CFG_ATTR_NOT_EVALUATED",
            Some(context.path),
            Some(subject),
        );
    }
    if has_attribute(attrs, "derive") {
        boundary(
            context.boundaries,
            "RUST_DERIVE_NOT_EXPANDED",
            Some(context.path),
            Some(subject),
        );
    }
    if attrs.iter().any(|attribute| {
        !BUILTIN_ATTRIBUTES
            .iter()
            .any(|name| attribute.path().is_ident(name))
    }) {
        boundary(
            context.boundaries,
            "RUST_ATTRIBUTE_MACRO_UNKNOWN",
            Some(context.path),
            Some(subject),
        );
    }
}

fn has_attribute(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}

fn has_any_attribute(attrs: &[syn::Attribute], names: &[&str]) -> bool {
    names.iter().any(|name| has_attribute(attrs, name))
}

const BUILTIN_ATTRIBUTES: &[&str] = &[
    "cfg",
    "cfg_attr",
    "derive",
    "doc",
    "allow",
    "warn",
    "deny",
    "forbid",
    "inline",
    "must_use",
    "deprecated",
    "repr",
    "non_exhaustive",
    "path",
    "test",
];

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::ExternCrate(v) => &v.attrs,
        syn::Item::ForeignMod(v) => &v.attrs,
        syn::Item::Macro(v) => &v.attrs,
        syn::Item::TraitAlias(v) => &v.attrs,
        syn::Item::Union(v) => &v.attrs,
        syn::Item::Use(v) => &v.attrs,
        _ => &[],
    }
}

struct SourceLineIndex {
    byte_starts: Vec<usize>,
    non_ascii_byte_columns: BTreeMap<usize, Vec<usize>>,
}

fn source_line_index(source: &str) -> SourceLineIndex {
    let byte_starts = std::iter::once(0)
        .chain(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        )
        .collect::<Vec<_>>();
    let mut non_ascii_byte_columns = BTreeMap::new();
    for line in 0..byte_starts.len() {
        let start = byte_starts[line];
        let end = byte_starts.get(line + 1).copied().unwrap_or(source.len());
        let text = &source[start..end];
        if !text.is_ascii() {
            let mut columns = text
                .char_indices()
                .map(|(byte_offset, _)| byte_offset)
                .collect::<Vec<_>>();
            columns.push(text.len());
            non_ascii_byte_columns.insert(line, columns);
        }
    }
    SourceLineIndex {
        byte_starts,
        non_ascii_byte_columns,
    }
}

fn offset(
    source: &str,
    index: &SourceLineIndex,
    location: proc_macro2::LineColumn,
) -> Result<usize, ClewError> {
    let line = location
        .line
        .checked_sub(1)
        .ok_or_else(|| invalid("Rust parser returned an invalid line"))?;
    let start = *index
        .byte_starts
        .get(line)
        .ok_or_else(|| invalid("Rust parser span escaped its source"))?;
    let line_end = index
        .byte_starts
        .get(line + 1)
        .copied()
        .unwrap_or(source.len());
    let byte_column = index
        .non_ascii_byte_columns
        .get(&line)
        .map(|columns| columns.get(location.column).copied())
        .unwrap_or(Some(location.column))
        .ok_or_else(|| invalid("Rust parser span escaped its source"))?;
    let offset = start
        .checked_add(byte_column)
        .ok_or_else(|| invalid("Rust parser span escaped its source"))?;
    if offset > line_end || !source.is_char_boundary(offset) {
        return Err(invalid("Rust parser span escaped its source"));
    }
    Ok(offset)
}

fn root_module_dir(source_path: &str) -> Result<String, ClewError> {
    let parent = Path::new(source_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    relative_path(parent)
}

fn join_path(base: &str, suffix: &str) -> Result<String, ClewError> {
    relative_path(&PathBuf::from(base).join(suffix))
}

fn relative_path(path: &Path) -> Result<String, ClewError> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("Rust module path is not UTF-8"))?;
    if text.is_empty() {
        Ok(String::new())
    } else if safe_path(text) {
        Ok(text.replace('\\', "/"))
    } else {
        Err(invalid("Rust module path is outside the repository"))
    }
}

fn boundary(
    boundaries: &mut BTreeSet<String>,
    code: &str,
    file: Option<&str>,
    subject: Option<&str>,
) {
    let value = json!({"code":code,"file":file,"subject":subject,"resolution":"UNKNOWN"});
    let encoded =
        String::from_utf8(canonical::bytes(&value).expect("static boundary is canonical"))
            .expect("canonical JSON is UTF-8");
    if boundaries.contains(&encoded) {
        return;
    }
    if boundaries.len() < MAX_BOUNDARIES - 1 {
        boundaries.insert(encoded);
    } else {
        let limit = json!({
            "code":"RUST_BOUNDARY_LIMIT",
            "file":Value::Null,
            "subject":Value::Null,
            "resolution":"UNKNOWN"
        });
        boundaries.insert(
            String::from_utf8(canonical::bytes(&limit).expect("static boundary is canonical"))
                .expect("canonical JSON is UTF-8"),
        );
    }
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn require_digest(value: &str) -> Result<(), ClewError> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("Rust adapter authority digest is invalid"));
    }
    Ok(())
}

fn cancelled_error() -> ClewError {
    ClewError::new(
        ErrorCode::TransactionRecoveryRequired,
        "Rust analysis was cancelled",
    )
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn poisoned<T>(error: std::sync::PoisonError<T>) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        PreparedFactBatch, RUST_INDEX_SCHEMA, RustSyntaxAuthority, build_syntax_index,
        translate_facts,
    };
    use crate::cas::CasStore;
    use crate::repository_snapshot;
    use crate::state::StateAuthority;
    use serde_json::{Value, json};
    use std::fs;
    use std::process::Command;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    #[test]
    fn cargo_target_facts_are_granular_path_safe_and_deterministic() {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let index = json!({
            "schema":RUST_INDEX_SCHEMA,
            "compilation":"cargo-Cargo.toml-demo-lib-demo",
            "modelDigest":digest('1'),
            "package":"demo",
            "targetKind":"lib",
            "targetName":"demo",
            "sourcePath":"src/lib.rs",
            "cargoVersion":"cargo 1.92.0",
            "rustcVersion":"rustc 1.92.0",
            "parserAuthority":super::PARSER_AUTHORITY,
            "analysisCoverage":"PARTIAL",
            "analysisCertainty":"UNSURE",
            "files":[{"path":"src/lib.rs","contentHash":digest('2')}],
            "declarationDescriptors":{"coverage":"PARTIAL","descriptors":[]},
            "declarationRelations":{"coverage":"PARTIAL","relations":[]},
            "boundaries":[],
        });
        let first = translate_facts(&store, &index).unwrap();
        let second = translate_facts(&store, &index).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_eq!(
            regular_file_count(&temporary.path().join("v2/objects/sha256")),
            0
        );
        assert_eq!(
            fs::read_dir(temporary.path().join("v2/objects/packs-v3"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension() == Some(std::ffi::OsStr::new("pack")))
                .count(),
            1
        );
        for fact in first {
            let lease = store.read(&fact.payload, 4096).unwrap();
            let text = std::str::from_utf8(lease.bytes()).unwrap();
            assert!(!text.contains("/Users/"));
            assert!(!text.contains("file://"));
        }
    }

    #[test]
    fn fact_batch_input_limit_rejects_before_append() {
        let first = json!({"schema":"test/fact/1","name":"first"});
        let second = json!({"schema":"test/fact/1","name":"second"});
        let exact = crate::canonical::bytes(&first).unwrap().len();
        let mut batch = PreparedFactBatch::default();
        batch.push(first, exact).unwrap();
        assert_eq!(batch.input_bytes, exact);
        assert_eq!(batch.prepared.len(), 1);
        let retained = batch.prepared.clone();
        assert!(batch.push(second, exact).is_err());
        assert_eq!(batch.input_bytes, exact);
        assert_eq!(batch.prepared, retained);
    }

    #[test]
    fn syntax_index_follows_only_unambiguous_modules_and_emits_no_relations() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "mod helper;\npub fn format_message() {}\nmacro_rules! generated { () => {} }\n",
        )
        .unwrap();
        fs::write(
            repo.join("src/helper.rs"),
            "#[derive(Debug)]\npub struct Formatter;\nimpl Formatter { pub fn format_message(&self) {} }\n#[cfg(test)] pub fn conditional() {}\n",
        )
        .unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        let state = StateAuthority::open(temporary.path().join("syntax-state-v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = repository_snapshot::capture(&repo, &store).unwrap();
        let first = build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        let second =
            build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        assert_eq!(first, second);
        assert_eq!(first["files"].as_array().unwrap().len(), 2);
        let descriptors = first["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap();
        assert!(descriptors.iter().any(|row| {
            row["name"] == "format_message"
                && row["file"] == "src/lib.rs"
                && row["resolution"] == "SYNTAX_EXACT"
        }));
        assert!(
            descriptors
                .iter()
                .any(|row| { row["name"] == "format_message" && row["file"] == "src/helper.rs" })
        );
        assert!(
            first["declarationRelations"]["relations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let boundary_codes = first["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(boundary_codes.contains(&"RUST_DERIVE_NOT_EXPANDED"));
        assert!(boundary_codes.contains(&"RUST_CFG_NOT_EVALUATED"));
        assert!(boundary_codes.contains(&"RUST_MACRO_ITEM_NOT_EXPANDED"));

        let loose_before =
            regular_file_count(&temporary.path().join("syntax-state-v2/objects/sha256"));
        let facts = translate_facts(&store, &first).unwrap();
        assert_eq!(
            regular_file_count(&temporary.path().join("syntax-state-v2/objects/sha256")),
            loose_before
        );
        assert!(facts.len() > 6);
        for fact in facts {
            let lease = store.read(&fact.payload, 64 * 1024).unwrap();
            assert!(lease.bytes().len() <= 64 * 1024);
            let text = std::str::from_utf8(lease.bytes()).unwrap();
            assert!(!text.contains("/Users/"));
            assert!(!text.contains("/private/"));
            assert!(!text.contains("\"target\":"));
        }
    }

    #[test]
    fn ambiguous_external_module_is_a_boundary_not_an_expansion() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        fs::create_dir_all(repo.join("src/helper")).unwrap();
        fs::write(repo.join("src/lib.rs"), "mod helper;\n").unwrap();
        fs::write(repo.join("src/helper.rs"), "pub fn flat() {}\n").unwrap();
        fs::write(repo.join("src/helper/mod.rs"), "pub fn nested() {}\n").unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        let state = StateAuthority::open(temporary.path().join("ambiguity-state-v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = repository_snapshot::capture(&repo, &store).unwrap();
        let index = build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        assert_eq!(index["files"].as_array().unwrap().len(), 1);
        assert!(
            index["boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| { row["code"] == "RUST_MODULE_SOURCE_AMBIGUOUS" })
        );
        assert!(
            index["declarationRelations"]["relations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cfg_attr_module_and_namespaced_builtin_names_fail_closed() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "#[cfg_attr(feature = \"alternate\", path = \"alternate.rs\")]\nmod selected;\n#[my::cfg]\npub fn namespaced_attribute() {}\n",
        )
        .unwrap();
        fs::write(repo.join("src/selected.rs"), "pub fn default_child() {}\n").unwrap();
        fs::write(
            repo.join("src/alternate.rs"),
            "pub fn alternate_child() {}\n",
        )
        .unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        let state = StateAuthority::open(temporary.path().join("cfg-state-v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = repository_snapshot::capture(&repo, &store).unwrap();
        let index = build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        assert_eq!(index["files"].as_array().unwrap().len(), 1);
        let names = index["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["name"].as_str())
            .collect::<Vec<_>>();
        assert!(!names.contains(&"default_child"));
        assert!(!names.contains(&"alternate_child"));
        assert!(
            index["declarationDescriptors"]["descriptors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"] == "selected" && row["cfgStatus"] == "UNKNOWN")
        );
        let boundaries = index["boundaries"].as_array().unwrap();
        assert!(
            boundaries
                .iter()
                .any(|row| { row["code"] == "RUST_CFG_ATTR_MODULE_NOT_EXPANDED" })
        );
        assert!(
            boundaries
                .iter()
                .any(|row| { row["code"] == "RUST_CFG_ATTR_NOT_EVALUATED" })
        );
        assert!(boundaries.iter().any(|row| {
            row["code"] == "RUST_ATTRIBUTE_MACRO_UNKNOWN"
                && row["subject"] == "namespaced_attribute"
        }));
        assert!(
            index["declarationRelations"]["relations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn match_case_sidecar_is_generic_exact_and_closed_for_unrelated_code() {
        let temporary = tempfile::tempdir().unwrap();
        let source = r#"
enum Mode { Keep, Change }
fn apply(mode: Mode, input: String) -> String {
    match mode {
        Mode::Keep => input,
        Mode::Change if !input.is_empty() => input.to_uppercase(),
        Mode::Change => String::new(),
    }
}
"#;
        let index = syntax_index_for_source(&temporary, source, "case-sidecar-state");
        let descriptor = index["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|descriptor| descriptor["name"] == "apply")
            .unwrap();
        let sidecar = &descriptor["matchArmCases"];
        assert_eq!(sidecar["authority"], "EXACT_SNAPSHOT_TEXT_SYNTAX");
        assert_eq!(sidecar["coverage"], "VISIBLE_PARSED_MATCH_ARMS_COMPLETE");
        let cases = sidecar["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 3);
        assert_eq!(cases[0]["pattern"]["text"], "Mode::Keep");
        assert_eq!(cases[0]["body"]["text"], "input");
        assert_eq!(cases[0]["guard"], Value::Null);
        assert_eq!(cases[1]["pattern"]["text"], "Mode::Change");
        assert_eq!(cases[1]["guard"]["text"], "!input.is_empty()");
        assert_eq!(cases[1]["body"]["text"], "input.to_uppercase()");
        assert!(cases.iter().all(|case| {
            case["caseId"]
                .as_str()
                .unwrap()
                .starts_with("rust-case:sha256:")
                && case["declarationIdentity"] == descriptor["symbolIdentity"]
        }));
        assert!(super::validate_index(&index).is_ok());

        let state = StateAuthority::open(temporary.path().join("case-sidecar-state")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let translated = translate_facts(&store, &index).unwrap();
        assert!(translated.iter().any(|fact| {
            let lease = store.read(&fact.payload, 64 * 1024).unwrap();
            let payload: Value = serde_json::from_slice(lease.bytes()).unwrap();
            payload["name"] == "apply"
                && payload["matchArmCases"]["cases"][1]["body"]["text"] == "input.to_uppercase()"
        }));

        let changed_temporary = tempfile::tempdir().unwrap();
        let changed_index = syntax_index_for_source(
            &changed_temporary,
            &source.replace("to_uppercase", "to_lowercase"),
            "changed-case-sidecar-state",
        );
        let changed_descriptor = changed_index["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|descriptor| descriptor["name"] == "apply")
            .unwrap();
        assert_ne!(
            cases[1]["caseId"],
            changed_descriptor["matchArmCases"]["cases"][1]["caseId"]
        );
    }

    #[test]
    fn direct_call_paths_preserve_utf8_positions_without_claiming_resolution() {
        let temporary = tempfile::tempdir().unwrap();
        let source = r#"pub fn aggregate_completeness() {}
mod child {
    pub fn caller() {
        let _label = "__UNICODE_LABEL__"; super::aggregate_completeness();
        alpha::same();
        beta::same();
        let callable = super::aggregate_completeness;
        (callable)();
        value.method();
        call_macro!();
        fn nested() { hidden(); }
    }
    pub trait Worker {
        fn required();
        fn defaulted() { super::aggregate_completeness(); }
    }
    pub struct Demo;
    impl Demo {
        fn run(&self) { super::aggregate_completeness(); self.method(); }
        fn method(&self) {}
    }
}
"#
        .replace(
            "__UNICODE_LABEL__",
            "\u{043a}\u{0438}\u{0440}\u{0438}\u{043b}\u{043b}\u{0438}\u{0446}\u{0430}\u{1f642}",
        );
        let index = syntax_index_for_source(&temporary, &source, "direct-reference-state-v2");
        let descriptors = index["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap();
        let caller = descriptors
            .iter()
            .find(|descriptor| descriptor["name"] == "caller")
            .unwrap();
        let references = caller["directReferences"].as_array().unwrap();
        assert_eq!(references.len(), 3);
        assert_eq!(references[0]["kind"], "CALL_PATH");
        assert_eq!(
            references[0]["pathSegments"],
            json!(["super", "aggregate_completeness"])
        );
        assert_eq!(references[0]["terminalName"], "aggregate_completeness");
        assert_eq!(references[0]["resolution"], "SYNTAX_UNRESOLVED");
        let expected_start = source.find("super::aggregate_completeness();").unwrap();
        assert_eq!(
            references[0]["rangeStart"].as_u64().unwrap(),
            expected_start as u64
        );
        assert_eq!(
            references[0]["rangeEnd"].as_u64().unwrap(),
            (expected_start + "super::aggregate_completeness".len()) as u64
        );
        assert_eq!(references[0]["startLine"], 4);
        assert_eq!(references[0]["endLine"], 4);
        assert_eq!(references[1]["pathSegments"], json!(["alpha", "same"]));
        assert_eq!(references[2]["pathSegments"], json!(["beta", "same"]));
        assert_eq!(caller["directReferencesTruncated"], false);
        assert!(references.iter().all(|reference| {
            !matches!(
                reference["terminalName"].as_str(),
                Some("method" | "call_macro" | "callable" | "hidden")
            )
        }));

        let required = descriptors
            .iter()
            .find(|descriptor| descriptor["name"] == "required")
            .unwrap();
        assert_eq!(required["directReferences"], json!([]));
        assert_eq!(required["directReferencesTruncated"], false);
        let defaulted = descriptors
            .iter()
            .find(|descriptor| descriptor["name"] == "defaulted")
            .unwrap();
        assert_eq!(defaulted["directReferences"].as_array().unwrap().len(), 1);
        let run = descriptors
            .iter()
            .find(|descriptor| descriptor["name"] == "run")
            .unwrap();
        assert_eq!(run["directReferences"].as_array().unwrap().len(), 1);
        let demo = descriptors
            .iter()
            .find(|descriptor| descriptor["name"] == "Demo")
            .unwrap();
        assert!(demo.get("directReferences").is_none());

        let mut tampered = index.clone();
        let tampered_reference = tampered["declarationDescriptors"]["descriptors"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|descriptor| descriptor["name"] == "caller")
            .unwrap()["directReferences"]
            .as_array_mut()
            .unwrap()
            .first_mut()
            .unwrap();
        tampered_reference["terminalName"] = json!("wrong");
        assert!(super::validate_index(&tampered).is_err());
    }

    #[test]
    fn direct_call_path_limits_truncate_with_closed_boundaries() {
        let temporary = tempfile::tempdir().unwrap();
        let calls = (0..65)
            .map(|index| format!("call_{index}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let long_path = (0..33)
            .map(|index| format!("segment_{index}"))
            .collect::<Vec<_>>()
            .join("::");
        let source = format!(
            "pub fn many() {{\n{calls}\n}}\npub fn oversized_path() {{ {long_path}(); }}\n"
        );
        let index = syntax_index_for_source(&temporary, &source, "reference-limit-state-v2");
        let descriptors = index["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap();
        let many = descriptors
            .iter()
            .find(|descriptor| descriptor["name"] == "many")
            .unwrap();
        assert_eq!(many["directReferences"].as_array().unwrap().len(), 64);
        assert_eq!(many["directReferencesTruncated"], true);
        let oversized = descriptors
            .iter()
            .find(|descriptor| descriptor["name"] == "oversized_path")
            .unwrap();
        assert!(oversized["directReferences"].as_array().unwrap().is_empty());
        assert_eq!(oversized["directReferencesTruncated"], true);
        let boundary_codes = index["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|boundary| boundary["code"].as_str())
            .collect::<Vec<_>>();
        assert!(boundary_codes.contains(&"RUST_DIRECT_REFERENCE_DECLARATION_LIMIT"));
        assert!(boundary_codes.contains(&"RUST_DIRECT_REFERENCE_PAYLOAD_LIMIT"));
        let state = StateAuthority::open(temporary.path().join("translated-state-v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        assert!(translate_facts(&store, &index).is_ok());
    }

    fn syntax_index_for_source(
        temporary: &tempfile::TempDir,
        source: &str,
        state_name: &str,
    ) -> serde_json::Value {
        let repo = temporary.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/lib.rs"), source).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        let state = StateAuthority::open(temporary.path().join(state_name)).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = repository_snapshot::capture(&repo, &store).unwrap();
        build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap()
    }

    fn git(repo: &std::path::Path, arguments: &[&str]) {
        assert!(
            Command::new("git")
                .args(arguments)
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
    }

    fn syntax_authority(model_digest: &str) -> RustSyntaxAuthority<'_> {
        RustSyntaxAuthority {
            compilation_id: "cargo-Cargo.toml-demo-lib-demo",
            model_digest,
            package: "demo",
            target_kind: "lib",
            target_name: "demo",
            source_path: "src/lib.rs",
            cargo_version: "cargo 1.92.0",
            rustc_version: "rustc 1.92.0",
        }
    }

    fn regular_file_count(root: &std::path::Path) -> usize {
        fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| {
                if entry.file_type().unwrap().is_dir() {
                    regular_file_count(&entry.path())
                } else {
                    usize::from(entry.file_type().unwrap().is_file())
                }
            })
            .sum()
    }
}
