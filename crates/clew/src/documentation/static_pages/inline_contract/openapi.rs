//! OpenAPI documents are operation containers, not JSON Schema root objects.
use super::{project_document, table_at};
use crate::documentation::render::escape;
use crate::error::ClewError;
use serde_json::{Value, json};

fn token(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

struct Display {
    parts: Vec<Value>,
    remaining: usize,
    parts_left: usize,
    omitted: bool,
}
impl Display {
    fn part(
        &mut self,
        title: String,
        value: &Value,
        pointer: String,
        document: &Value,
    ) -> Result<(), ClewError> {
        if self.parts_left == 0 || self.remaining == 0 {
            self.omitted = true;
            return Ok(());
        }
        let mut part = json!({"title":title,"pointer":pointer,"description":value["description"],"required":value["required"],"reference":value["$ref"]});
        if let Some(schema) = value.get("schema") {
            let mut declaration = project_document(schema, &format!("{pointer}/schema"), document)?;
            if let Some(nodes) = declaration["nodes"].as_array_mut() {
                if nodes.len() > self.remaining {
                    nodes.truncate(self.remaining);
                    self.omitted = true;
                }
                self.remaining -= nodes.len();
            }
            part["declaration"] = declaration;
        }
        self.parts_left -= 1;
        self.parts.push(part);
        Ok(())
    }
    fn content(
        &mut self,
        title: &str,
        value: &Value,
        pointer: &str,
        document: &Value,
    ) -> Result<(), ClewError> {
        // Keep absent bodies and references visible; a status without content
        // never establishes an empty runtime body.
        self.part(title.into(), value, pointer.into(), document)?;
        self.omitted |= value["content"].as_object().is_some_and(|m| m.len() > 256);
        for (media, body) in value["content"].as_object().into_iter().flatten().take(256) {
            self.part(
                format!("{title} · {media}"),
                body,
                format!("{pointer}/content/{}", token(media)),
                document,
            )?;
        }
        Ok(())
    }
}

pub(super) fn project(document: &Value, pointer: &str) -> Result<Value, ClewError> {
    let mut display = Display {
        parts: vec![],
        remaining: 512,
        parts_left: 256,
        omitted: false,
    };
    let mut operations = Vec::new();
    let mut frontier = Vec::new();
    for (path, item) in document["paths"].as_object().into_iter().flatten() {
        if let Some(reference) = item["$ref"].as_str() {
            frontier.push(format!(
                "{path}: reference {reference} retained without expansion"
            ));
        }
        for method in [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ] {
            let Some(operation) = item.get(method).filter(|v| v.is_object()) else {
                continue;
            };
            if operations.len() >= 128 {
                display.omitted = true;
                break;
            }
            display.parts.clear();
            let base = format!("{pointer}/paths/{}/{method}", token(path));
            // Operation parameters override path parameters with the same name
            // and location. References stay explicit and are never resolved.
            let operation_parameters = operation["parameters"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for (parameters, at, inherited) in [
                (
                    &item["parameters"],
                    format!("{pointer}/paths/{}/parameters", token(path)),
                    true,
                ),
                (
                    &operation["parameters"],
                    format!("{base}/parameters"),
                    false,
                ),
            ] {
                display.omitted |= parameters.as_array().is_some_and(|p| p.len() > 256);
                for (index, parameter) in parameters
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .take(256)
                {
                    if inherited
                        && parameter["name"].is_string()
                        && parameter["in"].is_string()
                        && operation_parameters
                            .iter()
                            .any(|p| p["name"] == parameter["name"] && p["in"] == parameter["in"])
                    {
                        continue;
                    }
                    let name = parameter["name"].as_str().unwrap_or("Reference");
                    let location = parameter["in"].as_str().unwrap_or("not expanded");
                    display.part(
                        format!("Parameter · {name} · {location}"),
                        parameter,
                        format!("{at}/{index}"),
                        document,
                    )?;
                    if parameter["content"].is_object() {
                        display.content(
                            &format!("Parameter · {name} · {location}"),
                            parameter,
                            &format!("{at}/{index}"),
                            document,
                        )?;
                    }
                }
            }
            if let Some(body) = operation.get("requestBody") {
                display.content(
                    "Request body",
                    body,
                    &format!("{base}/requestBody"),
                    document,
                )?;
            }
            display.omitted |= operation["responses"]
                .as_object()
                .is_some_and(|m| m.len() > 256);
            for (status, response) in operation["responses"]
                .as_object()
                .into_iter()
                .flatten()
                .take(256)
            {
                let at = format!("{base}/responses/{}", token(status));
                display.content(&format!("Response · {status}"), response, &at, document)?;
                display.omitted |= response["headers"]
                    .as_object()
                    .is_some_and(|m| m.len() > 256);
                for (name, header) in response["headers"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .take(256)
                {
                    display.part(
                        format!("Response · {status} · header {name}"),
                        header,
                        format!("{at}/headers/{}", token(name)),
                        document,
                    )?;
                }
            }
            operations.push(json!({"method":method.to_uppercase(),"path":path,"operationId":operation["operationId"],"summary":operation["summary"],"description":operation["description"],"pointer":base,"parts":display.parts}));
        }
    }
    if display.omitted {
        frontier.push("Display bound reached; omitted operation details remain in the exact retained document.".into());
    }
    Ok(
        json!({"kind":"OPENAPI_DOCUMENT","operations":operations,"frontier":frontier,"schema":document,"authority":"RETAINED_DECLARATION_NOT_VALIDATOR_OR_RUNTIME_PROOF"}),
    )
}

pub(super) fn table(declaration: &Value, language: &str, portable: bool) -> String {
    let lit = |s: &str| escape(s).replace('{', "&#123;").replace('}', "&#125;");
    let mut out = String::new();
    for operation in declaration["operations"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "<h3><code>{} {}</code></h3>\n\n",
            lit(operation["method"].as_str().unwrap_or("")),
            lit(operation["path"].as_str().unwrap_or(""))
        ));
        for key in ["operationId", "summary", "description"] {
            if let Some(value) = operation[key].as_str() {
                out.push_str(&format!("<p>{}</p>\n\n", lit(value)));
            }
        }
        for part in operation["parts"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "<h4>{}</h4>\n\n",
                lit(part["title"].as_str().unwrap_or(""))
            ));
            if let Some(required) = part["required"].as_bool() {
                out.push_str(&format!("<p>Required: {required}</p>\n\n"));
            }
            if let Some(description) = part["description"].as_str() {
                out.push_str(&format!("<p>{}</p>\n\n", lit(description)));
            }
            if let Some(reference) = part["reference"].as_str() {
                out.push_str(&format!(
                    "<p>Reference retained without expansion: <code>{}</code></p>\n\n",
                    lit(reference)
                ));
            }
            if part["declaration"].is_object() {
                out.push_str(&table_at(&part["declaration"], language, portable));
                for gap in part["declaration"]["frontier"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    out.push_str(&format!("<p>{}</p>\n\n", lit(gap)));
                }
            }
        }
    }
    for gap in declaration["frontier"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        out.push_str(&format!("<p>{}</p>\n\n", lit(gap)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_projects_operations_bodies_fields_and_parameter_override_without_resolving_refs() {
        let document = json!({"openapi":"3.0.3","paths":{"/cities/{id}":{"parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string","enum":["old"]}}],"post":{"operationId":"save","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string","enum":["oslo"]}}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","required":["cityId"],"properties":{"cityId":{"type":"string","nullable":true,"enum":["oslo",null]}}}}}},"responses":{"200":{"description":"Saved","content":{"application/json":{"schema":{"type":"array","maxItems":3,"items":{"type":"object","properties":{"latitude":{"type":"number","minimum":-90}}}}}}},"503":{"description":"Unavailable"},"default":{"$ref":"#/components/responses/Error"}}}}}});
        let original = document.clone();
        let projected = project(&document, "").unwrap();
        let operation = &projected["operations"][0];
        assert_eq!(operation["method"], "POST");
        let parts = operation["parts"].as_array().unwrap();
        assert_eq!(
            parts
                .iter()
                .filter(|p| p["title"] == "Parameter · id · path")
                .count(),
            1
        );
        let request = parts
            .iter()
            .find(|p| p["title"] == "Request body · application/json")
            .unwrap();
        assert_eq!(
            request["declaration"]["nodes"][1]["presence"],
            "Required in parent"
        );
        assert_eq!(request["declaration"]["nodes"][1]["nullability"], "Allowed");
        for portable in [false, true] {
            let body = table(&projected, "en", portable);
            for expected in [
                "POST /cities/&#123;id&#125;",
                "cityId",
                "latitude",
                "minimum",
                "maxItems",
                "Response · 503",
                "Required: true",
                "#/components/responses/Error",
            ] {
                assert!(body.contains(expected), "{expected}");
            }
            assert!(!body.contains("&quot;paths&quot;"));
            assert!(!body.contains("&quot;old&quot;"));
        }
        assert_eq!(projected["schema"], original);
        assert_eq!(document, original);
    }
}
