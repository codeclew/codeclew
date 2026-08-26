//! Validation and authority calculus for untrusted agent-authored explanations.

use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use crate::thread_callables::RelationshipAuthority;
use crate::thread_flow::{FlowBoundary, FlowEdge, FlowNode, FlowSlice, FlowStatus};
use crate::thread_flow_cfg::{LocalCfgEdgeKind, LocalCfgNodeRole};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use unicode_normalization::UnicodeNormalization;

pub const CLAIM_INPUT_SCHEMA: &str = "codeclew-explanation-claim-input/0.1";
pub const EXPLANATION_BUNDLE_SCHEMA: &str = "codeclew-explanation-bundle/0.1";
pub const EXPLANATION_PROJECTION_SCHEMA: &str = "codeclew-explanation-projection/0.1";
pub const MAX_CLAIM_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_CLAIMS: usize = 1_024;
pub const MAX_CLAIM_TEXT_BYTES: usize = 8 * 1024;
pub const MAX_EXPLANATION_STDOUT_BYTES: usize = 64 * 1024;
pub const MAX_EXPLANATION_BUNDLE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClaimAuthority {
    Unknown,
    AgentInferred,
    Declared,
    StaticDerived,
    CompilerProven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExplanationPredicateKind {
    CallExists,
    Constructs,
    BranchExists,
    OrderedBefore,
    ReachableStaticPath,
    NarrativeSummary,
    ComponentHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum ClaimPredicate {
    CallExists {
        subject: String,
        object: String,
    },
    Constructs {
        subject: String,
        object: String,
    },
    BranchExists {
        subject: String,
        region_id: String,
        branch_kind: LocalCfgEdgeKind,
    },
    OrderedBefore {
        subject: String,
        region_id: String,
        before_node_id: u64,
        after_node_id: u64,
    },
    ReachableStaticPath {
        subject: String,
        object: String,
    },
    NarrativeSummary {
        subject: String,
    },
    ComponentHandoff {
        subject: String,
        object: String,
    },
}

impl ClaimPredicate {
    pub fn kind(&self) -> ExplanationPredicateKind {
        match self {
            Self::CallExists { .. } => ExplanationPredicateKind::CallExists,
            Self::Constructs { .. } => ExplanationPredicateKind::Constructs,
            Self::BranchExists { .. } => ExplanationPredicateKind::BranchExists,
            Self::OrderedBefore { .. } => ExplanationPredicateKind::OrderedBefore,
            Self::ReachableStaticPath { .. } => ExplanationPredicateKind::ReachableStaticPath,
            Self::NarrativeSummary { .. } => ExplanationPredicateKind::NarrativeSummary,
            Self::ComponentHandoff { .. } => ExplanationPredicateKind::ComponentHandoff,
        }
    }

    fn subjects(&self) -> Vec<&str> {
        match self {
            Self::CallExists { subject, object }
            | Self::Constructs { subject, object }
            | Self::ReachableStaticPath { subject, object }
            | Self::ComponentHandoff { subject, object } => vec![subject, object],
            Self::BranchExists { subject, .. }
            | Self::OrderedBefore { subject, .. }
            | Self::NarrativeSummary { subject } => vec![subject],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimInput {
    pub local_id: String,
    pub locale: String,
    pub text: String,
    pub predicate: ClaimPredicate,
    pub support_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundary_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimInputDocument {
    pub schema: String,
    pub flow_id: String,
    pub claims: Vec<ClaimInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationClaim {
    pub claim_id: String,
    pub local_id: String,
    pub locale: String,
    pub text: String,
    pub predicate: ClaimPredicate,
    pub authority: ClaimAuthority,
    pub support_refs: Vec<String>,
    pub boundary_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationBundle {
    pub schema: String,
    pub explanation_id: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub fact_set_id: String,
    pub fact_set_authority_digest: String,
    pub flow_id: String,
    pub flow_slice_ref: CasObject,
    pub claims_input_digest: String,
    pub claims: Vec<ExplanationClaim>,
    pub boundaries: Vec<FlowBoundary>,
    pub verification_obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationProjection {
    pub schema: String,
    pub explanation_id: String,
    pub flow_id: String,
    pub claim_count: usize,
    pub authority_counts: BTreeMap<ClaimAuthority, usize>,
    pub boundary_count: usize,
    pub verification_obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExplanation {
    pub bundle: ExplanationBundle,
    pub bundle_bytes: Vec<u8>,
    pub bundle_ref: CasObject,
    pub projection: ExplanationProjection,
}

pub fn parse_claim_document(bytes: &[u8]) -> Result<ClaimInputDocument, ClewError> {
    if bytes.is_empty() || bytes.len() > MAX_CLAIM_DOCUMENT_BYTES {
        return Err(budget("claim input is empty or exceeds 1 MiB"));
    }
    let document: ClaimInputDocument = serde_json::from_slice(bytes)
        .map_err(|_| invalid("claim input is not a closed JSON document"))?;
    if canonical::bytes(&document).map_err(internal)? != bytes {
        return Err(invalid("claim input is not canonical JSON"));
    }
    Ok(document)
}

pub fn build(
    flow: &FlowSlice,
    flow_slice_ref: &CasObject,
    mut document: ClaimInputDocument,
) -> Result<PreparedExplanation, ClewError> {
    let expected_flow_ref = CasObject::for_bytes(
        crate::thread_flow::FLOW_SLICE_SCHEMA,
        &canonical::bytes(flow).map_err(internal)?,
    )?;
    if &expected_flow_ref != flow_slice_ref {
        return Err(invalid("explanation flow differs from its CAS authority"));
    }
    validate_document(flow, &document)?;
    for claim in &mut document.claims {
        claim.boundary_refs.sort();
        if !matches!(claim.predicate, ClaimPredicate::ReachableStaticPath { .. }) {
            claim.support_refs.sort();
        }
    }
    document
        .claims
        .sort_by(|left, right| left.local_id.cmp(&right.local_id));
    let nodes = flow
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let edges = flow
        .edges
        .iter()
        .map(|edge| (edge.edge_id.as_str(), edge))
        .collect::<BTreeMap<_, _>>();
    let regions = flow
        .control_flow_regions
        .iter()
        .map(|region| (region.region_id.as_str(), region))
        .collect::<BTreeMap<_, _>>();
    let boundaries = flow
        .boundaries
        .iter()
        .map(|boundary| (boundary.boundary_id.as_str(), boundary))
        .collect::<BTreeMap<_, _>>();
    let claims_input_digest = canonical::hash(&document).map_err(internal)?;
    let mut claims = Vec::with_capacity(document.claims.len());
    for input in document.claims {
        let authority = validate_claim(flow, &input, &nodes, &edges, &regions, &boundaries)?;
        let claim_id = format!(
            "explanation-claim:{}",
            canonical::hash(&json!({
                "flowId": flow.flow_id,
                "localId": input.local_id,
                "locale": input.locale,
                "text": input.text,
                "predicate": input.predicate,
                "supportRefs": input.support_refs,
                "boundaryRefs": input.boundary_refs,
            }))
            .map_err(internal)?
        );
        claims.push(ExplanationClaim {
            claim_id,
            local_id: input.local_id,
            locale: input.locale,
            text: input.text,
            predicate: input.predicate,
            authority,
            support_refs: input.support_refs,
            boundary_refs: input.boundary_refs,
        });
    }
    claims.sort_by(|left, right| left.claim_id.cmp(&right.claim_id));
    let mut bundle = ExplanationBundle {
        schema: EXPLANATION_BUNDLE_SCHEMA.into(),
        explanation_id: String::new(),
        thread_id: flow.request.thread_id.clone(),
        thread_authority_digest: flow.request.thread_authority_digest.clone(),
        fact_set_id: flow.request.fact_set_id.clone(),
        fact_set_authority_digest: flow.request.fact_set_authority_digest.clone(),
        flow_id: flow.flow_id.clone(),
        flow_slice_ref: flow_slice_ref.clone(),
        claims_input_digest,
        claims,
        boundaries: flow.boundaries.clone(),
        verification_obligations: flow.verification_obligations.clone(),
    };
    bundle.explanation_id = format!(
        "thread-explanation:{}",
        canonical::hash(&bundle).map_err(internal)?
    );
    let bundle_bytes = canonical::bytes(&bundle).map_err(internal)?;
    if bundle_bytes.len() > MAX_EXPLANATION_BUNDLE_BYTES {
        return Err(budget("explanation bundle exceeds 16 MiB"));
    }
    let bundle_ref = CasObject::for_bytes(EXPLANATION_BUNDLE_SCHEMA, &bundle_bytes)?;
    let projection = project(&bundle);
    Ok(PreparedExplanation {
        bundle,
        bundle_bytes,
        bundle_ref,
        projection,
    })
}

pub fn verify_prepared(flow: &FlowSlice, prepared: &PreparedExplanation) -> Result<(), ClewError> {
    if canonical::bytes(&prepared.bundle).map_err(internal)? != prepared.bundle_bytes
        || CasObject::for_bytes(EXPLANATION_BUNDLE_SCHEMA, &prepared.bundle_bytes)?
            != prepared.bundle_ref
        || prepared.bundle.flow_id != flow.flow_id
        || prepared.projection != project(&prepared.bundle)
    {
        return Err(corrupt("prepared explanation is internally inconsistent"));
    }
    let document = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: prepared.bundle.flow_id.clone(),
        claims: prepared
            .bundle
            .claims
            .iter()
            .map(|claim| ClaimInput {
                local_id: claim.local_id.clone(),
                locale: claim.locale.clone(),
                text: claim.text.clone(),
                predicate: claim.predicate.clone(),
                support_refs: claim.support_refs.clone(),
                boundary_refs: claim.boundary_refs.clone(),
            })
            .collect(),
    };
    if build(flow, &prepared.bundle.flow_slice_ref, document)? != *prepared {
        return Err(corrupt(
            "prepared explanation differs from deterministic authority calculus",
        ));
    }
    Ok(())
}

pub fn project(bundle: &ExplanationBundle) -> ExplanationProjection {
    let mut authority_counts = BTreeMap::new();
    for claim in &bundle.claims {
        *authority_counts.entry(claim.authority).or_insert(0) += 1;
    }
    ExplanationProjection {
        schema: EXPLANATION_PROJECTION_SCHEMA.into(),
        explanation_id: bundle.explanation_id.clone(),
        flow_id: bundle.flow_id.clone(),
        claim_count: bundle.claims.len(),
        authority_counts,
        boundary_count: bundle.boundaries.len(),
        verification_obligations: bundle.verification_obligations.clone(),
    }
}

fn validate_document(flow: &FlowSlice, document: &ClaimInputDocument) -> Result<(), ClewError> {
    if document.schema != CLAIM_INPUT_SCHEMA || document.flow_id != flow.flow_id {
        return Err(invalid("claim input is bound to another flow"));
    }
    if document.claims.is_empty() || document.claims.len() > MAX_CLAIMS {
        return Err(invalid(
            "claim input must contain between one and 1,024 claims",
        ));
    }
    let mut local_ids = BTreeSet::new();
    for claim in &document.claims {
        if !safe_local_id(&claim.local_id) || !local_ids.insert(claim.local_id.as_str()) {
            return Err(invalid("claim local IDs are unsafe or duplicated"));
        }
        if !matches!(claim.locale.as_str(), "en" | "ru") {
            return Err(invalid("claim locale is outside the closed v1 set"));
        }
        validate_text(&claim.text)?;
        for value in claim.predicate.subjects() {
            validate_identity_text(value)?;
        }
        unique_refs(&claim.support_refs, "claim support refs")?;
        unique_refs(&claim.boundary_refs, "claim boundary refs")?;
    }
    Ok(())
}

fn validate_claim<'a>(
    flow: &FlowSlice,
    input: &ClaimInput,
    nodes: &BTreeMap<&'a str, &'a FlowNode>,
    edges: &BTreeMap<&'a str, &'a FlowEdge>,
    regions: &BTreeMap<&'a str, &'a crate::thread_flow::FlowControlRegion>,
    boundaries: &BTreeMap<&'a str, &'a FlowBoundary>,
) -> Result<ClaimAuthority, ClewError> {
    for reference in &input.support_refs {
        if !nodes.contains_key(reference.as_str())
            && !edges.contains_key(reference.as_str())
            && !regions.contains_key(reference.as_str())
        {
            return Err(invalid("claim has a support ref outside its parent flow"));
        }
    }
    for reference in &input.boundary_refs {
        if !boundaries.contains_key(reference.as_str()) {
            return Err(invalid("claim has a boundary ref outside its parent flow"));
        }
    }
    let mut authority = match &input.predicate {
        ClaimPredicate::CallExists { subject, object } => {
            validate_relation_claim(input, nodes, edges, subject, object, "CALLS")?
        }
        ClaimPredicate::Constructs { subject, object } => {
            validate_relation_claim(input, nodes, edges, subject, object, "CONSTRUCTS")?
        }
        ClaimPredicate::BranchExists {
            subject,
            region_id,
            branch_kind,
        } => {
            require_exact_support(input, region_id)?;
            let region = regions
                .get(region_id.as_str())
                .ok_or_else(|| invalid("branch claim names an unknown CFG region"))?;
            let owner = nodes
                .get(region.owner_node_id.as_str())
                .ok_or_else(|| corrupt("CFG region has no owner flow node"))?;
            if owner.symbol_identity != *subject
                || !region
                    .graph
                    .edges
                    .iter()
                    .any(|edge| edge.kind == *branch_kind)
            {
                return Err(invalid("branch claim contradicts its CFG region"));
            }
            ClaimAuthority::CompilerProven
        }
        ClaimPredicate::OrderedBefore {
            subject,
            region_id,
            before_node_id,
            after_node_id,
        } => {
            require_exact_support(input, region_id)?;
            let region = regions
                .get(region_id.as_str())
                .ok_or_else(|| invalid("order claim names an unknown CFG region"))?;
            let owner = nodes
                .get(region.owner_node_id.as_str())
                .ok_or_else(|| corrupt("CFG region has no owner flow node"))?;
            if owner.symbol_identity != *subject
                || before_node_id == after_node_id
                || !cfg_reaches(&region.graph, *before_node_id, *after_node_id)
                || region.graph.nodes.iter().any(|node| {
                    (node.node_id == *before_node_id || node.node_id == *after_node_id)
                        && node.role == LocalCfgNodeRole::Dead
                })
            {
                return Err(invalid("order claim lacks CFG reachability proof"));
            }
            ClaimAuthority::StaticDerived
        }
        ClaimPredicate::ReachableStaticPath { subject, object } => {
            if flow.status == FlowStatus::Truncated || input.support_refs.is_empty() {
                return Err(invalid(
                    "static path claim is empty or uses a truncated flow",
                ));
            }
            validate_static_path(input, nodes, edges, subject, object)?;
            ClaimAuthority::StaticDerived
        }
        ClaimPredicate::NarrativeSummary { subject } => {
            if input.support_refs.is_empty() && input.boundary_refs.is_empty() {
                return Err(invalid("narrative claim has no support or boundary ref"));
            }
            if !flow
                .nodes
                .iter()
                .any(|node| node.symbol_identity == *subject)
                && input.boundary_refs.is_empty()
            {
                return Err(invalid("narrative subject is outside its parent flow"));
            }
            if input.support_refs.is_empty() {
                ClaimAuthority::Unknown
            } else {
                ClaimAuthority::AgentInferred
            }
        }
        ClaimPredicate::ComponentHandoff { .. } => {
            return Err(invalid(
                "COMPONENT_HANDOFF is unavailable before pair-flow T04",
            ));
        }
    };
    let relevant = relevant_boundaries(flow, input, nodes, edges);
    let supplied = input
        .boundary_refs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if !relevant.is_subset(&supplied) {
        return Err(invalid("claim omits a relevant flow boundary"));
    }
    if !relevant.is_empty() {
        authority = ClaimAuthority::Unknown;
    }
    Ok(authority)
}

fn validate_relation_claim(
    input: &ClaimInput,
    nodes: &BTreeMap<&str, &FlowNode>,
    edges: &BTreeMap<&str, &FlowEdge>,
    subject: &str,
    object: &str,
    expected_kind: &str,
) -> Result<ClaimAuthority, ClewError> {
    if input.support_refs.len() != 1 {
        return Err(invalid("relation claim requires exactly one flow edge"));
    }
    let edge = edges
        .get(input.support_refs[0].as_str())
        .ok_or_else(|| invalid("relation claim support is not a flow edge"))?;
    let source = nodes
        .get(edge.source_node_id.as_str())
        .ok_or_else(|| corrupt("flow edge has no source node"))?;
    let target = nodes
        .get(edge.target_node_id.as_str())
        .ok_or_else(|| corrupt("flow edge has no target node"))?;
    if edge.relation_kind != expected_kind
        || source.symbol_identity != subject
        || target.symbol_identity != object
    {
        return Err(invalid("relation claim contradicts its support edge"));
    }
    Ok(match edge.relationship_authority {
        RelationshipAuthority::VerifiedSameSnapshotCompilationDependency => {
            ClaimAuthority::CompilerProven
        }
        RelationshipAuthority::DeclaredTopology => ClaimAuthority::Declared,
        RelationshipAuthority::Unbound => ClaimAuthority::Unknown,
    })
}

fn validate_static_path(
    input: &ClaimInput,
    nodes: &BTreeMap<&str, &FlowNode>,
    edges: &BTreeMap<&str, &FlowEdge>,
    subject: &str,
    object: &str,
) -> Result<(), ClewError> {
    let path = input
        .support_refs
        .iter()
        .map(|reference| {
            edges
                .get(reference.as_str())
                .copied()
                .ok_or_else(|| invalid("static path support contains a non-edge"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if path
        .windows(2)
        .any(|pair| pair[0].target_node_id != pair[1].source_node_id)
    {
        return Err(invalid(
            "static path support is not a continuous edge chain",
        ));
    }
    let first = nodes
        .get(path[0].source_node_id.as_str())
        .ok_or_else(|| corrupt("static path has no source node"))?;
    let last = nodes
        .get(path.last().expect("non-empty path").target_node_id.as_str())
        .ok_or_else(|| corrupt("static path has no target node"))?;
    if first.symbol_identity != subject || last.symbol_identity != object {
        return Err(invalid("static path endpoints contradict its predicate"));
    }
    Ok(())
}

fn relevant_boundaries<'a>(
    flow: &'a FlowSlice,
    input: &ClaimInput,
    nodes: &BTreeMap<&str, &FlowNode>,
    edges: &BTreeMap<&str, &FlowEdge>,
) -> BTreeSet<&'a str> {
    let subjects = input
        .predicate
        .subjects()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut fact_ids = BTreeSet::new();
    for reference in &input.support_refs {
        if let Some(node) = nodes.get(reference.as_str()) {
            fact_ids.extend(
                node.support_refs
                    .iter()
                    .map(|support| support.fact_id.as_str()),
            );
        }
        if let Some(edge) = edges.get(reference.as_str()) {
            fact_ids.extend(
                edge.support_refs
                    .iter()
                    .map(|support| support.fact_id.as_str()),
            );
        }
    }
    flow.boundaries
        .iter()
        .filter(|boundary| {
            subjects.contains(boundary.subject.as_str())
                || boundary
                    .support_refs
                    .iter()
                    .any(|support| fact_ids.contains(support.fact_id.as_str()))
        })
        .map(|boundary| boundary.boundary_id.as_str())
        .collect()
}

fn cfg_reaches(graph: &crate::thread_flow_cfg::LocalCfgPayload, from: u64, to: u64) -> bool {
    let known = graph
        .nodes
        .iter()
        .map(|node| node.node_id)
        .collect::<BTreeSet<_>>();
    if !known.contains(&from) || !known.contains(&to) {
        return false;
    }
    let mut queue = VecDeque::from([from]);
    let mut visited = BTreeSet::from([from]);
    while let Some(node) = queue.pop_front() {
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.source_node_id == node)
        {
            if edge.target_node_id == to {
                return true;
            }
            if visited.insert(edge.target_node_id) {
                queue.push_back(edge.target_node_id);
            }
        }
    }
    false
}

fn require_exact_support(input: &ClaimInput, expected: &str) -> Result<(), ClewError> {
    if input.support_refs.len() != 1 || input.support_refs[0] != expected {
        return Err(invalid("claim support does not match its predicate region"));
    }
    Ok(())
}

fn safe_local_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_text(value: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > MAX_CLAIM_TEXT_BYTES
        || value.chars().any(|character| character == '\0')
        || value.nfc().collect::<String>() != value
    {
        return Err(invalid(
            "claim text is empty, oversized, non-NFC, or unsafe",
        ));
    }
    Ok(())
}

fn validate_identity_text(value: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > 4_096
        || value.starts_with('-')
        || value.contains('\0')
        || value.contains("../")
        || value.contains("/../")
        || value.contains("&&")
        || value.contains("||")
        || value.contains("$(")
        || value.contains('`')
        || value.nfc().collect::<String>() != value
    {
        return Err(invalid("claim predicate identity is unsafe"));
    }
    Ok(())
}

fn unique_refs(values: &[String], label: &str) -> Result<(), ClewError> {
    let mut observed = BTreeSet::new();
    for value in values {
        validate_identity_text(value)?;
        if !observed.insert(value.as_str()) {
            return Err(invalid(format!("{label} are not unique")));
        }
    }
    Ok(())
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
