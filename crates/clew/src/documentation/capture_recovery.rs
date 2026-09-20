//! Explicit recovery of saved captures without claiming current source authority.
use super::{
    bytes, cache,
    check::{self, SourceInputs},
    composition, digest, invalid,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub(super) fn run(root: &Path, captures: &[String]) -> Result<Value, ClewError> {
    if captures.is_empty() || captures.len() > store::MAX_RECORDS {
        return Err(invalid(
            "recovery requires a bounded explicit list of capture manifests",
        ));
    }
    let repo = Repository::open(root)?;
    let _lock = repo.lock()?;
    let inputs = repo.inputs()?;
    let input_digest = digest(&inputs)?;
    composition::validate_retained(&repo, &inputs)?;
    let mut manifests = BTreeMap::new();
    let mut evidence = BTreeMap::new();
    let mut provenance = BTreeMap::new();
    for name in captures {
        store::relative(name)?;
        if Path::new(name).components().count() != 1 || !name.ends_with(".json") {
            return Err(invalid(
                "capture must be a manifest basename within .codeclew/cache",
            ));
        }
        let path = repo.path(&format!(".codeclew/cache/{name}"))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)
            .map_err(super::io_error)?;
        let metadata = file.metadata().map_err(super::io_error)?;
        if !metadata.is_file() || metadata.len() > store::MAX_RECORD {
            return Err(invalid("capture manifest is not a bounded regular file"));
        }
        let mut raw = Vec::new();
        file.take(store::MAX_RECORD + 1)
            .read_to_end(&mut raw)
            .map_err(super::io_error)?;
        if raw.len() as u64 > store::MAX_RECORD {
            return Err(invalid("capture manifest grew beyond its bound"));
        }
        let manifest: cache::CaptureManifest = serde_json::from_slice(&raw)
            .map_err(|_| invalid("capture manifest violates its current JSON schema"))?;
        let service = inputs.services.get(&manifest.service).ok_or_else(|| {
            invalid("recovered capture refers to an unknown documentation service")
        })?;
        if manifests.contains_key(&manifest.service) {
            return Err(invalid("recovery accepts exactly one capture per service"));
        }
        if manifest.service_digest != digest(service)? {
            return Err(invalid(
                "recovered capture does not match the current service declaration",
            ));
        }
        if inputs.evidence_expectations.contains_key(&manifest.service)
            || inputs.update_policies.contains_key(&manifest.service)
            || inputs.update_state.targets.contains_key(&manifest.service)
        {
            return Err(invalid(
                "capture recovery does not yet support external evidence expectations or update policies/targets",
            ));
        }
        // NON_CACHEABLE disallows automatic source reuse. It does not erase the
        // historical evidence explicitly selected here; preserve its reason.
        let saved = cache::load_capture(&repo, &manifest)?;
        provenance.insert(
            manifest.service.clone(),
            json!({
                "manifest": name, "manifestDigest": cache::content_digest(&raw),
                "revision": manifest.revision, "cacheability": manifest.cacheability,
                "reason": manifest.reason,
                "sourceAuthority": "RETAINED_SOURCE_NOT_REVERIFIED"
            }),
        );
        evidence.insert(manifest.service.clone(), saved);
        manifests.insert(manifest.service.clone(), manifest);
    }
    let unresolved = inputs.services.keys().filter(|id| !evidence.contains_key(*id))
        .map(|id| (id.clone(), json!({"status":"UNRESOLVED", "reason":"NOT_RECOVERED",
            "nextAction":"Explicitly select a saved capture for this service or check its sources."})))
        .collect();
    let mut checked = check::assemble(
        input_digest.clone(),
        evidence,
        unresolved,
        &inputs.interactions,
        &inputs.scenarios,
    )?;
    check::attach_catalogue_from_inputs(&inputs, &mut checked)?;
    composition::attach_retained(&repo, &inputs, &mut checked)?;
    if repo.input_digest()? != input_digest {
        return Err(invalid(
            "documentation inputs changed during capture recovery",
        ));
    }
    composition::validate_retained(&repo, &inputs)?;
    checked.source_inputs = Some(SourceInputs {
        schema: check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest,
        selected_services: BTreeSet::new(),
        retained_services: manifests.keys().cloned().collect(),
        inputs,
    });
    // Store the shared reader closure but retain the producer's exact capture
    // envelopes, including NON_CACHEABLE. Never write latest or the keyed files.
    let mut manifest = checked.store_manifest(&repo)?;
    manifest.service_manifests = manifests;
    let encoded = bytes(&manifest)?;
    if encoded.len() as u64 > check::PORTABLE_CACHE_MAX_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "recovered snapshot exceeds its portable cache budget",
        ));
    }
    let reference = cache::put(&repo, check::CHECK_MANIFEST_SCHEMA, &encoded)?;
    let snapshot = format!("{}/{}", reference.digest, reference.size);
    Ok(
        json!({"schema":"codeclew-documentation-capture-recovery/1.0",
        "status":"RECOVERED", "snapshot":snapshot,
        "sourceAuthority":"RETAINED_SOURCE_NOT_REVERIFIED",
        "captures":provenance, "check":checked.summary()}),
    )
}

#[cfg(test)]
mod tests {
    use super::super::check::Check;
    use super::super::model::{Entrypoint, Observation, Service, ServiceEvidence, Source};
    use super::*;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, Repository, String) {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Recovery fixture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        for id in ["orders", "billing"] {
            let service: Service = serde_json::from_value(json!({
                "schema":"codeclew-documentation-service/1.0", "id":id,
                "title":id, "repositoryId":id,
                "repository":format!("https://example.invalid/{id}"), "language":"java",
                "profile":"java-17plus-maven-read-only", "compilations":[":/main"], "targetRef":"main"
            })).unwrap();
            repo.service_add(service, Some(&repo.input_digest().unwrap()))
                .unwrap();
        }
        let text = "public void dispatch() { deliver(); }";
        let source = Source {
            id: "orders-source".into(),
            service: "orders".into(),
            revision: "a".repeat(40),
            file: "src/Dispatch.java".into(),
            start_line: 1,
            end_line: 1,
            text: text.into(),
            text_digest: cache::content_digest(text.as_bytes()),
            evidence_digest: digest(&json!({"fixture":"dispatch"})).unwrap(),
            authority: "FIXTURE_SOURCE".into(),
            occurrence: None,
            url: None,
        };
        let flow = json!({"kind":"CALL", "target":"Dispatch.deliver()", "order":0});
        let observation = Observation {
            id: "orders-flow".into(),
            kind: "FLOW".into(),
            service: "orders".into(),
            symbol: "Dispatch.dispatch()".into(),
            digest: digest(&flow).unwrap(),
            normalized: flow,
            source_ids: vec![source.id.clone()],
        };
        let entrypoint = Entrypoint {
            id: "orders-dispatch".into(),
            service: "orders".into(),
            symbol: "Dispatch.dispatch()".into(),
            kind: "SCHEDULED".into(),
            trigger: json!({"schedule":"fixture"}),
            source_ids: vec![source.id.clone()],
            dependency_ids: vec![observation.id.clone()],
            boundaries: vec![],
        };
        let evidence = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "orders".into(),
            revision: "a".repeat(40),
            service_digest: digest(&repo.services().unwrap()["orders"]).unwrap(),
            extractor: "fixture/1.0".into(),
            runtime_mode: "FIXTURE".into(),
            coverage: "TEST_ONLY".into(),
            boundaries: vec![],
            entrypoints: vec![entrypoint],
            observations: BTreeMap::from([(observation.id.clone(), observation)]),
            sources: BTreeMap::from([(source.id.clone(), source)]),
            contracts: BTreeMap::from([("dispatch".into(), json!({"kind":"fixture-contract"}))]),
        };
        let mut manifest = cache::store_capture(&repo, &evidence).unwrap();
        cache::mark_non_cacheable(&mut manifest, "EXTERNAL_AUTHORITY_UNPROVEN");
        let name = format!("orders-{}.json", "b".repeat(64));
        repo.atomic(
            &format!(".codeclew/cache/{name}"),
            &serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        (temporary, repo, name)
    }

    #[test]
    fn offline_recovery_preserves_capture_authority_without_latest_mutation() {
        let (temporary, repo, name) = fixture();
        let keyed = repo.path(&format!(".codeclew/cache/{name}")).unwrap();
        let before = fs::read(&keyed).unwrap();
        let original: cache::CaptureManifest = serde_json::from_slice(&before).unwrap();
        let original_evidence = cache::load_capture(&repo, &original).unwrap();
        // No source repository, runtime, Git or Maven is available or needed.
        repo.atomic(
            ".codeclew/cache/latest-check.json",
            b"unrelated latest pointer",
        )
        .unwrap();
        let result = run(temporary.path(), &[name]).unwrap();
        assert_eq!(result["status"], "RECOVERED");
        assert_eq!(
            result["captures"]["orders"]["manifestDigest"],
            cache::content_digest(&before)
        );
        let handle = result["snapshot"].as_str().unwrap();
        let loaded = Check::load_snapshot(&repo, handle).unwrap();
        assert_eq!(
            digest(&loaded.services["orders"]).unwrap(),
            digest(&original_evidence).unwrap()
        );
        assert_eq!(
            loaded.dependencies["orders-flow"].source_ids,
            vec!["orders-source"]
        );
        assert_eq!(
            loaded.sources()["orders-source"].text,
            "public void dispatch() { deliver(); }"
        );
        assert_eq!(
            loaded.source_authorities()["orders"],
            "RETAINED_SOURCE_NOT_REVERIFIED"
        );
        assert_eq!(loaded.unresolved["billing"]["reason"], "NOT_RECOVERED");
        assert!(
            loaded
                .source_inputs
                .as_ref()
                .unwrap()
                .selected_services
                .is_empty()
        );
        let saved = Check::load_snapshot_manifest(&repo, handle).unwrap();
        assert_eq!(
            digest(&saved.service_manifests["orders"]).unwrap(),
            digest(&original).unwrap()
        );
        assert_eq!(fs::read(keyed).unwrap(), before);
        assert_eq!(
            fs::read(repo.path(".codeclew/cache/latest-check.json").unwrap()).unwrap(),
            b"unrelated latest pointer"
        );
        let (retained, returned) =
            Check::retained(&repo, Some(handle), &BTreeSet::from(["orders".into()])).unwrap();
        assert_eq!(returned, handle);
        assert_eq!(
            retained.source_authorities()["orders"],
            "RETAINED_SOURCE_NOT_REVERIFIED"
        );
        let (derived, derivative_handle) = composition::recompose(&repo, handle).unwrap();
        assert_eq!(
            derived.source_authorities()["orders"],
            "RETAINED_SOURCE_NOT_REVERIFIED"
        );
        let derivative = Check::load_snapshot_manifest(&repo, &derivative_handle).unwrap();
        assert_eq!(
            digest(&derivative.service_manifests["orders"]).unwrap(),
            digest(&original).unwrap()
        );
    }

    #[test]
    fn invalid_selection_and_corrupt_references_do_not_save_snapshots() {
        for kind in [
            "duplicate",
            "unknown",
            "mismatch",
            "corrupt",
            "path",
            "empty",
        ] {
            let (temporary, repo, name) = fixture();
            let path = repo.path(&format!(".codeclew/cache/{name}")).unwrap();
            let mut manifest: cache::CaptureManifest =
                store::read(&path, store::MAX_RECORD).unwrap();
            match kind {
                "unknown" => manifest.service = "missing".into(),
                "mismatch" => manifest.service_digest = format!("sha256:{}", "c".repeat(64)),
                "corrupt" => manifest.sources.digest = format!("sha256:{}", "d".repeat(64)),
                _ => {}
            }
            fs::write(path, bytes(&manifest).unwrap()).unwrap();
            let before = cache::owned_digests(&repo, 10000).unwrap();
            let captures = match kind {
                "duplicate" => vec![name.clone(), name],
                "path" => vec![format!("../{name}")],
                "empty" => vec![],
                _ => vec![name],
            };
            assert!(run(temporary.path(), &captures).is_err(), "{kind}");
            assert_eq!(
                cache::owned_digests(&repo, 10000).unwrap(),
                before,
                "{kind}"
            );
            assert!(
                !repo
                    .path(".codeclew/cache/latest-check.json")
                    .unwrap()
                    .exists()
            );
        }
    }
    #[test]
    fn external_authority_rejects_recovery_without_object_writes() {
        for kind in ["expectation", "policy", "target"] {
            let (temporary, repo, name) = fixture();
            let (path, record) = match kind {
                "expectation" => (
                    "catalog/evidence-trust/orders.json",
                    json!({
                        "schema":"codeclew-documentation-evidence-expectation/1.0",
                        "service":"orders", "repositoryId":"orders",
                        "serviceDigest":digest(&repo.services().unwrap()["orders"]).unwrap(),
                        "revision":"a".repeat(40), "manifestDigest":format!("sha256:{}", "b".repeat(64)), "sequence":1
                    }),
                ),
                "policy" => (
                    "catalog/update-policy/orders.json",
                    json!({
                        "schema":"codeclew-documentation-update-policy/1.0", "service":"orders",
                        "repositoryId":"orders", "acceptedRefs":["main"]
                    }),
                ),
                _ => (
                    "catalog/update-state.json",
                    json!({
                        "schema":"codeclew-documentation-update-state/1.0", "targets":{"orders":{
                            "schema":"codeclew-documentation-update-event/1.0", "id":"orders-update",
                            "service":"orders", "repositoryId":"orders", "sourceRef":"main",
                            "revision":"a".repeat(40), "sequence":1
                        }}
                    }),
                ),
            };
            repo.atomic(path, &bytes(&record).unwrap()).unwrap();
            let before = cache::owned_digests(&repo, 10000).unwrap();
            let error = run(temporary.path(), &[name]).unwrap_err();
            assert!(
                error
                    .message
                    .contains("capture recovery does not yet support"),
                "{kind}: {}",
                error.message
            );
            assert_eq!(
                cache::owned_digests(&repo, 10000).unwrap(),
                before,
                "{kind}"
            );
            assert!(
                !repo
                    .path(".codeclew/cache/latest-check.json")
                    .unwrap()
                    .exists()
            );
        }
    }

    #[test]
    fn dangling_baseline_fails_before_reading_selected_capture() {
        let (temporary, repo, _) = fixture();
        let before = cache::owned_digests(&repo, 10000).unwrap();
        let index = format!("<!-- codeclew-bundle {} -->\n", "a".repeat(64));
        repo.atomic("docs/index.html", index.as_bytes()).unwrap();
        let error = run(temporary.path(), &["nonexistent.json".into()]).unwrap_err();
        assert!(
            error.message.contains("DOCS_BASELINE_INCOMPLETE"),
            "{}",
            error.message
        );
        assert!(error.message.contains("bindings.json"));
        assert_eq!(cache::owned_digests(&repo, 10000).unwrap(), before);
        assert_eq!(
            fs::read_to_string(repo.path("docs/index.html").unwrap()).unwrap(),
            index
        );
    }
}
