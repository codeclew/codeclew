//! Cited, non-causal Kotlin PSI outlines for retained declaration pages.
use super::super::model::{
    CallableProjection, Citation, SourceOutline, SourceOutlineEvent, SourceOutlineOwnerKey,
};
use super::{Context, gap};
use crate::documentation::model::{Observation, ServiceEvidence, Source};
use crate::documentation::{digest, invalid};
use crate::error::ClewError;
use serde_json::Value;

const DOCUMENTATION_SCHEMA: &str = "codeclew-kotlin-documentation-flow/1.0";
const OUTLINE_AUTHORITY: &str = "KOTLIN_PSI_WITH_K2_CALL_TARGETS";

/// Attach an outline only to an already-admitted Kotlin declaration. Failed
/// outline admission is represented as one cited unavailable marker; it never
/// promotes the declaration into the causal source-behavior projection.
pub(super) fn attach(
    context: &mut Context<'_>,
    owner: &Observation,
    scope: &str,
    callable: &mut CallableProjection,
) -> Result<(), ClewError> {
    let documentation = &owner.normalized["documentation"];
    if documentation["schema"] != DOCUMENTATION_SCHEMA
        || documentation["authority"] != OUTLINE_AUTHORITY
    {
        return Ok(());
    }

    let owner_key = SourceOutlineOwnerKey {
        service: owner.service.clone(),
        scope: scope.to_owned(),
        symbol: owner.symbol.clone(),
    };
    let Some(owner_source) = retained_owner_source(context, owner)
        .filter(|source| valid_source(context.evidence, source, None))
        .cloned()
    else {
        callable.source_outline = Some(unavailable_outline(
            owner_key,
            callable.citation_id.clone(),
            "KOTLIN_SOURCE_OUTLINE_OWNER_SOURCE_UNAVAILABLE",
            "The selected Kotlin declaration source could not bind the retained PSI outline.",
        ));
        return Ok(());
    };

    let Some(documented_events) = documentation["events"].as_array() else {
        callable.source_outline = Some(unavailable_outline(
            owner_key,
            callable.citation_id.clone(),
            "KOTLIN_SOURCE_OUTLINE_EVENTS_UNAVAILABLE",
            "The retained Kotlin PSI event list is unavailable.",
        ));
        return Ok(());
    };
    if documentation["boundaries"].as_array().is_none() {
        callable.source_outline = Some(unavailable_outline(
            owner_key,
            callable.citation_id.clone(),
            "KOTLIN_SOURCE_OUTLINE_BOUNDARIES_UNAVAILABLE",
            "The retained Kotlin PSI control-boundary list is unavailable.",
        ));
        return Ok(());
    }

    let mut flows = context
        .evidence
        .observations
        .values()
        .filter(|candidate| {
            candidate.kind == "FLOW"
                && candidate.service == owner.service
                && candidate.symbol == owner.symbol
                && candidate.normalized["scope"] == scope
        })
        .cloned()
        .collect::<Vec<_>>();
    flows.sort_by(|left, right| {
        left.normalized["ordinal"]
            .as_u64()
            .unwrap_or(u64::MAX)
            .cmp(&right.normalized["ordinal"].as_u64().unwrap_or(u64::MAX))
            .then_with(|| left.id.cmp(&right.id))
    });
    for flow in &flows {
        context.retain(flow);
    }

    if documented_events.is_empty() || flows.len() != documented_events.len() {
        callable.source_outline = Some(unavailable_outline(
            owner_key,
            callable.citation_id.clone(),
            "KOTLIN_SOURCE_OUTLINE_EVENT_SET_MISMATCH",
            "The retained documentation events and exact-owner FLOW observations do not form one complete ordered set.",
        ));
        return Ok(());
    }

    let mut outline_events = Vec::with_capacity(documented_events.len());
    for (index, documented) in documented_events.iter().enumerate() {
        let ordinal = index as u64;
        let Some(flow) = flows
            .iter()
            .find(|flow| flow.normalized["ordinal"].as_u64() == Some(ordinal))
        else {
            callable.source_outline = Some(unavailable_outline(
                owner_key,
                callable.citation_id.clone(),
                "KOTLIN_SOURCE_OUTLINE_ORDINAL_MISMATCH",
                "A retained documentation event has no unique FLOW observation at its exact ordinal.",
            ));
            return Ok(());
        };
        if flow.normalized["ordinal"].as_u64() != Some(ordinal)
            || flow.normalized["scope"] != scope
            || flow.digest != digest(&flow.normalized)?
            || event_payload(&flow.normalized) != documented_payload(documented)
        {
            callable.source_outline = Some(unavailable_outline(
                owner_key,
                callable.citation_id.clone(),
                "KOTLIN_SOURCE_OUTLINE_EVENT_BINDING_MISMATCH",
                "A retained Kotlin PSI event differs from its exact FLOW ordinal or digest.",
            ));
            return Ok(());
        }

        let Some(kind) = documented["kind"].as_str() else {
            callable.source_outline = Some(unavailable_outline(
                owner_key,
                callable.citation_id.clone(),
                "KOTLIN_SOURCE_OUTLINE_EVENT_KIND_UNAVAILABLE",
                "A retained Kotlin PSI event lacks its kind.",
            ));
            return Ok(());
        };
        let source = match flow.source_ids.as_slice() {
            [source_id] => context
                .evidence
                .sources
                .get(source_id)
                .filter(|source| {
                    source.id == *source_id
                        && source.file == owner_source.file
                        && valid_source(context.evidence, source, Some(&owner_source))
                        && source_lines(&owner_source, source.start_line, source.end_line)
                            .is_some_and(|text| text == source.text)
                })
                .cloned(),
            _ => None,
        };
        let Some(source) = source else {
            callable.source_outline = Some(unavailable_outline(
                owner_key,
                callable.citation_id.clone(),
                "KOTLIN_SOURCE_OUTLINE_EVENT_SOURCE_MISMATCH",
                "A FLOW event source does not match the declaration file, evidence binding, or exact retained line text.",
            ));
            return Ok(());
        };

        let citation_id = context.citation(&source, 0, source.text.len());
        outline_events.push(SourceOutlineEvent {
            observation_id: flow.id.clone(),
            ordinal,
            kind: kind.to_owned(),
            file: source.file.clone(),
            start_line: source.start_line,
            end_line: source.end_line,
            event: documented_payload(documented),
            citation_id,
        });
    }

    let gaps = documentation["boundaries"]
        .as_array()
        .filter(|boundaries| !boundaries.is_empty())
        .map(|boundaries| {
            let detail = boundaries
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            vec![gap(
                "KOTLIN_SOURCE_OUTLINE_CONTROL_BOUNDARY",
                format!("A retained Kotlin PSI control boundary prevents a complete source outline: {detail}"),
                callable.citation_id.clone(),
            )]
        })
        .unwrap_or_default();
    if !gaps.is_empty() {
        callable.source_outline = Some(SourceOutline {
            authority: OUTLINE_AUTHORITY.into(),
            owner_key,
            events: outline_events,
            tree: None,
            gaps,
        });
        return Ok(());
    }

    let (tree, issue) = derive_tree(
        owner.normalized["compilerCallableId"]
            .as_str()
            .unwrap_or(&owner.symbol),
        &outline_events,
    );
    let outline_gaps = issue
        .map(|issue| {
            vec![gap(
                issue.code,
                issue.detail,
                outline_events
                    .get(issue.event_index.unwrap_or(0))
                    .map(|event| event.citation_id.clone())
                    .or_else(|| callable.citation_id.clone()),
            )]
        })
        .unwrap_or_default();
    callable.source_outline = Some(SourceOutline {
        authority: OUTLINE_AUTHORITY.into(),
        owner_key,
        events: outline_events,
        tree,
        gaps: outline_gaps,
    });
    Ok(())
}

fn unavailable_outline(
    owner_key: SourceOutlineOwnerKey,
    citation_id: Option<String>,
    code: &str,
    detail: &str,
) -> SourceOutline {
    SourceOutline {
        authority: OUTLINE_AUTHORITY.into(),
        owner_key,
        events: Vec::new(),
        tree: None,
        gaps: vec![gap(code, detail, citation_id)],
    }
}

fn retained_owner_source<'a>(context: &'a Context<'_>, owner: &Observation) -> Option<&'a Source> {
    match owner.source_ids.as_slice() {
        [id] => context
            .evidence
            .sources
            .get(id)
            .filter(|source| source.id == *id),
        _ => None,
    }
}

fn valid_source(evidence: &ServiceEvidence, source: &Source, owner: Option<&Source>) -> bool {
    let transformed = crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY;
    source.service == evidence.service
        && source.revision == evidence.revision
        && (source.authority == "EXACT_SNAPSHOT_TEXT" || source.authority == transformed)
        && (source.authority != transformed || source.url.is_none())
        && !source.file.is_empty()
        && !source.text.is_empty()
        && source.start_line > 0
        && source.end_line >= source.start_line
        && source.end_line - source.start_line + 1 == source.text.lines().count() as u64
        && source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
        && source.occurrence.as_ref().is_none_or(|occurrence| {
            occurrence.end_byte.checked_sub(occurrence.start_byte) == Some(source.text.len())
        })
        && canonical_sha256(&source.evidence_digest)
        && owner.is_none_or(|owner| {
            source.file == owner.file
                && source.evidence_digest == owner.evidence_digest
                && source.authority == owner.authority
                && source.start_line >= owner.start_line
                && source.end_line <= owner.end_line
        })
}

fn canonical_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn source_lines(source: &Source, start: u64, end: u64) -> Option<String> {
    if start < source.start_line || end < start || end > source.end_line {
        return None;
    }
    let lines = source.text.lines().collect::<Vec<_>>();
    let first = usize::try_from(start.checked_sub(source.start_line)?).ok()?;
    let last = usize::try_from(end.checked_sub(source.start_line)?).ok()?;
    Some(lines.get(first..=last)?.join("\n"))
}

fn documented_payload(event: &Value) -> Value {
    strip_capture_coordinates(event)
}

fn event_payload(normalized: &Value) -> Value {
    let mut payload = normalized.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("ordinal");
        object.remove("scope");
    }
    payload
}

fn strip_capture_coordinates(value: &Value) -> Value {
    const COORDINATES: &[&str] = &[
        "start",
        "end",
        "startLine",
        "endLine",
        "byteStart",
        "byteEnd",
        "line",
        "file",
    ];
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !COORDINATES.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), strip_capture_coordinates(value)))
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.iter().map(strip_capture_coordinates).collect())
        }
        value => value.clone(),
    }
}

struct OutlineIssue {
    code: &'static str,
    detail: String,
    event_index: Option<usize>,
}

fn derive_tree(
    entry: &str,
    events: &[SourceOutlineEvent],
) -> (Option<String>, Option<OutlineIssue>) {
    let mut steps = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        let issue = |code, detail| OutlineIssue {
            code,
            detail,
            event_index: Some(index),
        };
        match event.kind.as_str() {
            "IF" => {
                let Some(condition) = event.event["condition"].as_str().filter(|s| !s.is_empty())
                else {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_IF_CONDITION_UNAVAILABLE",
                            "A Kotlin PSI IF event lacks condition metadata for its structural outline.".into(),
                        )),
                    );
                };
                if event.event.get("subject").is_some() {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_WHEN_UNSUPPORTED",
                            "Subject-based Kotlin when structure is outside the admitted outline forms.".into(),
                        )),
                    );
                }
                steps.push(crate::documentation::process_flow::ProjectionStep::If(
                    format!("PSI condition metadata: {condition}"),
                ));
            }
            "ELSE" => {
                if event.event.get("condition").is_some() || event.event.get("subject").is_some() {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_CONDITIONED_ELSE_UNSUPPORTED",
                            "Conditioned Kotlin else or when structure is outside the admitted outline forms.".into(),
                        )),
                    );
                }
                steps.push(crate::documentation::process_flow::ProjectionStep::Else);
            }
            "CALL" | "CONSTRUCT" => {
                let Some(target) = event.event["target"].as_str().filter(|s| !s.is_empty()) else {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_CALL_TARGET_UNAVAILABLE",
                            "A Kotlin call event lacks a retained compiler target identity.".into(),
                        )),
                    );
                };
                if event.event["resolution"] != "COMPILER_EXACT" {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_CALL_TARGET_UNAVAILABLE",
                            "A Kotlin call event is not bound to an exact K2 target.".into(),
                        )),
                    );
                }
                let label = format!("{} target metadata: {target}", event.kind);
                steps.push(crate::documentation::process_flow::ProjectionStep::Action {
                    diagram: label.clone(),
                    tree: label,
                    category: None,
                });
            }
            "RETURN" => steps.push(
                crate::documentation::process_flow::ProjectionStep::MethodReturn(String::new()),
            ),
            "END" => steps.push(crate::documentation::process_flow::ProjectionStep::End),
            "LOCAL" | "STATEMENT" => steps.push(
                crate::documentation::process_flow::ProjectionStep::Gap(format!(
                    "KOTLIN_SOURCE_OUTLINE_{}_EXPRESSION_STRUCTURE_NOT_ESTABLISHED",
                    event.kind
                )),
            ),
            kind => {
                let code = match kind {
                    "BOUNDARY" => "KOTLIN_SOURCE_OUTLINE_CONTROL_BOUNDARY",
                    "LOOP" | "TRY" | "CATCH" | "FINALLY" | "DEFERRED" | "THROW" | "BREAK"
                    | "CONTINUE" => "KOTLIN_SOURCE_OUTLINE_CONTROL_FORM_UNSUPPORTED",
                    _ => "KOTLIN_SOURCE_OUTLINE_EVENT_UNSUPPORTED",
                };
                return (
                    None,
                    Some(issue(
                        code,
                        format!(
                            "Kotlin PSI event kind {kind:?} is outside the admitted outline forms."
                        ),
                    )),
                );
            }
        }
    }

    let projection =
        crate::documentation::process_flow::Projection::source(entry, entry.to_owned(), steps);
    let validated = crate::documentation::process_flow::validate_projection(projection);
    if let [crate::documentation::process_flow::ProjectionStep::Gap(reason)] =
        validated.projection.steps.as_slice()
        && let Some(reason) = reason.strip_prefix("MALFORMED_CONTROL:")
    {
        return (
            None,
            Some(OutlineIssue {
                code: "KOTLIN_SOURCE_OUTLINE_MALFORMED_STRUCTURE",
                detail: format!(
                    "The retained Kotlin PSI branch structure is incomplete or malformed: {reason}."
                ),
                event_index: events.len().checked_sub(1),
            }),
        );
    }
    (
        Some(crate::documentation::process_flow::render_tree(
            entry,
            &validated.projection.steps,
        )),
        None,
    )
}

fn validate_source_outline(
    service: &str,
    revision: &str,
    observations: &std::collections::BTreeMap<String, Observation>,
    sources: &std::collections::BTreeMap<String, Source>,
    citations: &std::collections::BTreeMap<String, Citation>,
    callable: &CallableProjection,
) -> Result<(), ClewError> {
    let Some(outline) = &callable.source_outline else {
        return Ok(());
    };
    let owner = observations
        .get(&callable.declaration_id)
        .ok_or_else(|| invalid("Kotlin source-outline owner is not retained"))?;
    if owner.id != callable.declaration_id
        || owner.kind != "SYMBOL"
        || owner.service != service
        || owner.symbol != callable.symbol
        || owner.digest != digest(&owner.normalized)?
        || owner.normalized["documentation"]["schema"] != DOCUMENTATION_SCHEMA
        || owner.normalized["documentation"]["authority"] != OUTLINE_AUTHORITY
        || owner.normalized["scope"] != outline.owner_key.scope
    {
        return Err(invalid(
            "Kotlin source-outline owner does not match its admitted declaration",
        ));
    }
    if outline.authority != OUTLINE_AUTHORITY
        || outline.owner_key.service != service
        || outline.owner_key.scope != owner.normalized["scope"]
        || outline.owner_key.symbol != callable.symbol
    {
        return Err(invalid(
            "Kotlin source-outline owner key or authority is inconsistent",
        ));
    }
    let owner_source = match owner.source_ids.as_slice() {
        [id] => sources.get(id).filter(|source| source.id == *id),
        _ => None,
    };
    let documentation = &owner.normalized["documentation"];
    let owner_source_valid = owner_source.is_some_and(|source| {
        source.service == service
            && source.revision == revision
            && valid_source_binding(source, None)
    });

    // An unavailable outline makes no event-order claim and therefore carries
    // no partial event list. It still has one named marker anchored to the
    // already-validated declaration citation.
    if outline.events.is_empty() && outline.tree.is_none() && !outline.gaps.is_empty() {
        if outline.gaps.len() != 1
            || !outline.gaps[0].code.starts_with("KOTLIN_SOURCE_OUTLINE_")
            || !outline.gaps[0].detail.starts_with("The retained ")
                && !outline.gaps[0].detail.starts_with("A retained ")
                && !outline.gaps[0].detail.starts_with("A FLOW ")
                && !outline.gaps[0].detail.starts_with("The selected ")
            || outline.gaps[0].citation_id.as_deref().is_none_or(|id| {
                !citations.contains_key(id) || Some(id) != callable.citation_id.as_deref()
            })
        {
            return Err(invalid(
                "Kotlin unavailable source-outline marker is malformed",
            ));
        }
        let gap_code = outline.gaps[0].code.as_str();
        let doc_events = documentation["events"].as_array();
        let boundaries = documentation["boundaries"].as_array();
        let unavailable_is_supported = match gap_code {
            "KOTLIN_SOURCE_OUTLINE_OWNER_SOURCE_UNAVAILABLE" => !owner_source_valid,
            "KOTLIN_SOURCE_OUTLINE_EVENTS_UNAVAILABLE" => {
                owner_source_valid && doc_events.is_none()
            }
            "KOTLIN_SOURCE_OUTLINE_BOUNDARIES_UNAVAILABLE" => {
                owner_source_valid && doc_events.is_some() && boundaries.is_none()
            }
            _ => {
                owner_source_valid
                    && doc_events.is_some()
                    && !complete_event_bindings(
                        service,
                        revision,
                        outline.owner_key.scope.as_str(),
                        callable.symbol.as_str(),
                        doc_events.unwrap(),
                        owner_source.unwrap(),
                        observations,
                        sources,
                    )
            }
        };
        if !unavailable_is_supported {
            return Err(invalid(
                "Kotlin unavailable source-outline marker does not match the retained contract shape",
            ));
        }
        return Ok(());
    }
    let owner_source = owner_source
        .filter(|source| {
            owner_source_valid && source.service == service && source.revision == revision
        })
        .ok_or_else(|| invalid("Kotlin source-outline declaration source is not retained"))?;
    let doc_events = documentation["events"]
        .as_array()
        .ok_or_else(|| invalid("Kotlin source-outline event list is not retained"))?;
    if documentation["boundaries"].as_array().is_none() {
        return Err(invalid(
            "Kotlin source-outline boundary list is not retained",
        ));
    }
    if outline.events.len() != doc_events.len() || outline.events.is_empty() {
        return Err(invalid(
            "Kotlin source-outline has a partial retained event list",
        ));
    }

    let mut flows = observations
        .values()
        .filter(|observation| {
            observation.kind == "FLOW"
                && observation.service == service
                && observation.symbol == callable.symbol
                && observation.normalized["scope"] == outline.owner_key.scope
        })
        .collect::<Vec<_>>();
    flows.sort_by(|left, right| {
        left.normalized["ordinal"]
            .as_u64()
            .unwrap_or(u64::MAX)
            .cmp(&right.normalized["ordinal"].as_u64().unwrap_or(u64::MAX))
            .then_with(|| left.id.cmp(&right.id))
    });
    if flows.len() != doc_events.len() {
        return Err(invalid(
            "Kotlin source-outline FLOW set is missing, duplicated, or has extras",
        ));
    }

    for (index, (outline_event, documented)) in outline.events.iter().zip(doc_events).enumerate() {
        let ordinal = index as u64;
        let flow = flows
            .iter()
            .copied()
            .find(|flow| flow.normalized["ordinal"].as_u64() == Some(ordinal))
            .ok_or_else(|| invalid("Kotlin source-outline FLOW ordinal is not unique"))?;
        if outline_event.observation_id != flow.id
            || outline_event.ordinal != ordinal
            || outline_event.kind != documented["kind"].as_str().unwrap_or_default()
            || outline_event.event != documented_payload(documented)
            || flow.service != service
            || flow.symbol != callable.symbol
            || flow.digest != digest(&flow.normalized)?
            || flow.normalized["scope"] != outline.owner_key.scope
            || event_payload(&flow.normalized) != outline_event.event
        {
            return Err(invalid(
                "Kotlin source-outline event differs from its retained FLOW binding",
            ));
        }
        let source = match flow.source_ids.as_slice() {
            [id] => sources.get(id).filter(|source| source.id == *id),
            _ => None,
        }
        .filter(|source| {
            source.file == outline_event.file
                && source.start_line == outline_event.start_line
                && source.end_line == outline_event.end_line
                && source.service == service
                && source.revision == revision
                && valid_source_binding(source, Some(owner_source))
                && source_lines(owner_source, source.start_line, source.end_line)
                    .is_some_and(|text| text == source.text)
        })
        .ok_or_else(|| {
            invalid("Kotlin source-outline event Source is not bound to retained lines")
        })?;
        validate_citation(
            outline_event,
            source,
            citations
                .get(&outline_event.citation_id)
                .filter(|citation| citation.id == outline_event.citation_id)
                .ok_or_else(|| invalid("Kotlin source-outline citation is not retained"))?,
        )?;
    }

    let boundaries = documentation["boundaries"].as_array().unwrap();
    if !boundaries.is_empty() {
        if outline.tree.is_some()
            || outline.gaps.len() != 1
            || outline.gaps[0].code != "KOTLIN_SOURCE_OUTLINE_CONTROL_BOUNDARY"
            || outline.gaps[0].citation_id.as_ref() != callable.citation_id.as_ref()
        {
            return Err(invalid(
                "Kotlin control boundary did not veto the whole source outline",
            ));
        }
        return Ok(());
    }
    let entry = owner.normalized["compilerCallableId"]
        .as_str()
        .unwrap_or(&owner.symbol);
    let (derived_tree, issue) = derive_tree(entry, &outline.events);
    match issue {
        None if outline.tree.as_deref() == derived_tree.as_deref() && outline.gaps.is_empty() => {
            Ok(())
        }
        Some(issue)
            if outline.tree.is_none()
                && outline.gaps.len() == 1
                && outline.gaps[0].code == issue.code
                && outline.gaps[0]
                    .citation_id
                    .as_deref()
                    .is_some_and(|id| citations.contains_key(id)) =>
        {
            Ok(())
        }
        _ => Err(invalid(
            "Kotlin source-outline tree or unavailable marker differs from its retained events",
        )),
    }
}

fn valid_source_binding(source: &Source, owner: Option<&Source>) -> bool {
    let transformed = crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY;
    (source.authority == "EXACT_SNAPSHOT_TEXT" || source.authority == transformed)
        && (source.authority != transformed || source.url.is_none())
        && !source.file.is_empty()
        && !source.text.is_empty()
        && source.start_line > 0
        && source.end_line >= source.start_line
        && source.end_line - source.start_line + 1 == source.text.lines().count() as u64
        && source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
        && source.occurrence.as_ref().is_none_or(|occurrence| {
            occurrence.end_byte.checked_sub(occurrence.start_byte) == Some(source.text.len())
        })
        && canonical_sha256(&source.evidence_digest)
        && owner.is_none_or(|owner| {
            source.service == owner.service
                && source.revision == owner.revision
                && source.file == owner.file
                && source.authority == owner.authority
                && source.evidence_digest == owner.evidence_digest
                && source.start_line >= owner.start_line
                && source.end_line <= owner.end_line
        })
}

fn validate_citation(
    event: &SourceOutlineEvent,
    source: &Source,
    citation: &Citation,
) -> Result<(), ClewError> {
    let start = 0usize;
    let end = source.text.len();
    let expected_id = format!(
        "citation-{}",
        &crate::canonical::hash_bytes(
            format!("{}:{start}:{end}:{}", source.id, source.text_digest).as_bytes()
        )[7..31]
    );
    let start_line = source.start_line;
    let end_line = source.start_line
        + source.text.as_bytes()[..end.saturating_sub(1).max(start)]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count() as u64;
    let expected_url = source.url.as_ref().map(|url| {
        format!(
            "{}#L{start_line}-L{end_line}",
            url.split('#').next().unwrap_or(url)
        )
    });
    if citation.id != expected_id
        || citation.source_id != source.id
        || citation.service != source.service
        || citation.revision != source.revision
        || citation.file != source.file
        || citation.start_line != start_line
        || citation.end_line != end_line
        || citation.start_byte != start
        || citation.end_byte != end
        || citation.text_digest != source.text_digest
        || citation.evidence_digest != source.evidence_digest
        || citation.authority != source.authority
        || citation.url != expected_url
        || event.file != source.file
        || event.start_line != source.start_line
        || event.end_line != source.end_line
    {
        return Err(invalid(
            "Kotlin source-outline citation does not cover its exact retained line text",
        ));
    }
    Ok(())
}

fn complete_event_bindings(
    service: &str,
    revision: &str,
    scope: &str,
    symbol: &str,
    documented_events: &[Value],
    owner_source: &Source,
    observations: &std::collections::BTreeMap<String, Observation>,
    sources: &std::collections::BTreeMap<String, Source>,
) -> bool {
    if documented_events.is_empty()
        || documented_events
            .iter()
            .any(|event| event.get("kind").and_then(Value::as_str).is_none())
    {
        return false;
    }
    let flows = observations
        .values()
        .filter(|flow| {
            flow.kind == "FLOW"
                && flow.service == service
                && flow.symbol == symbol
                && flow.normalized["scope"] == scope
        })
        .collect::<Vec<_>>();
    if flows.len() != documented_events.len() {
        return false;
    }
    for (index, documented) in documented_events.iter().enumerate() {
        let ordinal = index as u64;
        let matches = flows
            .iter()
            .filter(|flow| flow.normalized["ordinal"].as_u64() == Some(ordinal))
            .collect::<Vec<_>>();
        let [flow] = matches.as_slice() else {
            return false;
        };
        if flow.digest != digest(&flow.normalized).unwrap_or_default()
            || event_payload(&flow.normalized) != documented_payload(documented)
        {
            return false;
        }
        let source = match flow.source_ids.as_slice() {
            [id] => sources.get(id).filter(|source| source.id == *id),
            _ => None,
        };
        let Some(source) = source.filter(|source| {
            source.service == service
                && source.revision == revision
                && source.file == owner_source.file
                && valid_source_binding(source, Some(owner_source))
                && source_lines(owner_source, source.start_line, source.end_line)
                    .is_some_and(|text| text == source.text)
        }) else {
            return false;
        };
        if !valid_source_binding(source, Some(owner_source)) {
            return false;
        }
    }
    true
}

pub(in crate::documentation::static_pages) fn validate_page(
    page: &super::super::model::PageContent,
) -> Result<(), ClewError> {
    for callable in std::iter::once(&page.endpoint)
        .chain(std::iter::once(&page.worker))
        .chain(page.wiring.iter())
    {
        validate_source_outline(
            &page.selection.service,
            &page.service_revision,
            &page.observations,
            &page.sources,
            &page.citations,
            callable,
        )?;
    }
    Ok(())
}
