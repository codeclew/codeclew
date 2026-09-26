//! Declarative process state schema (`codeclew-documentation-process-states/1.0`):
//! load `<id>-states.yaml`, validate each transition's evidence against the
//! retained evidence, and render a PlantUML state diagram.
//!
//! The state machine is not derived from code — it is declared in
//! `scenarios/<id>-states.yaml` (per the approved design) and each transition
//! is cross-checked against retained evidence so that unresolved transitions
//! are reported as gaps rather than silently dropped.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::check::Check;
use super::store::{self, Repository};
use super::{invalid, io_error, plantuml};

/// Schema identifier accepted by `ProcessStates::load`.
pub const SCHEMA: &str = "codeclew-documentation-process-states/1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessStates {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub initial: String,
    #[serde(rename = "final")]
    pub final_states: Vec<String>,
    pub states: BTreeMap<String, String>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transition {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// Load `scenarios/<id>-states.yaml` from the repo, returning `None` when the
/// file is absent.
pub fn load(repo: &Repository, id: &str) -> Result<Option<ProcessStates>, crate::error::ClewError> {
    if !store::valid_id(id) {
        return Err(invalid("invalid process-state id"));
    }
    let path = repo.path(&format!("scenarios/{id}-states.yaml"))?;
    if !path.try_exists().map_err(io_error)? {
        return Ok(None);
    }
    let s: ProcessStates = store::read(&path, 256 * 1024)?;
    validate(&s, id)?;
    Ok(Some(s))
}

pub(super) fn validate(
    s: &ProcessStates,
    requested_id: &str,
) -> Result<(), crate::error::ClewError> {
    if s.schema != SCHEMA || s.id != requested_id || !store::valid_id(&s.id) {
        return Err(invalid("invalid process-state schema identity"));
    }
    if s.title.trim().is_empty() || s.title.len() > 1024 || s.states.is_empty() {
        return Err(invalid("invalid process-state title or empty state map"));
    }
    for (state, label) in &s.states {
        if !valid_state_id(state) || label.len() > 4096 {
            return Err(invalid("invalid process-state identifier or label"));
        }
    }
    if !state_ref_exists(s, &s.initial) {
        return Err(invalid("process-state initial state is not declared"));
    }
    for state in &s.final_states {
        if !state_ref_exists(s, state) {
            return Err(invalid("process-state final state is not declared"));
        }
    }
    for transition in &s.transitions {
        if !state_ref_exists(s, &transition.from)
            || !state_ref_exists(s, &transition.to)
            || transition.label.len() > 4096
            || transition.operation.len() > 4096
            || transition.evidence.len() > 1024
        {
            return Err(invalid(
                "invalid process-state transition reference or text",
            ));
        }
    }
    Ok(())
}

fn state_ref_exists(s: &ProcessStates, state: &str) -> bool {
    valid_state_id(state) && s.states.contains_key(state)
}

fn valid_state_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    id.len() <= 100
        && (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Captured process-state declarations for a checked input snapshot. A
/// composition is the latest declaration snapshot; otherwise use the original
/// source-input snapshot. Rendering must not consult the live repository.
pub fn captured<'a>(checked: &'a Check, id: &str) -> Option<&'a ProcessStates> {
    checked
        .composition
        .as_ref()
        .map(|composition| &composition.inputs)
        .or_else(|| checked.source_inputs.as_ref().map(|source| &source.inputs))?
        .process_states
        .get(id)
}

/// Capture sidecars only when their base scenario is present in the parsed
/// repository inputs. The generic scenario reader filters by schema, while
/// this pass applies the complete process-state validation rules.
pub(super) fn records(
    repo: &Repository,
    scenarios: &BTreeMap<String, super::model::Scenario>,
) -> Result<BTreeMap<String, ProcessStates>, crate::error::ClewError> {
    let directory = repo.path("scenarios")?;
    if !directory.try_exists().map_err(io_error)? {
        return Ok(BTreeMap::new());
    }
    let mut records = BTreeMap::new();
    for entry in std::fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("yaml") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let Some(id) = stem.strip_suffix("-states") else {
            continue;
        };
        if !scenarios.contains_key(id) {
            continue;
        }
        let safe_path = repo.path(&format!("scenarios/{stem}.yaml"))?;
        let value: serde_json::Value = store::read(&safe_path, 256 * 1024)?;
        if value.get("schema").and_then(serde_json::Value::as_str) != Some(SCHEMA) {
            continue;
        }
        if let Some(schema) = load(repo, id)? {
            if records.len() >= store::MAX_RECORDS {
                return Err(invalid("process-state input map exceeds its record bound"));
            }
            records.insert(id.to_string(), schema);
        }
    }
    Ok(records)
}

/// True when every explicit evidence reference resolves uniquely. When no
/// evidence is authored, a full symbol or unique exact method name can resolve
/// the operation.
fn resolved(checked: &Check, t: &Transition) -> bool {
    if !t.evidence.is_empty() {
        return t
            .evidence
            .iter()
            .all(|reference| evidence_reference_is_unique(checked, reference));
    }

    !t.operation.is_empty() && resolve_operation_symbol(checked, &t.operation).is_some()
}

fn evidence_reference_is_unique(checked: &Check, reference: &str) -> bool {
    // The same observation can be mirrored in service evidence and the
    // dependency map. Count distinct retained identities, not storage copies.
    let mut matches = BTreeMap::<String, BTreeSet<String>>::new();
    for service in checked.services.values() {
        for (id, observation) in &service.observations {
            if id == reference || observation.symbol == reference {
                matches
                    .entry(id.clone())
                    .or_default()
                    .insert(observation.symbol.clone());
            }
        }
    }
    for (id, observation) in &checked.dependencies {
        if id == reference || observation.symbol == reference {
            matches
                .entry(id.clone())
                .or_default()
                .insert(observation.symbol.clone());
        }
    }
    matches.len() == 1 && matches.values().all(|symbols| symbols.len() == 1)
}

/// Resolve a full symbol identity, or an exact simple method name when that
/// name identifies only one retained symbol. Mirrors between service evidence
/// and dependencies collapse by full symbol identity; similarly named methods
/// in separate services/scopes remain ambiguous.
pub(super) fn resolve_operation_symbol<'a>(checked: &'a Check, operation: &str) -> Option<&'a str> {
    if operation.is_empty() {
        return None;
    }
    let mut symbols = BTreeMap::<&'a str, BTreeSet<&'a str>>::new();
    for service in checked.services.values() {
        for entry in &service.entrypoints {
            symbols
                .entry(entry.symbol.as_str())
                .or_default()
                .insert(service.service.as_str());
        }
        for observation in service
            .observations
            .values()
            .filter(|observation| observation.kind == "SYMBOL")
        {
            symbols
                .entry(observation.symbol.as_str())
                .or_default()
                .insert(observation.service.as_str());
        }
    }
    for observation in checked
        .dependencies
        .values()
        .filter(|observation| observation.kind == "SYMBOL")
    {
        symbols
            .entry(observation.symbol.as_str())
            .or_default()
            .insert(observation.service.as_str());
    }

    if let Some((symbol, scopes)) = symbols.get_key_value(operation) {
        return (scopes.len() == 1).then_some(*symbol);
    }
    let mut matching_symbol = None;
    for (symbol, scopes) in &symbols {
        if simple_symbol_name(symbol) != Some(operation) {
            continue;
        }
        for scope in scopes {
            if matching_symbol.replace((*symbol, *scope)).is_some() {
                return None;
            }
        }
    }
    matching_symbol.map(|(symbol, _)| symbol)
}

fn simple_symbol_name(symbol: &str) -> Option<&str> {
    let tail = if let Some((_, method)) = symbol.rsplit_once('#') {
        method
    } else {
        symbol.rsplit(['.', '/', '$']).next()?
    };
    let name = tail.split('(').next()?;
    (!name.is_empty()).then_some(name)
}

/// Render a state diagram as PlantUML. Unresolved transitions (those whose
/// evidence does not resolve against retained evidence) are returned separately
/// so the caller can surface them as gaps.
pub fn render_puml(s: &ProcessStates, checked: &Check) -> (String, Vec<String>) {
    let mut out = String::new();
    out.push_str(&format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {} — states\nhide empty description\n",
        plantuml::escape(&s.title)
    ));
    for (id, label) in &s.states {
        out.push_str(&format!("state \"{}\" as {id}\n", plantuml::escape(label)));
    }
    out.push_str(&format!("[*] --> {} : initial\n", s.initial));
    let mut unresolved = Vec::new();
    for t in &s.transitions {
        if !resolved(checked, t) {
            unresolved.push(format!(
                "{} -> {} [{}]",
                t.from,
                t.to,
                plantuml::escape(&t.operation)
            ));
            continue;
        }
        let label = if t.operation.is_empty() {
            t.label.clone()
        } else if t.label.is_empty() {
            format!("[{}]", t.operation)
        } else {
            format!("{} [{}]", t.label, t.operation)
        };
        out.push_str(&format!(
            "{} --> {} : {}\n",
            t.from,
            t.to,
            plantuml::escape(&label)
        ));
    }
    for f in &s.final_states {
        out.push_str(&format!("{} --> [*] : final\n", f));
    }
    out.push_str("@enduml\n");
    (out, unresolved)
}

/// Render a compact "activity on transitions" diagram: each lifecycle operation
/// named by a state-schema transition becomes a branch that leads to the states
/// it transitions into. Mirrors the approved draft's transition activity view: one
/// branch per operation, each action names a target state. Returns `None` when
/// no transition names an operation.
pub fn activity_transitions_puml(s: &ProcessStates, checked: &Check) -> Option<String> {
    // Group transitions by operation, preserving first-seen order.
    let mut groups: Vec<(&str, Vec<&Transition>)> = Vec::new();
    for t in &s.transitions {
        if t.operation.is_empty() || !resolved(checked, t) {
            continue;
        }
        if let Some((_, ts)) = groups.iter_mut().find(|(op, _)| *op == t.operation) {
            ts.push(t);
        } else {
            groups.push((&t.operation, vec![t]));
        }
    }
    if groups.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {} — transition activities\nstart\n",
        plantuml::escape(&s.title)
    ));
    for (i, (op, ts)) in groups.iter().enumerate() {
        let branch = if i == 0 { "if" } else { "elseif" };
        out.push_str(&format!(
            "{branch} (operation) then ({}: {})\n",
            plantuml::escape(op),
            plantuml::escape(op)
        ));
        let mut seen: Vec<&str> = Vec::new();
        for t in ts {
            if seen.contains(&t.to.as_str()) {
                continue;
            }
            seen.push(&t.to);
            let self_loop = t.from == t.to;
            let suffix = if self_loop { " (self-transition)" } else { "" };
            out.push_str(&format!(
                "  :→ {} (from {}){};\n",
                plantuml::escape(&t.to),
                plantuml::escape(&t.from),
                suffix
            ));
        }
    }
    out.push_str("endif\nstop\n@enduml\n");
    Some(out)
}

/// Load `<id>-states.yaml` (if present) and render it, returning the PlantUML
/// document plus any unresolved transitions. Returns `None` when no schema file
/// exists for the id.
pub fn load_and_render(
    repo: &Repository,
    checked: &Check,
    id: &str,
) -> Result<Option<(String, Vec<String>)>, crate::error::ClewError> {
    let Some(s) = load(repo, id)? else {
        return Ok(None);
    };
    Ok(Some(render_puml(&s, checked)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn states(id: &str) -> ProcessStates {
        serde_yaml_ng::from_str(&format!(
            r#"
schema: codeclew-documentation-process-states/1.0
id: {id}
title: Task lifecycle
initial: WAIT
final: [FINISHED, ERROR]
states:
  WAIT: "waiting"
  PROCESSING: "processing"
  WAIT_DECISION: "waiting for decision"
  FINISHED: "finished"
  ERROR: "error"
transitions:
  - from: PROCESSING
    to: WAIT_DECISION
    label: "status → WAIT_DECISION"
    operation: changeTaskStatus
    evidence: [task-manager:symbol:65a49248775beae815197acc]
  - from: ERROR
    to: WAIT
    label: "restart"
    operation: restartTask
    evidence: [task-manager:symbol:missing]
"#
        ))
        .unwrap()
    }

    fn empty_check() -> Check {
        Check {
            schema: "test".into(),
            input_digest: "d".into(),
            context_digest: "d".into(),
            services: BTreeMap::new(),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        }
    }

    fn add_symbol(checked: &mut Check, service: &str, id: &str, symbol: &str) {
        let evidence = checked
            .services
            .entry(service.to_string())
            .or_insert_with(|| {
                serde_json::from_value(json!({
                    "schema":"codeclew-documentation-service-evidence/1.0",
                    "service":service,"revision":"rev","serviceDigest":"d",
                    "extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
                    "boundaries":[],"contracts":{},"entrypoints":[],
                    "observations":{},"sources":{}
                }))
                .unwrap()
            });
        evidence.observations.insert(
            id.to_string(),
            crate::documentation::model::Observation {
                id: id.to_string(),
                kind: "SYMBOL".into(),
                service: service.to_string(),
                symbol: symbol.to_string(),
                normalized: json!({"documentation":{"events":[]}}),
                digest: "d".into(),
                source_ids: vec![],
            },
        );
    }

    fn docs_repo() -> (tempfile::TempDir, Repository) {
        let root = tempfile::tempdir().unwrap();
        Repository::init(root.path(), "Process state tests").unwrap();
        let repo = Repository::open(root.path()).unwrap();
        (root, repo)
    }

    fn write_schema(repo: &Repository, schema: &ProcessStates) {
        std::fs::create_dir_all(repo.root.join("scenarios")).unwrap();
        let path = repo
            .path(&format!("scenarios/{}-states.yaml", schema.id))
            .unwrap();
        std::fs::write(path, serde_yaml_ng::to_string(schema).unwrap()).unwrap();
    }

    #[test]
    fn unresolved_transition_is_reported_as_gap() {
        // Empty evidence: the first transition's evidence id is absent, so it
        // is reported; no operation matches either.
        let (_, unresolved) = render_puml(&states("s"), &empty_check());
        assert_eq!(unresolved.len(), 2, "{unresolved:?}");
        assert!(unresolved[0].contains("PROCESSING -> WAIT_DECISION"));
    }

    #[test]
    fn resolved_transition_is_rendered_with_operation() {
        // Provide evidence matching the first transition's id.
        let mut checked = empty_check();
        add_symbol(
            &mut checked,
            "svc",
            "task-manager:symbol:65a49248775beae815197acc",
            "method:class:svc.TaskService#changeTaskStatus()V",
        );
        let (puml, unresolved) = render_puml(&states("s"), &checked);
        assert!(
            puml.contains(
                "PROCESSING --> WAIT_DECISION : status → WAIT_DECISION ［changeTaskStatus］"
            ),
            "{puml}"
        );
        assert!(puml.contains("state \"waiting\" as WAIT"));
        assert!(puml.contains("[*] --> WAIT : initial"));
        assert!(puml.contains("FINISHED --> [*] : final"));
        // The second transition (missing evidence) is still reported.
        assert_eq!(unresolved.len(), 1, "{unresolved:?}");
    }

    #[test]
    fn schema_identity_is_validated() {
        let s = states("s");
        assert_eq!(s.schema, SCHEMA);
        assert_eq!(s.initial, "WAIT");
        assert_eq!(
            s.final_states,
            vec!["FINISHED".to_string(), "ERROR".to_string()]
        );
        assert_eq!(s.transitions.len(), 2);
    }

    #[test]
    fn activity_transitions_groups_by_operation() {
        let mut checked = empty_check();
        add_symbol(
            &mut checked,
            "svc",
            "task-manager:symbol:65a49248775beae815197acc",
            "method:class:svc.TaskService#changeTaskStatus()V",
        );
        let puml = activity_transitions_puml(&states("s"), &checked).unwrap();
        assert!(puml.contains("title Task lifecycle — transition activities"));
        assert!(puml.contains("if (operation) then (changeTaskStatus: changeTaskStatus)"));
        assert!(
            puml.contains(":→ WAIT_DECISION (from PROCESSING);"),
            "{puml}"
        );
        assert!(
            !puml.contains("restartTask"),
            "unresolved transition leaked: {puml}"
        );
        assert!(puml.ends_with("endif\nstop\n@enduml\n"), "{puml}");
    }

    #[test]
    fn activity_transitions_none_without_operations() {
        let mut s = states("s");
        s.transitions.clear();
        assert!(activity_transitions_puml(&s, &empty_check()).is_none());
    }

    #[test]
    fn explicit_evidence_requires_every_reference_to_resolve_uniquely() {
        let mut checked = empty_check();
        add_symbol(
            &mut checked,
            "svc",
            "task-manager:symbol:65a49248775beae815197acc",
            "method:class:svc.TaskService#changeTaskStatus()V",
        );
        let mut transition = states("s").transitions.remove(0);
        transition.evidence = vec![
            "task-manager:symbol:65a49248775beae815197acc".into(),
            "missing:evidence".into(),
        ];
        // A resolvable operation cannot override a dangling authored reference.
        assert!(!resolved(&checked, &transition));

        transition.evidence.pop();
        assert!(resolved(&checked, &transition));
        let mirrored = checked.services["svc"].observations
            [&"task-manager:symbol:65a49248775beae815197acc".to_string()]
            .clone();
        checked.dependencies.insert(
            "task-manager:symbol:65a49248775beae815197acc".into(),
            mirrored,
        );
        assert!(resolved(&checked, &transition));
    }

    #[test]
    fn operation_names_match_exactly_and_must_be_unambiguous() {
        let mut checked = empty_check();
        add_symbol(
            &mut checked,
            "svc",
            "symbol:one",
            "method:class:svc.TaskService#prechangeTaskStatus()V",
        );
        assert!(resolve_operation_symbol(&checked, "changeTaskStatus").is_none());

        add_symbol(
            &mut checked,
            "svc",
            "symbol:two",
            "method:class:svc.TaskService#changeTaskStatus()V",
        );
        assert_eq!(
            resolve_operation_symbol(&checked, "method:class:svc.TaskService#changeTaskStatus()V"),
            Some("method:class:svc.TaskService#changeTaskStatus()V")
        );
        assert_eq!(
            resolve_operation_symbol(&checked, "changeTaskStatus"),
            Some("method:class:svc.TaskService#changeTaskStatus()V")
        );

        add_symbol(
            &mut checked,
            "other",
            "symbol:three",
            "method:class:other.TaskService#changeTaskStatus()V",
        );
        assert!(resolve_operation_symbol(&checked, "changeTaskStatus").is_none());
    }

    #[test]
    fn load_rejects_undeclared_state_references_and_invalid_ids() {
        let (_root, repo) = docs_repo();
        for invalid_schema in [
            {
                let mut schema = states("s");
                schema.initial = "UNDECLARED".into();
                schema
            },
            {
                let mut schema = states("s");
                schema.final_states.push("UNDECLARED".into());
                schema
            },
            {
                let mut schema = states("s");
                schema.transitions[0].from = "UNDECLARED".into();
                schema
            },
            {
                let mut schema = states("s");
                schema.transitions[0].to = "UNDECLARED".into();
                schema
            },
        ] {
            write_schema(&repo, &invalid_schema);
            assert!(load(&repo, "s").is_err());
        }
        assert!(load(&repo, "../outside").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_scenario_symlink_escape() {
        let (_root, repo) = docs_repo();
        let outside = tempfile::tempdir().unwrap();
        std::fs::remove_dir(repo.root.join("scenarios")).unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.root.join("scenarios")).unwrap();
        assert!(load(&repo, "outside").is_err());
    }
}
