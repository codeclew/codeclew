//! Recorded retained-evidence retrieval shared by documentation role coordinators.
//! Callers own model budgets, pending retrieval plans and phase checkpoints.

use super::{Repository, digest, invalid};
use crate::{
    documentation::{
        work::{self, Selection, Work},
        work_parts,
    },
    error::{ClewError, ErrorCode},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::documentation) enum ProgressStage {
    SourcePart,
    Page,
}

#[derive(Debug, Default)]
pub(in crate::documentation) struct Retrieval {
    pub selections: Vec<Value>,
    pub lookup_feedback: Vec<Value>,
    pub receipts: Vec<Value>,
}

/// One role decision may request many native selections. Native protocol limits
/// constrain individual reads only; the host drains technical content pages and
/// parts, while SYMBOL queries preserve one navigation page per selection.
pub(in crate::documentation) fn retrieve<F>(
    repo: &Repository,
    work: &Work,
    selections: &[Selection],
    pages: &mut Vec<Value>,
    source_parts: &mut Vec<Value>,
    mut preflight: F,
) -> Result<Retrieval, ClewError>
where
    F: FnMut(ProgressStage, &[Value], &[Value]) -> Result<(), ClewError>,
{
    let mut retrieval = Retrieval::default();
    for requested in selections {
        if requested.untracked_reads {
            return Err(invalid(
                "NEEDS_EVIDENCE: an isolated role cannot register an outside read after the fact",
            ));
        }
        for requested in native_selections(requested)? {
            let effective = effective_expansion_selection(&requested);
            let mut binding_selection = effective.clone();
            binding_selection.cursor = None;
            binding_selection.untracked_reads = false;
            retrieval.selections.push(json!({
                "requested":requested,"effective":effective,
                "effectiveSelectionDigest":digest(&(work.id.as_str(), &binding_selection))?
            }));
            let mut cursor = effective.cursor.clone();
            let navigation_page = effective.query.as_ref().is_some_and(|query| {
                query.kind == "SYMBOL" && query.projection == work::QueryProjection::Navigation
            });
            let mut seen_cursors = BTreeSet::new();
            loop {
                if let Some(current) = cursor.as_ref()
                    && !seen_cursors.insert(current.clone())
                {
                    return Err(invalid("NEEDS_EVIDENCE: expansion cursor repeated"));
                }
                let mut selection = effective.clone();
                selection.cursor = cursor.clone();
                let mut requested_page = requested.clone();
                requested_page.cursor = cursor.clone();
                let page = match work::read_loaded_with_requested(
                    repo,
                    work,
                    selection.clone(),
                    Some(requested_page),
                ) {
                    Ok(page) => page,
                    Err(error) => {
                        let action = json!({"action":"expand","selection":requested});
                        if selection.cursor.is_none()
                            && let Some(feedback) =
                                symbol_lookup_feedback(&action, &selection, &error)
                        {
                            retrieval.lookup_feedback.push(feedback);
                            break;
                        }
                        return Err(error);
                    }
                };
                let next_cursor = page["nextCursor"].as_str().map(str::to_owned);
                if next_cursor
                    .as_deref()
                    .is_some_and(|next| cursor.as_deref() == Some(next))
                {
                    return Err(invalid("NEEDS_EVIDENCE: expansion cursor made no progress"));
                }
                read_omitted_source_parts(repo, work, &page, source_parts, |parts| {
                    preflight(ProgressStage::SourcePart, pages, parts)
                })?;
                let omitted_references: BTreeSet<_> = page["omitted"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|row| row["reference"].as_str())
                    .collect();
                for part in source_parts.iter().filter(|part| {
                    part["reference"]
                        .as_str()
                        .is_some_and(|reference| omitted_references.contains(reference))
                }) {
                    retrieval.receipts.push(json!({
                        "kind":"SOURCE_PART","work":work.id,"snapshot":work.snapshot,
                        "reference":part["reference"],"receiptDigest":part["receiptDigest"],
                        "fragmentDigest":part["fragmentDigest"],"startByte":part["startByte"],"endByte":part["endByte"]
                    }));
                }
                retrieval.receipts.push(json!({
                    "kind":"PAGE","work":work.id,"snapshot":work.snapshot,
                    "receiptDigest":page["receiptDigest"],"membershipDigest":page["membershipDigest"],
                    "selection":selection,"nextCursor":page["nextCursor"]
                }));
                let page_digest = digest(&page)?;
                if !pages
                    .iter()
                    .any(|existing| digest(existing).ok().as_deref() == Some(page_digest.as_str()))
                {
                    pages.push(page);
                }
                preflight(ProgressStage::Page, pages, source_parts)?;
                if navigation_page {
                    break;
                }
                cursor = next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }
    }
    Ok(retrieval)
}

fn native_selections(requested: &Selection) -> Result<Vec<Selection>, ClewError> {
    if (!requested.references.is_empty() && !requested.symbols.is_empty())
        || (requested.query.is_some()
            && (!requested.references.is_empty() || !requested.symbols.is_empty()))
    {
        work::validate_selection(requested)?;
    }
    // Eight is the existing native Work Selection contract, not a grouped request limit.
    let count = requested.references.len().max(requested.symbols.len());
    if count <= 8 {
        work::validate_selection(requested)?;
        return Ok(vec![requested.clone()]);
    }
    if requested.cursor.is_some() {
        return Err(invalid(
            "a grouped selection cursor must belong to one native selection",
        ));
    }
    let values = if requested.references.is_empty() {
        &requested.symbols
    } else {
        &requested.references
    };
    let mut selections = Vec::new();
    for chunk in values.chunks(8) {
        let mut selection = requested.clone();
        if requested.references.is_empty() {
            selection.symbols = chunk.to_vec();
        } else {
            selection.references = chunk.to_vec();
        }
        work::validate_selection(&selection)?;
        selections.push(selection);
    }
    Ok(selections)
}

pub(super) fn read_omitted_source_parts<F>(
    repo: &Repository,
    work: &work::Work,
    page: &Value,
    source_parts: &mut Vec<Value>,
    mut preflight: F,
) -> Result<(), ClewError>
where
    F: FnMut(&[Value]) -> Result<(), ClewError>,
{
    let Some(omitted) = page["omitted"].as_array() else {
        return Ok(());
    };
    let state = work::read_state(repo, &work.id)?;
    let mut by_reference = std::collections::BTreeMap::<String, Vec<Value>>::new();
    for part in source_parts.iter() {
        let reference = part["reference"]
            .as_str()
            .ok_or_else(|| invalid("SOURCE_PART packet is missing a reference"))?;
        by_reference
            .entry(reference.into())
            .or_default()
            .push(part.clone());
    }
    // Complete sources keep the existing strict packet/receipt validation. A
    // durable partial source is validated below before its continuation resumes.
    let complete_parts: Vec<_> = by_reference
        .values()
        .filter(|parts| {
            parts
                .last()
                .is_some_and(|part| part["nextCursor"].is_null())
        })
        .flatten()
        .cloned()
        .collect();
    let mut delivered = work_parts::delivered_source_references(work, &state, &complete_parts)?;
    for row in omitted {
        if row["kind"] != "SOURCE" {
            return Err(invalid(
                "NEEDS_EVIDENCE: a non-SOURCE initial or expanded record exceeds the admitted Work byte budget",
            ));
        }
        let (Some(reference), Some(id)) = (row["reference"].as_str(), row["id"].as_str()) else {
            return Err(invalid("SOURCE omission has no exact Work reference"));
        };
        if !work
            .handles
            .get(reference)
            .is_some_and(|handle| handle.kind == "SOURCE" && handle.id == id)
        {
            return Err(invalid("SOURCE omission does not match this Work handle"));
        }
        if delivered.contains(reference) {
            continue;
        }

        let mut cursor: Option<String> = None;
        let mut seen_cursors = std::collections::BTreeSet::new();
        let mut next_offset = 0usize;
        let mut total_bytes = None;
        if let Some(prefix) = by_reference.get(reference) {
            for part in prefix {
                let receipt_digest = part["receiptDigest"]
                    .as_str()
                    .ok_or_else(|| invalid("SOURCE_PART packet has no receipt digest"))?;
                let saved_receipt =
                    state
                        .source_part_receipts
                        .get(receipt_digest)
                        .ok_or_else(|| {
                            invalid("SOURCE_PART packet has no matching recorded receipt")
                        })?;
                if part["startByte"].as_u64() != Some(next_offset as u64) {
                    return Err(invalid(
                        "SOURCE_PART packet contains a gap, overlap, or no-progress range",
                    ));
                }
                let expected = work_parts::read_part_loaded(
                    repo,
                    work,
                    work::SourcePartRequest {
                        schema: work_parts::REQUEST_SCHEMA.into(),
                        reference: reference.into(),
                        cursor: cursor.clone(),
                    },
                )?;
                let recorded = work::read_state(repo, &work.id)?;
                if part != &expected
                    || recorded.source_part_receipts.get(receipt_digest) != Some(saved_receipt)
                {
                    return Err(invalid(
                        "SOURCE_PART packet text, range, snapshot, or digest does not match retained evidence",
                    ));
                }
                next_offset = part["endByte"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| invalid("SOURCE_PART packet has an invalid end offset"))?;
                total_bytes = part["totalTextBytes"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok());
                cursor = part["nextCursor"].as_str().map(str::to_owned);
                if cursor.is_none() {
                    return Err(invalid(
                        "SOURCE_PART saved prefix has an unexpected completed part",
                    ));
                }
            }
        }
        loop {
            if let Some(value) = cursor.as_ref()
                && !seen_cursors.insert(value.clone())
            {
                return Err(invalid(
                    "SOURCE_PART_NO_PROGRESS: repeated continuation cursor",
                ));
            }
            let response = work_parts::read_part_loaded(
                repo,
                work,
                work::SourcePartRequest {
                    schema: work_parts::REQUEST_SCHEMA.into(),
                    reference: reference.into(),
                    cursor: cursor.clone(),
                },
            )?;
            let start = response["startByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| invalid("SOURCE_PART returned an invalid start offset"))?;
            let end = response["endByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| invalid("SOURCE_PART returned an invalid end offset"))?;
            let total = response["totalTextBytes"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| invalid("SOURCE_PART returned an invalid total byte count"))?;
            let text_bytes = response["text"]
                .as_str()
                .ok_or_else(|| invalid("SOURCE_PART returned no text fragment"))?
                .len();
            if start != next_offset || end < start || (total > 0 && end == start) {
                return Err(invalid(
                    "SOURCE_PART_NO_PROGRESS: returned byte ranges do not advance contiguously",
                ));
            }
            if total_bytes.is_some_and(|expected| expected != total) {
                return Err(invalid("SOURCE_PART changed total source byte count"));
            }
            total_bytes = Some(total);
            if end > total || end - start != text_bytes {
                return Err(invalid(
                    "SOURCE_PART returned an out-of-range byte interval",
                ));
            }
            next_offset = end;
            cursor = response["nextCursor"].as_str().map(str::to_owned);
            let final_part = cursor.is_none();
            if final_part != (end == total) {
                return Err(invalid(
                    "SOURCE_PART continuation cursor does not match the returned range",
                ));
            }
            source_parts.push(response);
            preflight(source_parts)?;
            if final_part {
                break;
            }
        }
        delivered = work_parts::delivered_source_references(
            work,
            &work::read_state(repo, &work.id)?,
            source_parts,
        )?;
        if !delivered.contains(reference) {
            return Err(invalid(
                "SOURCE_PART did not deliver complete retained source",
            ));
        }
    }
    Ok(())
}
pub(super) fn symbol_lookup_feedback(
    result: &Value,
    selection: &work::Selection,
    error: &ClewError,
) -> Option<Value> {
    let result = result.as_object()?;
    if result
        .keys()
        .any(|key| !matches!(key.as_str(), "action" | "selection"))
        || selection.cursor.is_some()
        || !selection.references.is_empty()
        || selection.query.is_some()
        || selection.untracked_reads
        || selection.symbols.is_empty()
        || selection.symbols.len() > 8
        || selection
            .symbols
            .iter()
            .any(|symbol| symbol.trim().is_empty())
    {
        return None;
    }
    let status = match &error.code {
        ErrorCode::SymbolNotFound => "NOT_FOUND",
        ErrorCode::AmbiguousSymbol => "AMBIGUOUS",
        _ => return None,
    };
    let selector = error.relevant_anchors_or_symbols.first()?;
    if !selection.symbols.contains(selector) {
        return None;
    }
    let message = if status == "NOT_FOUND" {
        "No captured declaration matches this selector in the selected Work. This lookup result is not proof that code is absent. Use a bounded SYMBOL query, then select only exact identities shown in returned records; do not guess a package, owner, or signature."
    } else {
        "This selector matches multiple captured declarations in the selected Work. Use a bounded SYMBOL query, then select only an exact identity shown in returned records; do not guess a package, owner, or signature."
    };
    Some(serde_json::json!({
        "kind":"SYMBOL_LOOKUP",
        "status":status,
        "requestedSelector":selector,
        "navigationOnly":true,
        "message":message
    }))
}

pub(super) fn effective_expansion_selection(requested: &work::Selection) -> work::Selection {
    let mut effective = requested.clone();
    if let Some(query) = effective.query.as_mut()
        && query.kind == "SYMBOL"
    {
        query.projection = work::QueryProjection::Navigation;
    }
    effective
}
