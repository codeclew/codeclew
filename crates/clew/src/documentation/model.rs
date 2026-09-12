//! Closed, portable authored records. No session IDs or machine paths are identities.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const VERSION: &str = "1.0";
pub const EXTRACTOR: &str = "codeclew-documentation-jvm/1.1";
pub const RENDERER: &str = "codeclew-documentation-html/1.2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Service {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub repository_id: String,
    pub repository: String,
    pub language: String,
    pub profile: String,
    pub compilation: String,
    pub target_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_link_template: Option<String>,
    #[serde(default)]
    pub contract_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selector {
    pub language: String,
    pub owner: String,
    pub name: String,
    /// None means overload resolution is still required; [] selects zero arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_types: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Endpoint {
    pub service: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<Selector>,
    /// Exact resolved target plus optional zero-based ordinal among those calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_site: Option<CallSite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallSite {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transport {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_config_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declaration {
    pub origin: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Applicability {
    pub environments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Interaction {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub from: Endpoint,
    pub to: Endpoint,
    pub transport: Transport,
    pub declaration: Declaration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applicability: Option<Applicability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_reference: Option<String>,
}

fn depth() -> usize {
    4
}
fn nodes() -> usize {
    64
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scenario {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub root: Endpoint,
    #[serde(default)]
    pub interactions: Vec<String>,
    #[serde(default = "depth")]
    pub max_depth: usize,
    #[serde(default = "nodes")]
    pub max_nodes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBinding {
    pub schema: String,
    pub service: String,
    pub repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub service: String,
    pub revision: String,
    pub file: String,
    pub start_line: u64,
    pub end_line: u64,
    pub text: String,
    pub text_digest: String,
    pub evidence_digest: String,
    pub authority: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub id: String,
    pub kind: String,
    pub service: String,
    pub symbol: String,
    pub normalized: Value,
    pub digest: String,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entrypoint {
    pub id: String,
    pub service: String,
    pub symbol: String,
    pub kind: String,
    pub trigger: Value,
    pub source_ids: Vec<String>,
    pub dependency_ids: Vec<String>,
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceEvidence {
    pub schema: String,
    pub service: String,
    pub revision: String,
    pub service_digest: String,
    pub extractor: String,
    pub runtime_mode: String,
    pub coverage: String,
    pub boundaries: Vec<String>,
    pub entrypoints: Vec<Entrypoint>,
    pub observations: BTreeMap<String, Observation>,
    pub sources: BTreeMap<String, Source>,
    pub contracts: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Participant {
    pub id: String,
    pub label: String,
    pub service: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub id: String,
    /// message, return, note, alt, else, loop, opt, end, or declared.
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub dependency_ids: Vec<String>,
    pub source_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fragment {
    pub id: String,
    pub text: String,
    pub dependency_ids: Vec<String>,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Explanation {
    pub id: String,
    pub text: String,
    pub event_ids: Vec<String>,
    pub dependency_ids: Vec<String>,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Operation {
    pub id: String,
    pub title: String,
    pub summary: Fragment,
    /// Domain explanation tied to diagram steps and their evidence.
    #[serde(default)]
    pub explanation: Vec<Explanation>,
    pub participants: Vec<Participant>,
    pub events: Vec<Event>,
    #[serde(default)]
    pub findings: Vec<Fragment>,
    #[serde(default)]
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Narrative {
    pub schema: String,
    /// service:<id> or scenario:<id>.
    pub subject: String,
    pub context_digest: String,
    pub operations: Vec<Operation>,
    /// Explicit entrypoint-ID to actionable gap; omitted endpoints are rejected.
    #[serde(default)]
    pub gaps: BTreeMap<String, String>,
}
