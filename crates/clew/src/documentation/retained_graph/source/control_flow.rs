//! Source-bound projection of retained Kotlin compiler local control flow.
use super::super::model::{
    CallableProjection, CompilerControlFlowNode, CompilerControlFlowOwnerKey,
    CompilerControlFlowProjection,
};
use super::{Context, gap};
use crate::documentation::local_cfg::{
    LOCAL_CFG_EVIDENCE_SCHEMA, LocalCfgBoundaryEvidence, LocalCfgEvidence,
};
use crate::documentation::model::Observation;
use crate::documentation::{digest, invalid};
use crate::error::ClewError;

fn graph_refers_to_owner(observation: &Observation, owner: &str, scope: &str) -> bool {
    observation.kind == "LOCAL_CFG"
        && observation.normalized["scope"] == scope
        && (observation.symbol == owner
            || observation.normalized["ownerSymbolIdentity"] == owner
            || observation.normalized["graph"]["ownerSymbolIdentity"] == owner)
}

fn boundary_matches_owner(observation: &Observation, owner: &str, scope: &str, file: &str) -> bool {
    observation.kind == "LOCAL_CFG_BOUNDARY"
        && observation.normalized["scope"] == scope
        && observation.normalized["ownerSymbolIdentity"]
            .as_str()
            .is_none_or(|boundary_owner| boundary_owner == owner)
        && observation.normalized["file"]
            .as_str()
            .is_none_or(|boundary_file| boundary_file == file)
}

fn unavailable(callable: &mut CallableProjection, boundary: Option<&str>) {
    let detail = boundary.map_or_else(
        || "No validated compiler local control-flow graph is available for this function in the retained Check.".to_owned(),
        |code| format!("The retained compiler control-flow boundary is {code}; no graph is projected."),
    );
    callable.gaps.push(gap(
        "KOTLIN_CONTROL_FLOW_UNAVAILABLE",
        detail,
        callable.citation_id.clone(),
    ));
}

/// Attach only one graph whose service, compiler scope, full owner identity and
/// source file all match the already-admitted Kotlin function.
pub(in crate::documentation) fn attach(
    context: &mut Context<'_>,
    owner: &Observation,
    scope: &str,
    callable: &mut CallableProjection,
) -> Result<(), ClewError> {
    let owner_source = match owner.source_ids.as_slice() {
        [source_id] => context
            .evidence
            .sources
            .get(source_id)
            .ok_or_else(|| invalid("Kotlin control-flow owner source is not retained"))?,
        _ => return Err(invalid("Kotlin control-flow owner source is ambiguous")),
    };

    let boundaries = context
        .evidence
        .observations
        .values()
        .filter(|candidate| {
            candidate.service == context.evidence.service
                && boundary_matches_owner(candidate, &owner.symbol, scope, &owner_source.file)
        })
        .collect::<Vec<_>>();
    if boundaries.len() > 1 {
        return Err(invalid(
            "Kotlin control-flow boundaries are ambiguous for the selected service, scope and owner",
        ));
    }
    if let Some(candidate) = boundaries.first().copied() {
        if candidate.digest != digest(&candidate.normalized)? {
            return Err(invalid("Kotlin local CFG boundary digest is inconsistent"));
        }
        let boundary: LocalCfgBoundaryEvidence =
            serde_json::from_value(candidate.normalized.clone())
                .map_err(|_| invalid("Kotlin local CFG boundary violates its typed contract"))?;
        boundary.validate(&boundary.evidence_binding)?;
        if candidate.symbol
            != boundary
                .owner_symbol_identity
                .as_deref()
                .unwrap_or_default()
            || boundary
                .owner_symbol_identity
                .as_deref()
                .is_some_and(|value| value != owner.symbol)
            || boundary.scope != scope
            || boundary
                .file
                .as_deref()
                .is_some_and(|value| value != owner_source.file)
        {
            return Err(invalid(
                "Kotlin local CFG boundary does not match the selected service, scope, owner or file",
            ));
        }
        context.retain(candidate);
        unavailable(callable, Some(&boundary.code));
        return Ok(());
    }

    let candidates = context
        .evidence
        .observations
        .values()
        .filter(|candidate| {
            candidate.service == context.evidence.service
                && graph_refers_to_owner(candidate, &owner.symbol, scope)
        })
        .collect::<Vec<_>>();
    if candidates.len() > 1 {
        return Err(invalid(
            "Kotlin control-flow evidence is ambiguous for the selected service, scope and owner",
        ));
    }
    let Some(candidate) = candidates.first().copied() else {
        unavailable(callable, None);
        return Ok(());
    };

    if candidate.digest != digest(&candidate.normalized)? || candidate.normalized["scope"] != scope
    {
        return Err(invalid(
            "Kotlin control-flow observation scope or digest is inconsistent",
        ));
    }
    if candidate.normalized["schema"] != LOCAL_CFG_EVIDENCE_SCHEMA {
        return Err(invalid(
            "Kotlin local CFG evidence has an unsupported schema",
        ));
    }

    let evidence: LocalCfgEvidence = serde_json::from_value(candidate.normalized.clone())
        .map_err(|_| invalid("Kotlin local CFG evidence violates its typed contract"))?;
    if candidate.kind != "LOCAL_CFG"
        || candidate.symbol != owner.symbol
        || evidence.scope != scope
        || evidence.owner_symbol_identity != owner.symbol
        || evidence.file != owner_source.file
        || candidate.normalized["graph"]["file"] != owner_source.file
        || candidate.normalized["graph"]["ownerSymbolIdentity"] != owner.symbol
        || candidate.source_ids.as_slice() != [evidence.source_site.source_id.as_str()]
    {
        return Err(invalid(
            "Kotlin local CFG is not bound to the exact selected function",
        ));
    }
    let source = context
        .evidence
        .sources
        .get(&evidence.source_site.source_id)
        .filter(|source| source.id == evidence.source_site.source_id)
        .ok_or_else(|| invalid("Kotlin local CFG source is not retained"))?;
    evidence.validate_source(
        source,
        &context.evidence.service,
        &context.evidence.revision,
    )?;

    // Validate every byte range and UTF-8 boundary before asking Context to
    // create citations; its legacy helper clamps inputs for Java callers.
    let mut spans = Vec::with_capacity(evidence.graph.nodes.len());
    let mut citations = evidence.node_citations.iter();
    for node in &evidence.graph.nodes {
        let span = if node.source.is_some() {
            let citation = citations
                .next()
                .ok_or_else(|| invalid("Kotlin local CFG source citation is missing"))?;
            let start = usize::try_from(citation.byte_start)
                .map_err(|_| invalid("Kotlin local CFG source start exceeds addressable bytes"))?;
            let end = usize::try_from(citation.byte_end)
                .map_err(|_| invalid("Kotlin local CFG source end exceeds addressable bytes"))?;
            if start >= end || source.text.get(start..end).is_none() {
                return Err(invalid(
                    "Kotlin local CFG source range is outside retained UTF-8 boundaries",
                ));
            }
            Some((start, end))
        } else {
            None
        };
        spans.push(span);
    }
    if citations.next().is_some() {
        return Err(invalid("Kotlin local CFG has an unbound source citation"));
    }

    let nodes = evidence
        .graph
        .nodes
        .iter()
        .zip(spans)
        .map(|(node, span)| {
            let citation_id = span.map(|(start, end)| context.citation(source, start, end));
            CompilerControlFlowNode {
                node_id: node.node_id,
                role: node.role,
                source: node.source.clone(),
                citation_id,
            }
        })
        .collect();
    context.retain(candidate);
    callable.control_flow = Some(CompilerControlFlowProjection {
        owner_key: CompilerControlFlowOwnerKey {
            service: context.evidence.service.clone(),
            scope: scope.into(),
            symbol: owner.symbol.clone(),
        },
        graph_observation_id: candidate.id.clone(),
        graph_id: evidence.graph.graph_id.clone(),
        graph_evidence_binding: evidence.graph_evidence_binding.clone(),
        descriptor_evidence_binding: evidence.descriptor_evidence_binding.clone(),
        provider: evidence.graph.provider.clone(),
        compiler_graph_name: evidence.graph.compiler_graph_name.clone(),
        nodes,
        edges: evidence.graph.edges.clone(),
    });
    Ok(())
}
