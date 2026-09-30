//! Deterministic offline rendering for a structured endpoint operation answer.
//!
//! This validates packet binding, evidence labels, and tree shape. It does not
//! review semantic correctness or publish documentation.

use super::{digest, invalid, proposals::Claim};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const ANSWER_SCHEMA_V1_0: &str = "codeclew-operation-answer/1.0";
const ANSWER_SCHEMA_V1_1: &str = "codeclew-operation-answer/1.1";
const PACKET_SCHEMA: &str = "codeclew-documentation-reader-packet/1.0";
const STEP_KINDS: &[&str] = &["action", "decision", "try", "return", "throw", "loop"];
// Bump when the answer schema or generic author instruction changes materially;
// this identity is persisted on newly prepared operation Work.
pub(super) const AUTHORING_CONTRACT: &str = "codeclew-operation-draft-authoring/1.1";

pub(super) fn output_schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../../schemas/documentation/operation-answer-1.1.schema.json"
    ))
    .expect("operation answer schema is valid JSON")
}

pub(super) fn validate_and_render_draft(
    packet: &Value,
    audit: &Value,
    answer: Value,
) -> Result<RenderedAnswer, crate::error::ClewError> {
    if answer["schema"].as_str() != Some(ANSWER_SCHEMA_V1_1) {
        return Err(invalid(
            "new operation drafts require codeclew-operation-answer/1.1; the raw author response was retained",
        ));
    }
    validate_and_render(packet, audit, answer)
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
    preparations: Vec<Preparation>,
    uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationStep {
    kind: String,
    meaning: Claim,
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

pub(super) struct RenderedAnswer {
    pub(super) html: String,
    pub(super) markdown: String,
    pub(super) answer: Value,
}

#[derive(Clone, Copy)]
struct ReaderLabels {
    russian: bool,
}

impl ReaderLabels {
    fn new(packet: &Value) -> Self {
        Self {
            russian: packet["documentationLanguage"].as_str() == Some("ru"),
        }
    }

    fn status(self) -> &'static str {
        if self.russian {
            "ЧЕРНОВИК / НЕ ПРОВЕРЕНО"
        } else {
            "DRAFT / UNREVIEWED"
        }
    }

    fn status_note(self) -> &'static str {
        if self.russian {
            "Проверены структура и привязка меток к свидетельствам. Семантическая корректность не проверялась."
        } else {
            "Structure and evidence-label binding were validated. Semantic correctness was not reviewed."
        }
    }

    fn summary(self) -> &'static str {
        if self.russian {
            "Кратко"
        } else {
            "Summary"
        }
    }

    fn ordered(self, process: bool) -> &'static str {
        match (self.russian, process) {
            (true, true) => "Порядок внутреннего процесса",
            (true, false) => "Порядок операции",
            (false, true) => "Ordered internal process behavior",
            (false, false) => "Ordered operation",
        }
    }

    fn facts(self, process: bool) -> &'static str {
        match (self.russian, process) {
            (true, true) => "Типы и поля процесса",
            (true, false) => "Поля DTO и сведения об аннотациях",
            (false, true) => "Process types and fields",
            (false, false) => "Captured DTO and annotation facts",
        }
    }

    fn owner(self) -> &'static str {
        if self.russian {
            "Владелец"
        } else {
            "Owner"
        }
    }

    fn type_name(self) -> &'static str {
        if self.russian { "Тип" } else { "Type" }
    }

    fn field_name(self) -> &'static str {
        if self.russian { "Поле" } else { "Field" }
    }

    fn declared_type(self) -> &'static str {
        if self.russian {
            "Объявленный тип"
        } else {
            "Declared type"
        }
    }

    fn modifiers(self) -> &'static str {
        if self.russian {
            "Модификаторы"
        } else {
            "Modifiers"
        }
    }

    fn annotations(self) -> &'static str {
        if self.russian {
            "Аннотации"
        } else {
            "Annotations"
        }
    }

    fn declaration(self) -> &'static str {
        if self.russian {
            "Токены объявления"
        } else {
            "Declaration tokens"
        }
    }

    fn evidence(self) -> &'static str {
        if self.russian {
            "Свидетельства"
        } else {
            "Evidence"
        }
    }

    fn superclass(self) -> &'static str {
        if self.russian {
            "Базовый класс"
        } else {
            "Superclass"
        }
    }

    fn interfaces(self) -> &'static str {
        if self.russian {
            "Интерфейсы"
        } else {
            "Interfaces"
        }
    }

    fn directions(self) -> &'static str {
        if self.russian {
            "Направления"
        } else {
            "Directions"
        }
    }

    fn no_fields(self) -> &'static str {
        if self.russian {
            "В пакете нет сохранённых объявлений полей."
        } else {
            "No field declarations are listed in this packet."
        }
    }

    fn cited_evidence(self) -> &'static str {
        if self.russian {
            "Использованные свидетельства"
        } else {
            "Cited evidence"
        }
    }

    fn technical_details(self) -> &'static str {
        if self.russian {
            "Технические сведения"
        } else {
            "Technical details"
        }
    }

    fn full_inventory(self) -> &'static str {
        if self.russian {
            "Полный список свидетельств"
        } else {
            "Full evidence inventory"
        }
    }

    fn source_locations(self) -> &'static str {
        if self.russian {
            "Сохранённые фрагменты исходного кода"
        } else {
            "Retained source locations"
        }
    }

    fn no_source_location(self) -> &'static str {
        if self.russian {
            "Для этого свидетельства в пакете нет сохранённой позиции исходного кода."
        } else {
            "No retained source location is available for this evidence in the packet."
        }
    }

    fn source_link(self) -> &'static str {
        if self.russian {
            "исходный код"
        } else {
            "source"
        }
    }

    fn first_match(self) -> &'static str {
        if self.russian {
            "Первое подходящее условие"
        } else {
            "First matching condition"
        }
    }

    fn preparations(self) -> &'static str {
        if self.russian {
            "Подготовка значений и проверки"
        } else {
            "Prepared values and checks"
        }
    }

    fn data_movement(self) -> &'static str {
        if self.russian {
            "Перемещение данных из шагов"
        } else {
            "Data movement stated in steps"
        }
    }

    fn subject(self) -> &'static str {
        if self.russian {
            "Исходный код"
        } else {
            "Source identity"
        }
    }

    fn from(self) -> &'static str {
        if self.russian { "Из" } else { "From" }
    }

    fn to(self) -> &'static str {
        if self.russian { "В" } else { "To" }
    }

    fn unknown(self) -> &'static str {
        if self.russian {
            "Неизвестно"
        } else {
            "Unknown"
        }
    }

    fn uncertainty(self) -> &'static str {
        if self.russian {
            "Неопределённость"
        } else {
            "Uncertainty"
        }
    }

    fn preparation_reference(self) -> &'static str {
        if self.russian {
            "Общее объяснение"
        } else {
            "Shared preparation"
        }
    }

    fn data_movement_note(self) -> &'static str {
        if self.russian {
            "Таблица показывает только явно указанные значения from/to в шагах; совпадение имён не создаёт связь."
        } else {
            "This view contains only explicit from/to values in authored steps; matching names do not create a link."
        }
    }

    fn condition(self) -> &'static str {
        if self.russian {
            "Условие"
        } else {
            "Condition"
        }
    }

    fn outcome(self) -> &'static str {
        if self.russian {
            "Краткое действие"
        } else {
            "Action summary"
        }
    }

    fn details_link(self) -> &'static str {
        if self.russian {
            "Подробности"
        } else {
            "Details"
        }
    }

    fn first_match_note(self) -> &'static str {
        if self.russian {
            "Условия рассматриваются по порядку; следующее условие относится к ветви, только если предыдущие не сработали. Эта таблица — справочное представление, а не исполняемое DMN-правило; она не доказывает чистоту или повторную вычислимость предикатов."
        } else {
            "Conditions are considered in order; a later row applies only when earlier conditions are false. This is a reading aid, not an executable DMN rule, and it does not establish predicate purity or reevaluation behavior."
        }
    }

    fn children_label(self, kind: &str) -> &'static str {
        match (self.russian, kind) {
            (true, "decision") => "Если условие выполнено",
            (true, "try") => "Защищённая ветвь try",
            (true, "loop") => "Тело цикла",
            (true, _) => "Следующие шаги",
            (false, "decision") => "When the condition holds",
            (false, "try") => "Protected try path",
            (false, "loop") => "Loop body",
            (false, _) => "Following substeps",
        }
    }

    fn otherwise_label(self, kind: &str) -> &'static str {
        match (self.russian, kind) {
            (true, "decision") => "Если условие не выполнено",
            (true, "try") => "Ветка catch / иначе",
            (true, "loop") => "Иначе / выход из цикла",
            (true, _) => "Иначе",
            (false, "decision") => "When the condition does not hold",
            (false, "try") => "Catch / otherwise path",
            (false, "loop") => "Otherwise / exit path",
            (false, _) => "Otherwise path",
        }
    }
}

/// Validate an answer against the exact compact packet and render all document
/// views from the same authored step tree.
pub(super) fn validate_and_render(
    packet: &Value,
    audit: &Value,
    answer: Value,
) -> Result<RenderedAnswer, crate::error::ClewError> {
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
    if packet_evidence_labels(packet)? != known_labels {
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
    if parsed.schema == ANSWER_SCHEMA_V1_1 {
        validate_preparations(&parsed, packet, &known_labels)?;
    }
    if parsed
        .uncertainties
        .iter()
        .any(|item| item.trim().is_empty())
    {
        return Err(invalid("operation answer uncertainties must not be empty"));
    }

    let labels = ReaderLabels::new(packet);
    let evidence_index = evidence_index(citations);
    let used_labels = answer_evidence_labels(&parsed);
    let source_navigation = source_navigation(packet, audit, &used_labels);
    let html = render_html(
        packet,
        &parsed,
        citations,
        &evidence_index,
        &used_labels,
        &source_navigation,
        labels,
    );
    let markdown = render_markdown(
        packet,
        &parsed,
        citations,
        &evidence_index,
        &used_labels,
        &source_navigation,
        labels,
    );
    Ok(RenderedAnswer {
        html,
        markdown,
        answer,
    })
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
        }
        ANSWER_SCHEMA_V1_1 => {
            if !answer["preparations"].is_array() {
                return Err(invalid(
                    "operation-answer/1.1 requires a preparations array, which may be empty",
                ));
            }
        }
        _ => return Err(invalid("unsupported operation-answer schema")),
    }
    Ok(schema)
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
    let mut subjects = BTreeMap::<String, String>::new();
    let mut insert = |reference: &str, identity: &str, owner: Option<&str>, scope: Option<&str>| {
        if reference.trim().is_empty() || identity.trim().is_empty() {
            return;
        }
        let mut display = format!("{reference} · {identity}");
        if let Some(owner) = owner.filter(|owner| !owner.trim().is_empty() && *owner != identity) {
            display.push_str(" · ");
            display.push_str(owner);
        }
        if let Some(scope) = scope.filter(|scope| !scope.trim().is_empty()) {
            display.push_str(" · ");
            display.push_str(scope);
        }
        subjects
            .entry(reference.to_owned())
            .and_modify(|existing| {
                if existing != &display {
                    existing.clear();
                }
            })
            .or_insert(display);
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
            insert(reference, &format!("{owner}#{name}"), None, None);
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
    subjects.retain(|_, display: &mut String| !display.is_empty());
    subjects
}

fn packet_evidence_labels(packet: &Value) -> Result<BTreeSet<String>, crate::error::ClewError> {
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
    visit(packet, &mut labels)?;
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

fn evidence_index(citations: &serde_json::Map<String, Value>) -> BTreeMap<String, usize> {
    let mut labels: Vec<_> = citations.keys().cloned().collect();
    labels.sort();
    labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| (label, index + 1))
        .collect()
}

fn answer_evidence_labels(answer: &OperationAnswer) -> BTreeSet<String> {
    fn visit(steps: &[OperationStep], labels: &mut BTreeSet<String>) {
        for step in steps {
            labels.extend(step.meaning.evidence.iter().cloned());
            visit(&step.children, labels);
            visit(&step.otherwise, labels);
        }
    }
    let mut labels = answer.summary.evidence.iter().cloned().collect();
    visit(&answer.steps, &mut labels);
    for preparation in &answer.preparations {
        labels.extend(preparation.summary.evidence.iter().cloned());
        visit(&preparation.steps, &mut labels);
    }
    labels
}

fn preparation_titles(answer: &OperationAnswer) -> BTreeMap<String, String> {
    answer
        .preparations
        .iter()
        .map(|preparation| (preparation.id.clone(), preparation.title.clone()))
        .collect()
}

fn preparation_step_prefix(id: &str) -> String {
    format!("prep-{}-{id}", id.len())
}

fn render_preparations_html(
    answer: &OperationAnswer,
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    if answer.preparations.is_empty() {
        return String::new();
    }
    let subjects = packet_subject_references(packet);
    let mut output = format!(
        "<section id=\"preparations\"><h2>{}</h2>",
        html_escape(labels.preparations())
    );
    for preparation in &answer.preparations {
        output.push_str(&format!(
            "<article class=\"preparation\" id=\"preparation-{}\"><h3>{}</h3>",
            html_escape(&preparation.id),
            html_escape(&preparation.title)
        ));
        if let Some(reference) = preparation.subject_reference.as_deref() {
            let identity = subjects
                .get(reference)
                .map(String::as_str)
                .unwrap_or(labels.unknown());
            output.push_str(&format!(
                "<p class=\"step-meta\"><strong>{}:</strong> <code>{}</code></p>",
                html_escape(labels.subject()),
                html_escape(identity)
            ));
        }
        output.push_str(&render_claim_html(
            &preparation.summary,
            evidence_index,
            labels,
        ));
        if !preparation.steps.is_empty() {
            output.push_str(&render_html_steps(
                &preparation.steps,
                evidence_index,
                true,
                &preparation_step_prefix(&preparation.id),
                preparation_titles,
                labels,
            ));
        }
        output.push_str("</article>");
    }
    output.push_str("</section>");
    output
}

fn render_preparations_markdown(
    answer: &OperationAnswer,
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    if answer.preparations.is_empty() {
        return String::new();
    }
    let subjects = packet_subject_references(packet);
    let mut output = format!(
        "<a id=\"preparations\"></a>\n## {}\n\n",
        markdown_escape(labels.preparations())
    );
    for preparation in &answer.preparations {
        output.push_str(&format!(
            "<a id=\"preparation-{}\"></a>\n### {}\n\n",
            preparation.id,
            markdown_escape(&preparation.title)
        ));
        if let Some(reference) = preparation.subject_reference.as_deref() {
            let identity = subjects
                .get(reference)
                .map(String::as_str)
                .unwrap_or(labels.unknown());
            output.push_str(&format!(
                "**{}:** `{}`\n\n",
                markdown_escape(labels.subject()),
                markdown_code_cell(identity)
            ));
        }
        output.push_str(&render_claim_markdown(
            &preparation.summary,
            evidence_index,
            labels,
        ));
        output.push_str("\n\n");
        if !preparation.steps.is_empty() {
            output.push_str(&render_markdown_steps(
                &preparation.steps,
                evidence_index,
                0,
                true,
                &preparation_step_prefix(&preparation.id),
                preparation_titles,
                labels,
            ));
            output.push('\n');
        }
    }
    output
}

struct DataMovementRow<'a> {
    path: String,
    context: Vec<String>,
    step: &'a OperationStep,
}

fn data_movement_rows<'a>(
    answer: &'a OperationAnswer,
    labels: ReaderLabels,
) -> Vec<DataMovementRow<'a>> {
    fn collect<'a>(
        steps: &'a [OperationStep],
        path_prefix: &str,
        context: &[String],
        output: &mut Vec<DataMovementRow<'a>>,
        labels: ReaderLabels,
    ) {
        for (index, step) in steps.iter().enumerate() {
            let path = step_path(path_prefix, index + 1);
            if step.from.is_some() || step.to.is_some() {
                output.push(DataMovementRow {
                    path: path.clone(),
                    context: context.to_vec(),
                    step,
                });
            }
            let context_summary = |branch: &str| {
                let mut summary = format!("{branch}: {}", step.meaning.text);
                if let Some(uncertainty) = step.meaning.uncertainty.as_deref() {
                    summary.push_str(&format!(" [{}: {uncertainty}]", labels.uncertainty()));
                }
                summary
            };
            let child_context = context_summary(labels.children_label(&step.kind));
            let mut nested_context = context.to_vec();
            nested_context.push(child_context);
            collect(
                &step.children,
                &format!("{path}-then"),
                &nested_context,
                output,
                labels,
            );
            let otherwise_context = context_summary(labels.otherwise_label(&step.kind));
            let mut nested_context = context.to_vec();
            nested_context.push(otherwise_context);
            collect(
                &step.otherwise,
                &format!("{path}-else"),
                &nested_context,
                output,
                labels,
            );
        }
    }

    let mut rows = Vec::new();
    collect(
        &answer.steps,
        "",
        &[if labels.russian {
            "Операция"
        } else {
            "Operation"
        }
        .into()],
        &mut rows,
        labels,
    );
    for preparation in &answer.preparations {
        let mut preparation_context = format!("{}: {}", labels.preparations(), preparation.title);
        if let Some(uncertainty) = preparation.summary.uncertainty.as_deref() {
            preparation_context.push_str(&format!(" [{}: {uncertainty}]", labels.uncertainty()));
        }
        collect(
            &preparation.steps,
            &preparation_step_prefix(&preparation.id),
            &[preparation_context],
            &mut rows,
            labels,
        );
    }
    rows
}

fn render_data_movement_html(
    rows: &[DataMovementRow<'_>],
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<section id=\"data-movement\"><h2>{}</h2><p class=\"muted\">{}</p><table><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
        html_escape(labels.data_movement()),
        html_escape(labels.data_movement_note()),
        html_escape(if labels.russian {
            "Контекст"
        } else {
            "Context"
        }),
        html_escape(labels.from()),
        html_escape(labels.to()),
        html_escape(if labels.russian { "Шаг" } else { "Step" }),
        html_escape(labels.evidence()),
        html_escape(labels.uncertainty())
    );
    for row in rows {
        let context = row
            .context
            .iter()
            .map(|part| html_escape(part))
            .collect::<Vec<_>>()
            .join(" → ");
        let path = format!("step-{}", row.path);
        output.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td><a href=\"#{path}\">{}</a> {}</td><td>{}</td><td>{}</td></tr>",
            context,
            html_escape(row.step.from.as_deref().unwrap_or(labels.unknown())),
            html_escape(row.step.to.as_deref().unwrap_or(labels.unknown())),
            html_escape(&row.step.kind),
            html_escape(&row.step.meaning.text),
            render_evidence_html(&row.step.meaning.evidence, evidence_index),
            row.step.meaning.uncertainty.as_deref().map(html_escape).map(|value| {
                format!("<strong>{}:</strong> {value}", html_escape(labels.uncertainty()))
            }).unwrap_or_else(|| "—".into())
        ));
    }
    output.push_str("</tbody></table></section>");
    output
}

fn render_data_movement_markdown(
    rows: &[DataMovementRow<'_>],
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<a id=\"data-movement\"></a>\n## {}\n\n{}\n\n| {} | {} | {} | {} | {} | {} |\n|---|---|---|---|---|---|\n",
        markdown_escape(labels.data_movement()),
        markdown_escape(labels.data_movement_note()),
        markdown_escape(if labels.russian {
            "Контекст"
        } else {
            "Context"
        }),
        markdown_escape(labels.from()),
        markdown_escape(labels.to()),
        markdown_escape(if labels.russian { "Шаг" } else { "Step" }),
        markdown_escape(labels.evidence()),
        markdown_escape(labels.uncertainty())
    );
    for row in rows {
        let context = row
            .context
            .iter()
            .map(|part| markdown_table_cell(part))
            .collect::<Vec<_>>()
            .join(" → ");
        let path = format!("step-{}", row.path);
        output.push_str(&format!(
            "| {} | {} | {} | [{}]({}) — {} | {} | {} |\n",
            context,
            markdown_table_cell(row.step.from.as_deref().unwrap_or(labels.unknown())),
            markdown_table_cell(row.step.to.as_deref().unwrap_or(labels.unknown())),
            markdown_table_cell(&row.step.kind),
            format!("#{path}"),
            markdown_table_cell(&row.step.meaning.text),
            markdown_evidence_cell(&row.step.meaning.evidence, evidence_index),
            row.step
                .meaning
                .uncertainty
                .as_deref()
                .map(markdown_escape)
                .map(|value| { format!("**{}:** {value}", markdown_escape(labels.uncertainty())) })
                .unwrap_or_else(|| "—".into())
        ));
    }
    output.push('\n');
    output
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceLocation {
    reference: String,
    file: String,
    start_line: u64,
    end_line: u64,
    excerpt: String,
}

#[derive(Default)]
struct SourceNavigation {
    locations: Vec<SourceLocation>,
    evidence_locations: BTreeMap<String, Vec<usize>>,
}

fn source_navigation(packet: &Value, audit: &Value, used: &BTreeSet<String>) -> SourceNavigation {
    if audit["packetDigest"] != packet["packetDigest"]
        || audit["schema"] != "codeclew-documentation-reader-packet-audit/1.0"
        || !audit_digest_matches(audit)
    {
        return SourceNavigation::default();
    }
    let mut source_by_id = BTreeMap::<String, SourceLocation>::new();
    let mut source_by_reference = BTreeMap::<String, SourceLocation>::new();
    let mut source_text_by_reference = BTreeMap::<String, String>::new();
    let mut evidence_source_ids = BTreeMap::<String, BTreeSet<String>>::new();
    for row in audit["records"].as_array().into_iter().flatten() {
        let Some(label) = row["label"].as_str() else {
            continue;
        };
        let Some(kind) = row["kind"].as_str() else {
            continue;
        };
        let record = &row["row"]["record"];
        if kind == "SOURCE"
            && let Some(location) = audit_source_location(label, record)
        {
            let id = row["id"].as_str().unwrap_or_default().to_owned();
            source_by_id.insert(id, location.clone());
            source_by_reference.insert(label.to_owned(), location.clone());
            source_text_by_reference.insert(label.to_owned(), location.excerpt.clone());
            evidence_source_ids
                .entry(label.to_owned())
                .or_default()
                .insert(row["id"].as_str().unwrap_or_default().to_owned());
        }
        if let Some(source_ids) = record["sourceIds"].as_array() {
            for source_id in source_ids.iter().filter_map(Value::as_str) {
                evidence_source_ids
                    .entry(label.to_owned())
                    .or_default()
                    .insert(source_id.to_owned());
            }
        }
    }

    let mut locations_by_evidence = BTreeMap::<String, BTreeSet<SourceLocation>>::new();
    for label in used {
        if let Some(source_ids) = evidence_source_ids.get(label) {
            for source_id in source_ids {
                if let Some(location) = source_by_id.get(source_id) {
                    locations_by_evidence
                        .entry(label.clone())
                        .or_default()
                        .insert(location.clone());
                }
            }
        }
    }

    for source in packet["methodSources"].as_array().into_iter().flatten() {
        let Some(reference) = source["reference"].as_str() else {
            continue;
        };
        if let Some(text) = source["text"].as_str() {
            source_text_by_reference.insert(reference.to_owned(), text.to_owned());
        }
        let Some(parent_location) = source_by_reference.get(reference).cloned() else {
            continue;
        };
        let Some(text) = source["text"].as_str() else {
            continue;
        };
        for alias in source["sourceAliases"].as_array().into_iter().flatten() {
            let Some(alias_reference) = alias["reference"].as_str() else {
                continue;
            };
            let start = alias["startByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok());
            let end = alias["endByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok());
            let Some(location) = start.zip(end).and_then(|(start, end)| {
                source_location_for_range(alias_reference, &parent_location, text, start, end)
            }) else {
                continue;
            };
            for label in std::iter::once(alias_reference)
                .chain(strings(&alias["evidence"]).iter().map(String::as_str))
            {
                if used.contains(label) {
                    locations_by_evidence
                        .entry(label.to_owned())
                        .or_default()
                        .insert(location.clone());
                }
            }
        }
    }

    let mut add_body_location =
        |source_reference: &str, start: usize, end: usize, evidence: Vec<String>| {
            let (Some(parent_location), Some(text)) = (
                source_by_reference.get(source_reference),
                source_text_by_reference.get(source_reference),
            ) else {
                return;
            };
            let Some(location) =
                source_location_for_range(source_reference, parent_location, text, start, end)
            else {
                return;
            };
            for label in evidence.into_iter().filter(|label| used.contains(label)) {
                locations_by_evidence
                    .entry(label)
                    .or_default()
                    .insert(location.clone());
            }
        };
    for body in packet["methodBodies"].as_array().into_iter().flatten() {
        let (Some(reference), Some(start), Some(end)) = (
            body["source"].as_str(),
            body["startByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok()),
            body["endByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok()),
        ) else {
            continue;
        };
        let labels = std::iter::once(reference.to_owned())
            .chain(
                strings(&body["evidence"])
                    .into_iter()
                    .filter(|label| label == reference),
            )
            .collect();
        add_body_location(reference, start, end, labels);
    }
    for method in packet["methods"].as_array().into_iter().flatten() {
        let body = &method["body"];
        let (Some(reference), Some(start), Some(end)) = (
            body["sourceReference"].as_str(),
            body["startByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok()),
            body["endByte"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok()),
        ) else {
            continue;
        };
        let mut labels = vec![reference.to_owned()];
        if let Some(declaration) = method["declarationReference"].as_str() {
            labels.push(declaration.to_owned());
        }
        add_body_location(reference, start, end, labels);
    }

    let mut locations = BTreeSet::new();
    for label in used {
        if let Some(candidates) = locations_by_evidence.get(label) {
            locations.extend(candidates.iter().cloned());
        }
    }
    let locations: Vec<_> = locations.into_iter().collect();
    let location_indices: BTreeMap<_, _> = locations
        .iter()
        .enumerate()
        .map(|(index, location)| (location.clone(), index))
        .collect();
    let evidence_locations = locations_by_evidence
        .into_iter()
        .filter(|(label, _)| used.contains(label))
        .map(|(label, candidates)| {
            let indices = candidates
                .iter()
                .filter_map(|location| location_indices.get(location).copied())
                .collect();
            (label, indices)
        })
        .collect();
    SourceNavigation {
        locations,
        evidence_locations,
    }
}

fn audit_digest_matches(audit: &Value) -> bool {
    let Some(declared_digest) = audit["auditDigest"].as_str() else {
        return false;
    };
    let mut unsigned = audit.clone();
    let Some(object) = unsigned.as_object_mut() else {
        return false;
    };
    object.remove("auditDigest");
    super::digest(&unsigned)
        .ok()
        .is_some_and(|actual| actual == declared_digest)
}

fn audit_source_location(reference: &str, source: &Value) -> Option<SourceLocation> {
    let file = source["file"].as_str()?.trim();
    let start_line = source["startLine"].as_u64()?;
    let end_line = source["endLine"].as_u64()?;
    if file.is_empty() || start_line == 0 || end_line < start_line {
        return None;
    }
    Some(SourceLocation {
        reference: reference.to_owned(),
        file: file.to_owned(),
        start_line,
        end_line,
        excerpt: source["text"].as_str().unwrap_or_default().to_owned(),
    })
}

fn source_location_for_range(
    reference: &str,
    parent: &SourceLocation,
    text: &str,
    start: usize,
    end: usize,
) -> Option<SourceLocation> {
    if start >= end {
        return None;
    }
    let excerpt = text.get(start..end)?;
    let start_prefix = text.get(..start)?;
    let end_char_start = start.checked_add(excerpt.char_indices().next_back()?.0)?;
    let end_prefix = text.get(..end_char_start)?;
    let start_line = parent
        .start_line
        .checked_add(start_prefix.bytes().filter(|byte| *byte == b'\n').count() as u64)?;
    let end_line = parent
        .start_line
        .checked_add(end_prefix.bytes().filter(|byte| *byte == b'\n').count() as u64)?;
    if start_line < parent.start_line || end_line > parent.end_line || end_line < start_line {
        return None;
    }
    Some(SourceLocation {
        reference: reference.to_owned(),
        file: parent.file.clone(),
        start_line,
        end_line,
        excerpt: excerpt.to_owned(),
    })
}

fn render_html(
    packet: &Value,
    answer: &OperationAnswer,
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    source_navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    let language = packet["documentationLanguage"].as_str().unwrap_or("en");
    let preparation_titles = preparation_titles(answer);
    let decision_tables =
        render_decision_tables_html(&answer.steps, evidence_index, &preparation_titles, labels);
    let preparations =
        render_preparations_html(answer, packet, evidence_index, &preparation_titles, labels);
    let movement_rows = data_movement_rows(answer, labels);
    let data_movement = render_data_movement_html(&movement_rows, evidence_index, labels);
    let mut html = String::from("<!doctype html><html lang=\"");
    html.push_str(&html_escape(language));
    html.push_str("\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>");
    html.push_str(&html_escape(&answer.title));
    html.push_str("</title><style>");
    html.push_str(OFFLINE_STYLE);
    html.push_str("</style></head><body><main>");
    html.push_str(&format!(
        "<h1>{}</h1><p class=\"review-status\"><strong>{}</strong> — {}</p>",
        html_escape(&answer.title),
        html_escape(labels.status()),
        html_escape(labels.status_note())
    ));
    let mut nav = vec![
        ("summary", labels.summary()),
        (
            "ordered-behavior",
            labels.ordered(packet["profile"] == "process-graph-v1"),
        ),
    ];
    if !decision_tables.is_empty() {
        nav.push(("first-match-decisions", labels.first_match()));
    }
    if !preparations.is_empty() {
        nav.push(("preparations", labels.preparations()));
    }
    if !data_movement.is_empty() {
        nav.push(("data-movement", labels.data_movement()));
    }
    nav.push(("cited-evidence", labels.cited_evidence()));
    html.push_str(&format!(
        "<nav class=\"document-nav\" aria-label=\"{}\">{}</nav>",
        html_escape(if labels.russian {
            "Навигация по документу"
        } else {
            "Document navigation"
        }),
        nav.iter()
            .map(|(anchor, title)| format!("<a href=\"#{anchor}\">{}</a>", html_escape(title)))
            .collect::<Vec<_>>()
            .join(" · ")
    ));
    html.push_str(&format!(
        "<section id=\"summary\"><h2>{}</h2>",
        html_escape(labels.summary())
    ));
    html.push_str(&render_claim_html(&answer.summary, evidence_index, labels));
    html.push_str("</section><section id=\"ordered-behavior\"><h2>");
    html.push_str(&html_escape(
        labels.ordered(packet["profile"] == "process-graph-v1"),
    ));
    html.push_str("</h2>");
    html.push_str(&render_html_steps(
        &answer.steps,
        evidence_index,
        true,
        "",
        &preparation_titles,
        labels,
    ));
    html.push_str("</section>");
    html.push_str(&decision_tables);
    html.push_str(&preparations);
    html.push_str(&data_movement);
    html.push_str(&render_packet_fact_tables_html(
        packet,
        evidence_index,
        labels,
    ));
    html.push_str(&render_uncertainties_html(answer, labels));
    html.push_str(&render_evidence_index_html(
        citations,
        evidence_index,
        used_labels,
        source_navigation,
        labels,
    ));
    html.push_str(&format!(
        "<details class=\"technical-details\"><summary>{}</summary><p class=\"packet-digest\">{}: <code>{}</code></p><details><summary>{}</summary><figure class=\"step-tree\"><figcaption>{}</figcaption>{}</figure></details>{}{}</details>",
        html_escape(labels.technical_details()),
        html_escape(if labels.russian { "Хеш пакета" } else { "Packet digest" }),
        html_escape(&answer.packet_digest),
        html_escape(if labels.russian { "Дерево шагов" } else { "Operation tree" }),
        html_escape(if labels.russian { "Построено из переданных шагов ответа" } else { "Derived from the supplied answer steps" }),
        render_tree_html(&answer.steps, evidence_index, &preparation_titles, labels),
        render_packet_limits_html(packet, labels),
        render_full_inventory_html(citations, evidence_index, used_labels, labels)
    ));
    html.push_str(&render_source_locations_html(source_navigation, labels));
    html.push_str("</main></body></html>");
    html
}

fn render_markdown(
    packet: &Value,
    answer: &OperationAnswer,
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    source_navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    let preparation_titles = preparation_titles(answer);
    let decision_tables =
        render_decision_tables_markdown(&answer.steps, evidence_index, &preparation_titles, labels);
    let preparations =
        render_preparations_markdown(answer, packet, evidence_index, &preparation_titles, labels);
    let movement_rows = data_movement_rows(answer, labels);
    let data_movement = render_data_movement_markdown(&movement_rows, evidence_index, labels);
    let mut nav = vec![
        format!("[ {} ](#summary)", markdown_escape(labels.summary())),
        format!(
            "[ {} ](#ordered-behavior)",
            markdown_escape(labels.ordered(packet["profile"] == "process-graph-v1"))
        ),
    ];
    if !decision_tables.is_empty() {
        nav.push(format!(
            "[ {} ](#first-match-decisions)",
            markdown_escape(labels.first_match())
        ));
    }
    if !preparations.is_empty() {
        nav.push(format!(
            "[ {} ](#preparations)",
            markdown_escape(labels.preparations())
        ));
    }
    if !data_movement.is_empty() {
        nav.push(format!(
            "[ {} ](#data-movement)",
            markdown_escape(labels.data_movement())
        ));
    }
    nav.push(format!(
        "[ {} ](#cited-evidence)",
        markdown_escape(labels.cited_evidence())
    ));
    let mut markdown = format!(
        "# {}\n\n> **{}** {}\n\n<nav>**{}:** {}</nav>\n\n<a id=\"summary\"></a>\n## {}\n\n{}\n\n<a id=\"ordered-behavior\"></a>\n## {}\n\n{}\n\n",
        markdown_escape(&answer.title),
        markdown_escape(labels.status()),
        markdown_escape(labels.status_note()),
        markdown_escape(if labels.russian {
            "Содержание"
        } else {
            "Contents"
        }),
        nav.join(" · "),
        markdown_escape(labels.summary()),
        render_claim_markdown(&answer.summary, evidence_index, labels),
        markdown_escape(labels.ordered(packet["profile"] == "process-graph-v1")),
        render_markdown_steps(
            &answer.steps,
            evidence_index,
            0,
            true,
            "",
            &preparation_titles,
            labels
        )
    );
    markdown.push_str(&decision_tables);
    markdown.push_str(&preparations);
    markdown.push_str(&data_movement);
    markdown.push_str(&render_packet_fact_tables_markdown(
        packet,
        evidence_index,
        labels,
    ));
    markdown.push_str(&render_uncertainties_markdown(answer, labels));
    markdown.push_str(&render_evidence_index_markdown(
        citations,
        evidence_index,
        used_labels,
        source_navigation,
        labels,
    ));
    markdown.push_str(&format!(
        "<details>\n<summary>{}</summary>\n\n**{}:** `{}`\n\n### {}\n\n{}\n\n{}{}\n</details>\n",
        markdown_escape(labels.technical_details()),
        markdown_escape(if labels.russian {
            "Хеш пакета"
        } else {
            "Packet digest"
        }),
        markdown_escape(&answer.packet_digest),
        markdown_escape(if labels.russian {
            "Дерево шагов"
        } else {
            "Operation tree"
        }),
        markdown_tree_block(&answer.steps, &preparation_titles, labels),
        render_packet_limits_markdown(packet, labels),
        render_full_inventory_markdown(citations, evidence_index, used_labels, labels)
    ));
    markdown.push_str(&render_source_locations_markdown(source_navigation, labels));
    markdown
}

const OFFLINE_STYLE: &str = r#"
:root{color-scheme:light dark;font:16px/1.55 system-ui,sans-serif;--line:#8792a2;--panel:#171b22;--accent:#73b7ff}
*{box-sizing:border-box}body{margin:0;background:#101319;color:#e8edf5}main{max-width:1120px;margin:auto;padding:2rem}
h1,h2,h3{line-height:1.2}h2{margin-top:2.2rem;border-bottom:1px solid #394252;padding-bottom:.45rem}
a{color:var(--accent)}code{overflow-wrap:anywhere}.review-status{padding:.85rem 1rem;border-left:4px solid #d99e45;background:#29231a}
.packet-digest{color:#bac4d3}.claim,.step-node,.tree-node{border:1px solid #394252;border-radius:.55rem;padding:.8rem 1rem;margin:.55rem 0;background:var(--panel)}
.claim-text,.step-text{white-space:pre-wrap}.claim-uncertainty{color:#ffd08a}.citations{display:inline-flex;gap:.45rem;flex-wrap:wrap;margin-left:.45rem;font-size:.9em}
.citation{border:1px solid #52627a;border-radius:1rem;padding:.05rem .5rem;text-decoration:none}.step-kind{font-size:.75em;text-transform:uppercase;letter-spacing:.06em;color:#9ed0ff;margin-right:.55rem}
.step-meta{color:#b7c1d0;font-size:.9em}.ordered-steps,.nested-steps{padding-left:1.5rem}.path-group{margin:.5rem 0 .75rem 1rem;padding-left:.8rem;border-left:2px solid #52627a}.path-label{font-weight:650;color:#bdc9dc}
.step-tree ul{list-style:none;margin:.25rem 0 .25rem 1rem;padding-left:1rem;border-left:2px solid var(--line)}.step-tree li{position:relative;padding:.25rem 0 .25rem .4rem}.step-tree li::before{content:"";position:absolute;left:-1rem;top:1.25rem;width:.8rem;border-top:2px solid var(--line)}
.tree-node{display:inline-block;max-width:100%}.tree-branch-label{margin:.35rem 0 0 1rem;color:#bdc9dc;font-size:.9em}
table{border-collapse:collapse;width:100%;margin:1rem 0 1.5rem}caption{text-align:left;font-weight:700;margin:.5rem 0}th,td{border:1px solid #596273;padding:.5rem .65rem;text-align:left;vertical-align:top;overflow-wrap:anywhere}th{background:#242b36}
.evidence-index,.limitations,.uncertainties{padding-left:1.4rem}.muted{color:#bac4d3}figure{margin:0}figcaption{font-weight:650}
@media(prefers-color-scheme:light){body{background:#fff;color:#18202b}.claim,.step-node,.tree-node{background:#f6f8fb;border-color:#ccd3df}.review-status{background:#fff7e8}.step-meta,.muted{color:#49586d}th{background:#edf1f7}}
"#;

fn render_claim_html(
    claim: &Claim,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    let mut output = format!(
        "<div class=\"claim\"><p class=\"claim-text\">{}</p>{}",
        html_escape(&claim.text),
        render_evidence_html(&claim.evidence, evidence_index)
    );
    if let Some(uncertainty) = claim.uncertainty.as_deref() {
        output.push_str(&format!(
            "<p class=\"claim-uncertainty\"><strong>{}:</strong> {}</p>",
            html_escape(if labels.russian {
                "Неопределённость"
            } else {
                "Uncertainty"
            }),
            html_escape(uncertainty)
        ));
    }
    output.push_str("</div>");
    output
}

fn render_evidence_html(labels: &[String], index: &BTreeMap<String, usize>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let links = labels
        .iter()
        .map(|label| match index.get(label) {
            Some(number) => format!(
                "<a class=\"citation\" href=\"#evidence-{number}\">{}</a>",
                html_escape(label)
            ),
            None => html_escape(label),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("<span class=\"citations\" aria-label=\"Packet evidence\">{links}</span>")
}

fn render_html_steps(
    steps: &[OperationStep],
    evidence_index: &BTreeMap<String, usize>,
    ordered: bool,
    path_prefix: &str,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let tag = if ordered { "ol" } else { "ul" };
    let class = if ordered {
        "ordered-steps"
    } else {
        "nested-steps"
    };
    let mut output = format!("<{tag} class=\"{class}\">");
    for (index, step) in steps.iter().enumerate() {
        let step_path = step_path(path_prefix, index + 1);
        output.push_str(&format!(
            "<li id=\"step-{step_path}\"><div class=\"step-node\"><span class=\"step-kind\">"
        ));
        output.push_str(&html_escape(&step.kind));
        output.push_str("</span><span class=\"step-text\">");
        output.push_str(&html_escape(&step.meaning.text));
        output.push_str("</span>");
        output.push_str(&render_evidence_html(
            &step.meaning.evidence,
            evidence_index,
        ));
        output.push_str(&render_step_metadata_html(step, labels));
        output.push_str(&render_preparation_links_html(
            &step.preparation_refs,
            preparation_titles,
            labels,
        ));
        if let Some(uncertainty) = step.meaning.uncertainty.as_deref() {
            output.push_str(&format!(
                "<p class=\"claim-uncertainty\"><strong>{}:</strong> {}</p>",
                html_escape(if labels.russian {
                    "Неопределённость"
                } else {
                    "Uncertainty"
                }),
                html_escape(uncertainty)
            ));
        }
        output.push_str("</div>");
        if !step.children.is_empty() {
            output.push_str(&format!(
                "<div class=\"path-group\"><div class=\"path-label\">{}</div>{}</div>",
                html_escape(labels.children_label(&step.kind)),
                render_html_steps(
                    &step.children,
                    evidence_index,
                    false,
                    &format!("{step_path}-then"),
                    preparation_titles,
                    labels
                )
            ));
        }
        if !step.otherwise.is_empty() {
            output.push_str(&format!(
                "<div class=\"path-group\"><div class=\"path-label\">{}</div>{}</div>",
                html_escape(labels.otherwise_label(&step.kind)),
                render_html_steps(
                    &step.otherwise,
                    evidence_index,
                    false,
                    &format!("{step_path}-else"),
                    preparation_titles,
                    labels
                )
            ));
        }
        output.push_str("</li>");
    }
    output.push_str(&format!("</{tag}>"));
    output
}

fn render_preparation_links_html(
    references: &[String],
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let links = references
        .iter()
        .filter_map(|reference| {
            let title = preparation_titles.get(reference)?;
            Some(format!(
                "<a href=\"#preparation-{}\">{}</a>",
                html_escape(reference),
                html_escape(title)
            ))
        })
        .collect::<Vec<_>>();
    if links.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"step-meta preparation-refs\"><strong>{}:</strong> {}</p>",
            html_escape(labels.preparation_reference()),
            links.join(" · ")
        )
    }
}

fn render_preparation_links_markdown(
    references: &[String],
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let links = references
        .iter()
        .filter_map(|reference| {
            let title = preparation_titles.get(reference)?;
            Some(format!(
                "[{}](#preparation-{})",
                markdown_escape(title),
                reference
            ))
        })
        .collect::<Vec<_>>();
    if links.is_empty() {
        String::new()
    } else {
        format!(
            "  **{}:** {}",
            markdown_escape(labels.preparation_reference()),
            links.join(" · ")
        )
    }
}

fn step_path(prefix: &str, index: usize) -> String {
    if prefix.is_empty() {
        index.to_string()
    } else {
        format!("{prefix}-{index}")
    }
}

fn render_step_metadata_html(step: &OperationStep, labels: ReaderLabels) -> String {
    let mut values = Vec::new();
    for (label, value) in [
        (
            if labels.russian { "От" } else { "From" },
            step.from.as_deref(),
        ),
        (if labels.russian { "К" } else { "To" }, step.to.as_deref()),
        (
            if labels.russian {
                "Взаимодействие"
            } else {
                "Interaction"
            },
            step.interaction.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            values.push(format!(
                "<span><strong>{label}:</strong> {}</span>",
                html_escape(value)
            ));
        }
    }
    if values.is_empty() {
        String::new()
    } else {
        format!("<p class=\"step-meta\">{}</p>", values.join(" · "))
    }
}

fn render_tree_html(
    steps: &[OperationStep],
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    if steps.is_empty() {
        return String::from("<p class=\"muted\">No steps supplied.</p>");
    }
    let mut output = String::from("<ul class=\"tree-root\">");
    for step in steps {
        output.push_str("<li><div class=\"tree-node\"><span class=\"step-kind\">");
        output.push_str(&html_escape(&step.kind));
        output.push_str("</span> ");
        output.push_str(&html_escape(&step.meaning.text));
        output.push_str(&render_evidence_html(
            &step.meaning.evidence,
            evidence_index,
        ));
        output.push_str(&render_step_metadata_html(step, labels));
        output.push_str(&render_preparation_links_html(
            &step.preparation_refs,
            preparation_titles,
            labels,
        ));
        if !step.children.is_empty() {
            output.push_str(&format!(
                "</div><div class=\"tree-branch-label\">{}</div>{}",
                html_escape(labels.children_label(&step.kind)),
                render_tree_html(&step.children, evidence_index, preparation_titles, labels)
            ));
        } else {
            output.push_str("</div>");
        }
        if !step.otherwise.is_empty() {
            output.push_str(&format!(
                "<div class=\"tree-branch-label\">{}</div>{}",
                html_escape(labels.otherwise_label(&step.kind)),
                render_tree_html(&step.otherwise, evidence_index, preparation_titles, labels)
            ));
        }
        output.push_str("</li>");
    }
    output.push_str("</ul>");
    output
}

fn render_tree_text(
    steps: &[OperationStep],
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    fn append(
        steps: &[OperationStep],
        depth: usize,
        label: Option<&str>,
        lines: &mut Vec<String>,
        preparation_titles: &BTreeMap<String, String>,
        labels: ReaderLabels,
    ) {
        if let Some(label) = label {
            lines.push(format!("{}[{}]", "  ".repeat(depth), label));
        }
        for (index, step) in steps.iter().enumerate() {
            let branch = if index + 1 == steps.len() {
                "└─"
            } else {
                "├─"
            };
            let preparation_links = step
                .preparation_refs
                .iter()
                .filter_map(|reference| preparation_titles.get(reference))
                .map(|title| format!("{}: {}", labels.preparation_reference(), title))
                .collect::<Vec<_>>();
            let preparation_suffix = if preparation_links.is_empty() {
                String::new()
            } else {
                format!(" [{}]", preparation_links.join("; "))
            };
            lines.push(format!(
                "{}{} {}: {}{}",
                "  ".repeat(depth),
                branch,
                step.kind,
                tree_plain_text(&step.meaning.text)
                    .replace('\n', " ")
                    .trim(),
                format!("{}{}", tree_metadata(step), preparation_suffix)
            ));
            if !step.children.is_empty() {
                append(
                    &step.children,
                    depth + 1,
                    Some(labels.children_label(&step.kind)),
                    lines,
                    preparation_titles,
                    labels,
                );
            }
            if !step.otherwise.is_empty() {
                append(
                    &step.otherwise,
                    depth + 1,
                    Some(labels.otherwise_label(&step.kind)),
                    lines,
                    preparation_titles,
                    labels,
                );
            }
        }
    }
    let mut lines = Vec::new();
    append(steps, 0, None, &mut lines, preparation_titles, labels);
    lines.join("\n")
}

fn tree_plain_text(value: &str) -> String {
    value.replace("\r", " ").replace('\t', " ")
}

fn tree_metadata(step: &OperationStep) -> String {
    let mut values = Vec::new();
    for (name, value) in [
        ("from", step.from.as_deref()),
        ("to", step.to.as_deref()),
        ("interaction", step.interaction.as_deref()),
    ] {
        if let Some(value) = value {
            values.push(format!(
                "{name}={}",
                tree_plain_text(value).replace('\n', " ")
            ));
        }
    }
    if values.is_empty() {
        String::new()
    } else {
        format!(" ({})", values.join(", "))
    }
}

fn render_markdown_steps(
    steps: &[OperationStep],
    evidence_index: &BTreeMap<String, usize>,
    depth: usize,
    ordered: bool,
    path_prefix: &str,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let mut output = String::new();
    let indent = "   ".repeat(depth);
    for (index, step) in steps.iter().enumerate() {
        let step_path = step_path(path_prefix, index + 1);
        output.push_str(&format!("<a id=\"step-{step_path}\"></a>"));
        let prefix = if ordered && depth == 0 {
            format!("{}. ", index + 1)
        } else {
            format!("{}- ", indent)
        };
        output.push_str(&format!(
            "{prefix}**{}:** {}{}{}{}\n",
            markdown_escape(&step.kind),
            markdown_escape(&step.meaning.text),
            render_evidence_markdown(&step.meaning.evidence, evidence_index, labels),
            markdown_step_metadata(step, labels),
            render_preparation_links_markdown(&step.preparation_refs, preparation_titles, labels)
        ));
        if let Some(uncertainty) = step.meaning.uncertainty.as_deref() {
            output.push_str(&format!(
                "{}  - **{}:** {}\n",
                indent,
                markdown_escape(if labels.russian {
                    "Неопределённость"
                } else {
                    "Uncertainty"
                }),
                markdown_escape(uncertainty)
            ));
        }
        if !step.children.is_empty() {
            output.push_str(&format!(
                "{}  **{}:**\n",
                indent,
                markdown_escape(labels.children_label(&step.kind))
            ));
            output.push_str(&render_markdown_steps(
                &step.children,
                evidence_index,
                depth + 1,
                false,
                &format!("{step_path}-then"),
                preparation_titles,
                labels,
            ));
        }
        if !step.otherwise.is_empty() {
            output.push_str(&format!(
                "{}  **{}:**\n",
                indent,
                markdown_escape(labels.otherwise_label(&step.kind))
            ));
            output.push_str(&render_markdown_steps(
                &step.otherwise,
                evidence_index,
                depth + 1,
                false,
                &format!("{step_path}-else"),
                preparation_titles,
                labels,
            ));
        }
    }
    output
}

fn markdown_step_metadata(step: &OperationStep, labels: ReaderLabels) -> String {
    let mut values = Vec::new();
    for (label, value) in [
        (
            if labels.russian { "От" } else { "From" },
            step.from.as_deref(),
        ),
        (if labels.russian { "К" } else { "To" }, step.to.as_deref()),
        (
            if labels.russian {
                "Взаимодействие"
            } else {
                "Interaction"
            },
            step.interaction.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            values.push(format!("**{label}:** {}", markdown_escape(value)));
        }
    }
    if values.is_empty() {
        String::new()
    } else {
        format!("  ({})", values.join(" · "))
    }
}

fn render_claim_markdown(
    claim: &Claim,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    let mut output = format!(
        "{}{}",
        markdown_escape(&claim.text),
        render_evidence_markdown(&claim.evidence, evidence_index, labels)
    );
    if let Some(uncertainty) = claim.uncertainty.as_deref() {
        output.push_str(&format!(
            "\n\n**{}:** {}",
            markdown_escape(if labels.russian {
                "Неопределённость"
            } else {
                "Uncertainty"
            }),
            markdown_escape(uncertainty)
        ));
    }
    output
}

fn render_evidence_markdown(
    evidence: &[String],
    index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if evidence.is_empty() {
        return String::new();
    }
    let links = evidence
        .iter()
        .map(|label| match index.get(label) {
            Some(number) => format!("[{}](#evidence-{number})", markdown_escape(label)),
            None => markdown_escape(label),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "  _({}: {links})_ ",
        if labels.russian {
            "Свидетельства"
        } else {
            "Evidence"
        }
    )
}

struct FirstMatchBranch<'a> {
    decision: &'a OperationStep,
    path: String,
}

struct FirstMatchTable<'a> {
    branches: Vec<FirstMatchBranch<'a>>,
    otherwise: &'a [OperationStep],
    otherwise_path: String,
}

fn first_match_table<'a>(
    root: &'a OperationStep,
    root_path: String,
) -> Option<FirstMatchTable<'a>> {
    let mut branches = Vec::new();
    let mut decision = root;
    let mut path = root_path;
    loop {
        if decision.kind != "decision" {
            return None;
        }
        branches.push(FirstMatchBranch {
            decision,
            path: path.clone(),
        });
        match decision.otherwise.as_slice() {
            [next] if next.kind == "decision" => {
                decision = next;
                path.push_str("-else-1");
            }
            otherwise => {
                if branches.len() + usize::from(!otherwise.is_empty()) <= 2 {
                    return None;
                }
                return Some(FirstMatchTable {
                    branches,
                    otherwise,
                    otherwise_path: format!("{path}-else"),
                });
            }
        }
    }
}

fn collect_first_match_tables<'a>(
    steps: &'a [OperationStep],
    path_prefix: &str,
    output: &mut Vec<FirstMatchTable<'a>>,
) {
    for (index, step) in steps.iter().enumerate() {
        let path = step_path(path_prefix, index + 1);
        if let Some(table) = (step.kind == "decision")
            .then(|| first_match_table(step, path.clone()))
            .flatten()
        {
            for branch in &table.branches {
                collect_first_match_tables(
                    &branch.decision.children,
                    &format!("{}-then", branch.path),
                    output,
                );
            }
            collect_first_match_tables(&table.otherwise, &table.otherwise_path, output);
            output.push(table);
            continue;
        }
        collect_first_match_tables(&step.children, &format!("{path}-then"), output);
        collect_first_match_tables(&step.otherwise, &format!("{path}-else"), output);
    }
}

fn branch_summary(steps: &[OperationStep]) -> Option<(&OperationStep, String)> {
    let first = steps.first()?;
    let text = first
        .meaning
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let remaining = steps.len().saturating_sub(1);
    Some((
        first,
        if remaining == 0 {
            text
        } else {
            format!("{text} (+{remaining})")
        },
    ))
}

fn table_outcome_html(
    steps: &[OperationStep],
    path_prefix: &str,
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let Some((first, summary)) = branch_summary(steps) else {
        return "<span class=\"muted\">—</span>".into();
    };
    let target = step_path(path_prefix, 1);
    format!(
        "<span class=\"step-kind\">{}</span> {} <a href=\"#step-{target}\">{}</a>{}{}{}",
        html_escape(&first.kind),
        html_escape(&summary),
        html_escape(labels.details_link()),
        render_evidence_html(&first.meaning.evidence, evidence_index),
        render_step_metadata_html(first, labels),
        render_preparation_links_html(&first.preparation_refs, preparation_titles, labels)
    )
}

fn table_outcome_markdown(
    steps: &[OperationStep],
    path_prefix: &str,
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let Some((first, summary)) = branch_summary(steps) else {
        return "—".into();
    };
    let target = step_path(path_prefix, 1);
    format!(
        "**{}:** {} [ {} ](#step-{target}){}{}{}",
        markdown_escape(&first.kind),
        markdown_escape(&summary),
        markdown_escape(labels.details_link()),
        render_evidence_markdown(&first.meaning.evidence, evidence_index, labels),
        markdown_step_metadata(first, labels),
        render_preparation_links_markdown(&first.preparation_refs, preparation_titles, labels)
    )
}

fn render_decision_tables_html(
    steps: &[OperationStep],
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let mut tables = Vec::new();
    collect_first_match_tables(steps, "", &mut tables);
    if tables.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<section id=\"first-match-decisions\"><h2>{}</h2>",
        html_escape(labels.first_match())
    );
    for table in tables {
        output.push_str(&format!(
            "<table class=\"first-match\"><caption>{}</caption><thead><tr><th>{}</th><th>{}</th></tr></thead><tbody>",
            html_escape(labels.first_match()),
            html_escape(labels.condition()),
            html_escape(labels.outcome())
        ));
        for branch in &table.branches {
            output.push_str(&format!(
                "<tr><td>{}{}</td><td>{}</td></tr>",
                html_escape(&branch.decision.meaning.text),
                render_evidence_html(&branch.decision.meaning.evidence, evidence_index),
                table_outcome_html(
                    &branch.decision.children,
                    &format!("{}-then", branch.path),
                    evidence_index,
                    preparation_titles,
                    labels
                )
            ));
        }
        if !table.otherwise.is_empty() {
            output.push_str(&format!(
                "<tr><td>{}</td><td>{}</td></tr>",
                html_escape(if labels.russian {
                    "Иначе"
                } else {
                    "Otherwise"
                }),
                table_outcome_html(
                    &table.otherwise,
                    &table.otherwise_path,
                    evidence_index,
                    preparation_titles,
                    labels
                )
            ));
        }
        output.push_str("</tbody></table>");
        output.push_str(&format!(
            "<p class=\"muted\">{}</p>",
            html_escape(labels.first_match_note())
        ));
    }
    output.push_str("</section>");
    output
}

fn render_decision_tables_markdown(
    steps: &[OperationStep],
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let mut tables = Vec::new();
    collect_first_match_tables(steps, "", &mut tables);
    if tables.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<a id=\"first-match-decisions\"></a>\n## {}\n\n",
        markdown_escape(labels.first_match())
    );
    for table in tables {
        output.push_str(&format!(
            "| {} | {} |\n|---|---|\n",
            markdown_escape(labels.condition()),
            markdown_escape(labels.outcome())
        ));
        for branch in &table.branches {
            output.push_str(&format!(
                "| {}{} | {} |\n",
                markdown_table_cell(&branch.decision.meaning.text),
                render_evidence_markdown(&branch.decision.meaning.evidence, evidence_index, labels),
                table_outcome_markdown(
                    &branch.decision.children,
                    &format!("{}-then", branch.path),
                    evidence_index,
                    preparation_titles,
                    labels
                )
            ));
        }
        if !table.otherwise.is_empty() {
            output.push_str(&format!(
                "| {} | {} |\n",
                markdown_escape(if labels.russian {
                    "Иначе"
                } else {
                    "Otherwise"
                }),
                table_outcome_markdown(
                    &table.otherwise,
                    &table.otherwise_path,
                    evidence_index,
                    preparation_titles,
                    labels
                )
            ));
        }
        output.push_str(&format!(
            "\n{}\n\n",
            markdown_escape(labels.first_match_note())
        ));
    }
    output
}

fn render_packet_fact_tables_html(
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if packet["profile"] == "process-graph-v1" {
        return render_process_fact_tables_html(packet, evidence_index, labels);
    }
    let types = packet["types"].as_array().cloned().unwrap_or_default();
    let constants = packet["constants"].as_array().cloned().unwrap_or_default();
    if types.is_empty() && constants.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<section id=\"packet-facts\"><h2>{}</h2>",
        html_escape(labels.facts(false))
    );
    for dto in types {
        let fields = dto["fields"].as_array().cloned().unwrap_or_default();
        output.push_str(&format!(
            "<table><caption>{} {} · {} {}</caption><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
            html_escape(labels.type_name()),
            html_escape(&value_text(&dto["identity"])),
            html_escape(labels.directions()),
            html_escape(&value_text(&dto["directions"])),
            html_escape(labels.field_name()),
            html_escape(labels.declared_type()),
            html_escape(labels.modifiers()),
            html_escape(labels.annotations()),
            html_escape(labels.declaration()),
            html_escape(labels.evidence())
        ));
        if fields.is_empty() {
            output.push_str(&format!(
                "<tr><td colspan=\"6\" class=\"muted\">{}</td></tr>",
                html_escape(labels.no_fields())
            ));
        }
        for field in fields {
            let labels = combined_labels(&dto["evidence"], &field["evidence"]);
            output.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                html_escape(&value_text(&field["name"])),
                html_escape(&value_text(&field["typeDescriptor"])),
                html_escape(&value_text(&field["modifiers"])),
                html_escape(&value_text(&field["annotations"])),
                html_escape(&tokens_text(&field["sourceTokens"])),
                render_evidence_html(&labels, evidence_index)
            ));
        }
        output.push_str("</tbody></table>");
    }
    if !constants.is_empty() {
        output.push_str(&format!(
            "<table><caption>{}</caption><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
            html_escape(if labels.russian { "Сохранённые константы" } else { "Retained constant declarations" }),
            html_escape(labels.owner()),
            html_escape(labels.field_name()),
            html_escape(labels.declared_type()),
            html_escape(labels.modifiers()),
            html_escape(labels.annotations()),
            html_escape(labels.declaration()),
            html_escape(labels.evidence())
        ));
        for constant in constants {
            output.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                html_escape(&value_text(&constant["ownerIdentity"])),
                html_escape(&value_text(&constant["name"])),
                html_escape(&value_text(&constant["typeDescriptor"])),
                html_escape(&value_text(&constant["modifiers"])),
                html_escape(&value_text(&constant["annotations"])),
                html_escape(&tokens_text(&constant["sourceTokens"])),
                render_evidence_html(&strings(&constant["evidence"]), evidence_index)
            ));
        }
        output.push_str("</tbody></table>");
    }
    output.push_str("</section>");
    output
}

fn render_packet_fact_tables_markdown(
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if packet["profile"] == "process-graph-v1" {
        return render_process_fact_tables_markdown(packet, evidence_index, labels);
    }
    let types = packet["types"].as_array().cloned().unwrap_or_default();
    let constants = packet["constants"].as_array().cloned().unwrap_or_default();
    if types.is_empty() && constants.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<a id=\"packet-facts\"></a>\n## {}\n\n",
        markdown_escape(labels.facts(false))
    );
    for dto in types {
        output.push_str(&format!(
            "### Type {} · directions {}\n\n| Field | Declared type | Modifiers | Annotations | Declaration tokens | Evidence |\n|---|---|---|---|---|---|\n",
            markdown_escape(&value_text(&dto["identity"])),
            markdown_escape(&value_text(&dto["directions"]))
        ));
        let fields = dto["fields"].as_array().cloned().unwrap_or_default();
        if fields.is_empty() {
            output
                .push_str("| No field declarations are listed in this packet. |  |  |  |  |  |\n");
        }
        for field in fields {
            let labels = combined_labels(&dto["evidence"], &field["evidence"]);
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                markdown_table_cell(&value_text(&field["name"])),
                markdown_table_cell(&value_text(&field["typeDescriptor"])),
                markdown_table_cell(&value_text(&field["modifiers"])),
                markdown_table_cell(&value_text(&field["annotations"])),
                markdown_code_cell(&tokens_text(&field["sourceTokens"])),
                markdown_evidence_cell(&labels, evidence_index)
            ));
        }
        output.push('\n');
    }
    if !constants.is_empty() {
        output.push_str("### Retained constant declarations\n\n| Owner | Name | Declared type | Modifiers | Annotations | Declaration tokens | Evidence |\n|---|---|---|---|---|---|---|\n");
        for constant in constants {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                markdown_table_cell(&value_text(&constant["ownerIdentity"])),
                markdown_table_cell(&value_text(&constant["name"])),
                markdown_table_cell(&value_text(&constant["typeDescriptor"])),
                markdown_table_cell(&value_text(&constant["modifiers"])),
                markdown_table_cell(&value_text(&constant["annotations"])),
                markdown_code_cell(&tokens_text(&constant["sourceTokens"])),
                markdown_evidence_cell(&strings(&constant["evidence"]), evidence_index)
            ));
        }
        output.push('\n');
    }
    output
}

fn process_owner_name(owner: &Value, type_names: &BTreeMap<String, String>) -> String {
    let identity = owner.as_str().unwrap_or_default();
    match type_names.get(identity) {
        Some(name) if name != identity => format!("{name} ({identity})"),
        _ => value_text(owner),
    }
}

fn render_process_fact_tables_html(
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    let types = packet["types"].as_array().cloned().unwrap_or_default();
    let fields = packet["fields"].as_array().cloned().unwrap_or_default();
    if types.is_empty() && fields.is_empty() {
        return String::new();
    }
    let type_names: BTreeMap<_, _> = types
        .iter()
        .filter_map(|type_row| {
            Some((
                type_row["symbolIdentity"].as_str()?.to_owned(),
                type_row["name"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let mut output = format!(
        "<section id=\"packet-facts\"><h2>{}</h2>",
        html_escape(labels.facts(true))
    );
    if !types.is_empty() {
        output.push_str(&format!(
            "<table><caption>{}</caption><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
            html_escape(if labels.russian { "Сохранённые типы" } else { "Retained types" }),
            html_escape(labels.type_name()),
            html_escape(if labels.russian { "Вид" } else { "Kind" }),
            html_escape(labels.owner()),
            html_escape(labels.superclass()),
            html_escape(labels.interfaces())
        ));
        for type_row in types {
            output.push_str(&format!(
                "<tr><td>{} <code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape(&value_text(&type_row["name"])),
                html_escape(&value_text(&type_row["symbolIdentity"])),
                html_escape(&value_text(&type_row["declarationKind"])),
                html_escape(&value_text(&type_row["ownerIdentity"])),
                html_escape(&value_text(&type_row["superclass"])),
                html_escape(&value_text(&type_row["interfaces"]))
            ));
        }
        output.push_str("</tbody></table>");
    }
    if !fields.is_empty() {
        output.push_str(&format!(
            "<table><caption>{}</caption><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
            html_escape(if labels.russian { "Сохранённые поля" } else { "Retained fields" }),
            html_escape(labels.owner()),
            html_escape(labels.field_name()),
            html_escape(labels.declared_type()),
            html_escape(labels.modifiers()),
            html_escape(labels.annotations()),
            html_escape(labels.declaration()),
            html_escape(labels.evidence())
        ));
        for field in fields {
            output.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                html_escape(&process_owner_name(&field["ownerIdentity"], &type_names)),
                html_escape(&value_text(&field["name"])),
                html_escape(&value_text(&field["typeDescriptor"])),
                html_escape(&value_text(&field["modifiers"])),
                html_escape(&value_text(&field["annotations"])),
                html_escape(&tokens_text(&field["sourceTokens"])),
                render_evidence_html(&strings(&field["evidence"]), evidence_index)
            ));
        }
        output.push_str("</tbody></table>");
    }
    output.push_str("</section>");
    output
}

fn render_process_fact_tables_markdown(
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    let types = packet["types"].as_array().cloned().unwrap_or_default();
    let fields = packet["fields"].as_array().cloned().unwrap_or_default();
    if types.is_empty() && fields.is_empty() {
        return String::new();
    }
    let type_names: BTreeMap<_, _> = types
        .iter()
        .filter_map(|type_row| {
            Some((
                type_row["symbolIdentity"].as_str()?.to_owned(),
                type_row["name"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let mut output = format!(
        "<a id=\"packet-facts\"></a>\n## {}\n\n",
        markdown_escape(labels.facts(true))
    );
    if !types.is_empty() {
        output.push_str(&format!(
            "### {}\n\n| {} | {} | {} | {} | {} |\n|---|---|---|---|---|\n",
            markdown_escape(if labels.russian {
                "Сохранённые типы"
            } else {
                "Retained types"
            }),
            markdown_escape(labels.type_name()),
            markdown_escape(if labels.russian { "Вид" } else { "Kind" }),
            markdown_escape(labels.owner()),
            markdown_escape(labels.superclass()),
            markdown_escape(labels.interfaces())
        ));
        for type_row in types {
            output.push_str(&format!(
                "| {} `{}` | {} | {} | {} | {} |\n",
                markdown_table_cell(&value_text(&type_row["name"])),
                markdown_code_cell(&value_text(&type_row["symbolIdentity"])),
                markdown_table_cell(&value_text(&type_row["declarationKind"])),
                markdown_table_cell(&value_text(&type_row["ownerIdentity"])),
                markdown_table_cell(&value_text(&type_row["superclass"])),
                markdown_table_cell(&value_text(&type_row["interfaces"]))
            ));
        }
        output.push('\n');
    }
    if !fields.is_empty() {
        output.push_str(&format!(
            "### {}\n\n| {} | {} | {} | {} | {} | {} | {} |\n|---|---|---|---|---|---|---|\n",
            markdown_escape(if labels.russian {
                "Сохранённые поля"
            } else {
                "Retained fields"
            }),
            markdown_escape(labels.owner()),
            markdown_escape(labels.field_name()),
            markdown_escape(labels.declared_type()),
            markdown_escape(labels.modifiers()),
            markdown_escape(labels.annotations()),
            markdown_escape(labels.declaration()),
            markdown_escape(labels.evidence())
        ));
        for field in fields {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                markdown_table_cell(&process_owner_name(&field["ownerIdentity"], &type_names)),
                markdown_table_cell(&value_text(&field["name"])),
                markdown_table_cell(&value_text(&field["typeDescriptor"])),
                markdown_table_cell(&value_text(&field["modifiers"])),
                markdown_table_cell(&value_text(&field["annotations"])),
                markdown_code_cell(&tokens_text(&field["sourceTokens"])),
                markdown_evidence_cell(&strings(&field["evidence"]), evidence_index)
            ));
        }
        output.push('\n');
    }
    output
}

fn render_packet_limits_html(packet: &Value, labels: ReaderLabels) -> String {
    let limits = packet["limitations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let interpretation = packet["interpretationLimits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let endpoint_boundaries = packet["endpoint"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let coverage_boundaries = packet["coverage"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let runtime = packet.get("runtimeAndSerialization");
    if limits.is_empty()
        && interpretation.is_empty()
        && endpoint_boundaries.is_empty()
        && coverage_boundaries.is_empty()
        && runtime.is_none()
    {
        return String::new();
    }
    let mut output = format!(
        "<section class=\"limitations\"><h2>{}</h2>",
        html_escape(if labels.russian {
            "Ограничения пакета"
        } else {
            "Packet gaps and limits"
        })
    );
    append_html_value_list(
        &mut output,
        if labels.russian {
            "Сохранённые пробелы"
        } else {
            "Captured gaps"
        },
        &limits,
    );
    append_html_value_list(
        &mut output,
        if labels.russian {
            "Ограничения интерпретации"
        } else {
            "Interpretation limits"
        },
        &interpretation,
    );
    append_html_value_list(
        &mut output,
        if labels.russian {
            "Границы HTTP-точки"
        } else {
            "Endpoint boundaries"
        },
        &endpoint_boundaries,
    );
    append_html_value_list(
        &mut output,
        if labels.russian {
            "Границы покрытия"
        } else {
            "Coverage boundaries"
        },
        &coverage_boundaries,
    );
    if let Some(runtime) = runtime {
        output.push_str(&format!(
            "<p><strong>{}:</strong> {}</p>",
            html_escape(if labels.russian {
                "Среда выполнения и сериализация"
            } else {
                "Runtime and serialization"
            }),
            html_escape(&value_text(runtime))
        ));
    }
    output.push_str("</section>");
    output
}

fn append_html_value_list(output: &mut String, title: &str, values: &[Value]) {
    if values.is_empty() {
        return;
    }
    output.push_str(&format!(
        "<h3>{}</h3><ul class=\"limitations\">",
        html_escape(title)
    ));
    for value in values {
        output.push_str(&format!(
            "<li><code>{}</code></li>",
            html_escape(&value_text(value))
        ));
    }
    output.push_str("</ul>");
}

fn render_packet_limits_markdown(packet: &Value, labels: ReaderLabels) -> String {
    let limits = packet["limitations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let interpretation = packet["interpretationLimits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let endpoint_boundaries = packet["endpoint"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let coverage_boundaries = packet["coverage"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let runtime = packet.get("runtimeAndSerialization");
    if limits.is_empty()
        && interpretation.is_empty()
        && endpoint_boundaries.is_empty()
        && coverage_boundaries.is_empty()
        && runtime.is_none()
    {
        return String::new();
    }
    let mut output = format!(
        "## {}\n\n",
        markdown_escape(if labels.russian {
            "Ограничения пакета"
        } else {
            "Packet gaps and limits"
        })
    );
    append_markdown_value_list(
        &mut output,
        if labels.russian {
            "Сохранённые пробелы"
        } else {
            "Captured gaps"
        },
        &limits,
    );
    append_markdown_value_list(
        &mut output,
        if labels.russian {
            "Ограничения интерпретации"
        } else {
            "Interpretation limits"
        },
        &interpretation,
    );
    append_markdown_value_list(
        &mut output,
        if labels.russian {
            "Границы HTTP-точки"
        } else {
            "Endpoint boundaries"
        },
        &endpoint_boundaries,
    );
    append_markdown_value_list(
        &mut output,
        if labels.russian {
            "Границы покрытия"
        } else {
            "Coverage boundaries"
        },
        &coverage_boundaries,
    );
    if let Some(runtime) = runtime {
        output.push_str(&format!(
            "**{}:** {}\n\n",
            markdown_escape(if labels.russian {
                "Среда выполнения и сериализация"
            } else {
                "Runtime and serialization"
            }),
            markdown_escape(&value_text(runtime))
        ));
    }
    output
}

fn append_markdown_value_list(output: &mut String, title: &str, values: &[Value]) {
    if values.is_empty() {
        return;
    }
    output.push_str(&format!("### {}\n\n", markdown_escape(title)));
    for value in values {
        output.push_str(&format!("- `{}`\n", markdown_escape(&value_text(value))));
    }
    output.push('\n');
}

fn render_uncertainties_html(answer: &OperationAnswer, labels: ReaderLabels) -> String {
    if answer.uncertainties.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<section><h2>{}</h2><ul class=\"uncertainties\">",
        html_escape(if labels.russian {
            "Неопределённости"
        } else {
            "Uncertainties"
        })
    );
    for uncertainty in &answer.uncertainties {
        output.push_str(&format!("<li>{}</li>", html_escape(uncertainty)));
    }
    output.push_str("</ul></section>");
    output
}

fn render_uncertainties_markdown(answer: &OperationAnswer, labels: ReaderLabels) -> String {
    if answer.uncertainties.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "## {}\n\n",
        markdown_escape(if labels.russian {
            "Неопределённости"
        } else {
            "Uncertainties"
        })
    );
    for uncertainty in &answer.uncertainties {
        output.push_str(&format!("- {}\n", markdown_escape(uncertainty)));
    }
    output.push('\n');
    output
}

fn render_evidence_index_html(
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    source_navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    if used_labels.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<section id=\"cited-evidence\"><h2>{}</h2><ol class=\"evidence-index\">",
        html_escape(labels.cited_evidence())
    );
    for label in used_labels {
        let Some(number) = evidence_index.get(label) else {
            continue;
        };
        output.push_str(&format!(
            "<li id=\"evidence-{number}\"><strong>{}</strong> — {}{} </li>",
            html_escape(label),
            html_escape(&value_text(&citations[label])),
            render_source_links_html(label, source_navigation, labels)
        ));
    }
    output.push_str("</ol></section>");
    output
}

fn render_evidence_index_markdown(
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    source_navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    if used_labels.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<a id=\"cited-evidence\"></a>\n## {}\n\n",
        markdown_escape(labels.cited_evidence())
    );
    for label in used_labels {
        let Some(number) = evidence_index.get(label) else {
            continue;
        };
        output.push_str(&format!(
            "<a id=\"evidence-{number}\"></a>- **{}** — {}{}\n",
            markdown_escape(label),
            markdown_escape(&value_text(&citations[label])),
            render_source_links_markdown(label, source_navigation, labels)
        ));
    }
    output.push('\n');
    output
}

fn render_source_links_html(
    label: &str,
    navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    match navigation
        .evidence_locations
        .get(label)
        .filter(|items| !items.is_empty())
    {
        Some(items) => {
            let links = items
                .iter()
                .map(|index| {
                    let location = &navigation.locations[*index];
                    format!(
                        "<a href=\"#source-{}\">{}:{}–{}</a>",
                        index + 1,
                        html_escape(&location.file),
                        location.start_line,
                        location.end_line
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                " <span class=\"source-links\">{}: {links}</span>",
                html_escape(labels.source_link())
            )
        }
        None => format!(
            " <span class=\"muted source-missing\">{}</span>",
            html_escape(labels.no_source_location())
        ),
    }
}

fn render_source_links_markdown(
    label: &str,
    navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    match navigation
        .evidence_locations
        .get(label)
        .filter(|items| !items.is_empty())
    {
        Some(items) => {
            let links = items
                .iter()
                .map(|index| {
                    let location = &navigation.locations[*index];
                    format!(
                        "[{}:{}–{}](#source-{})",
                        markdown_code_cell(&location.file),
                        location.start_line,
                        location.end_line,
                        index + 1
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("  _({}: {links})_", markdown_escape(labels.source_link()))
        }
        None => format!("  _({})_", markdown_escape(labels.no_source_location())),
    }
}

fn render_full_inventory_html(
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    labels: ReaderLabels,
) -> String {
    let unused: Vec<_> = evidence_index
        .keys()
        .filter(|label| !used_labels.contains(*label))
        .collect();
    if unused.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<details class=\"full-evidence\"><summary>{}</summary><ol class=\"evidence-index\">",
        html_escape(labels.full_inventory())
    );
    for label in unused {
        if let Some(number) = evidence_index.get(label) {
            output.push_str(&format!(
                "<li id=\"evidence-{number}\"><strong>{}</strong> — {}</li>",
                html_escape(label),
                html_escape(&value_text(&citations[label]))
            ));
        }
    }
    output.push_str("</ol></details>");
    output
}

fn render_full_inventory_markdown(
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    labels: ReaderLabels,
) -> String {
    let unused: Vec<_> = evidence_index
        .keys()
        .filter(|label| !used_labels.contains(*label))
        .collect();
    if unused.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<details>\n<summary>{}</summary>\n\n",
        markdown_escape(labels.full_inventory())
    );
    for label in unused {
        if let Some(number) = evidence_index.get(label) {
            output.push_str(&format!(
                "<a id=\"evidence-{number}\"></a>- **{}** — {}\n",
                markdown_escape(label),
                markdown_escape(&value_text(&citations[label]))
            ));
        }
    }
    output.push_str("\n</details>\n");
    output
}

fn render_source_locations_html(navigation: &SourceNavigation, labels: ReaderLabels) -> String {
    if navigation.locations.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<section id=\"source-locations\"><h2>{}</h2><ol class=\"source-locations\">",
        html_escape(labels.source_locations())
    );
    for (index, location) in navigation.locations.iter().enumerate() {
        output.push_str(&format!(
            "<li id=\"source-{}\"><strong><code>{}</code>: {}–{}</strong> <span class=\"muted\">{}</span>",
            index + 1,
            html_escape(&location.file),
            location.start_line,
            location.end_line,
            html_escape(&location.reference)
        ));
        if !location.excerpt.is_empty() {
            output.push_str(&format!(
                "<details><summary>{}</summary><pre><code>{}</code></pre></details>",
                html_escape(if labels.russian {
                    "Показать сохранённый фрагмент"
                } else {
                    "View retained excerpt"
                }),
                html_escape(&location.excerpt)
            ));
        }
        output.push_str("</li>");
    }
    output.push_str("</ol></section>");
    output
}

fn render_source_locations_markdown(navigation: &SourceNavigation, labels: ReaderLabels) -> String {
    if navigation.locations.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<a id=\"source-locations\"></a>\n## {}\n\n",
        markdown_escape(labels.source_locations())
    );
    for (index, location) in navigation.locations.iter().enumerate() {
        output.push_str(&format!(
            "<a id=\"source-{}\"></a>- **{}:** {}–{} _({})_\n",
            index + 1,
            markdown_code_cell(&location.file),
            location.start_line,
            location.end_line,
            markdown_escape(&location.reference)
        ));
        if !location.excerpt.is_empty() {
            output.push_str(&format!(
                "\n<details>\n<summary>{}</summary>\n\n{}\n</details>\n\n",
                markdown_escape(if labels.russian {
                    "Показать сохранённый фрагмент"
                } else {
                    "View retained excerpt"
                }),
                markdown_code_block(&location.excerpt)
            ));
        }
    }
    output.push('\n');
    output
}

fn combined_labels(first: &Value, second: &Value) -> Vec<String> {
    let mut seen = BTreeSet::new();
    strings(first)
        .into_iter()
        .chain(strings(second))
        .filter(|label| seen.insert(label.clone()))
        .collect()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::String(text) => text.clone(),
        Value::Array(values) => values.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Object(_) => compact_json(value),
        _ => value.to_string(),
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[unavailable]".into())
}

fn markdown_tree_block(
    steps: &[OperationStep],
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let tree = render_tree_text(steps, preparation_titles, labels);
    markdown_code_block(&tree)
}

fn markdown_code_block(text: &str) -> String {
    let longest_backtick_run = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(3.max(longest_backtick_run + 1));
    format!("{fence}text\n{text}\n{fence}")
}

fn markdown_table_cell(value: &str) -> String {
    markdown_escape(&value.replace(['\r', '\n'], " "))
}

fn markdown_code_cell(value: &str) -> String {
    if value == "—" {
        return value.into();
    }
    let value = value
        .replace(['\r', '\n'], " ")
        .replace('|', "\\|")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let longest_run = value
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run + 1);
    format!("{fence}{value}{fence}")
}

fn markdown_evidence_cell(labels: &[String], index: &BTreeMap<String, usize>) -> String {
    if labels.is_empty() {
        return "—".into();
    }
    labels
        .iter()
        .map(|label| match index.get(label) {
            Some(number) => format!("[{}](#evidence-{number})", markdown_escape(label)),
            None => markdown_escape(label),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn tokens_text(value: &Value) -> String {
    let Some(tokens) = value.as_array() else {
        return "—".into();
    };
    tokens
        .iter()
        .map(|token| {
            token
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value_text(token))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn markdown_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '.'
            | '!' | '|' | '~' => {
                output.push('\\');
                output.push(character);
            }
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '&' => output.push_str("&amp;"),
            '\r' => {}
            '\n' => output.push(' '),
            _ => output.push(character),
        }
    }
    output
}

fn html_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
    output
}

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

    #[test]
    fn version_1_0_answer_still_renders_offline_without_rewriting_its_value() {
        let packet = packet();
        let mut legacy = simple_answer(&packet, "d1");
        legacy["schema"] = json!(ANSWER_SCHEMA_V1_0);
        legacy.as_object_mut().unwrap().remove("preparations");
        let original = legacy.clone();

        let rendered = validate_and_render(&packet, &audit(&packet), legacy).unwrap();

        assert_eq!(rendered.answer, original);
        assert!(rendered.html.contains("Reader test"));
        assert!(!rendered.html.contains("id=\"preparations\""));
        assert!(!rendered.markdown.contains("## Prepared values and checks"));
        let mut unsupported = original.clone();
        unsupported["preparations"] = json!([]);
        assert!(validate_and_render(&packet, &audit(&packet), unsupported).is_err());
    }

    #[test]
    fn preparations_are_shared_linked_and_data_movement_keeps_evidence_uncertainty_and_context() {
        let mut packet = packet();
        packet["callMap"]["nodes"] = json!([
            {
                "id":"declaration-a",
                "identity":"method:example.first.Mapper#map()V",
                "ownerIdentity":"class:example.first.Mapper",
                "scope":":module-a",
                "evidence":["d1"]
            },
            {
                "id":"declaration-b",
                "identity":"method:example.second.Mapper#map()V",
                "ownerIdentity":"class:example.second.Mapper",
                "scope":":module-b",
                "evidence":["d1"]
            }
        ]);
        seal(&mut packet);

        let mut authored = answer(&packet);
        authored["steps"] = json!([{
            "kind":"decision",
            "meaning":{
                "text":"The caller selects a preparation path.",
                "evidence":["d1"],
                "uncertainty":"The retained packet does not establish the runtime receiver."
            },
            "children":[{
                "kind":"action",
                "meaning":{"text":"Use the shared preparation.","evidence":["d1"]},
                "preparationRefs":["a","unresolved-subject"]
            }],
            "otherwise":[{
                "kind":"action",
                "meaning":{"text":"Use the alternate explanation.","evidence":["d1"]},
                "preparationRefs":["a","a-1-then"]
            }]
        }]);
        authored["preparations"] = json!([
            {
                "id":"a",
                "title":"Shared mapper preparation",
                "subjectReference":"declaration-a",
                "summary":{"text":"The first mapper prepares a value.","evidence":["d1"]},
                "steps":[{
                    "kind":"decision",
                    "meaning":{
                        "text":"An optional input is present.",
                        "evidence":["d1"],
                        "uncertainty":"The predicate may depend on runtime state."
                    },
                    "children":[{
                        "kind":"action",
                        "meaning":{
                            "text":"Read the optional source value.",
                            "evidence":["d1"],
                            "uncertainty":"The destination is not identified in this packet."
                        },
                        "from":"request.options",
                        "preparationRefs":["a-1-then"]
                    }]
                }]
            },
            {
                "id":"a-1-then",
                "title":"Same-named mapper in another owner",
                "subjectReference":"declaration-b",
                "summary":{"text":"The second mapper has a distinct retained identity.","evidence":["d1"]},
                "steps":[{
                    "kind":"action",
                    "meaning":{"text":"Map the normalized value.","evidence":["d1"]},
                    "from":"normalized.input",
                    "to":"payload.value",
                    "preparationRefs":["a"]
                },{
                    "kind":"try",
                    "meaning":{
                        "text":"The caller attempts an optional retry.",
                        "evidence":["d1"],
                        "uncertainty":"The retained source does not show which external failures qualify."
                    },
                    "children":[{
                        "kind":"loop",
                        "meaning":{
                            "text":"Repeat for each selected item.",
                            "evidence":["d1"],
                            "uncertainty":"The loop termination condition is not retained."
                        },
                        "children":[{
                            "kind":"action",
                            "meaning":{"text":"Prepare one retry item.","evidence":["d1"]},
                            "from":"retry.input",
                            "to":"retry.output"
                        }]
                    }],
                    "otherwise":[{
                        "kind":"throw",
                        "meaning":{"text":"Record an unavailable retry outcome.","evidence":["d1"]}
                    }]
                }]
            },
            {
                "id":"unresolved-subject",
                "title":"Caller-side partial preparation",
                "summary":{
                    "text":"The caller prepares a value before the concrete helper is known.",
                    "evidence":["d1"],
                    "uncertainty":"No exact helper declaration is available in the packet."
                },
                "steps":[{
                    "kind":"action",
                    "meaning":{"text":"Retain the supported caller-side value.","evidence":["d1"]},
                    "from":"caller.value",
                    "to":"prepared.value"
                }]
            }
        ]);
        let original = authored.clone();

        let rendered = validate_and_render(&packet, &audit(&packet), authored).unwrap();

        assert_eq!(rendered.answer, original);
        assert_eq!(rendered.html.matches("id=\"preparation-a\"").count(), 1);
        assert_eq!(
            rendered.html.matches("id=\"preparation-a-1-then\"").count(),
            1
        );
        assert_eq!(rendered.html.matches("class=\"preparation\"").count(), 3);
        assert!(rendered.html.contains("method:example.first.Mapper#map()V"));
        assert!(
            rendered
                .html
                .contains("method:example.second.Mapper#map()V")
        );
        assert!(rendered.html.contains(":module-a"));
        assert!(rendered.html.contains(":module-b"));
        assert!(rendered.html.contains("id=\"step-prep-1-a-1-then-1\""));
        assert!(rendered.html.contains("id=\"step-prep-8-a-1-then-1\""));
        assert!(rendered.html.contains("href=\"#step-prep-1-a-1-then-1\""));
        assert!(rendered.html.contains("href=\"#step-prep-8-a-1-then-1\""));
        let movement_start = rendered.html.find("id=\"data-movement\"").unwrap();
        let movement_end =
            rendered.html[movement_start..].find("</section>").unwrap() + movement_start;
        let movement = &rendered.html[movement_start..movement_end];
        assert!(movement.contains("Unknown"));
        assert!(movement.contains("The predicate may depend on runtime state."));
        assert!(movement.contains("The destination is not identified in this packet."));
        assert!(movement.contains("request.options"));
        assert!(movement.contains("normalized.input"));
        assert!(movement.contains("payload.value"));
        assert!(movement.contains(
            "Prepared values and checks: Caller-side partial preparation [Uncertainty: No exact helper declaration is available in the packet.]"
        ));
        assert!(movement.contains("Protected try path"));
        assert!(movement.contains("Loop body"));
        assert!(
            movement.contains("The retained source does not show which external failures qualify.")
        );
        assert!(movement.contains("The loop termination condition is not retained."));
        assert!(movement.contains("href=\"#evidence-1\""));
        assert!(movement.contains("href=\"#step-prep-1-a-1-then-1\""));
        assert!(movement.contains("href=\"#step-prep-8-a-1-then-1\""));
        assert!(rendered.markdown.contains("Unknown"));
        assert!(
            rendered
                .markdown
                .contains("The predicate may depend on runtime state")
        );
        assert!(
            rendered
                .markdown
                .contains("The destination is not identified in this packet")
        );
        assert!(rendered.markdown.contains("Protected try path"));
        assert!(rendered.markdown.contains("Loop body"));
        assert!(rendered.markdown.contains(
            "Caller\\-side partial preparation \\[Uncertainty: No exact helper declaration is available in the packet\\.\\]"
        ));
        assert!(rendered.markdown.contains("external failures qualify"));
        assert!(rendered.markdown.contains("loop termination condition"));
        assert!(
            rendered
                .markdown
                .contains("[Shared mapper preparation](#preparation-a)")
        );
        assert!(
            rendered
                .markdown
                .contains("method:example.first.Mapper#map()V")
        );
        for href in rendered
            .html
            .split("href=\"#")
            .skip(1)
            .filter_map(|tail| tail.split('"').next())
        {
            assert!(
                rendered.html.contains(&format!("id=\"{href}\"")),
                "broken internal link #{href}"
            );
        }
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
        let rendered = validate_and_render(&packet, &audit(&packet), valid.clone()).unwrap();
        assert_eq!(
            rendered.html.matches("id=\"preparation-shared\"").count(),
            1
        );
        assert!(rendered.html.contains("href=\"#preparation-shared\""));

        let mut unknown_ref = valid.clone();
        unknown_ref["steps"][0]["preparationRefs"] = json!(["missing"]);
        assert!(validate_and_render(&packet, &audit(&packet), unknown_ref).is_err());

        let mut unknown_subject = valid.clone();
        unknown_subject["preparations"][0]["subjectReference"] = json!("missing-declaration");
        assert!(validate_and_render(&packet, &audit(&packet), unknown_subject).is_err());

        let mut missing_uncertainty = valid.clone();
        missing_uncertainty["preparations"][0]["summary"] =
            json!({"text":"The subject is absent.","evidence":["d1"]});
        assert!(validate_and_render(&packet, &audit(&packet), missing_uncertainty).is_err());

        let mut duplicate_id = valid.clone();
        let duplicate = duplicate_id["preparations"][0].clone();
        duplicate_id["preparations"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(validate_and_render(&packet, &audit(&packet), duplicate_id).is_err());

        let mut unreachable = valid.clone();
        unreachable["preparations"].as_array_mut().unwrap().push(json!({
            "id":"unused",
            "title":"Unreachable explanation",
            "summary":{"text":"No step links to this record.","evidence":["d1"],"uncertainty":"Its subject is not known."},
            "steps":[]
        }));
        assert!(validate_and_render(&packet, &audit(&packet), unreachable).is_err());
    }

    #[test]
    fn one_tree_preserves_decision_and_try_catch_paths_in_each_view() {
        let packet = packet();
        let rendered = validate_and_render(&packet, &audit(&packet), answer(&packet)).unwrap();

        assert!(rendered.html.contains("When the condition holds"));
        assert!(rendered.html.contains("When the condition does not hold"));
        assert!(rendered.html.contains("Protected try path"));
        assert!(rendered.html.contains("Catch / otherwise path"));
        assert_eq!(rendered.html.matches("class=\"first-match\"").count(), 0);
        assert_eq!(
            rendered
                .markdown
                .matches("| Condition | Action summary |")
                .count(),
            0
        );
        assert!(rendered.html.contains("Technical details"));
        assert!(rendered.html.contains("#summary"));
    }

    #[test]
    fn digest_evidence_labels_nonempty_steps_and_checks_are_validated() {
        let packet = packet();
        let valid = answer(&packet);
        let audit = audit(&packet);

        let mut wrong_digest = valid.clone();
        wrong_digest["packetDigest"] = json!("sha256:wrong");
        assert!(validate_and_render(&packet, &audit, wrong_digest).is_err());

        let mut unknown_label = valid.clone();
        unknown_label["summary"]["evidence"] = json!(["missing"]);
        assert!(validate_and_render(&packet, &audit, unknown_label).is_err());

        let mut empty_steps = valid.clone();
        empty_steps["steps"] = json!([]);
        assert!(validate_and_render(&packet, &audit, empty_steps).is_err());

        let mut authored_check = valid;
        authored_check["summary"]["checks"] = json!([{
            "kind":"factEquals",
            "evidence":"d1",
            "field":"name",
            "expected":"handle"
        }]);
        assert!(validate_and_render(&packet, &audit, authored_check).is_err());
    }

    #[test]
    fn packet_limitations_and_fact_tables_are_rendered_without_reauthoring() {
        let packet = packet();
        let rendered = validate_and_render(&packet, &audit(&packet), answer(&packet)).unwrap();

        assert!(rendered.html.contains("CALL_TARGET_BODY_NOT_CAPTURED"));
        assert!(rendered.html.contains("UNKNOWN_FROM_THIS_PACKET"));
        assert!(rendered.html.contains("&quot;value&quot;:0"));
        assert!(rendered.html.contains("@Min ( 0 ) int count"));
        assert!(rendered.html.contains("MAX_TRIES = 7"));
        assert!(rendered.html.contains("api.TransferPolicy"));

        // Markdown escapes punctuation in prose cells, while declaration tokens
        // are retained as code and remain verbatim.
        assert!(
            rendered
                .markdown
                .contains("CALL\\_TARGET\\_BODY\\_NOT\\_CAPTURED")
        );
        assert!(rendered.markdown.contains("UNKNOWN\\_FROM\\_THIS\\_PACKET"));
        assert!(rendered.markdown.contains("\"value\":0"));
        assert!(rendered.markdown.contains("@Min ( 0 ) int count"));
        assert!(rendered.markdown.contains("MAX_TRIES = 7"));
        assert!(rendered.markdown.contains("api\\.TransferPolicy"));
    }

    #[test]
    fn linear_else_if_chain_renders_once_and_links_to_unchanged_branch_details() {
        let packet = packet();
        let mut raw_answer = answer(&packet);
        raw_answer["steps"] = json!([{
            "kind":"decision",
            "meaning":{"text":"Condition alpha holds.","evidence":["d1"]},
            "children":[{
                "kind":"try",
                "meaning":{"text":"Try the alpha operation.","evidence":["d1"]},
                "children":[{"kind":"action","meaning":{"text":"Apply alpha.","evidence":["d1"]}}],
                "otherwise":[{"kind":"throw","meaning":{"text":"Record alpha failure.","evidence":["d1"]}}]
            }],
            "otherwise":[{
                "kind":"decision",
                "meaning":{"text":"Condition beta holds.","evidence":["d1"]},
                "children":[{
                    "kind":"loop",
                    "meaning":{"text":"Repeat beta checks while retaining the validated values and then prepare the outgoing request for the transport client.","evidence":["d1"]},
                    "to":"betaTransportRequest",
                    "children":[{"kind":"action","meaning":{"text":"Apply beta.","evidence":["d1"]}}],
                    "otherwise":[{"kind":"return","meaning":{"text":"Exit the beta loop.","evidence":["d1"]}}]
                }],
                "otherwise":[{
                    "kind":"decision",
                    "meaning":{"text":"Condition gamma holds.","evidence":["d1"]},
                    "children":[{"kind":"return","meaning":{"text":"Apply gamma.","evidence":["d1"]}}],
                    "otherwise":[{"kind":"throw","meaning":{"text":"Use the default outcome.","evidence":["d1"]}}]
                }]
            }]
        }]);
        let original_answer = raw_answer.clone();

        let rendered = validate_and_render(&packet, &audit(&packet), raw_answer).unwrap();

        assert_eq!(rendered.answer, original_answer);
        assert_eq!(rendered.html.matches("class=\"first-match\"").count(), 1);
        assert_eq!(
            rendered
                .markdown
                .matches("| Condition | Action summary |")
                .count(),
            1
        );
        let table_start = rendered.html.find("id=\"first-match-decisions\"").unwrap();
        let table_end = rendered.html[table_start..].find("</section>").unwrap() + table_start;
        let table = &rendered.html[table_start..table_end];
        assert!(table.find("Condition alpha").unwrap() < table.find("Condition beta").unwrap());
        assert!(table.find("Condition beta").unwrap() < table.find("Condition gamma").unwrap());
        assert!(table.contains("Try the alpha operation."));
        assert!(!table.contains("Apply alpha."));
        assert_eq!(
            table
                .matches("Repeat beta checks while retaining the validated values")
                .count(),
            1
        );
        assert!(table.contains("betaTransportRequest"));
        assert!(table.contains("Use the default outcome."));
        assert!(!table.contains("Record alpha failure."));
        assert!(!table.contains("Apply beta."));
        assert!(table.contains("not an executable DMN rule"));
        assert!(table.contains("reevaluation behavior"));
        assert!(rendered.html.contains("href=\"#step-1-then-1\""));
        assert!(rendered.html.contains("id=\"step-1-then-1\""));
        assert!(rendered.html.contains("href=\"#first-match-decisions\""));
        assert!(rendered.html.contains("Catch / otherwise path"));
        assert!(rendered.html.contains("Otherwise / exit path"));
        assert!(rendered.markdown.contains("#step-1-else-1-then-1"));
        assert!(rendered.markdown.contains("Record alpha failure."));
    }

    #[test]
    fn process_flat_types_and_fields_render_with_selected_language_and_cited_inventory() {
        let mut packet = json!({
            "schema":PACKET_SCHEMA,
            "profile":"process-graph-v1",
            "documentationLanguage":"ru",
            "citations":{"type-ref":"retained type","field-ref":"retained field","unused-ref":"other selected evidence"},
            "types":[{
                "reference":"type-ref",
                "symbolIdentity":"class:orders.OrderRules",
                "declarationKind":"CLASS",
                "ownerIdentity":null,
                "name":"OrderRules",
                "scope":":main",
                "superclass":"class:orders.BaseRules",
                "interfaces":[],
                "evidence":["type-ref"]
            }],
            "fields":[{
                "reference":"field-ref",
                "ownerIdentity":"class:orders.OrderRules",
                "name":"retryLimit",
                "typeDescriptor":"int",
                "modifiers":["PRIVATE"],
                "annotations":[],
                "sourceTokens":["private","int","retryLimit"],
                "evidence":["field-ref"]
            }],
            "coverage":{"evidence":["unused-ref"]}
        });
        seal(&mut packet);
        let answer = simple_answer(&packet, "field-ref");
        let original_answer = answer.clone();
        let rendered = validate_and_render(&packet, &audit(&packet), answer).unwrap();

        assert_eq!(rendered.answer, original_answer);
        assert!(rendered.html.contains("ЧЕРНОВИК / НЕ ПРОВЕРЕНО"));
        assert!(rendered.html.contains("Навигация по документу"));
        assert!(rendered.html.contains("Порядок внутреннего процесса"));
        assert!(rendered.html.contains("OrderRules"));
        assert!(rendered.html.contains("class:orders.OrderRules"));
        assert!(rendered.html.contains("retryLimit"));
        assert!(rendered.html.contains("int"));
        assert!(!rendered.html.contains("Type — · directions"));
        assert!(
            !rendered
                .markdown
                .contains("Captured DTO and annotation facts")
        );
        assert!(rendered.markdown.contains("## Типы и поля процесса"));
        let inventory = rendered
            .markdown
            .find("<summary>Полный список свидетельств</summary>")
            .unwrap();
        assert!(
            rendered.markdown[inventory..].contains("unused\\-ref"),
            "inventory section: {}",
            &rendered.markdown[inventory..]
        );
        assert!(rendered.html.contains("<details class=\"full-evidence\">"));
    }

    #[test]
    fn cited_evidence_navigates_to_verified_source_alias_range_and_escapes_source_html() {
        let mut packet = packet();
        let source_text = "class Demo {\n  String helper() { return \"<unsafe>&café\"; }\n}\n";
        let start = source_text.find("String helper").unwrap();
        let end = source_text.find("café").unwrap() + "café".len();
        packet["citations"]["source-container"] = json!("retained class source");
        packet["citations"]["method-alias"] = json!("retained method range");
        packet["methodSources"] = json!([{
            "reference":"source-container",
            "text":source_text,
            "evidence":["source-container"],
            "sourceAliases":[{
                "reference":"method-alias",
                "authority":"TRANSFORMED_SOURCE",
                "startByte":start,
                "endByte":end,
                "evidence":["method-alias"]
            }]
        }]);
        seal(&mut packet);
        let mut audit = json!({
            "schema":"codeclew-documentation-reader-packet-audit/1.0",
            "packetDigest":packet["packetDigest"],
            "records":[{
                "label":"source-container",
                "kind":"SOURCE",
                "id":"source-container-id",
                "row":{"record":{
                    "file":"src/orders/Demo.java",
                    "startLine":10,
                    "endLine":12,
                    "text":source_text
                }}
            }]
        });
        seal_audit(&mut audit);
        let mut answer = simple_answer(&packet, "method-alias");
        answer["steps"][0]["preparationRefs"] = json!(["container-context"]);
        answer["preparations"] = json!([{
            "id":"container-context",
            "title":"Surrounding declaration context",
            "summary":{
                "text":"The surrounding class source provides additional context.",
                "evidence":["source-container"],
                "uncertainty":"No separate declaration identity is supplied for this context."
            },
            "steps":[]
        }]);
        let original_answer = answer.clone();
        let rendered = validate_and_render(&packet, &audit, answer).unwrap();

        assert_eq!(rendered.answer, original_answer);
        assert!(
            rendered.markdown.contains("src/orders/Demo.java"),
            "source section: {}",
            &rendered.markdown[rendered
                .markdown
                .find("Retained source locations")
                .unwrap_or(0)..]
        );
        assert!(rendered.markdown.contains("11–11"));
        assert!(rendered.markdown.contains("10–12"));
        assert!(rendered.html.contains("src/orders/Demo.java</code>: 11–11"));
        assert!(rendered.html.contains("src/orders/Demo.java</code>: 10–12"));
        assert!(rendered.html.contains("href=\"#source-"));
        assert!(rendered.html.contains("id=\"source-"));
        assert!(rendered.html.contains("&lt;unsafe&gt;&amp;"));
        assert!(rendered.html.contains("café"));
        assert!(!rendered.html.contains("<unsafe>"));
        for href in rendered
            .html
            .split("href=\"#")
            .skip(1)
            .filter_map(|tail| tail.split('\"').next())
        {
            assert!(
                rendered.html.contains(&format!("id=\"{href}\"")),
                "broken internal link #{href}"
            );
        }
    }

    #[test]
    fn missing_source_metadata_is_reported_without_creating_a_broken_link() {
        let packet = packet();
        let rendered =
            validate_and_render(&packet, &audit(&packet), simple_answer(&packet, "d1")).unwrap();
        assert!(
            rendered
                .html
                .contains("No retained source location is available")
        );
        assert!(!rendered.html.contains("href=\"#source-"));
        assert!(!rendered.markdown.contains("](#source-"));
    }

    #[test]
    fn all_untrusted_html_text_is_escaped() {
        let mut packet = packet();
        packet["citations"]["d1"] = json!("role <img src=x onerror=alert(1)>");
        packet.as_object_mut().unwrap().remove("packetDigest");
        let packet_digest = crate::documentation::digest(&packet).unwrap();
        packet["packetDigest"] = json!(packet_digest);

        let mut answer = answer(&packet);
        answer["title"] = json!("<script>alert(1)</script>");
        answer["summary"]["text"] = json!("Use <b>captured</b> evidence.");
        let rendered = validate_and_render(&packet, &audit(&packet), answer).unwrap();

        assert!(!rendered.html.contains("<script>"));
        assert!(!rendered.html.contains("<img src=x"));
        assert!(
            rendered
                .html
                .contains("&lt;script&gt;alert(1)&lt;/script&gt;")
        );
        assert!(rendered.html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }
}
