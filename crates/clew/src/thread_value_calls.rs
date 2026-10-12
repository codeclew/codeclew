//! Bounded direct-call composition over one explicitly bound equation program.
//! The compiler bridge authenticates exact calls; this module does not resolve them.
use crate::canonical;
use clew_facts::values::{ModelBinding, ValueModel, ValueNode, ValueOperation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const POLICY: &str = "portable-exact-value-calls/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactValueCall {
    pub definition: String,
    pub target: String,
    /// Positional arguments after the bridge's exact parameter mapping.
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FunctionValues {
    pub owner: String,
    pub parameters: Vec<String>,
    pub model: ValueModel,
    pub calls: Vec<ExactValueCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueOrigin {
    pub definition: String,
    pub owner: String,
    pub local_definition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpandedValues {
    pub model: ValueModel,
    pub origins: Vec<ValueOrigin>,
    pub summary_keys: Vec<String>,
    pub summary_cache_hits: usize,
    pub context_count: usize,
    pub recursive_components: Vec<Vec<String>>,
}

fn components(registry: &BTreeMap<&str, &FunctionValues>) -> Vec<Vec<String>> {
    fn visit<'a>(
        owner: &'a str,
        graph: &BTreeMap<&'a str, Vec<&'a str>>,
        seen: &mut BTreeSet<&'a str>,
        order: &mut Vec<&'a str>,
    ) {
        if !seen.insert(owner) {
            return;
        }
        for next in &graph[owner] {
            visit(next, graph, seen, order);
        }
        order.push(owner);
    }
    let graph: BTreeMap<_, _> = registry
        .iter()
        .map(|(owner, function)| {
            (
                *owner,
                function
                    .calls
                    .iter()
                    .map(|c| c.target.as_str())
                    .filter(|target| registry.contains_key(target))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let mut reverse: BTreeMap<_, Vec<_>> =
        registry.keys().map(|owner| (*owner, Vec::new())).collect();
    for (owner, targets) in &graph {
        for target in targets {
            reverse.get_mut(target).unwrap().push(*owner);
        }
    }
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for owner in registry.keys() {
        visit(owner, &graph, &mut seen, &mut order);
    }
    seen.clear();
    let mut result = Vec::new();
    for owner in order.into_iter().rev() {
        if seen.contains(owner) {
            continue;
        }
        let mut group = Vec::new();
        visit(owner, &reverse, &mut seen, &mut group);
        group.sort();
        if group.len() > 1 || graph[group[0]].contains(&group[0]) {
            result.push(group.into_iter().map(str::to_owned).collect());
        }
    }
    result.sort();
    result
}

fn scoped(context: usize, id: &str) -> String {
    if context == 0 {
        id.into()
    } else {
        format!("c{context}/{id}")
    }
}
fn remap(op: &ValueOperation, context: usize) -> ValueOperation {
    match op {
        ValueOperation::Copy { value } => ValueOperation::Copy {
            value: scoped(context, value),
        },
        ValueOperation::Coalesce { left, right } => ValueOperation::Coalesce {
            left: scoped(context, left),
            right: scoped(context, right),
        },
        ValueOperation::Join { values } => ValueOperation::Join {
            values: values.iter().map(|v| scoped(context, v)).collect(),
        },
        _ => op.clone(),
    }
}

fn relevant(function: &FunctionValues, target: &str) -> BTreeSet<String> {
    let nodes: BTreeMap<_, _> = function
        .model
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect();
    let calls: BTreeMap<_, _> = function
        .calls
        .iter()
        .map(|c| (c.definition.as_str(), c))
        .collect();
    let mut pending = vec![target.to_string()];
    let mut selected = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !selected.insert(id.clone()) {
            continue;
        }
        let Some(node) = nodes.get(id.as_str()) else {
            selected.remove(&id);
            continue;
        };
        pending.extend(node.operation.operands().into_iter().map(str::to_owned));
        pending.extend(node.guards.iter().cloned());
        if let Some(call) = calls.get(id.as_str()) {
            pending.extend(call.arguments.iter().cloned());
        }
    }
    selected.extend(function.parameters.iter().cloned());
    selected
}

/// Instantiate exact call contexts without recompiling or merging overloads.
/// Recursive backedges join actuals into an ancestor context and use the
/// portable solver's finite fixed point. That widening remains a frontier.
pub fn expand(
    functions: &[FunctionValues],
    owner: &str,
    target: &str,
    max_contexts: usize,
    max_nodes: usize,
) -> Result<ExpandedValues, &'static str> {
    if target.is_empty()
        || target.len() > 1024
        || target.chars().any(char::is_control)
        || functions.is_empty()
        || functions.len() > 256
        || max_contexts == 0
        || max_contexts > 128
        || max_nodes == 0
        || max_nodes > clew_facts::values::MAX_NODES
    {
        return Err("invalid direct-call program budget");
    }
    let registry: BTreeMap<_, _> = functions.iter().map(|f| (f.owner.as_str(), f)).collect();
    if registry.len() != functions.len() {
        return Err("ambiguous function owner");
    }
    let entry = *registry
        .get(owner)
        .ok_or("missing direct-call entry owner")?;
    for f in functions {
        f.model.validate()?;
        if f.owner.is_empty()
            || f.owner.len() > 1024
            || f.owner.chars().any(char::is_control)
            || f.model.binding != entry.model.binding
        {
            return Err("foreign direct-call program binding");
        }
        let nodes: BTreeMap<_, _> = f.model.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        if !nodes.contains_key("result")
            || f.parameters.len() > 128
            || f.calls.len() > 128
            || f.parameters.iter().collect::<BTreeSet<_>>().len() != f.parameters.len()
            || f.parameters.iter().any(|p| {
                !nodes.get(p.as_str()).is_some_and(|n| {
                    matches!(n.operation, ValueOperation::Input | ValueOperation::NonNull)
                })
            })
            || f.calls
                .iter()
                .map(|c| &c.definition)
                .collect::<BTreeSet<_>>()
                .len()
                != f.calls.len()
        {
            return Err("invalid function parameter or call definitions");
        }
        for call in &f.calls {
            if call.target.is_empty()
                || call.target.len() > 1024
                || call.target.chars().any(char::is_control)
                || !nodes
                    .get(call.definition.as_str())
                    .is_some_and(|n| matches!(n.operation, ValueOperation::Unknown { .. }))
                || call.arguments.len() > 128
                || call
                    .arguments
                    .iter()
                    .any(|a| !nodes.contains_key(a.as_str()))
            {
                return Err("invalid exact call transfer");
            }
            if registry
                .get(call.target.as_str())
                .is_some_and(|target| target.parameters.len() != call.arguments.len())
            {
                return Err("exact call arity differs from callee");
            }
        }
    }
    if !entry.model.nodes.iter().any(|n| n.id == target) {
        return Ok(ExpandedValues {
            model: entry.model.clone(),
            origins: entry
                .model
                .nodes
                .iter()
                .map(|n| ValueOrigin {
                    definition: n.id.clone(),
                    owner: owner.into(),
                    local_definition: n.id.clone(),
                })
                .collect(),
            summary_keys: vec![],
            summary_cache_hits: 0,
            context_count: 0,
            recursive_components: components(&registry),
        });
    }
    if relevant(entry, target).len() > max_nodes {
        return Err("entry model exceeds expansion node budget");
    }
    let recursive_components = components(&registry);
    let program_key = canonical::hash(&(POLICY, functions))
        .map_err(|_| "direct-call program serialization failed")?;

    // Remove functions with opaque operations/targets, then their callers, to a
    // fixed point. Recursive components survive only if every transfer is in
    // this closed value profile. No unknown field/call is dropped as irrelevant.
    let mut eligible: BTreeSet<&str> = registry.keys().copied().collect();
    loop {
        let rejected: Vec<_> = eligible
            .iter()
            .copied()
            .filter(|id| {
                let f = registry[id];
                let calls: BTreeSet<_> = f.calls.iter().map(|c| c.definition.as_str()).collect();
                f.model.nodes.iter().any(|n| {
                    matches!(n.operation, ValueOperation::Unknown { .. })
                        && !calls.contains(n.id.as_str())
                }) || f
                    .calls
                    .iter()
                    .any(|c| !eligible.contains(c.target.as_str()))
            })
            .collect();
        if rejected.is_empty() {
            break;
        }
        for id in rejected {
            eligible.remove(id);
        }
    }

    #[derive(Clone)]
    struct PreparedSummary {
        nodes: Vec<ValueNode>,
        calls: Vec<ExactValueCall>,
        key: String,
        used: bool,
    }
    struct Builder<'a> {
        registry: BTreeMap<&'a str, &'a FunctionValues>,
        eligible: BTreeSet<&'a str>,
        nodes: BTreeMap<String, ValueNode>,
        origins: BTreeMap<String, ValueOrigin>,
        boundaries: BTreeSet<String>,
        summaries: BTreeMap<(String, String), PreparedSummary>,
        hits: usize,
        contexts: usize,
        max_contexts: usize,
        max_nodes: usize,
        program_key: String,
        entry_target: String,
    }
    impl Builder<'_> {
        fn prepare(&mut self, owner: &str, target: &str) -> usize {
            let identity = (owner.to_owned(), target.to_owned());
            if !self.summaries.contains_key(&identity) {
                let function = self.registry[owner];
                let selected = relevant(function, target);
                let key = canonical::hash(&(
                    POLICY,
                    &self.program_key,
                    include_str!("thread_value_calls.rs"),
                    include_str!("thread_values.rs"),
                    function,
                    target,
                ))
                .expect("serializable summary");
                self.summaries.insert(
                    identity.clone(),
                    PreparedSummary {
                        nodes: function
                            .model
                            .nodes
                            .iter()
                            .filter(|n| selected.contains(&n.id))
                            .cloned()
                            .collect(),
                        calls: function
                            .calls
                            .iter()
                            .filter(|c| selected.contains(&c.definition))
                            .cloned()
                            .collect(),
                        key,
                        used: false,
                    },
                );
            }
            self.summaries[&identity].nodes.len()
        }
        fn unknown(&mut self, definition: &str, code: &str) {
            self.nodes.get_mut(definition).unwrap().operation = ValueOperation::Unknown {
                boundary: code.into(),
            };
        }
        fn instantiate(
            &mut self,
            owner: &str,
            actuals: Option<&[String]>,
            stack: &mut Vec<(String, String, usize)>,
        ) -> usize {
            let context = self.contexts;
            self.contexts += 1;
            let function = self.registry[owner];
            let selected_target = if context == 0 {
                self.entry_target.clone()
            } else {
                "result".into()
            };
            self.prepare(owner, &selected_target);
            let identity = (owner.to_owned(), selected_target);
            let summary = self.summaries[&identity].clone();
            if summary.used {
                self.hits += 1;
            }
            self.summaries.get_mut(&identity).unwrap().used = true;
            self.boundaries
                .extend(function.model.global_boundaries.iter().cloned());
            for n in &summary.nodes {
                let id = scoped(context, &n.id);
                self.nodes.insert(
                    id.clone(),
                    ValueNode {
                        id: id.clone(),
                        operation: remap(&n.operation, context),
                        evidence_ids: n.evidence_ids.clone(),
                        guards: n.guards.iter().map(|g| scoped(context, g)).collect(),
                    },
                );
                self.origins.insert(
                    id.clone(),
                    ValueOrigin {
                        definition: id,
                        owner: owner.into(),
                        local_definition: n.id.clone(),
                    },
                );
            }
            if let Some(arguments) = actuals {
                for (parameter, argument) in function.parameters.iter().zip(arguments) {
                    self.nodes
                        .get_mut(&scoped(context, parameter))
                        .unwrap()
                        .operation = ValueOperation::Copy {
                        value: argument.clone(),
                    };
                }
            }
            stack.push((owner.into(), identity.1.clone(), context));
            for call in &summary.calls {
                let id = scoped(context, &call.definition);
                let arguments: Vec<_> = call.arguments.iter().map(|a| scoped(context, a)).collect();
                if !self.eligible.contains(call.target.as_str()) {
                    self.unknown(&id, "VALUE_CALL_TARGET_OR_PURE_PROFILE_UNAVAILABLE");
                    continue;
                }
                if let Some((_, _, ancestor)) = stack
                    .iter()
                    .rev()
                    .find(|(owner, target, _)| owner == &call.target && target == "result")
                {
                    let ancestor = *ancestor;
                    let target = self.registry[call.target.as_str()];
                    // Context joining bounds a recursive component. Preserve
                    // incoming seeds and add recursive actual dependencies.
                    let extra = target
                        .parameters
                        .iter()
                        .filter(|p| {
                            !matches!(
                                self.nodes[&scoped(ancestor, p)].operation,
                                ValueOperation::Join { .. }
                            )
                        })
                        .count();
                    if self.nodes.len() + extra > self.max_nodes {
                        self.unknown(&id, "VALUE_CALL_NODE_BUDGET");
                        continue;
                    }
                    for (parameter, actual) in target.parameters.iter().zip(&arguments) {
                        let parameter = scoped(ancestor, parameter);
                        let current = self.nodes[&parameter].clone();
                        let mut alternatives =
                            if let ValueOperation::Join { values } = current.operation.clone() {
                                values
                            } else {
                                let seed = format!("{parameter}/incoming");
                                self.nodes.insert(
                                    seed.clone(),
                                    ValueNode {
                                        id: seed.clone(),
                                        ..current.clone()
                                    },
                                );
                                let origin = self.origins[&parameter].clone();
                                self.origins.insert(
                                    seed.clone(),
                                    ValueOrigin {
                                        definition: seed.clone(),
                                        ..origin
                                    },
                                );
                                vec![seed]
                            };
                        alternatives.push(actual.clone());
                        alternatives.sort();
                        alternatives.dedup();
                        self.nodes.get_mut(&parameter).unwrap().operation = ValueOperation::Join {
                            values: alternatives,
                        };
                    }
                    self.nodes.get_mut(&id).unwrap().operation = ValueOperation::Copy {
                        value: scoped(ancestor, "result"),
                    };
                    self.boundaries
                        .insert("VALUE_RECURSIVE_CONTEXT_JOINED".into());
                    continue;
                }
                if self.contexts >= self.max_contexts {
                    self.unknown(&id, "VALUE_CALL_CONTEXT_BUDGET");
                    continue;
                }
                let size = self.prepare(&call.target, "result");
                if self.nodes.len() + size > self.max_nodes {
                    self.unknown(&id, "VALUE_CALL_NODE_BUDGET");
                    continue;
                }
                let callee = self.instantiate(&call.target, Some(&arguments), stack);
                self.nodes.get_mut(&id).unwrap().operation = ValueOperation::Copy {
                    value: scoped(callee, "result"),
                };
            }
            stack.pop();
            context
        }
    }
    let mut builder = Builder {
        registry,
        eligible,
        nodes: BTreeMap::new(),
        origins: BTreeMap::new(),
        boundaries: BTreeSet::new(),
        summaries: BTreeMap::new(),
        hits: 0,
        contexts: 0,
        max_contexts,
        max_nodes,
        program_key,
        entry_target: target.into(),
    };
    builder.instantiate(owner, None, &mut Vec::new());
    let mut binding: ModelBinding = entry.model.binding.clone();
    binding.producer = POLICY.into();
    binding.policy_digest = canonical::hash_bytes(POLICY.as_bytes());
    let model = ValueModel {
        schema: clew_facts::values::SCHEMA.into(),
        binding,
        nodes: builder.nodes.into_values().collect(),
        global_boundaries: builder.boundaries.into_iter().collect(),
    };
    model.validate()?;
    Ok(ExpandedValues {
        model,
        origins: builder.origins.into_values().collect(),
        summary_keys: builder
            .summaries
            .into_values()
            .filter(|s| s.used)
            .map(|s| s.key)
            .collect(),
        summary_cache_hits: builder.hits,
        context_count: builder.contexts,
        recursive_components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_values::{FrontierKind, Nullness, ValueQuery, analyze};
    use clew_facts::values::ObservationAuthority;

    fn node(id: &str, operation: ValueOperation) -> ValueNode {
        let guards = if let ValueOperation::Coalesce { left, .. } = &operation {
            vec![left.clone()]
        } else {
            vec![]
        };
        ValueNode {
            id: id.into(),
            operation,
            guards,
            evidence_ids: vec![format!("fact#{id}")],
        }
    }
    fn copy(id: &str, value: &str) -> ValueNode {
        node(
            id,
            ValueOperation::Copy {
                value: value.into(),
            },
        )
    }
    fn pending(id: &str) -> ValueNode {
        node(
            id,
            ValueOperation::Unknown {
                boundary: "CALL_NOT_EXPANDED".into(),
            },
        )
    }
    fn function(
        owner: &str,
        parameters: &[&str],
        mut nodes: Vec<ValueNode>,
        calls: Vec<ExactValueCall>,
    ) -> FunctionValues {
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let zero = format!("sha256:{}", "0".repeat(64));
        FunctionValues {
            owner: owner.into(),
            parameters: parameters.iter().map(|s| (*s).into()).collect(),
            calls,
            model: ValueModel {
                schema: clew_facts::values::SCHEMA.into(),
                nodes,
                global_boundaries: vec!["EFFECTS_UNQUALIFIED".into()],
                binding: ModelBinding {
                    language: "test".into(),
                    producer: "test".into(),
                    snapshot_digest: zero.clone(),
                    facts_digest: zero.clone(),
                    configuration_digest: zero.clone(),
                    policy_digest: zero,
                    observation_authority: ObservationAuthority::Declared,
                },
            },
        }
    }
    fn call(id: &str, target: &str, argument: &str) -> ExactValueCall {
        ExactValueCall {
            definition: id.into(),
            target: target.into(),
            arguments: vec![argument.into()],
        }
    }
    fn helper() -> FunctionValues {
        function(
            "identity(String)",
            &["p"],
            vec![node("p", ValueOperation::Input), copy("result", "p")],
            vec![],
        )
    }
    fn caller() -> FunctionValues {
        function(
            "caller",
            &[],
            vec![
                pending("a"),
                pending("b"),
                node("nil", ValueOperation::Null),
                node("text", ValueOperation::NonNull),
                node(
                    "selected",
                    ValueOperation::Coalesce {
                        left: "a".into(),
                        right: "b".into(),
                    },
                ),
                copy("result", "selected"),
            ],
            vec![
                call("a", "identity(String)", "nil"),
                call("b", "identity(String)", "text"),
            ],
        )
    }
    fn query() -> ValueQuery {
        ValueQuery {
            target: "result".into(),
            max_nodes: 4096,
            max_transfers: 4096,
        }
    }
    fn result(expanded: &ExpandedValues, target: &str) -> Nullness {
        let answer = analyze(
            &expanded.model,
            &ValueQuery {
                target: target.into(),
                ..query()
            },
        )
        .unwrap();
        answer
            .observations
            .iter()
            .find(|n| n.id == target)
            .unwrap()
            .normal_value_nullness
    }

    #[test]
    fn helper_reuse_preserves_different_call_contexts_and_provenance() {
        let functions = vec![caller(), helper()];
        let answer = expand(&functions, "caller", "result", 32, 4096).unwrap();
        assert_eq!(answer.context_count, 3);
        assert_eq!(answer.summary_cache_hits, 1);
        assert_eq!(result(&answer, "result"), Nullness::NonNull);
        assert_eq!(result(&answer, "c1/result"), Nullness::Null);
        assert_eq!(result(&answer, "c2/result"), Nullness::NonNull);
        assert_eq!(
            answer
                .origins
                .iter()
                .filter(|o| o.owner == "identity(String)" && o.local_definition == "result")
                .count(),
            2
        );
        assert_eq!(
            canonical::bytes(&answer).unwrap(),
            canonical::bytes(&expand(&functions, "caller", "result", 32, 4096).unwrap()).unwrap()
        );
        let mut only_null = caller();
        only_null
            .model
            .nodes
            .iter_mut()
            .find(|n| n.id == "result")
            .unwrap()
            .operation = ValueOperation::Copy { value: "a".into() };
        let answer = expand(&[only_null, helper()], "caller", "result", 32, 4096).unwrap();
        assert_eq!(answer.context_count, 2);
        assert_eq!(result(&answer, "result"), Nullness::Null);
        assert!(!answer.origins.iter().any(|o| o.local_definition == "b"));
        let mut entry = caller();
        entry
            .model
            .nodes
            .iter_mut()
            .find(|n| n.id == "result")
            .unwrap()
            .operation = ValueOperation::Copy { value: "a".into() };
        let answer = expand(&[entry, helper()], "caller", "b", 32, 4096).unwrap();
        assert!(
            !answer
                .model
                .nodes
                .iter()
                .any(|n| n.id == "result" || n.id == "a")
        );
        assert_eq!(result(&answer, "b"), Nullness::NonNull);
        assert_eq!(answer.context_count, 2);
    }

    #[test]
    fn opaque_transfers_and_call_budgets_remain_frontiers() {
        let mut opaque = helper();
        opaque.model.nodes.push(node(
            "unused-field",
            ValueOperation::Unknown {
                boundary: "FIELD_OPAQUE".into(),
            },
        ));
        opaque.model.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let answer = expand(&[caller(), opaque], "caller", "result", 32, 4096).unwrap();
        assert_eq!(result(&answer, "result"), Nullness::Unknown);
        let answer = expand(&[caller(), helper()], "caller", "result", 1, 4096).unwrap();
        let result = analyze(&answer.model, &query()).unwrap();
        assert!(
            result
                .frontier
                .iter()
                .any(|f| f.kind == FrontierKind::Budget && f.code == "VALUE_CALL_CONTEXT_BUDGET")
        );
        let answer = expand(&[caller(), helper()], "caller", "result", 32, 6).unwrap();
        assert!(
            analyze(&answer.model, &query())
                .unwrap()
                .frontier
                .iter()
                .any(|f| f.kind == FrontierKind::Budget && f.code == "VALUE_CALL_NODE_BUDGET")
        );
    }

    #[test]
    fn recursive_components_join_contexts_and_converge_without_inventing_returns() {
        let a = function(
            "A",
            &["p"],
            vec![
                node("p", ValueOperation::Input),
                pending("recur"),
                copy("result", "recur"),
            ],
            vec![call("recur", "B", "p")],
        );
        let b = function(
            "B",
            &["p"],
            vec![
                node("p", ValueOperation::Input),
                pending("recur"),
                copy("result", "recur"),
            ],
            vec![call("recur", "A", "p")],
        );
        let answer = expand(&[a, b], "A", "result", 32, 4096).unwrap();
        assert_eq!(answer.recursive_components, [vec!["A", "B"]]);
        assert_eq!(answer.context_count, 2);
        assert_eq!(result(&answer, "result"), Nullness::Unknown);
        let result = analyze(&answer.model, &query()).unwrap();
        assert!(result.equations_converged);
        assert!(!result.program_semantic_closure);
        assert!(
            result
                .frontier
                .iter()
                .any(|f| f.code == "VALUE_RECURSIVE_CONTEXT_JOINED")
        );
    }

    #[test]
    fn recursive_local_query_has_a_separate_return_context() {
        let a = function(
            "A",
            &["p"],
            vec![
                node("p", ValueOperation::Input),
                pending("local"),
                copy("result", "local"),
            ],
            vec![call("local", "B", "p")],
        );
        let b = function(
            "B",
            &["p"],
            vec![
                node("p", ValueOperation::Input),
                pending("local"),
                copy("result", "local"),
            ],
            vec![call("local", "A", "p")],
        );
        let answer = expand(&[a, b], "A", "local", 32, 4096).unwrap();
        assert_eq!(answer.context_count, 3);
        assert!(!answer.model.nodes.iter().any(|n| n.id == "result"));
        assert_eq!(result(&answer, "local"), Nullness::Unknown);
        assert!(answer.model.nodes.iter().any(|n| n.id == "c2/result"));
        let missing = expand(&[caller(), helper()], "caller", "missing", 32, 4096).unwrap();
        assert_eq!(missing.context_count, 0);
        assert!(
            analyze(
                &missing.model,
                &ValueQuery {
                    target: "missing".into(),
                    ..query()
                }
            )
            .unwrap()
            .frontier
            .iter()
            .any(|f| f.kind == FrontierKind::Discovery)
        );
    }

    #[test]
    fn foreign_bindings_overloads_and_freshness_are_not_merged() {
        let mut foreign = helper();
        foreign.model.binding.configuration_digest = format!("sha256:{}", "1".repeat(64));
        assert!(expand(&[caller(), foreign], "caller", "result", 32, 4096).is_err());
        let overload = function(
            "identity(Int)",
            &["p"],
            vec![
                node("p", ValueOperation::NonNull),
                node("result", ValueOperation::NonNull),
            ],
            vec![],
        );
        let answer = expand(
            &[caller(), helper(), overload],
            "caller",
            "result",
            32,
            4096,
        )
        .unwrap();
        assert!(answer.origins.iter().all(|o| o.owner != "identity(Int)"));
        let baseline = answer.summary_keys;
        let mut changed = helper();
        changed
            .model
            .nodes
            .iter_mut()
            .find(|n| n.id == "result")
            .unwrap()
            .operation = ValueOperation::NonNull;
        let answer = expand(&[caller(), changed], "caller", "result", 32, 4096).unwrap();
        assert_ne!(baseline, answer.summary_keys);
    }
}
