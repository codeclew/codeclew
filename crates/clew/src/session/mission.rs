pub mod development_record;

use super::{
    ContextObject, PlanObject, RunRecord, SessionAuthority, SessionLanguage, internal, invalid,
    read_managed_json, unix_ms, write_managed_json_create_new,
};
use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::runtime::RuntimeMode;
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

#[cfg(unix)]
use std::os::fd::AsRawFd;

pub const CHANGE_SPEC_SCHEMA: &str = "codeclew-change-spec/1.0";
pub const MISSION_IDENTITY_SCHEMA: &str = "codeclew-mission-identity/1.0";
pub const MISSION_EVENT_SCHEMA: &str = "codeclew-mission-event/1.0";
pub const MISSION_STATUS_SCHEMA: &str = "codeclew-mission-status/1.0";
pub const MISSION_INSPECTION_SCHEMA: &str = "codeclew-mission-inspection/1.0";
pub const MISSION_RECORD_RESULT_SCHEMA: &str = "codeclew-mission-record-result/1.0";
pub const MAX_CHANGE_SPEC_BYTES: usize = 256 * 1024;
const MAX_MISSION_IDENTITY_BYTES: usize = 256 * 1024;
const MAX_MISSION_EVENT_BYTES: usize = 128 * 1024;
const MAX_MISSION_EVENTS: usize = 4096;
const MAX_MISSION_MEMBERS: usize = 8;
const MAX_SPEC_ITEMS: usize = 256;
const MAX_ITEM_TEXT_BYTES: usize = 16 * 1024;
const MAX_SPEC_TEXT_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSpecItem {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocsPolicy {
    #[serde(default)]
    pub required_requirement_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSpec {
    pub schema: String,
    pub intent: String,
    pub requirements: Vec<ChangeSpecItem>,
    #[serde(default)]
    pub non_goals: Vec<ChangeSpecItem>,
    pub acceptance_criteria: Vec<ChangeSpecItem>,
    pub docs_policy: DocsPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissionMemberAuthority {
    pub session_id: String,
    pub session_authority_digest: String,
    pub repository_key: String,
    pub base_revision: String,
    pub target_ref: String,
    pub target_oid: String,
    pub runtime_key: String,
    pub runtime_mode: RuntimeMode,
    pub language: SessionLanguage,
    pub compilations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MissionIdentity {
    schema: String,
    mission_id: String,
    identity_digest: String,
    change_spec_digest: String,
    members: Vec<MissionMemberAuthority>,
    created_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MissionEventKind {
    Opened,
    Bound,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextBinding {
    pub context_id: String,
    pub authority_digest: String,
    pub evidence_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanBinding {
    pub plan_id: String,
    pub authority_digest: String,
    pub source_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunBinding {
    pub run_id: String,
    pub authority_digest: String,
    pub status: super::RunStatus,
    pub candidate_digest: Option<String>,
    pub prepared_authority_digest: Option<String>,
    pub validation_digest: String,
    pub final_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissionBinding {
    pub session_id: String,
    pub context: Option<ContextBinding>,
    pub plan: Option<PlanBinding>,
    pub run: Option<RunBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MissionEvent {
    schema: String,
    mission_id: String,
    sequence: u64,
    previous_event_hash: Option<String>,
    kind: MissionEventKind,
    binding: Option<MissionBinding>,
    event_hash: String,
    created_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MissionLifecycle {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissionStatus {
    pub schema: String,
    pub mission_id: String,
    pub authority_digest: String,
    pub identity_digest: String,
    pub change_spec_digest: String,
    pub event_head: String,
    pub event_count: usize,
    pub binding_count: usize,
    pub member_count: usize,
    pub lifecycle: MissionLifecycle,
    pub readiness: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSpecSummary {
    pub requirement_count: usize,
    pub non_goal_count: usize,
    pub acceptance_criterion_count: usize,
    pub documented_requirement_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissionInspection {
    pub schema: String,
    pub status: MissionStatus,
    pub change_spec: ChangeSpecSummary,
    pub members: Vec<MissionMemberAuthority>,
    pub latest_bindings: Vec<MissionBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissionRecordResult {
    pub schema: String,
    pub appended: bool,
    pub status: MissionStatus,
}

pub fn open(session_ids: &[String], source: &[u8]) -> Result<MissionInspection, ClewError> {
    let spec = parse_change_spec(source)?;
    let state = StateAuthority::process_default()?;
    let members = load_live_members(session_ids)?;
    create_with_state(&state, source, spec, members)
}

pub fn inspect(mission_id: &str) -> Result<MissionInspection, ClewError> {
    let state = StateAuthority::process_default()?;
    let loaded = load_with_state(&state, mission_id)?;
    if loaded.status.lifecycle == MissionLifecycle::Open {
        require_live_members(&loaded.identity.members)?;
    }
    Ok(inspection(&loaded))
}

pub fn status(mission_id: &str) -> Result<MissionStatus, ClewError> {
    let state = StateAuthority::process_default()?;
    let loaded = load_with_state(&state, mission_id)?;
    if loaded.status.lifecycle == MissionLifecycle::Open {
        require_live_members(&loaded.identity.members)?;
    }
    Ok(loaded.status)
}

pub fn record(
    mission_id: &str,
    session_id: &str,
    context_id: Option<&str>,
    plan_id: Option<&str>,
    run_id: Option<&str>,
) -> Result<MissionRecordResult, ClewError> {
    if context_id.is_none() || plan_id.is_none() && run_id.is_some() {
        return Err(invalid(
            "mission record requires context; run additionally requires plan",
        ));
    }
    let state = StateAuthority::process_default()?;
    let root = state.mission_root(mission_id)?;
    let _lock = MissionLock::acquire(&state, &root)?;
    let loaded = load_with_state_unlocked(&state, mission_id)?;
    require_open(&loaded)?;
    let member = loaded
        .identity
        .members
        .iter()
        .find(|member| member.session_id == session_id)
        .ok_or_else(|| invalid("mission record session is not a bound member"))?;
    let (session, _) = SessionAuthority::load(session_id)?;
    require_same_member(member, &session)?;
    let admission = session.open_admission()?;
    require_fresh(&session, &admission)?;
    let binding = build_binding(&session, context_id, plan_id, run_id)?;
    if loaded.events.iter().any(|event| {
        event.kind == MissionEventKind::Bound && event.binding.as_ref() == Some(&binding)
    }) {
        return Ok(MissionRecordResult {
            schema: MISSION_RECORD_RESULT_SCHEMA.into(),
            appended: false,
            status: loaded.status,
        });
    }
    append_event_unlocked(
        &state,
        &root,
        mission_id,
        &loaded.events,
        MissionEventKind::Bound,
        Some(binding),
    )?;
    let status = load_with_state_unlocked(&state, mission_id)?.status;
    Ok(MissionRecordResult {
        schema: MISSION_RECORD_RESULT_SCHEMA.into(),
        appended: true,
        status,
    })
}

pub fn close(mission_id: &str) -> Result<MissionRecordResult, ClewError> {
    let state = StateAuthority::process_default()?;
    let root = state.mission_root(mission_id)?;
    let _lock = MissionLock::acquire(&state, &root)?;
    let loaded = load_with_state_unlocked(&state, mission_id)?;
    if loaded.status.lifecycle == MissionLifecycle::Closed {
        return Ok(MissionRecordResult {
            schema: MISSION_RECORD_RESULT_SCHEMA.into(),
            appended: false,
            status: loaded.status,
        });
    }
    require_live_members(&loaded.identity.members)?;
    append_event_unlocked(
        &state,
        &root,
        mission_id,
        &loaded.events,
        MissionEventKind::Closed,
        None,
    )?;
    let status = load_with_state_unlocked(&state, mission_id)?.status;
    Ok(MissionRecordResult {
        schema: MISSION_RECORD_RESULT_SCHEMA.into(),
        appended: true,
        status,
    })
}

struct LoadedMission {
    identity: MissionIdentity,
    spec: ChangeSpec,
    events: Vec<MissionEvent>,
    status: MissionStatus,
}

fn create_with_state(
    state: &StateAuthority,
    source: &[u8],
    spec: ChangeSpec,
    mut members: Vec<MissionMemberAuthority>,
) -> Result<MissionInspection, ClewError> {
    validate_members(&mut members)?;
    let mission_id = format!("mission:{}", Uuid::new_v4());
    let root = state.mission_root(&mission_id)?;
    let directory = state.directory_at(&root)?;
    directory.child(Path::new("events"))?;
    directory.child(Path::new("records"))?;
    let change_spec_digest = canonical::hash_bytes(source);
    let created_unix_ms = unix_ms();
    let mut identity = MissionIdentity {
        schema: MISSION_IDENTITY_SCHEMA.into(),
        mission_id: mission_id.clone(),
        identity_digest: String::new(),
        change_spec_digest,
        members,
        created_unix_ms,
    };
    identity.identity_digest = identity_digest(&identity)?;
    write_managed_json_create_new(state, &root.join("identity.json"), &identity)?;
    let mut spec_file = directory.create_file(OsStr::new("change-spec.json"))?;
    spec_file.write_all(source).map_err(super::io_error)?;
    spec_file.sync_all().map_err(super::io_error)?;
    append_event_unlocked(
        state,
        &root,
        &mission_id,
        &[],
        MissionEventKind::Opened,
        None,
    )?;
    let loaded = load_with_state_unlocked(state, &mission_id)?;
    debug_assert_eq!(loaded.spec, spec);
    Ok(inspection(&loaded))
}

fn load_with_state(state: &StateAuthority, mission_id: &str) -> Result<LoadedMission, ClewError> {
    let root = state.mission_root(mission_id)?;
    let _lock = MissionLock::acquire(state, &root)?;
    load_with_state_unlocked(state, mission_id)
}

fn load_with_state_unlocked(
    state: &StateAuthority,
    mission_id: &str,
) -> Result<LoadedMission, ClewError> {
    let root = state.mission_root(mission_id)?;
    let identity: MissionIdentity = read_managed_json(
        state,
        &root.join("identity.json"),
        MAX_MISSION_IDENTITY_BYTES,
    )?;
    if identity.schema != MISSION_IDENTITY_SCHEMA
        || identity.mission_id != mission_id
        || identity.identity_digest != identity_digest(&identity)?
    {
        return Err(invalid("mission identity authority is invalid"));
    }
    let mut members = identity.members.clone();
    validate_members(&mut members)?;
    if members != identity.members {
        return Err(invalid("mission members are not in canonical order"));
    }
    let source = state
        .read_private_file(&root.join("change-spec.json"), MAX_CHANGE_SPEC_BYTES)
        .map_err(|_| invalid("mission ChangeSpec is missing or exceeds its limit"))?;
    let spec = parse_change_spec(&source)?;
    if canonical::hash_bytes(&source) != identity.change_spec_digest {
        return Err(invalid("mission ChangeSpec authority changed"));
    }
    let events = load_events(state, &root, mission_id)?;
    let status = derive_status(&identity, &events)?;
    Ok(LoadedMission {
        identity,
        spec,
        events,
        status,
    })
}

fn parse_change_spec(source: &[u8]) -> Result<ChangeSpec, ClewError> {
    if source.is_empty() || source.len() > MAX_CHANGE_SPEC_BYTES {
        return Err(invalid("ChangeSpec is empty or exceeds 256 KiB"));
    }
    let value: Value = serde_json::from_slice(source)
        .map_err(|error| invalid(&format!("ChangeSpec JSON is invalid: {error}")))?;
    if !crate::text_authority::json_strings_are_nfc(&value, 0) {
        return Err(invalid("ChangeSpec keys and strings must use NFC Unicode"));
    }
    let spec: ChangeSpec = serde_json::from_value(value)
        .map_err(|error| invalid(&format!("ChangeSpec schema is invalid: {error}")))?;
    if canonical::bytes(&spec).map_err(internal)? != source {
        return Err(invalid("ChangeSpec must use canonical JSON bytes"));
    }
    validate_change_spec(&spec)?;
    Ok(spec)
}

fn validate_change_spec(spec: &ChangeSpec) -> Result<(), ClewError> {
    if spec.schema != CHANGE_SPEC_SCHEMA {
        return Err(invalid("ChangeSpec schema is unsupported"));
    }
    validate_text("ChangeSpec intent", &spec.intent)?;
    if spec.requirements.is_empty() || spec.acceptance_criteria.is_empty() {
        return Err(invalid(
            "ChangeSpec requires at least one requirement and acceptance criterion",
        ));
    }
    if spec.requirements.len() > MAX_SPEC_ITEMS
        || spec.non_goals.len() > MAX_SPEC_ITEMS
        || spec.acceptance_criteria.len() > MAX_SPEC_ITEMS
    {
        return Err(invalid("ChangeSpec item count exceeds 256 per section"));
    }
    let mut ids = BTreeSet::new();
    let mut text_bytes = spec.intent.len();
    for item in spec
        .requirements
        .iter()
        .chain(&spec.non_goals)
        .chain(&spec.acceptance_criteria)
    {
        if !valid_stable_id(&item.id) || !ids.insert(item.id.clone()) {
            return Err(invalid(
                "ChangeSpec item IDs must be unique stable identifiers",
            ));
        }
        validate_text("ChangeSpec item text", &item.text)?;
        text_bytes = text_bytes
            .checked_add(item.text.len())
            .ok_or_else(|| invalid("ChangeSpec text size overflow"))?;
    }
    if text_bytes > MAX_SPEC_TEXT_BYTES {
        return Err(invalid("ChangeSpec text exceeds 128 KiB"));
    }
    let requirement_ids = spec
        .requirements
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut documented = BTreeSet::new();
    for requirement_id in &spec.docs_policy.required_requirement_ids {
        if !requirement_ids.contains(requirement_id.as_str()) || !documented.insert(requirement_id)
        {
            return Err(invalid(
                "docsPolicy requirement IDs must be unique bound requirements",
            ));
        }
    }
    Ok(())
}

fn validate_text(label: &str, value: &str) -> Result<(), ClewError> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > MAX_ITEM_TEXT_BYTES
        || value.contains('\0')
        || !crate::text_authority::is_nfc(value)
    {
        return Err(invalid(&format!(
            "{label} must be trimmed non-empty NFC text no larger than 16 KiB"
        )));
    }
    Ok(())
}

fn valid_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn load_live_members(session_ids: &[String]) -> Result<Vec<MissionMemberAuthority>, ClewError> {
    if session_ids.is_empty() || session_ids.len() > MAX_MISSION_MEMBERS {
        return Err(invalid("mission requires between 1 and 8 sessions"));
    }
    let mut sessions = Vec::with_capacity(session_ids.len());
    for session_id in session_ids {
        let (session, _) = SessionAuthority::load(session_id)?;
        sessions.push(session);
    }
    sessions.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    if sessions
        .windows(2)
        .any(|pair| pair[0].session_id == pair[1].session_id)
    {
        return Err(invalid("mission sessions must be unique"));
    }
    let mut admissions = Vec::with_capacity(sessions.len());
    for session in &sessions {
        admissions.push(session.open_admission()?);
    }
    for (session, admission) in sessions.iter().zip(&admissions) {
        require_fresh(session, admission)?;
    }
    let members = sessions.iter().map(member_from_session).collect::<Vec<_>>();
    drop(admissions);
    Ok(members)
}

fn require_live_members(members: &[MissionMemberAuthority]) -> Result<(), ClewError> {
    let mut sessions = Vec::with_capacity(members.len());
    for member in members {
        // Mission inspection and closure are authority checks, not new semantic
        // work. They must remain available after the launcher advances to a
        // newer capsule, while `open` and `record` still require the active
        // runtime to match exactly.
        let (session, _) = SessionAuthority::load_for_cleanup(&member.session_id)?;
        require_same_member(member, &session)?;
        sessions.push(session);
    }
    let mut admissions = Vec::with_capacity(sessions.len());
    for session in &sessions {
        admissions.push(session.open_admission()?);
    }
    for (session, admission) in sessions.iter().zip(&admissions) {
        require_fresh(session, admission)?;
    }
    Ok(())
}

fn require_fresh(
    session: &SessionAuthority,
    admission: &super::SessionAdmission,
) -> Result<(), ClewError> {
    let freshness = session.freshness_under_admission(admission)?;
    if freshness.status != "FRESH" {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            format!(
                "mission member {} is not fresh: {}",
                session.session_id, freshness.status
            ),
        ));
    }
    Ok(())
}

fn member_from_session(session: &SessionAuthority) -> MissionMemberAuthority {
    MissionMemberAuthority {
        session_id: session.session_id.clone(),
        session_authority_digest: session.authority_digest.clone(),
        repository_key: session.repository_key.clone(),
        base_revision: session.base_revision.clone(),
        target_ref: session.target_ref.clone(),
        target_oid: session.target_oid.clone(),
        runtime_key: session.runtime_key.clone(),
        runtime_mode: session.runtime_mode,
        language: session.language,
        compilations: session.compilations.clone(),
    }
}

fn require_same_member(
    expected: &MissionMemberAuthority,
    actual: &SessionAuthority,
) -> Result<(), ClewError> {
    if expected != &member_from_session(actual) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "mission member authority changed",
        ));
    }
    Ok(())
}

fn validate_members(members: &mut [MissionMemberAuthority]) -> Result<(), ClewError> {
    if members.is_empty() || members.len() > MAX_MISSION_MEMBERS {
        return Err(invalid("mission member count must be between 1 and 8"));
    }
    members.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    if members
        .windows(2)
        .any(|pair| pair[0].session_id == pair[1].session_id)
        || members.iter().any(|member| {
            member.session_id.is_empty()
                || member.session_authority_digest.is_empty()
                || member.compilations.is_empty()
        })
    {
        return Err(invalid("mission member authority is invalid"));
    }
    Ok(())
}

fn build_binding(
    session: &SessionAuthority,
    context_id: Option<&str>,
    plan_id: Option<&str>,
    run_id: Option<&str>,
) -> Result<MissionBinding, ClewError> {
    let context = context_id
        .map(|context_id| session.load_context(context_id))
        .transpose()?;
    let plan = plan_id
        .map(|plan_id| session.load_plan(plan_id))
        .transpose()?;
    if let (Some(context), Some(plan)) = (&context, &plan)
        && plan.context_id != context.context_id
    {
        return Err(invalid(
            "mission plan is not bound to the requested context",
        ));
    }
    let run = run_id.map(RunRecord::load).transpose()?;
    if let Some(run) = &run {
        let plan = plan
            .as_ref()
            .ok_or_else(|| invalid("mission run requires a bound plan"))?;
        let context = context
            .as_ref()
            .ok_or_else(|| invalid("mission run requires a bound context"))?;
        if run.session_id != session.session_id
            || run.context_id != context.context_id
            || run.plan_id != plan.plan_id
        {
            return Err(invalid(
                "mission run is foreign to the requested session/context/plan",
            ));
        }
    }
    Ok(MissionBinding {
        session_id: session.session_id.clone(),
        context: context.as_ref().map(context_binding).transpose()?,
        plan: plan.as_ref().map(plan_binding).transpose()?,
        run: run.as_ref().map(run_binding).transpose()?,
    })
}

fn context_binding(context: &ContextObject) -> Result<ContextBinding, ClewError> {
    Ok(ContextBinding {
        context_id: context.context_id.clone(),
        authority_digest: canonical::hash(context).map_err(internal)?,
        evidence_digest: context.evidence_digest.clone(),
    })
}

fn plan_binding(plan: &PlanObject) -> Result<PlanBinding, ClewError> {
    Ok(PlanBinding {
        plan_id: plan.plan_id.clone(),
        authority_digest: canonical::hash(plan).map_err(internal)?,
        source_digest: plan.source_digest.clone(),
    })
}

fn run_binding(run: &RunRecord) -> Result<RunBinding, ClewError> {
    let authority_digest = canonical::hash(run).map_err(internal)?;
    let validation_digest = canonical::hash(&json!({
        "schema":"codeclew-mission-run-validation/1.0",
        "runAuthorityDigest":authority_digest,
        "status":run.status,
        "publicationBlocked":run.publication_blocked,
        "failure":run.failure,
    }))
    .map_err(internal)?;
    Ok(RunBinding {
        run_id: run.run_id.clone(),
        authority_digest,
        status: run.status,
        candidate_digest: run
            .candidate_snapshot
            .as_ref()
            .map(|snapshot| snapshot.digest.clone()),
        prepared_authority_digest: run.prepared_authority_digest.clone(),
        validation_digest,
        final_commit: run.final_commit.clone(),
    })
}

fn identity_digest(identity: &MissionIdentity) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":MISSION_IDENTITY_SCHEMA,
        "missionId":identity.mission_id,
        "changeSpecDigest":identity.change_spec_digest,
        "members":identity.members,
        "createdUnixMs":identity.created_unix_ms,
    }))
    .map_err(internal)
}

fn append_event_unlocked(
    state: &StateAuthority,
    root: &Path,
    mission_id: &str,
    events: &[MissionEvent],
    kind: MissionEventKind,
    binding: Option<MissionBinding>,
) -> Result<MissionEvent, ClewError> {
    if events.len() >= MAX_MISSION_EVENTS {
        return Err(invalid("mission event count exceeds 4096"));
    }
    if events.is_empty() && kind != MissionEventKind::Opened
        || !events.is_empty() && kind == MissionEventKind::Opened
        || kind == MissionEventKind::Bound && binding.is_none()
        || kind != MissionEventKind::Bound && binding.is_some()
    {
        return Err(invalid("mission event transition is invalid"));
    }
    if events
        .last()
        .is_some_and(|event| event.kind == MissionEventKind::Closed)
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "closed mission cannot accept events",
        ));
    }
    let sequence = u64::try_from(events.len()).map_err(|_| invalid("mission sequence overflow"))?;
    let previous_event_hash = events.last().map(|event| event.event_hash.clone());
    let created_unix_ms = unix_ms();
    let event_hash = event_hash(
        mission_id,
        sequence,
        previous_event_hash.as_deref(),
        kind,
        binding.as_ref(),
        created_unix_ms,
    )?;
    let event = MissionEvent {
        schema: MISSION_EVENT_SCHEMA.into(),
        mission_id: mission_id.into(),
        sequence,
        previous_event_hash,
        kind,
        binding,
        event_hash,
        created_unix_ms,
    };
    let path = root.join("events").join(format!("{sequence:020}.json"));
    write_managed_json_create_new(state, &path, &event)?;
    Ok(event)
}

fn load_events(
    state: &StateAuthority,
    root: &Path,
    mission_id: &str,
) -> Result<Vec<MissionEvent>, ClewError> {
    let directory = state.directory_at(&root.join("events"))?;
    let mut names = directory
        .entries()?
        .into_iter()
        .filter(|name| !name.to_string_lossy().starts_with(".tmp-"))
        .collect::<Vec<_>>();
    names.sort();
    if names.is_empty() || names.len() > MAX_MISSION_EVENTS {
        return Err(invalid(
            "mission event ledger is empty or exceeds its limit",
        ));
    }
    let mut events = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let expected = format!("{index:020}.json");
        if name != OsStr::new(&expected) {
            return Err(invalid("mission event ledger sequence is invalid"));
        }
        let bytes = directory.read_file(name, MAX_MISSION_EVENT_BYTES)?;
        let event: MissionEvent = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("mission event is truncated or invalid"))?;
        if canonical::bytes(&event).map_err(internal)? != bytes {
            return Err(invalid("mission event bytes are not canonical"));
        }
        validate_event(mission_id, &events, &event)?;
        events.push(event);
    }
    Ok(events)
}

fn validate_event(
    mission_id: &str,
    prior: &[MissionEvent],
    event: &MissionEvent,
) -> Result<(), ClewError> {
    let sequence = u64::try_from(prior.len()).map_err(|_| invalid("mission sequence overflow"))?;
    let previous = prior.last().map(|event| event.event_hash.as_str());
    if event.schema != MISSION_EVENT_SCHEMA
        || event.mission_id != mission_id
        || event.sequence != sequence
        || event.previous_event_hash.as_deref() != previous
        || event.event_hash
            != event_hash(
                mission_id,
                event.sequence,
                previous,
                event.kind,
                event.binding.as_ref(),
                event.created_unix_ms,
            )?
        || prior.is_empty() && event.kind != MissionEventKind::Opened
        || !prior.is_empty() && event.kind == MissionEventKind::Opened
        || event.kind == MissionEventKind::Bound && event.binding.is_none()
        || event.kind != MissionEventKind::Bound && event.binding.is_some()
        || prior
            .last()
            .is_some_and(|prior| prior.kind == MissionEventKind::Closed)
    {
        return Err(invalid("mission event authority is invalid"));
    }
    Ok(())
}

fn event_hash(
    mission_id: &str,
    sequence: u64,
    previous_event_hash: Option<&str>,
    kind: MissionEventKind,
    binding: Option<&MissionBinding>,
    created_unix_ms: u128,
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":MISSION_EVENT_SCHEMA,
        "missionId":mission_id,
        "sequence":sequence,
        "previousEventHash":previous_event_hash,
        "kind":kind,
        "binding":binding,
        "createdUnixMs":created_unix_ms,
    }))
    .map_err(internal)
}

fn derive_status(
    identity: &MissionIdentity,
    events: &[MissionEvent],
) -> Result<MissionStatus, ClewError> {
    let event_head = events
        .last()
        .map(|event| event.event_hash.clone())
        .ok_or_else(|| invalid("mission event ledger is empty"))?;
    let lifecycle = if events
        .last()
        .is_some_and(|event| event.kind == MissionEventKind::Closed)
    {
        MissionLifecycle::Closed
    } else {
        MissionLifecycle::Open
    };
    let binding_count = events
        .iter()
        .filter(|event| event.kind == MissionEventKind::Bound)
        .count();
    let readiness = match (lifecycle, binding_count) {
        (MissionLifecycle::Open, 0) => "OPEN_UNLINKED",
        (MissionLifecycle::Open, _) => "OPEN_RECORDED",
        (MissionLifecycle::Closed, 0) => "CLOSED_UNLINKED",
        (MissionLifecycle::Closed, _) => "CLOSED_RECORDED",
    };
    let authority_digest = canonical::hash(&json!({
        "schema":MISSION_STATUS_SCHEMA,
        "missionId":identity.mission_id,
        "identityDigest":identity.identity_digest,
        "eventHead":event_head,
        "eventCount":events.len(),
        "lifecycle":lifecycle,
    }))
    .map_err(internal)?;
    Ok(MissionStatus {
        schema: MISSION_STATUS_SCHEMA.into(),
        mission_id: identity.mission_id.clone(),
        authority_digest,
        identity_digest: identity.identity_digest.clone(),
        change_spec_digest: identity.change_spec_digest.clone(),
        event_head,
        event_count: events.len(),
        binding_count,
        member_count: identity.members.len(),
        lifecycle,
        readiness: readiness.into(),
    })
}

fn inspection(loaded: &LoadedMission) -> MissionInspection {
    let mut latest = BTreeMap::<String, MissionBinding>::new();
    for event in &loaded.events {
        if let Some(binding) = &event.binding {
            latest.insert(binding.session_id.clone(), binding.clone());
        }
    }
    MissionInspection {
        schema: MISSION_INSPECTION_SCHEMA.into(),
        status: loaded.status.clone(),
        change_spec: ChangeSpecSummary {
            requirement_count: loaded.spec.requirements.len(),
            non_goal_count: loaded.spec.non_goals.len(),
            acceptance_criterion_count: loaded.spec.acceptance_criteria.len(),
            documented_requirement_count: loaded.spec.docs_policy.required_requirement_ids.len(),
        },
        members: loaded.identity.members.clone(),
        latest_bindings: latest.into_values().collect(),
    }
}

fn require_open(loaded: &LoadedMission) -> Result<(), ClewError> {
    if loaded.status.lifecycle != MissionLifecycle::Open {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "mission is closed",
        ));
    }
    Ok(())
}

struct MissionLock(File);

impl MissionLock {
    fn acquire(state: &StateAuthority, root: &Path) -> Result<Self, ClewError> {
        let directory = state.directory_at(root)?;
        let file = directory.open_lock(OsStr::new("mission.lock"))?;
        #[cfg(unix)]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(super::io_error(std::io::Error::last_os_error()));
        }
        Ok(Self(file))
    }
}

impl Drop for MissionLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn canonical_spec() -> Vec<u8> {
        canonical::bytes(&ChangeSpec {
            schema: CHANGE_SPEC_SCHEMA.into(),
            intent: "Keep a durable development record".into(),
            requirements: vec![ChangeSpecItem {
                id: "REQ-1".into(),
                text: "Bind exact authorities".into(),
            }],
            non_goals: vec![ChangeSpecItem {
                id: "NG-1".into(),
                text: "Do not replace transactions".into(),
            }],
            acceptance_criteria: vec![ChangeSpecItem {
                id: "AC-1".into(),
                text: "Replay is deterministic".into(),
            }],
            docs_policy: DocsPolicy {
                required_requirement_ids: vec!["REQ-1".into()],
            },
        })
        .unwrap()
    }

    fn member(suffix: &str) -> MissionMemberAuthority {
        MissionMemberAuthority {
            session_id: format!("session:{suffix}"),
            session_authority_digest: format!("sha256:{suffix:0<64}"),
            repository_key: format!("sha256:{:0<64}", "repo"),
            base_revision: format!("{:0<40}", "base"),
            target_ref: "refs/heads/feature".into(),
            target_oid: format!("{:0<40}", "base"),
            runtime_key: format!("sha256:{:0<64}", "runtime"),
            runtime_mode: RuntimeMode::Release,
            language: SessionLanguage::Rust,
            compilations: vec!["cargo:Cargo.toml#demo#lib#demo".into()],
        }
    }

    fn binding(session_id: &str, suffix: &str) -> MissionBinding {
        MissionBinding {
            session_id: session_id.into(),
            context: Some(ContextBinding {
                context_id: format!("context:sha256:{suffix:0<64}"),
                authority_digest: format!("sha256:{suffix:0<64}"),
                evidence_digest: format!("sha256:{:0<64}", "evidence"),
            }),
            plan: None,
            run: None,
        }
    }

    fn test_state() -> (tempfile::TempDir, StateAuthority) {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("state")).unwrap();
        (temporary, state)
    }

    #[test]
    fn changespec_requires_canonical_bytes_and_stable_ids() {
        let source = canonical_spec();
        assert!(parse_change_spec(&source).is_ok());
        let mut with_newline = source.clone();
        with_newline.push(b'\n');
        assert!(parse_change_spec(&with_newline).is_err());
        let duplicate = ChangeSpec {
            schema: CHANGE_SPEC_SCHEMA.into(),
            intent: "intent".into(),
            requirements: vec![ChangeSpecItem {
                id: "same".into(),
                text: "requirement".into(),
            }],
            non_goals: vec![],
            acceptance_criteria: vec![ChangeSpecItem {
                id: "same".into(),
                text: "criterion".into(),
            }],
            docs_policy: DocsPolicy {
                required_requirement_ids: vec![],
            },
        };
        assert!(parse_change_spec(&canonical::bytes(&duplicate).unwrap()).is_err());
    }

    #[test]
    fn replay_is_byte_identical_and_close_changes_authority() {
        let (_temporary, state) = test_state();
        let source = canonical_spec();
        let spec = parse_change_spec(&source).unwrap();
        let opened = create_with_state(&state, &source, spec, vec![member("one")]).unwrap();
        let first = load_with_state(&state, &opened.status.mission_id).unwrap();
        let second = load_with_state(&state, &opened.status.mission_id).unwrap();
        assert_eq!(
            canonical::bytes(&first.status).unwrap(),
            canonical::bytes(&second.status).unwrap()
        );
        let root = state.mission_root(&opened.status.mission_id).unwrap();
        let _lock = MissionLock::acquire(&state, &root).unwrap();
        append_event_unlocked(
            &state,
            &root,
            &opened.status.mission_id,
            &first.events,
            MissionEventKind::Closed,
            None,
        )
        .unwrap();
        let closed = load_with_state_unlocked(&state, &opened.status.mission_id).unwrap();
        assert_ne!(
            first.status.authority_digest,
            closed.status.authority_digest
        );
        assert_eq!(closed.status.lifecycle, MissionLifecycle::Closed);
    }

    #[test]
    fn duplicate_binding_is_detectable_without_an_extra_event() {
        let (_temporary, state) = test_state();
        let source = canonical_spec();
        let opened = create_with_state(
            &state,
            &source,
            parse_change_spec(&source).unwrap(),
            vec![member("one")],
        )
        .unwrap();
        let root = state.mission_root(&opened.status.mission_id).unwrap();
        let _lock = MissionLock::acquire(&state, &root).unwrap();
        let loaded = load_with_state_unlocked(&state, &opened.status.mission_id).unwrap();
        let binding = binding("session:one", "context");
        append_event_unlocked(
            &state,
            &root,
            &opened.status.mission_id,
            &loaded.events,
            MissionEventKind::Bound,
            Some(binding.clone()),
        )
        .unwrap();
        let loaded = load_with_state_unlocked(&state, &opened.status.mission_id).unwrap();
        assert!(loaded.events.iter().any(|event| {
            event.kind == MissionEventKind::Bound && event.binding.as_ref() == Some(&binding)
        }));
        assert_eq!(loaded.status.event_count, 2);
    }

    #[test]
    fn concurrent_appends_are_serialized_into_one_chain() {
        let (_temporary, state) = test_state();
        let source = canonical_spec();
        let opened = create_with_state(
            &state,
            &source,
            parse_change_spec(&source).unwrap(),
            vec![member("one"), member("two")],
        )
        .unwrap();
        let mission_id = opened.status.mission_id;
        let state = Arc::new(state);
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for (session_id, suffix) in [("session:one", "one"), ("session:two", "two")] {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            let mission_id = mission_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                let root = state.mission_root(&mission_id).unwrap();
                let _lock = MissionLock::acquire(&state, &root).unwrap();
                let loaded = load_with_state_unlocked(&state, &mission_id).unwrap();
                append_event_unlocked(
                    &state,
                    &root,
                    &mission_id,
                    &loaded.events,
                    MissionEventKind::Bound,
                    Some(binding(session_id, suffix)),
                )
                .unwrap();
            }));
        }
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }
        let loaded = load_with_state(&state, &mission_id).unwrap();
        assert_eq!(loaded.status.event_count, 3);
        assert_eq!(loaded.status.binding_count, 2);
    }

    #[test]
    fn truncated_event_fails_closed() {
        let (_temporary, state) = test_state();
        let source = canonical_spec();
        let opened = create_with_state(
            &state,
            &source,
            parse_change_spec(&source).unwrap(),
            vec![member("one")],
        )
        .unwrap();
        let root = state.mission_root(&opened.status.mission_id).unwrap();
        state
            .write_private_atomic(&root.join("events/00000000000000000000.json"), b"{")
            .unwrap();
        assert!(load_with_state(&state, &opened.status.mission_id).is_err());
    }

    #[test]
    fn foreign_session_binding_is_rejected_by_member_lookup() {
        let members = [member("one")];
        assert!(
            !members
                .iter()
                .any(|member| member.session_id == "session:foreign")
        );
    }

    #[test]
    fn inspection_is_path_free_and_bounded() {
        let (_temporary, state) = test_state();
        let source = canonical_spec();
        let opened = create_with_state(
            &state,
            &source,
            parse_change_spec(&source).unwrap(),
            vec![member("one")],
        )
        .unwrap();
        let bytes = canonical::bytes(&opened).unwrap();
        assert!(bytes.len() < 64 * 1024);
        assert!(!String::from_utf8(bytes).unwrap().contains("/tmp/"));
    }
}
