//! Portable rendering of the same source-derived payload projection as Pages.
use super::{details, fence, io_error, md, rows, text};
use crate::error::ClewError;
use serde_json::Value;

pub(super) fn render(
    page: &Value,
    operation: &str,
    heading: &str,
    language: &str,
) -> Result<String, ClewError> {
    let t = |key| crate::documentation::language::label(language, key);
    let mut body = String::new();
    if rows(&page["payloadViews"]).any(|view| view["operation"] == operation) {
        body.push_str(&details(t("How to read these data shapes"),&format!("{}\n\n{}",md(t("Source-derived representation shape. Type nullability and observed mappings do not establish wire-property presence, full schema or runtime conformance.")),md(t("Field definition unavailable in selected evidence. No empty or complete payload schema is asserted.")))));
    }
    for view in rows(&page["payloadViews"]).filter(|v| v["operation"] == operation) {
        let anchor = format!("cpv-{operation}-{}", text(&view["id"]));
        body.push_str(&format!(
            "{heading} {} {{#{anchor}}}\n\n",
            md(text(&view["title"]))
        ));
        if view["documentationLanguage"]
            .as_str()
            .is_some_and(|value| value != language)
        {
            body.push_str(&format!(
                "{}: {}\n\n",
                md(t("Translation required")),
                md(text(&view["documentationLanguage"]))
            ));
        }
        let coverage = match text(&view["coverage"]["status"]) {
            "complete-definition" => "Complete retained type definition",
            "observed-fields" => "Observed fields only",
            _ => "Field definition unavailable",
        };
        body.push_str(&format!(
            "**{} · {}** · {} · {}\n\n**{}** · {}\n\n",
            md(t(text(&view["component"]))),
            md(t(text(&view["direction"]))),
            md(text(&view["format"])),
            md(text(&view["status"])),
            md(t(coverage)),
            md(text(&view["coverage"]["scope"]))
        ));
        if rows(&view["fields"]).next().is_some() {
            body.push_str(&format!(
                "| {} | {} |\n| --- | --- |\n",
                md(t("Field or observed path")),
                md(t("Meaning and mapping"))
            ));
            let mut constraints = format!(
                "| {} | {} | {} | {} |\n| --- | --- | --- | --- |\n",
                md(t("Field or observed path")),
                md(t("Type")),
                md(t("Representation presence")),
                md(t("Representation nullability"))
            );
            for field in rows(&view["fields"]) {
                let readable_name = field["readableName"]["text"]
                    .as_str()
                    .filter(|name| !name.trim().is_empty());
                let mut field_label = format!(
                    "[{}](../data-model/{}.mdx#field-{})",
                    md(readable_name.unwrap_or_else(|| text(&field["path"]))),
                    text(&field["model"]),
                    text(&field["field"])
                );
                if readable_name.is_some() {
                    field_label.push_str(&format!("<br/>{}", md(text(&field["path"]))));
                }
                let mut meaning = md(text(&field["meaning"]["text"])).replace('\n', "<br/>");
                let mapping = text(&field["mapping"]["text"]);
                if !mapping.is_empty() && mapping != text(&field["meaning"]["text"]) {
                    meaning.push_str(&format!(
                        "<br/>**{}:** {}",
                        md(t("Representation mapping")),
                        md(mapping).replace('\n', "<br/>")
                    ));
                }
                body.push_str(&format!("| {field_label} | {meaning} |\n"));
                let type_label = if let Some(target) = field["relatedModel"].as_str() {
                    format!(
                        "[{}](../data-model/{}.mdx#fields-and-representations)",
                        md(text(&field["typeLabel"])).replace('\n', "<br/>"),
                        target
                    )
                } else {
                    md(text(&field["typeLabel"])).replace('\n', "<br/>")
                };
                constraints.push_str(&format!(
                    "| {}<br/>{} | {} | {} | {} |\n",
                    md(text(&field["path"])),
                    md(text(&field["representation"])).replace('\n', "<br/>"),
                    type_label,
                    md(text(&field["requirednessClaim"]["text"])).replace('\n', "<br/>"),
                    md(text(&field["nullabilityClaim"]["text"])).replace('\n', "<br/>")
                ));
            }
            body.push('\n');
            body.push_str(&details(t("Types, requiredness and null"), &constraints));
        }
        let arguments = rows(&view["constructorArguments"]).collect::<Vec<_>>();
        if !arguments.is_empty() {
            body.push_str(&format!("**{} ({})**\n\n{}\n\n| {} | {} | {} |\n| --- | --- | --- |\n",
                md(t("Observed constructor arguments")),arguments.len(),
                md(t("Source call positions and expressions; these are not schema field names, message keys or requiredness.")),
                md(t("Position")),md(t("Source expression")),md(t("Meaning and mapping"))));
            for argument in arguments {
                body.push_str(&format!(
                    "| {} | {} | {} |\n",
                    argument["position"],
                    md(text(&argument["expression"])),
                    md(text(&argument["claim"]["text"]))
                ));
            }
            body.push('\n');
        }
        let mut evidence = format!("{}\n\n", md(text(&view["coverage"]["claim"]["text"])));
        for field in rows(&view["fields"]) {
            evidence.push_str(&format!(
                "- {} · {} · [{}](../data-model/{}.mdx#field-{})\n",
                md(text(&field["representation"])),
                md(text(&field["mapping"]["text"])),
                md(t("Model details and sources")),
                text(&field["model"]),
                text(&field["field"])
            ));
        }
        evidence.push_str(&fence(
            &serde_json::to_string_pretty(view).map_err(io_error)?,
            "json",
        ));
        body.push_str(&details(
            t("Payload mapping and source evidence"),
            &evidence,
        ));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_wire_schema_keeps_observed_constructor_positions_visible() {
        let page = json!({"payloadViews":[{"id":"observed","operation":"publish","title":"Source constructor","component":"message-value","direction":"outgoing","format":"Imported constructor","coverage":{"status":"unknown","scope":"Schema not retained","claim":{"text":"Source call only"}},"fields":[],"constructorArguments":[{"position":1,"expression":"input.value","claim":{"text":"Pass the selected value"}},{"position":2,"expression":"enabled ? 1 : 0","claim":{"text":"Compute a local0/1 factor"}}]}]});
        let body = render(&page, "publish", "###", "en").unwrap();
        assert!(body.contains("Observed constructor arguments (2)"));
        assert!(body.contains("input.value"));
        assert!(body.contains("enabled ? 1 : 0"));
        assert!(!body.contains("| Representation presence |"));
        assert!(body.contains("not schema field names"));
    }

    #[test]
    fn payload_cards_share_secondary_caveats_and_keep_each_specific_gap_primary() {
        let page = json!({"payloadViews":[
            {"id":"request","operation":"quote","title":"Price request","component":"request-body","direction":"incoming","format":"json","coverage":{"status":"observed-fields","scope":"Only the selected manufactureYear property is retained","claim":{"text":"Named source property"}},"fields":[{"model":"vehicle","field":"year","path":"manufactureYear","typeLabel":"Int?","requiredness":"Wire presence unknown","nullability":"Nullable implementation type","meaning":{"text":"Year of manufacture"},"representation":"Request.year","mapping":{"text":"Explicit source mapping"}}]},
            {"id":"failure","operation":"quote","title":"Failure reply","component":"error-response-body","direction":"outgoing","format":"json","status":"502","coverage":{"status":"unknown","scope":"Error response DTO definition is unavailable","claim":{"text":"Named error type only"}},"fields":[]},
            {"id":"event","operation":"quote","title":"Produced event","component":"message-value","direction":"outgoing","format":"avro","coverage":{"status":"unknown","scope":"Imported Avro definition is unavailable","claim":{"text":"Producer call only"}},"fields":[]}
        ]});
        let original = page.clone();
        let body = render(&page, "quote", "###", "en").unwrap();
        assert_eq!(
            body.matches("Source-derived representation shape.").count(),
            1
        );
        assert_eq!(
            body.matches("No empty or complete payload schema is asserted.")
                .count(),
            1
        );
        let legend_end = body.find("</ClewDetails>").unwrap();
        assert!(body.find("Type nullability").unwrap() < legend_end);
        assert!(body.find("Price request").unwrap() > legend_end);
        for gap in [
            "Only the selected manufactureYear property is retained",
            "Error response DTO definition is unavailable",
            "Imported Avro definition is unavailable",
        ] {
            assert!(body.contains(gap));
        }
        assert!(body.contains("Wire presence unknown"));
        assert!(body.contains("502"));
        assert!(body.contains("{#cpv-quote-failure}"));
        assert!(body.contains("../data-model/vehicle.mdx#field-year"));
        assert!(!body.contains("No body declared"));
        assert_eq!(page, original);
    }

    #[test]
    fn payload_overview_preserves_each_constraint_in_one_secondary_table() {
        let page = json!({"payloadViews":[{"id":"input","operation":"apply","title":"Local input","coverage":{"status":"observed-fields","scope":"Two selected mappings, not a wire schema"},"fields":[
            {"model":"measurement","field":"value","path":"request.value","readableName":{"text":"Measured value"},"meaning":{"text":"Value used by the calculation"},"mapping":{"text":"Forwards input.value"},"representation":"Input.value","typeLabel":"Imported type definition unavailable","requiredness":"Wire presence is not established","nullability":"Type nullability is not established","relatedModel":"nested"},
            {"model":"measurement","field":"offset","path":"request.offset","readableName":{"text":"Selected offset"},"meaning":{"text":"Adjustment for this calculation"},"mapping":{"text":"Uses zero when absent"},"representation":"Input.offset","typeLabel":"Decimal","requiredness":"Only when adjustment is enabled","nullability":"Non-null source type; wire nullability unknown"}
        ]}]});
        let original = page.clone();
        let body = render(&page, "apply", "###", "en").unwrap();
        let split = body
            .find("<summary>Types, requiredness and null</summary>")
            .unwrap();
        let (primary, technical) = body.split_at(split);
        assert_eq!(
            body.matches("<summary>Types, requiredness and null</summary>")
                .count(),
            1
        );
        assert!(primary.contains("| Field or observed path | Meaning and mapping |"));
        for field in rows(&page["payloadViews"][0]["fields"]) {
            for key in ["meaning", "mapping"] {
                assert!(primary.contains(text(&field[key]["text"])));
            }
            for key in ["typeLabel", "requiredness", "nullability"] {
                assert!(!primary.contains(text(&field[key])));
                assert!(technical.contains(text(&field[key])));
            }
        }
        assert!(primary.contains("../data-model/measurement.mdx#field-value"));
        assert!(technical.contains("../data-model/nested.mdx#fields-and-representations"));
        assert_eq!(page, original);
    }

    #[test]
    fn payload_field_label_links_to_its_exact_canonical_field_and_preserves_path() {
        let mut page = json!({"payloadViews":[{"id":"input","operation":"apply","title":"Local input","fields":[{"path":"request.value","readableName":{"text":"Measured value"},"model":"measurement","field":"value"}],"coverage":{"status":"observed-fields"}}]});
        let original = page.clone();
        let body = render(&page, "apply", "###", "en").unwrap();
        assert!(body.contains(
            "[Measured value](../data-model/measurement.mdx#field-value)<br/>request.value"
        ));
        assert_eq!(page, original);
        page["payloadViews"][0]["fields"][0]
            .as_object_mut()
            .unwrap()
            .remove("readableName");
        let body = render(&page, "apply", "###", "en").unwrap();
        assert!(body.contains("[request.value](../data-model/measurement.mdx#field-value)"));
        assert!(!body.contains("Measured value"));
    }

    #[test]
    fn payload_nested_type_uses_the_exact_projected_model_link() {
        let page = json!({"payloadViews":[{"id":"input","operation":"apply","title":"Local input","fields":[{"path":"request.record","typeLabel":"Record?","relatedModel":"record","model":"request","field":"record"}],"coverage":{"status":"observed-fields"}}]});
        let original = page.clone();
        let body = render(&page, "apply", "###", "en").unwrap();
        assert!(body.contains("[Record?](../data-model/record.mdx#fields-and-representations)"));
        assert!(body.contains("../data-model/request.mdx#field-record"));
        assert_eq!(page, original);
    }
}
