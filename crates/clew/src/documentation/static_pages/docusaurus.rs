//! A portable publication projection. Retained data stays lossless behind
//! content-addressed references; Markdown is a separate reading surface.
use crate::documentation::{digest, invalid, io_error};
use crate::error::ClewError;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

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
    format!(
        "---\ntitle: {}\nslug: {}\n---\n\n",
        json!(title),
        json!(slug)
    )
}

fn source_link(source: &Value) -> Result<String, ClewError> {
    if !is_source(source) {
        return Ok("Source binding unavailable in this projection.".into());
    }
    let id = digest(source)?;
    Ok(format!(
        "[{}:{}–{} · revision {}](../sources/{}.mdx)",
        md(text(&source["file"])),
        source["startLine"],
        source["endLine"],
        md(text(&source["revision"])),
        &id[7..]
    ))
}

fn evidence(block: &Value) -> Result<String, ClewError> {
    let mut out = String::from("<details>\n<summary>Evidence and scope</summary>\n\n");
    for e in block["evidence"].as_array().into_iter().flatten() {
        if e["kind"] == "RETAINED_CLAIM" {
            out.push_str(&format!(
                "**{}** · {} · {}\n\n{}\n\n",
                md(text(&e["title"])),
                md(text(&e["state"]["freshness"])),
                md(text(&e["state"]["verification"])),
                md(text(&e["fragment"]["text"]))
            ));
            for source in e["sources"]
                .as_object()
                .into_iter()
                .flat_map(|v| v.values())
            {
                out.push_str(&format!("- {}\n", source_link(source)?));
            }
        } else {
            out.push_str(&format!("- {}\n", source_link(&e["source"])?));
        }
    }
    out.push_str("\n</details>\n\n");
    Ok(out)
}

const IMPORTS: &str = "import {ClewProcess, ClewDataModel, ClewDownload, ClewModelLink, ClewFieldTrace} from '@site/src/components/ClewReaders';\n\n";

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
    let mut out = format!("{}\n\n", md(text(&block["text"])));
    for link in rows(&block["links"]) {
        let subject = text(&link["subject"]);
        let operation = text(&link["operation"]);
        let anchor = format!("question-{section}");
        let canonical =
            &page["canonicalEntityLinks"][&anchor][subject.strip_prefix("service:").unwrap_or("")];
        if operation == "section-entities" && canonical.is_array() {
            for model in rows(canonical) {
                out.push_str(&format!("<ClewModelLink model={} process={} anchor={}>{} · data model</ClewModelLink>\n\n",
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
        let target = if directory == "scenarios"
            && operation != "process-overview"
            && operation != id
            && !operation.starts_with("question-")
        {
            format!("../operations/scenarios-{id}--{operation}.mdx")
        } else {
            format!("../{directory}/{id}.mdx#{operation}")
        };
        out.push_str(&format!("[{}]({target})\n\n", md(label)));
    }
    out.push_str(&evidence(block)?);
    if block["disclosure"] == true || block["disclosure"].is_string() {
        Ok(details(text(&block["title"]), &out))
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

fn process_page(page: &Value, id: &str) -> Result<String, ClewError> {
    let guide = &page["questionReader"];
    let mut out = header(text(&guide["title"]), &format!("/scenarios/{id}"));
    out.push_str(IMPORTS);
    out.push_str(&format!(
        "# {} {{#process-overview}}\n\n{}\n\n",
        md(text(&guide["title"])),
        md(text(&guide["purpose"]))
    ));
    out.push_str(
        "Editorial reading guide · original source scopes and review states remain separate.\n\n",
    );
    let flow = page["processDataFlow"].is_object();
    if flow {
        out.push_str(&format!(
            "<ClewProcess dataPath=\"/clew-data/widgets/scenario-{id}.json\">\n\n"
        ));
    }
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
        out.push_str(&details("Reading map",&(String::from("Reading order only. Conditions and failures remain beside the steps below; this is not a runtime trace.\n\n")+&fence(&graph,"mermaid"))));
    }
    for section in rows(&guide["sections"]) {
        let sid = text(&section["id"]);
        let anchor = format!("question-{sid}");
        let exception = section["number"].is_null() && sid != "confidence";
        if exception {
            out.push_str("<aside className=\"clew-exception\">\n\n");
        }
        out.push_str(&format!(
            "{} {} {{#{anchor}}}\n\n",
            if exception { "###" } else { "##" },
            md(text(&section["title"]))
        ));
        if !text(&section["route"]).is_empty() {
            out.push_str(&fence(text(&section["route"]), "text"));
        }
        for b in rows(&section["blocks"]) {
            out.push_str(&block(b, page, sid)?);
        }
        if exception {
            out.push_str("</aside>\n\n");
        }
    }
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

fn operation_page(page: &Value, op: &Value, prefix: &str) -> Result<String, ClewError> {
    let id = text(&op["id"]);
    let mut out = header(text(&op["title"]), &format!("/operations/{prefix}--{id}"));
    out.push_str(IMPORTS);
    let state = &page["operationStates"][id];
    out.push_str(&format!("# {}\n\nRetained account · {} · {}. Runtime behavior is not established by this export.\n\n{}\n\n",md(text(&op["title"])),md(text(&state["freshness"])),md(text(&state["verification"])),md(text(&op["summary"]["text"]))));
    let sources = &page["operationSources"][id];
    let cite = |claim: &Value| -> Result<String, ClewError> {
        let mut result = String::new();
        for sid in rows(&claim["sourceIds"]).filter_map(Value::as_str) {
            let s = sources.get(sid).or_else(|| page["sources"].get(sid));
            if let Some(s) = s {
                result.push_str(&format!("- {}\n", source_link(s)?));
            }
        }
        Ok(result)
    };
    out.push_str(&details("Summary source bindings", &cite(&op["summary"])?));
    out.push_str("## Sequence and branches\n\n");
    for (i, event) in rows(&op["events"]).enumerate() {
        out.push_str(&format!(
            "### {}. {}\n\n{}\n\n",
            i + 1,
            md(text(&event["kind"])),
            md(text(&event["text"]))
        ));
        out.push_str(&details("Source bindings", &cite(event)?));
    }
    out.push_str("## Implementation explanation\n\n");
    for claim in rows(&op["explanation"]) {
        out.push_str(&format!("{}\n\n", md(text(&claim["text"]))));
        out.push_str(&details("Source bindings", &cite(claim)?));
    }
    out.push_str(&details(
        "Retained boundaries",
        &rows(&op["boundaries"])
            .map(|v| format!("- {}\n", md(text(v))))
            .collect::<String>(),
    ));
    Ok(out)
}

fn model_page(model: &Value, catalog: &Value) -> Result<String, ClewError> {
    let id = text(&model["id"]);
    let mut out = header(text(&model["title"]), &format!("/data-model/{id}"));
    out.push_str(IMPORTS);
    out.push_str(&format!(
        "# {}\n\nAgent proposal · Not reviewed.\n\n<p id=\"dm-return\"></p>\n\n{}\n\n",
        md(text(&model["title"])),
        md(text(&model["purpose"]["text"]))
    ));
    out.push_str(&evidence(&model["purpose"])?);
    out.push_str(&format!(
        "## Identity and relationships\n\n{}\n\n",
        md(text(&model["identity"]["text"]))
    ));
    out.push_str(&evidence(&model["identity"])?);
    for related in rows(&model["relatedModels"]).filter_map(Value::as_str) {
        out.push_str(&format!("- [{}]({related}.mdx)\n", md(related)));
    }
    out.push('\n');
    out.push_str("## Fields and representations\n\n");
    for field in rows(&model["fields"]) {
        let fid = text(&field["id"]);
        out.push_str(&format!(
            "### {} {{#field-{fid}}}\n\n{}\n\n",
            md(text(&field["path"])),
            md(text(&field["meaning"]["text"]))
        ));
        out.push_str(&evidence(&field["meaning"])?);
        for rep in rows(&field["representations"]) {
            let mut body = format!(
                "{} · {} · {}\n\n{}\n\n",
                md(text(&rep["typeLabel"])),
                md(text(&rep["requiredness"])),
                md(text(&rep["nullability"])),
                md(text(&rep["mapping"]["text"]))
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
                        "Required when branch matches: {} · Accepts null: {}\n\n",
                        leaf["requiredWhenBranchMatches"], leaf["nullable"]
                    ));
                    body.push_str(&fence(
                        &serde_json::to_string_pretty(&leaf["schema"]).map_err(io_error)?,
                        "json",
                    ));
                }
            }
            body.push_str(&evidence(&rep["mapping"])?);
            out.push_str(&details(text(&rep["label"]), &body));
        }
        out.push_str(&format!("<ClewFieldTrace field={} />\n\n", json!(fid)));
    }
    out.push_str(&format!(
        "<ClewDataModel dataPath=\"/clew-data/widgets/model-{id}.json\" />\n\n## Processes\n\n"
    ));
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

pub(super) fn project(
    native: &BTreeMap<String, Vec<u8>>,
    manifest: &Value,
    inputs: &super::full::ReadingInputs<'_>,
) -> Result<(BTreeMap<String, Vec<u8>>, Value), ClewError> {
    let mut files = BTreeMap::new();
    let mut documents = BTreeMap::new();
    let mut sidebars = vec![json!("overview")];
    for (name, bytes) in native.iter().filter(|(name, _)| name.ends_with(".json")) {
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
    let mut process_items = Vec::new();
    let mut service_items = Vec::new();
    let mut overview = header("Documentation", "/");
    overview.push_str(IMPORTS);
    overview.push_str("# Documentation\n\nFollow a process, inspect its contracts and data, and return to the source evidence when needed.\n\n## Processes\n\n");
    for (name, page) in &pages {
        let id = name.rsplit('/').next().unwrap().trim_end_matches(".json");
        let process = name.starts_with("native/scenarios/");
        let directory = if process { "scenarios" } else { "services" };
        let title = if page["questionReader"].is_object() {
            text(&page["questionReader"]["title"])
        } else {
            text(&page["title"])
        };
        let body = if process && page["questionReader"].is_object() {
            process_page(page, id)?
        } else {
            let mut body = header(title, &format!("/{directory}/{id}"));
            body.push_str(IMPORTS);
            body.push_str(&format!(
                "# {}\n\n{}\n\n",
                md(title),
                md(text(&page["subtitle"]))
            ));
            for section in rows(&page["sections"]) {
                body.push_str(&format!(
                    "## {} {{#{}}}\n\n{}\n\n",
                    md(text(&section["title"])),
                    text(&section["id"]),
                    md(text(&section["summary"]["text"]))
                ));
            }
            // Legacy service context remains distinct from the canonical models.
            if !body.contains("{#section-entities}") {
                body.push_str("## Legacy entity context {#section-entities}\n\nThis saved service projection has its own evidence scope. Canonical models are linked separately from the process reading pages.\n\n");
            }
            body.push_str("## Original operations\n\n");
            for op in rows(&page["operations"]) {
                body.push_str(&format!(
                    "- [{}](../operations/{directory}-{id}--{}.mdx)\n",
                    md(text(&op["title"])),
                    text(&op["id"])
                ));
            }
            body.push('\n');
            body.push_str(&archive_link(name.trim_start_matches("native/")));
            body
        };
        put(&mut files, format!("docs/{directory}/{id}.mdx"), body);
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
    sidebars.push(json!({"type":"category","label":"Processes","items":process_items}));
    overview.push_str("\n## Data model\n\n");
    let mut model_items = Vec::new();
    for model in rows(&catalog["models"]) {
        let id = text(&model["id"]);
        put(
            &mut files,
            format!("docs/data-model/{id}.mdx"),
            model_page(model, &catalog)?,
        );
        documents.insert(format!("widgets/model-{id}.json"),json!({"models":[model],"contracts":super::data_model::model_contracts(model,&catalog["contracts"]),"authority":catalog["authority"],"review":catalog["review"]}));
        model_items.push(json!(format!("data-model/{id}")));
        overview.push_str(&format!(
            "- [{}](data-model/{id}.mdx)\n",
            md(text(&model["title"]))
        ));
    }
    sidebars.push(json!({"type":"category","label":"Data model","items":model_items}));
    let mut contract_items = Vec::new();
    for contract in rows(&catalog["contracts"]) {
        let id = text(&contract["id"]);
        let mut body = header(text(&contract["title"]), &format!("/contracts/{id}"));
        body.push_str(IMPORTS);
        body.push_str(&format!("# {}\n\nDeclared contract · runtime conformance is not established.\n\nExact pointer:\n\n{}",md(text(&contract["title"])),fence(text(&contract["branchPointer"]),"text")));
        body.push_str(&source_link(&contract["source"])?);
        body.push_str("\n\n## Properties\n\nRequiredness follows the selected branch and required parents. Required and nullable are separate.\n\n");
        for leaf in rows(&contract["leaves"]) {
            body.push_str(&format!(
                "### {}\n\nRequired when branch matches: {} · Accepts null: {}\n\n{}",
                md(text(&leaf["field"])),
                leaf["requiredWhenBranchMatches"],
                leaf["nullable"],
                fence(
                    &serde_json::to_string_pretty(&leaf["schema"]).map_err(io_error)?,
                    "json"
                )
            ));
        }
        put(&mut files, format!("docs/contracts/{id}.mdx"), body);
        contract_items.push(json!(format!("contracts/{id}")));
    }
    sidebars.push(json!({"type":"category","label":"Declared contracts","items":contract_items}));
    sidebars.push(json!({"type":"category","label":"Service context","items":service_items}));
    overview.push_str("\n## Provenance\n\nOriginal claims, evidence and their independent review states remain in the reference archive. Source references include the original revision, digest and line range; no runtime guarantee is added by export.\n\n");
    overview.push_str("[Source fragments](sources/index.mdx)\n\n");
    put(&mut files, "docs/overview.mdx", overview);
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
    let mut source_index = header("Source fragments", "/sources");
    source_index.push_str("# Source fragments\n\nEach page keeps its exact retained revision, range, text digest and authority. Identical text at another revision remains a separate binding.\n\n");
    for (id, source) in &objects.sources {
        let slug = &id[7..];
        let title = format!(
            "{}:{}–{}",
            text(&source["file"]),
            source["startLine"],
            source["endLine"]
        );
        let mut body = header(&title, &format!("/sources/{slug}"));
        body.push_str(IMPORTS);
        body.push_str(&format!(
            "# {}\n\nAuthority: {}\n\nRevision: {}\n\nText digest: {}\n\n## Retained text\n\n{}",
            md(&title),
            md(text(&source["authority"])),
            md(text(&source["revision"])),
            md(text(&source["textDigest"])),
            fence(text(&source["text"]), "text")
        ));
        body.push_str(&format!("<ClewDownload path=\"/clew-data/objects/{slug}.json\">Complete original source binding</ClewDownload>\n"));
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
            "src/components/clew-model-runtime.js",
            include_bytes!("../../../assets/documentation/data-model.js").as_slice(),
        ),
        (
            "src/css/clew.css",
            include_bytes!("../../../assets/documentation/docusaurus/clew.css").as_slice(),
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
        b"node_modules/\nbuild/\n.docusaurus/\n".to_vec(),
    );
    put(&mut files,"README.md",b"# Codeclew documentation export\n\nCommit inputs/, docs/, static/clew-data/, src/, package.json, package-lock.json, sidebars.js, docusaurus.config.js and the export manifest. Do not commit node_modules/, build/ or .docusaurus/. Markdown is the reading projection; normalized data and content-addressed objects retain complete original provenance. Source identity includes the whole binding, including revision and range.\n\nRun npm ci, then npm run build. Set CODECLEW_DOCS_BASE_URL for a non-root deployment path and CODECLEW_DOCS_URL for its origin. Deploy build/ only. This export does not deploy a website.\n\nThe standalone HTML profile remains the default. Historical paths inside source text are retained evidence, not dependencies on the author machine. New Markdown contains local file links; interactive readers use baseUrl-aware site routes. Original saved accounts and the selected snapshot retain independent authority and review state.\n".to_vec());
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
        assert!(objects.values.len() >= 1);
        assert_eq!(objects.expand(&value, 0).unwrap(), input);
        let id = objects.values.keys().next().unwrap().clone();
        objects.values.insert(id, json!({"changed":true}));
        assert!(objects.expand(&value, 0).is_err());
    }
    #[test]
    fn source_fences_and_editorial_text_cannot_become_executable_mdx() {
        let value = "import x from 'x'\n<script>{unsafe()}</script>\n```";
        assert!(md(value).starts_with("&#105;mport"));
        assert!(md(value).contains("&#123;unsafe()&#125;"));
        assert!(fence(value, "text").starts_with("````text\n"));
    }
}
