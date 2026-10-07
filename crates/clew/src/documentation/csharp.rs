//! Project Roslyn facts onto the declaration shapes documentation consumes.
//!
//! C# facts already use the portable JVM identity grammar. Documentation models
//! data members as fields and types as classes, so properties become `FIELD`
//! declarations (identity prefix `field:`) and structs/delegates become `CLASS`.
//! CLR-specific detail stays in the projected fact; the signature erasure into
//! descriptor grammar is recorded as an explicit boundary.
use crate::error::{ClewError, ErrorCode};
use serde_json::{Value, json};

pub(super) const CSHARP_FACT_SCHEMA: &str = crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA;
pub(super) const CSHARP_FACTS_DOMAIN: &str =
    crate::csharp_adapter_v2::CSHARP_COMPILER_FACTS_CAPABILITY;

pub(super) fn project_facts(
    facts: Vec<(Value, String)>,
) -> Result<Vec<(Value, String)>, ClewError> {
    let mut output = Vec::with_capacity(facts.len() + 1);
    let mut declarations = 0usize;
    for (mut fact, binding) in facts {
        match fact["kind"].as_str() {
            Some("DECLARATION") => {
                let kind = match fact["declarationKind"].as_str() {
                    Some("PROPERTY") => "FIELD",
                    Some("STRUCT" | "DELEGATE") => "CLASS",
                    Some("EVENT" | "NAMESPACE") | None => continue,
                    Some(other) => other,
                }
                .to_owned();
                fact["declarationKind"] = json!(kind);
                if let Some(identity) = fact["symbolIdentity"].as_str() {
                    fact["symbolIdentity"] = json!(portable_member(identity));
                }
                declarations += 1;
                output.push((fact, binding));
            }
            Some("RELATION") => {
                if let Some(identity) = fact["sourceIdentity"].as_str() {
                    fact["sourceIdentity"] = json!(portable_member(identity));
                }
                output.push((fact, binding));
            }
            Some("BOUNDARY") => output.push((fact, binding)),
            _ => {}
        }
    }
    if declarations == 0 {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "C# compiler declarations are unavailable; restore the selected projects before documenting behavior",
        ));
    }
    output.push((
        json!({"kind":"BOUNDARY","code":"CLR_SIGNATURE_PROJECTED_TO_JVM_DESCRIPTOR_GRAMMAR"}),
        crate::canonical::hash_bytes(CSHARP_FACT_SCHEMA.as_bytes()),
    ));
    Ok(output)
}

/// One linear mapping per immutable compilation source, shared by all owners.
pub(super) struct SourceCoordinates(Vec<u64>);
impl SourceCoordinates {
    pub(super) fn new(text: &str) -> Self {
        let bom = usize::from(text.starts_with('\u{feff}')) * 3;
        let mut offsets = Vec::with_capacity(text.len() - bom + 1);
        for (byte, character) in text[bom..].char_indices() {
            offsets.push((byte + bom) as u64);
            if character.len_utf16() == 2 {
                offsets.push(u64::MAX);
            }
        }
        offsets.push(text.len() as u64);
        Self(offsets)
    }
    pub(super) fn valid_anchor(&self, anchor: &Value) -> bool {
        let (Some(start), Some(end), Some(bytes_start), Some(bytes_end)) = (
            anchor["start"].as_u64(),
            anchor["end"].as_u64(),
            anchor["byteStart"].as_u64(),
            anchor["byteEnd"].as_u64(),
        ) else {
            return false;
        };
        start < end
            && bytes_start < bytes_end
            && bytes_start != u64::MAX
            && bytes_end != u64::MAX
            && usize::try_from(start)
                .ok()
                .and_then(|start| self.0.get(start))
                .copied()
                == Some(bytes_start)
            && usize::try_from(end)
                .ok()
                .and_then(|end| self.0.get(end))
                .copied()
                == Some(bytes_end)
    }
}

/// Normalize compiler-owned Roslyn anchors into the shared original-byte contract.
/// `start/end` are decoded UTF-16; byte offsets include an optional UTF-8 BOM.
/// No source syntax is parsed and no call target or event is inferred here.
pub(super) fn bind_source_spans(
    fact: &mut Value,
    scope: &str,
    coordinates: &SourceCoordinates,
    full_digest: &str,
) -> Result<(), ClewError> {
    use super::{
        invalid,
        source_span::{EventSourceSpan, SPAN_SCHEMA},
    };
    if fact["schema"] != CSHARP_FACT_SCHEMA
        || fact["kind"] != "DECLARATION"
        || fact["resolution"] != "COMPILER_EXACT"
        || !coordinates.valid_anchor(fact)
        || fact["documentation"]["schema"] != "codeclew-csharp-documentation-flow/1.0"
        || fact["documentation"]["authority"] != "ROSLYN_SOURCE_STRUCTURE"
    {
        return Err(invalid(
            "Roslyn documentation owner does not bind original UTF-8 source",
        ));
    }
    let symbol = fact["symbolIdentity"]
        .as_str()
        .filter(|symbol| !symbol.is_empty())
        .ok_or_else(|| invalid("Roslyn documentation owner identity is missing"))?
        .to_owned();
    let file = fact["file"]
        .as_str()
        .ok_or_else(|| invalid("Roslyn documentation owner file is missing"))?
        .to_owned();
    let owner_start = fact["byteStart"].as_u64().unwrap() as usize;
    let owner_end = fact["byteEnd"].as_u64().unwrap() as usize;
    let events = fact["documentation"]["events"]
        .as_array_mut()
        .ok_or_else(|| invalid("Roslyn documentation event list is missing"))?;
    for (ordinal, event) in events.iter_mut().enumerate() {
        if !coordinates.valid_anchor(event) || event["file"] != file {
            return Err(invalid(
                "Roslyn documentation event does not bind original UTF-8 source",
            ));
        }
        let start = event["byteStart"].as_u64().unwrap() as usize;
        let end = event["byteEnd"].as_u64().unwrap() as usize;
        if start < owner_start || end > owner_end {
            return Err(invalid(
                "Roslyn documentation event is outside its compiler owner",
            ));
        }
        event["sourceSpan"] = serde_json::to_value(EventSourceSpan {
            schema: SPAN_SCHEMA.into(),
            coordinate_domain: "ORIGINAL_UTF8_BYTES".into(),
            owner_symbol_identity: symbol.clone(),
            compilation_scope: scope.into(),
            file: file.clone(),
            ordinal: ordinal as u64,
            owner_byte_start: owner_start,
            owner_byte_end: owner_end,
            byte_start: start,
            byte_end: end,
            full_compilation_source_digest: full_digest.into(),
        })
        .map_err(|_| invalid("Roslyn source span cannot be normalized"))?;
    }
    Ok(())
}

/// Properties are data members in documentation; their identity uses `field:`.
fn portable_member(identity: &str) -> String {
    match identity.strip_prefix("property:") {
        Some(rest) => format!("field:{rest}"),
        None => identity.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roslyn_spans_bind_bom_crlf_unicode_and_reject_utf16_byte_confusion() {
        let text = "\u{feff}// π🙂\r\nstring Render() { return \"α🙂\"; }\r\n";
        let owner_start = text.find("string").unwrap();
        let owner_end = text.find('}').unwrap() + 1;
        let start = text.find("return").unwrap();
        let end = text.find(';').unwrap() + 1;
        let utf16 = |byte| text[3..byte].encode_utf16().count();
        let owner = json!({"schema":CSHARP_FACT_SCHEMA,"kind":"DECLARATION",
            "resolution":"COMPILER_EXACT","symbolIdentity":"method:class:Api#Render()Ljava/lang/String;",
            "file":"Api.cs","start":utf16(owner_start),"end":utf16(owner_end),
            "byteStart":owner_start,"byteEnd":owner_end,
            "documentation":{"schema":"codeclew-csharp-documentation-flow/1.0","authority":"ROSLYN_SOURCE_STRUCTURE",
                "boundaries":[],"events":[{"kind":"RETURN","file":"Api.cs",
                    "start":utf16(start),"end":utf16(end),"byteStart":start,"byteEnd":end}]}});
        let mut normalized = owner.clone();
        let digest = crate::canonical::hash_bytes(text.as_bytes());
        bind_source_spans(
            &mut normalized,
            "api",
            &SourceCoordinates::new(text),
            &digest,
        )
        .unwrap();
        let raw = &normalized["documentation"]["events"][0]["sourceSpan"];
        assert!(super::super::source_span::validate_capture_span(
            raw,
            owner["symbolIdentity"].as_str().unwrap(),
            "api",
            0,
            &owner,
            Some((text, &digest))
        ));
        assert_eq!(raw["byteStart"], start);
        assert_eq!(&text[start..end], "return \"α🙂\";");
        for (key, value) in [
            ("start", json!(start)),
            ("byteEnd", json!(end - 2)),
            ("file", json!("other.cs")),
        ] {
            let mut bad = owner.clone();
            bad["documentation"]["events"][0][key] = value;
            assert!(
                bind_source_spans(&mut bad, "api", &SourceCoordinates::new(text), &digest).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn properties_become_fields_and_types_keep_portable_identities() {
        let facts = vec![
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"DECLARATION","declarationKind":"PROPERTY",
                    "symbolIdentity":"property:class:Orders.Dto#Id:I","ownerIdentity":"class:Orders.Dto","name":"Id"}),
                "a".to_owned(),
            ),
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"DECLARATION","declarationKind":"STRUCT",
                    "symbolIdentity":"class:Orders.Point","ownerIdentity":"module:unnamed","name":"Point"}),
                "b".to_owned(),
            ),
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"RELATION","relationKind":"CALLS",
                    "sourceIdentity":"property:class:Orders.Dto#Total:I","targetIdentity":"method:class:Orders.Dto#Sum()I"}),
                "c".to_owned(),
            ),
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"SOURCE_FILE","file":"Dto.cs"}),
                "d".to_owned(),
            ),
        ];
        let projected = project_facts(facts).unwrap();
        assert_eq!(projected[0].0["declarationKind"], "FIELD");
        assert_eq!(
            projected[0].0["symbolIdentity"],
            "field:class:Orders.Dto#Id:I"
        );
        assert_eq!(projected[1].0["declarationKind"], "CLASS");
        assert_eq!(
            projected[2].0["sourceIdentity"],
            "field:class:Orders.Dto#Total:I"
        );
        assert_eq!(
            projected.last().unwrap().0["code"],
            "CLR_SIGNATURE_PROJECTED_TO_JVM_DESCRIPTOR_GRAMMAR"
        );
        assert_eq!(projected.len(), 4);
        assert!(project_facts(vec![]).is_err());
    }
}
