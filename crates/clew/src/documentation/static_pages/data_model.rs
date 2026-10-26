//! Optional authored domain mappings over admitted publication/source evidence.
use super::super::{
    check::Check, invalid, io_error, language::label, reader, render::escape, store,
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

mod payload_views;

const CONTRACT_REQUIRED_CONDITION: &str = "Selected contract branch matches; required parents and array element are present. Array leaves expanded once, independent of cardinality.";

#[derive(Default)]
pub(super) struct LanguageAvailability {
    pub models: BTreeMap<String, Value>,
    pub contracts: BTreeMap<String, Value>,
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    payload_views: Vec<payload_views::PayloadView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    interface_bindings: Vec<InterfaceBinding>,
    #[serde(default)]
    contracts: Vec<Contract>,
    #[serde(default)]
    process_views: Vec<ProcessView>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InterfaceBinding {
    contract: String,
    subject: String,
    operation: String,
    interface_contract: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FieldCoverage {
    status: String,
    scope: String,
    claim: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Model {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    field_coverage: Option<FieldCoverage>,
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
struct GraphBinding {
    subject: String,
    node: String,
    claim: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Field {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    graph_bindings: Vec<GraphBinding>,
    id: String,
    path: String,
    /// Explicit authored names with exact evidence, separate from the comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    readable_name: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_name: Option<Value>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    inputs: Vec<String>,
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
fn validate_interface_bindings(
    bindings: &[InterfaceBinding],
    contracts: &[Value],
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(), ClewError> {
    let mut bound_cards = BTreeSet::new();
    for binding in bindings {
        if !contracts.iter().any(|c| c["id"] == binding.contract)
            || !bound_cards.insert((
                &binding.subject,
                &binding.operation,
                &binding.interface_contract,
            ))
        {
            return Err(invalid(
                "interface declaration binding is absent or repeats a card",
            ));
        }
        let page = page_for(files, &binding.subject)?;
        let found = page["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|op| op["id"] == binding.operation)
            .flat_map(|op| op["interfaceContracts"].as_array().into_iter().flatten())
            .any(|card| card["id"] == binding.interface_contract);
        if !found {
            return Err(invalid(
                "interface declaration binding must name an exact retained operation/card",
            ));
        }
    }
    Ok(())
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
        || (plan.models.is_empty() && plan.contracts.is_empty() && plan.payload_views.is_empty())
        || plan.models.len() > 32
        || plan.contracts.len() > 32
        || plan.process_views.len() > 32
        || plan.interface_bindings.len() > 128
    {
        return Err(invalid("data-model schema, binding or bounds are invalid"));
    }
    let mut ids = BTreeSet::new();
    let mut bytes = super::question_reading::EvidenceBudget::default();
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
        let declaration =
            super::inline_contract::project_document(branch, &c.branch_pointer, &schema)?;
        contracts.push(json!({"declaration":declaration,"id":c.id,"title":c.title,"documentationLanguage":c.documentation_language.as_ref().or(plan.documentation_language.as_ref()),"branchPointer":c.branch_pointer,"leaves":rows,"source":source,"requiredCondition":CONTRACT_REQUIRED_CONDITION}));
    }
    validate_interface_bindings(&plan.interface_bindings, &contracts, files)?;
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
        if let Some(entity) = &model.entity
            && (!entity.starts_with("entity:")
                || checked
                    .dependencies
                    .get(entity)
                    .is_none_or(|d| d.kind != "DOMAIN_ENTITY"))
        {
            return Err(invalid(
                "model entity must name a selected declared domain concept",
            ));
        }
        if let Some(coverage) = &mut model.field_coverage {
            if !matches!(
                coverage.status.as_str(),
                "complete-definition" | "observed-fields" | "unknown"
            ) {
                return Err(invalid("invalid field coverage status"));
            }
            text(&coverage.scope)?;
            coverage.claim = super::question_reading::bind_block(
                coverage.claim.clone(),
                files,
                checked,
                &mut bytes,
            )?;
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
            if field.graph_bindings.len() > 16 {
                return Err(invalid("field graph bindings exceed bounds"));
            }
            for binding in &mut field.graph_bindings {
                if !binding.subject.starts_with("scenario:")
                    || !model
                        .process_links
                        .iter()
                        .any(|link| link.subject == binding.subject)
                {
                    return Err(invalid(
                        "field graph binding requires an explicit related process link",
                    ));
                }
                let page = page_for(files, &binding.subject)?;
                if !page["operations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|op| op["dataflow"]["nodes"].as_array())
                    .flatten()
                    .any(|node| node["id"] == binding.node)
                {
                    return Err(invalid(
                        "field binding must name a node in the exact selected graph",
                    ));
                }
                binding.claim = super::question_reading::bind_block(
                    binding.claim.clone(),
                    files,
                    checked,
                    &mut bytes,
                )?;
            }
            text(&field.path)?;
            for name in [&mut field.readable_name, &mut field.original_name]
                .into_iter()
                .flatten()
            {
                let value = name["text"]
                    .as_str()
                    .ok_or_else(|| invalid("field name needs an evidence-bound text claim"))?;
                if value.trim().is_empty()
                    || value.len() > 256
                    || value.chars().any(char::is_control)
                {
                    return Err(invalid(
                        "field name must be nonblank single-line text of at most 256 bytes",
                    ));
                }
                *name =
                    super::question_reading::bind_block(name.clone(), files, checked, &mut bytes)?;
            }
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
                if u.inputs.len() > 32 {
                    return Err(invalid("field usage inputs exceed bounds"));
                }
                for input in &u.inputs {
                    text(input)?;
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
    let payload_views = payload_views::validate_and_project(
        &mut plan.payload_views,
        &plan.models,
        files,
        checked,
        &mut bytes,
    )?;
    let mut value = serde_json::to_value(&plan).map_err(io_error)?;
    value["payloadViews"] = json!(payload_views);
    value["documentationLanguage"] = json!(plan.documentation_language);
    for kind in ["models", "processViews", "payloadViews"] {
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
fn evidence(claim: &Value, language: &str) -> String {
    evidence_content(claim, language, true)
}
fn evidence_content(claim: &Value, language: &str, show_text: bool) -> String {
    let mut out = format!(
        "{}<details><summary>{clew_label_0}</summary>",
        if show_text {
            format!(
                "<div class=\"clew-markdown\" data-clew-markdown>{}</div>",
                escape(strv(&claim["text"]))
            )
        } else {
            String::new()
        },
        clew_label_0 = escape(label(language, "Evidence and scope"))
    );
    for e in claim["evidence"].as_array().into_iter().flatten() {
        if !e["state"].is_null() {
            out.push_str(&format!(
                "<p>{clew_label_0}: {}</p>",
                escape(&e["state"].to_string()),
                clew_label_0 = escape(label(language, "Retained state"))
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
            let href = crate::documentation::source_export::range_url(url, start, end);
            out.push_str(&format!(
                "<p><a href=\"{}\">{}:{}–{}</a> · {} <code>{}</code> · {}</p>",
                escape(&href),
                escape(strv(&s["file"])),
                start,
                end,
                escape(label(language, "revision")),
                escape(strv(&s["revision"])),
                escape(strv(&s["authority"]))
            ));
            if !source.is_empty() {
                out.push_str(&format!("<pre tabindex=\"0\">{}</pre>", escape(&source)));
            }
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
pub(super) fn attach_external_payloads(
    model: Option<&Value>,
    files: &mut BTreeMap<String, Vec<u8>>,
    language: &str,
) -> Result<(), ClewError> {
    payload_views::attach_external_links(model, files, language)
}

pub(super) fn attach_process(
    model: Option<&Value>,
    subject: &str,
    payload: &mut Value,
) -> Result<(), ClewError> {
    let Some(model) = model else {
        return Ok(());
    };
    let payload_views = model["payloadViews"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["subject"] == subject)
        .collect::<Vec<_>>();
    if !payload_views.is_empty() {
        payload["payloadViews"] = json!(payload_views);
    }
    // The model's exact domain identity plus a retained explicit service
    // relation is sufficient for dictionary navigation. It does not assert
    // field usage, runtime ownership or a relationship to any other service.
    let declared_models: BTreeSet<&str> = model["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            m["entity"].as_str().is_some_and(|entity| {
                subject.strip_prefix("service:").is_some_and(|service| {
                    payload["entities"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|d| {
                            d["kind"] == "DOMAIN_ENTITY"
                                && d["id"] == entity
                                && d["normalized"]["entity"]["relations"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|relation| relation["service"] == service)
                        })
                })
            })
        })
        .filter_map(|m| m["id"].as_str())
        .collect();
    let reader_models = json!(
        model["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| declared_models.contains(strv(&m["id"]))
                || m["processLinks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|l| l["subject"] == subject)
                || m["fields"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|f| f["graphBindings"].as_array().into_iter().flatten())
                    .any(|b| b["subject"] == subject))
            .collect::<Vec<_>>()
    );
    let reader_enabled = subject.starts_with("service:")
        || payload["view"].is_object()
        || payload["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|operation| operation["dataflow"].is_object())
        || model["processViews"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|view| view["subject"] == subject && view["enabled"] == true);
    if reader_enabled
        && reader_models
            .as_array()
            .is_some_and(|models| !models.is_empty())
    {
        payload["readerModels"] = reader_models;
    }
    for binding in model["interfaceBindings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["subject"] == subject)
    {
        let contract = model["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["id"] == binding["contract"])
            .ok_or_else(|| invalid("bound inline contract is unavailable"))?;
        if !payload["interfaceContractDeclarations"].is_object() {
            payload["interfaceContractDeclarations"] = json!({});
        }
        let operation = strv(&binding["operation"]);
        if !payload["interfaceContractDeclarations"][operation].is_object() {
            payload["interfaceContractDeclarations"][operation] = json!({});
        }
        let mut declaration = contract.clone();
        declaration["binding"] = json!({"authority":model["authority"],"review":model["review"],"subject":subject,"operation":binding["operation"],"interfaceContract":binding["interfaceContract"]});
        payload["interfaceContractDeclarations"][operation][strv(&binding["interfaceContract"])] =
            declaration;
    }
    // Canonical navigation is independent of the optional field-flow reader.
    // The author must bind the exact legacy service link and question explicitly.
    let mut navigation = serde_json::Map::new();
    let mut references = Vec::new();
    let mut service_links = Vec::new();
    for m in model["models"].as_array().into_iter().flatten() {
        if subject.starts_with("service:")
            && (declared_models.contains(strv(&m["id"]))
                || m["processLinks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|link| link["subject"] == subject && link["anchor"] == "section-entities"))
        {
            let fields = m["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| {
                    json!({"id":f["id"],"path":f["path"],
                        "originalName":f.get("originalName").map(|n|json!({"text":n["text"]})),
                        "readableName":f.get("readableName").map(|n|json!({"text":n["text"]})),
                        "meaning":{"text":f["meaning"]["text"]}})
                })
                .collect::<Vec<_>>();
            let mut provenance = m.clone();
            // Full source and representation evidence remains on canonical model pages.
            // This compact disclosure retains every exact evidence identity without copying source bytes.
            fn compact(v: &mut Value) {
                match v {
                    Value::Object(map) => {
                        if map.contains_key("textDigest") && map.contains_key("file") {
                            map.remove("text");
                        }
                        for value in map.values_mut() {
                            compact(value);
                        }
                    }
                    Value::Array(values) => {
                        for value in values {
                            compact(value);
                        }
                    }
                    _ => (),
                }
            }
            compact(&mut provenance);
            let summary = json!({"id":m["id"],"title":m["title"],"fields":fields});
            let language = payload["language"].as_str().unwrap_or("en");
            service_links.push(json!({"id":m["id"],"title":m["title"],"kind":m["kind"],
                    "documentationLanguage":m["documentationLanguage"],"authority":model["authority"],"review":model["review"],
                    "fieldSummary":summary,"fieldTableHtml":service_field_table(&summary,language,false),"provenance":provenance,
                    "href":format!("../data-model/{}.html#fields-and-representations",strv(&m["id"]))}));
        }
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
    if !service_links.is_empty() {
        payload["serviceModelLinks"] = json!(service_links);
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

/// A static field inventory survives without the optional interactive reader.
/// Representation-specific types and requiredness are never collapsed into a
/// made-up domain-wide type or constraint. Existing field anchors hold evidence.
pub(super) fn field_table(model: &Value, language: &str, portable: bool) -> String {
    field_table_at(model, language, portable, None)
}
pub(super) fn service_field_table(model: &Value, language: &str, portable: bool) -> String {
    let prefix = format!(
        "../data-model/{}{}",
        strv(&model["id"]),
        if portable { "" } else { ".html" }
    );
    field_table_at(model, language, portable, Some(&prefix))
}
fn field_table_at(model: &Value, language: &str, portable: bool, prefix: Option<&str>) -> String {
    let literal = |value: &str| escape(value).replace('{', "&#123;").replace('}', "&#125;");
    let attr = if portable { "className" } else { "class" };
    let focus = if portable {
        "tabIndex={0}"
    } else {
        "tabindex=\"0\""
    };
    let heading = |key| literal(label(language, key));
    let fields = model["fields"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let missing_names = fields.iter().any(|field| {
        field["readableName"]["text"]
            .as_str()
            .is_none_or(|name| name.trim().is_empty())
    });
    let fallback_note = if missing_names {
        format!(
            "<p {attr}=\"small-label\">{}</p>",
            heading("Source paths are shown where readable names are not provided.")
        )
    } else {
        String::new()
    };
    let mut out = format!(
        "{fallback_note}<div {attr}=\"dm-fields-scroll\" {focus} role=\"region\" aria-label=\"{}\"><table {attr}=\"dm-fields\" role=\"table\"><caption>{}</caption><thead><tr><th scope=\"col\">{}</th><th scope=\"col\">{}</th></tr></thead><tbody>",
        heading("Field descriptions"),
        heading("Field descriptions"),
        heading("Readable name"),
        heading("Comment")
    );
    for field in fields {
        let readable_name = field["readableName"]["text"]
            .as_str()
            .filter(|name| !name.trim().is_empty());
        let name = readable_name
            .map(literal)
            .unwrap_or_else(|| format!("<code>{}</code>", literal(strv(&field["path"]))));
        let comment = field["meaning"]["text"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| label(language, "Field comment not provided"));
        let body = if portable {
            super::markdown::mdx(comment)
        } else {
            format!(
                "<div class=\"clew-markdown\" data-clew-markdown>{}</div>",
                literal(comment)
            )
        };
        let href = format!("{}#field-{}", prefix.unwrap_or(""), strv(&field["id"]));
        let disclosure = if prefix.is_none() {
            " data-clew-field-details=\"\""
        } else {
            ""
        };
        out.push_str(&format!("<tr><th scope=\"row\"><a href=\"{}\"{disclosure}>{name}</a></th><td data-label=\"{}\">{}</td></tr>",literal(&href),heading("Comment"),body));
    }
    out.push_str("</tbody></table></div>\n\n");
    out
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
#[cfg(test)]
pub(super) fn write(model: &Value, files: &mut BTreeMap<String, Vec<u8>>) -> Result<(), ClewError> {
    write_language(model, files, None).map(|_| ())
}

pub(super) fn write_language(
    model: &Value,
    files: &mut BTreeMap<String, Vec<u8>>,
    requested: Option<&str>,
) -> Result<LanguageAvailability, ClewError> {
    let language = requested.unwrap_or("en");
    let mut gaps = BTreeMap::new();
    let mut index = format!(
        "<div class=\"dm-page\"><h1>{clew_label_0}</h1><p>{clew_label_1}</p><p class=\"dm-status\">{clew_label_2}</p><div class=\"dm-cards\">",
        clew_label_0 = escape(label(language, "Data model")),
        clew_label_1 = escape(label(
            language,
            "Information the system works with, its representations, and the processes that use it."
        )),
        clew_label_2 = escape(label(
            language,
            "Agent proposals · Not reviewed. Original descriptions and review states are retained separately. These are static interpretations, not ownership or runtime guarantees."
        ))
    );
    for m in model["models"].as_array().unwrap() {
        let id = strv(&m["id"]);
        let title = strv(&m["title"]);
        let original_path = super::prose_language::original_path(id)?;
        files.insert(
            original_path.clone(),
            serde_json::to_vec_pretty(m).map_err(io_error)?,
        );
        let gap = super::prose_language::gap(
            &m["documentationLanguage"],
            requested,
            "MODEL_TRANSLATION",
            &original_path,
        )?;
        let card_notice = gap
            .as_ref()
            .map(|gap| super::prose_language::html(gap, &original_path))
            .unwrap_or_default();
        index.push_str(&format!(
            "<article><p>{}</p><h2><a href=\"data-model/{}.html\">{}</a></h2><p>{}</p>{card_notice}</article>",
            escape(kind_label(strv(&m["kind"]), language)),
            escape(id),
            escape(title),
            escape(strv(&m["purpose"]["text"]))
        ));
        let mut body = format!(
            "<div class=\"dm-page\"><p><a href=\"../data-model.html\">{clew_label_0}</a> / {}</p><p id=\"dm-return\"></p><h1>{}</h1><p class=\"dm-status\">{} · {clew_label_1} · {clew_label_2}</p>{}<h2>{clew_label_3}</h2>{}",
            escape(title),
            escape(title),
            escape(kind_label(strv(&m["kind"]), language)),
            evidence(&m["purpose"], language),
            model_identity(m, model, language),
            clew_label_0 = escape(label(language, "Data model")),
            clew_label_1 = escape(label(language, "Agent proposal")),
            clew_label_2 = escape(label(language, "Not reviewed")),
            clew_label_3 = escape(label(language, "Identity and relationships"))
        );
        if let Some(gap) = gap {
            let href = original_path.strip_prefix("data-model/").unwrap();
            body.push_str(&super::prose_language::html(&gap, href));
            gaps.insert(id.to_owned(), gap);
        }
        if m["records"].as_array().is_some_and(|rows| !rows.is_empty()) {
            body.push_str(&format!("<h2>{clew_label_0}</h2><table class=\"dm-records\"><caption>{clew_label_1}</caption><thead><tr>", clew_label_0 = escape(label(language, "Reference records")), clew_label_1 = escape(label(language, "Source-backed literal records · authored interpretation"))));
            for f in m["fields"].as_array().unwrap() {
                body.push_str(&format!(
                    "<th scope=\"col\">{}</th>",
                    escape(strv(&f["path"]))
                ));
            }
            body.push_str(&format!(
                "<th scope=\"col\">{clew_label_0}</th></tr></thead><tbody>",
                clew_label_0 = escape(label(language, "Evidence"))
            ));
            for record in m["records"].as_array().unwrap() {
                body.push_str("<tr>");
                for f in m["fields"].as_array().unwrap() {
                    body.push_str(&format!(
                        "<td data-field=\"{}\"><code>{}</code></td>",
                        escape(strv(&f["path"])),
                        escape(&record["values"][strv(&f["id"])].to_string())
                    ));
                }
                body.push_str(&format!("<td data-field=\"Evidence\" data-label=\"{clew_label_1}\"><details><summary>{clew_label_0}</summary>{}</details></td></tr>",
                    evidence(&record["claim"], language), clew_label_0 = escape(label(language, "Source and scope")), clew_label_1 = escape(label(language, "Evidence"))));
            }
            body.push_str("</tbody></table>");
        }
        body.push_str(&format!(
            "<h2 id=\"fields-and-representations\">{clew_label_0}</h2>",
            clew_label_0 = escape(label(language, "Fields and representations"))
        ));
        body.push_str(&field_table(m, language, false));
        for f in m["fields"].as_array().unwrap() {
            body.push_str(&format!("<details class=\"dm-field-details\" id=\"field-{}\"><summary><code>{}</code> · {}</summary>{}<table><caption>{clew_label_0}</caption><thead><tr><th>{clew_label_1}</th><th>{clew_label_2}</th><th>{clew_label_3}</th></tr></thead><tbody>",escape(strv(&f["id"])),escape(strv(&f["path"])),escape(label(language,"Technical representations")),evidence_content(&f["meaning"], language,false), clew_label_0 = escape(label(language, "Representation-specific constraints and mappings")), clew_label_1 = escape(label(language, "Representation")), clew_label_2 = escape(label(language, "Type / requiredness / nullability")), clew_label_3 = escape(label(language, "Mapping and evidence"))));
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
                            "{} · {clew_label_0}: {} · {clew_label_1}: {}",
                            schema_label(&v["schema"], language),
                            v["requiredWhenBranchMatches"],
                            v["nullable"],
                            clew_label_0 = label(language, "Required when branch matches"),
                            clew_label_1 = label(language, "Accepts null")
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
                    evidence(&r["mapping"], language)
                ));
            }
            body.push_str("</tbody></table>");
            for (key, title) in [
                ("originalName", "Original field"),
                ("readableName", "Readable name"),
            ] {
                if f[key].is_object() {
                    body.push_str(&format!(
                        "<details><summary>{}: {}</summary>{}</details>",
                        escape(label(language, "Field label provenance")),
                        escape(label(language, title)),
                        evidence(&f[key], language)
                    ));
                }
            }
            if reference_only(m) {
                body.push_str("</details>");
            } else {
                body.push_str(&format!("<p><a data-dm-context href=\"?field={}#field-usage\" data-dm-field=\"{}\">{clew_label_0}</a></p></details>",escape(strv(&f["id"])),escape(strv(&f["id"])), clew_label_0 = escape(label(language, "Trace this field"))));
            }
        }
        if !reference_only(m) {
            body.push_str(&format!("<section id=\"field-usage\"><h2>{clew_label_0}</h2><p>{clew_label_1}</p><div id=\"dm-usage\"></div></section>", clew_label_0 = escape(label(language, "Field usage and focused flow")), clew_label_1 = escape(label(language, "Choose one field or consumer cell. Validation, business decisions, display, storage, forwarding and whole-object comparison remain separate. Unknown is not zero usage."))));
        }
        body.push_str(&format!(
            "<h2>{clew_label_0}</h2><ul>",
            clew_label_0 = escape(label(language, "Processes"))
        ));
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
            page_language(title, &body, "../", requested.unwrap_or("en")).into_bytes(),
        );
    }
    index.push_str(&format!("</div><h2>{clew_label_0}</h2><p>{clew_label_1}</p>", clew_label_0 = escape(label(language, "Exact contract branches")), clew_label_1 = escape(label(language, "Requiredness is conditional on the selected branch and required parents. Arrays expand element leaves once; counts do not imply runtime frequency."))));
    let mut contract_gaps = BTreeMap::new();
    for c in model["contracts"].as_array().unwrap() {
        let id = strv(&c["id"]);
        if !store::valid_id(id) {
            return Err(invalid(
                "contract original description identifier is unsafe",
            ));
        }
        let original_path = format!("contracts/{id}.original.json");
        files.insert(
            original_path.clone(),
            serde_json::to_vec_pretty(c).map_err(io_error)?,
        );
        let gap = super::prose_language::gap(
            &c["documentationLanguage"],
            requested,
            "DECLARED_CONTRACT_TRANSLATION",
            &original_path,
        )?;
        if let Some(gap) = &gap {
            contract_gaps.insert(id.to_owned(), gap.clone());
        }
        let notice = gap
            .as_ref()
            .map(|gap| super::prose_language::html(gap, &original_path))
            .unwrap_or_default();
        let mut contract_body = format!(
            "<div class=\"dm-page dm-contract-page\"><p id=\"dm-return\"></p><h1>{}</h1><p>{}</p>",
            escape(strv(&c["title"])),
            escape(label(
                language,
                "Declared contract · runtime conformance is not established."
            ))
        );
        if let Some(gap) = &gap {
            contract_body.push_str(&super::prose_language::html(
                gap,
                &format!("{id}.original.json"),
            ));
        }
        if c["declaration"].is_object() {
            contract_body.push_str(&super::inline_contract::table_html(
                &c["declaration"],
                language,
            ));
        } else {
            contract_body.push_str(&format!("<p>{}</p><table><thead><tr><th>Field</th><th>Required</th><th>Nullable</th><th>Schema</th></tr></thead><tbody>", escape(label(language, CONTRACT_REQUIRED_CONDITION))));
            for field in c["leaves"].as_array().into_iter().flatten() {
                contract_body.push_str(&format!(
                    "<tr><th>{}</th><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
                    escape(strv(&field["field"])),
                    field["requiredWhenBranchMatches"],
                    field["nullable"],
                    escape(&field["schema"].to_string())
                ));
            }
            contract_body.push_str("</tbody></table>");
        }
        contract_body.push_str(&format!("<details><summary>{}</summary><p><a href=\"{}.original.json\">{}</a></p><pre>{}</pre></details></div>",
            escape(label(language,"Declaration and provenance")),escape(id),escape(label(language,"Complete original source binding")),escape(&serde_json::to_string_pretty(c).map_err(io_error)?)));
        contract_body.push_str(&data_script(&json!({"models":[],"contracts":[c],"authority":model["authority"],"review":model["review"]})));
        files.insert(
            format!("contracts/{id}.html"),
            page_language(strv(&c["title"]), &contract_body, "../", language).into_bytes(),
        );
        index.push_str(&format!("<details><summary>{} · {}</summary>{notice}<p>{}</p><table><tr><th>{clew_label_0}</th><th>{clew_label_1}</th><th>{clew_label_2}</th><th>{clew_label_3}</th></tr>",escape(strv(&c["title"])),escape(strv(&c["branchPointer"])),escape(label(language, CONTRACT_REQUIRED_CONDITION)), clew_label_0 = escape(label(language, "Field")), clew_label_1 = escape(label(language, "Required")), clew_label_2 = escape(label(language, "Nullable")), clew_label_3 = escape(label(language, "Schema and exact pointer"))));
        for f in c["leaves"].as_array().unwrap() {
            index.push_str(&format!("<tr><th>{}</th><td>{}</td><td>{}</td><td><code>{}</code><br><code>{}</code></td></tr>",escape(strv(&f["field"])),f["requiredWhenBranchMatches"],f["nullable"],escape(&f["schema"].to_string()),escape(strv(&f["pointer"]))));
        }
        index.push_str("</table></details>");
    }
    index.push_str(&format!(
        "<p><a href=\"data-model.json\">{clew_label_0}</a></p></div>",
        clew_label_0 = escape(label(language, "Data model and evidence (JSON)"))
    ));
    files.insert(
        "data-model.html".into(),
        page_language(label(language, "Data model"), &index, "", language).into_bytes(),
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
        *overview = format!(
            "{}<p class=\"dm-entry\"><a href=\"data-model.html\">{clew_label_0}</a></p>{}",
            &html[..i],
            &html[i..],
            clew_label_0 = escape(label(
                language,
                "Data model — entities, fields and field usage"
            ))
        )
        .into_bytes();
    }
    for view in model["payloadViews"].as_array().into_iter().flatten() {
        let key = format!(
            "payload:{}:{}:{}",
            strv(&view["subject"]),
            strv(&view["operation"]),
            strv(&view["id"])
        );
        let original_path = format!(
            "payload-views/{}.original.json",
            &super::super::digest(&key)?[7..]
        );
        files.insert(
            original_path.clone(),
            serde_json::to_vec_pretty(view).map_err(io_error)?,
        );
        if let Some(gap) = super::prose_language::gap(
            &view["documentationLanguage"],
            requested,
            "PAYLOAD_TRANSLATION",
            &original_path,
        )? {
            gaps.insert(key, gap);
        }
    }
    Ok(LanguageAvailability {
        models: gaps,
        contracts: contract_gaps,
    })
}
fn page_language(title: &str, body: &str, prefix: &str, language: &str) -> String {
    let html = reader::page_language(title, body, language).replace(
        "</head>",
        &format!(
            "<style>{}\n{}</style></head>",
            include_str!("../../../assets/documentation/data-model.css"),
            include_str!("../../../assets/documentation/markdown.css")
        ),
    );
    let html = reader::decorate(
        &html,
        &reader::navigation_language(
            prefix,
            &format!("{prefix}overview.html"),
            Some(&format!("{prefix}history.html")),
            &[],
            language,
        ),
    );
    html.replace(
        "</body>",
        &format!(
            "<script>{}\nclewMarkdown.enhance(document,{{tableLabel:{}}});</script></body>",
            include_str!("../../../assets/documentation/markdown.js"),
            json!(label(language, "Table"))
        ),
    )
}

fn kind_label<'a>(kind: &'a str, language: &str) -> &'a str {
    label(
        language,
        match kind {
            "ENTITY" => "Domain concept",
            "VALUE_OBJECT" => "Embedded value object",
            "READ_MODEL" => "Read model",
            _ => kind,
        },
    )
}
fn schema_label(schema: &Value, language: &str) -> String {
    if let Some(value) = schema.get("const") {
        return format!(
            "{clew_label_0} {value}",
            clew_label_0 = label(language, "Literal")
        );
    }
    let mut text = match &schema["type"] {
        Value::String(t) => t.clone(),
        Value::Array(ts) => ts.iter().map(strv).collect::<Vec<_>>().join(" | "),
        _ => label(language, "Type not declared").into(),
    };
    if let Some(values) = schema["enum"].as_array() {
        text.push_str(&format!(
            " · {clew_label_0} {}",
            values
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            clew_label_0 = label(language, "one of")
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

fn model_identity(m: &Value, all: &Value, language: &str) -> String {
    let mut text = evidence(&m["identity"], language);
    text.push_str(&format!(
        "<p class=\"dm-relations\">{clew_label_0}: ",
        clew_label_0 = escape(label(language, "Related concepts"))
    ));
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
        text.push_str(&format!(
            "<p><a href=\"#field-usage\">{clew_label_0}</a></p>",
            clew_label_0 = escape(label(language, "Explore field usage and focused flow"))
        ));
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
    fn repeated_shared_source_keeps_all_models_fields_and_payloads_within_unique_budget() {
        // Synthetic accounting fixture, not semantic approval: many independent
        // descriptions cite the same exact retained source body.
        let (mut plan, mut files, guides, checked) = fixture();
        let text = "s".repeat(16 * 1024);
        for bytes in files.values_mut() {
            let mut page: Value = serde_json::from_slice(bytes).unwrap();
            page["operationSources"]["saved"]["source"]["text"] = json!(text);
            page["operationSources"]["saved"]["source"]["textDigest"] =
                json!(crate::canonical::hash_bytes(text.as_bytes()));
            *bytes = serde_json::to_vec(&page).unwrap();
        }
        let original = plan["models"][0].clone();
        let models = (0..11)
            .map(|i| {
                let mut model = original.clone();
                model["id"] = json!(if i == 0 {
                    "record".into()
                } else {
                    format!("record-{i}")
                });
                model["fields"] = json!(
                    (0..6)
                        .map(|j| {
                            let mut field = original["fields"][0].clone();
                            field["id"] = json!(if j == 0 {
                                "value".into()
                            } else {
                                format!("value-{j}")
                            });
                            field["path"] = json!(format!("value{j}"));
                            field
                        })
                        .collect::<Vec<_>>()
                );
                model
            })
            .collect::<Vec<_>>();
        plan["models"] = json!(models);
        plan["payloadViews"]=json!((0..5).map(|i|json!({"id":format!("shape-{i}"),"title":"Observed source shape","subject":"scenario:example","operation":"saved","component":"response-body","direction":"outgoing","format":"source representation; wire format unknown","coverage":{"status":"observed-fields","scope":"Selected field only; not a complete wire schema","claim":original["purpose"]},"fields":[{"model":"record","field":"value","representation":"row","path":"value0"}]})).collect::<Vec<_>>());
        let projected = read(&plan, &files, &guides, &checked).unwrap();
        assert_eq!(projected["models"].as_array().unwrap().len(), 11);
        assert_eq!(
            projected["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|model| model["fields"].as_array().unwrap().len())
                .sum::<usize>(),
            66
        );
        assert_eq!(projected["payloadViews"].as_array().unwrap().len(), 5);
        plan["models"][10]["fields"][5]["meaning"]["evidence"][0]["textDigest"] =
            json!(format!("sha256:{}", "0".repeat(64)));
        assert!(
            read(&plan, &files, &guides, &checked)
                .unwrap_err()
                .message
                .contains("claim text changed")
        );
    }
    #[test]
    fn explicit_domain_field_coverage_and_graph_mappings_remain_bounded_and_evidence_bound() {
        let (mut plan, mut files, guides, mut check) = fixture();
        let claim = plan["models"][0]["purpose"].clone();
        check.dependencies.insert("entity:record".into(), serde_json::from_value(json!({"id":"entity:record","kind":"DOMAIN_ENTITY","service":"","symbol":"Record","digest":"owned","normalized":{},"sourceIds":[]})).unwrap());
        plan["models"][0]["entity"] = json!("entity:record");
        plan["models"][0]["fieldCoverage"] = json!({"status":"observed-fields","scope":"Only saved usage, not a complete definition","claim":claim});
        let mut page: Value = serde_json::from_slice(&files["scenarios/example.json"]).unwrap();
        page["operations"][0]["dataflow"] = json!({"nodes":[{"id":"result"}]});
        files.insert(
            "scenarios/example.json".into(),
            serde_json::to_vec(&page).unwrap(),
        );
        plan["models"][0]["fields"][0]["graphBindings"] =
            json!([{"subject":"scenario:example","node":"result","claim":claim}]);
        plan["models"][0]["fields"][0]["usage"][0]["inputs"] = json!(["request.value"]);
        let model = read(&plan, &files, &guides, &check).unwrap();
        assert_eq!(
            model["models"][0]["fieldCoverage"]["status"],
            "observed-fields"
        );
        assert_eq!(
            model["models"][0]["fields"][0]["usage"][0]["inputs"][0],
            "request.value"
        );
        for (pointer, replacement) in [
            ("/models/0/entity", json!("entity:missing")),
            (
                "/models/0/fieldCoverage/status",
                json!("all-fields-because-list-nonempty"),
            ),
            (
                "/models/0/fields/0/graphBindings/0/subject",
                json!("scenario:other"),
            ),
            ("/models/0/fields/0/graphBindings/0/node", json!("foreign")),
            (
                "/models/0/fields/0/graphBindings/0/claim/evidence/0/fragmentId",
                json!("unreceived"),
            ),
        ] {
            let mut bad = plan.clone();
            *bad.pointer_mut(pointer).unwrap() = replacement;
            assert!(read(&bad, &files, &guides, &check).is_err(), "{pointer}");
        }
        let mut payload = json!({"language":"en","view":{}});
        attach_process(Some(&model), "scenario:example", &mut payload).unwrap();
        assert_eq!(payload["readerModels"][0]["entity"], "entity:record");
        assert_eq!(
            payload["readerModels"][0]["fields"][0]["graphBindings"][0]["node"],
            "result"
        );
    }

    #[test]
    fn contracts_only_plan_binds_exact_declaration_without_models_or_field_flow() {
        let (mut plan, mut files, guides, mut checked) = fixture();
        let schema = json!({"$defs":{"Event":{"type":"object","required":["rain"],"properties":{"rain":{"type":["number","null"],"minimum":0}}}}});
        let text = schema.to_string();
        let text_digest = crate::canonical::hash_bytes(text.as_bytes());
        let source = json!({"id":"decl-source","service":"sample","revision":"r1","file":"events.json","startLine":1,"endLine":1,"text":text,"textDigest":text_digest,"evidenceDigest":"retained","authority":"EXACT_SNAPSHOT_TEXT"});
        checked.services.insert("sample".into(),serde_json::from_value(json!({"schema":"codeclew-documentation-service-evidence/1.0","service":"sample","revision":"r1","serviceDigest":"definition","extractor":"source-syntax","runtimeMode":"COMMITTED_SOURCE_NO_BUILD","coverage":"SYNTAX","boundaries":[],"entrypoints":[],"observations":{},"sources":{"decl-source":source},"contracts":{}})).unwrap());
        let mut page: Value = serde_json::from_slice(&files["scenarios/example.json"]).unwrap();
        page["operations"][0]["interfaceContracts"] = json!([{"id":"checks","rows":[]}]);
        files.insert(
            "scenarios/example.json".into(),
            serde_json::to_vec(&page).unwrap(),
        );
        plan["models"] = json!([]);
        plan["processViews"] = json!([]);
        plan["contracts"] = json!([{"id":"event","title":"Event declaration","service":"sample","sourceId":"decl-source","textDigest":text_digest,"branchPointer":"/$defs/Event"}]);
        plan["interfaceBindings"] = json!([{"contract":"event","subject":"scenario:example","operation":"saved","interfaceContract":"checks"}]);
        let model = read(&plan, &files, &guides, &checked).unwrap();
        assert_eq!(model["models"], json!([]));
        assert_eq!(
            model["contracts"][0]["declaration"]["nodes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let mut payload = page.clone();
        attach_process(Some(&model), "scenario:example", &mut payload).unwrap();
        assert_eq!(payload["operations"], page["operations"]);
        assert_eq!(
            payload["interfaceContractDeclarations"]["saved"]["checks"]["declaration"]["nodes"][1]
                ["nullability"],
            "Allowed"
        );
        assert!(payload.get("processDataFlow").is_none());
        let mut empty = plan.clone();
        empty["contracts"] = json!([]);
        assert!(read(&empty, &files, &guides, &checked).is_err());
        plan["contracts"][0]["textDigest"] = json!("sha256:wrong");
        assert!(read(&plan, &files, &guides, &checked).is_err());
    }

    #[test]
    fn inline_binding_requires_an_exact_card_and_never_rewrites_the_operation() {
        let (_, mut files, _, _) = fixture();
        let mut page: Value = serde_json::from_slice(&files["scenarios/example.json"]).unwrap();
        page["operations"][0]["interfaceContracts"] = json!([{"id":"checks","rows":[{"value":"Original validator wording","sourceIds":["source"]}]}]);
        files.insert(
            "scenarios/example.json".into(),
            serde_json::to_vec(&page).unwrap(),
        );
        let binding = || InterfaceBinding {
            contract: "declared".into(),
            subject: "scenario:example".into(),
            operation: "saved".into(),
            interface_contract: "checks".into(),
        };
        let contracts = vec![json!({"id":"declared"})];
        let before = files.clone();
        validate_interface_bindings(&[binding()], &contracts, &files).unwrap();
        assert_eq!(files, before);
        assert!(validate_interface_bindings(&[binding(), binding()], &contracts, &files).is_err());
        let mut wrong = binding();
        wrong.interface_contract = "missing".into();
        assert!(validate_interface_bindings(&[wrong], &contracts, &files).is_err());
        assert!(validate_interface_bindings(&[binding()], &[], &files).is_err());
        let model = json!({"models":[],"contracts":[{"id":"declared","source":{"text":"Exact declared schema"}}],"interfaceBindings":[{"contract":"declared","subject":"scenario:example","operation":"saved","interfaceContract":"checks"}],"authority":"AGENT_PROPOSAL","review":"UNASSESSED"});
        let mut payload = page.clone();
        attach_process(Some(&model), "scenario:example", &mut payload).unwrap();
        assert_eq!(payload["operations"], page["operations"]);
        assert_eq!(
            payload["interfaceContractDeclarations"]["saved"]["checks"]["binding"]["authority"],
            "AGENT_PROPOSAL"
        );
        let mut other = page.clone();
        attach_process(Some(&model), "scenario:other", &mut other).unwrap();
        assert!(other.get("interfaceContractDeclarations").is_none());
    }
    #[test]
    fn authored_field_names_keep_comments_and_bind_their_own_evidence() {
        let (mut plan, files, guides, check) = fixture();
        let baseline = read(&plan, &files, &guides, &check).unwrap();
        let mut name = plan["models"][0]["fields"][0]["meaning"].clone();
        name["title"] = json!("Authored readable field name");
        name["text"] = json!("Stored display value");
        plan["models"][0]["fields"][0]["readableName"] = name;
        let mut original = plan["models"][0]["fields"][0]["meaning"].clone();
        original["title"] = json!("Authored original field binding");
        original["text"] = json!("row.value");
        plan["models"][0]["fields"][0]["originalName"] = original;
        let value = read(&plan, &files, &guides, &check).unwrap();
        let field = &value["models"][0]["fields"][0];
        assert_eq!(
            field["meaning"],
            baseline["models"][0]["fields"][0]["meaning"]
        );
        assert_eq!(field["path"], "value");
        assert_eq!(
            field["readableName"]["evidence"],
            field["meaning"]["evidence"]
        );
        for portable in [false, true] {
            let table = field_table(&value["models"][0], "en", portable);
            for text in [
                "Readable name",
                "Comment",
                "Stored display value",
                "Proposed field meaning",
                "#field-value",
            ] {
                assert!(table.contains(text), "{text}");
            }
            assert!(
                !table.contains("row.value"),
                "code representation belongs in field details"
            );
            assert_eq!(field["originalName"]["text"], "row.value");
        }
        plan["models"][0]["fields"][0]["readableName"]["evidence"][0]["textDigest"] =
            json!("sha256:changed");
        assert!(read(&plan, &files, &guides, &check).is_err());
        plan["models"][0]["fields"][0]["readableName"]["text"] = json!("two\nlines");
        assert!(read(&plan, &files, &guides, &check).is_err());
    }
    #[test]
    fn field_names_are_not_inferred_from_generic_claim_titles() {
        let model = json!({"fields":[{"id":"id","path":"id","meaning":{"title":"Source interpretation","text":"Exact complete original comment"}}]});
        let table = field_table(&model, "en", false);
        assert!(table.contains("Source paths are shown where readable names are not provided."));
        assert!(table.contains("<code>id</code>"));
        assert!(table.contains("Exact complete original comment"));
        assert!(!table.contains("Source interpretation"));
        assert!(field_table(&model, "ru", false).contains(
            "Если понятное название не задано, показан точный путь поля в исходном коде."
        ));
    }
    #[test]
    fn legacy_field_links_keep_distinct_paths_without_inventing_business_names() {
        let model = json!({"fields":[
            {"id":"current","path":"dto.currentValue","meaning":{"text":"Current retained value"}},
            {"id":"prior","path":"dto.priorValue","meaning":{"text":"Earlier retained value"}},
            {"id":"named","path":"dto.technicalName","readableName":{"text":"Display value"},"meaning":{"text":"Named retained value"}}
        ]});
        let original = model.clone();
        for portable in [false, true] {
            let table = field_table(&model, "en", portable);
            for (id, path) in [("current", "dto.currentValue"), ("prior", "dto.priorValue")] {
                assert!(table.contains(&format!(
                    "href=\"#field-{id}\" data-clew-field-details=\"\"><code>{path}</code>"
                )));
            }
            assert!(table.contains(">Display value</a>"));
            assert!(!table.contains("dto.technicalName"));
            assert_eq!(
                table
                    .matches("Source paths are shown where readable names are not provided.")
                    .count(),
                1
            );
            assert!(!table.contains("Readable name not provided</a>"));
        }
        assert_eq!(model, original);
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
    fn selected_model_language_keeps_readable_originals_fields_and_exact_records() {
        let (mut plan, _, guides, check) = fixture();
        plan["models"][0]["title"] = json!("Data model");
        plan["models"][0]["fields"][0]["meaning"]["text"] = json!("Source and scope");
        plan["models"][0]["purpose"]["text"] = json!("View evidence");
        for target in ["en", "ru"] {
            for available in [Some("en"), Some("ru"), None] {
                let (_, mut files, _, _) = fixture();
                if let Some(language) = available {
                    plan["models"][0]["documentationLanguage"] = json!(language);
                } else {
                    plan["models"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("documentationLanguage");
                }
                let model = read(&plan, &files, &guides, &check).unwrap();
                let original = model.clone();
                let gaps = write_language(&model, &mut files, Some(target))
                    .unwrap()
                    .models;
                assert_eq!(model, original);
                let html = String::from_utf8(files["data-model/record.html"].clone()).unwrap();
                let index = String::from_utf8(files["data-model.html"].clone()).unwrap();
                for body in [&html, &index] {
                    assert!(body.contains(&format!("<html lang=\"{target}\"")));
                    assert!(body.contains("View evidence") && body.contains("Data model"));
                    assert_eq!(
                        body.contains("translation-gap gap-card"),
                        available != Some(target)
                    );
                }
                assert!(
                    html.contains("<h1>Data model</h1>"),
                    "authored title matches an owned label but remains exact"
                );
                assert!(
                    html.contains("data-clew-markdown>Source and scope</div>"),
                    "authored field meaning remains exact"
                );
                if target == "ru" {
                    assert!(html.contains("<h2>Идентичность и связи</h2>"));
                    assert!(html.contains(
                        "<h2 id=\"fields-and-representations\">Поля и представления</h2>"
                    ));
                    assert!(index.contains("<h1>Модель данных</h1>"));
                }
                assert!(html.contains("id=\"field-value\""));
                assert!(html.contains("id=\"field-usage\""));
                assert!(html.contains("../scenarios/example.html#question-apply"));
                let card = index.split("</article>").next().unwrap();
                assert_eq!(
                    card.contains("translation-gap gap-card"),
                    available != Some(target),
                    "availability notice belongs to its model card"
                );
                let retained: Value =
                    serde_json::from_slice(&files["data-model/record.original.json"]).unwrap();
                assert_eq!(retained, model["models"][0]);
                assert_eq!(gaps.is_empty(), available == Some(target));
                if let Some(gap) = gaps.get("record") {
                    assert_eq!(gap["availableLanguage"], json!(available));
                    assert_eq!(gap["kind"], "MODEL_TRANSLATION");
                    assert!(html.contains("href=\"record.original.json\""));
                    assert!(index.contains("href=\"data-model/record.original.json\""));
                }
            }
        }
    }

    #[test]
    fn declared_contract_availability_preserves_exact_schema_and_original_record() {
        for target in ["en", "ru"] {
            for available in [Some("en"), Some("ru"), None] {
                let contract = json!({"id":"event","title":"Properties","documentationLanguage":available,
                    "branchPointer":"/components/schemas/Event","requiredCondition":CONTRACT_REQUIRED_CONDITION,
                    "leaves":[{"field":"id","requiredWhenBranchMatches":true,"nullable":false,"schema":{"type":"string","enum":["Properties"],"default":"Exact pointer"},"pointer":"/components/schemas/Event/properties/id"}],
                    "source":{"text":"Original contract bytes","textDigest":"unchanged","revision":"r1","file":"event.json","startLine":1,"endLine":1}});
                let model = json!({"models":[],"contracts":[contract]});
                let before = model.clone();
                let mut files = BTreeMap::new();
                let availability = write_language(&model, &mut files, Some(target)).unwrap();
                assert!(availability.models.is_empty());
                assert_eq!(availability.contracts.is_empty(), available == Some(target));
                let html = String::from_utf8(files["data-model.html"].clone()).unwrap();
                assert!(html.contains("<summary>Properties · /components/schemas/Event</summary>"));
                assert!(html.contains("/components/schemas/Event/properties/id"));
                assert!(html.contains("Properties") && html.contains("Exact pointer"));
                assert_eq!(
                    html.contains("translation-gap gap-card"),
                    available != Some(target)
                );
                if available != Some(target) {
                    assert!(html.contains("href=\"contracts/event.original.json\""));
                    assert!(
                        html.find("translation-gap gap-card").unwrap()
                            < html.find("<table>").unwrap()
                    );
                }
                let original: Value =
                    serde_json::from_slice(&files["contracts/event.original.json"]).unwrap();
                assert_eq!(original, contract);
                let destination = String::from_utf8(files["contracts/event.html"].clone()).unwrap();
                assert!(
                    destination.contains("id=\"dm-return\"")
                        && destination.contains("Complete original source binding")
                        || target == "ru"
                );
                assert!(
                    destination.contains("/components/schemas/Event")
                        && destination.contains("event.original.json")
                );
                assert_eq!(model, before);
                if target == "ru" {
                    assert!(html.contains("Совпадение выбранной ветви контракта"));
                }
            }
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
        assert!(html.contains("<summary>Source and scope</summary><div class=\"clew-markdown\" data-clew-markdown>Proposed field meaning</div>"));
        assert!(html.contains("id=\"dm-return\""));
    }

    #[test]
    fn field_inventory_keeps_a_compact_purpose_overview_and_links_to_exact_details() {
        let model = json!({"fields":[
            {"id":"value","path":"payload.value","readableName":{"text":"Stored value"},"meaning":{"text":"Keep <source> {value} and | punctuation"},"representations":[
                {"label":"Request","typeLabel":"string","requiredness":"optional","nullability":"nullable"},
                {"label":"Storage","typeLabel":"int64","requiredness":"required","nullability":"not nullable"}
            ]},
            {"id":"unknown","path":"unknown","meaning":{"text":"No declared representation"},"representations":[]}
        ]});
        let before = model.clone();
        for portable in [false, true] {
            let html = field_table(&model, "en", portable);
            assert!(html.contains("<caption>Field descriptions</caption>"));
            assert_eq!(html.matches("scope=\"row\"").count(), 2);
            assert_eq!(html.matches("scope=\"col\"").count(), 2);
            assert_eq!(
                html.matches("<a href=").count(),
                2,
                "each field has one evidence entry point"
            );
            assert!(
                html.contains("href=\"#field-value\" data-clew-field-details=\"\">Stored value")
            );
            assert!(!html.contains("payload.value"));
            if portable {
                assert!(html.contains(
                    r#"<ClewMarkdown text={"Keep \u003csource\u003e {value} and | punctuation"} />"#
                ));
            } else {
                assert!(html.contains("Keep &lt;source&gt; &#123;value&#125; and | punctuation"));
                assert!(!html.contains("<source>"));
            }
            for technical in ["Request", "Storage", "int64", "optional", "nullable"] {
                assert!(
                    !html.contains(technical),
                    "technical representation leaked into overview: {technical}"
                );
            }
            assert_eq!(model, before);
        }
        let russian = field_table(&model, "ru", false);
        assert!(russian.contains("Описание полей") && russian.contains("Понятное название"));
        assert!(russian.contains("No declared representation"));
    }

    #[test]
    fn service_field_links_require_an_explicit_validated_model_link() {
        let (mut plan, mut files, guides, check) = fixture();
        files.insert(
            "services/sample.json".into(),
            serde_json::to_vec(&json!({
                "operations":[{"id":"section-entities"}]
            }))
            .unwrap(),
        );
        plan["models"][0]["processLinks"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "subject":"service:sample","anchor":"section-entities","label":"Sample data"
            }));
        let model = read(&plan, &files, &guides, &check).unwrap();
        let mut service = json!({"entities":[],"original":"unchanged"});
        attach_process(Some(&model), "service:sample", &mut service).unwrap();
        assert_eq!(service["serviceModelLinks"][0]["id"], "record");
        assert_eq!(
            service["serviceModelLinks"][0]["href"],
            "../data-model/record.html#fields-and-representations"
        );
        assert_eq!(
            service["entities"],
            json!([]),
            "model navigation does not assert domain ownership"
        );
        let table = service["serviceModelLinks"][0]["fieldTableHtml"]
            .as_str()
            .unwrap();
        assert!(table.contains("<table class=\"dm-fields\""));
        assert!(table.contains("Proposed field meaning"));
        assert!(table.contains("../data-model/record.html#field-value"));
        assert!(!table.contains("href=\"#field-value\""));
        assert_eq!(
            service["serviceModelLinks"][0]["fieldSummary"]["fields"][0]["meaning"]["text"],
            model["models"][0]["fields"][0]["meaning"]["text"]
        );
        assert_eq!(service["original"], "unchanged");
        let mut other = json!({});
        attach_process(Some(&model), "service:other", &mut other).unwrap();
        assert!(other.get("serviceModelLinks").is_none());
    }
    #[test]
    fn declared_entity_relation_exposes_its_dictionary_only_on_the_exact_service() {
        let (mut plan, files, guides, mut checked) = fixture();
        let entity = json!({"id":"entity:record","kind":"DOMAIN_ENTITY","service":"","symbol":"Record","digest":"fixture","sourceIds":[],"normalized":{"entity":{"id":"record","title":"Record","relations":[{"service":"sample","kind":"read","origin":"agent-proposal","confidence":"uncertain","rationale":"Selected source read only"}]}}});
        checked.dependencies.insert(
            "entity:record".into(),
            serde_json::from_value(entity.clone()).unwrap(),
        );
        plan["models"][0]["entity"] = json!("entity:record");
        plan["models"][0]["kind"] = json!("ENTITY");
        plan["models"][0]["processLinks"] = json!([]);
        let model = read(&plan, &files, &guides, &checked).unwrap();
        let original =
            json!({"language":"en","entities":[entity],"original":"Preserved service description"});
        let mut service = original.clone();
        attach_process(Some(&model), "service:sample", &mut service).unwrap();
        assert_eq!(service["readerModels"][0]["entity"], "entity:record");
        assert_eq!(
            service["readerModels"][0]["fields"],
            model["models"][0]["fields"]
        );
        assert_eq!(
            service["serviceModelLinks"][0]["fieldSummary"]["fields"][0]["meaning"]["text"],
            "Proposed field meaning"
        );
        assert_eq!(
            service["serviceModelLinks"][0]["href"],
            "../data-model/record.html#fields-and-representations"
        );
        assert_eq!(service["entities"], original["entities"]);
        assert_eq!(service["original"], original["original"]);
        assert_eq!(service["readerModels"][0]["processLinks"], json!([]));
        assert!(service.get("processFieldFlow").is_none());
        for subject in ["service:other", "service:sample-extra", "scenario:example"] {
            let mut other = original.clone();
            other["view"] = json!({});
            attach_process(Some(&model), subject, &mut other).unwrap();
            assert!(other.get("readerModels").is_none(), "{subject}");
            assert!(other.get("serviceModelLinks").is_none(), "{subject}");
        }
        // Same names alone do not bind a dictionary, nor does the presence of
        // a domain entity without a retained relation to the selected service.
        let mut unbound = model.clone();
        unbound["models"][0]
            .as_object_mut()
            .unwrap()
            .remove("entity");
        let mut service = original.clone();
        attach_process(Some(&unbound), "service:sample", &mut service).unwrap();
        assert!(service.get("readerModels").is_none());
        let mut service = original.clone();
        service["entities"][0]["normalized"]["entity"]["relations"] = json!([]);
        attach_process(Some(&model), "service:sample", &mut service).unwrap();
        assert!(service.get("readerModels").is_none());
        // An additional explicit navigation link still yields one dictionary.
        let mut linked = model;
        linked["models"][0]["processLinks"] =
            json!([{"subject":"service:sample","anchor":"section-entities"}]);
        let mut service = original;
        attach_process(Some(&linked), "service:sample", &mut service).unwrap();
        assert_eq!(service["readerModels"].as_array().unwrap().len(), 1);
        assert_eq!(service["serviceModelLinks"].as_array().unwrap().len(), 1);
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
    #[test]
    fn payload_views_reuse_exact_fields_and_keep_partial_type_and_wire_constraints_separate() {
        let (mut plan, files, guides, check) = fixture();
        let original_files = files.clone();
        let claim = plan["models"][0]["purpose"].clone();
        plan["payloadViews"] = json!([{"id":"request","title":"Selected request shape","subject":"scenario:example","operation":"saved","component":"request-body","direction":"incoming","format":"json","coverage":{"status":"observed-fields","scope":"Named source property only; wire presence unknown","claim":claim},"fields":[{"model":"record","field":"value","representation":"row","path":"value"}]}]);
        let value = read(&plan, &files, &guides, &check).unwrap();
        let view = &value["payloadViews"][0];
        assert_eq!(view["fields"][0]["typeLabel"], "string");
        assert_eq!(view["wireSchema"], "NOT_ESTABLISHED_BY_TYPE_OR_MAPPING");
        assert_eq!(view["runtime"], "UNVERIFIED");
        assert_eq!(files, original_files);
        let mut attached =
            json!({"operations":[{"id":"saved","summary":{"text":"Retained original"}}]});
        let operations = attached["operations"].clone();
        attach_process(Some(&value), "scenario:example", &mut attached).unwrap();
        assert_eq!(attached["operations"], operations);
        assert_eq!(attached["payloadViews"][0]["id"], "request");
        let mut fuzzy = plan.clone();
        fuzzy["payloadViews"][0]["fields"][0]["representation"] = json!("row.value");
        assert!(read(&fuzzy, &files, &guides, &check).is_err());
        let mut complete = plan.clone();
        complete["payloadViews"][0]["coverage"]["status"] = json!("complete-definition");
        assert!(read(&complete, &files, &guides, &check).is_err());
        let mut duplicate = plan.clone();
        let field = duplicate["payloadViews"][0]["fields"][0].clone();
        duplicate["payloadViews"][0]["fields"]
            .as_array_mut()
            .unwrap()
            .push(field);
        assert!(read(&duplicate, &files, &guides, &check).is_err());
        let mut unknown = plan;
        unknown["payloadViews"][0]["coverage"]["status"] = json!("unknown");
        assert!(read(&unknown, &files, &guides, &check).is_err());
        unknown["payloadViews"][0]["fields"] = json!([]);
        unknown["payloadViews"][0]["component"] = json!("error-response-body");
        unknown["payloadViews"][0]["status"] = json!("502");
        let result = read(&unknown, &files, &guides, &check).unwrap();
        assert_eq!(result["payloadViews"][0]["coverage"]["status"], "unknown");
    }

    #[test]
    fn payload_fields_keep_validated_canonical_name_claims_without_reauthoring() {
        let (mut plan, files, guides, checked) = fixture();
        let mut name = plan["models"][0]["purpose"].clone();
        name["text"] = json!("Measured value");
        plan["models"][0]["fields"][0]["readableName"] = name.clone();
        name["text"] = json!("row.value");
        plan["models"][0]["fields"][0]["originalName"] = name;
        plan["payloadViews"] = json!([{"id":"input","title":"Local input","subject":"scenario:example","operation":"saved","component":"request-body","direction":"local","format":"Source representation","coverage":{"status":"observed-fields","scope":"Selected mapper field","claim":plan["models"][0]["purpose"]},"fields":[{"model":"record","field":"value","representation":"row","path":"request.value"}]}]);
        let original = plan.clone();
        let projected = read(&plan, &files, &guides, &checked).unwrap();
        for key in ["readableName", "originalName"] {
            assert_eq!(
                projected["payloadViews"][0]["fields"][0][key],
                projected["models"][0]["fields"][0][key]
            );
        }
        assert_eq!(
            projected["payloadViews"][0]["fields"][0]["path"],
            "request.value"
        );
        assert_eq!(plan, original);
        plan["models"][0]["fields"][0]["readableName"]["evidence"][0]["textDigest"] =
            json!("changed");
        assert!(read(&plan, &files, &guides, &checked).is_err());
    }

    #[test]
    fn payload_nested_type_preserves_only_an_exact_validated_model_reference() {
        let (mut plan, files, guides, checked) = fixture();
        let mut nested = plan["models"][0].clone();
        nested["id"] = json!("nested");
        nested["title"] = json!("Nested record");
        plan["models"].as_array_mut().unwrap().push(nested);
        plan["models"][0]["fields"][0]["representations"][0]["relatedModel"] = json!("nested");
        plan["payloadViews"] = json!([{"id":"input","title":"Local input","subject":"scenario:example","operation":"saved","component":"request-body","direction":"local","format":"Source representation; wire format unknown","coverage":{"status":"observed-fields","scope":"Selected field only","claim":plan["models"][0]["purpose"]},"fields":[{"model":"record","field":"value","representation":"row","path":"request.record"}]}]);
        let projected = read(&plan, &files, &guides, &checked).unwrap();
        assert_eq!(
            projected["payloadViews"][0]["fields"][0]["relatedModel"],
            "nested"
        );
        plan["models"][0]["fields"][0]["representations"][0]["relatedModel"] =
            json!("nested-extra");
        assert!(read(&plan, &files, &guides, &checked).is_err());
    }

    #[test]
    fn payload_interaction_direction_and_external_links_use_exact_declared_identity() {
        let (mut plan, mut files, guides, mut checked) = fixture();
        files.insert(
            "services/owner.json".into(),
            files["scenarios/example.json"].clone(),
        );
        let tmp = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(tmp.path(), "Payload fixture").unwrap();
        let mut inputs = super::super::store::Repository::open(tmp.path())
            .unwrap()
            .inputs()
            .unwrap();
        inputs.interactions.insert("request-price".into(), serde_json::from_value(json!({"schema":"codeclew-documentation-interaction/1.0","id":"request-price","title":"Request price","from":{"service":"owner"},"to":{"service":"provider"},"external":true,"externalService":"provider","transport":{"kind":"http","method":"POST","path":"/price"},"declaration":{"origin":"human","rationale":"Explicit fixture boundary"}})).unwrap());
        checked.source_inputs = Some(crate::documentation::check::SourceInputs {
            schema: crate::documentation::check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest: "input".into(),
            inputs,
            selected_services: Default::default(),
            retained_services: Default::default(),
        });
        plan["payloadViews"] = json!([{"id":"request","title":"Price request","subject":"service:owner","operation":"saved","interaction":"request-price","component":"request-body","direction":"outgoing","format":"json","coverage":{"status":"observed-fields","scope":"Named source property","claim":plan["models"][0]["purpose"]},"fields":[{"model":"record","field":"value","representation":"row","path":"value"}]}]);
        let projected = read(&plan, &files, &guides, &checked).unwrap();
        let mut response = plan.clone();
        response["payloadViews"][0]["component"] = json!("response-body");
        response["payloadViews"][0]["direction"] = json!("incoming");
        assert!(read(&response, &files, &guides, &checked).is_ok());
        for direction in ["incoming", "local"] {
            let mut invalid = plan.clone();
            invalid["payloadViews"][0]["direction"] = json!(direction);
            assert!(read(&invalid, &files, &guides, &checked).is_err());
        }
        let mut missing = plan.clone();
        missing["payloadViews"][0]["interaction"] = json!("request-price-missing");
        assert!(read(&missing, &files, &guides, &checked).is_err());
        let original_interactions = json!([{"id":"request-price"},{"id":"unrelated"}]);
        files.insert(
            "external-services/provider.json".into(),
            serde_json::to_vec(&json!({"interactions":original_interactions})).unwrap(),
        );
        files.insert(
            "external-services/provider.html".into(),
            b"<html><body>Retained profile</body></html>".to_vec(),
        );
        attach_external_payloads(Some(&projected), &mut files, "en").unwrap();
        let profile: Value =
            serde_json::from_slice(&files["external-services/provider.json"]).unwrap();
        assert_eq!(profile["interactions"], original_interactions);
        assert_eq!(profile["payloadLinks"].as_array().unwrap().len(), 1);
        assert_eq!(
            profile["payloadLinks"][0]["href"],
            "../services/owner.html#cpv-saved-request"
        );
        assert!(
            String::from_utf8_lossy(&files["external-services/provider.html"])
                .contains("../services/owner.html#cpv-saved-request")
        );
    }

    #[test]
    fn payload_declarations_use_one_frozen_recomposition_closure() {
        use crate::documentation::{check, composition, digest, model, store::Repository};

        let (mut plan, mut files, guides, _) = fixture();
        files.insert(
            "services/owner.json".into(),
            files["scenarios/example.json"].clone(),
        );
        files.insert(
            "services/other.json".into(),
            files["scenarios/example.json"].clone(),
        );
        let mut page: Value = serde_json::from_slice(&files["services/owner.json"]).unwrap();
        page["operations"][0]["events"] = json!([]);
        files.insert(
            "services/owner.json".into(),
            serde_json::to_vec(&page).unwrap(),
        );
        plan["payloadViews"] = json!([{"id":"message","title":"Selected outgoing value","subject":"service:owner","operation":"saved","interaction":"publish-value","component":"message-value","direction":"outgoing","format":"source representation","coverage":{"status":"observed-fields","scope":"Selected mapper field only","claim":plan["models"][0]["purpose"]},"fields":[{"model":"record","field":"value","representation":"row","path":"value"}]}]);
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path(), "Frozen payload declarations").unwrap();
        let repo = Repository::open(tmp.path()).unwrap();
        for id in ["owner", "other"] {
            let service: model::Service = serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":id,"title":id,"repositoryId":id,"repository":format!("https://example.invalid/{id}"),"language":"java","profile":"java-17plus-maven-read-only","compilations":[":/main"],"targetRef":"HEAD"})).unwrap();
            repo.service_add(service, Some(&repo.input_digest().unwrap()))
                .unwrap();
        }
        // Save a real immutable Check contract before any boundary or process
        // declaration exists. Source extraction is orthogonal to this resolver.
        let capture = || {
            let inputs = repo.inputs().unwrap();
            let mut checked = check::assemble(
                digest(&inputs).unwrap(),
                BTreeMap::new(),
                BTreeMap::new(),
                &inputs.interactions,
                &inputs.scenarios,
            )
            .unwrap();
            checked.source_inputs = Some(check::SourceInputs {
                schema: check::SOURCE_INPUTS_SCHEMA.into(),
                input_digest: digest(&inputs).unwrap(),
                inputs,
                selected_services: BTreeSet::new(),
                retained_services: BTreeSet::new(),
            });
            checked.save_snapshot(&repo).unwrap()
        };
        let original = capture();
        let interaction = |sender: &str| -> model::Interaction {
            serde_json::from_value(json!({"schema":"codeclew-documentation-interaction/1.0","id":"publish-value","title":"Publish value","from":{"service":sender},"to":{"service":"provider"},"external":true,"transport":{"kind":"kafka","topic":"value.created"},"declaration":{"origin":"human","rationale":"Explicit fixture boundary"}})).unwrap()
        };
        let scenario = |owner: &str| -> model::Scenario {
            serde_json::from_value(json!({"schema":"codeclew-documentation-process/1.0","id":"example","title":"Example","summary":"Selected mapper fixture","root":{"service":owner},"interactions":[],"maxDepth":4,"maxNodes":64,"process":{"scope":"Selected value","participants":[owner],"trigger":"Mapping request","outcomes":["Mapped value"],"linkedSubviews":[]}})).unwrap()
        };
        repo.interaction_put(interaction("owner"), Some(&repo.input_digest().unwrap()))
            .unwrap();
        repo.put(
            "scenarios/example.yaml",
            &scenario("owner"),
            Some(&repo.input_digest().unwrap()),
        )
        .unwrap();
        let (_, composed_handle) = composition::recompose(&repo, &original).unwrap();
        let composed = Check::load_snapshot(&repo, &composed_handle).unwrap();
        assert!(
            composed
                .source_inputs
                .as_ref()
                .unwrap()
                .inputs
                .interactions
                .is_empty()
        );
        let projected = read(&plan, &files, &guides, &composed).unwrap();
        assert_eq!(projected["payloadViews"][0]["interaction"], "publish-value");
        attach_process(Some(&projected), "service:owner", &mut page).unwrap();
        assert_eq!(page["operations"][0]["events"], json!([]));
        let mut scoped = plan.clone();
        scoped["payloadViews"][0]["subject"] = json!("scenario:example");
        assert!(read(&scoped, &files, &guides, &composed).is_ok());
        for (field, value) in [
            ("direction", "incoming"),
            ("direction", "local"),
            ("subject", "service:other"),
            ("interaction", "publish-value-extra"),
        ] {
            let mut bad = plan.clone();
            bad["payloadViews"][0][field] = json!(value);
            assert!(
                read(&bad, &files, &guides, &composed).is_err(),
                "{field}={value}"
            );
        }
        let mut response = plan.clone();
        response["payloadViews"][0]["component"] = json!("response-body");
        response["payloadViews"][0]["direction"] = json!("incoming");
        assert!(read(&response, &files, &guides, &composed).is_ok());

        // Neither an old capture nor a saved composition follows live edits.
        assert!(
            read(
                &plan,
                &files,
                &guides,
                &Check::load_snapshot(&repo, &original).unwrap()
            )
            .is_err()
        );
        let with_declarations = capture();
        assert!(
            read(
                &plan,
                &files,
                &guides,
                &Check::load_snapshot(&repo, &with_declarations).unwrap()
            )
            .is_ok()
        );
        repo.interaction_put(interaction("other"), Some(&repo.input_digest().unwrap()))
            .unwrap();
        repo.put(
            "scenarios/example.yaml",
            &scenario("other"),
            Some(&repo.input_digest().unwrap()),
        )
        .unwrap();
        let frozen = Check::load_snapshot(&repo, &composed_handle).unwrap();
        assert!(read(&scoped, &files, &guides, &frozen).is_ok());
        let (changed, _) = composition::recompose(&repo, &original).unwrap();
        assert!(read(&plan, &files, &guides, &changed).is_err());
        assert!(read(&scoped, &files, &guides, &changed).is_ok());

        // A missing record in a composition must not resurrect the original
        // source contract's declaration (or a later live replacement).
        repo.interaction_remove("publish-value", &repo.input_digest().unwrap())
            .unwrap();
        let (_, removed_handle) = composition::recompose(&repo, &with_declarations).unwrap();
        repo.interaction_put(interaction("owner"), Some(&repo.input_digest().unwrap()))
            .unwrap();
        let removed = Check::load_snapshot(&repo, &removed_handle).unwrap();
        assert!(
            read(&plan, &files, &guides, &removed)
                .unwrap_err()
                .message
                .contains("payload interaction is unavailable")
        );
        std::fs::remove_file(repo.path("scenarios/example.yaml").unwrap()).unwrap();
        let (_, removed_scenario_handle) =
            composition::recompose(&repo, &with_declarations).unwrap();
        repo.put(
            "scenarios/example.yaml",
            &scenario("owner"),
            Some(&repo.input_digest().unwrap()),
        )
        .unwrap();
        let removed_scenario = Check::load_snapshot(&repo, &removed_scenario_handle).unwrap();
        assert!(
            read(&scoped, &files, &guides, &removed_scenario)
                .unwrap_err()
                .message
                .contains("payload scenario is unavailable")
        );
    }

    #[test]
    fn source_message_payload_can_belong_to_retained_egress_catalogue_without_a_trace() {
        let (mut plan, _, _, mut checked) = fixture();
        let statement = "Mapper supplies Mapped.value to the publisher; the imported Avro definition is unavailable.";
        let source_text = "data class Mapped(val value: String)\nfun publish(input: Mapped) { sender.send(GeneratedValue(input.value)) }";
        let source = json!({"id":"mapper-source","service":"owner","file":"Mapper.kt","revision":"retained","startLine":1,"endLine":2,"text":source_text,"textDigest":crate::canonical::hash_bytes(source_text.as_bytes()),"evidenceDigest":"retained","authority":"EXACT_SNAPSHOT_TEXT"});
        let operation = json!({"id":"section-egress","title":"Outgoing messages","events":[],"summary":{"id":"mapper-claim","text":statement,"sourceIds":["mapper-source"]}});
        let mut page = json!({"subject":"service:owner","operations":[operation],"operationSources":{"section-egress":{"mapper-source":source}},"operationStates":{"section-egress":{"freshness":"CURRENT","verification":"UNASSESSED"}}});
        let claim = json!({"title":"Selected mapper evidence","text":statement,"evidence":[{"kind":"RETAINED","subject":"service:owner","operation":"section-egress","fragmentId":"mapper-claim","textDigest":crate::canonical::hash_bytes(statement.as_bytes())}]});
        let model = &mut plan["models"][0];
        model["purpose"] = claim.clone();
        model["identity"] = claim.clone();
        model["processLinks"] = json!([]);
        model["fields"][0]["meaning"] = claim.clone();
        model["fields"][0]["usage"] = json!([]);
        model["fields"][0]["representations"] = json!([{"label":"Mapped.value","typeLabel":"String","requiredness":"Wire presence unknown","nullability":"Non-null source property; wire nullability unknown","mapping":claim}]);
        plan["processViews"] = json!([]);
        plan["payloadViews"] = json!([{"id":"observation-value","title":"Published observation value","subject":"service:owner","operation":"section-egress","interaction":"publish-observation","component":"message-value","direction":"outgoing","format":"avro","coverage":{"status":"observed-fields","scope":"Mapped.value only; imported Avro definition unavailable","claim":claim},"fields":[{"model":"record","field":"value","representation":"Mapped.value","path":"input.value (mapper access)"}]}]);
        let tmp = tempfile::tempdir().unwrap();
        crate::documentation::store::Repository::init(tmp.path(), "Catalogue payload fixture")
            .unwrap();
        let mut inputs = crate::documentation::store::Repository::open(tmp.path())
            .unwrap()
            .inputs()
            .unwrap();
        inputs.interactions.insert("publish-observation".into(),serde_json::from_value(json!({"schema":"codeclew-documentation-interaction/1.0","id":"publish-observation","title":"Publish observation","from":{"service":"owner"},"to":{"service":"provider"},"external":true,"externalService":"provider","transport":{"kind":"kafka","topic":"observation.created"},"declaration":{"origin":"human","rationale":"Exact outgoing fixture interaction"}})).unwrap());
        checked.source_inputs = Some(crate::documentation::check::SourceInputs {
            schema: crate::documentation::check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest: "input".into(),
            inputs,
            selected_services: Default::default(),
            retained_services: Default::default(),
        });
        let mut files = BTreeMap::from([(
            "services/owner.json".into(),
            serde_json::to_vec(&page).unwrap(),
        )]);
        let projected = read(&plan, &files, &BTreeMap::new(), &checked).unwrap();
        let original = page["operations"].clone();
        attach_process(Some(&projected), "service:owner", &mut page).unwrap();
        assert_eq!(page["operations"], original);
        assert_eq!(page["operations"][0]["events"], json!([]));
        assert_eq!(
            page["payloadViews"][0]["coverage"]["status"],
            "observed-fields"
        );
        assert_eq!(
            page["payloadViews"][0]["wireSchema"],
            "NOT_ESTABLISHED_BY_TYPE_OR_MAPPING"
        );
        files.insert(
            "external-services/provider.json".into(),
            serde_json::to_vec(&json!({"interactions":[{"id":"publish-observation"}]})).unwrap(),
        );
        attach_external_payloads(Some(&projected), &mut files, "en").unwrap();
        let profile: Value =
            serde_json::from_slice(&files["external-services/provider.json"]).unwrap();
        assert_eq!(
            profile["payloadLinks"][0]["href"],
            "../services/owner.html#cpv-section-egress-observation-value"
        );
        let mut unknown = plan.clone();
        unknown["payloadViews"][0]["coverage"]["status"] = json!("unknown");
        unknown["payloadViews"][0]["fields"] = json!([]);
        assert!(read(&unknown, &files, &BTreeMap::new(), &checked).is_ok());
        let mut absent = plan;
        absent["payloadViews"][0]["operation"] = json!("section-missing");
        assert!(read(&absent, &files, &BTreeMap::new(), &checked).is_err());
    }
}
