use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ADAPTER_SCHEMA: &str = "codeclew.adapter-output/0.1";
pub const IMPACT_SCHEMA: &str = "codeclew.impact-result/0.1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdapterOutput {
    pub schema: String,
    pub adapter: AdapterIdentity,
    pub snapshot_input: SnapshotInput,
    pub capability_descriptors: Vec<CapabilityDescriptor>,
    pub entities: Vec<Entity>,
    pub occurrences: Vec<Occurrence>,
    pub facts: Vec<Fact>,
    pub boundaries: Vec<Boundary>,
    pub compiler_receipt: CompilerReceipt,
    pub impact: ImpactOutput,
    pub cost: CostRecord,
    pub output_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdapterIdentity {
    pub adapter_id: String,
    pub version: String,
    pub binary_digest: String,
    pub language_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInput {
    pub repository_tree_digest: String,
    pub vcs_revision: Option<String>,
    pub dirty: bool,
    pub sources: Vec<SourceArtifact>,
    pub build_system_uri: String,
    pub build_model_digest: String,
    pub build_configuration_digest: String,
    pub dependency_graph_digest: String,
    pub toolchain: ToolchainInput,
    pub targets: Vec<TargetDescriptor>,
    pub relevant_environment: Vec<EnvironmentInput>,
    pub generated_sources_manifest_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceArtifact {
    pub artifact_id: String,
    pub normalized_path: String,
    pub content_digest: String,
    pub size_bytes: u64,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainInput {
    pub tool_uri: String,
    pub version: String,
    pub distribution_digest: String,
    pub provider_payload: ToolchainProviderPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainProviderPayload {
    pub language: String,
    pub tools: Vec<ToolIdentity>,
    pub rustc_sysroot_bytes_hashed: u64,
    pub generated_sources_completeness: String,
    pub repository_tree_exclusions: Vec<String>,
    pub scip_artifact_digest: String,
    pub inherited_environment_digest: String,
    pub telemetry_limitations: Vec<String>,
    pub provider_invocations: Vec<StableInvocationReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolIdentity {
    pub tool_id: String,
    pub requested_path: String,
    pub canonical_path: String,
    pub expected_binary_digest: String,
    pub observed_binary_digest: String,
    pub version: String,
    pub version_output_digest: String,
    pub distribution_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TargetDescriptor {
    pub target_id: String,
    pub configuration_digest: String,
    pub enabled_features: Vec<String>,
    pub platform: String,
    pub compiler_flags: Vec<String>,
    pub provider_payload: TargetProviderPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TargetProviderPayload {
    pub cargo_targets: Vec<CargoTarget>,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CargoTarget {
    pub package_id: String,
    pub package_name: String,
    pub target_name: String,
    pub kinds: Vec<String>,
    pub crate_types: Vec<String>,
    pub source_path: String,
    pub edition: String,
    pub required_features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInput {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    pub operation_uri: String,
    pub language_id: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub grade: String,
    pub support: String,
    pub guaranteed_enumeration: String,
    pub operation_version: String,
    pub operation_specification_digest: String,
    pub toolchain_digest: String,
    pub build_configuration_digest: String,
    pub target_digest: String,
    pub approximation: String,
    pub known_boundary_kinds: Vec<String>,
    pub cost_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub adapter_namespace: String,
    pub opaque_id: String,
    pub resolution: String,
    pub coarse_kind: String,
    pub display_name: Option<String>,
    pub primary_definition: Option<EvidenceRange>,
    pub language_payload: BTreeMap<String, String>,
    #[serde(skip_serializing)]
    pub native_identity: String,
    #[serde(skip_serializing)]
    pub document_scope: Option<String>,
    #[serde(skip_serializing)]
    pub definition_locations: Vec<Location>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub occurrence_id: String,
    pub role: String,
    pub origin: String,
    pub grade: String,
    pub entity_id: String,
    pub range: EvidenceRange,
    #[serde(skip_serializing)]
    pub source_location: Location,
    #[serde(skip_serializing)]
    pub native_identity: String,
    #[serde(skip_serializing)]
    pub position_encoding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub path: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRange {
    pub artifact_id: String,
    pub artifact_content_digest: String,
    pub start_byte: u64,
    pub end_byte: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub fact_id: String,
    pub relation: String,
    pub owner: String,
    pub target: String,
    pub truth: String,
    pub grade: String,
    pub enumeration: String,
    pub range: Option<EvidenceRange>,
    pub provider_payload: FactProviderPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactProviderPayload {
    pub operation_version: String,
    pub approximation: String,
    pub relation_kinds: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub source_location: Option<Location>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Boundary {
    pub boundary_id: String,
    pub kind_uri: String,
    pub consequence: String,
    pub origin: String,
    pub provider: String,
    pub details: BoundaryDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryDetails {
    pub status: String,
    pub category: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompilerReceipt {
    pub schema: String,
    pub method: String,
    pub status: String,
    pub grade: String,
    pub snapshot_tree_digest: String,
    pub claim: String,
    pub provider_payload: CompilerReceiptProviderPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompilerReceiptProviderPayload {
    pub configured_scope: Vec<String>,
    pub invocation: StableInvocationReceipt,
    pub source_tree_digest_before: String,
    pub source_tree_digest_after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InvocationReceipt {
    pub tool_id: String,
    pub executable_digest: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub environment_digest: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout_digest: String,
    pub stdout_bytes: u64,
    pub stderr_digest: String,
    pub stderr_bytes: u64,
    pub wall_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StableInvocationReceipt {
    pub tool_id: String,
    pub executable_digest: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub environment_digest: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub diagnostics_digest: String,
    pub diagnostic_count: u64,
}

impl StableInvocationReceipt {
    pub fn from_invocation(
        value: &InvocationReceipt,
        diagnostics_digest: String,
        diagnostic_count: u64,
    ) -> Self {
        Self {
            tool_id: value.tool_id.clone(),
            executable_digest: value.executable_digest.clone(),
            argv: value.argv.clone(),
            working_directory: value.working_directory.clone(),
            environment_digest: value.environment_digest.clone(),
            exit_code: value.exit_code,
            success: value.success,
            diagnostics_digest,
            diagnostic_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImpactOutput {
    pub schema: String,
    pub status: String,
    pub reason: String,
    pub closure_specification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_entity: Option<String>,
    pub max_depth: u32,
    pub max_entities: u32,
    pub affected: Vec<ImpactAffected>,
    pub paths: Vec<ImpactPath>,
    pub mandatory_obligations: Vec<MandatoryObligation>,
    pub boundaries: Vec<Boundary>,
    pub query_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImpactAffected {
    pub entity_id: String,
    pub impact_class: String,
    pub depth: u32,
    pub documents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImpactPath {
    pub from: String,
    pub to: String,
    pub fact_id: String,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MandatoryObligation {
    pub id: String,
    pub kind: String,
    pub mandatory: bool,
    pub status: String,
    pub boundary_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CostRecord {
    pub total_wall_micros: u64,
    pub repository_snapshot_micros: u64,
    pub build_discovery_micros: u64,
    pub cold_index_micros: u64,
    pub warm_index_micros: u64,
    pub adapter_micros: u64,
    pub query_micros: u64,
    pub source_bytes_read: u64,
    pub emitted_bytes: u64,
    pub stored_fact_bytes: u64,
    pub model_visible_source_bytes: u64,
    pub cache_requests: u64,
    pub cache_hits: u64,
    pub provider_processing_micros: u64,
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let value = serde_json::to_value(value)?;
    let value = canonicalize_value(value);
    Ok(serde_json::to_vec(&value)?)
}

pub fn canonicalize_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize_value).collect())
        }
        serde_json::Value::Object(values) => {
            let ordered = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_value(value)))
                .collect();
            serde_json::Value::Object(ordered)
        }
        other => other,
    }
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

pub fn canonical_digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(sha256_bytes(&canonical_json(value)?))
}
