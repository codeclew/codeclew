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

/// Syntax evidence has its own additive contract. It cannot carry compiler
/// relationships, binary definitions, or a resolved-annotation authority.
pub const SOURCE_ANNOTATION_SCHEMA: &str = "source-annotation-facts/1.0";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceAnnotationFacts {
    pub schema: String,
    pub authority: String,
    pub language: String,
    pub declaration: SourceAnnotatedElement,
    pub owners: Vec<SourceAnnotatedElement>,
    pub imports: Vec<String>,
    pub boundaries: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceAnnotatedElement {
    pub identity: String,
    pub annotations: Vec<SourceAnnotationUse>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceAnnotationUse {
    pub spelling: String,
    pub qualified_name: Option<String>,
    pub qualification: String,
    pub arguments: BTreeMap<String, AnnotationValue>,
    pub origin: Origin,
}
impl SourceAnnotationFacts {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != SOURCE_ANNOTATION_SCHEMA
            || self.authority != "SOURCE_ANNOTATIONS"
            || !matches!(self.language.as_str(), "java" | "kotlin")
            || self.owners.len() > 128
            || self.imports.len() > 2048
            || self.boundaries.len() > 256
            || self
                .imports
                .iter()
                .chain(&self.boundaries)
                .any(|s| s.is_empty() || s.len() > 2048)
        {
            return Err("invalid source annotation identity or bounds");
        }
        let mut budget = 32768;
        for element in std::iter::once(&self.declaration).chain(&self.owners) {
            if element.identity.is_empty()
                || element.identity.len() > 8192
                || element.annotations.len() > 128
            {
                return Err("invalid source declaration or annotation bounds");
            }
            for annotation in &element.annotations {
                spend(&mut budget)?;
                if annotation.spelling.is_empty()
                    || annotation.spelling.len() > 1024
                    || annotation.origin.kind != "SOURCE"
                    || annotation.arguments.len() > 512
                {
                    return Err("invalid source annotation origin or size");
                }
                origin(&annotation.origin)?;
                match (
                    annotation.qualification.as_str(),
                    &annotation.qualified_name,
                ) {
                    ("FULLY_QUALIFIED", Some(name))
                        if name.len() <= 1024
                            && name.contains('.')
                            && name == &annotation.spelling => {}
                    ("EXPLICIT_IMPORT", Some(name))
                        if name.len() <= 1024
                            && self.imports.iter().any(|import| {
                                source_import_matches(import, &annotation.spelling, name)
                            }) => {}
                    ("UNRESOLVED", None) => {}
                    _ => {
                        return Err(
                            "source annotation name qualification does not match its evidence",
                        );
                    }
                }
                for value in annotation.arguments.values() {
                    value.validate(0, &mut budget)?;
                    if !source_value(value) {
                        return Err(
                            "source annotations cannot carry compiler-exact class or nested annotation identities",
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
fn source_import_matches(import: &str, spelling: &str, qualified: &str) -> bool {
    let parts: Vec<_> = import.split_whitespace().collect();
    if parts.first() != Some(&"import") || parts.iter().any(|s| *s == "static" || *s == "*") {
        return false;
    }
    let alias = parts.iter().position(|p| *p == "as");
    let end = alias.unwrap_or(parts.len());
    let name = parts[1..end].join("").trim_end_matches(';').to_owned();
    let local = alias
        .and_then(|i| parts.get(i + 1).copied())
        .unwrap_or_else(|| name.rsplit('.').next().unwrap_or(""));
    name == qualified && local == spelling
}
fn source_value(value: &AnnotationValue) -> bool {
    match value {
        AnnotationValue::Class { .. } | AnnotationValue::Annotation { .. } => false,
        AnnotationValue::Array { values } => values.iter().all(source_value),
        _ => true,
    }
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

#[cfg(test)]
mod source_contract_tests {
    use super::*;
    use serde_json::json;
    fn record() -> serde_json::Value {
        json!({"schema":SOURCE_ANNOTATION_SCHEMA,"authority":"SOURCE_ANNOTATIONS","language":"java","declaration":{"identity":"source:Orders/reserve","annotations":[{"spelling":"PostMapping","qualifiedName":"org.springframework.web.bind.annotation.PostMapping","qualification":"EXPLICIT_IMPORT","arguments":{},"origin":{"kind":"SOURCE","identity":"Orders.java"}}]},"owners":[],"imports":["import org.springframework.web.bind.annotation.PostMapping"],"boundaries":[]})
    }
    #[test]
    fn source_contract_cannot_claim_compiler_authority_or_relationships() {
        let original = record();
        let valid: SourceAnnotationFacts = serde_json::from_value(original.clone()).unwrap();
        valid.validate().unwrap();
        let mut forged = original.clone();
        forged["authority"] = json!("JAVAC_RESOLVED_ANNOTATIONS");
        assert!(
            serde_json::from_value::<SourceAnnotationFacts>(forged)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut forged = original.clone();
        forged["declaration"]["overrides"] = json!(["compiler:base"]);
        assert!(serde_json::from_value::<SourceAnnotationFacts>(forged).is_err());
        let mut forged = original;
        forged["declaration"]["annotations"][0]["origin"]["kind"] = json!("BINARY");
        assert!(
            serde_json::from_value::<SourceAnnotationFacts>(forged)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    #[test]
    fn source_qualification_cannot_hide_unresolved_names_or_nested_compiler_values() {
        let mut unresolved = record();
        unresolved["declaration"]["annotations"][0]["qualification"] = json!("UNRESOLVED");
        assert!(
            serde_json::from_value::<SourceAnnotationFacts>(unresolved.clone())
                .unwrap()
                .validate()
                .is_err()
        );
        unresolved["declaration"]["annotations"][0]["qualifiedName"] = serde_json::Value::Null;
        serde_json::from_value::<SourceAnnotationFacts>(unresolved.clone())
            .unwrap()
            .validate()
            .unwrap();
        unresolved["declaration"]["annotations"][0]["arguments"]["type"] =
            json!({"kind":"CLASS","value":"compiler:exact"});
        assert!(
            serde_json::from_value::<SourceAnnotationFacts>(unresolved)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}
