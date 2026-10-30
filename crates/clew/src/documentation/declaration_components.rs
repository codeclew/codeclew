//! Immutable declaration components. Prose and runtime meaning stay authored.
use super::{
    cache,
    check::Check,
    digest, invalid, io_error, language,
    store::{self, Repository},
};
use crate::{canonical, error::ClewError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
mod postgres;
const SET: &str = "codeclew-declaration-components/1.0";
const REQUEST: &str = "codeclew-declaration-selection/1.0";
const TICKET: &str = "codeclew-declaration-components-ticket/1.0";
const MAX: u64 = 16 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceSelector {
    kind: String,
    service: String,
    id: String,
    revision: String,
    file: String,
    text_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evidence_set: Option<Value>,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SqlRange {
    coordinate_domain: String,
    byte_start: usize,
    byte_end: usize,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SqlScope {
    database: String,
    ranges: Vec<SqlRange>,
    reason: String,
    evidence: Vec<SourceSelector>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    id: String,
    title: String,
    format: String,
    source: SourceSelector,
    #[serde(default)]
    branch_pointer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sql_scope: Option<SqlScope>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema: String,
    snapshot: String,
    language: String,
    components: Vec<Selection>,
}

fn implementation() -> String {
    canonical::hash_bytes(
        &[
            include_bytes!("declaration_components.rs").as_slice(),
            include_bytes!("declaration_components/postgres.rs").as_slice(),
            include_bytes!("static_pages/inline_contract.rs").as_slice(),
            include_bytes!("static_pages/inline_contract/openapi.rs").as_slice(),
        ]
        .concat(),
    )
}
fn esc(s: &str) -> String {
    super::render::escape(s)
}
fn tr<'a>(lang: &str, en: &'a str, ru: &'a str) -> &'a str {
    if lang == "ru" { ru } else { en }
}
fn bind_record(
    source: Value,
    sel: &SourceSelector,
    snapshot: &str,
    native_binding: Value,
) -> Result<Value, ClewError> {
    let text = source["text"]
        .as_str()
        .ok_or_else(|| invalid("declaration source text unavailable"))?;
    if text.is_empty()
        || text.len() > 256 * 1024
        || source["id"] != sel.id
        || source["service"] != sel.service
        || source["revision"] != sel.revision
        || source["file"] != sel.file
        || source["textDigest"] != sel.text_digest
        || canonical::hash_bytes(text.as_bytes()) != sel.text_digest
    {
        return Err(invalid(
            "declaration source identity, revision, file or digest changed",
        ));
    }
    Ok(
        json!({"kind":sel.kind,"snapshot":snapshot,"selector":sel,"source":source,
        "range":{"coordinateDomain":"SOURCE_RECORD_UTF8_BYTES","byteStart":0,"byteEnd":text.len()},"nativeBinding":native_binding}),
    )
}
fn source(
    repo: &Repository,
    checked: &Check,
    snapshot: &str,
    sel: &SourceSelector,
) -> Result<Value, ClewError> {
    store::relative(&sel.file)?;
    match sel.kind.as_str() {
        "SOURCE" if sel.evidence_set.is_none() => {
            let record = checked
                .services
                .get(&sel.service)
                .and_then(|e| e.sources.get(&sel.id))
                .ok_or_else(|| invalid("selected retained declaration SOURCE is unavailable"))?;
            if !matches!(
                record.authority.as_str(),
                "EXACT_SNAPSHOT_TEXT" | "DECLARED_OPENAPI" | "EXACT_DEPENDENCY_SOURCE_ARCHIVE"
            ) {
                return Err(invalid("declaration SOURCE authority is unsupported"));
            }
            bind_record(
                serde_json::to_value(record).map_err(io_error)?,
                sel,
                snapshot,
                Value::Null,
            )
        }
        "SOURCE_NATIVE" => {
            let ticket = sel
                .evidence_set
                .as_ref()
                .ok_or_else(|| invalid("SOURCE_NATIVE requires its genuine evidenceSet"))?;
            if ticket["snapshot"] != snapshot || ticket["service"] != sel.service {
                return Err(invalid("native declaration evidenceSet scope changed"));
            }
            let replay =
                super::prepared_evidence::compare_value(repo, ticket.clone(), snapshot, true)?;
            if replay["evidenceSet"] != *ticket
                || replay["comparison"]["integrity"] != "BOTH_IMMUTABLE_SNAPSHOT_CLOSURES_ADMITTED"
                || replay["comparison"]["baselineSnapshot"] != snapshot
                || replay["comparison"]["targetSnapshot"] != snapshot
                || replay["comparison"]["baselineBinding"] != replay["comparison"]["targetBinding"]
            {
                return Err(invalid("native declaration immutable binding changed"));
            }
            let row = replay["items"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|r| r["kind"] == "SOURCE_NATIVE" && r["id"] == sel.id)
                .ok_or_else(|| invalid("selected native declaration source is unavailable"))?;
            if row["record"]["authority"] != "REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS" {
                return Err(invalid("native declaration authority changed"));
            }
            bind_record(
                row["record"].clone(),
                sel,
                snapshot,
                replay["comparison"]["targetBinding"].clone(),
            )
        }
        _ => Err(invalid("declaration source selector kind is invalid")),
    }
}
fn ddl_html(p: &Value, lang: &str) -> String {
    let mut html = String::new();
    for table in p["tables"].as_array().into_iter().flatten() {
        let name = table["name"].as_str().unwrap_or("");
        // Attach real context to this table, so an unknown later context cannot
        // accidentally inherit an earlier database label. Empty context adds no UI.
        let heading = table["database"].as_str().map_or_else(
            || esc(name),
            |database| {
                format!(
                    "{} · {}: {}",
                    esc(name),
                    tr(lang, "database", "база"),
                    esc(database)
                )
            },
        );
        html.push_str(&format!("<h3>{heading}</h3>"));
        if let Some(path) = table["searchPath"].as_array().filter(|v| !v.is_empty()) {
            let names = path
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            html.push_str(&format!("<p>search_path: {}</p>", esc(&names)));
        }
        let keys = table["primaryKeyDeclarations"].as_array();
        let bound = table["primaryKeyBindingEstablished"] == true;
        let composite = bound
            && keys
                .and_then(|k| k.first())
                .and_then(Value::as_array)
                .is_some_and(|k| k.len() > 1);
        if let Some(keys) = keys.filter(|keys| !keys.is_empty()) {
            let groups = keys
                .iter()
                .map(|key| {
                    let names = key
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|key| {
                            table["columns"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find(|c| c["key"] == key)
                                .and_then(|c| c["name"].as_str())
                                .unwrap_or(key)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({names})")
                })
                .collect::<Vec<_>>()
                .join("; ");
            let label = if composite {
                tr(lang, "Composite primary key", "Составной первичный ключ")
            } else if bound {
                tr(lang, "Primary key", "Первичный ключ")
            } else {
                tr(
                    lang,
                    "Declared primary key (binding unresolved)",
                    "Объявленный первичный ключ (привязка не установлена)",
                )
            };
            html.push_str(&format!("<p>{label}: <code>{}</code></p>", esc(&groups)));
        }
        let nodes = table["columns"].as_array().into_iter().flatten().map(|c| {
            let mut notes = Vec::new();
            if c["primaryKey"] == true {
                notes.push(if composite {tr(lang, "Part of composite primary key", "Часть составного первичного ключа")}
                    else {tr(lang, "Primary key", "Первичный ключ")});
            } else if c["primaryKeyDeclared"] == true {
                notes.push(tr(lang, "Primary key declaration (binding unresolved)", "Декларация первичного ключа (привязка не установлена)"));
            }
            notes.push(match c["nullabilityBasis"].as_str().unwrap_or("") {
                "DECLARED_NOT_NULL" => tr(lang, "NOT NULL declared", "Объявлено NOT NULL"),
                "PRIMARY_KEY_RULE" => tr(lang, "NOT NULL follows from PRIMARY KEY", "NOT NULL следует из PRIMARY KEY"),
                "DECLARED_NULL" => tr(lang, "NULL declared", "Объявлено NULL"),
                _ => tr(lang, "Nullability not explicitly declared", "Допустимость NULL явно не объявлена"),
            });
            if c["declaredNullability"] == "NULL" && c["primaryKey"] == true {
                notes.push(tr(lang, "Explicit NULL retained; PRIMARY KEY implies NOT NULL", "Явное NULL сохранено; PRIMARY KEY подразумевает NOT NULL"));
            }
            let mut literals = serde_json::Map::new();
            if let Some(value) = c["defaultExpression"].as_str() {
                literals.insert(tr(lang, "Default expression", "Выражение DEFAULT").into(), json!(value));
            }
            json!({"field":c["name"],"type":c["type"],"nullability":if c["acceptsNull"]==true {"Allowed"}else if c["acceptsNull"]==false {"Excluded"}else{"Not established"},"description":notes.join(". "),"literalConstraints":literals})
        }).collect::<Vec<_>>();
        html.push_str(
            &super::static_pages::inline_contract::table_html_without_presence(
                &json!({"nodes":nodes}),
                lang,
            ),
        );
    }
    html
}

fn project(selection: &Selection, binding: &Value, lang: &str) -> Result<Value, ClewError> {
    let text = binding["source"]["text"].as_str().unwrap();
    let (projection, body) = match selection.format.as_str() {
        "JSON_SCHEMA" | "OPENAPI" if selection.sql_scope.is_none() => {
            let document: Value =
                serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(io_error)?;
            if selection.format == "OPENAPI" && !document["openapi"].is_string() {
                return Err(invalid("OpenAPI declaration document marker is missing"));
            }
            let branch = document
                .pointer(&selection.branch_pointer)
                .ok_or_else(|| invalid("selected declaration branch is unavailable"))?;
            let p = super::static_pages::inline_contract::project_document(
                branch,
                &selection.branch_pointer,
                &document,
            )?;
            let body = super::static_pages::inline_contract::table_html(&p, lang);
            (p, body)
        }
        "POSTGRES_CREATE_TABLE" if selection.branch_pointer.is_empty() => {
            let p = match &selection.sql_scope {
                Some(scope) => postgres::project_scoped(text, scope)?,
                None => postgres::project(text),
            };
            let body = ddl_html(&p, lang);
            (p, body)
        }
        _ => {
            return Err(invalid(
                "declaration format or branch selection is unsupported",
            ));
        }
    };
    let mut html = format!(
        "<section id=\"clew-component-{}\" data-clew-component=\"{}\"><h2>{}</h2>{body}",
        selection.id,
        selection.id,
        esc(&selection.title)
    );
    let gaps = projection["frontier"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !gaps.is_empty() {
        html.push_str(&format!(
            "<p><strong>{} ({})</strong></p><details><summary>{}</summary><ul>",
            tr(lang, "Partial projection", "Неполная проекция"),
            gaps.len(),
            tr(lang, "Declaration limits", "Границы декларации")
        ));
        for g in &gaps {
            let detail = if let Some(s) = g.as_str() {
                s.into()
            } else {
                let a = g["source"]["byteStart"].as_u64().unwrap_or(0) as usize;
                let z = g["source"]["byteEnd"].as_u64().unwrap_or(a as u64) as usize;
                let base_line = binding["source"]["startLine"].as_u64().unwrap_or(1) as usize;
                let first = text[..a.min(text.len())]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + base_line;
                let last = text[..z.min(text.len())]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + base_line;
                format!(
                    "{} {first}–{last}.",
                    tr(
                        lang,
                        "A declaration or constraint is retained without interpretation; source lines",
                        "Декларация или ограничение сохранены без интерпретации; строки исходника"
                    )
                )
            };
            html.push_str(&format!("<li>{}</li>", esc(&detail)));
        }
        html.push_str("</ul></details>");
    }
    html.push_str("</section>");
    // Retained literals are HTML text, never Markdown citation syntax.
    html = html.replace('[', "&#91;").replace(']', "&#93;");
    // One raw HTML block survives the existing Markdown renderer. Its heading
    // pass touches only unadorned headings; component anchors remain native.
    for level in 1..=6 {
        html = html.replace(
            &format!("<h{level}>"),
            &format!("<h{level} class=\"clew-declaration-heading\">"),
        );
    }
    html = html.replace('\r', "&#13;").replace('\n', "&#10;");
    Ok(
        json!({"id":selection.id,"title":selection.title,"format":selection.format,"branchPointer":selection.branch_pointer,"binding":binding,"projection":projection,"htmlDigest":canonical::hash_bytes(html.as_bytes()),"html":html}),
    )
}
fn build(repo: &Repository, request: &Request) -> Result<Value, ClewError> {
    language::validate(Some(&request.language))?;
    if request.schema != REQUEST || request.components.is_empty() || request.components.len() > 32 {
        return Err(invalid(
            "declaration component selection schema or bounds are invalid",
        ));
    }
    let checked = Check::load_snapshot(repo, &request.snapshot)?;
    let (mut ids, mut scopes) = (BTreeSet::new(), BTreeSet::new());
    let mut components = Vec::new();
    let mut bytes = 0;
    for s in &request.components {
        if !store::valid_id(&s.id)
            || s.title.trim().is_empty()
            || s.title.len() > 256
            || !ids.insert(&s.id)
        {
            return Err(invalid(
                "declaration component identity repeats or is invalid",
            ));
        }
        let mut binding = source(repo, &checked, &request.snapshot, &s.source)?;
        if let Some(scope) = &s.sql_scope {
            if s.format != "POSTGRES_CREATE_TABLE"
                || scope.database.is_empty()
                || scope.database.len() > 512
                || scope.ranges.is_empty()
                || scope.ranges.len() > 64
                || scope.reason.trim().is_empty()
                || scope.reason.len() > 2048
                || scope.evidence.is_empty()
                || scope.evidence.len() > 8
            {
                return Err(invalid(
                    "SQL scope requires bounded whole database sections and retained selection evidence",
                ));
            }
            let mut evidence = Vec::new();
            for selector in &scope.evidence {
                let proof = source(repo, &checked, &request.snapshot, selector)?;
                bytes += proof["source"]["text"].as_str().unwrap().len();
                if bytes > 4 * 1024 * 1024 {
                    return Err(invalid("declaration source selection exceeds 4 MiB"));
                }
                evidence.push(proof);
            }
            binding["scopeEvidence"] = json!(evidence);
        }
        if !scopes.insert(digest(&json!([
            binding["source"]["service"],
            binding["source"]["revision"],
            binding["source"]["file"],
            binding["source"]["textDigest"],
            binding["range"],
            s.format,
            s.branch_pointer,
            s.sql_scope
                .as_ref()
                .map(|scope| json!({"database":scope.database,"ranges":scope.ranges}))
        ]))?) {
            return Err(invalid(
                "declaration source scope repeats under another component identity",
            ));
        }
        bytes += binding["source"]["text"].as_str().unwrap().len();
        if bytes > 4 * 1024 * 1024 {
            return Err(invalid("declaration source selection exceeds 4 MiB"));
        }
        components.push(project(s, &binding, &request.language)?);
    }
    Ok(
        json!({"schema":SET,"snapshot":request.snapshot,"language":request.language,"implementationDigest":implementation(),"selection":request,"components":components,"authority":"RETAINED_DECLARATIONS_NOT_RUNTIME_PROOF","semanticTruthAssessed":false}),
    )
}
fn ticket(reference: &cache::ObjectRef, value: &Value) -> Value {
    json!({"schema":TICKET,"id":reference.digest,"reference":reference,"snapshot":value["snapshot"],"implementationDigest":value["implementationDigest"],"components":value["components"].as_array().unwrap().iter().map(|c|json!({"id":c["id"],"title":c["title"],"format":c["format"]})).collect::<Vec<_>>()})
}
pub(super) fn prepare(repo: &Repository, input: &Path) -> Result<Value, ClewError> {
    let request: Request = store::read(input, 256 * 1024)?;
    let value = build(repo, &request)?;
    let raw = super::bytes(&value)?;
    if raw.len() as u64 > MAX {
        return Err(invalid("declaration component set exceeds 16 MiB"));
    }
    let reference = cache::put(repo, SET, &raw)?;
    Ok(ticket(&reference, &value))
}
pub(super) fn read(repo: &Repository, input: &Path) -> Result<Value, ClewError> {
    let supplied: Value = store::read(input, 256 * 1024)?;
    let reference: cache::ObjectRef =
        serde_json::from_value(supplied["reference"].clone()).map_err(io_error)?;
    if reference.schema != SET {
        return Err(invalid("declaration component reference schema changed"));
    }
    let saved: Value = cache::get_json(repo, &reference, MAX)?
        .ok_or_else(|| invalid("immutable declaration component set unavailable"))?;
    if saved["schema"] != SET
        || saved["components"]
            .as_array()
            .is_none_or(|c| c.is_empty() || c.len() > 32)
        || saved["implementationDigest"] != implementation()
        || ticket(&reference, &saved) != supplied
    {
        return Err(invalid(
            "declaration component ticket or implementation changed",
        ));
    }
    let request: Request = serde_json::from_value(saved["selection"].clone()).map_err(io_error)?;
    if build(repo, &request)? != saved {
        return Err(invalid("declaration source/projection binding changed"));
    }
    Ok(
        json!({"schema":"codeclew-declaration-components-read/1.0","ticket":supplied,"packet":saved}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sel() -> Selection {
        serde_json::from_value(json!({"id":"record","title":"Record declaration","format":"JSON_SCHEMA","source":{"kind":"SOURCE","service":"neutral","id":"s","revision":"r","file":"shape.json","textDigest":"unused"},"branchPointer":""})).unwrap()
    }
    #[test]
    fn ddl_display_distinguishes_composite_membership_and_omits_empty_bookkeeping() {
        let text = "CREATE TABLE pair (\"Message Key\" uuid, kind text, note text DEFAULT 'hello', PRIMARY KEY(\"Message Key\",kind)); CREATE TABLE single (id uuid PRIMARY KEY);";
        let projection = postgres::project(text);
        let original = projection.clone();
        let html = ddl_html(&projection, "en");
        assert_eq!(projection, original);
        assert_eq!(html.matches("Part of composite primary key").count(), 2);
        assert!(html.find("Composite primary key").unwrap() < html.find("<table").unwrap());
        assert!(html.contains("(&quot;Message Key&quot;, kind)"));
        assert!(!html.contains("Primary key</code>: true"));
        assert!(!html.contains("Presence"));
        assert!(!html.contains("Column declaration"));
        assert!(!html.contains("database:"));
        assert!(!html.contains("search_path"));
        assert!(html.contains("Primary key. NOT NULL follows from PRIMARY KEY"));
        assert!(
            html.contains("Default expression</code>: <code>&#39;hello&#39;</code>")
                || html.contains("Default expression</code>: <code>'hello'</code>")
        );
        let ru = ddl_html(&projection, "ru");
        assert_eq!(ru.matches("Часть составного первичного ключа").count(), 2);
        assert!(!ru.contains("Декларация столбца"));
    }
    #[test]
    fn real_context_and_material_frontier_survive_without_generic_disclaimer() {
        let text = "\\c alpha\nSET search_path TO archive; CREATE TABLE inbox (id uuid PRIMARY KEY);\n\\c beta\nCREATE TABLE inbox (id uuid PRIMARY KEY); ALTER TABLE inbox ADD CHECK(id IS NOT NULL);";
        let mut selection = sel();
        selection.format = "POSTGRES_CREATE_TABLE".into();
        let component = project(
            &selection,
            &json!({"source":{"text":text,"startLine":7}}),
            "en",
        )
        .unwrap();
        let html = component["html"].as_str().unwrap();
        assert!(html.contains("inbox · database: alpha"));
        assert!(html.contains("inbox · database: beta"));
        assert_eq!(html.matches("search_path: archive").count(), 1);
        assert!(html.contains("Partial projection (1)"));
        assert!(html.contains("source lines 10–10"));
        assert!(!html.contains("execution and runtime conformance"));
        assert!(!html.contains("not selected"));
        assert_eq!(component["projection"], postgres::project(text));
    }
    #[test]
    fn exact_source_identity_is_required_before_projection() {
        let text = "{\"type\":\"object\"}";
        let mut s = sel().source;
        s.text_digest = canonical::hash_bytes(text.as_bytes());
        let raw = json!({"id":"s","service":"neutral","revision":"r","file":"shape.json","text":text,"textDigest":s.text_digest});
        assert!(bind_record(raw.clone(), &s, "snap", Value::Null).is_ok());
        for field in ["id", "service", "revision", "file", "textDigest", "text"] {
            let mut bad = raw.clone();
            bad[field] = json!("changed");
            assert!(
                bind_record(bad, &s, "snap", Value::Null).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn json_projection_preserves_union_constraints_arrays_and_escaped_names() {
        let text=json!({"type":"object","required":["x/y"],"properties":{"x/y":{"type":["integer","null"],"minimum":2,"enum":[2,3,null]},"nested":{"type":"array","items":{"oneOf":[{"type":"boolean"},{"type":"string","maxLength":8}]}}}}).to_string();
        let bound = json!({"source":{"text":text}});
        let p = project(&sel(), &bound, "en").unwrap();
        let rows = p["projection"]["nodes"].as_array().unwrap();
        assert!(rows.iter().any(|n| n["pointer"] == "/properties/x~1y"
            && n["presence"] == "Required in parent"
            && n["nullability"] == "Allowed"));
        assert!(p["html"].as_str().unwrap().contains("minimum"));
        assert!(p["html"].as_str().unwrap().contains("maxLength"));
        assert!(rows.iter().any(|n| n["type"] == "boolean"));
    }
    #[test]
    fn openapi_status_media_and_reference_frontiers_survive() {
        let text=json!({"openapi":"3.1.0","paths":{"/records":{"get":{"responses":{"201":{"content":{"application/vnd.neutral+json":{"schema":{"type":"object","properties":{"a<b":{"type":"string","format":"uuid"}}}}}},"503":{"$ref":"#/components/responses/Busy"}}}}}}).to_string();
        let mut s = sel();
        s.format = "OPENAPI".into();
        let p = project(&s, &json!({"source":{"text":text}}), "en").unwrap();
        let html = p["html"].as_str().unwrap();
        assert!(html.contains("201"));
        assert!(html.contains("503"));
        assert!(html.contains("application/vnd.neutral+json"));
        assert!(html.contains("a&lt;b"));
        assert_eq!(
            p["projection"]["authority"],
            "RETAINED_DECLARATION_NOT_VALIDATOR_OR_RUNTIME_PROOF"
        );
    }

    #[test]
    fn source_component_roundtrip_uses_a_real_saved_check_and_immutable_cache() {
        use super::super::{
            check,
            model::{Service, ServiceEvidence},
        };
        use std::collections::BTreeMap;
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Neutral declaration components").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let service:Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"neutral","title":"Neutral","repositoryId":"neutral","repository":"https://example.invalid/neutral","language":"java","profile":"source-syntax","targetRef":"HEAD","source":{"roots":["."],"dialect":"17"},"contractFiles":["shape.json"]})).unwrap();
        repo.service_add(service.clone(), Some(&repo.input_digest().unwrap()))
            .unwrap();
        let text = "{\"type\":\"object\",\"properties\":{\"value\":{\"type\":\"integer\"}}}";
        let mut evidence:ServiceEvidence=serde_json::from_value(json!({"schema":"codeclew-documentation-service-evidence/1.0","service":"neutral","revision":"neutral-revision","serviceDigest":digest(&service).unwrap(),"extractor":super::super::model::SOURCE_EXTRACTOR,"runtimeMode":"COMMITTED_SOURCE_NO_BUILD","coverage":"SYNTAX","boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}})).unwrap();
        super::super::contracts::import(
            &service,
            &mut evidence,
            &BTreeMap::from([("shape.json".into(), text.into())]),
        )
        .unwrap();
        let source = evidence.sources.values().next().unwrap().clone();
        let mut checked = check::assemble(
            repo.input_digest().unwrap(),
            BTreeMap::from([("neutral".into(), evidence)]),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        checked.source_inputs = Some(check::SourceInputs {
            schema: check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest: checked.input_digest.clone(),
            inputs: repo.inputs().unwrap(),
            selected_services: BTreeSet::from(["neutral".into()]),
            retained_services: BTreeSet::new(),
        });
        let snapshot = checked.save_snapshot(&repo).unwrap();
        let mut selection = sel();
        selection.source = SourceSelector {
            kind: "SOURCE".into(),
            service: source.service.clone(),
            id: source.id.clone(),
            revision: source.revision.clone(),
            file: source.file.clone(),
            text_digest: source.text_digest.clone(),
            evidence_set: None,
        };
        let mut request = Request {
            schema: REQUEST.into(),
            snapshot: snapshot.clone(),
            language: "en".into(),
            components: vec![selection],
        };
        let input = temp.path().join("selection.json");
        std::fs::write(&input, super::super::bytes(&request).unwrap()).unwrap();
        let selected = prepare(&repo, &input).unwrap();
        let ticket_path = temp.path().join("ticket.json");
        std::fs::write(&ticket_path, super::super::bytes(&selected).unwrap()).unwrap();
        let replay = read(&repo, &ticket_path).unwrap();
        assert_eq!(
            replay["packet"]["components"][0]["binding"]["source"]["text"],
            text
        );
        assert_eq!(
            replay["packet"]["components"][0]["binding"]["range"]["byteEnd"],
            text.len()
        );
        assert_eq!(
            Check::load_snapshot(&repo, &snapshot).unwrap().sources()[&source.id],
            source
        );
        request.components[0].source.revision = "other".into();
        std::fs::write(&input, super::super::bytes(&request).unwrap()).unwrap();
        assert!(prepare(&repo, &input).is_err());
        let mut tampered = selected.clone();
        tampered["components"][0]["id"] = json!("other");
        std::fs::write(&ticket_path, super::super::bytes(&tampered).unwrap()).unwrap();
        assert!(read(&repo, &ticket_path).is_err());
    }
}
