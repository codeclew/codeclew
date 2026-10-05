//! Exact recorded context delivery for opt-in expanding operation roles.
//! Saved delivery is rebound to immutable Work bytes, not trusted by its hashes.
use super::{
    bytes, digest, invalid, io_error, job_context, operation_answer, operation_packet,
    store::Repository,
    work::{self, ReadReceipt, ReadState, Work},
    work_parts::{self, SourcePartReceipt},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const DELIVERY_SCHEMA: &str = "codeclew-operation-context-delivery/1.0";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Delivery {
    schema: String,
    work: String,
    snapshot: Option<String>,
    pages: Vec<Value>,
    source_parts: Vec<Value>,
    page_receipts: Vec<ReadReceipt>,
    source_part_receipts: Vec<SourcePartReceipt>,
    presentation: Value,
    citations: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delivered_digest: Option<String>,
}

pub(super) fn initial(work: &Work) -> Result<(Value, Value), ClewError> {
    operation_packet::build(work)
}

/// Inputs are the cumulative raw pages/parts delivered to this role. Exact
/// duplicates are retained only once; presentation may additionally alias rows.
pub(super) fn extend(
    repo: &Repository,
    work: &Work,
    base_packet: &Value,
    pages: &[Value],
    parts: &[Value],
) -> Result<(Value, Value), ClewError> {
    require_contract(work)?;
    let (base, _) = operation_packet::build(work)?;
    validate_base(work, base_packet, &base)?;
    if pages.is_empty() && parts.is_empty() {
        return Ok((
            base_packet.clone(),
            operation_packet::audit_saved_packet(work, base_packet)?,
        ));
    }
    let delivery = context(repo, work, pages, parts)?;
    let mut packet = base;
    packet["contextDelivery"] = delivery.clone();
    let citations = packet["citations"]
        .as_object_mut()
        .ok_or_else(|| invalid("packet citations missing"))?;
    for (reference, description) in delivery["citations"].as_object().into_iter().flatten() {
        citations
            .entry(reference.clone())
            .or_insert_with(|| description.clone());
    }
    packet.as_object_mut().unwrap().remove("packetDigest");
    packet["packetDigest"] = json!(digest(&packet)?);
    let audit = operation_packet::audit_saved_packet(work, &packet)?;
    Ok((packet, audit))
}

/// Independent reviewer evidence envelope. Never rewrites the saved author packet.
pub(super) fn context(
    repo: &Repository,
    work: &Work,
    pages: &[Value],
    parts: &[Value],
) -> Result<Value, ClewError> {
    require_contract(work)?;
    let state = work::read_state(repo, &work.id)?;
    if state.work != work.id || state.untracked_reads {
        return Err(invalid(
            "expanded context requires exact tracked Work receipts",
        ));
    }
    let pages = unique(pages)?;
    let parts = unique(parts)?;
    let mut page_receipts = Vec::new();
    for page in &pages {
        let receipt = state
            .receipts
            .values()
            .find(|receipt| page["receiptDigest"].as_str() == Some(receipt.result_digest.as_str()))
            .ok_or_else(|| invalid("expanded page has no recorded Work receipt"))?;
        page_receipts.push(receipt.clone());
    }
    let mut source_part_receipts = Vec::new();
    for part in &parts {
        let receipt = part["receiptDigest"]
            .as_str()
            .and_then(|key| state.source_part_receipts.get(key))
            .ok_or_else(|| invalid("expanded SOURCE_PART has no recorded Work receipt"))?;
        source_part_receipts.push(receipt.clone());
    }
    let mut delivery = Delivery {
        schema: DELIVERY_SCHEMA.into(),
        work: work.id.clone(),
        snapshot: work.snapshot.clone(),
        presentation: job_context::present(&pages, &parts),
        pages,
        source_parts: parts,
        page_receipts,
        source_part_receipts,
        citations: BTreeMap::new(),
        delivered_digest: None,
    };
    let records = delivered_records(work, &delivery)?;
    delivery.citations = record_citations(&records);
    delivery.delivered_digest = Some(digest(&delivery)?);
    serde_json::to_value(delivery).map_err(io_error)
}

fn unique(values: &[Value]) -> Result<Vec<Value>, ClewError> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for value in values {
        if seen.insert(digest(value)?) {
            result.push(value.clone());
        }
    }
    Ok(result)
}

fn require_contract(work: &Work) -> Result<(), ClewError> {
    if work.request.authoring_contract.as_deref()
        != Some(operation_answer::EXPANDING_AUTHORING_CONTRACT)
        || !operation_answer::expanding_authoring_eligible(&work.request)
    {
        return Err(invalid(
            "operation context expansion requires explicit authoring contract 1.6",
        ));
    }
    Ok(())
}

/// Return canonical, fully delivered record audit rows. Navigation rows and
/// oversized omitted records are never facts, even if their references are known.
pub(super) fn validate_saved(work: &Work, packet: &Value) -> Result<Vec<Value>, ClewError> {
    let Some(value) = packet.get("contextDelivery") else {
        return Ok(Vec::new());
    };
    require_contract(work)?;
    let (base, _) = operation_packet::build(work)?;
    validate_base(work, packet, &base)?;
    let mut delivery: Delivery = serde_json::from_value(value.clone()).map_err(io_error)?;
    let saved_digest = delivery
        .delivered_digest
        .take()
        .ok_or_else(|| invalid("context delivery digest missing"))?;
    if delivery.schema != DELIVERY_SCHEMA
        || delivery.work != work.id
        || delivery.snapshot != work.snapshot
        || digest(&delivery)? != saved_digest
        || unique(&delivery.pages)? != delivery.pages
        || unique(&delivery.source_parts)? != delivery.source_parts
        || delivery.presentation != job_context::present(&delivery.pages, &delivery.source_parts)
    {
        return Err(invalid(
            "saved context delivery identity, content or presentation differs",
        ));
    }
    let records = delivered_records(work, &delivery)?;
    if delivery.citations != record_citations(&records) {
        return Err(invalid(
            "context citations differ from complete delivered records",
        ));
    }
    let mut expected = base["citations"]
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("base citations missing"))?;
    for (reference, description) in &delivery.citations {
        expected
            .entry(reference.clone())
            .or_insert_with(|| json!(description));
    }
    if packet["citations"] != Value::Object(expected) {
        return Err(invalid(
            "expanded packet citations differ from exactly delivered evidence",
        ));
    }
    Ok(records)
}

/// Native display uses actual complete delivered records, never a citation-map
/// shortcut. The Work-bound audit must describe the same delivered bytes.
pub(super) fn displayed_records(packet: &Value, audit: &Value) -> Result<Vec<Value>, ClewError> {
    let Some(value) = packet.get("contextDelivery") else {
        return Ok(Vec::new());
    };
    let mut delivery: Delivery = serde_json::from_value(value.clone()).map_err(io_error)?;
    let declared = delivery
        .delivered_digest
        .take()
        .ok_or_else(|| invalid("display delivery digest missing"))?;
    let mut unsigned_audit = audit.clone();
    let audit_digest = unsigned_audit
        .as_object_mut()
        .and_then(|object| object.remove("auditDigest"));
    if delivery.schema != DELIVERY_SCHEMA
        || audit["schema"] != operation_packet::AUDIT_SCHEMA
        || digest(&delivery)? != declared
        || audit["workId"] != delivery.work
        || audit["snapshot"] != json!(delivery.snapshot)
        || audit["packetDigest"] != packet["packetDigest"]
        || audit_digest != Some(json!(digest(&unsigned_audit)?))
    {
        return Err(invalid(
            "display context is not bound to the saved packet audit",
        ));
    }
    let mut output = Vec::new();
    for reference in delivery.citations.keys() {
        let rows: Vec<_> = audit["records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| row["label"] == reference.as_str())
            .collect();
        let [row] = rows.as_slice() else {
            return Err(invalid("expanded display record is missing or ambiguous"));
        };
        if row["deliveredToAuthor"] != true || row["recordDigest"] != digest(&row["row"])? {
            return Err(invalid("expanded display record was not fully delivered"));
        }
        if row["kind"] == "SOURCE" {
            let source: super::model::Source =
                serde_json::from_value(row["row"]["record"].clone()).map_err(io_error)?;
            if source.file.trim().is_empty()
                || source.start_line == 0
                || source.end_line < source.start_line
            {
                return Err(invalid(
                    "expanded source has no valid native source location",
                ));
            }
        }
        let page_delivery = delivery
            .pages
            .iter()
            .flat_map(|page| page["items"].as_array().into_iter().flatten())
            .any(|item| {
                item["reference"] == reference.as_str()
                    && json!({"kind":item["kind"],"id":item["id"],"record":item["record"]})
                        == row["row"]
            });
        if !page_delivery && !source_parts_display_complete(&delivery, reference, row)? {
            return Err(invalid(
                "expanded displayed evidence has no exact complete delivered record",
            ));
        }
        output.push((*row).clone());
    }
    Ok(output)
}

fn source_parts_display_complete(
    delivery: &Delivery,
    reference: &str,
    row: &Value,
) -> Result<bool, ClewError> {
    if row["kind"] != "SOURCE" {
        return Ok(false);
    }
    let source: super::model::Source =
        serde_json::from_value(row["row"]["record"].clone()).map_err(io_error)?;
    let mut metadata = serde_json::to_value(&source).map_err(io_error)?;
    metadata.as_object_mut().unwrap().remove("text");
    let record_digest = digest(&source)?;
    let mut parts: Vec<_> = delivery
        .source_parts
        .iter()
        .filter(|part| part["reference"] == reference)
        .collect();
    if parts.is_empty() {
        return Ok(false);
    }
    parts.sort_by_key(|part| part["startByte"].as_u64());
    let mut covered = 0usize;
    for part in parts {
        let end = part["endByte"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok());
        let Some(end) = end else {
            return Ok(false);
        };
        if part["work"] != delivery.work
            || part["snapshot"] != json!(delivery.snapshot)
            || part["startByte"] != covered
            || end < covered
            || part["sourceId"] != source.id
            || part["source"] != metadata
            || part["recordDigest"] != record_digest
            || part["totalTextBytes"] != source.text.len()
            || source.text.get(covered..end) != part["text"].as_str()
        {
            return Ok(false);
        }
        covered = end;
    }
    Ok(covered == source.text.len())
}

fn validate_base(work: &Work, packet: &Value, base: &Value) -> Result<(), ClewError> {
    if packet.get("contextDelivery").is_some() {
        require_contract(work)?;
    }
    let mut candidate = packet.clone();
    let object = candidate
        .as_object_mut()
        .ok_or_else(|| invalid("operation packet must be object"))?;
    object.remove("contextDelivery");
    object.remove("packetDigest");
    object.insert("citations".into(), base["citations"].clone());
    let mut expected = base.clone();
    expected.as_object_mut().unwrap().remove("packetDigest");
    if candidate != expected {
        return Err(invalid(
            "expanded context changed canonical base packet evidence",
        ));
    }
    Ok(())
}

fn record_citations(records: &[Value]) -> BTreeMap<String, String> {
    records
        .iter()
        .filter_map(|record| {
            Some((
                record["label"].as_str()?.to_owned(),
                format!("expanded retained {} record", record["kind"].as_str()?),
            ))
        })
        .collect()
}

fn delivered_records(work: &Work, delivery: &Delivery) -> Result<Vec<Value>, ClewError> {
    if delivery.pages.len() != delivery.page_receipts.len()
        || delivery.source_parts.len() != delivery.source_part_receipts.len()
    {
        return Err(invalid(
            "context delivery receipts do not match delivered pages/parts",
        ));
    }
    let mut records = BTreeMap::new();
    for (page, receipt) in delivery.pages.iter().zip(&delivery.page_receipts) {
        validate_page(work, page, receipt)?;
        for row in page["items"].as_array().into_iter().flatten() {
            let Some(reference) = row["reference"].as_str() else {
                continue;
            };
            let handle = work
                .handles
                .get(reference)
                .ok_or_else(|| invalid("expanded row has foreign reference"))?;
            if row["kind"].as_str() == Some(handle.kind.as_str())
                && super::proposals::evidence_reference_allowed(handle)
            {
                insert_record(&mut records, reference, row.clone())?;
            }
        }
    }
    let state = ReadState {
        work: work.id.clone(),
        source_part_receipts: delivery
            .source_part_receipts
            .iter()
            .map(|receipt| (receipt.receipt_digest.clone(), receipt.clone()))
            .collect(),
        ..Default::default()
    };
    let complete = work_parts::delivered_source_references(work, &state, &delivery.source_parts)?;
    for reference in complete {
        let rows = work::audit_rows(
            work,
            &work::Selection {
                references: vec![reference.clone()],
                ..Default::default()
            },
        )?;
        let row = rows
            .into_iter()
            .find(|row| row["reference"] == reference && row["kind"] == "SOURCE")
            .ok_or_else(|| invalid("complete expanded source has no immutable Work row"))?;
        insert_record(&mut records, &reference, row)?;
    }
    Ok(records.into_values().collect())
}

fn insert_record(
    records: &mut BTreeMap<String, Value>,
    reference: &str,
    row: Value,
) -> Result<(), ClewError> {
    // Navigation annotations can vary with a page's selected companions.
    // Bind the audit to the complete captured record, not those annotations.
    let row = json!({"kind":row["kind"],"id":row["id"],"record":row["record"]});
    let record = json!({"label":reference,"kind":row["kind"],"id":row["id"],"workReference":reference,
        "recordDigest":digest(&row)?,"deliveredToAuthor":true,"row":row});
    if records
        .get(reference)
        .is_some_and(|existing| existing != &record)
    {
        return Err(invalid("expanded reference has conflicting exact records"));
    }
    records.insert(reference.into(), record);
    Ok(())
}

/// Replay the immutable page algorithm without writing or trusting saved text.
fn validate_page(work: &Work, page: &Value, receipt: &ReadReceipt) -> Result<(), ClewError> {
    if receipt.selection.untracked_reads {
        return Err(invalid("untracked pages cannot enter operation evidence"));
    }
    let items = work::audit_rows(work, &receipt.selection)?;
    let membership = digest(
        &items
            .iter()
            .map(|row| json!([row["kind"], row["id"]]))
            .collect::<Vec<_>>(),
    )?;
    let mut selection = receipt.selection.clone();
    selection.cursor = None;
    selection.untracked_reads = false;
    let binding = digest(&(work.id.as_str(), &selection))?;
    let prefix = &binding[7..];
    let start = match receipt.selection.cursor.as_deref() {
        None => 0,
        Some(cursor) => {
            let (owner, offset) = cursor
                .split_once(':')
                .ok_or_else(|| invalid("expanded page cursor invalid"))?;
            if owner != prefix {
                return Err(invalid("expanded page cursor belongs to foreign selection"));
            }
            offset
                .parse::<usize>()
                .map_err(|_| invalid("expanded page offset invalid"))?
        }
    };
    if start > items.len() {
        return Err(invalid("expanded page offset out of range"));
    }
    let mut expected = json!({"schema":"codeclew-documentation-work-page/1.0","work":work.id,
        "subject":work.subject,"audience":work.request.audience,"contextDigest":work.checked.context_digest,
        "inputDigest":work.checked.input_digest,"authority":"IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
        "influenceCoverage":"RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED","membershipDigest":membership,
        "total":items.len(),"items":[],"omitted":[],"nextCursor":null,
        "documentationLanguage":work.request.documentation_language()});
    if work.subject.starts_with("scenario:") {
        expected["subjectReference"] =
            json!({"reference":work.subject,"referenceRoles":["operation","gap"]});
    }
    if let Some(snapshot) = &work.snapshot {
        expected["snapshot"] = json!(snapshot);
    }
    if let Some(profile) = &work.request.context_profile {
        expected["contextProfile"] = json!(profile);
    }
    let mut out = Vec::new();
    let mut omitted = Vec::new();
    let mut supplied = Vec::new();
    let mut consumed = start;
    for item in items
        .iter()
        .skip(start)
        .take(work.request.max_items as usize)
    {
        let mut candidate = expected.clone();
        let mut trial = out.clone();
        trial.push(item.clone());
        candidate["items"] = json!(trial);
        if bytes(&candidate)?.len() + 512 > work.request.max_bytes {
            if !out.is_empty() || !omitted.is_empty() {
                break;
            }
            omitted.push(json!({"index":consumed,"kind":item["kind"],"id":item["id"],"reference":item["reference"],"reason":"ITEM_EXCEEDS_WORK_BYTE_BUDGET"}));
            expected["omitted"] = json!(omitted);
            consumed += 1;
            break;
        }
        if let Some(reference) = item["reference"].as_str() {
            supplied.push(reference.to_owned());
        }
        out.push(item.clone());
        consumed += 1;
    }
    expected["items"] = json!(out);
    if consumed < items.len() {
        expected["nextCursor"] = json!(format!("{prefix}:{consumed}"));
    }
    let result_digest = digest(&expected)?;
    if receipt.result_digest != result_digest
        || receipt.membership_digest != membership
        || receipt.supplied != supplied
        || receipt.omitted != omitted
        || receipt.next_cursor != expected["nextCursor"].as_str().map(str::to_owned)
    {
        return Err(invalid(
            "expanded page receipt differs from immutable Work delivery",
        ));
    }
    expected["receiptDigest"] = json!(result_digest);
    if page != &expected || bytes(page)?.len() + 1 > work.request.max_bytes {
        return Err(invalid(
            "expanded page differs from exact immutable Work records",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::work::{Handle, Query, QueryProjection, Selection};
    use super::*;

    fn fixture(text: &str, max_bytes: usize) -> (tempfile::TempDir, Repository, Work) {
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Expanded operation fixture").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let mut work = work::api_contract_tests::endpoint_context_fixture();
        work.request.authoring_contract =
            Some(operation_answer::EXPANDING_AUTHORING_CONTRACT.into());
        work.request.max_bytes = max_bytes;
        let mut source = work.checked.services["orders"]
            .sources
            .values()
            .next()
            .unwrap()
            .clone();
        source.id = "frontier-source".into();
        source.file = "Frontier.java".into();
        source.text = text.into();
        source.text_digest = crate::canonical::hash_bytes(text.as_bytes());
        source.occurrence = None;
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .insert(source.id.clone(), source);
        work.handles.insert(
            "frontier-source-ref".into(),
            Handle {
                kind: "SOURCE".into(),
                id: "frontier-source".into(),
            },
        );
        let mut symbol = work.checked.dependencies["endpoint-declaration"].clone();
        symbol.id = "frontier-symbol".into();
        symbol.symbol = "method:class:orders.Frontier#run()V".into();
        symbol.normalized = json!({"schema":"codeclew-java-compiler-fact/1.0","declarationKind":"METHOD",
            "symbolIdentity":symbol.symbol,"ownerIdentity":"class:orders.Frontier","scope":":main","name":"run"});
        symbol.source_ids = vec!["frontier-source".into()];
        symbol.digest = digest(&symbol.normalized).unwrap();
        work.influence
            .insert(symbol.id.clone(), symbol.digest.clone());
        work.checked
            .dependencies
            .insert(symbol.id.clone(), symbol.clone());
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(symbol.id.clone(), symbol);
        work.handles.insert(
            "frontier-symbol-ref".into(),
            Handle {
                kind: "DEPENDENCY".into(),
                id: "frontier-symbol".into(),
            },
        );
        work::api_contract_tests::persist_operation_fixture(&repo, &mut work);
        (temp, repo, work)
    }

    fn reseal(packet: &mut Value) {
        let appendix = &mut packet["contextDelivery"];
        appendix.as_object_mut().unwrap().remove("deliveredDigest");
        appendix["deliveredDigest"] = json!(digest(appendix).unwrap());
        packet.as_object_mut().unwrap().remove("packetDigest");
        packet["packetDigest"] = json!(digest(packet).unwrap());
    }

    fn render_expanded_source(packet: &Value, audit: &Value) -> operation_answer::RenderedAnswer {
        let answer = json!({"schema":"codeclew-operation-answer/1.1", "packetDigest":packet["packetDigest"],
            "title":"Expanded source evidence", "summary":{"text":"The captured helper source provides the local boundary.","evidence":["frontier-source-ref"]},
            "steps":[{"kind":"action","meaning":{"text":"Keep the retained expression and mutation order.","evidence":["frontier-source-ref"]}}],
            "preparations":[],"uncertainties":[]});
        operation_answer::validate_and_render(packet, audit, answer).unwrap()
    }

    #[test]
    fn exact_expansion_deduplicates_and_preserves_guard_null_operand_mutation_bytes() {
        let text = "class Frontier { void run() { if (task == null || !task.eligible) return; attempts++; String value = first != null ? first.name : fallback(); gateway.deliver(value); state = acknowledged; } }";
        let (_temp, repo, work) = fixture(text, 32768);
        let (base, base_audit) = initial(&work).unwrap();
        assert_eq!(
            (base.clone(), base_audit),
            operation_packet::build(&work).unwrap()
        );
        let page = work::read_loaded(
            &repo,
            &work,
            Selection {
                references: vec!["frontier-symbol-ref".into(), "frontier-source-ref".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let (expanded, audit) =
            extend(&repo, &work, &base, &[page.clone(), page.clone()], &[]).unwrap();
        assert_eq!(
            expanded["contextDelivery"]["pages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(expanded["citations"].get("frontier-symbol-ref").is_some());
        assert!(expanded["citations"].get("frontier-source-ref").is_some());
        let rows = expanded["contextDelivery"]["pages"][0]["items"]
            .as_array()
            .unwrap();
        assert!(rows.iter().any(|row| row["record"]["text"] == text));
        for key in [
            "sourceDataContext",
            "methods",
            "fields",
            "sourceContexts",
            "methodSources",
        ] {
            assert_eq!(expanded[key], base[key]);
        }
        assert_eq!(
            audit,
            operation_packet::audit_saved_packet(&work, &expanded).unwrap()
        );
        assert_eq!(
            extend(&repo, &work, &expanded, std::slice::from_ref(&page), &[])
                .unwrap()
                .0,
            expanded
        );
        let review = context(&repo, &work, &[page], &[]).unwrap();
        assert_eq!(review, expanded["contextDelivery"]);
        assert!(base.get("contextDelivery").is_none());
        let rendered = render_expanded_source(&expanded, &audit);
        assert!(rendered.html.contains("Frontier.java"));
        assert!(rendered.html.contains("href=\"#source-"));
        assert!(rendered.html.contains("task == null || !task.eligible"));
        assert!(rendered.markdown.contains(text));
    }

    #[test]
    fn saved_expansion_rejects_forged_source_foreign_receipts_and_navigation() {
        let (_temp, repo, work) = fixture(
            "class Frontier { void run() { if (x == null || !ready) return; attempts++; deliver(); } }",
            32768,
        );
        let (base, _) = initial(&work).unwrap();
        let page = work::read_loaded(
            &repo,
            &work,
            Selection {
                references: vec!["frontier-source-ref".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let (expanded, _) = extend(&repo, &work, &base, &[page], &[]).unwrap();
        let mut forged = expanded.clone();
        forged["contextDelivery"]["pages"][0]["items"][0]["record"]["text"] =
            json!("invented delivery success");
        // Keep every attacker-controlled hash and presentation consistent.
        // Only replay against immutable Work can reject this substitution.
        forged["contextDelivery"]["pages"][0]
            .as_object_mut()
            .unwrap()
            .remove("receiptDigest");
        let page_digest = digest(&forged["contextDelivery"]["pages"][0]).unwrap();
        forged["contextDelivery"]["pages"][0]["receiptDigest"] = json!(page_digest);
        forged["contextDelivery"]["pageReceipts"][0]["resultDigest"] = json!(page_digest);
        let pages = forged["contextDelivery"]["pages"].as_array().unwrap();
        forged["contextDelivery"]["presentation"] = job_context::present(pages, &[]);
        reseal(&mut forged);
        assert!(operation_packet::audit_saved_packet(&work, &forged).is_err());
        let mut foreign = expanded.clone();
        foreign["contextDelivery"]["work"] = json!("foreign-work");
        reseal(&mut foreign);
        assert!(operation_packet::audit_saved_packet(&work, &foreign).is_err());
        let navigation = work::read_loaded(
            &repo,
            &work,
            Selection {
                query: Some(Query {
                    kind: "SYMBOL".into(),
                    symbol_contains: "Frontier".into(),
                    projection: QueryProjection::Navigation,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let (navigated, _) = extend(&repo, &work, &base, &[navigation], &[]).unwrap();
        assert!(
            navigated["contextDelivery"]["citations"]
                .get("frontier-symbol-ref")
                .is_none()
        );
        assert!(navigated["citations"].get("frontier-symbol-ref").is_none());
        let mut false_citation = navigated;
        false_citation["citations"]["frontier-symbol-ref"] = json!("navigation is evidence");
        false_citation["contextDelivery"]["citations"]["frontier-symbol-ref"] =
            json!("navigation is evidence");
        reseal(&mut false_citation);
        assert!(operation_packet::audit_saved_packet(&work, &false_citation).is_err());
        let mut legacy = work.clone();
        legacy.request.authoring_contract = Some(operation_answer::AUTHORING_CONTRACT.into());
        assert!(operation_packet::audit_saved_packet(&legacy, &expanded).is_err());
        let mut changed_base = expanded.clone();
        changed_base["methodSources"][0]["text"] = json!("guard omitted");
        reseal(&mut changed_base);
        assert!(operation_packet::audit_saved_packet(&work, &changed_base).is_err());
    }

    #[test]
    fn oversized_source_requires_all_exact_contiguous_parts_and_matching_ledger() {
        let text = format!(
            "class Frontier {{ void run() {{ {} }} }}",
            "if (task == null || !ready) return; attempts++; deliver();\n".repeat(180)
        );
        let (_temp, repo, work) = fixture(&text, 4096);
        let (base, _) = initial(&work).unwrap();
        let mut parts = Vec::new();
        let mut cursor = None;
        loop {
            let part = work_parts::read_part_loaded(
                &repo,
                &work,
                work_parts::SourcePartRequest {
                    schema: work_parts::REQUEST_SCHEMA.into(),
                    reference: "frontier-source-ref".into(),
                    cursor,
                },
            )
            .unwrap();
            cursor = part["nextCursor"].as_str().map(str::to_owned);
            parts.push(part);
            if cursor.is_none() {
                break;
            }
        }
        assert!(parts.len() > 1);
        assert!(extend(&repo, &work, &base, &[], &parts[..1]).is_err());
        let (expanded, audit) = extend(&repo, &work, &base, &[], &parts).unwrap();
        assert!(expanded["citations"].get("frontier-source-ref").is_some());
        let rendered = render_expanded_source(&expanded, &audit);
        assert!(rendered.html.contains("Frontier.java"));
        assert!(rendered.markdown.contains(&text));
        let mut forged = expanded.clone();
        forged["contextDelivery"]["sourceParts"][0]["text"] = json!("invented");
        reseal(&mut forged);
        assert!(operation_packet::audit_saved_packet(&work, &forged).is_err());
        let mut gap = expanded.clone();
        gap["contextDelivery"]["sourceParts"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        gap["contextDelivery"]["sourcePartReceipts"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        reseal(&mut gap);
        assert!(operation_packet::audit_saved_packet(&work, &gap).is_err());
        let mut unrecorded = parts;
        unrecorded[0]["receiptDigest"] = json!("not-recorded");
        assert!(context(&repo, &work, &[], &unrecorded).is_err());
    }
}
