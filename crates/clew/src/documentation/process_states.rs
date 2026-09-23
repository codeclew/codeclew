//! Declarative process state schema (`codeclew-documentation-process-states/1.0`):
//! load `<id>-states.yaml`, validate each transition's evidence against the
//! retained evidence, and render a PlantUML state diagram.
//!
//! The state machine is not derived from code — it is declared in
//! `scenarios/<id>-states.yaml` (per the approved design) and each transition
//! is cross-checked against retained evidence so that unresolved transitions
//! are reported as gaps rather than silently dropped.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::check::Check;
use super::store::{self, Repository};
use super::{invalid, plantuml};

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
    let path = repo.root.join(format!("scenarios/{id}-states.yaml"));
    if !path.exists() {
        return Ok(None);
    }
    let s: ProcessStates = store::read(&path, 256 * 1024)?;
    if s.schema != SCHEMA || s.id != id || !store::valid_id(&s.id) {
        return Err(invalid("invalid process-state schema identity"));
    }
    Ok(Some(s))
}

/// True when a transition's evidence resolves in the retained evidence: any of
/// its evidence ids matches an observation id or observation symbol, or its
/// `operation` names a retained entrypoint/observation symbol.
fn resolved(checked: &Check, t: &Transition) -> bool {
    if t.evidence.iter().any(|id| {
        checked.services.values().any(|e| {
            e.observations.contains_key(id) || e.observations.values().any(|o| o.symbol == *id)
        }) || checked.dependencies.contains_key(id)
    }) {
        return true;
    }
    if !t.operation.is_empty() {
        let needle = &t.operation;
        if checked.services.values().any(|e| {
            e.observations.values().any(|o| o.symbol.contains(needle))
                || e.entrypoints.iter().any(|ep| ep.symbol.contains(needle))
        }) {
            return true;
        }
    }
    false
}

/// Render a state diagram as PlantUML. Unresolved transitions (those whose
/// evidence does not resolve against retained evidence) are returned separately
/// so the caller can surface them as gaps.
pub fn render_puml(s: &ProcessStates, checked: &Check) -> (String, Vec<String>) {
    let mut out = String::new();
    out.push_str(&format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {} — состояния\nhide empty description\n",
        plantuml::escape(&s.title)
    ));
    out.push_str(&format!("[*] --> {} : начальное\n", s.initial));
    let mut unresolved = Vec::new();
    for t in &s.transitions {
        if !resolved(checked, t) {
            unresolved.push(format!("{} -> {} [{}]", t.from, t.to, t.operation));
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
        out.push_str(&format!("{} --> [*] : конечное\n", f));
    }
    out.push_str("@enduml\n");
    (out, unresolved)
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
title: Управление жизненным циклом задачи
initial: WAIT
final: [FINISHED, ERROR]
states:
  WAIT: "ожидает обработки"
  PROCESSING: "обрабатывается"
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
        let svc: crate::documentation::model::ServiceEvidence = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
            "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
            "boundaries":[],"contracts":{},"entrypoints":[],"observations":{},"sources":{}
        }))
        .unwrap();
        let mut svc = svc;
        svc.observations.insert(
            "task-manager:symbol:65a49248775beae815197acc".into(),
            crate::documentation::model::Observation {
                id: "task-manager:symbol:65a49248775beae815197acc".into(),
                kind: "SYMBOL".into(),
                service: "svc".into(),
                symbol: "method:class:svc.TaskService#changeTaskStatus()V".into(),
                normalized: json!({"documentation":{"events":[]}}),
                digest: "d".into(),
                source_ids: vec![],
            },
        );
        checked.services.insert("svc".into(), svc);
        let (puml, unresolved) = render_puml(&states("s"), &checked);
        assert!(
            puml.contains(
                "PROCESSING --> WAIT_DECISION : status → WAIT_DECISION [changeTaskStatus]"
            ),
            "{puml}"
        );
        assert!(puml.contains("[*] --> WAIT : начальное"));
        assert!(puml.contains("FINISHED --> [*] : конечное"));
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
}
