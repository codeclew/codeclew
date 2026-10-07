//! Producer-normalized input for the shared bounded source transfer engine.
//! Syntax preserves evaluation order; compiler bindings identify storage and
//! formal slots independently of spelling, grammar and source argument order.
mod java;
mod kotlin;

use crate::{documentation::model::Source, error::ClewError};
use std::collections::BTreeMap;

pub(super) fn prepare<'a>(
    evidence: &'a crate::documentation::model::ServiceEvidence,
    node: &super::super::model::SourceCallNode,
) -> Result<Prepared<'a>, ClewError> {
    let owner = &evidence.observations[&node.callable.declaration_id];
    if super::super::source::compiler::kotlin_admitted(owner) {
        kotlin::prepare(evidence, node)
    } else {
        java::prepare(evidence, node)
    }
}
#[cfg(test)]
pub(super) use java::site_range as java_site_range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum Kind {
    Variable,
    Field,
    This,
    Super,
    Literal,
    Parenthesized,
    Unary,
    Binary,
    Call,
    Construct,
    Block,
    If,
    Local,
    Declarator,
    Assignment,
    Expression,
    Return,
    Throw,
    NestedBody,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum Role {
    Receiver,
    Operand,
    Operator,
    Left,
    Right,
    Condition,
    Then,
    Else,
    Initializer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VariableKind {
    Parameter,
    Local,
    Field,
}
impl VariableKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Parameter => "PARAMETER",
            Self::Local => "LOCAL_VARIABLE",
            Self::Field => "FIELD",
        }
    }
}

pub(super) struct Variable {
    pub identity: String,
    pub kind: VariableKind,
    pub declaration: bool,
    pub declaration_id: Option<String>,
    pub formal_slot: Option<usize>,
}

/// Actuals remain in source evaluation order. A missing formal slot withholds
/// binding; producers must never recover one by matching the argument's name.
pub(super) struct Actual {
    pub expression: usize,
    pub formal_slot: Option<usize>,
}

/// The producer has admitted one exact source call occurrence. This does not
/// assert runtime dispatch, normal completion or receiver identity.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct BoundCall {
    pub occurrence: String,
    pub target_node: Option<String>,
    pub status: String,
}

pub(super) struct SyntaxNode {
    pub kind: Kind,
    pub range: (usize, usize),
    pub children: Vec<usize>,
    pub roles: BTreeMap<Role, usize>,
    pub actuals: Vec<Actual>,
    /// Formal slots whose default initializers execute after explicit actuals.
    /// Until their transfer is qualified they are an explicit effect frontier.
    pub default_arguments: Vec<usize>,
}

pub(super) struct Input<'a> {
    pub source: &'a Source,
    pub nodes: Vec<SyntaxNode>,
    pub body: Option<usize>,
    pub variables: BTreeMap<(usize, usize), Variable>,
    pub calls: BTreeMap<(usize, usize), BoundCall>,
    pub missing_sites: bool,
}

pub(super) enum Prepared<'a> {
    Body(Input<'a>),
    Unavailable,
    Partial,
    VariablesUnavailable,
}

#[derive(Clone, Copy)]
pub(super) struct Node<'a> {
    input: &'a Input<'a>,
    index: usize,
}
impl<'a> Input<'a> {
    pub(super) fn node(&'a self, index: usize) -> Node<'a> {
        Node { input: self, index }
    }
    pub(super) fn formals(&self) -> Result<BTreeMap<usize, String>, ClewError> {
        let mut formals = BTreeMap::new();
        for variable in self.variables.values().filter(|v| {
            v.declaration && v.kind == VariableKind::Parameter && v.formal_slot.is_some()
        }) {
            let slot = variable.formal_slot.unwrap();
            if formals.insert(slot, variable.identity.clone()).is_some() {
                return Err(crate::documentation::invalid(
                    "data-state formal slot is ambiguous",
                ));
            }
        }
        Ok(formals)
    }
}
impl<'a> Node<'a> {
    pub(super) fn kind(self) -> Kind {
        self.input.nodes[self.index].kind
    }
    pub(super) fn range(self) -> (usize, usize) {
        self.input.nodes[self.index].range
    }
    pub(super) fn text(self) -> String {
        let (start, end) = self.range();
        self.input.source.text[start..end].to_owned()
    }
    pub(super) fn child(self, role: Role) -> Option<Self> {
        self.input.nodes[self.index]
            .roles
            .get(&role)
            .map(|&index| self.input.node(index))
    }
    pub(super) fn children(self) -> Vec<Self> {
        self.input.nodes[self.index]
            .children
            .iter()
            .map(|&index| self.input.node(index))
            .collect()
    }
    pub(super) fn default_arguments(self) -> &'a [usize] {
        &self.input.nodes[self.index].default_arguments
    }
    pub(super) fn actuals(self) -> Vec<(Self, Option<usize>)> {
        self.input.nodes[self.index]
            .actuals
            .iter()
            .map(|a| (self.input.node(a.expression), a.formal_slot))
            .collect()
    }
}
