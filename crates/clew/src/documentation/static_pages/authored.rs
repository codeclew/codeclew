//! Exact frozen-publication consumption; no live-index or current-source fallback.
use super::model::{AuthoredParagraph, BundleProjection, CallableProjection};
use crate::{
    documentation::{
        bindings::Bindings, check::Check, digest, explanation_authorship, history, invalid,
        model::ServiceEvidence, store::Repository,
    },
    error::ClewError,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SELECTIONS: usize = 64;
const MAX_BUNDLES: usize = 4;
const MAX_CONTEXTS: usize = 8;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn attach(
    repo: &Repository,
    checked: &Check,
    projection: &mut BundleProjection,
) -> Result<(), ClewError> {
    let count: usize = projection
        .pages
        .iter()
        .map(|p| p.selection.authored_paragraphs.len())
        .sum();
    if count > MAX_SELECTIONS {
        return Err(invalid(
            "native authored paragraph selection exceeds 64 total fragments",
        ));
    }
    let bundles: BTreeSet<_> = projection
        .pages
        .iter()
        .flat_map(|p| &p.selection.authored_paragraphs)
        .map(|s| &s.bundle)
        .collect();
    if bundles.len() > MAX_BUNDLES {
        return Err(invalid(
            "native authored paragraph selection exceeds four frozen bundles",
        ));
    }
    // Inspect all selected bundles and all authored contexts before loading any Check.
    let mut budget = history::FrozenInputBudget {
        remaining_bytes: 64 * 1024 * 1024,
    };
    let mut inputs = BTreeMap::new();
    let mut snapshots = BTreeSet::new();
    for id in bundles {
        let (publication, raw) = history::read_frozen_bindings(repo, id, &mut budget)?;
        let binding: Bindings =
            serde_yaml_ng::from_slice(&raw).map_err(crate::documentation::io_error)?;
        for fragment in binding.fragments.values() {
            if let Some(snapshot) = fragment.content["authorship"]["sourceSnapshot"].as_str() {
                snapshots.insert(snapshot.to_owned());
            }
        }
        if snapshots.len() > MAX_CONTEXTS {
            return Err(invalid(
                "native frozen bindings exceed eight original source contexts; narrow the selection",
            ));
        }
        inputs.insert(id.clone(), (publication, raw));
    }
    let mut loaded = BTreeMap::new();
    let mut contexts = BTreeMap::new();
    for (id, (publication, raw)) in inputs {
        loaded.insert(
            id,
            history::validate_frozen_bindings(repo, publication, &raw, &mut contexts)?,
        );
    }
    let mut result_bytes = 0usize;
    for page in &mut projection.pages {
        let mut seen = BTreeSet::new();
        for selected in &page.selection.authored_paragraphs {
            if !seen.insert(selected) {
                return Err(invalid("duplicate native authored paragraph selector"));
            }
            let (publication, binding) = &loaded[&selected.bundle];
            let subject = format!("service:{}", page.selection.service);
            let narrative = binding.narratives.get(&subject).ok_or_else(|| {
                invalid("selected authored paragraph service is not in the frozen publication")
            })?;
            let operations: Vec<_> = narrative
                .operations
                .iter()
                .filter(|o| o.id == selected.operation)
                .collect();
            if operations.len() != 1 {
                return Err(invalid(
                    "selected authored operation is missing or ambiguous",
                ));
            }
            let operation = operations[0];
            let paragraphs: Vec<_> = operation
                .explanation
                .iter()
                .filter(|p| p.id == selected.fragment)
                .collect();
            if paragraphs.len() != 1 {
                return Err(invalid(
                    "selected authored paragraph is missing or ambiguous",
                ));
            }
            let paragraph = paragraphs[0];
            let authorship = paragraph
                .authorship
                .as_ref()
                .ok_or_else(|| invalid("selected explanation has no declared user authorship"))?;
            let key = format!("{subject}/{}/{}", operation.id, paragraph.id);
            if !explanation_authorship::authored_binding(binding, &key)? {
                return Err(invalid(
                    "selected authored paragraph has no exact frozen fragment binding",
                ));
            }
            let snapshot = &authorship.source_snapshot;
            if !contexts.contains_key(snapshot) {
                if contexts.len() >= MAX_CONTEXTS {
                    return Err(invalid(
                        "native authored paragraphs exceed eight original source contexts",
                    ));
                }
                contexts.insert(snapshot.clone(), Check::load_snapshot(repo, snapshot)?);
            }
            let original = &contexts[snapshot];
            explanation_authorship::validate_binding_pin(binding, &key, original)?;
            let service = original
                .services
                .get(&page.selection.service)
                .ok_or_else(|| invalid("authored paragraph original service is unavailable"))?;
            if service.service_digest != page.service_digest {
                return Err(invalid(
                    "authored paragraph service declaration differs from the selected native service",
                ));
            }
            if !endpoint_matches(
                service,
                &checked.services[&page.selection.service],
                &operation.id,
                &page.endpoint,
            ) {
                return Err(invalid(
                    "authored operation does not match the selected endpoint compiler symbol and scoped declaration",
                ));
            }
            // No section/note root can pass the discovered endpoint association.
            let evidence = binding.fragments[&key]
                .evidence
                .as_ref()
                .ok_or_else(|| invalid("authored paragraph original evidence is unavailable"))?;
            let projected = AuthoredParagraph {
                selection: selected.clone(),
                publication_digest: digest(publication)?,
                bindings_digest: publication.files.get("bindings.json").unwrap().clone(),
                operation_digest: digest(operation)?,
                paragraph_digest: digest(paragraph)?,
                paragraph: paragraph.clone(),
                context_freshness:
                    if explanation_authorship::validate(paragraph, checked).is_ok() {
                        "CURRENT"
                    } else {
                        "STALE"
                    }
                    .into(),
                source_records: evidence.sources.clone(),
            };
            result_bytes = result_bytes
                .checked_add(
                    crate::canonical::bytes(&projected)
                        .map_err(crate::documentation::io_error)?
                        .len(),
                )
                .ok_or_else(|| invalid("native authored paragraph result size overflow"))?;
            if result_bytes > MAX_RESULT_BYTES {
                return Err(invalid(
                    "native authored paragraph results exceed eight MiB",
                ));
            }
            page.authored_paragraphs.push(projected);
        }
    }
    Ok(())
}

fn endpoint_matches(
    original: &ServiceEvidence,
    current: &ServiceEvidence,
    operation: &str,
    endpoint: &CallableProjection,
) -> bool {
    let entries: Vec<_> = original
        .entrypoints
        .iter()
        .filter(|e| e.id == operation)
        .collect();
    entries.len() == 1
        && entries[0].service == current.service
        && entries[0].symbol == endpoint.symbol
        && entries[0].dependency_ids.contains(&endpoint.declaration_id)
        && original
            .observations
            .get(&endpoint.declaration_id)
            .is_some_and(|old| {
                old.kind == "SYMBOL"
                    && old.service == current.service
                    && old.symbol == endpoint.symbol
                    && current
                        .observations
                        .get(&endpoint.declaration_id)
                        .is_some_and(|new| {
                            new.kind == "SYMBOL"
                                && new.service == old.service
                                && new.symbol == old.symbol
                                && !old.normalized["scope"].is_null()
                                && old.normalized["scope"] == new.normalized["scope"]
                        })
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::model::{Entrypoint, Observation};
    use serde_json::json;
    #[test]
    fn same_symbol_in_another_compilation_cannot_borrow_authored_endpoint() {
        let symbol = "method:class:Fixture#submit()V";
        let mut original = ServiceEvidence {
            schema: "synthetic".into(),
            service: "fixture".into(),
            revision: "old".into(),
            service_digest: "config".into(),
            extractor: "synthetic".into(),
            runtime_mode: "STATIC".into(),
            coverage: "SEMANTIC".into(),
            boundaries: vec![],
            entrypoints: vec![Entrypoint {
                id: "entry:A".into(),
                service: "fixture".into(),
                symbol: symbol.into(),
                kind: "METHOD".into(),
                trigger: json!({}),
                source_ids: vec![],
                dependency_ids: vec!["decl:A".into()],
                boundaries: vec![],
            }],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        for scope in ["A", "B"] {
            let id = format!("decl:{scope}");
            original.observations.insert(
                id.clone(),
                Observation {
                    id,
                    kind: "SYMBOL".into(),
                    service: "fixture".into(),
                    symbol: symbol.into(),
                    normalized: json!({"scope":{"compilation":scope}}),
                    digest: "synthetic".into(),
                    source_ids: vec![],
                },
            );
        }
        let mut endpoint = CallableProjection {
            declaration_id: "decl:A".into(),
            symbol: symbol.into(),
            authority: "COMPILER_DECLARATION".into(),
            citation_id: None,
            steps: vec![],
            state: vec![],
            gaps: vec![],
        };
        let mut current = original.clone();
        assert!(endpoint_matches(&original, &current, "entry:A", &endpoint));
        endpoint.declaration_id = "decl:B".into();
        assert!(!endpoint_matches(&original, &current, "entry:A", &endpoint));
        endpoint.declaration_id = "decl:A".into();
        current.observations.get_mut("decl:A").unwrap().normalized["scope"] =
            json!({"compilation":"B"});
        assert!(!endpoint_matches(&original, &current, "entry:A", &endpoint));
    }
}
