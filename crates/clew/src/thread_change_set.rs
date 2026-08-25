//! Pure, bounded before/after Kotlin change-set coverage.
//!
//! This module compares two already verified thread callable impacts.  It does
//! not read a repository, open managed state, execute a command, or publish an
//! object.  The result contains the exact two CAS objects a managed service
//! must publish (the normalized inert coverage document and private evidence),
//! the retained proof closure, and a bounded path-free public projection.

use crate::canonical;
use crate::cas::{CAS_OBJECT_SCHEMA, CasObject};
use crate::error::{ClewError, ErrorCode};
use crate::thread::{ThreadAuthority, ThreadMemberBinding};
use crate::thread_callables::{
    self, CallableMemberBinding, DeclarationKind, PreparedCallableFactSet, PreparedCasObject,
    RelationshipAuthority,
};
use crate::thread_impact::{
    self, ImpactFinding, ImpactFindingAuthority, ImpactFindingDetail, ImpactObligation,
    ImpactObligationCode, ImpactSide, KotlinImpactSubject, PreparedThreadImpact,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA: &str =
    "codeclew-kotlin-change-coverage-document/1.0";
pub const KOTLIN_CHANGE_COVERAGE_EVIDENCE_SCHEMA: &str =
    "codeclew-kotlin-change-coverage-evidence/1.0";
pub const KOTLIN_CHANGE_COVERAGE_AUTHORITY_SCHEMA: &str =
    "codeclew-kotlin-change-coverage-authority/1.0";
pub const KOTLIN_CHANGE_COVERAGE_PROJECTION_SCHEMA: &str =
    "codeclew-kotlin-change-coverage-projection/1.0";
pub const KOTLIN_CHANGE_RULES_SCHEMA: &str = "codeclew-kotlin-change-rules/1.0";

pub const MAX_CHANGE_MEMBER_CORRESPONDENCES: usize = 2;
pub const MAX_CHANGE_COVERAGE_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CHANGE_COVERAGE_ENTRIES: usize = 8_192;
pub const MAX_CHANGE_OBSERVATIONS: usize = 4_096;
pub const MAX_CHANGE_OBLIGATIONS: usize = 4_096;
pub const MAX_CHANGE_RESULT_ROWS: usize = 8_192;
pub const MAX_CHANGE_DERIVED_CAS_OBJECTS: usize = 2;
pub const MAX_CHANGE_RETAINED_CLOSURE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CHANGE_STDOUT_BYTES: usize = 64 * 1024;
const CHANGE_STDOUT_ENVELOPE_RESERVE: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSetBudgets {
    pub max_member_correspondences: usize,
    pub max_coverage_document_bytes: usize,
    pub max_coverage_entries: usize,
    pub max_observations: usize,
    pub max_obligations: usize,
    pub max_result_rows: usize,
    pub max_derived_cas_objects: usize,
    pub max_retained_closure_bytes: usize,
    pub max_stdout_bytes: usize,
}

impl ChangeSetBudgets {
    pub fn frozen() -> Self {
        Self {
            max_member_correspondences: MAX_CHANGE_MEMBER_CORRESPONDENCES,
            max_coverage_document_bytes: MAX_CHANGE_COVERAGE_DOCUMENT_BYTES,
            max_coverage_entries: MAX_CHANGE_COVERAGE_ENTRIES,
            max_observations: MAX_CHANGE_OBSERVATIONS,
            max_obligations: MAX_CHANGE_OBLIGATIONS,
            max_result_rows: MAX_CHANGE_RESULT_ROWS,
            max_derived_cas_objects: MAX_CHANGE_DERIVED_CAS_OBJECTS,
            max_retained_closure_bytes: MAX_CHANGE_RETAINED_CLOSURE_BYTES,
            max_stdout_bytes: MAX_CHANGE_STDOUT_BYTES,
        }
    }

    fn validate(&self) -> Result<(), ClewError> {
        if self != &Self::frozen() {
            return Err(invalid(
                "Kotlin change-set budgets differ from the frozen profile",
            ));
        }
        Ok(())
    }
}

impl Default for ChangeSetBudgets {
    fn default() -> Self {
        Self::frozen()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberCorrespondence {
    pub before_member_alias: String,
    pub after_member_alias: String,
}

#[derive(Debug, Clone)]
pub struct ThreadChangeSetRequest {
    pub member_correspondence: Vec<MemberCorrespondence>,
    pub coverage_document: Vec<u8>,
    pub validator_runtime: ValidatorRuntimeAuthority,
    pub budgets: ChangeSetBudgets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidatorRuntimeAuthority {
    pub runtime_key: String,
    pub runtime_mode: crate::runtime::RuntimeMode,
    pub manifest_digest: String,
}

#[derive(Clone, Copy)]
pub struct VerifiedChangeSide<'a> {
    pub thread: &'a ThreadAuthority,
    pub fact_set: &'a PreparedCallableFactSet,
    pub impact: &'a PreparedThreadImpact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KotlinChangeCode {
    KcdCallableAdded,
    KcdCallableRemoved,
    KcdOverloadSetChanged,
    KcdJvmDescriptorChanged,
    KcdParameterTypesChanged,
    KcdReturnTypeChanged,
    KcdReceiverTypeChanged,
    KcdNullabilityChanged,
    KcdTypeParameterBoundsChanged,
    KcdVisibilityChanged,
    KcdModalityChanged,
    KcdOverrideStatusChanged,
    KcdUnsupportedComparison,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChangeSetStatus {
    ValidatedConditional,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoverageHandlingKind {
    Action,
    ExternalWork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageHandling {
    pub kind: CoverageHandlingKind,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerificationCategory {
    MemberAuthority,
    StructuralComparison,
    CompilerBoundary,
    RelationshipAuthority,
    UpstreamObligation,
    ComparisonBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageDocumentEntry {
    pub target_id: String,
    pub required_categories: Vec<VerificationCategory>,
    pub handling: CoverageHandling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinChangeCoverageDocument {
    pub schema: String,
    pub entries: Vec<CoverageDocumentEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChangeMemberRole {
    Provider,
    Consumer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChangeObligationPhase {
    Before,
    After,
    Comparison,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ComparisonObligationCode {
    DisambiguateOverloadSet,
    VerifyUnsupportedComparison,
    VerifyTruncatedImpactEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChangeObligationDetail {
    Upstream {
        phase: ChangeObligationPhase,
        obligation: ImpactObligation,
    },
    Comparison {
        phase: ChangeObligationPhase,
        side: ImpactSide,
        code: ComparisonObligationCode,
        before_finding_ids: Vec<String>,
        after_finding_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeObligation {
    pub obligation_id: String,
    pub detail: ChangeObligationDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeDeclarationEvidence {
    pub finding_id: String,
    pub fact_id: String,
    pub member_alias: String,
    pub authority: ImpactFindingAuthority,
    pub declaration_kind: DeclarationKind,
    pub symbol_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler_callable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler_class_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_digest: Option<String>,
    pub projected_shape: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinChangeObservation {
    pub observation_id: String,
    pub code: KotlinChangeCode,
    pub side: ImpactSide,
    pub before: Vec<ChangeDeclarationEvidence>,
    pub after: Vec<ChangeDeclarationEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoverageTargetKind {
    DeclaredMember,
    Observation,
    Obligation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CoverageTargetSubject {
    DeclaredMember {
        before_member_alias: String,
        after_member_alias: String,
        service_alias: String,
        role: ChangeMemberRole,
    },
    Observation {
        observation_id: String,
        code: KotlinChangeCode,
        side: ImpactSide,
    },
    Obligation {
        obligation_id: String,
        phase: ChangeObligationPhase,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageTarget {
    pub target_id: String,
    pub target_kind: CoverageTargetKind,
    pub required_categories: Vec<VerificationCategory>,
    pub subject: CoverageTargetSubject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageValidationRow {
    pub target_id: String,
    pub target_kind: CoverageTargetKind,
    pub required_categories: Vec<VerificationCategory>,
    pub covered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handling: Option<CoverageHandling>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeRuntimeAuthority {
    pub before_member_alias: String,
    pub after_member_alias: String,
    pub runtime_key: String,
    pub runtime_mode: crate::runtime::RuntimeMode,
    pub compilation_authorities: Vec<ChangeCompilationAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeCompilationAuthority {
    pub compilation_id: String,
    pub semantic_authority: String,
    pub extractor_id: String,
    pub adapter_digest: String,
    pub runtime_digest: String,
    pub before_descriptor_coverage: thread_callables::GraphCoverage,
    pub after_descriptor_coverage: thread_callables::GraphCoverage,
    pub before_relation_coverage: thread_callables::GraphCoverage,
    pub after_relation_coverage: thread_callables::GraphCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSideAuthority {
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub thread_semantic_digest: String,
    pub fact_set_id: String,
    pub fact_set_authority_digest: String,
    pub fact_set_binding_digest: String,
    pub profile_digest: String,
    pub impact_id: String,
    pub impact_authority_digest: String,
    pub impact_binding_digest: String,
    pub pair_id: String,
    pub callable_id: String,
    pub relationship_authority: RelationshipAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinChangeCoverageEvidence {
    pub schema: String,
    pub comparison_digest: String,
    pub validation_binding_digest: String,
    pub rules_digest: String,
    pub before: ChangeSideAuthority,
    pub after: ChangeSideAuthority,
    pub runtime_authorities: Vec<ChangeRuntimeAuthority>,
    pub validator_runtime: ValidatorRuntimeAuthority,
    pub member_correspondence: Vec<MemberCorrespondence>,
    pub observations: Vec<KotlinChangeObservation>,
    pub obligations: Vec<ChangeObligation>,
    pub targets: Vec<CoverageTarget>,
    pub coverage_document: KotlinChangeCoverageDocument,
    pub validation_rows: Vec<CoverageValidationRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadChangeSetAuthority {
    pub schema: String,
    pub authority_digest: String,
    pub comparison_digest: String,
    pub validation_binding_digest: String,
    pub rules_digest: String,
    pub before: ChangeSideAuthority,
    pub after: ChangeSideAuthority,
    pub runtime_authorities: Vec<ChangeRuntimeAuthority>,
    pub validator_runtime: ValidatorRuntimeAuthority,
    pub member_correspondence: Vec<MemberCorrespondence>,
    pub status: ChangeSetStatus,
    pub member_target_count: usize,
    pub observation_count: usize,
    pub obligation_count: usize,
    pub target_count: usize,
    pub covered_target_count: usize,
    pub missing_target_count: usize,
    pub public_missing_targets: Vec<CoverageTargetProjection>,
    pub public_covered_target_ids: Vec<String>,
    pub public_covered_targets_truncated: bool,
    pub coverage_document_ref: CasObject,
    pub evidence_ref: CasObject,
    pub direct_cas_closure: Vec<CasObject>,
    pub retained_cas_bytes: usize,
    pub new_derived_cas_object_count: usize,
    pub budgets: ChangeSetBudgets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageTargetProjection {
    pub target_id: String,
    pub target_kind: CoverageTargetKind,
    pub required_categories: Vec<VerificationCategory>,
    pub subject: CoverageTargetSubject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadChangeSetProjection {
    pub schema: String,
    pub change_set_id: String,
    pub authority_digest: String,
    pub comparison_digest: String,
    pub validation_binding_digest: String,
    pub rules_digest: String,
    pub before_thread_id: String,
    pub after_thread_id: String,
    pub callable_id: String,
    pub status: ChangeSetStatus,
    pub member_target_count: usize,
    pub observation_count: usize,
    pub obligation_count: usize,
    pub target_count: usize,
    pub covered_target_count: usize,
    pub missing_target_count: usize,
    pub missing_targets: Vec<CoverageTargetProjection>,
    pub covered_target_ids: Vec<String>,
    pub covered_targets_truncated: bool,
    pub coverage_document_ref: CasObject,
    pub evidence_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedThreadChangeSet {
    pub authority: ThreadChangeSetAuthority,
    pub evidence: KotlinChangeCoverageEvidence,
    pub coverage_document: KotlinChangeCoverageDocument,
    pub coverage_document_object: PreparedCasObject,
    pub evidence_object: PreparedCasObject,
    pub authority_bytes: Vec<u8>,
    pub projection: ThreadChangeSetProjection,
    pub projection_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComparisonMaterial<'a> {
    schema: &'static str,
    before: &'a ChangeSideAuthority,
    after: &'a ChangeSideAuthority,
    runtime_authorities: &'a [ChangeRuntimeAuthority],
    member_correspondence: &'a [MemberCorrespondence],
    rules_digest: &'a str,
    budgets: &'a ChangeSetBudgets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationBindingMaterial<'a> {
    schema: &'static str,
    comparison_digest: &'a str,
    coverage_document_ref: &'a CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetMaterial<'a> {
    schema: &'static str,
    comparison_digest: &'a str,
    target_kind: CoverageTargetKind,
    required_categories: &'a [VerificationCategory],
    subject: &'a CoverageTargetSubject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationMaterial<'a> {
    schema: &'static str,
    code: KotlinChangeCode,
    side: ImpactSide,
    before: &'a [ChangeDeclarationEvidence],
    after: &'a [ChangeDeclarationEvidence],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObligationMaterial<'a> {
    schema: &'static str,
    detail: &'a ChangeObligationDetail,
}

/// Verify both immutable parents and construct the complete publication set.
pub fn build(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    request: ThreadChangeSetRequest,
) -> Result<PreparedThreadChangeSet, ClewError> {
    verify_side(before)?;
    verify_side(after)?;
    build_from_verified(before, after, request)
}

/// Construct after the managed service has loaded and verified both parent
/// roots.  Callers outside the crate should use [`build`].
pub(crate) fn build_from_verified(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    mut request: ThreadChangeSetRequest,
) -> Result<PreparedThreadChangeSet, ClewError> {
    request.budgets.validate()?;
    let correspondence = normalize_correspondence(
        before,
        after,
        std::mem::take(&mut request.member_correspondence),
        &request.budgets,
    )?;
    let before_authority = side_authority(before)?;
    let after_authority = side_authority(after)?;
    if before_authority.callable_id != after_authority.callable_id {
        return Err(invalid(
            "before and after impacts must select the same callable family",
        ));
    }
    if before_authority.relationship_authority != RelationshipAuthority::DeclaredTopology
        || after_authority.relationship_authority != RelationshipAuthority::DeclaredTopology
    {
        return Err(invalid(
            "S3K v1 requires DECLARED_TOPOLOGY impact authority",
        ));
    }
    if before_authority.profile_digest != after_authority.profile_digest {
        return Err(invalid(
            "before and after fact sets must use the same Kotlin profile",
        ));
    }
    let runtime_authorities = validate_member_authority(before, after, &correspondence)?;
    validate_validator_runtime(&request.validator_runtime)?;
    let rules_digest = rules_digest()?;
    let comparison_digest = canonical::hash(&ComparisonMaterial {
        schema: "codeclew-kotlin-change-comparison/1.0",
        before: &before_authority,
        after: &after_authority,
        runtime_authorities: &runtime_authorities,
        member_correspondence: &correspondence,
        rules_digest: &rules_digest,
        budgets: &request.budgets,
    })
    .map_err(internal)?;

    let (mut observations, mut comparison_obligations) = compare_impacts(before, after)?;
    observations.sort_by(observation_sort_key);
    enforce_at_most(
        observations.len(),
        request.budgets.max_observations,
        "Kotlin change observations exceed the frozen 4,096 bound",
    )?;
    let obligations = collect_obligations(
        before.impact,
        after.impact,
        &mut comparison_obligations,
        &request.budgets,
    )?;
    let targets = build_targets(
        &comparison_digest,
        before,
        &correspondence,
        &observations,
        &obligations,
        &request.budgets,
    )?;
    let (coverage_document, coverage_document_bytes) =
        normalize_coverage_document(&request.coverage_document, &request.budgets)?;
    let validation_rows = validate_coverage(&targets, &coverage_document)?;
    let missing_target_count = validation_rows.iter().filter(|row| !row.covered).count();
    let covered_target_count = validation_rows.len() - missing_target_count;
    let status = if missing_target_count == 0 {
        ChangeSetStatus::ValidatedConditional
    } else {
        ChangeSetStatus::Incomplete
    };
    let coverage_document_ref = CasObject::for_bytes(
        KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA,
        &coverage_document_bytes,
    )?;
    let coverage_document_object = PreparedCasObject {
        reference: coverage_document_ref.clone(),
        bytes: coverage_document_bytes,
    };
    let validation_binding_digest = canonical::hash(&ValidationBindingMaterial {
        schema: "codeclew-kotlin-change-validation-binding/1.0",
        comparison_digest: &comparison_digest,
        coverage_document_ref: &coverage_document_ref,
    })
    .map_err(internal)?;
    let evidence = KotlinChangeCoverageEvidence {
        schema: KOTLIN_CHANGE_COVERAGE_EVIDENCE_SCHEMA.into(),
        comparison_digest: comparison_digest.clone(),
        validation_binding_digest: validation_binding_digest.clone(),
        rules_digest: rules_digest.clone(),
        before: before_authority.clone(),
        after: after_authority.clone(),
        runtime_authorities: runtime_authorities.clone(),
        validator_runtime: request.validator_runtime.clone(),
        member_correspondence: correspondence.clone(),
        observations,
        obligations,
        targets: targets.clone(),
        coverage_document: coverage_document.clone(),
        validation_rows,
    };
    let evidence_bytes = canonical::bytes(&evidence).map_err(internal)?;
    let evidence_ref =
        CasObject::for_bytes(KOTLIN_CHANGE_COVERAGE_EVIDENCE_SCHEMA, &evidence_bytes)?;
    let evidence_object = PreparedCasObject {
        reference: evidence_ref.clone(),
        bytes: evidence_bytes,
    };
    let (direct_cas_closure, retained_cas_bytes) = direct_closure(
        before,
        after,
        &[coverage_document_ref.clone(), evidence_ref.clone()],
        &request.budgets,
    )?;
    let public_missing_targets = targets
        .iter()
        .zip(&evidence.validation_rows)
        .filter(|(_, row)| !row.covered)
        .map(|(target, _)| target_projection(target))
        .collect::<Vec<_>>();
    let public_covered_target_ids = evidence
        .validation_rows
        .iter()
        .filter(|row| row.covered)
        .map(|row| row.target_id.clone())
        .collect::<Vec<_>>();
    let member_target_count = correspondence.len();
    let mut authority = ThreadChangeSetAuthority {
        schema: KOTLIN_CHANGE_COVERAGE_AUTHORITY_SCHEMA.into(),
        authority_digest: String::new(),
        comparison_digest,
        validation_binding_digest,
        rules_digest,
        before: before_authority,
        after: after_authority,
        runtime_authorities,
        validator_runtime: request.validator_runtime,
        member_correspondence: correspondence,
        status,
        member_target_count,
        observation_count: evidence.observations.len(),
        obligation_count: evidence.obligations.len(),
        target_count: targets.len(),
        covered_target_count,
        missing_target_count,
        public_missing_targets,
        public_covered_target_ids,
        public_covered_targets_truncated: false,
        coverage_document_ref,
        evidence_ref,
        direct_cas_closure,
        retained_cas_bytes,
        new_derived_cas_object_count: 2,
        budgets: request.budgets,
    };
    let (projection, projection_bytes) = bounded_authority_projection(&mut authority)?;
    let authority_bytes = canonical::bytes(&authority).map_err(internal)?;
    Ok(PreparedThreadChangeSet {
        authority,
        evidence,
        coverage_document,
        coverage_document_object,
        evidence_object,
        authority_bytes,
        projection,
        projection_bytes,
    })
}

/// Reconstruct a prepared result and reject any parent, document, evidence,
/// closure, authority, or projection substitution.
pub fn verify_prepared(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    prepared: &PreparedThreadChangeSet,
) -> Result<(), ClewError> {
    verify_side(before)?;
    verify_side(after)?;
    verify_prepared_from_verified(before, after, prepared)
}

pub(crate) fn verify_prepared_from_verified(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    prepared: &PreparedThreadChangeSet,
) -> Result<(), ClewError> {
    let expected = build_from_verified(
        before,
        after,
        ThreadChangeSetRequest {
            member_correspondence: prepared.authority.member_correspondence.clone(),
            coverage_document: prepared.coverage_document_object.bytes.clone(),
            validator_runtime: prepared.authority.validator_runtime.clone(),
            budgets: prepared.authority.budgets.clone(),
        },
    )
    .map_err(|_| corrupt("prepared Kotlin change set cannot be reconstructed"))?;
    if &expected != prepared {
        return Err(corrupt(
            "prepared Kotlin change-set authority/evidence/projection was substituted",
        ));
    }
    Ok(())
}

/// Produce the compact public projection.  [`build`] additionally prunes the
/// optional covered-target prefix to the stdout budget.
pub fn project(authority: &ThreadChangeSetAuthority) -> ThreadChangeSetProjection {
    ThreadChangeSetProjection {
        schema: KOTLIN_CHANGE_COVERAGE_PROJECTION_SCHEMA.into(),
        change_set_id: format!("thread-coverage:{}", authority.authority_digest),
        authority_digest: authority.authority_digest.clone(),
        comparison_digest: authority.comparison_digest.clone(),
        validation_binding_digest: authority.validation_binding_digest.clone(),
        rules_digest: authority.rules_digest.clone(),
        before_thread_id: authority.before.thread_id.clone(),
        after_thread_id: authority.after.thread_id.clone(),
        callable_id: authority.after.callable_id.clone(),
        status: authority.status,
        member_target_count: authority.member_target_count,
        observation_count: authority.observation_count,
        obligation_count: authority.obligation_count,
        target_count: authority.target_count,
        covered_target_count: authority.covered_target_count,
        missing_target_count: authority.missing_target_count,
        missing_targets: authority.public_missing_targets.clone(),
        covered_target_ids: authority.public_covered_target_ids.clone(),
        covered_targets_truncated: authority.public_covered_targets_truncated,
        coverage_document_ref: authority.coverage_document_ref.clone(),
        evidence_ref: authority.evidence_ref.clone(),
    }
}

fn verify_side(side: VerifiedChangeSide<'_>) -> Result<(), ClewError> {
    side.thread.verify()?;
    thread_callables::verify_prepared(side.fact_set)?;
    thread_impact::verify_prepared(side.fact_set, side.impact)?;
    if side.fact_set.authority.thread_id != side.thread.thread_id
        || side.fact_set.authority.thread_authority_digest != side.thread.authority_digest
        || side.impact.authority.fact_set_authority_digest
            != side.fact_set.authority.authority_digest
    {
        return Err(corrupt(
            "Kotlin change-set parent thread/fact-set/impact authority is inconsistent",
        ));
    }
    Ok(())
}

fn side_authority(side: VerifiedChangeSide<'_>) -> Result<ChangeSideAuthority, ClewError> {
    let callable_id = match &side.impact.authority.request.subject {
        KotlinImpactSubject::CallableFamily { callable_id } => callable_id.clone(),
        KotlinImpactSubject::FullSymbol { .. } | KotlinImpactSubject::Token { .. } => {
            return Err(invalid(
                "S3K v1 accepts only CALLABLE_FAMILY impact subjects",
            ));
        }
    };
    if side.impact.evidence.selection.subject != side.impact.authority.request.subject
        || side.impact.evidence.selection.relationship_authority
            != side.impact.authority.relationship_authority
    {
        return Err(corrupt("Kotlin impact selection authority was substituted"));
    }
    Ok(ChangeSideAuthority {
        thread_id: side.thread.thread_id.clone(),
        thread_authority_digest: side.thread.authority_digest.clone(),
        thread_semantic_digest: side.thread.semantic_digest.clone(),
        fact_set_id: side.fact_set.projection.fact_set_id.clone(),
        fact_set_authority_digest: side.fact_set.authority.authority_digest.clone(),
        fact_set_binding_digest: side.fact_set.authority.binding_digest.clone(),
        profile_digest: side.fact_set.authority.profile_digest.clone(),
        impact_id: format!("thread-impact:{}", side.impact.authority.authority_digest),
        impact_authority_digest: side.impact.authority.authority_digest.clone(),
        impact_binding_digest: side.impact.authority.binding_digest.clone(),
        pair_id: side.impact.authority.pair.pair_id.clone(),
        callable_id,
        relationship_authority: side.impact.authority.relationship_authority,
    })
}

fn normalize_correspondence(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    mut correspondence: Vec<MemberCorrespondence>,
    budgets: &ChangeSetBudgets,
) -> Result<Vec<MemberCorrespondence>, ClewError> {
    let expected_before = BTreeSet::from([
        before.impact.authority.pair.provider_member.as_str(),
        before.impact.authority.pair.consumer_member.as_str(),
    ]);
    let expected_after = BTreeSet::from([
        after.impact.authority.pair.provider_member.as_str(),
        after.impact.authority.pair.consumer_member.as_str(),
    ]);
    enforce_exact(
        correspondence.len(),
        budgets.max_member_correspondences,
        "member correspondence must contain exactly the two impact-pair members",
    )?;
    if expected_before.len() != MAX_CHANGE_MEMBER_CORRESPONDENCES
        || expected_after.len() != MAX_CHANGE_MEMBER_CORRESPONDENCES
    {
        return Err(invalid(
            "member correspondence must be a total mapping of both impact pairs",
        ));
    }
    correspondence.sort();
    let mut before_aliases = BTreeSet::new();
    let mut after_aliases = BTreeSet::new();
    for mapping in &correspondence {
        if !safe_alias(&mapping.before_member_alias)
            || !safe_alias(&mapping.after_member_alias)
            || !before_aliases.insert(mapping.before_member_alias.as_str())
            || !after_aliases.insert(mapping.after_member_alias.as_str())
        {
            return Err(invalid(
                "member correspondence must be a sorted unique bijection",
            ));
        }
    }
    if before_aliases != expected_before || after_aliases != expected_after {
        return Err(invalid(
            "member correspondence omits or invents a compared pair member",
        ));
    }
    Ok(correspondence)
}

fn validate_member_authority(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    correspondence: &[MemberCorrespondence],
) -> Result<Vec<ChangeRuntimeAuthority>, ClewError> {
    let mut authorities = Vec::with_capacity(correspondence.len());
    for mapping in correspondence {
        let before_thread_member = thread_member(before.thread, &mapping.before_member_alias)?;
        let after_thread_member = thread_member(after.thread, &mapping.after_member_alias)?;
        let before_fact_member = fact_member(before.fact_set, &mapping.before_member_alias)?;
        let after_fact_member = fact_member(after.fact_set, &mapping.after_member_alias)?;
        if before_thread_member.service_alias != after_thread_member.service_alias
            || before_thread_member.session.repository_key
                != after_thread_member.session.repository_key
            || before_thread_member.session.language != crate::session::SessionLanguage::Kotlin
            || after_thread_member.session.language != crate::session::SessionLanguage::Kotlin
            || before_thread_member.session.language != after_thread_member.session.language
            || before_thread_member.session.compilations != after_thread_member.session.compilations
            || before_thread_member.session.runtime_key != after_thread_member.session.runtime_key
            || before_thread_member.session.runtime_mode != after_thread_member.session.runtime_mode
            || before_fact_member.service_alias != before_thread_member.service_alias
            || after_fact_member.service_alias != after_thread_member.service_alias
            || before_fact_member.session_id != before_thread_member.session.session_id
            || after_fact_member.session_id != after_thread_member.session.session_id
            || before_fact_member.session_authority_digest
                != before_thread_member.session.authority_digest
            || after_fact_member.session_authority_digest
                != after_thread_member.session.authority_digest
            || before_fact_member.base_revision != before_thread_member.session.base_revision
            || after_fact_member.base_revision != after_thread_member.session.base_revision
            || before_fact_member.repository_namespace != after_fact_member.repository_namespace
        {
            return Err(invalid(
                "member correspondence changes repository/service/language/compilation/runtime authority",
            ));
        }
        let before_compiler_identities = compilation_identities(before_fact_member);
        let after_compiler_identities = compilation_identities(after_fact_member);
        if before_compiler_identities != after_compiler_identities {
            return Err(invalid(
                "member correspondence changes extractor, adapter, or compiler authority",
            ));
        }
        authorities.push(ChangeRuntimeAuthority {
            before_member_alias: mapping.before_member_alias.clone(),
            after_member_alias: mapping.after_member_alias.clone(),
            runtime_key: before_thread_member.session.runtime_key.clone(),
            runtime_mode: before_thread_member.session.runtime_mode,
            compilation_authorities: combined_compilation_authorities(
                before_fact_member,
                after_fact_member,
            )?,
        });
    }
    validate_pair_roles(before, after, correspondence)?;
    Ok(authorities)
}

fn validate_validator_runtime(validator: &ValidatorRuntimeAuthority) -> Result<(), ClewError> {
    if !sha256_digest(&validator.runtime_key) || !sha256_digest(&validator.manifest_digest) {
        return Err(invalid(
            "validator runtime key and manifest digest must be SHA-256 authorities",
        ));
    }
    Ok(())
}

fn validate_pair_roles(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    correspondence: &[MemberCorrespondence],
) -> Result<(), ClewError> {
    let mappings = correspondence
        .iter()
        .map(|mapping| {
            (
                mapping.before_member_alias.as_str(),
                mapping.after_member_alias.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if mappings
        .get(before.impact.authority.pair.provider_member.as_str())
        .copied()
        != Some(after.impact.authority.pair.provider_member.as_str())
        || mappings
            .get(before.impact.authority.pair.consumer_member.as_str())
            .copied()
            != Some(after.impact.authority.pair.consumer_member.as_str())
    {
        return Err(invalid(
            "member correspondence must preserve provider and consumer roles",
        ));
    }
    Ok(())
}

fn thread_member<'a>(
    thread: &'a ThreadAuthority,
    alias: &str,
) -> Result<&'a ThreadMemberBinding, ClewError> {
    thread
        .members
        .iter()
        .find(|member| member.member_alias == alias)
        .ok_or_else(|| invalid("member correspondence references an unknown thread member"))
}

fn fact_member<'a>(
    fact_set: &'a PreparedCallableFactSet,
    alias: &str,
) -> Result<&'a CallableMemberBinding, ClewError> {
    fact_set
        .authority
        .members
        .iter()
        .find(|member| member.member_alias == alias)
        .ok_or_else(|| corrupt("fact set omits a declared thread member"))
}

fn compilation_identities(
    member: &CallableMemberBinding,
) -> Vec<(String, String, String, String, String)> {
    member
        .compilations
        .iter()
        .map(|compilation| {
            (
                compilation.compilation_id.clone(),
                compilation.semantic_authority.clone(),
                compilation.extractor_id.clone(),
                compilation.adapter_digest.clone(),
                compilation.runtime_digest.clone(),
            )
        })
        .collect()
}

fn combined_compilation_authorities(
    before: &CallableMemberBinding,
    after: &CallableMemberBinding,
) -> Result<Vec<ChangeCompilationAuthority>, ClewError> {
    before
        .compilations
        .iter()
        .zip(&after.compilations)
        .map(|(before, after)| {
            if before.compilation_id != after.compilation_id {
                return Err(invalid("compilation correspondence is not canonical"));
            }
            Ok(ChangeCompilationAuthority {
                compilation_id: before.compilation_id.clone(),
                semantic_authority: before.semantic_authority.clone(),
                extractor_id: before.extractor_id.clone(),
                adapter_digest: before.adapter_digest.clone(),
                runtime_digest: before.runtime_digest.clone(),
                before_descriptor_coverage: before.descriptor_coverage,
                after_descriptor_coverage: after.descriptor_coverage,
                before_relation_coverage: before.relation_coverage,
                after_relation_coverage: after.relation_coverage,
            })
        })
        .collect()
}

fn rules_digest() -> Result<String, ClewError> {
    rules_digest_for(&KotlinChangeRules {
        schema: KOTLIN_CHANGE_RULES_SCHEMA,
        rows: vec![
            rule(
                ChangeRuleInput::CallableFamilyAppears,
                KotlinChangeCode::KcdCallableAdded,
            ),
            rule(
                ChangeRuleInput::CallableFamilyDisappears,
                KotlinChangeCode::KcdCallableRemoved,
            ),
            rule(
                ChangeRuleInput::UnmatchedOverloadSet,
                KotlinChangeCode::KcdOverloadSetChanged,
            ),
            rule(
                ChangeRuleInput::JvmDescriptor,
                KotlinChangeCode::KcdJvmDescriptorChanged,
            ),
            rule(
                ChangeRuleInput::ParameterIndexAndTypeIgnoringTopLevelNullability,
                KotlinChangeCode::KcdParameterTypesChanged,
            ),
            rule(
                ChangeRuleInput::ReturnOrDeclaredTypeIgnoringTopLevelNullability,
                KotlinChangeCode::KcdReturnTypeChanged,
            ),
            rule(
                ChangeRuleInput::ReceiverTypeIgnoringTopLevelNullability,
                KotlinChangeCode::KcdReceiverTypeChanged,
            ),
            rule(
                ChangeRuleInput::ExplicitReturnDeclaredParameterReceiverNullability,
                KotlinChangeCode::KcdNullabilityChanged,
            ),
            rule(
                ChangeRuleInput::TypeParameterIndexesAndBounds,
                KotlinChangeCode::KcdTypeParameterBoundsChanged,
            ),
            rule(
                ChangeRuleInput::VisibilityEffectiveVisibilityExportBoundary,
                KotlinChangeCode::KcdVisibilityChanged,
            ),
            rule(
                ChangeRuleInput::Modality,
                KotlinChangeCode::KcdModalityChanged,
            ),
            rule(
                ChangeRuleInput::OverrideStatus,
                KotlinChangeCode::KcdOverrideStatusChanged,
            ),
            rule(
                ChangeRuleInput::UnknownPartialAmbiguousBoundaryOrResidualShape,
                KotlinChangeCode::KcdUnsupportedComparison,
            ),
        ],
        verdict_semantics: ChangeRuleSemantics::ObservationOnly,
    })
}

fn rules_digest_for(rules: &KotlinChangeRules<'_>) -> Result<String, ClewError> {
    canonical::hash(rules).map_err(internal)
}

fn rule(input: ChangeRuleInput, output: KotlinChangeCode) -> KotlinChangeRule {
    KotlinChangeRule {
        input,
        output,
        semantics: ChangeRuleSemantics::ObservationOnly,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct KotlinChangeRules<'a> {
    schema: &'a str,
    rows: Vec<KotlinChangeRule>,
    verdict_semantics: ChangeRuleSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct KotlinChangeRule {
    input: ChangeRuleInput,
    output: KotlinChangeCode,
    semantics: ChangeRuleSemantics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ChangeRuleSemantics {
    ObservationOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ChangeRuleInput {
    CallableFamilyAppears,
    CallableFamilyDisappears,
    UnmatchedOverloadSet,
    JvmDescriptor,
    ParameterIndexAndTypeIgnoringTopLevelNullability,
    ReturnOrDeclaredTypeIgnoringTopLevelNullability,
    ReceiverTypeIgnoringTopLevelNullability,
    ExplicitReturnDeclaredParameterReceiverNullability,
    TypeParameterIndexesAndBounds,
    VisibilityEffectiveVisibilityExportBoundary,
    Modality,
    OverrideStatus,
    UnknownPartialAmbiguousBoundaryOrResidualShape,
}

fn compare_impacts(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
) -> Result<(Vec<KotlinChangeObservation>, Vec<ChangeObligationDetail>), ClewError> {
    let mut observations = Vec::new();
    let mut obligations = Vec::new();
    for side in [ImpactSide::Provider, ImpactSide::Consumer] {
        compare_lane(side, before, after, &mut observations, &mut obligations)?;
    }
    Ok((observations, obligations))
}

fn compare_lane(
    side: ImpactSide,
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    observations: &mut Vec<KotlinChangeObservation>,
    obligations: &mut Vec<ChangeObligationDetail>,
) -> Result<(), ClewError> {
    let before_lane = lane_declarations(before, side)?;
    let after_lane = lane_declarations(after, side)?;
    let mut unsupported_before_ids = before_lane.unsupported_finding_ids.clone();
    let mut unsupported_after_ids = after_lane.unsupported_finding_ids.clone();
    let before_truncated = before.impact.evidence.selection.findings_truncated;
    let after_truncated = after.impact.evidence.selection.findings_truncated;
    unsupported_before_ids.sort();
    unsupported_before_ids.dedup();
    unsupported_after_ids.sort();
    unsupported_after_ids.dedup();
    if !unsupported_before_ids.is_empty()
        || !unsupported_after_ids.is_empty()
        || before_truncated
        || after_truncated
    {
        observations.push(observation(
            KotlinChangeCode::KcdUnsupportedComparison,
            side,
            before_lane.unsupported_declarations.clone(),
            after_lane.unsupported_declarations.clone(),
        )?);
        obligations.push(ChangeObligationDetail::Comparison {
            phase: ChangeObligationPhase::Comparison,
            side,
            code: if before_truncated || after_truncated {
                ComparisonObligationCode::VerifyTruncatedImpactEvidence
            } else {
                ComparisonObligationCode::VerifyUnsupportedComparison
            },
            before_finding_ids: unsupported_before_ids,
            after_finding_ids: unsupported_after_ids,
        });
    }

    compare_exact_lanes(side, before_lane, after_lane, observations, obligations)
}

fn compare_exact_lanes(
    side: ImpactSide,
    before_lane: LaneDeclarations,
    after_lane: LaneDeclarations,
    observations: &mut Vec<KotlinChangeObservation>,
    obligations: &mut Vec<ChangeObligationDetail>,
) -> Result<(), ClewError> {
    let mut before_by_symbol = unique_by_symbol(before_lane.exact)?;
    let mut after_by_symbol = unique_by_symbol(after_lane.exact)?;
    let common_symbols = before_by_symbol
        .keys()
        .filter(|symbol| after_by_symbol.contains_key(*symbol))
        .cloned()
        .collect::<Vec<_>>();
    let had_matched = !common_symbols.is_empty();
    for symbol in common_symbols {
        let before_declaration = before_by_symbol.remove(&symbol).expect("known key");
        let after_declaration = after_by_symbol.remove(&symbol).expect("known key");
        compare_declaration_pair(
            side,
            before_declaration,
            after_declaration,
            observations,
            obligations,
        )?;
    }
    let mut unmatched_before = before_by_symbol.into_values().collect::<Vec<_>>();
    let mut unmatched_after = after_by_symbol.into_values().collect::<Vec<_>>();
    unmatched_before.sort_by(|left, right| left.symbol_identity.cmp(&right.symbol_identity));
    unmatched_after.sort_by(|left, right| left.symbol_identity.cmp(&right.symbol_identity));

    if unmatched_before.len() == 1
        && unmatched_after.len() == 1
        && !had_matched
        && unmatched_before[0].compiler_callable_id.is_some()
        && unmatched_before[0].compiler_callable_id == unmatched_after[0].compiler_callable_id
    {
        compare_declaration_pair(
            side,
            unmatched_before.pop().expect("one row"),
            unmatched_after.pop().expect("one row"),
            observations,
            obligations,
        )?;
        return Ok(());
    }
    if unmatched_before.is_empty() && unmatched_after.is_empty() {
        return Ok(());
    }
    if !before_lane.absence_complete || !after_lane.absence_complete {
        let before_ids = unmatched_before
            .iter()
            .map(|row| row.finding_id.clone())
            .collect::<Vec<_>>();
        let after_ids = unmatched_after
            .iter()
            .map(|row| row.finding_id.clone())
            .collect::<Vec<_>>();
        observations.push(observation(
            KotlinChangeCode::KcdUnsupportedComparison,
            side,
            unmatched_before,
            unmatched_after,
        )?);
        obligations.push(ChangeObligationDetail::Comparison {
            phase: ChangeObligationPhase::Comparison,
            side,
            code: ComparisonObligationCode::VerifyUnsupportedComparison,
            before_finding_ids: before_ids,
            after_finding_ids: after_ids,
        });
        return Ok(());
    }
    if unmatched_before.is_empty() {
        observations.push(observation(
            if had_matched {
                KotlinChangeCode::KcdOverloadSetChanged
            } else {
                KotlinChangeCode::KcdCallableAdded
            },
            side,
            Vec::new(),
            unmatched_after,
        )?);
        return Ok(());
    }
    if unmatched_after.is_empty() {
        observations.push(observation(
            if had_matched {
                KotlinChangeCode::KcdOverloadSetChanged
            } else {
                KotlinChangeCode::KcdCallableRemoved
            },
            side,
            unmatched_before,
            Vec::new(),
        )?);
        return Ok(());
    }

    let before_ids = unmatched_before
        .iter()
        .map(|row| row.finding_id.clone())
        .collect::<Vec<_>>();
    let after_ids = unmatched_after
        .iter()
        .map(|row| row.finding_id.clone())
        .collect::<Vec<_>>();
    observations.push(observation(
        KotlinChangeCode::KcdOverloadSetChanged,
        side,
        unmatched_before,
        unmatched_after,
    )?);
    obligations.push(ChangeObligationDetail::Comparison {
        phase: ChangeObligationPhase::Comparison,
        side,
        code: ComparisonObligationCode::DisambiguateOverloadSet,
        before_finding_ids: before_ids,
        after_finding_ids: after_ids,
    });
    Ok(())
}

#[derive(Default)]
struct LaneDeclarations {
    exact: Vec<ChangeDeclarationEvidence>,
    unsupported_declarations: Vec<ChangeDeclarationEvidence>,
    unsupported_finding_ids: Vec<String>,
    absence_complete: bool,
}

fn lane_declarations(
    input: VerifiedChangeSide<'_>,
    side: ImpactSide,
) -> Result<LaneDeclarations, ClewError> {
    let impact = input.impact;
    let mut lane = LaneDeclarations {
        absence_complete: true,
        ..LaneDeclarations::default()
    };
    for finding in impact
        .evidence
        .selection
        .findings
        .iter()
        .filter(|finding| finding.side == side)
    {
        match &finding.detail {
            ImpactFindingDetail::Declaration { .. } => {
                let evidence = declaration_evidence(finding)?;
                if finding.authority == ImpactFindingAuthority::ExactProjectedDeclaration
                    && evidence.shape_digest.is_some()
                {
                    lane.exact.push(evidence);
                } else {
                    lane.unsupported_finding_ids
                        .push(finding.finding_id.clone());
                    lane.unsupported_declarations.push(evidence);
                }
            }
            ImpactFindingDetail::Boundary { .. } => lane
                .unsupported_finding_ids
                .push(finding.finding_id.clone()),
            ImpactFindingDetail::Use { .. } => {}
        }
    }
    lane.exact
        .sort_by(|left, right| left.symbol_identity.cmp(&right.symbol_identity));
    lane.unsupported_declarations
        .sort_by(|left, right| left.symbol_identity.cmp(&right.symbol_identity));
    lane.unsupported_finding_ids.sort();
    lane.unsupported_finding_ids.dedup();
    let member_alias = match side {
        ImpactSide::Provider => &impact.authority.pair.provider_member,
        ImpactSide::Consumer => &impact.authority.pair.consumer_member,
    };
    let member = fact_member(input.fact_set, member_alias)?;
    lane.absence_complete = lane.unsupported_finding_ids.is_empty()
        && !impact.evidence.selection.findings_truncated
        && input.fact_set.authority.completeness.coverage
            == thread_callables::CallableFactSetCoverage::Complete
        && member.compilations.iter().all(|compilation| {
            compilation.descriptor_coverage
                == thread_callables::GraphCoverage::CompleteSupportedSubset
        })
        && !impact
            .evidence
            .selection
            .obligations
            .iter()
            .any(|obligation| {
                obligation
                    .member_alias
                    .as_deref()
                    .is_none_or(|alias| alias == member_alias)
                    && obligation_blocks_absence(obligation.code)
            });
    Ok(lane)
}

fn obligation_blocks_absence(code: ImpactObligationCode) -> bool {
    matches!(
        code,
        ImpactObligationCode::CompleteDescriptorScope
            | ImpactObligationCode::VerifyDeclarationEvidence
            | ImpactObligationCode::VerifyRelatedBoundary
            | ImpactObligationCode::VerifyBoundaryCheck
            | ImpactObligationCode::VerifyFactSetBoundaryScope
            | ImpactObligationCode::NarrowOrExpandQuery
    )
}

fn declaration_evidence(finding: &ImpactFinding) -> Result<ChangeDeclarationEvidence, ClewError> {
    let ImpactFindingDetail::Declaration {
        declaration_kind,
        symbol_identity,
        compiler_callable_id,
        compiler_class_id,
        projected_shape,
    } = &finding.detail
    else {
        return Err(corrupt("impact finding is not a declaration"));
    };
    Ok(ChangeDeclarationEvidence {
        finding_id: finding.finding_id.clone(),
        fact_id: finding.evidence.fact_id.clone(),
        member_alias: finding.member_alias.clone(),
        authority: finding.authority,
        declaration_kind: *declaration_kind,
        symbol_identity: symbol_identity.clone(),
        compiler_callable_id: compiler_callable_id.clone(),
        compiler_class_id: compiler_class_id.clone(),
        shape_digest: finding.evidence.shape_digest.clone(),
        projected_shape: projected_shape.clone(),
    })
}

fn unique_by_symbol(
    declarations: Vec<ChangeDeclarationEvidence>,
) -> Result<BTreeMap<String, ChangeDeclarationEvidence>, ClewError> {
    let mut by_symbol = BTreeMap::new();
    for declaration in declarations {
        if by_symbol
            .insert(declaration.symbol_identity.clone(), declaration)
            .is_some()
        {
            return Err(corrupt(
                "impact repeats a declaration symbol within one comparison lane",
            ));
        }
    }
    Ok(by_symbol)
}

fn compare_declaration_pair(
    side: ImpactSide,
    before: ChangeDeclarationEvidence,
    after: ChangeDeclarationEvidence,
    observations: &mut Vec<KotlinChangeObservation>,
    obligations: &mut Vec<ChangeObligationDetail>,
) -> Result<(), ClewError> {
    let fields = [
        (
            KotlinChangeCode::KcdJvmDescriptorChanged,
            jvm_descriptor(&before),
            jvm_descriptor(&after),
        ),
        (
            KotlinChangeCode::KcdParameterTypesChanged,
            parameter_types(&before.projected_shape),
            parameter_types(&after.projected_shape),
        ),
        (
            KotlinChangeCode::KcdReturnTypeChanged,
            return_type(&before.projected_shape),
            return_type(&after.projected_shape),
        ),
        (
            KotlinChangeCode::KcdReceiverTypeChanged,
            receiver_type(&before.projected_shape),
            receiver_type(&after.projected_shape),
        ),
        (
            KotlinChangeCode::KcdNullabilityChanged,
            nullability(&before.projected_shape),
            nullability(&after.projected_shape),
        ),
        (
            KotlinChangeCode::KcdTypeParameterBoundsChanged,
            type_parameter_bounds(&before.projected_shape),
            type_parameter_bounds(&after.projected_shape),
        ),
        (
            KotlinChangeCode::KcdVisibilityChanged,
            visibility(&before.projected_shape),
            visibility(&after.projected_shape),
        ),
        (
            KotlinChangeCode::KcdModalityChanged,
            field(&before.projected_shape, "modality"),
            field(&after.projected_shape, "modality"),
        ),
        (
            KotlinChangeCode::KcdOverrideStatusChanged,
            field(&before.projected_shape, "isOverride"),
            field(&after.projected_shape, "isOverride"),
        ),
    ];
    for (code, before_value, after_value) in fields {
        if before_value != after_value {
            observations.push(observation(
                code,
                side,
                vec![before.clone()],
                vec![after.clone()],
            )?);
        }
    }
    if residual_shape(&before) != residual_shape(&after) {
        observations.push(observation(
            KotlinChangeCode::KcdUnsupportedComparison,
            side,
            vec![before.clone()],
            vec![after.clone()],
        )?);
        obligations.push(ChangeObligationDetail::Comparison {
            phase: ChangeObligationPhase::Comparison,
            side,
            code: ComparisonObligationCode::VerifyUnsupportedComparison,
            before_finding_ids: vec![before.finding_id],
            after_finding_ids: vec![after.finding_id],
        });
    }
    Ok(())
}

fn observation(
    code: KotlinChangeCode,
    side: ImpactSide,
    mut before: Vec<ChangeDeclarationEvidence>,
    mut after: Vec<ChangeDeclarationEvidence>,
) -> Result<KotlinChangeObservation, ClewError> {
    before.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));
    after.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));
    let observation_id = canonical::hash(&ObservationMaterial {
        schema: "codeclew-kotlin-change-observation/1.0",
        code,
        side,
        before: &before,
        after: &after,
    })
    .map_err(internal)?;
    Ok(KotlinChangeObservation {
        observation_id,
        code,
        side,
        before,
        after,
    })
}

fn observation_sort_key(
    left: &KotlinChangeObservation,
    right: &KotlinChangeObservation,
) -> std::cmp::Ordering {
    (left.side, left.code, left.observation_id.as_str()).cmp(&(
        right.side,
        right.code,
        right.observation_id.as_str(),
    ))
}

fn jvm_descriptor(declaration: &ChangeDeclarationEvidence) -> Value {
    declaration
        .projected_shape
        .get("jvmDescriptor")
        .cloned()
        .or_else(|| {
            declaration
                .symbol_identity
                .split_once("#jvm:")
                .map(|(_, descriptor)| Value::String(descriptor.into()))
        })
        .unwrap_or(Value::Null)
}

fn parameter_types(shape: &Value) -> Value {
    Value::Array(
        shape
            .get("parameterTypes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|parameter| {
                let mut normalized = Map::new();
                if let Some(index) = parameter.get("index") {
                    normalized.insert("index".into(), index.clone());
                }
                if let Some(value) = parameter.get("type") {
                    normalized.insert("type".into(), normalized_type(value));
                }
                Value::Object(normalized)
            })
            .collect(),
    )
}

fn return_type(shape: &Value) -> Value {
    let mut value = Map::new();
    for name in ["returnType", "declaredType"] {
        if let Some(field) = shape.get(name) {
            value.insert(name.into(), normalized_type(field));
        }
    }
    Value::Object(value)
}

fn receiver_type(shape: &Value) -> Value {
    shape
        .get("receiverType")
        .map(|receiver| {
            receiver
                .get("type")
                .map(normalized_type)
                .unwrap_or_else(|| normalized_type(receiver))
        })
        .unwrap_or(Value::Null)
}

fn normalized_type(value: &Value) -> Value {
    match value.as_str() {
        Some(rendered) => Value::String(rendered.strip_suffix('?').unwrap_or(rendered).into()),
        None => value.clone(),
    }
}

fn nullability(shape: &Value) -> Value {
    let mut result = Map::new();
    for name in ["returnNullable", "declaredNullable"] {
        if let Some(value) = shape.get(name) {
            result.insert(name.into(), value.clone());
        }
    }
    if let Some(parameters) = shape.get("parameterTypes").and_then(Value::as_array) {
        result.insert(
            "parameterNullability".into(),
            Value::Array(
                parameters
                    .iter()
                    .map(|parameter| parameter.get("nullable").cloned().unwrap_or(Value::Null))
                    .collect(),
            ),
        );
    }
    if let Some(receiver) = shape.get("receiverType") {
        result.insert(
            "receiverNullable".into(),
            receiver.get("nullable").cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(result)
}

fn type_parameter_bounds(shape: &Value) -> Value {
    Value::Array(
        shape
            .get("typeParameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|parameter| {
                let mut normalized = Map::new();
                for name in ["index", "bounds"] {
                    if let Some(value) = parameter.get(name) {
                        normalized.insert(name.into(), value.clone());
                    }
                }
                Value::Object(normalized)
            })
            .collect(),
    )
}

fn visibility(shape: &Value) -> Value {
    let mut value = Map::new();
    for name in ["visibility", "effectiveVisibility", "exportBoundary"] {
        if let Some(field) = shape.get(name) {
            value.insert(name.into(), field.clone());
        }
    }
    Value::Object(value)
}

fn field(shape: &Value, name: &str) -> Value {
    shape.get(name).cloned().unwrap_or(Value::Null)
}

fn residual_shape(declaration: &ChangeDeclarationEvidence) -> Value {
    let known = BTreeSet::from([
        "symbolIdentity",
        "jvmDescriptor",
        "parameterTypes",
        "returnType",
        "returnNullable",
        "declaredType",
        "declaredNullable",
        "receiverType",
        "typeParameters",
        "visibility",
        "effectiveVisibility",
        "exportBoundary",
        "modality",
        "isOverride",
    ]);
    let mut value = declaration
        .projected_shape
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !known.contains(key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    value.insert(
        "declarationKind".into(),
        serde_json::to_value(declaration.declaration_kind).unwrap_or(Value::Null),
    );
    value.insert(
        "compilerCallableId".into(),
        declaration
            .compiler_callable_id
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    value.insert(
        "compilerClassId".into(),
        declaration
            .compiler_class_id
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    Value::Object(value)
}

fn collect_obligations(
    before: &PreparedThreadImpact,
    after: &PreparedThreadImpact,
    comparison: &mut Vec<ChangeObligationDetail>,
    budgets: &ChangeSetBudgets,
) -> Result<Vec<ChangeObligation>, ClewError> {
    let mut details = BTreeSet::new();
    details.extend(
        before
            .evidence
            .selection
            .obligations
            .iter()
            .cloned()
            .map(|obligation| ChangeObligationDetail::Upstream {
                phase: ChangeObligationPhase::Before,
                obligation,
            }),
    );
    details.extend(
        after
            .evidence
            .selection
            .obligations
            .iter()
            .cloned()
            .map(|obligation| ChangeObligationDetail::Upstream {
                phase: ChangeObligationPhase::After,
                obligation,
            }),
    );
    details.extend(std::mem::take(comparison));
    enforce_at_most(
        details.len(),
        budgets.max_obligations,
        "Kotlin change obligations exceed the frozen 4,096 bound",
    )?;
    details
        .into_iter()
        .map(|detail| {
            let obligation_id = canonical::hash(&ObligationMaterial {
                schema: "codeclew-kotlin-change-obligation/1.0",
                detail: &detail,
            })
            .map_err(internal)?;
            Ok(ChangeObligation {
                obligation_id,
                detail,
            })
        })
        .collect()
}

fn build_targets(
    comparison_digest: &str,
    before: VerifiedChangeSide<'_>,
    correspondence: &[MemberCorrespondence],
    observations: &[KotlinChangeObservation],
    obligations: &[ChangeObligation],
    budgets: &ChangeSetBudgets,
) -> Result<Vec<CoverageTarget>, ClewError> {
    let mut targets = Vec::new();
    for mapping in correspondence {
        let member = thread_member(before.thread, &mapping.before_member_alias)?;
        let role = if mapping.before_member_alias == before.impact.authority.pair.provider_member {
            ChangeMemberRole::Provider
        } else if mapping.before_member_alias == before.impact.authority.pair.consumer_member {
            ChangeMemberRole::Consumer
        } else {
            return Err(corrupt(
                "compared pair member has no provider or consumer role",
            ));
        };
        targets.push(target(
            comparison_digest,
            CoverageTargetKind::DeclaredMember,
            vec![
                VerificationCategory::MemberAuthority,
                VerificationCategory::RelationshipAuthority,
            ],
            CoverageTargetSubject::DeclaredMember {
                before_member_alias: mapping.before_member_alias.clone(),
                after_member_alias: mapping.after_member_alias.clone(),
                service_alias: member.service_alias.clone(),
                role,
            },
        )?);
    }
    for observation in observations {
        let mut categories = vec![VerificationCategory::StructuralComparison];
        if observation.code == KotlinChangeCode::KcdUnsupportedComparison {
            categories.push(VerificationCategory::CompilerBoundary);
        }
        targets.push(target(
            comparison_digest,
            CoverageTargetKind::Observation,
            categories,
            CoverageTargetSubject::Observation {
                observation_id: observation.observation_id.clone(),
                code: observation.code,
                side: observation.side,
            },
        )?);
    }
    for obligation in obligations {
        let phase = obligation_phase(&obligation.detail);
        let mut categories = vec![match obligation.detail {
            ChangeObligationDetail::Upstream { .. } => VerificationCategory::UpstreamObligation,
            ChangeObligationDetail::Comparison { .. } => VerificationCategory::ComparisonBoundary,
        }];
        if obligation_requires_relationship(&obligation.detail) {
            categories.push(VerificationCategory::RelationshipAuthority);
        }
        if obligation_requires_compiler(&obligation.detail) {
            categories.push(VerificationCategory::CompilerBoundary);
        }
        targets.push(target(
            comparison_digest,
            CoverageTargetKind::Obligation,
            categories,
            CoverageTargetSubject::Obligation {
                obligation_id: obligation.obligation_id.clone(),
                phase,
            },
        )?);
    }
    targets.sort_by(|left, right| left.target_id.cmp(&right.target_id));
    enforce_at_most(
        targets.len(),
        budgets.max_result_rows,
        "Kotlin change validation rows exceed the frozen 8,192 bound",
    )?;
    if targets
        .windows(2)
        .any(|pair| pair[0].target_id == pair[1].target_id)
    {
        return Err(corrupt("Kotlin change targets repeat an identity"));
    }
    Ok(targets)
}

fn target(
    comparison_digest: &str,
    target_kind: CoverageTargetKind,
    mut required_categories: Vec<VerificationCategory>,
    subject: CoverageTargetSubject,
) -> Result<CoverageTarget, ClewError> {
    required_categories.sort();
    required_categories.dedup();
    let target_id = canonical::hash(&TargetMaterial {
        schema: "codeclew-kotlin-change-coverage-target/1.0",
        comparison_digest,
        target_kind,
        required_categories: &required_categories,
        subject: &subject,
    })
    .map_err(internal)?;
    Ok(CoverageTarget {
        target_id,
        target_kind,
        required_categories,
        subject,
    })
}

fn obligation_phase(detail: &ChangeObligationDetail) -> ChangeObligationPhase {
    match detail {
        ChangeObligationDetail::Upstream { phase, .. }
        | ChangeObligationDetail::Comparison { phase, .. } => *phase,
    }
}

fn obligation_requires_relationship(detail: &ChangeObligationDetail) -> bool {
    matches!(
        detail,
        ChangeObligationDetail::Upstream {
            obligation: ImpactObligation {
                code: ImpactObligationCode::VerifyRelationshipAuthority,
                ..
            },
            ..
        }
    )
}

fn obligation_requires_compiler(detail: &ChangeObligationDetail) -> bool {
    match detail {
        ChangeObligationDetail::Comparison { .. } => true,
        ChangeObligationDetail::Upstream { obligation, .. } => matches!(
            obligation.code,
            ImpactObligationCode::ProjectedDeclarationNotObserved
                | ImpactObligationCode::DisambiguateOverloadSet
                | ImpactObligationCode::CompleteDescriptorScope
                | ImpactObligationCode::VerifyDeclarationEvidence
                | ImpactObligationCode::VerifyRelatedBoundary
                | ImpactObligationCode::VerifyBoundaryCheck
                | ImpactObligationCode::VerifyFactSetBoundaryScope
        ),
    }
}

fn normalize_coverage_document(
    raw: &[u8],
    budgets: &ChangeSetBudgets,
) -> Result<(KotlinChangeCoverageDocument, Vec<u8>), ClewError> {
    enforce_at_most(
        raw.len(),
        budgets.max_coverage_document_bytes,
        "Kotlin coverage document exceeds the frozen 2 MiB bound",
    )?;
    let mut document: KotlinChangeCoverageDocument = serde_json::from_slice(raw)
        .map_err(|_| invalid("Kotlin coverage document is not a closed JSON document"))?;
    if document.schema != KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA {
        return Err(invalid("Kotlin coverage document schema is invalid"));
    }
    if document.entries.len() > budgets.max_coverage_entries {
        return Err(budget(
            "Kotlin coverage document exceeds the frozen 8,192-entry bound",
        ));
    }
    let mut targets = BTreeSet::new();
    for entry in &mut document.entries {
        if !sha256_digest(&entry.target_id)
            || !targets.insert(entry.target_id.clone())
            || !safe_tracking_id(&entry.handling.id)
            || entry.required_categories.is_empty()
        {
            return Err(invalid(
                "Kotlin coverage entry target/categories/handling identity is invalid",
            ));
        }
        let original_len = entry.required_categories.len();
        entry.required_categories.sort();
        entry.required_categories.dedup();
        if entry.required_categories.len() != original_len {
            return Err(invalid(
                "Kotlin coverage entry repeats a verification category",
            ));
        }
    }
    document
        .entries
        .sort_by(|left, right| left.target_id.cmp(&right.target_id));
    let bytes = canonical::bytes(&document).map_err(internal)?;
    enforce_at_most(
        bytes.len(),
        budgets.max_coverage_document_bytes,
        "canonical Kotlin coverage document exceeds the frozen 2 MiB bound",
    )?;
    Ok((document, bytes))
}

fn validate_coverage(
    targets: &[CoverageTarget],
    document: &KotlinChangeCoverageDocument,
) -> Result<Vec<CoverageValidationRow>, ClewError> {
    let target_by_id = targets
        .iter()
        .map(|target| (target.target_id.as_str(), target))
        .collect::<BTreeMap<_, _>>();
    let entries = document
        .entries
        .iter()
        .map(|entry| (entry.target_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    for entry in &document.entries {
        let Some(target) = target_by_id.get(entry.target_id.as_str()) else {
            return Err(invalid(
                "Kotlin coverage document contains an unknown or stale target",
            ));
        };
        if entry.required_categories != target.required_categories {
            return Err(invalid(
                "Kotlin coverage entry verification categories do not exactly match the target",
            ));
        }
    }
    Ok(targets
        .iter()
        .map(|target| {
            let entry = entries.get(target.target_id.as_str()).copied();
            CoverageValidationRow {
                target_id: target.target_id.clone(),
                target_kind: target.target_kind,
                required_categories: target.required_categories.clone(),
                covered: entry.is_some(),
                handling: entry.map(|entry| entry.handling.clone()),
            }
        })
        .collect())
}

fn direct_closure(
    before: VerifiedChangeSide<'_>,
    after: VerifiedChangeSide<'_>,
    new_objects: &[CasObject],
    budgets: &ChangeSetBudgets,
) -> Result<(Vec<CasObject>, usize), ClewError> {
    enforce_exact(
        new_objects.len(),
        budgets.max_derived_cas_objects,
        "Kotlin change-set publication must contain exactly two new CAS objects",
    )?;
    let mut references = Vec::new();
    references.extend(before.fact_set.authority.direct_cas_closure.iter().cloned());
    references.extend(before.impact.authority.direct_cas_closure.iter().cloned());
    references.extend(after.fact_set.authority.direct_cas_closure.iter().cloned());
    references.extend(after.impact.authority.direct_cas_closure.iter().cloned());
    references.extend(new_objects.iter().cloned());
    canonical_closure(references, budgets.max_retained_closure_bytes)
}

fn canonical_closure(
    references: Vec<CasObject>,
    max_bytes: usize,
) -> Result<(Vec<CasObject>, usize), ClewError> {
    let mut by_digest = BTreeMap::<String, CasObject>::new();
    for reference in references {
        if reference.schema != CAS_OBJECT_SCHEMA || reference.size == 0 {
            return Err(corrupt(
                "Kotlin change-set closure contains an invalid CAS reference",
            ));
        }
        match by_digest.entry(reference.digest.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(reference);
            }
            std::collections::btree_map::Entry::Occupied(entry) if entry.get() != &reference => {
                return Err(corrupt(
                    "Kotlin change-set closure repeats a digest with different authority",
                ));
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
    }
    let mut closure = by_digest.into_values().collect::<Vec<_>>();
    closure.sort_by(|left, right| {
        (left.object_schema.as_str(), left.digest.as_str(), left.size).cmp(&(
            right.object_schema.as_str(),
            right.digest.as_str(),
            right.size,
        ))
    });
    let bytes = closure.iter().try_fold(0usize, |total, reference| {
        let size = usize::try_from(reference.size)
            .map_err(|_| budget("Kotlin change-set retained size is not representable"))?;
        total
            .checked_add(size)
            .ok_or_else(|| budget("Kotlin change-set retained size overflowed"))
    })?;
    if bytes > max_bytes {
        return Err(budget(
            "Kotlin change-set proof closure exceeds the frozen 64 MiB bound",
        ));
    }
    Ok((closure, bytes))
}

fn bounded_authority_projection(
    authority: &mut ThreadChangeSetAuthority,
) -> Result<(ThreadChangeSetProjection, Vec<u8>), ClewError> {
    loop {
        authority.authority_digest = authority_digest(authority)?;
        let projection = project(authority);
        let bytes = canonical::bytes(&projection).map_err(internal)?;
        if bytes.len().saturating_add(CHANGE_STDOUT_ENVELOPE_RESERVE)
            <= authority.budgets.max_stdout_bytes
        {
            return Ok((projection, bytes));
        }
        if authority.public_covered_target_ids.pop().is_none() {
            return Err(budget(
                "mandatory missing Kotlin change targets exceed the stdout bound",
            ));
        }
        authority.public_covered_targets_truncated = true;
        authority.authority_digest.clear();
    }
}

fn target_projection(target: &CoverageTarget) -> CoverageTargetProjection {
    CoverageTargetProjection {
        target_id: target.target_id.clone(),
        target_kind: target.target_kind,
        required_categories: target.required_categories.clone(),
        subject: target.subject.clone(),
    }
}

fn authority_digest(authority: &ThreadChangeSetAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn safe_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && crate::text_authority::is_nfc(value)
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn safe_tracking_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.is_ascii()
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn enforce_at_most(actual: usize, maximum: usize, message: &'static str) -> Result<(), ClewError> {
    if actual > maximum {
        Err(budget(message))
    } else {
        Ok(())
    }
}

fn enforce_exact(actual: usize, expected: usize, message: &'static str) -> Result<(), ClewError> {
    if actual != expected {
        Err(budget(message))
    } else {
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn digest(label: &str) -> String {
        canonical::hash(&label).unwrap()
    }

    fn base_shape() -> Value {
        json!({
            "declarationKind":"FUNCTION",
            "symbolIdentity":"callable:p.Service.read#jvm:(Ljava/lang/String;)Ljava/lang/String;",
            "ownerIdentity":"class:p.Service",
            "containment":["class:p.Service"],
            "visibility":"public",
            "effectiveVisibility":"public",
            "exportBoundary":"PUBLIC_API",
            "modality":"FINAL",
            "compilerCallableId":"p.Service.read",
            "isOverride":false,
            "jvmDescriptor":"(Ljava/lang/String;)Ljava/lang/String;",
            "returnType":"kotlin/String",
            "returnNullable":false,
            "parameterTypes":[{"index":0,"type":"kotlin/String","nullable":false}],
            "receiverType":{"type":"p.Service","nullable":false},
            "typeParameters":[{"index":0,"compilerName":"T","bounds":["kotlin/Any"]}]
        })
    }

    fn declaration(label: &str, shape: Value) -> ChangeDeclarationEvidence {
        ChangeDeclarationEvidence {
            finding_id: digest(&format!("finding-{label}")),
            fact_id: digest(&format!("fact-{label}")),
            member_alias: "provider".into(),
            authority: ImpactFindingAuthority::ExactProjectedDeclaration,
            declaration_kind: DeclarationKind::Function,
            symbol_identity: shape
                .get("symbolIdentity")
                .and_then(Value::as_str)
                .unwrap()
                .into(),
            compiler_callable_id: Some("p.Service.read".into()),
            compiler_class_id: None,
            shape_digest: Some(canonical::hash(&shape).unwrap()),
            projected_shape: shape,
        }
    }

    fn compared_codes(
        before: ChangeDeclarationEvidence,
        after: ChangeDeclarationEvidence,
    ) -> (BTreeSet<KotlinChangeCode>, Vec<ChangeObligationDetail>) {
        let mut observations = Vec::new();
        let mut obligations = Vec::new();
        compare_declaration_pair(
            ImpactSide::Provider,
            before,
            after,
            &mut observations,
            &mut obligations,
        )
        .unwrap();
        (
            observations
                .into_iter()
                .map(|observation| observation.code)
                .collect(),
            obligations,
        )
    }

    fn mutate_shape(path: &str, value: Value) -> ChangeDeclarationEvidence {
        let mut shape = base_shape();
        *shape.pointer_mut(path).unwrap() = value;
        declaration(path, shape)
    }

    #[test]
    fn every_exact_field_mapping_emits_only_its_frozen_kcd_code() {
        let cases = [
            (
                "/jvmDescriptor",
                json!("(I)Ljava/lang/String;"),
                KotlinChangeCode::KcdJvmDescriptorChanged,
            ),
            (
                "/parameterTypes/0/type",
                json!("kotlin/Int"),
                KotlinChangeCode::KcdParameterTypesChanged,
            ),
            (
                "/returnType",
                json!("kotlin/Int"),
                KotlinChangeCode::KcdReturnTypeChanged,
            ),
            (
                "/receiverType/type",
                json!("p.Other"),
                KotlinChangeCode::KcdReceiverTypeChanged,
            ),
            (
                "/returnNullable",
                json!(true),
                KotlinChangeCode::KcdNullabilityChanged,
            ),
            (
                "/typeParameters/0/bounds/0",
                json!("kotlin/Number"),
                KotlinChangeCode::KcdTypeParameterBoundsChanged,
            ),
            (
                "/effectiveVisibility",
                json!("internal"),
                KotlinChangeCode::KcdVisibilityChanged,
            ),
            (
                "/modality",
                json!("OPEN"),
                KotlinChangeCode::KcdModalityChanged,
            ),
            (
                "/isOverride",
                json!(true),
                KotlinChangeCode::KcdOverrideStatusChanged,
            ),
        ];
        for (path, value, expected) in cases {
            let (codes, obligations) = compared_codes(
                declaration("before", base_shape()),
                mutate_shape(path, value),
            );
            assert_eq!(codes, BTreeSet::from([expected]), "case {path}");
            assert!(obligations.is_empty(), "case {path}");
        }
    }

    #[test]
    fn explicit_nullability_is_not_double_counted_as_a_type_change() {
        let mut after = base_shape();
        after["returnType"] = json!("kotlin/String?");
        after["returnNullable"] = json!(true);
        after["parameterTypes"][0]["type"] = json!("kotlin/String?");
        after["parameterTypes"][0]["nullable"] = json!(true);
        let (codes, _) = compared_codes(
            declaration("before", base_shape()),
            declaration("after", after),
        );
        assert_eq!(
            codes,
            BTreeSet::from([KotlinChangeCode::KcdNullabilityChanged])
        );
    }

    #[test]
    fn unknown_residual_shape_is_unsupported_and_has_an_obligation() {
        let (codes, obligations) = compared_codes(
            declaration("before", base_shape()),
            mutate_shape("/ownerIdentity", json!("class:p.Other")),
        );
        assert_eq!(
            codes,
            BTreeSet::from([KotlinChangeCode::KcdUnsupportedComparison])
        );
        assert!(matches!(
            obligations.as_slice(),
            [ChangeObligationDetail::Comparison {
                code: ComparisonObligationCode::VerifyUnsupportedComparison,
                ..
            }]
        ));
    }

    fn lane(rows: Vec<ChangeDeclarationEvidence>, absence_complete: bool) -> LaneDeclarations {
        LaneDeclarations {
            exact: rows,
            unsupported_declarations: Vec::new(),
            unsupported_finding_ids: Vec::new(),
            absence_complete,
        }
    }

    fn with_symbol(label: &str, symbol: &str, callable: &str) -> ChangeDeclarationEvidence {
        let mut shape = base_shape();
        shape["symbolIdentity"] = json!(symbol);
        shape["compilerCallableId"] = json!(callable);
        let mut row = declaration(label, shape);
        row.symbol_identity = symbol.into();
        row.compiler_callable_id = Some(callable.into());
        row
    }

    fn lane_codes(
        before: LaneDeclarations,
        after: LaneDeclarations,
    ) -> (Vec<KotlinChangeCode>, Vec<ChangeObligationDetail>) {
        let mut observations = Vec::new();
        let mut obligations = Vec::new();
        compare_exact_lanes(
            ImpactSide::Provider,
            before,
            after,
            &mut observations,
            &mut obligations,
        )
        .unwrap();
        (
            observations
                .into_iter()
                .map(|observation| observation.code)
                .collect(),
            obligations,
        )
    }

    #[test]
    fn complete_presence_and_absence_emit_added_and_removed() {
        let row = with_symbol("one", "callable:p.Service.read#jvm:()V", "p.Service.read");
        let (added, _) = lane_codes(lane(Vec::new(), true), lane(vec![row.clone()], true));
        let (removed, _) = lane_codes(lane(vec![row], true), lane(Vec::new(), true));
        assert_eq!(added, vec![KotlinChangeCode::KcdCallableAdded]);
        assert_eq!(removed, vec![KotlinChangeCode::KcdCallableRemoved]);
    }

    #[test]
    fn adding_or_removing_an_overload_does_not_claim_family_appearance() {
        let common = with_symbol(
            "common",
            "callable:p.Service.read#jvm:(I)V",
            "p.Service.read",
        );
        let added = with_symbol(
            "added",
            "callable:p.Service.read#jvm:(J)V",
            "p.Service.read",
        );
        let (addition, addition_obligations) = lane_codes(
            lane(vec![common.clone()], true),
            lane(vec![common.clone(), added.clone()], true),
        );
        let (removal, removal_obligations) = lane_codes(
            lane(vec![common, added], true),
            lane(
                vec![with_symbol(
                    "common-after",
                    "callable:p.Service.read#jvm:(I)V",
                    "p.Service.read",
                )],
                true,
            ),
        );
        assert_eq!(addition, vec![KotlinChangeCode::KcdOverloadSetChanged]);
        assert_eq!(removal, vec![KotlinChangeCode::KcdOverloadSetChanged]);
        assert!(addition_obligations.is_empty());
        assert!(removal_obligations.is_empty());
    }

    #[test]
    fn partial_absence_never_emits_added_removed_or_overload_claims() {
        let row = with_symbol("one", "callable:p.Service.read#jvm:()V", "p.Service.read");
        let (codes, obligations) = lane_codes(lane(Vec::new(), false), lane(vec![row], true));
        assert_eq!(codes, vec![KotlinChangeCode::KcdUnsupportedComparison]);
        assert!(matches!(
            obligations.as_slice(),
            [ChangeObligationDetail::Comparison {
                code: ComparisonObligationCode::VerifyUnsupportedComparison,
                ..
            }]
        ));
    }

    #[test]
    fn ambiguous_multiple_unmatched_overloads_use_no_pairing_heuristic() {
        let before = vec![
            with_symbol("b1", "callable:p.Service.read#jvm:(I)V", "p.Service.read"),
            with_symbol("b2", "callable:p.Service.read#jvm:(J)V", "p.Service.read"),
        ];
        let after = vec![
            with_symbol("a1", "callable:p.Service.read#jvm:(S)V", "p.Service.read"),
            with_symbol("a2", "callable:p.Service.read#jvm:(B)V", "p.Service.read"),
        ];
        let (codes, obligations) = lane_codes(lane(before, true), lane(after, true));
        assert_eq!(codes, vec![KotlinChangeCode::KcdOverloadSetChanged]);
        assert!(matches!(
            obligations.as_slice(),
            [ChangeObligationDetail::Comparison {
                code: ComparisonObligationCode::DisambiguateOverloadSet,
                ..
            }]
        ));
    }

    #[test]
    fn one_unmatched_callable_id_pair_is_compared_deterministically() {
        let before = with_symbol(
            "before",
            "callable:p.Service.read#jvm:(I)V",
            "p.Service.read",
        );
        let mut after = with_symbol(
            "after",
            "callable:p.Service.read#jvm:(J)V",
            "p.Service.read",
        );
        after.projected_shape["jvmDescriptor"] = json!("(J)V");
        let (codes, obligations) = lane_codes(lane(vec![before], true), lane(vec![after], true));
        assert!(codes.contains(&KotlinChangeCode::KcdJvmDescriptorChanged));
        assert!(!codes.contains(&KotlinChangeCode::KcdOverloadSetChanged));
        assert!(obligations.is_empty());
    }

    #[test]
    fn one_changed_overload_beside_a_common_overload_is_not_heuristically_paired() {
        let common = with_symbol(
            "common",
            "callable:p.Service.read#jvm:(I)V",
            "p.Service.read",
        );
        let before_changed = with_symbol(
            "before-changed",
            "callable:p.Service.read#jvm:(Ljava/lang/String;)V",
            "p.Service.read",
        );
        let after_changed = with_symbol(
            "after-changed",
            "callable:p.Service.read#jvm:(J)V",
            "p.Service.read",
        );
        let (codes, obligations) = lane_codes(
            lane(vec![common.clone(), before_changed], true),
            lane(vec![common, after_changed], true),
        );
        assert_eq!(codes, vec![KotlinChangeCode::KcdOverloadSetChanged]);
        assert!(matches!(
            obligations.as_slice(),
            [ChangeObligationDetail::Comparison {
                code: ComparisonObligationCode::DisambiguateOverloadSet,
                ..
            }]
        ));
    }

    fn coverage_target(comparison: &str, suffix: &str) -> CoverageTarget {
        target(
            comparison,
            CoverageTargetKind::Observation,
            vec![VerificationCategory::StructuralComparison],
            CoverageTargetSubject::Observation {
                observation_id: digest(suffix),
                code: KotlinChangeCode::KcdReturnTypeChanged,
                side: ImpactSide::Provider,
            },
        )
        .unwrap()
    }

    fn coverage_bytes(entries: Vec<CoverageDocumentEntry>) -> Vec<u8> {
        canonical::bytes(&KotlinChangeCoverageDocument {
            schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
            entries,
        })
        .unwrap()
    }

    fn entry(target: &CoverageTarget, id: &str) -> CoverageDocumentEntry {
        CoverageDocumentEntry {
            target_id: target.target_id.clone(),
            required_categories: target.required_categories.clone(),
            handling: CoverageHandling {
                kind: CoverageHandlingKind::ExternalWork,
                id: id.into(),
            },
        }
    }

    #[test]
    fn coverage_document_is_closed_normalized_and_permutation_stable() {
        let a = coverage_target(&digest("comparison"), "a");
        let b = coverage_target(&digest("comparison"), "b");
        let budgets = ChangeSetBudgets::frozen();
        let (left, left_bytes) = normalize_coverage_document(
            &coverage_bytes(vec![entry(&a, "work-a"), entry(&b, "work-b")]),
            &budgets,
        )
        .unwrap();
        let (right, right_bytes) = normalize_coverage_document(
            &coverage_bytes(vec![entry(&b, "work-b"), entry(&a, "work-a")]),
            &budgets,
        )
        .unwrap();
        assert_eq!(left, right);
        assert_eq!(left_bytes, right_bytes);

        let unknown_field = serde_json::to_vec(&json!({
            "schema":KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA,
            "entries":[],
            "command":"rm -rf anything"
        }))
        .unwrap();
        assert!(normalize_coverage_document(&unknown_field, &budgets).is_err());
    }

    #[test]
    fn tracking_identity_rejects_paths_shell_text_urls_and_colons() {
        for rejected in [
            "../work",
            "/tmp/work",
            "work item",
            "work:1",
            "work/1",
            "$(work)",
            "https://example.invalid",
            "a;echo",
            "a\\b",
        ] {
            assert!(!safe_tracking_id(rejected), "{rejected}");
        }
        for accepted in ["A", "work-123", "TASK_1.2"] {
            assert!(safe_tracking_id(accepted), "{accepted}");
        }
    }

    #[test]
    fn missing_coverage_is_incomplete_but_unknown_or_category_mismatch_is_rejected() {
        let target = coverage_target(&digest("comparison"), "target");
        let empty = KotlinChangeCoverageDocument {
            schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
            entries: Vec::new(),
        };
        let rows = validate_coverage(std::slice::from_ref(&target), &empty).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].covered);

        let mut wrong = entry(&target, "work");
        wrong.required_categories = vec![VerificationCategory::CompilerBoundary];
        assert!(
            validate_coverage(
                std::slice::from_ref(&target),
                &KotlinChangeCoverageDocument {
                    schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
                    entries: vec![wrong],
                }
            )
            .is_err()
        );
        let mut unknown = entry(&target, "work");
        unknown.target_id = digest("stale");
        assert!(
            validate_coverage(
                &[target],
                &KotlinChangeCoverageDocument {
                    schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
                    entries: vec![unknown],
                }
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_targets_and_categories_are_rejected_before_validation() {
        let target = coverage_target(&digest("comparison"), "target");
        let duplicate_targets = coverage_bytes(vec![entry(&target, "a"), entry(&target, "b")]);
        assert!(
            normalize_coverage_document(&duplicate_targets, &ChangeSetBudgets::frozen()).is_err()
        );
        let mut duplicate_category = entry(&target, "a");
        duplicate_category
            .required_categories
            .push(VerificationCategory::StructuralComparison);
        assert!(
            normalize_coverage_document(
                &coverage_bytes(vec![duplicate_category]),
                &ChangeSetBudgets::frozen()
            )
            .is_err()
        );
    }

    #[test]
    fn coverage_raw_and_canonical_bounds_are_exact_and_fail_at_plus_one() {
        let budgets = ChangeSetBudgets::frozen();
        let mut exact = coverage_bytes(Vec::new());
        exact.resize(MAX_CHANGE_COVERAGE_DOCUMENT_BYTES, b' ');
        assert!(normalize_coverage_document(&exact, &budgets).is_ok());
        exact.push(b' ');
        let error = normalize_coverage_document(&exact, &budgets).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }

    #[test]
    fn coverage_entry_bound_is_exact_and_fails_at_plus_one() {
        let budgets = ChangeSetBudgets::frozen();
        let entries = (0..MAX_CHANGE_COVERAGE_ENTRIES)
            .map(|index| CoverageDocumentEntry {
                target_id: digest(&format!("target-{index}")),
                required_categories: vec![VerificationCategory::StructuralComparison],
                handling: CoverageHandling {
                    kind: CoverageHandlingKind::Action,
                    id: format!("a{index}"),
                },
            })
            .collect::<Vec<_>>();
        let bytes = coverage_bytes(entries.clone());
        assert!(bytes.len() < MAX_CHANGE_COVERAGE_DOCUMENT_BYTES);
        assert!(normalize_coverage_document(&bytes, &budgets).is_ok());
        let mut too_many = entries;
        too_many.push(CoverageDocumentEntry {
            target_id: digest("one-too-many"),
            required_categories: vec![VerificationCategory::StructuralComparison],
            handling: CoverageHandling {
                kind: CoverageHandlingKind::Action,
                id: "overflow".into(),
            },
        });
        let error = normalize_coverage_document(&coverage_bytes(too_many), &budgets).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }

    #[test]
    fn canonical_coverage_bytes_can_reach_the_exact_frozen_limit() {
        let budgets = ChangeSetBudgets::frozen();
        let mut document = KotlinChangeCoverageDocument {
            schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
            entries: (0..MAX_CHANGE_COVERAGE_ENTRIES)
                .map(|index| CoverageDocumentEntry {
                    target_id: digest(&format!("canonical-target-{index}")),
                    required_categories: vec![VerificationCategory::StructuralComparison],
                    handling: CoverageHandling {
                        kind: CoverageHandlingKind::Action,
                        id: "a".into(),
                    },
                })
                .collect(),
        };
        let baseline = canonical::bytes(&document).unwrap().len();
        let mut remaining = MAX_CHANGE_COVERAGE_DOCUMENT_BYTES - baseline;
        for entry in &mut document.entries {
            let added = remaining.min(127);
            entry.handling.id.push_str(&"x".repeat(added));
            remaining -= added;
            if remaining == 0 {
                break;
            }
        }
        assert_eq!(
            remaining, 0,
            "closed grammar must span the frozen byte limit"
        );
        let exact = canonical::bytes(&document).unwrap();
        assert_eq!(exact.len(), MAX_CHANGE_COVERAGE_DOCUMENT_BYTES);
        assert!(normalize_coverage_document(&exact, &budgets).is_ok());
        assert!(
            enforce_at_most(
                exact.len() + 1,
                budgets.max_coverage_document_bytes,
                "coverage canonical overflow"
            )
            .is_err()
        );
    }

    #[test]
    fn every_frozen_count_bound_passes_at_limit_and_fails_at_plus_one() {
        let budgets = ChangeSetBudgets::frozen();
        for maximum in [
            budgets.max_observations,
            budgets.max_obligations,
            budgets.max_result_rows,
        ] {
            assert!(enforce_at_most(maximum, maximum, "bounded").is_ok());
            assert_eq!(
                enforce_at_most(maximum + 1, maximum, "bounded")
                    .unwrap_err()
                    .code,
                ErrorCode::SliceBudgetExceeded
            );
        }
        assert!(
            enforce_exact(
                MAX_CHANGE_MEMBER_CORRESPONDENCES,
                budgets.max_member_correspondences,
                "pair"
            )
            .is_ok()
        );
        assert!(
            enforce_exact(
                MAX_CHANGE_MEMBER_CORRESPONDENCES + 1,
                budgets.max_member_correspondences,
                "pair"
            )
            .is_err()
        );
        assert!(
            enforce_exact(
                MAX_CHANGE_DERIVED_CAS_OBJECTS,
                budgets.max_derived_cas_objects,
                "derived"
            )
            .is_ok()
        );
        assert!(
            enforce_exact(
                MAX_CHANGE_DERIVED_CAS_OBJECTS + 1,
                budgets.max_derived_cas_objects,
                "derived"
            )
            .is_err()
        );
    }

    #[test]
    fn comparison_digest_domain_makes_target_ids_authority_specific() {
        let left = coverage_target(&digest("comparison-left"), "same");
        let right = coverage_target(&digest("comparison-right"), "same");
        assert_ne!(left.target_id, right.target_id);
    }

    #[test]
    fn exact_rules_mapping_is_part_of_rules_digest() {
        let original = KotlinChangeRules {
            schema: KOTLIN_CHANGE_RULES_SCHEMA,
            rows: vec![rule(
                ChangeRuleInput::JvmDescriptor,
                KotlinChangeCode::KcdJvmDescriptorChanged,
            )],
            verdict_semantics: ChangeRuleSemantics::ObservationOnly,
        };
        let changed = KotlinChangeRules {
            schema: KOTLIN_CHANGE_RULES_SCHEMA,
            rows: vec![rule(
                ChangeRuleInput::JvmDescriptor,
                KotlinChangeCode::KcdReturnTypeChanged,
            )],
            verdict_semantics: ChangeRuleSemantics::ObservationOnly,
        };
        assert_ne!(
            rules_digest_for(&original).unwrap(),
            rules_digest_for(&changed).unwrap()
        );
    }

    #[test]
    fn closure_unions_deduplicates_sorts_and_enforces_exact_byte_bound() {
        let first = CasObject::for_bytes("proof/1.0", b"first").unwrap();
        let second = CasObject::for_bytes("proof/1.0", b"second").unwrap();
        let (closure, bytes) = canonical_closure(
            vec![second.clone(), first.clone(), first.clone()],
            first.size as usize + second.size as usize,
        )
        .unwrap();
        assert_eq!(closure.len(), 2);
        assert_eq!(bytes, first.size as usize + second.size as usize);
        assert!(canonical_closure(vec![first, second], bytes - 1).is_err());
    }

    #[test]
    fn frozen_budgets_reject_every_caller_override() {
        let mut budgets = ChangeSetBudgets::frozen();
        assert!(budgets.validate().is_ok());
        budgets.max_observations -= 1;
        assert!(budgets.validate().is_err());
    }

    #[test]
    fn validator_runtime_requires_sha256_but_is_independent_of_parent_runtime() {
        let key = digest("runtime");
        let manifest = digest("manifest");
        assert!(
            validate_validator_runtime(&ValidatorRuntimeAuthority {
                runtime_key: key.clone(),
                runtime_mode: crate::runtime::RuntimeMode::Development,
                manifest_digest: manifest.clone(),
            })
            .is_ok()
        );
        assert!(
            validate_validator_runtime(&ValidatorRuntimeAuthority {
                runtime_key: "runtime:ambient".into(),
                runtime_mode: crate::runtime::RuntimeMode::Development,
                manifest_digest: manifest,
            })
            .is_err()
        );
        assert!(
            validate_validator_runtime(&ValidatorRuntimeAuthority {
                runtime_key: key,
                runtime_mode: crate::runtime::RuntimeMode::Release,
                manifest_digest: "manifest:ambient".into(),
            })
            .is_err()
        );
    }
}
