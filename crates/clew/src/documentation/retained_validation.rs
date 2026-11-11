//! Validation and binding construction for retained structured records; no output writer.
use super::{
    bindings::{self, Bindings, FragmentBinding},
    check::Check,
    contracts, digest, invalid, io_error,
    model::*,
    store::{self},
};
use crate::{
    canonical,
    error::{ClewError, ErrorCode},
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SUMMARY_TEXT_MAX_BYTES: usize = 2048;
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
    if source_ids.iter().any(|id| {
        !supported.contains(id)
            && !checked
                .services
                .values()
                .find_map(|e| e.sources.get(id))
                .is_some_and(|source| {
                    deps.iter().any(|dependency| {
                        retained_file_scope_binds_source(
                            &checked.dependencies[dependency],
                            source,
                            checked,
                        )
                    })
                })
    }) {
        return Err(invalid(
            "fragment source is not bound to its claimed dependencies",
        ));
    }
    Ok(())
}
pub(super) fn retained_file_scope_binds_source(
    scope: &Observation,
    source: &Source,
    checked: &Check,
) -> bool {
    if scope.kind != "SOURCE_SCOPE"
        || scope.service != source.service
        || source.authority != "EXACT_SNAPSHOT_TEXT"
        || source.text.is_empty()
        || source.start_line != 1
        || source.end_line != source.text.lines().count() as u64
        || canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
        || digest(&scope.normalized).ok().as_ref() != Some(&scope.digest)
    {
        return false;
    }
    let Some(evidence) = checked.services.get(&source.service) else {
        return false;
    };
    if evidence.service != source.service
        || evidence.revision != source.revision
        || evidence.extractor != SOURCE_EXTRACTOR
        || evidence.sources.get(&source.id) != Some(source)
        || !evidence
            .observations
            .get(&scope.id)
            .is_some_and(|registered| {
                registered.kind == "SOURCE_SCOPE"
                    && registered.service == scope.service
                    && registered.digest == scope.digest
                    && digest(&registered.normalized).ok().as_ref() == Some(&scope.digest)
            })
    {
        return false;
    }
    let file = &scope.normalized["inventory"][&source.file];
    let inventory_matches = match file["coverage"].as_str() {
        Some("FILE_ONLY") => file["digest"] == source.text_digest,
        Some("SYNTAX") => {
            // SYNTAX inventory stores a tree fingerprint, not a text SHA256.
            // Only the producer's complete file record can use this binding;
            // an arbitrary declaration or expression span must not widen it.
            super::analysis::source_id(&source.service, &format!("file:{}", source.file))
                .ok()
                .as_ref()
                == Some(&source.id)
                && scope.normalized["language"]
                    .as_str()
                    .is_some_and(|language| {
                        super::syntax::retained_file_fingerprint(language, &source.text)
                            .ok()
                            .is_some_and(|fingerprint| file["digest"] == fingerprint)
                    })
        }
        _ => false,
    };
    if !inventory_matches {
        return false;
    }
    let Some(occurrence) = &source.occurrence else {
        return false;
    };
    occurrence.start_byte == 0
        && occurrence.end_byte == source.text.len()
        && !occurrence.snapshot.is_empty()
        && !occurrence.blob.is_empty()
        && digest(&(
            SOURCE_EXTRACTOR,
            &occurrence.snapshot,
            &occurrence.blob,
            occurrence.start_byte,
            occurrence.end_byte,
        ))
        .ok()
        .as_ref()
            == Some(&source.evidence_digest)
}
fn validate_summary_text(text: &str) -> Result<(), ClewError> {
    if text.trim().is_empty() || text.contains(['`', '<']) || text.len() > SUMMARY_TEXT_MAX_BYTES {
        return Err(invalid(format!(
            "summary.text must be nonblank plain prose of at most {SUMMARY_TEXT_MAX_BYTES} UTF-8 bytes, without backticks or '<'; received {} UTF-8 bytes. Rewrite the summary within these bounds, preserving its evidence references",
            text.len()
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
pub(super) fn sequence_diagnostic_id(id: &str) -> &str {
    // Evidence IDs are opaque handles. Never echo a path, snippet, or control
    // character from malformed imported evidence into a validation error.
    if !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_'))
    {
        id
    } else {
        "[redacted unsafe ID]"
    }
}
fn missing_sequence_flow(dependency: &Observation, operation_id: &str) -> ClewError {
    let flow_kind = dependency.normalized["kind"].as_str().unwrap_or("");
    let expected = sequence_event_kinds(flow_kind);
    let source_ids: Vec<_> = dependency
        .source_ids
        .iter()
        .take(8)
        .map(|id| sequence_diagnostic_id(id))
        .collect();
    let group_action = if expected
        .iter()
        .any(|kind| matches!(*kind, "alt" | "opt" | "loop"))
    {
        " Proposal groups use children; alt may also use otherwise. Their end events are generated automatically. Close the group with end only in a normalized narrative."
    } else {
        ""
    };
    invalid(format!(
        "sequence omits a mandatory source-backed FLOW: flowId={}, flowKind={}, sourceIds=[{}], sourceIdsOmitted={}, allowedEventKinds=[{}]. Add an event with one of these kinds. For proposal steps, cite this FLOW's Work reference in meaning.evidence; do not add dependencyIds. For normalized narrative events, put the FLOW ID in dependencyIds. Author the supported condition or outcome in text. Summary or explanation citations alone do not cover it.{}",
        sequence_diagnostic_id(&dependency.id),
        if expected.is_empty() {
            "UNKNOWN"
        } else {
            flow_kind
        },
        source_ids.join(", "),
        dependency.source_ids.len().saturating_sub(source_ids.len()),
        expected.join(", "),
        group_action,
    )).with_relevant("MANDATORY_FLOW_UNCOVERED")
        .with_relevant(sequence_diagnostic_id(&dependency.id))
        .with_relevant(sequence_diagnostic_id(operation_id))
}

pub(super) fn sequence_step_requires_endpoints(step_kind: &str) -> bool {
    matches!(
        step_kind,
        "message" | "return" | "declared" | "declared-boundary" | "declared-boundary-return"
    )
}
pub(super) fn sequence_step_requires_interaction(step_kind: &str) -> bool {
    step_kind == "declared" || super::declared_diagrams::boundary(step_kind)
}
fn validate_declared_arrow(event: &Event, checked: &Check, subject: &str) -> Result<(), ClewError> {
    let id = event
        .interaction
        .as_ref()
        .ok_or_else(|| invalid("declared transition needs an interaction ID"))?;
    let check = checked
        .interactions
        .get(id)
        .ok_or_else(|| invalid(format!("unknown declared interaction {id}")))?;
    let dependency = checked
        .dependencies
        .get(&format!("interaction:{id}"))
        .ok_or_else(|| invalid(format!("interaction {id}: declaration missing")))?;
    if dependency.normalized["external"] == true {
        return Err(invalid(format!(
            "interaction {id}: external boundary cannot be a resolved declared arrow; describe the caller's source-observed request or publish in a note and source-derived contract, and state the receiver gap; do not add a captured receiver or synchronous reply"
        )));
    }
    if event.kind == "return" && dependency.normalized["transport"]["kind"] == "kafka" {
        return Err(invalid(format!(
            "interaction {id}: Kafka delivery has no synchronous return; declare a separate reply-event interaction"
        )));
    }
    if check.origin == "agent-proposal"
        || check.from.status != "RESOLVED"
        || check.to.status != "RESOLVED"
        || check.call_site.status != "RESOLVED"
    {
        return Err(invalid(format!(
            "interaction {id}: declared arrow endpoints or selected call site are unresolved (from={}, to={}, callSite={}, origin={}); select unambiguous source endpoints and a resolved call site in an admitted declaration, then recompose; SOURCE_MATCH alone does not establish a transition",
            check.from.status, check.to.status, check.call_site.status, check.origin
        )));
    }
    if !subject.strip_prefix("scenario:").is_some_and(|scenario| {
        checked.scenarios.get(scenario).is_some_and(|scenario| {
            scenario.steps.iter().any(|step| {
                matches!(
                    step.kind.as_str(),
                    "DECLARED_HTTP_TRANSITION" | "DECLARED_KAFKA_TRANSITION"
                ) && step.detail["interaction"] == *id
            })
        })
    }) {
        return Err(invalid(format!(
            "interaction {id}: declared arrow is not reachable in this scenario context ({subject}); use a scenario Work whose composed steps contain this transition; service Work declaration evidence alone does not authorize an arrow"
        )));
    }
    Ok(())
}
pub(super) fn sequence_skipped(checked: &Check, subject: &str, operation_id: &str) -> bool {
    let service_summary = subject.starts_with("service:")
        && (super::sections::contains(operation_id) || super::notes::is_root(operation_id));
    super::dataflow::is_root(checked, subject, operation_id)
        || service_summary
        || super::processes::overview(checked, subject, operation_id)
}
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
            if let Some(root) =
                super::process_graph::internal_operation_root(checked, id, operation_id)
            {
                return Ok(evidence
                    .observations
                    .values()
                    .filter(|flow| {
                        flow.kind == "FLOW"
                            && flow.belongs_to_declaration(root)
                            && flow.normalized["scope"] == root.normalized["scope"]
                            && flow.normalized["kind"]
                                .as_str()
                                .is_some_and(is_required_flow)
                    })
                    .collect());
            }
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
    validate_with_retained(n, checked, None)
}
pub(super) fn validate_with_retained(
    n: &Narrative,
    checked: &Check,
    retained: Option<&Narrative>,
) -> Result<(), ClewError> {
    if !matches!(
        n.schema.as_str(),
        "codeclew-documentation-narrative/1.3" | "codeclew-documentation-narrative/1.4"
    ) {
        return Err(invalid(
            "only codeclew-documentation-narrative/1.3 or codeclew-documentation-narrative/1.4 is supported; author the current contract",
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
    let (mut expected, allowed): (BTreeSet<String>, BTreeSet<String>) = match kind {
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
            checked
                .scenarios
                .get(id)
                .ok_or_else(|| invalid("unknown scenario"))?;
            (
                super::processes::expected(checked, id),
                super::processes::selected_services(checked, id),
            )
        }
        _ => return Err(invalid("unsupported narrative subject")),
    };
    if kind == "service" {
        // Only authored internal roots enter this optional view. They neither
        // change the capture nor turn every callable into required coverage.
        for operation in &n.operations {
            if super::process_graph::internal_operation_root(checked, id, &operation.id).is_some() {
                expected.insert(operation.id.clone());
            }
        }
    }
    let mut covered = BTreeSet::new();
    for (operation_index, o) in n.operations.iter().enumerate() {
        super::language::validate(o.documentation_language.as_deref())?;
        if !expected.contains(&o.id) || !covered.insert(o.id.clone()) || o.title.trim().is_empty() {
            return Err(invalid("duplicate or out-of-scope operation"));
        }
        validate_summary_text(&o.summary.text)
            .map_err(|error| invalid(format!("operations[{operation_index}].{}", error.message)))?;
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
                || (!super::sections::contains(&o.id) && !o.explanation.is_empty())
                || !o.interface_contracts.is_empty()
                || !o.findings.is_empty()
                || !o.participants.is_empty()
                || o.overview_diagram.is_some()
            {
                return Err(invalid(
                    "standard sections contain an evidence-bound summary and limitations; sequence content belongs to operations",
                ));
            }
            if super::sections::contains(&o.id) {
                if o.explanation.len() > 128 {
                    return Err(invalid("service section body exceeds 128 paragraphs"));
                }
                let mut paragraphs = BTreeSet::from([o.summary.id.clone()]);
                for paragraph in &o.explanation {
                    if !store::valid_id(&paragraph.id)
                        || !paragraphs.insert(paragraph.id.clone())
                        || paragraph.text.trim().is_empty()
                        || paragraph.text.len() > 8192
                        || !paragraph.event_ids.is_empty()
                    {
                        return Err(invalid(
                            "service section body needs distinct evidence-bound paragraphs of at most 8192 bytes without sequence anchors",
                        ));
                    }
                    super::explanation_authorship::validate_metadata(paragraph)?;
                    let original = retained
                        .and_then(|n| n.operations.iter().find(|old| old.id == o.id))
                        .and_then(|old| old.explanation.iter().find(|p| p.id == paragraph.id));
                    let historical_context =
                        super::explanation_authorship::validate(paragraph, checked).is_err()
                            && original.is_some_and(|old| {
                                super::explanation_authorship::same_context(old, paragraph)
                            });
                    if !historical_context {
                        super::explanation_authorship::validate(paragraph, checked)?;
                        supported_refs(
                            &paragraph.dependency_ids,
                            &paragraph.source_ids,
                            checked,
                            &allowed,
                        )?;
                    }
                }
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
        // Explain unsupported boundary arrows before a missing external service
        // participant obscures the actual authoring constraint.
        for event in &o.events {
            if sequence_step_requires_interaction(&event.kind)
                || (event.kind == "return" && event.interaction.is_some())
            {
                if super::declared_diagrams::boundary(&event.kind) {
                    super::declared_diagrams::validate_arrow(event, checked, &n.subject)?;
                } else {
                    validate_declared_arrow(event, checked, &n.subject)?;
                }
            }
        }
        let external = super::external_services::for_page(checked, &n.subject);
        if n.schema.ends_with("/1.3")
            && (o.participants.iter().any(|p| p.external_service.is_some())
                || o.events
                    .iter()
                    .any(|e| super::declared_diagrams::boundary(&e.kind)))
        {
            return Err(invalid(
                "external participants and declared boundary steps require narrative/1.4",
            ));
        }
        let mut participants = BTreeMap::new();
        for p in &o.participants {
            if !store::valid_id(&p.id)
                || participants
                    .insert(
                        p.id.clone(),
                        p.service.clone().or_else(|| p.external_service.clone()),
                    )
                    .is_some()
                || p.label.trim().is_empty()
                || (p.service.is_some() && p.external_service.is_some())
                || p.external_service
                    .as_ref()
                    .is_some_and(|id| !external.as_object().is_some_and(|map| map.contains_key(id)))
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
        for (event_index, e) in o.events.iter().enumerate() {
            let group_error = |message: &str| {
                invalid(format!(
                    "operations[{operation_index}].events[{event_index}]: {message}"
                ))
            };
            if !store::valid_id(&e.id) || !ids.insert(e.id.clone()) {
                return Err(group_error("unsafe or duplicate sequence fragment ID"));
            }
            if e.text.len() > 1024 {
                return Err(group_error(&format!(
                    "sequence event text exceeds its UTF-8 byte limit: actualBytes={} maxBytes=1024; shorten the text or split a note into bounded steps while preserving meaning, evidence and the operation's event limit",
                    e.text.len()
                )));
            }
            if super::declared_diagrams::boundary(&e.kind) {
                super::declared_diagrams::supported_refs(
                    &e.dependency_ids,
                    &e.source_ids,
                    checked,
                    &n.subject,
                )?;
            } else {
                supported_refs(&e.dependency_ids, &e.source_ids, checked, &allowed)?;
            }
            if let Some(reader) = &e.reader {
                if reader
                    .label
                    .as_ref()
                    .is_some_and(|label| label.text.trim().is_empty() || label.text.len() > 160)
                    || reader.execution.as_ref().is_some_and(|execution| {
                        execution.meaning.text.trim().is_empty()
                            || execution.meaning.text.len() > 8192
                            || !matches!(
                                execution.mode.as_str(),
                                "synchronous"
                                    | "wait"
                                    | "parallel-await"
                                    | "background"
                                    | "unknown"
                            )
                    })
                {
                    return Err(invalid("reader label or execution mode is invalid"));
                }
                if let Some(label) = &reader.label {
                    let declaration_only = !label.dependency_ids.is_empty()
                        && label.dependency_ids.iter().all(|id| {
                            checked
                                .dependencies
                                .get(id)
                                .is_some_and(|dependency| dependency.kind == "DECLARED_INTERACTION")
                        });
                    if super::declared_diagrams::boundary(&e.kind) && declaration_only {
                        if label
                            .dependency_ids
                            .iter()
                            .any(|id| !e.dependency_ids.contains(id))
                            || label.source_ids.iter().any(|id| !e.source_ids.contains(id))
                        {
                            return Err(invalid(
                                "boundary label must retain its own event's declaration evidence",
                            ));
                        }
                        super::declared_diagrams::supported_refs(
                            &label.dependency_ids,
                            &label.source_ids,
                            checked,
                            &n.subject,
                        )?;
                    } else {
                        supported_refs(
                            &label.dependency_ids,
                            &label.source_ids,
                            checked,
                            &allowed,
                        )?;
                    }
                }
                if let Some(execution) = &reader.execution {
                    if execution.meaning.dependency_ids.iter().all(|id| {
                        checked
                            .dependencies
                            .get(id)
                            .is_some_and(|dependency| dependency.kind == "DECLARED_INTERACTION")
                    }) {
                        return Err(invalid(
                            "execution mode requires caller source evidence, not declaration-only evidence",
                        ));
                    }
                    supported_refs(
                        &execution.meaning.dependency_ids,
                        &execution.meaning.source_ids,
                        checked,
                        &allowed,
                    )?;
                }
            }
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
                    | "declared-boundary"
                    | "declared-boundary-return"
            ) {
                return Err(group_error(
                    "unsupported sequence event kind; use an admitted narrative event kind without changing the group structure",
                ));
            }
            match e.kind.as_str() {
                "alt" | "loop" | "opt" => groups.push((e.kind.as_str(), event_index)),
                "else" => {
                    if groups.last().map(|(kind, _)| *kind) != Some("alt") {
                        return Err(group_error(
                            "else requires an open alternative group; close the nested opt/loop with end before the outer else",
                        ));
                    }
                }
                "end" => {
                    if groups.pop().is_none() {
                        return Err(group_error(
                            "sequence has an unmatched end; preserve one end for each open alt/opt/loop",
                        ));
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
            if sequence_step_requires_endpoints(&e.kind) {
                if !super::declared_diagrams::boundary(&e.kind)
                    && o.participants.iter().any(|p| {
                        p.external_service.is_some()
                            && (e.from.as_ref() == Some(&p.id) || e.to.as_ref() == Some(&p.id))
                    })
                {
                    return Err(invalid(
                        "external participant arrows require declared-boundary or declared-boundary-return; source-derived messages cannot claim an external call",
                    ));
                }
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
                    && !super::declared_diagrams::boundary(&e.kind)
                    && !(e.kind == "return" && e.interaction.is_some())
                {
                    return Err(invalid(
                        "cross-service arrows must reference a declared interaction",
                    ));
                }
                if sequence_step_requires_interaction(&e.kind)
                    || (e.kind == "return" && e.interaction.is_some())
                {
                    let id = e
                        .interaction
                        .as_ref()
                        .ok_or_else(|| invalid("declared transition needs an interaction ID"))?;
                    let dependency = checked
                        .dependencies
                        .get(&format!("interaction:{id}"))
                        .ok_or_else(|| invalid("interaction declaration missing"))?;
                    let (expected_from, expected_to) =
                        if matches!(e.kind.as_str(), "return" | "declared-boundary-return") {
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
                }
            }
        }
        if let Some((_, event_index)) = groups.last() {
            return Err(invalid(format!(
                "operations[{operation_index}].events[{event_index}]: sequence contains an unclosed group; preserve its matching end before moving any later return or alternative"
            )));
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
                return Err(missing_sequence_flow(dependency, &o.id));
            }
        }
        if o.explanation.len() > 128 {
            return Err(invalid("operation explanation exceeds 128 paragraphs"));
        }
        for (paragraph_index, paragraph) in o.explanation.iter().enumerate() {
            super::explanation_authorship::validate_metadata(paragraph)?;
            let original = retained
                .and_then(|n| n.operations.iter().find(|old| old.id == o.id))
                .and_then(|old| old.explanation.iter().find(|p| p.id == paragraph.id));
            let historical_context = super::explanation_authorship::validate(paragraph, checked)
                .is_err()
                && original
                    .is_some_and(|old| super::explanation_authorship::same_context(old, paragraph));
            if !historical_context {
                super::explanation_authorship::validate(paragraph, checked)?;
            }
            let diagnostic_prefix = format!(
                "operation {}, explanation paragraph {}",
                operation_index + 1,
                paragraph_index + 1
            );
            if !store::valid_id(&paragraph.id) {
                return Err(invalid(format!(
                    "{diagnostic_prefix}: explanation paragraph ID has invalid syntax"
                )));
            }
            if !ids.insert(paragraph.id.clone()) {
                return Err(invalid(format!(
                    "{diagnostic_prefix}: explanation paragraph ID is already used in this operation"
                )));
            }
            if paragraph.text.trim().is_empty() {
                return Err(invalid(format!(
                    "{diagnostic_prefix}: explanation text must not be empty"
                )));
            }
            if paragraph.text.len() > 8192 {
                return Err(invalid(format!(
                    "{diagnostic_prefix}: explanation text exceeds the 8192-byte limit"
                )));
            }
            // Explanation bodies support safe Markdown. Summaries remain plain
            // prose; exact source/evidence views never pass through Markdown.
            if paragraph.event_ids.is_empty() {
                return Err(invalid(format!(
                    "{diagnostic_prefix}: reference at least one diagram step"
                )));
            }
            if paragraph
                .event_ids
                .iter()
                .any(|id| !o.events.iter().any(|e| &e.id == id && e.kind != "end"))
            {
                return Err(invalid(format!(
                    "{diagnostic_prefix}: references a missing or end-marker diagram step"
                )));
            }
            if !historical_context {
                if paragraph.source_ids.is_empty()
                    && paragraph.event_ids.iter().all(|id| {
                        o.events
                            .iter()
                            .any(|e| &e.id == id && super::declared_diagrams::boundary(&e.kind))
                    })
                {
                    super::declared_diagrams::supported_refs(
                        &paragraph.dependency_ids,
                        &paragraph.source_ids,
                        checked,
                        &n.subject,
                    )?;
                } else {
                    supported_refs(
                        &paragraph.dependency_ids,
                        &paragraph.source_ids,
                        checked,
                        &allowed,
                    )?;
                }
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
        }
        if o.events.iter().filter(|e| e.kind != "end").any(|e| {
            !o.explanation.iter().any(|p| {
                p.event_ids.contains(&e.id)
                    && super::explanation_authorship::validate(p, checked).is_ok()
            })
        }) {
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
            let mut fields = BTreeSet::new();
            for row in &contract.rows {
                if let Some(field) = &row.field
                    && (!field.valid()
                        || !fields.insert((field.direction.clone(), field.path.clone())))
                {
                    return Err(invalid(
                        "contract field requires a unique direction/path and an individual supported meaning",
                    ));
                }
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
        let internal_source = n
            .subject
            .strip_prefix("service:")
            .and_then(|service| checked.services.get(service))
            .is_some_and(|service| {
                service.entrypoints.iter().any(|entry| {
                    &entry.id == id && !super::process_candidates::public_boundary(entry)
                })
            });
        !internal_source
            && !super::sections::contains(id)
            && !super::notes::is_root(id)
            && id != super::processes::OVERVIEW
            && id != super::dataflow::ROOT
    }) {
        return Err(invalid(
            "full scope requires every discovered public boundary to have an operation or an explicit gap",
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
fn default_gap(id: &str) -> String {
    super::sections::REQUIRED
        .iter()
        .find(|(key, _, _)| *key == id)
        .map(|(_, _, purpose)| {
            format!("{purpose} Source-bound section content has not been accepted yet.")
        })
        .unwrap_or_else(|| "Behavior is not yet authored. Load this entrypoint with clew docs context and supply a source-bound sequence.".into())
}
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
    make_bindings_with_retained(checked, narratives, None)
}
fn make_bindings_with_retained(
    checked: &Check,
    narratives: BTreeMap<String, Narrative>,
    retained: Option<&Bindings>,
) -> Result<Bindings, ClewError> {
    let mut fragments = BTreeMap::new();
    for (subject, n) in &narratives {
        for o in &n.operations {
            let prefix = format!("{subject}/{}", o.id);
            if let Some(root) = subject.strip_prefix("service:").and_then(|service| {
                super::process_graph::internal_operation_root(checked, service, &o.id)
            }) {
                add_binding(
                    &mut fragments,
                    format!("{prefix}/root"),
                    subject,
                    &root.id,
                    std::slice::from_ref(&root.id),
                    &root.source_ids,
                    checked,
                )?;
            }
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
                let (dependencies, sources) = e.binding_refs();
                add_binding(
                    &mut fragments,
                    format!("{prefix}/{}", e.id),
                    subject,
                    e,
                    &dependencies,
                    &sources,
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
                let original = retained
                    .and_then(|binding| binding.narratives.get(subject))
                    .and_then(|n| n.operations.iter().find(|old| old.id == o.id))
                    .and_then(|old| old.explanation.iter().find(|p| p.id == paragraph.id));
                if original
                    .is_some_and(|old| super::explanation_authorship::same_context(old, paragraph))
                {
                    let key = format!("{prefix}/{}", paragraph.id);
                    let baseline = retained.unwrap();
                    if !super::explanation_authorship::authored_binding(baseline, &key)? {
                        return Err(invalid(
                            "protected paragraph original binding is unavailable",
                        ));
                    }
                    let mut binding = baseline.fragments[&key].clone();
                    binding.content = serde_json::to_value(paragraph).map_err(io_error)?;
                    binding.content_digest = digest(paragraph)?;
                    fragments.insert(key, binding);
                    continue;
                }
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
        endpoint_publication: None,
        reviewed_answers: retained
            .map(|b| b.reviewed_answers.clone())
            .unwrap_or_default(),
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
    super::freshness::update_states(&mut binding, checked);
    Ok(binding)
}
