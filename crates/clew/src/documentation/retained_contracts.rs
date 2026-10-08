//! Explicit OpenAPI reinterpretation of full retained declaration bytes.
use super::{
    analysis, cache, check, contracts, digest, invalid,
    model::{Service, ServiceEvidence},
    store::Repository,
};
use crate::{canonical, error::ClewError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const SCHEMA: &str = "codeclew-documentation-retained-contract-refresh/1.0";
const MAX_FILES: usize = 128;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Receipt {
    schema: String,
    reader_digest: String,
    services: BTreeMap<String, BTreeMap<String, String>>,
}

fn files(
    service: &Service,
    evidence: &ServiceEvidence,
) -> Result<BTreeMap<String, String>, ClewError> {
    if service.contract_files.is_empty() || service.contract_files.len() > MAX_FILES {
        return Err(invalid(
            "select a service with 1..128 registered retained contract files",
        ));
    }
    let mut bytes = 0usize;
    let mut result = BTreeMap::new();
    for path in &service.contract_files {
        let id = analysis::source_id(&service.id, &format!("contract/{path}"))?;
        let source = evidence.sources.get(&id).ok_or_else(|| {
            invalid("RETAINED_CONTRACT_SOURCE_UNAVAILABLE: restore the complete saved contract source; refresh never reacquires files")
        })?;
        if source.id != id
            || source.service != service.id
            || source.file != *path
            || source.revision != evidence.revision
            || source.authority != "DECLARED_OPENAPI"
            || source.start_line != 1
            || source.end_line != source.text.lines().count() as u64
            || source.text.is_empty()
            || canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
        {
            return Err(invalid(
                "retained contract source identity, full-file coverage or text digest is invalid",
            ));
        }
        bytes = bytes
            .checked_add(source.text.len())
            .ok_or_else(|| invalid("retained contract byte budget overflow"))?;
        if bytes > MAX_BYTES {
            return Err(invalid(
                "retained contract refresh exceeds 8 MiB; narrow the selected service contracts",
            ));
        }
        result.insert(path.clone(), source.text.clone());
    }
    Ok(result)
}

// These namespaces are emitted only by the contract producer. Resolver gaps
// remain operation-local; they are never removed from unrelated source facts.
fn contract_boundary(value: &str) -> bool {
    [
        "CONTRACT_",
        "OPENAPI_",
        "INVALID_CONTRACT_",
        "UNSUPPORTED_OPENAPI_",
        "UNSUPPORTED_CONTRACT_",
        "UNREADABLE_CONTRACT_",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
}

fn strip_projection(evidence: &mut ServiceEvidence) {
    evidence.contracts.clear();
    evidence
        .observations
        .retain(|_, value| !matches!(value.kind.as_str(), "CONTRACT_SCOPE" | "CONTRACT_OPERATION"));
    evidence
        .boundaries
        .retain(|value| !contract_boundary(value));
    evidence.boundaries.sort();
    evidence.boundaries.dedup();
}

pub(super) fn refresh(
    declarations: &BTreeMap<String, Service>,
    evidence: &mut BTreeMap<String, ServiceEvidence>,
    selected: &BTreeSet<String>,
) -> Result<Receipt, ClewError> {
    if selected.is_empty() || selected.len() > 8 {
        return Err(invalid(
            "select 1..8 captured services for retained contract refresh",
        ));
    }
    let mut services = BTreeMap::new();
    for id in selected {
        let declaration = declarations
            .get(id)
            .ok_or_else(|| invalid("unknown contract-refresh service"))?;
        let current = evidence
            .get_mut(id)
            .ok_or_else(|| invalid("selected service has no saved source evidence"))?;
        let input = files(declaration, current)?;
        services.insert(
            id.clone(),
            input
                .iter()
                .map(|(path, text)| (path.clone(), canonical::hash_bytes(text.as_bytes())))
                .collect(),
        );
        let original = current.clone();
        strip_projection(current);
        contracts::import(declaration, current, &input)?;
        contracts::enrich(current)?;
        // Import creates equivalent full-file declarations, but must preserve
        // original occurrence/evidence bindings rather than invent acquisition.
        current.sources = original.sources.clone();
        let mut before = original;
        let mut after = current.clone();
        strip_projection(&mut before);
        strip_projection(&mut after);
        if digest(&before)? != digest(&after)? {
            return Err(invalid(
                "contract refresh changed non-contract source evidence",
            ));
        }
    }
    Ok(Receipt {
        schema: SCHEMA.into(),
        reader_digest: canonical::hash_bytes(include_bytes!("contracts.rs")),
        services,
    })
}

pub(super) fn validate(
    repo: &Repository,
    receipt: &Receipt,
    declarations: &BTreeMap<String, Service>,
    parent: &check::CheckManifest,
    derived: &check::CheckManifest,
) -> Result<(), ClewError> {
    if receipt.schema != SCHEMA
        || receipt.services.is_empty()
        || receipt.services.len() > 8
        || !super::composition::valid_digest(&receipt.reader_digest)
        || parent
            .service_manifests
            .keys()
            .ne(derived.service_manifests.keys())
    {
        return Err(invalid("retained contract refresh receipt is invalid"));
    }
    for (id, old) in &parent.service_manifests {
        let new = &derived.service_manifests[id];
        let Some(recorded_files) = receipt.services.get(id) else {
            if digest(old)? != digest(new)? {
                return Err(invalid(
                    "unselected service changed during contract refresh",
                ));
            }
            continue;
        };
        if old.sources != new.sources
            || old.cacheability != new.cacheability
            || old.reason != new.reason
        {
            return Err(invalid(
                "contract refresh changed source bindings or capture authority",
            ));
        }
        let mut before = cache::load_capture(repo, old)?;
        let mut after = cache::load_capture(repo, new)?;
        let declaration = declarations
            .get(id)
            .ok_or_else(|| invalid("contract-refresh declaration is missing"))?;
        let expected: BTreeMap<_, _> = files(declaration, &before)?
            .into_iter()
            .map(|(path, text)| (path, canonical::hash_bytes(text.as_bytes())))
            .collect();
        if recorded_files != &expected {
            return Err(invalid(
                "contract refresh does not match retained input bytes",
            ));
        }
        let scope_id = analysis::dependency_id(id, "contract-scope", "selected-contracts")?;
        let scope = after
            .observations
            .get(&scope_id)
            .ok_or_else(|| invalid("refreshed contract reader scope is missing"))?;
        let inventory = scope.normalized["inputs"]
            .as_object()
            .ok_or_else(|| invalid("refreshed contract input inventory is invalid"))?;
        if scope.kind != "CONTRACT_SCOPE"
            || scope.service != *id
            || scope.normalized["implementationDigest"] != receipt.reader_digest
            || inventory.keys().ne(recorded_files.keys())
            || inventory
                .iter()
                .any(|(path, value)| value["digest"] != recorded_files[path])
        {
            return Err(invalid(
                "contract refresh reader or input-byte binding is invalid",
            ));
        }
        for (path, value) in &after.contracts {
            let contract_id = analysis::dependency_id(id, "contract", path)?;
            if !recorded_files.contains_key(path)
                || after
                    .observations
                    .get(&contract_id)
                    .is_none_or(|o| &o.normalized != value)
            {
                return Err(invalid(
                    "refreshed contract is outside its retained declarations",
                ));
            }
        }
        for observation in after
            .observations
            .values()
            .filter(|o| o.kind == "CONTRACT_OPERATION")
        {
            if observation.normalized["authority"] != "DECLARED_OPENAPI"
                || observation.normalized["runtimeEnforcement"] != "UNVERIFIED"
                || observation.source_ids.is_empty()
                || observation.source_ids.iter().any(|source| {
                    !expected.keys().any(|path| {
                        analysis::source_id(id, &format!("contract/{path}"))
                            .is_ok_and(|key| &key == source)
                    })
                })
            {
                return Err(invalid(
                    "refreshed operation has invalid retained declaration authority",
                ));
            }
        }
        strip_projection(&mut before);
        strip_projection(&mut after);
        if digest(&before)? != digest(&after)? {
            return Err(invalid(
                "contract refresh changed non-contract source evidence",
            ));
        }
    }
    if receipt
        .services
        .keys()
        .any(|id| !parent.service_manifests.contains_key(id))
    {
        return Err(invalid("contract refresh names an uncaptured service"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::{cli, store::Repository};
    use serde_json::json;

    fn legacy(repo: &Repository) -> check::Check {
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders",
            "repositoryId":"orders","repository":"https://example.invalid/orders",
            "language":"java","profile":"source-syntax","targetRef":"HEAD",
            "source":{"roots":["."],"dialect":"17"},"contractFiles":["api.json"]
        }))
        .unwrap();
        repo.atomic(
            "catalog/services/orders.json",
            &super::super::bytes(&service).unwrap(),
        )
        .unwrap();
        let paths: serde_json::Map<String, serde_json::Value> = (0..5).map(|i| (format!("/operation-{i}"), json!({"post":{"operationId":format!("operation{i}"),"requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"quantity":{"type":"integer"}}}}}},"responses":{"201":{"description":"Created"}}}}))).collect();
        let text = json!({"openapi":"3.1.2","info":{"title":"Synthetic retained API","version":"1"},"paths":paths}).to_string();
        let mut evidence = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "orders".into(),
            revision: "a".repeat(40),
            service_digest: digest(&service).unwrap(),
            extractor: crate::documentation::model::SOURCE_EXTRACTOR.into(),
            runtime_mode: "RETAINED".into(),
            coverage: "PARTIAL".into(),
            boundaries: vec!["SYNTHETIC_SOURCE_LIMIT".into()],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        contracts::import(
            &service,
            &mut evidence,
            &BTreeMap::from([("api.json".into(), text)]),
        )
        .unwrap();
        contracts::enrich(&mut evidence).unwrap();
        // Model a historical reader that retained all bytes but rejected 3.1.2.
        evidence.contracts.clear();
        evidence
            .observations
            .retain(|_, o| o.kind != "CONTRACT_OPERATION");
        evidence
            .boundaries
            .push("UNSUPPORTED_CONTRACT_VERSION:api.json".into());
        let scope = evidence
            .observations
            .values_mut()
            .find(|o| o.kind == "CONTRACT_SCOPE")
            .unwrap();
        scope.normalized["inputs"]["api.json"]["status"] = json!("UNSUPPORTED_CONTRACT_VERSION");
        scope.digest = digest(&scope.normalized).unwrap();
        let inputs = repo.inputs().unwrap();
        let input_digest = digest(&inputs).unwrap();
        let mut checked = check::assemble(
            input_digest.clone(),
            BTreeMap::from([("orders".into(), evidence)]),
            BTreeMap::new(),
            &inputs.interactions,
            &inputs.scenarios,
        )
        .unwrap();
        checked.source_inputs = Some(check::SourceInputs {
            schema: check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest,
            inputs,
            selected_services: BTreeSet::new(),
            retained_services: BTreeSet::from(["orders".into()]),
        });
        checked
    }

    #[test]
    fn refresh_reads_five_retained_posts_without_source_and_keeps_history() {
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Retained contract test").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let original = legacy(&repo);
        let snapshot = original.save_snapshot(&repo).unwrap();
        let latest =
            std::fs::read(repo.path(".codeclew/cache/latest-check.json").unwrap()).unwrap();
        let (plain, _) = super::super::composition::recompose(&repo, &snapshot).unwrap();
        assert_eq!(
            plain.services["orders"]
                .observations
                .values()
                .filter(|o| o.kind == "CONTRACT_OPERATION")
                .count(),
            0
        );
        let result = cli::run(cli::Command::RefreshContracts {
            root: temp.path().into(),
            snapshot: snapshot.clone(),
            services: vec!["orders".into()],
        })
        .unwrap();
        assert_eq!(result["sourceCapturePerformed"], false);
        assert!(
            result["authority"]
                .as_str()
                .unwrap()
                .contains("SOURCE_NOT_REVERIFIED")
        );
        let refreshed =
            check::Check::load_snapshot(&repo, result["snapshot"].as_str().unwrap()).unwrap();
        let service = &refreshed.services["orders"];
        let operations: Vec<_> = service
            .observations
            .values()
            .filter(|o| o.kind == "CONTRACT_OPERATION")
            .collect();
        assert_eq!(operations.len(), 5);
        assert!(operations.iter().all(|o| o.normalized["method"] == "POST"
            && o.normalized["openapi"] == "3.1.2"
            && o.normalized["runtimeEnforcement"] == "UNVERIFIED"));
        assert!(
            service
                .boundaries
                .contains(&"SYNTHETIC_SOURCE_LIMIT".into())
        );
        assert!(
            !service
                .boundaries
                .iter()
                .any(|s| s.starts_with("UNSUPPORTED_CONTRACT_VERSION"))
        );
        assert_eq!(
            digest(&service.sources).unwrap(),
            digest(&original.services["orders"].sources).unwrap()
        );
        assert_eq!(
            digest(&check::Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap(),
            digest(&original).unwrap()
        );
        assert_eq!(
            std::fs::read(repo.path(".codeclew/cache/latest-check.json").unwrap()).unwrap(),
            latest
        );
        let mut wrong_reader = refreshed.clone();
        let scope = wrong_reader
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .values_mut()
            .find(|o| o.kind == "CONTRACT_SCOPE")
            .unwrap();
        scope.normalized["implementationDigest"] = json!(format!("sha256:{}", "0".repeat(64)));
        scope.digest = digest(&scope.normalized).unwrap();
        assert!(
            wrong_reader
                .save_snapshot(&repo)
                .unwrap_err()
                .message
                .contains("reader or input-byte binding")
        );
        let mut forged = refreshed.clone();
        forged.services.get_mut("orders").unwrap().coverage = "FULL".into();
        assert!(forged.save_snapshot(&repo).is_err());
        let mut changed_source = refreshed;
        changed_source
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .values_mut()
            .next()
            .unwrap()
            .text
            .push('x');
        assert!(changed_source.save_snapshot(&repo).is_err());
    }

    #[test]
    fn refresh_refuses_missing_or_partial_retained_files_without_fallback() {
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Missing retained contract test").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let mut checked = legacy(&repo);
        checked.services.get_mut("orders").unwrap().sources.clear();
        let snapshot = checked.save_snapshot(&repo).unwrap();
        let latest =
            std::fs::read(repo.path(".codeclew/cache/latest-check.json").unwrap()).unwrap();
        let failure = super::super::composition::refresh_contracts(
            &repo,
            &snapshot,
            &BTreeSet::from(["orders".into()]),
        )
        .unwrap_err();
        assert!(
            failure
                .message
                .contains("RETAINED_CONTRACT_SOURCE_UNAVAILABLE")
        );
        assert_eq!(
            std::fs::read(repo.path(".codeclew/cache/latest-check.json").unwrap()).unwrap(),
            latest
        );
        let mut checked = legacy(&repo);
        checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .values_mut()
            .next()
            .unwrap()
            .start_line = 2;
        let snapshot = checked.save_snapshot(&repo).unwrap();
        assert!(
            super::super::composition::refresh_contracts(
                &repo,
                &snapshot,
                &BTreeSet::from(["orders".into()])
            )
            .unwrap_err()
            .message
            .contains("full-file coverage")
        );
    }
}
