//! Typed admission for retained Kotlin local control-flow context.

use super::invalid;
use crate::{
    canonical,
    documentation::model::Source,
    documentation::store,
    error::ClewError,
    thread_flow_cfg::{self, LocalCfgPayload},
};
use serde::{Deserialize, Serialize};

pub(crate) const LOCAL_CFG_EVIDENCE_SCHEMA: &str = "codeclew-kotlin-documentation-local-cfg/1.0";
pub(crate) const LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA: &str =
    "codeclew-kotlin-documentation-local-cfg-boundary/1.0";

fn canonical_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

pub(crate) fn canonical_evidence_binding(value: &str) -> bool {
    canonical_sha256(value)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalCfgNodeCitation {
    pub node_id: u64,
    pub byte_start: u64,
    pub byte_end: u64,
    pub text_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalCfgSourceSite {
    pub source_id: String,
    pub source_digest: String,
    pub source_evidence_digest: String,
    pub source_status: String,
    pub authority: String,
    pub file: String,
    pub start_line: u64,
    pub end_line: u64,
    pub owner_byte_start: u64,
    pub owner_byte_end: u64,
    pub source_content_digest: String,
    pub full_compilation_source_digest: String,
    pub span_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalCfgEvidence {
    pub schema: String,
    pub scope: String,
    pub owner_symbol_identity: String,
    pub file: String,
    pub graph: LocalCfgPayload,
    pub graph_evidence_binding: String,
    pub descriptor_evidence_binding: String,
    pub node_citations: Vec<LocalCfgNodeCitation>,
    pub source_site: LocalCfgSourceSite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalCfgBoundaryEvidence {
    pub schema: String,
    pub kind: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_symbol_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_graph_name: Option<String>,
    pub code: String,
    pub provider: String,
    pub evidence_binding: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor_evidence_binding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_row_hash: Option<String>,
}

impl LocalCfgEvidence {
    pub(crate) fn validate_source(
        &self,
        source: &Source,
        service: &str,
        revision: &str,
    ) -> Result<(), ClewError> {
        thread_flow_cfg::validate(&self.graph)?;
        crate::semantic_validation::validate_kotlin_full_symbol_identity(
            &self.owner_symbol_identity,
        )?;
        let site = &self.source_site;
        let transformed_authority = crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY;
        if self.schema != LOCAL_CFG_EVIDENCE_SCHEMA
            || self.scope.is_empty()
            || self.file != self.graph.file
            || self.owner_symbol_identity != self.graph.owner_symbol_identity
            || !canonical_sha256(&self.graph_evidence_binding)
            || !canonical_sha256(&self.descriptor_evidence_binding)
            || site.source_status != "SOURCE_RETAINED"
            || site.source_id != source.id
            || source.service != service
            || source.revision != revision
            || site.file != self.file
            || site.authority != source.authority
            || !matches!(
                source.authority.as_str(),
                "EXACT_SNAPSHOT_TEXT" | crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY
            )
            || source.authority == transformed_authority && source.url.is_some()
            || site.source_digest != source.text_digest
            || site.source_evidence_digest != self.graph_evidence_binding
            || source.evidence_digest != self.graph_evidence_binding
            || !canonical_sha256(&site.source_digest)
            || !canonical_sha256(&site.source_content_digest)
            || site.source_content_digest != site.full_compilation_source_digest
            || site.span_digest != canonical::hash_bytes(source.text.as_bytes())
            || source.text_digest != canonical::hash_bytes(source.text.as_bytes())
            || source.id != site.source_id
            || source.file != self.file
            || source.start_line != site.start_line
            || source.end_line != site.end_line
            || source.start_line == 0
            || source.end_line < source.start_line
            || source.end_line - source.start_line + 1 != source.text.lines().count() as u64
            || source.occurrence.is_some()
            || site.owner_byte_start >= site.owner_byte_end
            || site.owner_byte_end - site.owner_byte_start != source.text.len() as u64
        {
            return Err(invalid(
                "Kotlin local CFG source or evidence bindings are inconsistent",
            ));
        }
        let mut citations = self.node_citations.iter();
        for node in &self.graph.nodes {
            let Some(range) = &node.source else {
                continue;
            };
            let citation = citations
                .next()
                .ok_or_else(|| invalid("Kotlin local CFG node citation is missing"))?;
            let relative_start = range
                .start
                .checked_sub(site.owner_byte_start)
                .ok_or_else(|| invalid("Kotlin local CFG node precedes its owner"))?;
            let relative_end = range
                .end
                .checked_sub(site.owner_byte_start)
                .ok_or_else(|| invalid("Kotlin local CFG node precedes its owner"))?;
            let start = usize::try_from(relative_start)
                .map_err(|_| invalid("Kotlin local CFG citation start is too large"))?;
            let end = usize::try_from(relative_end)
                .map_err(|_| invalid("Kotlin local CFG citation end is too large"))?;
            let exact = source
                .text
                .get(start..end)
                .ok_or_else(|| invalid("Kotlin local CFG citation is not a source slice"))?;
            if citation.node_id != node.node_id
                || citation.byte_start != relative_start
                || citation.byte_end != relative_end
                || citation.text_digest != canonical::hash_bytes(exact.as_bytes())
            {
                return Err(invalid(
                    "Kotlin local CFG node citation differs from retained source",
                ));
            }
        }
        if citations.next().is_some() {
            return Err(invalid("Kotlin local CFG has an unbound node citation"));
        }
        Ok(())
    }

    pub(crate) fn source_bound(
        normalized: &serde_json::Value,
        source: &Source,
        service: &str,
        revision: &str,
    ) -> bool {
        serde_json::from_value::<Self>(normalized.clone())
            .ok()
            .is_some_and(|evidence| evidence.validate_source(source, service, revision).is_ok())
    }
}

impl LocalCfgBoundaryEvidence {
    pub(crate) fn validate(&self, binding: &str) -> Result<(), ClewError> {
        if self.schema != LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA
            || self.kind != "LOCAL_CFG_BOUNDARY"
            || self.code.is_empty()
            || self.evidence_binding != binding
            || !canonical_sha256(binding)
            || self
                .descriptor_evidence_binding
                .as_deref()
                .is_some_and(|value| !canonical_sha256(value))
            || !matches!(
                self.provider.as_str(),
                "K2_FIR_CFG" | "CODECLEW_LOCAL_CFG_NORMALIZER"
            )
            || self
                .raw_row_hash
                .as_deref()
                .is_some_and(|value| !canonical_sha256(value))
            || (self.provider == "K2_FIR_CFG"
                && self
                    .raw_row_hash
                    .as_deref()
                    .is_none_or(|value| !canonical_sha256(value)))
        {
            return Err(invalid("Kotlin local CFG boundary is malformed"));
        }
        if let Some(owner) = self.owner_symbol_identity.as_deref() {
            crate::semantic_validation::validate_kotlin_full_symbol_identity(owner)?;
        }
        if let Some(file) = self.file.as_deref() {
            store::relative(file)?;
        }
        Ok(())
    }
}
