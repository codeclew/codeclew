//! Idempotent preparation of independent repository candidates.
//!
//! Preparation is deliberately a barrier, not a distributed transaction. Each
//! member retains its own session, plan, run ledger, candidate commit, and
//! validation. The workspace projection is installed only after every member is
//! prepared, and this module never updates a Git ref.

use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use crate::session::{RunRecord, RunStatus};
use crate::state::StateAuthority;
use crate::task_run_v2::{
    PreparedCandidateV2, QualifiedObligation, ValidationEvidence, load_prepared_for_workspace,
    require_mutation_request,
};
use crate::workspace::{WorkspaceAuthority, WorkspaceEdgeAuthority, load_open_authority};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;

pub const WORKSPACE_PREPARE_INPUT_SCHEMA: &str = "codeclew-workspace-prepare-input/1.0";
pub const WORKSPACE_PREPARE_AUTHORITY_SCHEMA: &str = "codeclew-workspace-prepare-authority/1.0";
pub const WORKSPACE_PREPARE_PROGRESS_SCHEMA: &str = "codeclew-workspace-prepare-progress/1.0";
pub const AFTER_WORKSPACE_SCHEMA: &str = "codeclew-after-workspace/1.0";
pub const MAX_WORKSPACE_PREPARE_INPUT_BYTES: usize = 256 * 1024;
const MAX_WORKSPACE_PREPARE_STATE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePrepareInput {
    pub schema: String,
    pub members: Vec<WorkspacePrepareInputMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePrepareInputMember {
    pub alias: String,
    pub context_id: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePrepareMemberAuthority {
    pub alias: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub context_id: String,
    pub context_evidence_digest: String,
    pub plan_id: String,
    pub plan_source_digest: String,
    pub run_id: String,
    pub run_request_digest: String,
    pub before_oid: String,
    pub target_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePrepareAuthority {
    pub schema: String,
    pub preparation_id: String,
    pub authority_digest: String,
    pub workspace_id: String,
    pub workspace_authority_digest: String,
    pub workspace_semantic_digest: String,
    pub mission_identity_digest: String,
    pub change_spec_digest: String,
    pub members: Vec<WorkspacePrepareMemberAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePrepareMemberProgress {
    pub alias: String,
    pub run_id: String,
    pub status: RunStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePrepareProgress {
    pub schema: String,
    pub preparation_id: String,
    pub workspace_id: String,
    pub status: String,
    pub members: Vec<WorkspacePrepareMemberProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AfterWorkspaceMember {
    pub alias: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub context_id: String,
    pub context_evidence_digest: String,
    pub plan_id: String,
    pub plan_source_digest: String,
    pub run_id: String,
    pub run_request_digest: String,
    pub run_status: RunStatus,
    pub target_ref: String,
    pub before_oid: String,
    pub candidate_oid: String,
    pub candidate_snapshot: CasObject,
    pub prepared_authority_digest: String,
    pub validation_evidence: Vec<ValidationEvidence>,
    pub obligation_count: usize,
    pub obligation_tree_digest: String,
    pub publication_blocked: bool,
    pub conditional_publish_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AfterWorkspaceEdge {
    pub id: String,
    pub source: String,
    pub source_candidate_oid: String,
    pub target: String,
    pub target_candidate_oid: String,
    pub relation: String,
    pub certainty: crate::workspace::WorkspaceCertaintyAxes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AfterWorkspace {
    pub schema: String,
    pub after_workspace_id: String,
    pub authority_digest: String,
    pub preparation_id: String,
    pub preparation_authority_digest: String,
    pub workspace_id: String,
    pub workspace_authority_digest: String,
    pub workspace_semantic_digest: String,
    pub mission_identity_digest: String,
    pub change_spec_digest: String,
    pub status: String,
    pub members: Vec<AfterWorkspaceMember>,
    pub edges: Vec<AfterWorkspaceEdge>,
}

pub enum WorkspacePrepareObservation {
    Preparing(WorkspacePrepareProgress),
    Prepared(AfterWorkspace),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunPhase {
    Pending,
    Prepared,
    Invalid,
}

pub fn resolve(workspace_id: &str, source: &[u8]) -> Result<WorkspacePrepareAuthority, ClewError> {
    let input = parse_input(source)?;
    let (workspace, _) = load_open_authority(workspace_id)?;
    if input.members.len() != workspace.members.len() {
        return Err(invalid(
            "workspace prepare request must bind every member exactly once",
        ));
    }
    let mut requested = BTreeMap::new();
    for member in input.members {
        if requested.insert(member.alias.clone(), member).is_some() {
            return Err(invalid("workspace prepare member alias is duplicated"));
        }
    }
    let request_bindings = workspace
        .members
        .iter()
        .map(|member| {
            let requested = requested
                .get(&member.alias)
                .ok_or_else(|| invalid("workspace prepare member alias is missing or unknown"))?;
            Ok((
                member.alias.clone(),
                requested.context_id.clone(),
                requested.plan_id.clone(),
            ))
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    let preparation_id = preparation_id(
        &workspace.workspace_id,
        &workspace.authority_digest,
        &request_bindings,
    )?;
    if let Some(existing) = load_prepare_authority(
        &workspace.workspace_id,
        &preparation_id,
        &workspace,
        &request_bindings,
    )? {
        return Ok(existing);
    }
    let mut members = Vec::with_capacity(workspace.members.len());
    for member in &workspace.members {
        let requested = requested
            .remove(&member.alias)
            .ok_or_else(|| invalid("workspace prepare member alias is missing or unknown"))?;
        member.session.require_open()?;
        let context = member.session.load_context(&requested.context_id)?;
        let plan = member.session.load_plan(&requested.plan_id)?;
        require_mutation_request(&member.session, &context, &plan)?;
        let run = RunRecord::created(&member.session, &requested.context_id, &requested.plan_id)?;
        members.push(WorkspacePrepareMemberAuthority {
            alias: member.alias.clone(),
            session_id: member.session.session_id.clone(),
            session_authority_digest: member.session.authority_digest.clone(),
            context_id: context.context_id,
            context_evidence_digest: context.evidence_digest,
            plan_id: plan.plan_id,
            plan_source_digest: plan.source_digest,
            run_id: run.run_id,
            run_request_digest: run.request_digest,
            before_oid: member.session.target_oid.clone(),
            target_ref: member.session.target_ref.clone(),
        });
    }
    if !requested.is_empty() {
        return Err(invalid("workspace prepare member alias is unknown"));
    }
    members.sort_by(|left, right| left.alias.cmp(&right.alias));
    let mut authority = WorkspacePrepareAuthority {
        schema: WORKSPACE_PREPARE_AUTHORITY_SCHEMA.into(),
        preparation_id,
        authority_digest: String::new(),
        workspace_id: workspace.workspace_id,
        workspace_authority_digest: workspace.authority_digest,
        workspace_semantic_digest: workspace.semantic_digest,
        mission_identity_digest: workspace.mission_identity_digest,
        change_spec_digest: workspace.change_spec_digest,
        members,
    };
    authority.authority_digest = prepare_authority_digest(&authority)?;
    persist_prepare_authority(&authority)?;
    Ok(authority)
}

pub fn observe_and_finalize(
    authority: &WorkspacePrepareAuthority,
) -> Result<WorkspacePrepareObservation, ClewError> {
    validate_prepare_authority(authority)?;
    if let Some(existing) = load_after_workspace(authority)? {
        return Ok(WorkspacePrepareObservation::Prepared(existing));
    }

    let mut progress = Vec::with_capacity(authority.members.len());
    let mut prepared = Vec::with_capacity(authority.members.len());
    for member in &authority.members {
        let run = RunRecord::load(&member.run_id).map_err(|error| {
            ClewError::new(
                ErrorCode::PreconditionFailed,
                format!(
                    "workspace member {} has no attached task run: {error}",
                    member.alias
                ),
            )
            .with_relevant(member.alias.clone())
        })?;
        require_run_binding(member, &run)?;
        progress.push(WorkspacePrepareMemberProgress {
            alias: member.alias.clone(),
            run_id: run.run_id.clone(),
            status: run.status,
        });
        match run_phase(run.status) {
            RunPhase::Pending => {}
            RunPhase::Prepared => {
                let candidate = load_prepared_for_workspace(&run)?;
                prepared.push((member, run, candidate));
            }
            RunPhase::Invalid => {
                return Err(ClewError::new(
                    ErrorCode::PreconditionFailed,
                    format!(
                        "workspace member {} cannot satisfy prepare-all from status {:?}",
                        member.alias, run.status
                    ),
                )
                .with_relevant(member.alias.clone()));
            }
        }
    }
    progress.sort_by(|left, right| left.alias.cmp(&right.alias));
    if prepared.len() != authority.members.len() {
        return Ok(WorkspacePrepareObservation::Preparing(
            WorkspacePrepareProgress {
                schema: WORKSPACE_PREPARE_PROGRESS_SCHEMA.into(),
                preparation_id: authority.preparation_id.clone(),
                workspace_id: authority.workspace_id.clone(),
                status: "PREPARING".into(),
                members: progress,
            },
        ));
    }
    let (workspace, _) = load_open_authority(&authority.workspace_id)?;
    require_workspace_binding(&workspace, authority)?;
    let after = build_after_workspace(&workspace, authority, prepared)?;
    persist_after_workspace(authority, &after)?;
    Ok(WorkspacePrepareObservation::Prepared(after))
}

pub fn retained_after(
    authority: &WorkspacePrepareAuthority,
) -> Result<Option<AfterWorkspace>, ClewError> {
    validate_prepare_authority(authority)?;
    load_after_workspace(authority)
}

fn parse_input(source: &[u8]) -> Result<WorkspacePrepareInput, ClewError> {
    if source.is_empty() || source.len() > MAX_WORKSPACE_PREPARE_INPUT_BYTES {
        return Err(invalid(
            "workspace prepare input is empty or exceeds 256 KiB",
        ));
    }
    let value: Value = serde_json::from_slice(source)
        .map_err(|_| invalid("workspace prepare input is not valid JSON"))?;
    if canonical::bytes(&value).map_err(internal)? != source {
        return Err(invalid(
            "workspace prepare input must be canonical compact JSON with NFC strings",
        ));
    }
    let input: WorkspacePrepareInput = serde_json::from_value(value)
        .map_err(|_| invalid("workspace prepare schema is invalid"))?;
    if input.schema != WORKSPACE_PREPARE_INPUT_SCHEMA || !(2..=4).contains(&input.members.len()) {
        return Err(invalid(
            "workspace prepare input must contain two to four members",
        ));
    }
    Ok(input)
}

fn build_after_workspace(
    workspace: &WorkspaceAuthority,
    authority: &WorkspacePrepareAuthority,
    prepared: Vec<(
        &WorkspacePrepareMemberAuthority,
        RunRecord,
        PreparedCandidateV2,
    )>,
) -> Result<AfterWorkspace, ClewError> {
    let mut members = prepared
        .into_iter()
        .map(
            |(binding, run, candidate)| -> Result<AfterWorkspaceMember, ClewError> {
                let (obligation_count, obligation_tree_digest) =
                    obligation_binding(&candidate.qualified_obligations)?;
                Ok(AfterWorkspaceMember {
                    alias: binding.alias.clone(),
                    session_id: binding.session_id.clone(),
                    session_authority_digest: binding.session_authority_digest.clone(),
                    context_id: binding.context_id.clone(),
                    context_evidence_digest: binding.context_evidence_digest.clone(),
                    plan_id: binding.plan_id.clone(),
                    plan_source_digest: binding.plan_source_digest.clone(),
                    run_id: binding.run_id.clone(),
                    run_request_digest: binding.run_request_digest.clone(),
                    run_status: run.status,
                    target_ref: binding.target_ref.clone(),
                    before_oid: binding.before_oid.clone(),
                    candidate_oid: candidate.candidate_commit,
                    candidate_snapshot: candidate.candidate_snapshot,
                    prepared_authority_digest: candidate.prepared_authority_digest,
                    validation_evidence: candidate.validation_evidence,
                    obligation_count,
                    obligation_tree_digest,
                    publication_blocked: candidate.publication_blocked,
                    conditional_publish_eligible: candidate.conditional_publish_eligible,
                })
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    members.sort_by(|left, right| left.alias.cmp(&right.alias));
    let candidates = members
        .iter()
        .map(|member| (member.alias.as_str(), member.candidate_oid.as_str()))
        .collect::<BTreeMap<_, _>>();
    let edges = workspace
        .edges
        .iter()
        .map(|edge| after_edge(edge, &candidates))
        .collect::<Result<Vec<_>, _>>()?;
    let semantic = json!({
        "schema":"codeclew-after-workspace-semantic-authority/1.0",
        "preparationId":authority.preparation_id,
        "preparationAuthorityDigest":authority.authority_digest,
        "workspaceId":authority.workspace_id,
        "workspaceAuthorityDigest":authority.workspace_authority_digest,
        "workspaceSemanticDigest":authority.workspace_semantic_digest,
        "missionIdentityDigest":authority.mission_identity_digest,
        "changeSpecDigest":authority.change_spec_digest,
        "status":"PREPARED_ALL",
        "members":members,
        "edges":edges,
    });
    let semantic_digest = canonical::hash(&semantic).map_err(internal)?;
    let mut after = AfterWorkspace {
        schema: AFTER_WORKSPACE_SCHEMA.into(),
        after_workspace_id: format!("after-workspace:{semantic_digest}"),
        authority_digest: String::new(),
        preparation_id: authority.preparation_id.clone(),
        preparation_authority_digest: authority.authority_digest.clone(),
        workspace_id: authority.workspace_id.clone(),
        workspace_authority_digest: authority.workspace_authority_digest.clone(),
        workspace_semantic_digest: authority.workspace_semantic_digest.clone(),
        mission_identity_digest: authority.mission_identity_digest.clone(),
        change_spec_digest: authority.change_spec_digest.clone(),
        status: "PREPARED_ALL".into(),
        members,
        edges,
    };
    after.authority_digest = after_authority_digest(&after)?;
    if canonical::bytes(&after).map_err(internal)?.len() > MAX_WORKSPACE_PREPARE_STATE_BYTES {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "AfterWorkspace authority exceeds 1 MiB",
        ));
    }
    Ok(after)
}

fn after_edge(
    edge: &WorkspaceEdgeAuthority,
    candidates: &BTreeMap<&str, &str>,
) -> Result<AfterWorkspaceEdge, ClewError> {
    Ok(AfterWorkspaceEdge {
        id: edge.id.clone(),
        source: edge.source.clone(),
        source_candidate_oid: candidates
            .get(edge.source.as_str())
            .ok_or_else(|| internal("workspace edge source candidate is missing"))?
            .to_string(),
        target: edge.target.clone(),
        target_candidate_oid: candidates
            .get(edge.target.as_str())
            .ok_or_else(|| internal("workspace edge target candidate is missing"))?
            .to_string(),
        relation: edge.relation.clone(),
        certainty: edge.certainty.clone(),
    })
}

fn obligation_binding(obligations: &[QualifiedObligation]) -> Result<(usize, String), ClewError> {
    let mut projected = obligations
        .iter()
        .map(|obligation| {
            json!({
                "approvalId":obligation.approval_id,
                "source":obligation.source,
                "recordDigest":obligation.record_digest,
            })
        })
        .collect::<Vec<_>>();
    projected.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
    projected.dedup();
    Ok((
        projected.len(),
        canonical::hash(&projected).map_err(internal)?,
    ))
}

fn run_phase(status: RunStatus) -> RunPhase {
    match status {
        RunStatus::Created | RunStatus::Preparing => RunPhase::Pending,
        RunStatus::ReadyToPublish
        | RunStatus::ReadyToPublishConditional
        | RunStatus::ValidatedConditional => RunPhase::Prepared,
        RunStatus::Publishing
        | RunStatus::Published
        | RunStatus::PublishedConditional
        | RunStatus::Failed
        | RunStatus::WorktreeRecoveryRequired
        | RunStatus::Cancelled => RunPhase::Invalid,
    }
}

fn require_run_binding(
    member: &WorkspacePrepareMemberAuthority,
    run: &RunRecord,
) -> Result<(), ClewError> {
    if run.session_id != member.session_id
        || run.context_id != member.context_id
        || run.plan_id != member.plan_id
        || run.run_id != member.run_id
        || run.request_digest != member.run_request_digest
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "workspace member task run differs from prepare authority",
        )
        .with_relevant(member.alias.clone()));
    }
    Ok(())
}

fn require_workspace_binding(
    workspace: &WorkspaceAuthority,
    authority: &WorkspacePrepareAuthority,
) -> Result<(), ClewError> {
    if workspace.workspace_id != authority.workspace_id
        || workspace.authority_digest != authority.workspace_authority_digest
        || workspace.semantic_digest != authority.workspace_semantic_digest
        || workspace.mission_identity_digest != authority.mission_identity_digest
        || workspace.change_spec_digest != authority.change_spec_digest
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "workspace authority changed after prepare admission",
        ));
    }
    Ok(())
}

fn persist_prepare_authority(authority: &WorkspacePrepareAuthority) -> Result<(), ClewError> {
    let directory = preparation_directory(authority)?;
    let bytes = canonical::bytes(authority).map_err(internal)?;
    if bytes.len() > MAX_WORKSPACE_PREPARE_STATE_BYTES {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "workspace prepare authority exceeds 1 MiB",
        ));
    }
    if !directory.atomic_create(OsStr::new("authority.json"), &bytes)? {
        let existing = directory.read_file(
            OsStr::new("authority.json"),
            MAX_WORKSPACE_PREPARE_STATE_BYTES,
        )?;
        if existing != bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "workspace prepare authority already exists with different content",
            ));
        }
    }
    Ok(())
}

fn load_prepare_authority(
    workspace_id: &str,
    preparation_id: &str,
    workspace: &WorkspaceAuthority,
    request_bindings: &[(String, String, String)],
) -> Result<Option<WorkspacePrepareAuthority>, ClewError> {
    let directory = preparation_directory_at(workspace_id, preparation_id)?;
    if !directory.file_exists(OsStr::new("authority.json"))? {
        return Ok(None);
    }
    let bytes = directory.read_file(
        OsStr::new("authority.json"),
        MAX_WORKSPACE_PREPARE_STATE_BYTES,
    )?;
    let authority: WorkspacePrepareAuthority = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("workspace prepare authority is not valid JSON"))?;
    if canonical::bytes(&authority).map_err(internal)? != bytes {
        return Err(corrupt("workspace prepare authority is not canonical"));
    }
    validate_prepare_authority(&authority)?;
    require_workspace_binding(workspace, &authority)?;
    let stored_bindings = authority
        .members
        .iter()
        .map(|member| {
            (
                member.alias.clone(),
                member.context_id.clone(),
                member.plan_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    if authority.preparation_id != preparation_id || stored_bindings != request_bindings {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "workspace prepare request differs from its retained authority",
        ));
    }
    Ok(Some(authority))
}

fn persist_after_workspace(
    authority: &WorkspacePrepareAuthority,
    after: &AfterWorkspace,
) -> Result<(), ClewError> {
    validate_after_workspace(after, authority)?;
    let directory = preparation_directory(authority)?;
    let bytes = canonical::bytes(after).map_err(internal)?;
    if !directory.atomic_create(OsStr::new("after-workspace.json"), &bytes)? {
        let existing = directory.read_file(
            OsStr::new("after-workspace.json"),
            MAX_WORKSPACE_PREPARE_STATE_BYTES,
        )?;
        if existing != bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "AfterWorkspace already exists with different content",
            ));
        }
    }
    Ok(())
}

fn load_after_workspace(
    authority: &WorkspacePrepareAuthority,
) -> Result<Option<AfterWorkspace>, ClewError> {
    let directory = preparation_directory(authority)?;
    if !directory.file_exists(OsStr::new("after-workspace.json"))? {
        return Ok(None);
    }
    let bytes = directory.read_file(
        OsStr::new("after-workspace.json"),
        MAX_WORKSPACE_PREPARE_STATE_BYTES,
    )?;
    let after: AfterWorkspace =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("AfterWorkspace is not valid JSON"))?;
    if canonical::bytes(&after).map_err(internal)? != bytes {
        return Err(corrupt("AfterWorkspace is not canonical"));
    }
    validate_after_workspace(&after, authority)?;
    Ok(Some(after))
}

fn preparation_directory(
    authority: &WorkspacePrepareAuthority,
) -> Result<crate::state::ManagedDirectory, ClewError> {
    preparation_directory_at(&authority.workspace_id, &authority.preparation_id)
}

fn preparation_directory_at(
    workspace_id: &str,
    preparation_id: &str,
) -> Result<crate::state::ManagedDirectory, ClewError> {
    let state = StateAuthority::process_default()?;
    let component = preparation_id
        .strip_prefix("workspace-prepare:sha256:")
        .filter(|value| digest_component(value))
        .ok_or_else(|| invalid("workspace prepare id is invalid"))?;
    state.directory_at(
        &state
            .workspace_root(workspace_id)?
            .join("preparations")
            .join(component),
    )
}

fn validate_prepare_authority(authority: &WorkspacePrepareAuthority) -> Result<(), ClewError> {
    let mut aliases = BTreeSet::new();
    let mut previous_alias: Option<&str> = None;
    let bindings = authority
        .members
        .iter()
        .map(|member| {
            (
                member.alias.clone(),
                member.context_id.clone(),
                member.plan_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    if authority.schema != WORKSPACE_PREPARE_AUTHORITY_SCHEMA
        || authority.members.len() < 2
        || authority.members.len() > 4
        || authority.authority_digest != prepare_authority_digest(authority)?
        || authority.preparation_id
            != preparation_id(
                &authority.workspace_id,
                &authority.workspace_authority_digest,
                &bindings,
            )?
        || authority.members.iter().any(|member| {
            let out_of_order = previous_alias
                .replace(member.alias.as_str())
                .is_some_and(|previous| previous >= member.alias.as_str());
            out_of_order
                || !aliases.insert(member.alias.as_str())
                || !digest(&member.session_authority_digest)
                || !digest(&member.context_evidence_digest)
                || !digest(&member.plan_source_digest)
                || !digest(&member.run_request_digest)
                || !git_oid(&member.before_oid)
        })
    {
        return Err(corrupt("workspace prepare authority is invalid"));
    }
    Ok(())
}

fn preparation_id(
    workspace_id: &str,
    workspace_authority_digest: &str,
    bindings: &[(String, String, String)],
) -> Result<String, ClewError> {
    let digest = canonical::hash(&json!({
        "schema":"codeclew-workspace-prepare-request-authority/1.0",
        "workspaceId":workspace_id,
        "workspaceAuthorityDigest":workspace_authority_digest,
        "members":bindings.iter().map(|(alias, context_id, plan_id)| json!({
            "alias":alias,
            "contextId":context_id,
            "planId":plan_id,
        })).collect::<Vec<_>>(),
    }))
    .map_err(internal)?;
    Ok(format!("workspace-prepare:{digest}"))
}

fn validate_after_workspace(
    after: &AfterWorkspace,
    authority: &WorkspacePrepareAuthority,
) -> Result<(), ClewError> {
    if after.schema != AFTER_WORKSPACE_SCHEMA
        || after.status != "PREPARED_ALL"
        || after.preparation_id != authority.preparation_id
        || after.preparation_authority_digest != authority.authority_digest
        || after.workspace_id != authority.workspace_id
        || after.workspace_authority_digest != authority.workspace_authority_digest
        || after.members.len() != authority.members.len()
        || after.authority_digest != after_authority_digest(after)?
        || after.members.iter().any(|member| {
            !matches!(
                member.run_status,
                RunStatus::ReadyToPublish
                    | RunStatus::ReadyToPublishConditional
                    | RunStatus::ValidatedConditional
            ) || !git_oid(&member.before_oid)
                || !git_oid(&member.candidate_oid)
                || !digest(&member.prepared_authority_digest)
                || !digest(&member.obligation_tree_digest)
        })
    {
        return Err(corrupt("AfterWorkspace authority is invalid"));
    }
    Ok(())
}

fn prepare_authority_digest(authority: &WorkspacePrepareAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn after_authority_digest(after: &AfterWorkspace) -> Result<String, ClewError> {
    let mut unsigned = after.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
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

fn git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_run_state_has_one_prepare_all_classification() {
        for status in [RunStatus::Created, RunStatus::Preparing] {
            assert_eq!(run_phase(status), RunPhase::Pending);
        }
        for status in [
            RunStatus::ReadyToPublish,
            RunStatus::ReadyToPublishConditional,
            RunStatus::ValidatedConditional,
        ] {
            assert_eq!(run_phase(status), RunPhase::Prepared);
        }
        for status in [
            RunStatus::Publishing,
            RunStatus::Published,
            RunStatus::PublishedConditional,
            RunStatus::Failed,
            RunStatus::WorktreeRecoveryRequired,
            RunStatus::Cancelled,
        ] {
            assert_eq!(run_phase(status), RunPhase::Invalid);
        }
    }

    #[test]
    fn prepare_input_is_closed_canonical_and_member_bounded() {
        let valid = json!({
            "schema":WORKSPACE_PREPARE_INPUT_SCHEMA,
            "members":[
                {"alias":"api","contextId":"context:one","planId":"plan:one"},
                {"alias":"client","contextId":"context:two","planId":"plan:two"},
            ],
        });
        assert_eq!(
            parse_input(&canonical::bytes(&valid).unwrap())
                .unwrap()
                .members
                .len(),
            2
        );
        let mut noncanonical = canonical::bytes(&valid).unwrap();
        noncanonical.push(b'\n');
        assert_eq!(
            parse_input(&noncanonical).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        let open = json!({
            "schema":WORKSPACE_PREPARE_INPUT_SCHEMA,
            "members":[
                {"alias":"api","contextId":"context:one","planId":"plan:one","path":"private"},
                {"alias":"client","contextId":"context:two","planId":"plan:two"},
            ],
        });
        assert_eq!(
            parse_input(&canonical::bytes(&open).unwrap())
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn candidate_edges_bind_both_exact_revisions_without_promoting_certainty() {
        let edge = WorkspaceEdgeAuthority {
            id: "api-client".into(),
            source: "api".into(),
            target: "client".into(),
            relation: "depends-on".into(),
            certainty: crate::workspace::WorkspaceCertaintyAxes {
                topology: crate::workspace::WorkspaceEvidenceAuthority::DeclaredCatalog,
                compiler_shape: crate::workspace::WorkspaceEvidenceAuthority::Unknown,
                artifact_ownership: crate::workspace::WorkspaceEvidenceAuthority::Unknown,
                contract: crate::workspace::WorkspaceEvidenceAuthority::Unknown,
                runtime: crate::workspace::WorkspaceEvidenceAuthority::Unknown,
            },
        };
        let candidates = BTreeMap::from([("api", "a"), ("client", "b")]);
        let projected = after_edge(&edge, &candidates).unwrap();
        assert_eq!(projected.source_candidate_oid, "a");
        assert_eq!(projected.target_candidate_oid, "b");
        assert_eq!(
            projected.certainty.artifact_ownership,
            crate::workspace::WorkspaceEvidenceAuthority::Unknown
        );
    }
}
