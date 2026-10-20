use super::super::io_error;
use super::*;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

/// Text is inert in HTML and JSX, including MDX expression delimiters.
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '{' => out.push_str("&#123;"),
            '}' => out.push_str("&#125;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            '*' | '_' | '[' | ']' | '`' | '\\' => out.push_str(&format!("&#{};", c as u32)),
            _ => out.push(c),
        }
    }
    out
}
fn paragraph(s: &str) -> String {
    format!("<p>{}</p>\n", escape(s))
}
fn list(items: impl IntoIterator<Item = String>) -> String {
    format!(
        "<ul>\n{}</ul>\n",
        items
            .into_iter()
            .map(|s| format!("<li>{}</li>\n", escape(&s)))
            .collect::<String>()
    )
}
fn content(p: &Projection, language: &str, notice: &str) -> String {
    let l = |en, ru| language::choose(language, en, ru);
    let mut out = format!("<main>\n<h1>{}</h1>\n{notice}", escape(&p.title));
    out += &paragraph(l(
        "Source interpretation with a declared profile. This is a static possible path, not a runtime trace or an incident diagnosis.",
        "Интерпретация исходного кода по заявленному профилю. Это возможный статический путь, а не трассировка выполнения или диагностика инцидента.",
    ));
    out += &paragraph(&format!(
        "{}: {}. {}: {} ({}). {}: {}.",
        l("Framework", "Фреймворк"),
        p.framework,
        l("Engine version", "Версия движка"),
        p.engine_version
            .as_deref()
            .unwrap_or(l("unresolved", "не установлена")),
        l("profile declaration", "заявлено профилем"),
        l("Service", "Сервис"),
        p.service
    ));
    out += &format!(
        "<nav aria-label=\"{}\"><a href=\"#overview\">{}</a> · <a href=\"#steps\">{}</a> · <a href=\"#sources\">{}</a></nav>\n<h2 id=\"overview\">{}</h2>\n",
        l("Page sections", "Разделы страницы"),
        l("Overview", "Обзор"),
        l("Steps and decisions", "Шаги и решения"),
        l("Sources and version", "Источники и версия"),
        l("Overview", "Обзор")
    );
    out += &paragraph(l(
        "Phases group source declarations for reading. Open each phase for its full details; category order is not execution order.",
        "Этапы объединяют объявления исходного кода для чтения. Раскройте этап для подробностей; порядок категорий не задаёт порядок выполнения.",
    ));
    out += "<ol>\n";
    for phase in &p.phases {
        out += &format!(
            "<li><a href=\"#{}\">{}</a></li>\n",
            phase.id,
            escape(&phase.label)
        );
    }
    out += &format!(
        "</ol>\n<h2 id=\"steps\">{}</h2>\n",
        l("Steps and decisions", "Шаги и решения")
    );
    for phase in &p.phases {
        out += &format!(
            "<section id=\"{}\">\n<h3>{}</h3>\n",
            phase.id,
            escape(&phase.label)
        );
        for id in &phase.item_ids {
            let i = p.items.iter().find(|i| &i.id == id).unwrap();
            out += &format!(
                "<details id=\"{}\">\n<summary>{}</summary>\n",
                i.id,
                escape(&i.label)
            );
            out += &paragraph(&format!(
                "{}: {}. {}: {}; {}: {}. {}.",
                l("Record kind", "Вид записи"),
                i.kind,
                l("Binding", "Привязка"),
                i.reference.binding_authority,
                l("call target", "цель вызова"),
                i.reference.call_target_authority,
                l(
                    "Semantic authority: source interpretation with declared profile",
                    "Основание интерпретации: исходный код и заявленный профиль"
                )
            ));
            if !i.conditions.is_empty() {
                out += &format!("<h4>{}</h4>\n", l("Condition path", "Путь условия"));
                out += &list(i.conditions.clone());
            }
            if !i.members.is_empty() {
                let title = if i.kind == "registry-binding" {
                    l(
                        "Lookup key and callback (unordered registry)",
                        "Ключ поиска и обработчик (неупорядоченный реестр)",
                    )
                } else if i.kind == "callback-group" {
                    l(
                        "Declared callbacks (group does not establish parallelism)",
                        "Объявленные обработчики (группа не доказывает параллелизм)",
                    )
                } else {
                    l("Declared members", "Объявленные элементы")
                };
                out += &format!("<h4>{}</h4>\n", title);
                out += &list(i.members.clone());
            }
            if !i.expression.is_empty() {
                out += &format!("<pre>{}</pre>\n", escape(&i.expression));
            }
            out += &list(i.limitations.clone());
            let source = &p.sources[&i.reference.source_id];
            let href = if source.text.is_empty() {
                source
                    .url
                    .as_ref()
                    .map(|u| {
                        super::super::source_export::range_url(
                            u,
                            i.reference.start_line,
                            i.reference.end_line,
                        )
                    })
                    .unwrap_or_default()
            } else {
                format!(
                    "sources.html#source-{}",
                    p.sources
                        .keys()
                        .position(|id| id == &i.reference.source_id)
                        .unwrap()
                        + 1
                )
            };
            out += &format!(
                "<p><a href=\"{}\">{}: {}–{}</a></p>\n",
                escape(&href),
                l("Source: lines", "Исходный код: строки"),
                i.reference.start_line,
                i.reference.end_line
            );
            out += "</details>\n";
        }
        out += "</section>\n";
    }
    out += &format!(
        "<details id=\"sources\">\n<summary>{}</summary>\n",
        l("Sources and version", "Источники и версия")
    );
    out += &paragraph(&format!(
        "{}: {}. {}: {}. {}: {}.",
        l("Revision", "Версия"),
        p.revision,
        l("Original snapshot", "Исходный снимок"),
        p.snapshot,
        l("Profile digest", "Хеш профиля"),
        p.profile_digest
    ));
    out += &list(p.limitations.clone());
    out += &format!(
        "<p><a href=\"sources.html\">{}</a> · <a href=\"projection.json\">{}</a></p>\n</details>\n</main>\n",
        l(
            "Retained source appendix",
            "Приложение с сохранённым исходным кодом"
        ),
        l("Typed projection JSON", "Типизированная проекция JSON")
    );
    out
}
fn html(title: &str, body: &str, language: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"{language}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"></head><body>\n{body}</body></html>\n",
        escape(title)
    )
}
const CSS: &str = "html{background:#f7f4ec;color:#172d49;font-family:system-ui,sans-serif;line-height:1.6}body{margin:0}main{max-width:960px;margin:auto;padding:32px 24px;overflow-wrap:anywhere}h1{line-height:1.2}a{color:#245d97}nav{margin:20px 0}details{border-top:1px solid #d4d9df;padding:12px 0}summary{cursor:pointer;font-weight:600}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#edf0f2;padding:12px;font-size:14px}li{margin:8px 0}section{scroll-margin-top:16px}@media(max-width:600px){main{padding:20px 16px}h1{font-size:28px}nav a{display:inline-block;margin:4px 0}}\n";
#[cfg(test)]
pub(super) fn write(output: &Path, p: &Projection) -> Result<(), ClewError> {
    write_inner(
        output,
        p,
        None,
        "en",
        &super::super::source_export::Policy::default(),
    )
}
#[cfg(test)]
pub(super) fn write_language(
    output: &Path,
    p: &Projection,
    profile: &Profile,
    language: &str,
) -> Result<(), ClewError> {
    super::super::language::validate(Some(language))?;
    write_inner(
        output,
        p,
        Some(profile),
        language,
        &super::super::source_export::Policy::default(),
    )
}
pub(super) fn write_source_policy(
    output: &Path,
    p: &Projection,
    profile: &Profile,
    language: &str,
    policy: &super::super::source_export::Policy,
) -> Result<(), ClewError> {
    super::super::language::validate(Some(language))?;
    write_inner(output, p, Some(profile), language, policy)
}
fn write_inner(
    output: &Path,
    p: &Projection,
    profile: Option<&Profile>,
    language: &str,
    source_policy: &super::super::source_export::Policy,
) -> Result<(), ClewError> {
    let l = |en, ru| language::choose(language, en, ru);
    let available = profile.and_then(|p| p.documentation_language.as_deref());
    let complete = profile.is_none() || available == Some(language);
    let title = p.title.as_str();
    let gap_heading = if available.is_none() {
        l(
            "Availability in the selected language is not established",
            "Доступность на выбранном языке не установлена",
        )
    } else {
        l("Translation required", "Требуется перевод")
    };
    let notice = if complete {
        String::new()
    } else {
        format!(
            "<aside class=\"translation-gap\"><h2>{}</h2><p>{}</p><dl><dt>{}</dt><dd>{}</dd><dt>{}</dt><dd>{}</dd></dl><p><a href=\"profile.json\">{}</a> · <a href=\"projection.json\">{}</a> · <a href=\"sources.html\">{}</a></p><p>{}</p></aside>",
            gap_heading,
            if available.is_none() {
                l(
                    "The original profile has no declared prose language. Its original text and source bindings are preserved; no translation or review is inferred.",
                    "Язык авторского текста исходного профиля не указан. Исходный текст и привязки к коду сохранены; перевод и проверка не подразумеваются.",
                )
            } else {
                l(
                    "The profile prose has not been prepared in the requested language. Its original text is preserved; no translation or review is inferred.",
                    "Авторский текст профиля не подготовлен на запрошенном языке. Оригинал сохранён; перевод и проверка не подразумеваются.",
                )
            },
            l("Requested language", "Запрошенный язык"),
            language,
            l("Available prose language", "Язык доступного текста"),
            available.unwrap_or(l("unknown", "неизвестен")),
            l(
                "Original authored profile",
                "Оригинальный авторский профиль"
            ),
            l(
                "Original interpretation with source bindings",
                "Исходная интерпретация с привязками к коду"
            ),
            l(
                "Retained source appendix",
                "Приложение с сохранённым исходным кодом"
            ),
            l(
                "Readable original interpretation below",
                "Ниже доступна исходная интерпретация для чтения"
            )
        )
    };
    let body = content(p, language, &notice);
    let appendix_title = l(
        "Retained source appendix",
        "Приложение с сохранённым исходным кодом",
    );
    let mut appendix = format!(
        "<main><h1>{appendix_title}</h1><p><a href=\"page.html#sources\">{}</a></p>\n",
        l("Return to process", "Вернуться к процессу")
    );
    for (index, s) in p.sources.values().enumerate() {
        appendix += &format!(
            "<details id=\"source-{}\"><summary>{}:{}–{}</summary>\n",
            index + 1,
            escape(&s.file),
            s.start_line,
            s.end_line
        );
        appendix += &paragraph(&format!(
            "{}: {}. {}: {}. {}: {}. {}.",
            l("Source authority", "Основание исходного кода"),
            s.authority,
            l("Revision", "Версия"),
            s.revision,
            l("Text digest", "Хеш текста"),
            s.text_digest,
            l(
                "Exact retained source is distinct from DSL interpretation",
                "Точный сохранённый исходный код рассматривается отдельно от интерпретации DSL"
            )
        ));
        if s.text.is_empty() {
            if let Some(url) = &s.url {
                appendix += &format!(
                    "<p><a href=\"{}\">{}:{}–{} ↗</a></p>",
                    escape(url),
                    escape(&s.file),
                    s.start_line,
                    s.end_line
                );
            }
        } else {
            appendix += &format!("<pre>{}</pre>", escape(&s.text));
        }
        appendix += "</details>\n";
    }
    appendix += "</main>\n";
    let mdx = format!(
        "<link rel=\"stylesheet\" href=\"./style.css\" />\n\n<div lang=\"{language}\">\n{body}</div>\n"
    );
    let mut files = BTreeMap::from([
        ("page.mdx", mdx.into_bytes()),
        ("page.html", html(title, &body, language).into_bytes()),
        ("style.css", CSS.as_bytes().to_vec()),
        (
            "sources.html",
            html(appendix_title, &appendix, language).into_bytes(),
        ),
        (
            "projection.json",
            serde_json::to_vec_pretty(p).map_err(io_error)?,
        ),
    ]);
    if let Some(profile) = profile {
        files.insert(
            "profile.json",
            serde_json::to_vec_pretty(profile).map_err(io_error)?,
        );
    }
    let mut manifest = json!({"schema":"codeclew-flow-dsl-static-manifest/1.0","pageId":p.id,"documentationLanguage":language,"availableDocumentationLanguage":available,"translationComplete":complete,"translationGaps":if complete {json!([])} else {json!([{"kind":"PROFILE_PROSE_LANGUAGE","requestedLanguage":language,"availableLanguage":available,"original":"profile.json"}])},"snapshot":p.snapshot,"revision":p.revision,"profileDigest":p.profile_digest,"contentDigest":crate::canonical::hash_bytes(body.as_bytes()),"mdxProfile":"MDX 3; inert native JSX; no executable expressions or imports","authority":p.authority,"files":files.iter().map(|(name,bytes)|json!({"path":name,"digest":crate::canonical::hash_bytes(bytes)})).collect::<Vec<_>>()});
    manifest["sourceExport"] = source_policy.manifest()?;
    manifest["exportIdentity"] = super::super::digest(&(p, source_policy))?.into();
    files.insert(
        "manifest.json",
        serde_json::to_vec_pretty(&manifest).map_err(io_error)?,
    );
    if output.exists() {
        let m = fs::symlink_metadata(output).map_err(io_error)?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(invalid("flow DSL output must be a regular directory"));
        }
    } else {
        fs::create_dir_all(output).map_err(io_error)?;
    }
    // Never overwrite authored or unrelated files; preflight every generated name before writing.
    for name in files.keys() {
        store::relative(name)?;
        if fs::symlink_metadata(output.join(name)).is_ok() {
            return Err(invalid(
                "flow DSL output contains a generated filename; choose a new output directory",
            ));
        }
    }
    for (name, data) in files {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))
            .map_err(io_error)?;
        file.write_all(&data).map_err(io_error)?;
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn body(p: &Projection) -> String {
    content(p, "en", "")
}
