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
fn content(p: &Projection) -> String {
    let mut out = format!("<main>\n<h1>{}</h1>\n", escape(&p.title));
    out += &paragraph(
        "Source interpretation with a declared profile. This is a static possible path, not a runtime trace or an incident diagnosis.",
    );
    out += &paragraph(&format!(
        "Framework: {}. Engine version: {} (profile declaration). Service: {}.",
        p.framework,
        p.engine_version.as_deref().unwrap_or("unresolved"),
        p.service
    ));
    out += "<nav aria-label=\"Page sections\"><a href=\"#overview\">Overview</a> · <a href=\"#steps\">Steps and decisions</a> · <a href=\"#sources\">Sources and version</a></nav>\n<h2 id=\"overview\">Overview</h2>\n";
    out += &paragraph(
        "Phases group source declarations for reading. Open each phase for its full details; category order is not execution order.",
    );
    out += "<ol>\n";
    for phase in &p.phases {
        out += &format!(
            "<li><a href=\"#{}\">{}</a></li>\n",
            phase.id,
            escape(&phase.label)
        );
    }
    out += "</ol>\n<h2 id=\"steps\">Steps and decisions</h2>\n";
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
                "Record kind: {}. Binding: {}; call target: {}. Semantic authority: source interpretation with declared profile.",
                i.kind, i.reference.binding_authority, i.reference.call_target_authority
            ));
            if !i.conditions.is_empty() {
                out += "<h4>Condition path</h4>\n";
                out += &list(i.conditions.clone());
            }
            if !i.members.is_empty() {
                out += &format!(
                    "<h4>{}</h4>\n",
                    if i.kind == "registry-binding" {
                        "Lookup key and callback (unordered registry)"
                    } else if i.kind == "callback-group" {
                        "Declared callbacks (group does not establish parallelism)"
                    } else {
                        "Declared members"
                    }
                );
                out += &list(i.members.clone());
            }
            out += &format!("<pre>{}</pre>\n", escape(&i.expression));
            out += &list(i.limitations.clone());
            out += &format!(
                "<p><a href=\"sources.html#source-{}\">Source: lines {}–{}</a></p>\n",
                p.sources
                    .keys()
                    .position(|id| id == &i.reference.source_id)
                    .unwrap()
                    + 1,
                i.reference.start_line,
                i.reference.end_line
            );
            out += "</details>\n";
        }
        out += "</section>\n";
    }
    out += "<details id=\"sources\">\n<summary>Sources and version</summary>\n";
    out += &paragraph(&format!(
        "Revision: {}. Original snapshot: {}. Profile digest: {}.",
        p.revision, p.snapshot, p.profile_digest
    ));
    out += &list(p.limitations.clone());
    out += "<p><a href=\"sources.html\">Retained source appendix</a> · <a href=\"projection.json\">Typed projection JSON</a></p>\n</details>\n</main>\n";
    out
}
fn html(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"></head><body>\n{body}</body></html>\n",
        escape(title)
    )
}
const CSS: &str = "html{background:#f7f4ec;color:#172d49;font-family:system-ui,sans-serif;line-height:1.6}body{margin:0}main{max-width:960px;margin:auto;padding:32px 24px;overflow-wrap:anywhere}h1{line-height:1.2}a{color:#245d97}nav{margin:20px 0}details{border-top:1px solid #d4d9df;padding:12px 0}summary{cursor:pointer;font-weight:600}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#edf0f2;padding:12px;font-size:14px}li{margin:8px 0}section{scroll-margin-top:16px}@media(max-width:600px){main{padding:20px 16px}h1{font-size:28px}nav a{display:inline-block;margin:4px 0}}\n";
pub(super) fn write(output: &Path, p: &Projection) -> Result<(), ClewError> {
    let body = content(p);
    let mut appendix = "<main><h1>Retained source appendix</h1><p><a href=\"page.html#sources\">Return to process</a></p>\n".to_owned();
    for (index, s) in p.sources.values().enumerate() {
        appendix += &format!(
            "<details id=\"source-{}\"><summary>{}:{}–{}</summary>\n",
            index + 1,
            escape(&s.file),
            s.start_line,
            s.end_line
        );
        appendix += &paragraph(&format!(
            "Source authority: {}. Revision: {}. Text digest: {}. Exact retained source is distinct from DSL interpretation.",
            s.authority, s.revision, s.text_digest
        ));
        appendix += &format!("<pre>{}</pre></details>\n", escape(&s.text));
    }
    appendix += "</main>\n";
    let mdx = format!("<link rel=\"stylesheet\" href=\"./style.css\" />\n\n{body}");
    let files = BTreeMap::from([
        ("page.mdx", mdx.into_bytes()),
        ("page.html", html(&p.title, &body).into_bytes()),
        ("style.css", CSS.as_bytes().to_vec()),
        (
            "sources.html",
            html("Retained source appendix", &appendix).into_bytes(),
        ),
        (
            "projection.json",
            serde_json::to_vec_pretty(p).map_err(io_error)?,
        ),
    ]);
    let manifest = json!({"schema":"codeclew-flow-dsl-static-manifest/1.0","pageId":p.id,"snapshot":p.snapshot,"revision":p.revision,"profileDigest":p.profile_digest,"contentDigest":crate::canonical::hash_bytes(body.as_bytes()),"mdxProfile":"MDX 3; inert native JSX; no executable expressions or imports","authority":p.authority,"files":files.iter().map(|(name,bytes)|json!({"path":name,"digest":crate::canonical::hash_bytes(bytes)})).collect::<Vec<_>>()});
    let mut files = files;
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
    content(p)
}
