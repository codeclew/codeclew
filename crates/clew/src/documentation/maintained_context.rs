//! Frozen attributed context for a new draft; never current compiler evidence.
use super::{
    bindings::Bindings,
    check::Check,
    digest, explanation_authorship, history, invalid,
    model::{Event, Explanation, Observation, Source},
    store::Repository,
    work::Request,
};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const SCHEMA: &str = "codeclew-maintained-paragraph-context/1.0";
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selector {
    pub bundle: String,
    pub operation: String,
    pub fragment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FromBundle {
    pub bundle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextFreshness {
    Current,
    Stale,
}

/// Historical records are deliberately separate from packet citations and bodies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaintainedContext {
    pub schema: String,
    pub selection: Selector,
    pub subject: String,
    pub service_digest: String,
    pub publication_digest: String,
    pub bindings_digest: String,
    pub operation_digest: String,
    pub paragraph_digest: String,
    pub context_digest: String,
    pub context_freshness: ContextFreshness,
    pub paragraph: Explanation,
    pub root: Observation,
    pub root_sources: BTreeMap<String, Source>,
    pub anchors: BTreeMap<String, Event>,
    pub source_records: BTreeMap<String, Source>,
    pub dependency_records: BTreeMap<String, Observation>,
}

fn declared_root<'a>(
    subject: &str,
    request: &Request,
    checked: &'a Check,
) -> Result<&'a Observation, ClewError> {
    if request.context_profile.as_deref() != Some("process-graph-v1")
        || !subject.starts_with("service:")
        || request.entrypoint.is_some()
    {
        return Err(invalid(
            "maintainedParagraph requires a service endpoint process-graph-v1 Work",
        ));
    }
    let selected = super::process_graph::resolve_work_root(subject, request, checked)?;
    if selected.declaration.source_ids.is_empty() {
        return Err(invalid(
            "maintainedParagraph requires an exact retained endpoint source",
        ));
    }
    Ok(selected.declaration)
}

fn root<'a>(
    subject: &str,
    request: &Request,
    checked: &'a Check,
) -> Result<&'a Observation, ClewError> {
    let declaration = declared_root(subject, request, checked)?;
    let selector = request
        .maintained_paragraph
        .as_ref()
        .ok_or_else(|| invalid("maintained endpoint selection is missing"))?;
    let service = checked
        .services
        .get(&declaration.service)
        .ok_or_else(|| invalid("maintained current endpoint service is missing"))?;
    let entries: Vec<_> = service
        .entrypoints
        .iter()
        .filter(|entry| entry.id == selector.operation)
        .collect();
    if entries.len() != 1
        || entries[0].service != declaration.service
        || entries[0].symbol != declaration.symbol
        || !entries[0].dependency_ids.contains(&declaration.id)
    {
        return Err(invalid(
            "maintainedParagraph requires the exact discovered endpoint operation, service, symbol and root declaration in the selected current Check",
        ));
    }
    Ok(declaration)
}

/// Select only a unique canonical authored fragment at this exact endpoint.
/// Body/source changes affect freshness, never the original pins or text.
fn automatic_selection(
    binding: &Bindings,
    contexts: &BTreeMap<String, Check>,
    subject: &str,
    current: &Observation,
    checked: &Check,
    bundle: &str,
) -> Result<Selector, ClewError> {
    let service = &checked.services[&current.service];
    let narrative = binding.narratives.get(subject).ok_or_else(|| {
        invalid("maintained paragraph service is absent from the selected publication")
    })?;
    let mut count = 0usize;
    let mut candidates = Vec::new();
    for operation in &narrative.operations {
        let entries: Vec<_> = service
            .entrypoints
            .iter()
            .filter(|entry| entry.id == operation.id)
            .collect();
        if entries.len() != 1
            || entries[0].service != current.service
            || entries[0].symbol != current.symbol
            || !entries[0].dependency_ids.contains(&current.id)
        {
            continue;
        }
        for paragraph in &operation.explanation {
            let Some(auth) = &paragraph.authorship else {
                continue;
            };
            let original = contexts
                .get(&auth.source_snapshot)
                .ok_or_else(|| invalid("maintained paragraph original snapshot is unavailable"))?;
            let Some(old_service) = original.services.get(&current.service) else {
                continue;
            };
            let old_entries: Vec<_> = old_service
                .entrypoints
                .iter()
                .filter(|entry| entry.id == operation.id)
                .collect();
            if old_service.service_digest != service.service_digest
                || old_entries.len() != 1
                || old_entries[0].service != current.service
                || old_entries[0].symbol != current.symbol
                || !old_entries[0].dependency_ids.contains(&current.id)
                || !old_service
                    .observations
                    .get(&current.id)
                    .is_some_and(|old| associated(old, current))
            {
                continue;
            }
            let key = format!("{subject}/{}/{}", operation.id, paragraph.id);
            explanation_authorship::validate_binding_pin(binding, &key, original)?;
            count += 1;
            if candidates.len() < 8 {
                candidates.push(Selector {
                    bundle: bundle.into(),
                    operation: operation.id.clone(),
                    fragment: paragraph.id.clone(),
                });
            }
        }
    }
    if count != 1 {
        let ids: Vec<_> = candidates
            .iter()
            .map(|candidate| {
                format!(
                    "{}/{}",
                    candidate.operation.chars().take(256).collect::<String>(),
                    candidate.fragment.chars().take(256).collect::<String>()
                )
            })
            .collect();
        return Err(invalid(format!(
            "MAINTAINED_ATTACHMENT_NOT_UNIQUE: {count} exact authored fragments; candidates (first eight): {ids:?}; use maintainedParagraph for an explicit selection"
        )));
    }
    Ok(candidates.remove(0))
}

fn associated(old: &Observation, current: &Observation) -> bool {
    old.id == current.id
        && old.kind == "SYMBOL"
        && old.service == current.service
        && old.symbol == current.symbol
        && old.normalized["scope"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        && old.normalized["scope"] == current.normalized["scope"]
}

impl MaintainedContext {
    fn freshness(&self, checked: &Check, current: &Observation) -> ContextFreshness {
        let sources = checked.sources();
        if &self.root == current
            && self
                .root_sources
                .iter()
                .all(|(id, source)| sources.get(id) == Some(source))
            && explanation_authorship::validate(&self.paragraph, checked).is_ok()
            && self
                .dependency_records
                .iter()
                .all(|(id, observation)| checked.dependencies.get(id) == Some(observation))
        {
            ContextFreshness::Current
        } else {
            ContextFreshness::Stale
        }
    }

    /// Validate only frozen Work bytes and its pinned Check, without reading history.
    pub(super) fn validate(
        &self,
        subject: &str,
        request: &Request,
        checked: &Check,
    ) -> Result<(), ClewError> {
        let current = root(subject, request, checked)?;
        let auth = self
            .paragraph
            .authorship
            .as_ref()
            .ok_or_else(|| invalid("maintained paragraph has no declared authorship"))?;
        explanation_authorship::validate_metadata(&self.paragraph)?;
        if self.schema != SCHEMA
            || self.subject != subject
            || request.maintained_paragraph.as_ref() != Some(&self.selection)
            || self.selection.fragment != self.paragraph.id
            || !associated(&self.root, current)
            || checked
                .services
                .get(&current.service)
                .is_none_or(|s| s.service_digest != self.service_digest)
            || self.paragraph_digest != digest(&self.paragraph)?
            || self.context_digest != explanation_authorship::context_digest(&self.paragraph)?
            || self.root.digest != digest(&self.root.normalized)?
            || self.root.source_ids.is_empty()
            || self.paragraph.source_ids.iter().collect::<BTreeSet<_>>()
                != auth.source_refs.keys().collect::<BTreeSet<_>>()
            || self.root.source_ids.iter().collect::<BTreeSet<_>>()
                != self.root_sources.keys().collect::<BTreeSet<_>>()
            || self.paragraph.event_ids.iter().collect::<BTreeSet<_>>()
                != self.anchors.keys().collect::<BTreeSet<_>>()
            || self
                .source_records
                .iter()
                .map(|(id, s)| Ok((id.clone(), digest(s)?)))
                .collect::<Result<BTreeMap<_, _>, ClewError>>()?
                != auth.source_refs
            || self
                .paragraph
                .dependency_ids
                .iter()
                .map(|id| {
                    Ok((
                        id.clone(),
                        digest(
                            self.dependency_records
                                .get(id)
                                .ok_or_else(|| invalid("maintained dependency is missing"))?,
                        )?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, ClewError>>()?
                != auth.dependency_refs
            || self.context_freshness != self.freshness(checked, current)
        {
            return Err(invalid(
                "maintained paragraph frozen Work binding is inconsistent",
            ));
        }
        for (id, source) in self.source_records.iter().chain(&self.root_sources) {
            if id != &source.id
                || crate::canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
            {
                return Err(invalid(
                    "maintained source identity or digest is inconsistent",
                ));
            }
        }
        for (id, observation) in &self.dependency_records {
            if id != &observation.id || observation.digest != digest(&observation.normalized)? {
                return Err(invalid(
                    "maintained dependency identity or digest is inconsistent",
                ));
            }
        }
        if self.anchors.iter().any(|(id, event)| id != &event.id) {
            return Err(invalid("maintained event anchor identity is inconsistent"));
        }
        if crate::canonical::bytes(self)
            .map_err(super::io_error)?
            .len()
            > MAX_BYTES
        {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "maintained context exceeds eight MiB; narrow the selected paragraph context",
            ));
        }
        Ok(())
    }
}

pub(super) fn validate_optional(
    context: Option<&MaintainedContext>,
    subject: &str,
    request: &Request,
    checked: &Check,
) -> Result<(), ClewError> {
    if request.maintained_from_bundle.is_some() {
        return Err(invalid(
            "maintainedFromBundle must be resolved when preparing new Work; saved Work requires its exact maintainedParagraph selector",
        ));
    }
    match (context, &request.maintained_paragraph) {
        (None, None) => Ok(()),
        (Some(context), Some(_)) => context.validate(subject, request, checked),
        _ => Err(invalid(
            "maintained paragraph selector and frozen context must be present together",
        )),
    }
}

/// Select an exact immutable publication once, before saving a new Work.
pub(super) fn load(
    repo: &Repository,
    subject: &str,
    request: &mut Request,
    checked: &Check,
) -> Result<Option<MaintainedContext>, ClewError> {
    if request.maintained_paragraph.is_some() && request.maintained_from_bundle.is_some() {
        return Err(invalid(
            "maintainedFromBundle conflicts with maintainedParagraph",
        ));
    }
    if request.maintained_paragraph.is_none() && request.maintained_from_bundle.is_none() {
        return Ok(None);
    }
    if let Some(selected) = &request.maintained_paragraph
        && (selected.operation.trim().is_empty()
            || selected.operation.len() > 512
            || selected.fragment.trim().is_empty()
            || selected.fragment.len() > 512)
    {
        return Err(invalid(
            "maintainedParagraph requires bounded nonblank operation and fragment IDs",
        ));
    }
    let automatic = request.maintained_from_bundle.is_some();
    let current = if automatic {
        declared_root(subject, request, checked)?
    } else {
        root(subject, request, checked)?
    };
    if automatic
        && !checked.services[&current.service]
            .entrypoints
            .iter()
            .any(|entry| {
                entry.service == current.service
                    && entry.symbol == current.symbol
                    && entry.dependency_ids.contains(&current.id)
            })
    {
        return Err(invalid(
            "maintainedFromBundle requires the exact discovered current endpoint root",
        ));
    }
    let bundle = request
        .maintained_from_bundle
        .as_ref()
        .map(|s| s.bundle.clone())
        .or_else(|| {
            request
                .maintained_paragraph
                .as_ref()
                .map(|s| s.bundle.clone())
        })
        .expect("selection checked");
    let mut budget = history::FrozenInputBudget {
        remaining_bytes: 64 * 1024 * 1024,
    };
    let (publication, raw) = history::read_frozen_bindings(repo, &bundle, &mut budget)?;
    let preliminary: Bindings = serde_yaml_ng::from_slice(&raw).map_err(super::io_error)?;
    let contexts: BTreeSet<_> = preliminary
        .fragments
        .values()
        .filter_map(|f| f.content["authorship"]["sourceSnapshot"].as_str())
        .collect();
    if contexts.len() > 8 {
        return Err(invalid(
            "maintained frozen bindings exceed eight original source contexts; narrow the selection",
        ));
    }
    let mut contexts = BTreeMap::new();
    let (publication, binding) =
        history::validate_frozen_bindings(repo, publication, &raw, &mut contexts)?;
    if automatic {
        let selected =
            automatic_selection(&binding, &contexts, subject, current, checked, &bundle)?;
        request.maintained_from_bundle = None;
        request.maintained_paragraph = Some(selected);
    }
    let selected = request
        .maintained_paragraph
        .as_ref()
        .expect("exact selection resolved");
    let current = root(subject, request, checked)?;
    let operations: Vec<_> = binding
        .narratives
        .get(subject)
        .ok_or_else(|| {
            invalid("maintained paragraph service is absent from the selected publication")
        })?
        .operations
        .iter()
        .filter(|o| o.id == selected.operation)
        .collect();
    let [operation] = operations.as_slice() else {
        return Err(invalid("maintained operation is missing or ambiguous"));
    };
    let paragraphs: Vec<_> = operation
        .explanation
        .iter()
        .filter(|p| p.id == selected.fragment)
        .collect();
    let [paragraph] = paragraphs.as_slice() else {
        return Err(invalid("maintained paragraph is missing or ambiguous"));
    };
    let auth = paragraph
        .authorship
        .as_ref()
        .ok_or_else(|| invalid("maintained paragraph has no declared human authorship"))?;
    let key = format!("{subject}/{}/{}", operation.id, paragraph.id);
    if !explanation_authorship::authored_binding(&binding, &key)? {
        return Err(invalid(
            "maintained paragraph has no exact authored fragment binding",
        ));
    }
    let original = contexts
        .get(&auth.source_snapshot)
        .ok_or_else(|| invalid("maintained paragraph original snapshot is unavailable"))?;
    explanation_authorship::validate_binding_pin(&binding, &key, original)?;
    let service = original
        .services
        .get(&current.service)
        .ok_or_else(|| invalid("maintained paragraph original service is unavailable"))?;
    let entries: Vec<_> = service
        .entrypoints
        .iter()
        .filter(|e| e.id == operation.id)
        .collect();
    let original_root = service.observations.get(&current.id).ok_or_else(|| invalid("maintained endpoint root is absent from its original snapshot; no identity remapping is supported"))?;
    if entries.len() != 1
        || entries[0].service != current.service
        || entries[0].symbol != current.symbol
        || !entries[0].dependency_ids.contains(&current.id)
        || !associated(original_root, current)
    {
        return Err(invalid(
            "maintained operation does not match the exact endpoint symbol and scoped root declaration",
        ));
    }
    let evidence = binding.fragments[&key]
        .evidence
        .as_ref()
        .ok_or_else(|| invalid("maintained paragraph historical evidence is unavailable"))?;
    let mut context = MaintainedContext {
        schema: SCHEMA.into(),
        selection: selected.clone(),
        subject: subject.into(),
        service_digest: service.service_digest.clone(),
        publication_digest: digest(&publication)?,
        bindings_digest: publication
            .files
            .get("bindings.json")
            .ok_or_else(|| invalid("maintained publication has no bindings digest"))?
            .clone(),
        operation_digest: digest(operation)?,
        paragraph_digest: digest(paragraph)?,
        context_digest: explanation_authorship::context_digest(paragraph)?,
        context_freshness: ContextFreshness::Stale,
        paragraph: (**paragraph).clone(),
        root: original_root.clone(),
        root_sources: original_root
            .source_ids
            .iter()
            .map(|id| {
                Ok((
                    id.clone(),
                    service
                        .sources
                        .get(id)
                        .ok_or_else(|| invalid("maintained endpoint source is unavailable"))?
                        .clone(),
                ))
            })
            .collect::<Result<_, ClewError>>()?,
        anchors: paragraph
            .event_ids
            .iter()
            .map(|id| {
                let events: Vec<_> = operation.events.iter().filter(|e| &e.id == id).collect();
                if events.len() != 1 {
                    return Err(invalid(
                        "maintained historical event anchor is missing or ambiguous",
                    ));
                }
                Ok((id.clone(), events[0].clone()))
            })
            .collect::<Result<_, ClewError>>()?,
        source_records: evidence.sources.clone(),
        dependency_records: evidence.observations.clone(),
    };
    context.context_freshness = context.freshness(checked, current);
    context.validate(subject, request, checked)?;
    Ok(Some(context))
}

/// Owned synthetic packet fixture; it does not assert native capture or publication.
#[cfg(test)]
pub(in crate::documentation) fn fixture(work: &mut super::work::Work, text: String) {
    use super::model::{
        AuthoredContextRole, AuthoredMeaningReview, ExplanationAuthority, ExplanationAuthorship,
    };
    work.request.context_profile = Some("process-graph-v1".into());
    work.request.entrypoint = None;
    work.request.root_declaration = Some("endpoint-declaration".into());
    work.request.question =
        Some("Explain current source, preserving attributed context boundaries.".into());
    let selected = Selector {
        bundle: "c".repeat(64),
        operation: "entry-http".into(),
        fragment: "maintained".into(),
    };
    work.request.maintained_paragraph = Some(selected.clone());
    let root = work.checked.services["orders"].observations["endpoint-declaration"].clone();
    let source_records: BTreeMap<_, _> = root
        .source_ids
        .iter()
        .map(|id| (id.clone(), work.checked.sources()[id].clone()))
        .collect();
    let dependency_records = BTreeMap::from([(root.id.clone(), root.clone())]);
    let paragraph = Explanation {
        id: selected.fragment.clone(),
        text,
        event_ids: vec![],
        dependency_ids: vec![root.id.clone()],
        source_ids: root.source_ids.clone(),
        detail: false,
        authorship: Some(ExplanationAuthorship {
            authority: ExplanationAuthority::UserDocumentation,
            author: "Fixture <maintainer>".into(),
            meaning_review: AuthoredMeaningReview::Unassessed,
            context_role: AuthoredContextRole::RetainedUnverifiedContext,
            edit_digest: format!("sha256:{}", "e".repeat(64)),
            source_snapshot: work.snapshot.clone().unwrap(),
            source_refs: source_records
                .iter()
                .map(|(id, s)| (id.clone(), digest(s).unwrap()))
                .collect(),
            dependency_refs: dependency_records
                .iter()
                .map(|(id, o)| (id.clone(), digest(o).unwrap()))
                .collect(),
            context_migration: None,
        }),
    };
    work.maintained_context = Some(MaintainedContext {
        schema: SCHEMA.into(),
        selection: selected,
        subject: work.subject.clone(),
        service_digest: work.checked.services["orders"].service_digest.clone(),
        publication_digest: format!("sha256:{}", "f".repeat(64)),
        bindings_digest: format!("sha256:{}", "d".repeat(64)),
        operation_digest: format!("sha256:{}", "a".repeat(64)),
        paragraph_digest: digest(&paragraph).unwrap(),
        context_digest: explanation_authorship::context_digest(&paragraph).unwrap(),
        context_freshness: ContextFreshness::Current,
        paragraph,
        root,
        root_sources: source_records.clone(),
        anchors: BTreeMap::new(),
        source_records,
        dependency_records,
    });
}

/// A valid bounded paragraph whose complete pinned source context exceeds old page limits.
#[cfg(test)]
pub(in crate::documentation) fn large_fixture(work: &mut super::work::Work) {
    let source = work
        .checked
        .services
        .get_mut("orders")
        .unwrap()
        .sources
        .get_mut("endpoint-source")
        .unwrap();
    source.text.push_str(&format!(
        " /* {} */",
        "Synthetic historical context λ☕. ".repeat(1800)
    ));
    source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
    fixture(
        work,
        "Synthetic UNASSESSED human context; historical source is attributed context, not proof."
            .into(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn work() -> super::super::work::Work {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        large_fixture(&mut work);
        work
    }

    // Selection-only synthetic records. Frozen filesystem admission is exercised
    // separately through the existing actual native publication/capture journey.
    fn automatic_fixture() -> (super::super::work::Work, Bindings, BTreeMap<String, Check>) {
        use serde_json::json;
        let work = work();
        let paragraph = &work.maintained_context.as_ref().unwrap().paragraph;
        let operation = json!({"id":"entry-http","title":"Synthetic endpoint",
            "summary":{"id":"summary","text":"Synthetic summary","dependencyIds":[],"sourceIds":[]},
            "explanation":[paragraph],"participants":[],"events":[]});
        let mut binding: Bindings = serde_json::from_value(json!({"schema":"synthetic", "inputDigest":"synthetic",
            "renderer":"synthetic","extractor":"synthetic","influenceScopes":{},"revisions":{},"coverage":{},"catalogues":{},
            "fragments":{},"observations":{},"narratives":{(work.subject.clone()):{"schema":"synthetic","subject":work.subject,"contextDigest":"synthetic","operations":[operation]}},
            "outputHashes":{},"retainedSources":{},"sectionStates":{},"targetRevisions":{},"updateFailures":{}})).unwrap();
        let fragment = super::super::bindings::fragment(
            &work.subject,
            paragraph,
            &paragraph.dependency_ids,
            &paragraph.source_ids,
            &work.checked,
        )
        .unwrap();
        binding.fragments.insert(
            format!("{}/entry-http/{}", work.subject, paragraph.id),
            fragment,
        );
        let contexts = BTreeMap::from([(
            paragraph
                .authorship
                .as_ref()
                .unwrap()
                .source_snapshot
                .clone(),
            work.checked.clone(),
        )]);
        (work, binding, contexts)
    }

    fn resolve_automatic(
        work: &super::super::work::Work,
        binding: &Bindings,
        contexts: &BTreeMap<String, Check>,
        bundle: &str,
    ) -> Result<Selector, ClewError> {
        let current = declared_root(&work.subject, &work.request, &work.checked)?;
        automatic_selection(
            binding,
            contexts,
            &work.subject,
            current,
            &work.checked,
            bundle,
        )
    }

    #[test]
    fn automatic_attachment_preserves_exact_identity_body_relocation_and_specified_text_versions() {
        let (mut work, binding, contexts) = automatic_fixture();
        let old = super::super::bytes(&binding).unwrap();
        let bundle = "c".repeat(64);
        let selector = resolve_automatic(&work, &binding, &contexts, &bundle).unwrap();
        assert_eq!(selector, work.request.maintained_paragraph.clone().unwrap());
        let context = work.maintained_context.clone().unwrap();
        let source = work
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap();
        source.text.push_str(" /* changed current body */");
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        assert_eq!(
            resolve_automatic(&work, &binding, &contexts, &bundle).unwrap(),
            selector
        );
        assert_eq!(
            context.freshness(
                &work.checked,
                declared_root(&work.subject, &work.request, &work.checked).unwrap()
            ),
            ContextFreshness::Stale
        );
        let source = work
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap();
        source.start_line += 3;
        source.end_line += 3;
        assert_eq!(
            resolve_automatic(&work, &binding, &contexts, &bundle).unwrap(),
            selector
        );
        assert_eq!(super::super::bytes(&binding).unwrap(), old);
        // Two independently pinned input bindings retain their distinct authored text.
        let mut competing = binding.clone();
        let paragraph = &mut competing
            .narratives
            .get_mut(&work.subject)
            .unwrap()
            .operations[0]
            .explanation[0];
        paragraph.text = "Distinct synthetic human edit; never overwrite the earlier text.".into();
        paragraph.authorship.as_mut().unwrap().author = "Second fixture editor".into();
        let key = format!("{}/entry-http/{}", work.subject, paragraph.id);
        competing.fragments.insert(
            key,
            super::super::bindings::fragment(
                &work.subject,
                paragraph,
                &paragraph.dependency_ids,
                &paragraph.source_ids,
                &contexts[&paragraph.authorship.as_ref().unwrap().source_snapshot],
            )
            .unwrap(),
        );
        let other_bundle = "d".repeat(64);
        let other = resolve_automatic(&work, &competing, &contexts, &other_bundle).unwrap();
        assert_eq!(other.bundle, other_bundle);
        assert_eq!(other.fragment, selector.fragment);
        assert_ne!(
            competing.narratives[&work.subject].operations[0].explanation[0].text,
            context.paragraph.text
        );
        assert_eq!(super::super::bytes(&binding).unwrap(), old);
    }

    #[test]
    fn automatic_attachment_refuses_missing_ambiguous_foreign_declarations_and_corrupt_pins() {
        let (work, binding, contexts) = automatic_fixture();
        let bundle = "c".repeat(64);
        let mut missing = binding.clone();
        missing
            .narratives
            .get_mut(&work.subject)
            .unwrap()
            .operations[0]
            .explanation
            .clear();
        assert!(
            resolve_automatic(&work, &missing, &contexts, &bundle)
                .unwrap_err()
                .message
                .contains("0 exact authored fragments")
        );
        let mut ambiguous = binding.clone();
        for index in 0..12 {
            let mut paragraph =
                binding.narratives[&work.subject].operations[0].explanation[0].clone();
            paragraph.id = format!("another-{index}");
            let fragment = super::super::bindings::fragment(
                &work.subject,
                &paragraph,
                &paragraph.dependency_ids,
                &paragraph.source_ids,
                &work.checked,
            )
            .unwrap();
            ambiguous.fragments.insert(
                format!("{}/entry-http/{}", work.subject, paragraph.id),
                fragment,
            );
            ambiguous
                .narratives
                .get_mut(&work.subject)
                .unwrap()
                .operations[0]
                .explanation
                .push(paragraph);
        }
        let refused = resolve_automatic(&work, &ambiguous, &contexts, &bundle).unwrap_err();
        assert!(refused.message.contains("13 exact authored fragments"));
        assert!(refused.message.len() < 4096);
        assert!(!refused.message.contains("another-11"));
        for variant in ["missing", "scope", "descriptor", "identity"] {
            let mut foreign = work.clone();
            let service = foreign.checked.services.get_mut("orders").unwrap();
            match variant {
                "missing" => service.entrypoints.clear(),
                "scope" => {
                    service
                        .observations
                        .get_mut("endpoint-declaration")
                        .unwrap()
                        .normalized["scope"] = serde_json::json!("different-scope");
                }
                "descriptor" => {
                    service
                        .observations
                        .get_mut("endpoint-declaration")
                        .unwrap()
                        .symbol = "method:orders.Controller#handle(I)V".into();
                    service.entrypoints[0].symbol = "method:orders.Controller#handle(I)V".into();
                }
                _ => {
                    let mut root = service.observations.remove("endpoint-declaration").unwrap();
                    root.id = "different-method-identity".into();
                    service.observations.insert(root.id.clone(), root);
                    service.entrypoints[0].dependency_ids =
                        vec!["different-method-identity".into()];
                    foreign.request.root_declaration = Some("different-method-identity".into());
                }
            }
            assert!(
                resolve_automatic(&foreign, &binding, &contexts, &bundle).is_err(),
                "{variant}"
            );
        }
        let mut foreign_service = work.clone();
        foreign_service.subject = "service:another".into();
        assert!(resolve_automatic(&foreign_service, &binding, &contexts, &bundle).is_err());
        let mut corrupt = binding.clone();
        let key = format!("{}/entry-http/maintained", work.subject);
        corrupt
            .fragments
            .get_mut(&key)
            .unwrap()
            .evidence
            .as_mut()
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap()
            .text
            .push_str(" corrupt original bytes");
        assert!(resolve_automatic(&work, &corrupt, &contexts, &bundle).is_err());
        assert!(resolve_automatic(&work, &binding, &BTreeMap::new(), &bundle).is_err());
    }

    #[test]
    fn automatic_input_is_closed_conflicting_and_omitted_selection_never_reads_history() {
        let temp = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temp.path(), "Synthetic omitted context").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let work = work();
        let mut omitted = work.request.clone();
        omitted.maintained_paragraph = None;
        let legacy = super::super::bytes(&omitted).unwrap();
        assert!(
            load(&repo, &work.subject, &mut omitted, &work.checked)
                .unwrap()
                .is_none()
        );
        assert_eq!(super::super::bytes(&omitted).unwrap(), legacy);
        let mut explicit = work.request.clone();
        explicit.maintained_from_bundle = Some(FromBundle {
            bundle: "c".repeat(64),
        });
        assert!(
            load(&repo, &work.subject, &mut explicit, &work.checked)
                .unwrap_err()
                .message
                .contains("conflicts")
        );
        assert!(
            validate_optional(
                work.maintained_context.as_ref(),
                &work.subject,
                &explicit,
                &work.checked
            )
            .is_err()
        );
        let mut null = serde_json::to_value(&omitted).unwrap();
        null["maintainedFromBundle"] = serde_json::Value::Null;
        let restored: Request = serde_json::from_value(null).unwrap();
        assert_eq!(super::super::bytes(&restored).unwrap(), legacy);
        let invalid = serde_json::json!({"bundle":"c".repeat(64),"operation":"unrequested"});
        assert!(serde_json::from_value::<FromBundle>(invalid).is_err());
    }

    #[test]
    fn complete_context_is_hashed_without_becoming_a_compiler_citation() {
        let work = work();
        let (packet, audit) = super::super::operation_packet::build(&work).unwrap();
        assert!(
            packet["maintainedContext"]["sourceRecords"]["endpoint-source"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                > 49152
        );
        assert_eq!(
            packet["maintainedContext"],
            serde_json::to_value(work.maintained_context.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            packet["maintainedContext"]["paragraph"]["authorship"]["meaningReview"],
            "UNASSESSED"
        );
        let mut content = packet.clone();
        content.as_object_mut().unwrap().remove("packetDigest");
        assert_eq!(packet["packetDigest"], digest(&content).unwrap());
        let citations = packet["citations"].clone();
        let mut without = work.clone();
        without.request.maintained_paragraph = None;
        without.maintained_context = None;
        let (legacy, legacy_audit) = super::super::operation_packet::build(&without).unwrap();
        assert!(legacy.get("maintainedContext").is_none());
        assert_eq!(legacy["citations"], citations);
        assert_ne!(legacy["packetDigest"], packet["packetDigest"]);
        assert_eq!(audit["records"], legacy_audit["records"]);
        assert!(
            !serde_json::to_value(&without)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("maintainedContext")
        );
        // Explicit null selection is the same omitted legacy request after parsing.
        let mut request = serde_json::to_value(&without.request).unwrap();
        request["maintainedParagraph"] = serde_json::Value::Null;
        let restored: Request = serde_json::from_value(request).unwrap();
        assert_eq!(
            super::super::bytes(&restored).unwrap(),
            super::super::bytes(&without.request).unwrap()
        );
    }

    #[test]
    fn saved_packet_context_presence_and_full_content_must_match_frozen_work() {
        let work = work();
        let packet = super::super::operation_packet::build(&work).unwrap().0;
        super::super::operation_packet::validate_saved_maintained_context(&work, &packet).unwrap();
        let mut omitted = packet.clone();
        omitted.as_object_mut().unwrap().remove("maintainedContext");
        assert!(
            super::super::operation_packet::validate_saved_maintained_context(&work, &omitted)
                .is_err()
        );
        let mut null = packet.clone();
        null["maintainedContext"] = serde_json::Value::Null;
        assert!(
            super::super::operation_packet::validate_saved_maintained_context(&work, &null)
                .is_err()
        );
        for path in [
            vec!["paragraph", "text"],
            vec!["paragraph", "authorship", "author"],
            vec!["sourceRecords", "endpoint-source", "text"],
            vec!["contextDigest"],
        ] {
            let mut changed = packet.clone();
            let mut value = &mut changed["maintainedContext"];
            for key in path {
                value = &mut value[key];
            }
            *value = serde_json::json!("substituted historical content");
            // Global self-consistency cannot substitute a different frozen context.
            changed.as_object_mut().unwrap().remove("packetDigest");
            changed["packetDigest"] = serde_json::json!(digest(&changed).unwrap());
            assert!(
                super::super::operation_packet::validate_saved_maintained_context(&work, &changed)
                    .is_err()
            );
        }
        let mut legacy = work.clone();
        legacy.maintained_context = None;
        legacy.request.maintained_paragraph = None;
        let legacy_packet = super::super::operation_packet::build(&legacy).unwrap().0;
        super::super::operation_packet::validate_saved_maintained_context(&legacy, &legacy_packet)
            .unwrap();
        assert!(
            super::super::operation_packet::validate_saved_maintained_context(&legacy, &packet)
                .is_err()
        );
    }

    #[test]
    fn changed_source_is_stale_but_changed_scoped_root_cannot_borrow_context() {
        let mut work = work();
        let original = work.maintained_context.clone().unwrap();
        let source = work
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap();
        source.text.push_str(" /* changed current source */");
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        work.maintained_context.as_mut().unwrap().context_freshness = ContextFreshness::Stale;
        let packet = super::super::operation_packet::build(&work).unwrap().0;
        assert_eq!(
            packet["maintainedContext"]["paragraph"],
            serde_json::to_value(&original.paragraph).unwrap()
        );
        assert_eq!(
            packet["maintainedContext"]["sourceRecords"],
            serde_json::to_value(&original.source_records).unwrap()
        );
        assert_eq!(packet["maintainedContext"]["contextFreshness"], "STALE");
        let mut removed_endpoint = work.clone();
        removed_endpoint
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .entrypoints
            .clear();
        assert!(
            super::super::operation_packet::build(&removed_endpoint)
                .unwrap_err()
                .message
                .contains("exact discovered endpoint")
        );
        let mut unrelated_endpoint = work.clone();
        unrelated_endpoint
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .entrypoints[0]
            .symbol = "method:orders.Other#handle()V".into();
        assert!(
            super::super::operation_packet::build(&unrelated_endpoint)
                .unwrap_err()
                .message
                .contains("exact discovered endpoint")
        );
        let mut missing_root_membership = work.clone();
        missing_root_membership
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .entrypoints[0]
            .dependency_ids
            .clear();
        assert!(
            super::super::operation_packet::build(&missing_root_membership)
                .unwrap_err()
                .message
                .contains("exact discovered endpoint")
        );
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .get_mut("endpoint-declaration")
            .unwrap()
            .normalized["scope"] = serde_json::json!(":other");
        assert!(super::super::operation_packet::build(&work).is_err());
    }

    #[test]
    fn forged_missing_and_oversized_frozen_context_refuse_packet_construction() {
        let mut missing = work();
        missing.maintained_context = None;
        assert!(super::super::operation_packet::build(&missing).is_err());
        let mut forged = work();
        forged
            .maintained_context
            .as_mut()
            .unwrap()
            .paragraph
            .authorship
            .as_mut()
            .unwrap()
            .source_refs
            .clear();
        assert!(super::super::operation_packet::build(&forged).is_err());
        let mut wrong = work();
        wrong
            .request
            .maintained_paragraph
            .as_mut()
            .unwrap()
            .fragment = "other".into();
        assert!(super::super::operation_packet::build(&wrong).is_err());
        let mut missing_source = work();
        missing_source
            .maintained_context
            .as_mut()
            .unwrap()
            .root_sources
            .clear();
        assert!(super::super::operation_packet::build(&missing_source).is_err());
        let mut missing_anchor = work();
        let context = missing_anchor.maintained_context.as_mut().unwrap();
        context
            .paragraph
            .event_ids
            .push("historical-event-missing".into());
        context.paragraph_digest = digest(&context.paragraph).unwrap();
        assert!(super::super::operation_packet::build(&missing_anchor).is_err());
        let mut damaged_source = work();
        damaged_source
            .maintained_context
            .as_mut()
            .unwrap()
            .source_records
            .get_mut("endpoint-source")
            .unwrap()
            .text
            .push_str("forged bytes");
        assert!(super::super::operation_packet::build(&damaged_source).is_err());
        let mut oversized = work();
        let context = oversized.maintained_context.as_mut().unwrap();
        context.paragraph.text = "x".repeat(MAX_BYTES + 1);
        context.paragraph_digest = digest(&context.paragraph).unwrap();
        assert_eq!(
            super::super::operation_packet::build(&oversized)
                .unwrap_err()
                .code,
            ErrorCode::SliceBudgetExceeded
        );
    }
}
