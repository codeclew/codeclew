use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::session::{SESSION_SCHEMA, SessionAuthority};
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[cfg(unix)]
use std::os::fd::AsRawFd;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionTestHookPoint {
    BeforeLifecycleLock,
    AfterLifecycleLock,
}

#[cfg(test)]
type TransitionTestHook = Box<dyn Fn(TransitionTestHookPoint)>;

#[cfg(test)]
std::thread_local! {
    static TRANSITION_TEST_HOOK: std::cell::RefCell<Option<TransitionTestHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn invoke_transition_test_hook(point: TransitionTestHookPoint) {
    TRANSITION_TEST_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow().as_ref() {
            hook(point);
        }
    });
}

#[cfg(test)]
pub(crate) fn with_transition_test_hook<T>(
    hook: impl Fn(TransitionTestHookPoint) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    TRANSITION_TEST_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    let result = action();
    TRANSITION_TEST_HOOK.with(|slot| {
        slot.borrow_mut().take();
    });
    result
}

pub const THREAD_SCHEMA: &str = "codeclew-thread/1.0";
pub const THREAD_LIFECYCLE_SCHEMA: &str = "codeclew-thread-lifecycle-entry/1.0";
pub const MIN_THREAD_MEMBERS: usize = 2;
pub const MAX_THREAD_MEMBERS: usize = 8;
const MAX_THREAD_AUTHORITY_BYTES: usize = 1024 * 1024;
const MAX_THREAD_LIFECYCLE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ThreadMemberRequest {
    pub member_alias: String,
    pub service_alias: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadMemberBinding {
    pub member_alias: String,
    pub service_alias: String,
    pub session: SessionAuthority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadAuthority {
    pub schema: String,
    pub thread_id: String,
    pub authority_digest: String,
    pub semantic_digest: String,
    pub members: Vec<ThreadMemberBinding>,
    pub created_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThreadStatus {
    Open,
    Closed,
    GarbageCollected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadLifecycle {
    pub schema: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub sequence: u64,
    pub previous_event_hash: Option<String>,
    pub status: ThreadStatus,
    pub event_hash: String,
    pub updated_unix_ms: u128,
}

pub(crate) struct ThreadAdmission {
    _lock: ThreadLifecycleLock,
}

impl ThreadAuthority {
    pub fn open(requests: Vec<ThreadMemberRequest>) -> Result<Self, ClewError> {
        validate_requests(&requests)?;
        let mut members = Vec::with_capacity(requests.len());
        for request in requests {
            let (session, _) = SessionAuthority::load(&request.session_id)?;
            session.require_open()?;
            members.push(ThreadMemberBinding {
                member_alias: request.member_alias,
                service_alias: request.service_alias,
                session,
            });
        }
        let state = StateAuthority::process_default()?;
        create_with_state(
            &state,
            format!("thread:{}", Uuid::new_v4()),
            unix_ms(),
            members,
        )
    }

    pub fn load(thread_id: &str) -> Result<(Self, std::path::PathBuf), ClewError> {
        let state = StateAuthority::process_default()?;
        load_with_state(&state, thread_id)
    }

    pub fn lifecycle(&self) -> Result<ThreadLifecycle, ClewError> {
        self.verify()?;
        let state = StateAuthority::process_default()?;
        let root = state.thread_root(&self.thread_id)?;
        load_lifecycle(&state, &root, self)
    }

    pub fn close(&self) -> Result<ThreadLifecycle, ClewError> {
        self.verify()?;
        let state = StateAuthority::process_default()?;
        transition_with_state(&state, self, ThreadStatus::Closed)
    }

    pub fn gc(&self) -> Result<ThreadLifecycle, ClewError> {
        self.verify()?;
        let state = StateAuthority::process_default()?;
        transition_with_state(&state, self, ThreadStatus::GarbageCollected)
    }

    pub(crate) fn verify(&self) -> Result<(), ClewError> {
        validate_authority(self)
    }

    pub(crate) fn admit_with_state(
        &self,
        state: &StateAuthority,
    ) -> Result<ThreadAdmission, ClewError> {
        self.verify()?;
        let root = state.thread_root(&self.thread_id)?;
        let lock = ThreadLifecycleLock::acquire(state, &root)?;
        if load_lifecycle_unlocked(state, &root, self)?.status != ThreadStatus::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "thread is terminal and cannot accept new contexts",
            ));
        }
        Ok(ThreadAdmission { _lock: lock })
    }

    pub(crate) fn require_open_with_state(&self, state: &StateAuthority) -> Result<(), ClewError> {
        self.verify()?;
        let root = state.thread_root(&self.thread_id)?;
        if load_lifecycle(state, &root, self)?.status != ThreadStatus::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "thread is terminal and cannot accept new contexts",
            ));
        }
        Ok(())
    }
}

pub(crate) fn create_with_state(
    state: &StateAuthority,
    thread_id: String,
    created_unix_ms: u128,
    mut members: Vec<ThreadMemberBinding>,
) -> Result<ThreadAuthority, ClewError> {
    members.sort_by(|left, right| left.member_alias.cmp(&right.member_alias));
    let mut authority = ThreadAuthority {
        schema: THREAD_SCHEMA.into(),
        thread_id,
        authority_digest: String::new(),
        semantic_digest: String::new(),
        members,
        created_unix_ms,
    };
    authority.semantic_digest = semantic_digest(&authority)?;
    authority.authority_digest = authority_digest(&authority)?;
    validate_authority(&authority)?;

    let root = state.thread_root(&authority.thread_id)?;
    let directory = state.directory_at(&root)?;
    directory.require_path_identity()?;
    directory.child(Path::new("contexts"))?;
    write_json_create_new(state, &root.join("authority.json"), &authority)?;
    initialize_lifecycle(state, &root, &authority)?;
    Ok(authority)
}

pub(crate) fn load_with_state(
    state: &StateAuthority,
    thread_id: &str,
) -> Result<(ThreadAuthority, std::path::PathBuf), ClewError> {
    let root = state.thread_root(thread_id)?;
    let authority: ThreadAuthority = read_canonical_json(
        state,
        &root.join("authority.json"),
        MAX_THREAD_AUTHORITY_BYTES,
    )?;
    if authority.thread_id != thread_id {
        return Err(invalid("thread authority identity is invalid"));
    }
    validate_authority(&authority)?;
    load_lifecycle(state, &root, &authority)?;
    Ok((authority, root))
}

pub(crate) fn revalidate_authority_record(
    state: &StateAuthority,
    expected: &ThreadAuthority,
) -> Result<(), ClewError> {
    let path = state
        .thread_root(&expected.thread_id)?
        .join("authority.json");
    let bytes = state
        .read_private_file(&path, MAX_THREAD_AUTHORITY_BYTES)
        .map_err(|_| {
            ClewError::new(
                ErrorCode::BindingChanged,
                "persisted thread authority disappeared before derived publication",
            )
        })?;
    if bytes != canonical::bytes(expected).map_err(internal)? {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "persisted thread authority changed before derived publication",
        ));
    }
    Ok(())
}

fn validate_requests(requests: &[ThreadMemberRequest]) -> Result<(), ClewError> {
    if requests.len() < MIN_THREAD_MEMBERS || requests.len() > MAX_THREAD_MEMBERS {
        return Err(invalid(
            "thread must bind between two and eight analysis units",
        ));
    }
    let mut aliases = BTreeSet::new();
    let mut session_ids = BTreeSet::new();
    for request in requests {
        if !safe_alias(&request.member_alias)
            || !safe_alias(&request.service_alias)
            || !aliases.insert(request.member_alias.as_str())
            || !session_ids.insert(request.session_id.as_str())
        {
            return Err(invalid(
                "thread member aliases and session bindings must be safe and unique",
            ));
        }
    }
    Ok(())
}

fn validate_authority(authority: &ThreadAuthority) -> Result<(), ClewError> {
    if authority.schema != THREAD_SCHEMA
        || !safe_thread_id(&authority.thread_id)
        || authority.members.len() < MIN_THREAD_MEMBERS
        || authority.members.len() > MAX_THREAD_MEMBERS
    {
        return Err(invalid("thread authority shape is invalid"));
    }
    let mut aliases = BTreeSet::new();
    let mut sessions = BTreeSet::new();
    let mut previous_alias: Option<&str> = None;
    for member in &authority.members {
        if !safe_alias(&member.member_alias)
            || !safe_alias(&member.service_alias)
            || previous_alias.is_some_and(|previous| previous >= member.member_alias.as_str())
            || !aliases.insert(member.member_alias.as_str())
            || !sessions.insert(member.session.session_id.as_str())
            || member.session.schema != SESSION_SCHEMA
            || member.session.authority_digest != embedded_session_digest(&member.session)?
        {
            return Err(invalid("thread member authority is invalid"));
        }
        previous_alias = Some(&member.member_alias);
    }
    if authority.semantic_digest != semantic_digest(authority)?
        || authority.authority_digest != authority_digest(authority)?
    {
        return Err(invalid("thread authority digest is invalid"));
    }
    Ok(())
}

fn semantic_digest(authority: &ThreadAuthority) -> Result<String, ClewError> {
    let members = authority
        .members
        .iter()
        .map(|member| {
            json!({
                "memberAlias":member.member_alias,
                "serviceAlias":member.service_alias,
                "repositoryKey":member.session.repository_key,
                "baseRevision":member.session.base_revision,
                "runtimeKey":member.session.runtime_key,
                "runtimeMode":member.session.runtime_mode,
                "language":member.session.language,
                "compilations":member.session.compilations,
                "modelCachePolicy":member.session.model_cache_policy,
                "modelCacheAuthority":member.session.model_cache_authority,
            })
        })
        .collect::<Vec<_>>();
    canonical::hash(&json!({
        "schema":"codeclew-thread-semantic-authority/1.0",
        "members":members,
    }))
    .map_err(internal)
}

fn authority_digest(authority: &ThreadAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn embedded_session_digest(session: &SessionAuthority) -> Result<String, ClewError> {
    let mut unsigned = session.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn safe_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && crate::text_authority::is_nfc(value)
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn safe_thread_id(value: &str) -> bool {
    value.strip_prefix("thread:").is_some_and(|component| {
        !component.is_empty()
            && component.len() <= 128
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn initialize_lifecycle(
    state: &StateAuthority,
    root: &Path,
    authority: &ThreadAuthority,
) -> Result<(), ClewError> {
    let _lock = ThreadLifecycleLock::acquire(state, root)?;
    if state.private_file_exists(&root.join("lifecycle.jsonl"))? {
        return Err(invalid("thread lifecycle already exists"));
    }
    append_lifecycle(
        state,
        root,
        ThreadLifecycle {
            schema: THREAD_LIFECYCLE_SCHEMA.into(),
            thread_id: authority.thread_id.clone(),
            thread_authority_digest: authority.authority_digest.clone(),
            sequence: 0,
            previous_event_hash: None,
            status: ThreadStatus::Open,
            event_hash: String::new(),
            updated_unix_ms: unix_ms(),
        },
    )
}

fn load_lifecycle(
    state: &StateAuthority,
    root: &Path,
    authority: &ThreadAuthority,
) -> Result<ThreadLifecycle, ClewError> {
    let _lock = ThreadLifecycleLock::acquire(state, root)?;
    load_lifecycle_unlocked(state, root, authority)
}

fn load_lifecycle_unlocked(
    state: &StateAuthority,
    root: &Path,
    authority: &ThreadAuthority,
) -> Result<ThreadLifecycle, ClewError> {
    let bytes = state
        .read_private_file(&root.join("lifecycle.jsonl"), MAX_THREAD_LIFECYCLE_BYTES)
        .map_err(|_| invalid("thread lifecycle ledger is missing or unsafe"))?;
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err(invalid("thread lifecycle ledger is missing or unsafe"));
    }
    let mut previous: Option<ThreadLifecycle> = None;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: ThreadLifecycle = serde_json::from_slice(line)
            .map_err(|_| invalid("thread lifecycle entry is invalid"))?;
        if canonical::bytes(&entry).map_err(internal)? != line
            || entry.schema != THREAD_LIFECYCLE_SCHEMA
            || entry.thread_id != authority.thread_id
            || entry.thread_authority_digest != authority.authority_digest
            || entry.event_hash != lifecycle_hash(&entry)?
            || !sha256_digest(&entry.event_hash)
        {
            return Err(invalid("thread lifecycle authority is invalid"));
        }
        match &previous {
            None => {
                if entry.sequence != 0
                    || entry.previous_event_hash.is_some()
                    || entry.status != ThreadStatus::Open
                {
                    return Err(invalid("thread lifecycle genesis is invalid"));
                }
            }
            Some(prior) => {
                if entry.sequence != prior.sequence.saturating_add(1)
                    || entry.previous_event_hash.as_deref() != Some(prior.event_hash.as_str())
                    || !transition_allowed(prior.status, entry.status)
                {
                    return Err(invalid("thread lifecycle chain is invalid"));
                }
            }
        }
        previous = Some(entry);
    }
    previous.ok_or_else(|| invalid("thread lifecycle ledger is empty"))
}

fn transition_with_state(
    state: &StateAuthority,
    authority: &ThreadAuthority,
    requested: ThreadStatus,
) -> Result<ThreadLifecycle, ClewError> {
    let root = state.thread_root(&authority.thread_id)?;
    #[cfg(test)]
    invoke_transition_test_hook(TransitionTestHookPoint::BeforeLifecycleLock);
    let _lock = ThreadLifecycleLock::acquire(state, &root)?;
    #[cfg(test)]
    invoke_transition_test_hook(TransitionTestHookPoint::AfterLifecycleLock);
    let current = load_lifecycle_unlocked(state, &root, authority)?;
    if current.status == requested {
        return Ok(current);
    }
    if !transition_allowed(current.status, requested) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "thread lifecycle transition is not allowed",
        ));
    }
    let next = ThreadLifecycle {
        schema: THREAD_LIFECYCLE_SCHEMA.into(),
        thread_id: authority.thread_id.clone(),
        thread_authority_digest: authority.authority_digest.clone(),
        sequence: current.sequence.saturating_add(1),
        previous_event_hash: Some(current.event_hash),
        status: requested,
        event_hash: String::new(),
        updated_unix_ms: unix_ms(),
    };
    append_lifecycle(state, &root, next)?;
    load_lifecycle_unlocked(state, &root, authority)
}

#[cfg(test)]
pub(crate) fn transition_with_state_for_test(
    state: &StateAuthority,
    authority: &ThreadAuthority,
    requested: ThreadStatus,
) -> Result<ThreadLifecycle, ClewError> {
    transition_with_state(state, authority, requested)
}

fn transition_allowed(from: ThreadStatus, to: ThreadStatus) -> bool {
    matches!(
        (from, to),
        (ThreadStatus::Open, ThreadStatus::Closed)
            | (ThreadStatus::Closed, ThreadStatus::GarbageCollected)
    )
}

fn append_lifecycle(
    state: &StateAuthority,
    root: &Path,
    mut entry: ThreadLifecycle,
) -> Result<(), ClewError> {
    entry.event_hash = lifecycle_hash(&entry)?;
    let mut bytes = canonical::bytes(&entry).map_err(internal)?;
    bytes.push(b'\n');
    let path = root.join("lifecycle.jsonl");
    let existing = if state.private_file_exists(&path)? {
        state.read_private_file(&path, MAX_THREAD_LIFECYCLE_BYTES)?
    } else {
        Vec::new()
    };
    if existing.len().saturating_add(bytes.len()) > MAX_THREAD_LIFECYCLE_BYTES {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "thread lifecycle ledger exceeds 1 MiB",
        ));
    }
    let mut file = state.open_private_append(&path)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn lifecycle_hash(entry: &ThreadLifecycle) -> Result<String, ClewError> {
    let mut unsigned = entry.clone();
    unsigned.event_hash.clear();
    canonical::hash(&unsigned).map_err(internal)
}

struct ThreadLifecycleLock(File);

impl ThreadLifecycleLock {
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

impl Drop for ThreadLifecycleLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn write_json_create_new<T: Serialize>(
    state: &StateAuthority,
    path: &Path,
    value: &T,
) -> Result<(), ClewError> {
    let relative = path
        .strip_prefix(state.root())
        .map_err(|_| invalid("managed thread path escapes state authority"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("managed thread path has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("managed thread path has no file name"))?;
    let mut file = state.directory(parent)?.create_file(name)?;
    file.write_all(&canonical::bytes(value).map_err(internal)?)
        .map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn read_canonical_json<T: for<'de> Deserialize<'de> + Serialize>(
    state: &StateAuthority,
    path: &Path,
    limit: usize,
) -> Result<T, ClewError> {
    let bytes = state
        .read_private_file(path, limit)
        .map_err(|_| invalid("managed thread object is missing or exceeds its limit"))?;
    let value: T =
        serde_json::from_slice(&bytes).map_err(|_| invalid("managed thread object is invalid"))?;
    if canonical::bytes(&value).map_err(internal)? != bytes {
        return Err(invalid("managed thread object is not canonical"));
    }
    Ok(value)
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
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

    fn session(seed: char, repository_key: &str, created: u128) -> SessionAuthority {
        let digest = std::iter::repeat_n(seed, 64).collect::<String>();
        let mut session = SessionAuthority {
            schema: SESSION_SCHEMA.into(),
            authority_digest: String::new(),
            session_id: format!("session:{digest}"),
            repository_key: repository_key.into(),
            base_revision: std::iter::repeat_n(seed, 40).collect(),
            target_ref: "refs/heads/main".into(),
            target_oid: std::iter::repeat_n(seed, 40).collect(),
            runtime_key: format!("runtime:{digest}"),
            runtime_mode: RuntimeMode::Development,
            language: SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: created,
        };
        session.authority_digest = embedded_session_digest(&session).unwrap();
        session
    }

    fn binding(alias: &str, session: SessionAuthority) -> ThreadMemberBinding {
        ThreadMemberBinding {
            member_alias: alias.into(),
            service_alias: format!("{alias}-service"),
            session,
        }
    }

    fn built(
        thread_id: &str,
        created: u128,
        mut members: Vec<ThreadMemberBinding>,
    ) -> ThreadAuthority {
        members.sort_by(|left, right| left.member_alias.cmp(&right.member_alias));
        let mut authority = ThreadAuthority {
            schema: THREAD_SCHEMA.into(),
            thread_id: thread_id.into(),
            authority_digest: String::new(),
            semantic_digest: String::new(),
            members,
            created_unix_ms: created,
        };
        authority.semantic_digest = semantic_digest(&authority).unwrap();
        authority.authority_digest = authority_digest(&authority).unwrap();
        authority
    }

    #[test]
    fn thread_identity_is_order_independent_but_instance_distinct() {
        let left = binding("left", session('a', "repo:left", 1));
        let right = binding("right", session('b', "repo:right", 1));
        let forward = built("thread:fixed", 10, vec![left.clone(), right.clone()]);
        let reversed = built("thread:fixed", 10, vec![right, left]);
        assert_eq!(forward.authority_digest, reversed.authority_digest);
        assert_eq!(forward.semantic_digest, reversed.semantic_digest);

        let equivalent = built(
            "thread:other",
            20,
            vec![
                binding("left", session('c', "repo:left", 9)),
                binding("right", session('d', "repo:right", 9)),
            ],
        );
        assert_ne!(forward.authority_digest, equivalent.authority_digest);
        assert_ne!(forward.semantic_digest, equivalent.semantic_digest);
    }

    #[test]
    fn stable_semantic_identity_excludes_session_uuid_and_timestamp() {
        let first = session('a', "repo:shared", 1);
        let mut second = first.clone();
        second.session_id = format!("session:{}", "b".repeat(64));
        second.created_unix_ms = 99;
        second.authority_digest = embedded_session_digest(&second).unwrap();
        let other_first = session('c', "repo:other", 1);
        let mut other_second = other_first.clone();
        other_second.session_id = format!("session:{}", "d".repeat(64));
        other_second.created_unix_ms = 99;
        other_second.authority_digest = embedded_session_digest(&other_second).unwrap();

        let a = built(
            "thread:a",
            1,
            vec![binding("one", first), binding("two", other_first)],
        );
        let b = built(
            "thread:b",
            2,
            vec![binding("one", second), binding("two", other_second)],
        );
        assert_eq!(a.semantic_digest, b.semantic_digest);
        assert_ne!(a.authority_digest, b.authority_digest);
    }

    #[test]
    fn one_repository_can_bind_distinct_language_analysis_units() {
        let kotlin = session('a', "repo:shared", 1);
        let mut python = session('b', "repo:shared", 1);
        python.language = SessionLanguage::Python;
        python.compilations = vec!["python:.#src".into()];
        python.authority_digest = embedded_session_digest(&python).unwrap();
        let authority = built(
            "thread:mixed",
            1,
            vec![binding("kotlin", kotlin), binding("python", python)],
        );
        authority.verify().unwrap();
        assert_eq!(
            authority
                .members
                .iter()
                .map(|member| member.session.repository_key.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["repo:shared"])
        );
        assert_eq!(
            authority
                .members
                .iter()
                .map(|member| member.session.language)
                .collect::<Vec<_>>(),
            [SessionLanguage::Kotlin, SessionLanguage::Python]
        );
    }

    #[test]
    fn membership_is_bounded_unique_and_path_safe() {
        let valid = vec![
            ThreadMemberRequest {
                member_alias: "api".into(),
                service_alias: "api-service".into(),
                session_id: format!("session:{}", "a".repeat(64)),
            },
            ThreadMemberRequest {
                member_alias: "client".into(),
                service_alias: "client-service".into(),
                session_id: format!("session:{}", "b".repeat(64)),
            },
        ];
        validate_requests(&valid).unwrap();
        assert!(validate_requests(&valid[..1]).is_err());
        let eight = (0..MAX_THREAD_MEMBERS)
            .map(|index| ThreadMemberRequest {
                member_alias: format!("member-{index}"),
                service_alias: format!("service-{index}"),
                session_id: format!("session-{index}"),
            })
            .collect::<Vec<_>>();
        validate_requests(&eight).unwrap();
        let mut nine = eight;
        nine.push(ThreadMemberRequest {
            member_alias: "member-8".into(),
            service_alias: "service-8".into(),
            session_id: "session-8".into(),
        });
        assert!(validate_requests(&nine).is_err());
        let eight_bindings = (0..MAX_THREAD_MEMBERS)
            .map(|index| {
                binding(
                    &format!("member-{index}"),
                    session(
                        char::from(b'a' + u8::try_from(index).unwrap()),
                        &format!("repo:{index}"),
                        1,
                    ),
                )
            })
            .collect::<Vec<_>>();
        built("thread:eight", 1, eight_bindings.clone())
            .verify()
            .unwrap();
        let mut nine_bindings = eight_bindings;
        nine_bindings.push(binding("member-8", session('i', "repo:8", 1)));
        assert!(built("thread:nine", 1, nine_bindings).verify().is_err());
        let mut duplicate = valid.clone();
        duplicate[1].member_alias = "api".into();
        assert!(validate_requests(&duplicate).is_err());
        let mut unsafe_alias = valid;
        unsafe_alias[1].member_alias = "../client".into();
        assert!(validate_requests(&unsafe_alias).is_err());
    }

    #[test]
    fn lifecycle_is_append_only_and_never_cascades_to_sessions() {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let members = vec![
            binding("left", session('a', "repo:left", 1)),
            binding("right", session('b', "repo:right", 1)),
        ];
        for member in &members {
            let root = state.session_root(&member.session.session_id).unwrap();
            state
                .write_private_atomic(&root.join("sentinel"), b"member-owned")
                .unwrap();
        }
        let authority = create_with_state(&state, "thread:fixed".into(), 1, members).unwrap();
        assert_eq!(
            load_lifecycle(
                &state,
                &state.thread_root(&authority.thread_id).unwrap(),
                &authority,
            )
            .unwrap()
            .status,
            ThreadStatus::Open
        );
        assert_eq!(
            transition_with_state(&state, &authority, ThreadStatus::Closed)
                .unwrap()
                .status,
            ThreadStatus::Closed
        );
        assert_eq!(
            transition_with_state(&state, &authority, ThreadStatus::GarbageCollected)
                .unwrap()
                .status,
            ThreadStatus::GarbageCollected
        );
        for member in &authority.members {
            let root = state.session_root(&member.session.session_id).unwrap();
            assert_eq!(
                state.read_private_file(&root.join("sentinel"), 64).unwrap(),
                b"member-owned"
            );
        }
    }
}
