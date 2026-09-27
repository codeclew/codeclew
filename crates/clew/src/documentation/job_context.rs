//! Compact presentation context for documentation author and reviewer jobs.
//!
//! This module projects only the page rows and source parts passed to it. It
//! does not load Work state or resolve navigation handles.

use super::{model::Source, process_context};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Build the shared evidence envelope used by documentation author and review
/// jobs. Exact duplicate page rows are removed from this presentation only;
/// the page receipts and the inputs passed to this function remain unchanged.
pub(super) fn present(pages: &[Value], source_parts: &[Value]) -> Value {
    let (mut pages, presentation) = project_pages(pages);
    let callables = callable_inventory(&pages, source_parts);
    let baseline = evidence_context(&pages, source_parts, &callables, presentation.as_ref());
    let mut summary = ProjectionSummary::default();
    alias_source_texts(&mut pages, &mut summary);
    if summary.is_empty() {
        return baseline;
    }
    mark_projected_pages(&mut pages, &summary);
    let projected_presentation = merge_presentation(presentation, &summary);
    let projected = evidence_context(
        &pages,
        source_parts,
        &callables,
        Some(&projected_presentation),
    );
    if compact_json_len(&projected) < compact_json_len(&baseline) {
        projected
    } else {
        baseline
    }
}

fn evidence_context(
    pages: &[Value],
    source_parts: &[Value],
    callables: &[Value],
    presentation: Option<&Value>,
) -> Value {
    let mut context = Map::new();
    context.insert("pages".into(), json!(pages));
    context.insert("sourceParts".into(), json!(source_parts));
    context.insert("callables".into(), json!(callables));
    context.insert(
        "callablesMeaning".into(),
        json!("Each entry is a captured SYMBOL declaration; declarationKind distinguishes callable and non-callable declarations when available."),
    );
    if let Some(presentation) = presentation {
        context.insert("presentation".into(), presentation.clone());
    }
    Value::Object(context)
}

#[derive(Default)]
struct ProjectionSummary {
    source_text_alias_count: usize,
    source_aliases_by_page: BTreeMap<usize, PageProjectionCounts>,
}

#[derive(Default)]
struct PageProjectionCounts {
    source_text_aliases: usize,
}

impl ProjectionSummary {
    fn is_empty(&self) -> bool {
        self.source_text_alias_count == 0
    }

    fn page_counts(&mut self, page_index: usize) -> &mut PageProjectionCounts {
        self.source_aliases_by_page.entry(page_index).or_default()
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SourceOccurrenceKey {
    Explicit { snapshot: String, blob: String },
    Evidence(String),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceGroupKey {
    service: String,
    revision: String,
    file: String,
    authority: String,
    occurrence: SourceOccurrenceKey,
}

#[derive(Clone)]
struct SourceCandidate {
    page_index: usize,
    item_index: usize,
    reference: String,
    source: Source,
}

fn alias_source_texts(pages: &mut [Value], summary: &mut ProjectionSummary) {
    let mut groups = BTreeMap::<SourceGroupKey, Vec<SourceCandidate>>::new();
    for (page_index, page) in pages.iter().enumerate() {
        for (item_index, item) in page_items(std::slice::from_ref(page)).enumerate() {
            let Some(reference) = item["reference"].as_str() else {
                continue;
            };
            let Some(source) = valid_source_row(item) else {
                continue;
            };
            let occurrence = match &source.occurrence {
                Some(occurrence) => SourceOccurrenceKey::Explicit {
                    snapshot: occurrence.snapshot.clone(),
                    blob: occurrence.blob.clone(),
                },
                None => SourceOccurrenceKey::Evidence(source.evidence_digest.clone()),
            };
            let key = SourceGroupKey {
                service: source.service.clone(),
                revision: source.revision.clone(),
                file: source.file.clone(),
                authority: source.authority.clone(),
                occurrence,
            };
            groups.entry(key).or_default().push(SourceCandidate {
                page_index,
                item_index,
                reference: reference.to_owned(),
                source,
            });
        }
    }

    for candidates in groups.values_mut() {
        candidates.sort_by(|a, b| {
            b.source
                .text
                .len()
                .cmp(&a.source.text.len())
                .then_with(|| a.reference.cmp(&b.reference))
                .then_with(|| a.page_index.cmp(&b.page_index))
                .then_with(|| a.item_index.cmp(&b.item_index))
        });
        let mut retained = Vec::<SourceCandidate>::new();
        for candidate in candidates.iter() {
            let covered = retained.iter().find_map(|container| {
                process_context::covered_text(&container.source, &candidate.source)
                    .map(|range| (container, range))
            });
            if let Some((container, (start, end))) = covered {
                let location = json!({
                    "pageIndex":container.page_index,
                    "itemIndex":container.item_index,
                    "reference":container.reference
                });
                let alias = json!({
                    "target":location,
                    "startByte":start,
                    "endByte":end
                });
                let item = &mut pages[candidate.page_index]["items"][candidate.item_index];
                let original_size = compact_json_len(item);
                let mut projected_item = item.clone();
                if let Some(record) = projected_item["record"].as_object_mut() {
                    record.remove("text");
                    record.insert("displayTextAlias".into(), alias);
                } else {
                    retained.push(candidate.clone());
                    continue;
                }
                if compact_json_len(&projected_item) < original_size {
                    *item = projected_item;
                    summary.source_text_alias_count += 1;
                    summary
                        .page_counts(candidate.page_index)
                        .source_text_aliases += 1;
                } else {
                    retained.push(candidate.clone());
                }
            } else {
                retained.push(candidate.clone());
            }
        }
    }
}

fn valid_source_row(item: &Value) -> Option<Source> {
    if item["kind"] != "SOURCE"
        || item["id"].as_str().is_none_or(str::is_empty)
        || item["reference"].as_str().is_none_or(str::is_empty)
    {
        return None;
    }
    let source: Source = serde_json::from_value(item.get("record")?.clone()).ok()?;
    if source.id != item["id"].as_str()?
        || source.service.is_empty()
        || source.revision.is_empty()
        || source.file.is_empty()
        || source.authority.is_empty()
        || source.evidence_digest.is_empty()
        || source.text.is_empty()
        || source.start_line == 0
        || source.end_line < source.start_line
        || source.end_line - source.start_line + 1 != source.text.split('\n').count() as u64
        || crate::canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
    {
        return None;
    }
    if let Some(occurrence) = &source.occurrence
        && (occurrence.snapshot.is_empty()
            || occurrence.blob.is_empty()
            || occurrence.end_byte.checked_sub(occurrence.start_byte) != Some(source.text.len()))
    {
        return None;
    }
    Some(source)
}

fn mark_projected_pages(pages: &mut [Value], summary: &ProjectionSummary) {
    for (page_index, counts) in &summary.source_aliases_by_page {
        let page = &mut pages[*page_index];
        if let Some(object) = page.as_object_mut()
            && let Some(schema) = object.remove("schema")
        {
            let projection = object
                .entry("displayProjection".to_owned())
                .or_insert_with(|| json!({}));
            if projection.get("sourceSchema").is_none() {
                projection["sourceSchema"] = schema;
            }
        }
        if page["displayProjection"].is_null() {
            page["displayProjection"] = json!({});
        }
        let projection = page["displayProjection"]
            .as_object_mut()
            .expect("displayProjection object inserted above");
        projection
            .entry("kind".to_owned())
            .or_insert_with(|| json!("DISPLAY_ONLY_PAGE_REPRESENTATION"));
        if counts.source_text_aliases > 0 {
            projection.insert(
                "sourceTextAliasCount".into(),
                json!(counts.source_text_aliases),
            );
        }
        projection.insert(
            "authority".into(),
            json!("DISPLAY_PROJECTION_ONLY_ORIGINAL_PAGE_RECEIPT_UNCHANGED"),
        );
    }
}

fn merge_presentation(base: Option<Value>, summary: &ProjectionSummary) -> Value {
    let mut presentation = base
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    presentation.insert(
        "representationProjection".into(),
        json!({
            "kind":"DISPLAY_ONLY_REPRESENTATION_PROJECTION",
            "sourceTextAliasCount":summary.source_text_alias_count,
            "authority":"DISPLAY_PROJECTION_ONLY_ORIGINAL_PAGE_RECEIPTS_UNCHANGED",
            "sourceTextAliasInstruction":"For SOURCE rows with displayTextAlias, reconstruct record.text by slicing the delivered target SOURCE record.text at the stated UTF-8 byte offsets. The original SOURCE identity and citation reference remain on the alias row."
        }),
    );
    Value::Object(presentation)
}

fn compact_json_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |encoded| encoded.len())
}

#[derive(Clone, Copy)]
struct RowLocation {
    page_index: usize,
    source_item_index: usize,
    presented_item_index: usize,
}

fn project_pages(pages: &[Value]) -> (Vec<Value>, Option<Value>) {
    let mut seen = HashMap::<&Value, RowLocation>::new();
    let mut output = Vec::with_capacity(pages.len());
    let mut omitted_total = 0usize;
    let mut duplicate_locations = Vec::new();

    for (page_index, page) in pages.iter().enumerate() {
        let Some(original_items) = page.get("items").and_then(Value::as_array) else {
            output.push(page.clone());
            continue;
        };

        let mut retained_items = Vec::with_capacity(original_items.len());
        let mut omitted_here = Vec::new();
        for (item_index, item) in original_items.iter().enumerate() {
            if let Some(first_location) = seen.get(&item) {
                omitted_total = omitted_total.saturating_add(1);
                let retained_at = json!({
                    "pageIndex":first_location.page_index,
                    "itemIndex":first_location.presented_item_index,
                    "sourceItemIndex":first_location.source_item_index
                });
                omitted_here.push(json!({
                    "sourceItemIndex":item_index,
                    "retainedAt":retained_at
                }));
                duplicate_locations.push(json!({
                    "omittedAt":{"pageIndex":page_index,"sourceItemIndex":item_index},
                    "retainedAt":retained_at
                }));
            } else {
                seen.insert(
                    item,
                    RowLocation {
                        page_index,
                        source_item_index: item_index,
                        presented_item_index: retained_items.len(),
                    },
                );
                retained_items.push(item.clone());
            }
        }

        if !omitted_here.is_empty() {
            let mut projected_page = page.clone();
            projected_page["items"] = json!(retained_items);
            let source_schema = projected_page.get("schema").cloned();
            if let Some(object) = projected_page.as_object_mut() {
                object.remove("schema");
            }
            let mut display_projection = json!({
                "kind":"EXACT_DUPLICATE_PAGE_ITEMS_OMITTED",
                "sourceItemCount":original_items.len(),
                "presentedItemCount":original_items.len().saturating_sub(omitted_here.len()),
                "omittedDuplicateCount":omitted_here.len(),
                "omittedDuplicates":omitted_here
            });
            if let Some(source_schema) = source_schema {
                display_projection["sourceSchema"] = source_schema;
            }
            projected_page["displayProjection"] = display_projection;
            output.push(projected_page);
        } else {
            output.push(page.clone());
        }
    }

    if omitted_total == 0 {
        return (output, None);
    }

    let original_item_count = pages
        .iter()
        .filter_map(|page| page.get("items").and_then(Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    (
        output,
        Some(json!({
            "kind":"EXACT_DUPLICATE_PAGE_ITEMS_OMITTED",
            "sourcePageCount":pages.len(),
            "sourceItemCount":original_item_count,
            "presentedItemCount":original_item_count.saturating_sub(omitted_total),
            "omittedDuplicateCount":omitted_total,
            "omittedDuplicates":duplicate_locations,
            "authority":"DISPLAY_PROJECTION_ONLY_ORIGINAL_PAGE_RECEIPTS_REMAIN_UNCHANGED"
        })),
    )
}

fn callable_inventory(pages: &[Value], source_parts: &[Value]) -> Vec<Value> {
    let mut source_rows_by_reference = HashMap::<String, Vec<&Value>>::new();
    for item in page_items(pages) {
        if item["kind"] == "SOURCE" {
            if let Some(reference) = item["reference"].as_str() {
                source_rows_by_reference
                    .entry(reference.to_owned())
                    .or_default()
                    .push(item);
            }
        }
    }
    let mut source_parts_by_reference = HashMap::<String, Vec<&Value>>::new();
    for part in source_parts {
        if let Some(reference) = part["reference"].as_str() {
            source_parts_by_reference
                .entry(reference.to_owned())
                .or_default()
                .push(part);
        }
    }

    page_items(pages)
        .filter(|item| item["kind"] == "DEPENDENCY" && item["record"]["kind"] == "SYMBOL")
        .map(|item| callable_entry(item, &source_rows_by_reference, &source_parts_by_reference))
        .collect()
}

fn page_items(pages: &[Value]) -> impl Iterator<Item = &Value> {
    pages
        .iter()
        .flat_map(|page| page["items"].as_array().into_iter().flatten())
}

fn callable_entry(
    item: &Value,
    source_rows_by_reference: &HashMap<String, Vec<&Value>>,
    source_parts_by_reference: &HashMap<String, Vec<&Value>>,
) -> Value {
    let record = &item["record"];
    let normalized = &record["normalized"];
    let navigation_references = string_set(&item["sourceReferences"]);

    let related_source_rows: Vec<_> = navigation_references
        .iter()
        .filter_map(|reference| source_rows_by_reference.get(reference))
        .flat_map(|rows| rows.iter().map(|source| compact_source_record(source)))
        .collect();
    let related_part_rows: Vec<_> = navigation_references
        .iter()
        .filter_map(|reference| source_parts_by_reference.get(reference))
        .flat_map(|parts| parts.iter().map(|part| compact_source_part(part)))
        .collect();
    let supplied_references: BTreeSet<_> = related_source_rows
        .iter()
        .filter_map(|source| source["reference"].as_str().map(str::to_owned))
        .chain(
            related_part_rows
                .iter()
                .filter_map(|part| part["reference"].as_str().map(str::to_owned)),
        )
        .collect();
    let navigation_only_references: Vec<_> = navigation_references
        .difference(&supplied_references)
        .cloned()
        .collect();
    let source_parts_available = !related_part_rows.is_empty();

    let source_tokens = normalized["sourceTokens"].as_array();
    let declaration_tokens_available = source_tokens.is_some_and(|tokens| !tokens.is_empty());

    let mut entry = Map::new();
    insert_if_present(&mut entry, "dependencyReference", &item["reference"]);
    insert_if_present(
        &mut entry,
        "dependencyId",
        record.get("id").unwrap_or(&item["id"]),
    );
    insert_if_present(&mut entry, "dependencyDigest", &record["digest"]);
    insert_if_present(&mut entry, "symbol", &record["symbol"]);
    insert_if_present(&mut entry, "service", &record["service"]);
    insert_if_present(&mut entry, "symbolIdentity", &normalized["symbolIdentity"]);
    insert_if_present(
        &mut entry,
        "compilerCallableId",
        &normalized["compilerCallableId"],
    );
    insert_if_present(
        &mut entry,
        "declarationKind",
        &normalized["declarationKind"],
    );
    insert_if_present(&mut entry, "syntaxKind", &normalized["syntaxKind"]);
    insert_if_present(&mut entry, "owner", &normalized["ownerIdentity"]);
    insert_if_present(&mut entry, "name", &normalized["name"]);
    insert_if_present(&mut entry, "scope", &normalized["scope"]);
    let mut provenance = Map::new();
    insert_if_present(
        &mut provenance,
        "dependencyId",
        record.get("id").unwrap_or(&item["id"]),
    );
    insert_if_present(&mut provenance, "observationDigest", &record["digest"]);
    insert_if_present(&mut provenance, "kind", &record["kind"]);
    if !provenance.is_empty() {
        entry.insert("provenance".into(), Value::Object(provenance));
    }
    entry.insert(
        "deliveredDeclarationTokens".into(),
        json!({
            "status":if declaration_tokens_available {"AVAILABLE"} else {"NOT_AVAILABLE"},
            "tokenCount":source_tokens.map_or(0, Vec::len),
            "meaning":"TOKENIZED_DECLARATION_CONTENT_DOES_NOT_ESTABLISH_COMPLETE_CALLABLE_BODY"
        }),
    );
    entry.insert("relatedSourceRecords".into(), json!(related_source_rows));
    entry.insert("relatedSourceParts".into(), json!(related_part_rows));
    entry.insert(
        "navigationOnlySourceReferences".into(),
        json!(navigation_only_references),
    );
    entry.insert(
        "sourcePartsAvailability".into(),
        json!(if source_parts_available {
            "PRESENT_FOR_RELATED_SOURCE_REFERENCES"
        } else {
            "NONE_FOR_RELATED_SOURCE_REFERENCES"
        }),
    );
    entry.insert(
        "callableBodyCompleteness".into(),
        json!("PARTIAL_OR_UNKNOWN"),
    );
    entry.insert(
        "immediateTargetHints".into(),
        json!(immediate_target_hints(normalized)),
    );
    entry.insert(
        "immediateTargetHintMeaning".into(),
        json!("CAPTURED_COMPILER_EXACT_TARGET_IDENTITIES_FOR_NAVIGATION_ONLY"),
    );
    Value::Object(entry)
}

fn compact_source_record(item: &Value) -> Value {
    let record = &item["record"];
    let mut result = Map::new();
    insert_if_present(&mut result, "reference", &item["reference"]);
    insert_if_present(&mut result, "sourceId", &item["id"]);
    for field in [
        "revision",
        "file",
        "startLine",
        "endLine",
        "textDigest",
        "evidenceDigest",
        "authority",
    ] {
        insert_if_present(&mut result, field, &record[field]);
    }
    Value::Object(result)
}

fn compact_source_part(part: &Value) -> Value {
    let mut result = Map::new();
    for field in [
        "reference",
        "sourceId",
        "snapshot",
        "recordDigest",
        "startByte",
        "endByte",
        "totalTextBytes",
        "fragmentDigest",
        "receiptDigest",
    ] {
        insert_if_present(&mut result, field, &part[field]);
    }
    insert_if_present(&mut result, "sourceRevision", &part["source"]["revision"]);
    insert_if_present(&mut result, "sourceFile", &part["source"]["file"]);
    Value::Object(result)
}

fn immediate_target_hints(normalized: &Value) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    normalized["documentation"]["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| {
            matches!(event["kind"].as_str(), Some("CALL" | "CONSTRUCT"))
                && event["resolution"] == "COMPILER_EXACT"
        })
        .filter_map(|event| {
            let kind = event["kind"].as_str()?;
            let target = event["target"].as_str()?;
            if target.is_empty() || !seen.insert((kind.to_owned(), target.to_owned())) {
                return None;
            }
            Some(json!({"eventKind":kind,"targetIdentity":target}))
        })
        .collect()
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn insert_if_present(output: &mut Map<String, Value>, name: &str, value: &Value) {
    if !value.is_null() {
        output.insert(name.to_owned(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(receipt: &str, items: Vec<Value>) -> Value {
        json!({
            "schema":"codeclew-documentation-work-page/1.0",
            "pageId":receipt,
            "receiptDigest":receipt,
            "items":items
        })
    }

    fn symbol_row(reference: &str, source_references: &[&str], normalized: Value) -> Value {
        json!({
            "kind":"DEPENDENCY",
            "id":"dep-1",
            "reference":reference,
            "sourceReferences":source_references,
            "record":{
                "id":"dep-1",
                "kind":"SYMBOL",
                "service":"payments",
                "symbol":"ledger.Writer.flush",
                "digest":"dependency-digest",
                "normalized":normalized
            }
        })
    }

    fn source_row(
        reference: &str,
        id: &str,
        service: &str,
        revision: &str,
        authority: &str,
        text: &str,
        start_line: u64,
        occurrence_start: usize,
        blob: &str,
    ) -> Value {
        let end_line = start_line + text.lines().count() as u64 - 1;
        json!({
            "kind":"SOURCE",
            "id":id,
            "reference":reference,
            "referenceRoles":["evidence","operation"],
            "record":{
                "id":id,
                "service":service,
                "revision":revision,
                "file":"Worker.java",
                "startLine":start_line,
                "endLine":end_line,
                "text":text,
                "textDigest":crate::canonical::hash_bytes(text.as_bytes()),
                "evidenceDigest":format!("evidence-{id}"),
                "authority":authority,
                "occurrence":{
                    "snapshot":"snapshot-a",
                    "blob":blob,
                    "startByte":occurrence_start,
                    "endByte":occurrence_start + text.len()
                },
                "url":null
            }
        })
    }

    fn reconstruct_source_alias(pages: &[Value], row: &Value) -> Option<String> {
        let alias = &row["record"]["displayTextAlias"];
        let page_index = alias["target"]["pageIndex"].as_u64()? as usize;
        let item_index = alias["target"]["itemIndex"].as_u64()? as usize;
        let reference = alias["target"]["reference"].as_str()?;
        let target = pages
            .get(page_index)?
            .get("items")?
            .as_array()?
            .get(item_index)?;
        if target["kind"] != "SOURCE" || target["reference"] != reference {
            return None;
        }
        let text = target["record"]["text"].as_str()?;
        let start = alias["startByte"].as_u64()? as usize;
        let end = alias["endByte"].as_u64()? as usize;
        String::from_utf8(text.as_bytes().get(start..end)?.to_vec()).ok()
    }

    fn evidence_reference_set(evidence: &Value) -> BTreeSet<String> {
        let mut references = BTreeSet::new();
        for item in evidence["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|page| page["items"].as_array().into_iter().flatten())
        {
            if let Some(reference) = item["reference"].as_str() {
                references.insert(reference.to_owned());
            }
            for field in ["sourceReferences", "dependencyReferences"] {
                references.extend(
                    item[field]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
            }
        }
        references.extend(
            evidence["sourceParts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|part| part["reference"].as_str())
                .map(str::to_owned),
        );
        references
    }

    fn citation_role_rows(pages: &[Value]) -> BTreeSet<String> {
        pages
            .iter()
            .flat_map(|page| page["items"].as_array().into_iter().flatten())
            .filter_map(|item| {
                item.get("referenceRoles").map(|roles| {
                    serde_json::to_string(&json!({
                        "kind":item["kind"],
                        "id":item["id"],
                        "reference":item["reference"],
                        "referenceRoles":roles
                    }))
                    .unwrap()
                })
            })
            .collect()
    }

    fn merge_audited_evidence(original: &Value, projected: &Value) -> Value {
        let mut evidence = original.as_object().cloned().unwrap_or_default();
        for key in ["pages", "sourceParts", "callables", "callablesMeaning"] {
            if let Some(value) = projected.get(key) {
                evidence.insert(key.to_owned(), value.clone());
            }
        }
        if let Some(new_presentation) = projected["presentation"].as_object() {
            let mut presentation = original["presentation"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            presentation.extend(new_presentation.clone());
            evidence.insert("presentation".into(), Value::Object(presentation));
        }
        Value::Object(evidence)
    }

    fn source_alias_count(pages: &[Value]) -> usize {
        pages
            .iter()
            .flat_map(|page| page["items"].as_array().into_iter().flatten())
            .filter(|item| {
                item["kind"] == "SOURCE" && item["record"].get("displayTextAlias").is_some()
            })
            .count()
    }

    #[test]
    fn exact_duplicate_rows_collapse_across_pages_without_changing_receipts_or_inputs() {
        let repeated = json!({
            "kind":"DEPENDENCY",
            "id":"dep-1",
            "reference":"dependency-handle",
            "record":{"kind":"SYMBOL","digest":"digest-a"}
        });
        let original_pages = vec![
            page("receipt-a", vec![repeated.clone()]),
            page(
                "receipt-b",
                vec![repeated.clone(), json!({"kind":"COVERAGE","value":1})],
            ),
        ];
        let context = present(&original_pages, &[]);

        assert_eq!(context["pages"][0]["items"].as_array().unwrap().len(), 1);
        assert_eq!(context["pages"][1]["items"].as_array().unwrap().len(), 1);
        assert_eq!(context["pages"][1]["receiptDigest"], "receipt-b");
        assert_eq!(context["pages"][1]["pageId"], "receipt-b");
        assert_eq!(
            context["pages"][1]["displayProjection"]["sourceItemCount"],
            2
        );
        assert!(context["pages"][1].get("schema").is_none());
        assert_eq!(
            context["pages"][1]["displayProjection"]["sourceSchema"],
            "codeclew-documentation-work-page/1.0"
        );
        assert_eq!(
            context["pages"][1]["displayProjection"]["omittedDuplicateCount"],
            1
        );
        assert_eq!(context["presentation"]["omittedDuplicateCount"], 1);
        assert_eq!(original_pages[1]["items"].as_array().unwrap().len(), 2);
        assert_eq!(
            original_pages[1]["schema"],
            "codeclew-documentation-work-page/1.0"
        );
        assert!(original_pages[1].get("displayProjection").is_none());
    }

    #[test]
    fn duplicate_location_uses_presented_index_after_earlier_rows_collapse() {
        let row_a = json!({"kind":"COVERAGE","id":"a"});
        let row_b = json!({"kind":"COVERAGE","id":"b"});
        let context = present(
            &[
                page("receipt-a", vec![row_a.clone(), row_a, row_b.clone()]),
                page("receipt-b", vec![row_b]),
            ],
            &[],
        );

        assert_eq!(context["pages"][0]["items"].as_array().unwrap().len(), 2);
        assert_eq!(context["pages"][1]["items"].as_array().unwrap().len(), 0);
        assert_eq!(
            context["pages"][1]["displayProjection"]["omittedDuplicates"][0]["retainedAt"],
            json!({"pageIndex":0,"itemIndex":1,"sourceItemIndex":2})
        );
    }

    #[test]
    fn rows_with_same_ids_but_different_revision_digest_body_or_handle_survive() {
        let rows = vec![
            json!({"kind":"SOURCE","id":"source-1","reference":"source-h1","record":{"revision":"r1","textDigest":"t1","text":"body"}}),
            json!({"kind":"SOURCE","id":"source-1","reference":"source-h1","record":{"revision":"r2","textDigest":"t1","text":"body"}}),
            json!({"kind":"SOURCE","id":"source-1","reference":"source-h1","record":{"revision":"r1","textDigest":"t2","text":"body"}}),
            json!({"kind":"SOURCE","id":"source-1","reference":"source-h1","record":{"revision":"r1","textDigest":"t1","text":"different body"}}),
            json!({"kind":"SOURCE","id":"source-1","reference":"source-h2","record":{"revision":"r1","textDigest":"t1","text":"body"}}),
        ];
        let context = present(&[page("receipt", rows.clone())], &[]);

        assert_eq!(context["pages"][0]["items"].as_array().unwrap().len(), 5);
        assert_eq!(context["pages"][0]["items"], json!(rows));
        assert!(context.get("presentation").is_none());
    }

    #[test]
    fn covered_source_text_uses_exact_utf8_slice_and_preserves_each_identity() {
        let fragment_text = "αβ".repeat(1024);
        let body_text = format!("head\n{fragment_text}\ntail");
        let body = source_row(
            "source-body",
            "source-body-id",
            "svc",
            "rev-1",
            "COMPILER",
            &body_text,
            10,
            100,
            "blob-1",
        );
        let fragment = source_row(
            "source-fragment",
            "source-fragment-id",
            "svc",
            "rev-1",
            "COMPILER",
            &fragment_text,
            11,
            105,
            "blob-1",
        );
        let original_pages = vec![page("receipt-a", vec![body.clone(), fragment.clone()])];
        let original_parts = vec![json!({"reference":"part-ref","text":"caller input"})];
        let context = present(&original_pages, &original_parts);
        assert_eq!(context, present(&original_pages, &original_parts));

        let presented = &context["pages"][0]["items"];
        assert_eq!(presented.as_array().unwrap().len(), 2);
        assert_eq!(presented[0]["id"], "source-body-id");
        assert_eq!(presented[0]["reference"], "source-body");
        assert_eq!(presented[0]["record"]["text"], body_text);
        assert_eq!(presented[1]["id"], "source-fragment-id");
        assert_eq!(presented[1]["reference"], "source-fragment");
        assert_eq!(
            presented[1]["referenceRoles"],
            json!(["evidence", "operation"])
        );
        assert_eq!(presented[1]["record"].get("text"), None);
        assert_eq!(
            presented[1]["record"]["displayTextAlias"]["target"],
            json!({"pageIndex":0,"itemIndex":0,"reference":"source-body"})
        );
        assert_eq!(
            reconstruct_source_alias(context["pages"].as_array().unwrap(), &presented[1])
                .as_deref(),
            Some(fragment_text.as_str())
        );
        assert_eq!(
            presented[1]["record"]["textDigest"],
            fragment["record"]["textDigest"]
        );
        assert_eq!(presented[1]["referenceRoles"], fragment["referenceRoles"]);
        assert!(context["pages"][0].get("schema").is_none());
        assert_eq!(
            context["pages"][0]["displayProjection"]["sourceSchema"],
            "codeclew-documentation-work-page/1.0"
        );
        assert_eq!(context["pages"][0]["receiptDigest"], "receipt-a");
        assert!(
            original_pages[0]["items"][1]["record"]
                .get("displayTextAlias")
                .is_none()
        );
        assert_eq!(
            original_pages[0]["items"][1]["record"]["text"],
            fragment_text
        );
        assert_eq!(context["sourceParts"], json!(original_parts));
        assert_eq!(
            context["presentation"]["representationProjection"]["sourceTextAliasCount"],
            1
        );
    }

    #[test]
    fn source_text_is_retained_when_provenance_or_location_is_not_compatible() {
        let fragment_text = "needle".repeat(400);
        let body_text = format!("head\n{fragment_text}\ntail");
        let body_start = 100;
        let fragment_offset = body_text.find(&fragment_text).unwrap();
        let body = source_row(
            "body", "body-id", "svc", "rev-1", "COMPILER", &body_text, 10, body_start, "blob-1",
        );
        let make_fragment = |reference: &str,
                             id: &str,
                             service: &str,
                             revision: &str,
                             authority: &str,
                             start_line: u64,
                             offset: usize,
                             blob: &str| {
            source_row(
                reference,
                id,
                service,
                revision,
                authority,
                &fragment_text,
                start_line,
                body_start + offset,
                blob,
            )
        };
        let rows = vec![
            body,
            make_fragment(
                "a-control",
                "control-id",
                "svc",
                "rev-1",
                "COMPILER",
                11,
                fragment_offset,
                "blob-1",
            ),
            make_fragment(
                "service",
                "service-id",
                "other",
                "rev-1",
                "COMPILER",
                11,
                fragment_offset,
                "blob-1",
            ),
            make_fragment(
                "revision",
                "revision-id",
                "svc",
                "rev-2",
                "COMPILER",
                11,
                fragment_offset,
                "blob-1",
            ),
            make_fragment(
                "authority",
                "authority-id",
                "svc",
                "rev-1",
                "OTHER",
                11,
                fragment_offset,
                "blob-1",
            ),
            make_fragment(
                "occurrence",
                "occurrence-id",
                "svc",
                "rev-1",
                "COMPILER",
                11,
                fragment_offset,
                "blob-2",
            ),
            make_fragment(
                "wrong-line",
                "wrong-line-id",
                "svc",
                "rev-1",
                "COMPILER",
                12,
                fragment_offset,
                "blob-1",
            ),
            make_fragment(
                "wrong-offset",
                "wrong-offset-id",
                "svc",
                "rev-1",
                "COMPILER",
                11,
                0,
                "blob-1",
            ),
        ];
        let context = present(&[page("receipt", rows)], &[]);
        let items = context["pages"][0]["items"].as_array().unwrap();

        assert_eq!(items.len(), 8);
        assert!(items[1]["record"].get("text").is_none());
        assert!(items[1]["record"].get("displayTextAlias").is_some());
        assert!(
            items[2..]
                .iter()
                .all(|item| item["record"].get("text").is_some())
        );
        assert!(
            items[2..]
                .iter()
                .all(|item| item["record"].get("displayTextAlias").is_none())
        );
    }

    #[test]
    fn malformed_or_incomplete_source_metadata_is_left_unchanged() {
        let fragment_text = "very long source fragment ".repeat(100);
        let body_text = format!("body\n{fragment_text}\nend");
        let body = source_row(
            "body", "body-id", "svc", "rev", "COMPILER", &body_text, 1, 0, "blob",
        );
        let mut fragment = source_row(
            "fragment",
            "fragment-id",
            "svc",
            "rev",
            "COMPILER",
            &fragment_text,
            2,
            5,
            "blob",
        );
        fragment["record"]
            .as_object_mut()
            .unwrap()
            .remove("evidenceDigest");
        let context = present(&[page("receipt", vec![body, fragment.clone()])], &[]);

        assert_eq!(
            context["pages"][0]["items"][1]["record"],
            fragment["record"]
        );
        assert!(context.get("presentation").is_none());
    }

    #[test]
    #[ignore = "requires CODECLEW_JOB_AUDIT_INPUT pointing to a private documentation job packet"]
    fn audit_job_packet_from_env() {
        use std::path::{Path, PathBuf};

        let input_path = std::env::var_os("CODECLEW_JOB_AUDIT_INPUT")
            .map(PathBuf::from)
            .expect("CODECLEW_JOB_AUDIT_INPUT must name the private packet for this audit");
        let input_bytes = std::fs::read(&input_path).expect("read private job packet");
        let packet: Value = serde_json::from_slice(&input_bytes).expect("parse job packet");
        let original_evidence = packet
            .pointer("/payload/evidence")
            .cloned()
            .expect("job packet payload must contain evidence");
        let original_pages = original_evidence["pages"]
            .as_array()
            .cloned()
            .expect("evidence must contain page rows");
        let original_parts = original_evidence["sourceParts"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let input_content_digest =
            crate::canonical::bytes(&(original_pages.clone(), original_parts.clone()))
                .expect("serialize caller evidence");
        let before_size = crate::canonical::bytes(&packet)
            .expect("serialize original packet canonically")
            .len();

        let projected_context = present(&original_pages, &original_parts);
        let projected_pages = projected_context["pages"]
            .as_array()
            .expect("presentation returns pages");
        let mut reconstructed_aliases = 0usize;
        for (page_index, page) in projected_pages.iter().enumerate() {
            for row in page["items"].as_array().into_iter().flatten() {
                if row["kind"] != "SOURCE" || row["record"].get("displayTextAlias").is_none() {
                    continue;
                }
                let original_text = original_pages[page_index]["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|original| {
                        original["kind"] == "SOURCE"
                            && original["id"] == row["id"]
                            && original["reference"] == row["reference"]
                            && original["record"]["textDigest"] == row["record"]["textDigest"]
                    })
                    .and_then(|original| original["record"]["text"].as_str())
                    .expect("aliased source had original text");
                assert_eq!(
                    reconstruct_source_alias(projected_pages, row).as_deref(),
                    Some(original_text)
                );
                assert_eq!(
                    crate::canonical::hash_bytes(original_text.as_bytes()),
                    row["record"]["textDigest"].as_str().unwrap()
                );
                reconstructed_aliases += 1;
            }
        }
        let updated_evidence = merge_audited_evidence(&original_evidence, &projected_context);
        let retained_references = evidence_reference_set(&updated_evidence);
        assert_eq!(
            evidence_reference_set(&original_evidence),
            retained_references
        );
        assert_eq!(source_alias_count(projected_pages), reconstructed_aliases);
        assert_eq!(
            citation_role_rows(&original_pages),
            citation_role_rows(projected_pages)
        );
        for (before, after) in original_pages.iter().zip(projected_pages) {
            assert_eq!(before["pageId"], after["pageId"]);
            assert_eq!(before["receiptDigest"], after["receiptDigest"]);
        }
        assert_eq!(
            crate::canonical::bytes(&(original_pages, original_parts)).unwrap(),
            input_content_digest
        );

        let mut updated_packet = packet.clone();
        updated_packet["payload"]["evidence"] = updated_evidence;
        let mut original_packet_metadata = packet.clone();
        original_packet_metadata
            .pointer_mut("/payload")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("evidence");
        let mut updated_packet_metadata = updated_packet.clone();
        updated_packet_metadata
            .pointer_mut("/payload")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("evidence");
        assert_eq!(original_packet_metadata, updated_packet_metadata);
        if let Some(original_callables) = original_evidence.get("callables") {
            assert_eq!(
                updated_packet["payload"]["evidence"]["callables"],
                *original_callables
            );
        }
        let after_size = crate::canonical::bytes(&updated_packet)
            .expect("serialize projected packet canonically")
            .len();
        eprintln!(
            "documentation packet audit: inputFileBytes={} beforeCanonicalBytes={before_size} afterCanonicalBytes={after_size} sourceAliases={reconstructed_aliases} retainedReferences={} citationRoleRows={}",
            input_bytes.len(),
            retained_references.len(),
            citation_role_rows(projected_pages).len()
        );

        if let Some(output_path) = std::env::var_os("CODECLEW_JOB_AUDIT_OUTPUT") {
            let output_path = PathBuf::from(output_path);
            let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .find(|ancestor| ancestor.join(".git").exists())
                .expect("find repository root")
                .canonicalize()
                .expect("canonicalize repository root");
            let input_path = input_path
                .canonicalize()
                .expect("canonicalize input packet path");
            let output_parent = output_path
                .parent()
                .expect("audit output must have a parent directory")
                .canonicalize()
                .expect("canonicalize audit output directory");
            let output_path = output_parent.join(
                output_path
                    .file_name()
                    .expect("audit output must have a file name"),
            );
            let resolved_output = output_path
                .canonicalize()
                .unwrap_or_else(|_| output_path.clone());
            assert!(!resolved_output.starts_with(&repository));
            assert_ne!(resolved_output, input_path);
            std::fs::write(
                output_path,
                crate::canonical::bytes(&updated_packet).expect("serialize private projection"),
            )
            .expect("write private projection to explicit audit output");
        }
    }

    #[test]
    fn callable_navigation_summaries_do_not_enter_delivered_symbol_inventory() {
        let navigation = json!({
            "kind":"CALLABLE_SUMMARY",
            "id":"dep-1",
            "record":{"fullRecordReference":"dependency-handle","symbol":"ledger.Writer.flush"}
        });
        let context = present(&[page("receipt", vec![navigation])], &[]);

        assert_eq!(context["callables"], json!([]));
    }

    #[test]
    fn source_records_parts_and_declaration_tokens_keep_distinct_availability() {
        let callable = symbol_row(
            "dependency-handle",
            &["source-fragment", "source-part", "navigation-only"],
            json!({
                "name":"flush",
                "declarationKind":"METHOD",
                "syntaxKind":"METHOD_DECLARATION",
                "sourceTokens":["public","void","flush"]
            }),
        );
        let source = json!({
            "kind":"SOURCE",
            "id":"source-fragment-id",
            "reference":"source-fragment",
            "record":{
                "revision":"rev-7",
                "file":"Writer.java",
                "startLine":10,
                "endLine":15,
                "text":"private void flush() { SECRET_BODY_TEXT }",
                "textDigest":"text-digest",
                "evidenceDigest":"evidence-digest",
                "authority":"COMPILER"
            }
        });
        let part = json!({
            "reference":"source-part",
            "sourceId":"source-part-id",
            "snapshot":"snapshot-1",
            "recordDigest":"record-digest",
            "startByte":0,
            "endByte":20,
            "totalTextBytes":100,
            "fragmentDigest":"fragment-digest",
            "receiptDigest":"part-receipt",
            "source":{"revision":"rev-9","file":"Writer.java"},
            "text":"SECRET_PART_TEXT"
        });
        let context = present(&[page("receipt", vec![callable, source])], &[part]);
        let inventory = &context["callables"][0];
        let compact = serde_json::to_string(inventory).unwrap();

        assert_eq!(
            inventory["deliveredDeclarationTokens"]["status"],
            "AVAILABLE"
        );
        assert_eq!(inventory["deliveredDeclarationTokens"]["tokenCount"], 3);
        assert_eq!(
            inventory["relatedSourceRecords"][0]["reference"],
            "source-fragment"
        );
        assert_eq!(inventory["relatedSourceRecords"][0]["revision"], "rev-7");
        assert_eq!(
            inventory["relatedSourceParts"][0]["reference"],
            "source-part"
        );
        assert_eq!(
            inventory["relatedSourceParts"][0]["recordDigest"],
            "record-digest"
        );
        assert_eq!(
            inventory["relatedSourceParts"][0]["sourceRevision"],
            "rev-9"
        );
        assert_eq!(inventory["declarationKind"], "METHOD");
        assert_eq!(inventory["syntaxKind"], "METHOD_DECLARATION");
        assert_eq!(
            inventory["navigationOnlySourceReferences"],
            json!(["navigation-only"])
        );
        assert_eq!(inventory["callableBodyCompleteness"], "PARTIAL_OR_UNKNOWN");
        assert_eq!(
            context["callablesMeaning"],
            "Each entry is a captured SYMBOL declaration; declarationKind distinguishes callable and non-callable declarations when available."
        );
        assert!(!compact.contains("SECRET_BODY_TEXT"));
        assert!(!compact.contains("SECRET_PART_TEXT"));
        assert!(!compact.contains("\"public\""));
        assert!(!compact.contains("sourceTokens"));
    }

    #[test]
    fn only_nonempty_compiler_exact_call_and_construct_targets_are_hints() {
        let callable = symbol_row(
            "dependency-handle",
            &[],
            json!({
                "documentation":{"events":[
                    {"kind":"CALL","resolution":"COMPILER_EXACT","target":"svc:Writer.flush()"},
                    {"kind":"CALL","resolution":"COMPILER_EXACT","target":"svc:Writer.flush()"},
                    {"kind":"CONSTRUCT","resolution":"COMPILER_EXACT","target":"svc:Writer()"},
                    {"kind":"CALL","resolution":"LOCAL","target":"svc:Guessed.run()"},
                    {"kind":"CALL","resolution":"UNRESOLVED","target":"svc:Unknown.run()"},
                    {"kind":"BOUNDARY","resolution":"COMPILER_EXACT","target":"svc:Boundary"},
                    {"kind":"CALL","resolution":"COMPILER_EXACT","target":""}
                ]}
            }),
        );
        let context = present(&[page("receipt", vec![callable])], &[]);

        assert_eq!(
            context["callables"][0]["immediateTargetHints"],
            json!([
                {"eventKind":"CALL","targetIdentity":"svc:Writer.flush()"},
                {"eventKind":"CONSTRUCT","targetIdentity":"svc:Writer()"}
            ])
        );
    }
}
