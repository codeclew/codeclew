//! Closed K2 2.4 immutable-local projection. Parsing does not authenticate a run.
use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use clew_facts::values::{ModelBinding, ValueModel, ValueNode, ValueOperation};
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "k2-value-projection/0.1";
pub const POLICY: &str = "k2-2.4-immutable-local-values/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinValueProjection {
    pub schema: String,
    pub compiler_version: String,
    pub owner: String,
    pub owner_byte_start: u64,
    pub owner_byte_end: u64,
    pub full_compilation_source_digest: String,
    pub nodes: Vec<KotlinValueNode>,
    pub global_boundaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinValueNode {
    pub id: String,
    pub operation: ValueOperation,
    pub guards: Vec<String>,
    pub file: String,
    pub start: u64,
    pub end: u64,
    pub byte_start: u64,
    pub byte_end: u64,
    pub start_line: u64,
    pub end_line: u64,
}

fn invalid(message: &'static str) -> ClewError {
    ClewError::new(ErrorCode::WorkerProtocolMismatch, message)
}

/// Structural projection only. The caller authenticates binding and compiler
/// generation separately; the solver always retains its verification frontier.
pub fn project(
    projection: &KotlinValueProjection,
    binding: ModelBinding,
    fact_digest: &str,
) -> Result<ValueModel, ClewError> {
    if projection.schema != SCHEMA
        || projection.compiler_version != "2.4.10"
        || !projection.owner.starts_with("callable:")
        || !projection.owner.contains("#jvm:")
        || projection.owner.len() > 1024
        || projection.owner.chars().any(char::is_control)
        || projection.owner_byte_start >= projection.owner_byte_end
        || projection.nodes.is_empty()
        || projection.nodes.len() > 128
        || projection.global_boundaries != ["KOTLIN_VALUE_EFFECT_EXCEPTION_MODEL_UNAVAILABLE"]
        || binding.language != "language:kotlin"
        || binding.producer != POLICY
        || binding.policy_digest != canonical::hash_bytes(POLICY.as_bytes())
    {
        return Err(invalid(
            "Kotlin value projection version or profile is invalid",
        ));
    }
    let source = &projection.full_compilation_source_digest;
    if !source.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(invalid("Kotlin value source digest is invalid"));
    }
    let file = &projection.nodes[0].file;
    if file.len() > 4096
        || file.chars().any(char::is_control)
        || file.contains('\\')
        || !crate::kotlin_adapter_v2::safe_relative_source_path(file)
    {
        return Err(invalid("Kotlin value source file is invalid"));
    }
    for n in &projection.nodes {
        if &n.file != file
            || n.start >= n.end
            || n.byte_start < projection.owner_byte_start
            || n.byte_end > projection.owner_byte_end
            || n.byte_start >= n.byte_end
            || n.start_line == 0
            || n.start_line > n.end_line
            || n.id.len() > 128
            || n.operation.operands().iter().any(|id| id.len() > 128)
            || (n.id != "result" && n.id != format!("v{}_{}", n.start, n.end))
            || matches!(n.operation, ValueOperation::Join { .. })
        {
            return Err(invalid("Kotlin value node escapes source or profile"));
        }
        let guards = match &n.operation {
            ValueOperation::Coalesce { left, .. } => vec![left.clone()],
            _ => vec![],
        };
        if n.guards != guards {
            return Err(invalid("Kotlin value guards differ from profile"));
        }
        if let ValueOperation::Unknown { boundary } = &n.operation
            && !matches!(
                boundary.as_str(),
                "KOTLIN_VALUE_BODY_OR_BUDGET_UNSUPPORTED" | "KOTLIN_VALUE_EXPRESSION_UNSUPPORTED"
            )
        {
            return Err(invalid("Kotlin value transfer boundary is invalid"));
        }
    }
    if !projection.nodes.iter().any(|n| {
        n.id == "result"
            && matches!(
                n.operation,
                ValueOperation::Copy { .. } | ValueOperation::Unknown { .. }
            )
    }) {
        return Err(invalid("Kotlin value result definition is missing"));
    }
    if canonical::bytes(projection)
        .map_err(|_| invalid("Kotlin value serialization failed"))?
        .len()
        > 32768
    {
        return Err(invalid("Kotlin value projection exceeds byte budget"));
    }
    let model = ValueModel {
        schema: clew_facts::values::SCHEMA.into(),
        binding,
        nodes: projection
            .nodes
            .iter()
            .map(|n| ValueNode {
                id: n.id.clone(),
                operation: n.operation.clone(),
                guards: n.guards.clone(),
                evidence_ids: vec![format!("{fact_digest}#{}", n.id)],
            })
            .collect(),
        global_boundaries: projection.global_boundaries.clone(),
    };
    model.validate().map_err(invalid)?;
    Ok(model)
}

/// Reject malformed worker attachments before retaining their file fact. This
/// does not establish compiler execution or source-content authenticity.
pub(crate) fn validate_file_fact(row: &serde_json::Value) -> Result<(), ClewError> {
    for declaration in row
        .get("declarations")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(value) = declaration.get("valueProjection") else {
            continue;
        };
        let projection: KotlinValueProjection = serde_json::from_value(value.clone())
            .map_err(|_| invalid("Kotlin value projection contract is invalid"))?;
        if declaration
            .get("documentationSymbol")
            .and_then(serde_json::Value::as_str)
            != Some(&projection.owner)
            || row.get("path").and_then(serde_json::Value::as_str)
                != projection.nodes.first().map(|n| n.file.as_str())
            || row.get("contentHash").and_then(serde_json::Value::as_str)
                != Some(&projection.full_compilation_source_digest)
        {
            return Err(invalid(
                "Kotlin value projection differs from enclosing file or owner",
            ));
        }
        let zero = format!("sha256:{}", "0".repeat(64));
        project(
            &projection,
            ModelBinding {
                language: "language:kotlin".into(),
                producer: POLICY.into(),
                snapshot_digest: zero.clone(),
                facts_digest: zero.clone(),
                configuration_digest: zero,
                policy_digest: canonical::hash_bytes(POLICY.as_bytes()),
                observation_authority: clew_facts::values::ObservationAuthority::CompilerObserved,
            },
            "projection-validation",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_values::{Nullness, ValueQuery, analyze};
    use clew_facts::values::ObservationAuthority;

    fn projections() -> Vec<KotlinValueProjection> {
        serde_json::from_str(include_str!(
            "../../../fixtures/kotlin-values/projections.json"
        ))
        .unwrap()
    }
    fn binding() -> ModelBinding {
        let zero = format!("sha256:{}", "0".repeat(64));
        ModelBinding {
            language: "language:kotlin".into(),
            producer: POLICY.into(),
            snapshot_digest: zero.clone(),
            facts_digest: zero.clone(),
            configuration_digest: zero,
            policy_digest: canonical::hash_bytes(POLICY.as_bytes()),
            observation_authority: ObservationAuthority::CompilerObserved,
        }
    }
    fn query() -> ValueQuery {
        ValueQuery {
            target: "result".into(),
            max_nodes: 128,
            max_transfers: 4096,
        }
    }

    #[test]
    fn native_golden_uses_shared_nullness_transfers_and_exact_occurrences() {
        let source = include_str!("../../../fixtures/kotlin-values/Values.kt");
        for p in projections() {
            assert_eq!(
                p.full_compilation_source_digest,
                canonical::hash_bytes(source.as_bytes())
            );
            for n in &p.nodes {
                let start = n.byte_start as usize;
                let end = n.byte_end as usize;
                assert!(source.get(start..end).is_some_and(|s| !s.is_empty()));
                assert_eq!(source[..start].encode_utf16().count() as u64, n.start);
                assert_eq!(source[..end].encode_utf16().count() as u64, n.end);
                assert_eq!(
                    1 + source[..start].bytes().filter(|b| *b == b'\n').count() as u64,
                    n.start_line
                );
                assert_eq!(
                    1 + source[..end].bytes().filter(|b| *b == b'\n').count() as u64,
                    n.end_line
                );
            }
            let m = project(&p, binding(), "native-golden").unwrap();
            let result = analyze(&m, &query()).unwrap();
            let actual = result
                .observations
                .iter()
                .find(|n| n.id == "result")
                .unwrap()
                .normal_value_nullness;
            let cases: std::collections::BTreeMap<String, Nullness> =
                serde_json::from_str(include_str!("../../../fixtures/value-nullness-cases.json"))
                    .unwrap();
            let case = if p.owner.contains(".choose#") {
                "coalesce"
            } else if p.owner.contains(".booleanValue#") {
                "literal"
            } else if p.owner.contains(".nullValue#") {
                "null"
            } else if p.owner.contains(".defaulted#") {
                "input"
            } else if p.owner.contains(".field#") {
                "field"
            } else if p.owner.contains(".missing#") {
                "call"
            } else if p.owner.contains(".mutated#") {
                "write"
            } else {
                "control"
            };
            let expected = cases[case];
            assert_eq!(actual, expected, "{}", p.owner);
            assert!(!result.program_semantic_closure);
            assert!(
                result
                    .frontier
                    .iter()
                    .any(|f| f.code == "KOTLIN_VALUE_EFFECT_EXCEPTION_MODEL_UNAVAILABLE")
            );
            for n in &m.nodes {
                if let ValueOperation::Coalesce { left, .. } = &n.operation {
                    assert_eq!(n.guards.as_slice(), std::slice::from_ref(left));
                }
            }
            let cut = analyze(
                &m,
                &ValueQuery {
                    max_nodes: 1,
                    ..query()
                },
            )
            .unwrap();
            if m.nodes.len() > 1 {
                assert!(
                    cut.frontier
                        .iter()
                        .any(|f| f.code == "RELEVANCE_NODE_BUDGET")
                );
            }
        }
    }

    #[test]
    fn malformed_projection_and_foreign_ownership_fail_closed() {
        let p = projections()
            .into_iter()
            .find(|p| p.owner.contains(".choose#") && p.nodes.len() > 3)
            .unwrap();
        let mut dangling = p.clone();
        dangling.nodes[0].operation = ValueOperation::Copy {
            value: "foreign-definition".into(),
        };
        assert!(project(&dangling, binding(), "fact").is_err());
        let mut version = p.clone();
        version.compiler_version = "2.1.21".into();
        assert!(project(&version, binding(), "fact").is_err());
        let mut escaped = p.clone();
        escaped.nodes[0].byte_start = 0;
        assert!(project(&escaped, binding(), "fact").is_err());
        let mut guards = p.clone();
        guards.nodes[0].guards.push("result".into());
        assert!(project(&guards, binding(), "fact").is_err());
        let row = serde_json::json!({"path": p.nodes[0].file, "contentHash": p.full_compilation_source_digest,
            "declarations": [{"documentationSymbol": p.owner, "valueProjection": p}]});
        validate_file_fact(&row).unwrap();
        let mut foreign = row.clone();
        foreign["declarations"][0]["documentationSymbol"] = serde_json::json!("foreign");
        assert!(validate_file_fact(&foreign).is_err());
        let mut changed = row;
        changed["contentHash"] = serde_json::json!("sha256:changed");
        assert!(validate_file_fact(&changed).is_err());
        validate_file_fact(&serde_json::json!({"path": "Old.kt", "declarations": []})).unwrap();
    }
}
