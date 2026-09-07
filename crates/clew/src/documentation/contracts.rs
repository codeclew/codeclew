//! Declared OpenAPI is independent from compiler and runtime authority.
use super::{analysis, digest, invalid, model::*};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn resolved(
    value: &Value,
    document: &Value,
    seen: &mut BTreeSet<String>,
    gaps: &mut BTreeSet<String>,
    depth: usize,
) -> Value {
    if depth > 32 {
        gaps.insert("CONTRACT_REFERENCE_DEPTH_EXCEEDED".into());
        return value.clone();
    }
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                if !reference.starts_with("#/") {
                    gaps.insert(format!("EXTERNAL_CONTRACT_REFERENCE:{reference}"));
                    return value.clone();
                }
                if !seen.insert(reference.into()) {
                    gaps.insert(format!("CYCLIC_CONTRACT_REFERENCE:{reference}"));
                    return value.clone();
                }
                let out = match document.pointer(&reference[1..]) {
                    Some(target) => resolved(target, document, seen, gaps, depth + 1),
                    None => {
                        gaps.insert(format!("MISSING_CONTRACT_REFERENCE:{reference}"));
                        value.clone()
                    }
                };
                seen.remove(reference);
                return out;
            }
            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), resolved(v, document, seen, gaps, depth + 1)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| resolved(v, document, seen, gaps, depth + 1))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub fn enrich(evidence: &mut ServiceEvidence) -> Result<(), ClewError> {
    let mut operations = Vec::new();
    for entry in &evidence.entrypoints {
        if entry.kind != "HTTP_ENDPOINT" {
            continue;
        }
        let methods: Vec<_> = entry.trigger["methods"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let paths: Vec<_> = entry.trigger["paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for (file, document) in &evidence.contracts {
            for path in &paths {
                for method in &methods {
                    let operation = &document["paths"][*path][method.to_lowercase()];
                    if operation.is_null() {
                        continue;
                    }
                    let mut gaps = BTreeSet::new();
                    let mut seen = BTreeSet::new();
                    let value = resolved(operation, document, &mut seen, &mut gaps, 0);
                    let inherited = resolved(
                        &document["paths"][*path]["parameters"],
                        document,
                        &mut seen,
                        &mut gaps,
                        0,
                    );
                    let mut parameters = BTreeMap::new();
                    for p in inherited
                        .as_array()
                        .into_iter()
                        .flatten()
                        .chain(value["parameters"].as_array().into_iter().flatten())
                    {
                        parameters.insert(format!("{}:{}", p["in"], p["name"]), p.clone());
                    }
                    let source_id =
                        analysis::source_id(&evidence.service, &format!("contract/{file}"))?;
                    let normalized = json!({"entrypoint":entry.id,"method":method,"path":path,"operation":value,"parameters":parameters.into_values().collect::<Vec<_>>(),"security":operation.get("security").unwrap_or(&document["security"]),"securitySchemes":document["components"]["securitySchemes"],"servers":operation.get("servers").or_else(||document["paths"][*path].get("servers")).unwrap_or(&document["servers"]),"declaredSource":file,"boundaries":gaps,"authority":"DECLARED_OPENAPI"});
                    let id = analysis::dependency_id(
                        &evidence.service,
                        "contract-operation",
                        &format!("{}:{file}:{method}:{path}", entry.id),
                    )?;
                    operations.push(Observation {
                        id,
                        kind: "CONTRACT_OPERATION".into(),
                        service: evidence.service.clone(),
                        symbol: entry.symbol.clone(),
                        digest: digest(&normalized)?,
                        normalized,
                        source_ids: vec![source_id],
                    });
                }
            }
        }
    }
    for operation in operations {
        if operation
            .source_ids
            .iter()
            .any(|id| !evidence.sources.contains_key(id))
        {
            return Err(invalid("declared contract source is unavailable"));
        }
        evidence
            .observations
            .insert(operation.id.clone(), operation);
    }
    Ok(())
}

pub fn for_entry<'a>(evidence: &'a ServiceEvidence, entry: &str) -> Vec<&'a Observation> {
    evidence
        .observations
        .values()
        .filter(|o| o.kind == "CONTRACT_OPERATION" && o.normalized["entrypoint"] == entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_resolution_keeps_constraints_and_marks_unknowns() {
        let doc = json!({"components":{"schemas":{"Item":{"type":"object","required":["id"],"properties":{"id":{"type":"integer","minimum":1}}}}}});
        let mut gaps = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let value = resolved(
            &json!({"$ref":"#/components/schemas/Item"}),
            &doc,
            &mut seen,
            &mut gaps,
            0,
        );
        assert_eq!(value["properties"]["id"]["minimum"], 1);
        assert!(gaps.is_empty());
        resolved(
            &json!({"$ref":"https://example.invalid/schema"}),
            &doc,
            &mut seen,
            &mut gaps,
            0,
        );
        assert!(!gaps.is_empty());
    }
}
