//! Bounded reading summaries, never schema validation or reference resolution.
use super::{details, fence, md, reading, rows, text};
use crate::documentation::io_error;
use crate::error::ClewError;
use serde_json::Value;
use std::collections::BTreeSet;

fn literal(value: &str) -> String {
    crate::documentation::render::escape(value)
        .replace('{', "&#123;")
        .replace('}', "&#125;")
}
pub(super) fn schema_type(schema: &Value) -> String {
    schema_type_at(schema, 0)
}
fn schema_type_at(schema: &Value, depth: usize) -> String {
    if depth > 6 {
        return "See exact retained schema".into();
    }
    if schema.is_null() {
        return "Not declared".into();
    }
    if schema == &Value::Bool(false) {
        return "No value allowed (false schema)".into();
    }
    if schema == &Value::Bool(true) {
        return "Any value (true schema)".into();
    }
    if let Some(reference) = schema["$ref"].as_str() {
        return format!("Reference: {reference}");
    }
    if schema["type"] == "array" {
        return format!("array<{}>", schema_type_at(&schema["items"], depth + 1));
    }
    let kind = schema["type"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            schema["type"]
                .as_array()
                .map(|_| schema["type"].to_string())
        })
        .unwrap_or_else(|| {
            ["allOf", "oneOf", "anyOf"]
                .into_iter()
                .find(|k| schema[*k].is_array())
                .unwrap_or("Not declared")
                .to_owned()
        });
    if let Some(format) = schema["format"].as_str() {
        format!("{kind} · {format}")
    } else {
        kind
    }
}
pub(super) fn constraints(schema: &Value) -> String {
    let mut values = Vec::new();
    for key in [
        "const",
        "format",
        "enum",
        "pattern",
        "minLength",
        "maxLength",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "minItems",
        "maxItems",
        "uniqueItems",
        "nullable",
        "default",
        "readOnly",
        "writeOnly",
        "additionalProperties",
    ] {
        if let Some(value) = schema.get(key) {
            values.push(format!(
                "{key}: {}",
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            ));
        }
    }
    values.join("; ")
}
pub(super) fn fields(schema: &Value, depth: usize) -> String {
    if depth > 6 {
        return "<p>See the exact retained schema for deeper fields.</p>\n\n".into();
    }
    if schema.is_null() {
        return "<p>No schema declared.</p>\n\n".into();
    }
    if !schema.is_object() && !schema.is_boolean() {
        return "<p>Unsupported schema value; see the exact retained schema.</p>\n\n".into();
    }
    if schema["$ref"].is_string() {
        return format!(
            "<p><code>{}</code> · Referenced schema retained without expansion.</p>\n\n",
            literal(&schema_type(schema))
        );
    }
    let mut out = String::new();
    for kind in ["allOf", "oneOf", "anyOf"] {
        if let Some(branches) = schema[kind].as_array() {
            out.push_str(&format!("<p>{kind} composition · branches remain separate; this reader does not merge their validation rules.</p>\n\n"));
            for (index, branch) in branches.iter().enumerate() {
                out.push_str(&details(
                    &format!("{kind} branch {}", index + 1),
                    &fields(branch, depth + 1),
                ));
            }
        }
    }
    let declared = constraints(schema);
    out.push_str(&format!(
        "<p className=\"clew-schema-scope\">{}{}{}</p>\n\n",
        literal(&schema_type(schema)),
        if declared.is_empty() { "" } else { " · " },
        literal(&declared)
    ));
    if schema["type"] == "array" {
        out.push_str(&fields(&schema["items"], depth + 1));
        return out;
    }
    if let Some(properties) = schema["properties"].as_object().filter(|p| !p.is_empty()) {
        out.push_str("<div className=\"clew-table-scroll\"><table className=\"clew-schema-table\"><thead><tr><th>Field</th><th>Type</th><th>Required</th><th>Declared constraints / description</th></tr></thead><tbody>\n");
        for (name, property) in properties {
            let required = match schema.get("required") {
                None => "Optional",
                Some(Value::Array(names)) if names.iter().all(Value::is_string) => {
                    if names.iter().any(|v| v == name) {
                        "Required"
                    } else {
                        "Optional"
                    }
                }
                _ => "Requiredness unknown",
            };
            let mut nested = String::new();
            if property["properties"].is_object()
                || property["items"].is_object()
                || ["allOf", "oneOf", "anyOf"]
                    .iter()
                    .any(|k| property[*k].is_array())
            {
                nested = details("Nested fields", &fields(property, depth + 1));
            }
            let declared = constraints(property);
            out.push_str(&format!("<tr><td><code>{}</code></td><td>{}{nested}</td><td>{required}</td><td>{}{}{}</td></tr>\n", literal(name), literal(&schema_type(property)), literal(&declared), if declared.is_empty() { "" } else { "<br/>" }, literal(text(&property["description"]))));
        }
        out.push_str("</tbody></table></div>\n\n");
    }
    out
}
pub(super) fn content(content: &Value) -> Result<String, ClewError> {
    let mut out = String::new();
    for (media, body) in content.as_object().into_iter().flat_map(|m| m.iter()) {
        out.push_str(&format!(
            "**Media:** {}\n\n{}",
            md(media),
            fields(&body["schema"], 0)
        ));
        if !body["example"].is_null() {
            out.push_str(&details(
                "Declared example · not an observation",
                &fence(
                    &serde_json::to_string_pretty(&body["example"]).map_err(io_error)?,
                    "json",
                ),
            ));
        }
        out.push_str(&details(
            "Exact retained media schema and examples",
            &fence(
                &serde_json::to_string_pretty(body).map_err(io_error)?,
                "json",
            ),
        ));
    }
    if out.is_empty() {
        out.push_str("No response media or body schema is recorded for this status in the retained declaration.\n\n");
    }
    Ok(out)
}
pub(super) fn parameters(parameters: &Value) -> Result<String, ClewError> {
    if rows(parameters).next().is_none() {
        return Ok("No parameters declared.\n\n".into());
    }
    let mut out = String::from(
        "<div className=\"clew-table-scroll\"><table className=\"clew-schema-table\"><thead><tr><th>Parameter</th><th>Location / type</th><th>Required</th><th>Declared constraints</th></tr></thead><tbody>",
    );
    for p in rows(parameters) {
        out.push_str(&format!(
            "<tr><td><code>{}</code><br/>{}</td><td>{} / {}</td><td>{}</td><td>{}</td></tr>",
            literal(text(&p["name"])),
            literal(text(&p["description"])),
            literal(text(&p["in"])),
            literal(&schema_type(&p["schema"])),
            if p["required"] == true {
                "Required"
            } else {
                "Optional"
            },
            literal(&constraints(&p["schema"]))
        ));
    }
    out.push_str("</tbody></table></div>\n\n");
    out.push_str(&details(
        "Exact retained parameter declarations",
        &fence(
            &serde_json::to_string_pretty(parameters).map_err(io_error)?,
            "json",
        ),
    ));
    Ok(out)
}
pub(super) fn comparison_current(page: &Value, entry: &Value) -> bool {
    let c = &page["contractComparison"];
    let service = text(&c["service"]);
    c["schema"] == "codeclew-http-comparison-view/1.0"
        && c["bindingStatus"] == "MATCHES_PINNED_SELECTED_SNAPSHOT"
        && c["snapshot"] == page["selectedSnapshot"]
        && c["revision"] == page["selectedEvidence"]["revisions"][service]
        && page["subject"] == format!("service:{service}")
        && rows(&page["contracts"]).any(|r| {
            r["id"] == entry["contractId"]
                && r["digest"] == entry["contractDigest"]
                && r["normalized"]["entrypoint"] == entry["entrypoint"]
                && r["normalized"]["method"] == entry["method"]
                && r["normalized"]["path"] == entry["path"]
        })
}
pub(super) fn findings(entry: &Value) -> String {
    let mut out = String::new();
    for finding in rows(&entry["findings"]) {
        let (title, explanation) = match text(&finding["kind"]) {
            "BROADER_REQUEST_ACCEPTANCE" => (
                "Broader request acceptance",
                "A recorded request outside the declared schema received a successful response. This shows broader acceptance; it does not show failure for a contract-valid request.",
            ),
            "UNDECLARED_RESPONSE_MEDIA" => (
                "Undeclared response media type",
                "Recorded responses use a media type absent from the selected status declaration. Status-code and body-schema results are separate below.",
            ),
            "UNDECLARED_RESPONSE_STATUS" => (
                "Undeclared response status",
                "A recorded response status is absent from the selected contract.",
            ),
            "REPORTED_BODY_SCHEMA_DIFFERENCE" => (
                "Reported body-schema difference",
                "The retained external validator reports a difference against its selected response schema.",
            ),
            _ => continue,
        };
        out.push_str(&format!(
            "<aside className=\"clew-http-finding\" role=\"status\">\n\n#### {}\n\n{}\n\n",
            md(title),
            md(explanation)
        ));
        let cases = rows(&entry["cases"])
            .filter(|c| rows(&finding["caseIds"]).any(|id| id == &c["id"]))
            .collect::<Vec<_>>();
        if finding["kind"] == "BROADER_REQUEST_ACCEPTANCE" {
            for c in cases.iter().take(2) {
                let body = text(&c["request"]["body"]);
                let excerpt = body.chars().take(256).collect::<String>();
                let response = format!(
                    "{} {}",
                    c["response"]["status"],
                    text(&c["response"]["contentType"])
                        .split(';')
                        .next()
                        .unwrap_or("")
                );
                out.push_str(&format!(
                    "<p>Recorded request{}: <code>{}</code> → <code>{}</code>.</p>\n\n",
                    if body.chars().count() > 256 {
                        " excerpt"
                    } else {
                        ""
                    },
                    literal(&excerpt),
                    literal(&response)
                ));
                if let Some(error) = rows(&c["requestErrors"])
                    .next()
                    .and_then(|v| v["message"].as_str())
                {
                    out.push_str(&format!(
                        "External schema check: {}\n\n",
                        md(&error.chars().take(256).collect::<String>())
                    ));
                }
            }
        }
        if finding["kind"] == "UNDECLARED_RESPONSE_MEDIA" {
            let mut seen = BTreeSet::new();
            for c in cases {
                let observed = format!(
                    "{} {}",
                    c["response"]["status"],
                    text(&c["response"]["contentType"])
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                );
                let declared = rows(&c["declaredMedia"])
                    .map(text)
                    .collect::<Vec<_>>()
                    .join(", ");
                let declared = if declared.is_empty() {
                    "No media specified".into()
                } else {
                    declared
                };
                if seen.insert((observed.clone(), declared.clone())) {
                    out.push_str(&format!("<p>Observed: <code>{}</code>. Declared for this status: <code>{}</code>.</p>\n\n",literal(&observed),literal(&declared)));
                }
            }
        }
        out.push_str(&format!(
            "Cases: {}\n\n</aside>\n\n",
            rows(&finding["caseIds"])
                .map(|v| md(text(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out
}
pub(super) fn representations(field: &Value, catalog: &Value) -> String {
    if rows(&field["representations"]).next().is_none() {
        return String::new();
    }
    let mut out = String::from(
        "<div className=\"clew-table-scroll\"><table className=\"clew-schema-table\"><thead><tr><th>Representation</th><th>Type / declared constraints</th><th>Requiredness / nullability</th><th>Retained mapping</th></tr></thead><tbody>",
    );
    for rep in rows(&field["representations"]) {
        let leaf = rows(&catalog["contracts"])
            .find(|c| c["id"] == rep["contract"] && rep["contract"].is_string())
            .and_then(|c| rows(&c["leaves"]).find(|l| l["field"] == rep["field"]));
        let (kind, required, nullable) = if let Some(leaf) = leaf {
            (
                format!(
                    "{} · {}",
                    schema_type(&leaf["schema"]),
                    constraints(&leaf["schema"])
                ),
                match leaf["requiredWhenBranchMatches"] {
                    Value::Bool(true) => "Required when branch matches".into(),
                    Value::Bool(false) => "Not required by this branch".into(),
                    _ => "Requiredness unknown".into(),
                },
                match leaf["nullable"] {
                    Value::Bool(true) => "Nullable".into(),
                    Value::Bool(false) => "Not nullable".into(),
                    _ => "Nullability unknown".into(),
                },
            )
        } else {
            (
                text(&rep["typeLabel"]).to_owned(),
                text(&rep["requiredness"]).to_owned(),
                text(&rep["nullability"]).to_owned(),
            )
        };
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}<br/>{}</td><td>{}</td></tr>",
            literal(text(&rep["label"])),
            literal(&kind),
            literal(&required),
            literal(&nullable),
            reading::inline(text(&rep["mapping"]["text"]))
        ));
    }
    out.push_str("</tbody></table></div>\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn schema_reader_keeps_references_compositions_and_unknowns_bounded() {
        let schema = json!({"type":"object","required":["cityId"],"additionalProperties":false,"properties":{"cityId":{"type":"string","enum":["oslo","bergen"]},"nested":{"$ref":"#/components/schemas/City"}}});
        let output = fields(&schema, 0);
        assert!(output.contains("additionalProperties: false"));
        assert!(output.contains("Required</td>"));
        assert!(output.contains("oslo"));
        assert!(output.contains("Reference: #/components/schemas/City"));
        assert!(
            fields(&json!({"$ref":"#/components/schemas/City"}), 0)
                .contains("Referenced schema retained without expansion")
        );
        let composition = fields(&json!({"oneOf":[{"type":"string"},{"type":"integer"}]}), 0);
        assert!(composition.contains("branches remain separate"));
        assert!(composition.contains("oneOf branch 2"));
        assert!(
            fields(&json!({"required":true,"properties":{"x":{}}}), 0)
                .contains("Requiredness unknown")
        );
        assert!(fields(&json!(42), 0).contains("Unsupported schema value"));
        assert!(fields(&json!({}), 7).contains("deeper fields"));
        let mut deep = json!({"type":"string"});
        for _ in 0..12 {
            deep = json!({"type":"array","items":deep});
        }
        assert!(schema_type(&deep).contains("See exact retained schema"));
        assert!(
            fields(
                &json!({"properties":{"<field>":{"description":"{unsafe}"}}}),
                0
            )
            .contains("&lt;field&gt;")
        );
    }
    #[test]
    fn findings_require_exact_selected_bindings_and_keep_observation_scope() {
        let entry = json!({"contractId":"http","contractDigest":"digest","entrypoint":"input","method":"POST","path":"/places","findings":[{"kind":"BROADER_REQUEST_ACCEPTANCE","caseIds":["extra"]}],"cases":[{"id":"extra","request":{"body":"{\"cityId\":\"oslo\",\"extra\":true}"},"response":{"status":200,"contentType":"application/json"}}]});
        let mut page = json!({"subject":"service:places","selectedSnapshot":"snapshot","selectedEvidence":{"revisions":{"places":"rev"}},"contractComparison":{"schema":"codeclew-http-comparison-view/1.0","bindingStatus":"MATCHES_PINNED_SELECTED_SNAPSHOT","snapshot":"snapshot","revision":"rev","service":"places"},"contracts":[{"id":"http","digest":"digest","normalized":{"entrypoint":"input","method":"POST","path":"/places"}}]});
        assert!(comparison_current(&page, &entry));
        page["contractComparison"]["revision"] = json!("older");
        assert!(!comparison_current(&page, &entry));
        let output = findings(&entry);
        assert!(output.contains("does not show failure for a contract-valid request"));
        assert!(output.contains("application/json"));
        assert!(output.contains("&#123;"));
    }
    #[test]
    fn representations_show_event_and_normalized_types_without_inventing_requiredness() {
        let field = json!({"representations":[{"label":"Event","contract":"event","field":"updatedAt","mapping":{"text":"Separate contract."}},{"label":"Normalized","typeLabel":"string","requiredness":"Present","nullability":"Not nullable","mapping":{"text":"Stored as JSON."}}]});
        let catalog = json!({"contracts":[{"id":"event","leaves":[{"field":"updatedAt","schema":{"type":"string","format":"date-time"},"requiredWhenBranchMatches":true,"nullable":false}]}]});
        let output = representations(&field, &catalog);
        assert!(output.contains("date-time"));
        assert!(output.contains("Required when branch matches"));
        assert!(output.contains("Not nullable"));
        assert!(output.contains("Normalized"));
        assert!(output.contains("Stored as JSON."));
    }
}
