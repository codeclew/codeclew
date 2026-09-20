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
use std::sync::Arc;

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
    #[serde(deserialize_with = "deserialize_accepted_schema")]
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
    pub influence: Influence,
    pub external_request: Request,
    pub external_fingerprint: String,
    #[serde(deserialize_with = "deserialize_previous_digest")]
    pub previous_narrative_digest: String,
}

/// Serialized identity of an immutable influence set. The hydrated map is
/// shared across accepted operations; only its digest travels in each version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Influence {
    pub scope: String,
    #[serde(skip)]
    pub data: Arc<InfluenceScope>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfluenceScope {
    pub dependencies: BTreeMap<String, String>,
    pub declarations: BTreeMap<String, Observation>,
}

impl Influence {
    fn new(dependencies: BTreeMap<String, String>, checked: &Check) -> Result<Self, ClewError> {
        let declarations = dependencies
            .keys()
            .filter_map(|id| {
                let observation = checked.dependencies.get(id)?;
                matches!(
                    observation.kind.as_str(),
                    "NOTE_ASSOCIATION"
                        | "DECLARED_INTERACTION"
                        | "SCENARIO_SELECTION"
                        | "PROCESS_DEFINITION"
                        | "VIEW_DEFINITION"
                        | "PROCESS_SCOPE"
                        | "VIEW_SCOPE"
                )
                .then(|| (id.clone(), observation.clone()))
            })
            .collect();
        let data = InfluenceScope {
            dependencies,
            declarations,
        };
        Ok(Self {
            scope: digest(&data)?,
            data: Arc::new(data),
        })
    }
}

pub(super) fn influence_scopes(
    versions: &BTreeMap<String, AcceptedVersion>,
) -> Result<BTreeMap<String, InfluenceScope>, ClewError> {
    let mut scopes = BTreeMap::new();
    for version in versions.values() {
        if !scopes.contains_key(&version.influence.scope) {
            if digest(version.influence.data.as_ref())? != version.influence.scope {
                return Err(invalid("accepted influence scope is missing or corrupt"));
            }
            scopes.insert(
                version.influence.scope.clone(),
                version.influence.data.as_ref().clone(),
            );
        }
    }
    Ok(scopes)
}

pub(super) fn resolve_influence_scopes(
    versions: &mut BTreeMap<String, AcceptedVersion>,
    scopes: &BTreeMap<String, InfluenceScope>,
) -> Result<(), ClewError> {
    let mut shared = BTreeMap::new();
    for (id, dependencies) in scopes {
        if digest(dependencies)? != *id {
            return Err(invalid("accepted influence scope digest is invalid"));
        }
        for (key, observation) in &dependencies.declarations {
            if key != &observation.id
                || dependencies.dependencies.get(key) != Some(&observation.digest)
                || digest(&observation.normalized)? != observation.digest
            {
                return Err(invalid(
                    "influence declaration does not match its recorded digest",
                ));
            }
        }
        shared.insert(id, Arc::new(dependencies.clone()));
    }
    for version in versions.values_mut() {
        version.influence.data = shared
            .get(&version.influence.scope)
            .ok_or_else(|| invalid("accepted influence scope is missing"))?
            .clone();
    }
    Ok(())
}

const ACCEPTED_VERSION_SCHEMA: &str = "codeclew-documentation-accepted-version/1.2";
fn deserialize_accepted_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let schema = String::deserialize(deserializer)?;
    if schema != ACCEPTED_VERSION_SCHEMA {
        return Err(serde::de::Error::custom(
            "DOCS_REINDEX_REQUIRED: unsupported accepted-version schema; initialize a fresh documentation root and run docs check",
        ));
    }
    Ok(schema)
}
fn valid_previous_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn deserialize_previous_digest<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let digest = String::deserialize(deserializer)?;
    if !valid_previous_digest(&digest) {
        return Err(serde::de::Error::custom(
            "DOCS_REINDEX_REQUIRED: accepted version lacks a valid previous narrative binding; initialize a fresh documentation root and run docs check",
        ));
    }
    Ok(digest)
}
pub(super) fn validate_accepted_version(version: &AcceptedVersion) -> Result<(), ClewError> {
    version.external_request.validate_documentation_language()?;
    if version.schema != ACCEPTED_VERSION_SCHEMA
        || !valid_previous_digest(&version.previous_narrative_digest)
    {
        return Err(invalid(
            "DOCS_REINDEX_REQUIRED: invalid accepted-version provenance; initialize a fresh documentation root and run docs check",
        ));
    }
    Ok(())
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
    attach_scopes(checked, &capture_scopes(repo, versions, &BTreeMap::new())?)
}

pub(super) fn capture_scopes(
    repo: &Repository,
    versions: impl Iterator<Item = AcceptedVersion>,
    notes: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Observation>, ClewError> {
    capture_scopes_with(versions, notes, |request| {
        work::capture_inputs_with_membership(repo, request)
    })
}

fn capture_scopes_with(
    versions: impl Iterator<Item = AcceptedVersion>,
    notes: &BTreeMap<String, Value>,
    mut capture: impl FnMut(&Request) -> Result<(BTreeSet<String>, BTreeMap<String, Value>), ClewError>,
) -> Result<BTreeMap<String, Observation>, ClewError> {
    let membership = |value: &Value| json!({"status":value["status"],"digest":value["digest"],"reason":value["reason"]});
    let mut observed = BTreeMap::new();
    for note in notes.values() {
        let path = note["association"]["path"]
            .as_str()
            .ok_or_else(|| invalid("captured note has no original path"))?;
        let value = membership(&note["original"]);
        if observed
            .insert(path.to_owned(), value.clone())
            .is_some_and(|old| old != value)
        {
            return Err(invalid("captured notes disagree about the same input path"));
        }
    }
    let mut scopes = BTreeMap::new();
    let mut protected_note_membership: Option<BTreeSet<String>> = None;
    let mut captured_bytes = 0usize;
    for version in versions {
        // Preserve the ordered request identity. Deduplicate before reading,
        // not after repeatedly traversing notes for each accepted version.
        let id = format!(
            "documentation-input-scope:{}",
            &digest(&version.external_request.external_inputs)?[7..]
        );
        if scopes.contains_key(&id) {
            continue;
        }
        let (note_membership, inputs) = capture(&version.external_request)?;
        if protected_note_membership
            .as_ref()
            .is_some_and(|previous| previous != &note_membership)
        {
            return Err(invalid(
                "protected notes membership changed between captured scopes",
            ));
        }
        protected_note_membership = Some(note_membership);
        for (path, value) in &inputs {
            captured_bytes = captured_bytes
                .checked_add(value["text"].as_str().map_or(0, str::len))
                .ok_or_else(|| invalid("composition external-input byte count overflow"))?;
            if captured_bytes > 64 * 1024 * 1024 {
                return Err(invalid(
                    "composition external-input captures exceed 64 MiB; narrow accepted input scopes",
                ));
            }
            let value = membership(value);
            if observed
                .insert(path.clone(), value.clone())
                .is_some_and(|old| old != value)
            {
                return Err(invalid(
                    "documentation inputs changed between captured scopes",
                ));
            }
            if observed.len() > 1024 {
                return Err(invalid(
                    "composition external-input membership exceeds 1024 paths",
                ));
            }
        }
        let observation = input_scope(&version.external_request, &inputs)?;
        scopes.insert(id, observation);
    }
    Ok(scopes)
}

pub(super) fn attach_scopes(
    checked: &mut Check,
    scopes: &BTreeMap<String, Observation>,
) -> Result<(), ClewError> {
    checked.dependencies.extend(scopes.clone());
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
    let influence = Influence::new(influence, &work.checked)?;
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
                    schema: "codeclew-documentation-accepted-version/1.2".into(),
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
                    previous_narrative_digest: digest(&work.retained)?,
                },
            ))
        })
        .collect()
}
/// Bind every fragment to the complete influence boundary without copying its
/// map or observations into each operation. Scope hashes preserve mixed versions.
pub(super) fn attach(
    binding: &mut Bindings,
    checked: &Check,
    versions: BTreeMap<String, AcceptedVersion>,
) -> Result<(), ClewError> {
    let scopes = influence_scopes(&versions)?;
    for (scope, data) in scopes {
        for (dependency, expected) in &data.dependencies {
            if checked
                .dependencies
                .get(dependency)
                .is_none_or(|o| &o.digest != expected)
            {
                return Err(invalid("accepted influence changed during publication"));
            }
        }
        binding.influence_scopes.insert(scope, data);
    }
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
        for (_, fragment) in binding
            .fragments
            .iter_mut()
            .filter(|(id, _)| id.starts_with(&format!("{key}/")))
        {
            fragment.influence_scope = Some(version.influence.scope.clone());
            if let Some(evidence) = &mut fragment.evidence {
                evidence.revisions.extend(version.source_revisions.clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn version(paths: &[&str]) -> AcceptedVersion {
        serde_json::from_value(json!({
            "schema":"codeclew-documentation-accepted-version/1.2", "work":"w", "proposal":"p",
            "previousNarrativeDigest":digest(&Option::<super::super::model::Narrative>::None).unwrap(),
            "invocation":null,"reviewDigest":null,"reviewerDriverDigest":null,
            "evidenceDigest":"e", "readDigest":"r", "operationDigest":"o", "verification":"VERIFIED",
            "limitations":[], "sourceRevisions":{}, "influence":{"scope":digest(&InfluenceScope::default()).unwrap()}, "externalFingerprint":"f",
            "externalRequest":{"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers","externalInputs":paths}
        })).unwrap()
    }

    #[test]
    fn influence_scope_is_shared_across_large_operation_sets_and_resolves_fail_closed() {
        let dependencies: BTreeMap<_, _> = (0..120_000)
            .map(|i| (format!("fact:{i:06}"), format!("sha256:{i:064x}")))
            .collect();
        let data = InfluenceScope {
            dependencies,
            declarations: BTreeMap::new(),
        };
        let scope = digest(&data).unwrap();
        let mut original = version(&[]);
        original.influence = Influence {
            scope: scope.clone(),
            data: Arc::new(data),
        };
        let versions: BTreeMap<_, _> = (0..128)
            .map(|i| (format!("service:orders/op{i}"), original.clone()))
            .collect();
        assert!(
            versions
                .values()
                .all(|v| Arc::ptr_eq(&v.influence.data, &original.influence.data))
        );
        let scopes = influence_scopes(&versions).unwrap();
        assert_eq!(scopes.len(), 1);
        let wire = serde_json::to_vec(&versions).unwrap();
        assert!(
            wire.len() < 256 * 1024,
            "operation metadata must not multiply 120000 facts"
        );
        let mut decoded: BTreeMap<String, AcceptedVersion> = serde_json::from_slice(&wire).unwrap();
        assert!(resolve_influence_scopes(&mut decoded, &BTreeMap::new()).is_err());
        resolve_influence_scopes(&mut decoded, &scopes).unwrap();
        assert_eq!(
            decoded
                .values()
                .next()
                .unwrap()
                .influence
                .data
                .dependencies
                .len(),
            120_000
        );
        let shared = &decoded.values().next().unwrap().influence.data;
        assert!(
            decoded
                .values()
                .all(|v| Arc::ptr_eq(&v.influence.data, shared))
        );
        let mut corrupt = scopes;
        corrupt
            .get_mut(&scope)
            .unwrap()
            .dependencies
            .remove("fact:000001");
        assert!(resolve_influence_scopes(&mut decoded, &corrupt).is_err());
    }

    #[test]
    fn accepted_version_requires_current_schema_and_previous_narrative_binding() {
        let current = serde_json::to_value(version(&[])).unwrap();
        assert!(serde_json::from_value::<AcceptedVersion>(current.clone()).is_ok());
        let mut old = current.clone();
        old["schema"] = json!("codeclew-documentation-accepted-version/1.0");
        assert!(
            serde_json::from_value::<AcceptedVersion>(old)
                .unwrap_err()
                .to_string()
                .contains("DOCS_REINDEX_REQUIRED")
        );
        for value in [Value::Null, json!(""), json!("sha256:bad")] {
            let mut unsupported = current.clone();
            unsupported["previousNarrativeDigest"] = value;
            assert!(serde_json::from_value::<AcceptedVersion>(unsupported).is_err());
        }
        let mut missing = current;
        missing
            .as_object_mut()
            .unwrap()
            .remove("previousNarrativeDigest");
        assert!(serde_json::from_value::<AcceptedVersion>(missing).is_err());
    }

    #[test]
    fn captured_review_scopes_deduplicate_before_reading_and_preserve_ordered_identity() {
        let a = version(&["manual/a.md", "manual/b.md"]);
        let b = version(&["manual/b.md", "manual/a.md"]);
        let mut calls = 0;
        let scopes = capture_scopes_with([a.clone(), a, b].into_iter(), &BTreeMap::new(), |_| {
            calls += 1;
            Ok((
                BTreeSet::new(),
                BTreeMap::from([(
                    "manual/a.md".into(),
                    json!({"status":"CAPTURED","digest":"a","text":"private text"}),
                )]),
            ))
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(scopes.len(), 2);
        assert!(
            !serde_json::to_string(&scopes)
                .unwrap()
                .contains("private text")
        );
    }

    #[test]
    fn captured_review_scopes_reject_conflicting_shared_paths_and_note_bytes() {
        let a = version(&["manual/a.md"]);
        let b = version(&["manual/b.md"]);
        let mut calls = 0;
        let error = capture_scopes_with([a.clone(), b].into_iter(), &BTreeMap::new(), |_| {
            calls += 1;
            Ok((
                BTreeSet::new(),
                BTreeMap::from([(
                    "notes/shared.md".into(),
                    json!({"status":"CAPTURED","digest":calls.to_string()}),
                )]),
            ))
        })
        .unwrap_err();
        assert!(error.message.contains("changed between captured scopes"));
        let notes = BTreeMap::from([(
            "policy".into(),
            json!({"association":{"path":"notes/shared.md"},"original":{"status":"CAPTURED","digest":"original"}}),
        )]);
        let error = capture_scopes_with([a].into_iter(), &notes, |_| {
            Ok((
                BTreeSet::new(),
                BTreeMap::from([(
                    "notes/shared.md".into(),
                    json!({"status":"CAPTURED","digest":"changed"}),
                )]),
            ))
        })
        .unwrap_err();
        assert!(error.message.contains("changed between captured scopes"));
    }

    #[test]
    fn captured_review_scopes_reject_note_membership_changes_but_ignore_external_paths() {
        let first = version(&["notes/x.md"]);
        let second = version(&["manual/missing.md"]);
        let mut calls = 0;
        let error = capture_scopes_with(
            [first.clone(), second.clone()].into_iter(),
            &BTreeMap::new(),
            |_| {
                calls += 1;
                Ok(if calls == 1 {
                    (
                        BTreeSet::from(["notes/x.md".into()]),
                        BTreeMap::from([(
                            "notes/x.md".into(),
                            json!({"status":"CAPTURED","digest":"x"}),
                        )]),
                    )
                } else {
                    (BTreeSet::new(), BTreeMap::new())
                })
            },
        )
        .unwrap_err();
        assert!(error.message.contains("protected notes membership"));

        let mut calls = 0;
        let scopes = capture_scopes_with([first, second].into_iter(), &BTreeMap::new(), |_| {
            calls += 1;
            Ok((
                BTreeSet::from(["notes/x.md".into()]),
                BTreeMap::from([(
                    if calls == 1 {
                        "notes/x.md"
                    } else {
                        "manual/missing.md"
                    }
                    .into(),
                    json!({"status":"ABSENT"}),
                )]),
            ))
        })
        .unwrap();
        assert_eq!(scopes.len(), 2);
    }
}
