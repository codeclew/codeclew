use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use crate::runtime::{RuntimeAuthority, RuntimeMode};
use crate::state::{StateAuthority, create_private_directory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub const SESSION_SCHEMA: &str = "codeclew-session/1.0";
pub const CONTEXT_SCHEMA: &str = "codeclew-context/1.0";
pub const PLAN_SCHEMA: &str = "codeclew-plan/1.0";
pub const RUN_SCHEMA: &str = "codeclew-task-run/2.0";
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
    pub parent_context_id: Option<String>,
    pub intent: String,
    pub terms: Vec<String>,
    pub evidence_digest: String,
    pub projection: Value,
    pub evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanObject {
    pub schema: String,
    pub plan_id: String,
    pub session_id: String,
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
    repository_path: PathBuf,
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
        let authority = Self {
            schema: SESSION_SCHEMA.into(),
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
        let root = state.session_root(&authority.session_id)?;
        for child in ["objects/sha256", "contexts", "plans", "candidates"] {
            create_private_directory(&root.join(child))?;
        }
        write_json_create_new(&root.join("authority.json"), &authority)?;
        write_json_create_new(
            &root.join("locator.json"),
            &RepositoryLocator {
                schema: "codeclew-repository-locator/1.0".into(),
                repository_path: repo,
            },
        )?;
        Ok(authority)
    }

    pub fn load(session_id: &str) -> Result<(Self, PathBuf), ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.session_root(session_id)?;
        let authority: Self = read_json_limited(&root.join("authority.json"), MAX_PLAN_BYTES)?;
        if authority.schema != SESSION_SCHEMA || authority.session_id != session_id {
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
        let path = locator.repository_path.canonicalize().map_err(io_error)?;
        if state.repository(&path)?.key != self.repository_key {
            return Err(invalid(
                "repository locator no longer matches session authority",
            ));
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
        let evidence_digest = canonical::hash(&evidence).map_err(internal)?;
        let binding = json!({
            "schema": CONTEXT_SCHEMA,
            "sessionId": self.session_id,
            "parentContextId": parent_context_id,
            "intent": intent,
            "terms": terms,
            "evidenceDigest": evidence_digest,
        });
        let context_id = format!("context:{}", canonical::hash(&binding).map_err(internal)?);
        let object = ContextObject {
            schema: CONTEXT_SCHEMA.into(),
            context_id,
            session_id: self.session_id.clone(),
            parent_context_id,
            intent,
            terms,
            evidence_digest,
            projection,
            evidence,
        };
        let state = StateAuthority::process_default()?;
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
        let object: ContextObject = read_json_limited(
            &root.join("contexts").join(id_filename(context_id)?),
            64 * 1024 * 1024,
        )?;
        if object.schema != CONTEXT_SCHEMA
            || object.context_id != context_id
            || object.session_id != self.session_id
            || canonical::hash(&object.evidence).map_err(internal)? != object.evidence_digest
        {
            return Err(invalid("context authority is invalid"));
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
        Ok(Self {
            schema: RUN_SCHEMA.into(),
            transaction_id: format!("tx:{}", Uuid::new_v4()),
            run_id,
            session_id: session.session_id.clone(),
            context_id: context_id.into(),
            plan_id: plan_id.into(),
            request_digest,
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
        let record: Self = read_json_limited(&root.join("record.json"), MAX_PLAN_BYTES)?;
        if record.schema != RUN_SCHEMA || record.run_id != run_id {
            return Err(invalid("run record identity is invalid"));
        }
        Ok(record)
    }

    pub fn save(&mut self) -> Result<(), ClewError> {
        self.updated_unix_ms = unix_ms();
        let state = StateAuthority::process_default()?;
        let root = state.run_root(&self.run_id)?;
        state.write_private_atomic(
            &root.join("record.json"),
            &canonical::bytes(self).map_err(internal)?,
        )
    }

    pub fn create_once(&self) -> Result<bool, ClewError> {
        let state = StateAuthority::process_default()?;
        let root = state.run_root(&self.run_id)?;
        let path = root.join("record.json");
        if path.exists() {
            return Ok(false);
        }
        write_json_create_new(&path, self)?;
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
    }
}
