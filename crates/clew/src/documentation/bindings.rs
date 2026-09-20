//! Portable fragment dependencies; retained records are baselines, not new proof.
use super::{
    check::Check,
    digest, invalid, io_error,
    model::*,
    store::{self, Repository},
};
use crate::{
    canonical,
    error::{ClewError, ErrorCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FragmentBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub influence_scope: Option<String>,
    pub subject: String,
    pub content_digest: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub content: Value,
    pub dependencies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dependencies_from_evidence: bool,
    pub sources: BTreeMap<String, Value>,
    /// Each retained claim owns its evidence version, even when a sibling is updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<FragmentEvidence>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FragmentEvidence {
    pub revisions: BTreeMap<String, String>,
    pub observations: BTreeMap<String, Observation>,
    pub sources: BTreeMap<String, Source>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_observations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_sources: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Share identical evidence within one portable snapshot. Older versions of an
/// observation or source remain attached to their original fragment.
pub(super) fn compact(binding: &mut Bindings) {
    prune_influence_scopes(binding);
    binding.schema = "codeclew-documentation-bindings/1.4".into();
    for fragment in binding.fragments.values_mut() {
        let Some(evidence) = fragment.evidence.as_mut() else {
            continue;
        };
        let derived = evidence
            .observations
            .iter()
            .map(|(id, o)| (id.clone(), o.digest.clone()))
            .collect();
        if fragment.dependencies == derived {
            fragment.dependencies.clear();
            fragment.dependencies_from_evidence = true;
        }
        evidence.observations.retain(|id, value| {
            if binding.observations.get(id) == Some(value) {
                evidence.shared_observations.push(id.clone());
                false
            } else {
                true
            }
        });
        evidence.sources.retain(|id, value| {
            if binding.retained_sources.get(id) == Some(value) {
                evidence.shared_sources.push(id.clone());
                false
            } else {
                true
            }
        });
    }
}

/// Keep only scopes that still own published fragments or accepted versions.
/// Replacing one operation must not discard a scope retained by its siblings.
pub(super) fn prune_influence_scopes(binding: &mut Bindings) {
    let referenced: BTreeSet<_> = binding
        .fragments
        .values()
        .filter_map(|fragment| fragment.influence_scope.as_ref())
        .chain(
            binding
                .accepted_versions
                .values()
                .map(|version| &version.influence.scope),
        )
        .collect();
    binding
        .influence_scopes
        .retain(|scope, _| referenced.contains(scope));
}

fn expand_shared(binding: &mut Bindings) -> Result<(), ClewError> {
    for fragment in binding.fragments.values_mut() {
        if fragment.dependencies_from_evidence && fragment.evidence.is_none() {
            return Err(invalid(
                "derived fragment dependencies require retained evidence",
            ));
        }
        let Some(evidence) = fragment.evidence.as_mut() else {
            continue;
        };
        for id in std::mem::take(&mut evidence.shared_observations) {
            let value = binding
                .observations
                .get(&id)
                .ok_or_else(|| invalid("shared fragment observation is missing"))?;
            if evidence.observations.insert(id, value.clone()).is_some() {
                return Err(invalid("duplicate shared fragment observation"));
            }
        }
        for id in std::mem::take(&mut evidence.shared_sources) {
            let value = binding
                .retained_sources
                .get(&id)
                .ok_or_else(|| invalid("shared fragment source is missing"))?;
            if evidence.sources.insert(id, value.clone()).is_some() {
                return Err(invalid("duplicate shared fragment source"));
            }
        }
        if std::mem::take(&mut fragment.dependencies_from_evidence) {
            if !fragment.dependencies.is_empty() {
                return Err(invalid(
                    "derived fragment dependencies cannot also be inline",
                ));
            }
            fragment.dependencies = evidence
                .observations
                .iter()
                .map(|(id, o)| (id.clone(), o.digest.clone()))
                .collect();
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bindings {
    /// Presentation target only; does not change source-analysis identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_language: Option<String>,
    pub influence_scopes: BTreeMap<String, super::review::InfluenceScope>,
    pub schema: String,
    pub input_digest: String,
    pub renderer: String,
    pub extractor: String,
    pub revisions: BTreeMap<String, String>,
    pub coverage: BTreeMap<String, Value>,
    pub catalogues: BTreeMap<String, Vec<String>>,
    pub fragments: BTreeMap<String, FragmentBinding>,
    pub observations: BTreeMap<String, Observation>,
    pub narratives: BTreeMap<String, Narrative>,
    pub output_hashes: BTreeMap<String, String>,
    pub retained_sources: BTreeMap<String, Source>,
    pub section_states: BTreeMap<String, SectionState>,
    pub target_revisions: BTreeMap<String, Option<String>>,
    pub update_failures: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accepted_versions: BTreeMap<String, super::review::AcceptedVersion>,
}

pub fn expand_dependencies(
    initial: &[String],
    checked: &Check,
) -> Result<BTreeSet<String>, ClewError> {
    let mut output: BTreeSet<_> = initial.iter().cloned().collect();
    // Claims depend conservatively on each involved service's read/query scope.
    let services: BTreeSet<_> = initial
        .iter()
        .filter_map(|id| checked.dependencies.get(id))
        .map(|d| d.service.as_str())
        .collect();
    output.extend(
        checked
            .dependencies
            .values()
            .filter(|d| {
                matches!(
                    d.kind.as_str(),
                    "SOURCE_SCOPE"
                        | "EVIDENCE_PACKAGE"
                        | "MODULE_SCOPE"
                        | "CONTRACT_SCOPE"
                        | "ENTITY_SCOPE"
                        | "NOTE_SCOPE"
                ) && services.contains(d.service.as_str())
            })
            .map(|d| d.id.clone()),
    );
    let mut frontier = output.clone();
    for depth in 0..=16 {
        let mut next = BTreeSet::new();
        for id in &frontier {
            let dependency = checked
                .dependencies
                .get(id)
                .ok_or_else(|| invalid("fragment has a missing dependency"))?;
            if matches!(
                dependency.kind.as_str(),
                "DOMAIN_ENTITY"
                    | "ENTITY_SCOPE"
                    | "NOTE_SCOPE"
                    | "NOTE_ASSOCIATION"
                    | "PROCESS_DEFINITION"
                    | "PROCESS_COMPONENT"
                    | "PROCESS_SCOPE"
                    | "VIEW_DEFINITION"
                    | "VIEW_SCOPE"
            ) {
                for linked in dependency.normalized["dependencyIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if checked.dependencies.contains_key(linked) {
                        next.insert(linked.to_owned());
                    }
                }
            }
            if !matches!(dependency.kind.as_str(), "SYMBOL" | "FLOW") {
                continue;
            }
            let Some(service) = checked.services.get(&dependency.service) else {
                continue;
            };
            let parent = service
                .observations
                .values()
                .find(|o| o.kind == "SYMBOL" && o.symbol == dependency.symbol);
            if let Some(parent) = parent {
                next.insert(parent.id.clone());
                if let Some(events) = parent
                    .normalized
                    .pointer("/documentation/events")
                    .and_then(Value::as_array)
                {
                    for target in events.iter().filter_map(|e| e["target"].as_str()) {
                        if let Some(callee) = service
                            .observations
                            .values()
                            .find(|o| o.kind == "SYMBOL" && o.symbol == target)
                        {
                            next.insert(callee.id.clone());
                        }
                    }
                }
            }
        }
        next.retain(|id| !output.contains(id));
        if next.is_empty() {
            return Ok(output);
        }
        if depth == 16 || output.len() + next.len() > 4096 {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "fragment dependency closure exceeds its bounded scope",
            ));
        }
        output.extend(next.iter().cloned());
        frontier = next;
    }
    Ok(output)
}

pub fn fragment(
    subject: &str,
    value: &impl Serialize,
    dependency_ids: &[String],
    source_ids: &[String],
    checked: &Check,
) -> Result<FragmentBinding, ClewError> {
    let mut initial = dependency_ids.to_vec();
    if let Some(service) = subject.strip_prefix("service:") {
        initial.extend(
            checked
                .dependencies
                .values()
                .filter(|d| {
                    matches!(
                        d.kind.as_str(),
                        "SOURCE_SCOPE"
                            | "EVIDENCE_PACKAGE"
                            | "CONTRACT_SCOPE"
                            | "ENTITY_SCOPE"
                            | "NOTE_SCOPE"
                    ) && d.service == service
                })
                .map(|d| d.id.clone()),
        );
    }
    if let Some(id) = subject.strip_prefix("scenario:")
        && let Some(context) = checked.scenarios.get(id)
    {
        initial.extend(context.dependency_ids.iter().cloned());
    }
    let ids = expand_dependencies(&initial, checked)?;
    let sources = checked.sources();
    let observations: BTreeMap<_, _> = ids
        .iter()
        .map(|id| (id.clone(), checked.dependencies[id].clone()))
        .collect();
    let retained: BTreeMap<_, _> = source_ids
        .iter()
        .map(|id| {
            let source = sources
                .get(id)
                .ok_or_else(|| invalid("fragment source is unavailable"))?;
            Ok((id.clone(), source.clone()))
        })
        .collect::<Result<_, ClewError>>()?;
    let services: BTreeSet<_> = observations
        .values()
        .map(|o| o.service.as_str())
        .chain(retained.values().map(|s| s.service.as_str()))
        .filter(|s| !s.is_empty())
        .collect();
    Ok(FragmentBinding {
        influence_scope: None,
        dependencies_from_evidence: false,
        subject: subject.into(), content_digest: digest(value)?,
        content: serde_json::to_value(value).map_err(io_error)?,
        dependencies: observations.iter().map(|(id, o)| (id.clone(), o.digest.clone())).collect(),
        sources: retained.iter().map(|(id, source)| (id.clone(), json!({"revision":source.revision,"file":source.file,"startLine":source.start_line,"endLine":source.end_line,"textDigest":source.text_digest,"url":source.url}))).collect(),
        evidence: Some(FragmentEvidence {
            shared_observations: vec![], shared_sources: vec![],
            revisions: services.iter().filter_map(|id| checked.services.get(*id).map(|e| ((*id).to_owned(),e.revision.clone()))).collect(),
            observations, sources: retained,
        }),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BaselineReceipt {
    pub bundle: String,
    pub index_digest: String,
    pub bindings_digest: String,
}

pub fn baseline(repo: &Repository) -> Result<Option<(String, Bindings)>, ClewError> {
    Ok(capture_baseline(repo)?.map(|(receipt, binding)| (receipt.bundle, binding)))
}

/// Bind provenance to the exact bytes parsed and validated below, never to a
/// second read of the mutable pointer or its selected bindings file.
pub(super) fn capture_baseline(
    repo: &Repository,
) -> Result<Option<(BaselineReceipt, Bindings)>, ClewError> {
    let index = repo.path("docs/index.html")?;
    let metadata = match fs::symlink_metadata(&index) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
        return Err(invalid("documentation index is not a bounded regular file"));
    }
    let index_text = fs::read_to_string(&index).map_err(io_error)?;
    if index_text.len() > 2 * 1024 * 1024 {
        return Err(invalid("documentation index grew beyond its record budget"));
    }
    let Some(first) = index_text.lines().next() else {
        return Err(invalid(
            "existing docs/index.html is not generated by Codeclew",
        ));
    };
    let id = first
        .strip_prefix("<!-- codeclew-bundle ")
        .and_then(|s| s.strip_suffix(" -->"))
        .ok_or_else(|| {
            invalid("existing docs/index.html is manually owned; use a separate documentation root")
        })?;
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid documentation bundle pointer"));
    }
    let relative = format!("docs/generated/{id}/bindings.json");
    let path = repo.path(&relative)?;
    let missing_bindings = || {
        invalid(format!(
            "DOCS_BASELINE_INCOMPLETE: docs/index.html selects missing {relative}; restore that generated bundle from the same documentation root, or explicitly move the generated docs/index.html aside if discarding the old publication baseline; source capture cannot restore a missing generated bundle"
        ))
    };
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            missing_bindings()
        } else {
            io_error(error)
        }
    })?;
    if !metadata.is_file() || metadata.len() > super::check::PORTABLE_CACHE_MAX_BYTES {
        return Err(invalid(
            "documentation bindings are not a bounded regular file",
        ));
    }
    let raw = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            missing_bindings()
        } else {
            io_error(error)
        }
    })?;
    if raw.len() as u64 > super::check::PORTABLE_CACHE_MAX_BYTES {
        return Err(invalid(
            "documentation bindings grew beyond their record budget",
        ));
    }
    let mut binding: Bindings = serde_yaml_ng::from_slice(&raw).map_err(|error| invalid(format!(
        "DOCS_REINDEX_REQUIRED: unsupported documentation bindings ({error}); initialize a fresh documentation root and run docs check")))?;
    let receipt = BaselineReceipt {
        bundle: id.into(),
        index_digest: canonical::hash_bytes(index_text.as_bytes()),
        bindings_digest: canonical::hash_bytes(&raw),
    };
    if binding.schema != "codeclew-documentation-bindings/1.4" {
        return Err(invalid(
            "DOCS_REINDEX_REQUIRED: unsupported documentation bindings schema; initialize a fresh documentation root and run docs check",
        ));
    }
    super::review::resolve_influence_scopes(
        &mut binding.accepted_versions,
        &binding.influence_scopes,
    )?;
    for (id, fragment) in &binding.fragments {
        let operation = id.split('/').take(2).collect::<Vec<_>>().join("/");
        if let Some(version) = binding.accepted_versions.get(&operation)
            && fragment.influence_scope.as_ref() != Some(&version.influence.scope)
        {
            return Err(invalid(
                "accepted fragment does not match its influence scope",
            ));
        }
        if fragment
            .influence_scope
            .as_ref()
            .is_some_and(|scope| !binding.influence_scopes.contains_key(scope))
        {
            return Err(invalid("fragment influence scope is missing"));
        }
    }
    expand_shared(&mut binding)?;
    for (id, observation) in &binding.observations {
        if id != &observation.id || digest(&observation.normalized)? != observation.digest {
            return Err(invalid("portable baseline observation digest is invalid"));
        }
    }
    for source in binding.retained_sources.values() {
        if canonical::hash_bytes(source.text.as_bytes()) != source.text_digest {
            return Err(invalid("retained source digest is invalid"));
        }
    }
    for fragment in binding.fragments.values() {
        if !fragment.content.is_null() && digest(&fragment.content)? != fragment.content_digest {
            return Err(invalid("retained fragment content digest is invalid"));
        }
        if let Some(evidence) = &fragment.evidence {
            for (id, observation) in &evidence.observations {
                if id != &observation.id || digest(&observation.normalized)? != observation.digest {
                    return Err(invalid("retained section observation digest is invalid"));
                }
            }
            for source in evidence.sources.values() {
                if canonical::hash_bytes(source.text.as_bytes()) != source.text_digest {
                    return Err(invalid("retained section source digest is invalid"));
                }
            }
        }
        let observations = fragment
            .evidence
            .as_ref()
            .map(|e| &e.observations)
            .unwrap_or(&binding.observations);
        if fragment
            .dependencies
            .iter()
            .any(|(id, value)| observations.get(id).is_none_or(|o| &o.digest != value))
        {
            return Err(invalid(
                "portable baseline is missing a normalized dependency observation",
            ));
        }
    }
    // Renderer identity records how the previous presentation was produced.
    // Evidence compatibility is governed by the bindings schema and validated
    // evidence below it, not by a presentation-only version change. Preserve
    // this identity verbatim; newly rendered bundles use the current renderer.
    if binding.renderer.is_empty()
        || binding.renderer.len() > 128
        || !binding
            .renderer
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'/' | b'_' | b'-'))
    {
        return Err(invalid(
            "documentation renderer provenance identity is invalid",
        ));
    }
    let root_matches = binding.output_hashes.get("root-overview.html")
        == Some(&canonical::hash_bytes(index_text.as_bytes()));
    if !root_matches {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "generated overview was edited; preserve the edit as manual prose before regeneration",
        ));
    }
    Ok(Some((receipt, binding)))
}

pub fn verify_outputs(repo: &Repository, id: &str, binding: &Bindings) -> Result<(), ClewError> {
    for (path, expected) in &binding.output_hashes {
        store::relative(path)?;
        let path = repo.path(&format!("docs/generated/{id}/{path}"))?;
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        // Budget rejection is a distinct typed outcome from tampering: an
        // output that spilled past the portable budget is a resource-bound
        // failure (mirroring the pre-flight gate in render::commit_bundle),
        // not a manual edit/digest conflict.
        if metadata.len() > super::check::PORTABLE_CACHE_MAX_BYTES {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "generated documentation output exceeds its portable record budget",
            ));
        }
        if !metadata.is_file()
            || canonical::hash_bytes(&fs::read(&path).map_err(io_error)?) != *expected
        {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "generated documentation was edited or removed; preserve manual changes before regeneration",
            ));
        }
    }
    Ok(())
}

fn dependency_changes(dependencies: &BTreeMap<String, String>, checked: &Check) -> Vec<Value> {
    dependencies.iter().filter_map(|(dependency, previous)| {
        match checked.dependencies.get(dependency) {
            None => Some(json!({"reason":"DEPENDENCY_UNAVAILABLE","dependency":dependency,"before":previous,"after":Value::Null})),
            Some(current) if &current.digest != previous => Some(json!({"reason":match current.kind.as_str(){"DECLARED_INTERACTION"=>"DECLARATION_CHANGED","ENTRYPOINT"=>"ROUTE_OR_REGISTRATION_CHANGED","CONTRACT_OPERATION"|"CONTRACT"=>"CONTRACT_CHANGED","SCENARIO_SELECTION"=>"SCENARIO_SELECTION_CHANGED",_=>"SUPPORTED_BEHAVIOR_CHANGED"},"dependency":dependency,"before":previous,"after":current.digest})),
            _ => None,
        }
    }).collect()
}

pub fn freshness(old: Option<&Bindings>, checked: &Check) -> Value {
    let Some(old) = old else {
        return json!({"status":"UNRESOLVED","reasons":["MISSING_BASELINE"],"affected":[],"unaffected":[],"linkChanges":[],"catalogueChanges":[]});
    };
    let mut affected = Vec::new();
    let mut unaffected = Vec::new();
    let mut links = Vec::new();
    let mut catalogue = Vec::new();
    let sources = checked.sources();
    // Each complete scope is compared once. Fragment reports reference the
    // shared delta instead of multiplying a large changed set by every claim.
    let mut influence_changes = BTreeMap::new();
    for (scope, dependencies) in &old.influence_scopes {
        let changes = if digest(dependencies).ok().as_ref() != Some(scope) {
            vec![json!({"reason":"INFLUENCE_SCOPE_CORRUPT"})]
        } else {
            dependency_changes(&dependencies.dependencies, checked)
        };
        if !changes.is_empty() {
            influence_changes.insert(scope.clone(), changes);
        }
    }
    for (id, fragment) in &old.fragments {
        let mut reasons = Vec::new();
        if old.extractor != EXTRACTOR {
            reasons.push(json!({"reason":"EVIDENCE_VERSION_CHANGED"}));
        }
        reasons.extend(dependency_changes(&fragment.dependencies, checked));
        if let Some(scope) = &fragment.influence_scope {
            if !old.influence_scopes.contains_key(scope) {
                reasons.push(json!({"reason":"INFLUENCE_SCOPE_UNAVAILABLE","scope":scope}));
            } else if influence_changes.contains_key(scope) {
                reasons.push(json!({"reason":"INFLUENCE_SCOPE_CHANGED","scope":scope}));
            }
        }
        for (source_id, old_source) in &fragment.sources {
            if let Some(source) = sources.get(source_id)
                && (old_source["file"] != source.file
                    || old_source["startLine"] != source.start_line
                    || old_source["endLine"] != source.end_line
                    || old_source["textDigest"] != source.text_digest
                    || old_source["url"] != json!(source.url))
            {
                links.push(json!({"fragment":id,"source":source_id,"reason":"SOURCE_PRESENTATION_OR_REVISION_CHANGED","rewriteNarrative":false}));
            }
        }
        if reasons.is_empty() {
            unaffected.push(id.clone());
        } else {
            affected.push(json!({"fragment":id,"subject":fragment.subject,"reasons":reasons,"requiredAction":"REVIEW_AND_REGENERATE_FRAGMENT"}));
        }
    }
    for (service, evidence) in &checked.services {
        let old_ids: BTreeSet<_> = old
            .catalogues
            .get(service)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let new_ids: BTreeSet<_> = evidence.entrypoints.iter().map(|e| e.id.clone()).collect();
        for id in new_ids.difference(&old_ids) {
            catalogue.push(json!({"service":service,"entrypoint":id,"reason":"ENTRYPOINT_ADDED","requiredAction":"DOCUMENT_OR_RECORD_EXPLICIT_GAP"}));
        }
        for id in old_ids.difference(&new_ids) {
            catalogue
                .push(json!({"service":service,"entrypoint":id,"reason":"ENTRYPOINT_REMOVED"}));
        }
        if old.coverage.get(service)
            != Some(&json!({"coverage":evidence.coverage,"boundaries":evidence.boundaries}))
        {
            catalogue.push(json!({"service":service,"reason":"ANALYSIS_COVERAGE_CHANGED"}));
        }
    }
    let incomplete_source = checked
        .services
        .values()
        .any(|e| e.extractor == SOURCE_EXTRACTOR && e.coverage != "SYNTAX");
    let status = if !checked.unresolved.is_empty() || incomplete_source {
        "UNRESOLVED"
    } else if affected.is_empty() && catalogue.is_empty() {
        "CURRENT"
    } else if unaffected.is_empty() {
        "STALE"
    } else {
        "PARTIALLY_STALE"
    };
    json!({"status":status,"influenceChanges":influence_changes,"affected":affected,"unaffected":unaffected,"linkChanges":links,"catalogueChanges":catalogue,"scope":"Recorded dependencies and supported static analysis only","sourceAuthorities":checked.source_authorities(),"incompleteSource":incomplete_source,"unresolved":checked.unresolved})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::check;

    #[test]
    fn scope_retention_removes_last_replaced_owner_but_preserves_mixed_siblings() {
        let mut binding = old(&current());
        let mut scope_ids = Vec::new();
        for owner in ["original", "sibling", "accepted-only", "replacement"] {
            let scope = super::super::review::InfluenceScope {
                dependencies: BTreeMap::from([(owner.into(), "sha256:recorded".into())]),
                declarations: BTreeMap::new(),
            };
            let id = digest(&scope).unwrap();
            binding.influence_scopes.insert(id.clone(), scope);
            scope_ids.push(id);
        }
        let [original, sibling, accepted, replacement]: [String; 4] = scope_ids.try_into().unwrap();
        binding
            .fragments
            .get_mut("contract-row")
            .unwrap()
            .influence_scope = Some(original.clone());
        binding
            .fragments
            .get_mut("unrelated-summary")
            .unwrap()
            .influence_scope = Some(original.clone());
        binding
            .fragments
            .get_mut("transport-edge")
            .unwrap()
            .influence_scope = Some(sibling.clone());
        let version = serde_json::from_value(json!({
            "schema":"codeclew-documentation-accepted-version/1.2",
            "work":"w", "proposal":"p", "invocation":null,
            "reviewDigest":null, "reviewerDriverDigest":null,
            "evidenceDigest":"e", "readDigest":"r", "operationDigest":"o",
            "verification":"UNASSESSED", "limitations":[], "sourceRevisions":{},
            "influence":{"scope":accepted}, "externalFingerprint":"f",
            "previousNarrativeDigest":digest(&Option::<Narrative>::None).unwrap(),
            "externalRequest":{"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers","externalInputs":[]}
        })).unwrap();
        binding
            .accepted_versions
            .insert("service:other/overview".into(), version);
        binding
            .fragments
            .get_mut("contract-row")
            .unwrap()
            .influence_scope = Some(replacement.clone());
        prune_influence_scopes(&mut binding);
        assert_eq!(binding.influence_scopes.len(), 4);
        binding
            .fragments
            .get_mut("unrelated-summary")
            .unwrap()
            .influence_scope = Some(replacement.clone());
        prune_influence_scopes(&mut binding);
        assert!(!binding.influence_scopes.contains_key(&original));
        for retained in [&sibling, &accepted, &replacement] {
            assert!(binding.influence_scopes.contains_key(retained));
        }
        binding.accepted_versions.clear();
        compact(&mut binding);
        assert!(!binding.influence_scopes.contains_key(&accepted));
        assert_eq!(binding.influence_scopes.len(), 2);
    }

    #[test]
    fn shared_influence_preserves_full_invalidation_and_mixed_versions() {
        let mut checked = current();
        let mut binding = old(&checked);
        let scope = super::super::review::InfluenceScope {
            dependencies: checked
                .dependencies
                .iter()
                .map(|(id, o)| (id.clone(), o.digest.clone()))
                .collect(),
            declarations: BTreeMap::new(),
        };
        let before = digest(&scope).unwrap();
        binding.influence_scopes.insert(before.clone(), scope);
        let mut fragment = binding.fragments["contract-row"].clone();
        fragment.influence_scope = Some(before.clone());
        binding.fragments.clear();
        for i in 0..128 {
            binding
                .fragments
                .insert(format!("claim-{i}"), fragment.clone());
        }
        assert!(
            freshness(Some(&binding), &checked)["affected"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        // The changed fact is outside every fragment's local claim selection.
        assert!(!fragment.dependencies.contains_key("unrelated"));
        let observation = checked.dependencies.get_mut("unrelated").unwrap();
        observation.normalized = json!({"changed":true});
        observation.digest = digest(&observation.normalized).unwrap();
        let current_scope = super::super::review::InfluenceScope {
            dependencies: checked
                .dependencies
                .iter()
                .map(|(id, o)| (id.clone(), o.digest.clone()))
                .collect(),
            declarations: BTreeMap::new(),
        };
        let after = digest(&current_scope).unwrap();
        binding
            .influence_scopes
            .insert(after.clone(), current_scope);
        fragment.influence_scope = Some(after);
        binding.fragments.insert("new-claim".into(), fragment);
        let report = freshness(Some(&binding), &checked);
        assert_eq!(report["affected"].as_array().unwrap().len(), 128);
        assert_eq!(report["unaffected"], json!(["new-claim"]));
        assert_eq!(report["influenceChanges"].as_object().unwrap().len(), 1);
        assert_eq!(
            report["influenceChanges"][&before]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            report["affected"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["reasons"].as_array().unwrap().len() == 1)
        );
        binding.influence_scopes.remove(&before);
        assert_eq!(
            freshness(Some(&binding), &checked)["affected"]
                .as_array()
                .unwrap()
                .len(),
            128
        );
    }

    #[test]
    fn baseline_requires_current_format_and_preserves_current_portable_evidence() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Format boundary").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let bundle = "a".repeat(64);
        let path = repo
            .path(&format!("docs/generated/{bundle}/bindings.json"))
            .unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            repo.path("docs/index.html").unwrap(),
            format!("<!-- codeclew-bundle {bundle} -->\n"),
        )
        .unwrap();
        let mut binding = old(&current());
        binding.output_hashes.insert(
            "root-overview.html".into(),
            canonical::hash_bytes(format!("<!-- codeclew-bundle {bundle} -->\n").as_bytes()),
        );
        compact(&mut binding);
        let current = serde_json::to_value(binding).unwrap();
        fs::write(&path, serde_json::to_vec(&current).unwrap()).unwrap();
        assert_eq!(
            baseline(&repo).unwrap().unwrap().1.retained_sources.len(),
            1
        );
        let mut variants = Vec::new();
        for version in ["1.0", "1.1", "1.2", "1.3"] {
            let mut outdated = current.clone();
            outdated["schema"] = json!(format!("codeclew-documentation-bindings/{version}"));
            variants.push(outdated);
        }
        let mut heavy = current.clone();
        heavy["heavy"] = Value::Null;
        variants.push(heavy);
        let mut missing = current.clone();
        missing.as_object_mut().unwrap().remove("sectionStates");
        variants.push(missing);
        for variant in variants {
            let bytes = serde_json::to_vec(&variant).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(
                baseline(&repo)
                    .unwrap_err()
                    .message
                    .contains("DOCS_REINDEX_REQUIRED")
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::write(&path, serde_json::to_vec(&current).unwrap()).unwrap();
        assert!(baseline(&repo).is_ok());
    }

    #[test]
    fn renderer_provenance_does_not_invalidate_supported_evidence() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Renderer provenance").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let bundle = "b".repeat(64);
        let path = repo
            .path(&format!("docs/generated/{bundle}/bindings.json"))
            .unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let index = format!("<!-- codeclew-bundle {bundle} -->\n");
        fs::write(repo.path("docs/index.html").unwrap(), &index).unwrap();
        let checked = current();
        let mut binding = old(&checked);
        binding.renderer = "codeclew-documentation-html/1.14".into();
        binding.output_hashes.insert(
            "root-overview.html".into(),
            canonical::hash_bytes(index.as_bytes()),
        );
        compact(&mut binding);
        let bytes = serde_json::to_vec(&binding).unwrap();
        fs::write(&path, &bytes).unwrap();
        let (_, restored) = baseline(&repo).unwrap().unwrap();
        assert_eq!(restored.renderer, binding.renderer);
        assert_eq!(freshness(Some(&restored), &checked)["status"], "CURRENT");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        for identity in [
            "",
            " ",
            "renderer with spaces",
            "<script>",
            "renderer\n",
            &"x".repeat(129),
        ] {
            binding.renderer = identity.into();
            let bytes = serde_json::to_vec(&binding).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(
                baseline(&repo)
                    .unwrap_err()
                    .message
                    .contains("renderer provenance")
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn portable_evidence_sharing_preserves_mixed_revisions_and_rejects_missing_records() {
        let mut binding = old(&current());
        let original = serde_json::to_value(&binding.fragments).unwrap();
        // The same logical source can have a newer occurrence in the global
        // table while an old fragment still retains its exact prior text.
        binding
            .retained_sources
            .get_mut("inventory-source")
            .unwrap()
            .revision = "2".repeat(40);
        compact(&mut binding);
        assert!(
            binding.fragments.values().any(|f| !f
                .evidence
                .as_ref()
                .unwrap()
                .shared_observations
                .is_empty())
        );
        assert!(binding.fragments.values().all(|f| {
            f.evidence
                .as_ref()
                .unwrap()
                .sources
                .contains_key("inventory-source")
        }));
        assert!(
            binding
                .fragments
                .values()
                .all(|f| f.dependencies_from_evidence && f.dependencies.is_empty())
        );
        let mut ambiguous = binding.clone();
        ambiguous
            .fragments
            .values_mut()
            .next()
            .unwrap()
            .dependencies
            .insert("unexpected".into(), "digest".into());
        assert!(expand_shared(&mut ambiguous).is_err());
        let mut absent = binding.clone();
        absent.fragments.values_mut().next().unwrap().evidence = None;
        assert!(expand_shared(&mut absent).is_err());
        let mut damaged = binding.clone();
        damaged.observations.clear();
        assert!(expand_shared(&mut damaged).is_err());
        let encoded = serde_json::to_vec(&binding).unwrap();
        let mut decoded: Bindings = serde_json::from_slice(&encoded).unwrap();
        expand_shared(&mut decoded).unwrap();
        assert_eq!(serde_json::to_value(decoded.fragments).unwrap(), original);

        let mut matching = old(&current());
        let original = serde_json::to_value(&matching.fragments).unwrap();
        compact(&mut matching);
        assert!(
            matching
                .fragments
                .values()
                .all(|f| f.evidence.as_ref().unwrap().sources.is_empty())
        );
        expand_shared(&mut matching).unwrap();
        assert_eq!(serde_json::to_value(matching.fragments).unwrap(), original);
    }

    fn current() -> Check {
        let source = Source {
            id: "inventory-source".into(),
            service: "inventory".into(),
            revision: "1".repeat(40),
            file: "Controller.java".into(),
            start_line: 1,
            end_line: 1,
            text: "return reserve();".into(),
            text_digest: canonical::hash_bytes(b"return reserve();"),
            evidence_digest: canonical::hash_bytes(b"evidence"),
            authority: "EXACT_SNAPSHOT_TEXT".into(),
            occurrence: None,
            url: None,
        };
        let observation = |id: &str, kind: &str, value: Value| Observation {
            id: id.into(),
            kind: kind.into(),
            service: if kind == "DECLARED_INTERACTION" {
                ""
            } else {
                "inventory"
            }
            .into(),
            symbol: id.into(),
            digest: digest(&value).unwrap(),
            normalized: value,
            source_ids: vec![source.id.clone()],
        };
        let rows = BTreeMap::from([
            (
                "route".into(),
                observation("route", "ENTRYPOINT", json!({"path":"/reservations"})),
            ),
            (
                "unrelated".into(),
                observation("unrelated", "SYMBOL", json!({"value":1})),
            ),
            (
                "interaction:reserve".into(),
                observation(
                    "interaction:reserve",
                    "DECLARED_INTERACTION",
                    json!({"to":"inventory"}),
                ),
            ),
        ]);
        let evidence = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "inventory".into(),
            revision: "1".repeat(40),
            service_digest: "digest".into(),
            extractor: EXTRACTOR.into(),
            runtime_mode: "DEVELOPMENT".into(),
            coverage: "PARTIAL".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: rows,
            sources: BTreeMap::from([(source.id.clone(), source)]),
            contracts: BTreeMap::new(),
        };
        check::assemble(
            "input".into(),
            BTreeMap::from([("inventory".into(), evidence)]),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap()
    }
    fn old(checked: &Check) -> Bindings {
        let source = vec!["inventory-source".to_owned()];
        let fragments = BTreeMap::from([
            (
                "contract-row".into(),
                fragment(
                    "service:inventory",
                    &"Contract",
                    &["route".into()],
                    &source,
                    checked,
                )
                .unwrap(),
            ),
            (
                "transport-edge".into(),
                fragment(
                    "scenario:checkout",
                    &"Reserve",
                    &["route".into(), "interaction:reserve".into()],
                    &source,
                    checked,
                )
                .unwrap(),
            ),
            (
                "unrelated-summary".into(),
                fragment(
                    "scenario:other",
                    &"Unchanged",
                    &["unrelated".into()],
                    &source,
                    checked,
                )
                .unwrap(),
            ),
        ]);
        Bindings {
            documentation_language: None,
            influence_scopes: BTreeMap::new(),
            section_states: BTreeMap::new(),
            target_revisions: BTreeMap::new(),
            update_failures: BTreeMap::new(),
            accepted_versions: BTreeMap::new(),
            schema: "codeclew-documentation-bindings/1.4".into(),
            input_digest: "input".into(),
            renderer: RENDERER.into(),
            extractor: EXTRACTOR.into(),
            revisions: BTreeMap::from([("inventory".into(), "1".repeat(40))]),
            coverage: BTreeMap::from([(
                "inventory".into(),
                json!({"coverage":"PARTIAL","boundaries":[]}),
            )]),
            catalogues: BTreeMap::from([("inventory".into(), vec![])]),
            fragments,
            observations: checked.dependencies.clone(),
            narratives: BTreeMap::new(),
            output_hashes: BTreeMap::new(),
            retained_sources: checked.sources(),
        }
    }
    #[test]
    fn route_changes_affect_exact_dependants_and_leave_other_prose_current() {
        let mut checked = current();
        let old = old(&checked);
        let route = checked.dependencies.get_mut("route").unwrap();
        route.normalized = json!({"path":"/stock"});
        route.digest = digest(&route.normalized).unwrap();
        let report = freshness(Some(&old), &checked);
        assert_eq!(report["status"], "PARTIALLY_STALE");
        assert_eq!(report["unaffected"], json!(["unrelated-summary"]));
        assert_eq!(report["affected"].as_array().unwrap().len(), 2);
        assert_eq!(
            report["affected"][0]["reasons"][0]["reason"],
            "ROUTE_OR_REGISTRATION_CHANGED"
        );
    }
    #[test]
    fn line_movement_changes_source_presentation_without_staling_narrative() {
        let mut checked = current();
        let old = old(&checked);
        let source = checked
            .services
            .get_mut("inventory")
            .unwrap()
            .sources
            .get_mut("inventory-source")
            .unwrap();
        source.start_line = 9;
        source.end_line = 9;
        source.revision = "2".repeat(40);
        let report = freshness(Some(&old), &checked);
        assert_eq!(report["status"], "CURRENT");
        assert!(report["affected"].as_array().unwrap().is_empty());
        assert!(!report["linkChanges"].as_array().unwrap().is_empty());
    }
    #[test]
    fn declaration_only_edit_and_missing_evidence_never_look_unchanged() {
        let mut checked = current();
        let old = old(&checked);
        let declaration = checked.dependencies.get_mut("interaction:reserve").unwrap();
        declaration.normalized = json!({"to":"other"});
        declaration.digest = digest(&declaration.normalized).unwrap();
        let report = freshness(Some(&old), &checked);
        assert_eq!(report["affected"][0]["fragment"], "transport-edge");
        assert_eq!(
            report["affected"][0]["reasons"][0]["reason"],
            "DECLARATION_CHANGED"
        );
        checked
            .unresolved
            .insert("inventory".into(), json!({"reason":"MISSING_HISTORY"}));
        assert_eq!(freshness(Some(&old), &checked)["status"], "UNRESOLVED");
        assert_eq!(freshness(None, &checked)["status"], "UNRESOLVED");
    }
    #[test]
    fn module_rule_change_invalidates_bound_content_without_source_changes() {
        let mut checked = current();
        let normalized = json!({"implementationDigest":"old-rules","availability":"AVAILABLE"});
        checked.dependencies.insert(
            "module-scope".into(),
            Observation {
                id: "module-scope".into(),
                kind: "MODULE_SCOPE".into(),
                service: "inventory".into(),
                symbol: "module-scope".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
        let baseline = old(&checked);
        assert!(
            baseline.fragments["contract-row"]
                .dependencies
                .contains_key("module-scope")
        );
        let sources = checked.sources();
        let revisions = checked
            .services
            .iter()
            .map(|(s, e)| (s.clone(), e.revision.clone()))
            .collect::<BTreeMap<_, _>>();
        let scope = checked.dependencies.get_mut("module-scope").unwrap();
        scope.normalized["implementationDigest"] = json!("new-rules");
        scope.digest = digest(&scope.normalized).unwrap();
        let report = freshness(Some(&baseline), &checked);
        assert!(
            report["affected"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["fragment"] == "contract-row")
        );
        assert_eq!(sources, checked.sources());
        assert_eq!(
            revisions,
            checked
                .services
                .iter()
                .map(|(s, e)| (s.clone(), e.revision.clone()))
                .collect()
        );
    }

    #[test]
    fn unsupported_evidence_version_requires_review() {
        let checked = current();
        let mut old = old(&checked);
        old.extractor = "older".into();
        assert_eq!(freshness(Some(&old), &checked)["status"], "STALE");
    }
}
