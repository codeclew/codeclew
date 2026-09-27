//! Deterministic offline pages and a single atomic publication pointer.
use super::{
    bindings::{self, Bindings, FragmentBinding},
    bytes,
    check::{self, Check},
    contracts, digest, invalid, io_error,
    model::*,
    store::{self, Repository},
};
use crate::{
    canonical,
    error::{ClewError, ErrorCode},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

pub(super) const SUMMARY_TEXT_MAX_BYTES: usize = 2048;

const TEMPLATE: &str = include_str!("../../assets/documentation/template.html");
pub(super) const STYLE: &str = include_str!("../../assets/documentation/style.css");
const SCRIPT: &str = include_str!("../../assets/documentation/app.js");

pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
pub(super) fn html(data: &Value) -> Result<String, ClewError> {
    let payload = serde_json::to_string(data)
        .map_err(io_error)?
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let template = super::language::template(TEMPLATE, data["language"].as_str().unwrap_or("en"));
    Ok(template
        .replace("/*__STYLE__*/", STYLE)
        .replace("/*__SCRIPT__*/", SCRIPT)
        .replace(
            "/*__ANALYSIS_SCRIPT__*/",
            include_str!("../../assets/documentation/analysis.js"),
        )
        .replace("__DOCUMENT_DATA__", &payload))
}

fn supported_refs(
    deps: &[String],
    source_ids: &[String],
    checked: &Check,
    allowed_services: &BTreeSet<String>,
) -> Result<(), ClewError> {
    if deps.is_empty() || deps.len() > 128 || source_ids.is_empty() || source_ids.len() > 32 {
        return Err(invalid(
            "every authored fragment needs bounded dependencies and exact sources",
        ));
    }
    let mut supported = BTreeSet::new();
    for id in deps {
        let dependency = checked.dependencies.get(id).ok_or_else(|| {
            invalid("authored fragment dependency is absent from current context")
        })?;
        if !dependency.service.is_empty() && !allowed_services.contains(&dependency.service) {
            return Err(invalid("fragment crosses an unselected service boundary"));
        }
        supported.extend(dependency.source_ids.iter());
    }
    if source_ids.iter().any(|id| !supported.contains(id)) {
        return Err(invalid(
            "fragment source is not bound to its claimed dependencies",
        ));
    }
    Ok(())
}

fn validate_summary_text(text: &str) -> Result<(), ClewError> {
    if text.trim().is_empty() || text.contains(['`', '<']) || text.len() > SUMMARY_TEXT_MAX_BYTES {
        return Err(invalid(format!(
            "summary.text must be nonblank plain prose of at most {SUMMARY_TEXT_MAX_BYTES} UTF-8 bytes, without backticks or '<'"
        )));
    }
    Ok(())
}

pub(super) fn sequence_event_kinds(flow_kind: &str) -> &'static [&'static str] {
    match flow_kind {
        "IF" | "TRY" => &["alt"],
        "DEFERRED" => &["opt"],
        "LOOP" => &["loop"],
        "FINALLY" | "BREAK" | "CONTINUE" => &["note"],
        "RETURN" | "THROW" => &["return", "note"],
        _ => &[],
    }
}

pub(super) fn sequence_skipped(checked: &Check, subject: &str, operation_id: &str) -> bool {
    let service_summary = subject.starts_with("service:")
        && (super::sections::contains(operation_id) || super::notes::is_root(operation_id));
    super::dataflow::is_root(checked, subject, operation_id)
        || service_summary
        || super::processes::overview(checked, subject, operation_id)
}

/// The source-backed structural branches the renderer requires for one
/// selected operation. Keep service symbol matching and scenario first-edge
/// selection here so job guidance and validation use the same scope.
pub(super) fn required_sequence_flows<'a>(
    checked: &'a Check,
    subject: &str,
    operation_id: &str,
) -> Result<Vec<&'a Observation>, ClewError> {
    let (kind, id) = subject
        .split_once(':')
        .ok_or_else(|| invalid("subject must be service:<id> or scenario:<id>"))?;
    if sequence_skipped(checked, subject, operation_id) {
        return Ok(Vec::new());
    }
    let is_required_flow = |flow_kind: &str| !sequence_event_kinds(flow_kind).is_empty();
    match kind {
        "service" => {
            let evidence = checked
                .services
                .get(id)
                .ok_or_else(|| invalid("service evidence is unresolved"))?;
            let entry = evidence
                .entrypoints
                .iter()
                .find(|entry| entry.id == operation_id)
                .ok_or_else(|| invalid("operation entrypoint disappeared"))?;
            Ok(evidence
                .observations
                .values()
                .filter(|observation| {
                    observation.kind == "FLOW"
                        && observation.symbol == entry.symbol
                        && observation.normalized["kind"]
                            .as_str()
                            .is_some_and(is_required_flow)
                })
                .collect())
        }
        "scenario" => {
            let scenario = checked
                .scenarios
                .get(id)
                .ok_or_else(|| invalid("unknown scenario"))?;
            Ok(scenario
                .steps
                .iter()
                .filter(|step| is_required_flow(&step.kind))
                .filter_map(|step| step.dependency_ids.first())
                .filter_map(|dependency_id| checked.dependencies.get(dependency_id))
                .collect())
        }
        _ => Err(invalid("unsupported narrative subject")),
    }
}

pub fn validate(n: &Narrative, checked: &Check) -> Result<(), ClewError> {
    if n.schema != "codeclew-documentation-narrative/1.3" {
        return Err(invalid(
            "only codeclew-documentation-narrative/1.3 is supported; author the current contract",
        ));
    }
    if n.context_digest != checked.context_digest {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "narrative must bind the current contextDigest; refresh context and review affected fragments",
        ));
    }
    let (kind, id) = n
        .subject
        .split_once(':')
        .ok_or_else(|| invalid("subject must be service:<id> or scenario:<id>"))?;
    if !store::valid_id(id) {
        return Err(invalid("invalid narrative subject"));
    }
    let (expected, allowed): (BTreeSet<String>, BTreeSet<String>) = match kind {
        "service" => {
            let _service = checked
                .services
                .get(id)
                .ok_or_else(|| invalid("service evidence is unresolved"))?;
            (
                super::notes::expected(checked, id),
                BTreeSet::from([id.into()]),
            )
        }
        "scenario" => {
            let scenario = checked
                .scenarios
                .get(id)
                .ok_or_else(|| invalid("unknown scenario"))?;
            (
                super::processes::expected(checked, id),
                scenario.steps.iter().map(|s| s.service.clone()).collect(),
            )
        }
        _ => return Err(invalid("unsupported narrative subject")),
    };
    let mut covered = BTreeSet::new();
    for o in &n.operations {
        super::language::validate(o.documentation_language.as_deref())?;
        if !expected.contains(&o.id) || !covered.insert(o.id.clone()) || o.title.trim().is_empty() {
            return Err(invalid("duplicate or out-of-scope operation"));
        }
        validate_summary_text(&o.summary.text)?;
        super::visuals::validate_structure(&o.visuals)?;
        if (o.dataflow.is_some() || super::notes::is_root(&o.id)) && !o.visuals.is_empty() {
            return Err(invalid(
                "visual artifacts require a service section, process or operation",
            ));
        }
        for visual in &o.visuals {
            for claim in super::visuals::fragments(visual) {
                supported_refs(&claim.dependency_ids, &claim.source_ids, checked, &allowed)?;
            }
        }
        if super::dataflow::is_root(checked, &n.subject, &o.id) {
            if !o.events.is_empty()
                || !o.explanation.is_empty()
                || !o.interface_contracts.is_empty()
                || !o.findings.is_empty()
                || !o.participants.is_empty()
                || o.overview_diagram.is_some()
                || o.assessment.is_some()
            {
                return Err(invalid(
                    "a typed data-flow view cannot also contain a sequence or assessment",
                ));
            }
            super::dataflow::validate_graph(o, checked, &n.subject)?;
            continue;
        } else if o.dataflow.is_some() {
            return Err(invalid(
                "data-flow graph belongs to an explicit saved view root",
            ));
        }
        if super::notes::is_root(&o.id) {
            validate_assessment(o, checked, id, &allowed)?;
        } else {
            if o.assessment.is_some() {
                return Err(invalid("assessment metadata requires a note root"));
            }
            supported_refs(
                &o.summary.dependency_ids,
                &o.summary.source_ids,
                checked,
                &allowed,
            )?;
        }
        if (kind == "service" && (super::sections::contains(&o.id) || super::notes::is_root(&o.id)))
            || super::processes::overview(checked, &n.subject, &o.id)
        {
            if super::processes::overview(checked, &n.subject, &o.id)
                && checked.scenarios[id]
                    .boundaries
                    .iter()
                    .any(|b| !o.boundaries.contains(b))
            {
                return Err(invalid(
                    "process overview must retain every current composition boundary",
                ));
            }
            if !o.events.is_empty()
                || !o.explanation.is_empty()
                || !o.interface_contracts.is_empty()
                || !o.findings.is_empty()
                || !o.participants.is_empty()
                || o.overview_diagram.is_some()
            {
                return Err(invalid(
                    "standard sections contain an evidence-bound summary and limitations; sequence content belongs to operations",
                ));
            }
            continue;
        }
        if o.participants.len() < 2
            || o.participants.len() > 24
            || o.events.is_empty()
            || o.events.len() > 512
        {
            return Err(invalid(
                "sequence requires 2..24 participants and 1..512 events",
            ));
        }
        let mut participants = BTreeMap::new();
        for p in &o.participants {
            if !store::valid_id(&p.id)
                || participants
                    .insert(p.id.clone(), p.service.clone())
                    .is_some()
                || p.label.trim().is_empty()
                || p.service.as_ref().is_some_and(|s| !allowed.contains(s))
            {
                return Err(invalid(
                    "invalid, duplicate, or unselected sequence participant",
                ));
            }
        }
        let mut ids = BTreeSet::new();
        let mut groups = Vec::new();
        if !store::valid_id(&o.summary.id) {
            return Err(invalid("invalid summary fragment ID"));
        }
        ids.insert(o.summary.id.clone());
        for e in &o.events {
            if !store::valid_id(&e.id) || !ids.insert(e.id.clone()) || e.text.len() > 1024 {
                return Err(invalid("unsafe or duplicate sequence fragment ID"));
            }
            supported_refs(&e.dependency_ids, &e.source_ids, checked, &allowed)?;
            if !matches!(
                e.kind.as_str(),
                "message"
                    | "return"
                    | "note"
                    | "alt"
                    | "else"
                    | "loop"
                    | "opt"
                    | "end"
                    | "declared"
            ) {
                return Err(invalid("unsupported sequence event kind"));
            }
            match e.kind.as_str() {
                "alt" | "loop" | "opt" => groups.push(e.kind.as_str()),
                "else" => {
                    if groups.last() != Some(&"alt") {
                        return Err(invalid("else requires an open alternative group"));
                    }
                }
                "end" => {
                    if groups.pop().is_none() {
                        return Err(invalid("sequence has an unmatched end"));
                    }
                }
                _ => {}
            }
            if e.kind == "note"
                && e.from
                    .as_ref()
                    .is_some_and(|id| !participants.contains_key(id))
            {
                return Err(invalid("note references an unknown participant"));
            }
            if matches!(e.kind.as_str(), "message" | "return" | "declared") {
                let from = e
                    .from
                    .as_ref()
                    .and_then(|id| participants.get(id))
                    .ok_or_else(|| invalid("sequence message has no known sender"))?;
                let to =
                    e.to.as_ref()
                        .and_then(|id| participants.get(id))
                        .ok_or_else(|| invalid("sequence message has no known receiver"))?;
                let crosses = from.is_some() && to.is_some() && from != to;
                if crosses
                    && e.kind != "declared"
                    && !(e.kind == "return" && e.interaction.is_some())
                {
                    return Err(invalid(
                        "cross-service arrows must reference a declared interaction",
                    ));
                }
                if e.kind == "declared" || (e.kind == "return" && e.interaction.is_some()) {
                    let id = e
                        .interaction
                        .as_ref()
                        .ok_or_else(|| invalid("declared transition needs an interaction ID"))?;
                    let check = checked
                        .interactions
                        .get(id)
                        .ok_or_else(|| invalid("unknown declared interaction"))?;
                    let dependency = checked
                        .dependencies
                        .get(&format!("interaction:{id}"))
                        .ok_or_else(|| invalid("interaction declaration missing"))?;
                    if e.kind == "return" && dependency.normalized["transport"]["kind"] == "kafka" {
                        return Err(invalid(
                            "Kafka delivery has no synchronous return; declare a separate reply-event interaction",
                        ));
                    }
                    if check.origin == "agent-proposal"
                        || check.from.status != "RESOLVED"
                        || check.to.status != "RESOLVED"
                        || check.call_site.status != "RESOLVED"
                    {
                        return Err(invalid(
                            "declared arrow endpoints or selected call site are unresolved",
                        ));
                    }
                    let (expected_from, expected_to) = if e.kind == "return" {
                        (
                            &dependency.normalized["to"]["service"],
                            &dependency.normalized["from"]["service"],
                        )
                    } else {
                        (
                            &dependency.normalized["from"]["service"],
                            &dependency.normalized["to"]["service"],
                        )
                    };
                    if !e.dependency_ids.contains(&format!("interaction:{id}"))
                        || &json!(from) != expected_from
                        || &json!(to) != expected_to
                    {
                        return Err(invalid(
                            "declared arrow direction or dependency disagrees with the interaction",
                        ));
                    }
                    if kind != "scenario"
                        || !checked.scenarios[id_from_subject(&n.subject)]
                            .steps
                            .iter()
                            .any(|s| {
                                matches!(
                                    s.kind.as_str(),
                                    "DECLARED_HTTP_TRANSITION" | "DECLARED_KAFKA_TRANSITION"
                                ) && s.detail["interaction"] == *id
                            })
                    {
                        return Err(invalid(
                            "declared arrow is not reachable in this scenario context",
                        ));
                    }
                }
            }
        }
        if !groups.is_empty() {
            return Err(invalid("sequence contains an unclosed group"));
        }
        validate_overview(o)?;
        // Source-bound diagrams must preserve known conditions and return branches.
        if kind == "service"
            && checked.services[id]
                .entrypoints
                .iter()
                .find(|entry| entry.id == o.id)
                .is_some_and(|entry| {
                    entry
                        .boundaries
                        .iter()
                        .any(|b| b == "IMPLEMENTATION_BODY_UNAVAILABLE")
                })
        {
            return Err(invalid(
                "entrypoint implementation is unavailable; record an explicit gap instead of inventing its behavior",
            ));
        }
        let mandatory = required_sequence_flows(checked, &n.subject, &o.id)?;
        for dependency in mandatory {
            let expected_kinds =
                sequence_event_kinds(dependency.normalized["kind"].as_str().unwrap_or(""));
            if !o.events.iter().any(|event| {
                event.dependency_ids.contains(&dependency.id)
                    && expected_kinds.contains(&event.kind.as_str())
            }) {
                return Err(invalid(format!(
                    "sequence omits a source-backed condition or return: {}",
                    dependency.id
                )));
            }
        }
        if o.explanation.len() > 128 {
            return Err(invalid("operation explanation exceeds 128 paragraphs"));
        }
        for paragraph in &o.explanation {
            if !store::valid_id(&paragraph.id)
                || !ids.insert(paragraph.id.clone())
                || paragraph.text.trim().is_empty()
                || paragraph.text.len() > 8192
                || paragraph.text.contains(['`', '<'])
                || paragraph.event_ids.is_empty()
                || paragraph
                    .event_ids
                    .iter()
                    .any(|id| !o.events.iter().any(|e| &e.id == id && e.kind != "end"))
            {
                return Err(invalid(
                    "explanation requires unique IDs, plain domain prose and existing diagram steps",
                ));
            }
            supported_refs(
                &paragraph.dependency_ids,
                &paragraph.source_ids,
                checked,
                &allowed,
            )?;
            for event in o
                .events
                .iter()
                .filter(|e| paragraph.event_ids.contains(&e.id))
            {
                if !event
                    .dependency_ids
                    .iter()
                    .all(|id| paragraph.dependency_ids.contains(id))
                    || !event
                        .source_ids
                        .iter()
                        .all(|id| paragraph.source_ids.contains(id))
                {
                    return Err(invalid(
                        "explanation must retain the evidence of every referenced diagram step",
                    ));
                }
            }
        }
        if o.events
            .iter()
            .filter(|e| e.kind != "end")
            .any(|e| !o.explanation.iter().any(|p| p.event_ids.contains(&e.id)))
        {
            return Err(invalid(
                "narrative 1.3 requires a domain explanation covering every diagram step",
            ));
        }
        if o.interface_contracts.len() > 64 {
            return Err(invalid("operation exceeds 64 interface contracts"));
        }
        for contract in &o.interface_contracts {
            if !store::valid_id(&contract.id)
                || !ids.insert(contract.id.clone())
                || contract.title.trim().is_empty()
                || contract.title.len() > 512
                || !matches!(contract.kind.as_str(), "http" | "kafka" | "payload")
                || contract.rows.is_empty()
                || contract.rows.len() > 128
            {
                return Err(invalid(
                    "interface contract requires a unique ID, kind and bounded rows",
                ));
            }
            for row in &contract.rows {
                if !store::valid_id(&row.id)
                    || !ids.insert(row.id.clone())
                    || row.label.trim().is_empty()
                    || row.label.len() > 512
                    || row.value.trim().is_empty()
                    || row.value.len() > 8192
                {
                    return Err(invalid(
                        "contract row requires a unique ID, label and value",
                    ));
                }
                supported_refs(&row.dependency_ids, &row.source_ids, checked, &allowed)?;
            }
        }
        for finding in &o.findings {
            if !store::valid_id(&finding.id)
                || !ids.insert(finding.id.clone())
                || finding.text.trim().is_empty()
            {
                return Err(invalid("invalid or duplicate finding ID"));
            }
            supported_refs(
                &finding.dependency_ids,
                &finding.source_ids,
                checked,
                &allowed,
            )?;
        }
    }
    for (id, reason) in &n.gaps {
        if !expected.contains(id) || !covered.insert(id.clone()) || reason.trim().is_empty() {
            return Err(invalid(
                "gap is duplicate, out of scope, or lacks an actionable reason",
            ));
        }
    }
    if expected.difference(&covered).any(|id| {
        !super::sections::contains(id)
            && !super::notes::is_root(id)
            && id != super::processes::OVERVIEW
            && id != super::dataflow::ROOT
    }) {
        return Err(invalid(
            "full scope requires every discovered entrypoint to have an operation or an explicit gap",
        ));
    }
    Ok(())
}
fn validate_assessment(
    o: &Operation,
    checked: &Check,
    service: &str,
    allowed: &BTreeSet<String>,
) -> Result<(), ClewError> {
    let a = o
        .assessment
        .as_ref()
        .ok_or_else(|| invalid("note root requires assessment metadata"))?;
    let note = checked
        .dependencies
        .get(&format!("note:{}", a.note))
        .ok_or_else(|| invalid("assessment note is unavailable"))?;
    if a.schema != "codeclew-documentation-note-assessment/1.0"
        || o.id != super::notes::root(&a.note)
        || note.service != service
        || note.normalized["original"]["status"] != "CAPTURED"
        || note.normalized["original"]["digest"] != a.note_digest
        || note.normalized["associationDigest"] != a.association_digest
        || !matches!(
            a.outcome.as_str(),
            "CONSISTENT" | "CONTRADICTED" | "HISTORICAL" | "UNKNOWN"
        )
        || a.period.trim().is_empty()
        || a.period.len() > 512
        || !o.summary.dependency_ids.contains(&note.id)
    {
        return Err(invalid(
            "assessment must bind the exact note, association, period and declared outcome",
        ));
    }
    if a.outcome == "HISTORICAL" {
        let sources = checked.sources();
        let revision=a.period.strip_prefix("revision:").ok_or_else(||invalid("historical assessment period must name revision:FULL_SHA captured in its supporting source"))?;
        if o.summary.source_ids.is_empty()
            || o.summary
                .source_ids
                .iter()
                .any(|id| sources.get(id).is_none_or(|s| s.revision != revision))
        {
            return Err(invalid(
                "historical assessment period does not match retained source revisions",
            ));
        }
    }
    if o.summary.source_ids.is_empty() {
        if a.outcome != "UNKNOWN"
            || o.boundaries.is_empty()
            || o.summary.dependency_ids != vec![note.id.clone()]
            || a.proposed_correction.is_some()
        {
            return Err(invalid(
                "an assessment without code evidence must remain UNKNOWN with a limitation and no correction",
            ));
        }
    } else {
        supported_refs(
            &o.summary.dependency_ids,
            &o.summary.source_ids,
            checked,
            allowed,
        )?;
    }
    if let Some(c) = &a.proposed_correction {
        if c.text.trim().is_empty() || c.text.len() > 8192 {
            return Err(invalid("invalid proposed note correction"));
        }
        supported_refs(&c.dependency_ids, &c.source_ids, checked, allowed)?;
    }
    Ok(())
}
fn id_from_subject(subject: &str) -> &str {
    subject.split_once(':').map(|(_, id)| id).unwrap_or("")
}

fn validate_overview(o: &Operation) -> Result<(), ClewError> {
    let Some(d) = &o.overview_diagram else {
        return Ok(());
    };
    if d.nodes.is_empty() || d.nodes.len() > 12 || d.edges.len() > 20 {
        return Err(invalid(
            "overview diagram allows 1..12 nodes and at most 20 edges",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut positions = BTreeSet::new();
    let bound = |refs: &[String]| {
        !refs.is_empty()
            && refs.len() <= 8
            && refs
                .iter()
                .all(|id| o.events.iter().any(|e| e.id == *id && e.kind != "end"))
    };
    for node in &d.nodes {
        if !store::valid_id(&node.id)
            || !ids.insert(&node.id)
            || node.text.trim().is_empty()
            || node.text.chars().count() > 84
            || node.column > 3
            || node.row > 2
            || !positions.insert((node.column, node.row))
            || !o.participants.iter().any(|p| p.id == node.participant)
            || !bound(&node.event_ids)
        {
            return Err(invalid(
                "overview node needs a unique grid position, brief label, participant and retained events",
            ));
        }
    }
    for edge in &d.edges {
        let from = d.nodes.iter().find(|n| n.id == edge.from);
        let to = d.nodes.iter().find(|n| n.id == edge.to);
        if !store::valid_id(&edge.id)
            || !ids.insert(&edge.id)
            || edge.from == edge.to
            || edge.text.chars().count() > 48
            || from.is_none()
            || to.is_none()
            || !bound(&edge.event_ids)
        {
            return Err(invalid(
                "overview edge needs distinct known nodes and retained events",
            ));
        }
        let from = from.unwrap();
        let to = to.unwrap();
        let service = |id: &str| {
            o.participants
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.service.as_ref())
        };
        if service(&from.participant).is_some()
            && service(&to.participant).is_some()
            && service(&from.participant) != service(&to.participant)
            && !o.events.iter().any(|e| {
                edge.event_ids.contains(&e.id)
                    && e.kind == "declared"
                    && e.from.as_ref() == Some(&from.participant)
                    && e.to.as_ref() == Some(&to.participant)
            })
        {
            return Err(invalid(
                "cross-service overview edges must retain the matching declared transition",
            ));
        }
    }
    Ok(())
}

fn overview_refs(o: &Operation, ids: &[String]) -> (Vec<String>, Vec<String>) {
    let events: Vec<_> = o.events.iter().filter(|e| ids.contains(&e.id)).collect();
    (
        events
            .iter()
            .flat_map(|e| e.dependency_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        events
            .iter()
            .flat_map(|e| e.source_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    )
}

fn default_narrative(
    subject: String,
    ids: impl Iterator<Item = String>,
    checked: &Check,
) -> Narrative {
    Narrative {
        schema: "codeclew-documentation-narrative/1.3".into(),
        subject,
        context_digest: checked.context_digest.clone(),
        operations: vec![],
        gaps: ids
            .map(|id| {
                let gap = default_gap(&id);
                (id, gap)
            })
            .collect(),
    }
}

fn default_gap(id: &str) -> String {
    super::sections::REQUIRED
        .iter()
        .find(|(key, _, _)| *key == id)
        .map(|(_, _, purpose)| {
            format!("{purpose} Source-bound section content has not been accepted yet.")
        })
        .unwrap_or_else(|| "Behavior is not yet authored. Load this entrypoint with clew docs context and supply a source-bound sequence.".into())
}

/// Publishing a sibling service materializes empty reader pages for all subjects.
/// Those exact generated gaps are not an authored update to previously absent content.
/// Custom gaps and any authored operation must still participate in conflict checks.
pub(super) fn is_generated_placeholder(narrative: &Narrative) -> bool {
    narrative.schema == "codeclew-documentation-narrative/1.3"
        && narrative.operations.is_empty()
        && narrative
            .gaps
            .iter()
            .all(|(id, gap)| *gap == default_gap(id))
}

fn add_binding(
    out: &mut BTreeMap<String, FragmentBinding>,
    id: String,
    subject: &str,
    value: &impl serde::Serialize,
    deps: &[String],
    sources: &[String],
    checked: &Check,
) -> Result<(), ClewError> {
    if out
        .insert(
            id,
            bindings::fragment(subject, value, deps, sources, checked)?,
        )
        .is_some()
    {
        return Err(invalid("duplicate generated fragment ID"));
    }
    Ok(())
}

pub fn make_bindings(
    checked: &Check,
    narratives: BTreeMap<String, Narrative>,
) -> Result<Bindings, ClewError> {
    let mut fragments = BTreeMap::new();
    for (subject, n) in &narratives {
        for o in &n.operations {
            let prefix = format!("{subject}/{}", o.id);
            add_binding(
                &mut fragments,
                format!("{prefix}/{}", o.summary.id),
                subject,
                &o.summary,
                &o.summary.dependency_ids,
                &o.summary.source_ids,
                checked,
            )?;
            add_binding(
                &mut fragments,
                format!("{prefix}/title"),
                subject,
                &o.title,
                &o.summary.dependency_ids,
                &o.summary.source_ids,
                checked,
            )?;
            for visual in &o.visuals {
                let claims = super::visuals::fragments(visual);
                let deps = claims
                    .iter()
                    .flat_map(|f| f.dependency_ids.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let sources = claims
                    .iter()
                    .flat_map(|f| f.source_ids.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                add_binding(
                    &mut fragments,
                    format!("{prefix}/visual-{}", visual.id),
                    subject,
                    visual,
                    &deps,
                    &sources,
                    checked,
                )?;
            }
            if let Some(g) = &o.dataflow {
                add_binding(
                    &mut fragments,
                    format!("{prefix}/graph"),
                    subject,
                    g,
                    &o.summary.dependency_ids,
                    &o.summary.source_ids,
                    checked,
                )?;
                for (kind, id, value, claim) in g
                    .nodes
                    .iter()
                    .map(|n| {
                        (
                            "node",
                            &n.id,
                            serde_json::to_value(n).map_err(io_error),
                            &n.meaning,
                        )
                    })
                    .chain(g.edges.iter().map(|e| {
                        (
                            "edge",
                            &e.id,
                            serde_json::to_value(e).map_err(io_error),
                            &e.meaning,
                        )
                    }))
                {
                    add_binding(
                        &mut fragments,
                        format!("{prefix}/{kind}-{id}"),
                        subject,
                        &value?,
                        &claim.dependency_ids,
                        &claim.source_ids,
                        checked,
                    )?;
                }
            }
            if let Some(a) = &o.assessment {
                let mut deps = o.summary.dependency_ids.clone();
                let mut sources = o.summary.source_ids.clone();
                if let Some(c) = &a.proposed_correction {
                    deps.extend(c.dependency_ids.clone());
                    sources.extend(c.source_ids.clone());
                }
                deps.sort();
                deps.dedup();
                sources.sort();
                sources.dedup();
                add_binding(
                    &mut fragments,
                    format!("{prefix}/assessment"),
                    subject,
                    a,
                    &deps,
                    &sources,
                    checked,
                )?;
            }
            for p in &o.participants {
                add_binding(
                    &mut fragments,
                    format!("{prefix}/actor-{}", p.id),
                    subject,
                    p,
                    &o.summary.dependency_ids,
                    &o.summary.source_ids,
                    checked,
                )?;
            }
            for e in &o.events {
                add_binding(
                    &mut fragments,
                    format!("{prefix}/{}", e.id),
                    subject,
                    e,
                    &e.dependency_ids,
                    &e.source_ids,
                    checked,
                )?;
            }
            if let Some(diagram) = &o.overview_diagram {
                for node in &diagram.nodes {
                    let (deps, sources) = overview_refs(o, &node.event_ids);
                    add_binding(
                        &mut fragments,
                        format!("{prefix}/overview/{}", node.id),
                        subject,
                        node,
                        &deps,
                        &sources,
                        checked,
                    )?;
                }
                for edge in &diagram.edges {
                    let (deps, sources) = overview_refs(o, &edge.event_ids);
                    add_binding(
                        &mut fragments,
                        format!("{prefix}/overview/{}", edge.id),
                        subject,
                        edge,
                        &deps,
                        &sources,
                        checked,
                    )?;
                }
            }
            for paragraph in &o.explanation {
                add_binding(
                    &mut fragments,
                    format!("{prefix}/{}", paragraph.id),
                    subject,
                    paragraph,
                    &paragraph.dependency_ids,
                    &paragraph.source_ids,
                    checked,
                )?;
            }
            for contract in &o.interface_contracts {
                for row in &contract.rows {
                    add_binding(
                        &mut fragments,
                        format!("{prefix}/{}", row.id),
                        subject,
                        &(
                            contract.id.as_str(),
                            contract.title.as_str(),
                            contract.kind.as_str(),
                            &contract.boundaries,
                            row,
                        ),
                        &row.dependency_ids,
                        &row.source_ids,
                        checked,
                    )?;
                }
            }
            for f in &o.findings {
                add_binding(
                    &mut fragments,
                    format!("{prefix}/{}", f.id),
                    subject,
                    f,
                    &f.dependency_ids,
                    &f.source_ids,
                    checked,
                )?;
            }
        }
    }
    for (id, context) in &checked.scenarios {
        if checked.dependencies.contains_key(&format!("view:{id}")) {
            let subject = format!("scenario:{id}");
            add_binding(
                &mut fragments,
                format!("{subject}/view-definition"),
                &subject,
                &super::dataflow::page(checked, &subject),
                &context.dependency_ids,
                &[],
                checked,
            )?;
        }
        if checked.dependencies.contains_key(&format!("process:{id}")) {
            let subject = format!("scenario:{id}");
            add_binding(
                &mut fragments,
                format!("{subject}/process-definition"),
                &subject,
                &super::processes::page(checked, &subject),
                &context.dependency_ids,
                &[],
                checked,
            )?;
        }
    }
    for (id, evidence) in &checked.services {
        let subject = format!("service:{id}");
        if let Some(scope) = checked.dependencies.get(&format!("note-scope:{id}")) {
            add_binding(
                &mut fragments,
                format!("{subject}/note-catalogue"),
                &subject,
                &scope.normalized,
                std::slice::from_ref(&scope.id),
                &[],
                checked,
            )?;
        }
        if let Some(scope) = checked.dependencies.get(&format!("entity-scope:{id}")) {
            add_binding(
                &mut fragments,
                format!("{subject}/entity-catalogue"),
                &subject,
                &scope.normalized,
                std::slice::from_ref(&scope.id),
                &scope.source_ids,
                checked,
            )?;
        }
        if let Some(scope) = evidence
            .observations
            .values()
            .find(|o| o.kind == "SOURCE_SCOPE")
        {
            add_binding(
                &mut fragments,
                format!("{subject}/source-scope"),
                &subject,
                &json!({"coverage":evidence.coverage,"catalogue":evidence.entrypoints.iter().map(|e|&e.id).collect::<Vec<_>>()}),
                std::slice::from_ref(&scope.id),
                &[],
                checked,
            )?;
        }
        for contract in evidence
            .observations
            .values()
            .filter(|o| matches!(o.kind.as_str(), "CONTRACT_OPERATION" | "CONTRACT_SCOPE"))
        {
            add_binding(
                &mut fragments,
                format!("{subject}/declared-{}", contract.id),
                &subject,
                &contract.normalized,
                std::slice::from_ref(&contract.id),
                &contract.source_ids,
                checked,
            )?;
        }
        for entry in &evidence.entrypoints {
            add_binding(
                &mut fragments,
                format!("{subject}/{}/entrypoint", entry.id),
                &subject,
                &entry.trigger,
                &entry.dependency_ids,
                &entry.source_ids,
                checked,
            )?;
            for contract in contracts::for_entry(evidence, &entry.id) {
                let mut deps = entry.dependency_ids.clone();
                deps.push(contract.id.clone());
                for scenario in checked.scenarios.values().filter(|scenario| {
                    scenario
                        .steps
                        .iter()
                        .any(|step| step.service == *id && step.symbol == entry.symbol)
                }) {
                    let scenario_subject = format!("scenario:{}", scenario.id);
                    add_binding(
                        &mut fragments,
                        format!(
                            "{scenario_subject}/{}/contract-{}",
                            scenario.id,
                            &contract.id[contract.id.len() - 12..]
                        ),
                        &scenario_subject,
                        &contract.normalized,
                        &deps,
                        &contract.source_ids,
                        checked,
                    )?;
                }

                add_binding(
                    &mut fragments,
                    format!(
                        "{subject}/{}/contract-{}",
                        entry.id,
                        &contract.id[contract.id.len() - 12..]
                    ),
                    &subject,
                    &contract.normalized,
                    &deps,
                    &contract.source_ids,
                    checked,
                )?;
            }
        }
    }
    for (id, interaction) in &checked.interactions {
        let mut deps = vec![format!("interaction:{id}")];
        deps.extend(interaction.from.candidates.clone());
        deps.extend(interaction.to.candidates.clone());
        deps.extend(interaction.call_site.candidates.clone());
        for e in checked.services.values().flat_map(|s| s.entrypoints.iter()) {
            if interaction.to.candidates.iter().any(|id| {
                checked
                    .dependencies
                    .get(id)
                    .is_some_and(|d| d.symbol == e.symbol)
            }) {
                deps.extend(e.dependency_ids.clone());
            }
        }
        let source_ids = interaction
            .from
            .source_ids
            .iter()
            .chain(interaction.to.source_ids.iter())
            .cloned()
            .collect::<Vec<_>>();
        add_binding(
            &mut fragments,
            format!("interaction:{id}/card"),
            &format!("interaction:{id}"),
            interaction,
            &deps,
            &source_ids,
            checked,
        )?;
        for scenario in checked
            .scenarios
            .values()
            .filter(|s| s.steps.iter().any(|step| step.detail["interaction"] == *id))
        {
            add_binding(
                &mut fragments,
                format!("scenario:{}/interaction-{id}-contract", scenario.id),
                &format!("scenario:{}", scenario.id),
                &json!([
                    interaction.method,
                    interaction.path,
                    interaction.destination
                ]),
                &deps,
                &source_ids,
                checked,
            )?;
        }
    }
    let referenced: BTreeSet<_> = fragments
        .values()
        .flat_map(|f| f.dependencies.keys().cloned())
        .collect();
    let observations: BTreeMap<String, Observation> = referenced
        .into_iter()
        .map(|id| (id.clone(), checked.dependencies[&id].clone()))
        .collect();
    // Retain only the complete source closure reachable from the retained
    // observation references AND the fragment claim references. Unreferenced
    // sources are excluded, while every source a current or fragment claim
    // resolves to is preserved at its exact version.
    let reachable_sources: BTreeSet<String> = observations
        .values()
        .flat_map(|observation| observation.source_ids.iter().cloned())
        .chain(
            fragments
                .values()
                .flat_map(|fragment| fragment.sources.keys().cloned()),
        )
        .collect();
    let all_sources = checked.sources();
    let retained_sources: BTreeMap<String, Source> = all_sources
        .into_iter()
        .filter(|(id, _)| reachable_sources.contains(id))
        .collect();
    let mut binding = Bindings {
        documentation_language: None,
        influence_scopes: BTreeMap::new(),
        schema: "codeclew-documentation-bindings/1.4".into(),
        input_digest: checked.input_digest.clone(),
        renderer: RENDERER.into(),
        extractor: EXTRACTOR.into(),
        revisions: checked
            .services
            .iter()
            .map(|(id, e)| (id.clone(), e.revision.clone()))
            .collect(),
        coverage: checked
            .services
            .iter()
            .map(|(id, e)| {
                (
                    id.clone(),
                    json!({"coverage":e.coverage,"boundaries":e.boundaries}),
                )
            })
            .collect(),
        catalogues: checked
            .services
            .iter()
            .map(|(id, e)| {
                (
                    id.clone(),
                    e.entrypoints.iter().map(|e| e.id.clone()).collect(),
                )
            })
            .collect(),
        fragments,
        observations,
        narratives,
        output_hashes: BTreeMap::new(),
        retained_sources,
        section_states: BTreeMap::new(),
        target_revisions: BTreeMap::new(),
        update_failures: BTreeMap::new(),
        accepted_versions: BTreeMap::new(),
    };
    super::status::update_states(&mut binding, checked);
    Ok(binding)
}

// Keep the explicit page inputs visible here; bundling them would broaden this
// focused renderer cleanup into an unrelated data-model refactor.
#[allow(clippy::too_many_arguments)]
fn page_data(
    subject: &str,
    title: &str,
    subtitle: &str,
    n: &Narrative,
    checked: &Check,
    suppress: &BTreeSet<String>,
    state_diagram: Option<&str>,
    lifecycle: &[LifecycleArtifact],
    activity_transitions: Option<&str>,
) -> Value {
    let service_id = subject.strip_prefix("service:");
    let catalogue = service_id
        .and_then(|id| checked.services.get(id))
        .map(|s| s.entrypoints.clone())
        .unwrap_or_default();
    let selected_services: BTreeSet<_> = if let Some(id) = service_id {
        BTreeSet::from([id.to_owned()])
    } else {
        checked
            .scenarios
            .get(id_from_subject(subject))
            .map(|s| s.steps.iter().map(|step| step.service.clone()).collect())
            .unwrap_or_default()
    };
    let contract_rows: Vec<_> = selected_services
        .iter()
        .filter_map(|id| checked.services.get(id))
        .flat_map(|s| {
            s.observations
                .values()
                .filter(|o| {
                    o.kind == "CONTRACT_OPERATION"
                        && (service_id.is_some()
                            || checked.scenarios.get(id_from_subject(subject)).is_some_and(
                                |scenario| {
                                    s.entrypoints.iter().any(|entry| {
                                        o.normalized["entrypoint"] == entry.id
                                            && scenario.steps.iter().any(|step| {
                                                step.service == s.service
                                                    && step.symbol == entry.symbol
                                            })
                                    })
                                },
                            ))
                })
                .cloned()
        })
        .collect();
    let process_candidates = service_id
        .and_then(|id| checked.services.get(id))
        .map(|evidence| {
            let catalog = super::process_candidates::catalog(evidence, &BTreeSet::new(), suppress)
                .expect("an empty explicit selection cannot name an invalid declaration");
            let internal: Vec<_> = catalog
                .records
                .into_iter()
                .filter(|row| row["lane"] == "internal")
                .take(8)
                .collect();
            let omitted = catalog.summary["internalCandidateCount"]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(internal.len() as u64);
            json!({"summary":catalog.summary,"internal":internal,"omittedInternal":omitted,"suppressed":suppress})
        });
    let saved_processes: Vec<_> = checked
        .dependencies
        .values()
        .filter(|d| {
            d.kind == "PROCESS_DEFINITION"
                && service_id.is_some_and(|id| {
                    let definition = &d.normalized["definition"];
                    definition["root"]["service"] == id
                        || definition["process"]["participants"]
                            .as_array()
                            .is_some_and(|participants| {
                                participants.iter().any(|service| service == id)
                            })
                })
        })
        .map(|d| {
            let definition = &d.normalized["definition"];
            let id = definition["id"].as_str().unwrap_or("");
            json!({"id":id,"title":definition["title"],"trigger":definition["process"]["trigger"],
            "href":format!("../scenarios/{id}.html#process-overview"),"status":"AWAITING_AUTHORING",
            "boundaries":checked.scenarios.get(id).map(|s| &s.boundaries)})
        })
        .collect();
    let mut sources = BTreeSet::new();
    if let Some(preview) = &process_candidates {
        sources.extend(
            preview["internal"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|row| row["sourceIds"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    for o in &n.operations {
        sources.extend(o.summary.source_ids.clone());
        for visual in &o.visuals {
            for claim in super::visuals::fragments(visual) {
                sources.extend(claim.source_ids.iter().cloned());
            }
        }
        if let Some(g) = &o.dataflow {
            sources.extend(
                g.nodes
                    .iter()
                    .flat_map(|n| n.meaning.source_ids.iter().cloned()),
            );
            sources.extend(
                g.edges
                    .iter()
                    .flat_map(|e| e.meaning.source_ids.iter().cloned()),
            );
        }
        if let Some(c) = o
            .assessment
            .as_ref()
            .and_then(|a| a.proposed_correction.as_ref())
        {
            sources.extend(c.source_ids.clone());
        }
        for paragraph in &o.explanation {
            sources.extend(paragraph.source_ids.clone());
        }
        for contract in &o.interface_contracts {
            for row in &contract.rows {
                sources.extend(row.source_ids.clone());
            }
        }
        for e in &o.events {
            sources.extend(e.source_ids.clone());
        }
        for f in &o.findings {
            sources.extend(f.source_ids.clone());
        }
    }
    for e in &catalogue {
        sources.extend(e.source_ids.clone());
    }
    for c in &contract_rows {
        sources.extend(c.source_ids.clone());
    }
    sources.extend(
        checked
            .dependencies
            .values()
            .filter(|d| d.kind == "DOMAIN_ENTITY")
            .filter(|d| {
                d.normalized["entity"]["relations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|r| {
                        r["service"]
                            .as_str()
                            .is_some_and(|id| selected_services.contains(id))
                    })
            })
            .flat_map(|d| d.source_ids.iter().cloned()),
    );
    let all_sources = checked.sources();
    let chosen_sources: BTreeMap<_, _> = sources
        .iter()
        .filter_map(|id| all_sources.get(id).map(|s| (id, s)))
        .collect();
    let mut boundaries: Vec<_> = selected_services
        .iter()
        .filter_map(|id| checked.services.get(id))
        .flat_map(|s| s.boundaries.clone())
        .collect();
    if let Some(scenario) = checked.scenarios.get(id_from_subject(subject)) {
        boundaries.extend(scenario.boundaries.clone());
    }
    boundaries.push(
        "Static source interpretation; runtime activation and execution are not established."
            .into(),
    );
    boundaries.sort();
    boundaries.dedup();
    let analysis_evidence = selected_services.iter().filter_map(|id| {
        checked.services.get(id).map(|e| {
            let provider = e.observations.values().find(|o| o.kind == "SOURCE_SCOPE")
                .map(|o| o.normalized["semantic"]["provider"].clone()).filter(|v| !v.is_null())
                .unwrap_or_else(|| json!({"status": if e.coverage == "SYNTAX" { "NOT_REQUESTED" } else { "NATIVE_ANALYSIS" }}));
            let facts = e.observations.values().filter(|o| o.kind == "SEMANTIC_SYMBOL").collect::<Vec<_>>();
            (id, json!({"revision":e.revision,"extractor":e.extractor,"runtimeMode":e.runtime_mode,"coverage":e.coverage,"provider":provider,"mappedSymbols":facts.len(),"sampleFacts":facts.iter().take(3).map(|o| &o.normalized).collect::<Vec<_>>()}))
        })
    }).collect::<BTreeMap<_,_>>();
    json!({"processCandidates":process_candidates,"savedProcesses":saved_processes,"sourceAuthorities":checked.source_authorities(),"analysisEvidence":analysis_evidence,"view":super::dataflow::page(checked,subject),"relatedViews":checked.dependencies.values().filter(|d|d.kind=="VIEW_DEFINITION" && service_id.is_some_and(|id|d.normalized["definition"]["view"]["services"].as_array().is_some_and(|ss|ss.iter().any(|s|s==id)))).map(|d|json!({"id":d.normalized["definition"]["id"],"title":d.normalized["definition"]["title"],"inputObjects":d.normalized["definition"]["view"]["inputObjects"]})).collect::<Vec<_>>(),"process":super::processes::page(checked,subject),"notes":super::notes::page(checked,subject,n),"sections":service_id.map(|id|super::sections::records(id,Some(n))).unwrap_or_default(),"boundaryInventory":service_id.map(|id|super::sections::inventory(id,checked)),"entities":checked.dependencies.values().filter(|d|d.kind=="DOMAIN_ENTITY").collect::<Vec<_>>(),"subject":subject,"title":title,"subtitle":subtitle,"stateDiagram":state_diagram,"stateDiagramSvg":false,"activityTransitions":activity_transitions,"activityTransitionsSvg":false,"lifecycleOperations":lifecycle.iter().map(|artifact|json!({"name":artifact.name,"tree":artifact.tree,"origin":artifact.origin,"diagramStem":artifact.diagram_stem,"svgAvailable":false})).collect::<Vec<_>>(),"operations":n.operations,"gaps":n.gaps,"catalogue":catalogue,"sources":chosen_sources,"contracts":contract_rows,"revisions":selected_services.iter().filter_map(|id|checked.services.get(id).map(|e|(id,&e.revision))).collect::<BTreeMap<_,_>>(),"boundaries":boundaries,"coverage":selected_services.iter().filter_map(|id|checked.services.get(id).map(|e|(id,&e.coverage))).collect::<BTreeMap<_,_>>(),"interactions":checked.interactions.values().filter(|i|service_id.is_some_and(|id|checked.dependencies[&format!("interaction:{}",i.id)].normalized["from"]["service"]==id||checked.dependencies[&format!("interaction:{}",i.id)].normalized["to"]["service"]==id)||checked.scenarios.get(id_from_subject(subject)).is_some_and(|s|s.dependency_ids.contains(&format!("interaction:{}",i.id)))).collect::<Vec<_>>(),"extractor":EXTRACTOR,"renderer":RENDERER})
}

struct LifecycleArtifact {
    name: String,
    tree: String,
    origin: &'static str,
    diagram_stem: String,
    puml: String,
}

#[derive(Clone, Copy)]
struct ResolvedFlow<'a> {
    observation: &'a Observation,
    events: &'a Value,
}

struct AutoFlow {
    puml: String,
    tree: String,
    origin: &'static str,
    causal: bool,
}

/// If an operation has no authored events, produce an auto PlantUML activity
/// document from the operation's root FLOW evidence. Returns `None` when there
/// is authored content or no usable flow.
fn auto_flow_puml(checked: &Check, flow: ResolvedFlow<'_>, title: &str) -> Option<AutoFlow> {
    let symbol = flow.observation.symbol.as_str();
    let resolved = flow;
    let documentation = &resolved.observation.normalized["documentation"];
    let qualified_flow = super::process_flow::validate_projection(
        super::process_flow::project_flow(resolved.events, documentation, symbol),
    );

    // FLOW is a source-order candidate only. Its v1 authority does not prove
    // that every control transfer was emitted, so causal output requires an
    // exact retained method body that parses and validates in the same scope.
    if qualified_flow.source_eligible() {
        let evidence_gaps = qualified_flow.evidence_gaps.clone();
        let Some(source) = method_source(checked, resolved) else {
            return auto_flow_gap(
                symbol,
                &qualified_flow.projection.entry,
                "flow",
                "FLOW_CONTROL_CAPABILITY_UNVERIFIED",
                evidence_gaps,
                title,
            );
        };
        let Some(mut projection) = super::source_steps::projection(&source, symbol) else {
            return auto_flow_gap(
                symbol,
                &qualified_flow.projection.entry,
                "source",
                "SOURCE_METHOD_NOT_ISOLATED",
                evidence_gaps,
                title,
            );
        };
        projection.source_eligible = true;
        projection.evidence_gaps.extend(evidence_gaps);
        // A parser-reported uncertainty or malformed source structure is an
        // explicit veto. Preserve its named gap; never fall back to causal
        // arrows that can omit a source transfer.
        let validated = super::process_flow::validate_projection(projection);
        let causal = validated
            .projection
            .steps
            .iter()
            .any(|step| !matches!(step, super::process_flow::ProjectionStep::Gap(_)));
        let rendered = super::process_flow::render_validated(validated, title)?;
        return Some(AutoFlow {
            puml: rendered.puml,
            tree: rendered.tree,
            origin: rendered.origin,
            causal,
        });
    }

    let causal = qualified_flow
        .projection
        .steps
        .iter()
        .any(|step| !matches!(step, super::process_flow::ProjectionStep::Gap(_)));
    let rendered = super::process_flow::render_validated(qualified_flow, title)?;
    Some(AutoFlow {
        puml: rendered.puml,
        tree: rendered.tree,
        origin: rendered.origin,
        causal,
    })
}

fn auto_flow_gap(
    symbol: &str,
    entry: &str,
    origin: &'static str,
    reason: &str,
    evidence_gaps: Vec<String>,
    title: &str,
) -> Option<AutoFlow> {
    let mut projection =
        super::process_flow::Projection::source(symbol, entry.to_string(), Vec::new());
    projection.origin = origin;
    projection.evidence_gaps = evidence_gaps;
    projection.noncausal = Some(reason.to_string());
    let validated = super::process_flow::validate_projection(projection);
    let causal = validated
        .projection
        .steps
        .iter()
        .any(|step| !matches!(step, super::process_flow::ProjectionStep::Gap(_)));
    let rendered = super::process_flow::render_validated(validated, title)?;
    Some(AutoFlow {
        puml: rendered.puml,
        tree: rendered.tree,
        origin: rendered.origin,
        causal,
    })
}

/// Look up a method's retained `TRANSFORMED_SOURCE` observation by symbol,
/// preferring the source_text resolved through the `SYMBOL` observation's
/// `source_ids` (the source-linked path used on real evidence) and falling
/// back to the legacy TRANSFORMED_SOURCE observation body.
fn method_source(checked: &Check, flow: ResolvedFlow<'_>) -> Option<String> {
    let symbol = &flow.observation.symbol;
    let service = checked.services.get(&flow.observation.service)?;
    let linked: Vec<_> = flow
        .observation
        .source_ids
        .iter()
        .filter_map(|id| service.sources.get(id))
        .collect();
    if !flow.observation.source_ids.is_empty() {
        return (flow.observation.source_ids.len() == 1 && linked.len() == 1)
            .then(|| linked[0].text.clone());
    }

    // Older captures stored source text on a TRANSFORMED_SOURCE observation.
    // Accept it only within the selected service, symbol and normalized scope.
    let scope = flow.observation.normalized.get("scope");
    let mut legacy = BTreeSet::new();
    for observation in service.observations.values().filter(|observation| {
        observation.kind == "TRANSFORMED_SOURCE"
            && observation.symbol == *symbol
            && observation.normalized.get("scope") == scope
    }) {
        let normalized = &observation.normalized;
        let text = normalized
            .pointer("/documentation/source")
            .or_else(|| normalized.pointer("/source"))
            .and_then(Value::as_str)
            .or_else(|| normalized.as_str());
        if let Some(text) = text {
            legacy.insert(text.to_owned());
        }
    }
    (legacy.len() == 1).then(|| legacy.into_iter().next().unwrap())
}

/// Collect a `.puml` diagram into the bundle and remember it for the batch SVG
/// pre-render (all diagrams are rendered by one renderer process afterwards).
fn insert_diagram(
    files: &mut BTreeMap<String, Vec<u8>>,
    diagrams: &mut Vec<(String, String)>,
    base: String,
    puml: String,
) {
    files.insert(format!("{base}.puml"), puml.as_bytes().to_vec());
    diagrams.push((base, puml));
}

/// Pre-render every collected diagram to SVG in a single renderer invocation
/// and add the `.svg` siblings to the bundle.
fn batch_render_diagrams(
    files: &mut BTreeMap<String, Vec<u8>>,
    diagrams: &[(String, String)],
    jar: Option<&std::path::Path>,
) -> BTreeMap<String, bool> {
    let mut availability: BTreeMap<_, _> = diagrams
        .iter()
        .map(|(base, _)| (base.clone(), false))
        .collect();
    if let Ok(Some(svgs)) = super::plantuml::batch_render_svg(diagrams, jar) {
        for (base, svg) in svgs {
            let valid_svg = std::str::from_utf8(&svg).is_ok_and(|text| text.contains("<svg"));
            if valid_svg {
                files.insert(format!("{base}.svg"), svg);
                availability.insert(base, true);
            }
        }
    }
    availability
}

/// Resolve a root method's flow evidence for a candidate symbol list.
/// SYMBOL observations (with `documentation.events`) live in the service
/// evidence (mirror `check::Walker::walk`), not `checked.dependencies`.
/// Prefer the named service, then all services, then `checked.dependencies`
/// as a fallback.
///
/// A symbol can be captured more than once (e.g. a shallow `BOUNDARY`-only
/// entry plus a fully expanded method flow). When several observations match,
/// the one with the most `documentation.events` is chosen — i.e. the deepest
/// retained flow — so a shallow stub never wins over the expanded method body.
fn resolve_flow<'a>(
    checked: &'a Check,
    service: Option<&str>,
    candidates: &[&str],
) -> Option<ResolvedFlow<'a>> {
    for &symbol in candidates {
        let matches = |o: &Observation| o.kind == "SYMBOL" && o.symbol == symbol;
        let mut obs: Vec<&Observation> = Vec::new();
        if let Some(service) = service {
            if let Some(evidence) = checked.services.get(service) {
                obs.extend(evidence.observations.values().filter(|o| matches(o)));
            }
            if obs.is_empty() {
                obs.extend(
                    checked
                        .dependencies
                        .values()
                        .filter(|o| matches(o) && o.service == service),
                );
            }
        } else {
            for e in checked.services.values() {
                obs.extend(e.observations.values().filter(|o| matches(o)));
            }
            if obs.is_empty() {
                obs.extend(checked.dependencies.values().filter(|o| matches(o)));
            }
        }
        let identities: BTreeSet<_> = obs
            .iter()
            .map(|observation| {
                (
                    observation.service.as_str(),
                    observation.normalized["scope"].as_str().unwrap_or(""),
                )
            })
            .collect();
        if identities.len() != 1 {
            continue;
        }
        if let Some(obs) = deepest_flow(&obs) {
            let events = obs.normalized.pointer("/documentation/events")?;
            return Some(ResolvedFlow {
                observation: obs,
                events,
            });
        }
    }
    None
}

/// Return the matching observation with the most `documentation.events` (the
/// deepest retained flow), or `None` if none carries events.
fn deepest_flow<'a>(obs: &[&'a Observation]) -> Option<&'a Observation> {
    obs.iter()
        .filter(|o| o.normalized.pointer("/documentation/events").is_some())
        .max_by_key(|o| {
            o.normalized
                .pointer("/documentation/events")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
        .copied()
}

/// Resolve a lifecycle operation only when its exact full symbol or exact
/// simple method name identifies one retained service and scope.
fn lifecycle_flow<'a>(checked: &'a Check, operation: &str) -> Option<ResolvedFlow<'a>> {
    let symbol = super::process_states::resolve_operation_symbol(checked, operation)?;
    resolve_flow(checked, None, &[symbol])
}

fn lifecycle_diagram_stem(
    subject: &str,
    operation: &str,
    flow: ResolvedFlow<'_>,
) -> Result<String, ClewError> {
    let identity = digest(&json!({
        "subject": subject,
        "operation": operation,
        "service": flow.observation.service,
        "scope": flow.observation.normalized["scope"],
        "symbol": flow.observation.symbol,
        "observation": flow.observation.id,
    }))?;
    let short_hash = identity.strip_prefix("sha256:").unwrap_or(&identity);
    Ok(format!(
        "{}-lifecycle-{}",
        subject.replace(':', "-"),
        &short_hash[..16]
    ))
}

/// Resolve an operation's root FLOW evidence (`documentation.events` array) and
/// the root method symbol from the checked dependency map, mirroring
/// `check::Walker::walk`, which reads a method's flow from its SYMBOL
/// observation's `documentation.events`. Candidate root symbols: the
/// matching entrypoint's method symbol (for a service), the operation id, and
/// any operation boundary that names the root method.
fn root_flow_events<'a>(
    checked: &'a Check,
    operation: &Operation,
    service: Option<&str>,
) -> Option<ResolvedFlow<'a>> {
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(service) = service
        && let Some(entry) = checked
            .services
            .get(service)
            .and_then(|e| e.entrypoints.iter().find(|ep| ep.id == operation.id))
    {
        candidates.push(entry.symbol.as_str());
    }
    candidates.push(operation.id.as_str());
    candidates.extend(operation.boundaries.iter().map(String::as_str));
    resolve_flow(checked, service, &candidates)
}

/// Resolve an unauthored gap operation's root FLOW evidence by its entrypoint
/// id (gap operations are not present in `operations[]`, so they carry no
/// boundaries; the entrypoint's root method symbol is the only candidate).
fn root_flow_events_by_id<'a>(
    checked: &'a Check,
    id: &str,
    service: Option<&str>,
) -> Option<ResolvedFlow<'a>> {
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(service) = service
        && let Some(entry) = checked
            .services
            .get(service)
            .and_then(|e| e.entrypoints.iter().find(|ep| ep.id == id))
    {
        candidates.push(entry.symbol.as_str());
    }
    candidates.push(id);
    resolve_flow(checked, service, &candidates)
}

struct ProcessRootResolution<'a> {
    service: Option<String>,
    selector_scope: Option<String>,
    observations: Vec<&'a Observation>,
    gap: Option<&'static str>,
}

/// Resolve the explicitly saved process root through its exact selector and
/// selected service. Unlike lifecycle discovery, this path never guesses from
/// a simple method name or searches other services.
fn process_root_resolution<'a>(checked: &'a Check, id: &str) -> ProcessRootResolution<'a> {
    let Some(definition) = checked.dependencies.get(&format!("process:{id}")) else {
        return ProcessRootResolution {
            service: None,
            selector_scope: None,
            observations: Vec::new(),
            gap: Some("PROCESS_DEFINITION_UNAVAILABLE"),
        };
    };
    let root = &definition.normalized["definition"]["root"];
    let Some(service_id) = root["service"].as_str() else {
        return ProcessRootResolution {
            service: None,
            selector_scope: None,
            observations: Vec::new(),
            gap: Some("PROCESS_ROOT_SERVICE_MISSING"),
        };
    };
    let Some(service) = checked.services.get(service_id) else {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope: None,
            observations: Vec::new(),
            gap: Some("PROCESS_ROOT_SERVICE_UNAVAILABLE"),
        };
    };
    let Some(selector_value) = root.get("selector").filter(|value| !value.is_null()) else {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope: None,
            observations: Vec::new(),
            gap: Some("PROCESS_ROOT_SELECTOR_MISSING"),
        };
    };
    let Ok(selector) = serde_json::from_value::<Selector>(selector_value.clone()) else {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope: None,
            observations: Vec::new(),
            gap: Some("PROCESS_ROOT_SELECTOR_INVALID"),
        };
    };
    let selector_scope = selector.scope.clone();
    if selector.language != "java" {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope,
            observations: Vec::new(),
            gap: Some("PROCESS_ROOT_LANGUAGE_UNSUPPORTED"),
        };
    }
    let observations = super::analysis::resolve(Some(&selector), service);
    let gap = match observations.as_slice() {
        [] => Some("PROCESS_ROOT_SELECTOR_UNRESOLVED"),
        [observation]
            if observation
                .normalized
                .pointer("/documentation/events")
                .and_then(Value::as_array)
                .is_some() =>
        {
            None
        }
        [_] => Some("PROCESS_ROOT_FLOW_UNAVAILABLE"),
        _ => Some("PROCESS_ROOT_SELECTOR_AMBIGUOUS"),
    };
    ProcessRootResolution {
        service: Some(service_id.to_owned()),
        selector_scope,
        observations,
        gap,
    }
}

impl ProcessRootResolution<'_> {
    fn flow(&self) -> Option<ResolvedFlow<'_>> {
        if self.gap.is_some() || self.observations.len() != 1 {
            return None;
        }
        let observation = self.observations[0];
        let events = observation.normalized.pointer("/documentation/events")?;
        Some(ResolvedFlow {
            observation,
            events,
        })
    }

    fn metadata(&self, source_record_digests: &BTreeMap<String, String>) -> Value {
        let selected = (self.observations.len() == 1).then(|| self.observations[0]);
        let mut source_ids =
            selected.map_or_else(Vec::new, |observation| observation.source_ids.clone());
        source_ids.sort();
        source_ids.dedup();
        json!({
            "service": self.service,
            "scope": selected
                .and_then(|observation| observation.normalized["scope"].as_str())
                .or(self.selector_scope.as_deref()),
            "symbol": selected.map(|observation| &observation.symbol),
            "observation": selected.map(|observation| &observation.id),
            "observationDigest": selected.map(|observation| &observation.digest),
            "sourceIds": source_ids,
            "sourceRecordDigests": source_record_digests,
            "candidates": self.observations.iter().map(|observation| json!({
                "service": observation.service,
                "scope": observation.normalized["scope"],
                "symbol": observation.symbol,
                "observation": observation.id,
                "observationDigest": observation.digest,
            })).collect::<Vec<_>>(),
        })
    }
}

type ProcessRootSourceRecords = (BTreeMap<String, Source>, BTreeMap<String, String>);
type ProcessOutlineProjection = (Value, BTreeMap<String, Source>, Option<(String, String)>);

fn process_root_source_records(
    checked: &Check,
    root: &ProcessRootResolution<'_>,
) -> Result<ProcessRootSourceRecords, ClewError> {
    let mut records = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let (Some(service_id), [observation]) = (root.service.as_deref(), root.observations.as_slice())
    else {
        return Ok((records, digests));
    };
    let Some(service) = checked.services.get(service_id) else {
        return Ok((records, digests));
    };
    let mut ids = observation.source_ids.clone();
    ids.sort();
    ids.dedup();
    for id in ids {
        if let Some(source) = service.sources.get(&id) {
            digests.insert(id.clone(), digest(source)?);
            records.insert(id, source.clone());
        }
    }
    Ok((records, digests))
}

fn process_outline_diagram_stem(subject: &str, root: &Value) -> Result<String, ClewError> {
    let identity = digest(&json!({"subject":subject,"root":root}))?;
    let short_hash = identity.strip_prefix("sha256:").unwrap_or(&identity);
    Ok(format!(
        "{}-process-outline-{}",
        subject.replace(':', "-"),
        &short_hash[..16]
    ))
}

fn process_outline_from_root(
    checked: &Check,
    subject: &str,
    title: &str,
    root: &ProcessRootResolution<'_>,
) -> Result<ProcessOutlineProjection, ClewError> {
    let (sources, source_record_digests) = process_root_source_records(checked, root)?;
    let root_metadata = root.metadata(&source_record_digests);
    if let Some(reason) = root.gap {
        return Ok((
            process_outline_gap(reason, root_metadata),
            BTreeMap::new(),
            None,
        ));
    }
    let Some(flow) = root.flow() else {
        return Ok((
            process_outline_gap("PROCESS_ROOT_FLOW_UNAVAILABLE", root_metadata),
            BTreeMap::new(),
            None,
        ));
    };
    let source_ids = root_metadata["sourceIds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if source_ids.is_empty() || sources.len() != source_ids.len() {
        return Ok((
            process_outline_gap("PROCESS_ROOT_SOURCE_UNAVAILABLE", root_metadata),
            BTreeMap::new(),
            None,
        ));
    }
    let Some(generated) = auto_flow_puml(checked, flow, title) else {
        return Ok((
            process_outline_gap("PROCESS_FLOW_PROJECTION_UNAVAILABLE", root_metadata),
            BTreeMap::new(),
            None,
        ));
    };
    let stem = process_outline_diagram_stem(subject, &root_metadata)?;
    let causal = generated.causal;
    let outline = json!({
        "status":"STATIC_SOURCE_OUTLINE",
        "authority":"STATIC_SOURCE_STRUCTURE_NOT_REVIEWED",
        "root":root_metadata,
        "sourceIds":source_ids,
        "tree":generated.tree,
        "origin":generated.origin,
        "causal":causal,
        "diagramStem":stem,
        "pumlAvailable":true,
        "svgAvailable":false,
    });
    Ok((outline, sources, Some((stem, generated.puml))))
}

fn retained_process_outline_matches(
    outline: &Value,
    retained_sources: &Value,
    root: &ProcessRootResolution<'_>,
    current_sources: &BTreeMap<String, Source>,
    source_record_digests: &BTreeMap<String, String>,
) -> bool {
    root.gap.is_none()
        && root.flow().is_some()
        && outline["status"] == "STATIC_SOURCE_OUTLINE"
        && outline["root"] == root.metadata(source_record_digests)
        && retained_sources == &json!(current_sources)
}

fn copy_retained_process_outline(
    repo: &Repository,
    bundle: &str,
    outline: &Value,
    files: &mut BTreeMap<String, Vec<u8>>,
    diagrams: &mut Vec<(String, String)>,
) -> Result<bool, ClewError> {
    let Some(stem) = outline["diagramStem"].as_str() else {
        return Ok(false);
    };
    if stem.is_empty()
        || !stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Ok(false);
    }
    let puml_path = repo.path(&format!("docs/generated/{bundle}/diagrams/{stem}.puml"))?;
    let bounded_artifact = |path: &std::path::Path| {
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > check::PORTABLE_CACHE_MAX_BYTES {
            return None;
        }
        let bytes = fs::read(path).ok()?;
        (bytes.len() as u64 <= check::PORTABLE_CACHE_MAX_BYTES).then_some(bytes)
    };
    let Some(puml) = bounded_artifact(&puml_path) else {
        return Ok(false);
    };
    let Ok(puml_text) = String::from_utf8(puml.clone()) else {
        return Ok(false);
    };
    files.insert(format!("diagrams/{stem}.puml"), puml);
    if outline["svgAvailable"] == true {
        let svg_path = repo.path(&format!("docs/generated/{bundle}/diagrams/{stem}.svg"))?;
        if let Some(svg) = bounded_artifact(&svg_path)
            && std::str::from_utf8(&svg).is_ok_and(|text| text.contains("<svg"))
        {
            files.insert(format!("diagrams/{stem}.svg"), svg);
        }
    }
    diagrams.push((format!("diagrams/{stem}"), puml_text));
    Ok(true)
}

fn process_outline_gap(reason: &str, root: Value) -> Value {
    let source_ids = root.get("sourceIds").cloned().unwrap_or_else(|| json!([]));
    json!({
        "status":"GAP",
        "gap":reason,
        "root":root,
        "sourceIds":source_ids,
        "pumlAvailable":false,
        "svgAvailable":false,
    })
}

pub fn mermaid(o: &Operation) -> String {
    if let Some(g) = &o.dataflow {
        return super::dataflow::mermaid(g);
    }
    fn label(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace(';', "&#59;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace(['\n', '\r'], " ")
    }
    if let Some(d) = &o.overview_diagram {
        let mut out = "flowchart LR\n    %% Source-bound overview; conditional and declared links are not a runtime trace.\n".to_owned();
        for node in &d.nodes {
            out.push_str(&format!(
                "    %% {}: retained events {}\n    {}[\"{}\"]\n",
                node.id,
                node.event_ids.join(", "),
                node.id,
                label(&node.text)
            ));
        }
        for edge in &d.edges {
            out.push_str(&format!(
                "    %% {}: retained events {}\n    {} -->|\"{}\"| {}\n",
                edge.id,
                edge.event_ids.join(", "),
                edge.from,
                label(&edge.text),
                edge.to
            ));
        }
        return out;
    }
    if o.events.len() > 64 {
        return "flowchart LR\n    pending[\"A bounded overview has not been authored. Full source evidence is retained separately.\"]\n".into();
    }
    let mut out="sequenceDiagram\n    autonumber\n    %% Agent-interpreted static source; declared edges are not compiler calls.\n".to_owned();
    for p in &o.participants {
        out.push_str(&format!(
            "    participant {} as {}\n",
            p.id,
            label(&p.label)
        ));
    }
    for e in &o.events {
        out.push_str(&format!(
            "    %% {}: dependencies {}\n",
            e.id,
            e.dependency_ids.join(", ")
        ));
        match e.kind.as_str() {
            "message" | "return" | "declared" => out.push_str(&format!(
                "    {}{}{}: {}{}\n",
                e.from.as_deref().unwrap_or(""),
                if e.kind == "message" { "->>" } else { "-->>" },
                e.to.as_deref().unwrap_or(""),
                if e.kind == "declared" {
                    "[declared] "
                } else {
                    ""
                },
                label(&e.text)
            )),
            "note" => out.push_str(&format!(
                "    Note over {}: {}\n",
                e.from.as_deref().unwrap_or(&o.participants[0].id),
                label(&e.text)
            )),
            "alt" | "else" | "loop" | "opt" => {
                out.push_str(&format!("    {} {}\n", e.kind, label(&e.text)))
            }
            "end" => out.push_str("    end\n"),
            _ => {}
        }
    }
    out
}

pub(super) fn markdown(
    title: &str,
    n: &Narrative,
    states: &BTreeMap<String, SectionState>,
) -> String {
    let mut out = format!(
        "# {}\n\nStatic source interpretation. Declared interactions do not establish runtime routing.\n\n",
        escape(title)
    );
    if n.subject.starts_with("service:") {
        for (id, title, purpose) in super::sections::REQUIRED {
            let content = n.operations.iter().find(|o| o.id == id);
            out.push_str(&format!(
                "## {}\n\n{}\n\n",
                escape(title),
                escape(content.map(|o| o.summary.text.as_str()).unwrap_or(purpose))
            ));
            if let Some(state) = states.get(&format!("{}/{id}", n.subject)) {
                out.push_str(&format!(
                    "Source freshness: {}. Meaning review: {}.\n\n",
                    state.freshness.as_str(),
                    escape(&state.verification)
                ));
            }
            if content.is_none() {
                out.push_str(
                    "Documentation gap: source-bound section content has not been accepted.\n\n",
                );
            }
        }
    }
    for o in &n.operations {
        for visual in &o.visuals {
            out.push_str(&format!("## {}\n\nKind: {}. Source interpretation, not runtime proof.\n\n{}\n\nScope: {}\n\n", escape(&visual.title), escape(&visual.kind), escape(&visual.purpose.text), escape(&visual.scope.text)));
            for claim in super::visuals::fragments(visual).into_iter().skip(2) {
                out.push_str(&format!("- {}\n", escape(&claim.text)));
            }
            for limit in &visual.limitations {
                out.push_str(&format!("\nLimit: {}\n", escape(limit)));
            }
        }
    }
    for o in &n.operations {
        if super::sections::contains(&o.id) || super::notes::is_root(&o.id) {
            continue;
        }
        if let Some(state) = states.get(&format!("{}/{}", n.subject, o.id)) {
            out.push_str(&format!("Source freshness: {}. Meaning review: {}.\n\nContent revisions: {}\n\nTarget revisions: {}\n\n",state.freshness.as_str(),escape(&state.verification),json!(state.content_revisions),json!(state.target_revisions)));
        }
        out.push_str(&format!(
            "## {}\n\n{}\n\n",
            escape(&o.title),
            escape(&o.summary.text),
        ));
        let mut shown = BTreeSet::new();
        for paragraph in o.explanation.iter().filter(|p| !p.detail) {
            if !shown.insert(&paragraph.text) {
                continue;
            }
            out.push_str(&format!("{}\n\n", escape(&paragraph.text)));
        }
        for contract in &o.interface_contracts {
            out.push_str(&format!("### {}\n\nSource-derived {} interface description.\n\n| Element | Value / behavior |\n|---|---|\n", escape(&contract.title), escape(&contract.kind)));
            for row in &contract.rows {
                out.push_str(&format!(
                    "| {} | {} |\n",
                    escape(&row.label).replace('|', "&#124;"),
                    escape(&row.value)
                        .replace('|', "&#124;")
                        .replace('\n', "<br>")
                ));
            }
            for boundary in &contract.boundaries {
                out.push_str(&format!("\n{}\n", escape(boundary)));
            }
            out.push('\n');
        }
        out.push_str(&format!(
            "<details>\n<summary>Implementation details</summary>\n\n```mermaid\n{}```\n\n",
            mermaid(o)
        ));
        let mut shown = BTreeSet::new();
        for paragraph in o.explanation.iter().filter(|p| p.detail) {
            if !shown.insert(&paragraph.text) {
                continue;
            }
            out.push_str(&format!("{}\n\n", escape(&paragraph.text)));
        }
        out.push_str("</details>\n\n");
        for f in &o.findings {
            out.push_str(&format!("- {}\n", escape(&f.text)));
        }
    }
    for (id, gap) in &n.gaps {
        if super::sections::contains(id) {
            continue;
        }
        out.push_str(&format!(
            "## {}\n\nDocumentation gap: {}\n\n",
            escape(id),
            escape(gap)
        ));
    }
    out
}

pub fn publish(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
) -> Result<Value, ClewError> {
    publish_with_failures(repo, incoming, require_complete, BTreeMap::new())
}

pub fn publish_with_failures(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
) -> Result<Value, ClewError> {
    publish_internal(
        repo,
        incoming,
        require_complete,
        failures,
        BTreeMap::new(),
        EvidenceMode::Saved(None),
        None,
        false,
    )
}

/// Refresh current source evidence through the ordinary check/save path before
/// publishing. This is explicit because it may run project analyzers.
pub fn publish_from_current_source(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
) -> Result<Value, ClewError> {
    publish_internal(
        repo,
        incoming,
        require_complete,
        failures,
        BTreeMap::new(),
        EvidenceMode::Refresh,
        None,
        false,
    )
}

/// Render an explicitly selected saved analysis without acquiring current sources.
pub fn publish_from_snapshot(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
    snapshot: &str,
) -> Result<Value, ClewError> {
    publish_internal(
        repo,
        incoming,
        require_complete,
        failures,
        BTreeMap::new(),
        EvidenceMode::Saved(Some(snapshot)),
        None,
        false,
    )
}

pub(super) fn publish_reviewed(
    repo: &Repository,
    narrative: Narrative,
    versions: BTreeMap<String, super::review::AcceptedVersion>,
    snapshot: Option<&str>,
) -> Result<Value, ClewError> {
    let language = versions
        .values()
        .next()
        .and_then(|v| v.external_request.documentation_language.clone());
    if versions
        .values()
        .any(|v| v.external_request.documentation_language != language)
    {
        return Err(invalid(
            "one publication proposal must have one documentation language",
        ));
    }
    publish_reviewed_with_receipt(
        repo,
        narrative,
        versions,
        snapshot,
        language.as_deref(),
        None,
    )
}

/// Publish one reviewed subject with an optional same-lock receipt hook.
/// `requested_language` remains explicit even for proposals containing only gaps.
/// The hook runs under this repository's write lock and must write only to this
/// same repository using the supplied guard; it must not acquire the lock again.
pub(super) fn publish_reviewed_with_receipt(
    repo: &Repository,
    narrative: Narrative,
    versions: BTreeMap<String, super::review::AcceptedVersion>,
    snapshot: Option<&str>,
    requested_language: Option<&str>,
    before_switch: Option<BeforePublicationSwitch<'_>>,
) -> Result<Value, ClewError> {
    if versions.values().any(|version| {
        version.external_request.documentation_language.as_deref() != requested_language
    }) {
        return Err(invalid(
            "review language does not match requested publication language",
        ));
    }
    let receipt_request = PublicationReceiptRequest {
        requested_language: requested_language.map(str::to_owned),
        affected_subjects: BTreeSet::from([narrative.subject.clone()]),
    };
    publish_internal_with_receipt(
        repo,
        vec![narrative],
        false,
        BTreeMap::new(),
        versions,
        EvidenceMode::Saved(snapshot),
        requested_language,
        false,
        Some(receipt_request),
        before_switch,
    )
}

/// Explicit presentation language does not refresh source analysis.
pub fn publish_language(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
    snapshot: Option<&str>,
    refresh: bool,
    language: Option<&str>,
) -> Result<Value, ClewError> {
    publish_language_with_mode(
        repo,
        incoming,
        require_complete,
        failures,
        snapshot,
        refresh,
        language,
        false,
    )
}

/// Render saved evidence and release the resulting immutable snapshot only
/// when the caller explicitly opts in.
// Preserve the established wrapper API rather than grouping unrelated options.
#[allow(clippy::too_many_arguments)]
pub fn publish_language_with_mode(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
    snapshot: Option<&str>,
    refresh: bool,
    language: Option<&str>,
    released: bool,
) -> Result<Value, ClewError> {
    publish_internal(
        repo,
        incoming,
        require_complete,
        failures,
        BTreeMap::new(),
        if refresh {
            EvidenceMode::Refresh
        } else {
            EvidenceMode::Saved(snapshot)
        },
        language,
        released,
    )
}

#[derive(Clone, Copy)]
enum EvidenceMode<'a> {
    Saved(Option<&'a str>),
    Refresh,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PublicationReceipt {
    pub schema: String,
    pub bundle_id: String,
    pub root_index_hash: String,
    pub bindings_hash: String,
    pub publication_hash: String,
    pub effective_gaps: BTreeMap<String, BTreeMap<String, String>>,
    pub requested_language: Option<String>,
    /// UI language used when the request and retained binding omit one.
    pub effective_language: String,
}

type BeforePublicationSwitch<'a> =
    &'a mut (dyn FnMut(&store::WriteLock, &PublicationReceipt) -> Result<(), ClewError> + 'a);

struct PublicationReceiptRequest {
    requested_language: Option<String>,
    affected_subjects: BTreeSet<String>,
}

// Keep the explicit phase inputs aligned with the existing publish wrapper.
#[allow(clippy::too_many_arguments)]
fn publish_internal(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
    versions: BTreeMap<String, super::review::AcceptedVersion>,
    evidence_mode: EvidenceMode<'_>,
    language: Option<&str>,
    released: bool,
) -> Result<Value, ClewError> {
    publish_internal_with_receipt(
        repo,
        incoming,
        require_complete,
        failures,
        versions,
        evidence_mode,
        language,
        released,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn publish_internal_with_receipt(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
    versions: BTreeMap<String, super::review::AcceptedVersion>,
    evidence_mode: EvidenceMode<'_>,
    language: Option<&str>,
    released: bool,
    receipt_request: Option<PublicationReceiptRequest>,
    before_switch: Option<BeforePublicationSwitch<'_>>,
) -> Result<Value, ClewError> {
    super::progress::run("PUBLISH_DOCUMENTATION", || {
        publish_internal_phases(
            repo,
            incoming,
            require_complete,
            failures,
            versions,
            evidence_mode,
            language,
            released,
            receipt_request,
            before_switch,
        )
    })
}

// Keep the explicit phase inputs aligned with the existing publish wrapper.
#[allow(clippy::too_many_arguments)]
fn publish_internal_phases(
    repo: &Repository,
    mut incoming: Vec<Narrative>,
    require_complete: bool,
    mut failures: BTreeMap<String, Value>,
    versions: BTreeMap<String, super::review::AcceptedVersion>,
    evidence_mode: EvidenceMode<'_>,
    language: Option<&str>,
    released: bool,
    receipt_request: Option<PublicationReceiptRequest>,
    before_switch: Option<BeforePublicationSwitch<'_>>,
) -> Result<Value, ClewError> {
    super::language::validate(language)?;
    let previous = bindings::baseline(repo)?;
    let requested_language = language.map(str::to_owned).or_else(|| {
        previous
            .as_ref()
            .and_then(|(_, b)| b.documentation_language.clone())
    });
    super::language::validate(requested_language.as_deref())?;
    let ui_language = requested_language.as_deref().unwrap_or("en");
    for (key, version) in &versions {
        super::review::validate_accepted_version(version)?;
        let (subject, _) = key
            .split_once('/')
            .ok_or_else(|| invalid("invalid prepared section key"))?;
        let retained = previous
            .as_ref()
            .and_then(|(_, b)| b.narratives.get(subject));
        let generated_after_preparation = version.previous_narrative_digest
            == digest(&Option::<&Narrative>::None)?
            && retained.is_some_and(is_generated_placeholder);
        if digest(&retained)? != version.previous_narrative_digest && !generated_after_preparation {
            return Err(invalid("published content changed after work preparation"));
        }
    }
    if let Some((id, binding)) = &previous {
        bindings::verify_outputs(repo, id, binding)?;
    }
    let suppress = super::process_candidates::load_suppress(repo)?;
    let root = repo.path("docs/index.html")?;
    let previous_bytes = if root.exists() {
        Some(fs::read(&root).map_err(io_error)?)
    } else {
        None
    };
    let refreshing = matches!(evidence_mode, EvidenceMode::Refresh);
    let (mut checked, selected_snapshot) = match evidence_mode {
        EvidenceMode::Saved(snapshot) => Check::retained(repo, snapshot, &BTreeSet::new())?,
        EvidenceMode::Refresh => super::check::run_and_save_selected(repo, &BTreeSet::new(), None)?,
    };
    let snapshot = Some(selected_snapshot.as_str());
    // Re-rendering must restore registered-input scope observations for retained
    // accepted operations as well as newly submitted ones. Their absence is not
    // evidence that unchanged inputs became stale.
    let mut scope_versions = previous
        .as_ref()
        .map(|(_, binding)| binding.accepted_versions.clone())
        .unwrap_or_default();
    scope_versions.extend(versions.clone());
    super::review::scopes(repo, &mut checked, scope_versions.into_values())?;
    for version in versions.values() {
        if version.source_revisions.iter().any(|(id, revision)| {
            checked
                .services
                .get(id)
                .is_none_or(|e| &e.revision != revision)
        }) || digest(&super::work::capture_inputs(
            repo,
            &version.external_request,
        )?)? != version.external_fingerprint
        {
            return Err(invalid("reviewed work inputs changed before publication"));
        }
    }
    for narrative in &mut incoming {
        if narrative
            .operations
            .iter()
            .all(|o| versions.contains_key(&format!("{}/{}", narrative.subject, o.id)))
            && !versions.is_empty()
        {
            narrative.context_digest = checked.context_digest.clone();
        }
    }
    let services = repo.services()?;
    let scenarios = repo.scenarios()?;
    let mut fresh = BTreeMap::new();
    for id in checked.services.keys() {
        let subject = format!("service:{id}");
        fresh.insert(
            subject.clone(),
            default_narrative(
                subject,
                super::notes::expected(&checked, id).into_iter(),
                &checked,
            ),
        );
    }
    for id in services.keys() {
        let subject = format!("service:{id}");
        fresh
            .entry(subject.clone())
            .or_insert_with(|| default_narrative(subject, super::sections::ids(), &checked));
    }
    for id in scenarios.keys() {
        let subject = format!("scenario:{id}");
        fresh.insert(
            subject.clone(),
            default_narrative(
                subject,
                super::processes::expected(&checked, id).into_iter(),
                &checked,
            ),
        );
    }
    let mut accepted = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut updated_gaps = BTreeMap::new();
    for n in incoming {
        let Some(target) = fresh.get_mut(&n.subject) else {
            failures.insert(n.subject.clone(), json!({"reason":"SUBJECT_UNAVAILABLE","nextAction":"Register and capture this subject before authoring."}));
            continue;
        };
        let expected: BTreeSet<String> = if let Some(id) = n.subject.strip_prefix("service:") {
            super::notes::expected(&checked, id)
        } else {
            super::processes::expected(&checked, id_from_subject(&n.subject))
        };
        if !seen.insert(n.subject.clone()) {
            // Duplicate subjects are not merged in input order; retain all original content.
            accepted.retain(|key: &String| !key.starts_with(&format!("{}/", n.subject)));
            target.operations.clear();
            target.gaps = expected
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        "Duplicate subject proposals; resubmit one coherent input.".into(),
                    )
                })
                .collect();
            failures.insert(n.subject.clone(), json!({"reason":"DUPLICATE_SUBJECT","nextAction":"Supply one proposal per subject."}));
            continue;
        }
        let mut envelope = n.clone();
        envelope.operations.clear();
        envelope.gaps = expected
            .iter()
            .map(|id| (id.clone(), "Operation validation follows.".into()))
            .collect();
        if let Err(error) = validate(&envelope, &checked) {
            failures.insert(
                n.subject.clone(),
                json!({"reason":error.code,"nextAction":error.message}),
            );
            continue;
        }
        let mut operation_ids = BTreeSet::new();
        let duplicate_ids: BTreeSet<_> = n
            .operations
            .iter()
            .filter(|o| !operation_ids.insert(o.id.clone()))
            .map(|o| o.id.clone())
            .collect();
        for operation in &n.operations {
            let key = format!("{}/{}", n.subject, operation.id);
            let mut candidate = n.clone();
            candidate.operations = vec![operation.clone()];
            candidate.gaps = expected
                .iter()
                .filter(|id| **id != operation.id)
                .map(|id| {
                    (
                        id.clone(),
                        "Outside this operation proposal; retained or explicitly incomplete."
                            .into(),
                    )
                })
                .collect();
            let validation = if duplicate_ids.contains(&operation.id) {
                Err(invalid("duplicate operation proposal"))
            } else {
                validate(&candidate, &checked)
            };
            match validation {
                Ok(()) => {
                    accepted.insert(key);
                    target.operations.push(operation.clone());
                }
                Err(error) => {
                    failures.insert(key, json!({"reason":error.code,"nextAction":error.message}));
                }
            }
        }
        for (id, reason) in n.gaps {
            if expected.contains(&id) && !reason.trim().is_empty() && reason.len() <= 8192 {
                updated_gaps.insert((n.subject.clone(), id.clone()), reason.clone());
                target.gaps.insert(id, reason);
            } else {
                failures.insert(format!("{}/gap-{id}",n.subject),json!({"reason":"INVALID_GAP","nextAction":"Name an in-scope operation and an actionable missing-information reason."}));
            }
        }
        target
            .gaps
            .retain(|id, _| !target.operations.iter().any(|o| &o.id == id));
    }
    let mut binding = make_bindings(&checked, fresh.clone())?;
    binding.documentation_language = requested_language.clone();
    let mut narratives = previous
        .as_ref()
        .map(|(_, b)| b.narratives.clone())
        .unwrap_or_default();
    for (subject, n) in &fresh {
        let combined = narratives
            .entry(subject.clone())
            .or_insert_with(|| n.clone());
        for operation in &n.operations {
            combined.operations.retain(|old| old.id != operation.id);
            combined.operations.push(operation.clone());
        }
        combined.operations.sort_by(|a, b| a.id.cmp(&b.id));
        combined.gaps = n
            .gaps
            .iter()
            .filter(|(id, _)| !combined.operations.iter().any(|o| &o.id == *id))
            .map(|(id, v)| {
                (
                    id.clone(),
                    updated_gaps
                        .get(&(subject.clone(), id.clone()))
                        .or_else(|| combined.gaps.get(id))
                        .unwrap_or(v)
                        .clone(),
                )
            })
            .collect();
    }
    // A service whose first capture failed still has a reader-visible actionable gap.
    for id in services.keys() {
        let subject = format!("service:{id}");
        narratives.entry(subject.clone()).or_insert_with(|| {
            default_narrative(
                subject,
                std::iter::once("source-unavailable".into()),
                &checked,
            )
        });
    }
    if let Some((_, old)) = &previous {
        for (key, failure) in &old.update_failures {
            if key.contains('/') && !accepted.contains(key) {
                failures
                    .entry(key.clone())
                    .or_insert_with(|| failure.clone());
            }
        }
        for (id, fragment) in &old.fragments {
            let retained_operation = old.narratives.get(&fragment.subject).is_some_and(|n| {
                n.operations.iter().any(|o| {
                    let key = format!("{}/{}", fragment.subject, o.id);
                    id.starts_with(&format!("{key}/")) && !accepted.contains(&key)
                })
            });
            let unavailable_subject = fragment
                .subject
                .strip_prefix("service:")
                .is_some_and(|id| !checked.services.contains_key(id));
            if retained_operation || unavailable_subject {
                let mut retained = fragment.clone();
                if retained.evidence.is_none() {
                    retained.evidence = Some(bindings::FragmentEvidence {
                        shared_observations: vec![],
                        shared_sources: vec![],
                        revisions: old.revisions.clone(),
                        observations: retained
                            .dependencies
                            .keys()
                            .filter_map(|id| {
                                old.observations.get(id).map(|o| (id.clone(), o.clone()))
                            })
                            .collect(),
                        sources: retained
                            .sources
                            .keys()
                            .filter_map(|id| {
                                old.retained_sources
                                    .get(id)
                                    .map(|s| (id.clone(), s.clone()))
                            })
                            .collect(),
                    });
                }
                binding.fragments.insert(id.clone(), retained);
            }
        }
        for (id, state) in &old.section_states {
            if !accepted.contains(id)
                && (id.contains('/')
                    || id
                        .strip_prefix("service:")
                        .is_some_and(|service| !checked.services.contains_key(service)))
            {
                binding.section_states.insert(id.clone(), state.clone());
            }
        }
        for (id, revision) in &old.revisions {
            binding
                .revisions
                .entry(id.clone())
                .or_insert_with(|| revision.clone());
        }
        for (id, coverage) in &old.coverage {
            binding
                .coverage
                .entry(id.clone())
                .or_insert_with(|| coverage.clone());
        }
        for (id, source) in &old.retained_sources {
            binding
                .retained_sources
                .entry(id.clone())
                .or_insert_with(|| source.clone());
        }
        for (id, observation) in &old.observations {
            binding
                .observations
                .entry(id.clone())
                .or_insert_with(|| observation.clone());
        }
    }
    binding.narratives = narratives.clone();
    if let Some((_, old)) = &previous {
        for (scope, dependencies) in &old.influence_scopes {
            binding
                .influence_scopes
                .entry(scope.clone())
                .or_insert_with(|| dependencies.clone());
        }
        for (key, version) in &old.accepted_versions {
            if !accepted.contains(key) {
                binding
                    .accepted_versions
                    .insert(key.clone(), version.clone());
            }
        }
    }
    super::review::attach(&mut binding, &checked, versions)?;
    // A new accepted child changes its parents in this same atomic publication.
    super::processes::attach_versions(repo, &mut checked, Some(&binding))?;
    for (id, context) in &checked.scenarios {
        if checked.dependencies.contains_key(&format!("view:{id}")) {
            let subject = format!("scenario:{id}");
            binding.fragments.insert(
                format!("{subject}/view-definition"),
                bindings::fragment(
                    &subject,
                    &super::dataflow::page(&checked, &subject),
                    &context.dependency_ids,
                    &[],
                    &checked,
                )?,
            );
        }
        if checked.dependencies.contains_key(&format!("process:{id}")) {
            let subject = format!("scenario:{id}");
            binding.fragments.insert(
                format!("{subject}/process-definition"),
                bindings::fragment(
                    &subject,
                    &super::processes::page(&checked, &subject),
                    &context.dependency_ids,
                    &[],
                    &checked,
                )?,
            );
        }
    }
    binding.observations.extend(
        checked
            .dependencies
            .iter()
            .filter(|(_, d)| d.kind.starts_with("PROCESS_"))
            .map(|(id, d)| (id.clone(), d.clone())),
    );
    bindings::prune_influence_scopes(&mut binding);
    super::status::update_states(&mut binding, &checked);
    super::review::verification(&mut binding);
    // Whole-page freshness is an aggregate; each operation keeps its exact content vector.
    for subject in narratives.keys() {
        let children: Vec<_> = binding
            .section_states
            .iter()
            .filter(|(key, _)| key.starts_with(&format!("{subject}/")))
            .map(|(_, s)| s.clone())
            .collect();
        if let Some(state) = binding.section_states.get_mut(subject) {
            if state.freshness == Freshness::Stale
                || children.iter().any(|s| s.freshness == Freshness::Stale)
            {
                state.freshness = Freshness::Stale;
            } else if children
                .iter()
                .any(|s| s.freshness == Freshness::Unverified)
            {
                state.freshness = Freshness::Unverified;
            }
        }
    }
    if !refreshing {
        // A valid historical snapshot is not proof of current source freshness.
        // Keep stronger stale findings and the independent meaning-review state.
        for state in binding.section_states.values_mut() {
            if state.freshness == Freshness::Current {
                state.freshness = Freshness::Unverified;
            }
            let reason =
                json!({"reason":"PINNED_SNAPSHOT_NOT_REVERIFIED","snapshot":selected_snapshot});
            if !state.reasons.contains(&reason) {
                state.reasons.push(reason);
            }
        }
    }
    let translation_gap_count = requested_language
        .as_deref()
        .map(|language| {
            narratives
                .values()
                .flat_map(|n| &n.operations)
                .filter(|o| o.documentation_language.as_deref() != Some(language))
                .count()
        })
        .unwrap_or(0);
    let incomplete = translation_gap_count > 0
        || !checked.unresolved.is_empty()
        || !failures.is_empty()
        || narratives.values().any(|n| !n.gaps.is_empty())
        || binding
            .section_states
            .values()
            .any(|s| s.freshness != Freshness::Current);
    if require_complete && incomplete {
        let code = if binding
            .section_states
            .values()
            .any(|s| s.freshness == Freshness::Stale)
        {
            ErrorCode::StaleRequiresReslice
        } else {
            ErrorCode::IncompleteSemanticAnalysis
        };
        return Err(ClewError::new(
            code,
            "documentation has retained stale content or explicit gaps; inspect the local outcomes before --require-complete",
        ));
    }
    binding.update_failures = failures.clone();
    binding.output_hashes.clear();
    let mut files = BTreeMap::new();
    let mut diagrams: Vec<(String, String)> = Vec::new();
    let plantuml_jar = std::env::var("PLANTUML_JAR")
        .ok()
        .map(std::path::PathBuf::from);
    let mut cards = String::new();
    let mut pending_pages: Vec<(String, String, Value)> = Vec::new();
    for (subject, n) in &narratives {
        let (kind, id) = subject
            .split_once(':')
            .ok_or_else(|| invalid("invalid retained subject"))?;
        let folder = if kind == "service" {
            "services"
        } else {
            "scenarios"
        };
        let old_data: Option<Value> = if let Some((bundle, b)) = &previous {
            if b.narratives.contains_key(subject) {
                Some(store::read(
                    &repo.path(&format!("docs/generated/{bundle}/{folder}/{id}.json"))?,
                    check::PORTABLE_CACHE_MAX_BYTES,
                )?)
            } else {
                None
            }
        } else {
            None
        };
        let title = services
            .get(id)
            .filter(|_| kind == "service")
            .map(|s| s.title.as_str())
            .or_else(|| {
                scenarios
                    .get(id)
                    .filter(|_| kind == "scenario")
                    .map(|s| s.title.as_str())
            })
            .or_else(|| old_data.as_ref().and_then(|d| d["title"].as_str()))
            .unwrap_or(id);
        let process_state_schema = if kind == "scenario" {
            super::process_states::captured(&checked, id)
        } else {
            None
        };
        let state_diagram = process_state_schema.map(|_| format!("scenario-{id}-states"));
        let activity_puml = process_state_schema
            .and_then(|schema| super::process_states::activity_transitions_puml(schema, &checked));
        let activity_transitions = activity_puml
            .as_ref()
            .map(|_| format!("scenario-{id}-activity-transitions"));
        let lifecycle: Vec<LifecycleArtifact> = if let Some(schema) = process_state_schema {
            let mut out = Vec::new();
            let mut seen = BTreeSet::new();
            for t in &schema.transitions {
                if t.operation.is_empty()
                    || !seen.insert(t.operation.clone())
                    || n.operations
                        .iter()
                        .any(|operation| operation.id == t.operation)
                {
                    continue;
                }
                if let Some(flow) = lifecycle_flow(&checked, &t.operation)
                    && let Some(generated) = auto_flow_puml(&checked, flow, &t.operation)
                {
                    out.push(LifecycleArtifact {
                        name: t.operation.clone(),
                        tree: generated.tree,
                        origin: generated.origin,
                        diagram_stem: lifecycle_diagram_stem(subject, &t.operation, flow)?,
                        puml: generated.puml,
                    });
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            out
        } else {
            Vec::new()
        };
        let mut data = page_data(
            subject,
            title,
            super::reader::text(
                ui_language,
                "Service behavior and explicit evidence boundaries",
                "Поведение сервиса и границы подтверждённых сведений",
            ),
            n,
            &checked,
            &suppress,
            state_diagram.as_deref(),
            &lifecycle,
            activity_transitions.as_deref(),
        );
        if kind == "scenario" && !data["process"].is_null() {
            let root = process_root_resolution(&checked, id);
            let (current_root_sources, current_source_digests) =
                process_root_source_records(&checked, &root)?;
            let root_metadata = root.metadata(&current_source_digests);
            let overview_key = format!("{subject}/{}", super::processes::OVERVIEW);
            let retained_overview = n
                .operations
                .iter()
                .any(|operation| operation.id == super::processes::OVERVIEW)
                && !accepted.contains(&overview_key);
            if retained_overview {
                let retained_outline = old_data
                    .as_ref()
                    .map(|old| &old["processOutline"])
                    .filter(|outline| !outline.is_null());
                let retained_sources = old_data
                    .as_ref()
                    .map(|old| old["processOutlineSources"].clone())
                    .unwrap_or(Value::Null);
                if let Some(outline) = retained_outline.filter(|outline| {
                    retained_process_outline_matches(
                        outline,
                        &retained_sources,
                        &root,
                        &current_root_sources,
                        &current_source_digests,
                    )
                }) {
                    let copied = if let Some((bundle, _)) = &previous {
                        copy_retained_process_outline(
                            repo,
                            bundle,
                            outline,
                            &mut files,
                            &mut diagrams,
                        )?
                    } else {
                        false
                    };
                    if copied {
                        data["processOutline"] = outline.clone();
                        data["processOutlineSources"] = retained_sources;
                    } else {
                        data["processOutline"] = process_outline_gap(
                            "PROCESS_RETAINED_OUTLINE_ARTIFACT_UNAVAILABLE",
                            root_metadata,
                        );
                    }
                } else {
                    let reason = root.gap.unwrap_or_else(|| {
                        if root.flow().is_none() {
                            "PROCESS_ROOT_FLOW_UNAVAILABLE"
                        } else {
                            "PROCESS_EVIDENCE_CHANGED_REVIEW_REQUIRED"
                        }
                    });
                    data["processOutline"] = process_outline_gap(reason, root_metadata);
                }
            } else {
                let (outline, sources, artifact) = process_outline_from_root(
                    &checked,
                    subject,
                    data["process"]["definition"]["title"]
                        .as_str()
                        .unwrap_or(title),
                    &root,
                )?;
                data["processOutline"] = outline;
                data["processOutlineSources"] = json!(sources);
                if let Some((stem, puml)) = artifact {
                    insert_diagram(&mut files, &mut diagrams, format!("diagrams/{stem}"), puml);
                }
            }
        }
        for process in data["savedProcesses"].as_array_mut().into_iter().flatten() {
            if let Some(id) = process["id"].as_str().map(str::to_owned) {
                let key = format!("scenario:{id}/{}", super::processes::OVERVIEW);
                if let Some(state) = binding.section_states.get(&key) {
                    process["state"] = json!(state);
                }
                if binding
                    .narratives
                    .get(&format!("scenario:{id}"))
                    .is_some_and(|narrative| {
                        narrative
                            .operations
                            .iter()
                            .any(|operation| operation.id == super::processes::OVERVIEW)
                    })
                {
                    process["status"] = json!("AUTHORED");
                }
            }
        }
        let mut operation_sources = BTreeMap::new();
        let mut operation_contracts = BTreeMap::new();
        for operation in &n.operations {
            let key = format!("{subject}/{}", operation.id);
            let from = if !accepted.contains(&key) {
                old_data.as_ref().unwrap_or(&data)
            } else {
                &data
            };
            operation_sources.insert(
                operation.id.clone(),
                from["operationSources"]
                    .get(&operation.id)
                    .unwrap_or(&from["sources"])
                    .clone(),
            );
            operation_contracts.insert(
                operation.id.clone(),
                from["operationContracts"]
                    .get(&operation.id)
                    .unwrap_or(&from["contracts"])
                    .clone(),
            );
            if let Some(old) = &old_data {
                for entry in old["catalogue"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|e| e["id"] == operation.id)
                {
                    if !data["catalogue"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|e| e["id"] == operation.id)
                    {
                        let mut retained = entry.clone();
                        retained["retainedOnly"] = json!(true);
                        data["catalogue"].as_array_mut().unwrap().push(retained);
                    }
                }
            }
        }
        super::language::section_labels(&mut data, ui_language);
        data["language"] = json!(ui_language);
        data["requestedDocumentationLanguage"] = json!(requested_language);
        data["translationGaps"] = super::language::gaps(
            n,
            requested_language.as_deref(),
            previous.as_ref(),
            old_data.as_ref(),
            folder,
            id,
        )?;
        data["translationComplete"] = json!(
            data["translationGaps"]
                .as_object()
                .is_some_and(|g| g.is_empty())
        );
        data["operationSources"] = json!(operation_sources);
        data["operationContracts"] = json!(operation_contracts);
        data["updateFailures"] = json!(
            failures
                .iter()
                .filter(|(key, _)| *key == subject || key.starts_with(&format!("{subject}/")))
                .collect::<BTreeMap<_, _>>()
        );
        super::status::attach(&mut data, subject, &binding);
        pending_pages.push((folder.to_owned(), id.to_owned(), data.clone()));
        let state = &binding.section_states[subject];
        let displayed = super::language::display_narrative(n, requested_language.as_deref());
        let mut prose =
            super::language::markdown(title, &displayed, &binding.section_states, ui_language);
        if ui_language == "en" {
            prose += &super::notes::markdown(&data["notes"]);
            prose += &super::processes::markdown(&data["process"]);
            prose += &super::dataflow::markdown(&data["view"], &displayed);
        }
        files.insert(format!("{folder}/{id}.md"), prose.into_bytes());
        for operation in &displayed.operations {
            let state = &binding.section_states[&format!("{subject}/{}", operation.id)];
            files.insert(
                format!(
                    "diagrams/{}-{}.mmd",
                    subject.replace(':', "-"),
                    operation.id
                ),
                format!(
                    "%% Source freshness: {}; meaning review: {}\n{}",
                    state.freshness.as_str(),
                    state.verification,
                    mermaid(operation)
                )
                .into_bytes(),
            );
            // Un-authored operations get an auto PlantUML activity document
            // from the root method's FLOW evidence; authored events take
            // priority and suppress it.
            if operation.events.is_empty() {
                let service = (kind == "service").then_some(id);
                if let Some(flow) = root_flow_events(&checked, operation, service)
                    && let Some(generated) = auto_flow_puml(&checked, flow, &operation.title)
                {
                    insert_diagram(
                        &mut files,
                        &mut diagrams,
                        format!("diagrams/{}-{}", subject.replace(':', "-"), operation.id),
                        generated.puml,
                    );
                }
            }
        }
        // Gap entrypoints are un-authored operations that are absent from
        // `operations[]`; they still get an auto PlantUML activity document
        // when their root method's flow is retained. Authored operations are
        // never here, so this cannot override manual content.
        if kind == "service" {
            for gap_id in n.gaps.keys() {
                if let Some(flow) = root_flow_events_by_id(&checked, gap_id, Some(id))
                    && let Some(generated) = auto_flow_puml(&checked, flow, gap_id)
                {
                    insert_diagram(
                        &mut files,
                        &mut diagrams,
                        format!("diagrams/{}-{}", subject.replace(':', "-"), gap_id),
                        generated.puml,
                    );
                }
            }
        }
        // Declarative state diagram: scenarios/<id>-states.yaml → a PlantUML
        // state diagram. Unresolved transitions are surfaced as limitations
        // rather than dropped.
        if kind == "scenario" {
            if let Some(schema) = process_state_schema {
                let (mut puml, unresolved) = super::process_states::render_puml(schema, &checked);
                if !unresolved.is_empty() {
                    for u in &unresolved {
                        puml = format!("' unresolved: {}\n{}", u, puml);
                    }
                    cards.push_str(&format!(
                        "<p class=\"state-limitations\">{}: {}</p>",
                        super::reader::text(
                            ui_language,
                            "State transitions without evidence",
                            "Переходы состояния без подтверждающих сведений"
                        ),
                        escape(&unresolved.join("; "))
                    ));
                }
                insert_diagram(
                    &mut files,
                    &mut diagrams,
                    format!("diagrams/{}-states", subject.replace(':', "-")),
                    puml,
                );
            }
            // Compact "activity on transitions" diagram: each lifecycle operation
            // becomes a branch leading to the states it transitions into.
            if let Some(activity) = activity_puml {
                insert_diagram(
                    &mut files,
                    &mut diagrams,
                    format!(
                        "diagrams/{}-activity-transitions",
                        subject.replace(':', "-")
                    ),
                    activity,
                );
            }
            // Compact activity diagram per lifecycle operation named by the
            // state schema's transitions, so the process page can list every
            // sub-process like the approved draft.
            for artifact in &lifecycle {
                insert_diagram(
                    &mut files,
                    &mut diagrams,
                    format!("diagrams/{}", artifact.diagram_stem),
                    artifact.puml.clone(),
                );
            }
        }
        let translation_count = data["translationGaps"].as_object().map_or(0, |g| g.len());
        let operation_count = displayed
            .operations
            .iter()
            .filter(|o| {
                !super::sections::contains(&o.id)
                    && !super::notes::is_root(&o.id)
                    && o.id != super::processes::OVERVIEW
                    && o.dataflow.is_none()
            })
            .count();
        cards.push_str(&format!("<article class=\"gap-card\"><div class=\"eyebrow\">{}</div><h2><a href=\"generated/__BUNDLE__/{folder}/{}.html\">{}</a></h2><p>{}: {operation_count} · {}: {}</p><p>{}: {translation_count}</p><details><summary>{}</summary><pre>{}</pre></details></article>",
            super::reader::text(ui_language,kind,if kind=="service"{"Сервис"}else{"Процесс"}),escape(id),escape(title),
            super::reader::text(ui_language,"Documented operations","Описанные операции"),super::reader::text(ui_language,"Gaps","Пробелы"),n.gaps.len(),
            super::reader::text(ui_language,"Sections requiring translation","Разделы, требующие перевода"),
            super::reader::text(ui_language,"Revisions, status and update gaps","Версии, состояние и пробелы обновления"),
            escape(&serde_json::to_string_pretty(&json!({"state":state,"failures":data["updateFailures"]})).map_err(io_error)?)));
    }
    // Pre-render every auto-generated diagram to SVG in one renderer process
    // (avoid spawning a JVM per diagram), then commit the bundle.
    let mut svg_availability =
        batch_render_diagrams(&mut files, &diagrams, plantuml_jar.as_deref());
    for (_, _, data) in &pending_pages {
        if let Some(stem) = data["processOutline"]["diagramStem"].as_str() {
            let base = format!("diagrams/{stem}");
            if files.contains_key(&format!("{base}.svg")) {
                svg_availability.insert(base, true);
            }
        }
    }
    for (folder, id, mut data) in pending_pages {
        data["stateDiagramSvg"] = json!(
            data["stateDiagram"]
                .as_str()
                .and_then(|stem| svg_availability.get(&format!("diagrams/{stem}")))
                .copied()
                .unwrap_or(false)
        );
        data["activityTransitionsSvg"] = json!(
            data["activityTransitions"]
                .as_str()
                .and_then(|stem| svg_availability.get(&format!("diagrams/{stem}")))
                .copied()
                .unwrap_or(false)
        );
        for lifecycle in data["lifecycleOperations"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            let stem = lifecycle["diagramStem"].as_str().unwrap_or("");
            lifecycle["svgAvailable"] = json!(
                svg_availability
                    .get(&format!("diagrams/{stem}"))
                    .copied()
                    .unwrap_or(false)
            );
        }
        if let Some(stem) = data["processOutline"]["diagramStem"].as_str() {
            data["processOutline"]["svgAvailable"] = json!(
                svg_availability
                    .get(&format!("diagrams/{stem}"))
                    .copied()
                    .unwrap_or(false)
            );
        }
        files.insert(format!("{folder}/{id}.json"), bytes(&data)?);
        files.insert(format!("{folder}/{id}.html"), html(&data)?.into_bytes());
    }
    files.insert("status.json".into(),bytes(&json!({"schema":"codeclew-documentation-status/1.0","documentationLanguage":requested_language,"translationGaps":translation_gap_count,"sections":binding.section_states,"targetRevisions":binding.target_revisions,"updateFailures":failures,"unresolved":checked.unresolved}))?);
    // The bundle identity covers every output file (including auto-generated
    // diagrams), so any change to the rendered output produces a fresh
    // immutable bundle instead of conflicting with an existing one.
    let output_digest = digest(
        &files
            .iter()
            .map(|(path, data)| (path.clone(), crate::canonical::hash_bytes(data)))
            .collect::<BTreeMap<_, _>>(),
    )?;
    let bundle = digest(&json!({
        "binding": binding,
        "rendererAssets": renderer_digest()?,
        "output": output_digest,
        "released": released
    }))?[7..]
        .to_owned();
    let cards = cards.replace("__BUNDLE__", &bundle);
    let relationships=repo.interactions()?.values().map(|i|format!("<article class=\"gap-card\"><h3>{}</h3><p>{} → {} · {}</p><details><summary>{}</summary><p>{}</p><pre>{}</pre></details></article>",escape(&i.title),escape(&i.from.service),escape(&i.to.service),escape(&i.transport.kind),super::reader::text(ui_language,"Original declaration and source checks","Исходная декларация и проверки по коду"),escape(&i.declaration.rationale),escape(&serde_json::to_string_pretty(&checked.interactions.get(&i.id)).unwrap_or_default()))).collect::<String>();
    let update_gaps = if failures.is_empty() {
        String::new()
    } else {
        format!(
            "<section><h2>{}</h2><pre>{}</pre></section>",
            super::reader::text(ui_language, "Update gaps", "Пробелы обновления"),
            escape(&serde_json::to_string_pretty(&failures).map_err(io_error)?)
        )
    };
    let overview = format!(
        "<!-- codeclew-bundle {bundle} -->\n<!doctype html><html lang=\"{ui_language}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{STYLE}</style></head><body><main style=\"margin:auto;max-width:1180px;padding:32px\"><div class=\"eyebrow\">{}</div><h1>{}</h1><p>{}</p><div class=\"coverage-grid\">{cards}</div><h2>{}</h2>{relationships}{update_gaps}</main></body></html>\n",
        escape(&repo.manifest.title),
        super::reader::text(
            ui_language,
            "ARCHITECTURE DOCUMENTATION",
            "ДОКУМЕНТАЦИЯ АРХИТЕКТУРЫ"
        ),
        escape(&repo.manifest.title),
        super::reader::text(
            ui_language,
            "Each explanation retains its source revisions. Missing translations are separate from analysis gaps.",
            "Каждое описание сохраняет версии исходников. Отсутствие перевода учитывается отдельно от пробелов анализа."
        ),
        super::reader::text(
            ui_language,
            "Declared service relationships",
            "Заявленные связи сервисов"
        ),
    );
    let gap_count: usize = narratives.values().map(|n| n.gaps.len()).sum();
    commit_bundle_with_mode(
        repo,
        &bundle,
        binding,
        files,
        &overview,
        &checked.input_digest,
        previous.as_ref(),
        previous_bytes.as_deref(),
        released,
        receipt_request.as_ref(),
        before_switch,
    )?;
    Ok(
        json!({"schema":"codeclew-docs-render/1.0","documentationLanguage":requested_language,"released":released,"translationGaps":translation_gap_count,"translationComplete":translation_gap_count==0,"status":if incomplete{"PARTIAL"}else{"RENDERED"},"bundle":bundle,"index":"docs/index.html","services":services.len(),"scenarios":scenarios.len(),"documentedOperations":narratives.values().flat_map(|n|n.operations.iter()).filter(|o|requested_language.as_deref().is_none_or(|language|o.documentation_language.as_deref()==Some(language))).filter(|o|!super::sections::contains(&o.id)&&!super::notes::is_root(&o.id)&&o.id!=super::processes::OVERVIEW&&o.dataflow.is_none()).count(),"documentedViews":narratives.values().flat_map(|n|n.operations.iter()).filter(|o|requested_language.as_deref().is_none_or(|language|o.documentation_language.as_deref()==Some(language))).filter(|o|o.dataflow.is_some()).count(),"documentedSections":narratives.values().flat_map(|n|n.operations.iter()).filter(|o|requested_language.as_deref().is_none_or(|language|o.documentation_language.as_deref()==Some(language))).filter(|o|super::sections::contains(&o.id)).count(),"explicitGaps":gap_count,"inputDigest":checked.input_digest,"contextDigest":checked.context_digest,"updateFailures":failures,"unresolved":checked.unresolved,"runtime":"UNKNOWN","evidenceAuthority":if refreshing{"CURRENT_SOURCE_CHECK"}else{"PINNED_SNAPSHOT_NOT_REVERIFIED"},"snapshot":snapshot}),
    )
}

/// Commit all immutable files before switching the sole reader pointer.
#[allow(clippy::too_many_arguments)]
pub(super) fn commit_bundle(
    repo: &Repository,
    bundle: &str,
    binding: Bindings,
    files: BTreeMap<String, Vec<u8>>,
    overview: &str,
    input_digest: &str,
    previous: Option<&(String, Bindings)>,
    previous_bytes: Option<&[u8]>,
) -> Result<(), ClewError> {
    commit_bundle_with_mode(
        repo,
        bundle,
        binding,
        files,
        overview,
        input_digest,
        previous,
        previous_bytes,
        false,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn commit_bundle_with_mode(
    repo: &Repository,
    bundle: &str,
    mut binding: Bindings,
    mut files: BTreeMap<String, Vec<u8>>,
    overview: &str,
    input_digest: &str,
    previous: Option<&(String, Bindings)>,
    previous_bytes: Option<&[u8]>,
    released: bool,
    receipt_request: Option<&PublicationReceiptRequest>,
    before_switch: Option<BeforePublicationSwitch<'_>>,
) -> Result<(), ClewError> {
    let previous_root = repo.path("docs/index.html")?;
    let bundle_overview = overview.replace(&format!("href=\"generated/{bundle}/"), "href=\"");
    files.insert("overview.html".into(), bundle_overview.into_bytes());
    let history_label = super::reader::text(
        binding.documentation_language.as_deref().unwrap_or("en"),
        "Snapshot history",
        "История публикаций",
    );
    let live_overview=overview.replacen("<body>",&format!("<body><nav aria-label=\"{history_label}\" style=\"padding:12px 20px\"><a href=\"history.html\">{history_label}</a></nav>"),1);
    files.insert(
        "root-overview.html".into(),
        live_overview.as_bytes().to_vec(),
    );
    let ui_language = binding
        .documentation_language
        .as_deref()
        .unwrap_or("en")
        .to_owned();
    super::reader::decorate_bundle_language(&mut files, bundle, &ui_language)?;
    let live_overview = String::from_utf8(files["root-overview.html"].clone()).map_err(io_error)?;
    let mut publication =
        super::history::prepare(repo, bundle, &binding, &mut files, input_digest, released)?;
    binding.output_hashes = files
        .iter()
        .map(|(path, bytes)| (path.clone(), canonical::hash_bytes(bytes)))
        .collect();
    bindings::compact(&mut binding);
    // Portable bindings already retain the complete inline payload. Do not
    // also write unused CAS copies: baseline readers consume the inline data.
    files.insert("bindings.json".into(), bytes(&binding)?);
    publication.files = files
        .iter()
        .map(|(name, data)| (name.clone(), canonical::hash_bytes(data)))
        .collect();
    files.insert("publication.json".into(), bytes(&publication)?);
    let receipt = receipt_request
        .map(|request| -> Result<PublicationReceipt, ClewError> {
            let effective_gaps = request
                .affected_subjects
                .iter()
                .map(|subject| {
                    binding
                        .narratives
                        .get(subject)
                        .map(|narrative| (subject.clone(), narrative.gaps.clone()))
                        .ok_or_else(|| {
                            invalid("reviewed publication subject is absent from final binding")
                        })
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            Ok(PublicationReceipt {
                schema: "codeclew-documentation-publication-receipt/1.0".into(),
                bundle_id: bundle.to_owned(),
                root_index_hash: canonical::hash_bytes(live_overview.as_bytes()),
                bindings_hash: canonical::hash_bytes(&files["bindings.json"]),
                publication_hash: canonical::hash_bytes(&files["publication.json"]),
                effective_gaps,
                requested_language: request.requested_language.clone(),
                effective_language: ui_language.clone(),
            })
        })
        .transpose()?;
    if files
        .values()
        .any(|data| data.len() > check::PORTABLE_CACHE_MAX_BYTES as usize)
    {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "documentation output exceeds its portable record budget; narrow source roots",
        ));
    }
    let lock = repo.lock()?;
    if repo.input_digest()? != input_digest {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "documentation input changed before output publication",
        ));
    }
    let current_bytes = if previous_root.exists() {
        Some(fs::read(&previous_root).map_err(io_error)?)
    } else {
        None
    };
    if current_bytes.as_deref() != previous_bytes {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "documentation output changed during rendering",
        ));
    }
    if let Some((id, binding)) = previous {
        bindings::verify_outputs(repo, id, binding)?;
    }
    let destination = repo.path(&format!("docs/generated/{bundle}"))?;
    if !destination.exists() {
        let parent = repo.path("docs/generated")?;
        fs::create_dir_all(&parent).map_err(io_error)?;
        let temporary = tempfile::Builder::new()
            .prefix(".pending-")
            .tempdir_in(&parent)
            .map_err(io_error)?;
        for (relative, data) in &files {
            store::relative(relative)?;
            let path = temporary.path().join(relative);
            fs::create_dir_all(
                path.parent()
                    .ok_or_else(|| invalid("missing output parent"))?,
            )
            .map_err(io_error)?;
            fs::write(&path, data).map_err(io_error)?;
            fs::File::open(path)
                .and_then(|f| f.sync_all())
                .map_err(io_error)?;
        }
        fs::rename(temporary.path(), &destination).map_err(io_error)?;
    } else {
        for (relative, data) in &files {
            if fs::read(repo.path(&format!("docs/generated/{bundle}/{relative}"))?)
                .map_err(io_error)?
                != *data
            {
                return Err(ClewError::new(
                    ErrorCode::WwConflict,
                    "existing immutable documentation bundle was modified",
                ));
            }
        }
    }
    // One pointer changes only after all matching documents and bindings exist.
    // Keep history navigation available after a working render while the
    // index itself contains only explicit releases.
    super::history::index(repo, bundle)?;
    if let Some(callback) = before_switch {
        let receipt = receipt
            .as_ref()
            .ok_or_else(|| invalid("pre-switch callback requires a publication receipt"))?;
        // The supplied guard belongs to `repo`; callback writers must stay on
        // this same repository and use the held guard without relocking.
        callback(&lock, receipt)?;
    }
    repo.atomic("docs/index.html", live_overview.as_bytes())?;
    super::reader::connect_starters_language(
        repo,
        bundle,
        &files.keys().cloned().collect::<Vec<_>>(),
        &ui_language,
    )?;
    Ok(())
}

pub(super) fn renderer_digest() -> Result<String, ClewError> {
    digest(&[
        TEMPLATE,
        include_str!("language.rs"),
        STYLE,
        SCRIPT,
        include_str!("history.rs"),
        include_str!("process_candidates.rs"),
        include_str!("reader.rs"),
        super::reader::HELP,
        super::reader::RUNBOOKS,
        super::reader::ICON,
        super::reader::STYLE,
        super::reader::SCRIPT,
        super::reader::LIMITS,
        include_str!("../../assets/documentation/analysis.js"),
    ])
}

#[cfg(test)]
mod process_catalog_tests {
    use super::*;

    #[test]
    fn summary_text_enforces_utf8_byte_boundary_and_plain_prose() {
        assert!(validate_summary_text(&"a".repeat(SUMMARY_TEXT_MAX_BYTES)).is_ok());
        assert!(validate_summary_text(&"\u{044f}".repeat(SUMMARY_TEXT_MAX_BYTES / 2)).is_ok());
        for invalid_text in [
            "a".repeat(SUMMARY_TEXT_MAX_BYTES + 1),
            format!("{}a", "\u{044f}".repeat(SUMMARY_TEXT_MAX_BYTES / 2)),
            "\n ".into(),
            "code `example`".into(),
            "value < limit".into(),
        ] {
            let error = validate_summary_text(&invalid_text)
                .unwrap_err()
                .to_string();
            assert!(error.contains("2048 UTF-8 bytes"), "{error}");
        }
    }

    #[test]
    fn process_preview_keeps_only_displayed_source_closure_and_saved_links() {
        let mut evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"digest","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}
        }))
        .unwrap();
        for index in 0..10 {
            let id = format!("method-{index:02}");
            let events = json!([{"kind":"IF"},{"kind":"CALL","target":"method-00"},{"kind":"CALL","target":"method-01"}]);
            evidence.observations.insert(id.clone(), Observation {
                id:id.clone(),kind:"SYMBOL".into(),service:"svc".into(),symbol:id.clone(),
                normalized:json!({"declarationKind":"METHOD","scope":":main","name":id,"ownerIdentity":"class:Worker","documentation":{"events":events}}),
                digest:"digest".into(),source_ids:vec![id.clone()],
            });
            for (ordinal, event) in events.as_array().unwrap().iter().enumerate() {
                let mut normalized = event.clone();
                normalized["scope"] = json!(":main");
                normalized["ordinal"] = json!(ordinal);
                let flow_id = format!("{id}/event/{ordinal}");
                evidence.observations.insert(
                    flow_id.clone(),
                    Observation {
                        id: flow_id,
                        kind: "FLOW".into(),
                        service: "svc".into(),
                        symbol: id.clone(),
                        normalized,
                        digest: "digest".into(),
                        source_ids: vec![id.clone()],
                    },
                );
            }
            evidence.sources.insert(id.clone(),serde_json::from_value(json!({
                "id":id,"service":"svc","revision":"rev","file":format!("{id}.java"),"startLine":1,"endLine":1,
                "text":format!("void {id}() {{}}"),"textDigest":"digest","evidenceDigest":"digest","authority":"TEST"
            })).unwrap());
        }
        let saved = Observation {
            id: "process:worker".into(),
            kind: "PROCESS_DEFINITION".into(),
            service: "svc".into(),
            symbol: "worker".into(),
            normalized: json!({"definition":{"id":"worker","title":"Worker process","root":{"service":"svc"},"process":{"participants":["svc"],"trigger":"Unknown caller"}}}),
            digest: "digest".into(),
            source_ids: vec![],
        };
        let checked = Check {
            schema: "test".into(),
            input_digest: "digest".into(),
            context_digest: "digest".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::from([(saved.id.clone(), saved)]),
            source_inputs: None,
            composition: None,
        };
        let narrative = Narrative {
            schema: "test".into(),
            subject: "service:svc".into(),
            context_digest: "digest".into(),
            operations: vec![],
            gaps: BTreeMap::new(),
        };
        let data = page_data(
            "service:svc",
            "Service",
            "",
            &narrative,
            &checked,
            &BTreeSet::new(),
            None,
            &[],
            None,
        );
        assert_eq!(
            data["processCandidates"]["internal"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
        assert_eq!(data["processCandidates"]["omittedInternal"], 2);
        let suppress = BTreeSet::from([":main@method-00".to_string()]);
        let filtered = page_data(
            "service:svc",
            "Service",
            "",
            &narrative,
            &checked,
            &suppress,
            None,
            &[],
            None,
        );
        assert_eq!(
            filtered["processCandidates"]["suppressed"],
            json!([":main@method-00"])
        );
        assert!(
            filtered["processCandidates"]["internal"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["symbol"].as_str() != Some("method-00"))
        );
        assert_eq!(data["sources"].as_object().unwrap().len(), 8);
        for candidate in data["processCandidates"]["internal"].as_array().unwrap() {
            for source in candidate["sourceIds"].as_array().unwrap() {
                assert!(data["sources"].get(source.as_str().unwrap()).is_some());
            }
        }
        assert!(data["sources"].get("method-09").is_none());
        assert_eq!(
            data["savedProcesses"][0]["href"],
            "../scenarios/worker.html#process-overview"
        );
        assert_eq!(data["savedProcesses"][0]["status"], "AWAITING_AUTHORING");
    }
}

#[cfg(test)]
mod sequence_contract_tests {
    use super::sequence_event_kinds;

    #[test]
    fn renderer_flow_to_sequence_mapping_covers_all_nine_structural_kinds() {
        for (flow, events) in [
            ("IF", &["alt"][..]),
            ("TRY", &["alt"][..]),
            ("DEFERRED", &["opt"][..]),
            ("LOOP", &["loop"][..]),
            ("FINALLY", &["note"][..]),
            ("BREAK", &["note"][..]),
            ("CONTINUE", &["note"][..]),
            ("RETURN", &["return", "note"][..]),
            ("THROW", &["return", "note"][..]),
        ] {
            assert_eq!(sequence_event_kinds(flow), events, "{flow}");
        }
        assert!(sequence_event_kinds("CALL").is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow_observation(symbol: &str, service: &str, events: Value) -> Observation {
        Observation {
            id: format!("{service}:symbol:{symbol}"),
            kind: "SYMBOL".into(),
            service: service.into(),
            symbol: symbol.into(),
            normalized: json!({
                "scope":":main",
                "documentation":{
                    "authority":"JAVAC_SOURCE_STRUCTURE",
                    "boundaries":[],
                    "events":events
                }
            }),
            digest: "digest".into(),
            source_ids: vec![],
        }
    }

    fn checked_with_source_flow(
        symbol: &str,
        source: &str,
        events: Value,
        boundaries: Value,
    ) -> Check {
        let mut flow = flow_observation(symbol, "svc", events);
        flow.normalized["documentation"]["boundaries"] = boundaries;
        let source_observation = Observation {
            id: "svc:source:m1".into(),
            kind: "TRANSFORMED_SOURCE".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({"scope":":main","documentation":{"source":source}}),
            digest: "source-digest".into(),
            source_ids: vec![],
        };
        let mut evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0",
            "service":"svc","revision":"rev","serviceDigest":"d",
            "extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},"entrypoints":[],"observations":{},"sources":{}
        }))
        .unwrap();
        evidence.observations.insert(flow.id.clone(), flow);
        evidence
            .observations
            .insert(source_observation.id.clone(), source_observation);
        Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        }
    }

    #[test]
    fn auto_flow_without_exact_source_is_gap_not_a_causal_view() {
        let flow = json!([
            {"kind":"CALL","resolution":"COMPILER_EXACT","target":"method:class:ru.tins.CheckoutService#charge()V"},
            {"kind":"RETURN"}
        ]);
        let symbol = "method:class:ru.tins.CheckoutController#checkout";
        let title = "Checkout flow";
        let observation = flow_observation(symbol, "", flow.clone());
        let resolved = ResolvedFlow {
            observation: &observation,
            events: &flow,
        };
        let checked = Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services: BTreeMap::new(),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let doc = auto_flow_puml(&checked, resolved, title).unwrap();
        assert!(doc.puml.contains("title Checkout flow"), "{}", doc.puml);
        assert_eq!(doc.origin, "flow");
        assert!(
            doc.puml.contains("FLOW_CONTROL_CAPABILITY_UNVERIFIED"),
            "{}",
            doc.puml
        );
        assert!(
            !doc.puml.contains(":CheckoutService#charge"),
            "{}",
            doc.puml
        );
        assert!(!doc.puml.contains("start\n"), "{}", doc.puml);
    }

    #[test]
    fn source_text_cannot_override_unsupported_flow_boundary() {
        let source = "\
public void handle(Long taskId) {
    TaskInstance ti = null;
    if (ti == null) {
        ti = svc.create(taskId);
    }
    return ti;
}";
        let symbol = "method:class:svc.TaskService#handle";
        let flow = json!([{"kind":"BOUNDARY"}]);
        let src_obs = Observation {
            id: "svc:source:handle".into(),
            kind: "TRANSFORMED_SOURCE".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({"scope":":main","documentation":{"source":source}}),
            digest: "d".into(),
            source_ids: vec![],
        };
        let flow_obs = flow_observation(symbol, "svc", flow.clone());
        let evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},
            "entrypoints":[],"observations":{},"sources":{}
        }))
        .unwrap();
        let mut evidence = evidence;
        evidence.observations.insert(src_obs.id.clone(), src_obs);
        evidence.observations.insert(flow_obs.id.clone(), flow_obs);
        let checked = Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let resolved = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        let doc = auto_flow_puml(&checked, resolved, "t").unwrap();
        assert_eq!(doc.origin, "flow");
        assert!(
            doc.puml.contains("FLOW_BOUNDARY_UNSPECIFIED"),
            "{}",
            doc.puml
        );
        assert!(!doc.puml.contains("svc.create"), "{}", doc.puml);
        assert!(!doc.puml.contains("ti = null"), "{}", doc.puml);
        assert!(!doc.tree.contains("svc.create"), "{}", doc.tree);
    }

    #[test]
    fn checkout_source_keeps_guard_exit_save_and_reserve_in_one_projection() {
        let symbol = "method:class:example.CheckoutController#checkout()Ljava/lang/String;";
        let source = r#"
public String checkout(ReservationRequest request) {
    if (!hasPositiveQuantity(request)) return invalid();
    reservations.save(request);
    return inventory.reserve(request);
}
"#;
        let events = json!([
            {"kind":"CALL","target":"method:class:example.CheckoutController#hasPositiveQuantity(Lexample/ReservationRequest;)Z"},
            {"kind":"IF","condition":"!hasPositiveQuantity(request)"},
            {"kind":"CALL","target":"method:class:example.CheckoutController#invalid()Ljava/lang/String;"},
            {"kind":"RETURN"},
            {"kind":"END"},
            {"kind":"CALL","target":"method:class:org.springframework.data.repository.CrudRepository#save(Ljava/lang/Object;)Ljava/lang/Object;"},
            {"kind":"CALL","target":"method:class:example.InventoryClient#reserve(Lexample/ReservationRequest;)Ljava/lang/String;"},
            {"kind":"RETURN"}
        ]);
        let checked = checked_with_source_flow(symbol, source, events, json!([]));
        let resolved = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        let rendered = auto_flow_puml(&checked, resolved, "Checkout").unwrap();
        assert_eq!(rendered.origin, "source", "{}", rendered.tree);
        let guard = rendered
            .tree
            .find("[D] if (!hasPositiveQuantity(request))")
            .unwrap();
        let early_return = rendered.tree.find("  return invalid()").unwrap();
        let save = rendered
            .tree
            .find("[W] reservations.save(request)")
            .unwrap();
        let reserve = rendered
            .tree
            .find("return inventory.reserve(request)")
            .unwrap();
        assert!(
            guard < early_return && early_return < save && save < reserve,
            "{}",
            rendered.tree
        );
        assert!(
            rendered.puml.contains("reservations.save"),
            "{}",
            rendered.puml
        );
        assert!(
            rendered.puml.contains("inventory.reserve"),
            "{}",
            rendered.puml
        );
        assert_eq!(
            rendered.puml.matches("stop\n").count(),
            2,
            "{}",
            rendered.puml
        );
    }

    #[test]
    fn source_may_expose_one_opaque_short_circuit_return_with_gap_evidence() {
        let symbol = "method:class:example.CheckoutController#hasPositiveQuantity()Z";
        let predicate = "request != null && request.quantity() > 0";
        let source =
            format!("boolean hasPositiveQuantity(Request request) {{ return {predicate}; }}");
        let events = json!([
            {"kind":"BOUNDARY","code":"SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"},
            {"kind":"RETURN"}
        ]);
        let checked = checked_with_source_flow(
            symbol,
            &source,
            events,
            json!(["SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"]),
        );
        let resolved = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        let rendered = auto_flow_puml(&checked, resolved, "Predicate").unwrap();
        assert_eq!(rendered.origin, "source");
        assert!(
            rendered.tree.contains(&format!("return {predicate}")),
            "{}",
            rendered.tree
        );
        assert!(
            rendered
                .tree
                .contains("SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"),
            "{}",
            rendered.tree
        );
        assert!(
            rendered.puml.contains("request.quantity"),
            "{}",
            rendered.puml
        );
        assert!(rendered.puml.contains("&gt; 0"), "{}", rendered.puml);
        assert!(
            !rendered.tree.contains("request.quantity(...)"),
            "{}",
            rendered.tree
        );
    }

    #[test]
    fn lambda_boundary_blocks_source_actions_inside_callback_body() {
        let symbol = "method:class:example.CheckoutController#prepareSaveCallback()V";
        let source = r#"
void prepareSaveCallback() {
    callbacks.register(() -> repository.save(callbackSentinel));
}
"#;
        let events = json!([{"kind":"BOUNDARY","code":"LAMBDA_EXECUTION_NOT_EXPANDED"}]);
        let checked = checked_with_source_flow(
            symbol,
            source,
            events,
            json!(["LAMBDA_EXECUTION_NOT_EXPANDED"]),
        );
        let resolved = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        let rendered = auto_flow_puml(&checked, resolved, "Callback boundary").unwrap();
        assert_eq!(rendered.origin, "flow");
        assert!(
            rendered.tree.contains("LAMBDA_EXECUTION_NOT_EXPANDED"),
            "{}",
            rendered.tree
        );
        assert!(
            !rendered.tree.contains("callbackSentinel"),
            "{}",
            rendered.tree
        );
        assert!(
            !rendered.tree.contains("repository.save"),
            "{}",
            rendered.tree
        );
        assert!(
            !rendered.puml.contains("callbacks.register"),
            "{}",
            rendered.puml
        );
    }

    #[test]
    fn unsupported_retained_source_vetoes_a_balanced_flow_outline() {
        let symbol = "method:class:svc.Checkout#checkout()V";
        let source = r#"
public void checkout() {
    outer: {
        if (x) break outer;
        reserve();
    }
    done();
}
"#;
        let flow = json!([
            {"kind":"IF","condition":"x"},
            {"kind":"END"},
            {"kind":"CALL","target":"method:class:svc.Checkout#reserve()V"},
            {"kind":"CALL","target":"method:class:svc.Checkout#done()V"},
            {"kind":"RETURN"}
        ]);
        let checked = checked_with_source_flow(symbol, source, flow.clone(), json!([]));
        let resolved = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        let with_source = auto_flow_puml(&checked, resolved, "Labeled block").unwrap();
        assert_eq!(with_source.origin, "source");
        assert!(
            with_source
                .tree
                .contains("SOURCE_STATEMENT_OR_CONTROL_UNSUPPORTED"),
            "{}",
            with_source.tree
        );
        assert!(
            !with_source.tree.contains("reserve()"),
            "{}",
            with_source.tree
        );
        assert!(
            !with_source.puml.contains("start\n"),
            "{}",
            with_source.puml
        );

        let mut without_source = checked_with_source_flow(symbol, source, flow.clone(), json!([]));
        without_source
            .services
            .get_mut("svc")
            .unwrap()
            .observations
            .remove("svc:source:m1");
        let resolved = resolve_flow(&without_source, Some("svc"), &[symbol]).unwrap();
        let no_source = auto_flow_puml(&without_source, resolved, "Labeled block").unwrap();
        assert_eq!(no_source.origin, "flow");
        assert!(
            no_source
                .tree
                .contains("FLOW_CONTROL_CAPABILITY_UNVERIFIED"),
            "{}",
            no_source.tree
        );
        assert!(!no_source.tree.contains("reserve()"), "{}", no_source.tree);
        assert!(!no_source.puml.contains("start\n"), "{}", no_source.puml);
    }

    #[test]
    fn root_flow_events_resolves_entrypoint_method_symbol() {
        let flow = json!([{"kind":"STATEMENT","text":"load()"}]);
        let symbol = "method:class:CheckoutService#checkout";
        let symbol_obs = Observation {
            id: "svc:symbol:x".into(),
            kind: "SYMBOL".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({"documentation":{"events":flow}}),
            digest: "digest".into(),
            source_ids: vec![],
        };
        let evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"digest","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},
            "entrypoints":[{"id":"ep1","service":"svc","symbol":symbol,"kind":"ENTRYPOINT","trigger":{},"sourceIds":[],"dependencyIds":[],"boundaries":[]}],
            "observations":{},"sources":{}
        }))
        .unwrap();
        // realistic: SYMBOL observations (with documentation.events) live in the
        // service evidence, not checked.dependencies — mirror Walker::walk.
        let mut evidence = evidence;
        evidence
            .observations
            .insert(symbol_obs.id.clone(), symbol_obs);
        let checked = Check {
            schema: "test".into(),
            input_digest: "digest".into(),
            context_digest: "digest".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let operation = Operation {
            documentation_language: None,
            visuals: vec![],
            dataflow: None,
            id: "ep1".into(),
            title: "Checkout flow".into(),
            summary: Fragment {
                id: "s".into(),
                text: String::new(),
                dependency_ids: vec![],
                source_ids: vec![],
            },
            assessment: None,
            explanation: vec![],
            interface_contracts: vec![],
            overview_diagram: None,
            participants: vec![],
            events: vec![],
            findings: vec![],
            boundaries: vec![],
        };
        let resolved = root_flow_events(&checked, &operation, Some("svc")).unwrap();
        assert_eq!(resolved.observation.symbol, symbol);
        assert_eq!(resolved.events[0]["text"], "load()");
        // Operation id == method symbol resolves even without an entrypoint.
        let op2 = Operation {
            id: symbol.into(),
            ..operation.clone()
        };
        let resolved2 = root_flow_events(&checked, &op2, None).unwrap();
        assert_eq!(resolved2.observation.symbol, symbol);
        // An operation with no matching root yields None.
        let op3 = Operation {
            id: "unrelated".into(),
            ..operation
        };
        assert!(root_flow_events(&checked, &op3, None).is_none());
    }

    #[test]
    fn root_flow_events_by_id_resolves_gap_entrypoint_symbol() {
        let flow = json!([{"kind":"STATEMENT","text":"start()"}]);
        let symbol = "method:class:CheckoutService#start";
        let symbol_obs = Observation {
            id: "svc:symbol:start".into(),
            kind: "SYMBOL".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({"documentation":{"events":flow}}),
            digest: "digest".into(),
            source_ids: vec![],
        };
        let evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"digest","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},
            "entrypoints":[{"id":"svc-abc123","service":"svc","symbol":symbol,"kind":"ENTRYPOINT","trigger":{},"sourceIds":[],"dependencyIds":[],"boundaries":[]}],
            "observations":{},"sources":{}
        }))
        .unwrap();
        let mut evidence = evidence;
        evidence
            .observations
            .insert(symbol_obs.id.clone(), symbol_obs);
        let checked = Check {
            schema: "test".into(),
            input_digest: "digest".into(),
            context_digest: "digest".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        // A gap operation is absent from operations[]; resolve purely by id.
        let resolved = root_flow_events_by_id(&checked, "svc-abc123", Some("svc")).unwrap();
        assert_eq!(resolved.observation.symbol, symbol);
        assert_eq!(resolved.events[0]["text"], "start()");
        // Unknown gap id yields None.
        assert!(root_flow_events_by_id(&checked, "svc-missing", Some("svc")).is_none());
    }

    #[test]
    fn resolve_flow_prefers_deepest_observation_for_symbol() {
        let symbol = "method:class:svc.TaskService#changeStatus";
        let shallow = Observation {
            id: "svc:symbol:aaa-shallow".into(),
            kind: "SYMBOL".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({"documentation":{"events":[{"kind":"BOUNDARY"}]}}),
            digest: "digest".into(),
            source_ids: vec![],
        };
        let deep = Observation {
            id: "svc:symbol:zzz-deep".into(),
            kind: "SYMBOL".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({"documentation":{"events":[
                {"kind":"CALL","target":"method:class:svc.TaskService#changeStatus()V"},
                {"kind":"IF","condition":"ready"},
                {"kind":"CALL","target":"method:class:svc.Repo#save()V"},
                {"kind":"END"},
                {"kind":"RETURN"}
            ]}}),
            digest: "digest".into(),
            source_ids: vec![],
        };
        let evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"digest","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},
            "entrypoints":[],"observations":{},"sources":{}
        }))
        .unwrap();
        let mut evidence = evidence;
        // Id order (BTreeMap) puts the shallow observation first, so a naive
        // `.find()` would return the 1-event stub; the resolver must prefer
        // the deepest retained flow (5 events).
        evidence.observations.insert(shallow.id.clone(), shallow);
        evidence.observations.insert(deep.id.clone(), deep);
        let checked = Check {
            schema: "test".into(),
            input_digest: "digest".into(),
            context_digest: "digest".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let resolved = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        assert_eq!(resolved.observation.symbol, symbol);
        assert_eq!(resolved.events.as_array().map(Vec::len), Some(5));
    }

    #[test]
    fn lifecycle_resolution_requires_exact_unambiguous_symbol_and_scope() {
        let shared = "method:class:orders.TaskService#changeTaskStatus(Ljava/lang/Long;)V";
        let mut first: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"orders","revision":"r1",
            "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}
        })).unwrap();
        let mut second: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"billing","revision":"r2",
            "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}
        })).unwrap();
        let first_obs =
            flow_observation(shared, "orders", json!([{"kind":"RETURN","text":"orders"}]));
        let second_obs = flow_observation(
            shared,
            "billing",
            json!([{"kind":"RETURN","text":"billing"}]),
        );
        first.observations.insert(first_obs.id.clone(), first_obs);
        second
            .observations
            .insert(second_obs.id.clone(), second_obs);
        let checked = Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services: BTreeMap::from([("orders".into(), first), ("billing".into(), second)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };

        // A human transition label or method substring must not select an unrelated method.
        assert!(lifecycle_flow(&checked, "Status").is_none());
        // Even an exact JVM identity is ambiguous when retained by two services.
        assert!(lifecycle_flow(&checked, shared).is_none());
        // A service-bound entrypoint flow still resolves only within its owner.
        let operation = Operation {
            documentation_language: None,
            visuals: vec![],
            dataflow: None,
            id: shared.into(),
            title: "Update status".into(),
            summary: Fragment {
                id: "s".into(),
                text: String::new(),
                dependency_ids: vec![],
                source_ids: vec![],
            },
            assessment: None,
            explanation: vec![],
            interface_contracts: vec![],
            overview_diagram: None,
            participants: vec![],
            events: vec![],
            findings: vec![],
            boundaries: vec![],
        };
        let owned = root_flow_events(&checked, &operation, Some("orders")).unwrap();
        assert_eq!(owned.events[0]["text"], "orders");
    }

    #[test]
    fn lifecycle_simple_method_name_must_be_unique_across_services() {
        let mut services = BTreeMap::new();
        for (service, owner) in [
            ("orders", "orders.TaskService"),
            ("billing", "billing.TaskService"),
        ] {
            let symbol = format!("method:class:{owner}#changeTaskStatus(Ljava/lang/Long;)V");
            let mut evidence: ServiceEvidence = serde_json::from_value(json!({
                "schema":"codeclew-documentation-service-evidence/1.0","service":service,"revision":"r",
                "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
                "boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}
            })).unwrap();
            let observation = flow_observation(&symbol, service, json!([{"kind":"RETURN"}]));
            evidence
                .observations
                .insert(observation.id.clone(), observation);
            services.insert(service.to_owned(), evidence);
        }
        let checked = Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services,
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        assert!(lifecycle_flow(&checked, "changeTaskStatus").is_none());
    }

    #[test]
    fn method_source_resolves_via_symbol_source_ids() {
        let symbol = "method:class:svc.TaskService#handle(Ljava/lang/Long;)V";
        let source_text = "public void handle(Long id) {\n  svc.doSomething(id);\n}";
        let sym_obs = Observation {
            id: "svc:symbol:handle".into(),
            kind: "SYMBOL".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({"scope":":main","documentation":{"events":[]}}),
            digest: "d".into(),
            source_ids: vec!["svc:source:handle".into()],
        };
        let source = Source {
            id: "svc:source:handle".into(),
            service: "svc".into(),
            revision: "rev".into(),
            file: "TaskService.java".into(),
            start_line: 1,
            end_line: 3,
            text: source_text.into(),
            text_digest: "d".into(),
            evidence_digest: "d".into(),
            authority: "TRANSFORMED_SOURCE".into(),
            occurrence: None,
            url: None,
        };
        let mut evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},
            "entrypoints":[],"observations":{},"sources":{}
        }))
        .unwrap();
        evidence.observations.insert(sym_obs.id.clone(), sym_obs);
        evidence.sources.insert(source.id.clone(), source);
        let mut foreign: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"other","revision":"other-rev",
            "serviceDigest":"other-d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},"entrypoints":[],"observations":{},"sources":{}
        })).unwrap();
        let foreign_obs = Observation {
            id: "other:symbol:handle".into(),
            kind: "SYMBOL".into(),
            service: "other".into(),
            symbol: symbol.into(),
            normalized: json!({"scope":":main","documentation":{"events":[]}}),
            digest: "other-d".into(),
            source_ids: vec!["other:source:handle".into()],
        };
        let foreign_source = Source {
            id: "other:source:handle".into(),
            service: "other".into(),
            revision: "other-rev".into(),
            file: "Other.java".into(),
            start_line: 1,
            end_line: 1,
            text: "FOREIGN SOURCE".into(),
            text_digest: "other-d".into(),
            evidence_digest: "other-d".into(),
            authority: "TEST".into(),
            occurrence: None,
            url: None,
        };
        foreign
            .observations
            .insert(foreign_obs.id.clone(), foreign_obs);
        foreign
            .sources
            .insert(foreign_source.id.clone(), foreign_source);
        let checked = Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services: BTreeMap::from([("svc".into(), evidence), ("other".into(), foreign)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let resolved = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        assert_eq!(
            method_source(&checked, resolved).as_deref(),
            Some(source_text)
        );
    }

    fn checked_with_process_root(selector_scope: Option<&str>, ambiguous: bool) -> Check {
        let symbol = "method:class:example.CheckoutController#checkout()Ljava/lang/String;";
        let source_text = "public String checkout(Request request) {\n    if (!hasPositiveQuantity(request)) return invalid();\n    reservations.save(request);\n    return inventory.reserve(request);\n}";
        let mut observation = flow_observation(
            symbol,
            "svc",
            json!([
                {"kind":"CALL","target":"method:class:example.CheckoutController#hasPositiveQuantity(Lexample/Request;)Z"},
                {"kind":"IF","condition":"!hasPositiveQuantity(request)"},
                {"kind":"CALL","target":"method:class:example.CheckoutController#invalid()Ljava/lang/String;"},
                {"kind":"RETURN"},
                {"kind":"END"},
                {"kind":"CALL","target":"method:class:org.springframework.data.repository.CrudRepository#save(Ljava/lang/Object;)Ljava/lang/Object;"},
                {"kind":"CALL","target":"method:class:example.Inventory#reserve(Lexample/Request;)Ljava/lang/String;"},
                {"kind":"RETURN"}
            ]),
        );
        observation.normalized["name"] = json!("checkout");
        observation.normalized["ownerIdentity"] = json!("class:example.CheckoutController");
        observation.normalized["jvmDescriptor"] = json!("(Lexample/Request;)Ljava/lang/String;");
        observation.normalized["documentation"]["parameterTypes"] = json!(["example.Request"]);
        observation.source_ids = vec!["checkout-source".into()];
        let source = Source {
            id: "checkout-source".into(),
            service: "svc".into(),
            revision: "rev".into(),
            file: "CheckoutController.java".into(),
            start_line: 1,
            end_line: 5,
            text: source_text.into(),
            text_digest: "checkout-text-digest".into(),
            evidence_digest: "checkout-evidence-digest".into(),
            authority: "RETAINED_SOURCE_NOT_REVERIFIED".into(),
            occurrence: None,
            url: None,
        };
        let selector = json!({
            "language":"java",
            "scope":selector_scope,
            "owner":"example.CheckoutController",
            "name":"checkout",
            "parameterTypes":["example.Request"]
        });
        let definition = Observation {
            id: "process:checkout".into(),
            kind: "PROCESS_DEFINITION".into(),
            service: "svc".into(),
            symbol: "checkout".into(),
            normalized: json!({"definition":{"id":"checkout","title":"Checkout","root":{"service":"svc","selector":selector}}}),
            digest: "process-digest".into(),
            source_ids: vec![],
        };
        let mut evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0",
            "service":"svc","revision":"rev","serviceDigest":"d",
            "extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},"entrypoints":[],"observations":{},"sources":{}
        }))
        .unwrap();
        evidence
            .observations
            .insert(observation.id.clone(), observation);
        evidence.sources.insert(source.id.clone(), source);
        if ambiguous {
            let mut second = evidence.observations.values().next().unwrap().clone();
            second.id = "svc:symbol:checkout-test-scope".into();
            second.normalized["scope"] = json!(":test");
            second.digest = "checkout-test-scope-digest".into();
            evidence.observations.insert(second.id.clone(), second);
        }
        Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::from([("process:checkout".into(), definition)]),
            source_inputs: None,
            composition: None,
        }
    }

    #[test]
    fn process_outline_uses_exact_saved_selector_and_binds_source_records() {
        let checked = checked_with_process_root(None, false);
        let root = process_root_resolution(&checked, "checkout");
        assert_eq!(root.gap, None);
        assert_eq!(root.flow().unwrap().observation.service, "svc");
        let (outline, sources, artifact) =
            process_outline_from_root(&checked, "scenario:checkout", "Checkout", &root).unwrap();
        assert_eq!(outline["status"], "STATIC_SOURCE_OUTLINE");
        assert_eq!(outline["root"]["service"], "svc");
        assert_eq!(outline["root"]["scope"], ":main");
        assert_eq!(
            outline["root"]["observation"],
            "svc:symbol:method:class:example.CheckoutController#checkout()Ljava/lang/String;"
        );
        assert_eq!(outline["sourceIds"][0], "checkout-source");
        assert!(outline["root"]["sourceRecordDigests"]["checkout-source"].is_string());
        assert!(
            outline["tree"]
                .as_str()
                .unwrap()
                .contains("return inventory.reserve(request)")
        );
        assert!(
            outline["tree"]
                .as_str()
                .unwrap()
                .contains("return invalid()")
        );
        assert_eq!(sources.len(), 1);
        assert!(
            artifact
                .as_ref()
                .is_some_and(|(_, puml)| puml.contains("inventory.reserve"))
        );
        let (_, source_digests) = process_root_source_records(&checked, &root).unwrap();
        assert!(retained_process_outline_matches(
            &outline,
            &json!(sources),
            &root,
            &sources,
            &source_digests,
        ));

        let mut changed = checked.clone();
        changed
            .services
            .get_mut("svc")
            .unwrap()
            .sources
            .get_mut("checkout-source")
            .unwrap()
            .text
            .push_str("\n// changed");
        let changed_root = process_root_resolution(&changed, "checkout");
        let (changed_sources, changed_digests) =
            process_root_source_records(&changed, &changed_root).unwrap();
        assert!(!retained_process_outline_matches(
            &outline,
            &json!(sources),
            &changed_root,
            &changed_sources,
            &changed_digests,
        ));
    }

    #[test]
    fn ambiguous_process_root_is_a_gap_and_explicit_scope_resolves_one_observation() {
        let ambiguous = checked_with_process_root(None, true);
        let root = process_root_resolution(&ambiguous, "checkout");
        assert_eq!(root.gap, Some("PROCESS_ROOT_SELECTOR_AMBIGUOUS"));
        let (gap, sources, artifact) =
            process_outline_from_root(&ambiguous, "scenario:checkout", "Checkout", &root).unwrap();
        assert_eq!(gap["status"], "GAP");
        assert_eq!(gap["gap"], "PROCESS_ROOT_SELECTOR_AMBIGUOUS");
        assert_eq!(gap["root"]["candidates"].as_array().unwrap().len(), 2);
        assert!(sources.is_empty());
        assert!(artifact.is_none());

        let explicitly_scoped = checked_with_process_root(Some(":main"), true);
        let selected = process_root_resolution(&explicitly_scoped, "checkout");
        assert_eq!(selected.gap, None);
        assert_eq!(selected.observations.len(), 1);
        assert_eq!(
            selected.flow().unwrap().observation.normalized["scope"],
            ":main"
        );
    }
}

#[cfg(test)]
mod publication_receipt_tests {
    use super::*;

    const SUBJECT: &str = "scenario:receipt-test";
    type TestHook<'a> = BeforePublicationSwitch<'a>;

    fn repository() -> (tempfile::TempDir, Repository) {
        let directory = tempfile::tempdir().unwrap();
        Repository::init(directory.path(), "Publication receipt test").unwrap();
        let repo = Repository::open(directory.path()).unwrap();
        (directory, repo)
    }

    fn binding(repo: &Repository, language: Option<&str>, gap: &str) -> Bindings {
        Bindings {
            documentation_language: language.map(str::to_owned),
            influence_scopes: BTreeMap::new(),
            schema: "codeclew-documentation-bindings/1.4".into(),
            input_digest: repo.input_digest().unwrap(),
            renderer: RENDERER.into(),
            extractor: "test-extractor".into(),
            revisions: BTreeMap::new(),
            coverage: BTreeMap::new(),
            catalogues: BTreeMap::new(),
            fragments: BTreeMap::new(),
            observations: BTreeMap::new(),
            narratives: BTreeMap::from([(
                SUBJECT.into(),
                Narrative {
                    schema: "codeclew-documentation-narrative/1.3".into(),
                    subject: SUBJECT.into(),
                    context_digest: "sha256:test-context".into(),
                    operations: Vec::new(),
                    gaps: BTreeMap::from([("operation".into(), gap.into())]),
                },
            )]),
            output_hashes: BTreeMap::new(),
            retained_sources: BTreeMap::new(),
            section_states: BTreeMap::new(),
            target_revisions: BTreeMap::new(),
            update_failures: BTreeMap::new(),
            accepted_versions: BTreeMap::new(),
        }
    }

    fn commit(
        repo: &Repository,
        bundle: &str,
        binding: Bindings,
        previous: Option<&(String, Bindings)>,
        previous_bytes: Option<&[u8]>,
        receipt_request: Option<&PublicationReceiptRequest>,
        before_switch: Option<TestHook<'_>>,
    ) -> Result<(), ClewError> {
        let overview = format!(
            "<!-- codeclew-bundle {bundle} -->\n<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"></head><body><main>test</main></body></html>\n"
        );
        let files = BTreeMap::from([(
            "overview.html".into(),
            b"<!doctype html><html lang=\"en\"><head></head><body><main>test</main></body></html>\n"
                .to_vec(),
        )]);
        let input_digest = repo.input_digest()?;
        commit_bundle_with_mode(
            repo,
            bundle,
            binding,
            files,
            &overview,
            &input_digest,
            previous,
            previous_bytes,
            false,
            receipt_request,
            before_switch,
        )
    }

    fn receipt_request(language: Option<&str>) -> PublicationReceiptRequest {
        PublicationReceiptRequest {
            requested_language: language.map(str::to_owned),
            affected_subjects: BTreeSet::from([SUBJECT.into()]),
        }
    }

    #[test]
    fn pre_switch_receipt_uses_final_bundle_bytes_and_same_write_guard() {
        let (_directory, repo) = repository();
        let bundle = "a".repeat(64);
        let receipt_request = receipt_request(Some("ru"));
        let receipt = {
            let mut received = None;
            {
                let callback_repo = &repo;
                let mut callback = |guard: &store::WriteLock, receipt: &PublicationReceipt| {
                    let _: &store::WriteLock = guard;
                    assert!(
                        callback_repo.lock().is_err(),
                        "the callback must execute under the repository write lock"
                    );
                    received = Some(receipt.clone());
                    Ok(())
                };
                commit(
                    &repo,
                    &bundle,
                    binding(&repo, Some("ru"), "Missing reviewed detail"),
                    None,
                    None,
                    Some(&receipt_request),
                    Some(&mut callback),
                )
                .unwrap();
            }
            received.expect("pre-switch callback should receive the receipt")
        };
        assert_eq!(
            receipt.schema,
            "codeclew-documentation-publication-receipt/1.0"
        );
        assert_eq!(receipt.bundle_id, bundle);
        assert_eq!(receipt.requested_language.as_deref(), Some("ru"));
        assert_eq!(receipt.effective_language, "ru");
        assert_eq!(
            receipt.effective_gaps,
            BTreeMap::from([(
                SUBJECT.into(),
                BTreeMap::from([("operation".into(), "Missing reviewed detail".into())]),
            )])
        );

        let index = fs::read(repo.path("docs/index.html").unwrap()).unwrap();
        let bundle_root = format!("docs/generated/{bundle}");
        let bindings_bytes =
            fs::read(repo.path(&format!("{bundle_root}/bindings.json")).unwrap()).unwrap();
        let publication_bytes = fs::read(
            repo.path(&format!("{bundle_root}/publication.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(receipt.root_index_hash, canonical::hash_bytes(&index));
        assert_eq!(
            receipt.bindings_hash,
            canonical::hash_bytes(&bindings_bytes)
        );
        assert_eq!(
            receipt.publication_hash,
            canonical::hash_bytes(&publication_bytes)
        );
        let serialized = serde_json::to_value(&receipt).unwrap();
        assert_eq!(serialized["effectiveLanguage"], "ru");
        assert_eq!(
            serialized["effectiveGaps"][SUBJECT]["operation"],
            "Missing reviewed detail"
        );
    }

    #[test]
    fn pre_switch_callback_failure_keeps_the_previous_reader_pointer() {
        let (_directory, repo) = repository();
        let first_bundle = "b".repeat(64);
        commit(
            &repo,
            &first_bundle,
            binding(&repo, Some("en"), "Initial gap"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let old_index = fs::read(repo.path("docs/index.html").unwrap()).unwrap();
        let previous = bindings::baseline(&repo).unwrap().unwrap();
        assert_eq!(previous.0, first_bundle);

        let second_bundle = "c".repeat(64);
        let request = receipt_request(Some("ru"));
        let mut callback = |_guard: &store::WriteLock, _receipt: &PublicationReceipt| {
            Err(invalid("injected pre-switch failure"))
        };
        assert!(
            commit(
                &repo,
                &second_bundle,
                binding(&repo, Some("ru"), "Updated gap"),
                Some(&previous),
                Some(&old_index),
                Some(&request),
                Some(&mut callback),
            )
            .is_err()
        );
        assert_eq!(
            fs::read(repo.path("docs/index.html").unwrap()).unwrap(),
            old_index
        );
        assert_eq!(bindings::baseline(&repo).unwrap().unwrap().0, first_bundle);
    }
}
