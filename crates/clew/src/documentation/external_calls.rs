//! Explicit outgoing-call declarations projected from the selected saved Check.
//! No name matching, destination inference, source acquisition or network access.
use super::{
    check::{Check, InteractionCheck},
    model::{Observation, Operation},
};
use serde_json::{Value, json};

fn contract<'a>(
    reference: Option<&str>,
    receiver: &str,
    link: &InteractionCheck,
    checked: &'a Check,
) -> (Option<&'a Observation>, &'static str) {
    if let Some(id) = reference {
        let value = checked
            .dependencies
            .get(id)
            .filter(|o| o.kind == "CONTRACT_OPERATION");
        return (
            value,
            if value.is_some() {
                "EXPLICIT_SAVED_CONTRACT"
            } else {
                "REFERENCE_UNAVAILABLE"
            },
        );
    }
    if !matches!(link.to.status.as_str(), "RESOLVED" | "SOURCE_MATCH")
        || link.to.candidates.len() != 1
    {
        return (None, "NOT_BOUND");
    }
    let Some(evidence) = checked.services.get(receiver) else {
        return (None, "NOT_BOUND");
    };
    let Some(declaration) = evidence.observations.get(&link.to.candidates[0]) else {
        return (None, "NOT_BOUND");
    };
    let entries: Vec<_> = evidence
        .entrypoints
        .iter()
        .filter(|e| e.symbol == declaration.symbol && e.dependency_ids.contains(&declaration.id))
        .collect();
    let candidates: Vec<_> = checked
        .dependencies
        .values()
        .filter(|o| {
            o.kind == "CONTRACT_OPERATION"
                && o.service == receiver
                && entries
                    .iter()
                    .any(|entry| o.normalized["entrypoint"] == entry.id)
        })
        .collect();
    match candidates.as_slice() {
        [one] => (Some(one), "RECEIVER_SAVED_CONTRACT"),
        [] => (None, "NOT_BOUND"),
        _ => (None, "AMBIGUOUS_RECEIVER_CONTRACT"),
    }
}

pub(super) fn project(operation: &Operation, checked: &Check) -> Vec<Value> {
    let mut rows = Vec::new();
    for event in operation
        .events
        .iter()
        .filter(|e| matches!(e.kind.as_str(), "message" | "declared"))
    {
        let links: Vec<_> = checked
            .interactions
            .values()
            .filter(|link| {
                if event.kind == "declared" {
                    event.interaction.as_deref() == Some(&link.id)
                } else {
                    link.call_site.status == "RESOLVED"
                        && link.call_site.candidates.len() == 1
                        && event.dependency_ids.contains(&link.call_site.candidates[0])
                }
            })
            .collect();
        for link in &links {
            let Some(declaration) = checked
                .dependencies
                .get(&format!("interaction:{}", link.id))
            else {
                continue;
            };
            if declaration.kind != "DECLARED_INTERACTION" {
                continue;
            }
            let value = &declaration.normalized;
            let reference = value["contractReference"].as_str();
            let (contract, contract_status) = contract(
                reference,
                value["to"]["service"].as_str().unwrap_or(""),
                link,
                checked,
            );
            rows.push(json!({
                "eventId":event.id, "interactionId":link.id, "title":value["title"],
                "from":value["from"]["service"], "to":value["to"]["service"],
                "transport":value["transport"], "addresses":value.get("addresses").cloned().unwrap_or_else(|| json!([])),
                "declaration":value["declaration"], "applicability":value["applicability"],
                "declarationDigest":declaration.digest, "callSiteCandidates":link.call_site.candidates,
                "receiverCandidates":link.to.candidates,
                "bindingStatus":if links.len() == 1 { "EXPLICIT_INTERACTION" } else { "MULTIPLE_DECLARED_INTERACTIONS" },
                "bindingMode":if event.kind == "declared" { "DECLARED_EVENT" } else { "EXACT_CALL_SITE" },
                "contractReference":reference,
                "contractStatus":contract_status,
                "contract":contract, "snapshotContextDigest":checked.context_digest,
                "authority":"USER_DECLARATION_NOT_RUNTIME_PROOF", "retained":false, "targetChanged":false,
            }));
        }
    }
    rows
}

/// Keep the whole outgoing-call projection with its accepted operation. Never
/// substitute a new declaration or contract into old prose, including legacy pages.
pub(super) fn retain(previous: Option<&Value>, checked: &Check) -> Value {
    let mut rows = previous
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for row in &mut rows {
        row["retained"] = json!(true);
        let declaration_changed = row["interactionId"].as_str().is_none_or(|id| {
            checked
                .dependencies
                .get(&format!("interaction:{id}"))
                .is_none_or(|value| row["declarationDigest"] != value.digest)
        });
        let contract_changed = row["contract"]["id"].as_str().is_some_and(|id| {
            checked
                .dependencies
                .get(id)
                .is_none_or(|value| row["contract"]["digest"] != value.digest)
        });
        let binding_changed = row["interactionId"].as_str().is_none_or(|id| {
            checked.interactions.get(id).is_none_or(|link| {
                row["callSiteCandidates"] != json!(link.call_site.candidates)
                    || row["receiverCandidates"] != json!(link.to.candidates)
            })
        });
        let selection_changed = row["interactionId"].as_str().is_none_or(|id| {
            let Some(link) = checked.interactions.get(id) else {
                return true;
            };
            let Some(declaration) = checked.dependencies.get(&format!("interaction:{id}")) else {
                return true;
            };
            let (current, status) = contract(
                declaration.normalized["contractReference"].as_str(),
                declaration.normalized["to"]["service"]
                    .as_str()
                    .unwrap_or(""),
                link,
                checked,
            );
            row["contractStatus"] != status
                || row["contract"]["id"] != json!(current.map(|c| &c.id))
                || row["contract"]["digest"] != json!(current.map(|c| &c.digest))
                || (row["bindingMode"] == "EXACT_CALL_SITE" && {
                    let count = checked
                        .interactions
                        .values()
                        .filter(|candidate| {
                            candidate.call_site.status == "RESOLVED"
                                && candidate.call_site.candidates.len() == 1
                                && candidate.call_site.candidates == link.call_site.candidates
                        })
                        .count();
                    row["bindingStatus"]
                        != if count == 1 {
                            "EXPLICIT_INTERACTION"
                        } else {
                            "MULTIPLE_DECLARED_INTERACTIONS"
                        }
                })
        });
        row["targetChanged"] =
            json!(declaration_changed || contract_changed || binding_changed || selection_changed);
    }
    json!(rows)
}

#[cfg(test)]
mod tests {
    use super::super::model::{Entrypoint, ServiceEvidence};
    use super::*;
    use std::collections::BTreeMap;

    fn fixture() -> (Check, Operation) {
        let resolution =
            |id: &str| json!({"status":"RESOLVED","candidates":[id],"sourceIds":[],"details":[]});
        let mut checked: Check = serde_json::from_value(json!({
            "schema":"codeclew-documentation-check/1.0","inputDigest":"input","contextDigest":"snapshot",
            "services":{},"unresolved":{},"scenarios":{},"dependencies":{},
            "interactions":{"outgoing":{"id":"outgoing","origin":"human","declarationDigest":"declaration",
                "from":resolution("caller-decl"),"to":resolution("receiver-decl"),"callSite":resolution("call"),
                "method":{},"path":{},"destination":{},"contractStatus":"NOT_ASSESSED","runtime":"UNKNOWN","applicability":null,"boundaries":[]}}
        })).unwrap();
        let observation = |id: &str, kind: &str, service: &str, normalized: Value| Observation {
            id: id.into(),
            kind: kind.into(),
            service: service.into(),
            symbol: "handler".into(),
            digest: format!("digest-{id}"),
            normalized,
            source_ids: vec![],
        };
        checked.dependencies.insert("interaction:outgoing".into(),observation("interaction:outgoing","DECLARED_INTERACTION","",json!({
            "id":"outgoing","from":{"service":"caller"},"to":{"service":"receiver"},
            "declaration":{"origin":"human","rationale":"Operator declaration"},"contractReference":"outgoing-contract"
        })));
        for (id, service, entry) in [
            ("outgoing-contract", "receiver", "receiver-entry"),
            ("incoming-contract", "caller", "caller-entry"),
        ] {
            checked.dependencies.insert(
                id.into(),
                observation(
                    id,
                    "CONTRACT_OPERATION",
                    service,
                    json!({"entrypoint":entry,"method":"POST","path":"/same-name"}),
                ),
            );
        }
        let declaration = observation("receiver-decl", "SYMBOL", "receiver", json!({}));
        let entry = Entrypoint {
            id: "receiver-entry".into(),
            service: "receiver".into(),
            symbol: declaration.symbol.clone(),
            kind: "HTTP_ENDPOINT".into(),
            trigger: json!({}),
            source_ids: vec![],
            dependency_ids: vec![declaration.id.clone()],
            boundaries: vec![],
        };
        checked.services.insert(
            "receiver".into(),
            ServiceEvidence {
                schema: "codeclew-documentation-service-evidence/1.0".into(),
                service: "receiver".into(),
                revision: "revision".into(),
                service_digest: "service".into(),
                extractor: "fixture".into(),
                runtime_mode: "TEST".into(),
                coverage: "PARTIAL".into(),
                boundaries: vec![],
                entrypoints: vec![entry],
                observations: BTreeMap::from([(declaration.id.clone(), declaration)]),
                sources: BTreeMap::new(),
                contracts: BTreeMap::new(),
            },
        );
        let operation=serde_json::from_value(json!({"id":"operation","title":"Call receiver","summary":{"id":"summary","text":"Call receiver","dependencyIds":[],"sourceIds":[]},"participants":[],"events":[{"id":"message","kind":"message","text":"Call receiver","dependencyIds":["call"],"sourceIds":[]}],"explanation":[],"findings":[],"boundaries":[]})).unwrap();
        (checked, operation)
    }

    #[test]
    fn explicit_contract_never_falls_back_to_incoming_or_similar_name() {
        let (mut checked, operation) = fixture();
        assert_eq!(
            project(&operation, &checked)[0]["contract"]["id"],
            "outgoing-contract"
        );
        checked
            .dependencies
            .get_mut("interaction:outgoing")
            .unwrap()
            .normalized["contractReference"] = json!("/same-name");
        let rows = project(&operation, &checked);
        assert_eq!(rows[0]["contractStatus"], "REFERENCE_UNAVAILABLE");
        assert!(rows[0]["contract"].is_null());
        checked
            .interactions
            .get_mut("outgoing")
            .unwrap()
            .call_site
            .status = "AMBIGUOUS".into();
        assert!(project(&operation, &checked).is_empty());
    }

    #[test]
    fn receiver_contract_reuse_requires_exact_declaration_entry_and_one_contract() {
        let (mut checked, operation) = fixture();
        checked
            .dependencies
            .get_mut("interaction:outgoing")
            .unwrap()
            .normalized
            .as_object_mut()
            .unwrap()
            .remove("contractReference");
        let rows = project(&operation, &checked);
        assert_eq!(rows[0]["contractStatus"], "RECEIVER_SAVED_CONTRACT");
        assert_eq!(rows[0]["contract"]["id"], "outgoing-contract");
        let retained_unique = json!(rows);
        let mut alternative = checked.dependencies["outgoing-contract"].clone();
        alternative.id = "alternative".into();
        checked
            .dependencies
            .insert(alternative.id.clone(), alternative);
        let retained = retain(Some(&retained_unique), &checked);
        assert_eq!(retained[0]["targetChanged"], true);
        assert_eq!(retained[0]["contract"]["id"], "outgoing-contract");
        let rows = project(&operation, &checked);
        assert_eq!(rows[0]["contractStatus"], "AMBIGUOUS_RECEIVER_CONTRACT");
        assert!(rows[0]["contract"].is_null());
        checked.services.get_mut("receiver").unwrap().entrypoints[0].dependency_ids =
            vec!["other-scope-declaration".into()];
        assert_eq!(
            project(&operation, &checked)[0]["contractStatus"],
            "NOT_BOUND"
        );
    }

    #[test]
    fn retained_details_track_changes_without_importing_new_or_legacy_bindings() {
        let (mut checked, operation) = fixture();
        let rows = json!(project(&operation, &checked));
        assert_eq!(retain(Some(&rows), &checked)[0]["targetChanged"], false);
        checked
            .dependencies
            .get_mut("outgoing-contract")
            .unwrap()
            .digest = "changed-contract".into();
        let retained = retain(Some(&rows), &checked);
        assert_eq!(retained[0]["targetChanged"], true);
        assert_eq!(
            retained[0]["contract"]["digest"],
            "digest-outgoing-contract"
        );
        assert_eq!(retain(None, &checked), json!([]));
        let mut duplicate = checked.interactions["outgoing"].clone();
        duplicate.id = "second".into();
        checked.interactions.insert("second".into(), duplicate);
        let mut declaration = checked.dependencies["interaction:outgoing"].clone();
        declaration.id = "interaction:second".into();
        checked
            .dependencies
            .insert(declaration.id.clone(), declaration);
        let rows = project(&operation, &checked);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row["bindingStatus"] == "MULTIPLE_DECLARED_INTERACTIONS")
        );
    }

    #[test]
    fn historical_interactions_have_no_new_address_or_external_authority() {
        let interaction: super::super::model::Interaction=serde_json::from_str(include_str!("../../../../fixtures/durable-docs/architecture/catalog/interactions/reserve-inventory.json")).unwrap();
        assert!(interaction.addresses.is_empty());
        assert!(!interaction.external);
        let value = serde_json::to_value(interaction).unwrap();
        assert!(value.get("addresses").is_none());
        assert!(value.get("external").is_none());
    }
}
