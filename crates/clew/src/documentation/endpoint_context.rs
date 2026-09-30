//! Reachable, nonduplicating author context for one captured Java HTTP endpoint.
//!
//! This is a deterministic selector over immutable Work evidence. The packet
//! row is navigation only; provider declarations, FLOW, CALL_RELATION and
//! retained SOURCE records remain the only factual rows. Compiler REFERENCES
//! identify callback targets without establishing invocation or timing.

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
    methods_by_owner_scope: BTreeMap<(String, String), Vec<&'a Observation>>,
    method_scopes_by_identity: BTreeMap<String, BTreeSet<String>>,
    types_by_identity_scope: BTreeMap<MethodKey, Vec<&'a Observation>>,
    types_by_name_scope: BTreeMap<(String, String), Vec<&'a Observation>>,
    fields_by_owner_name: BTreeMap<(String, String), Vec<&'a Observation>>,
    fields_by_owner_scope: BTreeMap<(String, String), Vec<&'a Observation>>,
    outgoing_by_identity_scope: BTreeMap<MethodKey, OutgoingFacts<'a>>,
}

#[derive(Default)]
struct OutgoingFacts<'a> {
    flows: Vec<&'a Observation>,
    relations: Vec<&'a Observation>,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Edge {
    from: String,
    target: Option<String>,
    scope: String,
    kind: String,
    authority: String,
    fact_id: Option<String>,
    source_id: Option<String>,
    receiver_field_id: Option<String>,
}

struct FinishRows<'a> {
    work: &'a Work,
    evidence: &'a ServiceEvidence,
    entrypoint_id: &'a str,
    entry: Option<&'a super::model::Entrypoint>,
    process_root: Option<&'a Observation>,
    dependencies: BTreeMap<String, &'a Observation>,
    sources: BTreeMap<String, &'a Source>,
    nodes: Vec<Value>,
    node_keys: Vec<MethodKey>,
    edges: Vec<Edge>,
    source_contexts: BTreeSet<SourceContext>,
    dto_groups: Vec<Value>,
    owner_groups: Vec<Value>,
    gaps: Gaps,
    body_source_bytes: usize,
    field_source_bytes: usize,
    unique_source_bytes: usize,
    max_observed_traversal_depth: usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceContext {
    kind: String,
    declaration_id: String,
    source_id: Option<String>,
    referenced_from_source_id: Option<String>,
}

/// Return the initial packet membership for the explicit endpoint profile.
/// Follow-up Work reads remain the ordinary reference/query/read-part paths.
pub(super) fn profile_rows(work: &Work) -> Result<Vec<Value>, ClewError> {
    profile_rows_with_root(work, None)
}

pub(super) fn process_profile_rows(work: &Work) -> Result<Vec<Value>, ClewError> {
    let declaration = work
        .request
        .root_declaration
        .as_deref()
        .ok_or_else(|| invalid("process-graph-v1 requires an exact rootDeclaration"))?;
    profile_rows_with_root(work, Some(declaration))
}

fn profile_rows_with_root(
    work: &Work,
    process_root_id: Option<&str>,
) -> Result<Vec<Value>, ClewError> {
    let service = work
        .subject
        .strip_prefix("service:")
        .filter(|service| !service.is_empty())
        .ok_or_else(|| invalid("operation context requires one service subject"))?;
    let entrypoint_id = process_root_id
        .or(work.request.entrypoint.as_deref())
        .ok_or_else(|| invalid("endpoint-context-v3 requires a service HTTP endpoint"))?;
    let evidence = work
        .checked
        .services
        .get(service)
        .ok_or_else(|| invalid("operation context service evidence is unavailable"))?;
    let entry = process_root_id
        .is_none()
        .then(|| unique_entrypoint(evidence, entrypoint_id))
        .flatten();
    let mut gaps = Gaps::default();
    let mut dependencies = BTreeMap::<String, &Observation>::new();
    let mut sources = BTreeMap::<String, &Source>::new();
    let mut nodes = Vec::<Value>::new();
    let mut edges = Vec::<Edge>::new();
    let mut source_contexts = BTreeSet::<SourceContext>::new();
    let mut exact_constructor_types =
        BTreeMap::<MethodKey, BTreeMap<String, BTreeSet<String>>>::new();
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

    if let Some(declaration_id) = process_root_id {
        let root = evidence
            .observations
            .get(declaration_id)
            .filter(|observation| {
                observation.service == service
                    && work.influence.contains_key(&observation.id)
                    && super::process_graph::callable_observation(observation)
            });
        if let Some(root) = root {
            root_scope = exact_scope(&root.normalized["scope"]).map(str::to_owned);
            if root_scope.is_some() {
                root_callable = Some(callable_from_observation(root));
            } else {
                gaps.add(
                    "PROCESS_ROOT_SCOPE_UNAVAILABLE",
                    json!({"declarationId":declaration_id}),
                );
            }
        } else {
            gaps.add(
                "PROCESS_ROOT_DECLARATION_UNAVAILABLE",
                json!({"declarationId":declaration_id}),
            );
        }
    } else if let Some(entry) = entry {
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

    if process_root_id.is_none()
        && let Some(root) = root_callable.as_ref()
    {
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
    let mut candidate_body_sources = BTreeMap::<MethodKey, String>::new();
    let mut edge_keys = BTreeSet::<Edge>::new();
    let mut max_observed_traversal_depth = 0usize;

    while let Some((callable, depth, reserved_body_source_id)) = queue.pop_front() {
        let key = (callable.identity.clone(), callable.scope.clone());
        if !visited.insert(key.clone()) {
            continue;
        }
        if process_root_id.is_some() {
            dependencies.insert(callable.observation.id.clone(), callable.observation);
        }
        max_observed_traversal_depth = max_observed_traversal_depth.max(depth);
        let body_source_id = if let Some(source_id) = reserved_body_source_id {
            Some(source_id)
        } else {
            match method_body_source(evidence, &callable) {
                Ok((source_id, source)) => {
                    if counted_source_ids.contains(&source_id) {
                        Some(source_id)
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
        let mut provider_reference_targets = BTreeSet::<String>::new();
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
            if process_root_id.is_some()
                && observation.normalized["kind"] == "CONSTRUCT"
                && let Some(owner) = constructor_target_owner_identity(target)
            {
                let type_rows = indexes
                    .types_by_identity_scope
                    .get(&(owner.clone(), callable.scope.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                match type_rows {
                    [type_row] => {
                        if let Some(name) = type_row.normalized["name"].as_str() {
                            exact_constructor_types
                                .entry((callable.identity.clone(), callable.scope.clone()))
                                .or_default()
                                .entry(name.to_owned())
                                .or_default()
                                .insert(owner);
                        }
                        retain_process_type_context(
                            evidence,
                            service,
                            type_row,
                            body_source_id.as_deref(),
                            &mut dependencies,
                            &mut sources,
                            &mut source_contexts,
                            &mut gaps,
                            &mut unique_source_bytes,
                        );
                    }
                    [] => gaps.add(
                        "PROCESS_CONSTRUCTED_TYPE_NOT_CAPTURED",
                        json!({"from":callable.identity,"target":target,"scope":callable.scope}),
                    ),
                    _ => gaps.add(
                        "PROCESS_CONSTRUCTED_TYPE_AMBIGUOUS",
                        json!({"from":callable.identity,"target":target,"scope":callable.scope,"candidateCount":type_rows.len()}),
                    ),
                }
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
                &mut edge_keys,
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
                    receiver_field_id: None,
                },
            );
            provider_targets.insert(target.to_owned());
        }

        for observation in caller_relations.iter().copied() {
            dependencies.insert(observation.id.clone(), observation);
            let relation_kind = observation.normalized["relationKind"].as_str();
            let is_reference = relation_kind == Some("REFERENCES");
            let target = observation.normalized["targetIdentity"]
                .as_str()
                .filter(|v| !v.is_empty());
            let source_bound = relation_source_bound(evidence, observation, service);
            let reason = if observation.normalized["sourceIdentity"].as_str()
                != Some(callable.identity.as_str())
            {
                Some(if is_reference {
                    "REFERENCE_RELATION_OWNER_UNAVAILABLE"
                } else {
                    "CALL_RELATION_OWNER_UNAVAILABLE"
                })
            } else if exact_scope(&observation.normalized["scope"]) != Some(callable.scope.as_str())
            {
                Some(if is_reference {
                    "REFERENCE_RELATION_SCOPE_UNAVAILABLE"
                } else {
                    "CALL_RELATION_SCOPE_UNAVAILABLE"
                })
            } else if !(matches!(relation_kind, Some("CALLS" | "CONSTRUCTS")) || is_reference)
                || observation.normalized["resolution"] != "COMPILER_EXACT"
            {
                Some(if is_reference {
                    "REFERENCE_RELATION_RESOLUTION_UNVERIFIED"
                } else {
                    "CALL_RELATION_RESOLUTION_UNVERIFIED"
                })
            } else if !source_bound {
                Some(if is_reference {
                    "REFERENCE_SITE_SOURCE_UNAVAILABLE"
                } else {
                    "CALL_SITE_SOURCE_UNAVAILABLE"
                })
            } else if target.is_none() {
                Some(if is_reference {
                    "REFERENCE_TARGET_IDENTITY_UNAVAILABLE"
                } else {
                    "CALL_TARGET_IDENTITY_UNAVAILABLE"
                })
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
            let source_reference = body_source_id.as_deref().and_then(|body_id| {
                if is_reference {
                    reference_source_within_method(evidence, observation, body_id)
                } else {
                    owning_body_source_reference(evidence, observation, body_id)
                }
            });
            if source_reference.is_none() {
                if is_reference {
                    gaps.add(
                        "REFERENCE_SITE_NOT_CONTAINED_IN_METHOD_BODY",
                        json!({"from":callable.identity,"factId":observation.id}),
                    );
                    continue;
                }
                gaps.add(
                    "CALL_SITE_SOURCE_NOT_CONTAINED_IN_METHOD_BODY",
                    json!({"from":callable.identity,"factId":observation.id}),
                );
            }
            if is_reference {
                provider_reference_targets.insert(target.to_owned());
            } else {
                provider_targets.insert(target.to_owned());
            }
            add_edge(
                &mut edges,
                &mut edge_keys,
                Edge {
                    from: callable.identity.clone(),
                    target: Some(target.to_owned()),
                    scope: callable.scope.clone(),
                    kind: relation_kind.unwrap().to_owned(),
                    authority: if is_reference {
                        "COMPILER_EXACT_REFERENCE_RELATION".into()
                    } else {
                        "COMPILER_EXACT_CALL_RELATION".into()
                    },
                    fact_id: Some(observation.id.clone()),
                    source_id: source_reference,
                    receiver_field_id: None,
                },
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
            if process_root_id.is_some() {
                for spelling in &discoveries.unsupported_qualified_references {
                    gaps.add(
                        "PROCESS_QUALIFIED_SOURCE_REFERENCE_UNSUPPORTED",
                        json!({"from":callable.identity,"spelling":spelling,"scope":callable.scope}),
                    );
                }
            }
            if process_root_id.is_some() {
                for type_name in &discoveries.constructed_types {
                    let candidates = exact_constructor_types
                        .get(&(callable.identity.clone(), callable.scope.clone()))
                        .and_then(|types| types.get(type_name))
                        .filter(|identities| !identities.is_empty())
                        .map(|identities| {
                            identities
                                .iter()
                                .flat_map(|identity| {
                                    indexes
                                        .types_by_identity_scope
                                        .get(&(identity.clone(), callable.scope.clone()))
                                        .into_iter()
                                        .flatten()
                                        .copied()
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_else(|| {
                            process_type_candidates(&indexes, type_name, &callable.scope)
                        });
                    match candidates.as_slice() {
                        [type_row] => retain_process_type_context(
                            evidence,
                            service,
                            type_row,
                            Some(source_id),
                            &mut dependencies,
                            &mut sources,
                            &mut source_contexts,
                            &mut gaps,
                            &mut unique_source_bytes,
                        ),
                        [] => gaps.add(
                            "PROCESS_CONSTRUCTED_TYPE_NOT_CAPTURED",
                            json!({"from":callable.identity,"typeName":type_name,"scope":callable.scope}),
                        ),
                        _ => gaps.add(
                            "PROCESS_CONSTRUCTED_TYPE_AMBIGUOUS",
                            json!({"from":callable.identity,"typeName":type_name,"scope":callable.scope,"candidateCount":candidates.len()}),
                        ),
                    }
                }
                for qualified_field in &discoveries.qualified_fields {
                    let type_candidates = process_type_candidates(
                        &indexes,
                        &qualified_field.type_name,
                        &callable.scope,
                    );
                    if qualified_field.shadowed {
                        if !type_candidates.is_empty() {
                            gaps.add(
                                "PROCESS_QUALIFIED_FIELD_TYPE_SHADOWED",
                                json!({"from":callable.identity,"typeName":qualified_field.type_name,"field":qualified_field.field_name,"scope":callable.scope}),
                            );
                        }
                        continue;
                    }
                    let [type_row] = type_candidates.as_slice() else {
                        gaps.add(
                            if type_candidates.is_empty() {
                                "PROCESS_QUALIFIED_FIELD_TYPE_NOT_CAPTURED"
                            } else {
                                "PROCESS_QUALIFIED_FIELD_TYPE_AMBIGUOUS"
                            },
                            json!({"from":callable.identity,"typeName":qualified_field.type_name,"field":qualified_field.field_name,"scope":callable.scope,"candidateCount":type_candidates.len()}),
                        );
                        continue;
                    };
                    let owner = type_row.normalized["symbolIdentity"]
                        .as_str()
                        .unwrap_or_default();
                    let field_candidates = indexes
                        .fields_by_owner_name
                        .get(&(owner.to_owned(), qualified_field.field_name.clone()))
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        .iter()
                        .filter(|field| {
                            exact_scope(&field.normalized["scope"]) == Some(callable.scope.as_str())
                        })
                        .copied()
                        .collect::<Vec<_>>();
                    let [field] = field_candidates.as_slice() else {
                        gaps.add(
                            if field_candidates.is_empty() {
                                "PROCESS_QUALIFIED_FIELD_NOT_CAPTURED"
                            } else {
                                "PROCESS_QUALIFIED_FIELD_AMBIGUOUS"
                            },
                            json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"field":qualified_field.field_name,"scope":callable.scope,"candidateCount":field_candidates.len()}),
                        );
                        continue;
                    };
                    if !has_modifier(&field.normalized["modifiers"], "STATIC") {
                        gaps.add(
                            "PROCESS_QUALIFIED_FIELD_NOT_STATIC",
                            json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"field":qualified_field.field_name,"scope":callable.scope}),
                        );
                        continue;
                    }
                    retain_process_type_context(
                        evidence,
                        service,
                        type_row,
                        Some(source_id),
                        &mut dependencies,
                        &mut sources,
                        &mut source_contexts,
                        &mut gaps,
                        &mut unique_source_bytes,
                    );
                    dependencies.insert(field.id.clone(), *field);
                    owner_field_candidates.insert(field.id.clone());
                    source_contexts.insert(SourceContext {
                        kind: "STATIC_FIELD_REFERENCE".into(),
                        declaration_id: field.id.clone(),
                        source_id: None,
                        referenced_from_source_id: Some(source_id.to_owned()),
                    });
                }
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
                            &mut edge_keys,
                            Edge {
                                from: callable.identity.clone(),
                                target: Some(candidate.symbol.clone()),
                                scope: callable.scope.clone(),
                                kind: "CALLS".into(),
                            authority: "SOURCE_REFERENCE_CANDIDATE".into(),
                            fact_id: None,
                            source_id: Some(source_id.to_owned()),
                            receiver_field_id: None,
                            },
                        );
                    }
                    [] if !scope_candidates.is_empty() => gaps.add(
                        "SOURCE_REFERENCE_SCOPE_UNAVAILABLE",
                        json!({"from":callable.identity,"name":name,"scope":callable.scope}),
                    ),
                    [] if process_root_id.is_some()
                        && provider_targets.iter().any(|target| {
                            indexes
                                .methods_by_identity_scope
                                .get(&(target.clone(), callable.scope.clone()))
                                .is_some_and(|methods| {
                                    methods.iter().any(|method| {
                                        method.normalized["name"] == name
                                    })
                                })
                        }) => {}
                    [] if process_root_id.is_some() => gaps.add(
                        "SOURCE_HELPER_DECLARATION_NOT_CAPTURED",
                        json!({"from":callable.identity,"owner":callable.owner,"name":name,"scope":callable.scope}),
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
                    [candidate] => {
                        if provider_reference_targets.contains(&candidate.symbol) {
                            continue;
                        }
                        add_edge(
                            &mut edges,
                            &mut edge_keys,
                            Edge {
                                from: callable.identity.clone(),
                                target: Some(candidate.symbol.clone()),
                                scope: callable.scope.clone(),
                                kind: "METHOD_REFERENCE".into(),
                                authority: "SOURCE_REFERENCE_CANDIDATE".into(),
                            fact_id: None,
                            source_id: Some(source_id.to_owned()),
                            receiver_field_id: None,
                            },
                        );
                    }
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
            if discoveries.field_receiver_lambda_ambiguous {
                gaps.add(
                    "SOURCE_FIELD_RECEIVER_LAMBDA_SCOPE_AMBIGUOUS",
                    json!({"symbol":callable.identity}),
                );
            }
            for receiver_call in discoveries.field_receiver_calls {
                let same_owner = indexes
                    .fields_by_owner_name
                    .get(&(callable.owner.clone(), receiver_call.field_name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let same_scope_field_exists = same_owner.iter().any(|field| {
                    exact_scope(&field.normalized["scope"]) == Some(callable.scope.as_str())
                });
                if receiver_call.shadowed && !receiver_call.explicit_receiver {
                    if process_root_id.is_some() {
                        gaps.add(
                            "SOURCE_RECEIVER_SHADOWING_AMBIGUOUS",
                            json!({"from":callable.identity,"receiver":receiver_call.field_name,"method":receiver_call.method_name,"scope":callable.scope}),
                        );
                    }
                    continue;
                }
                if process_root_id.is_some()
                    && !receiver_call.explicit_receiver
                    && !same_scope_field_exists
                {
                    let type_candidates = indexes
                        .types_by_name_scope
                        .get(&(receiver_call.field_name.clone(), callable.scope.clone()))
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    match type_candidates {
                        [type_row] => {
                            let target_owner = type_row.normalized["symbolIdentity"]
                                .as_str()
                                .unwrap_or_default();
                            let owner_methods = indexes
                                .methods_by_owner_name
                                .get(&(target_owner.to_owned(), receiver_call.method_name.clone()))
                                .map(Vec::as_slice)
                                .unwrap_or_default();
                            let method_scope_candidates: Vec<_> = owner_methods
                                .iter()
                                .filter(|method| method.normalized["declarationKind"] == "METHOD")
                                .filter(|method| {
                                    exact_scope(&method.normalized["scope"])
                                        == Some(callable.scope.as_str())
                                })
                                .collect();
                            let Some(source_arity) = receiver_call.argument_count else {
                                gaps.add(
                                    "SOURCE_QUALIFIED_CALL_ARGUMENTS_UNSUPPORTED",
                                    json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"scope":callable.scope}),
                                );
                                continue;
                            };
                            let matching_arity: Vec<_> = method_scope_candidates
                                .iter()
                                .filter(|method| {
                                    method.normalized["jvmDescriptor"].as_str().and_then(
                                        |descriptor| {
                                            method_descriptor_parameter_count(descriptor).ok()
                                        },
                                    ) == Some(source_arity)
                                })
                                .copied()
                                .collect();
                            let static_candidates: Vec<_> = matching_arity
                                .iter()
                                .filter(|method| {
                                    has_modifier(&method.normalized["modifiers"], "STATIC")
                                })
                                .copied()
                                .collect();
                            let target = match static_candidates.as_slice() {
                                [target] => *target,
                                [] if !matching_arity.is_empty() => {
                                    gaps.add(
                                        "SOURCE_QUALIFIED_TARGET_NOT_STATIC",
                                        json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"method":receiver_call.method_name,"scope":callable.scope}),
                                    );
                                    continue;
                                }
                                [] if !method_scope_candidates.is_empty() => {
                                    gaps.add(
                                        "SOURCE_QUALIFIED_CALL_ARITY_UNMATCHED",
                                        json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"method":receiver_call.method_name,"argumentCount":source_arity,"scope":callable.scope}),
                                    );
                                    continue;
                                }
                                [] => {
                                    gaps.add(
                                        "SOURCE_QUALIFIED_TARGET_DECLARATION_NOT_CAPTURED",
                                        json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"method":receiver_call.method_name,"scope":callable.scope}),
                                    );
                                    continue;
                                }
                                _ => {
                                    gaps.add(
                                        "SOURCE_QUALIFIED_TARGET_AMBIGUOUS",
                                        json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"method":receiver_call.method_name,"candidateCount":static_candidates.len(),"scope":callable.scope}),
                                    );
                                    continue;
                                }
                            };
                            let target_callable = callable_from_observation(target);
                            let target_source = method_body_source(evidence, &target_callable)
                                .ok()
                                .filter(|(_, source)| {
                                    source_steps::method_body(
                                        &source.text,
                                        &target_callable.identity,
                                    )
                                    .is_some()
                                })
                                .map(|(id, _)| id)
                                .or_else(|| {
                                    type_row.source_ids.iter().find_map(|source_id| {
                                        let source = evidence.sources.get(source_id)?;
                                        (source.service == service
                                            && source.revision == evidence.revision
                                            && source.text_digest
                                                == crate::canonical::hash_bytes(
                                                    source.text.as_bytes(),
                                                )
                                            && source_steps::method_body(
                                                &source.text,
                                                &target_callable.identity,
                                            )
                                            .is_some())
                                        .then(|| source_id.clone())
                                    })
                                });
                            let Some(target_source) = target_source else {
                                gaps.add(
                                    "SOURCE_QUALIFIED_CALL_BODY_UNAVAILABLE",
                                    json!({"from":callable.identity,"typeReference":work_reference(work,"DEPENDENCY",&type_row.id),"target":target.symbol,"scope":callable.scope}),
                                );
                                continue;
                            };
                            let target_key = (target.symbol.clone(), callable.scope.clone());
                            candidate_body_sources.insert(target_key, target_source);
                            dependencies.insert(type_row.id.clone(), *type_row);
                            if provider_targets.contains(&target.symbol) {
                                continue;
                            }
                            add_edge(
                                &mut edges,
                                &mut edge_keys,
                                Edge {
                                    from: callable.identity.clone(),
                                    target: Some(target.symbol.clone()),
                                    scope: callable.scope.clone(),
                                    kind: "TYPE_QUALIFIED_CALL".into(),
                                    authority: "SOURCE_REFERENCE_CANDIDATE".into(),
                                    fact_id: None,
                                    source_id: Some(source_id.to_owned()),
                                    receiver_field_id: None,
                                },
                            );
                            continue;
                        }
                        [] => {}
                        _ => {
                            gaps.add(
                                "SOURCE_QUALIFIED_TYPE_AMBIGUOUS",
                                json!({"from":callable.identity,"receiver":receiver_call.field_name,"method":receiver_call.method_name,"scope":callable.scope,"candidateCount":type_candidates.len()}),
                            );
                            continue;
                        }
                    }
                }
                if process_root_id.is_some()
                    && !receiver_call.explicit_receiver
                    && !same_scope_field_exists
                {
                    gaps.add(
                        "SOURCE_QUALIFIED_RECEIVER_TYPE_NOT_CAPTURED",
                        json!({"from":callable.identity,"receiver":receiver_call.field_name,"method":receiver_call.method_name,"scope":callable.scope}),
                    );
                    continue;
                }
                if same_owner.is_empty() {
                    if process_root_id.is_some() {
                        gaps.add(
                            "SOURCE_FIELD_RECEIVER_FIELD_NOT_CAPTURED",
                            json!({"from":callable.identity,"field":receiver_call.field_name,"method":receiver_call.method_name,"scope":callable.scope}),
                        );
                    }
                    continue;
                }
                if receiver_call.shadowed && !receiver_call.explicit_receiver {
                    // The ordinary field-reference pass records the bounded
                    // shadowing gap for this same source identifier.
                    continue;
                }
                let fields: Vec<_> = same_owner
                    .iter()
                    .filter(|field| {
                        exact_scope(&field.normalized["scope"]) == Some(callable.scope.as_str())
                    })
                    .collect();
                let field = match fields.as_slice() {
                    [field] => *field,
                    [] => continue,
                    _ => continue,
                };
                let Some(field_reference) = work_reference(work, "DEPENDENCY", &field.id) else {
                    gaps.add(
                        "SOURCE_FIELD_RECEIVER_EVIDENCE_UNAVAILABLE",
                        json!({"from":callable.identity,"scope":callable.scope}),
                    );
                    continue;
                };
                let Some(descriptor) = field.normalized["jvmDescriptor"].as_str() else {
                    gaps.add(
                        "SOURCE_FIELD_RECEIVER_TYPE_UNSUPPORTED",
                        json!({"from":callable.identity,"scope":callable.scope}),
                    );
                    continue;
                };
                let Ok(target_owner) = field_descriptor_class_identity(descriptor) else {
                    gaps.add(
                        "SOURCE_FIELD_RECEIVER_TYPE_UNSUPPORTED",
                        json!({"from":callable.identity,"scope":callable.scope}),
                    );
                    continue;
                };
                let owner_methods = indexes
                    .methods_by_owner_name
                    .get(&(target_owner.clone(), receiver_call.method_name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let method_scope_candidates: Vec<_> = owner_methods
                    .iter()
                    .filter(|method| method.normalized["declarationKind"] == "METHOD")
                    .filter(|method| {
                        exact_scope(&method.normalized["scope"]) == Some(callable.scope.as_str())
                    })
                    .collect();
                let target = match method_scope_candidates.as_slice() {
                    [target] => *target,
                    [] if !owner_methods.is_empty() => {
                        gaps.add(
                            "SOURCE_FIELD_RECEIVER_TARGET_SCOPE_UNAVAILABLE",
                            json!({"from":callable.identity,"fieldReference":field_reference,"scope":callable.scope}),
                        );
                        continue;
                    }
                    [] if process_root_id.is_some() && owner_methods.is_empty() => {
                        gaps.add(
                            "SOURCE_FIELD_RECEIVER_TARGET_DECLARATION_NOT_CAPTURED",
                            json!({"from":callable.identity,"fieldReference":field_reference,"targetOwner":target_owner,"method":receiver_call.method_name,"scope":callable.scope}),
                        );
                        continue;
                    }
                    [] => continue,
                    _ => {
                        gaps.add(
                            "SOURCE_FIELD_RECEIVER_AMBIGUOUS_OVERLOAD",
                            json!({"from":callable.identity,"fieldReference":field_reference,"scope":callable.scope,"candidateCount":method_scope_candidates.len()}),
                        );
                        continue;
                    }
                };
                let Some(source_arity) = receiver_call.argument_count else {
                    gaps.add(
                        "SOURCE_FIELD_RECEIVER_ARGUMENTS_UNSUPPORTED",
                        json!({"from":callable.identity,"fieldReference":field_reference,"scope":callable.scope}),
                    );
                    continue;
                };
                let Some(target_arity) = target.normalized["jvmDescriptor"]
                    .as_str()
                    .and_then(|descriptor| method_descriptor_parameter_count(descriptor).ok())
                else {
                    gaps.add(
                        "SOURCE_FIELD_RECEIVER_DESCRIPTOR_UNSUPPORTED",
                        json!({"from":callable.identity,"fieldReference":field_reference,"scope":callable.scope}),
                    );
                    continue;
                };
                if source_arity != target_arity {
                    gaps.add(
                        "SOURCE_FIELD_RECEIVER_ARGUMENT_COUNT_MISMATCH",
                        json!({"from":callable.identity,"fieldReference":field_reference,"scope":callable.scope}),
                    );
                    continue;
                }
                let target_callable = callable_from_observation(target);
                let body_available = method_body_source(evidence, &target_callable)
                    .ok()
                    .is_some_and(|(_, body_source)| {
                        source_steps::method_body(&body_source.text, &target_callable.identity)
                            .is_some()
                    });
                if !body_available {
                    gaps.add(
                        "SOURCE_FIELD_RECEIVER_BODY_UNAVAILABLE",
                        json!({"from":callable.identity,"fieldReference":field_reference,"scope":callable.scope}),
                    );
                    continue;
                }
                if provider_targets.contains(&target.symbol) {
                    continue;
                }
                add_edge(
                    &mut edges,
                    &mut edge_keys,
                    Edge {
                        from: callable.identity.clone(),
                        target: Some(target.symbol.clone()),
                        scope: callable.scope.clone(),
                        kind: "FIELD_RECEIVER_CALL".into(),
                        authority: "SOURCE_REFERENCE_CANDIDATE".into(),
                        fact_id: None,
                        source_id: Some(source_id.to_owned()),
                        receiver_field_id: Some(field.id.clone()),
                    },
                );
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

        for target in unique_targets_for_caller(&edges, &callable.identity, &callable.scope) {
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
                    if process_root_id.is_some() {
                        dependencies.insert(candidate.id.clone(), *candidate);
                    }
                    let target_callable = callable_from_observation(candidate);
                    let source_candidate = candidate_body_sources
                        .get(&target_key)
                        .cloned()
                        .map(|source_id| {
                            evidence
                                .sources
                                .get(&source_id)
                                .map(|source| (source_id, source))
                                .ok_or("METHOD_BODY_SOURCE_UNAVAILABLE")
                        })
                        .transpose();
                    let body_source = match source_candidate {
                        Ok(Some(body)) => Ok(body),
                        Ok(None) => method_body_source(evidence, &target_callable),
                        Err(code) => Err(code),
                    };
                    let (source_id, source) = match body_source {
                        Ok(body) => body,
                        Err(code) => {
                            gaps.add(code, json!({"symbol":target_callable.identity}));
                            continue;
                        }
                    };
                    if source_steps::method_body(&source.text, &target_callable.identity).is_none()
                    {
                        gaps.add(
                            "METHOD_BODY_PARSE_UNAVAILABLE",
                            json!({"symbol":target_callable.identity}),
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

        // Preserve each selected callable's provider facts without synthesizing call order.
        for observation in caller_flows.iter().chain(caller_relations.iter()).copied() {
            if !dependencies.contains_key(&observation.id) {
                dependencies.insert(observation.id.clone(), observation);
            }
        }
    }

    if process_root_id.is_some()
        && let Some(scope) = root_scope.as_deref().filter(|scope| !scope.is_empty())
    {
        expand_process_source_contexts(
            evidence,
            service,
            scope,
            &indexes,
            &mut dependencies,
            &mut sources,
            &mut source_contexts,
            &mut owner_field_candidates,
            &mut gaps,
            &mut unique_source_bytes,
        );
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
            process_root: process_root_id.and_then(|id| evidence.observations.get(id)),
            dependencies,
            sources,
            nodes,
            node_keys,
            edges,
            source_contexts,
            dto_groups,
            owner_groups: Vec::new(),
            gaps,
            body_source_bytes,
            field_source_bytes: dto_source_bytes,
            unique_source_bytes,
            max_observed_traversal_depth,
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
    let mut selected_dto = Vec::new();
    for id in &dto_field_ids {
        let Some(field) = evidence.observations.get(id) else {
            gaps.add("DTO_FIELD_RECORD_UNAVAILABLE", json!({"id":id}));
            continue;
        };
        account_field_tokens(field, &mut unique_source_bytes, &mut dto_source_bytes);
        selected_dto.push(field);
    }
    for field in &selected_dto {
        dependencies.insert(field.id.clone(), *field);
    }

    let receiver_field_ids: BTreeSet<_> = edges
        .iter()
        .filter(|edge| edge.authority == "SOURCE_REFERENCE_CANDIDATE")
        .filter_map(|edge| edge.receiver_field_id.clone())
        .collect();
    for id in receiver_field_ids {
        if dependencies.contains_key(&id) {
            continue;
        }
        let Some(field) = evidence.observations.get(&id) else {
            gaps.add(
                "SOURCE_FIELD_RECEIVER_EVIDENCE_UNAVAILABLE",
                json!({"id":id}),
            );
            continue;
        };
        account_field_tokens(field, &mut unique_source_bytes, &mut dto_source_bytes);
        dependencies.insert(id, field);
    }

    let owner_ids: Vec<_> = owner_field_candidates.into_iter().collect();
    for id in owner_ids {
        let Some(field) = evidence.observations.get(&id) else {
            gaps.add("REFERENCED_FIELD_RECORD_UNAVAILABLE", json!({"id":id}));
            continue;
        };
        if dependencies.contains_key(&id) {
            owner_field_ids.insert(id);
            continue;
        }
        account_field_tokens(field, &mut unique_source_bytes, &mut dto_source_bytes);
        owner_field_ids.insert(id.clone());
        dependencies.insert(id, field);
    }

    if process_root_id.is_some() {
        let mut type_queue = VecDeque::<String>::new();
        for observation in dependencies.values() {
            if observation.kind != "SYMBOL" {
                continue;
            }
            if let Some(owner) = observation.normalized["ownerIdentity"].as_str() {
                type_queue.push_back(owner.to_owned());
            }
            if observation.normalized["declarationKind"] == "FIELD" {
                let descriptor = observation.normalized["jvmDescriptor"]
                    .as_str()
                    .or_else(|| observation.normalized["typeDescriptor"].as_str());
                if let Some(descriptor) = descriptor
                    && let Ok(identity) = field_descriptor_class_identity(descriptor)
                {
                    type_queue.push_back(identity);
                }
            }
        }
        let mut visited_types = BTreeSet::new();
        while let Some(identity) = type_queue.pop_front() {
            if !visited_types.insert(identity.clone()) {
                continue;
            }
            let type_rows = indexes
                .types_by_identity_scope
                .get(&(identity.clone(), scope.to_owned()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let [type_row] = type_rows else {
                gaps.add(
                    if type_rows.is_empty() {
                        "PROCESS_TYPE_DECLARATION_NOT_CAPTURED"
                    } else {
                        "PROCESS_TYPE_DECLARATION_AMBIGUOUS"
                    },
                    json!({"identity":identity,"scope":scope,"candidateCount":type_rows.len()}),
                );
                continue;
            };
            dependencies.insert(type_row.id.clone(), *type_row);
            for source_id in &type_row.source_ids {
                match evidence.sources.get(source_id) {
                    Some(source)
                        if source.service == service
                            && source.revision == evidence.revision
                            && source.text_digest
                                == crate::canonical::hash_bytes(source.text.as_bytes()) =>
                    {
                        sources.insert(source_id.clone(), source);
                    }
                    _ => gaps.add(
                        "PROCESS_TYPE_SOURCE_UNAVAILABLE",
                        json!({"identity":identity,"sourceId":source_id}),
                    ),
                }
            }
            if let Some(superclass) = type_row.normalized["superclass"].as_str() {
                if !superclass.trim().is_empty() {
                    type_queue.push_back(superclass.to_owned());
                }
            }
            for interface in type_row.normalized["interfaces"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|interface| !interface.trim().is_empty())
            {
                type_queue.push_back(interface.to_owned());
            }
        }
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
        process_root: process_root_id.and_then(|id| evidence.observations.get(id)),
        dependencies,
        sources,
        nodes,
        node_keys,
        edges,
        source_contexts,
        dto_groups,
        owner_groups,
        gaps,
        body_source_bytes,
        field_source_bytes: dto_source_bytes,
        unique_source_bytes,
        max_observed_traversal_depth,
    })
}

fn finish_rows(input: FinishRows<'_>) -> Result<Vec<Value>, ClewError> {
    let FinishRows {
        work,
        evidence,
        entrypoint_id,
        entry,
        process_root,
        dependencies,
        sources,
        nodes,
        node_keys,
        edges,
        source_contexts,
        dto_groups,
        owner_groups,
        mut gaps,
        body_source_bytes,
        field_source_bytes,
        unique_source_bytes,
        max_observed_traversal_depth,
    } = input;
    let mut rows = Vec::new();
    let delivered_sources: BTreeSet<_> = sources.keys().cloned().collect();
    let delivered_source_refs: BTreeSet<_> = delivered_sources
        .iter()
        .filter_map(|id| work_reference(work, "SOURCE", id))
        .collect();
    let mut provider_fact_references = BTreeSet::new();
    let mut source_reference_candidates = Vec::new();
    let mut source_context_rows = Vec::new();
    for edge in edges {
        if let Some(fact_id) = edge.fact_id.as_deref() {
            if let Some(reference) = work_reference(work, "DEPENDENCY", fact_id) {
                provider_fact_references.insert(reference.to_owned());
            } else {
                gaps.add(
                    "PROVIDER_EDGE_EVIDENCE_REFERENCE_UNAVAILABLE",
                    json!({"factId":fact_id,"from":edge.from,"target":edge.target}),
                );
            }
            continue;
        }
        if edge.authority != "SOURCE_REFERENCE_CANDIDATE" {
            continue;
        }
        if let Some(field_id) = edge.receiver_field_id.as_ref()
            && (!dependencies.contains_key(field_id)
                || work_reference(work, "DEPENDENCY", field_id).is_none())
        {
            gaps.add(
                "SOURCE_REFERENCE_CANDIDATE_RECEIVER_FIELD_EVIDENCE_UNAVAILABLE",
                json!({"from":edge.from,"target":edge.target,"fieldId":field_id}),
            );
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
        let mut candidate = json!({
            "fromNode":from_node,
            "toNode":to_node,
            "targetIdentity":if to_node.is_none(){edge.target}else{None::<String>},
            "kind":edge.kind,
            "authority":edge.authority,
        });
        if process_root.is_some() {
            candidate["sourceReference"] = json!(
                edge.source_id
                    .as_deref()
                    .and_then(|id| work_reference(work, "SOURCE", id))
            );
        }
        if let Some(field_id) = edge.receiver_field_id.as_deref() {
            candidate["receiverFieldReference"] =
                json!(work_reference(work, "DEPENDENCY", field_id));
        }
        source_reference_candidates.push(candidate);
    }
    gaps.add("STRUCTURAL_DATAFLOW_NOT_AVAILABLE", Value::Null);
    gaps.add("DIRECT_DECLARED_TYPES_ONLY", Value::Null);
    gaps.add("SOURCE_REFERENCE_CANDIDATE_AUTHORITY", Value::Null);
    if process_root.is_some() {
        for candidate in source_contexts {
            let declaration_reference =
                work_reference(work, "DEPENDENCY", &candidate.declaration_id);
            let source_reference = candidate
                .source_id
                .as_deref()
                .and_then(|id| work_reference(work, "SOURCE", id));
            let referenced_from_source_reference = candidate
                .referenced_from_source_id
                .as_deref()
                .and_then(|id| work_reference(work, "SOURCE", id));
            let Some(declaration_reference) = declaration_reference else {
                gaps.add(
                    "SOURCE_CONTEXT_DECLARATION_REFERENCE_UNAVAILABLE",
                    json!({"declarationId":candidate.declaration_id,"kind":candidate.kind}),
                );
                continue;
            };
            if candidate.source_id.is_some() && source_reference.is_none() {
                gaps.add(
                    "SOURCE_CONTEXT_SOURCE_REFERENCE_UNAVAILABLE",
                    json!({"declarationId":candidate.declaration_id,"sourceId":candidate.source_id,"kind":candidate.kind}),
                );
                continue;
            }
            if candidate.referenced_from_source_id.is_some()
                && referenced_from_source_reference.is_none()
            {
                gaps.add(
                    "SOURCE_CONTEXT_ORIGIN_REFERENCE_UNAVAILABLE",
                    json!({"declarationId":candidate.declaration_id,"sourceId":candidate.referenced_from_source_id,"kind":candidate.kind}),
                );
                continue;
            }
            let Some(declaration) = dependencies.get(&candidate.declaration_id) else {
                gaps.add(
                    "SOURCE_CONTEXT_DECLARATION_UNAVAILABLE",
                    json!({"declarationId":candidate.declaration_id,"kind":candidate.kind}),
                );
                continue;
            };
            let mut evidence = vec![declaration_reference.to_owned()];
            if let Some(reference) = source_reference {
                evidence.push(reference.to_owned());
            }
            if let Some(reference) = referenced_from_source_reference {
                if !evidence.iter().any(|item| item == reference) {
                    evidence.push(reference.to_owned());
                }
            }
            source_context_rows.push(json!({
                "kind":candidate.kind,
                "authority":"SOURCE_REFERENCE_CANDIDATE",
                "symbolIdentity":declaration.normalized["symbolIdentity"],
                "ownerIdentity":declaration.normalized["ownerIdentity"],
                "scope":declaration.normalized["scope"],
                "declarationReference":declaration_reference,
                "sourceReference":source_reference,
                "referencedFromSourceReference":referenced_from_source_reference,
                "evidence":evidence
            }));
        }
    }
    if let Some(entry) = entry {
        let root_node = (!nodes.is_empty()).then_some("m0");
        let dto_field_count = dto_groups
            .iter()
            .flat_map(|group| group["fieldReferences"].as_array().into_iter().flatten())
            .count();
        let referenced_owner_field_count = owner_groups
            .iter()
            .flat_map(|group| group["fieldReferences"].as_array().into_iter().flatten())
            .count();
        let callable_count = nodes.len();
        let provider_edge_fact_count = provider_fact_references.len();
        let source_reference_candidate_count = source_reference_candidates.len();
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
                    "maxObservedTraversalDepth":max_observed_traversal_depth
                },
                "referencedOwnerFields":owner_groups,
                "selection":{
                    "uniqueSourceBytes":unique_source_bytes,
                    "uniqueBodySourceBytes":body_source_bytes,
                    "fieldTokenBytes":field_source_bytes,
                    "callableCount":callable_count,
                    "providerEdgeFactCount":provider_edge_fact_count,
                    "sourceReferenceCandidateCount":source_reference_candidate_count,
                    "dtoFieldCount":dto_field_count,
                    "referencedOwnerFieldCount":referenced_owner_field_count,
                    "maxObservedTraversalDepth":max_observed_traversal_depth
                },
                "gaps":gaps.rows(),
            }
        }));
    } else if let Some(root) = process_root {
        let root_reference = work_reference(work, "DEPENDENCY", &root.id);
        let root_identity = root.normalized["symbolIdentity"]
            .as_str()
            .unwrap_or(&root.symbol);
        let root_evidence = root_reference.into_iter().collect::<Vec<_>>();
        let provider_edge_fact_count = provider_fact_references.len();
        let source_reference_candidate_count = source_reference_candidates.len();
        let source_context_count = source_context_rows.len();
        let referenced_owner_field_count = owner_groups
            .iter()
            .flat_map(|group| group["fieldReferences"].as_array().into_iter().flatten())
            .count();
        let mut process_nodes = nodes.clone();
        for node in &mut process_nodes {
            let identity = node["symbolIdentity"].as_str().unwrap_or_default();
            let scope = node["scope"].as_str().unwrap_or_default();
            let declarations: Vec<_> = dependencies
                .values()
                .filter(|observation| {
                    observation.kind == "SYMBOL"
                        && observation.normalized["symbolIdentity"] == identity
                        && observation.normalized["scope"] == scope
                })
                .collect();
            if let [declaration] = declarations.as_slice() {
                let reference = work_reference(work, "DEPENDENCY", &declaration.id);
                node["declarationReference"] = json!(reference);
                node["ownerIdentity"] = declaration.normalized["ownerIdentity"].clone();
                node["name"] = declaration.normalized["name"].clone();
                node["evidence"] = json!(reference.into_iter().collect::<Vec<_>>());
            }
        }
        rows.push(json!({
            "kind":"PROCESS_CONTEXT_PACKET",
            "id":format!("process-context:{entrypoint_id}"),
            "record":{
                "profile":"process-graph-v1",
                "authority":"DERIVED_NAVIGATION_ONLY",
                "rootDeclarationReference":root_reference,
                "root":{
                    "declarationId":root.id,
                    "symbolIdentity":root_identity,
                    "ownerIdentity":root.normalized["ownerIdentity"],
                    "name":root.normalized["name"],
                    "scope":root.normalized["scope"],
                    "evidence":root_evidence
                },
                "callGraph":{
                    "authority":"RETAINED_TARGET_RELATIONS_AND_SOURCE_CANDIDATES",
                    "order":"NOT_EXECUTION_ORDER",
                    "rootNode":(!nodes.is_empty()).then_some("m0"),
                    "nodes":process_nodes,
                    "providerEdgeFactReferences":provider_fact_references,
                    "sourceReferenceCandidates":source_reference_candidates,
                    "maxObservedTraversalDepth":max_observed_traversal_depth
                },
                "sourceContexts":source_context_rows,
                "referencedOwnerFields":owner_groups,
                "selection":{
                    "uniqueSourceBytes":unique_source_bytes,
                    "uniqueBodySourceBytes":body_source_bytes,
                    "fieldTokenBytes":field_source_bytes,
                    "callableCount":nodes.len(),
                    "providerEdgeFactCount":provider_edge_fact_count,
                    "sourceReferenceCandidateCount":source_reference_candidate_count,
                    "sourceContextCount":source_context_count,
                    "referencedOwnerFieldCount":referenced_owner_field_count,
                    "maxObservedTraversalDepth":max_observed_traversal_depth
                },
                "gaps":gaps.rows()
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

fn declaration_indexes<'a>(
    evidence: &'a ServiceEvidence,
    work: &Work,
    service: &str,
) -> DeclarationIndexes<'a> {
    let mut indexes = DeclarationIndexes {
        methods_by_identity_scope: BTreeMap::new(),
        methods_by_owner_name: BTreeMap::new(),
        methods_by_owner_scope: BTreeMap::new(),
        method_scopes_by_identity: BTreeMap::new(),
        types_by_identity_scope: BTreeMap::new(),
        types_by_name_scope: BTreeMap::new(),
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
                                .methods_by_owner_scope
                                .entry((owner.to_owned(), scope.clone()))
                                .or_default()
                                .push(observation);
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
                            let name = observation.normalized["name"].as_str().unwrap_or_default();
                            if !name.is_empty() {
                                indexes
                                    .types_by_name_scope
                                    .entry((name.to_owned(), scope.clone()))
                                    .or_default()
                                    .push(observation);
                            }
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
    for values in indexes.methods_by_owner_scope.values_mut() {
        values.sort_by(|a, b| a.id.cmp(&b.id));
    }
    for values in indexes.types_by_identity_scope.values_mut() {
        values.sort_by(|a, b| a.id.cmp(&b.id));
    }
    for values in indexes.types_by_name_scope.values_mut() {
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
    Ok((source_id.to_owned(), source))
}

fn process_type_candidates<'a>(
    indexes: &DeclarationIndexes<'a>,
    type_name: &str,
    scope: &str,
) -> Vec<&'a Observation> {
    if type_name.contains('.') {
        return indexes
            .types_by_identity_scope
            .get(&(format!("class:{type_name}"), scope.to_owned()))
            .map(Vec::as_slice)
            .unwrap_or_default()
            .to_vec();
    }
    indexes
        .types_by_name_scope
        .get(&(type_name.to_owned(), scope.to_owned()))
        .map(Vec::as_slice)
        .unwrap_or_default()
        .to_vec()
}

fn constructor_target_owner_identity(target: &str) -> Option<String> {
    let method_identity = target.strip_prefix("method:")?;
    let (owner, _) = method_identity.split_once('#')?;
    owner.starts_with("class:").then(|| owner.to_owned())
}

fn retain_process_type_context<'a>(
    evidence: &'a ServiceEvidence,
    service: &str,
    type_row: &'a Observation,
    referenced_from_source_id: Option<&str>,
    dependencies: &mut BTreeMap<String, &'a Observation>,
    sources: &mut BTreeMap<String, &'a Source>,
    source_contexts: &mut BTreeSet<SourceContext>,
    gaps: &mut Gaps,
    unique_source_bytes: &mut usize,
) {
    dependencies.insert(type_row.id.clone(), type_row);
    for source_id in &type_row.source_ids {
        let Some(source) = evidence.sources.get(source_id) else {
            gaps.add(
                "PROCESS_TYPE_SOURCE_UNAVAILABLE",
                json!({"identity":type_row.normalized["symbolIdentity"],"sourceId":source_id}),
            );
            continue;
        };
        if source.service != service
            || source.revision != evidence.revision
            || source.text.is_empty()
            || source.text_digest != crate::canonical::hash_bytes(source.text.as_bytes())
        {
            gaps.add(
                "PROCESS_TYPE_SOURCE_PROVENANCE_INVALID",
                json!({"identity":type_row.normalized["symbolIdentity"],"sourceId":source_id}),
            );
            continue;
        }
        if !sources.contains_key(source_id) {
            *unique_source_bytes += source.text.len();
        }
        sources.insert(source_id.clone(), source);
        source_contexts.insert(SourceContext {
            kind: "TYPE_SOURCE".into(),
            declaration_id: type_row.id.clone(),
            source_id: Some(source_id.clone()),
            referenced_from_source_id: referenced_from_source_id.map(str::to_owned),
        });
    }
    if type_row.source_ids.is_empty() {
        gaps.add(
            "PROCESS_TYPE_SOURCE_UNAVAILABLE",
            json!({"identity":type_row.normalized["symbolIdentity"]}),
        );
    }
}

fn process_context_method_body_source<'a>(
    evidence: &'a ServiceEvidence,
    service: &str,
    callable: &Callable<'a>,
    containing_source_ids: &[String],
) -> Result<(String, &'a Source, (usize, usize)), &'static str> {
    let mut containing = BTreeMap::<String, &'a Source>::new();
    for source_id in containing_source_ids {
        let Some(source) = evidence.sources.get(source_id) else {
            continue;
        };
        if source.service != service
            || source.revision != evidence.revision
            || source.text.is_empty()
            || source.text_digest != crate::canonical::hash_bytes(source.text.as_bytes())
        {
            continue;
        }
        if source_steps::method_body(&source.text, &callable.identity).is_some() {
            containing.insert(source_id.clone(), source);
        }
    }
    match containing.into_iter().collect::<Vec<_>>().as_slice() {
        [(source_id, source)] => {
            let range = source_steps::method_body(&source.text, &callable.identity)
                .ok_or("SOURCE_CONTEXT_METHOD_BODY_UNAVAILABLE")?;
            return Ok((source_id.clone(), source, range));
        }
        [_, _, ..] => return Err("SOURCE_CONTEXT_METHOD_BODY_AMBIGUOUS"),
        [] => {}
    }
    if let Ok((source_id, source)) = method_body_source(evidence, callable)
        && source.service == service
        && source.revision == evidence.revision
        && !source.text.is_empty()
        && source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
        && let Some(range) = source_steps::method_body(&source.text, &callable.identity)
    {
        return Ok((source_id, source, range));
    }
    Err("SOURCE_CONTEXT_METHOD_BODY_UNAVAILABLE")
}

fn enqueue_process_context_method<'a>(
    evidence: &'a ServiceEvidence,
    service: &str,
    indexes: &DeclarationIndexes<'a>,
    target: &'a Observation,
    from_source_id: &str,
    dependencies: &mut BTreeMap<String, &'a Observation>,
    sources: &mut BTreeMap<String, &'a Source>,
    source_contexts: &mut BTreeSet<SourceContext>,
    gaps: &mut Gaps,
    unique_source_bytes: &mut usize,
    queue: &mut VecDeque<(Callable<'a>, String, (usize, usize))>,
    scheduled: &mut BTreeSet<MethodKey>,
) {
    let callable = callable_from_observation(target);
    let key = (callable.identity.clone(), callable.scope.clone());
    dependencies.insert(target.id.clone(), target);
    let owner_types = indexes
        .types_by_identity_scope
        .get(&(callable.owner.clone(), callable.scope.clone()))
        .map(Vec::as_slice)
        .unwrap_or_default();
    if let [owner_type] = owner_types {
        retain_process_type_context(
            evidence,
            service,
            owner_type,
            Some(from_source_id),
            dependencies,
            sources,
            source_contexts,
            gaps,
            unique_source_bytes,
        );
    } else if owner_types.len() > 1 {
        gaps.add(
            "SOURCE_CONTEXT_OWNER_TYPE_AMBIGUOUS",
            json!({"method":callable.identity,"owner":callable.owner,"scope":callable.scope,"candidateCount":owner_types.len()}),
        );
    }
    let owner_source_ids: Vec<_> = owner_types
        .iter()
        .flat_map(|owner_type| owner_type.source_ids.iter().cloned())
        .collect();
    match process_context_method_body_source(evidence, service, &callable, &owner_source_ids) {
        Ok((source_id, source, body_range)) => {
            if !sources.contains_key(&source_id) {
                *unique_source_bytes += source.text.len();
            }
            sources.insert(source_id.clone(), source);
            source_contexts.insert(SourceContext {
                kind: "METHOD_SOURCE".into(),
                declaration_id: target.id.clone(),
                source_id: Some(source_id.clone()),
                referenced_from_source_id: Some(from_source_id.to_owned()),
            });
            if scheduled.insert(key) {
                queue.push_back((callable, source_id, body_range));
            }
        }
        Err(code) => {
            source_contexts.insert(SourceContext {
                kind: "METHOD_SOURCE".into(),
                declaration_id: target.id.clone(),
                source_id: None,
                referenced_from_source_id: Some(from_source_id.to_owned()),
            });
            gaps.add(
                code,
                json!({"method":target.normalized["symbolIdentity"],"owner":target.normalized["ownerIdentity"],"scope":target.normalized["scope"]}),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_process_source_contexts<'a>(
    evidence: &'a ServiceEvidence,
    service: &str,
    scope: &str,
    indexes: &DeclarationIndexes<'a>,
    dependencies: &mut BTreeMap<String, &'a Observation>,
    sources: &mut BTreeMap<String, &'a Source>,
    source_contexts: &mut BTreeSet<SourceContext>,
    owner_field_candidates: &mut BTreeSet<String>,
    gaps: &mut Gaps,
    unique_source_bytes: &mut usize,
) {
    let mut type_queue: VecDeque<String> = source_contexts
        .iter()
        .filter(|candidate| candidate.kind == "TYPE_SOURCE")
        .map(|candidate| candidate.declaration_id.clone())
        .collect();
    let mut visited_types = BTreeSet::new();
    let mut method_queue = VecDeque::<(Callable<'_>, String, (usize, usize))>::new();
    let mut scheduled_methods = BTreeSet::<MethodKey>::new();

    while !type_queue.is_empty() || !method_queue.is_empty() {
        while let Some(type_id) = type_queue.pop_front() {
            let Some(type_row) = dependencies.get(&type_id).copied() else {
                continue;
            };
            let (Some(identity), Some(type_scope)) = (
                type_row.normalized["symbolIdentity"].as_str(),
                exact_scope(&type_row.normalized["scope"]),
            ) else {
                gaps.add(
                    "SOURCE_CONTEXT_TYPE_IDENTITY_UNAVAILABLE",
                    json!({"typeDeclaration":type_id}),
                );
                continue;
            };
            if type_scope != scope
                || !visited_types.insert((identity.to_owned(), type_scope.to_owned()))
            {
                continue;
            }
            let Some(owner_methods) = indexes
                .methods_by_owner_scope
                .get(&(identity.to_owned(), scope.to_owned()))
            else {
                continue;
            };
            for method in owner_methods
                .iter()
                .filter(|method| method.normalized["declarationKind"] == "METHOD")
            {
                let callable = callable_from_observation(method);
                let key = (callable.identity.clone(), callable.scope.clone());
                if !scheduled_methods.insert(key) {
                    continue;
                }
                match process_context_method_body_source(
                    evidence,
                    service,
                    &callable,
                    &type_row.source_ids,
                ) {
                    Ok((source_id, source, body_range)) => {
                        if !sources.contains_key(&source_id) {
                            *unique_source_bytes += source.text.len();
                        }
                        sources.insert(source_id.clone(), source);
                        method_queue.push_back((callable, source_id, body_range));
                    }
                    Err("SOURCE_CONTEXT_METHOD_BODY_AMBIGUOUS") => gaps.add(
                        "SOURCE_CONTEXT_METHOD_BODY_AMBIGUOUS",
                        json!({"method":callable.identity,"owner":identity,"scope":scope}),
                    ),
                    Err(_)
                        if has_modifier(&method.normalized["modifiers"], "ABSTRACT")
                            || has_modifier(&method.normalized["modifiers"], "NATIVE") => {}
                    Err(code) => gaps.add(
                        code,
                        json!({"method":callable.identity,"owner":identity,"scope":scope}),
                    ),
                }
            }
        }

        while let Some((callable, source_id, body_range)) = method_queue.pop_front() {
            let Some(source) = sources.get(&source_id) else {
                gaps.add(
                    "SOURCE_CONTEXT_SOURCE_UNAVAILABLE",
                    json!({"method":callable.identity,"sourceId":source_id}),
                );
                continue;
            };
            let discoveries =
                lexical_discoveries(&source.text, body_range, &callable.owner, &callable.name);
            if discoveries.nested_executable_context {
                gaps.add(
                    "SOURCE_CONTEXT_NESTED_EXECUTABLE_CONTEXT_AMBIGUOUS",
                    json!({"method":callable.identity}),
                );
                continue;
            }
            for spelling in &discoveries.unsupported_qualified_references {
                gaps.add(
                    "PROCESS_QUALIFIED_SOURCE_REFERENCE_UNSUPPORTED",
                    json!({"method":callable.identity,"spelling":spelling,"scope":scope}),
                );
            }

            for type_name in discoveries.constructed_types {
                let type_candidates = process_type_candidates(indexes, &type_name, scope);
                match type_candidates.as_slice() {
                    [type_row] => {
                        retain_process_type_context(
                            evidence,
                            service,
                            type_row,
                            Some(&source_id),
                            dependencies,
                            sources,
                            source_contexts,
                            gaps,
                            unique_source_bytes,
                        );
                        type_queue.push_back(type_row.id.clone());
                    }
                    [] => gaps.add(
                        "PROCESS_SOURCE_CONTEXT_CONSTRUCTED_TYPE_NOT_CAPTURED",
                        json!({"method":callable.identity,"typeName":type_name,"scope":scope}),
                    ),
                    _ => gaps.add(
                        "PROCESS_SOURCE_CONTEXT_CONSTRUCTED_TYPE_AMBIGUOUS",
                        json!({"method":callable.identity,"typeName":type_name,"scope":scope,"candidateCount":type_candidates.len()}),
                    ),
                }
            }

            for name in discoveries
                .calls
                .into_iter()
                .chain(discoveries.method_references)
            {
                let candidates = indexes
                    .methods_by_owner_name
                    .get(&(callable.owner.clone(), name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter(|method| {
                        method.normalized["declarationKind"] == "METHOD"
                            && exact_scope(&method.normalized["scope"]) == Some(scope)
                    })
                    .copied()
                    .collect::<Vec<_>>();
                match candidates.as_slice() {
                    [candidate] => enqueue_process_context_method(
                        evidence,
                        service,
                        indexes,
                        candidate,
                        &source_id,
                        dependencies,
                        sources,
                        source_contexts,
                        gaps,
                        unique_source_bytes,
                        &mut method_queue,
                        &mut scheduled_methods,
                    ),
                    [] => gaps.add(
                        "SOURCE_CONTEXT_HELPER_NOT_CAPTURED",
                        json!({"method":callable.identity,"owner":callable.owner,"name":name,"scope":scope}),
                    ),
                    _ => gaps.add(
                        "SOURCE_CONTEXT_HELPER_AMBIGUOUS",
                        json!({"method":callable.identity,"owner":callable.owner,"name":name,"scope":scope,"candidateCount":candidates.len()}),
                    ),
                }
            }

            for receiver_call in discoveries.field_receiver_calls {
                if receiver_call.shadowed && !receiver_call.explicit_receiver {
                    gaps.add(
                        "SOURCE_CONTEXT_RECEIVER_SHADOWED",
                        json!({"method":callable.identity,"receiver":receiver_call.field_name,"target":receiver_call.method_name,"scope":scope}),
                    );
                    continue;
                }
                let same_owner_field = indexes
                    .fields_by_owner_name
                    .get(&(callable.owner.clone(), receiver_call.field_name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter(|field| exact_scope(&field.normalized["scope"]) == Some(scope))
                    .copied()
                    .collect::<Vec<_>>();
                let (target_owner, require_static, receiver_field) = match same_owner_field
                    .as_slice()
                {
                    [field] => {
                        let descriptor = field.normalized["jvmDescriptor"]
                            .as_str()
                            .or_else(|| field.normalized["typeDescriptor"].as_str());
                        let Some(target_owner) = descriptor
                            .and_then(|value| field_descriptor_class_identity(value).ok())
                        else {
                            gaps.add(
                                "SOURCE_CONTEXT_RECEIVER_FIELD_TYPE_UNSUPPORTED",
                                json!({"method":callable.identity,"field":receiver_call.field_name,"scope":scope}),
                            );
                            continue;
                        };
                        dependencies.insert(field.id.clone(), *field);
                        owner_field_candidates.insert(field.id.clone());
                        source_contexts.insert(SourceContext {
                            kind: "FIELD_RECEIVER_REFERENCE".into(),
                            declaration_id: field.id.clone(),
                            source_id: None,
                            referenced_from_source_id: Some(source_id.clone()),
                        });
                        (target_owner, false, Some(*field))
                    }
                    [] => {
                        let type_candidates =
                            process_type_candidates(indexes, &receiver_call.field_name, scope);
                        let [type_row] = type_candidates.as_slice() else {
                            gaps.add(
                                if type_candidates.is_empty() { "SOURCE_CONTEXT_RECEIVER_TYPE_NOT_CAPTURED" } else { "SOURCE_CONTEXT_RECEIVER_TYPE_AMBIGUOUS" },
                                json!({"method":callable.identity,"receiver":receiver_call.field_name,"scope":scope,"candidateCount":type_candidates.len()}),
                            );
                            continue;
                        };
                        retain_process_type_context(
                            evidence,
                            service,
                            type_row,
                            Some(&source_id),
                            dependencies,
                            sources,
                            source_contexts,
                            gaps,
                            unique_source_bytes,
                        );
                        (
                            type_row.normalized["symbolIdentity"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            true,
                            None,
                        )
                    }
                    _ => {
                        gaps.add(
                            "SOURCE_CONTEXT_RECEIVER_FIELD_AMBIGUOUS",
                            json!({"method":callable.identity,"receiver":receiver_call.field_name,"scope":scope,"candidateCount":same_owner_field.len()}),
                        );
                        continue;
                    }
                };
                let Some(source_arity) = receiver_call.argument_count else {
                    gaps.add(
                        "SOURCE_CONTEXT_RECEIVER_ARGUMENTS_UNSUPPORTED",
                        json!({"method":callable.identity,"receiver":receiver_call.field_name,"target":receiver_call.method_name,"scope":scope}),
                    );
                    continue;
                };
                let candidates =
                    indexes
                        .methods_by_owner_name
                        .get(&(target_owner.clone(), receiver_call.method_name.clone()))
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        .iter()
                        .filter(|method| {
                            method.normalized["declarationKind"] == "METHOD"
                                && exact_scope(&method.normalized["scope"]) == Some(scope)
                                && method.normalized["jvmDescriptor"].as_str().and_then(
                                    |descriptor| method_descriptor_parameter_count(descriptor).ok(),
                                ) == Some(source_arity)
                                && (!require_static
                                    || has_modifier(&method.normalized["modifiers"], "STATIC"))
                        })
                        .copied()
                        .collect::<Vec<_>>();
                match candidates.as_slice() {
                    [candidate] => {
                        if let Some(field) = receiver_field {
                            dependencies.insert(field.id.clone(), field);
                        }
                        enqueue_process_context_method(
                            evidence,
                            service,
                            indexes,
                            candidate,
                            &source_id,
                            dependencies,
                            sources,
                            source_contexts,
                            gaps,
                            unique_source_bytes,
                            &mut method_queue,
                            &mut scheduled_methods,
                        );
                    }
                    [] => gaps.add(
                        "SOURCE_CONTEXT_RECEIVER_TARGET_NOT_CAPTURED",
                        json!({"method":callable.identity,"targetOwner":target_owner,"target":receiver_call.method_name,"scope":scope}),
                    ),
                    _ => gaps.add(
                        "SOURCE_CONTEXT_RECEIVER_TARGET_AMBIGUOUS",
                        json!({"method":callable.identity,"targetOwner":target_owner,"target":receiver_call.method_name,"scope":scope,"candidateCount":candidates.len()}),
                    ),
                }
            }

            for qualified_field in discoveries.qualified_fields {
                let type_candidates =
                    process_type_candidates(indexes, &qualified_field.type_name, scope);
                if qualified_field.shadowed {
                    if !type_candidates.is_empty() {
                        gaps.add(
                            "PROCESS_QUALIFIED_FIELD_TYPE_SHADOWED",
                            json!({"method":callable.identity,"typeName":qualified_field.type_name,"field":qualified_field.field_name,"scope":scope}),
                        );
                    }
                    continue;
                }
                let [type_row] = type_candidates.as_slice() else {
                    gaps.add(
                        if type_candidates.is_empty() { "PROCESS_QUALIFIED_FIELD_TYPE_NOT_CAPTURED" } else { "PROCESS_QUALIFIED_FIELD_TYPE_AMBIGUOUS" },
                        json!({"method":callable.identity,"typeName":qualified_field.type_name,"field":qualified_field.field_name,"scope":scope,"candidateCount":type_candidates.len()}),
                    );
                    continue;
                };
                let owner = type_row.normalized["symbolIdentity"]
                    .as_str()
                    .unwrap_or_default();
                let candidates = indexes
                    .fields_by_owner_name
                    .get(&(owner.to_owned(), qualified_field.field_name.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter(|field| exact_scope(&field.normalized["scope"]) == Some(scope))
                    .copied()
                    .collect::<Vec<_>>();
                let [field] = candidates.as_slice() else {
                    gaps.add(
                        if candidates.is_empty() { "PROCESS_QUALIFIED_FIELD_NOT_CAPTURED" } else { "PROCESS_QUALIFIED_FIELD_AMBIGUOUS" },
                        json!({"method":callable.identity,"typeReference":type_row.id,"field":qualified_field.field_name,"scope":scope,"candidateCount":candidates.len()}),
                    );
                    continue;
                };
                if !has_modifier(&field.normalized["modifiers"], "STATIC") {
                    gaps.add(
                        "PROCESS_QUALIFIED_FIELD_NOT_STATIC",
                        json!({"method":callable.identity,"typeReference":type_row.id,"field":qualified_field.field_name,"scope":scope}),
                    );
                    continue;
                }
                retain_process_type_context(
                    evidence,
                    service,
                    type_row,
                    Some(&source_id),
                    dependencies,
                    sources,
                    source_contexts,
                    gaps,
                    unique_source_bytes,
                );
                dependencies.insert(field.id.clone(), *field);
                owner_field_candidates.insert(field.id.clone());
                source_contexts.insert(SourceContext {
                    kind: "STATIC_FIELD_REFERENCE".into(),
                    declaration_id: field.id.clone(),
                    source_id: None,
                    referenced_from_source_id: Some(source_id.clone()),
                });
            }
        }
    }
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

fn reference_source_within_method(
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
        let same_source_region = body.service == part.service
            && body.revision == part.revision
            && body.file == part.file
            && body.authority == part.authority
            && body.start_line <= part.start_line
            && body.end_line >= part.end_line
            && !part.text.is_empty()
            && part.start_line + part.text.lines().count().saturating_sub(1) as u64
                == part.end_line;
        if !same_source_region {
            return None;
        }
        body.text.match_indices(&part.text).find_map(|(start, _)| {
            let line = body.start_line
                + body.text[..start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count() as u64;
            (line == part.start_line).then(|| body_source_id.to_owned())
        })
    })
}

fn unique_targets_for_caller(edges: &[Edge], caller: &str, scope: &str) -> Vec<String> {
    edges
        .iter()
        .filter(|edge| edge.from == caller && edge.scope == scope)
        .filter_map(|edge| edge.target.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn add_edge(edges: &mut Vec<Edge>, edge_keys: &mut BTreeSet<Edge>, edge: Edge) {
    if !edge_keys.insert(edge.clone()) {
        return;
    }
    edges.push(edge);
}

fn work_reference<'a>(work: &'a Work, kind: &str, id: &str) -> Option<&'a str> {
    work.handles
        .iter()
        .find(|(_, handle)| handle.kind == kind && handle.id == id)
        .map(|(reference, _)| reference.as_str())
}

fn account_field_tokens(
    field: &Observation,
    total_reserved: &mut usize,
    category_reserved: &mut usize,
) {
    let Some(tokens) = field.normalized["sourceTokens"].as_array() else {
        return;
    };
    let Ok(encoded) = serde_json::to_vec(tokens) else {
        return;
    };
    *total_reserved += encoded.len();
    *category_reserved += encoded.len();
}

fn field_descriptor_class_identity(descriptor: &str) -> Result<String, ()> {
    let Some(internal_name) = descriptor
        .strip_prefix('L')
        .and_then(|descriptor| descriptor.strip_suffix(';'))
    else {
        return Err(());
    };
    if internal_name.is_empty()
        || internal_name.starts_with('/')
        || internal_name.ends_with('/')
        || internal_name.split('/').any(str::is_empty)
        || internal_name
            .bytes()
            .any(|byte| matches!(byte, b'.' | b';' | b'['))
    {
        return Err(());
    }
    Ok(format!("class:{}", internal_name.replace('/', ".")))
}

fn method_descriptor_parameter_count(descriptor: &str) -> Result<usize, ()> {
    fn parse_type(bytes: &[u8], cursor: &mut usize, allow_void: bool) -> Result<(), ()> {
        let mut dimensions = 0usize;
        while bytes.get(*cursor) == Some(&b'[') {
            dimensions += 1;
            if dimensions > 255 {
                return Err(());
            }
            *cursor += 1;
        }
        match bytes.get(*cursor).copied() {
            Some(b'V') if allow_void && dimensions == 0 => *cursor += 1,
            Some(b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z') => *cursor += 1,
            Some(b'L') => {
                let start = *cursor;
                let end = bytes[start..]
                    .iter()
                    .position(|byte| *byte == b';')
                    .map(|offset| start + offset)
                    .ok_or(())?;
                let reference = std::str::from_utf8(&bytes[start..=end]).map_err(|_| ())?;
                field_descriptor_class_identity(reference)?;
                *cursor = end + 1;
            }
            _ => return Err(()),
        }
        Ok(())
    }

    let bytes = descriptor.as_bytes();
    if bytes.first() != Some(&b'(') {
        return Err(());
    }
    let mut cursor = 1usize;
    let mut count = 0usize;
    while bytes.get(cursor) != Some(&b')') {
        if cursor >= bytes.len() || count >= 255 {
            return Err(());
        }
        parse_type(bytes, &mut cursor, false)?;
        count += 1;
    }
    cursor += 1;
    parse_type(bytes, &mut cursor, true)?;
    (cursor == bytes.len()).then_some(count).ok_or(())
}

fn has_modifier(modifiers: &Value, expected: &str) -> bool {
    modifiers
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|modifier| modifier == expected)
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
    constructed_types: BTreeSet<String>,
    field_receiver_calls: BTreeSet<FieldReceiverCall>,
    qualified_fields: BTreeSet<QualifiedFieldUse>,
    unsupported_qualified_references: BTreeSet<String>,
    field_receiver_lambda_ambiguous: bool,
    fields: BTreeSet<FieldUse>,
    nested_executable_context: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct QualifiedFieldUse {
    type_name: String,
    field_name: String,
    shadowed: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FieldUse {
    name: String,
    explicit_receiver: bool,
    shadowed: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FieldReceiverCall {
    field_name: String,
    method_name: String,
    argument_count: Option<usize>,
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
            constructed_types: BTreeSet::new(),
            field_receiver_calls: BTreeSet::new(),
            qualified_fields: BTreeSet::new(),
            unsupported_qualified_references: BTreeSet::new(),
            field_receiver_lambda_ambiguous: false,
            fields: BTreeSet::new(),
            nested_executable_context: true,
        };
    }
    let mut calls = BTreeSet::new();
    let mut method_references = BTreeSet::new();
    let mut constructed_types = BTreeSet::new();
    let mut qualified_fields = BTreeSet::new();
    let mut unsupported_qualified_references = BTreeSet::new();
    let mut referenced_fields = BTreeMap::<String, (bool, bool)>::new();
    let parameter_range = method_parameter_range(&tokens, body_range.0, method_name);
    let shadowed: BTreeSet<_> = tokens
        .iter()
        .enumerate()
        .filter(|(index, token)| {
            let in_body = token.start > body_range.0 && token.start < body_range.1;
            let in_parameters = parameter_range
                .is_some_and(|(start, end)| token.start > start && token.start < end);
            (in_body || in_parameters)
                && (looks_like_variable_declaration(&tokens, *index, token)
                    || looks_like_instanceof_binding(&tokens, *index)
                    || looks_like_multi_declarator_binding(&tokens, *index))
        })
        .map(|(_, token)| token.text.clone())
        .collect();
    let mut field_receiver_calls = field_receiver_calls(source, &tokens, body_range, &shadowed);
    let field_receiver_lambda_ambiguous =
        has_lambda_context(&tokens, body_range) && !field_receiver_calls.is_empty();
    if field_receiver_lambda_ambiguous {
        field_receiver_calls.clear();
    }

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
        if previous_dot && index >= 3 && tokens[index - 3].text == "." {
            unsupported_qualified_references.insert(format!(
                "{}.{}",
                tokens[index - 2].text,
                token.text
            ));
        }
        if token.text == "new"
            && let Some(type_token) = tokens.get(index + 1)
            && is_identifier(&type_token.text)
        {
            let mut qualified_name = type_token.text.clone();
            let mut cursor = index + 2;
            while tokens.get(cursor).is_some_and(|part| part.text == ".")
                && tokens
                    .get(cursor + 1)
                    .is_some_and(|part| is_identifier(&part.text))
            {
                qualified_name.push('.');
                qualified_name.push_str(&tokens[cursor + 1].text);
                cursor += 2;
            }
            constructed_types.insert(qualified_name);
        }
        let current_this_method_reference = index >= 3
            && tokens[index - 1].text == ":"
            && tokens[index - 2].text == ":"
            && tokens[index - 3].text == "this"
            && (index < 4 || tokens[index - 4].text != ".");
        if current_this_method_reference {
            method_references.insert(token.text.clone());
            continue;
        }
        if following_call && index > 0 && tokens[index - 1].text == "new" {
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
        if previous_dot
            && !explicit_receiver
            && !owner_qualified
            && index >= 2
            && is_identifier(&tokens[index - 2].text)
            && (index < 3 || tokens[index - 3].text != ".")
            && !following_call
        {
            qualified_fields.insert(QualifiedFieldUse {
                type_name: tokens[index - 2].text.clone(),
                field_name: token.text.clone(),
                shadowed: shadowed.contains(&tokens[index - 2].text),
            });
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
        constructed_types,
        field_receiver_calls,
        qualified_fields,
        unsupported_qualified_references,
        field_receiver_lambda_ambiguous,
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

fn field_receiver_calls(
    source: &str,
    tokens: &[Token],
    body_range: (usize, usize),
    shadowed: &BTreeSet<String>,
) -> BTreeSet<FieldReceiverCall> {
    let mut calls = BTreeSet::new();
    for (index, method) in tokens.iter().enumerate() {
        if method.start <= body_range.0
            || method.start >= body_range.1
            || !is_identifier(&method.text)
            || !tokens.get(index + 1).is_some_and(|next| next.text == "(")
            || index < 2
            || tokens[index - 1].text != "."
        {
            continue;
        }
        let receiver_index = index - 2;
        let receiver = &tokens[receiver_index];
        if !is_identifier(&receiver.text) || matches!(receiver.text.as_str(), "this" | "super") {
            continue;
        }
        let explicit_receiver = receiver_index >= 2
            && tokens[receiver_index - 1].text == "."
            && tokens[receiver_index - 2].text == "this"
            && (receiver_index < 3 || tokens[receiver_index - 3].text != ".");
        if receiver_index > 0 && tokens[receiver_index - 1].text == "." && !explicit_receiver {
            // Qualified and chained receivers require a broader resolver.
            continue;
        }
        calls.insert(FieldReceiverCall {
            field_name: receiver.text.clone(),
            method_name: method.text.clone(),
            argument_count: bounded_call_argument_count(source, tokens, index),
            explicit_receiver,
            shadowed: shadowed.contains(&receiver.text),
        });
    }
    calls
}

fn has_lambda_context(tokens: &[Token], body_range: (usize, usize)) -> bool {
    tokens.windows(2).any(|pair| {
        pair[0].start > body_range.0
            && pair[1].start < body_range.1
            && pair[0].text == "-"
            && pair[1].text == ">"
    })
}

fn bounded_call_argument_count(
    source: &str,
    tokens: &[Token],
    method_index: usize,
) -> Option<usize> {
    let open_index = method_index + 1;
    if tokens.get(open_index)?.text != "(" {
        return None;
    }
    let mut depth = 0usize;
    let close_index = (open_index..tokens.len()).find_map(|index| {
        match tokens[index].text.as_str() {
            "(" => depth += 1,
            ")" => {
                depth = depth.checked_sub(1)?;
                return (depth == 0).then_some(index);
            }
            _ => {}
        }
        None
    })?;
    let open_end = tokens[open_index].end;
    let close_start = tokens[close_index].start;
    if open_end > close_start || close_start > source.len() {
        return None;
    }
    let (code, comments) = source_steps::lexical_masks(source);
    let bytes = source.as_bytes();
    let mut parens = 1usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    let mut commas = 0usize;
    let mut segment_has_value = false;
    for index in open_end..close_start {
        if comments.get(index).copied().unwrap_or(false) {
            continue;
        }
        if !code.get(index).copied().unwrap_or(false) {
            // Literal text is argument content even though code_tokens masks it.
            segment_has_value = true;
            continue;
        }
        let byte = bytes[index];
        match byte {
            b'(' => parens += 1,
            b')' => parens = parens.checked_sub(1)?,
            b'[' => brackets += 1,
            b']' => brackets = brackets.checked_sub(1)?,
            b'{' => braces += 1,
            b'}' => braces = braces.checked_sub(1)?,
            b',' if parens == 1 && brackets == 0 && braces == 0 => {
                if !segment_has_value {
                    return None;
                }
                commas += 1;
                segment_has_value = false;
            }
            b'<' | b'>' if parens == 1 && brackets == 0 && braces == 0 => return None,
            _ if !byte.is_ascii_whitespace() => segment_has_value = true,
            _ => {}
        }
    }
    if !segment_has_value {
        return (commas == 0).then_some(0);
    }
    Some(commas + 1)
}

fn looks_like_instanceof_binding(tokens: &[Token], index: usize) -> bool {
    if index < 2
        || !is_identifier(&tokens[index].text)
        || !(is_identifier(&tokens[index - 1].text)
            || matches!(tokens[index - 1].text.as_str(), "]" | ">" | "?"))
    {
        return false;
    }
    for cursor in (0..index - 1).rev() {
        let text = tokens[cursor].text.as_str();
        if text == "instanceof" {
            return tokens[cursor + 1..index].iter().all(|token| {
                is_identifier(&token.text)
                    || matches!(
                        token.text.as_str(),
                        "." | "[" | "]" | "<" | ">" | "?" | "," | "&"
                    )
            });
        }
        if !is_identifier(text) && !matches!(text, "." | "[" | "]" | "<" | ">" | "?" | "," | "&") {
            return false;
        }
    }
    false
}

fn looks_like_multi_declarator_binding(tokens: &[Token], index: usize) -> bool {
    if !is_identifier(&tokens[index].text)
        || index == 0
        || tokens[index - 1].text != ","
        || !is_variable_declarator_end(tokens, index)
    {
        return false;
    }
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    let mut start = 0usize;
    for cursor in (0..index).rev() {
        match tokens[cursor].text.as_str() {
            ")" => parentheses += 1,
            "(" if parentheses > 0 => parentheses -= 1,
            "(" if brackets == 0 && braces == 0 => {
                start = cursor + 1;
                break;
            }
            "]" => brackets += 1,
            "[" if brackets > 0 => brackets -= 1,
            "}" => braces += 1,
            "{" if braces > 0 => braces -= 1,
            "{" | ";" if parentheses == 0 && brackets == 0 && braces == 0 => {
                start = cursor + 1;
                break;
            }
            _ => {}
        }
    }
    (start..index - 1)
        .any(|candidate| looks_like_variable_declaration(tokens, candidate, &tokens[candidate]))
}

fn is_variable_declarator_end(tokens: &[Token], index: usize) -> bool {
    let mut following = index + 1;
    while tokens.get(following).is_some_and(|token| token.text == "[") {
        if !tokens
            .get(following + 1)
            .is_some_and(|token| token.text == "]")
        {
            return false;
        }
        following += 2;
    }
    tokens
        .get(following)
        .is_some_and(|token| matches!(token.text.as_str(), "=" | ";" | ","))
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
    const BODYLESS_CANDIDATE_COUNT: usize = 11;

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
        for index in 0..BODYLESS_CANDIDATE_COUNT {
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
                root_declaration: None,
                question: None,
                authoring_contract: None,
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

    fn insert_evidence_observation(work: &mut Work, observation: Observation) {
        let id = observation.id.clone();
        work.checked
            .services
            .get_mut(SERVICE)
            .unwrap()
            .observations
            .insert(id.clone(), observation);
        work.influence.insert(id.clone(), "test-influence".into());
        work.handles.insert(
            format!("dependency:{id}"),
            Handle {
                kind: "DEPENDENCY".into(),
                id,
            },
        );
    }

    fn exact_call_relation(
        id: String,
        from: &str,
        target: &str,
        relation_kind: &str,
        source_digest: &str,
    ) -> Observation {
        Observation {
            id,
            kind: "CALL_RELATION".into(),
            service: SERVICE.into(),
            symbol: from.into(),
            normalized: json!({
                "scope":SCOPE,
                "sourceIdentity":from,
                "targetIdentity":target,
                "relationKind":relation_kind,
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
        }
    }

    fn reachable_context_exceeds_all_selection_cutoffs() -> Work {
        const DTO: &str = "class:orders.Dto";
        const PROVIDER_TARGET_COUNT: usize = 257;
        const SOURCE_CANDIDATE_COUNT: usize = 18;
        const OWNER_FIELD_COUNT: usize = 10;
        const DTO_FIELD_COUNT: usize = 65;
        const CHAIN_LENGTH: usize = 6;
        const RETAINED_PROVIDER_BODY_COUNT: usize = 13;

        let mut work = bodyless_candidates_do_not_spend_body_slots_before_a_real_getter();
        let root_identity = "method:class:orders.Service#handle()Lorders/Dto;";
        {
            let evidence = work.checked.services.get_mut(SERVICE).unwrap();
            evidence
                .observations
                .retain(|id, _| id == "root-declaration");
            let root = evidence.observations.get_mut("root-declaration").unwrap();
            root.symbol = root_identity.into();
            root.normalized["symbolIdentity"] = json!(root_identity);
            root.normalized["jvmDescriptor"] = json!("()Lorders/Dto;");
            evidence.entrypoints[0].symbol = root_identity.into();
        }
        work.influence.retain(|id, _| id == "root-declaration");
        work.handles
            .retain(|_, handle| handle.kind != "DEPENDENCY" || handle.id == "root-declaration");

        let provider_names: Vec<_> = (0..PROVIDER_TARGET_COUNT)
            .map(|index| format!("provider{index:03}"))
            .collect();
        let candidate_names: Vec<_> = (0..SOURCE_CANDIDATE_COUNT)
            .map(|index| format!("candidate{index:02}"))
            .collect();
        let chain_names: Vec<_> = (0..CHAIN_LENGTH)
            .map(|index| format!("chain{index:02}"))
            .collect();
        let method_names = provider_names
            .iter()
            .take(RETAINED_PROVIDER_BODY_COUNT)
            .chain(&candidate_names)
            .chain(&chain_names)
            .cloned()
            .collect::<Vec<_>>();
        let method_bodies = method_names
            .iter()
            .map(|name| format!("String {name}() {{ return \"\"; }}"))
            .collect::<Vec<_>>()
            .join(" ");
        let candidate_calls = candidate_names
            .iter()
            .map(|name| format!("this.{name}();"))
            .collect::<Vec<_>>()
            .join(" ");
        let owner_field_reads = (0..OWNER_FIELD_COUNT)
            .map(|index| format!("this.ownerField{index:02};"))
            .collect::<Vec<_>>()
            .join(" ");
        let source_text = format!(
            "class Service {{ Dto handle() {{ {candidate_calls} {owner_field_reads} return null; }} {method_bodies} }} class Dto {{}} /*{}*/",
            "x".repeat(50 * 1024)
        );
        let source_digest = crate::canonical::hash_bytes(source_text.as_bytes());
        {
            let source = work
                .checked
                .services
                .get_mut(SERVICE)
                .unwrap()
                .sources
                .get_mut(SOURCE_ID)
                .unwrap();
            source.text = source_text;
            source.text_digest = source_digest.clone();
            source.end_line = 1;
        }

        for (index, name) in provider_names.iter().enumerate() {
            let identity = format!("method:{OWNER}#{name}()Ljava/lang/String;");
            insert_evidence_observation(
                &mut work,
                observation(
                    format!("declaration-{name}"),
                    identity.clone(),
                    method_declaration(&identity, name),
                    if index < RETAINED_PROVIDER_BODY_COUNT {
                        vec![SOURCE_ID.into()]
                    } else {
                        Vec::new()
                    },
                ),
            );
        }
        for name in candidate_names.iter().chain(&chain_names) {
            let identity = format!("method:{OWNER}#{name}()Ljava/lang/String;");
            insert_evidence_observation(
                &mut work,
                observation(
                    format!("declaration-{name}"),
                    identity.clone(),
                    method_declaration(&identity, name),
                    vec![SOURCE_ID.into()],
                ),
            );
        }

        insert_evidence_observation(
            &mut work,
            observation(
                "dto-declaration",
                DTO,
                json!({
                    "schema":JAVA_COMPILER_FACT_SCHEMA,
                    "declarationKind":"CLASS",
                    "symbolIdentity":DTO,
                    "ownerIdentity":"class:orders",
                    "name":"Dto",
                    "scope":SCOPE
                }),
                vec![SOURCE_ID.into()],
            ),
        );
        for index in 0..DTO_FIELD_COUNT {
            let name = format!("dtoField{index:02}");
            let identity = format!("field:{DTO}#{name}:Ljava/lang/String;");
            insert_evidence_observation(
                &mut work,
                observation(
                    format!("dto-field-{index:02}"),
                    identity.clone(),
                    json!({
                        "schema":JAVA_COMPILER_FACT_SCHEMA,
                        "declarationKind":"FIELD",
                        "symbolIdentity":identity,
                        "ownerIdentity":DTO,
                        "name":name,
                        "scope":SCOPE,
                        "jvmDescriptor":"Ljava/lang/String;",
                        "modifiers":[],
                        "annotations":[],
                        "sourceTokens":[name]
                    }),
                    vec![SOURCE_ID.into()],
                ),
            );
        }
        for index in 0..OWNER_FIELD_COUNT {
            let name = format!("ownerField{index:02}");
            let identity = format!("field:{OWNER}#{name}:Ljava/lang/String;");
            let modifiers = if index == OWNER_FIELD_COUNT - 1 {
                json!(["STATIC", "FINAL"])
            } else {
                json!([])
            };
            insert_evidence_observation(
                &mut work,
                observation(
                    format!("owner-field-{index:02}"),
                    identity.clone(),
                    json!({
                        "schema":JAVA_COMPILER_FACT_SCHEMA,
                        "declarationKind":"FIELD",
                        "symbolIdentity":identity,
                        "ownerIdentity":OWNER,
                        "name":name,
                        "scope":SCOPE,
                        "jvmDescriptor":"Ljava/lang/String;",
                        "modifiers":modifiers,
                        "annotations":[],
                        "sourceTokens":[name]
                    }),
                    vec![SOURCE_ID.into()],
                ),
            );
        }

        for (index, name) in provider_names.iter().enumerate() {
            let target = format!("method:{OWNER}#{name}()Ljava/lang/String;");
            insert_evidence_observation(
                &mut work,
                exact_call_relation(
                    format!("root-provider-relation-{index:03}"),
                    root_identity,
                    &target,
                    "CALLS",
                    &source_digest,
                ),
            );
        }
        let first_provider = format!("method:{OWNER}#{}()Ljava/lang/String;", provider_names[0]);
        insert_evidence_observation(
            &mut work,
            exact_call_relation(
                "root-reference-relation".into(),
                root_identity,
                &first_provider,
                "REFERENCES",
                &source_digest,
            ),
        );

        let first_chain = format!("method:{OWNER}#{}()Ljava/lang/String;", chain_names[0]);
        insert_evidence_observation(
            &mut work,
            exact_call_relation(
                "root-chain-relation".into(),
                root_identity,
                &first_chain,
                "CALLS",
                &source_digest,
            ),
        );
        for index in 0..CHAIN_LENGTH {
            let from = format!("method:{OWNER}#{}()Ljava/lang/String;", chain_names[index]);
            let target_index = (index + 1) % CHAIN_LENGTH;
            let target = format!(
                "method:{OWNER}#{}()Ljava/lang/String;",
                chain_names[target_index]
            );
            insert_evidence_observation(
                &mut work,
                exact_call_relation(
                    format!("chain-relation-{index:02}"),
                    &from,
                    &target,
                    "CALLS",
                    &source_digest,
                ),
            );
        }
        work
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

    fn field_receiver_work(
        parameters: &str,
        body: &str,
        target_source: &str,
        fields: &[(&str, &str, &str)],
        methods: &[(&str, &str, &str, &str)],
    ) -> Work {
        let mut work = bodyless_candidates_do_not_spend_body_slots_before_a_real_getter();
        let service = work.checked.services.get_mut(SERVICE).unwrap();
        service
            .observations
            .retain(|id, _| id == "root-declaration");
        let source_text =
            format!("class Service {{ void handle({parameters}) {{ {body} }} }} {target_source}");
        let source = service.sources.get_mut(SOURCE_ID).unwrap();
        source.text = source_text.clone();
        source.text_digest = crate::canonical::hash_bytes(source_text.as_bytes());
        source.end_line = source_text.lines().count().max(1) as u64;
        work.influence.retain(|id, _| id == "root-declaration");
        work.handles
            .retain(|_, handle| handle.kind != "DEPENDENCY" || handle.id == "root-declaration");

        for (name, descriptor, field_scope) in fields {
            let id = format!("field-{name}");
            let identity = format!("field:{OWNER}#{name}:{descriptor}");
            let field = observation(
                id.clone(),
                identity.clone(),
                json!({
                    "schema":JAVA_COMPILER_FACT_SCHEMA,
                    "declarationKind":"FIELD",
                    "symbolIdentity":identity,
                    "ownerIdentity":OWNER,
                    "name":name,
                    "scope":field_scope,
                    "jvmDescriptor":descriptor,
                    "modifiers":[],
                    "annotations":[],
                    "sourceTokens":["field", name]
                }),
                vec![SOURCE_ID.into()],
            );
            service.observations.insert(id.clone(), field);
            work.influence.insert(id.clone(), "test-influence".into());
            work.handles.insert(
                format!("dependency:{id}"),
                Handle {
                    kind: "DEPENDENCY".into(),
                    id,
                },
            );
        }
        for (owner, name, descriptor, method_scope) in methods {
            let id = format!("method-{name}-{}", service.observations.len());
            let identity = format!("method:{owner}#{name}{descriptor}");
            let method = observation(
                id.clone(),
                identity.clone(),
                json!({
                    "schema":JAVA_COMPILER_FACT_SCHEMA,
                    "declarationKind":"METHOD",
                    "symbolIdentity":identity,
                    "ownerIdentity":owner,
                    "name":name,
                    "scope":method_scope,
                    "jvmDescriptor":descriptor
                }),
                vec![SOURCE_ID.into()],
            );
            service.observations.insert(id.clone(), method);
            work.influence.insert(id.clone(), "test-influence".into());
            work.handles.insert(
                format!("dependency:{id}"),
                Handle {
                    kind: "DEPENDENCY".into(),
                    id,
                },
            );
        }
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
            BODYLESS_CANDIDATE_COUNT + 1
        );
        let gaps = packet["record"]["gaps"].as_array().unwrap();
        assert!(gaps.iter().any(|gap| {
            gap["code"] == "METHOD_BODY_PARSE_UNAVAILABLE"
                && gap["count"] == BODYLESS_CANDIDATE_COUNT
        }));
        assert_eq!(graph["maxObservedTraversalDepth"], 1);
        assert!(graph.get("callableLimit").is_none());
        assert!(
            packet["record"]["selection"]
                .get("sourceByteLimit")
                .is_none()
        );
    }

    #[test]
    fn reachable_context_keeps_evidence_past_former_selection_cutoffs() {
        let work = reachable_context_exceeds_all_selection_cutoffs();
        let rows = profile_rows(&work).unwrap();
        let context = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        let record = &context["record"];
        let graph = &record["callGraph"];
        let nodes = graph["nodes"].as_array().unwrap();
        let candidates = graph["sourceReferenceCandidates"].as_array().unwrap();
        let selection = &record["selection"];

        assert_eq!(nodes.len(), 1 + 13 + 18 + 6);
        assert_eq!(
            nodes
                .iter()
                .filter(|node| node["symbolIdentity"]
                    == "method:class:orders.Service#provider000()Ljava/lang/String;")
                .count(),
            1
        );
        let unique_nodes: BTreeSet<_> = nodes
            .iter()
            .filter_map(|node| node["symbolIdentity"].as_str())
            .collect();
        assert_eq!(unique_nodes.len(), nodes.len());
        assert!(
            graph["providerEdgeFactReferences"]
                .as_array()
                .unwrap()
                .len()
                > 256
        );
        assert_eq!(candidates.len(), 18);
        assert_eq!(selection["dtoFieldCount"], 65);
        assert_eq!(selection["referencedOwnerFieldCount"], 10);
        assert!(
            record["gaps"].as_array().unwrap().iter().any(|gap| {
                gap["code"] == "METHOD_BODY_SOURCE_UNAVAILABLE" && gap["count"] == 244
            })
        );
        assert_eq!(graph["maxObservedTraversalDepth"], 6);
        assert!(selection["uniqueBodySourceBytes"].as_u64().unwrap() > 48 * 1024);
        assert!(selection["uniqueSourceBytes"].as_u64().unwrap() > 48 * 1024);
        assert_eq!(selection["callableCount"], nodes.len());
        assert_eq!(selection["sourceReferenceCandidateCount"], candidates.len());
        assert!(graph.get("depthLimit").is_none());
        assert!(graph.get("callableLimit").is_none());
        assert!(graph.get("providerEdgeReferenceLimit").is_none());
        assert!(graph.get("sourceReferenceCandidateLimit").is_none());
        assert!(selection.get("sourceByteLimit").is_none());
        assert!(selection.get("dtoFieldLimit").is_none());
        assert!(selection.get("referencedOwnerFieldLimit").is_none());
        assert!(
            !record["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| matches!(
                    gap["code"].as_str(),
                    Some(
                        "CALLABLE_BODY_LIMIT"
                            | "UNIQUE_SOURCE_BYTE_LIMIT"
                            | "CALL_GRAPH_DEPTH_LIMIT"
                            | "CALL_GRAPH_EDGE_LIMIT"
                            | "SOURCE_REFERENCE_CANDIDATE_LIMIT"
                            | "DTO_FIELD_LIMIT"
                            | "REFERENCED_OWNER_FIELD_LIMIT"
                    )
                ))
        );

        let (packet, audit) = super::super::operation_packet::build(&work).unwrap();
        assert_eq!(packet["callMap"]["order"], "NOT_EXECUTION_ORDER");
        assert_eq!(
            packet["callMap"]["nodes"].as_array().unwrap().len(),
            nodes.len()
        );
        assert_eq!(packet["methodSources"].as_array().unwrap().len(), 1);
        assert!(packet["methodSources"][0]["text"].as_str().unwrap().len() > 48 * 1024);
        assert_eq!(
            packet["methodBodies"].as_array().unwrap().len(),
            nodes.len()
        );
        assert_eq!(packet["types"][0]["fields"].as_array().unwrap().len(), 65);
        assert_eq!(packet["constants"].as_array().unwrap().len(), 1);
        let packet_edges = packet["callMap"]["edges"].as_array().unwrap();
        assert!(packet_edges.iter().any(|edge| {
            edge["kind"] == "CALLS" && edge["authority"] == "COMPILER_EXACT_CALL_RELATION"
        }));
        assert!(packet_edges.iter().any(|edge| {
            edge["kind"] == "REFERENCES" && edge["authority"] == "COMPILER_EXACT_REFERENCE_RELATION"
        }));
        assert_eq!(
            packet_edges
                .iter()
                .filter(|edge| edge["authority"] == "SOURCE_REFERENCE_CANDIDATE")
                .count(),
            18
        );
        let chain_zero = packet["callMap"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["identity"].as_str().unwrap().contains("#chain00("))
            .unwrap()["id"]
            .clone();
        let chain_last = packet["callMap"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["identity"].as_str().unwrap().contains("#chain05("))
            .unwrap()["id"]
            .clone();
        assert!(
            packet_edges
                .iter()
                .any(|edge| { edge["from"] == chain_last && edge["toNode"] == chain_zero })
        );
        assert_eq!(
            audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["kind"] == "SOURCE")
                .count(),
            1
        );
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
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.get("sourceReference").is_none())
        );
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
    fn field_receiver_calls_select_unique_same_scope_methods_by_declared_field_type() {
        let work = field_receiver_work(
            "",
            "worker.check(','); worker.noArgs(/* empty, after comment */); this.agent.combine(inner(new int[]{1,2}), \"one,two\");",
            "class Worker { void check(char value) {} void noArgs() {} } class Agent { void combine(Value value, String text) {} } class Value {}",
            &[
                ("worker", "Lorders/Worker;", SCOPE),
                ("agent", "Lorders/Agent;", SCOPE),
            ],
            &[
                ("class:orders.Worker", "check", "(C)V", SCOPE),
                ("class:orders.Worker", "noArgs", "()V", SCOPE),
                (
                    "class:orders.Agent",
                    "combine",
                    "(Lorders/Value;Ljava/lang/String;)V",
                    SCOPE,
                ),
            ],
        );
        let rows = profile_rows(&work).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        let graph = &packet["record"]["callGraph"];
        let candidates: Vec<_> = graph["sourceReferenceCandidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|candidate| candidate["kind"] == "FIELD_RECEIVER_CALL")
            .collect();
        assert_eq!(candidates.len(), 3);
        assert!(candidates.iter().all(|candidate| {
            candidate["authority"] == "SOURCE_REFERENCE_CANDIDATE"
                && candidate["toNode"].is_string()
                && candidate["receiverFieldReference"].is_string()
        }));
        let nodes = graph["nodes"].as_array().unwrap();
        assert!(
            nodes
                .iter()
                .any(|node| { node["symbolIdentity"] == "method:class:orders.Worker#check(C)V" })
        );
        assert!(nodes.iter().any(|node| {
            node["symbolIdentity"]
                == "method:class:orders.Agent#combine(Lorders/Value;Ljava/lang/String;)V"
        }));
        assert!(
            nodes
                .iter()
                .any(|node| node["symbolIdentity"] == "method:class:orders.Worker#noArgs()V")
        );
    }

    #[test]
    fn explicit_this_field_calls_survive_shadowing_but_parameter_and_local_receivers_do_not() {
        let explicit = field_receiver_work(
            "Worker worker",
            "this.worker.run();",
            "class Worker { void run() {} }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "run", "()V", SCOPE)],
        );
        let explicit_rows = profile_rows(&explicit).unwrap();
        let explicit_packet = explicit_rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert_eq!(
            explicit_packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|candidate| candidate["kind"] == "FIELD_RECEIVER_CALL")
                .count(),
            1
        );

        for (parameters, body) in [
            ("Worker worker", "worker.run();"),
            ("", "Worker worker = null; worker.run();"),
        ] {
            let shadowed = field_receiver_work(
                parameters,
                body,
                "class Worker { void run() {} }",
                &[("worker", "Lorders/Worker;", SCOPE)],
                &[("class:orders.Worker", "run", "()V", SCOPE)],
            );
            let rows = profile_rows(&shadowed).unwrap();
            let packet = rows
                .iter()
                .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
                .unwrap();
            assert!(
                packet["record"]["callGraph"]["sourceReferenceCandidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
            );
            assert!(
                packet["record"]["gaps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|gap| gap["code"] == "SOURCE_FIELD_SHADOWING_AMBIGUOUS")
            );
        }
    }

    #[test]
    fn pattern_bindings_and_multi_declarator_locals_shadow_receiver_fields() {
        let pattern = field_receiver_work(
            "",
            "if (candidate instanceof Worker worker && worker.ready()) { }",
            "class Worker { boolean ready() { return true; } }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "ready", "()Z", SCOPE)],
        );
        let multi_declarators = [
            "Worker first = null, worker = null; worker.run();",
            "Worker first, worker; worker = obtainWorker(); worker.run();",
            "Worker first, worker, third; worker.run();",
            "Worker first, worker[]; worker.run();",
            "Worker first, worker[][]; worker.run();",
        ]
        .map(|body| {
            field_receiver_work(
                "",
                body,
                "class Worker { void run() {} }",
                &[("worker", "Lorders/Worker;", SCOPE)],
                &[("class:orders.Worker", "run", "()V", SCOPE)],
            )
        });
        for shadowed in [pattern].into_iter().chain(multi_declarators.into_iter()) {
            let rows = profile_rows(&shadowed).unwrap();
            let packet = rows
                .iter()
                .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
                .unwrap();
            assert!(
                packet["record"]["callGraph"]["sourceReferenceCandidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
            );
            assert!(
                packet["record"]["gaps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|gap| gap["code"] == "SOURCE_FIELD_SHADOWING_AMBIGUOUS")
            );
        }
    }

    #[test]
    fn qualified_chained_and_masked_receiver_text_is_not_a_current_owner_field_call() {
        let work = field_receiver_work(
            "",
            concat!(
                "String text = \"this.worker.run()\"; ",
                "/* this.worker.run(); */ other.worker.run(); ",
                "factory().run(); Other.this.worker.run(); ",
                "this.worker.lookup().run();"
            ),
            "class Worker { void run() {} }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "run", "()V", SCOPE)],
        );
        let rows = profile_rows(&work).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
    }

    #[test]
    fn field_receiver_overloads_scopes_descriptors_and_bodies_fail_closed() {
        let ambiguous = field_receiver_work(
            "",
            "worker.run();",
            "class Worker { void run() {} void run(int value) {} }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[
                ("class:orders.Worker", "run", "()V", SCOPE),
                ("class:orders.Worker", "run", "(I)V", SCOPE),
            ],
        );
        let rows = profile_rows(&ambiguous).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
        assert!(
            packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_FIELD_RECEIVER_AMBIGUOUS_OVERLOAD")
        );

        let wrong_method_scope = field_receiver_work(
            "",
            "worker.run();",
            "class Worker { void run() {} }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "run", "()V", "compile:other")],
        );
        let wrong_scope_rows = profile_rows(&wrong_method_scope).unwrap();
        let wrong_scope_packet = wrong_scope_rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            wrong_scope_packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_FIELD_RECEIVER_TARGET_SCOPE_UNAVAILABLE")
        );

        let wrong_field_scope = field_receiver_work(
            "",
            "worker.run();",
            "class Worker { void run() {} }",
            &[("worker", "Lorders/Worker;", "compile:other")],
            &[("class:orders.Worker", "run", "()V", SCOPE)],
        );
        let wrong_field_rows = profile_rows(&wrong_field_scope).unwrap();
        let wrong_field_packet = wrong_field_rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            wrong_field_packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
        assert!(
            wrong_field_packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_FIELD_SCOPE_UNAVAILABLE")
        );

        for descriptor in ["I", "L/orders/Worker;", "Lorders/Worker"] {
            let unsupported = field_receiver_work(
                "",
                "worker.run();",
                "class Worker { void run() {} }",
                &[("worker", descriptor, SCOPE)],
                &[("class:orders.Worker", "run", "()V", SCOPE)],
            );
            let rows = profile_rows(&unsupported).unwrap();
            let packet = rows
                .iter()
                .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
                .unwrap();
            assert!(
                packet["record"]["callGraph"]["sourceReferenceCandidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
            );
            assert!(
                packet["record"]["gaps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|gap| gap["code"] == "SOURCE_FIELD_RECEIVER_TYPE_UNSUPPORTED")
            );
        }

        let missing_body = field_receiver_work(
            "",
            "worker.run();",
            "class Worker {}",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "run", "()V", SCOPE)],
        );
        let rows = profile_rows(&missing_body).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
        assert!(
            packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_FIELD_RECEIVER_BODY_UNAVAILABLE")
        );

        let arity_mismatch = field_receiver_work(
            "",
            "worker.ready(',');",
            "class Worker { boolean ready(int first, int second) { return true; } }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "ready", "(II)Z", SCOPE)],
        );
        let rows = profile_rows(&arity_mismatch).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
        assert!(
            packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_FIELD_RECEIVER_ARGUMENT_COUNT_MISMATCH")
        );

        let unsupported_arguments = field_receiver_work(
            "",
            "worker.ready(new Box<Left,Right>());",
            "class Worker { boolean ready(Object value) { return true; } } class Box<A,B> {} class Left {} class Right {}",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[(
                "class:orders.Worker",
                "ready",
                "(Ljava/lang/Object;)Z",
                SCOPE,
            )],
        );
        let rows = profile_rows(&unsupported_arguments).unwrap();
        let packet = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
        assert!(
            packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_FIELD_RECEIVER_ARGUMENTS_UNSUPPORTED")
        );
    }

    #[test]
    fn lambda_and_nested_executable_receiver_candidates_are_gaps() {
        let lambda = field_receiver_work(
            "",
            "Consumer<Worker> callback = worker -> worker.run();",
            "class Worker { void run() {} }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "run", "()V", SCOPE)],
        );
        let lambda_rows = profile_rows(&lambda).unwrap();
        let lambda_packet = lambda_rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            lambda_packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
        assert!(
            lambda_packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_FIELD_RECEIVER_LAMBDA_SCOPE_AMBIGUOUS")
        );

        let nested = field_receiver_work(
            "",
            "class Local { void callback() { worker.run(); } }",
            "class Worker { void run() {} }",
            &[("worker", "Lorders/Worker;", SCOPE)],
            &[("class:orders.Worker", "run", "()V", SCOPE)],
        );
        let nested_rows = profile_rows(&nested).unwrap();
        let nested_packet = nested_rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        assert!(
            nested_packet["record"]["callGraph"]["sourceReferenceCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["kind"] != "FIELD_RECEIVER_CALL")
        );
        assert!(
            nested_packet["record"]["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_NESTED_EXECUTABLE_CONTEXT_AMBIGUOUS")
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
