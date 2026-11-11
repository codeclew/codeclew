//! Closed author proposals and deterministic checks, separate from meaning acceptance.
use super::{
    bindings, check, digest, invalid,
    model::*,
    retained_validation,
    store::{self, Repository},
    work::{self, Work},
};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proposal {
    pub schema: String,
    pub operations: Vec<ProposedOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_edits: Vec<RetainedEdit>,
    #[serde(default)]
    pub gaps: BTreeMap<String, String>,
    #[serde(default)]
    pub uncertainties: Vec<String>,
}
/// A bounded edit against one exact canonical retained record. This is
/// deliberately not a generic JSON patch or a new evidence-bound source claim.
#[derive(Debug, Clone)]
pub struct RetainedEdit {
    pub kind: RetainedKind,
    pub id: String,
    pub record_digest: String,
    pub target: RetainedTarget,
    pub expected_old_value: String,
    pub replacement: String,
    pub fragment_id: Option<String>,
    pub author: Option<String>,
    pub context: Option<ContextInstruction>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetainedKind {
    RetainedOperation,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum RetainedTarget {
    OperationTitle,
    ExplanationText,
    ExplanationContext,
}
mod retained_wire;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextInstruction {
    pub expected_paragraph_digest: String,
    pub expected_context_digest: String,
    pub context_editor: String,
    pub source_references: Vec<String>,
    pub dependency_references: Vec<String>,
    pub anchors: Vec<ContextAnchor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextAnchor {
    pub event_id: String,
    pub expected_event_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedOperation {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "provided_visuals"
    )]
    pub visuals: Option<Vec<super::visuals::ProposedVisual>>,
    pub entrypoint: String,
    pub title: String,
    pub summary: Claim,
    #[serde(default)]
    pub assessment: Option<ProposedAssessment>,
    #[serde(default)]
    pub dataflow: Option<super::dataflow::ProposedGraph>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub contracts: Vec<Contract>,
    #[serde(default)]
    pub participants: Vec<ParticipantInput>,
    #[serde(default)]
    pub explanation: Vec<Claim>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticipantInput {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_service: Option<String>,
}
fn provided_visuals<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<super::visuals::ProposedVisual>>, D::Error> {
    Vec::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedAssessment {
    pub outcome: String,
    pub period: String,
    #[serde(default)]
    pub proposed_correction: Option<Claim>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Claim {
    pub text: String,
    pub evidence: Vec<String>,
    #[serde(default)]
    pub checks: Vec<Assertion>,
    #[serde(default)]
    pub uncertainty: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assertion {
    pub kind: String,
    pub evidence: String,
    pub field: String,
    pub expected: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedStepReader {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overview: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Claim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ProposedExecution>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedExecution {
    pub mode: String,
    pub meaning: Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise_reader: Option<ProposedStepReader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader: Option<ProposedStepReader>,
    pub kind: String,
    pub meaning: Claim,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub interaction: Option<String>,
    #[serde(default)]
    pub children: Vec<Step>,
    #[serde(default)]
    pub otherwise: Vec<Step>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contract {
    pub title: String,
    pub kind: String,
    pub rows: Vec<ContractRow>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<super::model::ContractField>,
    pub label: String,
    pub description: Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub schema: String,
    pub id: String,
    pub work: String,
    pub input: Proposal,
    pub narrative: Option<Narrative>,
    pub status: String,
    pub diagnostics: Vec<Value>,
    pub claims: BTreeMap<String, Value>,
    pub read_digest: String,
    pub influence: BTreeMap<String, String>,
    pub meaning_review: String,
}

// The exact Work identity already retains the full conservative influence map.
// Keep one authority for those bytes instead of copying them into every proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredArtifact {
    schema: String,
    artifact: Artifact,
    influence_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagnostics_ref: Option<super::cache::ObjectRef>,
}

impl StoredArtifact {
    #[cfg(test)]
    fn from_runtime(mut artifact: Artifact) -> Result<Self, ClewError> {
        let influence_digest = digest(&std::mem::take(&mut artifact.influence))?;
        artifact.id.clear();
        let mut stored = Self {
            schema: "codeclew-documentation-proposal-record/1.0".into(),
            artifact,
            influence_digest,
            diagnostics_ref: None,
        };
        stored.artifact.id = digest(&stored)?[7..].into();
        Ok(stored)
    }

    fn validate_identity(&self, id: &str) -> Result<(), ClewError> {
        let mut canonical = self.clone();
        canonical.artifact.id.clear();
        if self.schema != "codeclew-documentation-proposal-record/1.0"
            || self.artifact.schema != "codeclew-documentation-proposal-result/1.0"
            || !self.artifact.influence.is_empty()
            || self.diagnostics_ref.is_some() && !self.artifact.diagnostics.is_empty()
            || self.artifact.id != id
            || &digest(&canonical)?[7..] != id
        {
            return Err(invalid("proposal record digest or schema is invalid"));
        }
        Ok(())
    }

    fn hydrate(mut self, influence: BTreeMap<String, String>) -> Result<Artifact, ClewError> {
        if digest(&influence)? != self.influence_digest {
            return Err(invalid("proposal influence does not match its saved work"));
        }
        self.artifact.influence = influence;
        Ok(self.artifact)
    }
}

fn file(id: &str) -> Result<String, ClewError> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid proposal identity"));
    }
    Ok(format!(".codeclew/proposals/{id}.json"))
}
pub fn load(repo: &Repository, id: &str) -> Result<Artifact, ClewError> {
    let stored: StoredArtifact = store::read(&repo.path(&file(id)?)?, 4 * 1024 * 1024)?;
    stored.validate_identity(id)?;
    let influence = work::load_influence(repo, &stored.artifact.work)?;
    let reference = stored.diagnostics_ref.clone();
    let artifact = stored.hydrate(influence)?;
    hydrate_diagnostics(repo, reference, artifact)
}

fn hydrate_diagnostics(
    repo: &Repository,
    reference: Option<super::cache::ObjectRef>,
    mut artifact: Artifact,
) -> Result<Artifact, ClewError> {
    if let Some(reference) = reference {
        if reference.schema != DIAGNOSTICS_SCHEMA {
            return Err(invalid("proposal diagnostics reference schema is invalid"));
        }
        artifact.diagnostics =
            super::cache::get_json(repo, &reference, super::check::PORTABLE_CACHE_MAX_BYTES)?
                .ok_or_else(|| {
                    invalid(
                        "proposal diagnostics object is missing; restore its exact retained object",
                    )
                })?;
    }
    Ok(artifact)
}

const DIAGNOSTICS_SCHEMA: &str = "codeclew-documentation-proposal-diagnostics/1.0";

/// Validate saved evidence and admitted external inputs without checking whether
/// the retained narrative still matches the work's preparation baseline.
pub(super) fn current_evidence_inputs(repo: &Repository, work: &Work) -> Result<(), ClewError> {
    let snapshot = work.snapshot.as_deref().ok_or_else(|| ClewError::new(
        ErrorCode::StaleRequiresReslice,
        "DOCS_REINDEX_REQUIRED: Work requires a saved snapshot; prepare new Work from current-format evidence",
    ))?;
    let now = check::Check::load_snapshot(repo, snapshot)?;
    if now.input_digest != repo.input_digest()? || digest(&now)? != digest(&work.checked)? {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "saved work evidence or documentation declarations changed",
        ));
    }
    if now
        .services
        .iter()
        .map(|(id, e)| (id, &e.revision))
        .collect::<BTreeMap<_, _>>()
        != work
            .checked
            .services
            .iter()
            .map(|(id, e)| (id, &e.revision))
            .collect::<BTreeMap<_, _>>()
        || now.context_digest != work.checked.context_digest
        || now.input_digest != work.checked.input_digest
        || work::capture_inputs(repo, &work.request)? != work.external_inputs
    {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "work inputs changed; prepare new work and review the changed evidence",
        ));
    }
    Ok(())
}

/// Validate the selected evidence contract and admitted external inputs.
/// Snapshot work stays historical; its publication does not claim current sources.
pub fn current(repo: &Repository, work: &Work) -> Result<(), ClewError> {
    current_evidence_inputs(repo, work)?;
    let retained =
        bindings::baseline(repo)?.and_then(|(_, b)| b.narratives.get(&work.subject).cloned());
    let generated_after_preparation = work.retained.is_none()
        && retained
            .as_ref()
            .is_some_and(retained_validation::is_generated_placeholder);
    if retained != work.retained && !generated_after_preparation {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "published content changed after work preparation",
        ));
    }
    Ok(())
}

pub fn show(
    repo: &Repository,
    id: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    let a = load(repo, id)?;
    let mut rows = Vec::new();
    for (index, d) in a.diagnostics.iter().enumerate() {
        rows.push(json!({"kind":"DIAGNOSTIC","id":format!("diagnostic-{index}"),"record":d}));
    }
    for (id, claim) in &a.claims {
        rows.push(json!({"kind":"CLAIM","id":id,"record":claim}));
    }
    if let Some(n) = &a.narrative {
        for operation in &n.operations {
            rows.push(json!({"kind":"OPERATION","id":operation.id,"record":{"title":operation.title,"summary":operation.summary,"participants":operation.participants,"boundaries":operation.boundaries,"overviewDiagram":operation.overview_diagram}}));
            for visual in &operation.visuals {
                rows.push(json!({"kind":"VISUAL","id":format!("{}/{}",operation.id,visual.id),"record":visual}));
            }
            if let Some(g) = &operation.dataflow {
                rows.push(json!({"kind":"DATAFLOW_BINDING","id":operation.id,"record":{"schema":g.schema,"view":g.view,"definitionDigest":g.definition_digest,"moduleDigest":g.module_digest}}));
                for node in &g.nodes {
                    rows.push(json!({"kind":"DATAFLOW_NODE","id":node.id,"record":node}));
                }
                for edge in &g.edges {
                    rows.push(json!({"kind":"DATAFLOW_EDGE","id":edge.id,"record":edge}));
                }
            }
            for event in &operation.events {
                rows.push(json!({"kind":"DIAGRAM_STEP","id":event.id,"record":event}));
            }
            for paragraph in &operation.explanation {
                rows.push(json!({"kind":"EXPLANATION","id":paragraph.id,"record":paragraph}));
            }
            for contract in &operation.interface_contracts {
                rows.push(json!({"kind":"CONTRACT","id":contract.id,"record":{"title":contract.title,"kind":contract.kind,"boundaries":contract.boundaries}}));
                for row in &contract.rows {
                    rows.push(json!({"kind":"CONTRACT_ROW","id":row.id,"record":row}));
                }
            }
        }
        for (id, gap) in &n.gaps {
            rows.push(json!({"kind":"GAP","id":id,"record":gap}));
        }
    }
    super::cli::page(
        id,
        rows,
        cursor,
        limit,
        json!({"reportSchema":a.schema,"proposal":id,"work":a.work,"status":a.status,"meaningReview":a.meaning_review,"readDigest":a.read_digest,"narrativeDigest":a.narrative.as_ref().map(digest).transpose()?}),
    )
}

#[cfg(test)]
mod storage_tests {
    use super::super::bytes;
    use super::*;

    fn artifact(influence: BTreeMap<String, String>) -> Artifact {
        Artifact {
            schema: "codeclew-documentation-proposal-result/1.0".into(),
            id: String::new(),
            work: "a".repeat(64),
            input: Proposal {
                schema: "codeclew-documentation-proposal/1.0".into(),
                operations: vec![],
                retained_edits: Vec::new(),
                gaps: BTreeMap::new(),
                uncertainties: vec![],
            },
            narrative: None,
            status: "NEEDS_REPAIR".into(),
            diagnostics: vec![],
            claims: BTreeMap::new(),
            read_digest: "sha256:reads".into(),
            influence,
            meaning_review: "UNASSESSED".into(),
        }
    }

    #[test]
    fn large_influence_is_shared_without_changing_its_authority() {
        let influence: BTreeMap<_, _> = (0..40_000)
            .map(|i| {
                (
                    format!("service:dependency:{i:08}:{}", "x".repeat(40)),
                    format!("sha256:{}", "b".repeat(64)),
                )
            })
            .collect();
        assert!(bytes(&influence).unwrap().len() > 4 * 1024 * 1024);
        let original = artifact(influence.clone());
        let stored = StoredArtifact::from_runtime(original).unwrap();
        let encoded = bytes(&stored).unwrap();
        assert!(encoded.len() < 2048);
        let decoded: StoredArtifact = serde_json::from_slice(&encoded).unwrap();
        decoded.validate_identity(&stored.artifact.id).unwrap();
        let hydrated = decoded.hydrate(influence.clone()).unwrap();
        assert_eq!(hydrated.influence, influence);
        assert_eq!(hydrated.work, "a".repeat(64));
        assert_eq!(hydrated.id, stored.artifact.id);
        assert!(stored.hydrate(BTreeMap::new()).is_err());
    }

    #[test]
    fn proposal_record_rejects_mutation_and_competing_inline_authority() {
        let stored = StoredArtifact::from_runtime(artifact(BTreeMap::new())).unwrap();
        let id = stored.artifact.id.clone();
        let mut changed = stored.clone();
        changed.artifact.work = "b".repeat(64);
        assert!(changed.validate_identity(&id).is_err());
        let mut changed = stored.clone();
        changed.influence_digest = "sha256:wrong".into();
        assert!(changed.validate_identity(&id).is_err());
        let mut changed = stored;
        changed
            .artifact
            .influence
            .insert("unexpected".into(), "digest".into());
        changed.artifact.id.clear();
        changed.artifact.id = digest(&changed).unwrap()[7..].into();
        assert!(changed.validate_identity(&changed.artifact.id).is_err());
        assert!(
            serde_json::from_slice::<StoredArtifact>(&bytes(&artifact(BTreeMap::new())).unwrap())
                .is_err()
        );
    }
}

#[cfg(test)]
mod operation_input_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn historical_proposal_contract_row_keeps_its_closed_evidence_shape() {
        let claim = json!({"text":"Observed value meaning","evidence":["RETURNED_EVIDENCE_ID"]});
        let mut input = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{
            "entrypoint":"WORK_OPERATION_REFERENCE","title":"Observed operation","summary":claim,"steps":[],
            "contracts":[{"title":"Observed payload","kind":"payload","rows":[{"label":"value","description":claim}]}]
        }]});
        assert!(serde_json::from_value::<Proposal>(input.clone()).is_ok());
        input["operations"][0]["contracts"][0]["rows"][0]["description"]
            .as_object_mut()
            .unwrap()
            .remove("evidence");
        let error = serde_json::from_value::<Proposal>(input).unwrap_err();
        assert!(error.to_string().contains("evidence"));
    }
}

#[cfg(test)]
#[path = "proposals_retained_tests.rs"]
mod retained_edit_tests;
