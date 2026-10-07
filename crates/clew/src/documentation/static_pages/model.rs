//! Inert, source-bound content shared by the static HTML and MDX publishers.
use crate::documentation::model::{Explanation, Observation, Source};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "codeclew-native-page-projection/1.0";
pub const DECLARATION_SCHEMA: &str = "codeclew-native-page-projection/1.1";
pub const CONTROL_FLOW_SCHEMA: &str = "codeclew-native-page-projection/1.2";

/// Describes the scope of the selected page projection, not runtime certainty.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectionKind {
    DeclarationOnly,
    SourceBehavior,
    CompilerControlFlow,
}

impl ProjectionKind {
    pub(crate) const fn is_declaration_view(self) -> bool {
        matches!(self, Self::DeclarationOnly | Self::CompilerControlFlow)
    }

    pub(crate) const fn forbids_expansion(self) -> bool {
        matches!(self, Self::DeclarationOnly | Self::CompilerControlFlow)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerControlFlowOwnerKey {
    pub service: String,
    pub scope: String,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerControlFlowNode {
    pub node_id: u64,
    pub role: crate::thread_flow_cfg::LocalCfgNodeRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::thread_flow_cfg::LocalCfgSourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerControlFlowProjection {
    pub owner_key: CompilerControlFlowOwnerKey,
    pub graph_observation_id: String,
    pub graph_id: String,
    pub graph_evidence_binding: String,
    pub descriptor_evidence_binding: String,
    pub provider: String,
    pub compiler_graph_name: String,
    pub nodes: Vec<CompilerControlFlowNode>,
    pub edges: Vec<crate::thread_flow_cfg::LocalCfgEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceOutlineOwnerKey {
    pub service: String,
    pub scope: String,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceOutlineEvent {
    pub observation_id: String,
    pub ordinal: u64,
    pub kind: String,
    pub file: String,
    pub start_line: u64,
    pub end_line: u64,
    /// The documented PSI event with capture coordinates removed.
    pub event: serde_json::Value,
    pub citation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_source: Option<super::super::source_span::ExactSourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceOutline {
    pub authority: String,
    pub owner_key: SourceOutlineOwnerKey,
    pub events: Vec<SourceOutlineEvent>,
    /// Indented common projection tree, or absent when the whole outline is vetoed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NeutralExactCallSiteOwnerKey {
    pub service: String,
    pub scope: String,
    pub symbol: String,
}

/// Typed exact argument-to-formal mappings attached to a retained Kotlin call.
/// The envelope is optional on the parent site: absence means mapping metadata
/// was not captured, while an empty, gap-free envelope proves an empty mapping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NeutralCallArgumentBindings {
    pub schema: String,
    pub arguments: Vec<NeutralCallArgumentBinding>,
    pub omitted_default_parameter_indices: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NeutralCallArgumentBinding {
    pub compilation_byte_start: u64,
    pub compilation_byte_end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_name: Option<String>,
    pub argument_type: String,
    pub parameter: String,
    pub parameter_index: u64,
    pub parameter_type: String,
    pub expression: String,
    pub citation_id: String,
}

/// One captured compiler call relation and its exact retained source span.
/// Byte offsets address the full compilation source recorded by the call-site
/// envelope; `expression` and its citation are relative to the retained Source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NeutralExactCallSite {
    pub relation_id: String,
    pub normalized_digest: String,
    pub target_identity: String,
    pub source_id: String,
    pub file: String,
    pub start_line: u64,
    pub end_line: u64,
    pub compilation_byte_start: u64,
    pub compilation_byte_end: u64,
    pub source_digest: String,
    pub evidence_binding: String,
    pub full_compilation_source_digest: String,
    pub expression: String,
    pub citation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_bindings: Option<NeutralCallArgumentBindings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetainedCallSites {
    pub owner_key: NeutralExactCallSiteOwnerKey,
    pub sites: Vec<NeutralExactCallSite>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<Gap>,
}

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
    /// Explicit protected note IDs resolved only from the selected Check inputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub note_ids: Vec<String>,
    /// Ordinary user documentation selected from an exact frozen publication.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authored_paragraphs: Vec<AuthoredParagraphSelection>,
    /// Bounded source-call bodies and selected-process navigation, never runtime activation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub expand_source_calls: bool,
    /// Syntax transformations over exact compiler variable occurrences; no runtime values.
    #[serde(default, skip_serializing_if = "is_false")]
    pub expand_data_state: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthoredParagraphSelection {
    pub bundle: String,
    pub operation: String,
    pub fragment: String,
}

/// Frozen authored prose owns its original source versions, separately from the
/// native page's current captured sources. Authorship remains declared/unassessed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthoredParagraph {
    pub selection: AuthoredParagraphSelection,
    pub publication_digest: String,
    pub bindings_digest: String,
    pub operation_digest: String,
    pub paragraph_digest: String,
    pub paragraph: Explanation,
    pub context_freshness: String,
    pub source_records: BTreeMap<String, Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BundleProjection {
    pub schema: String,
    pub input_digest: String,
    pub context_digest: String,
    pub selection_digest: String,
    pub pages: Vec<PageContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_call_graph: Option<SourceCallGraph>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
/// One selected process aggregate; publishers derive linked views from it.
pub struct PageContent {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_kind: Option<ProjectionKind>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub human_instructions: Vec<HumanInstruction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authored_paragraphs: Vec<AuthoredParagraph>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examined_sources: Option<ExaminedSources>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_state: Option<ExaminedDataState>,
}

/// Each body is cached once by service, compilation scope and compiler symbol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceCallGraph {
    pub schema: String,
    pub authority: String,
    pub max_depth: usize,
    pub max_additional_bodies: usize,
    pub max_additional_source_bytes: usize,
    pub nodes: BTreeMap<String, SourceCallNode>,
    pub process_links: Vec<ProcessCallLink>,
    pub reverse_examined_processes: BTreeMap<String, Vec<ExaminedProcessReason>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reverse_field_references: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceCallNode {
    pub id: String,
    pub service: String,
    pub scope: String,
    pub callable: CallableProjection,
    pub calls: Vec<SourceCallEdge>,
    pub citations: BTreeMap<String, Citation>,
    pub observations: BTreeMap<String, Observation>,
    pub sources: BTreeMap<String, Source>,
    /// Local examined text and call semantics; excludes global capture provenance.
    pub examined_source_digest: String,
    /// Present only for compiler-admitted retained declaration nodes in the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_projection_kind: Option<ProjectionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_state: Option<NodeDataState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceCallEdge {
    /// Stable structural path within the caller body, including call ordinal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement_id: Option<String>,
    pub source_identity: String,
    pub target_scope: String,
    pub call_source_ids: Vec<String>,
    pub relation_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<CallProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<PathCondition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reachable: Option<bool>,
    /// Compiler-exact source site; its schema intentionally carries no
    /// Java statement path, ordering, conditions or reachability facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_call_site: Option<NeutralExactCallSite>,
    pub target_declaration: Option<String>,
    pub target_node: Option<String>,
    pub status: String,
    pub receiver_lineage: String,
    pub runtime_dispatch: String,
    pub frontiers: Vec<Gap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessCallLink {
    pub from_process: String,
    pub to_process: String,
    pub caller_node: String,
    pub occurrence_path: String,
    pub relation_id: String,
    pub citation_id: String,
    pub authority: String,
    pub limitation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct ExaminedMembership {
    pub node: String,
    pub reason: String,
    pub via_process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExaminedSources {
    pub schema: String,
    pub authority: String,
    pub examined_source_digest: String,
    pub memberships: Vec<ExaminedMembership>,
    /// Selected handoff status/field/gap semantics for this and linked processes.
    pub handoff_context_digests: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExaminedProcessReason {
    pub process_id: String,
    pub reasons: Vec<ExaminedMembership>,
}

/// Captured human/imported material is displayed unchanged, never interpreted as source truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HumanInstruction {
    pub id: String,
    pub title: String,
    pub declared_author: String,
    pub classification: String,
    pub period: String,
    pub version_digest: String,
    /// Digest of the canonical serialized original text, matching docs note capture.
    pub content_digest: String,
    pub association_digest: String,
    pub text: String,
    pub authority: String,
    pub source_claim_status: String,
    /// Full captured association, including targets, tags and declared metadata.
    pub association: serde_json::Value,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_flow: Option<CompilerControlFlowProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_outline: Option<SourceOutline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_call_sites: Option<RetainedCallSites>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded_node: Option<String>,
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

/// Declaration identity does not identify an object instance. Receiver expressions
/// are deliberately not unified across inputs or source-local calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct DataStorage {
    pub identity: String,
    pub kind: String,
    pub receiver: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum DataValue {
    Interference {
        occurrence: String,
        prior_definitions: Vec<String>,
    },
    Choice {
        alternatives: Vec<DataGuardedAlternative>,
    },
    Literal {
        text: String,
    },
    Input {
        storage: DataStorage,
    },
    Read {
        storage: DataStorage,
        alternatives: Vec<String>,
    },
    Unary {
        operator: String,
        operand: Box<DataValue>,
    },
    Binary {
        operator: String,
        left: Box<DataValue>,
        right: Box<DataValue>,
    },
    CallResult {
        occurrence: String,
        receiver: Option<Box<DataValue>>,
        arguments: Vec<DataValue>,
        target_node: Option<String>,
        target_authority: String,
        frontier: Option<String>,
    },
    Opaque {
        syntax: String,
        reason: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataGuardedAlternative {
    pub definitions: Vec<String>,
    pub conditions: Vec<DataGuard>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataGuard {
    pub expression: String,
    pub holds: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataCompletion {
    pub occurrence: String,
    pub conditions: Vec<DataGuard>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataDefinition {
    pub id: String,
    pub storage: Option<DataStorage>,
    pub value: DataValue,
    pub conditions: Vec<DataGuard>,
    pub normal_completion_of: Vec<DataCompletion>,
    pub citation_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataArgument {
    pub slot: usize,
    pub formal_identity: Option<String>,
    pub value: DataValue,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataCall {
    pub mapping_authority: String,
    pub conditions: Vec<DataGuard>,
    pub occurrence: String,
    pub target_node: Option<String>,
    pub arguments: Vec<DataArgument>,
    pub return_definitions: Vec<String>,
    pub normal_completion_of: Vec<DataCompletion>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NodeDataState {
    pub schema: String,
    pub authority: String,
    pub data_state_digest: String,
    pub definitions: Vec<DataDefinition>,
    pub calls: Vec<DataCall>,
    pub field_declarations: Vec<String>,
    pub gaps: Vec<Gap>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExaminedDataState {
    pub schema: String,
    pub authority: String,
    pub data_state_digest: String,
    pub nodes: Vec<String>,
}
