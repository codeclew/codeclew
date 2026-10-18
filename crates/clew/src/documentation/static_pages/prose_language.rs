//! Language availability is separate from evidence and review status.
use crate::documentation::{language, reader::text, render::escape, store};
use crate::error::ClewError;
use serde_json::{Value, json};

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
        "<aside class=\"translation-gap gap-card\"><h2>{}</h2><p>{}</p><p>{}: {}</p><p><a href=\"{}\">{}</a></p></aside>",
        text(language, "Translation pending", "Требуется перевод"),
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
