use crate::canonical;
use crate::model::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

pub fn enrich(mut graph: LocalGraph) -> LocalGraph {
    for node in &mut graph.nodes {
        node.editable = node.origin.is_some() && !node.kind.starts_with("PHI");
    }
    add_call_edges(&mut graph);
    add_effect_edges(&mut graph);
    let entry = graph
        .nodes
        .iter()
        .find(|node| {
            node.kind == "ENTRY"
                && node
                    .attributes
                    .get("firNodeKind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.contains("FunctionEnter"))
        })
        .or_else(|| graph.nodes.iter().find(|node| node.id == "entry"))
        .or_else(|| graph.nodes.iter().find(|node| node.kind == "ENTRY"))
        .map(|node| node.id.clone())
        .unwrap_or_else(|| "entry".into());
    let exit = graph
        .nodes
        .iter()
        .find(|node| {
            node.kind == "EXIT"
                && node
                    .attributes
                    .get("firNodeKind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.contains("FunctionExit"))
        })
        .or_else(|| graph.nodes.iter().find(|node| node.id == "exit"))
        .or_else(|| graph.nodes.iter().find(|node| node.kind == "EXIT"))
        .map(|node| node.id.clone())
        .unwrap_or_else(|| "exit".into());
    add_ssa_and_def_use(&mut graph, &entry);
    add_control_dependencies(&mut graph, &exit);
    graph.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    graph.edges.sort();
    graph.edges.dedup();
    graph
}

fn add_call_edges(graph: &mut LocalGraph) {
    let calls: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == "CALL")
        .cloned()
        .collect();
    for call in calls {
        let symbol = call
            .attributes
            .get("symbol")
            .and_then(Value::as_str)
            .unwrap_or("<unresolved>");
        let summary_hash = call
            .attributes
            .get("calleeSummaryHash")
            .and_then(Value::as_str)
            .unwrap_or("sha256:unknown");
        let callee_id = format!(
            "callee:{}",
            canonical::hash(&json!({"symbol":symbol,"summary":summary_hash}))
                .unwrap_or_else(|_| "sha256:unknown".into())
                .trim_start_matches("sha256:")
        );
        if !graph.nodes.iter().any(|node| node.id == callee_id) {
            let mut attributes = BTreeMap::new();
            attributes.insert("symbol".into(), json!(symbol));
            attributes.insert("calleeSummaryHash".into(), json!(summary_hash));
            graph.nodes.push(GraphNode {
                id: callee_id.clone(),
                kind: "CALLEE_SUMMARY".into(),
                defines: None,
                uses: vec![],
                origin: None,
                editable: false,
                attributes,
            });
        }
        graph.edges.push(Edge {
            from: call.id.clone(),
            to: callee_id.clone(),
            kind: "CALL".into(),
        });
        graph.edges.push(Edge {
            from: callee_id,
            to: call.id,
            kind: "RETURN".into(),
        });
    }
}

fn add_effect_edges(graph: &mut LocalGraph) {
    let originals = graph.nodes.clone();
    for node in originals {
        let mut effects: BTreeSet<String> = node
            .attributes
            .get("effects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        if node.kind == "CALL" && effects.is_empty() && !is_supported_intrinsic(&node) {
            effects.insert("READ_STATE".into());
        }
        for effect in effects {
            let id = format!("effect:{}:{}", node.id, effect.to_lowercase());
            let mut attributes = BTreeMap::new();
            attributes.insert("effect".into(), json!(effect));
            attributes.insert(
                "memoryKind".into(),
                node.attributes
                    .get("memoryKind")
                    .cloned()
                    .unwrap_or_else(|| json!("UNKNOWN_HEAP")),
            );
            attributes.insert(
                "memoryLocation".into(),
                node.attributes
                    .get("memoryLocation")
                    .cloned()
                    .unwrap_or_else(|| json!("UNKNOWN_HEAP")),
            );
            graph.nodes.push(GraphNode {
                id: id.clone(),
                kind: "EFFECT".into(),
                defines: None,
                uses: vec![],
                origin: node.origin.clone(),
                editable: false,
                attributes,
            });
            graph.edges.push(Edge {
                from: node.id.clone(),
                to: id,
                kind: effect,
            });
        }
    }
}

#[allow(dead_code)]
const MEMORY_ABSTRACTIONS: [&str; 4] = [
    "THIS_PROPERTY",
    "OBJECT_PROPERTY",
    "STATIC_PROPERTY",
    "UNKNOWN_HEAP",
];

fn is_supported_intrinsic(node: &GraphNode) -> bool {
    let symbol = node
        .attributes
        .get("symbol")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = symbol.rsplit(['.', '/']).next().unwrap_or_default();
    symbol.starts_with("kotlin/")
        && matches!(
            name,
            "plus"
                | "minus"
                | "times"
                | "div"
                | "rem"
                | "compareTo"
                | "inc"
                | "dec"
                | "not"
                | "unaryMinus"
                | "unaryPlus"
        )
        && node
            .attributes
            .get("effects")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
}

fn add_ssa_and_def_use(graph: &mut LocalGraph, entry: &str) {
    let dominator_sets = dominators(graph, entry, false);
    let immediate = immediate_dominators(&dominator_sets);
    let mut frontier: BTreeMap<String, BTreeSet<String>> = graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), BTreeSet::new()))
        .collect();
    let predecessors = cfg_predecessors(graph);
    for (block, incoming) in &predecessors {
        if incoming.len() < 2 {
            continue;
        }
        for predecessor in incoming {
            let mut runner = Some(predecessor.clone());
            while let Some(current) = runner {
                if immediate.get(block) == Some(&current) {
                    break;
                }
                frontier
                    .entry(current.clone())
                    .or_default()
                    .insert(block.clone());
                runner = immediate.get(&current).cloned();
            }
        }
    }

    let mut definitions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in &graph.nodes {
        if let Some(variable) = &node.defines {
            definitions
                .entry(variable.clone())
                .or_default()
                .insert(node.id.clone());
        }
    }
    let mut phi_at: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (variable, sites) in &definitions {
        let mut work: VecDeque<String> = sites.iter().cloned().collect();
        let mut placed = BTreeSet::new();
        while let Some(site) = work.pop_front() {
            for join in frontier.get(&site).into_iter().flatten() {
                if placed.insert(join.clone()) {
                    phi_at
                        .entry(join.clone())
                        .or_default()
                        .insert(variable.clone());
                    if !sites.contains(join) {
                        work.push_back(join.clone());
                    }
                }
            }
        }
    }

    let mut phi_ids = BTreeMap::new();
    for (block, variables) in &phi_at {
        for variable in variables {
            let id = format!("phi:{variable}@{block}");
            phi_ids.insert((block.clone(), variable.clone()), id.clone());
            let mut attributes = BTreeMap::new();
            attributes.insert("joinBlock".into(), json!(block));
            graph.nodes.push(GraphNode {
                id,
                kind: "PHI".into(),
                defines: Some(variable.clone()),
                uses: vec![],
                origin: None,
                editable: false,
                attributes,
            });
        }
    }

    let originals: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter(|node| node.kind != "PHI")
        .map(|node| (node.id.clone(), node.clone()))
        .collect();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (node, parent) in &immediate {
        children
            .entry(parent.clone())
            .or_default()
            .push(node.clone());
    }
    for values in children.values_mut() {
        values.sort();
    }
    let successors = cfg_successors(graph);
    let mut stacks: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let mut events = vec![RenameEvent::Enter(entry.into())];
    while let Some(event) = events.pop() {
        match event {
            RenameEvent::Exit(pushed) => {
                for variable in pushed.into_iter().rev() {
                    let _ = stacks.get_mut(&variable).and_then(Vec::pop);
                }
            }
            RenameEvent::Enter(block) => {
                let mut pushed = Vec::new();
                for variable in phi_at.get(&block).into_iter().flatten() {
                    let id = phi_ids[&(block.clone(), variable.clone())].clone();
                    stacks.entry(variable.clone()).or_default().push(id);
                    pushed.push(variable.clone());
                }
                if let Some(node) = originals.get(&block) {
                    for used in &node.uses {
                        if let Some(definition) = stacks.get(used).and_then(|stack| stack.last()) {
                            graph.edges.push(Edge {
                                from: definition.clone(),
                                to: block.clone(),
                                kind: "DEF_USE".into(),
                            });
                        }
                    }
                    if let Some(variable) = &node.defines {
                        let version = versions.entry(variable.clone()).or_default();
                        if let Some(target) = graph.nodes.iter_mut().find(|item| item.id == block) {
                            target
                                .attributes
                                .insert("ssaVersion".into(), json!(*version));
                        }
                        *version += 1;
                        stacks
                            .entry(variable.clone())
                            .or_default()
                            .push(block.clone());
                        pushed.push(variable.clone());
                    }
                }
                for successor in successors.get(&block).into_iter().flatten() {
                    for variable in phi_at.get(successor).into_iter().flatten() {
                        if let Some(definition) =
                            stacks.get(variable).and_then(|stack| stack.last())
                        {
                            graph.edges.push(Edge {
                                from: definition.clone(),
                                to: phi_ids[&(successor.clone(), variable.clone())].clone(),
                                kind: "PHI_INPUT".into(),
                            });
                        }
                    }
                }
                events.push(RenameEvent::Exit(pushed));
                for child in children.get(&block).into_iter().flatten().rev() {
                    events.push(RenameEvent::Enter(child.clone()));
                }
            }
        }
    }
}

enum RenameEvent {
    Enter(String),
    Exit(Vec<String>),
}

fn immediate_dominators(sets: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, String> {
    sets.iter()
        .filter_map(|(node, dominators)| {
            dominators
                .iter()
                .filter(|candidate| *candidate != node)
                .max_by_key(|candidate| sets.get(*candidate).map_or(0, BTreeSet::len))
                .map(|parent| (node.clone(), parent.clone()))
        })
        .collect()
}

fn cfg_predecessors(graph: &LocalGraph) -> BTreeMap<String, Vec<String>> {
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.kind.starts_with("CFG_"))
    {
        result
            .entry(edge.to.clone())
            .or_default()
            .push(edge.from.clone());
    }
    result
}

fn cfg_successors(graph: &LocalGraph) -> BTreeMap<String, Vec<String>> {
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.kind.starts_with("CFG_"))
    {
        result
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
    }
    result
}

fn add_control_dependencies(graph: &mut LocalGraph, exit: &str) {
    let postdominators = dominators(graph, exit, true);
    let immediate: BTreeMap<String, Option<String>> = postdominators
        .iter()
        .map(|(node, set)| {
            let parent = set
                .iter()
                .filter(|candidate| *candidate != node)
                .max_by_key(|candidate| postdominators.get(*candidate).map_or(0, BTreeSet::len))
                .cloned();
            (node.clone(), parent)
        })
        .collect();
    let cfg_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind.starts_with("CFG_") && edge.kind != "CFG_BACK")
        .cloned()
        .collect();
    for edge in cfg_edges {
        if postdominators
            .get(&edge.from)
            .is_some_and(|set| set.contains(&edge.to))
        {
            continue;
        }
        let stop = immediate.get(&edge.from).and_then(Clone::clone);
        let mut runner = Some(edge.to.clone());
        let mut seen = BTreeSet::new();
        while let Some(node) = runner {
            if Some(&node) == stop.as_ref() || !seen.insert(node.clone()) {
                break;
            }
            graph.edges.push(Edge {
                from: edge.from.clone(),
                to: node.clone(),
                kind: "CONTROL_DEP".into(),
            });
            runner = immediate.get(&node).and_then(Clone::clone);
        }
    }
}

pub fn dominators(
    graph: &LocalGraph,
    entry: &str,
    reverse: bool,
) -> BTreeMap<String, BTreeSet<String>> {
    let all: BTreeSet<_> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    let mut dom: BTreeMap<String, BTreeSet<String>> = all
        .iter()
        .map(|n| {
            (
                n.clone(),
                if n == entry {
                    [n.clone()].into()
                } else {
                    all.clone()
                },
            )
        })
        .collect();
    loop {
        let mut changed = false;
        for node in all.iter().filter(|n| n.as_str() != entry) {
            let predecessors: Vec<_> = graph
                .edges
                .iter()
                .filter_map(|e| {
                    let (from, to) = if reverse {
                        (&e.to, &e.from)
                    } else {
                        (&e.from, &e.to)
                    };
                    (to == node && e.kind.starts_with("CFG_")).then_some(from)
                })
                .collect();
            let mut next = if predecessors.is_empty() {
                BTreeSet::new()
            } else {
                all.clone()
            };
            for predecessor in predecessors {
                next = next.intersection(&dom[predecessor]).cloned().collect();
            }
            next.insert(node.clone());
            if next != dom[node] {
                dom.insert(node.clone(), next);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    dom
}

pub fn slice(
    graph: &LocalGraph,
    seed_id: &str,
    policy: SlicePolicy,
    snapshot: Snapshot,
    seed: Value,
) -> anyhow::Result<ThreadIr> {
    let started = Instant::now();
    let mut include: BTreeSet<_> = policy.include_edges.iter().cloned().collect();
    // PHI inputs are an intrinsic part of a DEF_USE walk even though the public
    // slice policy names DEF_USE rather than the SSA-internal PHI_INPUT edge.
    if include.contains("DEF_USE") {
        include.insert("PHI_INPUT".to_owned());
    }
    let mut selected: BTreeSet<String> = [seed_id.to_owned()].into();
    let mut queue = VecDeque::from([seed_id.to_owned()]);
    let mut budget_hit = false;
    while let Some(current) = queue.pop_front() {
        if selected.len() >= policy.max_nodes
            || started.elapsed().as_millis() as u64 >= policy.deadline_ms
        {
            budget_hit = true;
            break;
        }
        for edge in graph.edges.iter().filter(|e| include.contains(&e.kind)) {
            let next = match policy.direction {
                Direction::Forward if edge.from == current => Some(&edge.to),
                Direction::Backward if edge.to == current => Some(&edge.from),
                Direction::Both if edge.from == current => Some(&edge.to),
                Direction::Both if edge.to == current => Some(&edge.from),
                _ => None,
            };
            if let Some(next) = next
                && selected.insert(next.clone())
            {
                queue.push_back(next.clone());
            }
        }
    }
    let mut nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| selected.contains(&n.id))
        .cloned()
        .collect();
    let mut edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| selected.contains(&e.from) && selected.contains(&e.to))
        .cloned()
        .collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    edges.sort();
    // A depth-zero call anywhere in the selected local function can influence
    // a source-backed seed through control/value evaluation. Until a call
    // summary proves otherwise, completeness must remain partial.
    let external_calls: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "CALL" && !is_supported_intrinsic(n))
        .collect();
    let external = !external_calls.is_empty();
    let unsupported = !graph.boundaries.is_empty();
    let status = if budget_hit {
        CompletenessStatus::PartialBudget
    } else if unsupported {
        CompletenessStatus::PartialUnsupportedFeature
    } else if external {
        CompletenessStatus::PartialExternalBoundary
    } else {
        CompletenessStatus::CompleteSupportedSubset
    };
    let boundaries = if budget_hit {
        vec![json!({"kind":"BUDGET","maxNodes":policy.max_nodes})]
    } else if unsupported {
        graph.boundaries.clone()
    } else if external {
        external_calls.iter().map(|node| json!({"kind":"EXTERNAL_CALL","nodeId":node.id,"symbol":node.attributes.get("symbol"),"reason":"maxCallDepth=0"})).collect()
    } else {
        vec![]
    };
    let mut read_set = Vec::new();
    for node in &nodes {
        if let Some(origin) = &node.origin
            && let (Some(key), Some(hash)) = (
                origin.get("anchorId").and_then(Value::as_str),
                origin.get("exactTextHash").and_then(Value::as_str),
            )
        {
            read_set.push(ReadFact {
                kind: "SOURCE_NODE".into(),
                key: key.into(),
                hash: hash.into(),
            });
            if let Some(signature) = origin.get("ownerSignatureHash").and_then(Value::as_str) {
                read_set.push(ReadFact {
                    kind: "OWNER_SIGNATURE".into(),
                    key: origin
                        .get("ownerSymbolId")
                        .and_then(Value::as_str)
                        .unwrap_or(key)
                        .into(),
                    hash: signature.into(),
                });
            }
        }
        let semantic_key = node
            .origin
            .as_ref()
            .and_then(|origin| origin.get("anchorId"))
            .and_then(Value::as_str)
            .unwrap_or(&node.id);
        for (attribute, kind) in [
            ("symbol", "RESOLVED_SYMBOL"),
            ("type", "EXPRESSION_TYPE"),
            ("receiverType", "RECEIVER_TYPE"),
            ("calleeSummaryHash", "CALLEE_SUMMARY"),
        ] {
            if let Some(value) = node.attributes.get(attribute) {
                let hash = if attribute == "calleeSummaryHash" {
                    value.as_str().unwrap_or_default().to_owned()
                } else {
                    canonical::hash(value)?
                };
                read_set.push(ReadFact {
                    kind: kind.into(),
                    key: format!("{semantic_key}:{attribute}"),
                    hash,
                });
                if attribute == "symbol" && node.kind == "CALL" {
                    read_set.push(ReadFact {
                        kind: "CALL_TARGET".into(),
                        key: format!("{semantic_key}:callTarget"),
                        hash: canonical::hash(value)?,
                    });
                }
            }
        }
    }
    read_set.push(ReadFact {
        kind: "PROJECT_MODEL".into(),
        key: snapshot.project_model_hash.clone(),
        hash: snapshot.project_model_hash.clone(),
    });
    read_set.push(ReadFact {
        kind: "DIAGNOSTICS".into(),
        key: graph.symbol.clone(),
        hash: canonical::hash(&graph.diagnostics)?,
    });
    if let Some(hash) = &graph.compiler_options_hash {
        read_set.push(ReadFact {
            kind: "COMPILER_OPTIONS".into(),
            key: graph.symbol.clone(),
            hash: hash.clone(),
        });
    }
    if let Some(hash) = &graph.classpath_hash {
        read_set.push(ReadFact {
            kind: "CLASSPATH".into(),
            key: graph.symbol.clone(),
            hash: hash.clone(),
        });
    }
    for fact in &graph.inheritance_facts {
        read_set.push(ReadFact {
            kind: "INHERITANCE".into(),
            key: fact
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or(&graph.symbol)
                .into(),
            hash: canonical::hash(fact)?,
        });
    }
    read_set.sort();
    read_set.dedup();
    let thread_id = format!(
        "thread:{}",
        canonical::hash(
            &json!({"snapshot":snapshot,"seed":seed,"policy":policy,"nodes":nodes,"edges":edges})
        )?
        .trim_start_matches("sha256:")
    );
    let editable_units = nodes
        .iter()
        .filter(|n| n.editable)
        .filter_map(|n| n.origin.clone())
        .collect();
    Ok(ThreadIr {
        schema: "semantic-thread/0.1".into(),
        thread_id,
        snapshot,
        seed,
        policy,
        completeness: Completeness { status, boundaries },
        nodes,
        edges,
        editable_units,
        external_summaries: external_calls
            .iter()
            .map(|node| {
                json!({
                    "nodeId": node.id,
                    "symbol": node.attributes.get("symbol"),
                    "receiverType": node.attributes.get("receiverType"),
                    "calleeSummaryHash": node.attributes.get("calleeSummaryHash"),
                    "effects": node.attributes.get("effects")
                })
            })
            .collect(),
        read_set,
        validation_plan: vec![
            "SYNTAX".into(),
            "K2_DIAGNOSTICS".into(),
            "TYPE".into(),
            "PROTECTED_BINDINGS".into(),
            "EFFECTS".into(),
            "GRADLE_COMPILE".into(),
            "TESTS".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str, defines: Option<&str>, uses: &[&str]) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: kind.into(),
            defines: defines.map(str::to_owned),
            uses: uses.iter().map(|s| s.to_string()).collect(),
            origin: None,
            editable: false,
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn adds_phi_def_use_and_control_dependencies() {
        let graph = LocalGraph {
            schema: "local-cfg/0.1".into(),
            symbol: "total".into(),
            file: "Total.kt".into(),
            nodes: vec![
                node("entry", "ENTRY", None, &[]),
                node("base", "PARAMETER", Some("base"), &[]),
                node("premium", "PARAMETER", Some("premium"), &[]),
                node("init", "DEFINITION", Some("value"), &["base"]),
                node("if", "BRANCH", None, &["premium"]),
                node("double", "ASSIGNMENT", Some("value"), &["value"]),
                node("ret", "RETURN", None, &["value"]),
                node("exit", "EXIT", None, &[]),
            ],
            edges: vec![
                Edge {
                    from: "entry".into(),
                    to: "base".into(),
                    kind: "CFG_NORMAL".into(),
                },
                Edge {
                    from: "base".into(),
                    to: "premium".into(),
                    kind: "CFG_NORMAL".into(),
                },
                Edge {
                    from: "premium".into(),
                    to: "init".into(),
                    kind: "CFG_NORMAL".into(),
                },
                Edge {
                    from: "init".into(),
                    to: "if".into(),
                    kind: "CFG_NORMAL".into(),
                },
                Edge {
                    from: "if".into(),
                    to: "double".into(),
                    kind: "CFG_TRUE".into(),
                },
                Edge {
                    from: "if".into(),
                    to: "ret".into(),
                    kind: "CFG_FALSE".into(),
                },
                Edge {
                    from: "double".into(),
                    to: "ret".into(),
                    kind: "CFG_NORMAL".into(),
                },
                Edge {
                    from: "ret".into(),
                    to: "exit".into(),
                    kind: "CFG_NORMAL".into(),
                },
            ],
            boundaries: vec![],
            diagnostics: vec![],
            compiler_options_hash: None,
            classpath_hash: None,
            inheritance_facts: vec![],
        };
        let enriched = enrich(graph);
        assert!(enriched.nodes.iter().any(|n| n.id == "phi:value@ret"));
        assert!(
            enriched
                .edges
                .iter()
                .any(|e| e.kind == "PHI_INPUT" && e.from == "double"),
            "edges: {:?}",
            enriched.edges
        );
        assert!(
            enriched
                .edges
                .iter()
                .any(|e| e.kind == "DEF_USE" && e.to == "ret")
        );
        assert!(
            enriched
                .edges
                .iter()
                .any(|e| e.kind == "CONTROL_DEP" && e.from == "if")
        );
    }

    #[test]
    fn budget_is_explicitly_partial() {
        let graph = LocalGraph {
            schema: "local-cfg/0.1".into(),
            symbol: "x".into(),
            file: "X.kt".into(),
            nodes: vec![
                node("a", "EXPRESSION", None, &[]),
                node("b", "EXPRESSION", None, &[]),
            ],
            edges: vec![Edge {
                from: "a".into(),
                to: "b".into(),
                kind: "DEF_USE".into(),
            }],
            boundaries: vec![],
            diagnostics: vec![],
            compiler_options_hash: None,
            classpath_hash: None,
            inheritance_facts: vec![],
        };
        let ir = slice(
            &graph,
            "a",
            SlicePolicy {
                max_nodes: 1,
                ..Default::default()
            },
            Snapshot {
                base_revision: "x".into(),
                project_model_hash: "p".into(),
                compiler_version: "2.4.10".into(),
            },
            json!({}),
        )
        .unwrap();
        assert_eq!(ir.completeness.status, CompletenessStatus::PartialBudget);
    }
}
