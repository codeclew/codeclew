//! Exact retained process root identity and source record digests.
use super::{
    check::Check,
    digest,
    model::{Observation, Selector, Source},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[cfg(test)]
struct ProcessRootResolution<'a> {
    service: Option<String>,
    selector_scope: Option<String>,
    observations: Vec<&'a Observation>,
}

/// Resolve the explicitly saved process root through its exact selector and
/// selected service. Unlike lifecycle discovery, this path never guesses from
/// a simple method name or searches other services.
#[cfg(test)]
fn process_root_resolution<'a>(checked: &'a Check, id: &str) -> ProcessRootResolution<'a> {
    let Some(definition) = checked.dependencies.get(&format!("process:{id}")) else {
        return ProcessRootResolution {
            service: None,
            selector_scope: None,
            observations: Vec::new(),
        };
    };
    let root = &definition.normalized["definition"]["root"];
    let Some(service_id) = root["service"].as_str() else {
        return ProcessRootResolution {
            service: None,
            selector_scope: None,
            observations: Vec::new(),
        };
    };
    let Some(service) = checked.services.get(service_id) else {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope: None,
            observations: Vec::new(),
        };
    };
    let Some(selector_value) = root.get("selector").filter(|value| !value.is_null()) else {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope: None,
            observations: Vec::new(),
        };
    };
    let Ok(selector) = serde_json::from_value::<Selector>(selector_value.clone()) else {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope: None,
            observations: Vec::new(),
        };
    };
    let selector_scope = selector.scope.clone();
    if !matches!(
        selector.language.as_str(),
        "java" | "kotlin" | "typescript" | "csharp"
    ) {
        return ProcessRootResolution {
            service: Some(service_id.to_owned()),
            selector_scope,
            observations: Vec::new(),
        };
    }
    let observations = super::analysis::resolve(Some(&selector), service);

    ProcessRootResolution {
        service: Some(service_id.to_owned()),
        selector_scope,
        observations,
    }
}

#[cfg(test)]
impl ProcessRootResolution<'_> {
    #[cfg(test)]
    fn metadata(&self, source_record_digests: &BTreeMap<String, String>) -> Value {
        let selected = (self.observations.len() == 1).then(|| self.observations[0]);
        let mut source_ids =
            selected.map_or_else(Vec::new, |observation| observation.source_ids.clone());
        source_ids.sort();
        source_ids.dedup();
        json!({
            "service": self.service,
            "scope": selected
                .and_then(|observation| observation.normalized["scope"].as_str())
                .or(self.selector_scope.as_deref()),
            "symbol": selected.map(|observation| &observation.symbol),
            "observation": selected.map(|observation| &observation.id),
            "observationDigest": selected.map(|observation| &observation.digest),
            "sourceIds": source_ids,
            "sourceRecordDigests": source_record_digests,
            "candidates": self.observations.iter().map(|observation| json!({
                "service": observation.service,
                "scope": observation.normalized["scope"],
                "symbol": observation.symbol,
                "observation": observation.id,
                "observationDigest": observation.digest,
            })).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
pub(super) fn selected_process_root_metadata(
    checked: &Check,
    id: &str,
) -> Result<Value, ClewError> {
    let root = process_root_resolution(checked, id);
    let (_, digests) = process_root_source_records(checked, &root)?;
    Ok(root.metadata(&digests))
}
#[cfg(test)]
fn process_root_source_records(
    checked: &Check,
    root: &ProcessRootResolution<'_>,
) -> Result<ProcessRootSourceRecords, ClewError> {
    let mut records = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let (Some(service_id), [observation]) = (root.service.as_deref(), root.observations.as_slice())
    else {
        return Ok((records, digests));
    };
    let Some(service) = checked.services.get(service_id) else {
        return Ok((records, digests));
    };
    let mut ids = observation.source_ids.clone();
    ids.sort();
    ids.dedup();
    for id in ids {
        if let Some(source) = service.sources.get(&id) {
            digests.insert(id.clone(), digest(source)?);
            records.insert(id, source.clone());
        }
    }
    Ok((records, digests))
}
#[cfg(test)]
type ProcessRootSourceRecords = (BTreeMap<String, Source>, BTreeMap<String, String>);
