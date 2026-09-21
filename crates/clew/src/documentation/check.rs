//! Explicit certainty axes and bounded composition across declared HTTP and Kafka boundaries.
use super::{analysis, bytes, digest, invalid, io_error, model::*, store::Repository};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

/// Upper bound for a portable documentation check or rendered record. Large
/// services (for example 600+ source files) legitimately exceed 64 MiB, so the
/// portable cache and render reads/writes share this same bound.
pub const PORTABLE_CACHE_MAX_BYTES: u64 = 128 * 1024 * 1024;

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
    /// Frozen source-selection inputs; required when persisting a capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_inputs: Option<SourceInputs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<super::composition::Composition>,
}

pub const SOURCE_INPUTS_SCHEMA: &str = "codeclew-documentation-source-inputs/1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceInputs {
    pub schema: String,
    pub input_digest: String,
    pub inputs: super::store::RepositoryInputs,
    pub selected_services: BTreeSet<String>,
    /// Valid saved service results carried forward without visiting their source.
    /// These are deliberately separate from this operation's capture selection.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub retained_services: BTreeSet<String>,
}

pub const CHECK_MANIFEST_SCHEMA: &str = "codeclew-documentation-check-manifest/1.0";
/// Fact-index scope under which a check's dependency observations are indexed.
pub const CHECK_DEPENDENCIES_SCOPE: &str = "check-dependencies";

/// Reference envelope for a persisted check: per-service capture manifests and
/// a fact-index root replace the duplicated full `ServiceEvidence`
/// observations plus `Check.dependencies` inline copy. Light identity stays
/// inline; heavy payload lives once in the immutable object store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckManifest {
    pub schema: String,
    pub input_digest: String,
    pub context_digest: String,
    pub service_manifests: BTreeMap<String, super::cache::CaptureManifest>,
    pub unresolved: BTreeMap<String, Value>,
    pub interactions: BTreeMap<String, InteractionCheck>,
    pub scenarios: BTreeMap<String, ScenarioContext>,
    /// Required immutable fact-index root for dependency observations.
    pub dependencies_index: super::cache::ObjectRef,
    /// Required frozen source-selection inputs.
    pub source_inputs: super::cache::ObjectRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<super::cache::ObjectRef>,
}

/// Hold a per-repository advisory lock for the whole check run. Concurrent
/// `docs check` invocations on the same repository share one state root; without
/// serialization their attempt lifecycles can overlap and one instance's cleanup
/// can remove the other's attempt materialization mid-run, surfacing as a
/// spurious `No such file or directory` during seal. Serializing the run per
/// repository prevents that overlap without masking the error.
struct RepositoryRunLock(File);
impl Drop for RepositoryRunLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn acquire_repository_run_lock(repository: &Repository) -> Result<RepositoryRunLock, ClewError> {
    // The lock belongs to the docs root, not CODECLEW_HOME: separate runtime
    // installations can legitimately update the same documentation repository.
    std::fs::create_dir_all(repository.path(".codeclew")?).map_err(io_error)?;
    let path = repository.path(".codeclew/check-run.lock")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid("documentation run lock is not a regular file"));
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(RepositoryRunLock(file))
}

pub fn run(repository: &Repository) -> Result<Check, ClewError> {
    run_selected(repository, &BTreeSet::new())
}

pub fn run_selected(
    repository: &Repository,
    selected: &BTreeSet<String>,
) -> Result<Check, ClewError> {
    run_selected_with_diagnostics(repository, selected, None)
}

pub(crate) fn run_selected_with_diagnostics(
    repository: &Repository,
    selected: &BTreeSet<String>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<Check, ClewError> {
    let _run_lock = acquire_repository_run_lock(repository)?;
    run_selected_unlocked(repository, selected, debug_output)
}

/// Keep sibling selection and publication under the same operation lock. Two
/// independent scoped checks must never overwrite each other's newer siblings.
pub(crate) fn run_and_save_selected(
    repository: &Repository,
    selected: &BTreeSet<String>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<(Check, String), ClewError> {
    let _run_lock = acquire_repository_run_lock(repository)?;
    let checked = run_selected_unlocked(repository, selected, debug_output)?;
    let snapshot = checked.save_snapshot(repository)?;
    Ok((checked, snapshot))
}

fn run_selected_unlocked(
    repository: &Repository,
    selected: &BTreeSet<String>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<Check, ClewError> {
    let inputs = repository.inputs()?;
    let mut checked = capture_selected_from_inputs(repository, selected, inputs, debug_output)?;
    attach_retained_annotations(repository, &mut checked)?;
    Ok(checked)
}

fn attach_retained_annotations(
    repository: &Repository,
    checked: &mut Check,
) -> Result<(), ClewError> {
    if checked.input_digest != repository.input_digest()? {
        return Err(invalid("documentation input changed during checking"));
    }
    let source = checked
        .source_inputs
        .take()
        .ok_or_else(|| invalid("captured inputs are missing before retained attachment"))?;
    let result = super::composition::attach_retained(repository, &source.inputs, checked);
    checked.source_inputs = Some(source);
    result
}

/// Attach catalogue observations only to a freshly assembled source Check.
/// Every declaration and note comes from the same captured input value. The
/// retained baseline, review scopes and process versions are a separate stage;
/// this boundary does not claim to freeze those additional inputs.
pub(super) fn attach_catalogue_from_inputs(
    inputs: &super::store::RepositoryInputs,
    checked: &mut Check,
) -> Result<(), ClewError> {
    super::entities::attach_from_records(&inputs.entities, checked)?;
    super::notes::attach_from_inputs(inputs, checked)?;
    super::dataflow::attach_from_inputs(&inputs.scenarios, &inputs.interactions, checked)
}

fn capture_selected_from_inputs(
    repository: &Repository,
    selected: &BTreeSet<String>,
    inputs: super::store::RepositoryInputs,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<Check, ClewError> {
    capture_selected_with(
        repository,
        selected,
        inputs,
        |service, expectation, targets| {
            if targets.targets.is_empty() {
                analysis::capture_with_expectation(repository, service, expectation, debug_output)
            } else {
                super::updates::capture_with_expectation(repository, service, targets, expectation)
            }
        },
    )
}

fn capture_selected_with(
    repository: &Repository,
    selected: &BTreeSet<String>,
    inputs: super::store::RepositoryInputs,
    mut capture: impl FnMut(
        &Service,
        Option<&super::evidence_package::Expectation>,
        &super::updates::State,
    ) -> Result<ServiceEvidence, ClewError>,
) -> Result<Check, ClewError> {
    let input_digest = digest(&inputs)?;
    let services = &inputs.services;
    if selected.iter().any(|id| !services.contains_key(id)) {
        return Err(invalid("selected documentation service does not exist"));
    }
    // Retained annotations are required by the completed check. Reject invalid
    // baselines and input scopes before any compiler or build can be invoked.
    super::composition::validate_retained(repository, &inputs)?;
    let mut evidence = BTreeMap::new();
    let mut unresolved = BTreeMap::new();
    let previous = if selected.is_empty() {
        None
    } else {
        let path = repository.path(".codeclew/cache/latest-check.json")?;
        if path.try_exists().map_err(io_error)? {
            Some(Check::load(repository, &path)?)
        } else {
            None
        }
    };
    let mut retained_services = BTreeSet::new();
    let targets = &inputs.update_state;
    for (id, service) in services {
        if !selected.is_empty() && !selected.contains(id) {
            if let Some(saved) = previous.as_ref().and_then(|previous| {
                let original = &previous.source_inputs.as_ref()?.inputs;
                (original.services.get(id) == Some(service)
                    && original.evidence_expectations.get(id)
                        == inputs.evidence_expectations.get(id)
                    && original.update_policies.get(id) == inputs.update_policies.get(id)
                    && original.update_state.targets.get(id) == targets.targets.get(id))
                .then(|| previous.services.get(id))
                .flatten()
            }) {
                evidence.insert(id.clone(), saved.clone());
                retained_services.insert(id.clone());
                continue;
            }
            unresolved.insert(id.clone(), json!({"status":"NOT_CHECKED","reason":"SERVICE_NOT_SELECTED","nextAction":"Select this service explicitly to check its current source."}));
            continue;
        }
        match super::progress::run("ACQUIRE_SERVICE_EVIDENCE", || {
            capture(service, inputs.evidence_expectations.get(id), targets)
        }) {
            Ok(value) => {
                evidence.insert(id.clone(), value);
            }
            Err(error) => {
                let mut failure =
                    json!({"status":"UNRESOLVED","reason":error.code,"nextAction":error.message});
                if let Some(target) = targets.targets.get(id) {
                    failure["targetRevision"] = json!(target.revision);
                }
                if let Some(diagnostic) = crate::worker_diagnostics::from_evidence(&error.evidence)
                {
                    failure["workerFailure"] = diagnostic;
                }
                if let Some(diagnostic) = crate::maven_diagnostics::from_evidence(&error.evidence)
                    .and_then(|value| crate::maven_diagnostics::safe_summary(&value))
                {
                    failure["mavenFailure"] = diagnostic;
                }
                if let Some(report) = error.evidence.iter().find_map(|s| {
                    s.strip_prefix("documentation-evidence-report:")
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                }) {
                    failure["evidencePackage"] = report;
                }
                unresolved.insert(id.clone(), failure);
            }
        }
    }
    if input_digest != repository.input_digest()? {
        return Err(invalid("documentation input changed during checking"));
    }
    let mut checked = assemble(
        input_digest.clone(),
        evidence,
        unresolved,
        &inputs.interactions,
        &inputs.scenarios,
    )?;
    // Borrow the captured bundle before moving it onto Check: do not clone all
    // protected note bodies merely to attach their derived observations.
    attach_catalogue_from_inputs(&inputs, &mut checked)?;
    checked.source_inputs = Some(SourceInputs {
        schema: SOURCE_INPUTS_SCHEMA.into(),
        input_digest,
        selected_services: if selected.is_empty() {
            inputs.services.keys().cloned().collect()
        } else {
            selected.clone()
        },
        retained_services,
        inputs,
    });
    checked.validate_source_input_binding()?;
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
        source_inputs: None,
        composition: None,
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
    fn walk(&mut self, service: &str, declaration_id: &str, depth: usize) -> Result<(), ClewError> {
        if depth > self.selection.max_depth {
            self.truncated = true;
            self.boundaries.insert("DEPTH_BUDGET_EXHAUSTED".into());
            return Ok(());
        }
        if !self.active.insert((service.into(), declaration_id.into())) {
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
            .get(declaration_id)
            .filter(|o| o.kind == "SYMBOL")
        else {
            self.boundaries.insert("CALLEE_BODY_UNAVAILABLE".into());
            return Ok(());
        };
        let symbol = declaration.symbol.as_str();
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
            let scope = declaration.normalized["scope"].as_str().unwrap_or("");
            let identity = analysis::scoped_identity(scope, &format!("{symbol}/event/{ordinal}"));
            let id = analysis::dependency_id(service, "flow", &identity)?;
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
                self.walk(&interaction.to.service, &receiver.id, depth + 1)?;
            }
            if !bridged
                && matches!(event["kind"].as_str(), Some("CALL" | "CONSTRUCT"))
                && let Some(target) = event["target"].as_str()
            {
                let targets: Vec<_> = e
                    .observations
                    .values()
                    .filter(|o| o.kind == "SYMBOL" && o.symbol == target)
                    .collect();
                let same_scope: Vec<_> = targets
                    .iter()
                    .copied()
                    .filter(|o| o.normalized["scope"].as_str().unwrap_or("") == scope)
                    .collect();
                // Catalog uniqueness does not establish classpath visibility:
                // a test or unrelated module can define the same JVM symbol.
                // Expand only the caller's source scope until a compiler-backed
                // cross-scope source mapping is available.
                match same_scope.as_slice() {
                    [callee] => self.walk(service, &callee.id, depth + 1)?,
                    [] => {
                        self.boundaries.insert(
                            if targets.is_empty() {
                                "EXTERNAL_CALL_BODY_NOT_EXPANDED"
                            } else {
                                "CROSS_SCOPE_CALLEE_UNVERIFIED"
                            }
                            .into(),
                        );
                    }
                    _ => {
                        self.boundaries
                            .insert("AMBIGUOUS_CALLEE_SCOPE_NOT_EXPANDED".into());
                    }
                }
            }
        }
        self.active.remove(&(service.into(), declaration_id.into()));
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
            walker.walk(&s.root.service, &root.candidates[0], 0)?;
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
    // Validate the Check-to-input binding. The codec validates the full input
    // payload once; capture constructs its digest from the immutable value.
    fn validate_source_input_binding(&self) -> Result<(), ClewError> {
        let Some(record) = &self.source_inputs else {
            if self.composition.is_some() {
                return Err(invalid(
                    "saved composition has no original source-input contract",
                ));
            }
            return Ok(());
        };
        if record.schema != SOURCE_INPUTS_SCHEMA
            || (self.composition.is_none() && record.input_digest != self.input_digest)
            || record
                .selected_services
                .iter()
                .chain(&record.retained_services)
                .any(|id| !record.inputs.services.contains_key(id))
            || !record
                .selected_services
                .is_disjoint(&record.retained_services)
            || record
                .retained_services
                .iter()
                .any(|id| !self.services.contains_key(id))
        {
            return Err(invalid("saved source-selection input contract is invalid"));
        }
        if let Some(composition) = &self.composition {
            super::composition::validate(composition, record, &self.input_digest)?;
        }
        for (id, evidence) in &self.services {
            let service = record.inputs.services.get(id).ok_or_else(|| {
                invalid("saved source evidence is absent from its input contract")
            })?;
            if !(record.selected_services.contains(id) || record.retained_services.contains(id))
                || evidence.service != *id
                || evidence.service_digest != digest(service)?
            {
                return Err(invalid(
                    "saved source evidence does not match its captured service declaration",
                ));
            }
        }
        Ok(())
    }

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
    pub fn source_authorities(&self) -> BTreeMap<String, &'static str> {
        self.services
            .keys()
            .map(|id| {
                let authority = if self
                    .source_inputs
                    .as_ref()
                    .is_some_and(|s| s.retained_services.contains(id))
                {
                    "RETAINED_SOURCE_NOT_REVERIFIED"
                } else {
                    "CAPTURED_SOURCE"
                };
                (id.clone(), authority)
            })
            .collect()
    }
    pub fn summary(&self) -> Value {
        json!({"schema":self.schema,"inputDigest":self.input_digest,"contextDigest":self.context_digest,"status":if self.unresolved.is_empty(){"CHECKED"}else{"UNRESOLVED"},"services":self.services.iter().map(|(id,e)|(id,json!({"revision":e.revision,"coverage":e.coverage,"entrypoints":e.entrypoints.len(),"boundaries":e.boundaries,"sourceAuthority":if self.source_inputs.as_ref().is_some_and(|s|s.retained_services.contains(id)){"RETAINED_SOURCE_NOT_REVERIFIED"}else{"CAPTURED_SOURCE"}}))).collect::<BTreeMap<_,_>>(),"unresolved":self.unresolved,"interactions":self.interactions,"scenarios":self.scenarios.iter().map(|(id,s)|(id,json!({"steps":s.steps.len(),"truncated":s.truncated,"boundaries":s.boundaries}))).collect::<BTreeMap<_,_>>()})
    }
    pub fn save(&self, repo: &Repository) -> Result<(), ClewError> {
        self.save_snapshot(repo).map(|_| ())
    }

    /// Save a content-addressed manifest and update the convenience latest pointer.
    /// The returned handle remains readable when another check replaces latest.
    /// This is a retained evidence identity, not a claim about current sources.
    pub fn save_snapshot(&self, repo: &Repository) -> Result<String, ClewError> {
        let _lock = repo.lock()?;
        let (handle, encoded) = self.store_snapshot(repo)?;
        repo.atomic(".codeclew/cache/latest-check.json", &encoded)?;
        Ok(handle)
    }

    pub(super) fn store_snapshot(&self, repo: &Repository) -> Result<(String, Vec<u8>), ClewError> {
        let manifest = self.store_manifest(repo)?;
        let encoded = bytes(&manifest)?;
        if encoded.len() > PORTABLE_CACHE_MAX_BYTES as usize {
            return Err(crate::error::ClewError::new(
                crate::error::ErrorCode::SliceBudgetExceeded,
                "documentation check exceeds its portable cache budget; narrow source roots",
            ));
        }
        let reference = super::cache::put(repo, CHECK_MANIFEST_SCHEMA, &encoded)?;
        Ok((format!("{}/{}", reference.digest, reference.size), encoded))
    }

    /// Retain an immutable check without acquiring source evidence or changing
    /// the convenience latest pointer. `selector` is an explicit immutable
    /// snapshot handle; when absent, the current normalized latest-check record
    /// is read once and retained in the content-addressed store.
    pub fn retained(
        repo: &Repository,
        selector: Option<&str>,
        selected: &BTreeSet<String>,
    ) -> Result<(Check, String), ClewError> {
        let (checked, original_handle, normalized_bytes) = if let Some(handle) = selector {
            let checked = Self::load_snapshot(repo, handle).map_err(|error| {
                ClewError::new(
                    crate::error::ErrorCode::StateCorrupt,
                    format!(
                        "documentation snapshot is unavailable or corrupt; run docs check explicitly: {}",
                        error.message
                    ),
                )
            })?;
            (checked, Some(handle.to_owned()), None)
        } else {
            let path = repo.path(".codeclew/cache/latest-check.json")?;
            let raw = std::fs::read(&path).map_err(|_| ClewError::new(
                crate::error::ErrorCode::StateCorrupt,
                "latest saved documentation evidence is unavailable; run docs check explicitly or select --snapshot",
            ))?;
            let checked = Self::decode(repo, &raw).map_err(|error| ClewError::new(error.code,
                format!("saved documentation evidence is corrupt; select another snapshot or run docs check explicitly: {}", error.message)))?;
            (checked, None, Some(raw))
        };
        if checked.input_digest != repo.input_digest()? {
            return Err(crate::error::ClewError::new(
                crate::error::ErrorCode::StaleRequiresReslice,
                "retained documentation check is stale; run docs check explicitly",
            ));
        }
        let services = repo.services()?;
        if selected.iter().any(|id| !services.contains_key(id)) {
            return Err(invalid("selected documentation service does not exist"));
        }
        if selected.iter().any(|id| !checked.services.contains_key(id)) {
            return Err(invalid(
                "selected service is missing from retained evidence; select a saved snapshot containing it or explicitly run docs check --service ID",
            ));
        }
        if let Some(handle) = original_handle {
            return Ok((checked, handle));
        }
        let _lock = repo.lock()?;
        if checked.input_digest != repo.input_digest()? {
            return Err(invalid(
                "documentation declarations changed while selecting saved evidence",
            ));
        }
        let encoded = normalized_bytes.ok_or_else(|| invalid("saved check manifest is missing"))?;
        // Freeze the exact validated manifest, without rewriting fact memberships.
        let reference = super::cache::put(repo, CHECK_MANIFEST_SCHEMA, &encoded)?;
        let handle = format!("{}/{}", reference.digest, reference.size);
        Ok((checked, handle))
    }

    /// Read an explicitly selected immutable snapshot. Missing or damaged data
    /// is an error; this path never acquires evidence or follows latest-check.
    pub fn load_snapshot(repo: &Repository, handle: &str) -> Result<Check, ClewError> {
        Self::from_manifest(repo, Self::load_snapshot_manifest(repo, handle)?)
    }

    pub(super) fn load_snapshot_manifest(
        repo: &Repository,
        handle: &str,
    ) -> Result<CheckManifest, ClewError> {
        let (digest, size) = handle.rsplit_once('/').ok_or_else(|| {
            invalid("snapshot must be the sha256:identity/size returned by docs check")
        })?;
        let size: u64 = size.parse().map_err(|_| invalid("invalid snapshot size"))?;
        if handle != format!("{digest}/{size}") {
            return Err(invalid("snapshot size must use canonical decimal notation"));
        }
        let reference =
            super::cache::ObjectRef::new(CHECK_MANIFEST_SCHEMA.into(), digest.into(), size);
        let manifest: CheckManifest =
            super::cache::get_json(repo, &reference, PORTABLE_CACHE_MAX_BYTES).map_err(|error| invalid(format!(
                "DOCS_REINDEX_REQUIRED: unsupported or corrupt snapshot ({}); initialize a fresh documentation root and run docs check", error.message)))?.ok_or_else(
                || {
                    ClewError::new(
                        crate::error::ErrorCode::StateCorrupt,
                        "documentation snapshot is unavailable; capture evidence explicitly",
                    )
                },
            )?;
        if manifest.schema != CHECK_MANIFEST_SCHEMA {
            return Err(invalid(
                "DOCS_REINDEX_REQUIRED: unsupported documentation snapshot schema; initialize a fresh documentation root and run docs check",
            ));
        }
        Ok(manifest)
    }

    /// Persist heavy service evidence and the dependencies map. Observations
    /// are stored as per-fact memberships in the fact index and the manifest
    /// references the immutable snapshot root, so no second whole-map copy of
    /// every observation is serialized.
    pub fn store_manifest(&self, repo: &Repository) -> Result<CheckManifest, ClewError> {
        self.validate_source_input_binding()?;
        let source_inputs = super::source_inputs::store(repo, self.source_inputs.as_ref()
            .ok_or_else(|| invalid("DOCS_REINDEX_REQUIRED: capture has no source-input contract; initialize a fresh documentation root and run docs check"))?)?;
        let composition = self
            .composition
            .as_ref()
            .map(|value| super::composition::store(repo, value))
            .transpose()?;
        let parent = self
            .composition
            .as_ref()
            .map(|value| Self::load_snapshot_manifest(repo, &value.parent))
            .transpose()?;
        let mut service_manifests = BTreeMap::new();
        for (id, evidence) in &self.services {
            let mut capture = super::cache::store_capture(repo, evidence)?;
            if let Some(parent) = &parent {
                let original = parent.service_manifests.get(id).ok_or_else(|| {
                    invalid("composition service has no original capture manifest")
                })?;
                // Recomposition does not upgrade source acquisition authority.
                // Rebuild all evidence references above, then preserve only this
                // metadata; validate_parent_manifest still compares the full
                // envelope, so changed evidence cannot inherit the parent.
                capture.cacheability = original.cacheability.clone();
                capture.reason = original.reason.clone();
            }
            service_manifests.insert(id.clone(), capture);
        }
        // Dependencies are written through the fact index as per-fact memberships
        // (bounded pages, copy-on-write), not as one whole-map object.
        let dependencies_index = super::fact_index::store_dependency_map(repo, &self.dependencies)?;
        let manifest = CheckManifest {
            schema: CHECK_MANIFEST_SCHEMA.into(),
            input_digest: self.input_digest.clone(),
            context_digest: self.context_digest.clone(),
            service_manifests,
            unresolved: self.unresolved.clone(),
            interactions: self.interactions.clone(),
            scenarios: self.scenarios.clone(),
            dependencies_index,
            source_inputs,
            composition,
        };
        if let Some(composition) = &self.composition {
            super::composition::validate_parent_manifest(
                repo,
                composition,
                &manifest,
                self.source_inputs
                    .as_ref()
                    .ok_or_else(|| invalid("composition source-input contract is missing"))?
                    .input_digest
                    .as_str(),
            )?;
        }
        Ok(manifest)
    }

    /// Hydrate a current reference manifest; historical serialized formats require reindexing.
    pub fn load(repo: &Repository, path: &std::path::Path) -> Result<Check, ClewError> {
        let raw = std::fs::read(path).map_err(io_error)?;
        Self::decode(repo, &raw)
    }

    fn decode(repo: &Repository, raw: &[u8]) -> Result<Check, ClewError> {
        if raw.len() as u64 > PORTABLE_CACHE_MAX_BYTES {
            return Err(crate::error::ClewError::new(
                crate::error::ErrorCode::ResourceLimit,
                "documentation check exceeds its portable cache budget",
            ));
        }
        let manifest: CheckManifest = serde_json::from_slice(raw).map_err(|error| invalid(format!(
            "DOCS_REINDEX_REQUIRED: unsupported saved check format ({error}); initialize a fresh documentation root and run docs check")))?;
        Self::from_manifest(repo, manifest)
    }

    fn from_manifest(repo: &Repository, manifest: CheckManifest) -> Result<Check, ClewError> {
        if manifest.schema != CHECK_MANIFEST_SCHEMA {
            return Err(invalid(
                "DOCS_REINDEX_REQUIRED: unsupported documentation snapshot schema; initialize a fresh documentation root and run docs check",
            ));
        }
        let source_inputs = Some(super::progress::run("LOAD_SOURCE_INPUT_CONTRACT", || {
            super::source_inputs::load(repo, &manifest.source_inputs)
        })?);
        let composition = manifest
            .composition
            .as_ref()
            .map(|reference| super::composition::load(repo, reference))
            .transpose()?;
        if let Some(composition) = &composition {
            super::composition::validate_parent_manifest(
                repo,
                composition,
                &manifest,
                source_inputs
                    .as_ref()
                    .ok_or_else(|| invalid("composition source-input contract is missing"))?
                    .input_digest
                    .as_str(),
            )?;
        }
        let mut services = BTreeMap::new();
        for (id, capture) in &manifest.service_manifests {
            services.insert(
                id.clone(),
                super::progress::run("LOAD_RETAINED_SERVICE", || {
                    super::cache::load_capture(repo, capture)
                })?,
            );
        }
        let dependencies = super::progress::run("LOAD_RETAINED_DEPENDENCIES", || {
            super::fact_index::load_snapshot_observations(
                repo,
                &manifest.dependencies_index,
                CHECK_DEPENDENCIES_SCOPE,
            )
        })?;
        let checked = Check {
            schema: "codeclew-documentation-check/1.0".into(),
            input_digest: manifest.input_digest,
            context_digest: manifest.context_digest,
            services,
            unresolved: manifest.unresolved,
            interactions: manifest.interactions,
            scenarios: manifest.scenarios,
            dependencies,
            source_inputs,
            composition,
        };
        checked.validate_source_input_binding()?;
        Ok(checked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java_adapter_v2::{JavaCompilerFact, build_java_compiler_index};
    use crate::java_project_model::extract_java_model;
    use std::fs;

    #[test]
    fn retained_baseline_preflight_rejects_invalid_state_before_capture() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Retained baseline preflight").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0", "id":"orders",
            "title":"Orders", "repositoryId":"orders",
            "repository":"https://example.invalid/orders", "language":"java",
            "profile":"java-17plus-maven-read-only", "compilations":[":/main"], "targetRef":"main"
        }))
        .unwrap();
        repo.service_add(service, Some(&repo.input_digest().unwrap()))
            .unwrap();
        let inputs = repo.inputs().unwrap();
        let attempt = |calls: &mut usize| {
            capture_selected_with(&repo, &BTreeSet::new(), inputs.clone(), |_, _, _| {
                *calls += 1;
                Err(invalid("test source is unavailable"))
            })
        };
        // A new documentation root has no published baseline and can capture.
        let mut calls = 0;
        let fresh = attempt(&mut calls).unwrap();
        assert_eq!(calls, 1);
        assert!(fresh.unresolved.contains_key("orders"));
        let index = repo.path("docs/index.html").unwrap();
        fs::write(&index, "manually owned content\n").unwrap();
        calls = 0;
        let error = attempt(&mut calls).unwrap_err();
        assert!(error.message.contains("manually owned"));
        assert_eq!(calls, 0);

        let bundle = "a".repeat(64);
        let index_text = format!("<!-- codeclew-bundle {bundle} -->\n");
        fs::write(&index, &index_text).unwrap();
        let error = attempt(&mut calls).unwrap_err();
        assert!(error.message.contains("DOCS_BASELINE_INCOMPLETE"));
        assert!(
            error
                .message
                .contains(&format!("docs/generated/{bundle}/bindings.json"))
        );
        assert!(error.message.contains("restore"));
        assert!(!error.message.contains(temporary.path().to_str().unwrap()));
        assert_eq!(calls, 0);
        assert_eq!(fs::read_to_string(&index).unwrap(), index_text);

        let path = repo
            .path(&format!("docs/generated/{bundle}/bindings.json"))
            .unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{malformed bindings").unwrap();
        let error = attempt(&mut calls).unwrap_err();
        assert!(error.message.contains("DOCS_REINDEX_REQUIRED"));
        assert_eq!(calls, 0);

        let mut binding = super::super::render::make_bindings(&fresh, BTreeMap::new()).unwrap();
        binding.output_hashes.insert(
            "root-overview.html".into(),
            crate::canonical::hash_bytes(index_text.as_bytes()),
        );
        super::super::bindings::compact(&mut binding);
        let current = bytes(&binding).unwrap();
        binding.schema = "codeclew-documentation-bindings/1.2".into();
        let obsolete = bytes(&binding).unwrap();
        fs::write(&path, &obsolete).unwrap();
        let error = attempt(&mut calls).unwrap_err();
        assert!(error.message.contains("DOCS_REINDEX_REQUIRED"));
        assert_eq!(calls, 0);
        assert_eq!(fs::read(&path).unwrap(), obsolete);

        fs::write(&path, &current).unwrap();
        let mut checked = attempt(&mut calls).unwrap();
        assert_eq!(calls, 1);
        attach_retained_annotations(&repo, &mut checked).unwrap();
        // Preflight does not bypass post-capture validation of concurrent edits.
        fs::write(&path, &obsolete).unwrap();
        assert!(
            attach_retained_annotations(&repo, &mut checked)
                .unwrap_err()
                .message
                .contains("DOCS_REINDEX_REQUIRED")
        );
        assert!(checked.source_inputs.is_some());
    }

    #[test]
    fn dangling_index_symlink_is_not_treated_as_a_new_root() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Dangling baseline").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        symlink(
            "missing-overview.html",
            repo.path("docs/index.html").unwrap(),
        )
        .unwrap();
        assert!(super::super::bindings::capture_baseline(&repo).is_err());
    }

    #[test]
    fn catalogue_attachment_uses_captured_notes_and_memberships_across_aba() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Catalogue capture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        for id in ["orders", "inventory"] {
            let service: Service = serde_json::from_value(json!({
                "schema":"codeclew-documentation-service/1.0", "id":id,
                "title":id, "repositoryId":id,
                "repository":format!("https://example.invalid/{id}"), "language":"java",
                "profile":"java-17plus-maven-read-only", "compilations":[":/main"], "targetRef":"main"
            }))
            .unwrap();
            repo.service_add(service, Some(&repo.input_digest().unwrap()))
                .unwrap();
        }
        let records = [
            (
                "catalog/entities/quantity.json",
                json!({
                    "schema":"codeclew-documentation-entity/1.0", "id":"quantity",
                    "title":"Captured quantity", "description":"Declared domain identity",
                    "relations":[{"service":"orders", "kind":"owned", "origin":"human",
                        "rationale":"Maintainer declaration", "confidence":"declared"}], "limitations":[]
                }),
            ),
            (
                "catalog/interactions/transfer.json",
                json!({
                    "schema":"codeclew-documentation-interaction/1.0", "id":"transfer", "title":"Captured transfer",
                    "from":{"service":"orders"}, "to":{"service":"inventory"},
                    "transport":{"kind":"http", "method":"POST", "path":"/quantity"},
                    "declaration":{"origin":"human", "rationale":"Declared transfer"}
                }),
            ),
            (
                "scenarios/quantity-view.yaml",
                json!({
                    "schema":"codeclew-documentation-view/1.0", "id":"quantity-view",
                    "title":"Captured view", "summary":"Declared quantity flow", "root":{"service":"orders"},
                    "interactions":["transfer"], "view":{"module":"entity-dataflow/1.0",
                        "inputObjects":["entity:quantity"], "services":["orders","inventory"], "scope":"Declared quantity flow"}
                }),
            ),
            (
                "catalog/notes/policy.json",
                json!({
                    "schema":"codeclew-documentation-note-association/1.0", "id":"policy", "title":"Captured policy",
                    "service":"orders", "path":"notes/policy.md", "classification":"policy", "period":"Current",
                    "targets":["service:orders/section-entities", "service:inventory", "entity:quantity",
                        "view:quantity-view", "scenario:quantity-view", "service:orders/unknown-section",
                        "entity:missing", "view:missing", "scenario:missing", "service:missing", "unknown:target"]
                }),
            ),
        ];
        for (path, value) in &records {
            repo.atomic(path, &bytes(value).unwrap()).unwrap();
        }
        let original_text = "Protected original A.\r\n";
        repo.atomic("notes/policy.md", original_text.as_bytes())
            .unwrap();
        let inputs = repo.inputs().unwrap();
        let input_digest = digest(&inputs).unwrap();
        let source = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "orders".into(),
            revision: "a".repeat(40),
            service_digest: digest(&inputs.services["orders"]).unwrap(),
            extractor: SOURCE_EXTRACTOR.into(),
            runtime_mode: "SOURCE_SYNTAX".into(),
            coverage: "SYNTAX".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        let fresh = || {
            assemble(
                input_digest.clone(),
                BTreeMap::from([("orders".into(), source.clone())]),
                BTreeMap::new(),
                &inputs.interactions,
                &inputs.scenarios,
            )
            .unwrap()
        };
        let mut control = fresh();
        attach_catalogue_from_inputs(&inputs, &mut control).unwrap();

        // B removes A's target definitions, adds unrelated members and replaces
        // the human text. An end digest alone cannot detect these transient reads.
        for (path, value) in &records {
            fs::remove_file(repo.root.join(path)).unwrap();
            let mut changed = value.clone();
            changed["id"] = json!("concurrent");
            changed["title"] = json!("Concurrent B");
            let parent = std::path::Path::new(path)
                .parent()
                .unwrap()
                .to_str()
                .unwrap();
            let extension = std::path::Path::new(path)
                .extension()
                .unwrap()
                .to_str()
                .unwrap();
            repo.atomic(
                &format!("{parent}/concurrent.{extension}"),
                &bytes(&changed).unwrap(),
            )
            .unwrap();
        }
        repo.atomic("notes/policy.md", b"Concurrent B text")
            .unwrap();
        let mut actual = fresh();
        attach_catalogue_from_inputs(&inputs, &mut actual).unwrap();
        assert_eq!(actual.dependencies, control.dependencies);
        assert_eq!(
            serde_json::to_value(&actual.scenarios).unwrap(),
            serde_json::to_value(&control.scenarios).unwrap()
        );
        assert_eq!(actual.context_digest, control.context_digest);
        assert_eq!(actual.services["orders"], source);
        let note = &actual.dependencies["note:policy"].normalized;
        assert_eq!(note["original"]["text"], original_text);
        assert_eq!(
            note["missingTargets"],
            json!([
                "service:orders/unknown-section",
                "entity:missing",
                "view:missing",
                "scenario:missing",
                "service:missing",
                "unknown:target"
            ])
        );
        assert_eq!(
            actual.dependencies["entity-scope:orders"].normalized["dependencyIds"],
            json!(["entity:quantity"])
        );
        assert_eq!(
            actual.dependencies["note-scope:inventory"].normalized["dependencyIds"],
            json!(["note:policy"])
        );
        assert_eq!(
            actual.dependencies["view-scope:quantity-view"].normalized["interactionMembership"],
            json!(["transfer"])
        );
        assert_eq!(
            actual.dependencies["view-scope:quantity-view"].normalized["unavailableServices"],
            json!(["inventory"])
        );
        assert!(
            !actual
                .dependencies
                .keys()
                .any(|id| id.contains("concurrent"))
        );

        for (path, value) in &records {
            let parent = std::path::Path::new(path).parent().unwrap();
            let extension = std::path::Path::new(path)
                .extension()
                .unwrap()
                .to_str()
                .unwrap();
            fs::remove_file(
                repo.root
                    .join(parent)
                    .join(format!("concurrent.{extension}")),
            )
            .unwrap();
            repo.atomic(path, &bytes(value).unwrap()).unwrap();
        }
        repo.atomic("notes/policy.md", original_text.as_bytes())
            .unwrap();
        assert_eq!(repo.input_digest().unwrap(), input_digest);
    }

    #[test]
    fn source_selection_consumes_captured_inputs_across_aba_and_rejects_persistent_change() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Captured input test").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0", "id":"orders",
            "title":"Original title", "repositoryId":"orders",
            "repository":"https://example.invalid/orders", "language":"java",
            "profile":"java-17plus-maven-read-only", "compilations":[":/main"], "targetRef":"main"
        }))
        .unwrap();
        repo.service_add(service.clone(), Some(&repo.input_digest().unwrap()))
            .unwrap();
        let expectation = super::super::evidence_package::Expectation {
            schema: "codeclew-documentation-evidence-expectation/1.0".into(),
            service: "orders".into(),
            repository_id: "orders".into(),
            service_digest: digest(&service).unwrap(),
            revision: "a".repeat(40),
            manifest_digest: format!("sha256:{}", "b".repeat(64)),
            sequence: 1,
        };
        let target = super::super::updates::State {
            schema: "codeclew-documentation-update-state/1.0".into(),
            targets: BTreeMap::from([(
                "orders".into(),
                super::super::updates::Event {
                    schema: "codeclew-documentation-update-event/1.0".into(),
                    id: "original".into(),
                    service: "orders".into(),
                    repository_id: "orders".into(),
                    source_ref: "main".into(),
                    revision: "a".repeat(40),
                    sequence: 1,
                    tag: None,
                },
            )]),
        };
        fs::create_dir_all(repo.root.join("catalog/evidence-trust")).unwrap();
        let policy_path = repo.root.join("catalog/evidence-trust/orders.json");
        let target_path = repo.root.join("catalog/update-state.json");
        let service_path = repo.root.join("catalog/services/orders.json");
        fs::write(&policy_path, bytes(&expectation).unwrap()).unwrap();
        fs::write(&target_path, bytes(&target).unwrap()).unwrap();
        let captured = repo.inputs().unwrap();
        let original_digest = digest(&captured).unwrap();
        let mut changed_service = service.clone();
        changed_service.title = "Concurrent title".into();
        let mut changed_expectation = expectation.clone();
        changed_expectation.service_digest = digest(&changed_service).unwrap();
        changed_expectation.revision = "c".repeat(40);
        let mut changed_target = target.clone();
        changed_target.targets.get_mut("orders").unwrap().revision = "c".repeat(40);
        let replace = |changed: bool| {
            fs::write(
                &service_path,
                bytes(if changed { &changed_service } else { &service }).unwrap(),
            )
            .unwrap();
            fs::write(
                &policy_path,
                bytes(if changed {
                    &changed_expectation
                } else {
                    &expectation
                })
                .unwrap(),
            )
            .unwrap();
            fs::write(
                &target_path,
                bytes(if changed { &changed_target } else { &target }).unwrap(),
            )
            .unwrap();
        };
        replace(true);
        assert_ne!(repo.input_digest().unwrap(), original_digest);
        let mut calls = 0;
        let make_evidence = || ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "orders".into(),
            revision: "a".repeat(40),
            service_digest: digest(&service).unwrap(),
            extractor: SOURCE_EXTRACTOR.into(),
            runtime_mode: "SOURCE_SYNTAX".into(),
            coverage: "SYNTAX".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        let checked = capture_selected_with(
            &repo,
            &BTreeSet::new(),
            captured.clone(),
            |chosen, policy, state| {
                calls += 1;
                assert_eq!(chosen, &service);
                assert_eq!(policy, Some(&expectation));
                assert_eq!(state, &target);
                assert_eq!(repo.services().unwrap()["orders"], changed_service);
                replace(false);
                Ok(make_evidence())
            },
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(checked.input_digest, original_digest);
        assert_eq!(checked.source_inputs.as_ref().unwrap().inputs, captured);
        assert_eq!(checked.services["orders"].revision, "a".repeat(40));
        let error = capture_selected_with(&repo, &BTreeSet::new(), captured, |_, _, _| {
            replace(true);
            Ok(make_evidence())
        })
        .unwrap_err();
        assert!(error.message.contains("input changed during checking"));
    }

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
            let index =
                build_java_compiler_index(&root, &model, &hashes, false, None, &[], None).unwrap();
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
                modules: None,
                compilations: vec![":/main".into()],
                target_ref: "main".into(),
                source_link_template: None,
                contract_files: vec![contract.into()],
                annotation_processor_paths: vec![],
            };
            let evidence = analysis::project(
                &service,
                &"1".repeat(40),
                &digest(&service).unwrap(),
                "DEVELOPMENT",
                "PARTIAL",
                facts,
                &files,
                false,
            )
            .unwrap();
            services.insert(id.into(), evidence);
        }
        let selector = |service: &str, owner: &str, name: &str| Endpoint {
            service: service.into(),
            selector: Some(Selector {
                scope: None,
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
            process: Some(super::super::processes::Details {
                scope: "Order reservation".into(),
                participants: vec!["orders".into(), "inventory".into()],
                objects: vec![],
                trigger: "Checkout request".into(),
                outcomes: vec!["Reservation result".into()],
                linked_subviews: vec![],
            }),
            view: None,
            schema: "codeclew-documentation-process/1.0".into(),
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

    #[test]
    fn repository_run_lock_serializes_concurrent_holders() {
        let repo_root = tempfile::tempdir().unwrap();
        Repository::init(repo_root.path(), "Lock test").unwrap();
        let repo = Repository::open(repo_root.path()).unwrap();
        // First holder keeps the per-repository flock for the run's duration.
        let first = acquire_repository_run_lock(&repo).unwrap();
        let acquired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquired_thread = acquired.clone();
        let path_for_thread = repo_root.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            // Blocks until `first` is released; never observes the shared repo
            // while a concurrent run's attempt lifecycle is still active.
            let _second =
                acquire_repository_run_lock(&Repository::open(&path_for_thread).unwrap()).unwrap();
            acquired_thread.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(
            !acquired.load(std::sync::atomic::Ordering::SeqCst),
            "a second holder must block while the first run holds the lock"
        );
        drop(first);
        handle.join().unwrap();
        assert!(acquired.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn process_walk_uses_the_declarations_scoped_flow_identity() {
        let symbol = "method:example.Worker#process()V";
        let scope = ":worker/main";
        let event = json!({"kind":"RETURN","value":"done"});
        let declaration = json!({"ownerIdentity":"class:example.Worker","name":"process",
            "jvmDescriptor":"()V","scope":scope,"documentation":{"events":[event],"boundaries":[]}});
        let flow = json!({"kind":"RETURN","value":"done","ordinal":0,"scope":scope});
        let symbol_id = analysis::dependency_id(
            "worker",
            "symbol",
            &analysis::scoped_identity(scope, symbol),
        )
        .unwrap();
        let flow_id = analysis::dependency_id(
            "worker",
            "flow",
            &analysis::scoped_identity(scope, &format!("{symbol}/event/0")),
        )
        .unwrap();
        let observations = [
            (symbol_id.clone(), "SYMBOL", declaration),
            (flow_id.clone(), "FLOW", flow),
        ]
        .into_iter()
        .map(|(id, kind, normalized)| {
            (
                id.clone(),
                Observation {
                    id,
                    kind: kind.into(),
                    service: "worker".into(),
                    symbol: symbol.into(),
                    digest: digest(&normalized).unwrap(),
                    normalized,
                    source_ids: vec!["worker-source".into()],
                },
            )
        })
        .collect();
        let evidence = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "worker".into(),
            revision: "a".repeat(40),
            service_digest: "test".into(),
            extractor: EXTRACTOR.into(),
            runtime_mode: "DEVELOPMENT".into(),
            coverage: "PARTIAL".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations,
            sources: BTreeMap::from([(
                "worker-source".into(),
                Source {
                    id: "worker-source".into(),
                    service: "worker".into(),
                    revision: "a".repeat(40),
                    file: "Worker.java".into(),
                    start_line: 1,
                    end_line: 1,
                    text_digest: crate::canonical::hash_bytes(b"void process() { return; }"),
                    text: "void process() { return; }".into(),
                    evidence_digest: "fixture".into(),
                    authority: "EXACT_SNAPSHOT_TEXT".into(),
                    occurrence: None,
                    url: None,
                },
            )]),
            contracts: BTreeMap::new(),
        };
        let scenario: Scenario = serde_json::from_value(json!({
            "schema":"codeclew-documentation-process/1.0", "process":{"scope":"Worker processing","participants":["worker"],"trigger":"Work request","outcomes":["Processed work"]}, "id":"worker-flow", "title":"Worker processing", "summary":"Scoped internal method",
            "root":{"service":"worker","selector":{"language":"java","owner":"example.Worker","name":"process","parameterTypes":[]}},
            "interactions":[]
        })).unwrap();
        let services = BTreeMap::from([("worker".into(), evidence)]);
        let result = compose(&scenario, &services, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert_eq!(result.steps.len(), 1, "{:?}", result.boundaries);
        assert_eq!(result.steps[0].kind, "RETURN");
        assert!(result.dependency_ids.contains(&flow_id));
        assert!(result.dependency_ids.contains(&symbol_id));
        assert!(
            !result
                .boundaries
                .iter()
                .any(|b| b == "FLOW_EVENT_EVIDENCE_MISSING")
        );

        let mut services = services;
        let evidence = services.get_mut("worker").unwrap();
        let mut other = evidence.observations[&symbol_id].clone();
        other.id = "other-scope-declaration".into();
        other.normalized["scope"] = json!(":worker/test");
        other.normalized["documentation"]["events"] = json!([]);
        evidence.observations.insert(other.id.clone(), other);
        let ambiguous = compose(&scenario, &services, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert!(ambiguous.steps.is_empty());
        assert!(
            ambiguous
                .boundaries
                .contains(&"SCENARIO_ROOT_AMBIGUOUS".into())
        );
        let mut scenario = scenario;
        scenario.root.selector.as_mut().unwrap().scope = Some(scope.into());
        let selected = compose(&scenario, &services, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert_eq!(selected.steps.len(), 1);
        assert!(selected.dependency_ids.contains(&symbol_id));
        assert!(
            !selected
                .dependency_ids
                .contains(&"other-scope-declaration".into())
        );

        // Equal callee symbols in main/test must never pick map iteration order.
        let evidence = services.get_mut("worker").unwrap();
        let target = "method:example.Helper#apply()V";
        let call = json!({"kind":"CALL","target":target});
        evidence
            .observations
            .get_mut(&symbol_id)
            .unwrap()
            .normalized["documentation"]["events"] = json!([call]);
        evidence.observations.get_mut(&flow_id).unwrap().normalized = call;
        for (id, callee_scope) in [("a-test-callee", ":worker/test"), ("z-main-callee", scope)] {
            let mut callee = evidence.observations[&symbol_id].clone();
            callee.id = id.into();
            callee.symbol = target.into();
            callee.normalized["ownerIdentity"] = json!("class:example.Helper");
            callee.normalized["name"] = json!("apply");
            callee.normalized["scope"] = json!(callee_scope);
            callee.normalized["documentation"]["events"] = json!([]);
            evidence.observations.insert(id.into(), callee);
        }
        let selected = compose(&scenario, &services, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert!(selected.dependency_ids.contains(&"z-main-callee".into()));
        assert!(!selected.dependency_ids.contains(&"a-test-callee".into()));
        let evidence = services.get_mut("worker").unwrap();
        evidence
            .observations
            .get_mut("z-main-callee")
            .unwrap()
            .normalized["scope"] = json!(":other/main");
        let ambiguous = compose(&scenario, &services, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert!(
            ambiguous
                .boundaries
                .contains(&"CROSS_SCOPE_CALLEE_UNVERIFIED".into())
        );
        assert!(!ambiguous.dependency_ids.contains(&"a-test-callee".into()));
        assert!(!ambiguous.dependency_ids.contains(&"z-main-callee".into()));

        // A sole test-scope match is still not evidence for a production call.
        services
            .get_mut("worker")
            .unwrap()
            .observations
            .remove("z-main-callee");
        let unrelated = compose(&scenario, &services, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        assert!(
            unrelated
                .boundaries
                .contains(&"CROSS_SCOPE_CALLEE_UNVERIFIED".into())
        );
        assert!(!unrelated.dependency_ids.contains(&"a-test-callee".into()));
    }
}
