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

const TEMPLATE: &str = include_str!("../../assets/documentation/template.html");
const STYLE: &str = include_str!("../../assets/documentation/style.css");
const SCRIPT: &str = include_str!("../../assets/documentation/app.js");

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn html(data: &Value) -> Result<String, ClewError> {
    let payload = serde_json::to_string(data)
        .map_err(io_error)?
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(TEMPLATE
        .replace("/*__STYLE__*/", STYLE)
        .replace("/*__SCRIPT__*/", SCRIPT)
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

pub fn validate(n: &Narrative, checked: &Check) -> Result<(), ClewError> {
    if !matches!(
        n.schema.as_str(),
        "codeclew-documentation-narrative/1.0"
            | "codeclew-documentation-narrative/1.1"
            | "codeclew-documentation-narrative/1.2"
            | "codeclew-documentation-narrative/1.3"
    ) || n.context_digest != checked.context_digest
    {
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
            let service = checked
                .services
                .get(id)
                .ok_or_else(|| invalid("service evidence is unresolved"))?;
            (
                service.entrypoints.iter().map(|e| e.id.clone()).collect(),
                BTreeSet::from([id.into()]),
            )
        }
        "scenario" => {
            let scenario = checked
                .scenarios
                .get(id)
                .ok_or_else(|| invalid("unknown scenario"))?;
            (
                BTreeSet::from([id.into()]),
                scenario.steps.iter().map(|s| s.service.clone()).collect(),
            )
        }
        _ => return Err(invalid("unsupported narrative subject")),
    };
    let mut covered = BTreeSet::new();
    for o in &n.operations {
        if !expected.contains(&o.id) || !covered.insert(o.id.clone()) || o.title.trim().is_empty() {
            return Err(invalid("duplicate or out-of-scope operation"));
        }
        if o.summary.text.trim().is_empty()
            || o.summary.text.contains(['`', '<'])
            || o.summary.text.len() > 2048
        {
            return Err(invalid(
                "summary must be brief plain prose without implementation code",
            ));
        }
        supported_refs(
            &o.summary.dependency_ids,
            &o.summary.source_ids,
            checked,
            &allowed,
        )?;
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
        if n.schema.ends_with("/1.3") && o.overview_diagram.is_none() {
            return Err(invalid("narrative 1.3 requires a bounded overview diagram"));
        }
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
        if !n.schema.ends_with("/1.0")
            && o.events
                .iter()
                .filter(|e| e.kind != "end")
                .any(|e| !o.explanation.iter().any(|p| p.event_ids.contains(&e.id)))
        {
            return Err(invalid(
                "narrative 1.1+ requires a domain explanation covering every diagram step",
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
    if expected != covered {
        return Err(invalid(
            "full scope requires every discovered entrypoint to have an operation or an explicit gap",
        ));
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
    Narrative{schema:"codeclew-documentation-narrative/1.0".into(),subject,context_digest:checked.context_digest.clone(),operations:vec![],gaps:ids.map(|id|(id,"Behavior is not yet authored. Load this entrypoint with clew docs context and supply a source-bound sequence.".into())).collect()}
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
    for (id, evidence) in &checked.services {
        let subject = format!("service:{id}");
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
    let observations = referenced
        .into_iter()
        .map(|id| (id.clone(), checked.dependencies[&id].clone()))
        .collect();
    Ok(Bindings {
        schema: "codeclew-documentation-bindings/1.0".into(),
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
    })
}

fn page_data(subject: &str, title: &str, subtitle: &str, n: &Narrative, checked: &Check) -> Value {
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
    let mut sources = BTreeSet::new();
    for o in &n.operations {
        sources.extend(o.summary.source_ids.clone());
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
    json!({"subject":subject,"title":title,"subtitle":subtitle,"operations":n.operations,"gaps":n.gaps,"catalogue":catalogue,"sources":chosen_sources,"contracts":contract_rows,"revisions":selected_services.iter().filter_map(|id|checked.services.get(id).map(|e|(id,&e.revision))).collect::<BTreeMap<_,_>>(),"boundaries":boundaries,"coverage":selected_services.iter().filter_map(|id|checked.services.get(id).map(|e|(id,&e.coverage))).collect::<BTreeMap<_,_>>(),"interactions":checked.interactions.values().filter(|i|service_id.is_some_and(|id|checked.dependencies[&format!("interaction:{}",i.id)].normalized["from"]["service"]==id||checked.dependencies[&format!("interaction:{}",i.id)].normalized["to"]["service"]==id)||checked.scenarios.get(id_from_subject(subject)).is_some_and(|s|s.dependency_ids.contains(&format!("interaction:{}",i.id)))).collect::<Vec<_>>(),"extractor":EXTRACTOR,"renderer":RENDERER})
}

pub fn mermaid(o: &Operation) -> String {
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
    if o.events.len() > 12 {
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

fn markdown(title: &str, n: &Narrative) -> String {
    let mut out = format!(
        "# {}\n\nStatic source interpretation. Declared interactions do not establish runtime routing.\n\n",
        escape(title)
    );
    for o in &n.operations {
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
    let previous = bindings::baseline(repo)?;
    if let Some((id, binding)) = &previous {
        bindings::verify_outputs(repo, id, binding)?;
    }
    let previous_root = repo.path("docs/index.html")?;
    let previous_bytes = if previous_root.exists() {
        Some(fs::read(&previous_root).map_err(io_error)?)
    } else {
        None
    };
    let checked = check::run(repo)?;
    checked.save(repo)?;
    if !checked.unresolved.is_empty() {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "documentation source check is unresolved; inspect clew docs check before publication",
        ));
    }
    let freshness = bindings::freshness(previous.as_ref().map(|(_, b)| b), &checked);
    let mut narratives = previous
        .as_ref()
        .map(|(_, b)| b.narratives.clone())
        .unwrap_or_default();
    narratives.retain(|subject, _| {
        subject
            .strip_prefix("service:")
            .is_some_and(|id| checked.services.contains_key(id))
            || subject
                .strip_prefix("scenario:")
                .is_some_and(|id| checked.scenarios.contains_key(id))
    });
    let mut replaced = BTreeSet::new();
    for n in incoming {
        validate(&n, &checked)?;
        if !replaced.insert(n.subject.clone()) {
            return Err(invalid("duplicate incoming narrative subject"));
        }
        narratives.insert(n.subject.clone(), n);
    }
    let affected_subjects: BTreeSet<_> = freshness["affected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["subject"].as_str())
        .collect();
    for (subject, n) in &mut narratives {
        if !replaced.contains(subject) && affected_subjects.contains(subject.as_str()) {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                format!(
                    "review affected fragments for {subject} and provide its updated narrative"
                ),
            ));
        }
        n.context_digest = checked.context_digest.clone();
        validate(n, &checked)?;
    }
    let services = repo.services()?;
    let scenarios = repo.scenarios()?;
    let interactions = repo.interactions()?;
    for (id, e) in &checked.services {
        narratives
            .entry(format!("service:{id}"))
            .or_insert_with(|| {
                default_narrative(
                    format!("service:{id}"),
                    e.entrypoints.iter().map(|e| e.id.clone()),
                    &checked,
                )
            });
    }
    for id in scenarios.keys() {
        narratives
            .entry(format!("scenario:{id}"))
            .or_insert_with(|| {
                default_narrative(
                    format!("scenario:{id}"),
                    std::iter::once(id.clone()),
                    &checked,
                )
            });
    }
    let gap_count: usize = narratives.values().map(|n| n.gaps.len()).sum();
    if require_complete && gap_count > 0 {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "documentation contains explicit gaps; author every in-scope operation before --require-complete",
        ));
    }
    let bundle=digest(&json!({"context":checked.context_digest,"revisions":checked.services.iter().map(|(id,e)|(id,&e.revision)).collect::<BTreeMap<_,_>>(),"sources":checked.sources(),"narratives":narratives,"renderer":RENDERER,"rendererAssets":digest(&[TEMPLATE, STYLE, SCRIPT])?}))?[7..].to_owned();
    let mut binding = make_bindings(&checked, narratives.clone())?;
    let mut files = BTreeMap::new();
    for (subject, n) in &narratives {
        let (kind, id) = subject
            .split_once(':')
            .ok_or_else(|| invalid("invalid subject"))?;
        let (title, subtitle) = if kind == "service" {
            let s = &services[id];
            (
                s.title.as_str(),
                "All discovered entrypoints, contracts and behavior",
            )
        } else {
            let s = &scenarios[id];
            (s.title.as_str(), s.summary.as_str())
        };
        let folder = if kind == "service" {
            "services"
        } else {
            "scenarios"
        };
        let data = page_data(subject, title, subtitle, n, &checked);
        files.insert(format!("{folder}/{id}.html"), html(&data)?.into_bytes());
        files.insert(format!("{folder}/{id}.json"), bytes(&data)?);
        files.insert(format!("{folder}/{id}.md"), markdown(title, n).into_bytes());
        for o in &n.operations {
            files.insert(
                format!("diagrams/{}-{}.mmd", subject.replace(':', "-"), o.id),
                mermaid(o).into_bytes(),
            );
        }
    }
    let mut cards = String::new();
    for (id, s) in &services {
        let n = &narratives[&format!("service:{id}")];
        cards.push_str(&format!("<article class=\"gap-card\"><div class=\"eyebrow\">MICROSERVICE</div><h3><a href=\"generated/{bundle}/services/{}.html\">{}</a></h3><p>{} documented / {} entrypoints · {} gaps</p><p><code>{}</code> · {}</p></article>",escape(id),escape(&s.title),n.operations.len(),checked.services[id].entrypoints.len(),n.gaps.len(),&checked.services[id].revision[..12],escape(&checked.services[id].coverage)));
    }
    let scenario_cards=scenarios.iter().map(|(id,s)|format!("<article class=\"gap-card\"><div class=\"eyebrow\">INTERACTION SCENARIO</div><h3><a href=\"generated/{bundle}/scenarios/{}.html\">{}</a></h3><p>{}</p><p>{} declared interactions · {} documented operations</p></article>",escape(id),escape(&s.title),escape(&s.summary),s.interactions.len(),narratives[&format!("scenario:{id}")].operations.len())).collect::<String>();
    let interaction_cards=interactions.values().map(|i|format!("<article class=\"gap-card\"><h3>{}</h3><p>{} → {} · {} {} · {}</p><p>{}</p><details class=\"technical-evidence\"><summary>Declaration and supported checks</summary><pre>{}</pre></details></article>",escape(&i.title),escape(&i.from.service),escape(&i.to.service),escape(i.transport.method.as_deref().unwrap_or(&i.transport.kind)),escape(i.transport.topic.as_deref().or(i.transport.path.as_deref()).unwrap_or("unresolved route")),escape(&i.declaration.origin),escape(&i.declaration.rationale),escape(&serde_json::to_string_pretty(&checked.interactions[&i.id]).unwrap_or_default()))).collect::<String>();
    let overview = format!(
        "<!-- codeclew-bundle {bundle} -->\n<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{STYLE}</style></head><body><header class=\"topbar\"><span class=\"brand\"><span class=\"logo\">c</span><b>Codeclew</b><span>Service docs</span></span><span class=\"experiment\">{} services · {} scenarios</span></header><main style=\"margin:auto;max-width:1180px\"><div class=\"coverage-heading\"><div class=\"eyebrow\">ARCHITECTURE DOCUMENTATION</div><h2>{}</h2><p>Each microservice has its own documentation. Named scenarios connect bounded local flows through explicitly declared interactions.</p></div><h2>Microservices</h2><div class=\"coverage-grid\">{cards}</div><h2>Interaction scenarios</h2>{scenario_cards}<h2>Declared service relationships</h2>{interaction_cards}<p class=\"flow-note\">{gap_count} explicit documentation gaps. Source interpretation does not establish runtime activation or wire compatibility.</p></main></body></html>\n",
        escape(&repo.manifest.title),
        services.len(),
        scenarios.len(),
        escape(&repo.manifest.title)
    );
    let bundle_overview = overview.replace(&format!("href=\"generated/{bundle}/"), "href=\"");
    files.insert("overview.html".into(), bundle_overview.into_bytes());
    binding.output_hashes = files
        .iter()
        .map(|(path, bytes)| (path.clone(), canonical::hash_bytes(bytes)))
        .collect();
    files.insert("bindings.json".into(), bytes(&binding)?);
    let _lock = repo.lock()?;
    if repo.input_digest()? != checked.input_digest {
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
    if current_bytes != previous_bytes {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "documentation output changed during rendering",
        ));
    }
    if let Some((id, binding)) = &previous {
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
    repo.atomic("docs/index.html", overview.as_bytes())?;
    Ok(
        json!({"schema":"codeclew-docs-render/1.0","status":if gap_count==0{"RENDERED"}else{"PARTIAL"},"bundle":bundle,"index":"docs/index.html","services":services.len(),"scenarios":scenarios.len(),"documentedOperations":narratives.values().map(|n|n.operations.len()).sum::<usize>(),"explicitGaps":gap_count,"inputDigest":checked.input_digest,"contextDigest":checked.context_digest,"runtime":"UNKNOWN"}),
    )
}
