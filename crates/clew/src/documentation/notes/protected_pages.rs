//! Human-owned pages use safe Markdown presentation and exact original downloads.
use super::*;

pub fn body(text: &str) -> String {
    let lines: Vec<_> = text.lines().collect();
    if lines.first() == Some(&"---")
        && let Some(end) = lines.iter().skip(1).position(|l| *l == "---")
    {
        return lines[end + 2..].join("\n");
    }
    text.to_owned()
}

pub fn write(payload: &Value, files: &mut BTreeMap<String, Vec<u8>>) -> Result<(), ClewError> {
    for note in payload["notes"].as_array().into_iter().flatten() {
        let a = &note["association"];
        let id = a["id"]
            .as_str()
            .ok_or_else(|| invalid("protected page ID is absent"))?;
        if !store::valid_id(id)
            || payload["subject"] != format!("service:{}", a["service"].as_str().unwrap_or(""))
        {
            continue;
        }
        let original = note["original"]["text"].as_str().unwrap_or("");
        let escape = super::super::render::escape;
        let mut html = format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>body{{max-width:960px;margin:32px auto;padding:0 24px;font:17px/1.6 system-ui}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}table{{border-collapse:collapse}}th,td{{padding:8px;border:1px solid #ddd;text-align:left}}details{{margin:20px 0}}a{{color:#245aac}}</style></head><body><nav><a href=\"../overview.html\">Overview</a> · <a href=\"../services/{}.html#{}\">Service and separate assessment</a></nav><main><h1>{}</h1><p>Human-owned page · {} · Original text remains unverified.</p><article data-clew-markdown>{}</article><p>Page digest: <code>{}</code> · <a download href=\"{}.original.md\">Download exact original Markdown</a></p>",
            escape(a["title"].as_str().unwrap_or(id)),
            escape(a["service"].as_str().unwrap_or("")),
            escape(note["id"].as_str().unwrap_or("")),
            escape(a["title"].as_str().unwrap_or(id)),
            escape(a["classification"].as_str().unwrap_or("")),
            escape(&body(original)),
            escape(note["original"]["digest"].as_str().unwrap_or("UNAVAILABLE")),
            escape(id)
        );
        html.push_str("<h2>Source bindings</h2><p>Binding state and prose assessment are independent. Registered influence is conservatively scoped to the service.</p>");
        let bindings = note.get("currentBindings").unwrap_or(&note["bindings"]);
        for row in bindings.as_array().into_iter().flatten() {
            let binding_id = row["id"].as_str().unwrap_or("");
            if !store::valid_id(binding_id) {
                return Err(invalid("unsafe protected page binding ID"));
            }
            let name = format!("{id}-{binding_id}.source.txt");
            let source = &row["selectedSource"];
            html.push_str(&format!("<section><h3>{} · {}</h3><p><a href=\"{}\">{}:{}–{}</a> · revision <code>{}</code></p><p>Authority: {}</p><details><summary>Exact selected source and binding receipt</summary><pre>{}</pre></details></section>",escape(binding_id),escape(row["state"].as_str().unwrap_or("UNAVAILABLE")),escape(&name),escape(source["file"].as_str().unwrap_or("")),row["selection"]["startLine"],row["selection"]["endLine"],escape(source["revision"].as_str().unwrap_or("")),escape(row["authority"].as_str().unwrap_or("")),escape(&serde_json::to_string_pretty(row).map_err(io_error)?)));
            files.insert(
                format!("pages/{name}"),
                row["selectedText"]
                    .as_str()
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec(),
            );
        }
        if note["bindings"].as_array().is_none_or(Vec::is_empty) {
            html.push_str("<p>No typed source bindings are registered.</p>");
        }
        html.push_str(&format!("<h2>Separate assessment</h2><p>{}</p><details><summary>Original metadata and associations</summary><pre>{}</pre></details></main><script>{}</script><script>clewMarkdown.enhance(document);</script></body></html>",if note["assessment"].is_object(){"An assessment is retained separately; inspect its exact input and review status in service context."}else{"UNASSESSED. No semantic assessment has been accepted for this page."},escape(&serde_json::to_string_pretty(a).map_err(io_error)?),include_str!("../../../assets/documentation/markdown.js")));
        files.insert(format!("pages/{id}.html"), html.into_bytes());
        files.insert(
            format!("pages/{id}.original.md"),
            original.as_bytes().to_vec(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_table_is_safe_data_and_original_crlf_frontmatter_bytes_are_exact() {
        let original = "---\r\ntitle: Local policy\r\n---\r\n\r\nField | Meaning\r\n--- | ---\r\n`cityId` | [Source](../services/dashboard.html)\r\n\r\n<script>{unsafe()}</script>\r\n";
        let page = json!({"subject":"service:dashboard","notes":[{"id":"assessment-policy","association":{"id":"policy","service":"dashboard","title":"Local policy","classification":"mixed"},"original":{"text":original,"digest":"original-digest"},"assessment":null}]});
        let mut files = BTreeMap::new();
        write(&page, &mut files).unwrap();
        assert_eq!(files["pages/policy.original.md"], original.as_bytes());
        let html = String::from_utf8(files["pages/policy.html"].clone()).unwrap();
        assert!(html.contains("<head>") && html.contains("</head>"));
        assert!(html.contains("data-clew-markdown") && html.contains("Field | Meaning"));
        assert!(!html.contains("<script>{unsafe()}"));
        assert!(html.contains("&lt;script&gt;{unsafe()}&lt;/script&gt;"));
        assert!(!body(original).starts_with("---"));
        let mut renamed = page.clone();
        renamed["notes"][0]["association"]["title"] = json!("Renamed");
        let mut later = BTreeMap::new();
        write(&renamed, &mut later).unwrap();
        assert!(later.contains_key("pages/policy.html"));
        assert_eq!(later["pages/policy.original.md"], original.as_bytes());
    }

    #[test]
    fn protected_page_uses_the_shared_reader_header_styles_after_bundle_decoration() {
        let page = json!({"subject":"service:sample","notes":[{"id":"assessment-policy","association":{"id":"policy","service":"sample","title":"Local policy","classification":"mixed"},"original":{"text":"Field | Meaning\n--- | ---\nid | Identifier","digest":"original-digest"},"assessment":null}]});
        let mut files = BTreeMap::new();
        write(&page, &mut files).unwrap();
        let original = files["pages/policy.original.md"].clone();
        super::super::super::reader::decorate_bundle(&mut files, "synthetic").unwrap();
        let html = String::from_utf8(files["pages/policy.html"].clone()).unwrap();
        let head = html
            .split_once("<head>")
            .unwrap()
            .1
            .split_once("</head>")
            .unwrap()
            .0;
        assert!(head.contains(super::super::super::reader::STYLE));
        assert!(head.contains(".reader-brand svg"));
        assert_eq!(html.matches("<nav class=\"reader-nav\"").count(), 1);
        assert!(html.contains("data-clew-markdown"));
        assert_eq!(files["pages/policy.original.md"], original);
    }
}
