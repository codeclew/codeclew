//! Roslyn-backed C# facts replayed through the language adapter DAG.
use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::CasStore;
use crate::csharp_project_model::{CSharpProjectModel, digest, safe_relative_path, verify_model};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub const CSHARP_LANGUAGE: &str = "language:csharp";
pub const CSHARP_COMPILER_FACTS_CAPABILITY: &str = "analysis:csharp-compiler-facts";
pub const CSHARP_INDEX_SCHEMA: &str = "codeclew-csharp-compiler-index/1.0";
pub const CSHARP_FACT_SCHEMA: &str = "codeclew-csharp-compiler-fact/1.0";
const CSHARP_RECEIPT_SCHEMA: &str = "codeclew-csharp-compiler-completeness/1.0";
const CSHARP_ADAPTER_AUTHORITY_SCHEMA: &str = "codeclew-csharp-compiler-adapter/1.0";
const ADAPTER_ID: &str = "csharp-roslyn-1";
const MAX_CSHARP_FACTS: usize = 1_048_576;
const MAX_FACT_BYTES: usize = 256 * 1024;

/// Closed C# compiler fact contract. Identities use the portable grammar shared
/// with the JVM analyzers (`class:Ns.Type`, `method:class:Ns.Type#Name(desc)ret`);
/// `csharpIdentity` retains the Roslyn documentation-comment ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CSharpCompilerFact {
    SourceFile {
        schema: String,
        file: String,
        source_content_digest: String,
        resolution: String,
    },
    Declaration {
        schema: String,
        declaration_kind: String,
        name: String,
        symbol_identity: String,
        owner_identity: String,
        csharp_identity: String,
        accessibility: String,
        modifiers: Vec<String>,
        annotations: Vec<String>,
        project: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generated: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qualified_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interfaces: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        superclass: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial_parts: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jvm_descriptor: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        return_type: Option<String>,
        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        value_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parameters: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature_types: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attributes: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        overrides: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        implements: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clr_attributes: Option<Box<Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        documentation: Option<Box<Value>>,
        file: String,
        start: u64,
        end: u64,
        start_line: u64,
        end_line: u64,
        byte_start: u64,
        byte_end: u64,
        resolution: String,
    },
    Relation {
        schema: String,
        relation_kind: String,
        source_identity: String,
        target_identity: String,
        target_csharp_identity: String,
        file: String,
        start: u64,
        end: u64,
        start_line: u64,
        end_line: u64,
        byte_start: u64,
        byte_end: u64,
        resolution: String,
    },
    Boundary {
        schema: String,
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diagnostic_code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_line: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_line: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        byte_start: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        byte_end: Option<u64>,
        required_checks: Vec<String>,
        resolution: String,
    },
}

impl CSharpCompilerFact {
    fn schema(&self) -> &str {
        match self {
            Self::SourceFile { schema, .. }
            | Self::Declaration { schema, .. }
            | Self::Relation { schema, .. }
            | Self::Boundary { schema, .. } => schema,
        }
    }

    pub(crate) fn path(&self) -> Option<&str> {
        match self {
            Self::SourceFile { file, .. }
            | Self::Declaration { file, .. }
            | Self::Relation { file, .. } => Some(file),
            Self::Boundary { file, .. } => file.as_deref(),
        }
    }

    pub(crate) fn boundary_code(&self) -> Option<&str> {
        match self {
            Self::Boundary { code, .. } => Some(code),
            _ => None,
        }
    }

    fn query_family(&self) -> String {
        match self {
            Self::Declaration { name, .. } => {
                format!("0-declaration:{}", query_key_component(name))
            }
            Self::Relation { relation_kind, .. } => {
                format!("1-relation:{}", query_key_component(relation_kind))
            }
            Self::Boundary { code, .. } => format!("2-boundary:{}", query_key_component(code)),
            Self::SourceFile { .. } => "3-source-file".into(),
        }
    }

    fn position_is_valid(&self) -> bool {
        match self {
            Self::Declaration {
                start,
                end,
                start_line,
                end_line,
                byte_start,
                byte_end,
                ..
            }
            | Self::Relation {
                start,
                end,
                start_line,
                end_line,
                byte_start,
                byte_end,
                ..
            } => {
                start <= end && *start_line > 0 && start_line <= end_line && byte_start <= byte_end
            }
            _ => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CSharpCompilerIndex {
    pub schema: String,
    pub compilation: String,
    pub model: CSharpProjectModel,
    pub facts: Vec<CSharpCompilerFact>,
}

pub fn csharp_adapter_digest(worker_tree_hash: &str) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":CSHARP_ADAPTER_AUTHORITY_SCHEMA,
        "indexSchema":CSHARP_INDEX_SCHEMA,
        "factSchema":CSHARP_FACT_SCHEMA,
        "capability":CSHARP_COMPILER_FACTS_CAPABILITY,
        "workerTreeHash":worker_tree_hash,
        "compilerApi":"roslyn",
    }))
    .map_err(internal)
}

/// Seal worker facts with exact snapshot membership. Facts whose file is not a
/// sealed snapshot source (for example an ignored file the project compiles)
/// are excluded and recorded as one boundary.
pub fn build_csharp_compiler_index(
    model: CSharpProjectModel,
    facts: Vec<CSharpCompilerFact>,
    source_content_digests: &BTreeMap<String, String>,
) -> Result<CSharpCompilerIndex, ClewError> {
    verify_model(&model)?;
    let mut retained = Vec::with_capacity(facts.len() + source_content_digests.len());
    let mut outside = 0u64;
    for fact in facts {
        match fact.path() {
            Some(path) if !source_content_digests.contains_key(path) => outside += 1,
            _ => retained.push(fact),
        }
    }
    if outside > 0 {
        retained.push(CSharpCompilerFact::Boundary {
            schema: CSHARP_FACT_SCHEMA.into(),
            code: "CSHARP_SOURCE_OUTSIDE_SEALED_SNAPSHOT".into(),
            diagnostic_code: None,
            count: Some(outside),
            file: None,
            start: None,
            end: None,
            start_line: None,
            end_line: None,
            byte_start: None,
            byte_end: None,
            required_checks: vec!["VERIFY_CSHARP_SOURCE_MEMBERSHIP".into()],
            resolution: "UNKNOWN".into(),
        });
    }
    for (path, digest) in source_content_digests {
        retained.push(CSharpCompilerFact::SourceFile {
            schema: CSHARP_FACT_SCHEMA.into(),
            file: path.clone(),
            source_content_digest: digest.clone(),
            resolution: "SOURCE_MEMBERSHIP_EXACT".into(),
        });
    }
    retained.sort_by_cached_key(|fact| canonical::bytes(fact).expect("serializable C# fact"));
    retained.dedup();
    let index = CSharpCompilerIndex {
        schema: CSHARP_INDEX_SCHEMA.into(),
        compilation: model.compilation.clone(),
        model,
        facts: retained,
    };
    validate_index(&index)?;
    Ok(index)
}

pub fn csharp_scope_digest(index: &CSharpCompilerIndex) -> Result<String, ClewError> {
    validate_index(index)?;
    canonical::hash(&json!({
        "schema":"codeclew-csharp-compiler-scope/1.0",
        "compilation":index.compilation,
        "modelDigest":index.model.model_digest,
        "factsDigest":canonical::hash(&index.facts).map_err(internal)?,
        "factCount":index.facts.len(),
    }))
    .map_err(internal)
}

pub struct CSharpAdapterV2 {
    adapter_digest: String,
    toolchain_digest: String,
    compilation_id: String,
    store: CasStore,
    index: CSharpCompilerIndex,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl CSharpAdapterV2 {
    pub fn new(
        adapter_digest: String,
        toolchain_digest: String,
        compilation_id: String,
        store: CasStore,
        index: CSharpCompilerIndex,
    ) -> Result<Self, ClewError> {
        validate_index(&index)?;
        if !digest(&adapter_digest)
            || !digest(&toolchain_digest)
            || compilation_id.is_empty()
            || compilation_id.len() > 120
        {
            return Err(invalid("C# adapter authority is invalid"));
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

impl LanguageAdapter for CSharpAdapterV2 {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
        Ok(AdapterHandshake {
            protocol: ADAPTER_PROTOCOL.into(),
            adapter_id: ADAPTER_ID.into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(CSHARP_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(CSHARP_COMPILER_FACTS_CAPABILITY)?],
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
        if request.compilation.language_uri.as_str() != CSHARP_LANGUAGE
            || request.capability.as_str() != CSHARP_COMPILER_FACTS_CAPABILITY
            || request.compilation.toolchain.digest != self.toolchain_digest
            || request.compilation.compilation_id != self.compilation_id
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "C# request differs from its compiler authority",
            ));
        }
        let capability = CapabilityUri::parse(CSHARP_COMPILER_FACTS_CAPABILITY)?;
        let prepared = self
            .index
            .facts
            .iter()
            .map(|fact| {
                let bytes = canonical::bytes(fact).map_err(internal)?;
                let key = format!(
                    "csharp:{}:{}",
                    fact.query_family(),
                    canonical::hash_bytes(&bytes).trim_start_matches("sha256:")
                );
                Ok((key, bytes))
            })
            .collect::<Result<Vec<_>, ClewError>>()?;
        let payloads = self.store.put_batch(
            prepared
                .iter()
                .map(|(_, bytes)| (CSHARP_FACT_SCHEMA.into(), bytes.clone()))
                .collect(),
        )?;
        let mut records = prepared
            .into_iter()
            .zip(payloads)
            .map(|((fact_key, _), payload)| FactRecord {
                fact_key,
                domain_uri: capability.clone(),
                payload,
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.fact_key.cmp(&right.fact_key));
        for (sequence, chunk) in records.chunks(1024).enumerate() {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: u32::try_from(sequence)
                    .map_err(|_| resource("C# fact shard sequence overflow"))?,
                facts: chunk.to_vec(),
            }))?;
        }
        let scope_digest = csharp_scope_digest(&self.index)?;
        let boundary_count = self
            .index
            .facts
            .iter()
            .filter(|fact| fact.boundary_code().is_some())
            .count();
        let receipt = self.store.put(
            CSHARP_RECEIPT_SCHEMA,
            &canonical::bytes(&json!({
                "schema":CSHARP_RECEIPT_SCHEMA,
                "scopeDigest":scope_digest,
                "coverage":if boundary_count == 0 { "COMPLETE_SUPPORTED_SUBSET" } else { "PARTIAL" },
                "certainty":if boundary_count == 0 { "VERIFIED" } else { "UNSURE" },
                "boundaryCount":boundary_count,
                "obligations":if boundary_count == 0 { Vec::<String>::new() } else { vec!["REVIEW_CSHARP_ANALYSIS_BOUNDARIES".to_owned()] },
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
            return Err(invalid("C# attempt identity is invalid"));
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

fn validate_index(index: &CSharpCompilerIndex) -> Result<(), ClewError> {
    verify_model(&index.model)?;
    if index.schema != CSHARP_INDEX_SCHEMA
        || index.compilation != index.model.compilation
        || index.facts.len() > MAX_CSHARP_FACTS
    {
        return Err(corrupt("C# compiler index authority is invalid"));
    }
    let mut previous: Option<Vec<u8>> = None;
    for fact in &index.facts {
        if fact.schema() != CSHARP_FACT_SCHEMA
            || fact.path().is_some_and(|path| !safe_relative_path(path))
            || !fact.position_is_valid()
        {
            return Err(corrupt("C# compiler fact authority is invalid"));
        }
        let bytes = canonical::bytes(fact).map_err(internal)?;
        if bytes.len() > MAX_FACT_BYTES
            || previous.as_ref().is_some_and(|previous| previous >= &bytes)
        {
            return Err(corrupt("C# compiler facts are not canonical"));
        }
        previous = Some(bytes);
    }
    Ok(())
}

fn query_key_component(value: &str) -> String {
    let component = value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric() || *character == '_')
        .take(128)
        .collect::<String>();
    if component.is_empty() {
        "unknown".into()
    } else {
        component
    }
}

fn cancelled_error() -> ClewError {
    ClewError::new(
        ErrorCode::IncompleteSemanticAnalysis,
        "C# analysis was cancelled",
    )
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
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

fn poisoned<T>(error: std::sync::PoisonError<T>) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(name: &str, start: u64, end: u64) -> CSharpCompilerFact {
        serde_json::from_value(json!({
            "schema":CSHARP_FACT_SCHEMA,"kind":"DECLARATION","declarationKind":"METHOD","name":name,
            "symbolIdentity":format!("method:class:Orders.Api.OrdersController#{name}()V"),
            "ownerIdentity":"class:Orders.Api.OrdersController",
            "csharpIdentity":format!("csharp:M:Orders.Api.OrdersController.{name}"),
            "accessibility":"PUBLIC","modifiers":["PUBLIC"],"annotations":[],
            "project":"src/Orders.Api/Orders.Api.csproj","file":"src/Orders.Api/OrdersController.cs",
            "start":start,"end":end,"startLine":3,"endLine":5,"byteStart":start+3,"byteEnd":end+3,
            "resolution":"COMPILER_EXACT"
        }))
        .unwrap()
    }

    #[test]
    fn fact_contract_is_closed_and_keys_preserve_query_families() {
        let fact = declaration("Get", 10, 40);
        assert_eq!(fact.query_family(), "0-declaration:get");
        assert!(fact.position_is_valid());
        assert!(!declaration("Get", 40, 10).position_is_valid());
        let mut value = serde_json::to_value(&fact).unwrap();
        value["unexpected"] = json!(true);
        assert!(serde_json::from_value::<CSharpCompilerFact>(value).is_err());
        let relation: CSharpCompilerFact = serde_json::from_value(json!({
            "schema":CSHARP_FACT_SCHEMA,"kind":"RELATION","relationKind":"CALLS",
            "sourceIdentity":"method:class:A#a()V","targetIdentity":"method:class:B#b()V",
            "targetCsharpIdentity":"csharp:M:B.b","file":"A.cs","start":1,"end":2,
            "startLine":1,"endLine":1,"byteStart":1,"byteEnd":2,"resolution":"COMPILER_EXACT"
        }))
        .unwrap();
        assert_eq!(relation.query_family(), "1-relation:calls");
    }

    #[test]
    fn adapter_authority_binds_the_worker_distribution() {
        let first = csharp_adapter_digest(&format!("sha256:{}", "a".repeat(64))).unwrap();
        let second = csharp_adapter_digest(&format!("sha256:{}", "b".repeat(64))).unwrap();
        assert!(digest(&first));
        assert_ne!(first, second);
    }
}
