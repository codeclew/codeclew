//! Read-only question packets over a verified retained flow.
//!
//! This profile resolves evidence selectors, not natural-language intent. It
//! retains a conservative callable envelope and never claims semantic closure.
//! Callers must verify the parent against its retained compiler fact set first.

use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use crate::thread_callables::CallableCompleteness;
use crate::thread_flow::{FlowNodeKind, FlowSlice, FlowStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REQUEST_SCHEMA: &str = "codeclew-question-request/0.1";
pub const RESULT_SCHEMA: &str = "codeclew-question-packet/0.1";
pub const POLICY: &str = "retained-flow-envelope/0.1";
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_RESULT_BYTES: usize = 64 * 1024;
pub const MAX_CRITERIA: usize = 32;
pub const MAX_PACKET_METHODS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionRequest {
    pub schema: String,
    pub flow_id: String,
    pub flow_digest: String,
    pub purpose: Purpose,
    pub criteria: Vec<Criterion>,
    pub max_methods: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_order_queries: Vec<crate::thread_question_control::OrderQuery>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Purpose {
    Problems,
    Characteristics,
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    /// A user-authored label, never evidence of an effect or business meaning.
    pub label: String,
    pub selector: Selector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Selector {
    #[serde(rename_all = "camelCase")]
    Callable {
        member_alias: String,
        symbol_identity: String,
    },
    #[serde(rename_all = "camelCase")]
    RelationFact { fact_id: String },
    #[serde(rename_all = "camelCase")]
    CfgNode { region_id: String, node_id: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Discovery {
    FoundInRetainedFlow,
    NotFoundUnknown,
    AmbiguousUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SemanticClosure {
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FrontierKind {
    Semantic,
    Budget,
    Discovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frontier {
    pub code: String,
    pub kind: FrontierKind,
    pub origin_ids: Vec<String>,
    pub affected_criteria: Vec<String>,
    pub required_checks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CriterionResult {
    pub criterion_id: String,
    pub discovery: Discovery,
    pub matched_ids: Vec<String>,
    pub support_fact_ids: Vec<String>,
    pub semantic_closure: SemanticClosure,
}

/// A structural interface only: it does not summarize value or effect transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableEnvelopeSummary {
    pub owner_node_id: String,
    pub outgoing_relation_ids: Vec<String>,
    pub boundary_target_ids: Vec<String>,
    pub control_region_ids: Vec<String>,
    pub source_objects: Vec<CasObject>,
    pub semantic_closure: SemanticClosure,
}

/// The first profile cannot prove independent relevance cuts. One shared task
/// avoids duplicating the entire conservative envelope across model workers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterpretationPlan {
    pub workers: usize,
    pub reason: String,
    pub shared_source_objects: Vec<CasObject>,
    pub unique_source_bytes: u64,
    pub duplicated_source_bytes: u64,
    pub max_model_calls: usize,
    pub max_expansion_rounds: usize,
    pub actual_model_calls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionPacket {
    pub schema: String,
    pub packet_id: String,
    pub policy: String,
    pub request: QuestionRequest,
    pub parent_flow_ref: CasObject,
    pub extraction: CallableCompleteness,
    pub traversal: FlowStatus,
    pub presentation_complete: bool,
    pub presentation_method_ids: Vec<String>,
    /// Cuts are not irrelevance certificates. Their sources remain in the parent.
    pub omitted_method_ids: Vec<String>,
    /// All retained regions stay referenceable, including branches and failures.
    pub control_region_ids: Vec<String>,
    pub relation_ids: Vec<String>,
    pub criteria: Vec<CriterionResult>,
    pub frontier: Vec<Frontier>,
    pub local_order_observations: Vec<crate::thread_question_control::OrderObservation>,
    pub callable_envelope_summaries: Vec<CallableEnvelopeSummary>,
    pub interpretation_plan: InterpretationPlan,
}

pub fn parse_request(bytes: &[u8]) -> Result<QuestionRequest, ClewError> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(budget("question request exceeds 64 KiB"));
    }
    let value = canonical::parse_json_strict(bytes)
        .map_err(|_| invalid("question request is not strict JSON"))?;
    let request: QuestionRequest = serde_json::from_value(value)
        .map_err(|_| invalid("question request is outside its closed contract"))?;
    validate_request(&request)?;
    Ok(request)
}

fn validate_request(request: &QuestionRequest) -> Result<(), ClewError> {
    if request.schema != REQUEST_SCHEMA {
        return Err(invalid("question request schema is unsupported"));
    }
    if request.criteria.is_empty() || request.criteria.len() > MAX_CRITERIA {
        return Err(invalid("question requires between one and 32 criteria"));
    }
    if request.max_methods == 0 || request.max_methods > MAX_PACKET_METHODS {
        return Err(invalid(
            "question method budget must be between one and 128",
        ));
    }
    let mut ids = BTreeSet::new();
    if request.local_order_queries.len() > MAX_CRITERIA {
        return Err(invalid("question allows at most 32 local order queries"));
    }
    for query in &request.local_order_queries {
        bounded_text(&query.id)?;
        bounded_text(&query.region_id)?;
        if !ids.insert(&query.id) {
            return Err(invalid("local order query ids must be unique"));
        }
    }
    ids.clear();
    for criterion in &request.criteria {
        bounded_text(&criterion.id)?;
        bounded_text(&criterion.label)?;
        if !ids.insert(&criterion.id) {
            return Err(invalid("question criterion ids must be unique"));
        }
        match &criterion.selector {
            Selector::Callable {
                member_alias,
                symbol_identity,
            } => {
                bounded_text(member_alias)?;
                crate::semantic_validation::validate_kotlin_full_symbol_identity(symbol_identity)?;
            }
            Selector::RelationFact { fact_id } => bounded_text(fact_id)?,
            Selector::CfgNode { region_id, .. } => bounded_text(region_id)?,
        }
    }
    Ok(())
}

/// Integrity checks do not replace the service's retained authority verification.
pub fn build_verified(
    flow: &FlowSlice,
    flow_ref: &CasObject,
    extraction: &CallableCompleteness,
    mut request: QuestionRequest,
) -> Result<QuestionPacket, ClewError> {
    validate_request(&request)?;
    if request.flow_id != flow.flow_id
        || request.flow_digest != flow_ref.digest
        || CasObject::for_bytes(
            crate::thread_flow::FLOW_SLICE_SCHEMA,
            &canonical::bytes(flow).map_err(internal)?,
        )? != *flow_ref
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "question parent flow binding differs",
        ));
    }
    request.criteria.sort_by(|a, b| a.id.cmp(&b.id));
    request.local_order_queries.sort_by(|a, b| a.id.cmp(&b.id));
    let all_criteria = request
        .criteria
        .iter()
        .map(|c| c.id.clone())
        .collect::<Vec<_>>();
    let mut frontier = flow
        .boundaries
        .iter()
        .map(|b| Frontier {
            code: b.code.clone(),
            kind: if b.code.starts_with("FLOW_") && b.code.ends_with("TRUNCATED") {
                FrontierKind::Budget
            } else {
                FrontierKind::Semantic
            },
            origin_ids: vec![b.boundary_id.clone()],
            affected_criteria: all_criteria.clone(),
            required_checks: b.required_checks.clone(),
        })
        .collect::<Vec<_>>();
    // No universal value, alias, framework, exception or effect model is supplied.
    frontier.push(Frontier {
        code: "DEPENDENCE_MODEL_UNAVAILABLE".into(),
        kind: FrontierKind::Semantic,
        origin_ids: vec![flow.root_node_id.clone()],
        affected_criteria: all_criteria.clone(),
        required_checks: vec!["QUALIFY_CRITERION_DEPENDENCE_AND_EFFECT_MODEL".into()],
    });
    if flow.status == FlowStatus::Truncated {
        frontier.push(Frontier {
            code: "PARENT_TRAVERSAL_TRUNCATED".into(),
            kind: FrontierKind::Budget,
            origin_ids: vec![flow.root_node_id.clone()],
            affected_criteria: all_criteria.clone(),
            required_checks: vec!["EXPAND_PARENT_FLOW_OR_NARROW_CRITERION".into()],
        });
    }
    let mut criteria = Vec::new();
    for criterion in &request.criteria {
        let mut matches = BTreeSet::new();
        let mut facts = BTreeSet::new();
        match &criterion.selector {
            Selector::Callable {
                member_alias,
                symbol_identity,
            } => {
                for node in flow.nodes.iter().filter(|n| {
                    n.node_kind == FlowNodeKind::Callable
                        && n.member_alias == *member_alias
                        && n.symbol_identity == *symbol_identity
                }) {
                    matches.insert(node.node_id.clone());
                    facts.extend(node.support_refs.iter().map(|s| s.fact_id.clone()));
                }
            }
            Selector::RelationFact { fact_id } => {
                for edge in flow
                    .edges
                    .iter()
                    .filter(|e| e.support_refs.iter().any(|s| s.fact_id == *fact_id))
                {
                    matches.insert(edge.edge_id.clone());
                    facts.extend(edge.support_refs.iter().map(|s| s.fact_id.clone()));
                }
                for boundary in flow
                    .boundaries
                    .iter()
                    .filter(|b| b.support_refs.iter().any(|s| s.fact_id == *fact_id))
                {
                    matches.insert(boundary.boundary_id.clone());
                    facts.extend(boundary.support_refs.iter().map(|s| s.fact_id.clone()));
                }
            }
            Selector::CfgNode { region_id, node_id } => {
                for region in flow.control_flow_regions.iter().filter(|r| {
                    r.region_id == *region_id && r.graph.nodes.iter().any(|n| n.node_id == *node_id)
                }) {
                    matches.insert(format!("{}#{}", region.region_id, node_id));
                }
            }
        }
        let discovery = match matches.len() {
            0 => Discovery::NotFoundUnknown,
            // One fact can support an edge and several obligations. These are
            // representations of the selected fact, not overload candidates.
            _ if matches!(&criterion.selector, Selector::RelationFact { .. }) => {
                Discovery::FoundInRetainedFlow
            }
            1 => Discovery::FoundInRetainedFlow,
            _ => Discovery::AmbiguousUnknown,
        };
        if discovery != Discovery::FoundInRetainedFlow {
            frontier.push(Frontier {
                code: if matches.is_empty() {
                    "CRITERION_NOT_FOUND"
                } else {
                    "CRITERION_AMBIGUOUS"
                }
                .into(),
                kind: FrontierKind::Discovery,
                origin_ids: matches.iter().cloned().collect(),
                affected_criteria: vec![criterion.id.clone()],
                required_checks: vec!["RESOLVE_EXACT_CRITERION_OR_REQUEST_EVIDENCE".into()],
            });
        }
        criteria.push(CriterionResult {
            criterion_id: criterion.id.clone(),
            discovery,
            matched_ids: matches.into_iter().collect(),
            support_fact_ids: facts.into_iter().collect(),
            semantic_closure: SemanticClosure::Unknown,
        });
    }
    let all_methods = flow
        .nodes
        .iter()
        .filter(|n| n.node_kind == FlowNodeKind::Callable)
        .map(|n| n.node_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let method_set = all_methods.iter().collect::<BTreeSet<_>>();
    let mut prioritized = Vec::new();
    for criterion in &criteria {
        for id in &criterion.matched_ids {
            if method_set.contains(id) {
                prioritized.push(id.clone());
            }
            for edge in flow.edges.iter().filter(|e| &e.edge_id == id) {
                prioritized.push(edge.source_node_id.clone());
            }
            for region in flow
                .control_flow_regions
                .iter()
                .filter(|r| id.starts_with(&format!("{}#", r.region_id)))
            {
                prioritized.push(region.owner_node_id.clone());
            }
        }
    }
    prioritized.push(flow.root_node_id.clone());
    prioritized.extend(all_methods);
    let mut seen = BTreeSet::new();
    let methods = prioritized
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect::<Vec<_>>();
    let split = methods.len().min(request.max_methods);
    let omitted_method_ids = methods[split..].to_vec();
    if !omitted_method_ids.is_empty() {
        frontier.push(Frontier {
            code: "PRESENTATION_METHOD_BUDGET".into(),
            kind: FrontierKind::Budget,
            origin_ids: omitted_method_ids.clone(),
            affected_criteria: all_criteria,
            required_checks: vec!["INCREASE_PACKET_METHOD_BUDGET_OR_NARROW_ROOT".into()],
        });
    }
    frontier.sort_by(|a, b| {
        (&a.code, &a.origin_ids, &a.affected_criteria).cmp(&(
            &b.code,
            &b.origin_ids,
            &b.affected_criteria,
        ))
    });
    let local_order_observations = request
        .local_order_queries
        .iter()
        .map(|query| {
            let graph = flow
                .control_flow_regions
                .iter()
                .find(|r| r.region_id == query.region_id)
                .map(|r| &r.graph);
            crate::thread_question_control::observe(graph, query)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut shared_sources = BTreeMap::new();
    let mut callable_envelope_summaries = Vec::new();
    for node_id in &methods[..split] {
        let node = flow
            .nodes
            .iter()
            .find(|n| &n.node_id == node_id)
            .ok_or_else(|| invalid("question method is absent"))?;
        let mut sources = BTreeMap::new();
        for source in node.support_refs.iter().filter_map(|s| s.source.as_ref()) {
            let reference = &source.content_ref;
            if shared_sources
                .insert(reference.digest.clone(), reference.clone())
                .is_some_and(|previous| previous != *reference)
            {
                return Err(invalid(
                    "question source identity has conflicting references",
                ));
            }
            sources.insert(reference.digest.clone(), reference.clone());
        }
        let outgoing = flow
            .edges
            .iter()
            .filter(|e| &e.source_node_id == node_id)
            .collect::<Vec<_>>();
        callable_envelope_summaries.push(CallableEnvelopeSummary {
            owner_node_id: node_id.clone(),
            outgoing_relation_ids: outgoing
                .iter()
                .map(|e| e.edge_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            boundary_target_ids: outgoing
                .iter()
                .filter(|e| {
                    flow.nodes.iter().any(|n| {
                        n.node_id == e.target_node_id && n.node_kind == FlowNodeKind::Boundary
                    })
                })
                .map(|e| e.target_node_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            control_region_ids: flow
                .control_flow_regions
                .iter()
                .filter(|r| &r.owner_node_id == node_id)
                .map(|r| r.region_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            source_objects: sources.into_values().collect(),
            semantic_closure: SemanticClosure::Unknown,
        });
    }
    let unique_source_bytes = shared_sources
        .values()
        .try_fold(0_u64, |total, source| total.checked_add(source.size))
        .ok_or_else(|| budget("question source byte estimate overflows"))?;
    let interpretation_plan = InterpretationPlan {
        workers: 1,
        reason: "CONSERVATIVE_ENVELOPE_HAS_NO_QUALIFIED_INDEPENDENT_CUTS".into(),
        shared_source_objects: shared_sources.into_values().collect(),
        unique_source_bytes,
        duplicated_source_bytes: 0,
        max_model_calls: 2,
        max_expansion_rounds: 1,
        actual_model_calls: 0,
    };
    let mut packet = QuestionPacket {
        schema: RESULT_SCHEMA.into(),
        packet_id: String::new(),
        policy: POLICY.into(),
        request,
        parent_flow_ref: flow_ref.clone(),
        extraction: extraction.clone(),
        traversal: flow.status,
        presentation_complete: omitted_method_ids.is_empty(),
        presentation_method_ids: methods[..split].to_vec(),
        omitted_method_ids,
        control_region_ids: flow
            .control_flow_regions
            .iter()
            .map(|r| r.region_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        relation_ids: flow
            .edges
            .iter()
            .map(|e| e.edge_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        criteria,
        frontier,
        local_order_observations,
        callable_envelope_summaries,
        interpretation_plan,
    };
    packet.packet_id = format!(
        "thread-question:{}",
        canonical::hash(&packet).map_err(internal)?
    );
    if canonical::bytes(&packet)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_RESULT_BYTES
    {
        return Err(budget(
            "question result exceeds 64 KiB; narrow the parent flow",
        ));
    }
    Ok(packet)
}

pub fn verify_packet(
    flow: &FlowSlice,
    flow_ref: &CasObject,
    extraction: &CallableCompleteness,
    packet: &QuestionPacket,
) -> Result<(), ClewError> {
    if build_verified(flow, flow_ref, extraction, packet.request.clone())? != *packet {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "question packet differs from deterministic reconstruction",
        ));
    }
    Ok(())
}

fn bounded_text(value: &str) -> Result<(), ClewError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(invalid(
            "question text is empty, unsafe or exceeds 4096 bytes",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}
fn budget(message: &str) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}
fn internal(error: anyhow::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
