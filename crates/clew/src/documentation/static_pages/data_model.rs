//! Optional authored domain mappings over admitted publication/source evidence.
use super::super::{check::Check, invalid, io_error, reader, render::escape, store};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    schema: String,
    publication: String,
    snapshot: String,
    /// Language of authored prose; absence remains unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    documentation_language: Option<String>,
    models: Vec<Model>,
    #[serde(default)]
    contracts: Vec<Contract>,
    #[serde(default)]
    process_views: Vec<ProcessView>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Model {
    id: String,
    /// Overrides the plan language for this authored model only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    documentation_language: Option<String>,
    title: String,
    kind: String,
    purpose: Value,
    identity: Value,
    fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    records: Vec<ReferenceRecord>,
    #[serde(default)]
    process_links: Vec<ProcessLink>,
    #[serde(default)]
    related_models: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferenceRecord {
    values: BTreeMap<String, Value>,
    claim: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Field {
    id: String,
    path: String,
    meaning: Value,
    representations: Vec<Representation>,
    #[serde(default)]
    usage: Vec<Usage>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Representation {
    label: String,
    type_label: String,
    requiredness: String,
    nullability: String,
    mapping: Value,
    #[serde(default)]
    contract: Option<String>,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    related_model: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Usage {
    stage: String,
    producer: String,
    consumer: String,
    contract: String,
    representation: String,
    condition: String,
    kinds: Vec<String>,
    assessment: String,
    covered_scope: String,
    unresolved: String,
    claim: Value,
    #[serde(default)]
    processes: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Contract {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    documentation_language: Option<String>,
    title: String,
    service: String,
    source_id: String,
    text_digest: String,
    branch_pointer: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessView {
    subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    documentation_language: Option<String>,
    enabled: bool,
    anchor: String,
    title: String,
    fields: Vec<FieldRef>,
    stages: Vec<String>,
    #[serde(default)]
    related_models: Vec<String>,
    #[serde(default)]
    transfers: Vec<Transfer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reading_guide: Option<ReadingGuide>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadingGuide {
    purpose: String,
    points: Vec<ReadingPoint>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadingPoint {
    fields: Vec<FieldRef>,
    stages: Vec<String>,
    claim: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Transfer {
    from: String,
    to: String,
    kind: String,
    authority: String,
    condition: String,
    claim: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FieldRef {
    model: String,
    field: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProcessLink {
    subject: String,
    anchor: String,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<ProcessReference>,
    #[serde(
        rename = "replacesServiceEntities",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    replaces_service_entities: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessReference {
    #[serde(default)]
    stages: Vec<String>,
}
const KINDS: &[&str] = &[
    "validate",
    "identify-or-route",
    "calculate-or-decide",
    "display",
    "persist",
    "forward",
    "whole-object-serialization-or-comparison",
];
const STATES: &[&str] = &[
    "observed-static-use",
    "no-use-found-in-bounded-complete-scope",
    "not-investigated",
    "unresolved-dynamic-use",
    "stale-evidence",
];
fn text(value: &str) -> Result<(), ClewError> {
    if value.trim().is_empty() || value.len() > 2048 {
        return Err(invalid("data-model text is empty or too long"));
    }
    Ok(())
}
fn page_for(files: &BTreeMap<String, Vec<u8>>, subject: &str) -> Result<Value, ClewError> {
    let (kind, id) = subject
        .split_once(':')
        .ok_or_else(|| invalid("data-model subject is invalid"))?;
    if !store::valid_id(id) {
        return Err(invalid("data-model subject ID is invalid"));
    }
    let directory = match kind {
        "service" => "services",
        "scenario" => "scenarios",
        _ => return Err(invalid("data-model link needs a service or process")),
    };
    serde_json::from_slice(
        files
            .get(&format!("{directory}/{id}.json"))
            .ok_or_else(|| invalid("data-model linked page is unavailable"))?,
    )
    .map_err(io_error)
}
fn accepts_null(node: &Value) -> Option<bool> {
    if let Some(value) = node.as_bool() {
        return Some(value);
    }
    let mut permits = true;
    if let Some(value) = node.get("type") {
        permits &= value == "null"
            || value
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == "null"));
    }
    if let Some(value) = node.get("const") {
        permits &= value.is_null();
    }
    if let Some(values) = node["enum"].as_array() {
        permits &= values.iter().any(Value::is_null);
    }
    if !permits {
        return Some(false);
    }
    if [
        "$ref",
        "$dynamicRef",
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "if",
    ]
    .iter()
    .any(|key| node.get(key).is_some())
    {
        None
    } else {
        Some(true)
    }
}
fn leaves(node: &Value, path: &str, pointer: &str, required: bool, out: &mut Vec<Value>) {
    if let Some(properties) = node["properties"].as_object() {
        for (name, value) in properties {
            let needed = required
                && node["required"]
                    .as_array()
                    .is_some_and(|r| r.iter().any(|v| v == name));
            leaves(
                value,
                &if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                },
                &format!(
                    "{pointer}/properties/{}",
                    name.replace('~', "~0").replace('/', "~1")
                ),
                needed,
                out,
            );
        }
    } else if node.get("items").is_some() {
        leaves(
            &node["items"],
            &format!("{path}[]"),
            &format!("{pointer}/items"),
            required,
            out,
        );
    } else {
        let nullable = accepts_null(node);
        out.push(json!({"field":path,"pointer":pointer,"requiredWhenBranchMatches":required,"nullable":nullable.map_or(json!("UNKNOWN"), |v|json!(v)),"schema":node}));
    }
}
pub(super) fn load(
    path: Option<&Path>,
    publication: &str,
    snapshot: &str,
    files: &BTreeMap<String, Vec<u8>>,
    checked: &Check,
    guides: &BTreeMap<String, Value>,
) -> Result<Option<Value>, ClewError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let mut plan: Plan = store::read(path, 1024 * 1024)?;
    super::super::language::validate(plan.documentation_language.as_deref())?;
    for language in plan
        .models
        .iter()
        .map(|m| m.documentation_language.as_deref())
        .chain(
            plan.contracts
                .iter()
                .map(|c| c.documentation_language.as_deref()),
        )
        .chain(
            plan.process_views
                .iter()
                .map(|v| v.documentation_language.as_deref()),
        )
    {
        super::super::language::validate(language)?;
    }
    if plan.schema != "codeclew-data-model-plan/1.0"
        || plan.publication != publication
        || plan.snapshot != snapshot
        || plan.models.is_empty()
        || plan.models.len() > 32
        || plan.contracts.len() > 32
        || plan.process_views.len() > 32
    {
        return Err(invalid("data-model schema, binding or bounds are invalid"));
    }
    let mut ids = BTreeSet::new();
    let mut bytes = 0;
    let mut contracts = Vec::new();
    for c in &plan.contracts {
        if !store::valid_id(&c.id) || !ids.insert(c.id.clone()) {
            return Err(invalid("data-model contract ID repeats or is invalid"));
        }
        let source = checked
            .services
            .get(&c.service)
            .and_then(|s| s.sources.get(&c.source_id))
            .ok_or_else(|| invalid("data-model contract source is unavailable"))?;
        if source.text_digest != c.text_digest {
            return Err(invalid("data-model contract source changed"));
        }
        let schema: Value = serde_json::from_str(&source.text).map_err(io_error)?;
        let branch = schema
            .pointer(&c.branch_pointer)
            .ok_or_else(|| invalid("data-model contract branch is unavailable"))?;
        let mut rows = Vec::new();
        leaves(branch, "", &c.branch_pointer, true, &mut rows);
        contracts.push(json!({"id":c.id,"title":c.title,"documentationLanguage":c.documentation_language.as_ref().or(plan.documentation_language.as_ref()),"branchPointer":c.branch_pointer,"leaves":rows,"source":source,"requiredCondition":"Selected contract branch matches; required parents and array element are present. Array leaves expanded once, independent of cardinality."}));
    }
    ids.clear();
    for model in &mut plan.models {
        if !store::valid_id(&model.id)
            || !ids.insert(model.id.clone())
            || !matches!(
                model.kind.as_str(),
                "ENTITY" | "VALUE_OBJECT" | "READ_MODEL"
            )
            || model.fields.is_empty()
            || model.fields.len() > 128
            || model.process_links.len() > 32
            || model.related_models.len() > 32
        {
            return Err(invalid("data-model identity, kind or fields are invalid"));
        }
        text(&model.title)?;
        model.purpose =
            super::question_reading::bind_block(model.purpose.clone(), files, checked, &mut bytes)?;
        model.identity = super::question_reading::bind_block(
            model.identity.clone(),
            files,
            checked,
            &mut bytes,
        )?;
        let mut field_ids = BTreeSet::new();
        for field in &mut model.fields {
            if !store::valid_id(&field.id)
                || !field_ids.insert(field.id.clone())
                || field.representations.is_empty()
                || field.representations.len() > 16
                || field.usage.len() > 32
            {
                return Err(invalid("data-model field identity or bounds are invalid"));
            }
            text(&field.path)?;
            field.meaning = super::question_reading::bind_block(
                field.meaning.clone(),
                files,
                checked,
                &mut bytes,
            )?;
            for r in &mut field.representations {
                for v in [&r.label, &r.type_label, &r.requiredness, &r.nullability] {
                    text(v)?;
                }
                if let Some(c) = &r.contract {
                    let contract = contracts
                        .iter()
                        .find(|v| v["id"] == *c)
                        .ok_or_else(|| invalid("data-model representation contract is absent"))?;
                    let field = r
                        .field
                        .as_ref()
                        .ok_or_else(|| invalid("contract representation needs an exact field"))?;
                    if !contract["leaves"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v["field"] == *field)
                    {
                        return Err(invalid("data-model contract field is absent"));
                    }
                }
                r.mapping = super::question_reading::bind_block(
                    r.mapping.clone(),
                    files,
                    checked,
                    &mut bytes,
                )?;
            }
            for u in &mut field.usage {
                for process in &u.processes {
                    let _ = page_for(files, process)?;
                }
                if u.processes.len() > 32
                    || !STATES.contains(&u.assessment.as_str())
                    || u.kinds.iter().any(|k| !KINDS.contains(&k.as_str()))
                    || (u.assessment == "observed-static-use" && u.kinds.is_empty())
                {
                    return Err(invalid(
                        "data-model usage category or assessment is invalid",
                    ));
                }
                for v in [
                    &u.stage,
                    &u.producer,
                    &u.consumer,
                    &u.contract,
                    &u.representation,
                    &u.condition,
                    &u.covered_scope,
                    &u.unresolved,
                ] {
                    text(v)?;
                }
                u.claim = super::question_reading::bind_block(
                    u.claim.clone(),
                    files,
                    checked,
                    &mut bytes,
                )?;
                if u.assessment == "observed-static-use"
                    && u.claim["evidence"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|e| e["state"]["freshness"] == "STALE")
                {
                    return Err(invalid(
                        "stale retained usage evidence must remain stale-evidence",
                    ));
                }
            }
        }
        for link in &model.process_links {
            let page = page_for(files, &link.subject)?;
            valid_anchor(&page, guides.get(&link.subject), &link.anchor)?;
            if !store::valid_id(&link.anchor) {
                return Err(invalid("data-model process anchor is invalid"));
            }
            text(&link.label)?;
            if let Some(reference) = &link.reference
                && (!link.subject.starts_with("scenario:")
                    || !link.anchor.starts_with("question-")
                    || reference.stages.len() > 32
                    || reference.stages.iter().any(|stage| {
                        !plan
                            .process_views
                            .iter()
                            .any(|view| view.subject == link.subject && view.stages.contains(stage))
                    }))
            {
                return Err(invalid("model reference stage is outside its process view"));
            }
            if let Some(service) = &link.replaces_service_entities {
                let _ = page_for(files, &format!("service:{service}"))?;
                let matched = link.subject.starts_with("scenario:")
                    && guides.get(&link.subject).is_some_and(|guide| {
                        guide["sections"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|section| {
                                format!("question-{}", strv(&section["id"])) == link.anchor
                                    && section["blocks"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .flat_map(|block| {
                                            block["links"].as_array().into_iter().flatten()
                                        })
                                        .any(|old| {
                                            old["subject"] == format!("service:{service}")
                                                && old["operation"] == "section-entities"
                                        })
                            })
                    });
                if !matched {
                    return Err(invalid(
                        "canonical entity navigation must replace an explicit service entities link in the same process question",
                    ));
                }
            }
        }
        if model.records.len() > 64 {
            return Err(invalid("model reference records exceed bounds"));
        }
        for record in &mut model.records {
            if record.values.keys().cloned().collect::<BTreeSet<_>>() != field_ids
                || record.values.values().any(|value| match value {
                    Value::Array(_) | Value::Object(_) => true,
                    Value::String(value) => value.len() > 2048,
                    _ => false,
                })
            {
                return Err(invalid(
                    "model reference record must provide one scalar per field",
                ));
            }
            record.claim = super::question_reading::bind_block(
                record.claim.clone(),
                files,
                checked,
                &mut bytes,
            )?;
        }
    }
    for m in &plan.models {
        for related in m.related_models.iter().chain(m.fields.iter().flat_map(|f| {
            f.representations
                .iter()
                .filter_map(|r| r.related_model.as_ref())
        })) {
            if !ids.contains(related) {
                return Err(invalid("data-model related concept is absent"));
            }
        }
    }
    let mut processes = BTreeSet::new();
    let module = super::super::dataflow::module()?;
    for view in &mut plan.process_views {
        if !view.subject.starts_with("scenario:")
            || !processes.insert(&view.subject)
            || view.fields.is_empty()
            || view.fields.len() > 64
            || !store::valid_id(&view.anchor)
        {
            return Err(invalid("data-model process view request is invalid"));
        }
        let page = page_for(files, &view.subject)?;
        valid_anchor(&page, guides.get(&view.subject), &view.anchor)?;
        if view.enabled
            && guides
                .get(&view.subject)
                .is_none_or(|g| g["presentation"] != "SEQUENTIAL")
        {
            return Err(invalid(
                "process Data Flow requires an explicit sequential reading plan",
            ));
        }
        if view.enabled && (view.stages.is_empty() || view.stages.len() > 32) {
            return Err(invalid("process Data Flow needs bounded explicit stages"));
        }
        text(&view.title)?;
        if view.related_models.len() > 32 || view.related_models.iter().any(|id| !ids.contains(id))
        {
            return Err(invalid(
                "process Data Flow related model is absent or exceeds bounds",
            ));
        }
        for stage in &view.stages {
            text(stage)?;
            if view.enabled
                && !plan.models.iter().any(|m| {
                    m.fields.iter().any(|f| {
                        view.fields
                            .iter()
                            .any(|r| r.model == m.id && r.field == f.id)
                            && f.usage
                                .iter()
                                .any(|u| u.stage == *stage && u.processes.contains(&view.subject))
                    })
                })
            {
                return Err(invalid(
                    "requested Data Flow stage has no source-bound use in this process",
                ));
            }
        }
        if view.transfers.len() > 64 {
            return Err(invalid("process Data Flow transfers exceed bounds"));
        }
        for transfer in &mut view.transfers {
            if !view.stages.contains(&transfer.from)
                || !view.stages.contains(&transfer.to)
                || transfer.from == transfer.to
                || !module["edgeKinds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|k| k == &transfer.kind)
                || !module["authority"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a == &transfer.authority)
            {
                return Err(invalid(
                    "process Data Flow transfer has an unsupported stage, kind or authority",
                ));
            }
            text(&transfer.condition)?;
            transfer.claim = super::question_reading::bind_block(
                transfer.claim.clone(),
                files,
                checked,
                &mut bytes,
            )?;
        }
        if let Some(guide) = &mut view.reading_guide {
            text(&guide.purpose)?;
            if guide.points.is_empty() || guide.points.len() > 4 {
                return Err(invalid(
                    "process reading guide needs one to four evidence-bound points",
                ));
            }
            for point in &mut guide.points {
                if point.fields.is_empty()
                    || point.fields.len() > 8
                    || point.stages.is_empty()
                    || point.stages.len() > 4
                    || point.fields.iter().any(|f| {
                        !view
                            .fields
                            .iter()
                            .any(|r| r.model == f.model && r.field == f.field)
                    })
                    || point
                        .stages
                        .iter()
                        .any(|stage| !view.stages.contains(stage))
                {
                    return Err(invalid(
                        "process reading guide references fields or stages outside its selected scope",
                    ));
                }
                point.claim = super::question_reading::bind_block(
                    point.claim.clone(),
                    files,
                    checked,
                    &mut bytes,
                )?;
            }
        }
        for f in &view.fields {
            if !plan
                .models
                .iter()
                .any(|m| m.id == f.model && m.fields.iter().any(|x| x.id == f.field))
            {
                return Err(invalid("process Data Flow field is unavailable"));
            }
        }
    }
    let mut value = serde_json::to_value(&plan).map_err(io_error)?;
    value["documentationLanguage"] = json!(plan.documentation_language);
    for kind in ["models", "processViews"] {
        if let Some(rows) = value[kind].as_array_mut() {
            for row in rows {
                if row["documentationLanguage"].is_null() {
                    row["documentationLanguage"] = json!(plan.documentation_language);
                }
            }
        }
    }
    value["schema"] = json!("codeclew-data-model-reading/1.0");
    value["contracts"] = json!(contracts);
    value["authority"] = json!("AGENT_PROPOSAL");
    value["review"] = json!("UNASSESSED");
    value["mappingBasis"] = json!({"module":super::super::dataflow::MODULE,"scope":"Static authored field interpretation; no accepted ownership or runtime proof"});
    Ok(Some(value))
}
fn strv(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn evidence(claim: &Value) -> String {
    let mut out = format!(
        "<p>{}</p><details><summary>Evidence and scope</summary>",
        escape(strv(&claim["text"]))
    );
    for e in claim["evidence"].as_array().into_iter().flatten() {
        if !e["state"].is_null() {
            out.push_str(&format!(
                "<p>Retained state: {}</p>",
                escape(&e["state"].to_string())
            ));
        }
        let sources: Vec<(&Value, u64, u64)> = if e["kind"] == "SELECTED_SOURCE" {
            vec![(
                &e["source"],
                e["startLine"].as_u64().unwrap_or(1),
                e["endLine"].as_u64().unwrap_or(1),
            )]
        } else {
            e["sources"]
                .as_object()
                .into_iter()
                .flat_map(|s| s.values())
                .map(|s| {
                    (
                        s,
                        s["startLine"].as_u64().unwrap_or(1),
                        s["endLine"].as_u64().unwrap_or(1),
                    )
                })
                .collect()
        };
        for (s, start, end) in sources {
            let source = strv(&s["text"])
                .lines()
                .skip(start.saturating_sub(s["startLine"].as_u64().unwrap_or(1)) as usize)
                .take((end - start + 1) as usize)
                .collect::<Vec<_>>()
                .join("\n");
            let url = strv(&s["url"]).split('#').next().unwrap_or("");
            let url = if url.starts_with("https://") || url.starts_with("http://") {
                url
            } else {
                ""
            };
            out.push_str(&format!("<p><a href=\"{}#L{}-L{}\">{}:{}–{}</a> · revision <code>{}</code> · {}</p><pre tabindex=\"0\">{}</pre>",escape(url),start,end,escape(strv(&s["file"])),start,end,escape(strv(&s["revision"])),escape(strv(&s["authority"])),escape(&source)));
        }
    }
    out.push_str("</details>");
    out
}
fn valid_anchor(page: &Value, guide: Option<&Value>, anchor: &str) -> Result<(), ClewError> {
    let operation = page["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|op| op["id"] == anchor);
    let question = guide
        .into_iter()
        .flat_map(|g| g["sections"].as_array().into_iter().flatten())
        .any(|section| {
            format!("question-{}", strv(&section["id"])) == anchor
                || section["blocks"].as_array().into_iter().flatten().any(|b| {
                    b["id"]
                        .as_str()
                        .is_some_and(|id| format!("question-{id}") == anchor)
                })
        });
    if !operation && !question {
        return Err(invalid(
            "data-model process anchor is not present in the reading plan or retained page",
        ));
    }
    Ok(())
}
pub(super) fn attach_process(
    model: Option<&Value>,
    subject: &str,
    payload: &mut Value,
) -> Result<(), ClewError> {
    let Some(model) = model else {
        return Ok(());
    };
    // Canonical navigation is independent of the optional field-flow reader.
    // The author must bind the exact legacy service link and question explicitly.
    let mut navigation = serde_json::Map::new();
    let mut references = Vec::new();
    for m in model["models"].as_array().into_iter().flatten() {
        for link in m["processLinks"].as_array().into_iter().flatten() {
            if link["subject"] == subject && link["reference"].is_object() {
                references.push(json!({"id":m["id"],"title":m["title"],"documentationLanguage":m["documentationLanguage"],"anchor":link["anchor"],
                    "stages":link["reference"]["stages"],"href":format!("../data-model/{}.html",strv(&m["id"]))}));
            }
            let Some(service) = link["replacesServiceEntities"].as_str() else {
                continue;
            };
            if link["subject"] != subject {
                continue;
            }
            let section = navigation
                .entry(strv(&link["anchor"]).to_owned())
                .or_insert_with(|| json!({}));
            let links = section
                .as_object_mut()
                .unwrap()
                .entry(service.to_owned())
                .or_insert_with(|| json!([]));
            links
                .as_array_mut()
                .unwrap()
                .push(json!({"id":m["id"],"title":m["title"],"documentationLanguage":m["documentationLanguage"],
                "href":format!("../data-model/{}.html",strv(&m["id"]))}));
        }
    }
    if !navigation.is_empty() {
        payload["canonicalEntityLinks"] = Value::Object(navigation);
    }
    if !references.is_empty() {
        payload["modelReferences"] = json!(references);
    }
    let Some(view) = model["processViews"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["subject"] == subject && v["enabled"] == true)
    else {
        return Ok(());
    };
    let requested = view["fields"].as_array().unwrap();
    let stages = view["stages"].as_array().unwrap();
    let mut models = model["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| {
            requested.iter().any(|r| r["model"] == m["id"])
                || view["relatedModels"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&m["id"]))
        })
        .filter_map(|m| {
            let mut scoped = m.clone();
            let fields = m["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    let mut f = f.clone();
                    let selected = requested
                        .iter()
                        .any(|r| r["model"] == m["id"] && r["field"] == f["id"]);
                    f["selectedProcessField"] = json!(selected);
                    f["usage"] = json!(
                        f["usage"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|u| selected
                                && u["processes"]
                                    .as_array()
                                    .is_some_and(|ps| ps.iter().any(|p| p == subject))
                                && stages.contains(&u["stage"]))
                            .collect::<Vec<_>>()
                    );
                    f
                })
                .collect::<Vec<_>>();
            if fields.is_empty() {
                None
            } else {
                scoped["fields"] = json!(fields);
                Some(scoped)
            }
        })
        .collect::<Vec<_>>();
    let used_contracts = models
        .iter()
        .flat_map(|m| m["fields"].as_array().into_iter().flatten())
        .flat_map(|f| f["usage"].as_array().into_iter().flatten())
        .filter_map(|u| u["contract"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    for m in &mut models {
        for field in m["fields"].as_array_mut().unwrap() {
            field["representations"]
                .as_array_mut()
                .unwrap()
                .retain(|r| {
                    r["contract"]
                        .as_str()
                        .is_none_or(|id| used_contracts.contains(id))
                });
        }
    }
    let contracts = model["contracts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            c["id"]
                .as_str()
                .is_some_and(|id| used_contracts.contains(id))
        })
        .collect::<Vec<_>>();
    let related_models = model["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| {
            view["relatedModels"]
                .as_array()
                .is_some_and(|ids| ids.contains(&m["id"]))
                || models.iter().any(|used| used["id"] == m["id"])
        })
        .map(|m| json!({"id":m["id"],"title":m["title"],"documentationLanguage":m["documentationLanguage"]}))
        .collect::<Vec<_>>();
    payload["processDataFlow"] = json!({"relatedModels":related_models,"request":view,"models":models,"contracts":contracts,"authority":model["authority"],"review":model["review"],"scope":"Fields of directly referenced models and contract leaves. Only explicitly selected process fields and consumer stages are investigated here; omitted fields remain outside investigated scope. Static interpretation, not observed runtime transfer."});
    if !references.is_empty() {
        payload["processDataFlow"]["modelReferences"] = json!(references);
    }
    Ok(())
}
pub(super) fn model_contracts<'a>(model: &Value, contracts: &'a Value) -> Vec<&'a Value> {
    let used = model["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|f| {
            f["representations"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(f["usage"].as_array().into_iter().flatten())
        })
        .filter_map(|r| r["contract"].as_str())
        .collect::<BTreeSet<_>>();
    contracts
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["id"].as_str().is_some_and(|id| used.contains(id)))
        .collect()
}
pub(super) fn reference_only(model: &Value) -> bool {
    model["records"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty())
        && !model["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|field| {
                field["usage"]
                    .as_array()
                    .is_some_and(|uses| !uses.is_empty())
            })
}
pub(super) fn write(model: &Value, files: &mut BTreeMap<String, Vec<u8>>) -> Result<(), ClewError> {
    let mut index = String::from(
        "<div class=\"dm-page\"><h1>Data model</h1><p>Information the system works with, its representations, and the processes that use it.</p><p class=\"dm-status\">Agent proposals · Not reviewed. Original descriptions and review states are retained separately. These are static interpretations, not ownership or runtime guarantees.</p><div class=\"dm-cards\">",
    );
    for m in model["models"].as_array().unwrap() {
        let id = strv(&m["id"]);
        let title = strv(&m["title"]);
        index.push_str(&format!(
            "<article><p>{}</p><h2><a href=\"data-model/{}.html\">{}</a></h2><p>{}</p></article>",
            escape(kind_label(strv(&m["kind"]))),
            escape(id),
            escape(title),
            escape(strv(&m["purpose"]["text"]))
        ));
        let mut body = format!(
            "<div class=\"dm-page\"><p><a href=\"../data-model.html\">Data model</a> / {}</p><p id=\"dm-return\"></p><h1>{}</h1><p class=\"dm-status\">{} · Agent proposal · Not reviewed</p>{}<h2>Identity and relationships</h2>{}",
            escape(title),
            escape(title),
            escape(kind_label(strv(&m["kind"]))),
            evidence(&m["purpose"]),
            model_identity(m, model)
        );
        if m["records"].as_array().is_some_and(|rows| !rows.is_empty()) {
            body.push_str("<h2>Reference records</h2><table class=\"dm-records\"><caption>Source-backed literal records · authored interpretation</caption><thead><tr>");
            for f in m["fields"].as_array().unwrap() {
                body.push_str(&format!(
                    "<th scope=\"col\">{}</th>",
                    escape(strv(&f["path"]))
                ));
            }
            body.push_str("<th scope=\"col\">Evidence</th></tr></thead><tbody>");
            for record in m["records"].as_array().unwrap() {
                body.push_str("<tr>");
                for f in m["fields"].as_array().unwrap() {
                    body.push_str(&format!(
                        "<td data-field=\"{}\"><code>{}</code></td>",
                        escape(strv(&f["path"])),
                        escape(&record["values"][strv(&f["id"])].to_string())
                    ));
                }
                body.push_str(&format!(
                    "<td data-field=\"Evidence\"><details><summary>Source and scope</summary>{}</details></td></tr>",
                    evidence(&record["claim"])
                ));
            }
            body.push_str("</tbody></table>");
        }
        body.push_str("<h2>Fields and representations</h2>");
        for f in m["fields"].as_array().unwrap() {
            body.push_str(&format!("<section id=\"field-{}\"><h3><code>{}</code></h3>{}<table><caption>Representation-specific constraints and mappings</caption><thead><tr><th>Representation</th><th>Type / requiredness / nullability</th><th>Mapping and evidence</th></tr></thead><tbody>",escape(strv(&f["id"])),escape(strv(&f["path"])),evidence(&f["meaning"])));
            for r in f["representations"].as_array().unwrap() {
                let contract = r["contract"].as_str().and_then(|id| {
                    model["contracts"]
                        .as_array()?
                        .iter()
                        .find(|c| c["id"] == id)
                });
                let leaf = contract.and_then(|c| {
                    c["leaves"]
                        .as_array()?
                        .iter()
                        .find(|v| v["field"] == r["field"])
                });
                let constraints = leaf
                    .map(|v| {
                        format!(
                            "{} · Required when branch matches: {} · Accepts null: {}",
                            schema_label(&v["schema"]),
                            v["requiredWhenBranchMatches"],
                            v["nullable"]
                        )
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "{} · {} · {}",
                            strv(&r["typeLabel"]),
                            strv(&r["requiredness"]),
                            strv(&r["nullability"])
                        )
                    });
                let constraints_html = format!(
                    "{}{}",
                    escape(&constraints),
                    r["relatedModel"]
                        .as_str()
                        .map(|id| format!(
                            "<p><a data-dm-context href=\"{}.html\">{}</a></p>",
                            escape(id),
                            escape(
                                model["models"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .find(|m| m["id"] == id)
                                    .and_then(|m| m["title"].as_str())
                                    .unwrap_or(id)
                            )
                        ))
                        .unwrap_or_default()
                );
                body.push_str(&format!(
                    "<tr><th>{}</th><td>{}</td><td>{}</td></tr>",
                    escape(strv(&r["label"])),
                    constraints_html,
                    evidence(&r["mapping"])
                ));
            }
            if reference_only(m) {
                body.push_str("</tbody></table></section>");
            } else {
                body.push_str(&format!("</tbody></table><p><a data-dm-context href=\"?field={}#field-usage\" data-dm-field=\"{}\">Trace this field</a></p></section>",escape(strv(&f["id"])),escape(strv(&f["id"]))));
            }
        }
        if !reference_only(m) {
            body.push_str("<section id=\"field-usage\"><h2>Field usage and focused flow</h2><p>Choose one field or consumer cell. Validation, business decisions, display, storage, forwarding and whole-object comparison remain separate. Unknown is not zero usage.</p><div id=\"dm-usage\"></div></section>");
        }
        body.push_str("<h2>Processes</h2><ul>");
        for link in m["processLinks"].as_array().into_iter().flatten() {
            let subject = strv(&link["subject"]);
            let (kind, id) = subject.split_once(':').unwrap();
            let dir = if kind == "scenario" {
                "scenarios"
            } else {
                "services"
            };
            body.push_str(&format!(
                "<li><a href=\"../{dir}/{}.html#{}\">{}</a></li>",
                escape(id),
                escape(strv(&link["anchor"])),
                escape(strv(&link["label"]))
            ));
        }
        body.push_str("</ul></div>");
        let data = json!({"models":[m],"contracts":model_contracts(m,&model["contracts"]),"authority":model["authority"],"review":model["review"]});
        body.push_str(&data_script(&data));
        files.insert(
            format!("data-model/{id}.html"),
            page(title, &body, "../").into_bytes(),
        );
    }
    index.push_str("</div><h2>Exact contract branches</h2><p>Requiredness is conditional on the selected branch and required parents. Arrays expand element leaves once; counts do not imply runtime frequency.</p>");
    for c in model["contracts"].as_array().unwrap() {
        index.push_str(&format!("<details><summary>{} · {}</summary><p>{}</p><table><tr><th>Field</th><th>Required</th><th>Nullable</th><th>Schema and exact pointer</th></tr>",escape(strv(&c["title"])),escape(strv(&c["branchPointer"])),escape(strv(&c["requiredCondition"]))));
        for f in c["leaves"].as_array().unwrap() {
            index.push_str(&format!("<tr><th>{}</th><td>{}</td><td>{}</td><td><code>{}</code><br><code>{}</code></td></tr>",escape(strv(&f["field"])),f["requiredWhenBranchMatches"],f["nullable"],escape(&f["schema"].to_string()),escape(strv(&f["pointer"]))));
        }
        index.push_str("</table></details>");
    }
    index.push_str("<p><a href=\"data-model.json\">Data model and evidence (JSON)</a></p></div>");
    files.insert(
        "data-model.html".into(),
        page("Data model", &index, "").into_bytes(),
    );
    files.insert(
        "data-model.json".into(),
        serde_json::to_vec_pretty(model).map_err(io_error)?,
    );
    if let Some(overview) = files.get_mut("overview.html") {
        let html = String::from_utf8(overview.clone()).map_err(io_error)?;
        let i = html
            .find("</h1>")
            .ok_or_else(|| invalid("overview heading missing for data-model link"))?
            + 5;
        *overview=format!("{}<p class=\"dm-entry\"><a href=\"data-model.html\">Data model — entities, fields and field usage</a></p>{}",&html[..i],&html[i..]).into_bytes();
    }
    Ok(())
}
fn page(title: &str, body: &str, prefix: &str) -> String {
    let html = reader::page(title, body).replace(
        "</head>",
        &format!(
            "<style>{}</style></head>",
            include_str!("../../../assets/documentation/data-model.css")
        ),
    );
    reader::decorate(
        &html,
        &reader::navigation(
            prefix,
            &format!("{prefix}overview.html"),
            Some(&format!("{prefix}history.html")),
            &[],
        ),
    )
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "ENTITY" => "Domain concept",
        "VALUE_OBJECT" => "Embedded value object",
        "READ_MODEL" => "Read model",
        _ => kind,
    }
}
fn schema_label(schema: &Value) -> String {
    if let Some(value) = schema.get("const") {
        return format!("Literal {value}");
    }
    let mut text = match &schema["type"] {
        Value::String(t) => t.clone(),
        Value::Array(ts) => ts.iter().map(strv).collect::<Vec<_>>().join(" | "),
        _ => "Type not declared".into(),
    };
    if let Some(values) = schema["enum"].as_array() {
        text.push_str(&format!(
            " · one of {}",
            values
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for key in ["format", "minimum", "maximum", "minLength"] {
        if let Some(value) = schema.get(key) {
            text.push_str(&format!(" · {key}: {value}"));
        }
    }
    text
}
fn data_script(data: &Value) -> String {
    let encoded = data
        .to_string()
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!(
        "<script type=\"application/json\" id=\"dm-data\">{encoded}</script><script>{}{}</script>",
        super::super::language::browser_script(),
        include_str!("../../../assets/documentation/data-model.js")
    )
}

fn model_identity(m: &Value, all: &Value) -> String {
    let mut text = evidence(&m["identity"]);
    text.push_str("<p class=\"dm-relations\">Related concepts: ");
    for id in m["relatedModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let title = all["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .map(|m| strv(&m["title"]))
            .unwrap_or(id);
        text.push_str(&format!(
            "<a data-dm-context href=\"{}.html\">{}</a> ",
            escape(id),
            escape(title)
        ));
    }
    text.push_str("</p>");
    if !reference_only(m) {
        text.push_str("<p><a href=\"#field-usage\">Explore field usage and focused flow</a></p>");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        Value,
        BTreeMap<String, Vec<u8>>,
        BTreeMap<String, Value>,
        Check,
    ) {
        let source = json!({"text":"saved bytes","textDigest":crate::canonical::hash_bytes(b"saved bytes"),"revision":"old","startLine":1,"endLine":1});
        let page = json!({"operations":[{"id":"saved","summary":{"id":"claim","text":"Stored field","sourceIds":["source"]}}],"operationSources":{"saved":{"source":source}},"operationStates":{"saved":{"freshness":"CURRENT","verification":"UNASSESSED"}}});
        let block = json!({"title":"Meaning","text":"Proposed field meaning","evidence":[{"kind":"RETAINED","subject":"scenario:example","operation":"saved","fragmentId":"claim","textDigest":crate::canonical::hash_bytes(b"Stored field")} ]});
        let usage = |stage: &str, process: &str| json!({"stage":stage,"producer":"producer","consumer":"consumer","contract":"branch","representation":"row.value","condition":"branch matches","kinds":["persist"],"assessment":"observed-static-use","coveredScope":"selected function","unresolved":"runtime unknown","processes":[process],"claim":block});
        let field = json!({"id":"value","path":"value","meaning":block,"representations":[{"label":"row","typeLabel":"string","requiredness":"required","nullability":"not nullable","mapping":block}],"usage":[usage("Store","scenario:example"),usage("Other reader","scenario:other")]});
        let plan = json!({"schema":"codeclew-data-model-plan/1.0","publication":"pub","snapshot":"snap","contracts":[],"models":[{"id":"record","title":"Record","kind":"READ_MODEL","purpose":block,"identity":block,"fields":[field],"processLinks":[{"subject":"scenario:example","anchor":"question-apply","label":"Apply"}]}],"processViews":[{"subject":"scenario:example","enabled":true,"anchor":"question-apply","title":"Stored field","fields":[{"model":"record","field":"value"}],"stages":["Store"]}]});
        let files = BTreeMap::from([
            (
                "scenarios/example.json".into(),
                serde_json::to_vec(&page).unwrap(),
            ),
            (
                "scenarios/other.json".into(),
                serde_json::to_vec(&page).unwrap(),
            ),
        ]);
        let guides = BTreeMap::from([(
            "scenario:example".into(),
            json!({"presentation":"SEQUENTIAL","sections":[{"id":"apply","blocks":[]}]}),
        )]);
        let check=serde_json::from_value(json!({"schema":"codeclew-documentation-check/1.0","inputDigest":"input","contextDigest":"context","services":{},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap();
        (plan, files, guides, check)
    }
    fn read(
        plan: &Value,
        files: &BTreeMap<String, Vec<u8>>,
        guides: &BTreeMap<String, Value>,
        check: &Check,
    ) -> Result<Value, ClewError> {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.json");
        std::fs::write(&path, serde_json::to_vec(plan).unwrap()).unwrap();
        Ok(load(Some(&path), "pub", "snap", files, check, guides)?.unwrap())
    }
    #[test]
    fn authored_language_inherits_per_model_without_changing_bound_claims() {
        let (mut plan, files, guides, check) = fixture();
        let unknown = read(&plan, &files, &guides, &check).unwrap();
        assert!(unknown["documentationLanguage"].is_null());
        assert!(unknown["models"][0]["documentationLanguage"].is_null());
        plan["documentationLanguage"] = json!("en");
        let inherited = read(&plan, &files, &guides, &check).unwrap();
        assert_eq!(inherited["models"][0]["documentationLanguage"], "en");
        assert_eq!(inherited["processViews"][0]["documentationLanguage"], "en");
        plan["models"][0]["documentationLanguage"] = json!("ru");
        let mixed = read(&plan, &files, &guides, &check).unwrap();
        assert_eq!(mixed["models"][0]["documentationLanguage"], "ru");
        assert_eq!(mixed["processViews"][0]["documentationLanguage"], "en");
        assert_eq!(mixed["models"][0]["fields"], unknown["models"][0]["fields"]);
        assert_eq!(
            mixed["models"][0]["purpose"],
            unknown["models"][0]["purpose"]
        );
        plan["processViews"][0]["documentationLanguage"] = json!("en");
        for pointer in [
            "/documentationLanguage",
            "/models/0/documentationLanguage",
            "/processViews/0/documentationLanguage",
        ] {
            let mut invalid = plan.clone();
            *invalid.pointer_mut(pointer).unwrap() = json!("de");
            assert!(read(&invalid, &files, &guides, &check).is_err());
        }
    }

    #[test]
    fn reference_records_are_bound_bounded_and_linked_to_the_exact_process_stage() {
        let (mut plan, mut files, guides, check) = fixture();
        let mut lookup = plan["models"][0].clone();
        lookup["id"] = json!("lookup");
        lookup["fields"][0]["usage"] = json!([]);
        lookup["records"] = json!([{
            "values":{"value":"<literal>"},"claim":plan["models"][0]["purpose"]
        }]);
        lookup["processLinks"][0]["reference"] = json!({"stages":["Store"]});
        plan["models"].as_array_mut().unwrap().push(lookup);
        let model = read(&plan, &files, &guides, &check).unwrap();
        assert!(reference_only(&model["models"][1]));
        assert_eq!(
            model["models"][1]["records"][0]["claim"]["evidence"][0]["fragment"]["text"],
            "Stored field"
        );
        let mut payload = json!({});
        attach_process(Some(&model), "scenario:example", &mut payload).unwrap();
        assert_eq!(payload["modelReferences"][0]["anchor"], "question-apply");
        assert_eq!(
            payload["processDataFlow"]["modelReferences"][0]["stages"],
            json!(["Store"])
        );
        let mut other = json!({});
        attach_process(Some(&model), "scenario:other", &mut other).unwrap();
        assert!(other.get("modelReferences").is_none());
        write(&model, &mut files).unwrap();
        let html = String::from_utf8(files["data-model/lookup.html"].clone()).unwrap();
        assert!(html.contains("&lt;literal&gt;"));
        assert!(!html.contains("<literal>"));
        assert!(html.contains("Agent proposal · Not reviewed"));
        assert!(!html.contains("id=\"field-usage\""));
        assert!(!html.contains("href=\"#field-usage\""));
        assert!(html.contains("<summary>Source and scope</summary><p>Proposed field meaning</p>"));
        assert!(html.contains("id=\"dm-return\""));
    }
    #[test]
    fn reference_records_reject_wrong_shape_unbound_evidence_and_foreign_stages() {
        let (mut plan, files, guides, check) = fixture();
        plan["models"][0]["records"] =
            json!([{"values":{"value":1.25},"claim":plan["models"][0]["purpose"]}]);
        assert!(read(&plan, &files, &guides, &check).is_ok());
        for values in [
            json!({}),
            json!({"other":1}),
            json!({"value":[1]}),
            json!({"value":1,"other":2}),
        ] {
            let mut bad = plan.clone();
            bad["models"][0]["records"][0]["values"] = values;
            assert!(read(&bad, &files, &guides, &check).is_err());
        }
        let mut bad = plan.clone();
        bad["models"][0]["records"][0]["claim"]["evidence"][0]["textDigest"] = json!("changed");
        assert!(read(&bad, &files, &guides, &check).is_err());
        plan["models"][0]["processLinks"][0]["reference"] = json!({"stages":["Foreign"]});
        assert!(read(&plan, &files, &guides, &check).is_err());
    }
    #[test]
    fn contract_requiredness_keeps_null_and_missing_distinct_and_parent_scope() {
        let schema = json!({"type":"object","required":["forecast"],"properties":{"forecast":{"type":"object","required":["periods"],"properties":{"periods":{"type":"array","items":{"type":"object","required":["rain"],"properties":{"rain":{"type":["number","null"],"minimum":0},"symbol":{"type":"string"}}}}}}}});
        let mut rows = Vec::new();
        leaves(&schema, "", "/oneOf/1", true, &mut rows);
        let rain = rows
            .iter()
            .find(|v| v["field"] == "forecast.periods[].rain")
            .unwrap();
        assert_eq!(rain["requiredWhenBranchMatches"], true);
        assert_eq!(rain["nullable"], true);
        assert_eq!(
            rows.iter()
                .find(|v| v["field"] == "forecast.periods[].symbol")
                .unwrap()["requiredWhenBranchMatches"],
            false
        );
        assert_eq!(accepts_null(&json!({"$ref":"#/$defs/unknown"})), None);
        assert_eq!(
            accepts_null(&json!({"type":["number","null"],"enum":[1,2]})),
            Some(false)
        );
    }
    #[test]
    fn process_opt_in_is_explicit_and_projects_only_the_requested_scope() {
        let (mut plan, files, guides, check) = fixture();
        let mut model = read(&plan, &files, &guides, &check).unwrap();
        model["contracts"] = json!([{"id":"branch"},{"id":"unrelated"}]);
        model["models"][0]["fields"][0]["representations"] = json!([
            {"contract":"branch","field":"value"},
            {"contract":"unrelated","field":"other"},
            {"label":"storage mapping"}
        ]);
        let original = json!({"operations":[{"id":"saved","text":"unchanged"}],"entities":[]});
        let mut payload = original.clone();
        attach_process(None, "scenario:example", &mut payload).unwrap();
        assert_eq!(payload, original);
        attach_process(Some(&model), "scenario:other", &mut payload).unwrap();
        assert_eq!(payload, original);
        attach_process(Some(&model), "scenario:example", &mut payload).unwrap();
        assert_eq!(payload["operations"], original["operations"]);
        assert_eq!(
            payload["processDataFlow"]["contracts"],
            json!([{"id":"branch"}])
        );
        assert_eq!(
            payload["processDataFlow"]["models"][0]["fields"][0]["representations"],
            json!([{"contract":"branch","field":"value"},{"label":"storage mapping"}])
        );
        assert_eq!(
            payload["processDataFlow"]["models"][0]["fields"][0]["usage"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            payload["processDataFlow"]["models"][0]["fields"][0]["usage"][0]["stage"],
            "Store"
        );
        plan["processViews"][0]["enabled"] = json!(false);
        let disabled = read(&plan, &files, &guides, &check).unwrap();
        let mut off = original.clone();
        attach_process(Some(&disabled), "scenario:example", &mut off).unwrap();
        assert_eq!(off, original);
        assert_eq!(model["authority"], "AGENT_PROPOSAL");
        assert_eq!(model["review"], "UNASSESSED");
    }
    #[test]
    fn process_inventory_includes_direct_model_fields_but_not_unrelated_models_or_usage() {
        let (plan, files, guides, check) = fixture();
        let mut model = read(&plan, &files, &guides, &check).unwrap();
        let mut omitted = model["models"][0]["fields"][0].clone();
        omitted["id"] = json!("container");
        omitted["path"] = json!("container");
        model["models"][0]["fields"]
            .as_array_mut()
            .unwrap()
            .push(omitted);
        let mut unrelated = model["models"][0].clone();
        unrelated["id"] = json!("unrelated");
        model["models"].as_array_mut().unwrap().push(unrelated);
        let mut payload = json!({});
        attach_process(Some(&model), "scenario:example", &mut payload).unwrap();
        let models = payload["processDataFlow"]["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        let fields = models[0]["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0]["selectedProcessField"], true);
        assert_eq!(fields[1]["selectedProcessField"], false);
        assert_eq!(fields[1]["usage"], json!([]));
        assert_eq!(
            fields[1]["meaning"],
            model["models"][0]["fields"][1]["meaning"]
        );
    }
    #[test]
    fn reading_guide_is_additive_evidence_bound_and_limited_to_selected_fields_and_stages() {
        let (mut plan, files, guides, check) = fixture();
        let original = plan["models"].clone();
        plan["processViews"][0]["readingGuide"] = json!({"purpose":"Compare where this field is used.","points":[{"fields":[{"model":"record","field":"value"}],"stages":["Store"],"claim":plan["models"][0]["purpose"]}]});
        let model = read(&plan, &files, &guides, &check).unwrap();
        assert_eq!(
            model["processViews"][0]["readingGuide"]["points"][0]["claim"]["evidence"][0]["fragment"]
                ["text"],
            "Stored field"
        );
        assert_eq!(plan["models"], original);
        let mut bad = plan.clone();
        bad["processViews"][0]["readingGuide"]["points"][0]["stages"] = json!(["Other reader"]);
        assert!(read(&bad, &files, &guides, &check).is_err());
        let mut bad = plan.clone();
        bad["processViews"][0]["readingGuide"]["points"][0]["fields"][0]["field"] =
            json!("unmapped");
        assert!(read(&bad, &files, &guides, &check).is_err());
        plan["processViews"][0]["readingGuide"]["points"][0]["claim"]["evidence"][0]["textDigest"] =
            json!("changed");
        assert!(read(&plan, &files, &guides, &check).is_err());
    }
    #[test]
    fn unrelated_contract_changes_do_not_rewrite_a_canonical_model_page() {
        let (plan, mut files, guides, check) = fixture();
        let mut model = read(&plan, &files, &guides, &check).unwrap();
        model["contracts"] = json!([
            {"id":"branch","title":"Used branch","leaves":[]},
            {"id":"unrelated","title":"Unrelated branch","leaves":[]}
        ]);
        write(&model, &mut files).unwrap();
        let before = files["data-model/record.html"].clone();
        let index = files["data-model.html"].clone();
        model["contracts"][1]["title"] = json!("Unrelated branch changed");
        write(&model, &mut files).unwrap();
        assert_eq!(files["data-model/record.html"], before);
        assert_ne!(files["data-model.html"], index);
        let data: Value = serde_json::from_slice(&files["data-model.json"]).unwrap();
        assert_eq!(data["contracts"][1]["title"], "Unrelated branch changed");
    }
    #[test]
    fn canonical_entity_navigation_is_explicit_and_independent_of_field_flow() {
        let (mut plan, mut files, mut guides, check) = fixture();
        plan["processViews"][0]["enabled"] = json!(false);
        plan["models"][0]["processLinks"][0]["replacesServiceEntities"] = json!("sample");
        files.insert("services/sample.json".into(), b"{}".to_vec());
        guides.get_mut("scenario:example").unwrap()["sections"][0]["blocks"] =
            json!([{"links":[{"subject":"service:sample","operation":"section-entities"}]}]);
        let model = read(&plan, &files, &guides, &check).unwrap();
        let original = json!({"operations":[{"id":"saved","text":"unchanged"}]});
        let mut payload = original.clone();
        attach_process(Some(&model), "scenario:example", &mut payload).unwrap();
        assert_eq!(payload["operations"], original["operations"]);
        assert!(payload.get("processDataFlow").is_none());
        assert_eq!(
            payload["canonicalEntityLinks"]["question-apply"]["sample"],
            json!([{"id":"record","title":"Record","documentationLanguage":null,"href":"../data-model/record.html"}])
        );
        let mut other = original.clone();
        attach_process(Some(&model), "scenario:other", &mut other).unwrap();
        assert_eq!(other, original);
        guides.get_mut("scenario:example").unwrap()["sections"][0]["blocks"] = json!([]);
        assert!(read(&plan, &files, &guides, &check).is_err());
        files.remove("services/sample.json");
        assert!(read(&plan, &files, &guides, &check).is_err());
    }
    #[test]
    fn changed_evidence_missing_field_and_unrelated_stage_fail_closed() {
        let (plan, files, guides, check) = fixture();
        for field in ["snapshot", "publication"] {
            let mut bad = plan.clone();
            bad[field] = json!("wrong");
            assert!(read(&bad, &files, &guides, &check).is_err());
        }
        let mut bad = plan.clone();
        bad["processViews"][0]["fields"][0]["field"] = json!("absent");
        assert!(read(&bad, &files, &guides, &check).is_err());
        let mut bad = plan.clone();
        bad["processViews"][0]["stages"] = json!(["unrelated global graph"]);
        assert!(read(&bad, &files, &guides, &check).is_err());
        let mut bad = plan.clone();
        bad["models"][0]["purpose"]["evidence"][0]["textDigest"] = json!("changed");
        assert!(read(&bad, &files, &guides, &check).is_err());
        let mut bad = plan.clone();
        bad["processViews"][0]["relatedModels"] = json!(["absent"]);
        assert!(read(&bad, &files, &guides, &check).is_err());
        let mut bad = plan.clone();
        bad["models"][0]["fields"][0]["usage"][1]["processes"] = json!(["scenario:example"]);
        bad["processViews"][0]["stages"] = json!(["Store", "Other reader"]);
        bad["processViews"][0]["transfers"] = json!([{"from":"Store","to":"Other reader","kind":"transfer","authority":"OBSERVED_RUNTIME","condition":"Test condition","claim":plan["models"][0]["purpose"]}]);
        assert!(read(&bad, &files, &guides, &check).is_err());
        bad["processViews"][0]["transfers"][0]["authority"] = json!("UNKNOWN");
        assert!(read(&bad, &files, &guides, &check).is_ok());
        let mut non_sequential = guides.clone();
        non_sequential.get_mut("scenario:example").unwrap()["presentation"] = json!("QUESTIONS");
        assert!(read(&plan, &files, &non_sequential, &check).is_err());
        let mut bad = plan;
        bad["processViews"][0]["anchor"] = json!("question-missing");
        assert!(read(&bad, &files, &guides, &check).is_err());
    }
}
