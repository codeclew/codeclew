//! Closed normalization contract for compiler-produced local control-flow graphs.
//!
//! Raw FIR node class names are intentionally excluded. A version-specific
//! worker maps compiler internals to these stable roles and transition kinds.

use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const LOCAL_CFG_SCHEMA: &str = "local-cfg/0.1";
pub const LOCAL_CFG_BOUNDARY_SCHEMA: &str = "local-cfg-boundary/0.1";
pub const LOCAL_CFG_PAYLOAD_SCHEMA: &str = "codeclew-kotlin-semantic-fact/3.0";
pub const MAX_LOCAL_CFG_NODES: usize = 4_096;
pub const MAX_LOCAL_CFG_EDGES: usize = 8_192;
const MAX_CFG_TEXT_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LocalCfgNodeRole {
    Entry,
    Exit,
    Operation,
    Decision,
    Merge,
    Return,
    Throw,
    Catch,
    Finally,
    LoopCondition,
    LoopExit,
    Dead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LocalCfgEdgeKind {
    Next,
    True,
    False,
    WhenCase,
    Exception,
    Return,
    LoopBack,
    Break,
    Continue,
    Finally,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalCfgSourceRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalCfgNode {
    pub node_id: u64,
    pub role: LocalCfgNodeRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<LocalCfgSourceRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalCfgEdge {
    pub source_node_id: u64,
    pub target_node_id: u64,
    pub kind: LocalCfgEdgeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalCfgPayload {
    pub schema: String,
    pub graph_id: String,
    pub owner_symbol_identity: String,
    pub file: String,
    pub compiler_graph_name: String,
    pub provider: String,
    pub source_provenance: String,
    pub nodes: Vec<LocalCfgNode>,
    pub edges: Vec<LocalCfgEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalCfgSupport {
    pub member_alias: String,
    pub compilation_id: String,
    pub generation_ref: CasObject,
    pub payload_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedLocalCfg {
    pub payload: LocalCfgPayload,
    pub support: LocalCfgSupport,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalCfgCatalog {
    graphs: BTreeMap<(String, String), PreparedLocalCfg>,
}

impl LocalCfgCatalog {
    pub fn insert(&mut self, graph: PreparedLocalCfg) -> Result<(), ClewError> {
        validate(&graph.payload)?;
        if graph.support.payload_ref.object_schema != LOCAL_CFG_PAYLOAD_SCHEMA {
            return Err(corrupt("local CFG payload has another CAS schema"));
        }
        let key = (
            graph.support.member_alias.clone(),
            graph.payload.owner_symbol_identity.clone(),
        );
        if self.graphs.insert(key, graph).is_some() {
            return Err(corrupt("local CFG owner is duplicated in one member"));
        }
        Ok(())
    }

    pub fn get(
        &self,
        member_alias: &str,
        owner_symbol_identity: &str,
    ) -> Option<&PreparedLocalCfg> {
        self.graphs
            .get(&(member_alias.to_owned(), owner_symbol_identity.to_owned()))
    }

    pub fn is_empty(&self) -> bool {
        self.graphs.is_empty()
    }
}

pub fn validate(payload: &LocalCfgPayload) -> Result<(), ClewError> {
    if payload.schema != LOCAL_CFG_SCHEMA
        || payload.provider != "K2_FIR_CFG"
        || payload.source_provenance != "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
    {
        return Err(invalid("local CFG authority fields are invalid"));
    }
    crate::semantic_validation::validate_kotlin_full_symbol_identity(
        &payload.owner_symbol_identity,
    )?;
    validate_text(&payload.file, "local CFG file", 4_096)?;
    if payload.file.starts_with('/')
        || payload
            .file
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid("local CFG file is not repository-relative"));
    }
    validate_text(
        &payload.compiler_graph_name,
        "local CFG compiler graph name",
        MAX_CFG_TEXT_BYTES,
    )?;
    if payload.nodes.is_empty()
        || payload.nodes.len() > MAX_LOCAL_CFG_NODES
        || payload.edges.len() > MAX_LOCAL_CFG_EDGES
    {
        return Err(budget("local CFG exceeds its node or edge budget"));
    }
    let mut node_ids = BTreeSet::new();
    let mut previous = None;
    let mut entry_count = 0usize;
    let mut terminal_count = 0usize;
    for node in &payload.nodes {
        if previous.is_some_and(|value| value >= node.node_id) || !node_ids.insert(node.node_id) {
            return Err(invalid("local CFG nodes are not canonical and unique"));
        }
        previous = Some(node.node_id);
        entry_count += usize::from(node.role == LocalCfgNodeRole::Entry);
        terminal_count += usize::from(matches!(
            node.role,
            LocalCfgNodeRole::Exit | LocalCfgNodeRole::Return | LocalCfgNodeRole::Throw
        ));
        if let Some(source) = &node.source
            && source.end <= source.start
        {
            return Err(invalid("local CFG node source range is invalid"));
        }
    }
    if entry_count != 1 || terminal_count == 0 {
        return Err(invalid("local CFG must have one entry and a terminal node"));
    }
    let mut previous_edge = None::<LocalCfgEdge>;
    let mut adjacency = BTreeMap::<u64, Vec<u64>>::new();
    for edge in &payload.edges {
        if !node_ids.contains(&edge.source_node_id) || !node_ids.contains(&edge.target_node_id) {
            return Err(invalid("local CFG edge has a dangling endpoint"));
        }
        if previous_edge.as_ref().is_some_and(|value| value >= edge) {
            return Err(invalid("local CFG edges are not canonical and unique"));
        }
        if let Some(label) = &edge.label {
            validate_text(label, "local CFG edge label", MAX_CFG_TEXT_BYTES)?;
        }
        previous_edge = Some(edge.clone());
        adjacency
            .entry(edge.source_node_id)
            .or_default()
            .push(edge.target_node_id);
    }
    let entry = payload
        .nodes
        .iter()
        .find(|node| node.role == LocalCfgNodeRole::Entry)
        .expect("validated entry");
    let mut reachable = BTreeSet::from([entry.node_id]);
    let mut queue = VecDeque::from([entry.node_id]);
    while let Some(node) = queue.pop_front() {
        for target in adjacency.get(&node).into_iter().flatten() {
            if reachable.insert(*target) {
                queue.push_back(*target);
            }
        }
    }
    if payload
        .nodes
        .iter()
        .any(|node| node.role != LocalCfgNodeRole::Dead && !reachable.contains(&node.node_id))
    {
        return Err(invalid("local CFG has an unreachable non-dead node"));
    }
    let mut unsigned = payload.clone();
    unsigned.graph_id.clear();
    if payload.graph_id != canonical::hash(&unsigned).map_err(internal)? {
        return Err(invalid("local CFG graph identity is invalid"));
    }
    Ok(())
}

/// Validate a typed UNKNOWN boundary emitted either by the pinned K2 worker or
/// by the Rust admission normalizer. Boundaries deliberately carry no graph
/// topology and therefore cannot be mistaken for executable CFG evidence.
pub(crate) fn validate_boundary(value: &Value) -> Result<(), ClewError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("local CFG boundary is not an object"))?;
    let allowed = BTreeSet::from([
        "schema",
        "file",
        "ownerSymbolIdentity",
        "compilerGraphName",
        "stage",
        "code",
        "resolution",
        "provider",
        "sourceProvenance",
        "rawRowHash",
    ]);
    if object.keys().any(|key| !allowed.contains(key.as_str())) {
        return Err(invalid(
            "local CFG boundary has a field outside its exact contract",
        ));
    }
    let provider = value.get("provider").and_then(Value::as_str);
    let code = value.get("code").and_then(Value::as_str);
    let worker_code = matches!(
        code,
        Some(
            "INVALID_LOCAL_CFG_IDENTITY"
                | "UNSUPPORTED_LOCAL_CFG_NODE"
                | "UNSUPPORTED_LOCAL_CFG_EDGE"
                | "LOCAL_CFG_BUDGET_EXCEEDED"
                | "INVALID_LOCAL_CFG_TOPOLOGY"
                | "NO_SOURCE_FUNCTION"
                | "LOCAL_CFG_NORMALIZATION_FAILED"
                | "DUPLICATE_LOCAL_CFG_OWNER"
        )
    );
    let normalizer_code = matches!(code, Some("INVALID_LOCAL_CFG" | "UNPROVEN_LOCAL_CFG_OWNER"));
    if value.get("schema").and_then(Value::as_str) != Some(LOCAL_CFG_BOUNDARY_SCHEMA)
        || value.get("stage").and_then(Value::as_str) != Some("NORMALIZE")
        || value.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
        || value.get("sourceProvenance").and_then(Value::as_str)
            != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
        || !matches!(
            (provider, worker_code, normalizer_code),
            (Some("K2_FIR_CFG"), true, false)
                | (Some("CODECLEW_LOCAL_CFG_NORMALIZER"), false, true)
        )
    {
        return Err(invalid("malformed local CFG UNKNOWN boundary"));
    }
    let raw_hash = value
        .get("rawRowHash")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("local CFG boundary has no raw row hash"))?;
    if !canonical_sha256(raw_hash) {
        return Err(invalid("local CFG boundary raw row hash is invalid"));
    }
    if let Some(file) = value.get("file").and_then(Value::as_str) {
        validate_text(file, "local CFG boundary file", 4_096)?;
        if file.starts_with('/')
            || file.contains('\\')
            || file
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(invalid(
                "local CFG boundary file is not repository-relative",
            ));
        }
    } else if value.get("file").is_some() {
        return Err(invalid("local CFG boundary file is malformed"));
    }
    if let Some(owner) = value.get("ownerSymbolIdentity").and_then(Value::as_str) {
        crate::semantic_validation::validate_kotlin_full_symbol_identity(owner)?;
    } else if value.get("ownerSymbolIdentity").is_some() {
        return Err(invalid("local CFG boundary owner is malformed"));
    }
    if let Some(name) = value.get("compilerGraphName").and_then(Value::as_str) {
        validate_text(
            name,
            "local CFG boundary compiler graph name",
            MAX_CFG_TEXT_BYTES,
        )?;
    } else if value.get("compilerGraphName").is_some() {
        return Err(invalid(
            "local CFG boundary compiler graph name is malformed",
        ));
    }
    Ok(())
}

fn canonical_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

pub fn seal(mut payload: LocalCfgPayload) -> Result<LocalCfgPayload, ClewError> {
    payload.graph_id.clear();
    payload.graph_id = canonical::hash(&payload).map_err(internal)?;
    validate(&payload)?;
    Ok(payload)
}

fn validate_text(value: &str, label: &str, max_bytes: usize) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
        || unicode_normalization::UnicodeNormalization::nfc(value.chars()).collect::<String>()
            != value
    {
        return Err(invalid(format!(
            "{label} is empty, oversized, non-NFC, or unsafe"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn budget(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
