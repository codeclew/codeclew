//! Coordinator-owned review binding and portable accepted-version provenance.
use super::{
    bindings::Bindings,
    check::Check,
    digest, invalid,
    model::Observation,
    proposals::Artifact,
    store::Repository,
    work::{self, Request, Work},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeaningReview {
    pub schema: String,
    pub work: String,
    pub proposal: String,
    pub evidence_digest: String,
    pub verdict: String,
    pub assessed_claims: Vec<String>,
    pub assessed_operations: Vec<String>,
    pub issues: Vec<Issue>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Issue {
    pub severity: String,
    #[serde(default)]
    pub claim: Option<String>,
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptedVersion {
    pub schema: String,
    pub work: String,
    pub proposal: String,
    pub invocation: Option<String>,
    pub review_digest: Option<String>,
    pub reviewer_driver_digest: Option<String>,
    pub evidence_digest: String,
    pub read_digest: String,
    pub operation_digest: String,
    pub verification: String,
    pub limitations: Vec<String>,
    pub source_revisions: BTreeMap<String, String>,
    pub influence: BTreeMap<String, String>,
    pub external_request: Request,
    pub external_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_narrative_digest: Option<String>,
}

pub(super) fn validate(
    work: &Work,
    proposal: &Artifact,
    review: &MeaningReview,
    evidence_digest: &str,
) -> Result<(), ClewError> {
    if review.schema != "codeclew-documentation-review/1.0"
        || review.work != work.id
        || review.proposal != proposal.id
        || review.evidence_digest != evidence_digest
    {
        return Err(invalid(
            "REVIEW_BINDING_MISMATCH: review belongs to another work, proposal or evidence dispatch",
        ));
    }
    let actual: BTreeSet<_> = review.assessed_claims.iter().cloned().collect();
    let expected: BTreeSet<_> = proposal.claims.keys().cloned().collect();
    let operations: BTreeSet<_> = proposal
        .narrative
        .as_ref()
        .into_iter()
        .flat_map(|n| n.operations.iter().map(|o| o.id.clone()))
        .collect();
    if actual != expected
        || actual.len() != review.assessed_claims.len()
        || review
            .assessed_operations
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != operations
        || operations.len() != review.assessed_operations.len()
    {
        return Err(invalid(
            "REVIEW_COVERAGE_INCOMPLETE: assess every proposed claim and operation",
        ));
    }
    if !matches!(
        review.verdict.as_str(),
        "APPROVE" | "REJECT" | "NEEDS_EVIDENCE"
    ) || review.issues.len() > 128
        || review.limitations.len() > 64
        || review
            .limitations
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 2048)
    {
        return Err(invalid("invalid or unbounded meaning review"));
    }
    for issue in &review.issues {
        if !matches!(issue.severity.as_str(), "ERROR" | "LIMITATION")
            || issue.reason.trim().is_empty()
            || issue.reason.len() > 2048
            || issue
                .claim
                .as_ref()
                .is_some_and(|id| !expected.contains(id))
            || issue.evidence.len() > 32
            || issue.evidence.iter().any(|r| !work.handles.contains_key(r))
        {
            return Err(invalid(
                "review issue has invalid claim, evidence or severity",
            ));
        }
    }
    if review.verdict == "APPROVE" && review.issues.iter().any(|i| i.severity == "ERROR") {
        return Err(invalid("review approval contradicts its unresolved errors"));
    }
    if review.verdict != "APPROVE" && review.issues.is_empty() {
        return Err(invalid("non-approval needs an actionable review issue"));
    }
    Ok(())
}
fn input_scope(
    request: &Request,
    inputs: &BTreeMap<String, Value>,
) -> Result<Observation, ClewError> {
    let id = format!(
        "documentation-input-scope:{}",
        &digest(&request.external_inputs)?[7..]
    );
    let normalized = json!({"authority":"REGISTERED_HUMAN_OR_IMPORTED_INPUT","externalInputs":request.external_inputs,"membership":inputs.iter().map(|(path,record)|(path,json!({"status":record["status"],"digest":record["digest"],"reason":record["reason"]}))).collect::<BTreeMap<_,_>>()});
    Ok(Observation {
        id: id.clone(),
        kind: "DOCUMENTATION_INPUT_SCOPE".into(),
        service: String::new(),
        symbol: id,
        digest: digest(&normalized)?,
        normalized,
        source_ids: vec![],
    })
}
pub(super) fn scopes(
    repo: &Repository,
    checked: &mut Check,
    versions: impl Iterator<Item = AcceptedVersion>,
) -> Result<(), ClewError> {
    let mut seen = BTreeSet::new();
    for version in versions {
        let observation = input_scope(
            &version.external_request,
            &work::capture_inputs(repo, &version.external_request)?,
        )?;
        if seen.insert(observation.id.clone()) {
            checked
                .dependencies
                .insert(observation.id.clone(), observation);
        }
    }
    checked.refresh_digest()
}
pub(super) fn versions(
    work: &Work,
    proposal: &Artifact,
    review: &MeaningReview,
    invocation: &str,
    driver_digest: &str,
    evidence_digest: &str,
    read_digest: &str,
) -> Result<BTreeMap<String, AcceptedVersion>, ClewError> {
    validate(work, proposal, review, evidence_digest)?;
    if review.verdict != "APPROVE" || !proposal.status.starts_with("READY_") {
        return Err(invalid(
            "only an approved, machine-checked proposal can be accepted",
        ));
    }
    let mut limitations = review.limitations.clone();
    limitations.extend(
        review
            .issues
            .iter()
            .filter(|i| i.severity == "LIMITATION")
            .map(|i| i.reason.clone()),
    );
    if proposal.status == "READY_WITH_LIMITATIONS" {
        limitations.push(
            "The proposal retains explicit source/provider or authored uncertainty limitations."
                .into(),
        );
    }
    let verification = if limitations.is_empty() {
        "VERIFIED"
    } else {
        "VERIFIED_WITH_LIMITATIONS"
    };
    let mut versions = unassessed_versions(work, proposal, read_digest)?;
    for version in versions.values_mut() {
        version.invocation = Some(invocation.into());
        version.review_digest = Some(digest(review)?);
        version.reviewer_driver_digest = Some(driver_digest.into());
        version.evidence_digest = evidence_digest.into();
        version.verification = verification.into();
        version.limitations = limitations.clone();
    }
    Ok(versions)
}

/// Preserve the work boundary without inventing a reviewer or approval receipt.
pub(super) fn unassessed_versions(
    work: &Work,
    proposal: &Artifact,
    read_digest: &str,
) -> Result<BTreeMap<String, AcceptedVersion>, ClewError> {
    if !proposal.status.starts_with("READY_") {
        return Err(invalid(
            "only a machine-ready proposal can be published locally",
        ));
    }
    let scope = input_scope(&work.request, &work.external_inputs)?;
    let mut influence = work.influence.clone();
    influence.remove("documentation:external-inputs");
    influence.insert(scope.id, scope.digest);
    proposal
        .narrative
        .as_ref()
        .ok_or_else(|| invalid("proposal has no canonical content"))?
        .operations
        .iter()
        .map(|operation| {
            Ok((
                format!("{}/{}", work.subject, operation.id),
                AcceptedVersion {
                    schema: "codeclew-documentation-accepted-version/1.1".into(),
                    work: work.id.clone(),
                    proposal: proposal.id.clone(),
                    invocation: None,
                    review_digest: None,
                    reviewer_driver_digest: None,
                    evidence_digest: digest(&(&work.id, &proposal.id, read_digest))?,
                    read_digest: read_digest.into(),
                    operation_digest: digest(operation)?,
                    verification: "UNASSESSED".into(),
                    limitations: vec!["Published locally without separate meaning review.".into()],
                    source_revisions: work
                        .checked
                        .services
                        .iter()
                        .map(|(id, e)| (id.clone(), e.revision.clone()))
                        .collect(),
                    influence: influence.clone(),
                    external_request: work.request.clone(),
                    external_fingerprint: digest(&work.external_inputs)?,
                    previous_narrative_digest: Some(digest(&work.retained)?),
                },
            ))
        })
        .collect()
}
/// Add the entire recorded influence boundary to every rendered fragment of an operation.
pub(super) fn attach(
    binding: &mut Bindings,
    checked: &Check,
    versions: BTreeMap<String, AcceptedVersion>,
) -> Result<(), ClewError> {
    for (key, version) in versions {
        let (subject, operation) = key
            .split_once('/')
            .ok_or_else(|| invalid("invalid accepted section key"))?;
        let current = binding
            .narratives
            .get(subject)
            .and_then(|n| n.operations.iter().find(|o| o.id == operation))
            .ok_or_else(|| invalid("accepted operation is not present in publication"))?;
        if digest(current)? != version.operation_digest {
            return Err(invalid("accepted operation changed during publication"));
        }
        for (dependency, expected) in &version.influence {
            if checked
                .dependencies
                .get(dependency)
                .is_none_or(|o| &o.digest != expected)
            {
                return Err(invalid("accepted influence changed during publication"));
            }
        }
        for (id, fragment) in binding
            .fragments
            .iter_mut()
            .filter(|(id, _)| id.starts_with(&format!("{key}/")))
        {
            let _ = id;
            for dependency in version.influence.keys() {
                let observation = checked.dependencies[dependency].clone();
                fragment
                    .dependencies
                    .insert(dependency.clone(), observation.digest.clone());
                binding
                    .observations
                    .insert(dependency.clone(), observation.clone());
                if let Some(evidence) = &mut fragment.evidence {
                    evidence
                        .observations
                        .insert(dependency.clone(), observation);
                    evidence.revisions.extend(version.source_revisions.clone());
                }
            }
        }
        binding.accepted_versions.insert(key, version);
    }
    Ok(())
}
pub(super) fn verification(binding: &mut Bindings) {
    for (key, version) in &binding.accepted_versions {
        if let Some(state) = binding.section_states.get_mut(key) {
            state.verification = version.verification.clone();
        }
    }
    for subject in binding.narratives.keys() {
        let children: Vec<_> = binding
            .section_states
            .iter()
            .filter(|(key, _)| key.starts_with(&format!("{subject}/")))
            .map(|(_, s)| s.verification.as_str())
            .collect();
        let verification = if !children.is_empty() && children.iter().all(|s| *s == "VERIFIED") {
            "VERIFIED"
        } else if !children.is_empty() && children.iter().all(|s| s.starts_with("VERIFIED")) {
            "VERIFIED_WITH_LIMITATIONS"
        } else {
            "UNASSESSED"
        };
        if let Some(state) = binding.section_states.get_mut(subject) {
            state.verification = verification.into();
        }
    }
}
