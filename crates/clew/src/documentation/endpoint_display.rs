//! Reader selection keeps complete retained authoring content independently.
use super::{
    bindings::Bindings,
    check::Check,
    endpoint_publication::{Policy, Selector},
    model::{Narrative, SOURCE_EXTRACTOR, ServiceEvidence},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationSelection {
    pub policy_digest: String,
    /// Full catalogue and operation delivery retained before reader filtering.
    pub retained_pages: BTreeMap<String, Value>,
    pub selectors: BTreeMap<String, BTreeMap<String, Selector>>,
}

impl PublicationSelection {
    pub(super) fn prepare(
        policy: &Policy,
        checked: &Check,
        previous: Option<&Bindings>,
        previous_pages: &BTreeMap<String, Value>,
    ) -> Result<Self, ClewError> {
        let mut selection = previous
            .and_then(|b| b.endpoint_publication.clone())
            .unwrap_or(Self {
                policy_digest: policy.digest()?,
                retained_pages: BTreeMap::new(),
                selectors: BTreeMap::new(),
            });
        selection.policy_digest = policy.digest()?;
        if let Some(previous) = previous {
            for (subject, data) in previous_pages {
                let Some(service) = subject.strip_prefix("service:") else {
                    continue;
                };
                let mut entries = Vec::new();
                for row in data["catalogue"].as_array().into_iter().flatten() {
                    let mut row = row.clone();
                    if let Some(object) = row.as_object_mut() {
                        object.remove("retainedOnly");
                    }
                    entries.push(serde_json::from_value(row).map_err(super::io_error)?);
                }
                let observations: BTreeMap<_, _> = previous
                    .observations
                    .iter()
                    .filter(|(_, o)| o.service == service)
                    .map(|(id, o)| (id.clone(), o.clone()))
                    .collect();
                let syntax = observations
                    .values()
                    .any(|o| o.normalized["authority"] == "SYNTAX");
                let retained = ServiceEvidence {
                    schema: "codeclew-documentation-service-evidence/1.0".into(),
                    service: service.into(),
                    revision: previous.revisions.get(service).cloned().unwrap_or_default(),
                    service_digest: String::new(),
                    extractor: if syntax {
                        SOURCE_EXTRACTOR.into()
                    } else {
                        previous.extractor.clone()
                    },
                    runtime_mode: String::new(),
                    coverage: String::new(),
                    boundaries: Vec::new(),
                    entrypoints: entries,
                    observations,
                    sources: BTreeMap::new(),
                    contracts: BTreeMap::new(),
                };
                let selectors = selection.selectors.entry(service.into()).or_default();
                for (id, selector) in
                    super::endpoint_publication::selectors_for_entrypoints(&retained)
                {
                    selectors.entry(id).or_insert(selector);
                }
            }
        }
        for (service, evidence) in &checked.services {
            let selectors = selection.selectors.entry(service.clone()).or_default();
            let current = super::endpoint_publication::selectors_for_entrypoints(evidence);
            for entry in &evidence.entrypoints {
                if let Some(selector) = current.get(&entry.id) {
                    selectors.insert(entry.id.clone(), selector.clone());
                } else {
                    // An unresolvable current identity must not inherit an old association.
                    selectors.remove(&entry.id);
                }
            }
        }
        Ok(selection)
    }

    pub(super) fn hidden(&self, policy: &Policy, subject: &str) -> BTreeSet<String> {
        subject
            .strip_prefix("service:")
            .and_then(|id| self.selectors.get(id))
            .into_iter()
            .flat_map(|selectors| selectors.iter())
            .filter(|(_, selector)| policy.excludes(selector))
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub(super) fn restore(&self, subject: &str, data: &mut Value) {
        if let Some(retained) = self.retained_pages.get(subject) {
            for field in ["catalogue", "operationSources", "operationContracts"] {
                data[field] = retained[field].clone();
            }
        }
    }

    pub(super) fn retain(&mut self, subject: &str, data: &Value, hidden: &BTreeSet<String>) {
        if !hidden.is_empty() || self.retained_pages.contains_key(subject) {
            self.retained_pages.insert(subject.into(), json!({"catalogue":data["catalogue"],
                "operationSources":data["operationSources"], "operationContracts":data["operationContracts"]}));
        }
    }
}

pub(super) fn narrative(n: &Narrative, hidden: &BTreeSet<String>) -> Narrative {
    let mut displayed = n.clone();
    displayed.operations.retain(|op| !hidden.contains(&op.id));
    displayed.gaps.retain(|id, _| !hidden.contains(id));
    displayed
}

pub(super) fn filter_page(data: &mut Value, hidden: &BTreeSet<String>) {
    if hidden.is_empty() {
        return;
    }
    let hidden_declarations: BTreeSet<_> = data["catalogue"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["id"].as_str().is_some_and(|id| hidden.contains(id)))
        .flat_map(|entry| {
            entry["dependencyIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| {
                    Some((
                        entry["symbol"].as_str()?.to_owned(),
                        id.as_str()?.to_owned(),
                    ))
                })
        })
        .collect();
    for field in ["operations", "catalogue"] {
        if let Some(rows) = data[field].as_array_mut() {
            rows.retain(|row| row["id"].as_str().is_none_or(|id| !hidden.contains(id)));
        }
    }
    for field in [
        "gaps",
        "operationSources",
        "operationContracts",
        "operationStates",
        "translationGaps",
    ] {
        if let Some(rows) = data[field].as_object_mut() {
            rows.retain(|id, _| !hidden.contains(id));
        }
    }
    for field in ["fragmentSources", "fragmentStates"] {
        if let Some(rows) = data[field].as_object_mut() {
            rows.retain(|key, _| {
                !hidden.iter().any(|id| {
                    key == id || key.starts_with(&format!("{id}/")) || data_subject_key(key, id)
                })
            });
        }
    }
    if let Some(rows) = data["updateFailures"].as_object_mut() {
        rows.retain(|key, value| !failure_is_hidden(key, value, hidden));
    }
    if let Some(rows) = data["contracts"].as_array_mut() {
        rows.retain(|row| {
            row["normalized"]["entrypoint"]
                .as_str()
                .is_none_or(|id| !hidden.contains(id))
        });
    }
    if let Some(rows) = data["boundaryInventory"]["publicBoundaries"].as_array_mut() {
        rows.retain(|row| row["id"].as_str().is_none_or(|id| !hidden.contains(id)));
    }
    if let Some(rows) = data["boundaryInventory"]["internalCallables"].as_array_mut() {
        rows.retain(|row| {
            !row["dependencyIds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|id| {
                    row["symbol"]
                        .as_str()
                        .zip(id.as_str())
                        .is_some_and(|(symbol, id)| {
                            hidden_declarations.contains(&(symbol.to_owned(), id.to_owned()))
                        })
                })
        });
    }
    data["translationComplete"] = json!(
        data["translationGaps"]
            .as_object()
            .is_some_and(|g| g.is_empty())
    );
}

fn data_subject_key(key: &str, id: &str) -> bool {
    key.split_once('/')
        .is_some_and(|(_, operation)| operation == id || operation.starts_with(&format!("{id}/")))
}

pub(super) fn failure_is_hidden(key: &str, value: &Value, hidden: &BTreeSet<String>) -> bool {
    let operation = key.split_once('/').map_or(key, |(_, operation)| operation);
    let target = if value["reason"] == "INVALID_GAP" {
        operation.strip_prefix("gap-").unwrap_or(operation)
    } else {
        operation
    };
    hidden
        .iter()
        .any(|id| target == id || target.starts_with(&format!("{id}/")))
}
