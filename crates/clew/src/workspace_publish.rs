//! Ordered local publication of an immutable prepared workspace.
//!
//! The publication authority seals member order and conditional approvals
//! before the first ref update. The append-only ledger never rolls a published
//! candidate back: a failed or interrupted suffix remains recoverable by
//! deterministic roll-forward.

use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::state::StateAuthority;
use crate::workspace_prepare::{
    AfterWorkspace, AfterWorkspaceMember, load_bound_after_workspace, load_retained_after_workspace,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const WORKSPACE_PUBLISH_INPUT_SCHEMA: &str = "codeclew-workspace-publish-input/1.0";
pub const WORKSPACE_PUBLICATION_AUTHORITY_SCHEMA: &str =
    "codeclew-workspace-publication-authority/1.0";
pub const WORKSPACE_PUBLICATION_EVENT_SCHEMA: &str = "codeclew-workspace-publication-event/1.0";
pub const MAX_WORKSPACE_PUBLISH_INPUT_BYTES: usize = 256 * 1024;
const MAX_WORKSPACE_PUBLICATION_AUTHORITY_BYTES: usize = 512 * 1024;
const MAX_WORKSPACE_PUBLICATION_LEDGER_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePublishInput {
    pub schema: String,
    pub preparation_id: String,
    pub after_workspace_id: String,
    pub after_workspace_authority_digest: String,
    pub policy: String,
    pub members: Vec<WorkspacePublishInputMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePublishInputMember {
    pub alias: String,
    pub prepared_authority_digest: String,
    pub allow_conditional: bool,
    pub acknowledge_obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePublicationMemberAuthority {
    pub alias: String,
    pub session_id: String,
    pub run_id: String,
    pub target_ref: String,
    pub before_oid: String,
    pub candidate_oid: String,
    pub prepared_authority_digest: String,
    pub allow_conditional: bool,
    pub acknowledge_obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePublicationAuthority {
    pub schema: String,
    pub publication_id: String,
    pub authority_digest: String,
    pub workspace_id: String,
    pub workspace_authority_digest: String,
    pub workspace_semantic_digest: String,
    pub preparation_id: String,
    pub after_workspace_id: String,
    pub after_workspace_authority_digest: String,
    pub policy: String,
    pub members: Vec<WorkspacePublicationMemberAuthority>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkspacePublicationStatus {
    PreparedAll,
    Publishing,
    PartiallyPublished,
    RecoveryRequired,
    PublishedAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishedWorkspaceMember {
    pub alias: String,
    pub candidate_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePublicationEvent {
    pub schema: String,
    pub publication_id: String,
    pub publication_authority_digest: String,
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_event_hash: Option<String>,
    pub status: WorkspacePublicationStatus,
    pub completed: Vec<PublishedWorkspaceMember>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<ErrorCode>,
    pub updated_unix_ms: u128,
    pub event_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePublicationProjection {
    pub schema: String,
    pub publication_id: String,
    pub authority_digest: String,
    pub workspace_id: String,
    pub after_workspace_id: String,
    pub policy: String,
    pub status: WorkspacePublicationStatus,
    pub completed: Vec<PublishedWorkspaceMember>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<ErrorCode>,
    pub remaining_aliases: Vec<String>,
    pub sequence: u64,
    pub ledger_head: String,
}

pub fn resolve(
    workspace_id: &str,
    source: &[u8],
) -> Result<WorkspacePublicationProjection, ClewError> {
    let input = parse_input(source)?;
    let after = load_retained_after_workspace(
        workspace_id,
        &input.preparation_id,
        &input.after_workspace_id,
        &input.after_workspace_authority_digest,
    )?;
    let authority = build_authority(&after, input)?;
    let directory = publication_directory(workspace_id, &authority.publication_id)?;
    let _lock = PublicationLedgerLock::acquire(&directory)?;
    if directory.file_exists(OsStr::new("authority.json"))? {
        let existing = load_authority(&directory)?;
        if existing != authority {
            return Err(binding(
                "workspace publication identity already has different authority",
            ));
        }
        let current = load_event_chain(&directory, &existing)?;
        return project(&existing, current);
    }
    // Freshness is an admission property checked exactly once, before the
    // first possible ref update. The successful prefix of a retained
    // publication deliberately makes that original workspace stale.
    load_bound_after_workspace(
        workspace_id,
        &authority.preparation_id,
        &authority.after_workspace_id,
        &authority.after_workspace_authority_digest,
    )?;
    persist_authority(&directory, &authority)?;
    if !directory.file_exists(OsStr::new("ledger.jsonl"))? {
        let origin = WorkspacePublicationEvent {
            schema: WORKSPACE_PUBLICATION_EVENT_SCHEMA.into(),
            publication_id: authority.publication_id.clone(),
            publication_authority_digest: authority.authority_digest.clone(),
            sequence: 0,
            previous_event_hash: None,
            status: WorkspacePublicationStatus::PreparedAll,
            completed: Vec::new(),
            active_alias: None,
            last_error_code: None,
            updated_unix_ms: unix_ms(),
            event_hash: String::new(),
        };
        append_event(&directory, origin)?;
    }
    let current = load_event_chain(&directory, &authority)?;
    project(&authority, current)
}

pub fn load(
    workspace_id: &str,
    publication_id: &str,
) -> Result<
    (
        WorkspacePublicationAuthority,
        WorkspacePublicationProjection,
    ),
    ClewError,
> {
    let directory = publication_directory(workspace_id, publication_id)?;
    let authority = load_authority(&directory)?;
    if authority.workspace_id != workspace_id || authority.publication_id != publication_id {
        return Err(binding(
            "publication belongs to another workspace authority",
        ));
    }
    load_retained_after_workspace(
        workspace_id,
        &authority.preparation_id,
        &authority.after_workspace_id,
        &authority.after_workspace_authority_digest,
    )?;
    let current = load_event_chain(&directory, &authority)?;
    let projection = project(&authority, current)?;
    Ok((authority, projection))
}

pub fn active_member<'a>(
    authority: &'a WorkspacePublicationAuthority,
    projection: &WorkspacePublicationProjection,
) -> Result<Option<&'a WorkspacePublicationMemberAuthority>, ClewError> {
    if projection.publication_id != authority.publication_id
        || projection.authority_digest != authority.authority_digest
    {
        return Err(binding("publication projection differs from authority"));
    }
    if projection.status == WorkspacePublicationStatus::PublishedAll {
        return Ok(None);
    }
    let alias = if projection.status == WorkspacePublicationStatus::Publishing {
        projection
            .active_alias
            .as_deref()
            .ok_or_else(|| corrupt("publishing projection has no active member"))?
    } else {
        authority
            .members
            .get(projection.completed.len())
            .map(|member| member.alias.as_str())
            .ok_or_else(|| corrupt("publication has no remaining member"))?
    };
    authority
        .members
        .iter()
        .find(|member| member.alias == alias)
        .map(Some)
        .ok_or_else(|| corrupt("publication active member is unknown"))
}

pub fn begin_next(
    workspace_id: &str,
    publication_id: &str,
    expected_ledger_head: &str,
) -> Result<WorkspacePublicationProjection, ClewError> {
    transition(
        workspace_id,
        publication_id,
        expected_ledger_head,
        |authority, current| {
            if current.status == WorkspacePublicationStatus::Publishing {
                return Ok(current.clone());
            }
            if current.status == WorkspacePublicationStatus::PublishedAll {
                return Ok(current.clone());
            }
            if !matches!(
                current.status,
                WorkspacePublicationStatus::PreparedAll
                    | WorkspacePublicationStatus::PartiallyPublished
                    | WorkspacePublicationStatus::RecoveryRequired
            ) {
                return Err(precondition("publication cannot start its next member"));
            }
            let next = authority
                .members
                .get(current.completed.len())
                .ok_or_else(|| corrupt("publication has no remaining member"))?;
            Ok(next_event(
                current,
                WorkspacePublicationStatus::Publishing,
                current.completed.clone(),
                Some(next.alias.clone()),
                None,
            ))
        },
    )
}

pub fn record_published(
    workspace_id: &str,
    publication_id: &str,
    expected_ledger_head: &str,
    alias: &str,
    candidate_oid: &str,
) -> Result<WorkspacePublicationProjection, ClewError> {
    transition(
        workspace_id,
        publication_id,
        expected_ledger_head,
        |authority, current| {
            if current.status != WorkspacePublicationStatus::Publishing
                || current.active_alias.as_deref() != Some(alias)
            {
                return Err(precondition(
                    "only the active publishing member can complete",
                ));
            }
            let expected = authority
                .members
                .get(current.completed.len())
                .ok_or_else(|| corrupt("publication member order is exhausted"))?;
            if expected.alias != alias || expected.candidate_oid != candidate_oid {
                return Err(binding(
                    "published candidate differs from sealed member authority",
                ));
            }
            let mut completed = current.completed.clone();
            completed.push(PublishedWorkspaceMember {
                alias: alias.into(),
                candidate_oid: candidate_oid.into(),
            });
            let status = if completed.len() == authority.members.len() {
                WorkspacePublicationStatus::PublishedAll
            } else {
                WorkspacePublicationStatus::PartiallyPublished
            };
            Ok(next_event(current, status, completed, None, None))
        },
    )
}

pub fn record_failure(
    workspace_id: &str,
    publication_id: &str,
    expected_ledger_head: &str,
    alias: &str,
    error_code: ErrorCode,
) -> Result<WorkspacePublicationProjection, ClewError> {
    transition(
        workspace_id,
        publication_id,
        expected_ledger_head,
        |_authority, current| {
            if current.status != WorkspacePublicationStatus::Publishing
                || current.active_alias.as_deref() != Some(alias)
            {
                return Err(precondition("only the active publishing member can fail"));
            }
            Ok(next_event(
                current,
                WorkspacePublicationStatus::RecoveryRequired,
                current.completed.clone(),
                Some(alias.into()),
                Some(error_code),
            ))
        },
    )
}

fn transition(
    workspace_id: &str,
    publication_id: &str,
    expected_ledger_head: &str,
    build: impl FnOnce(
        &WorkspacePublicationAuthority,
        &WorkspacePublicationEvent,
    ) -> Result<WorkspacePublicationEvent, ClewError>,
) -> Result<WorkspacePublicationProjection, ClewError> {
    let directory = publication_directory(workspace_id, publication_id)?;
    let _lock = PublicationLedgerLock::acquire(&directory)?;
    let authority = load_authority(&directory)?;
    if authority.workspace_id != workspace_id || authority.publication_id != publication_id {
        return Err(binding(
            "publication belongs to another workspace authority",
        ));
    }
    let current = load_event_chain(&directory, &authority)?;
    if current.event_hash != expected_ledger_head {
        return Err(precondition(
            "publication ledger changed concurrently; reload before retrying",
        ));
    }
    let next = build(&authority, &current)?;
    if next == current {
        return project(&authority, current);
    }
    append_event(&directory, next)?;
    let current = load_event_chain(&directory, &authority)?;
    project(&authority, current)
}

fn build_authority(
    after: &AfterWorkspace,
    input: WorkspacePublishInput,
) -> Result<WorkspacePublicationAuthority, ClewError> {
    if input.schema != WORKSPACE_PUBLISH_INPUT_SCHEMA
        || input.policy != "ROLL_FORWARD_ONLY"
        || input.members.len() != after.members.len()
    {
        return Err(invalid("workspace publish input authority is invalid"));
    }
    let mut aliases = BTreeSet::new();
    let mut members = Vec::with_capacity(input.members.len());
    for requested in input.members {
        if !aliases.insert(requested.alias.clone())
            || requested.acknowledge_obligations.len() > 4096
            || requested
                .acknowledge_obligations
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(invalid(
                "publication aliases and acknowledgements must be unique and ordered",
            ));
        }
        let prepared = after
            .members
            .iter()
            .find(|member| member.alias == requested.alias)
            .ok_or_else(|| invalid("publication member is unknown"))?;
        require_publishable_member(prepared, &requested)?;
        members.push(WorkspacePublicationMemberAuthority {
            alias: prepared.alias.clone(),
            session_id: prepared.session_id.clone(),
            run_id: prepared.run_id.clone(),
            target_ref: prepared.target_ref.clone(),
            before_oid: prepared.before_oid.clone(),
            candidate_oid: prepared.candidate_oid.clone(),
            prepared_authority_digest: prepared.prepared_authority_digest.clone(),
            allow_conditional: requested.allow_conditional,
            acknowledge_obligations: requested.acknowledge_obligations,
        });
    }
    let mut authority = WorkspacePublicationAuthority {
        schema: WORKSPACE_PUBLICATION_AUTHORITY_SCHEMA.into(),
        publication_id: String::new(),
        authority_digest: String::new(),
        workspace_id: after.workspace_id.clone(),
        workspace_authority_digest: after.workspace_authority_digest.clone(),
        workspace_semantic_digest: after.workspace_semantic_digest.clone(),
        preparation_id: after.preparation_id.clone(),
        after_workspace_id: after.after_workspace_id.clone(),
        after_workspace_authority_digest: after.authority_digest.clone(),
        policy: input.policy,
        members,
    };
    authority.publication_id = publication_id(&authority)?;
    authority.authority_digest = authority_digest(&authority)?;
    validate_authority(&authority)?;
    Ok(authority)
}

fn require_publishable_member(
    prepared: &AfterWorkspaceMember,
    requested: &WorkspacePublishInputMember,
) -> Result<(), ClewError> {
    if requested.prepared_authority_digest != prepared.prepared_authority_digest {
        return Err(binding(
            "publication request differs from prepared member authority",
        ));
    }
    match prepared.run_status {
        crate::session::RunStatus::ReadyToPublish
            if !requested.allow_conditional && requested.acknowledge_obligations.is_empty() => {}
        crate::session::RunStatus::ReadyToPublishConditional
            if requested.allow_conditional
                && requested.acknowledge_obligations.len() == prepared.obligation_count
                && !requested.acknowledge_obligations.is_empty() => {}
        crate::session::RunStatus::ValidatedConditional => {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "workspace member is conditional but not eligible for publication",
            ));
        }
        _ => {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "publication mode or obligation count differs from prepared member",
            ));
        }
    }
    Ok(())
}

fn parse_input(source: &[u8]) -> Result<WorkspacePublishInput, ClewError> {
    if source.is_empty() || source.len() > MAX_WORKSPACE_PUBLISH_INPUT_BYTES {
        return Err(invalid(
            "workspace publish input is empty or exceeds 256 KiB",
        ));
    }
    let value: Value = serde_json::from_slice(source)
        .map_err(|_| invalid("workspace publish input is not valid JSON"))?;
    if canonical::bytes(&value).map_err(internal)? != source {
        return Err(invalid(
            "workspace publish input must be canonical compact JSON with NFC strings",
        ));
    }
    serde_json::from_value(value).map_err(|_| invalid("workspace publish input schema is invalid"))
}

fn persist_authority(
    directory: &crate::state::ManagedDirectory,
    authority: &WorkspacePublicationAuthority,
) -> Result<(), ClewError> {
    validate_authority(authority)?;
    let bytes = canonical::bytes(authority).map_err(internal)?;
    if bytes.len() > MAX_WORKSPACE_PUBLICATION_AUTHORITY_BYTES {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "workspace publication authority exceeds 512 KiB",
        ));
    }
    if !directory.atomic_create(OsStr::new("authority.json"), &bytes)? {
        let existing = directory.read_file(
            OsStr::new("authority.json"),
            MAX_WORKSPACE_PUBLICATION_AUTHORITY_BYTES,
        )?;
        if existing != bytes {
            return Err(binding(
                "workspace publication identity already has different authority",
            ));
        }
    }
    Ok(())
}

fn load_authority(
    directory: &crate::state::ManagedDirectory,
) -> Result<WorkspacePublicationAuthority, ClewError> {
    let bytes = directory.read_file(
        OsStr::new("authority.json"),
        MAX_WORKSPACE_PUBLICATION_AUTHORITY_BYTES,
    )?;
    let authority: WorkspacePublicationAuthority = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("workspace publication authority is not valid JSON"))?;
    if canonical::bytes(&authority).map_err(internal)? != bytes {
        return Err(corrupt("workspace publication authority is not canonical"));
    }
    validate_authority(&authority)?;
    Ok(authority)
}

fn validate_authority(authority: &WorkspacePublicationAuthority) -> Result<(), ClewError> {
    let mut aliases = BTreeSet::new();
    if authority.schema != WORKSPACE_PUBLICATION_AUTHORITY_SCHEMA
        || authority.policy != "ROLL_FORWARD_ONLY"
        || !(2..=4).contains(&authority.members.len())
        || authority.publication_id != publication_id(authority)?
        || authority.authority_digest != authority_digest(authority)?
        || !digest(&authority.workspace_authority_digest)
        || !digest(&authority.workspace_semantic_digest)
        || !digest(&authority.after_workspace_authority_digest)
        || authority.members.iter().any(|member| {
            !aliases.insert(member.alias.as_str())
                || member.session_id.is_empty()
                || member.session_id.len() > 128
                || member.run_id.is_empty()
                || member.run_id.len() > 128
                || member.target_ref.is_empty()
                || member.target_ref.len() > 512
                || !git_oid(&member.before_oid)
                || !git_oid(&member.candidate_oid)
                || !digest(&member.prepared_authority_digest)
                || member
                    .acknowledge_obligations
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || member.allow_conditional == member.acknowledge_obligations.is_empty()
        })
    {
        return Err(corrupt("workspace publication authority is invalid"));
    }
    Ok(())
}

fn load_event_chain(
    directory: &crate::state::ManagedDirectory,
    authority: &WorkspacePublicationAuthority,
) -> Result<WorkspacePublicationEvent, ClewError> {
    let bytes = directory.read_file(
        OsStr::new("ledger.jsonl"),
        MAX_WORKSPACE_PUBLICATION_LEDGER_BYTES,
    )?;
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err(corrupt(
            "workspace publication ledger is missing or incomplete",
        ));
    }
    let mut previous: Option<WorkspacePublicationEvent> = None;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let event: WorkspacePublicationEvent = serde_json::from_slice(line)
            .map_err(|_| corrupt("workspace publication event is not valid JSON"))?;
        if canonical::bytes(&event).map_err(internal)? != line
            || event.schema != WORKSPACE_PUBLICATION_EVENT_SCHEMA
            || event.publication_id != authority.publication_id
            || event.publication_authority_digest != authority.authority_digest
            || event.event_hash != event_hash(&event)?
        {
            return Err(corrupt("workspace publication event authority is invalid"));
        }
        validate_transition(authority, previous.as_ref(), &event)?;
        previous = Some(event);
    }
    previous.ok_or_else(|| corrupt("workspace publication ledger is empty"))
}

fn validate_transition(
    authority: &WorkspacePublicationAuthority,
    previous: Option<&WorkspacePublicationEvent>,
    current: &WorkspacePublicationEvent,
) -> Result<(), ClewError> {
    if current.updated_unix_ms == 0
        || current.completed.len() > authority.members.len()
        || current.completed.iter().enumerate().any(|(index, member)| {
            authority.members.get(index).is_none_or(|expected| {
                expected.alias != member.alias || expected.candidate_oid != member.candidate_oid
            })
        })
    {
        return Err(corrupt(
            "workspace publication completion prefix is invalid",
        ));
    }
    let valid = match previous {
        None => {
            current.sequence == 0
                && current.previous_event_hash.is_none()
                && current.status == WorkspacePublicationStatus::PreparedAll
                && current.completed.is_empty()
                && current.active_alias.is_none()
                && current.last_error_code.is_none()
        }
        Some(previous) => {
            if current.sequence != previous.sequence.saturating_add(1)
                || current.previous_event_hash.as_deref() != Some(previous.event_hash.as_str())
                || current.updated_unix_ms < previous.updated_unix_ms
            {
                return Err(corrupt(
                    "workspace publication ledger chain is discontinuous",
                ));
            }
            let expected_next = authority
                .members
                .get(previous.completed.len())
                .map(|member| member.alias.as_str());
            match current.status {
                WorkspacePublicationStatus::Publishing => {
                    matches!(
                        previous.status,
                        WorkspacePublicationStatus::PreparedAll
                            | WorkspacePublicationStatus::PartiallyPublished
                            | WorkspacePublicationStatus::RecoveryRequired
                    ) && current.completed == previous.completed
                        && current.active_alias.as_deref() == expected_next
                        && current.last_error_code.is_none()
                }
                WorkspacePublicationStatus::RecoveryRequired => {
                    previous.status == WorkspacePublicationStatus::Publishing
                        && current.completed == previous.completed
                        && current.active_alias == previous.active_alias
                        && current.last_error_code.is_some()
                }
                WorkspacePublicationStatus::PartiallyPublished
                | WorkspacePublicationStatus::PublishedAll => {
                    previous.status == WorkspacePublicationStatus::Publishing
                        && current.completed.len() == previous.completed.len() + 1
                        && current.completed[..previous.completed.len()] == previous.completed
                        && current.completed.last().is_some_and(|member| {
                            Some(member.alias.as_str()) == previous.active_alias.as_deref()
                        })
                        && current.active_alias.is_none()
                        && current.last_error_code.is_none()
                        && (current.status == WorkspacePublicationStatus::PublishedAll)
                            == (current.completed.len() == authority.members.len())
                }
                WorkspacePublicationStatus::PreparedAll => false,
            }
        }
    };
    if valid {
        Ok(())
    } else {
        Err(corrupt("workspace publication transition is invalid"))
    }
}

fn append_event(
    directory: &crate::state::ManagedDirectory,
    mut event: WorkspacePublicationEvent,
) -> Result<(), ClewError> {
    event.event_hash = event_hash(&event)?;
    let mut bytes = canonical::bytes(&event).map_err(internal)?;
    bytes.push(b'\n');
    let mut file = directory.open_append(OsStr::new("ledger.jsonl"))?;
    if file
        .metadata()
        .map_err(io_error)?
        .len()
        .saturating_add(bytes.len() as u64)
        > MAX_WORKSPACE_PUBLICATION_LEDGER_BYTES as u64
    {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "workspace publication ledger exceeds 4 MiB",
        ));
    }
    file.write_all(&bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn next_event(
    previous: &WorkspacePublicationEvent,
    status: WorkspacePublicationStatus,
    completed: Vec<PublishedWorkspaceMember>,
    active_alias: Option<String>,
    last_error_code: Option<ErrorCode>,
) -> WorkspacePublicationEvent {
    WorkspacePublicationEvent {
        schema: WORKSPACE_PUBLICATION_EVENT_SCHEMA.into(),
        publication_id: previous.publication_id.clone(),
        publication_authority_digest: previous.publication_authority_digest.clone(),
        sequence: previous.sequence.saturating_add(1),
        previous_event_hash: Some(previous.event_hash.clone()),
        status,
        completed,
        active_alias,
        last_error_code,
        updated_unix_ms: unix_ms(),
        event_hash: String::new(),
    }
}

fn project(
    authority: &WorkspacePublicationAuthority,
    current: WorkspacePublicationEvent,
) -> Result<WorkspacePublicationProjection, ClewError> {
    let completed = current.completed.clone();
    let remaining_aliases = authority
        .members
        .iter()
        .skip(completed.len())
        .map(|member| member.alias.clone())
        .collect::<Vec<_>>();
    Ok(WorkspacePublicationProjection {
        schema: "codeclew-workspace-publication-projection/1.0".into(),
        publication_id: authority.publication_id.clone(),
        authority_digest: authority.authority_digest.clone(),
        workspace_id: authority.workspace_id.clone(),
        after_workspace_id: authority.after_workspace_id.clone(),
        policy: authority.policy.clone(),
        status: current.status,
        completed,
        active_alias: current.active_alias,
        last_error_code: current.last_error_code,
        remaining_aliases,
        sequence: current.sequence,
        ledger_head: current.event_hash,
    })
}

fn publication_directory(
    workspace_id: &str,
    publication_id: &str,
) -> Result<crate::state::ManagedDirectory, ClewError> {
    let component = publication_id
        .strip_prefix("workspace-publication:sha256:")
        .filter(|value| digest_component(value))
        .ok_or_else(|| invalid("workspace publication id is invalid"))?;
    let state = StateAuthority::process_default()?;
    state.directory_at(
        &state
            .workspace_root(workspace_id)?
            .join(Path::new("publications"))
            .join(component),
    )
}

fn authority_digest(authority: &WorkspacePublicationAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn publication_id(authority: &WorkspacePublicationAuthority) -> Result<String, ClewError> {
    Ok(format!(
        "workspace-publication:{}",
        canonical::hash(&json!({
            "schema":"codeclew-workspace-publication-request-authority/1.0",
            "workspaceId":authority.workspace_id,
            "workspaceAuthorityDigest":authority.workspace_authority_digest,
            "workspaceSemanticDigest":authority.workspace_semantic_digest,
            "preparationId":authority.preparation_id,
            "afterWorkspaceId":authority.after_workspace_id,
            "afterWorkspaceAuthorityDigest":authority.after_workspace_authority_digest,
            "policy":authority.policy,
            "members":authority.members,
        }))
        .map_err(internal)?
    ))
}

fn event_hash(event: &WorkspacePublicationEvent) -> Result<String, ClewError> {
    let mut unsigned = event.clone();
    unsigned.event_hash.clear();
    canonical::hash(&unsigned).map_err(internal)
}

struct PublicationLedgerLock(std::fs::File);

impl PublicationLedgerLock {
    fn acquire(directory: &crate::state::ManagedDirectory) -> Result<Self, ClewError> {
        let file = directory.open_lock(OsStr::new("ledger.lock"))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self(file))
    }
}

impl Drop for PublicationLedgerLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
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
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn precondition(message: &str) -> ClewError {
    ClewError::new(ErrorCode::PreconditionFailed, message)
}

fn binding(message: &str) -> ClewError {
    ClewError::new(ErrorCode::BindingChanged, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority() -> WorkspacePublicationAuthority {
        let members = ["api", "worker"]
            .iter()
            .enumerate()
            .map(|(index, alias)| WorkspacePublicationMemberAuthority {
                alias: (*alias).into(),
                session_id: format!("session:{index}"),
                run_id: format!("run:{index}"),
                target_ref: "refs/heads/main".into(),
                before_oid: "a".repeat(40),
                candidate_oid: if index == 0 { "b" } else { "c" }.repeat(40),
                prepared_authority_digest: format!("sha256:{}", "d".repeat(64)),
                allow_conditional: false,
                acknowledge_obligations: Vec::new(),
            })
            .collect();
        let mut value = WorkspacePublicationAuthority {
            schema: WORKSPACE_PUBLICATION_AUTHORITY_SCHEMA.into(),
            publication_id: String::new(),
            authority_digest: String::new(),
            workspace_id: "workspace:test".into(),
            workspace_authority_digest: format!("sha256:{}", "f".repeat(64)),
            workspace_semantic_digest: format!("sha256:{}", "1".repeat(64)),
            preparation_id: format!("workspace-prepare:sha256:{}", "2".repeat(64)),
            after_workspace_id: format!("after-workspace:sha256:{}", "3".repeat(64)),
            after_workspace_authority_digest: format!("sha256:{}", "4".repeat(64)),
            policy: "ROLL_FORWARD_ONLY".into(),
            members,
        };
        value.publication_id = publication_id(&value).unwrap();
        value.authority_digest = authority_digest(&value).unwrap();
        value
    }

    fn origin(authority: &WorkspacePublicationAuthority) -> WorkspacePublicationEvent {
        let mut event = WorkspacePublicationEvent {
            schema: WORKSPACE_PUBLICATION_EVENT_SCHEMA.into(),
            publication_id: authority.publication_id.clone(),
            publication_authority_digest: authority.authority_digest.clone(),
            sequence: 0,
            previous_event_hash: None,
            status: WorkspacePublicationStatus::PreparedAll,
            completed: Vec::new(),
            active_alias: None,
            last_error_code: None,
            updated_unix_ms: 1,
            event_hash: String::new(),
        };
        event.event_hash = event_hash(&event).unwrap();
        event
    }

    fn signed(mut event: WorkspacePublicationEvent) -> WorkspacePublicationEvent {
        event.event_hash = event_hash(&event).unwrap();
        event
    }

    #[test]
    fn every_fault_boundary_preserves_an_ordered_recoverable_prefix() {
        let authority = authority();
        let origin = origin(&authority);
        validate_transition(&authority, None, &origin).unwrap();
        let publishing_api = signed(next_event(
            &origin,
            WorkspacePublicationStatus::Publishing,
            Vec::new(),
            Some("api".into()),
            None,
        ));
        validate_transition(&authority, Some(&origin), &publishing_api).unwrap();
        let failed_api = signed(next_event(
            &publishing_api,
            WorkspacePublicationStatus::RecoveryRequired,
            Vec::new(),
            Some("api".into()),
            Some(ErrorCode::RefCompareAndSwapFailed),
        ));
        validate_transition(&authority, Some(&publishing_api), &failed_api).unwrap();
        let retry_api = signed(next_event(
            &failed_api,
            WorkspacePublicationStatus::Publishing,
            Vec::new(),
            Some("api".into()),
            None,
        ));
        validate_transition(&authority, Some(&failed_api), &retry_api).unwrap();
        let api_done = PublishedWorkspaceMember {
            alias: "api".into(),
            candidate_oid: "b".repeat(40),
        };
        let partial = signed(next_event(
            &retry_api,
            WorkspacePublicationStatus::PartiallyPublished,
            vec![api_done.clone()],
            None,
            None,
        ));
        validate_transition(&authority, Some(&retry_api), &partial).unwrap();
        let publishing_worker = signed(next_event(
            &partial,
            WorkspacePublicationStatus::Publishing,
            vec![api_done.clone()],
            Some("worker".into()),
            None,
        ));
        validate_transition(&authority, Some(&partial), &publishing_worker).unwrap();
        let failed_worker = signed(next_event(
            &publishing_worker,
            WorkspacePublicationStatus::RecoveryRequired,
            vec![api_done.clone()],
            Some("worker".into()),
            Some(ErrorCode::WorktreeRecoveryRequired),
        ));
        validate_transition(&authority, Some(&publishing_worker), &failed_worker).unwrap();
        assert_eq!(failed_worker.completed, vec![api_done]);

        let retry_worker = signed(next_event(
            &failed_worker,
            WorkspacePublicationStatus::Publishing,
            failed_worker.completed.clone(),
            Some("worker".into()),
            None,
        ));
        validate_transition(&authority, Some(&failed_worker), &retry_worker).unwrap();
        let published_all = signed(next_event(
            &retry_worker,
            WorkspacePublicationStatus::PublishedAll,
            vec![
                PublishedWorkspaceMember {
                    alias: "api".into(),
                    candidate_oid: "b".repeat(40),
                },
                PublishedWorkspaceMember {
                    alias: "worker".into(),
                    candidate_oid: "c".repeat(40),
                },
            ],
            None,
            None,
        ));
        validate_transition(&authority, Some(&retry_worker), &published_all).unwrap();
    }

    #[test]
    fn completion_cannot_skip_reorder_or_roll_back_members() {
        let authority = authority();
        let origin = origin(&authority);
        let publishing = signed(next_event(
            &origin,
            WorkspacePublicationStatus::Publishing,
            Vec::new(),
            Some("api".into()),
            None,
        ));
        let wrong = PublishedWorkspaceMember {
            alias: "worker".into(),
            candidate_oid: "c".repeat(40),
        };
        let skipped = signed(next_event(
            &publishing,
            WorkspacePublicationStatus::PartiallyPublished,
            vec![wrong],
            None,
            None,
        ));
        assert!(validate_transition(&authority, Some(&publishing), &skipped).is_err());
        let terminal_without_all = signed(next_event(
            &publishing,
            WorkspacePublicationStatus::PublishedAll,
            vec![PublishedWorkspaceMember {
                alias: "api".into(),
                candidate_oid: "b".repeat(40),
            }],
            None,
            None,
        ));
        assert!(validate_transition(&authority, Some(&publishing), &terminal_without_all).is_err());
    }
}
