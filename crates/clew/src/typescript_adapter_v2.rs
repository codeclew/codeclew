use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::typescript_project_model::{
    TypeScriptOperationalModel, TypeScriptProjectModel, analyzer_digest, verify_model,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub const TYPESCRIPT_LANGUAGE: &str = "language:typescript";
pub const TYPESCRIPT_COMPILER_FACTS_CAPABILITY: &str = "analysis:typescript-compiler-facts";
pub const TYPESCRIPT_INDEX_SCHEMA: &str = "codeclew-typescript-compiler-index/1.0";
pub const TYPESCRIPT_FACT_SCHEMA: &str = "codeclew-typescript-compiler-fact/1.0";
const TYPESCRIPT_RECEIPT_SCHEMA: &str = "codeclew-typescript-compiler-completeness/1.0";
const TYPESCRIPT_ADAPTER_AUTHORITY_SCHEMA: &str = "codeclew-typescript-compiler-adapter/1.0";
const MAX_TYPESCRIPT_FACTS: usize = 262_144;
const MAX_FACT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TypeScriptCompilerFact {
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
        exported: bool,
        type_text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        file: String,
        start: u64,
        end: u64,
        resolution: String,
    },
    Relation {
        schema: String,
        relation_kind: String,
        source_identity: String,
        target_identity: String,
        file: String,
        start: u64,
        end: u64,
        resolution: String,
    },
    Boundary {
        schema: String,
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostic_code: Option<String>,
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

impl TypeScriptCompilerFact {
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
            Self::Boundary { code, .. } => {
                format!("2-boundary:{}", query_key_component(code))
            }
            Self::SourceFile { .. } => "3-source-file".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptCompilerIndex {
    pub schema: String,
    pub compilation: String,
    pub model: TypeScriptProjectModel,
    pub analyzer_digest: String,
    pub facts: Vec<TypeScriptCompilerFact>,
}

pub fn typescript_adapter_digest() -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":TYPESCRIPT_ADAPTER_AUTHORITY_SCHEMA,
        "indexSchema":TYPESCRIPT_INDEX_SCHEMA,
        "factSchema":TYPESCRIPT_FACT_SCHEMA,
        "capability":TYPESCRIPT_COMPILER_FACTS_CAPABILITY,
        "analyzerDigest":analyzer_digest(),
        "compilerApi":"typescript-5.x",
    }))
    .map_err(internal)
}

pub fn build_typescript_compiler_index(
    operational: TypeScriptOperationalModel,
    source_content_digests: &BTreeMap<String, String>,
) -> Result<TypeScriptCompilerIndex, ClewError> {
    verify_model(&operational.authority)?;
    let mut facts = operational.facts;
    for (path, digest) in source_content_digests {
        facts.push(TypeScriptCompilerFact::SourceFile {
            schema: TYPESCRIPT_FACT_SCHEMA.into(),
            file: path.clone(),
            source_content_digest: digest.clone(),
            resolution: "SOURCE_MEMBERSHIP_EXACT".into(),
        });
    }
    facts.sort_by_cached_key(|fact| {
        canonical::bytes(fact).expect("serializable TypeScript compiler fact")
    });
    facts.dedup();
    let index = TypeScriptCompilerIndex {
        schema: TYPESCRIPT_INDEX_SCHEMA.into(),
        compilation: operational.authority.compilation.clone(),
        model: operational.authority,
        analyzer_digest: analyzer_digest(),
        facts,
    };
    validate_index(&index)?;
    Ok(index)
}

pub fn typescript_scope_digest(index: &TypeScriptCompilerIndex) -> Result<String, ClewError> {
    validate_index(index)?;
    canonical::hash(&json!({
        "schema":"codeclew-typescript-compiler-scope/1.0",
        "compilation":index.compilation,
        "modelDigest":index.model.model_digest,
        "analyzerDigest":index.analyzer_digest,
        "factsDigest":canonical::hash(&index.facts).map_err(internal)?,
        "factCount":index.facts.len(),
    }))
    .map_err(internal)
}

pub struct TypeScriptAdapterV2 {
    adapter_digest: String,
    toolchain_digest: String,
    compilation_id: String,
    store: CasStore,
    index: TypeScriptCompilerIndex,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl TypeScriptAdapterV2 {
    pub fn new(
        adapter_digest: String,
        toolchain_digest: String,
        compilation_id: String,
        store: CasStore,
        index: TypeScriptCompilerIndex,
    ) -> Result<Self, ClewError> {
        validate_index(&index)?;
        if !digest(&adapter_digest)
            || !digest(&toolchain_digest)
            || compilation_id.is_empty()
            || compilation_id.len() > 120
        {
            return Err(invalid("TypeScript adapter authority is invalid"));
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

impl LanguageAdapter for TypeScriptAdapterV2 {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
        Ok(AdapterHandshake {
            protocol: ADAPTER_PROTOCOL.into(),
            adapter_id: "typescript-compiler-1".into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(TYPESCRIPT_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(TYPESCRIPT_COMPILER_FACTS_CAPABILITY)?],
            toolchains: vec![ToolchainConstraint {
                authority_digest: self.toolchain_digest.clone(),
                minimum_version: Some("5.0".into()),
                maximum_version_exclusive: Some("6.0".into()),
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
        if request.compilation.language_uri.as_str() != TYPESCRIPT_LANGUAGE
            || request.capability.as_str() != TYPESCRIPT_COMPILER_FACTS_CAPABILITY
            || request.compilation.toolchain.digest != self.toolchain_digest
            || request.compilation.compilation_id != self.compilation_id
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "TypeScript request differs from its compiler authority",
            ));
        }
        let capability = CapabilityUri::parse(TYPESCRIPT_COMPILER_FACTS_CAPABILITY)?;
        let prepared = self
            .index
            .facts
            .iter()
            .map(|fact| {
                let bytes = canonical::bytes(fact).map_err(internal)?;
                let key = format!(
                    "typescript:{}:{}",
                    fact.query_family(),
                    canonical::hash_bytes(&bytes).trim_start_matches("sha256:")
                );
                Ok((key, bytes))
            })
            .collect::<Result<Vec<_>, ClewError>>()?;
        let payloads = self.store.put_batch(
            prepared
                .iter()
                .map(|(_, bytes)| (TYPESCRIPT_FACT_SCHEMA.into(), bytes.clone()))
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
                    .map_err(|_| resource("TypeScript fact shard sequence overflow"))?,
                facts: chunk.to_vec(),
            }))?;
        }
        let scope_digest = typescript_scope_digest(&self.index)?;
        let boundary_count = self
            .index
            .facts
            .iter()
            .filter(|fact| fact.boundary_code().is_some())
            .count();
        let receipt = self.store.put(
            TYPESCRIPT_RECEIPT_SCHEMA,
            &canonical::bytes(&json!({
                "schema":TYPESCRIPT_RECEIPT_SCHEMA,
                "scopeDigest":scope_digest,
                "coverage":if boundary_count == 0 { "COMPLETE_SUPPORTED_SUBSET" } else { "PARTIAL" },
                "certainty":if boundary_count == 0 { "VERIFIED" } else { "UNSURE" },
                "boundaryCount":boundary_count,
                "obligations":if boundary_count == 0 { Vec::<String>::new() } else { vec!["FIX_TYPESCRIPT_CONFIGURATION_OR_DEPENDENCY".to_owned()] },
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
            return Err(invalid("TypeScript attempt identity is invalid"));
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

fn validate_index(index: &TypeScriptCompilerIndex) -> Result<(), ClewError> {
    verify_model(&index.model)?;
    if index.schema != TYPESCRIPT_INDEX_SCHEMA
        || index.compilation != index.model.compilation
        || index.analyzer_digest != analyzer_digest()
        || index.facts.len() > MAX_TYPESCRIPT_FACTS
    {
        return Err(corrupt("TypeScript compiler index authority is invalid"));
    }
    let mut previous = None;
    for fact in &index.facts {
        if fact.schema() != TYPESCRIPT_FACT_SCHEMA
            || fact.path().is_some_and(|path| !safe_relative_path(path))
        {
            return Err(corrupt("TypeScript compiler fact authority is invalid"));
        }
        let bytes = canonical::bytes(fact).map_err(internal)?;
        if bytes.len() > MAX_FACT_BYTES
            || previous.as_ref().is_some_and(|previous| previous >= &bytes)
        {
            return Err(corrupt("TypeScript compiler facts are not canonical"));
        }
        previous = Some(bytes);
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
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

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn cancelled_error() -> ClewError {
    ClewError::new(
        ErrorCode::IncompleteSemanticAnalysis,
        "TypeScript analysis was cancelled",
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

    #[test]
    fn analyzer_and_adapter_authority_are_content_bound() {
        assert!(digest(&typescript_adapter_digest().unwrap()));
        assert!(digest(&analyzer_digest()));
    }

    #[test]
    fn fact_keys_preserve_query_families_for_bounded_selection() {
        let declaration = TypeScriptCompilerFact::Declaration {
            schema: TYPESCRIPT_FACT_SCHEMA.into(),
            declaration_kind: "FUNCTION".into(),
            name: "usePersistentState".into(),
            symbol_identity: "ts:src/hooks.ts#function:usePersistentState@0-10".into(),
            owner_identity: "module:src/hooks.ts".into(),
            exported: true,
            type_text: "() => void".into(),
            signature: Some("(): void".into()),
            file: "src/hooks.ts".into(),
            start: 0,
            end: 10,
            resolution: "COMPILER_RESOLVED".into(),
        };
        let relation = TypeScriptCompilerFact::Relation {
            schema: TYPESCRIPT_FACT_SCHEMA.into(),
            relation_kind: "CALLS".into(),
            source_identity: "ts:src/page.ts#function:render@0-10".into(),
            target_identity: "ts:src/hooks.ts#function:usePersistentState@0-10".into(),
            file: "src/page.ts".into(),
            start: 1,
            end: 2,
            resolution: "COMPILER_RESOLVED".into(),
        };

        assert_eq!(
            declaration.query_family(),
            "0-declaration:usepersistentstate"
        );
        assert_eq!(relation.query_family(), "1-relation:calls");
    }
}
