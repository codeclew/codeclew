//! Shared cited, non-causal source outlines from admitted compiler producers.
use super::super::model::{
    CallableProjection, Citation, SourceOutline, SourceOutlineEvent, SourceOutlineOwnerKey,
};
use super::{Context, gap};
use crate::documentation::model::{Observation, ServiceEvidence, Source};
use crate::documentation::source_statement::{
    SourceStatement, StructureContract, StructureProducer,
};
use crate::documentation::{digest, invalid};
use crate::error::ClewError;
use serde_json::Value;

#[derive(Clone, Copy)]
pub(super) struct OutlineProducer {
    authority: &'static str,
    kotlin: bool,
    producer_label: &'static str,
    call_navigation: super::compiler::CallNavigation,
}

pub(super) fn producer(owner: &Observation) -> Option<OutlineProducer> {
    let contract = StructureContract::identify(&owner.normalized["documentation"])?;
    if super::compiler::capabilities(owner)?.producer != contract.producer {
        return None;
    }
    match contract.producer {
        StructureProducer::KotlinPsi => Some(OutlineProducer {
            authority: "KOTLIN_PSI_WITH_K2_CALL_TARGETS",
            kotlin: true,
            producer_label: "Kotlin",
            call_navigation: super::compiler::CallNavigation::RetainedCompilerSites,
        }),
        StructureProducer::Roslyn if super::compiler::csharp_admitted(owner) => {
            Some(OutlineProducer {
                authority: "ROSLYN_SOURCE_STRUCTURE",
                kotlin: false,
                producer_label: "Roslyn",
                call_navigation: super::compiler::CallNavigation::RetainedCompilerSites,
            })
        }
        StructureProducer::TypeScript => Some(OutlineProducer {
            authority: "TYPESCRIPT_COMPILER_SOURCE_STRUCTURE",
            kotlin: false,
            producer_label: "TypeScript compiler",
            call_navigation: super::compiler::CallNavigation::RetainedCompilerSites,
        }),
        StructureProducer::RustSyntax => Some(OutlineProducer {
            authority: "RUST_SYN_SOURCE_STRUCTURE",
            kotlin: false,
            producer_label: "Rust syntax",
            call_navigation: super::compiler::CallNavigation::UnresolvedSyntax,
        }),
        _ => None,
    }
}

impl OutlineProducer {
    fn code(self, code: &str) -> String {
        if self.kotlin {
            code.into()
        } else {
            code.replace("KOTLIN_SOURCE_OUTLINE_", "SOURCE_OUTLINE_")
        }
    }
    fn detail(self, detail: impl Into<String>) -> String {
        let detail = detail.into();
        if self.kotlin {
            detail
        } else {
            detail
                .replace("Kotlin PSI", &format!("{} source", self.producer_label))
                .replace("Kotlin", self.producer_label)
                .replace("PSI", "source")
                .replace("K2", self.producer_label)
        }
    }
    fn source_label(self) -> &'static str {
        if self.kotlin { "PSI source" } else { "Source" }
    }
}

fn outline_gap(
    contract: OutlineProducer,
    code: &str,
    detail: impl Into<String>,
    citation: Option<String>,
) -> super::super::model::Gap {
    gap(&contract.code(code), contract.detail(detail), citation)
}

/// Attach an outline only to an already-admitted compiler declaration. Failed
/// outline admission is represented as one cited unavailable marker; it never
/// promotes the declaration into the causal source-behavior projection.
pub(super) fn attach(
    context: &mut Context<'_>,
    owner: &Observation,
    scope: &str,
    callable: &mut CallableProjection,
) -> Result<(), ClewError> {
    let documentation = &owner.normalized["documentation"];
    let Some(contract) = producer(owner) else {
        return Ok(());
    };

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
            contract,
            owner_key,
            callable.citation_id.clone(),
            "KOTLIN_SOURCE_OUTLINE_OWNER_SOURCE_UNAVAILABLE",
            "The selected Kotlin declaration source could not bind the retained PSI outline.",
        ));
        return Ok(());
    };

    let Some(documented_events) = documentation["events"].as_array() else {
        callable.source_outline = Some(unavailable_outline(
            contract,
            owner_key,
            callable.citation_id.clone(),
            "KOTLIN_SOURCE_OUTLINE_EVENTS_UNAVAILABLE",
            "The retained Kotlin PSI event list is unavailable.",
        ));
        return Ok(());
    };
    if documentation["boundaries"].as_array().is_none() {
        callable.source_outline = Some(unavailable_outline(
            contract,
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
            contract,
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
                contract,
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
                contract,
                owner_key,
                callable.citation_id.clone(),
                "KOTLIN_SOURCE_OUTLINE_EVENT_BINDING_MISMATCH",
                "A retained Kotlin PSI event differs from its exact FLOW ordinal or digest.",
            ));
            return Ok(());
        }

        let Some(kind) = documented["kind"].as_str() else {
            callable.source_outline = Some(unavailable_outline(
                contract,
                owner_key,
                callable.citation_id.clone(),
                "KOTLIN_SOURCE_OUTLINE_EVENT_KIND_UNAVAILABLE",
                "A retained Kotlin PSI event lacks its kind.",
            ));
            return Ok(());
        };
        let (source, exact_source, event_gaps) = if documented.get("sourceSpan").is_some() {
            if let Some((source, exact)) = crate::documentation::source_span::retained_event_source(
                owner,
                scope,
                ordinal,
                documented,
                flow,
                &owner_source,
                &context.evidence.sources,
            ) {
                (source.clone(), Some(exact), Vec::new())
            } else {
                (
                    owner_source.clone(),
                    None,
                    vec![outline_gap(
                        contract,
                        "KOTLIN_SOURCE_OUTLINE_EXACT_SPAN_REJECTED",
                        "The optional PSI source span failed its exact owner, scope, ordinal, digest or UTF-8 bounds; the event remains retained without exact expression text.",
                        callable.citation_id.clone(),
                    )],
                )
            }
        } else {
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
                    contract,
                    owner_key,
                    callable.citation_id.clone(),
                    "KOTLIN_SOURCE_OUTLINE_EVENT_SOURCE_MISMATCH",
                    "A FLOW event source does not match the declaration file, evidence binding, or exact retained line text.",
                ));
                return Ok(());
            };
            (source, None, Vec::new())
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
            exact_source,
            gaps: event_gaps,
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
            vec![outline_gap(contract,
                "KOTLIN_SOURCE_OUTLINE_CONTROL_BOUNDARY",
                format!("A retained Kotlin PSI control boundary prevents a complete source outline: {detail}"),
                callable.citation_id.clone(),
            )]
        })
        .unwrap_or_default();
    if !gaps.is_empty() {
        callable.source_outline = Some(SourceOutline {
            authority: contract.authority.into(),
            owner_key,
            events: outline_events,
            tree: None,
            gaps,
        });
        return Ok(());
    }

    let (tree, issue) = derive_tree(
        contract,
        owner.normalized["compilerCallableId"]
            .as_str()
            .unwrap_or(&owner.symbol),
        &outline_events,
    );
    let outline_gaps = issue
        .map(|issue| {
            vec![outline_gap(
                contract,
                &issue.code,
                issue.detail,
                outline_events
                    .get(issue.event_index.unwrap_or(0))
                    .map(|event| event.citation_id.clone())
                    .or_else(|| callable.citation_id.clone()),
            )]
        })
        .unwrap_or_default();
    callable.source_outline = Some(SourceOutline {
        authority: contract.authority.into(),
        owner_key,
        events: outline_events,
        tree,
        gaps: outline_gaps,
    });
    Ok(())
}

fn unavailable_outline(
    contract: OutlineProducer,
    owner_key: SourceOutlineOwnerKey,
    citation_id: Option<String>,
    code: &str,
    detail: &str,
) -> SourceOutline {
    SourceOutline {
        authority: contract.authority.into(),
        owner_key,
        events: Vec::new(),
        tree: None,
        gaps: vec![outline_gap(contract, code, detail, citation_id)],
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

pub(super) fn documented_payload(event: &Value) -> Value {
    strip_capture_coordinates(event)
}

pub(super) fn event_payload(normalized: &Value) -> Value {
    let mut payload = normalized.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("ordinal");
        object.remove("scope");
        object.remove("outlineSite");
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
                .map(|(key, value)| {
                    (
                        key.clone(),
                        if key == "sourceSpan" {
                            value.clone()
                        } else {
                            strip_capture_coordinates(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.iter().map(strip_capture_coordinates).collect())
        }
        value => value.clone(),
    }
}

struct OutlineIssue {
    code: String,
    detail: String,
    event_index: Option<usize>,
}

fn derive_tree(
    contract: OutlineProducer,
    entry: &str,
    events: &[SourceOutlineEvent],
) -> (Option<String>, Option<OutlineIssue>) {
    let mut steps = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        let issue = |code: &str, detail: String| OutlineIssue {
            code: contract.code(code),
            detail: contract.detail(detail),
            event_index: Some(index),
        };
        let statement = SourceStatement::decode(&event.event);
        match statement {
            SourceStatement::If {
                condition,
                subject_present,
            } => {
                let Some(condition) = condition.filter(|s| !s.is_empty()) else {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_IF_CONDITION_UNAVAILABLE",
                            "A Kotlin PSI IF event lacks condition metadata for its structural outline.".into(),
                        )),
                    );
                };
                if subject_present {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_WHEN_UNSUPPORTED",
                            "Subject-based Kotlin when structure is outside the admitted outline forms.".into(),
                        )),
                    );
                }
                steps.push(crate::documentation::process_flow::ProjectionStep::If(
                    format!(
                        "{} condition metadata: {condition}",
                        if contract.kotlin { "PSI" } else { "Source" }
                    ),
                ));
            }
            SourceStatement::Else {
                condition_present,
                subject_present,
                ..
            } => {
                if condition_present || subject_present {
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
            SourceStatement::Invocation(call) => {
                if contract.call_navigation == super::compiler::CallNavigation::UnresolvedSyntax {
                    return (
                        None,
                        Some(issue(
                            "SOURCE_OUTLINE_CALL_AUTHORITY_UNAVAILABLE",
                            "Retained syntax cannot establish an exact compiler call target."
                                .into(),
                        )),
                    );
                }
                let Some(target) = call.target else {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_CALL_TARGET_UNAVAILABLE",
                            "A Kotlin call event lacks a retained compiler target identity.".into(),
                        )),
                    );
                };
                if call.exact_target().is_none() {
                    return (
                        None,
                        Some(issue(
                            "KOTLIN_SOURCE_OUTLINE_CALL_TARGET_UNAVAILABLE",
                            "A Kotlin call event is not bound to an exact K2 target.".into(),
                        )),
                    );
                }
                let label = if let Some(exact) = &event.exact_source {
                    format!(
                        "{} {}: {} · target metadata: {target}",
                        event.kind,
                        contract.source_label(),
                        exact.expression
                    )
                } else {
                    format!("{} target metadata: {target}", event.kind)
                };
                steps.push(crate::documentation::process_flow::ProjectionStep::Action {
                    diagram: label.clone(),
                    tree: label,
                    category: None,
                });
            }
            SourceStatement::Return => steps.push(
                crate::documentation::process_flow::ProjectionStep::MethodReturn(
                    event
                        .exact_source
                        .as_ref()
                        .map(|exact| format!("{}: {}", contract.source_label(), exact.expression))
                        .unwrap_or_default(),
                ),
            ),
            SourceStatement::End => {
                steps.push(crate::documentation::process_flow::ProjectionStep::End)
            }
            SourceStatement::Local | SourceStatement::Expression => {
                if let Some(exact) = &event.exact_source {
                    let label = format!(
                        "{} {}: {}",
                        event.kind,
                        contract.source_label(),
                        exact.expression
                    );
                    steps.push(crate::documentation::process_flow::ProjectionStep::Action {
                        diagram: label.clone(),
                        tree: label,
                        category: None,
                    });
                } else {
                    steps.push(crate::documentation::process_flow::ProjectionStep::Gap(
                        contract.code(&format!(
                            "KOTLIN_SOURCE_OUTLINE_{}_EXPRESSION_STRUCTURE_NOT_ESTABLISHED",
                            event.kind,
                        )),
                    ));
                }
            }
            unsupported => {
                let kind = unsupported.label();
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
                code: contract.code("KOTLIN_SOURCE_OUTLINE_MALFORMED_STRUCTURE"),
                detail: contract.detail(format!(
                    "The retained Kotlin PSI branch structure is incomplete or malformed: {reason}."
                )),
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

/// Rebuild with the same shared engine from the retained producer facts.
/// Validation compares the complete tree, event set, local gaps and citations;
/// no current checkout or reparsing participates in this preflight.
pub(in crate::documentation::static_pages) fn validate_source_outline(
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
        .ok_or_else(|| invalid("Source-outline owner is not retained"))?;
    if owner.id != callable.declaration_id
        || owner.kind != "SYMBOL"
        || owner.service != service
        || owner.symbol != callable.symbol
        || owner.digest != digest(&owner.normalized)?
        || producer(owner).is_none()
        || owner.normalized["scope"] != outline.owner_key.scope
    {
        return Err(invalid(
            "Source-outline owner or retained identities differ from compiler evidence",
        ));
    }
    let owned: std::collections::BTreeMap<_, _> = observations
        .iter()
        .filter(|(_, row)| {
            row.id == owner.id
                || (row.kind == "FLOW"
                    && row.service == service
                    && row.symbol == owner.symbol
                    && row.normalized["scope"] == outline.owner_key.scope)
        })
        .map(|(id, row)| (id.clone(), row.clone()))
        .collect();
    if owned.iter().any(|(id, row)| *id != row.id) {
        return Err(invalid(
            "Source-outline observation key differs from its retained identity",
        ));
    }
    let referenced: std::collections::BTreeSet<_> = owned
        .values()
        .flat_map(|row| row.source_ids.iter())
        .collect();
    let owned_sources = referenced
        .into_iter()
        .filter_map(|id| sources.get(id).map(|source| (id.clone(), source.clone())))
        .collect();
    let evidence = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.into(),
        revision: revision.into(),
        service_digest: String::new(),
        extractor: String::new(),
        runtime_mode: String::new(),
        coverage: String::new(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: owned,
        sources: owned_sources,
        contracts: Default::default(),
    };
    let mut context = Context::new(&evidence);
    let mut expected = callable.clone();
    expected.source_outline = None;
    attach(&mut context, owner, &outline.owner_key.scope, &mut expected)?;
    if expected.source_outline.as_ref() != Some(outline)
        || context
            .citations
            .iter()
            .any(|(id, expected)| citations.get(id) != Some(expected))
    {
        return Err(invalid(
            "Source-outline tree, events, gaps or citations differ from retained producer evidence",
        ));
    }
    if outline.events.is_empty()
        && callable
            .citation_id
            .as_ref()
            .is_none_or(|id| !citations.contains_key(id))
    {
        return Err(invalid(
            "Unavailable source outline requires its retained declaration citation",
        ));
    }
    Ok(())
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
