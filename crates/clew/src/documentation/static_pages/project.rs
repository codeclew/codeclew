//! Deterministic projection of selected source declarations, not an operation answer.
use super::{
    model::*,
    source::{self, Context, all_steps, gap, java_handoff},
};
use crate::documentation::{
    check::Check, digest, invalid, notes::Association, store::RepositoryInputs,
};
use crate::error::ClewError;
use std::collections::{BTreeMap, BTreeSet};

pub fn project(checked: &Check, selections: &[Selection]) -> Result<BundleProjection, ClewError> {
    if selections.iter().any(|s| !s.authored_paragraphs.is_empty()) {
        return Err(invalid(
            "frozen authored paragraphs require repository-aware docs pages render",
        ));
    }
    project_unresolved(checked, selections)
}
pub(super) fn project_unresolved(
    checked: &Check,
    selections: &[Selection],
) -> Result<BundleProjection, ClewError> {
    let mut projection = project_declarations(checked, selections)?;
    attach(checked, &mut projection)?;
    Ok(projection)
}

pub(super) fn project_with_policy(
    checked: &Check,
    selections: &[Selection],
    policy: &crate::documentation::endpoint_publication::Policy,
) -> Result<BundleProjection, ClewError> {
    // Validate every requested declaration and captured note against this Check
    // before exclusions can hide an invalid selection.
    let mut projection = project_declarations(checked, selections)?;
    let mut names = BTreeSet::new();
    for page in &projection.pages {
        if !crate::documentation::store::valid_id(&page.id)
            || !names.insert(page.id.to_ascii_lowercase())
        {
            return Err(invalid(
                "native page IDs must be safe and unique ignoring ASCII case",
            ));
        }
    }
    if !policy.exclusions.is_empty() {
        let selectors: BTreeMap<_, _> = selections
            .iter()
            .map(|selection| selection.service.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|service| {
                (
                    service,
                    crate::documentation::endpoint_publication::selectors_for_declarations(
                        &checked.services[service],
                    ),
                )
            })
            .collect();
        let mut retained = Vec::new();
        for page in projection.pages {
            let excluded = selectors[page.selection.service.as_str()]
                .get(&page.selection.endpoint_declaration)
                .is_some_and(|selector| policy.excludes(selector));
            // Native syntax projections can lack an admitted publication scope.
            // Only a resolved exact identity may match an explicit exclusion.
            if !excluded {
                retained.push(page);
            }
        }
        projection.requested_selection_digest = Some(projection.selection_digest.clone());
        projection.endpoint_publication_policy_digest = Some(policy.digest()?);
        projection.selection_digest = digest(
            &retained
                .iter()
                .map(|page| &page.selection)
                .collect::<Vec<_>>(),
        )?;
        projection.pages = retained;
        projection.schema = base_projection_shape(&mut projection.pages).into();
    }
    // Linked process navigation and reverse memberships see only effective pages;
    // the immutable Check still supplies excluded callable bodies as callees.
    attach(checked, &mut projection)?;
    Ok(projection)
}

fn attach(checked: &Check, projection: &mut BundleProjection) -> Result<(), ClewError> {
    super::linked::attach(checked, projection)?;
    super::data_state::attach(checked, projection)?;
    if uses_source_invocations(projection) {
        projection.schema = SOURCE_INVOCATION_SCHEMA.into();
    }
    Ok(())
}

fn project_declarations(
    checked: &Check,
    selections: &[Selection],
) -> Result<BundleProjection, ClewError> {
    let note_inputs = if selections
        .iter()
        .any(|selection| !selection.note_ids.is_empty())
    {
        Some(pinned_note_inputs(checked)?)
    } else {
        None
    };
    let mut ids = BTreeSet::new();
    let mut pages = Vec::new();
    for selection in selections {
        if selection.id.is_empty() || !ids.insert(&selection.id) {
            return Err(invalid(
                "native page selection IDs must be nonempty and unique",
            ));
        }
        let human_instructions = human_instructions(note_inputs, selection)?;
        let selected_evidence = checked.services.get(&selection.service);
        let selected_exact_calls = [
            Some(selection.endpoint_declaration.as_str()),
            Some(selection.worker_declaration.as_str()),
            selection.wiring_declaration.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|id| {
            selected_evidence
                .and_then(|evidence| evidence.observations.get(id))
                .is_some_and(source::has_exact_call_capability)
        });
        if selection.expand_data_state && !selection.expand_source_calls && !selected_exact_calls {
            return Err(invalid("expandDataState requires expandSourceCalls"));
        }
        let evidence = selected_evidence
            .ok_or_else(|| invalid("native page selected service is not retained in Check"))?;
        let mut ctx = Context::new(evidence);
        let endpoint = source::project_callable(&mut ctx, &selection.endpoint_declaration)?;
        let worker = source::project_callable(&mut ctx, &selection.worker_declaration)?;
        let wiring = selection
            .wiring_declaration
            .as_ref()
            .map(|id| source::project_callable(&mut ctx, id))
            .transpose()?;
        let declaration_only = [Some(&endpoint), Some(&worker), wiring.as_ref()]
            .into_iter()
            .flatten()
            .any(|callable| callable.kind == ProjectionKind::DeclarationOnly);
        let compiler_control_flow = [Some(&endpoint), Some(&worker), wiring.as_ref()]
            .into_iter()
            .flatten()
            .any(|callable| callable.kind == ProjectionKind::CompilerControlFlow);
        let declaration_view = declaration_only || compiler_control_flow;
        if declaration_view && selection.expand_source_calls && !selected_exact_calls {
            return Err(invalid(
                "expandSourceCalls on declaration-only pages requires an admitted compiler exact-call capability",
            ));
        }
        let selected_data_languages = [Some(&endpoint), Some(&worker), wiring.as_ref()]
            .into_iter()
            .flatten()
            .all(|callable| {
                evidence
                    .observations
                    .get(&callable.projection.declaration_id)
                    .is_some_and(super::data_state::supported_language)
            });
        if declaration_view && selection.expand_data_state && !selected_data_languages {
            return Err(invalid(
                "expandDataState is unavailable for declaration-only and compiler-control-flow pages",
            ));
        }
        if selection.expand_data_state && !selection.expand_source_calls {
            return Err(invalid("expandDataState requires expandSourceCalls"));
        }
        let kotlin_wiring = wiring.is_some()
            && [Some(&endpoint), Some(&worker), wiring.as_ref()]
                .into_iter()
                .flatten()
                .all(|c| {
                    let o = &evidence.observations[&c.projection.declaration_id];
                    source::compiler::kotlin_admitted(o)
                        && o.normalized["documentation"]["dataInput"].is_object()
                });
        let handoff = if declaration_view && !kotlin_wiring {
            declaration_only_handoff()
        } else {
            java_handoff(
                &mut ctx,
                &endpoint.projection,
                &worker.projection,
                wiring.as_ref().map(|w| &w.projection),
            )
        };
        let diagnostics = if declaration_view {
            vec![]
        } else {
            diagnostics(&worker.projection)
        };
        let mut limitations = vec![gap(
            "RUNTIME_NOT_OBSERVED",
            "Source declarations do not establish deployed activation, delivery, external success, or durable completion.",
            None,
        )];
        for boundary in &evidence.boundaries {
            limitations.push(gap("RETAINED_SERVICE_BOUNDARY", boundary, None));
        }
        if declaration_view && !kotlin_wiring {
            limitations.push(gap(
                "DECLARATION_ONLY_NO_RELATIONSHIP",
                "Selected declarations are retained without inferring behavior or a relationship between them.",
                None,
            ));
        }
        let non_callable = [
            &selection.endpoint_declaration,
            &selection.worker_declaration,
        ]
        .into_iter()
        .any(|id| {
            evidence
                .observations
                .get(id)
                .and_then(|row| row.normalized["declarationKind"].as_str())
                .is_some_and(|kind| {
                    !matches!(
                        kind,
                        "FUNCTION"
                            | "METHOD"
                            | "CONSTRUCTOR"
                            | "function"
                            | "impl-method"
                            | "trait-method"
                    )
                })
        });
        let declaration_label = if non_callable {
            "Selected declarations"
        } else {
            "Selected functions"
        };
        let title = if declaration_view
            && endpoint.projection.declaration_id == worker.projection.declaration_id
        {
            format!("{declaration_label}: {}", endpoint.projection.symbol)
        } else if declaration_view {
            format!(
                "{declaration_label}: {} · {}",
                endpoint.projection.symbol, worker.projection.symbol
            )
        } else {
            format!(
                "{} → {}",
                endpoint.projection.symbol, worker.projection.symbol
            )
        };
        pages.push(PageContent {
            id: selection.id.clone(),
            title,
            projection_kind: Some(if compiler_control_flow {
                ProjectionKind::CompilerControlFlow
            } else if declaration_only {
                ProjectionKind::DeclarationOnly
            } else {
                ProjectionKind::SourceBehavior
            }),
            selection: selection.clone(),
            service_revision: evidence.revision.clone(),
            service_digest: evidence.service_digest.clone(),
            endpoint: endpoint.projection,
            worker: worker.projection,
            wiring: wiring.map(|w| w.projection),
            handoff,
            diagnostics,
            citations: ctx.citations,
            observations: ctx.observations,
            sources: ctx.sources,
            limitations,
            human_instructions,
            authored_paragraphs: vec![],
            examined_sources: None,
            data_state: None,
        });
    }
    let schema = base_projection_shape(&mut pages);
    Ok(BundleProjection {
        schema: schema.into(),
        input_digest: checked.input_digest.clone(),
        context_digest: checked.context_digest.clone(),
        selection_digest: digest(&selections)?,
        endpoint_publication_policy_digest: None,
        requested_selection_digest: None,
        pages,
        source_call_graph: None,
    })
}

fn base_projection_shape(pages: &mut [PageContent]) -> &'static str {
    let declaration_view = pages.iter().any(|page| {
        page.projection_kind
            .is_some_and(ProjectionKind::is_declaration_view)
    });
    let compiler_control_flow = pages
        .iter()
        .any(|page| page.projection_kind == Some(ProjectionKind::CompilerControlFlow));
    for page in pages {
        if declaration_view {
            page.projection_kind
                .get_or_insert(ProjectionKind::SourceBehavior);
        } else {
            page.projection_kind = None;
        }
    }
    if compiler_control_flow {
        CONTROL_FLOW_SCHEMA
    } else if declaration_view {
        DECLARATION_SCHEMA
    } else {
        SCHEMA
    }
}

pub(super) fn uses_source_invocations(projection: &BundleProjection) -> bool {
    projection.pages.iter().any(|page| {
        [
            Some(&page.endpoint),
            Some(&page.worker),
            page.wiring.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|callable| source::call_sites::uses_source_events(callable, &page.observations))
    }) || projection.source_call_graph.as_ref().is_some_and(|graph| {
        graph
            .nodes
            .values()
            .any(|node| source::call_sites::uses_source_events(&node.callable, &node.observations))
    })
}

fn declaration_only_handoff() -> HandoffProjection {
    HandoffProjection {
        status: "DECLARATION_ONLY".into(),
        queue_allocation: None,
        endpoint_field: None,
        worker_field: None,
        citation_ids: vec![],
        gaps: vec![gap(
            "DECLARATION_ONLY_NO_RELATIONSHIP",
            "Selected declarations do not establish a call, wiring, or runtime relationship.",
            None,
        )],
        limitation: "Declaration-only pages retain compiler declarations without proving source behavior, shared-object wiring, activation, delivery, or completion.".into(),
    }
}

fn note_target(inputs: &RepositoryInputs, target: &str, service: &str) -> (bool, bool) {
    if let Some(target) = target.strip_prefix("service:") {
        let (id, section) = target
            .split_once('/')
            .map(|(id, section)| (id, Some(section)))
            .unwrap_or((target, None));
        let exists = inputs.services.contains_key(id)
            && section.is_none_or(crate::documentation::sections::contains);
        return (exists, exists && id == service);
    }
    (false, false)
}

fn pinned_note_inputs(checked: &Check) -> Result<&RepositoryInputs, ClewError> {
    let pinned = checked
        .source_inputs
        .as_ref()
        .ok_or_else(|| invalid("native page notes require pinned captured source inputs"))?;
    crate::documentation::source_inputs::validate(pinned)?;
    if pinned.input_digest != checked.input_digest {
        return Err(invalid(
            "native page note input identity does not match Check",
        ));
    }
    Ok(&pinned.inputs)
}

fn human_instructions(
    inputs: Option<&RepositoryInputs>,
    selection: &Selection,
) -> Result<Vec<HumanInstruction>, ClewError> {
    if selection.note_ids.is_empty() {
        return Ok(Vec::new());
    }
    if selection.note_ids.len() > 128 {
        return Err(invalid("native page note selection exceeds 128 IDs"));
    }
    let inputs =
        inputs.ok_or_else(|| invalid("native page notes require pinned captured source inputs"))?;
    let mut ids = BTreeSet::new();
    selection.note_ids.iter().map(|id| {
        if !crate::documentation::store::valid_id(id) || !ids.insert(id) {
            return Err(invalid("native page note IDs must be valid and unique"));
        }
        let captured = inputs.notes.get(id)
            .ok_or_else(|| invalid("native page selected note is not captured in Check"))?;
        let association: Association = serde_json::from_value(captured["association"].clone())
            .map_err(|_| invalid("native page captured note association is invalid"))?;
        let association_digest = digest(&association)?;
        if association.id != *id
            || association.schema != "codeclew-documentation-note-association/1.0"
            || captured["associationDigest"] != association_digest
            || captured["authority"] != "HUMAN_OR_IMPORTED_UNVERIFIED"
        {
            return Err(invalid("native page captured note identity or association digest is inconsistent"));
        }
        let author = association.metadata.get("author").and_then(serde_json::Value::as_str)
            .filter(|author| !author.trim().is_empty() && author.len() <= 512)
            .ok_or_else(|| invalid("native page selected note requires a nonblank metadata.author string of at most 512 bytes"))?;
        let targets: Vec<_> = association.targets.iter()
            .map(|target| note_target(inputs, target, &selection.service)).collect();
        if targets.is_empty() || targets.iter().any(|(exists, _)| !exists) {
            return Err(invalid("native page selected note has unavailable or unsupported captured targets"));
        }
        if !targets.iter().any(|(_, related)| *related) {
            return Err(invalid("native page selected note is unrelated to the selected service"));
        }
        let text = captured["original"]["text"].as_str()
            .filter(|text| text.len() <= 256 * 1024)
            .ok_or_else(|| invalid("native page selected note original text is unavailable"))?;
        let content_digest = digest(&text)?;
        if captured["original"]["status"] != "CAPTURED"
            || captured["original"]["digest"] != content_digest
        {
            return Err(invalid("native page selected note original capture or digest is inconsistent"));
        }
        Ok(HumanInstruction {
            id: id.clone(),
            title: association.title.clone(),
            declared_author: author.into(),
            classification: association.classification.clone(),
            period: association.period.clone(),
            version_digest: digest(&(&association_digest, &content_digest))?,
            content_digest,
            association_digest,
            text: text.into(),
            authority: "HUMAN_OR_IMPORTED_UNVERIFIED".into(),
            source_claim_status: "UNASSESSED".into(),
            association: captured["association"].clone(),
        })
    }).collect()
}

fn diagnostics(worker: &CallableProjection) -> Vec<Diagnostic> {
    let rows = all_steps(&worker.steps);
    let mut result = Vec::new();
    for row in &rows {
        if !row.reachable {
            continue;
        }
        for call in &row.calls {
            // Guard alternatives belong to the selected source occurrence,
            // whether its target is a repository declaration, binary dependency
            // or unresolved source call. This does not inspect helper bodies.
            if call.phase == "CREATION" {
                continue;
            }
            for condition in &row.conditions {
                result.push(Diagnostic { condition: format!("{} is {}", condition.expression, !condition.holds), possible_reason: "This alternative can leave the selected call unreached through an earlier return or throw, or choose a different branch.".into(), inspect: vec![condition.expression.clone(), call.expression.clone()], selected_call: call.target.clone().unwrap_or_else(|| call.expression.clone()), citation_ids: vec![condition.citation_id.clone(), call.citation_id.clone()] });
            }
        }
    }
    result
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
