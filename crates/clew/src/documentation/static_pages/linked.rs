//! Opt-in, bounded source-call navigation and examined documentation context.
//! No edge establishes receiver identity, worker invocation or runtime impact.
use super::{
    model::*,
    source::{self, CallableKey, Context, compiler},
};
use crate::{
    documentation::{check::Check, digest, model::ServiceEvidence},
    error::ClewError,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const GRAPH_SCHEMA: &str = "codeclew-native-source-calls/1.0";
const EXAMINED_SCHEMA: &str = "codeclew-native-examined-source/1.0";
const AUTHORITY: &str = "EXAMINED_DOCUMENTATION_CONTEXT_NOT_RUNTIME_IMPACT";
const MAX_DEPTH: usize = 2;
const MAX_ADDITIONAL_BODIES: usize = 64;
const MAX_ADDITIONAL_SOURCE_BYTES: usize = 1024 * 1024;

fn frontier(code: &str, detail: &str, citation: &str) -> Gap {
    Gap {
        code: code.into(),
        detail: detail.into(),
        citation_id: Some(citation.into()),
    }
}

fn key(identity: &CallableKey) -> Result<String, ClewError> {
    Ok(format!(
        "callable-{}",
        &digest(&(
            identity.service.as_str(),
            identity.scope.as_str(),
            identity.symbol.as_str()
        ))?[7..]
    ))
}

fn node(evidence: &ServiceEvidence, declaration: &str) -> Result<SourceCallNode, ClewError> {
    let mut ctx = Context::new(evidence);
    let callable = source::project_java(&mut ctx, declaration)?;
    let id = key(&callable.key)?;
    let mut calls = Vec::new();
    edges(
        &callable.projection.steps,
        "body",
        &callable.key.symbol,
        &callable.key.scope,
        &ctx.observations,
        &mut calls,
    );
    Ok(SourceCallNode {
        id,
        service: callable.key.service,
        scope: callable.key.scope,
        callable: callable.projection,
        calls,
        citations: ctx.citations,
        observations: ctx.observations,
        sources: ctx.sources,
        examined_source_digest: String::new(),
        data_state: None,
    })
}

fn edges(
    steps: &[Statement],
    path: &str,
    owner: &str,
    scope: &str,
    observations: &BTreeMap<String, crate::documentation::model::Observation>,
    out: &mut Vec<SourceCallEdge>,
) {
    for (index, statement) in steps.iter().enumerate() {
        let path = format!("{path}/{index}");
        for (ordinal, call) in statement.calls.iter().enumerate() {
            let relation = call
                .relation_id
                .as_ref()
                .and_then(|id| observations.get(id));
            out.push(SourceCallEdge {
                occurrence_path: format!("{path}/call/{ordinal}"),
                statement_id: statement.id.clone(),
                source_identity: owner.into(),
                target_scope: scope.into(),
                call_source_ids: relation.map(|r| r.source_ids.clone()).unwrap_or_default(),
                relation_digest: relation.map(|r| r.digest.clone()),
                call: call.clone(),
                conditions: statement.conditions.clone(),
                reachable: statement.reachable,
                target_declaration: None,
                target_node: None,
                status: "UNEXPANDED".into(),
                receiver_lineage: "UNRESOLVED".into(),
                runtime_dispatch: "UNRESOLVED".into(),
                frontiers: vec![],
            });
        }
        edges(
            &statement.children,
            &format!("{path}/true"),
            owner,
            scope,
            observations,
            out,
        );
        edges(
            &statement.alternative,
            &format!("{path}/false"),
            owner,
            scope,
            observations,
            out,
        );
    }
}

fn root_key(evidence: &ServiceEvidence, declaration: &str) -> Result<String, ClewError> {
    let o = &evidence.observations[declaration];
    key(&CallableKey {
        service: evidence.service.clone(),
        scope: o.normalized["scope"].as_str().unwrap_or("").to_owned(),
        symbol: o.symbol.clone(),
    })
}

/// Constructor bodies cited by the selected handoff were already examined by
/// the base projector. Make that dependency explicit instead of hashing every
/// target declaration that happens to be retained beside a call.
fn selected_roots(page: &PageContent, evidence: &ServiceEvidence) -> Vec<(String, &'static str)> {
    let mut roots = vec![
        (
            page.selection.endpoint_declaration.clone(),
            "SELECTED_ENDPOINT",
        ),
        (page.selection.worker_declaration.clone(), "SELECTED_WORKER"),
    ];
    if let Some(id) = &page.selection.wiring_declaration {
        roots.push((id.clone(), "SELECTED_WIRING"));
    }
    let cited_sources: BTreeSet<_> = page
        .handoff
        .citation_ids
        .iter()
        .chain(
            page.handoff
                .gaps
                .iter()
                .filter_map(|g| g.citation_id.as_ref()),
        )
        .filter_map(|id| page.citations.get(id))
        .map(|c| c.source_id.clone())
        .collect();
    let Some(wiring) = page
        .selection
        .wiring_declaration
        .as_ref()
        .and_then(|id| page.observations.get(id))
    else {
        return roots;
    };
    let owners: BTreeSet<_> = [&page.endpoint.declaration_id, &page.worker.declaration_id]
        .into_iter()
        .filter_map(|id| page.observations.get(id))
        .filter(|o| o.normalized["scope"] == wiring.normalized["scope"])
        .filter_map(|o| o.normalized["ownerIdentity"].as_str())
        .collect();
    for observation in page.observations.values().filter(|o| {
        compiler(o)
            && o.normalized["declarationKind"] == "CONSTRUCTOR"
            && o.normalized["scope"] == wiring.normalized["scope"]
            && o.normalized["ownerIdentity"]
                .as_str()
                .is_some_and(|owner| owners.contains(owner))
            && o.source_ids.iter().any(|id| cited_sources.contains(id))
            && evidence
                .observations
                .values()
                .filter(|candidate| {
                    compiler(candidate)
                        && candidate.normalized["declarationKind"] == "CONSTRUCTOR"
                        && candidate.symbol == o.symbol
                        && candidate.normalized["ownerIdentity"] == o.normalized["ownerIdentity"]
                        && candidate.normalized["scope"] == o.normalized["scope"]
                })
                .count()
                == 1
    }) {
        roots.push((observation.id.clone(), "SELECTED_HANDOFF_CONSTRUCTOR"));
    }
    roots
}

fn handoff_digest(page: &PageContent) -> Result<String, ClewError> {
    digest(&(
        EXAMINED_SCHEMA,
        &page.handoff.status,
        &page.handoff.endpoint_field,
        &page.handoff.worker_field,
        page.handoff
            .gaps
            .iter()
            .map(|g| g.code.as_str())
            .collect::<Vec<_>>(),
    ))
}

/// Roots are preloaded before breadth-first expansion, so a shared body has one
/// canonical record and the depth budget uses its shortest selected-root path.
fn build(checked: &Check, pages: &[PageContent]) -> Result<SourceCallGraph, ClewError> {
    let roots: Vec<_> = pages
        .iter()
        .filter(|p| p.selection.expand_source_calls)
        .flat_map(|p| {
            selected_roots(p, &checked.services[&p.selection.service])
                .into_iter()
                .map(move |(declaration, _)| (p.selection.service.clone(), declaration))
        })
        .collect();
    build_roots(checked, &roots)
}
/// Same bounded expansion over explicit callable declarations; no process/handoff
/// association is manufactured for model Work.
pub(super) fn build_roots(
    checked: &Check,
    roots: &[(String, String)],
) -> Result<SourceCallGraph, ClewError> {
    let mut graph = SourceCallGraph {
        schema: GRAPH_SCHEMA.into(),
        authority: AUTHORITY.into(),
        max_depth: MAX_DEPTH,
        max_additional_bodies: MAX_ADDITIONAL_BODIES,
        max_additional_source_bytes: MAX_ADDITIONAL_SOURCE_BYTES,
        nodes: BTreeMap::new(),
        process_links: vec![],
        reverse_examined_processes: BTreeMap::new(),
        reverse_field_references: BTreeMap::new(),
    };
    for (service, declaration) in roots {
        let evidence = checked.services.get(service).ok_or_else(|| {
            crate::documentation::invalid("source-data selected service is unavailable")
        })?;
        let id = root_key(evidence, declaration)?;
        if let std::collections::btree_map::Entry::Vacant(entry) = graph.nodes.entry(id) {
            entry.insert(node(evidence, declaration)?);
        }
    }
    let mut pending: VecDeque<_> = graph.nodes.keys().cloned().map(|id| (id, 0)).collect();
    let mut visited = BTreeSet::new();
    let mut additional_bodies = 0;
    let mut additional_bytes = 0;
    while let Some((id, depth)) = pending.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let mut current = graph.nodes[&id].clone();
        let evidence = &checked.services[&current.service];
        for edge in &mut current.calls {
            let citation = edge.call.citation_id.clone();
            if edge.call.phase == "CREATION" {
                edge.status = "CONSTRUCTION_NOT_EXPANDED".into();
                continue;
            }
            if edge.call.authority != "COMPILER_EXACT_CALL_RELATION" {
                edge.status = "EXACT_CALL_UNAVAILABLE".into();
                edge.frontiers.push(frontier("EXACT_CALL_UNAVAILABLE", "No unique retained compiler call occurrence; no target body or process is inferred.", &citation));
                continue;
            }
            if edge.call.external_boundary.is_some() {
                edge.status = "DEPENDENCY_BOUNDARY".into();
                edge.frontiers.push(frontier("DEPENDENCY_BODY_NOT_EXPANDED", "Dependency source availability does not establish an implementation body or runtime dispatch.", &citation));
                continue;
            }
            let target = edge.call.target.as_deref().unwrap_or("");
            let candidates: Vec<_> = evidence
                .observations
                .values()
                .filter(|o| {
                    compiler(o)
                        && o.normalized["resolution"] == "COMPILER_EXACT"
                        && o.normalized["declarationKind"] == "METHOD"
                        && o.symbol == target
                        && o.normalized["scope"] == current.scope
                })
                .collect();
            let ambiguous = evidence
                .boundaries
                .iter()
                .any(|b| b == &format!("SCOPE_AMBIGUOUS:{target}"));
            let declaration = match candidates.as_slice() {
                [o] if !ambiguous => *o,
                _ => {
                    let (code, detail) = if ambiguous || candidates.len() > 1 {
                        (
                            "CALL_TARGET_AMBIGUOUS",
                            "The target has no unique admitted declaration in this compiler scope.",
                        )
                    } else if evidence
                        .observations
                        .values()
                        .any(|o| compiler(o) && o.symbol == target)
                    {
                        (
                            "CALL_TARGET_SCOPE_MISMATCH",
                            "A declaration in another compilation scope is not a substitute for the exact target.",
                        )
                    } else {
                        (
                            "CALL_TARGET_BODY_NOT_CAPTURED",
                            "The exact target has no retained repository method body.",
                        )
                    };
                    edge.status = code.into();
                    edge.frontiers.push(frontier(code, detail, &citation));
                    continue;
                }
            };
            edge.target_declaration = Some(declaration.id.clone());
            let target_id = root_key(evidence, &declaration.id)?;
            if !graph.nodes.contains_key(&target_id) {
                if depth >= MAX_DEPTH {
                    edge.status = "DEPTH_FRONTIER".into();
                    edge.frontiers.push(frontier("SOURCE_CALL_DEPTH_FRONTIER", "The fixed source-call expansion depth was reached; the target body remains unexamined.", &citation));
                    continue;
                }
                if additional_bodies >= MAX_ADDITIONAL_BODIES {
                    edge.status = "BODY_BUDGET_FRONTIER".into();
                    edge.frontiers.push(frontier("SOURCE_CALL_BODY_BUDGET_FRONTIER", "The fixed additional-body budget was reached; this target is not silently omitted.", &citation));
                    continue;
                }
                let body_bytes: usize = declaration
                    .source_ids
                    .iter()
                    .filter_map(|id| evidence.sources.get(id))
                    .map(|s| s.text.len())
                    .sum();
                if body_bytes > MAX_ADDITIONAL_SOURCE_BYTES.saturating_sub(additional_bytes) {
                    edge.status = "SOURCE_BYTES_FRONTIER".into();
                    edge.frontiers.push(frontier("SOURCE_CALL_BYTES_FRONTIER", "The target body exceeds the remaining retained-source byte budget before parsing.", &citation));
                    continue;
                }
                let target_node = node(evidence, &declaration.id)?;
                if target_node.callable.gaps.iter().any(|g| {
                    matches!(
                        g.code.as_str(),
                        "CALLABLE_SOURCE_UNAVAILABLE"
                            | "JAVA_PARSE_UNAVAILABLE"
                            | "CALLABLE_BODY_AMBIGUOUS"
                            | "CALLABLE_SYNTAX_PARTIAL"
                            | "CALLABLE_BODY_UNAVAILABLE"
                    )
                }) {
                    edge.status = "BODY_UNAVAILABLE".into();
                    edge.frontiers.push(frontier("CALL_TARGET_BODY_UNAVAILABLE", "The retained target declaration has no uniquely available parseable source body; no implementation is chosen.", &citation));
                    continue;
                }
                let bytes: usize = target_node.sources.values().map(|s| s.text.len()).sum();
                if bytes > MAX_ADDITIONAL_SOURCE_BYTES.saturating_sub(additional_bytes) {
                    edge.status = "SOURCE_BYTES_FRONTIER".into();
                    edge.frontiers.push(frontier("SOURCE_CALL_BYTES_FRONTIER", "The fixed additional retained-source byte budget was reached; this target remains unexamined.", &citation));
                    continue;
                }
                additional_bodies += 1;
                additional_bytes += bytes;
                graph.nodes.insert(target_id.clone(), target_node);
                pending.push_back((target_id.clone(), depth + 1));
            }
            // A selected root can itself be an interface/abstract method.
            if graph.nodes[&target_id].callable.gaps.iter().any(|g| {
                matches!(
                    g.code.as_str(),
                    "CALLABLE_SOURCE_UNAVAILABLE"
                        | "JAVA_PARSE_UNAVAILABLE"
                        | "CALLABLE_BODY_AMBIGUOUS"
                        | "CALLABLE_SYNTAX_PARTIAL"
                        | "CALLABLE_BODY_UNAVAILABLE"
                )
            }) {
                edge.status = "BODY_UNAVAILABLE".into();
                edge.frontiers.push(frontier(
                    "CALL_TARGET_BODY_UNAVAILABLE",
                    "The selected target has no uniquely available source body.",
                    &citation,
                ));
                continue;
            }
            edge.target_node = Some(target_id);
            edge.status = "RETAINED_DECLARED_BODY".into();
            edge.call
                .gaps
                .retain(|g| g.code != "HELPER_BODY_NOT_EXPANDED");
        }
        graph.nodes.insert(id, current);
    }
    // Back edges remain explicit. Cached records are never recursively copied.
    let mut active = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut cycles = BTreeSet::new();
    for id in graph.nodes.keys() {
        cycles_from(id, &graph, &mut active, &mut complete, &mut cycles);
    }
    for (id, path) in cycles {
        if let Some(edge) = graph
            .nodes
            .get_mut(&id)
            .and_then(|n| n.calls.iter_mut().find(|e| e.occurrence_path == path))
        {
            edge.frontiers.push(frontier("SOURCE_CALL_CYCLE_FRONTIER", "The retained source graph returns to an already active callable; this back edge is navigation, not recursive body expansion or runtime recursion proof.", &edge.call.citation_id));
        }
    }
    Ok(graph)
}

fn cycles_from(
    id: &str,
    graph: &SourceCallGraph,
    active: &mut BTreeSet<String>,
    complete: &mut BTreeSet<String>,
    cycles: &mut BTreeSet<(String, String)>,
) {
    if complete.contains(id) {
        return;
    }
    active.insert(id.into());
    for edge in &graph.nodes[id].calls {
        if let Some(target) = &edge.target_node {
            if active.contains(target) {
                cycles.insert((id.into(), edge.occurrence_path.clone()));
            } else {
                cycles_from(target, graph, active, complete, cycles);
            }
        }
    }
    active.remove(id);
    complete.insert(id.into());
}

fn reachable_nodes(
    roots: impl IntoIterator<Item = String>,
    graph: &SourceCallGraph,
) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    let mut pending: Vec<_> = roots.into_iter().collect();
    while let Some(id) = pending.pop() {
        if result.insert(id.clone()) {
            pending.extend(
                graph.nodes[&id]
                    .calls
                    .iter()
                    .filter_map(|e| e.target_node.clone()),
            );
        }
    }
    result
}

fn decorate(steps: &mut [Statement], path: &str, calls: &[SourceCallEdge]) {
    for (index, row) in steps.iter_mut().enumerate() {
        let path = format!("{path}/{index}");
        for (ordinal, call) in row.calls.iter_mut().enumerate() {
            let occurrence = format!("{path}/call/{ordinal}");
            if let Some(edge) = calls.iter().find(|e| e.occurrence_path == occurrence) {
                call.expanded_node = edge.target_node.clone();
                if call.expanded_node.is_some() {
                    call.gaps.retain(|g| g.code != "HELPER_BODY_NOT_EXPANDED");
                }
            }
        }
        decorate(&mut row.children, &format!("{path}/true"), calls);
        decorate(&mut row.alternative, &format!("{path}/false"), calls);
    }
}

/// The fingerprint uses stable body-relative occurrences. Full Source/Observation
/// records keep their original coordinates, revision and binding provenance.
fn statement_paths(steps: &[Statement], path: &str, result: &mut BTreeMap<String, String>) {
    for (index, row) in steps.iter().enumerate() {
        let path = format!("{path}/{index}");
        result.insert(row.id.clone(), path.clone());
        statement_paths(&row.children, &format!("{path}/true"), result);
        statement_paths(&row.alternative, &format!("{path}/false"), result);
    }
}

pub(super) fn node_digest(node: &SourceCallNode) -> Result<String, ClewError> {
    let mut paths = BTreeMap::new();
    statement_paths(&node.callable.steps, "body", &mut paths);
    let own = &node.observations[&node.callable.declaration_id];
    let declarations: BTreeMap<_, _> = node
        .observations
        .values()
        .filter(|o| {
            o.id == own.id
                || (o.kind == "SYMBOL"
                    && o.normalized["declarationKind"] == "FIELD"
                    && o.normalized["ownerIdentity"] == own.normalized["ownerIdentity"]
                    && o.normalized["scope"] == own.normalized["scope"])
        })
        .map(|o| {
            (
                (
                    o.normalized["scope"].as_str().unwrap_or("").to_owned(),
                    o.symbol.clone(),
                ),
                o.source_ids
                    .iter()
                    .filter_map(|id| node.sources.get(id))
                    .map(|s| (s.text_digest.clone(), s.authority.clone()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let conditions = |edge: &SourceCallEdge| {
        edge.conditions
            .iter()
            .map(|c| {
                // Only projector-generated opaque continuation labels contain step IDs.
                let expression = if c.expression.starts_with("unsupported statement ") {
                    paths
                        .iter()
                        .fold(c.expression.clone(), |s, (id, path)| s.replace(id, path))
                } else {
                    c.expression.clone()
                };
                json!({"expression":expression,"holds":c.holds})
            })
            .collect::<Vec<_>>()
    };
    let calls: Vec<_> = node.calls.iter().map(|e| json!({
        "occurrencePath":e.occurrence_path,"expression":e.call.expression,"receiver":e.call.receiver,
        "arguments":e.call.arguments,"target":e.call.target,"scope":e.target_scope,
        "authority":e.call.authority,"phase":e.call.phase,"conditions":conditions(e),"reachable":e.reachable,
        "bodyStatus":e.status,"targetNode":e.target_node,
        "frontiers":e.frontiers.iter().map(|g|g.code.as_str()).collect::<Vec<_>>(),
        "callGaps":e.call.gaps.iter().map(|g|g.code.as_str()).collect::<Vec<_>>()
    })).collect();
    digest(&(
        EXAMINED_SCHEMA,
        &node.service,
        &node.scope,
        &node.callable.symbol,
        declarations.into_iter().collect::<Vec<_>>(),
        calls,
        node.callable
            .gaps
            .iter()
            .map(|g| g.code.as_str())
            .collect::<Vec<_>>(),
    ))
}

pub(super) fn attach(checked: &Check, projection: &mut BundleProjection) -> Result<(), ClewError> {
    if !projection
        .pages
        .iter()
        .any(|p| p.selection.expand_source_calls)
    {
        return Ok(());
    }
    let mut graph = build(checked, &projection.pages)?;
    let mut endpoint_processes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut local_memberships: BTreeMap<String, BTreeSet<ExaminedMembership>> = BTreeMap::new();
    for page in projection
        .pages
        .iter()
        .filter(|p| p.selection.expand_source_calls)
    {
        let evidence = &checked.services[&page.selection.service];
        let endpoint = root_key(evidence, &page.selection.endpoint_declaration)?;
        endpoint_processes
            .entry(endpoint)
            .or_default()
            .push(page.id.clone());
        let mut members = BTreeSet::new();
        let mut roots = Vec::new();
        for (declaration, reason) in selected_roots(page, evidence) {
            let id = root_key(evidence, &declaration)?;
            members.insert(ExaminedMembership {
                node: id.clone(),
                reason: reason.into(),
                via_process: None,
            });
            roots.push(id);
        }
        for id in reachable_nodes(roots, &graph) {
            members.insert(ExaminedMembership {
                node: id,
                reason: "SOURCE_CALL_BODY".into(),
                via_process: None,
            });
        }
        local_memberships.insert(page.id.clone(), members);
    }
    for page in projection
        .pages
        .iter()
        .filter(|p| p.selection.expand_source_calls)
    {
        let evidence = &checked.services[&page.selection.service];
        let roots = [
            &page.selection.endpoint_declaration,
            &page.selection.worker_declaration,
        ]
        .into_iter()
        .map(|d| root_key(evidence, d))
        .collect::<Result<Vec<_>, _>>()?;
        for id in reachable_nodes(roots, &graph) {
            for edge in &mut graph.nodes.get_mut(&id).unwrap().calls {
                let Some(target) = edge.target_node.as_ref() else {
                    continue;
                };
                let Some(processes) = endpoint_processes.get(target) else {
                    continue;
                };
                if processes.len() != 1 {
                    let gap = frontier(
                        "SELECTED_PROCESS_AMBIGUOUS",
                        "More than one opted-in process selects this endpoint; no canonical child process is inferred.",
                        &edge.call.citation_id,
                    );
                    if !edge.frontiers.contains(&gap) {
                        edge.frontiers.push(gap);
                    }
                    continue;
                }
                let child = &processes[0];
                if child == &page.id {
                    continue;
                }
                graph.process_links.push(ProcessCallLink { from_process: page.id.clone(), to_process: child.clone(),
                    caller_node: id.clone(), occurrence_path: edge.occurrence_path.clone(),
                    relation_id: edge.call.relation_id.clone().unwrap(), citation_id: edge.call.citation_id.clone(),
                    authority: "SOURCE_CALL_TO_SELECTED_ENDPOINT".into(),
                    limitation: "The child worker is selected process context. This call link proves neither receiver instance lineage, enqueue success, worker scheduling, runtime dispatch nor delivery.".into() });
            }
        }
    }
    graph.process_links.sort_by(|a, b| {
        (
            &a.from_process,
            &a.to_process,
            &a.caller_node,
            &a.occurrence_path,
        )
            .cmp(&(
                &b.from_process,
                &b.to_process,
                &b.caller_node,
                &b.occurrence_path,
            ))
    });
    graph.process_links.dedup();
    for n in graph.nodes.values_mut() {
        n.examined_source_digest = node_digest(n)?;
        decorate(&mut n.callable.steps, "body", &n.calls);
    }
    let local_handoff_digests = projection
        .pages
        .iter()
        .filter(|p| p.selection.expand_source_calls)
        .map(|p| Ok((p.id.clone(), handoff_digest(p)?)))
        .collect::<Result<BTreeMap<_, _>, ClewError>>()?;
    for page in projection
        .pages
        .iter_mut()
        .filter(|p| p.selection.expand_source_calls)
    {
        let mut memberships = local_memberships[&page.id].clone();
        let mut pending = vec![page.id.clone()];
        let mut seen = BTreeSet::from([page.id.clone()]);
        while let Some(current) = pending.pop() {
            for link in graph
                .process_links
                .iter()
                .filter(|l| l.from_process == current)
            {
                if seen.insert(link.to_process.clone()) {
                    for member in &local_memberships[&link.to_process] {
                        memberships.insert(ExaminedMembership {
                            node: member.node.clone(),
                            reason: "LINKED_PROCESS_CONTEXT".into(),
                            via_process: Some(link.to_process.clone()),
                        });
                    }
                    pending.push(link.to_process.clone());
                }
            }
        }
        let memberships: Vec<_> = memberships.into_iter().collect();
        let versions: Vec<_> = memberships
            .iter()
            .map(|m| (m, &graph.nodes[&m.node].examined_source_digest))
            .collect();
        let handoff_context_digests: BTreeMap<_, _> = seen
            .into_iter()
            .map(|id| (id.clone(), local_handoff_digests[&id].clone()))
            .collect();
        let examined_source_digest =
            digest(&(EXAMINED_SCHEMA, versions, &handoff_context_digests))?;
        for member in &memberships {
            let reverse = graph
                .reverse_examined_processes
                .entry(member.node.clone())
                .or_default();
            if let Some(row) = reverse.iter_mut().find(|r| r.process_id == page.id) {
                row.reasons.push(member.clone());
            } else {
                reverse.push(ExaminedProcessReason {
                    process_id: page.id.clone(),
                    reasons: vec![member.clone()],
                });
            }
        }
        let evidence = &checked.services[&page.selection.service];
        for callable in [&mut page.endpoint, &mut page.worker]
            .into_iter()
            .chain(page.wiring.iter_mut())
        {
            let id = root_key(evidence, &callable.declaration_id)?;
            decorate(&mut callable.steps, "body", &graph.nodes[&id].calls);
        }
        page.examined_sources = Some(ExaminedSources {
            schema: EXAMINED_SCHEMA.into(),
            authority: AUTHORITY.into(),
            examined_source_digest,
            memberships,
            handoff_context_digests,
        });
    }
    projection.source_call_graph = Some(graph);
    Ok(())
}
