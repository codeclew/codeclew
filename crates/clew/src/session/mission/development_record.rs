//! Evidence-native, immutable development records derived from mission bindings.

use super::{
    ChangeSpec, LoadedMission, MissionBinding, MissionLock, RunBinding, internal, invalid,
    load_with_state_unlocked, require_live_members, require_open,
};
use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::session::{ContextObject, PlanObject, RunRecord, RunStatus, SessionAuthority};
use crate::state::StateAuthority;
use crate::task_run_v2::{FileOperation, TaskPlanV2, validate_plan_value};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;

pub const INPUT_SCHEMA: &str = "codeclew-development-record-input/1.0";
pub const RECORD_SCHEMA: &str = "codeclew-development-record/1.0";
pub const DOSSIER_SCHEMA: &str = "codeclew-development-dossier/1.0";
pub const RESULT_SCHEMA: &str = "codeclew-development-record-result/1.0";
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLAIMS: usize = 1024;
const MAX_LINKS_PER_CLAIM: usize = 4096;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_STDOUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClaimCertainty {
    Exact,
    Observed,
    Declared,
    Conditional,
    Unsure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceInput {
    pub session_id: String,
    /// RFC 6901 pointer into the immutable context evidence object.
    pub pointer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationInput {
    pub session_id: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimInput {
    pub id: String,
    pub text: String,
    pub certainty: ClaimCertainty,
    #[serde(default)]
    pub requirement_ids: Vec<String>,
    #[serde(default)]
    pub acceptance_criterion_ids: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<EvidenceInput>,
    #[serde(default)]
    pub operations: Vec<OperationInput>,
    #[serde(default)]
    pub validation_session_ids: Vec<String>,
    #[serde(default)]
    pub documentation: Vec<OperationInput>,
    #[serde(default)]
    pub obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentRecordInput {
    pub schema: String,
    pub claims: Vec<ClaimInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceLink {
    pub evidence_id: String,
    pub session_id: String,
    pub context_id: String,
    pub context_authority_digest: String,
    pub context_evidence_digest: String,
    pub pointer: String,
    pub value_digest: String,
    pub exact_source: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationLink {
    pub node_id: String,
    pub session_id: String,
    pub plan_id: String,
    pub plan_authority_digest: String,
    pub operation_id: String,
    pub file_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationLink {
    pub node_id: String,
    pub session_id: String,
    pub run_id: String,
    pub run_authority_digest: String,
    pub validation_digest: String,
    pub status: RunStatus,
    pub successful: bool,
    pub conditional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentClaim {
    pub claim_id: String,
    pub local_id: String,
    pub text: String,
    pub certainty: ClaimCertainty,
    pub requirement_ids: Vec<String>,
    pub acceptance_criterion_ids: Vec<String>,
    pub evidence: Vec<EvidenceLink>,
    pub operations: Vec<OperationLink>,
    pub validations: Vec<ValidationLink>,
    pub documentation: Vec<OperationLink>,
    pub obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnresolvedObligation {
    pub obligation_id: String,
    pub code: String,
    pub subject_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentRecord {
    pub schema: String,
    pub record_id: String,
    pub mission_id: String,
    pub mission_identity_digest: String,
    pub change_spec_digest: String,
    pub input_digest: String,
    pub claims: Vec<DevelopmentClaim>,
    pub unresolved_obligations: Vec<UnresolvedObligation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RenderFormat {
    Json,
    Markdown,
    Dot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DossierEvidence {
    pub kind: String,
    pub authority_digest: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DossierNode {
    pub node_id: String,
    pub kind: String,
    pub label: String,
    pub certainty: ClaimCertainty,
    pub current: bool,
    pub evidence: Vec<DossierEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DossierEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Dossier {
    pub schema: String,
    pub record_id: String,
    pub readiness: String,
    pub selected_node: Option<String>,
    pub nodes: Vec<DossierNode>,
    pub edges: Vec<DossierEdge>,
    pub unresolved_obligations: Vec<UnresolvedObligation>,
}

struct ResolvedContext {
    object: ContextObject,
    authority_digest: String,
}

struct ResolvedPlan {
    object: PlanObject,
    authority_digest: String,
    operations: BTreeMap<String, String>,
}

struct ResolvedRun {
    binding: RunBinding,
    object: RunRecord,
    authority_digest: String,
}

struct ResolvedMember {
    context: Option<ResolvedContext>,
    plan: Option<ResolvedPlan>,
    run: Option<ResolvedRun>,
}

pub fn parse_input(source: &[u8]) -> Result<DevelopmentRecordInput, ClewError> {
    if source.is_empty() || source.len() > MAX_INPUT_BYTES {
        return Err(invalid(
            "development record input is empty or exceeds 1 MiB",
        ));
    }
    let document: DevelopmentRecordInput = serde_json::from_slice(source)
        .map_err(|_| invalid("development record input is not closed JSON"))?;
    if canonical::bytes(&document).map_err(internal)? != source {
        return Err(invalid(
            "development record input must use canonical JSON bytes",
        ));
    }
    validate_input(&document)?;
    Ok(document)
}

pub fn create(mission_id: &str, source: &[u8]) -> Result<Value, ClewError> {
    let document = parse_input(source)?;
    let state = StateAuthority::process_default()?;
    let root = state.mission_root(mission_id)?;
    let _lock = MissionLock::acquire(&state, &root)?;
    let loaded = load_with_state_unlocked(&state, mission_id)?;
    require_open(&loaded)?;
    require_live_members(&loaded.identity.members)?;
    let record = build_record(&loaded, &document, source)?;
    let records = state.directory_at(&root.join("records"))?;
    let name = record_filename(&record.record_id)?;
    let bytes = canonical::bytes(&record).map_err(internal)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(invalid("development record exceeds 8 MiB"));
    }
    if records.file_exists(OsStr::new(&name))? {
        if records.read_file(OsStr::new(&name), MAX_RECORD_BYTES)? != bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "development record identifier is already bound to different bytes",
            ));
        }
    } else {
        let mut file = records.create_file(OsStr::new(&name))?;
        file.write_all(&bytes).map_err(super::super::io_error)?;
        file.sync_all().map_err(super::super::io_error)?;
    }
    bounded(&json!({
        "schema":RESULT_SCHEMA,
        "missionId":record.mission_id,
        "recordId":record.record_id,
        "claimCount":record.claims.len(),
        "unresolvedObligationCount":record.unresolved_obligations.len(),
        "readiness":base_readiness(&record),
    }))
}

pub fn render(
    mission_id: &str,
    record_id: &str,
    format: RenderFormat,
    selected_node: Option<&str>,
) -> Result<Value, ClewError> {
    let state = StateAuthority::process_default()?;
    let root = state.mission_root(mission_id)?;
    let _lock = MissionLock::acquire(&state, &root)?;
    let loaded = load_with_state_unlocked(&state, mission_id)?;
    let record = load_record(&state, &root, mission_id, record_id)?;
    if loaded.identity.identity_digest != record.mission_identity_digest
        || loaded.identity.change_spec_digest != record.change_spec_digest
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "development record mission authority changed",
        ));
    }
    let dossier = build_dossier(&loaded.spec, &record, selected_node)?;
    match format {
        RenderFormat::Json => bounded(&serde_json::to_value(dossier).map_err(internal)?),
        RenderFormat::Markdown => bounded(&json!({
            "schema":"codeclew-development-dossier-render/1.0",
            "recordId":record.record_id,
            "format":"MARKDOWN",
            "content":render_markdown(&dossier),
        })),
        RenderFormat::Dot => bounded(&json!({
            "schema":"codeclew-development-dossier-render/1.0",
            "recordId":record.record_id,
            "format":"DOT",
            "content":render_dot(&dossier),
        })),
    }
}

fn build_record(
    loaded: &LoadedMission,
    document: &DevelopmentRecordInput,
    source: &[u8],
) -> Result<DevelopmentRecord, ClewError> {
    let latest = latest_bindings(loaded);
    let resolved = resolve_members(&latest)?;
    let requirement_ids = item_ids(&loaded.spec.requirements);
    let acceptance_ids = item_ids(&loaded.spec.acceptance_criteria);
    let mut claims = Vec::with_capacity(document.claims.len());
    let mut planned_files = BTreeSet::new();
    for (session_id, member) in &resolved {
        if let Some(plan) = &member.plan {
            for file in plan.operations.values() {
                planned_files.insert((session_id.clone(), file.clone()));
            }
        }
    }
    let mut covered_files = BTreeSet::new();
    for input in &document.claims {
        require_subset(&input.requirement_ids, &requirement_ids, "requirement")?;
        require_subset(
            &input.acceptance_criterion_ids,
            &acceptance_ids,
            "acceptance criterion",
        )?;
        let evidence = input
            .evidence
            .iter()
            .map(|item| resolve_evidence(&resolved, item))
            .collect::<Result<Vec<_>, _>>()?;
        let operations = input
            .operations
            .iter()
            .map(|item| resolve_operation(&resolved, item, "operation"))
            .collect::<Result<Vec<_>, _>>()?;
        let documentation = input
            .documentation
            .iter()
            .map(|item| resolve_operation(&resolved, item, "documentation"))
            .collect::<Result<Vec<_>, _>>()?;
        let validations = input
            .validation_session_ids
            .iter()
            .map(|session| resolve_validation(&resolved, session))
            .collect::<Result<Vec<_>, _>>()?;
        covered_files.extend(
            operations
                .iter()
                .chain(&documentation)
                .map(|link| (link.session_id.clone(), link.file_id.clone())),
        );
        validate_certainty(input, &evidence, &validations)?;
        let claim_id = format!(
            "claim:{}",
            canonical::hash(&json!({
                "missionIdentityDigest":loaded.identity.identity_digest,
                "input":input,
                "evidence":evidence,
                "operations":operations,
                "validations":validations,
                "documentation":documentation,
            }))
            .map_err(internal)?
        );
        claims.push(DevelopmentClaim {
            claim_id,
            local_id: input.id.clone(),
            text: input.text.clone(),
            certainty: input.certainty,
            requirement_ids: sorted(input.requirement_ids.clone()),
            acceptance_criterion_ids: sorted(input.acceptance_criterion_ids.clone()),
            evidence,
            operations,
            validations,
            documentation,
            obligations: sorted(input.obligations.clone()),
        });
    }
    claims.sort_by(|left, right| left.claim_id.cmp(&right.claim_id));
    let unresolved_obligations =
        coverage_obligations(&loaded.spec, &claims, &planned_files, &covered_files);
    let mut record = DevelopmentRecord {
        schema: RECORD_SCHEMA.into(),
        record_id: String::new(),
        mission_id: loaded.identity.mission_id.clone(),
        mission_identity_digest: loaded.identity.identity_digest.clone(),
        change_spec_digest: loaded.identity.change_spec_digest.clone(),
        input_digest: canonical::hash_bytes(source),
        claims,
        unresolved_obligations,
    };
    record.record_id = format!(
        "development-record:{}",
        canonical::hash(&record).map_err(internal)?
    );
    Ok(record)
}

fn resolve_members(
    latest: &BTreeMap<String, MissionBinding>,
) -> Result<BTreeMap<String, ResolvedMember>, ClewError> {
    let mut resolved = BTreeMap::new();
    for (session_id, binding) in latest {
        // Records derive from already-bound immutable authorities. Runtime
        // upgrades must not make that historical evidence unreadable.
        let (session, _) = SessionAuthority::load_for_cleanup(session_id)?;
        let context = binding
            .context
            .as_ref()
            .map(|bound| {
                let object = session.load_context(&bound.context_id)?;
                let authority_digest = canonical::hash(&object).map_err(internal)?;
                if authority_digest != bound.authority_digest
                    || object.evidence_digest != bound.evidence_digest
                {
                    return Err(ClewError::new(
                        ErrorCode::BindingChanged,
                        "development context authority changed",
                    ));
                }
                Ok(ResolvedContext {
                    object,
                    authority_digest,
                })
            })
            .transpose()?;
        let plan = binding
            .plan
            .as_ref()
            .map(|bound| {
                let context = context
                    .as_ref()
                    .ok_or_else(|| invalid("development plan has no resolved context authority"))?;
                let object = session.load_plan_for_context(&bound.plan_id, &context.object)?;
                let authority_digest = canonical::hash(&object).map_err(internal)?;
                if authority_digest != bound.authority_digest {
                    return Err(ClewError::new(
                        ErrorCode::BindingChanged,
                        "development plan authority changed",
                    ));
                }
                let operations = plan_operations(&object.plan)?;
                Ok(ResolvedPlan {
                    object,
                    authority_digest,
                    operations,
                })
            })
            .transpose()?;
        let run = binding
            .run
            .as_ref()
            .map(|bound| {
                let object = RunRecord::load(&bound.run_id)?;
                let authority_digest = canonical::hash(&object).map_err(internal)?;
                if authority_digest != bound.authority_digest || object.status != bound.status {
                    return Err(ClewError::new(
                        ErrorCode::BindingChanged,
                        "development run authority changed",
                    ));
                }
                Ok(ResolvedRun {
                    binding: bound.clone(),
                    object,
                    authority_digest,
                })
            })
            .transpose()?;
        resolved.insert(session_id.clone(), ResolvedMember { context, plan, run });
    }
    Ok(resolved)
}

fn resolve_evidence(
    resolved: &BTreeMap<String, ResolvedMember>,
    input: &EvidenceInput,
) -> Result<EvidenceLink, ClewError> {
    if !input.pointer.starts_with('/')
        || input.pointer.len() > 4096
        || contains_absolute_path(&input.pointer)
    {
        return Err(invalid("development evidence pointer is unsafe"));
    }
    let member = resolved
        .get(&input.session_id)
        .ok_or_else(|| invalid("development evidence session has no mission binding"))?;
    let context = member
        .context
        .as_ref()
        .ok_or_else(|| invalid("development evidence session has no bound context"))?;
    let value = context
        .object
        .evidence
        .pointer(&input.pointer)
        .ok_or_else(|| invalid("development evidence pointer does not resolve"))?;
    let value_digest = canonical::hash(value).map_err(internal)?;
    let exact_source = input.pointer.starts_with("/context/matches/")
        && context
            .object
            .evidence
            .pointer("/context/completeness/certainty")
            .and_then(Value::as_str)
            == Some("VERIFIED");
    let evidence_id = format!(
        "evidence:{}",
        canonical::hash(&json!({
            "contextAuthorityDigest":context.authority_digest,
            "pointer":input.pointer,
            "valueDigest":value_digest,
        }))
        .map_err(internal)?
    );
    Ok(EvidenceLink {
        evidence_id,
        session_id: input.session_id.clone(),
        context_id: context.object.context_id.clone(),
        context_authority_digest: context.authority_digest.clone(),
        context_evidence_digest: context.object.evidence_digest.clone(),
        pointer: input.pointer.clone(),
        value_digest,
        exact_source,
    })
}

fn resolve_operation(
    resolved: &BTreeMap<String, ResolvedMember>,
    input: &OperationInput,
    role: &str,
) -> Result<OperationLink, ClewError> {
    let member = resolved
        .get(&input.session_id)
        .ok_or_else(|| invalid("development operation session has no mission binding"))?;
    let plan = member
        .plan
        .as_ref()
        .ok_or_else(|| invalid("development operation session has no bound plan"))?;
    let file_id = plan
        .operations
        .get(&input.operation_id)
        .ok_or_else(|| invalid("development operation ID is not in the bound plan"))?
        .clone();
    let node_id = format!(
        "{role}:{}",
        canonical::hash(&json!({
            "role":role,
            "planAuthorityDigest":plan.authority_digest,
            "operationId":input.operation_id,
        }))
        .map_err(internal)?
    );
    Ok(OperationLink {
        node_id,
        session_id: input.session_id.clone(),
        plan_id: plan.object.plan_id.clone(),
        plan_authority_digest: plan.authority_digest.clone(),
        operation_id: input.operation_id.clone(),
        file_id,
    })
}

fn resolve_validation(
    resolved: &BTreeMap<String, ResolvedMember>,
    session_id: &str,
) -> Result<ValidationLink, ClewError> {
    let member = resolved
        .get(session_id)
        .ok_or_else(|| invalid("development validation session has no mission binding"))?;
    let run = member
        .run
        .as_ref()
        .ok_or_else(|| invalid("development validation session has no bound run"))?;
    let successful = matches!(
        run.object.status,
        RunStatus::ReadyToPublish
            | RunStatus::ReadyToPublishConditional
            | RunStatus::ValidatedConditional
            | RunStatus::Published
            | RunStatus::PublishedConditional
    );
    let conditional = matches!(
        run.object.status,
        RunStatus::ReadyToPublishConditional
            | RunStatus::ValidatedConditional
            | RunStatus::PublishedConditional
    );
    let node_id = format!(
        "validation:{}",
        canonical::hash(&json!({
            "runAuthorityDigest":run.authority_digest,
            "validationDigest":run.binding.validation_digest,
        }))
        .map_err(internal)?
    );
    Ok(ValidationLink {
        node_id,
        session_id: session_id.into(),
        run_id: run.object.run_id.clone(),
        run_authority_digest: run.authority_digest.clone(),
        validation_digest: run.binding.validation_digest.clone(),
        status: run.object.status,
        successful,
        conditional,
    })
}

fn validate_certainty(
    input: &ClaimInput,
    evidence: &[EvidenceLink],
    validations: &[ValidationLink],
) -> Result<(), ClewError> {
    match input.certainty {
        ClaimCertainty::Exact
            if evidence.is_empty()
                || evidence.iter().any(|item| !item.exact_source)
                || !input.obligations.is_empty() =>
        {
            Err(invalid(
                "EXACT claim requires only verified compiler fact evidence and no obligations",
            ))
        }
        ClaimCertainty::Observed
            if validations.is_empty()
                || validations
                    .iter()
                    .any(|item| !item.successful || item.conditional) =>
        {
            Err(invalid(
                "OBSERVED claim requires successful non-conditional validation",
            ))
        }
        ClaimCertainty::Declared if input.requirement_ids.is_empty() => Err(invalid(
            "DECLARED claim requires at least one ChangeSpec requirement",
        )),
        ClaimCertainty::Conditional | ClaimCertainty::Unsure if input.obligations.is_empty() => {
            Err(invalid(
                "CONDITIONAL and UNSURE claims require an explicit obligation",
            ))
        }
        _ => Ok(()),
    }
}

fn coverage_obligations(
    spec: &ChangeSpec,
    claims: &[DevelopmentClaim],
    planned_files: &BTreeSet<(String, String)>,
    covered_files: &BTreeSet<(String, String)>,
) -> Vec<UnresolvedObligation> {
    let linked_requirements = claims
        .iter()
        .flat_map(|claim| claim.requirement_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let linked_acceptance = claims
        .iter()
        .filter(|claim| !claim.evidence.is_empty() || !claim.validations.is_empty())
        .flat_map(|claim| claim.acceptance_criterion_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let documented = claims
        .iter()
        .filter(|claim| !claim.documentation.is_empty())
        .flat_map(|claim| claim.requirement_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut obligations = Vec::new();
    for item in &spec.requirements {
        if !linked_requirements.contains(&item.id) {
            obligations.push(obligation("LINK_REQUIREMENT", &item.id));
        }
    }
    for item in &spec.acceptance_criteria {
        if !linked_acceptance.contains(&item.id) {
            obligations.push(obligation("LINK_ACCEPTANCE_VALIDATION", &item.id));
        }
    }
    for requirement in &spec.docs_policy.required_requirement_ids {
        if !documented.contains(requirement) {
            obligations.push(obligation("LINK_CANONICAL_DOCUMENTATION", requirement));
        }
    }
    for (session, file) in planned_files.difference(covered_files) {
        obligations.push(obligation(
            "LINK_CHANGED_FILE",
            &format!("{}:{}", short_digest(session), file),
        ));
    }
    obligations.sort();
    obligations
}

fn obligation(code: &str, subject: &str) -> UnresolvedObligation {
    let obligation_id = format!(
        "obligation:{}",
        canonical::hash(&json!({"code":code,"subjectId":subject})).unwrap_or_default()
    );
    UnresolvedObligation {
        obligation_id,
        code: code.into(),
        subject_id: subject.into(),
    }
}

fn build_dossier(
    spec: &ChangeSpec,
    record: &DevelopmentRecord,
    selected_node: Option<&str>,
) -> Result<Dossier, ClewError> {
    let mut nodes = BTreeMap::<String, DossierNode>::new();
    let mut edges = BTreeSet::<(String, String, String)>::new();
    for item in &spec.requirements {
        nodes.insert(
            requirement_node(&item.id),
            declared_node(
                requirement_node(&item.id),
                "REQUIREMENT",
                item.text.clone(),
                &record.change_spec_digest,
                &item.id,
            ),
        );
    }
    for item in &spec.acceptance_criteria {
        nodes.insert(
            acceptance_node(&item.id),
            declared_node(
                acceptance_node(&item.id),
                "ACCEPTANCE_CRITERION",
                item.text.clone(),
                &record.change_spec_digest,
                &item.id,
            ),
        );
    }
    let mut any_stale = false;
    for claim in &record.claims {
        let current = claim_current(claim);
        any_stale |= !current;
        nodes.insert(
            claim.claim_id.clone(),
            DossierNode {
                node_id: claim.claim_id.clone(),
                kind: "CLAIM".into(),
                label: claim.text.clone(),
                certainty: if current {
                    claim.certainty
                } else {
                    ClaimCertainty::Conditional
                },
                current,
                evidence: claim
                    .evidence
                    .iter()
                    .map(|item| DossierEvidence {
                        kind: "CONTEXT_EVIDENCE".into(),
                        authority_digest: item.value_digest.clone(),
                        reference: item.evidence_id.clone(),
                    })
                    .chain(claim.validations.iter().map(|item| DossierEvidence {
                        kind: "VALIDATION".into(),
                        authority_digest: item.validation_digest.clone(),
                        reference: item.node_id.clone(),
                    }))
                    .collect(),
            },
        );
        for requirement in &claim.requirement_ids {
            edges.insert((
                requirement_node(requirement),
                claim.claim_id.clone(),
                "SUPPORTED_BY".into(),
            ));
        }
        for acceptance in &claim.acceptance_criterion_ids {
            edges.insert((
                claim.claim_id.clone(),
                acceptance_node(acceptance),
                "SATISFIES".into(),
            ));
        }
        for operation in &claim.operations {
            insert_operation_node(&mut nodes, operation, "OPERATION");
            edges.insert((
                claim.claim_id.clone(),
                operation.node_id.clone(),
                "REALIZED_BY".into(),
            ));
        }
        for validation in &claim.validations {
            nodes
                .entry(validation.node_id.clone())
                .or_insert(DossierNode {
                    node_id: validation.node_id.clone(),
                    kind: "VALIDATION".into(),
                    label: format!("{:?}", validation.status),
                    certainty: if validation.successful && !validation.conditional {
                        ClaimCertainty::Observed
                    } else {
                        ClaimCertainty::Conditional
                    },
                    current,
                    evidence: vec![DossierEvidence {
                        kind: "RUN_VALIDATION".into(),
                        authority_digest: validation.validation_digest.clone(),
                        reference: validation.run_id.clone(),
                    }],
                });
            edges.insert((
                claim.claim_id.clone(),
                validation.node_id.clone(),
                "VALIDATED_BY".into(),
            ));
        }
        for documentation in &claim.documentation {
            insert_operation_node(&mut nodes, documentation, "DOCUMENTATION");
            edges.insert((
                claim.claim_id.clone(),
                documentation.node_id.clone(),
                "DOCUMENTED_BY".into(),
            ));
        }
    }
    for obligation in &record.unresolved_obligations {
        nodes.insert(
            obligation.obligation_id.clone(),
            DossierNode {
                node_id: obligation.obligation_id.clone(),
                kind: "OBLIGATION".into(),
                label: format!("{}: {}", obligation.code, obligation.subject_id),
                certainty: ClaimCertainty::Unsure,
                current: true,
                evidence: vec![DossierEvidence {
                    kind: "RECORD_COVERAGE".into(),
                    authority_digest: record.record_id.clone(),
                    reference: obligation.subject_id.clone(),
                }],
            },
        );
    }
    if let Some(selected) = selected_node {
        let selected = nodes
            .remove(selected)
            .ok_or_else(|| invalid("selected dossier node does not exist"))?;
        nodes.clear();
        nodes.insert(selected.node_id.clone(), selected);
        edges.clear();
    }
    let conditional_claim = record.claims.iter().any(|claim| {
        matches!(
            claim.certainty,
            ClaimCertainty::Conditional | ClaimCertainty::Unsure
        )
    });
    Ok(Dossier {
        schema: DOSSIER_SCHEMA.into(),
        record_id: record.record_id.clone(),
        readiness: if !record.unresolved_obligations.is_empty() || conditional_claim || any_stale {
            "CONDITIONAL"
        } else {
            "READY"
        }
        .into(),
        selected_node: selected_node.map(str::to_owned),
        nodes: nodes.into_values().collect(),
        edges: edges
            .into_iter()
            .map(|(from, to, relation)| DossierEdge { from, to, relation })
            .collect(),
        unresolved_obligations: record.unresolved_obligations.clone(),
    })
}

fn claim_current(claim: &DevelopmentClaim) -> bool {
    claim
        .evidence
        .iter()
        .all(|link| evidence_current(link).unwrap_or(false))
        && claim
            .operations
            .iter()
            .chain(&claim.documentation)
            .all(|link| operation_current(link).unwrap_or(false))
        && claim
            .validations
            .iter()
            .all(|link| validation_current(link).unwrap_or(false))
}

fn evidence_current(link: &EvidenceLink) -> Result<bool, ClewError> {
    let (session, _) = SessionAuthority::load_for_cleanup(&link.session_id)?;
    session.context_binding_current(
        &link.context_id,
        &link.context_authority_digest,
        // The value digest is bound transitively by this exact evidence object
        // digest. Admission verified the pointer and value before publication.
        &link.context_evidence_digest,
    )
}

fn operation_current(link: &OperationLink) -> Result<bool, ClewError> {
    let (session, _) = SessionAuthority::load_for_cleanup(&link.session_id)?;
    session.plan_binding_current(&link.plan_id, &link.plan_authority_digest)
}

fn validation_current(link: &ValidationLink) -> Result<bool, ClewError> {
    let run = RunRecord::load(&link.run_id)?;
    Ok(canonical::hash(&run).map_err(internal)? == link.run_authority_digest)
}

fn insert_operation_node(
    nodes: &mut BTreeMap<String, DossierNode>,
    link: &OperationLink,
    kind: &str,
) {
    nodes.entry(link.node_id.clone()).or_insert(DossierNode {
        node_id: link.node_id.clone(),
        kind: kind.into(),
        label: format!("{} → {}", link.operation_id, link.file_id),
        certainty: ClaimCertainty::Declared,
        current: operation_current(link).unwrap_or(false),
        evidence: vec![DossierEvidence {
            kind: "PLAN_OPERATION".into(),
            authority_digest: link.plan_authority_digest.clone(),
            reference: link.operation_id.clone(),
        }],
    });
}

fn declared_node(
    node_id: String,
    kind: &str,
    label: String,
    authority: &str,
    reference: &str,
) -> DossierNode {
    DossierNode {
        node_id,
        kind: kind.into(),
        label,
        certainty: ClaimCertainty::Declared,
        current: true,
        evidence: vec![DossierEvidence {
            kind: "CHANGE_SPEC".into(),
            authority_digest: authority.into(),
            reference: reference.into(),
        }],
    }
}

fn load_record(
    state: &StateAuthority,
    root: &Path,
    mission_id: &str,
    record_id: &str,
) -> Result<DevelopmentRecord, ClewError> {
    let name = record_filename(record_id)?;
    let record: DevelopmentRecord =
        super::super::read_managed_json(state, &root.join("records").join(name), MAX_RECORD_BYTES)?;
    let mut unsigned = record.clone();
    unsigned.record_id.clear();
    if record.schema != RECORD_SCHEMA
        || record.mission_id != mission_id
        || record.record_id != record_id
        || record.record_id
            != format!(
                "development-record:{}",
                canonical::hash(&unsigned).map_err(internal)?
            )
    {
        return Err(invalid("development record authority is invalid"));
    }
    Ok(record)
}

fn plan_operations(value: &Value) -> Result<BTreeMap<String, String>, ClewError> {
    let plan: TaskPlanV2 = validate_plan_value(value)?;
    Ok(plan
        .operations
        .into_iter()
        .map(|operation| match operation {
            FileOperation::ReplaceText { op_id, target, .. }
            | FileOperation::DeleteFile { op_id, target } => (op_id, target.file_id),
            FileOperation::CreateFile { op_id, target, .. } => (op_id, target.file_id),
        })
        .collect())
}

fn latest_bindings(loaded: &LoadedMission) -> BTreeMap<String, MissionBinding> {
    let mut latest = BTreeMap::new();
    for event in &loaded.events {
        if let Some(binding) = &event.binding {
            latest.insert(binding.session_id.clone(), binding.clone());
        }
    }
    latest
}

fn validate_input(document: &DevelopmentRecordInput) -> Result<(), ClewError> {
    if document.schema != INPUT_SCHEMA
        || document.claims.is_empty()
        || document.claims.len() > MAX_CLAIMS
    {
        return Err(invalid(
            "development record input must contain between 1 and 1024 claims",
        ));
    }
    let mut ids = BTreeSet::new();
    for claim in &document.claims {
        if !stable_id(&claim.id) || !ids.insert(&claim.id) {
            return Err(invalid("development claim IDs are unsafe or duplicated"));
        }
        validate_text(&claim.text)?;
        for obligation in &claim.obligations {
            validate_text(obligation)?;
        }
        for values in [
            &claim.requirement_ids,
            &claim.acceptance_criterion_ids,
            &claim.validation_session_ids,
            &claim.obligations,
        ] {
            if values.len() > MAX_LINKS_PER_CLAIM
                || values.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid(
                    "development claim references must be sorted, unique, and bounded",
                ));
            }
        }
        for values in [&claim.operations, &claim.documentation] {
            if values.len() > MAX_LINKS_PER_CLAIM
                || values.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid(
                    "development operation references must be sorted, unique, and bounded",
                ));
            }
        }
        if claim.evidence.len() > MAX_LINKS_PER_CLAIM {
            return Err(invalid("development evidence reference count is too large"));
        }
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_TEXT_BYTES
        || value.contains('\0')
        || !crate::text_authority::is_nfc(value)
        || contains_absolute_path(value)
    {
        return Err(invalid(
            "development record text must be trimmed, path-free NFC text no larger than 16 KiB",
        ));
    }
    Ok(())
}

fn contains_absolute_path(value: &str) -> bool {
    let components = value.split('/').collect::<Vec<_>>();
    components.windows(3).any(|window| {
        window[0].is_empty()
            && matches!(window[1], "Users" | "home" | "private")
            && !window[2].is_empty()
    }) || value
        .as_bytes()
        .windows(3)
        .any(|window| window[0].is_ascii_alphabetic() && window[1] == b':' && window[2] == b'\\')
}

fn stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn require_subset(
    values: &[String],
    allowed: &BTreeSet<String>,
    label: &str,
) -> Result<(), ClewError> {
    if values.iter().any(|value| !allowed.contains(value)) {
        return Err(invalid(&format!(
            "development claim references a foreign {label}"
        )));
    }
    Ok(())
}

fn item_ids(items: &[super::ChangeSpecItem]) -> BTreeSet<String> {
    items.iter().map(|item| item.id.clone()).collect()
}

fn sorted<T: Ord>(mut values: Vec<T>) -> Vec<T> {
    values.sort();
    values
}

fn requirement_node(id: &str) -> String {
    format!("requirement:{id}")
}

fn acceptance_node(id: &str) -> String {
    format!("acceptance:{id}")
}

fn record_filename(record_id: &str) -> Result<String, ClewError> {
    let digest = record_id
        .strip_prefix("development-record:sha256:")
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("development record ID is invalid"))?;
    Ok(format!("{digest}.json"))
}

fn short_digest(value: &str) -> &str {
    value
        .rsplit(':')
        .next()
        .unwrap_or(value)
        .get(..12)
        .unwrap_or(value)
}

fn base_readiness(record: &DevelopmentRecord) -> &'static str {
    if record.unresolved_obligations.is_empty()
        && record.claims.iter().all(|claim| {
            !matches!(
                claim.certainty,
                ClaimCertainty::Conditional | ClaimCertainty::Unsure
            )
        })
    {
        "READY"
    } else {
        "CONDITIONAL"
    }
}

fn bounded(value: &Value) -> Result<Value, ClewError> {
    if canonical::bytes(value).map_err(internal)?.len() > MAX_STDOUT_BYTES {
        return Err(invalid("development record output exceeds 1 MiB"));
    }
    Ok(value.clone())
}

fn render_markdown(dossier: &Dossier) -> String {
    let mut output = format!(
        "# Development dossier\n\nReadiness: `{}`\n",
        dossier.readiness
    );
    for node in &dossier.nodes {
        output.push_str(&format!(
            "\n## {}\n\n{}\n\nCertainty: `{:?}`; current: `{}`.\n",
            markdown(&node.node_id),
            markdown(&node.label),
            node.certainty,
            node.current
        ));
        for evidence in &node.evidence {
            output.push_str(&format!(
                "\n- `{}`: `{}` (`{}`)\n",
                markdown(&evidence.kind),
                markdown(&evidence.reference),
                markdown(&evidence.authority_digest)
            ));
        }
    }
    output
}

fn render_dot(dossier: &Dossier) -> String {
    let mut output = String::from("digraph development_record {\n  rankdir=LR;\n");
    for node in &dossier.nodes {
        output.push_str(&format!(
            "  \"{}\" [label=\"{}\\n{:?}\"];\n",
            dot(&node.node_id),
            dot(&node.label),
            node.certainty
        ));
    }
    for edge in &dossier.edges {
        output.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{}\"];\n",
            dot(&edge.from),
            dot(&edge.to),
            dot(&edge.relation)
        ));
    }
    output.push_str("}\n");
    output
}

fn markdown(value: &str) -> String {
    value.replace('`', "\\`")
}

fn dot(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> DevelopmentRecord {
        DevelopmentRecord {
            schema: RECORD_SCHEMA.into(),
            record_id: "development-record:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            mission_id: "mission:00000000-0000-0000-0000-000000000000".into(),
            mission_identity_digest: format!("sha256:{:0<64}", "identity"),
            change_spec_digest: format!("sha256:{:0<64}", "spec"),
            input_digest: format!("sha256:{:0<64}", "input"),
            claims: vec![DevelopmentClaim {
                claim_id: "claim:one".into(),
                local_id: "C1".into(),
                text: "The behavior is preserved".into(),
                certainty: ClaimCertainty::Unsure,
                requirement_ids: vec!["R1".into()],
                acceptance_criterion_ids: vec!["A1".into()],
                evidence: Vec::new(),
                operations: Vec::new(),
                validations: Vec::new(),
                documentation: Vec::new(),
                obligations: vec!["Run the scenario".into()],
            }],
            unresolved_obligations: vec![obligation("LINK_CHANGED_FILE", "source.kt")],
        }
    }

    fn sample_spec() -> ChangeSpec {
        ChangeSpec {
            schema: super::super::CHANGE_SPEC_SCHEMA.into(),
            intent: "Preserve behavior".into(),
            requirements: vec![super::super::ChangeSpecItem {
                id: "R1".into(),
                text: "Keep behavior".into(),
            }],
            non_goals: Vec::new(),
            acceptance_criteria: vec![super::super::ChangeSpecItem {
                id: "A1".into(),
                text: "Scenario passes".into(),
            }],
            docs_policy: super::super::DocsPolicy {
                required_requirement_ids: Vec::new(),
            },
        }
    }

    #[test]
    fn selected_node_has_only_its_specific_evidence() {
        let dossier =
            build_dossier(&sample_spec(), &sample_record(), Some("requirement:R1")).unwrap();
        assert_eq!(dossier.nodes.len(), 1);
        assert_eq!(dossier.nodes[0].node_id, "requirement:R1");
        assert_eq!(dossier.nodes[0].evidence[0].kind, "CHANGE_SPEC");
        assert!(dossier.edges.is_empty());
    }

    #[test]
    fn markdown_and_dot_are_deterministic_and_path_free() {
        let dossier = build_dossier(&sample_spec(), &sample_record(), None).unwrap();
        assert_eq!(render_markdown(&dossier), render_markdown(&dossier));
        assert_eq!(render_dot(&dossier), render_dot(&dossier));
        assert!(!contains_absolute_path(&render_markdown(&dossier)));
        assert!(!contains_absolute_path(&render_dot(&dossier)));
    }

    #[test]
    fn input_rejects_absolute_paths() {
        let document = DevelopmentRecordInput {
            schema: INPUT_SCHEMA.into(),
            claims: vec![ClaimInput {
                id: "C1".into(),
                text: format!("Read {}{}", "C:", "\\private\\source"),
                certainty: ClaimCertainty::Unsure,
                requirement_ids: Vec::new(),
                acceptance_criterion_ids: Vec::new(),
                evidence: Vec::new(),
                operations: Vec::new(),
                validation_session_ids: Vec::new(),
                documentation: Vec::new(),
                obligations: vec!["Verify it".into()],
            }],
        };
        assert!(validate_input(&document).is_err());
    }

    #[test]
    fn coverage_names_each_missing_product_link() {
        let mut spec = sample_spec();
        spec.docs_policy.required_requirement_ids = vec!["R1".into()];
        let obligations = coverage_obligations(
            &spec,
            &[],
            &BTreeSet::from([("session:one".into(), "src/Main.kt".into())]),
            &BTreeSet::new(),
        );
        let codes = obligations
            .iter()
            .map(|item| item.code.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            codes,
            BTreeSet::from([
                "LINK_ACCEPTANCE_VALIDATION",
                "LINK_CANONICAL_DOCUMENTATION",
                "LINK_CHANGED_FILE",
                "LINK_REQUIREMENT",
            ])
        );
    }

    #[test]
    fn stale_evidence_downgrades_only_its_dependent_claim() {
        let mut record = sample_record();
        record.unresolved_obligations.clear();
        record.claims[0].certainty = ClaimCertainty::Exact;
        record.claims[0].evidence.push(EvidenceLink {
            evidence_id: "evidence:stale".into(),
            session_id: "session:missing".into(),
            context_id: "context:missing".into(),
            context_authority_digest: format!("sha256:{:0<64}", "context"),
            context_evidence_digest: format!("sha256:{:0<64}", "evidence"),
            pointer: "/context/matches/0".into(),
            value_digest: format!("sha256:{:0<64}", "value"),
            exact_source: true,
        });
        record.claims.push(DevelopmentClaim {
            claim_id: "claim:independent".into(),
            local_id: "C2".into(),
            text: "Independent declared fact".into(),
            certainty: ClaimCertainty::Declared,
            requirement_ids: vec!["R1".into()],
            acceptance_criterion_ids: Vec::new(),
            evidence: Vec::new(),
            operations: Vec::new(),
            validations: Vec::new(),
            documentation: Vec::new(),
            obligations: Vec::new(),
        });
        let dossier = build_dossier(&sample_spec(), &record, None).unwrap();
        let stale = dossier
            .nodes
            .iter()
            .find(|node| node.node_id == "claim:one")
            .unwrap();
        let independent = dossier
            .nodes
            .iter()
            .find(|node| node.node_id == "claim:independent")
            .unwrap();
        assert!(!stale.current);
        assert_eq!(stale.certainty, ClaimCertainty::Conditional);
        assert!(independent.current);
        assert_eq!(independent.certainty, ClaimCertainty::Declared);
        assert_eq!(dossier.readiness, "CONDITIONAL");
    }
}
