//! Bounded, nonduplicating author context for one captured Java HTTP endpoint.
//!
//! This is a deterministic selector over immutable Work evidence. The packet
//! row is navigation only; provider declarations, FLOW, CALL_RELATION and
//! retained SOURCE records remain the only factual rows.

use super::{
    invalid,
    model::{Observation, ServiceEvidence, Source},
    source_steps,
    work::Work,
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const PROFILE: &str = "endpoint-context-v3";

const JAVA_COMPILER_FACT_SCHEMA: &str = "codeclew-java-compiler-fact/1.0";
// Includes the endpoint root; descendants must have a parseable retained body.
const MAX_CALLABLES: usize = 12;
const MAX_SOURCE_BYTES: usize = 48 * 1024;
const MAX_DTO_FIELDS: usize = 64;
const MAX_REFERENCED_FIELDS: usize = 8;
const MAX_DEPTH: usize = 4;
const MAX_GRAPH_EDGES: usize = 256;
const MAX_INLINE_CANDIDATE_EDGES: usize = 16;
const MAX_GAP_EXAMPLES: usize = 8;
const MAX_GAP_EXAMPLE_BYTES: usize = 512;

type MethodKey = (String, String);
#[derive(Default)]
struct Gaps {
    counts: BTreeMap<String, usize>,
    examples: BTreeMap<String, Vec<Value>>,
}

impl Gaps {
    fn add(&mut self, code: &str, mut example: Value) {
        *self.counts.entry(code.to_owned()).or_default() += 1;
        let values = self.examples.entry(code.to_owned()).or_default();
        if values.len() < MAX_GAP_EXAMPLES {
            let size = serde_json::to_vec(&example).map_or(usize::MAX, |bytes| bytes.len());
            if size > MAX_GAP_EXAMPLE_BYTES {
                example = json!({"detail":"EXAMPLE_OMITTED_BY_SIZE_BOUND","bytes":size});
            }
            values.push(example);
        }
    }

    fn rows(self) -> Vec<Value> {
        self.counts
            .into_iter()
            .map(|(code, count)| {
                json!({
                    "code":code,
                    "count":count,
                    "examples":self.examples.get(&code).cloned().unwrap_or_default(),
                    "exampleLimit":MAX_GAP_EXAMPLES
                })
            })
            .collect()
    }
}

#[derive(Clone)]
struct Callable<'a> {
    identity: String,
    scope: String,
    owner: String,
    name: String,
    observation: &'a Observation,
}

struct DeclarationIndexes<'a> {
    methods_by_identity_scope: BTreeMap<MethodKey, Vec<&'a Observation>>,
    methods_by_owner_name: BTreeMap<(String, String), Vec<&'a Observation>>,
    method_scopes_by_identity: BTreeMap<String, BTreeSet<String>>,
    types_by_identity_scope: BTreeMap<MethodKey, Vec<&'a Observation>>,
    fields_by_owner_name: BTreeMap<(String, String), Vec<&'a Observation>>,
    fields_by_owner_scope: BTreeMap<(String, String), Vec<&'a Observation>>,
    outgoing_by_identity_scope: BTreeMap<MethodKey, OutgoingFacts<'a>>,
}

#[derive(Default)]
struct OutgoingFacts<'a> {
    flows: Vec<&'a Observation>,
    relations: Vec<&'a Observation>,
}

#[derive(Clone)]
struct Edge {
    from: String,
    target: Option<String>,
    scope: String,
    kind: String,
    authority: String,
    fact_id: Option<String>,
    source_id: Option<String>,
}

struct FinishRows<'a> {
    work: &'a Work,
    evidence: &'a ServiceEvidence,
    entrypoint_id: &'a str,
    entry: Option<&'a super::model::Entrypoint>,
    dependencies: BTreeMap<String, &'a Observation>,
    sources: BTreeMap<String, &'a Source>,
    nodes: Vec<Value>,
    node_keys: Vec<MethodKey>,
    edges: Vec<Edge>,
    dto_groups: Vec<Value>,
    owner_groups: Vec<Value>,
    gaps: Gaps,
    body_source_bytes: usize,
    field_source_bytes: usize,
    unique_source_bytes: usize,
}

/// Return the initial packet membership for the explicit endpoint profile.
/// Follow-up Work reads remain the ordinary reference/query/read-part paths.
pub(super) fn profile_rows(work: &Work) -> Result<Vec<Value>, ClewError> {
    let (service, entrypoint_id) = work
        .subject
        .strip_prefix("service:")
        .and_then(|service| {
            work.request
                .entrypoint
                .as_deref()
                .map(|entrypoint| (service, entrypoint))
        })
        .ok_or_else(|| invalid("endpoint-context-v3 requires a service HTTP endpoint"))?;
    let evidence = work
        .checked
        .services
        .get(service)
        .ok_or_else(|| invalid("endpoint-context-v3 service evidence is unavailable"))?;
    let entry = unique_entrypoint(evidence, entrypoint_id);
    let mut gaps = Gaps::default();
    let mut dependencies = BTreeMap::<String, &Observation>::new();
    let mut sources = BTreeMap::<String, &Source>::new();
    let mut nodes = Vec::<Value>::new();
    let mut edges = Vec::<Edge>::new();
    let mut direct_types = BTreeMap::<String, BTreeSet<String>>::new();
    let mut dto_field_ids = Vec::<String>::new();
    let mut owner_field_ids = BTreeSet::<String>::new();
    let mut dto_source_bytes = 0usize;
    let mut body_source_bytes = 0usize;
    let mut unique_source_bytes = 0usize;
    let mut counted_source_ids = BTreeSet::<String>::new();
    let indexes = declaration_indexes(evidence, work, service);

    let mut root_callable = None;
    let mut root_scope = None;
    let mut root_descriptor = None;

    if let Some(entry) = entry {
        let roots: Vec<_> = evidence
            .observations
            .values()
            .filter(|observation| {
                work.influence.contains_key(&observation.id)
                    && observation.service == service
                    && observation.kind == "SYMBOL"
                    && observation.symbol == entry.symbol
                    && observation.normalized["schema"] == JAVA_COMPILER_FACT_SCHEMA
                    && observation.normalized["declarationKind"] == "METHOD"
                    && observation.normalized["symbolIdentity"] == entry.symbol
            })
            .collect();
        match roots.as_slice() {
            [root] => {
                root_scope = exact_scope(&root.normalized["scope"]).map(str::to_owned);
                root_descriptor = root.normalized["jvmDescriptor"].as_str().map(str::to_owned);
                let owner = root.normalized["ownerIdentity"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let name = root.normalized["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let scope = root_scope.clone().unwrap_or_default();
                let callable = Callable {
                    identity: root.symbol.clone(),
                    scope,
                    owner,
                    name,
                    observation: root,
                };
                root_callable = Some(callable.clone());
                if root_scope.is_none() {
                    gaps.add(
                        "ENDPOINT_COMPILATION_SCOPE_UNAVAILABLE",
                        json!({"symbol":entry.symbol}),
                    );
                }
            }
            [] => gaps.add(
                "ENDPOINT_DECLARATION_UNAVAILABLE",
                json!({"symbol":entry.symbol}),
            ),
            _ => gaps.add(
                "ENDPOINT_DECLARATION_AMBIGUOUS",
                json!({"symbol":entry.symbol,"count":roots.len()}),
            ),
        }
    } else {
        gaps.add(
            "HTTP_ENDPOINT_UNAVAILABLE",
            json!({"entrypoint":entrypoint_id}),
        );
    }

    if let Some(root) = root_callable.as_ref() {
        if let Some(descriptor) = root_descriptor.as_deref() {
            match descriptor_object_roles(descriptor) {
                Ok((inputs, outputs)) => {
                    for identity in inputs {
                        direct_types
                            .entry(identity)
                            .or_default()
                            .insert("INPUT".into());
                    }
                    for identity in outputs {
                        direct_types
                            .entry(identity)
                            .or_default()
                            .insert("OUTPUT".into());
                    }
                }
                Err(()) => gaps.add(
                    "ENDPOINT_DESCRIPTOR_UNSUPPORTED",
                    json!({"symbol":root.identity}),
                ),
            }
        } else {
            gaps.add(
                "ENDPOINT_DESCRIPTOR_UNAVAILABLE",
                json!({"symbol":root.identity}),
            );
        }
    }

    let mut queue = VecDeque::<(Callable<'_>, usize, Option<String>)>::new();
    if let Some(root) = root_callable.clone() {
        queue.push_back((root, 0, None));
    }
    let mut visited = BTreeSet::<MethodKey>::new();
    let mut scheduled = BTreeSet::<MethodKey>::new();
    if let Some(root) = root_callable.as_ref() {
        scheduled.insert((root.identity.clone(), root.scope.clone()));
    }
    let mut node_keys = Vec::<MethodKey>::new();
    let mut owner_field_candidates = BTreeSet::<String>::new();
    let mut graph_edge_count = 0usize;

    while let Some((callable, depth, reserved_body_source_id)) = queue.pop_front() {
        let key = (callable.identity.clone(), callable.scope.clone());
        if !visited.insert(key.clone()) {
            continue;
        }
        if visited.len() > MAX_CALLABLES {
            gaps.add(
                "CALLABLE_BODY_LIMIT",
                json!({"symbol":callable.identity,"limit":MAX_CALLABLES}),
            );
            continue;
        }
        let body_source_id = if let Some(source_id) = reserved_body_source_id {
            Some(source_id)
        } else {
            match method_body_source(evidence, &callable) {
                Ok((source_id, source)) => {
                    if counted_source_ids.contains(&source_id) {
                        Some(source_id)
                    } else if unique_source_bytes.saturating_add(source.text.len())
                        > MAX_SOURCE_BYTES
                    {
                        gaps.add(
                            "UNIQUE_SOURCE_BYTE_LIMIT",
                            json!({"symbol":callable.identity,"bytes":source.text.len(),"limit":MAX_SOURCE_BYTES}),
                        );
                        None
                    } else {
                        body_source_bytes += source.text.len();
                        unique_source_bytes += source.text.len();
                        counted_source_ids.insert(source_id.clone());
                        sources.insert(source_id.clone(), source);
                        Some(source_id)
                    }
                }
                Err(code) => {
                    gaps.add(code, json!({"symbol":callable.identity}));
                    None
                }
            }
        };
        let body_range = body_source_id
            .as_deref()
            .and_then(|source_id| sources.get(source_id))
            .and_then(|source| source_steps::method_body(&source.text, &callable.identity));
        let node_id = format!("m{}", nodes.len());
        nodes.push(json!({
            "id":node_id,
            "symbolIdentity":callable.identity,
            "scope":callable.scope,
            "bodyReference":body_source_id.as_deref().and_then(|id| work_reference(work,"SOURCE",id)),
        }));
        node_keys.push(key.clone());

        let outgoing = indexes.outgoing_by_identity_scope.get(&key);
        let caller_flows = outgoing.map_or(&[][..], |facts| facts.flows.as_slice());
        let caller_relations = outgoing.map_or(&[][..], |facts| facts.relations.as_slice());

        let mut provider_targets = BTreeSet::<String>::new();
        for observation in caller_flows.iter().copied() {
            dependencies.insert(observation.id.clone(), observation);
            if !matches!(
                observation.normalized["kind"].as_str(),
                Some("CALL" | "CONSTRUCT")
            ) {
                continue;
            }
            let target = observation.normalized["target"]
                .as_str()
                .filter(|v| !v.is_empty());
            let target_scope = exact_scope(&observation.normalized["scope"]);
            let Some(target) = target else {
                gaps.add(
                    "FLOW_TARGET_IDENTITY_UNAVAILABLE",
                    json!({"from":callable.identity,"factId":observation.id}),
                );
                continue;
            };
            if target_scope != Some(callable.scope.as_str()) {
                gaps.add(
                    "FLOW_TARGET_SCOPE_UNAVAILABLE",
                    json!({"from":callable.identity,"target":target,"factId":observation.id}),
                );
                continue;
            }
            let source_reference = body_source_id
                .as_deref()
                .and_then(|body_id| owning_body_source_reference(evidence, observation, body_id));
            if source_reference.is_none() {
                gaps.add(
                    "FLOW_SOURCE_BODY_LINK_UNAVAILABLE",
                    json!({"from":callable.identity,"factId":observation.id}),
                );
            }
            add_edge(
                &mut edges,
                &mut graph_edge_count,
                Edge {
                    from: callable.identity.clone(),
                    target: Some(target.to_owned()),
                    scope: callable.scope.clone(),
                    kind: observation.normalized["kind"]
                        .as_str()
                        .unwrap_or("CALL")
                        .to_owned(),
                    authority: "RETAINED_FLOW_TARGET".into(),
                    fact_id: Some(observation.id.clone()),
                    source_id: source_reference,
                },
                &mut gaps,
            );
            provider_targets.insert(target.to_owned());
        }

        for observation in caller_relations.iter().copied() {
            dependencies.insert(observation.id.clone(), observation);
            let relation_kind = observation.normalized["relationKind"].as_str();
            let target = observation.normalized["targetIdentity"]
                .as_str()
                .filter(|v| !v.is_empty());
            let source_bound = relation_source_bound(evidence, observation, service);
            let reason = if observation.normalized["sourceIdentity"].as_str()
                != Some(callable.identity.as_str())
            {
                Some("CALL_RELATION_OWNER_UNAVAILABLE")
            } else if exact_scope(&observation.normalized["scope"]) != Some(callable.scope.as_str())
            {
                Some("CALL_RELATION_SCOPE_UNAVAILABLE")
            } else if !matches!(relation_kind, Some("CALLS" | "CONSTRUCTS"))
                || observation.normalized["resolution"] != "COMPILER_EXACT"
            {
                Some("CALL_RELATION_RESOLUTION_UNVERIFIED")
            } else if !source_bound {
                Some("CALL_SITE_SOURCE_UNAVAILABLE")
            } else if target.is_none() {
                Some("CALL_TARGET_IDENTITY_UNAVAILABLE")
            } else {
                None
            };
            if let Some(reason) = reason {
                gaps.add(
                    reason,
                    json!({"from":callable.identity,"factId":observation.id,"target":target}),
                );
                continue;
            }
            let target = target.unwrap();
            let source_reference = body_source_id
                .as_deref()
                .and_then(|body_id| owning_body_source_reference(evidence, observation, body_id));
            if source_reference.is_none() {
                gaps.add(
                    "CALL_SITE_SOURCE_NOT_CONTAINED_IN_METHOD_BODY",
                    json!({"from":callable.identity,"factId":observation.id}),
                );
            }
            provider_targets.insert(target.to_owned());
            add_edge(
                &mut edges,
                &mut graph_edge_count,
                Edge {
                    from: callable.identity.clone(),
                    target: Some(target.to_owned()),
                    scope: callable.scope.clone(),
                    kind: relation_kind.unwrap().to_owned(),
                    authority: "COMPILER_EXACT_CALL_RELATION".into(),
                    fact_id: Some(observation.id.clone()),
                    source_id: source_reference,
                },
                &mut gaps,
            );
        }

        if let Some(source_id) = body_source_id.as_deref()
            && let Some(source) = sources.get(source_id)
            && let Some((body_start, body_end)) = body_range
        {
            let discoveries = lexical_discoveries(
                &source.text,
                (body_start, body_end),
                &callable.owner,
                &callable.name,
            );
            if discoveries.nested_executable_context {
                gaps.add(
                    "SOURCE_NESTED_EXECUTABLE_CONTEXT_AMBIGUOUS",
                    json!({"symbol":callable.identity}),
                );
            }
            for name in discoveries.calls {
                let scope_candidates = indexes
                    .methods_by_owner_name
                    .get(&(callable.owner.clone(), name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let candidates: Vec<_> = scope_candidates
                    .iter()
                    .filter(|candidate| {
                        exact_scope(&candidate.normalized["scope"]) == Some(callable.scope.as_str())
                    })
                    .collect();
                match candidates.as_slice() {
                    [candidate] => {
                        if provider_targets.contains(&candidate.symbol) {
                            continue;
                        }
                        add_edge(
                            &mut edges,
                            &mut graph_edge_count,
                            Edge {
                                from: callable.identity.clone(),
                                target: Some(candidate.symbol.clone()),
                                scope: callable.scope.clone(),
                                kind: "CALLS".into(),
                                authority: "SOURCE_REFERENCE_CANDIDATE".into(),
                                fact_id: None,
                                source_id: Some(source_id.to_owned()),
                            },
                            &mut gaps,
                        );
                    }
                    [] if !scope_candidates.is_empty() => gaps.add(
                        "SOURCE_REFERENCE_SCOPE_UNAVAILABLE",
                        json!({"from":callable.identity,"name":name,"scope":callable.scope}),
                    ),
                    [] => {}
                    _ => gaps.add(
                        "SOURCE_REFERENCE_AMBIGUOUS_OVERLOAD",
                        json!({"from":callable.identity,"name":name,"scope":callable.scope,"candidateCount":candidates.len()}),
                    ),
                }
            }
            for name in discoveries.method_references {
                let same_owner = indexes
                    .methods_by_owner_name
                    .get(&(callable.owner.clone(), name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let candidates: Vec<_> = same_owner
                    .iter()
                    .filter(|candidate| {
                        exact_scope(&candidate.normalized["scope"]) == Some(callable.scope.as_str())
                    })
                    .collect();
                match candidates.as_slice() {
                    [candidate] => add_edge(
                        &mut edges,
                        &mut graph_edge_count,
                        Edge {
                            from: callable.identity.clone(),
                            target: Some(candidate.symbol.clone()),
                            scope: callable.scope.clone(),
                            kind: "METHOD_REFERENCE".into(),
                            authority: "SOURCE_REFERENCE_CANDIDATE".into(),
                            fact_id: None,
                            source_id: Some(source_id.to_owned()),
                        },
                        &mut gaps,
                    ),
                    [] if !same_owner.is_empty() => gaps.add(
                        "SOURCE_REFERENCE_SCOPE_UNAVAILABLE",
                        json!({"from":callable.identity,"name":name,"scope":callable.scope}),
                    ),
                    [] => gaps.add(
                        "SOURCE_METHOD_REFERENCE_DECLARATION_UNAVAILABLE",
                        json!({"from":callable.identity,"owner":callable.owner,"name":name,"scope":callable.scope}),
                    ),
                    _ => gaps.add(
                        "SOURCE_REFERENCE_AMBIGUOUS_OVERLOAD",
                        json!({"from":callable.identity,"name":name,"scope":callable.scope,"candidateCount":candidates.len()}),
                    ),
                }
            }
            for reference in discoveries.fields {
                let same_owner = indexes
                    .fields_by_owner_name
                    .get(&(callable.owner.clone(), reference.name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if reference.shadowed && !reference.explicit_receiver {
                    if !same_owner.is_empty() {
                        gaps.add(
                            "SOURCE_FIELD_SHADOWING_AMBIGUOUS",
                            json!({"from":callable.identity,"field":reference.name,"scope":callable.scope}),
                        );
                    }
                    continue;
                }
                let candidates: Vec<_> = same_owner
                    .iter()
                    .filter(|candidate| {
                        exact_scope(&candidate.normalized["scope"]) == Some(callable.scope.as_str())
                    })
                    .collect();
                match candidates.as_slice() {
                    [candidate] => {
                        if !direct_types.contains_key(candidate.normalized["ownerIdentity"].as_str().unwrap_or_default()) {
                            owner_field_candidates.insert(candidate.id.clone());
                        }
                    }
                    [] if !same_owner.is_empty() => gaps.add(
                        "SOURCE_FIELD_SCOPE_UNAVAILABLE",
                        json!({"from":callable.identity,"field":reference.name,"scope":callable.scope}),
                    ),
                    [] => {}
                    _ => gaps.add(
                        "SOURCE_FIELD_DECLARATION_AMBIGUOUS",
                        json!({"from":callable.identity,"field":reference.name,"scope":callable.scope,"candidateCount":candidates.len()}),
                    ),
                }
            }
        } else if body_source_id.is_some() && body_range.is_none() {
            gaps.add(
                "METHOD_BODY_PARSE_UNAVAILABLE",
                json!({"symbol":callable.identity}),
            );
        }

        for target in ordered_targets_for_caller(&edges, &callable.identity, &callable.scope) {
            let target_key = (target.clone(), callable.scope.clone());
            if visited.contains(&target_key) || scheduled.contains(&target_key) {
                continue;
            }
            let candidates = indexes
                .methods_by_identity_scope
                .get(&target_key)
                .map(Vec::as_slice)
                .unwrap_or_default();
            match candidates {
                [candidate] if candidate.normalized["declarationKind"] == "METHOD"
                    || candidate.normalized["declarationKind"] == "CONSTRUCTOR" =>
                {
                    if depth >= MAX_DEPTH {
                        gaps.add(
                            "CALL_GRAPH_DEPTH_LIMIT",
                            json!({"from":callable.identity,"target":target,"limit":MAX_DEPTH}),
                        );
                    } else {
                        let target_callable = callable_from_observation(candidate);
                        let (source_id, source) = match method_body_source(evidence, &target_callable)
                        {
                            Ok(body) => body,
                            Err(code) => {
                                gaps.add(code, json!({"symbol":target_callable.identity}));
                                continue;
                            }
                        };
                        if source_steps::method_body(&source.text, &target_callable.identity)
                            .is_none()
                        {
                            gaps.add(
                                "METHOD_BODY_PARSE_UNAVAILABLE",
                                json!({"symbol":target_callable.identity}),
                            );
                            continue;
                        }
                        if scheduled.len() >= MAX_CALLABLES {
                            gaps.add(
                                "CALLABLE_BODY_LIMIT",
                                json!({"from":callable.identity,"target":target,"limit":MAX_CALLABLES}),
                            );
                            continue;
                        }
                        if !counted_source_ids.contains(&source_id)
                            && unique_source_bytes.saturating_add(source.text.len())
                                > MAX_SOURCE_BYTES
                        {
                            gaps.add(
                                "UNIQUE_SOURCE_BYTE_LIMIT",
                                json!({"symbol":target_callable.identity,"bytes":source.text.len(),"limit":MAX_SOURCE_BYTES}),
                            );
                            continue;
                        }
                        if counted_source_ids.insert(source_id.clone()) {
                            body_source_bytes += source.text.len();
                            unique_source_bytes += source.text.len();
                            sources.insert(source_id.clone(), source);
                        }
                        scheduled.insert(target_key);
                        queue.push_back((target_callable, depth + 1, Some(source_id)));
                    }
                }
                [] => {
                    let other_scope = indexes
                        .method_scopes_by_identity
                        .get(&target)
                        .is_some_and(|scopes| !scopes.is_empty());
                    gaps.add(
                        if other_scope {
                            "CALL_TARGET_EXISTS_ONLY_IN_OTHER_SCOPE"
                        } else {
                            "CALL_TARGET_BODY_NOT_CAPTURED"
                        },
                        json!({"from":callable.identity,"target":target,"scope":callable.scope}),
                    );
                }
                _ => gaps.add(
                    "CALL_TARGET_DECLARATION_AMBIGUOUS",
                    json!({"from":callable.identity,"target":target,"scope":callable.scope,"candidateCount":candidates.len()}),
                ),
            }
        }

        // These are selected small provider facts, not a synthesized call order.
        for observation in caller_flows.iter().chain(caller_relations.iter()).copied() {
            if !dependencies.contains_key(&observation.id) {
                dependencies.insert(observation.id.clone(), observation);
            }
        }
    }

    let Some(scope) = root_scope.as_deref().filter(|scope| !scope.is_empty()) else {
        // Do not match an unscoped endpoint to a seemingly similar declaration.
        let dto_groups = direct_types
            .into_iter()
            .map(|(identity, directions)| {
                json!({"identity":identity,"directions":directions,"fieldReferences":[]})
            })
            .collect::<Vec<_>>();
        return finish_rows(FinishRows {
            work,
            evidence,
            entrypoint_id,
            entry,
            dependencies,
            sources,
            nodes,
            node_keys,
            edges,
            dto_groups,
            owner_groups: Vec::new(),
            gaps,
            body_source_bytes,
            field_source_bytes: dto_source_bytes,
            unique_source_bytes,
        });
    };

    for identity in direct_types.keys() {
        let type_matches = indexes
            .types_by_identity_scope
            .get(&(identity.clone(), scope.to_owned()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        if type_matches.len() != 1 {
            gaps.add(
                if type_matches.is_empty() {
                    "DIRECT_TYPE_DECLARATION_NOT_CAPTURED"
                } else {
                    "DIRECT_TYPE_DECLARATION_AMBIGUOUS"
                },
                json!({"identity":identity,"scope":scope,"candidateCount":type_matches.len()}),
            );
            continue;
        }
        let candidates = indexes
            .fields_by_owner_scope
            .get(&(identity.clone(), scope.to_owned()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        for field in candidates {
            dto_field_ids.push(field.id.clone());
        }
    }
    dto_field_ids.sort();
    dto_field_ids.dedup();
    if dto_field_ids.len() > MAX_DTO_FIELDS {
        let omitted = dto_field_ids.len() - MAX_DTO_FIELDS;
        dto_field_ids.truncate(MAX_DTO_FIELDS);
        gaps.add(
            "DTO_FIELD_LIMIT",
            json!({"omitted":omitted,"limit":MAX_DTO_FIELDS}),
        );
    }
    let mut selected_dto = Vec::new();
    for id in &dto_field_ids {
        let Some(field) = evidence.observations.get(id) else {
            gaps.add("DTO_FIELD_RECORD_UNAVAILABLE", json!({"id":id}));
            continue;
        };
        if !reserve_field_tokens(field, &mut unique_source_bytes, &mut dto_source_bytes) {
            gaps.add(
                "UNIQUE_SOURCE_BYTE_LIMIT",
                json!({"field":field.symbol,"limit":MAX_SOURCE_BYTES}),
            );
            continue;
        }
        selected_dto.push(field);
    }
    for field in &selected_dto {
        dependencies.insert(field.id.clone(), *field);
    }

    let mut owner_ids: Vec<_> = owner_field_candidates.into_iter().collect();
    owner_ids.sort_by(|left, right| {
        let priority = |id: &str| {
            evidence
                .observations
                .get(id)
                .is_some_and(is_static_final_field)
        };
        priority(right)
            .cmp(&priority(left))
            .then_with(|| left.cmp(right))
    });
    if owner_ids.len() > MAX_REFERENCED_FIELDS {
        let omitted = owner_ids.len() - MAX_REFERENCED_FIELDS;
        owner_ids.truncate(MAX_REFERENCED_FIELDS);
        gaps.add(
            "REFERENCED_OWNER_FIELD_LIMIT",
            json!({"omitted":omitted,"limit":MAX_REFERENCED_FIELDS}),
        );
    }
    for id in owner_ids {
        let Some(field) = evidence.observations.get(&id) else {
            gaps.add("REFERENCED_FIELD_RECORD_UNAVAILABLE", json!({"id":id}));
            continue;
        };
        if !reserve_field_tokens(field, &mut unique_source_bytes, &mut dto_source_bytes) {
            gaps.add(
                "UNIQUE_SOURCE_BYTE_LIMIT",
                json!({"field":field.symbol,"limit":MAX_SOURCE_BYTES}),
            );
            continue;
        }
        owner_field_ids.insert(id.clone());
        dependencies.insert(id, field);
    }

    let dto_groups: Vec<_> = direct_types
        .into_iter()
        .map(|(identity, roles)| {
            let fields: Vec<_> = selected_dto
                .iter()
                .filter(|field| field.normalized["ownerIdentity"] == identity)
                .filter_map(|field| work_reference(work, "DEPENDENCY", &field.id))
                .map(str::to_owned)
                .collect();
            json!({
                "identity":identity,
                "directions":roles,
                "fieldReferences":fields
            })
        })
        .collect();

    let owner_groups: Vec<_> = owner_field_ids
        .iter()
        .filter_map(|id| evidence.observations.get(id))
        .fold(
            BTreeMap::<String, Vec<String>>::new(),
            |mut groups, field| {
                let owner = field.normalized["ownerIdentity"]
                    .as_str()
                    .unwrap_or_default();
                if let Some(reference) = work_reference(work, "DEPENDENCY", &field.id) {
                    groups
                        .entry(owner.into())
                        .or_default()
                        .push(reference.into());
                }
                groups
            },
        )
        .into_iter()
        .map(|(owner, references)| json!({"owner":owner,"fieldReferences":references}))
        .collect();

    finish_rows(FinishRows {
        work,
        evidence,
        entrypoint_id,
        entry,
        dependencies,
        sources,
        nodes,
        node_keys,
        edges,
        dto_groups,
        owner_groups,
        gaps,
        body_source_bytes,
        field_source_bytes: dto_source_bytes,
        unique_source_bytes,
    })
}

fn finish_rows(input: FinishRows<'_>) -> Result<Vec<Value>, ClewError> {
    let FinishRows {
        work,
        evidence,
        entrypoint_id,
        entry,
        dependencies,
        sources,
        nodes,
        node_keys,
        edges,
        dto_groups,
        owner_groups,
        mut gaps,
        body_source_bytes,
        field_source_bytes,
        unique_source_bytes,
    } = input;
    let mut rows = Vec::new();
    let delivered_sources: BTreeSet<_> = sources.keys().cloned().collect();
    let delivered_source_refs: BTreeSet<_> = delivered_sources
        .iter()
        .filter_map(|id| work_reference(work, "SOURCE", id))
        .collect();
    let mut provider_fact_references = BTreeSet::new();
    let mut source_reference_candidates = Vec::new();
    let mut omitted_candidate_edges = 0usize;
    for edge in edges {
        if let Some(fact_id) = edge.fact_id.as_deref() {
            if let Some(reference) = work_reference(work, "DEPENDENCY", fact_id) {
                provider_fact_references.insert(reference.to_owned());
            }
            continue;
        }
        if edge.authority != "SOURCE_REFERENCE_CANDIDATE" {
            continue;
        }
        if source_reference_candidates.len() >= MAX_INLINE_CANDIDATE_EDGES {
            omitted_candidate_edges += 1;
            continue;
        }
        let from_node = node_keys
            .iter()
            .position(|key| key.0 == edge.from && key.1 == edge.scope)
            .map(|index| format!("m{index}"));
        let to_node = edge.target.as_deref().and_then(|target| {
            node_keys
                .iter()
                .position(|key| key.0 == target && key.1 == edge.scope)
                .map(|index| format!("m{index}"))
        });
        source_reference_candidates.push(json!({
            "fromNode":from_node,
            "toNode":to_node,
            "targetIdentity":if to_node.is_none(){edge.target}else{None::<String>},
            "kind":edge.kind,
            "authority":edge.authority
        }));
    }
    gaps.add("STRUCTURAL_DATAFLOW_NOT_AVAILABLE", Value::Null);
    gaps.add("DIRECT_DECLARED_TYPES_ONLY", Value::Null);
    gaps.add("SOURCE_REFERENCE_CANDIDATE_AUTHORITY", Value::Null);
    if omitted_candidate_edges > 0 {
        gaps.add(
            "SOURCE_REFERENCE_CANDIDATE_LIMIT",
            json!({"omitted":omitted_candidate_edges,"limit":MAX_INLINE_CANDIDATE_EDGES}),
        );
    }
    if let Some(entry) = entry {
        let root_node = (!nodes.is_empty()).then_some("m0");
        rows.push(json!({"kind":"ENTRYPOINT","id":entry.id,"record":entry}));
        rows.push(json!({
            "kind":"ENDPOINT_CONTEXT_PACKET",
            "id":format!("endpoint-context:{entrypoint_id}"),
            "record":{
                "profile":PROFILE,
                "authority":"DERIVED_NAVIGATION_ONLY",
                "entrypointReference":work_reference(work,"ENTRYPOINT",&entry.id),
                "rootNode":root_node,
                "inputAndOutputTypes":dto_groups,
                "callGraph":{
                    "authority":"RETAINED_TARGET_RELATIONS",
                    "order":"NOT_EXECUTION_ORDER",
                    "nodes":nodes,
                    "providerEdgeFactReferences":provider_fact_references,
                    "sourceReferenceCandidates":source_reference_candidates,
                    "depthLimit":MAX_DEPTH,
                    "callableLimit":MAX_CALLABLES,
                    "providerEdgeReferenceLimit":MAX_GRAPH_EDGES,
                    "sourceReferenceCandidateLimit":MAX_INLINE_CANDIDATE_EDGES
                },
                "referencedOwnerFields":owner_groups,
                "selection":{"uniqueSourceBytes":unique_source_bytes,"uniqueBodySourceBytes":body_source_bytes,"fieldTokenBytes":field_source_bytes,"sourceByteLimit":MAX_SOURCE_BYTES,
                    "dtoFieldLimit":MAX_DTO_FIELDS,"referencedOwnerFieldLimit":MAX_REFERENCED_FIELDS,
                    "omittedSourceReferenceCandidates":omitted_candidate_edges},
                "gaps":gaps.rows(),
            }
        }));
    } else {
        rows.push(json!({
            "kind":"ENDPOINT_CONTEXT_PACKET",
            "id":format!("endpoint-context:{entrypoint_id}"),
            "record":{"profile":PROFILE,"authority":"DERIVED_NAVIGATION_ONLY","gaps":gaps.rows()}
        }));
    }
    rows.push(json!({
        "kind":"COVERAGE",
        "id":evidence.service,
        "record":{"revision":evidence.revision,"extractor":evidence.extractor,"runtimeMode":evidence.runtime_mode,
            "coverage":evidence.coverage,"boundaries":evidence.boundaries,
            "callAuthority":if evidence.extractor==super::model::SOURCE_EXTRACTOR{"SYNTAX_UNRESOLVED"}else{"PROVIDER_EVIDENCE"}}
    }));
    let mut provider_facts = Vec::new();
    for observation in dependencies.into_values() {
        if observation.normalized["declarationKind"] == "FIELD" {
            rows.push(json!({"kind":"DEPENDENCY","id":observation.id,"record":observation}));
        } else {
            provider_facts.push(observation);
        }
    }
    for source in sources.into_values() {
        rows.push(json!({"kind":"SOURCE","id":source.id,"record":source}));
    }
    for observation in provider_facts {
        rows.push(json!({"kind":"DEPENDENCY","id":observation.id,"record":observation}));
    }
    for (index, record) in work.review_reasons.iter().enumerate() {
        rows.push(json!({"kind":"REVIEW_REASON","id":format!("review-{index}"),"record":record}));
    }
    for (path, record) in &work.external_inputs {
        rows.push(json!({"kind":"EXTERNAL_INPUT","id":path,"record":record}));
    }
    for (index, obligation) in work.obligations.iter().enumerate() {
        rows.push(
            json!({"kind":"OBLIGATION","id":format!("obligation-{}",index+1),"record":obligation}),
        );
    }

    // Do not expose a navigation handle to a source fragment unless the exact
    // SOURCE item is in this packet's membership. Read receipts still bind only
    // the outer handles actually returned by the page reader.
    for row in &mut rows {
        if row["kind"] != "DEPENDENCY" {
            continue;
        }
        if let Some(source_refs) = row["sourceReferences"].as_array_mut() {
            source_refs.retain(|reference| {
                reference
                    .as_str()
                    .is_some_and(|reference| delivered_source_refs.contains(reference))
            });
        }
        // annotate_rows adds source navigation after this function returns. The
        // caller repeats this filter on the final profile rows.
    }
    Ok(rows)
}

pub(super) fn restrict_unselected_source_references(work: &Work, rows: &mut [Value]) {
    let delivered_source_ids: BTreeSet<String> = rows
        .iter()
        .filter(|row| row["kind"] == "SOURCE")
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect();
    let delivered_dependency_ids: BTreeSet<String> = rows
        .iter()
        .filter(|row| row["kind"] == "DEPENDENCY")
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect();
    for row in rows.iter_mut() {
        if let Some(ids) = row["record"]["sourceIds"].as_array() {
            let allowed: BTreeSet<_> = ids
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| delivered_source_ids.contains(*id))
                .filter_map(|id| work_reference(work, "SOURCE", id))
                .collect();
            if let Some(references) = row["sourceReferences"].as_array_mut() {
                references.retain(|reference| {
                    reference
                        .as_str()
                        .is_some_and(|reference| allowed.contains(reference))
                });
            }
        }
        if let Some(ids) = row["record"]["dependencyIds"].as_array() {
            let allowed: BTreeSet<_> = ids
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| delivered_dependency_ids.contains(*id))
                .filter_map(|id| work_reference(work, "DEPENDENCY", id))
                .collect();
            if let Some(references) = row["dependencyReferences"].as_array_mut() {
                references.retain(|reference| {
                    reference
                        .as_str()
                        .is_some_and(|reference| allowed.contains(reference))
                });
            }
        }
    }
}

fn unique_entrypoint<'a>(
    evidence: &'a ServiceEvidence,
    entrypoint_id: &str,
) -> Option<&'a super::model::Entrypoint> {
    let mut entries = evidence
        .entrypoints
        .iter()
        .filter(|entry| entry.id == entrypoint_id && entry.kind == "HTTP_ENDPOINT");
    let entry = entries.next()?;
    entries.next().is_none().then_some(entry)
}

fn exact_scope(value: &Value) -> Option<&str> {
    value.as_str().filter(|scope| !scope.trim().is_empty())
}

fn unique_source_id(ids: &[String]) -> Option<&str> {
    match ids {
        [id] if !id.is_empty() => Some(id),
        _ => None,
    }
}

fn callable_from_observation(observation: &Observation) -> Callable<'_> {
    Callable {
        identity: observation.symbol.clone(),
        scope: exact_scope(&observation.normalized["scope"])
            .unwrap_or_default()
            .to_owned(),
        owner: observation.normalized["ownerIdentity"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        name: observation.normalized["name"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        observation,
    }
}

fn is_static_final_field(observation: &Observation) -> bool {
    let Some(modifiers) = observation.normalized["modifiers"].as_array() else {
        return false;
    };
    modifiers.iter().any(|modifier| modifier == "STATIC")
        && modifiers.iter().any(|modifier| modifier == "FINAL")
}

fn declaration_indexes<'a>(
    evidence: &'a ServiceEvidence,
    work: &Work,
    service: &str,
) -> DeclarationIndexes<'a> {
    let mut indexes = DeclarationIndexes {
        methods_by_identity_scope: BTreeMap::new(),
        methods_by_owner_name: BTreeMap::new(),
        method_scopes_by_identity: BTreeMap::new(),
        types_by_identity_scope: BTreeMap::new(),
        fields_by_owner_name: BTreeMap::new(),
        fields_by_owner_scope: BTreeMap::new(),
        outgoing_by_identity_scope: BTreeMap::new(),
    };
    for observation in evidence.observations.values().filter(|observation| {
        observation.service == service && work.influence.contains_key(&observation.id)
    }) {
        let scope = exact_scope(&observation.normalized["scope"]).map(str::to_owned);
        match observation.kind.as_str() {
            "FLOW" | "CALL_RELATION" => {
                if let Some(scope) = scope {
                    let outgoing = indexes
                        .outgoing_by_identity_scope
                        .entry((observation.symbol.clone(), scope))
                        .or_default();
                    if observation.kind == "FLOW" {
                        outgoing.flows.push(observation);
                    } else {
                        outgoing.relations.push(observation);
                    }
                }
            }
            "SYMBOL" if observation.normalized["schema"] == JAVA_COMPILER_FACT_SCHEMA => {
                let (Some(identity), Some(declaration_kind)) = (
                    observation.normalized["symbolIdentity"].as_str(),
                    observation.normalized["declarationKind"].as_str(),
                ) else {
                    continue;
                };
                match declaration_kind {
                    "METHOD" | "CONSTRUCTOR" => {
                        let owner = observation.normalized["ownerIdentity"]
                            .as_str()
                            .unwrap_or_default();
                        let name = observation.normalized["name"].as_str().unwrap_or_default();
                        indexes
                            .methods_by_owner_name
                            .entry((owner.to_owned(), name.to_owned()))
                            .or_default()
                            .push(observation);
                        if let Some(scope) = scope {
                            indexes
                                .methods_by_identity_scope
                                .entry((identity.to_owned(), scope.clone()))
                                .or_default()
                                .push(observation);
                            indexes
                                .method_scopes_by_identity
                                .entry(identity.to_owned())
                                .or_default()
                                .insert(scope);
                        }
                    }
                    "CLASS" | "INTERFACE" | "ENUM" | "RECORD" | "ANNOTATION_TYPE" => {
                        if let Some(scope) = scope {
                            indexes
                                .types_by_identity_scope
                                .entry((identity.to_owned(), scope))
                                .or_default()
                                .push(observation);
                        }
                    }
                    "FIELD" => {
                        let (Some(owner), Some(name)) = (
                            observation.normalized["ownerIdentity"].as_str(),
                            observation.normalized["name"].as_str(),
                        ) else {
                            continue;
                        };
                        indexes
                            .fields_by_owner_name
                            .entry((owner.to_owned(), name.to_owned()))
                            .or_default()
                            .push(observation);
                        if let Some(scope) = scope {
                            indexes
                                .fields_by_owner_scope
                                .entry((owner.to_owned(), scope))
                                .or_default()
                                .push(observation);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    for values in indexes.methods_by_identity_scope.values_mut() {
        values.sort_by(|a, b| a.id.cmp(&b.id));
    }
    for values in indexes.methods_by_owner_name.values_mut() {
        values.sort_by(|a, b| a.id.cmp(&b.id));
    }
    for values in indexes.types_by_identity_scope.values_mut() {
        values.sort_by(|a, b| a.id.cmp(&b.id));
    }
    for values in indexes.fields_by_owner_name.values_mut() {
        values.sort_by(|a, b| a.id.cmp(&b.id));
    }
    for values in indexes.fields_by_owner_scope.values_mut() {
        values.sort_by(|a, b| a.id.cmp(&b.id));
    }
    for outgoing in indexes.outgoing_by_identity_scope.values_mut() {
        outgoing.flows.sort_by(|a, b| a.id.cmp(&b.id));
        outgoing.relations.sort_by(|a, b| a.id.cmp(&b.id));
    }
    indexes
}

fn method_body_source<'a>(
    evidence: &'a ServiceEvidence,
    callable: &Callable<'a>,
) -> Result<(String, &'a Source), &'static str> {
    let source_id = unique_source_id(&callable.observation.source_ids)
        .ok_or("METHOD_BODY_SOURCE_UNAVAILABLE")?;
    let source = evidence
        .sources
        .get(source_id)
        .ok_or("METHOD_BODY_SOURCE_UNAVAILABLE")?;
    if source.service != callable.observation.service
        || source.revision != evidence.revision
        || source.authority.trim().is_empty()
        || source.text.is_empty()
        || source.text_digest != crate::canonical::hash_bytes(source.text.as_bytes())
    {
        return Err("METHOD_BODY_SOURCE_PROVENANCE_INVALID");
    }
    if source.text.len() > MAX_SOURCE_BYTES {
        return Err("METHOD_BODY_SOURCE_LIMIT");
    }
    Ok((source_id.to_owned(), source))
}

fn relation_source_bound(
    evidence: &ServiceEvidence,
    relation: &Observation,
    service: &str,
) -> bool {
    let call_site = &relation.normalized["callSite"];
    match relation.source_ids.as_slice() {
        [source_id] if call_site["sourceId"].as_str() == Some(source_id) => {
            evidence.sources.get(source_id).is_some_and(|source| {
                source.service == service
                    && source.revision == evidence.revision
                    && call_site["sourceStatus"] == "SOURCE_RETAINED"
                    && call_site["sourceDigest"] == source.text_digest
                    && call_site["evidenceDigest"] == source.evidence_digest
                    && source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
            })
        }
        _ => false,
    }
}

fn owning_body_source_reference(
    evidence: &ServiceEvidence,
    fact: &Observation,
    body_source_id: &str,
) -> Option<String> {
    let body = evidence.sources.get(body_source_id)?;
    fact.source_ids.iter().find_map(|source_id| {
        if source_id == body_source_id {
            return Some(body_source_id.to_owned());
        }
        let part = evidence.sources.get(source_id)?;
        super::process_context::covered_text(body, part).map(|_| body_source_id.to_owned())
    })
}

fn ordered_targets_for_caller(edges: &[Edge], caller: &str, scope: &str) -> Vec<String> {
    let mut priorities = BTreeMap::<String, u8>::new();
    for edge in edges
        .iter()
        .filter(|edge| edge.from == caller && edge.scope == scope)
    {
        let Some(target) = edge.target.as_ref() else {
            continue;
        };
        let priority = match edge.authority.as_str() {
            "COMPILER_EXACT_CALL_RELATION" => 0,
            "RETAINED_FLOW_TARGET" => 1,
            "SOURCE_REFERENCE_CANDIDATE" => 2,
            _ => 3,
        };
        priorities
            .entry(target.clone())
            .and_modify(|current| *current = (*current).min(priority))
            .or_insert(priority);
    }
    let mut targets: Vec<_> = priorities.into_iter().collect();
    targets.sort_by(
        |(left_target, left_priority), (right_target, right_priority)| {
            left_priority
                .cmp(right_priority)
                .then_with(|| left_target.cmp(right_target))
        },
    );
    targets.into_iter().map(|(target, _)| target).collect()
}

fn add_edge(edges: &mut Vec<Edge>, count: &mut usize, edge: Edge, gaps: &mut Gaps) {
    if *count >= MAX_GRAPH_EDGES {
        gaps.add(
            "CALL_GRAPH_EDGE_LIMIT",
            json!({"limit":MAX_GRAPH_EDGES,"from":edge.from,"target":edge.target}),
        );
        return;
    }
    if edges.iter().any(|candidate| {
        candidate.from == edge.from
            && candidate.target == edge.target
            && candidate.scope == edge.scope
            && candidate.kind == edge.kind
            && candidate.authority == edge.authority
            && candidate.fact_id == edge.fact_id
            && candidate.source_id == edge.source_id
    }) {
        return;
    }
    *count += 1;
    edges.push(edge);
}

fn work_reference<'a>(work: &'a Work, kind: &str, id: &str) -> Option<&'a str> {
    work.handles
        .iter()
        .find(|(_, handle)| handle.kind == kind && handle.id == id)
        .map(|(reference, _)| reference.as_str())
}

fn reserve_field_tokens(
    field: &Observation,
    total_reserved: &mut usize,
    category_reserved: &mut usize,
) -> bool {
    let Some(tokens) = field.normalized["sourceTokens"].as_array() else {
        return true;
    };
    let Ok(encoded) = serde_json::to_vec(tokens) else {
        return false;
    };
    if total_reserved.saturating_add(encoded.len()) > MAX_SOURCE_BYTES {
        return false;
    }
    *total_reserved += encoded.len();
    *category_reserved += encoded.len();
    true
}

fn descriptor_object_roles(descriptor: &str) -> Result<(BTreeSet<String>, BTreeSet<String>), ()> {
    fn parse_type(
        bytes: &[u8],
        cursor: &mut usize,
        allow_void: bool,
    ) -> Result<Option<String>, ()> {
        let mut dimensions = 0usize;
        while bytes.get(*cursor) == Some(&b'[') {
            dimensions += 1;
            if dimensions > 255 {
                return Err(());
            }
            *cursor += 1;
        }
        match bytes.get(*cursor).copied() {
            Some(b'V') if allow_void && dimensions == 0 => {
                *cursor += 1;
                Ok(None)
            }
            Some(b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z') => {
                *cursor += 1;
                Ok(None)
            }
            Some(b'L') => {
                *cursor += 1;
                let start = *cursor;
                let end = bytes[start..]
                    .iter()
                    .position(|byte| *byte == b';')
                    .map(|offset| start + offset)
                    .ok_or(())?;
                let name = std::str::from_utf8(&bytes[start..end]).map_err(|_| ())?;
                if name.is_empty()
                    || name.starts_with('/')
                    || name.ends_with('/')
                    || name.split('/').any(str::is_empty)
                    || name.bytes().any(|byte| matches!(byte, b'.' | b';' | b'['))
                {
                    return Err(());
                }
                *cursor = end + 1;
                Ok(Some(format!("class:{}", name.replace('/', "."))))
            }
            _ => Err(()),
        }
    }

    let bytes = descriptor.as_bytes();
    if bytes.first() != Some(&b'(') {
        return Err(());
    }
    let mut cursor = 1usize;
    let mut inputs = BTreeSet::new();
    let mut slots = 0usize;
    while bytes.get(cursor) != Some(&b')') {
        let start = cursor;
        let identity = parse_type(bytes, &mut cursor, false)?;
        if cursor <= start {
            return Err(());
        }
        slots += if bytes[start] == b'[' {
            1
        } else if matches!(bytes[start], b'J' | b'D') {
            2
        } else {
            1
        };
        if slots > 255 {
            return Err(());
        }
        inputs.extend(identity);
    }
    cursor += 1;
    let output = parse_type(bytes, &mut cursor, true)?;
    if cursor != bytes.len() {
        return Err(());
    }
    Ok((inputs, output.into_iter().collect()))
}

#[derive(Default)]
struct Discoveries {
    calls: BTreeSet<String>,
    method_references: BTreeSet<String>,
    fields: BTreeSet<FieldUse>,
    nested_executable_context: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FieldUse {
    name: String,
    explicit_receiver: bool,
    shadowed: bool,
}

#[derive(Clone)]
struct Token {
    text: String,
    start: usize,
    end: usize,
}

fn lexical_discoveries(
    source: &str,
    body_range: (usize, usize),
    owner: &str,
    method_name: &str,
) -> Discoveries {
    let tokens = code_tokens(source);
    if has_nested_executable_context(&tokens, body_range) {
        return Discoveries {
            calls: BTreeSet::new(),
            method_references: BTreeSet::new(),
            fields: BTreeSet::new(),
            nested_executable_context: true,
        };
    }
    let mut calls = BTreeSet::new();
    let mut method_references = BTreeSet::new();
    let mut referenced_fields = BTreeMap::<String, (bool, bool)>::new();
    let parameter_range = method_parameter_range(&tokens, body_range.0, method_name);
    let shadowed: BTreeSet<_> = tokens
        .iter()
        .enumerate()
        .filter(|(index, token)| {
            let in_body = token.start > body_range.0 && token.start < body_range.1;
            let in_parameters = parameter_range
                .is_some_and(|(start, end)| token.start > start && token.start < end);
            (in_body || in_parameters) && looks_like_variable_declaration(&tokens, *index, token)
        })
        .map(|(_, token)| token.text.clone())
        .collect();

    for (index, token) in tokens.iter().enumerate() {
        if token.start <= body_range.0 || token.start >= body_range.1 || !is_identifier(&token.text)
        {
            continue;
        }
        let previous_dot = index > 0 && tokens[index - 1].text == ".";
        let explicit_receiver = previous_dot && index > 1 && tokens[index - 2].text == "this";
        let owner_qualified =
            previous_dot && index > 1 && is_owner_qualifier(owner, &tokens[index - 2].text);
        let following_call = tokens.get(index + 1).is_some_and(|next| next.text == "(");
        let current_this_method_reference = index >= 3
            && tokens[index - 1].text == ":"
            && tokens[index - 2].text == ":"
            && tokens[index - 3].text == "this"
            && (index < 4 || tokens[index - 4].text != ".");
        if current_this_method_reference {
            method_references.insert(token.text.clone());
            continue;
        }
        if following_call && (!previous_dot || explicit_receiver) {
            if !matches!(
                token.text.as_str(),
                "if" | "for" | "while" | "switch" | "catch" | "synchronized" | "new" | "super"
            ) {
                calls.insert(token.text.clone());
            }
            continue;
        }
        let eligible_field = !previous_dot || explicit_receiver || owner_qualified;
        if eligible_field
            && (explicit_receiver
                || owner_qualified
                || !looks_like_variable_declaration(&tokens, index, token))
        {
            let entry = referenced_fields.entry(token.text.clone()).or_default();
            entry.0 |= explicit_receiver || owner_qualified;
            entry.1 |= shadowed.contains(&token.text);
        }
    }
    Discoveries {
        calls,
        method_references,
        fields: referenced_fields
            .into_iter()
            .map(|(name, (explicit_receiver, shadowed))| FieldUse {
                name,
                explicit_receiver,
                shadowed,
            })
            .collect(),
        nested_executable_context: false,
    }
}

fn has_nested_executable_context(tokens: &[Token], body_range: (usize, usize)) -> bool {
    for (index, token) in tokens.iter().enumerate() {
        if token.start <= body_range.0 || token.start >= body_range.1 {
            continue;
        }
        if matches!(
            token.text.as_str(),
            "class" | "interface" | "enum" | "record"
        ) && index > 0
            && tokens[index - 1].text != "."
        {
            return true;
        }
        if token.text == "new" {
            for (next_index, next) in tokens.iter().enumerate().skip(index + 1) {
                if next.start >= body_range.1 || matches!(next.text.as_str(), ";" | "->") {
                    break;
                }
                if next.text == "{" && next_index > 0 && tokens[next_index - 1].text == ")" {
                    return true;
                }
            }
        }
    }
    false
}

fn method_parameter_range(
    tokens: &[Token],
    body_open: usize,
    method_name: &str,
) -> Option<(usize, usize)> {
    let method_index = tokens.iter().enumerate().rev().find_map(|(index, token)| {
        (token.start < body_open
            && token.text == method_name
            && tokens.get(index + 1).is_some_and(|next| next.text == "("))
        .then_some(index)
    })?;
    let open = method_index + 1;
    let mut depth = 0usize;
    for index in open..tokens.len() {
        match tokens[index].text.as_str() {
            "(" => depth += 1,
            ")" => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((tokens[open].start, tokens[index].start));
                }
            }
            _ => {}
        }
    }
    None
}

fn code_tokens(source: &str) -> Vec<Token> {
    let (code, _) = source_steps::lexical_masks(source);
    let mut tokens = Vec::new();
    let mut cursor = 0usize;
    let mut identifier_start = None;
    let flush = |start: usize, end: usize, tokens: &mut Vec<Token>| {
        tokens.push(Token {
            text: source[start..end].to_owned(),
            start,
            end,
        });
    };
    for (start, character) in source.char_indices() {
        let end = start + character.len_utf8();
        let is_code = code.get(start).copied().unwrap_or(false);
        let ident = is_code && (character.is_alphanumeric() || matches!(character, '_' | '$'));
        if ident {
            identifier_start.get_or_insert(start);
        } else {
            if let Some(begin) = identifier_start.take() {
                flush(begin, start, &mut tokens);
            }
            if is_code && !character.is_whitespace() {
                flush(start, end, &mut tokens);
            }
        }
        cursor = end;
    }
    if let Some(begin) = identifier_start {
        flush(begin, cursor, &mut tokens);
    }
    tokens
}

fn is_identifier(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|first| first.is_alphabetic() || matches!(first, '_' | '$'))
        && value
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '_' | '$'))
}

fn looks_like_variable_declaration(tokens: &[Token], index: usize, token: &Token) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|index| tokens.get(index)) else {
        return false;
    };
    let following = tokens.get(index + 1).map(|token| token.text.as_str());
    let type_like =
        is_identifier(&previous.text) || matches!(previous.text.as_str(), "]" | ">" | "?");
    type_like
        && previous.text != "."
        && !matches!(previous.text.as_str(), "return" | "throw" | "case" | "new")
        && matches!(following, Some("=" | ";" | "," | ")" | ":" | "[" | "->"))
        && token.start < token.end
}

fn is_owner_qualifier(owner: &str, value: &str) -> bool {
    owner
        .rsplit(['.', '$'])
        .next()
        .is_some_and(|name| !name.is_empty() && name == value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::{
        check::Check,
        model::{Entrypoint, Observation, ServiceEvidence, Source},
        work::{Handle, Request},
    };

    const SERVICE: &str = "orders";
    const OWNER: &str = "class:orders.Service";
    const SCOPE: &str = "compile:orders:main";
    const ROOT: &str = "method:class:orders.Service#handle()V";
    const GETTER: &str = "method:class:orders.Service#getValue()Ljava/lang/String;";
    const SOURCE_ID: &str = "service-source";

    fn observation(
        id: impl Into<String>,
        symbol: impl Into<String>,
        normalized: Value,
        source_ids: Vec<String>,
    ) -> Observation {
        let id = id.into();
        Observation {
            id,
            kind: "SYMBOL".into(),
            service: SERVICE.into(),
            symbol: symbol.into(),
            normalized,
            digest: "observation-digest".into(),
            source_ids,
        }
    }

    fn method_declaration(identity: &str, name: &str) -> Value {
        json!({
            "schema":JAVA_COMPILER_FACT_SCHEMA,
            "declarationKind":"METHOD",
            "symbolIdentity":identity,
            "ownerIdentity":OWNER,
            "name":name,
            "scope":SCOPE,
            "jvmDescriptor":"()Ljava/lang/String;"
        })
    }

    fn bodyless_candidates_do_not_spend_body_slots_before_a_real_getter() -> Work {
        let source_text = "class Service { void handle() { this.getValue(); } String getValue() { return value; } }";
        let source_digest = crate::canonical::hash_bytes(source_text.as_bytes());
        let source = Source {
            id: SOURCE_ID.into(),
            service: SERVICE.into(),
            revision: "revision-1".into(),
            file: "Service.java".into(),
            start_line: 1,
            end_line: 1,
            text: source_text.into(),
            text_digest: source_digest.clone(),
            evidence_digest: "source-evidence-digest".into(),
            authority: "RETAINED_SNAPSHOT".into(),
            occurrence: None,
            url: None,
        };

        let mut observations = BTreeMap::<String, Observation>::new();
        let root = observation(
            "root-declaration",
            ROOT,
            json!({
                "schema":JAVA_COMPILER_FACT_SCHEMA,
                "declarationKind":"METHOD",
                "symbolIdentity":ROOT,
                "ownerIdentity":OWNER,
                "name":"handle",
                "scope":SCOPE,
                "jvmDescriptor":"()V"
            }),
            vec![SOURCE_ID.into()],
        );
        observations.insert(root.id.clone(), root);

        let mut targets = Vec::new();
        for index in 0..MAX_CALLABLES - 1 {
            let name = format!("getStatus{index:02}");
            let identity = format!("method:class:orders.Service#{name}()Ljava/lang/String;");
            let id = format!("synthetic-{index:02}");
            let declaration = observation(
                id.clone(),
                identity.clone(),
                method_declaration(&identity, &name),
                vec![SOURCE_ID.into()],
            );
            observations.insert(id, declaration);
            targets.push(identity);
        }
        let getter = observation(
            "getter-declaration",
            GETTER,
            method_declaration(GETTER, "getValue"),
            vec![SOURCE_ID.into()],
        );
        observations.insert(getter.id.clone(), getter);
        targets.push(GETTER.into());

        for (index, target) in targets.iter().enumerate() {
            let id = format!("relation-{index:02}");
            observations.insert(
                id.clone(),
                Observation {
                    id: id.clone(),
                    kind: "CALL_RELATION".into(),
                    service: SERVICE.into(),
                    symbol: ROOT.into(),
                    normalized: json!({
                        "scope":SCOPE,
                        "sourceIdentity":ROOT,
                        "targetIdentity":target,
                        "relationKind":"CALLS",
                        "resolution":"COMPILER_EXACT",
                        "callSite":{
                            "sourceId":SOURCE_ID,
                            "sourceStatus":"SOURCE_RETAINED",
                            "sourceDigest":source_digest,
                            "evidenceDigest":"source-evidence-digest"
                        }
                    }),
                    digest: "relation-digest".into(),
                    source_ids: vec![SOURCE_ID.into()],
                },
            );
        }

        let entrypoint = Entrypoint {
            id: "entry-http".into(),
            service: SERVICE.into(),
            symbol: ROOT.into(),
            kind: "HTTP_ENDPOINT".into(),
            trigger: json!({"method":"GET","path":"/value"}),
            source_ids: vec![SOURCE_ID.into()],
            dependency_ids: vec!["root-declaration".into()],
            boundaries: Vec::new(),
        };
        let evidence = ServiceEvidence {
            schema: "codeclew-service-evidence/1.0".into(),
            service: SERVICE.into(),
            revision: "revision-1".into(),
            service_digest: "service-digest".into(),
            extractor: "java-compiler".into(),
            runtime_mode: "test".into(),
            coverage: "COMPLETE".into(),
            boundaries: Vec::new(),
            entrypoints: vec![entrypoint],
            observations,
            sources: BTreeMap::from([(SOURCE_ID.into(), source)]),
            contracts: BTreeMap::new(),
        };
        let all_observation_ids: Vec<_> = evidence.observations.keys().cloned().collect();
        let handles = evidence
            .observations
            .keys()
            .map(|id| {
                (
                    format!("dependency:{id}"),
                    Handle {
                        kind: "DEPENDENCY".into(),
                        id: id.clone(),
                    },
                )
            })
            .chain([
                (
                    "entrypoint:entry-http".into(),
                    Handle {
                        kind: "ENTRYPOINT".into(),
                        id: "entry-http".into(),
                    },
                ),
                (
                    format!("source:{SOURCE_ID}"),
                    Handle {
                        kind: "SOURCE".into(),
                        id: SOURCE_ID.into(),
                    },
                ),
            ])
            .collect();
        Work {
            schema: "codeclew-documentation-work/1.0".into(),
            id: "test-work".into(),
            subject: format!("service:{SERVICE}"),
            request: Request {
                schema: "codeclew-documentation-request/1.0".into(),
                audience: "maintainer".into(),
                documentation_language: Some("en".into()),
                entrypoint: Some("entry-http".into()),
                context_profile: Some(PROFILE.into()),
                max_items: 100,
                max_bytes: 128 * 1024,
                external_inputs: Vec::new(),
            },
            checked: Check {
                schema: "codeclew-documentation-check/1.0".into(),
                input_digest: "input-digest".into(),
                context_digest: "context-digest".into(),
                services: BTreeMap::from([(SERVICE.into(), evidence)]),
                unresolved: BTreeMap::new(),
                interactions: BTreeMap::new(),
                scenarios: BTreeMap::new(),
                dependencies: BTreeMap::new(),
                source_inputs: None,
                composition: None,
            },
            snapshot: None,
            retained: None,
            external_inputs: BTreeMap::new(),
            handles,
            influence: all_observation_ids
                .into_iter()
                .map(|id| (id, "test-influence".into()))
                .collect(),
            obligations: Vec::new(),
            review_reasons: Vec::new(),
        }
    }

    fn method_reference_work() -> Work {
        let mut work = bodyless_candidates_do_not_spend_body_slots_before_a_real_getter();
        let service = work.checked.services.get_mut(SERVICE).unwrap();
        let source = service.sources.get_mut(SOURCE_ID).unwrap();
        source.text = source.text.replace(
            "this.getValue();",
            "Supplier<String> getter = this::getValue;",
        );
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        let source_digest = source.text_digest.clone();
        for relation in service
            .observations
            .values_mut()
            .filter(|observation| observation.kind == "CALL_RELATION")
        {
            relation.normalized["callSite"]["sourceDigest"] = json!(source_digest.clone());
        }
        let relation_id = service
            .observations
            .values()
            .find(|observation| {
                observation.kind == "CALL_RELATION"
                    && observation.normalized["targetIdentity"] == GETTER
            })
            .map(|observation| observation.id.clone())
            .unwrap();
        service.observations.remove(&relation_id);
        work.influence.remove(&relation_id);
        work.handles
            .retain(|_, handle| handle.kind != "DEPENDENCY" || handle.id != relation_id);
        work
    }

    #[test]
    fn bodyless_targets_do_not_starve_a_retained_getter_body() {
        let work = bodyless_candidates_do_not_spend_body_slots_before_a_real_getter();
        let rows = profile_rows(&work).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        let graph = &packet["record"]["callGraph"];
        let nodes = graph["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().any(|node| node["symbolIdentity"] == ROOT));
        assert!(
            nodes.iter().any(|node| {
                node["symbolIdentity"] == GETTER && node["bodyReference"].is_string()
            })
        );
        assert_eq!(
            graph["providerEdgeFactReferences"]
                .as_array()
                .unwrap()
                .len(),
            MAX_CALLABLES
        );
        let gaps = packet["record"]["gaps"].as_array().unwrap();
        assert!(gaps.iter().any(|gap| {
            gap["code"] == "METHOD_BODY_PARSE_UNAVAILABLE" && gap["count"] == MAX_CALLABLES - 1
        }));
        assert!(!gaps.iter().any(|gap| gap["code"] == "CALLABLE_BODY_LIMIT"));
    }

    #[test]
    fn current_this_method_reference_selects_a_unique_same_scope_method_as_a_candidate() {
        let work = method_reference_work();
        let rows = profile_rows(&work).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        let graph = &packet["record"]["callGraph"];
        let nodes = graph["nodes"].as_array().unwrap();
        assert!(nodes.iter().any(|node| node["symbolIdentity"] == GETTER));
        assert_eq!(graph["order"], "NOT_EXECUTION_ORDER");
        let candidates = graph["sourceReferenceCandidates"].as_array().unwrap();
        assert!(candidates.iter().any(|candidate| {
            candidate["kind"] == "METHOD_REFERENCE"
                && candidate["authority"] == "SOURCE_REFERENCE_CANDIDATE"
                && candidate["toNode"].is_string()
        }));
    }

    #[test]
    fn this_method_reference_ambiguity_and_other_scope_are_gaps() {
        let base = method_reference_work();
        let mut ambiguous = base.clone();
        let overloaded_identity = "method:class:orders.Service#getValue(I)Ljava/lang/String;";
        let mut overloaded = observation(
            "getter-overload",
            overloaded_identity,
            method_declaration(overloaded_identity, "getValue"),
            vec![SOURCE_ID.into()],
        );
        overloaded.normalized["jvmDescriptor"] = json!("(I)Ljava/lang/String;");
        ambiguous
            .checked
            .services
            .get_mut(SERVICE)
            .unwrap()
            .observations
            .insert(overloaded.id.clone(), overloaded.clone());
        ambiguous
            .influence
            .insert(overloaded.id.clone(), "test".into());
        ambiguous.handles.insert(
            format!("dependency:{}", overloaded.id),
            Handle {
                kind: "DEPENDENCY".into(),
                id: overloaded.id,
            },
        );
        let ambiguous_rows = profile_rows(&ambiguous).unwrap();
        let ambiguous_packet = ambiguous_rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        let ambiguous_graph = &ambiguous_packet["record"]["callGraph"];
        assert!(
            ambiguous_graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|node| node["symbolIdentity"] != GETTER)
        );
        assert!(
            ambiguous_packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| {
                    gap["code"] == "SOURCE_REFERENCE_AMBIGUOUS_OVERLOAD" && gap["count"] == 1
                })
        );

        let mut other_scope = base;
        other_scope
            .checked
            .services
            .get_mut(SERVICE)
            .unwrap()
            .observations
            .get_mut("getter-declaration")
            .unwrap()
            .normalized["scope"] = json!("compile:other");
        let other_scope_rows = profile_rows(&other_scope).unwrap();
        let other_scope_packet = other_scope_rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        let other_scope_graph = &other_scope_packet["record"]["callGraph"];
        assert!(
            other_scope_graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|node| node["symbolIdentity"] != GETTER)
        );
        assert!(
            other_scope_packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| {
                    gap["code"] == "SOURCE_REFERENCE_SCOPE_UNAVAILABLE" && gap["count"] == 1
                })
        );
    }

    #[test]
    fn outer_this_and_masked_text_are_not_current_this_method_references() {
        let source = "class Service { void handle() { Outer.this::getOuter; this::getCurrent; String text = \"this::getString\"; /* this::getComment */ } }";
        let body = source_steps::method_body(source, ROOT).unwrap();
        let discoveries = lexical_discoveries(source, body, OWNER, "handle");
        assert_eq!(
            discoveries.method_references,
            BTreeSet::from(["getCurrent".into()])
        );
    }
}
