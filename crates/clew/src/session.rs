pub mod mission;

use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::python_adapter_v2::{
    MAX_SOURCE_FILE_BYTES as MAX_PYTHON_SOURCE_FILE_BYTES,
    MAX_SOURCE_FILES as MAX_PYTHON_SOURCE_FILES,
    MAX_TOTAL_SOURCE_BYTES as MAX_PYTHON_TOTAL_SOURCE_BYTES,
};
use crate::python_project_model::PythonCompilationSelector;
use crate::repository_snapshot::{
    LEGACY_EXCLUDES, RepositoryInputSnapshot, SNAPSHOT_SCHEMA, TrackedScopeLimits,
    capture_commit_scope, isolated_git_command,
};
use crate::runtime::{RuntimeAuthority, RuntimeMode};
use crate::state::StateAuthority;
#[cfg(test)]
use crate::state::create_private_directory;
use crate::task_run_v2::ConditionalPublicationApproval;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use walkdir::WalkDir;

pub const SESSION_SCHEMA: &str = "codeclew-session/5.0";
pub const CONTEXT_SCHEMA: &str = "codeclew-context/3.0";
pub const PLAN_SCHEMA: &str = "codeclew-plan/2.0";
pub const RUN_SCHEMA: &str = "codeclew-task-run/3.0";
const RUN_LEDGER_SCHEMA: &str = "codeclew-task-run-ledger-entry/3.0";
const MAX_RUN_LEDGER_BYTES: u64 = 32 * 1024 * 1024;
const SESSION_LIFECYCLE_SCHEMA: &str = "codeclew-session-lifecycle-entry/1.0";
const SESSION_RUN_REFERENCE_SCHEMA: &str = "codeclew-session-run-reference/1.0";
const MAX_SESSION_LIFECYCLE_BYTES: u64 = 1024 * 1024;
const CONTEXT_EVIDENCE_SCHEMA: &str = "codeclew-context-evidence-object/3.0";
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
    pub language: SessionLanguage,
    pub compilations: Vec<String>,
    pub generation_jobs: Option<usize>,
    pub model_cache_policy: ModelCachePolicy,
    pub model_cache_authority: Option<String>,
    pub created_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionLanguage {
    Kotlin,
    Python,
    Rust,
}

impl SessionLanguage {
    pub fn uri(self) -> &'static str {
        match self {
            Self::Kotlin => "language:kotlin",
            Self::Python => "language:python",
            Self::Rust => "language:rust",
        }
    }
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
    pub prepared_authority_digest: Option<String>,
    pub final_commit: Option<String>,
    pub publication_blocked: bool,
    pub conditional_approval: Option<ConditionalPublicationApproval>,
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
    ReadyToPublishConditional,
    ValidatedConditional,
    Publishing,
    Published,
    PublishedConditional,
    Failed,
    WorktreeRecoveryRequired,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionStatus {
    Open,
    Closed,
    Aborted,
    GarbageCollected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionFreshness {
    pub schema: String,
    pub session_id: String,
    pub lifecycle_status: SessionStatus,
    pub status: String,
    pub head_matches_expected: Option<bool>,
    pub target_ref_matches_expected: Option<bool>,
    pub target_worktree_clean: Option<bool>,
    pub remediation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionLifecycle {
    pub schema: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub sequence: u64,
    pub previous_event_hash: Option<String>,
    pub status: SessionStatus,
    pub event_hash: String,
    pub updated_unix_ms: u128,
}

pub struct SessionAdmission {
    session_id: String,
    _lock: SessionLifecycleLock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionRunReference {
    schema: String,
    session_id: String,
    session_authority_digest: String,
    run_id: String,
    request_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryLocator {
    schema: String,
    target_repository_path: PathBuf,
    source_repository_path: PathBuf,
    external_build_state_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PythonSourceBinding {
    schema: String,
    session_id: String,
    session_authority_digest: String,
    base_revision: String,
    snapshot: CasObject,
}

const PYTHON_SOURCE_BINDING_SCHEMA: &str = "codeclew-python-session-source/1.0";

fn validate_locator(locator: &RepositoryLocator) -> Result<(), ClewError> {
    if locator.schema != "codeclew-repository-locator/3.0"
        || !locator.target_repository_path.is_absolute()
        || !locator.source_repository_path.is_absolute()
    {
        return Err(invalid("repository locator schema or paths are invalid"));
    }
    Ok(())
}

impl SessionAuthority {
    pub fn open(
        repo: &Path,
        target_ref: &str,
        language: SessionLanguage,
        compilations: &[String],
        generation_jobs: Option<usize>,
        model_cache_policy: ModelCachePolicy,
        external_build_state: Option<&Path>,
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
        let compilations = canonical_compilations(language, compilations)?;
        if !model_cache_policy_is_valid(language, model_cache_policy) {
            return Err(invalid(
                "read-only language sessions require NON_CACHEABLE live source authority",
            ));
        }
        if !generation_jobs_are_valid(generation_jobs) {
            return Err(invalid("generation jobs must be between 1 and 64"));
        }
        let base_revision = if language == SessionLanguage::Python {
            isolated_git_output(&repo, &["rev-parse", "--verify", "HEAD^{commit}"])?
        } else {
            git_output(&repo, &["rev-parse", "HEAD"])?
        };
        let target_oid = if language == SessionLanguage::Python {
            isolated_git_output(
                &repo,
                &["rev-parse", "--verify", &format!("{target_ref}^{{commit}}")],
            )?
        } else {
            git_output(&repo, &["rev-parse", &target_ref])?
        };
        if target_oid != base_revision {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session target ref must identify the checked-out base revision",
            ));
        }
        let (model_cache_authority, external_build_state_path) = model_cache_authority(
            &repo,
            &compilations,
            model_cache_policy,
            runtime.mode,
            external_build_state,
        )?;
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
            language,
            compilations,
            generation_jobs,
            model_cache_policy,
            model_cache_authority,
            created_unix_ms: unix_ms(),
        };
        authority.authority_digest = session_authority_digest(&authority)?;
        let root = state.session_root(&authority.session_id)?;
        let session_directory = state.directory_at(&root)?;
        session_directory.require_path_identity()?;
        for child in ["objects/sha256", "contexts", "plans", "candidates", "runs"] {
            session_directory.child(Path::new(child))?;
        }
        let source_repository_path = if language == SessionLanguage::Python {
            let selectors = authority
                .compilations
                .iter()
                .map(|value| PythonCompilationSelector::parse(value))
                .collect::<Result<Vec<_>, _>>()?;
            let source_roots = selectors
                .iter()
                .map(|selector| selector.source_root.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let store = CasStore::open(&state)?;
            let (_, snapshot) = capture_commit_scope(
                &repo,
                &authority.base_revision,
                &source_roots,
                &store,
                |path| selectors.iter().any(|selector| selector.contains(path)),
                TrackedScopeLimits {
                    max_files: MAX_PYTHON_SOURCE_FILES,
                    max_file_bytes: MAX_PYTHON_SOURCE_FILE_BYTES,
                    max_total_bytes: MAX_PYTHON_TOTAL_SOURCE_BYTES,
                    max_tree_entries: 262_144,
                    max_tree_bytes: 64 * 1024 * 1024,
                    max_tree_path_bytes: 4096,
                },
            )?;
            write_managed_json_create_new(
                &state,
                &root.join("python-source.json"),
                &PythonSourceBinding {
                    schema: PYTHON_SOURCE_BINDING_SCHEMA.into(),
                    session_id: authority.session_id.clone(),
                    session_authority_digest: authority.authority_digest.clone(),
                    base_revision: authority.base_revision.clone(),
                    snapshot,
                },
            )?;
            repo.clone()
        } else {
            let source = root.join("source");
            create_filtered_detached_worktree(&repo, &source, &authority.base_revision)?;
            seal_source_worktree(&source)?;
            source
        };
        write_managed_json_create_new(&state, &root.join("authority.json"), &authority)?;
        write_managed_json_create_new(
            &state,
            &root.join("locator.json"),
            &RepositoryLocator {
                schema: "codeclew-repository-locator/3.0".into(),
                target_repository_path: repo,
                source_repository_path,
                external_build_state_path,
            },
        )?;
        initialize_session_lifecycle(&state, &root, &authority)?;
        Ok(authority)
    }

    pub fn load(session_id: &str) -> Result<(Self, PathBuf), ClewError> {
        Self::load_with_runtime_policy(session_id, true)
    }

    pub fn load_for_cleanup(session_id: &str) -> Result<(Self, PathBuf), ClewError> {
        Self::load_with_runtime_policy(session_id, false)
    }

    fn load_with_runtime_policy(
        session_id: &str,
        require_runtime_match: bool,
    ) -> Result<(Self, PathBuf), ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(session_id)?;
        let authority: Self =
            read_managed_json(&state, &root.join("authority.json"), MAX_PLAN_BYTES)?;
        if authority.schema != SESSION_SCHEMA
            || authority.session_id != session_id
            || !compilations_are_canonical(authority.language, &authority.compilations)
            || !generation_jobs_are_valid(authority.generation_jobs)
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
        if require_runtime_match
            && (runtime.runtime_key != authority.runtime_key
                || runtime.mode != authority.runtime_mode)
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "session runtime authority does not match the active capsule",
            ));
        }
        load_session_lifecycle(&state, &root, &authority)?;
        Ok((authority, root))
    }

    pub fn lifecycle(&self) -> Result<SessionLifecycle, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        load_session_lifecycle(&state, &root, self)
    }

    pub fn require_open(&self) -> Result<(), ClewError> {
        let lifecycle = self.lifecycle()?;
        if lifecycle.status != SessionStatus::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session is terminal and cannot accept new work",
            ));
        }
        Ok(())
    }

    pub fn open_admission(&self) -> Result<SessionAdmission, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        open_session_admission_with_state(self, &state, &root)
    }

    pub fn close(&self) -> Result<SessionLifecycle, ClewError> {
        transition_session_terminal(self, SessionStatus::Closed)
    }

    pub fn abort(&self) -> Result<SessionLifecycle, ClewError> {
        transition_session_terminal(self, SessionStatus::Aborted)
    }

    pub fn relocate(&self, repository: &Path) -> Result<SessionLifecycle, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let _lock = SessionLifecycleLock::acquire(&state, &root)?;
        let lifecycle = load_session_lifecycle_unlocked(&state, &root, self)?;
        if lifecycle.status != SessionStatus::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "only an open session can be relocated",
            ));
        }
        let repository = repository.canonicalize().map_err(io_error)?;
        let runs = load_session_runs(&state, &root, self)?;
        let expected_target_oid = session_terminal_target_oid(self, &runs)?;
        if state.repository(&repository)?.key != self.repository_key
            || git_output(&repository, &["rev-parse", "HEAD"])? != expected_target_oid
            || git_output(&repository, &["rev-parse", &self.target_ref])? != expected_target_oid
            || git_output(
                &repository,
                &["rev-parse", &format!("{}^{{commit}}", self.base_revision)],
            )? != self.base_revision
        {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "relocation target does not preserve repository, base, and target authority",
            ));
        }
        let locator: RepositoryLocator =
            read_managed_json(&state, &root.join("locator.json"), MAX_PLAN_BYTES)?;
        validate_locator(&locator)?;
        if self.language == SessionLanguage::Python {
            let old_target = locator
                .target_repository_path
                .canonicalize()
                .map_err(io_error)?;
            let old_source = locator
                .source_repository_path
                .canonicalize()
                .map_err(io_error)?;
            if old_source != old_target || state.repository(&old_target)?.key != self.repository_key
            {
                return Err(invalid(
                    "Python session locator is invalid during relocation",
                ));
            }
            state.write_private_atomic(
                &root.join("locator.json"),
                &canonical::bytes(&RepositoryLocator {
                    schema: locator.schema,
                    target_repository_path: repository.clone(),
                    source_repository_path: repository,
                    external_build_state_path: locator.external_build_state_path,
                })
                .map_err(internal)?,
            )?;
            return Ok(lifecycle);
        }
        let source = locator
            .source_repository_path
            .canonicalize()
            .map_err(io_error)?;
        if !source.starts_with(&root)
            || git_output(&source, &["rev-parse", "HEAD"])? != self.base_revision
            || !filtered_worktree_clean(&source)?
            || state.repository(&source)?.key != self.repository_key
        {
            return Err(invalid(
                "session source locator is invalid during relocation",
            ));
        }
        state.write_private_atomic(
            &root.join("locator.json"),
            &canonical::bytes(&RepositoryLocator {
                schema: locator.schema,
                target_repository_path: repository,
                source_repository_path: locator.source_repository_path,
                external_build_state_path: locator.external_build_state_path,
            })
            .map_err(internal)?,
        )?;
        Ok(lifecycle)
    }

    pub fn gc(&self, force: bool) -> Result<SessionLifecycle, ClewError> {
        garbage_collect_session(self, force)
    }

    pub fn freshness(&self) -> Result<SessionFreshness, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let lifecycle = load_session_lifecycle(&state, &root, self)?;
        self.freshness_from_lifecycle(&state, &root, lifecycle)
    }

    pub fn freshness_under_admission(
        &self,
        admission: &SessionAdmission,
    ) -> Result<SessionFreshness, ClewError> {
        if admission.session_id != self.session_id {
            return Err(invalid("session admission belongs to another session"));
        }
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let lifecycle = load_session_lifecycle_unlocked(&state, &root, self)?;
        self.freshness_from_lifecycle(&state, &root, lifecycle)
    }

    fn freshness_from_lifecycle(
        &self,
        state: &StateAuthority,
        root: &Path,
        lifecycle: SessionLifecycle,
    ) -> Result<SessionFreshness, ClewError> {
        if lifecycle.status != SessionStatus::Open {
            return Ok(classify_freshness(
                &self.session_id,
                lifecycle.status,
                None,
                None,
                None,
            ));
        }
        let runs = load_session_runs(&state, &root, self)?;
        let expected = session_terminal_target_oid(self, &runs)?;
        let repository = self.target_repository_path()?;
        let head = isolated_git_text(&repository, &["rev-parse", "--verify", "HEAD^{commit}"]);
        let target = isolated_git_text(
            &repository,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", self.target_ref),
            ],
        );
        let clean = isolated_git_bytes(
            &repository,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )
        .map(|value| value.is_empty());
        Ok(classify_freshness(
            &self.session_id,
            lifecycle.status,
            head.as_deref().map(|value| value == expected),
            target.as_deref().map(|value| value == expected),
            clean,
        ))
    }

    pub fn repository_path(&self) -> Result<PathBuf, ClewError> {
        if self.language == SessionLanguage::Python {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "Python sessions have no materialized source worktree",
            ));
        }
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let locator: RepositoryLocator =
            read_managed_json(&state, &root.join("locator.json"), MAX_PLAN_BYTES)?;
        validate_locator(&locator)?;
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

    pub(crate) fn python_source_snapshot(
        &self,
        store: &CasStore,
    ) -> Result<(RepositoryInputSnapshot, CasObject), ClewError> {
        if self.language != SessionLanguage::Python {
            return Err(invalid("Python source authority requires a Python session"));
        }
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let binding: PythonSourceBinding =
            read_managed_json(&state, &root.join("python-source.json"), MAX_PLAN_BYTES)?;
        if binding.schema != PYTHON_SOURCE_BINDING_SCHEMA
            || binding.session_id != self.session_id
            || binding.session_authority_digest != self.authority_digest
            || binding.base_revision != self.base_revision
            || binding.snapshot.object_schema != SNAPSHOT_SCHEMA
        {
            return Err(invalid("Python session source binding is invalid"));
        }
        let limit = usize::try_from(binding.snapshot.size)
            .map_err(|_| invalid("Python session snapshot exceeds host size"))?;
        if limit > 16 * 1024 * 1024 {
            return Err(invalid(
                "Python session snapshot exceeds its metadata budget",
            ));
        }
        let lease = store.read(&binding.snapshot, limit)?;
        let snapshot: RepositoryInputSnapshot = serde_json::from_slice(lease.bytes())
            .map_err(|_| invalid("Python session snapshot is invalid"))?;
        if canonical::bytes(&snapshot).map_err(internal)? != lease.bytes() {
            return Err(invalid("Python session snapshot is not canonical"));
        }
        snapshot.verify()?;
        Ok((snapshot, binding.snapshot))
    }

    pub fn target_repository_path(&self) -> Result<PathBuf, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let locator: RepositoryLocator =
            read_managed_json(&state, &root.join("locator.json"), MAX_PLAN_BYTES)?;
        validate_locator(&locator)?;
        let path = locator
            .target_repository_path
            .canonicalize()
            .map_err(io_error)?;
        if state.repository(&path)?.key != self.repository_key {
            return Err(invalid("target repository locator is invalid"));
        }
        Ok(path)
    }

    pub fn external_build_state_path(&self) -> Result<Option<PathBuf>, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let locator: RepositoryLocator =
            read_managed_json(&state, &root.join("locator.json"), MAX_PLAN_BYTES)?;
        validate_locator(&locator)?;
        match (self.model_cache_policy, locator.external_build_state_path) {
            (ModelCachePolicy::SealedExternal, Some(path)) => {
                let canonical = path.canonicalize().map_err(io_error)?;
                if canonical != path {
                    return Err(invalid("external build-state locator changed"));
                }
                Ok(Some(canonical))
            }
            (ModelCachePolicy::SealedExternal, None) => {
                Err(invalid("sealed external build-state locator is missing"))
            }
            (_, None) => Ok(None),
            (_, Some(_)) => Err(invalid("unexpected external build-state locator")),
        }
    }

    pub fn store_context(
        &self,
        parent_context_id: Option<String>,
        intent: String,
        mut terms: Vec<String>,
        projection: Value,
        evidence: Value,
    ) -> Result<ContextObject, ClewError> {
        validate_context_request(&intent, &terms)?;
        crate::context_v2::validate_context_payload(&projection, &evidence)?;
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
        let root = state.session_root(&self.session_id)?;
        let _lock = SessionLifecycleLock::acquire(&state, &root)?;
        if load_session_lifecycle_unlocked(&state, &root, self)?.status != SessionStatus::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session is terminal and cannot store context",
            ));
        }
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
        store_cas_json(&state, &root, &object.context_id, &object)?;
        state.write_private_atomic(
            &root.join("contexts").join(id_filename(&object.context_id)?),
            &canonical::bytes(&object).map_err(internal)?,
        )?;
        Ok(object)
    }

    pub fn load_context(&self, context_id: &str) -> Result<ContextObject, ClewError> {
        reject_thread_context_for_mutation(context_id)?;
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let mut object: ContextObject = read_managed_json(
            &state,
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
        crate::context_v2::validate_context_payload(&object.projection, &object.evidence)?;
        Ok(object)
    }

    pub fn validate_plan(&self, context_id: &str, source: &[u8]) -> Result<PlanObject, ClewError> {
        reject_thread_context_for_mutation(context_id)?;
        if source.len() > MAX_PLAN_BYTES {
            return Err(invalid("plan exceeds the 1 MiB limit"));
        }
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let _lock = SessionLifecycleLock::acquire(&state, &root)?;
        if load_session_lifecycle_unlocked(&state, &root, self)?.status != SessionStatus::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session is terminal and cannot validate a plan",
            ));
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
        store_cas_json(&state, &root, &object.plan_id, &object)?;
        state.write_private_atomic(
            &root.join("plans").join(id_filename(&object.plan_id)?),
            &canonical::bytes(&object).map_err(internal)?,
        )?;
        Ok(object)
    }

    pub fn load_plan(&self, plan_id: &str) -> Result<PlanObject, ClewError> {
        reject_thread_coverage_for_mutation(plan_id)?;
        let state = StateAuthority::process_default()?;
        let root = state.session_root(&self.session_id)?;
        let object: PlanObject = read_managed_json(
            &state,
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
        reject_thread_coverage_for_mutation(plan_id)?;
        reject_thread_context_for_mutation(context_id)?;
        self.require_open()?;
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

fn open_session_admission_with_state(
    authority: &SessionAuthority,
    state: &StateAuthority,
    root: &Path,
) -> Result<SessionAdmission, ClewError> {
    let lock = SessionLifecycleLock::acquire(state, root)?;
    if load_session_lifecycle_unlocked(state, root, authority)?.status != SessionStatus::Open {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "session is terminal and cannot admit work",
        ));
    }
    Ok(SessionAdmission {
        session_id: authority.session_id.clone(),
        _lock: lock,
    })
}

struct SessionLifecycleLock(File);

impl SessionLifecycleLock {
    fn acquire(state: &StateAuthority, root: &Path) -> Result<Self, ClewError> {
        let directory = state.directory_at(root)?;
        let file = directory.open_lock(std::ffi::OsStr::new("lifecycle.lock"))?;
        #[cfg(unix)]
        if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self(file))
    }
}

impl Drop for SessionLifecycleLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&self.0), libc::LOCK_UN);
        }
    }
}

fn initialize_session_lifecycle(
    state: &StateAuthority,
    root: &Path,
    authority: &SessionAuthority,
) -> Result<(), ClewError> {
    let _lock = SessionLifecycleLock::acquire(state, root)?;
    if state.private_file_exists(&root.join("lifecycle.jsonl"))?
        || state.private_file_exists(&root.join("lifecycle.json"))?
    {
        return Err(invalid("session lifecycle already exists"));
    }
    let entry = SessionLifecycle {
        schema: SESSION_LIFECYCLE_SCHEMA.into(),
        session_id: authority.session_id.clone(),
        session_authority_digest: authority.authority_digest.clone(),
        sequence: 0,
        previous_event_hash: None,
        status: SessionStatus::Open,
        event_hash: String::new(),
        updated_unix_ms: unix_ms(),
    };
    append_session_lifecycle(state, root, entry)
}

fn load_session_lifecycle(
    state: &StateAuthority,
    root: &Path,
    authority: &SessionAuthority,
) -> Result<SessionLifecycle, ClewError> {
    let _lock = SessionLifecycleLock::acquire(state, root)?;
    load_session_lifecycle_unlocked(state, root, authority)
}

fn load_session_lifecycle_unlocked(
    state: &StateAuthority,
    root: &Path,
    authority: &SessionAuthority,
) -> Result<SessionLifecycle, ClewError> {
    let path = root.join("lifecycle.jsonl");
    let bytes = state
        .read_private_file(&path, MAX_SESSION_LIFECYCLE_BYTES as usize)
        .map_err(|_| invalid("session lifecycle ledger is missing or unsafe"))?;
    if bytes.is_empty() {
        return Err(invalid("session lifecycle ledger is missing or unsafe"));
    }
    if bytes.last() != Some(&b'\n') {
        return Err(invalid("session lifecycle ledger changed while reading"));
    }
    let mut previous: Option<SessionLifecycle> = None;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: SessionLifecycle = serde_json::from_slice(line)
            .map_err(|_| invalid("session lifecycle entry is invalid"))?;
        if canonical::bytes(&entry).map_err(internal)? != line
            || entry.schema != SESSION_LIFECYCLE_SCHEMA
            || entry.session_id != authority.session_id
            || entry.session_authority_digest != authority.authority_digest
            || !sha256_digest(&entry.event_hash)
            || entry.event_hash != session_lifecycle_hash(&entry)?
        {
            return Err(invalid("session lifecycle authority is invalid"));
        }
        match &previous {
            None => {
                if entry.sequence != 0
                    || entry.previous_event_hash.is_some()
                    || entry.status != SessionStatus::Open
                {
                    return Err(invalid("session lifecycle genesis is invalid"));
                }
            }
            Some(prior) => {
                if entry.sequence != prior.sequence.saturating_add(1)
                    || entry.previous_event_hash.as_deref() != Some(prior.event_hash.as_str())
                    || !session_transition_allowed(prior.status, entry.status)
                {
                    return Err(invalid("session lifecycle chain is invalid"));
                }
            }
        }
        previous = Some(entry);
    }
    let current = previous.ok_or_else(|| invalid("session lifecycle ledger is empty"))?;
    let projection_path = root.join("lifecycle.json");
    let projection_matches =
        read_managed_json::<SessionLifecycle>(state, &projection_path, MAX_PLAN_BYTES).is_ok_and(
            |projection| canonical::bytes(&projection).ok() == canonical::bytes(&current).ok(),
        );
    if !projection_matches {
        state.write_private_atomic(
            &projection_path,
            &canonical::bytes(&current).map_err(internal)?,
        )?;
    }
    Ok(current)
}

fn append_session_lifecycle(
    state: &StateAuthority,
    root: &Path,
    mut entry: SessionLifecycle,
) -> Result<(), ClewError> {
    entry.event_hash.clear();
    entry.event_hash = session_lifecycle_hash(&entry)?;
    let mut bytes = canonical::bytes(&entry).map_err(internal)?;
    bytes.push(b'\n');
    let directory = state.directory_at(root)?;
    let mut ledger = directory.open_append(std::ffi::OsStr::new("lifecycle.jsonl"))?;
    if ledger
        .metadata()
        .map_err(io_error)?
        .len()
        .saturating_add(bytes.len() as u64)
        > MAX_SESSION_LIFECYCLE_BYTES
    {
        return Err(invalid("session lifecycle ledger is unsafe or oversized"));
    }
    ledger.write_all(&bytes).map_err(io_error)?;
    ledger.sync_all().map_err(io_error)?;
    state.write_private_atomic(
        &root.join("lifecycle.json"),
        &canonical::bytes(&entry).map_err(internal)?,
    )
}

fn session_lifecycle_hash(entry: &SessionLifecycle) -> Result<String, ClewError> {
    let mut unsigned = entry.clone();
    unsigned.event_hash.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn session_transition_allowed(from: SessionStatus, to: SessionStatus) -> bool {
    matches!(
        (from, to),
        (
            SessionStatus::Open,
            SessionStatus::Closed | SessionStatus::Aborted
        ) | (
            SessionStatus::Closed | SessionStatus::Aborted,
            SessionStatus::GarbageCollected
        )
    )
}

fn transition_session_terminal(
    authority: &SessionAuthority,
    target: SessionStatus,
) -> Result<SessionLifecycle, ClewError> {
    if !matches!(target, SessionStatus::Closed | SessionStatus::Aborted) {
        return Err(invalid("invalid terminal session transition"));
    }
    let state = StateAuthority::process_default()?;
    let root = state.session_root(&authority.session_id)?;
    transition_session_terminal_with_state(authority, target, &state, &root)
}

fn transition_session_terminal_with_state(
    authority: &SessionAuthority,
    target: SessionStatus,
    state: &StateAuthority,
    root: &Path,
) -> Result<SessionLifecycle, ClewError> {
    let _lock = SessionLifecycleLock::acquire(state, root)?;
    let current = load_session_lifecycle_unlocked(state, root, authority)?;
    if current.status == target {
        return Ok(current);
    }
    if current.status != SessionStatus::Open {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "session already has a different terminal status",
        ));
    }
    let runs = load_session_runs(state, root, authority)?;
    if runs
        .iter()
        .any(|run| run_status_is_active(run.status) || run.process_id.is_some())
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "session has an active run; publish, recover, or cancel it before closing",
        ));
    }
    let _ = session_terminal_target_oid(authority, &runs)?;
    let next = SessionLifecycle {
        schema: SESSION_LIFECYCLE_SCHEMA.into(),
        session_id: authority.session_id.clone(),
        session_authority_digest: authority.authority_digest.clone(),
        sequence: current.sequence.saturating_add(1),
        previous_event_hash: Some(current.event_hash),
        status: target,
        event_hash: String::new(),
        updated_unix_ms: unix_ms(),
    };
    append_session_lifecycle(state, root, next)?;
    load_session_lifecycle_unlocked(state, root, authority)
}

fn run_status_is_active(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Created
            | RunStatus::Preparing
            | RunStatus::ReadyToPublish
            | RunStatus::ReadyToPublishConditional
            | RunStatus::Publishing
            | RunStatus::WorktreeRecoveryRequired
    )
}

fn session_terminal_target_oid(
    authority: &SessionAuthority,
    runs: &[RunRecord],
) -> Result<String, ClewError> {
    let mut published = runs
        .iter()
        .filter(|run| {
            matches!(
                run.status,
                RunStatus::Published | RunStatus::PublishedConditional
            )
        })
        .map(|run| {
            run.final_commit
                .clone()
                .ok_or_else(|| invalid("published run has no final commit authority"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    published.sort();
    published.dedup();
    match published.as_slice() {
        [] => Ok(authority.target_oid.clone()),
        [oid] => Ok(oid.clone()),
        _ => Err(invalid(
            "session contains conflicting published target commits",
        )),
    }
}

fn ensure_session_run_reference(
    state: &StateAuthority,
    session_root: &Path,
    session: &SessionAuthority,
    run: &RunRecord,
) -> Result<(), ClewError> {
    let reference = SessionRunReference {
        schema: SESSION_RUN_REFERENCE_SCHEMA.into(),
        session_id: session.session_id.clone(),
        session_authority_digest: session.authority_digest.clone(),
        run_id: run.run_id.clone(),
        request_digest: run.request_digest.clone(),
    };
    let path = session_root
        .join("runs")
        .join(format!("{}.json", id_component(&run.run_id, "run:")?));
    if state.private_file_exists(&path)? {
        let existing: SessionRunReference = read_managed_json(state, &path, MAX_PLAN_BYTES)?;
        if canonical::bytes(&existing).map_err(internal)?
            != canonical::bytes(&reference).map_err(internal)?
        {
            return Err(invalid(
                "session run reference conflicts with immutable authority",
            ));
        }
        return Ok(());
    }
    state.write_private_atomic(&path, &canonical::bytes(&reference).map_err(internal)?)
}

fn load_session_runs(
    state: &StateAuthority,
    session_root: &Path,
    session: &SessionAuthority,
) -> Result<Vec<RunRecord>, ClewError> {
    let references_root = session_root.join("runs");
    let references = state.directory_at(&references_root)?.entries()?;
    let mut runs = Vec::with_capacity(references.len());
    for name in references {
        let path = references_root.join(&name);
        let reference: SessionRunReference = read_managed_json(state, &path, MAX_PLAN_BYTES)?;
        if reference.schema != SESSION_RUN_REFERENCE_SCHEMA
            || reference.session_id != session.session_id
            || reference.session_authority_digest != session.authority_digest
            || Some(name.as_os_str())
                != Some(std::ffi::OsStr::new(&format!(
                    "{}.json",
                    id_component(&reference.run_id, "run:")?
                )))
        {
            return Err(invalid("session run reference authority is invalid"));
        }
        let run_root = state.run_root(&reference.run_id)?;
        let _run_lock = RunLedgerLock::acquire(state, &run_root)?;
        let run = load_run_projection(state, &run_root, &reference.run_id)?;
        if run.session_id != session.session_id || run.request_digest != reference.request_digest {
            return Err(invalid("session run ledger does not match its reference"));
        }
        runs.push(run);
    }
    Ok(runs)
}

#[derive(Debug)]
struct ManagedWorktreeRemoval {
    path: PathBuf,
    allow_forced_removal: bool,
}

fn garbage_collect_session(
    authority: &SessionAuthority,
    force: bool,
) -> Result<SessionLifecycle, ClewError> {
    let state = StateAuthority::process_default()?;
    let root = state.session_root(&authority.session_id)?;
    garbage_collect_session_with_state(authority, force, &state, &root)
}

fn garbage_collect_session_with_state(
    authority: &SessionAuthority,
    force: bool,
    state: &StateAuthority,
    root: &Path,
) -> Result<SessionLifecycle, ClewError> {
    let _lock = SessionLifecycleLock::acquire(state, root)?;
    let current = load_session_lifecycle_unlocked(state, root, authority)?;
    if current.status == SessionStatus::GarbageCollected {
        return Ok(current);
    }
    if !matches!(
        current.status,
        SessionStatus::Closed | SessionStatus::Aborted
    ) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "session GC requires a closed or aborted session",
        ));
    }
    let runs = load_session_runs(state, root, authority)?;
    if runs
        .iter()
        .any(|run| run_status_is_active(run.status) || run.process_id.is_some())
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "session GC refuses active runs",
        ));
    }
    let session_directory = state.directory_at(root)?;
    // Git requires a path, so destructive worktree operations are allowed
    // only while that path still names the exact directory authority pinned
    // by StateAuthority. A renamed/replaced state root fails closed here.
    session_directory.require_path_identity()?;
    let locator: RepositoryLocator =
        read_managed_json(state, &root.join("locator.json"), MAX_PLAN_BYTES)?;
    validate_locator(&locator)?;
    let target = locator
        .target_repository_path
        .canonicalize()
        .map_err(io_error)?;
    let expected_target_oid = session_terminal_target_oid(authority, &runs)?;
    let current_target_oid = git_output(&target, &["rev-parse", &authority.target_ref])?;
    let expected_is_ancestor = if current_target_oid == expected_target_oid {
        true
    } else {
        let output = Command::new("git")
            .args([
                "merge-base",
                "--is-ancestor",
                &expected_target_oid,
                &current_target_oid,
            ])
            .current_dir(&target)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(io_error)?;
        match output.status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => return Err(invalid("Git target ancestry is unavailable")),
        }
    };
    if state.repository(&target)?.key != authority.repository_key || !expected_is_ancestor {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "session target authority diverged before GC",
        ));
    }

    // Validate every relevant worktree before deleting any of them. This makes
    // GC fail closed rather than partially consuming an uncommitted candidate.
    let mut removals = Vec::new();
    let source = root.join("source");
    if source.exists() {
        let metadata = fs::symlink_metadata(&source).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("session source worktree is unsafe"));
        }
        let canonical = source.canonicalize().map_err(io_error)?;
        if !canonical.starts_with(root)
            || git_output(&canonical, &["rev-parse", "HEAD"])? != authority.base_revision
            || !managed_source_clean(&canonical)?
        {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session source worktree is not immutable and clean",
            ));
        }
        removals.push(ManagedWorktreeRemoval {
            path: canonical,
            allow_forced_removal: true,
        });
    }
    let expected_candidates = runs
        .iter()
        .map(|run| id_component(&run.run_id, "run:").map(str::to_owned))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    let candidates_directory = session_directory.child(Path::new("candidates"))?;
    for name in candidates_directory.entries()? {
        let name = name
            .into_string()
            .map_err(|_| invalid("candidate directory name is not UTF-8"))?;
        if !expected_candidates.contains(&name) {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session contains an unregistered candidate state directory",
            ));
        }
        let candidate_directory = candidates_directory.child(Path::new(&name))?;
        for entry in candidate_directory.entries()? {
            if entry == std::ffi::OsStr::new("worktree") {
                continue;
            }
            if candidate_derived_state_name(&entry) && candidate_directory.file_exists(&entry)? {
                continue;
            }
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "candidate contains unknown state; GC refuses deletion",
            ));
        }
    }
    let mut derived_output_cleanups = Vec::new();
    for run in &runs {
        let candidate_root = root
            .join("candidates")
            .join(id_component(&run.run_id, "run:")?);
        let worktree = candidate_root.join("worktree");
        if !worktree.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&worktree).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("candidate worktree is unsafe"));
        }
        let canonical = worktree.canonicalize().map_err(io_error)?;
        if !canonical.starts_with(&candidate_root) {
            return Err(invalid("candidate worktree escapes managed session state"));
        }
        let expected_head = run
            .candidate_commit
            .as_deref()
            .unwrap_or(authority.base_revision.as_str());
        if git_output(&canonical, &["rev-parse", "HEAD"])? != expected_head {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "candidate HEAD differs from its run authority",
            ));
        }
        let untracked = candidate_untracked_after_clean_tracked_check(&canonical)?;
        if !untracked.is_empty() {
            if !matches!(
                run.status,
                RunStatus::Published | RunStatus::PublishedConditional
            ) || run.final_commit.as_deref() != run.candidate_commit.as_deref()
                || (run.status == RunStatus::PublishedConditional
                    && run.conditional_approval.is_none())
            {
                return Err(ClewError::new(
                    ErrorCode::PreconditionFailed,
                    if force {
                        "candidate is non-empty and has no exact Codeclew-owned derived-output receipt; --force cannot delete it"
                    } else {
                        "candidate is non-empty; remove its untracked outputs before GC"
                    },
                ));
            }
            let prepared: crate::task_run_v2::PreparedCandidateV2 = read_managed_json(
                state,
                &state.run_root(&run.run_id)?.join("prepared-v2.json"),
                16 * 1024 * 1024,
            )?;
            if run.candidate_snapshot.as_ref() != Some(&prepared.candidate_snapshot) {
                return Err(ClewError::new(
                    ErrorCode::PreconditionFailed,
                    "published run snapshot differs from prepared cleanup authority",
                ));
            }
            crate::task_run_v2::verify_prepared_for_gc(authority, &prepared, &canonical)?;
            derived_output_cleanups.push((canonical.clone(), prepared.derived_outputs));
        }
        removals.push(ManagedWorktreeRemoval {
            path: canonical,
            // Filtered legacy paths make Git consider the managed checkout
            // incomplete, so the actual removal uses --force only after the
            // stricter product-level verification above has succeeded.
            allow_forced_removal: true,
        });
    }

    for (worktree, expected) in &derived_output_cleanups {
        crate::task_run_v2::remove_exact_derived_outputs(worktree, expected)?;
    }
    for removal in removals {
        remove_managed_worktree(&target, &removal)?;
    }
    for run in &runs {
        cleanup_candidate_derived_state_with_authority(state, root, run)?;
        cleanup_run_derived_state(state, run)?;
    }
    let next = SessionLifecycle {
        schema: SESSION_LIFECYCLE_SCHEMA.into(),
        session_id: authority.session_id.clone(),
        session_authority_digest: authority.authority_digest.clone(),
        sequence: current.sequence.saturating_add(1),
        previous_event_hash: Some(current.event_hash),
        status: SessionStatus::GarbageCollected,
        event_hash: String::new(),
        updated_unix_ms: unix_ms(),
    };
    append_session_lifecycle(state, root, next)?;
    load_session_lifecycle_unlocked(state, root, authority)
}

fn managed_source_clean(root: &Path) -> Result<bool, ClewError> {
    match candidate_untracked_after_clean_tracked_check(root) {
        Ok(paths) => Ok(paths.is_empty()),
        Err(error) if error.code == ErrorCode::PreconditionFailed => Ok(false),
        Err(error) => Err(error),
    }
}

fn candidate_untracked_after_clean_tracked_check(root: &Path) -> Result<Vec<String>, ClewError> {
    let output = Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--",
            ".",
        ])
        .args(LEGACY_EXCLUDES)
        .current_dir(root)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("candidate cleanliness is unavailable"));
    }
    let mut untracked = Vec::new();
    for row in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        if row.len() < 4 || row[2] != b' ' {
            return Err(invalid("candidate Git status row is invalid"));
        }
        if &row[..2] != b"??" && &row[..2] != b"!!" {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "candidate has staged or tracked changes and cannot be garbage-collected",
            ));
        }
        let path = std::str::from_utf8(&row[3..])
            .map_err(|_| invalid("candidate untracked path is not UTF-8"))?;
        if path.is_empty()
            || Path::new(path).is_absolute()
            || Path::new(path)
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(invalid("candidate untracked path is unsafe"));
        }
        untracked.push(path.into());
    }
    Ok(untracked)
}

fn remove_managed_worktree(
    repository: &Path,
    removal: &ManagedWorktreeRemoval,
) -> Result<(), ClewError> {
    make_managed_worktree_removable(&removal.path)?;
    let mut command = Command::new("git");
    command.args(["-c", "core.hooksPath=/dev/null", "worktree", "remove"]);
    if removal.allow_forced_removal {
        command.arg("--force");
    }
    let output = command
        .arg(&removal.path)
        .current_dir(repository)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "managed worktree removal failed without modifying its contents",
        ));
    }
    Ok(())
}

fn make_managed_worktree_removable(root: &Path) -> Result<(), ClewError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let owner = unsafe { libc::geteuid() };
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|error| internal(error.to_string()))?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.uid() != owner {
                return Err(invalid(
                    "managed worktree contains an entry owned by another user",
                ));
            }
            if metadata.is_dir() {
                fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700))
                    .map_err(io_error)?;
            } else if metadata.is_file() {
                let executable = metadata.permissions().mode() & 0o111 != 0;
                fs::set_permissions(
                    entry.path(),
                    fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
                )
                .map_err(io_error)?;
            } else {
                return Err(invalid("managed worktree contains an unsupported entry"));
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err(invalid("managed worktree GC requires POSIX"))
    }
}

fn cleanup_candidate_derived_state_with_authority(
    state: &StateAuthority,
    root: &Path,
    run: &RunRecord,
) -> Result<(), ClewError> {
    let relative = Path::new("candidates").join(id_component(&run.run_id, "run:")?);
    let session = state.directory_at(root)?;
    let candidate_directory = session.child(&relative)?;
    for entry in candidate_directory.entries()? {
        if candidate_derived_state_name(&entry) && candidate_directory.file_exists(&entry)? {
            candidate_directory.remove_file(&entry)?;
        }
    }
    let unexpected = candidate_directory.entries()?;
    if unexpected.is_empty() {
        return Ok(());
    }
    Err(ClewError::new(
        ErrorCode::PreconditionFailed,
        "candidate contains unknown state; GC refuses recursive deletion",
    ))
}

fn candidate_derived_state_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if matches!(
        name,
        "checkpoint-v2.json" | "staged-generation.json" | "staged-workspace-profile.json"
    ) {
        return true;
    }
    name.strip_prefix("staged-generation-")
        .and_then(|value| value.strip_suffix(".json"))
        .is_some_and(|component| {
            component.len() == 64
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

fn cleanup_run_derived_state(state: &StateAuthority, run: &RunRecord) -> Result<(), ClewError> {
    let root = state.run_root(&run.run_id)?;
    let directory = state.directory_at(&root)?;
    for name in ["prepared-v2.json", "stdout.log", "stderr.log"] {
        let name = std::ffi::OsStr::new(name);
        if directory.file_exists(name)? {
            directory.remove_file(name)?;
        }
    }
    Ok(())
}

impl RunRecord {
    pub fn created(
        session: &SessionAuthority,
        context_id: &str,
        plan_id: &str,
    ) -> Result<Self, ClewError> {
        reject_thread_coverage_for_mutation(plan_id)?;
        reject_thread_context_for_mutation(context_id)?;
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
            prepared_authority_digest: None,
            final_commit: None,
            publication_blocked: false,
            conditional_approval: None,
            process_id: None,
            process_start_token: None,
            failure: None,
            updated_unix_ms: unix_ms(),
        })
    }

    pub fn load(run_id: &str) -> Result<Self, ClewError> {
        reject_thread_coverage_for_mutation(run_id)?;
        let state = StateAuthority::process_default()?;
        let root = state.run_root(run_id)?;
        let _lock = RunLedgerLock::acquire(&state, &root)?;
        load_run_projection(&state, &root, run_id)
    }

    pub fn save(&mut self) -> Result<(), ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.run_root(&self.run_id)?;
        save_run_transition(&state, &root, self)
    }

    pub fn create_once(&self) -> Result<bool, ClewError> {
        let state = StateAuthority::process_default()?;
        let (session, session_root) = SessionAuthority::load(&self.session_id)?;
        let _session_lock = SessionLifecycleLock::acquire(&state, &session_root)?;
        let lifecycle = load_session_lifecycle_unlocked(&state, &session_root, &session)?;
        if lifecycle.status != SessionStatus::Open {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "session is terminal and cannot start a run",
            ));
        }
        ensure_session_run_reference(&state, &session_root, &session, self)?;
        let root = state.run_root(&self.run_id)?;
        let _lock = RunLedgerLock::acquire(&state, &root)?;
        if state.private_file_exists(&root.join("ledger.jsonl"))?
            || state.private_file_exists(&root.join("record.json"))?
        {
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
        let state = StateAuthority::process_default()?;
        let (_, session_root) = SessionAuthority::load(&self.session_id)?;
        let session = state.directory_at(&session_root)?;
        session.require_path_identity()?;
        let root = session_root
            .join("candidates")
            .join(id_component(&self.run_id, "run:")?);
        session.child(&Path::new("candidates").join(id_component(&self.run_id, "run:")?))?;
        Ok(root)
    }
}

fn reject_thread_context_for_mutation(context_id: &str) -> Result<(), ClewError> {
    reject_thread_coverage_for_mutation(context_id)?;
    if context_id.starts_with("thread-context:") {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "thread contexts are read-only analysis evidence and cannot authorize mutation",
        ));
    }
    Ok(())
}

fn reject_thread_coverage_for_mutation(value: &str) -> Result<(), ClewError> {
    if value.starts_with("thread-coverage:") {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "thread coverage is read-only analysis evidence and cannot authorize mutation",
        ));
    }
    Ok(())
}

fn save_run_transition(
    state: &StateAuthority,
    root: &Path,
    record: &mut RunRecord,
) -> Result<(), ClewError> {
    let _lock = RunLedgerLock::acquire(state, root)?;
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
    fn acquire(state: &StateAuthority, root: &Path) -> Result<Self, ClewError> {
        let directory = state.directory_at(root)?;
        let file = directory.open_lock(std::ffi::OsStr::new("ledger.lock"))?;
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
    let directory = state.directory_at(root)?;
    let mut ledger = directory.open_append(std::ffi::OsStr::new("ledger.jsonl"))?;
    if ledger
        .metadata()
        .map_err(io_error)?
        .len()
        .saturating_add(bytes.len() as u64)
        > MAX_RUN_LEDGER_BYTES
    {
        return Err(invalid("run ledger is unsafe or exceeds its limit"));
    }
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
    let bytes = state
        .read_private_file(&path, MAX_RUN_LEDGER_BYTES as usize)
        .map_err(|_| invalid("run ledger is missing or unsafe"))?;
    if bytes.is_empty() {
        return Err(invalid("run ledger is missing or unsafe"));
    }
    if bytes.last() != Some(&b'\n') {
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
    let projection_matches =
        read_managed_json::<RunRecord>(state, &projection_path, MAX_PLAN_BYTES).is_ok_and(
            |record| canonical::bytes(&record).ok() == canonical::bytes(&projected).ok(),
        );
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
        || (left.prepared_authority_digest.is_some()
            && left.prepared_authority_digest != right.prepared_authority_digest)
        || (left.final_commit.is_some() && left.final_commit != right.final_commit)
        || (left.publication_blocked && !right.publication_blocked)
        || (left.conditional_approval.is_some()
            && left.conditional_approval != right.conditional_approval)
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
                    | RunStatus::ReadyToPublishConditional
                    | RunStatus::ValidatedConditional
                    | RunStatus::Failed
                    | RunStatus::WorktreeRecoveryRequired
                    | RunStatus::Cancelled
            ) | (
                RunStatus::Failed | RunStatus::Cancelled,
                RunStatus::Created | RunStatus::WorktreeRecoveryRequired
            ) | (
                RunStatus::ReadyToPublish | RunStatus::ReadyToPublishConditional,
                RunStatus::Publishing | RunStatus::Cancelled
            ) | (
                RunStatus::Publishing,
                RunStatus::Published
                    | RunStatus::PublishedConditional
                    | RunStatus::ReadyToPublish
                    | RunStatus::ReadyToPublishConditional
                    | RunStatus::WorktreeRecoveryRequired
            ) | (
                RunStatus::WorktreeRecoveryRequired,
                RunStatus::ReadyToPublish
                    | RunStatus::ReadyToPublishConditional
                    | RunStatus::ValidatedConditional
                    | RunStatus::Published
                    | RunStatus::PublishedConditional
                    | RunStatus::Failed
                    | RunStatus::WorktreeRecoveryRequired
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
        "schema":"codeclew-context-result/2.0",
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
    validate_context_stdout_bytes(&bytes)?;
    Ok(summary)
}

fn validate_context_stdout_bytes(bytes: &[u8]) -> Result<(), ClewError> {
    // main emits canonical compact JSON followed by println!'s LF.
    if bytes.len().saturating_add(1) > MAX_CONTEXT_STDOUT_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "context stdout exceeds 64 KiB",
        ));
    }
    Ok(())
}

fn validate_plan_shape(plan: &Value) -> Result<(), ClewError> {
    if !crate::text_authority::json_strings_are_nfc(plan, 0) {
        return Err(invalid("plan keys and strings must use NFC Unicode"));
    }
    crate::task_run_v2::validate_plan_value(plan)?;
    Ok(())
}

pub fn validate_context_request(intent: &str, terms: &[String]) -> Result<(), ClewError> {
    if intent.trim().is_empty()
        || intent.len() > 32 * 1024
        || intent.contains('\0')
        || !crate::text_authority::is_nfc(intent)
    {
        return Err(invalid(
            "context intent must be non-empty NFC text no larger than 32 KiB",
        ));
    }
    let total = terms.iter().try_fold(0usize, |total, term| {
        total
            .checked_add(term.len())
            .ok_or_else(|| invalid("context term size overflow"))
    })?;
    if terms.is_empty()
        || terms.len() > 256
        || total > 64 * 1024
        || terms.iter().any(|term| {
            term.trim().is_empty()
                || term.len() > 4096
                || term.chars().any(char::is_control)
                || !crate::text_authority::is_nfc(term)
        })
    {
        return Err(invalid(
            "context terms must be bounded NFC identities without control characters",
        ));
    }
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
    if state.private_file_exists(&path)? {
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

fn write_managed_json_create_new<T: Serialize>(
    state: &StateAuthority,
    path: &Path,
    value: &T,
) -> Result<(), ClewError> {
    let relative = path
        .strip_prefix(state.root())
        .map_err(|_| invalid("managed JSON path escapes state authority"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("managed JSON path has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("managed JSON path has no file name"))?;
    let mut file = state.directory(parent)?.create_file(name)?;
    file.write_all(&canonical::bytes(value).map_err(internal)?)
        .map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn read_managed_json<T: for<'de> Deserialize<'de>>(
    state: &StateAuthority,
    path: &Path,
    limit: usize,
) -> Result<T, ClewError> {
    let bytes = state
        .read_private_file(path, limit)
        .map_err(|_| invalid("managed JSON object is missing or exceeds its limit"))?;
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

fn isolated_git_bytes(repo: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    let mut command = isolated_git_command(repo);
    let output = command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn isolated_git_text(repo: &Path, arguments: &[&str]) -> Option<String> {
    String::from_utf8(isolated_git_bytes(repo, arguments)?)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn isolated_git_output(repo: &Path, arguments: &[&str]) -> Result<String, ClewError> {
    let mut command = isolated_git_command(repo);
    let output = command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("isolated Git authority is unavailable"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().into())
        .map_err(|_| invalid("isolated Git authority is not UTF-8"))
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn classify_freshness(
    session_id: &str,
    lifecycle_status: SessionStatus,
    head_matches_expected: Option<bool>,
    target_ref_matches_expected: Option<bool>,
    target_worktree_clean: Option<bool>,
) -> SessionFreshness {
    let (status, remediation_id) = if lifecycle_status != SessionStatus::Open {
        ("TERMINAL", "NO_ACTION")
    } else if head_matches_expected.is_none()
        || target_ref_matches_expected.is_none()
        || target_worktree_clean.is_none()
    {
        ("UNAVAILABLE", "CHECK_REPOSITORY_LOCATOR")
    } else if head_matches_expected == Some(false) || target_ref_matches_expected == Some(false) {
        ("STALE", "OPEN_NEW_SESSION")
    } else if target_worktree_clean == Some(false) {
        ("DIRTY", "CLEAN_TARGET_WORKTREE")
    } else {
        ("FRESH", "NONE")
    };
    SessionFreshness {
        schema: "codeclew-session-freshness/1.0".into(),
        session_id: session_id.into(),
        lifecycle_status,
        status: status.into(),
        head_matches_expected,
        target_ref_matches_expected,
        target_worktree_clean,
        remediation_id: remediation_id.into(),
    }
}

fn session_authority_digest(authority: &SessionAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn model_cache_authority(
    repository: &Path,
    selected_compilations: &[String],
    policy: ModelCachePolicy,
    runtime_mode: RuntimeMode,
    external_build_state: Option<&Path>,
) -> Result<(Option<String>, Option<PathBuf>), ClewError> {
    match policy {
        ModelCachePolicy::NonCacheable => {
            if external_build_state.is_some() {
                return Err(invalid(
                    "external build state requires the sealed-external model-cache policy",
                ));
            }
            Ok((None, None))
        }
        ModelCachePolicy::TrackedManifest => {
            if external_build_state.is_some() {
                return Err(invalid(
                    "tracked-manifest policy cannot use external build state",
                ));
            }
            let relative = "codeclew.model-cache.json";
            for arguments in [
                vec!["ls-files", "--error-unmatch", "--", relative],
                vec!["diff", "--quiet", "HEAD", "--", relative],
                vec!["diff", "--cached", "--quiet", "HEAD", "--", relative],
            ] {
                if !Command::new("git")
                    .args(arguments)
                    .current_dir(repository)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map_err(io_error)?
                    .success()
                {
                    return Err(invalid(
                        "tracked model-cache manifest must exactly match HEAD",
                    ));
                }
            }
            let path = repository.join(relative);
            let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() == 0
                || metadata.len() > 64 * 1024
            {
                return Err(invalid("tracked model-cache manifest is unsafe"));
            }
            let bytes = fs::read(path).map_err(io_error)?;
            let manifest: Value = serde_json::from_slice(&bytes)
                .map_err(|_| invalid("tracked model-cache manifest is invalid JSON"))?;
            let object = manifest
                .as_object()
                .ok_or_else(|| invalid("tracked model-cache manifest must be an object"))?;
            if object.len() != 2
                || object.get("schema").and_then(Value::as_str)
                    != Some("codeclew-model-cache-policy/2.0")
            {
                return Err(invalid("tracked model-cache manifest envelope is invalid"));
            }
            let compilations = object
                .get("compilations")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("tracked model-cache compilations are missing"))?;
            let values = compilations
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or_else(|| invalid("tracked model-cache compilation must be a string"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if values.is_empty()
                || values.len() > 64
                || !values.windows(2).all(|pair| pair[0] < pair[1])
                || selected_compilations
                    .iter()
                    .any(|compilation| !values.contains(&compilation.as_str()))
            {
                return Err(invalid(
                    "tracked model-cache manifest does not authorize all selected compilations",
                ));
            }
            let mut expected = canonical::bytes(&manifest).map_err(internal)?;
            expected.push(b'\n');
            if bytes != expected {
                return Err(invalid(
                    "tracked model-cache manifest must be canonical JSON plus newline",
                ));
            }
            Ok((Some(canonical::hash_bytes(&bytes)), None))
        }
        ModelCachePolicy::SealedExternal => {
            if runtime_mode != RuntimeMode::Release {
                return Err(invalid(
                    "sealed external build state requires a RELEASE runtime capsule",
                ));
            }
            let configured = external_build_state
                .ok_or_else(|| invalid("sealed external build-state path is required"))?;
            if !configured.is_absolute()
                || configured
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(invalid(
                    "sealed external build-state path must be normalized and absolute",
                ));
            }
            let root = configured.canonicalize().map_err(io_error)?;
            let metadata = fs::symlink_metadata(configured).map_err(io_error)?;
            if root != configured || metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(invalid("sealed external build-state root is unsafe"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if metadata.uid() != unsafe { libc::geteuid() }
                    || metadata.permissions().mode() & 0o077 != 0
                {
                    return Err(invalid(
                        "sealed external build-state root must be private and caller-owned",
                    ));
                }
            }
            if root.starts_with(repository) || repository.starts_with(&root) {
                return Err(invalid(
                    "sealed external build state must be outside the repository",
                ));
            }
            let manifest_path = root.join("CODECLEW_K1_BUILD_STATE_MANIFEST.json");
            let marker_path = root.join("CODECLEW_K1_BUILD_STATE_SEED");
            let manifest_metadata = fs::symlink_metadata(&manifest_path).map_err(io_error)?;
            let marker_metadata = fs::symlink_metadata(&marker_path).map_err(io_error)?;
            if manifest_metadata.file_type().is_symlink()
                || !manifest_metadata.is_file()
                || manifest_metadata.len() == 0
                || manifest_metadata.len() > 64 * 1024 * 1024
                || marker_metadata.file_type().is_symlink()
                || !marker_metadata.is_file()
                || marker_metadata.len() != 72
            {
                return Err(invalid("sealed external build-state authority is unsafe"));
            }
            let manifest = fs::read(manifest_path).map_err(io_error)?;
            let manifest_value: Value = serde_json::from_slice(&manifest)
                .map_err(|_| invalid("sealed external build-state manifest is invalid JSON"))?;
            if manifest_value.get("schema").and_then(Value::as_str)
                != Some("codeclew.kotlin-k1-build-state-manifest/0.1")
            {
                return Err(invalid(
                    "sealed external build-state manifest schema is invalid",
                ));
            }
            let mut canonical_manifest = canonical::bytes(&manifest_value).map_err(internal)?;
            canonical_manifest.push(b'\n');
            if manifest != canonical_manifest {
                return Err(invalid(
                    "sealed external build-state manifest must be canonical JSON plus newline",
                ));
            }
            let manifest_digest = canonical::hash_bytes(&manifest);
            let marker = fs::read(marker_path).map_err(io_error)?;
            if marker != format!("{manifest_digest}\n").as_bytes() {
                return Err(invalid(
                    "sealed external build-state marker does not bind its manifest",
                ));
            }
            let authority = canonical::hash(&json!({
                "schema":"codeclew-sealed-external-model-cache/2.0",
                "manifestDigest":manifest_digest,
                "markerDigest":canonical::hash_bytes(&marker),
            }))
            .map_err(internal)?;
            Ok((Some(authority), Some(root)))
        }
    }
}

fn canonical_compilations(
    language: SessionLanguage,
    compilations: &[String],
) -> Result<Vec<String>, ClewError> {
    if compilations.is_empty() || compilations.len() > 64 {
        return Err(invalid("session must select between 1 and 64 compilations"));
    }
    if compilations
        .iter()
        .any(|compilation| !valid_compilation(language, compilation))
    {
        return Err(invalid("session compilation authority is invalid"));
    }
    let mut canonical = compilations.to_vec();
    canonical.sort();
    canonical.dedup();
    if canonical.len() != compilations.len() {
        return Err(invalid("session compilation authority is duplicated"));
    }
    Ok(canonical)
}

fn model_cache_policy_is_valid(language: SessionLanguage, policy: ModelCachePolicy) -> bool {
    language == SessionLanguage::Kotlin || policy == ModelCachePolicy::NonCacheable
}

fn compilations_are_canonical(language: SessionLanguage, compilations: &[String]) -> bool {
    !compilations.is_empty()
        && compilations.len() <= 64
        && compilations
            .iter()
            .all(|compilation| valid_compilation(language, compilation))
        && compilations.windows(2).all(|pair| pair[0] < pair[1])
}

fn valid_compilation(language: SessionLanguage, compilation: &str) -> bool {
    match language {
        SessionLanguage::Rust => {
            return crate::rust_project_model::RustCompilationSelector::parse(compilation).is_ok();
        }
        SessionLanguage::Python => {
            return crate::python_project_model::PythonCompilationSelector::parse(compilation)
                .is_ok();
        }
        SessionLanguage::Kotlin => {}
    }
    if compilation.len() > 256 || !compilation.starts_with(':') {
        return false;
    }
    let Some((project, source_set)) = compilation.split_once('/') else {
        return false;
    };
    if source_set.contains('/') || !safe_compilation_segment(source_set) {
        return false;
    }
    let project = &project[1..];
    project.is_empty() || project.split(':').all(safe_compilation_segment)
}

fn safe_compilation_segment(segment: &str) -> bool {
    let mut bytes = segment.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn generation_jobs_are_valid(generation_jobs: Option<usize>) -> bool {
    !generation_jobs.is_some_and(|jobs| jobs == 0 || jobs > 64)
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

    #[test]
    fn context_stdout_limit_includes_the_trailing_newline() {
        let at_limit = canonical::bytes(&json!("x".repeat(MAX_CONTEXT_STDOUT_BYTES - 3))).unwrap();
        let over_limit =
            canonical::bytes(&json!("x".repeat(MAX_CONTEXT_STDOUT_BYTES - 2))).unwrap();
        assert_eq!(at_limit.len() + 1, MAX_CONTEXT_STDOUT_BYTES);
        assert_eq!(over_limit.len() + 1, MAX_CONTEXT_STDOUT_BYTES + 1);
        validate_context_stdout_bytes(&at_limit).unwrap();
        assert!(validate_context_stdout_bytes(&over_limit).is_err());
    }

    fn test_session() -> SessionAuthority {
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut authority = SessionAuthority {
            schema: SESSION_SCHEMA.into(),
            authority_digest: String::new(),
            session_id: format!("session:{digest}"),
            repository_key: format!("repo:{digest}"),
            base_revision: "1111111111111111111111111111111111111111".into(),
            target_ref: "refs/heads/main".into(),
            target_oid: "1111111111111111111111111111111111111111".into(),
            runtime_key: format!("runtime:{digest}"),
            runtime_mode: RuntimeMode::Development,
            language: SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: 1,
        };
        authority.authority_digest = session_authority_digest(&authority).unwrap();
        authority
    }

    fn initialized_session() -> (tempfile::TempDir, StateAuthority, PathBuf, SessionAuthority) {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let authority = test_session();
        let root = state.session_root(&authority.session_id).unwrap();
        for child in ["runs", "candidates"] {
            create_private_directory(&root.join(child)).unwrap();
        }
        initialize_session_lifecycle(&state, &root, &authority).unwrap();
        (temporary, state, root, authority)
    }

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
            prepared_authority_digest: None,
            final_commit: None,
            publication_blocked: false,
            conditional_approval: None,
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
    fn context_request_is_bounded_and_nfc_before_analysis() {
        validate_context_request(
            "Rename the function without changing its behavior",
            &["com.example.Café".into()],
        )
        .unwrap();
        assert!(validate_context_request("", &["symbol".into()]).is_err());
        assert!(validate_context_request("intent", &["com.example.Cafe\u{301}".into()]).is_err());
        assert!(validate_context_request("intent", &["x".repeat(4097)]).is_err());
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
        assert_eq!(
            serde_json::to_value(RunStatus::ReadyToPublishConditional).unwrap(),
            "READY_TO_PUBLISH_CONDITIONAL"
        );
        assert_eq!(
            serde_json::to_value(RunStatus::PublishedConditional).unwrap(),
            "PUBLISHED_CONDITIONAL"
        );
    }

    #[test]
    fn lifecycle_is_append_only_terminal_and_projection_recovers() {
        let (_temporary, state, root, authority) = initialized_session();

        let closed = transition_session_terminal_with_state(
            &authority,
            SessionStatus::Closed,
            &state,
            &root,
        )
        .unwrap();
        assert_eq!(closed.status, SessionStatus::Closed);
        assert_eq!(closed.sequence, 1);
        fs::write(root.join("lifecycle.json"), b"corrupt projection").unwrap();
        let recovered = load_session_lifecycle(&state, &root, &authority).unwrap();
        assert_eq!(recovered.event_hash, closed.event_hash);
        assert!(
            transition_session_terminal_with_state(
                &authority,
                SessionStatus::Aborted,
                &state,
                &root,
            )
            .is_err()
        );
    }

    #[test]
    fn close_refuses_created_and_recovery_required_runs() {
        let (_temporary, state, root, authority) = initialized_session();
        let mut run = test_run();
        let run_root = state.run_root(&run.run_id).unwrap();
        append_run_entry(&state, &run_root, &mut run, None).unwrap();
        ensure_session_run_reference(&state, &root, &authority, &run).unwrap();

        let error = transition_session_terminal_with_state(
            &authority,
            SessionStatus::Closed,
            &state,
            &root,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);

        run.status = RunStatus::Preparing;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        run.status = RunStatus::WorktreeRecoveryRequired;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        assert!(
            transition_session_terminal_with_state(
                &authority,
                SessionStatus::Closed,
                &state,
                &root,
            )
            .is_err()
        );

        run.status = RunStatus::Published;
        run.final_commit = Some("2222222222222222222222222222222222222222".into());
        save_run_transition(&state, &run_root, &mut run).unwrap();
        assert_eq!(
            transition_session_terminal_with_state(
                &authority,
                SessionStatus::Closed,
                &state,
                &root,
            )
            .unwrap()
            .status,
            SessionStatus::Closed
        );
    }

    #[test]
    fn abort_allows_cancelled_run_but_not_a_live_preparation() {
        let (_temporary, state, root, authority) = initialized_session();
        let mut run = test_run();
        let run_root = state.run_root(&run.run_id).unwrap();
        append_run_entry(&state, &run_root, &mut run, None).unwrap();
        ensure_session_run_reference(&state, &root, &authority, &run).unwrap();
        run.status = RunStatus::Preparing;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        assert!(
            transition_session_terminal_with_state(
                &authority,
                SessionStatus::Aborted,
                &state,
                &root,
            )
            .is_err()
        );
        run.status = RunStatus::Cancelled;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        assert_eq!(
            transition_session_terminal_with_state(
                &authority,
                SessionStatus::Aborted,
                &state,
                &root,
            )
            .unwrap()
            .status,
            SessionStatus::Aborted
        );
    }

    #[test]
    fn descriptor_authority_ignores_a_replaced_state_root_path() {
        let (_temporary, state, root, authority) = initialized_session();
        let mut run = test_run();
        let run_root = state.run_root(&run.run_id).unwrap();
        append_run_entry(&state, &run_root, &mut run, None).unwrap();
        ensure_session_run_reference(&state, &root, &authority, &run).unwrap();
        run.status = RunStatus::Preparing;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        run.status = RunStatus::Failed;
        save_run_transition(&state, &run_root, &mut run).unwrap();

        let original_state_root = state.root().to_path_buf();
        let pinned_state_root = original_state_root.with_file_name("pinned-v2");
        fs::rename(&original_state_root, &pinned_state_root).unwrap();
        let fake_session_root =
            original_state_root.join(root.strip_prefix(&original_state_root).unwrap());
        fs::create_dir_all(&fake_session_root).unwrap();
        fs::write(fake_session_root.join("lifecycle.jsonl"), b"forged\n").unwrap();
        fs::write(fake_session_root.join("lifecycle.json"), b"forged").unwrap();

        let closed = transition_session_terminal_with_state(
            &authority,
            SessionStatus::Closed,
            &state,
            &root,
        )
        .unwrap();
        assert_eq!(closed.status, SessionStatus::Closed);
        assert_eq!(
            fs::read(fake_session_root.join("lifecycle.jsonl")).unwrap(),
            b"forged\n"
        );

        let pinned_session_root =
            pinned_state_root.join(root.strip_prefix(&original_state_root).unwrap());
        let pinned_ledger = fs::read(pinned_session_root.join("lifecycle.jsonl")).unwrap();
        assert!(
            pinned_ledger
                .windows(b"CLOSED".len())
                .any(|bytes| bytes == b"CLOSED")
        );
        assert_eq!(
            load_run_projection(&state, &run_root, &run.run_id)
                .unwrap()
                .status,
            RunStatus::Failed
        );
    }

    #[test]
    fn close_cannot_cross_an_in_progress_resume_admission() {
        use std::sync::{Arc, Barrier, mpsc};
        use std::time::Duration;

        let (_temporary, state, root, authority) = initialized_session();
        let mut run = test_run();
        let run_root = state.run_root(&run.run_id).unwrap();
        append_run_entry(&state, &run_root, &mut run, None).unwrap();
        ensure_session_run_reference(&state, &root, &authority, &run).unwrap();
        run.status = RunStatus::Preparing;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        run.status = RunStatus::Failed;
        save_run_transition(&state, &run_root, &mut run).unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let (release_sender, release_receiver) = mpsc::channel();
        let (close_sender, close_receiver) = mpsc::channel();
        let resume_state = state.clone();
        let resume_root = root.clone();
        let resume_authority = authority.clone();
        let resume_barrier = barrier.clone();
        let resume = std::thread::spawn(move || {
            let _admission =
                open_session_admission_with_state(&resume_authority, &resume_state, &resume_root)
                    .unwrap();
            resume_barrier.wait();
            release_receiver.recv().unwrap();
            run.status = RunStatus::Created;
            save_run_transition(&resume_state, &run_root, &mut run).unwrap();
        });

        let close_state = state.clone();
        let close_root = root.clone();
        let close_authority = authority.clone();
        let close = std::thread::spawn(move || {
            barrier.wait();
            let result = transition_session_terminal_with_state(
                &close_authority,
                SessionStatus::Closed,
                &close_state,
                &close_root,
            );
            close_sender.send(result).unwrap();
        });

        assert!(matches!(
            close_receiver.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release_sender.send(()).unwrap();
        let error = close_receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);
        resume.join().unwrap();
        close.join().unwrap();
        assert_eq!(
            load_session_lifecycle(&state, &root, &authority)
                .unwrap()
                .status,
            SessionStatus::Open
        );
    }

    #[test]
    fn gc_refuses_a_replaced_state_root_before_following_any_locator() {
        let (_temporary, state, root, authority) = initialized_session();
        transition_session_terminal_with_state(&authority, SessionStatus::Closed, &state, &root)
            .unwrap();
        state
            .write_private_atomic(&root.join("preserve.marker"), b"owned state")
            .unwrap();

        let original_state_root = state.root().to_path_buf();
        let pinned_state_root = original_state_root.with_file_name("pinned-v2");
        fs::rename(&original_state_root, &pinned_state_root).unwrap();
        let fake_session_root =
            original_state_root.join(root.strip_prefix(&original_state_root).unwrap());
        fs::create_dir_all(&fake_session_root).unwrap();
        fs::write(fake_session_root.join("locator.json"), b"forged locator").unwrap();

        let error =
            garbage_collect_session_with_state(&authority, true, &state, &root).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(
            fs::read(fake_session_root.join("locator.json")).unwrap(),
            b"forged locator"
        );
        let pinned_session_root =
            pinned_state_root.join(root.strip_prefix(&original_state_root).unwrap());
        assert_eq!(
            fs::read(pinned_session_root.join("preserve.marker")).unwrap(),
            b"owned state"
        );
        assert_eq!(
            load_session_lifecycle(&state, &root, &authority)
                .unwrap()
                .status,
            SessionStatus::Closed
        );
    }

    #[test]
    fn lifecycle_tampering_fails_closed() {
        let (_temporary, state, root, authority) = initialized_session();
        let path = root.join("lifecycle.jsonl");
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = b'[';
        fs::write(path, bytes).unwrap();
        assert!(load_session_lifecycle(&state, &root, &authority).is_err());
    }

    #[test]
    fn gc_cleanliness_reports_ignored_and_untracked_outputs_without_classifying_them() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path();
        assert!(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(repository)
                .status()
                .unwrap()
                .success()
        );
        fs::write(repository.join(".gitignore"), b"build/\n").unwrap();
        fs::write(repository.join("tracked"), b"base\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "."])
                .current_dir(repository)
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
                    "base",
                ])
                .current_dir(repository)
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(repository.join("build")).unwrap();
        fs::write(repository.join("build/output.class"), b"derived").unwrap();
        let paths = candidate_untracked_after_clean_tracked_check(repository).unwrap();
        assert_eq!(paths, vec!["build/".to_string()]);
        fs::write(repository.join("notes.txt"), b"keep me").unwrap();
        let paths = candidate_untracked_after_clean_tracked_check(repository).unwrap();
        assert!(paths.iter().any(|path| path == "notes.txt"));
        fs::write(repository.join("tracked"), b"changed\n").unwrap();
        assert!(candidate_untracked_after_clean_tracked_check(repository).is_err());
    }

    #[test]
    fn gc_removes_only_managed_worktrees_and_leaves_legacy_state_inert() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir(&repository).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        fs::write(repository.join("tracked"), b"base\n").unwrap();
        fs::write(repository.join(".gitignore"), b"build/\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "tracked", ".gitignore"])
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
                    "base",
                ])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let mut authority = test_session();
        authority.repository_key = state.repository(&repository).unwrap().key;
        authority.base_revision = git_output(&repository, &["rev-parse", "HEAD"]).unwrap();
        authority.target_oid = authority.base_revision.clone();
        authority.authority_digest.clear();
        authority.authority_digest = session_authority_digest(&authority).unwrap();
        let root = state.session_root(&authority.session_id).unwrap();
        for child in ["runs", "candidates"] {
            create_private_directory(&root.join(child)).unwrap();
        }
        let source = root.join("source");
        create_filtered_detached_worktree(&repository, &source, &authority.base_revision).unwrap();
        seal_source_worktree(&source).unwrap();
        write_managed_json_create_new(
            &state,
            &root.join("locator.json"),
            &RepositoryLocator {
                schema: "codeclew-repository-locator/3.0".into(),
                target_repository_path: repository.clone(),
                source_repository_path: source.clone(),
                external_build_state_path: None,
            },
        )
        .unwrap();
        initialize_session_lifecycle(&state, &root, &authority).unwrap();
        let mut run = test_run();
        let run_root = state.run_root(&run.run_id).unwrap();
        append_run_entry(&state, &run_root, &mut run, None).unwrap();
        ensure_session_run_reference(&state, &root, &authority, &run).unwrap();
        let candidate = root
            .join("candidates")
            .join(id_component(&run.run_id, "run:").unwrap());
        create_private_directory(&candidate).unwrap();
        let candidate_worktree = candidate.join("worktree");
        create_filtered_detached_worktree(
            &repository,
            &candidate_worktree,
            &authority.base_revision,
        )
        .unwrap();
        fs::create_dir(candidate_worktree.join("build")).unwrap();
        let private_output = candidate_worktree.join("build/private-notes.txt");
        fs::write(&private_output, b"must survive").unwrap();
        run.status = RunStatus::Preparing;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        run.candidate_commit = Some(authority.base_revision.clone());
        run.status = RunStatus::ValidatedConditional;
        save_run_transition(&state, &run_root, &mut run).unwrap();
        state
            .write_private_atomic(&run_root.join("prepared-v2.json"), b"derived evidence")
            .unwrap();
        transition_session_terminal_with_state(&authority, SessionStatus::Closed, &state, &root)
            .unwrap();
        fs::create_dir(repository.join(".semantic-thread")).unwrap();
        fs::write(repository.join(".semantic-thread/private"), b"legacy").unwrap();

        let error =
            garbage_collect_session_with_state(&authority, false, &state, &root).unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);
        assert!(source.exists());
        assert!(candidate_worktree.exists());

        let forced_error =
            garbage_collect_session_with_state(&authority, true, &state, &root).unwrap_err();
        assert_eq!(forced_error.code, ErrorCode::PreconditionFailed);
        assert_eq!(fs::read(&private_output).unwrap(), b"must survive");
        assert!(source.exists());
        assert!(candidate_worktree.exists());

        fs::remove_file(&private_output).unwrap();
        fs::remove_dir(candidate_worktree.join("build")).unwrap();
        let unknown_candidate_state = candidate.join("private-note");
        fs::write(&unknown_candidate_state, b"must also survive").unwrap();
        let unknown_error =
            garbage_collect_session_with_state(&authority, true, &state, &root).unwrap_err();
        assert_eq!(unknown_error.code, ErrorCode::PreconditionFailed);
        assert_eq!(
            fs::read(&unknown_candidate_state).unwrap(),
            b"must also survive"
        );
        assert!(source.exists());
        assert!(candidate_worktree.exists());
        fs::remove_file(unknown_candidate_state).unwrap();
        let lifecycle =
            garbage_collect_session_with_state(&authority, false, &state, &root).unwrap();

        assert_eq!(lifecycle.status, SessionStatus::GarbageCollected);
        assert!(!source.exists());
        assert!(candidate.is_dir());
        assert!(fs::read_dir(&candidate).unwrap().next().is_none());
        assert!(
            !state
                .private_file_exists(&run_root.join("prepared-v2.json"))
                .unwrap()
        );
        assert!(
            state
                .private_file_exists(&run_root.join("ledger.jsonl"))
                .unwrap()
        );
        assert_eq!(
            fs::read(repository.join(".semantic-thread/private")).unwrap(),
            b"legacy"
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
    fn candidate_generation_bindings_have_one_closed_file_shape() {
        let digest = "a".repeat(64);
        assert!(candidate_derived_state_name(std::ffi::OsStr::new(
            "staged-generation.json"
        )));
        assert!(candidate_derived_state_name(std::ffi::OsStr::new(
            &format!("staged-generation-{digest}.json")
        )));
        assert!(!candidate_derived_state_name(std::ffi::OsStr::new(
            "staged-compilations"
        )));
        assert!(!candidate_derived_state_name(std::ffi::OsStr::new(
            "staged-generation-ABC.json"
        )));
    }

    #[test]
    fn session_compilations_are_nonempty_bounded_and_canonical() {
        let kotlin = SessionLanguage::Kotlin;
        assert!(canonical_compilations(kotlin, &[]).is_err());
        assert!(canonical_compilations(kotlin, &[String::new()]).is_err());
        assert!(canonical_compilations(kotlin, &["x".repeat(257)]).is_err());
        for invalid in [
            "main",
            "--offline",
            ":main",
            ":/",
            ":../main",
            ":app/../main",
            ":app//main",
            ":app:/main",
            ":bad compilation/main",
            ":-option/main",
            ":app/-option",
            ":app/main:other",
        ] {
            assert!(
                canonical_compilations(kotlin, &[invalid.into()]).is_err(),
                "accepted invalid compilation {invalid}"
            );
        }
        assert!(canonical_compilations(kotlin, &vec![":/main".into(); 65]).is_err());
        assert!(canonical_compilations(kotlin, &[":/main".into(), ":/main".into()]).is_err());

        let compilations = canonical_compilations(
            kotlin,
            &[
                ":z/main".into(),
                ":/main".into(),
                ":app:core/integrationTest".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            compilations,
            [":/main", ":app:core/integrationTest", ":z/main"]
        );
        assert!(compilations_are_canonical(kotlin, &compilations));
        assert!(!compilations_are_canonical(
            kotlin,
            &[":z/main".into(), ":/main".into()]
        ));
        let rust = SessionLanguage::Rust;
        let selector = "cargo:crates/clew/Cargo.toml#clew#lib#clew".to_owned();
        assert_eq!(
            canonical_compilations(rust, std::slice::from_ref(&selector)).unwrap(),
            [selector]
        );
        assert!(canonical_compilations(rust, &[":/main".into()]).is_err());

        let python = SessionLanguage::Python;
        let selector = "python:.#backend".to_owned();
        assert_eq!(
            canonical_compilations(python, std::slice::from_ref(&selector)).unwrap(),
            [selector]
        );
        assert!(canonical_compilations(python, &[":/main".into()]).is_err());
    }

    #[test]
    fn session_generation_jobs_are_optional_and_bounded() {
        assert!(generation_jobs_are_valid(None));
        assert!(generation_jobs_are_valid(Some(1)));
        assert!(generation_jobs_are_valid(Some(64)));
        assert!(!generation_jobs_are_valid(Some(0)));
        assert!(!generation_jobs_are_valid(Some(65)));
    }

    #[test]
    fn read_only_languages_accept_only_non_cacheable_model_authority() {
        for language in [SessionLanguage::Python, SessionLanguage::Rust] {
            assert!(model_cache_policy_is_valid(
                language,
                ModelCachePolicy::NonCacheable
            ));
            assert!(!model_cache_policy_is_valid(
                language,
                ModelCachePolicy::TrackedManifest
            ));
            assert!(!model_cache_policy_is_valid(
                language,
                ModelCachePolicy::SealedExternal
            ));
        }
        assert!(model_cache_policy_is_valid(
            SessionLanguage::Kotlin,
            ModelCachePolicy::TrackedManifest
        ));
    }

    #[test]
    fn tracked_model_cache_requires_a_canonical_head_bound_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir(&repository).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        let manifest = json!({
            "schema":"codeclew-model-cache-policy/2.0",
            "compilations":[":/main",":/test"],
        });
        let mut bytes = canonical::bytes(&manifest).unwrap();
        bytes.push(b'\n');
        fs::write(repository.join("codeclew.model-cache.json"), &bytes).unwrap();
        assert!(
            Command::new("git")
                .args(["add", "codeclew.model-cache.json"])
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
                    "model cache authority",
                ])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );

        let (authority, external) = model_cache_authority(
            &repository,
            &[":/main".into(), ":/test".into()],
            ModelCachePolicy::TrackedManifest,
            RuntimeMode::Development,
            None,
        )
        .unwrap();
        assert_eq!(
            authority.as_deref(),
            Some(canonical::hash_bytes(&bytes).as_str())
        );
        assert_eq!(external, None);

        assert!(
            model_cache_authority(
                &repository,
                &[":/main".into(), ":missing/main".into()],
                ModelCachePolicy::TrackedManifest,
                RuntimeMode::Development,
                None,
            )
            .is_err()
        );

        fs::write(
            repository.join("codeclew.model-cache.json"),
            [bytes, b" \n".to_vec()].concat(),
        )
        .unwrap();
        assert!(
            model_cache_authority(
                &repository,
                &[":/main".into()],
                ModelCachePolicy::TrackedManifest,
                RuntimeMode::Development,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn model_cache_modes_reject_mixed_or_non_release_authority() {
        let repository = tempfile::tempdir().unwrap();
        assert!(
            model_cache_authority(
                repository.path(),
                &[":/main".into()],
                ModelCachePolicy::NonCacheable,
                RuntimeMode::Release,
                Some(repository.path()),
            )
            .is_err()
        );
        assert!(
            model_cache_authority(
                repository.path(),
                &[":/main".into()],
                ModelCachePolicy::SealedExternal,
                RuntimeMode::Development,
                Some(repository.path()),
            )
            .is_err()
        );
    }

    #[test]
    fn run_projection_is_recovered_from_the_authoritative_ledger() {
        let (_temporary, state, root, mut run) = initialized_run();
        run.status = RunStatus::Preparing;
        save_run_transition(&state, &root, &mut run).unwrap();
        state
            .write_private_atomic(&root.join("record.json"), b"corrupt projection")
            .unwrap();

        let recovered = load_run_projection(&state, &root, &run.run_id).unwrap();

        assert_eq!(recovered.status, RunStatus::Preparing);
        assert_eq!(recovered.sequence, 1);
        assert_eq!(recovered.ledger_head, run.ledger_head);
        let projection: RunRecord =
            read_managed_json(&state, &root.join("record.json"), MAX_PLAN_BYTES).unwrap();
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
    fn unpublished_ready_run_can_be_cancelled() {
        for ready in [
            RunStatus::ReadyToPublish,
            RunStatus::ReadyToPublishConditional,
        ] {
            let (_temporary, state, root, mut run) = initialized_run();
            run.status = RunStatus::Preparing;
            save_run_transition(&state, &root, &mut run).unwrap();
            run.status = ready;
            save_run_transition(&state, &root, &mut run).unwrap();
            run.status = RunStatus::Cancelled;
            save_run_transition(&state, &root, &mut run).unwrap();
            assert_eq!(
                load_run_projection(&state, &root, &run.run_id)
                    .unwrap()
                    .status,
                RunStatus::Cancelled
            );
        }
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

    #[test]
    fn thread_coverage_ids_cannot_enter_mutation_identity_namespaces() {
        let coverage = "thread-coverage:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(
            reject_thread_coverage_for_mutation(coverage)
                .unwrap_err()
                .code,
            ErrorCode::PreconditionFailed
        );
        assert!(reject_thread_context_for_mutation(coverage).is_err());
        assert!(RunRecord::load(coverage).is_err());
    }

    #[test]
    fn freshness_classification_is_actionable_and_path_free() {
        let stale = classify_freshness(
            "session:fixture",
            SessionStatus::Open,
            Some(true),
            Some(false),
            Some(true),
        );
        assert_eq!(stale.status, "STALE");
        assert_eq!(stale.remediation_id, "OPEN_NEW_SESSION");

        let dirty = classify_freshness(
            "session:fixture",
            SessionStatus::Open,
            Some(true),
            Some(true),
            Some(false),
        );
        assert_eq!(dirty.status, "DIRTY");
        assert_eq!(dirty.remediation_id, "CLEAN_TARGET_WORKTREE");

        let unavailable = classify_freshness(
            "session:fixture",
            SessionStatus::Open,
            None,
            Some(true),
            Some(true),
        );
        assert_eq!(unavailable.status, "UNAVAILABLE");
        assert_eq!(unavailable.remediation_id, "CHECK_REPOSITORY_LOCATOR");
    }
}
