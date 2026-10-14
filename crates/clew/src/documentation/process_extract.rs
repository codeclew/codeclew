//! Query packets over retained portable documentation evidence, without execution.
//! Relevance is a conservative callable envelope, not a data/effect closure proof.
use super::{check::Check, digest, invalid, model::Observation, process_graph};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const POLICY: &str = "retained-process-envelope/0.2";
pub const MAX_PACKET_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Budgets {
    pub max_methods: usize,
    pub max_events: usize,
    pub max_source_bytes: usize,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_methods: 32,
            max_events: 256,
            max_source_bytes: 256 * 1024,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub purpose: crate::thread_question::Purpose,
    /// Exact retained root observation, not a method name.
    pub service: String,
    pub root_declaration: String,
    pub configuration_scope: String,
    /// Exact retained FLOW IDs; labels and intent must be authored separately.
    #[serde(default)]
    pub outcome_event_ids: Vec<String>,
    #[serde(default)]
    pub budgets: Budgets,
}
impl Query {
    pub fn validate(&self) -> Result<(), ClewError> {
        if [
            self.service.as_str(),
            self.root_declaration.as_str(),
            self.configuration_scope.as_str(),
        ]
        .iter()
        .any(|s| s.trim().is_empty() || s.len() > 4096 || s.chars().any(char::is_control))
            || self.outcome_event_ids.len() > 32
            || self
                .outcome_event_ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 4096)
            || self.outcome_event_ids.iter().collect::<BTreeSet<_>>().len()
                != self.outcome_event_ids.len()
            || self.budgets.max_methods == 0
            || self.budgets.max_methods > 64
            || self.budgets.max_events == 0
            || self.budgets.max_events > 4096
            || self.budgets.max_source_bytes == 0
            || self.budgets.max_source_bytes > 1024 * 1024
        {
            return Err(invalid("invalid process extraction query or budgets"));
        }
        Ok(())
    }
}

/// The caller admits a retained Check; no producer label authenticates arbitrary JSON.
pub fn extract(checked: &Check, snapshot: &str, query: &Query) -> Result<Value, ClewError> {
    query.validate()?;
    let root_id = checked
        .services
        .get(&query.service)
        .map(|e| super::declaration_migration::canonical(e, &query.root_declaration))
        .transpose()?
        .unwrap_or_else(|| query.root_declaration.clone());
    let consumer = super::semantic_overlay::checked_view(checked)?;
    let checked = &consumer;
    let service = checked
        .services
        .get(&query.service)
        .ok_or_else(|| invalid("extraction service is unavailable"))?;
    if service.observations.len() > 131_072 {
        return Err(invalid("process input fact budget exceeded"));
    }
    let root = service
        .observations
        .get(&root_id)
        .filter(|o| {
            o.service == query.service
                && process_graph::callable_observation(o)
                && o.normalized["scope"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
        })
        .ok_or_else(|| invalid("extraction root must be an exact scoped callable observation"))?;
    if !checked
        .dependencies
        .get(&root.id)
        .is_some_and(|o| o.digest == root.digest && o.normalized == root.normalized)
    {
        return Err(invalid(
            "extraction root does not bind the retained Check dependency",
        ));
    }
    let graph = process_graph::collect_bounded(
        service,
        root,
        snapshot,
        &checked.input_digest,
        Some(&query.budgets),
    )?;
    packet(checked, snapshot, query, &graph, root)
}

fn packet(
    checked: &Check,
    snapshot: &str,
    query: &Query,
    graph: &Value,
    root: &Observation,
) -> Result<Value, ClewError> {
    let rows = |key: &str| graph[key].as_array().cloned().unwrap_or_default();
    let methods = rows("methods");
    let events = rows("events");
    let edges = rows("edges");
    let mut frontier = rows("frontiers");
    let mut edges = edges;
    for edge in &mut edges {
        if edge.get("relationshipAuthority").is_none() {
            edge["relationshipAuthority"] = json!("RETAINED_LOCAL_ASSOCIATION_NOT_RUNTIME_PROOF");
        }
    }
    frontier.push(
        json!({"kind":"SEMANTIC","code":"DATA_EFFECT_ALIAS_AND_RUNTIME_CLOSURE_NOT_ESTABLISHED"}),
    );
    frontier.push(json!({"kind":"SEMANTIC","code":"CROSS_SERVICE_LINKS_REQUIRE_INDEPENDENT_RELATIONSHIP_AUTHORITY"}));
    if query.configuration_scope == "UNSPECIFIED" {
        frontier.push(json!({"kind":"SEMANTIC","code":"CONFIGURATION_SCOPE_UNSPECIFIED"}));
    }
    let seeds: Vec<_> = if query.outcome_event_ids.is_empty() {
        Vec::new()
    } else {
        // Index event ownership once. A criterion must bind exactly one retained event
        // and one retained method; possible/ambiguous associations are not seeds.
        let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for method in &methods {
            if let Some(method_id) = method["id"].as_str() {
                for event in method["eventIds"].as_array().into_iter().flatten() {
                    if let Some(event_id) = event.as_str() {
                        owners.entry(event_id).or_default().push(method_id);
                    }
                }
            }
        }
        let mut event_counts = BTreeMap::<&str, usize>::new();
        for event in &events {
            if let Some(id) = event["id"].as_str() {
                *event_counts.entry(id).or_default() += 1;
            }
        }
        query
            .outcome_event_ids
            .iter()
            .map(|id| match owners.get(id.as_str()).map(Vec::as_slice) {
                Some([owner]) if event_counts.get(id.as_str()) == Some(&1) => Some(*owner),
                _ => None,
            })
            .collect()
    };
    let backward = if seeds.iter().all(Option::is_none) {
        BTreeMap::new()
    } else {
        let method_ids: Vec<_> = methods.iter().filter_map(|m| m["id"].as_str()).collect();
        let retained_edges: Vec<_> = edges
            .iter()
            .filter_map(|edge| {
                Some((
                    edge["fromMethodId"].as_str()?,
                    edge["targetMethodId"].as_str()?,
                ))
            })
            .collect();
        super::process_reachability::outcome_masks(&method_ids, &retained_edges, &seeds)
    };
    let mut outcome_results = Vec::new();
    for (bit, id) in query.outcome_event_ids.iter().enumerate() {
        let found = seeds[bit].is_some();
        if !found {
            frontier
                .push(json!({"kind":"DISCOVERY","code":"OUTCOME_NOT_FOUND_UNKNOWN","eventId":id}));
        }
        let structural_methods: Vec<_> = backward
            .iter()
            .filter_map(|(method, mask)| (mask & (1_u32 << bit) != 0).then_some(*method))
            .collect();
        outcome_results.push(json!({"eventId":id,"discovery":if found { "FOUND_IN_RETAINED_ENVELOPE" } else { "NOT_FOUND_UNKNOWN" },"structuralMethodIds":structural_methods,"semanticClosure":"UNKNOWN"}));
    }
    for (_, raw) in graph["sources"].as_object().into_iter().flatten() {
        let source: super::model::Source = serde_json::from_value(raw.clone())
            .map_err(|_| invalid("invalid extraction source"))?;
        if source.service != query.service
            || source.revision != checked.services[&query.service].revision
            || source.text_digest != crate::canonical::hash_bytes(source.text.as_bytes())
        {
            return Err(invalid("extraction source revision or digest is invalid"));
        }
    }
    let source_refs: BTreeMap<_, _> = graph["sources"].as_object().into_iter().flatten().map(|(id, source)| (id.clone(), json!({"id":id,"service":source["service"],"revision":source["revision"],"textDigest":source["textDigest"],"evidenceDigest":source["evidenceDigest"],"file":source["file"],"startLine":source["startLine"],"endLine":source["endLine"],"authority":source["authority"],"bytes":source["text"].as_str().map_or(0,str::len)}))).collect();
    let source_bytes: usize = source_refs
        .values()
        .map(|s| s["bytes"].as_u64().unwrap_or(0) as usize)
        .sum();
    for row in &mut frontier {
        row["category"] = json!(match row["kind"].as_str() {
            Some("BUDGET") => "BUDGET",
            Some("DISCOVERY") => "DISCOVERY",
            _ => "SEMANTIC",
        });
    }
    frontier.sort_by_key(Value::to_string);
    let method_rows: Vec<_> = methods.iter().map(|m| {
        let mask = m["id"].as_str().and_then(|id| backward.get(id)).copied().unwrap_or(0);
        let outcome_ids: Vec<_> = query.outcome_event_ids.iter().enumerate()
            .filter_map(|(bit, event)| (mask & (1_u32 << bit) != 0).then_some(event)).collect();
        json!({"id":m["id"],"symbol":m["symbol"],"scope":m["scope"],"service":m["service"],"sourceIds":m["sourceIds"],"eventIds":m["eventIds"],"eventAssociation":m["eventAssociation"],"providerSlots":m["providerSlots"],"callBindings":m["callBindings"],"onStructuralOutcomePath":mask != 0,"structuralOutcomeEventIds":outcome_ids,"semanticClosure":"UNKNOWN"})
    }).collect();
    let event_rows: Vec<_> = events.iter().map(|e| json!({"id":e["id"],"ownerSymbol":e["symbol"],"sourceIds":e["sourceIds"],"factDigest":e["digest"],"detail":e["normalized"]})).collect();
    let mut value = json!({
        "schema":"codeclew-process-extraction-packet/0.1","policy":POLICY,
        "implementationDigest":crate::canonical::hash_bytes(concat!(include_str!("process_extract.rs"),include_str!("process_graph.rs"),include_str!("process_call_bindings.rs"),include_str!("process_reachability.rs")).as_bytes()),
        "snapshot":snapshot,"inputDigest":checked.input_digest,"contextDigest":checked.context_digest,
        "query":query,"rootFactDigest":root.digest,"serviceDigest":checked.services[&query.service].service_digest,
        "extractor":checked.services[&query.service].extractor,"runtimeMode":checked.services[&query.service].runtime_mode,
        "status":if graph["coverage"]["knownReachableCollection"] == "TRUNCATED" { "TRUNCATED" } else { "COLLECTED_WITH_BOUNDARIES" },
        "programSemanticClosure":false,"runtime":"UNKNOWN","methods":method_rows,"events":event_rows,
        "unboundEvents":rows("unboundEvents"),"edges":edges,"outcomes":outcome_results,"sourceRefs":source_refs,"frontier":frontier,
        "characteristics":{"retainedMethods":methods.len(),"retainedEvents":events.len(),"retainedCallSites":events.iter().filter(|e| matches!(e["normalized"]["kind"].as_str(),Some("CALL"|"CONSTRUCT"))).count(),"retainedDecisionEvents":events.iter().filter(|e| matches!(e["normalized"]["kind"].as_str(),Some("IF"|"LOOP"|"WHEN"))).count(),"retainedThrowEvents":events.iter().filter(|e|e["normalized"]["kind"]=="THROW").count(),"runtimeIoCount":"UNKNOWN","timeBounds":"UNKNOWN","strictStateMachine":"NOT_ESTABLISHED"},
        "interpretationPlan":{"workers":1,"reason":"No independently closed relevance cuts; share one immutable packet","actualModelCalls":0,"uniqueSourceBytes":source_bytes,"duplicatedSourceBytes":0},
        "usage":"Source references remain in the retained Check. Absence or a budget cut never certifies irrelevance. Business labels and effect interpretations need a separate proposal and review."
    });
    value["packetDigest"] = json!(digest(&value)?);
    if super::bytes(&value)?.len() > MAX_PACKET_BYTES {
        return Err(invalid(
            "process extraction packet exceeds 1 MiB; narrow query",
        ));
    }
    Ok(value)
}

pub(super) fn project(
    checked: &Check,
    subject: &str,
    snapshot: &str,
) -> Result<Option<Value>, ClewError> {
    let Some(id) = subject.strip_prefix("scenario:") else {
        return Ok(None);
    };
    let Some(definition) = checked.dependencies.get(&format!("process:{id}")) else {
        return Ok(None);
    };
    let scenario: super::model::Scenario =
        serde_json::from_value(definition.normalized["definition"].clone())
            .map_err(|_| invalid("invalid captured process"))?;
    let Some(query) = scenario.process.and_then(|p| p.extraction) else {
        return Ok(None);
    };
    if scenario.root.service != query.service {
        return Err(invalid(
            "extraction service differs from saved process root",
        ));
    }
    let service = checked
        .services
        .get(&query.service)
        .ok_or_else(|| invalid("process root service is unavailable"))?;
    let matches = super::analysis::resolve(scenario.root.selector.as_ref(), service);
    if matches.len() != 1 || matches[0].id != query.root_declaration {
        return Err(invalid(
            "extraction declaration differs from exact saved process root",
        ));
    }
    extract(checked, snapshot, &query).map(Some)
}

#[cfg(test)]
mod criterion_membership_tests {
    use super::*;
    #[test]
    fn ambiguous_owner_or_event_membership_never_becomes_an_outcome_seed() {
        let checked: Check = serde_json::from_value(json!({
            "schema":"synthetic-test", "inputDigest":"input", "contextDigest":"context",
            "services":{"svc":{"schema":"codeclew-documentation-service-evidence/1.0", "service":"svc", "revision":"rev", "serviceDigest":"service", "extractor":"SYNTHETIC_TEST", "runtimeMode":"SYNTHETIC_TEST", "coverage":"PARTIAL", "boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}}},
            "dependencies":{},"interactions":{},"scenarios":{},"unresolved":{}
        })).unwrap();
        let root: Observation = serde_json::from_value(json!({"id":"a", "kind":"SYMBOL", "service":"svc", "symbol":"a", "normalized":{}, "digest":"root", "sourceIds":[]})).unwrap();
        let query = Query {
            purpose: crate::thread_question::Purpose::Process,
            service: "svc".into(),
            root_declaration: "a".into(),
            configuration_scope: "UNSPECIFIED".into(),
            outcome_event_ids: vec!["criterion".into()],
            budgets: Budgets::default(),
        };
        for case in [
            "unique",
            "two-owners",
            "repeated-owner-slot",
            "duplicate-events",
            "event-missing",
        ] {
            let mut graph = json!({"methods":[{"id":"a","eventIds":["criterion"]},{"id":"b","eventIds":[]}],"events":[{"id":"criterion"}],"edges":[],"sources":{},"frontiers":[],"coverage":{"knownReachableCollection":"COMPLETE"}});
            match case {
                "two-owners" => graph["methods"][1]["eventIds"] = json!(["criterion"]),
                "repeated-owner-slot" => {
                    graph["methods"][0]["eventIds"] = json!(["criterion", "criterion"])
                }
                "duplicate-events" => {
                    graph["events"] = json!([{"id":"criterion"},{"id":"criterion"}])
                }
                "event-missing" => graph["events"] = json!([]),
                _ => {}
            }
            let result = packet(&checked, "snapshot", &query, &graph, &root).unwrap();
            assert_eq!(
                result["outcomes"][0]["discovery"] == "FOUND_IN_RETAINED_ENVELOPE",
                case == "unique",
                "{case}"
            );
            assert_eq!(
                result["outcomes"][0]["structuralMethodIds"],
                if case == "unique" {
                    json!(["a"])
                } else {
                    json!([])
                },
                "{case}"
            );
            assert_eq!(result["programSemanticClosure"], false);
        }
    }
}
