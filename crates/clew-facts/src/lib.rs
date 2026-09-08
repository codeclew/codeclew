//! Portable, bounded JVM observations. No framework or compiler API dependencies.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const JVM_ANNOTATION_SCHEMA: &str = "jvm-annotation-facts/1.0";
pub const ANNOTATION_SCOPE: &str = "REACHABLE_ANNOTATIONS_AND_HIERARCHY";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmAnnotationFacts {
    pub schema: String,
    pub authority: String,
    pub declaration: String,
    pub definitions: BTreeMap<String, AnnotationDefinition>,
    pub types: Vec<AnnotatedType>,
    pub callables: Vec<AnnotatedCallable>,
    pub boundaries: Vec<String>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Coverage {
    pub status: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Origin {
    pub kind: String,
    pub identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationUse {
    pub type_name: String,
    pub arguments: BTreeMap<String, AnnotationValue>,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_site_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum AnnotationValue {
    Constant { value: Value },
    Enum { r#type: String, value: String },
    Class { value: String },
    Array { values: Vec<AnnotationValue> },
    Annotation { value: Box<AnnotationUse> },
    Unresolved { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationDefinition {
    pub origin: Origin,
    pub annotations: Vec<AnnotationUse>,
    pub members: BTreeMap<String, AnnotationMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationMember {
    pub annotations: Vec<AnnotationUse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<AnnotationValue>,
    pub return_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotatedType {
    pub identity: String,
    pub annotations: Vec<AnnotationUse>,
    pub direct_supertypes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotatedMethod {
    pub identity: String,
    pub annotations: Vec<AnnotationUse>,
    pub overrides: Vec<AnnotatedMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotatedCallable {
    pub method: AnnotatedMethod,
    pub classes: Vec<AnnotatedType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bean_class: Option<String>,
    pub abstract_method: bool,
    pub inherited: bool,
    pub implementation_source: bool,
}

pub fn digest<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    // Struct field order and BTreeMap keys are deterministic. This contract's
    // digest is versioned separately from envelopes owned by other consumers.
    Ok(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(serde_json::to_vec(value)?))
    ))
}

impl JvmAnnotationFacts {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != JVM_ANNOTATION_SCHEMA
            || !matches!(
                self.authority.as_str(),
                "K2_RESOLVED_ANNOTATIONS" | "JAVAC_RESOLVED_ANNOTATIONS"
            )
            || self.declaration.is_empty()
            || self.declaration.len() > 8192
            || self.definitions.len() > 2048
            || self.callables.len() > 4096
            || self.types.len() > 128
            || self.boundaries.len() > 256
            || !matches!(self.coverage.status.as_str(), "COMPLETE" | "PARTIAL")
            || self.coverage.scope != ANNOTATION_SCOPE
        {
            return Err("JVM annotation fact schema, authority, coverage or size is invalid");
        }
        if self.coverage.status == "COMPLETE" && !self.boundaries.is_empty() {
            return Err("complete JVM annotation coverage cannot hide extraction boundaries");
        }
        let mut budget = 32768usize;
        for class in &self.types {
            if class.identity.is_empty() || class.direct_supertypes.len() > 128 {
                return Err("JVM type identity or hierarchy is invalid");
            }
            uses(&class.annotations, 0, &mut budget)?;
        }
        for (name, definition) in &self.definitions {
            if name.is_empty() || name.len() > 1024 || definition.members.len() > 512 {
                return Err("JVM annotation definition is invalid or exceeds its bound");
            }
            origin(&definition.origin)?;
            uses(&definition.annotations, 0, &mut budget)?;
            for member in definition.members.values() {
                uses(&member.annotations, 0, &mut budget)?;
                if let Some(value) = &member.default_value {
                    value.validate(0, &mut budget)?;
                }
            }
        }
        for callable in &self.callables {
            if callable.classes.len() > 128 {
                return Err("JVM class hierarchy exceeds its bound");
            }
            method(&callable.method, 0, &mut budget)?;
            for class in &callable.classes {
                if class.identity.is_empty() || class.direct_supertypes.len() > 128 {
                    return Err("JVM type identity or hierarchy is invalid");
                }
                uses(&class.annotations, 0, &mut budget)?;
            }
        }
        Ok(())
    }
}

fn spend(budget: &mut usize) -> Result<(), &'static str> {
    *budget = budget
        .checked_sub(1)
        .ok_or("JVM annotation record budget exceeded")?;
    Ok(())
}
fn origin(value: &Origin) -> Result<(), &'static str> {
    if !matches!(value.kind.as_str(), "SOURCE" | "BINARY" | "UNKNOWN")
        || value.identity.is_empty()
        || value.identity.len() > 8192
        || value
            .start
            .zip(value.end)
            .is_some_and(|(start, end)| start > end)
        || value.start.is_some() != value.end.is_some()
    {
        return Err("JVM annotation origin is invalid");
    }
    Ok(())
}
fn uses(values: &[AnnotationUse], depth: usize, budget: &mut usize) -> Result<(), &'static str> {
    for value in values {
        spend(budget)?;
        if depth > 32
            || value.type_name.is_empty()
            || value.type_name.len() > 1024
            || value.arguments.len() > 512
        {
            return Err("JVM annotation use exceeds its bound");
        }
        origin(&value.origin)?;
        for argument in value.arguments.values() {
            argument.validate(depth + 1, budget)?;
        }
    }
    Ok(())
}
fn method(value: &AnnotatedMethod, depth: usize, budget: &mut usize) -> Result<(), &'static str> {
    spend(budget)?;
    if depth > 128 || value.identity.is_empty() || value.overrides.len() > 128 {
        return Err("JVM override hierarchy is invalid or exceeds its bound");
    }
    uses(&value.annotations, 0, budget)?;
    for base in &value.overrides {
        method(base, depth + 1, budget)?;
    }
    Ok(())
}
impl AnnotationValue {
    fn validate(&self, depth: usize, budget: &mut usize) -> Result<(), &'static str> {
        spend(budget)?;
        if depth > 32 {
            return Err("JVM annotation value nesting exceeds its bound");
        }
        match self {
            Self::Constant { value } if value.is_array() || value.is_object() => {
                return Err("JVM annotation constant must be scalar");
            }
            Self::Array { values } => {
                for value in values {
                    value.validate(depth + 1, budget)?;
                }
            }
            Self::Annotation { value } => uses(std::slice::from_ref(value), depth + 1, budget)?,
            _ => (),
        }
        Ok(())
    }
}
