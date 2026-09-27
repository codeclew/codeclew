//! Compact presentation context for documentation author and reviewer jobs.
//!
//! This module projects only the page rows and source parts passed to it. It
//! does not load Work state or resolve navigation handles.

use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};

/// Build the shared evidence envelope used by documentation author and review
/// jobs. Exact duplicate page rows are removed from this presentation only;
/// the page receipts and the inputs passed to this function remain unchanged.
pub(super) fn present(pages: &[Value], source_parts: &[Value]) -> Value {
    let (pages, presentation) = project_pages(pages);
    let callables = callable_inventory(&pages, source_parts);
    let mut context = Map::new();
    context.insert("pages".into(), json!(pages));
    context.insert("sourceParts".into(), json!(source_parts));
    context.insert("callables".into(), json!(callables));
    context.insert(
        "callablesMeaning".into(),
        json!("Each entry is a captured SYMBOL declaration; declarationKind distinguishes callable and non-callable declarations when available."),
    );
    if let Some(presentation) = presentation {
        context.insert("presentation".into(), presentation);
    }
    Value::Object(context)
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
