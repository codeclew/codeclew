//! A single semantic content tree is emitted as inert JSX and static HTML.
use super::model::*;
use crate::documentation::{digest, invalid, io_error, store};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            '@' | '{' | '}' | '\n' | '\r' | '\t' | '*' | '_' | '[' | ']' | '`' | '\\' => {
                format!("&#{};", c as u32)
            }
            _ => c.to_string(),
        })
        .collect()
}
fn anchor(id: &str) -> String {
    format!(
        "ref-{}",
        crate::canonical::hash_bytes(id.as_bytes()).trim_start_matches("sha256:")
    )
}
fn paragraph(text: &str) -> String {
    format!("<p>{}</p>\n", escape(text))
}
fn link(path: &str, label: &str) -> String {
    format!("<a href=\"{}\">{}</a>", escape(path), escape(label))
}
fn cite(id: &str, ext: &str) -> String {
    link(&format!("sources.{ext}#{}", anchor(id)), "Retained source")
}
fn gaps(rows: &[Gap], ext: &str) -> String {
    rows.iter()
        .map(|g| {
            format!(
                "<li>{}: {} {}</li>\n",
                escape(&g.code),
                escape(&g.detail),
                g.citation_id
                    .as_deref()
                    .map(|id| cite(id, ext))
                    .unwrap_or_default()
            )
        })
        .collect::<String>()
}
fn conditions(rows: &[PathCondition], ext: &str) -> String {
    rows.iter()
        .map(|p| {
            format!(
                "<li>{} = {} {}</li>\n",
                escape(&p.expression),
                p.holds,
                cite(&p.citation_id, ext)
            )
        })
        .collect()
}
fn statement(s: &Statement, ext: &str) -> String {
    let mut out = format!(
        "<li><pre>{}</pre><p>Kind: {:?}; reachable in retained source path: {}. {}</p>\n",
        escape(&s.expression),
        s.kind,
        s.reachable,
        cite(&s.citation_id, ext)
    );
    if !s.conditions.is_empty() {
        out += &format!("<ul>{}</ul>\n", conditions(&s.conditions, ext));
    }
    for call in &s.calls {
        out += &paragraph(&format!("Call phase: {}.", call.phase));
        out += &format!("<p>{}</p>\n", cite(&call.citation_id, ext));
        out += &format!(
            "<details><summary>Exact call identity</summary>{}</details>\n",
            paragraph(&format!(
                "Expression: {}. Target: {}. Authority: {}. Relation: {}.",
                call.expression,
                call.target.as_deref().unwrap_or("unresolved"),
                call.authority,
                call.relation_id.as_deref().unwrap_or("unresolved")
            ))
        );
        if let Some(b) = &call.external_boundary {
            out += &paragraph(&format!(
                "External boundary: {}. Source status: {}. {}",
                b.target, b.source_status, b.limitation
            ));
            for id in &b.citation_ids {
                out += &format!("<p>{}</p>\n", cite(id, ext));
            }
        }
        out += &format!("<ul>{}</ul>\n", gaps(&call.gaps, ext));
    }
    out += &format!("<ul>{}</ul>\n", gaps(&s.gaps, ext));
    if !s.children.is_empty() {
        out += &format!(
            "<p>Body / true alternative</p><ol>{}</ol>\n",
            s.children
                .iter()
                .map(|s| statement(s, ext))
                .collect::<String>()
        );
    }
    if !s.alternative.is_empty() {
        out += &format!(
            "<p>False alternative</p><ol>{}</ol>\n",
            s.alternative
                .iter()
                .map(|s| statement(s, ext))
                .collect::<String>()
        );
    }
    out += "</li>\n";
    out
}
fn callable_label(symbol: &str) -> String {
    symbol
        .strip_prefix("method:class:")
        .or_else(|| symbol.strip_prefix("method:"))
        .unwrap_or(symbol)
        .split('(')
        .next()
        .unwrap_or(symbol)
        .replace('#', ".")
}
fn process_label(p: &PageContent) -> String {
    format!(
        "{} → {}",
        callable_label(&p.endpoint.symbol),
        callable_label(&p.worker.symbol)
    )
}
fn citation_group(p: &PageContent, ids: &[String], ext: &str) -> String {
    if ids.is_empty() {
        return String::new();
    }
    format!(
        "<p>Sources: {}</p>\n",
        ids.iter()
            .map(|id| {
                let label = p
                    .citations
                    .get(id)
                    .map(|c| {
                        format!(
                            "{}:{}–{}",
                            c.file.rsplit('/').next().unwrap_or(&c.file),
                            c.start_line,
                            c.end_line
                        )
                    })
                    .unwrap_or_else(|| "Source".into());
                link(&format!("sources.{ext}#{}", anchor(id)), &label)
            })
            .collect::<Vec<_>>()
            .join(" · ")
    )
}
fn attempt_summary(callable: &CallableProjection, ext: &str) -> String {
    fn collect<'a>(steps: &'a [Statement], out: &mut Vec<&'a CallProjection>) {
        for step in steps {
            if step.reachable {
                out.extend(&step.calls);
            }
            collect(&step.children, out);
            collect(&step.alternative, out);
        }
    }
    let mut calls = Vec::new();
    collect(&callable.steps, &mut calls);
    calls
        .into_iter()
        .filter(|c| {
            matches!(
                c.phase.as_str(),
                "SUBMISSION_ATTEMPT" | "CONSUMPTION_ATTEMPT" | "DEPENDENCY_CALL_ATTEMPT"
            )
        })
        .map(|c| {
            format!(
                "<li><pre>{}</pre>{}</li>\n",
                escape(&c.expression),
                cite(&c.citation_id, ext)
            )
        })
        .collect()
}
fn callable(c: &CallableProjection, ext: &str) -> String {
    let mut out = paragraph(&callable_label(&c.symbol));
    if let Some(id) = &c.citation_id {
        out += &format!("<p>{}</p>\n", cite(id, ext));
    }
    out += &format!(
        "<ol>{}</ol><ul>{}</ul>\n",
        c.steps
            .iter()
            .map(|s| statement(s, ext))
            .collect::<String>(),
        gaps(&c.gaps, ext)
    );
    out += &format!(
        "<details><summary>Exact declaration identity</summary>{}</details>\n",
        paragraph(&format!(
            "Declaration: {}. Symbol: {}. Authority: {}.",
            c.declaration_id, c.symbol, c.authority
        ))
    );
    out
}
fn state(c: &CallableProjection, ext: &str) -> String {
    let mut out = format!(
        "<h2>{}</h2><table tabIndex=\"0\"><caption>Scroll horizontally: fields and state</caption><thead><tr><th scope=\"col\">Name</th><th scope=\"col\">Expression</th><th scope=\"col\">Kind</th><th scope=\"col\">Condition path</th><th scope=\"col\">Source</th></tr></thead><tbody>\n",
        escape(&callable_label(&c.symbol))
    );
    for row in &c.state {
        out += &format!(
            "<tr><td>{}</td><td><pre>{}</pre></td><td>{}</td><td><ul>{}</ul></td><td>{}</td></tr>\n",
            escape(&row.name),
            escape(&row.expression),
            escape(&row.kind),
            conditions(&row.conditions, ext),
            cite(&row.citation_id, ext)
        );
    }
    out += "</tbody></table>\n";
    out
}
fn operational_instructions(rows: &[HumanInstruction]) -> String {
    let mut out =
        "<section id=\"operational-instructions\"><h2>Operational instructions</h2>\n".to_string();
    out += &paragraph(
        "Captured human/imported instructions are unverified. Source-claim status: UNASSESSED; their content does not establish source or runtime behavior.",
    );
    for note in rows {
        out += &format!("<article><h3>{}</h3>\n", escape(&note.title));
        out += &paragraph(&format!(
            "Declared author: {}. Classification: {}. Period: {}. Authority: {}. Source-claim status: {}.",
            note.declared_author,
            note.classification,
            note.period,
            note.authority,
            note.source_claim_status
        ));
        out += &format!("<pre>{}</pre>\n", escape(&note.text));
        out += &format!(
            "<details><summary>Captured note identity</summary>{}</details></article>\n",
            paragraph(&format!(
                "Note ID: {}. Version digest: {}. Content digest: {}. Association digest: {}.",
                note.id, note.version_digest, note.content_digest, note.association_digest
            ))
        );
    }
    out += "</section>\n";
    out
}

const VIEWS: &[(&str, &str)] = &[
    ("overview", "Process overview"),
    ("endpoint", "Endpoint path"),
    ("worker", "Worker path"),
    ("fields-state", "Fields and state"),
    ("diagnostic", "Diagnostic matrix"),
];
fn authored_source_anchor(paragraph: &AuthoredParagraph, id: &str) -> String {
    // Presentation anchors name the existing frozen tuple, not a new SOURCE ID.
    anchor(
        &serde_json::json!([
            "authored-source",
            paragraph.selection,
            paragraph.paragraph_digest,
            id
        ])
        .to_string(),
    )
}
fn authored_paragraphs(rows: &[AuthoredParagraph], ext: &str) -> String {
    let mut out =
        "<section id=\"authored-paragraphs\"><h2>Maintained user documentation</h2>\n".to_owned();
    for row in rows {
        let authorship = row.paragraph.authorship.as_ref().unwrap();
        out += &format!("<article><pre>{}</pre>\n", escape(&row.paragraph.text));
        out += &paragraph(&format!(
            "Declared author: {}. Authority: USER_DOCUMENTATION. Meaning review: UNASSESSED. Originally linked code is retained unverified context. Source context freshness: {}.",
            authorship.author, row.context_freshness
        ));
        out += "<ul>\n";
        for id in &row.paragraph.source_ids {
            out += &format!(
                "<li>{}</li>\n",
                link(
                    &format!("sources.{ext}#{}", authored_source_anchor(row, id)),
                    &format!("Originally linked source {id}")
                )
            );
        }
        out += "</ul><details><summary>Frozen paragraph identity</summary>\n";
        out += &paragraph(&format!(
            "Bundle: {}. Operation: {}. Fragment: {}. Paragraph digest: {}. Operation digest: {}. Bindings digest: {}. Publication digest: {}. Original source snapshot: {}. Context role: RETAINED_UNVERIFIED_CONTEXT.",
            row.selection.bundle,
            row.selection.operation,
            row.selection.fragment,
            row.paragraph_digest,
            row.operation_digest,
            row.bindings_digest,
            row.publication_digest,
            authorship.source_snapshot
        ));
        out += "</details></article>\n";
    }
    out += "</section>\n";
    out
}
fn page(p: &PageContent, view: &str, title: &str, snapshot: &str, ext: &str) -> String {
    let mut out = format!(
        "<main><h1>{}: {}</h1>\n<nav aria-label=\"Documentation pages\">{}",
        escape(&process_label(p)),
        escape(title),
        link(&format!("index.{ext}"), "All processes")
    );
    for (slug, label) in VIEWS {
        out += &format!(" · {}", link(&format!("{}-{slug}.{ext}", p.id), label));
    }
    out += &format!(
        " · {}</nav>\n",
        link(&format!("sources.{ext}"), "Source appendix")
    );
    match view {
        "overview" => {
            if p.handoff.status == "SOURCE_DECLARED_SHARED_QUEUE" {
                out += &paragraph(&format!(
                    "Source binds {} and {} to the same queue allocation.",
                    p.handoff
                        .endpoint_field
                        .as_deref()
                        .unwrap_or("endpoint field"),
                    p.handoff.worker_field.as_deref().unwrap_or("worker field")
                ));
            } else {
                out += &paragraph("Shared queue wiring unresolved.");
                out += &format!("<ul>{}</ul>\n", gaps(&p.handoff.gaps, ext));
            }
            out += &paragraph(&p.handoff.limitation);
            out += &citation_group(p, &p.handoff.citation_ids, ext);
            out += "<h2>Source call attempts</h2><ul>\n";
            out += &attempt_summary(&p.endpoint, ext);
            out += &attempt_summary(&p.worker, ext);
            out += "</ul>\n";
            let mut seen = BTreeSet::new();
            let guards: Vec<_> = p
                .diagnostics
                .iter()
                .filter(|d| seen.insert(&d.condition))
                .take(4)
                .collect();
            if !guards.is_empty() {
                out += "<h2>Worker guard examples</h2><ul>\n";
                for diagnostic in guards {
                    out += &format!("<li>{}</li>\n", escape(&diagnostic.condition));
                }
                out += &format!(
                    "</ul><p>{}</p>\n",
                    link(
                        &format!("{}-diagnostic.{ext}", p.id),
                        "All diagnostic conditions and inspection points"
                    )
                );
            }
            out += &format!(
                "<details><summary>Exact handoff identity</summary>{}</details>\n",
                paragraph(&format!(
                    "Handoff status: {}. Queue allocation: {}. Endpoint field: {}. Worker field: {}.",
                    p.handoff.status,
                    p.handoff
                        .queue_allocation
                        .as_deref()
                        .unwrap_or("unresolved"),
                    p.handoff.endpoint_field.as_deref().unwrap_or("unresolved"),
                    p.handoff.worker_field.as_deref().unwrap_or("unresolved")
                ))
            );
            if let Some(c) = &p.wiring {
                out += "<h2>Selected wiring declaration</h2>\n";
                out += &callable(c, ext);
            }
        }
        "endpoint" => out += &callable(&p.endpoint, ext),
        "worker" => out += &callable(&p.worker, ext),
        "fields-state" => {
            out += &state(&p.endpoint, ext);
            out += &state(&p.worker, ext);
            if let Some(c) = &p.wiring {
                out += &state(c, ext);
            }
        }
        "diagnostic" => {
            if let Some(q) = &p.selection.question {
                out += &paragraph(&format!("Question: {q}"));
            }
            out += &paragraph(
                "Inspect rows describe source-derived possible reasons; they do not diagnose an observed incident.",
            );
            out += "<table tabIndex=\"0\"><caption>Scroll horizontally: source condition diagnostic matrix</caption><thead><tr><th scope=\"col\">Condition</th><th scope=\"col\">Possible reason</th><th scope=\"col\">Inspect</th><th scope=\"col\">Selected call</th><th scope=\"col\">Source</th></tr></thead><tbody>\n";
            for d in &p.diagnostics {
                out += &format!(
                    "<tr><td><pre>{}</pre></td><td>{}</td><td>{}</td><td><pre>{}</pre></td><td>{}</td></tr>\n",
                    escape(&d.condition),
                    escape(&d.possible_reason),
                    escape(&d.inspect.join("; ")),
                    escape(&d.selected_call),
                    d.citation_ids
                        .iter()
                        .map(|id| cite(id, ext))
                        .collect::<Vec<_>>()
                        .join(" · ")
                );
            }
            out += "</tbody></table>\n";
        }
        _ => unreachable!(),
    }
    if !p.human_instructions.is_empty() {
        if view == "overview" {
            out += &operational_instructions(&p.human_instructions);
        } else {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("{}-overview.{ext}#operational-instructions", p.id),
                    "Operational instructions and captured note identities"
                )
            );
        }
    }
    if !p.authored_paragraphs.is_empty() {
        if view == "overview" {
            out += &authored_paragraphs(&p.authored_paragraphs, ext);
        } else {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("{}-overview.{ext}#authored-paragraphs", p.id),
                    "Maintained user documentation and original source context"
                )
            );
        }
    }
    out += &format!(
        "<details><summary>Sources and version</summary>{}</details>\n",
        paragraph(&format!(
            "Service: {}. Revision: {}. Snapshot: {}. Service digest: {}. Endpoint symbol: {}. Worker symbol: {}.",
            p.selection.service,
            p.service_revision,
            snapshot,
            p.service_digest,
            p.endpoint.symbol,
            p.worker.symbol
        ))
    );
    out += &format!(
        "<details><summary>Retained limitations</summary><ul>{}</ul></details><p>{}</p></main>\n",
        gaps(&p.limitations, ext),
        link(
            "projection.json",
            "Complete typed projection and retained evidence"
        )
    );
    out
}
fn appendix(p: &BundleProjection, ext: &str) -> String {
    let mut out = format!(
        "<main><h1>Retained source appendix</h1><p>{}</p>\n",
        link(&format!("index.{ext}"), "All processes")
    );
    let mut citations = BTreeMap::new();
    for page in &p.pages {
        for (id, c) in &page.citations {
            citations.entry(id).or_insert((page, c));
        }
    }
    for (id, (page, c)) in citations {
        out += &format!(
            "<section id=\"{}\"><h2>{}:{}–{}</h2>\n",
            anchor(id),
            escape(&c.file),
            c.start_line,
            c.end_line
        );
        out += &paragraph(&format!(
            "Citation: {}. Service: {}. Revision: {}. Source ID: {}. Authority: {}. Text digest: {}. Evidence digest: {}. Retained byte range: {}–{}.",
            c.id,
            c.service,
            c.revision,
            c.source_id,
            c.authority,
            c.text_digest,
            c.evidence_digest,
            c.start_byte,
            c.end_byte
        ));
        if let Some(url) = &c.url {
            if url.starts_with("https://") || url.starts_with("http://") {
                out += &format!("<p>{}</p>\n", link(url, "Original source anchor"));
            } else {
                out += &paragraph(&format!("Original source anchor: {url}"));
            }
        } else {
            out += &paragraph(
                "Original source URL unavailable; retained source anchor is local to this appendix.",
            );
        }
        if let Some(source) = page.sources.get(&c.source_id) {
            // UTF-8 bounds are checked by the projector; use a checked slice here too.
            if let Some(text) = source.text.get(c.start_byte..c.end_byte) {
                out += &format!("<pre>{}</pre>\n", escape(text));
            }
        }
        out += "</section>\n";
    }
    // Preserve complete source snippets as well as individual citation ranges.
    let mut sources = BTreeMap::new();
    for page in &p.pages {
        for source in page.sources.values() {
            sources
                .entry((&source.service, &source.id))
                .or_insert(source);
        }
    }
    for source in sources.values() {
        out += &format!(
            "<details id=\"{}\"><summary>{}:{}–{}</summary><p>{}</p><pre>{}</pre></details>\n",
            anchor(&format!("source:{}:{}", source.service, source.id)),
            escape(&source.file),
            source.start_line,
            source.end_line,
            escape(&format!(
                "Revision: {}. Authority: {}. Text digest: {}. Evidence digest: {}.",
                source.revision, source.authority, source.text_digest, source.evidence_digest
            )),
            escape(&source.text)
        );
    }
    let mut shown = BTreeSet::new();
    for page in &p.pages {
        for paragraph in &page.authored_paragraphs {
            for (id, source) in &paragraph.source_records {
                let local_anchor = authored_source_anchor(paragraph, id);
                if !shown.insert(local_anchor.clone()) {
                    continue;
                }
                out += &format!(
                    "<section id=\"{}\"><h2>Original authored paragraph context: {}:{}–{}</h2>\n",
                    local_anchor,
                    escape(&source.file),
                    source.start_line,
                    source.end_line
                );
                out += &self::paragraph(&format!(
                    "Logical SOURCE ID: {}. Bundle: {}. Operation: {}. Fragment: {}. Revision: {}. Source authority: {}. Text digest: {}. Evidence digest: {}. This source context does not verify the authored prose.",
                    id,
                    paragraph.selection.bundle,
                    paragraph.selection.operation,
                    paragraph.selection.fragment,
                    source.revision,
                    source.authority,
                    source.text_digest,
                    source.evidence_digest
                ));
                out += &format!("<pre>{}</pre></section>\n", escape(&source.text));
            }
        }
    }
    out += "</main>\n";
    out
}
fn html(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"></head><body>\n{body}</body></html>\n",
        escape(title)
    )
}
const CSS: &str = "html{color:#172d49;background:#f7f4ec;font:16px/1.6 system-ui,sans-serif}main{max-width:1080px;margin:auto;padding:32px 24px;overflow-wrap:anywhere}a{color:#245d97}nav{margin:20px 0}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#edf0f2;padding:12px}table{border-collapse:collapse;width:100%;display:block;overflow-x:auto}table:focus-visible{outline:2px solid #245d97;outline-offset:3px}caption{text-align:left;font-weight:600}th,td{min-width:160px;padding:8px;border:1px solid #c7d0da;text-align:left;vertical-align:top}th:nth-child(3),td:nth-child(3){min-width:180px}th:nth-child(4),td:nth-child(4){min-width:220px}section,details{border-top:1px solid #c7d0da;padding:12px 0}h1{line-height:1.2}@media(max-width:600px){main{padding:20px 16px}h1{font-size:28px}}\n";
pub(super) fn write(
    output: &Path,
    snapshot: &str,
    p: &BundleProjection,
) -> Result<Value, ClewError> {
    // Portable preflight before constructing or creating any output files.
    let mut names = BTreeSet::new();
    for page in &p.pages {
        if !store::valid_id(&page.id) || !names.insert(page.id.to_ascii_lowercase()) {
            return Err(invalid(
                "native page IDs must be safe and unique ignoring ASCII case",
            ));
        }
    }
    let mut files = BTreeMap::new();
    let mut page_rows = Vec::new();
    for ext in ["html", "mdx"] {
        let mut bodies = BTreeMap::new();
        let mut index = "<main><h1>Native source documentation</h1><ul>\n".to_string();
        for page_content in &p.pages {
            if !store::valid_id(&page_content.id) {
                return Err(invalid("native page ID is invalid"));
            }
            index += &format!(
                "<li>{}</li>\n",
                link(
                    &format!("{}-overview.{ext}", page_content.id),
                    &process_label(page_content)
                )
            );
            for (slug, title) in VIEWS {
                bodies.insert(
                    format!("{}-{slug}", page_content.id),
                    (
                        title.to_string(),
                        page(page_content, slug, title, snapshot, ext),
                    ),
                );
            }
        }
        index += &format!(
            "</ul><p>{} · {}</p>",
            link(&format!("sources.{ext}"), "Shared source appendix"),
            link("projection.json", "Typed projection")
        );
        index += &format!(
            "<details><summary>Sources and version</summary>{}</details></main>\n",
            paragraph(&format!(
                "Snapshot: {snapshot}. Input digest: {}. Context digest: {}. Selection digest: {}.",
                p.input_digest, p.context_digest, p.selection_digest
            ))
        );
        bodies.insert(
            "index".into(),
            ("Native source documentation".into(), index),
        );
        bodies.insert(
            "sources".into(),
            ("Retained source appendix".into(), appendix(p, ext)),
        );
        for (slug, (title, body)) in bodies {
            // Scalar and source newlines are already escaped. These remaining
            // newlines only format native JSX and must not start MDX paragraphs.
            let body = body.replace('\n', "");
            if ext == "html" {
                page_rows.push(json!({"id":slug,"contentDigest":digest(&(p, &slug))?,"html":format!("{slug}.html"),"mdx":format!("{slug}.mdx")}));
            }
            let bytes = if ext == "html" {
                html(&title, &body).into_bytes()
            } else {
                body.into_bytes()
            };
            files.insert(format!("{slug}.{ext}"), bytes);
        }
    }
    files.insert("style.css".into(), CSS.as_bytes().to_vec());
    files.insert(
        "projection.json".into(),
        serde_json::to_vec_pretty(p).map_err(io_error)?,
    );
    let mut manifest = json!({"schema":"codeclew-native-pages-static-manifest/1.0", "snapshot":snapshot,
        "inputDigest":p.input_digest, "contextDigest":p.context_digest, "selectionDigest":p.selection_digest,
        "projectionDigest":digest(p)?, "mdxProfile":"MDX 3; inert native JSX; no imports, executable expressions, scripts or network dependencies",
        "pages":page_rows,
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"digest":crate::canonical::hash_bytes(bytes)})).collect::<Vec<_>>()});
    let selected_notes: Vec<_> = p
        .pages
        .iter()
        .flat_map(|page| {
            page.human_instructions.iter().map(move |note| json!({
            "pageId":format!("{}-overview", page.id),"processId":page.id,
            "html":format!("{}-overview.html", page.id),"mdx":format!("{}-overview.mdx", page.id),
            "noteId":note.id,"versionDigest":note.version_digest,
            "contentDigest":note.content_digest,"associationDigest":note.association_digest
        }))
        })
        .collect();
    if !selected_notes.is_empty() {
        manifest["selectedNotes"] = json!(selected_notes);
    }
    let selected_authored: Vec<_> = p.pages.iter().flat_map(|page| {
        page.authored_paragraphs.iter().map(move |row| json!({
            "processId":page.id, "html":format!("{}-overview.html", page.id), "mdx":format!("{}-overview.mdx", page.id),
            "bundle":row.selection.bundle, "operation":row.selection.operation, "fragment":row.selection.fragment,
            "paragraphDigest":row.paragraph_digest, "bindingsDigest":row.bindings_digest,
            "publicationDigest":row.publication_digest, "operationDigest":row.operation_digest,
            "sourceSnapshot":row.paragraph.authorship.as_ref().unwrap().source_snapshot
        }))
    }).collect();
    if !selected_authored.is_empty() {
        manifest["selectedAuthoredParagraphs"] = json!(selected_authored);
    }
    files.insert(
        "manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).map_err(io_error)?,
    );
    if output.exists() {
        let m = fs::symlink_metadata(output).map_err(io_error)?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(invalid("native pages output must be a regular directory"));
        }
        if fs::read_dir(output).map_err(io_error)?.next().is_some() {
            return Err(invalid(
                "native pages output must be empty; choose a new directory",
            ));
        }
    } else {
        fs::create_dir_all(output).map_err(io_error)?;
    }
    for name in files.keys() {
        store::relative(name)?;
        if fs::symlink_metadata(output.join(name)).is_ok() {
            return Err(invalid("native pages generated filename already exists"));
        }
    }
    for (name, data) in files {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))
            .map_err(io_error)?
            .write_all(&data)
            .map_err(io_error)?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mdx_source_text_is_inert_and_common_body_links_resolve_by_format() {
        let input = "@EXT@ {evil()} <script> import x from 'evil' `tick` [x] \\ * _\n";
        let text = escape(input);
        assert!(!text.contains('{') && !text.contains('<') && !text.contains('`'));
        assert!(text.contains("&#123;evil()&#125;"));
        assert!(text.contains("&lt;script&gt;"));
        assert!(text.contains("&#64;EXT&#64;"));
        let p = BundleProjection {
            schema: SCHEMA.into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            selection_digest: "selection".into(),
            pages: vec![],
        };
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("bundle");
        let manifest = write(&out, "snapshot/1", &p).unwrap();
        for row in manifest["files"].as_array().unwrap() {
            let bytes = fs::read(out.join(row["path"].as_str().unwrap())).unwrap();
            assert_eq!(crate::canonical::hash_bytes(&bytes), row["digest"]);
        }
        let mdx = fs::read_to_string(out.join("index.mdx")).unwrap();
        let html = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(mdx.contains("sources.mdx"));
        assert!(html.contains("sources.html"));
        assert!(!mdx.contains("@EXT@"));
        assert!(write(&out, "snapshot/1", &p).is_err());
        assert_eq!(fs::read_to_string(out.join("index.mdx")).unwrap(), mdx);
    }
    #[test]
    fn operational_note_text_and_metadata_are_inert_without_rewriting_crlf() {
        let input = "café☕\r\n<Panel>{probe()}</Panel> `code`\r\n";
        let note = HumanInstruction {
            id: "operations".into(),
            title: "<Title>{x}".into(),
            declared_author: "<Author>{y}".into(),
            classification: "policy".into(),
            period: "2026".into(),
            version_digest: "version".into(),
            content_digest: "content".into(),
            association_digest: "association".into(),
            text: input.into(),
            authority: "HUMAN_OR_IMPORTED_UNVERIFIED".into(),
            source_claim_status: "UNASSESSED".into(),
            association: json!({}),
        };
        let body = operational_instructions(&[note]);
        assert!(body.contains("café☕&#13;&#10;&lt;Panel&gt;&#123;probe()&#125;&lt;/Panel&gt;"));
        assert!(body.contains("Declared author: &lt;Author&gt;&#123;y&#125;"));
        assert!(
            body.contains("Note ID: operations. Version digest: version. Content digest: content.")
        );
        assert!(
            body.contains("HUMAN&#95;OR&#95;IMPORTED&#95;UNVERIFIED")
                && body.contains("UNASSESSED")
        );
        assert!(!body.contains("<Panel>") && !body.contains('{') && !body.contains('`'));
    }
    #[test]
    fn case_colliding_page_ids_refuse_before_touching_output() {
        let callable = CallableProjection {
            declaration_id: "decl".into(),
            symbol: "Example.run".into(),
            authority: "SOURCE".into(),
            citation_id: None,
            steps: vec![],
            state: vec![],
            gaps: vec![],
        };
        let make = |id: &str| PageContent {
            id: id.into(),
            title: id.into(),
            selection: Selection {
                id: id.into(),
                service: "service".into(),
                endpoint_declaration: "endpoint".into(),
                worker_declaration: "worker".into(),
                wiring_declaration: None,
                question: None,
                note_ids: vec![],
                authored_paragraphs: vec![],
            },
            service_revision: "revision".into(),
            service_digest: "digest".into(),
            endpoint: callable.clone(),
            worker: callable.clone(),
            wiring: None,
            handoff: HandoffProjection {
                status: "LOCAL_GAP".into(),
                queue_allocation: None,
                endpoint_field: None,
                worker_field: None,
                citation_ids: vec![],
                gaps: vec![],
                limitation: "Unproved".into(),
            },
            diagnostics: vec![],
            citations: BTreeMap::new(),
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            limitations: vec![],
            human_instructions: vec![],
            authored_paragraphs: vec![],
        };
        let p = BundleProjection {
            schema: SCHEMA.into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            selection_digest: "selection".into(),
            pages: vec![make("alpha-flow"), make("Alpha-flow")],
        };
        let temp = tempfile::tempdir().unwrap();
        let absent = temp.path().join("new");
        assert!(write(&absent, "snapshot/1", &p).is_err());
        assert!(!absent.exists());
        let empty = temp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert!(write(&empty, "snapshot/1", &p).is_err());
        assert!(fs::read_dir(&empty).unwrap().next().is_none());
        let single = BundleProjection {
            pages: vec![make("single")],
            ..p
        };
        let valid = temp.path().join("valid");
        write(&valid, "snapshot/1", &single).unwrap();
        for slug in ["fields-state", "diagnostic"] {
            let mdx = fs::read_to_string(valid.join(format!("single-{slug}.mdx"))).unwrap();
            assert!(mdx.contains("<table ") && mdx.contains("<tbody>"));
            assert!(!mdx.contains('\n'));
            let html = fs::read_to_string(valid.join(format!("single-{slug}.html"))).unwrap();
            let body = html
                .split_once("<body>\n")
                .unwrap()
                .1
                .strip_suffix("</body></html>\n")
                .unwrap();
            let normalized = body.replace(".html", ".mdx");
            assert_eq!(mdx, normalized);
        }
    }
}
