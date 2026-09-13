//! Offline navigation and product guides shared by every new publication.
use super::{io_error, render, store::Repository};
use crate::error::ClewError;
use std::collections::BTreeMap;

pub(super) const HELP: &str = include_str!("../../assets/documentation/help.html");
pub(super) const RUNBOOKS: &str = include_str!("../../assets/documentation/runbooks.html");
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
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body><main class=\"reader-guide\">{body}</main></body></html>",
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
    let mut links = format!(
        "<a class=\"reader-brand\" href=\"{}\">Codeclew</a><a href=\"{prefix}help.html\">Help</a><a href=\"{prefix}runbooks.html\">Runbooks</a>",
        render::escape(overview)
    );
    if let Some(history) = history {
        links.push_str(&format!(
            "<a href=\"{}\">History</a>",
            render::escape(history)
        ));
    }
    for path in pages {
        if (path.starts_with("services/") || path.starts_with("scenarios/"))
            && path.ends_with(".html")
        {
            let label = path.trim_end_matches(".html").replace('/', " / ");
            links.push_str(&format!(
                "<a href=\"{}{}\">{}</a>",
                render::escape(prefix),
                render::escape(path),
                render::escape(&label)
            ));
        }
    }
    format!(
        "<nav class=\"reader-nav\" aria-label=\"Documentation\">{links}<a href=\"https://codeclew.github.io/codeclew/\">Codeclew site ↗</a></nav>"
    )
}

pub(super) fn decorate(html: &str, navigation: &str) -> String {
    html.replacen("</head>", &format!("{}</head>", favicon()), 1)
        .replacen("<body>", &format!("<body>{navigation}"), 1)
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
    Ok(())
}

/// Upgrade only byte-identical starter output; preserve any user's guide edits.
pub(super) fn connect_starters(
    repo: &Repository,
    bundle: &str,
    pages: &[String],
) -> Result<(), ClewError> {
    let starter_nav = navigation("", "help.html", None, &[]);
    let nav = navigation(
        &format!("generated/{bundle}/"),
        "index.html",
        Some("history.html"),
        pages,
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
    Ok(())
}

pub(super) fn decorate_bundle(
    files: &mut BTreeMap<String, Vec<u8>>,
    bundle: &str,
) -> Result<(), ClewError> {
    guides(files);
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
        let nav = navigation(&prefix, &overview, Some(history), &pages);
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
        assert!(connected.contains("generated/snapshot/services/orders.html"));

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
            for target in [
                "help.html",
                "runbooks.html",
                "services/orders.html",
                "scenarios/reservation.html",
            ] {
                assert!(
                    html.contains(&format!("href=\"{prefix}{target}\"")),
                    "{name}: {target}"
                );
            }
        }
    }
}
