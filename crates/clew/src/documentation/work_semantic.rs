//! Bounded navigation to frozen declared catalog records, never inferred identity.
use super::{Query, QueryProjection, Work, bytes, digest, invalid};
use crate::error::ClewError;
use serde_json::{Value, json};

pub(super) fn rows(work: &Work, query: &Query) -> Result<Vec<Value>, ClewError> {
    let service = work
        .subject
        .strip_prefix("service:")
        .ok_or_else(|| invalid("SEMANTIC_INVENTORY requires a service Work"))?;
    if query.projection != QueryProjection::Raw
        || !query.ids.is_empty()
        || !query.symbol_contains.is_empty()
        || query.file.is_some()
        || !query.file_contains.is_empty()
        || work.snapshot.is_none()
        || query.expected_snapshot != work.snapshot
        || query.expected_context_digest.as_deref() != Some(work.checked.context_digest.as_str())
    {
        return Err(invalid(
            "SEMANTIC_INVENTORY requires RAW, exact expectedSnapshot/expectedContextDigest, and no filters",
        ));
    }
    let mut result = Vec::new();
    for dependency in work.checked.dependencies.values() {
        if dependency.kind == "EXTERNAL_SERVICE" {
            let declaration = &dependency.normalized["externalService"];
            let usages = dependency.normalized["interactions"]
                .as_array()
                .ok_or_else(|| invalid("SEMANTIC_INVENTORY external usages are unavailable"))?;
            let selected: Vec<_> = usages
                .iter()
                .enumerate()
                .filter(|(_, usage)| {
                    usage["from"]["service"] == service || usage["to"]["service"] == service
                })
                .collect();
            if selected.is_empty() && !usages.is_empty() {
                continue;
            }
            let scope = if usages.is_empty() {
                "UNASSIGNED_ROOT"
            } else {
                "SELECTED_SERVICE"
            };
            result.push(item(
                work,
                dependency,
                "EXTERNAL_SYSTEM",
                "/normalized/externalService",
                declaration,
                scope,
            )?);
            for (index, usage) in selected {
                result.push(item(
                    work,
                    dependency,
                    "EXTERNAL_USE",
                    &format!("/normalized/interactions/{index}"),
                    usage,
                    "SELECTED_SERVICE",
                )?);
            }
        } else if dependency.kind == "DOMAIN_ENTITY" {
            let declaration = &dependency.normalized["entity"];
            let relations = declaration["relations"].as_array().ok_or_else(|| {
                invalid("SEMANTIC_INVENTORY entity relationships are unavailable")
            })?;
            let selected = relations
                .iter()
                .any(|relation| relation["service"] == service);
            if !selected && !relations.is_empty() {
                continue;
            }
            result.push(item(
                work,
                dependency,
                "TERM_CANDIDATE",
                "/normalized/entity",
                declaration,
                if selected {
                    "SELECTED_SERVICE"
                } else {
                    "UNASSIGNED_ROOT"
                },
            )?);
        }
    }
    let evidence = work.checked.services.get(service).map(|value| {
        json!({"revision":value.revision,"extractor":value.extractor,
            "runtimeMode":value.runtime_mode,"coverage":value.coverage,
            "boundaries":value.boundaries.iter().take(8).collect::<Vec<_>>(),
            "boundaryCount":value.boundaries.len(),"boundariesComplete":value.boundaries.len()<=8})
    });
    result.insert(0, json!({"kind":"SEMANTIC_SCOPE","id":format!("semantic-scope:{service}"),
        "record":{"schema":"codeclew-semantic-inventory-scope/1.0","service":service,
        "snapshot":work.snapshot,"contextDigest":work.checked.context_digest,"items":result.len(),
        "serviceEvidence":evidence,"sourceAcquisition":"NONE",
        "meaning":"Frozen declared candidates and selected local uses; no shared-identity, term-definition or runtime approval"}}));
    Ok(result)
}

fn item(
    work: &Work,
    dependency: &super::Observation,
    kind: &str,
    pointer: &str,
    value: &Value,
    scope: &str,
) -> Result<Value, ClewError> {
    let local_id = value["id"]
        .as_str()
        .ok_or_else(|| invalid("SEMANTIC_INVENTORY declaration identity is unavailable"))?;
    let reference = work
        .references_for("DEPENDENCY", &dependency.id)
        .first()
        .ok_or_else(|| {
            invalid("SEMANTIC_INVENTORY retained dependency reference is unavailable")
        })?;
    let original = serde_json::to_value(dependency).map_err(super::io_error)?;
    if original.pointer(pointer) != Some(value) {
        return Err(invalid("SEMANTIC_INVENTORY record pointer mismatch"));
    }
    let title: String = value["title"]
        .as_str()
        .unwrap_or(local_id)
        .chars()
        .take(128)
        .collect();
    let record = json!({"schema":"codeclew-semantic-inventory-item/1.0","kind":kind,
        "localId":local_id,"title":title,"scope":scope,
        "dependencyIds":[dependency.id],"fullRecordReference":reference,
        "basis":{"snapshot":work.snapshot,"contextDigest":work.checked.context_digest,
            "dependencyId":dependency.id,"dependencyDigest":dependency.digest,
            "dependencyRecordDigest":digest(&original)?,"pointer":pointer,"recordDigest":digest(value)?},
        "authority":dependency.normalized["authority"],
        "content":"DEFERRED_RETAINED_RECORD","definitionStatus":"CANDIDATE_NOT_SHARED_DEFINITION"});
    if bytes(&record)?.len() > 2048 {
        return Err(invalid(
            "SEMANTIC_INVENTORY navigation record exceeds its bound",
        ));
    }
    Ok(json!({"kind":"SEMANTIC_INVENTORY","id":format!("{kind}:{local_id}"),"record":record}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::{model::Observation, store::Repository};

    fn fixture() -> Work {
        let mut work = super::super::api_contract_tests::endpoint_context_fixture();
        work.snapshot = Some("saved-semantic-snapshot".into());
        for (id, kind, normalized) in [
            (
                "external-service:bank",
                "EXTERNAL_SERVICE",
                json!({
                "externalService":{"schema":"codeclew-documentation-external-service/1.0","id":"bank","title":"Bank","description":"Declared payment system","declaration":{"origin":"human","rationale":"Owned fixture"},"limitations":[]},
                "interactions":[{"id":"order-pay","title":"Order payment","from":{"service":"orders"},"to":{"service":"bank"}},
                    {"id":"other-pay","title":"Other payment","from":{"service":"unselected"},"to":{"service":"bank"}}],"authority":"DECLARED_EXTERNAL_SERVICE"}),
            ),
            (
                "external-service:unused",
                "EXTERNAL_SERVICE",
                json!({"externalService":{"id":"unused","title":"Unassigned"},"interactions":[],"authority":"DECLARED_EXTERNAL_SERVICE"}),
            ),
            (
                "external-service:other",
                "EXTERNAL_SERVICE",
                json!({"externalService":{"id":"other","title":"Unselected provider"},"interactions":[{"id":"other-only","from":{"service":"unselected"},"to":{"service":"other"}}],"authority":"DECLARED_EXTERNAL_SERVICE"}),
            ),
            (
                "entity:order",
                "DOMAIN_ENTITY",
                json!({"entity":{"id":"order","title":"Order","description":"An explicitly declared order","relations":[{"service":"orders"}]},"authority":"DECLARED_DOMAIN_ENTITY"}),
            ),
            (
                "entity:other",
                "DOMAIN_ENTITY",
                json!({"entity":{"id":"other","title":"Other domain term","relations":[{"service":"unselected"}]},"authority":"DECLARED_DOMAIN_ENTITY"}),
            ),
        ] {
            let observation = Observation {
                id: id.into(),
                kind: kind.into(),
                service: String::new(),
                symbol: id.into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: Vec::new(),
            };
            work.influence.insert(id.into(), observation.digest.clone());
            let index = work.handles.len() + 1000;
            work.handles.insert(
                format!("d{index}"),
                super::super::Handle {
                    kind: "DEPENDENCY".into(),
                    id: id.into(),
                },
            );
            work.checked.dependencies.insert(id.into(), observation);
        }
        work.identity_references = Default::default();
        work.checked.refresh_digest().unwrap();
        work
    }

    fn query(work: &Work) -> Query {
        Query {
            kind: "SEMANTIC_INVENTORY".into(),
            expected_snapshot: work.snapshot.clone(),
            expected_context_digest: Some(work.checked.context_digest.clone()),
            ..Default::default()
        }
    }

    #[test]
    fn selected_service_inventory_keeps_local_uses_and_unassigned_candidates_separate() {
        let work = fixture();
        let records = super::super::rows(
            &work,
            &super::super::Selection {
                query: Some(query(&work)),
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<_> = records
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"EXTERNAL_SYSTEM:bank"));
        assert!(ids.contains(&"EXTERNAL_USE:order-pay"));
        assert!(ids.contains(&"EXTERNAL_SYSTEM:unused"));
        assert!(ids.contains(&"TERM_CANDIDATE:order"));
        assert!(!ids.contains(&"EXTERNAL_SYSTEM:other"));
        assert!(!ids.contains(&"EXTERNAL_USE:other-pay"));
        assert!(!ids.contains(&"TERM_CANDIDATE:other"));
        let unassigned = records
            .iter()
            .find(|row| row["id"] == "EXTERNAL_SYSTEM:unused")
            .unwrap();
        assert_eq!(unassigned["record"]["scope"], "UNASSIGNED_ROOT");
        for row in records.iter().skip(1) {
            let basis = &row["record"]["basis"];
            let original = serde_json::to_value(
                &work.checked.dependencies[basis["dependencyId"].as_str().unwrap()],
            )
            .unwrap();
            assert_eq!(
                basis["recordDigest"],
                digest(
                    original
                        .pointer(basis["pointer"].as_str().unwrap())
                        .unwrap()
                )
                .unwrap()
            );
            assert!(bytes(row).unwrap().len() < 4096);
        }
    }

    #[test]
    fn exact_frozen_scope_is_required_without_relaxing_flow_id_guards() {
        let work = fixture();
        let mut selection = super::super::Selection {
            query: Some(query(&work)),
            ..Default::default()
        };
        super::super::validate_selection(&selection).unwrap();
        selection.query.as_mut().unwrap().expected_snapshot = Some("wrong".into());
        assert!(rows(&work, selection.query.as_ref().unwrap()).is_err());
        selection.query.as_mut().unwrap().kind = "SYMBOL".into();
        assert!(super::super::validate_selection(&selection).is_err());
    }

    #[test]
    fn loaded_inventory_never_substitutes_a_mutated_live_catalog() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Owned semantic inventory").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let mut work = fixture();
        let old = work.checked.dependencies["external-service:bank"].normalized["externalService"]
            .clone();
        repo.atomic("catalog/external-services/bank.json", &bytes(&old).unwrap())
            .unwrap();
        super::super::api_contract_tests::persist_operation_fixture(&repo, &mut work);
        let mut changed = old;
        changed["title"] = json!("Changed live title");
        repo.atomic(
            "catalog/external-services/bank.json",
            &bytes(&changed).unwrap(),
        )
        .unwrap();
        let loaded = super::super::load(&repo, &work.id).unwrap();
        let records = rows(&loaded, &query(&loaded)).unwrap();
        assert_eq!(
            records
                .iter()
                .find(|r| r["id"] == "EXTERNAL_SYSTEM:bank")
                .unwrap()["record"]["title"],
            "Bank"
        );
        assert_eq!(
            crate::documentation::external_services::records(&repo).unwrap()["bank"].title,
            "Changed live title"
        );
    }

    #[test]
    fn inventory_pages_keep_complete_membership_and_recorded_cursors() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Paged semantic inventory").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let mut work = fixture();
        work.request.max_items = 1;
        work.request.max_bytes = 4096;
        super::super::api_contract_tests::persist_operation_fixture(&repo, &mut work);
        let expected = rows(&work, &query(&work)).unwrap();
        let mut selection = super::super::Selection {
            query: Some(query(&work)),
            ..Default::default()
        };
        let mut supplied = Vec::new();
        let mut membership = None;
        loop {
            let page = super::super::read(&repo, &work.id, selection.clone()).unwrap();
            assert_eq!(page["total"].as_u64(), Some(expected.len() as u64));
            assert_eq!(page["omitted"], json!([]));
            assert!(page["receiptDigest"].is_string());
            if let Some(previous) = &membership {
                assert_eq!(&page["membershipDigest"], previous);
            }
            membership = Some(page["membershipDigest"].clone());
            supplied.extend(
                page["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["id"].clone()),
            );
            selection.cursor = page["nextCursor"].as_str().map(str::to_owned);
            if selection.cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            supplied,
            expected.iter().map(|r| r["id"].clone()).collect::<Vec<_>>()
        );
        let state = super::super::read_state(&repo, &work.id).unwrap();
        assert_eq!(
            state
                .receipts
                .values()
                .filter(|r| r
                    .selection
                    .query
                    .as_ref()
                    .is_some_and(|q| q.kind == "SEMANTIC_INVENTORY"))
                .count(),
            expected.len()
        );
    }
}
