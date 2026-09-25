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
        let mandatory: Vec<&Observation> = if kind == "service" {
            let evidence = &checked.services[id];
            let entry = evidence
                .entrypoints
                .iter()
                .find(|entry| entry.id == o.id)
                .ok_or_else(|| invalid("operation entrypoint disappeared"))?;
            evidence
                .observations
                .values()
                .filter(|d| {
                    d.kind == "FLOW"
                        && d.symbol == entry.symbol
                        && matches!(
                            d.normalized["kind"].as_str(),
                            Some(
                                "IF" | "LOOP"
                                    | "RETURN"
                                    | "THROW"
                                    | "DEFERRED"
                                    | "TRY"
                                    | "FINALLY"
                                    | "BREAK"
                                    | "CONTINUE"
                            )
                        )
                })
                .collect()
        } else {
            checked.scenarios[id]
                .steps
                .iter()
                .filter(|s| {
                    matches!(
                        s.kind.as_str(),
                        "IF" | "LOOP"
                            | "RETURN"
                            | "THROW"
                            | "DEFERRED"
                            | "TRY"
                            | "FINALLY"
                            | "BREAK"
                            | "CONTINUE"
                    )
                })
                .filter_map(|s| s.dependency_ids.first())
                .filter_map(|id| checked.dependencies.get(id))
                .collect()
        };
        for dependency in mandatory {
            let expected_kind = dependency.normalized["kind"].as_str().unwrap_or("");
            if !o.events.iter().any(|event| {
                event.dependency_ids.contains(&dependency.id)
                    && match expected_kind {
                        "IF" | "TRY" => event.kind == "alt",
                        "DEFERRED" => event.kind == "opt",
                        "FINALLY" | "BREAK" | "CONTINUE" => event.kind == "note",
                        "LOOP" => event.kind == "loop",
                        "RETURN" | "THROW" => matches!(event.kind.as_str(), "return" | "note"),
                        _ => false,
                    }
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

fn page_data(
    subject: &str,
    title: &str,
    subtitle: &str,
    n: &Narrative,
    checked: &Check,
    suppress: &BTreeSet<String>,
    state_diagram: Option<&str>,
    lifecycle: &[(String, String, String)],
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
    json!({"processCandidates":process_candidates,"savedProcesses":saved_processes,"sourceAuthorities":checked.source_authorities(),"analysisEvidence":analysis_evidence,"view":super::dataflow::page(checked,subject),"relatedViews":checked.dependencies.values().filter(|d|d.kind=="VIEW_DEFINITION" && service_id.is_some_and(|id|d.normalized["definition"]["view"]["services"].as_array().is_some_and(|ss|ss.iter().any(|s|s==id)))).map(|d|json!({"id":d.normalized["definition"]["id"],"title":d.normalized["definition"]["title"],"inputObjects":d.normalized["definition"]["view"]["inputObjects"]})).collect::<Vec<_>>(),"process":super::processes::page(checked,subject),"notes":super::notes::page(checked,subject,n),"sections":service_id.map(|id|super::sections::records(id,Some(n))).unwrap_or_default(),"boundaryInventory":service_id.map(|id|super::sections::inventory(id,checked)),"entities":checked.dependencies.values().filter(|d|d.kind=="DOMAIN_ENTITY").collect::<Vec<_>>(),"subject":subject,"title":title,"subtitle":subtitle,"stateDiagram":state_diagram,"activityTransitions":activity_transitions,"lifecycleOperations":lifecycle.iter().map(|(n,_,t)|json!({"name":n,"tree":t})).collect::<Vec<_>>(),"operations":n.operations,"gaps":n.gaps,"catalogue":catalogue,"sources":chosen_sources,"contracts":contract_rows,"revisions":selected_services.iter().filter_map(|id|checked.services.get(id).map(|e|(id,&e.revision))).collect::<BTreeMap<_,_>>(),"boundaries":boundaries,"coverage":selected_services.iter().filter_map(|id|checked.services.get(id).map(|e|(id,&e.coverage))).collect::<BTreeMap<_,_>>(),"interactions":checked.interactions.values().filter(|i|service_id.is_some_and(|id|checked.dependencies[&format!("interaction:{}",i.id)].normalized["from"]["service"]==id||checked.dependencies[&format!("interaction:{}",i.id)].normalized["to"]["service"]==id)||checked.scenarios.get(id_from_subject(subject)).is_some_and(|s|s.dependency_ids.contains(&format!("interaction:{}",i.id)))).collect::<Vec<_>>(),"extractor":EXTRACTOR,"renderer":RENDERER})
}

/// If an operation has no authored events, produce an auto PlantUML activity
/// document from the operation's root FLOW evidence. Returns `None` when there
/// is authored content or no usable flow.
fn auto_flow_puml(
    checked: &Check,
    flow: &serde_json::Value,
    symbol: &str,
    title: &str,
) -> Option<String> {
    // Prefer source deepening: a retained TRANSFORMED_SOURCE yields readable
    // step text (assignments, returns, catch). Fall back to the FLOW renderer
    // when the source is absent or not parseable.
    if let Some(source) = method_source(checked, symbol) {
        if let Some(doc) = super::source_steps::document(&source, symbol, title) {
            return Some(doc);
        }
    }
    super::process_flow::document(flow, symbol, title)
}

/// Look up a method's retained `TRANSFORMED_SOURCE` observation by symbol,
/// preferring the source_text resolved through the `SYMBOL` observation's
/// `source_ids` (the source-linked path used on real evidence) and falling
/// back to the legacy TRANSFORMED_SOURCE observation body.
fn method_source(checked: &Check, symbol: &str) -> Option<String> {
    let sources = checked.sources();
    let via_source_ids = checked
        .services
        .values()
        .find_map(|e| {
            e.observations
                .values()
                .find(|o| o.kind == "SYMBOL" && o.symbol == symbol)
        })
        .and_then(|o| {
            o.source_ids
                .iter()
                .find_map(|id| sources.get(id).map(|s| s.text.clone()))
        });
    if via_source_ids.is_some() {
        return via_source_ids;
    }
    let obs = checked.services.values().find_map(|e| {
        e.observations
            .values()
            .find(|o| o.kind == "TRANSFORMED_SOURCE" && o.symbol == symbol)
    })?;
    let norm = &obs.normalized;
    if let Some(s) = norm
        .pointer("/documentation/source")
        .and_then(Value::as_str)
    {
        return Some(s.to_string());
    }
    if let Some(s) = norm.pointer("/source").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    if let Some(s) = norm.as_str() {
        return Some(s.to_string());
    }
    None
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
) {
    if let Ok(Some(svgs)) = super::plantuml::batch_render_svg(diagrams, jar) {
        for (base, svg) in svgs {
            files.insert(format!("{base}.svg"), svg);
        }
    }
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
) -> Option<(&'a Value, &'a str)> {
    for &symbol in candidates {
        let matches = |o: &Observation| o.kind == "SYMBOL" && o.symbol == symbol;
        let mut obs: Vec<&Observation> = Vec::new();
        if let Some(e) = checked.services.get(service.unwrap_or("")) {
            obs.extend(e.observations.values().filter(|o| matches(*o)));
        }
        if obs.is_empty() {
            for e in checked.services.values() {
                obs.extend(e.observations.values().filter(|o| matches(*o)));
            }
        }
        if obs.is_empty() {
            obs.extend(checked.dependencies.values().filter(|o| matches(*o)));
        }
        if let Some(obs) = deepest_flow(&obs) {
            let events = obs.normalized.pointer("/documentation/events")?;
            return Some((events, &obs.symbol));
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

/// Resolve the deepest retained flow for a lifecycle operation named by a
/// state-schema transition (`operation` field), matching any SYMBOL
/// observation whose symbol contains the operation name.
fn lifecycle_flow<'a>(checked: &'a Check, operation: &str) -> Option<(&'a Value, &'a str)> {
    let mut obs: Vec<&Observation> = Vec::new();
    for e in checked.services.values() {
        obs.extend(
            e.observations
                .values()
                .filter(|o| o.kind == "SYMBOL" && o.symbol.contains(operation)),
        );
    }
    if obs.is_empty() {
        obs.extend(
            checked
                .dependencies
                .values()
                .filter(|o| o.kind == "SYMBOL" && o.symbol.contains(operation)),
        );
    }
    let deepest = deepest_flow(&obs)?;
    let events = deepest.normalized.pointer("/documentation/events")?;
    Some((events, &deepest.symbol))
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
) -> Option<(&'a Value, &'a str)> {
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(service) = service {
        if let Some(entry) = checked
            .services
            .get(service)
            .and_then(|e| e.entrypoints.iter().find(|ep| ep.id == operation.id))
        {
            candidates.push(entry.symbol.as_str());
        }
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
) -> Option<(&'a Value, &'a str)> {
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(service) = service {
        if let Some(entry) = checked
            .services
            .get(service)
            .and_then(|e| e.entrypoints.iter().find(|ep| ep.id == id))
        {
            candidates.push(entry.symbol.as_str());
        }
    }
    candidates.push(id);
    resolve_flow(checked, service, &candidates)
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
    publish_internal(
        repo,
        vec![narrative],
        false,
        BTreeMap::new(),
        versions,
        EvidenceMode::Saved(snapshot),
        language.as_deref(),
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
    )
}

#[derive(Clone, Copy)]
enum EvidenceMode<'a> {
    Saved(Option<&'a str>),
    Refresh,
}

fn publish_internal(
    repo: &Repository,
    incoming: Vec<Narrative>,
    require_complete: bool,
    failures: BTreeMap<String, Value>,
    versions: BTreeMap<String, super::review::AcceptedVersion>,
    evidence_mode: EvidenceMode<'_>,
    language: Option<&str>,
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
        )
    })
}

fn publish_internal_phases(
    repo: &Repository,
    mut incoming: Vec<Narrative>,
    require_complete: bool,
    mut failures: BTreeMap<String, Value>,
    versions: BTreeMap<String, super::review::AcceptedVersion>,
    evidence_mode: EvidenceMode<'_>,
    language: Option<&str>,
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
        let state_diagram = if kind == "scenario" {
            super::process_states::load(repo, id)?.map(|_| format!("scenario-{id}-states"))
        } else {
            None
        };
        let activity_transitions = if kind == "scenario" {
            super::process_states::load(repo, id)?
                .and_then(|schema| super::process_states::activity_transitions_puml(&schema))
                .map(|_| format!("scenario-{id}-activity-transitions"))
        } else {
            None
        };
        let lifecycle: Vec<(String, String, String)> = if kind == "scenario" {
            let mut out = Vec::new();
            if let Some(schema) = super::process_states::load(repo, id)? {
                let mut seen = BTreeSet::new();
                for t in &schema.transitions {
                    if t.operation.is_empty() || !seen.insert(t.operation.clone()) {
                        continue;
                    }
                    if let Some((flow, symbol)) = lifecycle_flow(&checked, &t.operation) {
                        if let Some(puml) = auto_flow_puml(&checked, flow, symbol, &t.operation) {
                            let tree = super::process_flow::tree(flow, symbol).unwrap_or_default();
                            out.push((t.operation.clone(), puml, tree));
                        }
                    }
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
            }
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
        files.insert(format!("{folder}/{id}.json"), bytes(&data)?);
        files.insert(format!("{folder}/{id}.html"), html(&data)?.into_bytes());
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
                if let Some((flow, symbol)) = root_flow_events(&checked, operation, service) {
                    if let Some(puml) = auto_flow_puml(&checked, flow, symbol, &operation.title) {
                        insert_diagram(
                            &mut files,
                            &mut diagrams,
                            format!("diagrams/{}-{}", subject.replace(':', "-"), operation.id),
                            puml,
                        );
                    }
                }
            }
        }
        // Gap entrypoints are un-authored operations that are absent from
        // `operations[]`; they still get an auto PlantUML activity document
        // when their root method's flow is retained. Authored operations are
        // never here, so this cannot override manual content.
        if kind == "service" {
            for gap_id in n.gaps.keys() {
                if let Some((flow, symbol)) = root_flow_events_by_id(&checked, gap_id, Some(id)) {
                    if let Some(puml) = auto_flow_puml(&checked, flow, symbol, gap_id) {
                        insert_diagram(
                            &mut files,
                            &mut diagrams,
                            format!("diagrams/{}-{}", subject.replace(':', "-"), gap_id),
                            puml,
                        );
                    }
                }
            }
        }
        // Declarative state diagram: scenarios/<id>-states.yaml → a PlantUML
        // state diagram. Unresolved transitions are surfaced as limitations
        // rather than dropped.
        if kind == "scenario" {
            if let Some((mut puml, unresolved)) =
                super::process_states::load_and_render(repo, &checked, id)?
            {
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
            if let Some(activity) = super::process_states::load(repo, id)?
                .and_then(|schema| super::process_states::activity_transitions_puml(&schema))
            {
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
            for (op, puml, _) in &lifecycle {
                insert_diagram(
                    &mut files,
                    &mut diagrams,
                    format!("diagrams/{}-{}", subject.replace(':', "-"), op),
                    puml.clone(),
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
    batch_render_diagrams(&mut files, &diagrams, plantuml_jar.as_deref());
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
        "output": output_digest
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
    commit_bundle(
        repo,
        &bundle,
        binding,
        files,
        &overview,
        &checked.input_digest,
        previous.as_ref(),
        previous_bytes.as_deref(),
    )?;
    Ok(
        json!({"schema":"codeclew-docs-render/1.0","documentationLanguage":requested_language,"translationGaps":translation_gap_count,"translationComplete":translation_gap_count==0,"status":if incomplete{"PARTIAL"}else{"RENDERED"},"bundle":bundle,"index":"docs/index.html","services":services.len(),"scenarios":scenarios.len(),"documentedOperations":narratives.values().flat_map(|n|n.operations.iter()).filter(|o|requested_language.as_deref().is_none_or(|language|o.documentation_language.as_deref()==Some(language))).filter(|o|!super::sections::contains(&o.id)&&!super::notes::is_root(&o.id)&&o.id!=super::processes::OVERVIEW&&o.dataflow.is_none()).count(),"documentedViews":narratives.values().flat_map(|n|n.operations.iter()).filter(|o|requested_language.as_deref().is_none_or(|language|o.documentation_language.as_deref()==Some(language))).filter(|o|o.dataflow.is_some()).count(),"documentedSections":narratives.values().flat_map(|n|n.operations.iter()).filter(|o|requested_language.as_deref().is_none_or(|language|o.documentation_language.as_deref()==Some(language))).filter(|o|super::sections::contains(&o.id)).count(),"explicitGaps":gap_count,"inputDigest":checked.input_digest,"contextDigest":checked.context_digest,"updateFailures":failures,"unresolved":checked.unresolved,"runtime":"UNKNOWN","evidenceAuthority":if refreshing{"CURRENT_SOURCE_CHECK"}else{"PINNED_SNAPSHOT_NOT_REVERIFIED"},"snapshot":snapshot}),
    )
}

/// Commit all immutable files before switching the sole reader pointer.
#[allow(clippy::too_many_arguments)]
pub(super) fn commit_bundle(
    repo: &Repository,
    bundle: &str,
    mut binding: Bindings,
    mut files: BTreeMap<String, Vec<u8>>,
    overview: &str,
    input_digest: &str,
    previous: Option<&(String, Bindings)>,
    previous_bytes: Option<&[u8]>,
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
        super::history::prepare(repo, bundle, &binding, &mut files, input_digest, previous)?;
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
    if files
        .values()
        .any(|data| data.len() > check::PORTABLE_CACHE_MAX_BYTES as usize)
    {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "documentation output exceeds its portable record budget; narrow source roots",
        ));
    }
    let _lock = repo.lock()?;
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
    super::history::index(repo, bundle)?;
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
mod tests {
    use super::*;

    #[test]
    fn auto_flow_puml_is_emitted_when_events_empty() {
        let flow = json!([
            {"kind":"CALL","resolution":"COMPILER_EXACT","target":"method:class:ru.tins.CheckoutService#charge()V"},
            {"kind":"RETURN"}
        ]);
        let symbol = "method:class:ru.tins.CheckoutController#checkout";
        let title = "Checkout flow";
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
        let doc = auto_flow_puml(&checked, &flow, symbol, title).unwrap();
        assert!(doc.contains("title Checkout flow"), "{doc}");
        assert!(doc.contains(":CheckoutService#charge;"), "{doc}");
        assert!(
            doc.contains("' evidence: method:class:ru.tins.CheckoutController#checkout"),
            "{doc}"
        );
    }

    #[test]
    fn source_deepening_is_preferred_when_source_retained() {
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
            normalized: json!({"documentation":{"source":source}}),
            digest: "d".into(),
            source_ids: vec![],
        };
        let evidence: ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},
            "entrypoints":[],"observations":{},"sources":{}
        }))
        .unwrap();
        let mut evidence = evidence;
        evidence.observations.insert(src_obs.id.clone(), src_obs);
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
        let doc = auto_flow_puml(&checked, &flow, symbol, "t").unwrap();
        // Source deepening wins over the shallow BOUNDARY flow.
        assert!(doc.contains(":Вход: handle(taskId);"), "{doc}");
        assert!(doc.contains(":ti = null;"), "{doc}");
        assert!(doc.contains("if (ti == null) then (да)"), "{doc}");
        assert!(!doc.contains("BOUNDARY"), "{doc}");
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
        let (events, sym) = root_flow_events(&checked, &operation, Some("svc")).unwrap();
        assert_eq!(sym, symbol);
        assert_eq!(events[0]["text"], "load()");
        // Operation id == method symbol resolves even without an entrypoint.
        let op2 = Operation {
            id: symbol.into(),
            ..operation.clone()
        };
        let (_, sym2) = root_flow_events(&checked, &op2, None).unwrap();
        assert_eq!(sym2, symbol);
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
        let (events, sym) = root_flow_events_by_id(&checked, "svc-abc123", Some("svc")).unwrap();
        assert_eq!(sym, symbol);
        assert_eq!(events[0]["text"], "start()");
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
        let (events, sym) = resolve_flow(&checked, Some("svc"), &[symbol]).unwrap();
        assert_eq!(sym, symbol);
        assert_eq!(events.as_array().map(Vec::len), Some(5));
    }

    #[test]
    fn method_source_resolves_via_symbol_source_ids() {
        let symbol = "method:class:svc.TaskService#handle";
        let source_text = "public void handle(Long id) {\n  svc.doSomething(id);\n}";
        let sym_obs = Observation {
            id: "svc:symbol:handle".into(),
            kind: "SYMBOL".into(),
            service: "svc".into(),
            symbol: symbol.into(),
            normalized: json!({}),
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
        assert_eq!(method_source(&checked, symbol).as_deref(), Some(source_text));
    }
}
