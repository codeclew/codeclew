//! Bounded reads of one SOURCE retained by an immutable documentation Work.

use super::{
    bytes, digest, invalid,
    model::Source,
    work::{self, ReadState, Work},
};
use crate::canonical::hash_bytes;
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const REQUEST_SCHEMA: &str = "codeclew-documentation-source-part-request/1.0";
const RESPONSE_SCHEMA: &str = "codeclew-documentation-source-part/1.0";
const RECEIPT_SCHEMA: &str = "codeclew-documentation-source-part-receipt/1.0";
const CURSOR_VERSION: &str = "source-part-v1";
const RECEIPT_TYPE: &str = "SOURCE_PART";
const MAX_CURSOR_BYTES: usize = 256;
const MAX_READ_LEDGER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourcePartRequest {
    pub schema: String,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Typed receipt facts only; part text remains in the retained SOURCE record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourcePartReceipt {
    pub schema: String,
    pub work: String,
    pub snapshot: String,
    pub reference: String,
    pub source_id: String,
    pub record_digest: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub total_text_bytes: usize,
    pub fragment_digest: String,
    pub next_cursor: Option<String>,
    pub receipt_digest: String,
}

struct PartOutput {
    response: Value,
    receipt: SourcePartReceipt,
}

/// Read one source part from the exact saved snapshot held by Work.
///
/// This path never resolves a newer snapshot or acquires source evidence. A
/// successful response is recorded in the Work read ledger before it is
/// returned to the caller.
pub fn read_part(
    repo: &super::store::Repository,
    id: &str,
    request: SourcePartRequest,
) -> Result<Value, ClewError> {
    if request.schema != REQUEST_SCHEMA {
        return Err(invalid(
            "unsupported source-part request schema; expected codeclew-documentation-source-part-request/1.0",
        ));
    }
    if request.reference.is_empty() || request.reference.chars().count() > 256 {
        return Err(invalid(
            "source-part reference must contain 1..256 characters",
        ));
    }
    if request
        .cursor
        .as_deref()
        .is_some_and(|cursor| cursor.len() > MAX_CURSOR_BYTES)
    {
        return Err(invalid("source-part cursor exceeds its size limit"));
    }

    // Loading Work resolves only its validated manifest and immutable
    // snapshot; it does not consult latest-check or invoke source acquisition.
    let work = work::load(repo, id)?;
    let snapshot = work
        .snapshot
        .as_deref()
        .filter(|snapshot| !snapshot.is_empty())
        .ok_or_else(|| invalid("source-part reads require an immutable Work snapshot"))?;
    let source = source_for_reference(&work, &request.reference)?;
    let record_digest = digest(source)?;
    let start = match request.cursor.as_deref() {
        None => 0,
        Some(cursor) => parse_cursor(
            cursor,
            &work.id,
            snapshot,
            &request.reference,
            &source.id,
            &record_digest,
            &source.text,
        )?,
    };
    let part = build_part(&work, &request.reference, source, start)?;

    // Source lookup, parent validation, and sizing happen before the short
    // read-modify-write critical section. The existing repository lock is
    // intentionally non-blocking; callers can retry a conflict.
    let _lock = repo.lock()?;
    let mut state = work::read_state(repo, id)?;
    if state.work != id {
        return Err(invalid("read ledger belongs to another work"));
    }
    if let Some(existing) = state.source_part_receipts.get(&part.receipt.receipt_digest) {
        if existing != &part.receipt {
            return Err(invalid(
                "source-part receipt identity conflicts with saved evidence",
            ));
        }
    } else {
        state
            .source_part_receipts
            .insert(part.receipt.receipt_digest.clone(), part.receipt);
    }
    let encoded = bytes(&state)?;
    if encoded.len() > MAX_READ_LEDGER_BYTES {
        return Err(invalid(
            "work read ledger exceeds its bound; prepare narrower work",
        ));
    }
    repo.atomic(&format!("{}/reads.json", work::directory(id)?), &encoded)?;
    Ok(part.response)
}

fn source_for_reference<'a>(work: &'a Work, reference: &str) -> Result<&'a Source, ClewError> {
    let handle = work
        .handles
        .get(reference)
        .ok_or_else(|| invalid("unknown Work source reference"))?;
    if handle.kind != "SOURCE" {
        return Err(invalid("source-part reads require one SOURCE handle"));
    }
    let mut sources = work
        .checked
        .services
        .values()
        .filter_map(|service| service.sources.get(&handle.id));
    let source = sources
        .next()
        .ok_or_else(|| invalid("SOURCE handle is unavailable in the retained Work snapshot"))?;
    if sources.next().is_some() || source.id != handle.id {
        return Err(invalid(
            "SOURCE handle is ambiguous in the retained Work snapshot",
        ));
    }
    Ok(source)
}

fn source_metadata(source: &Source) -> Result<Value, ClewError> {
    let mut value = serde_json::to_value(source).map_err(super::io_error)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("retained SOURCE metadata is invalid"))?;
    object.remove("text");
    Ok(value)
}

fn cursor_for(
    work: &str,
    snapshot: &str,
    reference: &str,
    source_id: &str,
    record_digest: &str,
    offset: usize,
) -> Result<String, ClewError> {
    let binding = digest(&(
        CURSOR_VERSION,
        work,
        snapshot,
        reference,
        source_id,
        record_digest,
        offset,
    ))?;
    Ok(format!("{CURSOR_VERSION}:{offset}:{}", &binding[7..]))
}

fn parse_cursor(
    cursor: &str,
    work: &str,
    snapshot: &str,
    reference: &str,
    source_id: &str,
    record_digest: &str,
    text: &str,
) -> Result<usize, ClewError> {
    if cursor.len() > MAX_CURSOR_BYTES {
        return Err(invalid("source-part cursor exceeds its size limit"));
    }
    let (prefix, supplied_binding) = cursor
        .rsplit_once(':')
        .ok_or_else(|| invalid("invalid source-part cursor"))?;
    let (version, offset_text) = prefix
        .rsplit_once(':')
        .ok_or_else(|| invalid("invalid source-part cursor"))?;
    if version != CURSOR_VERSION || supplied_binding.len() != 64 {
        return Err(invalid("unsupported or malformed source-part cursor"));
    }
    let offset = offset_text
        .parse::<usize>()
        .map_err(|_| invalid("invalid source-part cursor offset"))?;
    if offset.to_string() != offset_text
        || offset == 0
        || offset >= text.len()
        || !text.is_char_boundary(offset)
        || cursor != cursor_for(work, snapshot, reference, source_id, record_digest, offset)?
    {
        return Err(invalid(
            "source-part cursor belongs to another Work, snapshot, source, or offset",
        ));
    }
    Ok(offset)
}

fn response_for_range(
    work: &Work,
    reference: &str,
    source: &Source,
    metadata: &Value,
    record_digest: &str,
    start: usize,
    end: usize,
) -> Result<Value, ClewError> {
    if start > end
        || end > source.text.len()
        || !source.text.is_char_boundary(start)
        || !source.text.is_char_boundary(end)
    {
        return Err(invalid("source-part byte range is invalid"));
    }
    let snapshot = work
        .snapshot
        .as_deref()
        .ok_or_else(|| invalid("source-part reads require an immutable Work snapshot"))?;
    let fragment = &source.text[start..end];
    let next_cursor = if end < source.text.len() {
        Some(cursor_for(
            &work.id,
            snapshot,
            reference,
            &source.id,
            record_digest,
            end,
        )?)
    } else {
        None
    };
    let mut response = json!({
        "schema": RESPONSE_SCHEMA,
        "work": work.id,
        "snapshot": snapshot,
        "reference": reference,
        "sourceId": source.id,
        "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
        "recordDigest": record_digest,
        "source": metadata,
        "startByte": start,
        "endByte": end,
        "totalTextBytes": source.text.len(),
        "text": fragment,
        "fragmentDigest": hash_bytes(fragment.as_bytes()),
        "nextCursor": next_cursor,
        "receiptType": RECEIPT_TYPE,
    });
    let receipt_digest = digest(&response)?;
    response["receiptDigest"] = json!(receipt_digest);
    Ok(response)
}

fn response_size(response: &Value) -> Result<usize, ClewError> {
    bytes(response)?
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("source-part response size overflow"))
}

fn build_part(
    work: &Work,
    reference: &str,
    source: &Source,
    start: usize,
) -> Result<PartOutput, ClewError> {
    let snapshot = work
        .snapshot
        .as_deref()
        .filter(|snapshot| !snapshot.is_empty())
        .ok_or_else(|| invalid("source-part reads require an immutable Work snapshot"))?;
    let record_digest = digest(source)?;
    let total = source.text.len();
    if start > total || !source.text.is_char_boundary(start) || (total > 0 && start == total) {
        return Err(invalid(
            "source-part cursor is outside the UTF-8 text range",
        ));
    }
    let metadata = source_metadata(source)?;
    let remaining_raw_bytes = total - start;
    let final_response = if remaining_raw_bytes <= work.request.max_bytes {
        Some(response_for_range(
            work,
            reference,
            source,
            &metadata,
            &record_digest,
            start,
            total,
        )?)
    } else {
        None
    };
    let final_response_fits = match final_response.as_ref() {
        Some(response) => response_size(response)? <= work.request.max_bytes,
        None => false,
    };
    let response = if final_response_fits {
        final_response.expect("checked as present")
    } else if total == 0 {
        return Err(no_progress_error());
    } else {
        let mut lower = start.saturating_add(1);
        let raw_upper = start.saturating_add(work.request.max_bytes);
        let mut upper = total.saturating_sub(1).min(raw_upper);
        let mut best: Option<(usize, Value)> = None;
        while lower <= upper {
            let middle = lower + (upper - lower) / 2;
            let mut end = middle;
            while end > start && !source.text.is_char_boundary(end) {
                end -= 1;
            }
            if end <= start {
                lower = middle.saturating_add(1);
                continue;
            }
            let candidate = response_for_range(
                work,
                reference,
                source,
                &metadata,
                &record_digest,
                start,
                end,
            )?;
            if response_size(&candidate)? <= work.request.max_bytes {
                best = Some((end, candidate));
                lower = middle.saturating_add(1);
            } else {
                if middle == 0 {
                    break;
                }
                upper = middle - 1;
            }
        }
        best.map(|(_, response)| response)
            .ok_or_else(no_progress_error)?
    };
    let end = response["endByte"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| invalid("source-part response has an invalid end offset"))?;
    let next_cursor = response["nextCursor"].as_str().map(str::to_owned);
    let receipt_digest = response["receiptDigest"]
        .as_str()
        .ok_or_else(|| invalid("source-part response has no receipt digest"))?
        .to_owned();
    let fragment_digest = response["fragmentDigest"]
        .as_str()
        .ok_or_else(|| invalid("source-part response has no fragment digest"))?
        .to_owned();
    let receipt = SourcePartReceipt {
        schema: RECEIPT_SCHEMA.into(),
        work: work.id.clone(),
        snapshot: snapshot.into(),
        reference: reference.into(),
        source_id: source.id.clone(),
        record_digest,
        start_byte: start,
        end_byte: end,
        total_text_bytes: total,
        fragment_digest,
        next_cursor,
        receipt_digest,
    };
    Ok(PartOutput { response, receipt })
}

fn no_progress_error() -> ClewError {
    invalid(
        "SOURCE_PART_NO_PROGRESS: retained SOURCE metadata and receipt fields exceed this Work byte budget; prepare a new Work against the same immutable snapshot with a larger maxBytes value (up to 49152); if the record still cannot fit, source-part reads do not support it",
    )
}

/// Return true only when validated receipts cover the exact SOURCE range.
pub(super) fn source_part_complete(
    work: &Work,
    state: &ReadState,
    reference: &str,
) -> Result<bool, ClewError> {
    if state.work != work.id
        || !state
            .source_part_receipts
            .values()
            .any(|receipt| receipt.reference == reference)
    {
        return Ok(false);
    }
    let Ok(source) = source_for_reference(work, reference) else {
        return Ok(false);
    };
    let Some(snapshot) = work.snapshot.as_deref() else {
        return Ok(false);
    };
    let record_digest = digest(source)?;
    let total = source.text.len();
    let metadata = source_metadata(source)?;
    let mut ranges = Vec::new();
    for (key, receipt) in &state.source_part_receipts {
        if receipt.reference != reference {
            continue;
        }
        if key != &receipt.receipt_digest
            || receipt.schema != RECEIPT_SCHEMA
            || receipt.work != work.id
            || receipt.snapshot != snapshot
            || receipt.source_id != source.id
            || receipt.record_digest != record_digest
            || receipt.total_text_bytes != total
            || receipt.start_byte > receipt.end_byte
            || receipt.end_byte > total
            || !source.text.is_char_boundary(receipt.start_byte)
            || !source.text.is_char_boundary(receipt.end_byte)
            || (total > 0 && receipt.start_byte == receipt.end_byte)
        {
            return Ok(false);
        }
        let fragment = &source.text[receipt.start_byte..receipt.end_byte];
        if hash_bytes(fragment.as_bytes()) != receipt.fragment_digest {
            return Ok(false);
        }
        let response = response_for_range(
            work,
            reference,
            source,
            &metadata,
            &record_digest,
            receipt.start_byte,
            receipt.end_byte,
        )?;
        if response["receiptDigest"].as_str() != Some(receipt.receipt_digest.as_str())
            || response["nextCursor"].as_str() != receipt.next_cursor.as_deref()
            || response_size(&response)? > work.request.max_bytes
        {
            return Ok(false);
        }
        ranges.push((receipt.start_byte, receipt.end_byte));
    }
    if ranges.is_empty() {
        return Ok(false);
    }
    ranges.sort_unstable();
    if total == 0 {
        return Ok(ranges == [(0, 0)]);
    }
    let mut covered = 0;
    for (start, end) in ranges {
        if start != covered {
            return Ok(false);
        }
        covered = end;
    }
    Ok(covered == total)
}

pub(super) fn completed_source_references(
    work: &Work,
    state: &ReadState,
) -> Result<std::collections::BTreeSet<String>, ClewError> {
    if state.source_part_receipts.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    let candidates: std::collections::BTreeSet<_> = state
        .source_part_receipts
        .values()
        .map(|receipt| receipt.reference.as_str())
        .collect();
    let mut completed = std::collections::BTreeSet::new();
    for reference in candidates {
        if work
            .handles
            .get(reference)
            .is_some_and(|handle| handle.kind == "SOURCE")
            && source_part_complete(work, state, reference)?
        {
            completed.insert(reference.to_owned());
        }
    }
    Ok(completed)
}

pub(super) fn initial_context_complete_with_parts(
    work: &Work,
    state: &ReadState,
) -> Result<bool, ClewError> {
    if state.work != work.id {
        return Ok(false);
    }
    let completed = completed_source_references(work, state)?;
    let mut cursor: Option<String> = None;
    let mut membership: Option<String> = None;
    for _ in 0..=state.receipts.len() {
        let Some(receipt) = state.receipts.values().find(|receipt| {
            receipt.selection.references.is_empty()
                && receipt.selection.symbols.is_empty()
                && receipt.selection.query.is_none()
                && receipt.selection.cursor == cursor
        }) else {
            return Ok(false);
        };
        if membership
            .as_deref()
            .is_some_and(|digest| digest != receipt.membership_digest)
        {
            return Ok(false);
        }
        if receipt.omitted.iter().any(|omitted| {
            if omitted["kind"] != "SOURCE" {
                return true;
            }
            let (Some(reference), Some(id)) =
                (omitted["reference"].as_str(), omitted["id"].as_str())
            else {
                return true;
            };
            !work.handles.get(reference).is_some_and(|handle| {
                handle.kind == "SOURCE" && handle.id == id && completed.contains(reference)
            })
        }) {
            return Ok(false);
        }
        membership = Some(receipt.membership_digest.clone());
        let Some(next_cursor) = receipt.next_cursor.clone() else {
            return Ok(true);
        };
        if cursor.as_deref() == Some(next_cursor.as_str()) {
            return Ok(false);
        }
        cursor = Some(next_cursor);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::{
        check::Check,
        model::{ServiceEvidence, SourceOccurrence},
        work::{Handle, ReadReceipt, ReadState, Request, Selection},
    };
    use std::collections::BTreeMap;

    fn fixture(text: String, max_bytes: usize) -> (Work, Source) {
        let source = Source {
            id: "source-one".into(),
            service: "orders".into(),
            revision: "revision-a".into(),
            file: "src/orders.rs".into(),
            start_line: 1,
            end_line: 1,
            text,
            text_digest: "sha256:stored-text-digest".into(),
            evidence_digest: "sha256:stored-evidence-digest".into(),
            authority: "CAPTURED_SOURCE".into(),
            occurrence: Some(SourceOccurrence {
                snapshot: "sha256:occurrence/1".into(),
                blob: "sha256:blob".into(),
                start_byte: 0,
                end_byte: 1,
            }),
            url: None,
        };
        let service = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "orders".into(),
            revision: "revision-a".into(),
            service_digest: "sha256:service".into(),
            extractor: "test".into(),
            runtime_mode: "TEST".into(),
            coverage: "COMPLETE".into(),
            boundaries: Vec::new(),
            entrypoints: Vec::new(),
            observations: BTreeMap::new(),
            sources: BTreeMap::from([(source.id.clone(), source.clone())]),
            contracts: BTreeMap::new(),
        };
        let checked = Check {
            schema: "codeclew-documentation-check/1.0".into(),
            input_digest: "sha256:input".into(),
            context_digest: "sha256:context".into(),
            services: BTreeMap::from([("orders".into(), service)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let work = Work {
            schema: "codeclew-documentation-work/1.0".into(),
            id: "a".repeat(64),
            subject: "service:orders".into(),
            request: Request {
                schema: "codeclew-documentation-work-request/1.0".into(),
                audience: "Maintainers".into(),
                documentation_language: None,
                entrypoint: None,
                context_profile: None,
                max_items: 20,
                max_bytes,
                external_inputs: Vec::new(),
            },
            checked,
            snapshot: Some("sha256:snapshot/100".into()),
            retained: None,
            external_inputs: BTreeMap::new(),
            handles: BTreeMap::from([(
                "s1".into(),
                Handle {
                    kind: "SOURCE".into(),
                    id: source.id.clone(),
                },
            )]),
            influence: BTreeMap::new(),
            obligations: Vec::new(),
            review_reasons: Vec::new(),
        };
        (work, source)
    }

    #[test]
    fn bounded_utf8_parts_reconstruct_the_exact_retained_source() {
        let text = format!(
            "first line\nquote: \"hello\"\\path\nCyrillic: {}; emoji: 🧭🚀\n{}tail λ",
            "\u{041f}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}",
            "long-source-line-".repeat(320)
        );
        let (work, source) = fixture(text, 2048);
        let metadata = source_metadata(&source).unwrap();
        let record_digest = digest(&source).unwrap();
        let mut start = 0;
        let mut rebuilt = String::new();
        let mut first_response: Option<Value> = None;
        let mut part_count = 0;
        loop {
            let part = build_part(&work, "s1", &source, start).unwrap();
            let response_bytes = bytes(&part.response).unwrap().len() + 1;
            assert!(response_bytes <= work.request.max_bytes);
            let response_start = part.response["startByte"].as_u64().unwrap() as usize;
            let response_end = part.response["endByte"].as_u64().unwrap() as usize;
            assert_eq!(response_start, start);
            assert!(source.text.is_char_boundary(response_start));
            assert!(source.text.is_char_boundary(response_end));
            assert_eq!(part.response["recordDigest"], record_digest);
            let fragment = part.response["text"].as_str().unwrap();
            assert_eq!(
                part.response["fragmentDigest"],
                hash_bytes(fragment.as_bytes())
            );
            rebuilt.push_str(fragment);
            part_count += 1;
            if first_response.is_none() {
                first_response = Some(part.response.clone());
            }
            if let Some(cursor) = part.receipt.next_cursor.as_deref() {
                start = parse_cursor(
                    cursor,
                    &work.id,
                    work.snapshot.as_deref().unwrap(),
                    "s1",
                    &source.id,
                    &record_digest,
                    &source.text,
                )
                .unwrap();
            } else {
                assert_eq!(part.receipt.end_byte, source.text.len());
                assert_eq!(part.receipt.total_text_bytes, source.text.len());
                let mut reconstructed = metadata.clone();
                reconstructed["text"] = json!(rebuilt);
                assert_eq!(
                    crate::canonical::bytes(&reconstructed).unwrap(),
                    crate::canonical::bytes(&source).unwrap()
                );
                let first = first_response.unwrap();
                assert_eq!(first["source"]["occurrence"]["blob"], "sha256:blob");
                assert!(first["source"].get("text").is_none());
                assert_eq!(first["source"]["url"], Value::Null);
                assert_eq!(first["source"]["textDigest"], source.text_digest);
                assert_eq!(first["source"]["evidenceDigest"], source.evidence_digest);
                break;
            }
        }
        assert!(part_count > 1);
        assert_eq!(rebuilt, source.text);
    }

    #[test]
    fn cursor_validation_binds_identity_and_rejects_old_or_invalid_offsets() {
        let (work, source) = fixture("a🧭z".into(), 2048);
        let snapshot = work.snapshot.as_deref().unwrap();
        let record_digest = digest(&source).unwrap();
        let valid = cursor_for(&work.id, snapshot, "s1", &source.id, &record_digest, 1).unwrap();
        let parse = |cursor: &str| {
            parse_cursor(
                cursor,
                &work.id,
                snapshot,
                "s1",
                &source.id,
                &record_digest,
                &source.text,
            )
        };
        assert_eq!(parse(&valid).unwrap(), 1);

        let foreign_work = cursor_for(
            &"b".repeat(64),
            snapshot,
            "s1",
            &source.id,
            &record_digest,
            1,
        )
        .unwrap();
        let foreign_snapshot = cursor_for(
            &work.id,
            "sha256:other-snapshot/1",
            "s1",
            &source.id,
            &record_digest,
            1,
        )
        .unwrap();
        let foreign_reference =
            cursor_for(&work.id, snapshot, "s2", &source.id, &record_digest, 1).unwrap();
        let foreign_source = cursor_for(
            &work.id,
            snapshot,
            "s1",
            "another-source",
            &record_digest,
            1,
        )
        .unwrap();
        let foreign_record = cursor_for(
            &work.id,
            snapshot,
            "s1",
            &source.id,
            "sha256:another-record",
            1,
        )
        .unwrap();
        let middle_of_codepoint =
            cursor_for(&work.id, snapshot, "s1", &source.id, &record_digest, 2).unwrap();
        let at_end = cursor_for(
            &work.id,
            snapshot,
            "s1",
            &source.id,
            &record_digest,
            source.text.len(),
        )
        .unwrap();
        let zero = cursor_for(&work.id, snapshot, "s1", &source.id, &record_digest, 0).unwrap();
        let overflow = format!("{CURSOR_VERSION}:{}:{}", "9".repeat(128), "0".repeat(64));
        let binding = valid.rsplit(':').next().unwrap();
        let old_page_cursor = format!("work-page-v1:1:{binding}");
        let noncanonical_offset = format!("{CURSOR_VERSION}:01:{binding}");
        let oversized_cursor = "c".repeat(MAX_CURSOR_BYTES + 1);
        for invalid_cursor in [
            "",
            "malformed",
            old_page_cursor.as_str(),
            noncanonical_offset.as_str(),
            overflow.as_str(),
            oversized_cursor.as_str(),
            foreign_work.as_str(),
            foreign_snapshot.as_str(),
            foreign_reference.as_str(),
            foreign_source.as_str(),
            foreign_record.as_str(),
            middle_of_codepoint.as_str(),
            at_end.as_str(),
            zero.as_str(),
        ] {
            assert!(
                parse(invalid_cursor).is_err(),
                "accepted {invalid_cursor:?}"
            );
        }
    }

    #[test]
    fn full_response_size_boundary_counts_the_trailing_newline() {
        let (mut work, source) = fixture("A".repeat(1800), 49_152);
        let metadata = source_metadata(&source).unwrap();
        let record_digest = digest(&source).unwrap();
        let full = response_for_range(
            &work,
            "s1",
            &source,
            &metadata,
            &record_digest,
            0,
            source.text.len(),
        )
        .unwrap();
        let exact_size = response_size(&full).unwrap();
        assert_eq!(exact_size, bytes(&full).unwrap().len() + 1);
        assert!(exact_size > 2048);

        work.request.max_bytes = exact_size - 1;
        let below = build_part(&work, "s1", &source, 0).unwrap();
        assert!(response_size(&below.response).unwrap() < exact_size);
        assert!(below.receipt.end_byte < source.text.len());
        assert!(below.receipt.next_cursor.is_some());

        work.request.max_bytes = exact_size;
        let exact = build_part(&work, "s1", &source, 0).unwrap();
        assert_eq!(response_size(&exact.response).unwrap(), exact_size);
        assert_eq!(exact.receipt.end_byte, source.text.len());
        assert!(exact.receipt.next_cursor.is_none());

        work.request.max_bytes = exact_size + 1;
        let above = build_part(&work, "s1", &source, 0).unwrap();
        assert_eq!(response_size(&above.response).unwrap(), exact_size);
    }

    fn collect_receipts(work: &Work, source: &Source) -> Vec<SourcePartReceipt> {
        let record_digest = digest(source).unwrap();
        let mut start = 0;
        let mut receipts = Vec::new();
        loop {
            let part = build_part(work, "s1", source, start).unwrap();
            receipts.push(part.receipt.clone());
            match part.receipt.next_cursor.as_deref() {
                Some(cursor) => {
                    start = parse_cursor(
                        cursor,
                        &work.id,
                        work.snapshot.as_deref().unwrap(),
                        "s1",
                        &source.id,
                        &record_digest,
                        &source.text,
                    )
                    .unwrap();
                }
                None => break,
            }
        }
        receipts
    }

    fn state_with_parts(
        work: &Work,
        receipts: impl IntoIterator<Item = SourcePartReceipt>,
    ) -> ReadState {
        ReadState {
            work: work.id.clone(),
            receipts: BTreeMap::new(),
            untracked_reads: false,
            source_part_receipts: receipts
                .into_iter()
                .map(|receipt| (receipt.receipt_digest.clone(), receipt))
                .collect(),
        }
    }

    fn custom_receipt(work: &Work, source: &Source, start: usize, end: usize) -> SourcePartReceipt {
        let metadata = source_metadata(source).unwrap();
        let record_digest = digest(source).unwrap();
        let response =
            response_for_range(work, "s1", source, &metadata, &record_digest, start, end).unwrap();
        SourcePartReceipt {
            schema: RECEIPT_SCHEMA.into(),
            work: work.id.clone(),
            snapshot: work.snapshot.clone().unwrap(),
            reference: "s1".into(),
            source_id: source.id.clone(),
            record_digest,
            start_byte: start,
            end_byte: end,
            total_text_bytes: source.text.len(),
            fragment_digest: response["fragmentDigest"].as_str().unwrap().into(),
            next_cursor: response["nextCursor"].as_str().map(str::to_owned),
            receipt_digest: response["receiptDigest"].as_str().unwrap().into(),
        }
    }

    #[test]
    fn part_receipt_coverage_requires_explicit_gap_free_nonoverlapping_ranges() {
        let (work, source) = fixture("x".repeat(9000), 2048);
        let receipts = collect_receipts(&work, &source);
        assert!(receipts.len() >= 3);
        assert!(
            source_part_complete(&work, &state_with_parts(&work, receipts.clone()), "s1").unwrap()
        );
        let mut reverse_order = receipts.clone();
        reverse_order.reverse();
        assert!(
            source_part_complete(&work, &state_with_parts(&work, reverse_order), "s1").unwrap()
        );
        assert!(
            !source_part_complete(
                &work,
                &state_with_parts(&work, receipts[..1].iter().cloned()),
                "s1"
            )
            .unwrap()
        );
        assert!(
            !source_part_complete(
                &work,
                &state_with_parts(&work, receipts[receipts.len() - 1..].iter().cloned()),
                "s1"
            )
            .unwrap()
        );
        assert!(
            !source_part_complete(
                &work,
                &state_with_parts(&work, receipts[..receipts.len() - 1].iter().cloned()),
                "s1"
            )
            .unwrap()
        );
        let middle = receipts.len() / 2;
        let without_middle = receipts
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != middle)
            .map(|(_, receipt)| receipt.clone());
        assert!(
            !source_part_complete(&work, &state_with_parts(&work, without_middle), "s1").unwrap()
        );

        let overlap = custom_receipt(&work, &source, 1, 2);
        let mut overlapping = receipts.clone();
        overlapping.push(overlap);
        assert!(!source_part_complete(&work, &state_with_parts(&work, overlapping), "s1").unwrap());

        let mut corrupted = receipts.clone();
        corrupted[0].fragment_digest = "sha256:wrong-fragment".into();
        assert!(!source_part_complete(&work, &state_with_parts(&work, corrupted), "s1").unwrap());
        let mut conflicting = receipts.clone();
        conflicting[0].work = "b".repeat(64);
        assert!(!source_part_complete(&work, &state_with_parts(&work, conflicting), "s1").unwrap());

        let mut mismatched_key = state_with_parts(&work, receipts.clone());
        let first = receipts[0].clone();
        mismatched_key
            .source_part_receipts
            .remove(&first.receipt_digest);
        mismatched_key
            .source_part_receipts
            .insert("sha256:wrong-key".into(), first);
        assert!(!source_part_complete(&work, &mismatched_key, "s1").unwrap());
    }

    #[test]
    fn empty_source_requires_an_explicit_empty_receipt_and_legacy_state_shape_is_stable() {
        let (work, source) = fixture(String::new(), 2048);
        let empty = state_with_parts(&work, []);
        assert!(!source_part_complete(&work, &empty, "s1").unwrap());
        let part = build_part(&work, "s1", &source, 0).unwrap();
        assert_eq!(part.receipt.start_byte, 0);
        assert_eq!(part.receipt.end_byte, 0);
        assert!(part.receipt.next_cursor.is_none());
        assert!(
            source_part_complete(&work, &state_with_parts(&work, [part.receipt]), "s1").unwrap()
        );

        let old_shape = json!({"work":"old-work","receipts":{},"untrackedReads":false});
        let old_state: ReadState = serde_json::from_value(old_shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(&old_state).unwrap(), old_shape);
        assert_eq!(
            crate::canonical::bytes(&old_state).unwrap(),
            crate::canonical::bytes(&old_shape).unwrap()
        );
        assert_eq!(
            crate::canonical::hash(&old_state).unwrap(),
            crate::canonical::hash(&old_shape).unwrap()
        );
        let mut state = old_state;
        assert!(state.source_part_receipts.is_empty());
        state.receipts.insert(
            "ordinary".into(),
            ReadReceipt {
                selection: Selection::default(),
                result_digest: "sha256:result".into(),
                supplied: Vec::new(),
                membership_digest: "sha256:membership".into(),
                omitted: Vec::new(),
                next_cursor: None,
            },
        );
        let mut expected = old_shape;
        expected["receipts"] = json!({"ordinary": {
            "selection": {"references": [], "symbols": [], "query": null, "cursor": null, "untrackedReads": false},
            "resultDigest":"sha256:result",
            "supplied": [],
            "membershipDigest":"sha256:membership",
            "omitted": [],
            "nextCursor": null
        }});
        assert_eq!(serde_json::to_value(&state).unwrap(), expected);
    }

    #[test]
    fn metadata_that_cannot_fit_returns_no_progress_without_a_receipt() {
        let (work, mut source) = fixture(String::new(), 2048);
        source.file = format!("generated/{}.java", "x".repeat(4096));
        let error = build_part(&work, "s1", &source, 0)
            .err()
            .expect("oversized metadata must not produce a response");
        assert!(error.message.contains("SOURCE_PART_NO_PROGRESS"));
        assert!(error.message.contains("prepare a new Work"));
        let empty = state_with_parts(&work, []);
        assert!(!source_part_complete(&work, &empty, "s1").unwrap());
    }

    #[test]
    fn only_the_exact_fully_delivered_source_omission_resolves_initial_context() {
        let (work, source) = fixture("x".repeat(6000), 2048);
        let omitted_source = json!({
            "kind":"SOURCE",
            "id": source.id.clone(),
            "reference":"s1",
            "reason":"ITEM_EXCEEDS_WORK_BYTE_BUDGET"
        });
        let omitted_other_source = json!({
            "kind":"SOURCE",
            "id":"another-source",
            "reference":"s2",
            "reason":"ITEM_EXCEEDS_WORK_BYTE_BUDGET"
        });
        let omitted_external_input = json!({
            "kind":"EXTERNAL_INPUT",
            "id":"notes/large.md",
            "reference":null,
            "reason":"ITEM_EXCEEDS_WORK_BYTE_BUDGET"
        });
        let initial_page = ReadReceipt {
            selection: Selection::default(),
            result_digest: "sha256:page-result".into(),
            supplied: Vec::new(),
            membership_digest: "sha256:page-membership".into(),
            omitted: vec![
                omitted_source.clone(),
                omitted_other_source.clone(),
                omitted_external_input.clone(),
            ],
            next_cursor: None,
        };
        let mut state = state_with_parts(&work, []);
        state
            .receipts
            .insert("initial".into(), initial_page.clone());
        assert!(!initial_context_complete_with_parts(&work, &state).unwrap());

        let receipts = collect_receipts(&work, &source);
        state.source_part_receipts =
            state_with_parts(&work, receipts[..1].iter().cloned()).source_part_receipts;
        assert!(!initial_context_complete_with_parts(&work, &state).unwrap());

        state.source_part_receipts = state_with_parts(&work, receipts.clone()).source_part_receipts;
        assert!(!initial_context_complete_with_parts(&work, &state).unwrap());

        let mut exact_source_only = state.clone();
        exact_source_only
            .receipts
            .get_mut("initial")
            .unwrap()
            .omitted = vec![omitted_source];
        assert!(initial_context_complete_with_parts(&work, &exact_source_only).unwrap());

        let mut with_other_source = exact_source_only.clone();
        with_other_source
            .receipts
            .get_mut("initial")
            .unwrap()
            .omitted
            .push(omitted_other_source);
        assert!(!initial_context_complete_with_parts(&work, &with_other_source).unwrap());

        let mut with_external_input = exact_source_only.clone();
        with_external_input
            .receipts
            .get_mut("initial")
            .unwrap()
            .omitted
            .push(omitted_external_input);
        assert!(!initial_context_complete_with_parts(&work, &with_external_input).unwrap());

        let mut wrong_id = exact_source_only.clone();
        wrong_id.receipts.get_mut("initial").unwrap().omitted[0]["id"] = json!("another-source");
        assert!(!initial_context_complete_with_parts(&work, &wrong_id).unwrap());
        let mut wrong_kind = exact_source_only;
        wrong_kind.receipts.get_mut("initial").unwrap().omitted[0]["kind"] = json!("DEPENDENCY");
        assert!(!initial_context_complete_with_parts(&work, &wrong_kind).unwrap());

        let without_page = state_with_parts(&work, receipts);
        assert!(!initial_context_complete_with_parts(&work, &without_page).unwrap());
    }
}
