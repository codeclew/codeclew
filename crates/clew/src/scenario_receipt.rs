//! Provider-neutral runtime observations bound to one prepared workspace.
//!
//! A receipt can add `OBSERVED_RUNTIME` evidence, but it cannot upgrade
//! compiler shape, artifact ownership, or contract certainty. Provider-specific
//! execution stays outside core; the submitted result and its private raw bytes
//! are verified and retained here.

use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::state::StateAuthority;
use crate::workspace::WorkspaceEvidenceAuthority;
use crate::workspace_prepare::{AfterWorkspace, load_after_for_receipt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

pub const SCENARIO_INPUT_SCHEMA: &str = "codeclew-scenario-observation-input/1.0";
pub const SCENARIO_RECEIPT_SCHEMA: &str = "codeclew-scenario-receipt/1.0";
pub const SCENARIO_RAW_EVIDENCE_SCHEMA: &str = "codeclew-scenario-raw-evidence/1.0";
pub const MAX_SCENARIO_INPUT_BYTES: usize = 256 * 1024;
pub const MAX_SCENARIO_RAW_EVIDENCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_SCENARIO_RECEIPT_BYTES: usize = 64 * 1024;
const MAX_SCENARIO_CHECKS: usize = 128;
const MAX_SCENARIO_DURATION_MS: u128 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioProviderSpec {
    pub provider_id: String,
    pub provider_version: String,
    pub config_digest: String,
    pub action_id: String,
    pub action_digest: String,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScenarioStatus {
    Passed,
    Failed,
    Inconclusive,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScenarioCheckStatus {
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioCheckInput {
    pub id: String,
    pub status: ScenarioCheckStatus,
    pub observation_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioObservationInput {
    pub schema: String,
    pub preparation_id: String,
    pub after_workspace_id: String,
    pub after_workspace_authority_digest: String,
    pub scenario_id: String,
    pub provider: ScenarioProviderSpec,
    pub status: ScenarioStatus,
    pub started_unix_ms: u128,
    pub finished_unix_ms: u128,
    pub checks: Vec<ScenarioCheckInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioCertaintyAxes {
    pub compiler_shape: String,
    pub artifact_ownership: WorkspaceEvidenceAuthority,
    pub contract: WorkspaceEvidenceAuthority,
    pub runtime: WorkspaceEvidenceAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioReceipt {
    pub schema: String,
    pub receipt_id: String,
    pub authority_digest: String,
    pub workspace_id: String,
    pub workspace_authority_digest: String,
    pub workspace_semantic_digest: String,
    pub preparation_id: String,
    pub after_workspace_id: String,
    pub after_workspace_authority_digest: String,
    pub candidate_set_digest: String,
    pub scenario_id: String,
    pub provider: ScenarioProviderSpec,
    pub status: ScenarioStatus,
    pub started_unix_ms: u128,
    pub finished_unix_ms: u128,
    pub checks: Vec<ScenarioCheckInput>,
    pub raw_evidence: CasObject,
    pub certainty: ScenarioCertaintyAxes,
    pub satisfaction: String,
    pub obligations: Vec<String>,
}

pub trait ScenarioProvider {
    fn specification(&self) -> &ScenarioProviderSpec;
}

pub trait RuntimeEvidenceProvider: ScenarioProvider {
    fn observation(&self) -> &ScenarioObservationInput;
    fn raw_evidence(&self) -> &[u8];
}

struct SubmittedObservation<'a> {
    input: &'a ScenarioObservationInput,
    raw: &'a [u8],
}

impl ScenarioProvider for SubmittedObservation<'_> {
    fn specification(&self) -> &ScenarioProviderSpec {
        &self.input.provider
    }
}

impl RuntimeEvidenceProvider for SubmittedObservation<'_> {
    fn observation(&self) -> &ScenarioObservationInput {
        self.input
    }

    fn raw_evidence(&self) -> &[u8] {
        self.raw
    }
}

pub fn record(
    workspace_id: &str,
    input_bytes: &[u8],
    raw_evidence: &[u8],
) -> Result<ScenarioReceipt, ClewError> {
    let input = parse_input(input_bytes)?;
    let submitted = SubmittedObservation {
        input: &input,
        raw: raw_evidence,
    };
    record_provider(workspace_id, &submitted)
}

fn record_provider(
    workspace_id: &str,
    provider: &impl RuntimeEvidenceProvider,
) -> Result<ScenarioReceipt, ClewError> {
    let input = provider.observation();
    if provider.specification() != &input.provider {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "runtime evidence provider differs from scenario request",
        ));
    }
    validate_input(input, provider.raw_evidence())?;
    let after = load_after_for_receipt(
        workspace_id,
        &input.preparation_id,
        &input.after_workspace_id,
        &input.after_workspace_authority_digest,
    )?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let raw_evidence = store.put(SCENARIO_RAW_EVIDENCE_SCHEMA, provider.raw_evidence())?;
    let candidate_set_digest = candidate_set_digest(&after)?;
    let (runtime, satisfaction, obligations) = receipt_status(input);
    let semantic = json!({
        "schema":"codeclew-scenario-receipt-semantic-authority/1.0",
        "workspaceId":after.workspace_id,
        "workspaceAuthorityDigest":after.workspace_authority_digest,
        "workspaceSemanticDigest":after.workspace_semantic_digest,
        "preparationId":after.preparation_id,
        "afterWorkspaceId":after.after_workspace_id,
        "afterWorkspaceAuthorityDigest":after.authority_digest,
        "candidateSetDigest":candidate_set_digest,
        "scenarioId":input.scenario_id,
        "provider":input.provider,
        "status":input.status,
        "startedUnixMs":input.started_unix_ms,
        "finishedUnixMs":input.finished_unix_ms,
        "checks":input.checks,
        "rawEvidence":raw_evidence,
        "certainty":{
            "compilerShape":"BOUND_AFTER_WORKSPACE_MEMBER_AUTHORITIES",
            "artifactOwnership":WorkspaceEvidenceAuthority::Unknown,
            "contract":WorkspaceEvidenceAuthority::Unknown,
            "runtime":runtime,
        },
        "satisfaction":satisfaction,
        "obligations":obligations,
    });
    let semantic_digest = canonical::hash(&semantic).map_err(internal)?;
    let mut receipt = ScenarioReceipt {
        schema: SCENARIO_RECEIPT_SCHEMA.into(),
        receipt_id: format!("scenario-receipt:{semantic_digest}"),
        authority_digest: String::new(),
        workspace_id: after.workspace_id,
        workspace_authority_digest: after.workspace_authority_digest,
        workspace_semantic_digest: after.workspace_semantic_digest,
        preparation_id: after.preparation_id,
        after_workspace_id: after.after_workspace_id,
        after_workspace_authority_digest: after.authority_digest,
        candidate_set_digest,
        scenario_id: input.scenario_id.clone(),
        provider: input.provider.clone(),
        status: input.status,
        started_unix_ms: input.started_unix_ms,
        finished_unix_ms: input.finished_unix_ms,
        checks: input.checks.clone(),
        raw_evidence,
        certainty: ScenarioCertaintyAxes {
            compiler_shape: "BOUND_AFTER_WORKSPACE_MEMBER_AUTHORITIES".into(),
            artifact_ownership: WorkspaceEvidenceAuthority::Unknown,
            contract: WorkspaceEvidenceAuthority::Unknown,
            runtime,
        },
        satisfaction,
        obligations,
    };
    receipt.authority_digest = receipt_authority_digest(&receipt)?;
    persist(&state, &receipt)?;
    Ok(receipt)
}

fn parse_input(source: &[u8]) -> Result<ScenarioObservationInput, ClewError> {
    if source.is_empty() || source.len() > MAX_SCENARIO_INPUT_BYTES {
        return Err(invalid("scenario input is empty or exceeds 256 KiB"));
    }
    let value: Value =
        serde_json::from_slice(source).map_err(|_| invalid("scenario input is not valid JSON"))?;
    if canonical::bytes(&value).map_err(internal)? != source {
        return Err(invalid(
            "scenario input must be canonical compact JSON with NFC strings",
        ));
    }
    let input: ScenarioObservationInput =
        serde_json::from_value(value).map_err(|_| invalid("scenario input schema is invalid"))?;
    Ok(input)
}

fn validate_input(input: &ScenarioObservationInput, raw_evidence: &[u8]) -> Result<(), ClewError> {
    let provider = &input.provider;
    let mut checks = BTreeSet::new();
    let raw_evidence_digest = canonical::hash_bytes(raw_evidence);
    if input.schema != SCENARIO_INPUT_SCHEMA
        || !safe_id(&input.scenario_id)
        || !safe_id(&provider.provider_id)
        || !safe_id(&provider.provider_version)
        || !safe_id(&provider.action_id)
        || !digest(&provider.config_digest)
        || !digest(&provider.action_digest)
        || input.started_unix_ms == 0
        || input.finished_unix_ms < input.started_unix_ms
        || input.finished_unix_ms - input.started_unix_ms > MAX_SCENARIO_DURATION_MS
        || input.checks.len() > MAX_SCENARIO_CHECKS
        || input.checks.iter().any(|check| {
            !safe_id(&check.id)
                || !checks.insert(check.id.as_str())
                || !digest(&check.observation_digest)
                || check.observation_digest != raw_evidence_digest
        })
        || raw_evidence.len() > MAX_SCENARIO_RAW_EVIDENCE_BYTES
        || (!matches!(input.status, ScenarioStatus::Unavailable)
            && (input.checks.is_empty() || raw_evidence.is_empty()))
        || (matches!(input.status, ScenarioStatus::Unavailable) && !input.checks.is_empty())
    {
        return Err(invalid("scenario observation authority is invalid"));
    }
    let expected = match input.status {
        ScenarioStatus::Passed => ScenarioCheckStatus::Passed,
        ScenarioStatus::Failed => ScenarioCheckStatus::Failed,
        ScenarioStatus::Inconclusive => ScenarioCheckStatus::Inconclusive,
        ScenarioStatus::Unavailable => return Ok(()),
    };
    if !input.checks.iter().any(|check| check.status == expected) {
        return Err(invalid(
            "scenario status is not supported by any submitted check",
        ));
    }
    Ok(())
}

fn receipt_status(
    input: &ScenarioObservationInput,
) -> (WorkspaceEvidenceAuthority, String, Vec<String>) {
    match input.status {
        ScenarioStatus::Passed => (
            WorkspaceEvidenceAuthority::ObservedRuntime,
            "OBSERVED_PASSED".into(),
            Vec::new(),
        ),
        ScenarioStatus::Failed => (
            WorkspaceEvidenceAuthority::ObservedRuntime,
            "OBSERVED_FAILED".into(),
            vec!["RESOLVE_OBSERVED_SCENARIO_FAILURE".into()],
        ),
        ScenarioStatus::Inconclusive => (
            WorkspaceEvidenceAuthority::ObservedRuntime,
            "OBSERVED_INCONCLUSIVE".into(),
            vec!["VERIFY_INCONCLUSIVE_SCENARIO".into()],
        ),
        ScenarioStatus::Unavailable if input.provider.required => (
            WorkspaceEvidenceAuthority::Unknown,
            "REQUIRED_PROVIDER_UNAVAILABLE".into(),
            vec!["RUN_REQUIRED_SCENARIO_PROVIDER".into()],
        ),
        ScenarioStatus::Unavailable => (
            WorkspaceEvidenceAuthority::Unknown,
            "OPTIONAL_PROVIDER_UNAVAILABLE".into(),
            vec!["OPTIONAL_SCENARIO_NOT_OBSERVED".into()],
        ),
    }
}

fn candidate_set_digest(after: &AfterWorkspace) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-scenario-candidate-set/1.0",
        "afterWorkspaceId":after.after_workspace_id,
        "members":after.members.iter().map(|member| json!({
            "alias":member.alias,
            "candidateOid":member.candidate_oid,
            "preparedAuthorityDigest":member.prepared_authority_digest,
        })).collect::<Vec<_>>(),
    }))
    .map_err(internal)
}

fn persist(state: &StateAuthority, receipt: &ScenarioReceipt) -> Result<(), ClewError> {
    validate_receipt(receipt)?;
    let bytes = canonical::bytes(receipt).map_err(internal)?;
    if bytes.len() > MAX_SCENARIO_RECEIPT_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "scenario receipt exceeds 64 KiB",
        ));
    }
    let component = receipt
        .receipt_id
        .strip_prefix("scenario-receipt:sha256:")
        .filter(|value| digest_component(value))
        .ok_or_else(|| invalid("scenario receipt id is invalid"))?;
    let directory = state.directory_at(
        &state
            .workspace_root(&receipt.workspace_id)?
            .join(Path::new("receipts")),
    )?;
    let name = format!("{component}.json");
    if !directory.atomic_create(OsStr::new(&name), &bytes)? {
        let existing = directory.read_file(OsStr::new(&name), MAX_SCENARIO_RECEIPT_BYTES)?;
        if existing != bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "scenario receipt identity already exists with different content",
            ));
        }
    }
    Ok(())
}

fn validate_receipt(receipt: &ScenarioReceipt) -> Result<(), ClewError> {
    if receipt.schema != SCENARIO_RECEIPT_SCHEMA
        || receipt.authority_digest != receipt_authority_digest(receipt)?
        || !digest(&receipt.workspace_authority_digest)
        || !digest(&receipt.workspace_semantic_digest)
        || !digest(&receipt.after_workspace_authority_digest)
        || !digest(&receipt.candidate_set_digest)
        || !digest(&receipt.raw_evidence.digest)
        || receipt.certainty.compiler_shape != "BOUND_AFTER_WORKSPACE_MEMBER_AUTHORITIES"
        || receipt.certainty.artifact_ownership != WorkspaceEvidenceAuthority::Unknown
        || receipt.certainty.contract != WorkspaceEvidenceAuthority::Unknown
    {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "scenario receipt authority is invalid",
        ));
    }
    Ok(())
}

fn receipt_authority_digest(receipt: &ScenarioReceipt) -> Result<String, ClewError> {
    let mut unsigned = receipt.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(digest_component)
}

fn digest_component(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(required: bool) -> ScenarioProviderSpec {
        ScenarioProviderSpec {
            provider_id: "local-runtime".into(),
            provider_version: "1.0".into(),
            config_digest: format!("sha256:{}", "a".repeat(64)),
            action_id: "smoke".into(),
            action_digest: format!("sha256:{}", "b".repeat(64)),
            required,
        }
    }

    fn input(status: ScenarioStatus, checks: Vec<ScenarioCheckInput>) -> ScenarioObservationInput {
        ScenarioObservationInput {
            schema: SCENARIO_INPUT_SCHEMA.into(),
            preparation_id: format!("workspace-prepare:sha256:{}", "c".repeat(64)),
            after_workspace_id: format!("after-workspace:sha256:{}", "d".repeat(64)),
            after_workspace_authority_digest: format!("sha256:{}", "e".repeat(64)),
            scenario_id: "multi-service-smoke".into(),
            provider: provider(false),
            status,
            started_unix_ms: 10,
            finished_unix_ms: 20,
            checks,
        }
    }

    fn check(status: ScenarioCheckStatus) -> ScenarioCheckInput {
        ScenarioCheckInput {
            id: "readiness".into(),
            status,
            observation_digest: canonical::hash_bytes(b"private raw evidence"),
        }
    }

    #[test]
    fn runtime_observation_never_promotes_contract_or_ownership() {
        let passed = input(
            ScenarioStatus::Passed,
            vec![check(ScenarioCheckStatus::Passed)],
        );
        validate_input(&passed, b"private raw evidence").unwrap();
        let (runtime, satisfaction, obligations) = receipt_status(&passed);
        assert_eq!(runtime, WorkspaceEvidenceAuthority::ObservedRuntime);
        assert_eq!(satisfaction, "OBSERVED_PASSED");
        assert!(obligations.is_empty());
    }

    #[test]
    fn unavailable_provider_is_an_explicit_optional_or_required_obligation() {
        let optional = input(ScenarioStatus::Unavailable, Vec::new());
        validate_input(&optional, b"").unwrap();
        assert_eq!(receipt_status(&optional).1, "OPTIONAL_PROVIDER_UNAVAILABLE");
        let mut required = optional;
        required.provider.required = true;
        assert_eq!(receipt_status(&required).1, "REQUIRED_PROVIDER_UNAVAILABLE");
    }

    #[test]
    fn aggregate_status_requires_a_supporting_check() {
        let mismatch = input(
            ScenarioStatus::Passed,
            vec![check(ScenarioCheckStatus::Failed)],
        );
        assert_eq!(
            validate_input(&mismatch, b"private raw evidence")
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        let duplicate = input(
            ScenarioStatus::Passed,
            vec![
                check(ScenarioCheckStatus::Passed),
                check(ScenarioCheckStatus::Passed),
            ],
        );
        assert_eq!(
            validate_input(&duplicate, b"private raw evidence")
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn every_check_is_bound_to_the_exact_private_evidence_bytes() {
        let bound = input(
            ScenarioStatus::Passed,
            vec![check(ScenarioCheckStatus::Passed)],
        );
        validate_input(&bound, b"private raw evidence").unwrap();
        assert_eq!(
            validate_input(&bound, b"changed evidence")
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }
}
