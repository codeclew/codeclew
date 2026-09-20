//! Offline navigation and product guides shared by every new publication.
use super::{io_error, render, store::Repository};
use crate::error::ClewError;
use std::collections::BTreeMap;

pub(super) const HELP: &str = include_str!("../../assets/documentation/help.html");
pub(super) const RUNBOOKS: &str = include_str!("../../assets/documentation/runbooks.html");
pub(super) const STYLE: &str = include_str!("../../assets/documentation/reader.css");
pub(super) const SCRIPT: &str = include_str!("../../assets/documentation/reader.js");
pub(super) const LIMITS: &str = include_str!("../../assets/documentation/limits.js");
pub(super) const ICON: &str = include_str!("../../assets/documentation/favicon.svg");

pub(super) fn favicon() -> String {
    let encoded = ICON
        .bytes()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    format!(
        "<link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml,{encoded}\"><meta name=\"theme-color\" content=\"#12334e\">"
    )
}

pub(super) fn page(title: &str, body: &str) -> String {
    page_language(title, body, "en")
}

pub(super) fn text<'a>(language: &str, en: &'a str, ru: &'a str) -> &'a str {
    if language == "ru" { ru } else { en }
}

pub(super) fn page_language(title: &str, body: &str, language: &str) -> String {
    let language = text(language, "en", "ru");
    format!(
        "<!doctype html><html lang=\"{language}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body><main class=\"reader-guide\">{body}</main></body></html>",
        render::escape(title),
        render::STYLE
    )
}

pub(super) fn navigation(
    prefix: &str,
    overview: &str,
    history: Option<&str>,
    pages: &[String],
) -> String {
    navigation_language(prefix, overview, history, pages, "en")
}

pub(super) fn navigation_language(
    prefix: &str,
    overview: &str,
    history: Option<&str>,
    pages: &[String],
    language: &str,
) -> String {
    let _ = pages;
    let icon = ICON.find("<svg").map(|i| &ICON[i..]).unwrap_or(ICON);
    let label = |en, ru| text(language, en, ru);
    let history_label = label("History", "История");
    let documentation = label("Documentation", "Документация");
    let overview_label = label("Overview", "Обзор");
    let browse = label("Browse", "Каталог");
    let help = label("Help", "Справка (английский)");
    let runbooks = label("Runbooks", "Инструкции (английский)");
    let search = label("Search document names", "Поиск по названиям документов");
    let placeholder = label("Search services, processes…", "Найти сервис или процесс…");
    let history_link = history
        .map(|path| format!("<a href=\"{}\">{history_label}</a>", render::escape(path)))
        .unwrap_or_default();
    format!(
        "<nav class=\"reader-nav\" aria-label=\"{documentation}\"><a class=\"reader-brand\" href=\"{}\">{icon}<span>Codeclew</span></a><div class=\"reader-links\"><a href=\"{}\">{overview_label}</a><a href=\"{prefix}catalog.html\">{browse}</a><a href=\"{prefix}help.html\">{help}</a><a href=\"{prefix}runbooks.html\">{runbooks}</a>{history_link}</div><form class=\"reader-search\" action=\"{prefix}catalog.html\" method=\"get\" role=\"search\"><input type=\"search\" name=\"q\" aria-label=\"{search}\" placeholder=\"{placeholder}\"></form></nav>",
        render::escape(overview),
        render::escape(overview)
    )
}

pub(super) fn decorate(html: &str, navigation: &str) -> String {
    html.replacen(
        "</head>",
        &format!("{}<style>{STYLE}</style></head>", favicon()),
        1,
    )
    .replacen("<body>", &format!("<body>{navigation}"), 1)
    .replacen(
        "</body>",
        &format!("<script>{SCRIPT}\n{LIMITS}</script></body>"),
        1,
    )
}

pub(super) fn catalog(files: &BTreeMap<String, Vec<u8>>) -> String {
    catalog_language(files, "en")
}

pub(super) fn catalog_language(files: &BTreeMap<String, Vec<u8>>, language: &str) -> String {
    let rows = files
        .iter()
        .filter_map(|(path, data)| {
            let kind = if path.starts_with("services/") {
                "Service"
            } else if path.starts_with("scenarios/") {
                "Process"
            } else {
                return None;
            };
            if !path.ends_with(".html") {
                return None;
            }
            let html = String::from_utf8_lossy(data);
            let payload = html
                .split("id=\"document-data\">")
                .nth(1)
                .and_then(|s| s.split("</script>").next())
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
            let id = path
                .split('/')
                .next_back()
                .unwrap_or(path)
                .trim_end_matches(".html");
            let title = payload
                .as_ref()
                .and_then(|v| v["title"].as_str())
                .unwrap_or(id);
            let kind = if payload.as_ref().is_some_and(|v| !v["view"].is_null()) {
                "Dataflow"
            } else {
                kind
            };
            Some(serde_json::json!({"id":id,"title":title,"kind":kind,"href":path}))
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_string(&rows)
        .expect("catalog strings serialize")
        .replace('<', "\\u003c");
    if language == "ru" {
        page_language("Каталог документации", &format!(r#"<div class="eyebrow">КАТАЛОГ ДОКУМЕНТАЦИИ</div><h1>Найти сервис или процесс</h1><p>Ищите по названию или идентификатору. Откройте документ, чтобы посмотреть его разделы, операции и подтверждающие данные.</p><div class="catalog-controls"><input id="catalog-query" type="search" aria-label="Найти документы" placeholder="Сервис, процесс или движение данных"><select id="catalog-kind" aria-label="Тип документа"><option value="">Все типы</option><option value="Service">Сервис</option><option value="Process">Процесс</option><option value="Dataflow">Движение данных</option></select></div><p id="catalog-status" class="catalog-status" role="status" aria-live="polite"></p><ul id="catalog-results" class="catalog-results"></ul><div class="catalog-pager"><button id="catalog-prev" type="button">Назад</button><button id="catalog-next" type="button">Далее</button></div><noscript><p>Для поиска в локальном каталоге включите JavaScript или откройте обзор через меню.</p></noscript><script id="catalog-data" type="application/json">{payload}</script>"#), language).replace("class=\"reader-guide\"", "class=\"reader-guide catalog-page\"")
    } else {
        page("Browse documentation", &format!(r#"<div class="eyebrow">DOCUMENTATION CATALOG</div><h1>Find a service or process</h1><p>Search by name or identifier. Open a document to browse its sections, operations and evidence.</p><div class="catalog-controls"><input id="catalog-query" type="search" aria-label="Find documents" placeholder="Service, process or entity flow"><select id="catalog-kind" aria-label="Document type"><option value="">All types</option><option>Service</option><option>Process</option><option>Dataflow</option></select></div><p id="catalog-status" class="catalog-status" role="status" aria-live="polite"></p><ul id="catalog-results" class="catalog-results"></ul><div class="catalog-pager"><button id="catalog-prev" type="button">Previous</button><button id="catalog-next" type="button">Next</button></div><noscript><p>Enable JavaScript to search this offline catalog, or open the overview from the navigation.</p></noscript><script id="catalog-data" type="application/json">{payload}</script>"#)).replace("class=\"reader-guide\"", "class=\"reader-guide catalog-page\"")
    }
}

pub(super) fn guides(files: &mut BTreeMap<String, Vec<u8>>) {
    files.insert("help.html".into(), page("Codeclew help", HELP).into_bytes());
    files.insert(
        "runbooks.html".into(),
        page("Codeclew runbooks", RUNBOOKS).into_bytes(),
    );
}

pub(super) fn init(repo: &Repository) -> Result<(), ClewError> {
    let overview = if repo.path("docs/index.html")?.exists() {
        "index.html"
    } else {
        "help.html"
    };
    let nav = navigation("", overview, None, &[]);
    for (name, title, body) in [
        ("help.html", "Codeclew help", HELP),
        ("runbooks.html", "Codeclew runbooks", RUNBOOKS),
    ] {
        if !repo.path(&format!("docs/{name}"))?.exists() {
            repo.atomic(
                &format!("docs/{name}"),
                decorate(&page(title, body), &nav).as_bytes(),
            )?;
        }
    }
    if !repo.path("docs/catalog.html")?.exists() {
        repo.atomic(
            "docs/catalog.html",
            decorate(&catalog(&BTreeMap::new()), &nav).as_bytes(),
        )?;
    }
    Ok(())
}

fn owned_catalog(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Some((marker, body)) = text.split_once('\n') else {
        return false;
    };
    marker
        == format!(
            "<!-- codeclew-catalog {} -->",
            crate::canonical::hash_bytes(body.as_bytes())
        )
}

/// Upgrade only byte-identical starter output; preserve any user's guide edits.
#[cfg(test)]
pub(super) fn connect_starters(
    repo: &Repository,
    bundle: &str,
    pages: &[String],
) -> Result<(), ClewError> {
    connect_starters_language(repo, bundle, pages, "en")
}

pub(super) fn connect_starters_language(
    repo: &Repository,
    bundle: &str,
    pages: &[String],
    language: &str,
) -> Result<(), ClewError> {
    let starter_nav = navigation("", "help.html", None, &[]);
    let nav = navigation_language(
        &format!("generated/{bundle}/"),
        "index.html",
        Some("history.html"),
        pages,
        language,
    );
    for (name, title, body) in [
        ("help.html", "Codeclew help", HELP),
        ("runbooks.html", "Codeclew runbooks", RUNBOOKS),
    ] {
        let path = format!("docs/{name}");
        if std::fs::read(repo.path(&path)?).ok().as_deref()
            == Some(decorate(&page(title, body), &starter_nav).as_bytes())
        {
            repo.atomic(&path, decorate(&page(title, body), &nav).as_bytes())?;
        }
    }
    let catalog_path = repo.path("docs/catalog.html")?;
    let existing = std::fs::read(&catalog_path).ok();
    let empty = catalog(&BTreeMap::new());
    if existing.is_none()
        || existing.as_deref().is_some_and(owned_catalog)
        || existing.as_deref() == Some(decorate(&empty, &starter_nav).as_bytes())
        || existing.as_deref()
            == Some(decorate(&empty, &navigation("", "index.html", None, &[])).as_bytes())
    {
        let mut documents = BTreeMap::new();
        for path in pages.iter().filter(|p| p.ends_with(".html")) {
            if let Ok(bytes) = std::fs::read(repo.path(&format!("docs/generated/{bundle}/{path}"))?)
            {
                documents.insert(path.clone(), bytes);
            }
        }
        let body = catalog_language(&documents, language)
            .replace(
                "\"href\":\"services/",
                &format!("\"href\":\"generated/{bundle}/services/"),
            )
            .replace(
                "\"href\":\"scenarios/",
                &format!("\"href\":\"generated/{bundle}/scenarios/"),
            );
        let html = decorate(&body, &nav);
        let owned = format!(
            "<!-- codeclew-catalog {} -->\n{html}",
            crate::canonical::hash_bytes(html.as_bytes())
        );
        repo.atomic("docs/catalog.html", owned.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn decorate_bundle(
    files: &mut BTreeMap<String, Vec<u8>>,
    bundle: &str,
) -> Result<(), ClewError> {
    decorate_bundle_language(files, bundle, "en")
}

pub(super) fn decorate_bundle_language(
    files: &mut BTreeMap<String, Vec<u8>>,
    bundle: &str,
    language: &str,
) -> Result<(), ClewError> {
    guides(files);
    files.insert(
        "catalog.html".into(),
        catalog_language(files, language).into_bytes(),
    );
    let pages = files.keys().cloned().collect::<Vec<_>>();
    for (name, contents) in files.iter_mut().filter(|(name, _)| name.ends_with(".html")) {
        let (prefix, overview, history) = if name == "root-overview.html" {
            (
                format!("generated/{bundle}/"),
                "index.html".to_owned(),
                "history.html",
            )
        } else if name.contains('/') {
            (
                "../".to_owned(),
                "../overview.html".to_owned(),
                "../../../history.html",
            )
        } else {
            (
                String::new(),
                "overview.html".to_owned(),
                "../../history.html",
            )
        };
        let nav = navigation_language(&prefix, &overview, Some(history), &pages, language);
        *contents = decorate(
            &String::from_utf8(contents.clone()).map_err(io_error)?,
            &nav,
        )
        .into_bytes();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn russian_catalog_localizes_controls_without_translating_authored_titles() {
        let mut files = BTreeMap::from([(
            "services/orders.html".into(),
            page(
                "OrderAPI",
                "<script id=\"document-data\">{\"title\":\"OrderAPI\"}</script>",
            )
            .into_bytes(),
        )]);
        decorate_bundle_language(&mut files, "snapshot", "ru").unwrap();
        let catalog = String::from_utf8_lossy(&files["catalog.html"]);
        assert!(catalog.contains("<html lang=\"ru\">"));
        assert!(catalog.contains("Найти сервис или процесс"));
        assert!(catalog.contains("value=\"Service\">Сервис"));
        assert!(catalog.contains("OrderAPI"));
        assert!(catalog.contains("Справка (английский)"));
        let guide = String::from_utf8_lossy(&files["help.html"]);
        assert!(guide.contains("<html lang=\"en\">"));
        assert!(guide.contains("История"));
        let service = String::from_utf8_lossy(&files["services/orders.html"]);
        assert!(service.contains("href=\"../catalog.html\""));
        assert!(service.contains("aria-label=\"Документация\""));
    }

    #[test]
    fn catalog_tracks_publications_but_preserves_user_edits() {
        let root = tempfile::tempdir().unwrap();
        Repository::init(root.path(), "Docs").unwrap();
        let repo = Repository::open(root.path()).unwrap();
        for bundle in ["first", "second"] {
            repo.atomic(
                &format!("docs/generated/{bundle}/services/orders.html"),
                b"<script id=\"document-data\">{\"title\":\"Orders\"}</script>",
            )
            .unwrap();
            connect_starters(&repo, bundle, &["services/orders.html".into()]).unwrap();
            let bytes = std::fs::read(root.path().join("docs/catalog.html")).unwrap();
            assert!(owned_catalog(&bytes));
            assert!(
                String::from_utf8(bytes)
                    .unwrap()
                    .contains(&format!("generated/{bundle}/services/orders.html"))
            );
        }
        let path = root.path().join("docs/catalog.html");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("Find a service or process", "Our team catalog");
        std::fs::write(&path, &edited).unwrap();
        connect_starters(&repo, "third", &[]).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
    }
    #[test]
    fn new_repository_has_offline_guides_without_fake_publication() {
        let root = tempfile::tempdir().unwrap();
        Repository::init(root.path(), "Fresh docs").unwrap();
        for name in ["help.html", "runbooks.html"] {
            let html = std::fs::read_to_string(root.path().join(format!("docs/{name}"))).unwrap();
            assert!(html.contains("aria-label=\"Documentation\""));
            assert!(html.contains("data:image/svg+xml,"));
            assert!(!html.contains("href=\"index.html\""));
        }
        assert!(!root.path().join("docs/index.html").exists());
        let repo = Repository::open(root.path()).unwrap();
        connect_starters(&repo, "snapshot", &["services/orders.html".into()]).unwrap();
        let connected = std::fs::read_to_string(root.path().join("docs/help.html")).unwrap();
        assert!(connected.contains("href=\"index.html\""));
        assert!(connected.contains("generated/snapshot/catalog.html"));
        assert!(!connected.contains("generated/snapshot/services/orders.html"));

        std::fs::write(root.path().join("docs/help.html"), "My existing guide").unwrap();
        Repository::init(root.path(), "Fresh docs").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("docs/help.html")).unwrap(),
            "My existing guide"
        );
    }
    #[test]
    fn every_published_page_links_with_correct_relative_depth() {
        let mut files = BTreeMap::new();
        for name in [
            "services/orders.html",
            "scenarios/reservation.html",
            "overview.html",
            "root-overview.html",
        ] {
            files.insert(name.to_owned(), page("Page", "<h1>Page</h1>").into_bytes());
        }
        decorate_bundle(&mut files, "snapshot").unwrap();
        for (name, data) in &files {
            let html = std::str::from_utf8(data).unwrap();
            assert!(html.contains("aria-label=\"Documentation\""), "{name}");
            assert!(html.contains("data:image/svg+xml,"), "{name}");
            let prefix = if name == "root-overview.html" {
                "generated/snapshot/"
            } else if name.contains('/') {
                "../"
            } else {
                ""
            };
            for target in ["help.html", "runbooks.html", "catalog.html"] {
                assert!(
                    html.contains(&format!("href=\"{prefix}{target}\"")),
                    "{name}: {target}"
                );
            }
        }
    }
    #[test]
    fn large_catalog_keeps_navigation_bounded_and_preserves_titles() {
        let mut files = BTreeMap::new();
        for n in 0..500 {
            let data = serde_json::json!({"title": format!("Service {n}"), "view": null});
            files.insert(
                format!("services/service-{n}.html"),
                format!("<script id=\"document-data\">{data}</script>").into_bytes(),
            );
        }
        files.insert(
            "scenarios/quantity.html".into(),
            b"<script id=\"document-data\">{\"title\":\"Quantity flow\",\"view\":{}}</script>"
                .to_vec(),
        );
        let pages = files.keys().cloned().collect::<Vec<_>>();
        let nav = navigation("", "overview.html", Some("../../history.html"), &pages);
        assert_eq!(nav.matches("<a ").count(), 6);
        assert!(!nav.contains("services/service-"));
        let html = catalog(&files);
        let data = html
            .split("id=\"catalog-data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(data).unwrap();
        assert_eq!(rows.len(), 501);
        assert!(
            rows.iter()
                .any(|r| r["title"] == "Quantity flow" && r["kind"] == "Dataflow")
        );
        assert!(rows.iter().any(|r| r["title"] == "Service 499"));
    }
}
