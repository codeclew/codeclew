//! Deterministic offline rendering for a structured endpoint operation answer.
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
pub(super) const PROCESS_DIAGRAM_PUML_FILE: &str = "process-flow.puml";
pub(super) const PROCESS_DIAGRAM_SVG_FILE: &str = "process-flow.svg";
pub(super) const PROCESS_DIAGRAM_HTML_MARKER: &str = "<!--CODECLEW_PROCESS_DIAGRAM_SVG-->";
pub(super) const PROCESS_DIAGRAM_MARKDOWN_MARKER: &str = "<!--CODECLEW_PROCESS_DIAGRAM_SVG-->";
const STEP_KINDS: &[&str] = &["action", "decision", "try", "return", "throw", "loop"];
// Bump when the answer schema or generic author instruction changes materially;
// this identity is persisted on newly prepared operation Work.
pub(super) const AUTHORING_CONTRACT: &str = "codeclew-operation-draft-authoring/1.4";
pub(super) const PREVIOUS_AUTHORING_CONTRACT: &str = "codeclew-operation-draft-authoring/1.3";

pub(super) fn output_schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../../schemas/documentation/operation-answer-1.2.schema.json"
    ))
    .expect("operation answer schema is valid JSON")
}

pub(super) fn validate_and_render_draft(
    packet: &Value,
    audit: &Value,
    answer: Value,
) -> Result<RenderedAnswer, crate::error::ClewError> {
    if answer["schema"].as_str() != Some(ANSWER_SCHEMA_V1_2) {
        return Err(invalid(
            "new operation drafts require codeclew-operation-answer/1.2; the raw author response was retained",
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
    kind: GlossaryKind,
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

pub(super) struct RenderedAnswer {
    pub(super) html: String,
    pub(super) markdown: String,
    pub(super) answer: Value,
    pub(super) process_diagram: Option<ProcessDiagram>,
}

pub(super) struct ProcessDiagram {
    pub(super) puml_filename: String,
    pub(super) puml: String,
    pub(super) tree: String,
    pub(super) source_reference: Option<String>,
    pub(super) source_anchor: Option<String>,
    pub(super) has_causal_projection: bool,
}

#[derive(Clone, Copy)]
struct ReaderLabels {
    russian: bool,
    reviewed: bool,
    published: bool,
}

impl ReaderLabels {
    fn new(packet: &Value) -> Self {
        Self {
            russian: packet["documentationLanguage"].as_str() == Some("ru"),
            reviewed: false,
            published: false,
        }
    }

    fn status(self) -> &'static str {
        if self.published {
            return if self.russian {
                "ОПУБЛИКОВАНО / ОДОБРЕНО МОДЕЛЬЮ"
            } else {
                "PUBLISHED / MODEL REVIEW: APPROVED"
            };
        }
        if self.reviewed {
            return if self.russian {
                "ЧЕРНОВИК / ОДОБРЕНО МОДЕЛЬЮ / НЕ ОПУБЛИКОВАНО"
            } else {
                "DRAFT / MODEL REVIEW: APPROVED / NOT PUBLISHED"
            };
        }
        if self.russian {
            "ЧЕРНОВИК / НЕ ПРОВЕРЕНО / НЕ ОПУБЛИКОВАНО"
        } else {
            "DRAFT / UNREVIEWED / NOT PUBLISHED"
        }
    }

    fn status_note(self) -> &'static str {
        if self.reviewed {
            return if self.russian {
                "Модель проверила смысл по сохранённому снимку. Это не доказательство компилятора, актуальности исходников или исполнения."
            } else {
                "A model reviewed meaning against the saved snapshot. This is not compiler proof, current-source verification, or execution evidence."
            };
        }
        if self.russian {
            "Проверены структура и привязка меток к свидетельствам. Семантическая корректность не проверялась."
        } else {
            "Structure and evidence-label binding were validated. Semantic correctness was not reviewed."
        }
    }

    fn summary(self) -> &'static str {
        if self.russian {
            "Полное резюме"
        } else {
            "Full summary"
        }
    }

    fn ordered(self, process: bool) -> &'static str {
        match (self.russian, process) {
            (true, true) => "Пояснение внутреннего процесса",
            (true, false) => "Пояснение операции",
            (false, true) => "Internal process explanation",
            (false, false) => "Operation explanation",
        }
    }

    fn no_match(self) -> &'static str {
        if self.russian {
            "Иначе (ни одно из условий выше не выполнено)"
        } else {
            "Otherwise (none of the conditions above matched)"
        }
    }

    fn no_match_outcome(self) -> &'static str {
        if self.russian {
            "Исход для этого случая не сохранён."
        } else {
            "No outcome for this case is retained."
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
            "Техническая справка"
        } else {
            "Technical reference"
        }
    }

    fn evidence_reference(self) -> &'static str {
        if self.russian {
            "Свидетельства к пояснениям и расположения исходного кода"
        } else {
            "Claim evidence and source locations"
        }
    }

    fn evidence_count(self, count: usize) -> String {
        format!(
            "{} ({count})",
            if self.russian {
                "Свидетельства"
            } else {
                "Evidence"
            }
        )
    }

    fn packet_facts_reference(self) -> &'static str {
        if self.russian {
            "Сохранённые факты пакета"
        } else {
            "Captured packet facts"
        }
    }

    fn data_movement_reference(self) -> &'static str {
        if self.russian {
            "Таблица перемещения данных"
        } else {
            "Data movement table"
        }
    }

    fn tree_reference(self) -> &'static str {
        if self.russian {
            "Псевдокод по шагам ответа"
        } else {
            "Authored pseudocode"
        }
    }

    fn process_outline(self) -> &'static str {
        if self.russian {
            "Краткий план процесса"
        } else {
            "Process outline"
        }
    }

    fn process_projection(self) -> &'static str {
        if self.russian {
            "Локальная проекция исходного кода"
        } else {
            "Source-local process projection"
        }
    }

    fn editable_plantuml(self) -> &'static str {
        if self.russian {
            "Редактируемый PlantUML"
        } else {
            "Editable PlantUML"
        }
    }

    fn source_local_notice(self) -> &'static str {
        if self.russian {
            "Построено по сохранённому синтаксису исходного кода; это не свидетельство выполнения."
        } else {
            "Built from retained source syntax; this is not execution evidence."
        }
    }

    fn cited_by_process(self) -> &'static str {
        if self.russian {
            "На этот фрагмент ссылаются блоки процесса"
        } else {
            "Process blocks citing this excerpt"
        }
    }

    fn packet_limits_reference(self) -> &'static str {
        if self.russian {
            "Пробелы и ограничения пакета"
        } else {
            "Packet gaps and limits"
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

    fn glossary(self) -> &'static str {
        if self.russian {
            "Словарь"
        } else {
            "Glossary"
        }
    }

    fn predicate_details(self) -> &'static str {
        if self.russian {
            "Пояснения условий и проверки в исходном коде"
        } else {
            "Condition meaning and source checks"
        }
    }

    fn predicate_meaning(self) -> &'static str {
        if self.russian {
            "Смысл условия"
        } else {
            "Condition meaning"
        }
    }

    fn source_check(self) -> &'static str {
        if self.russian {
            "Проверка в исходном коде"
        } else {
            "Exact source check"
        }
    }

    fn evaluation(self) -> &'static str {
        if self.russian {
            "Как вычисляется условие"
        } else {
            "Evaluation behavior"
        }
    }

    fn evidence_mapping(self) -> &'static str {
        if self.russian {
            "Свидетельства по блокам"
        } else {
            "Evidence by block"
        }
    }

    fn technical_names(self) -> &'static str {
        if self.russian {
            "Технические имена"
        } else {
            "Technical names"
        }
    }

    fn subject_references(self) -> &'static str {
        if self.russian {
            "Ссылки на объявления"
        } else {
            "Declaration references"
        }
    }

    fn glossary_kind(self, kind: &GlossaryKind) -> &'static str {
        match (self.russian, kind) {
            (true, GlossaryKind::BusinessEntity) => "Сущность предметной области",
            (true, GlossaryKind::Request) => "Запрос",
            (true, GlossaryKind::TechnicalCarrier) => "Технический носитель",
            (true, GlossaryKind::Term) => "Термин",
            (false, GlossaryKind::BusinessEntity) => "Business entity",
            (false, GlossaryKind::Request) => "Request",
            (false, GlossaryKind::TechnicalCarrier) => "Technical carrier",
            (false, GlossaryKind::Term) => "Term",
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

    fn horizontal_table_hint(self) -> &'static str {
        if self.russian {
            "Переведите фокус на таблицу и прокрутите её по горизонтали, чтобы увидеть все столбцы."
        } else {
            "Focus this table and scroll horizontally to view all columns."
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
            "Побеждает первая подходящая строка; следующая строка относится к ветви, только если предыдущие условия не сработали. Эта таблица — справочное представление, а не исполняемое DMN-правило; она не доказывает чистоту или повторную вычислимость предикатов."
        } else {
            "The first matching row wins; a later row applies only when earlier conditions are false. This is a reading aid, not an executable DMN rule, and it does not establish predicate purity or reevaluation behavior."
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
    validate_and_render_with_review(packet, audit, answer, None, None)
}

pub(super) fn validate_and_render_reviewed(
    packet: &Value,
    audit: &Value,
    answer: Value,
    provenance: &Value,
) -> Result<RenderedAnswer, crate::error::ClewError> {
    validate_and_render_with_review(packet, audit, answer, Some(provenance), None)
}

pub(super) fn validate_and_render_published(
    packet: &Value,
    audit: &Value,
    answer: Value,
    provenance: &Value,
    publication_id: &str,
) -> Result<RenderedAnswer, crate::error::ClewError> {
    validate_and_render_with_review(
        packet,
        audit,
        answer,
        Some(provenance),
        Some(publication_id),
    )
}

fn validate_and_render_with_review(
    packet: &Value,
    audit: &Value,
    answer: Value,
    provenance: Option<&Value>,
    publication_id: Option<&str>,
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
    if packet_displayed_citation_labels(packet)? != known_labels {
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

    let mut labels = ReaderLabels::new(packet);
    labels.reviewed = provenance.is_some();
    labels.published = publication_id.is_some();
    let evidence_index = evidence_index(citations);
    let used_labels = answer_evidence_labels(&parsed);
    let root_source_references = selected_root_source_references(packet);
    let source_navigation = source_navigation(
        packet,
        audit,
        &used_labels,
        &root_source_references,
        &parsed,
    );
    let mut process_diagram = root_process_diagram(packet, &parsed, &source_navigation);
    if let (Some(diagram), Some(provenance)) = (process_diagram.as_mut(), provenance) {
        if let Some(id) = publication_id {
            diagram.puml_filename = format!("{id}.puml");
            diagram.puml = diagram.puml.replacen(
                "ROOT_SOURCE_EXCERPT=index.html#",
                &format!("ROOT_SOURCE_EXCERPT={id}.html#"),
                1,
            );
        }
        diagram.puml = diagram.puml.replacen(
            "CODECLEW_STATUS=DRAFT/UNREVIEWED/NOT_PUBLISHED",
            if publication_id.is_some() {
                "CODECLEW_STATUS=PUBLISHED/MODEL_REVIEW_APPROVED"
            } else {
                "CODECLEW_STATUS=DRAFT/MODEL_REVIEW_APPROVED/NOT_PUBLISHED"
            },
            1,
        );
        diagram.puml.push_str(&format!(
            "\n' REVIEW_RESULT_DIGEST={}\n' REVIEW_SNAPSHOT={}\n",
            plantuml_comment_text(
                provenance["reviewer"]["resultDigest"]
                    .as_str()
                    .unwrap_or_default()
            ),
            plantuml_comment_text(provenance["snapshot"].as_str().unwrap_or_default())
        ));
    }
    let html = render_html(
        packet,
        &parsed,
        citations,
        &evidence_index,
        &used_labels,
        &source_navigation,
        process_diagram.as_ref(),
        labels,
        provenance,
    );
    let markdown = render_markdown(
        packet,
        &parsed,
        citations,
        &evidence_index,
        &used_labels,
        &source_navigation,
        process_diagram.as_ref(),
        labels,
        provenance,
    );
    Ok(RenderedAnswer {
        html,
        markdown,
        answer,
        process_diagram,
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
    }
    visit(&compiler_packet, &mut labels)?;
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
    for term in &answer.glossary {
        labels.extend(term.definition.evidence.iter().cloned());
    }
    for predicate in &answer.predicates {
        labels.extend(predicate.meaning.evidence.iter().cloned());
        labels.extend(predicate.source_check.evidence.iter().cloned());
        labels.extend(predicate.evaluation.evidence.iter().cloned());
    }
    for preparation in &answer.preparations {
        labels.extend(preparation.summary.evidence.iter().cloned());
        visit(&preparation.steps, &mut labels);
    }
    labels
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

struct BlockEvidence<'a> {
    id: String,
    anchor: String,
    label: String,
    evidence: &'a [String],
}

fn block_evidence(answer: &OperationAnswer) -> Vec<BlockEvidence<'_>> {
    fn visit_steps<'a>(
        steps: &'a [OperationStep],
        path_prefix: &str,
        rows: &mut Vec<BlockEvidence<'a>>,
    ) {
        for (index, step) in steps.iter().enumerate() {
            let path = step_path(path_prefix, index + 1);
            let id = step
                .id
                .as_deref()
                .map(str::to_owned)
                .map(|id| format!("step-{id}"))
                .unwrap_or_else(|| format!("step-{path}"));
            rows.push(BlockEvidence {
                anchor: step_anchor(step, &path),
                id,
                label: step.meaning.text.clone(),
                evidence: &step.meaning.evidence,
            });
            visit_steps(&step.children, &format!("{path}-then"), rows);
            visit_steps(&step.otherwise, &format!("{path}-else"), rows);
        }
    }

    let mut rows = vec![BlockEvidence {
        id: "summary".into(),
        anchor: "summary".into(),
        label: "Summary".into(),
        evidence: &answer.summary.evidence,
    }];
    for term in &answer.glossary {
        rows.push(BlockEvidence {
            id: format!("glossary-{}", term.id),
            anchor: format!("glossary-{}", term.id),
            label: term.label.clone(),
            evidence: &term.definition.evidence,
        });
    }
    for predicate in &answer.predicates {
        rows.push(BlockEvidence {
            id: format!("predicate-{}-meaning", predicate.id),
            anchor: format!("predicate-{}", predicate.id),
            label: format!("{} · {}", predicate.label, "meaning"),
            evidence: &predicate.meaning.evidence,
        });
        rows.push(BlockEvidence {
            id: format!("predicate-{}-source-check", predicate.id),
            anchor: format!("predicate-{}", predicate.id),
            label: format!("{} · exact source check", predicate.label),
            evidence: &predicate.source_check.evidence,
        });
        rows.push(BlockEvidence {
            id: format!("predicate-{}-evaluation", predicate.id),
            anchor: format!("predicate-{}", predicate.id),
            label: format!("{} · evaluation", predicate.label),
            evidence: &predicate.evaluation.evidence,
        });
    }
    visit_steps(&answer.steps, "", &mut rows);
    for preparation in &answer.preparations {
        rows.push(BlockEvidence {
            id: format!("preparation-{}", preparation.id),
            anchor: format!("preparation-{}", preparation.id),
            label: preparation.title.clone(),
            evidence: &preparation.summary.evidence,
        });
        visit_steps(
            &preparation.steps,
            &preparation_step_prefix(&preparation.id),
            &mut rows,
        );
    }
    rows
}

fn table_scroll_start(labels: ReaderLabels, region_label: &str) -> String {
    format!(
        "<div class=\"table-scroll\" tabindex=\"0\" role=\"region\" aria-label=\"{}\"><p class=\"table-scroll-hint\">{}</p>",
        html_escape(region_label),
        html_escape(labels.horizontal_table_hint())
    )
}

fn render_block_evidence_html(
    answer: &OperationAnswer,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if answer.schema != ANSWER_SCHEMA_V1_2 {
        return String::new();
    }
    let rows = block_evidence(answer);
    let mut output = format!(
        "<section class=\"block-evidence-map\"><h3>{}</h3>{}<table><thead><tr><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
        html_escape(labels.evidence_mapping()),
        table_scroll_start(labels, labels.evidence_mapping()),
        html_escape(if labels.russian {
            "Смысловой блок"
        } else {
            "Meaning block"
        }),
        html_escape(if labels.russian {
            "Метка блока"
        } else {
            "Block ID"
        }),
        html_escape(labels.evidence())
    );
    for row in rows {
        output.push_str(&format!(
            "<tr><td>{}</td><td><a href=\"#{}\"><code>{}</code></a></td><td>{}</td></tr>",
            html_escape(&row.label),
            html_escape(&row.anchor),
            html_escape(&row.id),
            render_evidence_html(row.evidence, evidence_index, labels)
        ));
    }
    output.push_str("</tbody></table></div></section>");
    output
}

fn render_block_evidence_markdown(
    answer: &OperationAnswer,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if answer.schema != ANSWER_SCHEMA_V1_2 {
        return String::new();
    }
    let mut output = format!(
        "### {}\n\n| {} | {} | {} |\n|---|---|---|\n",
        markdown_escape(labels.evidence_mapping()),
        markdown_escape(if labels.russian {
            "Смысловой блок"
        } else {
            "Meaning block"
        }),
        markdown_escape(if labels.russian {
            "Метка блока"
        } else {
            "Block ID"
        }),
        markdown_escape(labels.evidence())
    );
    for row in block_evidence(answer) {
        output.push_str(&format!(
            "| {} | [`{}`](#{}) | {} |\n",
            markdown_table_cell(&row.label),
            markdown_code_cell(&row.id),
            row.anchor,
            markdown_evidence_cell(row.evidence, evidence_index)
        ));
    }
    output.push('\n');
    output
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

fn render_glossary_links_html(
    references: &[String],
    glossary: &[GlossaryTerm],
    labels: ReaderLabels,
) -> String {
    let links = references
        .iter()
        .filter_map(|reference| {
            let term = glossary.iter().find(|term| &term.id == reference)?;
            Some(format!(
                "<a href=\"#glossary-{}\">{}</a>",
                html_escape(reference),
                html_escape(&term.label)
            ))
        })
        .collect::<Vec<_>>();
    if links.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"glossary-links\"><strong>{}:</strong> {}</p>",
            html_escape(labels.glossary()),
            links.join(" · ")
        )
    }
}

fn render_glossary_links_markdown(
    references: &[String],
    glossary: &[GlossaryTerm],
    labels: ReaderLabels,
) -> String {
    let links = references
        .iter()
        .filter_map(|reference| {
            let term = glossary.iter().find(|term| &term.id == reference)?;
            Some(format!(
                "[{}](#glossary-{})",
                markdown_escape(&term.label),
                reference
            ))
        })
        .collect::<Vec<_>>();
    if links.is_empty() {
        String::new()
    } else {
        format!(
            "  **{}:** {}",
            markdown_escape(labels.glossary()),
            links.join(" · ")
        )
    }
}

fn render_glossary_html(
    answer: &OperationAnswer,
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if answer.glossary.is_empty() {
        return String::new();
    }
    let subjects = packet_subject_references(packet);
    let mut output = format!(
        "<section id=\"glossary\"><h2>{}</h2><div class=\"glossary-list\">",
        html_escape(labels.glossary())
    );
    for term in &answer.glossary {
        output.push_str(&format!(
            "<article class=\"glossary-term\" id=\"glossary-{}\"><h3>{} <span class=\"term-kind\">{}</span></h3>{}",
            html_escape(&term.id),
            html_escape(&term.label),
            html_escape(labels.glossary_kind(&term.kind)),
            render_claim_html(&term.definition, evidence_index, &answer.glossary, labels)
        ));
        if !term.subject_refs.is_empty() || !term.technical_names.is_empty() {
            output.push_str(&format!(
                "<details class=\"technical-term-details\"><summary>{}</summary>",
                html_escape(labels.technical_names())
            ));
            if !term.technical_names.is_empty() {
                output.push_str(&format!(
                    "<p><strong>{}:</strong> {}</p>",
                    html_escape(labels.technical_names()),
                    term.technical_names
                        .iter()
                        .map(|name| format!("<code>{}</code>", html_escape(name)))
                        .collect::<Vec<_>>()
                        .join(" · ")
                ));
            }
            if !term.subject_refs.is_empty() {
                output.push_str(&format!(
                    "<p><strong>{}:</strong> {}</p>",
                    html_escape(labels.subject_references()),
                    term.subject_refs
                        .iter()
                        .map(|reference| {
                            format!(
                                "<code>{}</code> — {}",
                                html_escape(reference),
                                html_escape(
                                    subjects
                                        .get(reference)
                                        .map(String::as_str)
                                        .unwrap_or(labels.unknown())
                                )
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            output.push_str("</details>");
        }
        output.push_str("</article>");
    }
    output.push_str("</div></section>");
    output
}

fn render_glossary_markdown(
    answer: &OperationAnswer,
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if answer.glossary.is_empty() {
        return String::new();
    }
    let subjects = packet_subject_references(packet);
    let mut output = format!(
        "<a id=\"glossary\"></a>\n## {}\n\n",
        markdown_escape(labels.glossary())
    );
    for term in &answer.glossary {
        output.push_str(&format!(
            "<a id=\"glossary-{}\"></a>\n### {} _({})_\n\n{}\n\n",
            term.id,
            markdown_escape(&term.label),
            markdown_escape(labels.glossary_kind(&term.kind)),
            render_claim_markdown(&term.definition, evidence_index, &answer.glossary, labels)
        ));
        if !term.subject_refs.is_empty() || !term.technical_names.is_empty() {
            output.push_str(&format!(
                "<details>\n<summary>{}</summary>\n\n",
                markdown_escape(labels.technical_names())
            ));
            if !term.technical_names.is_empty() {
                output.push_str(&format!(
                    "**{}:** {}\n\n",
                    markdown_escape(labels.technical_names()),
                    term.technical_names
                        .iter()
                        .map(|name| format!("`{}`", markdown_code_cell(name)))
                        .collect::<Vec<_>>()
                        .join(" · ")
                ));
            }
            if !term.subject_refs.is_empty() {
                output.push_str(&format!(
                    "**{}:** {}\n\n",
                    markdown_escape(labels.subject_references()),
                    term.subject_refs
                        .iter()
                        .map(|reference| format!(
                            "`{}` — {}",
                            markdown_code_cell(reference),
                            markdown_escape(
                                subjects
                                    .get(reference)
                                    .map(String::as_str)
                                    .unwrap_or(labels.unknown())
                            )
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            output.push_str("</details>\n\n");
        }
    }
    output
}

fn render_predicates_html(
    answer: &OperationAnswer,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if answer.predicates.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<section id=\"semantic-predicates\"><h3>{}</h3>",
        html_escape(labels.predicate_details())
    );
    for predicate in &answer.predicates {
        output.push_str(&format!(
            "<details class=\"predicate-definition\" id=\"predicate-{}\"><summary>{}</summary><div class=\"predicate-meaning\"><strong>{}:</strong>{}</div><div class=\"predicate-source-check\"><strong>{}:</strong>{}</div><div class=\"predicate-evaluation\"><strong>{}:</strong>{}</div></details>",
            html_escape(&predicate.id),
            html_escape(&predicate.label),
            html_escape(labels.predicate_meaning()),
            render_claim_html(&predicate.meaning, evidence_index, &answer.glossary, labels),
            html_escape(labels.source_check()),
            render_claim_html(&predicate.source_check, evidence_index, &answer.glossary, labels),
            html_escape(labels.evaluation()),
            render_claim_html(&predicate.evaluation, evidence_index, &answer.glossary, labels)
        ));
    }
    output.push_str("</section>");
    output
}

fn render_predicates_markdown(
    answer: &OperationAnswer,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    if answer.predicates.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "<a id=\"semantic-predicates\"></a>\n### {}\n\n",
        markdown_escape(labels.predicate_details())
    );
    for predicate in &answer.predicates {
        output.push_str(&format!(
            "<details>\n<summary id=\"predicate-{}\">{}</summary>\n\n**{}:** {}\n\n**{}:** {}\n\n**{}:** {}\n\n</details>\n\n",
            predicate.id,
            markdown_escape(&predicate.label),
            markdown_escape(labels.predicate_meaning()),
            render_claim_markdown(&predicate.meaning, evidence_index, &answer.glossary, labels),
            markdown_escape(labels.source_check()),
            render_claim_markdown(&predicate.source_check, evidence_index, &answer.glossary, labels),
            markdown_escape(labels.evaluation()),
            render_claim_markdown(&predicate.evaluation, evidence_index, &answer.glossary, labels)
        ));
    }
    output
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
        if answer.schema != ANSWER_SCHEMA_V1_2
            && let Some(reference) = preparation.subject_reference.as_deref()
        {
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
            &answer.glossary,
            labels,
        ));
        if !preparation.steps.is_empty() {
            output.push_str(&render_html_steps(
                &preparation.steps,
                answer,
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
            html_escape(&preparation.id),
            markdown_escape(&preparation.title)
        ));
        if answer.schema != ANSWER_SCHEMA_V1_2
            && let Some(reference) = preparation.subject_reference.as_deref()
        {
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
            &answer.glossary,
            labels,
        ));
        output.push_str("\n\n");
        if !preparation.steps.is_empty() {
            output.push_str(&render_markdown_steps(
                &preparation.steps,
                answer,
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

fn render_preparation_technical_html(
    answer: &OperationAnswer,
    packet: &Value,
    labels: ReaderLabels,
) -> String {
    if answer.schema != ANSWER_SCHEMA_V1_2 {
        return String::new();
    }
    let subjects = packet_subject_references(packet);
    let mut output = String::new();
    for preparation in &answer.preparations {
        let mut step_rows = String::new();
        append_preparation_metadata_rows_html(
            &preparation.steps,
            &preparation.id,
            &mut step_rows,
            labels,
        );
        let subject = preparation.subject_reference.as_deref().map(|reference| {
            format!(
                "<p class=\"step-meta\"><strong>{}:</strong> <code>{}</code> — {}</p>",
                html_escape(labels.subject()),
                html_escape(reference),
                html_escape(
                    subjects
                        .get(reference)
                        .map(String::as_str)
                        .unwrap_or(labels.unknown())
                )
            )
        });
        if subject.is_none() && step_rows.is_empty() {
            continue;
        }
        if output.is_empty() {
            output.push_str(&format!(
                "<section class=\"preparation-technical-reference\"><h3>{}</h3>",
                html_escape(if labels.russian {
                    "Технические ссылки подготовок"
                } else {
                    "Preparation source and technical metadata"
                })
            ));
        }
        output.push_str(&format!(
            "<article class=\"preparation-metadata\"><h4>{}</h4>{}{}</article>",
            html_escape(&preparation.title),
            subject.unwrap_or_default(),
            if step_rows.is_empty() {
                String::new()
            } else {
                format!("<ul>{step_rows}</ul>")
            }
        ));
    }
    if !output.is_empty() {
        output.push_str("</section>");
    }
    output
}

fn append_preparation_metadata_rows_html(
    steps: &[OperationStep],
    path_prefix: &str,
    output: &mut String,
    labels: ReaderLabels,
) {
    for (index, step) in steps.iter().enumerate() {
        let path = step_path(path_prefix, index + 1);
        if step.from.is_some() || step.to.is_some() || step.interaction.is_some() {
            let block = step.id.as_deref().unwrap_or(&path);
            output.push_str(&format!(
                "<li><code>{}</code>{}</li>",
                html_escape(block),
                render_step_metadata_html(step, labels)
            ));
        }
        append_preparation_metadata_rows_html(
            &step.children,
            &format!("{path}-then"),
            output,
            labels,
        );
        append_preparation_metadata_rows_html(
            &step.otherwise,
            &format!("{path}-else"),
            output,
            labels,
        );
    }
}

fn render_preparation_technical_markdown(
    answer: &OperationAnswer,
    packet: &Value,
    labels: ReaderLabels,
) -> String {
    if answer.schema != ANSWER_SCHEMA_V1_2 {
        return String::new();
    }
    let subjects = packet_subject_references(packet);
    let mut rows = String::new();
    for preparation in &answer.preparations {
        let mut preparation_rows = String::new();
        if let Some(reference) = preparation.subject_reference.as_deref() {
            preparation_rows.push_str(&format!(
                "- **{}:** `{}` — {}\n",
                markdown_escape(labels.subject()),
                markdown_code_cell(reference),
                markdown_escape(
                    subjects
                        .get(reference)
                        .map(String::as_str)
                        .unwrap_or(labels.unknown())
                )
            ));
        }
        append_preparation_metadata_rows_markdown(
            &preparation.steps,
            &preparation.id,
            &mut preparation_rows,
            labels,
        );
        if !preparation_rows.is_empty() {
            rows.push_str(&format!(
                "### {}\n\n{}\n",
                markdown_escape(&preparation.title),
                preparation_rows
            ));
        }
    }
    if rows.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<details class=\"preparation-technical-reference\"><summary>{}</summary>\n\n{}\n</details>\n\n",
            markdown_escape(if labels.russian {
                "Технические ссылки подготовок"
            } else {
                "Preparation source and technical metadata"
            }),
            rows
        )
    }
}

fn append_preparation_metadata_rows_markdown(
    steps: &[OperationStep],
    path_prefix: &str,
    output: &mut String,
    labels: ReaderLabels,
) {
    for (index, step) in steps.iter().enumerate() {
        let path = step_path(path_prefix, index + 1);
        let metadata = markdown_step_metadata(step, labels);
        if !metadata.is_empty() {
            let block = step.id.as_deref().unwrap_or(&path);
            output.push_str(&format!(
                "- **{}:**{}\n",
                markdown_code_cell(block),
                metadata
            ));
        }
        append_preparation_metadata_rows_markdown(
            &step.children,
            &format!("{path}-then"),
            output,
            labels,
        );
        append_preparation_metadata_rows_markdown(
            &step.otherwise,
            &format!("{path}-else"),
            output,
            labels,
        );
    }
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
        "<section id=\"data-movement\"><h2>{}</h2><p class=\"muted\">{}</p>{}<table><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
        html_escape(labels.data_movement()),
        html_escape(labels.data_movement_note()),
        table_scroll_start(labels, labels.data_movement()),
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
        let path = step_anchor(row.step, &row.path);
        output.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td><a href=\"#{}\">{}</a> {}</td><td>{}</td><td>{}</td></tr>",
            context,
            html_escape(row.step.from.as_deref().unwrap_or(labels.unknown())),
            html_escape(row.step.to.as_deref().unwrap_or(labels.unknown())),
            html_escape(&path),
            html_escape(&row.step.kind),
            html_escape(&row.step.meaning.text),
            render_evidence_html(&row.step.meaning.evidence, evidence_index, labels),
            row.step.meaning.uncertainty.as_deref().map(html_escape).map(|value| {
                format!("<strong>{}:</strong> {value}", html_escape(labels.uncertainty()))
            }).unwrap_or_else(|| "—".into())
        ));
    }
    output.push_str("</tbody></table></div></section>");
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
        let path = step_anchor(row.step, &row.path);
        output.push_str(&format!(
            "| {} | {} | {} | [{}](#{path}) — {} | {} | {} |\n",
            context,
            markdown_table_cell(row.step.from.as_deref().unwrap_or(labels.unknown())),
            markdown_table_cell(row.step.to.as_deref().unwrap_or(labels.unknown())),
            markdown_table_cell(&row.step.kind),
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
    body_evidence_locations: BTreeMap<String, Vec<usize>>,
    /// Claim affinity excludes implicit body ranges attached to a containing SOURCE.
    claim_evidence_locations: BTreeMap<String, Vec<usize>>,
    citing_blocks: BTreeMap<usize, Vec<CitedBlock>>,
}

#[derive(Clone)]
struct CitedBlock {
    anchor: String,
    label: String,
}

fn selected_root_source_references(packet: &Value) -> Vec<String> {
    if packet["profile"] != "process-graph-v1" {
        return Vec::new();
    }
    let Some(root_id) = packet["root"]["methodId"].as_str() else {
        return Vec::new();
    };
    let matches: Vec<_> = packet["methods"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|method| method["id"].as_str() == Some(root_id))
        .collect();
    if matches.len() != 1 {
        return Vec::new();
    }
    let Some(reference) = matches[0]["body"]["sourceReference"].as_str() else {
        return Vec::new();
    };
    let mut references = BTreeSet::from([reference.to_owned()]);
    for source in packet["methodSources"].as_array().into_iter().flatten() {
        if source["reference"].as_str() != Some(reference) {
            continue;
        }
        references.extend(strings(&source["evidence"]));
        for alias in source["sourceAliases"].as_array().into_iter().flatten() {
            if let Some(alias_reference) = alias["reference"].as_str() {
                references.insert(alias_reference.to_owned());
            }
            references.extend(strings(&alias["evidence"]));
        }
    }
    references.into_iter().collect()
}

fn source_navigation(
    packet: &Value,
    audit: &Value,
    used: &BTreeSet<String>,
    root_references: &[String],
    answer: &OperationAnswer,
) -> SourceNavigation {
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

    let navigation_labels: BTreeSet<_> = used
        .iter()
        .cloned()
        .chain(root_references.iter().cloned())
        .collect();
    let mut locations_by_evidence = BTreeMap::<String, BTreeSet<SourceLocation>>::new();
    for label in &navigation_labels {
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
                if navigation_labels.contains(label) {
                    locations_by_evidence
                        .entry(label.to_owned())
                        .or_default()
                        .insert(location.clone());
                }
            }
        }
    }

    // Keep implicit body audit navigation separate from the exact source ranges
    // cited by prose. A containing class SOURCE also supplies the root diagram,
    // but citing that class does not cite every method body nested within it.
    let mut claim_locations_by_evidence = locations_by_evidence.clone();
    let mut callable_claim_locations = BTreeMap::<String, BTreeSet<SourceLocation>>::new();
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
            for label in evidence
                .into_iter()
                .filter(|label| navigation_labels.contains(label))
            {
                locations_by_evidence
                    .entry(label.clone())
                    .or_default()
                    .insert(location.clone());
                if label != source_reference {
                    callable_claim_locations
                        .entry(label)
                        .or_default()
                        .insert(location.clone());
                }
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
    for label in &navigation_labels {
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
    // An explicit callable declaration binds its claim to the verified method
    // range instead of a broad sourceIds association with the containing class.
    claim_locations_by_evidence.extend(callable_claim_locations);
    let indexed = |mapping: BTreeMap<String, BTreeSet<SourceLocation>>| {
        mapping
            .into_iter()
            .filter(|(label, _)| navigation_labels.contains(label))
            .map(|(label, candidates)| {
                let indices = candidates
                    .iter()
                    .filter_map(|location| location_indices.get(location).copied())
                    .collect();
                (label, indices)
            })
            .collect()
    };
    let mut navigation = SourceNavigation {
        locations,
        body_evidence_locations: indexed(locations_by_evidence),
        claim_evidence_locations: indexed(claim_locations_by_evidence),
        citing_blocks: BTreeMap::new(),
    };
    navigation.citing_blocks = source_citing_blocks(answer, &navigation);
    navigation
}

fn source_citing_blocks(
    answer: &OperationAnswer,
    navigation: &SourceNavigation,
) -> BTreeMap<usize, Vec<CitedBlock>> {
    fn add_claim(
        anchor: String,
        label: String,
        evidence: &[String],
        navigation: &SourceNavigation,
        cited: &mut BTreeMap<usize, BTreeMap<String, CitedBlock>>,
    ) {
        let block = CitedBlock { anchor, label };
        for reference in evidence {
            if let Some(locations) = navigation.claim_evidence_locations.get(reference) {
                for location in locations {
                    cited
                        .entry(*location)
                        .or_default()
                        .entry(block.anchor.clone())
                        .or_insert_with(|| block.clone());
                }
            }
        }
    }

    fn add_steps(
        steps: &[OperationStep],
        path_prefix: &str,
        navigation: &SourceNavigation,
        cited: &mut BTreeMap<usize, BTreeMap<String, CitedBlock>>,
    ) {
        for (index, step) in steps.iter().enumerate() {
            let path = step_path(path_prefix, index + 1);
            add_claim(
                step_anchor(step, &path),
                format!("{}: {}", step.kind, step.meaning.text),
                &step.meaning.evidence,
                navigation,
                cited,
            );
            add_steps(&step.children, &format!("{path}-then"), navigation, cited);
            add_steps(&step.otherwise, &format!("{path}-else"), navigation, cited);
        }
    }

    let mut cited = BTreeMap::<usize, BTreeMap<String, CitedBlock>>::new();
    add_claim(
        "summary".into(),
        "Summary".into(),
        &answer.summary.evidence,
        navigation,
        &mut cited,
    );
    add_steps(&answer.steps, "", navigation, &mut cited);
    for predicate in &answer.predicates {
        for claim in [
            &predicate.meaning,
            &predicate.source_check,
            &predicate.evaluation,
        ] {
            add_claim(
                format!("predicate-{}", predicate.id),
                predicate.label.clone(),
                &claim.evidence,
                navigation,
                &mut cited,
            );
        }
    }
    for preparation in &answer.preparations {
        add_claim(
            format!("preparation-{}", preparation.id),
            preparation.title.clone(),
            &preparation.summary.evidence,
            navigation,
            &mut cited,
        );
        add_steps(
            &preparation.steps,
            &preparation_step_prefix(&preparation.id),
            navigation,
            &mut cited,
        );
    }
    cited
        .into_iter()
        .map(|(index, blocks)| (index, blocks.into_values().collect()))
        .collect()
}

fn root_process_diagram(
    packet: &Value,
    answer: &OperationAnswer,
    navigation: &SourceNavigation,
) -> Option<ProcessDiagram> {
    if packet["profile"] != "process-graph-v1" {
        return None;
    }

    use super::process_flow::{Projection, ProjectionStep};

    let root_id = packet["root"]["methodId"].as_str();
    let root_symbol = packet["root"]["symbolIdentity"]
        .as_str()
        .unwrap_or("selected process root");
    let mut source_reference = None;
    let mut range = None;
    let mut source_text = None;
    let mut gap = None;

    let root_methods: Vec<_> = packet["methods"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|method| root_id.is_some_and(|root_id| method["id"].as_str() == Some(root_id)))
        .collect();
    let method = match root_id {
        None | Some("") => {
            gap = Some("ROOT_METHOD_ID_MISSING".to_owned());
            None
        }
        Some(_) if root_methods.is_empty() => {
            gap = Some("ROOT_METHOD_MATCH_MISSING".to_owned());
            None
        }
        Some(_) if root_methods.len() > 1 => {
            gap = Some("ROOT_METHOD_MATCH_AMBIGUOUS".to_owned());
            None
        }
        Some(_) => root_methods.first().copied(),
    };

    if let Some(method) = method {
        let method_symbol = method["symbolIdentity"].as_str();
        if method_symbol != Some(root_symbol) {
            gap = Some("ROOT_METHOD_IDENTITY_MISMATCH".into());
        } else {
            source_reference = method["body"]["sourceReference"]
                .as_str()
                .map(str::to_owned);
            range = method["body"]["startByte"]
                .as_u64()
                .and_then(|start| usize::try_from(start).ok())
                .zip(
                    method["body"]["endByte"]
                        .as_u64()
                        .and_then(|end| usize::try_from(end).ok()),
                );
            if source_reference.is_none() {
                gap = Some("ROOT_SOURCE_REFERENCE_MISSING".into());
            } else if range.is_none() {
                gap = Some("ROOT_SOURCE_BODY_RANGE_MISSING".into());
            }
        }
    }

    if gap.is_none()
        && let Some(reference) = source_reference.as_deref()
    {
        let sources: Vec<_> = packet["methodSources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|source| source["reference"].as_str() == Some(reference))
            .collect();
        match sources.as_slice() {
            [] => gap = Some("ROOT_SOURCE_METHOD_RECORD_MISSING".into()),
            [source] => match source["text"].as_str() {
                Some(text) => source_text = Some(text),
                None => gap = Some("ROOT_SOURCE_TEXT_MISSING".into()),
            },
            _ => gap = Some("ROOT_SOURCE_METHOD_RECORD_AMBIGUOUS".into()),
        }
    }

    let source_excerpt = source_text
        .zip(range)
        .and_then(|(text, (start, end))| text.get(start..end).map(str::to_owned));
    if gap.is_none() && source_excerpt.is_none() {
        gap = Some("ROOT_SOURCE_BODY_RANGE_INVALID".into());
    }

    let source_location_index = source_reference
        .as_deref()
        .zip(source_excerpt.as_deref())
        .and_then(|(reference, excerpt)| {
            let matches: Vec<_> = navigation
                .body_evidence_locations
                .get(reference)?
                .iter()
                .copied()
                .filter(|index| navigation.locations[*index].excerpt == excerpt)
                .collect();
            if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            }
        });
    if gap.is_none() && source_location_index.is_none() {
        gap = Some("ROOT_SOURCE_AUDIT_BINDING_UNAVAILABLE".into());
    }

    let projection = if let Some(reason) = gap {
        let mut projection = Projection::source(root_symbol, root_symbol.to_owned(), Vec::new());
        projection.noncausal = Some(reason);
        projection
    } else {
        let text = source_text.expect("a successful source binding has retained text");
        let expected_range = range.expect("a successful source binding has a body range");
        match super::source_steps::method_body(text, root_symbol) {
            Some(found_range) if found_range == expected_range => {
                super::source_steps::projection(text, root_symbol).unwrap_or_else(|| {
                    let mut projection =
                        Projection::source(root_symbol, root_symbol.to_owned(), Vec::new());
                    projection.noncausal = Some("SOURCE_ROOT_PROJECTION_UNSUPPORTED".into());
                    projection
                })
            }
            Some(_) => {
                let mut projection =
                    Projection::source(root_symbol, root_symbol.to_owned(), Vec::new());
                projection.noncausal = Some("ROOT_SOURCE_RANGE_MISMATCH".into());
                projection
            }
            None => {
                let mut projection =
                    Projection::source(root_symbol, root_symbol.to_owned(), Vec::new());
                projection.noncausal =
                    Some("ROOT_SOURCE_METHOD_SIGNATURE_UNMATCHED_OR_AMBIGUOUS".into());
                projection
            }
        }
    };
    let title = format!("Source-local projection · {}", answer.title);
    let validated = super::process_flow::validate_projection(projection);
    let has_causal_projection = validated
        .projection
        .steps
        .iter()
        .any(|step| !matches!(step, ProjectionStep::Gap(_)));
    let rendered = super::process_flow::render_validated(validated, &title)
        .expect("source root projection always contains a causal step or explicit gap");

    let source_anchor = source_location_index.map(|index| format!("source-{}", index + 1));
    let provenance = format!(
        "' CODECLEW_STATUS=DRAFT/UNREVIEWED/NOT_PUBLISHED\n' ORIGIN=SOURCE_LOCAL_NOT_EXECUTION_EVIDENCE\n' PACKET_DIGEST={}\n' ROOT_METHOD_ID={}\n' ROOT_SOURCE_REFERENCE={}\n' ROOT_SOURCE_EXCERPT={}\n",
        packet["packetDigest"].as_str().unwrap_or("unavailable"),
        plantuml_comment_text(root_id.unwrap_or("unavailable")),
        plantuml_comment_text(source_reference.as_deref().unwrap_or("unavailable")),
        plantuml_comment_text(
            source_anchor
                .as_deref()
                .map(|anchor| format!("index.html#{anchor}"))
                .as_deref()
                .unwrap_or("unavailable")
        )
    );
    Some(ProcessDiagram {
        puml_filename: PROCESS_DIAGRAM_PUML_FILE.into(),
        puml: format!("{provenance}{}", rendered.puml),
        tree: rendered.tree,
        source_reference,
        source_anchor,
        has_causal_projection,
    })
}

fn render_process_diagram_html(
    diagram: &ProcessDiagram,
    _navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    let source_link = match (
        diagram.source_reference.as_deref(),
        diagram.source_anchor.as_deref(),
    ) {
        (Some(reference), Some(anchor)) => format!(
            "<p><strong>{}:</strong> <a href=\"#{}\"><code>{}</code></a></p>",
            html_escape(labels.source_link()),
            html_escape(anchor),
            html_escape(reference)
        ),
        (Some(reference), None) => format!(
            "<p><strong>{}:</strong> <code>{}</code></p>",
            html_escape(labels.source_link()),
            html_escape(reference)
        ),
        _ => String::new(),
    };
    let causal_note = if diagram.has_causal_projection {
        String::new()
    } else if labels.russian {
        "<p class=\"muted\">Причинный путь не подтверждён; диаграмма содержит только пробел свидетельств.</p>"
            .to_owned()
    } else {
        "<p class=\"muted\">A causal path was not established; the diagram contains an evidence gap only.</p>"
            .to_owned()
    };
    format!(
        "<section id=\"source-process-projection\"><h2>{}</h2><p class=\"muted\">{}</p>{}<details><summary>{}</summary><pre><code>{}</code></pre></details><p><a href=\"{}\">{}</a></p>{}{}</section>",
        html_escape(labels.process_projection()),
        html_escape(labels.source_local_notice()),
        causal_note,
        html_escape(if labels.russian {
            "Проекция исходного кода"
        } else {
            "Parsed source outline"
        }),
        html_escape(&diagram.tree),
        html_escape(&diagram.puml_filename),
        html_escape(labels.editable_plantuml()),
        source_link,
        if labels.published {
            "<p>Diagram SVG was not rendered; the source tree and PlantUML are retained.</p>"
        } else {
            PROCESS_DIAGRAM_HTML_MARKER
        }
    )
}

fn render_process_diagram_markdown(
    diagram: &ProcessDiagram,
    _navigation: &SourceNavigation,
    labels: ReaderLabels,
) -> String {
    let mut output = format!(
        "<a id=\"source-process-projection\"></a>\n## {}\n\n{}\n\n",
        markdown_escape(labels.process_projection()),
        markdown_escape(labels.source_local_notice())
    );
    output.push_str(&format!(
        "### {}\n\n{}\n\n[{}]({})\n\n",
        markdown_escape(if labels.russian {
            "Проекция исходного кода"
        } else {
            "Parsed source outline"
        }),
        markdown_code_block(&diagram.tree),
        markdown_escape(labels.editable_plantuml()),
        diagram.puml_filename
    ));
    if let (Some(reference), Some(anchor)) = (
        diagram.source_reference.as_deref(),
        diagram.source_anchor.as_deref(),
    ) {
        output.push_str(&format!(
            "**{}:** [`{}`](#{anchor})\n\n",
            markdown_escape(labels.source_link()),
            markdown_code_cell(reference)
        ));
    }
    if !diagram.has_causal_projection {
        output.push_str(&format!(
            "_{}_\n\n",
            markdown_escape(if labels.russian {
                "Причинный путь не подтверждён; диаграмма содержит только пробел свидетельств."
            } else {
                "A causal path was not established; the diagram contains an evidence gap only."
            })
        ));
    }
    output.push_str(if labels.published {
        "Diagram SVG was not rendered; the source tree and PlantUML are retained."
    } else {
        PROCESS_DIAGRAM_MARKDOWN_MARKER
    });
    output.push_str("\n\n");
    output
}

pub(super) fn diagram_svg_status_html(available: bool, reason: Option<&str>) -> String {
    if available {
        format!(
            "<p><a href=\"{}\">Rendered SVG</a></p>",
            PROCESS_DIAGRAM_SVG_FILE
        )
    } else {
        format!(
            "<p class=\"muted\">SVG unavailable: {}</p>",
            html_escape(reason.unwrap_or("PlantUML did not produce an SVG."))
        )
    }
}

pub(super) fn diagram_svg_status_markdown(available: bool, reason: Option<&str>) -> String {
    if available {
        format!("[Rendered SVG]({})", PROCESS_DIAGRAM_SVG_FILE)
    } else {
        format!(
            "_{}: {}_",
            markdown_escape("SVG unavailable"),
            markdown_escape(reason.unwrap_or("PlantUML did not produce an SVG."))
        )
    }
}

fn plantuml_comment_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => ' ',
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect()
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

const HASH_NAVIGATION_SCRIPT: &str = r##"(()=>{
  const revealHashTarget=()=>{
    const hash=window.location.hash;
    if(!hash||hash==="#")return;
    let id;
    try{id=decodeURIComponent(hash.slice(1));}catch(_error){return;}
    const target=document.getElementById(id);
    if(!target)return;
    for(let node=target;node;node=node.parentElement){
      if(node.tagName==="DETAILS")node.open=true;
    }
  };
  window.addEventListener("hashchange",revealHashTarget);
  document.addEventListener("click",event=>{
    const link=event.target.closest&&event.target.closest('a[href^="#"]');
    if(link&&link.hash===window.location.hash)window.setTimeout(revealHashTarget,0);
  });
  revealHashTarget();
})();"##;

// Keep separate inputs visible because each renderer consumes distinct source sections.
fn render_review_provenance(provenance: &Value, html: bool) -> String {
    let mut rows = vec![
        ("Saved source snapshot", provenance["snapshot"].clone()),
        ("Author run", provenance["sourceRun"].clone()),
        ("Reviewer run", provenance["reviewRun"].clone()),
        ("Packet digest", provenance["packetDigest"].clone()),
        ("Answer digest", provenance["answerDigest"].clone()),
        ("Coverage digest", provenance["coverageDigest"].clone()),
    ];
    for (role, caption) in [
        ("author", "Declared author model"),
        ("reviewer", "Declared reviewer model"),
    ] {
        rows.push((caption, provenance[role]["model"].clone()));
        rows.push((
            if role == "author" {
                "Author input digest"
            } else {
                "Reviewer input digest"
            },
            provenance[role]["inputDigest"].clone(),
        ));
        rows.push((
            if role == "author" {
                "Author result digest"
            } else {
                "Reviewer result digest"
            },
            provenance[role]["resultDigest"].clone(),
        ));
    }
    let mut output: String = if html {
        "<section id=\"meaning-review\"><details><summary>Saved model review: provenance and limitations</summary><dl>".into()
    } else {
        "## Saved model review\n\n".into()
    };
    for (caption, value) in rows {
        let value = value.as_str().unwrap_or_default();
        if html {
            output.push_str(&format!(
                "<dt>{}</dt><dd><code>{}</code></dd>",
                html_escape(caption),
                html_escape(value)
            ));
        } else {
            output.push_str(&format!(
                "- **{}:** {}\n",
                markdown_escape(caption),
                markdown_escape(value)
            ));
        }
    }
    if html {
        output.push_str("</dl><h3>Reviewer limitations</h3><ul>");
    } else {
        output.push_str("\n### Reviewer limitations\n\n");
    }
    for limitation in provenance["limitations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if html {
            output.push_str(&format!("<li>{}</li>", html_escape(limitation)));
        } else {
            output.push_str(&format!("- {}\n", markdown_escape(limitation)));
        }
    }
    if html {
        output.push_str("</ul><h3>Reviewer findings</h3><ul>");
    } else {
        output.push_str("\n### Reviewer findings\n\n");
    }
    for issue in provenance["issues"].as_array().into_iter().flatten() {
        let text = format!(
            "{}: {} ({})",
            issue["severity"].as_str().unwrap_or_default(),
            issue["reason"].as_str().unwrap_or_default(),
            issue["block"].as_str().unwrap_or_default()
        );
        if html {
            output.push_str(&format!("<li>{}</li>", html_escape(&text)));
        } else {
            output.push_str(&format!("- {}\n", markdown_escape(&text)));
        }
    }
    if html {
        output.push_str("</ul></details></section>");
    } else {
        output.push('\n');
    }
    output
}

#[allow(clippy::too_many_arguments)]
fn render_html(
    packet: &Value,
    answer: &OperationAnswer,
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    source_navigation: &SourceNavigation,
    process_diagram: Option<&ProcessDiagram>,
    labels: ReaderLabels,
    provenance: Option<&Value>,
) -> String {
    let language = packet["documentationLanguage"].as_str().unwrap_or("en");
    let preparation_titles = preparation_titles(answer);
    let preparations =
        render_preparations_html(answer, packet, evidence_index, &preparation_titles, labels);
    let movement_rows = data_movement_rows(answer, labels);
    let data_movement = render_data_movement_html(&movement_rows, evidence_index, labels);
    let packet_facts = render_packet_fact_tables_html(packet, evidence_index, labels);
    let cited_evidence = render_evidence_index_html(
        citations,
        evidence_index,
        used_labels,
        source_navigation,
        labels,
    );
    let source_locations = render_source_locations_html(source_navigation, labels);
    let full_inventory = render_full_inventory_html(citations, evidence_index, used_labels, labels);
    let block_evidence = render_block_evidence_html(answer, evidence_index, labels);
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
    if let Some(provenance) = provenance {
        html.push_str(&render_review_provenance(provenance, true));
    }
    let mut nav = Vec::new();
    if !answer.glossary.is_empty() {
        nav.push(("glossary", labels.glossary()));
    }
    nav.push((
        "ordered-behavior",
        labels.ordered(packet["profile"] == "process-graph-v1"),
    ));
    nav.push(("process-outline", labels.process_outline()));
    nav.push(("authored-pseudocode", labels.tree_reference()));
    if process_diagram.is_some() {
        nav.push(("source-process-projection", labels.process_projection()));
    }
    if !preparations.is_empty() {
        nav.push(("preparations", labels.preparations()));
    }
    nav.push(("summary", labels.summary()));
    if !answer.predicates.is_empty() {
        nav.push(("semantic-predicates", labels.predicate_details()));
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
    html.push_str(&render_uncertainties_html(answer, labels));
    html.push_str(&render_process_outline_html(answer, labels));
    html.push_str(&render_glossary_html(
        answer,
        packet,
        evidence_index,
        labels,
    ));
    html.push_str(&format!(
        "<section id=\"authored-pseudocode\"><h2>{}</h2><p class=\"muted\">{}</p><figure class=\"step-tree\">{}</figure></section>",
        html_escape(labels.tree_reference()),
        html_escape(if labels.russian {
            "Структура и порядок ветвей повторяют переданные шаги ответа."
        } else {
            "Structure and branch order follow the supplied answer steps."
        }),
        render_pseudocode_html(&answer.steps, answer, labels)
    ));
    if let Some(diagram) = process_diagram {
        html.push_str(&render_process_diagram_html(
            diagram,
            source_navigation,
            labels,
        ));
    }
    html.push_str("<section id=\"ordered-behavior\"><h2>");
    html.push_str(&html_escape(
        labels.ordered(packet["profile"] == "process-graph-v1"),
    ));
    html.push_str("</h2>");
    html.push_str(&render_html_steps(
        &answer.steps,
        answer,
        evidence_index,
        false,
        "",
        &preparation_titles,
        labels,
    ));
    html.push_str(&render_predicates_html(answer, evidence_index, labels));
    html.push_str("</section>");
    html.push_str(&preparations);
    html.push_str(&render_summary_details_html(answer, evidence_index, labels));
    html.push_str(&format!(
        "<details class=\"technical-details\"><summary>{}</summary><p class=\"packet-digest\">{}: <code>{}</code></p>",
        html_escape(labels.technical_details()),
        html_escape(if labels.russian { "Хеш пакета" } else { "Packet digest" }),
        html_escape(&answer.packet_digest)
    ));
    html.push_str(&render_preparation_technical_html(answer, packet, labels));
    if !packet_facts.is_empty() {
        html.push_str(&format!(
            "<details class=\"packet-facts-reference\"><summary>{}</summary>{}</details>",
            html_escape(labels.packet_facts_reference()),
            packet_facts
        ));
    }
    if !data_movement.is_empty() {
        html.push_str(&format!(
            "<details class=\"data-movement-reference\"><summary>{}</summary>{}</details>",
            html_escape(labels.data_movement_reference()),
            data_movement
        ));
    }
    if !block_evidence.is_empty()
        || !cited_evidence.is_empty()
        || !source_locations.is_empty()
        || !full_inventory.is_empty()
    {
        html.push_str(&format!(
            "<details class=\"evidence-reference\"><summary>{}</summary>{}{}{}{}</details>",
            html_escape(labels.evidence_reference()),
            block_evidence,
            cited_evidence,
            source_locations,
            full_inventory
        ));
    }
    html.push_str(&format!(
        "<details class=\"packet-limits-reference\"><summary>{}</summary>{}</details>",
        html_escape(labels.packet_limits_reference()),
        render_packet_limits_html(packet, labels)
    ));
    html.push_str("</details></main><script>");
    html.push_str(HASH_NAVIGATION_SCRIPT);
    html.push_str("</script></body></html>");
    html
}

// Keep separate inputs visible because each renderer consumes distinct source sections.
#[allow(clippy::too_many_arguments)]
fn render_markdown(
    packet: &Value,
    answer: &OperationAnswer,
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
    used_labels: &BTreeSet<String>,
    source_navigation: &SourceNavigation,
    process_diagram: Option<&ProcessDiagram>,
    labels: ReaderLabels,
    provenance: Option<&Value>,
) -> String {
    let preparation_titles = preparation_titles(answer);
    let decision_tables = render_decision_tables_markdown(
        &answer.steps,
        answer,
        evidence_index,
        &preparation_titles,
        labels,
    );
    let preparations =
        render_preparations_markdown(answer, packet, evidence_index, &preparation_titles, labels);
    let movement_rows = data_movement_rows(answer, labels);
    let data_movement = render_data_movement_markdown(&movement_rows, evidence_index, labels);
    let mut nav = Vec::new();
    if !answer.glossary.is_empty() {
        nav.push(format!(
            "[ {} ](#glossary)",
            markdown_escape(labels.glossary())
        ));
    }
    nav.push(format!(
        "[ {} ](#ordered-behavior)",
        markdown_escape(labels.ordered(packet["profile"] == "process-graph-v1"))
    ));
    nav.push(format!(
        "[ {} ](#process-outline)",
        markdown_escape(labels.process_outline())
    ));
    nav.push(format!(
        "[ {} ](#authored-pseudocode)",
        markdown_escape(labels.tree_reference())
    ));
    if process_diagram.is_some() {
        nav.push(format!(
            "[ {} ](#source-process-projection)",
            markdown_escape(labels.process_projection())
        ));
    }
    if !preparations.is_empty() {
        nav.push(format!(
            "[ {} ](#preparations)",
            markdown_escape(labels.preparations())
        ));
    }
    if !decision_tables.is_empty() {
        nav.push(format!(
            "[ {} ](#first-match-decisions)",
            markdown_escape(labels.first_match())
        ));
    }
    nav.push(format!(
        "[ {} ](#summary)",
        markdown_escape(labels.summary())
    ));
    let mut markdown = format!(
        "# {}\n\n> **{}** {}\n\n<nav>**{}:** {}</nav>\n\n",
        markdown_escape(&answer.title),
        markdown_escape(labels.status()),
        markdown_escape(labels.status_note()),
        markdown_escape(if labels.russian {
            "Содержание"
        } else {
            "Contents"
        }),
        nav.join(" · ")
    );
    markdown.push_str(&render_uncertainties_markdown(answer, labels));
    markdown.push_str(&render_process_outline_markdown(answer, labels));
    markdown.push_str(&render_glossary_markdown(
        answer,
        packet,
        evidence_index,
        labels,
    ));
    markdown.push_str(&format!(
        "<a id=\"authored-pseudocode\"></a>\n## {}\n\n{}\n\n",
        markdown_escape(labels.tree_reference()),
        if labels.russian {
            "Структура и порядок ветвей повторяют переданные шаги ответа."
        } else {
            "Structure and branch order follow the supplied answer steps."
        }
    ));
    markdown.push_str(&render_pseudocode_markdown(&answer.steps, answer, labels));
    markdown.push('\n');
    if let Some(diagram) = process_diagram {
        markdown.push_str(&render_process_diagram_markdown(
            diagram,
            source_navigation,
            labels,
        ));
    }
    markdown.push_str(&format!(
        "<a id=\"ordered-behavior\"></a>\n## {}\n\n",
        markdown_escape(labels.ordered(packet["profile"] == "process-graph-v1"))
    ));
    markdown.push_str(&decision_tables);
    markdown.push_str(&render_markdown_steps(
        &answer.steps,
        answer,
        evidence_index,
        0,
        true,
        "",
        &preparation_titles,
        labels,
    ));
    markdown.push_str(&render_predicates_markdown(answer, evidence_index, labels));
    markdown.push_str(&preparations);
    markdown.push_str(&render_summary_details_markdown(
        answer,
        evidence_index,
        labels,
    ));
    markdown.push_str(&format!(
        "<details class=\"technical-details\"><summary>{}</summary>\n\n<a id=\"technical-reference\"></a>\n**{}:** `{}`\n\n",
        markdown_escape(labels.technical_details()),
        markdown_escape(if labels.russian {
            "Хеш пакета"
        } else {
            "Packet digest"
        }),
        markdown_escape(&answer.packet_digest)
    ));
    markdown.push_str(&render_preparation_technical_markdown(
        answer, packet, labels,
    ));
    markdown.push_str(&render_block_evidence_markdown(
        answer,
        evidence_index,
        labels,
    ));
    markdown.push_str(&data_movement);
    markdown.push_str(&render_packet_fact_tables_markdown(
        packet,
        evidence_index,
        labels,
    ));
    markdown.push_str(&render_evidence_index_markdown(
        citations,
        evidence_index,
        used_labels,
        source_navigation,
        labels,
    ));
    markdown.push_str(&render_source_locations_markdown(source_navigation, labels));
    markdown.push_str(&render_packet_limits_markdown(packet, labels));
    markdown.push_str(&render_full_inventory_markdown(
        citations,
        evidence_index,
        used_labels,
        labels,
    ));
    markdown.push_str("</details>\n");
    if let Some(provenance) = provenance {
        markdown.push_str(&render_review_provenance(provenance, false));
    }
    markdown
}

const OFFLINE_STYLE: &str = r#"
:root{color-scheme:light dark;font:16px/1.55 system-ui,sans-serif;--line:#8792a2;--panel:#171b22;--accent:#73b7ff}
*{box-sizing:border-box}html,body{width:100%;min-width:0}body{margin:0;background:#101319;color:#e8edf5}main{width:100%;max-width:1120px;min-width:0;margin:auto;padding:2rem;overflow-wrap:anywhere}
main>*,main section,main details,.document-nav,.claim{max-width:100%;min-width:0}.document-nav{display:flex;flex-wrap:wrap;align-items:baseline;gap:.25rem .65rem}.document-nav a{min-width:0;overflow-wrap:anywhere;word-break:break-word}
h1,h2,h3{line-height:1.2}h2{margin-top:2.2rem;border-bottom:1px solid #394252;padding-bottom:.45rem}
a{color:var(--accent);overflow-wrap:anywhere;word-break:break-word}code{overflow-wrap:anywhere;word-break:break-word}.review-status{padding:.85rem 1rem;border-left:4px solid #d99e45;background:#29231a;overflow-wrap:anywhere}
.packet-digest{color:#bac4d3}.claim,.step-node{border:1px solid #394252;border-radius:.55rem;padding:.8rem 1rem;margin:.55rem 0;background:var(--panel)}
.claim-text,.step-text{white-space:pre-wrap;overflow-wrap:anywhere}.full-summary-text{width:100%;min-width:0;white-space:pre-wrap;overflow-wrap:anywhere;word-break:break-word}.claim-uncertainty{color:#ffd08a}.citations{display:inline-flex;gap:.45rem;flex-wrap:wrap;margin-left:.45rem;font-size:.9em;min-width:0;max-width:100%}
.citation{border:1px solid #52627a;border-radius:1rem;padding:.05rem .5rem;text-decoration:none}.step-kind{font-size:.75em;text-transform:uppercase;letter-spacing:.06em;color:#9ed0ff;margin-right:.55rem}
.step-meta{color:#b7c1d0;font-size:.9em}.ordered-steps,.nested-steps{padding-left:1.5rem}.path-group{margin:.5rem 0 .75rem 1rem;padding-left:.8rem;border-left:2px solid #52627a}.path-label{font-weight:650;color:#bdc9dc}
.pseudocode{list-style:none;margin:0;padding:0;min-width:0;max-width:100%}.pseudocode-row{margin:.2rem 0;min-width:0;max-width:100%;overflow-wrap:anywhere;word-break:break-word}.pseudocode-depth-0{padding-left:0}.pseudocode-depth-1{padding-left:.8rem}.pseudocode-depth-2{padding-left:1.6rem}.pseudocode-link{display:inline;white-space:normal;overflow-wrap:anywhere;word-break:break-word;min-width:0;max-width:100%}.pseudocode-structure,.pseudocode-scope{color:#bdc9dc;font-size:.9em}.pseudocode-uncertainty{font-size:.8em;white-space:normal;overflow-wrap:anywhere}
table{border-collapse:collapse;width:100%;margin:1rem 0 1.5rem}caption{text-align:left;font-weight:700;margin:.5rem 0}th,td{border:1px solid #596273;padding:.5rem .65rem;text-align:left;vertical-align:top;overflow-wrap:anywhere}th{background:#242b36}
.table-scroll{width:100%;min-width:0;max-width:100%;overflow-x:auto;overscroll-behavior-inline:contain}.table-scroll:focus-visible{outline:2px solid var(--accent);outline-offset:2px}.table-scroll table{min-width:34rem}.table-scroll-hint{margin:.2rem 0 .45rem;font-size:.85em;color:#bac4d3}.decision-outcome>summary{cursor:pointer;font-weight:600}
pre{width:100%;min-width:0;max-width:100%;overflow:auto;white-space:pre}.source-locations pre{white-space:pre}.technical-details,.packet-facts-reference,.evidence-reference{width:100%;min-width:0;max-width:100%}.technical-details{contain:layout}.source-locations,.source-locations li,.source-locations li>* ,.source-locations strong,.evidence-index,.evidence-index li{max-width:100%;min-width:0;overflow-wrap:anywhere;word-break:break-word}
.evidence-index,.limitations,.uncertainties{padding-left:1.4rem}.muted{color:#bac4d3}figure{margin:0}figcaption{font-weight:650}
@media(max-width:720px){main{padding:1rem}}
@media(prefers-color-scheme:light){body{background:#fff;color:#18202b}a{color:#005ea8}.claim,.step-node{background:#f6f8fb;border-color:#ccd3df}.review-status{background:#fff7e8}.step-meta,.muted,.table-scroll-hint,.pseudocode-structure,.pseudocode-scope{color:#49586d}.claim-uncertainty{color:#704400}th{background:#edf1f7}}
"#;

fn render_claim_html(
    claim: &Claim,
    evidence_index: &BTreeMap<String, usize>,
    glossary: &[GlossaryTerm],
    labels: ReaderLabels,
) -> String {
    let mut output = format!(
        "<div class=\"claim\"><p class=\"claim-text\">{}</p>{}{}",
        html_escape(&claim.text),
        render_evidence_html(&claim.evidence, evidence_index, labels),
        render_glossary_links_html(&claim.glossary_refs, glossary, labels)
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

fn render_summary_details_html(
    answer: &OperationAnswer,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    let mut output = format!(
        "<details id=\"summary\" class=\"full-summary\"><summary>{}</summary><div class=\"claim\"><pre class=\"full-summary-text\"><code>{}</code></pre>{}{}",
        html_escape(labels.summary()),
        html_escape(&answer.summary.text),
        render_evidence_html(&answer.summary.evidence, evidence_index, labels),
        render_glossary_links_html(&answer.summary.glossary_refs, &answer.glossary, labels)
    );
    if let Some(uncertainty) = answer.summary.uncertainty.as_deref() {
        output.push_str(&format!(
            "<p class=\"claim-uncertainty\"><strong>{}:</strong> {}</p>",
            html_escape(labels.uncertainty()),
            html_escape(uncertainty)
        ));
    }
    output.push_str("</div></details>");
    output
}

fn render_evidence_html(
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
            Some(number) => format!(
                "<a class=\"citation\" href=\"#evidence-{number}\">{}</a>",
                html_escape(label)
            ),
            None => html_escape(label),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<details class=\"citations\"><summary>{}</summary><span class=\"citation-list\">{links}</span></details>",
        html_escape(&labels.evidence_count(evidence.len()))
    )
}

fn render_html_steps(
    steps: &[OperationStep],
    answer: &OperationAnswer,
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
        let anchor = step_anchor(step, &step_path);
        if let Some(table) = (step.kind == "decision")
            .then(|| first_match_table(step, step_path.clone()))
            .flatten()
        {
            output.push_str(&format!(
                "<li>{}</li>",
                render_inline_first_match_html(
                    &table,
                    answer,
                    evidence_index,
                    preparation_titles,
                    labels,
                )
            ));
            continue;
        }
        if ordered || !path_prefix.is_empty() {
            output.push_str(&format!(
                "<li id=\"{}\"><div class=\"step-node\"><span class=\"step-kind\">",
                html_escape(&anchor)
            ));
            output.push_str(&html_escape(&step.kind));
            output.push_str("</span>");
        } else {
            output.push_str(&format!(
                "<li id=\"{}\"><div class=\"step-node\">",
                html_escape(&anchor)
            ));
        }
        output.push_str("<span class=\"step-text\">");
        if let Some(predicate) = predicate_for_step(step, &answer.predicates) {
            output.push_str(&format!(
                "<a href=\"#predicate-{}\">{}</a>",
                html_escape(&predicate.id),
                html_escape(&predicate.label)
            ));
        } else {
            output.push_str(&html_escape(&step.meaning.text));
        }
        output.push_str("</span>");
        output.push_str(&render_evidence_html(
            &step.meaning.evidence,
            evidence_index,
            labels,
        ));
        if answer.schema != ANSWER_SCHEMA_V1_2 {
            output.push_str(&render_step_metadata_html(step, labels));
        }
        let mut glossary_refs = step.glossary_refs.clone();
        glossary_refs.extend(step.meaning.glossary_refs.iter().cloned());
        glossary_refs.sort();
        glossary_refs.dedup();
        output.push_str(&render_glossary_links_html(
            &glossary_refs,
            &answer.glossary,
            labels,
        ));
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
                    answer,
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
                    answer,
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

fn render_inline_first_match_html(
    table: &FirstMatchTable<'_>,
    answer: &OperationAnswer,
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let mut output = format!(
        "<div class=\"inline-decision\">{}<table class=\"first-match\"><caption>{}</caption><thead><tr><th>{}</th><th>{}</th></tr></thead><tbody>",
        table_scroll_start(labels, labels.first_match()),
        html_escape(labels.first_match()),
        html_escape(labels.condition()),
        html_escape(labels.outcome())
    );
    for branch in &table.branches {
        let condition = predicate_for_step(branch.decision, &answer.predicates)
            .map(|predicate| {
                format!(
                    "<a href=\"#predicate-{}\">{}</a>",
                    html_escape(&predicate.id),
                    html_escape(&predicate.label)
                )
            })
            .unwrap_or_else(|| html_escape(&branch.decision.meaning.text));
        let anchor = step_anchor(branch.decision, &branch.path);
        let mut glossary_refs = branch.decision.glossary_refs.clone();
        glossary_refs.extend(branch.decision.meaning.glossary_refs.iter().cloned());
        glossary_refs.sort();
        glossary_refs.dedup();
        output.push_str(&format!(
            "<tr id=\"{}\"><td>{}{}{}{}{}{}</td><td>{}</td></tr>",
            html_escape(&anchor),
            condition,
            if answer.schema == ANSWER_SCHEMA_V1_2 {
                String::new()
            } else {
                render_evidence_html(&branch.decision.meaning.evidence, evidence_index, labels)
            },
            render_glossary_links_html(&glossary_refs, &answer.glossary, labels),
            branch
                .decision
                .meaning
                .uncertainty
                .as_deref()
                .map(|uncertainty| format!(
                    "<p class=\"claim-uncertainty\"><strong>{}:</strong> {}</p>",
                    html_escape(labels.uncertainty()),
                    html_escape(uncertainty)
                ))
                .unwrap_or_default(),
            if answer.schema == ANSWER_SCHEMA_V1_2 {
                String::new()
            } else {
                render_step_metadata_html(branch.decision, labels)
            },
            render_preparation_links_html(
                &branch.decision.preparation_refs,
                preparation_titles,
                labels,
            ),
            render_decision_outcome_detail_html(
                &branch.decision.children,
                answer,
                &format!("{}-then", branch.path),
                evidence_index,
                preparation_titles,
                labels,
            )
        ));
    }
    if !table.otherwise.is_empty() || answer.schema == ANSWER_SCHEMA_V1_2 {
        output.push_str(&format!(
            "<tr><td>{}</td><td>{}</td></tr>",
            html_escape(labels.no_match()),
            if table.otherwise.is_empty() {
                format!(
                    "<span class=\"muted\">{}</span>",
                    html_escape(labels.no_match_outcome())
                )
            } else {
                render_decision_outcome_detail_html(
                    table.otherwise,
                    answer,
                    &table.otherwise_path,
                    evidence_index,
                    preparation_titles,
                    labels,
                )
            }
        ));
    }
    output.push_str("</tbody></table></div>");
    output.push_str(&format!(
        "<p class=\"muted\">{}</p></div>",
        html_escape(labels.first_match_note())
    ));
    output
}

fn render_decision_outcome_detail_html(
    steps: &[OperationStep],
    answer: &OperationAnswer,
    path_prefix: &str,
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let Some((first, summary)) = branch_summary(steps) else {
        return "<span class=\"muted\">—</span>".into();
    };
    format!(
        "<details class=\"decision-outcome\"><summary><span class=\"step-kind\">{}</span> {} <span class=\"muted\">{}</span></summary>{}</details>",
        html_escape(&first.kind),
        html_escape(&summary),
        html_escape(labels.details_link()),
        render_html_steps(
            steps,
            answer,
            evidence_index,
            false,
            path_prefix,
            preparation_titles,
            labels,
        )
    )
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

fn step_anchor(step: &OperationStep, path: &str) -> String {
    step.id
        .as_deref()
        .map(|id| format!("block-{id}"))
        .unwrap_or_else(|| format!("step-{path}"))
}

fn render_process_outline_html(answer: &OperationAnswer, labels: ReaderLabels) -> String {
    let mut output = format!(
        "<section id=\"process-outline\"><h2>{}</h2><ol class=\"process-outline\">",
        html_escape(labels.process_outline())
    );
    for (index, step) in answer.steps.iter().enumerate() {
        let path = step_path("", index + 1);
        output.push_str(&format!(
            "<li><a href=\"#{}\"><span class=\"step-kind\">{}</span> {}</a></li>",
            html_escape(&step_anchor(step, &path)),
            html_escape(&step.kind),
            html_escape(&step.meaning.text)
        ));
    }
    output.push_str("</ol></section>");
    output
}

fn render_process_outline_markdown(answer: &OperationAnswer, labels: ReaderLabels) -> String {
    let mut output = format!(
        "<a id=\"process-outline\"></a>\n## {}\n\n",
        markdown_escape(labels.process_outline())
    );
    for (index, step) in answer.steps.iter().enumerate() {
        let path = step_path("", index + 1);
        output.push_str(&format!(
            "{}. [**{}:** {}](#{})\n",
            index + 1,
            markdown_escape(&step.kind),
            markdown_escape(&step.meaning.text),
            step_anchor(step, &path)
        ));
    }
    output.push('\n');
    output
}

fn predicate_for_step<'a>(
    step: &OperationStep,
    predicates: &'a [Predicate],
) -> Option<&'a Predicate> {
    step.predicate_ref.as_deref().and_then(|reference| {
        predicates
            .iter()
            .find(|predicate| predicate.id == reference)
    })
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

enum PseudocodeRow<'a> {
    Step {
        step: &'a OperationStep,
        path: String,
        depth: usize,
        scope: Option<usize>,
    },
    BranchStart {
        scope: usize,
        label: String,
        depth: usize,
    },
    BranchEnd {
        scope: usize,
        depth: usize,
    },
    ChainStart {
        scope: usize,
        depth: usize,
    },
    ElseIf {
        scope: usize,
        depth: usize,
    },
    ChainEnd {
        scope: usize,
        depth: usize,
    },
}

fn next_pseudocode_scope(next_scope: &mut usize) -> usize {
    *next_scope += 1;
    *next_scope
}

fn collect_pseudocode_rows<'a>(
    steps: &'a [OperationStep],
    path_prefix: &str,
    depth: usize,
    enclosing_scope: Option<usize>,
    next_scope: &mut usize,
    output: &mut Vec<PseudocodeRow<'a>>,
    labels: ReaderLabels,
) {
    for (index, step) in steps.iter().enumerate() {
        let path = step_path(path_prefix, index + 1);
        if let Some(table) = (step.kind == "decision")
            .then(|| first_match_table(step, path.clone()))
            .flatten()
        {
            let chain_scope = next_pseudocode_scope(next_scope);
            output.push(PseudocodeRow::ChainStart {
                scope: chain_scope,
                depth,
            });
            for (branch_index, branch) in table.branches.iter().enumerate() {
                if branch_index > 0 {
                    output.push(PseudocodeRow::ElseIf {
                        scope: chain_scope,
                        depth,
                    });
                }
                output.push(PseudocodeRow::Step {
                    step: branch.decision,
                    path: branch.path.clone(),
                    depth,
                    scope: enclosing_scope,
                });
                if !branch.decision.children.is_empty() {
                    let branch_scope = next_pseudocode_scope(next_scope);
                    output.push(PseudocodeRow::BranchStart {
                        scope: branch_scope,
                        label: labels.children_label("decision").to_owned(),
                        depth: depth + 1,
                    });
                    collect_pseudocode_rows(
                        &branch.decision.children,
                        &format!("{}-then", branch.path),
                        depth + 2,
                        Some(branch_scope),
                        next_scope,
                        output,
                        labels,
                    );
                    output.push(PseudocodeRow::BranchEnd {
                        scope: branch_scope,
                        depth: depth + 1,
                    });
                }
            }
            if !table.otherwise.is_empty() {
                let fallback_scope = next_pseudocode_scope(next_scope);
                output.push(PseudocodeRow::BranchStart {
                    scope: fallback_scope,
                    label: labels.no_match().to_owned(),
                    depth: depth + 1,
                });
                collect_pseudocode_rows(
                    table.otherwise,
                    &table.otherwise_path,
                    depth + 2,
                    Some(fallback_scope),
                    next_scope,
                    output,
                    labels,
                );
                output.push(PseudocodeRow::BranchEnd {
                    scope: fallback_scope,
                    depth: depth + 1,
                });
            }
            output.push(PseudocodeRow::ChainEnd {
                scope: chain_scope,
                depth,
            });
            continue;
        }

        output.push(PseudocodeRow::Step {
            step,
            path: path.clone(),
            depth,
            scope: enclosing_scope,
        });
        if !step.children.is_empty() {
            let branch_scope = next_pseudocode_scope(next_scope);
            output.push(PseudocodeRow::BranchStart {
                scope: branch_scope,
                label: labels.children_label(&step.kind).to_owned(),
                depth: depth + 1,
            });
            collect_pseudocode_rows(
                &step.children,
                &format!("{path}-then"),
                depth + 2,
                Some(branch_scope),
                next_scope,
                output,
                labels,
            );
            output.push(PseudocodeRow::BranchEnd {
                scope: branch_scope,
                depth: depth + 1,
            });
        }
        if !step.otherwise.is_empty() {
            let branch_scope = next_pseudocode_scope(next_scope);
            output.push(PseudocodeRow::BranchStart {
                scope: branch_scope,
                label: labels.otherwise_label(&step.kind).to_owned(),
                depth: depth + 1,
            });
            collect_pseudocode_rows(
                &step.otherwise,
                &format!("{path}-else"),
                depth + 2,
                Some(branch_scope),
                next_scope,
                output,
                labels,
            );
            output.push(PseudocodeRow::BranchEnd {
                scope: branch_scope,
                depth: depth + 1,
            });
        }
    }
}

fn pseudocode_rows<'a>(steps: &'a [OperationStep], labels: ReaderLabels) -> Vec<PseudocodeRow<'a>> {
    let mut output = Vec::new();
    let mut next_scope = 0;
    collect_pseudocode_rows(steps, "", 0, None, &mut next_scope, &mut output, labels);
    output
}

fn pseudocode_step_label(step: &OperationStep, answer: &OperationAnswer) -> String {
    if step.kind == "decision"
        && let Some(predicate) = predicate_for_step(step, &answer.predicates)
    {
        return predicate.label.clone();
    }
    let normalized = step
        .meaning
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    const MAX_CHARS: usize = 96;
    if normalized.chars().count() <= MAX_CHARS {
        normalized
    } else {
        let mut excerpt = normalized.chars().take(MAX_CHARS - 1).collect::<String>();
        excerpt.truncate(excerpt.trim_end().len());
        excerpt.push('…');
        excerpt
    }
}

fn pseudocode_depth_class(depth: usize) -> &'static str {
    match depth.min(2) {
        0 => "pseudocode-depth-0",
        1 => "pseudocode-depth-1",
        _ => "pseudocode-depth-2",
    }
}

fn pseudocode_scope_html(scope: usize, labels: ReaderLabels) -> String {
    format!(
        "{} {scope}",
        if labels.russian {
            "Область"
        } else {
            "Scope"
        }
    )
}

fn render_pseudocode_html(
    steps: &[OperationStep],
    answer: &OperationAnswer,
    labels: ReaderLabels,
) -> String {
    let mut output = String::from("<ul class=\"pseudocode\">");
    for row in pseudocode_rows(steps, labels) {
        match row {
            PseudocodeRow::Step {
                step,
                path,
                depth,
                scope,
            } => {
                let anchor = step_anchor(step, &path);
                output.push_str(&format!(
                    "<li class=\"pseudocode-row {}\"><a class=\"pseudocode-link\" href=\"#{}\"><span class=\"step-kind\">{}</span> {}</a>",
                    pseudocode_depth_class(depth),
                    html_escape(&anchor),
                    html_escape(&step.kind),
                    html_escape(&pseudocode_step_label(step, answer))
                ));
                if depth > 2
                    && let Some(scope) = scope
                {
                    output.push_str(&format!(
                        " <span class=\"pseudocode-scope\">{}</span>",
                        html_escape(&pseudocode_scope_html(scope, labels))
                    ));
                }
                if step.meaning.uncertainty.is_some() {
                    let detail_label = if labels.russian {
                        "Открыть шаг с пояснением неопределённости"
                    } else {
                        "Open step details for uncertainty"
                    };
                    output.push_str(&format!(
                        " <a class=\"pseudocode-uncertainty\" href=\"#{}\" aria-label=\"{}\" title=\"{}\">ⓘ</a>",
                        html_escape(&anchor),
                        html_escape(labels.uncertainty()),
                        html_escape(detail_label)
                    ));
                }
                output.push_str("</li>");
            }
            PseudocodeRow::BranchStart {
                scope,
                label,
                depth,
            } => output.push_str(&format!(
                "<li class=\"pseudocode-row pseudocode-structure {}\">{} — {}</li>",
                pseudocode_depth_class(depth),
                html_escape(&pseudocode_scope_html(scope, labels)),
                html_escape(&label)
            )),
            PseudocodeRow::BranchEnd { scope, depth } => output.push_str(&format!(
                "<li class=\"pseudocode-row pseudocode-structure {}\">{} {}</li>",
                pseudocode_depth_class(depth),
                if labels.russian { "Конец" } else { "End" },
                html_escape(&pseudocode_scope_html(scope, labels))
            )),
            PseudocodeRow::ChainStart { scope, depth } => output.push_str(&format!(
                "<li class=\"pseudocode-row pseudocode-structure {}\">{} — {}</li>",
                pseudocode_depth_class(depth),
                if labels.russian {
                    "Цепочка решений"
                } else {
                    "Decision chain"
                },
                html_escape(&pseudocode_scope_html(scope, labels))
            )),
            PseudocodeRow::ElseIf { scope, depth } => output.push_str(&format!(
                "<li class=\"pseudocode-row pseudocode-structure {}\">{} ({})</li>",
                pseudocode_depth_class(depth),
                if labels.russian {
                    "Иначе если"
                } else {
                    "Else if"
                },
                html_escape(&pseudocode_scope_html(scope, labels))
            )),
            PseudocodeRow::ChainEnd { scope, depth } => output.push_str(&format!(
                "<li class=\"pseudocode-row pseudocode-structure {}\">{} {}</li>",
                pseudocode_depth_class(depth),
                if labels.russian {
                    "Конец цепочки решений"
                } else {
                    "End decision chain"
                },
                html_escape(&pseudocode_scope_html(scope, labels))
            )),
        }
    }
    output.push_str("</ul>");
    output
}

fn render_pseudocode_markdown(
    steps: &[OperationStep],
    answer: &OperationAnswer,
    labels: ReaderLabels,
) -> String {
    let mut output = String::new();
    for row in pseudocode_rows(steps, labels) {
        match row {
            PseudocodeRow::Step {
                step,
                path,
                depth,
                scope,
            } => {
                let anchor = step_anchor(step, &path);
                output.push_str(&format!(
                    "- [**{}:** {}](#{})",
                    markdown_escape(&step.kind),
                    markdown_escape(&pseudocode_step_label(step, answer)),
                    anchor
                ));
                if depth > 2
                    && let Some(scope) = scope
                {
                    output.push_str(&format!(
                        " _{}_",
                        markdown_escape(&pseudocode_scope_html(scope, labels))
                    ));
                }
                if step.meaning.uncertainty.is_some() {
                    output.push_str(&format!(
                        " [ⓘ](#{anchor} \"{}\")",
                        markdown_escape(labels.uncertainty())
                    ));
                }
                output.push('\n');
            }
            PseudocodeRow::BranchStart { scope, label, .. } => output.push_str(&format!(
                "- _{} — {}_\n",
                markdown_escape(&pseudocode_scope_html(scope, labels)),
                markdown_escape(&label)
            )),
            PseudocodeRow::BranchEnd { scope, .. } => output.push_str(&format!(
                "- _{} {}_\n",
                if labels.russian { "Конец" } else { "End" },
                markdown_escape(&pseudocode_scope_html(scope, labels))
            )),
            PseudocodeRow::ChainStart { scope, .. } => output.push_str(&format!(
                "- _{} — {}_\n",
                if labels.russian {
                    "Цепочка решений"
                } else {
                    "Decision chain"
                },
                markdown_escape(&pseudocode_scope_html(scope, labels))
            )),
            PseudocodeRow::ElseIf { scope, .. } => output.push_str(&format!(
                "- _{} ({})_\n",
                if labels.russian {
                    "Иначе если"
                } else {
                    "Else if"
                },
                markdown_escape(&pseudocode_scope_html(scope, labels))
            )),
            PseudocodeRow::ChainEnd { scope, .. } => output.push_str(&format!(
                "- _{} {}_\n",
                if labels.russian {
                    "Конец цепочки решений"
                } else {
                    "End decision chain"
                },
                markdown_escape(&pseudocode_scope_html(scope, labels))
            )),
        }
    }
    output
}

// Recursive rendering uses each option directly to preserve branch formatting.
#[allow(clippy::too_many_arguments)]
fn render_markdown_steps(
    steps: &[OperationStep],
    answer: &OperationAnswer,
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
        let anchor = step_anchor(step, &step_path);
        output.push_str(&format!("<a id=\"{}\"></a>", html_escape(&anchor)));
        let prefix = if ordered && depth == 0 {
            format!("{}. ", index + 1)
        } else {
            format!("{}- ", indent)
        };
        let condition = predicate_for_step(step, &answer.predicates)
            .map(|predicate| {
                format!(
                    "[{}](#predicate-{})",
                    markdown_escape(&predicate.label),
                    predicate.id
                )
            })
            .unwrap_or_else(|| markdown_escape(&step.meaning.text));
        output.push_str(&format!(
            "{prefix}**{}:** {}{}{}{}\n",
            markdown_escape(&step.kind),
            condition,
            if answer.schema == ANSWER_SCHEMA_V1_2 {
                String::new()
            } else {
                render_evidence_markdown(&step.meaning.evidence, evidence_index, labels)
            },
            if answer.schema == ANSWER_SCHEMA_V1_2 {
                String::new()
            } else {
                markdown_step_metadata(step, labels)
            },
            render_preparation_links_markdown(&step.preparation_refs, preparation_titles, labels)
        ));
        let mut glossary_refs = step.glossary_refs.clone();
        glossary_refs.extend(step.meaning.glossary_refs.iter().cloned());
        glossary_refs.sort();
        glossary_refs.dedup();
        let term_links = render_glossary_links_markdown(&glossary_refs, &answer.glossary, labels);
        if !term_links.is_empty() {
            output.push_str(&format!("{}- {}\n", indent, term_links.trim()));
        }
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
                answer,
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
                answer,
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
    glossary: &[GlossaryTerm],
    labels: ReaderLabels,
) -> String {
    let mut output = format!(
        "{}{}{}",
        markdown_escape(&claim.text),
        render_evidence_markdown(&claim.evidence, evidence_index, labels),
        render_glossary_links_markdown(&claim.glossary_refs, glossary, labels)
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

fn render_summary_details_markdown(
    answer: &OperationAnswer,
    evidence_index: &BTreeMap<String, usize>,
    labels: ReaderLabels,
) -> String {
    let mut body = vec![markdown_code_block(&answer.summary.text)];
    let evidence = render_evidence_markdown(&answer.summary.evidence, evidence_index, labels);
    if !evidence.is_empty() {
        body.push(evidence);
    }
    let glossary_links =
        render_glossary_links_markdown(&answer.summary.glossary_refs, &answer.glossary, labels);
    if !glossary_links.is_empty() {
        body.push(glossary_links);
    }
    if let Some(uncertainty) = answer.summary.uncertainty.as_deref() {
        body.push(format!(
            "**{}:** {}",
            markdown_escape(labels.uncertainty()),
            markdown_escape(uncertainty)
        ));
    }
    format!(
        "<details id=\"summary\" class=\"full-summary\">\n<summary>{}</summary>\n\n{}\n\n</details>\n\n",
        markdown_escape(labels.summary()),
        body.join("\n\n")
    )
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
        "<details class=\"citations\"><summary>{} ({})</summary><span class=\"citation-list\">{links}</span></details>",
        markdown_escape(labels.evidence()),
        evidence.len()
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
            collect_first_match_tables(table.otherwise, &table.otherwise_path, output);
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

fn table_outcome_markdown(
    steps: &[OperationStep],
    answer: &OperationAnswer,
    path_prefix: &str,
    evidence_index: &BTreeMap<String, usize>,
    preparation_titles: &BTreeMap<String, String>,
    labels: ReaderLabels,
) -> String {
    let Some((first, summary)) = branch_summary(steps) else {
        return "—".into();
    };
    let target = step_anchor(first, &step_path(path_prefix, 1));
    format!(
        "**{}:** {} [ {} ](#{target}){}{}{}",
        markdown_escape(&first.kind),
        markdown_escape(&summary),
        markdown_escape(labels.details_link()),
        render_evidence_markdown(&first.meaning.evidence, evidence_index, labels),
        if answer.schema == ANSWER_SCHEMA_V1_2 {
            String::new()
        } else {
            markdown_step_metadata(first, labels)
        },
        render_preparation_links_markdown(&first.preparation_refs, preparation_titles, labels)
    )
}

fn render_decision_tables_markdown(
    steps: &[OperationStep],
    answer: &OperationAnswer,
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
        "<a id=\"first-match-decisions\"></a>\n### {}\n\n",
        markdown_escape(labels.first_match())
    );
    for table in tables {
        output.push_str(&format!(
            "| {} | {} |\n|---|---|\n",
            markdown_escape(labels.condition()),
            markdown_escape(labels.outcome())
        ));
        for branch in &table.branches {
            let condition = predicate_for_step(branch.decision, &answer.predicates)
                .map(|predicate| {
                    format!(
                        "[{}](#predicate-{})",
                        markdown_table_cell(&predicate.label),
                        predicate.id
                    )
                })
                .unwrap_or_else(|| markdown_table_cell(&branch.decision.meaning.text));
            let mut glossary_refs = branch.decision.glossary_refs.clone();
            glossary_refs.extend(branch.decision.meaning.glossary_refs.iter().cloned());
            glossary_refs.sort();
            glossary_refs.dedup();
            output.push_str(&format!(
                "| {}{} | {} |\n",
                condition,
                render_glossary_links_markdown(&glossary_refs, &answer.glossary, labels),
                table_outcome_markdown(
                    &branch.decision.children,
                    answer,
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
                    table.otherwise,
                    answer,
                    &table.otherwise_path,
                    evidence_index,
                    preparation_titles,
                    labels
                )
            ));
        } else if answer.schema == ANSWER_SCHEMA_V1_2 {
            output.push_str(&format!(
                "| {} | {} |\n",
                markdown_escape(labels.no_match()),
                markdown_escape(labels.no_match_outcome())
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
        output.push_str(&table_scroll_start(labels, labels.facts(false)));
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
            let evidence_labels = combined_labels(&dto["evidence"], &field["evidence"]);
            output.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                html_escape(&value_text(&field["name"])),
                html_escape(&value_text(&field["typeDescriptor"])),
                html_escape(&value_text(&field["modifiers"])),
                html_escape(&value_text(&field["annotations"])),
                html_escape(&tokens_text(&field["sourceTokens"])),
                render_evidence_html(&evidence_labels, evidence_index, labels)
            ));
        }
        output.push_str("</tbody></table></div>");
    }
    if !constants.is_empty() {
        output.push_str(&table_scroll_start(labels, labels.facts(false)));
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
                render_evidence_html(&strings(&constant["evidence"]), evidence_index, labels)
            ));
        }
        output.push_str("</tbody></table></div>");
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
        output.push_str(&table_scroll_start(labels, labels.facts(true)));
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
        output.push_str("</tbody></table></div>");
    }
    if !fields.is_empty() {
        output.push_str(&table_scroll_start(labels, labels.facts(true)));
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
                render_evidence_html(&strings(&field["evidence"]), evidence_index, labels)
            ));
        }
        output.push_str("</tbody></table></div>");
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
        .claim_evidence_locations
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
        .claim_evidence_locations
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
        if let Some(blocks) = navigation.citing_blocks.get(&index)
            && !blocks.is_empty()
        {
            output.push_str(&format!(
                "<p class=\"source-citations\"><strong>{}:</strong> {}</p>",
                html_escape(labels.cited_by_process()),
                blocks
                    .iter()
                    .map(|block| format!(
                        "<a href=\"#{}\">{}</a>",
                        html_escape(&block.anchor),
                        html_escape(&block.label)
                    ))
                    .collect::<Vec<_>>()
                    .join(" · ")
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
        if let Some(blocks) = navigation.citing_blocks.get(&index)
            && !blocks.is_empty()
        {
            let links = blocks
                .iter()
                .map(|block| format!("[{}](#{})", markdown_escape(&block.label), block.anchor))
                .collect::<Vec<_>>()
                .join(" · ");
            output.push_str(&format!(
                "\n**{}:** {}\n",
                markdown_escape(labels.cited_by_process()),
                links
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

    fn process_packet(source: &str, symbol: &str) -> (Value, Value) {
        let (start, end) = super::super::source_steps::method_body(source, symbol).unwrap();
        let mut packet = packet();
        packet["profile"] = json!("process-graph-v1");
        packet["citations"]["source-root"] = json!("retained source for selected root");
        packet["root"] = json!({"methodId":"root-method","symbolIdentity":symbol});
        packet["methods"] = json!([{
            "id":"root-method",
            "symbolIdentity":symbol,
            "body":{
                "sourceReference":"source-root",
                "startByte":start,
                "endByte":end,
                "evidence":["source-root"]
            }
        }]);
        packet["methodSources"] = json!([{
            "reference":"source-root",
            "authority":"TRANSFORMED_SOURCE",
            "text":source,
            "evidence":["source-root"]
        }]);
        seal(&mut packet);

        let mut audit = audit(&packet);
        audit["records"] = json!([{
            "label":"source-root",
            "kind":"SOURCE",
            "id":"source-root-id",
            "row":{"record":{
                "file":"Demo.java",
                "startLine":1,
                "endLine":source.lines().count(),
                "text":source
            }}
        }]);
        seal_audit(&mut audit);
        (packet, audit)
    }

    fn rebind_audit(packet: &Value, audit: &mut Value) {
        audit["packetDigest"] = packet["packetDigest"].clone();
        seal_audit(audit);
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

    fn validate_field_subject(packet: &Value) -> Result<RenderedAnswer, crate::error::ClewError> {
        validate_and_render_draft(packet, &audit(packet), field_subject_answer(packet))
    }

    fn html_ids(document: &str) -> Vec<String> {
        let mut ids = Vec::new();
        let mut remaining = document;
        while let Some((_, after_key)) = remaining.split_once("id=\"") {
            let Some((id, after_id)) = after_key.split_once('"') else {
                break;
            };
            ids.push(id.to_owned());
            remaining = after_id;
        }
        ids
    }

    fn html_fragment_links(document: &str) -> Vec<String> {
        let mut links = Vec::new();
        let mut remaining = document;
        while let Some((_, after_key)) = remaining.split_once("href=\"#") {
            let Some((id, after_id)) = after_key.split_once('"') else {
                break;
            };
            links.push(id.to_owned());
            remaining = after_id;
        }
        links
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
    fn version_1_2_projects_glossary_predicates_and_block_evidence_with_technical_details_drilled_down()
     {
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

        let rendered = validate_and_render_draft(&packet, &audit(&packet), authored).unwrap();

        assert_eq!(rendered.answer, original);
        assert!(rendered.html.contains("<li>SOURCE_RECEIVER_SHADOWING_AMBIGUOUS and CALL_SITE_SOURCE_NOT_CONTAINED_IN_METHOD_BODY remain unresolved.</li>"));
        assert!(rendered.html.contains(&format!(
            "<code>long-declaration</code> — long-declaration · {long_declaration}"
        )));
        assert!(rendered.html.contains("Glossary"));
        assert!(rendered.html.contains("Transfer request"));
        assert!(
            rendered
                .html
                .contains("The packet does not identify the business entity")
        );
        assert!(rendered.html.contains("href=\"#glossary-request\""));
        assert!(rendered.html.contains("The request has a positive count"));
        assert!(
            rendered
                .html
                .contains("request != null &amp;&amp; request.count &gt; 0")
        );
        assert!(rendered.html.contains("The AND checks run left to right"));
        assert!(rendered.html.contains("The OR short-circuits"));
        assert!(rendered.html.contains("The first matching row wins"));
        assert!(
            rendered
                .html
                .contains("Otherwise (none of the conditions above matched)")
        );
        assert!(
            rendered
                .html
                .contains("No outcome for this case is retained.")
        );
        assert_eq!(
            rendered.html.matches("id=\"preparation-shared\"").count(),
            1
        );
        assert!(rendered.html.contains("href=\"#preparation-shared\""));
        assert!(rendered.html.contains("Evidence by block"));
        assert!(rendered.html.contains("predicate-details-source-check"));
        assert!(rendered.html.contains("id=\"semantic-predicates\""));
        assert!(rendered.html.contains("id=\"predicate-details\""));
        assert!(!rendered.html.contains("<section id=\"predicate-details\""));
        assert!(rendered.html.contains("id=\"block-valid-branch\""));
        assert_eq!(
            rendered.html.matches("id=\"block-valid-branch\"").count(),
            1
        );
        let ids = html_ids(&rendered.html);
        let unique_ids: BTreeSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique_ids.len(), "duplicate HTML ids");
        assert!(rendered.html.contains("source-from-hidden"));
        assert!(rendered.html.contains("&lt;script&gt;probe&lt;/script&gt;"));
        assert!(!rendered.html.contains("<script>probe</script>"));
        let technical_offset = rendered.html.find("class=\"technical-details\"").unwrap();
        let primary_html = rendered
            .html
            .split("class=\"technical-details\"")
            .next()
            .unwrap();
        assert!(!primary_html.contains("source-from-hidden"));
        assert!(!primary_html.contains("target-to-hidden"));
        assert!(!primary_html.contains("interaction-hidden"));
        assert!(!primary_html.contains("helper-interaction-hidden"));
        let source_metadata_offset = rendered.html.find("source-from-hidden").unwrap();
        assert!(source_metadata_offset > technical_offset);
        let preparation_metadata_offset = rendered
            .html
            .find("class=\"preparation-technical-reference\"")
            .unwrap();
        assert!(preparation_metadata_offset > technical_offset);
        assert!(rendered.html.contains("helper-interaction-hidden"));
        assert!(rendered.html.contains("api.TransferRequest"));
        assert!(rendered.html.contains("@media(max-width:720px)"));
        assert!(rendered.html.contains(".claim-uncertainty{color:#704400}"));
        assert!(rendered.html.contains("decodeURIComponent"));
        assert!(rendered.html.contains("hashchange"));
        assert!(
            rendered.markdown.find("id=\"glossary\"").unwrap()
                < rendered.markdown.find("id=\"ordered-behavior\"").unwrap()
        );
        assert!(
            rendered
                .markdown
                .contains("[The request has a positive count](#predicate-details)")
        );
        assert!(rendered.markdown.contains("Evidence by block"));
        assert!(rendered.markdown.contains("retryCount &gt;= maxRetries"));
        assert!(rendered.markdown.contains("helper\\-interaction\\-hidden"));
        assert!(rendered.markdown.contains("api.TransferRequest"));

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
    fn reader_summary_is_complete_escaped_and_closed_after_primary_sections() {
        let packet = packet();
        let summary_text = "First line\n</code></pre><script>alert(1)</script>\n```\n<component> &";
        let mut authored = semantic_answer(&packet);
        authored["summary"]["text"] = json!(summary_text);
        authored["summary"]["uncertainty"] = json!("The local condition remains unresolved.");
        authored["uncertainties"] = json!(["The global scope remains uncertain."]);
        let original = authored.clone();

        let rendered = validate_and_render_draft(&packet, &audit(&packet), authored).unwrap();

        assert_eq!(rendered.answer, original);
        let glossary = rendered.html.find("<section id=\"glossary\">").unwrap();
        let behavior = rendered
            .html
            .find("<section id=\"ordered-behavior\">")
            .unwrap();
        let preparations = rendered.html.find("id=\"preparations\"").unwrap();
        let global_uncertainty = rendered
            .html
            .find("The global scope remains uncertain.")
            .unwrap();
        let summary = rendered
            .html
            .find("<details id=\"summary\" class=\"full-summary\">")
            .unwrap();
        assert!(glossary < behavior && behavior < preparations && preparations < summary);
        assert!(global_uncertainty < summary);
        let html_nav_end = rendered.html.find("</nav>").unwrap();
        let html_nav = &rendered.html[..html_nav_end];
        assert!(
            html_nav.find("href=\"#glossary\"").unwrap()
                < html_nav.find("href=\"#ordered-behavior\"").unwrap()
        );
        assert!(
            html_nav.find("href=\"#ordered-behavior\"").unwrap()
                < html_nav.find("href=\"#preparations\"").unwrap()
        );
        let html_summary = &rendered.html[summary..];
        assert!(html_summary.starts_with(
            "<details id=\"summary\" class=\"full-summary\"><summary>Full summary</summary>"
        ));
        assert!(html_summary.contains(
            "<pre class=\"full-summary-text\"><code>First line\n&lt;/code&gt;&lt;/pre&gt;&lt;script&gt;alert(1)&lt;/script&gt;\n```\n&lt;component&gt; &amp;</code></pre>"
        ));
        assert!(!html_summary.contains("<script>alert(1)</script>"));
        assert!(html_summary.contains("href=\"#evidence-1\">d1</a>"));
        assert!(html_summary.contains("href=\"#glossary-request\">Transfer request</a>"));
        assert!(html_summary.contains("The local condition remains unresolved."));

        let markdown_glossary = rendered.markdown.find("id=\"glossary\"").unwrap();
        let markdown_behavior = rendered.markdown.find("id=\"ordered-behavior\"").unwrap();
        let markdown_preparations = rendered.markdown.find("id=\"preparations\"").unwrap();
        let markdown_uncertainty = rendered
            .markdown
            .find("The global scope remains uncertain")
            .unwrap();
        let markdown_summary = rendered
            .markdown
            .find("<details id=\"summary\" class=\"full-summary\">")
            .unwrap();
        assert!(markdown_glossary < markdown_behavior);
        assert!(markdown_behavior < markdown_preparations);
        assert!(markdown_preparations < markdown_summary);
        assert!(markdown_uncertainty < markdown_summary);
        let markdown_nav_end = rendered.markdown.find("</nav>").unwrap();
        let markdown_nav = &rendered.markdown[..markdown_nav_end];
        assert!(
            markdown_nav.find("(#glossary)").unwrap()
                < markdown_nav.find("(#ordered-behavior)").unwrap()
        );
        assert!(
            markdown_nav.find("(#ordered-behavior)").unwrap()
                < markdown_nav.find("(#preparations)").unwrap()
        );
        let markdown_summary = &rendered.markdown[markdown_summary..];
        let code_start = markdown_summary.find("````text\n").unwrap() + "````text\n".len();
        let code_end = markdown_summary[code_start..].find("\n````").unwrap() + code_start;
        assert_eq!(&markdown_summary[code_start..code_end], summary_text);
        let after_code = &markdown_summary[code_end..];
        assert!(after_code.contains("Evidence (1)"));
        assert!(after_code.contains("[Transfer request](#glossary-request)"));
        assert!(after_code.contains("**Uncertainty:** The local condition remains unresolved\\."));

        let table_count = rendered.html.matches("<table").count();
        let scroll_region_count = rendered
            .html
            .matches("<div class=\"table-scroll\" tabindex=\"0\" role=\"region\"")
            .count();
        assert_eq!(scroll_region_count, table_count);
        assert_eq!(
            rendered.html.matches("class=\"table-scroll-hint\"").count(),
            table_count
        );
        assert!(
            rendered
                .html
                .contains("aria-label=\"First matching condition\"")
        );
        assert!(
            rendered
                .html
                .contains("Focus this table and scroll horizontally to view all columns.")
        );
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
    fn semantic_contract_rejects_schema_mismatches_and_legacy_field_injection() {
        let packet = packet();
        let valid = semantic_answer(&packet);

        let mut missing_claim_refs = valid.clone();
        missing_claim_refs["summary"]
            .as_object_mut()
            .unwrap()
            .remove("glossaryRefs");
        assert!(validate_and_render_draft(&packet, &audit(&packet), missing_claim_refs).is_err());

        let mut duplicate_subject_refs = valid.clone();
        duplicate_subject_refs["glossary"][0]["subjectRefs"] =
            json!(["api.TransferRequest", "api.TransferRequest"]);
        assert!(
            validate_and_render_draft(&packet, &audit(&packet), duplicate_subject_refs).is_err()
        );

        let mut duplicate_technical_names = valid.clone();
        duplicate_technical_names["glossary"][0]["technicalNames"] =
            json!(["api.TransferRequest", "api.TransferRequest"]);
        assert!(
            validate_and_render_draft(&packet, &audit(&packet), duplicate_technical_names).is_err()
        );

        let mut predicate_on_action = valid.clone();
        predicate_on_action["steps"][0]["children"][0]["predicateRef"] = json!("details");
        assert!(validate_and_render_draft(&packet, &audit(&packet), predicate_on_action).is_err());

        let mut null_predicate_on_action = valid.clone();
        null_predicate_on_action["steps"][0]["children"][0]["predicateRef"] = Value::Null;
        assert!(
            validate_and_render_draft(&packet, &audit(&packet), null_predicate_on_action).is_err()
        );

        let mut unsafe_step_id = valid.clone();
        unsafe_step_id["steps"][0]["id"] = json!("valid\" onclick=\"alert(1)");
        assert!(validate_and_render_draft(&packet, &audit(&packet), unsafe_step_id).is_err());

        let mut legacy_with_new_id = simple_answer(&packet, "d1");
        legacy_with_new_id["steps"][0]["id"] = json!("step\" onclick=\"alert(1)");
        assert!(validate_and_render(&packet, &audit(&packet), legacy_with_new_id).is_err());
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
        assert!(rendered.html.contains("Technical reference"));
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
        let rendered =
            validate_and_render(&packet, &original_audit, simple_answer(&packet, "d1")).unwrap();

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
            validate_and_render(
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
            validate_and_render(
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
            validate_and_render(
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
            validate_and_render(
                &extra_citation,
                &audit(&extra_citation),
                simple_answer(&extra_citation, "d1"),
            )
            .is_err()
        );
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
    fn linear_else_if_chain_renders_inline_at_its_position_with_independent_outcomes() {
        let packet = packet();
        let mut raw_answer = answer(&packet);
        raw_answer["steps"] = json!([
            {"kind":"action","meaning":{"text":"Before selection.","evidence":["d1"]}},
            {
                "kind":"decision",
                "meaning":{"text":"Condition alpha holds.","evidence":["d1"]},
                "from":"incomingState",
                "to":"selectedOutcome",
                "interaction":"chooseNextOperation",
                "preparationRefs":["decision-context"],
                "children":[
                    {
                        "kind":"decision",
                        "meaning":{"text":"Nested condition delta holds.","evidence":["d1"]},
                        "children":[{"kind":"action","meaning":{"text":"Run nested delta.","evidence":["d1"]}}],
                        "otherwise":[{
                            "kind":"decision",
                            "meaning":{"text":"Nested condition epsilon holds.","evidence":["d1"]},
                            "children":[{"kind":"return","meaning":{"text":"Run nested epsilon.","evidence":["d1"]}}],
                            "otherwise":[{"kind":"throw","meaning":{"text":"Use nested fallback.","evidence":["d1"]}}]
                        }]
                    },
                    {
                        "kind":"try",
                        "meaning":{"text":"Try the alpha operation.","evidence":["d1"]},
                        "children":[{"kind":"action","meaning":{"text":"Apply alpha.","evidence":["d1"]}}],
                        "otherwise":[{"kind":"throw","meaning":{"text":"Record alpha failure.","evidence":["d1"]}}]
                    }
                ],
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
            },
            {"kind":"action","meaning":{"text":"After the selection boundary.","evidence":["d1"]}}
        ]);
        raw_answer["steps"][1]["preparationRefs"] = json!(["decision-context"]);
        raw_answer["preparations"] = json!([{
            "id":"decision-context",
            "title":"Shared decision context",
            "summary":{
                "text":"Preparation shared by the selected decision.",
                "evidence":["d1"],
                "uncertainty":"The exact helper owner is not retained."
            },
            "steps":[]
        }]);
        raw_answer["uncertainties"] = json!(["The runtime dispatcher is not established."]);
        let original_answer = raw_answer.clone();

        let rendered = validate_and_render(&packet, &audit(&packet), raw_answer).unwrap();

        assert_eq!(rendered.answer, original_answer);
        assert_eq!(rendered.html.matches("class=\"first-match\"").count(), 2);
        assert_eq!(
            rendered
                .markdown
                .matches("| Condition | Action summary |")
                .count(),
            2
        );
        let primary_end = rendered.html.find("class=\"technical-details\"").unwrap();
        let primary = &rendered.html[..primary_end];
        let pseudocode_start = primary
            .find("<section id=\"authored-pseudocode\">")
            .unwrap();
        let ordered_start = primary.find("<section id=\"ordered-behavior\">").unwrap();
        let pseudocode = &primary[pseudocode_start..ordered_start];
        let before = pseudocode_start + pseudocode.find("Before selection.").unwrap();
        let alpha = pseudocode_start + pseudocode.find("Condition alpha holds.").unwrap();
        let beta = pseudocode_start + pseudocode.find("Condition beta holds.").unwrap();
        let gamma = pseudocode_start + pseudocode.find("Condition gamma holds.").unwrap();
        let after = pseudocode_start + pseudocode.find("After the selection boundary.").unwrap();
        assert!(before < alpha && alpha < beta && beta < gamma && gamma < after);
        assert!(
            primary
                .find("The runtime dispatcher is not established.")
                .unwrap()
                < alpha
        );
        assert!(primary.contains("Otherwise (none of the conditions above matched)"));
        assert!(primary.contains("href=\"#preparation-decision-context\""));
        assert!(primary.contains("From:</strong> incomingState"));
        assert!(primary.contains("To:</strong> selectedOutcome"));
        assert!(primary.contains("Interaction:</strong> chooseNextOperation"));
        assert!(primary.contains("id=\"step-2\""));
        assert!(primary.contains("id=\"step-2-else-1\""));
        assert!(primary.contains("id=\"step-2-else-1-else-1\""));
        let alpha_outcome = &primary[alpha..beta];
        assert!(alpha_outcome.contains("Nested condition delta holds."));
        assert!(alpha_outcome.contains("Nested condition epsilon holds."));
        assert!(alpha_outcome.contains("Run nested delta."));
        assert!(alpha_outcome.contains("Try the alpha operation."));
        assert!(alpha_outcome.contains("Record alpha failure."));
        assert!(!alpha_outcome.contains("Condition beta holds."));
        assert!(!alpha_outcome.contains("Condition gamma holds."));
        let beta_outcome = &primary[beta..gamma];
        assert!(beta_outcome.contains("Apply beta."));
        assert!(!beta_outcome.contains("Apply alpha."));
        assert!(!beta_outcome.contains("Apply gamma."));
        assert!(primary.contains("Use the default outcome."));
        assert!(primary.contains("not an executable DMN rule"));
        assert!(primary.contains("reevaluation behavior"));
        assert!(rendered.html.contains("Catch / otherwise path"));
        assert!(rendered.html.contains("Otherwise / exit path"));
        assert!(rendered.markdown.contains("#step-2-else-1-then-1"));
        assert!(rendered.markdown.contains("Record alpha failure\\."));
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
        assert!(rendered.html.contains("<summary>Полное резюме</summary>"));
        assert!(rendered.html.contains("Пояснение внутреннего процесса"));
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
        assert!(
            rendered
                .markdown
                .contains("<summary>Полное резюме</summary>")
        );
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

        assert_eq!(rendered.html.matches("<script>").count(), 1);
        assert_eq!(rendered.html.matches("</script>").count(), 1);
        assert!(!rendered.html.contains("<script>alert(1)</script>"));
        assert!(!rendered.html.contains("<img src=x"));
        assert!(
            rendered
                .html
                .contains("&lt;script&gt;alert(1)&lt;/script&gt;")
        );
        assert!(rendered.html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    #[test]
    fn version_1_2_process_outline_pseudocode_and_evidence_links_preserve_authored_bytes() {
        let packet = packet();
        let authored = semantic_answer(&packet);
        let original_bytes = crate::documentation::bytes(&authored).unwrap();
        let rendered = validate_and_render_draft(&packet, &audit(&packet), authored).unwrap();

        assert_eq!(
            crate::documentation::bytes(&rendered.answer).unwrap(),
            original_bytes
        );
        assert!(rendered.html.contains("DRAFT / UNREVIEWED / NOT PUBLISHED"));
        let glossary = rendered.html.find("id=\"glossary\"").unwrap();
        let pseudocode = rendered.html.find("id=\"authored-pseudocode\"").unwrap();
        assert!(pseudocode > glossary);
        assert!(rendered.html.contains("id=\"process-outline\""));
        assert!(rendered.html.contains("href=\"#block-valid-branch\""));
        assert!(rendered.html.contains("href=\"#preparation-shared\""));
        assert!(
            rendered
                .html
                .contains("The runtime request value is not present in the packet.")
        );
        let pseudocode_text = &rendered.html[pseudocode..];
        let accepted = pseudocode_text
            .find("Continue with the accepted request.")
            .unwrap();
        let alternate = pseudocode_text.find("Check the retry outcome.").unwrap();
        assert!(
            accepted < alternate,
            "then branch must precede otherwise branch"
        );

        let ids = html_ids(&rendered.html);
        for target in html_fragment_links(&rendered.html) {
            assert_eq!(
                ids.iter().filter(|id| *id == &target).count(),
                1,
                "fragment link #{target} must resolve exactly once"
            );
        }
        for anchor in [
            "summary",
            "block-valid-branch",
            "predicate-details",
            "preparation-shared",
        ] {
            assert!(rendered.html.contains(&format!("<a href=\"#{anchor}\"")));
        }
    }

    #[test]
    fn pseudocode_flattens_long_first_match_chains_and_keeps_authored_targets_unique() {
        fn collect_ids(steps: &Value, output: &mut Vec<String>) {
            let Some(steps) = steps.as_array() else {
                return;
            };
            for step in steps {
                output.push(step["id"].as_str().unwrap().to_owned());
                collect_ids(&step["children"], output);
                collect_ids(&step["otherwise"], output);
            }
        }

        fn strip_v1_2_fields(value: &mut Value) {
            match value {
                Value::Object(fields) => {
                    for key in ["id", "predicateRef", "glossaryRefs", "preparationRefs"] {
                        fields.remove(key);
                    }
                    for value in fields.values_mut() {
                        strip_v1_2_fields(value);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        strip_v1_2_fields(value);
                    }
                }
                _ => {}
            }
        }

        let packet = packet();
        let mut authored = semantic_answer(&packet);
        let make_action = |id: &str, text: String, shared: bool| {
            let preparation_refs = if shared { json!(["shared"]) } else { json!([]) };
            json!({
                "id":id,
                "kind":"action",
                "glossaryRefs":["request"],
                "meaning":{
                    "text":text,
                    "evidence":["d1"],
                    "glossaryRefs":["request"]
                },
                "preparationRefs":preparation_refs
            })
        };
        let make_terminal = |id: &str, kind: &str, text: &str| {
            json!({
                "id":id,
                "kind":kind,
                "glossaryRefs":["request"],
                "meaning":{
                    "text":text,
                    "evidence":["d1"],
                    "glossaryRefs":["request"]
                }
            })
        };

        let mut nested_otherwise = json!([make_action(
            "nested-chain-fallback",
            "Use the nested chain fallback.".into(),
            true
        )]);
        for branch_number in (1..=3).rev() {
            nested_otherwise = json!([{
                "id":format!("nested-chain-condition-{branch_number}"),
                "kind":"decision",
                "predicateRef":"retry",
                "glossaryRefs":["request"],
                "meaning":{
                    "text":format!("Check nested condition {branch_number}."),
                    "evidence":["d1"],
                    "glossaryRefs":["request"]
                },
                "children":[make_action(
                    &format!("nested-chain-action-{branch_number}"),
                    format!("Apply nested branch {branch_number}."),
                    true
                )],
                "otherwise":nested_otherwise
            }]);
        }
        let nested_chain_root = nested_otherwise.as_array().unwrap()[0].clone();
        let mut otherwise = json!([make_action(
            "chain-fallback",
            "Use the final fallback action.".into(),
            true
        )]);
        for condition_number in (1..=18).rev() {
            let condition_id = format!("chain-condition-{condition_number}");
            let condition_meaning = if condition_number == 9 {
                json!({
                    "text":format!("Condition {condition_number} meaning."),
                    "evidence":["d1"],
                    "glossaryRefs":["request"],
                    "uncertainty":"The runtime condition value is not captured."
                })
            } else {
                json!({
                    "text":format!("Condition {condition_number} meaning."),
                    "evidence":["d1"],
                    "glossaryRefs":["request"]
                })
            };
            let condition_children = if condition_number == 7 {
                json!([
                    make_action(
                        "chain-action-7",
                        "Apply the selected seventh branch.".into(),
                        true
                    ),
                    {
                        "id":"nested-decision",
                        "kind":"decision",
                        "predicateRef":"retry",
                        "glossaryRefs":["request"],
                        "meaning":{
                            "text":"Check the nested outcome.",
                            "evidence":["d1"],
                            "glossaryRefs":["request"]
                        },
                        "children":[{
                            "id":"nested-try",
                            "kind":"try",
                            "glossaryRefs":["request"],
                            "meaning":{
                                "text":"Try the nested operation.",
                                "evidence":["d1"],
                                "glossaryRefs":["request"]
                            },
                            "children":[
                                make_action(
                                    "nested-action-before-loop",
                                    "Apply the nested action before the loop.".into(),
                                    true
                                ),
                                {
                                    "id":"nested-loop",
                                    "kind":"loop",
                                    "glossaryRefs":["request"],
                                    "meaning":{
                                        "text":"Repeat the nested operation.",
                                        "evidence":["d1"],
                                        "glossaryRefs":["request"]
                                    },
                                    "children":[make_terminal(
                                        "nested-loop-return",
                                        "return",
                                        "Return from the nested loop."
                                    )]
                                },
                                nested_chain_root
                            ]
                        }],
                        "otherwise":[make_terminal(
                            "nested-throw",
                            "throw",
                            "Raise for the nested fallback."
                        )]
                    }
                ])
            } else {
                let text = if condition_number == 11 {
                    format!("🧪 café e\u{301} {}", "branch action text ".repeat(12))
                } else {
                    format!("Apply the selected branch {condition_number}.")
                };
                json!([make_action(
                    &format!("chain-action-{condition_number}"),
                    text,
                    true
                )])
            };
            otherwise = json!([{
                "id":condition_id,
                "kind":"decision",
                "predicateRef":"details",
                "glossaryRefs":["request"],
                "meaning":condition_meaning,
                "children":condition_children,
                "otherwise":otherwise
            }]);
        }
        authored["steps"] = json!([
            make_action(
                "before-chain",
                "Run before the decision chain.".into(),
                false
            ),
            otherwise.as_array().unwrap()[0].clone(),
            make_action("after-chain", "Run after the decision chain.".into(), false)
        ]);
        authored["predicates"] = json!([
            authored["predicates"][0].clone(),
            authored["predicates"][1].clone()
        ]);
        let original_answer = authored.clone();
        let expected_ids = {
            let mut ids = Vec::new();
            collect_ids(&authored["steps"], &mut ids);
            ids
        };
        let decoded = serde_json::from_value::<OperationAnswer>(original_answer.clone()).unwrap();
        let rows = pseudocode_rows(&decoded.steps, ReaderLabels::new(&packet));
        let mut chain_starts = BTreeMap::new();
        let mut chain_ends = BTreeMap::new();
        for (position, row) in rows.iter().enumerate() {
            match row {
                PseudocodeRow::ChainStart { scope, .. } => {
                    assert!(chain_starts.insert(*scope, position).is_none());
                }
                PseudocodeRow::ElseIf { scope, .. } => {
                    assert!(
                        chain_starts
                            .get(scope)
                            .is_some_and(|start| *start < position),
                        "else-if scope {scope} must have exactly one preceding chain start"
                    );
                }
                PseudocodeRow::ChainEnd { scope, .. } => {
                    assert!(
                        chain_starts
                            .get(scope)
                            .is_some_and(|start| *start < position),
                        "chain-end scope {scope} must have exactly one preceding chain start"
                    );
                    assert!(chain_ends.insert(*scope, position).is_none());
                }
                _ => {}
            }
        }
        assert_eq!(chain_starts.len(), 2);
        assert_eq!(chain_ends.len(), chain_starts.len());
        for (scope, start) in &chain_starts {
            assert!(start < chain_ends.get(scope).unwrap());
        }

        let rendered = validate_and_render_draft(&packet, &audit(&packet), authored).unwrap();
        assert_eq!(rendered.answer, original_answer);

        let html_start = rendered
            .html
            .find("<section id=\"authored-pseudocode\">")
            .unwrap();
        let html_end = rendered.html[html_start..]
            .find("<section id=\"ordered-behavior\">")
            .map(|offset| html_start + offset)
            .unwrap();
        let pseudocode_html = &rendered.html[html_start..html_end];
        assert_eq!(pseudocode_html.matches("<ul").count(), 1);
        assert_eq!(
            pseudocode_html
                .matches("class=\"pseudocode-row pseudocode-depth-")
                .count(),
            expected_ids.len()
        );
        assert_eq!(pseudocode_html.matches("Else if (").count(), 19);
        assert_eq!(pseudocode_html.matches("Decision chain — Scope").count(), 2);
        assert!(pseudocode_html.contains("pseudocode-depth-2"));
        assert!(!pseudocode_html.contains("pseudocode-depth-3"));
        assert!(pseudocode_html.contains("Scope "));
        assert!(pseudocode_html.contains("End decision chain Scope"));
        assert!(pseudocode_html.contains("Otherwise (none of the conditions above matched)"));
        assert!(pseudocode_html.contains("🧪 café e\u{301}"));
        assert!(pseudocode_html.contains("…</a>"));
        assert!(
            pseudocode_html
                .contains("class=\"pseudocode-uncertainty\" href=\"#block-chain-condition-9\"")
        );
        for omitted_detail in [
            "Shared count preparation",
            "href=\"#preparation-shared\"",
            "href=\"#glossary-request\"",
            "class=\"citations\"",
            "class=\"claim\"",
            "From:",
            "To:",
        ] {
            assert!(
                !pseudocode_html.contains(omitted_detail),
                "pseudocode leaked detail {omitted_detail}"
            );
        }

        let condition_positions = (1..=18)
            .map(|number| {
                pseudocode_html
                    .find(&format!("href=\"#block-chain-condition-{number}\""))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(condition_positions.windows(2).all(|pair| pair[0] < pair[1]));
        let last_condition = *condition_positions.last().unwrap();
        let fallback_label = pseudocode_html[last_condition..]
            .find("Otherwise (none of the conditions above matched)")
            .map(|offset| last_condition + offset)
            .unwrap();
        let fallback_action = pseudocode_html
            .find("href=\"#block-chain-fallback\"")
            .unwrap();
        let chain_end = pseudocode_html[fallback_action..]
            .find("End decision chain Scope")
            .map(|offset| fallback_action + offset)
            .unwrap();
        let following_action = pseudocode_html.find("href=\"#block-after-chain\"").unwrap();
        assert!(last_condition < fallback_label);
        assert!(fallback_label < fallback_action);
        assert!(fallback_action < chain_end && chain_end < following_action);
        let selected_branch = pseudocode_html
            .find("href=\"#block-chain-action-7\"")
            .unwrap();
        let nested_decision = pseudocode_html
            .find("href=\"#block-nested-decision\"")
            .unwrap();
        let nested_action = pseudocode_html
            .find("href=\"#block-nested-action-before-loop\"")
            .unwrap();
        let nested_loop = pseudocode_html.find("href=\"#block-nested-loop\"").unwrap();
        assert!(selected_branch < nested_decision);
        assert!(nested_decision < nested_action && nested_action < nested_loop);
        assert!(pseudocode_html.contains("Protected try path"));
        assert!(pseudocode_html.contains("Loop body"));
        assert!(pseudocode_html.contains("End Scope"));
        for id in &expected_ids {
            assert_eq!(
                pseudocode_html
                    .matches(&format!("class=\"pseudocode-link\" href=\"#block-{id}\""))
                    .count(),
                1,
                "authored step {id} must have one pseudocode link"
            );
        }

        let markdown_start = rendered
            .markdown
            .find("<a id=\"authored-pseudocode\"></a>")
            .unwrap();
        let markdown_end = rendered.markdown[markdown_start..]
            .find("<a id=\"ordered-behavior\"></a>")
            .map(|offset| markdown_start + offset)
            .unwrap();
        let pseudocode_markdown = &rendered.markdown[markdown_start..markdown_end];
        assert!(!pseudocode_markdown.contains("   - "));
        assert!(pseudocode_markdown.contains("End decision chain Scope"));
        assert_eq!(
            pseudocode_markdown
                .matches("Decision chain — Scope")
                .count(),
            2
        );
        for id in &expected_ids {
            assert_eq!(
                pseudocode_markdown
                    .lines()
                    .filter(|line| {
                        line.starts_with("- [**") && line.contains(&format!("](#block-{id})"))
                    })
                    .count(),
                1,
                "authored step {id} must have one Markdown pseudocode link"
            );
        }
        let ids = html_ids(&rendered.html);
        for id in &ids {
            assert_eq!(ids.iter().filter(|candidate| *candidate == id).count(), 1);
        }
        for target in html_fragment_links(&rendered.html) {
            assert_eq!(
                ids.iter().filter(|id| *id == &target).count(),
                1,
                "fragment link #{target} must resolve exactly once"
            );
        }

        let mut legacy = original_answer.clone();
        legacy["schema"] = json!(ANSWER_SCHEMA_V1_0);
        for field in ["glossary", "predicates", "preparations"] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        strip_v1_2_fields(&mut legacy);
        let original_legacy = legacy.clone();
        let legacy_rendered = validate_and_render(&packet, &audit(&packet), legacy).unwrap();
        assert_eq!(legacy_rendered.answer, original_legacy);
        let legacy_ids = html_ids(&legacy_rendered.html);
        for id in &legacy_ids {
            assert_eq!(
                legacy_ids
                    .iter()
                    .filter(|candidate| *candidate == id)
                    .count(),
                1
            );
        }
        for target in html_fragment_links(&legacy_rendered.html) {
            assert_eq!(
                legacy_ids.iter().filter(|id| *id == &target).count(),
                1,
                "legacy fragment link #{target} must resolve exactly once"
            );
        }
    }

    #[test]
    fn containing_source_claims_do_not_attach_to_implicit_method_body_ranges() {
        let source = "class Demo {\n  final String prefix;\n  Demo(String prefix) { this.prefix = prefix; }\n  String prepare(String name) {\n    String chosen = name;\n    if (chosen == null) { chosen = \"anonymous\"; }\n    return prefix + chosen.trim();\n  }\n}\n";
        let symbol = "method:class:demo.Demo#prepare(Ljava/lang/String;)Ljava/lang/String;";
        let (mut packet, mut audit) = process_packet(source, symbol);
        let start = packet["methods"][0]["body"]["startByte"].as_u64().unwrap() as usize;
        let end = packet["methods"][0]["body"]["endByte"].as_u64().unwrap() as usize;
        packet["citations"]["method-alias"] = json!("exact retained prepare declaration");
        packet["citations"]["method-declaration"] = json!("prepare callable declaration");
        packet["methods"][0]["declarationReference"] = json!("method-declaration");
        packet["methods"][0]["evidence"] = json!(["method-declaration", "method-alias"]);
        packet["methods"][0]["body"]["evidence"] = json!(["method-alias"]);
        packet["methodSources"][0]["sourceAliases"] = json!([{
            "reference":"method-alias", "authority":"TRANSFORMED_SOURCE",
            "startByte":source.find("String prepare").unwrap(), "endByte":end,
            "evidence":["method-alias"]
        }]);
        seal(&mut packet);
        audit["records"].as_array_mut().unwrap().push(json!({
            "label":"method-declaration", "kind":"DEPENDENCY", "id":"method-declaration-id",
            "row":{"record":{"sourceIds":["source-root-id"]}}
        }));
        rebind_audit(&packet, &mut audit);
        let mut answer = simple_answer(&packet, "method-alias");
        answer["summary"]["evidence"] = json!(["method-declaration"]);
        answer["steps"][0]["preparationRefs"] = json!(["constructor-prefix"]);
        answer["preparations"] = json!([{
            "id":"constructor-prefix", "title":"Constructor prefix origin",
            "summary":{"text":"The constructor supplies prefix context.","evidence":["source-root"],
                "uncertainty":"No constructor declaration or invocation metadata is retained; the assignment is supported by the displayed class source."},
            "steps":[{"kind":"action","meaning":{
                "text":"Assign the constructor prefix parameter to the field.","evidence":["source-root"]
            }}]
        }]);
        let parsed: OperationAnswer = serde_json::from_value(answer.clone()).unwrap();
        let navigation = source_navigation(
            &packet,
            &audit,
            &BTreeSet::from([
                "source-root".into(),
                "method-alias".into(),
                "method-declaration".into(),
            ]),
            &selected_root_source_references(&packet),
            &parsed,
        );
        let container = navigation
            .locations
            .iter()
            .position(|l| l.reference == "source-root" && l.excerpt == source)
            .unwrap();
        let body = navigation
            .locations
            .iter()
            .position(|l| l.reference == "source-root" && l.excerpt == source[start..end])
            .unwrap();
        let alias = navigation
            .locations
            .iter()
            .position(|l| l.reference == "method-alias")
            .unwrap();
        let constructor_summary = "preparation-constructor-prefix";
        let constructor_step = step_anchor(
            &parsed.preparations[0].steps[0],
            &step_path(&preparation_step_prefix("constructor-prefix"), 1),
        );
        let anchors = |index| {
            navigation
                .citing_blocks
                .get(&index)
                .into_iter()
                .flatten()
                .map(|b| b.anchor.as_str())
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            anchors(container),
            BTreeSet::from([constructor_summary, constructor_step.as_str()])
        );
        assert_eq!(anchors(body), BTreeSet::from(["summary"]));
        assert_eq!(anchors(alias), BTreeSet::from(["step-1"]));
        // The containing source still indexes the body for the source-local
        // diagram; exact alias navigation and the authored answer stay intact.
        assert!(navigation.body_evidence_locations["source-root"].contains(&body));
        assert!(navigation.body_evidence_locations["method-alias"].contains(&alias));
        let labels = ReaderLabels::new(&packet);
        for links in [
            render_source_links_html("source-root", &navigation, labels),
            render_source_links_markdown("source-root", &navigation, labels),
        ] {
            assert!(links.contains(&format!("#source-{}", container + 1)));
            assert!(!links.contains(&format!("#source-{}", body + 1)));
            assert!(!links.contains(&format!("#source-{}", alias + 1)));
        }
        for (reference, expected) in [("method-declaration", body), ("method-alias", alias)] {
            for links in [
                render_source_links_html(reference, &navigation, labels),
                render_source_links_markdown(reference, &navigation, labels),
            ] {
                assert!(links.contains(&format!("#source-{}", expected + 1)));
                assert!(!links.contains(&format!("#source-{}", container + 1)));
            }
        }
        let rendered = validate_and_render(&packet, &audit, answer.clone()).unwrap();
        assert_eq!(rendered.answer, answer);
        let diagram = rendered.process_diagram.as_ref().unwrap();
        assert!(diagram.has_causal_projection);
        assert_eq!(diagram.source_anchor, Some(format!("source-{}", body + 1)));
        assert!(diagram.tree.contains("chosen") && !diagram.tree.contains("this.prefix = prefix"));
        for (text, section_start, section_end) in [
            (
                &rendered.html,
                format!("<li id=\"source-{}\">", body + 1),
                "</li>",
            ),
            (
                &rendered.markdown,
                format!("<a id=\"source-{}\"></a>", body + 1),
                "<a id=\"source-",
            ),
        ] {
            let section = text
                .split_once(&section_start)
                .unwrap()
                .1
                .split(section_end)
                .next()
                .unwrap();
            assert!(section.contains("#summary"));
            assert!(
                !section.contains("#preparation-constructor-prefix")
                    && !section.contains(&format!("#{constructor_step}"))
            );
        }
    }

    #[test]
    fn selected_root_source_projection_uses_one_validated_tree_for_early_return_and_following_action()
     {
        let source = "class Demo {\n  void run() {\n    if (done) {\n      return;\n    }\n    this.finish();\n  }\n}\n";
        let symbol = "method:class:demo.Handler#run()V";
        let (packet, audit) = process_packet(source, symbol);
        let mut answer = simple_answer(&packet, "source-root");
        answer["summary"]["text"] = json!("The selected root has a guarded early return.");
        let rendered = validate_and_render(&packet, &audit, answer).unwrap();
        let diagram = rendered.process_diagram.as_ref().unwrap();

        assert!(diagram.has_causal_projection);
        assert!(diagram.tree.contains("[D] if (done) then"));
        let tree_return = diagram.tree.find("return").unwrap();
        let tree_action = diagram.tree.find("finish").unwrap();
        assert!(tree_return < tree_action);
        assert!(diagram.puml.contains("if (done) then (C01)"));
        let puml_return = diagram.puml.find("return").unwrap();
        let puml_action = diagram.puml.find("finish").unwrap();
        assert!(puml_return < puml_action);
        assert!(
            diagram
                .puml
                .contains("ORIGIN=SOURCE_LOCAL_NOT_EXECUTION_EVIDENCE")
        );
        assert!(rendered.html.contains("Parsed source outline"));
        assert!(rendered.html.contains("Process blocks citing this excerpt"));
        assert!(rendered.html.contains("href=\"#summary\""));
        assert!(rendered.html.contains("href=\"#step-1\""));
        assert!(rendered.html.contains("href=\"#source-"));
    }

    #[test]
    fn published_process_routes_preserve_literal_source_and_answer_paths() {
        // Renderer fixture only: publication admission is independently tested
        // against durable fake-driver author and reviewer records.
        let literal =
            "index.html#anchor https://example.invalid/index.html#anchor process-flow.puml";
        let source = format!(
            "class Demo {{\n  void run() {{\n    // {literal}\n    this.finish();\n  }}\n}}\n"
        );
        let (packet, audit) = process_packet(&source, "method:class:demo.Handler#run()V");
        let mut answer = simple_answer(&packet, "source-root");
        answer["summary"]["text"] = json!(literal);
        let provenance = json!({"snapshot":"saved synthetic snapshot", "reviewer":{"resultDigest":"synthetic renderer result"}, "limitations":["Renderer fixture; no model approval asserted."], "issues":[]});
        let id = "a".repeat(64);
        let rendered =
            validate_and_render_published(&packet, &audit, answer.clone(), &provenance, &id)
                .unwrap();
        assert_eq!(rendered.answer, answer);
        for view in [&rendered.html, &rendered.markdown] {
            assert!(
                view.contains(&format!("// {literal}")),
                "source excerpt must retain its literal comment bytes"
            );
            assert!(view.contains(literal));
            assert!(view.contains("Diagram SVG was not rendered"));
            assert!(!view.contains(PROCESS_DIAGRAM_HTML_MARKER));
            assert!(!view.contains(PROCESS_DIAGRAM_MARKDOWN_MARKER));
        }
        assert!(rendered.html.contains(&format!("href=\"{id}.puml\"")));
        assert!(rendered.markdown.contains(&format!("]({id}.puml)")));
        let diagram = rendered.process_diagram.unwrap();
        assert!(diagram.has_causal_projection);
        assert_eq!(diagram.puml_filename, format!("{id}.puml"));
        let source_anchor = diagram.source_anchor.unwrap();
        assert!(
            diagram
                .puml
                .contains(&format!("ROOT_SOURCE_EXCERPT={id}.html#{source_anchor}"))
        );
        assert!(
            rendered
                .html
                .contains(&format!("href=\"#{source_anchor}\""))
        );
        assert!(rendered.html.contains(&format!("id=\"{source_anchor}\"")));
    }

    #[test]
    fn missing_ambiguous_and_unsupported_root_projection_emit_gap_only_diagrams() {
        let symbol = "method:class:demo.Handler#run()V";
        let source = "class Demo {\n  void run() {\n    this.finish();\n  }\n}\n";
        let (valid_packet, valid_audit) = process_packet(source, symbol);
        let mut cases = Vec::new();

        let mut missing = valid_packet.clone();
        missing["methods"] = json!([]);
        seal(&mut missing);
        let mut missing_audit = valid_audit.clone();
        rebind_audit(&missing, &mut missing_audit);
        cases.push((missing, missing_audit, "ROOT_METHOD_MATCH_MISSING"));

        let mut ambiguous = valid_packet.clone();
        ambiguous["methods"] = json!([valid_packet["methods"][0], valid_packet["methods"][0]]);
        seal(&mut ambiguous);
        let mut ambiguous_audit = valid_audit.clone();
        rebind_audit(&ambiguous, &mut ambiguous_audit);
        cases.push((ambiguous, ambiguous_audit, "ROOT_METHOD_MATCH_AMBIGUOUS"));

        let unsupported = "class Demo {\n  void run() {\n    try {\n      this.finish();\n    } catch (Exception error) {\n      return;\n    }\n  }\n}\n";
        let (unsupported_packet, unsupported_audit) = process_packet(unsupported, symbol);
        cases.push((
            unsupported_packet,
            unsupported_audit,
            "SOURCE_CONTROL_UNSUPPORTED:try",
        ));

        for (packet, audit, reason) in cases {
            let rendered =
                validate_and_render(&packet, &audit, simple_answer(&packet, "source-root"))
                    .unwrap();
            let diagram = rendered.process_diagram.as_ref().unwrap();
            assert!(!diagram.has_causal_projection, "{reason}");
            assert!(diagram.puml.contains(reason), "{}", diagram.puml);
            assert!(!diagram.puml.contains("start\n:Entry:"), "{}", diagram.puml);
        }
    }
}
