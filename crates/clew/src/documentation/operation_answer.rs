//! Source-bound validation for a structured bounded question answer.
//!
//! This validates packet binding, evidence labels, and tree shape. It does not
//! review semantic correctness or publish documentation.

use super::{digest, invalid};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const ANSWER_SCHEMA_V1_0: &str = "codeclew-operation-answer/1.0";
const ANSWER_SCHEMA_V1_1: &str = "codeclew-operation-answer/1.1";
const ANSWER_SCHEMA_V1_2: &str = "codeclew-operation-answer/1.2";
const PACKET_SCHEMA: &str = "codeclew-documentation-reader-packet/1.0";
const STEP_KINDS: &[&str] = &["action", "decision", "try", "return", "throw", "loop"];
// Bump when the answer schema or generic author instruction changes materially;
// this identity is persisted on newly prepared operation Work.
pub(super) const AUTHORING_CONTRACT: &str = "codeclew-operation-draft-authoring/1.4";
pub(super) const PREVIOUS_AUTHORING_CONTRACT: &str = "codeclew-operation-draft-authoring/1.3";
// A separate policy for bounded source-data questions; generic 1.4 stays immutable.
pub(super) const QUESTION_AUTHORING_CONTRACT: &str = "codeclew-operation-draft-authoring/1.5";
pub(super) const EXPANDING_AUTHORING_CONTRACT: &str = "codeclew-operation-draft-authoring/1.6";

pub(super) fn expanding_authoring_eligible(request: &super::work::Request) -> bool {
    request.context_profile.as_deref() == Some(super::endpoint_context::PROFILE)
        || (request.context_profile.as_deref() == Some("process-graph-v1")
            && request
                .question
                .as_deref()
                .is_some_and(|question| !question.trim().is_empty()))
}

pub(super) fn question_authoring_eligible(request: &super::work::Request) -> bool {
    request.context_profile.as_deref() == Some("process-graph-v1")
        && request.source_data_context
        && request
            .question
            .as_deref()
            .is_some_and(|question| !question.trim().is_empty())
}

pub(super) fn validate_authoring_request(
    request: &super::work::Request,
) -> Result<(), crate::error::ClewError> {
    if request.authoring_contract.as_deref() == Some(EXPANDING_AUTHORING_CONTRACT)
        && !expanding_authoring_eligible(request)
    {
        return Err(invalid(
            "OPERATION_AUTHORING_CONTRACT_PROFILE_MISMATCH: authoring contract 1.6 requires endpoint-context-v3 or process-graph-v1 with a non-empty question",
        ));
    }
    if request.authoring_contract.as_deref() == Some(QUESTION_AUTHORING_CONTRACT)
        && !question_authoring_eligible(request)
    {
        return Err(invalid(
            "OPERATION_AUTHORING_CONTRACT_PROFILE_MISMATCH: authoring contract 1.5 requires process-graph-v1, sourceDataContext and a non-empty question",
        ));
    }
    Ok(())
}

pub(super) fn output_schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../../schemas/documentation/operation-answer-1.2.schema.json"
    ))
    .expect("operation answer schema is valid JSON")
}

pub(super) fn validate_draft(
    packet: &Value,
    audit: &Value,
    answer: Value,
) -> Result<ValidatedAnswer, crate::error::ClewError> {
    if answer["schema"].as_str() != Some(ANSWER_SCHEMA_V1_2) {
        return Err(invalid(
            "new operation drafts require codeclew-operation-answer/1.2; the raw author response was retained",
        ));
    }
    validate(packet, audit, answer)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationAnswer {
    schema: String,
    packet_digest: String,
    title: String,
    summary: Claim,
    steps: Vec<OperationStep>,
    #[serde(default)]
    glossary: Vec<GlossaryTerm>,
    #[serde(default)]
    predicates: Vec<Predicate>,
    #[serde(default)]
    preparations: Vec<Preparation>,
    uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Claim {
    text: String,
    evidence: Vec<String>,
    #[serde(default)]
    checks: Vec<super::proposals::Assertion>,
    #[serde(default)]
    uncertainty: Option<String>,
    #[serde(default)]
    glossary_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GlossaryKind {
    BusinessEntity,
    Request,
    TechnicalCarrier,
    Term,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GlossaryTerm {
    id: String,
    label: String,
    // The original JSON retains this enum; this type validates its closed shape.
    #[serde(rename = "kind")]
    _kind: GlossaryKind,
    definition: Claim,
    #[serde(default)]
    subject_refs: Vec<String>,
    #[serde(default)]
    technical_names: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Predicate {
    id: String,
    label: String,
    meaning: Claim,
    source_check: Claim,
    evaluation: Claim,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationStep {
    #[serde(default)]
    id: Option<String>,
    kind: String,
    meaning: Claim,
    #[serde(default)]
    predicate_ref: Option<String>,
    #[serde(default)]
    glossary_refs: Vec<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    interaction: Option<String>,
    #[serde(default)]
    preparation_refs: Vec<String>,
    #[serde(default)]
    children: Vec<OperationStep>,
    #[serde(default)]
    otherwise: Vec<OperationStep>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preparation {
    id: String,
    title: String,
    #[serde(default)]
    subject_reference: Option<String>,
    summary: Claim,
    #[serde(default)]
    steps: Vec<OperationStep>,
}

pub(super) struct ValidatedAnswer {
    pub(super) answer: Value,
}

/// Validate the exact packet binding, evidence and answer shape. No document output.
pub(super) fn validate(
    packet: &Value,
    audit: &Value,
    answer: Value,
) -> Result<ValidatedAnswer, crate::error::ClewError> {
    validate_bound_answer(packet, audit, answer)
}

fn validate_bound_answer(
    packet: &Value,
    audit: &Value,
    answer: Value,
) -> Result<ValidatedAnswer, crate::error::ClewError> {
    if packet["schema"] != PACKET_SCHEMA
        || !matches!(
            packet["profile"].as_str(),
            Some("endpoint-context-v3" | "process-graph-v1")
        )
    {
        return Err(invalid(
            "operation answer requires a supported endpoint or internal process reader packet",
        ));
    }
    let answer_schema = validate_answer_version(&answer)?;
    let citations = packet["citations"]
        .as_object()
        .ok_or_else(|| invalid("reader packet has no citation-label map"))?;
    let known_labels: BTreeSet<_> = citations.keys().cloned().collect();
    if packet_displayed_citation_labels(packet, audit)? != known_labels {
        return Err(invalid(
            "reader packet citation labels do not match its displayed evidence labels",
        ));
    }

    let parsed: OperationAnswer = serde_json::from_value(answer.clone())
        .map_err(|_| invalid("operation answer has an invalid schema or shape"))?;
    if parsed.schema != answer_schema {
        return Err(invalid("unsupported operation-answer schema"));
    }
    let declared_packet_digest = packet["packetDigest"]
        .as_str()
        .ok_or_else(|| invalid("reader packet has no packetDigest"))?;
    let mut packet_without_digest = packet.clone();
    packet_without_digest
        .as_object_mut()
        .ok_or_else(|| invalid("reader packet must be a JSON object"))?
        .remove("packetDigest");
    let actual_packet_digest = digest(&packet_without_digest)?;
    if declared_packet_digest != actual_packet_digest
        || parsed.packet_digest != actual_packet_digest
    {
        return Err(invalid(
            "operation answer packetDigest does not match the compact reader packet",
        ));
    }
    if parsed.title.trim().is_empty() {
        return Err(invalid("operation answer title must not be empty"));
    }
    validate_claim(&parsed.summary, &known_labels, "summary")?;
    if parsed.steps.is_empty() {
        return Err(invalid("operation answer must contain at least one step"));
    }
    validate_steps(&parsed.steps, &known_labels)?;
    if matches!(
        parsed.schema.as_str(),
        ANSWER_SCHEMA_V1_1 | ANSWER_SCHEMA_V1_2
    ) {
        validate_preparations(&parsed, packet, &known_labels)?;
    }
    if parsed.schema == ANSWER_SCHEMA_V1_2 {
        validate_semantic_contract(&parsed, packet, &known_labels)?;
    }
    if parsed
        .uncertainties
        .iter()
        .any(|item| item.trim().is_empty())
    {
        return Err(invalid("operation answer uncertainties must not be empty"));
    }

    Ok(ValidatedAnswer { answer })
}

fn validate_answer_version(answer: &Value) -> Result<&str, crate::error::ClewError> {
    let schema = answer["schema"]
        .as_str()
        .ok_or_else(|| invalid("operation answer has no schema"))?;
    match schema {
        ANSWER_SCHEMA_V1_0 => {
            if answer.get("preparations").is_some()
                || step_tree_has_preparation_refs(&answer["steps"])
            {
                return Err(invalid(
                    "operation-answer/1.0 cannot contain preparations or preparationRefs",
                ));
            }
            if has_legacy_semantic_fields(answer) {
                return Err(invalid(
                    "operation-answer/1.0 cannot contain operation-answer/1.2 semantic fields",
                ));
            }
        }
        ANSWER_SCHEMA_V1_1 => {
            if !answer["preparations"].is_array() {
                return Err(invalid(
                    "operation-answer/1.1 requires a preparations array, which may be empty",
                ));
            }
            if has_legacy_semantic_fields(answer) {
                return Err(invalid(
                    "operation-answer/1.1 cannot contain operation-answer/1.2 semantic fields",
                ));
            }
        }
        ANSWER_SCHEMA_V1_2 => {
            for field in ["preparations", "glossary", "predicates"] {
                if !answer[field].is_array() {
                    return Err(invalid(format!(
                        "operation-answer/1.2 requires a {field} array, which may be empty"
                    )));
                }
            }
            if !claim_has_glossary_refs(&answer["summary"])
                || !steps_have_glossary_refs(&answer["steps"])
                || !predicate_refs_only_on_decisions(&answer["steps"])
                || answer["glossary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|term| !claim_has_glossary_refs(&term["definition"]))
                || answer["predicates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|predicate| {
                        ["meaning", "sourceCheck", "evaluation"]
                            .into_iter()
                            .any(|field| !claim_has_glossary_refs(&predicate[field]))
                    })
                || answer["preparations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|preparation| {
                        !preparation["steps"].is_array()
                            || !claim_has_glossary_refs(&preparation["summary"])
                            || !steps_have_glossary_refs(&preparation["steps"])
                            || !predicate_refs_only_on_decisions(&preparation["steps"])
                    })
            {
                return Err(invalid(
                    "operation-answer/1.2 requires an explicit glossaryRefs array on every claim and step",
                ));
            }
        }
        _ => return Err(invalid("unsupported operation-answer schema")),
    }
    Ok(schema)
}

fn claim_has_glossary_refs(value: &Value) -> bool {
    value["glossaryRefs"].as_array().is_some()
}

fn steps_have_glossary_refs(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(steps) => steps.iter().all(|step| {
            step["glossaryRefs"].as_array().is_some()
                && claim_has_glossary_refs(&step["meaning"])
                && steps_have_glossary_refs(&step["children"])
                && steps_have_glossary_refs(&step["otherwise"])
        }),
        _ => false,
    }
}

fn predicate_refs_only_on_decisions(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(steps) => steps.iter().all(predicate_refs_only_on_decisions),
        Value::Object(step) => {
            (!step.contains_key("predicateRef") || step["kind"] == "decision")
                && step
                    .get("children")
                    .is_none_or(predicate_refs_only_on_decisions)
                && step
                    .get("otherwise")
                    .is_none_or(predicate_refs_only_on_decisions)
        }
        _ => false,
    }
}

fn has_legacy_semantic_fields(answer: &Value) -> bool {
    fn contains_key(value: &Value, key: &str) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key(key) || object.values().any(|value| contains_key(value, key))
            }
            Value::Array(values) => values.iter().any(|value| contains_key(value, key)),
            _ => false,
        }
    }

    fn steps_have_ids(value: &Value) -> bool {
        match value {
            Value::Array(steps) => steps.iter().any(steps_have_ids),
            Value::Object(step) => {
                step.contains_key("id")
                    || step.get("children").is_some_and(steps_have_ids)
                    || step.get("otherwise").is_some_and(steps_have_ids)
            }
            _ => false,
        }
    }

    answer["glossary"]
        .as_array()
        .is_some_and(|terms| !terms.is_empty())
        || answer["predicates"]
            .as_array()
            .is_some_and(|predicates| !predicates.is_empty())
        || contains_key(answer, "predicateRef")
        || contains_key(answer, "glossaryRefs")
        || steps_have_ids(&answer["steps"])
        || answer["preparations"]
            .as_array()
            .is_some_and(|preparations| {
                preparations
                    .iter()
                    .any(|prep| steps_have_ids(&prep["steps"]))
            })
}

fn step_tree_has_preparation_refs(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(step_tree_has_preparation_refs),
        Value::Object(fields) => {
            fields.contains_key("preparationRefs")
                || fields
                    .get("children")
                    .is_some_and(step_tree_has_preparation_refs)
                || fields
                    .get("otherwise")
                    .is_some_and(step_tree_has_preparation_refs)
        }
        _ => false,
    }
}

fn validate_preparations(
    answer: &OperationAnswer,
    packet: &Value,
    known_labels: &BTreeSet<String>,
) -> Result<(), crate::error::ClewError> {
    let known_subjects = packet_subject_references(packet);
    let mut ids = BTreeSet::new();
    for preparation in &answer.preparations {
        if preparation.id.is_empty()
            || !preparation
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(invalid(
                "preparation id must contain only ASCII letters, digits, underscores, or hyphens",
            ));
        }
        if preparation.title.trim().is_empty() {
            return Err(invalid("preparation title must not be empty"));
        }
        if !ids.insert(preparation.id.clone()) {
            return Err(invalid("preparation ids must be unique"));
        }
        validate_claim(
            &preparation.summary,
            known_labels,
            &format!("preparation {} summary", preparation.id),
        )?;
        match preparation.subject_reference.as_deref() {
            Some(reference)
                if reference.trim().is_empty() || !known_subjects.contains_key(reference) =>
            {
                return Err(invalid(format!(
                    "preparation {} subjectReference must identify a declaration or type in this packet",
                    preparation.id
                )));
            }
            None if preparation
                .summary
                .uncertainty
                .as_deref()
                .is_none_or(|uncertainty| uncertainty.trim().is_empty()) =>
            {
                return Err(invalid(format!(
                    "preparation {} without subjectReference needs a precise summary uncertainty",
                    preparation.id
                )));
            }
            _ => {}
        }
        validate_steps(&preparation.steps, known_labels)?;
    }

    let mut references_by_preparation = BTreeMap::<String, Vec<String>>::new();
    let operation_references = resolved_preparation_references(&answer.steps, &ids, "operation")?;
    for preparation in &answer.preparations {
        references_by_preparation.insert(
            preparation.id.clone(),
            resolved_preparation_references(&preparation.steps, &ids, &preparation.id)?,
        );
    }

    let mut pending = VecDeque::from(operation_references);
    let mut reachable = BTreeSet::new();
    while let Some(reference) = pending.pop_front() {
        if !reachable.insert(reference.clone()) {
            continue;
        }
        if let Some(links) = references_by_preparation.get(&reference) {
            pending.extend(links.iter().cloned());
        }
    }
    if reachable != ids {
        return Err(invalid(
            "every preparation must be reachable from an operation step through preparationRefs",
        ));
    }
    Ok(())
}

fn validate_semantic_contract(
    answer: &OperationAnswer,
    packet: &Value,
    known_labels: &BTreeSet<String>,
) -> Result<(), crate::error::ClewError> {
    let known_subjects = packet_subject_references(packet);
    let mut all_ids = BTreeSet::new();
    let mut glossary_ids = BTreeSet::new();
    for term in &answer.glossary {
        validate_semantic_id(&term.id, "glossary")?;
        if !all_ids.insert(term.id.clone()) || !glossary_ids.insert(term.id.clone()) {
            return Err(invalid("operation answer block ids must be unique"));
        }
        if term.label.trim().is_empty() {
            return Err(invalid(format!(
                "glossary {} label must not be empty",
                term.id
            )));
        }
        if !unique_values(&term.subject_refs) || !unique_values(&term.technical_names) {
            return Err(invalid(format!(
                "glossary {} subjectRefs and technicalNames must not contain duplicates",
                term.id
            )));
        }
        validate_claim(
            &term.definition,
            known_labels,
            &format!("glossary {} definition", term.id),
        )?;
        for reference in &term.subject_refs {
            if reference.trim().is_empty() || !known_subjects.contains_key(reference) {
                return Err(invalid(format!(
                    "glossary {} subjectRefs must identify declarations or types in this packet",
                    term.id
                )));
            }
        }
        if term
            .technical_names
            .iter()
            .any(|name| name.trim().is_empty())
        {
            return Err(invalid(format!(
                "glossary {} technicalNames must not be empty",
                term.id
            )));
        }
    }
    for term in &answer.glossary {
        validate_glossary_refs(
            &term.definition.glossary_refs,
            &glossary_ids,
            &format!("glossary {} definition", term.id),
        )?;
    }

    let mut predicate_ids = BTreeSet::new();
    for predicate in &answer.predicates {
        validate_semantic_id(&predicate.id, "predicate")?;
        if !all_ids.insert(predicate.id.clone()) || !predicate_ids.insert(predicate.id.clone()) {
            return Err(invalid("operation answer block ids must be unique"));
        }
        if predicate.label.trim().is_empty() {
            return Err(invalid(format!(
                "predicate {} label must not be empty",
                predicate.id
            )));
        }
        for (name, claim) in [
            ("meaning", &predicate.meaning),
            ("sourceCheck", &predicate.source_check),
            ("evaluation", &predicate.evaluation),
        ] {
            let location = format!("predicate {} {name}", predicate.id);
            validate_claim(claim, known_labels, &location)?;
            validate_glossary_refs(&claim.glossary_refs, &glossary_ids, &location)?;
        }
    }

    let mut referenced_predicates = BTreeSet::new();
    validate_glossary_refs(&answer.summary.glossary_refs, &glossary_ids, "summary")?;
    validate_semantic_steps(
        &answer.steps,
        "operation",
        &mut all_ids,
        &glossary_ids,
        &predicate_ids,
        &mut referenced_predicates,
    )?;
    for preparation in &answer.preparations {
        if !all_ids.insert(preparation.id.clone()) {
            return Err(invalid("operation answer block ids must be unique"));
        }
        validate_glossary_refs(
            &preparation.summary.glossary_refs,
            &glossary_ids,
            &format!("preparation {} summary", preparation.id),
        )?;
        validate_semantic_steps(
            &preparation.steps,
            &format!("preparation {}", preparation.id),
            &mut all_ids,
            &glossary_ids,
            &predicate_ids,
            &mut referenced_predicates,
        )?;
    }
    if referenced_predicates != predicate_ids {
        return Err(invalid(
            "every predicate must be referenced by at least one decision step",
        ));
    }
    Ok(())
}

fn validate_semantic_steps(
    steps: &[OperationStep],
    owner: &str,
    all_ids: &mut BTreeSet<String>,
    glossary_ids: &BTreeSet<String>,
    predicate_ids: &BTreeSet<String>,
    referenced_predicates: &mut BTreeSet<String>,
) -> Result<(), crate::error::ClewError> {
    for (index, step) in steps.iter().enumerate() {
        let location = format!("{owner} step {}", index + 1);
        let id = step.id.as_deref().ok_or_else(|| {
            invalid(format!(
                "{location} requires a stable id in operation-answer/1.2"
            ))
        })?;
        validate_semantic_id(id, "step")?;
        if !all_ids.insert(id.to_owned()) {
            return Err(invalid("operation answer block ids must be unique"));
        }
        validate_glossary_refs(&step.glossary_refs, glossary_ids, &location)?;
        validate_glossary_refs(&step.meaning.glossary_refs, glossary_ids, &location)?;
        match (step.kind.as_str(), step.predicate_ref.as_deref()) {
            ("decision", Some(reference)) if predicate_ids.contains(reference) => {
                referenced_predicates.insert(reference.to_owned());
            }
            ("decision", _) => {
                return Err(invalid(format!(
                    "{location} decision requires a resolved predicateRef"
                )));
            }
            (_, Some(_)) => {
                return Err(invalid(format!(
                    "{location} may use predicateRef only when kind is decision"
                )));
            }
            (_, None) => {}
        }
        validate_semantic_steps(
            &step.children,
            owner,
            all_ids,
            glossary_ids,
            predicate_ids,
            referenced_predicates,
        )?;
        validate_semantic_steps(
            &step.otherwise,
            owner,
            all_ids,
            glossary_ids,
            predicate_ids,
            referenced_predicates,
        )?;
    }
    Ok(())
}

fn validate_semantic_id(id: &str, owner: &str) -> Result<(), crate::error::ClewError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid(format!(
            "{owner} id must contain only ASCII letters, digits, underscores, or hyphens"
        )));
    }
    Ok(())
}

fn unique_values(values: &[String]) -> bool {
    let mut unique = BTreeSet::new();
    values.iter().all(|value| unique.insert(value))
}

fn validate_glossary_refs(
    references: &[String],
    known_ids: &BTreeSet<String>,
    location: &str,
) -> Result<(), crate::error::ClewError> {
    let mut seen = BTreeSet::new();
    for reference in references {
        if reference.trim().is_empty() || !known_ids.contains(reference) {
            return Err(invalid(format!("{location} has an unresolved glossaryRef")));
        }
        if !seen.insert(reference) {
            return Err(invalid(format!("{location} repeats the same glossaryRef")));
        }
    }
    Ok(())
}

fn resolved_preparation_references(
    steps: &[OperationStep],
    known_ids: &BTreeSet<String>,
    owner: &str,
) -> Result<Vec<String>, crate::error::ClewError> {
    let mut references = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let location = format!("{owner} step {}", index + 1);
        let mut local = BTreeSet::new();
        for reference in &step.preparation_refs {
            if reference.trim().is_empty() || !known_ids.contains(reference) {
                return Err(invalid(format!(
                    "{location} has an unresolved preparationRef"
                )));
            }
            if !local.insert(reference) {
                return Err(invalid(format!(
                    "{location} repeats the same preparationRef"
                )));
            }
            references.push(reference.clone());
        }
        references.extend(resolved_preparation_references(
            &step.children,
            known_ids,
            owner,
        )?);
        references.extend(resolved_preparation_references(
            &step.otherwise,
            known_ids,
            owner,
        )?);
    }
    Ok(references)
}

fn packet_subject_references(packet: &Value) -> BTreeMap<String, String> {
    #[derive(Default)]
    struct Subject {
        identity: String,
        owner: Option<String>,
        scope: Option<String>,
        conflicted: bool,
    }

    fn metadata(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    fn merge_metadata(current: &mut Option<String>, incoming: Option<String>) -> bool {
        match (current.as_ref(), incoming) {
            (Some(current), Some(incoming)) if current != &incoming => false,
            (None, Some(incoming)) => {
                *current = Some(incoming);
                true
            }
            _ => true,
        }
    }

    let mut subjects = BTreeMap::<String, Subject>::new();
    let mut insert = |reference: &str, identity: &str, owner: Option<&str>, scope: Option<&str>| {
        if reference.trim().is_empty() || identity.trim().is_empty() {
            return;
        }
        let owner = metadata(owner);
        let scope = metadata(scope);
        let Some(existing) = subjects.get_mut(reference) else {
            subjects.insert(
                reference.to_owned(),
                Subject {
                    identity: identity.to_owned(),
                    owner,
                    scope,
                    conflicted: false,
                },
            );
            return;
        };
        if existing.conflicted {
            return;
        }
        if existing.identity != identity
            || !merge_metadata(&mut existing.owner, owner)
            || !merge_metadata(&mut existing.scope, scope)
        {
            // Conflict is sticky: a later duplicate cannot make this packet
            // reference trustworthy again.
            existing.conflicted = true;
        }
    };

    for node in packet["callMap"]["nodes"].as_array().into_iter().flatten() {
        if let (Some(reference), Some(identity)) = (node["id"].as_str(), node["identity"].as_str())
        {
            insert(
                reference,
                identity,
                node["ownerIdentity"].as_str(),
                node["scope"].as_str(),
            );
        }
    }
    for type_row in packet["types"].as_array().into_iter().flatten() {
        if let Some(identity) = type_row["symbolIdentity"].as_str() {
            if let Some(reference) = type_row["reference"].as_str() {
                insert(
                    reference,
                    identity,
                    type_row["ownerIdentity"].as_str(),
                    type_row["scope"].as_str(),
                );
            }
        } else if let Some(identity) = type_row["identity"].as_str() {
            insert(
                identity,
                identity,
                type_row["ownerIdentity"].as_str(),
                type_row["scope"].as_str(),
            );
        }
    }
    for field in packet["fields"].as_array().into_iter().flatten() {
        if let (Some(reference), Some(owner), Some(name)) = (
            field["reference"].as_str(),
            field["ownerIdentity"].as_str(),
            field["name"].as_str(),
        ) {
            let descriptor = metadata(field["typeDescriptor"].as_str());
            let identity = descriptor
                .as_deref()
                .map(|descriptor| format!("field:{owner}#{name}:{descriptor}"))
                .unwrap_or_else(|| format!("{owner}#{name}"));
            insert(reference, &identity, Some(owner), field["scope"].as_str());
        }
    }
    for method in packet["methods"].as_array().into_iter().flatten() {
        if let (Some(reference), Some(identity)) = (
            method["declarationReference"].as_str(),
            method["symbolIdentity"].as_str(),
        ) {
            insert(
                reference,
                identity,
                method["ownerIdentity"].as_str(),
                method["scope"].as_str(),
            );
        }
    }
    if let (Some(reference), Some(identity)) = (
        packet["root"]["declarationReference"].as_str(),
        packet["root"]["symbolIdentity"].as_str(),
    ) {
        insert(
            reference,
            identity,
            packet["root"]["ownerIdentity"].as_str(),
            packet["root"]["scope"].as_str(),
        );
    }
    for context in packet["sourceContexts"].as_array().into_iter().flatten() {
        if let (Some(reference), Some(identity)) = (
            context["declarationReference"].as_str(),
            context["symbolIdentity"].as_str(),
        ) {
            insert(
                reference,
                identity,
                context["ownerIdentity"].as_str(),
                context["scope"].as_str(),
            );
        }
    }
    if packet["contextDelivery"]["schema"] == super::operation_context::DELIVERY_SCHEMA {
        for row in packet["contextDelivery"]["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|page| page["items"].as_array().into_iter().flatten())
        {
            let normalized = &row["record"]["normalized"];
            if row["kind"] != "DEPENDENCY" || row["record"]["kind"] != "SYMBOL" {
                continue;
            }
            if let (Some(reference), Some(identity), Some(owner), Some(scope)) = (
                row["reference"].as_str(),
                normalized["symbolIdentity"].as_str(),
                normalized["ownerIdentity"].as_str(),
                normalized["scope"].as_str(),
            ) && packet["contextDelivery"]["citations"]
                .get(reference)
                .is_some()
                && packet["citations"].get(reference).is_some()
            {
                insert(reference, identity, Some(owner), Some(scope));
            }
        }
    }
    subjects
        .into_iter()
        .filter_map(|(reference, subject)| {
            if subject.conflicted {
                return None;
            }
            let mut display = format!("{reference} · {}", subject.identity);
            if let Some(owner) = subject.owner.as_deref().filter(|owner| {
                *owner != subject.identity && !subject.identity.starts_with(&format!("{owner}#"))
            }) {
                display.push_str(" · ");
                display.push_str(owner);
            }
            if let Some(scope) = subject.scope {
                display.push_str(" · ");
                display.push_str(&scope);
            }
            Some((reference, display))
        })
        .collect()
}

fn packet_displayed_citation_labels(
    packet: &Value,
    audit: &Value,
) -> Result<BTreeSet<String>, crate::error::ClewError> {
    fn visit(value: &Value, labels: &mut BTreeSet<String>) -> Result<(), crate::error::ClewError> {
        match value {
            Value::Array(values) => {
                for value in values {
                    visit(value, labels)?;
                }
            }
            Value::Object(values) => {
                for (name, value) in values {
                    if name == "evidence" {
                        let values = value.as_array().ok_or_else(|| {
                            invalid("reader packet evidence fields must be arrays")
                        })?;
                        for label in values {
                            let label = label.as_str().ok_or_else(|| {
                                invalid("reader packet evidence labels must be strings")
                            })?;
                            if label.trim().is_empty() {
                                return Err(invalid(
                                    "reader packet evidence labels must not be empty",
                                ));
                            }
                            labels.insert(label.to_owned());
                        }
                    }
                    visit(value, labels)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut labels = BTreeSet::new();
    // Frozen user context has historical record identities, never compiler citation labels.
    let mut compiler_packet = packet.clone();
    if let Some(fields) = compiler_packet.as_object_mut() {
        fields.remove("maintainedContext");
        // Raw delivery contains navigation and receipt metadata. Its actual
        // complete records are checked against the saved audit separately.
        fields.remove("contextDelivery");
    }
    visit(&compiler_packet, &mut labels)?;
    for record in super::operation_context::displayed_records(packet, audit)? {
        if let Some(label) = record["label"].as_str() {
            labels.insert(label.to_owned());
        }
    }
    if let Some(process_intent) = packet.get("processIntent") {
        let process_intent = process_intent
            .as_object()
            .ok_or_else(|| invalid("reader packet processIntent must be an object"))?;
        let definition_reference = process_intent
            .get("definitionReference")
            .and_then(Value::as_str)
            .filter(|reference| !reference.trim().is_empty())
            .ok_or_else(|| {
                invalid("reader packet processIntent.definitionReference must be a nonempty string")
            })?;
        labels.insert(definition_reference.to_owned());

        let continuations = process_intent
            .get("declaredContinuations")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                invalid("reader packet processIntent.declaredContinuations must be an array")
            })?;
        for continuation in continuations {
            let reference = continuation
                .as_object()
                .and_then(|continuation| continuation.get("reference"))
                .and_then(Value::as_str)
                .filter(|reference| !reference.trim().is_empty())
                .ok_or_else(|| {
                    invalid(
                        "reader packet processIntent.declaredContinuations references must be nonempty strings",
                    )
                })?;
            labels.insert(reference.to_owned());
        }
    }
    Ok(labels)
}

fn validate_claim(
    claim: &Claim,
    known_labels: &BTreeSet<String>,
    location: &str,
) -> Result<(), crate::error::ClewError> {
    if claim.text.trim().is_empty() {
        return Err(invalid(format!("{location} claim text must not be empty")));
    }
    if claim.evidence.is_empty() {
        return Err(invalid(format!("{location} claim needs packet evidence")));
    }
    for label in &claim.evidence {
        validate_evidence_label(label, known_labels, location)?;
    }
    if claim
        .uncertainty
        .as_ref()
        .is_some_and(|text| text.trim().is_empty())
    {
        return Err(invalid(format!("{location} uncertainty must not be empty")));
    }
    if !claim.checks.is_empty() {
        return Err(invalid(format!(
            "{location} checks are not supported by the operation-answer contract"
        )));
    }
    Ok(())
}

fn validate_evidence_label(
    label: &str,
    known_labels: &BTreeSet<String>,
    location: &str,
) -> Result<(), crate::error::ClewError> {
    if label.trim().is_empty() || !known_labels.contains(label) {
        return Err(invalid(format!(
            "{location} cites an unknown compact-packet evidence label"
        )));
    }
    Ok(())
}

fn validate_steps(
    steps: &[OperationStep],
    known_labels: &BTreeSet<String>,
) -> Result<(), crate::error::ClewError> {
    for (index, step) in steps.iter().enumerate() {
        let location = format!("step {}", index + 1);
        validate_step(step, known_labels, &location)?;
    }
    Ok(())
}

fn validate_step(
    step: &OperationStep,
    known_labels: &BTreeSet<String>,
    location: &str,
) -> Result<(), crate::error::ClewError> {
    if let Some(id) = step.id.as_deref() {
        validate_semantic_id(id, "step")?;
    }
    if !STEP_KINDS.contains(&step.kind.as_str()) {
        return Err(invalid(format!("{location} has an unsupported kind")));
    }
    validate_claim(&step.meaning, known_labels, location)?;
    for (name, value) in [
        ("from", step.from.as_deref()),
        ("to", step.to.as_deref()),
        ("interaction", step.interaction.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(invalid(format!("{location} {name} must not be empty")));
        }
    }

    match step.kind.as_str() {
        "decision" if step.children.is_empty() => {
            return Err(invalid(format!(
                "{location} decision has no supplied true path"
            )));
        }
        "try" | "loop" if step.children.is_empty() => {
            return Err(invalid(format!(
                "{location} requires a supplied child path"
            )));
        }
        "action" if !step.otherwise.is_empty() => {
            return Err(invalid(format!(
                "{location} otherwise path requires a decision, try, or loop"
            )));
        }
        "return" | "throw" if !step.children.is_empty() || !step.otherwise.is_empty() => {
            return Err(invalid(format!(
                "{location} terminal step cannot have child paths"
            )));
        }
        _ => {}
    }

    for (group, children) in [("child", &step.children), ("otherwise", &step.otherwise)] {
        for (index, child) in children.iter().enumerate() {
            validate_step(
                child,
                known_labels,
                &format!("{location} {group} {}", index + 1),
            )?;
        }
    }
    Ok(())
}

/// JSON paths are host identities; authored IDs and labels cannot collide with them.
pub(super) fn review_blocks(answer: &Value) -> Result<Vec<Value>, crate::error::ClewError> {
    let answer: OperationAnswer = serde_json::from_value(answer.clone())
        .map_err(|e| invalid(format!("invalid operation answer review input: {e}")))?;
    fn row(rows: &mut Vec<Value>, id: String, claim: &Claim) {
        rows.push(serde_json::json!({"id":id,"claim":{"text":claim.text,"checks":claim.checks,"uncertainty":claim.uncertainty,"glossaryRefs":claim.glossary_refs},"evidence":claim.evidence}));
    }
    fn visit(rows: &mut Vec<Value>, steps: &[OperationStep], prefix: &str) {
        for (i, step) in steps.iter().enumerate() {
            let path = format!("{prefix}/{i}");
            row(rows, format!("{path}/meaning"), &step.meaning);
            visit(rows, &step.children, &format!("{path}/children"));
            visit(rows, &step.otherwise, &format!("{path}/otherwise"));
        }
    }
    let mut rows = vec![
        serde_json::json!({"id":"/title","claim":answer.title,"evidence":[]}),
        serde_json::json!({"id":"/uncertainties","claim":answer.uncertainties,"evidence":[]}),
    ];
    row(&mut rows, "/summary".into(), &answer.summary);
    for (i, term) in answer.glossary.iter().enumerate() {
        row(
            &mut rows,
            format!("/glossary/{i}/definition"),
            &term.definition,
        );
    }
    for (i, predicate) in answer.predicates.iter().enumerate() {
        row(
            &mut rows,
            format!("/predicates/{i}/meaning"),
            &predicate.meaning,
        );
        row(
            &mut rows,
            format!("/predicates/{i}/sourceCheck"),
            &predicate.source_check,
        );
        row(
            &mut rows,
            format!("/predicates/{i}/evaluation"),
            &predicate.evaluation,
        );
    }
    visit(&mut rows, &answer.steps, "/steps");
    for (i, preparation) in answer.preparations.iter().enumerate() {
        row(
            &mut rows,
            format!("/preparations/{i}/summary"),
            &preparation.summary,
        );
        visit(
            &mut rows,
            &preparation.steps,
            &format!("/preparations/{i}/steps"),
        );
    }
    if rows.len() > 4096 {
        return Err(invalid(
            "DRAFT_REVIEW_COVERAGE_LIMIT: answer has more than 4096 review blocks",
        ));
    }
    Ok(rows)
}

// Keep separate inputs visible because each renderer consumes distinct source sections.

// Keep separate inputs visible because each renderer consumes distinct source sections.

// Recursive rendering uses each option directly to preserve branch formatting.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ANSWER_SCHEMA: &str = ANSWER_SCHEMA_V1_1;

    fn packet() -> Value {
        let mut packet = json!({
            "schema":PACKET_SCHEMA,
            "profile":"endpoint-context-v3",
            "documentationLanguage":"en",
            "audience":"API maintainers",
            "title":"Transfer request",
            "citations":{
                "d1":"retained handler and declaration facts",
                "p1":"captured endpoint outline"
            },
            "endpoint":{"symbol":"api.Transfer.handle","trigger":"POST /transfer","boundaries":[],"evidence":["p1"]},
            "types":[{
                "identity":"api.TransferRequest",
                "directions":["INPUT"],
                "evidence":["d1"],
                "fields":[{
                    "name":"count",
                    "typeDescriptor":"int",
                    "modifiers":["PRIVATE"],
                    "annotations":[{"name":"Min","value":0}],
                    "sourceTokens":["@Min","(","0",")","int","count"],
                    "evidence":["d1"]
                }]
            }],
            "constants":[{
                "ownerIdentity":"api.TransferPolicy",
                "name":"MAX_TRIES",
                "typeDescriptor":"int",
                "modifiers":["STATIC","FINAL"],
                "annotations":[],
                "sourceTokens":["static","final","int","MAX_TRIES","=","7"],
                "evidence":["d1"]
            }],
            "callMap":{"authority":"RETAINED_TARGET_RELATIONS","order":"NOT_EXECUTION_ORDER","nodes":[],"edges":[{"evidence":["d1"]}]},
            "methodBodies":[{"id":"b1","node":"m0","source":"private void handle() {}","sourceAuthority":"CAPTURED_SOURCE","text":"private void handle() {}","evidence":["d1"]}],
            "coverage":{"coverage":"PARTIAL","runtimeMode":"SOURCE_ONLY","boundaries":[],"callAuthority":"SYNTAX_UNRESOLVED","evidence":["p1"]},
            "limitations":[{"code":"CALL_TARGET_BODY_NOT_CAPTURED","count":2}],
            "interpretationLimits":["Call relations do not establish runtime execution."],
            "runtimeAndSerialization":"UNKNOWN_FROM_THIS_PACKET"
        });
        let packet_digest = crate::documentation::digest(&packet).unwrap();
        packet["packetDigest"] = json!(packet_digest);
        packet
    }

    fn audit(packet: &Value) -> Value {
        let mut audit = json!({
            "schema":"codeclew-documentation-reader-packet-audit/1.0",
            "packetDigest":packet["packetDigest"],
            "records":[]
        });
        seal_audit(&mut audit);
        audit
    }

    fn seal_audit(audit: &mut Value) {
        audit.as_object_mut().unwrap().remove("auditDigest");
        let audit_digest = crate::documentation::digest(audit).unwrap();
        audit["auditDigest"] = json!(audit_digest);
    }

    fn seal(packet: &mut Value) {
        packet.as_object_mut().unwrap().remove("packetDigest");
        let packet_digest = crate::documentation::digest(packet).unwrap();
        packet["packetDigest"] = json!(packet_digest);
    }

    fn simple_answer(packet: &Value, evidence: &str) -> Value {
        json!({
            "schema":ANSWER_SCHEMA,
            "packetDigest":packet["packetDigest"],
            "title":"Reader test",
            "summary":{"text":"A concise summary.","evidence":[evidence]},
            "steps":[{
                "kind":"action",
                "meaning":{"text":"Retain the supported action.","evidence":[evidence]}
            }],
            "preparations":[],
            "uncertainties":[]
        })
    }

    fn answer(packet: &Value) -> Value {
        json!({
            "schema":ANSWER_SCHEMA,
            "packetDigest":packet["packetDigest"],
            "title":"Transfer request handling",
            "summary":{"text":"The captured endpoint delegates to the retained handler.","evidence":["p1","d1"]},
            "steps":[
                {
                    "kind":"decision",
                    "meaning":{"text":"The handler checks whether the request is valid.","evidence":["d1"]},
                    "children":[{
                        "kind":"action",
                        "meaning":{"text":"Continue with the accepted request.","evidence":["d1"]}
                    }],
                    "otherwise":[{
                        "kind":"try",
                        "meaning":{"text":"The handler makes the upstream request.","evidence":["d1"]},
                        "children":[{
                            "kind":"decision",
                            "meaning":{"text":"The upstream response is present.","evidence":["d1"]},
                            "children":[{
                                "kind":"return",
                                "meaning":{"text":"Return the response.","evidence":["d1"]}
                            }],
                            "otherwise":[{
                                "kind":"throw",
                                "meaning":{"text":"Raise for the missing response.","evidence":["d1"]}
                            }]
                        }],
                        "otherwise":[{
                            "kind":"throw",
                            "meaning":{"text":"Translate the caught failure.","evidence":["d1"]}
                        }]
                    }]
                }
            ],
            "preparations":[],
            "uncertainties":[]
        })
    }

    fn semantic_answer(packet: &Value) -> Value {
        let claim = |text: &str, uncertainty: Option<&str>, glossary_refs: Value| {
            let mut value = json!({
                "text":text,
                "evidence":["d1"],
                "glossaryRefs":glossary_refs
            });
            if let Some(uncertainty) = uncertainty {
                value["uncertainty"] = json!(uncertainty);
            }
            value
        };
        json!({
            "schema":ANSWER_SCHEMA_V1_2,
            "packetDigest":packet["packetDigest"],
            "title":"Transfer request handling",
            "summary":claim("The captured handler checks the request count before returning.",None,json!(["request","count"])),
            "glossary":[
                {
                    "id":"request",
                    "label":"Transfer request",
                    "kind":"request",
                    "definition":claim("The captured request type carries an integer count field.",Some("The packet does not identify the business entity, if any, represented by this request."),json!(["count"])),
                    "subjectRefs":["api.TransferRequest"],
                    "technicalNames":["api.TransferRequest","<script>probe</script>"]
                },
                {
                    "id":"count",
                    "label":"Count value",
                    "kind":"term",
                    "definition":claim("The request declares count as an integer; its runtime value is not captured.",None,json!(["request"])),
                    "subjectRefs":["api.TransferRequest"],
                    "technicalNames":["count"]
                }
            ],
            "predicates":[
                {
                    "id":"details",
                    "label":"The request has a positive count",
                    "meaning":claim("The handler accepts the request only when its count is positive.",Some("The packet does not capture the runtime count."),json!(["request","count"])),
                    "sourceCheck":claim("request != null && request.count > 0",None,json!(["request","count"])),
                    "evaluation":claim("The AND checks run left to right. If request is null, short-circuiting prevents reading count; otherwise count must be greater than zero. The packet does not establish any additional null behavior.",Some("No retained source states whether count can be null."),json!(["request","count"]))
                },
                {
                    "id":"retry",
                    "label":"The request is retryable or retries are exhausted",
                    "meaning":claim("This condition selects the retained retry outcome.",None,json!(["request"])),
                    "sourceCheck":claim("!retryable || retryCount >= maxRetries",None,json!(["request"])),
                    "evaluation":claim("The negation applies to retryable. The OR short-circuits when retryable is false; otherwise the retry count comparison decides. Null handling is not established by this expression as captured.",None,json!(["request"]))
                },
                {
                    "id":"missing",
                    "label":"The request count is unavailable",
                    "meaning":claim("The remaining branch represents an unavailable count.",None,json!(["request","count"])),
                    "sourceCheck":claim("request.count == null",None,json!(["request","count"])),
                    "evaluation":claim("This check runs only after the previous conditions fail.",None,json!(["request","count"]))
                }
            ],
            "steps":[
                {
                    "id":"valid-branch",
                    "kind":"decision",
                    "predicateRef":"details",
                    "glossaryRefs":["request"],
                    "meaning":claim("Check request acceptance.",Some("The runtime request value is not present in the packet."),json!(["request"])),
                    "from":"source-from-hidden",
                    "to":"target-to-hidden",
                    "interaction":"interaction-hidden",
                    "children":[{
                        "id":"accept",
                        "kind":"action",
                        "glossaryRefs":["request","count"],
                        "meaning":claim("Continue with the accepted request.",None,json!(["request"])),
                        "preparationRefs":["shared"]
                    }],
                    "otherwise":[{
                        "id":"retry-branch",
                        "kind":"decision",
                        "predicateRef":"retry",
                        "glossaryRefs":["request"],
                        "meaning":claim("Check the retry outcome.",None,json!(["request"])),
                        "children":[{
                            "id":"retry-outcome",
                            "kind":"return",
                            "glossaryRefs":[],
                            "meaning":claim("Return the retained retry response.",None,json!([])),
                            "preparationRefs":["shared"]
                        }],
                        "otherwise":[{
                            "id":"missing-branch",
                            "kind":"decision",
                            "predicateRef":"missing",
                            "glossaryRefs":["count"],
                            "meaning":claim("Check for an unavailable count.",None,json!(["count"])),
                            "children":[{
                                "id":"missing-outcome",
                                "kind":"throw",
                                "glossaryRefs":[],
                                "meaning":claim("Raise for an unavailable count.",None,json!([]))
                            }]
                        }]
                    }]
                }
            ],
            "preparations":[{
                "id":"shared",
                "title":"Shared count preparation",
                "subjectReference":"api.TransferRequest",
                "summary":claim("The same captured preparation is referenced by two outcomes.",Some("The exact helper declaration is not retained."),json!(["count"])),
                "steps":[{
                    "id":"prepare-count",
                    "kind":"action",
                    "glossaryRefs":["count"],
                    "meaning":claim("Keep the supported count preparation.",None,json!(["count"])),
                    "from":"request.count",
                    "to":"prepared.count",
                    "interaction":"helper-interaction-hidden"
                }]
            }],
            "uncertainties":["The packet does not establish runtime values or an unretained no-match outcome."]
        })
    }

    fn field_subject_packet(source_contexts: Value) -> Value {
        let mut packet = packet();
        packet["fields"] = json!([{
            "reference":"field-ref",
            "ownerIdentity":"class:orders.Order",
            "name":"count",
            "typeDescriptor":"int",
            "scope":":main"
        }]);
        packet["sourceContexts"] = source_contexts;
        seal(&mut packet);
        packet
    }

    fn field_subject_answer(packet: &Value) -> Value {
        let mut answer = semantic_answer(packet);
        answer["glossary"][0]["subjectRefs"] = json!(["field-ref"]);
        answer
    }

    fn validate_field_subject(packet: &Value) -> Result<ValidatedAnswer, crate::error::ClewError> {
        validate_draft(packet, &audit(packet), field_subject_answer(packet))
    }

    #[test]
    fn version_1_0_answer_validates_without_rewriting_its_value() {
        let packet = packet();
        let mut legacy = simple_answer(&packet, "d1");
        legacy["schema"] = json!(ANSWER_SCHEMA_V1_0);
        legacy.as_object_mut().unwrap().remove("preparations");
        let original = legacy.clone();

        let rendered = validate(&packet, &audit(&packet), legacy).unwrap();

        assert_eq!(rendered.answer, original);

        let mut unsupported = original.clone();
        unsupported["preparations"] = json!([]);
        assert!(validate(&packet, &audit(&packet), unsupported).is_err());
    }

    #[test]
    fn version_1_2_preserves_authored_semantic_answer_and_validates_references() {
        let mut packet = packet();
        let long_declaration = "method:class:example.linked.ChildWorker#prepare(Lexample/linked/Task;)Ljava/lang/String;";
        packet["callMap"]["nodes"] = json!([{
            "id":"long-declaration",
            "identity":long_declaration,
            "ownerIdentity":"class:example.linked.ChildWorker",
            "scope":":/main"
        }]);
        seal(&mut packet);
        let mut authored = semantic_answer(&packet);
        authored["glossary"][0]["subjectRefs"] = json!(["api.TransferRequest", "long-declaration"]);
        authored["uncertainties"] = json!([
            "SOURCE_RECEIVER_SHADOWING_AMBIGUOUS and CALL_SITE_SOURCE_NOT_CONTAINED_IN_METHOD_BODY remain unresolved."
        ]);
        let original = authored.clone();

        let rendered = validate_draft(&packet, &audit(&packet), authored).unwrap();

        assert_eq!(rendered.answer, original);
    }

    #[test]
    fn semantic_contract_rejects_schema_mismatches_and_legacy_field_injection() {
        let packet = packet();
        let valid = semantic_answer(&packet);

        let mut missing_claim_refs = valid.clone();
        missing_claim_refs["summary"]
            .as_object_mut()
            .unwrap()
            .remove("glossaryRefs");
        assert!(validate_draft(&packet, &audit(&packet), missing_claim_refs).is_err());

        let mut duplicate_subject_refs = valid.clone();
        duplicate_subject_refs["glossary"][0]["subjectRefs"] =
            json!(["api.TransferRequest", "api.TransferRequest"]);
        assert!(validate_draft(&packet, &audit(&packet), duplicate_subject_refs).is_err());

        let mut duplicate_technical_names = valid.clone();
        duplicate_technical_names["glossary"][0]["technicalNames"] =
            json!(["api.TransferRequest", "api.TransferRequest"]);
        assert!(validate_draft(&packet, &audit(&packet), duplicate_technical_names).is_err());

        let mut predicate_on_action = valid.clone();
        predicate_on_action["steps"][0]["children"][0]["predicateRef"] = json!("details");
        assert!(validate_draft(&packet, &audit(&packet), predicate_on_action).is_err());

        let mut null_predicate_on_action = valid.clone();
        null_predicate_on_action["steps"][0]["children"][0]["predicateRef"] = Value::Null;
        assert!(validate_draft(&packet, &audit(&packet), null_predicate_on_action).is_err());

        let mut unsafe_step_id = valid.clone();
        unsafe_step_id["steps"][0]["id"] = json!("valid\" onclick=\"alert(1)");
        assert!(validate_draft(&packet, &audit(&packet), unsafe_step_id).is_err());

        let mut legacy_with_new_id = simple_answer(&packet, "d1");
        legacy_with_new_id["steps"][0]["id"] = json!("step\" onclick=\"alert(1)");
        assert!(validate(&packet, &audit(&packet), legacy_with_new_id).is_err());
    }

    #[test]
    fn expanded_symbol_subjects_require_delivered_identity_and_keep_conflicts_sticky() {
        let symbol = json!({"kind":"DEPENDENCY","reference":"expanded-method",
            "record":{"kind":"SYMBOL","normalized":{"symbolIdentity":"method:class:orders.Helper#run()V",
            "ownerIdentity":"class:orders.Helper","scope":":main"}}});
        let mut packet = json!({"citations":{"expanded-method":"delivered symbol"},
            "contextDelivery":{"schema":super::super::operation_context::DELIVERY_SCHEMA,
                "citations":{"expanded-method":"delivered symbol"},"pages":[{"items":[symbol.clone()]}]}});
        assert!(packet_subject_references(&packet).contains_key("expanded-method"));
        packet["contextDelivery"]["citations"] = serde_json::json!({});
        assert!(!packet_subject_references(&packet).contains_key("expanded-method"));
        packet["contextDelivery"]["citations"] =
            serde_json::json!({"expanded-method":"delivered symbol"});
        for field in ["symbolIdentity", "ownerIdentity", "scope"] {
            let mut conflicting = symbol.clone();
            conflicting["record"]["normalized"][field] = json!("different exact identity");
            packet["contextDelivery"]["pages"][0]["items"] =
                json!([symbol.clone(), conflicting, symbol.clone()]);
            assert!(
                !packet_subject_references(&packet).contains_key("expanded-method"),
                "{field}"
            );
        }
        let mut navigation = symbol.clone();
        navigation["kind"] = json!("SYMBOL_NAVIGATION");
        packet["contextDelivery"]["pages"][0]["items"] = json!([navigation]);
        assert!(!packet_subject_references(&packet).contains_key("expanded-method"));
    }

    #[test]
    fn packet_field_subjects_merge_by_compiler_identity_and_explicit_metadata() {
        let mut packet = field_subject_packet(json!([{
            "declarationReference":"field-ref",
            "symbolIdentity":"field:class:orders.Order#count:int",
            "ownerIdentity":"class:orders.Order",
            "scope":":main"
        }]));
        packet["fields"][0]["scope"] = Value::Null;
        seal(&mut packet);
        let subject = packet_subject_references(&packet);
        assert!(subject.contains_key("field-ref"));
        assert!(subject["field-ref"].ends_with(":main"));
        assert!(validate_field_subject(&packet).is_ok());

        let conflicts = [
            (
                "owner",
                json!({
                    "symbolIdentity":"field:class:orders.Order#count:int",
                    "ownerIdentity":"class:other.Order",
                    "scope":":main"
                }),
            ),
            (
                "name",
                json!({
                    "symbolIdentity":"field:class:orders.Order#total:int",
                    "ownerIdentity":"class:orders.Order",
                    "scope":":main"
                }),
            ),
            (
                "descriptor",
                json!({
                    "symbolIdentity":"field:class:orders.Order#count:long",
                    "ownerIdentity":"class:orders.Order",
                    "scope":":main"
                }),
            ),
            (
                "scope",
                json!({
                    "symbolIdentity":"field:class:orders.Order#count:int",
                    "ownerIdentity":"class:orders.Order",
                    "scope":":other"
                }),
            ),
        ];
        for (case, context) in conflicts {
            let packet = field_subject_packet(json!([{
                "declarationReference":"field-ref",
                "symbolIdentity":context["symbolIdentity"],
                "ownerIdentity":context["ownerIdentity"],
                "scope":context["scope"]
            }]));
            assert!(
                !packet_subject_references(&packet).contains_key("field-ref"),
                "{case} conflict must invalidate the field reference"
            );
            assert!(
                validate_field_subject(&packet).is_err(),
                "{case} conflict must reject glossary use"
            );
        }

        let sticky_conflict = field_subject_packet(json!([
            {
                "declarationReference":"field-ref",
                "symbolIdentity":"field:class:orders.Order#count:int",
                "ownerIdentity":"class:orders.Order",
                "scope":":other"
            },
            {
                "declarationReference":"field-ref",
                "symbolIdentity":"field:class:orders.Order#count:int",
                "ownerIdentity":"class:orders.Order",
                "scope":":main"
            }
        ]));
        assert!(!packet_subject_references(&sticky_conflict).contains_key("field-ref"));
        assert!(validate_field_subject(&sticky_conflict).is_err());
    }

    #[test]
    fn descriptorless_field_reference_remains_standalone_but_cannot_merge_with_full_identity() {
        let mut packet = field_subject_packet(json!([]));
        packet["fields"][0]["typeDescriptor"] = Value::Null;
        packet["fields"][0]["scope"] = Value::Null;
        seal(&mut packet);
        assert_eq!(
            packet_subject_references(&packet)["field-ref"],
            "field-ref · class:orders.Order#count"
        );
        assert!(validate_field_subject(&packet).is_ok());

        let mut unsupported_merge = field_subject_packet(json!([{
            "declarationReference":"field-ref",
            "symbolIdentity":"field:class:orders.Order#count:int",
            "ownerIdentity":"class:orders.Order",
            "scope":":main"
        }]));
        unsupported_merge["fields"][0]["typeDescriptor"] = Value::Null;
        unsupported_merge["fields"][0]["scope"] = Value::Null;
        seal(&mut unsupported_merge);
        assert!(!packet_subject_references(&unsupported_merge).contains_key("field-ref"));
        assert!(validate_field_subject(&unsupported_merge).is_err());
    }

    #[test]
    fn preparation_references_and_subjects_are_validated_without_expanding_cycles() {
        let packet = packet();
        let mut valid = simple_answer(&packet, "d1");
        valid["steps"][0]["preparationRefs"] = json!(["shared"]);
        valid["preparations"] = json!([{
            "id":"shared",
            "title":"Shared explanation",
            "summary":{"text":"A shared explanation with uncertainty.","evidence":["d1"],"uncertainty":"The exact helper is not retained."},
            "steps":[{
                "kind":"action",
                "meaning":{"text":"Keep the supported partial explanation.","evidence":["d1"]},
                "preparationRefs":["shared"]
            }]
        }]);
        let validated = validate(&packet, &audit(&packet), valid.clone()).unwrap();
        assert_eq!(validated.answer, valid);

        let mut unknown_ref = valid.clone();
        unknown_ref["steps"][0]["preparationRefs"] = json!(["missing"]);
        assert!(validate(&packet, &audit(&packet), unknown_ref).is_err());

        let mut unknown_subject = valid.clone();
        unknown_subject["preparations"][0]["subjectReference"] = json!("missing-declaration");
        assert!(validate(&packet, &audit(&packet), unknown_subject).is_err());

        let mut missing_uncertainty = valid.clone();
        missing_uncertainty["preparations"][0]["summary"] =
            json!({"text":"The subject is absent.","evidence":["d1"]});
        assert!(validate(&packet, &audit(&packet), missing_uncertainty).is_err());

        let mut duplicate_id = valid.clone();
        let duplicate = duplicate_id["preparations"][0].clone();
        duplicate_id["preparations"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(validate(&packet, &audit(&packet), duplicate_id).is_err());

        let mut unreachable = valid.clone();
        unreachable["preparations"].as_array_mut().unwrap().push(json!({
            "id":"unused",
            "title":"Unreachable explanation",
            "summary":{"text":"No step links to this record.","evidence":["d1"],"uncertainty":"Its subject is not known."},
            "steps":[]
        }));
        assert!(validate(&packet, &audit(&packet), unreachable).is_err());
    }

    #[test]
    fn digest_evidence_labels_nonempty_steps_and_checks_are_validated() {
        let packet = packet();
        let valid = answer(&packet);
        let audit = audit(&packet);

        let mut wrong_digest = valid.clone();
        wrong_digest["packetDigest"] = json!("sha256:wrong");
        assert!(validate(&packet, &audit, wrong_digest).is_err());

        let mut unknown_label = valid.clone();
        unknown_label["summary"]["evidence"] = json!(["missing"]);
        assert!(validate(&packet, &audit, unknown_label).is_err());

        let mut empty_steps = valid.clone();
        empty_steps["steps"] = json!([]);
        assert!(validate(&packet, &audit, empty_steps).is_err());

        let mut authored_check = valid;
        authored_check["summary"]["checks"] = json!([{
            "kind":"factEquals",
            "evidence":"d1",
            "field":"name",
            "expected":"handle"
        }]);
        assert!(validate(&packet, &audit, authored_check).is_err());
    }

    #[test]
    fn process_intent_references_are_displayed_citations_with_exact_packet_binding() {
        let mut packet = packet();
        packet["profile"] = json!("process-graph-v1");
        packet["citations"]["intent"] = json!("user-authored process definition");
        packet["citations"]["continuation"] = json!("declared continuation definition");
        packet["processIntent"] = json!({
            "authority":"USER_INTENTION_NOT_SOURCE_EVIDENCE",
            "definitionReference":"intent",
            "declaredContinuations":[{"reference":"continuation"}],
            "linkedSubviews":[{"id":"unresolved","authority":"USER_INTENTION_ONLY_NOT_RESOLVED_AS_A_CONTINUATION"}]
        });
        seal(&mut packet);
        let packet_before_validation = packet.clone();
        let original_audit = audit(&packet);
        let rendered = validate(&packet, &original_audit, simple_answer(&packet, "d1")).unwrap();

        assert_eq!(rendered.answer["packetDigest"], packet["packetDigest"]);
        assert_eq!(packet, packet_before_validation);
        let mut packet_without_digest = packet.clone();
        packet_without_digest
            .as_object_mut()
            .unwrap()
            .remove("packetDigest");
        assert_eq!(
            packet["packetDigest"],
            json!(digest(&packet_without_digest).unwrap())
        );

        let mut unknown_reference = packet.clone();
        unknown_reference["processIntent"]["definitionReference"] = json!("missing");
        seal(&mut unknown_reference);
        assert!(
            validate(
                &unknown_reference,
                &audit(&unknown_reference),
                simple_answer(&unknown_reference, "d1"),
            )
            .is_err()
        );

        let mut empty_reference = packet.clone();
        empty_reference["processIntent"]["declaredContinuations"][0]["reference"] = json!("  ");
        seal(&mut empty_reference);
        assert!(
            validate(
                &empty_reference,
                &audit(&empty_reference),
                simple_answer(&empty_reference, "d1"),
            )
            .is_err()
        );

        let mut malformed_references = packet.clone();
        malformed_references["processIntent"]["declaredContinuations"] = json!("not-an-array");
        seal(&mut malformed_references);
        assert!(
            validate(
                &malformed_references,
                &audit(&malformed_references),
                simple_answer(&malformed_references, "d1"),
            )
            .is_err()
        );

        let mut extra_citation = packet.clone();
        extra_citation["citations"]["unused"] = json!("not displayed by the packet");
        seal(&mut extra_citation);
        assert!(
            validate(
                &extra_citation,
                &audit(&extra_citation),
                simple_answer(&extra_citation, "d1"),
            )
            .is_err()
        );
    }
}
