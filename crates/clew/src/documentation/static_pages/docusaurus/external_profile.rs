use super::{fence, io_error, md, rows, text};
use crate::documentation::external_services::{
    configuration_reference_label, configuration_source_label,
};
use crate::error::ClewError;
use serde_json::Value;
use std::collections::BTreeMap;

fn code(value: &str) -> String {
    let literal = serde_json::to_string(value)
        .expect("string serialization")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    format!("<code>{{{literal}}}</code>")
}

pub(super) fn operations_and_configuration(
    record: &Value,
    language: &str,
) -> Result<String, ClewError> {
    let mut body = String::new();
    let l = |key| crate::documentation::language::label(language, key);
    body.push_str(&format!("\n## {}\n\n", md(l("Configuration references"))));
    let config = rows(&record["configurationEvidence"]).collect::<Vec<_>>();
    if config.is_empty() {
        body.push_str(&format!("{}\n\n",md(l("Configuration evidence is not recorded; this does not establish that the destination is unconfigured."))));
    }
    for row in config {
        let binding = match text(&row["binding"]) {
            "static-reference" => "Static reference",
            "explicitly-unconfigured" => "Declared unconfigured",
            _ => "Unresolved reference",
        };
        body.push_str(&format!(
            "### {} · {}\n\n{} · {}\n\n",
            code(text(&row["key"])),
            md(l(configuration_source_label(
                row["sourceAvailability"]["status"]
                    .as_str()
                    .unwrap_or("SOURCE_UNAVAILABLE")
            ))),
            md(l(binding)),
            md(text(&row["scope"]))
        ));
        if let Some(name) = row["reference"]["name"].as_str() {
            body.push_str(&format!(
                "{} · {}\n\n",
                code(name),
                md(l(configuration_reference_label(text(
                    &row["reference"]["kind"]
                ))))
            ));
        }
        body.push_str(&format!(
            "<details><summary>{}</summary>\n\n{}\n\n",
            md(l("Source and configuration evidence")),
            md(text(&row["declaration"]["rationale"]))
        ));
        for source in rows(&row["sources"]) {
            let location = if let Some(file) = source["file"].as_str() {
                format!(
                    "{} · {}–{} · {}",
                    code(file),
                    source["startLine"],
                    source["endLine"],
                    code(text(&source["revision"]))
                )
            } else {
                code(text(&source["sourceId"]))
            };
            body.push_str(&format!(
                "- {} · {} · {}\n",
                md(text(&source["service"])),
                location,
                md(l(configuration_source_label(text(&source["state"]))))
            ));
        }
        body.push_str("\n</details>\n\n");
    }
    body.push_str(&format!("{}\n\n",md(l("Static references and profile scopes do not establish active configuration, precedence, resolved values or runtime routing."))));
    let mut groups: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for interaction in rows(&record["interactions"]) {
        groups
            .entry(text(&interaction["from"]["service"]))
            .or_default()
            .push(interaction);
    }
    if !groups.is_empty() {
        body.push_str(&format!(
            "\n## {}\n\n",
            md(l("Operations and configuration"))
        ));
    }
    for (service, operations) in groups {
        body.push_str(&format!(
            "### [{}](../services/{service}.mdx#section-egress)\n\n",
            md(service)
        ));
        let mut binding_groups: BTreeMap<Option<&str>, Vec<&Value>> = BTreeMap::new();
        for operation in operations {
            binding_groups
                .entry(operation["transport"]["destinationConfigKey"].as_str())
                .or_default()
                .push(operation);
        }
        for operations in binding_groups.into_values() {
            let common_key = operations
                .first()
                .and_then(|i| i["transport"]["destinationConfigKey"].as_str())
                .filter(|key| {
                    operations
                        .iter()
                        .all(|i| i["transport"]["destinationConfigKey"].as_str() == Some(key))
                });
            if let Some(key) = common_key {
                body.push_str(&format!(
                    "{}: {}\n\n",
                    md(l("Shared configuration key")),
                    code(key)
                ));
            }
            for interaction in operations {
                let transport = &interaction["transport"];
                let destination = match text(&transport["kind"]) {
                    "http" => format!(
                        "{} {}",
                        transport["method"].as_str().unwrap_or("HTTP"),
                        text(&transport["path"])
                    ),
                    "kafka" => format!("{} {}", l("Topic"), text(&transport["topic"])),
                    _ => l("Logical access").into(),
                };
                body.push_str(&format!(
                    "<details><summary>{} · {}</summary>\n\n",
                    md(text(&interaction["title"])),
                    md(&destination)
                ));
                if common_key.is_none()
                    && let Some(key) = transport["destinationConfigKey"].as_str()
                {
                    body.push_str(&format!(
                        "{}: {}\n\n",
                        md(l("Configuration key")),
                        code(key)
                    ));
                }
                if text(&transport["kind"]) == "kafka" {
                    body.push_str(&format!(
                    "{}: {}\n\n{}\n\n",
                    md(l("Message key")),
                    code(
                        transport["messageKey"]
                            .as_str()
                            .unwrap_or(l("Not established"))
                    ),
                    md(l(
                        "A topic name does not establish its payload schema or consumer contract."
                    ))
                ));
                }
                if let Some(purpose) = transport["purpose"].as_str() {
                    body.push_str(&format!("{}\n\n", md(purpose)));
                }
                for address in rows(&interaction["addresses"]) {
                    body.push_str(&format!(
                        "{} · {}\n\n",
                        code(text(&address["url"])),
                        md(text(&address["source"]))
                    ));
                }
                for link in rows(&record["payloadLinks"])
                    .filter(|link| link["interaction"] == interaction["id"])
                {
                    body.push_str(&format!(
                        "- [{}]({}) · {}\n",
                        md(text(&link["title"])),
                        text(&link["href"]).replace(".html#", ".mdx#"),
                        md(l(text(&link["coverage"])))
                    ));
                }
                body.push_str(&format!(
                    "<details><summary>{}</summary>\n\n{}\n</details>\n\n</details>\n\n",
                    md(l("Declaration evidence")),
                    fence(
                        &serde_json::to_string_pretty(interaction).map_err(io_error)?,
                        "json"
                    )
                ));
            }
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn portable_external_profile_groups_backlinks_and_bindings_without_corrupting_static_names() {
        let mut operations = Vec::new();
        for (id, key) in [
            ("quote", "pricing.endpoint"),
            ("renew", "pricing.endpoint"),
            ("alternate", "pricing.alternate"),
        ] {
            operations.push(json!({"id":id,"title":id,"from":{"service":"owner"},"transport":{"kind":"http","method":"POST","path":"/price","destinationConfigKey":key}}));
        }
        operations.push(json!({"id":"store","title":"Save observation","from":{"service":"owner"},"transport":{"kind":"logical"}}));
        operations.push(json!({"id":"publish","title":"Publish observation","from":{"service":"owner"},"transport":{"kind":"kafka","topic":"observation.created"}}));
        let record = json!({"interactions":operations,"configurationEvidence":[{"key":"pricing.endpoint","binding":"static-reference","scope":"base profile; activation unknown","reference":{"kind":"environment-placeholder","name":"PRICING_ENDPOINT"},"sources":[{"service":"owner","sourceId":"cfg","state":"SOURCE_UNAVAILABLE"}],"declaration":{"rationale":"Static name only"}}]});
        let original = record.clone();
        let body = operations_and_configuration(&record, "en").unwrap();
        assert_eq!(
            body.matches("../services/owner.mdx#section-egress").count(),
            1
        );
        assert_eq!(
            body.matches("Shared configuration key: <code>{\"pricing.endpoint\"}</code>")
                .count(),
            1
        );
        for title in [
            "quote",
            "renew",
            "alternate",
            "Save observation",
            "Publish observation",
        ] {
            assert!(body.contains(title));
        }
        assert!(body.contains("<code>{\"PRICING_ENDPOINT\"}</code>"));
        assert!(!body.contains("PRICING\\_ENDPOINT"));
        assert!(!body.contains("null–null"));
        assert!(body.contains("Logical access"));
        assert!(body.contains("Topic observation.created"));
        assert!(!body.contains("Concrete URL"));
        assert!(body.contains("does not establish its payload schema"));
        assert_eq!(record, original);
        let literal = code("${REFERENCE}<tag>`_");
        assert!(literal.contains("\\u003ctag\\u003e"));
        assert!(literal.contains("${REFERENCE}"));
        let expression = literal
            .strip_prefix("<code>{")
            .unwrap()
            .strip_suffix("}</code>")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<String>(expression).unwrap(),
            "${REFERENCE}<tag>`_"
        );
    }

    #[test]
    fn mixed_configuration_evidence_is_primary_and_rationale_is_secondary() {
        let record = json!({"configurationEvidence":[{"key":"pricing.endpoint","binding":"static-reference","scope":"Base profile; activation unknown","sourceAvailability":{"status":"SOURCE_PARTIAL"},"reference":{"kind":"environment-placeholder","name":"PRICING_ENDPOINT"},"sources":[{"service":"owner","sourceId":"missing","state":"SOURCE_UNAVAILABLE"}],"declaration":{"rationale":"Selected resource is missing from this snapshot"}}]});
        let body = operations_and_configuration(&record, "en").unwrap();
        let primary = body.split("<details>").next().unwrap();
        assert!(primary.contains("Mixed or incomplete source evidence"));
        assert!(primary.contains("Environment variable name"));
        assert!(!primary.contains("Selected resource is missing"));
        assert!(!body.contains("null–null"));
        assert_eq!(
            body.matches("Static references and profile scopes").count(),
            1
        );
        assert!(
            !operations_and_configuration(&record, "ru")
                .unwrap()
                .split("<details>")
                .next()
                .unwrap()
                .contains("Mixed or incomplete source evidence")
        );
    }
}
