//! Reader selection keeps complete retained authoring content independently.
#[cfg(not(test))]
use super::endpoint_publication::Selector;
#[cfg(test)]
use super::endpoint_publication::Selector;
use serde::{Deserialize, Serialize};
#[cfg(not(test))]
use serde_json::Value;
#[cfg(test)]
use serde_json::Value;
#[cfg(not(test))]
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationSelection {
    pub policy_digest: String,
    /// Full catalogue and operation delivery retained before reader filtering.
    pub retained_pages: BTreeMap<String, Value>,
    pub selectors: BTreeMap<String, BTreeMap<String, Selector>>,
}
