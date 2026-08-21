//! Non-serializable authority receipts for semantic completeness inputs.
//!
//! A caller may propose a Thread IR, but cannot turn that proposal into an
//! authoritative receipt. This module rebuilds it through the Kotlin worker,
//! resolves every source anchor against the live checkout and runs the
//! snapshot's configured compile/tests before issuing session-bound handles.

use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::graph;
use crate::index::{REPOSITORY_INDEX_FACT, RepositoryIndex};
use crate::model::{
    BuildSystem, CompletenessStatus, EditIr, EditOperation, LocalGraph, Replacement,
    SemanticOperation, SlicePolicy, Snapshot, ThreadIr, Transaction,
};
use crate::proto::RequestKind;
use crate::semantic_goal::{
    BindingRole, ChangeGraph, ChangeObligation, ConstraintDomain, DischargeStatus,
    EvidenceRelation, EvidenceStrength, GoalFamily, ObligationKind, OperatorApplication,
    PrimitiveConstraint, SemanticGoal, TYPED_GOAL_MAX_REQUEST_BYTES, TypedGoalLanguageError,
    TypedSemanticGoal, TypedVariableDomain, UnresolvedVerificationObligation, VerificationMethod,
    VerificationObligationCode, constraint_op_spec,
};
pub use crate::semantic_goal::{
    TYPED_GOAL_BINDING_DECISION_SCHEMA, TYPED_GOAL_BINDING_REQUEST_SCHEMA, TypedGoalRefusalReason,
};
use crate::transaction;
use crate::worker::WorkerClient;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use uuid::Uuid;
use walkdir::WalkDir;

/// Capability handle. Its fields are private and the type deliberately has no
/// serde implementation: JSON cannot manufacture or replay it in a new
/// authority session.
#[derive(Debug)]
pub struct VerifiedThreadReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// Capability issued for a compiler-resolved test whose assertion consumes
/// the result of the production callable proven by a thread receipt.
#[derive(Debug)]
pub struct VerifiedBehavioralTestReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// Capability issued only after the configured build/test lifecycle succeeds.
#[derive(Debug)]
pub struct ValidationReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// Opaque handle for an authority-materialized candidate overlay. Candidate
/// sources and their production/test classification never cross this API.
#[derive(Debug)]
pub struct CandidateOverlayReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// Opaque capability proving that one exact candidate passed while the
/// authority-produced omission mutant failed the same selected test.
#[derive(Debug)]
pub struct DifferentialValidationReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
    summary: DifferentialValidationSummary,
}

pub const SIGNED_EXTERNAL_SPEC_SCHEMA: &str = "signed-external-task-spec/0.1";
pub const EXTERNAL_SPEC_PAYLOAD_SCHEMA: &str = "external-task-spec-payload/0.1";
pub const EXTERNAL_SPEC_PACKAGE_SCHEMA: &str = "external-task-package/0.1";
pub const EXTERNAL_SPEC_PROOF_SCHEMA: &str = "external-spec-proof/0.1";
pub const PRODUCTION_EXTERNAL_SPEC_ISSUER: &str = "codeclew-e04-production-2026-08";
const PRODUCTION_EXTERNAL_SPEC_VERIFYING_KEY_HEX: &str =
    "8bf9107a5274f66b454a74b0d6b64c7467145c3eb8a5c902ef108557345f4981";

/// Strict language- and family-neutral task specification. The authority
/// computes every digest itself; callers cannot submit a digest as evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExternalSpecPayload {
    pub schema: String,
    pub issuer: String,
    pub task: String,
    pub task_digest: String,
    pub public_manifest: String,
    pub public_manifest_digest: String,
    pub package_digest: String,
    pub repository: String,
    pub repository_revision: String,
    pub source_snapshot_sha256: String,
    pub request_digest: String,
    pub compilation: String,
    pub project_model_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SignedExternalSpecEnvelope {
    pub schema: String,
    pub payload: ExternalSpecPayload,
    pub signature: String,
}

/// Opaque same-process capability. It deliberately has no serde traits and
/// exposes no constructor or fields.
#[derive(Debug)]
pub struct ExternalSpecReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// Lossy proof provenance. It can explain a BOUND result but cannot authorize
/// materialization or be replayed as an ExternalSpecReceipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExternalSpecProof {
    pub schema: String,
    pub provenance_class: String,
    pub issuer: String,
    pub specification_digest: String,
    pub task_digest: String,
    pub public_manifest_digest: String,
    pub package_digest: String,
    pub request_digest: String,
    pub source_snapshot_sha256: String,
    pub compilation_digest: String,
    pub project_model_hash: String,
}

#[derive(Debug, Clone)]
struct AuthorizedExternalSpec {
    specification_path: PathBuf,
    repository_contained_paths: Vec<PathBuf>,
    proof: ExternalSpecProof,
    resolved_compilation: String,
    verifying_key: [u8; 32],
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DifferentialValidationSummary {
    pub schema: String,
    pub revision: String,
    pub overlay_hash: String,
    pub route_hash: String,
    pub production_write_count: usize,
    pub test_write_count: usize,
    pub candidate_artifact_hash: String,
    pub omission_artifact_hash: String,
    pub candidate_compile_duration_ms: u64,
    pub candidate_test_duration_ms: u64,
    pub omission_compile_duration_ms: u64,
    pub omission_test_duration_ms: u64,
}

impl DifferentialValidationReceipt {
    pub fn summary(&self) -> &DifferentialValidationSummary {
        &self.summary
    }
}

/// The currently proven structural contour. New families must add their own
/// worker-derived binder; changing this label never changes the evidence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProvenStructuralFamily {
    ProducerTransformConsumer,
}

/// Model-owned intent for the narrow D02 family. It contains no symbols,
/// source, edges, anchors or oracle claims; those are bound by the authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProducerTransformConsumerGoal {
    pub schema: String,
    pub base_revision: String,
}

impl ProducerTransformConsumerGoal {
    pub fn new(base_revision: impl Into<String>) -> Self {
        Self {
            schema: "producer-transform-consumer-goal/0.1".into(),
            base_revision: base_revision.into(),
        }
    }

    fn is_valid_for(&self, revision: &str) -> bool {
        self.schema == "producer-transform-consumer-goal/0.1"
            && !self.base_revision.is_empty()
            && self.base_revision == revision
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompleteForSummary {
    pub schema: String,
    pub family: ProvenStructuralFamily,
    pub revision: String,
    pub producer_node: String,
    pub transformer_node: String,
    pub consumer_node: String,
    pub goal_fingerprint: String,
    pub evidence_fingerprint: String,
}

/// Opaque family-relative theorem receipt. Like its prerequisites, it cannot
/// be deserialized or constructed outside this module.
#[derive(Debug)]
pub struct CompleteForReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
    summary: CompleteForSummary,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MapEdgeInvariant {
    TypeAssignable,
    ContextEvaluatedOnce,
    PlacementDominatesUses,
    OrderPreserved,
    CardinalityPreserved,
    LazinessPreserved,
    EffectsPreserved,
    NullabilityPreserved,
    ConsumerContractPreserved,
    AbiPreserved,
    BehavioralOracleAvailable,
    NoUnsupportedBoundary,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapEdgeInvariantProof {
    pub invariant: MapEdgeInvariant,
    pub evidence_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapEdgeBindingSummary {
    pub workflow_symbol: String,
    pub context_producer_symbol: String,
    pub transformer_symbol: String,
    pub value_edge_from: String,
    pub value_edge_to: String,
    pub value_parameter_index: usize,
    pub placement: String,
    pub collection_type: String,
    pub element_type: String,
    pub context_type: String,
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapEdgeProofSummary {
    pub schema: String,
    pub revision: String,
    pub goal_fingerprint: String,
    pub bindings: MapEdgeBindingSummary,
    pub invariants: Vec<MapEdgeInvariantProof>,
    pub change_graph: ChangeGraph,
    pub evidence_fingerprint: String,
}

#[derive(Debug)]
pub struct MapEdgeWithContextReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
    summary: MapEdgeProofSummary,
}

impl MapEdgeWithContextReceipt {
    pub fn summary(&self) -> &MapEdgeProofSummary {
        &self.summary
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapEdgeChoice {
    pub context_producer_symbol: String,
    pub transformer_symbol: String,
    pub element_type: String,
    pub context_type: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapEdgeAmbiguity {
    pub schema: String,
    pub status: String,
    pub choices: Vec<MapEdgeChoice>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MapEdgeRefusalReason {
    InvalidGoal,
    SnapshotMismatch,
    NonUniqueValueEdge,
    UnsupportedCollectionModality,
    UnsupportedBoundary,
    IdentityOrAliasExposure,
    NoCompatibleContextAndTransformer,
    UnknownEffects,
    MissingBehavioralOracle,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapEdgeRefusal {
    pub schema: String,
    pub status: String,
    pub reason: MapEdgeRefusalReason,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapEdgeConditional {
    pub schema: String,
    pub status: String,
    pub revision: String,
    pub goal_fingerprint: String,
    pub bindings: MapEdgeBindingSummary,
    pub established_invariants: Vec<MapEdgeInvariantProof>,
    pub change_graph: ChangeGraph,
    pub unresolved_obligations: Vec<UnresolvedVerificationObligation>,
    pub evidence_fingerprint: String,
}

#[derive(Debug)]
pub enum MapEdgeWithContextDecision {
    Bound(Box<MapEdgeWithContextReceipt>),
    Conditional(MapEdgeConditional),
    Ambiguous(MapEdgeAmbiguity),
    Refused(MapEdgeRefusal),
}

pub const TYPED_GOAL_PROOF_SUMMARY_SCHEMA: &str = "typed-goal-proof-summary/0.1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TypedGoalBindingRequest {
    pub schema: String,
    pub goal: TypedSemanticGoal,
    #[serde(default)]
    pub hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compilation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TypedGoalProofSummary {
    pub schema: String,
    pub revision: String,
    pub goal_fingerprint: String,
    pub bindings: BTreeMap<String, String>,
    pub discharged_operators: BTreeSet<OperatorApplication>,
    pub evidence_relations: Vec<ProvenRelationRecord>,
    pub change_graph: ChangeGraph,
    pub evidence_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_spec: Option<ExternalSpecProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProvenRelationRecord {
    pub operator: OperatorApplication,
    pub relation: EvidenceRelation,
    pub bound_operands: Vec<String>,
    pub evidence_fingerprint: String,
    pub current: bool,
    pub unknown: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_set_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrence_fingerprints: Vec<String>,
}

impl TypedGoalProofSummary {
    /// Structural check for a transferable proof summary. Authority
    /// recognition below additionally checks the session-owned evidence.
    pub fn is_complete_for(&self, goal: &TypedSemanticGoal) -> bool {
        let expected_goal_fingerprint = canonical::hash(goal).ok();
        let expected_closure = goal
            .execution_plan()
            .ok()
            .map(|plan| plan.mandatory_closure);
        let expected_variables = expected_closure.as_ref().map(|closure| {
            closure
                .iter()
                .flat_map(|application| application.operands.iter().cloned())
                .collect::<BTreeSet<_>>()
        });
        self.schema == TYPED_GOAL_PROOF_SUMMARY_SCHEMA
            && goal.validate_language().is_ok()
            && self.revision == goal.base_revision
            && self.goal_fingerprint == expected_goal_fingerprint.unwrap_or_default()
            && Some(self.discharged_operators.clone()) == expected_closure
            && Some(self.bindings.keys().cloned().collect::<BTreeSet<_>>()) == expected_variables
            && self.bindings.values().all(|value| !value.is_empty())
            && self.change_graph.goal_schema == goal.schema
            && self.change_graph.validate_closure().is_ok()
            && self.change_graph.obligations.iter().all(|obligation| {
                obligation.status == DischargeStatus::Proved && !obligation.evidence.is_empty()
            })
            && self.change_graph.obligations.iter().all(|obligation| {
                self.discharged_operators
                    .iter()
                    .any(|application| obligation_matches_application(obligation, application))
            })
            && self.discharged_operators.iter().all(|application| {
                self.change_graph
                    .obligations
                    .iter()
                    .any(|obligation| obligation_matches_application(obligation, application))
            })
            && self.discharged_operators.iter().all(|application| {
                constraint_op_spec(&application.operator)
                    .required_evidence_relations
                    .iter()
                    .all(|relation| {
                        self.evidence_relations.iter().any(|record| {
                            record.operator == *application
                                && record.relation == *relation
                                && record.current
                                && !record.unknown
                                && record.bound_operands
                                    == application
                                        .operands
                                        .iter()
                                        .filter_map(|operand| self.bindings.get(operand).cloned())
                                        .collect::<Vec<_>>()
                                && !record.evidence_fingerprint.is_empty()
                        })
                    })
            })
            && self.evidence_relations.iter().all(|record| {
                self.discharged_operators.contains(&record.operator)
                    && constraint_op_spec(&record.operator.operator)
                        .required_evidence_relations
                        .contains(&record.relation)
                    && record.current
                    && !record.unknown
                    && match (
                        record.occurrence_count,
                        record.occurrence_set_fingerprint.as_ref(),
                    ) {
                        (None, None) => record.occurrence_fingerprints.is_empty(),
                        (Some(count), Some(fingerprint)) => {
                            count > 0
                                && count == record.occurrence_fingerprints.len()
                                && record
                                    .occurrence_fingerprints
                                    .windows(2)
                                    .all(|pair| pair[0] < pair[1])
                                && canonical::hash(&record.occurrence_fingerprints)
                                    .is_ok_and(|actual| &actual == fingerprint)
                        }
                        _ => false,
                    }
            })
    }
}

#[derive(Debug)]
pub struct TypedGoalBindingReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
    summary: TypedGoalProofSummary,
}

impl TypedGoalBindingReceipt {
    pub fn summary(&self) -> &TypedGoalProofSummary {
        &self.summary
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TypedGoalChoice {
    pub bindings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OracleRejectionStage {
    ResolveIdentity,
    K2Validation,
    TargetAssertion,
    ContextArgument,
    AuthorityReceipt,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OracleCandidateRejection {
    pub identity_fingerprint: String,
    pub owner: String,
    pub stage: OracleRejectionStage,
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_diagnostic: Option<OracleCompilerDiagnostic>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OracleCompilerDiagnostic {
    pub requested_compilation: String,
    pub module: String,
    pub source_set: String,
    pub source_roots: Vec<String>,
    pub project_model_hash: String,
    pub classpath_hash: String,
    pub compiler_options_hash: String,
    pub candidate_symbol_identity: String,
    pub diagnostic_codes: Vec<String>,
    pub unresolved_symbols: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeclarationProviderStage {
    IndexCoverage,
    WorkerAuthority,
    VerifiedIndexReceipt,
    RawSchemaHash,
    DescriptorGraph,
    DescriptorKindIdentity,
    OwnerContainment,
    VisibilityModality,
    JvmSignature,
    ParameterSlots,
    UnknownDescriptorBoundary,
    RelationGraph,
    CrossGraphConsistency,
    SourceBinding,
    DistributionProvenance,
    K2Validation,
    SchemaProvenance,
    GraphCoverage,
    UnknownBoundary,
    NullPolicyRelation,
    DescriptorIdentity,
    SourceDescriptor,
    FallbackDescriptor,
    OverrideRelation,
    TypeCompatibility,
    UseClosure,
    UseCallType,
    UseReferenceType,
    AbiBoundary,
    ExternalSpec,
    PerOperatorEvaluation,
    NullPolicy,
    ConstructsSlot,
    ThreadPath,
    TypeNullability,
    ReturnRelation,
    ProjectionDescriptor,
    DownstreamCallSlot,
    ValueFlowThread,
    OccurrenceSetCoverage,
    SourceCallIdentity,
    DestCallIdentity,
    DestParameterSlot,
    SourceOccurrence,
    ArgParamEdge,
    DefUseEdge,
    ThreadBuild,
    CallBoundary,
    ReadsetLive,
    OccurrenceCardinality,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeclarationProviderRejection {
    pub stage: DeclarationProviderStage,
    pub code: ErrorCode,
    pub fact_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_cardinality: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_comparison: Option<DeclarationTypeComparisonDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_shapes: Option<BTreeMap<String, DeclarationFieldShape>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fact_cardinalities: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub range_relations: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fact_relations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NullableDiscoveryDiagnostic {
    schema: String,
    stage: DeclarationProviderStage,
    shapes: BTreeMap<String, DeclarationFieldShape>,
    cardinalities: BTreeMap<String, usize>,
    range_relations: BTreeMap<String, String>,
    fact_relations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProjectionDiscoveryDiagnostic {
    schema: String,
    stage: DeclarationProviderStage,
    counts: BTreeMap<String, usize>,
    hashes: BTreeMap<String, String>,
    shapes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeclarationTypeComparisonDiagnostic {
    pub relation_source_return_type: String,
    pub relation_base_return_type: String,
    pub source_descriptor_return_type: String,
    pub source_descriptor_nullable: bool,
    pub base_descriptor_return_type: String,
    pub base_descriptor_nullable: bool,
    pub rendering_classes: Vec<String>,
    pub canonical_type_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeclarationFieldShape {
    pub present: bool,
    pub json_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub array_length: Option<usize>,
    pub value_hash: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TypedGoalAmbiguity {
    pub schema: String,
    pub status: String,
    pub choices: Vec<TypedGoalChoice>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TypedGoalRefusal {
    pub schema: String,
    pub status: String,
    pub reason: TypedGoalRefusalReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejections: Vec<OracleCandidateRejection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declaration_rejections: Vec<DeclarationProviderRejection>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TypedGoalConditional {
    pub schema: String,
    pub status: String,
    pub revision: String,
    pub goal_fingerprint: String,
    pub bindings: BTreeMap<String, String>,
    pub established_evidence_fingerprint: String,
    pub unresolved_obligations: Vec<UnresolvedVerificationObligation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejections: Vec<OracleCandidateRejection>,
}

#[derive(Debug)]
pub enum TypedGoalBindingDecision {
    Bound(Box<TypedGoalBindingReceipt>),
    Conditional(TypedGoalConditional),
    Ambiguous(TypedGoalAmbiguity),
    Refused(TypedGoalRefusal),
}

impl CompleteForReceipt {
    pub fn summary(&self) -> &CompleteForSummary {
        &self.summary
    }
}

/// The only transferable result is a lossy summary. It is evidence about an
/// authority decision, not a capability that can authorize another decision.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceBundleSummary {
    pub schema: String,
    pub revision: String,
    pub thread_count: usize,
    pub behavioral_test_count: usize,
    pub evidence_fingerprint: String,
    pub validation_artifact_hash: String,
    pub executed_test_count: usize,
    pub compile_duration_ms: u64,
    pub test_duration_ms: u64,
}

#[derive(Debug)]
pub struct AuthoritativeEvidenceBundle {
    summary: EvidenceBundleSummary,
}

impl AuthoritativeEvidenceBundle {
    pub fn summary(&self) -> &EvidenceBundleSummary {
        &self.summary
    }
}

#[derive(Debug)]
struct VerifiedThread {
    fingerprint: String,
    thread: ThreadIr,
    source_files: BTreeMap<PathBuf, String>,
}

#[derive(Debug, Clone)]
struct VerifiedBehavioralTest {
    fingerprint: String,
    target_compiler_symbol: String,
    class_name: String,
    test_name: String,
    validation_route: ValidationRoute,
    source_files: BTreeMap<PathBuf, String>,
    candidate_overlay_receipt_id: Option<Uuid>,
    candidate_overlay_hash: Option<String>,
}

#[derive(Debug)]
struct ValidationRun {
    thread_set_fingerprint: String,
    test_set_fingerprint: String,
    artifact_hash: String,
    executed_test_count: usize,
    compile_duration_ms: u64,
    test_duration_ms: u64,
    report_route_fingerprint: String,
}

#[derive(Debug, Clone)]
struct CandidateOverlay {
    revision: String,
    thread_fingerprint: String,
    test_fingerprint: Option<String>,
    production_project_model_hash: String,
    test_compilation: String,
    test_project_model_hash: String,
    test_compile_task: String,
    route: Option<ValidationRoute>,
    candidates: BTreeMap<String, String>,
    production_files: BTreeSet<String>,
    test_files: BTreeSet<String>,
    affected_callables: BTreeSet<String>,
    oracle_rejections: Vec<CandidateOracleRejection>,
    overlay_hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CandidateOracleFailureStage {
    ResolveSymbol,
    K2Validation,
    MissingExactCall,
    AssertionActualNotDerived,
    IdentityMismatch,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CandidateOracleRejection {
    callable_fingerprint: String,
    stage: CandidateOracleFailureStage,
    code: ErrorCode,
    identity_comparison: Value,
}

#[derive(Debug)]
struct DifferentialValidationRun {
    overlay_receipt_id: Uuid,
    summary: DifferentialValidationSummary,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ValidationRoute {
    build_system: BuildSystem,
    compilation: String,
    module: String,
    source_set: String,
    build_launcher: String,
    test_binary_class: String,
    test_method: String,
    test_selector: String,
    invocation: Vec<String>,
    report_format: String,
    report_root: PathBuf,
    project_model_hash: String,
}

#[derive(Debug, Clone)]
struct MapValueEdge {
    workflow_symbol: String,
    from: String,
    to: String,
    parameter_index: usize,
    placement: String,
    collection_type: String,
    element_type: String,
}

#[derive(Debug)]
struct AuthorizedMapEdgeProof {
    thread_fingerprint: String,
    summary: MapEdgeProofSummary,
}

#[derive(Debug, Clone)]
struct CallableCandidate {
    compiler_symbol: String,
    query_symbol: String,
    parameter_types: Vec<String>,
    return_type: String,
}

#[derive(Debug, Clone)]
struct MapCandidate {
    context: CallableCandidate,
    transformer: CallableCandidate,
    context_resolution_hash: String,
    transformer_resolution_hash: String,
}

/// Language-neutral receipt for facts returned by a compiler-backed callable
/// provider.  The authority commits the complete opaque provider payload, not
/// a caller-supplied symbol or a reconstructed boolean.
#[derive(Debug, Clone)]
struct ResolvedCallableEvidence {
    callable: CallableCandidate,
    resolution_fingerprint: String,
    type_fingerprint: String,
    effect_fingerprint: String,
    effects_proven_pure: bool,
}

#[derive(Debug, Clone)]
struct ResolvedMapCandidate {
    context: ResolvedCallableEvidence,
    transformer: ResolvedCallableEvidence,
}

#[derive(Debug, Clone)]
struct DiscoveredValueFlow {
    thread: ThreadIr,
    thread_fingerprint: String,
    edge: MapValueEdge,
    map_candidates: Vec<ResolvedMapCandidate>,
    transformers: Vec<ResolvedCallableEvidence>,
    index_hash: String,
}

#[derive(Debug)]
struct SelectedFlowEvidence {
    flow_index: usize,
    transformer: ResolvedCallableEvidence,
    map_candidate: Option<ResolvedMapCandidate>,
}

/// One compiler-proven declaration propagation edge and its complete
/// repository-known override/use closure.  Symbols are JVM-disambiguated
/// identities from DeclarationDescriptor; source names and task vocabulary
/// are never used for candidate selection.
#[derive(Debug, Clone)]
struct DeclarationTypeCandidate {
    source_symbol: String,
    target_symbol: String,
    source_callable: String,
    target_callable: String,
    propagation_fingerprint: String,
    override_fingerprint: String,
    use_closure_fingerprint: String,
    contract_fingerprint: String,
    boundary_closure_fingerprint: String,
}

struct DeclarationTypeOperatorEvidenceProvider<'a> {
    bindings: &'a BTreeMap<String, String>,
    viable_bindings: &'a [BTreeMap<String, String>],
    candidate: &'a DeclarationTypeCandidate,
    external_spec: &'a ExternalSpecProof,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NullableConstructionOccurrence {
    owner_callable: String,
    slot_index: usize,
    module: String,
    source_set: String,
    source_range: (u64, u64),
    fallback_range: (u64, u64),
    result_range: (u64, u64),
    construction_range: (u64, u64),
    thread_id: String,
    null_policy_fingerprint: String,
    construction_fingerprint: String,
    value_flow_fingerprint: String,
    use_closure_fingerprint: String,
    contract_fingerprint: String,
    thread_fingerprint: String,
    read_set_fingerprint: String,
    provenance_fingerprint: String,
    boundary_closure_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NullableConstructionCandidate {
    source_symbol: String,
    fallback_symbol: String,
    destination_symbol: String,
    source_callable: String,
    fallback_callable: String,
    destination_callable: String,
    module: String,
    source_set: String,
    occurrences: Vec<NullableConstructionOccurrence>,
    occurrence_fingerprints: Vec<String>,
    occurrence_set_fingerprint: String,
    use_closure_fingerprint: String,
    contract_fingerprint: String,
    provenance_fingerprint: String,
}

struct NullableConstructionOperatorEvidenceProvider<'a> {
    bindings: &'a BTreeMap<String, String>,
    viable_bindings: &'a [BTreeMap<String, String>],
    candidate: &'a NullableConstructionCandidate,
    external_spec: &'a ExternalSpecProof,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionConsumerOccurrence {
    return_relation_fingerprint: String,
    value_flow: DeclarationValueFlowCandidate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionConsumerCandidate {
    source_symbol: String,
    projection_symbol: String,
    consumer_symbol: String,
    source_callable: String,
    projection_callable: String,
    consumer_callable: String,
    occurrences: Vec<ProjectionConsumerOccurrence>,
    occurrence_fingerprints: Vec<String>,
    occurrence_set_fingerprint: String,
    declared_type_fingerprint: String,
    use_closure_fingerprint: String,
    contract_fingerprint: String,
    provenance_fingerprint: String,
}

struct ProjectionConsumerOperatorEvidenceProvider<'a> {
    bindings: &'a BTreeMap<String, String>,
    viable_bindings: &'a [BTreeMap<String, String>],
    candidate: &'a ProjectionConsumerCandidate,
    external_spec: &'a ExternalSpecProof,
}

/// One occurrence-level value flow between two exact compiler declarations.
/// This is deliberately a proof fact, not an executable composition or edit
/// recipe. Every identity and position is derived from verified compiler/index
/// facts and a live local Thread IR.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeclarationValueFlowCandidate {
    source_symbol: String,
    destination_symbol: String,
    owner_callable: String,
    source_call_node_id: String,
    source_node_id: String,
    destination_node_id: String,
    slot_kind: &'static str,
    slot_index: usize,
    source_type: String,
    source_nullable: bool,
    destination_type: String,
    destination_nullable: bool,
    order: &'static str,
    dominance: &'static str,
    evaluation_count: usize,
    module: String,
    source_set: String,
    relation_fingerprint: String,
    descriptor_fingerprint: String,
    thread_fingerprint: String,
    read_set_fingerprint: String,
    provenance_fingerprint: String,
    boundary_closure_fingerprint: String,
}

/// Backend-independent evidence boundary. Implementations may use a language
/// compiler, graph engine, or another semantic provider, but callers only see
/// typed relations and fingerprints of the provider facts that established
/// them.
trait OperatorEvidenceProvider {
    fn prove(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProviderFactReceipt {
    relation: EvidenceRelation,
    provider_kind: &'static str,
    fact_fingerprint: String,
}

#[derive(Debug, Clone)]
struct DiscoveredTestCandidate {
    owner: String,
    compiler_identity: String,
    queries: Vec<String>,
}

#[derive(Debug, Clone)]
struct OracleCompilationContext {
    requested_compilation: String,
    module: String,
    source_set: String,
    source_roots: Vec<String>,
    project_model_hash: String,
    classpath_hash: String,
    compiler_options_hash: String,
}

/// Process-local authority. Receipts from one instance are meaningless to
/// every other instance, even for the same checkout and revision.
pub struct EvidenceAuthority {
    session_id: Uuid,
    repo: PathBuf,
    revision: String,
    threads: BTreeMap<Uuid, VerifiedThread>,
    tests: BTreeMap<Uuid, VerifiedBehavioralTest>,
    validations: BTreeMap<Uuid, ValidationRun>,
    candidate_overlays: BTreeMap<Uuid, CandidateOverlay>,
    differential_validations: BTreeMap<Uuid, DifferentialValidationRun>,
    completions: BTreeSet<Uuid>,
    map_edge_proofs: BTreeMap<Uuid, AuthorizedMapEdgeProof>,
    typed_goal_proofs: BTreeMap<Uuid, TypedGoalProofSummary>,
    external_specs: BTreeMap<Uuid, AuthorizedExternalSpec>,
}

impl EvidenceAuthority {
    pub fn open(repo: &Path, expected_revision: &str) -> Result<Self, ClewError> {
        let repo = repo.canonicalize().map_err(|error| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("cannot resolve evidence repository: {error}"),
            )
        })?;
        let revision = git_head(&repo)?;
        if revision != expected_revision {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!("authority expected revision {expected_revision}, found {revision}"),
            ));
        }
        ensure_clean_checkout(&repo)?;
        Ok(Self {
            session_id: Uuid::new_v4(),
            repo,
            revision,
            threads: BTreeMap::new(),
            tests: BTreeMap::new(),
            validations: BTreeMap::new(),
            candidate_overlays: BTreeMap::new(),
            differential_validations: BTreeMap::new(),
            completions: BTreeSet::new(),
            map_edge_proofs: BTreeMap::new(),
            typed_goal_proofs: BTreeMap::new(),
            external_specs: BTreeMap::new(),
        })
    }

    /// Issues a same-session capability for one immutable external task
    /// specification. The specification is evidence of stated intent only;
    /// it can discharge binder-level REQUIRE_ORACLE but can never authorize
    /// source materialization or test correctness.
    pub fn issue_external_spec(
        &mut self,
        specification_path: &Path,
        request: &TypedGoalBindingRequest,
        compilation: Option<&str>,
        worker: &mut WorkerClient,
    ) -> Result<ExternalSpecReceipt, ClewError> {
        let key = production_external_spec_verifying_key()?;
        self.issue_external_spec_with_verifier(
            specification_path,
            request,
            compilation,
            PRODUCTION_EXTERNAL_SPEC_ISSUER,
            key,
            worker,
        )
    }

    fn issue_external_spec_with_verifier(
        &mut self,
        specification_path: &Path,
        request: &TypedGoalBindingRequest,
        compilation: Option<&str>,
        expected_issuer: &str,
        verifying_key: [u8; 32],
        worker: &mut WorkerClient,
    ) -> Result<ExternalSpecReceipt, ClewError> {
        self.ensure_revision()?;
        if request.schema != TYPED_GOAL_BINDING_REQUEST_SCHEMA
            || request.goal.base_revision != self.revision
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "external specification request does not match the authority revision",
            ));
        }
        let (payload, canonical_path, contained_paths, specification_digest) =
            read_signed_external_spec(
                &self.repo,
                specification_path,
                expected_issuer,
                verifying_key,
            )?;
        let request_digest = canonical::hash(request).map_err(internal)?;
        if payload.repository_revision != self.revision || payload.request_digest != request_digest
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "signed external specification does not match revision or typed request",
            ));
        }
        let resolved_compilation = select_production_compilation(&self.repo, compilation)?;
        if payload.compilation != resolved_compilation {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "signed external specification does not match selected compilation",
            ));
        }
        let project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":resolved_compilation}),
        )?;
        require_exact_project_compilation(&project, &resolved_compilation)?;
        let project_model_hash = required_str(&project, "projectModelHash")?.to_owned();
        if payload.project_model_hash != project_model_hash {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "signed external specification does not match live project model",
            ));
        }
        let proof = ExternalSpecProof {
            schema: EXTERNAL_SPEC_PROOF_SCHEMA.into(),
            provenance_class: "EXTERNAL_SPEC".into(),
            issuer: payload.issuer.clone(),
            specification_digest,
            task_digest: payload.task_digest.clone(),
            public_manifest_digest: payload.public_manifest_digest.clone(),
            package_digest: payload.package_digest.clone(),
            request_digest,
            source_snapshot_sha256: payload.source_snapshot_sha256.clone(),
            compilation_digest: canonical::hash(&resolved_compilation).map_err(internal)?,
            project_model_hash,
        };
        let receipt_id = Uuid::new_v4();
        self.external_specs.insert(
            receipt_id,
            AuthorizedExternalSpec {
                specification_path: canonical_path,
                repository_contained_paths: contained_paths,
                proof,
                resolved_compilation,
                verifying_key,
            },
        );
        Ok(ExternalSpecReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    pub fn recognizes_external_spec(
        &self,
        receipt: &ExternalSpecReceipt,
    ) -> Result<bool, ClewError> {
        self.ensure_revision()?;
        Ok(receipt.session_id == self.session_id
            && self.external_specs.contains_key(&receipt.receipt_id))
    }

    fn revalidate_external_spec(
        &self,
        receipt: &ExternalSpecReceipt,
        request: &TypedGoalBindingRequest,
        compilation: Option<&str>,
        worker: &mut WorkerClient,
    ) -> Result<ExternalSpecProof, ClewError> {
        if receipt.session_id != self.session_id {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "external specification receipt belongs to another authority session",
            ));
        }
        let authorized = self
            .external_specs
            .get(&receipt.receipt_id)
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::InvalidInput,
                    "external specification receipt is not authority-owned",
                )
            })?;
        self.ensure_revision()?;
        let resolved_compilation = select_production_compilation(&self.repo, compilation)?;
        if resolved_compilation != authorized.resolved_compilation
            || canonical::hash(request).map_err(internal)? != authorized.proof.request_digest
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "typed request or compilation changed after external specification issuance",
            ));
        }
        let (payload, canonical_path, contained_paths, specification_digest) =
            read_signed_external_spec(
                &self.repo,
                &authorized.specification_path,
                &authorized.proof.issuer,
                authorized.verifying_key,
            )?;
        if canonical_path != authorized.specification_path
            || contained_paths != authorized.repository_contained_paths
            || specification_digest != authorized.proof.specification_digest
            || payload.task_digest != authorized.proof.task_digest
            || payload.public_manifest_digest != authorized.proof.public_manifest_digest
            || payload.package_digest != authorized.proof.package_digest
            || payload.source_snapshot_sha256 != authorized.proof.source_snapshot_sha256
            || payload.repository_revision != self.revision
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "external specification or repository snapshot changed after issuance",
            ));
        }
        let project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":resolved_compilation}),
        )?;
        require_exact_project_compilation(&project, &resolved_compilation)?;
        if required_str(&project, "projectModelHash")? != authorized.proof.project_model_hash {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "project model changed after external specification issuance",
            ));
        }
        Ok(authorized.proof.clone())
    }

    /// Rebuilds the proposal through the live worker and accepts it only when
    /// the resulting Thread IR is byte-for-byte canonical-equivalent.
    pub fn verify_thread(
        &mut self,
        proposed: &ThreadIr,
        worker: &mut WorkerClient,
    ) -> Result<VerifiedThreadReceipt, ClewError> {
        self.ensure_revision()?;
        if proposed.snapshot.base_revision != self.revision {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                "proposed thread belongs to another revision",
            ));
        }
        let project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":proposed.snapshot.compilation}),
        )?;
        if project.get("projectModelHash").and_then(Value::as_str)
            != Some(proposed.snapshot.project_model_hash.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                "live project model does not match proposed Thread IR",
            ));
        }
        let rebuilt =
            transaction::rebuild_thread(&self.repo, proposed, &project, &self.revision, worker)?;
        let proposed_hash = canonical::hash(proposed).map_err(internal)?;
        let rebuilt_hash = canonical::hash(&rebuilt).map_err(internal)?;
        if proposed_hash != rebuilt_hash {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                "worker-rebuilt Thread IR differs from the proposed evidence packet",
            ));
        }
        if rebuilt.completeness.status != CompletenessStatus::CompleteSupportedSubset
            || !rebuilt.completeness.boundaries.is_empty()
            || !rebuilt.external_summaries.is_empty()
        {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "worker-rebuilt Thread IR has an unsupported boundary",
            ));
        }
        let source_files = verify_live_sources(&self.repo, &rebuilt)?;
        let receipt_id = Uuid::new_v4();
        self.threads.insert(
            receipt_id,
            VerifiedThread {
                fingerprint: rebuilt_hash,
                thread: rebuilt,
                source_files,
            },
        );
        Ok(VerifiedThreadReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    /// Resolves a test through K2 and accepts it only when a recognized
    /// assertion consumes the exact compiler callable proven by `target`.
    pub fn verify_behavioral_test(
        &mut self,
        test_symbol: &str,
        compilation: &str,
        target: &VerifiedThreadReceipt,
        worker: &mut WorkerClient,
    ) -> Result<VerifiedBehavioralTestReceipt, ClewError> {
        self.ensure_revision()?;
        let targets = self.resolve_threads(&[target])?;
        let [target] = targets.as_slice() else {
            unreachable!("one receipt resolves to one thread")
        };
        let candidates = producer_transform_consumer_candidates(&target.thread);
        let [binding] = candidates.as_slice() else {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "target thread does not have one unique producer-transform-consumer binding",
            ));
        };
        let target_compiler_symbol = compiler_owner_symbol(&target.thread, &binding.0)?;
        transaction::validate_worktree(
            &self.repo,
            target.thread.snapshot.build_system,
            &target.thread.snapshot.build_launcher,
            &target.thread.snapshot.compile_task,
            &[],
        )?;
        self.issue_behavioral_test(
            test_symbol,
            compilation,
            &target_compiler_symbol,
            None,
            worker,
        )
    }

    fn issue_behavioral_test(
        &mut self,
        test_symbol: &str,
        compilation: &str,
        target_compiler_symbol: &str,
        required_context_symbol: Option<&str>,
        worker: &mut WorkerClient,
    ) -> Result<VerifiedBehavioralTestReceipt, ClewError> {
        let project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":compilation}),
        )?;
        if project.get("sourceSet").and_then(Value::as_str) != Some("test") {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "behavioral evidence must come from the test compilation",
            ));
        }
        let resolution = worker.request(
            RequestKind::ResolveSymbol,
            &json!({"repo":self.repo,"compilation":compilation,"symbol":test_symbol}),
        )?;
        if resolution.get("k2Validated").and_then(Value::as_bool) != Some(true)
            || resolution
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item.get("severity").and_then(Value::as_str) == Some("ERROR"))
                })
        {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!(
                    "behavioral test is not cleanly resolved by K2: {}",
                    resolution.get("diagnostics").unwrap_or(&Value::Null)
                ),
            ));
        }
        verify_assertion_of_target(&resolution, target_compiler_symbol)?;
        if let Some(context_symbol) = required_context_symbol {
            verify_context_argument_of_target(&resolution, target_compiler_symbol, context_symbol)?;
        }
        let declaration = resolution
            .get("declaration")
            .ok_or_else(|| invalid_source("test resolution has no declaration"))?;
        let identity = declaration
            .get("symbolIdentity")
            .ok_or_else(|| invalid_source("test declaration has no symbol identity"))?;
        let package = required_str(identity, "package")?;
        let containers = identity
            .get("containingDeclarations")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_source("test declaration has no containing class"))?;
        if containers.is_empty() {
            return Err(invalid_source(
                "behavioral test must belong to a test class",
            ));
        }
        let container_names = containers
            .iter()
            .map(|container| {
                container
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| invalid_source("test declaration has invalid containing class"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let binary_owner = container_names
            .split_first()
            .map(|(first, nested)| {
                format!(
                    "{first}{}",
                    nested
                        .iter()
                        .map(|name| format!("${name}"))
                        .collect::<String>()
                )
            })
            .ok_or_else(|| invalid_source("test declaration has no binary class"))?;
        let class_name = if package.is_empty() {
            binary_owner
        } else {
            format!("{package}.{binary_owner}")
        };
        let test_name = required_str(declaration, "name")?.to_owned();
        let report_route = validation_route(compilation, &project, &class_name, &test_name)?;
        let source_files = verify_resolution_source(&self.repo, &resolution)?;
        let fingerprint = canonical::hash(&(
            &self.revision,
            compilation,
            target_compiler_symbol,
            &class_name,
            &test_name,
            &report_route,
            &resolution,
        ))
        .map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        self.tests.insert(
            receipt_id,
            VerifiedBehavioralTest {
                fingerprint,
                target_compiler_symbol: target_compiler_symbol.to_owned(),
                class_name,
                test_name,
                validation_route: report_route,
                source_files,
                candidate_overlay_receipt_id: None,
                candidate_overlay_hash: None,
            },
        );
        Ok(VerifiedBehavioralTestReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    /// Runs the exact compile/test plan carried by the verified snapshot. A
    /// validation handle cannot be created from a claimed exit code or hash.
    pub fn run_validation(
        &mut self,
        receipts: &[&VerifiedThreadReceipt],
        tests: &[&VerifiedBehavioralTestReceipt],
        worker: &mut WorkerClient,
    ) -> Result<ValidationReceipt, ClewError> {
        self.ensure_revision()?;
        let verified = self.resolve_threads(receipts)?;
        let verified_tests = self.resolve_tests(tests)?;
        let primary = verified[0];
        for item in &verified {
            verify_sources_current(&self.repo, &item.source_files)?;
            if item.thread.snapshot.build_system != primary.thread.snapshot.build_system
                || item.thread.snapshot.build_launcher != primary.thread.snapshot.build_launcher
                || item.thread.snapshot.compile_task != primary.thread.snapshot.compile_task
                || item.thread.snapshot.test_tasks != primary.thread.snapshot.test_tasks
            {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "one validation receipt cannot cover different build plans",
                ));
            }
        }
        for test in &verified_tests {
            verify_sources_current(&self.repo, &test.source_files)?;
        }
        for item in &verified {
            let current = worker.request(
                RequestKind::OpenProject,
                &json!({"repo":self.repo,"compilation":item.thread.snapshot.compilation}),
            )?;
            let expected_build_system = match item.thread.snapshot.build_system {
                BuildSystem::Gradle => "GRADLE",
                BuildSystem::Maven => "MAVEN",
            };
            let production_module = item
                .thread
                .snapshot
                .compilation
                .rsplit_once('/')
                .map(|(module, _)| module)
                .ok_or_else(|| invalid_source("production compilation has no module identity"))?;
            if current.get("projectModelHash").and_then(Value::as_str)
                != Some(item.thread.snapshot.project_model_hash.as_str())
                || current.get("buildSystem").and_then(Value::as_str) != Some(expected_build_system)
                || current.get("buildLauncher").and_then(Value::as_str)
                    != Some(item.thread.snapshot.build_launcher.as_str())
                || current.get("module").and_then(Value::as_str) != Some(production_module)
            {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "production compilation model changed after authority verification",
                ));
            }
        }
        for test in &verified_tests {
            let current = worker.request(
                RequestKind::OpenProject,
                &json!({"repo":self.repo,"compilation":test.validation_route.compilation}),
            )?;
            if validation_route(
                &test.validation_route.compilation,
                &current,
                &test.class_name,
                &test.test_name,
            )? != test.validation_route
            {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "test compilation validation route changed after authority verification",
                ));
            }
        }
        let routes = verified_tests
            .iter()
            .map(|test| &test.validation_route)
            .collect::<Vec<_>>();
        let report_route = routes[0];
        if routes.iter().any(|route| *route != report_route) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "one validation receipt requires one exact test compilation/report route",
            ));
        }
        if report_route.build_system != primary.thread.snapshot.build_system
            || report_route.build_launcher != primary.thread.snapshot.build_launcher
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "behavioral validation route differs from the production snapshot build plan",
            ));
        }
        let production_module = primary
            .thread
            .snapshot
            .compilation
            .rsplit_once('/')
            .map(|(module, _)| module)
            .ok_or_else(|| invalid_source("production compilation has no module identity"))?;
        if production_module != report_route.module {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "behavioral test compilation belongs to a different production module",
            ));
        }
        let validation_tasks = report_route.invocation.clone();
        let run_nonce = Uuid::new_v4();
        let temporary = tempfile::tempdir().map_err(|error| {
            ClewError::new(
                ErrorCode::Internal,
                format!("cannot create validation workspace: {error}"),
            )
        })?;
        let worktree = temporary.path().join("validation-worktree");
        evidence_git(
            &self.repo,
            &[
                "worktree",
                "add",
                "--detach",
                worktree
                    .to_str()
                    .ok_or_else(|| invalid_source("validation worktree path is not UTF-8"))?,
                &self.revision,
            ],
        )?;
        let validation_result = (|| {
            prepare_validation_report_root(&worktree, report_route)?;
            let durations = transaction::validate_worktree_fresh(
                &worktree,
                primary.thread.snapshot.build_system,
                &primary.thread.snapshot.build_launcher,
                &primary.thread.snapshot.compile_task,
                &validation_tasks,
            )?;
            let artifact = test_artifact(&worktree, &verified_tests, report_route)?;
            Ok((durations, artifact))
        })();
        let _ = evidence_git(
            &self.repo,
            &[
                "worktree",
                "remove",
                "--force",
                worktree.to_str().unwrap_or_default(),
            ],
        );
        let ((compile_duration_ms, test_duration_ms), (test_artifact_hash, executed_test_count)) =
            validation_result?;
        let thread_set_fingerprint = thread_set_fingerprint(&verified)?;
        let test_set_fingerprint = test_set_fingerprint(&verified_tests)?;
        let report_route_fingerprint = canonical::hash(report_route).map_err(internal)?;
        let artifact_hash = canonical::hash(&(
            &self.revision,
            &thread_set_fingerprint,
            &test_set_fingerprint,
            primary.thread.snapshot.build_system,
            &primary.thread.snapshot.build_launcher,
            &primary.thread.snapshot.compile_task,
            &validation_tasks,
            &report_route_fingerprint,
            &test_artifact_hash,
            executed_test_count,
            &run_nonce,
        ))
        .map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        self.validations.insert(
            receipt_id,
            ValidationRun {
                thread_set_fingerprint,
                test_set_fingerprint,
                artifact_hash,
                executed_test_count,
                compile_duration_ms,
                test_duration_ms,
                report_route_fingerprint,
            },
        );
        Ok(ValidationReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    /// Materializes the currently supported authority-owned semantic edit
    /// into a private overlay and classifies every candidate file from the
    /// live production/test project models. The caller supplies capabilities,
    /// never candidate sources or file roles.
    pub fn materialize_candidate_overlay(
        &mut self,
        proof: &MapEdgeWithContextReceipt,
        worker: &mut WorkerClient,
    ) -> Result<CandidateOverlayReceipt, ClewError> {
        self.ensure_revision()?;
        if proof.session_id != self.session_id {
            return Err(wrong_session("candidate overlay prerequisite"));
        }
        let (thread, edit) = self.compile_map_edge_with_context_edit(proof)?;
        let stored_proof = self
            .map_edge_proofs
            .get(&proof.receipt_id)
            .ok_or_else(|| invalid_receipt("map-edge proof"))?;
        verify_sources_current(
            &self.repo,
            &self
                .threads
                .values()
                .find(|item| item.fingerprint == stored_proof.thread_fingerprint)
                .ok_or_else(|| invalid_receipt("candidate overlay thread"))?
                .source_files,
        )?;
        let production_project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":thread.snapshot.compilation}),
        )?;
        if production_project
            .get("projectModelHash")
            .and_then(Value::as_str)
            != Some(thread.snapshot.project_model_hash.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "candidate overlay production project model changed",
            ));
        }
        let preview =
            transaction::preview_authorized_semantic_overlay(&self.repo, &thread, &edit, worker)?;
        let (production_roots, production_generated) = project_source_roots(&production_project)?;
        let test_compilation = sibling_test_compilation(&thread.snapshot.compilation)?;
        let test_project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":test_compilation}),
        )?;
        if required_str(&production_project, "module")? != required_str(&test_project, "module")?
            || test_project.get("sourceSet").and_then(Value::as_str) != Some("test")
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "authority could not derive one same-module test compilation",
            ));
        }
        let (test_roots, test_generated) = project_source_roots(&test_project)?;
        let classification = classify_candidate_files(
            &preview.changed_files,
            &preview.candidates,
            &production_roots,
            &test_roots,
            &production_generated,
            &test_generated,
        )?;
        let production_files = classification.production_files;
        let test_files = classification.test_files;
        let test_project_model_hash = required_str(&test_project, "projectModelHash")?.to_owned();
        let test_compile_task = required_str(&test_project, "compileTask")?.to_owned();
        let affected_callables = edit
            .operations
            .iter()
            .filter_map(|operation| {
                operation
                    .target
                    .get("ownerSymbolId")
                    .and_then(Value::as_str)
                    .filter(|symbol| !symbol.is_empty())
                    .map(str::to_owned)
            })
            .collect::<BTreeSet<_>>();
        if affected_callables.is_empty() {
            return Err(invalid_source(
                "authority materializer produced no affected compiler callable",
            ));
        }
        let overlay_hash = canonical::hash(&(
            &self.revision,
            &stored_proof.thread_fingerprint,
            &thread.snapshot.project_model_hash,
            &test_compilation,
            &test_project_model_hash,
            &test_compile_task,
            &preview.candidates,
            &production_files,
            &test_files,
            &affected_callables,
        ))
        .map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        self.candidate_overlays.insert(
            receipt_id,
            CandidateOverlay {
                revision: self.revision.clone(),
                thread_fingerprint: stored_proof.thread_fingerprint.clone(),
                test_fingerprint: None,
                production_project_model_hash: thread.snapshot.project_model_hash,
                test_compilation,
                test_project_model_hash,
                test_compile_task,
                route: None,
                candidates: preview.candidates,
                production_files,
                test_files,
                affected_callables,
                oracle_rejections: Vec::new(),
                overlay_hash,
            },
        );
        Ok(CandidateOverlayReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    /// Issues a test capability scoped to one opaque candidate. The affected
    /// callable set comes from authority-materialized edit targets; the caller
    /// provides only the test identity and compilation selector.
    pub fn issue_candidate_behavioral_test(
        &mut self,
        overlay_receipt: &CandidateOverlayReceipt,
        test_symbol: &str,
        compilation: &str,
        worker: &mut WorkerClient,
    ) -> Result<VerifiedBehavioralTestReceipt, ClewError> {
        self.ensure_revision()?;
        if overlay_receipt.session_id != self.session_id {
            return Err(wrong_session("candidate overlay"));
        }
        let snapshot = self
            .candidate_overlays
            .get(&overlay_receipt.receipt_id)
            .ok_or_else(|| invalid_receipt("candidate overlay"))?
            .clone();
        if snapshot.test_fingerprint.is_some() || snapshot.route.is_some() {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "candidate overlay already has a bound behavioral test",
            ));
        }
        if compilation != snapshot.test_compilation {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "candidate behavioral test compilation differs from authority-derived test compilation",
            ));
        }
        let production_compilation = self
            .threads
            .values()
            .find(|thread| thread.fingerprint == snapshot.thread_fingerprint)
            .ok_or_else(|| invalid_receipt("candidate overlay thread"))?
            .thread
            .snapshot
            .compilation
            .clone();
        let mut issued = Vec::new();
        let mut rejections = Vec::new();
        for callable in &snapshot.affected_callables {
            let bridged = compiler_validated_test_call_identity(
                &self.repo,
                callable,
                test_symbol,
                &production_compilation,
                compilation,
                worker,
            );
            let bridged = match bridged {
                Ok(symbol) => symbol,
                Err(error) => {
                    rejections.push(CandidateOracleRejection {
                        callable_fingerprint: canonical::hash(callable).map_err(internal)?,
                        stage: candidate_oracle_failure_stage(&error),
                        code: error.code,
                        identity_comparison: candidate_identity_comparison(
                            &self.repo,
                            callable,
                            test_symbol,
                            &production_compilation,
                            compilation,
                            worker,
                        )?,
                    });
                    continue;
                }
            };
            match self.issue_behavioral_test(test_symbol, compilation, &bridged, None, worker) {
                Ok(receipt) => issued.push(receipt),
                Err(error)
                    if matches!(
                        error.code,
                        ErrorCode::IncompleteSemanticAnalysis
                            | ErrorCode::SymbolNotFound
                            | ErrorCode::AmbiguousSymbol
                    ) =>
                {
                    rejections.push(CandidateOracleRejection {
                        callable_fingerprint: canonical::hash(callable).map_err(internal)?,
                        stage: candidate_oracle_failure_stage(&error),
                        code: error.code,
                        identity_comparison: candidate_identity_comparison(
                            &self.repo,
                            callable,
                            test_symbol,
                            &production_compilation,
                            compilation,
                            worker,
                        )?,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        if issued.len() != 1 {
            let issued_count = issued.len();
            for receipt in issued {
                self.tests.remove(&receipt.receipt_id);
            }
            if snapshot.affected_callables.len() > 1 && issued_count > 1 {
                rejections.push(CandidateOracleRejection {
                    callable_fingerprint: canonical::hash(&snapshot.affected_callables)
                        .map_err(internal)?,
                    stage: CandidateOracleFailureStage::Ambiguous,
                    code: ErrorCode::AmbiguousTarget,
                    identity_comparison: json!({
                        "schema":"candidate-identity-comparison/0.1",
                        "status":"AMBIGUOUS",
                        "affectedCallableSetHash":canonical::hash(&snapshot.affected_callables).map_err(internal)?
                    }),
                });
            }
            if let Some(overlay) = self.candidate_overlays.get_mut(&overlay_receipt.receipt_id) {
                overlay.oracle_rejections = rejections.clone();
            }
            let mut refusal = ClewError::new(
                if snapshot.affected_callables.len() > 1 {
                    ErrorCode::AmbiguousTarget
                } else {
                    ErrorCode::IncompleteSemanticAnalysis
                },
                "selected assertion is not derived from exactly one affected callable result",
            );
            refusal
                .evidence
                .push(serde_json::to_string(&rejections).map_err(internal)?);
            return Err(refusal);
        }
        let receipt = issued.pop().expect("one candidate-scoped test");
        let test = self
            .tests
            .get(&receipt.receipt_id)
            .ok_or_else(|| invalid_receipt("candidate behavioral test"))?
            .clone();
        let test_project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":compilation}),
        )?;
        let production_project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":self.threads.values().find(|thread| thread.fingerprint == snapshot.thread_fingerprint).ok_or_else(|| invalid_receipt("candidate overlay thread"))?.thread.snapshot.compilation}),
        )?;
        if required_str(&production_project, "module")? != test.validation_route.module {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "candidate behavioral test belongs to a different production module",
            ));
        }
        let test_project_model_hash = required_str(&test_project, "projectModelHash")?.to_owned();
        let test_compile_task = required_str(&test_project, "compileTask")?.to_owned();
        if test_project_model_hash != snapshot.test_project_model_hash
            || test_compile_task != snapshot.test_compile_task
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "candidate behavioral test project model changed after overlay issuance",
            ));
        }
        let test_files = snapshot.test_files.clone();
        let route = test.validation_route.clone();
        let route_hash = canonical::hash(&route).map_err(internal)?;
        let scoped_fingerprint = canonical::hash(&(
            &test.fingerprint,
            overlay_receipt.receipt_id,
            &snapshot.overlay_hash,
            &snapshot.affected_callables,
            &test_project_model_hash,
            &test_compile_task,
            &route_hash,
        ))
        .map_err(internal)?;
        let finalized_hash = canonical::hash(&(
            &snapshot.revision,
            &snapshot.thread_fingerprint,
            &scoped_fingerprint,
            &snapshot.production_project_model_hash,
            &snapshot.test_compilation,
            &test_project_model_hash,
            &test_compile_task,
            &route_hash,
            &snapshot.candidates,
            &snapshot.production_files,
            &test_files,
            &snapshot.affected_callables,
        ))
        .map_err(internal)?;
        let overlay = self
            .candidate_overlays
            .get_mut(&overlay_receipt.receipt_id)
            .ok_or_else(|| invalid_receipt("candidate overlay"))?;
        overlay.test_fingerprint = Some(scoped_fingerprint.clone());
        overlay.route = Some(route);
        overlay.overlay_hash = finalized_hash.clone();
        let test = self
            .tests
            .get_mut(&receipt.receipt_id)
            .ok_or_else(|| invalid_receipt("candidate behavioral test"))?;
        test.fingerprint = scoped_fingerprint;
        test.candidate_overlay_receipt_id = Some(overlay_receipt.receipt_id);
        test.candidate_overlay_hash = Some(finalized_hash);
        Ok(receipt)
    }

    /// Runs the same compiler-derived route in candidate and omission
    /// worktrees. The omission retains all test-source writes but omits every
    /// production-source write classified by the authority.
    pub fn run_differential_validation(
        &mut self,
        receipt: &CandidateOverlayReceipt,
        worker: &mut WorkerClient,
    ) -> Result<DifferentialValidationReceipt, ClewError> {
        self.ensure_revision()?;
        if receipt.session_id != self.session_id {
            return Err(wrong_session("candidate overlay"));
        }
        let overlay = self
            .candidate_overlays
            .get(&receipt.receipt_id)
            .ok_or_else(|| invalid_receipt("candidate overlay"))?
            .clone();
        let test_fingerprint = overlay
            .test_fingerprint
            .as_ref()
            .ok_or_else(|| invalid_receipt("candidate overlay test binding"))?;
        let test_compile_task = &overlay.test_compile_task;
        let route = overlay
            .route
            .as_ref()
            .ok_or_else(|| invalid_receipt("candidate overlay validation route"))?;
        let thread = self
            .threads
            .values()
            .find(|item| item.fingerprint == overlay.thread_fingerprint)
            .ok_or_else(|| invalid_receipt("candidate overlay thread"))?;
        let test = self
            .tests
            .values()
            .find(|item| item.fingerprint == *test_fingerprint)
            .ok_or_else(|| invalid_receipt("candidate overlay test"))?;
        if test.candidate_overlay_receipt_id != Some(receipt.receipt_id)
            || test.candidate_overlay_hash.as_deref() != Some(overlay.overlay_hash.as_str())
        {
            return Err(invalid_receipt("candidate-scoped behavioral test"));
        }
        verify_sources_current(&self.repo, &thread.source_files)?;
        verify_sources_current(&self.repo, &test.source_files)?;
        let current_production = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":thread.thread.snapshot.compilation}),
        )?;
        let current_test = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":route.compilation}),
        )?;
        let current_route = validation_route(
            &route.compilation,
            &current_test,
            &test.class_name,
            &test.test_name,
        )?;
        validate_differential_overlay_state(
            &overlay,
            &self.revision,
            required_str(&current_production, "projectModelHash")?,
            required_str(&current_test, "projectModelHash")?,
            required_str(&current_test, "compileTask")?,
            &current_route,
        )?;

        let temporary = tempfile::tempdir().map_err(internal)?;
        let candidate = temporary.path().join("candidate");
        let omission = temporary.path().join("omission");
        evidence_git(
            &self.repo,
            &[
                "worktree",
                "add",
                "--detach",
                candidate
                    .to_str()
                    .ok_or_else(|| invalid_source("candidate worktree path is not UTF-8"))?,
                &self.revision,
            ],
        )?;
        if let Err(error) = evidence_git(
            &self.repo,
            &[
                "worktree",
                "add",
                "--detach",
                omission
                    .to_str()
                    .ok_or_else(|| invalid_source("omission worktree path is not UTF-8"))?,
                &self.revision,
            ],
        ) {
            let _ = evidence_git(
                &self.repo,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    candidate.to_str().unwrap_or_default(),
                ],
            );
            return Err(error);
        }
        let validation = (|| {
            apply_candidate_overlay(&candidate, &overlay, true)?;
            apply_candidate_overlay(&omission, &overlay, false)?;
            let candidate_compile = transaction::validate_worktree_fresh(
                &candidate,
                route.build_system,
                &route.build_launcher,
                test_compile_task,
                &[],
            )?;
            prepare_validation_report_root(&candidate, route)?;
            let candidate_test = transaction::run_test_lifecycle_fresh(
                &candidate,
                route.build_system,
                &route.build_launcher,
                &route.invocation,
            )?;
            let candidate_artifact = test_artifact_observed(&candidate, &[test], route)?;
            if candidate_artifact.executed_test_count == 0 {
                return Err(ClewError::new(
                    ErrorCode::TestFailed,
                    "candidate exact compiler-linked test did not execute",
                ));
            }

            let omission_compile = transaction::validate_worktree_fresh(
                &omission,
                route.build_system,
                &route.build_launcher,
                test_compile_task,
                &[],
            )?;
            prepare_validation_report_root(&omission, route)?;
            let omission_test = transaction::run_test_lifecycle_fresh(
                &omission,
                route.build_system,
                &route.build_launcher,
                &route.invocation,
            )?;
            let omission_artifact = test_artifact_observed(&omission, &[test], route)?;
            require_differential_outcomes(
                true,
                candidate_test.success,
                &candidate_artifact.selected_outcomes,
                true,
                omission_test.success,
                &omission_artifact.selected_outcomes,
            )?;
            Ok((
                candidate_compile,
                candidate_test,
                candidate_artifact,
                omission_compile,
                omission_test,
                omission_artifact,
            ))
        })();
        let _ = evidence_git(
            &self.repo,
            &[
                "worktree",
                "remove",
                "--force",
                candidate.to_str().unwrap_or_default(),
            ],
        );
        let _ = evidence_git(
            &self.repo,
            &[
                "worktree",
                "remove",
                "--force",
                omission.to_str().unwrap_or_default(),
            ],
        );
        let (
            candidate_compile,
            candidate_test,
            candidate_artifact,
            omission_compile,
            omission_test,
            omission_artifact,
        ) = validation?;
        let route_hash = canonical::hash(route).map_err(internal)?;
        let summary = DifferentialValidationSummary {
            schema: "differential-validation-summary/0.1".into(),
            revision: self.revision.clone(),
            overlay_hash: overlay.overlay_hash,
            route_hash,
            production_write_count: overlay.production_files.len(),
            test_write_count: overlay.test_files.len(),
            candidate_artifact_hash: candidate_artifact.artifact_hash,
            omission_artifact_hash: omission_artifact.artifact_hash,
            candidate_compile_duration_ms: candidate_compile.0,
            candidate_test_duration_ms: candidate_test.duration_ms,
            omission_compile_duration_ms: omission_compile.0,
            omission_test_duration_ms: omission_test.duration_ms,
        };
        let receipt_id = Uuid::new_v4();
        self.differential_validations.insert(
            receipt_id,
            DifferentialValidationRun {
                overlay_receipt_id: receipt.receipt_id,
                summary: summary.clone(),
            },
        );
        Ok(DifferentialValidationReceipt {
            session_id: self.session_id,
            receipt_id,
            summary,
        })
    }

    pub fn recognizes_differential_validation(
        &self,
        receipt: &DifferentialValidationReceipt,
    ) -> Result<bool, ClewError> {
        self.ensure_revision()?;
        if receipt.session_id != self.session_id {
            return Err(wrong_session("differential validation"));
        }
        Ok(self
            .differential_validations
            .get(&receipt.receipt_id)
            .is_some_and(|stored| {
                stored.summary == receipt.summary
                    && self
                        .candidate_overlays
                        .contains_key(&stored.overlay_receipt_id)
            }))
    }

    /// Joins worker/source evidence and validation evidence. This does not yet
    /// claim a structural-family theorem; it is the non-forgeable prerequisite
    /// that the rejected COMPLETE_FOR implementation lacked.
    pub fn authorize_bundle(
        &self,
        receipts: &[&VerifiedThreadReceipt],
        tests: &[&VerifiedBehavioralTestReceipt],
        validation: &ValidationReceipt,
    ) -> Result<AuthoritativeEvidenceBundle, ClewError> {
        self.ensure_revision()?;
        if validation.session_id != self.session_id {
            return Err(wrong_session("validation"));
        }
        let verified = self.resolve_threads(receipts)?;
        let verified_tests = self.resolve_tests(tests)?;
        for item in &verified {
            verify_sources_current(&self.repo, &item.source_files)?;
        }
        for test in &verified_tests {
            verify_sources_current(&self.repo, &test.source_files)?;
        }
        let thread_set_fingerprint = thread_set_fingerprint(&verified)?;
        let test_set_fingerprint = test_set_fingerprint(&verified_tests)?;
        let run = self
            .validations
            .get(&validation.receipt_id)
            .ok_or_else(|| invalid_receipt("validation"))?;
        if run.thread_set_fingerprint != thread_set_fingerprint {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "validation receipt covers a different exact thread set",
            ));
        }
        if run.test_set_fingerprint != test_set_fingerprint {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "validation receipt covers a different exact behavioral-test set",
            ));
        }
        let evidence_fingerprint = canonical::hash(&(
            &self.session_id,
            &self.revision,
            &thread_set_fingerprint,
            &test_set_fingerprint,
            &run.artifact_hash,
            &run.report_route_fingerprint,
        ))
        .map_err(internal)?;
        Ok(AuthoritativeEvidenceBundle {
            summary: EvidenceBundleSummary {
                schema: "authoritative-semantic-evidence/0.1".into(),
                revision: self.revision.clone(),
                thread_count: verified.len(),
                behavioral_test_count: verified_tests.len(),
                evidence_fingerprint,
                validation_artifact_hash: run.artifact_hash.clone(),
                executed_test_count: run.executed_test_count,
                compile_duration_ms: run.compile_duration_ms,
                test_duration_ms: run.test_duration_ms,
            },
        })
    }

    /// Proves the narrow producer-transform-consumer family from an exact
    /// worker-issued data-flow chain and the validation receipt for the same
    /// thread set. No role names or edge labels are accepted from the caller.
    pub fn complete_for_producer_transform_consumer(
        &mut self,
        goal: &ProducerTransformConsumerGoal,
        receipts: &[&VerifiedThreadReceipt],
        tests: &[&VerifiedBehavioralTestReceipt],
        validation: &ValidationReceipt,
    ) -> Result<CompleteForReceipt, ClewError> {
        if !goal.is_valid_for(&self.revision) {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "producer-transform-consumer goal does not match authority revision",
            ));
        }
        let bundle = self.authorize_bundle(receipts, tests, validation)?;
        let verified = self.resolve_threads(receipts)?;
        let verified_tests = self.resolve_tests(tests)?;
        let mut candidates = verified
            .iter()
            .flat_map(|item| {
                producer_transform_consumer_candidates(&item.thread)
                    .into_iter()
                    .map(|(producer, transformer, consumer)| {
                        (item.fingerprint.clone(), producer, transformer, consumer)
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        let [binding] = candidates.as_slice() else {
            return Err(ClewError::new(
                if candidates.is_empty() {
                    ErrorCode::IncompleteSemanticAnalysis
                } else {
                    ErrorCode::AmbiguousTarget
                },
                if candidates.is_empty() {
                    "worker evidence has no complete producer-transform-consumer chain"
                } else {
                    "worker evidence has multiple producer-transform-consumer chains"
                },
            ));
        };
        let bound_thread = verified
            .iter()
            .find(|item| item.fingerprint == binding.0)
            .ok_or_else(|| invalid_source("bound producer has no verified thread"))?;
        let target_compiler_symbol = compiler_owner_symbol(&bound_thread.thread, &binding.1)?;
        if !verified_tests
            .iter()
            .any(|test| test.target_compiler_symbol == target_compiler_symbol)
        {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "no verified behavioral test asserts the bound production callable",
            ));
        }
        let goal_fingerprint = canonical::hash(goal).map_err(internal)?;
        let evidence_fingerprint = canonical::hash(&(
            bundle.summary(),
            ProvenStructuralFamily::ProducerTransformConsumer,
            &goal_fingerprint,
            binding,
        ))
        .map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        self.completions.insert(receipt_id);
        Ok(CompleteForReceipt {
            session_id: self.session_id,
            receipt_id,
            summary: CompleteForSummary {
                schema: "complete-for-authority/0.1".into(),
                family: ProvenStructuralFamily::ProducerTransformConsumer,
                revision: self.revision.clone(),
                producer_node: binding.1.clone(),
                transformer_node: binding.2.clone(),
                consumer_node: binding.3.clone(),
                goal_fingerprint,
                evidence_fingerprint,
            },
        })
    }

    /// Computes a source-free MAP_EDGE_WITH_CONTEXT plan from live compiler
    /// evidence. The model supplies only the typed goal and a test candidate;
    /// bindings, placement and preservation invariants are authority-owned.
    pub fn bind_map_edge_with_context(
        &mut self,
        goal: &SemanticGoal,
        workflow: &VerifiedThreadReceipt,
        test_symbol: &str,
        test_compilation: &str,
        worker: &mut WorkerClient,
    ) -> Result<MapEdgeWithContextDecision, ClewError> {
        self.ensure_revision()?;
        if goal.validate().is_err() || goal.family != GoalFamily::MapEdgeWithContext {
            return Ok(map_edge_refused(MapEdgeRefusalReason::InvalidGoal));
        }
        if goal.base_revision != self.revision {
            return Ok(map_edge_refused(MapEdgeRefusalReason::SnapshotMismatch));
        }
        let (thread, thread_fingerprint) = {
            let verified = self.resolve_threads(&[workflow])?;
            let [workflow] = verified.as_slice() else {
                unreachable!("one receipt resolves to one thread")
            };
            (workflow.thread.clone(), workflow.fingerprint.clone())
        };
        let edge = match map_value_edge(&thread) {
            Ok(edge) => edge,
            Err(reason) => return Ok(map_edge_refused(reason)),
        };
        let index = worker.request(
            RequestKind::IndexFiles,
            &json!({"repo":self.repo,"compilation":thread.snapshot.compilation,"syntaxOnly":false}),
        )?;
        if index.get("k2Validated").and_then(Value::as_bool) != Some(true)
            || has_error_diagnostic(&index)
        {
            return Ok(map_edge_refused(MapEdgeRefusalReason::UnsupportedBoundary));
        }
        let structural_candidates = discover_map_candidates(&index, &edge)?;
        if structural_candidates.is_empty() {
            return Ok(map_edge_refused(
                MapEdgeRefusalReason::NoCompatibleContextAndTransformer,
            ));
        }
        let mut safe_candidates = Vec::new();
        for (context, transformer) in structural_candidates {
            let context_resolution =
                resolve_safe_callable(&self.repo, &thread.snapshot.compilation, &context, worker)?;
            let transformer_resolution = resolve_safe_callable(
                &self.repo,
                &thread.snapshot.compilation,
                &transformer,
                worker,
            )?;
            let (Some(context_resolution_hash), Some(transformer_resolution_hash)) =
                (context_resolution, transformer_resolution)
            else {
                continue;
            };
            safe_candidates.push(MapCandidate {
                context,
                transformer,
                context_resolution_hash,
                transformer_resolution_hash,
            });
        }
        if safe_candidates.is_empty() {
            return Ok(map_edge_refused(MapEdgeRefusalReason::UnknownEffects));
        }
        safe_candidates.sort_by(|left, right| {
            (
                &left.context.compiler_symbol,
                &left.transformer.compiler_symbol,
            )
                .cmp(&(
                    &right.context.compiler_symbol,
                    &right.transformer.compiler_symbol,
                ))
        });
        safe_candidates.dedup_by(|left, right| {
            left.context.compiler_symbol == right.context.compiler_symbol
                && left.transformer.compiler_symbol == right.transformer.compiler_symbol
        });
        if safe_candidates.len() != 1 {
            return Ok(MapEdgeWithContextDecision::Ambiguous(MapEdgeAmbiguity {
                schema: "map-edge-with-context-decision/0.1".into(),
                status: "AMBIGUOUS".into(),
                choices: safe_candidates
                    .into_iter()
                    .map(|candidate| MapEdgeChoice {
                        context_producer_symbol: candidate.context.compiler_symbol,
                        transformer_symbol: candidate.transformer.compiler_symbol,
                        element_type: edge.element_type.clone(),
                        context_type: candidate.context.return_type,
                    })
                    .collect(),
            }));
        }
        let candidate = safe_candidates.pop().expect("one candidate");
        let bindings = MapEdgeBindingSummary {
            workflow_symbol: edge.workflow_symbol.clone(),
            context_producer_symbol: candidate.context.compiler_symbol.clone(),
            transformer_symbol: candidate.transformer.compiler_symbol.clone(),
            value_edge_from: edge.from.clone(),
            value_edge_to: edge.to.clone(),
            value_parameter_index: edge.parameter_index,
            placement: edge.placement.clone(),
            collection_type: edge.collection_type.clone(),
            element_type: edge.element_type.clone(),
            context_type: candidate.context.return_type.clone(),
            strategy: "KOTLIN_EAGER_LIST_MAP_WITH_CONTEXT_ONCE".into(),
        };
        transaction::validate_worktree(
            &self.repo,
            thread.snapshot.build_system,
            &thread.snapshot.build_launcher,
            &thread.snapshot.compile_task,
            &[],
        )?;
        let test = match self.issue_behavioral_test(
            test_symbol,
            test_compilation,
            &candidate.transformer.compiler_symbol,
            Some(&candidate.context.compiler_symbol),
            worker,
        ) {
            Ok(test) => test,
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::IncompleteSemanticAnalysis
                        | ErrorCode::SymbolNotFound
                        | ErrorCode::AmbiguousSymbol
                ) =>
            {
                if let Ok(resolution) = worker.request(
                    RequestKind::ResolveSymbol,
                    &json!({"repo":self.repo,"compilation":test_compilation,"symbol":test_symbol}),
                ) && let Some(conditional_oracle) = conditional_oracle_evidence(
                    &resolution,
                    &candidate.transformer.compiler_symbol,
                    &candidate.context.compiler_symbol,
                )? {
                    let goal_fingerprint = canonical::hash(goal).map_err(internal)?;
                    let base_evidence = canonical::hash(&(
                        &thread_fingerprint,
                        index.get("indexHash"),
                        &candidate.context_resolution_hash,
                        &candidate.transformer_resolution_hash,
                        &bindings,
                        &conditional_oracle.evidence_fingerprint,
                    ))
                    .map_err(internal)?;
                    let mut invariants = map_edge_invariants(&base_evidence, &bindings)?;
                    invariants.retain(|proof| {
                        proof.invariant != MapEdgeInvariant::BehavioralOracleAvailable
                    });
                    let mut change_graph = map_edge_change_graph(goal, &bindings, &invariants);
                    if let Some(obligation) = change_graph
                        .obligations
                        .iter_mut()
                        .find(|obligation| obligation.kind == ObligationKind::RequireOracle)
                    {
                        obligation.status = DischargeStatus::Unproved;
                        obligation.evidence.clear();
                    }
                    let unresolved_obligations = conditional_oracle_obligations(
                        &conditional_oracle,
                        vec![
                            candidate.transformer.compiler_symbol.clone(),
                            candidate.context.compiler_symbol.clone(),
                        ],
                    );
                    let evidence_fingerprint = canonical::hash(&(
                        &goal_fingerprint,
                        &base_evidence,
                        &invariants,
                        &change_graph,
                        &unresolved_obligations,
                    ))
                    .map_err(internal)?;
                    return Ok(MapEdgeWithContextDecision::Conditional(
                        MapEdgeConditional {
                            schema: "map-edge-with-context-decision/0.2".into(),
                            status: "CONDITIONAL".into(),
                            revision: self.revision.clone(),
                            goal_fingerprint,
                            bindings,
                            established_invariants: invariants,
                            change_graph,
                            unresolved_obligations,
                            evidence_fingerprint,
                        },
                    ));
                }
                return Ok(map_edge_refused(
                    MapEdgeRefusalReason::MissingBehavioralOracle,
                ));
            }
            Err(error) => return Err(error),
        };
        let validation = self.run_validation(&[workflow], &[&test], worker)?;
        let bundle = self.authorize_bundle(&[workflow], &[&test], &validation)?;
        let goal_fingerprint = canonical::hash(goal).map_err(internal)?;
        let base_evidence = canonical::hash(&(
            &thread_fingerprint,
            index.get("indexHash"),
            &candidate.context_resolution_hash,
            &candidate.transformer_resolution_hash,
            bundle.summary(),
            &bindings,
        ))
        .map_err(internal)?;
        let invariants = map_edge_invariants(&base_evidence, &bindings)?;
        let change_graph = map_edge_change_graph(goal, &bindings, &invariants);
        change_graph
            .validate_closure()
            .map_err(|error| internal(format!("invalid authority change graph: {error:?}")))?;
        let evidence_fingerprint = canonical::hash(&(
            &goal_fingerprint,
            &base_evidence,
            &invariants,
            &change_graph,
        ))
        .map_err(internal)?;
        let summary = MapEdgeProofSummary {
            schema: "map-edge-with-context-proof/0.1".into(),
            revision: self.revision.clone(),
            goal_fingerprint,
            bindings,
            invariants,
            change_graph,
            evidence_fingerprint,
        };
        let receipt_id = Uuid::new_v4();
        self.map_edge_proofs.insert(
            receipt_id,
            AuthorizedMapEdgeProof {
                thread_fingerprint,
                summary: summary.clone(),
            },
        );
        Ok(MapEdgeWithContextDecision::Bound(Box::new(
            MapEdgeWithContextReceipt {
                session_id: self.session_id,
                receipt_id,
                summary,
            },
        )))
    }

    /// Binds a family-neutral typed goal from repository-wide live evidence.
    /// Hints are accepted only as advisory telemetry: removing or changing
    /// them cannot change candidate discovery, proof closure or authority.
    pub fn bind_typed_goal(
        &mut self,
        goal: &TypedSemanticGoal,
        hints: &[String],
        compilation: Option<&str>,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        let plan = match preflight_typed_goal(goal) {
            Ok(plan) => plan,
            Err(reason) => return Ok(typed_goal_refused(reason)),
        };
        self.bind_typed_goal_inner(goal, hints, compilation, None, plan, worker)
    }

    pub fn bind_typed_goal_with_external_spec(
        &mut self,
        request: &TypedGoalBindingRequest,
        compilation: Option<&str>,
        receipt: &ExternalSpecReceipt,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        // Language/domain preflight is pure and deliberately precedes all
        // authority/session/repository receipt validation. Unsupported or
        // caller-rooted auxiliary operators cannot turn a forged handle into
        // an observable authority lookup or stored proof.
        let plan = match preflight_typed_goal(&request.goal) {
            Ok(plan) => plan,
            Err(reason) => return Ok(typed_goal_refused(reason)),
        };
        let external_spec =
            match self.revalidate_external_spec(receipt, request, compilation, worker) {
                Ok(proof) => proof,
                Err(_) => {
                    return Ok(typed_goal_refused(
                        TypedGoalRefusalReason::ExternalSpecificationMismatch,
                    ));
                }
            };
        self.bind_typed_goal_inner(
            &request.goal,
            &request.hints,
            compilation,
            Some(external_spec),
            plan,
            worker,
        )
    }

    fn bind_typed_goal_inner(
        &mut self,
        goal: &TypedSemanticGoal,
        _hints: &[String],
        compilation: Option<&str>,
        external_spec: Option<ExternalSpecProof>,
        plan: crate::semantic_goal::ConstraintExecutionPlan,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        self.ensure_revision()?;
        if goal.base_revision != self.revision {
            return Ok(typed_goal_refused(TypedGoalRefusalReason::SnapshotMismatch));
        }
        let domain = match executable_plan_domain(&plan) {
            Ok(domain) => domain,
            Err(reason) => return Ok(typed_goal_refused(reason)),
        };
        match domain {
            ConstraintDomain::ValueFlow => self.bind_value_flow_worklist(
                goal,
                &plan,
                compilation,
                external_spec.as_ref(),
                worker,
            ),
            ConstraintDomain::DeclarationChange => self.bind_declaration_change_worklist(
                goal,
                &plan,
                compilation,
                external_spec.as_ref(),
                worker,
            ),
            ConstraintDomain::ResourceLifetime => Ok(typed_goal_refused(
                TypedGoalRefusalReason::UnsupportedConstraintDomain,
            )),
            ConstraintDomain::NullableConstruction => Ok(typed_goal_refused(
                TypedGoalRefusalReason::UnsupportedConstraintDomain,
            )),
            ConstraintDomain::Projection => Ok(typed_goal_refused(
                TypedGoalRefusalReason::UnsupportedConstraintDomain,
            )),
        }
    }

    fn bind_declaration_change_worklist(
        &mut self,
        goal: &TypedSemanticGoal,
        plan: &crate::semantic_goal::ConstraintExecutionPlan,
        compilation: Option<&str>,
        external_spec: Option<&ExternalSpecProof>,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        let Some(external_spec) = external_spec else {
            return Ok(typed_goal_refused_with_declaration_rejection(
                TypedGoalRefusalReason::MissingExternalSpecification,
                declaration_provider_rejection(
                    DeclarationProviderStage::ExternalSpec,
                    ErrorCode::PreconditionFailed,
                    &(goal, compilation),
                ),
            ));
        };
        if plan
            .mandatory_closure
            .iter()
            .any(|application| application.operator == PrimitiveConstraint::NullHandles)
        {
            return self.bind_nullable_construction_worklist(
                goal,
                plan,
                compilation,
                external_spec,
                worker,
            );
        }
        if plan
            .mandatory_closure
            .iter()
            .any(|application| application.operator == PrimitiveConstraint::ProjectsValue)
        {
            return self.bind_projection_consumer_worklist(
                goal,
                plan,
                compilation,
                external_spec,
                worker,
            );
        }
        let candidates =
            match discover_declaration_type_candidates(&self.repo, compilation, plan, worker) {
                Ok(candidates) => candidates,
                Err((reason, rejection)) => {
                    return Ok(typed_goal_refused_with_declaration_rejection(
                        reason, rejection,
                    ));
                }
            };
        let mut states = vec![BTreeMap::<String, String>::new()];
        let mut producers = plan
            .mandatory_closure
            .iter()
            .filter(|application| {
                application.operator == PrimitiveConstraint::PropagateDeclaredType
            })
            .collect::<Vec<_>>();
        producers.sort();
        for application in producers {
            let mut application_candidates = Vec::new();
            for candidate in &candidates {
                application_candidates.push(BTreeMap::from([
                    (
                        application.operands[0].clone(),
                        candidate.source_symbol.clone(),
                    ),
                    (
                        application.operands[1].clone(),
                        candidate.target_symbol.clone(),
                    ),
                ]));
            }
            if application_candidates.is_empty() {
                return Ok(typed_goal_refused(
                    TypedGoalRefusalReason::NoCompatibleBindings,
                ));
            }
            let mut next = Vec::new();
            for state in &states {
                for candidate in &application_candidates {
                    if let Some(merged) = merge_operator_bindings(state, candidate) {
                        next.push(merged);
                    }
                }
            }
            next.sort();
            next.dedup();
            if next.is_empty() {
                return Ok(typed_goal_refused(
                    TypedGoalRefusalReason::NoCompatibleBindings,
                ));
            }
            states = next;
        }
        let declared = goal.variables.keys().cloned().collect::<BTreeSet<_>>();
        states.retain(|state| state.keys().cloned().collect::<BTreeSet<_>>() == declared);
        if states.is_empty() {
            return Ok(typed_goal_refused(
                TypedGoalRefusalReason::UnsupportedOperatorComposition,
            ));
        }
        if states.len() != 1 {
            return Ok(TypedGoalBindingDecision::Ambiguous(TypedGoalAmbiguity {
                schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
                status: "AMBIGUOUS".into(),
                choices: states
                    .into_iter()
                    .map(|bindings| TypedGoalChoice { bindings })
                    .collect(),
            }));
        }
        let viable_bindings = states.clone();
        let bindings = states.pop().unwrap();
        let Some(candidate) = candidates.iter().find(|candidate| {
            plan.mandatory_closure.iter().any(|application| {
                application.operator == PrimitiveConstraint::PropagateDeclaredType
                    && bindings.get(&application.operands[0]) == Some(&candidate.source_symbol)
                    && bindings.get(&application.operands[1]) == Some(&candidate.target_symbol)
            })
        }) else {
            return Ok(typed_goal_refused(
                TypedGoalRefusalReason::NoCompatibleBindings,
            ));
        };
        let provider = DeclarationTypeOperatorEvidenceProvider {
            bindings: &bindings,
            viable_bindings: &viable_bindings,
            candidate,
            external_spec,
        };
        let relation_records = match prove_relation_records(&provider, plan, &bindings) {
            Ok(records) => records,
            Err(reason) => {
                let rejection = declaration_provider_rejection(
                    DeclarationProviderStage::PerOperatorEvaluation,
                    ErrorCode::IncompleteSemanticAnalysis,
                    &(&plan.mandatory_closure, &bindings, reason),
                );
                return Ok(typed_goal_refused_with_declaration_rejection(
                    reason, rejection,
                ));
            }
        };
        let evidence_seed = canonical::hash(&(
            &candidate.propagation_fingerprint,
            &candidate.override_fingerprint,
            &candidate.use_closure_fingerprint,
            &candidate.contract_fingerprint,
            &candidate.boundary_closure_fingerprint,
            external_spec,
            &relation_records,
        ))
        .map_err(internal)?;
        self.issue_typed_goal_receipt(
            goal,
            plan,
            bindings,
            relation_records,
            &evidence_seed,
            Some(external_spec.clone()),
        )
    }

    fn bind_nullable_construction_worklist(
        &mut self,
        goal: &TypedSemanticGoal,
        plan: &crate::semantic_goal::ConstraintExecutionPlan,
        compilation: Option<&str>,
        external_spec: &ExternalSpecProof,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        let candidates =
            match discover_nullable_construction_candidates(&self.repo, compilation, worker) {
                Ok(candidates) => candidates,
                Err(error) => {
                    let reason = if error.code == ErrorCode::StaleRequiresReslice {
                        TypedGoalRefusalReason::SnapshotMismatch
                    } else {
                        TypedGoalRefusalReason::InsufficientEvidence
                    };
                    let error_code = error.code.clone();
                    let diagnostic = error.evidence.iter().find_map(|value| {
                        serde_json::from_str::<NullableDiscoveryDiagnostic>(value).ok()
                    });
                    let mut rejection = declaration_provider_rejection(
                        diagnostic
                            .as_ref()
                            .map_or(DeclarationProviderStage::GraphCoverage, |value| value.stage),
                        error_code.clone(),
                        &(error_code, canonical::hash(&error.message).ok()),
                    );
                    rejection.candidate_cardinality = Some(0);
                    if let Some(diagnostic) = diagnostic {
                        rejection.type_shapes = Some(diagnostic.shapes);
                        rejection.fact_cardinalities = diagnostic.cardinalities;
                        rejection.range_relations = diagnostic.range_relations;
                        rejection.fact_relations = diagnostic.fact_relations;
                    }
                    return Ok(typed_goal_refused_with_declaration_rejection(
                        reason, rejection,
                    ));
                }
            };
        let mut states = vec![BTreeMap::<String, String>::new()];
        let mut producers = plan
            .mandatory_closure
            .iter()
            .filter(|application| application.operator == PrimitiveConstraint::NullHandles)
            .collect::<Vec<_>>();
        producers.sort();
        for application in producers {
            let application_candidates = candidates
                .iter()
                .map(|candidate| {
                    BTreeMap::from([
                        (
                            application.operands[0].clone(),
                            candidate.source_symbol.clone(),
                        ),
                        (
                            application.operands[1].clone(),
                            candidate.fallback_symbol.clone(),
                        ),
                        (
                            application.operands[2].clone(),
                            candidate.destination_symbol.clone(),
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            if application_candidates.is_empty() {
                let mut rejection = declaration_provider_rejection(
                    DeclarationProviderStage::NullPolicy,
                    ErrorCode::IncompleteSemanticAnalysis,
                    &(application.operator.clone(), "zero-candidates"),
                );
                rejection.candidate_cardinality = Some(0);
                return Ok(typed_goal_refused_with_declaration_rejection(
                    TypedGoalRefusalReason::NoCompatibleBindings,
                    rejection,
                ));
            }
            let mut next = Vec::new();
            for state in &states {
                for candidate in &application_candidates {
                    if let Some(merged) = merge_operator_bindings(state, candidate) {
                        next.push(merged);
                    }
                }
            }
            next.sort();
            next.dedup();
            states = next;
        }
        let declared = goal.variables.keys().cloned().collect::<BTreeSet<_>>();
        states.retain(|state| state.keys().cloned().collect::<BTreeSet<_>>() == declared);
        if states.is_empty() {
            return Ok(typed_goal_refused(
                TypedGoalRefusalReason::UnsupportedOperatorComposition,
            ));
        }
        if states.len() != 1 {
            return Ok(TypedGoalBindingDecision::Ambiguous(TypedGoalAmbiguity {
                schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
                status: "AMBIGUOUS".into(),
                choices: states
                    .into_iter()
                    .map(|bindings| TypedGoalChoice { bindings })
                    .collect(),
            }));
        }
        let viable_bindings = states.clone();
        let bindings = states.pop().unwrap();
        let candidate = candidates
            .iter()
            .find(|candidate| {
                plan.mandatory_closure.iter().any(|application| {
                    application.operator == PrimitiveConstraint::NullHandles
                        && bindings.get(&application.operands[0]) == Some(&candidate.source_symbol)
                        && bindings.get(&application.operands[1])
                            == Some(&candidate.fallback_symbol)
                        && bindings.get(&application.operands[2])
                            == Some(&candidate.destination_symbol)
                })
            })
            .ok_or_else(|| invalid_source("nullable construction binding has no candidate"))?;
        let provider = NullableConstructionOperatorEvidenceProvider {
            bindings: &bindings,
            viable_bindings: &viable_bindings,
            candidate,
            external_spec,
        };
        let mut relation_records = match prove_relation_records(&provider, plan, &bindings) {
            Ok(records) => records,
            Err(reason) => {
                let mut rejection = declaration_provider_rejection(
                    DeclarationProviderStage::PerOperatorEvaluation,
                    ErrorCode::IncompleteSemanticAnalysis,
                    &(&plan.mandatory_closure, &bindings, &reason),
                );
                rejection.candidate_cardinality = Some(viable_bindings.len());
                return Ok(typed_goal_refused_with_declaration_rejection(
                    reason, rejection,
                ));
            }
        };
        for record in &mut relation_records {
            record.occurrence_count = Some(candidate.occurrences.len());
            record.occurrence_set_fingerprint = Some(candidate.occurrence_set_fingerprint.clone());
            record.occurrence_fingerprints = candidate.occurrence_fingerprints.clone();
        }
        let evidence_seed =
            canonical::hash(&(candidate, external_spec, &relation_records)).map_err(internal)?;
        self.issue_typed_goal_receipt(
            goal,
            plan,
            bindings,
            relation_records,
            &evidence_seed,
            Some(external_spec.clone()),
        )
    }

    fn bind_projection_consumer_worklist(
        &mut self,
        goal: &TypedSemanticGoal,
        plan: &crate::semantic_goal::ConstraintExecutionPlan,
        compilation: Option<&str>,
        external_spec: &ExternalSpecProof,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        let candidates =
            match discover_projection_consumer_candidates(&self.repo, compilation, worker) {
                Ok(candidates) => candidates,
                Err(error) => {
                    let reason = if error.code == ErrorCode::StaleRequiresReslice {
                        TypedGoalRefusalReason::SnapshotMismatch
                    } else {
                        TypedGoalRefusalReason::InsufficientEvidence
                    };
                    let diagnostic = error.evidence.iter().find_map(|value| {
                        serde_json::from_str::<ProjectionDiscoveryDiagnostic>(value).ok()
                    });
                    let mut rejection = declaration_provider_rejection(
                        diagnostic
                            .as_ref()
                            .map_or(DeclarationProviderStage::GraphCoverage, |value| value.stage),
                        error.code,
                        &canonical::hash(&error.message).ok(),
                    );
                    if let Some(diagnostic) = diagnostic {
                        rejection.fact_cardinalities = diagnostic.counts;
                        rejection.fact_relations = diagnostic.hashes;
                        rejection.range_relations = diagnostic.shapes;
                    }
                    return Ok(typed_goal_refused_with_declaration_rejection(
                        reason, rejection,
                    ));
                }
            };
        let producers = plan
            .mandatory_closure
            .iter()
            .filter(|application| application.operator == PrimitiveConstraint::ProjectsValue)
            .collect::<Vec<_>>();
        let mut states = vec![BTreeMap::<String, String>::new()];
        for application in producers {
            let application_candidates = candidates
                .iter()
                .map(|candidate| {
                    BTreeMap::from([
                        (
                            application.operands[0].clone(),
                            candidate.source_symbol.clone(),
                        ),
                        (
                            application.operands[1].clone(),
                            candidate.projection_symbol.clone(),
                        ),
                        (
                            application.operands[2].clone(),
                            candidate.consumer_symbol.clone(),
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            let mut next = Vec::new();
            for state in &states {
                for candidate in &application_candidates {
                    if let Some(merged) = merge_operator_bindings(state, candidate) {
                        next.push(merged);
                    }
                }
            }
            next.sort();
            next.dedup();
            states = next;
        }
        let declared = goal.variables.keys().cloned().collect::<BTreeSet<_>>();
        states.retain(|state| state.keys().cloned().collect::<BTreeSet<_>>() == declared);
        if states.is_empty() {
            return Ok(typed_goal_refused(
                TypedGoalRefusalReason::NoCompatibleBindings,
            ));
        }
        if states.len() != 1 {
            return Ok(TypedGoalBindingDecision::Ambiguous(TypedGoalAmbiguity {
                schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
                status: "AMBIGUOUS".into(),
                choices: states
                    .into_iter()
                    .map(|bindings| TypedGoalChoice { bindings })
                    .collect(),
            }));
        }
        let viable_bindings = states.clone();
        let bindings = states.pop().unwrap();
        let candidate = candidates
            .iter()
            .find(|candidate| {
                plan.mandatory_closure.iter().any(|application| {
                    application.operator == PrimitiveConstraint::ProjectsValue
                        && bindings.get(&application.operands[0]) == Some(&candidate.source_symbol)
                        && bindings.get(&application.operands[1])
                            == Some(&candidate.projection_symbol)
                        && bindings.get(&application.operands[2])
                            == Some(&candidate.consumer_symbol)
                })
            })
            .ok_or_else(|| invalid_source("projection binding has no exact candidate"))?;
        let provider = ProjectionConsumerOperatorEvidenceProvider {
            bindings: &bindings,
            viable_bindings: &viable_bindings,
            candidate,
            external_spec,
        };
        let mut relation_records = match prove_relation_records(&provider, plan, &bindings) {
            Ok(records) => records,
            Err(reason) => return Ok(typed_goal_refused(reason)),
        };
        for record in &mut relation_records {
            record.occurrence_count = Some(candidate.occurrences.len());
            record.occurrence_set_fingerprint = Some(candidate.occurrence_set_fingerprint.clone());
            record.occurrence_fingerprints = candidate.occurrence_fingerprints.clone();
        }
        let evidence_seed =
            canonical::hash(&(candidate, external_spec, &relation_records)).map_err(internal)?;
        self.issue_typed_goal_receipt(
            goal,
            plan,
            bindings,
            relation_records,
            &evidence_seed,
            Some(external_spec.clone()),
        )
    }

    fn bind_value_flow_worklist(
        &mut self,
        goal: &TypedSemanticGoal,
        plan: &crate::semantic_goal::ConstraintExecutionPlan,
        compilation: Option<&str>,
        external_spec: Option<&ExternalSpecProof>,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        let flows = discover_value_flows(&self.repo, compilation, worker)?;
        let mut states = vec![BTreeMap::<String, String>::new()];
        let mut producers = plan
            .mandatory_closure
            .iter()
            .filter(|application| {
                matches!(
                    application.operator,
                    PrimitiveConstraint::MapEdge | PrimitiveConstraint::TypeAssignable
                )
            })
            .collect::<Vec<_>>();
        producers.sort();
        for application in producers {
            let candidates = operator_binding_candidates(application, &flows);
            if candidates.is_empty() {
                return Ok(typed_goal_refused(
                    TypedGoalRefusalReason::NoCompatibleBindings,
                ));
            }
            let mut next = Vec::new();
            for state in &states {
                for candidate in &candidates {
                    if let Some(merged) = merge_operator_bindings(state, candidate) {
                        next.push(merged);
                    }
                }
            }
            next.sort();
            next.dedup();
            if next.is_empty() {
                return Ok(typed_goal_refused(
                    TypedGoalRefusalReason::NoCompatibleBindings,
                ));
            }
            states = next;
        }
        let declared = goal.variables.keys().cloned().collect::<BTreeSet<_>>();
        states.retain(|state| state.keys().cloned().collect::<BTreeSet<_>>() == declared);
        if states.is_empty() {
            return Ok(typed_goal_refused(
                TypedGoalRefusalReason::UnsupportedOperatorComposition,
            ));
        }
        if states.len() != 1 {
            return Ok(TypedGoalBindingDecision::Ambiguous(TypedGoalAmbiguity {
                schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
                status: "AMBIGUOUS".into(),
                choices: states
                    .into_iter()
                    .map(|bindings| TypedGoalChoice { bindings })
                    .collect(),
            }));
        }
        let viable_bindings = states.clone();
        let bindings = states.pop().unwrap();
        let selected = select_flow_evidence(&bindings, plan, &flows);
        let Some(selected) = selected else {
            return Ok(typed_goal_refused(
                TypedGoalRefusalReason::NoCompatibleBindings,
            ));
        };
        let flow = &flows[selected.flow_index];

        let mut evidence_parts = vec![
            flow.thread_fingerprint.clone(),
            flow.index_hash.clone(),
            selected.transformer.resolution_fingerprint.clone(),
        ];
        let mut oracle_fingerprint = None;
        if plan
            .mandatory_closure
            .iter()
            .any(|application| application.operator == PrimitiveConstraint::RequireOracle)
        {
            let fingerprint = if let Some(proof) = external_spec {
                canonical::hash(proof).map_err(internal)?
            } else {
                let thread = flow.thread.clone();
                let verified = self.verify_thread(&thread, worker)?;
                transaction::validate_worktree(
                    &self.repo,
                    thread.snapshot.build_system,
                    &thread.snapshot.build_launcher,
                    &thread.snapshot.compile_task,
                    &[],
                )?;
                let test_compilation = authority_test_compilation(&thread.snapshot)?;
                let test_symbols = discover_test_symbols(&self.repo, &test_compilation, worker)?;
                let Some(map_candidate) = selected.map_candidate.as_ref() else {
                    return Ok(typed_goal_refused(
                        TypedGoalRefusalReason::InsufficientEvidence,
                    ));
                };
                let legacy_candidate = MapCandidate {
                    context: map_candidate.context.callable.clone(),
                    transformer: map_candidate.transformer.callable.clone(),
                    context_resolution_hash: map_candidate.context.resolution_fingerprint.clone(),
                    transformer_resolution_hash: map_candidate
                        .transformer
                        .resolution_fingerprint
                        .clone(),
                };
                let (test, rejections, conditional_oracle) = self.discover_behavioral_oracle(
                    &test_symbols,
                    &test_compilation,
                    &legacy_candidate,
                    worker,
                )?;
                let Some(test) = test else {
                    if let Some(conditional_oracle) = conditional_oracle {
                        let goal_fingerprint = canonical::hash(goal).map_err(internal)?;
                        let established_evidence_fingerprint = canonical::hash(&(
                            &flow.thread_fingerprint,
                            &flow.index_hash,
                            &selected.transformer.resolution_fingerprint,
                            &bindings,
                            &conditional_oracle.evidence_fingerprint,
                        ))
                        .map_err(internal)?;
                        return Ok(TypedGoalBindingDecision::Conditional(
                            TypedGoalConditional {
                                schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
                                status: "CONDITIONAL".into(),
                                revision: self.revision.clone(),
                                goal_fingerprint,
                                bindings,
                                established_evidence_fingerprint,
                                unresolved_obligations: conditional_oracle_obligations(
                                    &conditional_oracle,
                                    vec![
                                        map_candidate.transformer.callable.compiler_symbol.clone(),
                                        map_candidate.context.callable.compiler_symbol.clone(),
                                    ],
                                ),
                                rejections,
                            },
                        ));
                    }
                    return Ok(typed_goal_refused_with_rejections(
                        TypedGoalRefusalReason::MissingBehavioralOracle,
                        rejections,
                    ));
                };
                let validation = self.run_validation(&[&verified], &[&test], worker)?;
                let bundle = self.authorize_bundle(&[&verified], &[&test], &validation)?;
                bundle.summary().evidence_fingerprint.clone()
            };
            evidence_parts.push(fingerprint.clone());
            oracle_fingerprint = Some(fingerprint);
        }
        let provider = ValueFlowOperatorEvidenceProvider {
            bindings: &bindings,
            viable_bindings: &viable_bindings,
            flow,
            selected: &selected,
            oracle_fingerprint: oracle_fingerprint.as_deref(),
        };
        let relation_records = match prove_relation_records(&provider, plan, &bindings) {
            Ok(records) => records,
            Err(reason) => return Ok(typed_goal_refused(reason)),
        };
        let evidence_seed =
            canonical::hash(&(evidence_parts, &relation_records)).map_err(internal)?;
        self.issue_typed_goal_receipt(
            goal,
            plan,
            bindings,
            relation_records,
            &evidence_seed,
            external_spec.cloned(),
        )
    }

    fn discover_behavioral_oracle(
        &mut self,
        candidates: &[DiscoveredTestCandidate],
        compilation: &str,
        binding: &MapCandidate,
        worker: &mut WorkerClient,
    ) -> Result<
        (
            Option<VerifiedBehavioralTestReceipt>,
            Vec<OracleCandidateRejection>,
            Option<ConditionalOracleEvidence>,
        ),
        ClewError,
    > {
        let mut rejections = Vec::new();
        let mut conditional = BTreeMap::new();
        let diagnostic_context = oracle_compilation_context(&self.repo, compilation, worker)?;
        for test_candidate in candidates {
            for symbol in &test_candidate.queries {
                let identity_fingerprint = canonical::hash(symbol).map_err(internal)?;
                let resolution = match worker.request(
                    RequestKind::ResolveSymbol,
                    &json!({"repo":self.repo,"compilation":compilation,"symbol":symbol}),
                ) {
                    Ok(resolution) => resolution,
                    Err(error) => {
                        rejections.push(OracleCandidateRejection {
                            identity_fingerprint,
                            owner: test_candidate.owner.clone(),
                            stage: OracleRejectionStage::ResolveIdentity,
                            code: error.code,
                            compiler_diagnostic: None,
                        });
                        continue;
                    }
                };
                if resolution.get("k2Validated").and_then(Value::as_bool) != Some(true)
                    || has_error_diagnostic(&resolution)
                {
                    rejections.push(OracleCandidateRejection {
                        identity_fingerprint,
                        owner: test_candidate.owner.clone(),
                        stage: OracleRejectionStage::K2Validation,
                        code: ErrorCode::IncompleteSemanticAnalysis,
                        compiler_diagnostic: Some(oracle_compiler_diagnostic(
                            &diagnostic_context,
                            &test_candidate.compiler_identity,
                            &resolution,
                        )),
                    });
                    continue;
                }
                if let Err(error) =
                    verify_assertion_of_target(&resolution, &binding.transformer.compiler_symbol)
                {
                    if let Some(evidence) = conditional_oracle_evidence(
                        &resolution,
                        &binding.transformer.compiler_symbol,
                        &binding.context.compiler_symbol,
                    )? {
                        conditional.insert(evidence.evidence_fingerprint.clone(), evidence);
                    }
                    rejections.push(OracleCandidateRejection {
                        identity_fingerprint,
                        owner: test_candidate.owner.clone(),
                        stage: OracleRejectionStage::TargetAssertion,
                        code: error.code,
                        compiler_diagnostic: None,
                    });
                    continue;
                }
                if let Err(error) = verify_context_argument_of_target(
                    &resolution,
                    &binding.transformer.compiler_symbol,
                    &binding.context.compiler_symbol,
                ) {
                    if let Some(evidence) = conditional_oracle_evidence(
                        &resolution,
                        &binding.transformer.compiler_symbol,
                        &binding.context.compiler_symbol,
                    )? {
                        conditional.insert(evidence.evidence_fingerprint.clone(), evidence);
                    }
                    rejections.push(OracleCandidateRejection {
                        identity_fingerprint,
                        owner: test_candidate.owner.clone(),
                        stage: OracleRejectionStage::ContextArgument,
                        code: error.code,
                        compiler_diagnostic: None,
                    });
                    continue;
                }
                match self.issue_behavioral_test(
                    symbol,
                    compilation,
                    &binding.transformer.compiler_symbol,
                    Some(&binding.context.compiler_symbol),
                    worker,
                ) {
                    Ok(receipt) => return Ok((Some(receipt), rejections, None)),
                    Err(error) => rejections.push(OracleCandidateRejection {
                        identity_fingerprint,
                        owner: test_candidate.owner.clone(),
                        stage: OracleRejectionStage::AuthorityReceipt,
                        code: error.code,
                        compiler_diagnostic: None,
                    }),
                }
            }
        }
        let conditional = (conditional.len() == 1).then(|| {
            conditional
                .into_values()
                .next()
                .expect("one conditional oracle")
        });
        Ok((None, rejections, conditional))
    }

    #[cfg(any())]
    fn bind_value_flow_operators(
        &mut self,
        goal: &TypedSemanticGoal,
        plan: &crate::semantic_goal::ConstraintExecutionPlan,
        worker: &mut WorkerClient,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        let flows = discover_value_flows(&self.repo, worker)?;
        let map_applications: Vec<_> = goal
            .operators
            .iter()
            .filter(|item| item.operator == PrimitiveConstraint::MapEdge)
            .collect();
        let type_applications: Vec<_> = goal
            .operators
            .iter()
            .filter(|item| item.operator == PrimitiveConstraint::TypeAssignable)
            .collect();

        if map_applications.len() == 1 {
            let application = map_applications[0];
            let mut choices = Vec::new();
            for (flow_index, flow) in flows.iter().enumerate() {
                for candidate in &flow.candidates {
                    choices.push((flow_index, candidate));
                }
            }
            if choices.is_empty() {
                return Ok(typed_goal_refused(
                    TypedGoalRefusalReason::NoCompatibleBindings,
                ));
            }
            if choices.len() != 1 {
                return Ok(typed_operator_ambiguity(
                    application,
                    choices.into_iter().map(|(index, candidate)| {
                        let flow = &flows[index];
                        vec![
                            candidate.context.compiler_symbol.clone(),
                            candidate.transformer.compiler_symbol.clone(),
                            typed_edge_symbol(&flow.edge),
                        ]
                    }),
                ));
            }
            let (flow_index, candidate) = choices[0];
            let flow = &flows[flow_index];
            let thread = flow.thread.clone();
            let edge = flow.edge.clone();
            let candidate = candidate.clone();
            let verified = self.verify_thread(&thread, worker)?;
            transaction::validate_worktree(
                &self.repo,
                thread.snapshot.build_system,
                &thread.snapshot.build_launcher,
                &thread.snapshot.compile_task,
                &[],
            )?;
            let test_compilation = authority_test_compilation(&thread.snapshot)?;
            let test_symbols = discover_test_symbols(&self.repo, &test_compilation, worker)?;
            let mut behavioral_test = None;
            let mut rejections = Vec::new();
            'tests: for test_candidate in test_symbols {
                for symbol in test_candidate.queries {
                    let identity_fingerprint = canonical::hash(&symbol).map_err(internal)?;
                    let resolution = match worker.request(
                        RequestKind::ResolveSymbol,
                        &json!({"repo":self.repo,"compilation":":/test","symbol":&symbol}),
                    ) {
                        Ok(resolution) => resolution,
                        Err(error) => {
                            rejections.push(OracleCandidateRejection {
                                identity_fingerprint,
                                owner: test_candidate.owner.clone(),
                                stage: OracleRejectionStage::ResolveIdentity,
                                code: error.code,
                            });
                            continue;
                        }
                    };
                    if resolution.get("k2Validated").and_then(Value::as_bool) != Some(true)
                        || has_error_diagnostic(&resolution)
                    {
                        rejections.push(OracleCandidateRejection {
                            identity_fingerprint,
                            owner: test_candidate.owner.clone(),
                            stage: OracleRejectionStage::K2Validation,
                            code: ErrorCode::IncompleteSemanticAnalysis,
                        });
                        continue;
                    }
                    if let Err(error) = verify_assertion_of_target(
                        &resolution,
                        &candidate.transformer.compiler_symbol,
                    ) {
                        rejections.push(OracleCandidateRejection {
                            identity_fingerprint,
                            owner: test_candidate.owner.clone(),
                            stage: OracleRejectionStage::TargetAssertion,
                            code: error.code,
                        });
                        continue;
                    }
                    if let Err(error) = verify_context_argument_of_target(
                        &resolution,
                        &candidate.transformer.compiler_symbol,
                        &candidate.context.compiler_symbol,
                    ) {
                        rejections.push(OracleCandidateRejection {
                            identity_fingerprint,
                            owner: test_candidate.owner.clone(),
                            stage: OracleRejectionStage::ContextArgument,
                            code: error.code,
                        });
                        continue;
                    }
                    match self.issue_behavioral_test(
                        &symbol,
                        &test_compilation,
                        &candidate.transformer.compiler_symbol,
                        Some(&candidate.context.compiler_symbol),
                        worker,
                    ) {
                        Ok(receipt) => {
                            behavioral_test = Some(receipt);
                            break 'tests;
                        }
                        Err(error)
                            if matches!(
                                error.code,
                                ErrorCode::IncompleteSemanticAnalysis
                                    | ErrorCode::SymbolNotFound
                                    | ErrorCode::AmbiguousSymbol
                            ) =>
                        {
                            rejections.push(OracleCandidateRejection {
                                identity_fingerprint,
                                owner: test_candidate.owner.clone(),
                                stage: OracleRejectionStage::AuthorityReceipt,
                                code: error.code,
                            })
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            let Some(test) = behavioral_test else {
                return Ok(typed_goal_refused_with_rejections(
                    TypedGoalRefusalReason::MissingBehavioralOracle,
                    rejections,
                ));
            };
            let validation = self.run_validation(&[&verified], &[&test], worker)?;
            let bundle = self.authorize_bundle(&[&verified], &[&test], &validation)?;
            let bindings = BTreeMap::from([
                (
                    application.operands[0].clone(),
                    candidate.context.compiler_symbol.clone(),
                ),
                (
                    application.operands[1].clone(),
                    candidate.transformer.compiler_symbol.clone(),
                ),
                (application.operands[2].clone(), typed_edge_symbol(&edge)),
            ]);
            let evidence_seed = canonical::hash(&(
                &flow.thread_fingerprint,
                &flow.index_hash,
                &candidate.context_resolution_hash,
                &candidate.transformer_resolution_hash,
                bundle.summary(),
            ))
            .map_err(internal)?;
            return self.issue_typed_goal_receipt(goal, plan, bindings, &evidence_seed);
        }
        if !map_applications.is_empty() {
            return Ok(typed_goal_refused(TypedGoalRefusalReason::InvalidGoal));
        }

        if type_applications.len() == 1 {
            let application = type_applications[0];
            let mut choices = Vec::<(usize, MapCandidate)>::new();
            for (flow_index, flow) in flows.iter().enumerate() {
                for candidate in &flow.candidates {
                    if !choices.iter().any(|(known_flow, known)| {
                        *known_flow == flow_index
                            && known.transformer.compiler_symbol
                                == candidate.transformer.compiler_symbol
                    }) {
                        choices.push((flow_index, candidate.clone()));
                    }
                }
            }
            if choices.is_empty() {
                return Ok(typed_goal_refused(
                    TypedGoalRefusalReason::NoCompatibleBindings,
                ));
            }
            if choices.len() != 1 {
                return Ok(typed_operator_ambiguity(
                    application,
                    choices.iter().map(|(index, candidate)| {
                        vec![
                            candidate.transformer.compiler_symbol.clone(),
                            typed_edge_symbol(&flows[*index].edge),
                        ]
                    }),
                ));
            }
            let (flow_index, candidate) = &choices[0];
            let flow = &flows[*flow_index];
            let bindings = BTreeMap::from([
                (
                    application.operands[0].clone(),
                    candidate.transformer.compiler_symbol.clone(),
                ),
                (
                    application.operands[1].clone(),
                    typed_edge_symbol(&flow.edge),
                ),
            ]);
            let evidence_seed = canonical::hash(&(
                &flow.thread_fingerprint,
                &flow.index_hash,
                &candidate.transformer_resolution_hash,
            ))
            .map_err(internal)?;
            return self.issue_typed_goal_receipt(goal, plan, bindings, &evidence_seed);
        }
        if !type_applications.is_empty() {
            return Ok(typed_goal_refused(TypedGoalRefusalReason::InvalidGoal));
        }

        // A bare uniqueness assertion has no semantic role. It may bind only
        // when its declared domain has exactly one repository-wide candidate;
        // it never invents producer/transformer/edge roles.
        if goal
            .operators
            .iter()
            .all(|item| item.operator == PrimitiveConstraint::BindUnique)
        {
            let mut bindings = BTreeMap::new();
            let mut ambiguity = Vec::new();
            for application in &goal.operators {
                let variable = &application.operands[0];
                let mut candidates = match goal.variables[variable] {
                    TypedVariableDomain::Callable => flows
                        .iter()
                        .flat_map(|flow| {
                            flow.candidates.iter().flat_map(|candidate| {
                                [
                                    candidate.context.compiler_symbol.clone(),
                                    candidate.transformer.compiler_symbol.clone(),
                                ]
                            })
                        })
                        .collect::<BTreeSet<_>>(),
                    TypedVariableDomain::ValueEdge => flows
                        .iter()
                        .map(|flow| typed_edge_symbol(&flow.edge))
                        .collect(),
                };
                if candidates.len() == 1 {
                    bindings.insert(variable.clone(), candidates.pop_first().unwrap());
                } else {
                    ambiguity.push(TypedGoalChoice {
                        bindings: candidates
                            .into_iter()
                            .map(|candidate| (variable.clone(), candidate))
                            .collect(),
                    });
                }
            }
            if !ambiguity.is_empty() {
                return Ok(TypedGoalBindingDecision::Ambiguous(TypedGoalAmbiguity {
                    schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
                    status: "AMBIGUOUS".into(),
                    choices: ambiguity,
                }));
            }
            let evidence_seed = canonical::hash(&bindings).map_err(internal)?;
            return self.issue_typed_goal_receipt(goal, plan, bindings, &evidence_seed);
        }

        Ok(typed_goal_refused(
            TypedGoalRefusalReason::UnsupportedOperatorComposition,
        ))
    }

    fn issue_typed_goal_receipt(
        &mut self,
        goal: &TypedSemanticGoal,
        plan: &crate::semantic_goal::ConstraintExecutionPlan,
        bindings: BTreeMap<String, String>,
        evidence_relations: Vec<ProvenRelationRecord>,
        evidence_seed: &str,
        external_spec: Option<ExternalSpecProof>,
    ) -> Result<TypedGoalBindingDecision, ClewError> {
        let change_graph = typed_change_graph(goal, plan, &bindings, &evidence_relations)?;
        let goal_fingerprint = canonical::hash(goal).map_err(internal)?;
        let evidence_fingerprint =
            canonical::hash(&(&goal_fingerprint, evidence_seed, &bindings, &change_graph))
                .map_err(internal)?;
        let summary = TypedGoalProofSummary {
            schema: TYPED_GOAL_PROOF_SUMMARY_SCHEMA.into(),
            revision: self.revision.clone(),
            goal_fingerprint,
            bindings,
            discharged_operators: plan.mandatory_closure.clone(),
            evidence_relations,
            change_graph,
            evidence_fingerprint,
            external_spec,
        };
        if !summary.is_complete_for(goal) {
            return Err(internal(
                "authority produced an incomplete typed-goal proof",
            ));
        }
        let receipt_id = Uuid::new_v4();
        self.typed_goal_proofs.insert(receipt_id, summary.clone());
        Ok(TypedGoalBindingDecision::Bound(Box::new(
            TypedGoalBindingReceipt {
                session_id: self.session_id,
                receipt_id,
                summary,
            },
        )))
    }

    pub fn recognizes_typed_goal(
        &self,
        receipt: &TypedGoalBindingReceipt,
    ) -> Result<bool, ClewError> {
        self.ensure_revision()?;
        Ok(receipt.session_id == self.session_id
            && self
                .typed_goal_proofs
                .get(&receipt.receipt_id)
                .is_some_and(|summary| summary == &receipt.summary))
    }

    /// Verifies that a transferable summary is exactly one issued by this
    /// live authority. It is deliberately not an authorization capability.
    pub fn recognizes_typed_goal_summary(
        &self,
        summary: &TypedGoalProofSummary,
    ) -> Result<bool, ClewError> {
        self.ensure_revision()?;
        Ok(self
            .typed_goal_proofs
            .values()
            .any(|authorized| authorized == summary))
    }

    pub fn recognizes_map_edge_with_context(
        &self,
        receipt: &MapEdgeWithContextReceipt,
    ) -> Result<bool, ClewError> {
        self.ensure_revision()?;
        Ok(receipt.session_id == self.session_id
            && self.map_edge_proofs.contains_key(&receipt.receipt_id))
    }

    /// Compiles an authority-issued E03 receipt into one typed semantic edit.
    /// The returned operation contains no Kotlin replacement text. A summary
    /// copied from JSON cannot enter this path because only a live receipt from
    /// this exact authority session resolves in `map_edge_proofs`.
    pub fn compile_map_edge_with_context_edit(
        &self,
        receipt: &MapEdgeWithContextReceipt,
    ) -> Result<(ThreadIr, EditIr), ClewError> {
        self.ensure_revision()?;
        if receipt.session_id != self.session_id {
            return Err(wrong_session("map-edge proof"));
        }
        let stored = self
            .map_edge_proofs
            .get(&receipt.receipt_id)
            .ok_or_else(|| invalid_receipt("map-edge proof"))?;
        if stored.summary != receipt.summary {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "map-edge receipt summary differs from authority storage",
            ));
        }
        let verified = self
            .threads
            .values()
            .find(|thread| thread.fingerprint == stored.thread_fingerprint)
            .ok_or_else(|| invalid_receipt("map-edge thread"))?;
        let binding = &stored.summary.bindings;
        let mut target = verified
            .thread
            .nodes
            .iter()
            .find(|node| node.id == binding.value_edge_from && node.kind == "PARAMETER")
            .and_then(|node| node.origin.clone())
            .ok_or_else(|| invalid_source("bound value parameter has no exact source anchor"))?;
        target
            .as_object_mut()
            .ok_or_else(|| invalid_source("bound source anchor is not an object"))?
            .insert(
                "ownerSymbolId".into(),
                Value::String(binding.workflow_symbol.replace('/', ".")),
            );
        let edit = EditIr {
            schema: "semantic-edit/0.2".into(),
            thread_id: verified.thread.thread_id.clone(),
            base_revision: self.revision.clone(),
            operations: vec![EditOperation {
                op_id: format!("map-edge:{}", receipt.receipt_id),
                kind: "MAP_EDGE_WITH_CONTEXT".into(),
                target: target.clone(),
                replacement: Replacement {
                    kotlin: String::new(),
                },
                semantic_operation: Some(SemanticOperation::MapEdgeWithContext {
                    workflow_symbol: binding.workflow_symbol.clone(),
                    context_producer_symbol: binding.context_producer_symbol.clone(),
                    transformer_symbol: binding.transformer_symbol.clone(),
                    value_parameter_index: binding.value_parameter_index,
                    collection_type: binding.collection_type.clone(),
                    element_type: binding.element_type.clone(),
                    context_type: binding.context_type.clone(),
                    placement: binding.placement.clone(),
                    strategy: binding.strategy.clone(),
                }),
                preconditions: BTreeMap::from([(
                    "nodeTextHash".into(),
                    target.get("exactTextHash").cloned().unwrap_or(Value::Null),
                )]),
                postconditions: BTreeMap::from([(
                    "authorityEvidenceFingerprint".into(),
                    Value::String(stored.summary.evidence_fingerprint.clone()),
                )]),
            }],
            expected_write_set: vec![],
        };
        Ok((verified.thread.clone(), edit))
    }

    /// Applies the typed edit through the isolated semantic transaction. The
    /// generic preview and commit APIs reject this operation kind, so a live
    /// authority receipt is required all the way to the commit boundary.
    pub fn commit_map_edge_with_context(
        &self,
        receipt: &MapEdgeWithContextReceipt,
        actor: &str,
        target_ref: &str,
        worker: &mut WorkerClient,
    ) -> Result<(Value, Transaction), ClewError> {
        ensure_repository_root(&self.repo)?;
        let (thread, edit) = self.compile_map_edge_with_context_edit(receipt)?;
        let proof_hash = canonical::hash(receipt.summary()).map_err(internal)?;
        let mut transaction = Transaction {
            schema: "semantic-transaction/0.2".into(),
            tx_id: format!("tx:{}", Uuid::new_v4()),
            actor_id: actor.into(),
            intent: "MAP_EDGE_WITH_CONTEXT".into(),
            base_revision: self.revision.clone(),
            project_model_hash: thread.snapshot.project_model_hash.clone(),
            base_index_snapshot: Some(thread.snapshot.index_snapshot.clone()),
            status: "CREATED".into(),
            thread: thread.clone(),
            required_threads: vec![thread.clone()],
            edit,
            preview: None,
            expected_write_set_hash: None,
            actual_write_set_hash: None,
            validation_evidence: vec![json!({
                "kind":"AUTHORITY_MAP_EDGE_PROOF",
                "proofHash":proof_hash,
                "evidenceFingerprint":receipt.summary().evidence_fingerprint,
                "invariantCount":receipt.summary().invariants.len(),
                "obligationCount":receipt.summary().change_graph.obligations.len()
            })],
            test_tasks: thread.snapshot.test_tasks.clone(),
            candidate_commit: None,
            final_commit: None,
            target_ref: None,
        };
        let result = transaction::commit_authorized_semantic(
            &self.repo,
            &mut transaction,
            target_ref,
            worker,
        )?;
        Ok((result, transaction))
    }

    pub fn recognizes_complete_for(&self, receipt: &CompleteForReceipt) -> Result<bool, ClewError> {
        self.ensure_revision()?;
        Ok(receipt.session_id == self.session_id && self.completions.contains(&receipt.receipt_id))
    }

    fn resolve_threads<'a>(
        &'a self,
        receipts: &[&VerifiedThreadReceipt],
    ) -> Result<Vec<&'a VerifiedThread>, ClewError> {
        if receipts.is_empty() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "authority needs at least one verified thread",
            ));
        }
        let mut ids = BTreeSet::new();
        receipts
            .iter()
            .map(|receipt| {
                if receipt.session_id != self.session_id {
                    return Err(wrong_session("thread"));
                }
                if !ids.insert(receipt.receipt_id) {
                    return Err(ClewError::new(
                        ErrorCode::InvalidInput,
                        "duplicate verified thread receipt",
                    ));
                }
                self.threads
                    .get(&receipt.receipt_id)
                    .ok_or_else(|| invalid_receipt("thread"))
            })
            .collect()
    }

    fn resolve_tests<'a>(
        &'a self,
        receipts: &[&VerifiedBehavioralTestReceipt],
    ) -> Result<Vec<&'a VerifiedBehavioralTest>, ClewError> {
        if receipts.is_empty() {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "authority needs at least one verified behavioral test",
            ));
        }
        let mut ids = BTreeSet::new();
        receipts
            .iter()
            .map(|receipt| {
                if receipt.session_id != self.session_id {
                    return Err(wrong_session("behavioral test"));
                }
                if !ids.insert(receipt.receipt_id) {
                    return Err(ClewError::new(
                        ErrorCode::InvalidInput,
                        "duplicate behavioral-test receipt",
                    ));
                }
                self.tests
                    .get(&receipt.receipt_id)
                    .ok_or_else(|| invalid_receipt("behavioral test"))
            })
            .collect()
    }

    fn ensure_revision(&self) -> Result<(), ClewError> {
        let current = git_head(&self.repo)?;
        if current != self.revision {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                format!(
                    "authority revision changed from {} to {current}",
                    self.revision
                ),
            ));
        }
        ensure_clean_checkout(&self.repo)
    }
}

fn executable_plan_domain(
    plan: &crate::semantic_goal::ConstraintExecutionPlan,
) -> Result<ConstraintDomain, TypedGoalRefusalReason> {
    let domains = plan
        .mandatory_closure
        .iter()
        .filter_map(|application| {
            let spec = constraint_op_spec(&application.operator);
            (!spec.auxiliary_only).then_some(spec.domain)
        })
        .collect::<BTreeSet<_>>();
    if domains.len() != 1 {
        return Err(TypedGoalRefusalReason::UnsupportedConstraintDomain);
    }
    Ok(*domains.iter().next().unwrap())
}

fn preflight_typed_goal(
    goal: &TypedSemanticGoal,
) -> Result<crate::semantic_goal::ConstraintExecutionPlan, TypedGoalRefusalReason> {
    let plan = goal.execution_plan().map_err(|error| match error {
        TypedGoalLanguageError::UnsupportedConstraintDomain => {
            TypedGoalRefusalReason::UnsupportedConstraintDomain
        }
        _ => TypedGoalRefusalReason::InvalidGoal,
    })?;
    executable_plan_domain(&plan)?;
    Ok(plan)
}

fn map_edge_refused(reason: MapEdgeRefusalReason) -> MapEdgeWithContextDecision {
    MapEdgeWithContextDecision::Refused(MapEdgeRefusal {
        schema: "map-edge-with-context-decision/0.1".into(),
        status: "REFUSED".into(),
        reason,
    })
}

fn argument_mapping_obligation(subject: Vec<String>) -> UnresolvedVerificationObligation {
    UnresolvedVerificationObligation {
        id: "verify-argument-parameter-mapping".into(),
        code: VerificationObligationCode::VerifyArgumentParameterMapping,
        subject,
        established_authority: EvidenceStrength::SourceStructural,
        required_authority: EvidenceStrength::CompilerExact,
        acceptable_verifiers: vec![
            VerificationMethod::CompilerArgumentMapping,
            VerificationMethod::FocusedRuntimeTest,
            VerificationMethod::CandidateCompileAndTest,
            VerificationMethod::HumanReview,
        ],
        publication_blocking: true,
    }
}

fn conditional_oracle_obligations(
    evidence: &ConditionalOracleEvidence,
    subject: Vec<String>,
) -> Vec<UnresolvedVerificationObligation> {
    let mut obligations = Vec::new();
    if !evidence.target_identity_exact {
        obligations.push(UnresolvedVerificationObligation {
            id: "verify-call-target-identity".into(),
            code: VerificationObligationCode::VerifyCallTargetIdentity,
            subject: subject.clone(),
            established_authority: EvidenceStrength::SourceStructural,
            required_authority: EvidenceStrength::CompilerExact,
            acceptable_verifiers: vec![
                VerificationMethod::CompilerArgumentMapping,
                VerificationMethod::FocusedRuntimeTest,
                VerificationMethod::HumanReview,
            ],
            publication_blocking: true,
        });
    }
    obligations.push(argument_mapping_obligation(subject));
    obligations
}

fn typed_goal_refused(reason: TypedGoalRefusalReason) -> TypedGoalBindingDecision {
    TypedGoalBindingDecision::Refused(TypedGoalRefusal {
        schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
        status: "REFUSED".into(),
        reason,
        rejections: vec![],
        declaration_rejections: vec![],
    })
}

fn typed_goal_refused_with_rejections(
    reason: TypedGoalRefusalReason,
    rejections: Vec<OracleCandidateRejection>,
) -> TypedGoalBindingDecision {
    TypedGoalBindingDecision::Refused(TypedGoalRefusal {
        schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
        status: "REFUSED".into(),
        reason,
        rejections,
        declaration_rejections: vec![],
    })
}

fn typed_goal_refused_with_declaration_rejection(
    reason: TypedGoalRefusalReason,
    rejection: DeclarationProviderRejection,
) -> TypedGoalBindingDecision {
    TypedGoalBindingDecision::Refused(TypedGoalRefusal {
        schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
        status: "REFUSED".into(),
        reason,
        rejections: vec![],
        declaration_rejections: vec![rejection],
    })
}

fn declaration_provider_rejection<T: Serialize>(
    stage: DeclarationProviderStage,
    code: ErrorCode,
    facts: &T,
) -> DeclarationProviderRejection {
    DeclarationProviderRejection {
        stage,
        code,
        fact_fingerprint: canonical::hash(facts).unwrap_or_else(|_| "unavailable".into()),
        candidate_cardinality: None,
        type_comparison: None,
        type_shapes: None,
        fact_cardinalities: BTreeMap::new(),
        range_relations: BTreeMap::new(),
        fact_relations: BTreeMap::new(),
    }
}

fn obligation_matches_application(
    obligation: &ChangeObligation,
    application: &OperatorApplication,
) -> bool {
    obligation.id == typed_obligation_id(application)
        && matches!(
            (&application.operator, &obligation.kind),
            (PrimitiveConstraint::BindUnique, ObligationKind::BindUnique)
                | (
                    PrimitiveConstraint::TypeAssignable,
                    ObligationKind::TypeAssignable
                )
                | (
                    PrimitiveConstraint::IntroduceOnce,
                    ObligationKind::IntroduceOnce
                )
                | (PrimitiveConstraint::MapEdge, ObligationKind::MapEdge)
                | (
                    PrimitiveConstraint::PreserveOrder,
                    ObligationKind::PreserveOrder
                )
                | (
                    PrimitiveConstraint::PreserveCardinality,
                    ObligationKind::PreserveCardinality
                )
                | (
                    PrimitiveConstraint::PreserveLaziness,
                    ObligationKind::PreserveLaziness
                )
                | (
                    PrimitiveConstraint::PreserveEffects,
                    ObligationKind::PreserveEffects
                )
                | (
                    PrimitiveConstraint::PreserveNullability,
                    ObligationKind::PreserveNullability
                )
                | (
                    PrimitiveConstraint::PreserveConsumerContract,
                    ObligationKind::PreserveConsumerContract
                )
                | (
                    PrimitiveConstraint::PreserveAbi,
                    ObligationKind::PreserveAbi
                )
                | (
                    PrimitiveConstraint::RequireOracle,
                    ObligationKind::RequireOracle
                )
                | (
                    PrimitiveConstraint::MustRefuseOnBoundary,
                    ObligationKind::MustRefuseOnBoundary
                )
                | (
                    PrimitiveConstraint::PreserveResourceLifetime,
                    ObligationKind::PreserveResourceLifetime
                )
                | (
                    PrimitiveConstraint::PropagateDeclaredType,
                    ObligationKind::PropagateDeclaredType
                )
                | (
                    PrimitiveConstraint::PreserveOverrideCompatibility,
                    ObligationKind::PreserveOverrideCompatibility
                )
                | (
                    PrimitiveConstraint::PreserveAssignableUse,
                    ObligationKind::PreserveAssignableUse
                )
                | (
                    PrimitiveConstraint::RelaxNullability,
                    ObligationKind::RelaxNullability
                )
                | (
                    PrimitiveConstraint::PreserveConstruction,
                    ObligationKind::PreserveConstruction
                )
                | (
                    PrimitiveConstraint::ValueFlowsTo,
                    ObligationKind::ValueFlowsTo
                )
                | (
                    PrimitiveConstraint::PreserveOwnerBoundary,
                    ObligationKind::PreserveOwnerBoundary
                )
                | (
                    PrimitiveConstraint::RequireIndependentOracle,
                    ObligationKind::RequireIndependentOracle
                )
                | (
                    PrimitiveConstraint::RequireOmissionDetection,
                    ObligationKind::RequireOmissionDetection
                )
                | (
                    PrimitiveConstraint::PreserveProductionContract,
                    ObligationKind::PreserveProductionContract
                )
                | (
                    PrimitiveConstraint::NullHandles,
                    ObligationKind::NullHandles
                )
                | (
                    PrimitiveConstraint::ProjectsValue,
                    ObligationKind::ProjectsValue
                )
        )
}

fn typed_obligation_id(application: &OperatorApplication) -> String {
    let hash = canonical::hash(application).unwrap_or_default();
    format!("operator:{}", &hash[..hash.len().min(20)])
}

fn typed_obligation_kind(operator: &PrimitiveConstraint) -> ObligationKind {
    match operator {
        PrimitiveConstraint::BindUnique => ObligationKind::BindUnique,
        PrimitiveConstraint::TypeAssignable => ObligationKind::TypeAssignable,
        PrimitiveConstraint::IntroduceOnce => ObligationKind::IntroduceOnce,
        PrimitiveConstraint::MapEdge => ObligationKind::MapEdge,
        PrimitiveConstraint::PreserveOrder => ObligationKind::PreserveOrder,
        PrimitiveConstraint::PreserveCardinality => ObligationKind::PreserveCardinality,
        PrimitiveConstraint::PreserveLaziness => ObligationKind::PreserveLaziness,
        PrimitiveConstraint::PreserveEffects => ObligationKind::PreserveEffects,
        PrimitiveConstraint::PreserveNullability => ObligationKind::PreserveNullability,
        PrimitiveConstraint::PreserveConsumerContract => ObligationKind::PreserveConsumerContract,
        PrimitiveConstraint::PreserveAbi => ObligationKind::PreserveAbi,
        PrimitiveConstraint::RequireOracle => ObligationKind::RequireOracle,
        PrimitiveConstraint::MustRefuseOnBoundary => ObligationKind::MustRefuseOnBoundary,
        PrimitiveConstraint::PreserveResourceLifetime => ObligationKind::PreserveResourceLifetime,
        PrimitiveConstraint::PropagateDeclaredType => ObligationKind::PropagateDeclaredType,
        PrimitiveConstraint::PreserveOverrideCompatibility => {
            ObligationKind::PreserveOverrideCompatibility
        }
        PrimitiveConstraint::PreserveAssignableUse => ObligationKind::PreserveAssignableUse,
        PrimitiveConstraint::RelaxNullability => ObligationKind::RelaxNullability,
        PrimitiveConstraint::PreserveConstruction => ObligationKind::PreserveConstruction,
        PrimitiveConstraint::ValueFlowsTo => ObligationKind::ValueFlowsTo,
        PrimitiveConstraint::PreserveOwnerBoundary => ObligationKind::PreserveOwnerBoundary,
        PrimitiveConstraint::RequireIndependentOracle => ObligationKind::RequireIndependentOracle,
        PrimitiveConstraint::RequireOmissionDetection => ObligationKind::RequireOmissionDetection,
        PrimitiveConstraint::PreserveProductionContract => {
            ObligationKind::PreserveProductionContract
        }
        PrimitiveConstraint::NullHandles => ObligationKind::NullHandles,
        PrimitiveConstraint::ProjectsValue => ObligationKind::ProjectsValue,
    }
}

fn typed_change_graph(
    goal: &TypedSemanticGoal,
    plan: &crate::semantic_goal::ConstraintExecutionPlan,
    bindings: &BTreeMap<String, String>,
    relation_records: &[ProvenRelationRecord],
) -> Result<ChangeGraph, ClewError> {
    let obligations = plan
        .mandatory_closure
        .iter()
        .map(|application| {
            let subject = application
                .operands
                .iter()
                .map(|operand| bindings.get(operand).cloned().unwrap_or_default())
                .collect::<Vec<_>>();
            let evidence = relation_records
                .iter()
                .filter(|record| record.operator == *application)
                .map(|record| record.evidence_fingerprint.clone())
                .collect::<Vec<_>>();
            if evidence.is_empty() {
                return Err(internal("operator provider emitted no evidence"));
            }
            Ok(ChangeObligation {
                id: typed_obligation_id(application),
                kind: typed_obligation_kind(&application.operator),
                binding_role: None,
                subject,
                depends_on: vec![],
                evidence,
                status: DischargeStatus::Proved,
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    Ok(ChangeGraph {
        schema: crate::semantic_goal::CHANGE_GRAPH_SCHEMA.into(),
        goal_schema: goal.schema.clone(),
        obligations,
    })
}

fn prove_relation_records(
    provider: &dyn OperatorEvidenceProvider,
    plan: &crate::semantic_goal::ConstraintExecutionPlan,
    bindings: &BTreeMap<String, String>,
) -> Result<Vec<ProvenRelationRecord>, TypedGoalRefusalReason> {
    let mut records = Vec::new();
    for application in &plan.mandatory_closure {
        let facts = provider.prove(application)?;
        let spec = constraint_op_spec(&application.operator);
        if facts.len() != spec.required_evidence_relations.len()
            || spec.required_evidence_relations.iter().any(|relation| {
                facts
                    .iter()
                    .filter(|fact| fact.relation == *relation)
                    .count()
                    != 1
            })
        {
            return Err(TypedGoalRefusalReason::InsufficientEvidence);
        }
        let bound_operands = application
            .operands
            .iter()
            .map(|operand| bindings.get(operand).cloned().unwrap_or_default())
            .collect::<Vec<_>>();
        for fact in facts {
            records.push(ProvenRelationRecord {
                operator: application.clone(),
                relation: fact.relation,
                bound_operands: bound_operands.clone(),
                evidence_fingerprint: canonical::hash(&(application, &bound_operands, &fact))
                    .map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
                current: true,
                unknown: false,
                occurrence_count: None,
                occurrence_set_fingerprint: None,
                occurrence_fingerprints: vec![],
            });
        }
    }
    Ok(records)
}

#[cfg(any())]
fn typed_operator_ambiguity(
    application: &OperatorApplication,
    choices: impl IntoIterator<Item = Vec<String>>,
) -> TypedGoalBindingDecision {
    TypedGoalBindingDecision::Ambiguous(TypedGoalAmbiguity {
        schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
        status: "AMBIGUOUS".into(),
        choices: choices
            .into_iter()
            .map(|values| TypedGoalChoice {
                bindings: application.operands.iter().cloned().zip(values).collect(),
            })
            .collect(),
    })
}

fn typed_edge_symbol(edge: &MapValueEdge) -> String {
    format!("{}#{}->{}", edge.workflow_symbol, edge.from, edge.to)
}

fn authority_test_compilation(snapshot: &Snapshot) -> Result<String, ClewError> {
    let (module, _) = snapshot.compilation.rsplit_once('/').ok_or_else(|| {
        invalid_source("authority production compilation has no source-set identity")
    })?;
    let same_module_tasks =
        snapshot
            .test_tasks
            .iter()
            .filter(|task| {
                let task_module = task.rsplit_once(':').map_or(module, |(prefix, _)| {
                    if prefix.is_empty() { ":" } else { prefix }
                });
                task_module == module
            })
            .collect::<Vec<_>>();
    let test_task = same_module_tasks
        .iter()
        .find(|task| task.rsplit(':').next() == Some("test"))
        .copied()
        .or_else(|| (same_module_tasks.len() == 1).then_some(same_module_tasks[0]))
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "authority project does not identify one same-module test compilation",
            )
        })?;
    let source_set = test_task
        .rsplit(':')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_source("authority test task has no source-set identity"))?;
    Ok(format!("{module}/{source_set}"))
}

fn validation_route(
    compilation: &str,
    project: &Value,
    test_binary_class: &str,
    test_method: &str,
) -> Result<ValidationRoute, ClewError> {
    if test_binary_class.is_empty()
        || test_method.is_empty()
        || test_binary_class
            .chars()
            .any(|character| matches!(character, '*' | '?' | '#'))
        || test_method
            .chars()
            .any(|character| matches!(character, '*' | '?' | '#'))
    {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "compiler-derived test identity cannot be represented as an exact build selector",
        ));
    }
    let project_module = required_str(project, "module")?;
    let source_set = required_str(project, "sourceSet")?;
    let (compilation_module, compilation_source_set) = compilation
        .rsplit_once('/')
        .ok_or_else(|| invalid_source("test compilation has no module/source-set identity"))?;
    if compilation_module != project_module
        || compilation_source_set != source_set
        || source_set != "test"
        || !project_module.starts_with(':')
    {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "test compilation does not match its authorized module/source set",
        ));
    }
    let build_system = match project.get("buildSystem").and_then(Value::as_str) {
        Some("GRADLE") => BuildSystem::Gradle,
        Some("MAVEN") => BuildSystem::Maven,
        _ => {
            return Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "unknown validation build system",
            ));
        }
    };
    let build_launcher = required_str(project, "buildLauncher")?.to_owned();
    let project_model_hash = required_str(project, "projectModelHash")?.to_owned();
    let tasks = project
        .get("testTasks")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("project model has no test task set"))?
        .iter()
        .map(|task| task.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| invalid_source("project model contains a non-string test task"))?;
    let mut module_path = PathBuf::new();
    if project_module != ":" {
        for segment in project_module.trim_start_matches(':').split(':') {
            if segment.is_empty()
                || segment == "."
                || segment == ".."
                || segment.contains('/')
                || segment.contains('\\')
            {
                return Err(ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "Gradle module identity is not a contained project path",
                ));
            }
            module_path.push(segment);
        }
    }
    let (test_selector, invocation, report_root) = match build_system {
        BuildSystem::Gradle => {
            if build_launcher != "./gradlew" || tasks.as_slice() != ["test"] {
                return Err(ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "authority supports one standard Gradle test task and report route only",
                ));
            }
            let task = if project_module == ":" {
                ":test".to_owned()
            } else {
                format!("{project_module}:test")
            };
            let selector = format!("{test_binary_class}.{test_method}");
            (
                selector.clone(),
                vec![task, "--tests".to_owned(), selector],
                module_path.join("build/test-results/test"),
            )
        }
        BuildSystem::Maven => {
            if project_module != ":"
                || compilation != ":/test"
                || !matches!(build_launcher.as_str(), "./mvnw" | "mvn")
                || tasks.as_slice() != ["test"]
                || project.get("mavenTestLifecycle").and_then(Value::as_str) != Some("SUREFIRE")
            {
                return Err(ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "authority supports one single-module Maven Surefire test route only",
                ));
            }
            let selector = format!("{test_binary_class}#{test_method}");
            (
                selector.clone(),
                vec![format!("-Dtest={selector}"), "test".to_owned()],
                PathBuf::from("target/surefire-reports"),
            )
        }
    };
    safe_relative_path(report_root.to_string_lossy().as_ref())?;
    Ok(ValidationRoute {
        build_system,
        compilation: compilation.to_owned(),
        module: project_module.to_owned(),
        source_set: source_set.to_owned(),
        build_launcher,
        test_binary_class: test_binary_class.to_owned(),
        test_method: test_method.to_owned(),
        test_selector,
        invocation,
        report_format: "JUNIT_XML".to_owned(),
        report_root,
        project_model_hash,
    })
}

fn operator_binding_candidates(
    application: &OperatorApplication,
    flows: &[DiscoveredValueFlow],
) -> Vec<BTreeMap<String, String>> {
    let mut assignments = Vec::new();
    for flow in flows {
        match application.operator {
            PrimitiveConstraint::MapEdge => {
                for candidate in &flow.map_candidates {
                    let values = vec![
                        candidate.context.callable.compiler_symbol.clone(),
                        candidate.transformer.callable.compiler_symbol.clone(),
                        typed_edge_symbol(&flow.edge),
                    ];
                    assignments.push(application.operands.iter().cloned().zip(values).collect());
                }
            }
            PrimitiveConstraint::TypeAssignable => {
                for transformer in &flow.transformers {
                    let values = vec![
                        transformer.callable.compiler_symbol.clone(),
                        typed_edge_symbol(&flow.edge),
                    ];
                    assignments.push(application.operands.iter().cloned().zip(values).collect());
                }
            }
            _ => {}
        }
    }
    assignments.sort();
    assignments.dedup();
    assignments
}

fn merge_operator_bindings(
    current: &BTreeMap<String, String>,
    candidate: &BTreeMap<String, String>,
) -> Option<BTreeMap<String, String>> {
    if candidate
        .iter()
        .any(|(variable, value)| current.get(variable).is_some_and(|known| known != value))
    {
        return None;
    }
    let mut merged = current.clone();
    merged.extend(candidate.clone());
    Some(merged)
}

fn select_flow_evidence(
    bindings: &BTreeMap<String, String>,
    plan: &crate::semantic_goal::ConstraintExecutionPlan,
    flows: &[DiscoveredValueFlow],
) -> Option<SelectedFlowEvidence> {
    for (flow_index, flow) in flows.iter().enumerate() {
        let edge = typed_edge_symbol(&flow.edge);
        for transformer in &flow.transformers {
            let type_apps_match = plan.mandatory_closure.iter().all(|application| {
                application.operator != PrimitiveConstraint::TypeAssignable
                    || (bindings.get(&application.operands[0])
                        == Some(&transformer.callable.compiler_symbol)
                        && bindings.get(&application.operands[1]) == Some(&edge))
            });
            if !type_apps_match {
                continue;
            }
            let map_application = plan
                .mandatory_closure
                .iter()
                .find(|application| application.operator == PrimitiveConstraint::MapEdge);
            let map_candidate = match map_application {
                Some(application) => flow.map_candidates.iter().find(|candidate| {
                    candidate.transformer.callable.compiler_symbol
                        == transformer.callable.compiler_symbol
                        && bindings.get(&application.operands[0])
                            == Some(&candidate.context.callable.compiler_symbol)
                        && bindings.get(&application.operands[1])
                            == Some(&candidate.transformer.callable.compiler_symbol)
                        && bindings.get(&application.operands[2]) == Some(&edge)
                }),
                None => None,
            };
            if map_application.is_some() && map_candidate.is_none() {
                continue;
            }
            return Some(SelectedFlowEvidence {
                flow_index,
                transformer: transformer.clone(),
                map_candidate: map_candidate.cloned(),
            });
        }
    }
    None
}

struct ValueFlowOperatorEvidenceProvider<'a> {
    bindings: &'a BTreeMap<String, String>,
    viable_bindings: &'a [BTreeMap<String, String>],
    flow: &'a DiscoveredValueFlow,
    selected: &'a SelectedFlowEvidence,
    oracle_fingerprint: Option<&'a str>,
}

impl DeclarationTypeOperatorEvidenceProvider<'_> {
    fn bound_operands(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<&str>, TypedGoalRefusalReason> {
        application
            .operands
            .iter()
            .map(|operand| {
                self.bindings
                    .get(operand)
                    .map(String::as_str)
                    .ok_or(TypedGoalRefusalReason::InsufficientEvidence)
            })
            .collect()
    }

    fn receipt<T: Serialize>(
        &self,
        relation: EvidenceRelation,
        provider_kind: &'static str,
        facts: &T,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        Ok(vec![ProviderFactReceipt {
            relation,
            provider_kind,
            fact_fingerprint: canonical::hash(&(
                facts,
                &self.candidate.boundary_closure_fingerprint,
            ))
            .map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
        }])
    }

    fn exact_pair(&self, operands: &[&str]) -> bool {
        operands
            == [
                self.candidate.source_symbol.as_str(),
                self.candidate.target_symbol.as_str(),
            ]
    }
}

impl OperatorEvidenceProvider for DeclarationTypeOperatorEvidenceProvider<'_> {
    fn prove(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        let operands = self.bound_operands(application)?;
        match application.operator {
            PrimitiveConstraint::BindUnique => {
                let variable = &application.operands[0];
                let candidates = self
                    .viable_bindings
                    .iter()
                    .filter_map(|binding| binding.get(variable).cloned())
                    .collect::<BTreeSet<_>>();
                if candidates.len() != 1 || !candidates.contains(operands[0]) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::UniqueBinding,
                    "compiler-declaration-candidate-cardinality",
                    &(variable, candidates),
                )
            }
            PrimitiveConstraint::PropagateDeclaredType => {
                if !self.exact_pair(&operands) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::DeclaredTypePropagation,
                    "verified-declaration-descriptor-relation",
                    &(
                        &self.candidate.source_callable,
                        &self.candidate.target_callable,
                        &self.candidate.propagation_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::PreserveOverrideCompatibility => {
                if !self.exact_pair(&operands) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::OverrideCompatibility,
                    "verified-override-closure",
                    &self.candidate.override_fingerprint,
                )
            }
            PrimitiveConstraint::PreserveAssignableUse => {
                if !self.exact_pair(&operands) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::AssignableUsePreservation,
                    "verified-use-closure",
                    &self.candidate.use_closure_fingerprint,
                )
            }
            PrimitiveConstraint::PreserveProductionContract => {
                if operands != [self.candidate.target_symbol.as_str()] {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::ProductionContractPreservation,
                    "verified-module-contract",
                    &self.candidate.contract_fingerprint,
                )
            }
            PrimitiveConstraint::RequireIndependentOracle => {
                if !self.exact_pair(&operands) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::IndependentOracle,
                    "signed-external-spec",
                    self.external_spec,
                )
            }
            _ => Err(TypedGoalRefusalReason::UnsupportedOperatorComposition),
        }
    }
}

impl NullableConstructionOperatorEvidenceProvider<'_> {
    fn bound_operands(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<&str>, TypedGoalRefusalReason> {
        application
            .operands
            .iter()
            .map(|operand| {
                self.bindings
                    .get(operand)
                    .map(String::as_str)
                    .ok_or(TypedGoalRefusalReason::InsufficientEvidence)
            })
            .collect()
    }

    fn receipt<T: Serialize>(
        relation: EvidenceRelation,
        provider_kind: &'static str,
        facts: &T,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        Ok(vec![ProviderFactReceipt {
            relation,
            provider_kind,
            fact_fingerprint: canonical::hash(facts)
                .map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
        }])
    }
}

impl ProjectionConsumerOperatorEvidenceProvider<'_> {
    fn receipt<T: Serialize>(
        relation: EvidenceRelation,
        provider_kind: &'static str,
        facts: &T,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        Ok(vec![ProviderFactReceipt {
            relation,
            provider_kind,
            fact_fingerprint: canonical::hash(facts)
                .map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
        }])
    }
}

impl OperatorEvidenceProvider for ProjectionConsumerOperatorEvidenceProvider<'_> {
    fn prove(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        let operands = application
            .operands
            .iter()
            .map(|operand| {
                self.bindings
                    .get(operand)
                    .map(String::as_str)
                    .ok_or(TypedGoalRefusalReason::InsufficientEvidence)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let source = self.candidate.source_symbol.as_str();
        let projection = self.candidate.projection_symbol.as_str();
        let consumer = self.candidate.consumer_symbol.as_str();
        match application.operator {
            PrimitiveConstraint::BindUnique => {
                let variable = &application.operands[0];
                let candidates = self
                    .viable_bindings
                    .iter()
                    .filter_map(|binding| binding.get(variable).cloned())
                    .collect::<BTreeSet<_>>();
                if candidates.len() != 1 || !candidates.contains(operands[0]) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                Self::receipt(
                    EvidenceRelation::UniqueBinding,
                    "compiler-projection-candidate-cardinality",
                    &(variable, candidates),
                )
            }
            PrimitiveConstraint::ProjectsValue if operands == [source, projection, consumer] => {
                Self::receipt(
                    EvidenceRelation::ValueProjection,
                    "verified-return-and-consumer-occurrence-set",
                    &(
                        &self.candidate.occurrence_set_fingerprint,
                        &self.candidate.occurrence_fingerprints,
                    ),
                )
            }
            PrimitiveConstraint::ValueFlowsTo if operands == [projection, consumer] => {
                Self::receipt(
                    EvidenceRelation::DeclarationValueFlow,
                    "verified-projection-call-result-live-thread-flow",
                    &self.candidate.occurrences,
                )
            }
            PrimitiveConstraint::PreserveAssignableUse if operands == [source, projection] => {
                Self::receipt(
                    EvidenceRelation::AssignableUsePreservation,
                    "verified-return-type-and-use-closure",
                    &(
                        &self.candidate.declared_type_fingerprint,
                        &self.candidate.use_closure_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::PreserveOwnerBoundary if operands == [projection, consumer] => {
                Self::receipt(
                    EvidenceRelation::OwnerBoundaryPreservation,
                    "verified-compilation-owner-boundary",
                    &self.candidate.provenance_fingerprint,
                )
            }
            PrimitiveConstraint::PreserveProductionContract if operands == [consumer] => {
                Self::receipt(
                    EvidenceRelation::ProductionContractPreservation,
                    "verified-internal-consumer-contract",
                    &self.candidate.contract_fingerprint,
                )
            }
            PrimitiveConstraint::RequireIndependentOracle if operands == [source, consumer] => {
                Self::receipt(
                    EvidenceRelation::IndependentOracle,
                    "signed-external-spec",
                    self.external_spec,
                )
            }
            _ => Err(TypedGoalRefusalReason::UnsupportedOperatorComposition),
        }
    }
}

impl OperatorEvidenceProvider for NullableConstructionOperatorEvidenceProvider<'_> {
    fn prove(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        let operands = self.bound_operands(application)?;
        let source = self.candidate.source_symbol.as_str();
        let fallback = self.candidate.fallback_symbol.as_str();
        let destination = self.candidate.destination_symbol.as_str();
        match application.operator {
            PrimitiveConstraint::BindUnique => {
                let variable = &application.operands[0];
                let candidates = self
                    .viable_bindings
                    .iter()
                    .filter_map(|binding| binding.get(variable).cloned())
                    .collect::<BTreeSet<_>>();
                if candidates.len() != 1 || !candidates.contains(operands[0]) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                Self::receipt(
                    EvidenceRelation::UniqueBinding,
                    "compiler-null-construction-candidate-cardinality",
                    &(variable, candidates),
                )
            }
            PrimitiveConstraint::NullHandles if operands == [source, fallback, destination] => {
                Self::receipt(
                    EvidenceRelation::NullHandling,
                    "verified-null-coalescing-construction",
                    &(
                        &self.candidate.occurrence_set_fingerprint,
                        &self.candidate.occurrence_fingerprints,
                    ),
                )
            }
            PrimitiveConstraint::ValueFlowsTo if operands == [source, destination] => {
                Self::receipt(
                    EvidenceRelation::DeclarationValueFlow,
                    "verified-null-result-live-thread-flow",
                    &(
                        &self.candidate.occurrence_set_fingerprint,
                        &self.candidate.occurrences,
                    ),
                )
            }
            PrimitiveConstraint::PreserveConstruction if operands == [fallback, destination] => {
                Self::receipt(
                    EvidenceRelation::ConstructionPreservation,
                    "verified-constructor-slot",
                    &(
                        &self.candidate.destination_callable,
                        &self.candidate.occurrence_set_fingerprint,
                        self.candidate
                            .occurrences
                            .iter()
                            .map(|occurrence| {
                                (occurrence.slot_index, &occurrence.construction_fingerprint)
                            })
                            .collect::<Vec<_>>(),
                    ),
                )
            }
            PrimitiveConstraint::PreserveAssignableUse if operands == [source, destination] => {
                Self::receipt(
                    EvidenceRelation::AssignableUsePreservation,
                    "verified-nullability-and-use-closure",
                    &self.candidate.use_closure_fingerprint,
                )
            }
            PrimitiveConstraint::PreserveOwnerBoundary if operands == [source, destination] => {
                Self::receipt(
                    EvidenceRelation::OwnerBoundaryPreservation,
                    "verified-module-owner-boundary",
                    &(
                        &self.candidate.module,
                        &self.candidate.source_set,
                        &self.candidate.provenance_fingerprint,
                        &self.candidate.occurrence_set_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::PreserveProductionContract if operands == [destination] => {
                Self::receipt(
                    EvidenceRelation::ProductionContractPreservation,
                    "verified-internal-production-contract",
                    &self.candidate.contract_fingerprint,
                )
            }
            PrimitiveConstraint::RequireIndependentOracle if operands == [source, destination] => {
                Self::receipt(
                    EvidenceRelation::IndependentOracle,
                    "signed-external-spec",
                    self.external_spec,
                )
            }
            _ => Err(TypedGoalRefusalReason::UnsupportedOperatorComposition),
        }
    }
}

impl ValueFlowOperatorEvidenceProvider<'_> {
    fn bound_operands(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<&str>, TypedGoalRefusalReason> {
        application
            .operands
            .iter()
            .map(|operand| {
                self.bindings
                    .get(operand)
                    .map(String::as_str)
                    .ok_or(TypedGoalRefusalReason::InsufficientEvidence)
            })
            .collect()
    }

    fn receipt<T: Serialize>(
        &self,
        relation: EvidenceRelation,
        provider_kind: &'static str,
        facts: &T,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        Ok(vec![ProviderFactReceipt {
            relation,
            provider_kind,
            fact_fingerprint: canonical::hash(facts)
                .map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
        }])
    }

    fn direct_edge_facts(&self) -> Result<(String, String, String), TypedGoalRefusalReason> {
        let edge = &self.flow.edge;
        let parameter = self
            .flow
            .thread
            .nodes
            .iter()
            .find(|node| node.id == edge.from && node.kind == "PARAMETER")
            .ok_or(TypedGoalRefusalReason::InsufficientEvidence)?;
        let consumer = self
            .flow
            .thread
            .nodes
            .iter()
            .find(|node| node.id == edge.to && node.kind == "RETURN")
            .ok_or(TypedGoalRefusalReason::InsufficientEvidence)?;
        let def_use = self
            .flow
            .thread
            .edges
            .iter()
            .find(|item| item.from == edge.from && item.to == edge.to && item.kind == "DEF_USE")
            .ok_or(TypedGoalRefusalReason::InsufficientEvidence)?;
        Ok((
            canonical::hash(parameter).map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
            canonical::hash(consumer).map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
            canonical::hash(def_use).map_err(|_| TypedGoalRefusalReason::InsufficientEvidence)?,
        ))
    }

    fn graph_shape_facts(&self) -> Result<Vec<String>, TypedGoalRefusalReason> {
        if self.flow.edge.collection_type.contains('?')
            || eager_list_element_type(&self.flow.edge.collection_type)
                != Some(self.flow.edge.element_type.clone())
            || self.flow.thread.nodes.iter().any(|node| {
                matches!(
                    node.kind.as_str(),
                    "BRANCH" | "LOOP" | "CAPTURE" | "THROW" | "ASSIGNMENT"
                )
            })
        {
            return Err(TypedGoalRefusalReason::InsufficientEvidence);
        }
        let direct = self.direct_edge_facts()?;
        Ok(vec![
            self.flow.edge.collection_type.clone(),
            self.flow.edge.element_type.clone(),
            direct.0,
            direct.1,
            direct.2,
        ])
    }

    fn map_candidate(&self) -> Result<&ResolvedMapCandidate, TypedGoalRefusalReason> {
        self.selected
            .map_candidate
            .as_ref()
            .ok_or(TypedGoalRefusalReason::InsufficientEvidence)
    }
}

impl OperatorEvidenceProvider for ValueFlowOperatorEvidenceProvider<'_> {
    fn prove(
        &self,
        application: &OperatorApplication,
    ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
        let operands = self.bound_operands(application)?;
        let edge_symbol = typed_edge_symbol(&self.flow.edge);
        match application.operator {
            PrimitiveConstraint::BindUnique => {
                let variable = &application.operands[0];
                let candidates = self
                    .viable_bindings
                    .iter()
                    .filter_map(|binding| binding.get(variable).cloned())
                    .collect::<BTreeSet<_>>();
                if candidates.len() != 1 || !candidates.contains(operands[0]) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::UniqueBinding,
                    "candidate-cardinality",
                    &(variable, candidates),
                )
            }
            PrimitiveConstraint::TypeAssignable => {
                let callable = &self.selected.transformer.callable;
                if operands != [callable.compiler_symbol.as_str(), edge_symbol.as_str()]
                    || callable.parameter_types.first() != Some(&self.flow.edge.element_type)
                    || callable.return_type != self.flow.edge.element_type
                {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::TypeCompatibility,
                    "compiler-type-resolution",
                    &(
                        &self.selected.transformer.type_fingerprint,
                        &callable.parameter_types,
                        &callable.return_type,
                        &self.flow.edge.collection_type,
                        &self.flow.edge.element_type,
                    ),
                )
            }
            PrimitiveConstraint::IntroduceOnce => {
                let candidate = self.map_candidate()?;
                if operands
                    != [
                        candidate.context.callable.compiler_symbol.as_str(),
                        edge_symbol.as_str(),
                    ]
                    || !candidate.context.callable.parameter_types.is_empty()
                    || self.flow.edge.placement
                        != format!("{}#FUNCTION_ENTRY", self.flow.edge.workflow_symbol)
                {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::EvaluationOnce,
                    "control-flow-dominance",
                    &(
                        &self.flow.edge.placement,
                        self.graph_shape_facts()?,
                        &candidate.context.resolution_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::MapEdge => {
                let candidate = self.map_candidate()?;
                if operands
                    != [
                        candidate.context.callable.compiler_symbol.as_str(),
                        candidate.transformer.callable.compiler_symbol.as_str(),
                        edge_symbol.as_str(),
                    ]
                {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::EdgeMapping,
                    "value-flow-graph",
                    &(
                        &self.flow.edge.workflow_symbol,
                        &self.flow.edge.from,
                        &self.flow.edge.to,
                        self.flow.edge.parameter_index,
                        self.direct_edge_facts()?,
                        &candidate.context.resolution_fingerprint,
                        &candidate.transformer.resolution_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::PreserveOrder => self.receipt(
                EvidenceRelation::OrderPreservation,
                "collection-modality-and-graph-shape",
                &self.graph_shape_facts()?,
            ),
            PrimitiveConstraint::PreserveCardinality => self.receipt(
                EvidenceRelation::CardinalityPreservation,
                "collection-modality-and-graph-shape",
                &self.graph_shape_facts()?,
            ),
            PrimitiveConstraint::PreserveLaziness => self.receipt(
                EvidenceRelation::LazinessPreservation,
                "collection-modality-and-graph-shape",
                &("EAGER_LIST", self.graph_shape_facts()?),
            ),
            PrimitiveConstraint::PreserveEffects => {
                let candidate = self.map_candidate()?;
                if !candidate.context.effects_proven_pure
                    || !candidate.transformer.effects_proven_pure
                {
                    return Err(TypedGoalRefusalReason::UnknownEffects);
                }
                self.receipt(
                    EvidenceRelation::EffectPreservation,
                    "compiler-effect-summary",
                    &(
                        &candidate.context.effect_fingerprint,
                        &candidate.transformer.effect_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::PreserveNullability => {
                let candidate = self.map_candidate()?;
                let types = [
                    self.flow.edge.collection_type.as_str(),
                    self.flow.edge.element_type.as_str(),
                    candidate.context.callable.return_type.as_str(),
                    candidate.transformer.callable.return_type.as_str(),
                ];
                if types.iter().any(|kind| kind.contains('?')) {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                self.receipt(
                    EvidenceRelation::NullabilityPreservation,
                    "compiler-type-resolution",
                    &(types, &candidate.transformer.type_fingerprint),
                )
            }
            PrimitiveConstraint::PreserveConsumerContract | PrimitiveConstraint::PreserveAbi => {
                let parameter = self
                    .flow
                    .thread
                    .nodes
                    .iter()
                    .find(|node| node.id == self.flow.edge.from)
                    .ok_or(TypedGoalRefusalReason::InsufficientEvidence)?;
                let declared = parameter
                    .attributes
                    .get("declaredType")
                    .and_then(Value::as_str);
                let returned = parameter
                    .attributes
                    .get("ownerReturnType")
                    .and_then(Value::as_str);
                if declared != Some(self.flow.edge.collection_type.as_str()) || returned != declared
                {
                    return Err(TypedGoalRefusalReason::InsufficientEvidence);
                }
                let relation =
                    if application.operator == PrimitiveConstraint::PreserveConsumerContract {
                        EvidenceRelation::ConsumerContractPreservation
                    } else {
                        EvidenceRelation::AbiPreservation
                    };
                self.receipt(
                    relation,
                    "compiler-signature",
                    &(
                        &self.flow.edge.workflow_symbol,
                        declared,
                        returned,
                        self.flow.edge.parameter_index,
                        &self.selected.transformer.type_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::RequireOracle => {
                let fingerprint = self
                    .oracle_fingerprint
                    .ok_or(TypedGoalRefusalReason::MissingBehavioralOracle)?;
                self.receipt(
                    EvidenceRelation::BehavioralOracle,
                    "authority-validation-receipt",
                    &fingerprint,
                )
            }
            PrimitiveConstraint::MustRefuseOnBoundary => {
                if self.flow.thread.completeness.status
                    != CompletenessStatus::CompleteSupportedSubset
                    || !self.flow.thread.completeness.boundaries.is_empty()
                    || !self.flow.thread.external_summaries.is_empty()
                {
                    return Err(TypedGoalRefusalReason::UnsupportedBoundary);
                }
                self.receipt(
                    EvidenceRelation::NoUnsupportedBoundary,
                    "semantic-slice-completeness",
                    &(
                        &self.flow.thread.completeness.status,
                        &self.flow.thread.completeness.boundaries,
                        &self.flow.thread.external_summaries,
                        &self.flow.thread_fingerprint,
                    ),
                )
            }
            PrimitiveConstraint::PreserveResourceLifetime => {
                Err(TypedGoalRefusalReason::UnsupportedConstraintDomain)
            }
            PrimitiveConstraint::PropagateDeclaredType
            | PrimitiveConstraint::PreserveOverrideCompatibility
            | PrimitiveConstraint::PreserveAssignableUse
            | PrimitiveConstraint::RelaxNullability
            | PrimitiveConstraint::PreserveConstruction
            | PrimitiveConstraint::ValueFlowsTo
            | PrimitiveConstraint::PreserveOwnerBoundary
            | PrimitiveConstraint::RequireIndependentOracle
            | PrimitiveConstraint::RequireOmissionDetection
            | PrimitiveConstraint::PreserveProductionContract
            | PrimitiveConstraint::NullHandles
            | PrimitiveConstraint::ProjectsValue => {
                Err(TypedGoalRefusalReason::UnsupportedConstraintDomain)
            }
        }
    }
}

fn discover_production_compilations(repo: &Path) -> Result<BTreeSet<String>, ClewError> {
    let mut compilations = BTreeSet::new();
    for entry in WalkDir::new(repo)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some(".git" | "build" | "target" | ".gradle")
            )
        })
    {
        let entry = entry.map_err(internal)?;
        if !entry.file_type().is_dir() || entry.file_name() != "kotlin" {
            continue;
        }
        let path = entry.path();
        let Some(main) = path
            .parent()
            .filter(|parent| parent.file_name().and_then(|name| name.to_str()) == Some("main"))
        else {
            continue;
        };
        let Some(src) = main
            .parent()
            .filter(|parent| parent.file_name().and_then(|name| name.to_str()) == Some("src"))
        else {
            continue;
        };
        let Some(module_root) = src.parent() else {
            continue;
        };
        let has_supported_build = ["build.gradle.kts", "build.gradle", "pom.xml"]
            .iter()
            .any(|name| module_root.join(name).is_file());
        if !has_supported_build {
            continue;
        }
        let relative = module_root.strip_prefix(repo).map_err(internal)?;
        let module = if relative.as_os_str().is_empty() {
            ":".to_owned()
        } else {
            format!(
                ":{}",
                relative
                    .components()
                    .filter_map(|component| match component {
                        Component::Normal(value) => value.to_str(),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(":")
            )
        };
        compilations.insert(format!("{module}/main"));
    }
    Ok(compilations)
}

fn select_production_compilation(
    repo: &Path,
    requested: Option<&str>,
) -> Result<String, ClewError> {
    let available = discover_production_compilations(repo)?;
    match requested {
        Some(value) if available.is_empty() || available.contains(value) => Ok(value.to_owned()),
        Some(_) => Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "requested production compilation is not present in project discovery",
        )),
        None if available.len() == 1 => Ok(available.iter().next().unwrap().clone()),
        None => Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "authority cannot auto-select a unique production compilation; provide --compilation",
        )),
    }
}

fn require_exact_project_compilation(project: &Value, compilation: &str) -> Result<(), ClewError> {
    let resolved = format!(
        "{}/{}",
        required_str(project, "module")?,
        required_str(project, "sourceSet")?
    );
    if resolved != compilation
        || project
            .get("sourceRoots")
            .and_then(Value::as_array)
            .is_none_or(|roots| roots.is_empty())
    {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "authority project model did not resolve the selected production compilation exactly",
        ));
    }
    Ok(())
}

fn production_external_spec_verifying_key() -> Result<[u8; 32], ClewError> {
    let bytes = hex::decode(PRODUCTION_EXTERNAL_SPEC_VERIFYING_KEY_HEX).map_err(internal)?;
    bytes.try_into().map_err(|_| {
        ClewError::new(
            ErrorCode::Internal,
            "production external-spec verifying key has invalid length",
        )
    })
}

fn external_spec_package_digest(payload: &ExternalSpecPayload) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":EXTERNAL_SPEC_PACKAGE_SCHEMA,
        "issuer":payload.issuer,
        "task":payload.task,
        "publicManifest":payload.public_manifest,
        "publicManifestDigest":payload.public_manifest_digest,
        "taskDigest":payload.task_digest,
        "repository":payload.repository,
        "repositoryRevision":payload.repository_revision,
        "sourceSnapshotSha256":payload.source_snapshot_sha256,
        "requestDigest":payload.request_digest,
        "compilation":payload.compilation,
        "projectModelHash":payload.project_model_hash,
    }))
    .map_err(internal)
}

fn read_signed_external_spec(
    repo: &Path,
    path: &Path,
    expected_issuer: &str,
    verifying_key: [u8; 32],
) -> Result<(ExternalSpecPayload, PathBuf, Vec<PathBuf>, String), ClewError> {
    let symlink = std::fs::symlink_metadata(path)
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    if symlink.file_type().is_symlink() || !symlink.is_file() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "external specification must be one regular non-symlink file",
        ));
    }
    if symlink.len() > TYPED_GOAL_MAX_REQUEST_BYTES as u64 {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "external specification exceeds 16 KiB",
        ));
    }
    let canonical_path = path
        .canonicalize()
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    let bytes = std::fs::read(&canonical_path)
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    let envelope: SignedExternalSpecEnvelope = serde_json::from_slice(&bytes)
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    let payload = &envelope.payload;
    if envelope.schema != SIGNED_EXTERNAL_SPEC_SCHEMA
        || payload.schema != EXTERNAL_SPEC_PAYLOAD_SCHEMA
        || payload.issuer != expected_issuer
        || payload.task.trim().is_empty()
        || payload.task.len() > 8 * 1024
        || payload.repository.is_empty()
        || payload.public_manifest.is_empty()
        || !is_raw_sha256(&payload.source_snapshot_sha256)
        || !is_prefixed_sha256(&payload.task_digest)
        || !is_prefixed_sha256(&payload.public_manifest_digest)
        || !is_prefixed_sha256(&payload.package_digest)
        || !is_prefixed_sha256(&payload.request_digest)
        || payload.repository_revision.is_empty()
        || payload.compilation.is_empty()
        || payload.project_model_hash.is_empty()
        || envelope.signature.len() != 128
        || !envelope
            .signature
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "signed external specification has invalid required fields",
        ));
    }
    let canonical_bytes = canonical::bytes(&envelope).map_err(internal)?;
    if canonical_bytes != bytes {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "signed external specification must use canonical JSON encoding",
        ));
    }
    let key = VerifyingKey::from_bytes(&verifying_key).map_err(|_| {
        ClewError::new(
            ErrorCode::InvalidInput,
            "external-spec issuer key is invalid",
        )
    })?;
    let signature_bytes = hex::decode(&envelope.signature).map_err(|_| {
        ClewError::new(
            ErrorCode::InvalidInput,
            "external-spec signature is malformed",
        )
    })?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|_| {
        ClewError::new(
            ErrorCode::InvalidInput,
            "external-spec signature is malformed",
        )
    })?;
    key.verify(&canonical::bytes(payload).map_err(internal)?, &signature)
        .map_err(|_| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "external specification signature is not trusted",
            )
        })?;
    if canonical::hash(&payload.task).map_err(internal)? != payload.task_digest
        || external_spec_package_digest(payload)? != payload.package_digest
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "signed external specification digest closure is invalid",
        ));
    }
    let repository_ref = safe_external_package_path(&payload.repository)?;
    let manifest_ref = safe_external_package_path(&payload.public_manifest)?;
    let package_root = canonical_path
        .parent()
        .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "specification has no package"))?;
    let manifest_path = package_root.join(&manifest_ref);
    let manifest_metadata = std::fs::symlink_metadata(&manifest_path)
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "external specification public manifest must be a regular contained file",
        ));
    }
    let manifest_path = manifest_path
        .canonicalize()
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    if !manifest_path.starts_with(package_root) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "external specification public manifest escapes its package",
        ));
    }
    let manifest_bytes = std::fs::read(&manifest_path).map_err(internal)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    if canonical::hash(&manifest).map_err(internal)? != payload.public_manifest_digest
        || manifest.get("task").and_then(Value::as_str) != Some(payload.task.as_str())
        || manifest.get("repository").and_then(Value::as_str) != Some(payload.repository.as_str())
        || manifest.get("sourceSnapshotSha256").and_then(Value::as_str)
            != Some(payload.source_snapshot_sha256.as_str())
    {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "signed task and public manifest do not describe the same package",
        ));
    }
    let resolved_repo = resolve_strictly_contained_directory(package_root, &repository_ref)?;
    if resolved_repo != repo {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "external specification is bound to another repository package",
        ));
    }
    let mut contained_paths = [&canonical_path, &manifest_path]
        .into_iter()
        .filter_map(|candidate| candidate.strip_prefix(repo).ok().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    contained_paths.sort();
    contained_paths.dedup();
    for relative in &contained_paths {
        let expected = if repo.join(relative) == canonical_path {
            &bytes
        } else {
            &manifest_bytes
        };
        let committed = Command::new("git")
            .current_dir(repo)
            .args(["show", &format!("HEAD:{}", relative.to_string_lossy())])
            .output()
            .map_err(internal)?;
        if !committed.status.success() || committed.stdout != *expected {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "repository-contained external task package is not exact committed HEAD content",
            ));
        }
    }
    let source_snapshot = tracked_source_snapshot(repo, &contained_paths)?;
    if source_snapshot != payload.source_snapshot_sha256 {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "signed external specification source snapshot does not match the repository",
        ));
    }
    let specification_digest = canonical::hash(&envelope).map_err(internal)?;
    Ok((
        envelope.payload,
        canonical_path,
        contained_paths,
        specification_digest,
    ))
}

fn safe_external_package_path(value: &str) -> Result<PathBuf, ClewError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "external task package path is not safely contained",
        ));
    }
    Ok(path.to_owned())
}

fn resolve_strictly_contained_directory(
    package_root: &Path,
    relative: &Path,
) -> Result<PathBuf, ClewError> {
    let package_root = package_root
        .canonicalize()
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    let mut current = package_root.clone();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "external task repository path is not canonical",
            ));
        };
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "external task repository path contains a symlink or non-directory component",
            ));
        }
    }
    let resolved = current
        .canonicalize()
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    if resolved == package_root || !resolved.starts_with(&package_root) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "external task repository must be strictly contained by its package",
        ));
    }
    Ok(resolved)
}

fn is_raw_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_prefixed_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(is_raw_sha256)
}

fn tracked_source_snapshot(repo: &Path, excluded: &[PathBuf]) -> Result<String, ClewError> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["ls-files", "-z"])
        .output()
        .map_err(internal)?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "cannot enumerate repository snapshot",
        ));
    }
    let mut paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| {
            std::str::from_utf8(value)
                .map(str::to_owned)
                .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let excluded = excluded
        .iter()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .collect::<BTreeSet<_>>();
    let mut digest = Sha256::new();
    for relative in paths {
        if excluded.contains(relative.as_str()) {
            continue;
        }
        let path = repo.join(&relative);
        let metadata = std::fs::symlink_metadata(&path).map_err(internal)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "repository snapshot contains a non-regular tracked path",
            ));
        }
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(std::fs::read(path).map_err(internal)?);
        digest.update([0]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn boundary_affected_relations(boundary: &Value) -> Result<BTreeSet<EvidenceRelation>, ClewError> {
    let classification_error = |reason: &str| {
        let mut error = invalid_source(reason);
        let diagnostic = json!({
            "schema":"boundary-classification-diagnostic/0.1",
            "provider":boundary.get("provider").and_then(Value::as_str).unwrap_or("<MISSING>"),
            "stage":boundary.get("stage").and_then(Value::as_str).unwrap_or("<MISSING>"),
            "code":boundary.get("code").and_then(Value::as_str).unwrap_or("<MISSING>"),
            "boundarySchema":boundary.get("schema").and_then(Value::as_str).unwrap_or("<MISSING>"),
            "rowHash":canonical::hash(boundary).unwrap_or_else(|_| "unavailable".into()),
        });
        if let Ok(encoded) = serde_json::to_string(&diagnostic) {
            error.evidence.push(encoded);
        }
        error
    };
    let provider = boundary
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| classification_error("boundary provider is missing or malformed"))?;
    if !matches!(
        provider,
        "K2_FIR"
            | "K2_FIR_CFG"
            | "COMPILER_RELATION_NORMALIZER"
            | "CODECLEW_RELATION_NORMALIZER"
            | "WORKER"
    ) {
        return Err(classification_error(
            "Unknown boundary provider is not registered",
        ));
    }
    let stage = boundary
        .get("stage")
        .and_then(Value::as_str)
        .ok_or_else(|| classification_error("boundary stage is missing or malformed"))?;
    let code = boundary
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| classification_error("boundary code is missing or malformed"))?;
    let relations = match stage {
        "RETURN_VALUE"
            if matches!(
                code,
                "IMPLICIT_RETURN_UNSUPPORTED"
                    | "IMPLICIT_OR_MISSING_RETURN_SOURCE"
                    | "UNRESOLVED_RETURN_OWNER"
                    | "LOCAL_OR_GENERATED_RETURN_OWNER"
                    | "RETURN_TARGET_IDENTITY_MISMATCH"
                    | "NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"
                    | "RETURN_VALUE_NOT_DIRECT_RESOLVED_READ_OR_CALL"
                    | "MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES"
                    | "LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"
                    | "MISSING_RETURN_CFG"
                    | "AMBIGUOUS_RETURN_CFG_NODE"
                    | "RETURN_VALUE_CFG_PROOF_UNAVAILABLE"
            ) =>
        {
            BTreeSet::from([EvidenceRelation::ValueProjection])
        }
        "OVERRIDE"
            if matches!(
                code,
                "NO_RESOLVED_CLASS_SCOPE"
                    | "NO_RESOLVED_BASE"
                    | "NON_FUNCTION_RESOLVED_BASE"
                    | "NON_FUNCTION_OVERRIDE_UNSUPPORTED"
            ) =>
        {
            BTreeSet::from([
                EvidenceRelation::DeclaredTypePropagation,
                EvidenceRelation::OverrideCompatibility,
            ])
        }
        "ARGUMENT_MAPPING"
            if matches!(
                code,
                "ARGUMENT_OWNER_NOT_FUNCTION"
                    | "NO_COMPILER_CALLABLE_ID"
                    | "EXTERNAL_OR_LOCAL_ARGUMENT_TARGET"
                    | "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED"
                    | "CONTEXT_ARGUMENT_MAPPING_UNSUPPORTED"
                    | "VARARG_ARGUMENT_MAPPING_UNSUPPORTED"
                    | "MISSING_RESOLVED_ARGUMENT_MAPPING"
                    | "INCOMPLETE_ARGUMENT_MAPPING"
            ) =>
        {
            BTreeSet::from([
                EvidenceRelation::AssignableUsePreservation,
                EvidenceRelation::DeclarationValueFlow,
            ])
        }
        "OPTIONAL_RELATION_EVIDENCE" if code == "ARGUMENT_MAPPING_UNAVAILABLE" => BTreeSet::from([
            EvidenceRelation::AssignableUsePreservation,
            EvidenceRelation::DeclarationValueFlow,
        ]),
        "REFERENCE"
            if matches!(
                code,
                "UNRESOLVED_CALLABLE_TARGET" | "DYNAMIC_REFLECTION_BOUNDARY"
            ) =>
        {
            BTreeSet::from([
                EvidenceRelation::AssignableUsePreservation,
                EvidenceRelation::DeclarationValueFlow,
                EvidenceRelation::ProductionContractPreservation,
            ])
        }
        "INITIALIZER" if code == "NO_RESOLVED_OWNER" => BTreeSet::from([
            EvidenceRelation::DeclaredTypePropagation,
            EvidenceRelation::AssignableUsePreservation,
        ]),
        "WRITE" if code == "UNRESOLVED_PROPERTY_TARGET" => BTreeSet::from([
            EvidenceRelation::AssignableUsePreservation,
            EvidenceRelation::ProductionContractPreservation,
        ]),
        "NULL_POLICY"
            if matches!(
                code,
                "UNRESOLVED_NULL_POLICY_OWNER"
                    | "MISSING_NULL_POLICY_OCCURRENCE"
                    | "UNRESOLVED_NULLABLE_SOURCE_OCCURRENCE"
                    | "UNRESOLVED_FALLBACK_OCCURRENCE"
                    | "SOURCE_OCCURRENCE_NOT_NULLABLE"
                    | "FALLBACK_OCCURRENCE_NULLABLE"
                    | "MERGED_RESULT_NULLABLE"
                    | "SAFE_CALL_POLICY_UNSUPPORTED"
            ) =>
        {
            BTreeSet::from([
                EvidenceRelation::NullHandling,
                EvidenceRelation::NullabilityRelaxation,
            ])
        }
        "DECLARATION"
            if matches!(
                code,
                "GENERATED_OR_NO_SOURCE"
                    | "LOCAL_DECLARATION_UNSUPPORTED"
                    | "LOCAL_GENERATED_OR_NO_SOURCE"
                    | "UNRESOLVED_DESCRIPTOR_BOUNDARY"
                    | "NO_COMPILER_CALLABLE_ID"
            ) =>
        {
            BTreeSet::from([
                EvidenceRelation::DeclaredTypePropagation,
                EvidenceRelation::OverrideCompatibility,
                EvidenceRelation::AssignableUsePreservation,
                EvidenceRelation::ProductionContractPreservation,
            ])
        }
        "CONSTRUCTOR_DECLARATION"
            if matches!(
                code,
                "NO_COMPILER_CALLABLE_ID"
                    | "GENERATED_OR_NO_SOURCE"
                    | "LOCAL_CONSTRUCTOR_UNSUPPORTED"
                    | "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR"
                    | "INCOMPLETE_COMPILER_DESCRIPTOR"
            ) =>
        {
            BTreeSet::from([
                EvidenceRelation::NullHandling,
                EvidenceRelation::DeclarationValueFlow,
                EvidenceRelation::ConstructionPreservation,
                EvidenceRelation::AssignableUsePreservation,
                EvidenceRelation::OwnerBoundaryPreservation,
                EvidenceRelation::ProductionContractPreservation,
            ])
        }
        "NORMALIZE" if code == "INCOMPLETE_COMPILER_DESCRIPTOR" => BTreeSet::from([
            EvidenceRelation::DeclaredTypePropagation,
            EvidenceRelation::NullHandling,
            EvidenceRelation::ConstructionPreservation,
            EvidenceRelation::AssignableUsePreservation,
            EvidenceRelation::ProductionContractPreservation,
        ]),
        "ANALYSIS" if code == "SYNTAX_ONLY" => BTreeSet::from([
            EvidenceRelation::DeclaredTypePropagation,
            EvidenceRelation::NullHandling,
            EvidenceRelation::DeclarationValueFlow,
            EvidenceRelation::ConstructionPreservation,
            EvidenceRelation::AssignableUsePreservation,
            EvidenceRelation::OwnerBoundaryPreservation,
            EvidenceRelation::ProductionContractPreservation,
        ]),
        _ => {
            return Err(classification_error(
                "Unknown boundary stage/code is not registered",
            ));
        }
    };
    Ok(relations)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BoundaryClosureEvaluation {
    fingerprint: String,
    total_boundary_count: usize,
    relevant_boundary_count: usize,
    refused_boundary_count: usize,
    excluded_by_relation_count: usize,
    decisions: Vec<Value>,
}

fn mandatory_relations_for_plan(
    plan: &crate::semantic_goal::ConstraintExecutionPlan,
) -> BTreeSet<EvidenceRelation> {
    plan.mandatory_closure
        .iter()
        .flat_map(|application| {
            constraint_op_spec(&application.operator)
                .required_evidence_relations
                .iter()
                .copied()
        })
        .collect()
}

/// Evaluates compiler Unknown boundaries relative to an operator's mandatory
/// evidence closure and the exact compiler identities on the candidate path.
/// Both dimensions are required: path relevance cannot excuse an Unknown for
/// a mandatory relation, and an unrelated relation cannot block the goal.
fn evaluate_obligation_relative_boundaries(
    graphs: &[(&str, &Value)],
    mandatory_relations: &BTreeSet<EvidenceRelation>,
    roots: &BTreeSet<String>,
) -> Result<BoundaryClosureEvaluation, ClewError> {
    let mut decisions = Vec::new();
    let mut refused = Vec::new();
    let mut relevant_count = 0usize;
    let mut excluded_by_relation_count = 0usize;
    for (graph_kind, graph) in graphs {
        let boundaries = graph
            .get("boundaries")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_source("semantic fact graph has no typed boundary array"))?;
        let coverage = graph.get("coverage").and_then(Value::as_str);
        if !matches!(coverage, Some("COMPLETE_SUPPORTED_SUBSET" | "PARTIAL"))
            || (coverage == Some("PARTIAL") && boundaries.is_empty())
        {
            return Err(invalid_source(
                "semantic fact graph has invalid boundary coverage",
            ));
        }
        for boundary in boundaries {
            let affected = boundary_affected_relations(boundary).map_err(|mut error| {
                if let Some((ordinal, mut diagnostic)) =
                    error
                        .evidence
                        .iter()
                        .enumerate()
                        .find_map(|(ordinal, encoded)| {
                            serde_json::from_str::<Value>(encoded)
                                .ok()
                                .filter(|value| {
                                    value.get("schema").and_then(Value::as_str)
                                        == Some("boundary-classification-diagnostic/0.1")
                                })
                                .map(|value| (ordinal, value))
                        })
                {
                    if let Some(object) = diagnostic.as_object_mut() {
                        object.insert("graphKind".into(), Value::String((*graph_kind).into()));
                    }
                    if let Ok(encoded) = serde_json::to_string(&diagnostic) {
                        error.evidence[ordinal] = encoded;
                    }
                }
                error
            })?;
            let owner = boundary.get("owner").and_then(Value::as_str);
            let target = boundary.get("target").and_then(Value::as_str);
            let owner_relevant = owner.is_some_and(|value| roots.contains(value));
            let target_relevant = target.is_some_and(|value| roots.contains(value));
            let path_relevant = owner_relevant || target_relevant;
            let intersection = affected
                .intersection(mandatory_relations)
                .copied()
                .collect::<BTreeSet<_>>();
            let is_refused = path_relevant && !intersection.is_empty();
            if path_relevant {
                relevant_count += 1;
            }
            if path_relevant && intersection.is_empty() {
                excluded_by_relation_count += 1;
            }
            let decision = json!({
                "graphKind":graph_kind,
                "boundaryHash":canonical::hash(boundary).map_err(internal)?,
                "provider":required_str(boundary, "provider")?,
                "stage":required_str(boundary, "stage")?,
                "code":required_str(boundary, "code")?,
                "affectedRelations":affected,
                "mandatoryIntersection":intersection,
                "ownerRelevant":owner_relevant,
                "targetRelevant":target_relevant,
                "pathRelevant":path_relevant,
                "ownerHash":canonical::hash(&owner).map_err(internal)?,
                "targetHash":canonical::hash(&target).map_err(internal)?,
                "refused":is_refused,
                "exclusionReason":if is_refused { "INTERSECTS_MANDATORY_RELATION" } else if path_relevant { "RELATION_DISJOINT" } else { "PATH_DISJOINT" },
            });
            if is_refused {
                refused.push(decision.clone());
            }
            decisions.push(decision);
        }
    }
    decisions.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
    refused.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
    let roots_hash = canonical::hash(roots).map_err(internal)?;
    let decisions_hash = canonical::hash(&decisions).map_err(internal)?;
    let refused_hash = canonical::hash(&refused).map_err(internal)?;
    let fingerprint = canonical::hash(&json!({
        "schema":"obligation-relative-boundary-closure/0.1",
        "rootsHash":roots_hash,
        "mandatoryRelations":mandatory_relations,
        "totalBoundaryCount":decisions.len(),
        "totalBoundarySetHash":decisions_hash,
        "relevantBoundaryCount":relevant_count,
        "refusedBoundaryCount":refused.len(),
        "refusedBoundarySetHash":refused_hash,
        "excludedByRelationCount":excluded_by_relation_count,
        "decisions":decisions,
    }))
    .map_err(internal)?;
    if !refused.is_empty() {
        let excluded = decisions
            .iter()
            .filter(|decision| decision["exclusionReason"] != "INTERSECTS_MANDATORY_RELATION")
            .cloned()
            .collect::<Vec<_>>();
        let safe_diagnostic = json!({
            "schema":"obligation-relative-boundary-diagnostic/0.1",
            "fingerprint":fingerprint,
            "totalBoundaryCount":decisions.len(),
            "relevantBoundaryCount":relevant_count,
            "refusedBoundaryCount":refused.len(),
            "refusedBoundarySetHash":canonical::hash(&refused).map_err(internal)?,
            "excludedBoundaryCount":excluded.len(),
            "excludedBoundarySetHash":canonical::hash(&excluded).map_err(internal)?,
            "refused":refused,
        });
        let mut error = ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "typed compiler boundary intersects the mandatory evidence closure",
        );
        error.evidence.push(fingerprint);
        error
            .evidence
            .push(canonical::hash(&refused).map_err(internal)?);
        error
            .evidence
            .push(serde_json::to_string(&safe_diagnostic).map_err(internal)?);
        return Err(error);
    }
    Ok(BoundaryClosureEvaluation {
        fingerprint,
        total_boundary_count: decisions.len(),
        relevant_boundary_count: relevant_count,
        refused_boundary_count: 0,
        excluded_by_relation_count,
        decisions,
    })
}

// The structured refusal carries the full fail-closed evidence needed by the
// caller; boxing it would make the internal error path less transparent.
#[allow(clippy::result_large_err)]
fn discover_declaration_type_candidates(
    repo: &Path,
    requested_compilation: Option<&str>,
    plan: &crate::semantic_goal::ConstraintExecutionPlan,
    worker: &mut WorkerClient,
) -> Result<Vec<DeclarationTypeCandidate>, (TypedGoalRefusalReason, DeclarationProviderRejection)> {
    let failure = |stage, reason, facts: &Value| {
        (
            reason,
            declaration_provider_rejection(stage, ErrorCode::IncompleteSemanticAnalysis, facts),
        )
    };
    let compilation = select_production_compilation(repo, requested_compilation).map_err(|_| {
        failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"step":"select-compilation"}),
        )
    })?;
    let verified = worker
        .index_files_verified(&json!({"repo":repo,"compilation":&compilation,"syntaxOnly":false}))
        .map_err(|_| {
            failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"step":"verified-index"}),
            )
        })?;
    let inspected = worker.inspect_verified_index(&verified).map_err(|_| {
        failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"step":"inspect-index"}),
        )
    })?;
    if inspected.get("k2Validated").and_then(Value::as_bool) != Some(true)
        || has_error_diagnostic(inspected)
    {
        return Err(failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"k2Validated":inspected.get("k2Validated"),"diagnosticsHash":canonical::hash(inspected.get("diagnostics").unwrap_or(&Value::Null)).ok()}),
        ));
    }
    let mut index = RepositoryIndex::open_compilation(repo, Some(&compilation)).map_err(|_| {
        failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"step":"open-index"}),
        )
    })?;
    index.update_verified(&verified, worker).map_err(|_| {
        failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"step":"persist-index"}),
        )
    })?;
    index.require_fresh(REPOSITORY_INDEX_FACT).map_err(|_| {
        failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"step":"fresh-index"}),
        )
    })?;
    let relations = index
        .declaration_relations()
        .map_err(|_| {
            failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"step":"relation-snapshot"}),
            )
        })?
        .ok_or_else(|| {
            failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"step":"missing-relations"}),
            )
        })?;
    let descriptors = index
        .declaration_descriptors()
        .map_err(|_| {
            failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"step":"descriptor-snapshot"}),
            )
        })?
        .ok_or_else(|| {
            failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"step":"missing-descriptors"}),
            )
        })?;
    let relation_rows_for_boundary = relations.graph["relations"].as_array().ok_or_else(|| {
        failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"step":"boundary-relation-rows"}),
        )
    })?;
    let mut roots = BTreeSet::<&str>::new();
    for row in relation_rows_for_boundary
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("OVERRIDES"))
    {
        roots.extend(row.get("owner").and_then(Value::as_str));
        roots.extend(row.get("target").and_then(Value::as_str));
    }
    loop {
        let before = roots.len();
        for row in relation_rows_for_boundary.iter().filter(|row| {
            matches!(
                row.get("kind").and_then(Value::as_str),
                Some("OVERRIDES" | "CALLS" | "REFERENCES")
            )
        }) {
            let owner = row.get("owner").and_then(Value::as_str);
            let target = row.get("target").and_then(Value::as_str);
            if owner.is_some_and(|value| roots.contains(value))
                || target.is_some_and(|value| roots.contains(value))
            {
                roots.extend(owner);
                roots.extend(target);
            }
        }
        if roots.len() == before {
            break;
        }
    }
    let mandatory_relations = plan
        .mandatory_closure
        .iter()
        .flat_map(|application| {
            constraint_op_spec(&application.operator)
                .required_evidence_relations
                .iter()
                .copied()
        })
        .collect::<BTreeSet<_>>();
    let mut boundary_decisions = Vec::new();
    let mut refused_boundaries = Vec::new();
    for (graph_kind, graph) in [
        ("RELATION", &relations.graph),
        ("DESCRIPTOR", &descriptors.graph),
    ] {
        let boundaries = graph
            .get("boundaries")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                failure(
                    DeclarationProviderStage::IndexCoverage,
                    TypedGoalRefusalReason::UnsupportedBoundary,
                    &json!({"graphKind":graph_kind,"reason":"missing-boundary-array"}),
                )
            })?;
        let coverage = graph.get("coverage").and_then(Value::as_str);
        if !matches!(coverage, Some("COMPLETE_SUPPORTED_SUBSET" | "PARTIAL"))
            || (coverage == Some("PARTIAL") && boundaries.is_empty())
        {
            return Err(failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::UnsupportedBoundary,
                &json!({"graphKind":graph_kind,"coverage":coverage,"boundaryCount":boundaries.len()}),
            ));
        }
        for boundary in boundaries {
            let affected = match boundary_affected_relations(boundary) {
                Ok(affected) => affected,
                Err(_) => {
                    return Err(failure(
                        DeclarationProviderStage::IndexCoverage,
                        TypedGoalRefusalReason::UnsupportedBoundary,
                        &json!({"graphKind":graph_kind,"boundaryHash":canonical::hash(boundary).ok(),"reason":"unmapped-boundary"}),
                    ));
                }
            };
            let owner = boundary.get("owner").and_then(Value::as_str);
            let target = boundary.get("target").and_then(Value::as_str);
            let path_relevant = owner.is_some_and(|value| roots.contains(value))
                || target.is_some_and(|value| roots.contains(value));
            let relation_intersection = affected
                .intersection(&mandatory_relations)
                .copied()
                .collect::<BTreeSet<_>>();
            let refused = path_relevant && !relation_intersection.is_empty();
            let decision = json!({
                "graphKind":graph_kind,
                "boundaryHash":canonical::hash(boundary).ok(),
                "affectedRelations":affected,
                "mandatoryIntersection":relation_intersection,
                "pathRelevant":path_relevant,
                "refused":refused,
                "exclusionReason":if refused { "INTERSECTS_MANDATORY_RELATION" } else if path_relevant { "RELATION_DISJOINT" } else { "PATH_DISJOINT" },
            });
            if refused {
                refused_boundaries.push(decision.clone());
            }
            boundary_decisions.push(decision);
        }
    }
    boundary_decisions.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
    refused_boundaries.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
    let boundary_closure_fingerprint = canonical::hash(&json!({
        "schema":"obligation-relative-boundary-closure/0.1",
        "mandatoryRelations":mandatory_relations,
        "totalBoundaryCount":boundary_decisions.len(),
        "totalBoundarySetHash":canonical::hash(&boundary_decisions).ok(),
        "refusedBoundaryCount":refused_boundaries.len(),
        "refusedBoundarySetHash":canonical::hash(&refused_boundaries).ok(),
        "decisions":boundary_decisions,
    }))
    .map_err(|_| {
        failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::InsufficientEvidence,
            &json!({"step":"boundary-closure-fingerprint"}),
        )
    })?;
    if !refused_boundaries.is_empty() {
        return Err(failure(
            DeclarationProviderStage::IndexCoverage,
            TypedGoalRefusalReason::UnsupportedBoundary,
            &json!({"boundaryClosureFingerprint":boundary_closure_fingerprint,"refused":refused_boundaries}),
        ));
    }
    let relation_rows = relations
        .graph
        .get("relations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"step":"relation-rows"}),
            )
        })?;
    let descriptor_rows = descriptors
        .graph
        .get("descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            failure(
                DeclarationProviderStage::IndexCoverage,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"step":"descriptor-rows"}),
            )
        })?;
    let mut by_callable = BTreeMap::<String, Vec<Value>>::new();
    for descriptor in descriptor_rows {
        if descriptor.get("declarationKind").and_then(Value::as_str) != Some("FUNCTION") {
            continue;
        }
        let callable =
            declaration_required_str(descriptor, "compilerCallableId").map_err(|reason| {
                failure(
                    DeclarationProviderStage::DescriptorIdentity,
                    reason,
                    &json!({"descriptorHash":canonical::hash(descriptor).ok()}),
                )
            })?;
        by_callable
            .entry(callable.to_owned())
            .or_default()
            .push(descriptor.clone());
    }
    let exact_descriptor = |callable: &str| -> Result<
        &Value,
        (TypedGoalRefusalReason, DeclarationProviderRejection),
    > {
        let rows = by_callable.get(callable).ok_or_else(|| {
            failure(
                DeclarationProviderStage::DescriptorIdentity,
                TypedGoalRefusalReason::InsufficientEvidence,
                &json!({"identityHash":canonical::hash(&callable).ok(),"count":0}),
            )
        })?;
        if rows.len() != 1 {
            return Err(failure(
                DeclarationProviderStage::DescriptorIdentity,
                TypedGoalRefusalReason::NoCompatibleBindings,
                &json!({"identityHash":canonical::hash(&callable).ok(),"count":rows.len()}),
            ));
        }
        Ok(&rows[0])
    };
    let overrides = relation_rows
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("OVERRIDES"))
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for seed in &overrides {
        let source_callable = declaration_required_str(seed, "owner")
            .map_err(|reason| {
                failure(
                    DeclarationProviderStage::OverrideRelation,
                    reason,
                    &json!({"relationHash":canonical::hash(seed).ok()}),
                )
            })?
            .to_owned();
        let target_callable = declaration_required_str(seed, "target")
            .map_err(|reason| {
                failure(
                    DeclarationProviderStage::OverrideRelation,
                    reason,
                    &json!({"relationHash":canonical::hash(seed).ok()}),
                )
            })?
            .to_owned();
        let source = exact_descriptor(&source_callable)?;
        let target = exact_descriptor(&target_callable)?;
        if !declaration_override_types_match(seed, source, target).map_err(|reason| {
            let facts = json!({"relationHash":canonical::hash(seed).ok(),"sourceHash":canonical::hash(source).ok(),"targetHash":canonical::hash(target).ok()});
            let mut rejection = declaration_provider_rejection(
                DeclarationProviderStage::TypeCompatibility,
                ErrorCode::IncompleteSemanticAnalysis,
                &facts,
            );
            rejection.type_shapes = Some(declaration_type_shape_diagnostic(seed, source, target));
            (reason, rejection)
        })? {
            let facts = json!({"relationHash":canonical::hash(seed).ok(),"sourceHash":canonical::hash(source).ok(),"targetHash":canonical::hash(target).ok()});
            let mut rejection = declaration_provider_rejection(
                DeclarationProviderStage::TypeCompatibility,
                ErrorCode::IncompleteSemanticAnalysis,
                &facts,
            );
            rejection.type_comparison = Some(declaration_type_comparison_diagnostic(
                seed, source, target,
            )?);
            return Err((TypedGoalRefusalReason::InsufficientEvidence, rejection));
        }
        if !declaration_is_internal(source) || !declaration_is_internal(target) {
            continue;
        }

        let mut affected = BTreeSet::from([source_callable.clone(), target_callable.clone()]);
        loop {
            let before = affected.len();
            for relation in relation_rows {
                let kind = relation
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !matches!(kind, "OVERRIDES" | "CALLS" | "REFERENCES") {
                    continue;
                }
                let owner = declaration_required_str(relation, "owner").map_err(|reason| {
                    failure(
                        DeclarationProviderStage::UseClosure,
                        reason,
                        &json!({"relationHash":canonical::hash(relation).ok()}),
                    )
                })?;
                let target = declaration_required_str(relation, "target").map_err(|reason| {
                    failure(
                        DeclarationProviderStage::UseClosure,
                        reason,
                        &json!({"relationHash":canonical::hash(relation).ok()}),
                    )
                })?;
                if kind == "OVERRIDES" && (affected.contains(owner) || affected.contains(target)) {
                    affected.insert(owner.to_owned());
                    affected.insert(target.to_owned());
                } else if matches!(kind, "CALLS" | "REFERENCES") && affected.contains(target) {
                    affected.insert(owner.to_owned());
                }
            }
            if affected.len() == before {
                break;
            }
        }

        let mut closure_descriptors = Vec::new();
        for callable in &affected {
            let descriptor = exact_descriptor(callable)?;
            if !declaration_is_internal(descriptor) {
                return Err(failure(
                    DeclarationProviderStage::AbiBoundary,
                    TypedGoalRefusalReason::UnsupportedBoundary,
                    &json!({"descriptorHash":canonical::hash(descriptor).ok()}),
                ));
            }
            if descriptor.get("isOverride").and_then(Value::as_bool) == Some(true)
                && overrides
                    .iter()
                    .filter(|row| {
                        row.get("owner").and_then(Value::as_str) == Some(callable.as_str())
                    })
                    .count()
                    != 1
            {
                return Err(failure(
                    DeclarationProviderStage::OverrideRelation,
                    TypedGoalRefusalReason::InsufficientEvidence,
                    &json!({"descriptorHash":canonical::hash(descriptor).ok(),"overrideCount":overrides.iter().filter(|row| row.get("owner").and_then(Value::as_str) == Some(callable.as_str())).count()}),
                ));
            }
            closure_descriptors.push(descriptor.clone());
        }
        let mut closure_relations = Vec::new();
        for relation in relation_rows {
            let kind = relation
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let owner = declaration_required_str(relation, "owner").map_err(|reason| {
                failure(
                    DeclarationProviderStage::UseClosure,
                    reason,
                    &json!({"relationHash":canonical::hash(relation).ok()}),
                )
            })?;
            let target_id = declaration_required_str(relation, "target").map_err(|reason| {
                failure(
                    DeclarationProviderStage::UseClosure,
                    reason,
                    &json!({"relationHash":canonical::hash(relation).ok()}),
                )
            })?;
            let connected = match kind {
                "OVERRIDES" => affected.contains(owner) || affected.contains(target_id),
                "CALLS" | "REFERENCES" => affected.contains(target_id),
                _ => false,
            };
            if !connected {
                continue;
            }
            let owner_descriptor = exact_descriptor(owner)?;
            let target_descriptor = exact_descriptor(target_id)?;
            if !declaration_is_internal(owner_descriptor)
                || !declaration_is_internal(target_descriptor)
            {
                return Err(failure(
                    DeclarationProviderStage::UseClosure,
                    TypedGoalRefusalReason::InsufficientEvidence,
                    &json!({"relationHash":canonical::hash(relation).ok(),"ownerHash":canonical::hash(owner_descriptor).ok(),"targetHash":canonical::hash(target_descriptor).ok()}),
                ));
            }
            let use_stage = match kind {
                "CALLS" => DeclarationProviderStage::UseCallType,
                "REFERENCES" => DeclarationProviderStage::UseReferenceType,
                _ => DeclarationProviderStage::UseClosure,
            };
            let use_shapes = declaration_use_shape_diagnostic(relation, target_descriptor);
            let types_match = declaration_use_types_match(relation, target_descriptor).map_err(|reason| {
                let mut rejection = declaration_provider_rejection(
                    use_stage,
                    ErrorCode::IncompleteSemanticAnalysis,
                    &json!({"relationHash":canonical::hash(relation).ok(),"targetHash":canonical::hash(target_descriptor).ok()}),
                );
                rejection.type_shapes = Some(use_shapes.clone());
                (reason, rejection)
            })?;
            if !types_match {
                let mut rejection = declaration_provider_rejection(
                    use_stage,
                    ErrorCode::IncompleteSemanticAnalysis,
                    &json!({"relationHash":canonical::hash(relation).ok(),"targetHash":canonical::hash(target_descriptor).ok()}),
                );
                rejection.type_shapes = Some(use_shapes);
                return Err((TypedGoalRefusalReason::InsufficientEvidence, rejection));
            }
            closure_relations.push(relation.clone());
        }
        closure_descriptors.sort_by_key(Value::to_string);
        closure_relations.sort_by_key(Value::to_string);
        let source_symbol = declaration_required_str(source, "symbolIdentity")
            .map_err(|reason| {
                failure(
                    DeclarationProviderStage::DescriptorIdentity,
                    reason,
                    &json!({"descriptorHash":canonical::hash(source).ok()}),
                )
            })?
            .to_owned();
        let target_symbol = declaration_required_str(target, "symbolIdentity")
            .map_err(|reason| {
                failure(
                    DeclarationProviderStage::DescriptorIdentity,
                    reason,
                    &json!({"descriptorHash":canonical::hash(target).ok()}),
                )
            })?
            .to_owned();
        candidates.push(DeclarationTypeCandidate {
            source_symbol,
            target_symbol,
            source_callable,
            target_callable,
            propagation_fingerprint: canonical::hash(&(
                &relations.hash,
                &descriptors.hash,
                seed,
                source,
                target,
            ))
            .map_err(|_| {
                failure(
                    DeclarationProviderStage::OverrideRelation,
                    TypedGoalRefusalReason::InsufficientEvidence,
                    &json!({"step":"propagation-fingerprint"}),
                )
            })?,
            override_fingerprint: canonical::hash(&(&relations.hash, &closure_relations)).map_err(
                |_| {
                    failure(
                        DeclarationProviderStage::OverrideRelation,
                        TypedGoalRefusalReason::InsufficientEvidence,
                        &json!({"step":"override-fingerprint"}),
                    )
                },
            )?,
            use_closure_fingerprint: canonical::hash(&(
                &relations.hash,
                &descriptors.hash,
                &closure_relations,
                &closure_descriptors,
            ))
            .map_err(|_| {
                failure(
                    DeclarationProviderStage::UseClosure,
                    TypedGoalRefusalReason::InsufficientEvidence,
                    &json!({"step":"use-fingerprint"}),
                )
            })?,
            contract_fingerprint: canonical::hash(&(
                &descriptors.hash,
                &closure_descriptors,
                "MODULE_API",
            ))
            .map_err(|_| {
                failure(
                    DeclarationProviderStage::AbiBoundary,
                    TypedGoalRefusalReason::InsufficientEvidence,
                    &json!({"step":"contract-fingerprint"}),
                )
            })?,
            boundary_closure_fingerprint: boundary_closure_fingerprint.clone(),
        });
    }
    candidates.sort_by(|left, right| {
        (&left.source_symbol, &left.target_symbol)
            .cmp(&(&right.source_symbol, &right.target_symbol))
    });
    candidates.dedup_by(|left, right| {
        left.source_symbol == right.source_symbol && left.target_symbol == right.target_symbol
    });
    Ok(candidates)
}

fn discover_nullable_construction_candidates(
    repo: &Path,
    requested_compilation: Option<&str>,
    worker: &mut WorkerClient,
) -> Result<Vec<NullableConstructionCandidate>, ClewError> {
    let compilation =
        select_production_compilation(repo, requested_compilation).map_err(|error| {
            nullable_discovery_attach(
                error,
                &nullable_early_diagnostic(
                    DeclarationProviderStage::WorkerAuthority,
                    requested_compilation,
                    None,
                    None,
                    None,
                    false,
                    false,
                ),
            )
        })?;
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":repo,"compilation":&compilation}),
        )
        .map_err(|error| {
            nullable_discovery_attach(
                error,
                &nullable_early_diagnostic(
                    DeclarationProviderStage::WorkerAuthority,
                    Some(&compilation),
                    None,
                    None,
                    None,
                    false,
                    false,
                ),
            )
        })?;
    require_exact_project_compilation(&project, &compilation).map_err(|error| {
        nullable_discovery_attach(
            error,
            &nullable_early_diagnostic(
                DeclarationProviderStage::WorkerAuthority,
                Some(&compilation),
                Some(&project),
                None,
                None,
                false,
                false,
            ),
        )
    })?;
    let verified = worker
        .index_files_verified_after_project(
            &json!({"repo":repo,"compilation":&compilation,"syntaxOnly":false}),
            &project,
        )
        .map_err(|error| {
            let base = nullable_early_diagnostic(
                DeclarationProviderStage::VerifiedIndexReceipt,
                Some(&compilation),
                Some(&project),
                None,
                None,
                false,
                false,
            );
            let diagnostic = nullable_verified_index_failure_diagnostic(&error, base);
            nullable_discovery_attach(error, &diagnostic)
        })?;
    let authority_diagnostic = worker.safe_verified_index_diagnostic(&verified);
    let inspected = worker.inspect_verified_index(&verified).map_err(|error| {
        nullable_discovery_attach(
            error,
            &nullable_early_diagnostic(
                DeclarationProviderStage::VerifiedIndexReceipt,
                Some(&compilation),
                Some(&project),
                None,
                Some(&authority_diagnostic),
                true,
                false,
            ),
        )
    })?;
    if inspected.get("k2Validated").and_then(Value::as_bool) != Some(true)
        || has_error_diagnostic(inspected)
    {
        let diagnostic = nullable_early_diagnostic(
            DeclarationProviderStage::K2Validation,
            Some(&compilation),
            Some(&project),
            Some(inspected),
            Some(&authority_diagnostic),
            true,
            true,
        );
        return Err(nullable_discovery_error(
            "nullable construction index is not compiler validated",
            &diagnostic,
        ));
    }
    let mut index = RepositoryIndex::open_compilation(repo, Some(&compilation))?;
    let index_snapshot = index.update_verified(&verified, worker)?;
    index.require_fresh(REPOSITORY_INDEX_FACT)?;
    let relations = index
        .declaration_relations()?
        .ok_or_else(|| invalid_source("nullable construction has no relation graph"))?;
    let descriptors = index
        .declaration_descriptors()?
        .ok_or_else(|| invalid_source("nullable construction has no descriptor graph"))?;
    let schema_diagnostic = nullable_graph_diagnostic(
        DeclarationProviderStage::SchemaProvenance,
        &relations.graph,
        &descriptors.graph,
    );
    for graph in [&relations.graph, &descriptors.graph] {
        if graph
            .get("provenance")
            .and_then(|value| value.get("extractorSchema"))
            .and_then(Value::as_str)
            != Some("fir-facts-extractor/0.6")
        {
            return Err(nullable_discovery_error(
                "nullable construction requires schema0.6 facts",
                &schema_diagnostic,
            ));
        }
    }
    let graph_diagnostic = nullable_graph_diagnostic(
        DeclarationProviderStage::GraphCoverage,
        &relations.graph,
        &descriptors.graph,
    );
    let relation_rows = relations.graph["relations"].as_array().ok_or_else(|| {
        nullable_discovery_error(
            "nullable construction relation rows are absent",
            &graph_diagnostic,
        )
    })?;
    let descriptor_rows = descriptors.graph["descriptors"].as_array().ok_or_else(|| {
        nullable_discovery_error(
            "nullable construction descriptors are absent",
            &graph_diagnostic,
        )
    })?;
    if !relation_rows
        .iter()
        .any(|row| row.get("kind").and_then(Value::as_str) == Some("NULL_COALESCES"))
    {
        let diagnostic = nullable_graph_diagnostic(
            DeclarationProviderStage::NullPolicyRelation,
            &relations.graph,
            &descriptors.graph,
        );
        return Err(nullable_discovery_error(
            "nullable construction has no compiler-proven null policy relation",
            &diagnostic,
        ));
    }
    let exact_descriptor = |kind: &str, callable: &str| -> Result<&Value, ClewError> {
        let matches = descriptor_rows
            .iter()
            .filter(|descriptor| {
                descriptor.get("declarationKind").and_then(Value::as_str) == Some(kind)
                    && descriptor.get("compilerCallableId").and_then(Value::as_str)
                        == Some(callable)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [only] => Ok(*only),
            _ => Err(invalid_source(
                "nullable construction declaration identity is missing or ambiguous",
            )),
        }
    };
    let declarations = inspected["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|file| file["declarations"].as_array().into_iter().flatten())
        .collect::<Vec<_>>();
    let owner_query = |owner: &str| -> Result<&str, ClewError> {
        let matches = declarations
            .iter()
            .filter(|declaration| {
                declaration.get("compilerSymbol").and_then(Value::as_str) == Some(owner)
            })
            .filter_map(|declaration| declaration.get("legacySymbolId").and_then(Value::as_str))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [only] => Ok(*only),
            _ => Err(invalid_source(
                "nullable construction owner is missing or compiler-ambiguous",
            )),
        }
    };
    let boundary_goal = TypedSemanticGoal::new(
        "boundary-closure",
        [
            ("source".into(), TypedVariableDomain::Declaration),
            ("fallback".into(), TypedVariableDomain::Declaration),
            ("destination".into(), TypedVariableDomain::Declaration),
        ],
        [OperatorApplication {
            operator: PrimitiveConstraint::NullHandles,
            operands: vec!["source".into(), "fallback".into(), "destination".into()],
        }],
    );
    let boundary_plan = boundary_goal
        .execution_plan()
        .map_err(|_| invalid_source("NULL_HANDLES mandatory closure is invalid"))?;
    let mandatory_boundary_relations = mandatory_relations_for_plan(&boundary_plan);
    let mut candidates = Vec::new();
    for null_policy in relation_rows
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("NULL_COALESCES"))
    {
        let owner = required_str(null_policy, "owner")?;
        let source_callable = required_str(null_policy, "sourceTarget")?;
        let fallback_callable = required_str(null_policy, "fallbackTarget")?;
        let source_descriptor_diagnostic = nullable_descriptor_diagnostic(
            DeclarationProviderStage::SourceDescriptor,
            null_policy,
            descriptor_rows,
            source_callable,
        );
        let source_descriptor = exact_descriptor("FUNCTION", source_callable).map_err(|_| {
            nullable_discovery_error(
                "nullable source descriptor identity is missing or ambiguous",
                &source_descriptor_diagnostic,
            )
        })?;
        let fallback_descriptor_diagnostic = nullable_descriptor_diagnostic(
            DeclarationProviderStage::FallbackDescriptor,
            null_policy,
            descriptor_rows,
            fallback_callable,
        );
        let fallback_descriptor =
            exact_descriptor("FUNCTION", fallback_callable).map_err(|_| {
                nullable_discovery_error(
                    "nullable fallback descriptor identity is missing or ambiguous",
                    &fallback_descriptor_diagnostic,
                )
            })?;
        if !declaration_is_internal(source_descriptor)
            || !declaration_is_internal(fallback_descriptor)
            || source_descriptor
                .get("returnNullable")
                .and_then(Value::as_bool)
                != Some(true)
            || fallback_descriptor
                .get("returnNullable")
                .and_then(Value::as_bool)
                != Some(false)
            || source_descriptor.get("returnType") != null_policy["sourceOccurrence"].get("type")
            || fallback_descriptor.get("returnType")
                != null_policy["fallbackOccurrence"].get("type")
        {
            return Err(invalid_source(
                "nullable construction declaration type/nullability contour is incomplete",
            ));
        }
        let merged_start = null_policy["mergedOccurrence"]["start"]
            .as_u64()
            .ok_or_else(|| invalid_source("null result has no exact occurrence"))?;
        let constructions = relation_rows
            .iter()
            .filter(|row| row.get("kind").and_then(Value::as_str) == Some("CONSTRUCTS"))
            .filter(|row| row.get("owner").and_then(Value::as_str) == Some(owner))
            .filter_map(|row| {
                let mappings = row["argumentToParameter"]
                    .as_array()?
                    .iter()
                    .filter(|mapping| {
                        mapping.get("argumentStart").and_then(Value::as_u64) == Some(merged_start)
                    })
                    .collect::<Vec<_>>();
                (mappings.len() == 1).then_some((row, mappings[0]))
            })
            .collect::<Vec<_>>();
        let [(construction, mapping)] = constructions.as_slice() else {
            return Err(invalid_source(
                "nullable construction occurrence does not map to one exact constructor slot",
            ));
        };
        let destination_callable = required_str(construction, "target")?;
        let slot_diagnostic = nullable_constructs_slot_diagnostic(
            null_policy,
            construction,
            mapping,
            relation_rows,
            descriptor_rows,
            owner,
            destination_callable,
        );
        let destination_descriptor = exact_descriptor("CONSTRUCTOR", destination_callable)
            .map_err(|_| {
                nullable_constructs_slot_error(
                    "nullable construction declaration identity is missing or ambiguous",
                    &slot_diagnostic,
                )
            })?;
        if !declaration_is_internal(destination_descriptor) {
            return Err(invalid_source(
                "nullable construction destination crosses an ABI boundary",
            ));
        }
        let slot_index = mapping["parameterIndex"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                nullable_constructs_slot_error(
                    "constructor mapping has no exact slot",
                    &slot_diagnostic,
                )
            })?;
        let slot = destination_descriptor["parameterTypes"]
            .as_array()
            .and_then(|parameters| parameters.get(slot_index))
            .ok_or_else(|| {
                nullable_constructs_slot_error("constructor slot is absent", &slot_diagnostic)
            })?;
        let source_type = required_str(&null_policy["sourceOccurrence"], "type")?;
        let fallback_type = required_str(&null_policy["fallbackOccurrence"], "type")?;
        let merged_type = required_str(&null_policy["mergedOccurrence"], "type")?;
        if source_type.trim_end_matches('?') != merged_type
            || fallback_type != merged_type
            || mapping.get("parameterType").and_then(Value::as_str) != Some(merged_type)
            || slot.get("type").and_then(Value::as_str) != Some(merged_type)
            || slot.get("nullable").and_then(Value::as_bool) != Some(false)
            || mapping.get("argumentStart").and_then(Value::as_u64) != Some(merged_start)
        {
            return Err(invalid_source(
                "nullable construction occurrence type/nullability or slot mapping is incomplete",
            ));
        }
        let boundary_roots = BTreeSet::from([
            owner.to_owned(),
            source_callable.to_owned(),
            fallback_callable.to_owned(),
            destination_callable.to_owned(),
            required_str(source_descriptor, "symbolIdentity")?.to_owned(),
            required_str(fallback_descriptor, "symbolIdentity")?.to_owned(),
            required_str(destination_descriptor, "symbolIdentity")?.to_owned(),
        ]);
        let boundary_evaluation = evaluate_obligation_relative_boundaries(
            &[
                ("RELATION", &relations.graph),
                ("DESCRIPTOR", &descriptors.graph),
            ],
            &mandatory_boundary_relations,
            &boundary_roots,
        )
        .map_err(|error| {
            let mut diagnostic = nullable_graph_diagnostic(
                DeclarationProviderStage::UnknownBoundary,
                &relations.graph,
                &descriptors.graph,
            );
            if let Some(boundary) = error.evidence.iter().find_map(|encoded| {
                serde_json::from_str::<Value>(encoded).ok().filter(|value| {
                    value.get("schema").and_then(Value::as_str)
                        == Some("obligation-relative-boundary-diagnostic/0.1")
                })
            }) {
                for field in [
                    "totalBoundaryCount",
                    "relevantBoundaryCount",
                    "refusedBoundaryCount",
                    "excludedBoundaryCount",
                ] {
                    if let Some(count) = boundary
                        .get(field)
                        .and_then(Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                    {
                        diagnostic.cardinalities.insert(field.into(), count);
                    }
                }
                for field in [
                    "fingerprint",
                    "refusedBoundarySetHash",
                    "excludedBoundarySetHash",
                ] {
                    if let Some(value) = boundary.get(field).and_then(Value::as_str) {
                        diagnostic.fact_relations.insert(field.into(), value.into());
                    }
                }
                if let Some(refused) = boundary.get("refused") {
                    diagnostic
                        .fact_relations
                        .insert("refusedSubset".into(), refused.to_string());
                }
            }
            if let Some(classification) = error.evidence.iter().find_map(|encoded| {
                serde_json::from_str::<Value>(encoded).ok().filter(|value| {
                    value.get("schema").and_then(Value::as_str)
                        == Some("boundary-classification-diagnostic/0.1")
                })
            }) {
                for field in [
                    "graphKind",
                    "provider",
                    "stage",
                    "code",
                    "boundarySchema",
                    "rowHash",
                ] {
                    if let Some(value) = classification.get(field).and_then(Value::as_str) {
                        diagnostic
                            .fact_relations
                            .insert(format!("unmappedBoundary.{field}"), value.into());
                    }
                }
            }
            nullable_discovery_attach(error, &diagnostic)
        })?;
        let boundary_closure_fingerprint = canonical::hash(&json!({
            "schema":"nullable-boundary-closure/0.1",
            "evaluationFingerprint":boundary_evaluation.fingerprint,
            "rootsHash":canonical::hash(&boundary_roots).map_err(internal)?,
            "constructorSlotIndex":slot_index,
            "constructorSlotHash":canonical::hash(slot).map_err(internal)?,
        }))
        .map_err(internal)?;
        let source_start = null_policy["sourceOccurrence"]["start"]
            .as_u64()
            .ok_or_else(|| invalid_source("nullable source has no exact range"))?;
        let fallback_start = null_policy["fallbackOccurrence"]["start"]
            .as_u64()
            .ok_or_else(|| invalid_source("fallback has no exact range"))?;
        let exact_calls = |target: &str, start: u64| {
            relation_rows
                .iter()
                .filter(|row| row.get("kind").and_then(Value::as_str) == Some("CALLS"))
                .filter(|row| row.get("owner").and_then(Value::as_str) == Some(owner))
                .filter(|row| row.get("target").and_then(Value::as_str) == Some(target))
                .filter(|row| row.get("start").and_then(Value::as_u64) == Some(start))
                .collect::<Vec<_>>()
        };
        let source_calls = exact_calls(source_callable, source_start);
        let fallback_calls = exact_calls(fallback_callable, fallback_start);
        let ([source_call], [fallback_call]) = (source_calls.as_slice(), fallback_calls.as_slice())
        else {
            return Err(invalid_source(
                "nullable source or fallback evaluation is not unique",
            ));
        };
        let affected_targets =
            BTreeSet::from([source_callable, fallback_callable, destination_callable]);
        let mut use_rows = Vec::new();
        let mut use_descriptors = Vec::new();
        for relation in relation_rows.iter().filter(|row| {
            matches!(
                row.get("kind").and_then(Value::as_str),
                Some("CALLS" | "REFERENCES" | "CONSTRUCTS")
            ) && row
                .get("target")
                .and_then(Value::as_str)
                .is_some_and(|target| affected_targets.contains(target))
        }) {
            let use_owner = required_str(relation, "owner")?;
            let descriptor = exact_descriptor("FUNCTION", use_owner)?;
            if !declaration_is_internal(descriptor) {
                return Err(invalid_source(
                    "nullable construction use closure crosses an ABI boundary",
                ));
            }
            use_rows.push((*relation).clone());
            use_descriptors.push(descriptor.clone());
        }
        use_rows.sort_by_key(Value::to_string);
        use_descriptors.sort_by_key(Value::to_string);

        let query = owner_query(owner)?;
        let raw = worker.request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":repo,"symbol":query,"compilation":&compilation}),
        )?;
        let graph = graph::enrich(serde_json::from_value::<LocalGraph>(raw).map_err(|error| {
            ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
        })?);
        if !graph.boundaries.is_empty() || !graph.diagnostics.is_empty() {
            return Err(invalid_source(
                "nullable construction local graph has an unsupported boundary",
            ));
        }
        let source_nodes = exact_relation_call_nodes(&graph, source_call)?;
        let fallback_nodes = exact_relation_call_nodes(&graph, fallback_call)?;
        let destination_nodes = exact_relation_call_nodes(&graph, construction)?;
        let ([source_node], [fallback_node], [destination_node]) = (
            source_nodes.as_slice(),
            fallback_nodes.as_slice(),
            destination_nodes.as_slice(),
        ) else {
            return Err(invalid_source(
                "nullable construction call occurrence is missing or ambiguous",
            ));
        };
        let policy = occurrence_slice_policy();
        let snapshot = value_flow_snapshot(repo, &compilation, &project, &index_snapshot, worker)?;
        let thread = graph::slice(
            &graph,
            &destination_node.id,
            policy,
            snapshot,
            json!({
                "kind":"NULLABLE_CONSTRUCTION_OCCURRENCE",
                "symbol":query,
                "nodeId":destination_node.id,
            }),
        )
        .map_err(internal)?;
        verify_occurrence_thread_closure(
            &thread,
            &[
                source_node.id.as_str(),
                fallback_node.id.as_str(),
                destination_node.id.as_str(),
            ],
        )?;
        let rebuilt =
            transaction::rebuild_thread(repo, &thread, &project, &git_head(repo)?, worker)?;
        if canonical::hash(&thread).map_err(internal)?
            != canonical::hash(&rebuilt).map_err(internal)?
        {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                "nullable construction Thread IR changed during binding",
            ));
        }
        verify_live_sources(repo, &thread)?;
        if !thread_node_dominates(&thread, &source_node.id, &destination_node.id) {
            return Err(invalid_source(
                "nullable source occurrence does not dominate construction",
            ));
        }
        let null_cfg_nodes = null_policy["cfgNodeIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|id| format!("fir:{id}"))
            .collect::<Vec<_>>();
        if null_cfg_nodes.is_empty()
            || !null_cfg_nodes.iter().any(|node| {
                thread.nodes.iter().any(|candidate| candidate.id == *node)
                    && thread_node_dominates(&thread, node, &destination_node.id)
            })
        {
            return Err(invalid_source(
                "null merge has no compiler CFG node dominating construction",
            ));
        }
        let source_symbol = required_str(source_descriptor, "symbolIdentity")?.to_owned();
        let fallback_symbol = required_str(fallback_descriptor, "symbolIdentity")?.to_owned();
        let destination_symbol = required_str(destination_descriptor, "symbolIdentity")?.to_owned();
        let occurrence = NullableConstructionOccurrence {
            owner_callable: owner.into(),
            slot_index,
            module: project
                .get("module")
                .and_then(Value::as_str)
                .unwrap_or(":")
                .into(),
            source_set: project
                .get("sourceSet")
                .and_then(Value::as_str)
                .unwrap_or("main")
                .into(),
            source_range: (
                source_start,
                null_policy["sourceOccurrence"]["end"]
                    .as_u64()
                    .ok_or_else(|| invalid_source("nullable source has no exact range end"))?,
            ),
            fallback_range: (
                fallback_start,
                null_policy["fallbackOccurrence"]["end"]
                    .as_u64()
                    .ok_or_else(|| invalid_source("fallback has no exact range end"))?,
            ),
            result_range: (
                merged_start,
                null_policy["mergedOccurrence"]["end"]
                    .as_u64()
                    .ok_or_else(|| invalid_source("null result has no exact range end"))?,
            ),
            construction_range: (
                construction["start"]
                    .as_u64()
                    .ok_or_else(|| invalid_source("construction has no exact range"))?,
                construction["end"]
                    .as_u64()
                    .ok_or_else(|| invalid_source("construction has no exact range end"))?,
            ),
            thread_id: thread.thread_id.clone(),
            null_policy_fingerprint: canonical::hash(&(&relations.hash, null_policy))
                .map_err(internal)?,
            construction_fingerprint: canonical::hash(&(
                &relations.hash,
                &descriptors.hash,
                construction,
                mapping,
                destination_descriptor,
            ))
            .map_err(internal)?,
            value_flow_fingerprint: canonical::hash(&(
                source_call,
                fallback_call,
                null_policy,
                construction,
            ))
            .map_err(internal)?,
            use_closure_fingerprint: canonical::hash(&(
                &relations.hash,
                &descriptors.hash,
                &use_rows,
                &use_descriptors,
            ))
            .map_err(internal)?,
            contract_fingerprint: canonical::hash(&(
                source_descriptor,
                fallback_descriptor,
                destination_descriptor,
                "MODULE_API",
            ))
            .map_err(internal)?,
            thread_fingerprint: canonical::hash(&thread).map_err(internal)?,
            read_set_fingerprint: canonical::hash(&thread.read_set).map_err(internal)?,
            provenance_fingerprint: canonical::hash(&(
                &relations.provenance,
                &descriptors.provenance,
                &thread.snapshot,
            ))
            .map_err(internal)?,
            boundary_closure_fingerprint,
        };
        let occurrence_fingerprint = canonical::hash(&occurrence).map_err(internal)?;
        candidates.push(NullableConstructionCandidate {
            source_symbol,
            fallback_symbol,
            destination_symbol,
            source_callable: source_callable.into(),
            fallback_callable: fallback_callable.into(),
            destination_callable: destination_callable.into(),
            module: project
                .get("module")
                .and_then(Value::as_str)
                .unwrap_or(":")
                .into(),
            source_set: project
                .get("sourceSet")
                .and_then(Value::as_str)
                .unwrap_or("main")
                .into(),
            use_closure_fingerprint: occurrence.use_closure_fingerprint.clone(),
            contract_fingerprint: occurrence.contract_fingerprint.clone(),
            provenance_fingerprint: occurrence.provenance_fingerprint.clone(),
            occurrences: vec![occurrence],
            occurrence_fingerprints: vec![occurrence_fingerprint.clone()],
            occurrence_set_fingerprint: canonical::hash(&vec![occurrence_fingerprint])
                .map_err(internal)?,
        });
    }
    candidates.sort_by(|left, right| {
        (
            &left.source_symbol,
            &left.fallback_symbol,
            &left.destination_symbol,
        )
            .cmp(&(
                &right.source_symbol,
                &right.fallback_symbol,
                &right.destination_symbol,
            ))
    });
    let expected_occurrences = relation_rows
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("NULL_COALESCES"))
        .count();
    let mut grouped = Vec::<NullableConstructionCandidate>::new();
    for mut candidate in candidates {
        if let Some(existing) = grouped.last_mut().filter(|existing| {
            existing.source_symbol == candidate.source_symbol
                && existing.fallback_symbol == candidate.fallback_symbol
                && existing.destination_symbol == candidate.destination_symbol
        }) {
            if existing.source_callable != candidate.source_callable
                || existing.fallback_callable != candidate.fallback_callable
                || existing.destination_callable != candidate.destination_callable
                || existing.module != candidate.module
                || existing.source_set != candidate.source_set
            {
                return Err(invalid_source(
                    "nullable construction occurrence set has mixed compiler identity or compilation",
                ));
            }
            existing.occurrences.append(&mut candidate.occurrences);
        } else {
            grouped.push(candidate);
        }
    }
    for candidate in &mut grouped {
        let mut keyed_occurrences = candidate
            .occurrences
            .drain(..)
            .map(|occurrence| {
                Ok((
                    canonical::hash(&occurrence).map_err(internal)?,
                    canonical::bytes(&occurrence).map_err(internal)?,
                    occurrence,
                ))
            })
            .collect::<Result<Vec<_>, ClewError>>()?;
        keyed_occurrences.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
        for pair in keyed_occurrences.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(invalid_source(if pair[0].1 == pair[1].1 {
                    "nullable construction occurrence set contains a duplicate"
                } else {
                    "nullable construction occurrence fingerprint collision"
                }));
            }
        }
        candidate.occurrence_fingerprints = keyed_occurrences
            .iter()
            .map(|(fingerprint, _, _)| fingerprint.clone())
            .collect();
        candidate.occurrences = keyed_occurrences
            .into_iter()
            .map(|(_, _, occurrence)| occurrence)
            .collect();
        candidate.occurrence_set_fingerprint =
            canonical::hash(&candidate.occurrence_fingerprints).map_err(internal)?;
        candidate.use_closure_fingerprint = canonical::hash(
            &candidate
                .occurrences
                .iter()
                .map(|occurrence| &occurrence.use_closure_fingerprint)
                .collect::<Vec<_>>(),
        )
        .map_err(internal)?;
        candidate.contract_fingerprint = canonical::hash(
            &candidate
                .occurrences
                .iter()
                .map(|occurrence| &occurrence.contract_fingerprint)
                .collect::<Vec<_>>(),
        )
        .map_err(internal)?;
        candidate.provenance_fingerprint = canonical::hash(
            &candidate
                .occurrences
                .iter()
                .map(|occurrence| &occurrence.provenance_fingerprint)
                .collect::<Vec<_>>(),
        )
        .map_err(internal)?;
    }
    if grouped
        .iter()
        .map(|candidate| candidate.occurrences.len())
        .sum::<usize>()
        != expected_occurrences
    {
        return Err(invalid_source(
            "nullable construction occurrence set is incomplete for the verified relation graph",
        ));
    }
    Ok(grouped)
}

/// Discovers only the currently provable direct occurrence contour:
/// the result occurrence of one exact resolved declaration call is mapped by
/// the compiler to one exact value-parameter slot of another declaration call
/// or construction in the same owner. Local-variable aliases and general path
/// synthesis are intentionally refused.
fn discover_declaration_value_flow_candidates(
    repo: &Path,
    requested_compilation: Option<&str>,
    worker: &mut WorkerClient,
) -> Result<Vec<DeclarationValueFlowCandidate>, ClewError> {
    let compilation = select_production_compilation(repo, requested_compilation)?;
    let project = worker.request(
        RequestKind::OpenProject,
        &json!({"repo":repo,"compilation":&compilation}),
    )?;
    require_exact_project_compilation(&project, &compilation)?;
    let verified = worker.index_files_verified_after_project(
        &json!({"repo":repo,"compilation":&compilation,"syntaxOnly":false}),
        &project,
    )?;
    let inspected = worker.inspect_verified_index(&verified)?;
    if inspected.get("k2Validated").and_then(Value::as_bool) != Some(true)
        || has_error_diagnostic(inspected)
    {
        return Err(projection_discovery_error(
            "value-flow index is not compiler validated",
            DeclarationProviderStage::VerifiedIndexReceipt,
            BTreeMap::from([(
                "diagnosticCount".into(),
                inspected["diagnostics"].as_array().map_or(0, Vec::len),
            )]),
            BTreeMap::from([(
                "diagnosticCodes".into(),
                canonical::hash(&inspected["diagnostics"]).unwrap_or_default(),
            )]),
            BTreeMap::from([(
                "k2Validated".into(),
                format!(
                    "{:?}",
                    inspected.get("k2Validated").and_then(Value::as_bool)
                ),
            )]),
        ));
    }
    let mut index = RepositoryIndex::open_compilation(repo, Some(&compilation))?;
    let index_snapshot = index.update_verified(&verified, worker)?;
    index.require_fresh(REPOSITORY_INDEX_FACT)?;
    let relations = index
        .declaration_relations()?
        .ok_or_else(|| invalid_source("value-flow index has no relation graph"))?;
    let descriptors = index
        .declaration_descriptors()?
        .ok_or_else(|| invalid_source("value-flow index has no descriptor graph"))?;
    let relation_rows = relations
        .graph
        .get("relations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("value-flow relation graph has no rows"))?;
    let descriptor_rows = descriptors
        .graph
        .get("descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("value-flow descriptor graph has no rows"))?;
    // Candidate roots come only from exact compiler-proven source CALL
    // occurrences consumed by an exact CALL/CONSTRUCTS parameter slot. This
    // is deliberately established before consulting Unknown boundaries.
    let mut boundary_roots = BTreeSet::<String>::new();
    for source in relation_rows
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("CALLS"))
    {
        let (Some(owner), Some(source_target), Some(source_start)) = (
            source.get("owner").and_then(Value::as_str),
            source.get("target").and_then(Value::as_str),
            source.get("start").and_then(Value::as_u64),
        ) else {
            continue;
        };
        for destination in relation_rows.iter().filter(|row| {
            matches!(
                row.get("kind").and_then(Value::as_str),
                Some("CALLS" | "CONSTRUCTS")
            ) && row.get("owner").and_then(Value::as_str) == Some(owner)
        }) {
            let exact_slots = destination
                .get("argumentToParameter")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|mapping| {
                    mapping.get("argumentStart").and_then(Value::as_u64) == Some(source_start)
                        && mapping
                            .get("parameterIndex")
                            .and_then(Value::as_u64)
                            .is_some()
                })
                .count();
            if exact_slots == 1 {
                boundary_roots.extend([
                    owner.to_owned(),
                    source_target.to_owned(),
                    required_str(destination, "target")?.to_owned(),
                ]);
            }
        }
    }
    let boundary_evaluation = evaluate_obligation_relative_boundaries(
        &[
            ("RELATION", &relations.graph),
            ("DESCRIPTOR", &descriptors.graph),
        ],
        &BTreeSet::from([EvidenceRelation::DeclarationValueFlow]),
        &boundary_roots,
    )?;
    let mut by_callable = BTreeMap::<String, Vec<&Value>>::new();
    for descriptor in descriptor_rows {
        if descriptor.get("declarationKind").and_then(Value::as_str) == Some("FUNCTION") {
            let callable = declaration_required_str(descriptor, "compilerCallableId")
                .map_err(|_| invalid_source("descriptor has no compiler callable identity"))?;
            by_callable
                .entry(callable.into())
                .or_default()
                .push(descriptor);
        }
    }
    let exact_descriptor = |callable: &str| -> Result<&Value, ClewError> {
        let candidates = by_callable
            .get(callable)
            .ok_or_else(|| invalid_source("value-flow callable has no declaration descriptor"))?;
        if candidates.len() != 1 {
            return Err(invalid_source(
                "value-flow callable descriptor is overload-ambiguous",
            ));
        }
        Ok(candidates[0])
    };
    let declarations = inspected
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|file| {
            file.get("declarations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .collect::<Vec<_>>();
    let owner_query = |owner: &str| -> Result<&str, ClewError> {
        let candidates = declarations
            .iter()
            .filter(|declaration| {
                declaration.get("compilerSymbol").and_then(Value::as_str) == Some(owner)
            })
            .filter_map(|declaration| declaration.get("legacySymbolId").and_then(Value::as_str))
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [only] => Ok(*only),
            _ => Err(invalid_source(
                "value-flow owner does not resolve to one compiler declaration",
            )),
        }
    };

    let mut owners = relation_rows
        .iter()
        .filter(|row| {
            matches!(
                row.get("kind").and_then(Value::as_str),
                Some("CALLS" | "CONSTRUCTS")
            )
        })
        .filter_map(|row| row.get("owner").and_then(Value::as_str))
        .collect::<Vec<_>>();
    owners.sort();
    owners.dedup();
    let mut candidates = Vec::new();
    for owner in owners {
        let owner_relations = relation_rows
            .iter()
            .filter(|row| row.get("owner").and_then(Value::as_str) == Some(owner))
            .filter(|row| {
                matches!(
                    row.get("kind").and_then(Value::as_str),
                    Some("CALLS" | "CONSTRUCTS")
                )
            })
            .collect::<Vec<_>>();
        if owner_relations.len() < 2 {
            continue;
        }
        let query = owner_query(owner)?;
        let raw = worker.request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":repo,"symbol":query,"compilation":&compilation}),
        )?;
        let graph = graph::enrich(serde_json::from_value::<LocalGraph>(raw).map_err(|error| {
            ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
        })?);
        if !graph.boundaries.is_empty() || !graph.diagnostics.is_empty() {
            return Err(projection_discovery_error(
                "value-flow local graph has compiler diagnostics or boundaries",
                DeclarationProviderStage::CallBoundary,
                BTreeMap::from([
                    ("boundaryCount".into(), graph.boundaries.len()),
                    ("diagnosticCount".into(), graph.diagnostics.len()),
                ]),
                BTreeMap::from([
                    (
                        "boundaryKinds".into(),
                        canonical::hash(&graph.boundaries).unwrap_or_default(),
                    ),
                    (
                        "diagnosticKinds".into(),
                        canonical::hash(&graph.diagnostics).unwrap_or_default(),
                    ),
                ]),
                BTreeMap::new(),
            ));
        }
        for source_relation in &owner_relations {
            if source_relation.get("kind").and_then(Value::as_str) != Some("CALLS") {
                continue;
            }
            let source_callable = required_str(source_relation, "target")?;
            let source_start = source_relation
                .get("start")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid_source("source occurrence has no compiler range"))?;
            let source_nodes = exact_relation_call_nodes(&graph, source_relation)?;
            let [source_node] = source_nodes.as_slice() else {
                return Err(projection_discovery_error(
                    "source occurrence is missing or alias-ambiguous in Thread IR",
                    DeclarationProviderStage::SourceCallIdentity,
                    BTreeMap::from([("candidateCount".into(), source_nodes.len())]),
                    BTreeMap::from([
                        (
                            "relation".into(),
                            canonical::hash(source_relation).unwrap_or_default(),
                        ),
                        (
                            "nodeKinds".into(),
                            canonical::hash(
                                &source_nodes
                                    .iter()
                                    .map(|node| &node.kind)
                                    .collect::<Vec<_>>(),
                            )
                            .unwrap_or_default(),
                        ),
                    ]),
                    BTreeMap::new(),
                ));
            };
            for destination_relation in &owner_relations {
                if std::ptr::eq(*source_relation, *destination_relation) {
                    continue;
                }
                let mapping_rows = destination_relation
                    .get("argumentToParameter")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|mapping| {
                        mapping.get("argumentStart").and_then(Value::as_u64) == Some(source_start)
                    })
                    .collect::<Vec<_>>();
                let [mapping] = mapping_rows.as_slice() else {
                    continue;
                };
                let slot_index = mapping
                    .get("parameterIndex")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| invalid_source("destination occurrence has no exact slot"))?;
                let destination_nodes = exact_relation_call_nodes(&graph, destination_relation)?;
                let [destination_node] = destination_nodes.as_slice() else {
                    return Err(projection_discovery_error(
                        "destination occurrence is missing or alias-ambiguous in Thread IR",
                        DeclarationProviderStage::DestCallIdentity,
                        BTreeMap::from([("candidateCount".into(), destination_nodes.len())]),
                        BTreeMap::from([
                            (
                                "relation".into(),
                                canonical::hash(destination_relation).unwrap_or_default(),
                            ),
                            (
                                "nodeKinds".into(),
                                canonical::hash(
                                    &destination_nodes
                                        .iter()
                                        .map(|node| &node.kind)
                                        .collect::<Vec<_>>(),
                                )
                                .unwrap_or_default(),
                            ),
                        ]),
                        BTreeMap::from([("slotIndex".into(), slot_index.to_string())]),
                    ));
                };
                let source_origin = source_node
                    .origin
                    .as_ref()
                    .ok_or_else(|| invalid_source("resolved source call has no compiler origin"))?;
                let source_range = source_origin
                    .get("rangeHint")
                    .ok_or_else(|| invalid_source("resolved source call has no exact range"))?;
                let occurrence_nodes = graph
                    .nodes
                    .iter()
                    .filter(|node| node.origin.as_ref() == Some(source_origin))
                    .filter(|node| {
                        node.origin
                            .as_ref()
                            .and_then(|origin| origin.get("rangeHint"))
                            == Some(source_range)
                    })
                    .filter(|node| {
                        node.attributes.get("analysis").and_then(Value::as_str) == Some("K2_FIR")
                    })
                    .filter(|node| {
                        graph.edges.iter().any(|edge| {
                            edge.from == node.id
                                && edge.to == destination_node.id
                                && edge.kind == "ARG_PARAM"
                        })
                    })
                    .collect::<Vec<_>>();
                let [source_occurrence] = occurrence_nodes.as_slice() else {
                    return Err(projection_discovery_error(
                        "compiler origin maps to zero or multiple argument occurrences",
                        DeclarationProviderStage::SourceOccurrence,
                        BTreeMap::from([
                            ("sourceCallCount".into(), source_nodes.len()),
                            ("destinationCallCount".into(), destination_nodes.len()),
                            ("occurrenceCount".into(), occurrence_nodes.len()),
                        ]),
                        BTreeMap::from([
                            (
                                "origin".into(),
                                canonical::hash(source_origin).unwrap_or_default(),
                            ),
                            (
                                "range".into(),
                                canonical::hash(source_range).unwrap_or_default(),
                            ),
                            (
                                "nodeKinds".into(),
                                canonical::hash(
                                    &occurrence_nodes
                                        .iter()
                                        .map(|node| &node.kind)
                                        .collect::<Vec<_>>(),
                                )
                                .unwrap_or_default(),
                            ),
                        ]),
                        BTreeMap::from([("slotIndex".into(), slot_index.to_string())]),
                    ));
                };
                for kind in ["ARG_PARAM", "DEF_USE"] {
                    if graph
                        .edges
                        .iter()
                        .filter(|edge| {
                            edge.from == source_occurrence.id
                                && edge.to == destination_node.id
                                && edge.kind == kind
                        })
                        .count()
                        != 1
                    {
                        let diagnostic = json!({
                            "requiredKind": kind,
                            "sourceCandidateCardinality": source_nodes.len(),
                            "occurrenceCandidateCardinality": occurrence_nodes.len(),
                            "destinationCandidateCardinality": destination_nodes.len(),
                            "sourceCallKind": source_node.kind,
                            "occurrenceKind": source_occurrence.kind,
                            "sourceOriginHash": canonical::hash(source_origin).ok(),
                            "sourceRangeHash": canonical::hash(source_range).ok(),
                        });
                        return Err(projection_discovery_error(
                            "compiler-mapped occurrence has no unique direct value-flow edge",
                            if kind == "ARG_PARAM" {
                                DeclarationProviderStage::ArgParamEdge
                            } else {
                                DeclarationProviderStage::DefUseEdge
                            },
                            BTreeMap::from([
                                ("sourceCallCount".into(), source_nodes.len()),
                                ("destinationCallCount".into(), destination_nodes.len()),
                                ("occurrenceCount".into(), occurrence_nodes.len()),
                                (
                                    "edgeCount".into(),
                                    graph
                                        .edges
                                        .iter()
                                        .filter(|edge| {
                                            edge.from == source_occurrence.id
                                                && edge.to == destination_node.id
                                                && edge.kind == kind
                                        })
                                        .count(),
                                ),
                            ]),
                            BTreeMap::from([
                                (
                                    "origin".into(),
                                    canonical::hash(source_origin).unwrap_or_default(),
                                ),
                                (
                                    "range".into(),
                                    canonical::hash(source_range).unwrap_or_default(),
                                ),
                                (
                                    "edgeShape".into(),
                                    canonical::hash(&diagnostic).unwrap_or_default(),
                                ),
                            ]),
                            BTreeMap::from([
                                ("sourceKind".into(), source_node.kind.clone()),
                                ("occurrenceKind".into(), source_occurrence.kind.clone()),
                                ("slotIndex".into(), slot_index.to_string()),
                            ]),
                        ));
                    }
                }
                let source_descriptor = exact_descriptor(source_callable)?;
                let destination_callable = required_str(destination_relation, "target")?;
                let destination_descriptor = exact_descriptor(destination_callable)?;
                let source_type = required_str(source_relation, "resultType")?.to_owned();
                if source_type != required_str(source_descriptor, "returnType")?
                    || mapping.get("argumentType").and_then(Value::as_str)
                        != Some(source_type.as_str())
                {
                    return Err(invalid_source(
                        "source occurrence type is not compiler-consistent",
                    ));
                }
                let parameters = destination_descriptor
                    .get("parameterTypes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid_source("destination descriptor has no parameters"))?;
                let slot = parameters
                    .get(slot_index)
                    .ok_or_else(|| invalid_source("destination parameter slot is out of range"))?;
                let destination_type = required_str(slot, "type")?.to_owned();
                if mapping.get("parameterType").and_then(Value::as_str)
                    != Some(destination_type.as_str())
                    || source_type != destination_type
                {
                    return Err(invalid_source(
                        "source occurrence is not assignable to exact destination slot",
                    ));
                }
                let policy = occurrence_slice_policy();
                let snapshot =
                    value_flow_snapshot(repo, &compilation, &project, &index_snapshot, worker)?;
                let seed = json!({
                    "kind":"DECLARATION_VALUE_FLOW_OCCURRENCE",
                    "symbol":query,
                    "nodeId":destination_node.id,
                });
                let thread = graph::slice(&graph, &destination_node.id, policy, snapshot, seed)
                    .map_err(internal)?;
                verify_occurrence_thread_closure(
                    &thread,
                    &[source_node.id.as_str(), destination_node.id.as_str()],
                )
                .map_err(|error| {
                    projection_discovery_error(
                        "value-flow Thread boundary closure refused",
                        DeclarationProviderStage::CallBoundary,
                        BTreeMap::from([
                            ("boundaryCount".into(), thread.completeness.boundaries.len()),
                            ("summaryCount".into(), thread.external_summaries.len()),
                        ]),
                        BTreeMap::from([
                            (
                                "thread".into(),
                                canonical::hash(&thread).unwrap_or_default(),
                            ),
                            (
                                "upstream".into(),
                                canonical::hash(&error.message).unwrap_or_default(),
                            ),
                        ]),
                        BTreeMap::from([("slotIndex".into(), slot_index.to_string())]),
                    )
                })?;
                let rebuilt =
                    transaction::rebuild_thread(repo, &thread, &project, &git_head(repo)?, worker)?;
                if canonical::hash(&thread).map_err(internal)?
                    != canonical::hash(&rebuilt).map_err(internal)?
                {
                    let mut error = projection_discovery_error(
                        "live value-flow Thread IR did not rebuild canonically",
                        DeclarationProviderStage::ThreadBuild,
                        BTreeMap::new(),
                        BTreeMap::from([
                            (
                                "thread".into(),
                                canonical::hash(&thread).unwrap_or_default(),
                            ),
                            (
                                "rebuilt".into(),
                                canonical::hash(&rebuilt).unwrap_or_default(),
                            ),
                        ]),
                        BTreeMap::new(),
                    );
                    error.code = ErrorCode::StaleRequiresReslice;
                    return Err(error);
                }
                verify_live_sources(repo, &thread).map_err(|error| {
                    projection_discovery_error(
                        "value-flow Thread ReadSet is not live",
                        DeclarationProviderStage::ReadsetLive,
                        BTreeMap::from([("readFactCount".into(), thread.read_set.len())]),
                        BTreeMap::from([
                            (
                                "readSet".into(),
                                canonical::hash(&thread.read_set).unwrap_or_default(),
                            ),
                            (
                                "upstream".into(),
                                canonical::hash(&error.message).unwrap_or_default(),
                            ),
                        ]),
                        BTreeMap::new(),
                    )
                })?;
                if !thread_node_dominates(&thread, &source_occurrence.id, &destination_node.id) {
                    return Err(invalid_source(
                        "source occurrence does not dominate destination occurrence",
                    ));
                }
                let occurrence_count = owner_relations
                    .iter()
                    .filter(|row| {
                        row.get("target").and_then(Value::as_str) == Some(source_callable)
                            && row.get("start").and_then(Value::as_u64) == Some(source_start)
                    })
                    .count();
                if occurrence_count != 1 {
                    return Err(projection_discovery_error(
                        "source occurrence has non-unique evaluation provenance",
                        DeclarationProviderStage::OccurrenceCardinality,
                        BTreeMap::from([("occurrenceCount".into(), occurrence_count)]),
                        BTreeMap::from([(
                            "sourceRelation".into(),
                            canonical::hash(source_relation).unwrap_or_default(),
                        )]),
                        BTreeMap::from([("slotIndex".into(), slot_index.to_string())]),
                    ));
                }
                let source_symbol = required_str(source_descriptor, "symbolIdentity")?.to_owned();
                let destination_symbol =
                    required_str(destination_descriptor, "symbolIdentity")?.to_owned();
                candidates.push(DeclarationValueFlowCandidate {
                    source_symbol,
                    destination_symbol,
                    owner_callable: owner.into(),
                    source_call_node_id: source_node.id.clone(),
                    source_node_id: source_occurrence.id.clone(),
                    destination_node_id: destination_node.id.clone(),
                    slot_kind: "VALUE_PARAMETER",
                    slot_index,
                    source_type,
                    source_nullable: source_descriptor
                        .get("returnNullable")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| invalid_source("source nullability is unknown"))?,
                    destination_type,
                    destination_nullable: slot
                        .get("nullable")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| invalid_source("destination nullability is unknown"))?,
                    order: "SOURCE_BEFORE_DESTINATION",
                    dominance: "SOURCE_DOMINATES_DESTINATION",
                    evaluation_count: occurrence_count,
                    module: project
                        .get("module")
                        .and_then(Value::as_str)
                        .unwrap_or(":")
                        .into(),
                    source_set: project
                        .get("sourceSet")
                        .and_then(Value::as_str)
                        .unwrap_or("main")
                        .into(),
                    relation_fingerprint: canonical::hash(&(
                        &relations.hash,
                        source_relation,
                        destination_relation,
                        mapping,
                    ))
                    .map_err(internal)?,
                    descriptor_fingerprint: canonical::hash(&(
                        &descriptors.hash,
                        source_descriptor,
                        destination_descriptor,
                    ))
                    .map_err(internal)?,
                    thread_fingerprint: canonical::hash(&thread).map_err(internal)?,
                    read_set_fingerprint: canonical::hash(&thread.read_set).map_err(internal)?,
                    provenance_fingerprint: canonical::hash(&(
                        &relations.provenance,
                        &descriptors.provenance,
                        &thread.snapshot,
                    ))
                    .map_err(internal)?,
                    boundary_closure_fingerprint: boundary_evaluation.fingerprint.clone(),
                });
            }
        }
    }
    candidates.sort_by(|left, right| {
        (
            &left.source_symbol,
            &left.destination_symbol,
            left.slot_index,
            &left.owner_callable,
        )
            .cmp(&(
                &right.source_symbol,
                &right.destination_symbol,
                right.slot_index,
                &right.owner_callable,
            ))
    });
    if candidates.windows(2).any(|pair| {
        pair[0].source_symbol == pair[1].source_symbol
            && pair[0].destination_symbol == pair[1].destination_symbol
            && pair[0].owner_callable == pair[1].owner_callable
    }) {
        return Err(invalid_source(
            "value-flow declarations have multiple occurrence paths or evaluations",
        ));
    }
    candidates.dedup_by(|left, right| {
        left.source_symbol == right.source_symbol
            && left.destination_symbol == right.destination_symbol
            && left.slot_index == right.slot_index
            && left.owner_callable == right.owner_callable
    });
    Ok(candidates)
}

fn discover_projection_consumer_candidates(
    repo: &Path,
    requested_compilation: Option<&str>,
    worker: &mut WorkerClient,
) -> Result<Vec<ProjectionConsumerCandidate>, ClewError> {
    let flows = discover_declaration_value_flow_candidates(repo, requested_compilation, worker)
        .map_err(|error| {
            if let Some(diagnostic) = error
                .evidence
                .iter()
                .find_map(|value| serde_json::from_str::<ProjectionDiscoveryDiagnostic>(value).ok())
            {
                return projection_discovery_error(
                    "projection live value-flow discovery refused",
                    diagnostic.stage,
                    diagnostic.counts,
                    diagnostic.hashes,
                    diagnostic.shapes,
                );
            }
            projection_discovery_error(
                "projection live value-flow discovery refused",
                DeclarationProviderStage::ValueFlowThread,
                BTreeMap::from([("candidateCount".into(), 0)]),
                BTreeMap::from([
                    (
                        "upstreamError".into(),
                        canonical::hash(&error.message).unwrap_or_default(),
                    ),
                    (
                        "upstreamEvidence".into(),
                        canonical::hash(&error.evidence).unwrap_or_default(),
                    ),
                ]),
                BTreeMap::from([("errorCode".into(), format!("{:?}", error.code))]),
            )
        })?;
    let compilation = select_production_compilation(repo, requested_compilation)?;
    let index = RepositoryIndex::open_compilation(repo, Some(&compilation))?;
    index.require_fresh(REPOSITORY_INDEX_FACT)?;
    let relations = index
        .declaration_relations()?
        .ok_or_else(|| invalid_source("projection has no verified relation graph"))?;
    let descriptors = index
        .declaration_descriptors()?
        .ok_or_else(|| invalid_source("projection has no verified descriptor graph"))?;
    for graph in [&relations.graph, &descriptors.graph] {
        if graph.get("coverage").and_then(Value::as_str) != Some("COMPLETE_SUPPORTED_SUBSET")
            || graph
                .get("boundaries")
                .and_then(Value::as_array)
                .is_none_or(|boundaries| !boundaries.is_empty())
        {
            return Err(invalid_source(
                "projection intersects a partial or Unknown compiler boundary",
            ));
        }
    }
    let relation_rows = relations
        .graph
        .get("relations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("projection relation graph has no rows"))?;
    let descriptor_rows = descriptors
        .graph
        .get("descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("projection descriptor graph has no rows"))?;
    let exact_callable = |callable: &str| -> Result<&Value, ClewError> {
        let matches = descriptor_rows
            .iter()
            .filter(|descriptor| {
                descriptor.get("declarationKind").and_then(Value::as_str) == Some("FUNCTION")
                    && descriptor.get("compilerCallableId").and_then(Value::as_str)
                        == Some(callable)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [only] => Ok(*only),
            _ => Err(invalid_source(
                "projection callable descriptor is missing or ambiguous",
            )),
        }
    };
    let descriptor_by_symbol = |symbol: &str| -> Result<&Value, ClewError> {
        let matches = descriptor_rows
            .iter()
            .filter(|descriptor| {
                descriptor.get("symbolIdentity").and_then(Value::as_str) == Some(symbol)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [only] => Ok(*only),
            _ => Err(invalid_source(
                "projection bound descriptor is missing or ambiguous",
            )),
        }
    };

    let mut grouped = BTreeMap::<(String, String, String), ProjectionConsumerCandidate>::new();
    let mut expected_occurrences = 0usize;
    for returned in relation_rows
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("RETURNS_VALUE_FROM"))
    {
        let projection_callable = required_str(returned, "owner")?;
        let source_callable = required_str(returned, "target")?;
        let projection_descriptor = exact_callable(projection_callable)?;
        let source_descriptor = exact_callable(source_callable).or_else(|_| {
            let matches = descriptor_rows
                .iter()
                .filter(|descriptor| {
                    matches!(
                        descriptor.get("declarationKind").and_then(Value::as_str),
                        Some("PROPERTY" | "MUTABLE_PROPERTY")
                    ) && descriptor.get("compilerCallableId").and_then(Value::as_str)
                        == Some(source_callable)
                })
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [only] => Ok(*only),
                _ => Err(invalid_source(
                    "projection source descriptor is missing or ambiguous",
                )),
            }
        })?;
        if !declaration_is_internal(source_descriptor)
            || !declaration_is_internal(projection_descriptor)
        {
            return Err(invalid_source(
                "projection crosses an exported ABI boundary",
            ));
        }
        let source_type =
            if returned.get("sourceKind").and_then(Value::as_str) == Some("PROPERTY_READ") {
                required_str(source_descriptor, "declaredType")?
            } else {
                required_str(source_descriptor, "returnType")?
            };
        let result_type = required_str(returned, "resultType")?;
        if source_type != result_type
            || required_str(projection_descriptor, "returnType")? != result_type
            || projection_descriptor.get("returnNullable") != returned.get("resultNullable")
        {
            return Err(invalid_source(
                "projection declared type or nullability is inconsistent",
            ));
        }
        let source_symbol = required_str(source_descriptor, "symbolIdentity")?.to_owned();
        let projection_symbol = required_str(projection_descriptor, "symbolIdentity")?.to_owned();
        for flow in flows
            .iter()
            .filter(|flow| flow.source_symbol == projection_symbol)
        {
            let consumer_descriptor = descriptor_by_symbol(&flow.destination_symbol)?;
            if !declaration_is_internal(consumer_descriptor)
                || flow.source_type != result_type
                || flow.source_nullable
                    != returned
                        .get("resultNullable")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| invalid_source("projection nullability is Unknown"))?
                || flow.destination_type != flow.source_type
                || flow.destination_nullable != flow.source_nullable
                || flow.evaluation_count != 1
            {
                return Err(invalid_source(
                    "projection-to-consumer occurrence is not type/flow complete",
                ));
            }
            let consumer_callable = required_str(consumer_descriptor, "compilerCallableId")?;
            let affected =
                BTreeSet::from([source_callable, projection_callable, consumer_callable]);
            let mut use_rows = relation_rows
                .iter()
                .filter(|row| {
                    matches!(
                        row.get("kind").and_then(Value::as_str),
                        Some("CALLS" | "REFERENCES" | "READS" | "RETURNS_VALUE_FROM")
                    ) && (row
                        .get("target")
                        .and_then(Value::as_str)
                        .is_some_and(|target| affected.contains(target))
                        || row
                            .get("owner")
                            .and_then(Value::as_str)
                            .is_some_and(|owner| affected.contains(owner)))
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut use_descriptors = Vec::new();
            for row in &use_rows {
                if let Some(owner) = row.get("owner").and_then(Value::as_str) {
                    let descriptor = exact_callable(owner)?;
                    if !declaration_is_internal(descriptor) {
                        return Err(invalid_source(
                            "projection use closure crosses an exported ABI boundary",
                        ));
                    }
                    use_descriptors.push(descriptor.clone());
                }
            }
            use_rows.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
            use_descriptors.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
            let occurrence = ProjectionConsumerOccurrence {
                return_relation_fingerprint: canonical::hash(&(&relations.hash, returned))
                    .map_err(internal)?,
                value_flow: flow.clone(),
            };
            let key = (
                source_symbol.clone(),
                projection_symbol.clone(),
                flow.destination_symbol.clone(),
            );
            let entry = grouped
                .entry(key)
                .or_insert_with(|| ProjectionConsumerCandidate {
                    source_symbol: source_symbol.clone(),
                    projection_symbol: projection_symbol.clone(),
                    consumer_symbol: flow.destination_symbol.clone(),
                    source_callable: source_callable.into(),
                    projection_callable: projection_callable.into(),
                    consumer_callable: consumer_callable.into(),
                    occurrences: vec![],
                    occurrence_fingerprints: vec![],
                    occurrence_set_fingerprint: String::new(),
                    declared_type_fingerprint: canonical::hash(&(
                        source_descriptor,
                        projection_descriptor,
                        returned,
                    ))
                    .unwrap_or_default(),
                    use_closure_fingerprint: canonical::hash(&(
                        &relations.hash,
                        &use_rows,
                        &use_descriptors,
                    ))
                    .unwrap_or_default(),
                    contract_fingerprint: canonical::hash(&(consumer_descriptor, "MODULE_API"))
                        .unwrap_or_default(),
                    provenance_fingerprint: canonical::hash(&(
                        &relations.provenance,
                        &descriptors.provenance,
                        &flow.provenance_fingerprint,
                    ))
                    .unwrap_or_default(),
                });
            entry.occurrences.push(occurrence);
            expected_occurrences += 1;
        }
    }
    let mut candidates = grouped.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        (
            &left.source_symbol,
            &left.projection_symbol,
            &left.consumer_symbol,
        )
            .cmp(&(
                &right.source_symbol,
                &right.projection_symbol,
                &right.consumer_symbol,
            ))
    });
    for candidate in &mut candidates {
        let mut keyed = candidate
            .occurrences
            .drain(..)
            .map(|occurrence| {
                let bytes = canonical::bytes(&occurrence).map_err(internal)?;
                let fingerprint = canonical::hash_bytes(&bytes);
                Ok((fingerprint, bytes, occurrence))
            })
            .collect::<Result<Vec<_>, ClewError>>()?;
        keyed.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
        for pair in keyed.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(invalid_source(if pair[0].1 == pair[1].1 {
                    "projection occurrence set contains a duplicate"
                } else {
                    "projection occurrence fingerprint collision"
                }));
            }
        }
        candidate.occurrence_fingerprints = keyed
            .iter()
            .map(|(fingerprint, _, _)| fingerprint.clone())
            .collect();
        candidate.occurrences = keyed
            .into_iter()
            .map(|(_, _, occurrence)| occurrence)
            .collect();
        candidate.occurrence_set_fingerprint =
            canonical::hash(&candidate.occurrence_fingerprints).map_err(internal)?;
    }
    if candidates
        .iter()
        .map(|candidate| candidate.occurrences.len())
        .sum::<usize>()
        != expected_occurrences
    {
        return Err(invalid_source(
            "projection occurrence aggregation is incomplete",
        ));
    }
    Ok(candidates)
}

fn exact_relation_call_nodes<'a>(
    graph: &'a LocalGraph,
    relation: &Value,
) -> Result<Vec<&'a crate::model::GraphNode>, ClewError> {
    let target = required_str(relation, "target")?;
    let start = relation
        .get("start")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_source("relation occurrence has no start"))?;
    let cfg_ids = relation
        .get("cfgNodeIds")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("relation occurrence has no CFG provenance"))?
        .iter()
        .filter_map(Value::as_u64)
        .map(|id| format!("fir:{id}"))
        .collect::<BTreeSet<_>>();
    Ok(graph
        .nodes
        .iter()
        .filter(|node| node.kind == "CALL")
        .filter(|node| node.attributes.get("symbol").and_then(Value::as_str) == Some(target))
        .filter(|node| cfg_ids.contains(&node.id))
        .filter(|node| {
            node.origin
                .as_ref()
                .and_then(|origin| origin.pointer("/rangeHint/0"))
                .and_then(Value::as_u64)
                == Some(start)
        })
        .collect())
}

fn occurrence_slice_policy() -> SlicePolicy {
    let mut policy = SlicePolicy::default();
    policy.include_edges.extend(
        [
            "CFG_NORMAL",
            "CFG_TRUE",
            "CFG_FALSE",
            "CFG_BACK",
            "CFG_EXCEPTION",
        ]
        .map(str::to_owned),
    );
    policy.include_edges.sort();
    policy.include_edges.dedup();
    policy
}

fn value_flow_snapshot(
    repo: &Path,
    compilation: &str,
    project: &Value,
    index_snapshot: &str,
    worker: &WorkerClient,
) -> Result<Snapshot, ClewError> {
    Ok(Snapshot {
        base_revision: git_head(repo)?,
        project_model_hash: required_str(project, "projectModelHash")?.into(),
        compiler_version: worker.capabilities.compiler_version.clone(),
        build_system: match project.get("buildSystem").and_then(Value::as_str) {
            Some("MAVEN") => BuildSystem::Maven,
            _ => BuildSystem::Gradle,
        },
        build_launcher: project
            .get("buildLauncher")
            .and_then(Value::as_str)
            .unwrap_or("./gradlew")
            .into(),
        index_snapshot: index_snapshot.into(),
        compilation: compilation.into(),
        compile_task: project
            .get("compileTask")
            .and_then(Value::as_str)
            .unwrap_or(":compileKotlin")
            .into(),
        test_tasks: project
            .get("testTasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    })
}

fn verify_occurrence_thread_closure(
    thread: &ThreadIr,
    authorized_call_nodes: &[&str],
) -> Result<(), ClewError> {
    if matches!(
        thread.completeness.status,
        CompletenessStatus::PartialBudget
            | CompletenessStatus::PartialUnsupportedFeature
            | CompletenessStatus::PartialDynamicDispatch
            | CompletenessStatus::Failed
    ) {
        return Err(invalid_source("value-flow Thread IR is incomplete"));
    }
    let authorized = authorized_call_nodes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if authorized.len() != authorized_call_nodes.len() || authorized.is_empty() {
        return Err(invalid_source(
            "value-flow call boundary authorization is duplicate or empty",
        ));
    }
    let authorized_nodes = authorized
        .iter()
        .map(|node_id| {
            thread
                .nodes
                .iter()
                .find(|node| node.id == *node_id && node.kind == "CALL")
                .ok_or_else(|| invalid_source("authorized boundary is not an exact CALL node"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (index, node) in authorized_nodes.iter().enumerate() {
        let origin = node
            .origin
            .as_ref()
            .ok_or_else(|| invalid_source("authorized CALL has no compiler origin"))?;
        if node
            .attributes
            .get("symbol")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || node
                .attributes
                .get("calleeSummaryHash")
                .and_then(Value::as_str)
                .is_none_or(|hash| hash.is_empty() || hash == "sha256:unknown")
            || authorized_nodes[..index]
                .iter()
                .any(|other| other.origin.as_ref() == Some(origin))
        {
            return Err(invalid_source(
                "authorized CALL origin or compiler identity is ambiguous",
            ));
        }
    }
    let class_for = |node_id: &str| -> Result<usize, ClewError> {
        let node = thread
            .nodes
            .iter()
            .find(|node| node.id == node_id && node.kind == "CALL")
            .ok_or_else(|| invalid_source("boundary phase is not a CALL node"))?;
        let matches = authorized_nodes
            .iter()
            .enumerate()
            .filter(|(_, authorized)| {
                node.origin == authorized.origin
                    && node.attributes.get("symbol") == authorized.attributes.get("symbol")
                    && node.attributes.get("calleeSummaryHash")
                        == authorized.attributes.get("calleeSummaryHash")
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [only] => Ok(*only),
            _ => Err(invalid_source(
                "CALL boundary is outside or ambiguous across exact origin classes",
            )),
        }
    };
    let boundary_nodes = thread
        .completeness
        .boundaries
        .iter()
        .map(|boundary| {
            if boundary.get("kind").and_then(Value::as_str) != Some("EXTERNAL_CALL") {
                return Err(invalid_source(
                    "value-flow Thread IR has an unsupported boundary",
                ));
            }
            required_str(boundary, "nodeId")
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let summary_nodes = thread
        .external_summaries
        .iter()
        .map(|summary| required_str(summary, "nodeId"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if boundary_nodes != summary_nodes || boundary_nodes.is_empty() {
        return Err(invalid_source(
            "value-flow Thread IR boundary and summary sets differ",
        ));
    }
    let boundary_classes = boundary_nodes
        .iter()
        .map(|node| class_for(node))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if boundary_classes != (0..authorized_nodes.len()).collect::<BTreeSet<_>>() {
        return Err(invalid_source(
            "value-flow Thread IR does not close every authorized CALL class",
        ));
    }
    for summary in &thread.external_summaries {
        let node_id = required_str(summary, "nodeId")?;
        let node = thread
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| invalid_source("external summary has no CALL node"))?;
        if summary.get("symbol") != node.attributes.get("symbol")
            || summary.get("calleeSummaryHash") != node.attributes.get("calleeSummaryHash")
        {
            return Err(invalid_source(
                "external summary differs from exact compiler CALL identity",
            ));
        }
    }
    for boundary in &thread.completeness.boundaries {
        let node_id = required_str(boundary, "nodeId")?;
        let node = thread
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| invalid_source("call boundary has no graph node"))?;
        if boundary.get("symbol") != node.attributes.get("symbol") {
            return Err(invalid_source(
                "call boundary differs from exact compiler CALL identity",
            ));
        }
    }
    Ok(())
}

fn thread_node_dominates(thread: &ThreadIr, source: &str, destination: &str) -> bool {
    let cfg_kinds = |kind: &str| kind.starts_with("CFG_");
    let entry = thread.nodes.iter().find(|node| node.kind == "ENTRY");
    let Some(entry) = entry else { return false };
    let reachable = |removed: Option<&str>| {
        let mut seen = BTreeSet::from([entry.id.as_str()]);
        let mut queue = std::collections::VecDeque::from([entry.id.as_str()]);
        while let Some(current) = queue.pop_front() {
            for edge in thread
                .edges
                .iter()
                .filter(|edge| cfg_kinds(&edge.kind) && edge.from == current)
            {
                if removed == Some(edge.to.as_str()) {
                    continue;
                }
                if seen.insert(edge.to.as_str()) {
                    queue.push_back(edge.to.as_str());
                }
            }
        }
        seen
    };
    reachable(None).contains(destination) && !reachable(Some(source)).contains(destination)
}

fn declaration_required_str<'a>(
    value: &'a Value,
    field: &str,
) -> Result<&'a str, TypedGoalRefusalReason> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(TypedGoalRefusalReason::InsufficientEvidence)
}

fn declaration_is_internal(descriptor: &Value) -> bool {
    descriptor
        .get("effectiveVisibility")
        .and_then(Value::as_str)
        == Some("internal")
        && descriptor.get("exportBoundary").and_then(Value::as_str) == Some("MODULE_API")
}

fn declaration_parameter_types(descriptor: &Value) -> Option<Vec<&str>> {
    descriptor
        .get("parameterTypes")?
        .as_array()?
        .iter()
        .map(|parameter| parameter.get("type")?.as_str())
        .collect()
}

fn declaration_string_array<'a>(value: &'a Value, field: &str) -> Option<Vec<&'a str>> {
    value
        .get(field)?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect()
}

fn declaration_override_types_match(
    relation: &Value,
    source: &Value,
    target: &Value,
) -> Result<bool, TypedGoalRefusalReason> {
    Ok(declaration_required_str(relation, "sourceReturnType")?
        == declaration_required_str(source, "returnType")?
        && declaration_required_str(relation, "baseReturnType")?
            == declaration_required_str(target, "returnType")?
        && declaration_string_array(relation, "sourceParameterTypes")
            == declaration_parameter_types(source)
        && declaration_string_array(relation, "baseParameterTypes")
            == declaration_parameter_types(target))
}

// Keep the structured rejection inline so callers can enrich it before it is
// serialized into the typed refusal receipt.
#[allow(clippy::result_large_err)]
fn declaration_type_comparison_diagnostic(
    relation: &Value,
    source: &Value,
    target: &Value,
) -> Result<
    DeclarationTypeComparisonDiagnostic,
    (TypedGoalRefusalReason, DeclarationProviderRejection),
> {
    let values = [
        relation.get("sourceReturnType").and_then(Value::as_str),
        relation.get("baseReturnType").and_then(Value::as_str),
        source.get("returnType").and_then(Value::as_str),
        target.get("returnType").and_then(Value::as_str),
    ];
    let typed: Option<[String; 4]> = values
        .map(|value| value.map(str::to_owned))
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .and_then(|items| items.try_into().ok());
    let Some([relation_source, relation_base, source_return, base_return]) = typed else {
        let rejection = declaration_provider_rejection(
            DeclarationProviderStage::TypeCompatibility,
            ErrorCode::IncompleteSemanticAnalysis,
            &json!({"missingTypedField":true}),
        );
        return Err((TypedGoalRefusalReason::InsufficientEvidence, rejection));
    };
    let rendering_class = |rendered: &str| {
        if rendered.contains("<ERROR") {
            "ERROR"
        } else if rendered.contains("..") || rendered.contains('!') {
            "FLEXIBLE_OR_PLATFORM"
        } else if rendered.ends_with('?') {
            "NULLABLE"
        } else {
            "NON_NULL"
        }
        .to_owned()
    };
    let types = [
        relation_source.clone(),
        relation_base.clone(),
        source_return.clone(),
        base_return.clone(),
    ];
    Ok(DeclarationTypeComparisonDiagnostic {
        relation_source_return_type: relation_source,
        relation_base_return_type: relation_base,
        source_descriptor_return_type: source_return,
        source_descriptor_nullable: source
            .get("returnNullable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        base_descriptor_return_type: base_return,
        base_descriptor_nullable: target
            .get("returnNullable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        rendering_classes: types.iter().map(|value| rendering_class(value)).collect(),
        canonical_type_hashes: types
            .iter()
            .map(|value| canonical::hash(value).unwrap_or_else(|_| "unavailable".into()))
            .collect(),
    })
}

fn declaration_type_shape_diagnostic(
    relation: &Value,
    source: &Value,
    target: &Value,
) -> BTreeMap<String, DeclarationFieldShape> {
    fn shape(value: Option<&Value>) -> DeclarationFieldShape {
        let json_type = match value {
            None => "MISSING",
            Some(Value::Null) => "NULL",
            Some(Value::Bool(_)) => "BOOLEAN",
            Some(Value::Number(_)) => "NUMBER",
            Some(Value::String(_)) => "STRING",
            Some(Value::Array(_)) => "ARRAY",
            Some(Value::Object(_)) => "OBJECT",
        };
        DeclarationFieldShape {
            present: value.is_some(),
            json_type: json_type.into(),
            array_length: value.and_then(Value::as_array).map(Vec::len),
            value_hash: canonical::hash(value.unwrap_or(&Value::Null))
                .unwrap_or_else(|_| "unavailable".into()),
        }
    }
    BTreeMap::from([
        (
            "relation.sourceReturnType".into(),
            shape(relation.get("sourceReturnType")),
        ),
        (
            "relation.baseReturnType".into(),
            shape(relation.get("baseReturnType")),
        ),
        (
            "relation.sourceParameterTypes".into(),
            shape(relation.get("sourceParameterTypes")),
        ),
        (
            "relation.baseParameterTypes".into(),
            shape(relation.get("baseParameterTypes")),
        ),
        ("source.returnType".into(), shape(source.get("returnType"))),
        (
            "source.returnNullable".into(),
            shape(source.get("returnNullable")),
        ),
        (
            "source.parameterTypes".into(),
            shape(source.get("parameterTypes")),
        ),
        ("base.returnType".into(), shape(target.get("returnType"))),
        (
            "base.returnNullable".into(),
            shape(target.get("returnNullable")),
        ),
        (
            "base.parameterTypes".into(),
            shape(target.get("parameterTypes")),
        ),
    ])
}

fn declaration_value_shape(value: Option<&Value>) -> DeclarationFieldShape {
    let json_type = match value {
        None => "MISSING",
        Some(Value::Null) => "NULL",
        Some(Value::Bool(_)) => "BOOLEAN",
        Some(Value::Number(_)) => "NUMBER",
        Some(Value::String(_)) => "STRING",
        Some(Value::Array(_)) => "ARRAY",
        Some(Value::Object(_)) => "OBJECT",
    };
    DeclarationFieldShape {
        present: value.is_some(),
        json_type: json_type.into(),
        array_length: value.and_then(Value::as_array).map(Vec::len),
        value_hash: canonical::hash(value.unwrap_or(&Value::Null))
            .unwrap_or_else(|_| "unavailable".into()),
    }
}

fn nullable_discovery_error(message: &str, diagnostic: &NullableDiscoveryDiagnostic) -> ClewError {
    let mut error = invalid_source(message);
    if let Ok(encoded) = serde_json::to_string(diagnostic) {
        error.evidence.push(encoded);
    }
    error
}

fn projection_discovery_error(
    message: &str,
    stage: DeclarationProviderStage,
    counts: BTreeMap<String, usize>,
    hashes: BTreeMap<String, String>,
    shapes: BTreeMap<String, String>,
) -> ClewError {
    let mut error = invalid_source(message);
    let diagnostic = ProjectionDiscoveryDiagnostic {
        schema: "projection-discovery-diagnostic/0.1".into(),
        stage,
        counts,
        hashes,
        shapes,
    };
    if let Ok(encoded) = serde_json::to_string(&diagnostic) {
        error.evidence.push(encoded);
    }
    error
}

fn nullable_discovery_attach(
    mut error: ClewError,
    diagnostic: &NullableDiscoveryDiagnostic,
) -> ClewError {
    if let Ok(encoded) = serde_json::to_string(diagnostic) {
        error.evidence.push(encoded);
    }
    error
}

fn nullable_early_diagnostic(
    stage: DeclarationProviderStage,
    requested_compilation: Option<&str>,
    project: Option<&Value>,
    inspected: Option<&Value>,
    authority: Option<&Value>,
    receipt_issued: bool,
    receipt_recognized: bool,
) -> NullableDiscoveryDiagnostic {
    let requested_compilation = requested_compilation.map(|value| Value::String(value.into()));
    let diagnostic_codes = Value::Array(
        inspected
            .and_then(|value| value.get("diagnostics"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|diagnostic| diagnostic.get("code").cloned())
            .collect(),
    );
    let project_compilation = project.and_then(|value| value.get("compilation"));
    let inspected_compilation = inspected.and_then(|value| value.get("compilation"));
    let project_model = project.and_then(|value| value.get("projectModelHash"));
    let inspected_model = inspected.and_then(|value| value.get("projectModelHash"));
    let project_snapshot = project.and_then(|value| {
        value
            .get("sourceSnapshotHash")
            .or_else(|| value.get("sourceSnapshot"))
    });
    let inspected_snapshot = inspected.and_then(|value| {
        value
            .get("sourceSnapshotHash")
            .or_else(|| value.get("sourceSnapshot"))
    });
    let equality = |left: Option<&Value>, right: Option<&Value>| match (left, right) {
        (Some(left), Some(right)) if left == right => "EQUAL",
        (Some(_), Some(_)) => "DIFFERENT",
        _ => "UNKNOWN",
    };
    NullableDiscoveryDiagnostic {
        schema: "nullable-discovery-diagnostic/0.1".into(),
        stage,
        shapes: BTreeMap::from([
            (
                "requested.compilation".into(),
                declaration_value_shape(requested_compilation.as_ref()),
            ),
            (
                "project.compilation".into(),
                declaration_value_shape(project_compilation),
            ),
            (
                "index.compilation".into(),
                declaration_value_shape(inspected_compilation),
            ),
            (
                "project.projectModelHash".into(),
                declaration_value_shape(project_model),
            ),
            (
                "index.projectModelHash".into(),
                declaration_value_shape(inspected_model),
            ),
            (
                "project.sourceSnapshot".into(),
                declaration_value_shape(project_snapshot),
            ),
            (
                "index.sourceSnapshot".into(),
                declaration_value_shape(inspected_snapshot),
            ),
            (
                "authority.distribution".into(),
                declaration_value_shape(
                    authority.and_then(|value| value.get("distributionFingerprintHash")),
                ),
            ),
            (
                "authority.input".into(),
                declaration_value_shape(
                    authority.and_then(|value| value.get("buildInputDigestHash")),
                ),
            ),
            (
                "authority.plugin".into(),
                declaration_value_shape(
                    authority.and_then(|value| value.get("pluginFingerprintHash")),
                ),
            ),
            (
                "authority.tree".into(),
                declaration_value_shape(
                    authority.and_then(|value| value.get("distributionTreeHash")),
                ),
            ),
            (
                "authority.session".into(),
                declaration_value_shape(
                    authority.and_then(|value| value.get("authoritySessionHash")),
                ),
            ),
            (
                "authority.currentSession".into(),
                declaration_value_shape(
                    authority.and_then(|value| value.get("currentSessionHash")),
                ),
            ),
            (
                "index.k2Validated".into(),
                declaration_value_shape(inspected.and_then(|value| value.get("k2Validated"))),
            ),
            (
                "index.diagnosticCodes".into(),
                declaration_value_shape(Some(&diagnostic_codes)),
            ),
        ]),
        cardinalities: BTreeMap::from([(
            "diagnostics".into(),
            inspected
                .and_then(|value| value.get("diagnostics"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        )]),
        range_relations: BTreeMap::new(),
        fact_relations: BTreeMap::from([
            (
                "requestedToProjectCompilation".into(),
                equality(requested_compilation.as_ref(), project_compilation).into(),
            ),
            (
                "projectToIndexCompilation".into(),
                equality(project_compilation, inspected_compilation).into(),
            ),
            (
                "projectToIndexModel".into(),
                equality(project_model, inspected_model).into(),
            ),
            (
                "projectToIndexSourceSnapshot".into(),
                equality(project_snapshot, inspected_snapshot).into(),
            ),
            (
                "receiptIssued".into(),
                if receipt_issued { "TRUE" } else { "FALSE" }.into(),
            ),
            (
                "receiptRecognized".into(),
                if receipt_recognized { "TRUE" } else { "FALSE" }.into(),
            ),
            (
                "authoritySessionMatches".into(),
                authority
                    .and_then(|value| value.get("sessionMatches"))
                    .and_then(Value::as_bool)
                    .map_or("UNKNOWN", |value| if value { "TRUE" } else { "FALSE" })
                    .into(),
            ),
        ]),
    }
}

fn nullable_verified_index_failure_diagnostic(
    error: &ClewError,
    mut diagnostic: NullableDiscoveryDiagnostic,
) -> NullableDiscoveryDiagnostic {
    let Some(worker) = error.evidence.iter().find_map(|encoded| {
        serde_json::from_str::<Value>(encoded).ok().filter(|value| {
            value.get("schema").and_then(Value::as_str)
                == Some("verified-index-failure-diagnostic/0.1")
        })
    }) else {
        return diagnostic;
    };
    diagnostic.stage = match worker.get("stage").and_then(Value::as_str) {
        Some("RAW_SCHEMA_HASH") => DeclarationProviderStage::RawSchemaHash,
        Some("DESCRIPTOR_GRAPH") => DeclarationProviderStage::DescriptorGraph,
        Some("RELATION_GRAPH") => DeclarationProviderStage::RelationGraph,
        Some("CROSS_GRAPH_CONSISTENCY") => DeclarationProviderStage::CrossGraphConsistency,
        Some("SOURCE_BINDING") => DeclarationProviderStage::SourceBinding,
        Some("DISTRIBUTION_PROVENANCE") => DeclarationProviderStage::DistributionProvenance,
        _ => DeclarationProviderStage::VerifiedIndexReceipt,
    };
    if let Some(descriptor_failure) = worker
        .get("descriptorFailure")
        .filter(|value| value.is_object())
    {
        diagnostic.stage = match descriptor_failure.get("stage").and_then(Value::as_str) {
            Some("DESCRIPTOR_KIND_IDENTITY") => DeclarationProviderStage::DescriptorKindIdentity,
            Some("OWNER_CONTAINMENT") => DeclarationProviderStage::OwnerContainment,
            Some("VISIBILITY_MODALITY") => DeclarationProviderStage::VisibilityModality,
            Some("JVM_SIGNATURE") => DeclarationProviderStage::JvmSignature,
            Some("PARAMETER_SLOTS") => DeclarationProviderStage::ParameterSlots,
            Some("TYPE_NULLABILITY") => DeclarationProviderStage::TypeNullability,
            Some("UNKNOWN_BOUNDARY") => DeclarationProviderStage::UnknownDescriptorBoundary,
            _ => diagnostic.stage,
        };
        if let Some(ordinal) = descriptor_failure
            .get("ordinal")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        {
            diagnostic
                .cardinalities
                .insert("descriptorFailure.ordinal".into(), ordinal);
        }
        diagnostic.shapes.insert(
            "descriptorFailure.rowHash".into(),
            declaration_value_shape(descriptor_failure.get("rowHash")),
        );
        diagnostic.shapes.insert(
            "descriptorFailure.kind".into(),
            declaration_value_shape(descriptor_failure.get("kind")),
        );
        if let Some(shapes) = descriptor_failure.get("shapes").and_then(Value::as_object) {
            for (field, shape) in shapes {
                if let Ok(shape) = serde_json::from_value::<DeclarationFieldShape>(shape.clone()) {
                    diagnostic
                        .shapes
                        .insert(format!("descriptorFailure.{field}"), shape);
                }
            }
        }
    }
    for field in [
        "rawSchemaHash",
        "payloadHash",
        "relationGraphHash",
        "descriptorGraphHash",
        "relationProvenanceHash",
        "descriptorProvenanceHash",
    ] {
        diagnostic.shapes.insert(
            format!("verifiedIndex.{field}"),
            declaration_value_shape(worker.get(field)),
        );
    }
    for field in [
        "relationCount",
        "relationBoundaryCount",
        "descriptorCount",
        "descriptorBoundaryCount",
    ] {
        if let Some(count) = worker
            .get(field)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        {
            diagnostic
                .cardinalities
                .insert(format!("verifiedIndex.{field}"), count);
        }
    }
    diagnostic
}

fn nullable_constructs_slot_error(
    message: &str,
    diagnostic: &NullableDiscoveryDiagnostic,
) -> ClewError {
    nullable_discovery_error(message, diagnostic)
}

fn nullable_graph_diagnostic(
    stage: DeclarationProviderStage,
    relation_graph: &Value,
    descriptor_graph: &Value,
) -> NullableDiscoveryDiagnostic {
    let expected_schema = Value::String("fir-facts-extractor/0.6".into());
    let relation_boundaries = relation_graph.get("boundaries").and_then(Value::as_array);
    let descriptor_boundaries = descriptor_graph.get("boundaries").and_then(Value::as_array);
    let mut unknown_codes = relation_boundaries
        .into_iter()
        .flatten()
        .chain(descriptor_boundaries.into_iter().flatten())
        .filter_map(|boundary| {
            boundary
                .get("code")
                .or_else(|| boundary.get("reason"))
                .cloned()
        })
        .collect::<Vec<_>>();
    unknown_codes.sort_by_key(Value::to_string);
    let unknown_codes = Value::Array(unknown_codes);
    let relation_graph_hash =
        Value::String(canonical::hash(relation_graph).unwrap_or_else(|_| "unavailable".into()));
    let descriptor_graph_hash =
        Value::String(canonical::hash(descriptor_graph).unwrap_or_else(|_| "unavailable".into()));
    let relation_provenance_hash = Value::String(
        canonical::hash(relation_graph.get("provenance").unwrap_or(&Value::Null))
            .unwrap_or_else(|_| "unavailable".into()),
    );
    let descriptor_provenance_hash = Value::String(
        canonical::hash(descriptor_graph.get("provenance").unwrap_or(&Value::Null))
            .unwrap_or_else(|_| "unavailable".into()),
    );
    let relation_schema = relation_graph
        .get("provenance")
        .and_then(|value| value.get("extractorSchema"));
    let descriptor_schema = descriptor_graph
        .get("provenance")
        .and_then(|value| value.get("extractorSchema"));
    let proven_null_policies = relation_graph
        .get("relations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("NULL_COALESCES"))
        .count();
    NullableDiscoveryDiagnostic {
        schema: "nullable-discovery-diagnostic/0.1".into(),
        stage,
        shapes: BTreeMap::from([
            (
                "expected.extractorSchema".into(),
                declaration_value_shape(Some(&expected_schema)),
            ),
            (
                "relations.extractorSchema".into(),
                declaration_value_shape(relation_schema),
            ),
            (
                "descriptors.extractorSchema".into(),
                declaration_value_shape(descriptor_schema),
            ),
            (
                "relations.coverage".into(),
                declaration_value_shape(relation_graph.get("coverage")),
            ),
            (
                "descriptors.coverage".into(),
                declaration_value_shape(descriptor_graph.get("coverage")),
            ),
            (
                "unknown.codes".into(),
                declaration_value_shape(Some(&unknown_codes)),
            ),
            (
                "relations.graphHash".into(),
                declaration_value_shape(Some(&relation_graph_hash)),
            ),
            (
                "descriptors.graphHash".into(),
                declaration_value_shape(Some(&descriptor_graph_hash)),
            ),
            (
                "relations.provenanceHash".into(),
                declaration_value_shape(Some(&relation_provenance_hash)),
            ),
            (
                "descriptors.provenanceHash".into(),
                declaration_value_shape(Some(&descriptor_provenance_hash)),
            ),
        ]),
        cardinalities: BTreeMap::from([
            (
                "relationUnknowns".into(),
                relation_boundaries.map_or(0, Vec::len),
            ),
            (
                "descriptorUnknowns".into(),
                descriptor_boundaries.map_or(0, Vec::len),
            ),
            ("provenNullCoalesces".into(), proven_null_policies),
        ]),
        range_relations: BTreeMap::new(),
        fact_relations: BTreeMap::from([
            (
                "relationsSchemaExpected".into(),
                if relation_schema == Some(&expected_schema) {
                    "EQUAL"
                } else {
                    "DIFFERENT"
                }
                .into(),
            ),
            (
                "descriptorsSchemaExpected".into(),
                if descriptor_schema == Some(&expected_schema) {
                    "EQUAL"
                } else {
                    "DIFFERENT"
                }
                .into(),
            ),
        ]),
    }
}

fn nullable_descriptor_diagnostic(
    stage: DeclarationProviderStage,
    null_policy: &Value,
    descriptor_rows: &[Value],
    selected_target: &str,
) -> NullableDiscoveryDiagnostic {
    let function_descriptors = descriptor_rows
        .iter()
        .filter(|descriptor| {
            descriptor.get("declarationKind").and_then(Value::as_str) == Some("FUNCTION")
        })
        .collect::<Vec<_>>();
    let matches = function_descriptors
        .iter()
        .copied()
        .filter(|descriptor| {
            descriptor.get("compilerCallableId").and_then(Value::as_str) == Some(selected_target)
        })
        .collect::<Vec<_>>();
    let descriptor = matches.first().copied();
    let compiler_targets = Value::Array(
        function_descriptors
            .iter()
            .filter_map(|descriptor| descriptor.get("compilerCallableId").cloned())
            .collect(),
    );
    let selected_target_value = Value::String(selected_target.into());
    let shapes = BTreeMap::from([
        (
            "relation.sourceTarget".into(),
            declaration_value_shape(null_policy.get("sourceTarget")),
        ),
        (
            "relation.fallbackTarget".into(),
            declaration_value_shape(null_policy.get("fallbackTarget")),
        ),
        (
            "selected.compilerTarget".into(),
            declaration_value_shape(Some(&selected_target_value)),
        ),
        (
            "descriptors.compilerTargets".into(),
            declaration_value_shape(Some(&compiler_targets)),
        ),
        (
            "matched.compilerCallableId".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("compilerCallableId"))),
        ),
        (
            "matched.returnType".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("returnType"))),
        ),
        (
            "matched.returnNullable".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("returnNullable"))),
        ),
        (
            "matched.declaredType".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("declaredType"))),
        ),
        (
            "matched.declaredNullable".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("declaredNullable"))),
        ),
    ]);
    let selected_equality = descriptor
        .and_then(|value| value.get("compilerCallableId"))
        .and_then(Value::as_str)
        .map_or("NO_MATCH", |identity| {
            if identity == selected_target {
                "EQUAL"
            } else {
                "DIFFERENT"
            }
        });
    let source_fallback_equality = match (
        null_policy.get("sourceTarget").and_then(Value::as_str),
        null_policy.get("fallbackTarget").and_then(Value::as_str),
    ) {
        (Some(source), Some(fallback)) if source == fallback => "EQUAL",
        (Some(_), Some(_)) => "DIFFERENT",
        _ => "UNKNOWN",
    };
    NullableDiscoveryDiagnostic {
        schema: "nullable-discovery-diagnostic/0.1".into(),
        stage,
        shapes,
        cardinalities: BTreeMap::from([
            ("allDescriptors".into(), descriptor_rows.len()),
            ("functionDescriptors".into(), function_descriptors.len()),
            ("matchingKindAndIdentity".into(), matches.len()),
        ]),
        range_relations: BTreeMap::new(),
        fact_relations: BTreeMap::from([
            (
                "selectedTargetToMatchedCompilerIdentity".into(),
                selected_equality.into(),
            ),
            (
                "sourceTargetToFallbackTarget".into(),
                source_fallback_equality.into(),
            ),
        ]),
    }
}

fn nullable_range_relation(
    left_start: Option<u64>,
    left_end: Option<u64>,
    right_start: Option<u64>,
    right_end: Option<u64>,
) -> String {
    let (Some(left_start), Some(left_end), Some(right_start), Some(right_end)) =
        (left_start, left_end, right_start, right_end)
    else {
        return "UNKNOWN".into();
    };
    if left_start == right_start && left_end == right_end {
        "EQUAL".into()
    } else if left_start <= right_start && left_end >= right_end {
        "CONTAINS".into()
    } else if right_start <= left_start && right_end >= left_end {
        "CONTAINED".into()
    } else if left_end <= right_start || right_end <= left_start {
        "DISJOINT".into()
    } else {
        "OVERLAPS".into()
    }
}

fn nullable_constructs_slot_diagnostic(
    null_policy: &Value,
    construction: &Value,
    mapping: &Value,
    relation_rows: &[Value],
    descriptor_rows: &[Value],
    owner: &str,
    destination_callable: &str,
) -> NullableDiscoveryDiagnostic {
    let merged = null_policy.get("mergedOccurrence");
    let merged_start = merged
        .and_then(|value| value.get("start"))
        .and_then(Value::as_u64);
    let owner_constructions = relation_rows
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("CONSTRUCTS"))
        .filter(|row| row.get("owner").and_then(Value::as_str) == Some(owner))
        .collect::<Vec<_>>();
    let matching_constructions = owner_constructions
        .iter()
        .filter(|row| {
            row.get("argumentToParameter")
                .and_then(Value::as_array)
                .is_some_and(|mappings| {
                    mappings.iter().any(|candidate| {
                        candidate.get("argumentStart").and_then(Value::as_u64) == merged_start
                    })
                })
        })
        .count();
    let descriptor_matches = descriptor_rows
        .iter()
        .filter(|descriptor| {
            descriptor.get("declarationKind").and_then(Value::as_str) == Some("CONSTRUCTOR")
                && descriptor.get("compilerCallableId").and_then(Value::as_str)
                    == Some(destination_callable)
        })
        .collect::<Vec<_>>();
    let descriptor = descriptor_matches.first().copied();
    let slots = descriptor
        .and_then(|value| value.get("parameterTypes"))
        .and_then(Value::as_array);
    let mut shapes = BTreeMap::from([
        (
            "nullPolicy.sourceTarget".into(),
            declaration_value_shape(null_policy.get("sourceTarget")),
        ),
        (
            "nullPolicy.fallbackTarget".into(),
            declaration_value_shape(null_policy.get("fallbackTarget")),
        ),
        (
            "nullPolicy.resultStart".into(),
            declaration_value_shape(merged.and_then(|value| value.get("start"))),
        ),
        (
            "nullPolicy.resultEnd".into(),
            declaration_value_shape(merged.and_then(|value| value.get("end"))),
        ),
        (
            "nullPolicy.resultType".into(),
            declaration_value_shape(merged.and_then(|value| value.get("type"))),
        ),
        (
            "nullPolicy.resultNullable".into(),
            declaration_value_shape(merged.and_then(|value| value.get("nullable"))),
        ),
        (
            "constructs.target".into(),
            declaration_value_shape(construction.get("target")),
        ),
        (
            "constructs.argumentStart".into(),
            declaration_value_shape(mapping.get("argumentStart")),
        ),
        (
            "constructs.argumentEnd".into(),
            declaration_value_shape(mapping.get("argumentEnd")),
        ),
        (
            "constructs.parameterIndex".into(),
            declaration_value_shape(mapping.get("parameterIndex")),
        ),
        (
            "constructs.argumentType".into(),
            declaration_value_shape(mapping.get("argumentType")),
        ),
        (
            "constructs.parameterType".into(),
            declaration_value_shape(mapping.get("parameterType")),
        ),
        (
            "descriptor.symbolIdentity".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("symbolIdentity"))),
        ),
        (
            "descriptor.compilerCallableId".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("compilerCallableId"))),
        ),
        (
            "descriptor.jvmDescriptor".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("jvmDescriptor"))),
        ),
        (
            "descriptor.parameterTypes".into(),
            declaration_value_shape(descriptor.and_then(|value| value.get("parameterTypes"))),
        ),
    ]);
    for (index, slot) in slots.into_iter().flatten().enumerate() {
        shapes.insert(
            format!("descriptor.slot[{index}].index"),
            declaration_value_shape(slot.get("index")),
        );
        shapes.insert(
            format!("descriptor.slot[{index}].type"),
            declaration_value_shape(slot.get("type")),
        );
        shapes.insert(
            format!("descriptor.slot[{index}].nullable"),
            declaration_value_shape(slot.get("nullable")),
        );
    }
    NullableDiscoveryDiagnostic {
        schema: "nullable-discovery-diagnostic/0.1".into(),
        stage: DeclarationProviderStage::ConstructsSlot,
        shapes,
        cardinalities: BTreeMap::from([
            ("ownerConstructs".into(), owner_constructions.len()),
            ("resultRangeMatches".into(), matching_constructions),
            ("constructorDescriptors".into(), descriptor_matches.len()),
            ("descriptorSlots".into(), slots.map_or(0, Vec::len)),
        ]),
        range_relations: BTreeMap::from([(
            "resultToArgument".into(),
            nullable_range_relation(
                merged_start,
                merged
                    .and_then(|value| value.get("end"))
                    .and_then(Value::as_u64),
                mapping.get("argumentStart").and_then(Value::as_u64),
                mapping.get("argumentEnd").and_then(Value::as_u64),
            ),
        )]),
        fact_relations: BTreeMap::new(),
    }
}

fn declaration_use_shape_diagnostic(
    relation: &Value,
    target: &Value,
) -> BTreeMap<String, DeclarationFieldShape> {
    let mut shapes = BTreeMap::from([
        (
            "relation.resultType".into(),
            declaration_value_shape(relation.get("resultType")),
        ),
        (
            "relation.argumentToParameter".into(),
            declaration_value_shape(relation.get("argumentToParameter")),
        ),
        (
            "target.returnType".into(),
            declaration_value_shape(target.get("returnType")),
        ),
        (
            "target.returnNullable".into(),
            declaration_value_shape(target.get("returnNullable")),
        ),
        (
            "target.parameterTypes".into(),
            declaration_value_shape(target.get("parameterTypes")),
        ),
    ]);
    if let Some(arguments) = relation
        .get("argumentToParameter")
        .and_then(Value::as_array)
    {
        for (index, argument) in arguments.iter().enumerate() {
            shapes.insert(
                format!("argument[{index}].parameterIndex"),
                declaration_value_shape(argument.get("parameterIndex")),
            );
            shapes.insert(
                format!("argument[{index}].parameterType"),
                declaration_value_shape(argument.get("parameterType")),
            );
        }
    }
    shapes
}

fn declaration_use_types_match(
    relation: &Value,
    target: &Value,
) -> Result<bool, TypedGoalRefusalReason> {
    match relation.get("kind").and_then(Value::as_str) {
        Some("OVERRIDES") => Ok(true),
        Some("REFERENCES") => Ok(declaration_required_str(relation, "resultType")?
            == declaration_required_str(target, "returnType")?),
        Some("CALLS") => {
            if declaration_required_str(relation, "resultType")?
                != declaration_required_str(target, "returnType")?
            {
                return Ok(false);
            }
            let parameters = declaration_parameter_types(target)
                .ok_or(TypedGoalRefusalReason::InsufficientEvidence)?;
            let Some(arguments) = relation
                .get("argumentToParameter")
                .and_then(Value::as_array)
            else {
                return if parameters.is_empty() {
                    Ok(true)
                } else {
                    Err(TypedGoalRefusalReason::InsufficientEvidence)
                };
            };
            for argument in arguments {
                let index = argument
                    .get("parameterIndex")
                    .and_then(Value::as_u64)
                    .ok_or(TypedGoalRefusalReason::InsufficientEvidence)?
                    as usize;
                let parameter_type = declaration_required_str(argument, "parameterType")?;
                if parameters.get(index).copied() != Some(parameter_type) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Err(TypedGoalRefusalReason::InsufficientEvidence),
    }
}

fn discover_value_flows(
    repo: &Path,
    requested_compilation: Option<&str>,
    worker: &mut WorkerClient,
) -> Result<Vec<DiscoveredValueFlow>, ClewError> {
    let compilation = select_production_compilation(repo, requested_compilation)?;
    let project = worker.request(
        RequestKind::OpenProject,
        &json!({"repo":repo,"compilation":&compilation}),
    )?;
    require_exact_project_compilation(&project, &compilation)?;
    let verified_index = worker.index_files_verified_after_project(
        &json!({"repo":repo,"compilation":&compilation,"syntaxOnly":false}),
        &project,
    )?;
    let index = worker.inspect_verified_index(&verified_index)?;
    if index.get("k2Validated").and_then(Value::as_bool) != Some(true)
        || has_error_diagnostic(index)
    {
        return Ok(vec![]);
    }
    let mut repository_index = RepositoryIndex::open_compilation(repo, Some(&compilation))?;
    let index_snapshot = repository_index.update_verified(&verified_index, worker)?;
    repository_index.require_fresh(REPOSITORY_INDEX_FACT)?;
    let mut symbols = index
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|file| {
            file.get("declarations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|declaration| {
            declaration.get("kind").and_then(Value::as_str) == Some("KtNamedFunction")
        })
        .filter(|declaration| {
            declaration
                .pointer("/symbolIdentity/returnType")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.starts_with("kotlin/collections/List<"))
        })
        .filter_map(|declaration| declaration.get("legacySymbolId").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    symbols.sort();
    symbols.dedup();

    let mut discovered = Vec::new();
    for symbol in symbols {
        let thread = match build_discovered_thread(
            repo,
            &compilation,
            &project,
            &index_snapshot,
            &symbol,
            worker,
        ) {
            Ok(thread) => thread,
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::IncompleteSemanticAnalysis
                        | ErrorCode::SymbolNotFound
                        | ErrorCode::AmbiguousSymbol
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        let edge = match map_value_edge(&thread) {
            Ok(edge) => edge,
            Err(_) => continue,
        };
        let (contexts, transformer_candidates) = discover_callable_candidates(index, &edge)?;
        let mut resolved_contexts = Vec::new();
        for context in contexts {
            if let Some(resolved) = resolve_callable_evidence(repo, &compilation, &context, worker)?
            {
                resolved_contexts.push(resolved);
            }
        }
        let mut transformers = Vec::new();
        for transformer in transformer_candidates {
            if let Some(resolved) =
                resolve_callable_evidence(repo, &compilation, &transformer, worker)?
            {
                transformers.push(resolved);
            }
        }
        transformers.sort_by(|left, right| {
            left.callable
                .compiler_symbol
                .cmp(&right.callable.compiler_symbol)
        });
        transformers.dedup_by(|left, right| {
            left.callable.compiler_symbol == right.callable.compiler_symbol
        });
        let mut map_candidates = Vec::new();
        for transformer in &transformers {
            for context in resolved_contexts.iter().filter(|context| {
                transformer.callable.parameter_types.get(1) == Some(&context.callable.return_type)
            }) {
                map_candidates.push(ResolvedMapCandidate {
                    context: context.clone(),
                    transformer: transformer.clone(),
                });
            }
        }
        map_candidates.sort_by(|left, right| {
            (
                &left.context.callable.compiler_symbol,
                &left.transformer.callable.compiler_symbol,
            )
                .cmp(&(
                    &right.context.callable.compiler_symbol,
                    &right.transformer.callable.compiler_symbol,
                ))
        });
        map_candidates.dedup_by(|left, right| {
            left.context.callable.compiler_symbol == right.context.callable.compiler_symbol
                && left.transformer.callable.compiler_symbol
                    == right.transformer.callable.compiler_symbol
        });
        if !transformers.is_empty() {
            discovered.push(DiscoveredValueFlow {
                thread_fingerprint: canonical::hash(&thread).map_err(internal)?,
                thread,
                edge,
                map_candidates,
                transformers,
                index_hash: index
                    .get("indexHash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            });
        }
    }
    Ok(discovered)
}

fn build_discovered_thread(
    repo: &Path,
    compilation: &str,
    project: &Value,
    index_snapshot: &str,
    symbol: &str,
    worker: &mut WorkerClient,
) -> Result<ThreadIr, ClewError> {
    let raw = worker.request(
        RequestKind::BuildLocalGraph,
        &json!({"repo":repo,"symbol":symbol,"compilation":compilation}),
    )?;
    let local: LocalGraph = serde_json::from_value(raw)
        .map_err(|error| ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string()))?;
    let graph = graph::enrich(local);
    let seed_id = graph
        .nodes
        .iter()
        .filter(|node| node.kind == "RETURN")
        .max_by_key(|node| {
            node.origin
                .as_ref()
                .and_then(|origin| origin.pointer("/rangeHint/1"))
                .and_then(Value::as_u64)
                .unwrap_or_default()
        })
        .or_else(|| graph.nodes.iter().rfind(|node| node.origin.is_some()))
        .map(|node| node.id.clone())
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "discovered callable has no source-backed graph seed",
            )
        })?;
    let snapshot = Snapshot {
        base_revision: git_head(repo)?,
        project_model_hash: project["projectModelHash"]
            .as_str()
            .unwrap_or_default()
            .into(),
        compiler_version: worker.capabilities.compiler_version.clone(),
        build_system: match project["buildSystem"].as_str() {
            Some("MAVEN") => BuildSystem::Maven,
            _ => BuildSystem::Gradle,
        },
        build_launcher: project["buildLauncher"]
            .as_str()
            .unwrap_or("./gradlew")
            .into(),
        index_snapshot: index_snapshot.into(),
        compilation: compilation.into(),
        compile_task: project["compileTask"]
            .as_str()
            .unwrap_or(":compileKotlin")
            .into(),
        test_tasks: project["testTasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    };
    let seed = json!({"kind":"DISCOVERED_OPERATOR_ROOT","symbol":symbol,"nodeId":seed_id});
    graph::slice(&graph, &seed_id, SlicePolicy::default(), snapshot, seed)
        .map_err(|error| ClewError::new(ErrorCode::IncompleteSemanticAnalysis, error.to_string()))
}

fn oracle_compilation_context(
    repo: &Path,
    compilation: &str,
    worker: &mut WorkerClient,
) -> Result<OracleCompilationContext, ClewError> {
    let project = worker.request(
        RequestKind::OpenProject,
        &json!({"repo":repo,"compilation":compilation}),
    )?;
    let index = worker.request(
        RequestKind::IndexFiles,
        &json!({"repo":repo,"compilation":compilation,"syntaxOnly":false}),
    )?;
    Ok(OracleCompilationContext {
        requested_compilation: compilation.to_owned(),
        module: required_str(&project, "module")?.to_owned(),
        source_set: required_str(&project, "sourceSet")?.to_owned(),
        source_roots: project
            .get("sourceRoots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        project_model_hash: project
            .get("projectModelHash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        classpath_hash: index
            .get("classpathHash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        compiler_options_hash: index
            .get("compilerOptionsHash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

fn oracle_compiler_diagnostic(
    context: &OracleCompilationContext,
    candidate_symbol_identity: &str,
    resolution: &Value,
) -> OracleCompilerDiagnostic {
    let mut diagnostic_codes = BTreeSet::new();
    let mut unresolved_symbols = BTreeSet::new();
    for diagnostic in resolution
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let message = diagnostic
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let lower = message.to_ascii_lowercase();
        let code = if lower.contains("unresolved reference") {
            "UNRESOLVED_REFERENCE"
        } else if lower.contains("cannot access") {
            "CANNOT_ACCESS"
        } else if lower.contains("incompatible") {
            "INCOMPATIBLE_BINARY"
        } else if diagnostic.get("severity").and_then(Value::as_str) == Some("ERROR") {
            "COMPILER_ERROR"
        } else {
            "COMPILER_DIAGNOSTIC"
        };
        diagnostic_codes.insert(code.to_owned());
        if code == "UNRESOLVED_REFERENCE" {
            let suffix = lower
                .split_once("unresolved reference")
                .map(|(_, suffix)| suffix)
                .unwrap_or_default();
            let symbol = suffix
                .trim_matches(|character: char| {
                    character.is_whitespace() || matches!(character, ':' | '\'' | '"')
                })
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '.')
                })
                .collect::<String>();
            if !symbol.is_empty() {
                unresolved_symbols.insert(symbol);
            }
        }
    }
    OracleCompilerDiagnostic {
        requested_compilation: context.requested_compilation.clone(),
        module: context.module.clone(),
        source_set: context.source_set.clone(),
        source_roots: context.source_roots.clone(),
        project_model_hash: context.project_model_hash.clone(),
        classpath_hash: context.classpath_hash.clone(),
        compiler_options_hash: context.compiler_options_hash.clone(),
        candidate_symbol_identity: candidate_symbol_identity.to_owned(),
        diagnostic_codes: diagnostic_codes.into_iter().collect(),
        unresolved_symbols: unresolved_symbols.into_iter().collect(),
    }
}

fn discover_test_symbols(
    repo: &Path,
    compilation: &str,
    worker: &mut WorkerClient,
) -> Result<Vec<DiscoveredTestCandidate>, ClewError> {
    let index = worker.request(
        RequestKind::IndexFiles,
        &json!({"repo":repo,"compilation":compilation,"syntaxOnly":false}),
    )?;
    let mut symbols = index
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|file| {
            file.get("declarations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|declaration| {
            declaration.get("kind").and_then(Value::as_str) == Some("KtNamedFunction")
                && declaration
                    .pointer("/symbolIdentity/sourceSet")
                    .and_then(Value::as_str)
                    == Some("test")
                && declaration
                    .pointer("/symbolIdentity/containingDeclarations")
                    .and_then(Value::as_array)
                    .is_some_and(|owners| !owners.is_empty())
        })
        .filter_map(|declaration| {
            let owner = declaration
                .pointer("/symbolIdentity/containingDeclarations")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(".");
            // ResolveSymbol's authoritative exact-key is the serialized
            // compiler identity (`symbolId`). The other forms are retained
            // only for workers that cannot emit a resolved identity.
            let compiler_identity = declaration.get("symbolId")?.as_str()?.to_owned();
            let mut queries = ["symbolId", "legacySymbolId", "name"]
                .into_iter()
                .filter_map(|field| declaration.get(field).and_then(Value::as_str))
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            queries.sort();
            queries.dedup();
            (!queries.is_empty()).then_some(DiscoveredTestCandidate {
                owner,
                compiler_identity,
                queries,
            })
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| left.queries.cmp(&right.queries));
    symbols.dedup_by(|left, right| left.queries == right.queries);
    Ok(symbols)
}

fn has_error_diagnostic(value: &Value) -> bool {
    value
        .get("diagnostics")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("severity").and_then(Value::as_str) == Some("ERROR"))
        })
}

fn map_value_edge(thread: &ThreadIr) -> Result<MapValueEdge, MapEdgeRefusalReason> {
    if thread.completeness.status != CompletenessStatus::CompleteSupportedSubset
        || !thread.completeness.boundaries.is_empty()
        || !thread.external_summaries.is_empty()
    {
        return Err(MapEdgeRefusalReason::UnsupportedBoundary);
    }
    if thread.nodes.iter().any(|node| {
        matches!(
            node.kind.as_str(),
            "BRANCH" | "LOOP" | "CAPTURE" | "THROW" | "ASSIGNMENT"
        )
    }) {
        return Err(MapEdgeRefusalReason::UnsupportedBoundary);
    }
    let node_by_id = thread
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut candidates = Vec::new();
    let parameters = thread
        .nodes
        .iter()
        .filter(|node| node.kind == "PARAMETER")
        .collect::<Vec<_>>();
    for parameter in &parameters {
        let Some(name) = parameter.defines.as_deref() else {
            continue;
        };
        let Some(collection_type) = parameter
            .attributes
            .get("declaredType")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(element_type) = eager_list_element_type(collection_type) else {
            continue;
        };
        let return_type = parameter
            .attributes
            .get("ownerReturnType")
            .and_then(Value::as_str);
        if return_type != Some(collection_type) || collection_type.contains('?') {
            continue;
        }
        let Some(workflow_symbol) = parameter
            .attributes
            .get("ownerCompilerSymbol")
            .and_then(Value::as_str)
        else {
            continue;
        };
        for edge in thread
            .edges
            .iter()
            .filter(|edge| edge.from == parameter.id && edge.kind == "DEF_USE")
        {
            let Some(consumer) = node_by_id.get(edge.to.as_str()) else {
                continue;
            };
            if consumer.kind != "RETURN"
                || !consumer.uses.iter().any(|used| used == name)
                || !thread
                    .edges
                    .iter()
                    .any(|item| item.from == consumer.id && item.kind == "RETURN")
            {
                continue;
            }
            candidates.push(MapValueEdge {
                workflow_symbol: workflow_symbol.to_owned(),
                from: parameter.id.clone(),
                to: consumer.id.clone(),
                parameter_index: parameter
                    .id
                    .strip_prefix("param:")
                    .and_then(|value| value.parse().ok())
                    .ok_or(MapEdgeRefusalReason::UnsupportedBoundary)?,
                placement: format!("{workflow_symbol}#FUNCTION_ENTRY"),
                collection_type: collection_type.to_owned(),
                element_type: element_type.clone(),
            });
        }
    }
    if candidates.is_empty()
        && parameters.iter().any(|parameter| {
            parameter
                .attributes
                .get("declaredType")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    kind.contains("Sequence<")
                        || kind.contains("Flow<")
                        || kind.contains("Iterable<")
                })
        })
    {
        return Err(MapEdgeRefusalReason::UnsupportedCollectionModality);
    }
    if candidates.len() != 1 {
        return Err(MapEdgeRefusalReason::NonUniqueValueEdge);
    }
    let selected = candidates.pop().expect("one edge");
    if thread
        .edges
        .iter()
        .any(|edge| edge.from == selected.from && edge.kind == "DEF_USE" && edge.to != selected.to)
        || thread.nodes.iter().any(|node| {
            node.id != selected.to
                && node
                    .uses
                    .iter()
                    .any(|name| node_by_id[&selected.from.as_str()].defines.as_ref() == Some(name))
        })
    {
        return Err(MapEdgeRefusalReason::IdentityOrAliasExposure);
    }
    Ok(selected)
}

fn eager_list_element_type(collection_type: &str) -> Option<String> {
    collection_type
        .strip_prefix("kotlin/collections/List<")
        .and_then(|value| value.strip_suffix('>'))
        .filter(|value| !value.is_empty() && !value.contains('?'))
        .map(str::to_owned)
}

fn discover_map_candidates(
    index: &Value,
    edge: &MapValueEdge,
) -> Result<Vec<(CallableCandidate, CallableCandidate)>, ClewError> {
    let (contexts, transformers) = discover_callable_candidates(index, edge)?;
    let mut pairs = Vec::new();
    for transformer in transformers {
        for context in contexts
            .iter()
            .filter(|context| context.return_type == transformer.parameter_types[1])
        {
            pairs.push((context.clone(), transformer.clone()));
        }
    }
    Ok(pairs)
}

fn discover_callable_candidates(
    index: &Value,
    edge: &MapValueEdge,
) -> Result<(Vec<CallableCandidate>, Vec<CallableCandidate>), ClewError> {
    let mut contexts = Vec::new();
    let mut transformers = Vec::new();
    for declaration in index
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|file| {
            file.get("declarations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
    {
        if declaration.get("kind").and_then(Value::as_str) != Some("KtNamedFunction") {
            continue;
        }
        let Some(identity) = declaration.get("symbolIdentity") else {
            continue;
        };
        if identity.get("suspendFlag").and_then(Value::as_bool) != Some(false) {
            continue;
        }
        let is_empty_identity_list = |field: &str| {
            identity
                .get(field)
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        };
        if !is_empty_identity_list("containingDeclarations")
            || !is_empty_identity_list("receiverTypes")
            || !is_empty_identity_list("contextReceiverTypes")
        {
            continue;
        }
        let Some(compiler_symbol) = declaration.get("compilerSymbol").and_then(Value::as_str)
        else {
            continue;
        };
        let Some(query_symbol) = declaration.get("legacySymbolId").and_then(Value::as_str) else {
            continue;
        };
        let parameter_types = identity
            .get("parameterTypes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let Some(return_type) = identity.get("returnType").and_then(Value::as_str) else {
            continue;
        };
        let candidate = CallableCandidate {
            compiler_symbol: compiler_symbol.to_owned(),
            query_symbol: query_symbol.to_owned(),
            parameter_types: parameter_types.clone(),
            return_type: return_type.to_owned(),
        };
        if parameter_types.is_empty() && return_type != "kotlin/Unit" && !return_type.contains('?')
        {
            contexts.push(candidate.clone());
        }
        if parameter_types.len() == 2
            && parameter_types[0] == edge.element_type
            && return_type == edge.element_type
            && !parameter_types.iter().any(|kind| kind.contains('?'))
        {
            transformers.push(candidate);
        }
    }
    Ok((contexts, transformers))
}

fn resolve_safe_callable(
    repo: &Path,
    compilation: &str,
    candidate: &CallableCandidate,
    worker: &mut WorkerClient,
) -> Result<Option<String>, ClewError> {
    Ok(
        resolve_callable_evidence(repo, compilation, candidate, worker)?
            .filter(|evidence| evidence.effects_proven_pure)
            .map(|evidence| evidence.resolution_fingerprint),
    )
}

fn resolve_callable_evidence(
    repo: &Path,
    compilation: &str,
    candidate: &CallableCandidate,
    worker: &mut WorkerClient,
) -> Result<Option<ResolvedCallableEvidence>, ClewError> {
    let resolution = worker.request(
        RequestKind::ResolveSymbol,
        &json!({"repo":repo,"compilation":compilation,"symbol":candidate.query_symbol}),
    )?;
    if resolution.get("k2Validated").and_then(Value::as_bool) != Some(true)
        || has_error_diagnostic(&resolution)
        || resolution
            .pointer("/declaration/compilerSymbol")
            .and_then(Value::as_str)
            != Some(candidate.compiler_symbol.as_str())
    {
        return Ok(None);
    }
    let semantic_facts = resolution
        .get("semanticFacts")
        .and_then(Value::as_array)
        .cloned();
    let effects_are_known = semantic_facts.as_ref().is_some_and(|facts| {
        facts
            .iter()
            .all(|fact| fact.get("effects").is_some_and(Value::is_array))
    });
    let has_effect = semantic_facts.as_ref().is_some_and(|facts| {
        facts.iter().any(|fact| {
            fact.get("effects")
                .and_then(Value::as_array)
                .is_some_and(|effects| !effects.is_empty())
        })
    });
    let resolved_calls = resolution.get("resolvedCalls").and_then(Value::as_array);
    let calls_are_known_pure = resolved_calls.is_some_and(|calls| {
        calls.iter().all(|call| {
            call.get("symbol")
                .and_then(Value::as_str)
                .is_some_and(known_pure_callable)
        })
    });
    verify_resolution_source(repo, &resolution)?;
    let type_facts = (
        &candidate.compiler_symbol,
        &candidate.parameter_types,
        &candidate.return_type,
        resolution.pointer("/declaration/compilerSymbol"),
    );
    let effect_facts = (
        semantic_facts.as_ref(),
        resolved_calls,
        effects_are_known,
        has_effect,
        calls_are_known_pure,
    );
    Ok(Some(ResolvedCallableEvidence {
        callable: candidate.clone(),
        resolution_fingerprint: canonical::hash(&resolution).map_err(internal)?,
        type_fingerprint: canonical::hash(&type_facts).map_err(internal)?,
        effect_fingerprint: canonical::hash(&effect_facts).map_err(internal)?,
        effects_proven_pure: effects_are_known && !has_effect && calls_are_known_pure,
    }))
}

fn known_pure_callable(symbol: &str) -> bool {
    matches!(
        symbol,
        "kotlin/Int.plus"
            | "kotlin/Int.minus"
            | "kotlin/Int.times"
            | "kotlin/Long.plus"
            | "kotlin/Long.minus"
            | "kotlin/Long.times"
            | "kotlin/Double.plus"
            | "kotlin/Double.minus"
            | "kotlin/Double.times"
            | "kotlin/Double.div"
    )
}

fn map_edge_invariants(
    base_evidence: &str,
    bindings: &MapEdgeBindingSummary,
) -> Result<Vec<MapEdgeInvariantProof>, ClewError> {
    [
        MapEdgeInvariant::TypeAssignable,
        MapEdgeInvariant::ContextEvaluatedOnce,
        MapEdgeInvariant::PlacementDominatesUses,
        MapEdgeInvariant::OrderPreserved,
        MapEdgeInvariant::CardinalityPreserved,
        MapEdgeInvariant::LazinessPreserved,
        MapEdgeInvariant::EffectsPreserved,
        MapEdgeInvariant::NullabilityPreserved,
        MapEdgeInvariant::ConsumerContractPreserved,
        MapEdgeInvariant::AbiPreserved,
        MapEdgeInvariant::BehavioralOracleAvailable,
        MapEdgeInvariant::NoUnsupportedBoundary,
    ]
    .into_iter()
    .map(|invariant| {
        Ok(MapEdgeInvariantProof {
            invariant,
            evidence_fingerprint: canonical::hash(&(base_evidence, invariant, bindings))
                .map_err(internal)?,
        })
    })
    .collect()
}

fn map_edge_change_graph(
    goal: &SemanticGoal,
    bindings: &MapEdgeBindingSummary,
    invariants: &[MapEdgeInvariantProof],
) -> ChangeGraph {
    let evidence = |invariant: MapEdgeInvariant| {
        invariants
            .iter()
            .find(|item| item.invariant == invariant)
            .map(|item| vec![item.evidence_fingerprint.clone()])
            .unwrap_or_default()
    };
    let edge = format!(
        "{}#{}->{}",
        bindings.workflow_symbol, bindings.value_edge_from, bindings.value_edge_to
    );
    let binding = |id: &str, role: BindingRole, subject: String| ChangeObligation {
        id: id.into(),
        kind: ObligationKind::BindUnique,
        binding_role: Some(role),
        subject: vec![subject],
        depends_on: vec![],
        evidence: evidence(MapEdgeInvariant::NoUnsupportedBoundary),
        status: DischargeStatus::Proved,
    };
    let item = |id: &str,
                kind: ObligationKind,
                subject: Vec<String>,
                depends_on: Vec<&str>,
                invariant: MapEdgeInvariant| ChangeObligation {
        id: id.into(),
        kind,
        binding_role: None,
        subject,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        evidence: evidence(invariant),
        status: DischargeStatus::Proved,
    };
    let mut obligations = vec![
        binding(
            "bind-context",
            BindingRole::ContextProducer,
            bindings.context_producer_symbol.clone(),
        ),
        binding(
            "bind-transformer",
            BindingRole::Transformer,
            bindings.transformer_symbol.clone(),
        ),
        binding("bind-edge", BindingRole::ValueEdge, edge.clone()),
        item(
            "type-assignable",
            ObligationKind::TypeAssignable,
            vec![bindings.transformer_symbol.clone(), edge.clone()],
            vec!["bind-transformer", "bind-edge"],
            MapEdgeInvariant::TypeAssignable,
        ),
        item(
            "introduce-once",
            ObligationKind::IntroduceOnce,
            vec![
                bindings.context_producer_symbol.clone(),
                bindings.placement.clone(),
            ],
            vec!["bind-context", "bind-edge"],
            MapEdgeInvariant::ContextEvaluatedOnce,
        ),
        item(
            "map-edge",
            ObligationKind::MapEdge,
            vec![bindings.transformer_symbol.clone(), edge.clone()],
            vec!["type-assignable", "introduce-once"],
            MapEdgeInvariant::PlacementDominatesUses,
        ),
    ];
    for (id, kind, invariant) in [
        (
            "preserve-order",
            ObligationKind::PreserveOrder,
            MapEdgeInvariant::OrderPreserved,
        ),
        (
            "preserve-cardinality",
            ObligationKind::PreserveCardinality,
            MapEdgeInvariant::CardinalityPreserved,
        ),
        (
            "preserve-laziness",
            ObligationKind::PreserveLaziness,
            MapEdgeInvariant::LazinessPreserved,
        ),
        (
            "preserve-effects",
            ObligationKind::PreserveEffects,
            MapEdgeInvariant::EffectsPreserved,
        ),
        (
            "preserve-nullability",
            ObligationKind::PreserveNullability,
            MapEdgeInvariant::NullabilityPreserved,
        ),
        (
            "preserve-consumer-contract",
            ObligationKind::PreserveConsumerContract,
            MapEdgeInvariant::ConsumerContractPreserved,
        ),
        (
            "preserve-abi",
            ObligationKind::PreserveAbi,
            MapEdgeInvariant::AbiPreserved,
        ),
    ] {
        obligations.push(item(
            id,
            kind,
            vec![edge.clone()],
            vec!["map-edge"],
            invariant,
        ));
    }
    obligations.push(item(
        "require-oracle",
        ObligationKind::RequireOracle,
        vec![bindings.transformer_symbol.clone(), edge.clone()],
        vec!["map-edge"],
        MapEdgeInvariant::BehavioralOracleAvailable,
    ));
    obligations.push(item(
        "boundary-check",
        ObligationKind::MustRefuseOnBoundary,
        vec![
            bindings.context_producer_symbol.clone(),
            bindings.transformer_symbol.clone(),
            edge,
        ],
        vec!["bind-context", "bind-transformer", "bind-edge"],
        MapEdgeInvariant::NoUnsupportedBoundary,
    ));
    ChangeGraph {
        schema: crate::semantic_goal::CHANGE_GRAPH_SCHEMA.into(),
        goal_schema: goal.schema.clone(),
        obligations,
    }
}

fn producer_transform_consumer_candidates(thread: &ThreadIr) -> Vec<(String, String, String)> {
    let node_by_id = thread
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let edge_exists = |from: &str, to: &str, kind: &str| {
        thread
            .edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.kind == kind)
    };
    let mut candidates = Vec::new();
    for transformer in thread.nodes.iter().filter(|node| {
        node.kind == "DEFINITION" && node.defines.as_ref().is_some_and(|name| !name.is_empty())
    }) {
        let transformed = transformer.defines.as_deref().unwrap_or_default();
        let immutable_local = transformer
            .origin
            .as_ref()
            .and_then(|origin| origin.get("sourceText"))
            .and_then(Value::as_str)
            .is_some_and(|source| source.trim_start().starts_with("val "));
        let has_transform_call = thread.edges.iter().any(|edge| {
            edge.from == transformer.id
                && edge.kind == "AST_CHILD"
                && node_by_id.get(edge.to.as_str()).is_some_and(|node| {
                    node.kind == "CALL_RESULT"
                        && edge_exists(&node.id, &transformer.id, "CFG_NORMAL")
                })
        });
        if !immutable_local || !has_transform_call {
            continue;
        }
        for producer in thread.nodes.iter().filter(|node| {
            node.kind == "PARAMETER"
                && node
                    .defines
                    .as_ref()
                    .is_some_and(|name| transformer.uses.contains(name))
                && edge_exists(&node.id, &transformer.id, "DEF_USE")
        }) {
            for consumer in thread.nodes.iter().filter(|node| {
                node.kind == "RETURN"
                    && node.uses.iter().any(|name| name == transformed)
                    && edge_exists(&transformer.id, &node.id, "DEF_USE")
                    && thread
                        .edges
                        .iter()
                        .any(|edge| edge.from == node.id && edge.kind == "RETURN")
            }) {
                candidates.push((
                    producer.id.clone(),
                    transformer.id.clone(),
                    consumer.id.clone(),
                ));
            }
        }
    }
    candidates
}

fn compiler_owner_symbol(thread: &ThreadIr, producer_id: &str) -> Result<String, ClewError> {
    thread
        .nodes
        .iter()
        .find(|node| node.id == producer_id && node.kind == "PARAMETER")
        .and_then(|node| node.attributes.get("ownerCompilerSymbol"))
        .and_then(Value::as_str)
        .filter(|symbol| !symbol.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            invalid_source("bound producer has no compiler-issued owner callable symbol")
        })
}

fn verify_assertion_of_target(
    resolution: &Value,
    target_compiler_symbol: &str,
) -> Result<(), ClewError> {
    let semantic_facts = resolution
        .get("semanticFacts")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("test resolution has no semantic facts"))?;
    let is_test = semantic_facts.iter().any(|fact| {
        fact.get("kind").and_then(Value::as_str) == Some("FirAnnotationCallImpl")
            && fact
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.contains("org/junit/jupiter/api/Test"))
    });
    if !is_test {
        return Err(invalid_source(
            "resolved function has no compiler-confirmed JUnit test annotation",
        ));
    }
    let calls = resolution
        .get("resolvedCalls")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("test resolution has no resolved calls"))?;
    let target_calls = calls
        .iter()
        .filter(|call| call.get("symbol").and_then(Value::as_str) == Some(target_compiler_symbol))
        .collect::<Vec<_>>();
    let [target_call] = target_calls.as_slice() else {
        return Err(invalid_source(
            "test must call the exact production callable once",
        ));
    };
    let target_start = target_call
        .get("start")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_source("target call has no compiler source range"))?;
    let assertion = calls.iter().find(|call| {
        call.get("symbol").and_then(Value::as_str) == Some("kotlin/test/assertEquals")
            && call
                .get("argumentToParameter")
                .and_then(Value::as_array)
                .is_some_and(|arguments| {
                    let has_expected = arguments.iter().any(|argument| {
                        argument.get("parameter").and_then(Value::as_str) == Some("expected")
                    });
                    let actual_is_target = arguments.iter().any(|argument| {
                        argument.get("parameter").and_then(Value::as_str) == Some("actual")
                            && argument.get("argumentStart").and_then(Value::as_u64)
                                == Some(target_start)
                    });
                    has_expected && actual_is_target
                })
    });
    if assertion.is_none() {
        return Err(invalid_source(
            "test does not assert the result of the exact production call",
        ));
    }
    Ok(())
}

fn verify_context_argument_of_target(
    resolution: &Value,
    target_compiler_symbol: &str,
    context_compiler_symbol: &str,
) -> Result<(), ClewError> {
    let calls = resolution
        .get("resolvedCalls")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("test resolution has no resolved calls"))?;
    let targets = calls
        .iter()
        .filter(|call| call.get("symbol").and_then(Value::as_str) == Some(target_compiler_symbol))
        .collect::<Vec<_>>();
    let contexts = calls
        .iter()
        .filter(|call| call.get("symbol").and_then(Value::as_str) == Some(context_compiler_symbol))
        .collect::<Vec<_>>();
    let ([target], [context]) = (targets.as_slice(), contexts.as_slice()) else {
        return Err(invalid_source(
            "behavioral oracle must call the exact transformer and context producer once",
        ));
    };
    let context_start = context
        .get("start")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_source("context call has no compiler source range"))?;
    let context_is_target_argument = target
        .get("argumentToParameter")
        .and_then(Value::as_array)
        .is_some_and(|arguments| {
            arguments.iter().any(|argument| {
                argument.get("argumentStart").and_then(Value::as_u64) == Some(context_start)
            })
        });
    if !context_is_target_argument {
        return Err(invalid_source(
            "behavioral oracle does not pass the context producer to the transformer",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ConditionalOracleEvidence {
    evidence_fingerprint: String,
    target_identity_exact: bool,
}

fn conditional_oracle_evidence(
    resolution: &Value,
    target_compiler_symbol: &str,
    context_compiler_symbol: &str,
) -> Result<Option<ConditionalOracleEvidence>, ClewError> {
    let calls = resolution
        .get("resolvedCalls")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("test resolution has no resolved calls"))?;
    if calls.is_empty() {
        return conditional_source_oracle_evidence(
            resolution,
            target_compiler_symbol,
            context_compiler_symbol,
        );
    }
    let unique = |symbol: &str| {
        let matching = calls
            .iter()
            .filter(|call| call.get("symbol").and_then(Value::as_str) == Some(symbol))
            .collect::<Vec<_>>();
        if matching.len() == 1 {
            Some(matching[0])
        } else {
            None
        }
    };
    let (Some(target), Some(context), Some(assertion)) = (
        unique(target_compiler_symbol),
        unique(context_compiler_symbol),
        unique("kotlin/test/assertEquals"),
    ) else {
        return Ok(None);
    };
    let range = |call: &Value| Some((call.get("start")?.as_u64()?, call.get("end")?.as_u64()?));
    let (
        Some((target_start, target_end)),
        Some((context_start, context_end)),
        Some((assert_start, assert_end)),
    ) = (range(target), range(context), range(assertion))
    else {
        return Ok(None);
    };
    if !(assert_start <= target_start
        && target_end <= assert_end
        && target_start <= context_start
        && context_end <= target_end)
    {
        return Ok(None);
    }
    let assertion_mapping = assertion
        .get("argumentToParameter")
        .and_then(Value::as_array);
    let target_mapping = target.get("argumentToParameter").and_then(Value::as_array);
    let assertion_proves_target = assertion_mapping.is_some_and(|arguments| {
        arguments.iter().any(|argument| {
            argument.get("parameter").and_then(Value::as_str) == Some("actual")
                && argument.get("argumentStart").and_then(Value::as_u64) == Some(target_start)
        })
    });
    let target_proves_context = target_mapping.is_some_and(|arguments| {
        arguments.iter().any(|argument| {
            argument.get("argumentStart").and_then(Value::as_u64) == Some(context_start)
        })
    });
    if assertion_mapping.is_some_and(|arguments| !arguments.is_empty()) && !assertion_proves_target
        || target_mapping.is_some_and(|arguments| !arguments.is_empty()) && !target_proves_context
    {
        return Ok(None);
    }
    if assertion_proves_target && target_proves_context {
        return Ok(None);
    }
    Ok(Some(ConditionalOracleEvidence {
        evidence_fingerprint: canonical::hash(&json!({
            "authority":"SOURCE_STRUCTURAL",
            "target":target_compiler_symbol,
            "context":context_compiler_symbol,
            "targetRange":[target_start,target_end],
            "contextRange":[context_start,context_end],
            "assertionRange":[assert_start,assert_end],
            "assertionMappingProved":assertion_proves_target,
            "contextMappingProved":target_proves_context,
        }))
        .map_err(internal)?,
        target_identity_exact: true,
    }))
}

fn conditional_source_oracle_evidence(
    resolution: &Value,
    target_compiler_symbol: &str,
    context_compiler_symbol: &str,
) -> Result<Option<ConditionalOracleEvidence>, ClewError> {
    let short_name = |symbol: &str| {
        symbol
            .rsplit(['/', '.'])
            .next()
            .unwrap_or(symbol)
            .to_owned()
    };
    let target = short_name(target_compiler_symbol);
    let context = short_name(context_compiler_symbol);
    if target.is_empty() || context.is_empty() || target == context {
        return Ok(None);
    }
    let calls = resolution
        .get("calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if calls.iter().filter(|call| **call == target).count() != 1
        || calls.iter().filter(|call| **call == context).count() != 1
        || calls.iter().filter(|call| **call == "assertEquals").count() != 1
    {
        return Ok(None);
    }
    let source = resolution
        .pointer("/bodyAnchor/sourceText")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let call_start = |name: &str| {
        let needle = format!("{name}(");
        let starts = source
            .match_indices(&needle)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if starts.len() == 1 {
            Some(starts[0])
        } else {
            None
        }
    };
    let (Some(assertion_start), Some(target_start), Some(context_start)) = (
        call_start("assertEquals"),
        call_start(&target),
        call_start(&context),
    ) else {
        return Ok(None);
    };
    let closing_paren = |start: usize| {
        let mut depth = 0usize;
        for (offset, character) in source[start..].char_indices() {
            match character {
                '(' => depth += 1,
                ')' if depth == 1 => return Some(start + offset),
                ')' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        None
    };
    let (Some(assertion_end), Some(target_end), Some(context_end)) = (
        closing_paren(assertion_start),
        closing_paren(target_start),
        closing_paren(context_start),
    ) else {
        return Ok(None);
    };
    if !(assertion_start < target_start
        && target_end < assertion_end
        && target_start < context_start
        && context_end < target_end)
    {
        return Ok(None);
    }
    Ok(Some(ConditionalOracleEvidence {
        evidence_fingerprint: canonical::hash(&json!({
            "authority":"SOURCE_STRUCTURAL",
            "targetCandidate":target_compiler_symbol,
            "contextCandidate":context_compiler_symbol,
            "bodyAnchor":resolution.pointer("/bodyAnchor/anchorId"),
            "targetRange":[target_start,target_end],
            "contextRange":[context_start,context_end],
            "assertionRange":[assertion_start,assertion_end],
        }))
        .map_err(internal)?,
        target_identity_exact: false,
    }))
}

fn verify_live_sources(
    repo: &Path,
    thread: &ThreadIr,
) -> Result<BTreeMap<PathBuf, String>, ClewError> {
    let canonical_repo = repo.canonicalize().map_err(|error| {
        invalid_source(format!("cannot resolve authority repository root: {error}"))
    })?;
    let root_metadata = std::fs::symlink_metadata(&canonical_repo).map_err(internal)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(invalid_source(
            "authority repository root is not a real directory",
        ));
    }
    let mut files = BTreeMap::new();
    let mut exact_origins = 0usize;
    for node in &thread.nodes {
        let Some(origin) = node.origin.as_ref() else {
            continue;
        };
        let file_id = required_str(origin, "fileId")?;
        let anchor_id = required_str(origin, "anchorId")?;
        let exact_text_hash = required_str(origin, "exactTextHash")?;
        let source_text = required_str(origin, "sourceText")?;
        let range = origin
            .get("rangeHint")
            .and_then(Value::as_array)
            .filter(|range| range.len() == 2)
            .ok_or_else(|| invalid_source("source origin has no exact byte range"))?;
        let start = range[0]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| invalid_source("invalid source range start"))?;
        let end = range[1]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| invalid_source("invalid source range end"))?;
        let relative = safe_relative_path(file_id)?;
        let path = canonical_repo
            .join(&relative)
            .canonicalize()
            .map_err(|error| invalid_source(format!("cannot resolve source {file_id}: {error}")))?;
        if path == canonical_repo || !path.starts_with(&canonical_repo) {
            return Err(invalid_source("source origin escapes authority repository"));
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(internal)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid_source(
                "source origin is not a contained regular file",
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| invalid_source(format!("cannot read source {file_id}: {error}")))?;
        let exact = bytes
            .get(start..end)
            .ok_or_else(|| invalid_source("source origin range is outside the file"))?;
        if exact != source_text.as_bytes()
            || canonical::hash_bytes(exact) != exact_text_hash
            || !thread.read_set.iter().any(|fact| {
                fact.kind == "SOURCE_NODE" && fact.key == anchor_id && fact.hash == exact_text_hash
            })
        {
            return Err(invalid_source(
                "source bytes, anchor hash, and Thread IR ReadSet disagree",
            ));
        }
        files.insert(relative, canonical::hash_bytes(&bytes));
        exact_origins += 1;
    }
    if exact_origins == 0 {
        return Err(invalid_source(
            "worker-rebuilt Thread IR has no exact source origin",
        ));
    }
    Ok(files)
}

fn verify_resolution_source(
    repo: &Path,
    resolution: &Value,
) -> Result<BTreeMap<PathBuf, String>, ClewError> {
    let anchor = resolution
        .get("bodyAnchor")
        .ok_or_else(|| invalid_source("test resolution has no exact body anchor"))?;
    let file_id = required_str(anchor, "fileId")?;
    let exact_text_hash = required_str(anchor, "exactTextHash")?;
    let source_text = required_str(anchor, "sourceText")?;
    let range = anchor
        .get("rangeHint")
        .and_then(Value::as_array)
        .filter(|range| range.len() == 2)
        .ok_or_else(|| invalid_source("test anchor has no exact byte range"))?;
    let start = range[0]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| invalid_source("invalid test source range start"))?;
    let end = range[1]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| invalid_source("invalid test source range end"))?;
    let relative = safe_relative_path(file_id)?;
    let path = repo
        .join(&relative)
        .canonicalize()
        .map_err(|error| invalid_source(format!("cannot resolve test source: {error}")))?;
    if !path.starts_with(repo) {
        return Err(invalid_source("test source escapes authority repository"));
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| invalid_source(format!("cannot read test source: {error}")))?;
    let exact = bytes
        .get(start..end)
        .ok_or_else(|| invalid_source("test source range is outside the file"))?;
    if exact != source_text.as_bytes() || canonical::hash_bytes(exact) != exact_text_hash {
        return Err(invalid_source(
            "worker test anchor does not match the authority checkout",
        ));
    }
    Ok(BTreeMap::from([(relative, canonical::hash_bytes(&bytes))]))
}

fn verify_sources_current(
    repo: &Path,
    expected: &BTreeMap<PathBuf, String>,
) -> Result<(), ClewError> {
    for (relative, hash) in expected {
        let current = std::fs::read(repo.join(relative)).map_err(|error| {
            invalid_source(format!("cannot reread {}: {error}", relative.display()))
        })?;
        if canonical::hash_bytes(&current) != *hash {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                format!(
                    "source {} changed after authority verification",
                    relative.display()
                ),
            ));
        }
    }
    Ok(())
}

fn thread_set_fingerprint(verified: &[&VerifiedThread]) -> Result<String, ClewError> {
    let mut fingerprints = verified
        .iter()
        .map(|item| item.fingerprint.clone())
        .collect::<Vec<_>>();
    fingerprints.sort();
    if fingerprints.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "duplicate semantic thread evidence is not an independent required root",
        ));
    }
    canonical::hash(&fingerprints).map_err(internal)
}

fn test_set_fingerprint(verified: &[&VerifiedBehavioralTest]) -> Result<String, ClewError> {
    let mut fingerprints = verified
        .iter()
        .map(|item| item.fingerprint.clone())
        .collect::<Vec<_>>();
    fingerprints.sort();
    if fingerprints.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "duplicate behavioral-test evidence is not an independent oracle",
        ));
    }
    canonical::hash(&fingerprints).map_err(internal)
}

fn project_source_roots(project: &Value) -> Result<(Vec<PathBuf>, Vec<PathBuf>), ClewError> {
    let roots = project
        .get("sourceRoots")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("project model has no source roots"))?
        .iter()
        .map(|root| {
            root.as_str()
                .ok_or_else(|| invalid_source("project source root is not a string"))
                .and_then(safe_relative_path)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let generated = project
        .get("generatedSourceRoots")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("project model has no generated source roots"))?
        .iter()
        .map(|root| {
            root.as_str()
                .ok_or_else(|| invalid_source("generated source root is not a string"))
                .and_then(safe_relative_path)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if roots.is_empty() {
        return Err(invalid_source("project model source roots are empty"));
    }
    Ok((roots, generated))
}

#[derive(Debug, PartialEq, Eq)]
struct CandidateFileClassification {
    production_files: BTreeSet<String>,
    test_files: BTreeSet<String>,
}

fn sibling_test_compilation(production: &str) -> Result<String, ClewError> {
    production
        .strip_suffix("/main")
        .filter(|module| !module.is_empty())
        .map(|module| format!("{module}/test"))
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "production compilation has no authority-defined sibling test compilation",
            )
        })
}

fn classify_candidate_files(
    changed_files: &[String],
    candidates: &BTreeMap<String, String>,
    production_roots: &[PathBuf],
    test_roots: &[PathBuf],
    production_generated: &[PathBuf],
    test_generated: &[PathBuf],
) -> Result<CandidateFileClassification, ClewError> {
    let unique_changed = changed_files.iter().cloned().collect::<BTreeSet<_>>();
    let candidate_keys = candidates.keys().cloned().collect::<BTreeSet<_>>();
    if unique_changed.len() != changed_files.len() || unique_changed != candidate_keys {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "candidate preview contains duplicate or inconsistent changed-file identities",
        ));
    }
    let mut production_files = BTreeSet::new();
    let mut test_files = BTreeSet::new();
    for file in candidate_keys {
        let path = safe_relative_path(&file)?;
        if production_generated
            .iter()
            .chain(test_generated)
            .any(|root| path.starts_with(root))
        {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "candidate overlay writes a generated source root",
            ));
        }
        let production = production_roots.iter().any(|root| path.starts_with(root));
        let test = test_roots.iter().any(|root| path.starts_with(root));
        match (production, test) {
            (true, false) => {
                production_files.insert(file);
            }
            (false, true) => {
                test_files.insert(file);
            }
            (false, false) => {
                return Err(ClewError::new(
                    ErrorCode::PreconditionFailed,
                    "candidate overlay contains an unclassified or outside-authority write",
                ));
            }
            (true, true) => {
                return Err(ClewError::new(
                    ErrorCode::PreconditionFailed,
                    "candidate overlay source root classification is mixed",
                ));
            }
        }
    }
    if production_files.is_empty() {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "differential validation requires a non-empty production delta",
        ));
    }
    Ok(CandidateFileClassification {
        production_files,
        test_files,
    })
}

fn validate_differential_overlay_state(
    overlay: &CandidateOverlay,
    current_revision: &str,
    current_production_model_hash: &str,
    current_test_model_hash: &str,
    current_test_compile_task: &str,
    current_route: &ValidationRoute,
) -> Result<(), ClewError> {
    if overlay.revision != current_revision || overlay.production_files.is_empty() {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "candidate overlay revision or production delta is stale",
        ));
    }
    let test_fingerprint = overlay
        .test_fingerprint
        .as_ref()
        .ok_or_else(|| invalid_receipt("candidate overlay test binding"))?;
    let stored_route = overlay
        .route
        .as_ref()
        .ok_or_else(|| invalid_receipt("candidate overlay validation route"))?;
    if overlay.production_project_model_hash != current_production_model_hash
        || overlay.test_project_model_hash != current_test_model_hash
        || overlay.test_compile_task != current_test_compile_task
        || stored_route != current_route
    {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "differential validation project model or route changed",
        ));
    }
    let expected_hash = canonical::hash(&(
        &overlay.revision,
        &overlay.thread_fingerprint,
        test_fingerprint,
        &overlay.production_project_model_hash,
        &overlay.test_compilation,
        &overlay.test_project_model_hash,
        &overlay.test_compile_task,
        canonical::hash(stored_route).map_err(internal)?,
        &overlay.candidates,
        &overlay.production_files,
        &overlay.test_files,
        &overlay.affected_callables,
    ))
    .map_err(internal)?;
    if expected_hash != overlay.overlay_hash {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "candidate overlay authority storage changed",
        ));
    }
    Ok(())
}

fn candidate_oracle_failure_stage(error: &ClewError) -> CandidateOracleFailureStage {
    if matches!(error.code, ErrorCode::SymbolNotFound) {
        CandidateOracleFailureStage::ResolveSymbol
    } else if matches!(
        error.code,
        ErrorCode::AmbiguousSymbol | ErrorCode::AmbiguousTarget
    ) {
        CandidateOracleFailureStage::Ambiguous
    } else if error.message.contains("not cleanly resolved by K2")
        || error.message.contains("no semantic facts")
        || error.message.contains("no resolved calls")
    {
        CandidateOracleFailureStage::K2Validation
    } else if error
        .message
        .contains("must call the exact production callable once")
    {
        CandidateOracleFailureStage::MissingExactCall
    } else if error
        .message
        .contains("does not assert the result of the exact production call")
    {
        CandidateOracleFailureStage::AssertionActualNotDerived
    } else {
        CandidateOracleFailureStage::IdentityMismatch
    }
}

fn compiler_validated_test_call_identity(
    repo: &Path,
    authority_target: &str,
    test_symbol: &str,
    production_compilation: &str,
    test_compilation: &str,
    worker: &mut WorkerClient,
) -> Result<String, ClewError> {
    let target_resolution = worker.request(
        RequestKind::ResolveSymbol,
        &json!({"repo":repo,"compilation":production_compilation,"symbol":authority_target}),
    )?;
    let target_identity = exact_compiler_identity(&target_resolution)?;
    let test_resolution = worker.request(
        RequestKind::ResolveSymbol,
        &json!({"repo":repo,"compilation":test_compilation,"symbol":test_symbol}),
    )?;
    if test_resolution.get("k2Validated").and_then(Value::as_bool) != Some(true)
        || has_error_diagnostic(&test_resolution)
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "candidate-scoped test is not cleanly resolved by K2",
        ));
    }
    let calls = test_resolution
        .get("resolvedCalls")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("candidate-scoped test has no resolved calls"))?;
    let mut matches = Vec::new();
    for call in calls {
        let raw = call
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_source("K2 resolved call has no callableId"))?;
        let query = strict_top_level_callable_id_query(raw)?;
        match worker.request(
            RequestKind::ResolveSymbol,
            &json!({"repo":repo,"compilation":production_compilation,"symbol":query}),
        ) {
            Ok(resolution) if exact_compiler_identity(&resolution)? == target_identity => {
                matches.push(raw.to_owned());
            }
            Ok(_) => {}
            Err(error) if error.code == ErrorCode::SymbolNotFound => {}
            Err(error) if error.code == ErrorCode::AmbiguousSymbol => {
                return Err(ClewError::new(
                    ErrorCode::AmbiguousTarget,
                    "slash-form callableId resolves ambiguously in production compilation",
                ));
            }
            Err(error) => return Err(error),
        }
    }
    matches.sort();
    matches.dedup();
    select_unique_compiler_call(matches)
}

fn select_unique_compiler_call(matches: Vec<String>) -> Result<String, ClewError> {
    match matches.as_slice() {
        [matched] => Ok(matched.clone()),
        [] => Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "test must call the exact production callable once",
        )),
        _ => Err(ClewError::new(
            ErrorCode::AmbiguousTarget,
            "multiple K2 calls resolve to the affected compiler declaration",
        )),
    }
}

fn exact_compiler_identity(resolution: &Value) -> Result<Value, ClewError> {
    resolution
        .pointer("/declaration/symbolIdentity")
        .cloned()
        .ok_or_else(|| invalid_source("resolved declaration has no exact compiler identity"))
}

fn strict_top_level_callable_id_query(callable_id: &str) -> Result<String, ClewError> {
    if callable_id.is_empty() || callable_id.contains(['.', '#', '$', '{', '}', '(', ')', '<', '>'])
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "resolved callableId is not a supported top-level compiler identity form",
        ));
    }
    let parts = callable_id.split('/').collect::<Vec<_>>();
    if parts.len() < 2
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().enumerate().all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphanumeric() && (index > 0 || !byte.is_ascii_digit())
                })
        })
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "resolved callableId has invalid package/callable components",
        ));
    }
    Ok(parts.join("."))
}

fn candidate_identity_comparison(
    repo: &Path,
    target_symbol: &str,
    test_symbol: &str,
    production_compilation: &str,
    test_compilation: &str,
    worker: &mut WorkerClient,
) -> Result<Value, ClewError> {
    let target_resolution = worker.request(
        RequestKind::ResolveSymbol,
        &json!({"repo":repo,"compilation":production_compilation,"symbol":target_symbol}),
    )?;
    let target = compiler_identity_summary(target_symbol, &target_resolution)?;
    let target_identity_hash = target
        .get("identityHash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let test_resolution = worker.request(
        RequestKind::ResolveSymbol,
        &json!({"repo":repo,"compilation":test_compilation,"symbol":test_symbol}),
    )?;
    let mut call_symbols = test_resolution
        .get("resolvedCalls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| call.get("symbol").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    call_symbols.sort();
    call_symbols.dedup();
    let mut calls = Vec::new();
    for symbol in call_symbols {
        let symbol_summary = compiler_symbol_serialization_summary(&symbol)?;
        let resolution = worker.request(
            RequestKind::ResolveSymbol,
            &json!({"repo":repo,"compilation":production_compilation,"symbol":symbol}),
        );
        match resolution {
            Ok(resolution) => {
                let identity = compiler_identity_summary(&symbol, &resolution)?;
                calls.push(json!({
                    "symbol":symbol_summary,
                    "identity":identity,
                    "sameCompilerDeclaration":identity.get("identityHash").and_then(Value::as_str) == Some(target_identity_hash.as_str())
                }));
            }
            Err(error) => calls.push(json!({
                "symbol":symbol_summary,
                "identity":{"status":"UNRESOLVED","code":error.code}
            })),
        }
    }
    calls.sort_by_key(|call| canonical::hash(call).unwrap_or_default());
    Ok(json!({
        "schema":"candidate-identity-comparison/0.1",
        "productionCompilationHash":canonical::hash(&production_compilation).map_err(internal)?,
        "testCompilationHash":canonical::hash(&test_compilation).map_err(internal)?,
        "target":target,
        "resolvedCalls":calls
    }))
}

fn compiler_identity_summary(symbol: &str, resolution: &Value) -> Result<Value, ClewError> {
    let declaration = resolution
        .get("declaration")
        .ok_or_else(|| invalid_source("resolved compiler symbol has no declaration"))?;
    let identity = declaration
        .get("symbolIdentity")
        .ok_or_else(|| invalid_source("resolved compiler symbol has no symbol identity"))?;
    let hash_field = |name: &str| -> Result<String, ClewError> {
        canonical::hash(identity.get(name).unwrap_or(&Value::Null)).map_err(internal)
    };
    Ok(json!({
        "identitySchema":"semantic-symbol/0.1#symbolIdentity",
        "module":identity.get("module").and_then(Value::as_str).unwrap_or("<missing>"),
        "sourceSet":identity.get("sourceSet").and_then(Value::as_str).unwrap_or("<missing>"),
        "packageHash":hash_field("package")?,
        "containingDeclarationsHash":hash_field("containingDeclarations")?,
        "callableNameHash":hash_field("declarationName")?,
        "parameterTypesHash":hash_field("parameterTypes")?,
        "returnTypeHash":hash_field("returnType")?,
        "receiverTypesHash":hash_field("receiverTypes")?,
        "contextReceiverTypesHash":hash_field("contextReceiverTypes")?,
        "jvmDescriptorHash":hash_field("jvmDescriptor")?,
        "identityHash":canonical::hash(identity).map_err(internal)?,
        "symbol":compiler_symbol_serialization_summary(symbol)?
    }))
}

fn compiler_symbol_serialization_summary(symbol: &str) -> Result<Value, ClewError> {
    Ok(json!({
        "symbolHash":canonical::hash(&symbol).map_err(internal)?,
        "slashCount":symbol.bytes().filter(|byte| *byte == b'/').count(),
        "dotCount":symbol.bytes().filter(|byte| *byte == b'.').count(),
        "hasCallableSeparator":symbol.contains('#'),
        "terminalNameHash":canonical::hash(&symbol.rsplit(['/', '.', '#']).next().unwrap_or_default()).map_err(internal)?
    }))
}

fn apply_candidate_overlay(
    worktree: &Path,
    overlay: &CandidateOverlay,
    include_production: bool,
) -> Result<(), ClewError> {
    for (file, source) in &overlay.candidates {
        let production = overlay.production_files.contains(file);
        let test = overlay.test_files.contains(file);
        if !(test || include_production && production) {
            continue;
        }
        if production == test {
            return Err(invalid_source(
                "stored candidate overlay has an invalid source classification",
            ));
        }
        let relative = safe_relative_path(file)?;
        let path = worktree.join(&relative);
        let parent = path
            .parent()
            .ok_or_else(|| invalid_source("candidate overlay path has no parent"))?;
        std::fs::create_dir_all(parent).map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot create candidate overlay parent: {error}"),
            )
        })?;
        let canonical_parent = parent.canonicalize().map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot resolve candidate overlay parent: {error}"),
            )
        })?;
        let canonical_worktree = worktree.canonicalize().map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot resolve candidate worktree: {error}"),
            )
        })?;
        if !canonical_parent.starts_with(canonical_worktree)
            || std::fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "candidate overlay path escapes through a symlink",
            ));
        }
        std::fs::write(path, source.as_bytes()).map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot materialize candidate overlay: {error}"),
            )
        })?;
    }
    Ok(())
}

#[derive(Debug)]
struct TestArtifactObservation {
    artifact_hash: String,
    executed_test_count: usize,
    selected_outcomes: Vec<TestcaseOutcome>,
}

fn require_differential_outcomes(
    candidate_compile_succeeded: bool,
    candidate_lifecycle_succeeded: bool,
    candidate_outcomes: &[TestcaseOutcome],
    omission_compile_succeeded: bool,
    omission_lifecycle_succeeded: bool,
    omission_outcomes: &[TestcaseOutcome],
) -> Result<(), ClewError> {
    if !candidate_compile_succeeded || !omission_compile_succeeded {
        return Err(ClewError::new(
            ErrorCode::CompileFailed,
            "candidate and omission must both compile before differential oracle evaluation",
        ));
    }
    if !candidate_lifecycle_succeeded || candidate_outcomes != [TestcaseOutcome::Passed] {
        return Err(ClewError::new(
            ErrorCode::TestFailed,
            "candidate did not pass the exact compiler-linked behavioral test",
        ));
    }
    if omission_lifecycle_succeeded
        || !matches!(
            omission_outcomes,
            [TestcaseOutcome::Failed] | [TestcaseOutcome::Error]
        )
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "omission mutant did not fail the exact compiler-linked behavioral test",
        ));
    }
    Ok(())
}

fn reject_unrelated_test_failures(
    testcases: &[TestcaseRecord],
    selected_records: &BTreeSet<usize>,
) -> Result<(), ClewError> {
    if testcases.iter().enumerate().any(|(index, testcase)| {
        !selected_records.contains(&index)
            && matches!(
                testcase.outcome,
                TestcaseOutcome::Failed | TestcaseOutcome::Error
            )
    }) {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "validation lifecycle contains an unrelated failing testcase",
        ));
    }
    Ok(())
}

fn test_artifact(
    repo: &Path,
    expected: &[&VerifiedBehavioralTest],
    route: &ValidationRoute,
) -> Result<(String, usize), ClewError> {
    let observation = test_artifact_observed(repo, expected, route)?;
    if observation.executed_test_count == 0
        || observation
            .selected_outcomes
            .iter()
            .any(|outcome| *outcome != TestcaseOutcome::Passed)
    {
        let outcome = observation
            .selected_outcomes
            .first()
            .map(|outcome| outcome.as_str())
            .unwrap_or("MISSING");
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            format!("compiler-linked behavioral test did not pass: {outcome}"),
        ));
    }
    Ok((observation.artifact_hash, observation.executed_test_count))
}

fn test_artifact_observed(
    repo: &Path,
    expected: &[&VerifiedBehavioralTest],
    route: &ValidationRoute,
) -> Result<TestArtifactObservation, ClewError> {
    if expected.iter().any(|test| {
        test.validation_route != *route
            || test.class_name != route.test_binary_class
            || test.test_name != route.test_method
    }) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "validation report route is not bound to the exact compiler-linked test identity",
        ));
    }
    if route.report_format != "JUNIT_XML" {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "authority only accepts JUnit XML validation artifacts",
        ));
    }
    let relative = safe_relative_path(route.report_root.to_string_lossy().as_ref())?;
    let result_root = repo.join(relative);
    let result_metadata = std::fs::symlink_metadata(&result_root).map_err(|error| {
        ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            format!("validation report root was not recreated by this invocation: {error}"),
        )
    })?;
    if result_metadata.file_type().is_symlink() || !result_metadata.is_dir() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "validation report root is not a regular contained directory",
        ));
    }
    let canonical_repo = repo.canonicalize().map_err(|error| {
        ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            format!("cannot resolve validation repository: {error}"),
        )
    })?;
    let canonical_result_root = result_root.canonicalize().map_err(|error| {
        ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            format!("cannot resolve validation report root: {error}"),
        )
    })?;
    if !canonical_result_root.starts_with(&canonical_repo) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "validation report root escapes the authorized repository",
        ));
    }
    let mut reports = Vec::new();
    let mut executed = 0usize;
    let mut testcases = Vec::new();
    for entry in WalkDir::new(&canonical_result_root).follow_links(false) {
        let entry = entry.map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot inspect validation reports: {error}"),
            )
        })?;
        if entry.file_type().is_symlink() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "validation report tree contains a symlink",
            ));
        }
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("xml")
        {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot read validation report: {error}"),
            )
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("validation report is not UTF-8 XML: {error}"),
            )
        })?;
        let parsed = testcase_records(text)?;
        executed += parsed
            .iter()
            .filter(|testcase| testcase.outcome != TestcaseOutcome::Skipped)
            .count();
        testcases.extend(parsed);
        reports.push((
            entry
                .path()
                .strip_prefix(repo)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/"),
            canonical::hash_bytes(&bytes),
        ));
    }
    reports.sort();
    let mut selected_outcomes = Vec::new();
    let mut selected_records = BTreeSet::new();
    for test in expected {
        let escaped_class = xml_escape(&test.class_name);
        let escaped_plain_name = xml_escape(&test.test_name);
        let escaped_kotlin_name = format!("{escaped_plain_name}()");
        let matched = testcases
            .iter()
            .enumerate()
            .filter(|testcase| {
                testcase.1.class_name == escaped_class
                    && (testcase.1.name == escaped_plain_name
                        || testcase.1.name == escaped_kotlin_name)
            })
            .collect::<Vec<_>>();
        if matched.len() != 1 {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "validation lifecycle did not produce exactly one result for the compiler-linked behavioral test",
            ));
        }
        selected_records.insert(matched[0].0);
        selected_outcomes.push(matched[0].1.outcome);
    }
    reject_unrelated_test_failures(&testcases, &selected_records)?;
    Ok(TestArtifactObservation {
        artifact_hash: canonical::hash(&reports).map_err(internal)?,
        executed_test_count: executed,
        selected_outcomes,
    })
}

fn prepare_validation_report_root(
    worktree: &Path,
    route: &ValidationRoute,
) -> Result<(), ClewError> {
    let canonical_worktree = worktree
        .canonicalize()
        .map_err(|error| invalid_source(format!("cannot resolve validation worktree: {error}")))?;
    let relative = safe_relative_path(route.report_root.to_string_lossy().as_ref())?;
    let mut current = canonical_worktree.clone();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err(invalid_source("validation report route is not contained"));
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(ClewError::new(
                        ErrorCode::InvalidInput,
                        "validation report route crosses a symlink or non-directory",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(ClewError::new(
                    ErrorCode::IncompleteSemanticAnalysis,
                    format!("cannot inspect validation report route: {error}"),
                ));
            }
        }
    }
    let report_root = canonical_worktree.join(&relative);
    if let Ok(metadata) = std::fs::symlink_metadata(&report_root) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "pre-existing validation report root is unsafe",
            ));
        }
        std::fs::remove_dir_all(&report_root).map_err(|error| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot clear isolated validation report root: {error}"),
            )
        })?;
    }
    if report_root.exists() {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "isolated validation report root still exists before invocation",
        ));
    }
    Ok(())
}

fn evidence_git(repo: &Path, args: &[&str]) -> Result<(), ClewError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| {
            ClewError::new(
                ErrorCode::Internal,
                format!("cannot start git for validation isolation: {error}"),
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::Internal,
            format!(
                "git validation isolation command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestcaseOutcome {
    Passed,
    Failed,
    Error,
    Skipped,
}

impl TestcaseOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "PASSED",
            Self::Failed => "FAILED",
            Self::Error => "ERROR",
            Self::Skipped => "SKIPPED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestcaseRecord {
    class_name: String,
    name: String,
    outcome: TestcaseOutcome,
}

fn testcase_records(text: &str) -> Result<Vec<TestcaseRecord>, ClewError> {
    let mut records = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find('<') {
        let start = cursor + relative;
        if text[start..].starts_with("<![CDATA[") {
            let Some(end) = text[start + 9..].find("]]>") else {
                break;
            };
            cursor = start + 9 + end + 3;
            continue;
        }
        if text[start..].starts_with("<!--") {
            let end = text[start + 4..].find("-->").ok_or_else(|| {
                invalid_source("validation report contains an unterminated XML comment")
            })?;
            cursor = start + 4 + end + 3;
            continue;
        }
        let end = xml_tag_end(text, start)?;
        let tag = &text[start..end];
        if tag.starts_with("<testcase")
            && tag
                .as_bytes()
                .get("<testcase".len())
                .is_some_and(u8::is_ascii_whitespace)
        {
            let attributes = xml_attributes(tag)?;
            let class_name = attributes
                .get("classname")
                .cloned()
                .ok_or_else(|| invalid_source("validation testcase has no classname attribute"))?;
            let name = attributes
                .get("name")
                .cloned()
                .ok_or_else(|| invalid_source("validation testcase has no name attribute"))?;
            let self_closing = tag.trim_end_matches('>').trim_end().ends_with('/');
            let (body, next_cursor) = if self_closing {
                ("", end)
            } else {
                let relative_close = text[end..]
                    .find("</testcase>")
                    .ok_or_else(|| invalid_source("validation testcase has no closing element"))?;
                let close = end + relative_close;
                (&text[end..close], close + "</testcase>".len())
            };
            records.push(TestcaseRecord {
                class_name,
                name,
                outcome: testcase_outcome(&attributes, body)?,
            });
            cursor = next_cursor;
            continue;
        }
        cursor = end;
    }
    Ok(records)
}

fn xml_tag_end(text: &str, start: usize) -> Result<usize, ClewError> {
    let mut quote = None;
    for (offset, character) in text[start..].char_indices() {
        match character {
            '\'' | '"' if quote == Some(character) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(character),
            '>' if quote.is_none() => return Ok(start + offset + 1),
            _ => {}
        }
    }
    Err(invalid_source(
        "validation report contains an unterminated XML tag",
    ))
}

fn xml_attributes(tag: &str) -> Result<BTreeMap<String, String>, ClewError> {
    let mut attributes = BTreeMap::new();
    let mut rest = tag
        .strip_prefix("<testcase")
        .ok_or_else(|| invalid_source("invalid validation testcase element"))?
        .trim_start();
    while !rest.starts_with('>') && !rest.starts_with("/>") {
        let name_end = rest
            .find(|character: char| character.is_ascii_whitespace() || character == '=')
            .ok_or_else(|| invalid_source("malformed validation testcase attribute"))?;
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        rest = rest
            .strip_prefix('=')
            .ok_or_else(|| invalid_source("validation testcase attribute has no value"))?
            .trim_start();
        let quote = rest
            .chars()
            .next()
            .filter(|character| matches!(character, '\'' | '"'))
            .ok_or_else(|| invalid_source("validation testcase attribute is not quoted"))?;
        rest = &rest[quote.len_utf8()..];
        let value_end = rest
            .find(quote)
            .ok_or_else(|| invalid_source("unterminated validation testcase attribute"))?;
        if attributes
            .insert(name.to_owned(), rest[..value_end].to_owned())
            .is_some()
        {
            return Err(invalid_source("duplicate validation testcase attribute"));
        }
        rest = rest[value_end + quote.len_utf8()..].trim_start();
    }
    Ok(attributes)
}

fn testcase_outcome(
    attributes: &BTreeMap<String, String>,
    body: &str,
) -> Result<TestcaseOutcome, ClewError> {
    let mut outcomes = Vec::new();
    for name in ["status", "result"] {
        let Some(value) = attributes.get(name) else {
            continue;
        };
        let outcome = match value.to_ascii_lowercase().as_str() {
            "passed" | "success" | "successful" => TestcaseOutcome::Passed,
            "failed" | "failure" => TestcaseOutcome::Failed,
            "error" => TestcaseOutcome::Error,
            "skipped" | "disabled" | "ignored" | "notrun" | "not-run" => TestcaseOutcome::Skipped,
            _ => {
                return Err(invalid_source(format!(
                    "unknown validation testcase {name} classification"
                )));
            }
        };
        outcomes.push(outcome);
    }
    for (element, outcome) in [
        ("failure", TestcaseOutcome::Failed),
        ("error", TestcaseOutcome::Error),
        ("skipped", TestcaseOutcome::Skipped),
    ] {
        if xml_has_start_element(body, element)? {
            outcomes.push(outcome);
        }
    }
    outcomes.sort_by_key(|outcome| outcome.as_str());
    outcomes.dedup();
    match outcomes.as_slice() {
        [] => Ok(TestcaseOutcome::Passed),
        [outcome] => Ok(*outcome),
        _ => Err(invalid_source(
            "validation testcase contains conflicting outcome classifications",
        )),
    }
}

fn xml_has_start_element(text: &str, element: &str) -> Result<bool, ClewError> {
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find('<') {
        let start = cursor + relative;
        if text[start..].starts_with("<![CDATA[") {
            let end = text[start + 9..]
                .find("]]>")
                .ok_or_else(|| invalid_source("unterminated CDATA in validation report"))?;
            cursor = start + 9 + end + 3;
            continue;
        }
        if text[start..].starts_with("<!--") {
            let end = text[start + 4..]
                .find("-->")
                .ok_or_else(|| invalid_source("unterminated comment in validation report"))?;
            cursor = start + 4 + end + 3;
            continue;
        }
        let end = xml_tag_end(text, start)?;
        let tag = &text[start + 1..end - 1];
        let candidate = tag.trim_start();
        if candidate.starts_with(element)
            && candidate
                .as_bytes()
                .get(element.len())
                .is_some_and(|character| character.is_ascii_whitespace() || *character == b'/')
        {
            return Ok(true);
        }
        cursor = end;
    }
    Ok(false)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn safe_relative_path(value: &str) -> Result<PathBuf, ClewError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid_source("source path is not a safe relative path"));
    }
    Ok(path.to_owned())
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_source(format!("source origin has no {field}")))
}

fn git_head(repo: &Path) -> Result<String, ClewError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|error| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("cannot start git for evidence authority: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "evidence repository has no readable Git HEAD",
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(internal)
}

fn ensure_repository_root(repo: &Path) -> Result<(), ClewError> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(repo)
        .output()
        .map_err(|error| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("cannot locate git root for semantic commit: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "semantic commit requires a readable Git repository root",
        ));
    }
    let root = String::from_utf8(output.stdout)
        .map_err(internal)
        .and_then(|path| {
            Path::new(path.trim())
                .canonicalize()
                .map_err(|error| internal(error.to_string()))
        })?;
    let repo = repo
        .canonicalize()
        .map_err(|error| internal(error.to_string()))?;
    if root != repo {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "semantic commit currently requires --repo to be the Git worktree root",
        ));
    }
    Ok(())
}

fn ensure_clean_checkout(repo: &Path) -> Result<(), ClewError> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all", "--", "."])
        .current_dir(repo)
        .output()
        .map_err(|error| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("cannot inspect authority checkout: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "cannot inspect authority checkout state",
        ));
    }
    if output.stdout.is_empty() {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "authority requires a clean repository subtree bound to Git HEAD",
        ))
    }
}

fn wrong_session(kind: &str) -> ClewError {
    ClewError::new(
        ErrorCode::PreconditionFailed,
        format!("{kind} receipt was issued by another authority session"),
    )
}

fn invalid_receipt(kind: &str) -> ClewError {
    ClewError::new(
        ErrorCode::PreconditionFailed,
        format!("unknown {kind} receipt"),
    )
}

fn invalid_source(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::IncompleteSemanticAnalysis, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const TEST_EXTERNAL_SPEC_ISSUER: &str = "codeclew-test-issuer";

    struct SignedExternalSpecFixture {
        _temporary: tempfile::TempDir,
        repo: PathBuf,
        specification_path: PathBuf,
        manifest_path: PathBuf,
        request: TypedGoalBindingRequest,
        payload: ExternalSpecPayload,
        signing_key: SigningKey,
    }

    fn signed_external_spec_fixture(worker: &mut WorkerClient) -> SignedExternalSpecFixture {
        let temporary = tempfile::tempdir().unwrap();
        let checkout = temporary.path().join("checkout");
        let clone = Command::new("git")
            .args(["clone", "--quiet", "--no-hardlinks"])
            .arg(crate::worker::workspace_root())
            .arg(&checkout)
            .output()
            .unwrap();
        assert!(
            clone.status.success(),
            "{}",
            String::from_utf8_lossy(&clone.stderr)
        );
        for args in [
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "Codeclew Test"],
            vec![
                "rm",
                "--quiet",
                "fixtures/kotlin-2-1/src/main/kotlin/com/acme/RelationFacts.kt",
            ],
            vec!["commit", "--quiet", "-m", "isolate signed fixture"],
        ] {
            let output = Command::new("git")
                .current_dir(&checkout)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let repo = checkout.join("fixtures/kotlin-2-1");
        crate::worker::seed_test_build_caches(&repo);
        let revision = git_head(&repo).unwrap();
        let goal = TypedSemanticGoal::new(
            &revision,
            [
                ("context".into(), TypedVariableDomain::Callable),
                ("transform".into(), TypedVariableDomain::Callable),
                ("edge".into(), TypedVariableDomain::ValueEdge),
            ],
            [OperatorApplication {
                operator: PrimitiveConstraint::MapEdge,
                operands: vec!["context".into(), "transform".into(), "edge".into()],
            }],
        );
        let request = TypedGoalBindingRequest {
            schema: TYPED_GOAL_BINDING_REQUEST_SCHEMA.into(),
            goal,
            hints: vec![],
            compilation: Some(":/main".into()),
        };
        let project = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":repo,"compilation":":/main"}),
            )
            .unwrap();
        require_exact_project_compilation(&project, ":/main").unwrap();
        let source_snapshot_sha256 = tracked_source_snapshot(&repo, &[]).unwrap();
        let task =
            "Apply the unique compatible transformation while preserving the declared constraints.";
        let repository = "checkout/fixtures/kotlin-2-1";
        let manifest_path = temporary.path().join("task-manifest.json");
        let manifest = json!({
            "schema":"semantic-editing-public-task/0.1",
            "taskId":"opaque-task",
            "task":task,
            "repository":repository,
            "sourceSnapshotSha256":source_snapshot_sha256,
        });
        std::fs::write(&manifest_path, canonical::bytes(&manifest).unwrap()).unwrap();
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let mut payload = ExternalSpecPayload {
            schema: EXTERNAL_SPEC_PAYLOAD_SCHEMA.into(),
            issuer: TEST_EXTERNAL_SPEC_ISSUER.into(),
            task: task.into(),
            task_digest: canonical::hash(&task).unwrap(),
            public_manifest: "task-manifest.json".into(),
            public_manifest_digest: canonical::hash(&manifest).unwrap(),
            package_digest: String::new(),
            repository: repository.into(),
            repository_revision: revision,
            source_snapshot_sha256,
            request_digest: canonical::hash(&request).unwrap(),
            compilation: ":/main".into(),
            project_model_hash: required_str(&project, "projectModelHash").unwrap().into(),
        };
        payload.package_digest = external_spec_package_digest(&payload).unwrap();
        let specification_path = temporary.path().join("signed-external-spec.json");
        write_test_signed_external_spec(&specification_path, &payload, &signing_key);
        SignedExternalSpecFixture {
            _temporary: temporary,
            repo,
            specification_path,
            manifest_path,
            request,
            payload,
            signing_key,
        }
    }

    fn write_test_signed_external_spec(
        path: &Path,
        payload: &ExternalSpecPayload,
        signing_key: &SigningKey,
    ) {
        let signature = signing_key.sign(&canonical::bytes(payload).unwrap());
        let envelope = SignedExternalSpecEnvelope {
            schema: SIGNED_EXTERNAL_SPEC_SCHEMA.into(),
            payload: payload.clone(),
            signature: hex::encode(signature.to_bytes()),
        };
        std::fs::write(path, canonical::bytes(&envelope).unwrap()).unwrap();
    }

    fn declaration_type_spec_fixture(
        worker: &mut WorkerClient,
        source: &str,
    ) -> SignedExternalSpecFixture {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repository");
        std::fs::create_dir_all(repo.join("src/main/kotlin/example")).unwrap();
        std::fs::create_dir_all(repo.join("gradle/wrapper")).unwrap();
        let workspace = crate::worker::workspace_root();
        for relative in [
            "gradlew",
            "gradlew.bat",
            "gradle/wrapper/gradle-wrapper.jar",
            "gradle/wrapper/gradle-wrapper.properties",
        ] {
            std::fs::copy(workspace.join(relative), repo.join(relative)).unwrap();
        }
        std::fs::write(
            repo.join("settings.gradle.kts"),
            "pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }\nrootProject.name = \"declaration-facts\"\n",
        )
        .unwrap();
        std::fs::write(
            repo.join("build.gradle.kts"),
            "plugins { kotlin(\"jvm\") version \"2.4.10\" }\nrepositories { mavenCentral() }\n",
        )
        .unwrap();
        std::fs::write(
            repo.join(".gitignore"),
            ".gradle/\nbuild/\n.semantic-thread/\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/main/kotlin/example/Declarations.kt"), source).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "Codeclew Test"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "fixture"],
        ] {
            let output = Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        crate::worker::seed_test_build_caches(&repo);
        let revision = git_head(&repo).unwrap();
        let goal = TypedSemanticGoal::new(
            &revision,
            [
                ("origin".into(), TypedVariableDomain::Declaration),
                ("contract".into(), TypedVariableDomain::Declaration),
            ],
            [OperatorApplication {
                operator: PrimitiveConstraint::PropagateDeclaredType,
                operands: vec!["origin".into(), "contract".into()],
            }],
        );
        let request = TypedGoalBindingRequest {
            schema: TYPED_GOAL_BINDING_REQUEST_SCHEMA.into(),
            goal,
            hints: vec!["advisory text that cannot select a declaration".into()],
            compilation: Some(":/main".into()),
        };
        let project = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":repo,"compilation":":/main"}),
            )
            .unwrap();
        require_exact_project_compilation(&project, ":/main").unwrap();
        let source_snapshot_sha256 = tracked_source_snapshot(&repo, &[]).unwrap();
        let task = "Change one internal declared type while preserving all compiler-known overrides and uses.";
        let manifest_path = temporary.path().join("task-manifest.json");
        let manifest = json!({
            "schema":"semantic-editing-public-task/0.1",
            "taskId":"opaque-declaration-task",
            "task":task,
            "repository":"repository",
            "sourceSnapshotSha256":source_snapshot_sha256,
        });
        std::fs::write(&manifest_path, canonical::bytes(&manifest).unwrap()).unwrap();
        let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
        let mut payload = ExternalSpecPayload {
            schema: EXTERNAL_SPEC_PAYLOAD_SCHEMA.into(),
            issuer: TEST_EXTERNAL_SPEC_ISSUER.into(),
            task: task.into(),
            task_digest: canonical::hash(&task).unwrap(),
            public_manifest: "task-manifest.json".into(),
            public_manifest_digest: canonical::hash(&manifest).unwrap(),
            package_digest: String::new(),
            repository: "repository".into(),
            repository_revision: revision,
            source_snapshot_sha256,
            request_digest: canonical::hash(&request).unwrap(),
            compilation: ":/main".into(),
            project_model_hash: required_str(&project, "projectModelHash").unwrap().into(),
        };
        payload.package_digest = external_spec_package_digest(&payload).unwrap();
        let specification_path = temporary.path().join("signed-external-spec.json");
        write_test_signed_external_spec(&specification_path, &payload, &signing_key);
        SignedExternalSpecFixture {
            _temporary: temporary,
            repo,
            specification_path,
            manifest_path,
            request,
            payload,
            signing_key,
        }
    }

    fn declaration_type_maven_spec_fixture(
        worker: &mut WorkerClient,
        source: &str,
    ) -> SignedExternalSpecFixture {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repository");
        std::fs::create_dir_all(repo.join("src/main/kotlin/example")).unwrap();
        let template = crate::worker::workspace_root().join("fixtures/kotlin-maven");
        for relative in ["pom.xml", "mvnw", ".gitignore"] {
            std::fs::copy(template.join(relative), repo.join(relative)).unwrap();
        }
        std::fs::write(repo.join("src/main/kotlin/example/Declarations.kt"), source).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "Codeclew Test"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "fixture"],
        ] {
            let output = Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        crate::worker::seed_test_build_caches(&repo);
        let revision = git_head(&repo).unwrap();
        let request = TypedGoalBindingRequest {
            schema: TYPED_GOAL_BINDING_REQUEST_SCHEMA.into(),
            goal: TypedSemanticGoal::new(
                &revision,
                [
                    ("origin".into(), TypedVariableDomain::Declaration),
                    ("contract".into(), TypedVariableDomain::Declaration),
                ],
                [OperatorApplication {
                    operator: PrimitiveConstraint::PropagateDeclaredType,
                    operands: vec!["origin".into(), "contract".into()],
                }],
            ),
            hints: vec![],
            compilation: Some(":/main".into()),
        };
        let project = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":repo,"compilation":":/main"}),
            )
            .unwrap();
        require_exact_project_compilation(&project, ":/main").unwrap();
        assert_eq!(project["buildSystem"], "MAVEN");
        let source_snapshot_sha256 = tracked_source_snapshot(&repo, &[]).unwrap();
        let task = "Change one internal declared type while preserving all compiler-known overrides and uses.";
        let manifest_path = temporary.path().join("task-manifest.json");
        let manifest = json!({
            "schema":"semantic-editing-public-task/0.1",
            "taskId":"opaque-declaration-task",
            "task":task,
            "repository":"repository",
            "sourceSnapshotSha256":source_snapshot_sha256,
        });
        std::fs::write(&manifest_path, canonical::bytes(&manifest).unwrap()).unwrap();
        let signing_key = SigningKey::from_bytes(&[13_u8; 32]);
        let mut payload = ExternalSpecPayload {
            schema: EXTERNAL_SPEC_PAYLOAD_SCHEMA.into(),
            issuer: TEST_EXTERNAL_SPEC_ISSUER.into(),
            task: task.into(),
            task_digest: canonical::hash(&task).unwrap(),
            public_manifest: "task-manifest.json".into(),
            public_manifest_digest: canonical::hash(&manifest).unwrap(),
            package_digest: String::new(),
            repository: "repository".into(),
            repository_revision: revision,
            source_snapshot_sha256,
            request_digest: canonical::hash(&request).unwrap(),
            compilation: ":/main".into(),
            project_model_hash: required_str(&project, "projectModelHash").unwrap().into(),
        };
        payload.package_digest = external_spec_package_digest(&payload).unwrap();
        let specification_path = temporary.path().join("signed-external-spec.json");
        write_test_signed_external_spec(&specification_path, &payload, &signing_key);
        SignedExternalSpecFixture {
            _temporary: temporary,
            repo,
            specification_path,
            manifest_path,
            request,
            payload,
            signing_key,
        }
    }

    fn nullable_construction_spec_fixture(
        worker: &mut WorkerClient,
        source: &str,
        maven: bool,
        hints: Vec<String>,
    ) -> SignedExternalSpecFixture {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repository");
        std::fs::create_dir_all(repo.join("src/main/kotlin/neutral")).unwrap();
        let workspace = crate::worker::workspace_root();
        if maven {
            let template = workspace.join("fixtures/kotlin-maven");
            for relative in ["pom.xml", "mvnw", ".gitignore"] {
                std::fs::copy(template.join(relative), repo.join(relative)).unwrap();
            }
        } else {
            std::fs::create_dir_all(repo.join("gradle/wrapper")).unwrap();
            for relative in [
                "gradlew",
                "gradlew.bat",
                "gradle/wrapper/gradle-wrapper.jar",
                "gradle/wrapper/gradle-wrapper.properties",
            ] {
                std::fs::copy(workspace.join(relative), repo.join(relative)).unwrap();
            }
            std::fs::write(
                repo.join("settings.gradle.kts"),
                "pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }\nrootProject.name = \"nullable-facts\"\n",
            )
            .unwrap();
            std::fs::write(
                repo.join("build.gradle.kts"),
                "plugins { kotlin(\"jvm\") version \"2.4.10\" }\nrepositories { mavenCentral() }\n",
            )
            .unwrap();
            std::fs::write(
                repo.join(".gitignore"),
                ".gradle/\nbuild/\n.semantic-thread/\n",
            )
            .unwrap();
        }
        std::fs::write(repo.join("src/main/kotlin/neutral/Construction.kt"), source).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "Codeclew Test"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "fixture"],
        ] {
            let output = Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        crate::worker::seed_test_build_caches(&repo);
        let revision = git_head(&repo).unwrap();
        let request = TypedGoalBindingRequest {
            schema: TYPED_GOAL_BINDING_REQUEST_SCHEMA.into(),
            goal: TypedSemanticGoal::new(
                &revision,
                [
                    ("nullable".into(), TypedVariableDomain::Declaration),
                    ("fallback".into(), TypedVariableDomain::Declaration),
                    ("destination".into(), TypedVariableDomain::Declaration),
                ],
                [OperatorApplication {
                    operator: PrimitiveConstraint::NullHandles,
                    operands: vec!["nullable".into(), "fallback".into(), "destination".into()],
                }],
            ),
            hints,
            compilation: Some(":/main".into()),
        };
        let project = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":repo,"compilation":":/main"}),
            )
            .unwrap();
        require_exact_project_compilation(&project, ":/main").unwrap();
        assert_eq!(
            project["buildSystem"],
            if maven { "MAVEN" } else { "GRADLE" }
        );
        let source_snapshot_sha256 = tracked_source_snapshot(&repo, &[]).unwrap();
        let task = "Preserve the compiler-proven nullable construction behavior.";
        let manifest_path = temporary.path().join("task-manifest.json");
        let manifest = json!({
            "schema":"semantic-editing-public-task/0.1",
            "taskId":"opaque-nullable-task",
            "task":task,
            "repository":"repository",
            "sourceSnapshotSha256":source_snapshot_sha256,
        });
        std::fs::write(&manifest_path, canonical::bytes(&manifest).unwrap()).unwrap();
        let signing_key = SigningKey::from_bytes(&[17_u8; 32]);
        let mut payload = ExternalSpecPayload {
            schema: EXTERNAL_SPEC_PAYLOAD_SCHEMA.into(),
            issuer: TEST_EXTERNAL_SPEC_ISSUER.into(),
            task: task.into(),
            task_digest: canonical::hash(&task).unwrap(),
            public_manifest: "task-manifest.json".into(),
            public_manifest_digest: canonical::hash(&manifest).unwrap(),
            package_digest: String::new(),
            repository: "repository".into(),
            repository_revision: revision,
            source_snapshot_sha256,
            request_digest: canonical::hash(&request).unwrap(),
            compilation: ":/main".into(),
            project_model_hash: required_str(&project, "projectModelHash").unwrap().into(),
        };
        payload.package_digest = external_spec_package_digest(&payload).unwrap();
        let specification_path = temporary.path().join("signed-external-spec.json");
        write_test_signed_external_spec(&specification_path, &payload, &signing_key);
        SignedExternalSpecFixture {
            _temporary: temporary,
            repo,
            specification_path,
            manifest_path,
            request,
            payload,
            signing_key,
        }
    }

    fn projection_consumer_spec_fixture(
        worker: &mut WorkerClient,
        source: &str,
        maven: bool,
        hints: Vec<String>,
    ) -> SignedExternalSpecFixture {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repository");
        std::fs::create_dir_all(repo.join("src/main/kotlin/neutral")).unwrap();
        let workspace = crate::worker::workspace_root();
        if maven {
            let template = workspace.join("fixtures/kotlin-maven");
            for relative in ["pom.xml", "mvnw", ".gitignore"] {
                std::fs::copy(template.join(relative), repo.join(relative)).unwrap();
            }
        } else {
            std::fs::create_dir_all(repo.join("gradle/wrapper")).unwrap();
            for relative in [
                "gradlew",
                "gradlew.bat",
                "gradle/wrapper/gradle-wrapper.jar",
                "gradle/wrapper/gradle-wrapper.properties",
            ] {
                std::fs::copy(workspace.join(relative), repo.join(relative)).unwrap();
            }
            std::fs::write(
                repo.join("settings.gradle.kts"),
                "pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }\nrootProject.name = \"projection-facts\"\n",
            )
            .unwrap();
            std::fs::write(
                repo.join("build.gradle.kts"),
                "plugins { kotlin(\"jvm\") version \"2.4.10\" }\nrepositories { mavenCentral() }\n",
            )
            .unwrap();
            std::fs::write(
                repo.join(".gitignore"),
                ".gradle/\nbuild/\n.semantic-thread/\n",
            )
            .unwrap();
        }
        std::fs::write(repo.join("src/main/kotlin/neutral/Projection.kt"), source).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "Codeclew Test"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "fixture"],
        ] {
            let output = Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        crate::worker::seed_test_build_caches(&repo);
        let revision = git_head(&repo).unwrap();
        let request = TypedGoalBindingRequest {
            schema: TYPED_GOAL_BINDING_REQUEST_SCHEMA.into(),
            goal: TypedSemanticGoal::new(
                &revision,
                [
                    ("source".into(), TypedVariableDomain::Declaration),
                    ("projection".into(), TypedVariableDomain::Declaration),
                    ("consumer".into(), TypedVariableDomain::Declaration),
                ],
                [OperatorApplication {
                    operator: PrimitiveConstraint::ProjectsValue,
                    operands: vec!["source".into(), "projection".into(), "consumer".into()],
                }],
            ),
            hints,
            compilation: Some(":/main".into()),
        };
        let project = worker
            .request(
                RequestKind::OpenProject,
                &json!({"repo":repo,"compilation":":/main"}),
            )
            .unwrap();
        require_exact_project_compilation(&project, ":/main").unwrap();
        assert_eq!(
            project["buildSystem"],
            if maven { "MAVEN" } else { "GRADLE" }
        );
        let source_snapshot_sha256 = tracked_source_snapshot(&repo, &[]).unwrap();
        let task = "Preserve one compiler-proven returned value through its exact consumer slot.";
        let manifest_path = temporary.path().join("task-manifest.json");
        let manifest = json!({
            "schema":"semantic-editing-public-task/0.1",
            "taskId":"opaque-projection-task",
            "task":task,
            "repository":"repository",
            "sourceSnapshotSha256":source_snapshot_sha256,
        });
        std::fs::write(&manifest_path, canonical::bytes(&manifest).unwrap()).unwrap();
        let signing_key = SigningKey::from_bytes(&[19_u8; 32]);
        let mut payload = ExternalSpecPayload {
            schema: EXTERNAL_SPEC_PAYLOAD_SCHEMA.into(),
            issuer: TEST_EXTERNAL_SPEC_ISSUER.into(),
            task: task.into(),
            task_digest: canonical::hash(&task).unwrap(),
            public_manifest: "task-manifest.json".into(),
            public_manifest_digest: canonical::hash(&manifest).unwrap(),
            package_digest: String::new(),
            repository: "repository".into(),
            repository_revision: revision,
            source_snapshot_sha256,
            request_digest: canonical::hash(&request).unwrap(),
            compilation: ":/main".into(),
            project_model_hash: required_str(&project, "projectModelHash").unwrap().into(),
        };
        payload.package_digest = external_spec_package_digest(&payload).unwrap();
        let specification_path = temporary.path().join("signed-external-spec.json");
        write_test_signed_external_spec(&specification_path, &payload, &signing_key);
        SignedExternalSpecFixture {
            _temporary: temporary,
            repo,
            specification_path,
            manifest_path,
            request,
            payload,
            signing_key,
        }
    }

    fn bind_declaration_fixture(
        fixture: &SignedExternalSpecFixture,
        worker: &mut WorkerClient,
    ) -> TypedGoalBindingDecision {
        let mut authority =
            EvidenceAuthority::open(&fixture.repo, &fixture.payload.repository_revision).unwrap();
        let receipt = authority
            .issue_external_spec_with_verifier(
                &fixture.specification_path,
                &fixture.request,
                Some(":/main"),
                TEST_EXTERNAL_SPEC_ISSUER,
                fixture.signing_key.verifying_key().to_bytes(),
                worker,
            )
            .unwrap();
        authority
            .bind_typed_goal_with_external_spec(&fixture.request, Some(":/main"), &receipt, worker)
            .unwrap()
    }

    #[cfg(any())]
    #[test]
    fn nullable_construction_provider_gradle_maven_and_negative_group() {
        fn proof_shape(
            summary: &TypedGoalProofSummary,
        ) -> Vec<(PrimitiveConstraint, EvidenceRelation)> {
            summary
                .evidence_relations
                .iter()
                .map(|record| (record.operator.operator.clone(), record.relation))
                .collect()
        }
        let positive_source = r#"
package neutral.nulls
internal data class Vessel(val first: String, val second: String)
internal fun nullableValue(enabled: Boolean): String? = if (enabled) "value" else null
internal fun fallbackValue(): String = "fallback"
internal fun fallbackDecoy(): String = "decoy"
internal fun assemble(enabled: Boolean, sameTypeDecoy: String): Vessel =
    Vessel(second = nullableValue(enabled) ?: fallbackValue(), first = sameTypeDecoy)
"#;
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let gradle = nullable_construction_spec_fixture(
            &mut worker,
            positive_source,
            false,
            vec!["advisory hint must not select symbols".into()],
        );
        let gradle_decision = bind_declaration_fixture(&gradle, &mut worker);
        let TypedGoalBindingDecision::Bound(gradle_bound) = gradle_decision else {
            panic!("unique Gradle nullable construction must bind: {gradle_decision:#?}")
        };
        assert_eq!(gradle_bound.summary().bindings.len(), 3);
        assert!(
            gradle_bound
                .summary()
                .bindings
                .values()
                .all(|value| value.contains("#jvm:"))
        );
        let gradle_candidates =
            discover_nullable_construction_candidates(&gradle.repo, Some(":/main"), &mut worker)
                .unwrap();
        let [gradle_candidate] = gradle_candidates.as_slice() else {
            panic!("one nullable candidate expected for boundary closure assertions")
        };
        assert!(gradle_candidate.occurrences.iter().all(|occurrence| {
            occurrence
                .boundary_closure_fingerprint
                .starts_with("sha256:")
        }));
        let boundary_plan = gradle.request.goal.execution_plan().unwrap();
        let mandatory = mandatory_relations_for_plan(&boundary_plan);
        let roots = BTreeSet::from([
            gradle_candidate.source_callable.clone(),
            gradle_candidate.fallback_callable.clone(),
            gradle_candidate.destination_callable.clone(),
            gradle_candidate.occurrences[0].owner_callable.clone(),
        ]);
        let descriptor_graph = json!({
            "coverage":"COMPLETE_SUPPORTED_SUBSET",
            "boundaries":[],
        });
        let return_graph = json!({
            "coverage":"PARTIAL",
            "boundaries":[{
                "provider":"K2_FIR",
                "stage":"RETURN_VALUE",
                "code":"IMPLICIT_RETURN_UNSUPPORTED",
                "owner":gradle_candidate.occurrences[0].owner_callable,
            }],
        });
        let excluded = evaluate_obligation_relative_boundaries(
            &[
                ("RELATION", &return_graph),
                ("DESCRIPTOR", &descriptor_graph),
            ],
            &mandatory,
            &roots,
        )
        .unwrap();
        assert_eq!(excluded.excluded_by_relation_count, 1);
        assert!(excluded.fingerprint.starts_with("sha256:"));
        for relevant_boundary in [
            json!({
                "provider":"K2_FIR",
                "stage":"NULL_POLICY",
                "code":"MISSING_NULL_POLICY_OCCURRENCE",
                "owner":gradle_candidate.occurrences[0].owner_callable,
            }),
            json!({
                "provider":"K2_FIR",
                "stage":"CONSTRUCTOR_DECLARATION",
                "code":"UNRESOLVED_CONSTRUCTOR_DESCRIPTOR",
                "target":gradle_candidate.destination_callable,
            }),
        ] {
            let relevant_graph = json!({
                "coverage":"PARTIAL",
                "boundaries":[relevant_boundary],
            });
            assert!(
                evaluate_obligation_relative_boundaries(
                    &[
                        ("RELATION", &relevant_graph),
                        ("DESCRIPTOR", &descriptor_graph)
                    ],
                    &mandatory,
                    &roots,
                )
                .is_err()
            );
        }

        let maven = nullable_construction_spec_fixture(&mut worker, positive_source, true, vec![]);
        let maven_decision = bind_declaration_fixture(&maven, &mut worker);
        let TypedGoalBindingDecision::Bound(maven_bound) = maven_decision else {
            panic!("unique Maven nullable construction must bind: {maven_decision:#?}")
        };
        assert_eq!(
            proof_shape(gradle_bound.summary()),
            proof_shape(maven_bound.summary())
        );
        assert!(proof_shape(gradle_bound.summary()).contains(&(
            PrimitiveConstraint::NullHandles,
            EvidenceRelation::NullHandling,
        )));

        let repeated = nullable_construction_spec_fixture(
            &mut worker,
            &format!(
                "{positive_source}\n{}",
                r#"
internal fun assembleAgain(enabled: Boolean, sameTypeDecoy: String): Vessel =
    Vessel(second = nullableValue(enabled) ?: fallbackValue(), first = sameTypeDecoy)
"#
            ),
            false,
            vec![],
        );
        let repeated_candidates =
            discover_nullable_construction_candidates(&repeated.repo, Some(":/main"), &mut worker)
                .expect("repeated-owner diagnostic discovery");
        let repeated_diagnostic = repeated_candidates
            .iter()
            .flat_map(|candidate| {
                let final_retained_count = candidate.occurrences.len();
                candidate
                    .occurrences
                    .iter()
                    .zip(&candidate.occurrence_fingerprints)
                    .map(move |(occurrence, fingerprint)| {
                        json!({
                            "ownerHash":canonical::hash(&occurrence.owner_callable).unwrap(),
                            "occurrenceFingerprint":fingerprint,
                            "discoveryEntered":true,
                            "descriptorCandidates":1,
                            "constructCandidates":1,
                            "threadBuildStatus":"VERIFIED_REBUILT",
                            "threadIdHash":canonical::hash(&occurrence.thread_id).unwrap(),
                            "threadHash":occurrence.thread_fingerprint,
                            "boundaryStage":"CLEAR_RELEVANT_BOUNDARIES",
                            "coverageStage":"COMPLETE_VERIFIED_RELATION_OWNER",
                            "finalRetainedCount":final_retained_count,
                        })
                    })
            })
            .collect::<Vec<_>>();
        eprintln!(
            "nullable-repeated-owner-diagnostic={}",
            serde_json::to_string(&repeated_diagnostic).unwrap()
        );
        let repeated_decision = bind_declaration_fixture(&repeated, &mut worker);
        let TypedGoalBindingDecision::Bound(repeated_bound) = repeated_decision else {
            panic!(
                "two owners of one declaration triple must bind as one complete occurrence set: {repeated_decision:#?}"
            )
        };
        assert_eq!(repeated_bound.summary().bindings.len(), 3);
        assert!(
            repeated_bound
                .summary()
                .evidence_relations
                .iter()
                .all(|record| {
                    record.occurrence_count == Some(2)
                        && record.occurrence_fingerprints.len() == 2
                        && record
                            .occurrence_set_fingerprint
                            .as_ref()
                            .is_some_and(|fingerprint| {
                                canonical::hash(&record.occurrence_fingerprints)
                                    .is_ok_and(|actual| &actual == fingerprint)
                            })
                })
        );
        let mut missing_occurrence = repeated_bound.summary().clone();
        missing_occurrence.evidence_relations[0]
            .occurrence_fingerprints
            .pop();
        assert!(!missing_occurrence.is_complete_for(&repeated.request.goal));
        let mut stale_second_thread = repeated_bound.summary().clone();
        stale_second_thread.evidence_relations[0].occurrence_fingerprints[1] =
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into();
        assert!(!stale_second_thread.is_complete_for(&repeated.request.goal));

        let alpha = nullable_construction_spec_fixture(
            &mut worker,
            &positive_source
                .replace("Vessel", "Container")
                .replace("nullableValue", "optionalInput")
                .replace("fallbackValue", "defaultInput")
                .replace("assemble", "compose"),
            false,
            vec![],
        );
        let alpha_decision = bind_declaration_fixture(&alpha, &mut worker);
        let TypedGoalBindingDecision::Bound(alpha_bound) = alpha_decision else {
            panic!("alpha-renamed/hint-free construction must bind: {alpha_decision:#?}")
        };
        assert_eq!(
            proof_shape(gradle_bound.summary()),
            proof_shape(alpha_bound.summary())
        );

        let ambiguous = nullable_construction_spec_fixture(
            &mut worker,
            &format!(
                "{}\n{}",
                positive_source,
                positive_source
                    .replace("package neutral.nulls", "")
                    .replace("Vessel", "OtherVessel")
                    .replace("nullableValue", "otherNullable")
                    .replace("fallbackValue", "otherFallback")
                    .replace("fallbackDecoy", "otherDecoy")
                    .replace("assemble", "otherAssemble")
            ),
            false,
            vec![],
        );
        assert!(matches!(
            bind_declaration_fixture(&ambiguous, &mut worker),
            TypedGoalBindingDecision::Ambiguous(_)
        ));

        for source in [
            positive_source.replace("fallbackValue()", "\"literal\""),
            positive_source.replace(
                "nullableValue(enabled) ?: fallbackValue()",
                "nullableValue(enabled)?.trim() ?: fallbackValue()",
            ),
            positive_source
                .replace("internal data class", "data class")
                .replace("internal fun", "fun"),
        ] {
            let fixture = nullable_construction_spec_fixture(&mut worker, &source, false, vec![]);
            assert!(matches!(
                bind_declaration_fixture(&fixture, &mut worker),
                TypedGoalBindingDecision::Refused(_)
            ));
        }

        let stale = nullable_construction_spec_fixture(&mut worker, positive_source, false, vec![]);
        let mut authority =
            EvidenceAuthority::open(&stale.repo, &stale.payload.repository_revision).unwrap();
        let receipt = authority
            .issue_external_spec_with_verifier(
                &stale.specification_path,
                &stale.request,
                Some(":/main"),
                TEST_EXTERNAL_SPEC_ISSUER,
                stale.signing_key.verifying_key().to_bytes(),
                &mut worker,
            )
            .unwrap();
        std::fs::write(
            stale.repo.join("src/main/kotlin/neutral/Construction.kt"),
            format!("{positive_source}\ninternal fun changed(): Int = 1\n"),
        )
        .unwrap();
        assert!(matches!(
            authority
                .bind_typed_goal_with_external_spec(
                    &stale.request,
                    Some(":/main"),
                    &receipt,
                    &mut worker,
                )
                .unwrap(),
            TypedGoalBindingDecision::Refused(_)
        ));
        worker.shutdown().unwrap();
    }

    #[test]
    fn nullable_construction_provider_is_explicitly_unsupported() {
        let source = r#"
package neutral.nulls
internal data class Vessel(val first: String, val second: String)
internal fun nullableValue(enabled: Boolean): String? = if (enabled) "value" else null
internal fun fallbackValue(): String = "fallback"
internal fun assemble(enabled: Boolean, sameTypeDecoy: String): Vessel =
    Vessel(second = nullableValue(enabled) ?: fallbackValue(), first = sameTypeDecoy)
"#;
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let fixture = nullable_construction_spec_fixture(&mut worker, source, false, vec![]);
        assert_eq!(
            fixture.request.goal.execution_plan(),
            Err(TypedGoalLanguageError::UnsupportedConstraintDomain)
        );
        let mut authority =
            EvidenceAuthority::open(&fixture.repo, &fixture.payload.repository_revision).unwrap();
        assert!(authority.typed_goal_proofs.is_empty());
        let source_path = fixture.repo.join("src/main/kotlin/neutral/Construction.kt");
        let original = std::fs::read(&source_path).unwrap();
        let mut dirty = original.clone();
        dirty.extend_from_slice(b"\n// dirty before direct preflight\n");
        std::fs::write(&source_path, dirty).unwrap();
        let bare_value_flow = TypedSemanticGoal::new(
            &fixture.payload.repository_revision,
            [
                ("source".into(), TypedVariableDomain::Declaration),
                ("destination".into(), TypedVariableDomain::Declaration),
            ],
            [OperatorApplication {
                operator: PrimitiveConstraint::ValueFlowsTo,
                operands: vec!["source".into(), "destination".into()],
            }],
        );
        let bare_decision = authority
            .bind_typed_goal(&bare_value_flow, &[], Some(":/main"), &mut worker)
            .unwrap();
        assert!(matches!(
            bare_decision,
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::InvalidGoal
                    && refusal.rejections.is_empty()
                    && refusal.declaration_rejections.is_empty()
        ));
        assert!(authority.typed_goal_proofs.is_empty());
        assert!(matches!(
            authority
                .bind_typed_goal(
                    &fixture.request.goal,
                    &fixture.request.hints,
                    Some(":/main"),
                    &mut worker,
                )
                .unwrap(),
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::UnsupportedConstraintDomain
        ));
        assert!(authority.external_specs.is_empty());
        assert!(authority.typed_goal_proofs.is_empty());
        std::fs::write(&source_path, &original).unwrap();

        let absent_repo = fixture.repo.with_extension("temporarily-absent");
        std::fs::rename(&fixture.repo, &absent_repo).unwrap();
        let missing_repo_decision = authority
            .bind_typed_goal(
                &fixture.request.goal,
                &fixture.request.hints,
                Some(":/main"),
                &mut worker,
            )
            .unwrap();
        std::fs::rename(&absent_repo, &fixture.repo).unwrap();
        assert!(matches!(
            missing_repo_decision,
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::UnsupportedConstraintDomain
        ));
        assert!(authority.external_specs.is_empty());
        assert!(authority.typed_goal_proofs.is_empty());

        let unknown = ExternalSpecReceipt {
            session_id: authority.session_id,
            receipt_id: Uuid::new_v4(),
        };
        let stored_specs = authority.external_specs.len();
        let domain_first = authority
            .bind_typed_goal_with_external_spec(
                &fixture.request,
                Some(":/main"),
                &unknown,
                &mut worker,
            )
            .unwrap();
        assert!(matches!(
            domain_first,
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::UnsupportedConstraintDomain
                    && refusal.rejections.is_empty()
                    && refusal.declaration_rejections.is_empty()
        ));
        let cross_session = ExternalSpecReceipt {
            session_id: Uuid::new_v4(),
            receipt_id: unknown.receipt_id,
        };
        assert!(matches!(
            authority
                .bind_typed_goal_with_external_spec(
                    &fixture.request,
                    Some(":/main"),
                    &cross_session,
                    &mut worker,
                )
                .unwrap(),
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::UnsupportedConstraintDomain
        ));
        assert_eq!(authority.external_specs.len(), stored_specs);
        assert!(authority.typed_goal_proofs.is_empty());

        let receipt = authority
            .issue_external_spec_with_verifier(
                &fixture.specification_path,
                &fixture.request,
                Some(":/main"),
                TEST_EXTERNAL_SPEC_ISSUER,
                fixture.signing_key.verifying_key().to_bytes(),
                &mut worker,
            )
            .unwrap();
        let stored_specs = authority.external_specs.len();
        let mut changed = original.clone();
        changed.extend_from_slice(b"\n// changed after receipt\n");
        std::fs::write(&source_path, changed).unwrap();
        assert!(matches!(
            authority
                .bind_typed_goal_with_external_spec(
                    &fixture.request,
                    Some(":/main"),
                    &receipt,
                    &mut worker,
                )
                .unwrap(),
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::UnsupportedConstraintDomain
        ));
        assert_eq!(authority.external_specs.len(), stored_specs);
        assert!(authority.typed_goal_proofs.is_empty());

        let supported_request = TypedGoalBindingRequest {
            schema: TYPED_GOAL_BINDING_REQUEST_SCHEMA.into(),
            goal: TypedSemanticGoal::new(
                &fixture.payload.repository_revision,
                [
                    ("source".into(), TypedVariableDomain::Declaration),
                    ("target".into(), TypedVariableDomain::Declaration),
                ],
                [OperatorApplication {
                    operator: PrimitiveConstraint::PropagateDeclaredType,
                    operands: vec!["source".into(), "target".into()],
                }],
            ),
            hints: vec![],
            compilation: Some(":/main".into()),
        };
        let supported_direct =
            authority.bind_typed_goal(&supported_request.goal, &[], Some(":/main"), &mut worker);
        assert!(matches!(
            supported_direct,
            Err(error) if error.code == ErrorCode::StaleRequiresReslice
        ));
        assert!(matches!(
            authority
                .bind_typed_goal_with_external_spec(
                    &supported_request,
                    Some(":/main"),
                    &receipt,
                    &mut worker,
                )
                .unwrap(),
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::ExternalSpecificationMismatch
        ));
        std::fs::write(&source_path, original).unwrap();
        let decision = authority
            .bind_typed_goal(
                &fixture.request.goal,
                &fixture.request.hints,
                Some(":/main"),
                &mut worker,
            )
            .unwrap();
        let TypedGoalBindingDecision::Refused(refusal) = decision else {
            panic!("frozen nullable contour must not issue a proof: {decision:#?}")
        };
        assert_eq!(
            refusal.reason,
            TypedGoalRefusalReason::UnsupportedConstraintDomain
        );
        assert!(refusal.rejections.is_empty());
        assert!(refusal.declaration_rejections.is_empty());
        assert!(authority.typed_goal_proofs.is_empty());
        assert!(
            !crate::semantic_goal::typed_goal_language_schema()
                .executable_domains
                .contains(&ConstraintDomain::NullableConstruction)
        );
        worker.shutdown().unwrap();
    }

    #[test]
    fn projection_consumer_provider_is_explicitly_unsupported() {
        let source = r#"
package neutral.pipeline
internal val stored: String = "value"
internal fun source(): String { return stored }
internal fun projection(): String { return source() }
internal fun consume(first: String, second: String): String { return source() }
internal fun execute(): String { return consume(second = stored, first = projection()) }
"#;
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let fixture = projection_consumer_spec_fixture(&mut worker, source, false, vec![]);
        let mut authority =
            EvidenceAuthority::open(&fixture.repo, &fixture.payload.repository_revision).unwrap();
        let forged = ExternalSpecReceipt {
            session_id: Uuid::new_v4(),
            receipt_id: Uuid::new_v4(),
        };
        let decision = authority
            .bind_typed_goal_with_external_spec(
                &fixture.request,
                Some(":/main"),
                &forged,
                &mut worker,
            )
            .unwrap();
        let TypedGoalBindingDecision::Refused(refusal) = decision else {
            panic!("frozen projection contour must not issue a proof: {decision:#?}")
        };
        assert_eq!(
            refusal.reason,
            TypedGoalRefusalReason::UnsupportedConstraintDomain
        );
        assert!(refusal.rejections.is_empty());
        assert!(refusal.declaration_rejections.is_empty());
        assert!(authority.external_specs.is_empty());
        assert!(authority.typed_goal_proofs.is_empty());
        assert!(
            !crate::semantic_goal::typed_goal_language_schema()
                .executable_domains
                .contains(&ConstraintDomain::Projection)
        );
        worker.shutdown().unwrap();
    }

    #[cfg(any())]
    #[test]
    fn projection_consumer_provider_gradle_maven_and_negative_group() {
        fn proof_shape(
            summary: &TypedGoalProofSummary,
        ) -> Vec<(PrimitiveConstraint, EvidenceRelation)> {
            summary
                .evidence_relations
                .iter()
                .map(|record| (record.operator.operator.clone(), record.relation))
                .collect()
        }
        let positive = r#"
package neutral.pipeline
internal val stored: String = "value"
internal fun source(): String { return stored }
internal fun projection(): String { return source() }
internal fun consume(first: String, second: String): String { return source() }
internal fun execute(): String { return consume(second = stored, first = projection()) }
"#;
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let gradle = projection_consumer_spec_fixture(
            &mut worker,
            positive,
            false,
            vec!["advisory vocabulary cannot choose declarations".into()],
        );
        let gradle_decision = bind_declaration_fixture(&gradle, &mut worker);
        let TypedGoalBindingDecision::Bound(gradle_bound) = gradle_decision else {
            panic!("unique Gradle projection must bind: {gradle_decision:#?}")
        };
        assert_eq!(gradle_bound.summary().bindings.len(), 3);
        assert!(proof_shape(gradle_bound.summary()).contains(&(
            PrimitiveConstraint::ProjectsValue,
            EvidenceRelation::ValueProjection,
        )));

        let maven = projection_consumer_spec_fixture(&mut worker, positive, true, vec![]);
        let maven_decision = bind_declaration_fixture(&maven, &mut worker);
        let TypedGoalBindingDecision::Bound(maven_bound) = maven_decision else {
            panic!("unique Maven projection must bind: {maven_decision:#?}")
        };
        assert_eq!(
            proof_shape(gradle_bound.summary()),
            proof_shape(maven_bound.summary())
        );

        let repeated = projection_consumer_spec_fixture(
            &mut worker,
            &format!(
                "{positive}\n{}",
                r#"internal fun executeAgain(): String { return consume(second = stored, first = projection()) }"#
            ),
            false,
            vec![],
        );
        let repeated_decision = bind_declaration_fixture(&repeated, &mut worker);
        let TypedGoalBindingDecision::Bound(repeated_bound) = repeated_decision else {
            panic!("repeated projection occurrences must aggregate: {repeated_decision:#?}")
        };
        assert!(
            repeated_bound
                .summary()
                .evidence_relations
                .iter()
                .all(|record| {
                    record.occurrence_count == Some(2)
                        && record.occurrence_fingerprints.len() == 2
                        && record
                            .occurrence_set_fingerprint
                            .as_ref()
                            .is_some_and(|fingerprint| {
                                canonical::hash(&record.occurrence_fingerprints)
                                    .is_ok_and(|actual| &actual == fingerprint)
                            })
                })
        );

        let alpha = projection_consumer_spec_fixture(
            &mut worker,
            &positive
                .replace("stored", "retained")
                .replace("source", "origin")
                .replace("projection", "mapping")
                .replace("consume", "accept")
                .replace("execute", "run"),
            false,
            vec![],
        );
        let alpha_decision = bind_declaration_fixture(&alpha, &mut worker);
        let TypedGoalBindingDecision::Bound(alpha_bound) = alpha_decision else {
            panic!("alpha-renamed hint-free projection must bind: {alpha_decision:#?}")
        };
        assert_eq!(
            proof_shape(gradle_bound.summary()),
            proof_shape(alpha_bound.summary())
        );

        let second = positive
            .replace("package neutral.pipeline", "")
            .replace("stored", "storedTwo")
            .replace("source", "sourceTwo")
            .replace("projection", "projectionTwo")
            .replace("consume", "consumeTwo")
            .replace("execute", "executeTwo");
        let ambiguous = projection_consumer_spec_fixture(
            &mut worker,
            &format!("{positive}\n{second}"),
            false,
            vec![],
        );
        assert!(matches!(
            bind_declaration_fixture(&ambiguous, &mut worker),
            TypedGoalBindingDecision::Ambiguous(_)
        ));

        let unknown = projection_consumer_spec_fixture(
            &mut worker,
            &positive.replace(
                "internal fun projection(): String { return source() }",
                "internal fun projection(): String { val alias = source(); return alias }",
            ),
            false,
            vec![],
        );
        assert!(matches!(
            bind_declaration_fixture(&unknown, &mut worker),
            TypedGoalBindingDecision::Refused(_)
        ));

        let public_boundary = projection_consumer_spec_fixture(
            &mut worker,
            &positive.replace("internal fun consume", "public fun consume"),
            false,
            vec![],
        );
        assert!(matches!(
            bind_declaration_fixture(&public_boundary, &mut worker),
            TypedGoalBindingDecision::Refused(_)
        ));
    }

    #[test]
    fn declaration_type_provider_binds_positive_verified_index_closure() {
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let positive = declaration_type_spec_fixture(
            &mut worker,
            r#"
package example

internal interface MeasurePort { fun measure(): Number }
internal class ExactMeasure : MeasurePort {
    override fun measure(): Int = 1
}
internal fun render(port: MeasurePort): Number = port.measure()
internal fun unrelated(value: String): String = value
"#,
        );
        let decision = bind_declaration_fixture(&positive, &mut worker);
        let TypedGoalBindingDecision::Bound(bound) = decision else {
            panic!("internal exact declaration closure must bind: {decision:?}")
        };
        assert_eq!(bound.summary().bindings.len(), 2);
        assert!(
            bound
                .summary()
                .bindings
                .values()
                .all(|identity| identity.contains("#jvm:"))
        );
        assert!(bound.summary().external_spec.is_some());
        assert!(bound.summary().evidence_relations.iter().all(|record| {
            record.current && !record.unknown && !record.evidence_fingerprint.is_empty()
        }));
        worker.shutdown().unwrap();
    }

    #[test]
    fn declaration_type_provider_generalization_and_negative_group() {
        fn proof_shape(
            summary: &TypedGoalProofSummary,
        ) -> Vec<(PrimitiveConstraint, EvidenceRelation)> {
            summary
                .evidence_relations
                .iter()
                .map(|record| (record.operator.operator.clone(), record.relation))
                .collect()
        }
        fn assert_non_executable_refusal(decision: &TypedGoalBindingDecision) {
            let TypedGoalBindingDecision::Refused(refusal) = decision else {
                panic!("expected refusal, got {decision:?}")
            };
            let encoded = serde_json::to_string(refusal).unwrap();
            assert!(!encoded.contains("changeGraph"));
            assert!(!encoded.contains("plannedSemanticOperations"));
            assert!(!encoded.contains("receiptId"));
        }

        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();

        let ambiguous = declaration_type_spec_fixture(
            &mut worker,
            r#"
package neutral
internal interface LeftPort { fun read(): Number }
internal class LeftReader : LeftPort { override fun read(): Int = 1 }
internal fun leftUse(port: LeftPort): Number = port.read()
internal interface RightPort { fun size(): Number }
internal class RightReader : RightPort { override fun size(): Int = 1 }
internal fun rightUse(port: RightPort): Number = port.size()
"#,
        );
        let decision = bind_declaration_fixture(&ambiguous, &mut worker);
        let TypedGoalBindingDecision::Ambiguous(ambiguity) = decision else {
            panic!("two independent exact closures must be ambiguous: {decision:?}")
        };
        assert_eq!(ambiguity.choices.len(), 2);
        assert!(ambiguity.choices.iter().all(|choice| {
            choice.bindings.keys().cloned().collect::<BTreeSet<_>>()
                == BTreeSet::from(["origin".into(), "contract".into()])
                && choice
                    .bindings
                    .values()
                    .all(|value| value.contains("#jvm:"))
        }));
        let encoded = serde_json::to_string(&ambiguity).unwrap();
        assert!(!encoded.contains("changeGraph"));
        assert!(!encoded.contains("receiptId"));

        let exported = declaration_type_spec_fixture(
            &mut worker,
            r#"
package neutral
interface PublicPort { fun read(input: Int): Number }
class PublicReader : PublicPort { override fun read(input: Int): Int = input }
fun publicUse(port: PublicPort, input: Int): Number = port.read(input)
"#,
        );
        assert_non_executable_refusal(&bind_declaration_fixture(&exported, &mut worker));

        let overloaded = declaration_type_spec_fixture(
            &mut worker,
            r#"
package neutral
internal interface OverloadedPort {
    fun read(input: Int): Number
    fun read(input: String): Number
}
internal class OverloadedReader : OverloadedPort {
    override fun read(input: Int): Int = input
    override fun read(input: String): Int = input.length
}
"#,
        );
        assert_non_executable_refusal(&bind_declaration_fixture(&overloaded, &mut worker));

        let unknown = declaration_type_spec_fixture(
            &mut worker,
            r#"
package neutral
internal interface SafePort { fun read(input: Int): Number }
internal class SafeReader : SafePort { override fun read(input: Int): Int = input }
internal fun safeUse(port: SafePort, input: Int): Number = port.read(input)
internal fun unresolvedUse(): String = missingCompilerDeclaration()
"#,
        );
        assert_non_executable_refusal(&bind_declaration_fixture(&unknown, &mut worker));

        let incomplete_closure = declaration_type_spec_fixture(
            &mut worker,
            r#"
package neutral
internal interface ClosedPort { fun read(input: Int): Number }
internal class ClosedReader : ClosedPort { override fun read(input: Int): Int = input }
internal fun internalUse(port: ClosedPort, input: Int): Number = port.read(input)
private fun hiddenAdditionalUse(port: ClosedPort, input: Int): Number = port.read(input)
"#,
        );
        assert_non_executable_refusal(&bind_declaration_fixture(&incomplete_closure, &mut worker));

        let first = declaration_type_spec_fixture(
            &mut worker,
            r#"
package first.layout
internal interface Alpha { fun convert(): Number }
internal class Beta : Alpha { override fun convert(): Int = 1 }
internal fun gamma(port: Alpha): Number = port.convert()
internal fun lexicalDecoy(value: String): String = value
"#,
        );
        let first_decision = bind_declaration_fixture(&first, &mut worker);
        let TypedGoalBindingDecision::Bound(first_bound) = first_decision else {
            panic!("first alpha-renamed fixture must bind: {first_decision:?}")
        };

        let mut second = declaration_type_spec_fixture(
            &mut worker,
            r#"
package another.deep.layout
internal interface Delta { fun apply(): Number }
internal class Epsilon : Delta { override fun apply(): Int = 1 }
internal fun zeta(port: Delta): Number = port.apply()
internal class Decoy { fun apply(number: String): String = number }
"#,
        );
        second.request.hints.clear();
        second.payload.request_digest = canonical::hash(&second.request).unwrap();
        second.payload.package_digest = external_spec_package_digest(&second.payload).unwrap();
        write_test_signed_external_spec(
            &second.specification_path,
            &second.payload,
            &second.signing_key,
        );
        let second_decision = bind_declaration_fixture(&second, &mut worker);
        let TypedGoalBindingDecision::Bound(second_bound) = second_decision else {
            panic!("layout/alpha-renamed fixture without hints must bind: {second_decision:?}")
        };
        assert_eq!(
            proof_shape(first_bound.summary()),
            proof_shape(second_bound.summary())
        );
        for summary in [first_bound.summary(), second_bound.summary()] {
            let encoded = serde_json::to_string(summary).unwrap();
            for forbidden in [
                "MAP_EDGE_WITH_CONTEXT",
                "producer",
                "transformer",
                "valueEdge",
            ] {
                assert!(
                    !encoded.contains(forbidden),
                    "proof leaked vocabulary {forbidden}"
                );
            }
        }

        let stale = declaration_type_spec_fixture(
            &mut worker,
            r#"
package neutral
internal interface StalePort { fun read(): Number }
internal class StaleReader : StalePort { override fun read(): Int = 1 }
internal fun staleUse(port: StalePort): Number = port.read()
"#,
        );
        let mut authority =
            EvidenceAuthority::open(&stale.repo, &stale.payload.repository_revision).unwrap();
        let receipt = authority
            .issue_external_spec_with_verifier(
                &stale.specification_path,
                &stale.request,
                Some(":/main"),
                TEST_EXTERNAL_SPEC_ISSUER,
                stale.signing_key.verifying_key().to_bytes(),
                &mut worker,
            )
            .unwrap();
        let source = stale.repo.join("src/main/kotlin/example/Declarations.kt");
        let mut changed = std::fs::read_to_string(&source).unwrap();
        changed.push_str("\ninternal fun changedAfterReceipt(): Int = 1\n");
        std::fs::write(source, changed).unwrap();
        let stale_decision = authority
            .bind_typed_goal_with_external_spec(
                &stale.request,
                Some(":/main"),
                &receipt,
                &mut worker,
            )
            .unwrap();
        assert_non_executable_refusal(&stale_decision);
        worker.shutdown().unwrap();
    }

    #[test]
    fn declaration_type_provider_maven_matches_gradle_proof_shape() {
        fn proof_shape(
            summary: &TypedGoalProofSummary,
        ) -> Vec<(PrimitiveConstraint, EvidenceRelation)> {
            summary
                .evidence_relations
                .iter()
                .map(|record| (record.operator.operator.clone(), record.relation))
                .collect()
        }
        let source = r#"
package example
internal interface MeasurePort { fun measure(): Number }
internal class ExactMeasure : MeasurePort {
    override fun measure(): Int = 1
}
internal fun render(port: MeasurePort): Number = port.measure()
internal fun unrelated(value: String): String = value
"#;
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let gradle = declaration_type_spec_fixture(&mut worker, source);
        let gradle_decision = bind_declaration_fixture(&gradle, &mut worker);
        let TypedGoalBindingDecision::Bound(gradle_bound) = gradle_decision else {
            panic!("neutral Gradle cell must bind: {gradle_decision:?}")
        };

        let maven = declaration_type_maven_spec_fixture(&mut worker, source);
        let maven_decision = bind_declaration_fixture(&maven, &mut worker);
        let TypedGoalBindingDecision::Bound(maven_bound) = maven_decision else {
            panic!("neutral Maven cell must bind: {maven_decision:?}")
        };
        assert_eq!(
            maven_bound.summary().bindings,
            gradle_bound.summary().bindings
        );
        assert_eq!(
            proof_shape(maven_bound.summary()),
            proof_shape(gradle_bound.summary())
        );
        assert_ne!(
            maven_bound.summary().evidence_fingerprint,
            gradle_bound.summary().evidence_fingerprint,
            "build/compiler provenance must remain committed",
        );

        let exported = declaration_type_maven_spec_fixture(
            &mut worker,
            r#"
package example
interface PublicPort { fun measure(input: Int): Number }
class PublicMeasure : PublicPort { override fun measure(input: Int): Int = input }
fun render(port: PublicPort, input: Int): Number = port.measure(input)
"#,
        );
        let decision = bind_declaration_fixture(&exported, &mut worker);
        let TypedGoalBindingDecision::Refused(refusal) = decision else {
            panic!("public Maven ABI without explicit signed policy must refuse: {decision:?}")
        };
        let encoded = serde_json::to_string(&refusal).unwrap();
        assert!(!encoded.contains("changeGraph"));
        assert!(!encoded.contains("receiptId"));
        worker.shutdown().unwrap();
    }

    #[test]
    fn declaration_type_provider_binds_ambiguity_and_refuses_unsafe_closure() {
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let positive = declaration_type_spec_fixture(
            &mut worker,
            r#"
package example

internal interface MeasurePort { fun measure(): Number }
internal class ExactMeasure : MeasurePort {
    override fun measure(): Int = 1
}
internal fun render(port: MeasurePort): Number = port.measure()
internal fun unrelated(value: String): String = value
"#,
        );
        let decision = bind_declaration_fixture(&positive, &mut worker);
        let TypedGoalBindingDecision::Bound(bound) = decision else {
            panic!("internal exact declaration closure must bind: {decision:?}")
        };
        assert_eq!(bound.summary().bindings.len(), 2);
        assert!(
            bound
                .summary()
                .bindings
                .values()
                .all(|identity| identity.contains("#jvm:"))
        );
        assert!(bound.summary().external_spec.is_some());
        assert!(bound.summary().evidence_relations.iter().all(|record| {
            record.current && !record.unknown && !record.evidence_fingerprint.is_empty()
        }));
        let plan = positive.request.goal.execution_plan().unwrap();
        let mandatory_relations = plan
            .mandatory_closure
            .iter()
            .flat_map(|application| {
                constraint_op_spec(&application.operator)
                    .required_evidence_relations
                    .iter()
                    .copied()
            })
            .collect::<BTreeSet<_>>();
        let same_owner_return = json!({
            "provider":"K2_FIR",
            "stage":"RETURN_VALUE",
            "code":"IMPLICIT_RETURN_UNSUPPORTED",
        });
        assert!(
            boundary_affected_relations(&same_owner_return)
                .unwrap()
                .is_disjoint(&mandatory_relations),
            "same-owner return-flow Unknown must be committed but relation-disjoint"
        );
        let relabelled_override = json!({
            "provider":"K2_FIR",
            "stage":"OVERRIDE",
            "code":"NO_RESOLVED_BASE",
        });
        assert!(
            !boundary_affected_relations(&relabelled_override)
                .unwrap()
                .is_disjoint(&mandatory_relations),
            "a relabelled relevant override boundary must intersect and refuse"
        );

        let ambiguous = declaration_type_spec_fixture(
            &mut worker,
            r#"
package example

internal interface FirstPort { fun read(): Number }
internal class FirstReader : FirstPort { override fun read(): Int = 1 }
internal fun firstUse(port: FirstPort): Number = port.read()
internal interface SecondPort { fun size(): Number }
internal class SecondReader : SecondPort { override fun size(): Int = 1 }
internal fun secondUse(port: SecondPort): Number = port.size()
"#,
        );
        let decision = bind_declaration_fixture(&ambiguous, &mut worker);
        assert!(matches!(decision, TypedGoalBindingDecision::Ambiguous(_)));

        let exported = declaration_type_spec_fixture(
            &mut worker,
            r#"
package example

interface PublicPort { fun read(input: Int): Number }
class PublicReader : PublicPort { override fun read(input: Int): Int = input }
fun publicUse(port: PublicPort, input: Int): Number = port.read(input)
"#,
        );
        let decision = bind_declaration_fixture(&exported, &mut worker);
        assert!(matches!(
            decision,
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::NoCompatibleBindings
                    || refusal.reason == TypedGoalRefusalReason::UnsupportedBoundary
        ));
        worker.shutdown().unwrap();
    }

    #[test]
    fn declaration_type_provider_rejects_truly_mixed_executable_domains() {
        let goal = TypedSemanticGoal::new(
            "revision",
            [
                ("callable".into(), TypedVariableDomain::Callable),
                ("edge".into(), TypedVariableDomain::ValueEdge),
                ("source".into(), TypedVariableDomain::Declaration),
                ("target".into(), TypedVariableDomain::Declaration),
            ],
            [
                OperatorApplication {
                    operator: PrimitiveConstraint::TypeAssignable,
                    operands: vec!["callable".into(), "edge".into()],
                },
                OperatorApplication {
                    operator: PrimitiveConstraint::PropagateDeclaredType,
                    operands: vec!["source".into(), "target".into()],
                },
            ],
        );
        let plan = goal.execution_plan().unwrap();
        assert_eq!(
            executable_plan_domain(&plan),
            Err(TypedGoalRefusalReason::UnsupportedConstraintDomain)
        );
    }

    #[test]
    fn value_flows_to_provider_refuses_unmapped_occurrences() {
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let positive = declaration_type_spec_fixture(
            &mut worker,
            r#"
package neutral.flow
internal fun origin(value: String): String = value
internal fun destination(first: String, second: String): String = second
internal fun lexicalDecoy(value: String): String = value
internal fun route(seed: String, sameTypeDecoy: String): String =
    destination(sameTypeDecoy, origin(seed))
"#,
        );
        let candidates =
            discover_declaration_value_flow_candidates(&positive.repo, Some(":/main"), &mut worker)
                .unwrap();
        assert!(
            candidates.is_empty(),
            "a CALL without compiler argument-to-parameter evidence must not create a value-flow candidate"
        );

        let mandatory = BTreeSet::from([EvidenceRelation::DeclarationValueFlow]);
        let roots = BTreeSet::from([
            "neutral/flow/route".to_owned(),
            "neutral/flow/origin".to_owned(),
            "neutral/flow/destination".to_owned(),
        ]);
        let return_boundary = json!({
            "provider":"K2_FIR",
            "stage":"RETURN_VALUE",
            "code":"IMPLICIT_RETURN_UNSUPPORTED",
            "owner":"neutral/flow/route",
        });
        let relation_graph = json!({
            "coverage":"PARTIAL",
            "boundaries":[return_boundary],
        });
        let descriptor_graph = json!({
            "coverage":"COMPLETE_SUPPORTED_SUBSET",
            "boundaries":[],
        });
        let excluded = evaluate_obligation_relative_boundaries(
            &[
                ("RELATION", &relation_graph),
                ("DESCRIPTOR", &descriptor_graph),
            ],
            &mandatory,
            &roots,
        )
        .unwrap();
        assert_eq!(excluded.total_boundary_count, 1);
        assert_eq!(excluded.relevant_boundary_count, 1);
        assert_eq!(excluded.excluded_by_relation_count, 1);
        assert_eq!(excluded.refused_boundary_count, 0);
        assert!(excluded.fingerprint.starts_with("sha256:"));
        assert_eq!(
            excluded.decisions[0]["exclusionReason"],
            "RELATION_DISJOINT"
        );

        let mapped_argument_boundary = json!({
            "provider":"K2_FIR",
            "stage":"ARGUMENT_MAPPING",
            "code":"MISSING_RESOLVED_ARGUMENT_MAPPING",
            "owner":"neutral/flow/route",
        });
        let relevant_graph = json!({
            "coverage":"PARTIAL",
            "boundaries":[mapped_argument_boundary],
        });
        assert!(
            evaluate_obligation_relative_boundaries(
                &[
                    ("RELATION", &relevant_graph),
                    ("DESCRIPTOR", &descriptor_graph)
                ],
                &mandatory,
                &roots,
            )
            .is_err(),
            "a mapped CALL/argument Unknown intersects VALUE_FLOWS_TO and must refuse",
        );
        worker.shutdown().unwrap();
    }

    struct TestEvidenceProvider;

    impl OperatorEvidenceProvider for TestEvidenceProvider {
        fn prove(
            &self,
            application: &OperatorApplication,
        ) -> Result<Vec<ProviderFactReceipt>, TypedGoalRefusalReason> {
            Ok(constraint_op_spec(&application.operator)
                .required_evidence_relations
                .iter()
                .map(|relation| ProviderFactReceipt {
                    relation: *relation,
                    provider_kind: "test-fact-provider",
                    fact_fingerprint: canonical::hash(&(application, relation)).unwrap(),
                })
                .collect())
        }
    }

    fn valid_typed_proof() -> (TypedSemanticGoal, TypedGoalProofSummary) {
        let goal = TypedSemanticGoal::new(
            "base",
            [
                ("call".into(), TypedVariableDomain::Callable),
                ("edge".into(), TypedVariableDomain::ValueEdge),
            ],
            [OperatorApplication {
                operator: PrimitiveConstraint::TypeAssignable,
                operands: vec!["call".into(), "edge".into()],
            }],
        );
        let plan = goal.execution_plan().unwrap();
        let bindings = BTreeMap::from([
            ("call".into(), "compiler/call".into()),
            ("edge".into(), "compiler/flow#p->r".into()),
        ]);
        let relations = prove_relation_records(&TestEvidenceProvider, &plan, &bindings).unwrap();
        let graph = typed_change_graph(&goal, &plan, &bindings, &relations).unwrap();
        let summary = TypedGoalProofSummary {
            schema: TYPED_GOAL_PROOF_SUMMARY_SCHEMA.into(),
            revision: "base".into(),
            goal_fingerprint: canonical::hash(&goal).unwrap(),
            bindings,
            discharged_operators: plan.mandatory_closure,
            evidence_relations: relations,
            change_graph: graph,
            evidence_fingerprint: "evidence".into(),
            external_spec: None,
        };
        assert!(summary.is_complete_for(&goal));
        (goal, summary)
    }

    #[test]
    fn typed_proof_rejects_current_unknown_relation() {
        let (goal, mut proof) = valid_typed_proof();
        proof.evidence_relations[0].unknown = true;
        assert!(!proof.is_complete_for(&goal));
    }

    #[test]
    fn typed_proof_rejects_relabelled_relation() {
        let (goal, mut proof) = valid_typed_proof();
        proof.evidence_relations[0].relation = EvidenceRelation::ResourceLifetimePreservation;
        assert!(!proof.is_complete_for(&goal));
    }

    #[test]
    fn signed_external_spec_is_trust_root_and_runtime_bound() {
        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let fixture = signed_external_spec_fixture(&mut worker);
        let verifying_key = fixture.signing_key.verifying_key().to_bytes();
        let mut authority =
            EvidenceAuthority::open(&fixture.repo, &fixture.payload.repository_revision).unwrap();
        let receipt = authority
            .issue_external_spec_with_verifier(
                &fixture.specification_path,
                &fixture.request,
                Some(":/main"),
                TEST_EXTERNAL_SPEC_ISSUER,
                verifying_key,
                &mut worker,
            )
            .unwrap();
        assert!(authority.recognizes_external_spec(&receipt).unwrap());
        let decision = authority
            .bind_typed_goal_with_external_spec(
                &fixture.request,
                Some(":/main"),
                &receipt,
                &mut worker,
            )
            .unwrap();
        let TypedGoalBindingDecision::Bound(bound) = decision else {
            panic!("trusted external specification must bind: {decision:?}")
        };
        let proof = bound.summary().external_spec.as_ref().unwrap();
        assert_eq!(proof.issuer, TEST_EXTERNAL_SPEC_ISSUER);
        assert_eq!(proof.package_digest, fixture.payload.package_digest);
        let serialized = serde_json::to_string(proof).unwrap();
        assert!(!serialized.contains(&fixture.payload.task));
        assert!(!serialized.contains("receipt_id"));

        let changed_request = TypedGoalBindingRequest {
            hints: vec!["caller change".into()],
            ..fixture.request.clone()
        };
        let decision = authority
            .bind_typed_goal_with_external_spec(
                &changed_request,
                Some(":/main"),
                &receipt,
                &mut worker,
            )
            .unwrap();
        assert!(matches!(
            decision,
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::ExternalSpecificationMismatch
        ));

        let original_manifest = std::fs::read(&fixture.manifest_path).unwrap();
        std::fs::write(&fixture.manifest_path, b"{}").unwrap();
        let decision = authority
            .bind_typed_goal_with_external_spec(
                &fixture.request,
                Some(":/main"),
                &receipt,
                &mut worker,
            )
            .unwrap();
        assert!(matches!(
            decision,
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::ExternalSpecificationMismatch
        ));
        std::fs::write(&fixture.manifest_path, original_manifest).unwrap();

        let source = fixture.repo.join("src/main/kotlin/com/acme/Runner.kt");
        let original_source = std::fs::read(&source).unwrap();
        let mut changed_source = original_source.clone();
        changed_source.extend_from_slice(b"\n// changed after receipt\n");
        std::fs::write(&source, changed_source).unwrap();
        let decision = authority
            .bind_typed_goal_with_external_spec(
                &fixture.request,
                Some(":/main"),
                &receipt,
                &mut worker,
            )
            .unwrap();
        assert!(matches!(
            decision,
            TypedGoalBindingDecision::Refused(refusal)
                if refusal.reason == TypedGoalRefusalReason::ExternalSpecificationMismatch
        ));
        std::fs::write(&source, original_source).unwrap();

        let mut changed_payload = fixture.payload.clone();
        changed_payload.task.push_str(" changed without resigning");
        let original_envelope = std::fs::read(&fixture.specification_path).unwrap();
        let original: SignedExternalSpecEnvelope =
            serde_json::from_slice(&original_envelope).unwrap();
        let invalid = SignedExternalSpecEnvelope {
            payload: changed_payload,
            ..original.clone()
        };
        std::fs::write(
            &fixture.specification_path,
            canonical::bytes(&invalid).unwrap(),
        )
        .unwrap();
        assert!(
            authority
                .issue_external_spec_with_verifier(
                    &fixture.specification_path,
                    &fixture.request,
                    Some(":/main"),
                    TEST_EXTERNAL_SPEC_ISSUER,
                    verifying_key,
                    &mut worker,
                )
                .is_err()
        );

        let caller_key = SigningKey::from_bytes(&[9_u8; 32]);
        write_test_signed_external_spec(&fixture.specification_path, &fixture.payload, &caller_key);
        assert!(
            authority
                .issue_external_spec_with_verifier(
                    &fixture.specification_path,
                    &fixture.request,
                    Some(":/main"),
                    TEST_EXTERNAL_SPEC_ISSUER,
                    verifying_key,
                    &mut worker,
                )
                .is_err()
        );

        let mut wrong_issuer = fixture.payload.clone();
        wrong_issuer.issuer = "caller-selected-issuer".into();
        wrong_issuer.package_digest = external_spec_package_digest(&wrong_issuer).unwrap();
        write_test_signed_external_spec(
            &fixture.specification_path,
            &wrong_issuer,
            &fixture.signing_key,
        );
        assert!(
            authority
                .issue_external_spec_with_verifier(
                    &fixture.specification_path,
                    &fixture.request,
                    Some(":/main"),
                    TEST_EXTERNAL_SPEC_ISSUER,
                    verifying_key,
                    &mut worker,
                )
                .is_err()
        );

        let mut escape = fixture.payload.clone();
        escape.repository = "../checkout/fixtures/kotlin-2-1".into();
        escape.package_digest = external_spec_package_digest(&escape).unwrap();
        write_test_signed_external_spec(&fixture.specification_path, &escape, &fixture.signing_key);
        assert!(
            authority
                .issue_external_spec_with_verifier(
                    &fixture.specification_path,
                    &fixture.request,
                    Some(":/main"),
                    TEST_EXTERNAL_SPEC_ISSUER,
                    verifying_key,
                    &mut worker,
                )
                .is_err()
        );

        std::fs::write(
            &fixture.specification_path,
            canonical::bytes(&fixture.payload).unwrap(),
        )
        .unwrap();
        assert!(
            authority
                .issue_external_spec_with_verifier(
                    &fixture.specification_path,
                    &fixture.request,
                    Some(":/main"),
                    TEST_EXTERNAL_SPEC_ISSUER,
                    verifying_key,
                    &mut worker,
                )
                .is_err()
        );
        worker.shutdown().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn signed_external_spec_refuses_repository_symlink_to_identical_external_checkout() {
        use std::os::unix::fs::symlink;

        let mut worker = WorkerClient::start(&crate::worker::workspace_root()).unwrap();
        let fixture = signed_external_spec_fixture(&mut worker);
        let package_checkout = fixture._temporary.path().join("checkout");
        let external = tempfile::tempdir().unwrap();
        let external_checkout = external.path().join("checkout");
        std::fs::rename(&package_checkout, &external_checkout).unwrap();
        symlink(&external_checkout, &package_checkout).unwrap();
        let external_repo = external_checkout.join("fixtures/kotlin-2-1");
        let mut authority =
            EvidenceAuthority::open(&external_repo, &fixture.payload.repository_revision).unwrap();

        let result = authority.issue_external_spec_with_verifier(
            &fixture.specification_path,
            &fixture.request,
            Some(":/main"),
            TEST_EXTERNAL_SPEC_ISSUER,
            fixture.signing_key.verifying_key().to_bytes(),
            &mut worker,
        );
        assert!(result.is_err(), "a package repository symlink must refuse");
        worker.shutdown().unwrap();
    }

    #[test]
    fn external_spec_unknown_and_cross_session_handles_are_not_capabilities() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::write(temporary.path().join("tracked.txt"), "tracked").unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "tracked.txt"],
            vec![
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "authority fixture",
            ],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(temporary.path())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let revision = git_head(temporary.path()).unwrap();
        let authority = EvidenceAuthority::open(temporary.path(), &revision).unwrap();
        let unknown = ExternalSpecReceipt {
            session_id: authority.session_id,
            receipt_id: Uuid::new_v4(),
        };
        assert!(!authority.recognizes_external_spec(&unknown).unwrap());
        let cross_session = ExternalSpecReceipt {
            session_id: Uuid::new_v4(),
            receipt_id: unknown.receipt_id,
        };
        assert!(!authority.recognizes_external_spec(&cross_session).unwrap());
    }

    #[test]
    fn validation_route_is_backend_owned_module_scoped_and_contained() {
        let gradle_project = |module: &str| {
            json!({
                "buildSystem":"GRADLE",
                "buildLauncher":"./gradlew",
                "module":module,
                "sourceSet":"test",
                "testTasks":["test"],
                "projectModelHash":"model"
            })
        };
        let module = validation_route(
            ":service/test",
            &gradle_project(":service"),
            "p.ExampleTest",
            "checksValue",
        )
        .unwrap();
        assert_eq!(
            module.invocation,
            [":service:test", "--tests", "p.ExampleTest.checksValue"]
        );
        assert_eq!(
            module.report_root,
            PathBuf::from("service/build/test-results/test")
        );

        let root = validation_route(
            ":/test",
            &gradle_project(":"),
            "p.ExampleTest",
            "checksValue",
        )
        .unwrap();
        assert_eq!(
            root.invocation,
            [":test", "--tests", "p.ExampleTest.checksValue"]
        );
        assert_eq!(root.report_root, PathBuf::from("build/test-results/test"));

        let maven = validation_route(
            ":/test",
            &json!({
                "buildSystem":"MAVEN",
                "buildLauncher":"./mvnw",
                "module":":",
                "sourceSet":"test",
                "testTasks":["test"],
                "mavenTestLifecycle":"SUREFIRE",
                "projectModelHash":"maven-model"
            }),
            "p.ExampleTest",
            "checksValue",
        )
        .unwrap();
        assert_eq!(
            maven.invocation,
            ["-Dtest=p.ExampleTest#checksValue", "test"]
        );
        assert_eq!(maven.report_root, PathBuf::from("target/surefire-reports"));
        assert_eq!(maven.report_format, "JUNIT_XML");

        assert!(
            validation_route(
                ":..:outside/test",
                &gradle_project(":..:outside"),
                "p.T",
                "test"
            )
            .is_err()
        );
        assert!(
            validation_route(":service/test", &gradle_project(":other"), "p.T", "test").is_err()
        );
        let mut custom = gradle_project(":");
        custom["testTasks"] = json!(["integrationTest", "test"]);
        assert!(validation_route(":/test", &custom, "p.T", "test").is_err());
        let mut failsafe = json!({
            "buildSystem":"MAVEN","buildLauncher":"./mvnw","module":":",
            "sourceSet":"test","testTasks":["test"],
            "mavenTestLifecycle":"UNSUPPORTED_FAILSAFE","projectModelHash":"model"
        });
        assert!(validation_route(":/test", &failsafe, "p.T", "test").is_err());
        failsafe["mavenTestLifecycle"] = json!("SUREFIRE");
        failsafe["module"] = json!(":service");
        assert!(validation_route(":service/test", &failsafe, "p.T", "test").is_err());
    }

    #[test]
    fn source_change_and_path_escape_invalidate_authority_inputs() {
        let temporary = tempfile::tempdir().unwrap();
        let file = PathBuf::from("Source.kt");
        std::fs::write(temporary.path().join(&file), "fun stable() = 1\n").unwrap();
        let expected =
            BTreeMap::from([(file.clone(), canonical::hash_bytes(b"fun stable() = 1\n"))]);
        verify_sources_current(temporary.path(), &expected).unwrap();

        std::fs::write(temporary.path().join(&file), "fun stable() = 2\n").unwrap();
        let error = verify_sources_current(temporary.path(), &expected).unwrap_err();
        assert_eq!(error.code, ErrorCode::StaleRequiresReslice);
        assert!(safe_relative_path("../outside.kt").is_err());
        assert!(safe_relative_path("/absolute.kt").is_err());
    }

    #[test]
    fn testcase_parser_ignores_forged_cdata_output() {
        let report = r#"<testsuite>
          <testcase name="passed()" classname="Example"/>
          <testcase name="failed()" classname="Example"><failure/></testcase>
          <testcase name="errored()" classname="Example"><error/></testcase>
          <testcase name="skipped()" classname="Example"><skipped message="disabled"/></testcase>
          <system-out><![CDATA[<testcase name="forged()" classname="Expected"/>]]></system-out>
        </testsuite>"#;
        let records = testcase_records(report).unwrap();
        assert_eq!(
            records,
            [
                ("passed()", TestcaseOutcome::Passed),
                ("failed()", TestcaseOutcome::Failed),
                ("errored()", TestcaseOutcome::Error),
                ("skipped()", TestcaseOutcome::Skipped),
            ]
            .into_iter()
            .map(|(name, outcome)| TestcaseRecord {
                class_name: "Example".into(),
                name: name.into(),
                outcome,
            })
            .collect::<Vec<_>>()
        );

        let duplicates = testcase_records(
            r#"<testsuite>
              <testcase name="same()" classname="Example"/>
              <testcase name="same()" classname="Example"/>
            </testsuite>"#,
        )
        .unwrap();
        assert_eq!(duplicates.len(), 2, "duplicates must not be collapsed");

        let conflict = testcase_records(
            r#"<testsuite><testcase name="same()" classname="Example" status="passed"><skipped/></testcase></testsuite>"#,
        )
        .unwrap_err();
        assert_eq!(conflict.code, ErrorCode::IncompleteSemanticAnalysis);
    }

    #[test]
    fn compiler_identity_bridge_is_strict_and_never_name_only() {
        assert_eq!(
            strict_top_level_callable_id_query("com/acme/compute").unwrap(),
            "com.acme.compute"
        );
        for invalid in [
            "compute",
            "com.acme/compute",
            "com/acme/Owner.compute",
            "com/acme/Owner$Nested",
            "com//compute",
            "com/acme/1compute",
        ] {
            assert!(
                strict_top_level_callable_id_query(invalid).is_err(),
                "{invalid}"
            );
        }
        assert_eq!(
            select_unique_compiler_call(vec!["com/acme/compute".into()]).unwrap(),
            "com/acme/compute"
        );
        assert_eq!(
            select_unique_compiler_call(vec!["a/compute".into(), "b/compute".into()])
                .unwrap_err()
                .code,
            ErrorCode::AmbiguousTarget
        );

        let target = json!({
            "module":":","sourceSet":"main","package":"com.acme",
            "containingDeclarations":[],"declarationName":"compute",
            "parameterTypes":["kotlin/Int"],"returnType":"kotlin/Int",
            "receiverTypes":[],"contextReceiverTypes":[]
        });
        let decoy = json!({
            "module":":","sourceSet":"main","package":"com.decoy",
            "containingDeclarations":[],"declarationName":"compute",
            "parameterTypes":["kotlin/Int"],"returnType":"kotlin/Int",
            "receiverTypes":[],"contextReceiverTypes":[]
        });
        assert_ne!(
            target, decoy,
            "same terminal name is never identity equality"
        );
    }

    #[test]
    fn differential_validation_negative_private_authority_states() {
        let main_roots = vec![PathBuf::from("src/main/kotlin")];
        let test_roots = vec![PathBuf::from("src/test/kotlin")];
        let main_file = "src/main/kotlin/p/Main.kt".to_owned();
        let candidates = BTreeMap::from([(main_file.clone(), "candidate".into())]);
        assert_eq!(
            classify_candidate_files(
                std::slice::from_ref(&main_file),
                &candidates,
                &main_roots,
                &test_roots,
                &[],
                &[],
            )
            .unwrap()
            .production_files
            .len(),
            1
        );
        assert!(
            classify_candidate_files(
                &[main_file.clone(), main_file.clone()],
                &candidates,
                &main_roots,
                &test_roots,
                &[],
                &[],
            )
            .is_err()
        );
        assert!(
            classify_candidate_files(
                std::slice::from_ref(&main_file),
                &candidates,
                &[PathBuf::from("src")],
                &[PathBuf::from("src")],
                &[],
                &[],
            )
            .is_err()
        );
        let outside_file = "docs/Outside.kt".to_owned();
        assert!(
            classify_candidate_files(
                std::slice::from_ref(&outside_file),
                &BTreeMap::from([(outside_file.clone(), "candidate".into())]),
                &main_roots,
                &test_roots,
                &[],
                &[],
            )
            .is_err()
        );
        let generated_file = "build/generated/p/Generated.kt".to_owned();
        assert!(
            classify_candidate_files(
                std::slice::from_ref(&generated_file),
                &BTreeMap::from([(generated_file.clone(), "candidate".into())]),
                &[PathBuf::from("build/generated")],
                &test_roots,
                &[PathBuf::from("build/generated")],
                &[],
            )
            .is_err()
        );
        let test_file = "src/test/kotlin/p/Test.kt".to_owned();
        assert!(
            classify_candidate_files(
                std::slice::from_ref(&test_file),
                &BTreeMap::from([(test_file.clone(), "candidate".into())]),
                &main_roots,
                &test_roots,
                &[],
                &[],
            )
            .is_err()
        );

        assert_eq!(
            require_differential_outcomes(
                true,
                true,
                &[TestcaseOutcome::Passed],
                false,
                false,
                &[]
            )
            .unwrap_err()
            .code,
            ErrorCode::CompileFailed
        );
        assert_eq!(
            require_differential_outcomes(
                true,
                true,
                &[TestcaseOutcome::Passed],
                true,
                true,
                &[TestcaseOutcome::Passed],
            )
            .unwrap_err()
            .code,
            ErrorCode::PreconditionFailed
        );
        let unrelated = vec![
            TestcaseRecord {
                class_name: "Selected".into(),
                name: "test".into(),
                outcome: TestcaseOutcome::Passed,
            },
            TestcaseRecord {
                class_name: "Other".into(),
                name: "fails".into(),
                outcome: TestcaseOutcome::Failed,
            },
        ];
        assert!(reject_unrelated_test_failures(&unrelated, &BTreeSet::from([0])).is_err());

        let route = ValidationRoute {
            build_system: BuildSystem::Gradle,
            compilation: ":/test".into(),
            module: ":".into(),
            source_set: "test".into(),
            build_launcher: "./gradlew".into(),
            test_binary_class: "p.Test".into(),
            test_method: "checks".into(),
            test_selector: "p.Test.checks".into(),
            invocation: vec!["test".into(), "--tests".into(), "p.Test.checks".into()],
            report_format: "JUNIT_XML".into(),
            report_root: PathBuf::from("build/test-results/test"),
            project_model_hash: "test-model".into(),
        };
        let mut overlay = CandidateOverlay {
            revision: "revision".into(),
            thread_fingerprint: "thread".into(),
            test_fingerprint: Some("test".into()),
            production_project_model_hash: "main-model".into(),
            test_compilation: ":/test".into(),
            test_project_model_hash: "test-model".into(),
            test_compile_task: ":compileTestKotlin".into(),
            route: Some(route.clone()),
            candidates,
            production_files: BTreeSet::from([main_file]),
            test_files: BTreeSet::new(),
            affected_callables: BTreeSet::from(["p.Main".into()]),
            oracle_rejections: Vec::new(),
            overlay_hash: String::new(),
        };
        overlay.overlay_hash = canonical::hash(&(
            &overlay.revision,
            &overlay.thread_fingerprint,
            overlay.test_fingerprint.as_ref().unwrap(),
            &overlay.production_project_model_hash,
            &overlay.test_compilation,
            &overlay.test_project_model_hash,
            &overlay.test_compile_task,
            canonical::hash(&route).unwrap(),
            &overlay.candidates,
            &overlay.production_files,
            &overlay.test_files,
            &overlay.affected_callables,
        ))
        .unwrap();
        validate_differential_overlay_state(
            &overlay,
            "revision",
            "main-model",
            "test-model",
            ":compileTestKotlin",
            &route,
        )
        .unwrap();
        let mut stale = overlay.clone();
        stale.revision = "new".into();
        assert_eq!(
            validate_differential_overlay_state(
                &stale,
                "revision",
                "main-model",
                "test-model",
                ":compileTestKotlin",
                &route
            )
            .unwrap_err()
            .code,
            ErrorCode::StaleRequiresReslice
        );
        assert_eq!(
            validate_differential_overlay_state(
                &overlay,
                "revision",
                "changed",
                "test-model",
                ":compileTestKotlin",
                &route
            )
            .unwrap_err()
            .code,
            ErrorCode::ProjectModelChanged
        );
        let mut changed_route = route.clone();
        changed_route.test_method = "other".into();
        assert_eq!(
            validate_differential_overlay_state(
                &overlay,
                "revision",
                "main-model",
                "test-model",
                ":compileTestKotlin",
                &changed_route
            )
            .unwrap_err()
            .code,
            ErrorCode::ProjectModelChanged
        );
        let mut forged = overlay.clone();
        forged.overlay_hash = "forged".into();
        assert_eq!(
            validate_differential_overlay_state(
                &forged,
                "revision",
                "main-model",
                "test-model",
                ":compileTestKotlin",
                &route
            )
            .unwrap_err()
            .code,
            ErrorCode::PreconditionFailed
        );
        let mut zero = overlay;
        zero.production_files.clear();
        assert_eq!(
            validate_differential_overlay_state(
                &zero,
                "revision",
                "main-model",
                "test-model",
                ":compileTestKotlin",
                &route
            )
            .unwrap_err()
            .code,
            ErrorCode::StaleRequiresReslice
        );
    }

    #[test]
    fn integer_operations_that_can_throw_are_not_in_the_pure_allow_list() {
        assert!(known_pure_callable("kotlin/Int.plus"));
        assert!(!known_pure_callable("kotlin/Int.div"));
        assert!(!known_pure_callable("kotlin/Long.rem"));
    }

    #[test]
    fn exact_nested_calls_without_argument_mapping_are_conditional_not_proven() {
        let resolution = json!({
            "resolvedCalls":[
                {"symbol":"kotlin/test/assertEquals","start":10,"end":90},
                {"symbol":"p/transform","start":30,"end":80},
                {"symbol":"p/context","start":55,"end":70}
            ]
        });
        let conditional = conditional_oracle_evidence(&resolution, "p/transform", "p/context")
            .unwrap()
            .expect("exact nesting with missing mapping is actionable conditional evidence");
        assert!(!conditional.evidence_fingerprint.is_empty());

        let contradicted = json!({
            "resolvedCalls":[
                {"symbol":"kotlin/test/assertEquals","start":10,"end":90,
                 "argumentToParameter":[{"parameter":"actual","argumentStart":44}]},
                {"symbol":"p/transform","start":30,"end":80},
                {"symbol":"p/context","start":55,"end":70}
            ]
        });
        assert!(
            conditional_oracle_evidence(&contradicted, "p/transform", "p/context")
                .unwrap()
                .is_none(),
            "conflicting exact mapping must not be downgraded to conditional"
        );

        let ambiguous = json!({
            "resolvedCalls":[
                {"symbol":"kotlin/test/assertEquals","start":10,"end":90},
                {"symbol":"p/transform","start":30,"end":80},
                {"symbol":"p/transform","start":32,"end":75},
                {"symbol":"p/context","start":55,"end":70}
            ]
        });
        assert!(
            conditional_oracle_evidence(&ambiguous, "p/transform", "p/context")
                .unwrap()
                .is_none(),
            "ambiguous target occurrence must remain non-actionable"
        );
    }
}
