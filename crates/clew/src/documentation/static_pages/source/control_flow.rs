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
use crate::thread_flow_cfg::LocalCfgNode;

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
pub(super) fn attach(
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

fn same_node(left: &CompilerControlFlowNode, right: &LocalCfgNode) -> bool {
    left.node_id == right.node_id && left.role == right.role && left.source == right.source
}

/// Publisher preflight rechecks the copied panel against the exact typed graph
/// and retained source bindings before any files are created.
pub(super) fn validate_callable(
    service: &str,
    revision: &str,
    observations: &std::collections::BTreeMap<String, Observation>,
    sources: &std::collections::BTreeMap<String, crate::documentation::model::Source>,
    citations: &std::collections::BTreeMap<String, super::super::model::Citation>,
    callable: &CallableProjection,
) -> Result<bool, ClewError> {
    let Some(panel) = &callable.control_flow else {
        return Ok(false);
    };
    if callable.authority != "COMPILER_DECLARATION"
        || panel.owner_key.service != service
        || panel.owner_key.symbol != callable.symbol
    {
        return Err(invalid(
            "native compiler control-flow panel owner differs from its selected callable",
        ));
    }
    let owner_observation = observations
        .get(&callable.declaration_id)
        .filter(|observation| observation.id == callable.declaration_id)
        .ok_or_else(|| invalid("native compiler control-flow callable declaration is missing"))?;
    if owner_observation.kind != "SYMBOL"
        || owner_observation.service != service
        || owner_observation.symbol != callable.symbol
        || owner_observation.normalized["schema"] != "declaration-descriptor/0.1"
        || owner_observation.normalized["scope"] != panel.owner_key.scope
        || owner_observation.normalized["symbolIdentity"] != panel.owner_key.symbol
        || owner_observation.digest != digest(&owner_observation.normalized)?
    {
        return Err(invalid(
            "native compiler control-flow owner scope differs from its admitted declaration",
        ));
    }
    let observation = observations
        .get(&panel.graph_observation_id)
        .filter(|observation| observation.id == panel.graph_observation_id)
        .ok_or_else(|| invalid("native compiler control-flow graph observation is missing"))?;
    if observation.kind != "LOCAL_CFG"
        || observation.service != service
        || observation.symbol != panel.owner_key.symbol
        || observation.digest != digest(&observation.normalized)?
        || observation.normalized["scope"] != panel.owner_key.scope
        || observation.normalized["ownerSymbolIdentity"] != panel.owner_key.symbol
        || observation.normalized["graph"]["graphId"] != panel.graph_id
        || observation.normalized["graphEvidenceBinding"] != panel.graph_evidence_binding
        || observation.normalized["descriptorEvidenceBinding"] != panel.descriptor_evidence_binding
    {
        return Err(invalid(
            "native compiler control-flow panel evidence identity is inconsistent",
        ));
    }
    let evidence: LocalCfgEvidence = serde_json::from_value(observation.normalized.clone())
        .map_err(|_| {
            invalid("native compiler control-flow evidence violates its typed contract")
        })?;
    let declaration_source_id = match owner_observation.source_ids.as_slice() {
        [source_id] => source_id,
        _ => return Err(invalid("native control-flow owner source is ambiguous")),
    };
    let declaration_source = sources
        .get(declaration_source_id)
        .filter(|source| source.id == *declaration_source_id)
        .ok_or_else(|| invalid("native control-flow owner source is not retained"))?;
    if evidence.graph.provider != panel.provider
        || evidence.graph.compiler_graph_name != panel.compiler_graph_name
        || evidence.graph.owner_symbol_identity != panel.owner_key.symbol
        || evidence.scope != panel.owner_key.scope
        || evidence.file != declaration_source.file
        || declaration_source.service != service
        || declaration_source.revision != revision
        || observation.source_ids.as_slice() != [evidence.source_site.source_id.as_str()]
    {
        return Err(invalid(
            "native compiler control-flow panel differs from its retained graph",
        ));
    }
    let source = sources
        .get(&evidence.source_site.source_id)
        .filter(|source| source.id == evidence.source_site.source_id)
        .ok_or_else(|| invalid("native compiler control-flow source is missing"))?;
    evidence.validate_source(source, service, revision)?;

    if panel.nodes.len() != evidence.graph.nodes.len()
        || panel.edges != evidence.graph.edges
        || panel
            .nodes
            .iter()
            .zip(&evidence.graph.nodes)
            .any(|(actual, expected)| !same_node(actual, expected))
    {
        return Err(invalid(
            "native compiler control-flow nodes or edges differ from the retained graph",
        ));
    }

    let mut node_citations = evidence.node_citations.iter();
    for (node, graph_node) in panel.nodes.iter().zip(&evidence.graph.nodes) {
        if graph_node.source.is_none() {
            if node.citation_id.is_some() {
                return Err(invalid(
                    "source-less compiler control-flow node cannot carry a citation",
                ));
            }
            continue;
        }
        let node_citation = node_citations
            .next()
            .ok_or_else(|| invalid("native compiler control-flow node citation is missing"))?;
        let start = usize::try_from(node_citation.byte_start)
            .map_err(|_| invalid("native compiler control-flow citation start is too large"))?;
        let end = usize::try_from(node_citation.byte_end)
            .map_err(|_| invalid("native compiler control-flow citation end is too large"))?;
        let exact = source
            .text
            .get(start..end)
            .ok_or_else(|| invalid("native compiler control-flow citation is not valid UTF-8"))?;
        let id = node
            .citation_id
            .as_deref()
            .ok_or_else(|| invalid("native compiler control-flow citation ID is missing"))?;
        let citation = citations
            .get(id)
            .filter(|citation| citation.id == id)
            .ok_or_else(|| invalid("native compiler control-flow citation is not retained"))?;
        let expected_id = format!(
            "citation-{}",
            &crate::canonical::hash_bytes(
                format!("{}:{start}:{end}:{}", source.id, source.text_digest).as_bytes()
            )[7..31]
        );
        let start_line = source.start_line
            + source.text.as_bytes()[..start]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64;
        let end_line = source.start_line
            + source.text.as_bytes()[..end.saturating_sub(1).max(start)]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64;
        let expected_url = source.url.as_ref().map(|url| {
            format!(
                "{}#L{start_line}-L{end_line}",
                url.split('#').next().unwrap_or(url)
            )
        });
        if id != expected_id
            || citation.source_id != source.id
            || citation.service != source.service
            || citation.revision != source.revision
            || citation.file != source.file
            || citation.start_line != start_line
            || citation.end_line != end_line
            || citation.start_byte != start
            || citation.end_byte != end
            || citation.text_digest != crate::canonical::hash_bytes(exact.as_bytes())
            || node_citation.text_digest != crate::canonical::hash_bytes(exact.as_bytes())
            || citation.evidence_digest != source.evidence_digest
            || citation.authority != source.authority
            || citation.url != expected_url
        {
            return Err(invalid(
                "native compiler control-flow source citation differs from retained bytes",
            ));
        }
    }
    if node_citations.next().is_some() {
        return Err(invalid(
            "native compiler control-flow graph has an unbound citation",
        ));
    }
    Ok(true)
}

pub(in crate::documentation::static_pages) fn validate_page(
    page: &super::super::model::PageContent,
) -> Result<usize, ClewError> {
    let mut count = 0usize;
    for callable in std::iter::once(&page.endpoint)
        .chain(std::iter::once(&page.worker))
        .chain(page.wiring.iter())
    {
        count += usize::from(validate_callable(
            &page.selection.service,
            &page.service_revision,
            &page.observations,
            &page.sources,
            &page.citations,
            callable,
        )?);
    }
    Ok(count)
}
