use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::session::mission::{self, MissionInspection, MissionLifecycle, MissionMemberAuthority};
use crate::session::{SESSION_SCHEMA, SessionAuthority};
use crate::state::StateAuthority;
use crate::thread::{self, ThreadAuthority, ThreadMemberBinding, ThreadStatus};
use crate::thread_context::{
    MAX_THREAD_STDOUT_BYTES, bounded_thread_context_stdout, create as create_thread_context,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::fd::AsRawFd;

pub const WORKSPACE_CATALOG_SCHEMA: &str = "codeclew-workspace-catalog-input/1.0";
pub const WORKSPACE_AUTHORITY_SCHEMA: &str = "codeclew-workspace-authority/1.0";
pub const WORKSPACE_LIFECYCLE_SCHEMA: &str = "codeclew-workspace-lifecycle-entry/1.0";
pub const WORKSPACE_INSPECTION_SCHEMA: &str = "codeclew-workspace-inspection/1.0";
pub const WORKSPACE_CONTEXT_RESULT_SCHEMA: &str = "codeclew-workspace-context-result/1.0";
pub const MAX_WORKSPACE_CATALOG_BYTES: usize = 256 * 1024;
const MAX_WORKSPACE_AUTHORITY_BYTES: usize = 1024 * 1024;
const MAX_WORKSPACE_LIFECYCLE_BYTES: usize = 128 * 1024;
const MIN_WORKSPACE_MEMBERS: usize = 2;
const MAX_WORKSPACE_MEMBERS: usize = 4;
const MAX_WORKSPACE_EDGES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCatalogInput {
    pub schema: String,
    pub mission_id: String,
    pub members: Vec<WorkspaceCatalogMember>,
    #[serde(default)]
    pub edges: Vec<WorkspaceCatalogEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCatalogMember {
    pub alias: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCatalogEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkspaceEvidenceAuthority {
    DeclaredCatalog,
    CompilerShape,
    VerifiedArtifactOwnership,
    ContractVerified,
    ObservedRuntime,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCertaintyAxes {
    pub topology: WorkspaceEvidenceAuthority,
    pub compiler_shape: WorkspaceEvidenceAuthority,
    pub artifact_ownership: WorkspaceEvidenceAuthority,
    pub contract: WorkspaceEvidenceAuthority,
    pub runtime: WorkspaceEvidenceAuthority,
}

impl WorkspaceCertaintyAxes {
    fn catalog_only() -> Self {
        Self {
            topology: WorkspaceEvidenceAuthority::DeclaredCatalog,
            compiler_shape: WorkspaceEvidenceAuthority::Unknown,
            artifact_ownership: WorkspaceEvidenceAuthority::Unknown,
            contract: WorkspaceEvidenceAuthority::Unknown,
            runtime: WorkspaceEvidenceAuthority::Unknown,
        }
    }

    fn validate_catalog_only(&self) -> Result<(), ClewError> {
        if self != &Self::catalog_only() {
            return Err(invalid(
                "workspace catalog cannot promote compiler, ownership, contract, or runtime authority",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceMemberAuthority {
    pub alias: String,
    pub session: SessionAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceEdgeAuthority {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub certainty: WorkspaceCertaintyAxes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceAuthority {
    pub schema: String,
    pub workspace_id: String,
    pub authority_digest: String,
    pub semantic_digest: String,
    pub mission_id: String,
    pub mission_identity_digest: String,
    pub change_spec_digest: String,
    pub members: Vec<WorkspaceMemberAuthority>,
    pub edges: Vec<WorkspaceEdgeAuthority>,
    pub analysis_thread: ThreadAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkspaceStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceLifecycle {
    pub schema: String,
    pub workspace_id: String,
    pub workspace_authority_digest: String,
    pub sequence: u64,
    pub previous_event_hash: Option<String>,
    pub status: WorkspaceStatus,
    pub event_hash: String,
    pub updated_unix_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceMemberInspection {
    pub alias: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub repository_key: String,
    pub base_revision: String,
    pub language: crate::session::SessionLanguage,
    pub compilations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceInspection {
    pub schema: String,
    pub workspace_id: String,
    pub authority_digest: String,
    pub semantic_digest: String,
    pub mission_id: String,
    pub mission_identity_digest: String,
    pub change_spec_digest: String,
    pub status: WorkspaceStatus,
    pub members: Vec<WorkspaceMemberInspection>,
    pub edges: Vec<WorkspaceEdgeAuthority>,
}

#[derive(Debug)]
struct ResolvedCatalog {
    mission: MissionInspection,
    members: Vec<WorkspaceMemberAuthority>,
    edges: Vec<WorkspaceEdgeAuthority>,
}

/// Resolves only an explicit, private local manifest. It performs no repository
/// discovery and never places local paths into workspace authority.
pub struct WorkspaceCatalogProvider;

impl WorkspaceCatalogProvider {
    fn resolve(source: &[u8]) -> Result<ResolvedCatalog, ClewError> {
        let input = parse_catalog(source)?;
        let mission = mission::inspect(&input.mission_id)?;
        if mission.status.lifecycle != MissionLifecycle::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "workspace mission must be open",
            ));
        }
        if mission.members.len() != input.members.len()
            || mission.members.len() < MIN_WORKSPACE_MEMBERS
            || mission.members.len() > MAX_WORKSPACE_MEMBERS
        {
            return Err(invalid(
                "workspace catalog must cover exactly two to four mission members",
            ));
        }

        let mission_members = mission
            .members
            .iter()
            .map(|member| (member.session_id.as_str(), member))
            .collect::<BTreeMap<_, _>>();
        let mut members = Vec::with_capacity(input.members.len());
        let mut aliases = BTreeSet::new();
        let mut sessions = BTreeSet::new();
        let mut repositories = BTreeSet::new();
        for requested in input.members {
            if !safe_alias(&requested.alias)
                || !aliases.insert(requested.alias.clone())
                || !sessions.insert(requested.session_id.clone())
            {
                return Err(invalid(
                    "workspace member aliases and session bindings must be safe and unique",
                ));
            }
            let mission_member = mission_members
                .get(requested.session_id.as_str())
                .ok_or_else(|| invalid("workspace member is not bound by the mission"))?;
            let (session, _) = SessionAuthority::load(&requested.session_id)?;
            session.require_open()?;
            require_same_mission_member(mission_member, &session)?;
            if !repositories.insert(session.repository_key.clone()) {
                return Err(invalid(
                    "workspace members must belong to distinct local repositories",
                ));
            }
            members.push(WorkspaceMemberAuthority {
                alias: requested.alias,
                session,
            });
        }
        if sessions.len() != mission_members.len() {
            return Err(invalid(
                "workspace catalog must cover every mission member exactly once",
            ));
        }
        members.sort_by(|left, right| left.alias.cmp(&right.alias));

        let edges = resolve_edges(input.edges, &aliases)?;
        Ok(ResolvedCatalog {
            mission,
            members,
            edges,
        })
    }
}

pub fn open(source: &[u8]) -> Result<WorkspaceInspection, ClewError> {
    let resolved = WorkspaceCatalogProvider::resolve(source)?;
    let semantic_digest = semantic_digest(&resolved.mission, &resolved.members, &resolved.edges)?;
    let digest_component = semantic_digest
        .strip_prefix("sha256:")
        .ok_or_else(|| internal("workspace semantic digest domain is invalid"))?;
    let workspace_id = format!("workspace:{digest_component}");
    let state = StateAuthority::process_default()?;
    let analysis_thread = open_analysis_thread(&state, digest_component, &resolved.members)?;

    let mut authority = WorkspaceAuthority {
        schema: WORKSPACE_AUTHORITY_SCHEMA.into(),
        workspace_id,
        authority_digest: String::new(),
        semantic_digest,
        mission_id: resolved.mission.status.mission_id.clone(),
        mission_identity_digest: resolved.mission.status.identity_digest.clone(),
        change_spec_digest: resolved.mission.status.change_spec_digest.clone(),
        members: resolved.members,
        edges: resolved.edges,
        analysis_thread,
    };
    authority.authority_digest = authority_digest(&authority)?;
    validate_authority(&authority)?;
    let root = state.workspace_root(&authority.workspace_id)?;
    let directory = state.directory_at(&root)?;
    let authority_bytes = canonical::bytes(&authority).map_err(internal)?;
    if !directory.atomic_create(std::ffi::OsStr::new("authority.json"), &authority_bytes)? {
        let existing =
            state.read_private_file(&root.join("authority.json"), MAX_WORKSPACE_AUTHORITY_BYTES)?;
        if existing != authority_bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "workspace authority already exists with different content",
            ));
        }
    }

    let _lock = WorkspaceLifecycleLock::acquire(&state, &root)?;
    if !state.private_file_exists(&root.join("lifecycle.jsonl"))? {
        append_lifecycle(
            &state,
            &root,
            WorkspaceLifecycle {
                schema: WORKSPACE_LIFECYCLE_SCHEMA.into(),
                workspace_id: authority.workspace_id.clone(),
                workspace_authority_digest: authority.authority_digest.clone(),
                sequence: 0,
                previous_event_hash: None,
                status: WorkspaceStatus::Open,
                event_hash: String::new(),
                updated_unix_ms: unix_ms(),
            },
        )?;
    }
    let lifecycle = load_lifecycle_unlocked(&state, &root, &authority)?;
    if lifecycle.status != WorkspaceStatus::Open {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "identical workspace authority is already closed",
        ));
    }
    Ok(inspection(&authority, lifecycle.status))
}

pub fn inspect(workspace_id: &str) -> Result<WorkspaceInspection, ClewError> {
    let state = StateAuthority::process_default()?;
    let (authority, root) = load_with_state(&state, workspace_id)?;
    let lifecycle = load_lifecycle(&state, &root, &authority)?;
    if lifecycle.status == WorkspaceStatus::Open {
        verify_live_authorities(&authority)?;
    }
    Ok(inspection(&authority, lifecycle.status))
}

pub fn context(
    workspace_id: &str,
    intent: &str,
    terms: &[String],
    max_roots: usize,
) -> Result<Value, ClewError> {
    let state = StateAuthority::process_default()?;
    let (authority, root) = load_with_state(&state, workspace_id)?;
    let _lock = WorkspaceLifecycleLock::acquire(&state, &root)?;
    let lifecycle = load_lifecycle_unlocked(&state, &root, &authority)?;
    if lifecycle.status != WorkspaceStatus::Open {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "workspace is closed and cannot accept new contexts",
        ));
    }
    verify_live_authorities(&authority)?;
    let context = create_thread_context(&authority.analysis_thread, intent, terms, max_roots)?;
    let mut result = bounded_thread_context_stdout(&context)?;
    let object = result
        .as_object_mut()
        .ok_or_else(|| internal("thread context result is not an object"))?;
    object.insert(
        "schema".into(),
        Value::String(WORKSPACE_CONTEXT_RESULT_SCHEMA.into()),
    );
    object.insert(
        "workspaceId".into(),
        Value::String(authority.workspace_id.clone()),
    );
    object.insert(
        "workspaceAuthorityDigest".into(),
        Value::String(authority.authority_digest.clone()),
    );
    object.insert(
        "missionIdentityDigest".into(),
        Value::String(authority.mission_identity_digest.clone()),
    );
    object.insert(
        "changeSpecDigest".into(),
        Value::String(authority.change_spec_digest.clone()),
    );
    object.insert(
        "declaredEdges".into(),
        serde_json::to_value(&authority.edges).map_err(internal)?,
    );
    let bytes = canonical::bytes(&result).map_err(internal)?;
    if bytes.len().saturating_add(1) > MAX_THREAD_STDOUT_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "workspace context stdout exceeds the shared 64 KiB budget",
        ));
    }
    Ok(result)
}

pub fn close(workspace_id: &str) -> Result<WorkspaceInspection, ClewError> {
    let state = StateAuthority::process_default()?;
    let (authority, root) = load_with_state(&state, workspace_id)?;
    let _lock = WorkspaceLifecycleLock::acquire(&state, &root)?;
    let current = load_lifecycle_unlocked(&state, &root, &authority)?;
    if current.status == WorkspaceStatus::Closed {
        return Ok(inspection(&authority, WorkspaceStatus::Closed));
    }

    match authority.analysis_thread.lifecycle()?.status {
        ThreadStatus::Open => {
            authority.analysis_thread.close()?;
        }
        ThreadStatus::Closed => {}
        ThreadStatus::GarbageCollected => {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "workspace analysis authority was collected before workspace close",
            ));
        }
    }
    append_lifecycle(
        &state,
        &root,
        WorkspaceLifecycle {
            schema: WORKSPACE_LIFECYCLE_SCHEMA.into(),
            workspace_id: authority.workspace_id.clone(),
            workspace_authority_digest: authority.authority_digest.clone(),
            sequence: current.sequence + 1,
            previous_event_hash: Some(current.event_hash),
            status: WorkspaceStatus::Closed,
            event_hash: String::new(),
            updated_unix_ms: unix_ms(),
        },
    )?;
    Ok(inspection(&authority, WorkspaceStatus::Closed))
}

fn parse_catalog(source: &[u8]) -> Result<WorkspaceCatalogInput, ClewError> {
    if source.is_empty() || source.len() > MAX_WORKSPACE_CATALOG_BYTES {
        return Err(invalid("workspace catalog is empty or exceeds 256 KiB"));
    }
    let value: Value = serde_json::from_slice(source)
        .map_err(|_| invalid("workspace catalog is not valid JSON"))?;
    if canonical::bytes(&value).map_err(internal)? != source {
        return Err(invalid(
            "workspace catalog must be canonical compact JSON with NFC strings",
        ));
    }
    let input: WorkspaceCatalogInput = serde_json::from_value(value)
        .map_err(|_| invalid("workspace catalog schema is invalid"))?;
    if input.schema != WORKSPACE_CATALOG_SCHEMA {
        return Err(invalid("workspace catalog schema is unsupported"));
    }
    if input.members.len() < MIN_WORKSPACE_MEMBERS
        || input.members.len() > MAX_WORKSPACE_MEMBERS
        || input.edges.len() > MAX_WORKSPACE_EDGES
    {
        return Err(invalid(
            "workspace catalog must contain two to four members and at most 32 edges",
        ));
    }
    Ok(input)
}

fn resolve_edges(
    edges: Vec<WorkspaceCatalogEdge>,
    aliases: &BTreeSet<String>,
) -> Result<Vec<WorkspaceEdgeAuthority>, ClewError> {
    let mut resolved = Vec::with_capacity(edges.len());
    let mut ids = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for edge in edges {
        if !safe_alias(&edge.id)
            || !safe_alias(&edge.source)
            || !safe_alias(&edge.target)
            || !safe_alias(&edge.relation)
            || edge.source == edge.target
            || !aliases.contains(&edge.source)
            || !aliases.contains(&edge.target)
            || !ids.insert(edge.id.clone())
            || !identities.insert((
                edge.source.clone(),
                edge.target.clone(),
                edge.relation.clone(),
            ))
        {
            return Err(invalid(
                "workspace dependency edges must be safe, unique, non-self, and member-bound",
            ));
        }
        resolved.push(WorkspaceEdgeAuthority {
            id: edge.id,
            source: edge.source,
            target: edge.target,
            relation: edge.relation,
            certainty: WorkspaceCertaintyAxes::catalog_only(),
        });
    }
    resolved.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(resolved)
}

fn semantic_digest(
    mission: &MissionInspection,
    members: &[WorkspaceMemberAuthority],
    edges: &[WorkspaceEdgeAuthority],
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-workspace-semantic-authority/1.0",
        "missionId":mission.status.mission_id,
        "missionIdentityDigest":mission.status.identity_digest,
        "changeSpecDigest":mission.status.change_spec_digest,
        "members":members,
        "edges":edges,
    }))
    .map_err(internal)
}

fn authority_digest(authority: &WorkspaceAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn validate_authority(authority: &WorkspaceAuthority) -> Result<(), ClewError> {
    if authority.schema != WORKSPACE_AUTHORITY_SCHEMA
        || !safe_workspace_id(&authority.workspace_id)
        || authority.members.len() < MIN_WORKSPACE_MEMBERS
        || authority.members.len() > MAX_WORKSPACE_MEMBERS
        || authority.edges.len() > MAX_WORKSPACE_EDGES
    {
        return Err(invalid("workspace authority shape is invalid"));
    }
    let mut aliases = BTreeSet::new();
    let mut sessions = BTreeSet::new();
    let mut repositories = BTreeSet::new();
    let mut previous_alias: Option<&str> = None;
    for member in &authority.members {
        if !safe_alias(&member.alias)
            || previous_alias.is_some_and(|previous| previous >= member.alias.as_str())
            || !aliases.insert(member.alias.as_str())
            || !sessions.insert(member.session.session_id.as_str())
            || !repositories.insert(member.session.repository_key.as_str())
            || member.session.schema != SESSION_SCHEMA
            || member.session.authority_digest != embedded_session_digest(&member.session)?
        {
            return Err(invalid("workspace member authority is invalid"));
        }
        previous_alias = Some(&member.alias);
    }
    let mut previous_edge: Option<&str> = None;
    let mut edge_ids = BTreeSet::new();
    let mut edge_identities = BTreeSet::new();
    for edge in &authority.edges {
        edge.certainty.validate_catalog_only()?;
        if !safe_alias(&edge.id)
            || !safe_alias(&edge.source)
            || !safe_alias(&edge.target)
            || !safe_alias(&edge.relation)
            || edge.source == edge.target
            || !aliases.contains(edge.source.as_str())
            || !aliases.contains(edge.target.as_str())
            || previous_edge.is_some_and(|previous| previous >= edge.id.as_str())
            || !edge_ids.insert(edge.id.as_str())
            || !edge_identities.insert((
                edge.source.as_str(),
                edge.target.as_str(),
                edge.relation.as_str(),
            ))
        {
            return Err(invalid("workspace edge authority is invalid"));
        }
        previous_edge = Some(&edge.id);
    }
    authority.analysis_thread.verify()?;
    let expected_thread_members = thread_members(&authority.members);
    if canonical::bytes(&expected_thread_members).map_err(internal)?
        != canonical::bytes(&authority.analysis_thread.members).map_err(internal)?
    {
        return Err(invalid(
            "workspace analysis thread does not bind the exact member authorities",
        ));
    }
    if authority.authority_digest != authority_digest(authority)? {
        return Err(invalid("workspace authority digest is invalid"));
    }
    Ok(())
}

fn open_analysis_thread(
    state: &StateAuthority,
    digest_component: &str,
    members: &[WorkspaceMemberAuthority],
) -> Result<ThreadAuthority, ClewError> {
    let thread_id = format!("thread:workspace-{digest_component}");
    let root = state.thread_root(&thread_id)?;
    let requested_members = thread_members(members);
    if state.private_file_exists(&root.join("authority.json"))? {
        let (existing, _) = thread::load_with_state(state, &thread_id)?;
        if canonical::bytes(&existing.members).map_err(internal)?
            != canonical::bytes(&requested_members).map_err(internal)?
            || existing.lifecycle()?.status != ThreadStatus::Open
        {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "workspace analysis authority is unavailable or changed",
            ));
        }
        return Ok(existing);
    }
    thread::create_with_state(state, thread_id, 0, requested_members)
}

fn thread_members(members: &[WorkspaceMemberAuthority]) -> Vec<ThreadMemberBinding> {
    members
        .iter()
        .map(|member| ThreadMemberBinding {
            member_alias: member.alias.clone(),
            service_alias: member.alias.clone(),
            session: member.session.clone(),
        })
        .collect()
}

fn verify_live_authorities(authority: &WorkspaceAuthority) -> Result<(), ClewError> {
    let mission = mission::inspect(&authority.mission_id)?;
    if mission.status.lifecycle != MissionLifecycle::Open
        || mission.status.identity_digest != authority.mission_identity_digest
        || mission.status.change_spec_digest != authority.change_spec_digest
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "workspace mission authority changed or became terminal",
        ));
    }
    for member in &authority.members {
        let mission_member = mission
            .members
            .iter()
            .find(|candidate| candidate.session_id == member.session.session_id)
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::BindingChanged,
                    "workspace mission member disappeared",
                )
            })?;
        require_same_mission_member(mission_member, &member.session)?;
    }
    let (thread, _) = ThreadAuthority::load(&authority.analysis_thread.thread_id)?;
    if canonical::bytes(&thread).map_err(internal)?
        != canonical::bytes(&authority.analysis_thread).map_err(internal)?
        || thread.lifecycle()?.status != ThreadStatus::Open
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "workspace analysis authority changed or became terminal",
        ));
    }
    Ok(())
}

fn require_same_mission_member(
    expected: &MissionMemberAuthority,
    session: &SessionAuthority,
) -> Result<(), ClewError> {
    if expected.session_id != session.session_id
        || expected.session_authority_digest != session.authority_digest
        || expected.repository_key != session.repository_key
        || expected.base_revision != session.base_revision
        || expected.target_ref != session.target_ref
        || expected.target_oid != session.target_oid
        || expected.runtime_key != session.runtime_key
        || expected.runtime_mode != session.runtime_mode
        || expected.language != session.language
        || expected.compilations != session.compilations
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "workspace session no longer matches mission authority",
        ));
    }
    Ok(())
}

fn embedded_session_digest(session: &SessionAuthority) -> Result<String, ClewError> {
    let mut unsigned = session.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn inspection(authority: &WorkspaceAuthority, status: WorkspaceStatus) -> WorkspaceInspection {
    WorkspaceInspection {
        schema: WORKSPACE_INSPECTION_SCHEMA.into(),
        workspace_id: authority.workspace_id.clone(),
        authority_digest: authority.authority_digest.clone(),
        semantic_digest: authority.semantic_digest.clone(),
        mission_id: authority.mission_id.clone(),
        mission_identity_digest: authority.mission_identity_digest.clone(),
        change_spec_digest: authority.change_spec_digest.clone(),
        status,
        members: authority
            .members
            .iter()
            .map(|member| WorkspaceMemberInspection {
                alias: member.alias.clone(),
                session_id: member.session.session_id.clone(),
                session_authority_digest: member.session.authority_digest.clone(),
                repository_key: member.session.repository_key.clone(),
                base_revision: member.session.base_revision.clone(),
                language: member.session.language,
                compilations: member.session.compilations.clone(),
            })
            .collect(),
        edges: authority.edges.clone(),
    }
}

fn load_with_state(
    state: &StateAuthority,
    workspace_id: &str,
) -> Result<(WorkspaceAuthority, std::path::PathBuf), ClewError> {
    let root = state.workspace_root(workspace_id)?;
    let authority: WorkspaceAuthority = read_canonical_json(
        state,
        &root.join("authority.json"),
        MAX_WORKSPACE_AUTHORITY_BYTES,
    )?;
    if authority.workspace_id != workspace_id {
        return Err(invalid("workspace authority identity is invalid"));
    }
    validate_authority(&authority)?;
    load_lifecycle(state, &root, &authority)?;
    Ok((authority, root))
}

fn load_lifecycle(
    state: &StateAuthority,
    root: &Path,
    authority: &WorkspaceAuthority,
) -> Result<WorkspaceLifecycle, ClewError> {
    let _lock = WorkspaceLifecycleLock::acquire(state, root)?;
    load_lifecycle_unlocked(state, root, authority)
}

fn load_lifecycle_unlocked(
    state: &StateAuthority,
    root: &Path,
    authority: &WorkspaceAuthority,
) -> Result<WorkspaceLifecycle, ClewError> {
    let bytes =
        state.read_private_file(&root.join("lifecycle.jsonl"), MAX_WORKSPACE_LIFECYCLE_BYTES)?;
    let mut previous: Option<WorkspaceLifecycle> = None;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: WorkspaceLifecycle = serde_json::from_slice(line)
            .map_err(|_| invalid("workspace lifecycle ledger is invalid"))?;
        if canonical::bytes(&entry).map_err(internal)? != line
            || entry.schema != WORKSPACE_LIFECYCLE_SCHEMA
            || entry.workspace_id != authority.workspace_id
            || entry.workspace_authority_digest != authority.authority_digest
            || entry.event_hash != lifecycle_hash(&entry)?
        {
            return Err(invalid("workspace lifecycle authority is invalid"));
        }
        match previous.as_ref() {
            None => {
                if entry.sequence != 0
                    || entry.previous_event_hash.is_some()
                    || entry.status != WorkspaceStatus::Open
                {
                    return Err(invalid("workspace lifecycle origin is invalid"));
                }
            }
            Some(prior) => {
                if entry.sequence != prior.sequence + 1
                    || entry.previous_event_hash.as_deref() != Some(prior.event_hash.as_str())
                    || prior.status != WorkspaceStatus::Open
                    || entry.status != WorkspaceStatus::Closed
                {
                    return Err(invalid("workspace lifecycle transition is invalid"));
                }
            }
        }
        previous = Some(entry);
    }
    previous.ok_or_else(|| invalid("workspace lifecycle ledger is empty"))
}

fn append_lifecycle(
    state: &StateAuthority,
    root: &Path,
    mut entry: WorkspaceLifecycle,
) -> Result<(), ClewError> {
    entry.event_hash = lifecycle_hash(&entry)?;
    let mut bytes = canonical::bytes(&entry).map_err(internal)?;
    bytes.push(b'\n');
    let path = root.join("lifecycle.jsonl");
    let existing = if state.private_file_exists(&path)? {
        state.read_private_file(&path, MAX_WORKSPACE_LIFECYCLE_BYTES)?
    } else {
        Vec::new()
    };
    if existing.len().saturating_add(bytes.len()) > MAX_WORKSPACE_LIFECYCLE_BYTES {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "workspace lifecycle ledger exceeds its bound",
        ));
    }
    let mut file = state.open_private_append(&path)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn lifecycle_hash(entry: &WorkspaceLifecycle) -> Result<String, ClewError> {
    let mut unsigned = entry.clone();
    unsigned.event_hash.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn read_canonical_json<T: for<'de> Deserialize<'de> + Serialize>(
    state: &StateAuthority,
    path: &Path,
    limit: usize,
) -> Result<T, ClewError> {
    let bytes = state.read_private_file(path, limit)?;
    let value: T =
        serde_json::from_slice(&bytes).map_err(|_| invalid("managed workspace JSON is invalid"))?;
    if canonical::bytes(&value).map_err(internal)? != bytes {
        return Err(invalid("managed workspace JSON is not canonical"));
    }
    Ok(value)
}

struct WorkspaceLifecycleLock(File);

impl WorkspaceLifecycleLock {
    fn acquire(state: &StateAuthority, root: &Path) -> Result<Self, ClewError> {
        let directory = state.directory_at(root)?;
        let file = directory.open_lock(std::ffi::OsStr::new("lifecycle.lock"))?;
        #[cfg(unix)]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self(file))
    }
}

impl Drop for WorkspaceLifecycleLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn safe_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && crate::text_authority::is_nfc(value)
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn safe_workspace_id(value: &str) -> bool {
    value.strip_prefix("workspace:").is_some_and(|component| {
        component.len() == 64 && component.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
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
    use crate::runtime::RuntimeMode;
    use crate::session::{ModelCachePolicy, SessionLanguage};

    fn session(seed: char, repository_key: &str, revision_seed: char) -> SessionAuthority {
        let digest = std::iter::repeat_n(seed, 64).collect::<String>();
        let revision = std::iter::repeat_n(revision_seed, 40).collect::<String>();
        let mut session = SessionAuthority {
            schema: SESSION_SCHEMA.into(),
            authority_digest: String::new(),
            session_id: format!("session:{digest}"),
            repository_key: repository_key.into(),
            base_revision: revision.clone(),
            target_ref: "refs/heads/main".into(),
            target_oid: revision,
            runtime_key: format!("runtime:{digest}"),
            runtime_mode: RuntimeMode::Development,
            language: SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: 1,
        };
        session.authority_digest = embedded_session_digest(&session).unwrap();
        session
    }

    fn member(alias: &str, session: SessionAuthority) -> WorkspaceMemberAuthority {
        WorkspaceMemberAuthority {
            alias: alias.into(),
            session,
        }
    }

    fn edge(id: &str, source: &str, target: &str) -> WorkspaceEdgeAuthority {
        WorkspaceEdgeAuthority {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            relation: "depends-on".into(),
            certainty: WorkspaceCertaintyAxes::catalog_only(),
        }
    }

    fn semantic_for(
        mut members: Vec<WorkspaceMemberAuthority>,
        mut edges: Vec<WorkspaceEdgeAuthority>,
    ) -> String {
        members.sort_by(|left, right| left.alias.cmp(&right.alias));
        edges.sort_by(|left, right| left.id.cmp(&right.id));
        canonical::hash(&json!({
            "schema":"codeclew-workspace-semantic-authority/1.0",
            "missionId":"mission:fixed",
            "missionIdentityDigest":format!("sha256:{}", "1".repeat(64)),
            "changeSpecDigest":format!("sha256:{}", "2".repeat(64)),
            "members":members,
            "edges":edges,
        }))
        .unwrap()
    }

    fn lifecycle_authority() -> WorkspaceAuthority {
        let members = vec![
            member("left", session('a', "repo:left", 'a')),
            member("right", session('b', "repo:right", 'b')),
        ];
        let mut thread = ThreadAuthority {
            schema: crate::thread::THREAD_SCHEMA.into(),
            thread_id: "thread:test".into(),
            authority_digest: String::new(),
            semantic_digest: String::new(),
            members: thread_members(&members),
            created_unix_ms: 0,
        };
        thread.semantic_digest = canonical::hash(&json!({"members":thread.members})).unwrap();
        thread.authority_digest = canonical::hash(&json!({"thread":thread.thread_id})).unwrap();
        WorkspaceAuthority {
            schema: WORKSPACE_AUTHORITY_SCHEMA.into(),
            workspace_id: format!("workspace:{}", "f".repeat(64)),
            authority_digest: format!("sha256:{}", "e".repeat(64)),
            semantic_digest: format!("sha256:{}", "d".repeat(64)),
            mission_id: "mission:test".into(),
            mission_identity_digest: format!("sha256:{}", "c".repeat(64)),
            change_spec_digest: format!("sha256:{}", "b".repeat(64)),
            members,
            edges: vec![],
            analysis_thread: thread,
        }
    }

    #[test]
    fn member_and_edge_order_do_not_change_semantic_authority() {
        let left = member("left", session('a', "repo:left", 'a'));
        let right = member("right", session('b', "repo:right", 'b'));
        let dependency = edge("left-right", "left", "right");
        assert_eq!(
            semantic_for(vec![left.clone(), right.clone()], vec![dependency.clone()]),
            semantic_for(vec![right, left], vec![dependency])
        );
    }

    #[test]
    fn member_revision_and_edge_changes_change_semantic_authority() {
        let left = member("left", session('a', "repo:left", 'a'));
        let right = member("right", session('b', "repo:right", 'b'));
        let baseline = semantic_for(
            vec![left.clone(), right.clone()],
            vec![edge("left-right", "left", "right")],
        );
        let revised = semantic_for(
            vec![left, member("right", session('b', "repo:right", 'c'))],
            vec![edge("left-right", "left", "right")],
        );
        let reversed_edge = semantic_for(
            vec![member("left", session('a', "repo:left", 'a')), right],
            vec![edge("right-left", "right", "left")],
        );
        assert_ne!(baseline, revised);
        assert_ne!(baseline, reversed_edge);
    }

    #[test]
    fn catalog_edge_never_promotes_independent_certainty_axes() {
        let axes = WorkspaceCertaintyAxes::catalog_only();
        axes.validate_catalog_only().unwrap();
        assert_eq!(axes.topology, WorkspaceEvidenceAuthority::DeclaredCatalog);
        assert_eq!(axes.compiler_shape, WorkspaceEvidenceAuthority::Unknown);
        assert_eq!(axes.artifact_ownership, WorkspaceEvidenceAuthority::Unknown);
        assert_eq!(axes.contract, WorkspaceEvidenceAuthority::Unknown);
        assert_eq!(axes.runtime, WorkspaceEvidenceAuthority::Unknown);
    }

    #[test]
    fn catalog_requires_canonical_path_free_identifiers() {
        let canonical = br#"{"edges":[],"members":[{"alias":"left","sessionId":"session:a"},{"alias":"right","sessionId":"session:b"}],"missionId":"mission:fixed","schema":"codeclew-workspace-catalog-input/1.0"}"#;
        let parsed = parse_catalog(canonical).unwrap();
        assert_eq!(parsed.members.len(), 2);

        let pretty = br#"{ "schema": "codeclew-workspace-catalog-input/1.0", "missionId": "mission:fixed", "members": [], "edges": [] }"#;
        assert!(parse_catalog(pretty).is_err());
        assert!(!safe_alias("/private/repository"));
        assert!(!safe_alias("left;touch"));
    }

    #[test]
    fn workspace_lifecycle_is_append_only_and_terminal() {
        let parent = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(parent.path().join("state")).unwrap();
        let authority = lifecycle_authority();
        let root = state.workspace_root(&authority.workspace_id).unwrap();
        append_lifecycle(
            &state,
            &root,
            WorkspaceLifecycle {
                schema: WORKSPACE_LIFECYCLE_SCHEMA.into(),
                workspace_id: authority.workspace_id.clone(),
                workspace_authority_digest: authority.authority_digest.clone(),
                sequence: 0,
                previous_event_hash: None,
                status: WorkspaceStatus::Open,
                event_hash: String::new(),
                updated_unix_ms: 1,
            },
        )
        .unwrap();
        let open = load_lifecycle_unlocked(&state, &root, &authority).unwrap();
        append_lifecycle(
            &state,
            &root,
            WorkspaceLifecycle {
                schema: WORKSPACE_LIFECYCLE_SCHEMA.into(),
                workspace_id: authority.workspace_id.clone(),
                workspace_authority_digest: authority.authority_digest.clone(),
                sequence: 1,
                previous_event_hash: Some(open.event_hash),
                status: WorkspaceStatus::Closed,
                event_hash: String::new(),
                updated_unix_ms: 2,
            },
        )
        .unwrap();
        let closed = load_lifecycle_unlocked(&state, &root, &authority).unwrap();
        assert_eq!(closed.status, WorkspaceStatus::Closed);

        append_lifecycle(
            &state,
            &root,
            WorkspaceLifecycle {
                schema: WORKSPACE_LIFECYCLE_SCHEMA.into(),
                workspace_id: authority.workspace_id.clone(),
                workspace_authority_digest: authority.authority_digest.clone(),
                sequence: 2,
                previous_event_hash: Some(closed.event_hash),
                status: WorkspaceStatus::Closed,
                event_hash: String::new(),
                updated_unix_ms: 3,
            },
        )
        .unwrap();
        assert!(load_lifecycle_unlocked(&state, &root, &authority).is_err());
    }
}
