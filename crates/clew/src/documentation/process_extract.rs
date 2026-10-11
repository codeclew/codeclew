//! Query packets over retained portable documentation evidence, without execution.
//! Relevance is a conservative callable envelope, not a data/effect closure proof.
use super::{check::Check, digest, invalid, model::Observation, process_graph};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const POLICY: &str = "retained-process-envelope/0.1";
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
    let service = checked
        .services
        .get(&query.service)
        .ok_or_else(|| invalid("extraction service is unavailable"))?;
    if service.observations.len() > 131_072 {
        return Err(invalid("process input fact budget exceeded"));
    }
    let root = service
        .observations
        .get(&query.root_declaration)
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
        edge["relationshipAuthority"] = json!("RETAINED_LOCAL_ASSOCIATION_NOT_RUNTIME_PROOF");
    }
    frontier.push(
        json!({"kind":"SEMANTIC","code":"DATA_EFFECT_ALIAS_AND_RUNTIME_CLOSURE_NOT_ESTABLISHED"}),
    );
    frontier.push(json!({"kind":"SEMANTIC","code":"CROSS_SERVICE_LINKS_REQUIRE_INDEPENDENT_RELATIONSHIP_AUTHORITY"}));
    if query.configuration_scope == "UNSPECIFIED" {
        frontier.push(json!({"kind":"SEMANTIC","code":"CONFIGURATION_SCOPE_UNSPECIFIED"}));
    }
    let mut outcome_results = Vec::new();
    let mut backward = BTreeSet::new();
    for id in &query.outcome_event_ids {
        let matches: Vec<_> = methods
            .iter()
            .filter(|m| {
                m["eventIds"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|v| v == id))
            })
            .collect();
        let found = matches.len() == 1 && events.iter().any(|e| e["id"] == *id);
        if found {
            backward.insert(matches[0]["id"].as_str().unwrap_or("").to_owned());
        } else {
            frontier
                .push(json!({"kind":"DISCOVERY","code":"OUTCOME_NOT_FOUND_UNKNOWN","eventId":id}));
        }
        outcome_results.push(json!({"eventId":id,"discovery":if found { "FOUND_IN_RETAINED_ENVELOPE" } else { "NOT_FOUND_UNKNOWN" },"semanticClosure":"UNKNOWN"}));
    }
    // Fixed point over call membership only; no path feasibility or data-dependence claim.
    loop {
        let before = backward.len();
        for edge in &edges {
            if edge["targetMethodId"]
                .as_str()
                .is_some_and(|id| backward.contains(id))
            {
                if let Some(id) = edge["fromMethodId"].as_str() {
                    backward.insert(id.to_owned());
                }
            }
        }
        if backward.len() == before {
            break;
        }
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
    let method_rows: Vec<_> = methods.iter().map(|m| json!({"id":m["id"],"symbol":m["symbol"],"scope":m["scope"],"service":m["service"],"sourceIds":m["sourceIds"],"eventIds":m["eventIds"],"eventAssociation":m["eventAssociation"],"providerSlots":m["providerSlots"],"onStructuralOutcomePath":m["id"].as_str().is_some_and(|id| backward.contains(id)),"semanticClosure":"UNKNOWN"})).collect();
    let event_rows: Vec<_> = events.iter().map(|e| json!({"id":e["id"],"ownerSymbol":e["symbol"],"sourceIds":e["sourceIds"],"factDigest":e["digest"],"detail":e["normalized"]})).collect();
    let mut value = json!({
        "schema":"codeclew-process-extraction-packet/0.1","policy":POLICY,
        "implementationDigest":crate::canonical::hash_bytes(concat!(include_str!("process_extract.rs"),include_str!("process_graph.rs")).as_bytes()),
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
