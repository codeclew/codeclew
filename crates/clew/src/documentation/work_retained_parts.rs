//! Bounded canonical JSON reads of an immutable Work's retained operations.
//!
//! Retained authoring is context, not freshly verified source evidence. These
//! receipts satisfy only the exact retained-operation omission in manual Work
//! reads; automatic author/reviewer packets must establish their own delivery.

use super::{
    bytes, digest, invalid,
    model::Operation,
    store::Repository,
    work::{self, ReadState, Work},
};
use crate::{canonical::hash_bytes, error::ClewError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) const REQUEST_SCHEMA: &str = "codeclew-documentation-retained-part-request/1.0";
const RESPONSE_SCHEMA: &str = "codeclew-documentation-retained-part/1.0";
const RECEIPT_SCHEMA: &str = "codeclew-documentation-retained-part-receipt/1.0";
pub(super) const RECORD_KIND: &str = "RETAINED_OPERATION";
const CURSOR_VERSION: &str = "retained-part-v1";
const MAX_CURSOR_BYTES: usize = 256;
const MAX_READ_LEDGER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetainedPartRequest {
    pub schema: String,
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Only receipt facts are persisted; fragments remain in the immutable Work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetainedPartReceipt {
    pub schema: String,
    pub work: String,
    pub snapshot: String,
    pub kind: String,
    pub id: String,
    pub record_digest: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub total_record_bytes: usize,
    pub fragment_digest: String,
    pub next_cursor: Option<String>,
    pub receipt_digest: String,
}

struct Record<'a> {
    work: &'a Work,
    snapshot: &'a str,
    id: &'a str,
    digest: String,
    text: String,
}

/// Resolve exactly one retained operation without following a newer baseline.
pub(super) fn retained_operation<'a>(work: &'a Work, id: &str) -> Result<&'a Operation, ClewError> {
    if id.is_empty() || id.chars().count() > 256 {
        return Err(invalid(
            "retained operation identity must contain 1..256 characters",
        ));
    }
    if work
        .request
        .entrypoint
        .as_deref()
        .is_some_and(|selected| selected != id)
    {
        return Err(invalid(
            "retained operation is outside the Work's selected entrypoint",
        ));
    }
    let narrative = work
        .retained
        .as_ref()
        .filter(|n| n.subject == work.subject)
        .ok_or_else(|| invalid("Work has no retained narrative for this subject"))?;
    let mut matching = narrative
        .operations
        .iter()
        .filter(|operation| operation.id == id);
    let operation = matching
        .next()
        .ok_or_else(|| invalid("unknown retained operation in this immutable Work"))?;
    if matching.next().is_some() {
        return Err(invalid(
            "retained operation identity is ambiguous in this Work",
        ));
    }
    Ok(operation)
}

pub(super) fn retained_record_digest(work: &Work, id: &str) -> Result<String, ClewError> {
    digest(retained_operation(work, id)?)
}

impl<'a> Record<'a> {
    fn new(work: &'a Work, id: &'a str) -> Result<Self, ClewError> {
        let snapshot = work
            .snapshot
            .as_deref()
            .filter(|snapshot| !snapshot.is_empty())
            .ok_or_else(|| invalid("retained-part reads require an immutable Work snapshot"))?;
        let encoded = bytes(retained_operation(work, id)?)?;
        let text = String::from_utf8(encoded).map_err(super::io_error)?;
        Ok(Self {
            work,
            snapshot,
            id,
            digest: hash_bytes(text.as_bytes()),
            text,
        })
    }

    fn cursor(&self, offset: usize) -> Result<String, ClewError> {
        let binding = digest(&(
            CURSOR_VERSION,
            &self.work.id,
            self.snapshot,
            RECORD_KIND,
            self.id,
            &self.digest,
            offset,
        ))?;
        Ok(format!("{CURSOR_VERSION}:{offset}:{}", &binding[7..]))
    }

    fn parse_cursor(&self, cursor: &str) -> Result<usize, ClewError> {
        if cursor.len() > MAX_CURSOR_BYTES {
            return Err(invalid("retained-part cursor exceeds its size limit"));
        }
        let (prefix, supplied_binding) = cursor
            .rsplit_once(':')
            .ok_or_else(|| invalid("invalid retained-part cursor"))?;
        let (version, offset_text) = prefix
            .rsplit_once(':')
            .ok_or_else(|| invalid("invalid retained-part cursor"))?;
        if version != CURSOR_VERSION || supplied_binding.len() != 64 {
            return Err(invalid("unsupported or malformed retained-part cursor"));
        }
        let offset = offset_text
            .parse::<usize>()
            .map_err(|_| invalid("invalid retained-part cursor offset"))?;
        if offset.to_string() != offset_text
            || offset == 0
            || offset >= self.text.len()
            || !self.text.is_char_boundary(offset)
            || cursor != self.cursor(offset)?
        {
            return Err(invalid(
                "retained-part cursor belongs to another Work, snapshot, record, or offset",
            ));
        }
        Ok(offset)
    }

    fn response(&self, start: usize, end: usize) -> Result<Value, ClewError> {
        if start >= end
            || end > self.text.len()
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
        {
            return Err(invalid(
                "retained-part byte range is invalid or makes no progress",
            ));
        }
        let fragment = &self.text[start..end];
        let next_cursor = if end < self.text.len() {
            Some(self.cursor(end)?)
        } else {
            None
        };
        let mut response = json!({
            "schema": RESPONSE_SCHEMA, "work": self.work.id, "snapshot": self.snapshot,
            "kind": RECORD_KIND, "id": self.id,
            "authority": "RETAINED_NARRATIVE_NOT_REVERIFIED",
            "recordDigest": self.digest, "startByte": start, "endByte": end,
            "totalRecordBytes": self.text.len(), "text": fragment,
            "fragmentDigest": hash_bytes(fragment.as_bytes()), "nextCursor": next_cursor,
            "receiptType": "RETAINED_OPERATION_PART",
        });
        response["receiptDigest"] = json!(digest(&response)?);
        Ok(response)
    }

    fn receipt(&self, response: &Value) -> Result<RetainedPartReceipt, ClewError> {
        let mut value = response.clone();
        let object = value
            .as_object_mut()
            .ok_or_else(|| invalid("invalid retained-part response"))?;
        object.remove("text");
        object.remove("authority");
        object.remove("receiptType");
        object.insert("schema".into(), json!(RECEIPT_SCHEMA));
        serde_json::from_value(value).map_err(super::io_error)
    }

    fn part(&self, start: usize) -> Result<(Value, RetainedPartReceipt), ClewError> {
        if start >= self.text.len() || !self.text.is_char_boundary(start) {
            return Err(invalid(
                "retained-part cursor is outside the UTF-8 record range",
            ));
        }
        if self.text.len() - start <= self.work.request.max_bytes {
            let response = self.response(start, self.text.len())?;
            if response_size(&response)? <= self.work.request.max_bytes {
                let receipt = self.receipt(&response)?;
                return Ok((response, receipt));
            }
        }
        let mut lower = start.saturating_add(1);
        let mut upper = self
            .text
            .len()
            .saturating_sub(1)
            .min(start.saturating_add(self.work.request.max_bytes));
        let mut best = None;
        while lower <= upper {
            let middle = lower + (upper - lower) / 2;
            let mut end = middle;
            while end > start && !self.text.is_char_boundary(end) {
                end -= 1;
            }
            if end <= start {
                lower = middle.saturating_add(1);
                continue;
            }
            let response = self.response(start, end)?;
            if response_size(&response)? <= self.work.request.max_bytes {
                best = Some(response);
                lower = middle.saturating_add(1);
            } else {
                upper = middle - 1;
            }
        }
        let response = best.ok_or_else(|| invalid(
            "RETAINED_PART_NO_PROGRESS: receipt fields and one UTF-8 record byte cannot fit this Work byte budget; prepare new Work against the same saved snapshot with a larger maxBytes value up to 49152",
        ))?;
        let receipt = self.receipt(&response)?;
        Ok((response, receipt))
    }
}

fn response_size(response: &Value) -> Result<usize, ClewError> {
    bytes(response)?
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("retained-part response size overflow"))
}

pub fn read_retained_part(
    repo: &Repository,
    id: &str,
    request: RetainedPartRequest,
) -> Result<Value, ClewError> {
    read_retained_part_loaded(repo, &work::load(repo, id)?, request)
}

pub(super) fn read_retained_part_loaded(
    repo: &Repository,
    work: &Work,
    request: RetainedPartRequest,
) -> Result<Value, ClewError> {
    if request.schema != REQUEST_SCHEMA || request.kind != RECORD_KIND {
        return Err(invalid(
            "retained-part request requires codeclew-documentation-retained-part-request/1.0 and kind RETAINED_OPERATION",
        ));
    }
    let record = Record::new(work, &request.id)?;
    let start = request
        .cursor
        .as_deref()
        .map(|cursor| record.parse_cursor(cursor))
        .transpose()?
        .unwrap_or(0);
    let (response, receipt) = record.part(start)?;
    let _lock = repo.lock()?;
    let mut state = work::read_state(repo, &work.id)?;
    if state.work != work.id {
        return Err(invalid("read ledger belongs to another Work"));
    }
    if let Some(existing) = state.retained_part_receipts.get(&receipt.receipt_digest) {
        if existing != &receipt {
            return Err(invalid(
                "retained-part receipt identity conflicts with saved context",
            ));
        }
    } else {
        state
            .retained_part_receipts
            .insert(receipt.receipt_digest.clone(), receipt);
    }
    let encoded = bytes(&state)?;
    if encoded.len() > MAX_READ_LEDGER_BYTES {
        return Err(invalid(
            "Work read ledger exceeds its bound; prepare narrower Work",
        ));
    }
    repo.atomic(
        &format!("{}/reads.json", work::directory(&work.id)?),
        &encoded,
    )?;
    Ok(response)
}

/// Complete recorded delivery of exactly one retained operation's canonical JSON.
pub(super) fn retained_part_complete(
    work: &Work,
    state: &ReadState,
    id: &str,
) -> Result<bool, ClewError> {
    if state.work != work.id {
        return Ok(false);
    }
    let record = match Record::new(work, id) {
        Ok(record) => record,
        Err(_) => return Ok(false),
    };
    let mut ranges = Vec::new();
    for (key, receipt) in &state.retained_part_receipts {
        if receipt.id != id {
            continue;
        }
        if key != &receipt.receipt_digest
            || receipt.schema != RECEIPT_SCHEMA
            || receipt.work != work.id
            || receipt.snapshot != record.snapshot
            || receipt.kind != RECORD_KIND
            || receipt.record_digest != record.digest
            || receipt.total_record_bytes != record.text.len()
            || receipt.start_byte >= receipt.end_byte
            || receipt.end_byte > record.text.len()
            || !record.text.is_char_boundary(receipt.start_byte)
            || !record.text.is_char_boundary(receipt.end_byte)
        {
            return Ok(false);
        }
        let response = record.response(receipt.start_byte, receipt.end_byte)?;
        if record.receipt(&response)? != *receipt
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
    let mut covered = 0;
    for (start, end) in ranges {
        if start != covered {
            return Ok(false);
        }
        covered = end;
    }
    Ok(covered == record.text.len())
}

pub(super) fn completed_retained_operation_ids(
    work: &Work,
    state: &ReadState,
) -> Result<BTreeSet<String>, ClewError> {
    let candidates: BTreeSet<_> = state
        .retained_part_receipts
        .values()
        .map(|receipt| receipt.id.as_str())
        .collect();
    let mut completed = BTreeSet::new();
    for id in candidates {
        if retained_part_complete(work, state, id)? {
            completed.insert(id.to_owned());
        }
    }
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::super::work::{ReadReceipt, Selection};
    use super::*;

    fn fixture(max_bytes: usize) -> Work {
        serde_json::from_value(json!({
            "schema":"codeclew-documentation-work/1.0", "id":"a".repeat(64),
            "subject":"service:orders", "snapshot":"sha256:saved-snapshot",
            "request":{"schema":"codeclew-documentation-work-request/1.0",
                "audience":"Maintainers", "maxItems":20, "maxBytes":max_bytes},
            "checked":{"schema":"codeclew-documentation-check/1.0", "inputDigest":"input",
                "contextDigest":"context", "services":{}, "unresolved":{},
                "interactions":{}, "scenarios":{}, "dependencies":{}},
            "retained":{"schema":"codeclew-documentation-narrative/1.3",
                "subject":"service:orders", "contextDigest":"context", "gaps":{},
                "operations":[{"id":"reserve", "title":"Reserve order",
                    "summary":{"id":"summary", "text":"Retained λ🔥 quote \" and slash \\ with newline\n".repeat(2000),
                        "dependencyIds":[], "sourceIds":[]},
                    "explanation":[], "interfaceContracts":[], "participants":[], "events":[],
                    "findings":[{"id":"finding", "text":"Keep unrelated finding 終端",
                        "dependencyIds":[], "sourceIds":[]}], "boundaries":["Retained limitation"]}]},
            "externalInputs":{}, "handles":{}, "influence":{}, "obligations":[], "reviewReasons":[]
        })).unwrap()
    }

    fn request(cursor: Option<String>) -> RetainedPartRequest {
        RetainedPartRequest {
            schema: REQUEST_SCHEMA.into(),
            kind: RECORD_KIND.into(),
            id: "reserve".into(),
            cursor,
        }
    }

    fn collect(work: &Work) -> (Vec<Value>, ReadState) {
        let record = Record::new(work, "reserve").unwrap();
        let mut state = ReadState {
            work: work.id.clone(),
            ..Default::default()
        };
        let mut responses = Vec::new();
        let mut start = 0;
        loop {
            let (response, receipt) = record.part(start).unwrap();
            start = receipt.end_byte;
            let done = receipt.next_cursor.is_none();
            state
                .retained_part_receipts
                .insert(receipt.receipt_digest.clone(), receipt);
            responses.push(response);
            if done {
                break;
            }
        }
        (responses, state)
    }

    #[test]
    fn oversized_canonical_operation_reconstructs_exact_utf8_with_full_response_bound() {
        let work = fixture(49_152);
        let original = bytes(&work).unwrap();
        let record = Record::new(&work, "reserve").unwrap();
        assert!(record.text.len() > 49_152);
        assert_eq!(
            record.digest,
            retained_record_digest(&work, "reserve").unwrap()
        );
        let (parts, state) = collect(&work);
        assert!(parts.len() > 1);
        let mut reassembled = String::new();
        for part in &parts {
            assert!(response_size(part).unwrap() <= work.request.max_bytes);
            assert_eq!(
                part["startByte"].as_u64().unwrap() as usize,
                reassembled.len()
            );
            assert!(part["endByte"].as_u64().unwrap() > part["startByte"].as_u64().unwrap());
            reassembled.push_str(part["text"].as_str().unwrap());
            if let Some(cursor) = part["nextCursor"].as_str() {
                assert_eq!(record.parse_cursor(cursor).unwrap(), reassembled.len());
            }
        }
        assert_eq!(
            reassembled.as_bytes(),
            bytes(retained_operation(&work, "reserve").unwrap()).unwrap()
        );
        let decoded: Operation = serde_json::from_str(&reassembled).unwrap();
        assert_eq!(&decoded, retained_operation(&work, "reserve").unwrap());
        assert!(retained_part_complete(&work, &state, "reserve").unwrap());
        assert_eq!(bytes(&work).unwrap(), original);
    }

    #[test]
    fn interrupted_reads_resume_and_exact_replays_do_not_invent_coverage() {
        let work = fixture(49_152);
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Retained record read test").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let first = read_retained_part_loaded(&repo, &work, request(None)).unwrap();
        let before = bytes(&work::read_state(&repo, &work.id).unwrap()).unwrap();
        assert_eq!(
            read_retained_part_loaded(&repo, &work, request(None)).unwrap(),
            first
        );
        assert_eq!(
            bytes(&work::read_state(&repo, &work.id).unwrap()).unwrap(),
            before
        );
        assert!(
            !retained_part_complete(
                &work,
                &work::read_state(&repo, &work.id).unwrap(),
                "reserve"
            )
            .unwrap()
        );
        let mut text = first["text"].as_str().unwrap().to_owned();
        let mut cursor = first["nextCursor"].as_str().map(str::to_owned);
        while cursor.is_some() {
            let part = read_retained_part_loaded(&repo, &work, request(cursor)).unwrap();
            text.push_str(part["text"].as_str().unwrap());
            cursor = part["nextCursor"].as_str().map(str::to_owned);
        }
        assert_eq!(text, Record::new(&work, "reserve").unwrap().text);
        assert!(
            retained_part_complete(
                &work,
                &work::read_state(&repo, &work.id).unwrap(),
                "reserve"
            )
            .unwrap()
        );
    }

    #[test]
    fn cursors_bind_exact_work_snapshot_record_digest_and_utf8_offset() {
        let work = fixture(4096);
        let record = Record::new(&work, "reserve").unwrap();
        let (part, _) = record.part(0).unwrap();
        let cursor = part["nextCursor"].as_str().unwrap();
        assert!(record.parse_cursor(cursor).is_ok());
        for change in ["work", "snapshot", "record", "identity"] {
            let mut changed = work.clone();
            match change {
                "work" => changed.id = "b".repeat(64),
                "snapshot" => changed.snapshot = Some("sha256:new-snapshot".into()),
                "record" => changed.retained.as_mut().unwrap().operations[0]
                    .title
                    .push_str(" changed"),
                "identity" => {
                    changed.retained.as_mut().unwrap().operations[0].id = "another".into()
                }
                _ => unreachable!(),
            }
            let id = if change == "identity" {
                "another"
            } else {
                "reserve"
            };
            assert!(
                Record::new(&changed, id)
                    .unwrap()
                    .parse_cursor(cursor)
                    .is_err(),
                "{change}"
            );
        }
        let unicode_offset = record.text.find('🔥').unwrap() + 1;
        for offset in [0, record.text.len(), record.text.len() + 1, unicode_offset] {
            assert!(
                record
                    .parse_cursor(&record.cursor(offset).unwrap())
                    .is_err()
            );
        }
        assert!(record.parse_cursor(&format!("{}x", cursor)).is_err());
        assert!(record.parse_cursor(&"x".repeat(257)).is_err());
    }

    #[test]
    fn completeness_rejects_missing_altered_borrowed_and_overlapping_receipts() {
        let work = fixture(49_152);
        let (_, state) = collect(&work);
        let first_key = state
            .retained_part_receipts
            .iter()
            .min_by_key(|(_, receipt)| receipt.start_byte)
            .unwrap()
            .0
            .clone();
        let mut missing = state.clone();
        missing.retained_part_receipts.remove(&first_key);
        assert!(!retained_part_complete(&work, &missing, "reserve").unwrap());
        for field in [
            "schema",
            "work",
            "snapshot",
            "kind",
            "id",
            "recordDigest",
            "fragmentDigest",
            "receiptDigest",
            "nextCursor",
        ] {
            let mut altered = state.clone();
            let mut value =
                serde_json::to_value(&altered.retained_part_receipts[&first_key]).unwrap();
            value[field] = json!("altered");
            altered
                .retained_part_receipts
                .insert(first_key.clone(), serde_json::from_value(value).unwrap());
            assert!(
                !retained_part_complete(&work, &altered, "reserve").unwrap(),
                "{field}"
            );
        }
        for field in ["startByte", "endByte", "totalRecordBytes"] {
            let mut altered = state.clone();
            let mut value =
                serde_json::to_value(&altered.retained_part_receipts[&first_key]).unwrap();
            value[field] = json!(usize::MAX);
            altered
                .retained_part_receipts
                .insert(first_key.clone(), serde_json::from_value(value).unwrap());
            assert!(
                !retained_part_complete(&work, &altered, "reserve").unwrap(),
                "{field}"
            );
        }
        let record = Record::new(&work, "reserve").unwrap();
        let mut split_utf8 = state.clone();
        split_utf8
            .retained_part_receipts
            .get_mut(&first_key)
            .unwrap()
            .end_byte = record.text.find('🔥').unwrap() + 1;
        assert!(!retained_part_complete(&work, &split_utf8, "reserve").unwrap());
        let mut overlapping = state.clone();
        let extra = record.receipt(&record.response(0, 1).unwrap()).unwrap();
        overlapping
            .retained_part_receipts
            .insert(extra.receipt_digest.clone(), extra);
        assert!(!retained_part_complete(&work, &overlapping, "reserve").unwrap());
        let mut borrowed = work.clone();
        borrowed.id = "b".repeat(64);
        let mut changed_state = state.clone();
        changed_state.work = borrowed.id.clone();
        assert!(!retained_part_complete(&borrowed, &changed_state, "reserve").unwrap());
        let mut changed = work.clone();
        changed.retained.as_mut().unwrap().operations[0].findings[0]
            .text
            .push_str(" changed");
        assert!(!retained_part_complete(&changed, &state, "reserve").unwrap());
        let mut wrong_key = state.clone();
        let receipt = wrong_key.retained_part_receipts.remove(&first_key).unwrap();
        wrong_key
            .retained_part_receipts
            .insert("wrong-key".into(), receipt);
        assert!(!retained_part_complete(&work, &wrong_key, "reserve").unwrap());
        let mut no_progress = state.clone();
        no_progress
            .retained_part_receipts
            .get_mut(&first_key)
            .unwrap()
            .end_byte = 0;
        assert!(!retained_part_complete(&work, &no_progress, "reserve").unwrap());
    }

    #[test]
    fn only_exact_retained_omission_is_resolved_and_automatic_packet_gate_stays_closed() {
        let work = fixture(49_152);
        let (_, mut state) = collect(&work);
        let page = ReadReceipt {
            selection: Selection::default(),
            requested_selection: None,
            result_digest: "page-digest".into(),
            supplied: Vec::new(),
            membership_digest: "membership".into(),
            omitted: vec![
                json!({"kind":RECORD_KIND, "id":"reserve", "reason":"ITEM_EXCEEDS_WORK_BYTE_BUDGET"}),
            ],
            next_cursor: None,
        };
        state.receipts.insert("page".into(), page.clone());
        assert!(
            super::super::work_parts::initial_context_complete_with_parts(&work, &state).unwrap()
        );
        assert!(
            !super::super::work_parts::initial_context_complete_with_packet_parts(
                &work,
                &state,
                &BTreeSet::new()
            )
            .unwrap()
        );
        let mut no_pages = state.clone();
        no_pages.receipts.clear();
        assert!(
            !super::super::work_parts::initial_context_complete_with_parts(&work, &no_pages)
                .unwrap()
        );
        for (kind, id) in [
            (RECORD_KIND, "another"),
            ("DEPENDENCY", "reserve"),
            ("EXTERNAL_INPUT", "reserve"),
            ("SOURCE", "reserve"),
        ] {
            let mut altered = state.clone();
            altered.receipts.get_mut("page").unwrap().omitted = vec![json!({"kind":kind,"id":id})];
            assert!(
                !super::super::work_parts::initial_context_complete_with_parts(&work, &altered)
                    .unwrap(),
                "{kind}/{id}"
            );
        }
        let mut missing = state.clone();
        missing.retained_part_receipts.clear();
        assert!(
            !super::super::work_parts::initial_context_complete_with_parts(&work, &missing)
                .unwrap()
        );
    }

    #[test]
    fn invalid_requests_and_no_progress_write_no_receipt_and_legacy_ledger_stays_stable() {
        let work = fixture(49_152);
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Retained record rejection test").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let legacy = json!({"work":work.id,"receipts":{},"untrackedReads":false});
        let state: ReadState = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(&state).unwrap(), legacy);
        assert!(state.retained_part_receipts.is_empty());
        for mode in ["schema", "kind", "unknown", "cursor"] {
            let mut invalid_request = request(None);
            match mode {
                "schema" => invalid_request.schema = "unsupported".into(),
                "kind" => invalid_request.kind = "DEPENDENCY".into(),
                "unknown" => invalid_request.id = "unknown".into(),
                "cursor" => invalid_request.cursor = Some("bad-cursor".into()),
                _ => unreachable!(),
            }
            assert!(read_retained_part_loaded(&repo, &work, invalid_request).is_err());
        }
        let mut narrow = work.clone();
        narrow.request.max_bytes = 1;
        assert!(
            read_retained_part_loaded(&repo, &narrow, request(None))
                .unwrap_err()
                .message
                .contains("RETAINED_PART_NO_PROGRESS")
        );
        assert_eq!(
            serde_json::to_value(work::read_state(&repo, &work.id).unwrap()).unwrap(),
            legacy
        );
        let mut scoped = work.clone();
        scoped.request.entrypoint = Some("another".into());
        assert!(retained_operation(&scoped, "reserve").is_err());
        let mut ambiguous = work.clone();
        let duplicate = ambiguous.retained.as_ref().unwrap().operations[0].clone();
        ambiguous
            .retained
            .as_mut()
            .unwrap()
            .operations
            .push(duplicate);
        assert!(retained_operation(&ambiguous, "reserve").is_err());
        assert!(
            serde_json::from_value::<RetainedPartRequest>(
                json!({"schema":REQUEST_SCHEMA,"kind":RECORD_KIND,"id":"reserve","path":"/summary"})
            )
            .is_err()
        );
        let mut missing_snapshot = work.clone();
        missing_snapshot.snapshot = None;
        assert!(read_retained_part_loaded(&repo, &missing_snapshot, request(None)).is_err());
    }

    #[test]
    fn response_size_counts_newline_and_escape_expansion_at_the_exact_boundary() {
        let mut work = fixture(49_152);
        work.retained.as_mut().unwrap().operations[0].summary.text = "λ🔥\"\\\n".into();
        let record = Record::new(&work, "reserve").unwrap();
        let response = record.response(0, record.text.len()).unwrap();
        let exact = response_size(&response).unwrap();
        assert_eq!(exact, bytes(&response).unwrap().len() + 1);
        work.request.max_bytes = exact;
        assert!(Record::new(&work, "reserve").unwrap().part(0).unwrap().0["nextCursor"].is_null());
        work.request.max_bytes = exact - 1;
        let (smaller, _) = Record::new(&work, "reserve").unwrap().part(0).unwrap();
        assert!(!smaller["nextCursor"].is_null());
        assert!(response_size(&smaller).unwrap() < exact);
    }
}
