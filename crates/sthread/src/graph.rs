use crate::canonical;
use crate::model::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

pub fn enrich(mut graph: LocalGraph) -> LocalGraph {
    for node in &mut graph.nodes {
        node.editable = node.origin.is_some() && !node.kind.starts_with("PHI");
    }
    let mut definitions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in &graph.nodes {
        if let Some(name) = &node.defines {
            definitions
                .entry(name.clone())
                .or_default()
                .push(node.id.clone());
        }
    }
    let mut phi_for = BTreeMap::new();
    for (name, defs) in definitions.iter().filter(|(_, defs)| defs.len() > 1) {
        let id = format!("phi:{}", name);
        graph.nodes.push(GraphNode {
            id: id.clone(),
            kind: "PHI".into(),
            defines: Some(name.clone()),
            uses: vec![],
            origin: None,
            editable: false,
            attributes: BTreeMap::new(),
        });
        for def in defs {
            graph.edges.push(Edge {
                from: def.clone(),
                to: id.clone(),
                kind: "PHI_INPUT".into(),
            });
        }
        phi_for.insert(name.clone(), id);
    }
    for node in &graph.nodes.clone() {
        for used in &node.uses {
            if let Some(phi) = phi_for.get(used) {
                if phi != &node.id {
                    graph.edges.push(Edge {
                        from: phi.clone(),
                        to: node.id.clone(),
                        kind: "DEF_USE".into(),
                    });
                }
            } else if let Some(def) = definitions.get(used).and_then(|d| d.last())
                && def != &node.id
            {
                graph.edges.push(Edge {
                    from: def.clone(),
                    to: node.id.clone(),
                    kind: "DEF_USE".into(),
                });
            }
        }
    }
    add_control_dependencies(&mut graph);
    graph.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    graph.edges.sort();
    graph.edges.dedup();
    graph
}

fn add_control_dependencies(graph: &mut LocalGraph) {
    let postdominators = dominators(graph, "exit", true);
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
    let external = nodes.iter().any(|n| n.kind == "CALL");
    let status = if budget_hit {
        CompletenessStatus::PartialBudget
    } else if external {
        CompletenessStatus::PartialExternalBoundary
    } else {
        CompletenessStatus::CompleteSupportedSubset
    };
    let boundaries = if budget_hit {
        vec![json!({"kind":"BUDGET","maxNodes":policy.max_nodes})]
    } else if external {
        vec![json!({"kind":"EXTERNAL_CALL","reason":"maxCallDepth=0"})]
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
        }
    }
    read_set.push(ReadFact {
        kind: "PROJECT_MODEL".into(),
        key: snapshot.project_model_hash.clone(),
        hash: snapshot.project_model_hash.clone(),
    });
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
        external_summaries: vec![],
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
        };
        let enriched = enrich(graph);
        assert!(enriched.nodes.iter().any(|n| n.id == "phi:value"));
        assert!(
            enriched
                .edges
                .iter()
                .any(|e| e.kind == "PHI_INPUT" && e.from == "double")
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
