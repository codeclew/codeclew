//! Exact retained SOURCE syntax, with lexical structure and unresolved calls.
use super::super::model::{
    CallableProjection, ProjectionKind, SourceOutline, SourceOutlineEvent, SourceOutlineOwnerKey,
};
use super::{CallableKey, Context, ProjectedCallable, gap};
use crate::documentation::{
    digest, invalid,
    model::{Observation, SOURCE_EXTRACTOR},
};
use crate::error::ClewError;

pub(super) fn candidate(owner: &Observation) -> bool {
    owner.kind == "SYMBOL"
        && owner.normalized["kind"] == "DECLARATION"
        && owner.normalized["authority"] == "SYNTAX"
        && owner.symbol.starts_with("source:")
}

pub(super) fn project(context: &mut Context<'_>, id: &str) -> Result<ProjectedCallable, ClewError> {
    let owner = context
        .evidence
        .observations
        .get(id)
        .ok_or_else(|| invalid("selected SOURCE declaration is missing"))?
        .clone();
    if !candidate(&owner)
        || context.evidence.extractor != SOURCE_EXTRACTOR
        || context.evidence.runtime_mode != "COMMITTED_SOURCE_NO_BUILD"
        || owner.normalized.get("schema").is_some()
        || !(owner.normalized["scope"].is_null() || owner.normalized["scope"] == "")
        || owner.normalized["symbolIdentity"] != owner.symbol
        || owner.id != id
        || owner.service != context.evidence.service
        || owner.digest != digest(&owner.normalized)?
        || context
            .evidence
            .observations
            .values()
            .filter(|o| o.kind == "SYMBOL" && o.symbol == owner.symbol)
            .count()
            != 1
    {
        return Err(invalid(
            "SOURCE syntax declaration producer, identity or digest is inconsistent",
        ));
    }
    let source = match owner.source_ids.as_slice() {
        [id] => context
            .evidence
            .sources
            .get(id)
            .filter(|s| {
                s.id == *id
                    && context.valid_source(s)
                    && s.occurrence.is_some()
                    && crate::documentation::store::relative(&s.file).is_ok()
            })
            .cloned(),
        _ => None,
    }
    .ok_or_else(|| {
        invalid("SOURCE syntax declaration requires one exact retained source occurrence")
    })?;
    let owner_site = source.occurrence.as_ref().unwrap();
    let events = owner.normalized["documentation"]["events"]
        .as_array()
        .ok_or_else(|| invalid("SOURCE syntax declaration event list is unavailable"))?;
    let mut flows: Vec<_> = context
        .evidence
        .observations
        .values()
        .filter(|o| o.kind == "FLOW" && o.symbol == owner.symbol)
        .cloned()
        .collect();
    flows.sort_by_key(|o| o.normalized["ordinal"].as_u64().unwrap_or(u64::MAX));
    if flows.len() != events.len() {
        return Err(invalid(
            "SOURCE syntax event set differs from retained declaration",
        ));
    }
    // SOURCE capture has no compiler compilation scope. Keep the same empty
    // scope used by endpoint publication selectors instead of inventing one.
    let scope = "";
    let citation = context.citation(&source, 0, source.text.len());
    context.retain(&owner);
    let mut outline_events = Vec::with_capacity(events.len());
    let mut depths = Vec::with_capacity(events.len());
    let mut tree = format!("{} [lexical syntax; not execution order]\n", owner.symbol);
    for (ordinal, (event, flow)) in events.iter().zip(&flows).enumerate() {
        if flow.service != owner.service
            || flow.normalized != *event
            || flow.digest != digest(event)?
            || event["ordinal"].as_u64() != Some(ordinal as u64)
            || event["authority"] != "SYNTAX"
            || event["ordering"] != "LEXICAL_ONLY"
            || event["targetStatus"] != "UNRESOLVED"
        {
            return Err(invalid(
                "SOURCE syntax event identity, ordering or digest is inconsistent",
            ));
        }
        if event.get("parentOrdinal").is_none() {
            return Err(invalid("SOURCE syntax event parent is unavailable"));
        }
        let depth = if event["parentOrdinal"].is_null() {
            0
        } else {
            let parent = event["parentOrdinal"]
                .as_u64()
                .and_then(|p| usize::try_from(p).ok())
                .filter(|p| *p < ordinal)
                .ok_or_else(|| invalid("SOURCE syntax event parent is invalid"))?;
            depths[parent] + 1
        };
        if depth > 256 {
            return Err(invalid(
                "SOURCE syntax event nesting exceeds capture bounds",
            ));
        }
        depths.push(depth);
        let kind = event["kind"]
            .as_str()
            .ok_or_else(|| invalid("SOURCE syntax event kind is unavailable"))?;
        let event_source = match flow.source_ids.as_slice() {
            [id] => context
                .evidence
                .sources
                .get(id)
                .filter(|s| s.id == *id && context.valid_source(s))
                .cloned(),
            _ => None,
        }
        .ok_or_else(|| invalid("SOURCE syntax event source is unavailable"))?;
        let site = event_source
            .occurrence
            .as_ref()
            .ok_or_else(|| invalid("SOURCE syntax event occurrence is unavailable"))?;
        let start = site.start_byte.checked_sub(owner_site.start_byte);
        let end = site.end_byte.checked_sub(owner_site.start_byte);
        if event_source.file != source.file
            || site.snapshot != owner_site.snapshot
            || site.blob != owner_site.blob
            || start.zip(end).and_then(|(a, b)| source.text.get(a..b))
                != Some(event_source.text.as_str())
            || event["text"].as_str() != Some(event_source.text.as_str())
        {
            return Err(invalid(
                "SOURCE syntax event bytes do not match the selected declaration",
            ));
        }
        let event_citation = context.citation(&event_source, 0, event_source.text.len());
        context.retain(flow);
        tree.push_str(&format!(
            "{}{} {}\n",
            "  ".repeat(depth + 1),
            kind,
            event_source.text
        ));
        outline_events.push(SourceOutlineEvent {
            observation_id: flow.id.clone(),
            ordinal: ordinal as u64,
            kind: kind.into(),
            file: event_source.file,
            start_line: event_source.start_line,
            end_line: event_source.end_line,
            event: event.clone(),
            citation_id: event_citation,
            exact_source: None,
            gaps: vec![],
        });
    }
    let gaps = vec![gap(
        "SOURCE_SYNTAX_NOT_COMPILER_OR_RUNTIME_PROOF",
        "Lexical events retain branches, calls and deferred bodies. Call targets, execution order, reachability, receiver identity and runtime effects remain unresolved.",
        Some(citation.clone()),
    )];
    Ok(ProjectedCallable {
        key: CallableKey {
            service: owner.service.clone(),
            scope: scope.into(),
            symbol: owner.symbol.clone(),
        },
        kind: ProjectionKind::DeclarationOnly,
        projection: CallableProjection {
            declaration_id: owner.id,
            symbol: owner.symbol,
            authority: "SYNTAX_DECLARATION".into(),
            citation_id: Some(citation),
            control_flow: None,
            source_outline: Some(SourceOutline {
                authority: "SYNTAX_SOURCE_STRUCTURE".into(),
                owner_key: SourceOutlineOwnerKey {
                    service: owner.service,
                    scope: scope.into(),
                    symbol: source_symbol(&owner.normalized),
                },
                events: outline_events,
                tree: Some(tree),
                gaps: gaps.clone(),
            }),
            retained_call_sites: None,
            steps: vec![],
            state: vec![],
            gaps,
        },
    })
}

fn source_symbol(normalized: &serde_json::Value) -> String {
    normalized["symbolIdentity"]
        .as_str()
        .unwrap_or_default()
        .into()
}

pub(in crate::documentation::static_pages) fn validate(
    service: &str,
    revision: &str,
    observations: &std::collections::BTreeMap<String, Observation>,
    sources: &std::collections::BTreeMap<String, crate::documentation::model::Source>,
    citations: &std::collections::BTreeMap<String, super::super::model::Citation>,
    callable: &CallableProjection,
) -> Result<(), ClewError> {
    let owner = observations
        .get(&callable.declaration_id)
        .ok_or_else(|| invalid("SOURCE syntax outline owner is not retained"))?;
    let owned: std::collections::BTreeMap<_, _> = observations
        .iter()
        .filter(|(_, row)| {
            row.symbol == owner.symbol && matches!(row.kind.as_str(), "SYMBOL" | "FLOW")
        })
        .map(|(id, row)| (id.clone(), row.clone()))
        .collect();
    if owned.iter().any(|(id, row)| *id != row.id) {
        return Err(invalid(
            "SOURCE syntax observation key differs from retained identity",
        ));
    }
    let referenced: std::collections::BTreeSet<_> =
        owned.values().flat_map(|o| o.source_ids.iter()).collect();
    let owned_sources = referenced
        .into_iter()
        .filter_map(|id| sources.get(id).map(|s| (id.clone(), s.clone())))
        .collect();
    let evidence = crate::documentation::model::ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.into(),
        revision: revision.into(),
        service_digest: String::new(),
        extractor: SOURCE_EXTRACTOR.into(),
        runtime_mode: "COMMITTED_SOURCE_NO_BUILD".into(),
        coverage: "SYNTAX".into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: owned,
        sources: owned_sources,
        contracts: Default::default(),
    };
    let mut context = Context::new(&evidence);
    let expected = project(&mut context, &callable.declaration_id)?;
    if &expected.projection != callable
        || context
            .citations
            .iter()
            .any(|(id, expected)| citations.get(id) != Some(expected))
    {
        return Err(invalid(
            "SOURCE syntax outline or citations differ from retained producer evidence",
        ));
    }
    Ok(())
}
