use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub const RUST_LANGUAGE: &str = "language:rust";
pub const RUST_SYNTAX_FACTS_CAPABILITY: &str = "analysis:rust-syntax-facts";
pub const RUST_INDEX_SCHEMA: &str = "codeclew-rust-syntax-index/1.0";
const FACT_SCHEMA: &str = "codeclew-rust-syntax-fact/1.0";
const RECEIPT_SCHEMA: &str = "codeclew-rust-syntax-completeness/1.0";
const ADAPTER_AUTHORITY_SCHEMA: &str = "codeclew-rust-syntax-adapter/1.0";

pub fn rust_adapter_digest() -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":ADAPTER_AUTHORITY_SCHEMA,
        "indexSchema":RUST_INDEX_SCHEMA,
        "factSchema":FACT_SCHEMA,
        "capability":RUST_SYNTAX_FACTS_CAPABILITY,
    }))
    .map_err(internal)
}

pub fn rust_scope_digest(index: &Value) -> Result<String, ClewError> {
    validate_index(index)?;
    canonical::hash(&json!({
        "schema":"codeclew-rust-syntax-scope/1.0",
        "compilation":index["compilation"],
        "modelDigest":index["modelDigest"],
        "files":index["files"],
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
    let mut rows = vec![json!({
        "schema":FACT_SCHEMA,
        "kind":"cargo-target",
        "package":index["package"],
        "targetKind":index["targetKind"],
        "targetName":index["targetName"],
        "sourcePath":index["sourcePath"],
        "cargoVersion":index["cargoVersion"],
        "rustcVersion":index["rustcVersion"],
        "resolution":"CARGO_MODEL_EXACT",
    })];
    for file in index["files"].as_array().expect("validated files") {
        rows.push(json!({
            "schema":FACT_SCHEMA,
            "kind":"source-file",
            "path":file["path"],
            "contentHash":file["contentHash"],
            "package":index["package"],
            "targetName":index["targetName"],
            "resolution":"SOURCE_MEMBERSHIP_EXACT",
        }));
    }
    let mut facts = rows
        .into_iter()
        .map(|payload| {
            let bytes = canonical::bytes(&payload).map_err(internal)?;
            let digest = canonical::hash_bytes(&bytes);
            let object = store.put(FACT_SCHEMA, &bytes)?;
            Ok(FactRecord {
                fact_key: format!("rust-syntax:{digest}"),
                domain_uri: capability.clone(),
                payload: object,
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    facts.sort_by(|left, right| left.fact_key.cmp(&right.fact_key));
    Ok(facts)
}

fn validate_index(index: &Value) -> Result<(), ClewError> {
    let files = index
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no file manifest"))?;
    if index.get("schema").and_then(Value::as_str) != Some(RUST_INDEX_SCHEMA)
        || index.get("analysisCoverage").and_then(Value::as_str) != Some("PARTIAL")
        || index.get("analysisCertainty").and_then(Value::as_str) != Some("UNSURE")
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
    use super::{RUST_INDEX_SCHEMA, translate_facts};
    use crate::cas::CasStore;
    use crate::state::StateAuthority;
    use serde_json::json;

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
        for fact in first {
            let lease = store.read(&fact.payload, 4096).unwrap();
            let text = std::str::from_utf8(lease.bytes()).unwrap();
            assert!(!text.contains("/Users/"));
            assert!(!text.contains("file://"));
        }
    }
}
