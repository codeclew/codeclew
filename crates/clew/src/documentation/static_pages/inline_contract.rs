//! Display-only projection of an explicitly bound retained declaration.
//! It does not execute JSON Schema rules or resolve references.
use super::super::{invalid, language::label, render::escape};
use crate::error::ClewError;
use serde_json::{Value, json};

mod openapi;

fn nullability(s: &Value, openapi30: bool) -> &'static str {
    if s == &Value::Bool(false) {
        return "Excluded";
    }
    if s.get("const").is_some_and(|v| !v.is_null())
        || s["enum"]
            .as_array()
            .is_some_and(|v| !v.iter().any(Value::is_null))
    {
        return "Excluded";
    }
    let types = s.get("type");
    let typed = match types {
        Some(Value::String(t)) => Some(t == "null"),
        Some(Value::Array(t)) if t.iter().all(Value::is_string) => {
            Some(t.iter().any(|t| t == "null"))
        }
        _ => None,
    };
    let nullable = openapi30 && s["nullable"] == true && types.is_some();
    if typed == Some(false) && !nullable {
        return "Excluded";
    }
    if [
        "$ref",
        "$dynamicRef",
        "allOf",
        "oneOf",
        "anyOf",
        "not",
        "if",
        "then",
        "else",
    ]
    .iter()
    .any(|k| s.get(k).is_some())
    {
        return "Not established";
    }
    if s == &Value::Bool(true)
        || typed == Some(true)
        || nullable
        || s.get("const").is_some_and(Value::is_null)
        || s["enum"]
            .as_array()
            .is_some_and(|v| v.iter().any(Value::is_null))
    {
        "Allowed"
    } else {
        "Not established"
    }
}

#[cfg(test)]
fn project(schema: &Value, pointer: &str) -> Result<Value, ClewError> {
    project_document(schema, pointer, &Value::Null)
}
pub(in crate::documentation) fn project_document(
    schema: &Value,
    pointer: &str,
    document: &Value,
) -> Result<Value, ClewError> {
    if !schema.is_object() && !schema.is_boolean() {
        return Err(invalid(
            "inline contract branch must be a schema object or boolean",
        ));
    }
    if schema["openapi"]
        .as_str()
        .is_some_and(|v| v.starts_with("3."))
    {
        return openapi::project(schema, pointer);
    }
    let openapi30 = document["openapi"]
        .as_str()
        .is_some_and(|v| v.starts_with("3.0."));
    let mut projection = Projection {
        openapi30,
        nodes: Vec::new(),
        frontier: Vec::new(),
    };
    visit(schema, "$", pointer, "Root schema", 0, &mut projection);
    Ok(
        json!({"nodes":projection.nodes,"schema":schema,"frontier":projection.frontier,
        "authority":"RETAINED_DECLARATION_NOT_VALIDATOR_OR_RUNTIME_PROOF"}),
    )
}
struct Projection {
    openapi30: bool,
    nodes: Vec<Value>,
    frontier: Vec<String>,
}
fn visit(
    s: &Value,
    field: &str,
    pointer: &str,
    presence: &str,
    depth: usize,
    projection: &mut Projection,
) {
    if depth > 16 || projection.nodes.len() >= 512 {
        projection.frontier.push(format!(
            "{field}: display bound reached; exact schema retained below"
        ));
        return;
    }
    let kind = if let Some(t) = s.get("type") {
        t.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| t.to_string())
    } else if s == &Value::Bool(true) {
        "Any value (true schema)".into()
    } else if s == &Value::Bool(false) {
        "No value allowed (false schema)".into()
    } else {
        "Not declared".into()
    };
    let constraints: serde_json::Map<String, Value> = s
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "type"
                    | "properties"
                    | "items"
                    | "description"
                    | "title"
                    | "oneOf"
                    | "anyOf"
                    | "allOf"
            )
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for key in [
        "$ref",
        "$dynamicRef",
        "not",
        "if",
        "then",
        "else",
        "dependentSchemas",
        "patternProperties",
        "unevaluatedProperties",
        "prefixItems",
        "contains",
    ] {
        if s.get(key).is_some() {
            projection.frontier.push(format!(
                "{field}: {key} retained without expansion or rule merging"
            ));
        }
    }
    projection.nodes.push(json!({"field":field,"pointer":pointer,"type":kind,"presence":presence,
        "nullability":nullability(s, projection.openapi30),"description":s["description"].as_str().unwrap_or(""),
        "constraints":constraints}));
    for composition in ["oneOf", "anyOf", "allOf"] {
        if let Some(branches) = s[composition].as_array() {
            for (index, branch) in branches.iter().enumerate() {
                if projection.nodes.len() >= 512 {
                    projection.frontier.push(format!(
                        "{field}: composition display bound reached; exact branches retained below"
                    ));
                    break;
                }
                let condition = if composition == "allOf" {
                    format!("allOf branch {} · constraints apply together", index + 1)
                } else {
                    format!(
                        "{composition} alternative {} · requirements apply within this branch",
                        index + 1
                    )
                };
                visit(
                    branch,
                    &format!("{field}.{composition}[{}]", index + 1),
                    &format!("{pointer}/{composition}/{index}"),
                    &condition,
                    depth + 1,
                    projection,
                );
            }
        }
    }
    if let Some(properties) = s["properties"].as_object() {
        // Required names first preserves an explicit useful declaration order.
        let required = s["required"].as_array();
        let mut names: Vec<&String> = Vec::new();
        for name in required.into_iter().flatten().filter_map(Value::as_str) {
            if let Some((key, _)) = properties.get_key_value(name)
                && !names.contains(&key)
            {
                names.push(key);
            }
        }
        for name in properties.keys() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        for name in names {
            let presence = match s.get("required") {
                None => "Optional in parent",
                Some(Value::Array(values)) if values.iter().all(Value::is_string) => {
                    if values.iter().any(|v| v == name) {
                        "Required in parent"
                    } else {
                        "Optional in parent"
                    }
                }
                _ => "Not established",
            };
            let path = if field == "$" {
                name.clone()
            } else {
                format!("{field}.{name}")
            };
            let escaped = name.replace('~', "~0").replace('/', "~1");
            visit(
                &properties[name],
                &path,
                &format!("{pointer}/properties/{escaped}"),
                presence,
                depth + 1,
                projection,
            );
        }
    }
    if let Some(items) = s.get("items") {
        if items.is_object() || items.is_boolean() {
            visit(
                items,
                &format!("{field}[]"),
                &format!("{pointer}/items"),
                "Each array item",
                depth + 1,
                projection,
            );
        } else {
            projection.frontier.push(format!(
                "{field}: tuple/unsupported items retained without expansion"
            ));
        }
    }
}

pub(super) fn table(declaration: &Value, language: &str) -> String {
    table_at(declaration, language, true)
}
pub(in crate::documentation) fn table_html(declaration: &Value, language: &str) -> String {
    table_at(declaration, language, false)
}
pub(in crate::documentation) fn table_html_without_presence(
    declaration: &Value,
    language: &str,
) -> String {
    table_with_presence(declaration, language, false, false)
}
fn table_at(declaration: &Value, language: &str, portable: bool) -> String {
    table_with_presence(declaration, language, portable, true)
}
fn table_with_presence(
    declaration: &Value,
    language: &str,
    portable: bool,
    show_presence: bool,
) -> String {
    if declaration["kind"] == "OPENAPI_DOCUMENT" {
        return openapi::table(declaration, language, portable);
    }
    let lit = |s: &str| escape(s).replace('{', "&#123;").replace('}', "&#125;");
    let headers = [
        "Field",
        "Type",
        "Presence",
        "Accepts null",
        "Declared constraints / description",
    ];
    let attribute = if portable { "className" } else { "class" };
    let mut out = format!(
        "<div {attribute}=\"clew-table-scroll contract-declaration\"><table {attribute}=\"clew-schema-table inline-contract-table\"><thead><tr>"
    );
    for h in headers {
        if h == "Presence" && !show_presence {
            continue;
        }
        out.push_str(&format!(
            "<th scope=\"col\">{}</th>",
            lit(label(language, h))
        ));
    }
    out.push_str("</tr></thead><tbody>");
    for n in declaration["nodes"].as_array().into_iter().flatten() {
        out.push_str("<tr>");
        for (index, key) in ["field", "type", "presence", "nullability"]
            .into_iter()
            .enumerate()
        {
            if key == "presence" && !show_presence {
                continue;
            }
            let value = n[key].as_str().unwrap_or("");
            let cell = if key == "field" {
                format!("<code>{}</code>", lit(value))
            } else {
                lit(label(language, value))
            };
            out.push_str(&format!(
                "<td data-label=\"{}\">{cell}</td>",
                lit(label(language, headers[index]))
            ));
        }
        out.push_str(&format!(
            "<td data-label=\"{}\">",
            lit(label(language, headers[4]))
        ));
        if let Some(description) = n["description"].as_str().filter(|v| !v.is_empty()) {
            out.push_str(&format!("<p>{}</p>", lit(description)));
        }
        for (key, value) in n["constraints"].as_object().into_iter().flatten() {
            out.push_str(&format!(
                "<p><code>{}</code>: {}</p>",
                lit(key),
                lit(&value.to_string())
            ));
        }
        // DDL expressions are already exact source literals. JSON quoting would
        // turn now() into a misleading string default in the human table.
        for (key, value) in n["literalConstraints"].as_object().into_iter().flatten() {
            if let Some(value) = value.as_str() {
                out.push_str(&format!(
                    "<p><code>{}</code>: <code>{}</code></p>",
                    lit(key),
                    lit(value)
                ));
            }
        }
        out.push_str("</td></tr>");
    }
    out.push_str("</tbody></table></div>\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_presence_display_keeps_schema_presence_and_other_columns_intact() {
        let projection = project(
            &json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}),
            "",
        )
        .unwrap();
        let regular = table_html(&projection, "en");
        let ddl = table_html_without_presence(&projection, "en");
        assert!(regular.contains("<th scope=\"col\">Presence</th>"));
        assert!(regular.contains("Required in parent"));
        assert!(!ddl.contains("<th scope=\"col\">Presence</th>"));
        assert_eq!(regular.matches("<tr>").count(), ddl.matches("<tr>").count());
        for header in [
            "Field",
            "Type",
            "Accepts null",
            "Declared constraints / description",
        ] {
            assert!(ddl.contains(&format!("<th scope=\"col\">{header}</th>")));
        }
    }
    #[test]
    fn nested_required_nullable_and_container_constraints_remain_separate() {
        let schema = json!({"type":"object","additionalProperties":false,"required":["periods"],"properties":{
            "periods":{"type":"array","minItems":1,"maxItems":6,"items":{"type":"object","required":["rain"],"properties":{
                "rain":{"type":["number","null"],"minimum":0},"hint":{"description":"<script>{unsafe}</script>"}}}}}});
        let d = project(&schema, "/oneOf/1").unwrap();
        let rows = d["nodes"].as_array().unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0]["constraints"]["additionalProperties"], false);
        assert_eq!(rows[1]["constraints"]["maxItems"], 6);
        assert_eq!(rows[2]["presence"], "Each array item");
        let rain = rows
            .iter()
            .find(|r| r["field"] == "periods[].rain")
            .unwrap();
        assert_eq!(rain["presence"], "Required in parent");
        assert_eq!(rain["nullability"], "Allowed");
        let hint = rows
            .iter()
            .find(|r| r["field"] == "periods[].hint")
            .unwrap();
        assert_eq!(hint["presence"], "Optional in parent");
        assert_eq!(hint["nullability"], "Not established");
        assert!(rows[2]["constraints"].get("additionalProperties").is_none());
        let html = table(&d, "en");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;&#123;unsafe&#125;"));
        assert_eq!(d["schema"], schema);
    }
    #[test]
    fn nullability_respects_intersection_and_openapi_dialect() {
        assert_eq!(
            nullability(&json!({"type":"string","const":null}), false),
            "Excluded"
        );
        assert_eq!(
            nullability(&json!({"type":["number","null"],"allOf":[false]}), false),
            "Not established"
        );
        assert_eq!(
            nullability(&json!({"type":"number","nullable":true}), false),
            "Excluded"
        );
        assert_eq!(
            nullability(&json!({"type":"number","nullable":true}), true),
            "Allowed"
        );
        assert_eq!(
            nullability(&json!({"type":"number","nullable":true,"enum":[1]}), true),
            "Excluded"
        );
    }
    #[test]
    fn event_alternatives_expose_fields_without_combining_requiredness() {
        let schema = json!({"oneOf":[
            {"type":"object","required":["cityId"],"properties":{"cityId":{"type":"string","enum":["oslo"]}}},
            {"type":"object","required":["periods"],"properties":{"periods":{"type":"array","minItems":1,"maxItems":6,"items":{"type":"number"}}}}
        ]});
        let d = project(&schema, "").unwrap();
        let rows = d["nodes"].as_array().unwrap();
        assert!(rows.iter().any(|n| n["field"] == "$.oneOf[1].cityId" && n["presence"] == "Required in parent"));
        assert!(
            rows.iter()
                .any(|n| n["field"] == "$.oneOf[2].periods" && n["constraints"]["maxItems"] == 6)
        );
        assert!(rows[0]["constraints"].get("oneOf").is_none());
        assert!(table(&d, "en").contains("oneOf alternative 2"));
        assert_eq!(d["schema"], schema);
    }
    #[test]
    fn references_remain_unexpanded_and_composition_branches_stay_separate() {
        let s = json!({"$ref":"#/$defs/Other","allOf":[{"required":["hidden"]}]});
        let d = project(&s, "").unwrap();
        assert_eq!(d["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(d["frontier"].as_array().unwrap().len(), 1);
        assert!(
            d["nodes"][1]["presence"]
                .as_str()
                .unwrap()
                .starts_with("allOf branch")
        );
        assert_eq!(d["nodes"][0]["nullability"], "Not established");
        assert_eq!(d["schema"], s);
        assert!(project(&json!("not schema"), "").is_err());
    }
}
