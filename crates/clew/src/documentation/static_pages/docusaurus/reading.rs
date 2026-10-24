//! Reading projections reuse retained declarations and exact source bindings.
use super::{IMPORTS, details, fence, header, md, rows, source_link, text};
use crate::documentation::io_error;
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn status(page: &Value) -> String {
    format!(
        "<ClewStatus freshness={} verification={} />\n\n",
        json!(
            page["sectionState"]["freshness"]
                .as_str()
                .unwrap_or("UNVERIFIED")
        ),
        json!(
            page["sectionState"]["verification"]
                .as_str()
                .unwrap_or("UNASSESSED")
        )
    )
}
pub(super) fn process_contracts(page: &Value, anchor: &str) -> String {
    let mut seen = BTreeSet::new();
    let mut out = String::new();
    for contract in page["interfaceContractDeclarations"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter_map(Value::as_object)
        .flat_map(|m| m.values())
    {
        let id = text(&contract["id"]);
        if id.is_empty() || !seen.insert(id) {
            continue;
        }
        out.push_str(&format!("<ClewContextLink path={} fromProcess={} fromQuestion={}>Declared contract: {}</ClewContextLink>\n\n",
            json!(format!("/contracts/{id}")), json!(text(&page["subject"]).strip_prefix("scenario:").unwrap_or("")),
            json!(anchor), md(text(&contract["title"]))));
    }
    out
}

pub(super) fn step_models(page: &Value, anchor: &str) -> String {
    let data = &page["processDataFlow"];
    if data["request"]["anchor"] != anchor {
        return String::new();
    }
    let models = data
        .get("relatedModels")
        .filter(|v| v.is_array())
        .unwrap_or(&data["models"]);
    let links = rows(models)
        .map(|model| {
            format!(
                "<ClewModelLink model={} process={} anchor={}>{}</ClewModelLink>",
                json!(text(&model["id"])),
                json!(
                    text(&page["subject"])
                        .strip_prefix("scenario:")
                        .unwrap_or("")
                ),
                json!(anchor),
                md(text(&model["title"]))
            )
        })
        .collect::<Vec<_>>();
    if links.is_empty() {
        return String::new();
    }
    format!(
        "<p className=\"clew-step-data\">Data handled by this step: {}</p>\n\n",
        links.join(" · ")
    )
}
/// Match the native reader's single-backtick spans without admitting authored JSX.
pub(super) fn inline(value: &str) -> String {
    let mut remaining = value;
    let mut out = String::new();
    while let Some(start) = remaining.find('`') {
        out.push_str(&md(&remaining[..start]));
        if remaining[..start].ends_with('\n') {
            out.push('\n');
        }
        let after = &remaining[start + 1..];
        if after.starts_with('`') {
            let count = remaining[start..]
                .bytes()
                .take_while(|b| *b == b'`')
                .count();
            out.push_str(&md(&remaining[start..start + count]));
            remaining = &remaining[start + count..];
            continue;
        }
        if let Some(end) = after.find('`')
            && end > 0
            && !after[..end].contains(['\n', '\r'])
            && !after[end + 1..].starts_with('`')
        {
            let literal = json!(&after[..end]).to_string().replace('|', "\\u007c");
            out.push_str(&format!("<code>{{{literal}}}</code>"));
            remaining = &after[end + 1..];
        } else {
            out.push_str("\\`");
            remaining = after;
        }
    }
    out.push_str(&md(remaining));
    out
}
pub(super) fn note_title(note: &Value) -> &str {
    note["assessment"]["title"]
        .as_str()
        .unwrap_or(text(&note["association"]["title"]))
}
fn observation(note: &Value) -> Option<Value> {
    let raw = note["original"]["text"].as_str()?;
    if raw.len() > 65536 {
        return None;
    }
    let v: Value = serde_json::from_str(raw).ok()?;
    let revision = v["sourceCommit"]
        .as_str()
        .or_else(|| v["repositoryRevision"].as_str())?;
    if v["schema"] != "clew-weather-runtime-observation/1.0"
        || v["origin"] != "LOCAL_TEST_OBSERVATION"
        || v["compilerEvidence"] != false
        || !v["id"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 160)
        || revision.len() != 40
        || !revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !matches!(text(&v["status"]), "PASS" | "PASSED" | "FAIL" | "FAILED")
    {
        return None;
    }
    if v["cases"].as_array().is_some_and(|cases| {
        cases.len() <= 100
            && cases.iter().all(|c| {
                c["case"].as_str().is_some_and(|s| s.len() <= 160) && c["pass"].is_boolean()
            })
    }) && v["startedAt"].as_str().is_some_and(|s| s.len() <= 80)
    {
        return Some(json!({"kind":"replay","revision":revision,"record":v}));
    }
    let node = text(&v["nodeVersion"]);
    if node.len() <= 32
        && node.strip_prefix('v').is_some_and(|s| {
            let parts = s.split('.').collect::<Vec<_>>();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        && v["exerciseResult"]["runtime"] == v["nodeVersion"]
        && v["exerciseResult"]["assertions"]
            .as_u64()
            .is_some_and(|n| n <= 10000)
    {
        return Some(json!({"kind":"node","revision":revision,"record":v}));
    }
    None
}
pub(super) fn note_page(page: &Value, service: &str, note: &Value) -> Result<String, ClewError> {
    let mut out = header(
        note_title(note),
        &format!("/notes/{service}--{}", text(&note["id"])),
    );
    out.push_str(IMPORTS);
    out.push_str(&format!("# {}\n\n", md(note_title(note))));
    out.push_str("<ClewPageContext />\n\n");
    out.push_str(&super::operation_language_notice(
        page,
        &note["assessment"],
    )?);
    out.push_str(&format!(
        "Original record: **{}** · {}\n\n",
        md(text(&note["association"]["title"])),
        md(text(&note["association"]["classification"]))
    ));
    if let Some(observed) = observation(note) {
        let v = &observed["record"];
        out.push_str(&format!("## Historical runtime observation\n\n**Reported result: {}** · Repository revision `{}`\n\n", md(text(&v["status"])), text(&observed["revision"])));
        if observed["kind"] == "node" {
            out.push_str(&format!(
                "{} assertions recorded · Node version: **{}**\n\n",
                v["exerciseResult"]["assertions"],
                md(text(&v["nodeVersion"]))
            ));
        } else {
            let cases = rows(&v["cases"]).collect::<Vec<_>>();
            out.push_str(&format!(
                "{} of {} replay cases reported passing · Recorded at {}\n\n",
                cases.iter().filter(|c| c["pass"] == true).count(),
                cases.len(),
                md(text(&v["startedAt"]))
            ));
            let table = String::from("| Case | Recorded result |\n| --- | --- |\n")
                + &cases
                    .iter()
                    .map(|c| {
                        format!(
                            "| {} | {} |\n",
                            md(text(&c["case"])),
                            if c["pass"] == true {
                                "Passed"
                            } else {
                                "Failed"
                            }
                        )
                    })
                    .collect::<String>();
            out.push_str(&details("Recorded replay cases", &table));
        }
        out.push_str("Imported, unverified record. This page does not report a current execution, compiler verification, or a general runtime guarantee.\n\n");
    }
    out.push_str("## Separate agent assessment\n\n");
    let assessment = &note["assessment"];
    if assessment["assessment"].is_object() {
        let a = &assessment["assessment"];
        let matches = a["noteDigest"] == note["original"]["digest"]
            && a["associationDigest"] == note["associationDigest"]
            && note["targetChanged"] != true;
        out.push_str(&format!("**Assessment conclusion: {}** · Assessed label: {}\n\nInput binding: {} · Meaning review: {} · Freshness: {}\n\n{}\n\n", md(text(&a["outcome"])), md(text(&a["period"])), if matches { "matches displayed capture" } else { "STALE" }, md(page["operationStates"][text(&note["id"])]["verification"].as_str().unwrap_or("UNASSESSED")), md(page["operationStates"][text(&note["id"])]["freshness"].as_str().unwrap_or("UNVERIFIED")), inline(text(&assessment["summary"]["text"]))));
        out.push_str(&sources(
            page,
            rows(&assessment["summary"]["sourceIds"]).map(text),
        )?);
        if a["proposedCorrection"].is_object() {
            out.push_str(&format!(
                "### Proposed correction · original unchanged\n\n{}\n\n",
                inline(text(&a["proposedCorrection"]["text"]))
            ));
            out.push_str(&sources(
                page,
                rows(&a["proposedCorrection"]["sourceIds"]).map(text),
            )?);
        }
    } else {
        out.push_str("No assessment has been accepted for this note on this page.\n\n");
    }
    out.push_str("Classification and original text remain human/imported declarations. Evidence tracking covers captured inputs; it cannot establish every implicit claim in arbitrary prose.\n\n");
    out.push_str(&details(
        &format!("Original record · {}", text(&note["association"]["title"])),
        &format!(
            "{}\n\nCaptured text digest: `{}`\n\n{}",
            md(text(&note["snapshotLabel"])),
            md(text(&note["original"]["digest"])),
            fence(
                note["original"]["text"]
                    .as_str()
                    .or_else(|| note["original"]["reason"].as_str())
                    .unwrap_or(text(&note["original"]["status"])),
                "text"
            )
        ),
    ));
    out.push_str(&details("Original metadata and associations", &fence(&serde_json::to_string_pretty(&json!({"association":note["association"],"associationDigest":note["associationDigest"],"authority":note["authority"],"missingTargets":note["missingTargets"]})).map_err(io_error)?, "json")));
    out.push_str(&format!(
        "[Return to service context](../services/{service}.mdx#{})\n",
        text(&note["id"])
    ));
    Ok(out)
}
fn sources<'a>(page: &Value, ids: impl Iterator<Item = &'a str>) -> Result<String, ClewError> {
    let mut out = String::new();
    for id in ids.collect::<BTreeSet<_>>() {
        if let Some(source) = page["sources"].get(id) {
            out.push_str(&format!("- {}\n", source_link(source)?));
        }
    }
    out.push('\n');
    Ok(out)
}
fn topics(page: &Value) -> impl Iterator<Item = &Value> {
    rows(&page["interactions"]).filter(|i| {
        i["declaration"]["transport"]["kind"] == "kafka"
            && i["declaration"]["transport"]["topic"].is_string()
    })
}
fn matching_step<'a>(view: &'a Value, i: &Value, side: &str) -> Option<&'a Value> {
    let topic = text(&i["declaration"]["transport"]["topic"]);
    rows(&view["steps"]).find(|s| {
        rows(&s["blocks"]).any(|b| {
            text(&b["text"]).contains(topic)
                && (rows(&b["sourceIds"]).any(|id| rows(&i[side]["sourceIds"]).any(|n| n == id))
                    || rows(&b["dependencyIds"]).any(|id| {
                        rows(&i[side]["candidates"])
                            .chain(rows(&i["callSite"]["candidates"]).filter(|_| side == "from"))
                            .any(|n| n == id)
                    }))
        })
    })
}
fn direction(view: &Value, i: &Value) -> Option<&'static str> {
    if !view["root"].is_object() || !rows(&view["interactions"]).any(|id| id == &i["id"]) {
        return None;
    }
    for (side, direction) in [("from", "outgoing"), ("to", "incoming")] {
        let root = &view["root"];
        let endpoint = &i["declaration"][side];
        let same = root["service"] == endpoint["service"]
            && root["selector"].is_object()
            && endpoint["selector"].is_object()
            && ["language", "name", "owner"]
                .iter()
                .all(|k| root["selector"][*k] == endpoint["selector"][*k])
            && rows(&root["selector"]["parameterTypes"])
                .eq(rows(&endpoint["selector"]["parameterTypes"]));
        if same || root["service"] == endpoint["service"] && matching_step(view, i, side).is_some()
        {
            return Some(direction);
        }
    }
    None
}
fn link(path: &str, label: &str, page: &Value, question: Option<&str>, query: Value) -> String {
    let process = text(&page["subject"])
        .strip_prefix("scenario:")
        .unwrap_or("");
    format!(
        "<ClewContextLink path={} fromProcess={} fromQuestion={} query={{{}}}>{}</ClewContextLink>",
        json!(path),
        json!(process),
        json!(
            question
                .map(|q| format!("question-{q}"))
                .unwrap_or_default()
        ),
        query,
        md(label)
    )
}
fn cards(
    page: &Value,
    context: &Value,
    selected: Vec<&Value>,
    role: &str,
    question: Option<&str>,
) -> Result<String, ClewError> {
    let mut groups = BTreeMap::<String, Vec<&Value>>::new();
    for i in selected {
        let d = &i["declaration"];
        groups
            .entry(format!(
                "{}|{}|{}",
                text(&d["transport"]["topic"]),
                text(&d["from"]["service"]),
                text(&d["from"]["selector"]["name"])
            ))
            .or_default()
            .push(i);
    }
    let mut out = String::new();
    for group in groups.values() {
        let i = group[0];
        let topic = text(&i["declaration"]["transport"]["topic"]);
        let heading = match role {
            "incoming" => "Incoming Kafka topic",
            "outgoing" => "Outgoing Kafka topic",
            _ => "Related asynchronous continuation",
        };
        out.push_str(&format!("<article className=\"clew-topic\" id={} tabIndex={{-1}}>\n\n### {heading} · {}\n\n<ClewProcessReturn topic={} direction={} />\n\n<div className=\"clew-topic-path\" aria-label=\"Declared asynchronous handoff\">\n\n", json!(format!("topic-{role}-{topic}")), md(topic), json!(topic), json!(role)));
        for (index, side) in ["from", "to"].iter().enumerate() {
            if index == 1 {
                out.push_str(&format!("<span aria-hidden=\"true\">→</span>\n\n<div>\n\n**{}**\n\nKafka topic · asynchronous\n\n</div>\n\n<span aria-hidden=\"true\">→</span>\n\n", md(topic)));
            }
            out.push_str("<div>\n\n");
            let mut seen = BTreeSet::new();
            for row in if index == 0 { vec![i] } else { group.clone() } {
                let endpoint = &row["declaration"][*side];
                let service = text(&endpoint["service"]);
                let name = text(&endpoint["selector"]["name"]);
                if !seen.insert((service, name)) {
                    continue;
                }
                let external = index == 1 && row["declaration"]["external"] == true;
                let registered = external
                    && row["declaration"]["externalService"] == service
                    && page["externalServices"][service].is_object();
                let label = if registered {
                    page["externalServices"][service]["title"]
                        .as_str()
                        .unwrap_or(service)
                } else {
                    context["serviceLabels"][service]
                        .as_str()
                        .unwrap_or(service)
                };
                if external {
                    if registered {
                        out.push_str(&link(
                            &format!("/external-services/{service}"),
                            label,
                            page,
                            question,
                            json!({}),
                        ));
                    } else {
                        out.push_str(&md(label));
                    }
                    out.push_str("\n\nSource not captured; external execution is unverified.\n\n");
                } else {
                    let flow = if index == 0 { "outgoing" } else { "incoming" };
                    let path = format!(
                        "/services/{service}#section-{}",
                        if index == 0 { "egress" } else { "ingress" }
                    );
                    out.push_str(&link(
                        &path,
                        label,
                        page,
                        question,
                        json!({"topic":topic,"direction":flow}),
                    ));
                }
                out.push_str(&format!("\n\n{}\n\n", md(name)));
            }
            out.push_str("</div>\n\n");
        }
        out.push_str(&format!(
            "</div>\n\n{}; runtime delivery has not been verified.\n\n",
            if i["origin"] == "agent-proposal" {
                "Proposed handoff"
            } else {
                "Declared handoff"
            }
        ));
        let mut seen = BTreeSet::new();
        for view in rows(&context["processLinks"]) {
            if view["id"] == page["process"]["definition"]["id"] {
                continue;
            }
            let role = group.iter().find_map(|i| direction(view, i));
            if let Some(role) = role
                && seen.insert(text(&view["id"]))
            {
                out.push_str(&format!(
                    "Saved {} view: {}\n\n",
                    if role == "outgoing" {
                        "publisher"
                    } else {
                        "consumer"
                    },
                    link(
                        &format!("/scenarios/{}#process-overview", text(&view["id"])),
                        text(&view["title"]),
                        page,
                        question,
                        json!({})
                    )
                ));
            }
        }
        for note in rows(&page["notes"]).filter(|n| {
            n["assessment"].is_object()
                && group
                    .iter()
                    .any(|i| i["declaration"]["to"]["service"] == n["association"]["service"])
        }) {
            out.push_str(&format!(
                "Related service assessment: {}\n\n",
                link(
                    &format!(
                        "/notes/{}--{}",
                        text(&note["association"]["service"]),
                        text(&note["id"])
                    ),
                    note_title(note),
                    page,
                    question,
                    json!({})
                )
            ));
        }
        let ids = group
            .iter()
            .flat_map(|i| {
                ["from", "to", "callSite"]
                    .into_iter()
                    .flat_map(|s| rows(&i[s]["sourceIds"]).map(text))
            })
            .chain(
                page["sources"]
                    .as_object()
                    .into_iter()
                    .flat_map(|s| s.iter())
                    .filter(|(_, s)| text(&s["text"]).contains(topic))
                    .take(12)
                    .map(|(id, _)| id.as_str()),
            );
        let mut evidence = String::from(
            "Topic names and the payload's event type are separate values. These references retain their own revisions; source matches do not prove topic resolution or delivery.\n\n",
        );
        evidence.push_str(&sources(page, ids)?);
        evidence.push_str(&fence(&serde_json::to_string_pretty(&group.iter().map(|i| json!({"interaction":i["id"],"origin":i["origin"],"from":i["declaration"]["from"],"to":i["declaration"]["to"],"topic":i["topic"],"runtime":i["runtime"],"rationale":i["declaration"]["declaration"]["rationale"],"boundaries":i["boundaries"]})).collect::<Vec<_>>()).map_err(io_error)?, "json"));
        out.push_str(&details(
            "Retained source and declaration context",
            &evidence,
        ));
        out.push_str("</article>\n\n");
    }
    Ok(out)
}
pub(super) fn service_topics(
    page: &Value,
    context: &Value,
    incoming: bool,
) -> Result<String, ClewError> {
    let side = if incoming { "to" } else { "from" };
    let service = text(&page["subject"])
        .strip_prefix("service:")
        .unwrap_or("");
    cards(
        page,
        context,
        topics(page)
            .filter(|i| i["declaration"][side]["service"] == service)
            .collect(),
        if incoming { "incoming" } else { "outgoing" },
        None,
    )
}
pub(super) fn process_topics(
    page: &Value,
    context: &Value,
    section: Option<&str>,
) -> Result<String, ClewError> {
    let Some(view) =
        rows(&context["processLinks"]).find(|v| v["id"] == page["process"]["definition"]["id"])
    else {
        return Ok(String::new());
    };
    let mut out = String::new();
    if let Some(section) = section {
        for role in ["incoming", "outgoing"] {
            let selected = topics(page)
                .filter(|i| {
                    direction(view, i) == Some(role)
                        && matching_step(view, i, if role == "incoming" { "to" } else { "from" })
                            .or_else(|| rows(&view["steps"]).next())
                            .is_some_and(|s| s["id"] == section)
                })
                .collect();
            out.push_str(&cards(page, context, selected, role, Some(section))?);
        }
    } else {
        let selected = topics(page)
            .filter(|i| {
                rows(&view["interactions"]).any(|id| id == &i["id"]) && direction(view, i).is_none()
            })
            .collect::<Vec<_>>();
        if !selected.is_empty() {
            out.push_str("## Related asynchronous continuations {#question-async-continuations}\n\nThese declared handoffs belong to the linked continuation views. They are not additional publishing steps in the local account above.\n\n");
            out.push_str(&cards(
                page,
                context,
                selected,
                "related",
                Some("async-continuations"),
            )?);
        }
    }
    Ok(out)
}

fn main_html(html: &str) -> Option<String> {
    let start = html.find("<main")?;
    let start = start + html[start..].find('>')? + 1;
    let end = start + html[start..].find("</main>")?;
    let mut body = html[start..end].to_owned();
    for tag in ["script", "style"] {
        while let Some(start) = body.find(&format!("<{tag}")) {
            let end = start + body[start..].find(&format!("</{tag}>"))? + tag.len() + 3;
            body.replace_range(start..end, "");
        }
    }
    Some(body)
}
pub(super) fn ancillary_pages(
    native: &BTreeMap<String, Vec<u8>>,
    files: &mut BTreeMap<String, Vec<u8>>,
    sidebars: &mut Vec<Value>,
    language: &str,
) -> Result<(), ClewError> {
    use crate::documentation::language::label;
    let mut items = Vec::new();
    for (file, slug, title) in [
        ("help.html", "help", "Help"),
        ("runbooks.html", "runbooks", "Runbooks"),
        ("history.html", "history", "Publication history"),
        (
            "retained-overview.html",
            "retained-overview",
            "Retained publication overview",
        ),
        ("data-model.html", "data-model", "Data model"),
    ] {
        let Some(bytes) = native.get(file) else {
            continue;
        };
        let html = std::str::from_utf8(bytes).map_err(io_error)?;
        let Some(body) = main_html(html) else {
            continue;
        };
        let mut out = header(label(language, title), &format!("/{slug}"));
        out.push_str(IMPORTS);
        out.push_str(&format!(
            "<ClewRetainedPage nativePath={} html={{{}}} />\n",
            json!(file),
            json!(body)
        ));
        files.insert(format!("docs/{slug}.mdx"), out.into_bytes());
        if slug == "data-model" {
            if let Some(category) = sidebars
                .iter_mut()
                .find(|c| c["label"] == label(language, "Data model"))
            {
                category["items"]
                    .as_array_mut()
                    .expect("generated sidebar items")
                    .insert(0, json!(slug));
            } else {
                sidebars.push(
                    json!({"type":"category","label":label(language, "Data model"),"items":[slug]}),
                );
            }
        } else {
            items.push(json!(slug));
        }
    }
    if let Some(bytes) = native.get("catalog.html") {
        let html = std::str::from_utf8(bytes).map_err(io_error)?;
        if let Some(at) = html.find("id=\"catalog-data\"") {
            let start =
                at + html[at..].find('>').ok_or_else(|| {
                    crate::documentation::invalid("catalog data opening is missing")
                })? + 1;
            let end = start
                + html[start..].find("</script>").ok_or_else(|| {
                    crate::documentation::invalid("catalog data closing is missing")
                })?;
            let entries: Value = serde_json::from_str(&html[start..end]).map_err(io_error)?;
            let mut out = header(label(language, "Search documentation"), "/catalog");
            out.push_str(IMPORTS);
            out.push_str(&format!("# {} {{#search-documentation}}\n\n{}\n\n<ClewCatalog entries={{{entries}}} />\n", md(label(language, "Search documentation")), md(label(language, "Catalog metadata helps discovery; it does not establish that explanations are complete."))));
            files.insert("docs/catalog.mdx".into(), out.into_bytes());
            items.push(json!("catalog"));
        }
    }
    if !items.is_empty() {
        sidebars.push(json!({"type":"category","label":label(language, "Reader help and reference"),"items":items}));
    }
    Ok(())
}

pub(super) fn route_map(
    native: &BTreeMap<String, Vec<u8>>,
    pages: &[(String, Value)],
) -> Result<Value, ClewError> {
    let mut routes = serde_json::Map::new();
    for name in native.keys().filter(|n| n.ends_with(".html")) {
        let path = if matches!(name.as_str(), "index.html" | "overview.html") {
            "/".to_owned()
        } else {
            format!("/{}", name.trim_end_matches(".html"))
        };
        routes.insert(name.clone(), json!(path));
    }
    for (name, page) in pages {
        let name = name.trim_start_matches("native/").trim_end_matches(".json");
        for entry in rows(&page["catalogue"]) {
            if serde_json::from_value::<crate::documentation::model::Entrypoint>(entry.clone())
                .is_ok_and(|e| crate::documentation::process_candidates::public_boundary(&e))
            {
                routes.insert(
                    format!("{name}.html#{}", text(&entry["id"])),
                    json!(format!("/{name}#entry-{}", text(&entry["id"]))),
                );
            } else if let Some(source) = rows(&entry["sourceIds"])
                .filter_map(|id| page["sources"].get(text(id)))
                .find(|source| super::is_source(source))
            {
                let identity = crate::documentation::digest(source)?;
                routes.insert(
                    format!("{name}.html#{}", text(&entry["id"])),
                    json!(format!("/sources/{}", &identity[7..])),
                );
            }
        }
        for op in rows(&page["operations"]) {
            let id = text(&op["id"]);
            if id == "process-overview" || id.starts_with("section-") {
                routes.insert(format!("{name}.html#{id}"), json!(format!("/{name}#{id}")));
                continue;
            }
            routes.insert(
                format!("{name}.html#{id}"),
                json!(format!("/operations/{}--{id}", name.replace('/', "-"))),
            );
        }
        for note in rows(&page["notes"]) {
            let id = text(&note["id"]);
            let service = text(&note["association"]["service"]);
            routes.insert(
                format!("{name}.html#{id}"),
                json!(format!("/notes/{service}--{id}")),
            );
        }
    }
    Ok(Value::Object(routes))
}

pub(super) fn linked_downloads(
    native: &BTreeMap<String, Vec<u8>>,
    files: &mut BTreeMap<String, Vec<u8>>,
    routes: &mut Value,
) -> Result<(), ClewError> {
    use std::path::{Component, Path};
    let mut linked = BTreeSet::new();
    for (name, bytes) in native.iter().filter(|(name, _)| name.ends_with(".html")) {
        let html = std::str::from_utf8(bytes).map_err(io_error)?;
        for remainder in html.split("href=\"").skip(1) {
            let href = remainder
                .split('"')
                .next()
                .unwrap_or("")
                .split(['?', '#'])
                .next()
                .unwrap_or("");
            if !href.ends_with(".json") || href.contains(':') || href.starts_with("//") {
                continue;
            }
            let path = if let Some(root) = href.strip_prefix('/') {
                Path::new(root).to_path_buf()
            } else {
                Path::new(name).parent().unwrap_or(Path::new("")).join(href)
            };
            let mut parts = Vec::new();
            let mut valid = true;
            for component in path.components() {
                match component {
                    Component::Normal(part) => parts.push(part),
                    Component::ParentDir => {
                        if parts.pop().is_none() {
                            valid = false;
                            break;
                        }
                    }
                    Component::CurDir => {}
                    _ => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                continue;
            }
            let name = parts
                .iter()
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if native.contains_key(&name) {
                linked.insert(name);
            }
        }
    }
    for name in linked {
        let target = format!("clew-data/downloads/{name}");
        files.insert(format!("static/{target}"), native[&name].clone());
        routes[&name] = json!(format!("/{target}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn step_data_links_follow_the_explicit_anchor_and_existing_model_selection() {
        let page = json!({"subject":"scenario:save","processDataFlow":{"request":{"anchor":"question-request"},"models":[{"id":"other","title":"Other"}],"relatedModels":[{"id":"request","title":"Save city request"},{"id":"place","title":"Place"}]}});
        assert!(step_models(&page, "question-validation").is_empty());
        let rendered = step_models(&page, "question-request");
        assert!(rendered.contains("Data handled by this step:"));
        assert_eq!(rendered.matches("<ClewModelLink ").count(), 2);
        assert!(!rendered.contains("Other"));
        assert_eq!(rendered.matches("anchor=\"question-request\"").count(), 2);
        assert_eq!(rendered.matches("process=\"save\"").count(), 2);
    }
    #[test]
    fn inline_tokens_are_literal_react_children_and_cannot_execute_mdx() {
        let rendered = inline("Send `weather.place-requested`; retry `PUBLISH_RETRY`.");
        assert!(rendered.contains("<code>{\"weather.place-requested\"}</code>"));
        assert!(!rendered.contains('`'));
        let payload = "<script>{execute()}|</script>";
        let rendered = inline(&format!(
            "Before `{payload}` after <script>{{unsafe()}}</script>"
        ));
        assert!(rendered.contains("<code>{\"<script>{execute()}\\u007c</script>\"}</code>"));
        assert!(rendered.contains("&lt;script&gt;&#123;unsafe()&#125;&lt;/script&gt;"));
        assert_eq!(
            inline("```text\nnot inline\n```"),
            md("```text\nnot inline\n```")
        );
    }
    #[test]
    fn only_bounded_recognized_observations_receive_runtime_projection() {
        let raw = json!({"schema":"clew-weather-runtime-observation/1.0","origin":"LOCAL_TEST_OBSERVATION","compilerEvidence":false,"id":"runtime","status":"PASS","sourceCommit":"a".repeat(40),"nodeVersion":"v24.21.0","exerciseResult":{"runtime":"v24.21.0","assertions":6}});
        let note = json!({"id":"note","association":{"title":"record.json","classification":"IMPORTED"},"original":{"text":raw.to_string(),"digest":"original"},"assessment":{"title":"Readable title","summary":{"text":"Every original assessment paragraph."},"assessment":{"outcome":"CONTRADICTION","period":"v24.21.0"}}});
        let body = note_page(&json!({}), "service", &note).unwrap();
        assert!(body.contains("# Readable title"));
        assert!(body.contains("6 assertions recorded"));
        assert!(body.contains("Every original assessment paragraph."));
        assert!(body.contains("Historical runtime observation"));
        assert!(body.contains(&raw.to_string()));
        let mut unknown = note.clone();
        unknown["original"]["text"] = json!(r#"{"status":"PASS","assertions":6}"#);
        let body = note_page(&json!({}), "service", &unknown).unwrap();
        assert!(!body.contains("Historical runtime observation"));
        assert!(body.contains("Original record"));
        let mut oversized = note.clone();
        oversized["original"]["text"] = json!(" ".repeat(65537));
        assert!(observation(&oversized).is_none());
    }
    #[test]
    fn external_kafka_recipient_links_to_registry_page_and_never_invents_ingress() {
        let i = json!({"id":"event","origin":"human","declaration":{"external":true,"externalService":"provider","transport":{"kind":"kafka","topic":"requests"},"from":{"service":"owned"},"to":{"service":"provider"}},"runtime":"UNKNOWN"});
        let mut page = json!({"subject":"service:owned","externalServices":{"provider":{"title":"External provider"}}});
        let body = cards(&page, &json!({}), vec![&i], "outgoing", None).unwrap();
        assert!(body.contains("/external-services/provider"));
        assert!(!body.contains("/services/provider"));
        assert!(body.contains("Declared handoff"));
        page["externalServices"] = json!({});
        let legacy = cards(&page, &json!({}), vec![&i], "outgoing", None).unwrap();
        assert!(!legacy.contains("/services/provider"));
        assert!(!legacy.contains("/external-services/provider"));
        assert!(legacy.contains("Source not captured"));
    }

    #[test]
    fn local_http_roots_do_not_become_publishers_and_source_bound_wrappers_do() {
        let i = json!({"id":"event","declaration":{"transport":{"kind":"kafka","topic":"weather.requested"},"from":{"service":"places","selector":{"name":"publish","language":"csharp","owner":"Publisher"}},"to":{"service":"dashboard","selector":{"name":"consume"}}},"from":{"sourceIds":["send"]},"topic":{"status":"UNRESOLVED"},"runtime":"UNKNOWN"});
        let publisher = json!({"id":"publisher","root":{"service":"places","selector":{"name":"execute","language":"csharp","owner":"Publisher"}},"interactions":["event"],"steps":[{"id":"send","blocks":[{"text":"Send weather.requested","sourceIds":["send"]}]}]});
        let local = json!({"id":"save","root":{"service":"dashboard","selector":{"name":"save"}},"interactions":["event"],"steps":[{"id":"save","blocks":[{"text":"Save the request","sourceIds":["http"]}]}]});
        assert_eq!(direction(&publisher, &i), Some("outgoing"));
        assert_eq!(direction(&local, &i), None);
        let page = json!({"subject":"scenario:save","process":{"definition":{"id":"save"}},"interactions":[i]});
        let context = json!({"processLinks":[local,publisher]});
        assert!(
            process_topics(&page, &context, Some("save"))
                .unwrap()
                .is_empty()
        );
        let body = process_topics(&page, &context, None).unwrap();
        assert!(body.contains("Related asynchronous continuations"));
        assert!(body.contains("UNRESOLVED"));
        assert!(body.contains("UNKNOWN"));
        assert!(body.contains("/scenarios/publisher#process-overview"));
        assert!(body.contains("fromQuestion=\"question-async-continuations\""));
        assert!(
            body.contains(
                "<ClewProcessReturn topic=\"weather.requested\" direction=\"related\" />"
            )
        );
    }
    #[test]
    fn ancillary_titles_and_categories_use_the_locale_without_changing_routes_or_retained_html() {
        let original = "<main><h1>Source fragments</h1><p>Revision</p></main>";
        let native = BTreeMap::from([
            ("help.html".into(), original.as_bytes().to_vec()),
            ("data-model.html".into(), original.as_bytes().to_vec()),
            (
                "catalog.html".into(),
                b"<script id=\"catalog-data\">[]</script>".to_vec(),
            ),
        ]);
        let mut routes = Vec::new();
        for language in ["en", "ru"] {
            let model_label = crate::documentation::language::label(language, "Data model");
            let mut sidebar =
                vec![json!({"type":"category","label":model_label,"items":["data-model/record"]})];
            let mut files = BTreeMap::new();
            ancillary_pages(&native, &mut files, &mut sidebar, language).unwrap();
            assert_eq!(
                sidebar
                    .iter()
                    .filter(|category| category["label"] == model_label)
                    .count(),
                1
            );
            assert_eq!(
                sidebar[0]["items"],
                json!(["data-model", "data-model/record"])
            );
            let body = String::from_utf8(files["docs/help.mdx"].clone()).unwrap();
            assert!(body.contains("<h1>Source fragments</h1><p>Revision</p>"));
            let catalog = String::from_utf8(files["docs/catalog.mdx"].clone()).unwrap();
            assert!(catalog.contains("{#search-documentation}"));
            assert!(catalog.contains("<ClewCatalog entries={[]} />"));
            if language == "ru" {
                assert_ne!(sidebar[0]["label"], "Data model");
                assert!(!catalog.contains("# Search documentation"));
            }
            routes.push(files.keys().cloned().collect::<Vec<_>>());
        }
        assert_eq!(routes[0], routes[1]);
    }

    #[test]
    fn ancillary_projection_drops_shell_scripts_but_retains_content_and_routes() {
        let html = b"<main><h1>Original heading</h1><p>All retained prose.</p><details><summary>Long/pointer</summary>Original scope</details><script>not portable</script><style>old shell</style></main>".to_vec();
        let native = BTreeMap::from([
            ("help.html".into(), html),
            (
                "data-model.html".into(),
                b"<main><h1>Models</h1></main>".to_vec(),
            ),
            ("services/s.html".into(), Vec::new()),
        ]);
        let mut files = BTreeMap::new();
        let mut sidebar = vec![];
        ancillary_pages(&native, &mut files, &mut sidebar, "en").unwrap();
        let body = String::from_utf8(files["docs/help.mdx"].clone()).unwrap();
        assert!(body.contains("All retained prose."));
        assert!(body.contains("Original scope"));
        assert!(!body.contains("<script>"));
        assert!(!body.contains("old shell"));
        assert!(body.contains("hide_title: true"));
        assert!(
            sidebar
                .iter()
                .any(|c| c["label"] == "Data model" && c["items"][0] == "data-model")
        );
        let pages = vec![(
            "native/services/s.json".into(),
            json!({"operations":[{"id":"op"},{"id":"section-ingress"},{"id":"process-overview"}],"notes":[{"id":"note","association":{"service":"s"}}]}),
        )];
        let routes = route_map(&native, &pages).unwrap();
        assert_eq!(routes["services/s.html#op"], "/operations/services-s--op");
        assert_eq!(routes["services/s.html#note"], "/notes/s--note");
        assert_eq!(
            routes["services/s.html#section-ingress"],
            "/services/s#section-ingress"
        );
        assert_eq!(
            routes["services/s.html#process-overview"],
            "/services/s#process-overview"
        );
    }
    #[test]
    fn retained_downloads_keep_exact_bytes_and_source_only_catalogue_links_resolve() {
        let bytes = b"{\r\n  \"literal\": 1.0\r\n}\r\n".to_vec();
        let native = BTreeMap::from([
            ("data-model.json".into(), bytes.clone()),
            (
                "data-model.html".into(),
                b"<main><a href=\"data-model.json\" download>Original JSON</a></main>".to_vec(),
            ),
        ]);
        let source = json!({"file":"Example.kt","revision":"a".repeat(40),"startLine":1,"endLine":1,"text":"fun main() {}","textDigest":"retained"});
        let pages = vec![(
            "native/services/s.json".into(),
            json!({"catalogue":[{"id":"source-only","sourceIds":["source"]}],"sources":{"source":source}}),
        )];
        let mut routes = route_map(&native, &pages).unwrap();
        let mut files = BTreeMap::new();
        linked_downloads(&native, &mut files, &mut routes).unwrap();
        assert_eq!(files["static/clew-data/downloads/data-model.json"], bytes);
        assert_eq!(
            routes["data-model.json"],
            "/clew-data/downloads/data-model.json"
        );
        assert_eq!(
            routes["services/s.html#source-only"],
            format!(
                "/sources/{}",
                &crate::documentation::digest(&source).unwrap()[7..]
            )
        );
    }
}
