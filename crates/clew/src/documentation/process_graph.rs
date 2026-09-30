//! Complete, finite call/evidence graphs over one retained service snapshot.
use super::{
    bytes, digest, invalid,
    model::{Observation, ServiceEvidence, Source},
    process_candidates,
    store::Repository,
    work::{self, Work},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};

const GRAPH_SCHEMA: &str = "codeclew-documentation-process-graph/1.0";

/// Collect the complete known-reachable graph from an exact retained root.
pub(super) fn run(
    repo: &Repository,
    service: &str,
    declaration: &str,
    snapshot: &str,
    output: &Path,
) -> Result<Value, ClewError> {
    let (checked, handle) =
        super::check::Check::retained(repo, Some(snapshot), &BTreeSet::from([service.to_owned()]))?;
    let evidence = checked
        .services
        .get(service)
        .ok_or_else(|| invalid("selected service is missing from retained evidence"))?;
    let root = evidence
        .observations
        .get(declaration)
        .filter(|observation| {
            observation.service == service && callable_observation(observation)
        })
        .ok_or_else(|| {
            invalid(format!(
                "--declaration must name an exact callable SYMBOL observation in service {service} and snapshot {handle}; use the exact retained SYMBOL observation ID"
            ))
        })?;
    let _root_scope = exact_scope(root).ok_or_else(|| {
        invalid("selected declaration has no exact retained scope; choose a scoped callable observation")
    })?;

    let mut artifact = collect(evidence, root, &handle, &checked.input_digest)?;
    let artifact_digest = digest(&artifact)?;
    artifact["artifactDigest"] = json!(artifact_digest);
    let encoded = bytes(&artifact)?;
    work::write_atomic_file(output, &encoded)?;

    Ok(json!({
        "schema": "codeclew-documentation-process-graph-result/1.0",
        "status": "SAVED",
        "snapshot": handle,
        "rootDeclaration": declaration,
        "methodCount": artifact["methods"].as_array().map_or(0, Vec::len),
        "eventCount": artifact["events"].as_array().map_or(0, Vec::len)
            + artifact["unboundEvents"].as_array().map_or(0, Vec::len),
        "unboundEventCount": artifact["unboundEvents"].as_array().map_or(0, Vec::len),
        "edgeCount": artifact["edges"].as_array().map_or(0, Vec::len),
        "frontierCount": artifact["frontiers"].as_array().map_or(0, Vec::len),
        "artifactDigest": artifact_digest,
        "output": output.to_string_lossy(),
    }))
}

fn exact_scope(observation: &Observation) -> Option<&str> {
    observation.normalized["scope"]
        .as_str()
        .filter(|value| !value.is_empty())
}

pub(super) fn callable_observation(observation: &Observation) -> bool {
    process_candidates::callable(observation)
        && (matches!(
            observation.normalized["declarationKind"].as_str(),
            Some("METHOD" | "CONSTRUCTOR" | "FUNCTION")
        ) || (observation.normalized.get("declarationKind").is_none()
            && matches!(
                observation.normalized["syntaxKind"].as_str(),
                Some(
                    "function_definition"
                        | "function_declaration"
                        | "method_declaration"
                        | "constructor_declaration"
                        | "secondary_constructor"
                )
            )))
}

/// Rebuild the full audit graph from Work's already hydrated retained snapshot.
/// This path performs no repository reads or source capture.
pub(super) fn collect_from_work(work: &Work) -> Result<Value, ClewError> {
    if work.request.context_profile.as_deref() != Some("process-graph-v1") {
        return Err(invalid(
            "PROCESS_GRAPH_PROFILE_REQUIRED: prepare fresh Work with contextProfile=process-graph-v1",
        ));
    }
    let snapshot = work
        .snapshot
        .as_deref()
        .ok_or_else(|| invalid("PROCESS_GRAPH_SNAPSHOT_REQUIRED: saved snapshot is unavailable"))?;
    let service = work
        .subject
        .strip_prefix("service:")
        .filter(|service| !service.trim().is_empty())
        .ok_or_else(|| invalid("PROCESS_GRAPH_SERVICE_REQUIRED: Work must select one service"))?;
    let declaration = work
        .request
        .root_declaration
        .as_deref()
        .filter(|declaration| !declaration.trim().is_empty())
        .ok_or_else(|| invalid("PROCESS_GRAPH_ROOT_REQUIRED: Work has no exact rootDeclaration"))?;
    let evidence = work.checked.services.get(service).ok_or_else(|| {
        invalid("PROCESS_GRAPH_SERVICE_UNAVAILABLE: Work has no selected service evidence")
    })?;
    let root = evidence
        .observations
        .get(declaration)
        .filter(|observation| {
            observation.service == service
                && callable_observation(observation)
                && exact_scope(observation).is_some()
        })
        .ok_or_else(|| {
            invalid(
                "PROCESS_GRAPH_ROOT_INVALID: Work root is not an exact scoped callable observation",
            )
        })?;
    let mut artifact = collect(evidence, root, snapshot, &work.checked.input_digest)?;
    let artifact_digest = digest(&artifact)?;
    artifact["artifactDigest"] = json!(artifact_digest);
    Ok(artifact)
}

fn collect(
    evidence: &ServiceEvidence,
    root: &Observation,
    snapshot: &str,
    input_digest: &str,
) -> Result<Value, ClewError> {
    let mut methods_by_scope_symbol: BTreeMap<(&str, &str), Vec<&Observation>> = BTreeMap::new();
    let mut scopes_by_symbol: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut unscoped_methods_by_symbol: BTreeMap<&str, Vec<&Observation>> = BTreeMap::new();
    let mut flows_by_scope_symbol: BTreeMap<(&str, &str), Vec<&Observation>> = BTreeMap::new();
    let mut unscoped_flows_by_symbol: BTreeMap<&str, Vec<&Observation>> = BTreeMap::new();
    let mut fields_by_scope_owner_name: BTreeMap<(&str, &str, &str), Vec<&Observation>> =
        BTreeMap::new();
    for observation in evidence.observations.values() {
        if observation.service != evidence.service {
            continue;
        }
        match observation.kind.as_str() {
            "SYMBOL" if callable_observation(observation) => {
                if let Some(scope) = exact_scope(observation) {
                    methods_by_scope_symbol
                        .entry((scope, observation.symbol.as_str()))
                        .or_default()
                        .push(observation);
                    scopes_by_symbol
                        .entry(observation.symbol.as_str())
                        .or_default()
                        .insert(scope);
                } else {
                    unscoped_methods_by_symbol
                        .entry(observation.symbol.as_str())
                        .or_default()
                        .push(observation);
                }
            }
            "FLOW" => {
                if let Some(scope) = exact_scope(observation) {
                    flows_by_scope_symbol
                        .entry((scope, observation.symbol.as_str()))
                        .or_default()
                        .push(observation);
                } else {
                    unscoped_flows_by_symbol
                        .entry(observation.symbol.as_str())
                        .or_default()
                        .push(observation);
                }
            }
            "SYMBOL" if observation.normalized["declarationKind"] == "FIELD" => {
                if let (Some(scope), Some(owner), Some(name)) = (
                    exact_scope(observation),
                    observation.normalized["ownerIdentity"].as_str(),
                    observation.normalized["name"].as_str(),
                ) {
                    fields_by_scope_owner_name
                        .entry((scope, owner, name))
                        .or_default()
                        .push(observation);
                }
            }
            _ => {}
        }
    }
    for rows in methods_by_scope_symbol.values_mut() {
        rows.sort_by(|left, right| left.id.cmp(&right.id));
    }

    let mut queue = VecDeque::from([root.id.as_str()]);
    let mut queued = BTreeSet::from([root.id.as_str()]);
    let mut visited = BTreeSet::new();
    let mut methods = Vec::new();
    let mut events = Vec::new();
    let mut unbound_events = BTreeMap::<String, Value>::new();
    let mut sources_by_id = BTreeMap::<String, Source>::new();
    let mut edges = Vec::new();
    let mut frontiers = Vec::new();
    for boundary in &evidence.boundaries {
        frontiers.push(json!({
            "kind": "SERVICE_PROVIDER_BOUNDARY",
            "detail": boundary,
        }));
    }

    while let Some(method_id) = queue.pop_front() {
        if !visited.insert(method_id) {
            continue;
        }
        let method = evidence
            .observations
            .get(method_id)
            .filter(|observation| callable_observation(observation))
            .ok_or_else(|| invalid("retained graph queue contained an unavailable method"))?;
        let scope = exact_scope(method).ok_or_else(|| {
            invalid("reachable method has no exact retained scope; graph cannot merge unscoped declarations")
        })?;
        let method_key = (scope, method.symbol.as_str());
        let mut method_frontiers = Vec::new();
        for flow in unscoped_flows_by_symbol
            .get(method.symbol.as_str())
            .into_iter()
            .flatten()
        {
            unbound_events.insert(flow.id.clone(), json!(flow));
            for source_id in &flow.source_ids {
                if let Some(source) = evidence.sources.get(source_id) {
                    sources_by_id.insert(source_id.clone(), source.clone());
                } else {
                    method_frontiers.push(json!({
                        "kind": "FLOW_SOURCE_MISSING",
                        "methodId": method.id,
                        "flowId": flow.id,
                        "sourceId": source_id,
                    }));
                }
            }
            method_frontiers.push(json!({
                "kind": "FLOW_SCOPE_UNAVAILABLE",
                "methodId": method.id,
                "eventId": flow.id,
                "observation": flow,
            }));
        }
        let mut source_ids = method.source_ids.clone();
        source_ids.sort();
        source_ids.dedup();
        for source_id in &source_ids {
            if let Some(source) = evidence.sources.get(source_id) {
                sources_by_id.insert(source_id.clone(), source.clone());
            } else {
                method_frontiers.push(json!({
                    "kind": "METHOD_SOURCE_MISSING",
                    "sourceId": source_id,
                    "methodId": method.id,
                }));
            }
        }
        if source_ids.is_empty() {
            method_frontiers.push(json!({
                "kind": "METHOD_SOURCE_UNAVAILABLE",
                "methodId": method.id,
            }));
        }
        if let Some(boundaries) = method
            .normalized
            .pointer("/documentation/boundaries")
            .and_then(Value::as_array)
        {
            for boundary in boundaries {
                method_frontiers.push(json!({
                    "kind": "METHOD_PROVIDER_BOUNDARY",
                    "methodId": method.id,
                    "detail": boundary,
                }));
            }
        }

        let expected_events = method
            .normalized
            .pointer("/documentation/events")
            .and_then(Value::as_array);
        let scoped_candidates = methods_by_scope_symbol
            .get(&method_key)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let association_ambiguous = scoped_candidates.len() != 1;
        let candidate_method_ids = scoped_candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect::<Vec<_>>();
        let mut flows = flows_by_scope_symbol
            .get(&method_key)
            .cloned()
            .unwrap_or_default();
        flows.sort_by(|left, right| {
            left.normalized["ordinal"]
                .as_u64()
                .cmp(&right.normalized["ordinal"].as_u64())
                .then_with(|| left.id.cmp(&right.id))
        });
        if association_ambiguous && !flows.is_empty() {
            method_frontiers.push(json!({
                "kind": "FLOW_METHOD_ASSOCIATION_AMBIGUOUS",
                "methodId": method.id,
                "candidateMethodIds": candidate_method_ids,
                "flowIds": flows.iter().map(|event| &event.id).collect::<Vec<_>>(),
            }));
        }

        let mut slots = Vec::new();
        let mut flow_by_ordinal: BTreeMap<u64, Vec<&Observation>> = BTreeMap::new();
        for flow in &flows {
            if let Some(ordinal) = flow.normalized["ordinal"].as_u64() {
                flow_by_ordinal.entry(ordinal).or_default().push(flow);
            } else {
                method_frontiers.push(json!({
                    "kind": "FLOW_ORDINAL_UNAVAILABLE",
                    "methodId": method.id,
                    "flowId": flow.id,
                }));
            }
        }
        if let Some(expected_events) = expected_events {
            for ordinal in 0..expected_events.len() as u64 {
                let matching = flow_by_ordinal.get(&ordinal).cloned().unwrap_or_default();
                let status = if association_ambiguous && !matching.is_empty() {
                    let frontier = json!({
                        "kind": "FLOW_METHOD_ASSOCIATION_AMBIGUOUS",
                        "methodId": method.id,
                        "ordinal": ordinal,
                        "candidateMethodIds": candidate_method_ids,
                        "flowIds": matching.iter().map(|event| &event.id).collect::<Vec<_>>(),
                    });
                    method_frontiers.push(frontier.clone());
                    frontier
                } else {
                    match matching.len() {
                        0 => {
                            let frontier = json!({
                                "kind": "EXPECTED_FLOW_SLOT_MISSING",
                                "methodId": method.id,
                                "ordinal": ordinal,
                                "providerEvent": expected_events[ordinal as usize],
                            });
                            method_frontiers.push(frontier.clone());
                            frontier
                        }
                        1 => json!({"status":"OBSERVED"}),
                        _ => {
                            let frontier = json!({
                                "kind": "DUPLICATE_FLOW_SLOT",
                                "methodId": method.id,
                                "ordinal": ordinal,
                                "flowIds": matching.iter().map(|event| &event.id).collect::<Vec<_>>(),
                            });
                            method_frontiers.push(frontier.clone());
                            frontier
                        }
                    }
                };
                slots.push(json!({
                    "ordinal": ordinal,
                    "providerEvent": expected_events[ordinal as usize],
                    "flowIds": matching.iter().map(|event| &event.id).collect::<Vec<_>>(),
                    "evidence": status,
                }));
            }
            for (ordinal, observations) in flow_by_ordinal.range(expected_events.len() as u64..) {
                let frontier = json!({
                    "kind": "UNEXPECTED_FLOW_SLOT",
                    "methodId": method.id,
                    "ordinal": ordinal,
                    "flowIds": observations.iter().map(|event| &event.id).collect::<Vec<_>>(),
                });
                method_frontiers.push(frontier);
            }
        } else {
            let frontier = json!({
                "kind": "PROVIDER_FLOW_SLOTS_UNAVAILABLE",
                "methodId": method.id,
            });
            slots.push(json!({"status":"UNAVAILABLE","frontier":frontier}));
            method_frontiers.push(frontier);
        }

        let mut event_ids = Vec::new();
        for flow in flows {
            if association_ambiguous {
                unbound_events.insert(flow.id.clone(), json!(flow));
            } else {
                event_ids.push(flow.id.clone());
                events.push(json!(flow));
            }
            for source_id in &flow.source_ids {
                if let Some(source) = evidence.sources.get(source_id) {
                    sources_by_id.insert(source_id.clone(), source.clone());
                } else {
                    method_frontiers.push(json!({
                        "kind": "FLOW_SOURCE_MISSING",
                        "methodId": method.id,
                        "flowId": flow.id,
                        "sourceId": source_id,
                    }));
                }
            }
            if flow.normalized["kind"] == "BOUNDARY" {
                method_frontiers.push(json!({
                    "kind": "PROVIDER_FLOW_BOUNDARY",
                    "methodId": method.id,
                    "flowId": flow.id,
                    "detail": flow.normalized,
                }));
            }
            if !matches!(flow.normalized["kind"].as_str(), Some("CALL" | "CONSTRUCT")) {
                continue;
            }
            let edge_id = format!("edge:{}", flow.id);
            let target = flow.normalized["target"]
                .as_str()
                .filter(|value| !value.is_empty());
            let (target_resolution, target_candidate) = match target {
                None => ("TARGET_IDENTITY_UNAVAILABLE", None),
                Some(target) => match methods_by_scope_symbol
                    .get(&(scope, target))
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                {
                    [candidate]
                        if !unscoped_methods_by_symbol
                            .get(target)
                            .is_some_and(|methods| !methods.is_empty()) =>
                    {
                        ("SAME_SCOPE_UNIQUE", Some(*candidate))
                    }
                    [_] => ("TARGET_SCOPE_ASSOCIATION_AMBIGUOUS", None),
                    [] if scopes_by_symbol
                        .get(target)
                        .is_some_and(|scopes| !scopes.is_empty()) =>
                    {
                        ("TARGET_ONLY_IN_OTHER_SCOPE", None)
                    }
                    [] if unscoped_methods_by_symbol
                        .get(target)
                        .is_some_and(|methods| !methods.is_empty()) =>
                    {
                        ("TARGET_SCOPE_UNAVAILABLE", None)
                    }
                    [] => ("EXTERNAL_OR_UNRETAINED_TARGET", None),
                    _ => ("SAME_SCOPE_AMBIGUOUS", None),
                },
            };
            let (resolution, target_method, possible_target_method) = if association_ambiguous {
                (
                    "SOURCE_METHOD_ASSOCIATION_AMBIGUOUS",
                    None,
                    target_candidate,
                )
            } else {
                (target_resolution, target_candidate, None)
            };
            if let Some(target_method) = target_method {
                if !queued.contains(target_method.id.as_str()) {
                    queued.insert(target_method.id.as_str());
                    queue.push_back(target_method.id.as_str());
                }
            } else {
                frontiers.push(json!({
                    "kind": resolution,
                    "fromMethodId": if association_ambiguous { Value::Null } else { json!(method.id) },
                    "possibleFromMethodIds": candidate_method_ids,
                    "eventId": flow.id,
                    "target": flow.normalized["target"],
                    "possibleTargetMethodId": possible_target_method.map(|target| &target.id),
                }));
            }
            edges.push(json!({
                "id": edge_id,
                "kind": flow.normalized["kind"],
                "fromMethodId": if association_ambiguous { Value::Null } else { json!(method.id) },
                "possibleFromMethodIds": if association_ambiguous { json!(candidate_method_ids) } else { json!([method.id]) },
                "eventId": flow.id,
                "target": flow.normalized["target"],
                "targetMethodId": target_method.map(|target| &target.id),
                "possibleTargetMethodId": possible_target_method.map(|target| &target.id),
                "resolution": resolution,
                "sourceIds": flow.source_ids,
                "dependencyIds": [flow.id],
            }));
        }
        method_frontiers.sort_by_key(|frontier| frontier.to_string());
        for frontier in &method_frontiers {
            frontiers.push(frontier.clone());
        }
        methods.push(json!({
            "id": method.id,
            "service": method.service,
            "scope": scope,
            "symbol": method.symbol,
            "ownerIdentity": method.normalized["ownerIdentity"],
            "name": method.normalized["name"],
            "sourceIds": source_ids,
            "eventIds": event_ids,
            "eventAssociation": if association_ambiguous { "AMBIGUOUS_METHOD_SYMBOL" } else { "UNIQUE_METHOD_SYMBOL" },
            "candidateMethodIds": candidate_method_ids,
            "providerSlots": slots,
            "frontiers": method_frontiers,
            "observation": method,
        }));
    }

    frontiers.sort_by_key(|frontier| frontier.to_string());
    let evidence_gaps = !frontiers.is_empty();
    let mut unbound_events: Vec<_> = unbound_events.into_values().collect();
    unbound_events.sort_by(|left, right| {
        left["scope"]
            .as_str()
            .cmp(&right["scope"].as_str())
            .then_with(|| left["symbol"].as_str().cmp(&right["symbol"].as_str()))
            .then_with(|| {
                left["normalized"]["ordinal"]
                    .as_u64()
                    .cmp(&right["normalized"]["ordinal"].as_u64())
            })
            .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
    });
    let mut field_observations = BTreeMap::<String, Observation>::new();
    let mut overview = Vec::new();
    let mut inbound_edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &edges {
        if let (Some(target), Some(id)) = (edge["targetMethodId"].as_str(), edge["id"].as_str()) {
            inbound_edges.entry(target).or_default().push(id);
        }
    }
    for edge_ids in inbound_edges.values_mut() {
        edge_ids.sort_unstable();
    }
    for method_row in &methods {
        let Some(method_id) = method_row["id"].as_str() else {
            continue;
        };
        let Some(method) = evidence.observations.get(method_id) else {
            continue;
        };
        let callsite_edge_ids = inbound_edges
            .get(method_id)
            .into_iter()
            .flatten()
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>();
        if let Some(proof) = prove_accessor(evidence, method, &fields_by_scope_owner_name) {
            let mut accessor_source_ids =
                method.source_ids.iter().cloned().collect::<BTreeSet<_>>();
            accessor_source_ids.extend(proof.field.source_ids.iter().cloned());
            for source_id in &accessor_source_ids {
                if let Some(source) = evidence.sources.get(source_id) {
                    sources_by_id.insert(source_id.clone(), source.clone());
                }
            }
            field_observations.insert(proof.field.id.clone(), proof.field.clone());
            overview.push(json!({
                "kind": proof.kind,
                "methodId": method.id,
                "fieldId": proof.field.id,
                "fieldIdentity": {
                    "ownerIdentity": proof.field.normalized["ownerIdentity"],
                    "name": proof.field.normalized["name"],
                },
                "callsiteEdgeIds": callsite_edge_ids,
                "sourceIds": accessor_source_ids,
                "sourceRule": "ISOLATED_SOURCE_BODY_ONLY",
                "callsiteReceiverBinding": "UNKNOWN",
                "callsiteArgumentBinding": proof.formal_parameter.as_ref().map_or(
                    json!("UNKNOWN"),
                    |name| json!({"status":"UNKNOWN","formalParameterFromRetainedSource":name}),
                ),
            }));
        } else {
            overview.push(json!({
                "kind": "METHOD",
                "methodId": method.id,
                "callsiteEdgeIds": callsite_edge_ids,
            }));
        }
    }
    Ok(json!({
        "schema": GRAPH_SCHEMA,
        "snapshot": {
            "handle": snapshot,
            "inputDigest": input_digest,
            "service": evidence.service,
            "serviceRevision": evidence.revision,
            "serviceDigest": evidence.service_digest,
        },
        "rootMethodId": root.id,
        "methods": methods,
        "events": events,
        "unboundEvents": unbound_events,
        "edges": edges,
        "sources": sources_by_id,
        "fieldObservations": field_observations,
        "overview": {
            "schema": "codeclew-documentation-process-graph-overview/1.0",
            "projectionAuthority": "SOURCE_BODY_ACCESSOR_PROJECTION",
            "items": overview,
            "limitation": "This reversible overview only projects isolated retained accessor source bodies. It does not prove runtime purity, receiver identity, or complete runtime behavior; the full graph and source evidence remain in this artifact.",
        },
        "frontiers": frontiers,
        "coverage": {
            "knownReachableCollection": "EXHAUSTED",
            "runtimeGraphCompleteness": "NOT_ESTABLISHED",
            "evidenceGapsPresent": evidence_gaps,
            "interpretation": "All retained FLOW observations for collected methods are preserved. This artifact does not establish the complete runtime call graph.",
        },
    }))
}

struct AccessorProof<'a> {
    kind: &'static str,
    field: &'a Observation,
    formal_parameter: Option<String>,
}

fn prove_accessor<'a>(
    evidence: &'a ServiceEvidence,
    method: &Observation,
    fields_by_scope_owner_name: &BTreeMap<(&str, &str, &str), Vec<&'a Observation>>,
) -> Option<AccessorProof<'a>> {
    let owner = method.normalized["ownerIdentity"].as_str()?;
    let scope = exact_scope(method)?;
    let [source_id] = method.source_ids.as_slice() else {
        return None;
    };
    let source = evidence.sources.get(source_id)?;
    let (open, close) = super::source_steps::method_body(&source.text, &method.symbol)?;
    let body = comments_as_spaces(&source.text)[open + 1..close]
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let statement = body.strip_suffix(';')?;
    if statement.contains(';') {
        return None;
    }

    let (kind, field_name, formal_parameter) = if let Some(value) =
        statement.strip_prefix("return ")
    {
        if let Some(field_name) = value.strip_prefix("this.") {
            if !identifier(field_name) {
                return None;
            }
            ("READ", field_name, None)
        } else if identifier(value) && signature_parameters(&source.text, open, method)?.is_empty()
        {
            // A no-receiver expression is accepted only when the source
            // signature proves zero parameters, so it cannot be a parameter
            // shadowing an owner field.
            ("READ", value, None)
        } else {
            return None;
        }
    } else {
        let value = statement.strip_prefix("this.")?;
        let (field_name, parameter) = value.split_once(" = ")?;
        if !identifier(field_name) || !identifier(parameter) {
            return None;
        }
        let parameters = signature_parameters(&source.text, open, method)?;
        if !parameters.iter().any(|name| name == parameter) {
            return None;
        }
        ("WRITE", field_name, Some(parameter.to_owned()))
    };

    let [field] = fields_by_scope_owner_name
        .get(&(scope, owner, field_name))?
        .as_slice()
    else {
        return None;
    };
    Some(AccessorProof {
        kind,
        field,
        formal_parameter,
    })
}

fn comments_as_spaces(source: &str) -> String {
    let (_, comments) = super::source_steps::lexical_masks(source);
    let mut bytes = source.as_bytes().to_vec();
    for (index, is_comment) in comments.into_iter().enumerate() {
        if is_comment && bytes[index] != b'\n' {
            bytes[index] = b' ';
        }
    }
    String::from_utf8(bytes).unwrap_or_default()
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || matches!(first, '_' | '$'))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
}

fn signature_parameters(
    source: &str,
    body_open: usize,
    method: &Observation,
) -> Option<Vec<String>> {
    let method_name = method.normalized["name"].as_str()?;
    if !identifier(method_name) {
        return None;
    }
    let cleaned = comments_as_spaces(source);
    let bytes = cleaned.as_bytes();
    let (code, _) = super::source_steps::lexical_masks(&cleaned);
    let name = method_name.as_bytes();
    let mut candidates = Vec::new();
    for start in 0..body_open.saturating_sub(name.len()).saturating_add(1) {
        let end_name = start + name.len();
        if end_name > body_open
            || &bytes[start..end_name] != name
            || !code[start..end_name].iter().all(|b| *b)
        {
            continue;
        }
        let boundary = |byte: Option<&u8>| {
            byte.is_none_or(|byte| !(byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'$')))
        };
        if !boundary(start.checked_sub(1).and_then(|index| bytes.get(index)))
            || !boundary(bytes.get(end_name))
        {
            continue;
        }
        let mut open = end_name;
        while open < body_open && bytes[open].is_ascii_whitespace() {
            open += 1;
        }
        if bytes.get(open) != Some(&b'(') || !code[open] {
            continue;
        }
        let Some(close) = matching_paren(&bytes[..body_open], &code[..body_open], open) else {
            continue;
        };
        candidates.push(&cleaned[open + 1..close]);
    }
    let [parameters] = candidates.as_slice() else {
        return None;
    };
    let parameters = parameters.trim();
    if parameters.is_empty() {
        return Some(Vec::new());
    }
    let mut names = Vec::new();
    for parameter in parameters.split(',') {
        let words = parameter.split_whitespace().collect::<Vec<_>>();
        let name = words.last().copied()?;
        if words.len() < 2 || !identifier(name) {
            return None;
        }
        names.push(name.to_owned());
    }
    Some(names)
}

fn matching_paren(bytes: &[u8], code: &[bool], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in open..bytes.len() {
        if !code[index] {
            continue;
        }
        match bytes[index] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_evidence() -> ServiceEvidence {
        serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0",
            "service":"svc",
            "revision":"rev",
            "serviceDigest":"digest",
            "extractor":"test",
            "runtimeMode":"TEST",
            "coverage":"PARTIAL",
            "boundaries":[],
            "entrypoints":[],
            "observations":{},
            "sources":{},
            "contracts":{}
        }))
        .unwrap()
    }

    fn add_method_in_scope(
        evidence: &mut ServiceEvidence,
        id: &str,
        scope: &str,
        symbol: &str,
        events: Vec<Value>,
    ) {
        let mut normalized = json!({
            "declarationKind":"METHOD",
            "scope":scope,
            "name":symbol,
            "ownerIdentity":"class:Worker",
            "documentation":{"events":events,"parameterTypes":[]}
        });
        for (ordinal, mut event) in events.into_iter().enumerate() {
            event["ordinal"] = json!(ordinal);
            event["scope"] = json!(scope);
            let event_id = format!("{id}-event-{ordinal}");
            evidence.observations.insert(
                event_id.clone(),
                Observation {
                    id: event_id,
                    kind: "FLOW".into(),
                    service: "svc".into(),
                    symbol: symbol.into(),
                    normalized: event,
                    digest: "digest".into(),
                    source_ids: vec![],
                },
            );
        }
        normalized["documentation"]["parameterTypes"] = json!([]);
        evidence.observations.insert(
            id.into(),
            Observation {
                id: id.into(),
                kind: "SYMBOL".into(),
                service: "svc".into(),
                symbol: symbol.into(),
                normalized,
                digest: "digest".into(),
                source_ids: vec![],
            },
        );
    }

    fn add_method(evidence: &mut ServiceEvidence, id: &str, symbol: &str, events: Vec<Value>) {
        add_method_in_scope(evidence, id, ":main", symbol, events);
    }

    fn add_source(evidence: &mut ServiceEvidence, id: &str, text: &str) {
        evidence.sources.insert(
            id.into(),
            Source {
                id: id.into(),
                service: "svc".into(),
                revision: "rev".into(),
                file: "Worker.java".into(),
                start_line: 1,
                end_line: text.lines().count() as u64,
                text: text.into(),
                text_digest: "text-digest".into(),
                evidence_digest: "evidence-digest".into(),
                authority: "RETAINED_SOURCE".into(),
                occurrence: None,
                url: None,
            },
        );
    }

    fn add_field(evidence: &mut ServiceEvidence, id: &str, name: &str, source_id: &str) {
        evidence.observations.insert(
            id.into(),
            Observation {
                id: id.into(),
                kind: "SYMBOL".into(),
                service: "svc".into(),
                symbol: format!("field:class:Worker#{name}:I"),
                normalized: json!({
                    "declarationKind":"FIELD",
                    "scope":":main",
                    "ownerIdentity":"class:Worker",
                    "name":name,
                }),
                digest: "digest".into(),
                source_ids: vec![source_id.into()],
            },
        );
    }

    fn field_index(evidence: &ServiceEvidence) -> BTreeMap<(&str, &str, &str), Vec<&Observation>> {
        let mut index = BTreeMap::new();
        for observation in evidence.observations.values().filter(|observation| {
            observation.kind == "SYMBOL" && observation.normalized["declarationKind"] == "FIELD"
        }) {
            if let (Some(scope), Some(owner), Some(name)) = (
                observation.normalized["scope"].as_str(),
                observation.normalized["ownerIdentity"].as_str(),
                observation.normalized["name"].as_str(),
            ) {
                index
                    .entry((scope, owner, name))
                    .or_insert_with(Vec::new)
                    .push(observation);
            }
        }
        index
    }

    #[test]
    fn retains_every_root_event_past_legacy_sixty_four_step_limit() {
        let mut evidence = empty_evidence();
        let events = (0..70)
            .map(|ordinal| {
                if ordinal == 69 {
                    json!({"kind":"CALL","target":"method:Worker#helper()V","detail":"last call"})
                } else {
                    json!({"kind":"IF","detail":ordinal,"groups":[{"arm":ordinal}]})
                }
            })
            .collect::<Vec<_>>();
        add_method(&mut evidence, "root-id", "method:Worker#root()V", events);
        add_method(
            &mut evidence,
            "helper-id",
            "method:Worker#helper()V",
            vec![json!({"kind":"RETURN","detail":"helper returns"})],
        );

        let root = evidence.observations["root-id"].clone();
        let graph = collect(&evidence, &root, "snapshot-handle", "input-digest").unwrap();
        let events = graph["events"].as_array().unwrap();
        assert_eq!(events.len(), 71);
        assert_eq!(events[0]["normalized"]["ordinal"], 0);
        assert_eq!(events[69]["normalized"]["ordinal"], 69);
        assert_eq!(events[69]["normalized"]["detail"], "last call");
        assert_eq!(events[70]["normalized"]["ordinal"], 0);
        assert_eq!(graph["methods"].as_array().unwrap().len(), 2);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
        assert_eq!(graph["edges"][0]["targetMethodId"], "helper-id");
        assert_eq!(graph["coverage"]["knownReachableCollection"], "EXHAUSTED");
        assert_eq!(
            graph["coverage"]["runtimeGraphCompleteness"],
            "NOT_ESTABLISHED"
        );
    }

    #[test]
    fn preserves_repeated_callsites_and_terminates_cycles_with_one_method_body() {
        let mut evidence = empty_evidence();
        add_method(
            &mut evidence,
            "root-id",
            "method:Worker#root()V",
            vec![
                json!({"kind":"CALL","target":"method:Worker#a()V"}),
                json!({"kind":"CALL","target":"method:Worker#a()V"}),
            ],
        );
        add_method(
            &mut evidence,
            "a-id",
            "method:Worker#a()V",
            vec![json!({"kind":"CALL","target":"method:Worker#b()V"})],
        );
        add_method(
            &mut evidence,
            "b-id",
            "method:Worker#b()V",
            vec![json!({"kind":"CALL","target":"method:Worker#c()V"})],
        );
        add_method(
            &mut evidence,
            "c-id",
            "method:Worker#c()V",
            vec![json!({"kind":"CALL","target":"method:Worker#d()V"})],
        );
        add_method(
            &mut evidence,
            "d-id",
            "method:Worker#d()V",
            vec![json!({"kind":"CALL","target":"method:Worker#e()V"})],
        );
        add_method(
            &mut evidence,
            "e-id",
            "method:Worker#e()V",
            vec![json!({"kind":"CALL","target":"method:Worker#b()V"})],
        );

        let root = evidence.observations["root-id"].clone();
        let graph = collect(&evidence, &root, "snapshot-handle", "input-digest").unwrap();
        assert_eq!(graph["methods"].as_array().unwrap().len(), 6);
        assert_eq!(graph["events"].as_array().unwrap().len(), 7);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 7);
        assert_eq!(graph["edges"][0]["targetMethodId"], "a-id");
        assert_eq!(graph["edges"][1]["targetMethodId"], "a-id");
        assert_ne!(graph["edges"][0]["id"], graph["edges"][1]["id"]);
        assert_eq!(graph["edges"][6]["fromMethodId"], "e-id");
        assert_eq!(graph["edges"][6]["targetMethodId"], "b-id");
    }

    #[test]
    fn same_scope_duplicate_symbol_keeps_flow_unbound_from_exact_root() {
        let mut evidence = empty_evidence();
        let symbol = "method:Worker#root()V";
        add_method(
            &mut evidence,
            "root-id",
            symbol,
            vec![json!({"kind":"CALL","target":"method:Worker#target()V"})],
        );
        add_method(
            &mut evidence,
            "duplicate-id",
            symbol,
            vec![json!({"kind":"CALL","target":"method:Worker#target()V"})],
        );
        add_method(
            &mut evidence,
            "target-id",
            "method:Worker#target()V",
            vec![],
        );

        let root = evidence.observations["root-id"].clone();
        let graph = collect(&evidence, &root, "snapshot-handle", "input-digest").unwrap();
        let root_row = graph["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|method| method["id"] == "root-id")
            .unwrap();
        assert_eq!(root_row["eventAssociation"], "AMBIGUOUS_METHOD_SYMBOL");
        assert_eq!(root_row["eventIds"], json!([]));
        assert_eq!(graph["methods"].as_array().unwrap().len(), 1);
        assert_eq!(graph["events"].as_array().unwrap().len(), 0);
        assert_eq!(graph["unboundEvents"].as_array().unwrap().len(), 2);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 2);
        assert_eq!(
            graph["edges"][0]["resolution"],
            "SOURCE_METHOD_ASSOCIATION_AMBIGUOUS"
        );
        assert!(graph["edges"][0]["targetMethodId"].is_null());
        assert_eq!(graph["edges"][0]["possibleTargetMethodId"], "target-id");
        assert!(graph["edges"].as_array().unwrap().iter().all(|edge| {
            edge["fromMethodId"].is_null()
                && edge["possibleFromMethodIds"]
                    .as_array()
                    .is_some_and(|ids| ids.len() == 2)
        }));
        assert!(
            graph["frontiers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|frontier| frontier["kind"] == "FLOW_METHOD_ASSOCIATION_AMBIGUOUS")
        );
    }

    #[test]
    fn keeps_scope_ambiguity_and_missing_provider_slots_as_frontiers() {
        let mut evidence = empty_evidence();
        add_method(
            &mut evidence,
            "root-id",
            "method:Worker#root()V",
            vec![
                json!({"kind":"CALL","target":"method:Worker#other()V"}),
                json!({"kind":"CALL","target":"method:Worker#ambiguous()V"}),
                json!({"kind":"CALL","target":"library:External#run()V"}),
                json!({"kind":"CALL","target":"method:Worker#unscoped()V"}),
            ],
        );
        add_method_in_scope(
            &mut evidence,
            "other-scope-id",
            ":test",
            "method:Worker#other()V",
            vec![],
        );
        add_method(
            &mut evidence,
            "ambiguous-a",
            "method:Worker#ambiguous()V",
            vec![],
        );
        add_method(
            &mut evidence,
            "ambiguous-b",
            "method:Worker#ambiguous()V",
            vec![],
        );
        add_method_in_scope(
            &mut evidence,
            "unscoped-method-id",
            "",
            "method:Worker#unscoped()V",
            vec![],
        );
        let root = evidence.observations["root-id"].clone();
        let mut graph = collect(&evidence, &root, "snapshot-handle", "input-digest").unwrap();
        assert_eq!(graph["methods"].as_array().unwrap().len(), 1);
        assert_eq!(
            graph["edges"][0]["resolution"],
            "TARGET_ONLY_IN_OTHER_SCOPE"
        );
        assert_eq!(graph["edges"][1]["resolution"], "SAME_SCOPE_AMBIGUOUS");
        assert_eq!(
            graph["edges"][2]["resolution"],
            "EXTERNAL_OR_UNRETAINED_TARGET"
        );
        assert_eq!(graph["edges"][3]["resolution"], "TARGET_SCOPE_UNAVAILABLE");

        let mut incomplete = empty_evidence();
        add_method(
            &mut incomplete,
            "incomplete-root",
            "method:Worker#incomplete()V",
            vec![
                json!({"kind":"IF","condition":"ready"}),
                json!({"kind":"CALL","target":"library:External#run()V"}),
            ],
        );
        incomplete.observations.remove("incomplete-root-event-1");
        let root = incomplete.observations["incomplete-root"].clone();
        graph = collect(&incomplete, &root, "snapshot-handle", "input-digest").unwrap();
        assert_eq!(graph["events"].as_array().unwrap().len(), 1);
        assert_eq!(
            graph["methods"][0]["providerSlots"][1]["evidence"]["kind"],
            "EXPECTED_FLOW_SLOT_MISSING"
        );
        assert_eq!(
            graph["methods"][0]["providerSlots"][1]["providerEvent"]["kind"],
            "CALL"
        );
    }

    #[test]
    fn projects_only_exact_trivial_accessor_source_bodies() {
        let mut evidence = empty_evidence();
        let getter_source = "class Worker { int value; int readValue() { return this.value; } }";
        add_source(&mut evidence, "getter-source", getter_source);
        add_method(
            &mut evidence,
            "caller-id",
            "method:Worker#caller()V",
            vec![json!({"kind":"CALL","target":"method:Worker#readValue()I"})],
        );
        add_method(
            &mut evidence,
            "getter-id",
            "method:Worker#readValue()I",
            vec![],
        );
        evidence
            .observations
            .get_mut("getter-id")
            .unwrap()
            .source_ids = vec!["getter-source".into()];
        add_field(&mut evidence, "field-id", "value", "getter-source");
        let getter = evidence.observations["getter-id"].clone();
        assert_eq!(
            prove_accessor(&evidence, &getter, &field_index(&evidence))
                .unwrap()
                .kind,
            "READ"
        );
        let root = evidence.observations["caller-id"].clone();
        let graph = collect(&evidence, &root, "snapshot-handle", "input-digest").unwrap();
        let overview_item = graph["overview"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["methodId"] == "getter-id")
            .unwrap();
        assert_eq!(overview_item["kind"], "READ");
        assert_eq!(overview_item["fieldId"], "field-id");
        assert_eq!(
            overview_item["callsiteEdgeIds"],
            json!(["edge:caller-id-event-0"])
        );
        assert_eq!(overview_item["callsiteReceiverBinding"], "UNKNOWN");
        assert_eq!(graph["sources"]["getter-source"]["text"], getter_source);
        assert_eq!(
            graph["fieldObservations"]["field-id"]["normalized"]["name"],
            "value"
        );

        let mut setter_evidence = empty_evidence();
        let setter_source =
            "class Worker { int value; void setValue(int next) { this.value = next; } }";
        add_source(&mut setter_evidence, "setter-source", setter_source);
        add_method(
            &mut setter_evidence,
            "setter-id",
            "method:Worker#setValue(I)V",
            vec![],
        );
        {
            let setter = setter_evidence.observations.get_mut("setter-id").unwrap();
            setter.normalized["name"] = json!("setValue");
            setter.source_ids = vec!["setter-source".into()];
        }
        add_field(
            &mut setter_evidence,
            "field-setter-id",
            "value",
            "setter-source",
        );
        let setter = setter_evidence.observations["setter-id"].clone();
        let proof =
            prove_accessor(&setter_evidence, &setter, &field_index(&setter_evidence)).unwrap();
        assert_eq!(proof.kind, "WRITE");
        assert_eq!(proof.formal_parameter.as_deref(), Some("next"));

        let mut deceptive_evidence = empty_evidence();
        let deceptive_source =
            "class Worker { int value; int readValue() { audit(); return this.value; } }";
        add_source(
            &mut deceptive_evidence,
            "deceptive-source",
            deceptive_source,
        );
        add_method(
            &mut deceptive_evidence,
            "deceptive-id",
            "method:Worker#readValue()I",
            vec![],
        );
        {
            let deceptive = deceptive_evidence
                .observations
                .get_mut("deceptive-id")
                .unwrap();
            deceptive.normalized["name"] = json!("readValue");
            deceptive.source_ids = vec!["deceptive-source".into()];
        }
        add_field(
            &mut deceptive_evidence,
            "field-deceptive-id",
            "value",
            "deceptive-source",
        );
        let deceptive = deceptive_evidence.observations["deceptive-id"].clone();
        assert!(
            prove_accessor(
                &deceptive_evidence,
                &deceptive,
                &field_index(&deceptive_evidence)
            )
            .is_none()
        );

        let mut unqualified_evidence = empty_evidence();
        let unqualified_source = "class Worker { int value; int readValue() { return value; } }";
        add_source(
            &mut unqualified_evidence,
            "unqualified-source",
            unqualified_source,
        );
        add_method(
            &mut unqualified_evidence,
            "unqualified-id",
            "method:Worker#readValue()I",
            vec![],
        );
        unqualified_evidence
            .observations
            .get_mut("unqualified-id")
            .unwrap()
            .source_ids = vec!["unqualified-source".into()];
        unqualified_evidence
            .observations
            .get_mut("unqualified-id")
            .unwrap()
            .normalized["name"] = json!("readValue");
        add_field(
            &mut unqualified_evidence,
            "unqualified-field",
            "value",
            "unqualified-source",
        );
        let unqualified = unqualified_evidence.observations["unqualified-id"].clone();
        assert_eq!(
            prove_accessor(
                &unqualified_evidence,
                &unqualified,
                &field_index(&unqualified_evidence)
            )
            .unwrap()
            .kind,
            "READ"
        );

        let mut shadowed_evidence = empty_evidence();
        let shadowed_source =
            "class Worker { int value; int readValue(int value) { return value; } }";
        add_source(&mut shadowed_evidence, "shadowed-source", shadowed_source);
        add_method(
            &mut shadowed_evidence,
            "shadowed-id",
            "method:Worker#readValue(I)I",
            vec![],
        );
        shadowed_evidence
            .observations
            .get_mut("shadowed-id")
            .unwrap()
            .source_ids = vec!["shadowed-source".into()];
        shadowed_evidence
            .observations
            .get_mut("shadowed-id")
            .unwrap()
            .normalized["name"] = json!("readValue");
        add_field(
            &mut shadowed_evidence,
            "shadowed-field",
            "value",
            "shadowed-source",
        );
        let shadowed = shadowed_evidence.observations["shadowed-id"].clone();
        assert!(
            prove_accessor(
                &shadowed_evidence,
                &shadowed,
                &field_index(&shadowed_evidence)
            )
            .is_none()
        );
    }

    #[test]
    fn command_saves_full_artifact_and_returns_only_a_compact_summary() {
        use crate::documentation::{
            check::{Check, SourceInputs},
            model::Service,
            store::Repository,
        };
        use std::collections::BTreeMap;

        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Process graph test").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0",
            "id":"svc",
            "title":"Service",
            "repositoryId":"svc",
            "repository":"https://example.invalid/svc",
            "language":"java",
            "profile":"source-syntax",
            "targetRef":"main",
            "source":{"roots":["src"],"dialect":"17"}
        }))
        .unwrap();
        repo.service_add(service.clone(), Some(&repo.input_digest().unwrap()))
            .unwrap();

        let mut evidence = empty_evidence();
        let source_text = "class Worker { void run() { return; } }";
        add_source(&mut evidence, "source-run", source_text);
        add_method(
            &mut evidence,
            "root-observation",
            "method:Worker#run()V",
            vec![json!({"kind":"RETURN","detail":"exit"})],
        );
        evidence
            .observations
            .get_mut("root-observation")
            .unwrap()
            .source_ids = vec!["source-run".into()];
        evidence.service_digest = crate::documentation::digest(&service).unwrap();
        let input_digest = repo.input_digest().unwrap();
        let checked = Check {
            schema: "codeclew-documentation-check/1.0".into(),
            input_digest: input_digest.clone(),
            context_digest: "sha256:test-context".into(),
            source_inputs: Some(SourceInputs {
                schema: crate::documentation::check::SOURCE_INPUTS_SCHEMA.into(),
                input_digest,
                inputs: repo.inputs().unwrap(),
                selected_services: BTreeSet::from(["svc".into()]),
                retained_services: BTreeSet::new(),
            }),
            composition: None,
            services: BTreeMap::from([("svc".into(), evidence.clone())]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: evidence.observations.clone(),
        };
        let snapshot = checked.save_snapshot(&repo).unwrap();
        let output = temporary.path().join("artifacts/process-graph.json");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, b"old artifact").unwrap();

        let summary = run(&repo, "svc", "root-observation", &snapshot, &output).unwrap();
        let artifact: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        let recorded_digest = artifact["artifactDigest"].as_str().unwrap().to_owned();
        let mut digest_input = artifact.clone();
        digest_input
            .as_object_mut()
            .unwrap()
            .remove("artifactDigest");
        assert_eq!(digest(&digest_input).unwrap(), recorded_digest);
        assert_eq!(artifact["sources"]["source-run"]["text"], source_text);
        assert_eq!(artifact["events"].as_array().unwrap().len(), 1);
        assert_eq!(
            artifact["coverage"]["runtimeGraphCompleteness"],
            "NOT_ESTABLISHED"
        );
        assert_eq!(summary["status"], "SAVED");
        assert_eq!(summary["artifactDigest"], recorded_digest);
        assert_eq!(summary["eventCount"], 1);
        assert!(summary.get("methods").is_none());
        assert!(summary.get("complete").is_none());
    }
}
