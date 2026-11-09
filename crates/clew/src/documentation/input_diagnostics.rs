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
    SourcePart,
    SourceBatch,
    SourceSelection,
    NativePageSelections,
    SourceRepositories,
}

impl Input {
    pub(super) fn schema(self) -> &'static str {
        match self {
            Self::Draft => "codeclew-documentation-operation-draft-execution/1.0",
            Self::DraftContext => "codeclew-documentation-operation-draft-execution/1.1",
            Self::Proposal => "codeclew-documentation-proposal/1.0",
            Self::DataModel => "codeclew-data-model-plan/1.0",
            Self::NativePageSelections => {
                "native page declaration selections (array; no schema field)"
            }
            Self::SourceRepositories => "codeclew-source-repositories/1.0",
            Self::SourcePart => "codeclew-documentation-source-part-request/1.0",
            Self::SourceBatch | Self::SourceSelection => {
                "codeclew-documentation-work-batch-request/1.0"
            }
        }
    }

    pub(super) fn guidance(self) -> String {
        match self {
            Self::Draft | Self::DraftContext => format!(
                "--config describes an isolated author driver and finite budget, not provider endpoints or API keys. Required: schema, author (adapter, model, command, runtimeReads, cap), budget (account, costUnit, ceiling, stopLoss){}. Shape example (replace placeholders; no driver is launched by this example): {}. Version 1.0 is single-call; version 1.1 requires positive authorCalls and Work authoringContract 1.6. Next: clew docs work run --help",
                if matches!(self, Self::DraftContext) { ", authorCalls" } else { "" },
                draft_example(matches!(self, Self::DraftContext)),
            ),
            Self::Proposal => "Required: schema and operations. Contract rows use {\"label\":\"value\",\"description\":{\"text\":\"Meaning established by evidence\",\"evidence\":[\"RETURNED_EVIDENCE_ID\"]}} inside contracts:[{title,kind,rows}], with kind http, kafka or payload. Every new payload row also requires field:{path,direction} and its own evidenced meaning; directions are request, response, event or record. Keep group headings in title and transport metadata separate. Repair retained groups using Work evidence without recapture. Replace the placeholder with evidence returned by this Work's supported reads; operation entrypoint is a Work-local reference, not a guessed callable or endpoint ID. Source DTO properties/observed payloads do not establish a wire schema. Next: clew docs proposal submit --help".into(),
            Self::DataModel => "Required: schema, publication, snapshot, models. Field shape: {id,path,meaning:{title,text,evidence},representations:[{label,typeLabel,requiredness,nullability,mapping:{title,text,evidence}}]}. SOURCE evidence shape: {kind:\"SOURCE\",service,sourceId,textDigest,startLine,endLine}; copy exact values from the selected snapshot. RETAINED evidence instead names an exact subject/operation/fragmentId/textDigest. Use payloadViews with coverage.status observed-fields for partial source observations, or unknown with fields:[] when the field definition is unavailable. Source DTO properties/observed payloads do not establish a wire schema; contracts require an actual retained JSON schema source. Next: clew docs render --help; see schemas/documentation/data-model-plan.schema.json".into(),
            Self::SourcePart => "Required: {\"schema\":\"codeclew-documentation-source-part-request/1.0\",\"reference\":\"RETURNED_SOURCE_REFERENCE\"}; optional cursor copied from this source part. No projection field. The response schema source-part/1.0 is not request input. Read until nextCursor is null. See schemas/source-part-request.schema.json in the installed skill; clew docs work read-part --help.".into(),
            Self::SourceBatch => "Required: {\"schema\":\"codeclew-documentation-work-batch-request/1.0\",\"selections\":[{\"projection\":\"SOURCE_ROWS\",\"references\":[\"RETURNED_SOURCE_REFERENCE\"]}]}. Use 1..64 tracked SOURCE selections. Inspect each result.page.nextCursor, result.page.omitted or result.error independently; a successful envelope is not complete delivery. See schemas/work-batch-request.schema.json in the installed skill; clew docs work read-batch --help.".into(),
            Self::NativePageSelections => "--input is a file path containing a closed JSON array, not an inline selector or scenario:ID. Minimal example: [{\"id\":\"selected-operation\",\"service\":\"SERVICE_ID\",\"endpointDeclaration\":\"RETURNED_DECLARATION_ID\",\"workerDeclaration\":\"RETURNED_DECLARATION_ID\"}]. Both declaration slots may use the same exact ID; slot names do not establish HTTP/worker roles. Discover IDs with clew docs context --root ROOT --snapshot SNAP --service SERVICE_ID. For a saved service/process publication use --publication PUB instead of --input. See installed references/service-documentation.md, Render exact retained declarations.".into(),
            Self::SourceRepositories => "--source-map is a file path with wrapper {\"schema\":\"codeclew-source-repositories/1.0\",\"repositories\":{\"IMMUTABLE_REPOSITORY_ID\":{\"provider\":\"gitlab\",\"webBase\":\"https://gitlab.example.invalid/group/subgroup/repository\"}}}. Keys are retained repository IDs, not service IDs. Optional origin must match the exact historical clone origin. The output sourceExport policy is not this input wrapper. See installed references/service-documentation.md, Source links for shared documentation.".into(),
            Self::SourceSelection => "One batch selection must be a closed Selection: {\"projection\":\"SOURCE_ROWS\",\"references\":[\"RETURNED_SOURCE_REFERENCE\"]} or {\"query\":{\"kind\":\"SOURCE\"}} with its own optional cursor. At most8 references; tracked SOURCE only. A malformed selection is returned as its own result.error; other selections remain independent. See installed schemas/work-batch-request.schema.json#/$defs/sourceSelection.".into(),
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
        match serde_yaml_ng::from_slice::<Value>(&data) {
        Err(syntax) => {
            let location = syntax.location().map(|p| format!(" at line {}, column {}", p.line(), p.column())).unwrap_or_default();
            let mut error = invalid(format!("documentation input violates its closed JSON/YAML schema: malformed JSON/YAML at ${location}; expected {}. {}", input.schema(), input.guidance()));
            error.details = Some(Box::new(json!({"schema":input.schema(),"path":"$","category":"SYNTAX","keyword":"syntax","expectedShape":"valid JSON/YAML","actualShape":"malformed","repair":input.guidance()})));
            error
        },
        Ok(value) => input_error(error.path(), &error.inner().to_string(), input, Some(&value)),
        }
    })
}

pub(super) fn from_value<T: DeserializeOwned>(value: Value, input: Input) -> Result<T, ClewError> {
    serde_path_to_error::deserialize(&value).map_err(|error| {
        input_error(
            error.path(),
            &error.inner().to_string(),
            input,
            Some(&value),
        )
    })
}

fn input_error(
    error_path: &serde_path_to_error::Path,
    detail: &str,
    input: Input,
    value: Option<&Value>,
) -> ClewError {
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
    let mut actual = value;
    for segment in error_path {
        actual = match segment {
            serde_path_to_error::Segment::Seq { index } => actual
                .and_then(|v| v.as_array())
                .and_then(|v| v.get(*index)),
            serde_path_to_error::Segment::Map { key } => {
                actual.and_then(|v| v.as_object()).and_then(|v| v.get(key))
            }
            _ => actual,
        };
    }
    let actual_type = match actual {
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => "integer",
        Some(Value::Number(_)) => "number",
        None => "unknown",
    };
    let (category, keyword, actual_shape) = if detail.contains("missing field `") {
        ("REQUIRED_FIELD_MISSING", "required", "absent")
    } else if detail.contains("unknown field ") {
        ("UNEXPECTED_FIELD", "additionalProperties", "present")
    } else if detail.contains("duplicate field ") {
        ("DUPLICATE_FIELD", "uniqueKeys", "duplicate")
    } else if detail.contains("unknown variant") {
        ("UNRECOGNIZED_VARIANT", "enum", actual_type)
    } else {
        ("TYPE_OR_SHAPE_MISMATCH", "type", actual_type)
    };
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
    let mut error = invalid(format!(
        "{prefix}: {path}: {expected}; schema={}. {}",
        input.schema(),
        input.guidance()
    ));
    error.details = Some(Box::new(
        json!({"schema":input.schema(),"path":path,"category":category,
        "keyword":keyword,"expectedShape":expected,"actualShape":actual_shape,"repair":input.guidance()}),
    ));
    error
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
        include_str!("../../../../schemas/documentation/source-part-request.schema.json"),
        include_str!("../../../../schemas/documentation/work-batch-request.schema.json"),
    ] {
        collect(
            &serde_json::from_str(schema).expect("shipped schema JSON"),
            &mut names,
        );
    }
    // Static properties of the two Rust-only export input contracts. Dynamic
    // repository IDs and arbitrary unknown keys remain redacted.
    names.extend(
        [
            "schema",
            "repositories",
            "provider",
            "webBase",
            "origin",
            "id",
            "service",
            "endpointDeclaration",
            "workerDeclaration",
            "wiringDeclaration",
            "question",
            "documentationLanguage",
            "noteIds",
            "authoredParagraphs",
            "expandSourceCalls",
            "expandDataState",
            "bundle",
            "operation",
            "fragment",
        ]
        .into_iter()
        .map(str::to_owned),
    );
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
    #[test]
    fn source_part_and_batch_guidance_keeps_requests_distinct_and_redacts_values() {
        let error=from_value::<super::super::work::SourcePartRequest>(json!({"schema":"codeclew-documentation-source-part-request/1.0","reference":"s1","projection":"PRIVATE_SOURCE_SENTINEL"}),Input::SourcePart).unwrap_err();
        assert_eq!(error.details.as_ref().unwrap()["path"], "$.projection");
        assert!(error.message.contains("source-part-request/1.0"));
        assert!(error.message.contains("No projection field"));
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("PRIVATE_SOURCE_SENTINEL")
        );
        let error = from_value::<super::super::work::Selection>(
            json!("PRIVATE_BATCH_SENTINEL"),
            Input::SourceSelection,
        )
        .unwrap_err();
        assert!(error.message.contains("SOURCE_ROWS"));
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("PRIVATE_BATCH_SENTINEL")
        );
    }
}
