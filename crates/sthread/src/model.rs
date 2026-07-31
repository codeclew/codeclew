use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompletenessStatus {
    CompleteSupportedSubset,
    PartialBudget,
    PartialUnsupportedFeature,
    PartialExternalBoundary,
    PartialDynamicDispatch,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlicePolicy {
    pub direction: Direction,
    pub include_edges: Vec<String>,
    pub max_nodes: usize,
    pub max_files: usize,
    pub max_call_depth: usize,
    pub max_dispatch_targets: usize,
    pub deadline_ms: u64,
}

impl Default for SlicePolicy {
    fn default() -> Self {
        Self {
            direction: Direction::Both,
            include_edges: [
                "DEF_USE",
                "CONTROL_DEP",
                "CALL",
                "RETURN",
                "ARG_PARAM",
                "RECEIVER",
                "CAPTURE",
                "READ_STATE",
                "WRITE_STATE",
            ]
            .map(str::to_owned)
            .to_vec(),
            max_nodes: 200,
            max_files: 20,
            max_call_depth: 0,
            max_dispatch_targets: 8,
            deadline_ms: 2000,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Direction {
    Forward,
    Backward,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defines: Option<String>,
    #[serde(default)]
    pub uses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Value>,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalGraph {
    pub schema: String,
    pub symbol: String,
    pub file: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub base_revision: String,
    pub project_model_hash: String,
    pub compiler_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Completeness {
    pub status: CompletenessStatus,
    pub boundaries: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadIr {
    pub schema: String,
    pub thread_id: String,
    pub snapshot: Snapshot,
    pub seed: Value,
    pub policy: SlicePolicy,
    pub completeness: Completeness,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
    pub editable_units: Vec<Value>,
    pub external_summaries: Vec<Value>,
    pub read_set: Vec<ReadFact>,
    pub validation_plan: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct ReadFact {
    pub kind: String,
    pub key: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditIr {
    pub schema: String,
    pub thread_id: String,
    pub base_revision: String,
    pub operations: Vec<EditOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOperation {
    pub op_id: String,
    pub kind: String,
    pub target: Value,
    pub replacement: Replacement,
    #[serde(default)]
    pub preconditions: BTreeMap<String, Value>,
    #[serde(default)]
    pub postconditions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replacement {
    pub kotlin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewReport {
    pub schema: String,
    pub transaction_id: String,
    pub base_revision: String,
    pub valid: bool,
    pub changed_files: Vec<String>,
    pub diff: String,
    pub candidates: BTreeMap<String, String>,
    pub actual_write_set: Vec<WriteFact>,
    pub diagnostics: Vec<Value>,
    pub formatting_windows: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct WriteFact {
    pub kind: String,
    pub key: String,
    pub before_hash: String,
    pub after_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub schema: String,
    pub tx_id: String,
    pub actor_id: String,
    pub intent: String,
    pub base_revision: String,
    pub project_model_hash: String,
    pub status: String,
    pub thread: ThreadIr,
    pub edit: EditIr,
    #[serde(default)]
    pub preview: Option<PreviewReport>,
    #[serde(default)]
    pub test_tasks: Vec<String>,
    #[serde(default)]
    pub candidate_commit: Option<String>,
    #[serde(default)]
    pub final_commit: Option<String>,
}
