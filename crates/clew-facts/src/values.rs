//! Language-neutral value equations for a bounded, explicitly supplied model.
//!
//! These are not compiler observations. A bridge must establish SSA definitions,
//! operands and control dependence from its own qualified facts. Parsing this
//! contract authenticates neither that bridge nor the selected program snapshot.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA: &str = "portable-value-model/0.1";
pub const MAX_NODES: usize = 4096;
pub const MAX_DEPENDENCIES: usize = 16384;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueModel {
    pub schema: String,
    pub binding: ModelBinding,
    /// Strictly sorted by id. Values identify definitions, never variable names.
    pub nodes: Vec<ValueNode>,
    /// Unlocalized extraction gaps affect every query in this model.
    pub global_boundaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelBinding {
    pub language: String,
    pub producer: String,
    pub snapshot_digest: String,
    pub facts_digest: String,
    pub configuration_digest: String,
    pub policy_digest: String,
    /// Carried through as a declaration, never upgraded by the solver.
    pub observation_authority: ObservationAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObservationAuthority {
    CompilerObserved,
    SyntaxObserved,
    Declared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueNode {
    pub id: String,
    pub operation: ValueOperation,
    /// Exact evidence references supplied by the bridge; caller verifies them.
    pub evidence_ids: Vec<String>,
    /// Predicate dependencies, not an ordered list of executed conditions.
    pub guards: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum ValueOperation {
    /// Parameter input with unknown nullness. Type-based guarantees need a bridge.
    Input,
    Null,
    NonNull,
    Copy {
        value: String,
    },
    /// SSA merge: all alternatives are feasible unless separately qualified.
    Join {
        values: Vec<String>,
    },
    /// Null coalescing only. Logical OR and truthiness are different operations.
    Coalesce {
        left: String,
        right: String,
    },
    /// Unsupported transfer, alias, reflection, call or runtime configuration.
    Unknown {
        boundary: String,
    },
}

impl ValueOperation {
    pub fn operands(&self) -> Vec<&str> {
        match self {
            Self::Copy { value } => vec![value],
            Self::Join { values } => values.iter().map(String::as_str).collect(),
            Self::Coalesce { left, right } => vec![left, right],
            _ => vec![],
        }
    }
}

fn text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control)
}
fn sorted(values: &[String]) -> bool {
    values.iter().all(|s| text(s)) && values.windows(2).all(|w| w[0] < w[1])
}
fn digest(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
impl ValueModel {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != SCHEMA || self.nodes.is_empty() || self.nodes.len() > MAX_NODES {
            return Err("invalid value model schema or node budget");
        }
        let b = &self.binding;
        if !text(&b.language)
            || !text(&b.producer)
            || ![
                &b.snapshot_digest,
                &b.facts_digest,
                &b.configuration_digest,
                &b.policy_digest,
            ]
            .into_iter()
            .all(|s| digest(s))
            || self.global_boundaries.len() > 64
            || !sorted(&self.global_boundaries)
        {
            return Err("invalid model binding or boundaries");
        }
        if !self.nodes.iter().all(|n| text(&n.id))
            || !self.nodes.windows(2).all(|w| w[0].id < w[1].id)
        {
            return Err("value definitions must be unique and sorted");
        }
        let ids: BTreeSet<_> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        let mut dependencies = 0usize;
        let mut evidence = 0usize;
        for n in &self.nodes {
            if n.evidence_ids.is_empty()
                || n.evidence_ids.len() > MAX_DEPENDENCIES - evidence
                || n.guards.len() > MAX_DEPENDENCIES - dependencies
            {
                return Err("value reference budget exceeded");
            }
            evidence += n.evidence_ids.len();
            dependencies += n.guards.len();
            let operand_count = match &n.operation {
                ValueOperation::Join { values } => values.len(),
                ValueOperation::Copy { .. } => 1,
                ValueOperation::Coalesce { .. } => 2,
                _ => 0,
            };
            if operand_count > MAX_DEPENDENCIES - dependencies {
                return Err("value dependency budget exceeded");
            }
            dependencies += operand_count;
            if !sorted(&n.evidence_ids) || !sorted(&n.guards) {
                return Err("invalid value definition or evidence");
            }
            if let ValueOperation::Join { values } = &n.operation
                && (values.is_empty() || !sorted(values))
            {
                return Err("invalid SSA join");
            }
            if let ValueOperation::Unknown { boundary } = &n.operation
                && !text(boundary)
            {
                return Err("invalid transfer boundary");
            }
            let operands = n.operation.operands();
            if operands
                .iter()
                .copied()
                .chain(n.guards.iter().map(String::as_str))
                .any(|id| !ids.contains(id))
            {
                return Err("dangling value or guard dependency");
            }
        }
        if dependencies > MAX_DEPENDENCIES || evidence > MAX_DEPENDENCIES {
            return Err("value dependency budget exceeded");
        }
        Ok(())
    }
}
