//! Read-only comparison of a saved approval's selected evidence, never its meaning.
use super::{
    agent_jobs::operation_draft_review,
    check::Check,
    digest, invalid,
    model::{Observation, Source},
    store::Repository,
    work::{self, Work},
};
use crate::{canonical, error::ClewError};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn run(
    repo: &Repository,
    id: &str,
    review_run: &str,
    snapshot: &str,
) -> Result<Value, ClewError> {
    let work = work::load(repo, id)?;
    let selected = operation_draft_review::load_approved_answer(repo, &work, review_run)?;
    let current = Check::load_snapshot(repo, snapshot)?;
    let delta = compare(&work, &selected.audit, &current)?;
    Ok(json!({
        "schema":"codeclew-saved-answer-context/1.0", "work":work.id,"reviewRun":review_run,
        "subject":work.subject,"question":work.request.question,"rootDeclaration":work.request.root_declaration,
        "entrypoint":work.request.entrypoint,"profile":work.request.context_profile,
        "savedSnapshot":work.snapshot,"comparedSnapshot":snapshot,"packetDigest":selected.packet["packetDigest"],
        "savedContextDigest":work.checked.context_digest,"comparedContextDigest":current.context_digest,
        "meaningReview":"MODEL_APPROVED_AGAINST_SAVED_PACKET", "capturedContext":delta,
        "authority":"SELECTED_SAVED_PACKET_EVIDENCE_COMPARISON_NOT_NEW_MEANING_REVIEW",
        "limitations":["CURRENT only describes matching selected captured context; historical model approval is unchanged.",
            "Human/imported maintained context remains unverified and UNASSESSED; this comparison does not assess its truth.",
            "Comparison is conservative on changed compiler pins; identities and unsupported coordinate drift are never remapped.",
            "This is comparison with the explicit Check, not a new capture or a claim about live runtime."],
        "captures":0,"agentInvocations":0,"writes":0
    }))
}

pub(super) fn compare(work: &Work, audit: &Value, current: &Check) -> Result<Value, ClewError> {
    let records = audit["records"]
        .as_array()
        .ok_or_else(|| invalid("saved answer audit records are unavailable"))?;
    let sources = current.sources();
    let mut changed_pins = Vec::new();
    let mut changed_sources = Vec::new();
    let mut missing = Vec::new();
    let mut links = Vec::new();
    let mut services = BTreeSet::new();
    let mut source_count = 0;
    let mut dependency_count = 0;
    for row in records {
        match row["kind"].as_str() {
            Some("SOURCE") => {
                source_count += 1;
                let old: Source = serde_json::from_value(row["row"]["record"].clone())
                    .map_err(super::io_error)?;
                services.insert(old.service.clone());
                let Some(new) = sources.get(&old.id) else {
                    missing.push(
                        json!({"kind":"SOURCE","id":old.id,"reason":"SELECTED_SOURCE_UNAVAILABLE"}),
                    );
                    continue;
                };
                if old.id != new.id
                    || old.service != new.service
                    || canonical::hash_bytes(old.text.as_bytes()) != old.text_digest
                    || canonical::hash_bytes(new.text.as_bytes()) != new.text_digest
                    || old.authority != new.authority
                {
                    missing.push(json!({"kind":"SOURCE","id":old.id,"reason":"SOURCE_BINDING_UNSUPPORTED_OR_INVALID"}));
                    continue;
                }
                if old.file != new.file
                    || old.text_digest != new.text_digest
                    || old.text != new.text
                {
                    changed_sources.push(json!({"id":old.id,"reason":"SELECTED_SOURCE_CONTENT_OR_FILE_CHANGED","before":old.text_digest,"after":new.text_digest,"beforeFile":old.file,"afterFile":new.file}));
                } else if old.start_line != new.start_line
                    || old.end_line != new.end_line
                    || old.revision != new.revision
                    || old.url != new.url
                    || old.occurrence != new.occurrence
                    || old.evidence_digest != new.evidence_digest
                {
                    links.push(json!({"id":old.id,"reason":"SOURCE_RECEIPT_OR_PRESENTATION_CHANGED","beforeRevision":old.revision,"afterRevision":new.revision,"beforeLines":[old.start_line,old.end_line],"afterLines":[new.start_line,new.end_line],"beforeEvidenceDigest":old.evidence_digest,"afterEvidenceDigest":new.evidence_digest}));
                }
            }
            Some("DEPENDENCY") => {
                dependency_count += 1;
                let old: Observation = serde_json::from_value(row["row"]["record"].clone())
                    .map_err(super::io_error)?;
                if work.checked.services.contains_key(&old.service) {
                    services.insert(old.service.clone());
                }
                let Some(new) = current.dependencies.get(&old.id) else {
                    missing.push(json!({"kind":"DEPENDENCY","id":old.id,"reason":"SELECTED_COMPILER_OR_DECLARATION_PIN_UNAVAILABLE"}));
                    continue;
                };
                if old.id != new.id
                    || old.kind != new.kind
                    || old.service != new.service
                    || old.symbol != new.symbol
                    || old.source_ids != new.source_ids
                    || old.normalized["schema"] != new.normalized["schema"]
                    || (!old.normalized["scope"].is_null()
                        && old.normalized["scope"].as_str().is_none_or(str::is_empty))
                    || old.normalized["scope"] != new.normalized["scope"]
                    || digest(&old.normalized)? != old.digest
                    || digest(&new.normalized)? != new.digest
                {
                    missing.push(json!({"kind":"DEPENDENCY","id":old.id,"reason":"DEPENDENCY_IDENTITY_SCOPE_OR_BINDING_UNSUPPORTED"}));
                    continue;
                }
                if old.digest != new.digest {
                    changed_pins.push(json!({"id":old.id,"kind":old.kind,"reason":"SELECTED_DEPENDENCY_CHANGED","before":old.digest,"after":new.digest}));
                }
            }
            _ => {}
        }
    }
    if source_count == 0 || dependency_count == 0 || services.is_empty() {
        missing.push(json!({"reason":"SELECTED_SOURCE_DEPENDENCY_SCOPE_UNAVAILABLE"}));
    }
    for service in &services {
        let captured = current.source_inputs.as_ref().is_some_and(|inputs| {
            inputs.selected_services.contains(service)
                && !inputs.retained_services.contains(service)
        });
        if !captured || current.unresolved.contains_key(service) {
            missing
                .push(json!({"service":service,"reason":"COMPARED_SERVICE_NOT_FRESHLY_CAPTURED"}));
            continue;
        }
        match (work.checked.services.get(service), current.services.get(service)) {
            (Some(old), Some(new)) if old.service_digest != new.service_digest => {
                missing.push(json!({"service":service,"reason":"CAPTURED_SERVICE_DECLARATION_OR_ORIGIN_CHANGED", "before":old.service_digest,"after":new.service_digest}));
            },
            (Some(old), Some(new)) if old.extractor == new.extractor && old.runtime_mode == new.runtime_mode && old.coverage == new.coverage && old.boundaries == new.boundaries => {},
            _ => missing.push(json!({"service":service,"reason":"CAPTURE_AUTHORITY_OR_COVERAGE_UNAVAILABLE_OR_CHANGED"})),
        }
    }
    Ok(
        json!({"status":if !missing.is_empty(){"UNKNOWN"}else if !changed_pins.is_empty()||!changed_sources.is_empty(){"STALE"}else{"CURRENT"},
        "scope":"Complete selected packet SOURCE and DEPENDENCY records, not only answer citations or global Work influence",
        "selectedSourceCount":source_count,"selectedDependencyCount":dependency_count,"services":services,
        "changedPins":changed_pins,"changedSources":changed_sources,"missingPinsOrBindings":missing,"linkChanges":links}),
    )
}

/// Only the reuse path may interpret a dependency digest change as provenance.
/// Its complete, validated initial semantic projection must already be equal;
/// the known-ID answer-context command retains its strict digest policy.
pub(super) fn compare_with_verified_replay(
    work: &Work,
    audit: &Value,
    current: &Check,
    replay: &Value,
) -> Result<Value, ClewError> {
    let mut delta = compare(work, audit, current)?;
    let pins = delta["changedPins"].as_array().cloned().unwrap_or_default();
    if super::answer_reuse_projection::selected_provenance_only(&delta, replay) {
        delta["strictDependencyDigestChanges"] = json!(pins);
        let links = delta["linkChanges"].as_array_mut().unwrap();
        for mut pin in pins {
            pin["reason"] =
                json!("DEPENDENCY_PROVENANCE_CHANGED_WITH_VERIFIED_EQUAL_INITIAL_REPLAY");
            links.push(pin);
        }
        delta["changedPins"] = json!([]);
        delta["status"] = json!("CURRENT");
        delta["dependencyComparison"] = json!(
            "STRICT_IDENTITY_AND_DIGEST_VALIDATION_WITH_EQUAL_COMPLETE_INITIAL_SEMANTIC_REPLAY"
        );
    }
    Ok(delta)
}

#[cfg(test)]
mod tests {
    use super::super::{operation_packet, work::api_contract_tests};
    use super::*;

    fn fixture() -> (tempfile::TempDir, Repository, Work, Value) {
        // Host comparison fixtures use typed synthetic compiler records. They
        // qualify admission/deltas, not real compiler behavior or model meaning.
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Saved answer comparison fixture").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let mut work = api_contract_tests::endpoint_context_fixture();
        api_contract_tests::persist_operation_fixture(&repo, &mut work);
        let (_, audit) = operation_packet::build(&work).unwrap();
        (temp, repo, work, audit)
    }

    #[test]
    fn selected_body_and_uncited_branch_helper_pins_stale_but_unrelated_records_do_not() {
        let (_temp, _repo, work, audit) = fixture();
        assert_eq!(
            compare(&work, &audit, &work.checked).unwrap()["status"],
            "CURRENT"
        );
        let records = audit["records"].as_array().unwrap();
        let source_ids = records
            .iter()
            .filter(|row| row["kind"] == "SOURCE")
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(!source_ids.is_empty());
        for source_id in source_ids {
            let mut current = work.checked.clone();
            let service = current.services.get_mut("orders").unwrap();
            let source = service.sources.get_mut(source_id).unwrap();
            source
                .text
                .push_str("\nif (changedGuard) return changedHelper();");
            source.text_digest = canonical::hash_bytes(source.text.as_bytes());
            let result = compare(&work, &audit, &current).unwrap();
            assert_eq!(result["status"], "STALE", "{source_id}: {result}");
            assert_eq!(result["changedSources"].as_array().unwrap().len(), 1);
        }
        let dependencies = records
            .iter()
            .filter(|row| row["kind"] == "DEPENDENCY")
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(dependencies.len() > 1);
        // Change every selected dependency independently, including dependencies
        // not cited by an answer. A citation-only check would miss these deltas.
        for id in &dependencies {
            let mut current = work.checked.clone();
            let pin = current.dependencies.get_mut(*id).unwrap();
            pin.normalized["changedBranchOrHelper"] = json!(true);
            pin.digest = digest(&pin.normalized).unwrap();
            assert_eq!(
                compare(&work, &audit, &current).unwrap()["status"],
                "STALE",
                "{id}"
            );
        }
        let mut current = work.checked.clone();
        let mut unrelated = current.dependencies[dependencies[0]].clone();
        unrelated.id = "not-selected-in-author-packet".into();
        unrelated.normalized["changedOutsidePacket"] = json!(true);
        unrelated.digest = digest(&unrelated.normalized).unwrap();
        current.dependencies.insert(unrelated.id.clone(), unrelated);
        assert_eq!(
            compare(&work, &audit, &current).unwrap()["status"],
            "CURRENT"
        );
    }

    #[test]
    fn exact_content_and_compiler_pins_allow_link_revision_changes_without_new_approval() {
        let (_temp, _repo, work, audit) = fixture();
        let source_id = audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "SOURCE")
            .unwrap()["id"]
            .as_str()
            .unwrap();
        let mut current = work.checked.clone();
        let source = current
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut(source_id)
            .unwrap();
        source.start_line += 10;
        source.end_line += 10;
        source.revision = "new captured revision".into();
        source.url = Some("https://example.invalid/new-link".into());
        source.evidence_digest = "sha256:new-coordinate-receipt".into();
        let result = compare(&work, &audit, &current).unwrap();
        assert_eq!(result["status"], "CURRENT");
        assert_eq!(result["linkChanges"].as_array().unwrap().len(), 1);
        assert!(result["changedPins"].as_array().unwrap().is_empty());
        // Coordinates embedded inside a changed compiler pin are not erased or
        // heuristically remapped. Such a binding remains conservatively stale.
        let dep_id = audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "DEPENDENCY")
            .unwrap()["id"]
            .as_str()
            .unwrap();
        let pin = current.dependencies.get_mut(dep_id).unwrap();
        pin.normalized["changedCallSiteReceipt"] = json!("new");
        pin.digest = digest(&pin.normalized).unwrap();
        assert_eq!(compare(&work, &audit, &current).unwrap()["status"], "STALE");
    }

    #[test]
    fn different_captured_service_origin_is_unknown_despite_identical_ids_source_and_pins() {
        let (_temp, _repo, work, audit) = fixture();
        let mut current = work.checked.clone();
        let (declaration_digest, input_digest) = {
            let inputs = current.source_inputs.as_mut().unwrap();
            let declaration = inputs.inputs.services.get_mut("orders").unwrap();
            declaration.repository_id = "another-configured-repository".into();
            declaration.repository = "https://example.invalid/other/orders".into();
            let declaration_digest = digest(declaration).unwrap();
            inputs.input_digest = digest(&inputs.inputs).unwrap();
            (declaration_digest, inputs.input_digest.clone())
        };
        current.input_digest = input_digest;
        current.services.get_mut("orders").unwrap().service_digest = declaration_digest;
        assert_eq!(current.sources(), work.checked.sources());
        assert_eq!(current.dependencies, work.checked.dependencies);
        let result = compare(&work, &audit, &current).unwrap();
        assert_eq!(result["status"], "UNKNOWN");
        assert!(result["changedPins"].as_array().unwrap().is_empty());
        assert!(result["changedSources"].as_array().unwrap().is_empty());
        assert!(
            result["missingPinsOrBindings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason["reason"] == "CAPTURED_SERVICE_DECLARATION_OR_ORIGIN_CHANGED")
        );
        assert_eq!(
            compare(&work, &audit, &work.checked).unwrap()["status"],
            "CURRENT"
        );
    }

    #[test]
    fn missing_retained_only_scope_and_invalid_content_bindings_never_claim_current() {
        let (_temp, _repo, work, audit) = fixture();
        let dep_id = audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "DEPENDENCY")
            .unwrap()["id"]
            .as_str()
            .unwrap();
        let source_id = audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "SOURCE")
            .unwrap()["id"]
            .as_str()
            .unwrap();
        for mode in [
            "missing-dependency",
            "missing-source",
            "retained-service",
            "legacy-check",
            "scope",
            "source-join",
            "schema",
            "bad-digest",
            "source-hash",
            "coverage",
        ] {
            let mut current = work.checked.clone();
            match mode {
                "missing-dependency" => {
                    current.dependencies.remove(dep_id);
                }
                "missing-source" => {
                    current
                        .services
                        .get_mut("orders")
                        .unwrap()
                        .sources
                        .remove(source_id);
                }
                "retained-service" => {
                    let inputs = current.source_inputs.as_mut().unwrap();
                    inputs.selected_services.clear();
                    inputs.retained_services.insert("orders".into());
                }
                "legacy-check" => current.source_inputs = None,
                "scope" => {
                    let pin = current.dependencies.get_mut(dep_id).unwrap();
                    pin.normalized["scope"] = json!(":/foreign");
                    pin.digest = digest(&pin.normalized).unwrap();
                }
                "source-join" => {
                    current.dependencies.get_mut(dep_id).unwrap().source_ids =
                        vec!["foreign-source".into()]
                }
                "schema" => {
                    let pin = current.dependencies.get_mut(dep_id).unwrap();
                    pin.normalized["schema"] = json!("unsupported-version");
                    pin.digest = digest(&pin.normalized).unwrap();
                }
                "bad-digest" => {
                    current.dependencies.get_mut(dep_id).unwrap().digest = "sha256:forged".into()
                }
                "source-hash" => current
                    .services
                    .get_mut("orders")
                    .unwrap()
                    .sources
                    .get_mut(source_id)
                    .unwrap()
                    .text
                    .push('x'),
                "coverage" => {
                    current.services.get_mut("orders").unwrap().coverage = "UNRESOLVED".into()
                }
                _ => unreachable!(),
            }
            let result = compare(&work, &audit, &current).unwrap();
            assert_eq!(result["status"], "UNKNOWN", "{mode}: {result}");
            assert!(
                !result["missingPinsOrBindings"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
