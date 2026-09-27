//! Compact presentation context for documentation author and reviewer jobs.
//!
//! This module projects only the page rows and source parts passed to it. It
//! does not load Work state or resolve navigation handles.

use super::{analysis, model::Source, process_context};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Build the shared evidence envelope used by documentation author and review
/// jobs. Exact duplicate page rows are removed from this presentation only;
/// the page receipts and the inputs passed to this function remain unchanged.
pub(super) fn present(pages: &[Value], source_parts: &[Value]) -> Value {
    let (mut pages, presentation) = project_pages(pages);
    let callables = callable_inventory(&pages, source_parts);
    let callable_read_actions = callable_read_actions(&pages, source_parts, &callables);
    let baseline = evidence_context(
        &pages,
        source_parts,
        &callables,
        &callable_read_actions,
        presentation.as_ref(),
    );
    let raw_pages = pages.clone();
    let mut summary = ProjectionSummary::default();
    alias_source_texts(&mut pages, &mut summary);
    alias_symbol_tokens_and_events(&raw_pages, &mut pages, &mut summary);
    if summary.is_empty() {
        return baseline;
    }
    mark_projected_pages(&mut pages, &summary);
    let projected_presentation = merge_presentation(presentation, &summary);
    let projected = evidence_context(
        &pages,
        source_parts,
        &callables,
        &callable_read_actions,
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
    callable_read_actions: &[Value],
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
    if !callable_read_actions.is_empty() {
        context.insert("callableReadActions".into(), json!(callable_read_actions));
    }
    if let Some(presentation) = presentation {
        context.insert("presentation".into(), presentation.clone());
    }
    Value::Object(context)
}

#[derive(Default)]
struct ProjectionSummary {
    source_text_alias_count: usize,
    source_token_alias_count: usize,
    documentation_event_alias_count: usize,
    source_aliases_by_page: BTreeMap<usize, PageProjectionCounts>,
}

#[derive(Default)]
struct PageProjectionCounts {
    source_text_aliases: usize,
    source_token_aliases: usize,
    documentation_event_aliases: usize,
}

impl ProjectionSummary {
    fn is_empty(&self) -> bool {
        self.source_text_alias_count == 0
            && self.source_token_alias_count == 0
            && self.documentation_event_alias_count == 0
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FlowKey {
    service: String,
    symbol: String,
    scope: Option<String>,
    ordinal: u64,
}

struct FlowCandidate<'a> {
    page_index: usize,
    item_index: usize,
    reference: String,
    item: &'a Value,
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

fn alias_symbol_tokens_and_events(
    raw_pages: &[Value],
    pages: &mut [Value],
    summary: &mut ProjectionSummary,
) {
    let mut sources = HashMap::<(String, String), Vec<(usize, usize, Source)>>::new();
    let mut flows = BTreeMap::<FlowKey, Vec<FlowCandidate<'_>>>::new();
    for (page_index, page) in raw_pages.iter().enumerate() {
        for (item_index, item) in page_items(std::slice::from_ref(page)).enumerate() {
            if let Some(source) = valid_source_row(item)
                && let (Some(id), Some(reference)) =
                    (item["id"].as_str(), item["reference"].as_str())
            {
                sources
                    .entry((id.to_owned(), reference.to_owned()))
                    .or_default()
                    .push((page_index, item_index, source));
            }
            if item["kind"] != "DEPENDENCY" || item["record"]["kind"] != "FLOW" {
                continue;
            }
            let (Some(service), Some(symbol), Some(ordinal), Some(reference)) = (
                item["record"]["service"].as_str(),
                item["record"]["symbol"].as_str(),
                item["record"]["normalized"]["ordinal"].as_u64(),
                item["reference"].as_str(),
            ) else {
                continue;
            };
            let Some(scope) = optional_scope(&item["record"]["normalized"]) else {
                continue;
            };
            let key = FlowKey {
                service: service.to_owned(),
                symbol: symbol.to_owned(),
                scope,
                ordinal,
            };
            flows.entry(key).or_default().push(FlowCandidate {
                page_index,
                item_index,
                reference: reference.to_owned(),
                item,
            });
        }
    }

    for (page_index, raw_page) in raw_pages.iter().enumerate() {
        let Some(items) = raw_page["items"].as_array() else {
            continue;
        };
        for (item_index, raw_item) in items.iter().enumerate() {
            if raw_item["kind"] != "DEPENDENCY" || raw_item["record"]["kind"] != "SYMBOL" {
                continue;
            }
            let Some(source) = unique_bound_source(raw_item, &sources) else {
                continue;
            };
            let location = json!({
                "pageIndex":source.0,
                "itemIndex":source.1,
                "reference":source.3
            });
            let item = &mut pages[page_index]["items"][item_index];
            let original_size = compact_json_len(item);
            let mut projected_item = item.clone();
            let normalized = &mut projected_item["record"]["normalized"];
            let mut changed_tokens = false;
            if let Some(tokens) = raw_item["record"]["normalized"]["sourceTokens"].as_array()
                && !tokens.is_empty()
                && analysis::java_tokens(&source.2.text)
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>()
                    == *tokens
                && let Some(normalized) = normalized.as_object_mut()
            {
                normalized.remove("sourceTokens");
                normalized.insert(
                    "sourceTokensDisplayAlias".into(),
                    json!({"target":location}),
                );
                changed_tokens = true;
            }

            let mut changed_events = false;
            let raw_events = raw_item["record"]["normalized"]["documentation"]["events"].as_array();
            if let Some(events) = raw_events
                && !events.is_empty()
                && events.iter().all(|event| {
                    event.is_object()
                        && event.get("scope").is_none()
                        && event.get("ordinal").is_none()
                })
                && let Some(scope) = optional_scope(&raw_item["record"]["normalized"])
                && let (Some(service), Some(symbol)) = (
                    raw_item["record"]["service"].as_str(),
                    raw_item["record"]["symbol"].as_str(),
                )
            {
                let mut targets = Vec::with_capacity(events.len());
                let mut all_covered = true;
                for (ordinal, event) in events.iter().enumerate() {
                    let key = FlowKey {
                        service: service.to_owned(),
                        symbol: symbol.to_owned(),
                        scope: scope.clone(),
                        ordinal: ordinal as u64,
                    };
                    let Some([flow]) = flows.get(&key).map(Vec::as_slice) else {
                        all_covered = false;
                        break;
                    };
                    let normalized_flow = &flow.item["record"]["normalized"];
                    if flow_payload(normalized_flow).as_ref() != Some(event) {
                        all_covered = false;
                        break;
                    }
                    let Some(flow_source) = unique_bound_source(flow.item, &sources) else {
                        all_covered = false;
                        break;
                    };
                    if process_context::covered_text(&source.2, &flow_source.2).is_none() {
                        all_covered = false;
                        break;
                    }
                    targets.push(json!({
                        "ordinal":ordinal,
                        "target":{
                            "pageIndex":flow.page_index,
                            "itemIndex":flow.item_index,
                            "reference":flow.reference
                        }
                    }));
                }
                if all_covered
                    && let Some(documentation) = normalized["documentation"].as_object_mut()
                {
                    documentation.remove("events");
                    documentation.insert(
                        "eventsDisplayAlias".into(),
                        json!({"orderedFlowTargets":targets}),
                    );
                    changed_events = true;
                }
            }

            if (changed_tokens || changed_events)
                && compact_json_len(&projected_item) < original_size
            {
                *item = projected_item;
                if changed_tokens {
                    summary.source_token_alias_count += 1;
                    summary.page_counts(page_index).source_token_aliases += 1;
                }
                if changed_events {
                    summary.documentation_event_alias_count += 1;
                    summary.page_counts(page_index).documentation_event_aliases += 1;
                }
            }
        }
    }
}

fn optional_scope(normalized: &Value) -> Option<Option<String>> {
    match normalized.get("scope") {
        None => Some(None),
        Some(Value::String(scope)) if !scope.is_empty() => Some(Some(scope.clone())),
        _ => None,
    }
}

fn unique_bound_source<'r, 's>(
    row: &'r Value,
    sources: &'s HashMap<(String, String), Vec<(usize, usize, Source)>>,
) -> Option<(usize, usize, &'s Source, &'r str)> {
    let ids = row["record"]["sourceIds"].as_array()?;
    let references = row["sourceReferences"].as_array()?;
    if ids.len() != 1 || references.len() != 1 {
        return None;
    }
    let id = ids[0].as_str()?;
    let reference = references[0].as_str()?;
    if id.is_empty() || reference.is_empty() {
        return None;
    }
    let candidates = sources.get(&(id.to_owned(), reference.to_owned()))?;
    let [(page_index, item_index, source)] = candidates.as_slice() else {
        return None;
    };
    if source.service != row["record"]["service"].as_str()? {
        return None;
    }
    Some((*page_index, *item_index, source, reference))
}

fn flow_payload(normalized: &Value) -> Option<Value> {
    let mut payload = normalized.as_object()?.clone();
    payload.remove("ordinal")?;
    if payload.contains_key("scope") {
        payload.remove("scope");
    }
    Some(Value::Object(payload))
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
        if counts.source_token_aliases > 0 {
            projection.insert(
                "sourceTokenAliasCount".into(),
                json!(counts.source_token_aliases),
            );
        }
        if counts.documentation_event_aliases > 0 {
            projection.insert(
                "documentationEventAliasCount".into(),
                json!(counts.documentation_event_aliases),
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
    let mut projection = Map::new();
    projection.insert(
        "kind".into(),
        json!("DISPLAY_ONLY_REPRESENTATION_PROJECTION"),
    );
    projection.insert(
        "authority".into(),
        json!("DISPLAY_PROJECTION_ONLY_ORIGINAL_PAGE_RECEIPTS_UNCHANGED"),
    );
    if summary.source_text_alias_count > 0 {
        projection.insert(
            "sourceTextAliasCount".into(),
            json!(summary.source_text_alias_count),
        );
        projection.insert(
            "sourceTextAliasInstruction".into(),
            json!("For SOURCE rows with displayTextAlias, reconstruct record.text by slicing the delivered target SOURCE record.text at the stated UTF-8 byte offsets. The original SOURCE identity and citation reference remain on the alias row."),
        );
    }
    if summary.source_token_alias_count > 0 {
        projection.insert(
            "sourceTokenAliasCount".into(),
            json!(summary.source_token_alias_count),
        );
        projection.insert(
            "sourceTokenAliasInstruction".into(),
            json!("For SYMBOL rows with sourceTokensDisplayAlias, the original token list was generated from the identified delivered SOURCE text by Codeclew's Java tokenizer; read that SOURCE directly for explanation, and regenerate the list only when exact representation reconstruction is required. The alias is display-only; SYMBOL identity, digest and citation reference remain unchanged."),
        );
    }
    if summary.documentation_event_alias_count > 0 {
        projection.insert(
            "documentationEventAliasCount".into(),
            json!(summary.documentation_event_alias_count),
        );
        projection.insert(
            "documentationEventAliasInstruction".into(),
            json!("For SYMBOL rows with documentation.eventsDisplayAlias, restore events in orderedFlowTargets order by copying each target standalone FLOW normalized payload and removing its capture-added ordinal and scope. Each FLOW row and its citation remain delivered; the alias does not add source authority."),
        );
    }
    presentation.insert("representationProjection".into(), Value::Object(projection));
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

fn callable_read_actions(
    pages: &[Value],
    source_parts: &[Value],
    callables: &[Value],
) -> Vec<Value> {
    let mut rows_by_reference = HashMap::<String, Vec<&Value>>::new();
    let mut source_rows_by_reference = HashMap::<String, Vec<&Value>>::new();
    for item in page_items(pages) {
        if let Some(reference) = item["reference"].as_str() {
            rows_by_reference
                .entry(reference.to_owned())
                .or_default()
                .push(item);
            if item["kind"] == "SOURCE" {
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
        .filter(|item| {
            item["kind"] == "CALLABLE_SUMMARY"
                && item["record"]["authority"] == "NAVIGATION_ONLY"
                && item["record"]["provenance"]["kind"] == "SYMBOL"
        })
        .filter_map(|navigation| {
            let full_reference = navigation["record"]["fullRecordReference"]
                .as_str()
                .filter(|reference| !reference.is_empty())?;
            let (declaration_status, declaration_reason) = declaration_delivery(
                navigation,
                rows_by_reference.get(full_reference).map(Vec::as_slice),
            );
            let source_delivery = source_delivery(
                &navigation["record"]["sourceReferences"],
                &navigation["record"]["service"],
                &source_rows_by_reference,
                &source_parts_by_reference,
            );
            let source_references = string_set(&navigation["record"]["sourceReferences"]);
            let delivered_source_references = string_set(&source_delivery["deliveredReferences"]);
            let ambiguous_source_references =
                string_set(&source_delivery["ambiguousReferences"]);
            let source_only_reads = source_references
                .difference(&delivered_source_references)
                .filter(|reference| !ambiguous_source_references.contains(*reference))
                .map(|reference| {
                    json!({
                        "label":"Read associated source only (within existing Work limits)",
                        "selection":{"references":[reference]}
                    })
                })
                .collect::<Vec<_>>();

            let mut action = json!({
                "fullRecordReference":full_reference,
                "authority":"NAVIGATION_ONLY",
                "declarationDelivery":{
                    "status":declaration_status,
                    "reason":declaration_reason
                },
                "sourceDelivery":source_delivery,
                "sourceOnlyReads":source_only_reads
            });
            if declaration_status == "DELIVERED"
                && callables
                    .iter()
                    .filter(|entry| entry["dependencyReference"].as_str() == Some(full_reference))
                    .count()
                    == 1
            {
                action["callableEntryReference"] = json!(full_reference);
            } else {
                action["primaryRead"] = json!({
                    "label":"Read declaration; service reads also include associated flow/source (within Work limits)",
                    "selection":{"references":[full_reference]}
                });
            }
            Some(action)
        })
        .collect()
}

fn declaration_delivery(
    navigation: &Value,
    rows: Option<&[&Value]>,
) -> (&'static str, &'static str) {
    let Some(rows) = rows else {
        return ("NOT_DELIVERED", "NO_ROW_FOR_REGISTERED_HANDLE");
    };
    let [row] = rows else {
        return ("AMBIGUOUS_OR_MISMATCHED", "CONFLICTING_ROWS_FOR_HANDLE");
    };
    if row["kind"] != "DEPENDENCY" || row["record"]["kind"] != "SYMBOL" {
        return ("AMBIGUOUS_OR_MISMATCHED", "HANDLE_ROW_IS_NOT_A_SYMBOL");
    }
    if !navigation_matches_declaration(navigation, row) {
        return ("AMBIGUOUS_OR_MISMATCHED", "IDENTITY_OR_PROVENANCE_MISMATCH");
    }
    ("DELIVERED", "MATCHING_SYMBOL_ROW_PRESENT")
}

fn navigation_matches_declaration(navigation: &Value, declaration: &Value) -> bool {
    let navigation_record = &navigation["record"];
    let record = &declaration["record"];
    let normalized = &record["normalized"];
    let provenance = &navigation_record["provenance"];

    let identity = normalized["symbolIdentity"]
        .as_str()
        .filter(|identity| !identity.is_empty())
        .or_else(|| {
            normalized["compilerCallableId"]
                .as_str()
                .filter(|identity| !identity.is_empty())
        })
        .or_else(|| record["symbol"].as_str());
    if navigation_record["identity"]
        .as_str()
        .is_some_and(|expected| identity.is_none_or(|actual| expected != actual))
    {
        return false;
    }

    [
        (&navigation_record["symbol"], &record["symbol"]),
        (&navigation_record["service"], &record["service"]),
        (&navigation_record["scope"], &normalized["scope"]),
        (&navigation_record["name"], &normalized["name"]),
        (&navigation_record["owner"], &normalized["ownerIdentity"]),
        (&navigation["id"], &record["id"]),
        (&navigation["id"], &declaration["id"]),
        (&provenance["dependencyId"], &record["id"]),
        (&provenance["dependencyId"], &declaration["id"]),
        (&provenance["observationDigest"], &record["digest"]),
        (&provenance["kind"], &record["kind"]),
    ]
    .into_iter()
    .all(|(expected, actual)| values_consistent(expected, actual))
}

fn values_consistent(expected: &Value, actual: &Value) -> bool {
    expected.is_null() || expected == actual
}

fn source_delivery(
    navigation_references: &Value,
    navigation_service: &Value,
    source_rows_by_reference: &HashMap<String, Vec<&Value>>,
    source_parts_by_reference: &HashMap<String, Vec<&Value>>,
) -> Value {
    let references = string_set(navigation_references);
    let mut delivered = Vec::new();
    let mut partial = Vec::new();
    let mut unread = Vec::new();
    let mut ambiguous = Vec::new();
    for reference in &references {
        let source_rows = source_rows_by_reference.get(reference).map(Vec::as_slice);
        let source_parts = source_parts_by_reference.get(reference).map(Vec::as_slice);
        let source_row = match source_rows {
            Some([row])
                if row["record"]["text"].is_string()
                    && source_record_matches_navigation(row, navigation_service) =>
            {
                Some(*row)
            }
            Some(_) => {
                ambiguous.push(reference.clone());
                continue;
            }
            None => None,
        };
        if !source_parts_match(source_parts.unwrap_or(&[]), source_row, navigation_service) {
            ambiguous.push(reference.clone());
        } else if source_row.is_some() {
            delivered.push(reference.clone());
        } else if source_parts.is_some_and(|parts| !parts.is_empty()) {
            partial.push(reference.clone());
        } else {
            unread.push(reference.clone());
        }
    }
    let status = if references.is_empty() {
        "NONE_RECORDED"
    } else if !ambiguous.is_empty() {
        "AMBIGUOUS_OR_MISMATCHED"
    } else if delivered.len() == references.len() {
        "DELIVERED"
    } else if !delivered.is_empty() || !partial.is_empty() {
        "PARTIAL"
    } else {
        "NOT_DELIVERED"
    };
    json!({
        "status":status,
        "deliveredReferences":delivered,
        "partialReferences":partial,
        "unreadReferences":unread,
        "ambiguousReferences":ambiguous
    })
}

fn source_record_matches_navigation(source: &Value, navigation_service: &Value) -> bool {
    values_consistent(navigation_service, &source["record"]["service"])
        && available_values_consistent(&source["id"], &source["record"]["id"])
        && source["id"].is_string()
        && source["record"]["id"].is_string()
}

fn source_parts_match(
    parts: &[&Value],
    source_row: Option<&Value>,
    navigation_service: &Value,
) -> bool {
    let mut identity_values = HashMap::<&'static str, &Value>::new();
    for part in parts {
        if !values_consistent(navigation_service, &part["source"]["service"])
            || !available_values_consistent(&part["sourceId"], &part["source"]["id"])
            || source_row.is_some_and(|source| {
                !available_values_consistent(&source["id"], &part["sourceId"])
                    || !available_values_consistent(&source["record"]["id"], &part["source"]["id"])
                    || !available_values_consistent(
                        &source["record"]["service"],
                        &part["source"]["service"],
                    )
                    || !available_values_consistent(
                        &source["record"]["revision"],
                        &part["source"]["revision"],
                    )
                    || !available_values_consistent(
                        &source["record"]["file"],
                        &part["source"]["file"],
                    )
                    || !available_values_consistent(
                        &source["record"]["textDigest"],
                        &part["source"]["textDigest"],
                    )
                    || !available_values_consistent(
                        &source["record"]["evidenceDigest"],
                        &part["source"]["evidenceDigest"],
                    )
                    || !available_values_consistent(
                        &source["record"]["authority"],
                        &part["source"]["authority"],
                    )
            })
        {
            return false;
        }
        for (field, value) in [
            ("sourceId", &part["sourceId"]),
            ("snapshot", &part["snapshot"]),
            ("recordDigest", &part["recordDigest"]),
            ("totalTextBytes", &part["totalTextBytes"]),
            ("source.id", &part["source"]["id"]),
            ("source.service", &part["source"]["service"]),
            ("source.revision", &part["source"]["revision"]),
            ("source.file", &part["source"]["file"]),
            ("source.textDigest", &part["source"]["textDigest"]),
            ("source.evidenceDigest", &part["source"]["evidenceDigest"]),
            ("source.authority", &part["source"]["authority"]),
        ] {
            if value.is_null() {
                continue;
            }
            if identity_values
                .get(field)
                .is_some_and(|previous| *previous != value)
            {
                return false;
            }
            identity_values.entry(field).or_insert(value);
        }
    }
    true
}

fn available_values_consistent(left: &Value, right: &Value) -> bool {
    left.is_null() || right.is_null() || left == right
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

    fn callable_summary(full_reference: &str, source_references: &[&str]) -> Value {
        json!({
            "kind":"CALLABLE_SUMMARY",
            "id":"dep-1",
            "referenceRoles":[],
            "record":{
                "identity":"method:class:payments.Writer#flush()V",
                "symbol":"ledger.Writer.flush",
                "service":"payments",
                "scope":"writer",
                "name":"flush",
                "owner":"ledger.Writer",
                "fullRecordReference":full_reference,
                "sourceReferences":source_references,
                "authority":"NAVIGATION_ONLY",
                "provenance":{
                    "dependencyId":"dep-1",
                    "observationDigest":"dependency-digest",
                    "kind":"SYMBOL"
                },
                "bodyAvailability":{
                    "capturedDeclarationTokensAvailable":true,
                    "relatedSourceReferences":source_references,
                    "relatedSourceRecords":"NAVIGATION_ONLY"
                }
            }
        })
    }

    fn matching_symbol_row(reference: &str, source_references: &[&str]) -> Value {
        symbol_row(
            reference,
            source_references,
            json!({
                "symbolIdentity":"method:class:payments.Writer#flush()V",
                "scope":"writer",
                "name":"flush",
                "ownerIdentity":"ledger.Writer",
                "declarationKind":"METHOD",
                "sourceTokens":["private","void","flush"],
                "documentation":{"events":[
                    {"kind":"CALL","resolution":"COMPILER_EXACT","target":"method:class:payments.Writer#target()V"},
                    {"kind":"BOUNDARY","code":"SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"}
                ]}
            }),
        )
    }

    fn symbol_row_with_source(
        reference: &str,
        source_reference: &str,
        source_id: &str,
        normalized: Value,
    ) -> Value {
        let mut row = symbol_row(reference, &[source_reference], normalized);
        row["record"]["sourceIds"] = json!([source_id]);
        row
    }

    fn flow_row(
        reference: &str,
        flow_id: &str,
        service: &str,
        symbol: &str,
        scope: Option<&str>,
        ordinal: u64,
        event: Value,
        source_reference: &str,
        source_id: &str,
    ) -> Value {
        let mut normalized = event.as_object().cloned().unwrap_or_default();
        normalized.insert("ordinal".into(), json!(ordinal));
        if let Some(scope) = scope {
            normalized.insert("scope".into(), json!(scope));
        }
        json!({
            "kind":"DEPENDENCY",
            "id":flow_id,
            "reference":reference,
            "referenceRoles":["evidence"],
            "sourceReferences":[source_reference],
            "record":{
                "id":flow_id,
                "kind":"FLOW",
                "service":service,
                "symbol":symbol,
                "digest":format!("digest-{flow_id}"),
                "sourceIds":[source_id],
                "normalized":normalized
            }
        })
    }

    fn event_fixture(
        event: Value,
        flow_event: Option<Value>,
        flow_scope: Option<&str>,
        flow_ordinal: u64,
    ) -> Vec<Value> {
        let source_text = format!("class Worker {{\n{}\n}}", "  void work() {}\n".repeat(120));
        let symbol_source = source_row(
            "symbol-source-ref",
            "symbol-source-id",
            "payments",
            "rev-1",
            "EXACT_SNAPSHOT_TEXT",
            &source_text,
            1,
            0,
            "shared-blob",
        );
        let flow_source = source_row(
            "flow-source-ref",
            "flow-source-id",
            "payments",
            "rev-1",
            "EXACT_SNAPSHOT_TEXT",
            &source_text,
            1,
            0,
            "shared-blob",
        );
        let symbol = symbol_row_with_source(
            "symbol-ref",
            "symbol-source-ref",
            "symbol-source-id",
            json!({
                "scope":"method-scope",
                "documentation":{"events":[event]}
            }),
        );
        let mut rows = vec![symbol_source, flow_source, symbol];
        if let Some(flow_event) = flow_event {
            rows.push(flow_row(
                "flow-ref-0",
                "flow-id-0",
                "payments",
                "ledger.Writer.flush",
                flow_scope,
                flow_ordinal,
                flow_event,
                "flow-source-ref",
                "flow-source-id",
            ));
        }
        rows
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

    fn source_text_for_location(
        pages: &[Value],
        location: &Value,
        seen: &mut BTreeSet<(usize, usize)>,
    ) -> Option<String> {
        let page_index = location["pageIndex"].as_u64()? as usize;
        let item_index = location["itemIndex"].as_u64()? as usize;
        let reference = location["reference"].as_str()?;
        if !seen.insert((page_index, item_index)) {
            return None;
        }
        let row = pages
            .get(page_index)?
            .get("items")?
            .as_array()?
            .get(item_index)?;
        if row["kind"] != "SOURCE" || row["reference"] != reference {
            return None;
        }
        if let Some(text) = row["record"]["text"].as_str() {
            return Some(text.to_owned());
        }
        let alias = &row["record"]["displayTextAlias"];
        let text = source_text_for_location(pages, &alias["target"], seen)?;
        let start = alias["startByte"].as_u64()? as usize;
        let end = alias["endByte"].as_u64()? as usize;
        String::from_utf8(text.as_bytes().get(start..end)?.to_vec()).ok()
    }

    fn reconstruct_display_item(pages: &[Value], row: &Value) -> Option<Value> {
        let mut restored = row.clone();
        if restored["kind"] == "SOURCE"
            && let Some(alias) = restored["record"].get("displayTextAlias")
        {
            let text = source_text_for_location(pages, &alias["target"], &mut BTreeSet::new())?;
            let start = alias["startByte"].as_u64()? as usize;
            let end = alias["endByte"].as_u64()? as usize;
            let exact = String::from_utf8(text.as_bytes().get(start..end)?.to_vec()).ok()?;
            let record = restored["record"].as_object_mut()?;
            record.remove("displayTextAlias");
            record.insert("text".into(), json!(exact));
        }
        if restored["kind"] == "DEPENDENCY" && restored["record"]["kind"] == "SYMBOL" {
            let normalized = restored["record"]["normalized"].as_object_mut()?;
            if let Some(alias) = normalized.get("sourceTokensDisplayAlias").cloned() {
                let text = source_text_for_location(pages, &alias["target"], &mut BTreeSet::new())?;
                let tokens = analysis::java_tokens(&text)
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>();
                normalized.remove("sourceTokensDisplayAlias");
                normalized.insert("sourceTokens".into(), json!(tokens));
            }
            if let Some(alias) = normalized
                .get("documentation")
                .and_then(|documentation| documentation.get("eventsDisplayAlias"))
                .cloned()
            {
                let mut events = Vec::new();
                let mut targets = alias["orderedFlowTargets"].as_array()?.clone();
                targets.sort_by_key(|target| target["ordinal"].as_u64().unwrap_or(u64::MAX));
                for target in targets {
                    let location = &target["target"];
                    let page_index = location["pageIndex"].as_u64()? as usize;
                    let item_index = location["itemIndex"].as_u64()? as usize;
                    let reference = location["reference"].as_str()?;
                    let flow = pages
                        .get(page_index)?
                        .get("items")?
                        .as_array()?
                        .get(item_index)?;
                    if flow["kind"] != "DEPENDENCY"
                        || flow["record"]["kind"] != "FLOW"
                        || flow["reference"] != reference
                    {
                        return None;
                    }
                    let mut event = flow["record"]["normalized"].as_object()?.clone();
                    event.remove("ordinal")?;
                    event.remove("scope");
                    events.push(Value::Object(event));
                }
                let documentation = normalized.get_mut("documentation")?.as_object_mut()?;
                documentation.remove("eventsDisplayAlias");
                documentation.insert("events".into(), json!(events));
            }
        }
        Some(restored)
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
        for key in [
            "pages",
            "sourceParts",
            "callables",
            "callablesMeaning",
            "callableReadActions",
        ] {
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

    fn symbol_alias_counts(pages: &[Value]) -> (usize, usize) {
        let mut tokens = 0;
        let mut events = 0;
        for row in pages
            .iter()
            .flat_map(|page| page["items"].as_array().into_iter().flatten())
        {
            if row["kind"] == "DEPENDENCY" && row["record"]["kind"] == "SYMBOL" {
                if row["record"]["normalized"]
                    .get("sourceTokensDisplayAlias")
                    .is_some()
                {
                    tokens += 1;
                }
                if row["record"]["normalized"]["documentation"]
                    .get("eventsDisplayAlias")
                    .is_some()
                {
                    events += 1;
                }
            }
        }
        (tokens, events)
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
    fn exact_source_tokens_are_represented_by_a_reconstructible_source_link() {
        let text = format!(
            "public class Worker {{\n{}\n}}",
            "  int count = 1; count++;\n".repeat(80)
        );
        let source = source_row(
            "token-source-ref",
            "token-source-id",
            "payments",
            "rev-1",
            "EXACT_SNAPSHOT_TEXT",
            &text,
            1,
            0,
            "token-blob",
        );
        let tokens = analysis::java_tokens(&text)
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        let symbol = symbol_row_with_source(
            "symbol-ref",
            "token-source-ref",
            "token-source-id",
            json!({
                "scope":"method-scope",
                "sourceTokens":tokens,
                "documentation":{"events":[],"boundaries":["ORDER_LEXICAL_ONLY"]}
            }),
        );
        let raw_pages = vec![page("receipt", vec![source, symbol])];
        let raw_parts = Vec::new();
        let original = raw_pages.clone();

        let context = present(&raw_pages, &raw_parts);
        let projected_pages = context["pages"].as_array().unwrap();
        let projected_symbol = projected_pages[0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["record"]["kind"] == "SYMBOL")
            .unwrap();
        assert!(projected_symbol["record"]["normalized"]["sourceTokens"].is_null());
        assert!(
            projected_symbol["record"]["normalized"]["sourceTokensDisplayAlias"]["target"]["reference"]
                == "token-source-ref"
        );
        assert_eq!(symbol_alias_counts(projected_pages).0, 1);
        for (raw, shown) in raw_pages[0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .zip(projected_pages[0]["items"].as_array().unwrap())
        {
            assert!(
                reconstruct_display_item(projected_pages, shown).as_ref() == Some(raw),
                "token projection restores the whole original row"
            );
        }
        assert!(
            raw_pages == original,
            "presentation does not mutate caller pages"
        );
        assert!(
            present(&raw_pages, &raw_parts) == context,
            "presentation is repeatable"
        );
    }

    #[test]
    fn source_parts_and_mismatched_tokens_do_not_justify_token_aliases() {
        let text = "public void work() { return; }\n".repeat(80);
        let text = text.trim_end().to_owned();
        let source_parts = vec![json!({
            "reference":"source-ref",
            "sourceId":"source-id",
            "source":{"revision":"rev-1","file":"Worker.java"},
            "startByte":0,
            "endByte":text.len(),
            "fragmentDigest":"fragment-digest"
        })];
        let tokens = analysis::java_tokens(&text)
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        let exact_symbol = symbol_row_with_source(
            "symbol-ref",
            "source-ref",
            "source-id",
            json!({"sourceTokens":tokens.clone()}),
        );
        let incomplete = present(
            &[page("parts-only", vec![exact_symbol.clone()])],
            &source_parts,
        );
        let incomplete_symbol = &incomplete["pages"][0]["items"][0];
        assert!(incomplete_symbol["record"]["normalized"]["sourceTokensDisplayAlias"].is_null());
        assert!(incomplete_symbol["record"]["normalized"]["sourceTokens"].is_array());

        let source = source_row(
            "source-ref",
            "source-id",
            "payments",
            "rev-1",
            "EXACT_SNAPSHOT_TEXT",
            &text,
            1,
            0,
            "token-mismatch-blob",
        );
        let mut mismatched_tokens = tokens;
        mismatched_tokens.pop();
        let mismatched_symbol = symbol_row_with_source(
            "symbol-ref",
            "source-ref",
            "source-id",
            json!({"sourceTokens":mismatched_tokens}),
        );
        let mismatched = present(&[page("mismatch", vec![source, mismatched_symbol])], &[]);
        let mismatched_symbol = mismatched["pages"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["record"]["kind"] == "SYMBOL")
            .unwrap();
        assert!(mismatched_symbol["record"]["normalized"]["sourceTokensDisplayAlias"].is_null());
        assert!(mismatched_symbol["record"]["normalized"]["sourceTokens"].is_array());
    }

    #[test]
    fn covered_flow_events_alias_exactly_and_leave_flow_rows_and_citations_intact() {
        let event = json!({
            "kind":"CALL",
            "resolution":"COMPILER_EXACT",
            "target":"method:ledger.Writer.write()V",
            "condition":"ready ".repeat(140)
        });
        let rows = event_fixture(event.clone(), Some(event.clone()), Some("method-scope"), 0);
        let raw_pages = vec![page("receipt", rows.clone())];
        let context = present(&raw_pages, &[]);
        let projected_pages = context["pages"].as_array().unwrap();
        let projected_symbol = projected_pages[0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["record"]["kind"] == "SYMBOL")
            .unwrap();
        assert!(projected_symbol["record"]["normalized"]["documentation"]["events"].is_null());
        assert_eq!(symbol_alias_counts(projected_pages).1, 1);
        for (raw, shown) in rows
            .iter()
            .zip(projected_pages[0]["items"].as_array().unwrap())
        {
            assert!(
                reconstruct_display_item(projected_pages, shown).as_ref() == Some(raw),
                "event projection restores the whole original row"
            );
        }
        let original_flow = rows
            .iter()
            .find(|row| row["record"]["kind"] == "FLOW")
            .unwrap();
        let projected_flow = projected_pages[0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["record"]["kind"] == "FLOW")
            .unwrap();
        assert!(
            projected_flow == original_flow,
            "standalone FLOW remains unchanged"
        );
        assert!(
            citation_role_rows(&[page("receipt", rows)]) == citation_role_rows(projected_pages),
            "citation identities and roles remain unchanged"
        );
    }

    #[test]
    fn incomplete_or_ambiguous_flow_coverage_keeps_nested_events() {
        let event = json!({
            "kind":"CALL",
            "resolution":"COMPILER_EXACT",
            "target":"method:ledger.Writer.write()V",
            "condition":"ready ".repeat(140)
        });
        let mut variants = vec![event_fixture(event.clone(), None, None, 0)];
        variants.push(event_fixture(
            event.clone(),
            Some(event.clone()),
            Some("different-scope"),
            0,
        ));
        variants.push(event_fixture(
            event.clone(),
            Some(event.clone()),
            Some("method-scope"),
            1,
        ));
        let mut wrong_payload = event.clone();
        wrong_payload["condition"] = json!("different");
        variants.push(event_fixture(
            event.clone(),
            Some(wrong_payload),
            Some("method-scope"),
            0,
        ));
        let mut ambiguous =
            event_fixture(event.clone(), Some(event.clone()), Some("method-scope"), 0);
        let mut duplicate_flow = ambiguous.last().unwrap().clone();
        duplicate_flow["id"] = json!("flow-id-duplicate");
        duplicate_flow["reference"] = json!("flow-ref-duplicate");
        duplicate_flow["record"]["id"] = json!("flow-id-duplicate");
        ambiguous.push(duplicate_flow);
        variants.push(ambiguous);
        let mut wrong_occurrence =
            event_fixture(event.clone(), Some(event.clone()), Some("method-scope"), 0);
        wrong_occurrence[1]["record"]["occurrence"]["blob"] = json!("different-blob");
        variants.push(wrong_occurrence);
        let mut wrong_line =
            event_fixture(event.clone(), Some(event.clone()), Some("method-scope"), 0);
        wrong_line[1]["record"]["startLine"] = json!(2);
        wrong_line[1]["record"]["endLine"] = json!(2);
        variants.push(wrong_line);
        let mut nested_ordinal = event.clone();
        nested_ordinal["ordinal"] = json!(0);
        variants.push(event_fixture(
            nested_ordinal,
            Some(event.clone()),
            Some("method-scope"),
            0,
        ));
        let mut nested_scope = event.clone();
        nested_scope["scope"] = json!("original-event-scope");
        variants.push(event_fixture(
            nested_scope,
            Some(event.clone()),
            Some("method-scope"),
            0,
        ));

        for (index, rows) in variants.into_iter().enumerate() {
            let context = present(&[page("receipt", rows.clone())], &[]);
            let symbol = context["pages"][0]["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["record"]["kind"] == "SYMBOL")
                .unwrap();
            assert!(
                symbol["record"]["normalized"]["documentation"]["eventsDisplayAlias"].is_null(),
                "unsafe event coverage variant {index} remains inline"
            );
            assert!(
                symbol["record"]["normalized"]["documentation"]["events"].as_array()
                    == Some(&vec![if index == 7 {
                        let mut value = event.clone();
                        value["ordinal"] = json!(0);
                        value
                    } else if index == 8 {
                        let mut value = event.clone();
                        value["scope"] = json!("original-event-scope");
                        value
                    } else {
                        event.clone()
                    }]),
                "unsafe coverage keeps its full original event payload"
            );
        }
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
        assert!(
            present(&original_pages, &original_parts) == projected_context,
            "presentation must be deterministic for the same immutable input"
        );
        let projected_pages = projected_context["pages"]
            .as_array()
            .expect("presentation returns pages");
        let mut reconstructed_aliases = 0usize;
        let mut reconstructed_rows = 0usize;
        let (token_aliases, event_aliases) = symbol_alias_counts(projected_pages);
        for (page_index, page) in projected_pages.iter().enumerate() {
            for row in page["items"].as_array().into_iter().flatten() {
                let original = original_pages[page_index]["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|original| {
                        original["kind"] == row["kind"]
                            && original["id"] == row["id"]
                            && original["reference"] == row["reference"]
                            && original["record"]["digest"] == row["record"]["digest"]
                            && original["record"]["textDigest"] == row["record"]["textDigest"]
                    })
                    .expect("projected row identity exists in original packet");
                let restored = reconstruct_display_item(projected_pages, row)
                    .expect("display projection is reconstructible");
                assert!(restored == *original, "projected row restores exactly");
                if row["kind"] == "SOURCE" && row["record"].get("displayTextAlias").is_some() {
                    let original_text = original["record"]["text"]
                        .as_str()
                        .expect("original source has exact text");
                    assert!(
                        crate::canonical::hash_bytes(original_text.as_bytes())
                            == row["record"]["textDigest"].as_str().unwrap_or_default(),
                        "source alias retains the original digest"
                    );
                    reconstructed_aliases += 1;
                }
                reconstructed_rows += 1;
            }
        }
        let original_row_count = original_pages
            .iter()
            .map(|page| page["items"].as_array().map_or(0, Vec::len))
            .sum::<usize>();
        assert!(
            reconstructed_rows == original_row_count,
            "all page rows restored"
        );
        assert!(
            source_alias_count(projected_pages) == reconstructed_aliases,
            "all source aliases were reconstructed"
        );
        let updated_evidence = merge_audited_evidence(&original_evidence, &projected_context);
        let retained_references = evidence_reference_set(&updated_evidence);
        assert!(
            evidence_reference_set(&original_evidence) == retained_references,
            "projected evidence preserves its full reference set"
        );
        assert!(
            citation_role_rows(&original_pages) == citation_role_rows(projected_pages),
            "projected evidence preserves citation-role rows"
        );
        for (before, after) in original_pages.iter().zip(projected_pages) {
            assert!(
                before["pageId"] == after["pageId"],
                "page identity is unchanged"
            );
            assert!(
                before["receiptDigest"] == after["receiptDigest"],
                "page receipt is unchanged"
            );
        }
        assert!(
            crate::canonical::bytes(&(original_pages.clone(), original_parts.clone())).unwrap()
                == input_content_digest,
            "caller evidence remains immutable"
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
        assert!(
            original_packet_metadata == updated_packet_metadata,
            "packet metadata outside evidence is unchanged"
        );
        if let Some(original_callables) = original_evidence.get("callables") {
            assert!(
                updated_packet["payload"]["evidence"]["callables"] == *original_callables,
                "callable inventory remains unchanged"
            );
        }
        assert!(
            updated_packet["payload"]["evidence"]["sourceParts"]
                == original_evidence["sourceParts"],
            "source parts remain unchanged"
        );
        let after_size = crate::canonical::bytes(&updated_packet)
            .expect("serialize projected packet canonically")
            .len();
        eprintln!(
            "documentation packet audit: inputFileBytes={} beforeCanonicalBytes={before_size} afterCanonicalBytes={after_size} sourceAliases={reconstructed_aliases} tokenAliases={token_aliases} eventAliases={event_aliases} restoredRows={reconstructed_rows} retainedReferences={} citationRoleRows={}",
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
    fn callable_navigation_card_tracks_source_and_declaration_separately() {
        use super::super::work::Selection;

        let navigation = callable_summary("dependency-handle", &["source-fragment"]);
        let navigation_pages = vec![page("navigation", vec![navigation.clone()])];
        let original_navigation_pages = navigation_pages.clone();
        let navigation_context = present(&navigation_pages, &[]);
        let navigation_card = &navigation_context["callableReadActions"][0];
        assert_eq!(navigation_context["callables"], json!([]));
        assert_eq!(navigation_card["authority"], "NAVIGATION_ONLY");
        assert_eq!(
            navigation_card["declarationDelivery"]["status"],
            "NOT_DELIVERED"
        );
        assert_eq!(navigation_card["sourceDelivery"]["status"], "NOT_DELIVERED");
        let full_read = &navigation_card["primaryRead"]["selection"];
        assert_eq!(full_read, &json!({"references":["dependency-handle"]}));
        let parsed_full_read: Selection = serde_json::from_value(full_read.clone()).unwrap();
        assert_eq!(parsed_full_read.references, ["dependency-handle"]);
        let source_read = &navigation_card["sourceOnlyReads"][0]["selection"];
        let parsed_source_read: Selection = serde_json::from_value(source_read.clone()).unwrap();
        assert_eq!(parsed_source_read.references, ["source-fragment"]);
        assert_eq!(navigation_pages, original_navigation_pages);

        let source = json!({
            "kind":"SOURCE",
            "id":"source-fragment-id",
            "reference":"source-fragment",
            "record":{
                "id":"source-fragment-id",
                "service":"payments",
                "revision":"rev-7",
                "file":"Writer.java",
                "text":"private void flush() { SECRET_BODY_TEXT }"
            }
        });
        let source_pages = vec![page("source", vec![navigation.clone(), source.clone()])];
        let original_source_pages = source_pages.clone();
        let source_context = present(&source_pages, &[]);
        let source_card = &source_context["callableReadActions"][0];
        assert_eq!(
            source_card["declarationDelivery"]["status"],
            "NOT_DELIVERED"
        );
        assert_eq!(source_card["sourceDelivery"]["status"], "DELIVERED");
        assert_eq!(
            source_card["primaryRead"]["selection"],
            json!({"references":["dependency-handle"]})
        );
        assert_eq!(source_pages, original_source_pages);

        let declaration = matching_symbol_row("dependency-handle", &["source-fragment"]);
        let full_pages = vec![page("full", vec![navigation, source, declaration])];
        let original_full_pages = full_pages.clone();
        let full_context = present(&full_pages, &[]);
        let full_card = &full_context["callableReadActions"][0];
        assert_eq!(full_card["declarationDelivery"]["status"], "DELIVERED");
        assert!(full_card.get("primaryRead").is_none());
        assert_eq!(full_card["callableEntryReference"], "dependency-handle");
        assert_eq!(
            full_context["callables"][0]["immediateTargetHints"],
            json!([{"eventKind":"CALL","targetIdentity":"method:class:payments.Writer#target()V"}])
        );
        assert_eq!(
            full_context["pages"][0]["items"][2]["record"]["normalized"]["documentation"]["events"]
                [1]["code"],
            "SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"
        );
        let compact_card = serde_json::to_string(full_card).unwrap();
        for forbidden in [
            "SECRET_BODY_TEXT",
            "sourceTokens",
            "immediateTargetHints",
            "SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW",
        ] {
            assert!(!compact_card.contains(forbidden));
        }
        assert_eq!(full_pages, original_full_pages);
        assert_eq!(
            full_context["callablesMeaning"],
            "Each entry is a captured SYMBOL declaration; declarationKind distinguishes callable and non-callable declarations when available."
        );
    }

    #[test]
    fn callable_navigation_card_keeps_partial_and_mismatched_evidence_unconfirmed() {
        let navigation = callable_summary("dependency-handle", &["source-fragment"]);
        let part = json!({
            "reference":"source-fragment",
            "sourceId":"source-fragment-id",
            "snapshot":"snapshot-1",
            "recordDigest":"record-digest",
            "startByte":0,
            "endByte":20,
            "totalTextBytes":100,
            "source":{"id":"source-fragment-id","service":"payments","revision":"rev-7","file":"Writer.java","textDigest":"source-text-digest","evidenceDigest":"source-evidence-digest","authority":"COMPILER"},
            "text":"private void flush"
        });
        let mut second_part = part.clone();
        second_part["startByte"] = json!(20);
        second_part["endByte"] = json!(40);
        second_part["text"] = json!("() { body }");
        let partial_context = present(
            &[page("partial", vec![navigation.clone()])],
            &[part.clone(), second_part.clone()],
        );
        let partial_card = &partial_context["callableReadActions"][0];
        assert_eq!(
            partial_card["declarationDelivery"]["status"],
            "NOT_DELIVERED"
        );
        assert_eq!(partial_card["sourceDelivery"]["status"], "PARTIAL");
        assert_eq!(
            partial_card["sourceDelivery"]["partialReferences"],
            json!(["source-fragment"])
        );

        let mut conflicting_part = part.clone();
        conflicting_part["sourceId"] = json!("other-source-id");
        conflicting_part["source"]["id"] = json!("other-source-id");
        let conflicting_parts_context = present(
            &[page("partial-conflict", vec![navigation.clone()])],
            &[part.clone(), conflicting_part],
        );
        assert_eq!(
            conflicting_parts_context["callableReadActions"][0]["sourceDelivery"]["status"],
            "AMBIGUOUS_OR_MISMATCHED"
        );

        let source = json!({
            "kind":"SOURCE",
            "id":"source-fragment-id",
            "reference":"source-fragment",
            "record":{
                "id":"source-fragment-id",
                "service":"payments",
                "revision":"rev-7",
                "file":"Writer.java",
                "textDigest":"source-text-digest",
                "evidenceDigest":"source-evidence-digest",
                "authority":"COMPILER",
                "text":"private void flush() { body }"
            }
        });
        let matching_source_and_part = present(
            &[page(
                "source-and-part",
                vec![navigation.clone(), source.clone()],
            )],
            &[part.clone()],
        );
        assert_eq!(
            matching_source_and_part["callableReadActions"][0]["sourceDelivery"]["status"],
            "DELIVERED"
        );
        let mut source_identity_conflict = part.clone();
        source_identity_conflict["sourceId"] = json!("other-source-id");
        source_identity_conflict["source"]["id"] = json!("other-source-id");
        let source_part_conflict = present(
            &[page(
                "source-part-conflict",
                vec![navigation.clone(), source.clone()],
            )],
            &[source_identity_conflict],
        );
        assert_eq!(
            source_part_conflict["callableReadActions"][0]["sourceDelivery"]["status"],
            "AMBIGUOUS_OR_MISMATCHED"
        );
        let mut source_provenance_conflict = part.clone();
        source_provenance_conflict["source"]["textDigest"] = json!("different-text-digest");
        let source_digest_conflict = present(
            &[page(
                "source-digest-conflict",
                vec![navigation.clone(), source.clone()],
            )],
            &[source_provenance_conflict],
        );
        assert_eq!(
            source_digest_conflict["callableReadActions"][0]["sourceDelivery"]["status"],
            "AMBIGUOUS_OR_MISMATCHED"
        );
        let mut wrong_service = source;
        wrong_service["record"]["service"] = json!("other-service");
        let source_service_conflict = present(
            &[page(
                "source-service-conflict",
                vec![navigation.clone(), wrong_service],
            )],
            &[],
        );
        assert_eq!(
            source_service_conflict["callableReadActions"][0]["sourceDelivery"]["status"],
            "AMBIGUOUS_OR_MISMATCHED"
        );

        let matching_declaration = matching_symbol_row("dependency-handle", &["source-fragment"]);
        let matching_context = present(
            &[page(
                "matching",
                vec![navigation.clone(), matching_declaration.clone()],
            )],
            &[],
        );
        assert_eq!(
            matching_context["callableReadActions"][0]["declarationDelivery"]["status"],
            "DELIVERED"
        );

        let mut mismatched_declaration = matching_declaration.clone();
        mismatched_declaration["record"]["digest"] = json!("different-digest");
        let mismatch_context = present(
            &[page(
                "digest-mismatch",
                vec![navigation.clone(), mismatched_declaration],
            )],
            &[],
        );
        assert_eq!(
            mismatch_context["callableReadActions"][0]["declarationDelivery"]["status"],
            "AMBIGUOUS_OR_MISMATCHED"
        );

        let mut missing_provenance = matching_declaration.clone();
        missing_provenance["record"]
            .as_object_mut()
            .unwrap()
            .remove("digest");
        let missing_context = present(
            &[page(
                "missing-digest",
                vec![navigation.clone(), missing_provenance],
            )],
            &[],
        );
        assert_eq!(
            missing_context["callableReadActions"][0]["declarationDelivery"]["status"],
            "AMBIGUOUS_OR_MISMATCHED"
        );

        let first = matching_declaration;
        let mut second = first.clone();
        second["record"]["digest"] = json!("other-digest");
        let conflicting_rows = present(
            &[page(
                "same-handle-conflict",
                vec![navigation.clone(), first, second],
            )],
            &[],
        );
        assert_eq!(
            conflicting_rows["callableReadActions"][0]["declarationDelivery"]["status"],
            "AMBIGUOUS_OR_MISMATCHED"
        );

        let mut non_symbol_navigation = navigation;
        non_symbol_navigation["record"]["provenance"]["kind"] = json!("FLOW");
        let non_symbol_context =
            present(&[page("flow-navigation", vec![non_symbol_navigation])], &[]);
        assert!(non_symbol_context.get("callableReadActions").is_none());
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
