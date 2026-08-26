//! Deterministic freshness comparison for one immutable explanation bundle.
//!
//! The comparison consumes only already verified retained inputs. Source
//! locations and CAS identities are provenance, not semantic change signals.

use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use crate::explanation::{ClaimPredicate, PreparedExplanation};
use crate::thread_callables::{
    CallableFact, CallableFactShard, DeclarationFact, PreparedCallableFactSet, UseFact,
};
use crate::thread_change_set::MemberCorrespondence;
use crate::thread_flow::{FlowBoundary, FlowNodeKind, FlowSlice, FlowStatus, PreparedFlowSlice};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const EXPLANATION_FRESHNESS_REPORT_SCHEMA: &str = "codeclew-explanation-freshness-report/0.1";
pub const EXPLANATION_FRESHNESS_PROJECTION_SCHEMA: &str =
    "codeclew-explanation-freshness-projection/0.1";
pub const MAX_FRESHNESS_AFFECTED_CLAIMS: usize = 1_024;
pub const MAX_FRESHNESS_CANDIDATES: usize = 4_096;
pub const MAX_FRESHNESS_REPORT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_FRESHNESS_STDOUT_BYTES: usize = 64 * 1024;
pub const MAX_FRESHNESS_RETAINED_CLOSURE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FreshnessStatus {
    Current,
    PartiallyStale,
    Stale,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FreshnessReason {
    RootMissing,
    RootAmbiguous,
    RootShapeChanged,
    ComparisonTruncated,
    EvidenceMissing,
    EvidenceAmbiguous,
    SemanticDigestChanged,
    NewRelevantBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FreshnessRequest {
    pub old_thread_id: String,
    pub old_thread_authority_digest: String,
    pub old_explanation_id: String,
    pub old_fact_set_id: String,
    pub old_flow_id: String,
    pub against_thread_id: String,
    pub against_thread_authority_digest: String,
    pub against_fact_set_id: String,
    pub against_flow_id: String,
    pub member_correspondence: Vec<MemberCorrespondence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AffectedClaim {
    pub claim_id: String,
    pub old_refs: Vec<String>,
    pub observed_new_candidates: Vec<String>,
    pub reasons: Vec<FreshnessReason>,
    pub regeneration_obligation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationFreshnessReport {
    pub schema: String,
    pub freshness_id: String,
    pub request: FreshnessRequest,
    pub status: FreshnessStatus,
    pub root_semantic_digest_before: Option<String>,
    pub root_semantic_digest_after: Option<String>,
    pub total_claim_count: usize,
    pub affected_claims: Vec<AffectedClaim>,
    pub unaffected_claim_ids: Vec<String>,
    pub verification_obligations: Vec<String>,
    pub retained_closure: Vec<CasObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationFreshnessProjection {
    pub schema: String,
    pub freshness_id: String,
    pub old_explanation_id: String,
    pub against_flow_id: String,
    pub status: FreshnessStatus,
    pub total_claim_count: usize,
    pub affected_claims: Vec<AffectedClaim>,
    pub unaffected_claim_ids: Vec<String>,
    pub verification_obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExplanationFreshness {
    pub report: ExplanationFreshnessReport,
    pub report_bytes: Vec<u8>,
    pub report_ref: CasObject,
    pub projection: ExplanationFreshnessProjection,
}

#[derive(Clone, Copy)]
pub struct FreshnessSide<'a> {
    pub thread_id: &'a str,
    pub thread_authority_digest: &'a str,
    pub fact_set: &'a PreparedCallableFactSet,
    pub flow: &'a PreparedFlowSlice,
}

pub fn build(
    old: FreshnessSide<'_>,
    explanation: &PreparedExplanation,
    against: FreshnessSide<'_>,
    mut member_correspondence: Vec<MemberCorrespondence>,
) -> Result<PreparedExplanationFreshness, ClewError> {
    validate_bindings(&old, explanation, &against)?;
    member_correspondence.sort();
    validate_correspondence(&old, &against, &member_correspondence)?;
    let request = FreshnessRequest {
        old_thread_id: old.thread_id.into(),
        old_thread_authority_digest: old.thread_authority_digest.into(),
        old_explanation_id: explanation.bundle.explanation_id.clone(),
        old_fact_set_id: old.fact_set.projection.fact_set_id.clone(),
        old_flow_id: old.flow.slice.flow_id.clone(),
        against_thread_id: against.thread_id.into(),
        against_thread_authority_digest: against.thread_authority_digest.into(),
        against_fact_set_id: against.fact_set.projection.fact_set_id.clone(),
        against_flow_id: against.flow.slice.flow_id.clone(),
        member_correspondence,
    };
    let correspondence = request
        .member_correspondence
        .iter()
        .map(|entry| {
            (
                entry.before_member_alias.as_str(),
                entry.after_member_alias.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let old_facts = FactCatalog::read(old.fact_set)?;
    let new_facts = FactCatalog::read(against.fact_set)?;
    let old_graph = SemanticGraph::new(&old.flow.slice, &old_facts, Some(&correspondence))?;
    let new_graph = SemanticGraph::new(&against.flow.slice, &new_facts, None)?;

    let mapped_root_member = correspondence
        .get(old.flow.slice.request.member_alias.as_str())
        .ok_or_else(|| invalid("root member is absent from exact correspondence"))?;
    let root_key = EvidenceKey::Node {
        member_alias: (*mapped_root_member).into(),
        symbol_identity: old.flow.slice.request.root.clone(),
        node_kind: FlowNodeKind::Callable,
    };
    let old_root = old_graph
        .by_key
        .get(&root_key)
        .and_then(|values| values.first());
    let new_roots = new_graph.by_key.get(&root_key).cloned().unwrap_or_default();
    let mut unresolved_reason = None;
    if old.flow.slice.status == FlowStatus::Truncated
        || against.flow.slice.status == FlowStatus::Truncated
    {
        unresolved_reason = Some(FreshnessReason::ComparisonTruncated);
    } else if new_roots.is_empty() {
        unresolved_reason = Some(FreshnessReason::RootMissing);
    } else if new_roots.len() != 1 {
        unresolved_reason = Some(FreshnessReason::RootAmbiguous);
    }
    let old_root_digest = old_root.map(|evidence| evidence.digest.clone());
    let new_root_digest = (new_roots.len() == 1).then(|| new_roots[0].digest.clone());
    let root_shape_changed = unresolved_reason.is_none()
        && old_root_digest.is_some()
        && new_root_digest.is_some()
        && old_root_digest != new_root_digest;

    let mut affected = Vec::new();
    let mut unaffected = Vec::new();
    for claim in &explanation.bundle.claims {
        let mut reasons = BTreeSet::new();
        let mut candidates = BTreeSet::new();
        if let Some(reason) = unresolved_reason {
            reasons.insert(reason);
        } else if root_shape_changed {
            reasons.insert(FreshnessReason::RootShapeChanged);
        } else {
            compare_claim(
                claim,
                &old_graph,
                &new_graph,
                &old.flow.slice,
                &against.flow.slice,
                &mut reasons,
                &mut candidates,
            );
        }
        if reasons.is_empty() {
            unaffected.push(claim.claim_id.clone());
        } else {
            let mut old_refs = claim.support_refs.clone();
            old_refs.extend(claim.boundary_refs.iter().cloned());
            old_refs.sort();
            old_refs.dedup();
            affected.push(AffectedClaim {
                claim_id: claim.claim_id.clone(),
                old_refs,
                observed_new_candidates: candidates.into_iter().collect(),
                reasons: reasons.into_iter().collect(),
                regeneration_obligation: "REGENERATE_CLAIM_FROM_AGAINST_FLOW".into(),
            });
        }
    }
    if affected.len() > MAX_FRESHNESS_AFFECTED_CLAIMS
        || affected
            .iter()
            .map(|claim| claim.observed_new_candidates.len())
            .sum::<usize>()
            > MAX_FRESHNESS_CANDIDATES
    {
        return Err(budget(
            "freshness affected-claim projection exceeds its bounds",
        ));
    }
    affected.sort_by(|left, right| left.claim_id.cmp(&right.claim_id));
    unaffected.sort();
    let status = if unresolved_reason.is_some() {
        FreshnessStatus::Unresolved
    } else if root_shape_changed || affected.len() == explanation.bundle.claims.len() {
        FreshnessStatus::Stale
    } else if affected.is_empty() {
        FreshnessStatus::Current
    } else {
        FreshnessStatus::PartiallyStale
    };
    let verification_obligations = match status {
        FreshnessStatus::Current => Vec::new(),
        FreshnessStatus::Unresolved => vec!["RESOLVE_FRESHNESS_COMPARISON".into()],
        FreshnessStatus::PartiallyStale | FreshnessStatus::Stale => {
            vec!["REGENERATE_AFFECTED_CLAIMS".into()]
        }
    };
    let retained_closure = retained_closure(&old, explanation, &against)?;
    let identity = json!({
        "schema": EXPLANATION_FRESHNESS_REPORT_SCHEMA,
        "request": request,
        "status": status,
        "rootSemanticDigestBefore": old_root_digest,
        "rootSemanticDigestAfter": new_root_digest,
        "totalClaimCount": explanation.bundle.claims.len(),
        "affectedClaims": affected,
        "unaffectedClaimIds": unaffected,
        "verificationObligations": verification_obligations,
        "retainedClosure": retained_closure,
    });
    let freshness_id = format!(
        "explanation-freshness:{}",
        canonical::hash(&identity).map_err(internal)?
    );
    let report = ExplanationFreshnessReport {
        schema: EXPLANATION_FRESHNESS_REPORT_SCHEMA.into(),
        freshness_id: freshness_id.clone(),
        request,
        status,
        root_semantic_digest_before: old_root_digest,
        root_semantic_digest_after: new_root_digest,
        total_claim_count: explanation.bundle.claims.len(),
        affected_claims: affected.clone(),
        unaffected_claim_ids: unaffected.clone(),
        verification_obligations: verification_obligations.clone(),
        retained_closure,
    };
    let report_bytes = canonical::bytes(&report).map_err(internal)?;
    if report_bytes.len() > MAX_FRESHNESS_REPORT_BYTES {
        return Err(budget("freshness report exceeds 16 MiB"));
    }
    let report_ref = CasObject::for_bytes(EXPLANATION_FRESHNESS_REPORT_SCHEMA, &report_bytes)?;
    let projection = ExplanationFreshnessProjection {
        schema: EXPLANATION_FRESHNESS_PROJECTION_SCHEMA.into(),
        freshness_id,
        old_explanation_id: explanation.bundle.explanation_id.clone(),
        against_flow_id: against.flow.slice.flow_id.clone(),
        status,
        total_claim_count: explanation.bundle.claims.len(),
        affected_claims: affected,
        unaffected_claim_ids: unaffected,
        verification_obligations,
    };
    if canonical::bytes(&projection)
        .map_err(internal)?
        .len()
        .saturating_add(4 * 1024)
        > MAX_FRESHNESS_STDOUT_BYTES
    {
        return Err(budget(
            "freshness projection plus stdout envelope exceeds 64 KiB",
        ));
    }
    let prepared = PreparedExplanationFreshness {
        report,
        report_bytes,
        report_ref,
        projection,
    };
    Ok(prepared)
}

pub fn verify_prepared(
    old: FreshnessSide<'_>,
    explanation: &PreparedExplanation,
    against: FreshnessSide<'_>,
    prepared: &PreparedExplanationFreshness,
) -> Result<(), ClewError> {
    if canonical::bytes(&prepared.report).map_err(internal)? != prepared.report_bytes
        || CasObject::for_bytes(EXPLANATION_FRESHNESS_REPORT_SCHEMA, &prepared.report_bytes)?
            != prepared.report_ref
        || prepared.projection.freshness_id != prepared.report.freshness_id
    {
        return Err(corrupt(
            "prepared freshness report is internally inconsistent",
        ));
    }
    let rebuilt = build(
        old,
        explanation,
        against,
        prepared.report.request.member_correspondence.clone(),
    )?;
    if rebuilt != *prepared {
        return Err(corrupt(
            "freshness report differs from deterministic retained inputs",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
enum EvidenceKey {
    Node {
        member_alias: String,
        symbol_identity: String,
        node_kind: FlowNodeKind,
    },
    Edge {
        source_member_alias: String,
        source_symbol_identity: String,
        target_member_alias: String,
        target_symbol_identity: String,
        relation_kind: String,
        relationship_authority: crate::thread_callables::RelationshipAuthority,
    },
    Region {
        member_alias: String,
        owner_symbol_identity: String,
    },
    Boundary {
        code: String,
        subject: String,
        required_checks: Vec<String>,
    },
}

#[derive(Debug, Clone)]
struct SemanticEvidence {
    reference: String,
    key: EvidenceKey,
    digest: String,
}

#[derive(Default)]
struct SemanticGraph {
    by_ref: BTreeMap<String, SemanticEvidence>,
    by_key: BTreeMap<EvidenceKey, Vec<SemanticEvidence>>,
    fact_keys: BTreeMap<String, BTreeSet<EvidenceKey>>,
    boundaries: Vec<BoundarySemantic>,
}

#[derive(Clone)]
struct BoundarySemantic {
    key: EvidenceKey,
    subject: String,
    support_keys: BTreeSet<EvidenceKey>,
}

impl SemanticGraph {
    fn new(
        flow: &FlowSlice,
        facts: &FactCatalog,
        correspondence: Option<&BTreeMap<&str, &str>>,
    ) -> Result<Self, ClewError> {
        let mut graph = Self::default();
        let nodes = flow
            .nodes
            .iter()
            .map(|node| (node.node_id.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        for node in &flow.nodes {
            let key = EvidenceKey::Node {
                member_alias: mapped_member(&node.member_alias, correspondence),
                symbol_identity: node.symbol_identity.clone(),
                node_kind: node.node_kind,
            };
            let digest = match node
                .support_refs
                .iter()
                .filter_map(|support| facts.by_id.get(&support.fact_id))
                .find_map(|fact| match fact {
                    CallableFact::Declaration(row) => Some(declaration_digest(row)),
                    _ => None,
                }) {
                Some(digest) => digest?,
                None => canonical::hash(&key).map_err(internal)?,
            };
            graph.insert(SemanticEvidence {
                reference: node.node_id.clone(),
                key: key.clone(),
                digest,
            });
            for support in &node.support_refs {
                graph
                    .fact_keys
                    .entry(support.fact_id.clone())
                    .or_default()
                    .insert(key.clone());
            }
        }
        for edge in &flow.edges {
            let source = nodes
                .get(edge.source_node_id.as_str())
                .ok_or_else(|| corrupt("freshness flow edge has no source node"))?;
            let target = nodes
                .get(edge.target_node_id.as_str())
                .ok_or_else(|| corrupt("freshness flow edge has no target node"))?;
            let key = EvidenceKey::Edge {
                source_member_alias: mapped_member(&source.member_alias, correspondence),
                source_symbol_identity: source.symbol_identity.clone(),
                target_member_alias: mapped_member(&target.member_alias, correspondence),
                target_symbol_identity: target.symbol_identity.clone(),
                relation_kind: edge.relation_kind.clone(),
                relationship_authority: edge.relationship_authority,
            };
            let digest = match edge
                .support_refs
                .iter()
                .filter_map(|support| facts.by_id.get(&support.fact_id))
                .find_map(|fact| match fact {
                    CallableFact::Use(row) => Some(use_digest(row, correspondence)),
                    _ => None,
                }) {
                Some(digest) => digest?,
                None => canonical::hash(&key).map_err(internal)?,
            };
            graph.insert(SemanticEvidence {
                reference: edge.edge_id.clone(),
                key: key.clone(),
                digest,
            });
            for support in &edge.support_refs {
                graph
                    .fact_keys
                    .entry(support.fact_id.clone())
                    .or_default()
                    .insert(key.clone());
            }
        }
        for region in &flow.control_flow_regions {
            let owner = nodes
                .get(region.owner_node_id.as_str())
                .ok_or_else(|| corrupt("freshness CFG region has no owner node"))?;
            let key = EvidenceKey::Region {
                member_alias: mapped_member(&owner.member_alias, correspondence),
                owner_symbol_identity: owner.symbol_identity.clone(),
            };
            let digest = cfg_digest(&region.graph)?;
            graph.insert(SemanticEvidence {
                reference: region.region_id.clone(),
                key,
                digest,
            });
        }
        for boundary in &flow.boundaries {
            let key = boundary_key(boundary);
            let support_keys = boundary
                .support_refs
                .iter()
                .flat_map(|support| {
                    graph
                        .fact_keys
                        .get(&support.fact_id)
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect::<BTreeSet<_>>();
            let digest = canonical::hash(&json!({
                "key": key,
                "supportKeys": support_keys,
            }))
            .map_err(internal)?;
            graph.insert(SemanticEvidence {
                reference: boundary.boundary_id.clone(),
                key: key.clone(),
                digest,
            });
            graph.boundaries.push(BoundarySemantic {
                key,
                subject: boundary.subject.clone(),
                support_keys,
            });
        }
        for values in graph.by_key.values_mut() {
            values.sort_by(|left, right| left.reference.cmp(&right.reference));
        }
        Ok(graph)
    }

    fn insert(&mut self, evidence: SemanticEvidence) {
        self.by_key
            .entry(evidence.key.clone())
            .or_default()
            .push(evidence.clone());
        self.by_ref.insert(evidence.reference.clone(), evidence);
    }
}

#[derive(Default)]
struct FactCatalog {
    by_id: BTreeMap<String, CallableFact>,
}

impl FactCatalog {
    fn read(fact_set: &PreparedCallableFactSet) -> Result<Self, ClewError> {
        let mut catalog = Self::default();
        for object in &fact_set.fact_shards {
            if CasObject::for_bytes(&object.reference.object_schema, &object.bytes)?
                != object.reference
            {
                return Err(corrupt("freshness fact shard differs from CAS authority"));
            }
            let shard: CallableFactShard = serde_json::from_slice(&object.bytes)
                .map_err(|_| corrupt("freshness fact shard is invalid"))?;
            if canonical::bytes(&shard).map_err(internal)? != object.bytes {
                return Err(corrupt("freshness fact shard is not canonical"));
            }
            for fact in shard.facts {
                if catalog.by_id.insert(fact.fact_id().into(), fact).is_some() {
                    return Err(corrupt("freshness fact ID is duplicated"));
                }
            }
        }
        Ok(catalog)
    }
}

fn compare_claim(
    claim: &crate::explanation::ExplanationClaim,
    old_graph: &SemanticGraph,
    new_graph: &SemanticGraph,
    old_flow: &FlowSlice,
    _new_flow: &FlowSlice,
    reasons: &mut BTreeSet<FreshnessReason>,
    candidates: &mut BTreeSet<String>,
) {
    let mut support_keys = BTreeSet::new();
    let mut old_boundary_keys = BTreeSet::new();
    for reference in claim.support_refs.iter().chain(claim.boundary_refs.iter()) {
        let Some(old) = old_graph.by_ref.get(reference) else {
            reasons.insert(FreshnessReason::EvidenceMissing);
            continue;
        };
        if claim.support_refs.contains(reference) {
            support_keys.insert(old.key.clone());
        } else {
            old_boundary_keys.insert(old.key.clone());
        }
        let matches = new_graph.by_key.get(&old.key).cloned().unwrap_or_default();
        candidates.extend(matches.iter().map(|candidate| candidate.reference.clone()));
        match matches.as_slice() {
            [] => {
                reasons.insert(FreshnessReason::EvidenceMissing);
            }
            [candidate] if candidate.digest != old.digest => {
                reasons.insert(FreshnessReason::SemanticDigestChanged);
            }
            [_] => {}
            _ => {
                reasons.insert(FreshnessReason::EvidenceAmbiguous);
            }
        }
    }
    let subjects = claim_subjects(&claim.predicate);
    let old_all_boundary_keys = old_flow
        .boundaries
        .iter()
        .map(boundary_key)
        .collect::<BTreeSet<_>>();
    if new_graph.boundaries.iter().any(|boundary| {
        !old_all_boundary_keys.contains(&boundary.key)
            && (subjects.contains(boundary.subject.as_str())
                || !boundary.support_keys.is_disjoint(&support_keys))
            && !old_boundary_keys.contains(&boundary.key)
    }) {
        reasons.insert(FreshnessReason::NewRelevantBoundary);
    }
}

fn validate_bindings(
    old: &FreshnessSide<'_>,
    explanation: &PreparedExplanation,
    against: &FreshnessSide<'_>,
) -> Result<(), ClewError> {
    crate::thread_callables::verify_prepared(old.fact_set)?;
    crate::thread_callables::verify_prepared(against.fact_set)?;
    crate::explanation::verify_prepared(&old.flow.slice, explanation)?;
    for flow in [old.flow, against.flow] {
        if canonical::bytes(&flow.slice).map_err(internal)? != flow.slice_bytes
            || CasObject::for_bytes(crate::thread_flow::FLOW_SLICE_SCHEMA, &flow.slice_bytes)?
                != flow.slice_ref
        {
            return Err(corrupt("freshness flow differs from its CAS authority"));
        }
    }
    if explanation.bundle.thread_id != old.thread_id
        || explanation.bundle.thread_authority_digest != old.thread_authority_digest
        || explanation.bundle.fact_set_id != old.fact_set.projection.fact_set_id
        || explanation.bundle.flow_id != old.flow.slice.flow_id
        || old.flow.slice.request.fact_set_id != old.fact_set.projection.fact_set_id
        || against.flow.slice.request.fact_set_id != against.fact_set.projection.fact_set_id
        || old.flow.slice.request.thread_id != old.thread_id
        || against.flow.slice.request.thread_id != against.thread_id
    {
        return Err(invalid("freshness inputs do not form exact binding chains"));
    }
    if old.fact_set.authority.profile_digest != against.fact_set.authority.profile_digest {
        return Err(invalid("freshness Kotlin profile digest changed"));
    }
    Ok(())
}

fn validate_correspondence(
    old: &FreshnessSide<'_>,
    against: &FreshnessSide<'_>,
    correspondence: &[MemberCorrespondence],
) -> Result<(), ClewError> {
    let old_pair = old
        .fact_set
        .authority
        .pairs
        .iter()
        .find(|pair| pair.pair_id == old.flow.slice.request.pair_id)
        .ok_or_else(|| corrupt("old freshness pair is missing"))?;
    let new_pair = against
        .fact_set
        .authority
        .pairs
        .iter()
        .find(|pair| pair.pair_id == against.flow.slice.request.pair_id)
        .ok_or_else(|| corrupt("against freshness pair is missing"))?;
    let expected_old = [&old_pair.provider_member, &old_pair.consumer_member]
        .into_iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_new = [&new_pair.provider_member, &new_pair.consumer_member]
        .into_iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual_old = correspondence
        .iter()
        .map(|entry| entry.before_member_alias.as_str())
        .collect::<BTreeSet<_>>();
    let actual_new = correspondence
        .iter()
        .map(|entry| entry.after_member_alias.as_str())
        .collect::<BTreeSet<_>>();
    if correspondence.len() != 2 || actual_old != expected_old || actual_new != expected_new {
        return Err(invalid(
            "freshness correspondence is not total and bijective for both selected pairs",
        ));
    }
    for entry in correspondence {
        let old_member = old
            .fact_set
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == entry.before_member_alias)
            .ok_or_else(|| corrupt("old freshness member is missing"))?;
        let new_member = against
            .fact_set
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == entry.after_member_alias)
            .ok_or_else(|| corrupt("against freshness member is missing"))?;
        if old_member.repository_namespace != new_member.repository_namespace
            || old_member.repository_key != new_member.repository_key
        {
            return Err(invalid(
                "freshness member correspondence changes repository identity",
            ));
        }
    }
    Ok(())
}

fn retained_closure(
    old: &FreshnessSide<'_>,
    explanation: &PreparedExplanation,
    against: &FreshnessSide<'_>,
) -> Result<Vec<CasObject>, ClewError> {
    let mut objects = BTreeMap::<(String, String), CasObject>::new();
    for object in [
        explanation.bundle_ref.clone(),
        old.flow.slice_ref.clone(),
        against.flow.slice_ref.clone(),
    ]
    .into_iter()
    .chain(old.fact_set.authority.direct_cas_closure.iter().cloned())
    .chain(
        against
            .fact_set
            .authority
            .direct_cas_closure
            .iter()
            .cloned(),
    ) {
        objects.insert(
            (object.object_schema.clone(), object.digest.clone()),
            object,
        );
    }
    let closure = objects.into_values().collect::<Vec<_>>();
    let total = closure.iter().try_fold(0usize, |total, object| {
        let size = usize::try_from(object.size)
            .map_err(|_| budget("freshness closure exceeds host size"))?;
        total
            .checked_add(size)
            .ok_or_else(|| budget("freshness closure size overflow"))
    })?;
    if total > MAX_FRESHNESS_RETAINED_CLOSURE_BYTES {
        return Err(budget("freshness retained closure exceeds 128 MiB"));
    }
    Ok(closure)
}

fn mapped_member(value: &str, correspondence: Option<&BTreeMap<&str, &str>>) -> String {
    correspondence
        .and_then(|mapping| mapping.get(value).copied())
        .unwrap_or(value)
        .into()
}

fn declaration_digest(row: &DeclarationFact) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "symbolIdentity": row.symbol_identity,
        "declarationKind": row.declaration_kind,
        "compilerCallableId": row.compiler_callable_id,
        "compilerClassId": row.compiler_class_id,
        "jvmDescriptor": row.jvm_descriptor,
        "ownerIdentity": row.owner_identity,
        "containment": row.containment,
        "projectedShape": strip_source_fields(&row.projected_shape),
        "exactEligible": row.exact_eligible,
        "uncertaintyReasons": row.uncertainty_reasons,
    }))
    .map_err(internal)
}

fn use_digest(
    row: &UseFact,
    correspondence: Option<&BTreeMap<&str, &str>>,
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "memberAlias": mapped_member(&row.provenance.member_alias, correspondence),
        "relationKind": row.relation_kind,
        "sourceOwner": row.source_owner,
        "targetCallableId": row.target_callable_id,
        "targetSymbolIdentity": row.target_symbol_identity,
        "targetResolution": row.target_resolution,
        "relationshipAuthority": row.relationship_authority,
        "relationEvidence": strip_source_fields(&row.relation_evidence),
        "exactEligible": row.exact_eligible,
        "uncertaintyReasons": row.uncertainty_reasons,
    }))
    .map_err(internal)
}

fn cfg_digest(graph: &crate::thread_flow_cfg::LocalCfgPayload) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "ownerSymbolIdentity": graph.owner_symbol_identity,
        "provider": graph.provider,
        "nodes": graph.nodes.iter().map(|node| json!({
            "nodeId": node.node_id,
            "role": node.role,
        })).collect::<Vec<_>>(),
        "edges": graph.edges,
    }))
    .map_err(internal)
}

fn boundary_key(boundary: &FlowBoundary) -> EvidenceKey {
    EvidenceKey::Boundary {
        code: boundary.code.clone(),
        subject: boundary.subject.clone(),
        required_checks: boundary.required_checks.clone(),
    }
}

fn claim_subjects(predicate: &ClaimPredicate) -> BTreeSet<&str> {
    match predicate {
        ClaimPredicate::CallExists { subject, object }
        | ClaimPredicate::Constructs { subject, object }
        | ClaimPredicate::ReachableStaticPath { subject, object }
        | ClaimPredicate::ComponentHandoff { subject, object } => {
            [subject.as_str(), object.as_str()].into_iter().collect()
        }
        ClaimPredicate::BranchExists { subject, .. }
        | ClaimPredicate::OrderedBefore { subject, .. }
        | ClaimPredicate::NarrativeSummary { subject } => [subject.as_str()].into_iter().collect(),
    }
}

fn strip_source_fields(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "file" | "start" | "end" | "orderKey" | "sourceProvenance"
                    )
                })
                .map(|(key, value)| (key.clone(), strip_source_fields(value)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(strip_source_fields).collect()),
        _ => value.clone(),
    }
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn budget(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
