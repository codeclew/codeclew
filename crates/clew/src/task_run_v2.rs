use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot::capture;
use crate::session::{ContextObject, PlanObject, SessionAuthority};
use crate::state::{StateAuthority, create_private_directory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const PLAN_V2_SCHEMA: &str = "codeclew-task-plan/2.0";
pub const PREPARED_V2_SCHEMA: &str = "codeclew-prepared-candidate/2.0";
const MAX_EDIT_BYTES: usize = 8 * 1024 * 1024;
const MAX_VALIDATION_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskPlanV2 {
    pub schema: String,
    pub operations: Vec<FileOperation>,
    pub validation: Vec<ValidationStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FileOperation {
    ReplaceText {
        op_id: String,
        target: ExistingTarget,
        old_text: String,
        new_text: String,
    },
    CreateFile {
        op_id: String,
        target: NewTarget,
        text: String,
    },
    DeleteFile {
        op_id: String,
        target: ExistingTarget,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExistingTarget {
    pub file_id: String,
    pub content_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewTarget {
    pub file_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationStep {
    pub launcher: ValidationLauncher,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationLauncher {
    Gradle,
    Maven,
    Cargo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationEvidence {
    pub launcher: ValidationLauncher,
    pub args_digest: String,
    pub output_digest: String,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedCandidateV2 {
    pub schema: String,
    pub session_id: String,
    pub context_id: String,
    pub plan_id: String,
    pub base_revision: String,
    pub target_ref: String,
    pub target_oid: String,
    pub candidate_commit: String,
    pub candidate_snapshot: CasObject,
    pub changed_files: Vec<String>,
    pub validation_evidence: Vec<ValidationEvidence>,
    pub publication_blocked: bool,
}

pub fn validate_plan_value(value: &Value) -> Result<TaskPlanV2, ClewError> {
    let plan: TaskPlanV2 = serde_json::from_value(value.clone()).map_err(parse_error)?;
    if plan.schema != PLAN_V2_SCHEMA || plan.operations.is_empty() || plan.operations.len() > 256 {
        return Err(invalid("task plan v2 operation set is invalid"));
    }
    if plan.validation.is_empty() || plan.validation.len() > 32 {
        return Err(invalid("task plan v2 validation set is invalid"));
    }
    let mut operation_ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut edit_bytes = 0usize;
    for operation in &plan.operations {
        let (op_id, file) = match operation {
            FileOperation::ReplaceText {
                op_id,
                target,
                old_text,
                new_text,
            } => {
                if old_text.is_empty() {
                    return Err(invalid("REPLACE_TEXT oldText cannot be empty"));
                }
                edit_bytes = edit_bytes
                    .checked_add(old_text.len())
                    .and_then(|size| size.checked_add(new_text.len()))
                    .ok_or_else(|| resource("task plan edit size overflow"))?;
                validate_cas(&target.content_ref)?;
                (op_id, &target.file_id)
            }
            FileOperation::CreateFile {
                op_id,
                target,
                text,
            } => {
                edit_bytes = edit_bytes
                    .checked_add(text.len())
                    .ok_or_else(|| resource("task plan edit size overflow"))?;
                (op_id, &target.file_id)
            }
            FileOperation::DeleteFile { op_id, target } => {
                validate_cas(&target.content_ref)?;
                (op_id, &target.file_id)
            }
        };
        if !safe_id(op_id)
            || !operation_ids.insert(op_id)
            || !safe_path(file)
            || !files.insert(file)
        {
            return Err(invalid(
                "task plan operation id or single-write file set is invalid",
            ));
        }
    }
    if files.len() > 256 || edit_bytes > MAX_EDIT_BYTES {
        return Err(resource("task plan exceeds file or edit byte limits"));
    }
    for step in &plan.validation {
        if step.args.len() > 128
            || step.args.iter().any(|argument| {
                argument.is_empty()
                    || argument.len() > 4096
                    || argument.contains('\0')
                    || argument.starts_with('/')
                    || argument.split('/').any(|component| component == "..")
                    || matches!(
                        argument.as_str(),
                        "--gradle-user-home" | "--project-cache-dir" | "-Dmaven.repo.local"
                    )
            })
        {
            return Err(invalid("task plan validation arguments are unsafe"));
        }
    }
    Ok(plan)
}

pub fn prepare(
    session: &SessionAuthority,
    context: &ContextObject,
    plan_object: &PlanObject,
    candidate_root: &Path,
) -> Result<PreparedCandidateV2, ClewError> {
    let plan = validate_plan_value(&plan_object.plan)?;
    if context.session_id != session.session_id
        || plan_object.context_id != context.context_id
        || plan_object.base_revision != session.base_revision
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "candidate inputs do not share one session authority",
        ));
    }
    let status = context
        .evidence
        .pointer("/context/completeness/status")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("context has no completeness status"))?;
    if !matches!(status, "COMPLETE_TASK" | "CONDITIONAL_TASK") {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "context cannot authorize candidate preparation",
        ));
    }
    let publication_blocked = status != "COMPLETE_TASK"
        || context
            .evidence
            .pointer("/context/verificationObligations")
            .and_then(Value::as_array)
            .is_some_and(|obligations| !obligations.is_empty());
    let repo = session.repository_path()?;
    if git(&repo, &["rev-parse", "HEAD"])? != session.base_revision
        || git(&repo, &["rev-parse", &session.target_ref])? != session.target_oid
    {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "session repository authority moved before preparation",
        ));
    }
    let worktree = candidate_root.join("worktree");
    if worktree.exists() {
        return Err(ClewError::new(
            ErrorCode::TransactionRecoveryRequired,
            "candidate worktree already exists; resume the bound run",
        ));
    }
    create_private_directory(candidate_root)?;
    git_status(
        Command::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "worktree",
                "add",
                "--detach",
                worktree
                    .to_str()
                    .ok_or_else(|| invalid("candidate path is not UTF-8"))?,
                &session.base_revision,
            ])
            .current_dir(&repo),
        "candidate worktree creation failed",
    )?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let authorized = authorized_sources(context)?;
    let changed_files = apply_operations(&store, &worktree, &plan.operations, &authorized)?;
    let validation_evidence = run_validation(&worktree, &plan.validation)?;
    let observed = changed_paths(&worktree)?;
    if observed != changed_files.iter().cloned().collect() {
        return Err(ClewError::new(
            ErrorCode::RwConflict,
            "candidate changed paths differ from the immutable plan write set",
        ));
    }
    git_add_paths(&worktree, &changed_files)?;
    let staged = staged_paths(&worktree)?;
    if staged != changed_files.iter().cloned().collect() {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "staged candidate paths differ from the immutable plan write set",
        ));
    }
    commit_candidate(&worktree, &plan_object.plan_id)?;
    let candidate_commit = git(&worktree, &["rev-parse", "HEAD"])?;
    let (_, candidate_snapshot) = capture(&worktree, &store)?;
    Ok(PreparedCandidateV2 {
        schema: PREPARED_V2_SCHEMA.into(),
        session_id: session.session_id.clone(),
        context_id: context.context_id.clone(),
        plan_id: plan_object.plan_id.clone(),
        base_revision: session.base_revision.clone(),
        target_ref: session.target_ref.clone(),
        target_oid: session.target_oid.clone(),
        candidate_commit,
        candidate_snapshot,
        changed_files,
        validation_evidence,
        publication_blocked,
    })
}

pub fn publish(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    candidate_root: &Path,
) -> Result<Value, ClewError> {
    validate_prepared(session, prepared)?;
    if prepared.publication_blocked {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "conditional candidate cannot be published",
        ));
    }
    let repo = session.repository_path()?;
    let worktree = candidate_root.join("worktree");
    verify_candidate_snapshot(&worktree, &prepared.candidate_snapshot)?;
    let current = git(&repo, &["rev-parse", &session.target_ref])?;
    if current == prepared.candidate_commit {
        synchronize_checked_out_target(&repo, prepared)?;
        return Ok(json!({
            "schema":"codeclew-publish-result/2.0",
            "status":"PUBLISHED",
            "candidateCommit":prepared.candidate_commit,
            "recovered":true,
        }));
    }
    if current != prepared.target_oid {
        return Err(ClewError::new(
            ErrorCode::RefCompareAndSwapFailed,
            "target ref moved after session open",
        ));
    }
    let checked_out = git_optional(&repo, &["symbolic-ref", "-q", "HEAD"])?.as_deref()
        == Some(session.target_ref.as_str());
    require_clean(&repo)?;
    if checked_out {
        git_status(
            Command::new("git")
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgSign=false",
                    "merge",
                    "--ff-only",
                    "--no-edit",
                    &prepared.candidate_commit,
                ])
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_MERGE_AUTOEDIT", "no")
                .current_dir(&repo),
            "checked-out target fast-forward failed",
        )?;
    } else {
        git_status(
            Command::new("git")
                .args([
                    "update-ref",
                    &session.target_ref,
                    &prepared.candidate_commit,
                    &prepared.target_oid,
                ])
                .current_dir(&repo),
            "target compare-and-swap failed",
        )?;
    }
    if git(&repo, &["rev-parse", &session.target_ref])? != prepared.candidate_commit {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "target publication result is inconsistent",
        ));
    }
    Ok(json!({
        "schema":"codeclew-publish-result/2.0",
        "status":"PUBLISHED",
        "candidateCommit":prepared.candidate_commit,
        "recovered":false,
    }))
}

pub fn recover(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    candidate_root: &Path,
) -> Result<Value, ClewError> {
    publish(session, prepared, candidate_root)
}

pub fn verify_candidate_snapshot(path: &Path, expected: &CasObject) -> Result<(), ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let (_, observed) = capture(path, &store)?;
    if &observed != expected {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "candidate changed after validation",
        ));
    }
    Ok(())
}

fn validate_prepared(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
) -> Result<(), ClewError> {
    if prepared.schema != PREPARED_V2_SCHEMA
        || prepared.session_id != session.session_id
        || prepared.base_revision != session.base_revision
        || prepared.target_ref != session.target_ref
        || prepared.target_oid != session.target_oid
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "prepared candidate authority differs from the session",
        ));
    }
    Ok(())
}

fn authorized_sources(context: &ContextObject) -> Result<BTreeMap<String, CasObject>, ClewError> {
    let mut sources = BTreeMap::new();
    for source in context
        .evidence
        .pointer("/context/sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = source
            .get("fileId")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("context source has no file identity"))?;
        let reference: CasObject = serde_json::from_value(
            source
                .get("contentRef")
                .cloned()
                .ok_or_else(|| invalid("context source has no content authority"))?,
        )
        .map_err(parse_error)?;
        if !safe_path(path) || sources.insert(path.into(), reference).is_some() {
            return Err(invalid("context source authority is invalid"));
        }
    }
    Ok(sources)
}

fn apply_operations(
    store: &CasStore,
    root: &Path,
    operations: &[FileOperation],
    authorized: &BTreeMap<String, CasObject>,
) -> Result<Vec<String>, ClewError> {
    let mut changed = Vec::new();
    for operation in operations {
        match operation {
            FileOperation::ReplaceText {
                target,
                old_text,
                new_text,
                ..
            } => {
                require_authorized(store, root, target, authorized)?;
                let path = safe_target(root, &target.file_id, false)?;
                let bytes = fs::read(&path).map_err(io_error)?;
                let source = String::from_utf8(bytes)
                    .map_err(|_| invalid("REPLACE_TEXT target is not UTF-8"))?;
                let occurrences = source.match_indices(old_text).count();
                if occurrences != 1 {
                    return Err(ClewError::new(
                        ErrorCode::AmbiguousTarget,
                        "REPLACE_TEXT oldText must occur exactly once",
                    ));
                }
                write_atomic(&path, source.replacen(old_text, new_text, 1).as_bytes())?;
                changed.push(target.file_id.clone());
            }
            FileOperation::CreateFile { target, text, .. } => {
                let path = safe_target(root, &target.file_id, true)?;
                if path.exists() {
                    return Err(ClewError::new(
                        ErrorCode::WwConflict,
                        "CREATE_FILE target already exists",
                    ));
                }
                write_atomic(&path, text.as_bytes())?;
                changed.push(target.file_id.clone());
            }
            FileOperation::DeleteFile { target, .. } => {
                require_authorized(store, root, target, authorized)?;
                let path = safe_target(root, &target.file_id, false)?;
                fs::remove_file(path).map_err(io_error)?;
                changed.push(target.file_id.clone());
            }
        }
    }
    changed.sort();
    Ok(changed)
}

fn require_authorized(
    store: &CasStore,
    root: &Path,
    target: &ExistingTarget,
    authorized: &BTreeMap<String, CasObject>,
) -> Result<(), ClewError> {
    if authorized.get(&target.file_id) != Some(&target.content_ref) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "plan target is absent from the bounded context",
        ));
    }
    let path = safe_target(root, &target.file_id, false)?;
    let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid("plan target is not a regular file"));
    }
    let expected = store.read(
        &target.content_ref,
        usize::try_from(target.content_ref.size)
            .map_err(|_| resource("target content exceeds host size"))?,
    )?;
    let actual = fs::read(path).map_err(io_error)?;
    if actual != expected.bytes() {
        return Err(ClewError::new(
            ErrorCode::StaleTarget,
            "plan target bytes differ from context authority",
        ));
    }
    Ok(())
}

fn run_validation(
    root: &Path,
    steps: &[ValidationStep],
) -> Result<Vec<ValidationEvidence>, ClewError> {
    let mut evidence = Vec::new();
    for step in steps {
        let executable = match step.launcher {
            ValidationLauncher::Gradle if root.join("gradlew").is_file() => "./gradlew",
            ValidationLauncher::Gradle => "gradle",
            ValidationLauncher::Maven if root.join("mvnw").is_file() => "./mvnw",
            ValidationLauncher::Maven => "mvn",
            ValidationLauncher::Cargo => "cargo",
        };
        let output = Command::new(executable)
            .args(&step.args)
            .current_dir(root)
            .env_remove("CODECLEW_HOME")
            .env_remove("CODECLEW_RUNTIME_ROOT")
            .env_remove("CODECLEW_RUNTIME_KEY")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .map_err(io_error)?;
        let total = output.stdout.len().saturating_add(output.stderr.len());
        if total > MAX_VALIDATION_OUTPUT_BYTES {
            return Err(resource("validation output exceeds 4 MiB"));
        }
        let mut hasher = Sha256::new();
        hasher.update(&output.stdout);
        hasher.update([0]);
        hasher.update(&output.stderr);
        let result = ValidationEvidence {
            launcher: step.launcher,
            args_digest: canonical::hash(&step.args).map_err(internal)?,
            output_digest: format!("sha256:{}", hex::encode(hasher.finalize())),
            success: output.status.success(),
        };
        evidence.push(result);
        if !output.status.success() {
            return Err(ClewError::new(
                match step.launcher {
                    ValidationLauncher::Cargo
                    | ValidationLauncher::Gradle
                    | ValidationLauncher::Maven => ErrorCode::TestFailed,
                },
                "candidate validation failed; inspect private run evidence",
            ));
        }
    }
    Ok(evidence)
}

fn changed_paths(root: &Path) -> Result<BTreeSet<String>, ClewError> {
    nul_paths(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
}

fn staged_paths(root: &Path) -> Result<BTreeSet<String>, ClewError> {
    nul_paths(root, &["diff", "--cached", "--name-only", "-z"])
}

fn nul_paths(root: &Path, args: &[&str]) -> Result<BTreeSet<String>, ClewError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("Git changed-path query failed"));
    }
    let mut paths = BTreeSet::new();
    for row in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let row = std::str::from_utf8(row).map_err(|_| invalid("Git path is not UTF-8"))?;
        let path = if args.first() == Some(&"status") {
            row.get(3..)
                .ok_or_else(|| invalid("Git status row is invalid"))?
        } else {
            row
        };
        if !safe_path(path) {
            return Err(invalid("Git emitted an unsafe changed path"));
        }
        paths.insert(path.into());
    }
    Ok(paths)
}

fn git_add_paths(root: &Path, paths: &[String]) -> Result<(), ClewError> {
    let mut command = Command::new("git");
    command.arg("add").arg("--");
    command.args(paths);
    git_status(command.current_dir(root), "candidate staging failed")
}

fn commit_candidate(root: &Path, plan_id: &str) -> Result<(), ClewError> {
    git_status(
        Command::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
                "commit",
                "-m",
                &format!(
                    "Codeclew candidate {}",
                    &plan_id[plan_id.len().saturating_sub(16)..]
                ),
            ])
            .env("GIT_AUTHOR_NAME", "Codeclew")
            .env("GIT_AUTHOR_EMAIL", "noreply@example.invalid")
            .env("GIT_COMMITTER_NAME", "Codeclew")
            .env("GIT_COMMITTER_EMAIL", "noreply@example.invalid")
            .env("GIT_TERMINAL_PROMPT", "0")
            .current_dir(root),
        "candidate commit failed",
    )
}

fn synchronize_checked_out_target(
    repo: &Path,
    prepared: &PreparedCandidateV2,
) -> Result<(), ClewError> {
    if git_optional(repo, &["symbolic-ref", "-q", "HEAD"])?.as_deref()
        != Some(prepared.target_ref.as_str())
    {
        return Ok(());
    }
    if git(repo, &["rev-parse", "HEAD"])? == prepared.candidate_commit {
        return Ok(());
    }
    require_clean(repo)?;
    git_status(
        Command::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "merge",
                "--ff-only",
                "--no-edit",
                &prepared.candidate_commit,
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .current_dir(repo),
        "published target worktree requires recovery",
    )
    .map_err(|error| ClewError {
        code: ErrorCode::WorktreeRecoveryRequired,
        ..error
    })
}

fn require_clean(repo: &Path) -> Result<(), ClewError> {
    if !changed_paths(repo)?.is_empty() {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "checked-out target worktree is dirty",
        ));
    }
    Ok(())
}

fn safe_target(root: &Path, relative: &str, create_parents: bool) -> Result<PathBuf, ClewError> {
    if !safe_path(relative) {
        return Err(invalid("task target path is unsafe"));
    }
    let canonical_root = root.canonicalize().map_err(io_error)?;
    let path = root.join(relative);
    let parent = path
        .parent()
        .ok_or_else(|| invalid("task target has no parent"))?;
    if create_parents {
        let mut cursor = root.to_path_buf();
        for component in Path::new(relative).components() {
            let Component::Normal(component) = component else {
                return Err(invalid("task target component is unsafe"));
            };
            if component == OsStr::new(Path::new(relative).file_name().unwrap_or_default()) {
                break;
            }
            cursor.push(component);
            if cursor.exists() {
                let metadata = fs::symlink_metadata(&cursor).map_err(io_error)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(invalid("task target parent is unsafe"));
                }
            } else {
                fs::create_dir(&cursor).map_err(io_error)?;
            }
        }
    }
    let canonical_parent = parent.canonicalize().map_err(io_error)?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(invalid("task target escapes candidate root"));
    }
    if path.exists()
        && fs::symlink_metadata(&path)
            .map_err(io_error)?
            .file_type()
            .is_symlink()
    {
        return Err(invalid("task target is a symlink"));
    }
    Ok(path)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ClewError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("edit path has no parent"))?;
    let temporary = parent.join(format!(".codeclew-edit-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::rename(&temporary, path).map_err(io_error)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o644)).map_err(io_error)?;
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<String, ClewError> {
    git_optional(repo, args)?.ok_or_else(|| invalid("Git authority is unavailable"))
}

fn git_optional(repo: &Path, args: &[&str]) -> Result<Option<String>, ClewError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .map(|value| Some(value.trim().into()))
        .map_err(|_| invalid("Git output is not UTF-8"))
}

fn git_status(command: &mut Command, message: &str) -> Result<(), ClewError> {
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(io_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(ClewError::new(ErrorCode::Internal, message))
    }
}

fn validate_cas(reference: &CasObject) -> Result<(), ClewError> {
    if reference.schema != crate::cas::CAS_OBJECT_SCHEMA
        || !digest(&reference.digest)
        || reference.object_schema.is_empty()
    {
        return Err(invalid("plan content reference is invalid"));
    }
    Ok(())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('/')
        && !value.contains('\0')
        && !value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
}

fn digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn parse_error(error: impl std::fmt::Display) -> ClewError {
    invalid(&error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::CAS_OBJECT_SCHEMA;

    fn reference() -> CasObject {
        CasObject {
            schema: CAS_OBJECT_SCHEMA.into(),
            object_schema: "test/source/1".into(),
            digest: format!("sha256:{}", "a".repeat(64)),
            size: 4,
        }
    }

    #[test]
    fn closed_plan_rejects_duplicate_files_and_unsafe_validation_state() {
        let duplicate = json!({
            "schema":PLAN_V2_SCHEMA,
            "operations":[
                {"kind":"DELETE_FILE","opId":"one","target":{"fileId":"A.kt","contentRef":reference()}},
                {"kind":"DELETE_FILE","opId":"two","target":{"fileId":"A.kt","contentRef":reference()}},
            ],
            "validation":[{"launcher":"GRADLE","args":["test"]}],
        });
        assert_eq!(
            validate_plan_value(&duplicate).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        let unsafe_plan = json!({
            "schema":PLAN_V2_SCHEMA,
            "operations":[{"kind":"DELETE_FILE","opId":"one","target":{"fileId":"A.kt","contentRef":reference()}}],
            "validation":[{"launcher":"GRADLE","args":["--project-cache-dir","/tmp/x"]}],
        });
        assert!(validate_plan_value(&unsafe_plan).is_err());
    }

    #[test]
    fn exact_replacement_is_single_occurrence_and_content_bound() {
        let root = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.path())
            .status()
            .unwrap();
        fs::write(root.path().join("A.kt"), "before\n").unwrap();
        let state_root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(state_root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let content = store.put("test/source/1", b"before\n").unwrap();
        let authorized = BTreeMap::from([("A.kt".into(), content.clone())]);
        let changed = apply_operations(
            &store,
            root.path(),
            &[FileOperation::ReplaceText {
                op_id: "replace".into(),
                target: ExistingTarget {
                    file_id: "A.kt".into(),
                    content_ref: content,
                },
                old_text: "before".into(),
                new_text: "after".into(),
            }],
            &authorized,
        )
        .unwrap();
        assert_eq!(changed, ["A.kt"]);
        assert_eq!(
            fs::read_to_string(root.path().join("A.kt")).unwrap(),
            "after\n"
        );
    }
}
