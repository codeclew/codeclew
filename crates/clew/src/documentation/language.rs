//! Presentation language never participates in compiler or source-capture identity.
use super::{bindings::Bindings, digest, invalid, model::*, render};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

pub(super) const RUSSIAN_LABELS: &str = include_str!("../../assets/documentation/ru.json");
pub(super) const BROWSER_LOCALE: &str = include_str!("../../assets/documentation/locale.js");

/// Only owned, compiled labels enter this script; source and prose are inserted
/// separately by the caller after the locale implementation has been selected.
pub(super) fn browser_script() -> String {
    format!(
        "var CLEW_RU_MESSAGES={};\n{BROWSER_LOCALE}\n",
        RUSSIAN_LABELS.replace('<', "\\u003c")
    )
}

/// Look up an owned display label. Callers insert source and authored values
/// separately; this function never scans a completed page or rewrites its data.
pub(super) fn label<'a>(language: &str, key: &'a str) -> &'a str {
    if language != "ru" {
        return key;
    }
    static LABELS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    LABELS
        .get_or_init(|| {
            serde_json::from_str(RUSSIAN_LABELS).expect("bundled reader labels are valid JSON")
        })
        .get(key)
        .map(String::as_str)
        .unwrap_or(key)
}

pub(super) fn validate(language: Option<&str>) -> Result<(), ClewError> {
    if language.is_some_and(|value| !matches!(value, "en" | "ru")) {
        return Err(invalid("documentation language must be en or ru"));
    }
    Ok(())
}

/// Only the owned empty template is translated, before inserting authored JSON.
pub(super) fn template(template: &str, language: &str) -> String {
    if language != "ru" {
        return template.to_owned();
    }
    let mut output = template.replace("lang=\"en\"", "lang=\"ru\"");
    for (en, ru) in [
        ("Service documentation", "Документация сервиса"),
        ("Skip to content", "Перейти к содержанию"),
        ("Page tools", "Инструменты страницы"),
        ("Coverage and evidence", "Полнота и источники"),
        ("Documentation navigation", "Навигация по документации"),
        (
            "Find a section, note or operation",
            "Найти раздел, заметку или операцию",
        ),
        (
            "Sections, notes and operations",
            "Разделы, заметки и операции",
        ),
        ("Source and API inventory", "Перечень исходников и API"),
        ("Sections and source", "Разделы и исходный код"),
        ("On this page", "На этой странице"),
        (
            "Sources pinned to revisions",
            "Исходники привязаны к версиям",
        ),
        (
            "Declared contracts and implementation retain separate authority.",
            "Заявленные контракты и реализация рассматриваются отдельно.",
        ),
        ("Documentation freshness", "Актуальность документации"),
        ("Selected source evidence", "Выбранный исходный код"),
        ("SOURCE EVIDENCE", "ИСХОДНЫЙ КОД"),
        ("Close source panel", "Закрыть исходный код"),
        (">Source</label>", ">Исходник</label>"),
        (">Copy</button>", ">Скопировать</button>"),
        (
            "Exact source with line numbers",
            "Исходный код с номерами строк",
        ),
    ] {
        output = output.replace(en, ru);
    }
    output
}

pub(super) fn missing(language: &str) -> &'static str {
    if language == "ru" {
        "Этот раздел пока не подготовлен на русском языке."
    } else {
        "This section has not yet been prepared in English."
    }
}

pub(super) fn gaps(
    narrative: &Narrative,
    requested: Option<&str>,
    previous: Option<&(String, Bindings)>,
    old_data: Option<&Value>,
    folder: &str,
    id: &str,
) -> Result<Value, ClewError> {
    let mut gaps = serde_json::Map::new();
    let Some(requested) = requested else {
        return Ok(Value::Object(gaps));
    };
    for op in &narrative.operations {
        if op.documentation_language.as_deref() == Some(requested) {
            continue;
        }
        let mut href = None;
        if let Some((bundle, binding)) = previous {
            let prior = binding
                .narratives
                .get(&narrative.subject)
                .and_then(|n| n.operations.iter().find(|old| old.id == op.id));
            if prior.map(digest).transpose()? == Some(digest(op)?) {
                let prior_target =
                    old_data.and_then(|data| data["requestedDocumentationLanguage"].as_str());
                if prior_target.is_none() || prior_target == op.documentation_language.as_deref() {
                    href = Some(format!("../../{bundle}/{folder}/{id}.html#{}", op.id));
                } else {
                    // Reuse the existing terminal link, rather than pointing to another placeholder.
                    href = old_data
                        .and_then(|data| data["translationGaps"][&op.id]["href"].as_str())
                        .map(str::to_owned);
                }
            }
        }
        gaps.insert(
            op.id.clone(),
            json!({"requestedLanguage":requested,
            "availableLanguage":op.documentation_language,"href":href}),
        );
    }
    Ok(Value::Object(gaps))
}

pub(super) fn display_narrative(n: &Narrative, requested: Option<&str>) -> Narrative {
    let mut display = n.clone();
    if let Some(language) = requested {
        display.operations.retain(|op| {
            let matches = op.documentation_language.as_deref() == Some(language);
            if !matches {
                display.gaps.insert(op.id.clone(), missing(language).into());
            }
            matches
        });
    }
    display
}

pub(super) fn markdown(
    title: &str,
    n: &Narrative,
    states: &BTreeMap<String, SectionState>,
    language: &str,
) -> String {
    if language != "ru" {
        return render::markdown(title, n, states);
    }
    let mut out = format!(
        "# {}\n\nОписание по исходному коду. Поведение работающей системы не проверено.\n\n",
        render::escape(title)
    );
    for (id, title, _) in super::sections::REQUIRED {
        if !n.subject.starts_with("service:") {
            break;
        }
        let title = match id {
            "section-overview" => "Обзор",
            "section-responsibilities" => "Назначение и процессы",
            "section-entities" => "Предметные сущности",
            "section-ingress" => "Точки входа",
            "section-egress" => "Исходящие вызовы",
            _ => title,
        };
        out.push_str(&format!(
            "## {title}\n\n{}\n\n",
            n.operations
                .iter()
                .find(|o| o.id == id)
                .map(|o| render::escape(&o.summary.text))
                .unwrap_or_else(|| missing(language).into())
        ));
    }
    for op in &n.operations {
        if !super::sections::contains(&op.id) {
            out.push_str(&format!(
                "## {}\n\n{}\n\n",
                render::escape(&op.title),
                render::escape(&op.summary.text)
            ));
        }
        if let Some(state) = states.get(&format!("{}/{}", n.subject, op.id)) {
            out.push_str(&format!(
                "Актуальность исходников: {}. Проверка смысла: {}.\n\n",
                state.freshness.as_str(),
                render::escape(&state.verification)
            ));
        }
        for paragraph in &op.explanation {
            out.push_str(&format!("{}\n\n", render::escape(&paragraph.text)));
            if let Some(authorship) = &paragraph.authorship {
                out.push_str(&format!("Пользовательская документация. Указанный автор: {}. Проверка смысла: UNASSESSED. Исходно связанный код — сохранённый непроверенный контекст (снимок {}).\n\n",
                    render::escape(&authorship.author), render::escape(&authorship.source_snapshot)));
            }
        }
        for visual in &op.visuals {
            out.push_str(&format!(
                "### {}\n\n{}\n\nОбласть описания: {}\n\n",
                render::escape(&visual.title),
                render::escape(&visual.purpose.text),
                render::escape(&visual.scope.text)
            ));
            for fragment in super::visuals::fragments(visual).into_iter().skip(2) {
                out.push_str(&format!("- {}\n", render::escape(&fragment.text)));
            }
            for limit in &visual.limitations {
                out.push_str(&format!("\nОграничение: {}\n", render::escape(limit)));
            }
        }
        for contract in &op.interface_contracts {
            out.push_str(&format!(
                "### {}\n\n| Элемент | Значение или поведение |\n|---|---|\n",
                render::escape(&contract.title)
            ));
            for row in &contract.rows {
                out.push_str(&format!(
                    "| {} | {} |\n",
                    render::escape(&row.label).replace('|', "&#124;"),
                    render::escape(&row.value)
                        .replace('|', "&#124;")
                        .replace('\n', "<br>")
                ));
            }
            for boundary in &contract.boundaries {
                out.push_str(&format!("\n{}\n", render::escape(boundary)));
            }
        }
        for finding in &op.findings {
            out.push_str(&format!("- {}\n", render::escape(&finding.text)));
        }
        for boundary in &op.boundaries {
            out.push_str(&format!("\nОграничение: {}\n", render::escape(boundary)));
        }
        if !op.events.is_empty() {
            out.push_str(&format!("\n<details><summary>Детали реализации</summary>\n\n```mermaid\n{}```\n</details>\n",render::mermaid(op)));
        }
    }
    for id in n.gaps.keys() {
        out.push_str(&format!(
            "\n## {}\n\nРаздел требует дополнения; сведения о пробеле сохранены в JSON.\n",
            render::escape(id)
        ));
    }
    out
}

pub(super) fn section_labels(data: &mut Value, language: &str) {
    if language != "ru" {
        return;
    }
    for section in data["sections"].as_array_mut().into_iter().flatten() {
        let (title, purpose) = match section["id"].as_str().unwrap_or("") {
            "section-overview" => (
                "Обзор",
                "Назначение сервиса, область ответственности и результаты его работы.",
            ),
            "section-responsibilities" => (
                "Назначение и процессы",
                "Обязанности сервиса и подтверждённые по коду процессы.",
            ),
            "section-entities" => (
                "Предметные сущности",
                "Предметные понятия отдельно от объектов передачи данных и хранения.",
            ),
            "section-ingress" => (
                "Точки входа",
                "Входящие вызовы и заявленные контракты с явными ограничениями анализа.",
            ),
            "section-egress" => (
                "Исходящие вызовы",
                "Вызовы внешних систем и отправка сообщений с явными неизвестными участками.",
            ),
            _ => continue,
        };
        section["title"] = json!(title);
        section["purpose"] = json!(purpose);
        if section["content"].is_null() {
            section["gap"] = json!("Описание раздела по исходному коду ещё не принято.");
        }
        section["workRequest"]["documentationLanguage"] = json!(language);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn narrative() -> Narrative {
        serde_json::from_value(json!({"schema":"codeclew-documentation-narrative/1.3","subject":"service:orders","contextDigest":"digest","operations":[{"id":"section-overview","title":"Orders","summary":{"id":"f","text":"English source explanation", "dependencyIds":["d"],"sourceIds":["s"]},"participants":[],"events":[]}]})).unwrap()
    }
    #[test]
    fn unknown_or_other_language_is_a_translation_gap_without_relabeling() {
        let mut n = narrative();
        let original = serde_json::to_value(&n).unwrap();
        assert!(display_narrative(&n, Some("ru")).operations.is_empty());
        assert_eq!(serde_json::to_value(&n).unwrap(), original);
        assert!(
            original["operations"][0]
                .get("documentationLanguage")
                .is_none()
        );
        n.operations[0].documentation_language = Some("en".into());
        let gap = gaps(&n, Some("ru"), None, None, "services", "orders").unwrap();
        assert_eq!(gap["section-overview"]["availableLanguage"], "en");
        assert!(gap["section-overview"]["href"].is_null());
        assert_eq!(display_narrative(&n, Some("en")).operations.len(), 1);
        assert_eq!(display_narrative(&n, None).operations.len(), 1);
    }
    #[test]
    fn russian_template_localizes_only_owned_template_and_preserves_data_token() {
        let template = template(
            include_str!("../../assets/documentation/template.html"),
            "ru",
        );
        assert!(template.contains("lang=\"ru\""));
        assert!(template.contains("Перечень исходников и API"));
        assert!(template.contains("<summary>На этой странице</summary>"));
        assert!(template.contains("aria-label=\"На этой странице\""));
        assert!(!template.contains("On this page"));
        let filled = template.replace("__DOCUMENT_DATA__", r#"{"text":"On this page"}"#);
        assert!(filled.contains(r#"{"text":"On this page"}"#));
        assert_eq!(
            super::template(
                include_str!("../../assets/documentation/template.html"),
                "en"
            ),
            include_str!("../../assets/documentation/template.html")
        );
        assert!(!template.contains("Sections and source"));
        assert!(template.contains("__DOCUMENT_DATA__"));
        assert!(validate(Some("fr")).is_err());
        if let Some(root) = std::env::var_os("CODECLEW_UI_NATIVE_FIXTURE_ROOT") {
            let root = std::path::PathBuf::from(root);
            let output = root.join("native");
            std::fs::create_dir_all(&output).unwrap();
            for entry in std::fs::read_dir(root.join("inputs")).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                let data: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                let language = data["language"].as_str().unwrap();
                let html = crate::documentation::render::html(&data).unwrap();
                if language == "ru" {
                    assert!(html.contains("<summary>На этой странице</summary>"));
                    assert!(html.contains("Полнота и источники"));
                }
                let navigation = crate::documentation::reader::navigation_language(
                    "",
                    "index.html",
                    None,
                    &[],
                    language,
                );
                let html = crate::documentation::reader::decorate(&html, &navigation);
                std::fs::write(
                    output
                        .join(path.file_stem().unwrap())
                        .with_extension("html"),
                    html,
                )
                .unwrap();
            }
        }
    }
    #[test]
    fn russian_markdown_never_includes_filtered_english_prose_or_changes_identifiers() {
        let n = narrative();
        let output = markdown(
            "Orders",
            &display_narrative(&n, Some("ru")),
            &BTreeMap::new(),
            "ru",
        );
        assert!(!output.contains("English source explanation"));
        assert!(output.contains("Orders"));
        assert!(output.contains("Точки входа"));
    }
}
