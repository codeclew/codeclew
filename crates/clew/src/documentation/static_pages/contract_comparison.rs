//! Retained external HTTP observations, admitted against one exact selected snapshot.
//! Integrity/binding checks do not execute the probe or repeat its schema validation.
use super::super::{check::Check, invalid, io_error, store};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Packet {
    schema: String,
    publication: String,
    snapshot: String,
    service: String,
    revision: String,
    scope: String,
    limitations: Vec<String>,
    sources: Vec<Binding>,
    artifacts: BTreeMap<String, Artifact>,
    probe_artifact: String,
    assessment_artifact: String,
    entries: Vec<Entry>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Binding {
    source_id: String,
    revision: String,
    text_digest: String,
    artifact: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    text: String,
    digest: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    entrypoint: String,
    contract_id: String,
    contract_digest: String,
    method: String,
    path: String,
    coverage: String,
}
fn text(value: &Value, key: &str) -> Result<String, ClewError> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("HTTP comparison string is missing"))
}
fn bounded(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit
}
fn artifact_json(packet: &Packet, id: &str) -> Result<Value, ClewError> {
    let artifact = packet
        .artifacts
        .get(id)
        .ok_or_else(|| invalid("HTTP comparison artifact is missing"))?;
    serde_json::from_str(&artifact.text).map_err(io_error)
}
fn route_matches(template: &str, path: &str) -> bool {
    let a: Vec<_> = template.split('/').collect();
    let b: Vec<_> = path.split('/').collect();
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| *a == b || (a.starts_with('{') && a.ends_with('}') && !b.is_empty()))
}
fn response_declaration(operation: &Value, status: u64) -> &Value {
    let responses = &operation["responses"];
    responses
        .get(status.to_string())
        .or_else(|| responses.get(format!("{}XX", status / 100)))
        .or_else(|| responses.get("default"))
        .unwrap_or(&Value::Null)
}
fn declared_media(content: &serde_json::Map<String, Value>, actual: &str) -> bool {
    let range = actual.split_once('/').map(|(kind, _)| format!("{kind}/*"));
    content.keys().any(|key| {
        let key = media(key);
        key == actual || key == "*/*" || range.as_ref() == Some(&key)
    })
}
fn media(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

pub(super) fn load(
    path: Option<&Path>,
    publication: &str,
    snapshot: &str,
    checked: &Check,
) -> Result<Option<Value>, ClewError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let packet: Packet = store::read(path, 1024 * 1024)?;
    project(packet, publication, snapshot, checked).map(Some)
}
fn project(
    packet: Packet,
    publication: &str,
    snapshot: &str,
    checked: &Check,
) -> Result<Value, ClewError> {
    if packet.schema != "codeclew-http-comparison/1.0"
        || packet.publication != publication
        || packet.snapshot != snapshot
        || !bounded(&packet.scope, 2048)
        || packet.limitations.is_empty()
        || packet.limitations.len() > 16
        || packet.limitations.iter().any(|v| !bounded(v, 1024))
        || packet.sources.is_empty()
        || packet.sources.len() > 16
        || packet.artifacts.is_empty()
        || packet.artifacts.len() > 16
        || packet.entries.is_empty()
        || packet.entries.len() > 64
    {
        return Err(invalid(
            "HTTP comparison scope, snapshot or bounds are invalid",
        ));
    }
    let service = checked
        .services
        .get(&packet.service)
        .ok_or_else(|| invalid("HTTP comparison service is absent"))?;
    if service.revision != packet.revision {
        return Err(invalid("HTTP comparison source revision changed"));
    }
    for (id, artifact) in &packet.artifacts {
        if !store::valid_id(id)
            || artifact.text.len() > 256 * 1024
            || crate::canonical::hash_bytes(artifact.text.as_bytes()) != artifact.digest
        {
            return Err(invalid("HTTP comparison artifact digest or bounds differ"));
        }
    }
    let mut sources = BTreeMap::new();
    let mut artifact_bindings = Vec::new();
    for binding in &packet.sources {
        let source = service
            .sources
            .get(&binding.source_id)
            .ok_or_else(|| invalid("HTTP comparison selected source is absent"))?;
        let artifact = packet
            .artifacts
            .get(&binding.artifact)
            .ok_or_else(|| invalid("HTTP comparison source artifact is absent"))?;
        // Compiler spans can begin at the first token rather than column zero.
        // Locate the exact captured bytes inside their declared line window;
        // only leading indentation and one final newline may be outside it.
        if source.start_line == 0 || source.end_line < source.start_line || source.text.is_empty() {
            return Err(invalid("HTTP comparison source line bounds are invalid"));
        }
        let lines: Vec<_> = artifact.text.split_inclusive('\n').collect();
        if source.end_line > lines.len() as u64 {
            return Err(invalid(
                "HTTP comparison declared lines exceed the source artifact",
            ));
        }
        let window_start = lines
            .iter()
            .take(source.start_line as usize - 1)
            .map(|v| v.len())
            .sum::<usize>();
        let span = lines
            .iter()
            .skip(source.start_line as usize - 1)
            .take((source.end_line - source.start_line + 1) as usize)
            .copied()
            .collect::<String>();
        let offset = span.find(&source.text).filter(|offset| {
            span[..*offset].bytes().all(|b| matches!(b, b' ' | b'\t'))
                && matches!(&span[*offset + source.text.len()..], "" | "\n")
        });
        if binding.revision != source.revision
            || binding.revision != packet.revision
            || binding.text_digest != source.text_digest
            || crate::canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
            || offset.is_none()
            || sources.insert(binding.source_id.clone(), source).is_some()
        {
            return Err(invalid(format!(
                "HTTP comparison source bytes, span or revision differ: {}",
                binding.source_id
            )));
        }
        artifact_bindings.push(json!({"sourceId":binding.source_id,"artifact":binding.artifact,"artifactDigest":artifact.digest,"startByte":window_start+offset.unwrap(),"endByte":window_start+offset.unwrap()+source.text.len(),"match":"EXACT_CAPTURED_BYTES_WITHIN_DECLARED_LINES"}));
    }
    let probe = artifact_json(&packet, &packet.probe_artifact)?;
    let assessment = artifact_json(&packet, &packet.assessment_artifact)?;
    let raw_cases = probe["cases"]
        .as_array()
        .ok_or_else(|| invalid("HTTP comparison probe cases are absent"))?;
    let results = assessment["results"]
        .as_array()
        .ok_or_else(|| invalid("HTTP comparison assessments are absent"))?;
    if raw_cases.len() > 128
        || results.len() != raw_cases.len()
        || !bounded(&text(&assessment, "validator")?, 256)
    {
        return Err(invalid(
            "HTTP comparison case bounds or validator are invalid",
        ));
    }
    let mut ids = BTreeSet::new();
    for case in raw_cases {
        let id = text(case, "id")?;
        if !store::valid_id(&id) || !ids.insert(id) {
            return Err(invalid("HTTP comparison probe case identity repeats"));
        }
    }
    let mut result_ids = BTreeSet::new();
    for result in results {
        if !result_ids.insert(text(result, "id")?) {
            return Err(invalid("HTTP comparison assessment identity repeats"));
        }
    }
    if ids != result_ids {
        return Err(invalid("HTTP comparison cases and assessments differ"));
    }
    let mut seen = BTreeSet::new();
    let mut used_cases = BTreeSet::new();
    let mut entries = Vec::new();
    for entry in &packet.entries {
        if !seen.insert(&entry.contract_id) || !bounded(&entry.coverage, 2048) {
            return Err(invalid(
                "HTTP comparison repeats an entry or lacks coverage",
            ));
        }
        let contract = service
            .observations
            .get(&entry.contract_id)
            .filter(|o| o.kind == "CONTRACT_OPERATION")
            .ok_or_else(|| invalid("HTTP comparison contract is absent"))?;
        let n = &contract.normalized;
        let handler = service
            .entrypoints
            .iter()
            .find(|e| e.id == entry.entrypoint)
            .ok_or_else(|| invalid("HTTP comparison handler is absent"))?;
        if contract.digest != entry.contract_digest
            || n["method"] != entry.method
            || n["path"] != entry.path
            || n["entrypoint"] != entry.entrypoint
            || n["sourceMapping"] != "SOURCE_ROUTE_MATCH_ONLY"
            || handler.kind != "HTTP_ENDPOINT"
            || !handler.trigger["methods"]
                .as_array()
                .is_some_and(|a| a.contains(&json!(entry.method)))
            || !handler.trigger["paths"]
                .as_array()
                .is_some_and(|a| a.contains(&json!(entry.path)))
            || contract
                .source_ids
                .iter()
                .chain(&handler.source_ids)
                .any(|id| !sources.contains_key(id))
        {
            return Err(invalid(
                "HTTP comparison route, contract or handler binding changed",
            ));
        }
        let mut cases = Vec::new();
        for result in results
            .iter()
            .filter(|r| r["contractId"] == entry.contract_id)
        {
            let id = text(result, "id")?;
            used_cases.insert(id.clone());
            let raw = raw_cases.iter().find(|c| c["id"] == id).unwrap();
            let status = raw["response"]["status"]
                .as_u64()
                .filter(|s| (100..=599).contains(s))
                .ok_or_else(|| invalid("HTTP comparison response status is invalid"))?;
            let response_media = media(&text(&raw["response"], "contentType")?);
            let declaration = response_declaration(&n["operation"], status);
            let status_declared = !declaration.is_null();
            let content = declaration["content"].as_object();
            let media_assessment = if content.is_none_or(|c| c.is_empty()) {
                "not-specified"
            } else if declared_media(content.unwrap(), &response_media) {
                "declared"
            } else {
                "undeclared-media"
            };
            if result["contractDigest"] != entry.contract_digest
                || result["method"] != entry.method
                || result["route"] != entry.path
                || raw["request"]["method"] != entry.method
                || !route_matches(&entry.path, &text(&raw["request"], "path")?)
                || result["observedStatus"] != status
                || result["observedMedia"] != response_media
                || result["statusDeclared"] != status_declared
                || result["mediaAssessment"] != media_assessment
            {
                return Err(invalid(
                    "HTTP comparison assessment differs from recorded HTTP or selected contract",
                ));
            }
            let request = text(result, "requestAssessment")?;
            let body = text(result, "bodyAssessment")?;
            if !matches!(
                request.as_str(),
                "within-schema"
                    | "outside-schema"
                    | "undeclared-media"
                    | "no-body-declared"
                    | "not-checked"
            ) || !matches!(
                body.as_str(),
                "matches-selected-schema"
                    | "violates-selected-schema"
                    | "not-specified"
                    | "not-checked"
            ) {
                return Err(invalid("HTTP comparison schema assessment is unknown"));
            }
            if body == "matches-selected-schema" || body == "violates-selected-schema" {
                let selected = text(result, "comparedSchemaMedia")?;
                if declaration["content"][&selected]["schema"].is_null() {
                    return Err(invalid("HTTP comparison assessed schema is absent"));
                }
            }
            cases.push(json!({"id":id,"request":raw["request"],"response":raw["response"],"statusDeclared":status_declared,"declaredMedia":content.map(|c|c.keys().collect::<Vec<_>>()).unwrap_or_default(),"mediaAssessment":media_assessment,"bodyAssessment":body,"requestAssessment":request,"comparedSchemaMedia":result["comparedSchemaMedia"],"requestErrors":result["requestErrors"],"bodyErrors":result["bodyErrors"]}));
        }
        let mut findings = Vec::new();
        for (kind, matching) in [
            (
                "BROADER_REQUEST_ACCEPTANCE",
                cases
                    .iter()
                    .filter(|c| {
                        c["requestAssessment"] == "outside-schema"
                            && c["response"]["status"]
                                .as_u64()
                                .is_some_and(|s| (200..300).contains(&s))
                    })
                    .map(|c| c["id"].clone())
                    .collect::<Vec<_>>(),
            ),
            (
                "UNDECLARED_RESPONSE_MEDIA",
                cases
                    .iter()
                    .filter(|c| c["mediaAssessment"] == "undeclared-media")
                    .map(|c| c["id"].clone())
                    .collect(),
            ),
            (
                "UNDECLARED_RESPONSE_STATUS",
                cases
                    .iter()
                    .filter(|c| c["statusDeclared"] == false)
                    .map(|c| c["id"].clone())
                    .collect(),
            ),
            (
                "REPORTED_BODY_SCHEMA_DIFFERENCE",
                cases
                    .iter()
                    .filter(|c| c["bodyAssessment"] == "violates-selected-schema")
                    .map(|c| c["id"].clone())
                    .collect(),
            ),
        ] {
            if !matching.is_empty() {
                findings.push(json!({"kind":kind,"caseIds":matching}));
            }
        }
        entries.push(json!({"entrypoint":entry.entrypoint,"contractId":entry.contract_id,"contractDigest":entry.contract_digest,"method":entry.method,"path":entry.path,"coverage":entry.coverage,"cases":cases,"findings":findings}));
    }
    if used_cases != ids {
        return Err(invalid("HTTP comparison contains unbound case assessments"));
    }
    Ok(
        json!({"schema":"codeclew-http-comparison-view/1.0","service":packet.service,"revision":packet.revision,"snapshot":packet.snapshot,"publication":packet.publication,"bindingStatus":"MATCHES_PINNED_SELECTED_SNAPSHOT","authority":"EXTERNAL_RECORDED_LOCAL_PROBE","schemaValidation":"RETAINED_EXTERNAL_ASSESSMENT_NOT_REEXECUTED","validator":assessment["validator"],"scope":packet.scope,"limitations":packet.limitations,"entries":entries,"sources":sources,"artifactBindings":artifact_bindings,"artifacts":packet.artifacts,"packetDigest":super::super::digest(&packet)?}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, Check) {
        let hash = crate::canonical::hash_bytes(b"handler");
        let check: Check = serde_json::from_value(json!({"schema":"check","inputDigest":"input","contextDigest":"context","services":{"sample":{
            "schema":"service","service":"sample","revision":"revision","serviceDigest":"service-digest","extractor":"syntax","runtimeMode":"pinned","coverage":"PARTIAL","boundaries":[],"contracts":{},
            "sources":{"source":{"id":"source","service":"sample","revision":"revision","file":"Handler.cs","startLine":1,"endLine":1,"text":"handler","textDigest":hash,"evidenceDigest":"evidence","authority":"EXACT_SNAPSHOT_TEXT","url":null}},
            "entrypoints":[{"id":"handler","service":"sample","symbol":"Handle","kind":"HTTP_ENDPOINT","trigger":{"methods":["POST"],"paths":["/work"]},"sourceIds":["source"],"dependencyIds":[],"boundaries":[]}],
            "observations":{"contract":{"id":"contract","kind":"CONTRACT_OPERATION","service":"sample","symbol":"Handle","digest":"contract-digest","sourceIds":["source"],"normalized":{"method":"POST","path":"/work","entrypoint":"handler","sourceMapping":"SOURCE_ROUTE_MATCH_ONLY","operation":{"responses":{"200":{"content":{"application/json":{"schema":{"type":"object"}}}},"400":{"content":{"application/json":{"schema":{"type":"object"}}}}}}}}}}
        },"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap();
        let raw = json!({"cases":[
            {"id":"extra","request":{"method":"POST","path":"/work","body":"{\"extra\":true}"},"response":{"status":200,"contentType":"application/json","body":"{}"}},
            {"id":"invalid","request":{"method":"POST","path":"/work","body":"{}"},"response":{"status":400,"contentType":"application/problem+json; charset=utf-8","body":"{}"}}
        ]});
        let report = json!({"validator":"external Draft 2020-12 check","results":[
            {"id":"extra","contractId":"contract","contractDigest":"contract-digest","method":"POST","route":"/work","observedStatus":200,"observedMedia":"application/json","statusDeclared":true,"mediaAssessment":"declared","requestAssessment":"outside-schema","bodyAssessment":"matches-selected-schema","comparedSchemaMedia":"application/json"},
            {"id":"invalid","contractId":"contract","contractDigest":"contract-digest","method":"POST","route":"/work","observedStatus":400,"observedMedia":"application/problem+json","statusDeclared":true,"mediaAssessment":"undeclared-media","requestAssessment":"outside-schema","bodyAssessment":"matches-selected-schema","comparedSchemaMedia":"application/json"}
        ]});
        let artifact = |v: Value| {
            let text = serde_json::to_string(&v).unwrap();
            json!({"digest":crate::canonical::hash_bytes(text.as_bytes()),"text":text})
        };
        (
            json!({"schema":"codeclew-http-comparison/1.0","publication":"publication","snapshot":"snapshot","service":"sample","revision":"revision","scope":"Local probe with substituted persistence","limitations":["No deployed runtime or database proof"],"sources":[{"sourceId":"source","revision":"revision","textDigest":hash,"artifact":"source"}],"artifacts":{"source":{"text":"handler\n","digest":crate::canonical::hash_bytes(b"handler\n")},"probe":artifact(raw),"assessment":artifact(report)},"probeArtifact":"probe","assessmentArtifact":"assessment","entries":[{"entrypoint":"handler","contractId":"contract","contractDigest":"contract-digest","method":"POST","path":"/work","coverage":"Two observed cases; failures not exhaustively exercised"}]}),
            check,
        )
    }
    fn run(packet: Value, checked: &Check) -> Result<Value, ClewError> {
        project(
            serde_json::from_value(packet).unwrap(),
            "publication",
            "snapshot",
            checked,
        )
    }
    #[test]
    fn separates_external_schema_checks_status_and_media_and_retains_provenance() {
        let (packet, check) = fixture();
        let result = run(packet.clone(), &check).unwrap();
        assert_eq!(
            result["entries"][0]["findings"],
            json!([
                {"kind":"BROADER_REQUEST_ACCEPTANCE","caseIds":["extra"]},
                {"kind":"UNDECLARED_RESPONSE_MEDIA","caseIds":["invalid"]}
            ])
        );
        assert_eq!(result["entries"][0]["cases"][1]["statusDeclared"], true);
        assert_eq!(
            result["entries"][0]["cases"][1]["bodyAssessment"],
            "matches-selected-schema"
        );
        assert_eq!(result["artifacts"], packet["artifacts"]);
        assert_eq!(result["authority"], "EXTERNAL_RECORDED_LOCAL_PROBE");
        assert_eq!(
            result["schemaValidation"],
            "RETAINED_EXTERNAL_ASSESSMENT_NOT_REEXECUTED"
        );
    }
    #[test]
    fn locates_exact_compiler_bytes_after_first_line_indent_without_normalizing_them() {
        let (mut packet, check) = fixture();
        let text = "    handler\n";
        packet["artifacts"]["source"] =
            json!({"text":text,"digest":crate::canonical::hash_bytes(text.as_bytes())});
        let result = run(packet.clone(), &check).unwrap();
        assert_eq!(result["artifactBindings"][0]["startByte"], 4);
        assert_eq!(result["artifactBindings"][0]["endByte"], 11);
        assert_eq!(result["sources"]["source"]["text"], "handler");
        let mut wrong_lines = check.clone();
        wrong_lines
            .services
            .get_mut("sample")
            .unwrap()
            .sources
            .get_mut("source")
            .unwrap()
            .end_line = 2;
        assert!(run(packet.clone(), &wrong_lines).is_err());
        for text in ["prefix handler\n", "    hand ler\n", "    handler suffix\n"] {
            packet["artifacts"]["source"] =
                json!({"text":text,"digest":crate::canonical::hash_bytes(text.as_bytes())});
            assert!(run(packet.clone(), &check).is_err());
        }
    }
    #[test]
    fn rejects_stale_revision_route_contract_snapshot_source_and_corrupt_artifact() {
        let (packet, check) = fixture();
        for pointer in [
            "/revision",
            "/snapshot",
            "/publication",
            "/entries/0/contractDigest",
            "/entries/0/entrypoint",
            "/entries/0/path",
            "/sources/0/textDigest",
            "/sources/0/revision",
            "/artifacts/probe/text",
        ] {
            let mut changed = packet.clone();
            *changed.pointer_mut(pointer).unwrap() = json!("changed");
            assert!(run(changed, &check).is_err(), "{pointer}");
        }
        let mut changed = check.clone();
        changed
            .services
            .get_mut("sample")
            .unwrap()
            .sources
            .get_mut("source")
            .unwrap()
            .text = "other handler".into();
        assert!(run(packet, &changed).is_err());
    }
    #[test]
    fn rejects_unbound_or_fabricated_status_media_and_missing_assessments() {
        let (packet, check) = fixture();
        for pointer in [
            "/results/1/observedStatus",
            "/results/1/observedMedia",
            "/results/1/contractId",
            "/results/1/id",
            "/results/1/bodyAssessment",
        ] {
            let mut changed = packet.clone();
            let mut report: Value =
                serde_json::from_str(changed["artifacts"]["assessment"]["text"].as_str().unwrap())
                    .unwrap();
            *report.pointer_mut(pointer).unwrap() = json!("different");
            let text = serde_json::to_string(&report).unwrap();
            changed["artifacts"]["assessment"] =
                json!({"digest":crate::canonical::hash_bytes(text.as_bytes()),"text":text});
            assert!(run(changed, &check).is_err(), "{pointer}");
        }
        let mut changed = packet.clone();
        changed["entries"] = json!([]);
        assert!(run(changed, &check).is_err());
        let mut changed = packet;
        changed["sources"] = json!([]);
        assert!(run(changed, &check).is_err());
    }
    #[test]
    fn openapi_response_ranges_and_media_ranges_do_not_create_false_differences() {
        let op = json!({"responses":{"400":{"description":"exact"},"4XX":{"description":"range"},"default":{"description":"default"}}});
        assert_eq!(response_declaration(&op, 400)["description"], "exact");
        assert_eq!(response_declaration(&op, 404)["description"], "range");
        assert_eq!(response_declaration(&op, 503)["description"], "default");
        for declaration in [
            "application/json",
            "application/*",
            "*/*",
            "Application/JSON",
        ] {
            let content = json!({declaration:{"schema":{}}});
            assert!(declared_media(
                content.as_object().unwrap(),
                "application/json"
            ));
        }
        assert!(!declared_media(
            json!({"text/*":{}}).as_object().unwrap(),
            "application/json"
        ));
        assert!(route_matches("/work/{id}/refresh", "/work/one/refresh"));
        assert!(!route_matches(
            "/work/{id}/refresh",
            "/work/one/two/refresh"
        ));
    }
}
