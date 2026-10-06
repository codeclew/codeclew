//! Deterministic projection of selected Java source, not an operation answer.
use super::{
    model::*,
    source::{self, Context, all_steps, gap, java_handoff},
};
use crate::documentation::{
    check::Check, digest, invalid, notes::Association, store::RepositoryInputs,
};
use crate::error::ClewError;
use std::collections::BTreeSet;

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
        if selection.expand_data_state && !selection.expand_source_calls {
            return Err(invalid("expandDataState requires expandSourceCalls"));
        }
        if selection.id.is_empty() || !ids.insert(&selection.id) {
            return Err(invalid(
                "native page selection IDs must be nonempty and unique",
            ));
        }
        let human_instructions = human_instructions(note_inputs, selection)?;
        let evidence = checked
            .services
            .get(&selection.service)
            .ok_or_else(|| invalid("native page selected service is not retained in Check"))?;
        let mut ctx = Context::new(evidence);
        let endpoint = source::project_java(&mut ctx, &selection.endpoint_declaration)?;
        let worker = source::project_java(&mut ctx, &selection.worker_declaration)?;
        let wiring = selection
            .wiring_declaration
            .as_ref()
            .map(|id| source::project_java(&mut ctx, id))
            .transpose()?;
        let handoff = java_handoff(
            &mut ctx,
            &endpoint.projection,
            &worker.projection,
            wiring.as_ref().map(|w| &w.projection),
        );
        let diagnostics = diagnostics(&worker.projection);
        let mut limitations = vec![gap(
            "RUNTIME_NOT_OBSERVED",
            "Source declarations do not establish deployed activation, delivery, external success, or durable completion.",
            None,
        )];
        for boundary in &evidence.boundaries {
            limitations.push(gap("RETAINED_SERVICE_BOUNDARY", boundary, None));
        }
        pages.push(PageContent {
            id: selection.id.clone(),
            title: format!(
                "{} → {}",
                endpoint.projection.symbol, worker.projection.symbol
            ),
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
    let mut projection = BundleProjection {
        schema: SCHEMA.into(),
        input_digest: checked.input_digest.clone(),
        context_digest: checked.context_digest.clone(),
        selection_digest: digest(&selections)?,
        pages,
        source_call_graph: None,
    };
    super::linked::attach(checked, &mut projection)?;
    super::data_state::attach(checked, &mut projection)?;
    Ok(projection)
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
