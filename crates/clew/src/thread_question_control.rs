//! Local graph observations, deliberately distinct from feasible execution order.

use crate::error::{ClewError, ErrorCode};
use crate::thread_flow_cfg::{self, LocalCfgEdgeKind, LocalCfgNodeRole, LocalCfgPayload};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderQuery {
    pub id: String,
    pub region_id: String,
    pub before: u64,
    pub after: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderObservation {
    pub query_id: String,
    pub region_id: String,
    /// Includes exceptional and loop edges, but excludes explicit DEAD topology.
    pub may_reach_in_active_cfg: Option<bool>,
    /// Every active entry-to-after graph path visits before. No feasibility claim.
    pub strictly_dominates_in_active_cfg: Option<bool>,
    pub unknown_reason: Option<String>,
}

pub fn observe(
    graph: Option<&LocalCfgPayload>,
    query: &OrderQuery,
) -> Result<OrderObservation, ClewError> {
    let mut result = OrderObservation {
        query_id: query.id.clone(),
        region_id: query.region_id.clone(),
        may_reach_in_active_cfg: None,
        strictly_dominates_in_active_cfg: None,
        unknown_reason: None,
    };
    let Some(graph) = graph else {
        result.unknown_reason = Some("LOCAL_CFG_REGION_NOT_FOUND".into());
        return Ok(result);
    };
    thread_flow_cfg::validate(graph)?;
    let active = graph
        .nodes
        .iter()
        .filter(|n| n.role != LocalCfgNodeRole::Dead)
        .map(|n| n.node_id)
        .collect::<BTreeSet<_>>();
    if !active.contains(&query.before) || !active.contains(&query.after) {
        result.unknown_reason = Some("LOCAL_CFG_NODE_MISSING_OR_DEAD".into());
        return Ok(result);
    }
    let entry = graph
        .nodes
        .iter()
        .find(|n| n.role == LocalCfgNodeRole::Entry)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "local order requires an entry node",
            )
        })?
        .node_id;
    let mut successors = BTreeMap::<u64, Vec<u64>>::new();
    for edge in &graph.edges {
        if edge.kind != LocalCfgEdgeKind::Dead
            && active.contains(&edge.source_node_id)
            && active.contains(&edge.target_node_id)
        {
            successors
                .entry(edge.source_node_id)
                .or_default()
                .push(edge.target_node_id);
        }
    }
    if !reaches(&successors, entry, query.before, None)
        || !reaches(&successors, entry, query.after, None)
    {
        result.unknown_reason = Some("LOCAL_CFG_NODE_NOT_ACTIVE_REACHABLE".into());
        return Ok(result);
    }
    result.may_reach_in_active_cfg = Some(reaches(&successors, query.before, query.after, None));
    result.strictly_dominates_in_active_cfg = Some(
        query.before != query.after
            && !reaches(&successors, entry, query.after, Some(query.before)),
    );
    Ok(result)
}

fn reaches(
    successors: &BTreeMap<u64, Vec<u64>>,
    start: u64,
    target: u64,
    blocked: Option<u64>,
) -> bool {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([start]);
    while let Some(node) = queue.pop_front() {
        if Some(node) == blocked || !seen.insert(node) {
            continue;
        }
        if node == target {
            return true;
        }
        if let Some(next) = successors.get(&node) {
            queue.extend(next);
        }
    }
    false
}
