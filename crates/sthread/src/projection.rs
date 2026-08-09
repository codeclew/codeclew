//! Repository-neutral, bounded semantic projections.
//!
//! A projection is deliberately a view over already-produced semantic facts;
//! it never discovers files or performs text search.  Every high-level claim
//! is linked to immutable L0 facts, so consumers can move from an L5 summary
//! back to exact source provenance without receiving source text at L1-L5.

use crate::canonical;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

pub const PROJECTION_SCHEMA: &str = "semantic-projection/0.1";

/// Complete replay identity for a semantic projection.  This is intentionally
/// local rather than reusing index-only provenance: compiler and compilation
/// choices can change semantic facts without changing a file digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionProvenance {
    pub base_revision: String,
    pub composite_snapshot_hash: String,
    pub index_snapshot_hash: String,
    pub project_model_hash: String,
    pub classpath_hash: String,
    pub compiler_version: String,
    pub compiler_options_hash: String,
    pub compilation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectionLevel {
    L0,
    L1,
    L2,
    L3,
    L4,
    L5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThreadKind {
    Control,
    Data,
    Journey,
    State,
    Effect,
    Failure,
    Config,
    TestEvidence,
    Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Traversal {
    Forward,
    Backward,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BoundaryState {
    Unknown,
    Partial,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BoundaryPolicy {
    ReturnPartial,
    Refuse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectionStatus {
    Complete,
    PartialBudget,
    PartialBoundary,
    UnknownBoundary,
    RefusedUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionBudget {
    pub max_nodes: usize,
    pub max_bytes: usize,
}

impl Default for ProjectionBudget {
    fn default() -> Self {
        Self {
            max_nodes: 64,
            max_bytes: 32 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct L0Source {
    pub file: String,
    pub content_hash: String,
    pub range_start: u64,
    pub range_end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticFact {
    pub id: String,
    pub level: ProjectionLevel,
    pub kind: String,
    pub summary: String,
    /// The L0 fact ids which prove this claim.  For an L0 fact this is itself.
    pub l0_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<L0Source>,
    pub fingerprint: String,
}

impl SemanticFact {
    pub fn new(
        id: impl Into<String>,
        level: ProjectionLevel,
        kind: impl Into<String>,
        summary: impl Into<String>,
        mut l0_evidence: Vec<String>,
        source: Option<L0Source>,
    ) -> Result<Self, ProjectionError> {
        let id = id.into();
        if level == ProjectionLevel::L0 && l0_evidence.is_empty() {
            l0_evidence.push(id.clone());
        }
        l0_evidence.sort();
        l0_evidence.dedup();
        let mut fact = Self {
            id,
            level,
            kind: kind.into(),
            summary: summary.into(),
            l0_evidence,
            source,
            fingerprint: String::new(),
        };
        fact.fingerprint = fact_fingerprint(&fact)?;
        Ok(fact)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: ThreadKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticBoundary {
    pub id: String,
    pub state: BoundaryState,
    pub affected_fact_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticProjectionInput {
    pub schema: String,
    pub provenance: ProjectionProvenance,
    pub facts: Vec<SemanticFact>,
    pub edges: Vec<SemanticEdge>,
    #[serde(default)]
    pub boundaries: Vec<SemanticBoundary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionQuery {
    pub schema: String,
    pub level: ProjectionLevel,
    pub roots: Vec<String>,
    #[serde(default)]
    pub thread_kinds: Vec<ThreadKind>,
    #[serde(default = "default_traversal")]
    pub traversal: Traversal,
    #[serde(default)]
    pub budget: ProjectionBudget,
    #[serde(default = "default_boundary_policy")]
    pub boundary_policy: BoundaryPolicy,
}

fn default_traversal() -> Traversal {
    Traversal::Both
}
fn default_boundary_policy() -> BoundaryPolicy {
    BoundaryPolicy::ReturnPartial
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceLink {
    pub l0_fact_id: String,
    pub l0_fingerprint: String,
    pub claim_node_id: String,
    pub claim_fingerprint: String,
    /// Directed semantic path from the claim to the L0 fact (inclusive).
    pub path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionNode {
    pub id: String,
    pub level: ProjectionLevel,
    pub kind: String,
    pub summary: String,
    pub fingerprint: String,
    pub evidence: Vec<EvidenceLink>,
    /// Present only for L0.  Source text is never materialized above L0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<L0Source>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpansionHandle {
    pub schema: String,
    pub composite_snapshot_hash: String,
    pub query_fingerprint: String,
    pub after_fact_id: String,
    pub remaining_nodes: usize,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionResult {
    pub schema: String,
    pub provenance: ProjectionProvenance,
    pub query_fingerprint: String,
    pub status: ProjectionStatus,
    pub nodes: Vec<ProjectionNode>,
    pub boundaries: Vec<SemanticBoundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<ExpansionHandle>,
    pub fingerprint: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProjectionError {
    #[error("unsupported projection schema {0}")]
    UnsupportedSchema(String),
    #[error("projection root {0} is not a semantic fact")]
    UnknownRoot(String),
    #[error("semantic fact {0} has an invalid fingerprint")]
    InvalidFactFingerprint(String),
    #[error("semantic fact {0} has invalid L0 evidence: {1}")]
    InvalidEvidence(String, String),
    #[error("semantic fact {0} exposes source above L0")]
    SourceAboveL0(String),
    #[error("duplicate {0} id {1}")]
    DuplicateId(&'static str, String),
    #[error("semantic edge {0} references a missing fact")]
    BrokenEdge(String),
    #[error("projection node {0} has an invalid fingerprint")]
    InvalidNodeFingerprint(String),
    #[error("evidence link for claim {0} is stale or does not reach L0")]
    InvalidTrace(String),
    #[error("expansion handle is stale, forged, or belongs to another query: {0}")]
    InvalidExpansion(String),
    #[error("projection budget cannot emit even one target-level fact")]
    BudgetTooSmall,
    #[error("cannot canonicalize projection: {0}")]
    Canonical(String),
}

/// Evaluates a bounded semantic view.  Only the reachable subgraph from the
/// explicit roots is considered, so unrelated repository padding cannot alter
/// an under-budget result or its fingerprint.
pub fn project(
    input: &SemanticProjectionInput,
    query: &ProjectionQuery,
) -> Result<ProjectionResult, ProjectionError> {
    project_page(input, query, None)
}

/// Continues a previously budget-limited projection. The handle is bound to
/// both the composite snapshot and the canonical, cursor-free query; a naked
/// fact id cannot be replayed against a different graph or request.
pub fn expand(
    input: &SemanticProjectionInput,
    query: &ProjectionQuery,
    handle: &ExpansionHandle,
) -> Result<ProjectionResult, ProjectionError> {
    let query_fingerprint = query_fingerprint(query)?;
    if handle.schema != PROJECTION_SCHEMA
        || handle.composite_snapshot_hash != input.provenance.composite_snapshot_hash
        || handle.query_fingerprint != query_fingerprint
        || handle.fingerprint != expansion_fingerprint(handle)?
    {
        return Err(ProjectionError::InvalidExpansion(
            handle.after_fact_id.clone(),
        ));
    }
    let facts: BTreeMap<_, _> = input
        .facts
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let target_ids: Vec<_> = reachable_fact_ids(input, query, &facts)
        .into_iter()
        .filter(|id| facts[id.as_str()].level == query.level)
        .collect();
    if !handle.after_fact_id.is_empty() && !target_ids.contains(&handle.after_fact_id) {
        return Err(ProjectionError::InvalidExpansion(
            handle.after_fact_id.clone(),
        ));
    }
    let expected_remaining = target_ids
        .iter()
        .filter(|id| id.as_str() > handle.after_fact_id.as_str())
        .count();
    if expected_remaining != handle.remaining_nodes {
        return Err(ProjectionError::InvalidExpansion(
            handle.after_fact_id.clone(),
        ));
    }
    project_page(input, query, Some(&handle.after_fact_id))
}

fn project_page(
    input: &SemanticProjectionInput,
    query: &ProjectionQuery,
    after_fact_id: Option<&str>,
) -> Result<ProjectionResult, ProjectionError> {
    validate_input(input)?;
    if query.schema != PROJECTION_SCHEMA {
        return Err(ProjectionError::UnsupportedSchema(query.schema.clone()));
    }
    let query_fingerprint = query_fingerprint(query)?;
    let facts: BTreeMap<_, _> = input
        .facts
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let allowed_kinds: BTreeSet<_> = query.thread_kinds.iter().copied().collect();
    for root in &query.roots {
        if !facts.contains_key(root.as_str()) {
            return Err(ProjectionError::UnknownRoot(root.clone()));
        }
    }
    let traversed = reachable_fact_ids(input, query, &facts);
    // A level is an abstraction contract, not a display label.  Never turn an
    // L0 fact into an alleged L5 summary merely because a caller asked for L5.
    let reachable: Vec<_> = traversed
        .iter()
        .filter(|id| facts[id.as_str()].level == query.level)
        .cloned()
        .collect();
    let mut relevant_boundaries = relevant_boundaries(input, &traversed);
    if reachable.is_empty() {
        relevant_boundaries.push(SemanticBoundary {
            id: "NO_FACTS_AT_LEVEL".into(),
            state: BoundaryState::Unknown,
            affected_fact_ids: traversed.clone(),
            reason: format!("no reachable semantic facts at {:?}", query.level),
        });
    }
    relevant_boundaries.sort_by(|left, right| left.id.cmp(&right.id));
    let refused = query.boundary_policy == BoundaryPolicy::Refuse
        && relevant_boundaries.iter().any(|boundary| {
            matches!(
                boundary.state,
                BoundaryState::Unknown | BoundaryState::Unsupported
            )
        });
    if refused {
        let result = finish(
            input,
            query_fingerprint,
            ProjectionStatus::RefusedUnsupported,
            Vec::new(),
            relevant_boundaries,
            None,
        )?;
        return ensure_result_fits(result, query.budget.max_bytes);
    }

    let mut selected = Vec::new();
    for id in &reachable {
        if after_fact_id.is_some_and(|after| id.as_str() <= after) {
            continue;
        }
        if selected.len() >= query.budget.max_nodes {
            break;
        }
        selected.push(projection_node(
            facts[id.as_str()],
            &facts,
            &input.edges,
            Some(&allowed_kinds),
        )?);
    }
    let available_nodes = reachable
        .iter()
        .filter(|id| after_fact_id.is_none_or(|after| id.as_str() > after))
        .count();
    if available_nodes > 0 && selected.is_empty() {
        return Err(ProjectionError::BudgetTooSmall);
    }
    loop {
        let remaining_nodes = available_nodes.saturating_sub(selected.len());
        let expansion =
            expansion_handle(input, &query_fingerprint, selected.last(), remaining_nodes)?;
        let status = if remaining_nodes > 0 {
            ProjectionStatus::PartialBudget
        } else {
            boundary_status(&relevant_boundaries).unwrap_or(ProjectionStatus::Complete)
        };
        let result = finish(
            input,
            query_fingerprint.clone(),
            status,
            selected.clone(),
            relevant_boundaries.clone(),
            expansion,
        )?;
        if rendered_result_bytes(&result)? <= query.budget.max_bytes {
            return Ok(result);
        }
        if selected.pop().is_none() || selected.is_empty() {
            return Err(ProjectionError::BudgetTooSmall);
        }
    }
}

fn expansion_handle(
    input: &SemanticProjectionInput,
    query_fingerprint: &str,
    last: Option<&ProjectionNode>,
    remaining_nodes: usize,
) -> Result<Option<ExpansionHandle>, ProjectionError> {
    if remaining_nodes == 0 {
        return Ok(None);
    }
    let mut handle = ExpansionHandle {
        schema: PROJECTION_SCHEMA.into(),
        composite_snapshot_hash: input.provenance.composite_snapshot_hash.clone(),
        query_fingerprint: query_fingerprint.to_owned(),
        after_fact_id: last.map(|node| node.id.clone()).unwrap_or_default(),
        remaining_nodes,
        fingerprint: String::new(),
    };
    handle.fingerprint = expansion_fingerprint(&handle)?;
    Ok(Some(handle))
}

fn ensure_result_fits(
    result: ProjectionResult,
    max_bytes: usize,
) -> Result<ProjectionResult, ProjectionError> {
    let bytes = rendered_result_bytes(&result)?;
    (bytes <= max_bytes)
        .then_some(result)
        .ok_or(ProjectionError::BudgetTooSmall)
}

/// Size of the actual CLI representation, including its trailing newline.
/// This is the model-visible payload governed by `ProjectionBudget.max_bytes`.
fn rendered_result_bytes(result: &ProjectionResult) -> Result<usize, ProjectionError> {
    canonical::pretty(result)
        .map(|rendered| rendered.len().saturating_add(1))
        .map_err(|error| ProjectionError::Canonical(error.to_string()))
}

/// Replays an initial query and verifies the complete result, not only its
/// included claim nodes. This binds status, boundaries, expansion cursor,
/// provenance and query fingerprint to the immutable semantic input.
pub fn validate_projection(
    input: &SemanticProjectionInput,
    query: &ProjectionQuery,
    result: &ProjectionResult,
) -> Result<(), ProjectionError> {
    validate_trace_to_l0(input, result)?;
    let expected = project(input, query)?;
    if expected != *result {
        return Err(ProjectionError::InvalidTrace(
            "query-bound projection result".into(),
        ));
    }
    Ok(())
}

/// Query-bound replay validation for an expansion page.
pub fn validate_expansion(
    input: &SemanticProjectionInput,
    query: &ProjectionQuery,
    handle: &ExpansionHandle,
    result: &ProjectionResult,
) -> Result<(), ProjectionError> {
    validate_trace_to_l0(input, result)?;
    let expected = expand(input, query, handle)?;
    if expected != *result {
        return Err(ProjectionError::InvalidTrace(
            "query-bound expanded projection result".into(),
        ));
    }
    Ok(())
}

/// Verifies that every returned claim still has exact, unchanged links to L0.
/// It is intentionally usable independently of `project` for replay checks.
pub fn validate_trace_to_l0(
    input: &SemanticProjectionInput,
    result: &ProjectionResult,
) -> Result<(), ProjectionError> {
    validate_input(input)?;
    if result.provenance != input.provenance {
        return Err(ProjectionError::InvalidTrace("snapshot provenance".into()));
    }
    let facts: BTreeMap<_, _> = input
        .facts
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    for node in &result.nodes {
        if node.level != ProjectionLevel::L0 && node.source.is_some() {
            return Err(ProjectionError::SourceAboveL0(node.id.clone()));
        }
        let expected_node = projection_node(
            facts
                .get(node.id.as_str())
                .ok_or_else(|| ProjectionError::InvalidTrace(node.id.clone()))?,
            &facts,
            &input.edges,
            None,
        )?;
        if node.fingerprint != expected_node.fingerprint
            || node.summary != expected_node.summary
            || node.evidence != expected_node.evidence
        {
            return Err(ProjectionError::InvalidNodeFingerprint(node.id.clone()));
        }
        for link in &node.evidence {
            let l0 = facts
                .get(link.l0_fact_id.as_str())
                .ok_or_else(|| ProjectionError::InvalidTrace(link.claim_node_id.clone()))?;
            if l0.level != ProjectionLevel::L0
                || l0.fingerprint != link.l0_fingerprint
                || link.claim_node_id != node.id
                || link.claim_fingerprint != node.fingerprint
            {
                return Err(ProjectionError::InvalidTrace(link.claim_node_id.clone()));
            }
        }
    }
    if result.fingerprint != result_fingerprint(result)? {
        return Err(ProjectionError::InvalidTrace("result fingerprint".into()));
    }
    Ok(())
}

fn validate_input(input: &SemanticProjectionInput) -> Result<(), ProjectionError> {
    if input.schema != PROJECTION_SCHEMA {
        return Err(ProjectionError::UnsupportedSchema(input.schema.clone()));
    }
    for value in [
        &input.provenance.base_revision,
        &input.provenance.composite_snapshot_hash,
        &input.provenance.index_snapshot_hash,
        &input.provenance.project_model_hash,
        &input.provenance.classpath_hash,
        &input.provenance.compiler_version,
        &input.provenance.compiler_options_hash,
        &input.provenance.compilation,
    ] {
        if value.is_empty() {
            return Err(ProjectionError::InvalidTrace(
                "empty snapshot provenance".into(),
            ));
        }
    }
    let expected_composite = canonical::hash(&serde_json::json!({
        "baseRevision":input.provenance.base_revision,
        "indexSnapshot":input.provenance.index_snapshot_hash,
        "projectModelHash":input.provenance.project_model_hash,
        "classpathHash":input.provenance.classpath_hash,
        "compilerVersion":input.provenance.compiler_version,
        "compilerOptionsHash":input.provenance.compiler_options_hash,
        "compilation":input.provenance.compilation,
    }))
    .map_err(|error| ProjectionError::Canonical(error.to_string()))?;
    if input.provenance.composite_snapshot_hash != expected_composite {
        return Err(ProjectionError::InvalidTrace(
            "composite snapshot provenance".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for fact in &input.facts {
        if !ids.insert(fact.id.clone()) {
            return Err(ProjectionError::DuplicateId("fact", fact.id.clone()));
        }
        if fact.source.is_some() && fact.level != ProjectionLevel::L0 {
            return Err(ProjectionError::SourceAboveL0(fact.id.clone()));
        }
        if fact.fingerprint != fact_fingerprint(fact)? {
            return Err(ProjectionError::InvalidFactFingerprint(fact.id.clone()));
        }
    }
    let map: BTreeMap<_, _> = input
        .facts
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    for fact in &input.facts {
        if fact.level == ProjectionLevel::L0 {
            let Some(source) = &fact.source else {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    "L0 requires source provenance".into(),
                ));
            };
            if source.file.is_empty()
                || source.content_hash.is_empty()
                || source.range_end <= source.range_start
            {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    "L0 source provenance is incomplete".into(),
                ));
            }
            let Some(snippet) = source
                .snippet
                .as_deref()
                .filter(|snippet| !snippet.is_empty())
            else {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    "L0 requires nonempty exact source text".into(),
                ));
            };
            if canonical::hash_bytes(snippet.as_bytes()) != source.content_hash {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    "L0 snippet does not match its content hash".into(),
                ));
            }
            if fact.l0_evidence != [fact.id.clone()] {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    "L0 evidence must be self".into(),
                ));
            }
        }
        if fact.l0_evidence.is_empty() {
            return Err(ProjectionError::InvalidEvidence(
                fact.id.clone(),
                "claim has no L0 evidence".into(),
            ));
        }
        for evidence in &fact.l0_evidence {
            let Some(l0) = map.get(evidence.as_str()) else {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    format!("missing {evidence}"),
                ));
            };
            if l0.level != ProjectionLevel::L0 {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    format!("{evidence} is not L0"),
                ));
            }
        }
    }
    let mut edge_ids = BTreeSet::new();
    for edge in &input.edges {
        if !edge_ids.insert(edge.id.clone()) {
            return Err(ProjectionError::DuplicateId("edge", edge.id.clone()));
        }
        if !map.contains_key(edge.from.as_str()) || !map.contains_key(edge.to.as_str()) {
            return Err(ProjectionError::BrokenEdge(edge.id.clone()));
        }
        let from = map[edge.from.as_str()].level;
        let to = map[edge.to.as_str()].level;
        if from > to && !is_immediately_lower(from, to) {
            return Err(ProjectionError::InvalidEvidence(
                edge.from.clone(),
                format!(
                    "edge {} skips from L{} to L{}",
                    edge.id,
                    level_number(from),
                    level_number(to)
                ),
            ));
        }
    }
    for fact in &input.facts {
        for l0_id in &fact.l0_evidence {
            if exact_ladder_path(&input.edges, &map, &fact.id, l0_id, None).is_none() {
                return Err(ProjectionError::InvalidEvidence(
                    fact.id.clone(),
                    format!(
                        "no exact descending L{}-to-L0 path to {l0_id}",
                        level_number(fact.level)
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn fact_fingerprint(fact: &SemanticFact) -> Result<String, ProjectionError> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        id: &'a str,
        level: ProjectionLevel,
        kind: &'a str,
        summary: &'a str,
        l0_evidence: &'a [String],
        source: &'a Option<L0Source>,
    }
    canonical::hash(&Fingerprint {
        id: &fact.id,
        level: fact.level,
        kind: &fact.kind,
        summary: &fact.summary,
        l0_evidence: &fact.l0_evidence,
        source: &fact.source,
    })
    .map_err(|error| ProjectionError::Canonical(error.to_string()))
}

fn query_fingerprint(query: &ProjectionQuery) -> Result<String, ProjectionError> {
    #[derive(Serialize)]
    struct Stable<'a> {
        schema: &'a str,
        level: ProjectionLevel,
        roots: Vec<&'a str>,
        kinds: Vec<ThreadKind>,
        traversal: Traversal,
        budget: ProjectionBudget,
        boundary_policy: BoundaryPolicy,
    }
    let mut roots: Vec<_> = query.roots.iter().map(String::as_str).collect();
    roots.sort();
    roots.dedup();
    let mut kinds = query.thread_kinds.clone();
    kinds.sort();
    kinds.dedup();
    canonical::hash(&Stable {
        schema: &query.schema,
        level: query.level,
        roots,
        kinds,
        traversal: query.traversal,
        budget: query.budget.clone(),
        boundary_policy: query.boundary_policy,
    })
    .map_err(|error| ProjectionError::Canonical(error.to_string()))
}

fn reachable_fact_ids(
    input: &SemanticProjectionInput,
    query: &ProjectionQuery,
    facts: &BTreeMap<&str, &SemanticFact>,
) -> Vec<String> {
    let allowed: BTreeSet<_> = query.thread_kinds.iter().copied().collect();
    let mut adjacent: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &input.edges {
        if !allowed.is_empty() && !allowed.contains(&edge.kind) {
            continue;
        }
        if matches!(query.traversal, Traversal::Forward | Traversal::Both) {
            adjacent
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
        }
        if matches!(query.traversal, Traversal::Backward | Traversal::Both) {
            adjacent
                .entry(edge.to.as_str())
                .or_default()
                .push(edge.from.as_str());
        }
    }
    for targets in adjacent.values_mut() {
        targets.sort();
        targets.dedup();
    }
    let mut queue: VecDeque<_> = query.roots.iter().map(String::as_str).collect();
    let mut seen = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(next) = adjacent.get(id) {
            queue.extend(next.iter().copied());
        }
    }
    let mut ids: Vec<_> = seen
        .into_iter()
        .filter(|id| facts.contains_key(id))
        .map(str::to_owned)
        .collect();
    ids.sort();
    ids
}

fn exact_ladder_path(
    edges: &[SemanticEdge],
    facts: &BTreeMap<&str, &SemanticFact>,
    from: &str,
    to: &str,
    allowed_kinds: Option<&BTreeSet<ThreadKind>>,
) -> Option<Vec<String>> {
    if from == to {
        return (facts.get(from)?.level == ProjectionLevel::L0).then(|| vec![from.into()]);
    }
    let mut adjacent: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in edges {
        if allowed_kinds.is_some_and(|allowed| !allowed.is_empty() && !allowed.contains(&edge.kind))
        {
            continue;
        }
        adjacent
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }
    for next in adjacent.values_mut() {
        next.sort();
        next.dedup();
    }
    let mut queue = VecDeque::from([from]);
    let mut seen = BTreeSet::from([from]);
    let mut previous: BTreeMap<&str, &str> = BTreeMap::new();
    while let Some(current) = queue.pop_front() {
        for next in adjacent.get(current).into_iter().flatten() {
            let current_level = facts.get(current)?.level;
            let next_level = facts.get(*next)?.level;
            if !is_immediately_lower(current_level, next_level) {
                continue;
            }
            if seen.insert(*next) {
                previous.insert(*next, current);
                if *next == to {
                    let mut path = vec![to];
                    let mut cursor = to;
                    while let Some(parent) = previous.get(cursor) {
                        path.push(*parent);
                        cursor = parent;
                    }
                    path.reverse();
                    return Some(path.into_iter().map(str::to_owned).collect());
                }
                queue.push_back(next);
            }
        }
    }
    None
}

fn is_immediately_lower(from: ProjectionLevel, to: ProjectionLevel) -> bool {
    matches!(
        (from, to),
        (ProjectionLevel::L5, ProjectionLevel::L4)
            | (ProjectionLevel::L4, ProjectionLevel::L3)
            | (ProjectionLevel::L3, ProjectionLevel::L2)
            | (ProjectionLevel::L2, ProjectionLevel::L1)
            | (ProjectionLevel::L1, ProjectionLevel::L0)
    )
}

fn level_number(level: ProjectionLevel) -> u8 {
    match level {
        ProjectionLevel::L0 => 0,
        ProjectionLevel::L1 => 1,
        ProjectionLevel::L2 => 2,
        ProjectionLevel::L3 => 3,
        ProjectionLevel::L4 => 4,
        ProjectionLevel::L5 => 5,
    }
}

fn relevant_boundaries(
    input: &SemanticProjectionInput,
    reachable: &[String],
) -> Vec<SemanticBoundary> {
    let ids: BTreeSet<_> = reachable.iter().map(String::as_str).collect();
    let mut boundaries: Vec<_> = input
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary
                .affected_fact_ids
                .iter()
                .any(|id| ids.contains(id.as_str()))
        })
        .cloned()
        .collect();
    boundaries.sort_by(|a, b| a.id.cmp(&b.id));
    boundaries
}

fn boundary_status(boundaries: &[SemanticBoundary]) -> Option<ProjectionStatus> {
    if boundaries
        .iter()
        .any(|b| matches!(b.state, BoundaryState::Partial | BoundaryState::Unsupported))
    {
        Some(ProjectionStatus::PartialBoundary)
    } else if boundaries.iter().any(|b| b.state == BoundaryState::Unknown) {
        Some(ProjectionStatus::UnknownBoundary)
    } else {
        None
    }
}

fn projection_node(
    fact: &SemanticFact,
    facts: &BTreeMap<&str, &SemanticFact>,
    edges: &[SemanticEdge],
    allowed_kinds: Option<&BTreeSet<ThreadKind>>,
) -> Result<ProjectionNode, ProjectionError> {
    let source = if fact.level == ProjectionLevel::L0 {
        fact.source.clone()
    } else {
        None
    };
    let mut node = ProjectionNode {
        id: fact.id.clone(),
        level: fact.level,
        kind: fact.kind.clone(),
        summary: fact.summary.clone(),
        fingerprint: String::new(),
        evidence: Vec::new(),
        source,
    };
    node.fingerprint = node_fingerprint(&node, &fact.fingerprint)?;
    node.evidence = fact
        .l0_evidence
        .iter()
        .map(|id| {
            let l0 = facts[id.as_str()];
            let path =
                exact_ladder_path(edges, facts, &fact.id, id, allowed_kinds).ok_or_else(|| {
                    ProjectionError::InvalidEvidence(
                        fact.id.clone(),
                        format!("no directed path to {id}"),
                    )
                })?;
            Ok(EvidenceLink {
                l0_fact_id: id.clone(),
                l0_fingerprint: l0.fingerprint.clone(),
                claim_node_id: node.id.clone(),
                claim_fingerprint: node.fingerprint.clone(),
                path,
            })
        })
        .collect::<Result<_, ProjectionError>>()?;
    Ok(node)
}

fn node_fingerprint(
    node: &ProjectionNode,
    fact_fingerprint: &str,
) -> Result<String, ProjectionError> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        id: &'a str,
        level: ProjectionLevel,
        kind: &'a str,
        summary: &'a str,
        fact_fingerprint: &'a str,
        source: &'a Option<L0Source>,
    }
    canonical::hash(&Fingerprint {
        id: &node.id,
        level: node.level,
        kind: &node.kind,
        summary: &node.summary,
        fact_fingerprint,
        source: &node.source,
    })
    .map_err(|error| ProjectionError::Canonical(error.to_string()))
}

fn expansion_fingerprint(handle: &ExpansionHandle) -> Result<String, ProjectionError> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        schema: &'a str,
        composite_snapshot_hash: &'a str,
        query_fingerprint: &'a str,
        after_fact_id: &'a str,
        remaining_nodes: usize,
    }
    canonical::hash(&Fingerprint {
        schema: &handle.schema,
        composite_snapshot_hash: &handle.composite_snapshot_hash,
        query_fingerprint: &handle.query_fingerprint,
        after_fact_id: &handle.after_fact_id,
        remaining_nodes: handle.remaining_nodes,
    })
    .map_err(|error| ProjectionError::Canonical(error.to_string()))
}

fn finish(
    input: &SemanticProjectionInput,
    query_fingerprint: String,
    status: ProjectionStatus,
    nodes: Vec<ProjectionNode>,
    boundaries: Vec<SemanticBoundary>,
    expansion: Option<ExpansionHandle>,
) -> Result<ProjectionResult, ProjectionError> {
    let mut result = ProjectionResult {
        schema: PROJECTION_SCHEMA.into(),
        provenance: input.provenance.clone(),
        query_fingerprint,
        status,
        nodes,
        boundaries,
        expansion,
        fingerprint: String::new(),
    };
    result.fingerprint = result_fingerprint(&result)?;
    Ok(result)
}

fn result_fingerprint(result: &ProjectionResult) -> Result<String, ProjectionError> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        schema: &'a str,
        provenance: &'a ProjectionProvenance,
        query_fingerprint: &'a str,
        status: ProjectionStatus,
        nodes: &'a [ProjectionNode],
        boundaries: &'a [SemanticBoundary],
        expansion: &'a Option<ExpansionHandle>,
    }
    canonical::hash(&Fingerprint {
        schema: &result.schema,
        provenance: &result.provenance,
        query_fingerprint: &result.query_fingerprint,
        status: result.status,
        nodes: &result.nodes,
        boundaries: &result.boundaries,
        expansion: &result.expansion,
    })
    .map_err(|error| ProjectionError::Canonical(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> ProjectionProvenance {
        let mut provenance = ProjectionProvenance {
            base_revision: "revision:1".into(),
            composite_snapshot_hash: String::new(),
            index_snapshot_hash: "index:1".into(),
            project_model_hash: "model:1".into(),
            classpath_hash: "classpath:1".into(),
            compiler_version: "2.1.21".into(),
            compiler_options_hash: "options:1".into(),
            compilation: ":/main".into(),
        };
        provenance.composite_snapshot_hash = canonical::hash(&serde_json::json!({
            "baseRevision":provenance.base_revision,
            "indexSnapshot":provenance.index_snapshot_hash,
            "projectModelHash":provenance.project_model_hash,
            "classpathHash":provenance.classpath_hash,
            "compilerVersion":provenance.compiler_version,
            "compilerOptionsHash":provenance.compiler_options_hash,
            "compilation":provenance.compilation,
        }))
        .unwrap();
        provenance
    }
    fn source(file: &str, snippet: &str) -> L0Source {
        L0Source {
            file: file.into(),
            content_hash: canonical::hash_bytes(snippet.as_bytes()),
            range_start: 1,
            range_end: 10,
            snippet: Some(snippet.into()),
        }
    }
    fn input(facts: Vec<SemanticFact>, edges: Vec<SemanticEdge>) -> SemanticProjectionInput {
        SemanticProjectionInput {
            schema: PROJECTION_SCHEMA.into(),
            provenance: provenance(),
            facts,
            edges,
            boundaries: vec![],
        }
    }
    fn query(root: &str, level: ProjectionLevel) -> ProjectionQuery {
        ProjectionQuery {
            schema: PROJECTION_SCHEMA.into(),
            level,
            roots: vec![root.into()],
            thread_kinds: vec![ThreadKind::Data],
            traversal: Traversal::Both,
            budget: ProjectionBudget::default(),
            boundary_policy: BoundaryPolicy::ReturnPartial,
        }
    }

    #[test]
    fn l5_claim_traces_to_exact_l0_without_leaking_source() {
        let l0 = SemanticFact::new(
            "l0:call",
            ProjectionLevel::L0,
            "CALL",
            "resolved call",
            vec![],
            Some(source("A.kt", "service.load()")),
        )
        .unwrap();
        let l1 = SemanticFact::new(
            "l1:symbol",
            ProjectionLevel::L1,
            "SYMBOL",
            "service.load",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let l2 = SemanticFact::new(
            "l2:component",
            ProjectionLevel::L2,
            "COMPONENT_CONTRACT_EFFECT",
            "load component",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let l3 = SemanticFact::new(
            "l3:thread",
            ProjectionLevel::L3,
            "SEMANTIC_THREAD",
            "load journey",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let l4 = SemanticFact::new(
            "l4:architecture",
            ProjectionLevel::L4,
            "ARCHITECTURE_OWNERSHIP",
            "load owner",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let l5 = SemanticFact::new(
            "l5:goal",
            ProjectionLevel::L5,
            "CHANGE",
            "update load journey",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let graph = input(
            vec![l0, l1, l2, l3, l4, l5],
            [
                ("e54", "l5:goal", "l4:architecture"),
                ("e43", "l4:architecture", "l3:thread"),
                ("e32", "l3:thread", "l2:component"),
                ("e21", "l2:component", "l1:symbol"),
                ("e10", "l1:symbol", "l0:call"),
            ]
            .into_iter()
            .map(|(id, from, to)| SemanticEdge {
                id: id.into(),
                from: from.into(),
                to: to.into(),
                kind: ThreadKind::Data,
            })
            .collect(),
        );
        let result = project(&graph, &query("l5:goal", ProjectionLevel::L5)).unwrap();
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].id, "l5:goal");
        assert!(result.nodes[0].source.is_none());
        assert_eq!(result.nodes[0].evidence.len(), 1);
        validate_trace_to_l0(&graph, &result).unwrap();
        let mut wrong_kind = query("l5:goal", ProjectionLevel::L5);
        wrong_kind.thread_kinds = vec![ThreadKind::Effect];
        assert!(matches!(
            project(&graph, &wrong_kind),
            Err(ProjectionError::InvalidEvidence(id, _)) if id == "l5:goal"
        ));
    }

    #[test]
    fn direct_l5_to_l0_relabeling_is_rejected() {
        let l0 = SemanticFact::new(
            "l0",
            ProjectionLevel::L0,
            "VALUE",
            "source",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let intermediates = [
            ("l1", ProjectionLevel::L1),
            ("l2", ProjectionLevel::L2),
            ("l3", ProjectionLevel::L3),
            ("l4", ProjectionLevel::L4),
        ]
        .into_iter()
        .map(|(id, level)| {
            SemanticFact::new(id, level, "LADDER", id, vec![l0.id.clone()], None).unwrap()
        })
        .collect::<Vec<_>>();
        let l5 = SemanticFact::new(
            "l5",
            ProjectionLevel::L5,
            "OUTCOME",
            "unsupported leap",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let graph = input(
            std::iter::once(l0)
                .chain(intermediates)
                .chain(std::iter::once(l5))
                .collect(),
            [
                ("e54", "l5", "l4"),
                ("e43", "l4", "l3"),
                ("e32", "l3", "l2"),
                ("e21", "l2", "l1"),
                ("e10", "l1", "l0"),
                ("shortcut", "l5", "l0"),
            ]
            .into_iter()
            .map(|(id, from, to)| SemanticEdge {
                id: id.into(),
                from: from.into(),
                to: to.into(),
                kind: ThreadKind::Data,
            })
            .collect(),
        );
        assert!(matches!(
            project(&graph, &query("l5", ProjectionLevel::L5)),
            Err(ProjectionError::InvalidEvidence(id, _)) if id == "l5"
        ));
    }

    #[test]
    fn ten_x_irrelevant_padding_does_not_change_bounded_projection() {
        let root = SemanticFact::new(
            "root",
            ProjectionLevel::L0,
            "VALUE",
            "root",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let symbol = SemanticFact::new(
            "symbol",
            ProjectionLevel::L1,
            "SYMBOL",
            "symbol",
            vec![root.id.clone()],
            None,
        )
        .unwrap();
        let child = SemanticFact::new(
            "child",
            ProjectionLevel::L2,
            "SYMBOL",
            "child",
            vec![root.id.clone()],
            None,
        )
        .unwrap();
        let journey = SemanticFact::new(
            "journey",
            ProjectionLevel::L3,
            "SEMANTIC_THREAD",
            "journey",
            vec![root.id.clone()],
            None,
        )
        .unwrap();
        let goal = SemanticFact::new(
            "goal",
            ProjectionLevel::L4,
            "JOURNEY",
            "goal",
            vec![root.id.clone()],
            None,
        )
        .unwrap();
        let base = input(
            vec![root.clone(), symbol, child.clone(), journey, goal],
            vec![
                SemanticEdge {
                    id: "child-to-symbol".into(),
                    from: "child".into(),
                    to: "symbol".into(),
                    kind: ThreadKind::Data,
                },
                SemanticEdge {
                    id: "symbol-to-root".into(),
                    from: "symbol".into(),
                    to: "root".into(),
                    kind: ThreadKind::Data,
                },
                SemanticEdge {
                    id: "goal-to-journey".into(),
                    from: "goal".into(),
                    to: "journey".into(),
                    kind: ThreadKind::Data,
                },
                SemanticEdge {
                    id: "journey-to-child".into(),
                    from: "journey".into(),
                    to: "child".into(),
                    kind: ThreadKind::Data,
                },
            ],
        );
        let mut padded = base.clone();
        for number in 0..20 {
            let l0 = SemanticFact::new(
                format!("pad:l0:{number}"),
                ProjectionLevel::L0,
                "VALUE",
                format!("padding {number}"),
                vec![],
                Some(source(&format!("P{number}.kt"), "ignored")),
            )
            .unwrap();
            let l1 = SemanticFact::new(
                format!("pad:l1:{number}"),
                ProjectionLevel::L1,
                "SYMBOL",
                "ignored",
                vec![l0.id.clone()],
                None,
            )
            .unwrap();
            let l2 = SemanticFact::new(
                format!("pad:l2:{number}"),
                ProjectionLevel::L2,
                "SYMBOL",
                "ignored",
                vec![l0.id.clone()],
                None,
            )
            .unwrap();
            padded.edges.push(SemanticEdge {
                id: format!("pad-l2-l1:{number}"),
                from: l2.id.clone(),
                to: l1.id.clone(),
                kind: ThreadKind::Data,
            });
            padded.edges.push(SemanticEdge {
                id: format!("pad-l1-l0:{number}"),
                from: l1.id.clone(),
                to: l0.id.clone(),
                kind: ThreadKind::Data,
            });
            padded.facts.extend([l0, l1, l2]);
        }
        let baseline = project(&base, &query("root", ProjectionLevel::L4)).unwrap();
        let with_padding = project(&padded, &query("root", ProjectionLevel::L4)).unwrap();
        assert_eq!(baseline.fingerprint, with_padding.fingerprint);
        assert_eq!(baseline.nodes, with_padding.nodes);
    }

    #[test]
    fn upper_summary_mutation_invalidates_claim_evidence() {
        let l0 = SemanticFact::new(
            "l0",
            ProjectionLevel::L0,
            "VALUE",
            "source",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let l1 = SemanticFact::new(
            "l1",
            ProjectionLevel::L1,
            "SYMBOL",
            "symbol",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let l2 = SemanticFact::new(
            "l2",
            ProjectionLevel::L2,
            "COMPONENT_CONTRACT_EFFECT",
            "effect contract",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let l3 = SemanticFact::new(
            "l3",
            ProjectionLevel::L3,
            "SEMANTIC_THREAD",
            "effect thread",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let upper = SemanticFact::new(
            "upper",
            ProjectionLevel::L4,
            "EFFECT",
            "pure",
            vec![l0.id.clone()],
            None,
        )
        .unwrap();
        let graph = input(
            vec![l0, l1, l2, l3, upper],
            [
                ("e43", "upper", "l3"),
                ("e32", "l3", "l2"),
                ("e21", "l2", "l1"),
                ("e10", "l1", "l0"),
            ]
            .into_iter()
            .map(|(id, from, to)| SemanticEdge {
                id: id.into(),
                from: from.into(),
                to: to.into(),
                kind: ThreadKind::Effect,
            })
            .collect(),
        );
        let mut effect_query = query("upper", ProjectionLevel::L4);
        effect_query.thread_kinds = vec![ThreadKind::Effect];
        let mut result = project(&graph, &effect_query).unwrap();
        result.nodes[0].summary = "impure".into();
        assert_eq!(
            validate_trace_to_l0(&graph, &result),
            Err(ProjectionError::InvalidNodeFingerprint("upper".into()))
        );
    }

    #[test]
    fn unsupported_boundary_can_be_partial_or_refused() {
        let l0 = SemanticFact::new(
            "l0",
            ProjectionLevel::L0,
            "VALUE",
            "source",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let mut graph = input(vec![l0], vec![]);
        graph.boundaries.push(SemanticBoundary {
            id: "reflection".into(),
            state: BoundaryState::Unsupported,
            affected_fact_ids: vec!["l0".into()],
            reason: "dynamic dispatch".into(),
        });
        let partial = project(&graph, &query("l0", ProjectionLevel::L3)).unwrap();
        assert_eq!(partial.status, ProjectionStatus::PartialBoundary);
        let mut refusing = query("l0", ProjectionLevel::L3);
        refusing.boundary_policy = BoundaryPolicy::Refuse;
        let refused = project(&graph, &refusing).unwrap();
        assert_eq!(refused.status, ProjectionStatus::RefusedUnsupported);
        assert!(refused.nodes.is_empty());
    }

    #[test]
    fn byte_budget_returns_deterministic_expansion_handle() {
        let root = SemanticFact::new(
            "a",
            ProjectionLevel::L0,
            "VALUE",
            "a",
            vec![],
            Some(source("A.kt", "a")),
        )
        .unwrap();
        let child = SemanticFact::new(
            "b",
            ProjectionLevel::L0,
            "VALUE",
            "b",
            vec![],
            Some(source("B.kt", "b")),
        )
        .unwrap();
        let graph = input(
            vec![root, child],
            vec![SemanticEdge {
                id: "e".into(),
                from: "a".into(),
                to: "b".into(),
                kind: ThreadKind::Data,
            }],
        );
        let mut limited = query("a", ProjectionLevel::L0);
        limited.budget.max_nodes = 1;
        let first = project(&graph, &limited).unwrap();
        assert_eq!(first.status, ProjectionStatus::PartialBudget);
        assert!(rendered_result_bytes(&first).unwrap() <= limited.budget.max_bytes);
        let handle = first.expansion.clone().unwrap();
        assert_eq!(handle.after_fact_id, "a");
        let second = expand(&graph, &limited, &handle).unwrap();
        assert_eq!(
            second.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
            vec!["b"]
        );
    }

    #[test]
    fn stale_expansion_handle_is_refused() {
        let root = SemanticFact::new(
            "a",
            ProjectionLevel::L0,
            "VALUE",
            "a",
            vec![],
            Some(source("A.kt", "a")),
        )
        .unwrap();
        let child = SemanticFact::new(
            "b",
            ProjectionLevel::L0,
            "VALUE",
            "b",
            vec![],
            Some(source("B.kt", "b")),
        )
        .unwrap();
        let graph = input(
            vec![root, child],
            vec![SemanticEdge {
                id: "e".into(),
                from: "a".into(),
                to: "b".into(),
                kind: ThreadKind::Data,
            }],
        );
        let mut limited = query("a", ProjectionLevel::L0);
        limited.budget.max_nodes = 1;
        let handle = project(&graph, &limited).unwrap().expansion.unwrap();
        let other = query("b", ProjectionLevel::L0);
        assert!(matches!(
            expand(&graph, &other, &handle),
            Err(ProjectionError::InvalidExpansion(_))
        ));
    }

    #[test]
    fn missing_target_level_is_an_explicit_unknown_not_complete() {
        let l0 = SemanticFact::new(
            "l0",
            ProjectionLevel::L0,
            "VALUE",
            "source",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let result = project(&input(vec![l0], vec![]), &query("l0", ProjectionLevel::L5)).unwrap();
        assert_eq!(result.status, ProjectionStatus::UnknownBoundary);
        assert_eq!(result.boundaries[0].id, "NO_FACTS_AT_LEVEL");
    }

    #[test]
    fn non_progressing_budget_is_refused() {
        let l0 = SemanticFact::new(
            "l0",
            ProjectionLevel::L0,
            "VALUE",
            "source",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let graph = input(vec![l0], vec![]);
        let mut limited = query("l0", ProjectionLevel::L0);
        limited.budget.max_nodes = 0;
        assert_eq!(
            project(&graph, &limited),
            Err(ProjectionError::BudgetTooSmall)
        );
    }

    #[test]
    fn query_bound_validation_rejects_rehashed_result_mutations() {
        let l0 = SemanticFact::new(
            "l0",
            ProjectionLevel::L0,
            "VALUE",
            "source",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let graph = input(vec![l0], vec![]);
        let query = query("l0", ProjectionLevel::L0);
        let mut result = project(&graph, &query).unwrap();
        result.status = ProjectionStatus::PartialBoundary;
        result.fingerprint = result_fingerprint(&result).unwrap();
        assert_eq!(
            validate_projection(&graph, &query, &result),
            Err(ProjectionError::InvalidTrace(
                "query-bound projection result".into()
            ))
        );
    }

    #[test]
    fn rehashed_forged_expansion_cursor_is_rejected() {
        let first = SemanticFact::new(
            "a",
            ProjectionLevel::L0,
            "VALUE",
            "a",
            vec![],
            Some(source("A.kt", "a")),
        )
        .unwrap();
        let second = SemanticFact::new(
            "b",
            ProjectionLevel::L0,
            "VALUE",
            "b",
            vec![],
            Some(source("B.kt", "b")),
        )
        .unwrap();
        let graph = input(
            vec![first, second],
            vec![SemanticEdge {
                id: "e".into(),
                from: "a".into(),
                to: "b".into(),
                kind: ThreadKind::Data,
            }],
        );
        let mut limited = query("a", ProjectionLevel::L0);
        limited.budget.max_nodes = 1;
        let mut handle = project(&graph, &limited).unwrap().expansion.unwrap();
        handle.remaining_nodes = 0;
        handle.fingerprint = expansion_fingerprint(&handle).unwrap();
        assert!(matches!(
            expand(&graph, &limited, &handle),
            Err(ProjectionError::InvalidExpansion(_))
        ));
    }

    #[test]
    fn l0_source_and_composite_provenance_mutations_fail_closed() {
        let l0 = SemanticFact::new(
            "l0",
            ProjectionLevel::L0,
            "VALUE",
            "source",
            vec![],
            Some(source("A.kt", "x")),
        )
        .unwrap();
        let mut graph = input(vec![l0], vec![]);
        let query = query("l0", ProjectionLevel::L0);
        let result = project(&graph, &query).unwrap();

        graph.facts[0].source.as_mut().unwrap().snippet = None;
        graph.facts[0].fingerprint = fact_fingerprint(&graph.facts[0]).unwrap();
        assert!(matches!(
            project(&graph, &query),
            Err(ProjectionError::InvalidEvidence(_, _))
        ));

        graph.facts[0].source = Some(source("A.kt", "x"));
        graph.facts[0].source.as_mut().unwrap().snippet = Some("y".into());
        graph.facts[0].fingerprint = fact_fingerprint(&graph.facts[0]).unwrap();
        assert!(matches!(
            project(&graph, &query),
            Err(ProjectionError::InvalidEvidence(_, _))
        ));

        graph.facts[0].source = Some(source("A.kt", "x"));
        graph.facts[0].fingerprint = fact_fingerprint(&graph.facts[0]).unwrap();
        graph.provenance.compiler_version = "different".into();
        graph.provenance.composite_snapshot_hash = canonical::hash(&serde_json::json!({
            "baseRevision":graph.provenance.base_revision,
            "indexSnapshot":graph.provenance.index_snapshot_hash,
            "projectModelHash":graph.provenance.project_model_hash,
            "classpathHash":graph.provenance.classpath_hash,
            "compilerVersion":graph.provenance.compiler_version,
            "compilerOptionsHash":graph.provenance.compiler_options_hash,
            "compilation":graph.provenance.compilation,
        }))
        .unwrap();
        assert_eq!(
            validate_projection(&graph, &query, &result),
            Err(ProjectionError::InvalidTrace("snapshot provenance".into()))
        );
    }
}
