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
    pub subject: String,
    pub content_digest: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub content: Value,
    pub dependencies: BTreeMap<String, String>,
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
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bindings {
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
    #[serde(default)]
    pub retained_sources: BTreeMap<String, Source>,
    /// Missing in legacy bundles; it never implies an accepted meaning review.
    #[serde(default)]
    pub section_states: BTreeMap<String, SectionState>,
    #[serde(default)]
    pub target_revisions: BTreeMap<String, Option<String>>,
    #[serde(default)]
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
                "DOMAIN_ENTITY" | "ENTITY_SCOPE" | "NOTE_SCOPE" | "NOTE_ASSOCIATION"
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
                        "SOURCE_SCOPE" | "CONTRACT_SCOPE" | "ENTITY_SCOPE" | "NOTE_SCOPE"
                    ) && d.service == service
                })
                .map(|d| d.id.clone()),
        );
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
        subject: subject.into(), content_digest: digest(value)?,
        content: serde_json::to_value(value).map_err(io_error)?,
        dependencies: observations.iter().map(|(id, o)| (id.clone(), o.digest.clone())).collect(),
        sources: retained.iter().map(|(id, source)| (id.clone(), json!({"revision":source.revision,"file":source.file,"startLine":source.start_line,"endLine":source.end_line,"textDigest":source.text_digest,"url":source.url}))).collect(),
        evidence: Some(FragmentEvidence {
            revisions: services.iter().filter_map(|id| checked.services.get(*id).map(|e| ((*id).to_owned(),e.revision.clone()))).collect(),
            observations, sources: retained,
        }),
    })
}

pub fn baseline(repo: &Repository) -> Result<Option<(String, Bindings)>, ClewError> {
    let index = repo.path("docs/index.html")?;
    if !index.exists() {
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(&index).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
        return Err(invalid("documentation index is not a bounded regular file"));
    }
    let index_text = fs::read_to_string(&index).map_err(io_error)?;
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
    let binding: Bindings = store::read(
        &repo.path(&format!("docs/generated/{id}/bindings.json"))?,
        64 * 1024 * 1024,
    )?;
    if !matches!(
        binding.schema.as_str(),
        "codeclew-documentation-bindings/1.0" | "codeclew-documentation-bindings/1.1"
    ) {
        return Err(invalid("unsupported documentation bindings schema"));
    }
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
    // The portable overview resolves service/scenario links within its bundle;
    // the root index prefixes those same links with the immutable bundle path.
    let comparison_text = if matches!(
        binding.renderer.as_str(),
        "codeclew-documentation-html/1.2"
            | "codeclew-documentation-html/1.3"
            | "codeclew-documentation-html/1.4"
            | "codeclew-documentation-html/1.5"
            | "codeclew-documentation-html/1.6"
            | "codeclew-documentation-html/1.7"
            | "codeclew-documentation-html/1.8"
    ) {
        if index_text.contains("href=\"services/") || index_text.contains("href=\"scenarios/") {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "generated root overview links were edited",
            ));
        }
        index_text.replace(&format!("href=\"generated/{id}/"), "href=\"")
    } else {
        index_text.clone()
    };
    let root_hash = canonical::hash_bytes(comparison_text.as_bytes());
    if binding.output_hashes.get("overview.html") != Some(&root_hash) {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "generated overview was edited; preserve the edit as manual prose before regeneration",
        ));
    }
    Ok(Some((id.into(), binding)))
}

pub fn verify_outputs(repo: &Repository, id: &str, binding: &Bindings) -> Result<(), ClewError> {
    for (path, expected) in &binding.output_hashes {
        store::relative(path)?;
        let path = repo.path(&format!("docs/generated/{id}/{path}"))?;
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.is_file()
            || metadata.len() > 64 * 1024 * 1024
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

pub fn freshness(old: Option<&Bindings>, checked: &Check) -> Value {
    let Some(old) = old else {
        return json!({"status":"UNRESOLVED","reasons":["MISSING_BASELINE"],"affected":[],"unaffected":[],"linkChanges":[],"catalogueChanges":[]});
    };
    let mut affected = Vec::new();
    let mut unaffected = Vec::new();
    let mut links = Vec::new();
    let mut catalogue = Vec::new();
    let sources = checked.sources();
    for (id, fragment) in &old.fragments {
        let mut reasons = Vec::new();
        if old.renderer != RENDERER || old.extractor != EXTRACTOR {
            reasons.push(json!({"reason":"EVIDENCE_VERSION_CHANGED"}));
        }
        for (dependency, previous) in &fragment.dependencies {
            match checked.dependencies.get(dependency){
                None=>reasons.push(json!({"reason":"DEPENDENCY_UNAVAILABLE","dependency":dependency,"before":previous,"after":Value::Null})),
                Some(current) if &current.digest!=previous=>reasons.push(json!({"reason":match current.kind.as_str(){"DECLARED_INTERACTION"=>"DECLARATION_CHANGED","ENTRYPOINT"=>"ROUTE_OR_REGISTRATION_CHANGED","CONTRACT_OPERATION"|"CONTRACT"=>"CONTRACT_CHANGED","SCENARIO_SELECTION"=>"SCENARIO_SELECTION_CHANGED",_=>"SUPPORTED_BEHAVIOR_CHANGED"},"dependency":dependency,"before":previous,"after":current.digest})),
                _=>{},
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
    json!({"status":status,"affected":affected,"unaffected":unaffected,"linkChanges":links,"catalogueChanges":catalogue,"scope":"Recorded dependencies and supported static analysis only","incompleteSource":incomplete_source,"unresolved":checked.unresolved})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::check;

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
            section_states: BTreeMap::new(),
            target_revisions: BTreeMap::new(),
            update_failures: BTreeMap::new(),
            accepted_versions: BTreeMap::new(),
            schema: "codeclew-documentation-bindings/1.0".into(),
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
