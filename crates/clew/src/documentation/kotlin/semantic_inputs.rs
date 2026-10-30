//! Validate optional documentation-only compiler metadata at its existing source owner.
use super::DeclarationOccurrence;
use crate::{documentation::invalid, error::ClewError};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn require(ok: bool) -> Result<(), ClewError> {
    if ok {
        Ok(())
    } else {
        Err(invalid(
            "Kotlin semantic inputs have an invalid source/scope/symbol binding",
        ))
    }
}

fn range(
    row: &Value,
    start: &str,
    end: &str,
    owner: &DeclarationOccurrence,
) -> Result<(u64, u64), ClewError> {
    let a = row[start]
        .as_u64()
        .ok_or_else(|| invalid("Kotlin semantic input lacks a byte range"))?;
    let b = row[end]
        .as_u64()
        .ok_or_else(|| invalid("Kotlin semantic input lacks a byte range"))?;
    require(owner.3 <= a && a < b && b <= owner.4)?;
    Ok((a, b))
}

fn region(row: &Value, owner: &DeclarationOccurrence) -> Result<(), ClewError> {
    let span = range(row, "byteStart", "byteEnd", owner)?;
    let kind = row["kind"].as_str().unwrap_or_default();
    let identity = row["identity"].as_str().unwrap_or_default();
    require(
        row["ownerSymbolIdentity"] == owner.2
            && match kind {
                "NAMED_FUNCTION" => {
                    identity == owner.2
                        && span == (owner.3, owner.4)
                        && row["execution"] == "OWNER_BODY"
                }
                "ANONYMOUS_FUNCTION" | "LOCAL_FUNCTION" => {
                    identity.starts_with(&format!("{}/lexical/{kind}/", owner.2))
                        && row["execution"] == "DEFERRED"
                        && row["invocationCountAndOrder"] == "NOT_ESTABLISHED"
                }
                _ => false,
            },
    )
}

pub(super) fn validate(
    value: &Value,
    owner: &DeclarationOccurrence,
    source_digest: Option<&str>,
) -> Result<(), ClewError> {
    require(
        value["schema"] == "codeclew-kotlin-documentation-semantic-inputs/1.0"
            && value["authority"] == "K2_LEXICAL_AND_ARGUMENT_SYMBOLS_AND_RETURN_TARGETS"
            && value["coordinateDomain"] == "ORIGINAL_UTF8_BYTES"
            && value["ownerSymbolIdentity"] == owner.2
            && value["file"] == owner.1
            && value["compilationScope"] == owner.0
            && value["ownerByteStart"] == owner.3
            && value["ownerByteEnd"] == owner.4
            && source_digest.is_some_and(|d| {
                d.strip_prefix("sha256:")
                    .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            })
            && value["fullCompilationSourceDigest"].as_str() == source_digest
            && value["cfgAndExecutionProof"] == "SEPARATE_NOT_ASSERTED",
    )?;
    let variables = value["variables"]
        .as_array()
        .ok_or_else(|| invalid("Kotlin semantic inputs lack variables"))?;
    let calls = value["calls"]
        .as_array()
        .ok_or_else(|| invalid("Kotlin semantic inputs lack calls"))?;
    let returns = value["returns"]
        .as_array()
        .ok_or_else(|| invalid("Kotlin semantic inputs lack returns"))?;
    require(
        variables.len() <= 4096
            && calls.len() <= 512
            && returns.len() <= 512
            && serde_json::to_vec(value)
                .map_err(|_| invalid("Kotlin semantic input serialization failed"))?
                .len()
                <= 524288,
    )?;
    if value["resolution"] == "UNKNOWN" {
        return require(
            variables.is_empty()
                && calls.is_empty()
                && returns.is_empty()
                && value["boundaries"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty()),
        );
    }
    require(value["resolution"] == "COMPILER_METADATA_NOT_EXECUTION_PROOF")?;
    let mut declarations = BTreeMap::new();
    for row in variables {
        let occurrence = range(row, "byteStart", "byteEnd", owner)?;
        let declaration = range(row, "declarationByteStart", "declarationByteEnd", owner)?;
        region(&row["lexicalRegion"], owner)?;
        region(&row["declarationRegion"], owner)?;
        let lexical = range(&row["lexicalRegion"], "byteStart", "byteEnd", owner)?;
        let declared = range(&row["declarationRegion"], "byteStart", "byteEnd", owner)?;
        require(
            lexical.0 <= occurrence.0
                && occurrence.1 <= lexical.1
                && declared.0 <= declaration.0
                && declaration.1 <= declared.1,
        )?;
        require(
            row["captured"].as_bool()
                == Some(row["lexicalRegion"]["identity"] != row["declarationRegion"]["identity"]),
        )?;
        if row.get("initializerSpan").is_some() {
            let initialized = range(&row["initializerSpan"], "byteStart", "byteEnd", owner)?;
            require(declaration.0 <= initialized.0 && initialized.1 <= declaration.1)?;
        }
        if row["kind"] == "VARIABLE_DECLARATION" {
            let id = row["variableIdentity"]
                .as_str()
                .ok_or_else(|| invalid("Kotlin semantic declaration lacks identity"))?;
            require(declarations.insert(id, row).is_none())?;
        } else {
            require(row["kind"] == "VARIABLE_ACCESS")?;
        }
    }
    for row in variables.iter().filter(|r| r["kind"] == "VARIABLE_ACCESS") {
        let declared = row["variableIdentity"]
            .as_str()
            .and_then(|id| declarations.get(id))
            .ok_or_else(|| invalid("Kotlin semantic access has no unique declaration"))?;
        require(
            row["declarationRegion"] == declared["declarationRegion"]
                && row["declarationByteStart"] == declared["declarationByteStart"]
                && row["declarationByteEnd"] == declared["declarationByteEnd"],
        )?;
    }
    for call in calls {
        let occurrence = range(call, "byteStart", "byteEnd", owner)?;
        let arguments = call["arguments"]
            .as_array()
            .ok_or_else(|| invalid("Kotlin semantic call lacks argument mapping"))?;
        if call["resolution"] == "UNKNOWN" {
            require(
                arguments.is_empty()
                    && call["boundaries"].as_array().is_some_and(|a| !a.is_empty()),
            )?;
            continue;
        }
        require(
            call["resolution"] == "COMPILER_EXACT" && call["receiverObjectIdentity"] == "UNKNOWN",
        )?;
        region(&call["lexicalRegion"], owner)?;
        let lexical = range(&call["lexicalRegion"], "byteStart", "byteEnd", owner)?;
        require(lexical.0 <= occurrence.0 && occurrence.1 <= lexical.1)?;
        let target = call["targetIdentity"]
            .as_str()
            .filter(|s| {
                (s.starts_with("callable:") || s.starts_with("constructor:")) && s.contains("#jvm:")
            })
            .ok_or_else(|| invalid("Kotlin semantic call target is not exact"))?;
        let mut slots = BTreeSet::new();
        let mut previous = None;
        for (index, arg) in arguments.iter().enumerate() {
            let syntactic = range(arg, "argumentByteStart", "argumentByteEnd", owner)?;
            let actual = range(arg, "actualByteStart", "actualByteEnd", owner)?;
            let slot = arg["parameterIndex"]
                .as_u64()
                .ok_or_else(|| invalid("Kotlin semantic argument lacks formal slot"))?;
            require(
                arg["evaluationIndex"] == index as u64
                    && arg["resolution"] == "COMPILER_EXACT"
                    && slots.insert(slot)
                    && arg["formalIdentity"] == format!("{target}/parameter/{slot}")
                    && occurrence.0 <= syntactic.0
                    && syntactic.1 <= occurrence.1
                    && syntactic.0 <= actual.0
                    && actual.1 <= syntactic.1
                    && previous.is_none_or(|p| p < syntactic.0),
            )?;
            previous = Some(syntactic.0);
            if let Some(id) = arg["actualVariableIdentity"].as_str() {
                let refs: Vec<_> = variables
                    .iter()
                    .filter(|r| {
                        r["kind"] == "VARIABLE_ACCESS"
                            && r["accessMode"] == "READ"
                            && r["variableIdentity"] == id
                            && r["byteStart"] == actual.0
                            && r["byteEnd"] == actual.1
                            && r["lexicalRegion"] == call["lexicalRegion"]
                    })
                    .collect();
                require(
                    refs.len() == 1
                        && refs[0]["declarationRegion"] == arg["declarationRegion"]
                        && refs[0]["declarationByteStart"] == arg["declarationByteStart"]
                        && refs[0]["declarationByteEnd"] == arg["declarationByteEnd"]
                        && refs[0].get("initializerSpan") == arg.get("initializerSpan")
                        && refs[0]["captured"] == arg["captured"],
                )?;
            }
        }
        if call["omittedDefaultParameterIndices"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            require(call["boundaries"].as_array().is_some_and(|a| {
                a.iter()
                    .any(|v| v == "DEFAULT_ARGUMENT_EVALUATION_UNSUPPORTED")
            }))?;
        }
    }
    for row in returns {
        let occurrence = range(row, "byteStart", "byteEnd", owner)?;
        require(row["returnValueProof"] == "SEPARATE_NOT_ASSERTED")?;
        if row["resolution"] == "COMPILER_EXACT" {
            require(row["authority"] == "K2_SELECTED_RETURN_TARGET")?;
            region(&row["lexicalRegion"], owner)?;
            region(&row["targetRegion"], owner)?;
            for target in [&row["lexicalRegion"], &row["targetRegion"]] {
                let lexical = range(target, "byteStart", "byteEnd", owner)?;
                require(lexical.0 <= occurrence.0 && occurrence.1 <= lexical.1)?;
            }
            require(
                row["nonLocal"].as_bool()
                    == Some(row["lexicalRegion"]["identity"] != row["targetRegion"]["identity"]),
            )?;
        } else {
            require(row["resolution"] == "UNKNOWN" && row["code"].is_string())?;
        }
    }
    Ok(())
}

pub(super) fn validate_event_carriers(flow: &Value) -> Result<(), ClewError> {
    for event in flow["events"].as_array().into_iter().flatten() {
        let Some(row) = event.get("semanticInput") else {
            continue;
        };
        let list = match event["kind"].as_str() {
            Some("CALL" | "CONSTRUCT") => "calls",
            Some("RETURN") => "returns",
            _ => {
                return Err(invalid(
                    "Kotlin semantic carrier belongs to an unsupported event",
                ));
            }
        };
        let candidates: Vec<_> = flow["semanticInputs"][list]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|candidate| {
                candidate["byteStart"] == event["sourceSpan"]["byteStart"]
                    && candidate["byteEnd"] == event["sourceSpan"]["byteEnd"]
            })
            .collect();
        require(candidates.len() == 1 && candidates[0] == row)?;
    }
    Ok(())
}

/// Existing ambiguous descriptor quarantine also applies to the optional carrier.
pub(super) fn quarantine(
    flow: &mut Value,
    scope: &str,
    owner_ambiguous: bool,
    ambiguous: &BTreeSet<(String, String)>,
) {
    fn call(row: &mut Value, scope: &str, blocked: bool, ambiguous: &BTreeSet<(String, String)>) {
        if blocked
            || row["targetIdentity"]
                .as_str()
                .is_some_and(|id| ambiguous.contains(&(scope.into(), id.into())))
        {
            row["reportedTargetIdentity"] = row["targetIdentity"].clone();
            if let Some(object) = row.as_object_mut() {
                object.remove("targetIdentity");
            }
            row["resolution"] = json!("UNKNOWN");
            row["arguments"] = json!([]);
            row["boundaries"] = json!(["KOTLIN_CALL_DECLARATION_IDENTITY_NOT_UNIQUE"]);
        }
    }
    if let Some(metadata) = flow.get_mut("semanticInputs") {
        if owner_ambiguous {
            metadata["resolution"] = json!("UNKNOWN");
            for key in ["variables", "calls", "returns"] {
                metadata[key] = json!([]);
            }
            metadata["boundaries"] = json!(["KOTLIN_SEMANTIC_OWNER_IDENTITY_NOT_UNIQUE"]);
        } else {
            for row in metadata["calls"].as_array_mut().into_iter().flatten() {
                call(row, scope, false, ambiguous);
            }
        }
    }
    for event in flow["events"].as_array_mut().into_iter().flatten() {
        if owner_ambiguous {
            if event.get("semanticInput").is_some() {
                event["semanticInput"] = json!({"resolution":"UNKNOWN","code":"KOTLIN_SEMANTIC_OWNER_IDENTITY_NOT_UNIQUE"});
            }
        } else if matches!(event["kind"].as_str(), Some("CALL" | "CONSTRUCT"))
            && let Some(row) = event.get_mut("semanticInput")
        {
            call(row, scope, false, ambiguous);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, DeclarationOccurrence) {
        let value: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/kotlin-semantic-inputs/consume-k2-2.4.10.json"
        )))
        .unwrap();
        let owner = (
            value["compilationScope"].as_str().unwrap().into(),
            value["file"].as_str().unwrap().into(),
            value["symbolIdentity"].as_str().unwrap().into(),
            value["byteStart"].as_u64().unwrap(),
            value["byteEnd"].as_u64().unwrap(),
        );
        assert_eq!(
            crate::canonical::hash_bytes(value["source"].as_str().unwrap().as_bytes()),
            value["contentHash"]
        );
        (value, owner)
    }
    #[test]
    fn real_compiler_projection_is_source_bound_and_refuses_wrong_symbol_slot_order_and_target() {
        let (fixture, owner) = fixture();
        let metadata = &fixture["semanticInputs"];
        validate(metadata, &owner, fixture["contentHash"].as_str()).unwrap();
        let index = metadata["calls"]
            .as_array()
            .unwrap()
            .iter()
            .position(|r| {
                r["targetIdentity"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("docs/refresh#")
            })
            .unwrap();
        let call = &metadata["calls"][index];
        let argument = &call["arguments"][0];
        assert_eq!(
            fixture["source"].as_str().unwrap().get(
                argument["actualByteStart"].as_u64().unwrap() as usize
                    ..argument["actualByteEnd"].as_u64().unwrap() as usize
            ),
            Some("place")
        );
        assert!(validate(metadata, &owner, Some("sha256:wrong")).is_err());
        for (field, wrong) in [
            ("actualVariableIdentity", json!("upstream-event")),
            (
                "formalIdentity",
                json!("callable:docs/refresh#jvm:(Ljava/lang/String;)V/parameter/1"),
            ),
            ("evaluationIndex", json!(1)),
        ] {
            let mut changed = metadata.clone();
            changed["calls"][index]["arguments"][0][field] = wrong;
            assert!(
                validate(&changed, &owner, fixture["contentHash"].as_str()).is_err(),
                "accepted {field}"
            );
        }
        let mut changed = metadata.clone();
        changed["returns"][0]["targetRegion"]["identity"] = json!(owner.2);
        assert!(validate(&changed, &owner, fixture["contentHash"].as_str()).is_err());
        let mut changed = metadata.clone();
        changed["compilationScope"] = json!("another-compilation");
        assert!(validate(&changed, &owner, fixture["contentHash"].as_str()).is_err());
    }
    #[test]
    fn event_carrier_and_descriptor_quarantine_cannot_reintroduce_an_exact_ambiguous_mapping() {
        let (fixture, owner) = fixture();
        let metadata = &fixture["semanticInputs"];
        let call = metadata["calls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                r["targetIdentity"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("docs/refresh#")
            })
            .unwrap();
        let mut flow = json!({"semanticInputs":metadata,"events":[{"kind":"CALL","sourceSpan":{"byteStart":call["byteStart"],"byteEnd":call["byteEnd"]},"semanticInput":call}]});
        validate_event_carriers(&flow).unwrap();
        let mut changed = flow.clone();
        changed["events"][0]["semanticInput"]["arguments"][0]["formalIdentity"] =
            json!("unrelated-target");
        assert!(validate_event_carriers(&changed).is_err());
        let ambiguous = BTreeSet::from([(
            owner.0.clone(),
            call["targetIdentity"].as_str().unwrap().into(),
        )]);
        quarantine(&mut flow, &owner.0, false, &ambiguous);
        assert_eq!(flow["events"][0]["semanticInput"]["resolution"], "UNKNOWN");
        assert_eq!(flow["events"][0]["semanticInput"]["arguments"], json!([]));
        assert!(
            flow["events"][0]["semanticInput"]
                .get("targetIdentity")
                .is_none()
        );
    }
}
