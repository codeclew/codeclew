//! A portable publication projection. Retained data stays lossless behind
//! content-addressed references; Markdown is a separate reading surface.
use crate::documentation::{digest, invalid, io_error};
use crate::error::ClewError;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
mod contracts;
mod reading;

const REF: &str = "$clewRef";
const LITERAL: &str = "$clewLiteral";

#[derive(Default)]
struct Objects {
    counts: BTreeMap<String, usize>,
    values: BTreeMap<String, Value>,
    sources: BTreeMap<String, Value>,
}

fn is_source(v: &Value) -> bool {
    ["text", "textDigest", "revision", "file"]
        .iter()
        .all(|k| v[*k].is_string())
        && v["startLine"].is_u64()
        && v["endLine"].is_u64()
}

impl Objects {
    fn count(&mut self, v: &Value) -> Result<(), ClewError> {
        if v.is_object() || v.is_array() {
            let bytes = crate::canonical::bytes(v).map_err(io_error)?;
            if is_source(v) || bytes.len() >= 512 {
                *self
                    .counts
                    .entry(crate::canonical::hash_bytes(&bytes))
                    .or_default() += 1;
            }
        }
        if !is_source(v) {
            match v {
                Value::Object(o) => {
                    for value in o.values() {
                        self.count(value)?;
                    }
                }
                Value::Array(a) => {
                    for value in a {
                        self.count(value)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn normalize(&mut self, v: &Value) -> Result<Value, ClewError> {
        if v.is_object() || v.is_array() {
            let id = digest(v)?;
            if is_source(v) || self.counts.get(&id).is_some_and(|n| *n > 1) {
                if !self.values.contains_key(&id) {
                    let body = if is_source(v) {
                        v.clone()
                    } else {
                        self.contents(v)?
                    };
                    self.values.insert(id.clone(), body);
                }
                if is_source(v) {
                    self.sources.insert(id.clone(), v.clone());
                }
                return Ok(json!({REF:id}));
            }
        }
        self.contents(v)
    }

    fn contents(&mut self, v: &Value) -> Result<Value, ClewError> {
        Ok(match v {
            Value::Object(o) => {
                let mut normalized = Map::new();
                for (key, value) in o {
                    normalized.insert(key.clone(), self.normalize(value)?);
                }
                let literal = o.contains_key(REF) || o.contains_key(LITERAL);
                let result = Value::Object(normalized);
                if literal {
                    json!({LITERAL:result})
                } else {
                    result
                }
            }
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|v| self.normalize(v))
                    .collect::<Result<_, _>>()?,
            ),
            _ => v.clone(),
        })
    }

    fn expand(&self, v: &Value, depth: usize) -> Result<Value, ClewError> {
        if depth > 512 {
            return Err(invalid(
                "portable evidence reference depth exceeds its bound",
            ));
        }
        if let Some(o) = v.as_object() {
            if o.len() == 1
                && let Some(id) = o.get(REF).and_then(Value::as_str)
            {
                let stored = self
                    .values
                    .get(id)
                    .ok_or_else(|| invalid("portable evidence object is missing"))?;
                let result = self.expand(stored, depth + 1)?;
                if digest(&result)? != id {
                    return Err(invalid(
                        "portable evidence object identity differs after resolution",
                    ));
                }
                return Ok(result);
            }
            let values = if o.len() == 1 && o.contains_key(LITERAL) {
                o[LITERAL]
                    .as_object()
                    .ok_or_else(|| invalid("portable literal object is malformed"))?
            } else {
                o
            };
            return Ok(Value::Object(
                values
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.expand(v, depth + 1)?)))
                    .collect::<Result<_, ClewError>>()?,
            ));
        }
        Ok(match v {
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|v| self.expand(v, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            _ => v.clone(),
        })
    }
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn md(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '{' => out.push_str("&#123;"),
            '}' => out.push_str("&#125;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '!' | '|' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.lines()
        .map(|line| {
            if line.starts_with("import ") {
                format!("&#105;{}", &line[1..])
            } else if line.starts_with("export ") {
                format!("&#101;{}", &line[1..])
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fence(value: &str, language: &str) -> String {
    let longest = value.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let ticks = "`".repeat(3.max(longest + 1));
    format!("{ticks}{language}\n{value}\n{ticks}\n\n")
}

fn header(title: &str, slug: &str) -> String {
    // Generated documents supply their own H1, including retained HTML pages.
    // A preceding gap notice must not add a second framework-generated title.
    format!(
        "---\ntitle: {}\nslug: {}\nhide_title: true\n---\n\n",
        json!(title),
        json!(slug)
    )
}

fn source_link(source: &Value) -> Result<String, ClewError> {
    source_link_in(source, "../")
}
fn source_link_in(source: &Value, prefix: &str) -> Result<String, ClewError> {
    source_link_in_language(source, prefix, "en")
}
fn source_link_in_language(
    source: &Value,
    prefix: &str,
    language: &str,
) -> Result<String, ClewError> {
    if !is_source(source) {
        return Ok(crate::documentation::language::label(
            language,
            "Source binding unavailable in this projection.",
        )
        .into());
    }
    let id = digest(source)?;
    Ok(format!(
        "[{}:{}–{} · {} {}]({prefix}sources/{}.mdx)",
        md(text(&source["file"])),
        source["startLine"],
        source["endLine"],
        md(crate::documentation::language::label(language, "revision")),
        md(text(&source["revision"])),
        &id[7..]
    ))
}

fn evidence_in(block: &Value, prefix: &str) -> Result<String, ClewError> {
    evidence_in_language(block, prefix, "en")
}
fn evidence_language(block: &Value, language: &str) -> Result<String, ClewError> {
    evidence_in_language(block, "../", language)
}
fn evidence_in_language(block: &Value, prefix: &str, language: &str) -> Result<String, ClewError> {
    let mut out = format!(
        "<details>\n<summary>{}</summary>\n\n",
        crate::documentation::render::escape(crate::documentation::language::label(
            language,
            "Evidence and scope"
        ))
    );
    for e in block["evidence"].as_array().into_iter().flatten() {
        if e["kind"] == "RETAINED_CLAIM" {
            out.push_str(&format!(
                "**{}** · {} · {}\n\n{}\n\n",
                md(text(&e["title"])),
                md(text(&e["state"]["freshness"])),
                md(text(&e["state"]["verification"])),
                reading::inline(text(&e["fragment"]["text"]))
            ));
            for source in e["sources"]
                .as_object()
                .into_iter()
                .flat_map(|v| v.values())
            {
                out.push_str(&format!(
                    "- {}\n",
                    source_link_in_language(source, prefix, language)?
                ));
            }
        } else {
            out.push_str(&format!(
                "- {}\n",
                source_link_in_language(&e["source"], prefix, language)?
            ));
        }
    }
    out.push_str("\n</details>\n\n");
    Ok(out)
}

const IMPORTS: &str = "import {ClewProcess, ClewDataModel, ClewDownload, ClewModelLink, ClewRelatedModelLink, ClewFieldTrace, ClewContextLink, ClewPageContext, ClewProcessReturn, ClewStatus, ClewRetainedPage, ClewCatalog, ClewAnchor} from '@site/src/components/ClewReaders';\n\n";

fn rows(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn details(title: &str, body: &str) -> String {
    format!(
        "<details>\n<summary>{}</summary>\n\n{body}\n</details>\n\n",
        md(title)
    )
}

fn archive_link(name: &str) -> String {
    format!(
        "<ClewDownload path=\"/clew-data/native/{name}\">Saved data and complete provenance (reference format)</ClewDownload>\n\n"
    )
}

fn block(block: &Value, page: &Value, section: &str) -> Result<String, ClewError> {
    block_in(block, page, section, "../")
}
fn block_in(block: &Value, page: &Value, section: &str, prefix: &str) -> Result<String, ClewError> {
    let mut out = format!("{}\n\n", reading::inline(text(&block["text"])));
    let mut links = String::new();
    for link in rows(&block["links"]) {
        let subject = text(&link["subject"]);
        let operation = text(&link["operation"]);
        let anchor = format!("question-{section}");
        let canonical =
            &page["canonicalEntityLinks"][&anchor][subject.strip_prefix("service:").unwrap_or("")];
        if operation == "section-entities" && canonical.is_array() {
            for model in rows(canonical) {
                links.push_str(&format!("<ClewModelLink model={} process={} anchor={}>{} · data model</ClewModelLink>\n\n",
                    json!(text(&model["id"])),json!(text(&page["subject"]).strip_prefix("scenario:").unwrap_or("")),json!(anchor),md(text(&model["title"]))));
            }
        }
        let directory = if subject.starts_with("service:") {
            "services"
        } else {
            "scenarios"
        };
        let id = subject.split_once(':').map(|(_, id)| id).unwrap_or("");
        let label = if canonical.is_array() {
            "Technical context"
        } else {
            text(&link["label"])
        };
        let target = if directory == "services"
            && !operation.starts_with("section-")
            && operation != "service-overview"
        {
            format!("{prefix}operations/services-{id}--{operation}.mdx")
        } else if directory == "scenarios"
            && operation != "process-overview"
            && operation != id
            && !operation.starts_with("question-")
        {
            format!("{prefix}operations/scenarios-{id}--{operation}.mdx")
        } else {
            format!("{prefix}{directory}/{id}.mdx#{operation}")
        };
        if let Some(process) = text(&page["subject"]).strip_prefix("scenario:") {
            let path = target.trim_start_matches("../").replace(".mdx", "");
            links.push_str(&format!(
                "<ClewContextLink path={} fromProcess={} fromQuestion={}>{}</ClewContextLink>\n\n",
                json!(format!("/{path}")),
                json!(process),
                json!(anchor),
                md(label)
            ));
        } else {
            links.push_str(&format!("[{}]({target})\n\n", md(label)));
        }
    }
    if !links.is_empty() {
        out.push_str(&format!(
            "<div className=\"clew-context-links\">\n\n{links}</div>\n\n"
        ));
    }
    out.push_str(&evidence_in(block, prefix)?);
    if block["disclosure"] == true || block["disclosure"].is_string() {
        let title = block["disclosure"]
            .as_str()
            .unwrap_or(text(&block["title"]));
        let preview = block["preview"]
            .as_str()
            .map(|value| {
                format!(
                    "<span className=\"clew-disclosure-preview\">{}</span>",
                    reading::inline(value)
                )
            })
            .unwrap_or_default();
        Ok(format!(
            "<details className=\"clew-context-disclosure\">\n<summary>{}{preview}</summary>\n\n{out}\n</details>\n\n",
            md(title)
        ))
    } else if matches!(text(&block["tone"]), "ERROR" | "WARNING" | "INFO") {
        let anchor = block["id"]
            .as_str()
            .map(|id| format!(" {{#question-{id}}}"))
            .unwrap_or_default();
        let class = if block["tone"] == "INFO" {
            "clew-info"
        } else {
            "clew-exception"
        };
        Ok(format!(
            "<aside className=\"{class}\">\n\n### {}{anchor}\n\n{out}</aside>\n\n",
            md(text(&block["title"]))
        ))
    } else {
        Ok(out)
    }
}

#[cfg(test)]
fn process_page(page: &Value, id: &str) -> Result<String, ClewError> {
    process_page_with_context(page, id, &Value::Null)
}
fn process_page_with_context(page: &Value, id: &str, context: &Value) -> Result<String, ClewError> {
    let guide = &page["questionReader"];
    let mut out = header(text(&guide["title"]), &format!("/scenarios/{id}"));
    out.push_str(IMPORTS);
    out.push_str("<ClewAnchor id=\"process-overview\" pageStart />\n\n");
    out.push_str(&format!("# {}\n\n", md(text(&guide["title"]))));

    out.push_str(&page_prose_gaps(page)?);
    if page["questionReaderTranslationGap"].is_object() {
        out.push_str(&reading_guide_gap(
            &page["questionReaderTranslationGap"],
            page["language"].as_str().unwrap_or("en"),
        )?);
    }
    out.push_str(&reading::status(page));
    let flow = page["processDataFlow"].is_object();
    if flow {
        out.push_str(&format!(
            "<ClewProcess dataPath=\"/clew-data/widgets/scenario-{id}.json\" defaultAnchor={}>\n\n",
            json!(text(&page["processDataFlow"]["request"]["anchor"]))
        ));
    }
    out.push_str(&format!("{}\n\n", reading::inline(text(&guide["purpose"]))));
    let mut reading_context = String::from(
        "Editorial reading guide · original source scopes and review states remain separate.\n\n",
    );
    let mut graph = String::from("flowchart TD\n");
    let mut previous = None;
    for (i, section) in rows(&guide["sections"])
        .filter(|s| s["number"].is_number())
        .enumerate()
    {
        let label = crate::documentation::render::escape(text(&section["title"]))
            .replace('"', "&quot;")
            .replace(['\n', '\r'], " ");
        graph.push_str(&format!("s{i}[\"{label}\"]\n"));
        if let Some(previous) = previous {
            graph.push_str(&format!("s{previous} --> s{i}\n"));
        }
        previous = Some(i);
    }
    if previous.is_some() {
        reading_context.push_str(&(String::from("Reading order only. Conditions and failures remain beside the steps below; this is not a runtime trace.\n\n")+&fence(&graph,"mermaid")));
    }
    for section in rows(&guide["sections"]) {
        let sid = text(&section["id"]);
        let anchor = format!("question-{sid}");
        let exception = section["number"].is_null() && sid != "confidence";
        if exception {
            out.push_str("<aside className=\"clew-exception\">\n\n");
        }
        let number = section["number"]
            .as_u64()
            .map(|n| format!("{n}. "))
            .unwrap_or_default();
        out.push_str(&format!(
            "{} {number}{} {{#{anchor}}}\n\n",
            if exception { "###" } else { "##" },
            md(text(&section["title"]))
        ));
        if page["questionReaderTranslationGap"].is_object() {
            let language = page["language"].as_str().unwrap_or("en");
            out.push_str(&format!(
                "<p className=\"question-original-language\">{} · {}</p>\n\n",
                crate::documentation::reader::text(
                    language,
                    "Original reading guide",
                    "Исходный план чтения"
                ),
                super::question_reading::available_language_label(
                    &page["questionReaderTranslationGap"],
                    language
                )
            ));
        }
        if !text(&section["route"]).is_empty() {
            out.push_str(&format!(
                "<p className=\"clew-route\"><code>{{{}}}</code></p>\n\n",
                json!(text(&section["route"]))
            ));
        }
        for (index, b) in rows(&section["blocks"]).enumerate() {
            out.push_str(&block(b, page, sid)?);
            if index == 0 {
                out.push_str(&reading::step_models(page, &anchor));
                out.push_str(&reading::process_topics(page, context, Some(sid))?);
                for reference in rows(&page["modelReferences"]).filter(|r| r["anchor"] == anchor) {
                    out.push_str(&format!(
                        "<ClewModelLink model={} process={} anchor={}>{}</ClewModelLink>\n\n",
                        json!(text(&reference["id"])),
                        json!(id),
                        json!(anchor),
                        md(text(&reference["title"]))
                    ));
                }
            }
        }
        if exception {
            out.push_str("</aside>\n\n");
        }
    }
    out.push_str(&reading::process_topics(page, context, None)?);
    out.push_str(&details("Reading guide and map", &reading_context));
    if guide["conclusion"].is_object() {
        out.push_str(&details(
            text(&guide["conclusion"]["title"]),
            &block(&guide["conclusion"], page, "confidence")?,
        ));
    }
    out.push_str("## Original accounts and implementation\n\n");
    for op in rows(&page["operations"]) {
        out.push_str(&format!(
            "- [{}](../operations/scenarios-{id}--{}.mdx)\n",
            md(text(&op["title"])),
            text(&op["id"])
        ));
    }
    out.push('\n');
    out.push_str(&archive_link(&format!("scenarios/{id}.json")));
    if flow {
        out.push_str("</ClewProcess>\n");
    }
    Ok(out)
}

fn citations(page: &Value, operation: &str, claim: &Value) -> Result<String, ClewError> {
    let mut out = String::new();
    for id in rows(&claim["sourceIds"]).filter_map(Value::as_str) {
        if let Some(source) = page["operationSources"][operation]
            .get(id)
            .or_else(|| page["sources"].get(id))
        {
            out.push_str(&format!("- {}\n", source_link(source)?));
        }
    }
    Ok(out)
}

fn diagram_text(value: &str) -> String {
    let short = value.chars().take(140).collect::<String>();
    let words = short.split_whitespace();
    let mut lines = vec![String::new()];
    for word in words {
        if lines.last().unwrap().len() + word.len() > 42 {
            lines.push(String::new());
        }
        let line = lines.last_mut().unwrap();
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines
        .iter()
        .map(|line| {
            line.chars()
                .map(|c| match c {
                    '&' => "#38;".to_owned(),
                    '<' => "#60;".to_owned(),
                    '>' => "#62;".to_owned(),
                    ';' => "#59;".to_owned(),
                    '"' => "#quot;".to_owned(),
                    _ => c.to_string(),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("<br/>")
}

enum SequenceNode {
    Event(Value),
    Group(SequenceFrame),
}

struct SequenceFrame {
    kind: String,
    arms: Vec<(Value, Vec<SequenceNode>)>,
    end: Option<Value>,
}

// Match the standalone reader's visible-action projection. In particular an
// else action must keep the earlier authored guards when empty arms disappear.
fn sequence_events(op: &Value) -> Result<Vec<Value>, ClewError> {
    let mut stack = vec![SequenceFrame {
        kind: String::new(),
        arms: vec![(Value::Null, Vec::new())],
        end: None,
    }];
    for event in rows(&op["events"]) {
        match text(&event["kind"]) {
            kind @ ("alt" | "opt" | "loop") => stack.push(SequenceFrame {
                kind: kind.into(),
                arms: vec![(event.clone(), Vec::new())],
                end: None,
            }),
            "else" => {
                let frame = stack.last_mut().unwrap();
                if frame.kind != "alt" {
                    return Err(invalid("retained sequence else has no alternative"));
                }
                frame.arms.push((event.clone(), Vec::new()));
            }
            "end" => {
                if stack.len() == 1 {
                    return Err(invalid("retained sequence end has no open branch"));
                }
                let mut frame = stack.pop().unwrap();
                frame.end = Some(event.clone());
                stack
                    .last_mut()
                    .unwrap()
                    .arms
                    .last_mut()
                    .unwrap()
                    .1
                    .push(SequenceNode::Group(frame));
            }
            "message" | "return" | "declared" => stack
                .last_mut()
                .unwrap()
                .arms
                .last_mut()
                .unwrap()
                .1
                .push(SequenceNode::Event(event.clone())),
            "note" => {}
            _ => {
                return Err(invalid(
                    "retained sequence kind is unsupported by the native MDX diagram",
                ));
            }
        }
    }
    if stack.len() != 1 {
        return Err(invalid("retained sequence has an unclosed branch"));
    }
    fn project(nodes: Vec<SequenceNode>) -> Vec<Value> {
        let mut result = Vec::new();
        for node in nodes {
            match node {
                SequenceNode::Event(event) => result.push(event),
                SequenceNode::Group(frame) => {
                    let markers = frame
                        .arms
                        .iter()
                        .map(|(marker, _)| marker.clone())
                        .collect::<Vec<_>>();
                    let mut first = true;
                    let mut visible = false;
                    for (index, (mut marker, nodes)) in frame.arms.into_iter().enumerate() {
                        let events = project(nodes);
                        if events.is_empty() {
                            continue;
                        }
                        if first && index > 0 {
                            let mut guards = vec![format!(
                                "{}: {}",
                                if index == 1 {
                                    "Condition not met"
                                } else {
                                    "Earlier branches not taken"
                                },
                                markers[..index]
                                    .iter()
                                    .map(|m| text(&m["text"]))
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            )];
                            if !text(&marker["text"]).eq_ignore_ascii_case("otherwise") {
                                guards.push(format!("Selected branch: {}", text(&marker["text"])));
                            }
                            marker["kind"] = json!("alt");
                            marker["text"] = json!(guards.join(" · "));
                            for key in ["sourceIds", "dependencyIds"] {
                                let mut seen = std::collections::BTreeSet::new();
                                marker[key] = json!(
                                    markers[..=index]
                                        .iter()
                                        .flat_map(|m| rows(&m[key]))
                                        .filter_map(Value::as_str)
                                        .filter(|id| seen.insert((*id).to_owned()))
                                        .collect::<Vec<_>>()
                                );
                            }
                        }
                        result.push(marker);
                        result.extend(events);
                        first = false;
                        visible = true;
                    }
                    if visible && let Some(end) = frame.end {
                        result.push(end);
                    }
                }
            }
        }
        result
    }
    Ok(project(stack.pop().unwrap().arms.pop().unwrap().1))
}

fn sequence(op: &Value) -> Result<Option<String>, ClewError> {
    let events = sequence_events(op)?;
    if !events
        .iter()
        .any(|e| matches!(text(&e["kind"]), "message" | "return" | "declared"))
    {
        return Ok(None);
    }
    let referenced = events
        .iter()
        .filter(|e| matches!(text(&e["kind"]), "message" | "return" | "declared"))
        .flat_map(|e| [text(&e["from"]), text(&e["to"])])
        .collect::<std::collections::BTreeSet<_>>();
    let mut participants = Vec::new();
    for p in rows(&op["participants"]).filter(|p| referenced.contains(text(&p["id"]))) {
        participants.push((text(&p["id"]).to_owned(), text(&p["label"]).to_owned()));
    }
    for e in events
        .iter()
        .filter(|e| matches!(text(&e["kind"]), "message" | "return" | "declared"))
    {
        for key in ["from", "to"] {
            let id = text(&e[key]);
            if !id.is_empty() && !participants.iter().any(|(p, _)| p == id) {
                participants.push((id.into(), id.into()));
            }
        }
    }
    let names = participants
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id.as_str(), format!("p{i}")))
        .collect::<BTreeMap<_, _>>();
    let mut out = String::from("sequenceDiagram\n");
    for (id, label) in &participants {
        out.push_str(&format!(
            "participant {} as {}\n",
            names[id.as_str()],
            diagram_text(label)
        ));
    }
    let mut stack = Vec::new();
    for event in &events {
        let kind = text(&event["kind"]);
        let label = diagram_text(text(&event["text"]));
        match kind {
            "message" | "return" | "declared" => {
                let (Some(from), Some(to)) = (
                    names.get(text(&event["from"])),
                    names.get(text(&event["to"])),
                ) else {
                    return Err(invalid("retained sequence has an unbound participant"));
                };
                out.push_str(&format!(
                    "{from}{}{to}: {}{label}\n",
                    if kind == "message" { "->>" } else { "-->>" },
                    if kind == "declared" {
                        "[declared] "
                    } else {
                        ""
                    }
                ));
            }
            "alt" | "opt" | "loop" => {
                stack.push(kind);
                out.push_str(&format!("{kind} {label}\n"));
            }
            "else" => {
                if stack.last() != Some(&"alt") {
                    return Err(invalid("retained sequence else has no alternative"));
                }
                out.push_str(&format!("else {label}\n"));
            }
            "end" => {
                if stack.pop().is_none() {
                    return Err(invalid("retained sequence end has no open branch"));
                }
                out.push_str("end\n");
            }
            "note" => {}
            _ => {
                return Err(invalid(
                    "retained sequence kind is unsupported by the native MDX diagram",
                ));
            }
        }
    }
    if !stack.is_empty() {
        return Err(invalid("retained sequence has an unclosed branch"));
    }
    Ok(Some(out))
}

fn http_binding(page: &Value, operation: Option<&str>, record: &Value) -> Value {
    let sources = rows(&record["sourceIds"])
        .filter_map(Value::as_str)
        .filter_map(|id| {
            let source = if let Some(operation) = operation {
                page["operationSources"][operation]
                    .get(id)
                    .or_else(|| page["sources"].get(id))
            } else {
                page["selectedSources"]
                    .get(id)
                    .or_else(|| page["sources"].get(id))
            };
            source.map(|source| (id.to_owned(), source.clone()))
        })
        .collect::<Map<_, _>>();
    json!({"record":record,"sources":sources})
}

fn http_record_links<'a>(
    page: &Value,
    records: impl Iterator<Item = &'a Value>,
    operation: Option<&str>,
) -> Result<String, ClewError> {
    let mut links = String::new();
    let mut seen = std::collections::BTreeSet::new();
    for record in records.filter(|r| r["normalized"]["method"].is_string()) {
        let key = digest(&http_binding(page, operation, record))?;
        if seen.insert(key.clone()) {
            links.push_str(&format!(
                "<ClewContextLink path={}>{} {}</ClewContextLink>\n\n",
                json!(format!("/http/{}", &key[7..])),
                md(text(&record["normalized"]["method"])),
                md(text(&record["normalized"]["path"]))
            ));
        }
    }
    if links.is_empty() {
        Ok(links)
    } else {
        Ok(format!(
            "<div className=\"clew-context-links\">\n\n{links}</div>\n\n"
        ))
    }
}
fn http_links(page: &Value, operation: &str) -> Result<String, ClewError> {
    if text(&page["subject"]).starts_with("service:") && operation.starts_with("section-") {
        return Ok(String::new());
    }
    http_record_links(
        page,
        rows(&page["operationContracts"][operation]),
        Some(operation),
    )
}
fn selected_http_links(page: &Value, input: Option<&str>) -> Result<String, ClewError> {
    http_record_links(
        page,
        rows(&page["contracts"])
            .filter(|r| input.is_none_or(|id| r["normalized"]["entrypoint"] == id)),
        None,
    )
}

fn operation_body(page: &Value, op: &Value, heading: &str) -> Result<String, ClewError> {
    let id = text(&op["id"]);
    let state = page["operationStates"]
        .get(id)
        .or_else(|| page["sectionState"].get(id))
        .unwrap_or(&page["sectionState"]);
    let mut out = format!(
        "Retained account · {} · {}. Runtime behavior is not established by this export.\n\n{}\n\n",
        md(state["freshness"].as_str().unwrap_or("UNVERIFIED")),
        md(state["verification"].as_str().unwrap_or("UNASSESSED")),
        reading::inline(text(&op["summary"]["text"]))
    );
    out.push_str(&details(
        "Summary source bindings",
        &citations(page, id, &op["summary"])?,
    ));
    for contract in rows(&op["interfaceContracts"]) {
        out.push_str(&format!(
            "{heading} {}\n\n| Topic | Retained contract detail |\n| --- | --- |\n",
            md(text(&contract["title"]))
        ));
        let mut bound = String::new();
        for row in rows(&contract["rows"]) {
            out.push_str(&format!(
                "| {} | {} |\n",
                md(text(&row["label"])),
                md(text(&row["value"])).replace('\n', "<br/>")
            ));
            bound.push_str(&format!(
                "**{}**\n\n{}\n",
                md(text(&row["label"])),
                citations(page, id, row)?
            ));
        }
        out.push('\n');
        out.push_str(&details("Contract source bindings", &bound));
    }
    let declared = http_links(page, id)?;
    if !declared.is_empty() {
        out.push_str(&format!(
            "{heading} Declared interface schemas\n\n{declared}\n"
        ));
    }
    if let Some(diagram) = sequence(op)? {
        out.push_str(&format!("{heading} Interaction diagram\n\nActors and nested branches from the retained account; not a runtime trace. Labels are shortened for readability. Full messages and notes follow.\n\n{}",fence(&diagram,"mermaid")));
    }
    let mut events = String::new();
    for (i, event) in rows(&op["events"]).enumerate() {
        events.push_str(&format!(
            "#### {}. {}\n\n{}\n\n{}",
            i + 1,
            md(text(&event["kind"])),
            reading::inline(text(&event["text"])),
            details("Source bindings", &citations(page, id, event)?)
        ));
    }
    if !events.is_empty() {
        out.push_str(&details(
            "Full original sequence, branches and notes",
            &events,
        ));
    }
    if rows(&op["explanation"]).next().is_some() {
        out.push_str(&format!("{heading} Implementation explanation\n\n"));
    }
    for claim in rows(&op["explanation"]) {
        out.push_str(&format!(
            "{}\n\n{}",
            reading::inline(text(&claim["text"])),
            details("Source bindings", &citations(page, id, claim)?)
        ));
    }
    for finding in rows(&op["findings"]) {
        out.push_str(&fence(
            &serde_json::to_string_pretty(finding).map_err(io_error)?,
            "json",
        ));
    }
    out.push_str(&details(
        "Retained boundaries",
        &rows(&op["boundaries"])
            .map(|v| format!("- {}\n", md(text(v))))
            .collect::<String>(),
    ));
    Ok(out)
}

fn operation_page(page: &Value, op: &Value, prefix: &str) -> Result<String, ClewError> {
    let id = text(&op["id"]);
    let mut out = header(text(&op["title"]), &format!("/operations/{prefix}--{id}"));
    out.push_str(IMPORTS);
    out.push_str(&format!("# {}\n\n", md(text(&op["title"]))));
    out.push_str("<ClewPageContext />\n\n");
    out.push_str(&operation_body(page, op, "##")?);
    Ok(out)
}

fn http_page(binding: &Value, key: &str, owner: Option<&Value>) -> Result<String, ClewError> {
    let record = &binding["record"];
    let n = &record["normalized"];
    let operation = &n["operation"];
    let title = format!("{} {}", text(&n["method"]), text(&n["path"]));
    let mut out = header(&title, &format!("/http/{key}"));
    out.push_str(IMPORTS);
    out.push_str(&format!("# {}\n\n<ClewPageContext />\n\nDeclared interface · {}. Runtime enforcement and full specification validation are not established by this bounded record.\n\n", md(&title), md(text(&n["authority"]))));
    if let Some(page) = owner {
        for gap in rows(&page["authoredTranslationGaps"])
            .filter(|gap| gap["record"] == "contractComparison")
        {
            out.push_str(&prose_gap(gap, page["language"].as_str().unwrap_or("en"))?);
        }
        let service = text(&page["subject"])
            .strip_prefix("service:")
            .unwrap_or("");
        let input = rows(&page["catalogue"]).find(|e| {
            e["id"] == n["entrypoint"]
                && serde_json::from_value::<crate::documentation::model::Entrypoint>((*e).clone())
                    .is_ok_and(|e| crate::documentation::process_candidates::public_boundary(&e))
        });
        let anchor = input
            .map(|e| format!("entry-{}", text(&e["id"])))
            .unwrap_or_else(|| {
                if rows(&page["sections"]).any(|v| v["id"] == "section-ingress") {
                    "section-ingress".into()
                } else {
                    "service-overview".into()
                }
            });
        out.push_str(&format!(
            "<ClewContextLink path={}>Service and input context · {}</ClewContextLink>\n\n",
            json!(format!("/services/{service}#{anchor}")),
            md(text(&page["title"]))
        ));
    }
    let mut sources = String::new();
    for source in binding["sources"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.values())
    {
        sources.push_str(&format!("- {}\n", source_link(source)?));
    }
    out.push_str(&details("Declaration source and provenance", &sources));
    out.push_str("## Parameters\n\n");
    out.push_str(&contracts::parameters(&n["parameters"])?);
    out.push_str("## Request body\n\n");
    if operation["requestBody"].is_object() {
        out.push_str(if operation["requestBody"]["required"] == true {
            "Required body.\n\n"
        } else {
            "Optional or absent body.\n\n"
        });
        out.push_str(&contracts::content(&operation["requestBody"]["content"])?);
    } else {
        out.push_str("No request body is recorded in this declaration.\n\n");
    }
    out.push_str("## Declared responses\n\n");
    for (status, response) in operation["responses"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.iter())
    {
        out.push_str(&format!(
            "### {}\n\n{}\n\n",
            md(status),
            reading::inline(text(&response["description"]))
        ));
        if response["content"].as_object().is_none_or(|v| v.is_empty()) {
            out.push_str(&contracts::content(&response["content"])?);
        } else {
            out.push_str(&format!("<details{}>\n<summary>Response media and body schema</summary>\n\n{}\n</details>\n\n",if status.starts_with('2'){" open"}else{""},contracts::content(&response["content"])?));
        }
    }
    out.push_str(&details(
        "Complete retained operation contract",
        &fence(
            &serde_json::to_string_pretty(operation).map_err(io_error)?,
            "json",
        ),
    ));
    let mut context = n.clone();
    context.as_object_mut().unwrap().remove("operation");
    out.push_str(&details(
        "Declaration reader and source context",
        &fence(
            &serde_json::to_string_pretty(&context).map_err(io_error)?,
            "json",
        ),
    ));
    Ok(out)
}

fn comparison_rows(entry: &Value, packet: &Value) -> String {
    let mut out = format!(
        "{}\n\n{}",
        md(text(&entry["coverage"])),
        contracts::findings(entry)
    );
    let mut cases = String::from(
        "<div className=\"clew-table-scroll\"><table className=\"clew-schema-table clew-case-table\"><thead><tr><th scope=\"col\">Retained case</th><th scope=\"col\">HTTP status</th><th scope=\"col\">Status declared</th><th scope=\"col\">Recorded media</th><th scope=\"col\">Request assessment</th><th scope=\"col\">Body assessment</th></tr></thead><tbody>",
    );
    for case in rows(&entry["cases"]) {
        cases.push_str(&format!("<tr><td data-label=\"Retained case\">{}</td><td data-label=\"HTTP status\">{}</td><td data-label=\"Status declared\">{}</td><td data-label=\"Recorded media\">{} · {}</td><td data-label=\"Request assessment\">{}</td><td data-label=\"Body assessment\">{}</td></tr>",
            contracts::literal(text(&case["id"])),
            contracts::literal(&case["response"]["status"].to_string()),
            contracts::literal(&case["statusDeclared"].to_string()),
            contracts::literal(text(&case["response"]["contentType"])),
            contracts::literal(text(&case["mediaAssessment"])),
            contracts::literal(text(&case["requestAssessment"])),
            contracts::literal(text(&case["bodyAssessment"]))));
    }
    cases.push_str("</tbody></table></div>\n\n");
    cases.push_str(&format!("\n{}\n\nExternal JSON-schema assessments are retained from {}; this reader verifies saved bindings and does not rerun that validator.\n\n",md(text(&packet["scope"])),md(text(&packet["validator"]))));
    for limit in rows(&packet["limitations"]) {
        cases.push_str(&format!("- {}\n", md(text(limit))));
    }
    cases.push_str(&details(
        "Raw recorded cases, differences and bindings",
        &fence(
            &serde_json::to_string_pretty(entry).expect("JSON value"),
            "json",
        ),
    ));
    out.push_str(&details("Cases, evidence and limits", &cases));
    out
}

#[cfg(test)]
fn service_page(page: &Value, id: &str) -> Result<String, ClewError> {
    service_page_with_context(page, id, &Value::Null)
}
fn service_page_with_context(page: &Value, id: &str, context: &Value) -> Result<String, ClewError> {
    let mut out = header(text(&page["title"]), &format!("/services/{id}"));
    out.push_str(IMPORTS);
    out.push_str(&format!(
        "<span id=\"service-overview\"></span>\n\n# {}\n\n",
        md(text(&page["title"]))
    ));
    out.push_str(&page_prose_gaps(page)?);
    out.push_str(&reading::status(page));
    out.push_str(&format!("{}\n\n", md(text(&page["subtitle"]))));
    for section in rows(&page["sections"]) {
        let sid = text(&section["id"]);
        out.push_str(&format!(
            "## {} {{#{sid}}}\n\n",
            md(text(&section["title"]))
        ));
        if matches!(sid, "section-ingress" | "section-egress") {
            out.push_str(&reading::service_topics(
                page,
                context,
                sid == "section-ingress",
            )?);
        }
        if section["content"].is_object() {
            out.push_str(&operation_body(page, &section["content"], "###")?);
        } else {
            out.push_str(&format!("{}\n\n", md(text(&section["gap"]))));
        }
        if sid == "section-ingress" {
            let inventory = selected_http_links(page, None)?;
            if !inventory.is_empty() {
                out.push_str("### Declared HTTP interface schemas\n\n");
                out.push_str(&inventory);
            }
            out.push_str("### Captured public inputs\n\n");
            for entry in rows(&page["catalogue"]).filter(|v| {
                serde_json::from_value::<crate::documentation::model::Entrypoint>((*v).clone())
                    .is_ok_and(|e| crate::documentation::process_candidates::public_boundary(&e))
            }) {
                let eid = text(&entry["id"]);
                let label = entry["trigger"]["name"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .unwrap_or(text(&entry["symbol"]));
                let route = rows(&entry["trigger"]["methods"])
                    .chain(rows(&entry["trigger"]["paths"]))
                    .map(text)
                    .collect::<Vec<_>>()
                    .join(" ");
                let label = if route.is_empty() { label } else { &route };
                out.push_str(&format!(
                    "#### {} {{#entry-{eid}}}\n\n{}\n\n{}",
                    md(label),
                    md(text(&entry["kind"])),
                    details(
                        "Declared trigger details",
                        &fence(
                            &serde_json::to_string_pretty(&entry["trigger"]).map_err(io_error)?,
                            "json"
                        )
                    )
                ));
                out.push_str(&selected_http_links(page, Some(eid))?);
                for process in rows(&page["entryProcesses"][eid]) {
                    out.push_str(&format!("- [Retained process: {}](../scenarios/{}.mdx#process-overview) · {} · source revision {}\n",md(text(&process["id"])),text(&process["id"]),md(text(&process["binding"])),md(text(&process["sourceRevision"]))));
                }
                out.push('\n');
                for compared in rows(&page["contractComparison"]["entries"])
                    .filter(|v| v["entrypoint"] == entry["id"])
                {
                    out.push_str("##### Retained HTTP comparison\n\n");
                    if contracts::comparison_current(page, compared) {
                        out.push_str(&comparison_rows(compared, &page["contractComparison"]));
                    } else {
                        out.push_str("Retained HTTP comparison does not match this selected snapshot. Its findings are not applied.\n\n");
                    }
                    out.push_str(&format!("[Comparison scope and original test artifacts](../comparisons/{id}.mdx)\n\n"));
                }
            }
        }
    }
    if rows(&page["notes"]).next().is_some() {
        out.push_str("## Notes and assessments\n\n");
        for note in rows(&page["notes"]) {
            out.push_str(&format!("### {} {{#{}}}\n\n[Open the retained record and separate assessment](../notes/{id}--{}.mdx)\n\n", md(reading::note_title(note)), text(&note["id"]), text(&note["id"])));
        }
    }
    out.push_str("## Original operations\n\n");
    for op in rows(&page["operations"]) {
        out.push_str(&format!(
            "- [{}](../operations/services-{id}--{}.mdx)\n",
            md(text(&op["title"])),
            text(&op["id"])
        ));
    }
    out.push('\n');
    out.push_str(&archive_link(&format!("services/{id}.json")));
    Ok(out)
}

fn comparison_page(c: &Value, id: &str, page: &Value) -> Result<String, ClewError> {
    let title = format!("{id}: retained HTTP comparison");
    let mut out = header(&title, &format!("/comparisons/{id}"));
    out.push_str(IMPORTS);
    out.push_str(&format!("# {}\n\n", md(&title)));
    for gap in
        rows(&page["authoredTranslationGaps"]).filter(|gap| gap["record"] == "contractComparison")
    {
        out.push_str(&prose_gap(gap, page["language"].as_str().unwrap_or("en"))?);
    }
    out.push_str(&format!(
        "{}\n\nAuthority: {} · binding: {}\n\nSnapshot: {}\n\nRevision: {}\n\n",
        md(text(&c["scope"])),
        md(text(&c["authority"])),
        md(text(&c["bindingStatus"])),
        md(text(&c["snapshot"])),
        md(text(&c["revision"]))
    ));
    for entry in rows(&c["entries"]) {
        out.push_str(&format!(
            "## {} {}\n\n{}",
            md(text(&entry["method"])),
            md(text(&entry["path"])),
            comparison_rows(entry, c)
        ));
    }
    out.push_str("## Scope and limitations\n\n");
    for limit in rows(&c["limitations"]) {
        out.push_str(&format!("- {}\n", md(text(limit))));
    }
    out.push('\n');
    for source in c["sources"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.values())
    {
        out.push_str(&format!("- {}\n", source_link(source)?));
    }
    out.push('\n');
    for (name, artifact) in c["artifacts"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.iter())
    {
        out.push_str(&details(
            &format!("Retained artifact: {name}"),
            &format!(
                "Digest: {}\n\n{}",
                md(text(&artifact["digest"])),
                fence(text(&artifact["text"]), "text")
            ),
        ));
    }
    Ok(out)
}

fn source_page(source: &Value, title: &str, slug: &str, language: &str) -> String {
    use crate::documentation::language::label;
    let mut body = header(title, &format!("/sources/{slug}"));
    body.push_str(IMPORTS);
    body.push_str(&format!(
        "# {}\n\n{}: {}\n\n{}: {}\n\n{}: {}\n\n## {} {{#retained-text}}\n\n{}",
        md(title),
        md(label(language, "Authority")),
        md(text(&source["authority"])),
        md(label(language, "Revision")),
        md(text(&source["revision"])),
        md(label(language, "Text digest")),
        md(text(&source["textDigest"])),
        md(label(language, "Retained text")),
        fence(text(&source["text"]), "text")
    ));
    body.push_str(&format!(
        "<ClewDownload path=\"/clew-data/objects/{slug}.json\">{}</ClewDownload>\n",
        contracts::literal(label(language, "Complete original source binding"))
    ));
    body
}

fn declared_contract_page(
    contract: &Value,
    gap: Option<&Value>,
    language: &str,
) -> Result<String, ClewError> {
    use crate::documentation::language::label;
    let id = text(&contract["id"]);
    let mut body = header(text(&contract["title"]), &format!("/contracts/{id}"));
    body.push_str(IMPORTS);
    body.push_str(&format!("# {}\n\n", md(text(&contract["title"]))));
    if let Some(gap) = gap {
        body.push_str(&prose_gap(gap, language)?);
    }
    body.push_str(&format!(
        "{}\n\n{}:\n\n{}",
        md(label(
            language,
            "Declared contract · runtime conformance is not established."
        )),
        md(label(language, "Exact pointer")),
        fence(text(&contract["branchPointer"]), "text")
    ));
    body.push_str(&source_link_in_language(
        &contract["source"],
        "../",
        language,
    )?);
    body.push_str(&format!("\n\n## {} {{#properties}}\n\n{}\n\n",
        md(label(language, "Properties")),
        md(label(language, "Requiredness follows the selected branch and required parents. Required and nullable are separate."))));
    for leaf in rows(&contract["leaves"]) {
        body.push_str(&format!(
            "### {}\n\n{}: {} · {}: {}\n\n{}",
            md(text(&leaf["field"])),
            md(label(language, "Required when branch matches")),
            leaf["requiredWhenBranchMatches"],
            md(label(language, "Accepts null")),
            leaf["nullable"],
            fence(
                &serde_json::to_string_pretty(&leaf["schema"]).map_err(io_error)?,
                "json"
            )
        ));
    }
    Ok(body)
}

fn model_page(
    model: &Value,
    catalog: &Value,
    gap: Option<&Value>,
    language: &str,
) -> Result<String, ClewError> {
    let id = text(&model["id"]);
    let mut out = header(text(&model["title"]), &format!("/data-model/{id}"));
    out.push_str(IMPORTS);
    out.push_str(&format!("# {}\n\n", md(text(&model["title"]))));
    if let Some(gap) = gap {
        out.push_str(&prose_gap(gap, language)?);
    }

    out.push_str(&format!(
        "{clew_label_0} · {clew_label_1}.\n\n<p id=\"dm-return\"></p>\n\n{}\n\n",
        reading::inline(text(&model["purpose"]["text"])),
        clew_label_0 = contracts::literal(crate::documentation::language::label(
            language,
            "Agent proposal"
        )),
        clew_label_1 = contracts::literal(crate::documentation::language::label(
            language,
            "Not reviewed"
        ))
    ));
    out.push_str(&evidence_language(&model["purpose"], language)?);
    out.push_str(&format!(
        "## {clew_label_0} {{#identity-and-relationships}}\n\n{}\n\n",
        reading::inline(text(&model["identity"]["text"])),
        clew_label_0 = contracts::literal(crate::documentation::language::label(
            language,
            "Identity and relationships"
        ))
    ));
    out.push_str(&evidence_language(&model["identity"], language)?);
    let related = rows(&model["relatedModels"])
        .filter_map(Value::as_str)
        .map(|id| {
            let title = rows(&catalog["models"])
                .find(|m| m["id"] == id)
                .and_then(|m| m["title"].as_str())
                .unwrap_or(id);
            format!(
                "<ClewRelatedModelLink model={}>{}</ClewRelatedModelLink>",
                json!(id),
                md(title)
            )
        })
        .collect::<Vec<_>>();
    if !related.is_empty() {
        out.push_str(&format!(
            "<p className=\"clew-related-models\">{clew_label_0}: {}</p>\n\n",
            related.join(" · "),
            clew_label_0 = contracts::literal(crate::documentation::language::label(
                language,
                "Related concepts"
            ))
        ));
    }
    out.push('\n');
    if rows(&model["records"]).next().is_some() {
        let literal = |value: &str| {
            super::super::render::escape(value)
                .replace('{', "&#123;")
                .replace('}', "&#125;")
        };
        out.push_str(&format!("## {clew_label_0} {{#reference-records}}\n\n<table className=\"dm-records\"><caption>{clew_label_1}</caption><thead><tr>", clew_label_0 = contracts::literal(crate::documentation::language::label(language, "Reference records")), clew_label_1 = contracts::literal(crate::documentation::language::label(language, "Source-backed literal records · authored interpretation"))));
        for field in rows(&model["fields"]) {
            out.push_str(&format!(
                "<th scope=\"col\">{}</th>",
                literal(text(&field["path"]))
            ));
        }
        out.push_str(&format!(
            "<th scope=\"col\">{clew_label_0}</th></tr></thead><tbody>",
            clew_label_0 =
                contracts::literal(crate::documentation::language::label(language, "Evidence"))
        ));
        for (index, record) in rows(&model["records"]).enumerate() {
            out.push_str("<tr>");
            for field in rows(&model["fields"]) {
                out.push_str(&format!(
                    "<td data-field={{{}}}><code>{}</code></td>",
                    json!(text(&field["path"])),
                    literal(&record["values"][text(&field["id"])].to_string())
                ));
            }
            out.push_str(&format!("<td data-field=\"Evidence\" data-label=\"{clew_label_1}\"><a href=\"#reference-record-{}\">{clew_label_0} {}</a></td></tr>",
                index + 1, index + 1, clew_label_0 = contracts::literal(crate::documentation::language::label(language, "Source")), clew_label_1 = contracts::literal(crate::documentation::language::label(language, "Evidence"))));
        }
        out.push_str("</tbody></table>\n\n");
        for (index, record) in rows(&model["records"]).enumerate() {
            out.push_str(&format!(
                "<span id=\"reference-record-{}\"></span>\n\n{}",
                index + 1,
                details(
                    &format!(
                        "{clew_label_0} {}: {clew_label_1}",
                        index + 1,
                        clew_label_0 = contracts::literal(crate::documentation::language::label(
                            language, "Record"
                        )),
                        clew_label_1 = contracts::literal(crate::documentation::language::label(
                            language,
                            "source and scope"
                        ))
                    ),
                    &format!(
                        "{}\n\n{}",
                        reading::inline(text(&record["claim"]["text"])),
                        evidence_language(&record["claim"], language)?
                    )
                )
            ));
        }
    }
    out.push_str(&format!(
        "## {clew_label_0} {{#fields-and-representations}}\n\n",
        clew_label_0 = contracts::literal(crate::documentation::language::label(
            language,
            "Fields and representations"
        ))
    ));
    for field in rows(&model["fields"]) {
        let fid = text(&field["id"]);
        out.push_str(&format!(
            "### {} {{#field-{fid}}}\n\n{}\n\n",
            md(text(&field["path"])),
            reading::inline(text(&field["meaning"]["text"]))
        ));
        out.push_str(&evidence_language(&field["meaning"], language)?);
        out.push_str(&contracts::representations(field, catalog, language));
        for rep in rows(&field["representations"]) {
            let mut body = format!(
                "{} · {} · {}\n\n{}\n\n",
                md(text(&rep["typeLabel"])),
                md(text(&rep["requiredness"])),
                md(text(&rep["nullability"])),
                reading::inline(text(&rep["mapping"]["text"]))
            );
            if let Some(contract) = rows(&catalog["contracts"])
                .find(|c| c["id"] == rep["contract"] && rep["contract"].is_string())
            {
                body.push_str(&format!(
                    "[{}](../contracts/{}.mdx)\n\n",
                    md(text(&contract["title"])),
                    text(&contract["id"])
                ));
                for leaf in rows(&contract["leaves"]).filter(|l| l["field"] == rep["field"]) {
                    body.push_str(&format!(
                        "{clew_label_0}: {} · {clew_label_1}: {}\n\n",
                        leaf["requiredWhenBranchMatches"],
                        leaf["nullable"],
                        clew_label_0 = contracts::literal(crate::documentation::language::label(
                            language,
                            "Required when branch matches"
                        )),
                        clew_label_1 = contracts::literal(crate::documentation::language::label(
                            language,
                            "Accepts null"
                        ))
                    ));
                    body.push_str(&fence(
                        &serde_json::to_string_pretty(&leaf["schema"]).map_err(io_error)?,
                        "json",
                    ));
                }
            }
            body.push_str(&evidence_language(&rep["mapping"], language)?);
            out.push_str(&details(text(&rep["label"]), &body));
        }
        if !super::data_model::reference_only(model) {
            out.push_str(&format!("<ClewFieldTrace field={} />\n\n", json!(fid)));
        }
    }
    if super::data_model::reference_only(model) {
        out.push_str(&format!("<ClewDataModel dataPath=\"/clew-data/widgets/model-{id}.json\" contextOnly />\n\n## {clew_label_0} {{#processes}}\n\n", clew_label_0 = contracts::literal(crate::documentation::language::label(language, "Processes"))));
    } else {
        out.push_str(&format!("## {clew_label_0} {{#field-usage}}\n\n<ClewDataModel dataPath=\"/clew-data/widgets/model-{id}.json\" />\n\n## {clew_label_1} {{#processes}}\n\n", clew_label_0 = contracts::literal(crate::documentation::language::label(language, "Field usage and focused flow")), clew_label_1 = contracts::literal(crate::documentation::language::label(language, "Processes"))));
    }
    for link in rows(&model["processLinks"]) {
        if let Some(process) = text(&link["subject"]).strip_prefix("scenario:") {
            out.push_str(&format!(
                "- [{}](../scenarios/{process}.mdx#{})\n",
                md(text(&link["label"])),
                text(&link["anchor"])
            ));
        }
    }
    out.push('\n');
    Ok(out)
}

fn put(files: &mut BTreeMap<String, Vec<u8>>, name: impl Into<String>, body: impl Into<Vec<u8>>) {
    files.insert(name.into(), body.into());
}

fn page_prose_gaps(page: &Value) -> Result<String, ClewError> {
    let mut notices = String::new();
    let language = page["language"].as_str().unwrap_or("en");
    for gap in rows(&page["authoredTranslationGaps"]) {
        notices.push_str(&prose_gap(gap, language)?);
    }
    Ok(notices)
}

fn prose_gap(gap: &Value, language: &str) -> Result<String, ClewError> {
    use crate::documentation::{reader::text as label, render::escape, store};
    let original = text(&gap["originalPath"]);
    store::relative(original)?;
    let path = escape(&format!("/clew-original/{original}"))
        .replace('{', "&#123;")
        .replace('}', "&#125;");
    Ok(format!(
        "## {}: {}\n\n{}\n\n{}: {}\n\n<ClewDownload path=\"{path}\">{}</ClewDownload>\n\n",
        label(language, "Translation pending", "Требуется перевод"),
        super::prose_language::description_label(gap, language),
        super::prose_language::explanation(language),
        label(
            language,
            "Original prose language",
            "Язык исходного описания"
        ),
        super::question_reading::available_language_label(gap, language),
        label(
            language,
            "Original description (JSON)",
            "Исходное описание (JSON)"
        )
    ))
}

fn reading_guide_gap(gap: &Value, language: &str) -> Result<String, ClewError> {
    use crate::documentation::{reader::text as label, render::escape, store};
    let original = text(&gap["originalPath"]);
    store::relative(original)?;
    let path = escape(&format!("/clew-original/{original}"))
        .replace('{', "&#123;")
        .replace('}', "&#125;");
    let available = match gap["availableLanguage"].as_str() {
        Some("en") => label(language, "English", "Английский"),
        Some("ru") => label(language, "Russian", "Русский"),
        _ => label(language, "unknown", "неизвестен"),
    };
    Ok(format!(
        "## {}\n\n{}\n\n{}: {}\n\n<ClewDownload path=\"{path}\">{}</ClewDownload>\n",
        label(language, "Translation pending", "Требуется перевод"),
        label(
            language,
            "This reading guide is not available in the selected language. Its readable original remains below, with the same anchors and evidence. This is not a translation or a new review.",
            "План чтения недоступен на выбранном языке. Ниже доступен исходный план с прежними ссылками, якорями и основаниями. Это не перевод и не новая проверка."
        ),
        label(
            language,
            "Original prose language",
            "Язык исходного описания"
        ),
        available,
        label(
            language,
            "Original reading guide (JSON)",
            "Исходный план чтения (JSON)"
        )
    ))
}

pub(super) fn project(
    native: &BTreeMap<String, Vec<u8>>,
    manifest: &Value,
    inputs: &super::full::ReadingInputs<'_>,
) -> Result<(BTreeMap<String, Vec<u8>>, Value), ClewError> {
    let language = manifest["documentationLanguage"].as_str().unwrap_or("en");
    crate::documentation::language::validate(Some(language))?;
    let mut files = BTreeMap::new();
    let mut documents = BTreeMap::new();
    let context = crate::documentation::reader::navigation_context(native)?;
    let mut sidebars = vec![json!("overview")];
    for (name, bytes) in native.iter().filter(|(name, _)| name.ends_with(".json")) {
        if name.ends_with(".original.json") {
            put(
                &mut files,
                format!("static/clew-original/{name}"),
                bytes.clone(),
            );
            continue;
        }
        let value = crate::canonical::parse_json_strict(bytes).map_err(io_error)?;
        documents.insert(format!("native/{name}"), value);
    }
    let catalog = documents
        .get("native/data-model.json")
        .cloned()
        .unwrap_or(json!({"models":[],"contracts":[]}));
    let pages = documents
        .iter()
        .filter(|(name, _)| {
            name.starts_with("native/scenarios/") || name.starts_with("native/services/")
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    let mut http_documents = BTreeMap::new();
    let mut process_items = Vec::new();
    let mut service_items = Vec::new();
    let system_title = documents
        .get("native/system-reading-guide.json")
        .and_then(|g| g["title"].as_str())
        .unwrap_or("Documentation");
    let mut overview = header(system_title, "/");
    overview.push_str(IMPORTS);
    overview.push_str(&format!("# {}\n\n", md(system_title)));
    overview.push_str("<ClewStatus />\n\nFollow a process, inspect its contracts and data, and return to the source evidence when needed.\n\n## Processes\n\n");
    for (name, page) in &pages {
        for op in rows(&page["operations"]).chain(rows(&page["sections"]).map(|s| &s["content"])) {
            for contract in rows(&page["operationContracts"][text(&op["id"])])
                .filter(|c| c["normalized"]["method"].is_string())
            {
                let binding = http_binding(page, Some(text(&op["id"])), contract);
                http_documents.insert(digest(&binding)?, binding);
            }
        }
        for contract in rows(&page["contracts"]).filter(|c| c["normalized"]["method"].is_string()) {
            let binding = http_binding(page, None, contract);
            http_documents.insert(digest(&binding)?, binding);
        }
        let id = name.rsplit('/').next().unwrap().trim_end_matches(".json");
        let process = name.starts_with("native/scenarios/");
        let directory = if process { "scenarios" } else { "services" };
        let title = if page["questionReader"].is_object() {
            text(&page["questionReader"]["title"])
        } else {
            text(&page["title"])
        };
        let body = if process && page["questionReader"].is_object() {
            process_page_with_context(page, id, &context)?
        } else if !process {
            service_page_with_context(page, id, &context)?
        } else {
            let mut body = header(title, &format!("/scenarios/{id}"));
            body.push_str(IMPORTS);
            body.push_str(&format!(
                "<ClewAnchor id=\"process-overview\" pageStart />\n\n# {}\n\n",
                md(title)
            ));
            body.push_str(&page_prose_gaps(page)?);
            for op in rows(&page["operations"]) {
                body.push_str(&format!(
                    "## {} {{#{}}}\n\n",
                    md(text(&op["title"])),
                    text(&op["id"])
                ));
                body.push_str(&operation_body(page, op, "###")?);
            }
            body
        };
        put(&mut files, format!("docs/{directory}/{id}.mdx"), body);
        if !process {
            for note in rows(&page["notes"]) {
                put(
                    &mut files,
                    format!("docs/notes/{id}--{}.mdx", text(&note["id"])),
                    reading::note_page(page, id, note)?,
                );
            }
        }
        if process {
            process_items.push(json!(format!("scenarios/{id}")));
            overview.push_str(&format!("- [{}](scenarios/{id}.mdx)\n", md(title)));
        } else {
            service_items.push(json!(format!("services/{id}")));
        }
        if page["processDataFlow"].is_object() {
            documents.insert(
                format!("widgets/scenario-{id}.json"),
                page["processDataFlow"].clone(),
            );
        }
        for op in rows(&page["operations"]) {
            put(
                &mut files,
                format!("docs/operations/{directory}-{id}--{}.mdx", text(&op["id"])),
                operation_page(page, op, &format!("{directory}-{id}"))?,
            );
        }
    }
    for (key, binding) in &http_documents {
        put(
            &mut files,
            format!("docs/http/{}.mdx", &key[7..]),
            http_page(
                binding,
                &key[7..],
                pages.iter().map(|(_, p)| p).find(|p| {
                    p["subject"] == format!("service:{}", text(&binding["record"]["service"]))
                }),
            )?,
        );
    }
    for (name, page) in &pages {
        if name.starts_with("native/services/") && page["contractComparison"].is_object() {
            let id = name.rsplit('/').next().unwrap().trim_end_matches(".json");
            put(
                &mut files,
                format!("docs/comparisons/{id}.mdx"),
                comparison_page(&page["contractComparison"], id, page)?,
            );
        }
    }
    if !process_items.is_empty() {
        sidebars.push(json!({"type":"category","label":crate::documentation::language::label(language, "Processes"),"items":process_items}));
    }
    if rows(&catalog["models"]).next().is_some() {
        overview.push_str("\n## Data model\n\n");
    }
    let mut model_items = Vec::new();
    for model in rows(&catalog["models"]) {
        let id = text(&model["id"]);
        put(
            &mut files,
            format!("docs/data-model/{id}.mdx"),
            model_page(
                model,
                &catalog,
                manifest["modelTranslationGaps"].get(id),
                language,
            )?,
        );
        documents.insert(format!("widgets/model-{id}.json"),json!({"models":[model],"contracts":super::data_model::model_contracts(model,&catalog["contracts"]),"authority":catalog["authority"],"review":catalog["review"]}));
        model_items.push(json!(format!("data-model/{id}")));
        overview.push_str(&format!(
            "- [{}](data-model/{id}.mdx)\n",
            md(text(&model["title"]))
        ));
    }
    if !model_items.is_empty() {
        sidebars.push(json!({"type":"category","label":crate::documentation::language::label(language, "Data model"),"items":model_items}));
    }
    let mut contract_items = Vec::new();
    for contract in rows(&catalog["contracts"]) {
        let id = text(&contract["id"]);
        let body = declared_contract_page(
            contract,
            manifest["declaredContractTranslationGaps"].get(id),
            language,
        )?;
        put(&mut files, format!("docs/contracts/{id}.mdx"), body);
        contract_items.push(json!(format!("contracts/{id}")));
    }
    if !contract_items.is_empty() {
        sidebars
            .push(json!({"type":"category","label":crate::documentation::language::label(language, "Declared contracts"),"items":contract_items}));
    }
    if !service_items.is_empty() {
        sidebars.push(json!({"type":"category","label":crate::documentation::language::label(language, "Service context"),"items":service_items}));
    }
    if let Some(guide) = documents.get("native/system-reading-guide.json") {
        let mut story = if manifest["readingGuideTranslationGaps"]["system:overview"].is_object() {
            reading_guide_gap(
                &manifest["readingGuideTranslationGaps"]["system:overview"],
                language,
            )?
        } else {
            String::new()
        };
        story.push_str(&format!("{}\n\n", reading::inline(text(&guide["purpose"]))));
        for section in rows(&guide["sections"]) {
            story.push_str(&format!("## {}\n\n", md(text(&section["title"]))));
            for b in rows(&section["blocks"]) {
                story.push_str("<article className=\"clew-outcome\">\n\n");
                story.push_str(&format!("### {}\n\n", md(text(&b["title"]))));
                story.push_str(&block_in(b, &Value::Null, text(&section["id"]), "")?);
                story.push_str("</article>\n\n");
            }
        }
        if !text(&guide["conclusion"]["title"]).is_empty() {
            story.push_str(&format!(
                "<aside className=\"clew-completion-boundary\">\n\n## {}\n\n",
                md(text(&guide["conclusion"]["title"]))
            ));
        }
        story.push_str(&block_in(
            &guide["conclusion"],
            &Value::Null,
            "confidence",
            "",
        )?);
        if !text(&guide["conclusion"]["title"]).is_empty() {
            story.push_str("</aside>\n\n");
        }
        let marker = "## Processes\n\n";
        let position = overview.find(marker).unwrap();
        overview.insert_str(position, &story);
    }
    overview.push_str("\n## Provenance\n\nOriginal claims, evidence and their independent review states remain in the reference archive. Source references include the original revision, digest and line range; no runtime guarantee is added by export.\n\n");
    overview.push_str("[Source fragments](sources/index.mdx)\n\n");
    put(&mut files, "docs/overview.mdx", overview);
    reading::ancillary_pages(native, &mut files, &mut sidebars, language)?;
    let mut routes = reading::route_map(native, &pages)?;
    reading::linked_downloads(native, &mut files, &mut routes)?;
    put(
        &mut files,
        "src/components/clew-routes.json",
        serde_json::to_vec(&routes).map_err(io_error)?,
    );
    put(
        &mut files,
        "route-map.json",
        serde_json::to_vec_pretty(&routes).map_err(io_error)?,
    );
    let mut objects = Objects::default();
    for value in documents.values() {
        objects.count(value)?;
    }
    let mut verified = 0;
    for (name, value) in &documents {
        let normalized = objects.normalize(value)?;
        if objects.expand(&normalized, 0)? != *value {
            return Err(invalid(
                "portable publication differs after resolving its evidence references",
            ));
        }
        verified += 1;
        put(
            &mut files,
            format!("static/clew-data/{name}"),
            serde_json::to_vec_pretty(
                &json!({"schema":"codeclew-normalized-data/1.0","value":normalized}),
            )
            .map_err(io_error)?,
        );
    }
    for (id, value) in &objects.values {
        put(
            &mut files,
            format!("static/clew-data/objects/{}.json", &id[7..]),
            serde_json::to_vec_pretty(
                &json!({"schema":"codeclew-export-object/1.0","identity":id,"value":value}),
            )
            .map_err(io_error)?,
        );
    }
    let mut source_index = header(
        crate::documentation::language::label(language, "Source fragments"),
        "/sources",
    );
    source_index.push_str(&format!("# {} {{#source-fragments}}\n\n{}\n\n", md(crate::documentation::language::label(language, "Source fragments")), md(crate::documentation::language::label(language, "Each page keeps its exact retained revision, range, text digest and authority. Identical text at another revision remains a separate binding."))));
    for (id, source) in &objects.sources {
        let slug = &id[7..];
        let title = format!(
            "{}:{}–{}",
            text(&source["file"]),
            source["startLine"],
            source["endLine"]
        );
        let body = source_page(source, &title, slug, language);
        put(&mut files, format!("docs/sources/{slug}.mdx"), body);
        source_index.push_str(&format!(
            "- [{}]({slug}.mdx) · {}\n",
            md(&title),
            md(text(&source["revision"]))
        ));
    }
    put(&mut files, "docs/sources/index.mdx", source_index);
    sidebars.push(json!("sources/index"));
    put(
        &mut files,
        "sidebars.js",
        format!(
            "export default {{docs: {}}};\n",
            serde_json::to_string_pretty(&sidebars).map_err(io_error)?
        ),
    );
    for (name, path) in [
        ("reading-plan.json", inputs.reading_plan),
        ("data-model-plan.json", inputs.data_model),
        ("contract-comparison.json", inputs.contract_comparison),
    ] {
        if let Some(path) = path {
            put(
                &mut files,
                format!("inputs/{name}"),
                std::fs::read(path).map_err(io_error)?,
            );
        }
    }
    let locale = format!(
        "{}\nexport const language={};\nconst {{t,chromeHtml}}=clewLocale(language,CLEW_RU_MESSAGES);\nexport {{t,chromeHtml}};\n",
        crate::documentation::language::browser_script(),
        serde_json::to_string(language).map_err(io_error)?
    );
    put(&mut files, "src/components/clew-locale.js", locale);
    let model_runtime = format!(
        "{}{}",
        crate::documentation::language::browser_script(),
        include_str!("../../../assets/documentation/data-model.js")
    );
    for (path, bytes) in [
        (
            "package.json",
            include_bytes!("../../../assets/documentation/docusaurus/package.json").as_slice(),
        ),
        (
            "package-lock.json",
            include_bytes!("../../../assets/documentation/docusaurus/package-lock.json").as_slice(),
        ),
        (
            "docusaurus.config.js",
            include_bytes!("../../../assets/documentation/docusaurus/docusaurus.config.js")
                .as_slice(),
        ),
        (
            "src/components/ClewReaders.jsx",
            include_bytes!("../../../assets/documentation/docusaurus/ClewReaders.jsx").as_slice(),
        ),
        (
            "src/theme/NavbarItem/ComponentTypes.js",
            include_bytes!("../../../assets/documentation/docusaurus/NavbarItems.js").as_slice(),
        ),
        (
            "src/components/ClewSearch.jsx",
            include_bytes!("../../../assets/documentation/docusaurus/ClewSearch.jsx").as_slice(),
        ),
        (
            "src/components/clew-model-runtime.js",
            model_runtime.as_bytes(),
        ),
        (
            "scripts/pack-evidence.mjs",
            include_bytes!("../../../assets/documentation/docusaurus/pack-evidence.mjs").as_slice(),
        ),
        (
            "src/css/clew.css",
            include_bytes!("../../../assets/documentation/docusaurus/clew.css").as_slice(),
        ),
        (
            "src/css/typography.css",
            include_bytes!("../../../assets/documentation/typography.css").as_slice(),
        ),
        (
            "src/css/data-model.css",
            include_bytes!("../../../assets/documentation/data-model.css").as_slice(),
        ),
    ] {
        put(&mut files, path, bytes.to_vec());
    }
    put(
        &mut files,
        ".gitignore",
        b"node_modules/\nbuild/\n.docusaurus/\nstatic/clew-reader-packs/\n".to_vec(),
    );
    put(
        &mut files,
        "README.md",
        include_bytes!("../../../assets/documentation/docusaurus/README.md").to_vec(),
    );
    let mut result = manifest.clone();
    result["schema"] = json!("codeclew-docusaurus-export/1.0");
    result["format"] = json!("docusaurus");
    result["output"] = json!(".");
    result["index"] = json!("docs/overview.mdx");
    result["pages"] = json!(pages.iter().map(|(name,page)| {
        let original=name.trim_start_matches("native/");
        json!({"subject":page["subject"],"mdx":format!("docs/{}.mdx",original.trim_end_matches(".json")),"data":format!("static/clew-data/native/{original}")})
    }).collect::<Vec<_>>());
    result["referenceEncoding"] = json!("codeclew-export-object/1.0");
    result["resolvedPayloadParity"] = json!({"status":"VERIFIED","documents":verified,"sourceBindings":objects.sources.len(),"sharedObjects":objects.values.len()});
    result["files"] = json!(
        files
            .iter()
            .map(|(path, bytes)| json!({"path":path,"digest":crate::canonical::hash_bytes(bytes)}))
            .collect::<Vec<_>>()
    );
    Ok((files, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guide_language_gap_keeps_original_steps_context_links_and_provenance_readable() {
        let block = json!({"title":"Original block","text":"Original retained explanation.","evidence":[],"links":[{"subject":"service:sample","operation":"section-entities","label":"Original service context"}]});
        let guide = json!({"title":"Original process","purpose":"Original purpose.","language":"en","sections":[
            {"id":"request","title":"Forward request","number":1,"blocks":[block]},
            {"id":"save","title":"Save place","number":2,"blocks":[block]},
            {"id":"apply","title":"Apply response","number":3,"blocks":[block]},
            {"id":"confidence","title":"Original source and scope","blocks":[block]}],"conclusion":{"title":"Unknown completion","text":"Completion remains unknown.","evidence":[],"links":[]}});
        let mut page = json!({"subject":"scenario:save","language":"ru","questionReader":guide,"operations":[],"operationSources":{},"sources":{}});
        page["questionReaderTranslationGap"] = super::super::question_reading::translation_gap(
            &page["questionReader"],
            Some("ru"),
            "scenarios/save.reading-guide.original.json",
        )
        .unwrap()
        .unwrap();
        let original = page.clone();
        let rendered = process_page_with_context(&page, "save", &Value::Null).unwrap();
        for id in ["request", "save", "apply", "confidence"] {
            assert!(rendered.contains(&format!("{{#question-{id}}}")), "{id}");
        }
        assert!(
            rendered.contains("Требуется перевод") && rendered.contains("Исходный план чтения")
        );
        assert!(rendered.contains("Original retained explanation."));
        assert!(rendered.contains("section-entities") && rendered.contains("fromQuestion"));
        assert!(
            rendered.contains("<ClewStatus"),
            "review and source scope retained"
        );
        assert!(rendered.contains("/clew-original/scenarios/save.reading-guide.original.json"));
        assert_eq!(page, original);
    }

    #[test]
    fn retained_http_cases_have_named_cells_and_preserve_every_assessment() {
        let rendered = comparison_rows(
            &json!({"coverage":"Selected cases only","cases":[{"id":"extra-property","response":{"status":200,"contentType":"application/json"},"statusDeclared":true,"mediaAssessment":"declared","requestAssessment":"outside-schema","bodyAssessment":"matches-selected-schema"}]}),
            &json!({"validator":"retained validator"}),
        );
        for label in [
            "Retained case",
            "HTTP status",
            "Status declared",
            "Recorded media",
            "Request assessment",
            "Body assessment",
        ] {
            assert!(rendered.contains(&format!("data-label=\"{label}\"")));
        }
        for value in [
            "extra-property",
            "200",
            "true",
            "application/json",
            "outside-schema",
            "matches-selected-schema",
            "Selected cases only",
            "retained validator",
            "Raw recorded cases",
        ] {
            assert!(rendered.contains(value), "{value}");
        }
    }

    #[test]
    fn portable_flow_and_comparison_notices_keep_readable_originals_and_widgets() {
        let original = json!({"language":"ru","subject":"scenario:save","title":"Original service",
            "questionReader":{"title":"Original process","purpose":"Original process purpose","sections":[]},
            "processDataFlow":{"request":{"documentationLanguage":"en","anchor":"question-save","title":"Original flow"},"models":[]},
            "contractComparison":{"documentationLanguage":"en","scope":"Original comparison scope","entries":[],"limitations":["Original bounded limitation"],"sources":{},"artifacts":{}}});
        let mut page = original.clone();
        let mut files = BTreeMap::new();
        super::super::prose_language::attach_payload(
            &mut page,
            "scenarios/save.json",
            Some("ru"),
            &mut files,
        )
        .unwrap();
        let process = process_page(&page, "save").unwrap();
        assert!(process.contains("Original process purpose"));
        assert_eq!(
            process
                .lines()
                .filter(|line| line.starts_with("# "))
                .count(),
            1
        );
        assert_eq!(process.matches("hide_title: true").count(), 1);
        assert!(process.contains("<ClewProcess"));
        assert!(process.contains("question-save"));
        assert!(process.contains("/clew-original/scenarios/save.data-flow.original.json"));
        let comparison = comparison_page(&page["contractComparison"], "save", &page).unwrap();
        assert!(comparison.contains("Original comparison scope"));
        assert_eq!(
            comparison
                .lines()
                .filter(|line| line.starts_with("# "))
                .count(),
            1
        );
        assert_eq!(comparison.matches("hide_title: true").count(), 1);
        assert!(comparison.contains("Original bounded limitation"));
        assert!(comparison.contains("/clew-original/scenarios/save.comparison.original.json"));
        assert_eq!(page["contractComparison"], original["contractComparison"]);
        assert_eq!(page["processDataFlow"], original["processDataFlow"]);
    }

    #[test]
    fn source_appendix_localizes_only_owned_labels_and_keeps_source_bytes_and_routes() {
        let raw = "// Revision\nclass Authority { String value = \"Retained text\"; }\n";
        let source = json!({"file":"Authority.java","revision":"Revision","authority":"COMPILER_BOUND","text":raw,"textDigest":crate::canonical::hash_bytes(raw.as_bytes()),"startLine":1,"endLine":2});
        let original = source.clone();
        let id = digest(&source).unwrap();
        for language in ["en", "ru"] {
            let body = source_page(&source, "Authority.java:1-2", &id[7..], language);
            assert!(body.contains(&fence(raw, "text")));
            assert!(body.contains("Authority.java:1-2"));
            assert!(body.contains(&md("COMPILER_BOUND")));
            assert!(body.contains("{#retained-text}"));
            assert!(body.contains(&format!("/clew-data/objects/{}.json", &id[7..])));
            if language == "ru" {
                assert!(body.contains("Основание сведений: COMPILER"));
                assert!(body.contains("Версия: Revision"));
                assert!(body.contains("## Сохранённый текст"));
                assert!(!body.contains("## Retained text"));
            }
            assert_eq!(source, original);
        }
    }

    #[test]
    fn portable_declared_contract_keeps_schema_title_pointer_and_source_identity_in_both_languages()
    {
        for target in ["en", "ru"] {
            for available in [Some("en"), Some("ru"), None] {
                let contract = json!({"id":"event","title":"Properties","documentationLanguage":available,
                    "branchPointer":"/components/schemas/Event","leaves":[{"field":"id","requiredWhenBranchMatches":true,"nullable":false,"schema":{"type":"string","enum":["Properties"],"default":"Exact pointer"}}],
                    "source":{"text":"Original contract bytes","textDigest":"unchanged","revision":"r1","file":"event.json","startLine":1,"endLine":1}});
                let original = contract.clone();
                let gap = super::super::prose_language::gap(
                    &contract["documentationLanguage"],
                    Some(target),
                    "DECLARED_CONTRACT_TRANSLATION",
                    "contracts/event.original.json",
                )
                .unwrap();
                let body = declared_contract_page(&contract, gap.as_ref(), target).unwrap();
                assert!(body.contains("# Properties"));
                assert_eq!(
                    body.lines().filter(|line| line.starts_with("# ")).count(),
                    1
                );
                assert_eq!(body.matches("hide_title: true").count(), 1);
                assert!(body.contains("{#properties}"));
                assert!(body.contains("/components/schemas/Event"));
                assert!(body.contains(&format!(
                    "sources/{}.mdx",
                    &digest(&contract["source"]).unwrap()[7..]
                )));
                assert!(body.contains(&fence(
                    &serde_json::to_string_pretty(&contract["leaves"][0]["schema"]).unwrap(),
                    "json"
                )));
                assert_eq!(
                    body.contains("/clew-original/contracts/event.original.json"),
                    available != Some(target)
                );
                if target == "ru" {
                    assert!(body.contains("## Свойства"));
                    assert!(body.contains("Обязательно при совпадении ветви: true"));
                    assert!(body.contains("Допускает null: false"));
                    assert!(!body.contains("runtime conformance is not established"));
                }
                assert_eq!(contract, original);
            }
        }
    }

    #[test]
    fn portable_model_language_notice_keeps_original_prose_fields_and_technical_navigation() {
        let model = json!({"id":"record","title":"Overview","documentationLanguage":"en",
            "purpose":{"text":"View evidence","evidence":[]},"identity":{"text":"Original identity","evidence":[]},
            "fields":[{"id":"value","path":"value","meaning":{"text":"Original meaning","evidence":[]},"representations":[],"usage":[{}]}],
            "records":[],"processLinks":[{"subject":"scenario:example","label":"Original process","anchor":"question-apply"}],"relatedModels":[]});
        let gap = super::super::prose_language::gap(
            &model["documentationLanguage"],
            Some("ru"),
            "MODEL_TRANSLATION",
            "data-model/record.original.json",
        )
        .unwrap()
        .unwrap();
        let body = model_page(&model, &json!({"contracts":[]}), Some(&gap), "ru").unwrap();
        for original in [
            "Overview",
            "View evidence",
            "Original identity",
            "Original meaning",
            "Original process",
        ] {
            assert!(body.contains(original));
        }
        assert!(body.contains("Требуется перевод"));
        assert_eq!(
            body.lines().filter(|line| line.starts_with("# ")).count(),
            1
        );
        assert_eq!(body.matches("hide_title: true").count(), 1);
        assert!(body.contains("## Идентичность и связи"));
        assert!(body.contains("## Поля и представления"));
        assert!(body.contains("<summary>Основания и область</summary>"));
        assert!(!body.contains("## Identity and relationships"));
        assert!(body.contains("/clew-original/data-model/record.original.json"));
        assert!(body.contains("#field-value"));
        assert!(body.contains("#field-usage"));
        assert!(body.contains("../scenarios/example.mdx#question-apply"));
        assert!(body.contains("<ClewDataModel"));
        for anchor in [
            "identity-and-relationships",
            "fields-and-representations",
            "processes",
        ] {
            assert!(body.contains(&format!("{{#{anchor}}}")));
        }
        let matching = model_page(&model, &json!({"contracts":[]}), None, "en").unwrap();
        assert!(!matching.contains("Translation pending"));
        assert!(matching.contains("View evidence"));
    }

    #[test]
    fn reference_records_and_process_links_are_portable_and_keep_evidence() {
        let model = json!({"id":"lookup","title":"Lookup","purpose":{"text":"Supported values","evidence":[]},"identity":{"text":"Source literals","evidence":[]},"fields":[{"id":"id","path":"id","meaning":{"text":"Key","evidence":[]},"representations":[],"usage":[]}],"records":[{"values":{"id":"<literal>|value"},"claim":{"text":"Exact retained literal","evidence":[]}}],"processLinks":[],"relatedModels":[]});
        let body = model_page(&model, &json!({"contracts":[]}), None, "en").unwrap();
        assert!(body.contains("Reference records"));
        assert!(!body.contains("<literal>"));
        assert!(body.contains("#reference-record-1"));
        assert!(body.contains("contextOnly"));
        assert!(!body.contains("<ClewFieldTrace"));
        let page = json!({"questionReader":{"title":"Process","purpose":"Select","sections":[{"id":"select","title":"Select","number":1,"blocks":[{"text":"Existing explanation","evidence":[],"links":[]}]}]},"modelReferences":[{"id":"lookup","title":"Lookup","anchor":"question-select"}]});
        let process = process_page(&page, "example").unwrap();
        assert!(process.contains("Existing explanation"));
        assert!(process.contains("<ClewModelLink model=\"lookup\" process=\"example\" anchor=\"question-select\">Lookup</ClewModelLink>"));
    }
    #[test]
    fn sequence_preserves_nested_branches_returns_and_participants_without_diagram_notes() {
        let op = json!({"participants":[{"id":"caller","label":"Caller"},{"id":"service","label":"Service"}],"events":[
            {"kind":"alt","text":"accepted"},{"kind":"loop","text":"each item"},
            {"kind":"message","from":"caller","to":"service","text":"post <body>; safely"},
            {"kind":"return","from":"service","to":"caller","text":"200"},
            {"kind":"end"},{"kind":"else","text":"rejected"},
            {"kind":"note","text":"Keep this only in the original account"},{"kind":"end"}
        ]});
        let diagram = sequence(&op).unwrap().unwrap();
        assert!(diagram.contains("participant p0 as Caller"));
        assert!(diagram.contains("alt accepted\nloop each item\np0->>p1: post #60;body#62;#59; safely\np1-->>p0: 200\nend\nend"));
        assert!(!diagram.contains("else rejected"));
        assert!(!diagram.contains("Keep this"));
        assert!(
            sequence(&json!({"events":[{"kind":"message","from":"a","to":"b"},{"kind":"end"}]}))
                .is_err()
        );
    }
    #[test]
    fn sequence_removes_empty_groups_and_unused_actors_without_losing_else_conditions() {
        let op = json!({"participants":[{"id":"caller","label":"Unused caller"},{"id":"service","label":"Service"},{"id":"db","label":"Database"}],"events":[
            {"kind":"alt","text":"Duplicate event","sourceIds":["duplicate"]},
            {"kind":"note","text":"Return without changing the card"},
            {"kind":"else","text":"Otherwise","sourceIds":["new-event"]},
            {"kind":"message","from":"service","to":"db","text":"Insert or update the card"},
            {"kind":"alt","text":"Changed row"},{"kind":"note","text":"Applied"},
            {"kind":"else","text":"Otherwise"},{"kind":"note","text":"Older or equal"},
            {"kind":"end"},{"kind":"end"},
            {"kind":"declared","from":"service","to":"db","text":"Declared transfer only"}
        ]});
        let projected = sequence_events(&op).unwrap();
        assert_eq!(projected.len(), 4);
        assert_eq!(projected[0]["text"], "Condition not met: Duplicate event");
        assert_eq!(projected[0]["sourceIds"], json!(["duplicate", "new-event"]));
        let diagram = sequence(&op).unwrap().unwrap();
        assert!(!diagram.contains("Unused caller"));
        assert!(!diagram.contains("Changed row"));
        assert!(diagram.contains("alt Condition not met: Duplicate event\np0->>p1: Insert or update the card\nend\np0-->>p1: [declared] Declared transfer only"));
    }
    #[test]
    fn service_content_and_interface_rows_are_readable_without_opening_the_json_archive() {
        let op = json!({"id":"section-ingress","summary":{"text":"Actual saved ingress account."},"interfaceContracts":[{"title":"Declared statuses","rows":[{"label":"Responses","value":"400, 415, 500 and 503 retain independent schemas."}]}]});
        let page = json!({"title":"Service","sections":[{"id":"section-ingress","title":"Ingress","content":op}],"catalogue":[],"operations":[]});
        let rendered = service_page(&page, "service").unwrap();
        assert!(rendered.contains("Actual saved ingress account."));
        assert!(
            rendered.contains("| Responses | 400, 415, 500 and 503 retain independent schemas. |")
        );
        assert!(rendered.contains("UNVERIFIED · UNASSESSED"));
    }
    #[test]
    fn overview_keeps_the_system_guide_and_uses_root_markdown_file_links() {
        let guide = json!({"title":"Saved system","purpose":"Real system purpose.","sections":[{"id":"story","title":"System story","blocks":[{"title":"Save","text":"Real saved outcome.","links":[{"subject":"scenario:save","operation":"process-overview","label":"Follow save"}],"evidence":[]}]}],"conclusion":{"text":"No delivery guarantee.","evidence":[]}});
        let native = BTreeMap::from([(
            "system-reading-guide.json".into(),
            serde_json::to_vec(&guide).unwrap(),
        )]);
        let inputs = super::super::full::ReadingInputs {
            language: None,
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
            format: super::super::PageFormat::Docusaurus,
        };
        let (files, manifest) =
            project(&native, &json!({"output":"machine-only"}), &inputs).unwrap();
        let overview = std::str::from_utf8(&files["docs/overview.mdx"]).unwrap();
        assert!(overview.contains("Real saved outcome."));
        assert!(overview.contains("[Follow save](scenarios/save.mdx#process-overview)"));
        assert_eq!(manifest["output"], ".");
    }
    #[test]
    fn revisions_ranges_and_literal_reference_keys_round_trip_without_source_aliasing() {
        let source = json!({"id":"same","file":"a.cs","revision":"old","startLine":1,"endLine":2,"text":"a\r\nλ","textDigest":"saved","authority":"EXACT_SNAPSHOT_TEXT"});
        let mut new = source.clone();
        new["revision"] = json!("new");
        let mut range = source.clone();
        range["startLine"] = json!(3);
        let input = json!({"a":source,"b":source,"new":new,"range":range,"literal":{REF:digest(&source).unwrap()},"nested":{LITERAL:{REF:"literal"}}});
        let mut objects = Objects::default();
        objects.count(&input).unwrap();
        let value = objects.normalize(&input).unwrap();
        assert_eq!(objects.sources.len(), 3);
        assert_eq!(objects.expand(&value, 0).unwrap(), input);
        let again = objects.normalize(&input).unwrap();
        assert_eq!(value, again);
    }
    #[test]
    fn repeated_provenance_is_shared_and_missing_or_changed_objects_fail_resolution() {
        let context = json!({"revision":"old","coverage":"SYNTAX","limits":"bounded source only ".repeat(50)});
        let input = json!([{"context":context},{"context":context}]);
        let mut objects = Objects::default();
        objects.count(&input).unwrap();
        let value = objects.normalize(&input).unwrap();
        assert!(!objects.values.is_empty());
        assert_eq!(objects.expand(&value, 0).unwrap(), input);
        let id = objects.values.keys().next().unwrap().clone();
        objects.values.insert(id, json!({"changed":true}));
        assert!(objects.expand(&value, 0).is_err());
    }
    #[test]
    fn process_entry_precedes_title_and_controls_while_step_content_stays_in_the_panel() {
        let page = json!({"questionReader":{"title":"Save","purpose":"Saved purpose.","sections":[]},"processDataFlow":{"request":{"anchor":"question-save"}}});
        let mdx = process_page(&page, "save").unwrap();
        let start = mdx.find("<ClewProcess ").unwrap();
        let overview = mdx
            .find("<ClewAnchor id=\"process-overview\" pageStart />")
            .unwrap();
        let title = mdx.find("# Save\n").unwrap();
        let end = mdx.find("</ClewProcess>").unwrap();
        assert!(overview < title && title < start && start < end);
        assert!(mdx[start..end].contains("Saved purpose."));
        let plain =
            process_page(&json!({"questionReader":page["questionReader"]}), "save").unwrap();
        assert!(plain.contains("Saved purpose."));
        assert!(!plain.contains("<ClewProcess"));
    }
    #[test]
    fn source_fences_and_editorial_text_cannot_become_executable_mdx() {
        let value = "import x from 'x'\n<script>{unsafe()}</script>\n```";
        assert!(md(value).starts_with("&#105;mport"));
        assert!(md(value).contains("&#123;unsafe()&#125;"));
        assert!(fence(value, "text").starts_with("````text\n"));
    }
    #[test]
    fn closed_context_summary_keeps_authored_preview_and_separates_adjacent_links() {
        let preview = "Compose POST http://places:8080/places · production URL unknown.";
        let fragment = json!({"title":"Technical context","disclosure":"Address and contracts","preview":preview,"text":"Full retained address explanation.","links":[{"subject":"service:dashboard","operation":"save","label":"Dashboard entry and contract"},{"subject":"service:places","operation":"save","label":"Places handler and contract"}],"evidence":[]});
        let rendered = block(&fragment, &json!({"subject":"scenario:save"}), "request").unwrap();
        let summary = &rendered[..rendered.find("</summary>").unwrap()];
        assert!(summary.contains("Address and contracts"));
        assert!(summary.contains(preview));
        assert!(rendered.contains("Full retained address explanation."));
        assert!(rendered.contains("<div className=\"clew-context-links\">"));
        assert_eq!(rendered.matches("<ClewContextLink ").count(), 2);
        assert!(rendered.contains("fromQuestion=\"question-request\""));
    }
}
