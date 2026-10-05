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
    let search = label(
        "Search documentation metadata",
        "Поиск по метаданным документации",
    );
    let placeholder = label(
        "Search services, APIs, handlers and entities…",
        "Найти сервис, API, обработчик или сущность…",
    );
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

fn document_payload(html: &str) -> Option<serde_json::Value> {
    html.split("id=\"document-data\">")
        .nth(1)
        .and_then(|s| s.split("</script>").next())
        .and_then(|s| serde_json::from_str(s).ok())
}

fn string_values(value: &serde_json::Value) -> Vec<String> {
    fn collect(value: &serde_json::Value, values: &mut BTreeMap<String, ()>) {
        match value {
            serde_json::Value::String(text) if !text.is_empty() => {
                values.insert(text.clone(), ());
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect(item, values);
                }
            }
            serde_json::Value::Object(items) => {
                for item in items.values() {
                    collect(item, values);
                }
            }
            _ => {}
        }
    }
    let mut values = BTreeMap::new();
    collect(value, &mut values);
    values.into_keys().collect()
}

fn source_callable_label(symbol: &str) -> Option<String> {
    let identity = symbol.strip_prefix("source:")?;
    let (declaration, digest) = identity.rsplit_once('/')?;
    if digest.len() != 20
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let (scope, name) = declaration.rsplit_once('/')?;
    let (file, owner) = scope.rsplit_once('/')?;
    let class_scoped = owner.starts_with("class:");
    let owner = owner
        .strip_prefix("package:")
        .or_else(|| owner.strip_prefix("class:"))?;
    if file.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    let name = if class_scoped {
        format!("{}.{name}", owner.rsplit('.').next()?)
    } else {
        name.to_owned()
    };
    Some(format!("{name}() · {}", file.rsplit('/').next()?))
}

fn entrypoint_trigger_metadata(
    trigger: &serde_json::Value,
    language: &str,
) -> (Vec<String>, Vec<String>) {
    let mut terms = BTreeMap::new();
    let mut groups: BTreeMap<&str, BTreeMap<String, ()>> = BTreeMap::new();
    for (key, group) in [
        ("topic", "topic"),
        ("topics", "topic"),
        ("destination", "destination"),
        ("channel", "destination"),
        ("groupId", "consumer group"),
        ("cron", "schedule"),
        ("schedule", "schedule"),
        ("fixedDelay", "schedule"),
        ("fixedDelayString", "schedule"),
        ("fixedRate", "schedule"),
        ("fixedRateString", "schedule"),
        ("command", "command"),
    ] {
        for source in [trigger, &trigger["configuration"]] {
            for value in string_values(&source[key]) {
                terms.insert(value.clone(), ());
                groups.entry(group).or_default().insert(value, ());
            }
        }
    }
    if terms.is_empty()
        && let Some(value) = trigger.as_str().filter(|value| !value.trim().is_empty())
    {
        terms.insert(value.to_owned(), ());
    }
    let mut display: Vec<String> = groups
        .into_iter()
        .map(|(group, values)| {
            let group = match group {
                "topic" => text(language, "Topic", "Тема"),
                "destination" => text(language, "Destination", "Назначение"),
                "consumer group" => text(language, "Consumer group", "Группа потребителей"),
                "schedule" => text(language, "Schedule", "Расписание"),
                "command" => text(language, "Command", "Команда"),
                _ => group,
            };
            format!(
                "{group}: {}",
                values.into_keys().collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    if display.is_empty()
        && let Some(value) = trigger.as_str().filter(|value| !value.trim().is_empty())
    {
        display.push(value.to_owned());
    }
    (terms.into_keys().collect(), display)
}

fn search_terms(row: &mut serde_json::Value, terms: impl IntoIterator<Item = String>) {
    let mut values: BTreeMap<String, ()> = row["searchText"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(|term| (term.to_owned(), ()))
        .collect();
    values.extend(
        terms
            .into_iter()
            .filter(|term| !term.is_empty())
            .map(|v| (v, ())),
    );
    row["searchText"] = serde_json::json!(values.into_keys().collect::<Vec<_>>());
}

fn coverage_presence(payload: &serde_json::Value, id: &str) -> &'static str {
    if payload["translationGaps"].get(id).is_some() || payload["gaps"].get(id).is_some() {
        "unavailable"
    } else if payload["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|operation| {
            operation["id"] == id
                && operation["summary"]["text"]
                    .as_str()
                    .is_some_and(|text| !text.trim().is_empty())
        })
    {
        "authored"
    } else {
        "inventory"
    }
}

pub(super) fn catalog_language(files: &BTreeMap<String, Vec<u8>>, language: &str) -> String {
    let mut documents = BTreeMap::new();
    let mut pages = BTreeMap::new();
    for (path, data) in files.iter().filter(|(path, _)| path.ends_with(".html")) {
        let kind = if path.starts_with("services/") {
            "Service"
        } else if path.starts_with("scenarios/") {
            "Process"
        } else {
            continue;
        };
        let html = String::from_utf8_lossy(data);
        let payload = document_payload(&html);
        let id = path
            .split('/')
            .next_back()
            .unwrap_or(path)
            .trim_end_matches(".html");
        let title = payload
            .as_ref()
            .and_then(|value| value["title"].as_str())
            .unwrap_or(id);
        let kind = if kind == "Process"
            && payload
                .as_ref()
                .is_some_and(|value| !value["view"].is_null())
        {
            "Dataflow"
        } else {
            kind
        };
        let mut row = serde_json::json!({
            "id":id,"title":title,"kind":kind,"href":path,"searchText":[]
        });
        let mut terms = vec![id.to_owned(), title.to_owned()];
        if let Some(payload) = payload.as_ref() {
            for process in payload["savedProcesses"].as_array().into_iter().flatten() {
                for key in ["id", "title", "trigger"] {
                    terms.extend(string_values(&process[key]));
                }
            }
            let process_definition = &payload["process"]["definition"];
            for key in ["id", "title", "root"] {
                terms.extend(string_values(&process_definition[key]));
            }
            let process = &process_definition["process"];
            for key in ["scope", "participants", "objects", "trigger", "outcomes"] {
                terms.extend(string_values(&process[key]));
            }
            let view = &payload["view"]["definition"]["view"];
            for key in [
                "scope",
                "inputObjects",
                "services",
                "contracts",
                "relatedProcesses",
            ] {
                terms.extend(string_values(&view[key]));
            }
        }
        search_terms(&mut row, terms);
        pages.insert(path.clone(), row.clone());
        if let Some(payload) = payload {
            documents.insert(path.clone(), payload);
        }
    }

    let service_titles: BTreeMap<_, _> = documents
        .iter()
        .filter(|(path, _)| path.starts_with("services/"))
        .map(|(path, payload)| {
            (
                path.trim_start_matches("services/")
                    .trim_end_matches(".html")
                    .to_owned(),
                payload["title"].as_str().unwrap_or(path).to_owned(),
            )
        })
        .collect();
    let mut api_rows: BTreeMap<(String, String), serde_json::Value> = BTreeMap::new();
    let mut entity_rows: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    for (path, payload) in documents
        .iter()
        .filter(|(path, _)| path.starts_with("services/"))
    {
        let service_id = path
            .trim_start_matches("services/")
            .trim_end_matches(".html");
        let context = payload["title"].as_str().unwrap_or(service_id);
        let entries: Vec<_> = payload["catalogue"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let entry_ids: BTreeMap<_, _> = entries
            .iter()
            .filter_map(|entry| entry["id"].as_str().map(|id| (id.to_owned(), ())))
            .collect();
        let mut linked_contracts: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for entry in entries {
            let Some(id) = entry["id"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            let symbol = entry["symbol"].as_str().unwrap_or(id);
            let trigger = &entry["trigger"];
            let mut methods = string_values(&trigger["methods"]);
            let mut paths = string_values(&trigger["paths"]);
            methods.sort();
            paths.sort();
            let inferred_http = methods.iter().any(|method| {
                matches!(
                    method.to_ascii_uppercase().as_str(),
                    "GET" | "PUT" | "POST" | "DELETE" | "OPTIONS" | "HEAD" | "PATCH" | "TRACE"
                )
            }) || !paths.is_empty();
            let http = entry["kind"] == "HTTP_ENDPOINT" || entry["kind"].is_null() && inferred_http;
            let source_declaration = entry["kind"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("SOURCE_"));
            let source_label = source_declaration
                .then(|| source_callable_label(symbol))
                .flatten();
            let coverage = coverage_presence(payload, id);
            let authored_title = (coverage == "authored")
                .then(|| {
                    payload["operations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|operation| operation["id"] == id)
                })
                .flatten()
                .and_then(|operation| operation["title"].as_str())
                .filter(|title| !title.trim().is_empty());
            let display_title = source_label
                .as_deref()
                .map(|label| authored_title.unwrap_or(label));
            let route_known = paths.iter().any(|path| !path.trim().is_empty());
            let title = if http && route_known && methods.len() == 1 && paths.len() == 1 {
                format!("{} {}", methods[0], paths[0])
            } else if http && route_known && methods.is_empty() {
                paths.join(", ")
            } else if http && route_known {
                format!("{} {}", methods.join(", "), paths.join(", "))
                    .trim()
                    .to_owned()
            } else {
                symbol.to_owned()
            };
            let summary = if http {
                let methods = if methods.is_empty() {
                    text(language, "unresolved", "не определены").to_owned()
                } else {
                    methods.join(", ")
                };
                let paths = if route_known {
                    paths.join(", ")
                } else {
                    text(language, "unresolved", "не определены").to_owned()
                };
                format!(
                    "{}: {} · {}: {} · {}: {}",
                    text(language, "Methods", "Методы"),
                    methods,
                    text(language, "Paths", "Пути"),
                    paths,
                    text(language, "Handler", "Обработчик"),
                    symbol
                )
            } else if let Some(label) = source_label.as_deref() {
                format!(
                    "{}: {}",
                    text(language, "Source declaration", "Объявление в исходном коде"),
                    label
                )
            } else {
                let (_, trigger_parts) = entrypoint_trigger_metadata(trigger, language);
                if trigger_parts.is_empty() {
                    format!("{}: {}", text(language, "Handler", "Обработчик"), symbol)
                } else {
                    format!(
                        "{}: {} · {}: {}",
                        text(language, "Trigger", "Событие запуска"),
                        trigger_parts.join(" · "),
                        text(language, "Handler", "Обработчик"),
                        symbol
                    )
                }
            };
            let mut terms = if http {
                let mut values = methods.clone();
                values.extend(paths.clone());
                values
            } else {
                entrypoint_trigger_metadata(trigger, language).0
            };
            terms.extend([
                id.to_owned(),
                symbol.to_owned(),
                title.clone(),
                service_id.to_owned(),
                context.to_owned(),
            ]);
            if let Some(display_title) = display_title {
                terms.push(display_title.to_owned());
            }
            let mut row = serde_json::json!({
                "id":id,"title":title,"kind":if http {"API"} else if source_declaration {"Source declaration"} else {"Entrypoint"},
                "href":format!("{path}#{id}"),"context":context,"summary":summary,
                "coverage":coverage,"searchText":[]
            });
            if let Some(display_title) = display_title {
                row["displayTitle"] = serde_json::json!(display_title);
            }
            search_terms(&mut row, terms);
            api_rows.insert((service_id.to_owned(), id.to_owned()), row);
        }

        for contract in payload["contracts"].as_array().into_iter().flatten() {
            let normalized = &contract["normalized"];
            let method = normalized["method"].as_str().unwrap_or("");
            let route = normalized["path"].as_str().unwrap_or("");
            let entrypoint = normalized["entrypoint"].as_str();
            let operation_id = normalized["operation"]["operationId"]
                .as_str()
                .unwrap_or("");
            let terms = [service_id, context, method, route, operation_id]
                .into_iter()
                .filter(|term| !term.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if let Some(entrypoint) = entrypoint.filter(|id| entry_ids.contains_key(*id)) {
                linked_contracts
                    .entry(entrypoint.to_owned())
                    .or_default()
                    .extend(terms);
                continue;
            }
            let mapping = normalized["sourceMapping"].as_str().unwrap_or("");
            let title = format!("{method} {route}").trim().to_owned();
            let key = (service_id.to_owned(), format!("contract:{method}:{route}"));
            let mut row = serde_json::json!({
                "id":contract["id"],"title":title,"kind":"API",
                "href":format!("{path}#section-ingress"),"context":context,
                "summary":format!("{}: {mapping}",text(language,"Source mapping","Связь с исходным кодом")),
                "coverage":"unavailable","searchText":[]
            });
            let mut terms = terms;
            terms.extend(string_values(&contract["id"]));
            terms.extend(string_values(&normalized["declaredSource"]));
            terms.extend(string_values(&normalized["sourceMapping"]));
            search_terms(&mut row, terms);
            api_rows.entry(key).or_insert(row);
        }
        for ((row_service, entry_id), row) in api_rows.iter_mut() {
            if row_service == service_id
                && let Some(terms) = linked_contracts.get(entry_id)
            {
                search_terms(row, terms.clone());
            }
        }

        for process in payload["savedProcesses"].as_array().into_iter().flatten() {
            let Some(id) = process["id"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            let process_path = format!("scenarios/{id}.html");
            if let Some(row) = pages
                .get_mut(&process_path)
                .filter(|row| row["kind"] == "Process")
            {
                row["href"] = serde_json::json!(format!("{process_path}#process-overview"));
                let mut terms = string_values(&process["trigger"]);
                terms.extend([id.to_owned()]);
                if let Some(title) = process["title"].as_str() {
                    terms.push(title.to_owned());
                }
                search_terms(row, terms);
                row["coverage"] = serde_json::json!(
                    documents
                        .get(&process_path)
                        .map(|payload| coverage_presence(payload, "process-overview"))
                        .unwrap_or("unavailable")
                );
                row["summary"] = serde_json::json!(format!(
                    "{}: {}",
                    text(language, "Trigger", "Событие запуска"),
                    process["trigger"].as_str().unwrap_or("")
                ));
            }
        }

        for entity in payload["entities"].as_array().into_iter().flatten() {
            if entity["kind"] != "DOMAIN_ENTITY" {
                continue;
            }
            let definition = &entity["normalized"]["entity"];
            let Some(id) = definition["id"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            if entity_rows.contains_key(id) {
                continue;
            }
            let title = definition["title"].as_str().unwrap_or(id);
            let service_ids = definition["relations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|relation| relation["service"].as_str().map(|id| (id.to_owned(), ())))
                .collect::<BTreeMap<_, ()>>();
            let link_service = service_ids
                .keys()
                .map(String::as_str)
                .find(|service| files.contains_key(&format!("services/{service}.html")))
                .unwrap_or(service_id);
            let mut terms = vec![id.to_owned(), title.to_owned()];
            let mut representations = BTreeMap::new();
            for relation in definition["relations"].as_array().into_iter().flatten() {
                for key in ["service", "kind", "representations"] {
                    terms.extend(string_values(&relation[key]));
                }
                for representation in string_values(&relation["representations"]) {
                    representations.insert(representation, ());
                }
            }
            terms.extend(string_values(&definition["relatedEntities"]));
            let unavailable = entity["normalized"]["missingDependencies"]
                .as_array()
                .is_some_and(|missing| !missing.is_empty());
            let has_description = definition["description"]
                .as_str()
                .is_some_and(|description| !description.trim().is_empty());
            let mut row = serde_json::json!({
                "id":id,"title":title,"kind":"Entity",
                "href":format!("services/{link_service}.html#section-entities"),
                "context":service_ids.keys().filter_map(|service|service_titles.get(service)).cloned().collect::<Vec<_>>().join(", "),
                "summary":representations.into_keys().collect::<Vec<_>>(),
                "coverage":if unavailable {"unavailable"} else if has_description {"authored"} else {"inventory"},
                "searchText":[]
            });
            search_terms(&mut row, terms);
            entity_rows.insert(id.to_owned(), row);
        }
    }

    let mut rows: Vec<_> = pages.into_values().collect();
    rows.extend(api_rows.into_values());
    rows.extend(entity_rows.into_values());
    for (_, bytes) in files
        .iter()
        .filter(|(p, _)| p.starts_with("answers/") && p.ends_with(".publication.json"))
    {
        let Ok(entry) = serde_json::from_slice::<super::reviewed_answers::PublishedAnswer>(bytes)
        else {
            continue;
        };
        if super::reviewed_answers::validate_entries(&BTreeMap::from([(
            entry.id.clone(),
            entry.clone(),
        )]))
        .is_err()
        {
            continue;
        }
        rows.push(serde_json::json!({"id":entry.id,"title":entry.title,"kind":"Reviewed answer","href":entry.route(),"context":entry.source_snapshot,"summary":"MODEL REVIEW: APPROVED · saved snapshot; current source and runtime not verified","searchText":[entry.work,entry.review_run,entry.title]}));
    }
    rows.sort_by(|left, right| {
        (
            left["kind"].as_str().unwrap_or(""),
            left["title"].as_str().unwrap_or(""),
            left["id"].as_str().unwrap_or(""),
            left["href"].as_str().unwrap_or(""),
        )
            .cmp(&(
                right["kind"].as_str().unwrap_or(""),
                right["title"].as_str().unwrap_or(""),
                right["id"].as_str().unwrap_or(""),
                right["href"].as_str().unwrap_or(""),
            ))
    });
    let has_reviewed_answers = rows.iter().any(|r| r["kind"] == "Reviewed answer");
    let has_source_declarations = rows.iter().any(|r| r["kind"] == "Source declaration");
    let payload = serde_json::to_string(&rows)
        .expect("catalog strings serialize")
        .replace('<', "\\u003c");
    let catalog = if language == "ru" {
        page_language("Каталог документации", &format!(r#"<div class="eyebrow">КАТАЛОГ ДОКУМЕНТАЦИИ</div><h1>Поиск по документации</h1><p>Ищите сервисы, маршруты API, обработчики, процессы и сущности по названию или идентификатору. Откройте результат, чтобы посмотреть разделы, операции и подтверждения.</p><p class="catalog-caveat">Метаданные каталога помогают искать сведения, но не подтверждают полноту описания.</p><div class="catalog-controls"><input id="catalog-query" type="search" aria-label="Найти документы" placeholder="Сервис, API, обработчик или сущность"><select id="catalog-kind" aria-label="Тип документа"><option value="">Все типы</option><option value="Service">Сервис</option><option value="Process">Процесс</option><option value="Dataflow">Движение данных</option><option value="API">API</option><option value="Entrypoint">Точка входа</option><option value="Entity">Сущность</option></select></div><p id="catalog-status" class="catalog-status" role="status" aria-live="polite"></p><ul id="catalog-results" class="catalog-results"></ul><div class="catalog-pager"><button id="catalog-prev" type="button">Назад</button><button id="catalog-next" type="button">Далее</button></div><noscript><p>Для поиска в локальном каталоге включите JavaScript или откройте обзор через меню.</p></noscript><script id="catalog-data" type="application/json">{payload}</script>"#), language).replace("class=\"reader-guide\"", "class=\"reader-guide catalog-page\"")
    } else {
        page("Browse documentation", &format!(r#"<div class="eyebrow">DOCUMENTATION CATALOG</div><h1>Search documentation</h1><p>Search services, API routes, handlers, processes and entities by name or identifier. Open a result to browse its sections, operations and evidence.</p><p class="catalog-caveat">Catalog metadata helps discovery; it does not establish that explanations are complete.</p><div class="catalog-controls"><input id="catalog-query" type="search" aria-label="Find documentation metadata" placeholder="Service, API, handler or entity"><select id="catalog-kind" aria-label="Document type"><option value="">All types</option><option>Service</option><option>Process</option><option>Dataflow</option><option>API</option><option>Entrypoint</option><option>Entity</option></select></div><p id="catalog-status" class="catalog-status" role="status" aria-live="polite"></p><ul id="catalog-results" class="catalog-results"></ul><div class="catalog-pager"><button id="catalog-prev" type="button">Previous</button><button id="catalog-next" type="button">Next</button></div><noscript><p>Enable JavaScript to search this offline catalog, or open the overview from the navigation.</p></noscript><script id="catalog-data" type="application/json">{payload}</script>"#)).replace("class=\"reader-guide\"", "class=\"reader-guide catalog-page\"")
    };
    let mut catalog = catalog;
    if has_source_declarations {
        let label = text(language, "Source declaration", "Объявление в исходном коде");
        catalog = catalog.replacen(
            "</select>",
            &format!("<option value=\"Source declaration\">{label}</option></select>"),
            1,
        );
    }
    if has_reviewed_answers {
        let option = if language == "ru" {
            "<option value=\"Reviewed answer\">Ответ с проверкой модели</option>"
        } else {
            "<option>Reviewed answer</option>"
        };
        catalog.replacen("</select>", &format!("{option}</select>"), 1)
    } else {
        catalog
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
                owned_html("guide", &decorate(&page(title, body), &nav)).as_bytes(),
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
    is_owned_html("catalog", bytes)
}

fn owned_html(kind: &str, html: &str) -> String {
    format!(
        "<!-- codeclew-{kind} {} -->\n{html}",
        crate::canonical::hash_bytes(html.as_bytes())
    )
}

fn is_owned_html(kind: &str, bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Some((marker, body)) = text.split_once('\n') else {
        return false;
    };
    marker
        == format!(
            "<!-- codeclew-{kind} {} -->",
            crate::canonical::hash_bytes(body.as_bytes())
        )
}

// Older connected guides have no ownership marker. Recognize only exact
// generated output using current assets and one of the supported navigations.
fn unedited_legacy_guide(bytes: &[u8], title: &str, body: &str) -> bool {
    let Ok(html) = std::str::from_utf8(bytes) else {
        return false;
    };
    let page = page(title, body);
    for overview in ["help.html", "index.html"] {
        if html == decorate(&page, &navigation("", overview, None, &[])) {
            return true;
        }
    }
    let Some(bundle) = html
        .split_once("href=\"generated/")
        .and_then(|(_, suffix)| suffix.split_once('/').map(|(bundle, _)| bundle))
    else {
        return false;
    };
    for language in ["en", "ru"] {
        let nav = navigation_language(
            &format!("generated/{bundle}/"),
            "index.html",
            Some("history.html"),
            &[],
            language,
        );
        if html == decorate(&page, &nav) {
            return true;
        }
    }
    false
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
        let existing = std::fs::read(repo.path(&path)?).ok();
        if existing.as_deref().is_some_and(|bytes| {
            is_owned_html("guide", bytes) || unedited_legacy_guide(bytes, title, body)
        }) {
            repo.atomic(
                &path,
                owned_html("guide", &decorate(&page(title, body), &nav)).as_bytes(),
            )?;
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
        for path in pages.iter().filter(|p| {
            p.ends_with(".html") || (p.starts_with("answers/") && p.ends_with(".publication.json"))
        }) {
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
                "\"href\":\"answers/",
                &format!("\"href\":\"generated/{bundle}/answers/"),
            )
            .replace(
                "\"href\":\"scenarios/",
                &format!("\"href\":\"generated/{bundle}/scenarios/"),
            );
        let html = decorate(&body, &nav);
        let owned = owned_html("catalog", &html);
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
    fn source_declaration_catalog_uses_readable_labels_and_retains_exact_identities() {
        let symbol =
            "source:workers/Main.kt/package:example.worker/processRequest/114e9ee8cc58ecb97b3e";
        let class_symbol = "source:workers/Main.kt/class:example.worker.Nested/optionalString/e0efee2144d2ca813dd2";
        let unrecognized = "source:workers/Main.kt/unknown:example/handler/114e9ee8cc58ecb97b3e";
        let payload = serde_json::json!({
            "title":"Worker", "subject":"service:worker",
            "catalogue":[
                {"id":"pending","symbol":symbol,"kind":"SOURCE_DECLARATION"},
                {"id":"authored","symbol":class_symbol,"kind":"SOURCE_DECLARATION"},
                {"id":"translation-gap","symbol":symbol,"kind":"SOURCE_DECLARATION"},
                {"id":"unrecognized","symbol":unrecognized,"kind":"SOURCE_DECLARATION"}
            ],
            "operations":[
                {"id":"authored","title":"Read the optional string","summary":{"text":"Accepted explanation."}},
                {"id":"translation-gap","title":"Unavailable translated title","summary":{"text":"Prior language."}}
            ],
            "translationGaps":{"translation-gap":{"availableLanguage":"en"}}
        });
        let files = BTreeMap::from([(
            "services/worker.html".into(),
            format!("<script id=\"document-data\">{payload}</script>").into_bytes(),
        )]);
        for language in ["en", "ru"] {
            let html = catalog_language(&files, language);
            let encoded = html
                .split("id=\"catalog-data\" type=\"application/json\">")
                .nth(1)
                .unwrap()
                .split("</script>")
                .next()
                .unwrap();
            let rows: Vec<serde_json::Value> = serde_json::from_str(encoded).unwrap();
            let find = |id: &str| rows.iter().find(|row| row["id"] == id).unwrap();
            let pending = find("pending");
            assert_eq!(pending["displayTitle"], "processRequest() · Main.kt");
            assert_eq!(pending["title"], symbol);
            assert_eq!(pending["kind"], "Source declaration");
            assert_eq!(pending["href"], "services/worker.html#pending");
            assert!(
                pending["searchText"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!(symbol))
            );
            assert!(!pending["summary"].as_str().unwrap().contains(symbol));
            assert!(
                pending["summary"]
                    .as_str()
                    .unwrap()
                    .contains("processRequest() · Main.kt")
            );
            assert_eq!(find("authored")["displayTitle"], "Read the optional string");
            assert_eq!(find("authored")["title"], class_symbol);
            assert!(
                find("authored")["searchText"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("Read the optional string"))
            );
            assert_eq!(
                find("translation-gap")["displayTitle"],
                "processRequest() · Main.kt"
            );
            assert_eq!(find("unrecognized")["title"], unrecognized);
            assert!(find("unrecognized").get("displayTitle").is_none());
            assert!(html.contains("value=\"Source declaration\""));
            assert!(html.contains(text(
                language,
                "Source declaration",
                "Объявление в исходном коде"
            )));
        }
        assert_eq!(
            source_callable_label(class_symbol).as_deref(),
            Some("Nested.optionalString() · Main.kt")
        );
        let decode =
            "source:workers/Main.kt/class:example.worker.decodeRequest/string/114e9ee8cc58ecb97b3e";
        let response = "source:workers/Main.kt/class:example.worker.typedResponsePayload/string/114e9ee8cc58ecb97b3e";
        assert_eq!(
            source_callable_label(decode).as_deref(),
            Some("decodeRequest.string() · Main.kt")
        );
        assert_eq!(
            source_callable_label(response).as_deref(),
            Some("typedResponsePayload.string() · Main.kt")
        );
        assert_ne!(
            source_callable_label(decode),
            source_callable_label(response)
        );
        for invalid in [
            unrecognized,
            "source:Main.kt/package:worker/name/not-a-digest",
            "source:Main.kt/package:/name/114e9ee8cc58ecb97b3e",
        ] {
            assert!(source_callable_label(invalid).is_none());
        }
    }

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
        assert!(catalog.contains("Сервис, API, обработчик или сущность"));
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
    fn catalog_indexes_api_entity_and_saved_process_fields_without_duplicates() {
        let entity = serde_json::json!({
            "kind":"DOMAIN_ENTITY",
            "normalized":{"entity":{"id":"order","title":"Order",
                "description":"Do not copy this prose into catalog search.","relatedEntities":[],
                "relations":[{"service":"orders","kind":"owned","representations":["OrderRecord","OrderRecord"]},
                    {"service":"inventory","kind":"read","representations":["StockView"]}]},
                "missingDependencies":["missing-evidence"]}
        });
        let linked_contract = serde_json::json!({
            "id":"contract-post-order","normalized":{"entrypoint":"entry-post",
                "method":"POST","path":"/orders","sourceMapping":"SOURCE_ROUTE_MATCH_ONLY",
                "operation":{"operationId":"createOrder"}}
        });
        let orphan_contract = serde_json::json!({
            "id":"contract-get-legacy","normalized":{"entrypoint":null,"method":"GET",
                "path":"/legacy","sourceMapping":"NO_SOURCE_ROUTE_MATCH",
                "operation":{"operationId":"legacyLookup"}}
        });
        let service = serde_json::json!({
            "title":"Orders API","subject":"service:orders",
            "catalogue":[
                {"id":"entry-post","symbol":"OrdersController.createOrder","kind":"HTTP_ENDPOINT",
                    "trigger":{"methods":["POST","POST"],"paths":["/orders","/orders"]}},
                {"id":"entry-lookup","symbol":"OrdersController.lookup","kind":"HTTP_ENDPOINT",
                    "trigger":{"methods":["GET"],"paths":["/lookup"]}},
                {"id":"entry-gap","symbol":"OrdersController.gap","kind":"HTTP_ENDPOINT",
                    "trigger":{"methods":["GET"],"paths":["/gap"]}},
                {"id":"entry-unresolved","symbol":"OrdersController.unresolved","kind":"HTTP_ENDPOINT",
                    "trigger":{"methods":null,"paths":null,"pathResolution":"REQUIRES_RUNTIME_OR_PATH_PATTERN_RESOLUTION"}},
                {"id":"entry-topic","symbol":"OrderListener.handle","kind":"KAFKA_LISTENER",
                    "trigger":{"configuration":{"topics":["orders.created"],"groupId":"order-workers"},"authority":"SPRING_ANNOTATION_RULES"}},
                {"id":"entry-destination","symbol":"OrdersConsumer.receive","kind":"JMS_LISTENER",
                    "trigger":{"configuration":{"destination":"orders.in"}}}
            ],
            "contracts":[linked_contract.clone(),linked_contract,orphan_contract.clone(),orphan_contract],
            "operations":[{"id":"entry-post","summary":{"text":"Creates an order."}}],
            "gaps":{"entry-gap":"No accepted explanation."},
            "translationGaps":{"entry-gap":{"availableLanguage":"en"}},
            "entities":[entity.clone()],
            "savedProcesses":[{"id":"checkout","title":"Checkout","trigger":"Scheduled import","status":"AUTHORED"}]
        });
        let inventory = serde_json::json!({"title":"Inventory","subject":"service:inventory","entities":[entity]});
        let process = serde_json::json!({
            "title":"Checkout","subject":"scenario:checkout",
            "process":{"definition":{"id":"checkout","title":"Checkout",
                "root":{"service":"orders","selector":{"owner":"CheckoutJob","name":"importOrders"}},
                "process":{"scope":"checkout variants","participants":["orders","inventory"],
                    "objects":["entity:order"],"trigger":"Scheduled import","outcomes":["variant A accepted"]}}},
            "operations":[{"id":"process-overview","summary":{"text":"Imports selected orders."}}],
            "translationGaps":{"process-overview":{"availableLanguage":"en"}}
        });
        let files = BTreeMap::from([
            (
                "scenarios/checkout.html".into(),
                format!("<script id=\"document-data\">{process}</script>").into_bytes(),
            ),
            (
                "services/inventory.html".into(),
                format!("<script id=\"document-data\">{inventory}</script>").into_bytes(),
            ),
            (
                "services/legacy.html".into(),
                b"<h1>Legacy without document data</h1>".to_vec(),
            ),
            (
                "services/broken.html".into(),
                b"<script type=\"application/json\" id=\"document-data\">not-json</script>"
                    .to_vec(),
            ),
            (
                "services/orders.html".into(),
                format!("<script id=\"document-data\">{service}</script>").into_bytes(),
            ),
        ]);
        let html = catalog_language(&files, "en");
        let encoded = html
            .split("id=\"catalog-data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(encoded).unwrap();
        let find = |kind: &str, id: &str| {
            rows.iter()
                .find(|row| row["kind"] == kind && row["id"] == id)
                .unwrap()
        };

        let explained = find("API", "entry-post");
        assert_eq!(explained["title"], "POST /orders");
        assert_eq!(explained["href"], "services/orders.html#entry-post");
        assert_eq!(explained["context"], "Orders API");
        assert_eq!(explained["coverage"], "authored");
        assert!(explained["searchText"].to_string().contains("createOrder"));
        let inventory_only = find("API", "entry-lookup");
        assert_eq!(inventory_only["href"], "services/orders.html#entry-lookup");
        assert_eq!(inventory_only["coverage"], "inventory");
        assert_eq!(find("API", "entry-gap")["coverage"], "unavailable");
        let unresolved = find("API", "entry-unresolved");
        assert_eq!(unresolved["title"], "OrdersController.unresolved");
        assert!(
            unresolved["summary"]
                .as_str()
                .unwrap()
                .contains("Paths: unresolved")
        );
        let topic = find("Entrypoint", "entry-topic");
        assert!(
            topic["summary"]
                .as_str()
                .unwrap()
                .contains("orders.created")
        );
        assert!(topic["summary"].as_str().unwrap().contains("order-workers"));
        assert!(
            !topic["summary"]
                .as_str()
                .unwrap()
                .contains("SPRING_ANNOTATION_RULES")
        );
        assert!(topic["searchText"].to_string().contains("orders.created"));
        let destination = find("Entrypoint", "entry-destination");
        assert!(
            destination["summary"]
                .as_str()
                .unwrap()
                .contains("Destination: orders.in")
        );

        let orphan = rows
            .iter()
            .find(|row| row["title"] == "GET /legacy")
            .unwrap();
        assert_eq!(orphan["href"], "services/orders.html#section-ingress");
        assert_eq!(orphan["coverage"], "unavailable");
        assert_eq!(
            rows.iter()
                .filter(|row| row["title"] == "GET /legacy")
                .count(),
            1
        );
        let domain = find("Entity", "order");
        assert!(
            domain["href"]
                .as_str()
                .unwrap()
                .ends_with("#section-entities")
        );
        assert_eq!(domain["coverage"], "unavailable");
        assert!(domain["summary"].to_string().contains("OrderRecord"));
        assert!(domain["searchText"].to_string().contains("StockView"));
        assert!(
            !domain["searchText"]
                .to_string()
                .contains("Do not copy this prose")
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row["kind"] == "Entity" && row["id"] == "order")
                .count(),
            1
        );

        let saved = find("Process", "checkout");
        assert_eq!(saved["href"], "scenarios/checkout.html#process-overview");
        assert_eq!(saved["coverage"], "unavailable");
        assert!(saved["searchText"].to_string().contains("Scheduled import"));
        assert!(saved["searchText"].to_string().contains("CheckoutJob"));
        assert!(
            saved["searchText"]
                .to_string()
                .contains("variant A accepted")
        );
        assert_eq!(find("Service", "legacy")["title"], "legacy");
        assert_eq!(find("Service", "broken")["title"], "broken");
        assert_eq!(catalog_language(&files, "en"), html);
        assert!(html.contains("does not establish that explanations are complete"));
        let russian_html = catalog_language(&files, "ru");
        let russian_rows: Vec<serde_json::Value> = serde_json::from_str(
            russian_html
                .split("id=\"catalog-data\" type=\"application/json\">")
                .nth(1)
                .unwrap()
                .split("</script>")
                .next()
                .unwrap(),
        )
        .unwrap();
        let russian_process = russian_rows
            .iter()
            .find(|row| row["kind"] == "Process" && row["id"] == "checkout")
            .unwrap();
        assert_eq!(russian_process["coverage"], "unavailable");
    }

    #[test]
    fn guide_navigation_tracks_publications_and_preserves_human_edits() {
        let root = tempfile::tempdir().unwrap();
        Repository::init(root.path(), "Docs").unwrap();
        let repo = Repository::open(root.path()).unwrap();
        let legacy_nav = navigation("generated/legacy/", "index.html", Some("history.html"), &[]);
        repo.atomic(
            "docs/help.html",
            decorate(&page("Codeclew help", HELP), &legacy_nav).as_bytes(),
        )
        .unwrap();
        for (bundle, language) in [("first", "en"), ("second", "ru")] {
            connect_starters_language(&repo, bundle, &[], language).unwrap();
            for name in ["help.html", "runbooks.html"] {
                let html = std::fs::read(root.path().join(format!("docs/{name}"))).unwrap();
                assert!(is_owned_html("guide", &html));
                let text = String::from_utf8(html).unwrap();
                assert!(text.contains(&format!("generated/{bundle}/catalog.html")));
                assert!(!text.contains("generated/legacy/"));
            }
        }
        for name in ["help.html", "runbooks.html"] {
            let path = root.path().join(format!("docs/{name}"));
            let edited = std::fs::read_to_string(&path)
                .unwrap()
                .replace("</main>", "<p>Team instructions</p></main>");
            std::fs::write(&path, &edited).unwrap();
            connect_starters(&repo, "third", &[]).unwrap();
            assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
        }
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
            .replace("Search documentation", "Our team catalog");
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
