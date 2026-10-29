//! Guidance at authored-input boundaries; Serde remains the validator.
use super::{invalid, store};
use crate::error::ClewError;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

pub(super) const SELECTOR_GUIDANCE: &str = "Expected a retained entrypoint ID for a service Work, a saved scenario:ID for scenario selection, or an exact callable SYMBOL ID for rootDeclaration/--symbol; a qualified method name may be ambiguous and is not an entrypoint ID. Work-local proposal/navigation references are a separate namespace. Discover with clew docs endpoint list --root ROOT --service SERVICE --snapshot SNAPSHOT, or clew docs context --root ROOT --service SERVICE --snapshot SNAPSHOT --format compact --limit 20 (follow nextCursor). Replace ROOT/SERVICE/SNAPSHOT with the selected documentation root, service and immutable snapshot; copy returned IDs, do not guess them.";

#[derive(Clone, Copy)]
pub(super) enum Input {
    Draft,
    DraftContext,
    Proposal,
    DataModel,
}

impl Input {
    pub(super) fn schema(self) -> &'static str {
        match self {
            Self::Draft => "codeclew-documentation-operation-draft-execution/1.0",
            Self::DraftContext => "codeclew-documentation-operation-draft-execution/1.1",
            Self::Proposal => "codeclew-documentation-proposal/1.0",
            Self::DataModel => "codeclew-data-model-plan/1.0",
        }
    }

    pub(super) fn guidance(self) -> String {
        match self {
            Self::Draft | Self::DraftContext => format!(
                "--config describes an isolated author driver and finite budget, not provider endpoints or API keys. Required: schema, author (adapter, model, command, runtimeReads, cap), budget (account, costUnit, ceiling, stopLoss){}. Shape example (replace placeholders; no driver is launched by this example): {}. Version 1.0 is single-call; version 1.1 requires positive authorCalls and Work authoringContract 1.6. Next: clew docs work run --help",
                if matches!(self, Self::DraftContext) { ", authorCalls" } else { "" },
                draft_example(matches!(self, Self::DraftContext)),
            ),
            Self::Proposal => "Required: schema and operations. Contract rows use {\"label\":\"value\",\"description\":{\"text\":\"Meaning established by evidence\",\"evidence\":[\"RETURNED_EVIDENCE_ID\"]}} inside contracts:[{title,kind,rows}], with kind http, kafka or payload. Replace the placeholder with evidence returned by this Work's supported reads; operation entrypoint is a Work-local reference, not a guessed callable or endpoint ID. Source DTO properties/observed payloads do not establish a wire schema. Next: clew docs proposal submit --help".into(),
            Self::DataModel => "Required: schema, publication, snapshot, models. Field shape: {id,path,meaning:{title,text,evidence},representations:[{label,typeLabel,requiredness,nullability,mapping:{title,text,evidence}}]}. SOURCE evidence shape: {kind:\"SOURCE\",service,sourceId,textDigest,startLine,endLine}; copy exact values from the selected snapshot. RETAINED evidence instead names an exact subject/operation/fragmentId/textDigest. Use payloadViews with coverage.status observed-fields for partial source observations, or unknown with fields:[] when the field definition is unavailable. Source DTO properties/observed payloads do not establish a wire schema; contracts require an actual retained JSON schema source. Next: clew docs render --help; see schemas/documentation/data-model-plan.schema.json".into(),
        }
    }
}

pub(super) fn draft_example(expanding: bool) -> Value {
    let mut value = json!({
        "schema": if expanding { Input::DraftContext.schema() } else { Input::Draft.schema() },
        "author": {"adapter":"macos-seatbelt-stdio/1.0", "model":"MODEL_ID",
            "command":["/path/to/isolated-author-driver"], "runtimeReads":[],
            "cap":{"maximum":{"inputTokens":1000,"outputTokens":1000,"costUnits":1},
                "overheadInputTokens":0,"timeoutMs":10000,"outputBytes":65536}},
        "budget":{"account":"ACCOUNT_ID","costUnit":"unit",
            "ceiling":{"inputTokens":4000,"outputTokens":4000,"costUnits":4},
            "stopLoss":{"inputTokens":3000,"outputTokens":3000,"costUnits":3}}
    });
    if expanding {
        value["authorCalls"] = json!(2);
    }
    value
}

pub(super) fn read<T: DeserializeOwned>(
    path: &Path,
    limit: u64,
    input: Input,
) -> Result<T, ClewError> {
    let data = store::read_bytes(path, limit)?;
    serde_path_to_error::deserialize(serde_yaml_ng::Deserializer::from_slice(&data)).map_err(|error| {
        if let Err(syntax) = serde_yaml_ng::from_slice::<Value>(&data) {
            let location = syntax.location().map(|p| format!(" at line {}, column {}", p.line(), p.column())).unwrap_or_default();
            invalid(format!("documentation input violates its closed JSON/YAML schema: malformed JSON/YAML at ${location}; expected {}. {}", input.schema(), input.guidance()))
        } else {
            input_error(error.path(), &error.inner().to_string(), input)
        }
    })
}

pub(super) fn from_value<T: DeserializeOwned>(value: Value, input: Input) -> Result<T, ClewError> {
    serde_path_to_error::deserialize(value)
        .map_err(|error| input_error(error.path(), &error.inner().to_string(), input))
}

fn input_error(error_path: &serde_path_to_error::Path, detail: &str, input: Input) -> ClewError {
    // Paths may contain arbitrary map keys (including secrets). Only names
    // declared by the shipped schemas are safe to display. This inventory
    // sanitizes diagnostics; it does not validate or resolve schema refs.
    let names = property_names();
    let mut path = "$".to_owned();
    for segment in error_path {
        match segment {
            serde_path_to_error::Segment::Seq { index } => path.push_str(&format!("[{index}]")),
            serde_path_to_error::Segment::Map { key } if names.contains(key) => {
                path.push('.');
                path.push_str(key);
            }
            _ => path.push_str("[<key>]"),
        }
    }
    let expected = if let Some(name) = detail
        .split_once("missing field `")
        .and_then(|(_, s)| s.split_once('`').map(|(name, _)| name))
        .filter(|name| names.contains(*name))
    {
        path.push('.');
        path.push_str(name);
        "required field is missing".to_owned()
    } else if detail.contains("unknown field ") {
        "unexpected field; use only properties declared by this schema".into()
    } else {
        // Serde's expected side is a static type description. Its invalid
        // value/variant side may contain input and must never be emitted.
        detail
            .rsplit_once(", expected ")
            .map(|(_, expected)| format!("expected {expected}"))
            .unwrap_or_else(|| "expected the declared field type/shape".into())
    };
    let prefix = if matches!(input, Input::DraftContext) {
        "OPERATION_DRAFT_CONFIG_UNSUPPORTED"
    } else {
        "documentation input violates its closed JSON/YAML schema"
    };
    invalid(format!(
        "{prefix}: {path}: {expected}; schema={}. {}",
        input.schema(),
        input.guidance()
    ))
}

fn property_names() -> BTreeSet<String> {
    fn collect(value: &Value, names: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::Object(properties)) = map.get("properties") {
                    names.extend(properties.keys().cloned());
                }
                for child in map.values() {
                    collect(child, names);
                }
            }
            Value::Array(values) => {
                for child in values {
                    collect(child, names);
                }
            }
            _ => {}
        }
    }
    let mut names = BTreeSet::new();
    for schema in [
        include_str!("../../../../schemas/documentation/operation-draft-execution.schema.json"),
        include_str!("../../../../schemas/documentation/operation-draft-execution-1.1.schema.json"),
        include_str!("../../../../schemas/documentation/agent-job.schema.json"),
        include_str!("../../../../schemas/documentation/proposal.schema.json"),
        include_str!("../../../../schemas/documentation/data-model-plan.schema.json"),
    ] {
        collect(
            &serde_json::from_str(schema).expect("shipped schema JSON"),
            &mut names,
        );
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fields {
        models: Vec<Field>,
    }
    #[derive(Debug, Deserialize)]
    struct Field {
        fields: Vec<String>,
    }

    #[test]
    fn diagnostic_typed_paths_keep_indices_and_expected_type_without_input_values() {
        let error = from_value::<Fields>(
            json!({"models":[{"fields":"private-value, expected private-suffix"}]}),
            Input::DataModel,
        )
        .unwrap_err();
        assert!(
            error
                .message
                .contains("$.models[0].fields: expected a sequence"),
            "{error}"
        );
        assert!(!error.message.contains("private-value"));
        assert!(!error.message.contains("private-suffix"));
        assert!(error.message.contains("wire schema"));
        let missing = from_value::<Fields>(json!({"models":[{}]}), Input::DataModel).unwrap_err();
        assert!(
            missing
                .message
                .contains("$.models[0].fields: required field is missing"),
            "{missing}"
        );
        let extra = from_value::<Fields>(
            json!({"private-key":"private-value","models":[]}),
            Input::DataModel,
        )
        .unwrap_err();
        assert!(extra.message.contains("unexpected field"));
        assert!(!extra.message.contains("private-key"));
        let dynamic = from_value::<std::collections::BTreeMap<String, Fields>>(
            json!({"private-map-key":{"models":[{"fields":42}]}}),
            Input::DataModel,
        )
        .unwrap_err();
        assert!(dynamic.message.contains("$[<key>].models[0].fields"));
        assert!(!dynamic.message.contains("private-map-key"));
        let ok: Fields =
            from_value(json!({"models":[{"fields":["value"]}]}), Input::DataModel).unwrap();
        assert_eq!(ok.models[0].fields, ["value"]);
    }

    #[test]
    fn diagnostic_malformed_document_is_distinct_and_does_not_echo_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.json");
        std::fs::write(&path, b"{\"private-key\":\"private-value\"").unwrap();
        let error = read::<Value>(&path, 1024, Input::Proposal).unwrap_err();
        assert!(error.message.contains("malformed JSON/YAML at $"));
        assert!(!error.message.contains("private-key"));
        assert!(!error.message.contains("private-value"));
    }

    #[test]
    fn diagnostic_file_reader_preserves_closed_serde_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.json");
        for text in [
            r#"{"models":[],"models":[]}"#,
            r#"{"models":[{}]}"#,
            r#"{"models":[{"fields":42}]}"#,
        ] {
            std::fs::write(&path, text).unwrap();
            let error = read::<Fields>(&path, 1024, Input::DataModel).unwrap_err();
            assert_eq!(error.code, crate::error::ErrorCode::InvalidInput);
            assert!(
                error
                    .message
                    .contains("schema=codeclew-data-model-plan/1.0"),
                "{error}"
            );
        }
    }
}
