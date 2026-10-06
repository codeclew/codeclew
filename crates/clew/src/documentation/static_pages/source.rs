//! Shared internal context for source-language projection adapters.
use super::model::{CallableProjection, Citation, Gap};
use crate::documentation::invalid;
use crate::documentation::model::{Observation, ServiceEvidence, Source};
use crate::error::ClewError;
use std::collections::BTreeMap;

pub(super) mod java;
pub(super) use java::{Parsed, all_steps, compiler, handoff as java_handoff};

pub(super) fn gap(code: &str, detail: impl Into<String>, citation_id: Option<String>) -> Gap {
    Gap {
        code: code.into(),
        detail: detail.into(),
        citation_id,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CallableKey {
    pub service: String,
    pub scope: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedCallable {
    pub key: CallableKey,
    pub projection: CallableProjection,
}

pub(super) struct Context<'a> {
    pub(super) evidence: &'a ServiceEvidence,
    pub(super) citations: BTreeMap<String, Citation>,
    pub(super) observations: BTreeMap<String, Observation>,
    pub(super) sources: BTreeMap<String, Source>,
}

impl<'a> Context<'a> {
    pub(super) fn new(evidence: &'a ServiceEvidence) -> Self {
        Self {
            evidence,
            citations: BTreeMap::new(),
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
        }
    }

    fn retain(&mut self, o: &Observation) {
        self.observations.insert(o.id.clone(), o.clone());
        for id in &o.source_ids {
            if let Some(source) = self.evidence.sources.get(id) {
                self.sources.insert(id.clone(), source.clone());
            }
        }
    }
    fn valid_source(&self, source: &Source) -> bool {
        source.service == self.evidence.service
            && source.revision == self.evidence.revision
            && !source.authority.is_empty()
            && !source.evidence_digest.is_empty()
            && !source.text.is_empty()
            && source.start_line > 0
            && source.end_line >= source.start_line
            && source.end_line - source.start_line + 1 == source.text.lines().count() as u64
            && source
                .occurrence
                .as_ref()
                .is_none_or(|o| o.end_byte.checked_sub(o.start_byte) == Some(source.text.len()))
            && source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
    }
    fn citation(&mut self, source: &Source, start: usize, end: usize) -> String {
        let start = start.min(source.text.len());
        let end = end.min(source.text.len()).max(start);
        let id = format!(
            "citation-{}",
            &crate::canonical::hash_bytes(
                format!("{}:{start}:{end}:{}", source.id, source.text_digest).as_bytes()
            )[7..31]
        );
        let start_line =
            source.start_line + source.text[..start].bytes().filter(|b| *b == b'\n').count() as u64;
        let end_line = source.start_line
            + source.text.as_bytes()[..end.saturating_sub(1).max(start)]
                .iter()
                .filter(|b| **b == b'\n')
                .count() as u64;
        let url = source.url.as_ref().map(|u| {
            format!(
                "{}#L{start_line}-L{end_line}",
                u.split('#').next().unwrap_or(u)
            )
        });
        self.sources.insert(source.id.clone(), source.clone());
        self.citations.entry(id.clone()).or_insert(Citation {
            id: id.clone(),
            source_id: source.id.clone(),
            service: source.service.clone(),
            revision: source.revision.clone(),
            file: source.file.clone(),
            start_line,
            end_line,
            start_byte: start,
            end_byte: end,
            text_digest: crate::canonical::hash_bytes(&source.text.as_bytes()[start..end]),
            evidence_digest: source.evidence_digest.clone(),
            authority: source.authority.clone(),
            url,
        });
        id
    }
}

pub(super) fn project_java(
    context: &mut Context<'_>,
    declaration: &str,
) -> Result<ProjectedCallable, ClewError> {
    let projection = context.callable(declaration)?;
    let observation = context
        .evidence
        .observations
        .get(declaration)
        .ok_or_else(|| {
            invalid(format!(
                "selected callable declaration {declaration} is missing"
            ))
        })?;
    let key = CallableKey {
        service: context.evidence.service.clone(),
        scope: observation.normalized["scope"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        symbol: projection.symbol.clone(),
    };
    Ok(ProjectedCallable { key, projection })
}
