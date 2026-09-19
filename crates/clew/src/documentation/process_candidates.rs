//! Deterministic structural candidates, never inferred business processes.
use super::{
    invalid,
    model::{Entrypoint, Observation, ServiceEvidence},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_VERSION: &str = "local-calls-control/2";
fn preview_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = maximum_bytes.saturating_sub(3);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

pub struct Catalog {
    pub records: Vec<Value>,
    pub summary: Value,
}
pub fn public_boundary(entry: &Entrypoint) -> bool {
    !entry.kind.starts_with("SOURCE_")
        || entry
            .trigger
            .get("frameworkDeclarations")
            .and_then(Value::as_array)
            .is_some_and(|v| !v.is_empty())
}
fn scope(observation: &Observation) -> &str {
    observation.normalized["scope"].as_str().unwrap_or("")
}
fn callable(observation: &Observation) -> bool {
    observation.kind == "SYMBOL"
        && (matches!(
            observation.normalized["declarationKind"].as_str(),
            Some("METHOD" | "FUNCTION" | "CONSTRUCTOR")
        ) || (observation.normalized.get("declarationKind").is_none()
            && matches!(
                observation.normalized["syntaxKind"].as_str(),
                Some(
                    "function_definition"
                        | "function_declaration"
                        | "method_declaration"
                        | "constructor_declaration"
                        | "secondary_constructor"
                )
            )
            && observation.normalized.get("documentation").is_some()))
}
/// One row per service/scope/symbol, preserving incompatible occurrences as an explicit gap.
pub fn callables(evidence: &ServiceEvidence) -> Vec<Value> {
    let mut groups: BTreeMap<(&str, &str), Vec<&Observation>> = BTreeMap::new();
    for observation in evidence.observations.values().filter(|o| callable(o)) {
        groups
            .entry((scope(observation), &observation.symbol))
            .or_default()
            .push(observation);
    }
    groups
        .into_iter()
        .map(|((scope, symbol), occurrences)| {
            let first = occurrences[0];
            let ambiguous = occurrences.iter().any(|o| o.normalized != first.normalized);
            let ids: Vec<_> = occurrences.iter().map(|o| &o.id).collect();
            let sources: BTreeSet<_> = occurrences.iter().flat_map(|o| &o.source_ids).collect();
            let public = evidence
                .entrypoints
                .iter()
                .any(|e| public_boundary(e) && e.dependency_ids.iter().any(|id| ids.contains(&id)));
            json!({"id":first.id,"service":evidence.service,"scope":scope,"symbol":symbol,
            "name":first.normalized["name"],"owner":first.normalized["ownerIdentity"],
            "parameterTypes":first.normalized.pointer("/documentation/parameterTypes"),
            "sourceIds":sources,"dependencyIds":ids,"publicBoundary":public,
            "status":if ambiguous {"AMBIGUOUS"} else {"RECORDED"}})
        })
        .collect()
}
/// Reads saved observations only. A missing body/flow remains unknown, not zero processes.
pub fn catalog(
    evidence: &ServiceEvidence,
    explicit: &BTreeSet<String>,
) -> Result<Catalog, ClewError> {
    if explicit
        .iter()
        .any(|id| !evidence.observations.get(id).is_some_and(callable))
    {
        return Err(invalid(
            "explicit process root must name a callable SYMBOL observation in the selected service snapshot",
        ));
    }
    let declarations = callables(evidence);
    let local: BTreeSet<_> = declarations
        .iter()
        .filter(|row| row["status"] != "AMBIGUOUS")
        .map(|row| {
            (
                row["scope"].as_str().unwrap(),
                row["symbol"].as_str().unwrap(),
            )
        })
        .collect();
    let mut flows: BTreeMap<(&str, &str), Vec<&Observation>> = BTreeMap::new();
    for observation in evidence.observations.values().filter(|o| o.kind == "FLOW") {
        flows
            .entry((scope(observation), &observation.symbol))
            .or_default()
            .push(observation);
    }
    let mut records = Vec::new();
    let mut unavailable = 0;
    let mut unavailable_scopes = BTreeSet::new();
    for declaration in &declarations {
        let id = declaration["id"].as_str().unwrap();
        let observation = &evidence.observations[id];
        let scope = scope(observation);
        let flow = flows.get(&(scope, observation.symbol.as_str()));
        let expected = observation
            .normalized
            .pointer("/documentation/events")
            .and_then(Value::as_array);
        let mut gaps = BTreeSet::new();
        if observation.source_ids.is_empty()
            || observation
                .source_ids
                .iter()
                .any(|id| !evidence.sources.contains_key(id))
        {
            gaps.insert("METHOD_SOURCE_UNAVAILABLE".to_owned());
        }
        if let Some(expected) = expected {
            if flow.map_or(0, Vec::len) != expected.len()
                || flow
                    .into_iter()
                    .flatten()
                    .filter_map(|event| event.normalized["ordinal"].as_u64())
                    .collect::<BTreeSet<_>>()
                    != (0..expected.len() as u64).collect::<BTreeSet<_>>()
            {
                gaps.insert("FLOW_EVENT_EVIDENCE_MISSING".to_owned());
            }
        } else {
            gaps.insert("METHOD_FLOW_UNAVAILABLE".to_owned());
        }
        if declaration["status"] == "AMBIGUOUS" {
            gaps.insert("SCOPED_DECLARATION_AMBIGUOUS".into());
        }
        if let Some(boundaries) = observation
            .normalized
            .pointer("/documentation/boundaries")
            .and_then(Value::as_array)
        {
            gaps.extend(
                boundaries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
        }
        let mut targets = BTreeSet::new();
        let mut controls = 0;
        let mut lexical_calls = 0;
        let mut event_ids = Vec::new();
        for event in flow.into_iter().flatten() {
            event_ids.push(event.id.clone());
            if matches!(
                event.normalized["kind"].as_str(),
                Some("CALL" | "CONSTRUCT")
            ) && event.normalized["authority"] == "SYNTAX"
            {
                lexical_calls += 1;
            }
            if matches!(event.normalized["kind"].as_str(), Some("IF" | "LOOP")) {
                controls += 1;
            }
            if matches!(
                event.normalized["kind"].as_str(),
                Some("CALL" | "CONSTRUCT")
            ) && let Some(target) = event.normalized["target"].as_str()
                && local.contains(&(scope, target))
            {
                targets.insert(target.to_owned());
            }
            if event.normalized["kind"] == "BOUNDARY" {
                gaps.insert("FLOW_HAS_UNEXPANDED_BOUNDARIES".into());
            }
        }
        let mut reasons = Vec::new();
        if declaration["dependencyIds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| explicit.contains(id.as_str().unwrap()))
        {
            reasons.push("EXPLICIT_ROOT");
        }
        let trigger_ids: Vec<_> = evidence
            .entrypoints
            .iter()
            .filter(|entry| {
                public_boundary(entry)
                    && entry.dependency_ids.iter().any(|id| {
                        declaration["dependencyIds"]
                            .as_array()
                            .unwrap()
                            .contains(&json!(id))
                    })
            })
            .map(|entry| entry.id.clone())
            .collect();
        if !trigger_ids.is_empty() {
            reasons.push("DISCOVERED_TRIGGER");
        }
        if targets.len() >= 2 && controls > 0 && declaration["status"] != "AMBIGUOUS" {
            reasons.push("INTERNAL_ORCHESTRATION_CANDIDATE");
        }
        if lexical_calls >= 2 && controls > 0 && declaration["status"] != "AMBIGUOUS" {
            reasons.push("LEXICAL_ORCHESTRATION_CANDIDATE");
            gaps.insert("LEXICAL_CALL_TARGETS_UNRESOLVED".into());
        }
        if !gaps.is_empty() {
            unavailable += 1;
            unavailable_scopes.insert(scope.to_owned());
        }
        if reasons.is_empty() {
            continue;
        }
        let mut record = declaration.clone();
        record["reasons"] = json!(reasons);
        record["lane"] = json!(if trigger_ids.is_empty() {
            "internal"
        } else {
            "trigger"
        });
        record["status"] = json!(if gaps.is_empty() {
            "CANDIDATE"
        } else {
            "NEEDS_EVIDENCE"
        });
        record["localCallTargets"] = json!(targets);
        record["localCallTargetCount"] = json!(targets.len());
        record["controlEventCount"] = json!(controls);
        record["lexicalCallSiteCount"] = json!(lexical_calls);
        record["flowDependencyIds"] = json!(event_ids);
        record["triggerIds"] = json!(trigger_ids);
        record["gaps"] = json!(gaps);
        record["meaningReview"] = json!("UNASSESSED");
        records.push(record);
    }
    records.sort_by(|a, b| {
        let rank = |r: &Value| {
            (
                r["reasons"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("EXPLICIT_ROOT")),
                r["localCallTargetCount"].as_u64().unwrap_or(0),
                r["controlEventCount"].as_u64().unwrap_or(0),
            )
        };
        rank(b)
            .cmp(&rank(a))
            .then_with(|| a["scope"].as_str().cmp(&b["scope"].as_str()))
            .then_with(|| a["symbol"].as_str().cmp(&b["symbol"].as_str()))
    });
    let triggers = records.iter().filter(|r| r["lane"] == "trigger").count();
    let internal = records.len() - triggers;
    let summary = json!({"ruleVersion":RULE_VERSION,"service":evidence.service,
        "status":if declarations.is_empty() {"UNKNOWN"} else {"PARTIAL"},
        "consideredCallableCount":declarations.len(),"candidateCount":records.len(),
        "triggerCandidateCount":triggers,"internalCandidateCount":internal,
        "flowUnavailableCount":unavailable,
        "unavailableScopeCount":unavailable_scopes.len(),
        "unavailableScopes":unavailable_scopes.iter().take(8).map(|scope|preview_text(scope,128)).collect::<Vec<_>>(),
        "omittedUnavailableScopes":unavailable_scopes.len().saturating_sub(8),
        "previewLimits":{"scopeItems":8,"scopeBytes":128,"boundaryItems":4,"boundaryBytes":256},
        "gaps":["CANDIDATES_ARE_STRUCTURAL_SIGNALS_NOT_BUSINESS_PROCESS_COVERAGE",
            "DISCOVERY_IS_BOUNDED_BY_RETAINED_SCOPES_AND_FLOW_SUPPORT",
            "DYNAMIC_REGISTRATION_AND_RUNTIME_ACTIVATION_UNVERIFIED"],
        "sourceBoundaries":evidence.boundaries.iter().take(4).map(|boundary|preview_text(boundary,256)).collect::<Vec<_>>(),
        "omittedSourceBoundaries":evidence.boundaries.len().saturating_sub(4),
        "sourceBoundaryCount":evidence.boundaries.len(),
        "detailAuthority":"Selected immutable snapshot; preview strings may be truncated"});
    Ok(Catalog { records, summary })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence() -> ServiceEvidence {
        serde_json::from_value(
            json!({"schema":"codeclew-documentation-service-evidence/1.0",
            "service":"svc","revision":"rev","serviceDigest":"digest","extractor":"test",
            "runtimeMode":"TEST","coverage":"PARTIAL","boundaries":[],"entrypoints":[],
            "observations":{},"sources":{},"contracts":{}}),
        )
        .unwrap()
    }
    fn add_method(
        e: &mut ServiceEvidence,
        id: &str,
        scope: &str,
        symbol: &str,
        events: Option<Vec<Value>>,
    ) {
        let mut normalized = json!({"declarationKind":"METHOD","scope":scope,"name":symbol,"ownerIdentity":"class:Worker"});
        if let Some(events) = events {
            normalized["documentation"] = json!({"events":events,"parameterTypes":[]});
            for (ordinal, mut event) in events.into_iter().enumerate() {
                event["ordinal"] = json!(ordinal);
                event["scope"] = json!(scope);
                let event_id = format!("{id}-event-{ordinal}");
                e.observations.insert(
                    event_id.clone(),
                    Observation {
                        id: event_id,
                        kind: "FLOW".into(),
                        service: "svc".into(),
                        symbol: symbol.into(),
                        normalized: event,
                        digest: "digest".into(),
                        source_ids: vec![],
                    },
                );
            }
        }
        e.observations.insert(
            id.into(),
            Observation {
                id: id.into(),
                kind: "SYMBOL".into(),
                service: "svc".into(),
                symbol: symbol.into(),
                normalized,
                digest: "digest".into(),
                source_ids: vec![],
            },
        );
    }
    fn orchestration() -> Vec<Value> {
        vec![
            json!({"kind":"IF"}),
            json!({"kind":"CALL","target":"find"}),
            json!({"kind":"CALL","target":"dispatch"}),
        ]
    }
    #[test]
    fn syntax_type_declarations_are_not_callable_or_explicit_process_roots() {
        let mut e = evidence();
        for kind in [
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
            "method_declaration",
        ] {
            let normalized = json!({"syntaxKind":kind,"name":kind,"ownerIdentity":"class:Example","documentation":{"events":[]}});
            e.observations.insert(
                kind.into(),
                Observation {
                    id: kind.into(),
                    kind: "SYMBOL".into(),
                    service: "svc".into(),
                    symbol: kind.into(),
                    normalized,
                    digest: "digest".into(),
                    source_ids: vec![],
                },
            );
        }
        let declarations = callables(&e);
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0]["id"], "method_declaration");
        for kind in [
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
        ] {
            assert!(
                catalog(&e, &BTreeSet::from([kind.into()])).is_err(),
                "{kind}"
            );
        }
        assert_eq!(
            catalog(&e, &BTreeSet::from(["method_declaration".into()]))
                .unwrap()
                .records
                .len(),
            1
        );
    }
    #[test]
    fn nominates_control_and_distinct_local_calls_in_exact_scope() {
        let mut e = evidence();
        add_method(
            &mut e,
            "worker-main",
            ":worker/main",
            "run",
            Some(orchestration()),
        );
        add_method(&mut e, "finder", ":worker/main", "find", Some(vec![]));
        add_method(&mut e, "router", ":worker/main", "dispatch", Some(vec![]));
        add_method(
            &mut e,
            "worker-test",
            ":worker/test",
            "run",
            Some(orchestration()),
        );
        let result = catalog(&e, &BTreeSet::new()).unwrap();
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0]["id"], "worker-main");
        assert_eq!(result.records[0]["localCallTargetCount"], 2);
        assert_eq!(result.summary["consideredCallableCount"], 4);
    }
    #[test]
    fn lexical_calls_nominate_without_claiming_resolved_local_targets() {
        let mut e = evidence();
        add_method(
            &mut e,
            "worker",
            "",
            "run",
            Some(vec![
                json!({"kind":"LOOP","authority":"SYNTAX"}),
                json!({"kind":"CALL","authority":"SYNTAX","targetStatus":"UNRESOLVED"}),
                json!({"kind":"CALL","authority":"SYNTAX","targetStatus":"UNRESOLVED"}),
            ]),
        );
        let result = catalog(&e, &BTreeSet::new()).unwrap();
        assert_eq!(result.records.len(), 1);
        assert_eq!(
            result.records[0]["reasons"],
            json!(["LEXICAL_ORCHESTRATION_CANDIDATE"])
        );
        assert_eq!(result.records[0]["lexicalCallSiteCount"], 2);
        assert_eq!(result.records[0]["localCallTargetCount"], 0);
        assert_eq!(result.records[0]["status"], "NEEDS_EVIDENCE");
        assert!(
            result.records[0]["gaps"]
                .as_array()
                .unwrap()
                .contains(&json!("LEXICAL_CALL_TARGETS_UNRESOLVED"))
        );
    }
    #[test]
    fn missing_flow_is_visible_and_explicit_root_is_preserved() {
        let mut e = evidence();
        add_method(&mut e, "worker", ":main", "run", None);
        let empty = catalog(&e, &BTreeSet::new()).unwrap();
        assert!(empty.records.is_empty());
        assert_eq!(empty.summary["status"], "PARTIAL");
        assert_eq!(empty.summary["flowUnavailableCount"], 1);
        let selected = catalog(&e, &BTreeSet::from(["worker".into()])).unwrap();
        assert_eq!(selected.records[0]["status"], "NEEDS_EVIDENCE");
        assert!(
            selected.records[0]["gaps"]
                .as_array()
                .unwrap()
                .contains(&json!("METHOD_FLOW_UNAVAILABLE"))
        );
        assert!(catalog(&e, &BTreeSet::from(["not-a-method".into()])).is_err());
        assert_eq!(
            catalog(&evidence(), &BTreeSet::new()).unwrap().summary["status"],
            "UNKNOWN"
        );
    }
    #[test]
    fn detects_missing_event_and_keeps_trigger_separate() {
        let mut e = evidence();
        add_method(&mut e, "method", ":main", "run", Some(orchestration()));
        e.observations.remove("method-event-1");
        e.entrypoints.push(Entrypoint {
            id: "entry".into(),
            service: "svc".into(),
            symbol: "run".into(),
            kind: "HTTP".into(),
            trigger: json!({}),
            source_ids: vec![],
            dependency_ids: vec!["method".into()],
            boundaries: vec![],
        });
        let result = catalog(&e, &BTreeSet::new()).unwrap();
        assert_eq!(result.records[0]["lane"], "trigger");
        assert!(
            result.records[0]["gaps"]
                .as_array()
                .unwrap()
                .contains(&json!("FLOW_EVENT_EVIDENCE_MISSING"))
        );
        assert_eq!(result.summary["triggerCandidateCount"], 1);
        assert_eq!(result.summary["internalCandidateCount"], 0);
    }
    #[test]
    fn deterministic_order_and_ambiguous_occurrences_are_not_collapsed() {
        let mut e = evidence();
        add_method(&mut e, "b", ":b", "run", Some(vec![]));
        add_method(&mut e, "a", ":a", "run", Some(vec![]));
        let selected = BTreeSet::from(["a".into(), "b".into()]);
        let first = catalog(&e, &selected).unwrap();
        let mut rebuilt = e.clone();
        rebuilt.observations = e.observations.into_iter().rev().collect();
        assert_eq!(first.records, catalog(&rebuilt, &selected).unwrap().records);
        assert_eq!(first.records[0]["id"], "a");
        let mut duplicate = rebuilt.observations["a"].clone();
        duplicate.id = "a-conflict".into();
        duplicate.normalized["name"] = json!("conflicting");
        rebuilt.observations.insert(duplicate.id.clone(), duplicate);
        let conflicted = catalog(&rebuilt, &selected).unwrap();
        assert_eq!(conflicted.records.len(), 2);
        assert!(
            conflicted.records[0]["gaps"]
                .as_array()
                .unwrap()
                .contains(&json!("SCOPED_DECLARATION_AMBIGUOUS"))
        );
        assert_eq!(
            conflicted.records[0]["dependencyIds"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
