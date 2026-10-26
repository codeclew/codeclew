//! Source-derived payload views reuse canonical fields and exact representations.
//! They are not generated wire schemas or validator/runtime proof.
use super::{Check, FieldCoverage, Model, invalid, page_for, text};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PayloadView {
    pub id: String,
    pub title: String,
    pub subject: String,
    pub operation: String,
    pub component: String,
    pub format: String,
    pub direction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub coverage: FieldCoverage,
    pub fields: Vec<PayloadField>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PayloadField {
    pub model: String,
    pub field: String,
    /// Exact existing representation label; no prefix/name matching.
    pub representation: String,
    /// Observed slot/path only; its wire meaning belongs to the cited claim.
    pub path: String,
}
pub(super) fn validate_and_project(
    views: &mut [PayloadView],
    models: &[Model],
    files: &BTreeMap<String, Vec<u8>>,
    checked: &Check,
    bytes: &mut super::super::question_reading::EvidenceBudget,
) -> Result<Vec<Value>, ClewError> {
    if views.len() > 128 {
        return Err(invalid("payload views exceed 128 entries"));
    }
    let mut ids = BTreeSet::new();
    let mut result = Vec::new();
    for view in views {
        text(&view.title)?;
        text(&view.format)?;
        if view.format.len() > 128 {
            return Err(invalid("payload format exceeds 128 bytes"));
        }
        text(&view.coverage.scope)?;
        if !super::store::valid_id(&view.id)
            || !super::store::valid_id(&view.operation)
            || !ids.insert((
                view.subject.clone(),
                view.operation.clone(),
                view.id.clone(),
            ))
            || !matches!(
                view.component.as_str(),
                "request-body"
                    | "request-path"
                    | "request-query"
                    | "request-headers"
                    | "response-body"
                    | "error-response-body"
                    | "message-key"
                    | "message-headers"
                    | "message-value"
            )
            || !matches!(
                view.coverage.status.as_str(),
                "complete-definition" | "observed-fields" | "unknown"
            )
            || !matches!(view.direction.as_str(), "incoming" | "outgoing" | "local")
            || view.fields.len() > 128
            || (view.coverage.status == "unknown" && !view.fields.is_empty())
            || (view.coverage.status != "unknown" && view.fields.is_empty())
        {
            return Err(invalid(
                "payload view identity, component, coverage or field bounds are invalid",
            ));
        }
        if let Some(status) = &view.status
            && (!matches!(
                view.component.as_str(),
                "response-body" | "error-response-body"
            ) || !(status == "default"
                || (status.len() == 3
                    && status.bytes().all(|b| b.is_ascii_digit())
                    && matches!(status.as_bytes()[0], b'1'..=b'5'))))
        {
            return Err(invalid(
                "payload status belongs to an explicit response component",
            ));
        }
        if let Some(id) = &view.interaction {
            let inputs = checked
                .source_inputs
                .as_ref()
                .ok_or_else(|| invalid("payload interaction needs frozen source inputs"))?;
            let interaction = inputs
                .inputs
                .interactions
                .get(id)
                .ok_or_else(|| invalid("payload interaction is unavailable"))?;
            let service = if let Some(id) = view.subject.strip_prefix("service:") {
                id.to_owned()
            } else if let Some(id) = view.subject.strip_prefix("scenario:") {
                inputs
                    .inputs
                    .scenarios
                    .get(id)
                    .ok_or_else(|| invalid("payload scenario is unavailable"))?
                    .root
                    .service
                    .clone()
            } else {
                return Err(invalid("payload subject must be a service or scenario"));
            };
            let response = matches!(
                view.component.as_str(),
                "response-body" | "error-response-body"
            );
            let sender = (view.direction == "outgoing") != response;
            if view.direction == "local"
                || service
                    != if sender {
                        interaction.from.service.clone()
                    } else {
                        interaction.to.service.clone()
                    }
            {
                return Err(invalid(
                    "payload interaction direction is inconsistent with its selected local service/component",
                ));
            }
        }
        let page = page_for(files, &view.subject)?;
        if !page["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|op| op["id"] == view.operation)
        {
            return Err(invalid(
                "payload view must name an exact retained operation",
            ));
        }
        view.coverage.claim = super::super::question_reading::bind_block(
            view.coverage.claim.clone(),
            files,
            checked,
            bytes,
        )?;
        let mut paths = BTreeSet::new();
        let mut fields = Vec::new();
        let mut selected: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for reference in &view.fields {
            text(&reference.path)?;
            if !paths.insert(&reference.path) {
                return Err(invalid(
                    "payload view repeats an observed slot; distinct scopes need distinct views",
                ));
            }
            let model = models
                .iter()
                .find(|m| m.id == reference.model)
                .ok_or_else(|| invalid("payload model is unavailable"))?;
            let field = model
                .fields
                .iter()
                .find(|f| f.id == reference.field)
                .ok_or_else(|| invalid("payload canonical field is unavailable"))?;
            let representations = field
                .representations
                .iter()
                .filter(|r| r.label == reference.representation)
                .collect::<Vec<_>>();
            if representations.len() != 1 {
                return Err(invalid(
                    "payload field needs one exact existing representation label",
                ));
            }
            let representation = representations[0];
            selected.entry(&model.id).or_default().insert(&field.id);
            fields.push(json!({"model":model.id,"field":field.id,"path":reference.path,"representation":representation.label,"typeLabel":representation.type_label,"requiredness":representation.requiredness,"nullability":representation.nullability,"meaning":field.meaning,"mapping":representation.mapping}));
        }
        if view.coverage.status == "complete-definition" {
            for (id, fields) in selected {
                let model = models.iter().find(|m| m.id == id).unwrap();
                if model
                    .field_coverage
                    .as_ref()
                    .is_none_or(|c| c.status != "complete-definition")
                    || fields.len() != model.fields.len()
                {
                    return Err(invalid(
                        "complete payload type definition needs complete selected model definitions and every field; observed mappings remain partial",
                    ));
                }
            }
        }
        result.push(json!({"id":view.id,"title":view.title,"subject":view.subject,"operation":view.operation,"component":view.component,"format":view.format,"direction":view.direction,"interaction":view.interaction,"status":view.status,"coverage":view.coverage,"fields":fields,"authority":"SOURCE_DERIVED_REPRESENTATION","wireSchema":"NOT_ESTABLISHED_BY_TYPE_OR_MAPPING","runtime":"UNVERIFIED"}));
    }
    Ok(result)
}

pub(super) fn attach_external_links(
    model: Option<&Value>,
    files: &mut BTreeMap<String, Vec<u8>>,
    language: &str,
) -> Result<(), ClewError> {
    let Some(model) = model else {
        return Ok(());
    };
    let names = files
        .keys()
        .filter(|n| n.starts_with("external-services/") && n.ends_with(".json"))
        .cloned()
        .collect::<Vec<_>>();
    for name in names {
        let mut record: Value =
            serde_json::from_slice(&files[&name]).map_err(super::super::super::io_error)?;
        let mut links = Vec::new();
        for view in model["payloadViews"].as_array().into_iter().flatten() {
            let Some(interaction) = view["interaction"].as_str() else {
                continue;
            };
            if !record["interactions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|i| i["id"] == interaction)
            {
                continue;
            }
            let subject = view["subject"].as_str().unwrap_or("");
            let (directory, id) = if let Some(id) = subject.strip_prefix("service:") {
                ("services", id)
            } else if let Some(id) = subject.strip_prefix("scenario:") {
                ("scenarios", id)
            } else {
                continue;
            };
            links.push(json!({"title":view["title"],"interaction":interaction,"component":view["component"],"direction":view["direction"],"coverage":view["coverage"]["status"],"href":format!("../{directory}/{id}.html#cpv-{}-{}",view["operation"].as_str().unwrap_or(""),view["id"].as_str().unwrap_or(""))}));
        }
        if links.is_empty() {
            continue;
        }
        record["payloadLinks"] = json!(links);
        files.insert(
            name.clone(),
            serde_json::to_vec(&record).map_err(super::super::super::io_error)?,
        );
        let html_name = name.trim_end_matches(".json").to_owned() + ".html";
        if let Some(raw) = files.get_mut(&html_name) {
            let mut section = format!(
                "<section class=\"profile-payloads\"><h2>{}</h2><ul>",
                super::super::super::language::label(language, "Payload shapes")
            );
            for link in &links {
                section += &format!(
                    "<li><a href=\"{}\">{}</a> · {} · {}</li>",
                    super::escape(link["href"].as_str().unwrap_or("")),
                    super::escape(link["title"].as_str().unwrap_or("")),
                    super::super::super::language::label(
                        language,
                        link["direction"].as_str().unwrap_or("")
                    ),
                    super::super::super::language::label(
                        language,
                        link["coverage"].as_str().unwrap_or("")
                    )
                );
            }
            section += "</ul></section>";
            let html = String::from_utf8(raw.clone()).map_err(super::super::super::io_error)?;
            *raw = html.replace("</body>", &(section + "</body>")).into_bytes();
        }
    }
    Ok(())
}
