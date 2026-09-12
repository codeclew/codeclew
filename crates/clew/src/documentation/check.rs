//! Explicit certainty axes and bounded composition across declared HTTP and Kafka boundaries.
use super::{analysis, bytes, digest, invalid, io_error, model::*, store::Repository};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resolution {
    pub status: String,
    pub candidates: Vec<String>,
    pub source_ids: Vec<String>,
    pub details: Vec<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionCheck {
    pub id: String,
    pub origin: String,
    pub declaration_digest: String,
    pub from: Resolution,
    pub to: Resolution,
    pub call_site: Resolution,
    pub method: Value,
    pub path: Value,
    pub destination: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub topic: Value,
    pub contract_status: String,
    pub runtime: String,
    pub applicability: Option<Applicability>,
    pub boundaries: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowStep {
    pub id: String,
    pub service: String,
    pub symbol: String,
    pub kind: String,
    pub dependency_ids: Vec<String>,
    pub source_ids: Vec<String>,
    pub detail: Value,
    pub depth: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioContext {
    pub id: String,
    pub steps: Vec<FlowStep>,
    pub boundaries: Vec<String>,
    pub truncated: bool,
    pub dependency_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Check {
    pub schema: String,
    pub input_digest: String,
    pub context_digest: String,
    pub services: BTreeMap<String, ServiceEvidence>,
    pub unresolved: BTreeMap<String, Value>,
    pub interactions: BTreeMap<String, InteractionCheck>,
    pub scenarios: BTreeMap<String, ScenarioContext>,
    pub dependencies: BTreeMap<String, Observation>,
}

pub fn run(repository: &Repository) -> Result<Check, ClewError> {
    run_selected(repository, &BTreeSet::new())
}

pub fn run_selected(
    repository: &Repository,
    selected: &BTreeSet<String>,
) -> Result<Check, ClewError> {
    let input_digest = repository.input_digest()?;
    let services = repository.services()?;
    if selected.iter().any(|id| !services.contains_key(id)) {
        return Err(invalid("selected documentation service does not exist"));
    }
    let interactions = repository.interactions()?;
    let scenarios = repository.scenarios()?;
    let mut evidence = BTreeMap::new();
    let mut unresolved = BTreeMap::new();
    for (id, service) in &services {
        if !selected.is_empty() && !selected.contains(id) {
            unresolved.insert(id.clone(), json!({"status":"NOT_CHECKED","reason":"SERVICE_NOT_SELECTED","nextAction":"Select this service explicitly to check its current source."}));
            continue;
        }
        match analysis::capture(repository, service) {
            Ok(value) => {
                evidence.insert(id.clone(), value);
            }
            Err(error) => {
                let mut failure =
                    json!({"status":"UNRESOLVED","reason":error.code,"nextAction":error.message});
                if let Some(diagnostic) = crate::worker_diagnostics::from_evidence(&error.evidence)
                {
                    failure["workerFailure"] = diagnostic;
                }
                unresolved.insert(id.clone(), failure);
            }
        }
    }
    if input_digest != repository.input_digest()? {
        return Err(invalid("documentation input changed during checking"));
    }
    let mut checked = assemble(
        input_digest,
        evidence,
        unresolved,
        &interactions,
        &scenarios,
    )?;
    if let Some((_, baseline)) = super::bindings::baseline(repository)? {
        super::review::scopes(
            repository,
            &mut checked,
            baseline.accepted_versions.into_values(),
        )?;
    }
    Ok(checked)
}

pub fn assemble(
    input_digest: String,
    services: BTreeMap<String, ServiceEvidence>,
    unresolved: BTreeMap<String, Value>,
    interactions: &BTreeMap<String, Interaction>,
    scenarios: &BTreeMap<String, Scenario>,
) -> Result<Check, ClewError> {
    let mut dependencies: BTreeMap<String, Observation> = services
        .values()
        .flat_map(|s| s.observations.clone())
        .collect();
    for interaction in interactions.values() {
        let id = format!("interaction:{}", interaction.id);
        let value = serde_json::to_value(interaction).map_err(io_error)?;
        dependencies.insert(
            id.clone(),
            Observation {
                id,
                kind: "DECLARED_INTERACTION".into(),
                service: String::new(),
                symbol: interaction.id.clone(),
                digest: digest(&value)?,
                normalized: value,
                source_ids: vec![],
            },
        );
    }
    for scenario in scenarios.values() {
        let id = format!("scenario:{}", scenario.id);
        let value = serde_json::to_value(scenario).map_err(io_error)?;
        dependencies.insert(
            id.clone(),
            Observation {
                id,
                kind: "SCENARIO_SELECTION".into(),
                service: String::new(),
                symbol: scenario.id.clone(),
                digest: digest(&value)?,
                normalized: value,
                source_ids: vec![],
            },
        );
    }
    let interactions_checked = interactions
        .iter()
        .map(|(id, i)| Ok((id.clone(), check_interaction(i, &services)?)))
        .collect::<Result<BTreeMap<_, _>, ClewError>>()?;
    let scenarios_checked = scenarios
        .iter()
        .map(|(id, s)| {
            Ok((
                id.clone(),
                compose(s, &services, interactions, &interactions_checked)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ClewError>>()?;
    // Semantic context excludes source line coordinates; relocation still updates source links.
    let context_digest = digest(
        &json!({"inputDigest":input_digest,"extractor":EXTRACTOR,"dependencies":dependencies.iter().map(|(id,d)|(id,&d.digest)).collect::<BTreeMap<_,_>>(),"coverage":services.iter().map(|(id,e)|(id,json!([e.coverage,e.boundaries]))).collect::<BTreeMap<_,_>>(),"unresolved":unresolved}),
    )?;
    Ok(Check {
        schema: "codeclew-documentation-check/1.0".into(),
        input_digest,
        context_digest,
        services,
        unresolved,
        interactions: interactions_checked,
        scenarios: scenarios_checked,
        dependencies,
    })
}

fn resolution(endpoint: &Endpoint, services: &BTreeMap<String, ServiceEvidence>) -> Resolution {
    let matches = services
        .get(&endpoint.service)
        .map(|e| analysis::resolve(endpoint.selector.as_ref(), e))
        .unwrap_or_default();
    Resolution {
        status: if endpoint.selector.is_none() {
            "INCOMPLETE"
        } else {
            match matches.len() {
                0 => "MISSING",
                1 => {
                    if matches[0].source_ids.is_empty() {
                        "SOURCE_UNAVAILABLE"
                    } else if matches[0].normalized["authority"] == "SYNTAX" {
                        "SOURCE_MATCH"
                    } else {
                        "RESOLVED"
                    }
                }
                _ => "AMBIGUOUS",
            }
        }
        .into(),
        candidates: matches.iter().map(|o| o.id.clone()).collect(),
        source_ids: matches.iter().flat_map(|o| o.source_ids.clone()).collect(),
        details: matches.iter().map(|o| json!({"id":o.id,"symbol":o.symbol,"descriptor":o.normalized["jvmDescriptor"],"parameterTypes":o.normalized.pointer("/documentation/parameterTypes"),"sourceIds":o.source_ids})).collect(),
    }
}
fn compare(declared: Option<&str>, observed: Vec<String>) -> Value {
    let unique: BTreeSet<_> = observed.into_iter().collect();
    let status = if declared.is_none() || unique.is_empty() {
        "UNRESOLVED"
    } else if unique.len() != 1 {
        "AMBIGUOUS"
    } else if unique.first().map(String::as_str) == declared {
        "MATCH"
    } else {
        "MISMATCH"
    };
    json!({"status":status,"declared":declared,"observed":unique})
}
fn compare_topic(declared: Option<&str>, observed: Vec<String>) -> Value {
    let dynamic = |value: &str| value.contains("${") || value.contains("#{");
    let unresolved = declared.is_some_and(dynamic) || observed.iter().any(|s| dynamic(s));
    let mut result = compare(declared, observed);
    if unresolved {
        result["status"] = json!("UNRESOLVED");
        result["reason"] = json!("TOPIC_CONFIGURATION_EXPRESSION_NOT_RESOLVED");
    }
    result
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn check_interaction(
    i: &Interaction,
    services: &BTreeMap<String, ServiceEvidence>,
) -> Result<InteractionCheck, ClewError> {
    let transport = i.transport.kind.as_str();
    let from = resolution(&i.from, services);
    let to = resolution(&i.to, services);
    let mut calls = Vec::new();
    if from.status == "RESOLVED" {
        let e = &services[&i.from.service];
        let symbol = &e.observations[&from.candidates[0]].symbol;
        let events = e
            .observations
            .values()
            .filter(|o| o.kind == "FLOW" && o.symbol == *symbol && o.normalized["kind"] == "CALL");
        calls = events
            .filter(|o| {
                if let Some(call) = &i.from.call_site {
                    o.normalized["target"] == call.target
                } else {
                    o.normalized[transport]
                        .as_object()
                        .is_some_and(|h| !h.is_empty())
                }
            })
            .collect();
        // Source ordering, not hash/map ordering, controls an explicitly selected ordinal.
        calls.sort_by_key(|o| o.normalized["ordinal"].as_u64().unwrap_or(0));
        if let Some(ordinal) = i.from.call_site.as_ref().and_then(|c| c.ordinal) {
            calls = calls.get(ordinal).copied().into_iter().collect();
        }
    }
    let call_site = Resolution {
        status: match calls.len() {
            0 => "MISSING",
            1 => "RESOLVED",
            _ => "AMBIGUOUS",
        }
        .into(),
        candidates: calls.iter().map(|o| o.id.clone()).collect(),
        source_ids: calls.iter().flat_map(|o| o.source_ids.clone()).collect(),
        details: calls.iter().enumerate().map(|(ordinal, o)| json!({"id":o.id,"target":o.normalized["target"],"selectedOrdinal":ordinal,"transport":o.normalized[transport],"sourceIds":o.source_ids})).collect(),
    };
    let caller = if calls.len() == 1 {
        calls[0].normalized[transport].clone()
    } else {
        Value::Null
    };
    let receiver: Vec<_> = if to.status == "RESOLVED" {
        let e = &services[&i.to.service];
        let symbol = &e.observations[&to.candidates[0]].symbol;
        e.entrypoints
            .iter()
            .filter(|r| {
                r.symbol == *symbol
                    && r.kind
                        == if transport == "kafka" {
                            "KAFKA_LISTENER"
                        } else {
                            "HTTP_ENDPOINT"
                        }
            })
            .collect()
    } else {
        vec![]
    };
    let server_methods: Vec<_> = receiver
        .iter()
        .flat_map(|r| strings(&r.trigger["methods"]))
        .collect();
    let server_paths: Vec<_> = receiver
        .iter()
        .flat_map(|r| strings(&r.trigger["paths"]))
        .collect();
    let method = json!({"caller":compare(i.transport.method.as_deref(),caller["method"].as_str().map(str::to_owned).into_iter().collect()),"receiver":compare(i.transport.method.as_deref(),server_methods)});
    let path = json!({"caller":compare(i.transport.path.as_deref(),caller["path"].as_str().map(str::to_owned).into_iter().collect()),"receiver":compare(i.transport.path.as_deref(),server_paths)});
    let destination = compare(
        i.transport.destination_config_key.as_deref(),
        caller["destinationConfigKey"]
            .as_str()
            .map(str::to_owned)
            .into_iter()
            .collect(),
    );
    let topic = if transport == "kafka" {
        json!({"caller":compare_topic(i.transport.topic.as_deref(), caller["topic"].as_str().map(str::to_owned).into_iter().collect()),
            "receiver":compare_topic(i.transport.topic.as_deref(), receiver.iter().flat_map(|r| strings(&r.trigger["configuration"]["topics"])).collect())})
    } else {
        Value::Null
    };
    let mut boundaries = vec![
        "RUNTIME_DESTINATION_AND_ACTIVATION_UNKNOWN".into(),
        "WIRE_SERIALIZATION_NOT_ASSESSED".into(),
    ];
    if transport == "kafka" {
        boundaries.push("MESSAGE_DELIVERY_ORDER_RETRY_AND_CONSUMER_ACTIVATION_UNKNOWN".into());
    }
    if i.applicability
        .as_ref()
        .is_none_or(|a| a.environments.is_empty())
    {
        boundaries.push("ENVIRONMENT_SCOPE_UNSPECIFIED".into());
    }
    if caller.is_null() || caller.as_object().is_some_and(|o| o.is_empty()) {
        boundaries.push("CLIENT_PATTERN_UNSUPPORTED_OR_AMBIGUOUS".into());
    }
    if i.declaration.origin == "agent-proposal" {
        boundaries.push("UNACCEPTED_AGENT_PROPOSAL".into());
    }
    Ok(InteractionCheck {
        id: i.id.clone(),
        origin: i.declaration.origin.clone(),
        declaration_digest: digest(i)?,
        from,
        to,
        call_site,
        method,
        path,
        destination,
        topic,
        contract_status: "NOT_ASSESSED".into(),
        runtime: "UNKNOWN".into(),
        applicability: i.applicability.clone(),
        boundaries,
    })
}

struct Walker<'a> {
    services: &'a BTreeMap<String, ServiceEvidence>,
    interactions: &'a BTreeMap<String, Interaction>,
    checks: &'a BTreeMap<String, InteractionCheck>,
    selection: &'a Scenario,
    steps: Vec<FlowStep>,
    boundaries: BTreeSet<String>,
    dependencies: BTreeSet<String>,
    active: BTreeSet<(String, String)>,
    truncated: bool,
    transitions: BTreeSet<String>,
}
impl Walker<'_> {
    fn push(&mut self, step: FlowStep) {
        if self.steps.len() >= self.selection.max_nodes {
            self.truncated = true;
            self.boundaries.insert("NODE_BUDGET_EXHAUSTED".into());
            return;
        }
        self.dependencies
            .extend(step.dependency_ids.iter().cloned());
        self.steps.push(step);
    }
    fn walk(&mut self, service: &str, symbol: &str, depth: usize) -> Result<(), ClewError> {
        if depth > self.selection.max_depth {
            self.truncated = true;
            self.boundaries.insert("DEPTH_BUDGET_EXHAUSTED".into());
            return Ok(());
        }
        if !self.active.insert((service.into(), symbol.into())) {
            self.boundaries
                .insert("LOCAL_OR_DECLARED_CYCLE_NOT_EXPANDED".into());
            return Ok(());
        }
        let Some(e) = self.services.get(service) else {
            self.boundaries
                .insert("SERVICE_EVIDENCE_UNAVAILABLE".into());
            return Ok(());
        };
        let Some(declaration) = e
            .observations
            .values()
            .find(|o| o.kind == "SYMBOL" && o.symbol == symbol)
        else {
            self.boundaries.insert("CALLEE_BODY_UNAVAILABLE".into());
            return Ok(());
        };
        self.dependencies.insert(declaration.id.clone());
        let flow = declaration.normalized.get("documentation");
        if flow.is_none() {
            self.boundaries.insert("METHOD_FLOW_UNAVAILABLE".into());
        }
        if let Some(boundaries) = flow.and_then(|f| f["boundaries"].as_array()) {
            self.boundaries.extend(
                boundaries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
        }
        let events = flow
            .and_then(|f| f["events"].as_array())
            .cloned()
            .unwrap_or_default();
        for (ordinal, event) in events.iter().enumerate() {
            if self.steps.len() >= self.selection.max_nodes {
                self.truncated = true;
                self.boundaries.insert("NODE_BUDGET_EXHAUSTED".into());
                break;
            }
            let id =
                analysis::dependency_id(service, "flow", &format!("{symbol}/event/{ordinal}"))?;
            let Some(observation) = e.observations.get(&id) else {
                self.boundaries.insert("FLOW_EVENT_EVIDENCE_MISSING".into());
                continue;
            };
            self.push(FlowStep {
                id: format!("step-{}", self.steps.len()),
                service: service.into(),
                symbol: symbol.into(),
                kind: event["kind"].as_str().unwrap_or("BOUNDARY").into(),
                dependency_ids: vec![id.clone(), declaration.id.clone()],
                source_ids: observation.source_ids.clone(),
                detail: event.clone(),
                depth,
            });
            let mut bridged = false;
            let competing = self
                .selection
                .interactions
                .iter()
                .filter(|interaction_id| {
                    let interaction = &self.interactions[*interaction_id];
                    let checked = &self.checks[*interaction_id];
                    interaction.from.service == service
                        && checked.call_site.status == "RESOLVED"
                        && checked.call_site.candidates.first() == Some(&id)
                })
                .count()
                > 1;
            if competing {
                self.boundaries
                    .insert("AMBIGUOUS_DECLARED_DESTINATION".into());
            }
            for interaction_id in &self.selection.interactions {
                if competing {
                    break;
                }
                let interaction = &self.interactions[interaction_id];
                let checked = &self.checks[interaction_id];
                if interaction.from.service != service
                    || checked.call_site.status != "RESOLVED"
                    || checked.call_site.candidates.first() != Some(&id)
                {
                    continue;
                }
                if interaction.declaration.origin == "agent-proposal" {
                    self.boundaries.insert("UNACCEPTED_AGENT_PROPOSAL".into());
                    continue;
                }
                if checked.to.status != "RESOLVED" {
                    self.boundaries
                        .insert("DECLARED_RECEIVER_NOT_RESOLVED".into());
                    continue;
                }
                let receiver =
                    &self.services[&interaction.to.service].observations[&checked.to.candidates[0]];
                let mut deps = vec![
                    format!("interaction:{interaction_id}"),
                    id.clone(),
                    receiver.id.clone(),
                ];
                for entry in &self.services[&interaction.to.service].entrypoints {
                    if entry.symbol == receiver.symbol {
                        deps.extend(entry.dependency_ids.clone());
                    }
                }
                self.push(FlowStep{id:format!("step-{}",self.steps.len()),service:service.into(),symbol:symbol.into(),kind:if interaction.transport.kind == "kafka" {"DECLARED_KAFKA_TRANSITION"} else {"DECLARED_HTTP_TRANSITION"}.into(),dependency_ids:deps,source_ids:checked.call_site.source_ids.iter().chain(checked.to.source_ids.iter()).cloned().collect(),detail:json!({"interaction":interaction_id,"origin":interaction.declaration.origin,"toService":interaction.to.service,"runtime":"UNKNOWN","transport":interaction.transport.kind,"checks":checked}),depth});
                self.transitions.insert(interaction_id.clone());
                bridged = true;
                self.walk(&interaction.to.service, &receiver.symbol, depth + 1)?;
            }
            if !bridged
                && matches!(event["kind"].as_str(), Some("CALL" | "CONSTRUCT"))
                && let Some(target) = event["target"].as_str()
            {
                if e.observations
                    .values()
                    .any(|o| o.kind == "SYMBOL" && o.symbol == target)
                {
                    self.walk(service, target, depth + 1)?;
                } else {
                    self.boundaries
                        .insert("EXTERNAL_CALL_BODY_NOT_EXPANDED".into());
                }
            }
        }
        self.active.remove(&(service.into(), symbol.into()));
        Ok(())
    }
}

pub fn compose(
    s: &Scenario,
    services: &BTreeMap<String, ServiceEvidence>,
    interactions: &BTreeMap<String, Interaction>,
    checks: &BTreeMap<String, InteractionCheck>,
) -> Result<ScenarioContext, ClewError> {
    let mut walker = Walker {
        services,
        interactions,
        checks,
        selection: s,
        steps: vec![],
        boundaries: BTreeSet::new(),
        dependencies: BTreeSet::from([format!("scenario:{}", s.id)]),
        active: BTreeSet::new(),
        truncated: false,
        transitions: BTreeSet::new(),
    };
    let involved: BTreeSet<_> = s
        .interactions
        .iter()
        .flat_map(|id| {
            [
                interactions[id].from.service.as_str(),
                interactions[id].to.service.as_str(),
            ]
        })
        .chain([s.root.service.as_str()])
        .collect();
    if involved.len() > 8 {
        walker
            .boundaries
            .insert("MORE_THAN_EIGHT_SERVICES_NOT_SUPPORTED_IN_ONE_SCENARIO".into());
    } else {
        let root = resolution(&s.root, services);
        if matches!(root.status.as_str(), "RESOLVED" | "SOURCE_MATCH") {
            walker.walk(
                &s.root.service,
                &services[&s.root.service].observations[&root.candidates[0]].symbol,
                0,
            )?;
        } else {
            walker
                .boundaries
                .insert(format!("SCENARIO_ROOT_{}", root.status));
        }
    }
    for interaction in &s.interactions {
        if !walker.transitions.contains(interaction) {
            walker
                .boundaries
                .insert(format!("DECLARED_TRANSITION_NOT_REACHED:{interaction}"));
        }
        walker
            .dependencies
            .insert(format!("interaction:{interaction}"));
    }
    Ok(ScenarioContext {
        id: s.id.clone(),
        steps: walker.steps,
        boundaries: walker.boundaries.into_iter().collect(),
        truncated: walker.truncated,
        dependency_ids: walker.dependencies.into_iter().collect(),
    })
}

impl Check {
    pub(super) fn refresh_digest(&mut self) -> Result<(), ClewError> {
        self.context_digest = digest(
            &json!({"inputDigest":self.input_digest,"extractor":EXTRACTOR,"dependencies":self.dependencies.iter().map(|(id,d)|(id,&d.digest)).collect::<BTreeMap<_,_>>(),"coverage":self.services.iter().map(|(id,e)|(id,json!([e.coverage,e.boundaries]))).collect::<BTreeMap<_,_>>(),"unresolved":self.unresolved}),
        )?;
        Ok(())
    }

    pub fn sources(&self) -> BTreeMap<String, Source> {
        self.services
            .values()
            .flat_map(|s| s.sources.clone())
            .collect()
    }
    pub fn summary(&self) -> Value {
        json!({"schema":self.schema,"inputDigest":self.input_digest,"contextDigest":self.context_digest,"status":if self.unresolved.is_empty(){"CHECKED"}else{"UNRESOLVED"},"services":self.services.iter().map(|(id,e)|(id,json!({"revision":e.revision,"coverage":e.coverage,"entrypoints":e.entrypoints.len(),"boundaries":e.boundaries}))).collect::<BTreeMap<_,_>>(),"unresolved":self.unresolved,"interactions":self.interactions,"scenarios":self.scenarios.iter().map(|(id,s)|(id,json!({"steps":s.steps.len(),"truncated":s.truncated,"boundaries":s.boundaries}))).collect::<BTreeMap<_,_>>()})
    }
    pub fn save(&self, repo: &Repository) -> Result<(), ClewError> {
        let _lock = repo.lock()?;
        let encoded = bytes(self)?;
        if encoded.len() > 64 * 1024 * 1024 {
            return Err(crate::error::ClewError::new(
                crate::error::ErrorCode::SliceBudgetExceeded,
                "documentation check exceeds its portable cache budget; narrow source roots",
            ));
        }
        repo.atomic(".codeclew/cache/latest-check.json", &encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java_adapter_v2::{JavaCompilerFact, build_java_compiler_index};
    use crate::java_project_model::extract_java_model;
    use std::fs;

    #[test]
    fn kafka_configuration_expressions_are_not_literal_topic_mismatches() {
        assert_eq!(
            compare_topic(Some("stock-dev"), vec!["${topics.stock}".into()])["status"],
            "UNRESOLVED"
        );
        assert_eq!(
            compare_topic(Some("stock-${STAND_NAME}"), vec!["stock-dev".into()])["status"],
            "UNRESOLVED"
        );
        assert_eq!(
            compare_topic(Some("stock"), vec!["stock".into()])["status"],
            "MATCH"
        );
        assert_eq!(
            compare_topic(Some("stock"), vec!["other".into()])["status"],
            "MISMATCH"
        );
    }

    #[test]
    #[ignore = "launches Maven and javac for the two-service acceptance fixture"]
    fn two_java_services_resolve_and_compose_declared_http_with_branches() {
        let temporary = tempfile::tempdir().unwrap();
        let mut services = BTreeMap::new();
        for id in ["orders", "inventory"] {
            let fixture = crate::worker::workspace_root()
                .join("fixtures/durable-docs")
                .join(id);
            let root = temporary.path().join(id);
            fs::create_dir_all(&root).unwrap();
            for item in walkdir::WalkDir::new(&fixture)
                .into_iter()
                .filter_entry(|e| e.file_name() != "target")
            {
                let item = item.unwrap();
                let path = root.join(item.path().strip_prefix(&fixture).unwrap());
                if item.file_type().is_dir() {
                    fs::create_dir_all(path).unwrap();
                } else {
                    fs::copy(item.path(), path).unwrap();
                }
            }
            let model = extract_java_model(&root, ":/main").unwrap();
            let hashes = model
                .authority
                .source_files
                .iter()
                .map(|file| {
                    (
                        file.clone(),
                        crate::canonical::hash_bytes(&fs::read(root.join(file)).unwrap()),
                    )
                })
                .collect();
            let index = build_java_compiler_index(&root, &model, &hashes).unwrap();
            let facts: Vec<_> = index
                .facts
                .iter()
                .map(|fact| (serde_json::to_value(fact).unwrap(), digest(fact).unwrap()))
                .collect();
            assert!(
                !index
                    .facts
                    .iter()
                    .any(|f| matches!(f, JavaCompilerFact::Boundary { .. })),
                "{facts:?}"
            );
            let mut files: BTreeMap<_, _> = model
                .authority
                .source_files
                .iter()
                .map(|file| (file.clone(), fs::read_to_string(root.join(file)).unwrap()))
                .collect();
            let contract = "src/main/resources/openapi.yaml";
            files.insert(
                contract.into(),
                fs::read_to_string(root.join(contract)).unwrap(),
            );
            let service = Service {
                schema: "codeclew-documentation-service/1.0".into(),
                id: id.into(),
                title: id.into(),
                repository_id: id.into(),
                repository: format!("https://example.invalid/{id}"),
                language: "java".into(),
                profile: "java-17plus-maven-read-only".into(),
                source: None,
                compilation: ":/main".into(),
                target_ref: "main".into(),
                source_link_template: None,
                contract_files: vec![contract.into()],
            };
            let evidence = analysis::project(
                &service,
                &"1".repeat(40),
                &digest(&service).unwrap(),
                "DEVELOPMENT",
                "PARTIAL",
                facts,
                &files,
            )
            .unwrap();
            services.insert(id.into(), evidence);
        }
        let selector = |service: &str, owner: &str, name: &str| Endpoint {
            service: service.into(),
            selector: Some(Selector {
                language: "java".into(),
                owner: owner.into(),
                name: name.into(),
                parameter_types: Some(vec![format!("example.{service}.ReservationRequest")]),
            }),
            call_site: None,
        };
        let interaction = Interaction {
            schema: "codeclew-documentation-interaction/1.0".into(),
            id: "reserve".into(),
            title: "Reserve inventory".into(),
            from: selector("orders", "example.orders.InventoryClient", "reserve"),
            to: selector(
                "inventory",
                "example.inventory.ReservationController",
                "create",
            ),
            transport: Transport {
                kind: "http".into(),
                topic: None,
                method: Some("POST".into()),
                path: Some("/reservations".into()),
                destination_config_key: Some("inventory.base-url".into()),
            },
            declaration: Declaration {
                origin: "human".into(),
                rationale: "The engineer declared this link.".into(),
            },
            applicability: None,
            contract_reference: None,
        };
        let scenario = Scenario {
            schema: "codeclew-documentation-scenario/1.0".into(),
            id: "checkout".into(),
            title: "Checkout".into(),
            summary: "Reserve stock for an order.".into(),
            root: selector("orders", "example.orders.CheckoutController", "checkout"),
            interactions: vec!["reserve".into()],
            max_depth: 4,
            max_nodes: 64,
        };
        let checked = assemble(
            "input".into(),
            services.clone(),
            BTreeMap::new(),
            &BTreeMap::from([("reserve".into(), interaction.clone())]),
            &BTreeMap::from([("checkout".into(), scenario)]),
        )
        .unwrap();
        let result = &checked.interactions["reserve"];
        assert_eq!(result.from.status, "RESOLVED");
        assert_eq!(result.to.status, "RESOLVED");
        assert_eq!(result.call_site.status, "RESOLVED", "{result:?}");
        assert_eq!(result.method["caller"]["status"], "MATCH");
        assert_eq!(result.method["receiver"]["status"], "MATCH");
        assert_eq!(result.path["caller"]["status"], "MATCH");
        assert_eq!(result.path["receiver"]["status"], "MATCH");
        assert_eq!(result.destination["status"], "MATCH", "{result:?}");
        assert_eq!(result.runtime, "UNKNOWN");
        assert_eq!(result.origin, "human");
        let scenario = &checked.scenarios["checkout"];
        assert!(
            scenario
                .steps
                .iter()
                .any(|s| s.kind == "DECLARED_HTTP_TRANSITION")
        );
        for service in ["orders", "inventory"] {
            assert!(
                scenario
                    .steps
                    .iter()
                    .any(|s| s.service == service && s.kind == "IF")
            );
        }
        assert!(!scenario.truncated);
        assert!(
            !scenario
                .boundaries
                .iter()
                .any(|b| b.starts_with("DECLARED_TRANSITION_NOT_REACHED"))
        );
        let baseline = super::super::render::make_bindings(&checked, BTreeMap::new()).unwrap();
        let mut changed_contract = checked.clone();
        let dependency = changed_contract
            .dependencies
            .values_mut()
            .find(|o| o.kind == "CONTRACT_OPERATION" && o.service == "inventory")
            .unwrap();
        dependency.normalized["operation"]["summary"] = json!("Changed reservation contract");
        dependency.digest = digest(&dependency.normalized).unwrap();
        let report = super::super::bindings::freshness(Some(&baseline), &changed_contract);
        assert!(
            report["affected"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["fragment"]
                    .as_str()
                    .unwrap()
                    .starts_with("scenario:checkout/checkout/contract-")),
            "{report}"
        );
        let mut changed = interaction;
        changed.transport.path = Some("/other".into());
        let mismatch = check_interaction(&changed, &services).unwrap();
        assert_eq!(mismatch.path["receiver"]["status"], "MISMATCH");
        assert_eq!(mismatch.path["caller"]["status"], "MISMATCH");
    }
}
