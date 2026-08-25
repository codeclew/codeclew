//! Pure, bounded Kotlin callable impact selection.
//!
//! The module consumes an already verified [`PreparedCallableFactSet`].  It
//! does not read source files, open the CAS, start a process, mutate managed
//! state, or publish a root.  Instead it predicts the one evidence CAS object,
//! its retained proof closure, the private authority bytes, and the compact
//! public projection.  A service layer may atomically publish those prepared
//! bytes only after revalidating the owning thread authority.

use crate::canonical;
use crate::cas::{CAS_OBJECT_SCHEMA, CasObject};
use crate::error::{ClewError, ErrorCode};
use crate::thread_callables::{
    self, CallableFact, CallableFactKind, CallableFactSetCertainty, CallableFactShard,
    CallableLookup, CallableMemberBinding, CallablePairBinding, CallableQueryRequest,
    GraphCoverage, PreparedCallableFactSet, PreparedCasObject, RelationshipAuthority, SourceAnchor,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const THREAD_IMPACT_EVIDENCE_SCHEMA: &str = "codeclew-kotlin-thread-impact-evidence/1.0";
pub const THREAD_IMPACT_AUTHORITY_SCHEMA: &str = "codeclew-kotlin-thread-impact/1.0";
pub const THREAD_IMPACT_PROJECTION_SCHEMA: &str = "codeclew-kotlin-thread-impact-projection/1.0";

pub const MAX_IMPACT_FINDINGS: usize = 4_096;
pub const MAX_IMPACT_OBLIGATIONS: usize = 4_096;
pub const MAX_IMPACT_SOURCE_WINDOWS: usize = 32;
pub const MAX_IMPACT_SOURCE_WINDOW_BYTES: usize = 256 * 1024;
pub const MAX_IMPACT_DERIVED_CAS_OBJECTS: usize = 64;
pub const MAX_IMPACT_RETAINED_CLOSURE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_IMPACT_STDOUT_BYTES: usize = 64 * 1024;
const IMPACT_STDOUT_ENVELOPE_RESERVE: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactBudgets {
    pub max_findings: usize,
    pub max_obligations: usize,
    pub max_source_windows: usize,
    pub max_source_window_bytes: usize,
    pub max_derived_cas_objects: usize,
    pub max_retained_closure_bytes: usize,
    pub max_stdout_bytes: usize,
}

impl ImpactBudgets {
    pub fn frozen() -> Self {
        Self {
            max_findings: MAX_IMPACT_FINDINGS,
            max_obligations: MAX_IMPACT_OBLIGATIONS,
            max_source_windows: MAX_IMPACT_SOURCE_WINDOWS,
            max_source_window_bytes: MAX_IMPACT_SOURCE_WINDOW_BYTES,
            max_derived_cas_objects: MAX_IMPACT_DERIVED_CAS_OBJECTS,
            max_retained_closure_bytes: MAX_IMPACT_RETAINED_CLOSURE_BYTES,
            max_stdout_bytes: MAX_IMPACT_STDOUT_BYTES,
        }
    }

    fn validate(&self) -> Result<(), ClewError> {
        if self != &Self::frozen() {
            return Err(invalid(
                "Kotlin impact budgets differ from the frozen profile",
            ));
        }
        Ok(())
    }
}

impl Default for ImpactBudgets {
    fn default() -> Self {
        Self::frozen()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum KotlinImpactSubject {
    FullSymbol {
        symbol_identity: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        member_alias: Option<String>,
    },
    CallableFamily {
        callable_id: String,
    },
    Token {
        term: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImpactSubjectKind {
    FullSymbol,
    CallableFamily,
    Token,
}

impl KotlinImpactSubject {
    pub fn kind(&self) -> ImpactSubjectKind {
        match self {
            Self::FullSymbol { .. } => ImpactSubjectKind::FullSymbol,
            Self::CallableFamily { .. } => ImpactSubjectKind::CallableFamily,
            Self::Token { .. } => ImpactSubjectKind::Token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadImpactRequest {
    pub fact_set_authority_digest: String,
    pub pair_id: String,
    pub subject: KotlinImpactSubject,
    pub budgets: ImpactBudgets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImpactSide {
    Provider,
    Consumer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImpactShapeStatus {
    ExactProjectedShapeEqual,
    ExactProjectedShapeDelta,
    Unsure,
    NotComparable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImpactCertainty {
    Verified,
    Unsure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImpactFindingAuthority {
    ExactProjectedDeclaration,
    NavigationOnly,
    Unsure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactFactPointer {
    pub fact_id: String,
    pub fact_shard_ref: CasObject,
    pub provenance: thread_callables::CallableFactProvenance,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactFinding {
    pub finding_id: String,
    pub side: ImpactSide,
    pub member_alias: String,
    pub fact_kind: CallableFactKind,
    pub authority: ImpactFindingAuthority,
    pub evidence: ImpactFactPointer,
    pub detail: ImpactFindingDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "detail",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ImpactFindingDetail {
    Declaration {
        declaration_kind: thread_callables::DeclarationKind,
        symbol_identity: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        compiler_callable_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        compiler_class_id: Option<String>,
        projected_shape: Value,
    },
    Use {
        relation_kind: String,
        source_owner: String,
        target_callable_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_symbol_identity: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_repository_namespace: Option<String>,
        target_resolution: thread_callables::TargetResolution,
    },
    Boundary {
        stage: String,
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        required_checks: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactPublicFinding {
    pub finding_id: String,
    pub side: ImpactSide,
    pub member_alias: String,
    pub fact_id: String,
    pub authority: ImpactFindingAuthority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceAnchor>,
    pub detail: ImpactFindingDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactSourceWindow {
    pub window_id: String,
    pub side: ImpactSide,
    pub member_alias: String,
    pub anchor: SourceAnchor,
    pub span_bytes: usize,
    pub finding_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImpactObligationCode {
    SubjectNotObservedInMember,
    ProjectedDeclarationNotObserved,
    DisambiguateOverloadSet,
    CompleteDescriptorScope,
    VerifyDeclarationEvidence,
    VerifyUseEvidence,
    VerifyRelatedBoundary,
    VerifyBoundaryCheck,
    VerifyFactSetBoundaryScope,
    VerifyRelationshipAuthority,
    ResolveNavigationSubject,
    NarrowOrExpandQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactObligation {
    pub code: ImpactObligationCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_check: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactMemberResult {
    pub side: ImpactSide,
    pub member_alias: String,
    pub repository_namespace: String,
    pub observed: bool,
    pub matched_finding_count: usize,
    pub selected_finding_count: usize,
    pub declaration_count: usize,
    pub use_count: usize,
    pub boundary_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_shape_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactQuerySelection {
    pub schema: String,
    pub binding_digest: String,
    pub fact_set_authority_digest: String,
    pub pair: CallablePairBinding,
    pub subject: KotlinImpactSubject,
    pub query_index_ref: CasObject,
    pub fact_set_evidence_ref: CasObject,
    pub consulted_query_shard_refs: Vec<CasObject>,
    pub relationship_authority: RelationshipAuthority,
    pub shape_status: ImpactShapeStatus,
    pub certainty: ImpactCertainty,
    pub members: Vec<ImpactMemberResult>,
    pub findings: Vec<ImpactFinding>,
    pub obligation_evidence: Vec<ImpactFactPointer>,
    pub source_windows: Vec<ImpactSourceWindow>,
    pub obligations: Vec<ImpactObligation>,
    pub findings_truncated: bool,
    pub source_windows_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadImpactEvidence {
    pub schema: String,
    pub selection: ImpactQuerySelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadImpactAuthority {
    pub schema: String,
    pub authority_digest: String,
    pub binding_digest: String,
    pub fact_set_authority_digest: String,
    pub request: ThreadImpactRequest,
    pub pair: CallablePairBinding,
    pub relationship_authority: RelationshipAuthority,
    pub shape_status: ImpactShapeStatus,
    pub certainty: ImpactCertainty,
    pub members: Vec<ImpactMemberResult>,
    pub finding_count: usize,
    pub source_window_count: usize,
    pub obligation_count: usize,
    pub findings_truncated: bool,
    pub source_windows_truncated: bool,
    pub public_findings: Vec<ImpactPublicFinding>,
    pub public_findings_truncated: bool,
    pub public_obligations: Vec<ImpactObligation>,
    pub public_source_windows: Vec<ImpactSourceWindow>,
    pub evidence_ref: CasObject,
    pub direct_cas_closure: Vec<CasObject>,
    pub retained_cas_bytes: usize,
    pub new_derived_cas_object_count: usize,
    pub budgets: ImpactBudgets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactMemberProjection {
    pub side: ImpactSide,
    pub member_alias: String,
    pub observed: bool,
    pub matched_finding_count: usize,
    pub selected_finding_count: usize,
    pub declaration_count: usize,
    pub use_count: usize,
    pub boundary_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_shape_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadImpactProjection {
    pub schema: String,
    pub impact_id: String,
    pub authority_digest: String,
    pub binding_digest: String,
    pub fact_set_authority_digest: String,
    pub pair_id: String,
    pub subject_kind: ImpactSubjectKind,
    pub relationship_authority: RelationshipAuthority,
    pub shape_status: ImpactShapeStatus,
    pub certainty: ImpactCertainty,
    pub members: Vec<ImpactMemberProjection>,
    pub finding_count: usize,
    pub source_window_count: usize,
    pub obligation_count: usize,
    pub findings_truncated: bool,
    pub source_windows_truncated: bool,
    pub findings: Vec<ImpactPublicFinding>,
    pub public_findings_truncated: bool,
    pub obligations: Vec<ImpactObligation>,
    pub source_windows: Vec<ImpactSourceWindow>,
    pub evidence_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedThreadImpact {
    pub authority: ThreadImpactAuthority,
    pub evidence: ThreadImpactEvidence,
    pub evidence_object: PreparedCasObject,
    pub authority_bytes: Vec<u8>,
    pub projection: ThreadImpactProjection,
    pub projection_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImpactBindingMaterial<'a> {
    schema: &'static str,
    fact_set_authority_digest: &'a str,
    request: &'a ThreadImpactRequest,
    pair: &'a CallablePairBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImpactAuthorityMaterial<'a> {
    schema: &'a str,
    binding_digest: &'a str,
    fact_set_authority_digest: &'a str,
    request: &'a ThreadImpactRequest,
    pair: &'a CallablePairBinding,
    relationship_authority: RelationshipAuthority,
    shape_status: ImpactShapeStatus,
    certainty: ImpactCertainty,
    members: &'a [ImpactMemberResult],
    finding_count: usize,
    source_window_count: usize,
    obligation_count: usize,
    findings_truncated: bool,
    source_windows_truncated: bool,
    public_findings: &'a [ImpactPublicFinding],
    public_findings_truncated: bool,
    public_obligations: &'a [ImpactObligation],
    public_source_windows: &'a [ImpactSourceWindow],
    evidence_ref: &'a CasObject,
    direct_cas_closure: &'a [CasObject],
    retained_cas_bytes: usize,
    new_derived_cas_object_count: usize,
    budgets: &'a ImpactBudgets,
}

#[derive(Debug, Clone)]
struct FactEntry {
    fact: CallableFact,
    shard_ref: CasObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Lane {
    side: ImpactSide,
    kind: CallableFactKind,
}

#[derive(Debug, Clone)]
struct Candidate {
    side: ImpactSide,
    member_alias: String,
    fact: CallableFact,
    shard_ref: CasObject,
}

impl Candidate {
    fn lane(&self) -> Lane {
        Lane {
            side: self.side,
            kind: fact_kind(&self.fact),
        }
    }
}

/// Select a deterministic, fair, bounded set of impact evidence without
/// producing or publishing any new CAS object.
pub fn select(
    prepared: &PreparedCallableFactSet,
    mut request: ThreadImpactRequest,
) -> Result<ImpactQuerySelection, ClewError> {
    thread_callables::verify_prepared(prepared)?;
    normalize_request(prepared, &mut request)?;
    select_verified(prepared, &request)
}

/// Build all bytes and CAS identities required for an atomic impact publish.
pub fn build(
    prepared: &PreparedCallableFactSet,
    request: ThreadImpactRequest,
) -> Result<PreparedThreadImpact, ClewError> {
    thread_callables::verify_prepared(prepared)?;
    build_from_verified(prepared, request)
}

pub(crate) fn build_from_verified(
    prepared: &PreparedCallableFactSet,
    mut request: ThreadImpactRequest,
) -> Result<PreparedThreadImpact, ClewError> {
    normalize_request(prepared, &mut request)?;
    build_verified(prepared, request)
}

/// Build after the managed service has loaded and fully verified the parent
/// fact set exactly once.  Callers outside the crate use [`build`].
pub(crate) fn build_verified(
    prepared: &PreparedCallableFactSet,
    request: ThreadImpactRequest,
) -> Result<PreparedThreadImpact, ClewError> {
    let selection = select_verified(prepared, &request)?;
    let evidence = ThreadImpactEvidence {
        schema: THREAD_IMPACT_EVIDENCE_SCHEMA.into(),
        selection: selection.clone(),
    };
    let evidence_bytes = canonical::bytes(&evidence).map_err(internal)?;
    if evidence_bytes.len() > request.budgets.max_retained_closure_bytes {
        return Err(budget(
            "Kotlin impact evidence exceeds the retained byte bound",
        ));
    }
    let evidence_ref = CasObject::for_bytes(THREAD_IMPACT_EVIDENCE_SCHEMA, &evidence_bytes)?;
    let evidence_object = PreparedCasObject {
        reference: evidence_ref.clone(),
        bytes: evidence_bytes,
    };

    let pair_members = pair_members(prepared, &selection.pair)?;
    let mut closure = selected_proof_closure(&selection, &pair_members);
    closure.push(evidence_ref.clone());
    let (direct_cas_closure, retained_cas_bytes) =
        canonical_cas_closure(closure, request.budgets.max_retained_closure_bytes)?;
    let new_derived_cas_object_count =
        checked_derived_cas_object_count(1, request.budgets.max_derived_cas_objects)?;

    let mut authority = ThreadImpactAuthority {
        schema: THREAD_IMPACT_AUTHORITY_SCHEMA.into(),
        authority_digest: String::new(),
        binding_digest: selection.binding_digest.clone(),
        fact_set_authority_digest: prepared.authority.authority_digest.clone(),
        request: request.clone(),
        pair: selection.pair.clone(),
        relationship_authority: selection.relationship_authority,
        shape_status: selection.shape_status,
        certainty: selection.certainty,
        members: selection.members.clone(),
        finding_count: selection.findings.len(),
        source_window_count: selection.source_windows.len(),
        obligation_count: selection.obligations.len(),
        findings_truncated: selection.findings_truncated,
        source_windows_truncated: selection.source_windows_truncated,
        public_findings: selection.findings.iter().map(public_finding).collect(),
        public_findings_truncated: selection.findings_truncated,
        public_obligations: selection.obligations.clone(),
        public_source_windows: selection.source_windows.clone(),
        evidence_ref,
        direct_cas_closure,
        retained_cas_bytes,
        new_derived_cas_object_count,
        budgets: request.budgets.clone(),
    };
    loop {
        authority.authority_digest = authority_digest(&authority)?;
        let authority_bytes = canonical::bytes(&authority).map_err(internal)?;
        let projection = project(&authority);
        let projection_bytes = canonical::bytes(&projection).map_err(internal)?;
        if projection_bytes
            .len()
            .saturating_add(IMPACT_STDOUT_ENVELOPE_RESERVE)
            <= request.budgets.max_stdout_bytes
        {
            return Ok(PreparedThreadImpact {
                authority,
                evidence,
                evidence_object,
                authority_bytes,
                projection,
                projection_bytes,
            });
        }
        if authority.public_findings.pop().is_none() {
            return Err(budget(
                "mandatory Kotlin impact obligations/source anchors exceed the stdout bound",
            ));
        }
        authority.public_findings_truncated = true;
        authority.authority_digest.clear();
    }
}

/// Reconstruct an impact authority from its immutable parent fact set and
/// reject any binding, evidence, closure, or projection substitution.
pub fn verify_prepared(
    fact_set: &PreparedCallableFactSet,
    prepared: &PreparedThreadImpact,
) -> Result<(), ClewError> {
    thread_callables::verify_prepared(fact_set)?;
    verify_prepared_from_verified(fact_set, prepared)
}

pub(crate) fn verify_prepared_from_verified(
    fact_set: &PreparedCallableFactSet,
    prepared: &PreparedThreadImpact,
) -> Result<(), ClewError> {
    let mut request = prepared.authority.request.clone();
    normalize_request(fact_set, &mut request)
        .map_err(|_| corrupt("prepared impact request is invalid"))?;
    let expected = build_verified(fact_set, request)
        .map_err(|_| corrupt("prepared impact cannot be reconstructed"))?;
    if &expected != prepared {
        return Err(corrupt(
            "prepared impact authority/evidence/projection was substituted",
        ));
    }
    Ok(())
}

/// Produce the compact, path-free public projection of an impact authority.
pub fn project(authority: &ThreadImpactAuthority) -> ThreadImpactProjection {
    ThreadImpactProjection {
        schema: THREAD_IMPACT_PROJECTION_SCHEMA.into(),
        impact_id: format!("thread-impact:{}", authority.authority_digest),
        authority_digest: authority.authority_digest.clone(),
        binding_digest: authority.binding_digest.clone(),
        fact_set_authority_digest: authority.fact_set_authority_digest.clone(),
        pair_id: authority.pair.pair_id.clone(),
        subject_kind: authority.request.subject.kind(),
        relationship_authority: authority.relationship_authority,
        shape_status: authority.shape_status,
        certainty: authority.certainty,
        members: authority
            .members
            .iter()
            .map(|member| ImpactMemberProjection {
                side: member.side,
                member_alias: member.member_alias.clone(),
                observed: member.observed,
                matched_finding_count: member.matched_finding_count,
                selected_finding_count: member.selected_finding_count,
                declaration_count: member.declaration_count,
                use_count: member.use_count,
                boundary_count: member.boundary_count,
                exact_shape_digest: member.exact_shape_digest.clone(),
            })
            .collect(),
        finding_count: authority.finding_count,
        source_window_count: authority.source_window_count,
        obligation_count: authority.obligation_count,
        findings_truncated: authority.findings_truncated,
        source_windows_truncated: authority.source_windows_truncated,
        findings: authority.public_findings.clone(),
        public_findings_truncated: authority.public_findings_truncated,
        obligations: authority.public_obligations.clone(),
        source_windows: authority.public_source_windows.clone(),
        evidence_ref: authority.evidence_ref.clone(),
    }
}

pub(crate) fn select_verified(
    prepared: &PreparedCallableFactSet,
    request: &ThreadImpactRequest,
) -> Result<ImpactQuerySelection, ClewError> {
    let pair = selected_pair(prepared, &request.pair_id)?;
    let members = pair_members(prepared, &pair)?;
    let binding_digest = canonical::hash(&ImpactBindingMaterial {
        schema: "codeclew-kotlin-thread-impact-binding/1.0",
        fact_set_authority_digest: &prepared.authority.authority_digest,
        request,
        pair: &pair,
    })
    .map_err(internal)?;
    let entries = fact_entries(prepared)?;
    let lookup = subject_lookup(&request.subject, &members)?;
    let query_trace = thread_callables::query_verified_with_trace(
        prepared,
        CallableQueryRequest {
            lookups: vec![lookup],
            max_results: MAX_IMPACT_FINDINGS,
        },
    )?;
    let matched_fact_ids = query_trace
        .matched_postings
        .iter()
        .map(|posting| posting.fact_id.clone())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for (side, member) in &members {
        let related_declarations = entries
            .iter()
            .filter_map(|entry| match &entry.fact {
                CallableFact::Declaration(row)
                    if row.provenance.member_alias == member.member_alias
                        && matched_fact_ids.contains(&row.fact_id) =>
                {
                    Some(row)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for entry in &entries {
            if fact_provenance(&entry.fact).member_alias != member.member_alias {
                continue;
            }
            if fact_matches(
                &entry.fact,
                &request.subject,
                &matched_fact_ids,
                &related_declarations,
            ) {
                candidates.push(Candidate {
                    side: *side,
                    member_alias: member.member_alias.clone(),
                    fact: entry.fact.clone(),
                    shard_ref: entry.shard_ref.clone(),
                });
            }
        }
    }
    candidates.sort_by(|left, right| {
        (left.lane(), left.fact.fact_id()).cmp(&(right.lane(), right.fact.fact_id()))
    });

    let mut obligations = BTreeSet::new();
    let analyses = members
        .iter()
        .map(|(side, member)| {
            analyze_member(
                *side,
                member,
                &request.subject,
                candidates
                    .iter()
                    .filter(|candidate| candidate.side == *side)
                    .collect(),
                &mut obligations,
            )
        })
        .collect::<Vec<_>>();
    let shape_status = shape_status(&request.subject, &analyses);

    if prepared.authority.completeness.certainty == CallableFactSetCertainty::Unsure {
        obligations.insert(obligation(
            ImpactObligationCode::VerifyFactSetBoundaryScope,
            None,
            None,
            None,
        ));
    }
    if pair.relationship_authority
        != RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
    {
        obligations.insert(obligation(
            ImpactObligationCode::VerifyRelationshipAuthority,
            None,
            None,
            None,
        ));
    }
    if matches!(request.subject, KotlinImpactSubject::Token { .. }) {
        obligations.insert(obligation(
            ImpactObligationCode::ResolveNavigationSubject,
            None,
            None,
            None,
        ));
    }

    add_fact_obligations(&candidates, &mut obligations);
    if obligations.len() > request.budgets.max_obligations {
        return Err(budget(
            "Kotlin impact obligations exceed the fail-closed 4,096 bound",
        ));
    }
    let mut candidate_by_fact_id = BTreeMap::new();
    for candidate in &candidates {
        if candidate_by_fact_id
            .insert(candidate.fact.fact_id().to_owned(), candidate.clone())
            .is_some()
        {
            return Err(corrupt("impact candidate fact identity is not unique"));
        }
    }
    let (selected_candidates, findings_truncated) =
        fair_take_candidates(candidates, request.budgets.max_findings);
    if findings_truncated {
        obligations.insert(obligation(
            ImpactObligationCode::NarrowOrExpandQuery,
            None,
            None,
            None,
        ));
    }
    let findings = selected_candidates
        .into_iter()
        .map(|candidate| finding_from_candidate(&binding_digest, candidate, &request.subject))
        .collect::<Result<Vec<_>, _>>()?;
    let (source_windows, source_windows_truncated) = select_source_windows(
        &binding_digest,
        &findings,
        request.budgets.max_source_windows,
        request.budgets.max_source_window_bytes,
    )?;
    if source_windows_truncated {
        obligations.insert(obligation(
            ImpactObligationCode::NarrowOrExpandQuery,
            None,
            None,
            None,
        ));
    }
    let obligations = checked_obligations(obligations, request.budgets.max_obligations)?;
    let obligation_evidence = obligation_evidence(&obligations, &candidate_by_fact_id)?;

    let mut member_results = analyses
        .into_iter()
        .map(|analysis| analysis.result)
        .collect::<Vec<_>>();
    for result in &mut member_results {
        result.selected_finding_count = findings
            .iter()
            .filter(|finding| finding.side == result.side)
            .count();
    }
    let certainty = if pair.relationship_authority
        == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
        && matches!(
            shape_status,
            ImpactShapeStatus::ExactProjectedShapeEqual
                | ImpactShapeStatus::ExactProjectedShapeDelta
        )
        && obligations.is_empty()
        && !findings_truncated
        && !source_windows_truncated
    {
        ImpactCertainty::Verified
    } else {
        ImpactCertainty::Unsure
    };

    Ok(ImpactQuerySelection {
        schema: "codeclew-kotlin-thread-impact-selection/1.0".into(),
        binding_digest,
        fact_set_authority_digest: prepared.authority.authority_digest.clone(),
        pair: pair.clone(),
        subject: request.subject.clone(),
        query_index_ref: prepared.authority.query_index_ref.clone(),
        fact_set_evidence_ref: prepared.authority.evidence_ref.clone(),
        consulted_query_shard_refs: query_trace.query_shard_refs,
        relationship_authority: pair.relationship_authority,
        shape_status,
        certainty,
        members: member_results,
        findings,
        obligation_evidence,
        source_windows,
        obligations,
        findings_truncated,
        source_windows_truncated,
    })
}

#[derive(Debug)]
struct MemberAnalysis {
    result: ImpactMemberResult,
}

fn analyze_member(
    side: ImpactSide,
    member: &CallableMemberBinding,
    subject: &KotlinImpactSubject,
    candidates: Vec<&Candidate>,
    obligations: &mut BTreeSet<ImpactObligation>,
) -> MemberAnalysis {
    let declarations = candidates
        .iter()
        .filter_map(|candidate| match &candidate.fact {
            CallableFact::Declaration(row) => Some(row),
            _ => None,
        })
        .collect::<Vec<_>>();
    let use_count = candidates
        .iter()
        .filter(|candidate| matches!(candidate.fact, CallableFact::Use(_)))
        .count();
    let boundaries = candidates
        .iter()
        .filter_map(|candidate| match &candidate.fact {
            CallableFact::Boundary(row) => Some(row),
            _ => None,
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        obligations.insert(obligation(
            ImpactObligationCode::SubjectNotObservedInMember,
            Some(&member.member_alias),
            None,
            None,
        ));
    }
    if declarations.is_empty() {
        obligations.insert(obligation(
            ImpactObligationCode::ProjectedDeclarationNotObserved,
            Some(&member.member_alias),
            None,
            None,
        ));
    }
    if declarations.len() > 1 {
        obligations.insert(obligation(
            ImpactObligationCode::DisambiguateOverloadSet,
            Some(&member.member_alias),
            None,
            None,
        ));
    }
    let descriptor_complete = descriptor_scope_complete(member);
    if matches!(subject, KotlinImpactSubject::CallableFamily { .. }) && !descriptor_complete {
        obligations.insert(obligation(
            ImpactObligationCode::CompleteDescriptorScope,
            Some(&member.member_alias),
            None,
            None,
        ));
    }
    let exact_shape_digest = if !matches!(subject, KotlinImpactSubject::Token { .. })
        && declarations.len() == 1
        && boundaries.is_empty()
        && declarations[0].exact_eligible
        && declarations[0].shape_digest.is_some()
        && (!matches!(subject, KotlinImpactSubject::CallableFamily { .. }) || descriptor_complete)
    {
        declarations[0].shape_digest.clone()
    } else {
        None
    };
    MemberAnalysis {
        result: ImpactMemberResult {
            side,
            member_alias: member.member_alias.clone(),
            repository_namespace: member.repository_namespace.clone(),
            observed: !candidates.is_empty(),
            matched_finding_count: candidates.len(),
            selected_finding_count: 0,
            declaration_count: declarations.len(),
            use_count,
            boundary_count: boundaries.len(),
            exact_shape_digest,
        },
    }
}

fn shape_status(subject: &KotlinImpactSubject, analyses: &[MemberAnalysis]) -> ImpactShapeStatus {
    if matches!(subject, KotlinImpactSubject::Token { .. }) {
        return ImpactShapeStatus::NotComparable;
    }
    let provider = analyses
        .iter()
        .find(|analysis| analysis.result.side == ImpactSide::Provider)
        .expect("pair always has provider analysis");
    let consumer = analyses
        .iter()
        .find(|analysis| analysis.result.side == ImpactSide::Consumer)
        .expect("pair always has consumer analysis");
    match (
        &provider.result.exact_shape_digest,
        &consumer.result.exact_shape_digest,
    ) {
        (Some(left), Some(right)) if left == right => ImpactShapeStatus::ExactProjectedShapeEqual,
        (Some(_), Some(_)) => ImpactShapeStatus::ExactProjectedShapeDelta,
        _ if provider.result.declaration_count == 0 || consumer.result.declaration_count == 0 => {
            ImpactShapeStatus::NotComparable
        }
        _ => ImpactShapeStatus::Unsure,
    }
}

fn add_fact_obligations(candidates: &[Candidate], obligations: &mut BTreeSet<ImpactObligation>) {
    for candidate in candidates {
        match &candidate.fact {
            CallableFact::Declaration(row) if !row.exact_eligible => {
                obligations.insert(obligation(
                    ImpactObligationCode::VerifyDeclarationEvidence,
                    Some(&candidate.member_alias),
                    Some(&row.fact_id),
                    None,
                ));
            }
            CallableFact::Use(row) if !row.exact_eligible => {
                obligations.insert(obligation(
                    ImpactObligationCode::VerifyUseEvidence,
                    Some(&candidate.member_alias),
                    Some(&row.fact_id),
                    None,
                ));
            }
            CallableFact::Boundary(row) => {
                obligations.insert(obligation(
                    ImpactObligationCode::VerifyRelatedBoundary,
                    Some(&candidate.member_alias),
                    Some(&row.fact_id),
                    None,
                ));
                for check in &row.required_checks {
                    obligations.insert(obligation(
                        ImpactObligationCode::VerifyBoundaryCheck,
                        Some(&candidate.member_alias),
                        Some(&row.fact_id),
                        Some(check),
                    ));
                }
            }
            _ => {}
        }
    }
}

fn obligation_evidence(
    obligations: &[ImpactObligation],
    candidates: &BTreeMap<String, Candidate>,
) -> Result<Vec<ImpactFactPointer>, ClewError> {
    let mut pointers = BTreeMap::new();
    for fact_id in obligations
        .iter()
        .filter_map(|obligation| obligation.fact_id.as_ref())
    {
        let candidate = candidates
            .get(fact_id)
            .ok_or_else(|| corrupt("fact-specific impact obligation has no candidate evidence"))?;
        pointers
            .entry(fact_id.clone())
            .or_insert_with(|| fact_pointer(candidate));
    }
    Ok(pointers.into_values().collect())
}

fn fact_pointer(candidate: &Candidate) -> ImpactFactPointer {
    let (provenance, shape_digest) = match &candidate.fact {
        CallableFact::Declaration(row) => (row.provenance.clone(), row.shape_digest.clone()),
        CallableFact::Use(row) => (row.provenance.clone(), None),
        CallableFact::Boundary(row) => (row.provenance.clone(), None),
    };
    ImpactFactPointer {
        fact_id: candidate.fact.fact_id().to_owned(),
        fact_shard_ref: candidate.shard_ref.clone(),
        provenance,
        shape_digest,
    }
}

fn finding_from_candidate(
    binding_digest: &str,
    candidate: Candidate,
    subject: &KotlinImpactSubject,
) -> Result<ImpactFinding, ClewError> {
    let (provenance, shape_digest, row_exact, detail) = match &candidate.fact {
        CallableFact::Declaration(row) => (
            row.provenance.clone(),
            row.shape_digest.clone(),
            row.exact_eligible,
            ImpactFindingDetail::Declaration {
                declaration_kind: row.declaration_kind,
                symbol_identity: row.symbol_identity.clone(),
                compiler_callable_id: row.compiler_callable_id.clone(),
                compiler_class_id: row.compiler_class_id.clone(),
                projected_shape: row.projected_shape.clone(),
            },
        ),
        CallableFact::Use(row) => (
            row.provenance.clone(),
            None,
            row.exact_eligible,
            ImpactFindingDetail::Use {
                relation_kind: row.relation_kind.clone(),
                source_owner: row.source_owner.clone(),
                target_callable_id: row.target_callable_id.clone(),
                target_symbol_identity: row.target_symbol_identity.clone(),
                target_repository_namespace: row.target_repository_namespace.clone(),
                target_resolution: row.target_resolution,
            },
        ),
        CallableFact::Boundary(row) => (
            row.provenance.clone(),
            None,
            false,
            ImpactFindingDetail::Boundary {
                stage: row.stage.clone(),
                code: row.code.clone(),
                subject: row.subject.clone(),
                required_checks: row.required_checks.clone(),
            },
        ),
    };
    let fact_kind = fact_kind(&candidate.fact);
    let authority = match &candidate.fact {
        CallableFact::Declaration(_) if matches!(subject, KotlinImpactSubject::Token { .. }) => {
            ImpactFindingAuthority::NavigationOnly
        }
        // Exactness belongs to the individual compiler-projected declaration,
        // not to the aggregate family comparison.  A multi-overload family
        // therefore keeps exact rows while its aggregate shapeStatus remains
        // UNSURE until the overload set is disambiguated.
        CallableFact::Declaration(_) if row_exact && shape_digest.is_some() => {
            ImpactFindingAuthority::ExactProjectedDeclaration
        }
        CallableFact::Declaration(_) if row_exact => ImpactFindingAuthority::NavigationOnly,
        CallableFact::Use(_) if row_exact => ImpactFindingAuthority::NavigationOnly,
        CallableFact::Use(_) if matches!(subject, KotlinImpactSubject::Token { .. }) => {
            ImpactFindingAuthority::NavigationOnly
        }
        _ => ImpactFindingAuthority::Unsure,
    };
    let finding_id = canonical::hash(&(
        "codeclew-kotlin-thread-impact-finding/1.0",
        binding_digest,
        candidate.side,
        fact_kind,
        candidate.fact.fact_id(),
        &candidate.shard_ref,
    ))
    .map_err(internal)?;
    Ok(ImpactFinding {
        finding_id,
        side: candidate.side,
        member_alias: candidate.member_alias,
        fact_kind,
        authority,
        evidence: ImpactFactPointer {
            fact_id: candidate.fact.fact_id().to_owned(),
            fact_shard_ref: candidate.shard_ref,
            provenance,
            shape_digest,
        },
        detail,
    })
}

fn public_finding(finding: &ImpactFinding) -> ImpactPublicFinding {
    ImpactPublicFinding {
        finding_id: finding.finding_id.clone(),
        side: finding.side,
        member_alias: finding.member_alias.clone(),
        fact_id: finding.evidence.fact_id.clone(),
        authority: finding.authority,
        shape_digest: finding.evidence.shape_digest.clone(),
        source: finding.evidence.provenance.source.clone(),
        detail: finding.detail.clone(),
    }
}

fn select_source_windows(
    binding_digest: &str,
    findings: &[ImpactFinding],
    max_windows: usize,
    max_bytes: usize,
) -> Result<(Vec<ImpactSourceWindow>, bool), ClewError> {
    type WindowKey = (ImpactSide, String, String, Option<u64>, Option<u64>, String);
    let mut grouped = BTreeMap::<WindowKey, (SourceAnchor, BTreeSet<String>)>::new();
    for finding in findings {
        let Some(anchor) = &finding.evidence.provenance.source else {
            continue;
        };
        let key = (
            finding.side,
            finding.member_alias.clone(),
            anchor.path.clone(),
            anchor.start,
            anchor.end,
            anchor.content_ref.digest.clone(),
        );
        grouped
            .entry(key)
            .or_insert_with(|| (anchor.clone(), BTreeSet::new()))
            .1
            .insert(finding.finding_id.clone());
    }
    let mut lanes = BTreeMap::<ImpactSide, VecDeque<ImpactSourceWindow>>::new();
    for ((side, member_alias, _, _, _, _), (anchor, finding_ids)) in grouped {
        let span_bytes_u64 = match (anchor.start, anchor.end) {
            (Some(start), Some(end)) => end.saturating_sub(start),
            (None, None) => anchor.content_ref.size,
            _ => return Err(corrupt("impact source anchor range is inconsistent")),
        };
        let span_bytes = usize::try_from(span_bytes_u64)
            .map_err(|_| budget("impact source span is not representable"))?;
        let finding_ids = finding_ids.into_iter().collect::<Vec<_>>();
        let window_id = canonical::hash(&(
            "codeclew-kotlin-thread-impact-source-window/1.0",
            binding_digest,
            side,
            &member_alias,
            &anchor,
            &finding_ids,
        ))
        .map_err(internal)?;
        lanes
            .entry(side)
            .or_default()
            .push_back(ImpactSourceWindow {
                window_id,
                side,
                member_alias,
                anchor,
                span_bytes,
                finding_ids,
            });
    }
    let candidate_count = lanes.values().map(VecDeque::len).sum::<usize>();
    let mut selected = Vec::new();
    let mut selected_bytes = 0usize;
    'windows: loop {
        let mut progressed = false;
        for lane in [ImpactSide::Provider, ImpactSide::Consumer] {
            let Some(queue) = lanes.get_mut(&lane) else {
                continue;
            };
            let Some(window) = queue.pop_front() else {
                continue;
            };
            progressed = true;
            let Some(next_bytes) = selected_bytes.checked_add(window.span_bytes) else {
                break 'windows;
            };
            if selected.len() == max_windows || next_bytes > max_bytes {
                break 'windows;
            }
            selected_bytes = next_bytes;
            selected.push(window);
        }
        if !progressed {
            break;
        }
    }
    let truncated = selected.len() < candidate_count;
    Ok((selected, truncated))
}

fn fair_take_candidates(candidates: Vec<Candidate>, limit: usize) -> (Vec<Candidate>, bool) {
    let candidate_count = candidates.len();
    let mut lanes = BTreeMap::<Lane, VecDeque<Candidate>>::new();
    for candidate in candidates {
        lanes
            .entry(candidate.lane())
            .or_default()
            .push_back(candidate);
    }
    let lane_order = [
        Lane {
            side: ImpactSide::Provider,
            kind: CallableFactKind::Declaration,
        },
        Lane {
            side: ImpactSide::Provider,
            kind: CallableFactKind::Use,
        },
        Lane {
            side: ImpactSide::Provider,
            kind: CallableFactKind::Boundary,
        },
        Lane {
            side: ImpactSide::Consumer,
            kind: CallableFactKind::Declaration,
        },
        Lane {
            side: ImpactSide::Consumer,
            kind: CallableFactKind::Use,
        },
        Lane {
            side: ImpactSide::Consumer,
            kind: CallableFactKind::Boundary,
        },
    ];
    let mut selected = Vec::new();
    while selected.len() < limit {
        let mut progressed = false;
        for lane in lane_order {
            if selected.len() == limit {
                break;
            }
            if let Some(candidate) = lanes.get_mut(&lane).and_then(VecDeque::pop_front) {
                selected.push(candidate);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    (selected, candidate_count > limit)
}

fn selected_proof_closure(
    selection: &ImpactQuerySelection,
    members: &[(ImpactSide, &CallableMemberBinding)],
) -> Vec<CasObject> {
    let mut closure = Vec::new();
    for (_, member) in members {
        closure.push(member.snapshot_ref.clone());
        closure.extend(
            member
                .compilations
                .iter()
                .map(|compilation| compilation.generation_ref.clone()),
        );
    }
    if let Some(reference) = &selection.pair.dependency_evidence_ref {
        closure.push(reference.clone());
    }
    closure.extend([
        selection.query_index_ref.clone(),
        selection.fact_set_evidence_ref.clone(),
    ]);
    closure.extend(selection.consulted_query_shard_refs.iter().cloned());
    for finding in &selection.findings {
        closure.extend([
            finding.evidence.fact_shard_ref.clone(),
            finding.evidence.provenance.generation_ref.clone(),
            finding.evidence.provenance.input_payload_ref.clone(),
        ]);
        if let Some(source) = &finding.evidence.provenance.source {
            closure.push(source.content_ref.clone());
        }
    }
    for evidence in &selection.obligation_evidence {
        closure.extend([
            evidence.fact_shard_ref.clone(),
            evidence.provenance.generation_ref.clone(),
            evidence.provenance.input_payload_ref.clone(),
        ]);
        if let Some(source) = &evidence.provenance.source {
            closure.push(source.content_ref.clone());
        }
    }
    closure
}

fn canonical_cas_closure(
    references: Vec<CasObject>,
    max_bytes: usize,
) -> Result<(Vec<CasObject>, usize), ClewError> {
    let mut by_digest = BTreeMap::<String, CasObject>::new();
    for reference in references {
        if reference.schema != CAS_OBJECT_SCHEMA || reference.size == 0 {
            return Err(corrupt("impact closure contains an invalid CAS reference"));
        }
        match by_digest.entry(reference.digest.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(reference);
            }
            std::collections::btree_map::Entry::Occupied(entry) if entry.get() != &reference => {
                return Err(corrupt(
                    "impact closure repeats a digest with different authority",
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
    let retained_bytes = closure.iter().try_fold(0usize, |total, reference| {
        let size = usize::try_from(reference.size)
            .map_err(|_| budget("impact retained object size is not representable"))?;
        total
            .checked_add(size)
            .ok_or_else(|| budget("impact retained byte count overflowed"))
    })?;
    if retained_bytes > max_bytes {
        return Err(budget(
            "Kotlin impact selected proof exceeds the 64 MiB retained bound",
        ));
    }
    Ok((closure, retained_bytes))
}

fn checked_derived_cas_object_count(count: usize, max_count: usize) -> Result<usize, ClewError> {
    if count > max_count {
        return Err(budget(
            "Kotlin impact derived CAS object count exceeds the frozen bound",
        ));
    }
    Ok(count)
}

fn authority_digest(authority: &ThreadImpactAuthority) -> Result<String, ClewError> {
    canonical::hash(&ImpactAuthorityMaterial {
        schema: &authority.schema,
        binding_digest: &authority.binding_digest,
        fact_set_authority_digest: &authority.fact_set_authority_digest,
        request: &authority.request,
        pair: &authority.pair,
        relationship_authority: authority.relationship_authority,
        shape_status: authority.shape_status,
        certainty: authority.certainty,
        members: &authority.members,
        finding_count: authority.finding_count,
        source_window_count: authority.source_window_count,
        obligation_count: authority.obligation_count,
        findings_truncated: authority.findings_truncated,
        source_windows_truncated: authority.source_windows_truncated,
        public_findings: &authority.public_findings,
        public_findings_truncated: authority.public_findings_truncated,
        public_obligations: &authority.public_obligations,
        public_source_windows: &authority.public_source_windows,
        evidence_ref: &authority.evidence_ref,
        direct_cas_closure: &authority.direct_cas_closure,
        retained_cas_bytes: authority.retained_cas_bytes,
        new_derived_cas_object_count: authority.new_derived_cas_object_count,
        budgets: &authority.budgets,
    })
    .map_err(internal)
}

fn checked_obligations(
    obligations: BTreeSet<ImpactObligation>,
    max_obligations: usize,
) -> Result<Vec<ImpactObligation>, ClewError> {
    if obligations.len() > max_obligations {
        return Err(budget(
            "Kotlin impact obligations exceed the fail-closed 4,096 bound",
        ));
    }
    Ok(obligations.into_iter().collect())
}

fn obligation(
    code: ImpactObligationCode,
    member_alias: Option<&str>,
    fact_id: Option<&str>,
    required_check: Option<&str>,
) -> ImpactObligation {
    ImpactObligation {
        code,
        member_alias: member_alias.map(str::to_owned),
        fact_id: fact_id.map(str::to_owned),
        required_check: required_check.map(str::to_owned),
    }
}

fn fact_entries(prepared: &PreparedCallableFactSet) -> Result<Vec<FactEntry>, ClewError> {
    let mut entries = Vec::new();
    for (reference, object) in prepared
        .authority
        .fact_shards
        .iter()
        .zip(&prepared.fact_shards)
    {
        let shard: CallableFactShard = serde_json::from_slice(&object.bytes)
            .map_err(|_| corrupt("verified callable fact shard cannot be decoded"))?;
        entries.extend(shard.facts.into_iter().map(|fact| FactEntry {
            fact,
            shard_ref: reference.object.clone(),
        }));
    }
    Ok(entries)
}

fn subject_lookup(
    subject: &KotlinImpactSubject,
    members: &[(ImpactSide, &CallableMemberBinding)],
) -> Result<CallableLookup, ClewError> {
    Ok(match subject {
        KotlinImpactSubject::FullSymbol {
            symbol_identity,
            member_alias: Some(member_alias),
        } => {
            let member = members
                .iter()
                .find_map(|(_, member)| (&member.member_alias == member_alias).then_some(*member))
                .ok_or_else(|| invalid("full-symbol member is not in the selected pair"))?;
            CallableLookup::FullSymbol {
                repository_namespace: member.repository_namespace.clone(),
                symbol_identity: symbol_identity.clone(),
            }
        }
        KotlinImpactSubject::FullSymbol {
            member_alias: None, ..
        } => {
            return Err(invalid(
                "FULL_SYMBOL impact subject requires an explicit pair member",
            ));
        }
        KotlinImpactSubject::CallableFamily { callable_id } => CallableLookup::CallableFamily {
            repository_namespace: None,
            callable_id: callable_id.clone(),
        },
        KotlinImpactSubject::Token { term } => CallableLookup::Token { term: term.clone() },
    })
}

fn fact_matches(
    fact: &CallableFact,
    subject: &KotlinImpactSubject,
    matched_fact_ids: &BTreeSet<String>,
    related_declarations: &[&thread_callables::DeclarationFact],
) -> bool {
    match subject {
        KotlinImpactSubject::Token { .. } => matched_fact_ids.contains(fact.fact_id()),
        KotlinImpactSubject::FullSymbol {
            symbol_identity, ..
        } => match fact {
            CallableFact::Declaration(row) => matched_fact_ids.contains(&row.fact_id),
            CallableFact::Use(row) => matched_fact_ids.contains(&row.fact_id),
            CallableFact::Boundary(row) => {
                row.subject.as_ref() == Some(symbol_identity)
                    || boundary_matches_declarations(row, related_declarations)
            }
        },
        KotlinImpactSubject::CallableFamily { callable_id } => match fact {
            CallableFact::Declaration(row) => matched_fact_ids.contains(&row.fact_id),
            CallableFact::Use(row) => matched_fact_ids.contains(&row.fact_id),
            CallableFact::Boundary(row) => {
                row.subject.as_ref().is_some_and(|subject| {
                    subject == callable_id
                        || symbol_callable_family(subject) == Some(callable_id.as_str())
                }) || boundary_matches_declarations(row, related_declarations)
            }
        },
    }
}

fn boundary_matches_declarations(
    boundary: &thread_callables::BoundaryFact,
    declarations: &[&thread_callables::DeclarationFact],
) -> bool {
    if !boundary
        .provenance
        .input_fact_key
        .starts_with("kotlin:descriptor-boundary:")
    {
        return false;
    }
    let Some(subject) = boundary.subject.as_deref() else {
        return false;
    };
    declarations.iter().any(|declaration| {
        declaration.provenance.member_alias == boundary.provenance.member_alias
            && declaration.provenance.compilation_id == boundary.provenance.compilation_id
            && (subject == declaration.symbol_identity
                || declaration.compiler_callable_id.as_deref() == Some(subject)
                || declaration.compiler_class_id.as_deref() == Some(subject)
                || subject == declaration.owner_identity
                || declaration.containment.iter().any(|owner| owner == subject))
    })
}

fn symbol_callable_family(symbol: &str) -> Option<&str> {
    symbol
        .strip_prefix("callable:")?
        .split_once("#jvm:")
        .map(|(family, _)| family)
}

fn selected_pair(
    prepared: &PreparedCallableFactSet,
    pair_id: &str,
) -> Result<CallablePairBinding, ClewError> {
    let mut matches = prepared
        .authority
        .pairs
        .iter()
        .filter(|pair| pair.pair_id == pair_id);
    let pair = matches
        .next()
        .ok_or_else(|| invalid("selected Kotlin impact pair does not exist"))?;
    if matches.next().is_some() {
        return Err(corrupt("verified callable fact set repeats a pair id"));
    }
    Ok(pair.clone())
}

fn pair_members<'a>(
    prepared: &'a PreparedCallableFactSet,
    pair: &CallablePairBinding,
) -> Result<Vec<(ImpactSide, &'a CallableMemberBinding)>, ClewError> {
    let find = |alias: &str| {
        let mut matches = prepared
            .authority
            .members
            .iter()
            .filter(|member| member.member_alias == alias);
        let member = matches
            .next()
            .ok_or_else(|| corrupt("callable pair references a missing member"))?;
        if matches.next().is_some() {
            return Err(corrupt("verified callable fact set repeats a member alias"));
        }
        Ok(member)
    };
    Ok(vec![
        (ImpactSide::Provider, find(&pair.provider_member)?),
        (ImpactSide::Consumer, find(&pair.consumer_member)?),
    ])
}

fn descriptor_scope_complete(member: &CallableMemberBinding) -> bool {
    !member.compilations.is_empty()
        && member.compilations.iter().all(|compilation| {
            compilation.descriptor_coverage == GraphCoverage::CompleteSupportedSubset
        })
}

fn normalize_request(
    prepared: &PreparedCallableFactSet,
    request: &mut ThreadImpactRequest,
) -> Result<(), ClewError> {
    request.budgets.validate()?;
    if request.fact_set_authority_digest != prepared.authority.authority_digest {
        return Err(invalid(
            "Kotlin impact request is bound to a different callable fact set",
        ));
    }
    validate_text(&request.pair_id, "impact pair id", 4_096)?;
    let pair = selected_pair(prepared, &request.pair_id)?;
    match &mut request.subject {
        KotlinImpactSubject::FullSymbol {
            symbol_identity,
            member_alias,
        } => {
            validate_text(symbol_identity, "impact full symbol", 4_096)?;
            crate::semantic_validation::validate_kotlin_full_symbol_identity(symbol_identity)
                .map_err(|_| {
                    invalid("impact FULL_SYMBOL subject is not a compiler full symbol identity")
                })?;
            let alias = member_alias.as_ref().ok_or_else(|| {
                invalid("impact FULL_SYMBOL subject requires an explicit pair member")
            })?;
            validate_text(alias, "impact member alias", 4_096)?;
            if alias != &pair.provider_member && alias != &pair.consumer_member {
                return Err(invalid(
                    "impact FULL_SYMBOL member is not part of the selected pair",
                ));
            }
        }
        KotlinImpactSubject::CallableFamily { callable_id } => {
            validate_text(callable_id, "impact CallableId family", 4_096)?;
            if !raw_callable_id(callable_id) {
                return Err(invalid(
                    "impact CallableId family does not match the closed raw grammar",
                ));
            }
        }
        KotlinImpactSubject::Token { term } => {
            validate_text(term, "impact token", 256)?;
            if !term
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
            {
                return Err(invalid("impact token must be one identifier token"));
            }
            *term = term.chars().flat_map(char::to_lowercase).collect();
            if term.len() < 2 || !crate::text_authority::is_nfc(term) {
                return Err(invalid("impact token must contain at least two bytes"));
            }
        }
    }
    Ok(())
}

fn validate_text(value: &str, label: &str, max_bytes: usize) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || !crate::text_authority::is_nfc(value)
    {
        return Err(invalid(format!("{label} is empty or invalid")));
    }
    Ok(())
}

fn raw_callable_id(value: &str) -> bool {
    if !value.is_ascii()
        || value.starts_with(['/', '.'])
        || value.ends_with(['/', '.'])
        || value.starts_with('-')
        || value.contains("://")
        || value.contains(['\\', ':', '?', '#', '%', '@', '='])
        || ["//", "..", "/.", "./"]
            .iter()
            .any(|needle| value.contains(needle))
        || !value.contains(['/', '.'])
    {
        return false;
    }
    value.split(['/', '.']).all(|segment| {
        segment == "<init>"
            || (!segment.is_empty()
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
                }))
    })
}

fn fact_kind(fact: &CallableFact) -> CallableFactKind {
    match fact {
        CallableFact::Declaration(_) => CallableFactKind::Declaration,
        CallableFact::Use(_) => CallableFactKind::Use,
        CallableFact::Boundary(_) => CallableFactKind::Boundary,
    }
}

fn fact_provenance(fact: &CallableFact) -> &thread_callables::CallableFactProvenance {
    match fact {
        CallableFact::Declaration(row) => &row.provenance,
        CallableFact::Use(row) => &row.provenance,
        CallableFact::Boundary(row) => &row.provenance,
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
    use crate::thread_callables::{
        CallableBuildInput, CallableCompilationAuthority, CallableFactSetRequest,
        CallableMemberAuthority, CallableSelectedCompilation, CallableTaskBinding,
        QualifiedCallablePayload,
    };
    use serde_json::{Value, json};

    fn digest(label: &str) -> String {
        canonical::hash(&label).unwrap()
    }

    fn object(schema: &str, label: &str) -> CasObject {
        CasObject::for_bytes(schema, label.as_bytes()).unwrap()
    }

    fn member(alias: &str) -> CallableMemberAuthority {
        CallableMemberAuthority {
            member_alias: alias.into(),
            service_alias: format!("{alias}-service"),
            session_id: format!("session:{alias}"),
            session_authority_digest: digest(&format!("session-{alias}")),
            repository_key: format!("repository-{alias}"),
            base_revision: digest(&format!("revision-{alias}")),
            snapshot_ref: object(
                "codeclew-repository-input-snapshot/1.0",
                &format!("snapshot-{alias}"),
            ),
        }
    }

    fn compilation(
        alias: &str,
        descriptor_coverage: GraphCoverage,
        relation_coverage: GraphCoverage,
    ) -> CallableCompilationAuthority {
        CallableCompilationAuthority {
            compilation_id: ":app/main".into(),
            generation_id: digest(&format!("generation-id-{alias}")),
            generation_ref: object(
                "codeclew-generation-manifest/2.0",
                &format!("generation-{alias}"),
            ),
            semantic_authority: "K2_FIR".into(),
            extractor_id: "fir-facts-extractor/0.6".into(),
            adapter_digest: digest("adapter"),
            runtime_digest: digest("runtime"),
            descriptor_coverage,
            relation_coverage,
        }
    }

    fn descriptor(callable: &str, jvm: &str, return_type: &str, file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-descriptor/0.1",
            "file":file,
            "start":start,
            "end":start + 8,
            "symbolIdentity":format!("callable:{callable}#jvm:{jvm}"),
            "declarationKind":"FUNCTION",
            "ownerIdentity":format!("class:{}", callable.rsplit_once('.').unwrap().0),
            "containment":[format!("class:{}", callable.rsplit_once('.').unwrap().0)],
            "visibility":"public",
            "effectiveVisibility":"public",
            "exportBoundary":"PUBLIC_API",
            "modality":"FINAL",
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "module":":app",
            "sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "typeParameters":[],
            "compilerCallableId":callable,
            "isOverride":false,
            "returnType":return_type,
            "returnNullable":false,
            "parameterTypes":[],
        })
    }

    fn boundary(subject: &str, file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-descriptor-boundary/0.1",
            "file":file,
            "start":start,
            "end":start + 1,
            "stage":"DECLARATION",
            "code":"UNRESOLVED_DESCRIPTOR_TYPE",
            "symbolIdentity":subject,
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
            "module":"main",
            "sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
        })
    }

    fn relation_boundary(owner: &str, file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-relation-boundary/0.1",
            "file":file,
            "start":start,
            "end":start + 1,
            "owner":owner,
            "stage":"CALL_RESOLUTION",
            "code":"UNRESOLVED_RELATION_TARGET",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
        })
    }

    fn qualified(
        member: CallableMemberAuthority,
        compilation: CallableCompilationAuthority,
        payload: Value,
    ) -> QualifiedCallablePayload {
        let schema = payload.get("schema").unwrap().as_str().unwrap();
        let bytes = canonical::bytes(&payload).unwrap();
        let fact_hash = canonical::hash_bytes(&bytes);
        let file = payload.get("file").unwrap().as_str().unwrap();
        QualifiedCallablePayload {
            source_ref: Some(object(
                "codeclew-repository-source-content/1.0",
                &format!("{}:{file}", member.member_alias),
            )),
            member,
            compilation,
            fact_key: format!(
                "kotlin:{}:{}",
                if schema == "declaration-descriptor-boundary/0.1" {
                    "descriptor-boundary"
                } else if schema == "declaration-relation-boundary/0.1" {
                    "relation-boundary"
                } else {
                    "descriptor"
                },
                fact_hash.strip_prefix("sha256:").unwrap()
            ),
            payload_ref: CasObject::for_bytes(
                thread_callables::KOTLIN_SEMANTIC_FACT_SCHEMA,
                &bytes,
            )
            .unwrap(),
            payload,
        }
    }

    fn fact_set(left_payloads: Vec<Value>, right_payloads: Vec<Value>) -> PreparedCallableFactSet {
        let left = member("left");
        let right = member("right");
        let coverages = |payloads: &[Value]| {
            (
                if payloads.iter().any(|payload| {
                    payload.get("schema").and_then(Value::as_str)
                        == Some("declaration-descriptor-boundary/0.1")
                }) {
                    GraphCoverage::Partial
                } else {
                    GraphCoverage::CompleteSupportedSubset
                },
                if payloads.iter().any(|payload| {
                    payload.get("schema").and_then(Value::as_str)
                        == Some("declaration-relation-boundary/0.1")
                }) {
                    GraphCoverage::Partial
                } else {
                    GraphCoverage::CompleteSupportedSubset
                },
            )
        };
        let (left_descriptor, left_relation) = coverages(&left_payloads);
        let (right_descriptor, right_relation) = coverages(&right_payloads);
        let left_compilation = compilation("left", left_descriptor, left_relation);
        let right_compilation = compilation("right", right_descriptor, right_relation);
        let mut payloads = left_payloads
            .into_iter()
            .map(|payload| qualified(left.clone(), left_compilation.clone(), payload))
            .chain(
                right_payloads
                    .into_iter()
                    .map(|payload| qualified(right.clone(), right_compilation.clone(), payload)),
            )
            .collect::<Vec<_>>();
        let visited_payload_bytes = payloads
            .iter()
            .map(|payload| canonical::bytes(&payload.payload).unwrap().len())
            .sum();
        let input = CallableBuildInput {
            visited_fact_count: payloads.len(),
            visited_payload_bytes,
            selected_compilations: vec![
                CallableSelectedCompilation {
                    member: left,
                    compilation: left_compilation,
                },
                CallableSelectedCompilation {
                    member: right,
                    compilation: right_compilation,
                },
            ],
            payloads: std::mem::take(&mut payloads),
        };
        thread_callables::build(
            CallableFactSetRequest {
                thread_id: "thread:test".into(),
                thread_authority_digest: digest("thread"),
                thread_context_id: "thread-context:test".into(),
                thread_context_authority_digest: digest("context"),
                profile_digest: digest("profile"),
                tasks: vec![CallableTaskBinding {
                    task_id: "task-one".into(),
                    pair_id: "pair-one".into(),
                    terms: vec!["FindOrder".into()],
                }],
                pairs: vec![CallablePairBinding {
                    pair_id: "pair-one".into(),
                    provider_member: "left".into(),
                    consumer_member: "right".into(),
                    relationship_authority: RelationshipAuthority::DeclaredTopology,
                    dependency_evidence_ref: None,
                }],
                budgets: thread_callables::CallableBudgets::frozen(),
            },
            input,
        )
        .unwrap()
    }

    fn request(
        prepared: &PreparedCallableFactSet,
        subject: KotlinImpactSubject,
    ) -> ThreadImpactRequest {
        ThreadImpactRequest {
            fact_set_authority_digest: prepared.authority.authority_digest.clone(),
            pair_id: "pair-one".into(),
            subject,
            budgets: ImpactBudgets::frozen(),
        }
    }

    fn family(callable: &str) -> KotlinImpactSubject {
        KotlinImpactSubject::CallableFamily {
            callable_id: callable.into(),
        }
    }

    fn repeated_candidates(prepared: &PreparedCallableFactSet, count: usize) -> Vec<Candidate> {
        let base = fact_entries(prepared).unwrap().remove(0);
        (0..count)
            .map(|index| {
                let mut fact = base.fact.clone();
                let CallableFact::Declaration(row) = &mut fact else {
                    panic!("fixture declaration expected")
                };
                row.fact_id = format!("fact-{index:05}");
                Candidate {
                    side: ImpactSide::Provider,
                    member_alias: "left".into(),
                    fact,
                    shard_ref: base.shard_ref.clone(),
                }
            })
            .collect()
    }

    fn findings_with_source_spans(
        prepared: &PreparedCallableFactSet,
        count: usize,
        span_bytes: u64,
    ) -> Vec<ImpactFinding> {
        let selection = select(prepared, request(prepared, family("p/Orders.findOrder"))).unwrap();
        let template = selection.findings[0].clone();
        (0..count)
            .map(|index| {
                let mut finding = template.clone();
                finding.finding_id = format!("finding-{index:05}");
                let source = finding.evidence.provenance.source.as_mut().unwrap();
                source.path = format!("src/Window{index:05}.kt");
                source.start = Some(0);
                source.end = Some(span_bytes);
                source.content_ref = object(
                    "codeclew-repository-source-content/1.0",
                    &format!("source-window-{index:05}"),
                );
                source.content_ref.size = span_bytes.max(1);
                finding
            })
            .collect()
    }

    #[test]
    fn exact_family_shapes_report_equal_and_delta_without_upgrading_topology() {
        let equal_facts = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Right.kt",
                10,
            )],
        );
        let equal = build(
            &equal_facts,
            request(&equal_facts, family("p/Orders.findOrder")),
        )
        .unwrap();
        assert_eq!(
            equal.authority.shape_status,
            ImpactShapeStatus::ExactProjectedShapeEqual
        );
        assert_eq!(equal.authority.certainty, ImpactCertainty::Unsure);
        assert_eq!(
            equal.authority.relationship_authority,
            RelationshipAuthority::DeclaredTopology
        );
        assert_eq!(equal.authority.members.len(), 2);
        assert!(equal.authority.members.iter().all(|member| member.observed));
        assert!(equal.projection_bytes.len() < MAX_IMPACT_STDOUT_BYTES);
        assert!(!equal.projection.findings.is_empty());
        assert!(matches!(
            equal.projection.findings[0].detail,
            ImpactFindingDetail::Declaration { .. }
        ));
        assert_eq!(
            equal.projection.obligations.len(),
            equal.authority.obligation_count
        );
        assert!(!equal.projection.source_windows.is_empty());
        verify_prepared(&equal_facts, &equal).unwrap();
        let mut tampered = equal.clone();
        tampered.projection.findings[0].shape_digest = Some(digest("tampered"));
        assert!(verify_prepared(&equal_facts, &tampered).is_err());

        let delta_facts = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![descriptor(
                "p/Orders.findOrder",
                "()I",
                "kotlin/Int",
                "src/Right.kt",
                10,
            )],
        );
        let delta = select(
            &delta_facts,
            request(&delta_facts, family("p/Orders.findOrder")),
        )
        .unwrap();
        assert_eq!(
            delta.shape_status,
            ImpactShapeStatus::ExactProjectedShapeDelta
        );
    }

    #[test]
    fn overload_family_is_ambiguous_and_never_exact() {
        let prepared = fact_set(
            vec![
                descriptor(
                    "p/Orders.findOrder",
                    "()Ljava/lang/String;",
                    "kotlin/String",
                    "src/Left.kt",
                    0,
                ),
                descriptor(
                    "p/Orders.findOrder",
                    "(I)Ljava/lang/String;",
                    "kotlin/String",
                    "src/Left.kt",
                    20,
                ),
            ],
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Right.kt",
                10,
            )],
        );
        let impact = select(&prepared, request(&prepared, family("p/Orders.findOrder"))).unwrap();
        assert_eq!(impact.shape_status, ImpactShapeStatus::Unsure);
        assert!(impact.obligations.iter().any(|obligation| {
            obligation.code == ImpactObligationCode::DisambiguateOverloadSet
                && obligation.member_alias.as_deref() == Some("left")
        }));
        assert_eq!(
            impact
                .findings
                .iter()
                .filter(|finding| finding.side == ImpactSide::Provider)
                .filter(|finding| {
                    finding.authority == ImpactFindingAuthority::ExactProjectedDeclaration
                })
                .count(),
            2,
            "each compiler-projected overload stays exact even though the family aggregate is UNSURE"
        );
    }

    #[test]
    fn token_is_navigation_only_and_missing_side_is_explicit() {
        let prepared = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![],
        );
        let impact = select(
            &prepared,
            request(
                &prepared,
                KotlinImpactSubject::Token {
                    term: "FIND".into(),
                },
            ),
        )
        .unwrap();
        assert_eq!(
            impact.subject,
            KotlinImpactSubject::Token {
                term: "find".into()
            }
        );
        assert_eq!(impact.shape_status, ImpactShapeStatus::NotComparable);
        assert!(impact.findings.iter().all(|finding| {
            finding.authority != ImpactFindingAuthority::ExactProjectedDeclaration
        }));
        assert!(impact.obligations.iter().any(|obligation| {
            obligation.code == ImpactObligationCode::SubjectNotObservedInMember
                && obligation.member_alias.as_deref() == Some("right")
        }));
    }

    #[test]
    fn full_symbol_is_exact_only_in_the_explicit_selected_namespace() {
        let prepared = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Right.kt",
                10,
            )],
        );
        let impact = select(
            &prepared,
            request(
                &prepared,
                KotlinImpactSubject::FullSymbol {
                    symbol_identity: "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;".into(),
                    member_alias: Some("left".into()),
                },
            ),
        )
        .unwrap();
        assert_eq!(impact.shape_status, ImpactShapeStatus::NotComparable);
        assert_eq!(impact.findings.len(), 1);
        assert_eq!(
            impact.findings[0].authority,
            ImpactFindingAuthority::ExactProjectedDeclaration
        );
        assert_eq!(impact.findings[0].member_alias, "left");
        assert!(impact.members[0].exact_shape_digest.is_some());
        assert!(impact.members[1].exact_shape_digest.is_none());
        assert!(!impact.consulted_query_shard_refs.is_empty());
    }

    #[test]
    fn relevant_boundary_downgrades_but_unrelated_boundary_does_not() {
        let relevant = fact_set(
            vec![
                descriptor(
                    "p/Orders.findOrder",
                    "()Ljava/lang/String;",
                    "kotlin/String",
                    "src/Left.kt",
                    0,
                ),
                boundary("class:p/Orders", "src/Left.kt", 30),
            ],
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Right.kt",
                10,
            )],
        );
        let relevant_impact =
            select(&relevant, request(&relevant, family("p/Orders.findOrder"))).unwrap();
        assert_eq!(relevant_impact.shape_status, ImpactShapeStatus::Unsure);
        assert!(
            relevant_impact
                .findings
                .iter()
                .any(|finding| finding.fact_kind == CallableFactKind::Boundary)
        );
        let boundary_obligation = relevant_impact
            .obligations
            .iter()
            .find(|obligation| obligation.code == ImpactObligationCode::VerifyBoundaryCheck)
            .expect("owner-scoped boundary required check");
        assert_eq!(
            boundary_obligation.required_check.as_deref(),
            Some("VERIFY_UNRESOLVED_DESCRIPTOR_TYPE")
        );
        assert!(relevant_impact.obligation_evidence.iter().any(|evidence| {
            Some(evidence.fact_id.as_str()) == boundary_obligation.fact_id.as_deref()
        }));

        let unrelated = fact_set(
            vec![
                descriptor(
                    "p/Orders.findOrder",
                    "()Ljava/lang/String;",
                    "kotlin/String",
                    "src/Left.kt",
                    0,
                ),
                relation_boundary("callable:p/Other.load#jvm:()V", "src/Left.kt", 30),
            ],
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Right.kt",
                10,
            )],
        );
        let unrelated_impact = select(
            &unrelated,
            request(&unrelated, family("p/Orders.findOrder")),
        )
        .unwrap();
        assert_eq!(
            unrelated_impact.shape_status,
            ImpactShapeStatus::ExactProjectedShapeEqual
        );
        assert!(unrelated_impact.obligations.iter().any(|obligation| {
            obligation.code == ImpactObligationCode::VerifyFactSetBoundaryScope
        }));
    }

    #[test]
    fn payload_permutation_preserves_impact_identity() {
        let mut left = vec![
            descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            ),
            descriptor("p/Other.load", "()V", "kotlin/Unit", "src/Other.kt", 20),
        ];
        let right = vec![descriptor(
            "p/Orders.findOrder",
            "()Ljava/lang/String;",
            "kotlin/String",
            "src/Right.kt",
            10,
        )];
        let first = fact_set(left.clone(), right.clone());
        left.reverse();
        let second = fact_set(left, right);
        let one = build(&first, request(&first, family("p/Orders.findOrder"))).unwrap();
        let two = build(&second, request(&second, family("p/Orders.findOrder"))).unwrap();
        assert_eq!(
            first.authority.authority_digest,
            second.authority.authority_digest
        );
        assert_eq!(one, two);
    }

    #[test]
    fn obligation_limit_accepts_exact_and_rejects_plus_one() {
        let make = |count| {
            (0..count)
                .map(|index| {
                    obligation(
                        ImpactObligationCode::VerifyBoundaryCheck,
                        Some("left"),
                        Some(&format!("fact-{index:05}")),
                        Some("VERIFY"),
                    )
                })
                .collect::<BTreeSet<_>>()
        };
        assert!(checked_obligations(make(MAX_IMPACT_OBLIGATIONS), MAX_IMPACT_OBLIGATIONS).is_ok());
        assert!(
            checked_obligations(make(MAX_IMPACT_OBLIGATIONS + 1), MAX_IMPACT_OBLIGATIONS).is_err()
        );
    }

    #[test]
    fn finding_limit_accepts_exact_and_truncates_limit_plus_one() {
        let prepared = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![],
        );
        let (at_limit, at_limit_truncated) = fair_take_candidates(
            repeated_candidates(&prepared, MAX_IMPACT_FINDINGS),
            MAX_IMPACT_FINDINGS,
        );
        assert_eq!(at_limit.len(), MAX_IMPACT_FINDINGS);
        assert!(!at_limit_truncated);

        let (over_limit, over_limit_truncated) = fair_take_candidates(
            repeated_candidates(&prepared, MAX_IMPACT_FINDINGS + 1),
            MAX_IMPACT_FINDINGS,
        );
        assert_eq!(over_limit.len(), MAX_IMPACT_FINDINGS);
        assert!(over_limit_truncated);
        assert_eq!(
            at_limit
                .iter()
                .map(|candidate| candidate.fact.fact_id())
                .collect::<Vec<_>>(),
            over_limit
                .iter()
                .map(|candidate| candidate.fact.fact_id())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn retained_byte_limit_accepts_exact_and_rejects_limit_plus_one() {
        let sized_reference = |label: &str, size: usize| {
            let mut reference = object("codeclew-test-impact-retained/1.0", label);
            reference.size = u64::try_from(size).unwrap();
            reference
        };
        let (closure, retained_bytes) = canonical_cas_closure(
            vec![sized_reference("exact", MAX_IMPACT_RETAINED_CLOSURE_BYTES)],
            MAX_IMPACT_RETAINED_CLOSURE_BYTES,
        )
        .unwrap();
        assert_eq!(closure.len(), 1);
        assert_eq!(retained_bytes, MAX_IMPACT_RETAINED_CLOSURE_BYTES);

        let error = canonical_cas_closure(
            vec![sized_reference(
                "plus-one",
                MAX_IMPACT_RETAINED_CLOSURE_BYTES + 1,
            )],
            MAX_IMPACT_RETAINED_CLOSURE_BYTES,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }

    #[test]
    fn derived_cas_object_limit_accepts_exact_and_rejects_limit_plus_one() {
        assert_eq!(
            checked_derived_cas_object_count(
                MAX_IMPACT_DERIVED_CAS_OBJECTS,
                MAX_IMPACT_DERIVED_CAS_OBJECTS,
            )
            .unwrap(),
            MAX_IMPACT_DERIVED_CAS_OBJECTS
        );
        let error = checked_derived_cas_object_count(
            MAX_IMPACT_DERIVED_CAS_OBJECTS + 1,
            MAX_IMPACT_DERIVED_CAS_OBJECTS,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }

    #[test]
    fn source_window_count_and_byte_limits_truncate_only_limit_plus_one() {
        let prepared = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![],
        );
        let exact_count = findings_with_source_spans(&prepared, MAX_IMPACT_SOURCE_WINDOWS, 1);
        let (windows, truncated) = select_source_windows(
            "sha256:test",
            &exact_count,
            MAX_IMPACT_SOURCE_WINDOWS,
            MAX_IMPACT_SOURCE_WINDOW_BYTES,
        )
        .unwrap();
        assert_eq!(windows.len(), MAX_IMPACT_SOURCE_WINDOWS);
        assert!(!truncated);

        let plus_one_count =
            findings_with_source_spans(&prepared, MAX_IMPACT_SOURCE_WINDOWS + 1, 1);
        let (windows, truncated) = select_source_windows(
            "sha256:test",
            &plus_one_count,
            MAX_IMPACT_SOURCE_WINDOWS,
            MAX_IMPACT_SOURCE_WINDOW_BYTES,
        )
        .unwrap();
        assert_eq!(windows.len(), MAX_IMPACT_SOURCE_WINDOWS);
        assert!(truncated);

        let exact_bytes =
            findings_with_source_spans(&prepared, 1, MAX_IMPACT_SOURCE_WINDOW_BYTES as u64);
        let (windows, truncated) = select_source_windows(
            "sha256:test",
            &exact_bytes,
            MAX_IMPACT_SOURCE_WINDOWS,
            MAX_IMPACT_SOURCE_WINDOW_BYTES,
        )
        .unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].span_bytes, MAX_IMPACT_SOURCE_WINDOW_BYTES);
        assert!(!truncated);

        let plus_one_bytes =
            findings_with_source_spans(&prepared, 1, (MAX_IMPACT_SOURCE_WINDOW_BYTES + 1) as u64);
        let (windows, truncated) = select_source_windows(
            "sha256:test",
            &plus_one_bytes,
            MAX_IMPACT_SOURCE_WINDOWS,
            MAX_IMPACT_SOURCE_WINDOW_BYTES,
        )
        .unwrap();
        assert!(windows.is_empty());
        assert!(truncated);
    }

    #[test]
    fn stdout_bound_trims_findings_but_fails_closed_for_mandatory_projection() {
        let exact_projection_bytes = MAX_IMPACT_STDOUT_BYTES - IMPACT_STDOUT_ENVELOPE_RESERVE;
        assert!(
            exact_projection_bytes.saturating_add(IMPACT_STDOUT_ENVELOPE_RESERVE)
                <= MAX_IMPACT_STDOUT_BYTES
        );
        assert!(
            (exact_projection_bytes + 1).saturating_add(IMPACT_STDOUT_ENVELOPE_RESERVE)
                > MAX_IMPACT_STDOUT_BYTES
        );

        let many_declarations = (0..64)
            .map(|index| {
                let jvm_descriptor = format!("(Lp/Argument{index};)Ljava/lang/String;");
                descriptor(
                    "p/Orders.findOrder",
                    &jvm_descriptor,
                    "kotlin/String",
                    &format!("src/Declaration{index:05}.kt"),
                    u64::try_from(index * 16).unwrap(),
                )
            })
            .collect();
        let trimmable = fact_set(
            many_declarations,
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Right.kt",
                0,
            )],
        );
        let trimmed = build(
            &trimmable,
            request(&trimmable, family("p/Orders.findOrder")),
        )
        .unwrap();
        assert!(trimmed.authority.public_findings_truncated);
        assert!(trimmed.authority.public_findings.len() < trimmed.authority.finding_count);
        assert_eq!(
            trimmed.projection.obligations.len(),
            trimmed.authority.obligation_count
        );
        assert_eq!(
            trimmed.projection.source_windows.len(),
            trimmed.authority.source_window_count
        );
        assert!(
            trimmed
                .projection_bytes
                .len()
                .saturating_add(IMPACT_STDOUT_ENVELOPE_RESERVE)
                <= MAX_IMPACT_STDOUT_BYTES
        );

        let mut mandatory_rows = vec![descriptor(
            "p/Orders.findOrder",
            "()Ljava/lang/String;",
            "kotlin/String",
            "src/Left.kt",
            0,
        )];
        mandatory_rows.extend((0..160).map(|index| {
            boundary(
                "class:p/Orders",
                &format!("src/Boundary{index:05}.kt"),
                u64::try_from(index * 2 + 16).unwrap(),
            )
        }));
        let fail_closed = fact_set(
            mandatory_rows,
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Right.kt",
                0,
            )],
        );
        let error = build(
            &fail_closed,
            request(&fail_closed, family("p/Orders.findOrder")),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
        assert!(error.message.contains("obligations/source anchors"));
    }

    #[test]
    fn omitted_obligation_fact_keeps_a_deduplicated_evidence_pointer() {
        let prepared = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![],
        );
        let base = fact_entries(&prepared).unwrap().remove(0);
        let mut candidates = (0..=MAX_IMPACT_FINDINGS)
            .map(|index| {
                let mut fact = base.fact.clone();
                let CallableFact::Declaration(row) = &mut fact else {
                    panic!("fixture declaration expected")
                };
                row.fact_id = format!("fact-{index:05}");
                row.exact_eligible = index != MAX_IMPACT_FINDINGS;
                Candidate {
                    side: ImpactSide::Provider,
                    member_alias: "left".into(),
                    fact,
                    shard_ref: base.shard_ref.clone(),
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.fact.fact_id().cmp(right.fact.fact_id()));
        let by_id = candidates
            .iter()
            .map(|candidate| (candidate.fact.fact_id().to_owned(), candidate.clone()))
            .collect::<BTreeMap<_, _>>();
        let omitted_id = format!("fact-{MAX_IMPACT_FINDINGS:05}");
        let (selected, truncated) = fair_take_candidates(candidates, MAX_IMPACT_FINDINGS);
        assert!(truncated);
        assert!(
            selected
                .iter()
                .all(|candidate| candidate.fact.fact_id() != omitted_id)
        );
        let obligations = vec![obligation(
            ImpactObligationCode::VerifyDeclarationEvidence,
            Some("left"),
            Some(&omitted_id),
            None,
        )];
        let evidence = obligation_evidence(&obligations, &by_id).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].fact_id, omitted_id);
        assert_eq!(evidence[0].fact_shard_ref, base.shard_ref);
    }

    #[test]
    fn source_windows_are_a_strict_prefix_when_the_first_span_is_oversized() {
        let prepared = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![],
        );
        let selection =
            select(&prepared, request(&prepared, family("p/Orders.findOrder"))).unwrap();
        let mut first = selection.findings[0].clone();
        first.finding_id = "finding-a".into();
        let first_source = first.evidence.provenance.source.as_mut().unwrap();
        first_source.path = "a.kt".into();
        first_source.start = Some(0);
        first_source.end = Some(11);
        let mut second = first.clone();
        second.finding_id = "finding-b".into();
        let second_source = second.evidence.provenance.source.as_mut().unwrap();
        second_source.path = "b.kt".into();
        second_source.end = Some(1);
        let (windows, truncated) =
            select_source_windows("sha256:test", &[first, second], 2, 10).unwrap();
        assert!(truncated);
        assert!(windows.is_empty());
    }

    #[test]
    fn closed_subject_validation_rejects_implicit_full_symbol_and_path_like_family() {
        let prepared = fact_set(
            vec![descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "kotlin/String",
                "src/Left.kt",
                0,
            )],
            vec![],
        );
        let full = KotlinImpactSubject::FullSymbol {
            symbol_identity: "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;".into(),
            member_alias: None,
        };
        assert!(select(&prepared, request(&prepared, full)).is_err());
        assert!(select(&prepared, request(&prepared, family("../../private/path"))).is_err());
    }
}
