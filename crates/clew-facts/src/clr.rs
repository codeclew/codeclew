//! Portable, bounded CLR attribute observations from a C# compiler.
//!
//! The record describes one ordinary method: its own attributes, its containing
//! type's attributes (including those inherited from base types whose attribute
//! usage allows inheritance), its base types and attributes of overridden base
//! methods. Attribute types carry their base classes and interfaces so framework
//! packages can recognize derived attributes without compiler access.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CLR_ATTRIBUTE_SCHEMA: &str = "clr-attribute-facts/1.0";
pub const CLR_ATTRIBUTE_AUTHORITY: &str = "ROSLYN_RESOLVED_ATTRIBUTES";
pub const CLR_ATTRIBUTE_SCOPE: &str = "METHOD_TYPE_AND_INHERITED_ATTRIBUTES";
const MAX_ATTRIBUTES: usize = 512;
const MAX_VALUE_DEPTH: usize = 9;
const MAX_ARRAY_VALUES: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClrAttributeFacts {
    pub schema: String,
    pub authority: String,
    pub declaration: String,
    pub method: ClrMethod,
    pub containing_type: ClrType,
    pub overridden_attributes: Vec<ClrAttributeUse>,
    pub boundaries: Vec<String>,
    pub coverage: crate::Coverage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClrMethod {
    pub name: String,
    pub accessibility: String,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_generic: bool,
    pub is_override: bool,
    pub attributes: Vec<ClrAttributeUse>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClrType {
    pub identity: String,
    pub name: String,
    pub accessibility: String,
    pub is_abstract: bool,
    pub is_generic: bool,
    pub is_nested: bool,
    pub attributes: Vec<ClrAttributeUse>,
    pub base_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClrAttributeUse {
    pub attribute_type: String,
    pub attribute_bases: Vec<String>,
    pub attribute_interfaces: Vec<String>,
    pub constructor_arguments: Vec<ClrValue>,
    pub named_arguments: BTreeMap<String, ClrValue>,
    #[serde(default)]
    pub constructor_parameters: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_from: Option<String>,
}

impl ClrAttributeUse {
    /// The attribute type is the identity or derives from it.
    pub fn is_a(&self, identity: &str) -> bool {
        self.attribute_type == identity || self.attribute_bases.iter().any(|base| base == identity)
    }

    pub fn implements(&self, identity: &str) -> bool {
        self.attribute_interfaces
            .iter()
            .any(|value| value == identity)
    }

    /// Constructor argument bound to a parameter name, when the constructor is known.
    pub fn argument(&self, parameter: &str) -> Option<&ClrValue> {
        self.constructor_parameters
            .iter()
            .position(|name| name == parameter)
            .and_then(|index| self.constructor_arguments.get(index))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum ClrValue {
    Primitive {
        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        value_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    Enum {
        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        value_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member: Option<String>,
    },
    Type {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<String>,
    },
    Array {
        values: Vec<ClrValue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
    },
    Null,
    Unresolved {
        reason: String,
    },
}

impl ClrValue {
    /// A string constant, if this value is one.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::Primitive {
                value_type: Some(kind),
                value: Some(value),
            } if kind == "string" => Some(value),
            _ => None,
        }
    }

    /// String constants of a value or of a complete array of values.
    pub fn strings(&self) -> Option<Vec<String>> {
        match self {
            Self::Array {
                values,
                truncated: None | Some(false),
            } => values
                .iter()
                .map(|value| value.as_string().map(str::to_owned))
                .collect(),
            other => other.as_string().map(|value| vec![value.to_owned()]),
        }
    }

    fn depth(&self) -> usize {
        match self {
            Self::Array { values, .. } => 1 + values.iter().map(Self::depth).max().unwrap_or(0),
            _ => 1,
        }
    }

    fn bounded(&self) -> bool {
        match self {
            Self::Array { values, .. } => {
                values.len() <= MAX_ARRAY_VALUES && values.iter().all(Self::bounded)
            }
            _ => true,
        }
    }
}

impl ClrAttributeFacts {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != CLR_ATTRIBUTE_SCHEMA
            || self.authority != CLR_ATTRIBUTE_AUTHORITY
            || !self.declaration.starts_with("method:class:")
            || !self.containing_type.identity.starts_with("class:")
            || self.coverage.scope != CLR_ATTRIBUTE_SCOPE
            || !matches!(self.coverage.status.as_str(), "COMPLETE" | "PARTIAL")
            || (self.coverage.status == "COMPLETE") != self.boundaries.is_empty()
            || self.boundaries.len() > 64
        {
            return Err("CLR attribute facts authority is invalid");
        }
        let uses = self
            .method
            .attributes
            .iter()
            .chain(&self.containing_type.attributes)
            .chain(&self.overridden_attributes)
            .collect::<Vec<_>>();
        if uses.len() > MAX_ATTRIBUTES
            || uses.iter().any(|use_| {
                !use_.attribute_type.starts_with("class:")
                    || use_.constructor_arguments.len() > 64
                    || use_.named_arguments.len() > 64
                    || use_
                        .constructor_arguments
                        .iter()
                        .chain(use_.named_arguments.values())
                        .any(|value| value.depth() > MAX_VALUE_DEPTH || !value.bounded())
            })
        {
            return Err("CLR attribute facts exceed their bounds");
        }
        Ok(())
    }
}
