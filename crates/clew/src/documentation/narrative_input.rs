//! Safe diagnostics for wholly rejected narrative files. Serializer messages can
//! contain submitted values or snippets, so none enter the public response.
use super::{MAX_RECORD, read_bytes};
use crate::{
    documentation::model::{Narrative, Operation},
    error::ClewError,
};
use serde::{
    Serialize,
    de::{self, DeserializeOwned, Visitor},
};
use serde_json::{Value, json};
use serde_yaml_ng::Value as Yaml;
use std::path::Path;

const MAX_DIAGNOSTICS: usize = 16;

#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum DiagnosticKind {
    UnknownField,
    InvalidSchema,
    InvalidSyntax,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostic {
    kind: DiagnosticKind,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    field_redacted: bool,
}

#[derive(Debug)]
pub struct NarrativeInputError {
    error: Box<ClewError>,
    diagnostics: Vec<Diagnostic>,
    truncated: bool,
}

impl NarrativeInputError {
    fn new(error: ClewError) -> Self {
        Self {
            error: Box::new(error),
            diagnostics: Vec::new(),
            truncated: false,
        }
    }

    fn push(&mut self, diagnostic: Diagnostic) {
        if self.diagnostics.len() == MAX_DIAGNOSTICS {
            self.truncated = true;
        } else {
            self.diagnostics.push(diagnostic);
        }
    }

    pub fn update_failure(&self) -> Value {
        let detail = self
            .diagnostics
            .first()
            .map(|diagnostic| match diagnostic.kind {
                DiagnosticKind::UnknownField => format!(
                    " Remove unknown field {} at {} and resubmit the complete narrative.",
                    diagnostic.field.as_deref().unwrap_or("[redacted]"),
                    diagnostic.path
                ),
                DiagnosticKind::InvalidSchema => format!(
                    " Correct the closed schema at {} and resubmit the complete narrative.",
                    diagnostic.path
                ),
                DiagnosticKind::InvalidSyntax => {
                    " Correct JSON/YAML syntax and resubmit the complete narrative.".into()
                }
            })
            .unwrap_or_else(|| format!(" {}.", self.error.message.trim_end_matches('.')));
        json!({
            "reason":self.error.code,
            "nextAction":format!("Incoming narrative rejected in full; no part was applied.{detail}"),
            "rejectionScope":"ENTIRE_NARRATIVE", "applied":false,
            "diagnostics":self.diagnostics, "diagnosticsTruncated":self.truncated,
        })
    }
}

pub fn read_narrative(path: &Path) -> Result<Narrative, NarrativeInputError> {
    let data = read_bytes(path, MAX_RECORD).map_err(NarrativeInputError::new)?;
    match serde_yaml_ng::from_slice::<Narrative>(&data) {
        Ok(narrative) => Ok(narrative),
        Err(_) => {
            let mut failure = NarrativeInputError::new(crate::documentation::invalid(
                "documentation input violates its closed JSON/YAML schema",
            ));
            match serde_yaml_ng::from_slice::<Yaml>(&data) {
                Ok(value) => diagnose(&value, &mut failure),
                Err(_) => failure.push(Diagnostic {
                    kind: DiagnosticKind::InvalidSyntax,
                    path: "$".into(),
                    field: None,
                    field_redacted: false,
                }),
            }
            Err(failure)
        }
    }
}

fn safe_field(field: &str) -> bool {
    !field.is_empty()
        && field.len() <= 64
        && field.as_bytes()[0].is_ascii_alphabetic()
        && field
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn unknown_fields(
    value: &Yaml,
    fields: &[&str],
    path: &str,
    failure: &mut NarrativeInputError,
) -> bool {
    let Some(mapping) = value.as_mapping() else {
        return false;
    };
    let mut unknown = false;
    for key in mapping.keys() {
        if key.as_str().is_some_and(|field| fields.contains(&field)) {
            continue;
        }
        unknown = true;
        let field = key
            .as_str()
            .filter(|field| safe_field(field))
            .map(str::to_owned);
        failure.push(Diagnostic {
            kind: DiagnosticKind::UnknownField,
            path: path.into(),
            field_redacted: field.is_none(),
            field,
        });
        if failure.truncated {
            break;
        }
    }
    unknown
}

fn diagnose(value: &Yaml, failure: &mut NarrativeInputError) {
    unknown_fields(value, schema_fields::<Narrative>(), "$", failure);
    if let Some(operations) = value.get("operations").and_then(Yaml::as_sequence) {
        let fields = schema_fields::<Operation>();
        for (index, operation) in operations.iter().enumerate() {
            let path = format!("operations[{index}]");
            if !unknown_fields(operation, fields, &path, failure)
                && serde_yaml_ng::from_value::<Operation>(operation.clone()).is_err()
            {
                failure.push(Diagnostic {
                    kind: DiagnosticKind::InvalidSchema,
                    path,
                    field: None,
                    field_redacted: false,
                });
            }
            if failure.truncated {
                break;
            }
        }
    }
    if failure.diagnostics.is_empty() {
        failure.push(Diagnostic {
            kind: DiagnosticKind::InvalidSchema,
            path: "$".into(),
            field: None,
            field_redacted: false,
        });
    }
}

// Ask the derived deserializer for its accepted field names. This keeps the
// diagnostics aligned with the closed schema without parsing error prose or
// maintaining a second list of model fields.
fn schema_fields<T: DeserializeOwned>() -> &'static [&'static str] {
    let mut fields = &[][..];
    let _ = T::deserialize(SchemaFields(&mut fields));
    fields
}

struct SchemaFields<'a>(&'a mut &'static [&'static str]);
impl<'de> de::Deserializer<'de> for SchemaFields<'_> {
    type Error = de::value::Error;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("schema inspection only"))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Self::Error> {
        *self.0 = fields;
        Err(de::Error::custom("schema inspection only"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map enum identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn narrative() -> Value {
        json!({
            "schema":"codeclew-documentation-narrative/1.3", "subject":"service:fixture",
            "contextDigest":"synthetic-context", "operations":[
                {"id":"first", "title":"First", "summary":{"id":"summary","text":"Synthetic text",
                    "dependencyIds":[],"sourceIds":[]},"participants":[],"events":[]},
                {"id":"second", "title":"Second", "summary":{"id":"summary","text":"Synthetic text",
                    "dependencyIds":[],"sourceIds":[]},"participants":[],"events":[]}
            ], "gaps":{}
        })
    }

    fn reject(value: &Value) -> Value {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("input.json");
        std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        read_narrative(&path).unwrap_err().update_failure()
    }

    #[test]
    fn narrative_input_unknown_fields_are_safe_and_operation_localized() {
        let mut value = narrative();
        value["operationId"] = json!("TOP_LEVEL_PRIVATE_VALUE");
        value["operations"][1]["interaction"] = json!("OPERATION_PRIVATE_VALUE");
        value["operations"][1]["id"] = json!("/private/synthetic-owner/PRIVATE_OPERATION_ID");
        let failure = reject(&value);
        assert_eq!(failure["reason"], "INVALID_INPUT");
        assert_eq!(failure["rejectionScope"], "ENTIRE_NARRATIVE");
        assert_eq!(failure["applied"], false);
        let diagnostics = failure["diagnostics"].as_array().unwrap();
        assert!(diagnostics.iter().any(|d| d["kind"] == "UNKNOWN_FIELD"
            && d["field"] == "operationId"
            && d["path"] == "$"));
        assert!(diagnostics.iter().any(|d| d["kind"] == "UNKNOWN_FIELD"
            && d["field"] == "interaction"
            && d["path"] == "operations[1]"));
        let encoded = failure.to_string();
        for private in [
            "TOP_LEVEL_PRIVATE_VALUE",
            "OPERATION_PRIVATE_VALUE",
            "PRIVATE_OPERATION_ID",
            "/private/",
        ] {
            assert!(!encoded.contains(private));
        }
    }

    #[test]
    fn narrative_input_redacts_unsafe_field_names_and_bounds_diagnostics() {
        let mut value = narrative();
        for key in [
            "/private/synthetic-owner/private-key",
            "owner@example.invalid",
            "unsafe\nkey",
        ] {
            value[key] = json!("PRIVATE_UNKNOWN_VALUE");
        }
        let failure = reject(&value);
        assert!(
            failure["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| { d["fieldRedacted"] == true && d.get("field").is_none() })
        );
        let encoded = failure.to_string();
        for private in [
            "synthetic-owner",
            "owner@example.invalid",
            "unsafe",
            "PRIVATE_UNKNOWN_VALUE",
        ] {
            assert!(!encoded.contains(private));
        }
        let mut many = narrative();
        for index in 0..30 {
            many[format!("extraField{index}")] = json!("PRIVATE_UNKNOWN_VALUE");
        }
        let failure = reject(&many);
        assert_eq!(
            failure["diagnostics"].as_array().unwrap().len(),
            MAX_DIAGNOSTICS
        );
        assert_eq!(failure["diagnosticsTruncated"], true);
    }

    #[test]
    fn narrative_input_type_and_syntax_errors_never_echo_submitted_values() {
        let mut value = narrative();
        value["operations"][1]["events"] = json!("PRIVATE_WRONG_TYPE_VALUE");
        let failure = reject(&value);
        assert_eq!(failure["diagnostics"][0]["path"], "operations[1]");
        assert_eq!(failure["diagnostics"][0]["kind"], "INVALID_SCHEMA");
        assert!(!failure.to_string().contains("PRIVATE_WRONG_TYPE_VALUE"));

        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("malformed.yaml");
        std::fs::write(&path, b"schema: [PRIVATE_SOURCE_SNIPPET\n").unwrap();
        let failure = read_narrative(&path).unwrap_err().update_failure();
        assert_eq!(failure["diagnostics"][0]["kind"], "INVALID_SYNTAX");
        assert!(!failure.to_string().contains("PRIVATE_SOURCE_SNIPPET"));
        assert!(
            !failure
                .to_string()
                .contains(&path.to_string_lossy().to_string())
        );
    }

    #[test]
    fn narrative_input_valid_json_and_yaml_preserve_the_typed_record() {
        let expected: Narrative = serde_json::from_value(narrative()).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        for (file, bytes) in [
            ("input.json", serde_json::to_vec(&expected).unwrap()),
            (
                "input.yaml",
                serde_yaml_ng::to_string(&expected).unwrap().into_bytes(),
            ),
        ] {
            let path = temporary.path().join(file);
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(read_narrative(&path).unwrap(), expected);
        }
    }
}
