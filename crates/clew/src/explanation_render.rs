//! Deterministic semantic-zoom projections for one immutable explanation bundle.

use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::explanation::{
    ClaimAuthority, ExplanationBundle, ExplanationClaim, ExplanationPredicateKind,
    MAX_EXPLANATION_STDOUT_BYTES,
};
use crate::thread_flow::{
    FlowBoundary, FlowControlRegion, FlowEdge, FlowNode, FlowSlice, FlowSupportRef,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const RENDER_SCHEMA: &str = "codeclew-explanation-render/0.1";
pub const MARKDOWN_RESULT_SCHEMA: &str = "codeclew-explanation-markdown-result/0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DetailLevel {
    Summary,
    Scenario,
    Technical,
    Evidence,
    Compiler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RenderFormat {
    Json,
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderClaim {
    pub claim_id: String,
    pub authority: ClaimAuthority,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<crate::explanation::ClaimPredicate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub support_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundary_refs: Vec<String>,
    pub expand_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationRender {
    pub schema: String,
    pub explanation_id: String,
    pub flow_id: String,
    pub detail: DetailLevel,
    pub semantic_digest: String,
    pub claims: Vec<RenderClaim>,
    pub boundaries: Vec<FlowBoundary>,
    pub verification_obligations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<FlowNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<FlowEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_flow_regions: Vec<FlowControlRegion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compiler_support: Vec<FlowSupportRef>,
    pub truncated: bool,
}

pub fn render(
    bundle: &ExplanationBundle,
    flow: &FlowSlice,
    detail: DetailLevel,
    format: RenderFormat,
) -> Result<Value, ClewError> {
    validate_binding(bundle, flow)?;
    let semantic_digest = canonical::hash(bundle).map_err(internal)?;
    let mut projection = ExplanationRender {
        schema: RENDER_SCHEMA.into(),
        explanation_id: bundle.explanation_id.clone(),
        flow_id: bundle.flow_id.clone(),
        detail,
        semantic_digest,
        claims: Vec::new(),
        boundaries: bundle.boundaries.clone(),
        verification_obligations: bundle.verification_obligations.clone(),
        nodes: Vec::new(),
        edges: Vec::new(),
        control_flow_regions: Vec::new(),
        compiler_support: Vec::new(),
        truncated: false,
    };
    projection
        .boundaries
        .sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));
    projection.verification_obligations.sort();
    projection.verification_obligations.dedup();
    ensure_safe_paths(flow)?;
    ensure_fits(&projection, format)?;

    let claim_candidates = selected_claims(bundle, detail)
        .into_iter()
        .map(|claim| project_claim(claim, detail))
        .collect::<Vec<_>>();
    let node_candidates = matches!(
        detail,
        DetailLevel::Technical | DetailLevel::Evidence | DetailLevel::Compiler
    )
    .then(|| sorted(flow.nodes.clone(), |value| value.node_id.clone()))
    .unwrap_or_default();
    let edge_candidates = matches!(
        detail,
        DetailLevel::Technical | DetailLevel::Evidence | DetailLevel::Compiler
    )
    .then(|| sorted(flow.edges.clone(), |value| value.edge_id.clone()))
    .unwrap_or_default();
    let region_candidates = matches!(
        detail,
        DetailLevel::Scenario
            | DetailLevel::Technical
            | DetailLevel::Evidence
            | DetailLevel::Compiler
    )
    .then(|| {
        sorted(flow.control_flow_regions.clone(), |value| {
            value.region_id.clone()
        })
    })
    .unwrap_or_default();
    let support_candidates = if detail == DetailLevel::Compiler {
        compiler_support(flow)
    } else {
        Vec::new()
    };

    let mut positions = [0usize; 5];
    loop {
        let mut advanced = false;
        advanced |= try_add(
            &mut projection,
            format,
            &claim_candidates,
            &mut positions[0],
            |projection, value| projection.claims.push(value),
            |projection| {
                projection.claims.pop();
            },
        )?;
        advanced |= try_add(
            &mut projection,
            format,
            &node_candidates,
            &mut positions[1],
            |projection, value| projection.nodes.push(value),
            |projection| {
                projection.nodes.pop();
            },
        )?;
        advanced |= try_add(
            &mut projection,
            format,
            &edge_candidates,
            &mut positions[2],
            |projection, value| projection.edges.push(value),
            |projection| {
                projection.edges.pop();
            },
        )?;
        advanced |= try_add(
            &mut projection,
            format,
            &region_candidates,
            &mut positions[3],
            |projection, value| projection.control_flow_regions.push(value),
            |projection| {
                projection.control_flow_regions.pop();
            },
        )?;
        advanced |= try_add(
            &mut projection,
            format,
            &support_candidates,
            &mut positions[4],
            |projection, value| projection.compiler_support.push(value),
            |projection| {
                projection.compiler_support.pop();
            },
        )?;
        if !advanced {
            break;
        }
    }
    projection.truncated = positions[0] < claim_candidates.len()
        || positions[1] < node_candidates.len()
        || positions[2] < edge_candidates.len()
        || positions[3] < region_candidates.len()
        || positions[4] < support_candidates.len();
    encoded_value(&projection, format)
}

fn selected_claims(bundle: &ExplanationBundle, detail: DetailLevel) -> Vec<&ExplanationClaim> {
    let mut claims = bundle
        .claims
        .iter()
        .filter(|claim| match detail {
            DetailLevel::Summary => {
                claim.predicate.kind() == ExplanationPredicateKind::NarrativeSummary
                    || claim.authority == ClaimAuthority::Unknown
            }
            DetailLevel::Scenario => matches!(
                claim.predicate.kind(),
                ExplanationPredicateKind::NarrativeSummary
                    | ExplanationPredicateKind::BranchExists
                    | ExplanationPredicateKind::OrderedBefore
                    | ExplanationPredicateKind::ReachableStaticPath
            ),
            DetailLevel::Technical | DetailLevel::Evidence | DetailLevel::Compiler => true,
        })
        .collect::<Vec<_>>();
    if claims.is_empty() && !bundle.claims.is_empty() {
        claims.push(&bundle.claims[0]);
    }
    claims.sort_by(|left, right| left.claim_id.cmp(&right.claim_id));
    claims
}

fn project_claim(claim: &ExplanationClaim, detail: DetailLevel) -> RenderClaim {
    let show_predicate = !matches!(detail, DetailLevel::Summary);
    let show_support = matches!(
        detail,
        DetailLevel::Technical | DetailLevel::Evidence | DetailLevel::Compiler
    );
    let mut expand_refs = claim.support_refs.clone();
    expand_refs.extend(claim.boundary_refs.iter().cloned());
    expand_refs.sort();
    expand_refs.dedup();
    RenderClaim {
        claim_id: claim.claim_id.clone(),
        authority: claim.authority,
        text: claim.text.clone(),
        predicate: show_predicate.then(|| claim.predicate.clone()),
        support_refs: if show_support {
            claim.support_refs.clone()
        } else {
            Vec::new()
        },
        boundary_refs: claim.boundary_refs.clone(),
        expand_refs,
    }
}

fn compiler_support(flow: &FlowSlice) -> Vec<FlowSupportRef> {
    let mut support = BTreeMap::<String, FlowSupportRef>::new();
    for reference in flow
        .nodes
        .iter()
        .flat_map(|node| &node.support_refs)
        .chain(flow.edges.iter().flat_map(|edge| &edge.support_refs))
        .chain(
            flow.boundaries
                .iter()
                .flat_map(|boundary| &boundary.support_refs),
        )
    {
        support
            .entry(reference.fact_id.clone())
            .or_insert_with(|| reference.clone());
    }
    support.into_values().collect()
}

fn try_add<T: Clone>(
    projection: &mut ExplanationRender,
    format: RenderFormat,
    candidates: &[T],
    position: &mut usize,
    add: impl FnOnce(&mut ExplanationRender, T),
    remove: impl FnOnce(&mut ExplanationRender),
) -> Result<bool, ClewError> {
    let Some(candidate) = candidates.get(*position).cloned() else {
        return Ok(false);
    };
    add(projection, candidate);
    if encoded_len(projection, format)? <= MAX_EXPLANATION_STDOUT_BYTES {
        *position += 1;
        Ok(true)
    } else {
        remove(projection);
        Ok(false)
    }
}

fn ensure_fits(projection: &ExplanationRender, format: RenderFormat) -> Result<(), ClewError> {
    if encoded_len(projection, format)? > MAX_EXPLANATION_STDOUT_BYTES {
        return Err(budget(
            "critical explanation boundaries exceed the 64 KiB render budget",
        ));
    }
    Ok(())
}

fn encoded_len(projection: &ExplanationRender, format: RenderFormat) -> Result<usize, ClewError> {
    Ok(canonical::bytes(&encoded_value(projection, format)?)
        .map_err(internal)?
        .len()
        .saturating_add(1))
}

fn encoded_value(projection: &ExplanationRender, format: RenderFormat) -> Result<Value, ClewError> {
    match format {
        RenderFormat::Json => serde_json::to_value(projection).map_err(internal),
        RenderFormat::Markdown => {
            let content = markdown(projection);
            Ok(json!({
                "schema": MARKDOWN_RESULT_SCHEMA,
                "explanationId": projection.explanation_id,
                "flowId": projection.flow_id,
                "detail": projection.detail,
                "semanticDigest": projection.semantic_digest,
                "truncated": projection.truncated,
                "content": content,
            }))
        }
    }
}

fn markdown(projection: &ExplanationRender) -> String {
    let mut output = format!(
        "# Explanation {}\n\nDetail: `{:?}` · Flow: `{}` · Truncated: `{}`\n",
        escape(&projection.explanation_id),
        projection.detail,
        escape(&projection.flow_id),
        projection.truncated
    );
    if !projection.boundaries.is_empty() {
        output.push_str("\n## Boundaries\n");
        for boundary in &projection.boundaries {
            output.push_str(&format!(
                "\n- <a id=\"{}\"></a>**{}** — `{}` (subject `{}`)\n",
                anchor(&boundary.boundary_id),
                escape(&boundary.code),
                escape(&boundary.boundary_id),
                escape(&boundary.subject)
            ));
        }
    }
    if !projection.verification_obligations.is_empty() {
        output.push_str("\n## Verification obligations\n");
        for obligation in &projection.verification_obligations {
            output.push_str(&format!("\n- `{}`\n", escape(obligation)));
        }
    }
    if !projection.claims.is_empty() {
        output.push_str("\n## Claims\n");
        for claim in &projection.claims {
            output.push_str(&format!(
                "\n### {}\n\nAuthority: `{}`\n\n{}\n",
                escape(&claim.claim_id),
                authority_label(claim.authority),
                escape(&claim.text)
            ));
            if !claim.expand_refs.is_empty() {
                output.push_str("\nExpand: ");
                for (index, reference) in claim.expand_refs.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    output.push_str(&format!(
                        "[`{}`](#{})",
                        escape(reference),
                        anchor(reference)
                    ));
                }
                output.push('\n');
            }
        }
    }
    if !projection.nodes.is_empty() || !projection.edges.is_empty() {
        output.push_str("\n## Technical graph\n");
        for node in &projection.nodes {
            output.push_str(&format!(
                "\n- <a id=\"{}\"></a>Node `{}`: `{}`\n",
                anchor(&node.node_id),
                escape(&node.node_id),
                escape(&node.symbol_identity)
            ));
        }
        for edge in &projection.edges {
            output.push_str(&format!(
                "\n- <a id=\"{}\"></a>Edge `{}`: `{}` → `{}` (`{}`)\n",
                anchor(&edge.edge_id),
                escape(&edge.edge_id),
                escape(&edge.source_node_id),
                escape(&edge.target_node_id),
                escape(&edge.relation_kind)
            ));
        }
    }
    if !projection.control_flow_regions.is_empty() {
        output.push_str("\n## Control-flow regions\n");
        for region in &projection.control_flow_regions {
            output.push_str(&format!(
                "\n- <a id=\"{}\"></a>`{}`: {} nodes, {} edges\n",
                anchor(&region.region_id),
                escape(&region.region_id),
                region.graph.nodes.len(),
                region.graph.edges.len()
            ));
        }
    }
    if !projection.compiler_support.is_empty() {
        output.push_str("\n## Compiler evidence\n");
        for support in &projection.compiler_support {
            output.push_str(&format!(
                "\n- <a id=\"{}\"></a>`{}` — payload `{}`, shard `{}`\n",
                anchor(&support.fact_id),
                escape(&support.fact_id),
                escape(&support.input_payload_ref.digest),
                escape(&support.fact_shard_ref.digest)
            ));
        }
    }
    output
}

fn ensure_safe_paths(flow: &FlowSlice) -> Result<(), ClewError> {
    for support in flow
        .nodes
        .iter()
        .flat_map(|node| &node.support_refs)
        .chain(flow.edges.iter().flat_map(|edge| &edge.support_refs))
        .chain(
            flow.boundaries
                .iter()
                .flat_map(|boundary| &boundary.support_refs),
        )
    {
        if support.source.as_ref().is_some_and(|source| {
            source.path.starts_with('/')
                || source
                    .path
                    .split('/')
                    .any(|part| part.is_empty() || part == "..")
        }) {
            return Err(corrupt(
                "explanation source path is not repository-relative",
            ));
        }
    }
    Ok(())
}

fn validate_binding(bundle: &ExplanationBundle, flow: &FlowSlice) -> Result<(), ClewError> {
    if bundle.flow_id != flow.flow_id
        || bundle.thread_id != flow.request.thread_id
        || bundle.fact_set_id != flow.request.fact_set_id
        || bundle.boundaries != flow.boundaries
        || bundle.verification_obligations != flow.verification_obligations
    {
        return Err(corrupt(
            "explanation render inputs have different authority",
        ));
    }
    Ok(())
}

fn sorted<T>(mut values: Vec<T>, key: impl Fn(&T) -> String) -> Vec<T> {
    values.sort_by_key(key);
    values
}

fn escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if matches!(
                character,
                '\\' | '`' | '*' | '{' | '}' | '[' | ']' | '<' | '>' | '#' | '|' | '!'
            ) {
                vec!['\\', character]
            } else {
                vec![character]
            }
        })
        .collect()
}

fn authority_label(authority: ClaimAuthority) -> &'static str {
    match authority {
        ClaimAuthority::Unknown => "UNKNOWN",
        ClaimAuthority::AgentInferred => "AGENT_INFERRED",
        ClaimAuthority::Declared => "DECLARED",
        ClaimAuthority::StaticDerived => "STATIC_DERIVED",
        ClaimAuthority::CompilerProven => "COMPILER_PROVEN",
    }
}

fn anchor(value: &str) -> String {
    let digest = canonical::hash(&value).unwrap_or_else(|_| "sha256:invalid".into());
    format!("ref-{}", digest.trim_start_matches("sha256:"))
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
