//! Inert, source-bound content shared by the static HTML and MDX publishers.
use crate::documentation::model::{Observation, Source};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "codeclew-native-page-projection/1.0";

/// Inputs select retained declarations. They never supply conclusions or labels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub id: String,
    pub service: String,
    pub endpoint_declaration: String,
    pub worker_declaration: String,
    #[serde(default)]
    pub wiring_declaration: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BundleProjection {
    pub schema: String,
    pub input_digest: String,
    pub context_digest: String,
    pub selection_digest: String,
    pub pages: Vec<PageContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
/// One selected process aggregate; publishers derive linked views from it.
pub struct PageContent {
    pub id: String,
    pub title: String,
    pub selection: Selection,
    pub service_revision: String,
    pub service_digest: String,
    pub endpoint: CallableProjection,
    pub worker: CallableProjection,
    pub wiring: Option<CallableProjection>,
    pub handoff: HandoffProjection,
    pub diagnostics: Vec<Diagnostic>,
    pub citations: BTreeMap<String, Citation>,
    /// Full retained evidence supporting this selection; overview limits must
    /// never truncate these records or the recursive statement projection.
    pub observations: BTreeMap<String, Observation>,
    pub sources: BTreeMap<String, Source>,
    pub limitations: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub id: String,
    pub source_id: String,
    pub service: String,
    pub revision: String,
    pub file: String,
    pub start_line: u64,
    pub end_line: u64,
    /// Byte range relative to the retained Source.text, end exclusive.
    pub start_byte: usize,
    pub end_byte: usize,
    pub text_digest: String,
    pub evidence_digest: String,
    pub authority: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    pub code: String,
    pub detail: String,
    pub citation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallableProjection {
    pub declaration_id: String,
    pub symbol: String,
    pub authority: String,
    pub citation_id: Option<String>,
    pub steps: Vec<Statement>,
    pub state: Vec<StateRow>,
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatementKind {
    Declaration,
    Assignment,
    Expression,
    If,
    Return,
    Throw,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PathCondition {
    /// Exact source expression, or an explicitly unresolved continuation
    /// restriction accompanied by a local Gap. `holds=false` means the false
    /// alternative; negated source expressions are deliberately not rewritten.
    pub expression: String,
    pub holds: bool,
    pub citation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Statement {
    pub id: String,
    pub kind: StatementKind,
    pub expression: String,
    pub citation_id: String,
    pub conditions: Vec<PathCondition>,
    pub reachable: bool,
    pub children: Vec<Statement>,
    pub alternative: Vec<Statement>,
    pub calls: Vec<CallProjection>,
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StateRow {
    pub name: String,
    pub expression: String,
    pub kind: String,
    pub conditions: Vec<PathCondition>,
    pub citation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallProjection {
    pub expression: String,
    pub receiver: Option<String>,
    pub name: String,
    pub arguments: Vec<String>,
    pub target: Option<String>,
    pub authority: String,
    pub relation_id: Option<String>,
    pub citation_id: String,
    /// Phase names describe source syntax only; none imply runtime success.
    pub phase: String,
    pub external_boundary: Option<ExternalBoundary>,
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalBoundary {
    pub target: String,
    pub dependency_id: String,
    pub source_status: String,
    pub citation_ids: Vec<String>,
    pub limitation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HandoffProjection {
    pub status: String,
    pub queue_allocation: Option<String>,
    pub endpoint_field: Option<String>,
    pub worker_field: Option<String>,
    pub citation_ids: Vec<String>,
    pub gaps: Vec<Gap>,
    pub limitation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub condition: String,
    pub possible_reason: String,
    pub inspect: Vec<String>,
    pub selected_call: String,
    pub citation_ids: Vec<String>,
}
