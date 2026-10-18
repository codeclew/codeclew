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
        text(language, "Translation pending", "Требуется перевод"),
        description_label(gap, language),
        explanation(language),
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

pub(super) fn description_label(gap: &Value, language: &str) -> &'static str {
    match gap["kind"].as_str() {
        Some("MODEL_TRANSLATION") => text(language, "Model description", "Описание модели"),
        Some("PROCESS_DATA_FLOW_TRANSLATION") => {
            text(language, "Data flow description", "Описание потока данных")
        }
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
}
