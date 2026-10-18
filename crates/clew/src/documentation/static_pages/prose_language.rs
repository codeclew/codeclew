//! Language availability is separate from evidence and review status.
use crate::documentation::{language, reader::text, render::escape, store};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn gap(
    available: &Value,
    requested: Option<&str>,
    kind: &str,
    original_path: &str,
) -> Result<Option<Value>, ClewError> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    language::validate(Some(requested))?;
    language::validate(available.as_str())?;
    store::relative(original_path)?;
    if available.as_str() == Some(requested) {
        return Ok(None);
    }
    Ok(Some(json!({"kind":kind,"requestedLanguage":requested,
        "availableLanguage":available,"originalPath":original_path,
        "authority":"ORIGINAL_AUTHORED_DESCRIPTION_UNCHANGED"})))
}

pub(super) fn html(gap: &Value, href: &str) -> String {
    let language = gap["requestedLanguage"].as_str().unwrap_or("en");
    format!(
        "<aside class=\"translation-gap gap-card\"><h2>{}: {}</h2><p>{}</p><p>{}: {}</p><p><a href=\"{}\">{}</a></p></aside>",
        availability_heading(gap, language),
        description_label(gap, language),
        availability_explanation(gap, language),
        text(
            language,
            "Original prose language",
            "Язык исходного описания"
        ),
        super::question_reading::available_language_label(gap, language),
        escape(href),
        text(
            language,
            "Original description (JSON)",
            "Исходное описание (JSON)"
        )
    )
}

pub(super) fn availability_heading(gap: &Value, language: &str) -> &'static str {
    if gap["availableLanguage"].is_null() {
        text(
            language,
            "Availability in the selected language is not established",
            "Доступность на выбранном языке не установлена",
        )
    } else {
        text(language, "Translation pending", "Требуется перевод")
    }
}

pub(super) fn availability_explanation(gap: &Value, language: &str) -> &'static str {
    if gap["availableLanguage"].is_null() {
        text(
            language,
            "The original description has no declared language. Its readable original and evidence are retained; no translation or review is inferred.",
            "Язык исходного описания не указан. Исходный текст и основания сохранены; перевод и новая проверка не подразумеваются.",
        )
    } else {
        explanation(language)
    }
}

pub(super) fn description_label(gap: &Value, language: &str) -> &'static str {
    match gap["kind"].as_str() {
        Some("SELECTED_QUESTION_TRANSLATION") => {
            text(language, "Selected question", "Выбранный вопрос")
        }
        Some("SELECTED_PARAGRAPH_TRANSLATION") => text(
            language,
            "Maintained user documentation",
            "Поддерживаемая пользовательская документация",
        ),
        Some("SELECTED_NOTE_TRANSLATION" | "NOTE_ORIGINAL_TRANSLATION") => text(
            language,
            "Human or imported instruction",
            "Пользовательская или импортированная инструкция",
        ),
        Some("MODEL_TRANSLATION") => text(language, "Model description", "Описание модели"),
        Some("PROCESS_DATA_FLOW_TRANSLATION") => {
            text(language, "Data flow description", "Описание потока данных")
        }
        Some("DECLARED_CONTRACT_TRANSLATION") => text(
            language,
            "Declared contract description",
            "Описание заявленного контракта",
        ),
        Some("HTTP_COMPARISON_TRANSLATION") => text(
            language,
            "HTTP comparison description",
            "Описание сравнения HTTP",
        ),
        _ => text(language, "Description", "Описание"),
    }
}

pub(super) fn explanation(language: &str) -> &'static str {
    text(
        language,
        "This description is not available in the selected language. Its readable original remains on this page, with the same anchors and evidence. This is not a translation or a new review.",
        "Описание недоступно на выбранном языке. На этой странице доступно исходное описание с прежними ссылками, якорями и основаниями. Это не перевод и не новая проверка.",
    )
}

pub(super) fn original_path(id: &str) -> Result<String, ClewError> {
    // IDs are already admitted by the model contract; retain a local boundary here.
    if !store::valid_id(id) {
        return Err(crate::documentation::invalid(
            "model original description identifier is unsafe",
        ));
    }
    Ok(format!("data-model/{id}.original.json"))
}

/// Retain records before attaching availability metadata to a derived page.
/// Neither the frozen publication nor the original records are rewritten.
pub(super) fn attach_payload(
    payload: &mut Value,
    name: &str,
    requested: Option<&str>,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<Vec<Value>, ClewError> {
    store::relative(name)?;
    let base = name
        .strip_suffix(".json")
        .ok_or_else(|| crate::documentation::invalid("authored page record path must be JSON"))?;
    let mut gaps = Vec::new();
    for (key, suffix, kind) in [
        (
            "processDataFlow",
            "data-flow",
            "PROCESS_DATA_FLOW_TRANSLATION",
        ),
        (
            "contractComparison",
            "comparison",
            "HTTP_COMPARISON_TRANSLATION",
        ),
    ] {
        let record = &payload[key];
        if !record.is_object() {
            continue;
        }
        let original_path = format!("{base}.{suffix}.original.json");
        let available = if key == "processDataFlow" {
            &record["request"]["documentationLanguage"]
        } else {
            &record["documentationLanguage"]
        };
        files.insert(
            original_path.clone(),
            serde_json::to_vec_pretty(record).map_err(crate::documentation::io_error)?,
        );
        if let Some(mut gap) = gap(available, requested, kind, &original_path)? {
            gap["record"] = json!(key);
            gaps.push(gap);
        }
    }
    payload["authoredTranslationGaps"] = json!(gaps);
    Ok(gaps)
}

pub(super) fn page_notices(gaps: &[Value]) -> String {
    gaps.iter()
        .map(|gap| {
            let href = gap["originalPath"]
                .as_str()
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("");
            html(gap, href)
        })
        .collect()
}

/// Recompute presentation availability without changing accepted operations.
/// The frozen publication's language request is retained as provenance.
pub(super) fn attach_narrative(
    payload: &mut Value,
    name: &str,
    requested: Option<&str>,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Value>, ClewError> {
    store::relative(name)?;
    language::validate(requested)?;
    let base = name
        .strip_suffix(".json")
        .ok_or_else(|| crate::documentation::invalid("narrative page path must be JSON"))?;
    let mut operations = BTreeMap::new();
    for operation in payload["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            payload["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|section| &section["content"]),
        )
        .chain(
            payload["notes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|note| &note["assessment"]),
        )
    {
        let Some(id) = operation["id"].as_str() else {
            continue;
        };
        if !store::valid_id(id) {
            return Err(crate::documentation::invalid(
                "narrative original identifier is unsafe",
            ));
        }
        operations
            .entry(id.to_owned())
            .or_insert_with(|| operation.clone());
    }
    let mut gaps = BTreeMap::new();
    for (id, operation) in operations {
        let original_path = format!("{base}.operation-{id}.original.json");
        let original = json!({"operation":operation,"sources":payload["operationSources"][&id],
            "state":payload["operationStates"][&id],"sectionState":payload["sectionState"].get(&id).unwrap_or(&payload["sectionState"]),"contracts":payload["operationContracts"][&id],
            "externalCalls":payload["operationExternalCalls"][&id]});
        files.insert(
            original_path.clone(),
            serde_json::to_vec_pretty(&original).map_err(crate::documentation::io_error)?,
        );
        if let Some(mut gap) = gap(
            &operation["documentationLanguage"],
            requested,
            "RETAINED_OPERATION_TRANSLATION",
            &original_path,
        )? {
            gap["operationId"] = json!(id);
            gap["title"] = operation["title"].clone();
            gaps.insert(id, gap);
        }
    }
    for note in payload["notes"].as_array().into_iter().flatten() {
        if note["original"]["text"].as_str().is_none() {
            continue;
        }
        let id = note["association"]["id"]
            .as_str()
            .filter(|id| store::valid_id(id))
            .ok_or_else(|| crate::documentation::invalid("original note identifier is unsafe"))?;
        let original_path = format!("{base}.note-{id}.original.json");
        let original = json!({"association":note["association"],"original":note["original"],
            "associationDigest":note["associationDigest"],"authority":note["authority"],"snapshotLabel":note["snapshotLabel"]});
        files.insert(
            original_path.clone(),
            serde_json::to_vec_pretty(&original).map_err(crate::documentation::io_error)?,
        );
        if let Some(mut gap) = gap(
            &note["association"]["documentationLanguage"],
            requested,
            "NOTE_ORIGINAL_TRANSLATION",
            &original_path,
        )? {
            gap["noteId"] = json!(id);
            gap["title"] = note["association"]["title"].clone();
            gaps.insert(format!("original-note-{id}"), gap);
        }
    }
    if !payload["frozenLanguageAvailability"].is_object() {
        payload["frozenLanguageAvailability"] = json!({
            "requestedDocumentationLanguage":payload["requestedDocumentationLanguage"],
            "translationComplete":payload["translationComplete"],"translationGaps":payload["translationGaps"]});
    }
    payload["requestedDocumentationLanguage"] = json!(requested);
    payload["translationComplete"] = json!(requested.map(|_| gaps.is_empty()));
    payload["translationGaps"] = json!(gaps);
    payload["proseDisplayPolicy"] = json!("READABLE_ORIGINALS");
    // Only generated section labels and future request templates use the target
    // locale; accepted section content and authored gap reasons stay unchanged.
    if let Some(target) = requested {
        let original_gaps: BTreeMap<String, Value> = payload["sections"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|section| {
                section["id"]
                    .as_str()
                    .map(|id| (id.to_owned(), section["gap"].clone()))
            })
            .collect();
        crate::documentation::language::section_labels(payload, target);
        for section in payload["sections"].as_array_mut().into_iter().flatten() {
            if target == "en"
                && let Some((_, title, purpose)) = crate::documentation::sections::REQUIRED
                    .iter()
                    .find(|(id, _, _)| section["id"] == *id)
            {
                section["title"] = json!(title);
                section["purpose"] = json!(purpose);
            }
            if let Some(original_gap) = section["id"].as_str().and_then(|id| original_gaps.get(id))
            {
                section["gap"] = original_gap.clone();
            }
            if section["workRequest"].is_object() {
                section["workRequest"]["documentationLanguage"] = json!(target);
            }
        }
    }
    Ok(gaps)
}

pub(super) fn narrative_notice_html(gaps: &BTreeMap<String, Value>, language: &str) -> String {
    if gaps.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "<aside class=\"translation-gap gap-card\"><p>{}</p><details><summary>{}: {}</summary><ul>",
        narrative_explanation(language),
        text(
            language,
            "Descriptions without established availability in the selected language",
            "Описания, доступность которых на выбранном языке не установлена"
        ),
        gaps.len()
    );
    for gap in gaps.values() {
        let original = gap["originalPath"].as_str().unwrap_or("");
        let href = original.rsplit('/').next().unwrap_or("");
        out.push_str(&format!(
            "<li>{} · {} · <a href=\"{}\">{}</a></li>",
            escape(gap["title"].as_str().unwrap_or("")),
            super::question_reading::available_language_label(gap, language),
            escape(href),
            text(
                language,
                "Original description and evidence (JSON)",
                "Исходное описание и основания (JSON)"
            )
        ));
    }
    out.push_str("</ul></details></aside>");
    out
}

pub(super) fn narrative_explanation(language: &str) -> &'static str {
    text(
        language,
        "Some retained descriptions have another or unknown prose language. Their originals remain readable; evidence, diagrams and review states are unchanged. Availability in the selected language is not established for the listed descriptions.",
        "Некоторые сохранённые описания имеют другой или неизвестный язык. Их оригиналы доступны для чтения; основания, диаграммы и состояния проверки сохранены. Для перечисленных описаний доступность на выбранном языке не подтверждена.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_flow_and_comparison_languages_preserve_records_without_a_false_translation() {
        for target in ["en", "ru"] {
            for available in [Some("en"), Some("ru"), None] {
                let original = json!({
                    "processDataFlow":{"request":{"documentationLanguage":available,"title":"View evidence","stages":["Store"]},"models":[{"id":"record","fields":[{"id":"value","meaning":{"text":"Overview"}}]}],"scope":"Original flow scope"},
                    "contractComparison":{"documentationLanguage":available,"scope":"Original comparison scope","limitations":["Selected requests only"],"entries":[{"method":"POST","path":"/save","bodyAssessment":"outside-schema"}],"sources":{"fragment":{"text":"class Original {}","textDigest":"unchanged"}}}
                });
                let mut payload = original.clone();
                let mut files = BTreeMap::new();
                let gaps = attach_payload(
                    &mut payload,
                    "scenarios/save.json",
                    Some(target),
                    &mut files,
                )
                .unwrap();
                assert_eq!(gaps.len(), if available == Some(target) { 0 } else { 2 });
                for (key, suffix) in [
                    ("processDataFlow", "data-flow"),
                    ("contractComparison", "comparison"),
                ] {
                    assert_eq!(payload[key], original[key]);
                    let path = format!("scenarios/save.{suffix}.original.json");
                    let retained: Value = serde_json::from_slice(&files[&path]).unwrap();
                    assert_eq!(retained, original[key]);
                }
                let html = page_notices(&gaps);
                for gap in &gaps {
                    assert_eq!(gap["availableLanguage"], json!(available));
                    assert_eq!(gap["requestedLanguage"], target);
                    let name = gap["originalPath"]
                        .as_str()
                        .unwrap()
                        .rsplit('/')
                        .next()
                        .unwrap();
                    assert!(html.contains(&format!("href=\"{name}\"")));
                }
                if !gaps.is_empty() {
                    assert!(html.contains(description_label(&gaps[0], target)));
                    assert!(html.contains(description_label(&gaps[1], target)));
                }
                let no_target =
                    attach_payload(&mut payload, "scenarios/save.json", None, &mut files).unwrap();
                assert!(no_target.is_empty());
                assert_eq!(payload["authoredTranslationGaps"], json!([]));
            }
        }
    }

    #[test]
    fn original_description_paths_cannot_escape_the_export() {
        let mut files = BTreeMap::new();
        let mut page = json!({"contractComparison":{"scope":"original"}});
        assert!(attach_payload(&mut page, "../outside.json", Some("en"), &mut files).is_err());
        assert!(files.is_empty());
        assert!(original_path("../outside").is_err());
    }
    #[test]
    fn original_note_and_assessment_keep_independent_language_and_provenance() {
        for target in ["en", "ru"] {
            for available in [Some("en"), Some("ru"), None] {
                let assessment_language = if target == "en" { "ru" } else { "en" };
                let note = json!({"id":"assessment-operations", "association":{"id":"operations",
                    "title":"Source appendix", "documentationLanguage":available},
                    "original":{"status":"CAPTURED","text":"Original note <Data>{kept}\r\n","digest":"original-digest"},
                    "associationDigest":"association-digest","authority":"HUMAN_OR_IMPORTED_UNVERIFIED",
                    "assessment":{"id":"assessment-operations","title":"Assessment", "documentationLanguage":assessment_language}});
                let mut payload = json!({"notes":[note],"operations":[],"sections":[]});
                let mut files = BTreeMap::new();
                let gaps = attach_narrative(
                    &mut payload,
                    "services/orders.json",
                    Some(target),
                    &mut files,
                )
                .unwrap();
                assert_eq!(payload["notes"][0], note);
                assert_eq!(
                    gaps["assessment-operations"]["availableLanguage"],
                    assessment_language
                );
                assert_eq!(
                    gaps.contains_key("original-note-operations"),
                    available != Some(target)
                );
                assert_eq!(gaps.len(), 1 + usize::from(available != Some(target)));
                let original: Value =
                    serde_json::from_slice(&files["services/orders.note-operations.original.json"])
                        .unwrap();
                assert_eq!(original["association"], note["association"]);
                assert_eq!(original["original"], note["original"]);
                assert_eq!(original["authority"], note["authority"]);
                assert_eq!(original["associationDigest"], note["associationDigest"]);
                assert!(original.get("assessment").is_none());
                assert_eq!(files.len(), 2);
            }
        }
    }

    #[test]
    fn target_language_refreshes_availability_and_requests_without_rewriting_retained_content() {
        for target in ["en", "ru"] {
            for available in [Some("en"), Some("ru"), None] {
                let operation = json!({"id":"section-overview","title":"Source fragments","documentationLanguage":available,"summary":{"text":"Original account"},"events":[{"text":"Revision"}]});
                let mut payload = json!({"operations":[operation],"sections":[{"id":"section-overview","title":"Overview","purpose":"Original generated purpose","content":operation,"gap":"Original diagnostic reason","workRequest":{"entrypoint":"section-overview","documentationLanguage":"en"}}],
                    "notes":[{"assessment":operation}],"requestedDocumentationLanguage":"en","translationComplete":true,"translationGaps":{},
                    "operationSources":{"section-overview":{"source":{"text":"Original source bytes","textDigest":"same","revision":"r1"}}},"operationStates":{"section-overview":{"freshness":"STALE","verification":"UNASSESSED"}}});
                let original = payload.clone();
                let mut files = BTreeMap::new();
                let gaps = attach_narrative(
                    &mut payload,
                    "services/sample.json",
                    Some(target),
                    &mut files,
                )
                .unwrap();
                assert_eq!(payload["operations"], original["operations"]);
                assert_eq!(payload["notes"], original["notes"]);
                assert_eq!(payload["sections"][0]["content"], operation);
                assert_eq!(payload["sections"][0]["gap"], "Original diagnostic reason");
                assert_eq!(payload["operationSources"], original["operationSources"]);
                assert_eq!(payload["operationStates"], original["operationStates"]);
                assert_eq!(payload["requestedDocumentationLanguage"], target);
                assert_eq!(
                    payload["sections"][0]["workRequest"]["documentationLanguage"],
                    target
                );
                assert_eq!(payload["translationComplete"], available == Some(target));
                assert_eq!(gaps.len(), usize::from(available != Some(target)));
                assert_eq!(
                    payload["frozenLanguageAvailability"]["requestedDocumentationLanguage"],
                    "en"
                );
                assert_eq!(
                    payload["frozenLanguageAvailability"]["translationComplete"],
                    true
                );
                let packet: Value = serde_json::from_slice(
                    &files["services/sample.operation-section-overview.original.json"],
                )
                .unwrap();
                assert_eq!(packet["operation"], operation);
                assert_eq!(
                    packet["sources"],
                    original["operationSources"]["section-overview"]
                );
                assert_eq!(
                    packet["state"],
                    original["operationStates"]["section-overview"]
                );
                assert_eq!(
                    files.len(),
                    1,
                    "the operation, section and assessment share one retained operation"
                );
                let notice = narrative_notice_html(&gaps, target);
                assert_eq!(notice.is_empty(), available == Some(target));
                if !gaps.is_empty() {
                    assert!(notice.contains("sample.operation-section-overview.original.json"));
                    assert!(notice.contains("Source fragments"));
                }
                let legacy =
                    attach_narrative(&mut payload, "services/sample.json", None, &mut files)
                        .unwrap();
                assert!(legacy.is_empty());
                assert!(payload["translationComplete"].is_null());
                assert_eq!(
                    payload["frozenLanguageAvailability"]["requestedDocumentationLanguage"],
                    "en"
                );
            }
        }
    }
}
