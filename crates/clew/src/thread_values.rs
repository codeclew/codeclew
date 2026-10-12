//! Deterministic backward relevance and nullness over portable value equations.
//!
//! This is a conditional model calculation, not a compiler-fact verifier, source
//! extractor, path-feasibility solver or proof of program-wide absence of errors.
use clew_facts::values::{ModelBinding, ValueModel, ValueOperation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const RESULT_SCHEMA: &str = "codeclew-value-query/0.1";
pub const POLICY: &str = "portable-nullness-equations/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueQuery {
    pub target: String,
    pub max_nodes: usize,
    pub max_transfers: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Nullness {
    Null,
    NonNull,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueObservation {
    pub id: String,
    pub normal_value_nullness: Nullness,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueFrontier {
    pub kind: FrontierKind,
    pub subject: String,
    pub code: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FrontierKind {
    Discovery,
    Semantic,
    Budget,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueResult {
    pub schema: String,
    pub policy: String,
    pub calculation_authority: String,
    pub model_digest: String,
    pub binding: ModelBinding,
    pub query: ValueQuery,
    /// Always false: this profile does not verify extraction or program closure.
    pub program_semantic_closure: bool,
    /// Completion of selected equations, independently of semantic frontiers.
    pub equations_converged: bool,
    pub observations: Vec<ValueObservation>,
    pub frontier: Vec<ValueFrontier>,
    pub transfer_count: usize,
}

// Powerset {null, non-null}; bottom is internal only. Public bottom is unknown.
const NULL: u8 = 1;
const NON_NULL: u8 = 2;
const TOP: u8 = NULL | NON_NULL;
fn public_value(v: u8) -> Nullness {
    match v {
        NULL => Nullness::Null,
        NON_NULL => Nullness::NonNull,
        _ => Nullness::Unknown,
    }
}
fn transfer(op: &ValueOperation, state: &BTreeMap<String, u8>) -> u8 {
    // A budget-excluded operand is unknown, never bottom or irrelevant.
    let get = |id: &str| state.get(id).copied().unwrap_or(TOP);
    match op {
        ValueOperation::Null => NULL,
        ValueOperation::NonNull => NON_NULL,
        ValueOperation::Copy { value } => get(value),
        ValueOperation::Join { values } => values.iter().fold(0, |v, id| v | get(id)),
        ValueOperation::Coalesce { left, right } => {
            let l = get(left);
            (l & NON_NULL) | if l & NULL != 0 { get(right) } else { 0 }
        }
        ValueOperation::Input | ValueOperation::Unknown { .. } => TOP,
    }
}

/// Calculate over a supplied model. The binding and evidence are declarations;
/// callers must independently authenticate retained inputs before publication.
/// No language, producer or authority label bypasses this requirement.
pub fn analyze(model: &ValueModel, query: &ValueQuery) -> Result<ValueResult, &'static str> {
    model.validate()?;
    if query.target.is_empty()
        || query.target.len() > 1024
        || query.target.chars().any(char::is_control)
        || query.max_nodes == 0
        || query.max_nodes > clew_facts::values::MAX_NODES
        || query.max_transfers == 0
        || query.max_transfers > 1_000_000
    {
        return Err("invalid value query or budget");
    }
    let nodes: BTreeMap<_, _> = model.nodes.iter().map(|n| (n.id.clone(), n)).collect();
    let mut frontier = BTreeSet::new();
    frontier.insert(ValueFrontier {
        kind: FrontierKind::Semantic,
        subject: query.target.clone(),
        code: "MODEL_BINDING_AND_EXTRACTION_NOT_VERIFIED_BY_SOLVER".into(),
    });
    for code in &model.global_boundaries {
        frontier.insert(ValueFrontier {
            kind: if code == "VALUE_CALL_PROGRAM_BUDGET" {
                FrontierKind::Budget
            } else {
                FrontierKind::Semantic
            },
            subject: query.target.clone(),
            code: code.clone(),
        });
    }
    let mut selected = BTreeSet::new();
    if nodes.contains_key(&query.target) {
        // Root first; stable lexical tie-break for subsequent pending nodes.
        let mut pending = BTreeSet::from([query.target.clone()]);
        while let Some(id) = pending.pop_first() {
            if selected.contains(&id) {
                continue;
            }
            if selected.len() == query.max_nodes {
                frontier.insert(ValueFrontier {
                    kind: FrontierKind::Budget,
                    subject: id,
                    code: "RELEVANCE_NODE_BUDGET".into(),
                });
                continue;
            }
            selected.insert(id.clone());
            let node = nodes[&id];
            pending.extend(node.operation.operands().into_iter().map(str::to_owned));
            pending.extend(node.guards.iter().cloned());
        }
    } else {
        frontier.insert(ValueFrontier {
            kind: FrontierKind::Discovery,
            subject: query.target.clone(),
            code: "VALUE_NOT_FOUND_UNKNOWN".into(),
        });
    }
    let mut state: BTreeMap<_, _> = selected.iter().map(|id| (id.clone(), 0u8)).collect();
    let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for id in &selected {
        for operand in nodes[id].operation.operands() {
            users.entry(operand.into()).or_default().insert(id.clone());
        }
        if let ValueOperation::Unknown { boundary } = &nodes[id].operation {
            frontier.insert(ValueFrontier {
                kind: if matches!(
                    boundary.as_str(),
                    "VALUE_CALL_NODE_BUDGET" | "VALUE_CALL_CONTEXT_BUDGET"
                ) {
                    FrontierKind::Budget
                } else {
                    FrontierKind::Semantic
                },
                subject: id.clone(),
                code: boundary.clone(),
            });
        }
    }
    let mut pending = selected.clone();
    let mut transfer_count = 0;
    let mut seeded_bottom = false;
    let mut exhausted = false;
    let equations_converged = loop {
        while let Some(id) = pending.pop_first() {
            if transfer_count == query.max_transfers {
                exhausted = true;
                frontier.insert(ValueFrontier {
                    kind: FrontierKind::Budget,
                    subject: query.target.clone(),
                    code: "FIXPOINT_TRANSFER_BUDGET".into(),
                });
                // Partial fixpoint may underapproximate. Never expose its bits.
                state.values_mut().for_each(|v| *v = TOP);
                break;
            }
            transfer_count += 1;
            let next = state[&id] | transfer(&nodes[&id].operation, &state);
            if next != state[&id] {
                state.insert(id.clone(), next);
                if let Some(dependents) = users.get(&id) {
                    pending.extend(dependents.iter().cloned());
                }
            }
        }
        if exhausted {
            break false;
        }
        if seeded_bottom {
            break true;
        }
        seeded_bottom = true;
        // Unseeded cycles or no modeled normal value cannot establish nullness.
        for (id, value) in &mut state {
            if *value == 0 {
                *value = TOP;
                frontier.insert(ValueFrontier {
                    kind: FrontierKind::Semantic,
                    subject: id.clone(),
                    code: "NO_MODELED_NORMAL_VALUE_UNKNOWN".into(),
                });
                if let Some(dependents) = users.get(id) {
                    pending.extend(dependents.iter().cloned());
                }
            }
        }
    };
    Ok(ValueResult {
        schema: RESULT_SCHEMA.into(),
        policy: POLICY.into(),
        calculation_authority: "DECLARED_MODEL_EQUATIONS".into(),
        model_digest: clew_facts::digest(model).map_err(|_| "value model serialization failed")?,
        binding: model.binding.clone(),
        query: query.clone(),
        program_semantic_closure: false,
        equations_converged,
        observations: state
            .into_iter()
            .map(|(id, v)| ValueObservation {
                evidence_ids: nodes[&id].evidence_ids.clone(),
                id,
                normal_value_nullness: public_value(v),
            })
            .collect(),
        frontier: frontier.into_iter().collect(),
        transfer_count,
    })
}
