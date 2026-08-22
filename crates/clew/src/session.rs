use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot::LEGACY_EXCLUDES;
use crate::runtime::{RuntimeAuthority, RuntimeMode};
use crate::state::{StateAuthority, create_private_directory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use walkdir::WalkDir;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub const SESSION_SCHEMA: &str = "codeclew-session/2.0";
pub const CONTEXT_SCHEMA: &str = "codeclew-context/2.0";
pub const PLAN_SCHEMA: &str = "codeclew-plan/2.0";
pub const RUN_SCHEMA: &str = "codeclew-task-run/2.0";
const RUN_LEDGER_SCHEMA: &str = "codeclew-task-run-ledger-entry/2.0";
const MAX_RUN_LEDGER_BYTES: u64 = 32 * 1024 * 1024;
const CONTEXT_EVIDENCE_SCHEMA: &str = "codeclew-context-evidence-object/2.0";
const MAX_CONTEXT_EVIDENCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CONTEXT_STDOUT_BYTES: usize = 64 * 1024;
pub const MAX_PLAN_BYTES: usize = 1024 * 1024;
pub const MAX_PLAN_OPERATIONS: usize = 256;
pub const MAX_PLAN_FILES: usize = 256;
pub const MAX_WRITE_SET_FACTS: usize = 4096;
pub const MAX_WRITE_SET_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionAuthority {
    pub schema: String,
    pub authority_digest: String,
    pub session_id: String,
    pub repository_key: String,
    pub base_revision: String,
    pub target_ref: String,
    pub target_oid: String,
    pub runtime_key: String,
    pub runtime_mode: RuntimeMode,
    pub compilation: String,
    pub model_cache_policy: ModelCachePolicy,
    pub created_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ModelCachePolicy {
    NonCacheable,
    TrackedManifest,
    SealedExternal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextObject {
    pub schema: String,
    pub context_id: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub parent_context_id: Option<String>,
    pub intent: String,
    pub terms: Vec<String>,
    pub evidence_digest: String,
    pub evidence_ref: CasObject,
    pub projection: Value,
    #[serde(skip)]
    pub evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanObject {
    pub schema: String,
    pub plan_id: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub context_id: String,
    pub context_digest: String,
    pub base_revision: String,
    pub runtime_key: String,
    pub source_digest: String,
    pub plan: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRecord {
    pub schema: String,
    pub run_id: String,
    pub session_id: String,
    pub context_id: String,
    pub plan_id: String,
    pub request_digest: String,
    pub sequence: u64,
    pub ledger_head: String,
    pub status: RunStatus,
    pub transaction_id: String,
    pub candidate_commit: Option<String>,
    pub candidate_snapshot: Option<CasObject>,
    pub final_commit: Option<String>,
    pub publication_blocked: bool,
    pub process_id: Option<u32>,
    pub process_start_token: Option<String>,
    pub failure: Option<Value>,
    pub updated_unix_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunLedgerEntry {
    schema: String,
    sequence: u64,
    previous_event_hash: Option<String>,
    record: RunRecord,
    event_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunStatus {
    Created,
    Preparing,
    ReadyToPublish,
    ValidatedConditional,
    Publishing,
    Published,
    Failed,
    WorktreeRecoveryRequired,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryLocator {
    schema: String,
    target_repository_path: PathBuf,
    source_repository_path: PathBuf,
}

impl SessionAuthority {
    pub fn open(
        repo: &Path,
        target_ref: &str,
        compilation: &str,
        model_cache_policy: ModelCachePolicy,
    ) -> Result<Self, ClewError> {
        let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerPreparationRequired,
                "session commands must be launched through ./clew",
            )
        })?;
        let repo = repo.canonicalize().map_err(io_error)?;
        let state = StateAuthority::process_default()?;
        let repository = state.repository(&repo)?;
        let target_ref = qualify_ref(target_ref)?;
        let base_revision = git_output(&repo, &["rev-parse", "HEAD"])?;
        let target_oid = git_output(&repo, &["rev-parse", &target_ref])?;
        if target_oid != base_revision {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session target ref must identify the checked-out base revision",
            ));
        }
        let mut authority = Self {
            schema: SESSION_SCHEMA.into(),
            authority_digest: String::new(),
            session_id: format!("session:{}", Uuid::new_v4()),
            repository_key: repository.key,
            base_revision,
            target_ref,
            target_oid,
            runtime_key: runtime.runtime_key,
            runtime_mode: runtime.mode,
            compilation: compilation.into(),
            model_cache_policy,
            created_unix_ms: unix_ms(),
        };
        authority.authority_digest = session_authority_digest(&authority)?;
        let root = state.session_root(&authority.session_id)?;
        for child in ["objects/sha256", "contexts", "plans", "candidates"] {
            create_private_directory(&root.join(child))?;
        }
        let source_repository_path = root.join("source");
        create_filtered_detached_worktree(
            &repo,
            &source_repository_path,
            &authority.base_revision,
        )?;
        seal_source_worktree(&source_repository_path)?;
        write_json_create_new(&root.join("authority.json"), &authority)?;
        write_json_create_new(
            &root.join("locator.json"),
            &RepositoryLocator {
                schema: "codeclew-repository-locator/2.0".into(),
                target_repository_path: repo,
                source_repository_path,
            },
        )?;
        Ok(authority)
    }

    pub fn load(session_id: &str) -> Result<(Self, PathBuf), ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(session_id)?;
        let authority: Self = read_json_limited(&root.join("authority.json"), MAX_PLAN_BYTES)?;
        if authority.schema != SESSION_SCHEMA
            || authority.session_id != session_id
            || authority.authority_digest != session_authority_digest(&authority)?
        {
            return Err(invalid("session authority identity is invalid"));
        }
        let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerPreparationRequired,
                "session commands must be launched through ./clew",
            )
        })?;
        if runtime.runtime_key != authority.runtime_key || runtime.mode != authority.runtime_mode {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "session runtime authority does not match the active capsule",
            ));
        }
        Ok((authority, root))
    }

    pub fn repository_path(&self) -> Result<PathBuf, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let locator: RepositoryLocator =
            read_json_limited(&root.join("locator.json"), MAX_PLAN_BYTES)?;
        if locator.schema != "codeclew-repository-locator/2.0" {
            return Err(invalid("repository locator schema is invalid"));
        }
        let path = locator
            .source_repository_path
            .canonicalize()
            .map_err(io_error)?;
        if !path.starts_with(&root)
            || git_output(&path, &["rev-parse", "HEAD"])? != self.base_revision
            || !filtered_worktree_clean(&path)?
        {
            return Err(invalid("session source authority is invalid"));
        }
        if state.repository(&path)?.key != self.repository_key {
            return Err(invalid(
                "repository locator no longer matches session authority",
            ));
        }
        Ok(path)
    }

    pub fn target_repository_path(&self) -> Result<PathBuf, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let locator: RepositoryLocator =
            read_json_limited(&root.join("locator.json"), MAX_PLAN_BYTES)?;
        if locator.schema != "codeclew-repository-locator/2.0" {
            return Err(invalid("repository locator schema is invalid"));
        }
        let path = locator
            .target_repository_path
            .canonicalize()
            .map_err(io_error)?;
        if state.repository(&path)?.key != self.repository_key {
            return Err(invalid("target repository locator is invalid"));
        }
        Ok(path)
    }

    pub fn store_context(
        &self,
        parent_context_id: Option<String>,
        intent: String,
        mut terms: Vec<String>,
        projection: Value,
        evidence: Value,
    ) -> Result<ContextObject, ClewError> {
        terms.sort();
        terms.dedup();
        let evidence_bytes = canonical::bytes(&evidence).map_err(internal)?;
        if evidence_bytes.len() > MAX_CONTEXT_EVIDENCE_BYTES {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "context evidence exceeds the 64 MiB CAS object limit",
            ));
        }
        let state = StateAuthority::process_default()?;
        let store = CasStore::open(&state)?;
        let evidence_ref = store.put(CONTEXT_EVIDENCE_SCHEMA, &evidence_bytes)?;
        let evidence_digest = evidence_ref.digest.clone();
        let binding = json!({
            "schema": CONTEXT_SCHEMA,
            "sessionId": self.session_id,
            "sessionAuthorityDigest":self.authority_digest,
            "parentContextId": parent_context_id,
            "intent": intent,
            "terms": terms,
            "evidenceRef": evidence_ref,
        });
        let context_id = format!("context:{}", canonical::hash(&binding).map_err(internal)?);
        let object = ContextObject {
            schema: CONTEXT_SCHEMA.into(),
            context_id,
            session_id: self.session_id.clone(),
            session_authority_digest: self.authority_digest.clone(),
            parent_context_id,
            intent,
            terms,
            evidence_digest,
            evidence_ref,
            projection,
            evidence,
        };
        let root = state.session_root(&self.session_id)?;
        store_cas_json(&state, &root, &object.context_id, &object)?;
        state.write_private_atomic(
            &root.join("contexts").join(id_filename(&object.context_id)?),
            &canonical::bytes(&object).map_err(internal)?,
        )?;
        Ok(object)
    }

    pub fn load_context(&self, context_id: &str) -> Result<ContextObject, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let mut object: ContextObject = read_json_limited(
            &root.join("contexts").join(id_filename(context_id)?),
            MAX_PLAN_BYTES * 2,
        )?;
        if object.schema != CONTEXT_SCHEMA
            || object.context_id != context_id
            || object.session_id != self.session_id
            || object.session_authority_digest != self.authority_digest
            || object.evidence_ref.object_schema != CONTEXT_EVIDENCE_SCHEMA
            || object.evidence_ref.digest != object.evidence_digest
        {
            return Err(invalid("context authority is invalid"));
        }
        let store = CasStore::open(&state)?;
        let limit = usize::try_from(object.evidence_ref.size)
            .map_err(|_| invalid("context evidence exceeds the host size"))?;
        if limit > MAX_CONTEXT_EVIDENCE_BYTES {
            return Err(invalid("context evidence exceeds its CAS object limit"));
        }
        let lease = store.read(&object.evidence_ref, limit)?;
        object.evidence = serde_json::from_slice(lease.bytes())
            .map_err(|_| invalid("context evidence CAS object is invalid"))?;
        if canonical::bytes(&object.evidence).map_err(internal)? != lease.bytes() {
            return Err(invalid("context evidence CAS object is not canonical"));
        }
        Ok(object)
    }

    pub fn validate_plan(&self, context_id: &str, source: &[u8]) -> Result<PlanObject, ClewError> {
        if source.len() > MAX_PLAN_BYTES {
            return Err(invalid("plan exceeds the 1 MiB limit"));
        }
        let context = self.load_context(context_id)?;
        let plan: Value = serde_json::from_slice(source).map_err(parse_error)?;
        validate_plan_shape(&plan)?;
        let source_digest = canonical::hash_bytes(source);
        let context_digest = canonical::hash(&context).map_err(internal)?;
        let binding = json!({
            "schema":PLAN_SCHEMA,
            "sessionId":self.session_id,
            "sessionAuthorityDigest":self.authority_digest,
            "contextId":context_id,
            "contextDigest":context_digest,
            "baseRevision":self.base_revision,
            "runtimeKey":self.runtime_key,
            "sourceDigest":source_digest,
            "plan":plan,
        });
        let plan_id = format!("plan:{}", canonical::hash(&binding).map_err(internal)?);
        let object = PlanObject {
            schema: PLAN_SCHEMA.into(),
            plan_id,
            session_id: self.session_id.clone(),
            session_authority_digest: self.authority_digest.clone(),
            context_id: context_id.into(),
            context_digest,
            base_revision: self.base_revision.clone(),
            runtime_key: self.runtime_key.clone(),
            source_digest,
            plan,
        };
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        store_cas_json(&state, &root, &object.plan_id, &object)?;
        state.write_private_atomic(
            &root.join("plans").join(id_filename(&object.plan_id)?),
            &canonical::bytes(&object).map_err(internal)?,
        )?;
        Ok(object)
    }

    pub fn load_plan(&self, plan_id: &str) -> Result<PlanObject, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let object: PlanObject = read_json_limited(
            &root.join("plans").join(id_filename(plan_id)?),
            MAX_PLAN_BYTES * 2,
        )?;
        if object.schema != PLAN_SCHEMA
            || object.plan_id != plan_id
            || object.session_id != self.session_id
            || object.session_authority_digest != self.authority_digest
            || object.base_revision != self.base_revision
            || object.runtime_key != self.runtime_key
        {
            return Err(invalid("plan authority is invalid"));
        }
        let context = self.load_context(&object.context_id)?;
        if canonical::hash(&context).map_err(internal)? != object.context_digest {
            return Err(invalid("plan context binding is stale"));
        }
        Ok(object)
    }

    pub fn run_identity(
        &self,
        context_id: &str,
        plan_id: &str,
    ) -> Result<(String, String), ClewError> {
        let request = json!({
            "schema":"codeclew-task-run-request/1.0",
            "sessionId":self.session_id,
            "sessionAuthorityDigest":self.authority_digest,
            "contextId":context_id,
            "planId":plan_id,
            "baseRevision":self.base_revision,
            "targetRef":self.target_ref,
            "runtimeKey":self.runtime_key,
        });
        let request_digest = canonical::hash(&request).map_err(internal)?;
        let path_digest = request_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| internal("canonical digest has no sha256 prefix"))?;
        Ok((format!("run:{path_digest}"), request_digest))
    }
}

impl RunRecord {
    pub fn created(
        session: &SessionAuthority,
        context_id: &str,
        plan_id: &str,
    ) -> Result<Self, ClewError> {
        let plan = session.load_plan(plan_id)?;
        if plan.context_id != context_id {
            return Err(invalid("plan is not bound to the requested context"));
        }
        let (run_id, request_digest) = session.run_identity(context_id, plan_id)?;
        let transaction_id = transaction_id(&request_digest)?;
        Ok(Self {
            schema: RUN_SCHEMA.into(),
            transaction_id,
            run_id,
            session_id: session.session_id.clone(),
            context_id: context_id.into(),
            plan_id: plan_id.into(),
            request_digest,
            sequence: 0,
            ledger_head: String::new(),
            status: RunStatus::Created,
            candidate_commit: None,
            candidate_snapshot: None,
            final_commit: None,
            publication_blocked: false,
            process_id: None,
            process_start_token: None,
            failure: None,
            updated_unix_ms: unix_ms(),
        })
    }

    pub fn load(run_id: &str) -> Result<Self, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.run_root(run_id)?;
        let _lock = RunLedgerLock::acquire(&root)?;
        load_run_projection(&state, &root, run_id)
    }

    pub fn save(&mut self) -> Result<(), ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.run_root(&self.run_id)?;
        save_run_transition(&state, &root, self)
    }

    pub fn create_once(&self) -> Result<bool, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.run_root(&self.run_id)?;
        let _lock = RunLedgerLock::acquire(&root)?;
        if root.join("ledger.jsonl").exists() || root.join("record.json").exists() {
            let _ = load_run_projection(&state, &root, &self.run_id)?;
            return Ok(false);
        }
        if self.sequence != 0 || !self.ledger_head.is_empty() || self.status != RunStatus::Created {
            return Err(invalid("initial run ledger record is invalid"));
        }
        let mut initial = self.clone();
        initial.updated_unix_ms = unix_ms();
        append_run_entry(&state, &root, &mut initial, None)?;
        Ok(true)
    }

    pub fn candidate_root(&self) -> Result<PathBuf, ClewError> {
        let (_, session_root) = SessionAuthority::load(&self.session_id)?;
        let root = session_root
            .join("candidates")
            .join(id_component(&self.run_id, "run:")?);
        create_private_directory(&root)?;
        Ok(root)
    }
}

fn save_run_transition(
    state: &StateAuthority,
    root: &Path,
    record: &mut RunRecord,
) -> Result<(), ClewError> {
    let _lock = RunLedgerLock::acquire(root)?;
    let current = load_run_projection(state, root, &record.run_id)?;
    if record.sequence != current.sequence || record.ledger_head != current.ledger_head {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run ledger changed concurrently; reload the run before retrying",
        ));
    }
    require_same_run_identity(&current, record)?;
    if !run_transition_allowed(current.status, record.status) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run state transition is not allowed",
        ));
    }
    record.sequence = current
        .sequence
        .checked_add(1)
        .ok_or_else(|| internal("run ledger sequence overflow"))?;
    record.updated_unix_ms = unix_ms();
    append_run_entry(state, root, record, Some(current.ledger_head))
}

struct RunLedgerLock(File);

impl RunLedgerLock {
    fn acquire(root: &Path) -> Result<Self, ClewError> {
        let path = root.join("ledger.lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path).map_err(io_error)?;
        #[cfg(unix)]
        if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self(file))
    }
}

impl Drop for RunLedgerLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&self.0), libc::LOCK_UN);
        }
    }
}

fn append_run_entry(
    state: &StateAuthority,
    root: &Path,
    record: &mut RunRecord,
    previous_event_hash: Option<String>,
) -> Result<(), ClewError> {
    if previous_event_hash
        .as_deref()
        .is_some_and(|value| !sha256_digest(value))
    {
        return Err(invalid("run ledger predecessor is invalid"));
    }
    record.ledger_head.clear();
    let mut entry = RunLedgerEntry {
        schema: RUN_LEDGER_SCHEMA.into(),
        sequence: record.sequence,
        previous_event_hash,
        record: record.clone(),
        event_hash: String::new(),
    };
    entry.event_hash = run_event_hash(&entry)?;
    let mut bytes = canonical::bytes(&entry).map_err(internal)?;
    bytes.push(b'\n');
    let path = root.join("ledger.jsonl");
    let existing = fs::symlink_metadata(&path).ok();
    if existing.as_ref().is_some_and(|metadata| {
        metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len().saturating_add(bytes.len() as u64) > MAX_RUN_LEDGER_BYTES
    }) {
        return Err(invalid("run ledger is unsafe or exceeds its limit"));
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut ledger = options.open(path).map_err(io_error)?;
    ledger.write_all(&bytes).map_err(io_error)?;
    ledger.sync_all().map_err(io_error)?;
    record.ledger_head = entry.event_hash;
    state.write_private_atomic(
        &root.join("record.json"),
        &canonical::bytes(record).map_err(internal)?,
    )
}

fn load_run_projection(
    state: &StateAuthority,
    root: &Path,
    expected_run_id: &str,
) -> Result<RunRecord, ClewError> {
    let path = root.join("ledger.jsonl");
    let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_RUN_LEDGER_BYTES
    {
        return Err(invalid("run ledger is missing or unsafe"));
    }
    let bytes = fs::read(&path).map_err(io_error)?;
    if bytes.len() as u64 != metadata.len() || bytes.last() != Some(&b'\n') {
        return Err(invalid("run ledger changed while reading"));
    }
    let mut previous = None::<RunLedgerEntry>;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: RunLedgerEntry =
            serde_json::from_slice(line).map_err(|_| invalid("run ledger entry is invalid"))?;
        if canonical::bytes(&entry).map_err(internal)? != line
            || entry.schema != RUN_LEDGER_SCHEMA
            || entry.record.schema != RUN_SCHEMA
            || entry.record.run_id != expected_run_id
            || entry.sequence != entry.record.sequence
            || !entry.record.ledger_head.is_empty()
            || !sha256_digest(&entry.event_hash)
            || entry.event_hash != run_event_hash(&entry)?
        {
            return Err(invalid("run ledger authority is invalid"));
        }
        match &previous {
            None => {
                if entry.sequence != 0
                    || entry.previous_event_hash.is_some()
                    || entry.record.status != RunStatus::Created
                {
                    return Err(invalid("run ledger genesis is invalid"));
                }
            }
            Some(prior) => {
                if entry.sequence != prior.sequence.saturating_add(1)
                    || entry.previous_event_hash.as_deref() != Some(prior.event_hash.as_str())
                {
                    return Err(invalid("run ledger chain is discontinuous"));
                }
                require_same_run_identity(&prior.record, &entry.record)?;
                if !run_transition_allowed(prior.record.status, entry.record.status) {
                    return Err(invalid("run ledger contains an invalid transition"));
                }
            }
        }
        previous = Some(entry);
    }
    let last = previous.ok_or_else(|| invalid("run ledger is empty"))?;
    let mut projected = last.record;
    projected.ledger_head = last.event_hash;
    let projection_path = root.join("record.json");
    let projection_matches = read_json_limited::<RunRecord>(&projection_path, MAX_PLAN_BYTES)
        .is_ok_and(|record| canonical::bytes(&record).ok() == canonical::bytes(&projected).ok());
    if !projection_matches {
        state.write_private_atomic(
            &projection_path,
            &canonical::bytes(&projected).map_err(internal)?,
        )?;
    }
    Ok(projected)
}

fn run_event_hash(entry: &RunLedgerEntry) -> Result<String, ClewError> {
    let mut unsigned = entry.clone();
    unsigned.event_hash.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn require_same_run_identity(left: &RunRecord, right: &RunRecord) -> Result<(), ClewError> {
    if left.schema != right.schema
        || left.transaction_id != right.transaction_id
        || left.run_id != right.run_id
        || left.session_id != right.session_id
        || left.context_id != right.context_id
        || left.plan_id != right.plan_id
        || left.request_digest != right.request_digest
        || (left.candidate_commit.is_some() && left.candidate_commit != right.candidate_commit)
        || (left.candidate_snapshot.is_some()
            && left.candidate_snapshot != right.candidate_snapshot)
        || (left.final_commit.is_some() && left.final_commit != right.final_commit)
        || (left.publication_blocked && !right.publication_blocked)
    {
        return Err(invalid("run ledger changed immutable authority"));
    }
    Ok(())
}

fn run_transition_allowed(from: RunStatus, to: RunStatus) -> bool {
    from == to
        || matches!(
            (from, to),
            (
                RunStatus::Created,
                RunStatus::Preparing | RunStatus::Cancelled
            ) | (
                RunStatus::Preparing,
                RunStatus::ReadyToPublish
                    | RunStatus::ValidatedConditional
                    | RunStatus::Failed
                    | RunStatus::WorktreeRecoveryRequired
                    | RunStatus::Cancelled
            ) | (RunStatus::Failed | RunStatus::Cancelled, RunStatus::Created)
                | (RunStatus::ReadyToPublish, RunStatus::Publishing)
                | (
                    RunStatus::Publishing,
                    RunStatus::Published
                        | RunStatus::ReadyToPublish
                        | RunStatus::WorktreeRecoveryRequired
                )
                | (
                    RunStatus::WorktreeRecoveryRequired,
                    RunStatus::Published | RunStatus::WorktreeRecoveryRequired
                )
        )
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn bounded_context_stdout(context: &ContextObject) -> Result<Value, ClewError> {
    let summary = json!({
        "schema":"codeclew-context-result/1.0",
        "sessionId":context.session_id,
        "contextId":context.context_id,
        "parentContextId":context.parent_context_id,
        "intent":context.intent,
        "terms":context.terms,
        "evidenceDigest":context.evidence_digest,
        "context":context.projection,
        "completeness":context.evidence.pointer("/context/completeness"),
        "publicationPolicy":context.evidence.pointer("/context/publicationPolicy"),
    });
    let bytes = canonical::bytes(&summary).map_err(internal)?;
    if bytes.len() > MAX_CONTEXT_STDOUT_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "context stdout exceeds 64 KiB",
        ));
    }
    Ok(summary)
}

fn validate_plan_shape(plan: &Value) -> Result<(), ClewError> {
    crate::task_run_v2::validate_plan_value(plan)?;
    Ok(())
}

fn store_cas_json<T: Serialize>(
    state: &StateAuthority,
    session_root: &Path,
    identifier: &str,
    value: &T,
) -> Result<(), ClewError> {
    let digest = identifier
        .rsplit_once("sha256:")
        .map(|(_, digest)| digest)
        .ok_or_else(|| invalid("CAS identifier has no digest"))?;
    let path = session_root
        .join("objects/sha256")
        .join(format!("{digest}.json"));
    if path.exists() {
        return Ok(());
    }
    state.write_private_atomic(&path, &canonical::bytes(value).map_err(internal)?)
}

fn qualify_ref(value: &str) -> Result<String, ClewError> {
    if value.starts_with("refs/heads/") {
        Ok(value.into())
    } else if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_/.".contains(&byte))
        && !value.contains("..")
    {
        Ok(format!("refs/heads/{value}"))
    } else {
        Err(invalid("target ref is invalid"))
    }
}

fn id_filename(value: &str) -> Result<String, ClewError> {
    let (_, digest) = value
        .rsplit_once("sha256:")
        .ok_or_else(|| invalid("content identifier has no digest"))?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("content identifier digest is invalid"));
    }
    Ok(format!("{digest}.json"))
}

fn id_component<'a>(value: &'a str, prefix: &str) -> Result<&'a str, ClewError> {
    let value = value
        .strip_prefix(prefix)
        .ok_or_else(|| invalid("identifier has the wrong prefix"))?;
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(invalid("identifier is not a safe path component"));
    }
    Ok(value)
}

fn transaction_id(request_digest: &str) -> Result<String, ClewError> {
    let digest = request_digest
        .strip_prefix("sha256:")
        .ok_or_else(|| internal("request digest has no sha256 prefix"))?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(internal("request digest is not canonical"));
    }
    Ok(format!("tx:{digest}"))
}

fn write_json_create_new<T: Serialize>(path: &Path, value: &T) -> Result<(), ClewError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).map_err(io_error)?;
    serde_json::to_writer(&mut file, value).map_err(parse_error)?;
    file.sync_all().map_err(io_error)
}

fn read_json_limited<T: for<'de> Deserialize<'de>>(
    path: &Path,
    limit: usize,
) -> Result<T, ClewError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() as usize > limit {
        return Err(invalid(
            "managed JSON object is missing or exceeds its limit",
        ));
    }
    let bytes = fs::read(path).map_err(io_error)?;
    serde_json::from_slice(&bytes).map_err(parse_error)
}

fn create_filtered_detached_worktree(
    repository: &Path,
    destination: &Path,
    revision: &str,
) -> Result<(), ClewError> {
    if destination.exists() {
        return Err(invalid("session source worktree already exists"));
    }
    let add = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "worktree",
            "add",
            "--detach",
            "--no-checkout",
        ])
        .arg(destination)
        .arg(revision)
        .current_dir(repository)
        .output()
        .map_err(io_error)?;
    if !add.status.success() {
        return Err(invalid("session source worktree creation failed"));
    }
    let checkout = Command::new("git")
        .args(["checkout", "--force", revision, "--", "."])
        .args(LEGACY_EXCLUDES)
        .current_dir(destination)
        .output()
        .map_err(io_error)?;
    if !checkout.status.success() {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(destination)
            .current_dir(repository)
            .status();
        return Err(invalid("session source checkout failed"));
    }
    Ok(())
}

fn seal_source_worktree(root: &Path) -> Result<(), ClewError> {
    let mut directories = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| internal(error.to_string()))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            directories.push(entry.path().to_path_buf());
        } else if metadata.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let executable = metadata.permissions().mode() & 0o111 != 0;
                fs::set_permissions(
                    entry.path(),
                    fs::Permissions::from_mode(if executable { 0o500 } else { 0o400 }),
                )
                .map_err(io_error)?;
            }
        } else {
            return Err(invalid("session source contains an unsupported entry"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).map_err(io_error)?;
        }
    }
    Ok(())
}

fn filtered_worktree_clean(repository: &Path) -> Result<bool, ClewError> {
    let output = Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ])
        .args(LEGACY_EXCLUDES)
        .current_dir(repository)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("session source cleanliness is unavailable"));
    }
    Ok(output.stdout.is_empty())
}

fn git_output(repo: &Path, arguments: &[&str]) -> Result<String, ClewError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("Git authority is unavailable"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().into())
        .map_err(|_| invalid("Git authority is not UTF-8"))
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn session_authority_digest(authority: &SessionAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn parse_error(error: impl std::fmt::Display) -> ClewError {
    invalid(&error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    internal(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_run() -> RunRecord {
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        RunRecord {
            schema: RUN_SCHEMA.into(),
            run_id: format!("run:{digest}"),
            session_id: format!("session:{digest}"),
            context_id: format!("context:{digest}"),
            plan_id: format!("plan:{digest}"),
            request_digest: format!("sha256:{digest}"),
            sequence: 0,
            ledger_head: String::new(),
            status: RunStatus::Created,
            transaction_id: format!("tx:{digest}"),
            candidate_commit: None,
            candidate_snapshot: None,
            final_commit: None,
            publication_blocked: false,
            process_id: None,
            process_start_token: None,
            failure: None,
            updated_unix_ms: 1,
        }
    }

    fn initialized_run() -> (tempfile::TempDir, StateAuthority, PathBuf, RunRecord) {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let mut run = test_run();
        let root = state.run_root(&run.run_id).unwrap();
        append_run_entry(&state, &root, &mut run, None).unwrap();
        (temporary, state, root, run)
    }

    #[test]
    fn rejects_oversized_plan_shapes() {
        let operations = (0..=MAX_PLAN_OPERATIONS)
            .map(|index| json!({"target":{"fileId":format!("src/{index}.kt")}}))
            .collect::<Vec<_>>();
        assert!(validate_plan_shape(&json!({"operations":operations})).is_err());
    }

    #[test]
    fn run_states_are_explicit_and_non_legacy() {
        assert_eq!(
            serde_json::to_value(RunStatus::ReadyToPublish).unwrap(),
            "READY_TO_PUBLISH"
        );
        assert_eq!(
            serde_json::to_value(RunStatus::ValidatedConditional).unwrap(),
            "VALIDATED_CONDITIONAL"
        );
    }

    #[test]
    fn run_ids_are_path_safe_while_request_digests_remain_typed() {
        let request_digest =
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let path_digest = request_digest.strip_prefix("sha256:").unwrap();
        let run_id = format!("run:{path_digest}");
        assert_eq!(id_component(&run_id, "run:").unwrap(), path_digest);
        assert!(!run_id[4..].contains(':'));
        assert!(request_digest.starts_with("sha256:"));
        assert_eq!(
            transaction_id(request_digest).unwrap(),
            "tx:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn run_projection_is_recovered_from_the_authoritative_ledger() {
        let (_temporary, state, root, mut run) = initialized_run();
        run.status = RunStatus::Preparing;
        save_run_transition(&state, &root, &mut run).unwrap();
        fs::write(root.join("record.json"), b"corrupt projection").unwrap();

        let recovered = load_run_projection(&state, &root, &run.run_id).unwrap();

        assert_eq!(recovered.status, RunStatus::Preparing);
        assert_eq!(recovered.sequence, 1);
        assert_eq!(recovered.ledger_head, run.ledger_head);
        let projection: RunRecord =
            read_json_limited(&root.join("record.json"), MAX_PLAN_BYTES).unwrap();
        assert_eq!(projection.ledger_head, recovered.ledger_head);
    }

    #[test]
    fn stale_run_writer_is_rejected_by_sequence_and_ledger_head_cas() {
        let (_temporary, state, root, run) = initialized_run();
        let mut first = run.clone();
        let mut stale = run;
        first.status = RunStatus::Preparing;
        save_run_transition(&state, &root, &mut first).unwrap();
        stale.status = RunStatus::Cancelled;

        let error = save_run_transition(&state, &root, &mut stale).unwrap_err();

        assert_eq!(error.code, ErrorCode::PreconditionFailed);
    }

    #[test]
    fn invalid_terminal_run_transition_is_rejected() {
        let (_temporary, state, root, mut run) = initialized_run();
        run.status = RunStatus::Published;

        let error = save_run_transition(&state, &root, &mut run).unwrap_err();

        assert_eq!(error.code, ErrorCode::PreconditionFailed);
        assert_eq!(
            load_run_projection(&state, &root, &run.run_id)
                .unwrap()
                .status,
            RunStatus::Created
        );
    }

    #[test]
    fn tampered_run_ledger_fails_closed() {
        let (_temporary, state, root, run) = initialized_run();
        let ledger_path = root.join("ledger.jsonl");
        let mut bytes = fs::read(&ledger_path).unwrap();
        let offset = bytes.iter().position(|byte| *byte == b'{').unwrap();
        bytes[offset] = b'[';
        fs::write(ledger_path, bytes).unwrap();

        assert!(load_run_projection(&state, &root, &run.run_id).is_err());
    }

    #[test]
    fn detached_source_is_filtered_clean_and_read_only() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        let source = temporary.path().join("managed-source");
        fs::create_dir(&repository).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        fs::write(repository.join("README.md"), b"fixture\n").unwrap();
        fs::create_dir(repository.join(".semantic-thread")).unwrap();
        fs::write(repository.join(".semantic-thread/tracked"), b"legacy\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "."])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=Codeclew Test",
                    "-c",
                    "user.email=codeclew@example.invalid",
                    "commit",
                    "-q",
                    "-m",
                    "baseline",
                ])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        let revision = git_output(&repository, &["rev-parse", "HEAD"]).unwrap();
        create_filtered_detached_worktree(&repository, &source, &revision).unwrap();
        assert!(source.join("README.md").is_file());
        assert!(!source.join(".semantic-thread").exists());
        assert!(filtered_worktree_clean(&source).unwrap());
        seal_source_worktree(&source).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(source.join("README.md"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o400
            );
            for entry in WalkDir::new(&source).contents_first(true) {
                let path = entry.unwrap().into_path();
                if !path.is_symlink() {
                    let mode = if path.is_dir() { 0o700 } else { 0o600 };
                    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
                }
            }
        }
        assert!(
            Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(&source)
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
    }
}
