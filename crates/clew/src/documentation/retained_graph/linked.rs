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

const GRAPH_SCHEMA_JAVA: &str = "codeclew-native-source-calls/1.0";
const GRAPH_SCHEMA_KOTLIN: &str = "codeclew-native-source-calls/1.1";
const GRAPH_SCHEMA_SOURCE_INVOCATIONS: &str = "codeclew-native-source-calls/1.2";

fn graph_schema(graph: &SourceCallGraph) -> &'static str {
    if graph
        .nodes
        .values()
        .any(|node| source::call_sites::uses_source_events(&node.callable, &node.observations))
    {
        GRAPH_SCHEMA_SOURCE_INVOCATIONS
    } else if graph
        .nodes
        .values()
        .any(|node| node.node_projection_kind.is_some())
    {
        GRAPH_SCHEMA_KOTLIN
    } else {
        GRAPH_SCHEMA_JAVA
    }
}
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
    let callable = source::project_callable(&mut ctx, declaration)?;
    let id = key(&callable.key)?;
    let kotlin = source::has_exact_call_capability(&evidence.observations[declaration]);
    let mut calls = Vec::new();
    if kotlin {
        if let Some(retained) = &callable.projection.retained_call_sites {
            for site in &retained.sites {
                calls.push(SourceCallEdge {
                    occurrence_path: None,
                    statement_id: None,
                    source_identity: callable.key.symbol.clone(),
                    target_scope: callable.key.scope.clone(),
                    call_source_ids: vec![site.source_id.clone()],
                    relation_digest: Some(site.normalized_digest.clone()),
                    call: None,
                    conditions: None,
                    reachable: None,
                    exact_call_site: Some(site.clone()),
                    target_declaration: None,
                    target_node: None,
                    status: "UNEXPANDED".into(),
                    receiver_lineage: "UNRESOLVED".into(),
                    runtime_dispatch: "UNRESOLVED".into(),
                    frontiers: vec![],
                });
            }
        }
    } else {
        edges(
            &callable.projection.steps,
            "body",
            &callable.key.symbol,
            &callable.key.scope,
            &ctx.observations,
            &mut calls,
        );
    }
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
        node_projection_kind: kotlin.then_some(callable.kind),
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
                occurrence_path: Some(format!("{path}/call/{ordinal}")),
                statement_id: Some(statement.id.clone()),
                source_identity: owner.into(),
                target_scope: scope.into(),
                call_source_ids: relation.map(|r| r.source_ids.clone()).unwrap_or_default(),
                relation_digest: relation.map(|r| r.digest.clone()),
                call: Some(call.clone()),
                conditions: Some(statement.conditions.clone()),
                reachable: Some(statement.reachable),
                exact_call_site: None,
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

pub(in crate::documentation) fn build_roots_with_depth(
    checked: &Check,
    roots: &[(String, String)],
    max_depth: usize,
) -> Result<SourceCallGraph, ClewError> {
    if max_depth > MAX_DEPTH {
        return Err(crate::documentation::invalid(
            "source-data depth must be between zero and two",
        ));
    }
    let mut graph = SourceCallGraph {
        schema: GRAPH_SCHEMA_JAVA.into(),
        authority: AUTHORITY.into(),
        max_depth,
        max_additional_bodies: MAX_ADDITIONAL_BODIES,
        max_additional_source_bytes: MAX_ADDITIONAL_SOURCE_BYTES,
        nodes: BTreeMap::new(),
        process_links: vec![],
        reverse_examined_processes: BTreeMap::new(),
        reverse_field_references: BTreeMap::new(),
        reverse_property_references: BTreeMap::new(),
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
            if edge.exact_call_site.is_some() {
                expand_kotlin_edge(
                    evidence,
                    edge,
                    depth,
                    &mut graph,
                    &mut additional_bodies,
                    &mut additional_bytes,
                    &mut pending,
                )?;
                continue;
            }
            let Some(call) = edge.call.as_mut() else {
                return Err(crate::documentation::invalid(
                    "source-call edge has neither Java call nor Kotlin exact site",
                ));
            };
            let citation = call.citation_id.clone();
            if call.phase == "CREATION" {
                edge.status = "CONSTRUCTION_NOT_EXPANDED".into();
                continue;
            }
            if call.authority != "COMPILER_EXACT_CALL_RELATION" {
                edge.status = "EXACT_CALL_UNAVAILABLE".into();
                edge.frontiers.push(frontier("EXACT_CALL_UNAVAILABLE", "No unique retained compiler call occurrence; no target body or process is inferred.", &citation));
                continue;
            }
            if call.external_boundary.is_some() {
                edge.status = "DEPENDENCY_BOUNDARY".into();
                edge.frontiers.push(frontier("DEPENDENCY_BODY_NOT_EXPANDED", "Dependency source availability does not establish an implementation body or runtime dispatch.", &citation));
                continue;
            }
            let target = call.target.as_deref().unwrap_or("");
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
                if depth >= graph.max_depth {
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
            call.gaps.retain(|g| g.code != "HELPER_BODY_NOT_EXPANDED");
        }
        graph.nodes.insert(id, current);
    }
    graph.schema = graph_schema(&graph).into();
    // Back edges remain explicit. Cached records are never recursively copied.
    let mut active = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut cycles = BTreeSet::new();
    for id in graph.nodes.keys() {
        cycles_from(id, &graph, &mut active, &mut complete, &mut cycles);
    }
    for (id, path) in cycles {
        if let Some(edge) = graph.nodes.get_mut(&id).and_then(|n| {
            n.calls.iter_mut().find(|e| {
                e.occurrence_path.as_deref() == Some(path.as_str())
                    || e.exact_call_site
                        .as_ref()
                        .is_some_and(|site| site.relation_id == path)
            })
        }) {
            let citation = edge
                .call
                .as_ref()
                .map(|call| call.citation_id.as_str())
                .or_else(|| {
                    edge.exact_call_site
                        .as_ref()
                        .map(|site| site.citation_id.as_str())
                })
                .unwrap_or("");
            edge.frontiers.push(frontier("SOURCE_CALL_CYCLE_FRONTIER", "The retained source graph returns to an already active callable; this back edge is navigation, not recursive body expansion or runtime recursion proof.", citation));
        }
    }
    Ok(graph)
}

pub(in crate::documentation) fn expand_kotlin_edge(
    evidence: &ServiceEvidence,
    edge: &mut SourceCallEdge,
    depth: usize,
    graph: &mut SourceCallGraph,
    additional_bodies: &mut usize,
    additional_bytes: &mut usize,
    pending: &mut VecDeque<(String, usize)>,
) -> Result<(), ClewError> {
    let site = edge
        .exact_call_site
        .as_ref()
        .ok_or_else(|| crate::documentation::invalid("Compiler exact call site disappeared"))?;
    let citation = site.citation_id.as_str();
    let target = site.target_identity.as_str();
    let candidates: Vec<_> = evidence
        .observations
        .values()
        .filter(|observation| {
            source::has_exact_call_capability(observation)
                && observation.kind == "SYMBOL"
                && observation.service == evidence.service
                && source::compiler::admitted(observation)
                && observation.normalized["symbolIdentity"] == target
                && observation.symbol == target
                && observation.normalized["scope"] == edge.target_scope
        })
        .collect();
    let ambiguous = evidence
        .boundaries
        .iter()
        .any(|boundary| boundary == &format!("SCOPE_AMBIGUOUS:{target}"));
    let declaration = match candidates.as_slice() {
        [observation] if !ambiguous => *observation,
        _ => {
            let (code, detail) = if ambiguous || candidates.len() > 1 {
                (
                    "CALL_TARGET_AMBIGUOUS",
                    "The exact compiler target has no unique admitted function declaration in this compilation scope.",
                )
            } else if evidence.observations.values().any(|observation| {
                source::has_exact_call_capability(observation) && observation.symbol == target
            }) {
                (
                    "CALL_TARGET_SCOPE_MISMATCH",
                    "A compiler declaration in another compilation scope is not a substitute for the exact target.",
                )
            } else {
                (
                    "CALL_TARGET_BODY_NOT_CAPTURED",
                    "The exact compiler target has no retained repository function declaration.",
                )
            };
            edge.status = code.into();
            edge.frontiers.push(frontier(code, detail, citation));
            return Ok(());
        }
    };
    edge.target_declaration = Some(declaration.id.clone());
    if !source::compiler::body_envelope(declaration) {
        edge.status = "CALL_TARGET_BODY_NOT_CAPTURED".into();
        edge.frontiers.push(frontier(
            "CALL_TARGET_BODY_NOT_CAPTURED",
            "The exact compiler declaration has no intact admitted documentation-flow body envelope; its declaration identity is retained without examining a body.",
            citation,
        ));
        return Ok(());
    }

    let target_id = root_key(evidence, &declaration.id)?;
    if !graph.nodes.contains_key(&target_id) {
        if depth >= graph.max_depth {
            edge.status = "DEPTH_FRONTIER".into();
            edge.frontiers.push(frontier("SOURCE_CALL_DEPTH_FRONTIER", "The fixed source-call expansion depth was reached; the target body remains unexamined.", citation));
            return Ok(());
        }
        if *additional_bodies >= MAX_ADDITIONAL_BODIES {
            edge.status = "BODY_BUDGET_FRONTIER".into();
            edge.frontiers.push(frontier("SOURCE_CALL_BODY_BUDGET_FRONTIER", "The fixed additional-body budget was reached; this target is not silently omitted.", citation));
            return Ok(());
        }
        let target_node = match node(evidence, &declaration.id) {
            Ok(node) => node,
            Err(error)
                if error.code == crate::error::ErrorCode::InvalidInput
                    && (error.message.starts_with("Compiler ")
                        || error.message.starts_with("Kotlin ")
                        || error.message.starts_with("Roslyn ")
                        || error.message.starts_with("retained Kotlin ")) =>
            {
                edge.status = "BODY_UNAVAILABLE".into();
                edge.frontiers.push(frontier(
                    "CALL_TARGET_BODY_UNAVAILABLE",
                    "The exact compiler declaration was identified, but its retained source evidence did not pass body admission.",
                    citation,
                ));
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let bytes: usize = target_node
            .sources
            .values()
            .map(|source| source.text.len())
            .sum();
        if bytes > MAX_ADDITIONAL_SOURCE_BYTES.saturating_sub(*additional_bytes) {
            edge.status = "SOURCE_BYTES_FRONTIER".into();
            edge.frontiers.push(frontier("SOURCE_CALL_BYTES_FRONTIER", "The fixed additional retained-source byte budget was reached; this target remains unexamined.", citation));
            return Ok(());
        }
        *additional_bodies += 1;
        *additional_bytes += bytes;
        graph.nodes.insert(target_id.clone(), target_node);
        graph.schema = GRAPH_SCHEMA_KOTLIN.into();
        pending.push_back((target_id.clone(), depth + 1));
    }
    edge.target_node = Some(target_id);
    edge.status = "RETAINED_DECLARED_BODY".into();
    Ok(())
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
                let occurrence = edge
                    .occurrence_path
                    .clone()
                    .or_else(|| {
                        edge.exact_call_site
                            .as_ref()
                            .map(|site| site.relation_id.clone())
                    })
                    .unwrap_or_default();
                cycles.insert((id.into(), occurrence));
            } else {
                cycles_from(target, graph, active, complete, cycles);
            }
        }
    }
    active.remove(id);
    complete.insert(id.into());
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

pub(in crate::documentation) fn node_digest(node: &SourceCallNode) -> Result<String, ClewError> {
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
            .as_deref()
            .unwrap_or_default()
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
    let calls: Vec<_> = node
        .calls
        .iter()
        .map(|edge| {
            if let Some(call) = &edge.call {
                json!({
                    "occurrencePath":edge.occurrence_path,"expression":call.expression,"receiver":call.receiver,
                    "arguments":call.arguments,"target":call.target,"scope":edge.target_scope,
                    "authority":call.authority,"phase":call.phase,"conditions":conditions(edge),"reachable":edge.reachable,
                    "bodyStatus":edge.status,"targetNode":edge.target_node,
                    "frontiers":edge.frontiers.iter().map(|g|g.code.as_str()).collect::<Vec<_>>(),
                    "callGaps":call.gaps.iter().map(|g|g.code.as_str()).collect::<Vec<_>>()
                })
            } else {
                let site = edge.exact_call_site.as_ref().unwrap();
                let mut summary = json!({
                    "relationId":site.relation_id,"relationDigest":site.normalized_digest,
                    "expression":site.expression,"target":site.target_identity,"scope":edge.target_scope,
                    "sourceDigest":site.source_digest,"byteStart":site.compilation_byte_start,
                    "byteEnd":site.compilation_byte_end,"bodyStatus":edge.status,
                    "targetNode":edge.target_node,
                    "frontiers":edge.frontiers.iter().map(|g|g.code.as_str()).collect::<Vec<_>>()
                });
                if let Some(bindings) = &site.argument_bindings {
                    summary["argumentBindings"] = json!(bindings);
                }
                summary
            }
        })
        .collect();
    if let Some(kind) = node.node_projection_kind {
        let kotlin_sites = node
            .calls
            .iter()
            .filter_map(|edge| edge.exact_call_site.as_ref())
            .map(|site| {
                (
                    site.relation_id.as_str(),
                    site.normalized_digest.as_str(),
                    site.target_identity.as_str(),
                    site.source_digest.as_str(),
                    site.compilation_byte_start,
                    site.compilation_byte_end,
                )
            })
            .collect::<Vec<_>>();
        let fingerprint = digest(&(
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
            kind,
            &node.callable.source_outline,
            &node.callable.control_flow,
            kotlin_sites,
        ))?;
        if source::compiler::kotlin_constructor_admitted(own) {
            digest(&(
                EXAMINED_SCHEMA,
                fingerprint,
                &own.normalized["documentationConstructorStorage"],
            ))
        } else {
            Ok(fingerprint)
        }
    } else {
        // Keep the frozen Java digest input byte-for-byte unchanged.
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
}
