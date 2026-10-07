//! Shared optional original-byte coordinates for retained documentation events.
use super::model::{Observation, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SPAN_SCHEMA: &str = "codeclew-documentation-source-span/1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventSourceSpan {
    pub schema: String,
    pub coordinate_domain: String,
    pub owner_symbol_identity: String,
    pub compilation_scope: String,
    pub file: String,
    pub ordinal: u64,
    pub owner_byte_start: usize,
    pub owner_byte_end: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub full_compilation_source_digest: String,
}

pub fn validate_capture_span(
    raw: &Value,
    symbol: &str,
    scope: &str,
    ordinal: u64,
    owner: &Value,
    source: Option<(&str, &str)>,
) -> bool {
    let (Ok(span), Some((source, full_source_digest))) = (
        serde_json::from_value::<EventSourceSpan>(raw.clone()),
        source,
    ) else {
        return false;
    };
    span.schema == SPAN_SCHEMA
        && span.coordinate_domain == "ORIGINAL_UTF8_BYTES"
        && span.owner_symbol_identity == symbol
        && span.compilation_scope == scope
        && span.ordinal == ordinal
        && owner["file"] == span.file
        && owner["byteStart"]
            .as_u64()
            .or_else(|| owner["start"].as_u64())
            == Some(span.owner_byte_start as u64)
        && owner["byteEnd"].as_u64().or_else(|| owner["end"].as_u64())
            == Some(span.owner_byte_end as u64)
        && span.owner_byte_start <= span.byte_start
        && span.byte_start < span.byte_end
        && span.byte_end <= span.owner_byte_end
        && source
            .get(span.owner_byte_start..span.owner_byte_end)
            .is_some()
        && source.get(span.byte_start..span.byte_end).is_some()
        && span.full_compilation_source_digest == full_source_digest
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactSourceSpan {
    pub schema: String,
    pub compilation_byte_start: usize,
    pub compilation_byte_end: usize,
    pub full_compilation_source_digest: String,
    pub expression: String,
}

fn canonical_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn valid_source_binding(source: &Source, owner: &Source) -> bool {
    source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
        && owner.text_digest == crate::canonical::hash_bytes(owner.text.as_bytes())
        && canonical_sha256(&source.evidence_digest)
        && !source.authority.is_empty()
        && source.occurrence.as_ref().is_none_or(|range| {
            range.end_byte.checked_sub(range.start_byte) == Some(source.text.len())
        })
        && source.service == owner.service
        && source.revision == owner.revision
        && source.file == owner.file
        && source.authority == owner.authority
        && source.evidence_digest == owner.evidence_digest
        && !source.text.is_empty()
        && source.start_line >= owner.start_line
        && source.end_line <= owner.end_line
        && source.end_line >= source.start_line
        && source.end_line - source.start_line + 1 == source.text.lines().count() as u64
}

/// Original-byte source admission is independent of semantic outline support.
pub fn retained_event_source<'a>(
    owner: &Observation,
    scope: &str,
    ordinal: u64,
    documented: &Value,
    flow: &Observation,
    owner_source: &Source,
    sources: &'a std::collections::BTreeMap<String, Source>,
) -> Option<(&'a Source, ExactSourceSpan)> {
    let span: EventSourceSpan =
        serde_json::from_value(documented.get("sourceSpan")?.clone()).ok()?;
    let owner_site = &owner.normalized["outlineOwnerSource"];
    let site = &flow.normalized["outlineSite"];
    let [source_id] = flow.source_ids.as_slice() else {
        return None;
    };
    let source = sources
        .get(source_id)
        .filter(|source| source.id == *source_id)?;
    let owner_start = usize::try_from(owner_site["byteStart"].as_u64()?).ok()?;
    let owner_end = usize::try_from(owner_site["byteEnd"].as_u64()?).ok()?;
    let relative_start = span.byte_start.checked_sub(owner_start)?;
    let relative_end = span.byte_end.checked_sub(owner_start)?;
    let exact = owner_source.text.get(relative_start..relative_end)?;
    if span.schema != SPAN_SCHEMA
        || span.coordinate_domain != "ORIGINAL_UTF8_BYTES"
        || span.owner_symbol_identity != owner.symbol
        || span.compilation_scope != scope
        || span.ordinal != ordinal
        || span.file != owner_source.file
        || span.owner_byte_start != owner_start
        || span.owner_byte_end != owner_end
        || span.byte_start >= span.byte_end
        || span.byte_end > owner_end
        || owner_end.checked_sub(owner_start) != Some(owner_source.text.len())
        || owner_site["sourceStatus"] != "SOURCE_RETAINED"
        || owner_site["sourceId"] != owner_source.id
        || owner_site["sourceDigest"] != owner_source.text_digest
        || owner_site["evidenceDigest"] != owner_source.evidence_digest
        || owner_site["file"] != owner_source.file
        || owner_site["fullCompilationSourceDigest"] != span.full_compilation_source_digest
        || !canonical_sha256(&span.full_compilation_source_digest)
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || site["sourceId"] != source.id
        || site["file"] != span.file
        || site["byteStart"].as_u64() != Some(span.byte_start as u64)
        || site["byteEnd"].as_u64() != Some(span.byte_end as u64)
        || site["sourceDigest"] != source.text_digest
        || site["evidenceDigest"] != source.evidence_digest
        || site["fullCompilationSourceDigest"] != span.full_compilation_source_digest
        || site["startLine"] != source.start_line
        || site["endLine"] != source.end_line
        || !valid_source_binding(source, owner_source)
        || exact != source.text
        || source.start_line
            != owner_source.start_line
                + owner_source.text.as_bytes()[..relative_start]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count() as u64
        || source.end_line
            != owner_source.start_line
                + owner_source.text.as_bytes()[..relative_end - 1]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count() as u64
    {
        return None;
    }
    Some((
        source,
        ExactSourceSpan {
            schema: SPAN_SCHEMA.into(),
            compilation_byte_start: span.byte_start,
            compilation_byte_end: span.byte_end,
            full_compilation_source_digest: span.full_compilation_source_digest,
            expression: source.text.clone(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn common_span_contract_binds_original_bytes_without_language_or_parser_assumptions() {
        let text = "// π🙂\r\nlet chosen = pick(\"α🙂\");\r\n";
        let start = text.find("pick").unwrap();
        let end = start + "pick(\"α🙂\")".len();
        let owner = json!({"file":"source.txt","byteStart":0,"byteEnd":text.len()});
        let raw = json!({"schema":SPAN_SCHEMA,"coordinateDomain":"ORIGINAL_UTF8_BYTES",
            "ownerSymbolIdentity":"opaque/compiler/owner","compilationScope":"main","file":"source.txt",
            "ordinal":3,"ownerByteStart":0,"ownerByteEnd":text.len(),"byteStart":start,"byteEnd":end,
            "fullCompilationSourceDigest":crate::canonical::hash_bytes(text.as_bytes())});
        let digest = crate::canonical::hash_bytes(text.as_bytes());
        assert!(validate_capture_span(
            &raw,
            "opaque/compiler/owner",
            "main",
            3,
            &owner,
            Some((text, &digest))
        ));
        for (key, value) in [
            ("ownerSymbolIdentity", json!("wrong")),
            ("compilationScope", json!("other")),
            ("ordinal", json!(0)),
            ("fullCompilationSourceDigest", json!("wrong")),
            ("coordinateDomain", json!("UTF16")),
            ("file", json!("other.txt")),
            ("byteStart", json!(text.find('α').unwrap() + 1)),
            ("byteEnd", json!(text.len() + 1)),
        ] {
            let mut bad = raw.clone();
            bad[key] = value;
            assert!(
                !validate_capture_span(
                    &bad,
                    "opaque/compiler/owner",
                    "main",
                    3,
                    &owner,
                    Some((text, &digest))
                ),
                "{key}"
            );
        }
        let mut unknown = raw;
        unknown["unrecognized"] = json!(true);
        assert!(!validate_capture_span(
            &unknown,
            "opaque/compiler/owner",
            "main",
            3,
            &owner,
            Some((text, &digest))
        ));
    }
}
